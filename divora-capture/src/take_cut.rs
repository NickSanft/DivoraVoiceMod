//! A take cut out of a longer-running audio timeline (v1.54.0).
//!
//! With Replay on, one [`AudioTimeline`](crate::timeline::AudioTimeline)
//! runs for as long as the pipeline does, and a take is a stretch of it: from
//! its first sample `s0` — the pipeline sample of its first video frame `k0`,
//! `k0 · 48000 / fps` — to its end. The timeline's blocks are 1024 frames
//! counted from pipeline sample 0. A take's must be 1024 frames counted from
//! its own first sample and stamped from 0, because the MP4 writer and the
//! stem files take blocks as they come ([`StemWriter`](crate::stems::StemWriter)
//! refuses one out of step). `s0` is seldom on a block boundary — 15 times in
//! 16 at 30 fps — so each take block is the end of one pipeline block and the
//! start of the next: [`TakeAudioCut`] re-blocks them, the mixed track and the
//! stems at the same sample.
//!
//! The timeline can run ahead of the video clock: the A/V offset stamps every
//! chunk later, and a frame is written as soon as its audio is in. So by the
//! time a take starts, the blocks that hold its first sample may already have
//! gone by. The pipeline keeps the newest of them in a [`BlockTail`], and a
//! cut starts from it.

// Frame counts move between `u64` (the timeline's) and `usize` (buffer
// lengths). Every one is bounded by a take's length at 48 kHz — about 10^9
// frames for six hours — far inside `usize` on the 64-bit targets the app
// ships for.
#![allow(clippy::cast_possible_truncation)]

use std::collections::VecDeque;

use crate::timeline::{ticks, AudioBlock, StemBlock, BLOCK_FRAMES, OUT_RATE};

/// How much a [`BlockTail`] keeps: 600 ms, the most the app's A/V offset can
/// put the timeline ahead of the video (500 ms) and room for a block more.
pub const TAIL_FRAMES: u64 = OUT_RATE as u64 * 6 / 10;

/// The newest blocks and stems the timeline has handed out: at least
/// [`TAIL_FRAMES`] of each, and at most a block more.
#[derive(Debug, Clone, Default)]
pub struct BlockTail {
    blocks: VecDeque<AudioBlock>,
    stems: VecDeque<StemBlock>,
    block_frames: u64,
    stem_frames: u64,
}

impl BlockTail {
    /// Keep `block`, letting the oldest go once the rest still cover
    /// [`TAIL_FRAMES`].
    pub fn push(&mut self, block: AudioBlock) {
        self.block_frames += (block.samples.len() / 2) as u64;
        self.blocks.push_back(block);
        keep_newest(&mut self.blocks, &mut self.block_frames, |b| {
            (b.samples.len() / 2) as u64
        });
    }

    /// The same for the stems handed out with them. Drained from the
    /// timeline on every pump, whether or not a take wants them, so they
    /// never pile up.
    pub fn push_stems(&mut self, stems: impl IntoIterator<Item = StemBlock>) {
        for s in stems {
            self.stem_frames += s.voice.len() as u64;
            self.stems.push_back(s);
        }
        keep_newest(&mut self.stems, &mut self.stem_frames, |s| {
            s.voice.len() as u64
        });
    }

    /// The blocks kept, oldest first.
    pub fn blocks(&self) -> impl Iterator<Item = &AudioBlock> {
        self.blocks.iter()
    }

    /// The stems kept, oldest first.
    pub fn stems(&self) -> impl Iterator<Item = &StemBlock> {
        self.stems.iter()
    }

    /// Frames of mixed audio kept.
    #[must_use]
    pub fn frames(&self) -> u64 {
        self.block_frames
    }

    /// Frames of stems kept.
    #[must_use]
    pub fn stem_frames(&self) -> u64 {
        self.stem_frames
    }

    /// Forget everything: the timeline it came from has gone.
    pub fn clear(&mut self) {
        *self = Self::default();
    }
}

/// Drop the oldest of `q` while the rest still hold [`TAIL_FRAMES`].
fn keep_newest<T>(q: &mut VecDeque<T>, held: &mut u64, frames: impl Fn(&T) -> u64) {
    while let Some(front) = q.front() {
        let n = frames(front);
        if *held < TAIL_FRAMES + n {
            break;
        }
        *held -= n;
        q.pop_front();
    }
}

