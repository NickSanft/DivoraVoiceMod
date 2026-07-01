//! Breath / whisperizer (v1.35.0) — additive band-passed noise whose level
//! **rides UP with your voice**. It is the deliberate inverse of
//! [`super::VintageNoise`], which ducks its bed *under* speech (an inverse
//! gate): here the hiss SWELLS as you speak (a fast-attack / slow-release
//! envelope follower gates the noise UP), so the sibilant "sss" builds with
//! each syllable — the building-fuse cue that makes the **Creeper** voice read
//! as a creeper rather than a generic breathy whisper. Also good for
//! whisper / ASMR / wind treatments.
//!
//! The noise is band-passed into the sss/air band (roughly 2.5 kHz…`color`)
//! so it reads as "hiss/fuse crackle", not low rumble. Additive and
//! sample-by-sample: **run it late** — after the gate (which would otherwise
//! re-silence it) and before any downstream loudness limiter. Zero latency,
//! no per-buffer allocation, RT-safe.

use super::{AudioEffect, EffectKind};

/// Fixed high-pass corner (Hz): strip the low rumble so the hiss sits up in
/// the sibilant band, not the chest.
const BREATH_HP_HZ: f32 = 2500.0;
/// Envelope-follower sensitivity range: `sens` 0..1 maps to this multiplier
/// applied to the input level before it saturates the 0..1 follow gate.
const SENS_MIN: f32 = 1.0;
const SENS_MAX: f32 = 16.0;
/// Peak additive hiss level at full amount + full swell.
const HISS_SCALE: f32 = 0.25;

pub struct Breath {
    enabled: bool,
    amount: f32,
    color: f32,
    swell: f32,
    sens: f32,
    // --- state (allocated once; no per-buffer allocation) ---
    rng: u32,
    hp_lp: f32,
    lp: f32,
    env: f32,
}

impl Breath {
    #[must_use]
    pub fn new() -> Self {
        Self {
            enabled: false,
            amount: 0.7,
            color: 0.65,
            swell: 0.8,
            sens: 0.6,
            rng: 0x2545_f491,
            hp_lp: 0.0,
            lp: 0.0,
            env: 0.0,
        }
    }

    fn next_u32(&mut self) -> u32 {
        // xorshift32 — cheap, RT-safe, deterministic (same PRNG as VintageNoise).
        let mut x = self.rng;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.rng = x;
        x
    }

    /// White noise in [-1, 1].
    fn next_noise(&mut self) -> f32 {
        #[allow(clippy::cast_precision_loss)]
        let v = (self.next_u32() >> 8) as f32 / (1u32 << 24) as f32;
        v * 2.0 - 1.0
    }
}

impl Default for Breath {
    fn default() -> Self {
        Self::new()
    }
}

impl AudioEffect for Breath {
    fn process(&mut self, buffer: &mut [f32], sample_rate: u32) {
        #[allow(clippy::cast_precision_loss)]
        let sr = sample_rate.max(1) as f32;
        let two_pi = 2.0 * std::f32::consts::PI;
        // Envelope follower: fast attack (catch the onset), slow release (the
        // hiss lingers and swells like a fuse) — the SAME shape as VintageNoise
        // but used to gate the noise UP instead of ducking it down.
        let atk = (-1.0 / (0.008 * sr)).exp();
        let rel = (-1.0 / (0.180 * sr)).exp();
        // Noise band-pass: fixed HP to strip rumble, `color`-swept LP for
        // dark↔bright (≈ 3.5 kHz dark … 8 kHz bright).
        let hp_c = (-two_pi * BREATH_HP_HZ / sr).exp();
        let lp_hz = 3500.0 + self.color * 4500.0;
        let lp_c = (-two_pi * lp_hz / sr).exp();
        let sens_mult = SENS_MIN + self.sens * (SENS_MAX - SENS_MIN);
        // `swell` 1 → follows the envelope fully (silent in gaps); 0 → a static
        // bed. The floor is the always-on fraction.
        let floor = 1.0 - self.swell;

        for sample in buffer.iter_mut() {
            let x = if sample.is_finite() { *sample } else { 0.0 };

            // Input-level follower → 0..1 gate that RISES with the voice.
            let lvl = x.abs();
            let c = if lvl > self.env { atk } else { rel };
            self.env = c * self.env + (1.0 - c) * lvl;
            let follow = (self.env * sens_mult).min(1.0);
            let noise_gain = self.amount * (floor + (1.0 - floor) * follow);

            // Band-passed white noise (HP then LP) → sits in the sss/air band.
            let n = self.next_noise();
            self.hp_lp = hp_c * self.hp_lp + (1.0 - hp_c) * n;
            let hp = n - self.hp_lp;
            self.lp = lp_c * self.lp + (1.0 - lp_c) * hp;
            let hiss = self.lp * noise_gain * HISS_SCALE;

            let out = x + hiss;
            *sample = if out.is_finite() { out } else { x };
        }
    }

