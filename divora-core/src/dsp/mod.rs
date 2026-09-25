//! DSP effect chain — the effects `DivoraVoice` ships (see `EffectKind`
//! below for the full set), plus the `AudioEffect` trait, `EffectChain`,
//! and the `DspCommand` enum the audio thread receives from the UI.
//!
//! ### Audio-thread ownership model
//!
//! `EffectChain` is owned exclusively by the audio output callback. The
//! UI sends [`DspCommand`]s, which `AudioEngine::send_dsp` lowers into
//! [`DspEdit`]s on the calling thread; the callback drains those from a
//! bounded channel at the top of each buffer, applies any structural or
//! parameter changes, and then runs `process` on the mono buffer.
//!
//! The two enums exist because the callback must not allocate, free,
//! lock or spawn. Building a chain does the first — every effect is
//! boxed and the constructors allocate their STFT rings, comb buffers
//! and harmonizer state — and *replacing* one also frees the chain it
//! displaces. So a `SetChain` is built into a whole [`EffectChain`] on
//! the control thread and arrives as [`DspEdit::ReplaceChain`], and
//! [`EffectChain::apply`] hands what it displaced back to the caller as
//! [`Displaced`] instead of dropping it: the engine passes that to a
//! graveyard thread, which does the freeing.
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

mod bitcrush;
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
mod reactive;
mod reading;
mod reverb;
mod robot;
mod stft;
mod tremolo;
mod vintage_noise;
mod voice_convert;
mod warble;

pub use bitcrush::Bitcrush;
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
pub use reactive::{
    mod_target_range, ModRoute, ReactiveConfig, ReactiveModulator, ReactiveRouteSpec,
    ReactiveSource, ResolvedReactive, DEFAULT_CEIL_DB, DEFAULT_FLOOR_DB,
};
pub use reading::{
    Analyzer, Descriptor, InputFacts, Metrics, ReadingState, VoiceReading, F0_MAX_HZ, F0_MIN_HZ,
    MAX_DESCRIPTORS, READING_EMIT_S, READING_WINDOW_S,
};
pub use reverb::Reverb;
pub use robot::Robot;
pub use tremolo::Tremolo;
pub use vintage_noise::VintageNoise;
pub use voice_convert::{onnx_runtime_available, VoiceConverter, VoiceModel, MODEL_RESOURCE_KEY};
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
    /// v1.48.0: bitcrusher — amplitude quantisation + sample-and-hold
    /// decimation. Distinct from `Distortion`'s smooth `tanh` waveshaper:
    /// decimation folds high frequencies down as INHARMONIC images that move
    /// downward as the speaker's pitch rises, which is the "8-bit / broken
    /// machine" cue a waveshaper cannot produce. Zero latency.
    Bitcrush,
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

    /// Install a resource prepared OFF the audio thread — e.g. the ONNX
    /// model the `VoiceConvert` effect should use, whose session is
    /// already loading on a control thread. `f32` params go through
    /// `set_param`; this carries the things that aren't numbers.
    ///
    /// Returns whatever the change displaced — an old ONNX session, the
    /// strings naming it — so the audio thread can hand that to the
    /// graveyard rather than free it. The default implementation has no
    /// resource to install, so it hands the whole thing straight back;
    /// only effects that actually have one override this.
    fn install(&mut self, resource: Prepared) -> Option<Displaced> {
        Some(resource.displaced())
    }

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

/// A resource prepared off the audio thread, ready for an effect to install
/// with a handful of moves.
///
/// Everything slow or allocating about the change — deriving the voice name,
/// owning the path, starting the ONNX session load on its own thread — has
/// already happened on the control thread that built this.
pub enum Prepared {
    /// The voice-conversion model `VoiceConvert` should use. A [`VoiceModel`]
    /// naming no path clears the effect back to passthrough.
    VoiceModel(VoiceModel),
}

impl Prepared {
    /// Turn a resource that was never installed into something for the
    /// graveyard — the effect it named is gone (a stale index from a chain
    /// that has since moved on), and the audio thread must not free it.
    #[must_use]
    pub fn displaced(self) -> Displaced {
        match self {
            Self::VoiceModel(model) => Displaced::VoiceModel(model),
        }
    }
}

