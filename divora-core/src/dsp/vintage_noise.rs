//! Vintage-noise bed (v1.32.0) — the additive hiss + mains hum + sparse
//! crackle of an old radio's noise floor.
//!
//! This is the catalog's first **additive, signal-independent** effect. Every
//! other effect transforms the signal in place and zeros on silence, so the
//! gaps between words come out digitally silent — the dead giveaway that a
//! band-limited voice is just "a clean voice through a filter", not "came out
//! of a 1947 set". This bed is loudest in the gaps and **ducks under speech**
//! (an inverse gate), which is what flips the old-radio voices from "EQ'd
//! voice" to "real". **Run it LAST** — after the gate (which would otherwise
//! re-silence the bed) and before the downstream loudness limiter.

use super::{AudioEffect, EffectKind};

/// One-pole low-pass corner (Hz) for the "dark" end of the hiss colour.
const HISS_LP_HZ: f32 = 2200.0;
/// Crackle click decay (~1.5 ms).
const CRACKLE_DECAY_MS: f32 = 1.5;
/// Maps the ducking follower to a 0..1 gate: speech (|x| ≳ 1/K) fully ducks.
const DUCK_SENSITIVITY: f32 = 6.0;

pub struct VintageNoise {
    enabled: bool,
    hiss: f32,
    hum: f32,
    hum_hz: f32,
    crackle: f32,
    tone: f32,
    duck: f32,
    // --- state (allocated once; no per-buffer allocation) ---
    rng: u32,
    hiss_lp: f32,
    hum_phase: f32,
    crackle_env: f32,
    duck_env: f32,
}

impl VintageNoise {
    #[must_use]
    pub fn new() -> Self {
        Self {
            enabled: false,
            hiss: 0.25,
            hum: 0.12,
            hum_hz: 60.0,
            crackle: 0.15,
            tone: 0.5,
            duck: 0.70,
            rng: 0x1234_5678,
            hiss_lp: 0.0,
            hum_phase: 0.0,
            crackle_env: 0.0,
            duck_env: 0.0,
        }
    }

    fn next_u32(&mut self) -> u32 {
        // xorshift32 — cheap, RT-safe, deterministic.
        let mut x = self.rng;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.rng = x;
        x
    }

    /// Uniform in [0, 1).
    fn next_unit(&mut self) -> f32 {
        #[allow(clippy::cast_precision_loss)]
        let v = (self.next_u32() >> 8) as f32 / (1u32 << 24) as f32;
        v
    }

    /// White noise in [-1, 1].
    fn next_noise(&mut self) -> f32 {
        self.next_unit() * 2.0 - 1.0
    }
}

impl Default for VintageNoise {
    fn default() -> Self {
        Self::new()
    }
}

impl AudioEffect for VintageNoise {
    fn process(&mut self, buffer: &mut [f32], sample_rate: u32) {
        #[allow(clippy::cast_precision_loss)]
        let sr = sample_rate.max(1) as f32;
        let two_pi = 2.0 * std::f32::consts::PI;
        // Ducking follower: fast attack (catch speech), slow release (bed swells
        // back in the gaps).
        let duck_atk = (-1.0 / (0.002 * sr)).exp();
        let duck_rel = (-1.0 / (0.150 * sr)).exp();
        let hum_inc = self.hum_hz / sr; // cycles per sample
        let hiss_lp_c = (-two_pi * HISS_LP_HZ / sr).exp();
        let crackle_decay = (-1.0 / ((CRACKLE_DECAY_MS / 1000.0) * sr)).exp();
        // Per-sample crackle trigger probability (≈ up to ~38 clicks/s at full).
        let crackle_prob = self.crackle * 0.0008;

        for sample in buffer.iter_mut() {
            let x = if sample.is_finite() { *sample } else { 0.0 };

            // Input-level follower → 0..1 duck gate.
            let lvl = x.abs();
            let c = if lvl > self.duck_env {
                duck_atk
            } else {
                duck_rel
            };
            self.duck_env = c * self.duck_env + (1.0 - c) * lvl;
            let gate = (self.duck_env * DUCK_SENSITIVITY).min(1.0);
            let bed_gain = 1.0 - self.duck * gate;

            // Hiss: white noise, blended dark↔bright by `tone`.
            let n = self.next_noise();
            self.hiss_lp = hiss_lp_c * self.hiss_lp + (1.0 - hiss_lp_c) * n;
            let colored = self.tone * n + (1.0 - self.tone) * self.hiss_lp;
            let hiss = colored * self.hiss * 0.08;

            // Mains hum: a full-wave-rectified-ish stack (2× dominant).
            self.hum_phase = (self.hum_phase + hum_inc).fract();
            let w = two_pi * self.hum_phase;
            let hum = (w.sin() * 0.5 + (2.0 * w).sin() + (3.0 * w).sin() * 0.3) * self.hum * 0.035;

            // Crackle: sparse, sharp clicks with a short decay.
            if self.next_unit() < crackle_prob {
                self.crackle_env = self.next_noise() * (0.15 + 0.25 * self.next_unit());
            }
            let crackle = self.crackle_env * self.crackle;
            self.crackle_env *= crackle_decay;

            let bed = (hiss + hum + crackle) * bed_gain;
            let out = x + bed;
            *sample = if out.is_finite() { out } else { x };
        }
    }

