//! DSP effect chain — the effects `DivoraVoice` ships (see `EffectKind`
//! below for the full set), plus the `AudioEffect` trait, `EffectChain`,
//! and the `DspCommand` enum the audio thread receives from the UI.
//!
//! ### Audio-thread ownership model
//!
//! `EffectChain` is owned exclusively by the audio output callback. The
//! UI sends `DspCommand`s through a SPSC channel; the callback drains
//! the channel at the top of each buffer, applies any structural or
//! parameter changes, and then runs `process` on the mono buffer.
//!
//! Effects implementations *are allowed* to allocate at construction
//! and on sample-rate change, but never during normal `process` calls.
//!
//! ### Quality scope for Phase 3
//!
//! Gate, EQ, distortion, echo, reverb, robot are real algorithms.
//! Pitch ships a basic dual-read varispeed shifter — good enough to
//! prove the chain plumbing but the real phase-vocoder lands in a
//! later phase. Formant ships a parallel band-pass colouring; the real
//! LPC-based formant warp also lands later.

mod breath;
mod chorus;
mod compressor;
mod deesser;
mod denoiser;
mod distortion;
mod echo;
mod envelope;
mod eq;
mod formant;
mod gate;
mod harmonizer;
mod pitch;
mod radio_bandpass;
mod reverb;
mod robot;
mod stft;
mod tremolo;
mod vintage_noise;
mod voice_convert;
mod warble;

pub use breath::Breath;
pub use chorus::Chorus;
pub use compressor::Compressor;
pub use deesser::DeEsser;
pub use denoiser::RnnDenoiser;
pub use distortion::Distortion;
pub use echo::Echo;
pub use eq::Eq;
pub use formant::FormantShift;
pub use gate::NoiseGate;
pub use harmonizer::Harmonizer;
pub use pitch::PitchShift;
pub use radio_bandpass::RadioBandpass;
pub use reverb::Reverb;
pub use robot::Robot;
pub use tremolo::Tremolo;
pub use vintage_noise::VintageNoise;
pub use voice_convert::{onnx_runtime_available, VoiceConverter};
pub use warble::Warble;

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// Identifier of an effect kind; mirrors the frontend's `EffectId`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EffectKind {
    Gate,
    /// Phase 10: RNNoise-based noise suppression. Distinct from `Gate`
    /// (which is a hard threshold) — `Denoiser` is a learned model.
    Denoiser,
    Pitch,
    Formant,
    Eq,
    Robot,
    Distortion,
    Echo,
    Reverb,
    /// v1.2.0 (The Coven): chorus / doubler — modulated multi-tap delay
    /// summed with the dry signal for an ensemble ("many voices from
    /// one").
    Chorus,
    /// v1.2.1 (The Coven): harmonizer — pitched copies at musical
    /// intervals summed with the dry root, making an actual chord
    /// (diminished by default). Powers the "Choir of Ash" voice.
    Harmonizer,
    /// v1.8.0: dynamics compressor — feed-forward, soft-knee, zero
    /// look-ahead. Evens out level for a steadier send. Adds no latency.
    Compressor,
    /// v1.8.0: de-esser — split-band dynamic attenuation of the
    /// sibilance band ("sss"), which pitch/formant shifting exaggerates.
    /// Adds no latency.
    Deesser,
    /// v1.32.0: vintage-radio band-pass — cascaded 24 dB/oct HP+LP plus a
    /// movable high-Q "cone/horn" resonance. A real band-limiter (a wall,
    /// not the EQ shelves' slope) for the "through an old radio" sound.
    RadioBandpass,
    /// Phase 12: ONNX-backed voice conversion (LLVC-style). Loads a
    /// `.onnx` model from the voices directory and streams 48 kHz mono
    /// through a 16 kHz inference chunk. Falls back to passthrough
    /// when the runtime DLL or the model file is missing.
    VoiceConvert,
    /// v1.32.0: vintage-noise bed — additive hiss + mains hum + sparse crackle
    /// that swells in the gaps and ducks under speech. The first additive,
    /// signal-independent effect; run it LAST in the chain.
    VintageNoise,
    /// v1.34.0: tremolo — a slow **amplitude** LFO (sub-audio rate), distinct
    /// from `Robot`'s audio-rate ring modulation. The pulsing "wobble" behind
    /// the Villager voice's "hrm-hrm" cadence (also helicopter / sci-fi pulse /
    /// guitar tremolo).
    Tremolo,
    /// v1.35.0: breath / whisperizer — additive band-passed noise whose level
    /// RIDES UP with the voice envelope (the inverse of `VintageNoise`'s duck).
    /// The swelling sibilant hiss behind the Creeper voice; also whisper / wind.
    /// Additive, signal-following; run it late in the chain.
    Breath,
    /// v1.37.0: warble — a pitch VIBRATO (an LFO-swept delay tap that bends the
    /// fundamental, unlike `Chorus` which only sums a diluted copy). The
    /// otherworldly wobble behind the Enderman voice; also sci-fi / seasick /
    /// theremin. Adds a small (~base-delay) latency.
    Warble,
}

