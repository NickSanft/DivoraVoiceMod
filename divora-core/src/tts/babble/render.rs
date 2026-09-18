//! Events → audio.
//!
//! Each event reads its unit's audio at the event's pitch ratio, shapes it
//! with a short fade-in and, where the event cuts the unit short, a fade-out
//! over the end of what it reads (the crossfade into the overlapping
//! neighbour), and adds it into the output. Units are placed so the point
//! where each one gets loud, not its first sample, lands on the scheduler's
//! beat. The whole utterance is then levelled to a fixed loudness and
//! soft-limited under a peak ceiling.
//!
//! Levelling matters more here than for ordinary playback: Speak output goes
//! straight to the call without passing through the effect chain or the
//! loudness stage, so the level this produces is the level people hear.
//!
//! This runs on a worker thread, never the audio callback, so it allocates.

use super::phonics::Unit;
use super::schedule::Schedule;
use crate::tts::TTS_SAMPLE_RATE;

/// Where unit audio comes from. Implementations return mono samples at
/// [`TTS_SAMPLE_RATE`] for every unit in the inventory, each already trimmed,
/// ending in silence, and at a consistent level.
///
/// Sources must be band-limited. The renderer reads units at pitch ratios up
/// to about ×1.35 with interpolation alone and does not anti-alias, so
/// anything above `TTS_SAMPLE_RATE / 2 / 1.35` (≈ 8.9 kHz) folds back down
/// into the band people hear.
pub trait UnitSource {
    /// The audio for `unit`. May be empty, which renders as silence.
    fn unit(&self, unit: Unit) -> &[f32];

    /// Samples from the start of `unit` to where it gets loud: the renderer
    /// puts that point, not the first sample, on the beat. A source that
    /// doesn't know leaves every unit starting on its beat.
    fn lead(&self, _unit: Unit) -> usize {
        0
    }
}

/// Loudness the whole utterance is levelled to, in dBFS on [`loudness_dbfs`]'s
/// meter. The app's Kokoro voice reading a reference line measures −21.15 dBFS
/// on the same meter, so switching between voices doesn't jump in level.
pub const TARGET_DBFS: f64 = -21.15;
/// Frames of this many samples (20 ms) count toward loudness...
pub const GATE_FRAME: usize = 480;
/// ...when their own RMS is at least this, so pauses and faint tails don't
/// drag the reading down.
pub const GATE_DBFS: f64 = -45.0;
/// Nothing leaves the renderer louder than this.
pub const PEAK_CEILING: f32 = 0.89;
/// Where the soft limiter starts bending.
const KNEE: f32 = 0.6;

const FADE_IN_SECS: f64 = 0.003;
/// A cut-short unit fades over the last 25 ms of what's read, or its last
/// quarter if that is shorter. A unit read to its end decays by itself and
/// gets only a click guard as short as the fade-in.
const FADE_OUT_SECS: f64 = 0.025;
const FADE_OUT_SHARE: f64 = 0.25;

/// Where one event's unit lands in the output.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct Placement {
    pub unit: Unit,
    /// First output sample written.
    pub start: usize,
    /// Output samples written.
    pub len: usize,
    /// Source position read at `start`, in samples.
    pub phase: f64,
    /// Source samples advanced per output sample.
    pub step: f64,
    /// Output sample the unit's lead lands on: its place on the beat grid.
    pub beat: usize,
    /// Samples of fade-in and fade-out.
    pub fade_in: f64,
    pub fade_out: f64,
    pub gain: f32,
}

impl Placement {
    /// The `n`th output sample of this event before gain: the unit read at
    /// its position, under the fades.
    pub fn sample(&self, src: &[f32], n: usize) -> f32 {
        let t = to_f64(n);
        let rise = t / self.fade_in;
        let fall = (to_f64(self.len) - t) / self.fade_out;
        #[allow(clippy::cast_possible_truncation)]
        let env = raised_cosine(rise.min(fall).min(1.0)) as f32;
        self.read(src, n) * env
    }

