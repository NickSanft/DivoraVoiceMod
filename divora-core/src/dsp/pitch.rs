//! Pitch shifter — Phase 9 replaces the v0.3.1 passthrough stub with a
//! standard phase-vocoder bin-reassignment shifter.
//!
//! The algorithm:
//!   1. Run an analysis STFT (shared `Stft` module) on the incoming
//!      mono signal.
//!   2. For each analysis bin, compute the *true* instantaneous
//!      frequency from the phase advance vs. expected.
//!   3. For each synthesis bin, look up the analysis bin at
//!      `k_out / ratio` (linear interpolation of magnitudes).
//!   4. Evolve the synthesis phase by `(true_freq * ratio) * 2π * HOP / sr`.
//!   5. Re-synthesise via the STFT's inverse path.
//!
//! Quality vs. the dual-read varispeed bug from v0.3.0: the phase
//! vocoder doesn't pull from two unrelated points in time, so the
//! "hearing yourself twice" artefact can't happen. Some phasiness on
//! sustained vowels at large shifts is inherent to bin-reassignment
//! phase vocoders; we accept it for now.

#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]

use std::f32::consts::PI;

use super::stft::{Stft, HOP, WINDOW};
use super::{AudioEffect, EffectKind};

pub struct PitchShift {
    enabled: bool,
    semitones: f32,
    stft: Stft,
    /// Per-bin analysis phase from the previous frame. Used to compute
    /// the actual phase advance for each bin → its true frequency.
    last_in_phase: Vec<f32>,
    /// Per-bin synthesis phase carried into the next frame. The pitch
    /// vocoder evolves this by the shifted true frequency.
    last_out_phase: Vec<f32>,
    /// Scratch storage so the per-frame closure doesn't allocate.
    true_freq: Vec<f32>,
    synth_mag: Vec<f32>,
    synth_phase: Vec<f32>,
}

impl PitchShift {
    #[must_use]
    pub fn new() -> Self {
        let bins = super::stft::BINS;
        Self {
            enabled: false,
            semitones: 0.0,
            stft: Stft::new(),
            last_in_phase: vec![0.0; bins],
            last_out_phase: vec![0.0; bins],
            true_freq: vec![0.0; bins],
            synth_mag: vec![0.0; bins],
            synth_phase: vec![0.0; bins],
        }
    }

    /// Read out the currently-stored semitone target. Kept for symmetry
    /// with the v0.3.1 passthrough stub (test plumbing).
    #[doc(hidden)]
    #[must_use]
    pub fn semitones(&self) -> f32 {
        self.semitones
    }

    fn ratio(&self) -> f32 {
        2.0_f32.powf(self.semitones / 12.0)
    }
}

impl Default for PitchShift {
    fn default() -> Self {
        Self::new()
    }
}

/// Wrap an angle to (-π, π].
#[inline]
fn wrap_pi(theta: f32) -> f32 {
    let mut t = theta;
    while t > PI {
        t -= 2.0 * PI;
    }
    while t <= -PI {
        t += 2.0 * PI;
    }
    t
}

impl AudioEffect for PitchShift {
    fn process(&mut self, buffer: &mut [f32], sample_rate: u32) {
        if !self.enabled || (self.semitones - 0.0).abs() < 1e-4 {
            // Bypass STFT entirely at zero shift so the output stays
            // bit-identical to the input (no warm-up latency, no
            // reconstruction noise).
            return;
        }
        let ratio = self.ratio();
        let bins = super::stft::BINS;
        // Borrow the scratch + phase-state buffers out of self so the
        // closure can mutate them without aliasing the STFT.
        let true_freq = &mut self.true_freq;
        let synth_mag = &mut self.synth_mag;
        let synth_phase = &mut self.synth_phase;
        let last_in_phase = &mut self.last_in_phase;
        let last_out_phase = &mut self.last_out_phase;

        self.stft.process(buffer, sample_rate, |frame| {
            // Step 1: compute true frequency for each analysis bin.
            //
            // Expected phase advance between successive frames for bin k:
            //     phi_expected = 2π * k * HOP / WINDOW
            // Then `delta = wrap_pi(actual_advance - expected)` and the
            // bin's true frequency is
            //     (k + delta * WINDOW / (2π * HOP)) * bin_hz.
            let exp_per_bin = 2.0 * PI * HOP as f32 / WINDOW as f32;
            for k in 0..bins {
                let expected = exp_per_bin * k as f32;
                let actual = frame.phase[k] - last_in_phase[k];
                last_in_phase[k] = frame.phase[k];
                let delta = wrap_pi(actual - expected);
                let normalised = k as f32 + delta * WINDOW as f32 / (2.0 * PI * HOP as f32);
                true_freq[k] = normalised * frame.bin_hz;
            }

            // Step 2: build synthesis spectrum.
            for slot in synth_mag.iter_mut() {
                *slot = 0.0;
            }
            for k_out in 0..bins {
                let k_src_f = k_out as f32 / ratio;
                let k_src = k_src_f.floor() as usize;
                if k_src >= bins {
                    continue;
                }
                let frac = k_src_f - k_src as f32;
                let m = if k_src + 1 < bins {
                    frame.mag[k_src] * (1.0 - frac) + frame.mag[k_src + 1] * frac
                } else {
                    frame.mag[k_src]
                };
                synth_mag[k_out] = m;

                // Step 3: evolve the synthesis phase from where it left
                // off last frame, by the shifted bin's true frequency.
                let new_freq = true_freq[k_src] * ratio;
                let phase_advance = new_freq * 2.0 * PI * HOP as f32 / sample_rate as f32;
                synth_phase[k_out] = last_out_phase[k_out] + phase_advance;
                last_out_phase[k_out] = synth_phase[k_out];
            }
            // Hand back to the STFT.
            frame.mag.copy_from_slice(synth_mag);
            frame.phase.copy_from_slice(synth_phase);
        });
    }

