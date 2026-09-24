//! End-to-end behaviour of the analysis engine, on synthetic signals only.
//!
//! No assets and no mic: every signal here is built from a formula, so a
//! failure names a property rather than a recording. The per-part tests
//! (pitch, spectrum, baseline, descriptors) live next to their modules; these
//! are the ones about the whole pipeline and the state machine.

// A frozen reading must be bit-identical, not merely close: the point of the
// freeze is that nothing recomputes it.
#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::float_cmp
)]

use std::f32::consts::{PI, TAU};

use super::super::{Descriptor, Metrics, ReadingState, VoiceReading, MAX_DESCRIPTORS};
use super::analyzer::{Analyzer, InputFacts};

/// A synthetic talker: a harmonic stack, gated into bursts, optionally with
/// vibrato. Enough structure for the analyzer to have something to measure and
/// simple enough that every metric has a closed form to check against.
#[derive(Clone)]
struct Voice {
    rate: f32,
    f0: f32,
    /// Amplitude of harmonic k, 1-indexed. Normalised to unit RMS at render.
    harmonics: Vec<f32>,
    /// Target RMS of the voiced portion.
    amp: f32,
    /// Peak vibrato excursion, semitones (so peak-to-peak is twice this).
    vibrato_st: f32,
    vibrato_hz: f32,
    /// Voiced burst and the gap after it, seconds. A zero gap is a
    /// continuous tone.
    burst_s: f32,
    gap_s: f32,
}

impl Voice {
    fn talking(rate: f32) -> Self {
        Self {
            rate,
            f0: 140.0,
            harmonics: vec![1.0, 0.6, 0.4, 0.25, 0.15, 0.1],
            amp: 0.1,
            vibrato_st: 0.0,
            vibrato_hz: 0.0,
            burst_s: 0.28,
            gap_s: 0.12,
        }
    }

    fn tone(rate: f32, f0: f32) -> Self {
        Self {
            rate,
            f0,
            harmonics: vec![1.0],
            amp: 0.1,
            vibrato_st: 0.0,
            vibrato_hz: 0.0,
            burst_s: 1.0e9,
            gap_s: 0.0,
        }
    }

    fn render(&self, secs: f32) -> Vec<f32> {
        let n = (secs * self.rate) as usize;
        let norm = self
            .harmonics
            .iter()
            .map(|a| a * a)
            .sum::<f32>()
            .sqrt()
            .max(f32::MIN_POSITIVE);
        let period = self.burst_s + self.gap_s;
        let edge = 0.02f32.min(self.burst_s * 0.4);
        let mut phase = 0.0f32;
        let mut out = Vec::with_capacity(n);
        for i in 0..n {
            let t = i as f32 / self.rate;
            let hz =
                self.f0 * ((self.vibrato_st * (TAU * self.vibrato_hz * t).sin()) / 12.0).exp2();
            phase += TAU * hz / self.rate;
            if phase > TAU {
                phase -= TAU;
            }
            let pos = t % period;
            // Raised-cosine edges, so a burst boundary is a syllable and not
            // a click with a spectrum of its own.
            let env = if pos >= self.burst_s {
                0.0
            } else if pos < edge {
                0.5 - 0.5 * (PI * pos / edge).cos()
            } else if pos > self.burst_s - edge {
                0.5 - 0.5 * (PI * (self.burst_s - pos) / edge).cos()
            } else {
                1.0
            };
            let mut s = 0.0f32;
            for (k, a) in self.harmonics.iter().enumerate() {
                s = a.mul_add((((k + 1) as f32) * phase).sin(), s);
            }
            // sqrt(2) puts a unit-amplitude sine at unit RMS.
            out.push(self.amp * env * s * std::f32::consts::SQRT_2 / norm);
        }
        out
    }
}

/// Deterministic white noise at a given RMS.
fn noise(rate: f32, secs: f32, amp: f32) -> Vec<f32> {
    let n = (secs * rate) as usize;
    let mut state = 0x853c_49e6_748f_ea9bu64;
    (0..n)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            amp * (((state >> 40) as f32) / 8_388_608.0 - 1.0)
        })
        .collect()
}