    fn set_param(&mut self, key: &str, value: f32) {
        match key {
            "amount" => self.amount = (value / 100.0).clamp(0.0, 1.0),
            "color" => self.color = (value / 100.0).clamp(0.0, 1.0),
            "swell" => self.swell = (value / 100.0).clamp(0.0, 1.0),
            "sens" => self.sens = (value / 100.0).clamp(0.0, 1.0),
            _ => {}
        }
    }

    fn enabled(&self) -> bool {
        self.enabled
    }

    fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
        if !enabled {
            self.env = 0.0;
        }
    }

    fn kind(&self) -> EffectKind {
        EffectKind::Breath
    }
    // latency_samples: default 0 — additive, sample-by-sample.
}

#[cfg(test)]
mod tests {
    use super::{AudioEffect, Breath};

    fn rms(buf: &[f32]) -> f32 {
        #[allow(clippy::cast_precision_loss)]
        let n = buf.len().max(1) as f32;
        (buf.iter().map(|s| s * s).sum::<f32>() / n).sqrt()
    }

    /// The DEFINING behaviour, and the exact inverse of `VintageNoise`'s
    /// `ducks_under_speech`: the hiss RISES with input level.
    #[test]
    fn hiss_rises_with_input() {
        let bed_level = |input: f32| {
            let mut b = Breath::new();
            b.set_enabled(true);
            b.set_param("amount", 90.0);
            b.set_param("swell", 70.0);
            b.set_param("sens", 60.0);
            let mut buf = vec![input; 9600];
            b.process(&mut buf, 48_000);
            // The added hiss is (output - input); measure its rms in the
            // settled half so the envelope follower has caught up.
            let settled: Vec<f32> = buf[4800..].iter().map(|s| s - input).collect();
            rms(&settled)
        };
        let quiet_bed = bed_level(0.0);
        let loud_bed = bed_level(0.5);
        assert!(
            loud_bed > quiet_bed * 1.8,
            "hiss should SWELL with a loud signal (quiet {quiet_bed}, loud {loud_bed})"
        );
    }

    /// A loud voice passes through with the hiss riding on top (output ≈ input).
    #[test]
    fn passes_the_voice_through() {
        let mut b = Breath::new();
        b.set_enabled(true);
        b.set_param("amount", 40.0);
        let mut buf = [0.5_f32; 256];
        b.process(&mut buf, 48_000);
        for &s in &buf {
            assert!((s - 0.5).abs() < 0.2, "voice should pass ~through, got {s}");
        }
    }

    #[test]
    fn stays_finite_on_extreme_input() {
        let mut b = Breath::new();
        b.set_enabled(true);
        let mut buf = [f32::NAN, f32::INFINITY, -f32::INFINITY, 1e30, -1e30, 0.0];
        b.process(&mut buf, 48_000);
        for s in buf {
            assert!(s.is_finite(), "output must stay finite, got {s}");
        }
    }

    #[test]
    fn adds_no_latency() {
        assert_eq!(Breath::new().latency_samples(48_000), 0);
    }
}
