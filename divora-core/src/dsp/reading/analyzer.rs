//! The analysis engine: audio in, [`VoiceReading`] out.
//!
//! Runs on a worker thread. The audio callback's only job is to copy blocks
//! into the rings in `crate::audio::engine`; everything here — framing, pitch
//! tracking, an FFT — is far too expensive for the real-time path and none of
//! it would be safe there.
//!
//! ### The two time constants, and why these numbers
//!
//! * **[`WINDOW_S`] = 2.5 s.** Speech's amplitude modulation peaks at 4–5 Hz,
//!   so 2.5 s spans ~10–12 syllables: a phrase. A phrase is the shortest unit
//!   over which a pitch *range* or a pace means anything — measure half of one
//!   and the p90−p10 is whatever single accent happened to fall inside. Longer
//!   than ~3 s and the readout lags the speaker by more than a breath group,
//!   which is the point at which people stop believing it is about them. At
//!   this length a window holds 250 level frames and ~125 pitch frames, so the
//!   p10 and p90 each sit on a dozen voiced frames rather than one.
//! * **[`EMIT_S`] = 0.25 s.** Four readings a second. A three-word phrase
//!   takes about a second to read, so emitting faster only makes the numbers
//!   move; emitting slower makes a preset switch (which moves the wet reading
//!   instantly and is meant to) look like a hang.
//!
//! Successive windows therefore overlap by 90 %, which is deliberate: the
//! numbers glide instead of stepping, and the descriptor hysteresis in
//! [`super::describe`] is what keeps the *words* from gliding with them.
//!
//! ### Framing
//!
//! One framing, three analysis rates, each at the bandwidth its measurement
//! needs: 40 ms frames every 10 ms for level and onsets, pitch on every second
//! frame (20 ms — vibrato and intonation live well under 25 Hz), the spectral
//! centroid on every fourth (40 ms — brightness is a window statistic and an
//! FFT is the expensive one). Every constant below is in seconds or Hz and
//! every length is derived from the sample rate, so 44.1, 48 and 96 kHz agree.
//!
//! ### What this engine does not see
//!
//! Speak and Critter Chatter audio never reaches either tap. `engine.rs`
//! renders those voices into their own `soundboard_out` buffer inside
//! `mix_voice_and_soundboard` and folds them into the send *after* the chain
//! has run; the wet tap is the chain's output on the mic buffer alone. That is
//! correct and is not worth "fixing" — a panel that read the send instead of
//! the chain would start reading a sound effect the moment one played.

#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]

use super::super::{Descriptor, Metrics, ReadingState, VoiceReading};
use super::baseline::{Baseline, AXES};
use super::describe::{deviations, observation, Describer};
use super::pitch::{Decimator, PitchDetector, PITCH_FRAME_S};
use super::spectrum::Centroid;

/// Rolling analysis window.
pub const WINDOW_S: f32 = 2.5;
/// How often a reading is produced.
pub const EMIT_S: f32 = 0.25;
/// Level / spectral frame length.
const FRAME_S: f32 = 0.040;
/// Hop between frames.
const HOP_S: f32 = 0.010;
/// Pitch runs on every Nth frame.
const PITCH_EVERY: u8 = 2;
/// The spectral centroid runs on every Nth frame.
const SPECTRAL_EVERY: u8 = 4;
/// Seconds between pitch frames.
const PITCH_HOP_S: f32 = HOP_S * PITCH_EVERY as f32;