/// Something the audio callback swapped OUT of the live chain and must not
/// free itself.
///
/// Dropping a chain frees every boxed effect and everything inside it — STFT
/// rings, reverb combs, harmonizer state, an ONNX session — which is a
/// deallocation storm no callback can afford. So the callback hands these to
/// the engine's graveyard: a bounded ring that a dedicated thread drains and
/// drops.
pub enum Displaced {
    /// A whole chain: the one a `ReplaceChain` pushed out, or the effects a
    /// `Clear` emptied.
    Chain(EffectChain),
    /// A voice-conversion model — its ONNX session, any load still in flight,
    /// and the strings naming it.
    VoiceModel(VoiceModel),
}

/// A chain edit in the form the audio callback can apply.
///
/// [`DspCommand`] is what the UI sends; a `DspEdit` is the same intent with
/// the allocating, blocking, thread-spawning parts already done. They are two
/// types rather than one enum with an extra variant so the compiler enforces
/// the distinction: the callback's channel carries `DspEdit`, so a chain that
/// has not been built yet cannot reach it.
pub enum DspEdit {
    /// Replace the live chain with one built off the audio thread.
    ///
    /// By value, not boxed, on purpose: installing a `Box<EffectChain>` means
    /// moving out of the box and then FREEING the box, in the callback, which
    /// is the class of thing this type exists to avoid. A chain is one `Vec`.
    ReplaceChain(EffectChain),
    SetParam {
        index: usize,
        key: String,
        value: f32,
    },
    SetEnabled {
        index: usize,
        enabled: bool,
    },
    /// Install a prepared resource on the effect at `index`.
    SetResource {
        index: usize,
        resource: Prepared,
    },
    /// Empty the chain. Like `ReplaceChain`, the effects leave through
    /// [`EffectChain::apply`]'s return value — `Vec::clear` would drop them
    /// on the spot.
    Clear,
}

impl DspEdit {
    /// Lower a UI command into a realtime-safe edit.
    ///
    /// **Call this on a control thread.** It is where a chain gets built and
    /// a model load gets started, precisely so the audio callback does
    /// neither; `AudioEngine::send_dsp` is the one caller that matters.
    ///
    /// `None` when the command asks for something nothing here can prepare
    /// (a resource key no effect claims), which is how a newer or older
    /// frontend fails safe instead of wedging the engine.
    #[must_use]
    pub fn prepare(cmd: DspCommand) -> Option<Self> {
        Some(match cmd {
            // The build — every effect boxed, every constructor allocating —
            // happens HERE, on the caller's thread.
            DspCommand::SetChain { specs } => Self::ReplaceChain(EffectChain::from_specs(&specs)),
            DspCommand::SetParam { index, key, value } => Self::SetParam { index, key, value },
            DspCommand::SetEnabled { index, enabled } => Self::SetEnabled { index, enabled },
            // Same thing for a voice model: `VoiceModel::start` spawns the
            // loader thread, so the callback only moves the result in.
            DspCommand::SetResource { index, key, value } if key == MODEL_RESOURCE_KEY => {
                Self::SetResource {
                    index,
                    resource: Prepared::VoiceModel(VoiceModel::start(value)),
                }
            }
            DspCommand::SetResource { .. } => return None,
            DspCommand::Clear => Self::Clear,
        })
    }
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

    /// Build a chain from a list of specs.
    ///
    /// Allocates, once per effect plus whatever each constructor reserves, so
    /// this runs on a control thread — see [`DspEdit::prepare`]. Never in the
    /// audio callback.
    #[must_use]
    pub fn from_specs(specs: &[EffectSpec]) -> Self {
        let mut chain = Self::new();
        for spec in specs {
            chain.effects.push(build_effect(spec));
        }
        chain
    }