    /// The unit at output sample `n`, without fades.
    pub fn read(&self, src: &[f32], n: usize) -> f32 {
        hermite(src, self.phase + to_f64(n) * self.step)
    }
}

/// Render `schedule` from `source` to mono audio at [`TTS_SAMPLE_RATE`].
#[must_use]
pub fn render(schedule: &Schedule, source: &dyn UnitSource) -> Vec<f32> {
    let (placements, len) = place(schedule, source);
    let mut out = vec![0.0_f32; len];
    for p in &placements {
        let src = source.unit(p.unit);
        // `place` sizes the output to hold every placement, but `Schedule` is
        // public: a hand-built one with a short `len` must clip, not panic.
        let Some(window) = out.get_mut(p.start..(p.start + p.len).min(len)) else {
            continue;
        };
        for (n, slot) in window.iter_mut().enumerate() {
            *slot += p.sample(src, n) * p.gain;
        }
    }
    level(&mut out);
    out
}

/// Lay every sounding event out in the output, and the output's length.
///
/// Each unit starts `lead` (in output samples) ahead of its event's slot, so
/// where it gets loud lands on the slot. The first unit's lead would start
/// before sample 0, so the whole utterance shifts later by a shared pre-roll
/// just big enough that nothing does. The unit that sets the pre-roll starts
/// at sample 0; one with a shorter lead can begin a few milliseconds in, which
/// is its own silence, not padding. Each unit still stops where its event's
/// tail ends on the grid.
pub(super) fn place(schedule: &Schedule, source: &dyn UnitSource) -> (Vec<Placement>, usize) {
    let sr = f64::from(TTS_SAMPLE_RATE);
    let fade_in = (FADE_IN_SECS * sr).max(1.0);
    let sounding = || {
        schedule.events.iter().filter_map(move |e| {
            let src = source.unit(e.unit);
            if src.len() < 2 || e.len == 0 {
                return None;
            }
            let step = f64::from(e.pitch.max(0.01));
            let lead = to_f64(source.lead(e.unit).min(src.len() - 1));
            // Whole output samples ahead of the beat; the remainder becomes a
            // fractional read position, so the lead lands exactly on it.
            let ahead = to_usize((lead / step).floor());
            Some((e, src.len(), step, ahead, lead - to_f64(ahead) * step))
        })
    };
    let pre_roll = sounding()
        .map(|(e, _, _, ahead, _)| ahead.saturating_sub(e.start))
        .max()
        .unwrap_or(0);

    let placements = sounding()
        .filter_map(|(e, src_len, step, ahead, phase)| {
            let beat = e.start + pre_roll;
            // Output samples until the unit's last sample is read.
            let natural = to_usize(((to_f64(src_len - 1) - phase) / step).floor()) + 1;
            let len = natural.min(ahead + e.len);
            let fade_out = if len < natural {
                (FADE_OUT_SECS * sr).min(FADE_OUT_SHARE * to_f64(len))
            } else {
                fade_in
            };
            (len > 0).then_some(Placement {
                unit: e.unit,
                start: beat - ahead,
                len,
                phase,
                step,
                beat,
                fade_in,
                fade_out: fade_out.max(1.0),
                gain: e.gain,
            })
        })
        .collect();
    let len = if schedule.len == 0 {
        0
    } else {
        schedule.len + pre_roll
    };
    (placements, len)
}

/// Four-point cubic Hermite (Catmull-Rom) interpolation, with silence
/// beyond both ends of `src`. Linear interpolation dulls the top octave and
/// buzzes with images of it; this keeps both an order of magnitude down.
fn hermite(src: &[f32], pos: f64) -> f32 {
    let i = pos.floor();
    #[allow(clippy::cast_possible_truncation)]
    let frac = (pos - i) as f32;
    #[allow(clippy::cast_possible_truncation)]
    let i = i as i64;
    let at = |k: i64| {
        usize::try_from(k)
            .ok()
            .and_then(|k| src.get(k))
            .copied()
            .unwrap_or(0.0)
    };
    let (xm1, x0, x1, x2) = (at(i - 1), at(i), at(i + 1), at(i + 2));
    let c1 = 0.5 * (x1 - xm1);
    let c2 = xm1 - 2.5 * x0 + 2.0 * x1 - 0.5 * x2;
    let c3 = 0.5 * (x2 - xm1) + 1.5 * (x0 - x1);
    ((c3 * frac + c2) * frac + c1) * frac + x0
}

