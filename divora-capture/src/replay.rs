//! The replay ring: the last moments of encoded video and mixed audio, in memory (v1.54.0).
//!
//! While Replay is on, the encoder thread pushes every access unit the
//! encoder hands out ([`ReplayRing::push_video`]) and every block of the
//! audio timeline's mixed track ([`ReplayRing::push_pcm`]). The ring keeps
//! the newest `N` seconds of both, and a press of Save cuts a window from
//! them ([`ReplayRing::window`], [`ReplayRing::snapshot`]) for another thread
//! to write as an MP4.
//!
//! What it has to get right:
//!
//! 1. **A clip opens on a keyframe.** The front of the ring is always an IDR,
//!    and eviction drops whole GOPs from the front, never single frames: a
//!    clip that opened on a P-frame would have nothing to decode it against.
//!    The MP4 sink writes such a frame anyway, and players drop it.
//! 2. **Video and audio are cut at the same instant.** Times come from the
//!    pacer index, never from subtracting timestamps: frame `i` is at
//!    `pts_of(i)` and its audio starts at sample [`sample_of`]`(i)`, which is
//!    exact at 30 and 60 fps. Audio older than the front IDR is dropped with
//!    it.
//! 3. **One segment at a time.** Everything in the ring shares one sequence
//!    header and one unbroken clock. A header change, an engine restart after
//!    a hold, or a rebuilt encoder starts a new segment, which clears it.
//! 4. **A cut can wait without losing its start.** Save's audio arrives up
//!    to a second after its picture; while it waits, the cut pins its first
//!    GOP so eviction can't take it ([`ReplayRing::pin`], [`PendingCut`]).
//!
//! It is pure: the encoder thread owns it, with no lock, and every rule is
//! tested without a screen or an encoder.

use std::collections::VecDeque;
use std::sync::Arc;
use std::time::Duration;

use crate::annexb;
use crate::pacer::{FramePacer, TICKS_PER_SECOND};
use crate::timeline::{ticks, OUT_RATE};

/// The direct path's keyframe interval, in seconds: GOP = 2 × fps, set on
/// the encoder and forced at every take and segment start.
const GOP_SECONDS: u64 = 2;

/// Interleaved channels in the PCM ring: the timeline's mixed track is stereo.
const CHANNELS: usize = 2;

/// Audio frames per PCM write in a save ([`interleave`]): 100 ms, so
/// neither stream gets more than one such piece ahead of the other.
pub const PCM_CHUNK_FRAMES: u64 = 4_800;

/// One H.264 access unit as the ring and the muxers keep it: Annex B, one
/// presentation time.
#[derive(Clone, Debug)]
pub struct AccessUnit {
    /// Pacer index of the input frame, on the pipeline's clock.
    pub index: u64,
    /// `pts_of(index)`, in 100-ns ticks.
    pub pts: u64,
    /// `pts_of(index + 1) − pts_of(index)`.
    pub dur: u64,
    /// [`annexb::has_idr`] of `data`: NAL type 5, never the encoder's
    /// `CleanPoint`.
    pub key: bool,
    pub data: Arc<[u8]>,
}

impl AccessUnit {
    /// Frame `index` at `fps`, its times from the pacer and its key flag
    /// from its own bytes.
    #[must_use]
    pub fn new(index: u64, fps: u32, data: Arc<[u8]>) -> Self {
        let pts = pts_of(index, fps);
        Self {
            index,
            pts,
            dur: pts_of(index.saturating_add(1), fps) - pts,
            key: annexb::has_idr(&data),
            data,
        }
    }
}

/// How long, and how much, the ring keeps.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RingConfig {
    pub fps: u32,
    /// `N`: the clip length the user picked.
    pub seconds: u32,
    /// [`budget::cap_bytes`]: video plus the PCM ring's allocation.
    pub cap_bytes: u64,
}

/// What [`ReplayRing::push_video`] did with an access unit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pushed {
    Stored,
    /// The second piece of a frame already stored (the encoder split it
    /// across two outputs): appended to it, so the frame keeps one
    /// presentation time, which the MP4 sink requires.
    Merged,
    /// Dropped: this segment has no IDR yet, and the ring's front must be one.
    SkippedNoKey,
    /// Stored, and this many GOPs left the front to make room.
    Evicted {
        gops: u32,
    },
    /// Not stored: an IDR whose in-band SPS/PPS differ from the segment's.
    /// The caller resolves any pending cut, starts a segment with the new
    /// header and pushes the unit again.
    HeaderChanged,
    /// Dropped: the ring is frozen at or before this frame.
    Frozen,
    /// Dropped: older than the newest stored frame. The pipeline's output
    /// matching never does this, so it is a bug, counted in
    /// [`RingStats::out_of_order`].
    OutOfOrder,
}

/// The ring's own backstop against an encoder that stops making keyframes:
/// without them nothing can be evicted, and the ring would grow one GOP
/// without bound.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RingFault {
    NoKeyframe { frames_since: u64 },
}

/// What the ring holds and has done. The first four describe it now; the
/// rest count up over the ring's life, across segments, for the one summary
/// line at teardown.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RingStats {
    /// From the front IDR to the end of the newest frame.
    pub buffered_ms: u64,
    /// Video bytes plus the PCM ring's allocation: what eviction holds
    /// under `cap_bytes`.
    pub bytes: u64,
    pub cap_bytes: u64,
    pub gops: u32,
    /// GOPs evicted because they were older than `N`.
    pub evicted_by_time: u64,
    /// GOPs evicted to stay under `cap_bytes`: a save is then shorter than
    /// `N`.
    pub evicted_by_bytes: u64,
    /// Audio frames that arrived with the PCM ring full.
    pub pcm_overflow: u64,
    /// Frames dropped because they precede the segment.
    pub pcm_clipped: u64,
    /// Frames zero-filled because they came before the segment began.
    pub pcm_head_missing: u64,
    /// Access units dropped for arriving out of order (see [`Pushed::OutOfOrder`]).
    pub out_of_order: u64,
}

/// Where a frozen ring stops: video before `video_index`, audio before
/// `pcm_sample`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Freeze {
    pub video_index: u64,
    pub pcm_sample: u64,
}

/// A span of the ring to save: frames `[first_index, end_index)` and the
/// audio samples that play under them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Window {
    /// An IDR.
    pub first_index: u64,
    /// Exclusive.
    pub end_index: u64,
    /// `sample_of(first_index)`.
    pub first_sample: u64,
    /// `sample_of(end_index)`, exclusive.
    pub end_sample: u64,
}

/// A window copied out of the ring, for the save thread. It owns all of it:
/// later pushes and evictions can't touch it.
#[derive(Clone, Debug)]
pub struct Snapshot {
    /// The segment's SPS and PPS, with start codes.
    pub seq: Arc<[u8]>,
    pub fps: u32,
    /// The encoded picture's size; the pipeline fills it in.
    pub size: (u32, u32),
    /// The window's access units in order, the first an IDR. Shared with
    /// the ring, not copied.
    pub video: Vec<AccessUnit>,
    /// Interleaved stereo, exactly `end_sample − first_sample` frames:
    /// silence wherever the ring had no audio.
    pub pcm: Vec<i16>,
    pub window: Window,
    /// Silence added at the tail because the audio had not arrived (a cut
    /// that timed out).
    pub padded_frames: u64,
}

impl Snapshot {
    /// About what the save will write: for the free-space check.
    #[must_use]
    pub fn bytes(&self) -> u64 {
        let video: u64 = self.video.iter().map(|au| au.data.len() as u64).sum();
        video + (self.pcm.len() * std::mem::size_of::<i16>()) as u64
    }

    #[must_use]
    pub fn duration_ms(&self) -> u64 {
        let frames = self
            .window
            .end_index
            .saturating_sub(self.window.first_index);
        frames * 1_000 / u64::from(self.fps.max(1))
    }

    /// The order to write this snapshot in ([`interleave`]).
    #[must_use]
    pub fn writes(&self) -> Vec<Write> {
        let indices: Vec<u64> = self.video.iter().map(|au| au.index).collect();
        let frames = (self.pcm.len() / CHANNELS) as u64;
        interleave(
            &indices,
            self.window.first_index,
            self.fps,
            frames,
            PCM_CHUNK_FRAMES,
        )
    }
}

/// One step of a save: an access unit, by its position in the snapshot's
/// `video`, or audio frames `[from, to)` counted from the clip's start.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Write {
    Video(usize),
    Pcm { from: u64, to: u64 },
}

