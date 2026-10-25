//! Shared state for the audio engine. The audio thread writes; the UI
//! thread reads. Lock-free via atomic loads/stores; f32 values are encoded
//! through their bit pattern.

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};

use serde::{Deserialize, Serialize};

use super::mute::{mute_transition, unpack, InputMute, MuteChange, MuteOp};
use super::ptm::{is_bypassed, ptm_transition, unpack as unpack_ptm, PtmChange, PtmOp, PtmState};

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
    /// v1.39.0: true when device-loss auto-recovery has GIVEN UP (exhausted its
    /// retry budget, or the rebuild itself failed) and the engine is stopped
    /// with no audio. Cleared on a successful recovery or a manual Start/Stop.
    /// The UI polls this to show a "device lost — restart" banner instead of a
    /// silent, metering-but-dead "running" state.
    pub recovery_failed: AtomicBool,
    /// v1.54.0: how many times the user's own command has left the engine
    /// stopped — a Stop, or a Start whose devices would not open. Written by
    /// the engine thread only, never by a callback, and never by a device-loss
    /// rebuild. See [`AudioEngine::user_stop_count`](super::AudioEngine::user_stop_count).
    pub user_stops: AtomicU64,
    pub input_rms_bits: AtomicU32,
    pub input_peak_bits: AtomicU32,
    pub output_rms_bits: AtomicU32,
    pub output_peak_bits: AtomicU32,
    /// Phase 14: latency ADDED by the active DSP chain, in milliseconds
    /// (f32 bit pattern). Written by the output callback each buffer.
    pub dsp_latency_ms_bits: AtomicU32,
    /// v1.46.0: current reactive-modulation depth, 0..=1 (f32 bit pattern).
    /// Written by the output callback each buffer; drives the UI meter.
    /// Rests at 0.0, which is also the correct value when the feature is
    /// off, so the derived `Default` needs no special-casing here.
    pub reactive_depth_bits: AtomicU32,
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
    /// v1.52.0: true while a screen recording takes the voice and soundboard.
    /// Gates the output callback's push into the screen recorder's tap — the
    /// one atomic load it costs a buffer while nothing records.
    pub av_recording: AtomicBool,
    /// v1.54.0: the take the screen recorder's tap is for, while one is
    /// attached (0 otherwise). Each attach is a new number, so the callback
    /// starts a new run for it even when it never saw the last take let go.
    pub av_take: AtomicU64,
    /// v1.52.0: engine-rate frames the screen recorder's tap could not keep,
    /// since the take began: full rings in the callback, or a full channel to
    /// the recorder. The recording has silence where they were.
    pub av_dropped: AtomicU64,
    /// v1.52.0: how old the newest microphone frame is, on average, when the
    /// input callback hands it to the ring, in microseconds. Written by the
    /// input callback only while a screen recording runs.
    pub input_latency_us: AtomicU32,
    /// v1.56.0: Divora's mute of the voice: `seq << 1 | muted` (see
    /// `audio::mute`). Engine-lifetime: teardown never touches it. `Default`
    /// 0 = unmuted, seq 0.
    pub input_mute: AtomicU64,
    /// v1.58.0: push-to-modulate: `seq << 16 | mode << 8 | held` (see
    /// `audio::ptm`). Engine-lifetime: teardown never touches it. `Default`
    /// 0 = always (push-to-modulate off), nothing held, seq 0.
    pub ptm: AtomicU64,
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

    pub fn store_reactive_depth(&self, depth: f32) {
        self.reactive_depth_bits
            .store(depth.to_bits(), Ordering::Release);
    }

    #[must_use]
    pub fn load_reactive_depth(&self) -> f32 {
        f32::from_bits(self.reactive_depth_bits.load(Ordering::Acquire))
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

    pub fn store_input_latency_us(&self, us: u32) {
        self.input_latency_us.store(us, Ordering::Relaxed);
    }

    #[must_use]
    pub fn load_input_latency_us(&self) -> u32 {
        self.input_latency_us.load(Ordering::Relaxed)
    }

    /// v1.56.0: the callback's read of the mute: bit 0, `Acquire`.
    #[must_use]
    pub fn input_muted(&self) -> bool {
        self.input_mute.load(Ordering::Acquire) & 1 == 1
    }

    /// v1.56.0: the mute and its change count, from one load.
    #[must_use]
    pub fn load_input_mute(&self) -> InputMute {
        unpack(self.input_mute.load(Ordering::Acquire))
    }

    /// v1.56.0: apply `op` in one atomic op. `before` is the word it found;
    /// `after` is the word it left, which is `before` when nothing changed.
    ///
    /// The compare-and-swap loop `fetch_update` runs, written out: that name
    /// is deprecated on current stable, and its replacement `try_update` is
    /// newer than the workspace's `rust-version`.
    pub(crate) fn apply_input_mute(&self, op: MuteOp) -> MuteChange {
        let mut prev = self.input_mute.load(Ordering::Acquire);
        let next = loop {
            let Some(next) = mute_transition(prev, op) else {
                break prev;
            };
            match self.input_mute.compare_exchange_weak(
                prev,
                next,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => break next,
                Err(actual) => prev = actual,
            }
        };
        MuteChange {
            before: unpack(prev),
            after: unpack(next),
        }
    }

    /// v1.58.0: the callback's read of push-to-modulate: whether it switches
    /// to the dry voice, from one `Acquire` load.
    #[must_use]
    pub fn ptm_bypassed(&self) -> bool {
        is_bypassed(self.ptm.load(Ordering::Acquire))
    }

    /// v1.58.0: push-to-modulate and its change count, from one load.
    #[must_use]
    pub fn load_ptm(&self) -> PtmState {
        unpack_ptm(self.ptm.load(Ordering::Acquire))
    }

    /// v1.58.0: apply `op` in one atomic op, the compare-and-swap loop
    /// [`apply_input_mute`](Self::apply_input_mute) runs. `before` is the
    /// word it found; `after` is the word it left, which is `before` when
    /// nothing changed.
    pub(crate) fn apply_ptm(&self, op: PtmOp) -> PtmChange {
        let mut prev = self.ptm.load(Ordering::Acquire);
        let next = loop {
            let Some(next) = ptm_transition(prev, op) else {
                break prev;
            };
            match self
                .ptm
                .compare_exchange_weak(prev, next, Ordering::AcqRel, Ordering::Acquire)
            {
                Ok(_) => break next,
                Err(actual) => prev = actual,
            }
        };
        PtmChange {
            before: unpack_ptm(prev),
            after: unpack_ptm(next),
        }
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
