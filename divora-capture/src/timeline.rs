//! The recording's audio track, built from the engine's buffers.
//!
//! The engine hands over two signals per output buffer — the voice and the
//! soundboard — with a timestamp and a frame index ([`AudioChunk`]). This
//! turns them into what the AAC encoder wants: 48 kHz stereo 16-bit in
//! 1024-frame blocks, each stamped on the video's clock ([`AudioBlock`]).
//!
//! In order, per chunk:
//!
//! 1. **Steps.** A chunk that arrives well after the line [`ClockFit`] has
//!    drawn through the earlier ones means the audio really did stop — the
//!    device starved, as one voice-conversion buffer in twenty-six makes it
//!    — and the gap is kept, as silence, rather than averaged into the clock.
//! 2. **Drops.** A jump in the frame index is buffers the engine could not
//!    keep; they become silence of exactly their length.
//! 3. **Placement.** The soundboard goes where its buffer's timestamp says.
//!    The voice goes earlier, by how long it had already been on its way:
//!    input latency, the input ring, and the effect chain's own delay — a
//!    non-streaming voice model's 256 ms included. The jittery part is
//!    smoothed, and the voice is only moved to follow it in a pause — or
//!    mid-word if it is 20 ms out; the chain's part is exact and moves when
//!    the chain does.
//! 4. **Clock.** [`ClockFit`] maps engine frames onto the video's clock, so
//!    the audio is resampled — from any engine rate, to 48 kHz — onto the
//!    clock the frames are timed by, and does not slide against them over a
//!    long take.
//!
//! A sample is written when the audio for it is in. If none comes — the
//! engine was stopped mid-take — it is written as silence once it is
//! [`MAX_WAIT_SECONDS`] old, so the track never stalls the file.
//!
//! An **epoch** is one unbroken run of the engine's timeline: a take, or the
//! rest of one after a device-loss rebuild (new devices, maybe a new rate).
//! Each gets its own fit and its own buffer; a new one closes the last.

// Sample arithmetic: frame indices, positions and rates are converted between
// integers and floats throughout. Every value involved is bounded by a take's
// length at audio rates — about 10^8 frames for ten hours at 48 kHz, inside
// f64's exact-integer range and far from i64's — and the float-to-int
// conversions are of rounded or floored positions. `x`, `i`, `p`, `w` are the
// interpolation formula's own names.
#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    clippy::many_single_char_names
)]

use std::collections::VecDeque;
use std::sync::Arc;

use crate::clock_fit::{ClockFit, MAX_DRIFT};

/// The track's sample rate.
pub const OUT_RATE: u32 = 48_000;

/// Frames per block: one AAC frame.
pub const BLOCK_FRAMES: usize = 1024;

/// How far off the clock line a chunk must land to count as a step rather
/// than jitter. Scheduling jitter is cancelled before this is checked (see
/// [`AudioChunk::out_frames`]), and what is left is well under it.
const STEP_SECONDS: f64 = 0.020;

/// How long a sample may wait for its audio before it is written as silence.
/// Longer than the slowest voice chain (≈ 256 ms) plus the drain, so a live
/// engine never trips it.
pub const MAX_WAIT_SECONDS: f64 = 1.0;

/// How long a desktop-audio frame may wait for its packet before it is
/// written as silence. Packets come every 10 ms while anything plays, but an
/// idle playback device sends nothing at all, and that must not hold the
/// track back for a whole [`MAX_WAIT_SECONDS`].
pub const DESKTOP_WAIT_SECONDS: f64 = 0.25;

/// How far from where its run's line says a timed desktop packet may land
/// and still continue the run. Windows loses these packets whole, 10 ms at
/// a time, and times them to within a millisecond — except that a playback
/// device's first packet after a start can be stamped about 4 ms off the
/// rest (measured on one USB DAC). Three quarters of a packet keeps clear of
/// both.
const DESKTOP_GAP_SECONDS: f64 = 0.0075;

/// The same for a packet Windows could not time, whose time is only when it
/// was read: that jitters by the reader's wake-ups and the queue's depth.
const DESKTOP_UNTIMED_GAP_SECONDS: f64 = 0.040;

/// A run's first packets set where it sits rather than vote on it, so an
/// odd first timestamp does not drag the line with it.
const ANCHOR_PACKETS: u64 = 3;

/// Where the mix stops being linear: −3 dBFS.
const SOFT_KNEE: f32 = 0.707_945_8;

/// Below this (−60 dBFS) on both sides, desktop audio counts as silent.
const DESKTOP_QUIET: f32 = 0.001;

/// How far the voice's smoothed delay may be from the one in use before the
/// voice is moved while it is sounding. Moving it skips or repeats a few
/// milliseconds, which is a click mid-word and nothing at all in a pause, so
/// in a pause it follows exactly and mid-word only past this.
const LAG_MOVE_IN_SPEECH_SECONDS: f64 = 0.020;

/// Time constant of the voice-delay smoothing. The thing it smooths away —
/// where in the input period each output buffer happens to fall — repeats
/// every few buffers; what it tracks — the input ring slowly filling as two
/// device clocks disagree — takes minutes.
const LAG_TIME_CONSTANT_SECONDS: f64 = 1.0;

/// A voice buffer whose every sample is below this (about −50 dBFS) is a
/// pause, where moving the voice cannot be heard.
const QUIET: f32 = 0.003;

/// How far the resampler's position may be from where the clock says before
/// it jumps instead of gliding.
const RESYNC_SECONDS: f64 = 0.020;

/// Half the interpolation kernel, in source frames, at 48 kHz and below.
/// Scaled up when downsampling so the filter's shape stays the same.
const HALF_TAPS: usize = 24;

/// Kernel phases tabulated per source frame; between two, the coefficients
/// are interpolated linearly.
const PHASES: usize = 256;

/// One output buffer of the engine's, as the recorder hands it in.
#[derive(Debug, Clone)]
pub struct AudioChunk {
    /// Changes when the engine's timeline restarts (see the module docs).
    pub epoch: u64,
    /// When the buffer's callback ran, in seconds on the take's clock: 0 is
    /// the first video frame, and it is negative before that.
    pub at: f64,
    /// Engine-rate frames before this buffer in the epoch, dropped ones
    /// included.
    pub index: u64,
    /// The engine rate.
    pub rate: u32,
    /// Device frames the callback wrote, at `out_rate`. A callback that runs
    /// late finds the device's buffer emptier and writes more, so
    /// `at − out_frames / out_rate` is steady where `at` alone jitters.
    pub out_frames: u32,
    pub out_rate: u32,
    /// How long the oldest voice frame here had been on its way before the
    /// chain ran: input latency plus the input ring's depth, in seconds.
    pub voice_wait: f64,
    /// Frames the effect chain delays the voice by.
    pub chain_latency: u32,
    pub voice: Vec<f32>,
    pub sb: Vec<f32>,
}

/// 1024 frames of 48 kHz stereo 16-bit, ready for the encoder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioBlock {
    /// Start, in 100-ns ticks on the take's clock.
    pub pts: u64,
    /// Length, in 100-ns ticks.
    pub duration: u64,
    /// Interleaved left/right.
    pub samples: Vec<i16>,
}

/// One loopback packet of desktop audio, as the recorder hands it in.
#[derive(Debug, Clone)]
pub struct DesktopChunk {
    /// Changes when the capture device is opened again; a new stream never
    /// continues the last one.
    pub stream: u64,
    /// When `frames[0]` was captured, in seconds on the take's clock.
    pub at: f64,
    /// False when `at` is only an estimate from when the packet was read:
    /// the packet then continues its run wherever it lands.
    pub timed: bool,
    /// Windows said this packet does not follow on from the last.
    pub discontinuity: bool,
    /// The first packet after the playback device went quiet: a gap before
    /// it is silence, not audio lost.
    pub resumed: bool,
    pub rate: u32,
    /// Interleaved left/right.
    pub frames: Vec<f32>,
}

/// What the track is made of.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct TrackOptions {
    /// Mix in desktop audio at this gain, in dB. `None`: the voice and
    /// soundboard only, exactly as v1.52.0 wrote them.
    pub desktop_gain_db: Option<f32>,
    /// Also hand out each source on its own ([`AudioTimeline::take_stems`]).
    pub stems: bool,
}

/// One block of each source before they were mixed, aligned with the
/// [`AudioBlock`] of the same `pts`.
#[derive(Debug, Clone, PartialEq)]
pub struct StemBlock {
    pub pts: u64,
    /// The voice and soundboard, mono, [`BLOCK_FRAMES`] of them.
    pub voice: Vec<f32>,
    /// Desktop audio before its gain, interleaved left/right; `None` when
    /// the track has none.
    pub desktop: Option<Vec<f32>>,
}

/// What the timeline has seen.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct TimelineStats {
    /// Stalls kept as silence.
    pub steps: u64,
    /// Output frames written as silence because their audio never came in
    /// time (the engine was stopped, or nothing was recording yet).
    pub waited_frames: u64,
    /// The latest epoch's clock against the video's, in ppm.
    pub drift_ppm: f64,
    /// The voice's latest placement, in milliseconds before the soundboard.
    pub voice_delay_ms: f64,
    /// Desktop audio: the latest run's clock against the video's, in ppm.
    pub desktop_drift_ppm: f64,
    /// Desktop audio: unbroken runs so far. Each break — the device went
    /// idle, a packet was lost, the device was opened again — starts one.
    pub desktop_runs: u64,
    /// Desktop audio lost mid-run — the reader fell behind and Windows
    /// dropped packets: how many times, and how long in all.
    pub desktop_gaps: u64,
    pub desktop_gap_seconds: f64,
    /// Desktop frames that arrived after their place in the track had been
    /// written, and so were left out.
    pub desktop_late_frames: u64,
    /// Desktop frames written as silence because none came in time (the
    /// device was idle, or the capture stopped).
    pub desktop_waited_frames: u64,
    /// How long the desktop audio has had nothing to hear, in seconds, up to
    /// the newest frame written; `None` without desktop audio.
    pub desktop_quiet_seconds: Option<f64>,
}

/// One output frame before the mix.
#[derive(Debug, Clone, Copy)]
struct Frame {
    voice: f32,
    desktop: [f32; 2],
}

/// Builds the track. Feed it with [`push`](Self::push) (and
/// [`push_desktop`](Self::push_desktop)), take blocks with
/// [`pump`](Self::pump), end with [`finish`](Self::finish).
pub struct AudioTimeline {
    epochs: VecDeque<Epoch>,
    desktop: Option<Desktop>,
    /// The next output frame to write.
    next: u64,
    block: Vec<Frame>,
    stems: Option<Vec<StemBlock>>,
    stats: TimelineStats,
}

impl Default for AudioTimeline {
    fn default() -> Self {
        Self::new()
    }
}

impl AudioTimeline {
    /// The voice and soundboard, as v1.52.0 recorded them.
    #[must_use]
    pub fn new() -> Self {
        Self::with_options(TrackOptions::default())
    }

    #[must_use]
    pub fn with_options(opts: TrackOptions) -> Self {
        let desktop = opts.desktop_gain_db.map(|db| Desktop {
            runs: VecDeque::new(),
            gain: 10f32.powf(db / 20.0),
            last_sound: 0,
            kernel: None,
        });
        Self {
            stats: TimelineStats {
                desktop_quiet_seconds: desktop.as_ref().map(|_| 0.0),
                ..TimelineStats::default()
            },
            epochs: VecDeque::new(),
            desktop,
            next: 0,
            block: Vec::with_capacity(BLOCK_FRAMES),
            stems: opts.stems.then(Vec::new),
        }
    }

