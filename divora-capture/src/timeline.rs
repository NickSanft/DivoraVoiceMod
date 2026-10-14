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
}

/// Builds the track. Feed it with [`push`](Self::push), take blocks with
/// [`pump`](Self::pump), end with [`finish`](Self::finish).
pub struct AudioTimeline {
    epochs: VecDeque<Epoch>,
    /// The next output frame to write.
    next: u64,
    block: Vec<f32>,
    stats: TimelineStats,
}

impl Default for AudioTimeline {
    fn default() -> Self {
        Self::new()
    }
}

impl AudioTimeline {
    #[must_use]
    pub fn new() -> Self {
        Self {
            epochs: VecDeque::new(),
            next: 0,
            block: Vec::with_capacity(BLOCK_FRAMES),
            stats: TimelineStats::default(),
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
            self.stats.drift_ppm = back.fit.drift_ppm();
            self.stats.voice_delay_ms = back.voice_delay_ms;
        }
    }

    /// Where the audio is complete up to, in seconds on the take's clock;
    /// `None` before any audio.
    #[must_use]
    pub fn ready_until(&self) -> Option<f64> {
        let back = self.epochs.back()?;
        Some(back.fit.time_of(back.ready_end() as f64))
    }

    /// Every block that can be written now. `now` is the take's clock: a
    /// sample older than `now − MAX_WAIT_SECONDS` with no audio is silence.
    pub fn pump(&mut self, now: f64) -> Vec<AudioBlock> {
        let mut out = Vec::new();
        while let Some(v) = self.sample(now) {
            self.block.push(v);
            self.next += 1;
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
        let end_frame = (end.max(0.0) * f64::from(OUT_RATE)).ceil() as u64;
        let mut out = Vec::new();
        while self.next < end_frame || !self.block.is_empty() {
            // Past the end is padding, not audio that failed to come.
            let v = if self.next < end_frame {
                self.sample(f64::INFINITY).unwrap_or(0.0)
            } else {
                0.0
            };
            self.block.push(v);
            self.next += 1;
            if self.block.len() == BLOCK_FRAMES {
                out.push(self.emit());
            }
        }
        out
    }

    /// The next output frame's value, or `None` to wait for more audio.
    fn sample(&mut self, now: f64) -> Option<f32> {
        let j = self.next;
        let tau = frame_time(j);
        loop {
            let Some(e) = self.epochs.front_mut() else {
                return self.waited(tau, now);
            };
            match e.sample(j) {
                Sampled::Value(v) => return Some(v),
                Sampled::Done => {
                    self.epochs.pop_front();
                }
                Sampled::Wait => return self.waited(tau, now),
            }
        }
    }

    /// Silence for a sample whose audio is overdue; `None` if it may still come.
    fn waited(&mut self, tau: f64, now: f64) -> Option<f32> {
        if tau <= now - MAX_WAIT_SECONDS {
            self.stats.waited_frames += 1;
            Some(0.0)
        } else {
            None
        }
    }

    fn emit(&mut self) -> AudioBlock {
        let first = self.next - BLOCK_FRAMES as u64;
        let pts = ticks(first);
        let duration = ticks(self.next) - pts;
        let mut samples = Vec::with_capacity(BLOCK_FRAMES * 2);
        for &v in &self.block {
            let s = (v.clamp(-1.0, 1.0) * f32::from(i16::MAX)) as i16;
            samples.push(s);
            samples.push(s);
        }
        self.block.clear();
        for e in &mut self.epochs {
            e.trim();
        }
        AudioBlock {
            pts,
            duration,
            samples,
        }
    }
}

/// Output frame `j`'s time on the take's clock, in seconds.
fn frame_time(j: u64) -> f64 {
    j as f64 / f64::from(OUT_RATE)
}

/// Output frame `j`'s time in 100-ns ticks, exactly.
fn ticks(j: u64) -> u64 {
    u64::try_from(u128::from(j) * 10_000_000 / u128::from(OUT_RATE)).unwrap_or(u64::MAX)
}

enum Pushed {
    Placed,
    Stepped,
    Early,
}

enum Sampled {
    Value(f32),
    /// The audio for this frame has not arrived yet.
    Wait,
    /// This epoch is over and the frame is past it.
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

/// One unbroken run of the engine's timeline.
struct Epoch {
    id: u64,
    rate: f64,
    fit: ClockFit,
    kernel: Kernel,
    /// Frames inserted for steps so far: a chunk's position is its index
    /// plus this.
    shift: i64,
    /// Position of `mix[0]`.
    base: i64,
    /// Soundboard and voice, summed, at the engine rate.
    mix: VecDeque<f32>,
    /// Where the next voice frame goes. Everything before it is final.
    voice_cursor: i64,
    sb_end: i64,
    started: bool,
    closed: bool,
    /// Smoothed voice wait, in seconds.
    lag: Option<f64>,
    /// The voice wait in use, in frames.
    lag_frames: i64,
    voice_delay_ms: f64,
    /// Smoothed callback period, in seconds.
    period: Option<f64>,
    track: Option<Track>,
}

impl Epoch {
    fn new(id: u64, rate: u32) -> Self {
        Self {
            id,
            rate: f64::from(rate.max(1)),
            fit: ClockFit::new(rate),
            kernel: Kernel::new(rate),
            shift: 0,
            base: 0,
            mix: VecDeque::new(),
            voice_cursor: 0,
            sb_end: 0,
            started: false,
            closed: false,
            lag: None,
            lag_frames: 0,
            voice_delay_ms: 0.0,
            period: None,
            track: None,
        }
    }