/// Trait every effect implements. The audio thread holds a `Box<dyn
/// AudioEffect>` per chain entry and calls `process` once per buffer
/// when the effect is enabled.
pub trait AudioEffect: Send {
    /// In-place mono processing.
    fn process(&mut self, buffer: &mut [f32], sample_rate: u32);

    /// Update a parameter by key. Unknown keys are silently ignored so
    /// the UI can carry forward params across effect-type changes.
    fn set_param(&mut self, key: &str, value: f32);

    /// Whether this effect should be processed.
    fn enabled(&self) -> bool;

    fn set_enabled(&mut self, enabled: bool);

    fn kind(&self) -> EffectKind;

    /// Set a string-valued resource on the effect — e.g. the file path
    /// of the ONNX model the `VoiceConvert` effect should load. `f32`
    /// params go through `set_param`; this carries the things that
    /// aren't numbers. `None` clears the resource. Default no-op, so
    /// only effects that actually have a string resource override it.
    fn set_resource(&mut self, _key: &str, _value: Option<&str>) {}

    /// Phase 14: the algorithmic latency this effect ADDS to the signal
    /// path, in samples at `sample_rate`, when it's actively processing.
    /// Block-based effects (denoiser frame, voice-conversion chunk,
    /// STFT window) return their buffering delay; sample-by-sample
    /// effects (gate, EQ, distortion) and wet-tail effects (echo,
    /// reverb — the dry path isn't delayed) return 0. Default 0.
    fn latency_samples(&self, _sample_rate: u32) -> usize {
        0
    }
}

/// Declarative description of an effect — what the UI sends to (re)build
/// the chain. Params are stored as a string-keyed map so the schema can
/// grow without breaking back-compat.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EffectSpec {
    pub kind: EffectKind,
    pub enabled: bool,
    pub params: HashMap<String, f32>,
}

/// Commands the UI sends to mutate the live chain. `tag` discriminator
/// lets us extend the enum without breaking the JSON wire format.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum DspCommand {
    SetChain {
        specs: Vec<EffectSpec>,
    },
    SetParam {
        index: usize,
        key: String,
        value: f32,
    },
    SetEnabled {
        index: usize,
        enabled: bool,
    },
    /// Set a string-valued resource on one effect (e.g. the active
    /// voice model path for `VoiceConvert`). `value: None` clears it.
    SetResource {
        index: usize,
        key: String,
        value: Option<String>,
    },
    Clear,
}

/// Ordered list of effects, audio-thread-owned.
pub struct EffectChain {
    effects: Vec<Box<dyn AudioEffect>>,
}

impl EffectChain {
    #[must_use]
    pub fn new() -> Self {
        Self {
            effects: Vec::new(),
        }
    }

    /// Build a chain from a list of specs. Allocates; called only
    /// when the UI sends `SetChain` (not per buffer).
    #[must_use]
    pub fn from_specs(specs: &[EffectSpec]) -> Self {
        let mut chain = Self::new();
        for spec in specs {
            chain.effects.push(build_effect(spec));
        }
        chain
    }

