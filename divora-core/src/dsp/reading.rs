//! Voice reading — measured acoustic properties of a signal.
//!
//! This reports what a signal *is*: how loud, how high, how much the pitch
//! moves, how fast, how bright. It is **not** emotion recognition, and the
//! distinction is load-bearing rather than cosmetic.
//!
//! Why it can't be. The axis people mean by "what emotion is this" —
//! pleasant versus unpleasant — is carried by the *words*, not the voice.
//! Re-synthesise emotional speech with flat, neutral delivery and a model's
//! valence score largely survives; keep the delivery and remove the words and
//! its arousal score falls to nothing. Voice alone measures intensity. It
//! does not know whether you are delighted or furious, and this app makes
//! that worse: the chain pitch-shifts, bitcrushes and ring-modulates, and a
//! published formant shift alone costs a four-class emotion model 18 points.
//!
//! Why it must not pretend to be. EU AI Act Recital 18 excludes "the mere
//! detection of readily apparent expressions ... or characteristics of a
//! person's voice, such as a raised voice or whispering" from the definition
//! of an emotion recognition system — *unless* used to infer emotions. The
//! word on screen is the whole difference, so [`Descriptor`] is a closed
//! vocabulary describing a **signal**, never a person, and a test enforces it.
//!
//! [`crate::dsp::reactive`] made the same call for modulation in v1.46.0.
//!
//! Analysis runs on a worker thread fed by a ring, never in the audio
//! callback: pitch tracking is far too expensive for the RT path.
//!
//! ### Where the parts live
//!
//! This file is the contract — the words, the states and the shape of a
//! reading. [`analyzer`] turns audio into one; [`pitch`], [`spectrum`],
//! [`baseline`] and [`describe`] are the measurements and the comparison
//! behind it. The lock-free taps and the worker that drains them live in
//! `crate::audio::engine`, beside the callback that fills them.

mod analyzer;
#[cfg(test)]
mod analyzer_tests;
mod baseline;
mod describe;
mod pitch;
mod spectrum;

/// How often a reading is produced, seconds.
pub use analyzer::EMIT_S as READING_EMIT_S;
/// The rolling analysis window, seconds.
pub use analyzer::WINDOW_S as READING_WINDOW_S;
pub use analyzer::{Analyzer, InputFacts};
pub use pitch::{F0_MAX_HZ, F0_MIN_HZ};

use serde::{Deserialize, Serialize};

/// What the input is doing. A reading is only meaningful while someone is
/// actually speaking; the other states exist so the UI can say why it isn't
/// showing one instead of quietly decaying toward "quiet, flat and narrow",
/// which would read as a verdict on the speaker several times a minute.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ReadingState {
    /// The audio engine isn't running.
    Stopped,
    /// Input is digital silence — muted at the device or by push-to-modulate.
    Muted,
    /// Signal present but below the speech floor: room tone, breathing.
    Quiet,
    /// Voiced speech in the window. Only here are the metrics live.
    Speaking,
}

/// Acoustic properties over one analysis window.
///
/// Every field is a measurement of the signal. None is an inference about
/// the speaker.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Metrics {
    /// Level of the window, dBFS.
    pub energy_dbfs: f32,
    /// How much the level moves within the window, in dB (p90 − p10). Low is
    /// a monotone delivery, high is an animated one.
    pub energy_range_db: f32,
    /// Median pitch of the voiced frames, Hz. 0 when nothing was voiced.
    pub f0_hz: f32,
    /// Pitch spread across voiced frames, in semitones (p90 − p10).
    pub f0_range_st: f32,
    /// Share of frames that were voiced, 0..1 — speech versus pauses.
    pub voiced_ratio: f32,
    /// Voiced onsets per second: a proxy for pace, not a syllable count.
    pub pace_ops: f32,
    /// Spectral centroid, Hz — the "brightness" of the signal.
    pub brightness_hz: f32,
}

impl Metrics {
    /// A silent window: everything zero.
    #[must_use]
    pub const fn silent() -> Self {
        Self {
            energy_dbfs: f32::NEG_INFINITY,
            energy_range_db: 0.0,
            f0_hz: 0.0,
            f0_range_st: 0.0,
            voiced_ratio: 0.0,
            pace_ops: 0.0,
            brightness_hz: 0.0,
        }
    }
}