fn scaled(signal: &[f32], gain: f32) -> Vec<f32> {
    signal.iter().map(|s| s * gain).collect()
}

/// Feed both taps in 10 ms blocks, the size a device callback delivers.
fn feed(a: &mut Analyzer, dry: &[f32], wet: &[f32], facts: InputFacts) -> Vec<VoiceReading> {
    let block = 480;
    let mut out = Vec::new();
    let n = dry.len().min(wet.len());
    let mut start = 0;
    while start < n {
        let end = (start + block).min(n);
        if let Some(r) = a.observe(&dry[start..end], &wet[start..end], facts) {
            out.push(r);
        }
        start = end;
    }
    out
}

/// The common case: one signal on both taps, engine running, nothing muted.
fn run(rate: u32, signal: &[f32]) -> (Analyzer, Vec<VoiceReading>) {
    let mut a = Analyzer::new(rate);
    let readings = feed(&mut a, signal, signal, InputFacts::default());
    (a, readings)
}

fn last_speaking(readings: &[VoiceReading]) -> &VoiceReading {
    readings
        .iter()
        .rev()
        .find(|r| r.state == ReadingState::Speaking)
        .expect("nothing in this run was read as speech")
}

// ---------------------------------------------------------------- metrics

#[test]
fn a_pure_sine_reads_its_own_pitch() {
    for &hz in &[95.0f32, 150.0, 240.0] {
        let sig = Voice::tone(48_000.0, hz).render(4.0);
        let (_, readings) = run(48_000, &sig);
        let m = last_speaking(&readings).dry;
        assert!(
            (m.f0_hz - hz).abs() < hz * 0.01,
            "{hz} Hz sine read {} Hz",
            m.f0_hz
        );
        assert!(m.voiced_ratio > 0.9, "voiced ratio {}", m.voiced_ratio);
        assert!(
            m.f0_range_st < 0.5,
            "a steady tone spread {} st",
            m.f0_range_st
        );
    }
}

#[test]
fn a_harmonic_stack_is_not_read_an_octave_off() {
    let mut v = Voice::talking(48_000.0);
    v.f0 = 120.0;
    // Second harmonic twice the fundamental: the case a bare autocorrelation
    // peak reads at 240 Hz every time.
    v.harmonics = vec![0.5, 1.0, 0.5, 0.25];
    let (_, readings) = run(48_000, &v.render(5.0));
    let f0 = last_speaking(&readings).dry.f0_hz;
    assert!(
        (f0 - 120.0).abs() < 8.0,
        "read {f0} Hz; an octave would be 240 or 60"
    );
}

#[test]
fn white_noise_is_not_read_as_speech() {
    let sig = noise(48_000.0, 4.0, 0.1);
    let (_, readings) = run(48_000, &sig);
    let last = readings.last().expect("no readings");
    assert_ne!(last.state, ReadingState::Speaking, "noise read as speech");
    assert!(
        !readings.iter().any(|r| r.state == ReadingState::Speaking),
        "some window of white noise was read as speech"
    );
}

#[test]
fn three_decibels_louder_moves_the_level_by_three_decibels() {
    let sig = Voice::talking(48_000.0).render(5.0);
    let loud = scaled(&sig, 10f32.powf(3.0 / 20.0));
    let quiet_level = {
        let (_, r) = run(48_000, &sig);
        last_speaking(&r).dry.energy_dbfs
    };
    let loud_level = {
        let (_, r) = run(48_000, &loud);
        last_speaking(&r).dry.energy_dbfs
    };
    let moved = loud_level - quiet_level;
    assert!(
        (moved - 3.0).abs() < 0.2,
        "a +3 dB input moved the readout by {moved} dB"
    );
}

