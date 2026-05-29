//! Noise gate. Hysteresis-gated linear gain driven by a peak-following
//! envelope. Below the threshold the gain closes smoothly to silence;
//! above it the gate opens.

use super::{AudioEffect, EffectKind};

/// Hysteresis-gated noise gate.
pub struct NoiseGate {
    enabled: bool,
    threshold_db: f32,
    envelope: f32,
    gain: f32,
}

impl NoiseGate {
    #[must_use]
    pub fn new() -> Self {
        Self {
            enabled: false,
            threshold_db: -52.0,
            envelope: 0.0,
            gain: 0.0,
        }
    }
}

impl Default for NoiseGate {
    fn default() -> Self {
        Self::new()
    }
}

impl AudioEffect for NoiseGate {
    fn process(&mut self, buffer: &mut [f32], _sample_rate: u32) {
        let open_amp = 10_f32.powf(self.threshold_db / 20.0);
        let close_amp = open_amp * 0.8; // hysteresis floor
        for sample in buffer.iter_mut() {
            // Peak envelope follower (fast attack, slow release).
            let abs = sample.abs();
            self.envelope = if abs > self.envelope {
                self.envelope * 0.5 + abs * 0.5 // attack
            } else {
                self.envelope * 0.995 + abs * 0.005 // release
            };
            let target = if self.envelope > open_amp {
                1.0
            } else if self.envelope < close_amp {
                0.0
            } else {
                self.gain
            };
            // Smooth gain ramp to avoid clicks.
            self.gain += (target - self.gain) * 0.04;
            *sample *= self.gain;
        }
    }

    fn set_param(&mut self, key: &str, value: f32) {
        if key == "thresh" {
            self.threshold_db = value.clamp(-90.0, 0.0);
        }
    }

    fn enabled(&self) -> bool {
        self.enabled
    }

    fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
        if !enabled {
            self.gain = 1.0; // open when disabled so signal passes through
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
}