/// A take's audio, cut out of the pipeline's: its mixed track and, when the
/// timeline has them, its stems, both from the take's first sample.
#[derive(Debug, Clone)]
pub struct TakeAudioCut {
    /// The take's first sample, on the pipeline's count.
    s0: u64,
    /// Take frames handed out as blocks so far.
    next: u64,
    /// Interleaved left/right from take frame `next`, not yet handed out:
    /// less than a block, except just after seeding from a tail.
    carry: Vec<i16>,
    /// The same for the stems, once any have come.
    stem_carry: Option<StemCarry>,
    /// Frames of silence the take starts with because no audio reached
    /// back to its first sample.
    lead_silence: u64,
}

/// The stems' share of a cut: they arrive apart from the mixed blocks, so
/// they keep their own count.
#[derive(Debug, Clone)]
struct StemCarry {
    next: u64,
    voice: Vec<f32>,
    /// Interleaved left/right, two per voice frame; `None` when the track has
    /// no desktop audio, as its first stem block says (a timeline's stems
    /// are all one shape).
    desktop: Option<Vec<f32>>,
}

impl TakeAudioCut {
    /// A cut whose take starts at pipeline sample `s0`, seeded from `tail`:
    /// whatever of the take the timeline handed out before the take began is
    /// in it, and goes out with the first blocks after.
    #[must_use]
    pub fn new(s0: u64, tail: &BlockTail) -> Self {
        let mut cut = Self {
            s0,
            next: 0,
            carry: Vec::new(),
            stem_carry: None,
            lead_silence: 0,
        };
        for b in tail.blocks() {
            cut.take_audio(b);
        }
        for s in tail.stems() {
            cut.take_stems(s);
        }
        cut
    }

    /// The take's first sample, on the pipeline's count.
    #[must_use]
    pub fn s0(&self) -> u64 {
        self.s0
    }

    /// Frames of silence the take starts with because no audio reached back
    /// to its first sample: the tail didn't go back far enough.
    #[must_use]
    pub fn lead_silence(&self) -> u64 {
        self.lead_silence
    }

    /// Take frames of the mixed track handed out so far.
    #[must_use]
    pub fn frames(&self) -> u64 {
        self.next
    }

    /// Pipeline blocks in, take blocks out: what comes before `s0` is
    /// dropped, the rest re-blocked into [`BLOCK_FRAMES`]-frame blocks stamped
    /// `ticks(n)` for take frame `n` — the timeline's own stamping, so the
    /// writer takes them unchanged.
    pub fn audio(&mut self, block: &AudioBlock) -> Vec<AudioBlock> {
        self.take_audio(block);
        let whole = self.carry.len() / (2 * BLOCK_FRAMES) * (2 * BLOCK_FRAMES);
        let rest = self.carry.split_off(whole);
        let head = std::mem::replace(&mut self.carry, rest);
        audio_blocks(&mut self.next, &head)
    }

    /// The same for the stems, cut at the same sample — a stem block across
    /// `s0` is split there, not held back to the next — so they line up
    /// with the take's track exactly as a take of its own would.
    pub fn stems(&mut self, block: &StemBlock) -> Vec<StemBlock> {
        self.take_stems(block);
        self.stem_carry
            .as_mut()
            .map(StemCarry::whole_blocks)
            .unwrap_or_default()
    }

    /// The take ends after `end_frames` of its own frames (`k0`'s to the end
    /// frame's, at 48 kHz). The rest of the track up to there, with silence
    /// for what never came, and the last block filled out with silence —
    /// never with audio from past the end. Stems the same, once any came.
    ///
    /// Blocks already handed out stay out: a timeline running ahead of the
    /// video can have passed `end_frames` already, and then this adds
    /// nothing.
    pub fn finish(&mut self, end_frames: u64) -> (Vec<AudioBlock>, Vec<StemBlock>) {
        let mut audio = Vec::new();
        if let Some(keep) = end_frames.checked_sub(self.next).filter(|&k| k > 0) {
            let padded = keep.div_ceil(BLOCK_FRAMES as u64) as usize * BLOCK_FRAMES;
            self.carry.truncate(2 * keep as usize);
            self.carry.resize(2 * padded, 0);
            audio = audio_blocks(&mut self.next, &self.carry);
        }
        self.carry.clear();
        let stems = self
            .stem_carry
            .as_mut()
            .map(|s| s.finish(end_frames))
            .unwrap_or_default();
        (audio, stems)
    }