/// 0 → 0, 1 → 1, smooth at both ends.
fn raised_cosine(x: f64) -> f64 {
    0.5 - 0.5 * (std::f64::consts::PI * x.clamp(0.0, 1.0)).cos()
}

/// Loudness of `x` in dBFS: RMS over the [`GATE_FRAME`] frames whose own RMS
/// reaches [`GATE_DBFS`]. `None` when no frame does.
#[must_use]
pub fn loudness_dbfs(x: &[f32]) -> Option<f64> {
    let rms = gated_rms(&frames(x), 1.0)?;
    Some(20.0 * rms.log10())
}

/// (energy, samples) per meter frame.
fn frames(x: &[f32]) -> Vec<(f64, usize)> {
    x.chunks(GATE_FRAME)
        .map(|f| {
            (
                f.iter().map(|&s| f64::from(s) * f64::from(s)).sum(),
                f.len(),
            )
        })
        .collect()
}

/// RMS over the frames that pass the gate once scaled by `gain`, unscaled.
fn gated_rms(frames: &[(f64, usize)], gain: f64) -> Option<f64> {
    let gate = 10_f64.powf(GATE_DBFS / 20.0) / gain;
    let (energy, count) = frames
        .iter()
        .filter(|&&(e, n)| (e / to_f64(n)).sqrt() >= gate)
        .fold((0.0, 0), |(e, c), &(fe, n)| (e + fe, c + n));
    (count > 0 && energy > 0.0).then(|| (energy / to_f64(count)).sqrt())
}

/// Level to [`TARGET_DBFS`], then soft-limit under [`PEAK_CEILING`].
/// Non-finite samples become silence.
///
/// The gate applies to the levelled output, which depends on the gain being
/// chosen, so the gain is found by iterating: each pass drops frames that
/// would fall under the gate, which can only raise the reading and lower the
/// gain, so it settles in a few passes.
fn level(buf: &mut [f32]) {
    for x in buf.iter_mut() {
        if !x.is_finite() {
            *x = 0.0;
        }
    }
    let frames = frames(buf);
    let target = 10_f64.powf(TARGET_DBFS / 20.0);
    let mut gain = f64::INFINITY;
    for _ in 0..16 {
        let Some(rms) = gated_rms(&frames, gain) else {
            break;
        };
        let next = target / rms;
        let settled = (next - gain).abs() <= 1e-9 * next;
        gain = next;
        if settled {
            break;
        }
    }
    if !gain.is_finite() {
        return;
    }
    #[allow(clippy::cast_possible_truncation)]
    let gain = gain as f32;
    for x in buf.iter_mut() {
        *x = soft_limit(*x * gain);
    }
}

/// Identity below the knee, then a tanh bend toward the ceiling. The bound is
/// inclusive: in `f32`, `tanh` saturates to exactly 1.0 for large inputs, so a
/// very hot sample lands *on* the ceiling rather than just under it.
fn soft_limit(x: f32) -> f32 {
    let a = x.abs();
    if a <= KNEE {
        return x;
    }
    let room = PEAK_CEILING - KNEE;
    let bent = KNEE + room * ((a - KNEE) / room).tanh();
    bent.copysign(x)
}

#[allow(clippy::cast_precision_loss)]
const fn to_f64(n: usize) -> f64 {
    n as f64
}

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn to_usize(x: f64) -> usize {
    x.max(0.0) as usize
}

#[cfg(test)]
mod tests {
    use super::super::schedule::{schedule, VoiceParams};
    use super::*;

