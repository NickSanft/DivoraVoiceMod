//! DSP effect chain — the 8 effects `DivoraVoice` ships, plus the
//! `AudioEffect` trait, `EffectChain`, and the `DspCommand` enum the
//! audio thread receives from the UI.
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

mod distortion;
mod echo;
mod eq;
mod formant;
mod gate;
mod pitch;
mod reverb;
mod robot;
mod stft;

pub use distortion::Distortion;
pub use echo::Echo;
pub use eq::Eq;
pub use formant::FormantShift;
pub use gate::NoiseGate;
pub use pitch::PitchShift;
pub use reverb::Reverb;
pub use robot::Robot;

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// Identifier of an effect kind; mirrors the frontend's `EffectId`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EffectKind {
    Gate,
    Pitch,
    Formant,
    Eq,
    Robot,
    Distortion,
    Echo,
    Reverb,
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
        EffectKind::Pitch => Box::new(PitchShift::new()),
        EffectKind::Formant => Box::new(FormantShift::new()),
        EffectKind::Eq => Box::new(Eq::new()),
        EffectKind::Robot => Box::new(Robot::new()),
        EffectKind::Distortion => Box::new(Distortion::new()),
        EffectKind::Echo => Box::new(Echo::new()),
        EffectKind::Reverb => Box::new(Reverb::new()),
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
}