/// The order a save writes a clip in: before each access unit, every
/// `chunk`-frame piece of audio that starts at or before its time; the rest
/// of the audio at the end.
///
/// Video times are `pts_of(index − first_index)` and audio starts at file
/// time 0, so both streams begin at exactly 0. Writing them in time order
/// keeps either from running far ahead in the sink's queue — all the video
/// then all the audio is what hangs a throttled writer.
#[must_use]
pub fn interleave(
    video_indices: &[u64],
    first_index: u64,
    fps: u32,
    pcm_frames: u64,
    chunk: u64,
) -> Vec<Write> {
    let chunk = chunk.max(1);
    let mut out = Vec::new();
    let mut next = 0;
    let mut audio_until = |out: &mut Vec<Write>, at: Option<u64>| {
        while next < pcm_frames && at.is_none_or(|t| ticks(next) <= t) {
            let to = (next + chunk).min(pcm_frames);
            out.push(Write::Pcm { from: next, to });
            next = to;
        }
    };
    for (pos, &index) in video_indices.iter().enumerate() {
        let at = pts_of(index.saturating_sub(first_index), fps);
        audio_until(&mut out, Some(at));
        out.push(Write::Video(pos));
    }
    audio_until(&mut out, None);
    out
}

/// A pending Save: its window is pinned in the ring while the audio for its
/// last frames comes in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PendingCut {
    pub window: Window,
    /// On the pipeline's clock.
    pub registered_at: Duration,
}

/// What a [`PendingCut`] is waiting for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CutState {
    Wait,
    Ready,
    /// Waited long enough: save it now, the missing audio as silence.
    TimedOut,
}

impl PendingCut {
    /// Longer than the timeline's own wait for audio (`MAX_WAIT_SECONDS`,
    /// 1 s, after which a stopped engine's samples are written as silence)
    /// plus one 20 ms audio pump, with some slack.
    pub const TIMEOUT: Duration = Duration::from_millis(1_200);

    /// Ready once the ring holds the window's last frame (`newest_index ≥
    /// end_index − 1`) AND its audio reaches `end_sample`: the picture
    /// arrives about a frame after the press, the audio up to a second later.
    #[must_use]
    pub fn poll(&self, now: Duration, newest_index: Option<u64>, pcm_end_sample: u64) -> CutState {
        let video = newest_index.is_some_and(|i| i.saturating_add(1) >= self.window.end_index);
        let audio = pcm_end_sample >= self.window.end_sample;
        if video && audio {
            CutState::Ready
        } else if now.saturating_sub(self.registered_at) >= Self::TIMEOUT {
            CutState::TimedOut
        } else {
            CutState::Wait
        }
    }
}

/// The audio sample frame `index` starts on: `index · 48000 / fps`. Exact
/// whenever 48000 is a multiple of `fps`, as it is at 30 and 60, so the
/// audio of a clip starts on its first frame to the sample.
#[must_use]
pub fn sample_of(index: u64, fps: u32) -> u64 {
    let s = u128::from(index) * u128::from(OUT_RATE) / u128::from(fps.max(1));
    u64::try_from(s).unwrap_or(u64::MAX)
}

/// Frame `index`'s presentation time at `fps`, as the pacer gives it.
fn pts_of(index: u64, fps: u32) -> u64 {
    FramePacer::new(fps).pts_of(index)
}

/// Frames since the last IDR after which the ring gives up on the encoder:
/// two GOPs, plus the three frames (at 30 fps) a forced IDR may land late.
fn keyframe_limit(fps: u32) -> u64 {
    let fps = u64::from(fps);
    2 * GOP_SECONDS * fps + 3 * fps / 30
}

/// The PCM ring's size for clips of `seconds`: the clip, one GOP more, a
/// pending cut's wait and the timeline running ahead of the picture, with
/// room to spare ([`budget::PCM_SLACK_S`]).
fn pcm_capacity(seconds: u32) -> usize {
    let frames = (u64::from(seconds) + budget::PCM_SLACK_S) * u64::from(OUT_RATE);
    usize::try_from(frames).unwrap_or(usize::MAX)
}

fn to_usize(n: u64) -> usize {
    usize::try_from(n).unwrap_or(usize::MAX)
}

/// Interleaved stereo i16 at 48 kHz, allocated once, written round and
/// round. It holds the timeline's mixed track from `first_sample` on, on the
/// pipeline's sample clock.
#[derive(Debug)]
pub struct PcmRing {
    buf: Box<[i16]>,
    /// The slot of the first frame.
    start: usize,
    len_frames: usize,
    /// The pipeline sample index of the first frame: the anchor that puts
    /// the audio on the video's clock. When empty, the sample the ring
    /// expects next.
    first_sample: u64,
    /// Where the current segment begins; nothing before it is kept.
    segment_from: u64,
}

impl PcmRing {
    fn new(capacity_frames: usize, from: u64) -> Self {
        Self {
            buf: vec![0; capacity_frames.max(1).saturating_mul(CHANNELS)].into_boxed_slice(),
            start: 0,
            len_frames: 0,
            first_sample: from,
            segment_from: from,
        }
    }

    #[must_use]
    pub fn first_sample(&self) -> u64 {
        self.first_sample
    }

    /// One past the newest frame: where the next push must start.
    #[must_use]
    pub fn end_sample(&self) -> u64 {
        self.first_sample + self.len_frames as u64
    }

    #[must_use]
    pub fn len_frames(&self) -> usize {
        self.len_frames
    }

    #[must_use]
    pub fn capacity_frames(&self) -> usize {
        self.buf.len() / CHANNELS
    }

    /// The allocation, full or not: it counts against the memory cap.
    fn alloc_bytes(&self) -> u64 {
        (self.buf.len() * std::mem::size_of::<i16>()) as u64
    }

    fn free_frames(&self) -> usize {
        self.capacity_frames() - self.len_frames
    }

    /// Slot `n` frames after the first.
    fn slot(&self, n: usize) -> usize {
        (self.start + n) % self.capacity_frames()
    }

    fn clear(&mut self, from: u64) {
        self.start = 0;
        self.len_frames = 0;
        self.first_sample = from;
        self.segment_from = from;
    }

    /// Drop the oldest `frames` (at most all of them).
    fn drop_front(&mut self, frames: usize) {
        let n = frames.min(self.len_frames);
        self.start = self.slot(n);
        self.len_frames -= n;
        self.first_sample += n as u64;
    }

    /// Drop every frame before `sample`. Past the end it empties the ring
    /// and moves where it expects the next frame, so audio for frames
    /// already evicted is never taken in.
    fn trim_before(&mut self, sample: u64) {
        if sample <= self.first_sample {
            return;
        }
        let n = sample - self.first_sample;
        if n >= self.len_frames as u64 {
            self.start = 0;
            self.len_frames = 0;
            self.first_sample = sample;
        } else {
            self.drop_front(to_usize(n));
        }
    }

    /// Append `frames` of silence, or of `samples` if given. The caller
    /// makes room first; what still doesn't fit is not written.
    fn append(&mut self, samples: Option<&[i16]>, frames: usize) {
        let mut frames = frames.min(self.free_frames());
        if let Some(s) = samples {
            frames = frames.min(s.len() / CHANNELS);
        }
        let mut done = 0;
        while done < frames {
            let slot = self.slot(self.len_frames);
            let n = (frames - done).min(self.capacity_frames() - slot);
            let dst = &mut self.buf[slot * CHANNELS..(slot + n) * CHANNELS];
            match samples {
                Some(s) => dst.copy_from_slice(&s[done * CHANNELS..(done + n) * CHANNELS]),
                None => dst.fill(0),
            }
            self.len_frames += n;
            done += n;
        }
    }

    /// Copy what the ring holds of `[from, to)` into `out` (which is
    /// `to − from` frames, zeroed). Returns the frames missing at the tail.
    fn copy_window(&self, from: u64, to: u64, out: &mut [i16]) -> u64 {
        let a = self.first_sample.clamp(from, to);
        let b = self.end_sample().clamp(a, to);
        let at = to_usize(a - from);
        let skip = to_usize(a.saturating_sub(self.first_sample));
        let count = to_usize(b - a).min((out.len() / CHANNELS).saturating_sub(at));
        let mut done = 0;
        while done < count {
            let slot = self.slot(skip + done);
            let n = (count - done).min(self.capacity_frames() - slot);
            out[(at + done) * CHANNELS..(at + done + n) * CHANNELS]
                .copy_from_slice(&self.buf[slot * CHANNELS..(slot + n) * CHANNELS]);
            done += n;
        }
        to - b
    }

