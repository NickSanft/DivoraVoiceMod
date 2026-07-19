//! Schroeder-style reverb. Four parallel comb filters feed two series
//! allpass filters; the output is mixed back with the dry signal.
//! Lighter than full Freeverb but produces a recognisable hall.

use super::{AudioEffect, EffectKind};

/// Comb/allpass buffer sizes (samples) at the 48 kHz reference rate. The
/// classic Freeverb magic numbers; sized in samples, so at another device
/// rate they are scaled to keep the same TIME (see `Reverb::retune`). 48 kHz
/// is the reference (the rate presets were authored at), so 48 kHz is
/// unchanged and only other rates are corrected. (v1.42.0)
const COMB_SIZES: [usize; 4] = [1116, 1188, 1277, 1356];
const ALLPASS_SIZES: [usize; 2] = [556, 441];
const REF_RATE: f32 = 48_000.0;

/// One comb filter. `feedback` controls tail length; `damp` low-passes the
/// feedback loop so the tail loses highs as it decays (a real room, not a
/// ringing spring).
struct Comb {
    buffer: Vec<f32>,
    pos: usize,
    feedback: f32,
    damp: f32,
    filt: f32,
}

impl Comb {
    fn new(size: usize) -> Self {
        Self {
            buffer: vec![0.0; size.max(1)],
            pos: 0,
            feedback: 0.84,
            damp: 0.2,
            filt: 0.0,
        }
    }

    fn process(&mut self, input: f32) -> f32 {
        let out = self.buffer[self.pos];
        // v1.42.0: one-pole damping low-pass inside the feedback loop — the
        // tail darkens as it decays, like a real hall (Freeverb "damp").
        self.filt = out * (1.0 - self.damp) + self.filt * self.damp;
        self.buffer[self.pos] = input + self.filt * self.feedback;
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
    damp: f32,      // 0..1 — feedback-loop damping (tail darkness)
    /// Rate the comb/allpass buffers are currently sized for. Rebuilt on a
    /// change so the room TIME is the same at any device rate. (v1.42.0)
    sample_rate: u32,
    combs: [Comb; 4],
    allpasses: [Allpass; 2],
}

impl Reverb {
    #[must_use]
    pub fn new() -> Self {
        let mut r = Self {
            enabled: false,
            room_size: 0.4,
            mix: 0.25,
            damp: 0.2,
            sample_rate: 48_000, // == REF_RATE; kept as an integer literal to avoid a float cast
            combs: COMB_SIZES.map(Comb::new),
            allpasses: ALLPASS_SIZES.map(Allpass::new),
        };
        r.update_feedback();
        r
    }

    fn update_feedback(&mut self) {
        let fb = 0.7 + 0.28 * self.room_size; // 0.7..0.98
        for c in &mut self.combs {
            c.feedback = fb;
            c.damp = self.damp;
        }
    }

    /// v1.42.0: rebuild the comb/allpass buffers scaled to `sr` so the reverb
    /// TIME (and thus the room character presets were tuned for) stays the same
    /// at 48 kHz, 96 kHz, etc. 48 kHz is the reference, so it is a no-op there.
    fn retune(&mut self, sr: u32) {
        #[allow(
            clippy::cast_precision_loss,
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss
        )]
        let scale = |base: usize| ((base as f32 * sr as f32 / REF_RATE).round() as usize).max(1);
        self.combs = COMB_SIZES.map(|s| Comb::new(scale(s)));
        self.allpasses = ALLPASS_SIZES.map(|s| Allpass::new(scale(s)));
        self.sample_rate = sr;
        self.update_feedback();
    }
}

impl Default for Reverb {
    fn default() -> Self {
        Self::new()
    }
}

impl AudioEffect for Reverb {
    fn process(&mut self, buffer: &mut [f32], sample_rate: u32) {
        // v1.42.0: re-scale the comb/allpass buffers if the device rate changed
        // (rare — only on a session rebuild), keeping the room time constant.
        if sample_rate != 0 && sample_rate != self.sample_rate {
            self.retune(sample_rate);
        }
        for sample in buffer.iter_mut() {
            // Guard the input so a stray NaN/Inf can't enter (and latch into)
            // the recursive comb feedback loops (feedback runs up to 0.98).
            let dry = if sample.is_finite() { *sample } else { 0.0 };
            let mut wet = 0.0_f32;
            for c in &mut self.combs {
                wet += c.process(dry);
            }
            wet *= 0.25;
            for ap in &mut self.allpasses {
                wet = ap.process(wet);
            }
            let out = dry * (1.0 - self.mix) + wet * self.mix;
            *sample = if out.is_finite() { out } else { dry };
        }
    }

    fn set_param(&mut self, key: &str, value: f32) {
        match key {
            "size" => {
                self.room_size = (value / 100.0).clamp(0.0, 1.0);
                self.update_feedback();
            }
            "mix" => self.mix = (value / 100.0).clamp(0.0, 1.0),
            // v1.42.0: tail darkness (feedback-loop damping). 0 = bright/ringy
            // (the old behaviour), higher = a darker, more natural decay.
            "damp" => {
                self.damp = (value / 100.0).clamp(0.0, 1.0);
                self.update_feedback();
            }
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

    #[test]
    fn survives_nan_without_latching_the_tail() {
        let mut r = Reverb::new();
        r.set_enabled(true);
        r.set_param("size", 100.0); // comb feedback ~0.98
        r.set_param("mix", 100.0);
        let mut buf = [f32::NAN, f32::INFINITY, -f32::INFINITY, 0.5, 0.5, 0.5];
        r.process(&mut buf, 48_000);
        for s in buf {
            assert!(s.is_finite(), "output must stay finite, got {s}");
        }
        // A poisoned sample must not have latched into the recursive combs.
        let mut buf2 = vec![0.1_f32; 4000];
        r.process(&mut buf2, 48_000);
        for s in buf2 {
            assert!(s.is_finite(), "no latched non-finite comb state, got {s}");
        }
    }

    // v1.42.0: the buffers re-scale to the device rate — at 96 kHz the reverb
    // still tails cleanly (rebuilt on the rate change, not just ignored).
    #[test]
    fn survives_and_tails_at_a_higher_rate() {
        let mut r = Reverb::new();
        r.set_enabled(true);
        r.set_param("size", 80.0);
        r.set_param("mix", 50.0);
        let mut buf = vec![0.0_f32; 8192];
        buf[0] = 1.0;
        r.process(&mut buf, 96_000); // triggers a retune to 96 kHz
        for s in &buf {
            assert!(s.is_finite(), "reverb stayed finite at 96 kHz");
        }
        let tail: f32 = buf[4000..8000].iter().map(|s| s.abs()).sum();
        assert!(tail > 0.1, "reverb should still tail at 96 kHz, got {tail}");
    }

    // v1.42.0: damping darkens/shortens the tail — heavy damping leaves less
    // late-tail energy than none.
    #[test]
    fn damping_reduces_late_tail_energy() {
        let run = |damp: f32| {
            let mut r = Reverb::new();
            r.set_enabled(true);
            r.set_param("size", 90.0);
            r.set_param("mix", 100.0);
            r.set_param("damp", damp);
            let mut buf = vec![0.0_f32; 8192];
            buf[0] = 1.0;
            r.process(&mut buf, 48_000);
            buf[5000..].iter().map(|s| s.abs()).sum::<f32>()
        };
        let bright = run(0.0);
        let dark = run(90.0);
        assert!(
            dark < bright,
            "damping should reduce late-tail energy (bright {bright}, dark {dark})"
        );
    }
}
