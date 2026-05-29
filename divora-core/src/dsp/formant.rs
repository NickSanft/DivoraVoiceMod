//! Formant shifter — Phase 9 replaces the v0.3 three-bandpass coloration
//! with a phase-vocoder spectrum-warping shifter.
//!
//! Algorithm: in each STFT frame, compute a smooth spectral envelope
//! (the formant track) by low-pass filtering the magnitude spectrum on
//! the frequency axis, then resample that envelope along the frequency
//! axis by the formant ratio. The fine structure (which carries the
//! pitch) stays where it is; only the broad shape moves. End result:
//! the voice's *vowel colour* darkens / brightens without the pitch
//! changing.
//!
//! This is a simplified take on LPC-envelope warping: instead of
//! fitting an LPC model and re-driving the residual through the
//! warped filter, we approximate the envelope with a moving-average
//! smoothed spectrum and warp the magnitudes directly. Cheaper than
//! full LPC and noticeably better than the old bandpass colouring.

#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::needless_range_loop
)]

use super::stft::{Stft, BINS};
use super::{AudioEffect, EffectKind};

/// Smoothing window for the envelope estimator. Larger = smoother
/// envelope (more "formant-like", less "tracks every harmonic").
const ENV_SMOOTH_HALF: usize = 16;

pub struct FormantShift {
    enabled: bool,
    shift: f32,
    stft: Stft,
    envelope: Vec<f32>,
    excitation: Vec<f32>,
    warped_env: Vec<f32>,
}

impl FormantShift {
    #[must_use]
    pub fn new() -> Self {
        Self {
            enabled: false,
            shift: 0.0,
            stft: Stft::new(),
            envelope: vec![0.0; BINS],
            excitation: vec![0.0; BINS],
            warped_env: vec![0.0; BINS],
        }
    }

    #[doc(hidden)]
    #[must_use]
    pub fn shift(&self) -> f32 {
        self.shift
    }

    fn ratio(&self) -> f32 {
        // Same control mapping as the v0.3 effect: ±12 ≈ one octave of
        // formant movement.
        2.0_f32.powf(self.shift / 12.0)
    }
}

impl Default for FormantShift {
    fn default() -> Self {
        Self::new()
    }
}

/// Smooth `mag` using a 2*half+1 moving average (with edge clamping).
/// Used to extract a coarse spectral envelope.
fn smooth_spectrum(mag: &[f32], out: &mut [f32], half: usize) {
    let n = mag.len();
    if n == 0 {
        return;
    }
    for i in 0..n {
        let lo = i.saturating_sub(half);
        let hi = (i + half + 1).min(n);
        let mut sum = 0.0_f32;
        for s in &mag[lo..hi] {
            sum += *s;
        }
        let span = (hi - lo) as f32;
        out[i] = sum / span;
    }
}

impl AudioEffect for FormantShift {
    fn process(&mut self, buffer: &mut [f32], sample_rate: u32) {
        if !self.enabled || (self.shift - 0.0).abs() < 1e-4 {
            // Bypass at zero shift so output is bit-identical to input.
            return;
        }
        let ratio = self.ratio();
        let envelope = &mut self.envelope;
        let excitation = &mut self.excitation;
        let warped_env = &mut self.warped_env;

        self.stft.process(buffer, sample_rate, |frame| {
            let bins = frame.mag.len();
            // 1. Estimate the spectral envelope by smoothing |X|.
            smooth_spectrum(frame.mag, envelope, ENV_SMOOTH_HALF);

            // 2. Excitation = magnitude divided by envelope. This is
            // the "fine structure" (harmonic peaks) with the formant
            // colour removed.
            for k in 0..bins {
                let env = envelope[k].max(1e-6);
                excitation[k] = frame.mag[k] / env;
            }

            // 3. Warp the envelope along the frequency axis by `ratio`.
            // Shifting envelope[k] → warped_env[k * ratio] effectively
            // moves the formants up by `ratio` (and vice versa for
            // ratio < 1).
            for k in 0..bins {
                let k_src_f = k as f32 / ratio;
                let k_src = k_src_f.floor() as usize;
                if k_src >= bins {
                    warped_env[k] = envelope[bins - 1];
                    continue;
                }
                let frac = k_src_f - k_src as f32;
                let lo = envelope[k_src];
                let hi = if k_src + 1 < bins {
                    envelope[k_src + 1]
                } else {
                    lo
                };
                warped_env[k] = lo * (1.0 - frac) + hi * frac;
            }

            // 4. Re-impose the (unchanged) excitation on the warped
            // envelope. Phases pass through untouched — the formant
            // shifter doesn't move energy across bins, so the existing
            // phases of `frame.phase` are still meaningful.
            for k in 0..bins {
                frame.mag[k] = warped_env[k] * excitation[k];
            }
        });
    }

