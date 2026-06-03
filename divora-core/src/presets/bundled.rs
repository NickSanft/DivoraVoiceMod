//! Compile-time-embedded bundled presets. Editing a bundled preset
//! file under `bundled/` and rebuilding ships the new defaults.

use super::Preset;

/// One bundled preset entry: id + the raw JSON source baked in.
struct BundledSource {
    id: &'static str,
    json: &'static str,
}

const BUNDLED: &[BundledSource] = &[
    BundledSource {
        id: "hollow-king",
        json: include_str!("bundled/hollow-king.json"),
    },
    BundledSource {
        id: "static-wraith",
        json: include_str!("bundled/static-wraith.json"),
    },
    BundledSource {
        id: "velvet-demon",
        json: include_str!("bundled/velvet-demon.json"),
    },
    BundledSource {
        id: "choir-of-ash",
        json: include_str!("bundled/choir-of-ash.json"),
    },
    BundledSource {
        id: "the-oracle",
        json: include_str!("bundled/the-oracle.json"),
    },
    // v1.4.0: the choir family — adjustable-Harmonizer chord voices.
    BundledSource {
        id: "seraph",
        json: include_str!("bundled/seraph.json"),
    },
    BundledSource {
        id: "dirge",
        json: include_str!("bundled/dirge.json"),
    },
    BundledSource {
        id: "the-swarm",
        json: include_str!("bundled/the-swarm.json"),
    },
    BundledSource {
        id: "the-possessed",
        json: include_str!("bundled/the-possessed.json"),
    },
    // v1.5.0: range fillers + utility voices.
    BundledSource {
        id: "leviathan",
        json: include_str!("bundled/leviathan.json"),
    },
    BundledSource {
        id: "the-imp",
        json: include_str!("bundled/the-imp.json"),
    },
    BundledSource {
        id: "dispatch",
        json: include_str!("bundled/dispatch.json"),
    },
    BundledSource {
        id: "corrupted",
        json: include_str!("bundled/corrupted.json"),
    },
    BundledSource {
        id: "whisper-wraith",
        json: include_str!("bundled/whisper-wraith.json"),
    },
    BundledSource {
        id: "clean",
        json: include_str!("bundled/clean.json"),
    },
    BundledSource {
        id: "deep-narrator-ai",
        json: include_str!("bundled/deep-narrator-ai.json"),
    },
];

/// Parse every bundled preset. Panics if any JSON is malformed — that's
/// a build-time bug, not a runtime one.
#[must_use]
pub fn bundled_presets() -> Vec<Preset> {
    BUNDLED
        .iter()
        .map(|b| {
            serde_json::from_str::<Preset>(b.json)
                .unwrap_or_else(|e| panic!("bundled preset {} failed to parse: {e}", b.id))
        })
        .collect()
}

/// IDs of every bundled preset. Useful for tests that don't want to
/// pay the JSON-parse cost.
#[must_use]
pub fn bundled_preset_ids() -> Vec<&'static str> {
    BUNDLED.iter().map(|b| b.id).collect()
}

#[cfg(test)]
mod tests {
    use super::{bundled_preset_ids, bundled_presets};
    use crate::presets::{Preset, PresetTag};

    #[test]
    fn every_bundled_preset_parses() {
        let presets = bundled_presets();
        assert_eq!(presets.len(), bundled_preset_ids().len());
        for p in &presets {
            assert!(!p.id.is_empty());
            assert!(!p.name.is_empty());
            assert!(matches!(p.tag, PresetTag::Bundled));
        }
    }