    /// The same audio in a ring of `capacity_frames`, keeping the newest
    /// frames when it is smaller.
    fn reallocated(&self, capacity_frames: usize) -> Self {
        let mut ring = Self::new(capacity_frames, self.segment_from);
        let keep = self.len_frames.min(ring.capacity_frames());
        let skip = self.len_frames - keep;
        ring.first_sample = self.first_sample + skip as u64;
        let mut copy = vec![0; keep * CHANNELS];
        self.copy_window(ring.first_sample, self.end_sample(), &mut copy);
        ring.append(Some(&copy), keep);
        ring
    }
}

/// The last `N` seconds of encoded video and mixed audio. See the module
/// docs.
#[derive(Debug)]
pub struct ReplayRing {
    cfg: RingConfig,
    /// This segment's SPS and PPS (Annex B, with start codes).
    seq: Option<Arc<[u8]>>,
    video: VecDeque<AccessUnit>,
    /// Indices of the IDRs in `video`, oldest first. The front is the
    /// front of `video`.
    gop_starts: VecDeque<u64>,
    video_bytes: u64,
    pcm: PcmRing,
    /// Bumps on every [`start_segment`](Self::start_segment).
    segment: u64,
    frozen: Option<Freeze>,
    /// A pending cut's window start: the GOP holding it is never evicted.
    pin: Option<u64>,
    /// The counters; the "now" fields are filled in by [`stats`](Self::stats).
    stats: RingStats,
}

impl ReplayRing {
    /// An empty ring with its PCM ring allocated. `fps` is held to the
    /// pacer's 1..=240, so the ring's frame times are the pacer's.
    #[must_use]
    pub fn new(cfg: RingConfig) -> Self {
        let cfg = RingConfig {
            fps: cfg.fps.clamp(1, 240),
            ..cfg
        };
        Self {
            cfg,
            seq: None,
            video: VecDeque::new(),
            gop_starts: VecDeque::new(),
            video_bytes: 0,
            pcm: PcmRing::new(pcm_capacity(cfg.seconds), 0),
            segment: 0,
            frozen: None,
            pin: None,
            stats: RingStats::default(),
        }
    }

    /// Clear everything — video, audio, the freeze and the pin — and start
    /// a segment whose header is `seq` and whose audio starts at sample
    /// `pcm_from`. The counters carry on.
    ///
    /// The pipeline resolves any pending cut first: a cut's window is about
    /// to be cleared away, so a pin here is a bug.
    pub fn start_segment(&mut self, seq: Option<Arc<[u8]>>, pcm_from: u64) {
        debug_assert!(
            self.pin.is_none(),
            "a pending cut must be resolved before a new segment"
        );
        self.seq = seq;
        self.video.clear();
        self.gop_starts.clear();
        self.video_bytes = 0;
        self.pcm.clear(pcm_from);
        self.frozen = None;
        self.pin = None;
        self.segment += 1;
    }

    /// Take in one access unit; see [`Pushed`]. Eviction runs after every
    /// store.
    pub fn push_video(&mut self, au: AccessUnit) -> Result<Pushed, RingFault> {
        if self.frozen.is_some_and(|f| au.index >= f.video_index) {
            return Ok(Pushed::Frozen);
        }
        if let Some(back) = self.video.back_mut() {
            if au.index == back.index {
                // One frame's slices split across two outputs.
                let mut data = Vec::with_capacity(back.data.len() + au.data.len());
                data.extend_from_slice(&back.data);
                data.extend_from_slice(&au.data);
                self.video_bytes += au.data.len() as u64;
                back.data = data.into();
                if au.key && !back.key {
                    back.key = true;
                    self.gop_starts.push_back(back.index);
                }
                self.evict();
                return Ok(Pushed::Merged);
            }
            debug_assert!(
                au.index > back.index,
                "access unit {} after {}",
                au.index,
                back.index
            );
            if au.index < back.index {
                self.stats.out_of_order += 1;
                return Ok(Pushed::OutOfOrder);
            }
        }
        if au.key {
            if annexb::has_parameter_sets(&au.data) {
                match &self.seq {
                    Some(seq) if !annexb::same_parameter_sets(seq, &au.data) => {
                        return Ok(Pushed::HeaderChanged);
                    }
                    Some(_) => {}
                    // No header from the encoder's media type: the first
                    // IDR's in-band sets are the segment's.
                    None => self.seq = annexb::parameter_sets(&au.data),
                }
            }
        } else {
            let Some(&last_idr) = self.gop_starts.back() else {
                return Ok(Pushed::SkippedNoKey);
            };
            let frames_since = au.index.saturating_sub(last_idr);
            if frames_since > keyframe_limit(self.cfg.fps) {
                // Not stored: a GOP that never ends can't be evicted.
                return Err(RingFault::NoKeyframe { frames_since });
            }
        }
        if au.key {
            self.gop_starts.push_back(au.index);
        }
        self.video_bytes += au.data.len() as u64;
        self.video.push_back(au);
        // Audio before the front IDR plays under no frame the ring can save.
        if let Some(&front) = self.gop_starts.front() {
            self.pcm.trim_before(sample_of(front, self.cfg.fps));
        }
        Ok(match self.evict() {
            0 => Pushed::Stored,
            gops => Pushed::Evicted { gops },
        })
    }

    /// Take in one block of the timeline's mixed track, interleaved stereo,
    /// whose first frame is pipeline sample `first_sample`.
    ///
    /// The timeline's blocks are aligned to 1024 frames from sample 0, but a
    /// segment starts on a frame's sample, mostly inside a block; and the
    /// timeline can be ahead of the picture (the A/V offset) or behind it
    /// (its one-second wait). So the block is clipped to the segment: frames
    /// before it are dropped, a late first block leaves a silent head, and
    /// only then must blocks follow on.
    pub fn push_pcm(&mut self, first_sample: u64, frames: &[i16]) {
        let n = (frames.len() / CHANNELS) as u64;
        let end = first_sample.saturating_add(n);
        let clipped = self.pcm.segment_from.saturating_sub(first_sample).min(n);
        self.stats.pcm_clipped += clipped;
        // What the ring already holds (or has trimmed past) is not taken twice.
        let expect = self.pcm.end_sample();
        let from = (first_sample + clipped).max(expect).min(end);
        let to = match self.frozen {
            Some(f) => end.min(f.pcm_sample),
            None => end,
        };
        if from >= to {
            return;
        }
        let gap = from - expect;
        if gap > 0 {
            debug_assert!(
                self.pcm.len_frames == 0,
                "a gap of {gap} frames in the timeline's blocks"
            );
            if self.pcm.len_frames == 0 {
                self.stats.pcm_head_missing += gap;
            }
        }
        let data = &frames
            [to_usize(from - first_sample) * CHANNELS..to_usize(to - first_sample) * CHANNELS];
        let mut silence = to_usize(gap);
        let mut sound = to_usize(to - from);
        // Anything that can't fit even in an emptied ring is the oldest of
        // what came in: lose that, silence first.
        let lost = self.make_room(silence + sound);
        let from_silence = lost.min(silence);
        silence -= from_silence;
        let from_sound = lost - from_silence;
        sound -= from_sound;
        self.pcm.first_sample += lost as u64;
        self.pcm.append(None, silence);
        self.pcm.append(Some(&data[from_sound * CHANNELS..]), sound);
    }

    /// Room for `frames` more in the PCM ring. A full ring evicts the front
    /// GOP when that frees audio (an encoder ignoring its GOP length makes
    /// the video span longer than the PCM ring); a pinned GOP is never
    /// evicted, so then the oldest audio goes, and a snapshot fills that head
    /// with silence. Returns how many of the new frames can't fit at all,
    /// with the ring emptied.
    ///
    /// The ring's end never moves here, so the incoming frames still follow
    /// on: a GOP is only evicted while its successor starts inside the audio
    /// held.
    fn make_room(&mut self, frames: usize) -> usize {
        let free = self.pcm.free_frames();
        if frames <= free {
            return 0;
        }
        self.stats.pcm_overflow += (frames - free) as u64;
        while self.pcm.free_frames() < frames {
            let held = self.pcm.first_sample + 1..=self.pcm.end_sample();
            match self.evictable_front() {
                Some(next) if held.contains(&sample_of(next, self.cfg.fps)) => {
                    self.evict_front_gop(next);
                }
                _ => break,
            }
        }
        let free = self.pcm.free_frames();
        if free < frames {
            self.pcm.drop_front(frames - free);
        }
        frames.saturating_sub(self.pcm.capacity_frames())
    }

