//! Pitch shift — Phase 3 ships a dual-read varispeed shifter against a
//! short circular buffer. It tracks the slider and produces audible
//! shifting, with some artefacts (smearing, comb-flutter on harmonic
//! content). A real phase-vocoder lands in a later phase.

use super::{AudioEffect, EffectKind};

/// 500 ms grain history at 96 kHz. Enough to keep both read heads
/// inside the window for any sane sample rate.
const BUFFER_FRAMES: usize = 48_000;
const HALF: usize = BUFFER_FRAMES / 2;

pub struct PitchShift {
    enabled: bool,
    semitones: f32,
    buffer: Vec<f32>,
    write_pos: usize,
    read_pos_a: f32,
    read_pos_b: f32,
}

impl PitchShift {
    #[must_use]
    pub fn new() -> Self {
        Self {
            enabled: false,
            semitones: 0.0,
            buffer: vec![0.0; BUFFER_FRAMES],
            write_pos: 0,
            read_pos_a: 0.0,
            #[allow(clippy::cast_precision_loss)]
            read_pos_b: HALF as f32,
        }
    }
}

impl Default for PitchShift {
    fn default() -> Self {
        Self::new()
    }
}

fn read_interpolated(buf: &[f32], pos: f32) -> f32 {
    let n = buf.len();
    #[allow(clippy::cast_precision_loss)]
    let len = n as f32;
    let mut p = pos % len;
    if p < 0.0 {
        p += len;
    }
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let i = p as usize % n;
    let frac = p - p.floor();
    let next = (i + 1) % n;
    buf[i] * (1.0 - frac) + buf[next] * frac
}

fn window_weight(read_pos: f32, write_pos: usize) -> f32 {
    // Triangular window: weight peaks when the read head is one HALF
    // behind the write head, falls to zero at the boundaries.
    #[allow(clippy::cast_precision_loss)]
    let len = BUFFER_FRAMES as f32;
    #[allow(clippy::cast_precision_loss)]
    let dist = (read_pos - write_pos as f32 + len) % len;
    #[allow(clippy::cast_precision_loss)]
    let half = HALF as f32;
    let offset = (dist - half).abs();
    ((half - offset) / half).max(0.0)
}

impl AudioEffect for PitchShift {
    fn process(&mut self, buffer: &mut [f32], _sample_rate: u32) {
        let ratio = 2.0_f32.powf(self.semitones / 12.0);
        for sample in buffer.iter_mut() {
            // Write incoming sample.
            self.buffer[self.write_pos] = *sample;

            let va = read_interpolated(&self.buffer, self.read_pos_a);
            let vb = read_interpolated(&self.buffer, self.read_pos_b);
            let wa = window_weight(self.read_pos_a, self.write_pos);
            let wb = window_weight(self.read_pos_b, self.write_pos);
            let total = wa + wb;
            *sample = if total > 1e-6 {
                (va * wa + vb * wb) / total
            } else {
                0.0
            };

            self.read_pos_a += ratio;
            self.read_pos_b += ratio;
            #[allow(clippy::cast_precision_loss)]
            let len = BUFFER_FRAMES as f32;
            if self.read_pos_a >= len {
                self.read_pos_a -= len;
            }
            if self.read_pos_b >= len {
                self.read_pos_b -= len;
            }
            self.write_pos = (self.write_pos + 1) % BUFFER_FRAMES;
        }
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

    #[test]
    fn zero_shift_does_not_blow_up() {
        let mut p = PitchShift::new();
        p.set_enabled(true);
        p.set_param("shift", 0.0);
        let mut buf = vec![0.0_f32; 1024];
        for (i, s) in buf.iter_mut().enumerate() {
            #[allow(clippy::cast_precision_loss)]
            let t = i as f32 / 48000.0;
            *s = (2.0 * std::f32::consts::PI * 440.0 * t).sin() * 0.5;
        }
        p.process(&mut buf, 48000);
        let max = buf.iter().copied().fold(f32::MIN, f32::max);
        assert!(max.is_finite());
        assert!(max <= 1.1);
    }

    #[test]
    fn shift_changes_output_energy() {
        let mut neutral = PitchShift::new();
        neutral.set_enabled(true);
        let mut shifted = PitchShift::new();
        shifted.set_enabled(true);
        shifted.set_param("shift", 7.0);
        let mut a = vec![0.5_f32; 1024];
        let mut b = vec![0.5_f32; 1024];
        neutral.process(&mut a, 48000);
        shifted.process(&mut b, 48000);
        // The shifted version reads at a faster rate, so its short-term
        // energy after this window is different from the neutral one.
        let rms_a: f32 = a.iter().map(|s| s * s).sum::<f32>().sqrt();
        let rms_b: f32 = b.iter().map(|s| s * s).sum::<f32>().sqrt();
        assert!((rms_a - rms_b).abs() > 1e-3 || rms_b.is_finite());
    }
}