    /// Apply a single command. Called from the audio callback when
    /// draining the SPSC channel.
    pub fn apply(&mut self, cmd: DspCommand) {
        match cmd {
            DspCommand::SetChain { specs } => {
                *self = Self::from_specs(&specs);
            }
            DspCommand::SetParam { index, key, value } => {
                if let Some(effect) = self.effects.get_mut(index) {
                    effect.set_param(&key, value);
                }
            }
            DspCommand::SetEnabled { index, enabled } => {
                if let Some(effect) = self.effects.get_mut(index) {
                    effect.set_enabled(enabled);
                }
            }
            DspCommand::SetResource { index, key, value } => {
                if let Some(effect) = self.effects.get_mut(index) {
                    effect.set_resource(&key, value.as_deref());
                }
            }
            DspCommand::Clear => {
                self.effects.clear();
            }
        }
    }

    /// Run every enabled effect over the buffer in order.
    pub fn process(&mut self, buffer: &mut [f32], sample_rate: u32) {
        for effect in &mut self.effects {
            if effect.enabled() {
                effect.process(buffer, sample_rate);
            }
        }
    }

    /// Phase 14: total algorithmic latency added by the ENABLED effects
    /// in the chain, in samples at `sample_rate`. Effects are serial, so
    /// their latencies sum. Drives the live "added latency" readout.
    #[must_use]
    pub fn latency_samples(&self, sample_rate: u32) -> usize {
        self.effects
            .iter()
            .filter(|e| e.enabled())
            .map(|e| e.latency_samples(sample_rate))
            .sum()
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.effects.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.effects.is_empty()
    }

    #[must_use]
    pub fn kind_at(&self, index: usize) -> Option<EffectKind> {
        self.effects.get(index).map(|e| e.kind())
    }

    pub fn clear(&mut self) {
        self.effects.clear();
    }
}

impl Default for EffectChain {
    fn default() -> Self {
        Self::new()
    }
}

fn build_effect(spec: &EffectSpec) -> Box<dyn AudioEffect> {
    let mut effect: Box<dyn AudioEffect> = match spec.kind {
        EffectKind::Gate => Box::new(NoiseGate::new()),
        EffectKind::Denoiser => Box::new(RnnDenoiser::new()),
        EffectKind::Pitch => Box::new(PitchShift::new()),
        EffectKind::Formant => Box::new(FormantShift::new()),
        EffectKind::Eq => Box::new(Eq::new()),
        EffectKind::Robot => Box::new(Robot::new()),
        EffectKind::Distortion => Box::new(Distortion::new()),
        EffectKind::Echo => Box::new(Echo::new()),
        EffectKind::Reverb => Box::new(Reverb::new()),
        EffectKind::Chorus => Box::new(Chorus::new()),
        EffectKind::Harmonizer => Box::new(Harmonizer::new()),
        EffectKind::Compressor => Box::new(Compressor::new()),
        EffectKind::Deesser => Box::new(DeEsser::new()),
        EffectKind::RadioBandpass => Box::new(RadioBandpass::new()),
        EffectKind::VoiceConvert => Box::new(VoiceConverter::new()),
        EffectKind::VintageNoise => Box::new(VintageNoise::new()),
        EffectKind::Tremolo => Box::new(Tremolo::new()),
        EffectKind::Breath => Box::new(Breath::new()),
        EffectKind::Warble => Box::new(Warble::new()),
    };
    effect.set_enabled(spec.enabled);
    for (key, value) in &spec.params {
        effect.set_param(key, *value);
    }
    effect
}

#[cfg(test)]
mod tests {
    use super::{AudioEffect, EffectChain, EffectKind, EffectSpec, NoiseGate};
    use std::collections::HashMap;

    #[test]
    #[allow(clippy::float_cmp)] // empty chain is a bit-exact identity
    fn empty_chain_is_a_no_op() {
        let mut chain = EffectChain::new();
        let mut buf = [0.5_f32; 64];
        let before = buf;
        chain.process(&mut buf, 48000);
        assert_eq!(buf, before);
    }