    /// Keep clips of `seconds` within `cap_bytes` from now on. Shrinking
    /// evicts at once; growing keeps every frame of audio the ring holds.
    pub fn resize(&mut self, seconds: u32, cap_bytes: u64) {
        self.cfg.seconds = seconds;
        self.cfg.cap_bytes = cap_bytes;
        let capacity = pcm_capacity(seconds);
        if capacity != self.pcm.capacity_frames() {
            self.pcm = self.pcm.reallocated(capacity);
        }
        self.evict();
    }

    /// Hold the ring: from now on it takes video before `video_index` only
    /// (frames already submitted to the encoder still come out) and audio
    /// before `pcm_sample` (the voice's latency and the timeline's wait still
    /// land in the clip's tail). A second freeze keeps the earlier point.
    pub fn freeze(&mut self, video_index: u64, pcm_sample: u64) {
        let at = Freeze {
            video_index,
            pcm_sample,
        };
        self.frozen = Some(match self.frozen {
            Some(f) => Freeze {
                video_index: f.video_index.min(at.video_index),
                pcm_sample: f.pcm_sample.min(at.pcm_sample),
            },
            None => at,
        });
    }

    #[must_use]
    pub fn stats(&self) -> RingStats {
        let buffered = match self.video.front() {
            Some(front) => self.newest_end() - pts_of(front.index, self.cfg.fps),
            None => 0,
        };
        RingStats {
            buffered_ms: buffered / (TICKS_PER_SECOND / 1_000),
            bytes: self.video_bytes + self.pcm.alloc_bytes(),
            cap_bytes: self.cfg.cap_bytes,
            gops: u32::try_from(self.gop_starts.len()).unwrap_or(u32::MAX),
            ..self.stats
        }
    }

    /// The span a Save at frame `end_index` (exclusive) gets: from the
    /// latest IDR at least `N` seconds before it, or from the front when the
    /// ring is shorter than that. `None` while the ring holds no IDR before
    /// `end_index`.
    #[must_use]
    pub fn window(&self, end_index: u64) -> Option<Window> {
        let front = *self.gop_starts.front()?;
        if front >= end_index {
            return None;
        }
        let fps = self.cfg.fps;
        let n = u64::from(self.cfg.seconds) * TICKS_PER_SECOND;
        let end_pts = pts_of(end_index, fps);
        let first_index = self
            .gop_starts
            .iter()
            .rev()
            .copied()
            .find(|&i| i < end_index && pts_of(i, fps).saturating_add(n) <= end_pts)
            .unwrap_or(front);
        Some(Window {
            first_index,
            end_index,
            first_sample: sample_of(first_index, fps),
            end_sample: sample_of(end_index, fps),
        })
    }

    /// Pin a pending cut's window start (`Some`), so eviction keeps the GOP
    /// that holds it, or release it (`None`). The cost is memory: up to the
    /// cut's 1.2 s wait of video over the cap.
    pub fn pin(&mut self, first_index: Option<u64>) {
        self.pin = first_index;
    }

    /// Copy `w` out: the access units as shared references, the audio as a
    /// copy, silence wherever the ring has none.
    #[must_use]
    pub fn snapshot(&self, w: &Window) -> Snapshot {
        let lo = self.video.partition_point(|au| au.index < w.first_index);
        let hi = self
            .video
            .partition_point(|au| au.index < w.end_index)
            .max(lo);
        let video: Vec<AccessUnit> = self.video.range(lo..hi).cloned().collect();
        let frames = to_usize(w.end_sample.saturating_sub(w.first_sample));
        let mut pcm = vec![0; frames.saturating_mul(CHANNELS)];
        let padded_frames =
            self.pcm
                .copy_window(w.first_sample, w.first_sample + frames as u64, &mut pcm);
        let seq = self
            .seq
            .clone()
            .or_else(|| {
                video
                    .first()
                    .and_then(|au| annexb::parameter_sets(&au.data))
            })
            .unwrap_or_else(|| Arc::from(&[][..]));
        Snapshot {
            seq,
            fps: self.cfg.fps,
            size: (0, 0),
            video,
            pcm,
            window: *w,
            padded_frames,
        }
    }

    /// The newest frame stored, for [`PendingCut::poll`].
    #[must_use]
    pub fn newest_index(&self) -> Option<u64> {
        self.video.back().map(|au| au.index)
    }

    /// Where the audio ends, for [`PendingCut::poll`].
    #[must_use]
    pub fn pcm_end_sample(&self) -> u64 {
        self.pcm.end_sample()
    }

    #[must_use]
    pub fn pcm(&self) -> &PcmRing {
        &self.pcm
    }

    #[must_use]
    pub fn frozen(&self) -> Option<Freeze> {
        self.frozen
    }

    #[must_use]
    pub fn segment(&self) -> u64 {
        self.segment
    }

    #[must_use]
    pub fn config(&self) -> RingConfig {
        self.cfg
    }

    /// The end of the newest frame, in ticks.
    fn newest_end(&self) -> u64 {
        self.video
            .back()
            .map_or(0, |au| pts_of(au.index.saturating_add(1), self.cfg.fps))
    }

    /// The second GOP's start, if the front GOP may go: there is another
    /// behind it, and no pending cut starts in it.
    fn evictable_front(&self) -> Option<u64> {
        let next = *self.gop_starts.get(1)?;
        self.pin.is_none_or(|p| p >= next).then_some(next)
    }

    /// Drop every frame before the IDR `next`, and the audio under them.
    fn evict_front_gop(&mut self, next: u64) {
        self.gop_starts.pop_front();
        while let Some(au) = self.video.front() {
            if au.index >= next {
                break;
            }
            self.video_bytes = self.video_bytes.saturating_sub(au.data.len() as u64);
            self.video.pop_front();
        }
        self.pcm.trim_before(sample_of(next, self.cfg.fps));
    }

    /// Whole GOPs from the front, while the next IDR is `N` old or the ring
    /// is over its cap; always keeping the newest GOP and a pinned one.
    /// Returns how many went.
    fn evict(&mut self) -> u32 {
        let fps = self.cfg.fps;
        let n = u64::from(self.cfg.seconds) * TICKS_PER_SECOND;
        let newest = self.newest_end();
        let mut gops = 0u32;
        while let Some(next) = self.evictable_front() {
            if pts_of(next, fps).saturating_add(n) > newest {
                break;
            }
            self.evict_front_gop(next);
            self.stats.evicted_by_time += 1;
            gops = gops.saturating_add(1);
        }
        while let Some(next) = self.evictable_front() {
            if self.video_bytes + self.pcm.alloc_bytes() <= self.cfg.cap_bytes {
                break;
            }
            self.evict_front_gop(next);
            self.stats.evicted_by_bytes += 1;
            gops = gops.saturating_add(1);
        }
        gops
    }
}

/// How much memory Replay may take: the estimate the UI shows a cap from,
/// and the ceiling a cap must stay under.
pub mod budget {
    /// The clip lengths offered, in seconds.
    pub const LENGTHS: [u32; 4] = [15, 30, 60, 120];
    /// The ring holds up to one GOP more than the clip: it evicts whole GOPs.
    pub const GOP_SLACK_S: u64 = 2;
    /// The PCM ring's room beyond the clip — the same figure sizes it, so
    /// the estimate is its allocation exactly.
    pub const PCM_SLACK_S: u64 = 4;
    /// 48 kHz stereo 16-bit.
    pub const PCM_BYTES_PER_S: u64 = 192_000;

    /// What `seconds` of Replay at `bitrate` bits/s holds: the video of the
    /// clip and a GOP at the encoder's mean rate, plus the PCM ring.
    #[must_use]
    pub fn estimate_bytes(seconds: u32, bitrate: u32) -> u64 {
        let video = (u64::from(seconds) + GOP_SLACK_S).saturating_mul(u64::from(bitrate)) / 8;
        let pcm = (u64::from(seconds) + PCM_SLACK_S) * PCM_BYTES_PER_S;
        video.saturating_add(pcm)
    }

    /// The ring's cap: a quarter over the estimate, for a constant-bitrate
    /// encoder's overshoot on a busy screen.
    #[must_use]
    pub fn cap_bytes(estimate: u64) -> u64 {
        estimate.saturating_mul(5) / 4
    }

    /// The most a cap may be on a PC that gives Windows `total_phys` bytes
    /// (`MEMORYSTATUSEX.ullTotalPhys`, less than the RAM installed): an
    /// eighth of it, and never over 1 GiB.
    #[must_use]
    pub fn ceiling(total_phys: u64) -> u64 {
        (1u64 << 30).min(total_phys / 8)
    }