/// Absolute floor below which a frame is not speech at any gain setting.
/// Above digital silence and the noise floor of a gated headset, below any
/// level a person can be heard at on a call.
const SPEECH_FLOOR_DBFS: f32 = -55.0;
/// A frame further than this below the window's loudest is a pause, not quiet
/// delivery. Including pauses would make every speaker read as maximally
/// dynamic, because the gaps between words are 40 dB down.
const ACTIVE_SPAN_DB: f32 = 40.0;
/// Level rise that marks a new onset, dB over the recent local minimum.
const ONSET_RISE_DB: f32 = 4.0;
/// How far back that minimum is taken.
const ONSET_LOOKBACK_S: f32 = 0.08;
/// Periodicity a frame needs before it may plant an onset. Named for the
/// property of the waveform, not for anything about whoever produced it —
/// "confident" would be a word about a person, and there are none of those in
/// this module.
const ONSET_MIN_PERIODICITY: f32 = 0.70;
/// Minimum spacing between onsets. The syllable rate peaks at 4–5 Hz and tops
/// out near 7–8, so 100 ms admits any real pace while refusing to count one
/// syllable twice.
const ONSET_REFRACTORY_S: f32 = 0.10;
/// Voiced audio in the window needed to call it speech.
const SPEAK_ENTER_S: f32 = 0.30;
/// ... and to keep calling it speech. The gap is hysteresis: without it the
/// panel flips state on every inter-word pause.
const SPEAK_EXIT_S: f32 = 0.15;
/// How long after the last voiced frame the window is still called speech.
///
/// Without this the state would trail the speaker by the whole window: a
/// 2.5 s window still holds a phrase's worth of voiced frames two seconds
/// after someone stops talking, so it would keep saying Speaking long after
/// they had. Half a second is longer than any stop consonant or inter-word
/// gap and longer than most breaths between phrases, so it does not blink
/// mid-sentence, and it is short enough that the numbers freeze while they
/// still describe speech.
const TAIL_GRACE_S: f32 = 0.50;
/// Shortest stretch a pace is quoted over. One burst inside half a second is
/// not a rate.
const MIN_PACE_SPAN_S: f32 = 0.50;
/// Emits between baseline observations. Windows overlap 90 %, so taking one
/// every emit would fill the ring with ten copies of the same 2.5 seconds and
/// make a robust statistic over it a fiction.
pub const STRIDE_EMITS: usize = 4;
/// Wet audio needed after a chain change before its metrics are shown.
const MIN_WET_S: f32 = 0.40;

/// What the engine knows that the signal cannot tell us.
///
/// `Stopped` and `Muted` are not inferred from silence — silence is what all
/// three of stopped, muted and a quiet room look like, and guessing wrong
/// means the panel tells someone their mic is dead when it is fine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InputFacts {
    /// The audio engine is running.
    pub engine_running: bool,
    /// The input is muted — at the device, or by the app.
    pub input_muted: bool,
    /// The effect chain is bypassed, so the wet tap is carrying the dry
    /// signal. Push-to-modulate with the key up is exactly this: the dry tap
    /// has speech on it and the wet tap is a passthrough, which is not the
    /// preset failing.
    pub chain_bypassed: bool,
}

impl Default for InputFacts {
    fn default() -> Self {
        Self {
            engine_running: true,
            input_muted: false,
            chain_bypassed: false,
        }
    }
}

/// Fixed-capacity ring of window statistics. Order does not matter to any
/// consumer here (percentiles, medians, counts), so this never rotates.
struct Window<T> {
    buf: Vec<T>,
    pos: usize,
    len: usize,
}

impl<T: Copy + Default> Window<T> {
    fn new(cap: usize) -> Self {
        Self {
            buf: vec![T::default(); cap.max(1)],
            pos: 0,
            len: 0,
        }
    }

    fn push(&mut self, v: T) {
        let cap = self.buf.len();
        self.buf[self.pos] = v;
        self.pos = (self.pos + 1) % cap;
        self.len = (self.len + 1).min(cap);
    }

    fn items(&self) -> &[T] {
        &self.buf[..self.len]
    }

    /// Oldest first. Only the metrics that care where in the window something
    /// happened use this.
    fn ordered(&self) -> std::iter::Chain<std::slice::Iter<'_, T>, std::slice::Iter<'_, T>> {
        if self.len < self.buf.len() {
            self.buf[..self.len].iter().chain(self.buf[..0].iter())
        } else {
            self.buf[self.pos..]
                .iter()
                .chain(self.buf[..self.pos].iter())
        }
    }

    fn clear(&mut self) {
        self.pos = 0;
        self.len = 0;
    }
}

/// Everything measured on one tap.
struct Tap {
    frame_len: usize,
    hop_len: usize,
    // --- full-rate framing ---
    ring: Vec<f32>,
    ring_pos: usize,
    ring_filled: usize,
    frame: Vec<f32>,
    hop_count: usize,
    pitch_phase: u8,
    spectral_phase: u8,
    // --- pitch path ---
    decimator: Decimator,
    pitch_ring: Vec<f32>,
    pitch_pos: usize,
    pitch_filled: usize,
    pitch_frame: Vec<f32>,
    detector: PitchDetector,
    // --- spectral path ---
    centroid: Centroid,
    // --- window statistics ---
    levels: Window<f32>,
    f0s: Window<f32>,
    onsets: Window<bool>,
    brights: Window<f32>,
    // --- streaming onset state ---
    recent: Vec<f32>,
    recent_pos: usize,
    recent_len: usize,
    frames_since_onset: usize,
    onset_refractory_frames: usize,
    prev_voiced: bool,
    frames_since_voiced: usize,
    // --- bookkeeping ---
    frames_since_reset: usize,
    sort: Vec<f32>,
}