    /// A unit source that gives every unit a distinct decaying tone, so tests
    /// exercise real overlap without any shipped audio.
    struct Tones(Vec<Vec<f32>>);

    impl Tones {
        fn new() -> Self {
            let sr = f64::from(TTS_SAMPLE_RATE);
            Self(
                Unit::all()
                    .map(|u| {
                        #[allow(clippy::cast_precision_loss)]
                        let f = 200.0 + 7.0 * u.index() as f64;
                        (0..4_800)
                            .map(|n| {
                                let t = f64::from(n) / sr;
                                #[allow(clippy::cast_possible_truncation)]
                                let s = ((std::f64::consts::TAU * f * t).sin() * (-t * 12.0).exp())
                                    as f32;
                                s * 0.5
                            })
                            .collect()
                    })
                    .collect(),
            )
        }
    }

    impl UnitSource for Tones {
        fn unit(&self, unit: Unit) -> &[f32] {
            &self.0[unit.index()]
        }
    }

    /// Every unit is silence for a known, unit-specific lead, then a single
    /// click, then silence: where each click lands in the output is where
    /// the renderer put that unit's lead.
    struct Clicks(Vec<Vec<f32>>);

    impl Clicks {
        fn lead_of(unit: Unit) -> usize {
            200 + 37 * unit.index()
        }

        fn new() -> Self {
            Self(
                Unit::all()
                    .map(|u| {
                        let mut x = vec![0.0; Self::lead_of(u) + 2_000];
                        x[Self::lead_of(u)] = 0.5;
                        x
                    })
                    .collect(),
            )
        }
    }

    impl UnitSource for Clicks {
        fn unit(&self, unit: Unit) -> &[f32] {
            &self.0[unit.index()]
        }

        fn lead(&self, unit: Unit) -> usize {
            Self::lead_of(unit)
        }
    }

    const P: VoiceParams = VoiceParams {
        rate: 18.0,
        pitch: 1.2,
        jitter_cents: 40.0,
        tail_slots: 1.0,
    };

    #[test]
    fn the_loudness_target_is_the_measured_kokoro_reference() {
        // Pinned deliberately: the levelling tests assert against TARGET_DBFS
        // itself, so editing the constant would otherwise break nothing. This
        // value was measured from the app's own Kokoro voice (af_bella)
        // reading the preview line on the same gated meter — Speak skips the
        // loudness stage, so it is the level a call actually hears.
        assert!((TARGET_DBFS - (-21.15)).abs() < 1e-9, "{TARGET_DBFS}");
        assert_eq!(GATE_FRAME, TTS_SAMPLE_RATE as usize / 50);
        assert!((GATE_DBFS - (-45.0)).abs() < 1e-9);
    }

    #[test]
    fn a_schedule_shorter_than_its_events_clips_instead_of_panicking() {
        // `Schedule` is public, so a caller can hand us an inconsistent one.
        let source = Tones::new();
        let mut s = schedule("hello there", &P, 3);
        s.len = 64;
        let out = render(&s, &source);
        assert!(out.len() <= 64);
        assert!(out.iter().all(|x| x.is_finite()));
        // Clipped, not dropped: the part that fits is still rendered, so a
        // truncating caller hears the start rather than silence.
        assert!(out.iter().any(|&x| x != 0.0), "everything was skipped");
    }

    #[test]
    fn output_length_matches_the_schedule() {
        let s = schedule("Hi there, how are you?", &P, 3);
        assert_eq!(render(&s, &Tones::new()).len(), s.len);
    }

    #[test]
    fn output_is_finite_and_under_the_ceiling() {
        let s = schedule("HELLO!!! the quick brown fox... 12345 日本語", &P, 3);
        let out = render(&s, &Tones::new());
        assert!(!out.is_empty());
        assert!(out.iter().all(|x| x.is_finite()));
        assert!(out.iter().all(|x| x.abs() <= PEAK_CEILING));
    }