    #[must_use]
    pub fn allowed(seconds: u32, bitrate: u32, total_phys: u64) -> bool {
        cap_bytes(estimate_bytes(seconds, bitrate)) <= ceiling(total_phys)
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        /// `Quality::bitrate` for the table's rows (src-tauri), bits/s.
        const RATES: [u32; 8] = [
            8_087_040,  // 1920×1080 30 Balanced
            16_174_080, // 1920×1080 60 Balanced
            14_376_960, // 2560×1440 30 Balanced (the default)
            28_753_920, // 2560×1440 60 Balanced
            10_682_880, // 2560×1070 30 Balanced
            21_365_760, // 2560×1070 60 Balanced
            46_006_272, // 2560×1440 60 High
            8_626_176,  // 2560×1440 30 Small
        ];
        const GIB: u64 = 1 << 30;

        /// Every (seconds, bitrate) the table offers that `total_phys` refuses.
        fn refused(total_phys: u64) -> Vec<(u32, u32)> {
            let mut out = Vec::new();
            for rate in RATES {
                for s in LENGTHS {
                    if !allowed(s, rate, total_phys) {
                        out.push((s, rate));
                    }
                }
            }
            out
        }

        #[test]
        fn the_ceiling_is_min_of_a_gib_and_an_eighth_of_ram() {
            // ullTotalPhys is what Windows has, less than what is installed:
            // a "4 GB" PC reports about 3.8 GiB.
            let four = 4 * GIB;
            let three_eight = 38 * GIB / 10;
            let seven_eight = 78 * GIB / 10;
            assert_eq!(ceiling(four), 536_870_912);
            assert_eq!(ceiling(three_eight), 510_027_366);
            assert_eq!(ceiling(seven_eight), 1_046_898_278);
            for total in [8 * GIB, 16 * GIB, 96 * GIB, 102_782_390_272] {
                assert_eq!(ceiling(total), GIB, "{total}");
            }

            // 4 GB: 120 s at 1440p60, Balanced (577.9 MB) and High (906.8),
            // are refused; 60 s High (461.0) is allowed.
            for total in [four, three_eight] {
                assert_eq!(
                    refused(total),
                    [(120, 28_753_920), (120, 46_006_272)],
                    "{total}"
                );
                assert!(allowed(60, 46_006_272, total));
            }
            // 8 GB and up: nothing at these settings.
            for total in [seven_eight, 8 * GIB, 16 * GIB, 96 * GIB] {
                assert!(refused(total).is_empty(), "{total}");
            }
        }

        /// The default (1440p30 Balanced, 30 s): 64.0 MB estimated, an
        /// 80.0 MB cap, and the PCM term is the PCM ring's allocation.
        #[test]
        fn the_default_estimate_is_64_mb_with_an_80_mb_cap() {
            assert_eq!(estimate_bytes(30, 14_376_960), 64_035_840);
            assert_eq!(cap_bytes(64_035_840), 80_044_800);
            let ring = crate::replay::ReplayRing::new(crate::replay::RingConfig {
                fps: 30,
                seconds: 30,
                cap_bytes: 0,
            });
            assert_eq!(
                ring.pcm.alloc_bytes(),
                estimate_bytes(30, 0),
                "the estimate's PCM term is the ring's allocation"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::timeline::BLOCK_FRAMES;

    const AUD: &[u8] = &[0, 0, 0, 1, 0x09, 0xF0];
    const SPS: &[u8] = &[0, 0, 0, 1, 0x67, 0x64, 0x00, 0x28, 0xAC];
    const SPS_B: &[u8] = &[0, 0, 0, 1, 0x67, 0x64, 0x00, 0x32, 0xAC];
    const PPS: &[u8] = &[0, 0, 0, 1, 0x68, 0xEE, 0x3C, 0x80];

    /// An IDR access unit of `size` bytes with in-band `sps` and PPS, as
    /// NVIDIA lays them out: AUD, SPS, PPS, IDR.
    fn idr_bytes(sps: &[u8], size: usize) -> Vec<u8> {
        let mut v = [AUD, sps, PPS, &[0, 0, 1, 0x65]].concat();
        v.resize(size.max(v.len() + 1), 0xAB);
        v
    }

    /// A P-frame access unit of `size` bytes: AUD, a non-IDR slice.
    fn p_bytes(size: usize) -> Vec<u8> {
        let mut v = [AUD, &[0, 0, 1, 0x41]].concat();
        v.resize(size.max(v.len() + 1), 0xCD);
        v
    }

    fn header(sps: &[u8]) -> Arc<[u8]> {
        annexb::parameter_sets(&idr_bytes(sps, 0)).unwrap()
    }

    fn au(index: u64, fps: u32, key: bool, size: usize) -> AccessUnit {
        let data = if key {
            idr_bytes(SPS, size)
        } else {
            p_bytes(size)
        };
        AccessUnit::new(index, fps, data.into())
    }

    fn ring(fps: u32, seconds: u32) -> ReplayRing {
        let mut r = ReplayRing::new(RingConfig {
            fps,
            seconds,
            cap_bytes: u64::MAX,
        });
        r.start_segment(Some(header(SPS)), 0);
        r
    }

    /// The test signal: every sample of the timeline's track is its own
    /// index, so a misplaced copy can't pass, and none is silence.
    fn pat(s: u64) -> [i16; 2] {
        let v = i16::try_from(s % 32_000 + 1).unwrap();
        [v, -v]
    }

    fn block(first: u64, frames: usize) -> Vec<i16> {
        (first..first + frames as u64).flat_map(pat).collect()
    }

    /// A pipeline in miniature: frames in order, an IDR every GOP from
    /// `key_from`, and the timeline's 1024-frame blocks (aligned from sample
    /// 0) up to the end of each frame, as the live loop interleaves them.
    struct Feed {
        fps: u32,
        gop: u64,
        size: usize,
        key_from: u64,
        index: u64,
        sample: u64,
    }

    impl Feed {
        fn new(fps: u32) -> Self {
            Self {
                fps,
                gop: GOP_SECONDS * u64::from(fps),
                size: 64,
                key_from: 0,
                index: 0,
                sample: 0,
            }
        }

        fn video(&mut self, r: &mut ReplayRing) -> Pushed {
            let key = self.index >= self.key_from
                && (self.index - self.key_from).is_multiple_of(self.gop);
            let p = r.push_video(au(self.index, self.fps, key, self.size));
            self.index += 1;
            p.expect("no keyframe fault")
        }

        /// Whole blocks while they end at or before `until`.
        fn audio_until(&mut self, r: &mut ReplayRing, until: u64) {
            let n = BLOCK_FRAMES as u64;
            while self.sample + n <= until {
                r.push_pcm(self.sample, &block(self.sample, BLOCK_FRAMES));
                self.sample += n;
            }
        }

        /// The audio under every frame pushed so far, so a window's tail is
        /// all there.
        fn flush(&mut self, r: &mut ReplayRing) {
            let end = sample_of(self.index, self.fps);
            self.audio_until(r, end.div_ceil(BLOCK_FRAMES as u64) * BLOCK_FRAMES as u64);
        }

        /// One frame, then the audio under it.
        fn step(&mut self, r: &mut ReplayRing) -> Pushed {
            let p = self.video(r);
            self.audio_until(r, sample_of(self.index, self.fps));
            p
        }

        fn seconds(&mut self, r: &mut ReplayRing, s: u64) {
            for _ in 0..s * u64::from(self.fps) {
                self.step(r);
            }
        }
    }

    fn front(r: &ReplayRing) -> &AccessUnit {
        r.video.front().unwrap()
    }

    /// The snapshot's audio against the test signal from `first`: silent
    /// before `sound_from`, the signal from there on.
    fn assert_pcm(snap: &Snapshot, sound_from: u64) {
        let first = snap.window.first_sample;
        for (j, lr) in snap.pcm.as_chunks::<2>().0.iter().enumerate() {
            let s = first + j as u64;
            let want = if s < sound_from { [0, 0] } else { pat(s) };
            assert_eq!(*lr, want, "sample {s}");
        }
    }

    #[test]
    fn once_full_the_length_stays_between_n_and_n_plus_a_gop() {
        for fps in [30, 60] {
            for n in budget::LENGTHS {
                let mut r = ring(fps, n);
                let gop = GOP_SECONDS * u64::from(fps);
                let (lo, hi) = (
                    u64::from(n) * 1_000,
                    (u64::from(n) + GOP_SECONDS) * 1_000 + 5 * 1_000 / u64::from(fps),
                );
                // Every seventh IDR is a forced one that came 3 frames late.
                let mut next_idr = 0;
                let mut idrs = 0;
                for i in 0..600 * u64::from(fps) {
                    let key = i == next_idr;
                    if key {
                        idrs += 1;
                        next_idr += if idrs % 7 == 0 { gop + 3 } else { gop };
                    }
                    r.push_video(au(i, fps, key, 16)).unwrap();
                    assert!(front(&r).key, "{fps} fps, {n} s: front of {i} not an IDR");
                    if pts_of(i + 1, fps) >= u64::from(n) * TICKS_PER_SECOND {
                        let ms = r.stats().buffered_ms;
                        assert!(
                            (lo..=hi).contains(&ms),
                            "{fps} fps, {n} s, frame {i}: {ms} ms buffered"
                        );
                    }
                }
                assert!(r.stats().evicted_by_time > 0);
            }
        }
    }

    #[test]
    fn eviction_by_bytes_drops_whole_gops_and_keeps_two() {
        let fps = 30;
        let size = 10_000;
        let gop_bytes = 60 * size as u64;
        // Room for two and a half GOPs of video beside the PCM ring; N is
        // far longer than that, so nothing goes by time.
        let pcm = ring(fps, 15).pcm.alloc_bytes();
        let cap = pcm + gop_bytes * 5 / 2;
        let mut r = ReplayRing::new(RingConfig {
            fps,
            seconds: 15,
            cap_bytes: cap,
        });
        r.start_segment(Some(header(SPS)), 0);
        let mut feed = Feed::new(fps);
        feed.size = size;
        for _ in 0..600 {
            let pushed = feed.video(&mut r);
            let s = r.stats();
            let f = front(&r);
            assert!(f.key, "front {} is not an IDR", f.index);
            assert_eq!(f.index, r.gop_starts[0]);
            // Whole GOPs: every frame from the front IDR on is still there.
            assert_eq!(r.video.len() as u64, feed.index - f.index);
            if s.gops >= 2 {
                assert!(
                    s.bytes <= cap,
                    "{} over {cap} with {} GOPs",
                    s.bytes,
                    s.gops
                );
            }
            if let Pushed::Evicted { .. } = pushed {
                // No more than needed: one GOP back would not have fitted.
                assert!(
                    s.bytes + gop_bytes > cap,
                    "evicted too much at {}",
                    feed.index
                );
            }
            if feed.index > 3 * 60 {
                assert!(s.gops >= 2, "{} GOPs at {}", s.gops, feed.index);
            }
        }
        let s = r.stats();
        assert!(s.evicted_by_bytes > 0);
        assert_eq!(s.evicted_by_time, 0);

        // A single GOP bigger than the cap is kept: it is the newest.
        feed.size = 40_000;
        for _ in 0..120 {
            feed.video(&mut r);
        }
        let s = r.stats();
        assert!(s.bytes > cap);
        assert_eq!(s.gops, 1);
        assert_eq!(front(&r).index, 660);
    }

    #[test]
    fn audio_older_than_the_front_idr_is_dropped() {
        let fps = 30;
        let mut r = ring(fps, 15);
        let mut feed = Feed::new(fps);
        let mut evictions = 0;
        for _ in 0..120 * fps {
            // The pacer skips a tick now and then. On a perfectly regular
            // stream eviction fires on the very frame where `newest − N`
            // reaches the next IDR, so trimming the audio to either would
            // agree; skipped ticks make them differ, as they do live.
            if feed.index % 7 == 3 {
                feed.index += 1;
            }
            if let Pushed::Evicted { .. } = feed.step(&mut r) {
                evictions += 1;
                assert_eq!(r.pcm.first_sample, sample_of(front(&r).index, fps));
            }
        }
        assert!(evictions > 40);
    }

    #[test]
    fn a_keyframeless_stream_raises_the_fault() {
        for fps in [30, 60] {
            let mut r = ring(fps, 15);
            // Two GOPs plus 3 frames at 30 fps (100 ms).
            let limit = 2 * 2 * u64::from(fps) + 3 * u64::from(fps) / 30;
            assert_eq!(r.push_video(au(0, fps, true, 16)), Ok(Pushed::Stored));
            for i in 1..=limit {
                assert!(
                    r.push_video(au(i, fps, false, 16)).is_ok(),
                    "{fps} fps, frame {i}"
                );
            }
            let late = limit + 1;
            assert_eq!(
                r.push_video(au(late, fps, false, 16)),
                Err(RingFault::NoKeyframe { frames_since: late })
            );
            // The faulting frame is not kept, and an IDR clears the fault.
            assert_eq!(r.newest_index(), Some(limit));
            assert!(r.push_video(au(late + 1, fps, true, 16)).is_ok());
            assert!(r.push_video(au(late + 2, fps, false, 16)).is_ok());
        }
    }

    #[test]
    fn same_index_slices_are_merged() {
        let fps = 30;
        let mut r = ring(fps, 15);
        r.push_video(au(0, fps, true, 64)).unwrap();
        // Frame 1 in two pieces with one presentation time.
        let a = p_bytes(40);
        let b: Vec<u8> = [&[0, 0, 1, 0x01][..], &[0xEE; 20]].concat();
        assert_eq!(
            r.push_video(AccessUnit::new(1, fps, a.clone().into())),
            Ok(Pushed::Stored)
        );
        assert_eq!(
            r.push_video(AccessUnit::new(1, fps, b.clone().into())),
            Ok(Pushed::Merged)
        );
        assert_eq!(r.video.len(), 2);
        let one = &r.video[1];
        assert_eq!(&one.data[..], &[a, b].concat()[..]);
        assert!(!one.key);
        assert_eq!(
            (one.pts, one.dur),
            (pts_of(1, fps), pts_of(2, fps) - pts_of(1, fps))
        );
        assert_eq!(r.stats().bytes - r.pcm.alloc_bytes(), 64 + 40 + 24);

        // Frame 2's parameter sets, then its IDR slice: the key is ORed, and
        // the frame starts a GOP.
        let sets = [AUD, SPS, PPS].concat();
        let slice: Vec<u8> = [&[0, 0, 1, 0x65][..], &[0x11; 30]].concat();
        assert_eq!(
            r.push_video(AccessUnit::new(2, fps, sets.into())),
            Ok(Pushed::Stored)
        );
        assert_eq!(
            r.push_video(AccessUnit::new(2, fps, slice.into())),
            Ok(Pushed::Merged)
        );
        assert_eq!(r.video.len(), 3);
        assert!(r.video[2].key);
        assert!(annexb::has_idr(&r.video[2].data));
        assert_eq!(r.gop_starts, [0, 2]);
    }

    #[test]
    fn resizing_shrinks_now_and_grows_without_losing_audio() {
        let fps = 30;
        let mut r = ring(fps, 60);
        let mut feed = Feed::new(fps);
        // Past the PCM ring's 64 s, so it has wrapped.
        feed.seconds(&mut r, 90);
        assert!(r.stats().buffered_ms >= 60_000);

        r.resize(15, u64::MAX);
        let s = r.stats();
        assert!(
            (15_000..=17_000).contains(&s.buffered_ms),
            "{} ms",
            s.buffered_ms
        );
        assert_eq!(r.pcm.capacity_frames(), 19 * 48_000);
        assert_eq!(r.pcm.first_sample, sample_of(front(&r).index, fps));
        assert_eq!(r.config().seconds, 15);

        feed.seconds(&mut r, 7);
        let (first, end) = (r.pcm.first_sample, r.pcm.end_sample());
        let mut before = vec![0; to_usize(end - first) * 2];
        r.pcm.copy_window(first, end, &mut before);
        assert_eq!(before, block(first, to_usize(end - first)));

        r.resize(60, u64::MAX);
        assert_eq!(r.pcm.capacity_frames(), 64 * 48_000);
        assert_eq!((r.pcm.first_sample, r.pcm.end_sample()), (first, end));
        let mut after = vec![0; before.len()];
        r.pcm.copy_window(first, end, &mut after);
        assert_eq!(after, before, "the PCM is kept sample for sample");

        // And it fills to the new length, the audio still continuous.
        feed.seconds(&mut r, 60);
        assert!(r.stats().buffered_ms >= 60_000);
        feed.flush(&mut r);
        let w = r.window(feed.index).unwrap();
        assert_pcm(&r.snapshot(&w), 0);
    }

    #[test]
    fn a_new_segment_clears_both_rings() {
        let fps = 30;
        let mut r = ring(fps, 15);
        let mut feed = Feed::new(fps);
        feed.seconds(&mut r, 20);
        assert!(r.window(feed.index).is_some());

        let k = feed.index + 5;
        r.start_segment(Some(header(SPS)), sample_of(k, fps));
        assert_eq!(r.segment(), 2);
        assert_eq!(r.pcm.len_frames(), 0);
        assert_eq!(r.pcm_end_sample(), sample_of(k, fps));
        assert_eq!(r.stats().gops, 0);
        assert_eq!(r.window(u64::MAX), None);

        // Frames before the new segment's IDR are skipped; the window never
        // reaches back past it.
        feed.key_from = k;
        while feed.index < k {
            assert_eq!(feed.step(&mut r), Pushed::SkippedNoKey);
            assert_eq!(r.window(u64::MAX), None);
        }
        for _ in 0..600 {
            feed.step(&mut r);
            let w = r.window(feed.index).unwrap();
            assert!(w.first_index >= k);
            assert!(r.pcm.first_sample >= sample_of(k, fps));
        }

        // With a cut still pinned, starting a segment is a bug.
        if cfg!(debug_assertions) {
            r.pin(Some(k));
            let pinned = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                r.start_segment(None, 0);
            }));
            assert!(pinned.is_err(), "start_segment with a pin set must assert");
        }
    }

