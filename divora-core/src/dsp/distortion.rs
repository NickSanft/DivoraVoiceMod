//! Soft-clip distortion. `drive` scales the input into the non-linear
//! region of `tanh`; the output is renormalised by the square root of
//! the drive so loudness stays roughly constant.
//!
//! v1.32.0: an optional `warmth` (bias) makes the saturation **asymmetric** —
//! biasing the input before `tanh` clips the two polarities unequally, which
//! produces **even harmonics** ("valve warmth") instead of the symmetric
//! tanh's odd-harmonic "transistor edge". A one-pole DC-block removes the DC
//! offset the bias introduces. `warmth = 0` is the exact pre-v1.32 behaviour
//! (symmetric, no DC-block — so old presets are bit-for-bit unchanged).

use super::{AudioEffect, EffectKind};

/// DC-block corner (~30 Hz one-pole high-pass) — removes the bias's DC offset
/// while leaving the voice band untouched.
const DC_BLOCK_HZ: f32 = 30.0;

pub struct Distortion {
    enabled: bool,
    /// 0..1, mapped to a drive multiplier of 1..11.
    drive: f32,
    /// 0..~0.6 input bias for asymmetric (even-harmonic) saturation. 0 = the
    /// original symmetric tanh, bit-exact (DC-block bypassed too).
    bias: f32,
    /// DC-block state (previous input/output), used only when `bias > 0`.
    dc_x1: f32,
    dc_y1: f32,
}

impl Distortion {
    #[must_use]
    pub fn new() -> Self {
        Self {
            enabled: false,
            drive: 0.35,
            bias: 0.0,
            dc_x1: 0.0,
            dc_y1: 0.0,
        }
    }
}

impl Default for Distortion {
    fn default() -> Self {
        Self::new()
    }
}

impl AudioEffect for Distortion {
    fn process(&mut self, buffer: &mut [f32], sample_rate: u32) {
        let amount = 1.0 + self.drive * 10.0;
        let normaliser = amount.sqrt();
        if self.bias <= 0.0 {
            // Original symmetric path — bit-exact, no DC-block.
            for sample in buffer.iter_mut() {
                *sample = (*sample * amount).tanh() / normaliser;
            }
            return;
        }
        // Asymmetric (even-harmonic) drive + DC-block to strip the bias offset.
        #[allow(clippy::cast_precision_loss)]
        let sr = sample_rate.max(1) as f32;
        let r = (-2.0 * std::f32::consts::PI * DC_BLOCK_HZ / sr).exp();
        for sample in buffer.iter_mut() {
            let x = if sample.is_finite() { *sample } else { 0.0 };
            let driven = (x * amount + self.bias).tanh() / normaliser;
            // One-pole DC-block: y = x - x1 + R*y1.
            let y = driven - self.dc_x1 + r * self.dc_y1;
            self.dc_x1 = driven;
            self.dc_y1 = y;
            *sample = if y.is_finite() { y } else { 0.0 };
        }
    }

    fn set_param(&mut self, key: &str, value: f32) {
        match key {
            "drive" => self.drive = (value / 100.0).clamp(0.0, 1.0),
            // 0..100 → 0..0.6 input bias for the asymmetric (warm) saturation.
            "warmth" => self.bias = (value / 100.0).clamp(0.0, 1.0) * 0.6,
            _ => {}
        }
    }

    fn enabled(&self) -> bool {
        self.enabled
    }

    fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
        if !enabled {
            self.dc_x1 = 0.0;
            self.dc_y1 = 0.0;
        }
    }

    fn kind(&self) -> EffectKind {
        EffectKind::Distortion
    }
}

#[cfg(test)]
mod tests {
    use super::{AudioEffect, Distortion};

    #[test]
    fn zero_drive_is_close_to_passthrough() {
        let mut d = Distortion::new();
        d.set_enabled(true);
        d.set_param("drive", 0.0);
        let mut buf = [0.5_f32; 64];
        d.process(&mut buf, 48000);
        for s in buf {
            assert!((s - 0.5).abs() < 0.1);
        }
    }

    #[test]
    fn high_drive_clips_within_unity() {
        let mut d = Distortion::new();
        d.set_enabled(true);
        d.set_param("drive", 100.0);
        let mut buf = [1.0_f32; 64];
        d.process(&mut buf, 48000);
        for s in buf {
            assert!(s.abs() <= 1.0);
        }
    }

    #[test]
    fn negative_input_yields_negative_output() {
        let mut d = Distortion::new();
        d.set_enabled(true);
        d.set_param("drive", 50.0);
        let mut buf = [-0.3_f32; 8];
        d.process(&mut buf, 48000);
        for s in buf {
            assert!(s < 0.0);
        }
    }

    /// `warmth = 0` is the exact pre-v1.32 symmetric tanh — old presets must
    /// sound byte-for-byte identical.
    #[test]
    #[allow(clippy::float_cmp)]
    fn warmth_zero_is_bit_exact_symmetric() {
        let mut d = Distortion::new();
        d.set_enabled(true);
        d.set_param("drive", 60.0);
        d.set_param("warmth", 0.0);
        let mut got = [0.3_f32, -0.4, 0.7, -0.2];
        d.process(&mut got, 48_000);
        let amount = 1.0_f32 + 0.6 * 10.0;
        let norm = amount.sqrt();
        let want = [0.3_f32, -0.4, 0.7, -0.2].map(|x: f32| (x * amount).tanh() / norm);
        assert_eq!(got, want, "warmth 0 must be the plain symmetric tanh");
    }

    /// The asymmetric (warm) path stays finite + bounded and the DC-block nulls
    /// the offset the bias introduces.
    #[test]
    fn warmth_is_bounded_and_dc_free() {
        let mut d = Distortion::new();
        d.set_enabled(true);
        d.set_param("drive", 50.0);
        d.set_param("warmth", 80.0);
        #[allow(clippy::cast_precision_loss)]
        let mut buf: Vec<f32> = (0..4800)
            .map(|i| (2.0 * std::f32::consts::PI * 200.0 * (i as f32 / 48_000.0)).sin() * 0.5)
            .collect();
        d.process(&mut buf, 48_000);
        let settled = &buf[2400..];
        #[allow(clippy::cast_precision_loss)]
        let mean = settled.iter().sum::<f32>() / settled.len() as f32;
        assert!(
            mean.abs() < 0.02,
            "DC-block should null the bias offset (mean {mean})"
        );
        for &s in settled {
            assert!(s.is_finite() && s.abs() <= 1.5, "bounded + finite, got {s}");
        }
    }
}
