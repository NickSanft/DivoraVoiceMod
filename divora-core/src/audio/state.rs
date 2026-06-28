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
    /// v1.29.0: independent gate for hearing **Speak / soundboard "monitor-only"
    /// voices** (e.g. a TTS preview) in the monitor, separate from [`monitor`]
    /// (which gates hearing your own mic). Lets you disable mic monitoring
    /// without silencing Speak previews. `Default` false; `AudioEngine::new`
    /// sets it true so previews are audible out of the box.
    pub monitor_soundboard: AtomicBool,
    /// Phase 16: true while the modulated output is being recorded to a
    /// WAV file. Gates the output callback's push into the recording ring.
    pub recording: AtomicBool,
    /// v1.23.0: true while the DRY input (raw mic, pre-effects) is being
    /// captured to a WAV for voice cloning. Gates the input callback's push
    /// into the separate reference ring. Independent of `recording`.
    pub reference_recording: AtomicBool,
    /// v1.33.0: set by an output stream's error callback, cleared by the
    /// engine thread once it begins recovery. It COALESCES the burst of
    /// repeated errors a dying stream emits into a single rebuild request,
    /// so a device loss triggers exactly one teardown+rebuild, not a flood.
    pub stream_error_pending: AtomicBool,
    pub input_rms_bits: AtomicU32,
    pub input_peak_bits: AtomicU32,
    pub output_rms_bits: AtomicU32,
    pub output_peak_bits: AtomicU32,
    /// Phase 14: latency ADDED by the active DSP chain, in milliseconds
    /// (f32 bit pattern). Written by the output callback each buffer.
    pub dsp_latency_ms_bits: AtomicU32,
    /// v1.6.0: gain applied to the monitor ("hear yourself") stream
    /// (f32 bit pattern). 1.0 = unity. Set from the UI; read by the
    /// monitor callback. `Default` leaves this 0.0, so `AudioEngine::new`
    /// initializes it to 1.0.
    pub monitor_gain_bits: AtomicU32,
    /// v1.7.0: loudness normalization (auto-gain) on/off. `Default` false
    /// (the stage is opt-in). Read by the output callback each buffer.
    pub loudness_enabled: AtomicBool,
    /// v1.7.0: loudness target level in dBFS (f32 bit pattern). `Default`
    /// leaves 0.0, so `AudioEngine::new` initializes it to the real
    /// default target (−18 dBFS).
    pub loudness_target_bits: AtomicU32,
    /// v1.7.0: makeup gain the normalizer is currently applying, in dB
    /// (f32 bit pattern). Written by the output callback for the UI
    /// "it's working" readout; 0 dB while disabled.
    pub loudness_gain_db_bits: AtomicU32,
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

    pub fn store_monitor_gain(&self, gain: f32) {
        self.monitor_gain_bits
            .store(gain.to_bits(), Ordering::Release);
    }

    #[must_use]
    pub fn load_monitor_gain(&self) -> f32 {
        f32::from_bits(self.monitor_gain_bits.load(Ordering::Acquire))
    }

    pub fn store_loudness_enabled(&self, enabled: bool) {
        self.loudness_enabled.store(enabled, Ordering::Release);
    }

    #[must_use]
    pub fn load_loudness_enabled(&self) -> bool {
        self.loudness_enabled.load(Ordering::Acquire)
    }

    pub fn store_loudness_target(&self, dbfs: f32) {
        self.loudness_target_bits
            .store(dbfs.to_bits(), Ordering::Release);
    }

    #[must_use]
    pub fn load_loudness_target(&self) -> f32 {
        f32::from_bits(self.loudness_target_bits.load(Ordering::Acquire))
    }

    pub fn store_loudness_gain_db(&self, db: f32) {
        self.loudness_gain_db_bits
            .store(db.to_bits(), Ordering::Release);
    }

    #[must_use]
    pub fn load_loudness_gain_db(&self) -> f32 {
        f32::from_bits(self.loudness_gain_db_bits.load(Ordering::Acquire))
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