    fn set_param(&mut self, key: &str, value: f32) {
        match key {
            "hiss" => self.hiss = (value / 100.0).clamp(0.0, 1.0),
            "hum" => self.hum = (value / 100.0).clamp(0.0, 1.0),
            "humFreq" => self.hum_hz = value.clamp(40.0, 120.0),
            "crackle" => self.crackle = (value / 100.0).clamp(0.0, 1.0),
            "tone" => self.tone = (value / 100.0).clamp(0.0, 1.0),
            "duck" => self.duck = (value / 100.0).clamp(0.0, 1.0),
            _ => {}
        }
    }

    fn enabled(&self) -> bool {
        self.enabled
    }

    fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
        if !enabled {
            self.crackle_env = 0.0;
            self.duck_env = 0.0;
        }
    }

    fn kind(&self) -> EffectKind {
        EffectKind::VintageNoise
    }
    // latency_samples: default 0 — additive, sample-by-sample.
}

#[cfg(test)]
mod tests {
    use super::{AudioEffect, VintageNoise};

    fn rms(buf: &[f32]) -> f32 {
        #[allow(clippy::cast_precision_loss)]
        let n = buf.len().max(1) as f32;
        (buf.iter().map(|s| s * s).sum::<f32>() / n).sqrt()
    }

    /// On a SILENT input the bed is audible — the whole point (the gaps must
    /// not be digitally silent).
    #[test]
    fn adds_a_bed_on_silence() {
        let mut v = VintageNoise::new();
        v.set_enabled(true);
        v.set_param("hiss", 60.0);
        v.set_param("hum", 40.0);
        v.set_param("duck", 70.0);
        let mut buf = vec![0.0_f32; 4800];
        v.process(&mut buf, 48_000);
        assert!(rms(&buf) > 0.001, "a noise bed should appear in silence");
    }

    /// The bed ducks under a loud signal (inverse gate) — its contribution is
    /// smaller relative to a quiet passage.
    #[test]
    fn ducks_under_speech() {
        let bed_level = |input: f32| {
            let mut v = VintageNoise::new();
            v.set_enabled(true);
            v.set_param("hiss", 80.0);
            v.set_param("duck", 100.0);
            let mut buf = vec![input; 9600];
            v.process(&mut buf, 48_000);
            // The bed is (output - input); measure its rms in the settled half.
            let settled: Vec<f32> = buf[4800..].iter().map(|s| s - input).collect();
            rms(&settled)
        };
        let quiet_bed = bed_level(0.0);
        let loud_bed = bed_level(0.5);
        assert!(
            loud_bed < quiet_bed * 0.6,
            "bed should duck under a loud signal (quiet {quiet_bed}, loud {loud_bed})"
        );
    }

    #[test]
    fn passes_the_voice_through() {
        let mut v = VintageNoise::new();
        v.set_enabled(true);
        v.set_param("hiss", 20.0);
        v.set_param("hum", 10.0);
        v.set_param("crackle", 0.0);
        // A strong signal dominates; the small bed rides on top, output ≈ input.
        let mut buf = [0.5_f32; 256];
        v.process(&mut buf, 48_000);
        for &s in &buf {
            assert!((s - 0.5).abs() < 0.1, "voice should pass ~through, got {s}");
        }
    }

    #[test]
    fn stays_finite_on_extreme_input() {
        let mut v = VintageNoise::new();
        v.set_enabled(true);
        let mut buf = [f32::NAN, f32::INFINITY, -f32::INFINITY, 1e30, -1e30, 0.0];
        v.process(&mut buf, 48_000);
        for s in buf {
            assert!(s.is_finite(), "output must stay finite, got {s}");
        }
    }

    #[test]
    fn adds_no_latency() {
        assert_eq!(VintageNoise::new().latency_samples(48_000), 0);
    }
}