#[test]
fn vibrato_widens_the_pitch_range_by_the_amount_it_carries() {
    // A sinusoidal excursion of +/- a semitones is arcsine-distributed, whose
    // p90 - p10 is 1.902 * a. With a = 2 st that is 3.80 st, which is what
    // f0_range_st should report; the tolerance covers the smearing of a 60 ms
    // pitch frame across a moving fundamental.
    let mut v = Voice::tone(48_000.0, 180.0);
    v.vibrato_st = 2.0;
    v.vibrato_hz = 1.0;
    let (_, readings) = run(48_000, &v.render(6.0));
    let wide = last_speaking(&readings).dry.f0_range_st;
    assert!(
        (3.0..=4.6).contains(&wide),
        "+/-2 st vibrato read a range of {wide} st, expected ~3.80"
    );

    let steady = {
        let (_, r) = run(48_000, &Voice::tone(48_000.0, 180.0).render(6.0));
        last_speaking(&r).dry.f0_range_st
    };
    assert!(
        wide > steady + 2.5,
        "vibrato ({wide} st) barely beat a steady tone ({steady} st)"
    );
}

#[test]
fn a_bright_signal_reads_brighter_than_a_dark_one_of_the_same_level() {
    let rate = 48_000.0;
    let mut dark = Voice::talking(rate);
    dark.harmonics = vec![1.0, 0.5, 0.2, 0.05];
    let mut bright = dark.clone();
    // Same fundamental, same RMS (render normalises), energy moved upward.
    bright.harmonics = vec![0.2, 0.4, 0.8, 1.0, 1.0, 0.9, 0.8, 0.7];

    let (_, dark_r) = run(48_000, &dark.render(4.0));
    let (_, bright_r) = run(48_000, &bright.render(4.0));
    let d = last_speaking(&dark_r).dry;
    let b = last_speaking(&bright_r).dry;
    assert!(
        (d.energy_dbfs - b.energy_dbfs).abs() < 1.5,
        "the two signals were not at the same level: {} vs {} dBFS",
        d.energy_dbfs,
        b.energy_dbfs
    );
    assert!(
        b.brightness_hz > d.brightness_hz * 1.5,
        "bright read {} Hz, dark read {} Hz",
        b.brightness_hz,
        d.brightness_hz
    );
}

#[test]
fn pace_counts_the_onsets_that_were_planted() {
    let mut v = Voice::talking(48_000.0);
    v.burst_s = 0.20;
    v.gap_s = 0.10; // 3.33 bursts per second
    let (_, readings) = run(48_000, &v.render(5.0));
    let pace = last_speaking(&readings).dry.pace_ops;
    assert!(
        (pace - 3.33).abs() < 0.8,
        "planted 3.33 onsets/s, read {pace}"
    );
}

#[test]
fn every_supported_sample_rate_agrees() {
    let mut reference: Option<Metrics> = None;
    for &rate in &[44_100u32, 48_000, 96_000] {
        let mut v = Voice::talking(rate as f32);
        v.f0 = 165.0;
        let (_, readings) = run(rate, &v.render(5.0));
        let m = last_speaking(&readings).dry;
        if let Some(r) = reference {
            assert!(
                (m.f0_hz - r.f0_hz).abs() < 1.0,
                "{rate}: f0 {} vs {}",
                m.f0_hz,
                r.f0_hz
            );
            assert!(
                (m.energy_dbfs - r.energy_dbfs).abs() < 0.5,
                "{rate}: level {} vs {} dBFS",
                m.energy_dbfs,
                r.energy_dbfs
            );
            assert!(
                (m.brightness_hz - r.brightness_hz).abs() < r.brightness_hz * 0.05,
                "{rate}: brightness {} vs {} Hz",
                m.brightness_hz,
                r.brightness_hz
            );
            assert!(
                (m.pace_ops - r.pace_ops).abs() < 0.4,
                "{rate}: pace {} vs {}",
                m.pace_ops,
                r.pace_ops
            );
            assert!(
                (m.voiced_ratio - r.voiced_ratio).abs() < 0.1,
                "{rate}: voiced ratio {} vs {}",
                m.voiced_ratio,
                r.voiced_ratio
            );
        } else {
            reference = Some(m);
        }
    }
}

#[test]
fn the_same_audio_twice_reads_the_same() {
    let sig = Voice::talking(48_000.0).render(4.0);
    let (_, first) = run(48_000, &sig);
    let (_, second) = run(48_000, &sig);
    assert_eq!(first.len(), second.len());
    assert_eq!(first, second, "the analyzer is not deterministic");
}