    /// Apply a single edit, handing back anything it displaced.
    ///
    /// Called from the audio callback when draining the edit channel, so
    /// nothing in here allocates, frees, locks or blocks. The return value is
    /// the whole point: this cannot see the engine's channels, so a displaced
    /// chain leaves through the caller, which routes it to the graveyard
    /// thread (see [`Displaced`]).
    #[must_use = "a displaced chain must be freed off the audio thread"]
    pub fn apply(&mut self, edit: DspEdit) -> Option<Displaced> {
        match edit {
            DspEdit::ReplaceChain(chain) => Some(Displaced::Chain(std::mem::replace(self, chain))),
            DspEdit::SetParam { index, key, value } => {
                if let Some(effect) = self.effects.get_mut(index) {
                    effect.set_param(&key, value);
                }
                None
            }
            DspEdit::SetEnabled { index, enabled } => {
                if let Some(effect) = self.effects.get_mut(index) {
                    effect.set_enabled(enabled);
                }
                None
            }
            DspEdit::SetResource { index, resource } => match self.effects.get_mut(index) {
                Some(effect) => effect.install(resource),
                // Nothing at that index any more: hand the prepared resource
                // back rather than dropping it here.
                None => Some(resource.displaced()),
            },
            // An empty chain allocates nothing, so a swap is the realtime-safe
            // way to clear: `Vec::clear` would drop every boxed effect — and
            // an ONNX session with them — inside the callback.
            DspEdit::Clear => Some(Displaced::Chain(std::mem::take(self))),
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

    /// Index of the `nth` (0-based) occurrence of `kind`, if present.
    ///
    /// Used by reactive modulation, which addresses its targets by kind +
    /// occurrence rather than raw index: `SetChain` rebuilds the whole chain
    /// on every preset switch, and a drag-reorder shuffles positions, so a
    /// stored index would silently start pointing at a different effect.
    /// Resolving fresh each block is a scan of at most a couple of dozen
    /// entries and cannot go stale.
    #[must_use]
    pub fn index_of_kind(&self, kind: EffectKind, nth: u8) -> Option<usize> {
        self.effects
            .iter()
            .enumerate()
            .filter(|(_, e)| e.kind() == kind)
            .nth(nth as usize)
            .map(|(index, _)| index)
    }

    /// Set one parameter by index, without allocating.
    ///
    /// [`DspCommand::SetParam`] carries an owned `String`, so driving
    /// modulation through the command channel would allocate per message and
    /// free on the audio thread. This is the per-buffer path.
    pub fn set_param_at(&mut self, index: usize, key: &str, value: f32) {
        if let Some(effect) = self.effects.get_mut(index) {
            effect.set_param(key, value);
        }
    }

    /// Drop every effect, freeing them **on this thread**. Not for the audio
    /// callback: its way to empty the chain is [`DspEdit::Clear`], which hands
    /// the effects out instead of freeing them where it cannot afford to.
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
        EffectKind::Bitcrush => Box::new(Bitcrush::new()),
    };
    effect.set_enabled(spec.enabled);
    for (key, value) in &spec.params {
        effect.set_param(key, *value);
    }
    effect
}

/// Test-only, and shared with the audio engine's tests: an effect that
/// reports its own destruction.
///
/// It is how a test tells "freed right here" — which in production is the
/// audio callback — from "handed off to be freed where that is affordable".
#[cfg(test)]
pub(crate) mod witness {
    use super::{AudioEffect, EffectChain, EffectKind};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    pub(crate) struct DropWitness(Arc<AtomicBool>);

    impl Drop for DropWitness {
        fn drop(&mut self) {
            self.0.store(true, Ordering::SeqCst);
        }
    }

    impl AudioEffect for DropWitness {
        fn process(&mut self, _: &mut [f32], _: u32) {}
        fn set_param(&mut self, _: &str, _: f32) {}
        fn enabled(&self) -> bool {
            true
        }
        fn set_enabled(&mut self, _: bool) {}
        fn kind(&self) -> EffectKind {
            EffectKind::Gate
        }
    }

    /// A one-effect chain, plus the flag that says when that chain was freed.
    pub(crate) fn witnessed_chain() -> (EffectChain, Arc<AtomicBool>) {
        let freed = Arc::new(AtomicBool::new(false));
        let mut chain = EffectChain::new();
        chain.effects.push(Box::new(DropWitness(freed.clone())));
        (chain, freed)
    }
}

#[cfg(test)]
mod tests {
    use super::witness::witnessed_chain;
    use super::{
        AudioEffect, Displaced, DspCommand, DspEdit, EffectChain, EffectKind, EffectSpec,
        NoiseGate, Prepared, VoiceModel,
    };
    use std::collections::HashMap;
    use std::sync::atomic::Ordering;

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
        let displaced = chain.apply(DspEdit::SetParam {
            index: 0,
            key: "thresh".into(),
            value: -30.0,
        });
        // No panic and no out-of-bounds; behaviour verified in
        // each effect's own tests.
        assert_eq!(chain.len(), 1);
        assert!(displaced.is_none(), "a param change displaces nothing");
    }