impl Tap {
    fn new(sample_rate: f32) -> Self {
        let frame_len = (FRAME_S * sample_rate).round().max(2.0) as usize;
        let hop_len = (HOP_S * sample_rate).round().max(1.0) as usize;
        let decimator = Decimator::new(sample_rate);
        let pitch_rate = decimator.rate(sample_rate);
        let pitch_frame_len = (PITCH_FRAME_S * pitch_rate).round().max(2.0) as usize;
        let detector = PitchDetector::new(pitch_rate);
        let pitch_frame_len = pitch_frame_len.max(detector.min_frame_len() + 4);
        let level_cap = (WINDOW_S / HOP_S).ceil() as usize;
        let lookback = (ONSET_LOOKBACK_S / HOP_S).round().max(1.0) as usize;
        Self {
            frame_len,
            hop_len,
            ring: vec![0.0; frame_len],
            ring_pos: 0,
            ring_filled: 0,
            frame: vec![0.0; frame_len],
            hop_count: 0,
            pitch_phase: 0,
            spectral_phase: 0,
            decimator,
            pitch_ring: vec![0.0; pitch_frame_len],
            pitch_pos: 0,
            pitch_filled: 0,
            pitch_frame: vec![0.0; pitch_frame_len],
            detector,
            centroid: Centroid::new(sample_rate, frame_len),
            levels: Window::new(level_cap),
            f0s: Window::new(level_cap / PITCH_EVERY as usize + 1),
            onsets: Window::new(level_cap / PITCH_EVERY as usize + 1),
            brights: Window::new(level_cap / SPECTRAL_EVERY as usize + 1),
            recent: vec![f32::NEG_INFINITY; lookback],
            recent_pos: 0,
            recent_len: 0,
            frames_since_onset: usize::MAX / 2,
            onset_refractory_frames: (ONSET_REFRACTORY_S / PITCH_HOP_S).round().max(1.0) as usize,
            prev_voiced: false,
            frames_since_voiced: usize::MAX / 2,
            frames_since_reset: 0,
            sort: Vec::with_capacity(level_cap),
        }
    }

    /// Number of level frames a full window holds.
    fn window_frames(&self) -> usize {
        self.levels.buf.len()
    }

    fn feed(&mut self, block: &[f32]) {
        for &x in block {
            // Sanitise here rather than trusting the device: one NaN would
            // poison every running statistic downstream for a full window.
            let x = if x.is_finite() { x } else { 0.0 };
            self.ring[self.ring_pos] = x;
            self.ring_pos = (self.ring_pos + 1) % self.frame_len;
            self.ring_filled = (self.ring_filled + 1).min(self.frame_len);
            if let Some(d) = self.decimator.push(x) {
                let cap = self.pitch_ring.len();
                self.pitch_ring[self.pitch_pos] = d;
                self.pitch_pos = (self.pitch_pos + 1) % cap;
                self.pitch_filled = (self.pitch_filled + 1).min(cap);
            }
            self.hop_count += 1;
            if self.hop_count >= self.hop_len {
                self.hop_count = 0;
                if self.ring_filled >= self.frame_len {
                    self.on_frame();
                }
            }
        }
    }

    fn on_frame(&mut self) {
        // Oldest-first copy out of the circular buffer.
        let split = self.ring_pos;
        let (head, tail) = self.ring.split_at(split);
        self.frame[..tail.len()].copy_from_slice(tail);
        self.frame[tail.len()..].copy_from_slice(head);

        let mut sum = 0.0f32;
        for &s in &self.frame {
            sum = s.mul_add(s, sum);
        }
        let rms = (sum / self.frame_len as f32).sqrt();
        // Exact zero stays -inf: that is how digital silence (a mute) is told
        // apart from a very quiet room, which never reaches exactly zero.
        let level_db = if rms > 0.0 {
            20.0 * rms.log10()
        } else {
            f32::NEG_INFINITY
        };
        self.levels.push(level_db);
        self.frames_since_reset += 1;

        if tick(&mut self.pitch_phase, PITCH_EVERY) {
            self.on_pitch_frame(level_db);
        }
        if tick(&mut self.spectral_phase, SPECTRAL_EVERY) && level_db >= SPEECH_FLOOR_DBFS {
            if let Some(hz) = self.centroid.measure(&self.frame) {
                self.brights.push(hz);
            }
        }
        self.remember_level(level_db);
    }

