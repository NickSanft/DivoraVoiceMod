//! Pitch shift — Phase 3 passthrough stub.
//!
//! The previous Phase 3 implementation was a dual-read varispeed
//! shifter against a 500 ms circular buffer. At any non-unity ratio the
//! two read pointers — separated by `HALF` (≈ 500 ms) in the buffer —
//! drifted through the crossfade together, sampling audio that was
//! 500 ms apart in time. With both weights at ~0.5 during the
//! transition, listeners heard their own voice **doubled**, with the
//! second copy delayed by ~500 ms. The Hollow King default
//! (`shift = -5`) made the bug audible on every use.
//!
//! Until a proper algorithm ships (phase-vocoder for tempo-preserving
//! shift, or a real granular SOLA with short overlapping grains), we
//! pass audio through unchanged. The slider still moves and feeds the
//! audio thread, so the chain plumbing is exercised end-to-end; it
//! just doesn't apply any DSP yet.

use super::{AudioEffect, EffectKind};

pub struct PitchShift {
    enabled: bool,
    /// Last-set semitone target. Recorded so the future algorithm can
    /// pick up where the UI left off, and so debug builds can confirm
    /// the parameter is reaching the audio thread.
    semitones: f32,
}

impl PitchShift {
    #[must_use]
    pub fn new() -> Self {
        Self {
            enabled: false,
            semitones: 0.0,
        }
    }

    /// Inspect the currently-stored semitone target. Used by tests; not
    /// part of the public effect surface.
    #[doc(hidden)]
    #[must_use]
    pub fn semitones(&self) -> f32 {
        self.semitones
    }
}

impl Default for PitchShift {
    fn default() -> Self {
        Self::new()
    }
}

impl AudioEffect for PitchShift {
    fn process(&mut self, _buffer: &mut [f32], _sample_rate: u32) {
        // Passthrough; see module-level docs for the reasoning.
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

    fn sine_buffer(len: usize, freq_hz: f32, sample_rate: u32) -> Vec<f32> {
        (0..len)
            .map(|i| {
                #[allow(clippy::cast_precision_loss)]
                let t = i as f32 / sample_rate as f32;
                (2.0 * std::f32::consts::PI * freq_hz * t).sin() * 0.5
            })
            .collect()
    }

    fn assert_passthrough(input: &[f32], output: &[f32]) {
        assert_eq!(input.len(), output.len());
        for (i, (a, b)) in output.iter().zip(input.iter()).enumerate() {
            assert!(
                (a - b).abs() < 1e-6,
                "passthrough mismatch at index {i}: expected {b}, got {a}",
            );
        }
    }

    #[test]
    fn passthrough_at_zero_shift() {
        let input = sine_buffer(1024, 440.0, 48_000);
        let mut buf = input.clone();
        let mut p = PitchShift::new();
        p.set_enabled(true);
        p.set_param("shift", 0.0);
        p.process(&mut buf, 48_000);
        assert_passthrough(&input, &buf);
    }

    #[test]
    fn passthrough_at_nonzero_shift_too() {
        // The Hollow King preset enables pitch with shift = -5; this
        // was the configuration that produced audible "twice yourself"
        // doubling in the previous implementation. The fix is verified
        // here as bit-identical passthrough.
        let input = sine_buffer(1024, 440.0, 48_000);
        let mut buf = input.clone();
        let mut p = PitchShift::new();
        p.set_enabled(true);
        p.set_param("shift", -5.0);
        p.process(&mut buf, 48_000);
        assert_passthrough(&input, &buf);
    }

    #[test]
    fn passthrough_across_a_sweep_of_semitones() {
        // Iterate across the full ±12 st design range and a few
        // out-of-range values; every setting should remain a clean
        // identity until the real algorithm ships.
        let input = sine_buffer(512, 220.0, 48_000);
        for shift in [-24.0, -12.0, -7.0, -1.0, 0.0, 1.0, 7.0, 12.0, 24.0] {
            let mut buf = input.clone();
            let mut p = PitchShift::new();
            p.set_enabled(true);
            p.set_param("shift", shift);
            p.process(&mut buf, 48_000);
            assert_passthrough(&input, &buf);
        }
    }

    #[test]
    fn set_param_reaches_internal_state_and_clamps_to_range() {
        // Even though no DSP runs, the slider must drive the param so
        // the chain plumbing is exercised end-to-end. Out-of-range
        // values are clamped to the supported window.
        let mut p = PitchShift::new();
        p.set_param("shift", 7.0);
        assert!((p.semitones() - 7.0).abs() < 1e-6);
        p.set_param("shift", 99.0);
        assert!((p.semitones() - 24.0).abs() < 1e-6);
        p.set_param("shift", -99.0);
        assert!((p.semitones() - -24.0).abs() < 1e-6);
        p.set_param("unknown", 1.0);
        // Unknown keys are silently ignored; value unchanged.
        assert!((p.semitones() - -24.0).abs() < 1e-6);
    }

    #[test]
    fn dc_signal_unchanged() {
        // The most damning version of the old bug was that a constant
        // input would still produce delayed-overlay artefacts because
        // the two read heads stepped at different rates through the
        // buffer. A constant input must come out exactly the same.
        let input = vec![0.42_f32; 8192];
        let mut buf = input.clone();
        let mut p = PitchShift::new();
        p.set_enabled(true);
        p.set_param("shift", -5.0);
        p.process(&mut buf, 48_000);
        assert_passthrough(&input, &buf);
    }
}