    #[test]
    fn a_changed_sequence_header_starts_a_segment() {
        let fps = 30;
        let mut r = ring(fps, 15);
        let idr = |i: u64, sps: &[u8]| AccessUnit::new(i, fps, idr_bytes(sps, 100).into());
        assert_eq!(r.push_video(idr(0, SPS)), Ok(Pushed::Stored));
        r.push_video(au(1, fps, false, 50)).unwrap();
        // The same sets after three-byte codes: no change.
        let three: Vec<u8> = [
            &[0, 0, 1, 0x09, 0xF0][..],
            &SPS[1..],
            &PPS[1..],
            &[0, 0, 1, 0x65, 1],
        ]
        .concat();
        assert_eq!(
            r.push_video(AccessUnit::new(60, fps, three.into())),
            Ok(Pushed::Stored)
        );
        // An IDR without in-band sets keeps the segment's.
        let bare: Vec<u8> = [AUD, &[0, 0, 1, 0x65, 2][..]].concat();
        assert_eq!(
            r.push_video(AccessUnit::new(120, fps, bare.into())),
            Ok(Pushed::Stored)
        );

        // A different SPS: not stored.
        assert_eq!(r.push_video(idr(180, SPS_B)), Ok(Pushed::HeaderChanged));
        assert_eq!(r.newest_index(), Some(120));

        // The pipeline starts a segment with the new header and pushes again.
        r.start_segment(
            annexb::parameter_sets(&idr_bytes(SPS_B, 0)),
            sample_of(180, fps),
        );
        assert_eq!(r.push_video(idr(180, SPS_B)), Ok(Pushed::Stored));
        let snap = r.snapshot(&r.window(181).unwrap());
        assert!(annexb::same_parameter_sets(&snap.seq, &idr_bytes(SPS_B, 0)));

        // A ring opened without a header takes the first IDR's.
        let mut bare_ring = ring(fps, 15);
        bare_ring.start_segment(None, 0);
        bare_ring.push_video(idr(0, SPS_B)).unwrap();
        assert_eq!(
            bare_ring.push_video(idr(60, SPS)),
            Ok(Pushed::HeaderChanged)
        );
    }