    /// No more audio is coming: the voice is complete wherever it stopped.
    fn close(&mut self) {
        self.closed = true;
    }

    fn ready_end(&self) -> i64 {
        if self.closed {
            self.voice_cursor.max(self.sb_end)
        } else {
            self.voice_cursor
        }
    }

    fn push(&mut self, c: &AudioChunk) -> Pushed {
        let span = c.voice.len().min(c.sb.len());
        let period = f64::from(c.out_frames) / f64::from(c.out_rate.max(1));
        let mean = *self.period.get_or_insert(period);
        let tau = c.at - (period - mean);
        self.period = Some(mean + 0.05 * (period - mean));

        let mut pos = c.index as i64 + self.shift;
        let mut outcome = Pushed::Placed;
        if let Some(r) = self.fit.residual(pos as f64, tau) {
            if r > STEP_SECONDS {
                let k = (r * self.rate).round() as i64;
                self.shift += k;
                pos += k;
                outcome = Pushed::Stepped;
            } else if r < -STEP_SECONDS {
                return Pushed::Early;
            }
        }
        self.fit.add(pos as f64, tau);

        // The voice's delay: the smoothed wait — followed exactly in a pause,
        // mid-word only when far out — plus the chain's exact latency.
        let alpha = (span as f64 / self.rate / LAG_TIME_CONSTANT_SECONDS).min(1.0);
        let lag = self
            .lag
            .map_or(c.voice_wait, |l| l + alpha * (c.voice_wait - l));
        self.lag = Some(lag);
        let target = (lag * self.rate).round() as i64;
        let quiet = c.voice[..span].iter().all(|s| s.abs() < QUIET);
        let far = (target - self.lag_frames).abs() as f64 > LAG_MOVE_IN_SPEECH_SECONDS * self.rate;
        if !self.started || quiet || far {
            self.lag_frames = target;
        }
        let delay = self.lag_frames + i64::from(c.chain_latency);
        self.voice_delay_ms = delay as f64 / self.rate * 1000.0;

        let voice_at = pos - delay;
        if !self.started {
            self.started = true;
            self.base = voice_at.min(pos);
            self.voice_cursor = voice_at;
            self.sb_end = pos;
        }
        self.add_at(pos, &c.sb[..span]);
        self.sb_end = self.sb_end.max(pos + span as i64);
        // A voice delay that grew overlaps what is already placed: those
        // frames are the new chain's priming and are dropped. One that shrank
        // leaves a gap, which stays silent.
        let skip = (self.voice_cursor - voice_at).max(0) as usize;
        if skip < span {
            self.add_at(voice_at + skip as i64, &c.voice[skip..span]);
        }
        self.voice_cursor = self.voice_cursor.max(voice_at + span as i64);
        outcome
    }

    /// Add `samples` into the mix starting at `pos`.
    fn add_at(&mut self, pos: i64, samples: &[f32]) {
        let skip = (self.base - pos).max(0) as usize;
        if skip >= samples.len() {
            return;
        }
        let start = (pos + skip as i64 - self.base) as usize;
        let end = start + samples.len() - skip;
        if self.mix.len() < end {
            self.mix.resize(end, 0.0);
        }
        for (m, &s) in self.mix.range_mut(start..end).zip(&samples[skip..]) {
            *m += s;
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

    fn sample(&mut self, j: u64) -> Sampled {
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
        let end = self.ready_end();
        if hi < self.base {
            return Sampled::Value(0.0);
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

    /// The signal in `src` (whose first element sits at `base`; zero outside
    /// it) at position `x`.
    fn at(&self, src: &VecDeque<f32>, base: i64, x: f64) -> f32 {
        let i = x.floor();
        let p = (x - i) * PHASES as f64;
        let p0 = (p.floor() as usize).min(PHASES - 1);
        let w = (p - p0 as f64) as f32;
        let taps = 2 * self.half;
        let r0 = &self.rows[p0 * taps..(p0 + 1) * taps];
        let r1 = &self.rows[(p0 + 1) * taps..(p0 + 2) * taps];
        let first = i as i64 - self.half as i64 + 1;
        let len = src.len() as i64;
        let mut acc = 0.0f32;
        for k in 0..taps {
            let at = first + k as i64 - base;
            if at < 0 || at >= len {
                continue;
            }
            let c = r0[k] + (r1[k] - r0[k]) * w;
            acc += c * src[at as usize];
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
    use super::{AudioBlock, AudioChunk, AudioTimeline, BLOCK_FRAMES, OUT_RATE};
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
                b.samples
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .map(|lr| f32::from(lr[0]) / 32767.0)
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
}
