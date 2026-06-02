//! Harmonizer — adds pitched copies of the voice at fixed musical
//! intervals, summed with the dry "root," producing an actual chord
//! rather than the subtle detune of a chorus. Each harmony voice is an
//! independent phase-vocoder pitch shift (reusing `PitchShift`).
//!
//! Defaults to a **diminished** stack — root plus minor-3rd, diminished-5th,
//! and diminished-7th (3 / 6 / 9 semitones) — the eerie, unresolved sound
//! behind the Coven's "Choir of Ash." The three intervals are adjustable per
//! voice (`v1` / `v2` / `v3`), so it doubles as a general 3-voice harmonizer.
//!
//! The dry path is undelayed; only the harmony voices carry the
//! phase-vocoder window delay, so the user's own voice has no added
//! latency (the chord pad simply lags ~one window behind, inaudibly for
//! a sustained pad).

#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]

use super::{AudioEffect, EffectKind, PitchShift};

/// Number of harmony voices stacked above the dry root.
const VOICES: usize = 3;

/// Scratch capacity — larger than any realistic device callback buffer
/// (the engine sends <= 4096 frames), so `process` never allocates.
const MAX_FRAMES: usize = 8192;

/// Default intervals (semitones): a diminished-7th chord above the root.
const DEFAULT_INTERVALS: [f32; VOICES] = [3.0, 6.0, 9.0];

pub struct Harmonizer {
    enabled: bool,
    mix: f32, // 0..1 — level of the summed harmony voices
    voices: Vec<PitchShift>,
    scratch: Vec<f32>,
    acc: Vec<f32>,
}

impl Harmonizer {
    #[must_use]
    pub fn new() -> Self {
        let mut voices = Vec::with_capacity(VOICES);
        for &st in &DEFAULT_INTERVALS {
            let mut p = PitchShift::new();
            p.set_enabled(true);
            p.set_param("shift", st);
            voices.push(p);
        }
        Self {
            enabled: false,
            mix: 0.7,
            voices,
            scratch: vec![0.0; MAX_FRAMES],
            acc: vec![0.0; MAX_FRAMES],
        }
    }

    fn set_voice_interval(&mut self, index: usize, semitones: f32) {
        if let Some(v) = self.voices.get_mut(index) {
            v.set_param("shift", semitones.clamp(-24.0, 24.0));
        }
    }
}

impl Default for Harmonizer {
    fn default() -> Self {
        Self::new()
    }
}

impl AudioEffect for Harmonizer {
    fn process(&mut self, buffer: &mut [f32], sample_rate: u32) {
        if self.mix <= 0.0 {
            return; // fully dry — no harmony to add
        }
        let n = buffer.len().min(self.scratch.len());
        // Borrow disjoint fields so the per-voice loop can use the
        // scratch/accumulator without aliasing `self`.
        let scratch = &mut self.scratch;
        let acc = &mut self.acc;
        let voices = &mut self.voices;

        for slot in &mut acc[..n] {
            *slot = 0.0;
        }
        for voice in voices.iter_mut() {
            // Each voice pitches a fresh copy of the dry input, then we
            // sum the result into the accumulator.
            scratch[..n].copy_from_slice(&buffer[..n]);
            voice.process(&mut scratch[..n], sample_rate);
            for (a, &s) in acc[..n].iter_mut().zip(&scratch[..n]) {
                *a += s;
            }
        }
        // Root (dry) stays at unity; the harmony stack is mixed on top.
        // Scaled per-voice so a 3-note chord doesn't blow past the dry.
        let wet_gain = self.mix * (1.35 / VOICES as f32);
        for (b, &a) in buffer[..n].iter_mut().zip(&acc[..n]) {
            *b += a * wet_gain;
        }
    }

    fn set_param(&mut self, key: &str, value: f32) {
        match key {
            "mix" => self.mix = (value / 100.0).clamp(0.0, 1.0),
            "v1" => self.set_voice_interval(0, value),
            "v2" => self.set_voice_interval(1, value),
            "v3" => self.set_voice_interval(2, value),
            _ => {}
        }
    }

    fn enabled(&self) -> bool {
        self.enabled
    }

    fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }

    fn kind(&self) -> EffectKind {
        EffectKind::Harmonizer
    }

    fn latency_samples(&self, _sample_rate: u32) -> usize {
        // The dry root passes through undelayed; only the harmony pad
        // lags by the pitch-vocoder window, so the user's voice adds no
        // perceptible latency.
        0
    }
}

#[cfg(test)]
mod tests {
    use super::{AudioEffect, Harmonizer};

    fn sine(len: usize, freq: f32, sample_rate: f32) -> Vec<f32> {
        (0..len)
            .map(|i| {
                let t = i as f32 / sample_rate;
                (2.0 * std::f32::consts::PI * freq * t).sin() * 0.4
            })
            .collect()
    }

    #[test]
    #[allow(clippy::float_cmp)] // mix=0 is a bit-exact passthrough
    fn mix_zero_is_passthrough() {
        let mut h = Harmonizer::new();
        h.set_enabled(true);
        h.set_param("mix", 0.0);
        let input = sine(2048, 220.0, 48_000.0);
        let mut buf = input.clone();
        h.process(&mut buf, 48_000);
        assert_eq!(buf, input);
    }

    #[test]
    fn adds_harmony_energy() {
        let sr = 48_000_f32;
        let mut h = Harmonizer::new();
        h.set_enabled(true);
        h.set_param("mix", 100.0);
        // Drive a continuous tone through successive callback-sized
        // chunks so the phase vocoders warm up (as they do live), then
        // compare the last steady chunk: the chord stack should add
        // clear energy on top of the dry root.
        let full = sine(16_384, 220.0, sr);
        let mut dry_tail = 0.0_f32;
        let mut out_tail = 0.0_f32;
        let mut start = 0;
        while start < full.len() {
            let end = (start + 1024).min(full.len());
            let mut buf = full[start..end].to_vec();
            h.process(&mut buf, sr as u32);
            dry_tail = full[start..end].iter().map(|s| s.abs()).sum();
            out_tail = buf.iter().map(|s| s.abs()).sum();
            start = end;
        }
        assert!(
            out_tail > dry_tail * 1.1,
            "expected harmony to add energy once warm: dry {dry_tail}, out {out_tail}"
        );
        assert!(out_tail.is_finite());
    }

    #[test]
    fn dc_signal_stays_finite() {
        let mut buf = vec![0.3_f32; 8192];
        let mut h = Harmonizer::new();
        h.set_enabled(true);
        h.set_param("mix", 80.0);
        h.process(&mut buf, 48_000);
        assert!(buf.iter().all(|s| s.is_finite()));
    }

    #[test]
    fn adds_no_latency() {
        assert_eq!(Harmonizer::new().latency_samples(48_000), 0);
    }
}
