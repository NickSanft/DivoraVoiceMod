//! Shared state for the audio engine. The audio thread writes; the UI
//! thread reads. Lock-free via atomic loads/stores; f32 values are encoded
//! through their bit pattern.

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use serde::{Deserialize, Serialize};

/// RMS and peak for one audio path.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct Levels {
    pub rms: f32,
    pub peak: f32,
}

/// Atomically-shared engine state.
#[derive(Debug, Default)]
pub struct EngineState {
    pub running: AtomicBool,
    pub monitor: AtomicBool,
    pub input_rms_bits: AtomicU32,
    pub input_peak_bits: AtomicU32,
    pub output_rms_bits: AtomicU32,
    pub output_peak_bits: AtomicU32,
    /// Phase 14: latency ADDED by the active DSP chain, in milliseconds
    /// (f32 bit pattern). Written by the output callback each buffer.
    pub dsp_latency_ms_bits: AtomicU32,
}

impl EngineState {
    pub fn store_input(&self, levels: Levels) {
        self.input_rms_bits
            .store(levels.rms.to_bits(), Ordering::Release);
        self.input_peak_bits
            .store(levels.peak.to_bits(), Ordering::Release);
    }

    pub fn store_output(&self, levels: Levels) {
        self.output_rms_bits
            .store(levels.rms.to_bits(), Ordering::Release);
        self.output_peak_bits
            .store(levels.peak.to_bits(), Ordering::Release);
    }

    pub fn load_input(&self) -> Levels {
        Levels {
            rms: f32::from_bits(self.input_rms_bits.load(Ordering::Acquire)),
            peak: f32::from_bits(self.input_peak_bits.load(Ordering::Acquire)),
        }
    }

    pub fn load_output(&self) -> Levels {
        Levels {
            rms: f32::from_bits(self.output_rms_bits.load(Ordering::Acquire)),
            peak: f32::from_bits(self.output_peak_bits.load(Ordering::Acquire)),
        }
    }

    pub fn store_dsp_latency_ms(&self, ms: f32) {
        self.dsp_latency_ms_bits
            .store(ms.to_bits(), Ordering::Release);
    }

    #[must_use]
    pub fn load_dsp_latency_ms(&self) -> f32 {
        f32::from_bits(self.dsp_latency_ms_bits.load(Ordering::Acquire))
    }
}

#[cfg(test)]
mod tests {
    use super::{EngineState, Levels};

    #[test]
    fn store_and_load_input_roundtrip() {
        let state = EngineState::default();
        let l = Levels {
            rms: 0.42,
            peak: 0.81,
        };
        state.store_input(l);
        let back = state.load_input();
        assert!((back.rms - 0.42).abs() < 1e-6);
        assert!((back.peak - 0.81).abs() < 1e-6);
    }

    #[test]
    fn store_and_load_output_roundtrip() {
        let state = EngineState::default();
        let l = Levels {
            rms: 0.123,
            peak: 0.456,
        };
        state.store_output(l);
        let back = state.load_output();
        assert!((back.rms - 0.123).abs() < 1e-6);
        assert!((back.peak - 0.456).abs() < 1e-6);
    }

    #[test]
    fn nan_round_trips() {
        let state = EngineState::default();
        state.store_input(Levels {
            rms: f32::NAN,
            peak: f32::NAN,
        });
        let back = state.load_input();
        assert!(back.rms.is_nan());
        assert!(back.peak.is_nan());
    }
}
