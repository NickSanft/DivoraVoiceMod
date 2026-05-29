//! Robot / vocoder-lite. Ring-modulates the input with a sine carrier
//! and crossfades with the dry signal. Simple but unmistakable.

use std::f32::consts::TAU;

use super::{AudioEffect, EffectKind};

pub struct Robot {
    enabled: bool,
    carrier_hz: f32,
    mix: f32,
    phase: f32,
}

impl Robot {
    #[must_use]
    pub fn new() -> Self {
        Self {
            enabled: false,
            carrier_hz: 120.0,
            mix: 0.7,
            phase: 0.0,
        }
    }
}

impl Default for Robot {
    fn default() -> Self {
        Self::new()
    }
}

impl AudioEffect for Robot {
    fn process(&mut self, buffer: &mut [f32], sample_rate: u32) {
        #[allow(clippy::cast_precision_loss)]
        let sr = sample_rate as f32;
        let phase_inc = TAU * self.carrier_hz / sr;
        for sample in buffer.iter_mut() {
            let carrier = self.phase.sin();
            let robotic = *sample * carrier;
            *sample = *sample * (1.0 - self.mix) + robotic * self.mix;
            self.phase += phase_inc;
            if self.phase > TAU {
                self.phase -= TAU;
            }
        }
    }

    fn set_param(&mut self, key: &str, value: f32) {
        match key {
            "freq" => self.carrier_hz = value.clamp(20.0, 4000.0),
            "mix" => self.mix = (value / 100.0).clamp(0.0, 1.0),
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
        EffectKind::Robot
    }
}

#[cfg(test)]
mod tests {
    use super::{AudioEffect, Robot};

    #[test]
    fn zero_mix_is_passthrough() {
        let mut r = Robot::new();
        r.set_enabled(true);
        r.set_param("mix", 0.0);
        let mut buf = [0.3_f32; 64];
        r.process(&mut buf, 48000);
        for s in buf {
            assert!((s - 0.3).abs() < 1e-6);
        }
    }

    #[test]
    fn full_mix_carries_carrier() {
        let mut r = Robot::new();
        r.set_enabled(true);
        r.set_param("freq", 100.0);
        r.set_param("mix", 100.0);
        let mut buf = vec![0.5_f32; 480]; // 10 ms at 48 kHz
        r.process(&mut buf, 48000);
        // Output should oscillate (range of values across the buffer).
        let max = buf.iter().copied().fold(f32::MIN, f32::max);
        let min = buf.iter().copied().fold(f32::MAX, f32::min);
        assert!(max - min > 0.4);
    }
}
