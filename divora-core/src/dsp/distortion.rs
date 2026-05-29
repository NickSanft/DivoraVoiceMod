//! Soft-clip distortion. `drive` scales the input into the non-linear
//! region of `tanh`; the output is renormalised by the square root of
//! the drive so loudness stays roughly constant.

use super::{AudioEffect, EffectKind};

pub struct Distortion {
    enabled: bool,
    /// 0..1, mapped to a drive multiplier of 1..11.
    drive: f32,
}

impl Distortion {
    #[must_use]
    pub fn new() -> Self {
        Self {
            enabled: false,
            drive: 0.35,
        }
    }
}

impl Default for Distortion {
    fn default() -> Self {
        Self::new()
    }
}

impl AudioEffect for Distortion {
    fn process(&mut self, buffer: &mut [f32], _sample_rate: u32) {
        let amount = 1.0 + self.drive * 10.0;
        let normaliser = amount.sqrt();
        for sample in buffer.iter_mut() {
            *sample = (*sample * amount).tanh() / normaliser;
        }
    }

    fn set_param(&mut self, key: &str, value: f32) {
        if key == "drive" {
            self.drive = (value / 100.0).clamp(0.0, 1.0);
        }
    }

    fn enabled(&self) -> bool {
        self.enabled
    }

    fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
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
}