    #[must_use]
    pub fn stats(&self) -> TimelineStats {
        self.stats
    }

    /// Take in one engine buffer.
    pub fn push(&mut self, chunk: &AudioChunk) {
        match self.epochs.back_mut() {
            Some(back) if chunk.epoch < back.id => return, // a closed epoch's straggler
            Some(back) if chunk.epoch > back.id => {
                back.close();
                self.epochs.push_back(Epoch::new(chunk.epoch, chunk.rate));
            }
            Some(_) => {}
            None => self.epochs.push_back(Epoch::new(chunk.epoch, chunk.rate)),
        }
        let Some(back) = self.epochs.back_mut() else {
            return;
        };
        match back.push(chunk) {
            Pushed::Placed => {}
            Pushed::Stepped => self.stats.steps += 1,
            Pushed::Early => {
                // The clock went backwards, which no stall explains. Start
                // the line again from here rather than bend it.
                back.close();
                let mut fresh = Epoch::new(chunk.epoch, chunk.rate);
                let _ = fresh.push(chunk);
                self.epochs.push_back(fresh);
                self.stats.steps += 1;
            }
        }
        if let Some(back) = self.epochs.back() {
            self.stats.drift_ppm = back.lane.fit.drift_ppm();
            self.stats.voice_delay_ms = back.voice_delay_ms;
        }
    }

    /// Take in one packet of desktop audio. Ignored when the track has none.
    ///
    /// A packet continues the run before it only if it lands where that
    /// run's line says it should. Anything else — the device went idle and
    /// came back, packets were lost, Windows flagged a break, the device was
    /// opened again — starts a new run on a line of its own. Never a step:
    /// frames invented to bridge a long gap would carry no information
    /// about the device's clock and would bend its fit.
    pub fn push_desktop(&mut self, c: &DesktopChunk) {
        let next_time = frame_time(self.next);
        let Some(d) = self.desktop.as_mut() else {
            return;
        };
        let frames = c.frames.as_chunks::<2>().0;
        if frames.is_empty() {
            return;
        }
        let rate = f64::from(c.rate.max(1));
        // Where the packet lands against the open run's line, and how far
        // off that may be.
        let landing = d
            .runs
            .back()
            .filter(|r| {
                !r.lane.closed && r.stream == c.stream && r.lane.rate.to_bits() == rate.to_bits()
            })
            .map(|r| {
                let off = c.at - r.lane.fit.time_of(r.end as f64);
                let tolerance = if c.timed && r.timed_pairs > 0 {
                    DESKTOP_GAP_SECONDS
                } else {
                    DESKTOP_UNTIMED_GAP_SECONDS
                };
                (off, tolerance)
            });
        let continues = !c.discontinuity && landing.is_some_and(|(off, tol)| off.abs() <= tol);
        if !continues {
            if let Some((off, tol)) = landing {
                if off > tol && !c.resumed {
                    self.stats.desktop_gaps += 1;
                    self.stats.desktop_gap_seconds += off;
                }
            }
            if let Some(r) = d.runs.back_mut() {
                r.lane.closed = true;
            }
            let mut lane = Lane::with_kernel(c.rate, d.kernel(c.rate));
            lane.started = true;
            d.runs.push_back(Run {
                stream: c.stream,
                lane,
                end: 0,
                packets: 0,
                timed_pairs: 0,
            });
            self.stats.desktop_runs += 1;
        }
        let Some(r) = d.runs.back_mut() else {
            return;
        };
        // An untimed packet only says where a run sits when nothing better
        // has; the first real timestamp replaces its guesses.
        if c.timed || r.timed_pairs == 0 {
            if r.packets < ANCHOR_PACKETS || (c.timed && r.timed_pairs == 0) {
                r.lane.fit = ClockFit::new(c.rate);
            }
            r.lane.fit.add(r.end as f64, c.at);
            if c.timed {
                r.timed_pairs += 1;
            }
        }
        r.packets += 1;
        // Late: in the take (not before it starts) and behind the write head.
        let from = c.at.max(0.0);
        let to = c.at + frames.len() as f64 / rate;
        self.stats.desktop_late_frames += ((next_time.min(to) - from).max(0.0) * rate) as u64;
        r.lane.add_at(r.end, frames);
        r.end += frames.len() as i64;
        r.lane.ready_end = r.end;
        self.stats.desktop_drift_ppm = r.lane.fit.drift_ppm();
    }

    /// Where the audio is complete up to, in seconds on the take's clock;
    /// `None` before any audio. `now` is the take's clock: desktop audio that
    /// has not come by `now − DESKTOP_WAIT_SECONDS` never will.
    #[must_use]
    pub fn ready_until(&self, now: f64) -> Option<f64> {
        let engine = self
            .epochs
            .back()
            .map(|e| e.lane.fit.time_of(e.lane.ready_end as f64));
        let desktop = self.desktop.as_ref().map(|d| {
            let floor = now - DESKTOP_WAIT_SECONDS;
            d.runs
                .back()
                .map_or(floor, |r| r.lane.fit.time_of(r.end as f64).max(floor))
        });
        match (engine, desktop) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        }
    }

    /// Every block that can be written now. `now` is the take's clock: a
    /// sample older than `now − MAX_WAIT_SECONDS` with no audio is silence.
    pub fn pump(&mut self, now: f64) -> Vec<AudioBlock> {
        let mut out = Vec::new();
        while let Some(f) = self.sample(now) {
            self.push_frame(f);
            if self.block.len() == BLOCK_FRAMES {
                out.push(self.emit());
            }
        }
        out
    }

    /// The rest of the track, through `end` (the take's clock), padded to a
    /// whole block. Whatever has not arrived by now never will.
    pub fn finish(&mut self, end: f64) -> Vec<AudioBlock> {
        for e in &mut self.epochs {
            e.close();
        }
        if let Some(d) = self.desktop.as_mut() {
            for r in &mut d.runs {
                r.lane.closed = true;
            }
        }
        let end_frame = (end.max(0.0) * f64::from(OUT_RATE)).ceil() as u64;
        let mut out = Vec::new();
        while self.next < end_frame || !self.block.is_empty() {
            // Past the end is padding, not audio that failed to come.
            let f = if self.next < end_frame {
                self.sample(f64::INFINITY).unwrap_or(SILENT_FRAME)
            } else {
                SILENT_FRAME
            };
            self.push_frame(f);
            if self.block.len() == BLOCK_FRAMES {
                out.push(self.emit());
            }
        }
        out
    }

    /// Every block's sources as they were before the mix, since the last
    /// call; always empty unless the track was made with stems.
    pub fn take_stems(&mut self) -> Vec<StemBlock> {
        self.stems.as_mut().map(std::mem::take).unwrap_or_default()
    }

    fn push_frame(&mut self, f: Frame) {
        self.block.push(f);
        self.next += 1;
        if let Some(d) = self.desktop.as_mut() {
            if f.desktop.iter().any(|s| s.abs() >= DESKTOP_QUIET) {
                d.last_sound = self.next;
            }
            self.stats.desktop_quiet_seconds = Some(frame_time(self.next - d.last_sound));
        }
    }

    /// The next output frame, or `None` to wait for more audio. A frame is
    /// written only once every source has either delivered it or is
    /// overdue, and only then is the silence counted.
    fn sample(&mut self, now: f64) -> Option<Frame> {
        let j = self.next;
        let tau = frame_time(j);
        let voice = self.engine_at(j, tau, now)?;
        let desktop = match self.desktop.as_mut() {
            Some(d) => d.at(j, tau, now)?,
            None => Got::Audio([0.0; 2]),
        };
        let voice = match voice {
            Got::Audio([v]) => v,
            Got::Overdue => {
                self.stats.waited_frames += 1;
                0.0
            }
        };
        let desktop = match desktop {
            Got::Audio(v) => v,
            Got::Overdue => {
                self.stats.desktop_waited_frames += 1;
                [0.0; 2]
            }
        };
        Some(Frame { voice, desktop })
    }

    /// The engine's audio for output frame `j`: `None` while it may still
    /// come.
    fn engine_at(&mut self, j: u64, tau: f64, now: f64) -> Option<Got<1>> {
        while let Some(e) = self.epochs.front_mut() {
            match e.lane.sample(j) {
                Sampled::Value(v) => return Some(Got::Audio(v)),
                Sampled::Done => {
                    self.epochs.pop_front();
                }
                Sampled::Wait => break,
            }
        }
        (tau <= now - MAX_WAIT_SECONDS).then_some(Got::Overdue)
    }

    fn emit(&mut self) -> AudioBlock {
        let first = self.next - BLOCK_FRAMES as u64;
        let pts = ticks(first);
        let duration = ticks(self.next) - pts;
        let mut samples = Vec::with_capacity(BLOCK_FRAMES * 2);
        match &self.desktop {
            // Exactly v1.52.0's track: the mono mix on both sides, clamped.
            None => {
                for f in &self.block {
                    let s = to_i16(f.voice);
                    samples.push(s);
                    samples.push(s);
                }
            }
            // The voice in the middle, desktop audio in stereo around it,
            // and a soft knee rather than a hard edge where they add up.
            Some(d) => {
                for f in &self.block {
                    for ch in f.desktop {
                        samples.push(to_i16(soft_clip(f.voice + d.gain * ch)));
                    }
                }
            }
        }
        if let Some(stems) = self.stems.as_mut() {
            stems.push(StemBlock {
                pts,
                voice: self.block.iter().map(|f| f.voice).collect(),
                desktop: self
                    .desktop
                    .is_some()
                    .then(|| self.block.iter().flat_map(|f| f.desktop).collect()),
            });
        }
        self.block.clear();
        for e in &mut self.epochs {
            e.lane.trim();
        }
        if let Some(d) = self.desktop.as_mut() {
            for r in &mut d.runs {
                r.lane.trim();
            }
        }
        AudioBlock {
            pts,
            duration,
            samples,
        }
    }
}

const SILENT_FRAME: Frame = Frame {
    voice: 0.0,
    desktop: [0.0; 2],
};

/// Output frame `j`'s time on the take's clock, in seconds.
fn frame_time(j: u64) -> f64 {
    j as f64 / f64::from(OUT_RATE)
}

/// Output frame `j`'s time in 100-ns ticks, exactly.
pub(crate) fn ticks(j: u64) -> u64 {
    u64::try_from(u128::from(j) * 10_000_000 / u128::from(OUT_RATE)).unwrap_or(u64::MAX)
}

/// A mixed sample to 16 bits: clamped, as v1.52.0 did.
fn to_i16(v: f32) -> i16 {
    (v.clamp(-1.0, 1.0) * f32::from(i16::MAX)) as i16
}

/// Unity up to −3 dBFS, then a smooth knee toward full scale: the voice's
/// limiter stops it at −1 dBFS, so on its own it barely touches the knee, and
/// voice plus a loud game bend rather than crack. (In `f32` the curve meets
/// full scale only past three times it.)
fn soft_clip(x: f32) -> f32 {
    let a = x.abs();
    if a <= SOFT_KNEE {
        return x;
    }
    let room = 1.0 - SOFT_KNEE;
    (SOFT_KNEE + room * ((a - SOFT_KNEE) / room).tanh()).copysign(x)
}

/// A source's say on one output frame.
enum Got<const C: usize> {
    Audio([f32; C]),
    /// Nothing came, and it is too late to wait: silence.
    Overdue,
}

