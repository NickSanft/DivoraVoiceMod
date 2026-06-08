//! Dynamics compressor (v1.8.0). Feed-forward, log-domain, soft-knee,
//! **zero look-ahead** — so it adds no algorithmic latency. Evens out a
//! voice's level for a steadier, broadcast-sounding send.
//!
//! Per sample: measure the input level in dB, run it through a soft-knee
//! gain computer (threshold / ratio / knee), smooth the resulting gain
//! reduction with separate attack / release time constants, then apply
//! that gain plus a fixed makeup. The detector envelope lives in the
//! gain-reduction (dB) domain, which is click-free and matches how
//! hardware compressors behave.

use super::{AudioEffect, EffectKind};

/// Soft-knee width in dB (fixed). Wide enough to be musical, narrow
/// enough that the ratio still bites above the threshold.
const KNEE_DB: f32 = 6.0;

pub struct Compressor {
    enabled: bool,
    threshold_db: f32,
    ratio: f32,
    attack_ms: f32,
    release_ms: f32,
    makeup_db: f32,
    /// Current gain reduction in dB (≤ 0), smoothed across samples.
    env_db: f32,
}

impl Compressor {
    #[must_use]
    pub fn new() -> Self {
        Self {
            enabled: false,
            threshold_db: -24.0,
            ratio: 3.0,
            attack_ms: 10.0,
            release_ms: 120.0,
            makeup_db: 0.0,
            env_db: 0.0,
        }
    }

    /// Desired *output* level (dB) for a given input level (dB), using the
    /// standard soft-knee static curve. Returns the gain change in dB
    /// (output − input), always ≤ 0.
    fn gain_db_for(&self, level_db: f32) -> f32 {
        let over = level_db - self.threshold_db;
        let half_knee = KNEE_DB * 0.5;
        let out_db = if 2.0 * over < -KNEE_DB {
            // Below the knee — no compression.
            level_db
        } else if 2.0 * over.abs() <= KNEE_DB {
            // Inside the knee — quadratic interpolation.
            let x = over + half_knee;
            level_db + (1.0 / self.ratio - 1.0) * (x * x) / (2.0 * KNEE_DB)
        } else {
            // Above the knee — full ratio.
            self.threshold_db + over / self.ratio
        };
        out_db - level_db
    }
}

impl Default for Compressor {
    fn default() -> Self {
        Self::new()
    }
}

impl AudioEffect for Compressor {
    fn process(&mut self, buffer: &mut [f32], sample_rate: u32) {
        #[allow(clippy::cast_precision_loss)]
        let sr = sample_rate.max(1) as f32;
        // One-pole smoothing coefficients (computed per buffer, not per
        // sample — no allocation, cheap).
        let smoothing = |ms: f32| (-1.0 / ((ms / 1000.0).max(1e-4) * sr)).exp();
        let attack_coeff = smoothing(self.attack_ms);
        let release_coeff = smoothing(self.release_ms);
        let makeup_lin = 10_f32.powf(self.makeup_db / 20.0);

        for sample in buffer.iter_mut() {
            let x = *sample;
            // Sanitize: non-finite input would poison the envelope.
            let level = if x.is_finite() { x.abs() } else { 0.0 };
            let level_db = 20.0 * level.max(1e-9).log10();
            let target_db = self.gain_db_for(level_db).min(0.0);

            // Attack when more reduction is needed (gain dropping), release
            // when backing off. Both move env_db toward target_db.
            let coeff = if target_db < self.env_db {
                attack_coeff
            } else {
                release_coeff
            };
            self.env_db = coeff * self.env_db + (1.0 - coeff) * target_db;

            let gain = 10_f32.powf(self.env_db / 20.0) * makeup_lin;
            let out = x * gain;
            *sample = if out.is_finite() { out } else { 0.0 };
        }
    }

    fn set_param(&mut self, key: &str, value: f32) {
        match key {
            "thresh" => self.threshold_db = value.clamp(-80.0, 0.0),
            "ratio" => self.ratio = value.clamp(1.0, 20.0),
            "attack" => self.attack_ms = value.clamp(0.1, 200.0),
            "release" => self.release_ms = value.clamp(5.0, 1000.0),
            "makeup" => self.makeup_db = value.clamp(0.0, 24.0),
            _ => {}
        }
    }