    fn set_param(&mut self, key: &str, value: f32) {
        if key == "shift" {
            self.shift = value.clamp(-24.0, 24.0);
        }
    }

    fn enabled(&self) -> bool {
        self.enabled
    }

    fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }

    fn kind(&self) -> EffectKind {
        EffectKind::Formant
    }
}

#[cfg(test)]
mod tests {
    use super::{smooth_spectrum, AudioEffect, FormantShift};

    fn sine(len: usize, freq: f32, sample_rate: f32) -> Vec<f32> {
        (0..len)
            .map(|i| {
                let t = i as f32 / sample_rate;
                (2.0 * std::f32::consts::PI * freq * t).sin() * 0.5
            })
            .collect()
    }

    /// Zero shift = bit-identical passthrough (matches the bypass policy
    /// the chain relies on).
    #[test]
    fn passthrough_at_zero_shift() {
        let input = sine(2048, 440.0, 48_000.0);
        let mut buf = input.clone();
        let mut f = FormantShift::new();
        f.set_enabled(true);
        f.set_param("shift", 0.0);
        f.process(&mut buf, 48_000);
        for (a, b) in buf.iter().zip(input.iter()) {
            assert!((a - b).abs() < 1e-6);
        }
    }

    #[test]
    fn passthrough_when_disabled() {
        let input = sine(2048, 440.0, 48_000.0);
        let mut buf = input.clone();
        let mut f = FormantShift::new();
        f.set_param("shift", 5.0);
        // enabled = false
        f.process(&mut buf, 48_000);
        for (a, b) in buf.iter().zip(input.iter()) {
            assert!((a - b).abs() < 1e-6);
        }
    }

    /// Non-zero shift must produce a finite-valued output (no NaN / Inf
    /// from the envelope/excitation division).
    #[test]
    fn shifted_signal_stays_finite() {
        let mut buf = sine(8192, 200.0, 48_000.0);
        let mut f = FormantShift::new();
        f.set_enabled(true);
        f.set_param("shift", 5.0);
        f.process(&mut buf, 48_000);
        for s in &buf {
            assert!(s.is_finite(), "non-finite sample {s} under formant shift");
        }
    }

    /// DC stays bounded too.
    #[test]
    fn dc_signal_stays_finite_under_shift() {
        let mut buf = vec![0.3_f32; 8192];
        let mut f = FormantShift::new();
        f.set_enabled(true);
        f.set_param("shift", -7.0);
        f.process(&mut buf, 48_000);
        for s in &buf {
            assert!(s.is_finite());
        }
    }

    /// The pitch (zero-crossing rate) of a pure sine should NOT change
    /// substantially under formant shifting — that's the whole point.
    /// Compare against the pitch shifter, which would double the rate.
    #[test]
    fn formant_does_not_change_fundamental_frequency() {
        let sr = 48_000_f32;
        let input = sine(16_384, 440.0, sr);
        let mut buf = input.clone();
        let mut f = FormantShift::new();
        f.set_enabled(true);
        f.set_param("shift", 7.0); // up 7 st of formant
        f.process(&mut buf, sr as u32);

        // Steady-state zero-crossing rate after warm-up.
        let steady = &buf[4096..];
        let mut zc = 0usize;
        for w in steady.windows(2) {
            if w[0].is_sign_negative() != w[1].is_sign_negative() {
                zc += 1;
            }
        }
        let duration = steady.len() as f32 / sr;
        let est = (zc as f32 / 2.0) / duration;
        // Expect ~440 Hz (formant shift shouldn't move pitch); allow ±25%
        // because the envelope warp can ring slightly at the formant
        // ratio's "edge" of the spectrum.
        assert!(
            est > 440.0 * 0.75 && est < 440.0 * 1.25,
            "formant shifted pitch unexpectedly; got {est} Hz",
        );
    }

    /// `smooth_spectrum` should leave a constant input as a constant.
    #[test]
    fn smooth_spectrum_preserves_dc() {
        let input = vec![0.5_f32; 64];
        let mut out = vec![0.0_f32; 64];
        smooth_spectrum(&input, &mut out, 8);
        for s in &out {
            assert!((s - 0.5).abs() < 1e-3);
        }
    }

    #[test]
    fn set_param_clamps_to_range() {
        let mut f = FormantShift::new();
        f.set_param("shift", 99.0);
        assert!((f.shift() - 24.0).abs() < 1e-6);
        f.set_param("shift", -99.0);
        assert!((f.shift() - -24.0).abs() < 1e-6);
        f.set_param("unknown", 1.0);
        assert!((f.shift() - -24.0).abs() < 1e-6);
    }
}