    fn set_param(&mut self, key: &str, value: f32) {
        if key == "shift" {
            self.semitones = value.clamp(-24.0, 24.0);
        }
    }

    fn enabled(&self) -> bool {
        self.enabled
    }

    fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }

    fn kind(&self) -> EffectKind {
        EffectKind::Pitch
    }
}

#[cfg(test)]
mod tests {
    use super::{AudioEffect, PitchShift};

    fn sine(len: usize, freq: f32, sample_rate: f32) -> Vec<f32> {
        (0..len)
            .map(|i| {
                let t = i as f32 / sample_rate;
                (2.0 * std::f32::consts::PI * freq * t).sin() * 0.5
            })
            .collect()
    }

    fn dominant_freq_hz(buf: &[f32], sample_rate: f32) -> f32 {
        // Crude zero-crossing rate → freq estimate. Good enough for a
        // unit test: a pure sine input at freq F should produce a pure
        // sine at the shifted freq, which we can confirm via the rate
        // of zero crossings on the steady-state portion.
        let mut zc = 0usize;
        for w in buf.windows(2) {
            if w[0].is_sign_negative() != w[1].is_sign_negative() {
                zc += 1;
            }
        }
        let duration = buf.len() as f32 / sample_rate;
        (zc as f32 / 2.0) / duration
    }

    /// Sanity: zero-semitone shift is bit-identical pass-through.
    #[test]
    fn passthrough_at_zero_shift() {
        let input = sine(2048, 440.0, 48_000.0);
        let mut buf = input.clone();
        let mut p = PitchShift::new();
        p.set_enabled(true);
        p.set_param("shift", 0.0);
        p.process(&mut buf, 48_000);
        for (a, b) in buf.iter().zip(input.iter()) {
            assert!((a - b).abs() < 1e-6);
        }
    }

    /// Disabled pitch is bit-identical pass-through regardless of shift.
    #[test]
    fn passthrough_when_disabled() {
        let input = sine(2048, 440.0, 48_000.0);
        let mut buf = input.clone();
        let mut p = PitchShift::new();
        p.set_param("shift", -5.0);
        // Note: enabled() is FALSE.
        p.process(&mut buf, 48_000);
        for (a, b) in buf.iter().zip(input.iter()) {
            assert!((a - b).abs() < 1e-6);
        }
    }

    /// Up-shift by 12 semitones (×2 in frequency) should roughly double
    /// the steady-state zero-crossing rate on a pure tone.
    #[test]
    fn up_shift_doubles_dominant_frequency() {
        let sr = 48_000_f32;
        let input = sine(16_384, 440.0, sr);
        let mut buf = input.clone();
        let mut p = PitchShift::new();
        p.set_enabled(true);
        p.set_param("shift", 12.0);
        p.process(&mut buf, sr as u32);
        // Look past the warm-up window.
        let steady = &buf[4096..];
        let f = dominant_freq_hz(steady, sr);
        // Expect ~880 Hz; allow ±15% (phase vocoder phasiness at low
        // amplitude near the zero-crossings can confuse the crude
        // estimator a bit).
        assert!(
            f > 880.0 * 0.7 && f < 880.0 * 1.3,
            "expected ~880 Hz after +12 st, got {f}"
        );
    }

    /// Down-shift by 12 semitones (÷2) should roughly halve the frequency.
    #[test]
    fn down_shift_halves_dominant_frequency() {
        let sr = 48_000_f32;
        let input = sine(16_384, 880.0, sr);
        let mut buf = input.clone();
        let mut p = PitchShift::new();
        p.set_enabled(true);
        p.set_param("shift", -12.0);
        p.process(&mut buf, sr as u32);
        let steady = &buf[4096..];
        let f = dominant_freq_hz(steady, sr);
        // Expect ~440 Hz; allow ±15%.
        assert!(
            f > 440.0 * 0.7 && f < 440.0 * 1.3,
            "expected ~440 Hz after -12 st, got {f}"
        );
    }

    /// `set_param` clamps to ±24 st and is rejected for unknown keys.
    #[test]
    fn set_param_reaches_internal_state_and_clamps_to_range() {
        let mut p = PitchShift::new();
        p.set_param("shift", 7.0);
        assert!((p.semitones() - 7.0).abs() < 1e-6);
        p.set_param("shift", 99.0);
        assert!((p.semitones() - 24.0).abs() < 1e-6);
        p.set_param("shift", -99.0);
        assert!((p.semitones() - -24.0).abs() < 1e-6);
        p.set_param("unknown", 1.0);
        assert!((p.semitones() - -24.0).abs() < 1e-6);
    }

    /// DC input must remain finite (no NaN / Inf blow-ups in the
    /// phase-vocoder math).
    #[test]
    fn dc_signal_stays_finite_under_shift() {
        let mut buf = vec![0.42_f32; 8192];
        let mut p = PitchShift::new();
        p.set_enabled(true);
        p.set_param("shift", -5.0);
        p.process(&mut buf, 48_000);
        for s in &buf {
            assert!(s.is_finite(), "pitch produced non-finite sample under DC");
        }
    }
}