// ------------------------------------------------------------ state machine

#[test]
fn silence_reads_quiet_and_freezes_the_last_speaking_window() {
    let rate = 48_000.0;
    let mut a = Analyzer::new(48_000);
    let speech = Voice::talking(rate).render(5.0);
    let spoken = feed(&mut a, &speech, &speech, InputFacts::default());
    // A reading taken in the middle of the talking, to compare the frozen one
    // against. Freezing a window that has already half emptied would pass a
    // "it stopped moving" test while still putting the wrong numbers on
    // screen for the rest of the session.
    let mid = last_speaking(&spoken).dry;

    // Room tone, 20 dB under the speech floor: present, not speech, not a
    // mute. The window still holds speech for a moment after it stops, so the
    // freeze is measured from the last live reading onward.
    let room = noise(rate, 6.0, 0.0003);
    let quiet = feed(&mut a, &room, &room, InputFacts::default());
    let split = quiet
        .iter()
        .rposition(|r| r.state == ReadingState::Speaking)
        .expect("the window never carried the speech across");
    let frozen = quiet[split].dry;
    // What froze has to still be a reading of speech, not of the pause that
    // was already half-filling the window when it stopped.
    assert!(
        (frozen.pace_ops - mid.pace_ops).abs() < mid.pace_ops * 0.15,
        "the frozen pace ({}) had already drifted from mid-speech ({})",
        frozen.pace_ops,
        mid.pace_ops
    );
    assert!(
        (frozen.energy_range_db - mid.energy_range_db).abs() < 1.5,
        "the frozen level span ({} dB) had drifted from mid-speech ({} dB)",
        frozen.energy_range_db,
        mid.energy_range_db
    );
    assert!(
        frozen.voiced_ratio > mid.voiced_ratio * 0.8,
        "the frozen voiced ratio ({}) had drained from mid-speech ({})",
        frozen.voiced_ratio,
        mid.voiced_ratio
    );

    let tail = &quiet[split + 1..];
    assert!(tail.len() > 8, "only {} readings after speech", tail.len());
    // And it stops calling it speech promptly, rather than trailing the
    // speaker by the length of its own window.
    let elapsed = (split + 1) as f32 * 0.25;
    assert!(
        elapsed < 1.25,
        "it was still reading speech {elapsed} s after it stopped"
    );
    for r in tail {
        assert_eq!(
            r.state,
            ReadingState::Quiet,
            "room tone read as {:?}",
            r.state
        );
        assert!(r.held, "a pause produced a live reading");
        assert!(r.descriptors.is_empty(), "a pause described something");
        assert_eq!(
            r.dry, frozen,
            "the readout drifted during a pause: {:?}",
            r.dry
        );
    }
    // And specifically: it did not slide toward the words a decaying panel
    // would end up printing.
    let end = tail.last().unwrap();
    assert_eq!(end.dry.energy_dbfs, frozen.energy_dbfs);
    assert_eq!(end.dry.f0_range_st, frozen.f0_range_st);
    assert_eq!(end.dry.voiced_ratio, frozen.voiced_ratio);
}

#[test]
fn digital_silence_reads_muted_and_a_live_room_does_not() {
    let rate = 48_000.0;
    let mut a = Analyzer::new(48_000);
    let zeros = vec![0.0f32; 48_000 * 2];
    let readings = feed(&mut a, &zeros, &zeros, InputFacts::default());
    assert_eq!(readings.last().unwrap().state, ReadingState::Muted);

    let mut b = Analyzer::new(48_000);
    let room = noise(rate, 2.0, 0.0003);
    let readings = feed(&mut b, &room, &room, InputFacts::default());
    assert_eq!(
        readings.last().unwrap().state,
        ReadingState::Quiet,
        "a quiet room was called muted"
    );
}