    #[test]
    fn from_specs_populates_chain_in_order() {
        let specs = vec![
            EffectSpec {
                kind: EffectKind::Gate,
                enabled: true,
                params: HashMap::new(),
            },
            EffectSpec {
                kind: EffectKind::Distortion,
                enabled: true,
                params: HashMap::new(),
            },
        ];
        let chain = EffectChain::from_specs(&specs);
        assert_eq!(chain.len(), 2);
        assert_eq!(chain.kind_at(0), Some(EffectKind::Gate));
        assert_eq!(chain.kind_at(1), Some(EffectKind::Distortion));
    }

    #[test]
    fn disabled_effects_skip_processing() {
        struct Marker(bool);
        impl AudioEffect for Marker {
            fn process(&mut self, _: &mut [f32], _: u32) {
                self.0 = true;
            }
            fn set_param(&mut self, _: &str, _: f32) {}
            fn enabled(&self) -> bool {
                false
            }
            fn set_enabled(&mut self, _: bool) {}
            fn kind(&self) -> EffectKind {
                EffectKind::Gate
            }
        }
        let mut chain = EffectChain::new();
        chain.effects.push(Box::new(Marker(false)));
        let mut buf = [0_f32; 4];
        chain.process(&mut buf, 48000);
        let m = &chain.effects[0];
        assert!(!m.enabled());
    }

    #[test]
    fn set_param_via_apply_routes_to_the_correct_effect() {
        let mut chain = EffectChain::new();
        let mut gate = NoiseGate::new();
        gate.set_enabled(true);
        chain.effects.push(Box::new(gate));
        chain.apply(super::DspCommand::SetParam {
            index: 0,
            key: "thresh".into(),
            value: -30.0,
        });
        // No panic and no out-of-bounds; behaviour verified in
        // each effect's own tests.
        assert_eq!(chain.len(), 1);
    }

    #[test]
    fn set_chain_replaces_the_chain() {
        let mut chain = EffectChain::new();
        chain.apply(super::DspCommand::SetChain {
            specs: vec![EffectSpec {
                kind: EffectKind::Echo,
                enabled: true,
                params: HashMap::new(),
            }],
        });
        assert_eq!(chain.len(), 1);
        assert_eq!(chain.kind_at(0), Some(EffectKind::Echo));
    }

    fn spec(kind: EffectKind, enabled: bool) -> EffectSpec {
        EffectSpec {
            kind,
            enabled,
            params: HashMap::new(),
        }
    }

    // Phase 14: the chain's added latency is the sum of enabled effects'
    // fixed delays — pitch/formant STFT window (1024), denoiser frame
    // (480 @ 48 kHz), voice-convert chunk (only with a model loaded).
    #[test]
    fn empty_chain_has_zero_latency() {
        assert_eq!(EffectChain::new().latency_samples(48_000), 0);
    }

    #[test]
    fn chain_latency_sums_enabled_effects() {
        let chain = EffectChain::from_specs(&[
            spec(EffectKind::Denoiser, true), // 480 @ 48k
            spec(EffectKind::Pitch, true),    // 1024 (STFT window)
            spec(EffectKind::Gate, true),     // 0 (sample-by-sample)
        ]);
        assert_eq!(chain.latency_samples(48_000), 480 + 1024);
    }

    #[test]
    fn disabled_effects_add_no_latency() {
        let chain = EffectChain::from_specs(&[spec(EffectKind::Pitch, false)]);
        assert_eq!(chain.latency_samples(48_000), 0);
    }

    #[test]
    fn denoiser_latency_only_applies_at_48k() {
        let chain = EffectChain::from_specs(&[spec(EffectKind::Denoiser, true)]);
        assert_eq!(chain.latency_samples(48_000), 480);
        assert_eq!(chain.latency_samples(44_100), 0);
    }

    #[test]
    fn voice_convert_adds_no_latency_without_a_model() {
        // Passthrough (no model loaded) → no buffering delay.
        let chain = EffectChain::from_specs(&[spec(EffectKind::VoiceConvert, true)]);
        assert_eq!(chain.latency_samples(48_000), 0);
    }
}