    fn on_pitch_frame(&mut self, level_db: f32) {
        let loud_enough = level_db >= SPEECH_FLOOR_DBFS;
        let pitch = if loud_enough && self.pitch_filled >= self.pitch_ring.len() {
            let split = self.pitch_pos;
            let (head, tail) = self.pitch_ring.split_at(split);
            self.pitch_frame[..tail.len()].copy_from_slice(tail);
            self.pitch_frame[tail.len()..].copy_from_slice(head);
            self.detector.estimate(&self.pitch_frame)
        } else {
            None
        };
        let hz = pitch.map_or(0.0, |p| p.hz);
        self.f0s.push(hz);

        self.frames_since_onset = self.frames_since_onset.saturating_add(1);
        // An onset is a clearly-voiced frame that either starts a voiced run
        // (a syllable after a consonant) or steps up out of the local dip
        // between two syllables inside one. Neither is syllable recognition;
        // it is a count of rising edges, which is why the metric is named as
        // a proxy. The confidence floor is stricter than the voicing decision
        // on purpose: a marginal frame is fine to include in a ratio over a
        // window, but it should not plant an event at a moment.
        let periodic = pitch.is_some_and(|p| p.confidence >= ONSET_MIN_PERIODICITY);
        let rise = level_db - self.recent_min();
        let onset = periodic
            && (!self.prev_voiced || rise >= ONSET_RISE_DB)
            && self.frames_since_onset >= self.onset_refractory_frames;
        if onset {
            self.frames_since_onset = 0;
        }
        self.prev_voiced = hz > 0.0;
        self.frames_since_voiced = if self.prev_voiced {
            0
        } else {
            self.frames_since_voiced.saturating_add(1)
        };
        self.onsets.push(onset);
    }

    fn remember_level(&mut self, level_db: f32) {
        let cap = self.recent.len();
        self.recent[self.recent_pos] = level_db;
        self.recent_pos = (self.recent_pos + 1) % cap;
        self.recent_len = (self.recent_len + 1).min(cap);
    }

    fn recent_min(&self) -> f32 {
        self.recent[..self.recent_len]
            .iter()
            .copied()
            .fold(f32::INFINITY, f32::min)
    }

    /// True when every frame in the window was bit-exact zero.
    fn digitally_silent(&self) -> bool {
        self.levels.len >= 4 && self.levels.items().iter().all(|l| l.is_infinite())
    }

    fn voiced_frames(&self) -> usize {
        self.f0s.items().iter().filter(|hz| **hz > 0.0).count()
    }

    /// Time since the last voiced frame. `f32::INFINITY` before there has
    /// been one.
    fn since_voiced_s(&self) -> f32 {
        if self.frames_since_voiced > self.f0s.buf.len() * 4 {
            f32::INFINITY
        } else {
            self.frames_since_voiced as f32 * PITCH_HOP_S
        }
    }

    /// Onsets per second over the stretch of the window that actually carried
    /// voice, first voiced frame to last.
    ///
    /// Dividing by the whole window instead would make the number fall
    /// through every pause as the window emptied — which is the readout
    /// sliding toward "slow" while the speaker is simply not talking, and it
    /// is the frozen number the panel would then sit on.
    fn pace(&self) -> f32 {
        let mut first = None;
        let mut last = 0usize;
        let mut onsets = 0usize;
        for (i, (hz, onset)) in self.f0s.ordered().zip(self.onsets.ordered()).enumerate() {
            if *hz > 0.0 {
                if first.is_none() {
                    first = Some(i);
                }
                last = i;
            }
            if *onset {
                onsets += 1;
            }
        }
        let Some(first) = first else {
            return 0.0;
        };
        let span = ((last - first + 1) as f32 * PITCH_HOP_S).max(MIN_PACE_SPAN_S);
        onsets as f32 / span
    }