/// The desktop source: its runs, oldest first, and its place in the mix.
struct Desktop {
    runs: VecDeque<Run>,
    /// Linear.
    gain: f32,
    /// The output frame after the last one with any desktop sound.
    last_sound: u64,
    /// One interpolation kernel for every run at the same rate (they are all
    /// 48 kHz): a run starts at every idle resume, and building one is not
    /// free.
    kernel: Option<(u32, Arc<Kernel>)>,
}

impl Desktop {
    fn kernel(&mut self, rate: u32) -> Arc<Kernel> {
        match &self.kernel {
            Some((r, k)) if *r == rate => Arc::clone(k),
            _ => {
                let k = Arc::new(Kernel::new(rate));
                self.kernel = Some((rate, Arc::clone(&k)));
                k
            }
        }
    }

    /// The desktop audio for output frame `j`: `None` while it may still
    /// come.
    ///
    /// Every run that reaches `j` is summed, not just the oldest: each is
    /// silent outside its own audio, so where one run ends and the next
    /// begins — a flagged break with nothing lost — the two halves of the
    /// interpolation kernel add back up to the signal, with no dip.
    fn at(&mut self, j: u64, tau: f64, now: f64) -> Option<Got<2>> {
        loop {
            let mut sum = [0.0f32; 2];
            let mut waiting = false;
            let mut any = false;
            self.runs.retain_mut(|r| {
                // A run still settling where it sits: no resampler on it yet,
                // or the resampler would start on a line about to move and
                // glide after it. Before its audio it is silence; at it, wait
                // for the packets that settle it (or for it to be overdue).
                if !r.lane.closed && r.packets < ANCHOR_PACKETS {
                    let x = r.lane.fit.frame_at(tau);
                    if x + (r.lane.kernel.half as f64) < r.lane.base as f64 {
                        any = true;
                    } else {
                        waiting = true;
                    }
                    return true;
                }
                match r.lane.sample(j) {
                    Sampled::Value([l, r]) => {
                        sum[0] += l;
                        sum[1] += r;
                        any = true;
                        true
                    }
                    Sampled::Wait => {
                        waiting = true;
                        true
                    }
                    Sampled::Done => false,
                }
            });
            if any && !waiting {
                return Some(Got::Audio(sum));
            }
            if tau > now - DESKTOP_WAIT_SECONDS {
                return None;
            }
            // Overdue: the device has gone quiet (an idle playback device
            // sends nothing at all). End the open run so what it did send is
            // written out in full, and silence follows.
            match self.runs.back_mut() {
                Some(r) if !r.lane.closed => r.lane.closed = true,
                _ => return Some(Got::Overdue),
            }
        }
    }
}

/// One unbroken run of desktop audio.
struct Run {
    stream: u64,
    lane: Lane<2>,
    /// Frames in the run so far: where the next packet goes.
    end: i64,
    /// Packets in the run, and how many of them Windows timed.
    packets: u64,
    timed_pairs: u64,
}

enum Pushed {
    Placed,
    Stepped,
    Early,
}

enum Sampled<const C: usize> {
    Value([f32; C]),
    /// The audio for this frame has not arrived yet.
    Wait,
    /// This stretch is over and the frame is past it.
    Done,
}

/// Where the resampler is: output frame `j` reads source position `x`, and
/// each frame after it `step` further, until `until`.
#[derive(Debug, Clone, Copy)]
struct Track {
    j: u64,
    x: f64,
    step: f64,
    until: u64,
}

/// One unbroken stretch of a source's audio, `C` channels at its own rate:
/// where it sits on the take's clock ([`ClockFit`]) and how it is read out
/// at 48 kHz. The engine's epochs are one channel; desktop audio is two.
struct Lane<const C: usize> {
    rate: f64,
    fit: ClockFit,
    kernel: Arc<Kernel>,
    /// Position of `mix[0]`.
    base: i64,
    mix: VecDeque<[f32; C]>,
    /// Everything before this position is final.
    ready_end: i64,
    started: bool,
    closed: bool,
    track: Option<Track>,
}

impl<const C: usize> Lane<C> {
    fn new(rate: u32) -> Self {
        Self::with_kernel(rate, Arc::new(Kernel::new(rate)))
    }

    fn with_kernel(rate: u32, kernel: Arc<Kernel>) -> Self {
        Self {
            rate: f64::from(rate.max(1)),
            fit: ClockFit::new(rate),
            kernel,
            base: 0,
            mix: VecDeque::new(),
            ready_end: 0,
            started: false,
            closed: false,
            track: None,
        }
    }

    /// Add `frames` into the mix starting at `pos`.
    fn add_at(&mut self, pos: i64, frames: &[[f32; C]]) {
        let skip = (self.base - pos).max(0) as usize;
        if skip >= frames.len() {
            return;
        }
        let start = (pos + skip as i64 - self.base) as usize;
        let end = start + frames.len() - skip;
        if self.mix.len() < end {
            self.mix.resize(end, [0.0; C]);
        }
        for (m, s) in self.mix.range_mut(start..end).zip(&frames[skip..]) {
            for (m, s) in m.iter_mut().zip(s) {
                *m += s;
            }
        }
    }

    /// The source position for output frame `j`, moving the resampler there.
    fn position(&mut self, j: u64) -> f64 {
        let nominal = self.rate / f64::from(OUT_RATE);
        let track = match self.track {
            Some(t) if j >= t.j => Track {
                j,
                x: t.x + t.step * (j - t.j) as f64,
                ..t
            },
            _ => Track {
                j,
                x: self.fit.frame_at(frame_time(j)),
                step: nominal,
                until: j,
            },
        };
        let mut t = track;
        if j >= t.until {
            // Aim to land exactly where the clock says at the next block
            // boundary, gliding no faster than the drift clamp allows.
            let until = (j / BLOCK_FRAMES as u64 + 1) * BLOCK_FRAMES as u64;
            let now_x = self.fit.frame_at(frame_time(j));
            if (now_x - t.x).abs() > RESYNC_SECONDS * self.rate {
                t.x = now_x;
            }
            let target = self.fit.frame_at(frame_time(until));
            let step = (target - t.x) / (until - j) as f64;
            t.step = step.clamp(nominal * (1.0 - MAX_DRIFT), nominal * (1.0 + MAX_DRIFT));
            t.until = until;
        }
        self.track = Some(t);
        t.x
    }

    fn sample(&mut self, j: u64) -> Sampled<C> {
        if !self.started {
            return if self.closed {
                Sampled::Done
            } else {
                Sampled::Wait
            };
        }
        let x = self.position(j);
        let i = x.floor() as i64;
        let lo = i - self.kernel.half as i64 + 1;
        let hi = i + self.kernel.half as i64;
        let end = self.ready_end;
        if hi < self.base {
            return Sampled::Value([0.0; C]);
        }
        if self.closed && lo >= end {
            return Sampled::Done;
        }
        if !self.closed && hi >= end {
            return Sampled::Wait;
        }
        Sampled::Value(self.kernel.at(&self.mix, self.base, x))
    }

    /// Drop what the resampler has moved past.
    fn trim(&mut self) {
        let Some(t) = self.track else {
            return;
        };
        let keep_from = t.x.floor() as i64 - self.kernel.half as i64 - 1;
        let drop = (keep_from - self.base).clamp(0, self.mix.len() as i64) as usize;
        if drop > 0 {
            self.mix.drain(..drop);
            self.base += drop as i64;
        }
    }
}

/// One unbroken run of the engine's timeline.
struct Epoch {
    id: u64,
    lane: Lane<1>,
    /// Frames inserted for steps so far: a chunk's position is its index
    /// plus this.
    shift: i64,
    /// Where the next voice frame goes. Everything before it is final.
    voice_cursor: i64,
    sb_end: i64,
    /// Smoothed voice wait, in seconds.
    lag: Option<f64>,
    /// The voice wait in use, in frames.
    lag_frames: i64,
    voice_delay_ms: f64,
    /// Smoothed callback period, in seconds.
    period: Option<f64>,
}

impl Epoch {
    fn new(id: u64, rate: u32) -> Self {
        Self {
            id,
            lane: Lane::new(rate),
            shift: 0,
            voice_cursor: 0,
            sb_end: 0,
            lag: None,
            lag_frames: 0,
            voice_delay_ms: 0.0,
            period: None,
        }
    }

    /// No more audio is coming: the voice is complete wherever it stopped.
    fn close(&mut self) {
        self.lane.closed = true;
        self.sync_ready();
    }

    /// While audio may still come, the voice's cursor is the edge — the
    /// soundboard ahead of it waits for the voice to catch up. Once closed,
    /// whichever reached further.
    fn sync_ready(&mut self) {
        self.lane.ready_end = if self.lane.closed {
            self.voice_cursor.max(self.sb_end)
        } else {
            self.voice_cursor
        };
    }

    fn push(&mut self, c: &AudioChunk) -> Pushed {
        let span = c.voice.len().min(c.sb.len());
        let period = f64::from(c.out_frames) / f64::from(c.out_rate.max(1));
        let mean = *self.period.get_or_insert(period);
        let tau = c.at - (period - mean);
        self.period = Some(mean + 0.05 * (period - mean));

        let rate = self.lane.rate;
        let mut pos = c.index as i64 + self.shift;
        let mut outcome = Pushed::Placed;
        if let Some(r) = self.lane.fit.residual(pos as f64, tau) {
            if r > STEP_SECONDS {
                let k = (r * rate).round() as i64;
                self.shift += k;
                pos += k;
                outcome = Pushed::Stepped;
            } else if r < -STEP_SECONDS {
                return Pushed::Early;
            }
        }
        self.lane.fit.add(pos as f64, tau);

        // The voice's delay: the smoothed wait — followed exactly in a pause,
        // mid-word only when far out — plus the chain's exact latency.
        let alpha = (span as f64 / rate / LAG_TIME_CONSTANT_SECONDS).min(1.0);
        let lag = self
            .lag
            .map_or(c.voice_wait, |l| l + alpha * (c.voice_wait - l));
        self.lag = Some(lag);
        let target = (lag * rate).round() as i64;
        let quiet = c.voice[..span].iter().all(|s| s.abs() < QUIET);
        let far = (target - self.lag_frames).abs() as f64 > LAG_MOVE_IN_SPEECH_SECONDS * rate;
        if !self.lane.started || quiet || far {
            self.lag_frames = target;
        }
        let delay = self.lag_frames + i64::from(c.chain_latency);
        self.voice_delay_ms = delay as f64 / rate * 1000.0;

        let voice_at = pos - delay;
        if !self.lane.started {
            self.lane.started = true;
            self.lane.base = voice_at.min(pos);
            self.voice_cursor = voice_at;
            self.sb_end = pos;
        }
        self.lane.add_at(pos, c.sb[..span].as_chunks::<1>().0);
        self.sb_end = self.sb_end.max(pos + span as i64);
        // A voice delay that grew overlaps what is already placed: those
        // frames are the new chain's priming and are dropped. One that shrank
        // leaves a gap, which stays silent.
        let skip = (self.voice_cursor - voice_at).max(0) as usize;
        if skip < span {
            self.lane.add_at(
                voice_at + skip as i64,
                c.voice[skip..span].as_chunks::<1>().0,
            );
        }
        self.voice_cursor = self.voice_cursor.max(voice_at + span as i64);
        self.sync_ready();
        outcome
    }
}

/// A Kaiser-windowed sinc, tabulated: band-limited interpolation at any
/// fractional position, low-passed below the lower of the two Nyquists.
struct Kernel {
    half: usize,
    /// `PHASES + 1` rows of `2 · half` coefficients.
    rows: Vec<f32>,
}