    #[test]
    fn levelled_to_the_target_loudness() {
        for text in [
            "a perfectly ordinary sentence to level",
            "hi. ... ... ... ... there",
            "HELLO!!! LOUD AND PROUD!!!",
        ] {
            let out = render(&schedule(text, &P, 3), &Tones::new());
            let loudness = loudness_dbfs(&out).expect("sounds");
            // The limiter can only pull it down, and only a little at this level.
            assert!(
                (loudness - TARGET_DBFS).abs() < 0.3,
                "{text:?} at {loudness:.2} dBFS"
            );
        }
    }

    #[test]
    fn the_meter_gates_out_silence_and_faint_frames() {
        let tone = |amp: f32, n: usize| -> Vec<f32> {
            (0..n)
                .map(|i| if i % 2 == 0 { amp } else { -amp })
                .collect()
        };
        // A square wave's RMS is its amplitude: 0.1 is −20 dBFS.
        let mut x = tone(0.1, 4_800);
        let loud = loudness_dbfs(&x).expect("sounds");
        assert!((loud + 20.0).abs() < 1e-6, "{loud}");
        // Pauses and a −50 dBFS murmur don't count; a −40 dBFS one does.
        x.extend(vec![0.0; 9_600]);
        x.extend(tone(0.003_162, 4_800));
        assert!((loudness_dbfs(&x).expect("sounds") - loud).abs() < 1e-9);
        x.extend(tone(0.01, 4_800));
        assert!(loudness_dbfs(&x).expect("sounds") < loud - 2.0);
        assert_eq!(loudness_dbfs(&[0.0; 960]), None);
    }

    #[test]
    fn deterministic() {
        let s = schedule("say it the same way twice", &P, 3);
        let tones = Tones::new();
        assert_eq!(render(&s, &tones), render(&s, &tones));
    }

    #[test]
    fn pauses_are_true_silence() {
        let s = schedule("ba.   ba", &P, 3);
        let out = render(&s, &Tones::new());
        let first_end = s.events[0].start + s.events[0].len;
        let second = s.events[1].start;
        assert!(first_end < second, "test needs a real gap");
        assert!(out[first_end..second].iter().all(|&x| x == 0.0));
    }

    #[test]
    fn empty_schedule_renders_nothing() {
        let s = schedule("?!", &P, 3);
        assert!(render(&s, &Tones::new()).is_empty());
    }

    #[test]
    fn an_empty_unit_is_silence_not_a_panic() {
        struct Nothing;
        impl UnitSource for Nothing {
            fn unit(&self, _: Unit) -> &[f32] {
                &[]
            }
        }
        let s = schedule("hello", &P, 3);
        assert!(render(&s, &Nothing).iter().all(|&x| x == 0.0));
    }

    #[test]
    fn leads_land_exactly_on_the_beat_grid() {
        // Jitter, declination, `?` and `!` all move the read rate off 1.0,
        // so the clicks are read at fractional positions too.
        let s = schedule("be quick, tell me why? no way!", &P, 5);
        let clicks = Clicks::new();
        let (placements, len) = place(&s, &clicks);
        let out = render(&s, &clicks);
        assert_eq!(out.len(), len);
        assert_eq!(placements.len(), s.events.len());

        // Pre-roll is exactly what the first unit's lead needs: it starts at
        // sample 0, and every unit's lead sits a fixed offset from its slot.
        let pre_roll = placements[0].beat - s.events[0].start;
        assert_eq!(placements.iter().map(|p| p.start).min(), Some(0));
        assert_eq!(len, s.len + pre_roll);
        for (p, e) in placements.iter().zip(&s.events) {
            assert_eq!(p.beat, e.start + pre_roll);
            // The read position at the beat is the lead, to rounding.
            let at_beat = p.phase + to_f64(p.beat - p.start) * p.step;
            let lead = to_f64(Clicks::lead_of(e.unit));
            assert!((at_beat - lead).abs() < 1e-9, "{at_beat} vs {lead}");
            // And the click itself is loudest on the beat.
            let loudest = (p.beat - 2..=p.beat + 2)
                .max_by(|&a, &b| out[a].abs().total_cmp(&out[b].abs()))
                .expect("window");
            assert_eq!(loudest, p.beat, "{:?}", e.unit);
        }
    }