    #[test]
    fn the_window_starts_at_the_latest_idr_at_or_before_cut_minus_n() {
        let fps = 30;
        let mut r = ring(fps, 30);
        // Pinned at the start, the ring keeps every GOP, so the earliest IDR
        // (0) and the first after cut − N (360) are both there to be wrongly
        // chosen.
        r.pin(Some(0));
        let mut feed = Feed::new(fps);
        while feed.index < 1234 {
            feed.step(&mut r);
        }
        let w = r.window(1234).unwrap();
        // 1234 is 41.13 s; 41.13 − 30 = 11.13 s = frame 334; IDRs every 60.
        assert_eq!(w.first_index, 300);
        assert_eq!(w.end_index, 1234);
        assert_eq!(
            (w.first_sample, w.end_sample),
            (sample_of(300, fps), sample_of(1234, fps))
        );
        let ms = r.snapshot(&w).duration_ms();
        assert!((30_000..=32_000).contains(&ms), "{ms} ms");
        // On an IDR exactly N before the cut, that IDR.
        assert_eq!(r.window(1260).unwrap().first_index, 360);
        assert_eq!(r.window(1259).unwrap().first_index, 300);
    }

    #[test]
    fn a_short_ring_gives_the_front() {
        let fps = 30;
        let mut r = ring(fps, 30);
        let mut feed = Feed::new(fps);
        feed.seconds(&mut r, 12);
        let w = r.window(feed.index).unwrap();
        assert_eq!(w.first_index, 0);
        let snap = r.snapshot(&w);
        assert_eq!(snap.duration_ms(), 12_000);
        assert_eq!(snap.video.len(), 360);
        assert!(snap.video[0].key);
        // Nothing before the first IDR, so nothing to save.
        assert_eq!(r.window(0), None);
    }

    #[test]
    fn the_pending_cut_waits_for_both_rings_then_times_out() {
        let w = Window {
            first_index: 60,
            end_index: 100,
            first_sample: sample_of(60, 30),
            end_sample: sample_of(100, 30),
        };
        let at = Duration::from_secs(10);
        let cut = PendingCut {
            window: w,
            registered_at: at,
        };
        let soon = at + Duration::from_millis(300);
        // Video in, audio not yet: wait.
        assert_eq!(cut.poll(soon, Some(99), w.end_sample - 1), CutState::Wait);
        // Audio in, video not yet: wait.
        assert_eq!(cut.poll(soon, Some(98), w.end_sample), CutState::Wait);
        assert_eq!(cut.poll(soon, None, w.end_sample), CutState::Wait);
        // Both: ready, even past the timeout.
        assert_eq!(cut.poll(soon, Some(99), w.end_sample), CutState::Ready);
        assert_eq!(
            cut.poll(soon, Some(140), w.end_sample + 5_000),
            CutState::Ready
        );
        assert_eq!(
            cut.poll(at + Duration::from_secs(5), Some(99), w.end_sample),
            CutState::Ready
        );
        // Neither by 1.2 s: time out.
        let edge = at + PendingCut::TIMEOUT;
        assert_eq!(
            cut.poll(at + Duration::from_millis(1_199), Some(99), 0),
            CutState::Wait
        );
        assert_eq!(cut.poll(edge, Some(99), 0), CutState::TimedOut);
        assert_eq!(cut.poll(edge, None, 0), CutState::TimedOut);
        assert_eq!(PendingCut::TIMEOUT, Duration::from_millis(1_200));
    }

    #[test]
    fn a_pending_cut_pins_its_window() {
        let fps = 30;
        for pinned in [true, false] {
            let mut r = ring(fps, 15);
            let mut feed = Feed::new(fps);
            feed.seconds(&mut r, 30);
            // Saved at frame 900 (30 s): the window starts at the IDR at or
            // before 15 s, which is the ring's front.
            let w = r.window(feed.index).unwrap();
            assert_eq!(w.first_index, 420);
            assert_eq!(front(&r).index, 420);
            if pinned {
                r.pin(Some(w.first_index));
            }
            // A second of pushes while the cut waits: at frame 930 the next
            // IDR (480) is 15 s old and the front GOP would go.
            feed.seconds(&mut r, 1);
            if pinned {
                assert_eq!(front(&r).index, 420, "the pinned GOP survives");
                let snap = r.snapshot(&w);
                assert_eq!(snap.video[0].index, 420);
                assert!(snap.video[0].key);
                assert_eq!(snap.video.len(), 480);
                assert_pcm(&snap, 0);
                // Released, it goes with the next push.
                r.pin(None);
                feed.step(&mut r);
            }
            assert_eq!(front(&r).index, 480);
        }
    }

    #[test]
    fn audio_starts_on_the_keyframes_sample_exactly() {
        for fps in [30, 60] {
            assert_eq!(48_000 % fps, 0);
            for i in (0..1_000_000).step_by(997) {
                let t = ticks(sample_of(i, fps));
                assert!(t.abs_diff(pts_of(i, fps)) <= 1, "{fps} fps, frame {i}: {t}");
            }
            let mut r = ring(fps, 15);
            let mut feed = Feed::new(fps);
            feed.seconds(&mut r, 20);
            for end in [feed.index, feed.index - 1, feed.index - 7] {
                let w = r.window(end).unwrap();
                let snap = r.snapshot(&w);
                let frames = (w.end_index - w.first_index) * 48_000 / u64::from(fps);
                assert_eq!(snap.pcm.len() as u64 / 2, frames);
                assert_eq!(snap.pcm.len() as u64 / 2, w.end_sample - w.first_sample);
                assert_eq!(&snap.pcm[..2], &pat(w.first_sample));
            }
        }
    }