    fn enabled(&self) -> bool {
        self.enabled
    }

    fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
        if !enabled {
            self.env_db = 0.0; // reset so a re-enable doesn't jump
        }
    }

    fn kind(&self) -> EffectKind {
        EffectKind::Compressor
    }
    // latency_samples: default 0 — feed-forward, no look-ahead.
}

#[cfg(test)]
mod tests {
    use super::{AudioEffect, Compressor};

    fn rms(buf: &[f32]) -> f32 {
        #[allow(clippy::cast_precision_loss)]
        let n = buf.len() as f32;
        (buf.iter().map(|s| s * s).sum::<f32>() / n).sqrt()
    }

    /// A quiet signal well below the threshold should pass essentially
    /// unchanged (no reduction, unity makeup by default).
    #[test]
    fn below_threshold_is_near_passthrough() {
        let mut c = Compressor::new();
        c.set_enabled(true);
        c.set_param("thresh", -24.0);
        // ~ -40 dBFS sine, comfortably under the threshold.
        let mut buf = vec![0.0_f32; 4800];
        for (i, s) in buf.iter_mut().enumerate() {
            #[allow(clippy::cast_precision_loss)]
            let t = i as f32 / 48_000.0;
            *s = (2.0 * std::f32::consts::PI * 200.0 * t).sin() * 0.01;
        }
        let before = rms(&buf);
        c.process(&mut buf, 48_000);
        let after = rms(&buf);
        assert!(
            (after - before).abs() / before < 0.05,
            "expected near-passthrough below threshold (before {before}, after {after})"
        );
    }

    /// A loud signal above the threshold should be attenuated.
    #[test]
    fn above_threshold_is_compressed() {
        let mut c = Compressor::new();
        c.set_enabled(true);
        c.set_param("thresh", -24.0);
        c.set_param("ratio", 8.0);
        c.set_param("attack", 1.0);
        // ~ -6 dBFS sine — well over the threshold.
        let mut buf = vec![0.0_f32; 48_000];
        for (i, s) in buf.iter_mut().enumerate() {
            #[allow(clippy::cast_precision_loss)]
            let t = i as f32 / 48_000.0;
            *s = (2.0 * std::f32::consts::PI * 200.0 * t).sin() * 0.5;
        }
        let before = rms(&buf);
        c.process(&mut buf, 48_000);
        // Measure the settled second half (past attack/onset).
        let after = rms(&buf[24_000..]);
        assert!(
            after < before * 0.9,
            "expected compression above threshold (before {before}, after {after})"
        );
    }

    /// Ratio 1:1 with no makeup is a passthrough (no reduction).
    #[test]
    fn unity_ratio_does_not_change_level() {
        let mut c = Compressor::new();
        c.set_enabled(true);
        c.set_param("thresh", -40.0);
        c.set_param("ratio", 1.0);
        let mut buf = vec![0.4_f32; 1024];
        c.process(&mut buf, 48_000);
        for s in &buf {
            assert!(
                (s - 0.4).abs() < 1e-3,
                "unity ratio should not change level"
            );
        }
    }

    /// Non-finite input must not produce non-finite output.
    #[test]
    fn stays_finite_on_extreme_input() {
        let mut c = Compressor::new();
        c.set_enabled(true);
        let mut buf = [f32::NAN, f32::INFINITY, -f32::INFINITY, 1e30, -1e30, 0.0];
        c.process(&mut buf, 48_000);
        for s in buf {
            assert!(s.is_finite(), "output must stay finite, got {s}");
        }
    }

    #[test]
    fn adds_no_latency() {
        let c = Compressor::new();
        assert_eq!(c.latency_samples(48_000), 0);
    }

    /// A sample-rate change is absorbed (coefficients recomputed); the
    /// effect keeps working without panicking.
    #[test]
    fn handles_sample_rate_change() {
        let mut c = Compressor::new();
        c.set_enabled(true);
        c.set_param("thresh", -24.0);
        let mut buf = vec![0.5_f32; 512];
        c.process(&mut buf, 48_000);
        let mut buf2 = vec![0.5_f32; 512];
        c.process(&mut buf2, 44_100);
        for s in buf2 {
            assert!(s.is_finite());
        }
    }
}