    fn metrics(&mut self) -> Metrics {
        let (energy_dbfs, energy_range_db) = self.energy();
        let (f0_hz, f0_range_st) = self.pitch_stats();
        let pitch_frames = self.f0s.len.max(1);
        let voiced_ratio = self.voiced_frames() as f32 / pitch_frames as f32;
        let pace_ops = self.pace();
        Metrics {
            energy_dbfs,
            energy_range_db,
            f0_hz,
            f0_range_st,
            voiced_ratio,
            pace_ops,
            brightness_hz: self.brightness(),
        }
    }

    /// Median level and p90−p10 spread over the window's *active* frames.
    fn energy(&mut self) -> (f32, f32) {
        let peak = self
            .levels
            .items()
            .iter()
            .copied()
            .fold(f32::NEG_INFINITY, f32::max);
        if !peak.is_finite() {
            return (f32::NEG_INFINITY, 0.0);
        }
        let floor = SPEECH_FLOOR_DBFS.max(peak - ACTIVE_SPAN_DB);
        self.sort.clear();
        self.sort
            .extend(self.levels.items().iter().copied().filter(|l| *l >= floor));
        if self.sort.is_empty() {
            return (f32::NEG_INFINITY, 0.0);
        }
        sort_scratch(&mut self.sort);
        let median = percentile(&self.sort, 0.5);
        (
            median,
            percentile(&self.sort, 0.9) - percentile(&self.sort, 0.1),
        )
    }

    /// Median voiced F0 and its p90−p10 spread, the latter in semitones.
    /// Percentiles commute with the log, so they are taken in Hz and
    /// converted once.
    fn pitch_stats(&mut self) -> (f32, f32) {
        self.sort.clear();
        self.sort
            .extend(self.f0s.items().iter().copied().filter(|hz| *hz > 0.0));
        if self.sort.len() < 3 {
            return (0.0, 0.0);
        }
        sort_scratch(&mut self.sort);
        let median = percentile(&self.sort, 0.5);
        let lo = percentile(&self.sort, 0.1);
        let hi = percentile(&self.sort, 0.9);
        let range = if lo > 0.0 {
            12.0 * (hi / lo).log2()
        } else {
            0.0
        };
        (median, range)
    }

    fn brightness(&mut self) -> f32 {
        if self.brights.len == 0 {
            return 0.0;
        }
        self.sort.clear();
        self.sort.extend(self.brights.items().iter().copied());
        sort_scratch(&mut self.sort);
        percentile(&self.sort, 0.5)
    }

    /// Drop the window and the streaming detectors, keeping nothing that
    /// could carry a discontinuity across.
    fn reset(&mut self) {
        self.ring.fill(0.0);
        self.ring_pos = 0;
        self.ring_filled = 0;
        self.hop_count = 0;
        self.pitch_phase = 0;
        self.spectral_phase = 0;
        self.decimator.reset();
        self.pitch_ring.fill(0.0);
        self.pitch_pos = 0;
        self.pitch_filled = 0;
        self.detector.reset();
        self.levels.clear();
        self.f0s.clear();
        self.onsets.clear();
        self.brights.clear();
        self.recent.fill(f32::NEG_INFINITY);
        self.recent_pos = 0;
        self.recent_len = 0;
        self.frames_since_onset = usize::MAX / 2;
        self.prev_voiced = false;
        self.frames_since_voiced = usize::MAX / 2;
        self.frames_since_reset = 0;
    }
}

/// Advance a divider and say whether this call is the one that fires.
fn tick(phase: &mut u8, every: u8) -> bool {
    let due = *phase == 0;
    *phase += 1;
    if *phase >= every {
        *phase = 0;
    }
    due
}

fn sort_scratch(v: &mut [f32]) {
    v.sort_unstable_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
}

/// Linear-interpolated percentile of an ascending slice.
fn percentile(sorted: &[f32], p: f32) -> f32 {
    if sorted.is_empty() {
        return 0.0;
    }
    if sorted.len() == 1 {
        return sorted[0];
    }
    let pos = p.clamp(0.0, 1.0) * (sorted.len() - 1) as f32;
    let lo = pos.floor() as usize;
    let hi = (lo + 1).min(sorted.len() - 1);
    let frac = pos - lo as f32;
    (sorted[hi] - sorted[lo]).mul_add(frac, sorted[lo])
}