    #[test]
    fn ids_are_unique() {
        let presets = bundled_presets();
        let mut ids: Vec<_> = presets.iter().map(|p| p.id.clone()).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), presets.len());
    }

    #[test]
    fn hollow_king_is_present_with_five_effects() {
        let presets = bundled_presets();
        let hk = presets.iter().find(|p| p.id == "hollow-king").unwrap();
        assert_eq!(hk.name, "Hollow King");
        assert_eq!(hk.chain.len(), 5);
        assert_eq!(hk.chain[0].id, "gate");
    }

    // v1.1.0 (The Coven): the Oracle is the calm/resonant cast member —
    // natural pitch (no shift, unlike the rest of the cast), a resonant
    // mid bump, and spacious reverb. Lock that character in.
    #[test]
    fn the_oracle_is_calm_and_resonant() {
        let presets = bundled_presets();
        let o = presets
            .iter()
            .find(|p| p.id == "the-oracle")
            .expect("the-oracle must be bundled");
        assert_eq!(o.name, "The Oracle");
        assert!(
            o.chain.iter().all(|e| e.id != "pitch"),
            "Oracle keeps natural pitch (no pitch effect)"
        );
        assert!(
            o.chain.iter().any(|e| e.id == "reverb" && e.enabled),
            "Oracle is spacious (reverb enabled)"
        );
        let eq = o
            .chain
            .iter()
            .find(|e| e.id == "eq")
            .expect("Oracle shapes tone with EQ");
        assert!(
            eq.vals.get("mid").copied().unwrap_or(0.0) > 0.0,
            "Oracle has a resonant mid bump"
        );
    }

    // v1.2.1 (The Coven): Choir of Ash gets its chord from the
    // harmonizer (a diminished stack), not a single (high) voice. Lock
    // it in so a future edit can't quietly drop it back to one voice.
    #[test]
    fn choir_of_ash_layers_a_harmonizer() {
        let presets = bundled_presets();
        let choir = presets
            .iter()
            .find(|p| p.id == "choir-of-ash")
            .expect("choir-of-ash must be bundled");
        assert!(
            choir
                .chain
                .iter()
                .any(|e| e.id == "harmonizer" && e.enabled),
            "Choir of Ash should layer an enabled harmonizer for the chord"
        );
    }

    // v1.4.0: the choir family is built on the adjustable Harmonizer —
    // each member layers a harmonizer with its own chord intervals. Lock
    // the identities so a future edit can't flatten them.
    #[test]
    fn choir_family_intervals_are_distinct_chords() {
        let presets = bundled_presets();
        let find = |id: &str| presets.iter().find(|p| p.id == id).unwrap().clone();
        let harm = |p: &Preset| {
            p.chain
                .iter()
                .find(|e| e.id == "harmonizer" && e.enabled)
                .unwrap_or_else(|| panic!("{} must layer a harmonizer", p.id))
                .clone()
        };
        // Seraph = major triad (v1 = +4, major third).
        assert_eq!(harm(&find("seraph")).vals.get("v1").copied(), Some(4.0));
        // Dirge = minor third + a low octave toll (v1 = +3, v3 = -12).
        let d = harm(&find("dirge"));
        assert_eq!(d.vals.get("v1").copied(), Some(3.0));
        assert_eq!(d.vals.get("v3").copied(), Some(-12.0));
        // The Possessed = octave-down doubling (v1 = -12).
        assert_eq!(
            harm(&find("the-possessed")).vals.get("v1").copied(),
            Some(-12.0)
        );
        // The Swarm = a tight dissonant cluster (v1 = +1).
        assert_eq!(harm(&find("the-swarm")).vals.get("v1").copied(), Some(1.0));
    }

    // v1.5.0: range fillers + utility voices keep their defining
    // character (Leviathan deepest, Imp pitched up, Dispatch a dry
    // bandlimited comms voice). Lock it so edits can't blur them.
    #[test]
    fn voice_pack_v15_keeps_its_character() {
        let presets = bundled_presets();
        let p = |id: &str| presets.iter().find(|x| x.id == id).cloned();
        let pitch = |id: &str| {
            p(id)
                .and_then(|x| {
                    x.chain
                        .iter()
                        .find(|e| e.id == "pitch")
                        .and_then(|e| e.vals.get("shift").copied())
                })
                .unwrap_or(0.0)
        };
        // Leviathan is the deepest in the cast — below Hollow King.
        assert!(
            pitch("leviathan") < pitch("hollow-king"),
            "Leviathan must be deeper than Hollow King"
        );
        // The Imp pitches UP.
        assert!(pitch("the-imp") > 0.0, "The Imp pitches up");
        // Dispatch is a dry comms voice — no reverb, with a mid-forward EQ.
        let dispatch = p("dispatch").expect("dispatch bundled");
        assert!(
            dispatch.chain.iter().all(|e| e.id != "reverb"),
            "Dispatch stays dry (no reverb)"
        );
        // The atmospheric utility voices are present.
        assert!(p("corrupted").is_some());
        assert!(p("whisper-wraith").is_some());
    }

    // v0.12.2: the Deep Narrator preset must actually *sound* deep via
    // DSP — it can't lean on the (passthrough-until-a-model-is-installed)
    // voice_convert effect for its audible character. Lock in the
    // pitch-down + formant-down that do the work, so a future edit can't
    // silently flatten it back to a clean voice.
    #[test]
    fn deep_narrator_lowers_the_voice_via_dsp() {
        let presets = bundled_presets();
        let dn = presets.iter().find(|p| p.id == "deep-narrator-ai").unwrap();
        assert_eq!(dn.name, "Deep Narrator");

        let pitch = dn
            .chain
            .iter()
            .find(|e| e.id == "pitch")
            .expect("deep narrator must include a pitch effect");
        assert!(pitch.enabled, "pitch must be enabled");
        let shift = pitch.vals.get("shift").copied().unwrap_or(0.0);
        assert!(shift < 0.0, "pitch must shift DOWN (got {shift})");

        let formant = dn
            .chain
            .iter()
            .find(|e| e.id == "formant")
            .expect("deep narrator must include a formant effect");
        assert!(formant.enabled, "formant must be enabled");
        let fshift = formant.vals.get("shift").copied().unwrap_or(0.0);
        assert!(fshift < 0.0, "formant must shift DOWN (got {fshift})");

        // The bring-your-own-AI slot is present + enabled so a selected
        // voice activates without re-editing the chain.
        assert!(
            dn.chain
                .iter()
                .any(|e| e.id == "voice_convert" && e.enabled),
            "deep narrator keeps an enabled voice_convert slot for BYO models"
        );
    }
}
