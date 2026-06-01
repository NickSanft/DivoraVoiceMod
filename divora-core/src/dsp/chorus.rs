//! Chorus / doubler. Sums the dry signal with several short, LFO-
//! modulated delay taps. Continuously varying each tap's delay detunes
//! its pitch slightly (Doppler), so one voice reads as an ensemble —
//! "many voices from one." Used by the Coven's "Choir of Ash" voice.
//!
//! The dry path is never delayed, so this adds no algorithmic latency
//! (like echo / reverb, the wet is a tail on top of the dry).

use std::f32::consts::TAU;

use super::{AudioEffect, EffectKind};

/// Delay buffer length. ~120 ms at 192 kHz — comfortably covers the base
/// delay plus the full modulation swing at any device rate.
const MAX_DELAY_FRAMES: usize = 192_000 / 8;

/// Number of detuned voices summed into the wet signal.
const VOICES: usize = 3;

/// Base delay before modulation (ms).
const BASE_MS: f32 = 20.0;

/// Maximum extra modulation swing (ms) at depth = 100%.
const MOD_MS: f32 = 8.0;

pub struct Chorus {
    enabled: bool,
    mix: f32,   // 0..1 wet amount
    depth: f32, // 0..1 modulation depth
    rate: f32,  // base LFO rate (Hz)
    buffer: Vec<f32>,
    write_pos: usize,
    phases: [f32; VOICES],
}

impl Chorus {
    #[must_use]
    pub fn new() -> Self {
        // Spread the voices evenly around the LFO cycle so at any instant
        // they detune in different directions — a wider ensemble.
        let mut phases = [0.0_f32; VOICES];
        for (i, p) in phases.iter_mut().enumerate() {
            #[allow(clippy::cast_precision_loss)]
            let frac = i as f32 / VOICES as f32;
            *p = TAU * frac;
        }
        Self {
            enabled: false,
            mix: 0.7,
            depth: 0.6,
            rate: 0.6,
            buffer: vec![0.0; MAX_DELAY_FRAMES],
            write_pos: 0,
            phases,
        }
    }

    /// Linear-interpolated read of the buffer `delay_samples` (fractional)
    /// behind the write head, wrapping around the circular buffer.
    fn read_delayed(&self, delay_samples: f32) -> f32 {
        let len = self.buffer.len();
        #[allow(clippy::cast_precision_loss)]
        let read = (self.write_pos as f32) - delay_samples;
        #[allow(clippy::cast_precision_loss)]
        let read = read.rem_euclid(len as f32);
        let floor = read.floor();
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let i0 = (floor as usize) % len;
        let i1 = (i0 + 1) % len;
        let frac = read - floor;
        self.buffer[i0] * (1.0 - frac) + self.buffer[i1] * frac
    }
}

impl Default for Chorus {
    fn default() -> Self {
        Self::new()
    }
}

impl AudioEffect for Chorus {
    fn process(&mut self, buffer: &mut [f32], sample_rate: u32) {
        if self.mix <= 0.0 {
            return; // fully dry — nothing to add
        }
        #[allow(clippy::cast_precision_loss)]
        let sr = sample_rate as f32;
        let buf_len = self.buffer.len();
        // Slightly detuned per-voice rates make the ensemble less
        // periodic (richer, less "wobble").
        let rates = [self.rate, self.rate * 1.07, self.rate * 0.91];
        let mod_ms = MOD_MS * self.depth;
        #[allow(clippy::cast_precision_loss)]
        let per_voice = 1.5 / VOICES as f32;
        for sample in buffer.iter_mut() {
            let dry = *sample;
            self.buffer[self.write_pos] = dry;
            // Read pass: sum each voice's modulated-delay tap. Borrows
            // self immutably (phases + buffer), so read_delayed is fine.
            let mut wet = 0.0_f32;
            for &phase in &self.phases {
                let delay_ms = BASE_MS + mod_ms * phase.sin();
                let delay_samp = (delay_ms / 1000.0) * sr;
                wet += self.read_delayed(delay_samp);
            }
            // Advance pass: step each LFO at its (slightly detuned) rate.
            for (phase, rate) in self.phases.iter_mut().zip(rates.iter()) {
                *phase += TAU * rate / sr;
                if *phase >= TAU {
                    *phase -= TAU;
                }
            }
            self.write_pos = (self.write_pos + 1) % buf_len;
            // Dry stays fully present; the summed detuned copies add the
            // ensemble body on top, scaled by mix.
            *sample = dry + wet * per_voice * self.mix;
        }
    }

    fn set_param(&mut self, key: &str, value: f32) {
        match key {
            "mix" => self.mix = (value / 100.0).clamp(0.0, 1.0),
            "depth" => self.depth = (value / 100.0).clamp(0.0, 1.0),
            "rate" => self.rate = value.clamp(0.05, 8.0),
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
        EffectKind::Chorus
    }
}

#[cfg(test)]
mod tests {
    use super::{AudioEffect, Chorus};

    #[test]
    #[allow(clippy::float_cmp)] // mix=0 is a bit-exact passthrough
    fn mix_zero_is_passthrough() {
        let mut c = Chorus::new();
        c.set_enabled(true);
        c.set_param("mix", 0.0);
        let mut buf = vec![0.0_f32; 128];
        buf[0] = 1.0;
        let before = buf.clone();
        c.process(&mut buf, 48_000);
        assert_eq!(buf, before);
    }

    #[test]
    fn produces_delayed_ensemble_copies() {
        let mut c = Chorus::new();
        c.set_enabled(true);
        c.set_param("mix", 100.0);
        c.set_param("depth", 50.0);
        // An impulse should reappear as several smeared, detuned copies
        // around the base delay (~20 ms = 960 samples at 48 kHz, ±swing).
        let mut buf = vec![0.0_f32; 2000];
        buf[0] = 1.0;
        c.process(&mut buf, 48_000);
        // Dry impulse passes through untouched.
        assert!((buf[0] - 1.0).abs() < 0.01, "dry impulse preserved");
        // Wet copies show up in the delay window.
        let wet_energy: f32 = buf[400..1500].iter().map(|s| s.abs()).sum();
        assert!(
            wet_energy > 0.1,
            "expected delayed ensemble copies, got energy {wet_energy}"
        );
    }

    #[test]
    fn adds_no_latency() {
        // Wet-tail effect — the dry path isn't delayed.
        let c = Chorus::new();
        assert_eq!(c.latency_samples(48_000), 0);
    }
}