/// The last speaking window, kept so a pause does not decay the panel.
struct Held {
    dry: Metrics,
    wet: Metrics,
    descriptors: Vec<Descriptor>,
}

/// Turns dry and wet audio into [`VoiceReading`]s.
///
/// Construct once per engine session with the engine's sample rate, feed it
/// equal-length dry and wet blocks from a worker thread, and take the readings
/// it hands back roughly every [`EMIT_S`].
pub struct Analyzer {
    dry: Tap,
    wet: Tap,
    baseline: Baseline,
    describer: Describer,
    state: ReadingState,
    started: bool,
    since_emit: usize,
    emit_len: usize,
    min_wet_frames: usize,
    last_wet: Metrics,
    emits_since_observation: usize,
    descriptors: Vec<Descriptor>,
    held: Option<Held>,
}

impl Analyzer {
    /// Allocates every buffer it will ever use.
    #[must_use]
    pub fn new(sample_rate: u32) -> Self {
        let rate = sample_rate as f32;
        Self {
            dry: Tap::new(rate),
            wet: Tap::new(rate),
            baseline: Baseline::new(),
            describer: Describer::new(),
            state: ReadingState::Stopped,
            started: false,
            since_emit: 0,
            emit_len: (EMIT_S * rate).round().max(1.0) as usize,
            min_wet_frames: (MIN_WET_S / HOP_S).round().max(1.0) as usize,
            last_wet: Metrics::silent(),
            emits_since_observation: 0,
            descriptors: Vec::with_capacity(super::super::MAX_DESCRIPTORS),
            held: None,
        }
    }

    /// The state of the last reading produced.
    #[must_use]
    pub fn state(&self) -> ReadingState {
        self.state
    }

    /// Feed one block of each tap. `dry` and `wet` must be the *same* audio at
    /// two points in the chain, so they are truncated to the shorter length
    /// rather than misaligned.
    ///
    /// Returns a reading every [`EMIT_S`], and immediately whenever the state
    /// changes — including with empty blocks, which is how a stopped engine
    /// (no audio at all) still produces [`ReadingState::Stopped`].
    pub fn observe(&mut self, dry: &[f32], wet: &[f32], facts: InputFacts) -> Option<VoiceReading> {
        let n = dry.len().min(wet.len());
        if n > 0 {
            self.dry.feed(&dry[..n]);
            self.wet.feed(&wet[..n]);
            self.since_emit += n;
        }
        let state = self.classify(facts);
        if state == self.state && self.started && self.since_emit < self.emit_len {
            return None;
        }
        Some(self.emit(state, facts))
    }

    fn classify(&self, facts: InputFacts) -> ReadingState {
        if !facts.engine_running {
            return ReadingState::Stopped;
        }
        if facts.input_muted || self.dry.digitally_silent() {
            return ReadingState::Muted;
        }
        let voiced_s = self.dry.voiced_frames() as f32 * PITCH_HOP_S;
        let needed = if self.state == ReadingState::Speaking {
            SPEAK_EXIT_S
        } else {
            SPEAK_ENTER_S
        };
        // Two conditions, and both matter: enough voiced audio in the window
        // for its statistics to be about speech, and speech still arriving.
        if voiced_s >= needed && self.dry.since_voiced_s() <= TAIL_GRACE_S {
            ReadingState::Speaking
        } else {
            ReadingState::Quiet
        }
    }

    fn emit(&mut self, state: ReadingState, facts: InputFacts) -> VoiceReading {
        let leaving_running = state == ReadingState::Stopped && self.state != ReadingState::Stopped;
        self.state = state;
        self.started = true;
        self.since_emit = 0;
        if leaving_running {
            // An engine restart may bring a different device, a different
            // gain structure and a different room. None of that is the
            // speaker, so the baseline they were being compared against is
            // gone, and so is the last reading.
            self.reset_session();
        }
        if state == ReadingState::Speaking {
            self.emit_speaking(facts)
        } else {
            self.emit_held(state, facts)
        }
    }

