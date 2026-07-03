//! Warble (v1.37.0) — a pitch VIBRATO. An LFO sweeps the read position of a
//! short delay line, so the output pitch bends up and down (Doppler) at
//! `rate` Hz by `depth`. Unlike `Chorus` (which SUMS a diluted modulated copy
//! on top of the untouched dry, so the fundamental never moves), Warble puts
//! the wobbled tap on the DRY path via `mix`, so at full mix the fundamental
//! itself warbles — the missing primitive behind the Enderman's otherworldly,
//! reversed-sounding wobble. Also useful for sci-fi, seasick, and theremin
//! voices. Because the dry is replaced by a delayed read, it adds a small
//! (~base-delay) latency — reported honestly.

use std::f32::consts::TAU;

use super::{AudioEffect, EffectKind};

/// Delay buffer length — generous headroom over base + swing at any device
/// rate (~125 ms at 192 kHz).
const MAX_DELAY_FRAMES: usize = 192_000 / 8;

/// Centre delay (ms) the LFO sweeps around. Must be >= `MOD_MS` so the read
/// position never crosses the write head. Also the effect's ~latency.
const BASE_MS: f32 = 12.0;

/// Maximum LFO swing (ms) either side of `BASE_MS` at depth = 100%.
const MOD_MS: f32 = 8.0;

pub struct Warble {
    enabled: bool,
    rate: f32,  // LFO rate (Hz)
    depth: f32, // 0..1 modulation depth
    mix: f32,   // 0..1 — 1.0 = pure vibrato (dry fully replaced by the tap)
    buffer: Vec<f32>,
    write_pos: usize,
    phase: f32,
}

impl Warble {
    #[must_use]
    pub fn new() -> Self {
        Self {
            enabled: false,
            rate: 6.0,
            depth: 0.5,
            mix: 1.0,
            buffer: vec![0.0; MAX_DELAY_FRAMES],
            write_pos: 0,
            phase: 0.0,
        }
    }

    /// Linear-interpolated read `delay_samples` (fractional) behind the write
    /// head, wrapping the circular buffer (mirrors `Chorus::read_delayed`).
    fn read_delayed(&self, delay_samples: f32) -> f32 {
        let len = self.buffer.len();
        #[allow(clippy::cast_precision_loss)]
        let read = ((self.write_pos as f32) - delay_samples).rem_euclid(len as f32);
        let floor = read.floor();
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let i0 = (floor as usize) % len;
        let i1 = (i0 + 1) % len;
        let frac = read - floor;
        self.buffer[i0] * (1.0 - frac) + self.buffer[i1] * frac
    }
}

impl Default for Warble {
    fn default() -> Self {
        Self::new()
    }
}

impl AudioEffect for Warble {
    fn process(&mut self, buffer: &mut [f32], sample_rate: u32) {
        if self.mix <= 0.0 {
            return; // fully dry — nothing to do
        }
        #[allow(clippy::cast_precision_loss)]
        let sr = sample_rate.max(1) as f32;
        let buf_len = self.buffer.len();
        let mod_ms = MOD_MS * self.depth;
        let inc = TAU * self.rate / sr;
        for sample in buffer.iter_mut() {
            let dry = if sample.is_finite() { *sample } else { 0.0 };
            self.buffer[self.write_pos] = dry;
            // Sweep the read delay around the base by the LFO → pitch bends.
            let delay_ms = BASE_MS + mod_ms * self.phase.sin();
            let delay_samp = (delay_ms / 1000.0) * sr;
            let wet = self.read_delayed(delay_samp);
            self.phase += inc;
            if self.phase >= TAU {
                self.phase -= TAU;
            }
            self.write_pos = (self.write_pos + 1) % buf_len;
            let out = dry * (1.0 - self.mix) + wet * self.mix;
            *sample = if out.is_finite() { out } else { dry };
        }
    }

    fn set_param(&mut self, key: &str, value: f32) {
        match key {
            "rate" => self.rate = value.clamp(0.1, 20.0),
            "depth" => self.depth = (value / 100.0).clamp(0.0, 1.0),
            "mix" => self.mix = (value / 100.0).clamp(0.0, 1.0),
            _ => {}
        }
    }

    fn enabled(&self) -> bool {
        self.enabled
    }

    fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
        if !enabled {
            self.phase = 0.0;
        }
    }

    fn kind(&self) -> EffectKind {
        EffectKind::Warble
    }

    fn latency_samples(&self, sample_rate: u32) -> usize {
        // The wet tap sits ~BASE_MS behind the input; when it replaces the dry
        // (mix > 0) that base delay is real latency.
        if self.mix <= 0.0 {
            return 0;
        }
        #[allow(
            clippy::cast_precision_loss,
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss
        )]
        let s = (BASE_MS / 1000.0 * sample_rate as f32) as usize;
        s
    }
}

#[cfg(test)]
mod tests {
    use super::{AudioEffect, Warble, BASE_MS};

    #[test]
    #[allow(clippy::float_cmp)] // mix=0 is a bit-exact passthrough
    fn mix_zero_is_passthrough() {
        let mut w = Warble::new();
        w.set_enabled(true);
        w.set_param("mix", 0.0);
        let mut buf = vec![0.0_f32; 128];
        buf[0] = 1.0;
        let before = buf.clone();
        w.process(&mut buf, 48_000);
        assert_eq!(buf, before);
    }

    #[test]
    fn full_mix_delays_and_wobbles_the_signal() {
        let mut w = Warble::new();
        w.set_enabled(true);
        w.set_param("mix", 100.0);
        w.set_param("depth", 60.0);
        // At full mix the dry is REPLACED by the delayed tap, so an impulse
        // does NOT pass straight through — it re-emerges around the base
        // delay (~12 ms = 576 samples at 48 kHz, ± the LFO swing).
        let mut buf = vec![0.0_f32; 2000];
        buf[0] = 1.0;
        w.process(&mut buf, 48_000);
        assert!(
            buf[0].abs() < 0.5,
            "dry impulse should be delayed, not passed"
        );
        let wet_energy: f32 = buf[300..900].iter().map(|s| s.abs()).sum();
        assert!(
            wet_energy > 0.1,
            "expected a delayed copy, got energy {wet_energy}"
        );
    }

    #[test]
    fn reports_its_base_delay_as_latency() {
        let w = Warble::new(); // default mix 1.0
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let expected = (BASE_MS / 1000.0 * 48_000.0) as usize;
        assert_eq!(w.latency_samples(48_000), expected);
        assert!(expected > 0);
    }

    #[test]
    fn stays_finite_on_extreme_input() {
        let mut w = Warble::new();
        w.set_enabled(true);
        let mut buf = [
            f32::NAN,
            f32::INFINITY,
            -f32::INFINITY,
            1e30,
            -1e30,
            0.0,
            0.5,
            -0.5,
        ];
        w.process(&mut buf, 48_000);
        for s in buf {
            assert!(s.is_finite(), "output must stay finite, got {s}");
        }
    }
}