#[test]
fn the_engine_facts_decide_stopped_and_muted_not_the_signal() {
    let speech = Voice::talking(48_000.0).render(3.0);

    // Speech on the wire, but the engine says muted.
    let mut a = Analyzer::new(48_000);
    let facts = InputFacts {
        input_muted: true,
        ..InputFacts::default()
    };
    let readings = feed(&mut a, &speech, &speech, facts);
    assert!(
        readings.iter().all(|r| r.state == ReadingState::Muted),
        "a muted input was read from the signal instead of the fact"
    );

    // Digital silence with the engine running is Muted, never Stopped —
    // guessing "stopped" from silence is how a panel tells someone their
    // engine died when it is fine.
    let mut b = Analyzer::new(48_000);
    let zeros = vec![0.0f32; 48_000 * 2];
    let readings = feed(&mut b, &zeros, &zeros, InputFacts::default());
    assert_eq!(readings.last().unwrap().state, ReadingState::Muted);

    // Only the fact produces Stopped.
    let stopped = b
        .observe(
            &[],
            &[],
            InputFacts {
                engine_running: false,
                ..InputFacts::default()
            },
        )
        .expect("no reading for a stopped engine");
    assert_eq!(stopped.state, ReadingState::Stopped);
}

#[test]
fn a_state_change_is_reported_without_waiting_for_the_next_window() {
    let mut a = Analyzer::new(48_000);
    // One 10 ms block: far short of the 250 ms emit interval, but the state
    // went from its initial Stopped to Quiet.
    let block = vec![0.001f32; 480];
    assert!(a.observe(&block, &block, InputFacts::default()).is_some());
    // The next block changes nothing, so nothing is emitted.
    assert!(a.observe(&block, &block, InputFacts::default()).is_none());
}

#[test]
fn an_engine_restart_forgets_the_speaker() {
    let rate = 48_000.0;
    let mut a = Analyzer::new(48_000);
    let speech = Voice::talking(rate).render(16.0);
    let readings = feed(&mut a, &speech, &speech, InputFacts::default());
    assert!(
        last_speaking(&readings).calibrated,
        "16 s of speech did not calibrate"
    );

    let stopped = a
        .observe(
            &[],
            &[],
            InputFacts {
                engine_running: false,
                ..InputFacts::default()
            },
        )
        .expect("no reading");
    assert_eq!(stopped.state, ReadingState::Stopped);
    assert!(
        !stopped.calibrated,
        "the baseline survived an engine restart"
    );
    assert!(!stopped.held, "the last reading survived an engine restart");
}

// ------------------------------------------------------------- the wet tap

#[test]
fn the_wet_tap_reads_the_chain_and_the_dry_tap_reads_the_speaker() {
    let rate = 48_000.0;
    let dry = Voice::talking(rate);
    let mut wet = dry.clone();
    // What a +5 semitone preset does, and nothing else changed.
    wet.f0 = dry.f0 * 2f32.powf(5.0 / 12.0);
    let mut a = Analyzer::new(48_000);
    let readings = feed(
        &mut a,
        &dry.render(5.0),
        &wet.render(5.0),
        InputFacts::default(),
    );
    let r = last_speaking(&readings);
    let shift = 12.0 * (r.wet.f0_hz / r.dry.f0_hz).log2();
    assert!(
        (shift - 5.0).abs() < 0.5,
        "dry {} Hz, wet {} Hz: {shift} st apart",
        r.dry.f0_hz,
        r.wet.f0_hz
    );
    assert!(
        (r.dry.f0_hz - dry.f0).abs() < 4.0,
        "the dry tap moved with the preset: {} Hz",
        r.dry.f0_hz
    );
}

#[test]
fn a_bypassed_chain_is_labelled_not_diagnosed() {
    // Push-to-modulate, key up: the chain is out of the path, so the wet tap
    // is carrying the dry signal. That is not the preset having stopped
    // working, and the reading says which it is.
    let sig = Voice::talking(48_000.0).render(4.0);
    let mut a = Analyzer::new(48_000);
    let facts = InputFacts {
        chain_bypassed: true,
        ..InputFacts::default()
    };
    let readings = feed(&mut a, &sig, &sig, facts);
    let r = last_speaking(&readings);
    assert!(r.wet_bypassed, "a bypassed chain was not flagged");
    assert_eq!(
        r.state,
        ReadingState::Speaking,
        "the dry tap still has speech"
    );
    assert!(
        (r.wet.f0_hz - r.dry.f0_hz).abs() < 1.0,
        "a passthrough read differently from its own input"
    );
}

