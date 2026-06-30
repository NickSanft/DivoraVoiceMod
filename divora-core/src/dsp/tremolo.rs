//! Tremolo (v1.34.0) — a slow **amplitude** LFO. Multiplies the signal by a
//! sine gain envelope that dips below unity at `rate` Hz by `depth`. Distinct
//! from `robot` (which ring-modulates at *audio* rate for a metallic carrier);
//! this modulates *loudness* at a sub-audio rate. The gentle pulsing "wobble"
//! behind a helicopter, a sci-fi pulse, classic guitar tremolo — and the
//! "hrm-hrm" mumble cadence that flips the Villager voice from "nasal voice"
//! to "villager". Sample-by-sample, zero latency, no per-buffer allocation.

use std::f32::consts::TAU;

use super::{AudioEffect, EffectKind};

pub struct Tremolo {
    enabled: bool,
    rate_hz: f32,
    /// Modulation depth in [0, 1]: the fraction the gain dips at the trough
    /// (0 = no effect, 1 = dips fully to silence).
    depth: f32,
    phase: f32,
}

impl Tremolo {
    #[must_use]
    pub fn new() -> Self {
        Self {
            enabled: false,
            rate_hz: 5.0,
            depth: 0.4,
            phase: 0.0,
        }
    }
}

impl Default for Tremolo {
    fn default() -> Self {
        Self::new()
    }
}

impl AudioEffect for Tremolo {
    fn process(&mut self, buffer: &mut [f32], sample_rate: u32) {
        #[allow(clippy::cast_precision_loss)]
        let sr = sample_rate.max(1) as f32;
        let phase_inc = TAU * self.rate_hz / sr;
        for sample in buffer.iter_mut() {
            // LFO in [0, 1]: 0 at the crest (full level), 1 at the trough.
            let lfo = 0.5 - 0.5 * self.phase.cos();
            let gain = 1.0 - self.depth * lfo;
            let out = *sample * gain;
            *sample = if out.is_finite() { out } else { 0.0 };
            self.phase += phase_inc;
            if self.phase > TAU {
                self.phase -= TAU;
            }
        }
    }

    fn set_param(&mut self, key: &str, value: f32) {
        match key {
            "rate" => self.rate_hz = value.clamp(0.1, 20.0),
            "depth" => self.depth = (value / 100.0).clamp(0.0, 1.0),
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
        EffectKind::Tremolo
    }
    // latency_samples: default 0 — sample-by-sample.
}

#[cfg(test)]
mod tests {
    use super::{AudioEffect, Tremolo};

    #[test]
    fn zero_depth_is_passthrough() {
        let mut t = Tremolo::new();
        t.set_enabled(true);
        t.set_param("depth", 0.0);
        let mut buf = [0.5_f32; 256];
        t.process(&mut buf, 48_000);
        for s in buf {
            assert!((s - 0.5).abs() < 1e-6, "depth 0 must pass through, got {s}");
        }
    }

    #[test]
    fn full_depth_swings_from_full_to_silence() {
        let mut t = Tremolo::new();
        t.set_enabled(true);
        t.set_param("rate", 10.0);
        t.set_param("depth", 100.0);
        // A constant input under a full-depth ~10 Hz LFO should swing from
        // ~full level down to ~silence across one cycle (100 ms = 4800 smp).
        let mut buf = vec![0.5_f32; 4800];
        t.process(&mut buf, 48_000);
        let max = buf.iter().copied().fold(f32::MIN, f32::max);
        let min = buf.iter().copied().fold(f32::MAX, f32::min);
        assert!(max > 0.49, "crest should reach ~full level, got {max}");
        assert!(min < 0.05, "trough should dip near silence, got {min}");
    }

    #[test]
    fn partial_depth_stays_above_its_floor() {
        // depth 40% should never dip below 60% of the input level.
        let mut t = Tremolo::new();
        t.set_enabled(true);
        t.set_param("depth", 40.0);
        let mut buf = vec![1.0_f32; 9600];
        t.process(&mut buf, 48_000);
        let min = buf.iter().copied().fold(f32::MAX, f32::min);
        assert!(min > 0.59, "depth 40% floors at ~0.6, got {min}");
    }

    #[test]
    fn stays_finite_on_extreme_input() {
        let mut t = Tremolo::new();
        t.set_enabled(true);
        t.set_param("depth", 60.0);
        let mut buf = [f32::NAN, f32::INFINITY, -f32::INFINITY, 1e30, -1e30, 0.0];
        t.process(&mut buf, 48_000);
        for s in buf {
            assert!(s.is_finite(), "output must stay finite, got {s}");
        }
    }

    #[test]
    fn adds_no_latency() {
        assert_eq!(Tremolo::new().latency_samples(48_000), 0);
    }
}