    fn emit_speaking(&mut self, facts: InputFacts) -> VoiceReading {
        let dry = self.dry.metrics();
        let wet = self.wet_metrics();
        self.emits_since_observation += 1;
        if self.emits_since_observation >= STRIDE_EMITS {
            self.emits_since_observation = 0;
            self.baseline.push(observation(&dry));
        }
        if let Some(base) = self.baseline.summary() {
            let dev = deviations(&dry, &base);
            self.describer.describe(&dev, &mut self.descriptors);
        } else {
            // Nothing to compare against yet: say nothing rather than compare
            // against too little data.
            self.describer.reset();
            self.descriptors.clear();
        }
        self.held = Some(Held {
            dry,
            wet,
            descriptors: self.descriptors.clone(),
        });
        VoiceReading {
            state: ReadingState::Speaking,
            dry,
            wet,
            descriptors: self.descriptors.clone(),
            calibrated: self.baseline.calibrated(),
            held: false,
            held_descriptors: Vec::new(),
            wet_bypassed: facts.chain_bypassed,
            wet_settled: self.wet_settled(),
        }
    }

    /// Not speaking: hand back the last speaking window unchanged.
    ///
    /// Nothing decays. Most of a session is not speech, and a readout that
    /// slid toward "quiet, flat, narrow" through every pause would be handing
    /// down a verdict on the speaker several times a minute without printing a
    /// word. The state says why the numbers are standing still.
    fn emit_held(&mut self, state: ReadingState, facts: InputFacts) -> VoiceReading {
        let held = self.held.as_ref();
        VoiceReading {
            state,
            dry: held.map_or_else(Metrics::silent, |h| h.dry),
            wet: held.map_or_else(Metrics::silent, |h| h.wet),
            descriptors: Vec::new(),
            calibrated: self.baseline.calibrated(),
            held: held.is_some(),
            held_descriptors: held.map_or_else(Vec::new, |h| h.descriptors.clone()),
            wet_bypassed: facts.chain_bypassed,
            wet_settled: self.wet_settled(),
        }
    }

    /// Wet metrics, or the previous ones while a freshly-changed chain fills
    /// its window.
    fn wet_metrics(&mut self) -> Metrics {
        if self.wet.frames_since_reset >= self.min_wet_frames {
            self.last_wet = self.wet.metrics();
        }
        self.last_wet
    }

    fn wet_settled(&self) -> bool {
        self.wet.frames_since_reset >= self.wet.window_frames()
    }

    /// The chain changed — a preset switch, an effect toggled, a parameter
    /// moved.
    ///
    /// The wet window is dropped, because a window straddling two presets is a
    /// reading of neither. A +5 semitone preset then moves the wet pitch by
    /// +5 with no change whatsoever in the speaker, which is the entire point
    /// of the wet tap and is why the panel must label it as after-effects. The
    /// new numbers appear within [`MIN_WET_S`]; `wet_settled` stays false
    /// until a full window has passed.
    ///
    /// The dry tap and the baseline are untouched: the chain does not reach
    /// them.
    pub fn note_chain_changed(&mut self) {
        self.wet.reset();
    }

    /// Samples were lost between the audio thread and here, so the windows no
    /// longer cover the time they think they do. Drop them; keep the baseline,
    /// which is about the speaker rather than about the timeline.
    pub fn note_discontinuity(&mut self) {
        self.dry.reset();
        self.wet.reset();
    }

    /// Start over: new device, new engine session, new everything derived from
    /// the speaker.
    ///
    /// Nothing here was ever written anywhere — the baseline is a ring in
    /// memory owned by this analyzer, and dropping it is the whole of forgetting
    /// it.
    pub fn reset_session(&mut self) {
        self.dry.reset();
        self.wet.reset();
        self.baseline.reset();
        self.describer.reset();
        self.descriptors.clear();
        self.held = None;
        self.last_wet = Metrics::silent();
        self.emits_since_observation = 0;
    }

    /// Bytes of heap held by the framing buffers, the windows and the
    /// baseline, for the cost report. Excludes the detector and FFT
    /// scratch inside [`PitchDetector`] and [`Centroid`], which are a few
    /// kilobytes each.
    #[must_use]
    pub fn heap_bytes(&self) -> usize {
        let f32s = |n: usize| n * std::mem::size_of::<f32>();
        let tap = |t: &Tap| {
            f32s(t.ring.len() + t.frame.len() + t.pitch_ring.len() + t.pitch_frame.len())
                + f32s(t.levels.buf.len() + t.f0s.buf.len() + t.brights.buf.len())
                + t.onsets.buf.len()
                + f32s(t.recent.len() + t.sort.capacity())
        };
        tap(&self.dry) + tap(&self.wet) + AXES * 90 * std::mem::size_of::<f32>()
    }
}