#[test]
fn a_preset_switch_moves_the_wet_reading_within_half_a_second() {
    let rate = 48_000.0;
    let dry = Voice::talking(rate);
    let mut a = Analyzer::new(48_000);
    let before = feed(
        &mut a,
        &dry.render(4.0),
        &dry.render(4.0),
        InputFacts::default(),
    );
    assert!(last_speaking(&before).wet_settled);
    assert!((last_speaking(&before).wet.f0_hz - dry.f0).abs() < 4.0);

    // The user picks a +7 semitone preset.
    let mut shifted = dry.clone();
    shifted.f0 = dry.f0 * 2f32.powf(7.0 / 12.0);
    a.note_chain_changed();
    let after = feed(
        &mut a,
        &dry.render(0.75),
        &shifted.render(0.75),
        InputFacts::default(),
    );
    let r = after.last().expect("no reading after the switch");
    assert!(
        !r.wet_settled,
        "the wet window claimed to be full 0.75 s after a chain change"
    );
    let shift = 12.0 * (r.wet.f0_hz / dry.f0).log2();
    assert!(
        (shift - 7.0).abs() < 1.0,
        "0.75 s after a +7 st switch the wet tap read {shift} st"
    );
}

// ------------------------------------------------------------- descriptors

/// Enough speech to calibrate: 12 observations at one per second of speech.
fn calibrate(a: &mut Analyzer, v: &Voice) -> Vec<VoiceReading> {
    let sig = v.render(16.0);
    feed(a, &sig, &sig, InputFacts::default())
}

#[test]
fn nothing_is_described_until_there_is_a_baseline() {
    let v = Voice::talking(48_000.0);
    let mut a = Analyzer::new(48_000);
    let sig = v.render(4.0);
    let readings = feed(&mut a, &sig, &sig, InputFacts::default());
    for r in readings
        .iter()
        .filter(|r| r.state == ReadingState::Speaking)
    {
        assert!(!r.calibrated);
        assert!(
            r.descriptors.is_empty(),
            "described {:?} against too little data",
            r.descriptors
        );
    }
}

#[test]
fn a_calibrated_speaker_gets_a_phrase_of_at_most_three_words() {
    let v = Voice::talking(48_000.0);
    let mut a = Analyzer::new(48_000);
    let readings = calibrate(&mut a, &v);
    let r = last_speaking(&readings);
    assert!(r.calibrated);
    assert!(!r.descriptors.is_empty());
    assert!(
        r.descriptors.len() <= MAX_DESCRIPTORS,
        "{} words: {:?}",
        r.descriptors.len(),
        r.descriptors
    );
    // Against its own baseline, an unchanging talker is not remarkable.
    assert_eq!(r.descriptors, vec![Descriptor::Steady]);
}

#[test]
fn a_real_change_is_described_and_a_small_one_is_not() {
    let rate = 48_000.0;
    let v = Voice::talking(rate);
    let mut a = Analyzer::new(48_000);
    calibrate(&mut a, &v);

    // +10 dB is well past the 6 dB threshold.
    let loud = scaled(&v.render(4.0), 10f32.powf(10.0 / 20.0));
    let readings = feed(&mut a, &loud, &loud, InputFacts::default());
    assert!(
        last_speaking(&readings)
            .descriptors
            .contains(&Descriptor::Loud),
        "a 10 dB rise was not described: {:?}",
        last_speaking(&readings).descriptors
    );

    // +2 dB is inside anyone's mic-distance drift.
    let mut b = Analyzer::new(48_000);
    calibrate(&mut b, &v);
    let nudged = scaled(&v.render(4.0), 10f32.powf(2.0 / 20.0));
    let readings = feed(&mut b, &nudged, &nudged, InputFacts::default());
    assert_eq!(
        last_speaking(&readings).descriptors,
        vec![Descriptor::Steady],
        "a 2 dB nudge was reported as something"
    );
}