    /// Add what `block` holds of the take to the carry.
    fn take_audio(&mut self, block: &AudioBlock) {
        let len = (block.samples.len() / 2) as u64;
        let held = (self.carry.len() / 2) as u64;
        let at = self.s0 + self.next + held;
        let Some((gap, skip)) = join(frame_of(block.pts), len, at) else {
            return;
        };
        if self.next == 0 && held == 0 {
            self.lead_silence += gap;
        }
        self.carry.resize(self.carry.len() + 2 * gap as usize, 0);
        let from = 2 * skip as usize;
        let to = 2 * len as usize;
        self.carry
            .extend_from_slice(block.samples.get(from..to).unwrap_or_default());
    }

    fn take_stems(&mut self, block: &StemBlock) {
        let s0 = self.s0;
        let c = self.stem_carry.get_or_insert_with(|| StemCarry {
            next: 0,
            voice: Vec::new(),
            desktop: block.desktop.as_ref().map(|_| Vec::new()),
        });
        let len = block.voice.len() as u64;
        let at = s0 + c.next + c.voice.len() as u64;
        let Some((gap, skip)) = join(frame_of(block.pts), len, at) else {
            return;
        };
        c.voice.resize(c.voice.len() + gap as usize, 0.0);
        c.voice
            .extend_from_slice(block.voice.get(skip as usize..).unwrap_or_default());
        if let Some(d) = c.desktop.as_mut() {
            d.resize(d.len() + 2 * gap as usize, 0.0);
            let src = block.desktop.as_deref().unwrap_or_default();
            d.extend_from_slice(src.get(2 * skip as usize..).unwrap_or_default());
            // Two desktop samples to every voice frame, whatever came in:
            // the two files must never slide against each other.
            d.resize(2 * c.voice.len(), 0.0);
        }
    }
}

impl StemCarry {
    fn whole_blocks(&mut self) -> Vec<StemBlock> {
        let whole = self.voice.len() / BLOCK_FRAMES * BLOCK_FRAMES;
        let rest = self.voice.split_off(whole);
        let voice = std::mem::replace(&mut self.voice, rest);
        let desktop = self.desktop.as_mut().map(|d| {
            let rest = d.split_off(2 * whole);
            std::mem::replace(d, rest)
        });
        self.blocks(&voice, desktop.as_deref())
    }

    fn finish(&mut self, end_frames: u64) -> Vec<StemBlock> {
        let Some(keep) = end_frames.checked_sub(self.next).filter(|&k| k > 0) else {
            self.voice.clear();
            if let Some(d) = self.desktop.as_mut() {
                d.clear();
            }
            return Vec::new();
        };
        let padded = keep.div_ceil(BLOCK_FRAMES as u64) as usize * BLOCK_FRAMES;
        let mut voice = std::mem::take(&mut self.voice);
        voice.truncate(keep as usize);
        voice.resize(padded, 0.0);
        let desktop = self.desktop.as_mut().map(|d| {
            let mut d = std::mem::take(d);
            d.truncate(2 * keep as usize);
            d.resize(2 * padded, 0.0);
            d
        });
        self.blocks(&voice, desktop.as_deref())
    }

    /// `voice` (and `desktop`), a whole number of blocks, as stem blocks
    /// from take frame `next` on.
    fn blocks(&mut self, voice: &[f32], desktop: Option<&[f32]>) -> Vec<StemBlock> {
        voice
            .as_chunks::<BLOCK_FRAMES>()
            .0
            .iter()
            .enumerate()
            .map(|(k, v)| {
                let (pts, _) = stamp(&mut self.next);
                StemBlock {
                    pts,
                    voice: v.to_vec(),
                    desktop: desktop.map(|d| {
                        d.get(2 * k * BLOCK_FRAMES..2 * (k + 1) * BLOCK_FRAMES)
                            .unwrap_or_default()
                            .to_vec()
                    }),
                }
            })
            .collect()
    }
}

/// `samples`, a whole number of blocks of interleaved left/right, as take
/// blocks from take frame `*next` on.
fn audio_blocks(next: &mut u64, samples: &[i16]) -> Vec<AudioBlock> {
    samples
        .as_chunks::<{ 2 * BLOCK_FRAMES }>()
        .0
        .iter()
        .map(|block| {
            let (pts, duration) = stamp(next);
            AudioBlock {
                pts,
                duration,
                samples: block.to_vec(),
            }
        })
        .collect()
}

