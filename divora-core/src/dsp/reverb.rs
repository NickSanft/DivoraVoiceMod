//! Schroeder-style reverb. Four parallel comb filters feed two series
//! allpass filters; the output is mixed back with the dry signal.
//! Lighter than full Freeverb but produces a recognisable hall.

use super::{AudioEffect, EffectKind};

/// One comb filter. `feedback` controls tail length.
struct Comb {
    buffer: Vec<f32>,
    pos: usize,
    feedback: f32,
}

impl Comb {
    fn new(size: usize) -> Self {
        Self {
            buffer: vec![0.0; size],
            pos: 0,
            feedback: 0.84,
        }
    }

    fn process(&mut self, input: f32) -> f32 {
        let out = self.buffer[self.pos];
        self.buffer[self.pos] = input + out * self.feedback;
        self.pos = (self.pos + 1) % self.buffer.len();
        out
    }
}

/// One allpass filter (Schroeder).
struct Allpass {
    buffer: Vec<f32>,
    pos: usize,
}

impl Allpass {
    fn new(size: usize) -> Self {
        Self {
            buffer: vec![0.0; size],
            pos: 0,
        }
    }

    fn process(&mut self, input: f32) -> f32 {
        let buf_out = self.buffer[self.pos];
        self.buffer[self.pos] = input + buf_out * 0.5;
        self.pos = (self.pos + 1) % self.buffer.len();
        buf_out - input * 0.5
    }
}

pub struct Reverb {
    enabled: bool,
    room_size: f32, // 0..1
    mix: f32,       // 0..1
    combs: [Comb; 4],
    allpasses: [Allpass; 2],
}

impl Reverb {
    #[must_use]
    pub fn new() -> Self {
        // Magic numbers from the Freeverb reference; coprime-ish lengths
        // avoid resonances at any single frequency.
        let mut r = Self {
            enabled: false,
            room_size: 0.4,
            mix: 0.25,
            combs: [
                Comb::new(1116),
                Comb::new(1188),
                Comb::new(1277),
                Comb::new(1356),
            ],
            allpasses: [Allpass::new(556), Allpass::new(441)],
        };
        r.update_feedback();
        r
    }

    fn update_feedback(&mut self) {
        let fb = 0.7 + 0.28 * self.room_size; // 0.7..0.98
        for c in &mut self.combs {
            c.feedback = fb;
        }
    }
}

impl Default for Reverb {
    fn default() -> Self {
        Self::new()
    }
}

impl AudioEffect for Reverb {
    fn process(&mut self, buffer: &mut [f32], _sample_rate: u32) {
        for sample in buffer.iter_mut() {
            let dry = *sample;
            let mut wet = 0.0_f32;
            for c in &mut self.combs {
                wet += c.process(dry);
            }
            wet *= 0.25;
            for ap in &mut self.allpasses {
                wet = ap.process(wet);
            }
            *sample = dry * (1.0 - self.mix) + wet * self.mix;
        }
    }

    fn set_param(&mut self, key: &str, value: f32) {
        match key {
            "size" => {
                self.room_size = (value / 100.0).clamp(0.0, 1.0);
                self.update_feedback();
            }
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
        EffectKind::Reverb
    }
}

#[cfg(test)]
mod tests {
    use super::{AudioEffect, Reverb};

    #[test]
    fn zero_mix_is_passthrough() {
        let mut r = Reverb::new();
        r.set_enabled(true);
        r.set_param("mix", 0.0);
        let mut buf = [0.5_f32; 32];
        r.process(&mut buf, 48000);
        for s in buf {
            assert!((s - 0.5).abs() < 1e-3);
        }
    }

    #[test]
    fn impulse_decays_into_a_tail() {
        let mut r = Reverb::new();
        r.set_enabled(true);
        r.set_param("size", 80.0);
        r.set_param("mix", 50.0);
        let mut buf = vec![0.0_f32; 4096];
        buf[0] = 1.0;
        r.process(&mut buf, 48000);
        // Expect non-zero energy well after the impulse.
        let tail_energy: f32 = buf[2000..4000].iter().map(|s| s.abs()).sum();
        assert!(tail_energy > 0.1);
    }
}