    #[test]
    fn units_read_to_their_end_are_not_tapered() {
        // One event, read at exactly 1.0: "ba" at 18/s is 4000 samples long.
        let flat = VoiceParams {
            pitch: 1.0,
            jitter_cents: 0.0,
            ..P
        };
        let s = schedule("ba", &flat, 3);
        assert_eq!(s.events.len(), 1);
        let dc = |n: usize| {
            struct Dc(Vec<f32>);
            impl UnitSource for Dc {
                fn unit(&self, _: Unit) -> &[f32] {
                    &self.0
                }
            }
            Dc(vec![0.25; n])
        };
        let guard = to_usize(FADE_IN_SECS * f64::from(TTS_SAMPLE_RATE));

        // Shorter than the event: full level right up to a click guard.
        let short = render(&s, &dc(2_000));
        let peak = short.iter().fold(0.0_f32, |m, &x| m.max(x));
        assert!(short[guard..2_000 - guard]
            .iter()
            .all(|x| x.to_bits() == peak.to_bits()));
        assert!(short[2_000..].iter().all(|&x| x == 0.0));

        // Longer than the event: cut, with a fade over the last 25 ms read.
        let long = render(&s, &dc(10_000));
        let fade = to_usize(FADE_OUT_SECS * f64::from(TTS_SAMPLE_RATE));
        let end = s.events[0].len;
        let peak = long.iter().fold(0.0_f32, |m, &x| m.max(x));
        assert!(long[guard..end - fade]
            .iter()
            .all(|x| x.to_bits() == peak.to_bits()));
        assert!(long[end - fade + 1..end]
            .windows(2)
            .all(|w| w[1] < w[0] && w[1] > 0.0));
        assert_eq!(long.len(), end);
    }

    #[test]
    fn hermite_is_exact_on_samples_and_tracks_a_tone() {
        let sr = f64::from(TTS_SAMPLE_RATE);
        let hz = 3_000.0;
        #[allow(clippy::cast_possible_truncation)]
        let tone: Vec<f32> = (0..2_000)
            .map(|n| (std::f64::consts::TAU * hz * f64::from(n) / sr).sin() as f32)
            .collect();
        for (i, &x) in tone.iter().enumerate() {
            assert!((hermite(&tone, to_f64(i)) - x).abs() < f32::EPSILON);
        }
        let linear = |pos: f64| {
            let i = to_usize(pos);
            #[allow(clippy::cast_possible_truncation)]
            let frac = (pos - pos.floor()) as f32;
            tone[i] + (tone[i + 1] - tone[i]) * frac
        };
        let (mut cubic_err, mut linear_err) = (0.0_f64, 0.0_f64);
        for n in 10..1_400 {
            let pos = to_f64(n) * 1.3 + 0.37;
            #[allow(clippy::cast_possible_truncation)]
            let want = (std::f64::consts::TAU * hz * pos / sr).sin() as f32;
            cubic_err += f64::from(hermite(&tone, pos) - want).powi(2);
            linear_err += f64::from(linear(pos) - want).powi(2);
        }
        assert!(
            cubic_err < 0.1 * linear_err,
            "cubic {cubic_err:.3e}, linear {linear_err:.3e}"
        );
    }

    #[test]
    fn soft_limit_is_monotonic_and_bounded() {
        let mut prev = -1.0_f32;
        for k in 0..=4_000 {
            #[allow(clippy::cast_precision_loss)]
            let x = k as f32 / 1_000.0;
            let y = soft_limit(x);
            assert!(y >= prev, "not monotonic at {x}");
            assert!(y <= PEAK_CEILING);
            assert!((soft_limit(-x) + y).abs() < 1e-6, "not odd at {x}");
            prev = y;
        }
        assert!((soft_limit(0.3) - 0.3).abs() < f32::EPSILON);
    }
}
