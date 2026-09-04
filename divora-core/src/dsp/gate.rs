//! Noise gate / downward expander (v1.42.0). A soft downward expander driven
//! by a peak-following envelope: below the threshold the gain is reduced in
//! proportion to how far below (by `ratio`), down to a `range` floor, with
//! `attack`/`release` timing and a `hold` that keeps it open briefly after the
//! signal dips — so it doesn't chatter on quiet/breathy input the way the old
//! hard hysteresis gate did. At a high ratio + range it still acts as a hard
//! gate; at a low ratio + small range it's a gentle expander. All timing is
//! sample-rate-aware.

use super::envelope::{coeff_ms, coeff_secs, EnvelopeFollower};
use super::{AudioEffect, EffectKind};

/// Soft downward expander (a superset of the old hard gate).
pub struct NoiseGate {
    enabled: bool,
    threshold_db: f32,
    /// Downward-expansion ratio (1 = off, higher = steeper; large = gate-like).
    ratio: f32,
    attack_ms: f32,
    release_ms: f32,
    /// Maximum attenuation below the threshold, in dB (the floor).
    range_db: f32,
    hold_ms: f32,
    envelope: EnvelopeFollower,
    gain: EnvelopeFollower,
    /// Remaining "hold open" countdown in samples.
    hold_left: u32,
}

impl NoiseGate {
    #[must_use]
    pub fn new() -> Self {
        Self {
            enabled: false,
            threshold_db: -52.0,
            ratio: 4.0,
            attack_ms: 5.0,
            release_ms: 120.0,
            range_db: 60.0,
            hold_ms: 20.0,
            envelope: EnvelopeFollower::new(0.0),
            gain: EnvelopeFollower::new(1.0),
            hold_left: 0,
        }
    }
}

impl Default for NoiseGate {
    fn default() -> Self {
        Self::new()
    }
}

impl AudioEffect for NoiseGate {
    fn process(&mut self, buffer: &mut [f32], sample_rate: u32) {
        #[allow(clippy::cast_precision_loss)]
        let sr = sample_rate.max(1) as f32;
        let open_amp = 10_f32.powf(self.threshold_db / 20.0);
        // Sample-rate-aware one-pole coefficients (attack/release time constants).
        let env_atk = coeff_secs(0.001, sr); // 1 ms envelope attack
        let env_rel = coeff_secs(0.050, sr); // 50 ms envelope release
        let gain_atk = coeff_ms(self.attack_ms, sr);
        let gain_rel = coeff_ms(self.release_ms, sr);
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let hold_samples = (self.hold_ms / 1000.0 * sr) as u32;
        for sample in buffer.iter_mut() {
            let x = if sample.is_finite() { *sample } else { 0.0 };
            let abs = x.abs();
            // Peak envelope follower.
            let envelope = self.envelope.attack_on_rise(abs, env_atk, env_rel);

            // Hold: refresh while above threshold; count down once below, so a
            // brief dip doesn't slam the gate shut (anti-chatter).
            if envelope >= open_amp {
                self.hold_left = hold_samples;
            } else if self.hold_left > 0 {
                self.hold_left -= 1;
            }

            // Target gain: fully open above threshold / during hold, else a
            // downward-expansion reduction proportional to dB below threshold,
            // clamped to the `range` floor.
            let target = if envelope >= open_amp || self.hold_left > 0 {
                1.0
            } else {
                let env_db = 20.0 * envelope.max(1e-9).log10();
                let below = (self.threshold_db - env_db).max(0.0);
                let reduction = (below * (self.ratio - 1.0)).min(self.range_db);
                10_f32.powf(-reduction / 20.0)
            };

            // Smooth toward the target (fast when opening, slow when closing).
            let gain = self.gain.attack_on_rise(target, gain_atk, gain_rel);
            *sample = x * gain;
        }
    }

    fn set_param(&mut self, key: &str, value: f32) {
        match key {
            "thresh" => self.threshold_db = value.clamp(-90.0, 0.0),
            "ratio" => self.ratio = value.clamp(1.0, 20.0),
            "attack" => self.attack_ms = value.clamp(0.1, 100.0),
            "release" => self.release_ms = value.clamp(5.0, 1000.0),
            "range" => self.range_db = value.clamp(0.0, 90.0),
            "hold" => self.hold_ms = value.clamp(0.0, 500.0),
            _ => {}
        }
    }

    fn enabled(&self) -> bool {
        self.enabled
    }

    fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
        if !enabled {
            self.gain.reset(1.0); // open when disabled so signal passes through
        }
    }

    fn kind(&self) -> EffectKind {
        EffectKind::Gate
    }
}

#[cfg(test)]
mod tests {
    use super::{AudioEffect, NoiseGate};

    #[test]
    fn gate_closes_on_silent_input() {
        let mut g = NoiseGate::new();
        g.set_enabled(true);
        g.set_param("thresh", -40.0);
        let mut buf = [0.0001_f32; 512];
        for _ in 0..16 {
            // Run multiple buffers so the gain has time to settle to 0.
            buf.copy_from_slice(&[0.0001_f32; 512]);
            g.process(&mut buf, 48000);
        }
        #[allow(clippy::cast_precision_loss)]
        let avg = buf.iter().map(|s| s.abs()).sum::<f32>() / buf.len() as f32;
        assert!(avg < 5e-5);
    }

    #[test]
    fn gate_passes_loud_input() {
        let mut g = NoiseGate::new();
        g.set_enabled(true);
        g.set_param("thresh", -40.0);
        let mut buf = [0.5_f32; 512];
        for _ in 0..16 {
            buf.copy_from_slice(&[0.5_f32; 512]);
            g.process(&mut buf, 48000);
        }
        #[allow(clippy::cast_precision_loss)]
        let avg = buf.iter().map(|s| s.abs()).sum::<f32>() / buf.len() as f32;
        assert!(avg > 0.4);
    }

    #[test]
    fn disabled_gate_does_not_attenuate() {
        let mut g = NoiseGate::new();
        // Stays disabled; ensure passthrough at unity.
        let mut buf = [0.3_f32; 64];
        g.set_enabled(false);
        // Even though enabled() is false the chain skips process(), but
        // calling process directly shouldn't blow up either.
        g.process(&mut buf, 48000);
        // gain was set to 1.0 on disable, so the buffer is unchanged.
        assert!((buf[0] - 0.3).abs() < 1e-3);
    }

    // v1.42.0: with a gentle ratio + small range, a below-threshold signal is
    // REDUCED but not slammed to silence — the expander behaviour a hard gate
    // can't do.
    #[test]
    fn gentle_expansion_reduces_but_does_not_close() {
        let mut g = NoiseGate::new();
        g.set_enabled(true);
        g.set_param("thresh", -30.0);
        g.set_param("ratio", 2.0);
        g.set_param("range", 12.0); // cap attenuation at 12 dB
        g.set_param("hold", 0.0);
        // -40 dBFS, i.e. 10 dB below the -30 threshold.
        let mut buf = [0.01_f32; 512];
        for _ in 0..40 {
            buf.copy_from_slice(&[0.01_f32; 512]);
            g.process(&mut buf, 48_000);
        }
        #[allow(clippy::cast_precision_loss)]
        let avg = buf.iter().map(|s| s.abs()).sum::<f32>() / buf.len() as f32;
        // 10 dB below × (ratio-1)=1 → ~10 dB reduction (10^-0.5 ≈ 0.316×):
        // clearly reduced from 0.01, but far from silent.
        assert!(avg < 0.008 && avg > 0.002, "gentle expansion, got {avg}");
    }
}