/// Declares the whole shown vocabulary in one place.
///
/// The enum, the word each variant prints, and the list everything is audited
/// against are all generated from this single list, so a variant cannot exist
/// without a word and cannot exist outside the audit. The earlier shape — an
/// enum, a `match` for the words, and a hand-written `ALL` — let a variant be
/// added that skipped the audit entirely, which review demonstrated.
macro_rules! vocabulary {
    ($( $(#[$doc:meta])* $variant:ident => $word:literal ),+ $(,)?) => {
        /// One word from the closed vocabulary the UI is allowed to show.
        ///
        /// Every variant describes the **signal**. A variant that names a
        /// feeling, or that only makes sense as a sentence about a person,
        /// turns this into something it deliberately is not — see the module
        /// docs and `the_vocabulary_is_exactly_this_list`.
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
        #[serde(rename_all = "lowercase")]
        pub enum Descriptor {
            $( $(#[$doc])* $variant, )+
        }

        impl Descriptor {
            /// Every descriptor. Generated, so nothing can be missing from it.
            pub const ALL: &'static [Self] = &[ $(Self::$variant,)+ ];

            /// The word shown in the UI.
            #[must_use]
            pub const fn word(self) -> &'static str {
                match self { $(Self::$variant => $word,)+ }
            }
        }
    };
}

vocabulary! {
    /// Level well under this speaker's own baseline.
    Quiet => "quiet",
    /// Level well over it.
    Loud => "loud",
    /// Little level movement across the window.
    Flat => "flat",
    /// A lot of it.
    Dynamic => "dynamic",
    /// Pitch barely moves.
    Narrow => "narrow range",
    /// Pitch moves a lot.
    Wide => "wide range",
    /// Low median pitch for this speaker.
    Low => "low",
    /// High median pitch for this speaker.
    High => "high",
    /// Few onsets per second.
    Slow => "slow",
    /// Many.
    Fast => "fast",
    /// Little high-frequency energy.
    Dark => "dark",
    /// Plenty.
    Bright => "bright",
    /// Nothing stands out from the baseline.
    Steady => "steady",
}

impl Descriptor {
    /// How many descriptors exist.
    pub const COUNT: usize = Self::ALL.len();

    /// This descriptor's position in [`Descriptor::ALL`].
    #[must_use]
    pub fn index(self) -> usize {
        Self::ALL.iter().position(|d| *d == self).unwrap_or(0)
    }

    /// The descriptor at position `i`, if there is one.
    #[must_use]
    pub fn from_index(i: usize) -> Option<Self> {
        Self::ALL.get(i).copied()
    }
}

/// Most descriptors shown at once. More than three stops reading as a
/// description and starts reading as a verdict.
pub const MAX_DESCRIPTORS: usize = 3;

/// A complete reading: the speaker's own voice, the same voice after the
/// effect chain, and a short description of the dry signal.
// Four flags, each answering a question the numbers alone cannot: is this
// reading live, is the wet side bypassed, has the wet side caught up, and is
// there a baseline to compare against. Collapsing them into an enum would
// force the UI to handle combinations that can and do occur together.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VoiceReading {
    pub state: ReadingState,
    /// The mic, before any effect. Frozen from the last speaking window when
    /// `held` is set.
    pub dry: Metrics,
    /// The same audio after the chain — what the call actually hears. Frozen
    /// with `dry`.
    pub wet: Metrics,
    /// Describes `dry`, most salient first. Empty unless
    /// [`ReadingState::Speaking`] — see `held_descriptors` for the frozen one.
    pub descriptors: Vec<Descriptor>,
    /// True once enough speech has been heard for the baseline to mean
    /// anything; until then the UI should say it is still listening rather
    /// than show a comparison against too little data.
    pub calibrated: bool,
    /// `dry` and `wet` are the last speaking window's, held unchanged because
    /// the input is not currently speech.
    ///
    /// They do not decay. Most of a session is not speech, and a panel that
    /// slid toward "quiet, flat, narrow" through every pause would hand down a
    /// verdict on the speaker several times a minute. Show them greyed; the
    /// `state` says why they are standing still.
    pub held: bool,
    /// The frozen phrase that goes with held `dry`/`wet`, so the UI can grey
    /// the whole reading instead of blinking the words out on every pause.
    /// Empty while [`ReadingState::Speaking`], where `descriptors` is live.
    pub held_descriptors: Vec<Descriptor>,
    /// The chain was bypassed over this window, so `wet` is `dry`.
    ///
    /// Push-to-modulate with the key up is exactly this: the dry tap has
    /// speech on it while the wet tap is a passthrough. Without this flag the
    /// panel would read that as the preset having stopped working.
    pub wet_bypassed: bool,
    /// `wet` covers a full window of the *current* chain.
    ///
    /// False for a moment after a preset switch. A +5 semitone preset moves
    /// the wet pitch by +5 with no change at all in the speaker — that is the
    /// point of the wet side, and it is why the panel has to label it as
    /// after-effects rather than as something the speaker did.
    pub wet_settled: bool,
}

impl VoiceReading {
    /// A reading with nothing measured yet.
    #[must_use]
    pub fn idle(state: ReadingState) -> Self {
        Self {
            state,
            dry: Metrics::silent(),
            wet: Metrics::silent(),
            descriptors: Vec::new(),
            calibrated: false,
            held: false,
            held_descriptors: Vec::new(),
            wet_bypassed: false,
            wet_settled: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The honesty guarantee, made mechanical.
    ///
    /// Recital 18's own illustrative list of emotions is the banned set: the
    /// moment one of those words can appear, the feature stops being a meter
    /// and becomes an emotion recognition system — technically indefensible
    /// on a modulated voice, and a regulated category. Second-person
    /// phrasing is banned for the same reason: "you sound ..." is a claim
    /// about a person, while "wide range" is a fact about a signal.
    #[test]
    fn vocabulary_describes_signals_never_people() {
        // A second belt, not the primary guard — that is
        // `the_vocabulary_is_exactly_this_list`. This one makes a careless
        // ADDITION obvious where it is written. Review showed a denylist
        // alone is not enough: it only catches what it was taught, and
        // "bored" walked straight through the first version of it.
        const EMOTIONS: &[&str] = &[
            "happy",
            "happiness",
            "joy",
            "sad",
            "sadness",
            "angry",
            "anger",
            "surprise",
            "surprised",
            "disgust",
            "disgusted",
            "embarrass",
            "excited",
            "excitement",
            "shame",
            "ashamed",
            "contempt",
            "satisfaction",
            "satisfied",
            "amused",
            "amusement",
            "fear",
            "afraid",
            "anxious",
            "stress",
            "mood",
            "emotion",
            "feeling",
            "upset",
            "calm",
            "tense",
            "confident",
            "nervous",
            "anxiety",
            "bored",
            "boredom",
            "frustrat",
            "irritat",
            "annoy",
            "depress",
            "worried",
            "tired",
            "enthusiast",
            "aggressive",
            "hostile",
            "agitated",
            "timid",
            "hesitant",
            "uncertain",
            "relaxed",
            "cheerful",
            "gloomy",
            "rage",
            "furious",
            "sarcas",
            "sincere",
        ];
        for d in Descriptor::ALL {
            let word = d.word().to_ascii_lowercase();
            assert!(!word.is_empty(), "{d:?} has no word");
            for banned in EMOTIONS {
                assert!(
                    !word.contains(banned),
                    "{d:?} says {word:?}, which names a feeling, not a sound"
                );
            }
            assert!(
                !word.contains("you") && !word.contains("sound like"),
                "{d:?} says {word:?}, which is a sentence about a person"
            );
        }
    }

    #[test]
    fn the_vocabulary_is_exactly_this_list() {
        // The primary guard, and the reason it is a list rather than a filter:
        // a denylist only catches words it was taught. Review changed one word
        // to "bored" — not an emotion by spelling, but plainly a claim about a
        // person — and every test stayed green. Pinning the exact set means
        // any change to a shown word has to edit this line, where a reviewer
        // sees it.
        const WORDS: [&str; 13] = [
            "quiet",
            "loud",
            "flat",
            "dynamic",
            "narrow range",
            "wide range",
            "low",
            "high",
            "slow",
            "fast",
            "dark",
            "bright",
            "steady",
        ];
        assert_eq!(Descriptor::COUNT, WORDS.len());
        let shown: Vec<&str> = Descriptor::ALL.iter().map(|d| d.word()).collect();
        assert_eq!(shown, WORDS, "the shown vocabulary changed");

        // ALL is generated from the same list the enum is, so this cannot be
        // partial — but the round trip still pins the ordering the wire and
        // the UI rely on.
        let mut seen = std::collections::HashSet::new();
        for (i, d) in Descriptor::ALL.iter().enumerate() {
            assert_eq!(d.index(), i, "{d:?} is not at its own position");
            assert_eq!(Descriptor::from_index(i), Some(*d));
            assert!(seen.insert(*d), "{d:?} appears twice");
        }
        assert_eq!(Descriptor::from_index(Descriptor::COUNT), None);
        assert_eq!(seen.len(), Descriptor::COUNT);
    }

    #[test]
    fn a_silent_window_measures_nothing() {
        let m = Metrics::silent();
        assert!(m.energy_dbfs.is_infinite() && m.energy_dbfs.is_sign_negative());
        assert!(m.f0_hz.abs() < f32::EPSILON);
        assert!(m.voiced_ratio.abs() < f32::EPSILON);
    }

    #[test]
    fn an_idle_reading_makes_no_claims() {
        for state in [
            ReadingState::Stopped,
            ReadingState::Muted,
            ReadingState::Quiet,
        ] {
            let r = VoiceReading::idle(state);
            assert_eq!(r.state, state);
            assert!(r.descriptors.is_empty(), "{state:?} described something");
            assert!(!r.calibrated);
        }
    }
}