impl Kernel {
    fn new(rate: u32) -> Self {
        let scale = (f64::from(OUT_RATE) / f64::from(rate.max(1))).min(1.0);
        let half = (HALF_TAPS as f64 / scale).ceil() as usize;
        let taps = 2 * half;
        // Cut-off in cycles per source frame: just under the output's (or
        // the source's) Nyquist, leaving the window room to roll off.
        let fc = 0.5 * scale * 0.90;
        let beta = 8.0;
        let i0_beta = bessel_i0(beta);
        let mut rows = Vec::with_capacity((PHASES + 1) * taps);
        for p in 0..=PHASES {
            let f = p as f64 / PHASES as f64;
            let start = rows.len();
            let mut sum = 0.0f64;
            let row: Vec<f64> = (0..taps)
                .map(|k| {
                    let t = k as f64 - half as f64 + 1.0 - f;
                    let u = t / half as f64;
                    let window = if u.abs() < 1.0 {
                        bessel_i0(beta * (1.0 - u * u).sqrt()) / i0_beta
                    } else {
                        0.0
                    };
                    let x = 2.0 * fc * t;
                    let sinc = if x.abs() < 1e-12 {
                        1.0
                    } else {
                        (std::f64::consts::PI * x).sin() / (std::f64::consts::PI * x)
                    };
                    let h = 2.0 * fc * sinc * window;
                    sum += h;
                    h
                })
                .collect();
            // Unity gain at DC on every phase, so a constant stays constant.
            rows.extend(row.iter().map(|h| (h / sum) as f32));
            debug_assert_eq!(rows.len() - start, taps);
        }
        Self { half, rows }
    }

    /// The signal in `src` (whose first frame sits at `base`; silence outside
    /// it) at position `x`, every channel through the same coefficients.
    fn at<const C: usize>(&self, src: &VecDeque<[f32; C]>, base: i64, x: f64) -> [f32; C] {
        let i = x.floor();
        let p = (x - i) * PHASES as f64;
        let p0 = (p.floor() as usize).min(PHASES - 1);
        let w = (p - p0 as f64) as f32;
        let taps = 2 * self.half;
        let r0 = &self.rows[p0 * taps..(p0 + 1) * taps];
        let r1 = &self.rows[(p0 + 1) * taps..(p0 + 2) * taps];
        let first = i as i64 - self.half as i64 + 1;
        let len = src.len() as i64;
        let mut acc = [0.0f32; C];
        for k in 0..taps {
            let at = first + k as i64 - base;
            if at < 0 || at >= len {
                continue;
            }
            let c = r0[k] + (r1[k] - r0[k]) * w;
            for (a, s) in acc.iter_mut().zip(&src[at as usize]) {
                *a += c * s;
            }
        }
        acc
    }
}

/// The zeroth-order modified Bessel function of the first kind, by its
/// series — converges fast for the Kaiser window's arguments.
fn bessel_i0(x: f64) -> f64 {
    let mut sum = 1.0;
    let mut term = 1.0;
    let half = x / 2.0;
    for k in 1..64 {
        let f = half / f64::from(k);
        term *= f * f;
        sum += term;
        if term < sum * 1e-17 {
            break;
        }
    }
    sum
}

#[cfg(test)]
mod tests {
    use super::{
        soft_clip, to_i16, AudioBlock, AudioChunk, AudioTimeline, DesktopChunk, StemBlock,
        TrackOptions, BLOCK_FRAMES, DESKTOP_WAIT_SECONDS, OUT_RATE, SOFT_KNEE,
    };
    use std::f64::consts::TAU;

    /// A pretend engine: buffers of `span` frames at `rate`, on a clock
    /// `ppm` fast, the voice `wait + chain` late.
    struct Engine {
        epoch: u64,
        rate: u32,
        ppm: f64,
        span: usize,
        /// Take-clock time of frame 0.
        t0: f64,
        index: u64,
        wait: f64,
        /// Added to the wait the engine REPORTS, alternately + and −, without
        /// moving the voice: the measurement jitters, the voice does not.
        wait_jitter: f64,
        chain: u32,
    }

    impl Engine {
        fn new(rate: u32, t0: f64) -> Self {
            Self {
                epoch: 1,
                rate,
                ppm: 0.0,
                span: (rate / 100) as usize,
                t0,
                index: 0,
                wait: 0.0,
                wait_jitter: 0.0,
                chain: 0,
            }
        }

        /// When frame `n` of this epoch sounds, on the take's clock.
        fn time(&self, n: u64) -> f64 {
            self.t0 + n as f64 / (f64::from(self.rate) * (1.0 + self.ppm * 1e-6))
        }

        /// The next buffer: the soundboard as `sb(t)`, the voice as `voice(t)`
        /// captured `wait + chain` before it plays.
        fn chunk(&mut self, sb: impl Fn(f64) -> f32, voice: impl Fn(f64) -> f32) -> AudioChunk {
            let delay = self.wait + f64::from(self.chain) / f64::from(self.rate);
            let n0 = self.index;
            let sbv = (0..self.span as u64)
                .map(|i| sb(self.time(n0 + i)))
                .collect();
            let vv = (0..self.span as u64)
                .map(|i| voice(self.time(n0 + i) - delay))
                .collect();
            let c = AudioChunk {
                epoch: self.epoch,
                at: self.time(n0),
                index: n0,
                rate: self.rate,
                out_frames: 480,
                out_rate: 48_000,
                voice_wait: if (n0 / self.span as u64).is_multiple_of(2) {
                    self.wait + self.wait_jitter
                } else {
                    self.wait - self.wait_jitter
                },
                chain_latency: self.chain,
                voice: vv,
                sb: sbv,
            };
            self.index += self.span as u64;
            c
        }

        /// Skip `frames` as if their buffers were dropped.
        fn drop_frames(&mut self, frames: u64) {
            self.index += frames;
        }
    }

    fn mono(blocks: &[AudioBlock]) -> Vec<f32> {
        blocks
            .iter()
            .flat_map(|b| {
                b.samples.as_chunks::<2>().0.iter().map(|lr| {
                    // Without desktop audio the voice sits in the middle.
                    assert_eq!(lr[0], lr[1], "a voice-only track is not centred");
                    f32::from(lr[0]) / 32767.0
                })
            })
            .collect()
    }

    /// Run `seconds` of engine buffers through a timeline, pumping as a live
    /// recorder would, and return the track.
    fn record(
        engine: &mut Engine,
        seconds: f64,
        sb: impl Fn(f64) -> f32,
        voice: impl Fn(f64) -> f32,
    ) -> Vec<f32> {
        let mut tl = AudioTimeline::new();
        let mut blocks = Vec::new();
        while engine.time(engine.index) < seconds {
            let c = engine.chunk(&sb, &voice);
            let now = c.at + 0.01;
            tl.push(&c);
            blocks.extend(tl.pump(now));
        }
        blocks.extend(tl.finish(seconds));
        mono(&blocks)
    }

    /// Error of `track` against `truth(t)` over [from, to), in dB below the
    /// truth's own power.
    fn error_db(track: &[f32], truth: impl Fn(f64) -> f64, from: f64, to: f64) -> f64 {
        let a = (from * f64::from(OUT_RATE)) as usize;
        let b = ((to * f64::from(OUT_RATE)) as usize).min(track.len());
        let (mut err, mut sig) = (0.0, 0.0);
        for (j, &y) in track.iter().enumerate().take(b).skip(a) {
            let want = truth(j as f64 / f64::from(OUT_RATE));
            err += (f64::from(y) - want).powi(2);
            sig += want * want;
        }
        10.0 * (err / sig).log10()
    }

    fn tone(freq: f64, amp: f64) -> impl Fn(f64) -> f32 {
        move |t| (amp * (TAU * freq * t).sin()) as f32
    }