/// The time and length of the take block starting at take frame `*next`,
/// moving `*next` past it.
fn stamp(next: &mut u64) -> (u64, u64) {
    let pts = ticks(*next);
    *next += BLOCK_FRAMES as u64;
    (pts, ticks(*next) - pts)
}

/// The output frame a block stamped `pts` starts at: [`ticks`] undone.
/// `ticks(j)` is `⌊j · 625 / 3⌋` at 48 kHz, so `j` is the least whole
/// number with `ticks(j) ≥ pts`.
pub(crate) fn frame_of(pts: u64) -> u64 {
    u64::try_from((u128::from(pts) * 3).div_ceil(625)).unwrap_or(u64::MAX)
}

/// How `len` frames starting at pipeline frame `first` join a carry that
/// reaches `at`: frames of silence to put before them (they start past it),
/// and how many of them to skip (they come before it — before the take, or
/// already in it). `None` when none of them is new.
///
/// The timeline's blocks follow one another, so past the take's head there
/// is never a gap; if there were, silence keeps the take in step.
fn join(first: u64, len: u64, at: u64) -> Option<(u64, u64)> {
    (first + len > at).then(|| (first.saturating_sub(at), at.saturating_sub(first)))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::cast_precision_loss, clippy::cast_sign_loss)]

    use super::{frame_of, BlockTail, TakeAudioCut, TAIL_FRAMES};
    use crate::stems::StemWriter;
    use crate::timeline::tests::{
        blocks_of, fingerprint, mixed, sides, silence, tone, voice_only_takes, Engine, Loopback,
        VOICE_ONLY_V1_52,
    };
    use crate::timeline::{
        ticks, AudioBlock, AudioTimeline, StemBlock, TrackOptions, BLOCK_FRAMES, MAX_WAIT_SECONDS,
        OUT_RATE,
    };

    const BF: u64 = BLOCK_FRAMES as u64;

    /// What pipeline frame `j` holds in a [`coded`] track: a code no other
    /// frame has (below 2^30) and never silence.
    fn code(j: u64) -> [i16; 2] {
        [(j % 32_767 + 1) as i16, (j / 32_767 % 32_767 + 1) as i16]
    }

    /// The voice and desktop stems of frame `j` in a [`coded`] track.
    fn stem_code(j: u64) -> (f32, [f32; 2]) {
        let v = j as f32 + 1.0;
        (v, [-v, v + 0.5])
    }

    /// A pipeline's blocks and stems, `n` of each from frame 0, stamped as
    /// the timeline stamps them, every frame saying which it is.
    fn coded(n: u64) -> (Vec<AudioBlock>, Vec<StemBlock>) {
        (0..n)
            .map(|b| {
                let frames = b * BF..(b + 1) * BF;
                let block = AudioBlock {
                    pts: ticks(b * BF),
                    duration: ticks((b + 1) * BF) - ticks(b * BF),
                    samples: frames.clone().flat_map(code).collect(),
                };
                let stems = StemBlock {
                    pts: ticks(b * BF),
                    voice: frames.clone().map(|j| stem_code(j).0).collect(),
                    desktop: Some(frames.flat_map(|j| stem_code(j).1).collect()),
                };
                (block, stems)
            })
            .unzip()
    }

    fn frames(blocks: &[AudioBlock]) -> Vec<[i16; 2]> {
        blocks
            .iter()
            .flat_map(|b| b.samples.as_chunks::<2>().0.iter().copied())
            .collect()
    }

    fn voice(stems: &[StemBlock]) -> Vec<f32> {
        stems.iter().flat_map(|s| s.voice.iter().copied()).collect()
    }

    fn desktop(stems: &[StemBlock]) -> Vec<[f32; 2]> {
        stems
            .iter()
            .flat_map(|s| {
                s.desktop
                    .as_ref()
                    .expect("desktop")
                    .as_chunks::<2>()
                    .0
                    .to_vec()
            })
            .collect()
    }

    /// Every block where a take's should be: take frame `n` at `ticks(n)`.
    fn assert_stamped(pts: impl Iterator<Item = u64>) {
        for (i, pts) in pts.enumerate() {
            assert_eq!(pts, ticks(i as u64 * BF), "take block {i}");
        }
    }

    /// Feed `cut` the pipeline's blocks and stems in step, as the pipeline
    /// pumps them, and finish it at `end`.
    fn run(
        mut cut: TakeAudioCut,
        blocks: &[AudioBlock],
        stems: &[StemBlock],
        end: u64,
    ) -> (Vec<AudioBlock>, Vec<StemBlock>, TakeAudioCut) {
        let (mut a, mut s) = (Vec::new(), Vec::new());
        for (b, st) in blocks.iter().zip(stems) {
            a.extend(cut.audio(b));
            s.extend(cut.stems(st));
        }
        let (fa, fs) = cut.finish(end);
        a.extend(fa);
        s.extend(fs);
        (a, s, cut)
    }

    #[test]
    fn frame_of_undoes_ticks() {
        for j in (0..5_000_000u64)
            .step_by(7)
            .chain([u64::from(u32::MAX) * 3])
        {
            assert_eq!(frame_of(ticks(j)), j, "frame {j}");
        }
    }

    /// A take that starts with the pipeline is the take v1.52.0 wrote: the
    /// goldens' blocks, through a cut at 0 and finished at their end, hash to
    /// the very fingerprints the timeline's own goldens pin.
    #[test]
    fn a_cut_at_zero_is_the_uncut_track_bit_for_bit() {
        let got: Vec<String> = voice_only_takes()
            .into_iter()
            .map(|mut t| {
                let blocks = blocks_of(&mut t.engine, t.seconds, &t.sb, &t.voice);
                let mut cut = TakeAudioCut::new(0, &BlockTail::default());
                let mut out = Vec::new();
                for b in &blocks {
                    out.extend(cut.audio(b));
                }
                out.extend(
                    cut.finish((t.seconds * f64::from(OUT_RATE)).ceil() as u64)
                        .0,
                );
                format!("{}: {:#018x}", t.name, fingerprint(&out))
            })
            .collect();
        assert_eq!(got, VOICE_ONLY_V1_52);
    }

    /// The production path: the pipeline's timeline is shared, so it is
    /// never finished. The same takes, and the engine carrying on silent past
    /// the end — for the longest a frame waits plus the voice's delay — as a
    /// live one does. Cut at their end, they are the goldens' tracks to
    /// within −80 dB: the chunks past the end refine the clock's line for
    /// the last frames, so it need not be to the bit.
    ///
    /// Pumped as the goldens pump them, as each 10 ms chunk completes: a
    /// drifting clock's line is still settling in a take this short, so
    /// where a frame lands depends a little on when it was pumped: a 20 ms
    /// cadence alone leaves the drifting take −64 dB from its golden.
    #[test]
    fn an_unfinished_shared_timeline_gives_the_same_track() {
        for (mut gold, mut live) in voice_only_takes().into_iter().zip(voice_only_takes()) {
            let golden = frames(&blocks_of(
                &mut gold.engine,
                gold.seconds,
                &gold.sb,
                &gold.voice,
            ));
            let e = &mut live.engine;
            let delay = e.wait + f64::from(e.chain) / f64::from(e.rate);
            let mut tl = AudioTimeline::new();
            let mut cut = TakeAudioCut::new(0, &BlockTail::default());
            let mut out = Vec::new();
            while e.time(e.index) < live.seconds + MAX_WAIT_SECONDS + delay {
                let c = if e.time(e.index) < live.seconds {
                    e.chunk(&live.sb, &live.voice)
                } else {
                    e.chunk(silence, silence)
                };
                tl.push(&c);
                for b in tl.pump(c.at + 0.01) {
                    out.extend(cut.audio(&b));
                }
            }
            let end = (live.seconds * f64::from(OUT_RATE)).ceil() as usize;
            out.extend(cut.finish(end as u64).0);
            let got = frames(&out);
            assert!(got.len() >= end, "{}: {} frames", live.name, got.len());
            let (mut err, mut sig) = (0.0f64, 0.0f64);
            for (g, w) in got[..end].iter().zip(&golden[..end]) {
                for ch in 0..2 {
                    err += (f64::from(g[ch]) - f64::from(w[ch])).powi(2);
                    sig += f64::from(w[ch]).powi(2);
                }
            }
            let db = 10.0 * (err.max(1e-30) / sig).log10();
            assert!(db < -80.0, "{}: {db:.1} dB from the golden", live.name);
        }
    }

    /// A take starting at pipeline sample `s0 = k · 1600` — video frame `k`
    /// at 30 fps — is the pipeline's track from `s0` on, sample for sample,
    /// in blocks stamped from 0.
    #[test]
    fn a_cut_at_k_is_the_tail_of_the_uncut_track() {
        let (uncut, stems) = coded(200);
        let total = 200 * BF;
        for k in [0u64, 1, 2, 15, 16, 17, 100] {
            let s0 = k * 1_600;
            let end = total - s0;
            let (out, _, cut) = run(
                TakeAudioCut::new(s0, &BlockTail::default()),
                &uncut,
                &stems,
                end,
            );
            assert_stamped(out.iter().map(|b| b.pts));
            for (i, b) in out.iter().enumerate() {
                let n = i as u64 * BF;
                assert_eq!(b.duration, ticks(n + BF) - ticks(n), "s0 {s0}: block {i}");
            }
            let got = frames(&out);
            assert_eq!(got.len() as u64, end.div_ceil(BF) * BF, "s0 {s0}");
            for (n, f) in got.iter().enumerate() {
                let n = n as u64;
                let want = if n < end { code(s0 + n) } else { [0, 0] };
                assert_eq!(*f, want, "s0 {s0}: take frame {n}");
            }
            assert_eq!(
                (cut.lead_silence(), cut.frames(), cut.s0()),
                (0, got.len() as u64, s0)
            );
        }
    }

    /// The stems are cut at the take's first sample too — a stem block
    /// across it split there — so each take frame's stems are the very
    /// sources the take's track mixed for that frame, and the stem files take
    /// them in order.
    #[test]
    fn stems_are_cut_at_the_same_sample() {
        // A real track with every source in it: a voice, and desktop audio
        // on a clock of its own, mixed at −6 dB.
        let mut tl = AudioTimeline::with_options(TrackOptions {
            desktop_gain_db: Some(-6.0),
            stems: true,
        });
        let mut engine = Engine::new(48_000, 0.0);
        engine.wait = 0.02;
        let mut desk = Loopback::new(0.004);
        desk.ppm = 120.0;
        let speech = tone(300.0, 0.5);
        let sound = sides(tone(500.0, 0.9), tone(700.0, 0.9));
        let (mut blocks, mut stems) = (Vec::new(), Vec::new());
        for tick in 1..=300u32 {
            let now = f64::from(tick) * 0.01;
            for chunk in engine.until(now, &silence, &speech) {
                tl.push(&chunk);
            }
            for packet in desk.until(now, &sound) {
                tl.push_desktop(&packet);
            }
            blocks.extend(tl.pump(now));
            stems.extend(tl.take_stems());
        }
        assert_eq!(blocks.len(), stems.len());
        let total = blocks.len() as u64 * BF;
        let (uncut_voice, uncut_desk) = (voice(&stems), desktop(&stems));
        let dir = std::env::temp_dir().join(format!("divora-cut-stems-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        for k in [1u64, 16, 37] {
            let s0 = k * 1_600;
            let end = total - s0;
            let (track, cut_stems, _) = run(
                TakeAudioCut::new(s0, &BlockTail::default()),
                &blocks,
                &stems,
                end,
            );
            assert_eq!(track.len(), cut_stems.len(), "s0 {s0}");
            assert_stamped(cut_stems.iter().map(|b| b.pts));
            let (cut_voice, cut_desk) = (voice(&cut_stems), desktop(&cut_stems));
            for n in 0..end as usize {
                let j = s0 as usize + n;
                assert_eq!(
                    cut_voice[n].to_bits(),
                    uncut_voice[j].to_bits(),
                    "s0 {s0}: voice {n}"
                );
                assert_eq!(cut_desk[n], uncut_desk[j], "s0 {s0}: desktop {n}");
            }
            for (n, frame) in frames(&track).iter().enumerate() {
                for ch in 0..2 {
                    assert_eq!(
                        frame[ch],
                        mixed(cut_voice[n], cut_desk[n][ch], -6.0),
                        "s0 {s0}: frame {n} ch {ch}"
                    );
                }
            }
            let mut writer = StemWriter::create(&dir.join(format!("take-{k}.mp4")), true).unwrap();
            for b in &cut_stems {
                writer.write(b).unwrap();
            }
            assert_eq!(writer.finish().unwrap().frames, track.len() as u64 * BF);
        }
        // And the voice is really in them, not silence that would match too.
        assert!(uncut_voice.iter().fold(0.0f32, |m, v| m.max(v.abs())) > 0.45);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// The take's last block is filled out with silence: never with audio
    /// that came after its end, and silence too for what never came at all.
    #[test]
    fn the_end_is_padded_to_a_whole_block() {
        let (uncut, stems) = coded(40);
        let s0 = 1_600;
        // Pipeline blocks 0..7 reach take frame 5 568: five take blocks out,
        // 448 frames of real audio still waiting when the take ends.
        for end in [5_300u64, 5_120, 5_121, 8_000] {
            let (track, cut_stems, _) = run(
                TakeAudioCut::new(s0, &BlockTail::default()),
                &uncut[..7],
                &stems[..7],
                end,
            );
            let padded = end.div_ceil(BF) * BF;
            let got = frames(&track);
            let (cut_voice, cut_desk) = (voice(&cut_stems), desktop(&cut_stems));
            assert_eq!(got.len() as u64, padded, "end {end}");
            assert_eq!(cut_voice.len() as u64, padded, "end {end}: stems");
            for n in 0..padded {
                let i = n as usize;
                let stem = (cut_voice[i], cut_desk[i]);
                if n < end.min(5_568) {
                    assert_eq!(got[i], code(s0 + n), "end {end}: frame {n}");
                    assert_eq!(stem, stem_code(s0 + n), "end {end}: stems {n}");
                } else {
                    assert_eq!(got[i], [0, 0], "end {end}: frame {n} is not silence");
                    assert_eq!(stem, (0.0, [0.0; 2]), "end {end}: stems {n}");
                }
            }
        }
        // A take whose end the track has already passed — the timeline was
        // ahead of the video — gets nothing more.
        let (track, cut_stems, _) = run(
            TakeAudioCut::new(s0, &BlockTail::default()),
            &uncut[..7],
            &stems[..7],
            4_000,
        );
        assert_eq!((track.len(), cut_stems.len()), (5, 5));
    }

    /// A take starting while the timeline runs ahead of the video: the blocks
    /// holding its first sample have gone by, into the tail, and the cut
    /// takes them from there — the same take as one fed from the start. Too
    /// far ahead for the tail, the head it can't reach is silence, counted.
    #[test]
    fn a_cut_seeded_from_the_tail_has_its_head() {
        let (uncut, stems) = coded(120);
        let s0 = 37 * 1_600; // 59 200: inside block 57
        let end = 120 * BF - s0;
        let whole = run(
            TakeAudioCut::new(s0, &BlockTail::default()),
            &uncut,
            &stems,
            end,
        );
        for ahead_ms in [300u64, 800] {
            let gone = (s0 + ahead_ms * 48).div_ceil(BF) as usize;
            let mut tail = BlockTail::default();
            for (b, st) in uncut[..gone].iter().zip(&stems[..gone]) {
                tail.push(b.clone());
                tail.push_stems([st.clone()]);
                assert!(
                    tail.frames() < TAIL_FRAMES + BF,
                    "the tail holds {}",
                    tail.frames()
                );
            }
            assert!(tail.frames() >= TAIL_FRAMES && tail.stem_frames() == tail.frames());
            let (track, cut_stems, cut) = run(
                TakeAudioCut::new(s0, &tail),
                &uncut[gone..],
                &stems[gone..],
                end,
            );
            let first = frame_of(tail.blocks().next().unwrap().pts);
            let lead = first.saturating_sub(s0);
            assert_eq!(lead > 0, ahead_ms > 600, "{ahead_ms} ms ahead");
            assert_eq!(cut.lead_silence(), lead, "{ahead_ms} ms ahead");
            if lead == 0 {
                assert_eq!(track, whole.0, "{ahead_ms} ms ahead");
                assert_eq!(cut_stems, whole.1, "{ahead_ms} ms ahead: stems");
                continue;
            }
            let (got, cut_voice) = (frames(&track), voice(&cut_stems));
            assert_eq!(
                (got.len(), cut_voice.len()),
                (frames(&whole.0).len(), voice(&whole.1).len())
            );
            for n in 0..end {
                let i = n as usize;
                if n < lead {
                    assert_eq!((got[i], cut_voice[i]), ([0, 0], 0.0), "head frame {n}");
                } else {
                    assert_eq!(
                        (got[i], cut_voice[i]),
                        (code(s0 + n), stem_code(s0 + n).0),
                        "frame {n}"
                    );
                }
            }
        }
        let mut tail = BlockTail::default();
        tail.push(uncut[0].clone());
        tail.clear();
        assert_eq!((tail.frames(), tail.blocks().count()), (0, 0));
    }
}