    #[test]
    fn a_snapshot_is_untouched_by_later_pushes() {
        let fps = 30;
        let mut r = ring(fps, 15);
        let mut feed = Feed::new(fps);
        feed.seconds(&mut r, 20);
        let snap = r.snapshot(&r.window(feed.index).unwrap());
        let video: Vec<Vec<u8>> = snap.video.iter().map(|au| au.data.to_vec()).collect();
        let pcm = snap.pcm.clone();

        feed.seconds(&mut r, 300);
        assert!(front(&r).index > snap.window.end_index, "all of it evicted");
        let now: Vec<Vec<u8>> = snap.video.iter().map(|au| au.data.to_vec()).collect();
        assert_eq!(now, video);
        assert_eq!(snap.pcm, pcm);
        // The snapshot is the last owner of its units.
        assert!(snap.video.iter().all(|au| Arc::strong_count(&au.data) == 1));
    }

    #[test]
    fn a_frozen_ring_takes_audio_until_the_freeze_point() {
        let fps = 30;
        let mut r = ring(fps, 15);
        let mut feed = Feed::new(fps);
        // Frames to 299; the audio a third of a second behind, as the
        // timeline's wait leaves it.
        while feed.index < 300 {
            feed.video(&mut r);
            feed.audio_until(&mut r, sample_of(feed.index.saturating_sub(10), fps));
        }
        let i = 301;
        let s = sample_of(i, fps);
        r.freeze(i, s);
        assert_eq!(
            r.frozen(),
            Some(Freeze {
                video_index: i,
                pcm_sample: s
            })
        );
        assert!(r.pcm_end_sample() < s);

        // Frame 300 was already in the encoder: taken. 301 on: not.
        assert_eq!(feed.video(&mut r), Pushed::Stored);
        assert_eq!(feed.video(&mut r), Pushed::Frozen);
        assert_eq!(feed.video(&mut r), Pushed::Frozen);
        assert_eq!(r.newest_index(), Some(300));

        // Audio keeps coming to the freeze point exactly, a straddling block
        // cut there, and nothing after it.
        let cut = PendingCut {
            window: r.window(i).unwrap(),
            registered_at: Duration::ZERO,
        };
        assert_eq!(
            cut.poll(Duration::ZERO, r.newest_index(), r.pcm_end_sample()),
            CutState::Wait
        );
        feed.audio_until(&mut r, s + 10 * 1_024);
        assert_eq!(r.pcm_end_sample(), s);
        assert_eq!(
            cut.poll(Duration::ZERO, r.newest_index(), r.pcm_end_sample()),
            CutState::Ready
        );
        let snap = r.snapshot(&cut.window);
        assert_eq!(snap.padded_frames, 0);
        assert_pcm(&snap, 0);

        // A later freeze can't reopen what an earlier one closed.
        r.freeze(400, sample_of(400, fps));
        assert_eq!(
            r.frozen(),
            Some(Freeze {
                video_index: i,
                pcm_sample: s
            })
        );
    }

    #[test]
    fn interleave_orders_by_time_and_leaves_no_audio_hole() {
        let fps = 30;
        let mut r = ring(fps, 15);
        let mut feed = Feed::new(fps);
        feed.seconds(&mut r, 10);
        // A frame the encoder dropped leaves a gap in the indices.
        feed.index += 1;
        feed.seconds(&mut r, 10);
        let snap = r.snapshot(&r.window(feed.index).unwrap());
        let writes = snap.writes();

        let first = snap.window.first_index;
        let time = |pos: usize| pts_of(snap.video[pos].index - first, fps);
        let mut video = Vec::new();
        let mut next = 0;
        for (n, w) in writes.iter().enumerate() {
            match *w {
                Write::Video(pos) => {
                    video.push(pos);
                    // Every chunk starting at or before this frame came first…
                    assert!(next >= snap.pcm.len() as u64 / 2 || ticks(next) > time(pos));
                }
                Write::Pcm { from, to } => {
                    assert_eq!(from, next, "contiguous from 0");
                    assert!(to > from && to - from <= PCM_CHUNK_FRAMES);
                    next = to;
                    // …and none after it started at or before it.
                    if let Some(Write::Video(pos)) = writes[..n]
                        .iter()
                        .rev()
                        .find(|w| matches!(w, Write::Video(_)))
                    {
                        assert!(ticks(from) > time(*pos));
                    }
                }
            }
        }
        assert_eq!(video, (0..snap.video.len()).collect::<Vec<_>>());
        assert_eq!(
            next,
            snap.pcm.len() as u64 / 2,
            "the audio sums to the window"
        );
        assert!(matches!(writes[0], Write::Pcm { from: 0, .. }));

        // No video at all: the audio alone, in chunks.
        assert_eq!(
            interleave(&[], 0, fps, 10_000, 4_800),
            [
                Write::Pcm { from: 0, to: 4_800 },
                Write::Pcm {
                    from: 4_800,
                    to: 9_600
                },
                Write::Pcm {
                    from: 9_600,
                    to: 10_000
                },
            ]
        );
    }

    #[test]
    fn push_pcm_clips_to_the_segment() {
        let n = BLOCK_FRAMES as u64;
        for fps in [30, 60] {
            for k in [17, 1_017] {
                let from = sample_of(k, fps);
                assert_ne!(from % n, 0, "a segment start inside a block");
                let mut r = ring(fps, 15);
                r.start_segment(Some(header(SPS)), from);
                // The timeline is ahead: its blocks begin up to a second
                // before the segment, aligned from sample 0.
                let mut feed = Feed::new(fps);
                feed.key_from = k;
                feed.index = k;
                feed.sample = from.saturating_sub(48_000) / n * n;
                let clipped = from - feed.sample;
                feed.audio_until(&mut r, from + 3 * n);
                feed.seconds(&mut r, 3);
                feed.flush(&mut r);
                let w = r.window(feed.index).unwrap();
                assert_eq!(w.first_index, k);
                let snap = r.snapshot(&w);
                assert_eq!(&snap.pcm[..2], &pat(from), "{fps} fps, k {k}");
                assert_eq!(
                    snap.pcm.len() as u64 / 2,
                    (w.end_index - k) * 48_000 / u64::from(fps)
                );
                assert_pcm(&snap, 0);
                let s = r.stats();
                assert_eq!(s.pcm_clipped, clipped);
                assert_eq!(s.pcm_head_missing, 0);
            }

            // The timeline behind: the first block comes 300 ms after the
            // segment begins, and the head is silence.
            let k = 17;
            let from = sample_of(k, fps);
            let mut r = ring(fps, 15);
            r.start_segment(Some(header(SPS)), from);
            let mut feed = Feed::new(fps);
            feed.key_from = k;
            feed.index = k;
            feed.sample = (from + 14_400).div_ceil(n) * n;
            let late = feed.sample;
            feed.seconds(&mut r, 3);
            feed.flush(&mut r);
            let snap = r.snapshot(&r.window(feed.index).unwrap());
            assert_eq!(snap.padded_frames, 0);
            assert_pcm(&snap, late);
            let s = r.stats();
            assert_eq!(s.pcm_head_missing, late - from);
            assert_eq!(s.pcm_clipped, 0);
        }
    }

    #[test]
    fn overflow_never_evicts_a_pinned_gop() {
        let fps = 30;
        for pinned in [true, false] {
            let mut r = ring(fps, 15);
            let mut feed = Feed::new(fps);
            feed.seconds(&mut r, 20);
            let w = r.window(feed.index).unwrap();
            assert_eq!(w.first_index, 120);
            if pinned {
                r.pin(Some(w.first_index));
            }
            // The encoder goes quiet; the audio runs on past the PCM ring's
            // 19 s.
            let start = r.pcm.first_sample;
            feed.audio_until(&mut r, start + 21 * 48_000);
            assert!(r.stats().pcm_overflow > 0);
            if pinned {
                assert_eq!(front(&r).index, 120, "the pinned GOP survives");
                assert!(r.pcm.first_sample > w.first_sample, "the oldest audio went");
                assert_eq!(r.pcm.len_frames(), r.pcm.capacity_frames());
                let snap = r.snapshot(&w);
                assert_eq!(snap.video[0].index, 120);
                assert_eq!(snap.pcm.len() as u64 / 2, w.end_sample - w.first_sample);
                // The head the ring lost is silence; the rest is there.
                assert_pcm(&snap, r.pcm.first_sample);
                assert_eq!(snap.padded_frames, 0);
            } else {
                // Unpinned, the front GOP goes instead, and its audio with it.
                assert!(front(&r).index > 120);
                assert_eq!(r.pcm.first_sample, sample_of(front(&r).index, fps));
            }
        }
    }
}