    #[test]
    fn replace_chain_swaps_in_the_prebuilt_chain() {
        let mut chain = EffectChain::new();
        let built = EffectChain::from_specs(&[spec(EffectKind::Echo, true)]);
        let _ = chain.apply(DspEdit::ReplaceChain(built));
        assert_eq!(chain.len(), 1);
        assert_eq!(chain.kind_at(0), Some(EffectKind::Echo));
    }

    /// The defect this whole seam exists for: replacing the chain used to free
    /// the old one on the audio thread. It must leave through the return value.
    #[test]
    fn replace_chain_hands_the_old_chain_back_instead_of_freeing_it() {
        let (mut chain, freed) = witnessed_chain();

        let displaced = chain
            .apply(DspEdit::ReplaceChain(EffectChain::from_specs(&[spec(
                EffectKind::Echo,
                true,
            )])))
            .expect("the displaced chain must come back");
        assert!(
            !freed.load(Ordering::SeqCst),
            "the old chain was freed inside apply — in production that is the audio callback"
        );
        assert_eq!(chain.kind_at(0), Some(EffectKind::Echo));

        // What the graveyard thread does, here done inline.
        drop(displaced);
        assert!(
            freed.load(Ordering::SeqCst),
            "the displaced chain was never freed at all"
        );
    }

    /// `Clear` has the same problem: `Vec::clear` drops every boxed effect.
    #[test]
    fn clear_hands_the_effects_back_instead_of_freeing_them() {
        let (mut chain, freed) = witnessed_chain();

        let displaced = chain
            .apply(DspEdit::Clear)
            .expect("a cleared chain's effects must come back");
        assert!(chain.is_empty(), "Clear must leave an empty chain");
        assert!(matches!(displaced, Displaced::Chain(_)));
        assert!(
            !freed.load(Ordering::SeqCst),
            "Clear freed the effects inside apply"
        );
        drop(displaced);
        assert!(freed.load(Ordering::SeqCst));
    }

    /// `prepare` is the control-thread half: the chain is fully built before
    /// anything reaches the audio thread, which is what makes the swap cheap.
    #[test]
    fn prepare_builds_the_chain_on_the_calling_thread() {
        let edit = DspEdit::prepare(DspCommand::SetChain {
            specs: vec![spec(EffectKind::Gate, true), spec(EffectKind::Reverb, true)],
        })
        .expect("a SetChain always prepares");
        match edit {
            DspEdit::ReplaceChain(chain) => {
                assert_eq!(chain.len(), 2);
                assert_eq!(chain.kind_at(1), Some(EffectKind::Reverb));
            }
            _ => panic!("a SetChain must arrive as a built chain"),
        }
    }

    #[test]
    fn prepare_starts_a_voice_model_load() {
        let edit = DspEdit::prepare(DspCommand::SetResource {
            index: 3,
            key: super::MODEL_RESOURCE_KEY.to_string(),
            value: Some("/voices/sage.onnx".into()),
        })
        .expect("a model resource always prepares");
        assert!(matches!(
            edit,
            DspEdit::SetResource {
                index: 3,
                resource: Prepared::VoiceModel(_)
            }
        ));
    }

    /// A key no effect claims produces no edit at all, rather than something
    /// the audio thread has to sort out.
    #[test]
    fn prepare_rejects_a_resource_key_nothing_claims() {
        assert!(DspEdit::prepare(DspCommand::SetResource {
            index: 0,
            key: "nonsense".into(),
            value: Some("/voices/other.onnx".into()),
        })
        .is_none());
    }

    /// A resource can name an index the chain no longer has (it was rebuilt
    /// under the command). It must come back out, not be freed here.
    #[test]
    fn a_resource_for_a_missing_index_comes_straight_back() {
        let mut chain = EffectChain::new();
        let displaced = chain.apply(DspEdit::SetResource {
            index: 7,
            resource: Prepared::VoiceModel(VoiceModel::start(None)),
        });
        assert!(matches!(displaced, Some(Displaced::VoiceModel(_))));
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