#[test]
fn the_phrase_does_not_flicker_under_a_small_perturbation() {
    let rate = 48_000.0;
    let v = Voice::talking(rate);
    let mut a = Analyzer::new(48_000);
    calibrate(&mut a, &v);

    // The level wobbles by +/-0.5 dB block to block, which is less than a
    // speaker breathing. Nothing should be said about it.
    let mut phrases: Vec<Vec<Descriptor>> = Vec::new();
    for round in 0..6 {
        let db = if round % 2 == 0 { 0.5 } else { -0.5 };
        let sig = scaled(&v.render(1.0), 10f32.powf(db / 20.0));
        for r in feed(&mut a, &sig, &sig, InputFacts::default()) {
            if r.state == ReadingState::Speaking {
                phrases.push(r.descriptors);
            }
        }
    }
    assert!(phrases.len() > 10, "only {} phrases", phrases.len());
    for (i, p) in phrases.iter().enumerate() {
        assert_eq!(
            *p,
            vec![Descriptor::Steady],
            "reading {i} said {p:?} about a half-decibel wobble"
        );
    }
}

#[test]
fn a_word_holds_through_the_band_below_the_threshold_that_raised_it() {
    let rate = 48_000.0;
    let v = Voice::talking(rate);
    let mut a = Analyzer::new(48_000);
    calibrate(&mut a, &v);

    let at = |a: &mut Analyzer, db: f32| -> Vec<Descriptor> {
        let sig = scaled(&v.render(3.0), 10f32.powf(db / 20.0));
        let readings = feed(a, &sig, &sig, InputFacts::default());
        last_speaking(&readings).descriptors.clone()
    };

    // Past the 6 dB threshold: the word appears.
    assert!(
        at(&mut a, 8.0).contains(&Descriptor::Loud),
        "a clear 8 dB rise was not described"
    );
    // Back inside the band (under 6 dB on, over 3.6 dB off): it must hold,
    // or the phrase blinks every time a speaker crosses their own threshold.
    assert!(
        at(&mut a, 4.5).contains(&Descriptor::Loud),
        "the word blinked out at 4.5 dB, inside its own hysteresis band"
    );
    // Clearly back to normal: it goes.
    assert_eq!(at(&mut a, 1.0), vec![Descriptor::Steady]);
}

// -------------------------------------------------------------- the shape

#[test]
fn a_reading_carries_nothing_but_the_reading() {
    // Whatever else changes, what leaves this crate is measurements and
    // words from the closed vocabulary. No audio, no baseline, no identity,
    // nothing that could be written anywhere.
    let sig = Voice::talking(48_000.0).render(4.0);
    let (_, readings) = run(48_000, &sig);
    let value = serde_json::to_value(last_speaking(&readings)).expect("serialise");
    let mut keys: Vec<&str> = value
        .as_object()
        .expect("an object")
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort_unstable();
    assert_eq!(
        keys,
        [
            "calibrated",
            "descriptors",
            "dry",
            "held",
            "held_descriptors",
            "state",
            "wet",
            "wet_bypassed",
            "wet_settled",
        ]
    );
}

#[test]
fn an_unmeasured_level_does_not_survive_json() {
    // Recorded, not worked around: `serde_json` has no way to write -inf, so
    // `Metrics::silent()` crosses the bridge with `energy_dbfs: null` and
    // will not parse back. Anything consuming a reading has to treat that
    // field as nullable — it is what an idle panel sends before anyone has
    // spoken.
    let json = serde_json::to_string(&Metrics::silent()).expect("serialise");
    assert!(json.contains("null"), "{json}");
    assert!(serde_json::from_str::<Metrics>(&json).is_err());
}

#[test]
fn the_analyzer_fits_in_a_few_hundred_kilobytes() {
    let a = Analyzer::new(96_000);
    let bytes = a.heap_bytes();
    assert!(
        bytes < 512 * 1024,
        "the analyzer holds {bytes} bytes at 96 kHz"
    );
}