    /// Where the loudest sample is, in seconds.
    fn peak_time(track: &[f32]) -> f64 {
        let (j, _) = track
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.abs().total_cmp(&b.1.abs()))
            .unwrap();
        j as f64 / f64::from(OUT_RATE)
    }

    /// A band-limited click centred on `at`.
    fn click(at: f64) -> impl Fn(f64) -> f32 {
        move |t| {
            let d = (t - at) * 4_000.0;
            (0.8 * (-d * d).exp()) as f32
        }
    }

    #[test]
    fn a_48k_soundboard_comes_through_on_time() {
        let mut e = Engine::new(48_000, 0.0);
        let track = record(&mut e, 4.0, tone(440.0, 0.5), |_| 0.0);
        let db = error_db(&track, |t| 0.5 * (TAU * 440.0 * t).sin(), 0.5, 3.5);
        assert!(db < -60.0, "error {db:.1} dB");
    }

    #[test]
    fn other_engine_rates_are_resampled_to_48k() {
        for rate in [44_100, 96_000, 16_000] {
            let mut e = Engine::new(rate, 0.0);
            let track = record(&mut e, 3.0, tone(440.0, 0.5), |_| 0.0);
            let db = error_db(&track, |t| 0.5 * (TAU * 440.0 * t).sin(), 0.5, 2.5);
            assert!(db < -55.0, "{rate} Hz: error {db:.1} dB");
        }
    }

    /// The voice arrives late by its wait and its chain's latency, and is
    /// put back where it was spoken — a 256 ms voice model included.
    #[test]
    fn the_voice_is_placed_when_it_was_spoken() {
        let mut e = Engine::new(48_000, 0.0);
        e.wait = 0.023;
        e.chain = 12_288; // 256 ms
        let track = record(&mut e, 3.0, |_| 0.0, click(1.5));
        let at = peak_time(&track);
        assert!((at - 1.5).abs() < 0.0005, "voice click at {at:.4} s");
    }

    #[test]
    fn the_soundboard_is_placed_when_it_played() {
        let mut e = Engine::new(44_100, 0.0);
        e.wait = 0.03;
        e.chain = 4_410;
        let track = record(&mut e, 3.0, click(2.0), |_| 0.0);
        let at = peak_time(&track);
        assert!((at - 2.0).abs() < 0.0005, "soundboard click at {at:.4} s");
    }

    /// Both together, voice and soundboard each on their own time.
    #[test]
    fn voice_and_soundboard_sum_on_their_own_times() {
        let mut e = Engine::new(48_000, 0.0);
        // Delays that are not whole periods of either tone, or a misplaced
        // voice would line up with itself and pass. Whole FRAMES, though: the
        // voice is placed to the nearest frame (±10 µs, nothing next to a
        // 40 ms budget), and a waveform match at 1 kHz would see the rest.
        e.wait = 1_021.0 / 48_000.0;
        e.chain = 2_431;
        let track = record(&mut e, 4.0, tone(300.0, 0.3), tone(1_013.0, 0.3));
        let truth = |t: f64| 0.3 * (TAU * 300.0 * t).sin() + 0.3 * (TAU * 1_013.0 * t).sin();
        let db = error_db(&track, truth, 1.0, 3.5);
        assert!(db < -50.0, "error {db:.1} dB");
    }

    /// The measured wait swings ±5 ms buffer to buffer. The voice follows the
    /// smoothed figure in the pause before it speaks and lands on time.
    #[test]
    fn a_jittery_wait_still_places_the_voice() {
        let mut e = Engine::new(48_000, 0.0);
        e.wait = 0.020;
        e.wait_jitter = 0.005;
        let track = record(&mut e, 5.0, |_| 0.0, click(4.0));
        let at = peak_time(&track);
        assert!((at - 4.0).abs() < 0.0005, "voice click at {at:.4} s");
    }

    /// ...and while it speaks, the jitter never moves it: a moved voice is a
    /// step in the waveform, which a pure tone makes easy to see.
    #[test]
    fn a_jittery_wait_never_moves_a_speaking_voice() {
        let mut e = Engine::new(48_000, 0.0);
        e.wait = 0.020;
        e.wait_jitter = 0.005;
        let speak = |t: f64| {
            if t < 0.5 {
                0.0
            } else {
                (0.3 * (TAU * 997.0 * t).sin()) as f32
            }
        };
        let track = record(&mut e, 4.0, |_| 0.0, speak);
        // A sine's second difference is bounded by A·ω²; one skipped or
        // repeated frame is roughly ten times that.
        let bound = 0.3 * (TAU * 997.0 / 48_000.0).powi(2) * 1.5;
        let worst = track[(1.0 * 48_000.0) as usize..(3.9 * 48_000.0) as usize]
            .windows(3)
            .map(|w| f64::from(w[0] - 2.0 * w[1] + w[2]).abs())
            .fold(0.0, f64::max);
        assert!(worst < bound, "a discontinuity: {worst:.4} > {bound:.4}");
    }

    /// 80 ppm is 48 ms over ten minutes; the fit takes it out. Checked by a
    /// click near the end of a long take.
    #[test]
    fn a_drifting_clock_stays_in_sync() {
        let mut e = Engine::new(48_000, 0.0);
        e.ppm = 80.0;
        e.span = 960;
        let track = record(&mut e, 300.0, click(295.0), |_| 0.0);
        let at = peak_time(&track);
        assert!(
            (at - 295.0).abs() < 0.001,
            "click at {at:.4} s after 295 s of +80 ppm"
        );
    }

    /// Dropped buffers are silence of exactly their length; what follows is
    /// still on time.
    #[test]
    fn drops_become_silence_and_the_rest_stays_on_time() {
        let mut e = Engine::new(48_000, 0.0);
        let sb = tone(440.0, 0.5);
        let mut tl = AudioTimeline::new();
        let mut blocks = Vec::new();
        while e.time(e.index) < 3.0 {
            if (e.index..e.index + 480).contains(&72_000) {
                e.drop_frames(4_800); // 100 ms gone at 1.5 s
                continue;
            }
            let c = e.chunk(&sb, |_| 0.0);
            tl.push(&c);
            blocks.extend(tl.pump(c.at));
        }
        blocks.extend(tl.finish(3.0));
        let track = mono(&blocks);
        let silent = &track[(1.52 * 48_000.0) as usize..(1.58 * 48_000.0) as usize];
        assert!(
            silent.iter().all(|s| s.abs() < 1e-3),
            "the drop is not silent"
        );
        let db = error_db(&track, |t| 0.5 * (TAU * 440.0 * t).sin(), 1.7, 2.9);
        assert!(db < -55.0, "after the drop: error {db:.1} dB");
        assert_eq!(tl.stats().steps, 0, "a counted drop is not a step");
    }

    /// The device stalls for 100 ms: no buffers, and the index does not move
    /// either. The stall is kept as silence and the audio after it is on time.
    #[test]
    fn a_stall_is_kept_not_averaged() {
        let mut e = Engine::new(48_000, 0.0);
        let sb = tone(440.0, 0.5);
        let mut tl = AudioTimeline::new();
        let mut blocks = Vec::new();
        let mut stalled = false;
        loop {
            let c = e.chunk(&sb, |_| 0.0);
            if c.at >= 4.0 {
                break;
            }
            tl.push(&c);
            blocks.extend(tl.pump(c.at));
            if !stalled && c.at > 2.0 {
                stalled = true;
                e.t0 += 0.1;
            }
        }
        blocks.extend(tl.finish(4.0));
        let track = mono(&blocks);
        assert_eq!(tl.stats().steps, 1);
        // `sb(t)` is evaluated on the stalled clock, so the truth after the
        // stall is the same tone, still at its own time.
        let db = error_db(&track, |t| 0.5 * (TAU * 440.0 * t).sin(), 2.4, 3.9);
        assert!(db < -55.0, "after the stall: error {db:.1} dB");
    }

    /// No engine at all — stopped before the take — still makes a track:
    /// silence, block after block, once each block is old enough.
    #[test]
    fn no_audio_is_silence_not_a_stall() {
        let mut tl = AudioTimeline::new();
        assert!(tl.pump(0.5).is_empty(), "wrote audio that might still come");
        let blocks = tl.pump(3.0);
        let frames = blocks.len() * BLOCK_FRAMES;
        assert!(frames as f64 >= 1.9 * 48_000.0);
        assert!(mono(&blocks).iter().all(|&s| s == 0.0));
        for w in blocks.windows(2) {
            assert_eq!(
                w[0].pts + w[0].duration,
                w[1].pts,
                "blocks are not contiguous"
            );
        }
    }

    /// The engine is rebuilt mid-take on a device at another rate: the new
    /// epoch continues on time.
    #[test]
    fn a_rebuild_at_a_new_rate_continues_on_time() {
        let sb = tone(440.0, 0.5);
        let mut tl = AudioTimeline::new();
        let mut blocks = Vec::new();
        let mut a = Engine::new(48_000, 0.0);
        while a.time(a.index) < 1.5 {
            let c = a.chunk(&sb, |_| 0.0);
            tl.push(&c);
            blocks.extend(tl.pump(c.at));
        }
        // 300 ms of nothing while the device comes back, at 44.1 kHz.
        let mut b = Engine::new(44_100, 1.8);
        b.epoch = 2;
        while b.time(b.index) < 3.5 {
            let c = b.chunk(&sb, |_| 0.0);
            tl.push(&c);
            blocks.extend(tl.pump(c.at));
        }
        blocks.extend(tl.finish(3.5));
        let track = mono(&blocks);
        let gap = &track[(1.55 * 48_000.0) as usize..(1.75 * 48_000.0) as usize];
        assert!(gap.iter().all(|s| s.abs() < 1e-3), "the gap is not silent");
        let db = error_db(&track, |t| 0.5 * (TAU * 440.0 * t).sin(), 2.0, 3.4);
        assert!(db < -55.0, "after the rebuild: error {db:.1} dB");
    }

    /// Audio from before the take started is not in it.
    #[test]
    fn audio_before_the_take_is_cut() {
        let mut e = Engine::new(48_000, -2.0);
        let track = record(&mut e, 2.0, click(-1.0), |_| 0.0);
        assert!(track.iter().all(|s| s.abs() < 1e-3));
    }

    /// FNV-1a over every block's timing and samples: one number for a whole
    /// track, so a test can say "exactly this track" without a fixture file.
    fn fingerprint(blocks: &[AudioBlock]) -> u64 {
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        let mut eat = |bytes: &[u8]| {
            for &b in bytes {
                h ^= u64::from(b);
                h = h.wrapping_mul(0x0100_0000_01b3);
            }
        };
        for b in blocks {
            eat(&b.pts.to_le_bytes());
            eat(&b.duration.to_le_bytes());
            for s in &b.samples {
                eat(&s.to_le_bytes());
            }
        }
        h
    }

    /// Run `seconds` of `engine` through a timeline as `record` does, and
    /// return the blocks themselves.
    fn blocks_of(
        engine: &mut Engine,
        seconds: f64,
        sb: impl Fn(f64) -> f32,
        voice: impl Fn(f64) -> f32,
    ) -> Vec<AudioBlock> {
        let mut tl = AudioTimeline::new();
        let mut blocks = Vec::new();
        while engine.time(engine.index) < seconds {
            let c = engine.chunk(&sb, &voice);
            let now = c.at + 0.01;
            tl.push(&c);
            blocks.extend(tl.pump(now));
        }
        blocks.extend(tl.finish(seconds));
        blocks
    }

    /// A take with no desktop audio is exactly what v1.52.0 wrote, sample
    /// for sample: these fingerprints were taken from v1.52.0's timeline
    /// before it learned a second source. Voice, soundboard, other rates,
    /// drift, jitter and a clipping sum each take their own path through it.
    #[test]
    fn voice_only_tracks_are_unchanged_since_v1_52() {
        let mut cases: Vec<(&str, u64)> = Vec::new();
        let mut e = Engine::new(48_000, 0.0);
        e.wait = 1_021.0 / 48_000.0;
        e.chain = 2_431;
        cases.push((
            "48k voice + soundboard",
            fingerprint(&blocks_of(
                &mut e,
                2.0,
                tone(300.0, 0.3),
                tone(1_013.0, 0.3),
            )),
        ));
        let mut e = Engine::new(44_100, 0.0);
        e.wait = 0.03;
        e.chain = 4_410;
        cases.push((
            "44.1k clicks",
            fingerprint(&blocks_of(&mut e, 2.0, click(1.0), click(1.5))),
        ));
        let mut e = Engine::new(96_000, 0.013);
        e.ppm = 80.0;
        cases.push((
            "96k drifting",
            fingerprint(&blocks_of(&mut e, 3.0, tone(440.0, 0.5), |_| 0.0)),
        ));
        let mut e = Engine::new(48_000, 0.0);
        e.wait = 0.020;
        e.wait_jitter = 0.005;
        cases.push((
            "jittery wait",
            fingerprint(&blocks_of(&mut e, 2.0, |_| 0.0, tone(997.0, 0.4))),
        ));
        let mut e = Engine::new(48_000, 0.0);
        cases.push((
            "a sum past full scale",
            fingerprint(&blocks_of(&mut e, 1.0, tone(220.0, 0.9), tone(330.0, 0.9))),
        ));
        let got: Vec<String> = cases
            .iter()
            .map(|(n, h)| format!("{n}: {h:#018x}"))
            .collect();
        let want = [
            "48k voice + soundboard: 0xca19a0077409ecdb",
            "44.1k clicks: 0x1c7c3b1c10f9684b",
            "96k drifting: 0xdc9e2855b758d128",
            "jittery wait: 0x377b2220aea001cf",
            "a sum past full scale: 0x3108ca4e1fc5e4c3",
        ];
        assert_eq!(got, want);
    }

    /// Deterministic noise of time, with no libm in it: an LCG hash of the
    /// time in microseconds. Every sample differs, so a track slid by even
    /// one frame — or reweighted by one ulp — hashes differently.
    fn noise(seed: u64, amp: f32) -> impl Fn(f64) -> f32 {
        move |t| {
            let k = (t * 1_000_000.0).round() as i64 as u64;
            let x = k
                .wrapping_add(seed)
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            let x = (x ^ (x >> 29)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
            ((x >> 40) as f32 / 16_777_216.0 * 2.0 - 1.0) * amp
        }
    }

    /// A track and its final stats, fed `chunks` in order and pumped every
    /// `cadence` seconds of their time, as the recorder would.
    fn golden(chunks: &[AudioChunk], cadence: f64, end: f64) -> String {
        let mut tl = AudioTimeline::new();
        let mut blocks = Vec::new();
        let mut next_pump = f64::NEG_INFINITY;
        for c in chunks {
            tl.push(c);
            if c.at >= next_pump {
                blocks.extend(tl.pump(c.at + 0.01));
                next_pump = c.at + cadence;
            }
        }
        blocks.extend(tl.finish(end));
        let s = tl.stats();
        format!(
            "{:#018x} blocks={} steps={} waited={} drift={:#x} delay={:#x}",
            fingerprint(&blocks),
            blocks.len(),
            s.steps,
            s.waited_frames,
            s.drift_ppm.to_bits(),
            s.voice_delay_ms.to_bits()
        )
    }

    fn chunks_until(e: &mut Engine, until: f64, sb: f32, voice: f32) -> Vec<AudioChunk> {
        let mut out = Vec::new();
        while e.time(e.index) < until {
            out.push(e.chunk(noise(e.epoch * 7 + 1, sb), noise(e.epoch * 7 + 2, voice)));
        }
        out
    }

    /// The same, on the paths the tones above do not reach: a 256 ms chain
    /// with the soundboard still sounding in its tail when the take ends;
    /// drops; a stall; a rebuild at another rate with a straggler from the
    /// old epoch; a clock that jumps back; no engine at all for 1.5 s; sums
    /// over full scale; pumps every 5, 20 and 150 ms. Fingerprints and final
    /// stats from v1.52.0's timeline, as above.
    #[test]
    fn voice_only_edge_cases_are_unchanged_since_v1_52() {
        let mut got = Vec::new();

        let mut e = Engine::new(48_000, 0.0);
        e.wait = 0.021;
        e.chain = 12_288;
        got.push(golden(&chunks_until(&mut e, 2.0, 0.5, 0.5), 0.02, 2.0));

        let mut e = Engine::new(44_100, 0.004);
        let mut c = chunks_until(&mut e, 1.0, 0.9, 0.6);
        e.drop_frames(4_410);
        c.extend(chunks_until(&mut e, 2.5, 0.9, 0.6));
        got.push(golden(&c, 0.005, 2.5));

        let mut e = Engine::new(16_000, 0.0);
        let mut c = chunks_until(&mut e, 1.2, 0.4, 0.4);
        e.t0 += 0.1;
        c.extend(chunks_until(&mut e, 3.0, 0.4, 0.4));
        got.push(golden(&c, 0.15, 3.0));

        let mut a = Engine::new(96_000, 0.0);
        a.wait = 0.03;
        a.chain = 4_800;
        let mut c = chunks_until(&mut a, 1.5, 0.7, 0.7);
        let straggler = a.chunk(noise(9, 0.7), noise(10, 0.7));
        let mut b = Engine::new(44_100, 1.7);
        b.epoch = 2;
        b.wait = 0.01;
        let mut rest = chunks_until(&mut b, 3.0, 0.7, 0.7);
        rest.insert(1, straggler);
        c.extend(rest);
        got.push(golden(&c, 0.02, 3.0));

        let mut e = Engine::new(48_000, 0.0);
        let mut c = chunks_until(&mut e, 1.5, 0.5, 0.5);
        e.t0 -= 0.05;
        c.extend(chunks_until(&mut e, 3.0, 0.5, 0.5));
        got.push(golden(&c, 0.02, 3.0));

        let mut e = Engine::new(48_000, 1.5);
        e.wait = 0.015;
        got.push(golden(&chunks_until(&mut e, 3.0, 0.6, 0.6), 0.02, 3.0));

        let want: [&str; 6] = [
            "0xfa58903bda399367 blocks=94 steps=0 waited=0 drift=0x0 delay=0x4071500000000000",
            "0x450d224fec94c3a8 blocks=118 steps=0 waited=0 drift=0xbe1312d000000000 delay=0x0",
            "0xb253818f5aae65cc blocks=141 steps=1 waited=0 drift=0xbdee848000000000 delay=0x0",
            "0xefc3d460cfbe8ad0 blocks=141 steps=0 waited=0 drift=0x0 delay=0x4024000000000000",
            "0x3d94b04bef6c765c blocks=141 steps=1 waited=0 drift=0x0 delay=0x0",
            "0x98ca4af08ceb0878 blocks=141 steps=0 waited=0 drift=0x0 delay=0x402e000000000000",
        ];
        assert_eq!(got, want);
    }

    #[test]
    fn finish_pads_to_a_whole_block_past_the_end() {
        let mut e = Engine::new(48_000, 0.0);
        let mut tl = AudioTimeline::new();
        let mut blocks = Vec::new();
        while e.time(e.index) < 1.0 {
            let c = e.chunk(tone(440.0, 0.5), |_| 0.0);
            tl.push(&c);
            blocks.extend(tl.pump(c.at));
        }
        blocks.extend(tl.finish(1.0));
        let frames = blocks.len() * BLOCK_FRAMES;
        assert!((48_000..48_000 + BLOCK_FRAMES).contains(&frames));
        assert_eq!(blocks[0].pts, 0);
        assert_eq!(blocks[1].pts, 213_333); // 1024 frames at 48 kHz, in 100 ns
    }

    // ---- Desktop audio (v1.53.0) ----

    /// A pretend loopback device: 10 ms packets of stereo at `rate`, on a
    /// clock `ppm` fast, each stamped by Windows with when its first frame
    /// was captured — give or take `jitter`, alternately early and late.
    /// It sends nothing during `idle` stretches (an idle playback device).
    struct Loopback {
        stream: u64,
        rate: u32,
        ppm: f64,
        t0: f64,
        index: u64,
        jitter: f64,
        idle: Vec<(f64, f64)>,
        packets: u64,
    }

    impl Loopback {
        fn new(t0: f64) -> Self {
            Self {
                stream: 1,
                rate: 48_000,
                ppm: 0.0,
                t0,
                index: 0,
                jitter: 0.0,
                idle: Vec::new(),
                packets: 0,
            }
        }

        fn span(&self) -> u64 {
            u64::from(self.rate / 100)
        }

        /// When frame `n` sounded, on the take's clock.
        fn time(&self, n: u64) -> f64 {
            self.t0 + n as f64 / (f64::from(self.rate) * (1.0 + self.ppm * 1e-6))
        }

        /// The next packet, whether or not it would be sent.
        fn packet(&mut self, sound: &impl Fn(f64) -> [f32; 2]) -> DesktopChunk {
            let n0 = self.index;
            let mut frames = Vec::new();
            for i in 0..self.span() {
                frames.extend(sound(self.time(n0 + i)));
            }
            let wobble = if self.packets.is_multiple_of(2) {
                self.jitter
            } else {
                -self.jitter
            };
            self.index += self.span();
            self.packets += 1;
            DesktopChunk {
                stream: self.stream,
                at: self.time(n0) + wobble,
                timed: true,
                discontinuity: false,
                resumed: false,
                rate: self.rate,
                frames,
            }
        }

        /// Every packet completed by `t`, less those the device never sent.
        fn until(&mut self, t: f64, sound: &impl Fn(f64) -> [f32; 2]) -> Vec<DesktopChunk> {
            let mut out = Vec::new();
            while self.time(self.index + self.span()) <= t {
                let at = self.time(self.index);
                let c = self.packet(sound);
                if !self.idle.iter().any(|&(a, b)| at >= a && at < b) {
                    out.push(c);
                }
            }
            out
        }
    }

    impl Engine {
        /// Every buffer completed by `t`.
        fn until(
            &mut self,
            t: f64,
            sb: &impl Fn(f64) -> f32,
            voice: &impl Fn(f64) -> f32,
        ) -> Vec<AudioChunk> {
            let mut out = Vec::new();
            while self.time(self.index + self.span as u64) <= t {
                out.push(self.chunk(sb, voice));
            }
            out
        }
    }

    /// Run a take the way the recorder does: every 10 ms of take time, what
    /// each source has produced by then goes in and the track is pumped.
    fn drive(
        tl: &mut AudioTimeline,
        seconds: f64,
        mut engine: impl FnMut(f64) -> Vec<AudioChunk>,
        mut desktop: impl FnMut(f64) -> Vec<DesktopChunk>,
    ) -> Vec<AudioBlock> {
        let mut blocks = Vec::new();
        let mut t = 0.0;
        while t < seconds {
            t += 0.01;
            for c in engine(t) {
                tl.push(&c);
            }
            for c in desktop(t) {
                tl.push_desktop(&c);
            }
            blocks.extend(tl.pump(t));
        }
        blocks.extend(tl.finish(seconds));
        blocks
    }

    fn with_desktop(gain_db: f32) -> AudioTimeline {
        AudioTimeline::with_options(TrackOptions {
            desktop_gain_db: Some(gain_db),
            stems: false,
        })
    }

    /// Left and right, as floats.
    fn stereo(blocks: &[AudioBlock]) -> (Vec<f32>, Vec<f32>) {
        blocks
            .iter()
            .flat_map(|b| b.samples.as_chunks::<2>().0.iter())
            .map(|lr| (f32::from(lr[0]) / 32767.0, f32::from(lr[1]) / 32767.0))
            .unzip()
    }

    fn silence(_: f64) -> f32 {
        0.0
    }

    fn sides(l: impl Fn(f64) -> f32, r: impl Fn(f64) -> f32) -> impl Fn(f64) -> [f32; 2] {
        move |t| [l(t), r(t)]
    }

    /// A take of the engine's silence with desktop audio playing `sound`.
    fn desktop_take(
        seconds: f64,
        gain_db: f32,
        desk: &mut Loopback,
        sound: impl Fn(f64) -> [f32; 2],
    ) -> (AudioTimeline, Vec<AudioBlock>) {
        let mut tl = with_desktop(gain_db);
        let mut e = Engine::new(48_000, 0.0);
        let blocks = drive(
            &mut tl,
            seconds,
            |t| e.until(t, &silence, &silence),
            |t| desk.until(t, &sound),
        );
        (tl, blocks)
    }

    /// Desktop audio lands when it was captured, in stereo, untouched at
    /// 0 dB: left and right each carry their own tone.
    #[test]
    fn desktop_audio_comes_through_on_time_in_stereo() {
        let mut desk = Loopback::new(0.012_3);
        let (_, blocks) = desktop_take(
            3.0,
            0.0,
            &mut desk,
            sides(tone(440.0, 0.4), tone(660.0, 0.4)),
        );
        let (l, r) = stereo(&blocks);
        let dl = error_db(&l, |t| 0.4 * (TAU * 440.0 * t).sin(), 0.5, 2.5);
        let dr = error_db(&r, |t| 0.4 * (TAU * 660.0 * t).sin(), 0.5, 2.5);
        assert!(
            dl < -60.0 && dr < -60.0,
            "left {dl:.1} dB, right {dr:.1} dB"
        );
    }

    /// A click on the left is on the left, and one on the right on the right.
    #[test]
    fn desktop_channels_are_not_swapped() {
        let mut desk = Loopback::new(0.0);
        let (_, blocks) = desktop_take(3.0, 0.0, &mut desk, sides(click(1.0), click(2.0)));
        let (l, r) = stereo(&blocks);
        let (lt, rt) = (peak_time(&l), peak_time(&r));
        assert!((lt - 1.0).abs() < 0.000_5, "left click at {lt:.4} s");
        assert!((rt - 2.0).abs() < 0.000_5, "right click at {rt:.4} s");
        let bleed = |x: &[f32], at: f64| {
            let j = (at * 48_000.0) as usize;
            x[j - 480..j + 480]
                .iter()
                .fold(0.0f32, |m, s| m.max(s.abs()))
        };
        assert!(
            bleed(&r, 1.0) < 1e-3 && bleed(&l, 2.0) < 1e-3,
            "a click leaked across"
        );
    }

    /// The voice, the soundboard and the desktop each on their own clock —
    /// the engine 80 ppm slow, the desktop 120 ppm fast — and after five
    /// minutes each still lands where it played.
    #[test]
    fn every_source_keeps_its_own_clock() {
        let mut tl = with_desktop(0.0);
        let mut e = Engine::new(48_000, 0.0);
        e.ppm = -80.0;
        e.span = 960;
        e.wait = 0.02;
        let mut desk = Loopback::new(0.003);
        desk.ppm = 120.0;
        let voice = click(295.0);
        let sound = sides(click(296.0), click(296.0));
        let blocks = drive(
            &mut tl,
            298.0,
            |t| e.until(t, &silence, &voice),
            |t| desk.until(t, &sound),
        );
        let (l, _) = stereo(&blocks);
        let near = |at: f64| {
            let a = ((at - 0.3) * 48_000.0) as usize;
            let b = ((at + 0.3) * 48_000.0) as usize;
            a as f64 / 48_000.0 + peak_time(&l[a..b])
        };
        let (v, d) = (near(295.0), near(296.0));
        assert!((v - 295.0).abs() < 0.001, "voice click at {v:.4} s");
        assert!((d - 296.0).abs() < 0.001, "desktop click at {d:.4} s");
        assert_eq!(tl.stats().desktop_runs, 1);
        let ppm = tl.stats().desktop_drift_ppm;
        assert!((ppm - 120.0).abs() < 10.0, "desktop drift {ppm:.1} ppm");
    }

    /// An idle playback device sends nothing; the gap is silence and what
    /// comes after it is on time, as a run of its own.
    #[test]
    fn an_idle_device_is_silence_and_resumes_on_time() {
        let mut desk = Loopback::new(0.0);
        desk.idle = vec![(1.0, 4.0)];
        let (tl, blocks) = desktop_take(
            6.0,
            0.0,
            &mut desk,
            sides(tone(440.0, 0.4), tone(440.0, 0.4)),
        );
        let (l, _) = stereo(&blocks);
        let gap = &l[(1.01 * 48_000.0) as usize..(3.99 * 48_000.0) as usize];
        assert!(
            gap.iter().all(|s| s.abs() < 1e-3),
            "the idle stretch is not silent"
        );
        let db = error_db(&l, |t| 0.4 * (TAU * 440.0 * t).sin(), 4.1, 5.9);
        assert!(db < -55.0, "after the idle stretch: {db:.1} dB");
        let before = error_db(&l, |t| 0.4 * (TAU * 440.0 * t).sin(), 0.2, 0.99);
        assert!(before < -55.0, "before it: {before:.1} dB");
        assert_eq!(tl.stats().desktop_runs, 2);
    }

    /// Ten minutes of nothing between two minutes of drifting audio: the
    /// run after it has its own fit and is not dragged off by the gap.
    #[test]
    fn a_long_idle_gap_does_not_bend_the_clock() {
        let mut desk = Loopback::new(0.0);
        desk.ppm = 100.0;
        desk.idle = vec![(60.0, 660.0)];
        let (_, blocks) = desktop_take(722.0, 0.0, &mut desk, sides(click(719.0), click(719.0)));
        let (l, _) = stereo(&blocks);
        let a = (718.0 * 48_000.0) as usize;
        let at = 718.0 + peak_time(&l[a..]);
        assert!((at - 719.0).abs() < 0.001, "click at {at:.4} s");
    }

    /// Windows' timestamps wobble a little; that is not a gap.
    #[test]
    fn timestamp_jitter_is_not_a_gap() {
        let mut desk = Loopback::new(0.0);
        desk.jitter = 0.002;
        let (tl, blocks) = desktop_take(4.0, 0.0, &mut desk, sides(click(3.0), click(3.0)));
        let (l, _) = stereo(&blocks);
        let at = peak_time(&l);
        assert!((at - 3.0).abs() < 0.000_5, "click at {at:.4} s");
        assert_eq!(tl.stats().desktop_runs, 1);
    }

    /// One 10 ms packet lost: its time is silence, and the rest stays put.
    #[test]
    fn a_lost_packet_leaves_its_own_length_of_silence() {
        let mut desk = Loopback::new(0.0);
        desk.idle = vec![(1.500, 1.505)];
        let (tl, blocks) = desktop_take(
            3.0,
            0.0,
            &mut desk,
            sides(tone(440.0, 0.4), tone(440.0, 0.4)),
        );
        let (l, _) = stereo(&blocks);
        let lost = &l[(1.501 * 48_000.0) as usize..(1.509 * 48_000.0) as usize];
        assert!(
            lost.iter().all(|s| s.abs() < 1e-3),
            "the lost packet is not silent"
        );
        let db = error_db(&l, |t| 0.4 * (TAU * 440.0 * t).sin(), 1.6, 2.9);
        assert!(db < -55.0, "after the loss: {db:.1} dB");
        assert_eq!(tl.stats().desktop_runs, 2);
        assert_eq!(tl.stats().desktop_gaps, 1);
        let gap = tl.stats().desktop_gap_seconds;
        assert!((gap - 0.01).abs() < 1e-6, "a {gap:.5} s gap");
    }

    /// Windows' "this does not follow on" flag starts a run even when the
    /// timestamp looks continuous — and where nothing was lost, the two runs
    /// meet without a seam: no step in a sine across the join.
    #[test]
    fn a_flagged_break_with_nothing_lost_has_no_seam() {
        let mut tl = with_desktop(0.0);
        let mut desk = Loopback::new(0.0);
        let sound = sides(tone(997.0, 0.3), tone(997.0, 0.3));
        for i in 0..300 {
            let mut c = desk.packet(&sound);
            c.discontinuity = i == 150;
            tl.push_desktop(&c);
        }
        assert_eq!(tl.stats().desktop_runs, 2);
        assert_eq!(tl.stats().desktop_gaps, 0);
        let blocks = tl.finish(3.0);
        let (l, _) = stereo(&blocks);
        // As in the voice test: a sine's second difference is bounded by
        // A·ω²; a dip or a skipped frame at the join is far over it.
        let bound = 0.3 * (TAU * 997.0 / 48_000.0).powi(2) * 1.5;
        let worst = l[(1.4 * 48_000.0) as usize..(1.6 * 48_000.0) as usize]
            .windows(3)
            .map(|w| f64::from(w[0] - 2.0 * w[1] + w[2]).abs())
            .fold(0.0, f64::max);
        assert!(worst < bound, "a seam at the join: {worst:.5} > {bound:.5}");
    }

    /// A playback device's first packet after a start can be stamped about
    /// 4 ms off the rest (measured on one DAC; 6 ms here, for margin), with
    /// the rest wobbling ±0.6 ms. That is neither a break nor a pull on the
    /// line: one run, and the audio where the steady stamps put it.
    #[test]
    fn a_device_s_odd_first_stamp_is_not_a_break() {
        let mut tl = with_desktop(0.0);
        let mut desk = Loopback::new(0.0);
        let sound = sides(click(0.5), click(0.5));
        for i in 0..300u32 {
            let mut c = desk.packet(&sound);
            if i == 0 {
                c.at -= 0.006;
            } else {
                c.at += if i % 2 == 0 { 0.0006 } else { -0.0006 };
            }
            tl.push_desktop(&c);
        }
        assert_eq!(tl.stats().desktop_runs, 1);
        assert_eq!(tl.stats().desktop_gaps, 0);
        let (l, _) = stereo(&tl.finish(3.0));
        let at = peak_time(&l);
        assert!((at - 0.5).abs() < 0.000_2, "click at {at:.5} s");
    }

    /// The same, as it happens in the app: the recorder pumps between
    /// packets, so the run is looked at before its first packets have
    /// settled where it sits. A device back from idle with its first packet
    /// stamped off by up to 6 ms still puts a click 30 ms into the run where
    /// it played — whatever the voice's delay holding the track back.
    #[test]
    fn a_resumed_device_s_odd_first_stamp_does_not_move_it_live() {
        for wait in [0.010, 0.050, 0.250] {
            for shift in [-0.006, -0.004, 0.004] {
                let mut tl = with_desktop(0.0);
                let mut e = Engine::new(48_000, 0.0);
                e.wait = wait;
                let mut desk = Loopback::new(0.0);
                desk.idle = vec![(1.0, 1.5)];
                let sound = sides(click(1.53), click(1.53));
                let mut first = true;
                let blocks = drive(
                    &mut tl,
                    2.5,
                    |t| e.until(t, &silence, &silence),
                    |t| {
                        let mut out = desk.until(t, &sound);
                        for c in &mut out {
                            if first && c.at >= 1.5 {
                                c.at += shift;
                                c.resumed = true;
                                first = false;
                            }
                        }
                        out
                    },
                );
                let (l, _) = stereo(&blocks);
                let a = (1.4 * 48_000.0) as usize;
                let at = 1.4 + peak_time(&l[a..]);
                assert!(
                    (at - 1.53).abs() < 0.000_2,
                    "voice wait {wait}, first stamp {shift:+}: click at {at:.5} s"
                );
                assert_eq!(tl.stats().desktop_gaps, 0, "a resume counted as lost audio");
                assert_eq!(tl.stats().desktop_runs, 2);
            }
        }
    }

    /// Audio from before the take is not "late", however much of it is
    /// waiting when the take's first frame comes; audio in the take that
    /// arrives after its place was written is.
    #[test]
    fn only_audio_in_the_take_can_be_late() {
        let mut tl = with_desktop(0.0);
        let mut desk = Loopback::new(-0.5);
        let sound = sides(tone(440.0, 0.4), tone(440.0, 0.4));
        for c in desk.until(0.2, &sound) {
            tl.push_desktop(&c);
        }
        // No engine: the track is written up to a second behind now.
        let _ = tl.pump(2.0);
        assert_eq!(
            tl.stats().desktop_late_frames,
            0,
            "pre-take audio counted late"
        );
        // A packet from 0.05 s, long since written, arrives now.
        let mut stray = Loopback::new(0.05);
        stray.stream = 2;
        let c = stray.packet(&sound);
        tl.push_desktop(&c);
        assert_eq!(tl.stats().desktop_late_frames, 480);
    }

    /// A playback device back from idle is a new run, not lost audio; the
    /// same jump without the device saying so is counted as a gap.
    #[test]
    fn a_resume_is_not_a_gap() {
        for resumed in [true, false] {
            let mut tl = with_desktop(0.0);
            let mut desk = Loopback::new(0.0);
            let sound = sides(tone(440.0, 0.4), tone(440.0, 0.4));
            for i in 0..100 {
                if i == 50 {
                    desk.index += 4_800; // 100 ms with nothing
                }
                let mut c = desk.packet(&sound);
                c.resumed = resumed && i == 50;
                tl.push_desktop(&c);
            }
            assert_eq!(tl.stats().desktop_runs, 2);
            assert_eq!(
                tl.stats().desktop_gaps,
                u64::from(!resumed),
                "resumed {resumed}"
            );
        }
    }

    /// Packets Windows could not time continue their run while their read
    /// times agree with it, and a hole in them — packets lost while the
    /// reader was held up — is a break, not audio slid early.
    #[test]
    fn untimed_packets_still_show_a_hole() {
        let mut tl = with_desktop(0.0);
        let mut desk = Loopback::new(0.0);
        desk.idle = vec![(1.5, 1.6)];
        let sound = sides(click(2.5), click(2.5));
        let mut t = 0.0;
        let mut k = 0u32;
        while t < 3.0 {
            t += 0.01;
            for mut c in desk.until(t, &sound) {
                // Read up to 9 ms after it was captured, never on time.
                c.timed = false;
                c.at += 0.010 + f64::from(k % 10) * 0.001;
                k += 1;
                tl.push_desktop(&c);
            }
        }
        assert_eq!(tl.stats().desktop_runs, 2);
        assert_eq!(tl.stats().desktop_gaps, 1);
        let gap = tl.stats().desktop_gap_seconds;
        assert!((gap - 0.1).abs() < 0.015, "a {gap:.3} s gap");
        let (l, _) = stereo(&tl.finish(3.0));
        let at = peak_time(&l);
        assert!((at - 2.5).abs() < 0.015, "click at {at:.4} s");
    }

    /// A packet Windows could not time, in a run it could: it continues the
    /// run wherever its estimate lands, adding nothing to the line.
    #[test]
    fn one_untimed_packet_does_not_move_a_timed_run() {
        let mut tl = with_desktop(0.0);
        let mut desk = Loopback::new(0.0);
        let sound = sides(tone(440.0, 0.4), tone(440.0, 0.4));
        for i in 0..300 {
            let mut c = desk.packet(&sound);
            if i == 200 {
                c.timed = false;
                c.at += 0.02; // read late, as an estimate is
            }
            tl.push_desktop(&c);
        }
        assert_eq!(tl.stats().desktop_runs, 1);
        let (l, _) = stereo(&tl.finish(3.0));
        let db = error_db(&l, |t| 0.4 * (TAU * 440.0 * t).sin(), 2.1, 2.9);
        assert!(db < -55.0, "after the untimed packet: {db:.1} dB");
    }

    /// Silence for a source is counted once, when its frame is written —
    /// not again each time the frame is looked at while the other source
    /// is still on its way.
    #[test]
    fn waited_frames_are_counted_once() {
        let mut tl = with_desktop(0.0);
        let mut desk = Loopback::new(0.0);
        let sound = sides(tone(440.0, 0.4), tone(440.0, 0.4));
        let mut written = 0;
        let mut t = 0.0;
        while t < 3.0 {
            t += 0.02;
            // Desktop audio 0.1 s behind; no engine at all.
            for c in desk.until(t - 0.1, &sound) {
                tl.push_desktop(&c);
            }
            written += tl.pump(t).len() * BLOCK_FRAMES;
        }
        let s = tl.stats();
        assert!(written > 0);
        // Every frame taken so far — written out, or in the block being
        // filled — was waited for once.
        assert_eq!(s.waited_frames, tl.next);
        assert!(tl.next - (written as u64) < BLOCK_FRAMES as u64);
        assert_eq!(s.desktop_waited_frames, 0);
    }

    /// Desktop audio that never comes holds nothing back: the voice is
    /// written as soon as it is in, the desktop side silent once it is
    /// `DESKTOP_WAIT_SECONDS` overdue.
    #[test]
    fn a_silent_desktop_never_holds_back_the_voice() {
        let mut tl = with_desktop(0.0);
        let mut e = Engine::new(48_000, 0.0);
        let voice = tone(440.0, 0.4);
        let mut blocks = Vec::new();
        let mut t = 0.0;
        while t < 3.0 {
            t += 0.01;
            for c in e.until(t, &silence, &voice) {
                tl.push(&c);
            }
            blocks.extend(tl.pump(t));
        }
        let written = (blocks.len() * BLOCK_FRAMES) as f64 / 48_000.0;
        assert!(
            written >= 3.0 - DESKTOP_WAIT_SECONDS - 0.05,
            "only {written:.2} s written"
        );
        let (l, _) = stereo(&blocks);
        let db = error_db(&l, |t| 0.4 * (TAU * 440.0 * t).sin(), 0.5, written - 0.05);
        assert!(db < -55.0, "voice {db:.1} dB");
        assert!(tl.stats().desktop_waited_frames > 0);
    }

    /// And a stopped engine holds desktop audio back no longer than it ever
    /// held the soundboard: a second.
    #[test]
    fn a_stopped_engine_never_holds_back_the_desktop() {
        let mut tl = with_desktop(0.0);
        let mut desk = Loopback::new(0.0);
        let sound = sides(tone(440.0, 0.4), tone(440.0, 0.4));
        let mut blocks = Vec::new();
        let mut t = 0.0;
        while t < 4.0 {
            t += 0.01;
            for c in desk.until(t, &sound) {
                tl.push_desktop(&c);
            }
            blocks.extend(tl.pump(t));
        }
        let written = (blocks.len() * BLOCK_FRAMES) as f64 / 48_000.0;
        assert!(written >= 4.0 - 1.0 - 0.05, "only {written:.2} s written");
        let (l, _) = stereo(&blocks);
        let db = error_db(&l, |t| 0.4 * (TAU * 440.0 * t).sin(), 0.5, written - 0.05);
        assert!(db < -55.0, "desktop {db:.1} dB");
    }

    /// Audio from before the take started is not in it.
    #[test]
    fn desktop_audio_before_the_take_is_cut() {
        let mut desk = Loopback::new(-2.0);
        let (_, blocks) = desktop_take(2.0, 0.0, &mut desk, sides(click(-1.0), click(-0.5)));
        let (l, r) = stereo(&blocks);
        assert!(l.iter().chain(&r).all(|s| s.abs() < 1e-3));
    }

    /// The knee: exactly linear up to −3 dBFS, then smooth, odd and rising,
    /// and short of full scale until well past twice it.
    #[test]
    fn the_soft_clip_curve() {
        for x in [0.0f32, 0.1, -0.5, SOFT_KNEE, -SOFT_KNEE] {
            assert_eq!(soft_clip(x).to_bits(), x.to_bits(), "{x} moved");
        }
        let mut last = 0.0f32;
        for i in 1..=2_000 {
            let x = i as f32 / 100.0;
            let y = soft_clip(x);
            assert!(y >= last && y <= 1.0, "not rising to 1 at {x}: {y}");
            if x <= 2.0 {
                assert!(y < 1.0, "full scale already at {x}");
            }
            assert_eq!(soft_clip(-x), -y, "not odd at {x}");
            last = y;
        }
        // Slope one either side of the knee: no corner.
        let h = 1e-3f32;
        let slope = (soft_clip(SOFT_KNEE + h) - soft_clip(SOFT_KNEE)) / h;
        assert!((slope - 1.0).abs() < 0.01, "slope {slope} at the knee");
    }

    /// Voice and desktop add up: under the knee the track is exactly their
    /// sum, and a sum nearly twice full scale still never reaches it.
    #[test]
    fn voice_and_desktop_add_and_bend_rather_than_clip() {
        let mix = |level: f32| {
            let mut tl = with_desktop(0.0);
            let mut e = Engine::new(48_000, 0.0);
            let mut desk = Loopback::new(0.0);
            let voice = tone(300.0, f64::from(level));
            let sound = sides(tone(200.0, f64::from(level)), tone(200.0, f64::from(level)));
            let blocks = drive(
                &mut tl,
                2.0,
                |t| e.until(t, &silence, &voice),
                |t| desk.until(t, &sound),
            );
            stereo(&blocks).0[48_000..(1.5 * 48_000.0) as usize].to_vec()
        };
        // 0.3 + 0.3 peaks at 0.6, under the knee: the plain sum, as an
        // unmixed track would carry it.
        let quiet = mix(0.3);
        let db = error_db(
            &quiet,
            |t| 0.3 * (TAU * 300.0 * (t + 1.0)).sin() + 0.3 * (TAU * 200.0 * (t + 1.0)).sin(),
            0.0,
            0.5,
        );
        assert!(db < -60.0, "the sum is off by {db:.1} dB");
        // 0.9 + 0.9: bent, loud, and never at full scale.
        let loud = mix(0.9);
        let top = loud.iter().fold(0.0f32, |m, s| m.max(s.abs()));
        assert!(top > 0.99 && top < 1.0, "peak {top}");
    }

    /// The desktop's gain applies to it alone: −6.02 dB halves it.
    #[test]
    fn the_desktop_gain_scales_the_desktop() {
        let mut desk = Loopback::new(0.0);
        let (_, blocks) = desktop_take(
            2.0,
            -6.020_6,
            &mut desk,
            sides(tone(440.0, 0.6), tone(440.0, 0.6)),
        );
        let (l, _) = stereo(&blocks);
        let db = error_db(&l, |t| 0.3 * (TAU * 440.0 * t).sin(), 0.5, 1.5);
        assert!(db < -55.0, "{db:.1} dB from half the level");
    }

    /// Stems are the track's own sources before the mix: one block each per
    /// track block, contiguous from 0, and the track is exactly their mix.
    #[test]
    fn stems_are_the_track_before_the_mix() {
        let mut tl = AudioTimeline::with_options(TrackOptions {
            desktop_gain_db: Some(-6.0),
            stems: true,
        });
        let mut e = Engine::new(48_000, 0.0);
        e.wait = 0.02;
        let mut desk = Loopback::new(0.004);
        let voice = tone(300.0, 0.5);
        let sound = sides(tone(500.0, 0.9), tone(700.0, 0.9));
        let mut blocks = Vec::new();
        let mut stems: Vec<StemBlock> = Vec::new();
        let mut t = 0.0;
        while t < 3.0 {
            t += 0.01;
            for c in e.until(t, &silence, &voice) {
                tl.push(&c);
            }
            for c in desk.until(t, &sound) {
                tl.push_desktop(&c);
            }
            blocks.extend(tl.pump(t));
            stems.extend(tl.take_stems());
        }
        blocks.extend(tl.finish(3.0));
        stems.extend(tl.take_stems());
        assert_eq!(stems.len(), blocks.len());
        let g = 10f32.powf(-6.0 / 20.0);
        for (b, s) in blocks.iter().zip(&stems) {
            assert_eq!(b.pts, s.pts, "a stem block out of step");
            let d = s.desktop.as_ref().expect("a desktop stem");
            assert_eq!(s.voice.len(), BLOCK_FRAMES);
            assert_eq!(d.len(), 2 * BLOCK_FRAMES);
            for (k, lr) in b.samples.as_chunks::<2>().0.iter().enumerate() {
                for ch in 0..2 {
                    let want = to_i16(soft_clip(s.voice[k] + g * d[2 * k + ch]));
                    assert_eq!(lr[ch], want, "frame {k} ch {ch} of block at {}", b.pts);
                }
            }
        }
        // And the stems hold real audio, not zeros that would also "match".
        let voice_peak = stems
            .iter()
            .flat_map(|s| &s.voice)
            .fold(0.0f32, |m, v| m.max(v.abs()));
        assert!(voice_peak > 0.45, "voice stem peak {voice_peak}");
    }

    /// The silence counter: how long since desktop audio last made a sound,
    /// reset by any; absent without desktop audio.
    #[test]
    fn desktop_quiet_seconds_counts_silence_not_packets() {
        let mut desk = Loopback::new(0.0);
        // Packets every 10 ms throughout — zeros until 12 s, a click at 12.5.
        let (tl, _) = desktop_take(12.4, 0.0, &mut desk, |_| [0.0, 0.0]);
        let quiet = tl.stats().desktop_quiet_seconds.expect("desktop audio");
        assert!(quiet >= 12.0, "quiet for {quiet:.2} s");
        let mut desk = Loopback::new(0.0);
        let (tl, _) = desktop_take(14.0, 0.0, &mut desk, sides(click(12.5), click(12.5)));
        let quiet = tl.stats().desktop_quiet_seconds.expect("desktop audio");
        assert!(quiet < 2.0, "the click did not count: {quiet:.2} s");
        assert_eq!(AudioTimeline::new().stats().desktop_quiet_seconds, None);
    }

    /// While the desktop is idle, "complete up to" trails now by the
    /// desktop's wait, not by the engine's second.
    #[test]
    fn an_idle_desktop_is_ready_up_to_its_wait() {
        let mut tl = with_desktop(0.0);
        let mut e = Engine::new(48_000, 0.0);
        for c in e.until(2.0, &silence, &silence) {
            tl.push(&c);
        }
        let ready = tl.ready_until(2.0).expect("audio");
        assert!(
            (ready - (2.0 - DESKTOP_WAIT_SECONDS)).abs() < 0.02,
            "ready to {ready:.3}"
        );
        assert_eq!(AudioTimeline::new().ready_until(2.0), None);
    }
}
