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
    // v1.32.0: old-timey radio pair (researched + adversarially design-reviewed),
    // built on the new radio_bandpass + vintage_noise + asymmetric-warmth DSP.
    // "spirit-radio" = warm vintage broadcast (the voice inside the valve set);
    // "parlor-augur" = tinny across-the-room tabletop set. Both distinct from the
    // modern tactical "dispatch".
    BundledSource {
        id: "spirit-radio",
        json: include_str!("bundled/spirit-radio.json"),
    },
    BundledSource {
        id: "parlor-augur",
        json: include_str!("bundled/parlor-augur.json"),
    },
    // v1.34.0: Minecraft-villager voice (researched + adversarially reviewed).
    // Built on the new `tremolo` (the "hrm-hrm" wobble) + a two-`radio_bandpass`
    // nasal pole/notch pair (a murmur boost low, a cut at the ~1 kHz "m"
    // antiformant) enabled by the negative-gain band-pass.
    BundledSource {
        id: "villager",
        json: include_str!("bundled/villager.json"),
    },
    // v1.35.0: Minecraft-creeper voice (researched + adversarially reviewed).
    // Built on the new `breath` whisperizer — additive noise that SWELLS with
    // the voice envelope (the building fuse), the inverse of vintage_noise's
    // duck — plus a sibilant band boost, light rasp, and a slow simmer.
    BundledSource {
        id: "creeper",
        json: include_str!("bundled/creeper.json"),
    },
    // v1.36.0: Minecraft-zombie voice (researched + adversarially reviewed).
    // A SHALLOW pitch/formant drop (undead human, NOT a demon octave-drop),
    // an octave-down harmonizer ground into asymmetric distortion for the
    // gravel/sub, a slow tremolo moan + chorus wet-smear, kept dark but with
    // the chest body preserved (low HP). No new DSP — pure recipe.
    BundledSource {
        id: "zombie",
        json: include_str!("bundled/zombie.json"),
    },
    // v1.37.0: Minecraft-enderman voice (researched + adversarially reviewed).
    // An honest close-approximation, NOT literal reversed speech (which needs
    // buffering + latency): built on the new `warble` pitch-vibrato for the
    // otherworldly wobble, plus a low ring-mod garble, heavy chorus, and a big
    // void reverb. Pitched/formant down for the tall being.
    BundledSource {
        id: "enderman",
        json: include_str!("bundled/enderman.json"),
    },
    // v1.38.0: Minecraft-ghast voice (researched + adversarially reviewed).
    // The family's odd one out — pitched UP (a high, plaintive cat-cry, the
    // game pitched its cat recording up too). A pure recipe: the `breath`
    // whisperizer for the airy exhale + a slow shallow `warble` for the
    // mournful waver + a big cavernous reverb for the floating Nether hall.
    BundledSource {
        id: "ghast",
        json: include_str!("bundled/ghast.json"),
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

    // v1.32.0: Spirit Radio is the warm vintage-broadcast voice, rebuilt on real
    // DSP — a `radio_bandpass` (steep band-limit + cone honk, replacing the old
    // EQ/de-esser fakes), a broadcast compressor, asymmetric "warm" tube
    // saturation, and a `vintage_noise` floor that fills the gaps. Lock that
    // identity so an edit can't blur it into the modern, tactical "dispatch".
    #[test]
    fn spirit_radio_is_a_vintage_broadcast_chain() {
        let presets = bundled_presets();
        let p = presets
            .iter()
            .find(|p| p.id == "spirit-radio")
            .expect("spirit-radio must be bundled");
        assert_eq!(p.name, "Spirit Radio");
        // A close-mic'd announcer, not a hall — stays dry (like dispatch).
        assert!(
            p.chain.iter().all(|e| e.id != "reverb"),
            "Spirit Radio stays dry (no reverb)"
        );
        // The real band-limiter (a wall, not the EQ shelves' slope).
        assert!(
            p.chain
                .iter()
                .any(|e| e.id == "radio_bandpass" && e.enabled),
            "band-limited via radio_bandpass"
        );
        // Relentless broadcast level + warm (even-harmonic) tube grit.
        assert!(
            p.chain.iter().any(|e| e.id == "compressor" && e.enabled),
            "broadcast compressor"
        );
        let dist = p
            .chain
            .iter()
            .find(|e| e.id == "distortion")
            .expect("tube saturation");
        assert!(
            dist.vals.get("warmth").copied().unwrap_or(0.0) > 0.0,
            "warm (asymmetric) saturation"
        );
        // The noise floor that fills the silences.
        assert!(
            p.chain.iter().any(|e| e.id == "vintage_noise" && e.enabled),
            "vintage noise floor"
        );
    }

    // v1.32.0: Parlor Augur is the tinny, distant tabletop set. Its identity is a
    // NARROWER radio_bandpass than Spirit (higher low cut + lower high cut), a
    // whisper of reverb for the across-the-room distance, and a hotter noise bed.
    #[test]
    fn parlor_augur_is_the_distant_tinny_set() {
        let presets = bundled_presets();
        let bp = |id: &str| {
            presets
                .iter()
                .find(|p| p.id == id)
                .unwrap_or_else(|| panic!("{id} must be bundled"))
                .chain
                .iter()
                .find(|e| e.id == "radio_bandpass")
                .unwrap_or_else(|| panic!("{id} must band-limit with radio_bandpass"))
                .clone()
        };
        let spirit = bp("spirit-radio");
        let parlor = bp("parlor-augur");
        // Narrower band than Spirit — thinner, tinnier.
        assert!(
            parlor.vals.get("hp").copied().unwrap_or(0.0)
                > spirit.vals.get("hp").copied().unwrap_or(0.0),
            "Parlor cuts more low end than Spirit"
        );
        assert!(
            parlor.vals.get("lp").copied().unwrap_or(0.0)
                < spirit.vals.get("lp").copied().unwrap_or(0.0),
            "Parlor cuts more top than Spirit"
        );
        let p = presets.iter().find(|p| p.id == "parlor-augur").unwrap();
        // Across the room (Spirit is dry), with the little box's noise floor.
        assert!(
            p.chain.iter().any(|e| e.id == "reverb" && e.enabled),
            "a little room for the distance"
        );
        assert!(
            p.chain.iter().any(|e| e.id == "vintage_noise" && e.enabled),
            "the little box's noise floor"
        );
    }

    // v1.34.0: the Villager voice. Its identity is the nasal pole/notch pair
    // (a low murmur BOOST + a CUT at the ~1 kHz "m" antiformant — the latter
    // only possible now that radio_bandpass gain can go negative) plus the
    // tremolo "hrm-hrm" wobble that flips it from "nasal voice" to "villager".
    // Pitched UP (the game's own production) and dry. Lock all of that.
    #[test]
    fn villager_is_a_nasal_wobbling_npc() {
        let presets = bundled_presets();
        let p = presets
            .iter()
            .find(|p| p.id == "villager")
            .expect("villager must be bundled");
        assert_eq!(p.name, "Villager");
        // Pitched UP (small comic NPC) — direction the game's sound confirms.
        let pitch = p
            .chain
            .iter()
            .find(|e| e.id == "pitch")
            .expect("villager is pitched");
        assert!(
            pitch.vals.get("shift").copied().unwrap_or(0.0) > 0.0,
            "villager is pitched UP"
        );
        // The amplitude wobble — the single most villager-specific cue.
        assert!(
            p.chain.iter().any(|e| e.id == "tremolo" && e.enabled),
            "the 'hrm-hrm' wobble (tremolo)"
        );
        // Two radio_bandpass stages: a nasal POLE (boost) and a nasal NOTCH
        // (negative gain) — true /m/ nasality, not a generic midrange honk.
        let bands: Vec<_> = p
            .chain
            .iter()
            .filter(|e| e.id == "radio_bandpass" && e.enabled)
            .collect();
        assert!(bands.len() >= 2, "a nasal pole + notch pair");
        assert!(
            bands
                .iter()
                .any(|e| e.vals.get("gain").copied().unwrap_or(0.0) > 0.0),
            "a nasal-murmur BOOST (pole)"
        );
        assert!(
            bands
                .iter()
                .any(|e| e.vals.get("gain").copied().unwrap_or(0.0) < 0.0),
            "a nasal-antiformant CUT (notch) — needs negative band-pass gain"
        );
        // Close-mic NPC, not a hall — dry.
        assert!(
            p.chain.iter().all(|e| e.id != "reverb"),
            "villager stays dry (no reverb)"
        );
    }

    // v1.35.0: the Creeper voice. Its identity is the `breath` whisperizer —
    // additive noise that SWELLS with the voice (the building fuse) — plus a
    // sibilant-band BOOST (not a de-ess) and NO tonal carrier. Pitched DOWN
    // (the game hiss is the fuse pitched down), the opposite of the villager.
    #[test]
    fn creeper_is_a_swelling_hiss_voice() {
        let presets = bundled_presets();
        let p = presets
            .iter()
            .find(|p| p.id == "creeper")
            .expect("creeper must be bundled");
        assert_eq!(p.name, "Creeper");
        // The linchpin: the envelope-following hiss that swells with the voice.
        assert!(
            p.chain.iter().any(|e| e.id == "breath" && e.enabled),
            "the swelling fuse hiss (breath whisperizer)"
        );
        // Pitched DOWN (the fuse pitched down) — opposite of the villager (+).
        let pitch = p.chain.iter().find(|e| e.id == "pitch");
        assert!(
            pitch
                .and_then(|e| e.vals.get("shift").copied())
                .unwrap_or(0.0)
                < 0.0,
            "creeper is pitched DOWN"
        );
        // Sibilance is BOOSTED, never de-essed (the de-esser can only cut).
        assert!(
            p.chain.iter().all(|e| e.id != "deesser"),
            "no de-esser — the sss band is boosted, not cut"
        );
        // No metallic ring-mod carrier — a creeper is a hiss, not a robot.
        assert!(
            p.chain.iter().all(|e| e.id != "robot"),
            "no robot ring-mod (antithetical to a hiss)"
        );
    }

    // v1.36.0: the Zombie voice. Its identity is a SHALLOW pitch drop (an
    // undead human, not a deep demon), grit + an octave-down sub, and a slow
    // moan — deliberately distinct from the deep, clean Leviathan. Lock the
    // shallow pitch (must be higher than Leviathan's), the moan, and the grit.
    #[test]
    fn zombie_is_a_shallow_gravelly_undead() {
        let presets = bundled_presets();
        let p = presets
            .iter()
            .find(|p| p.id == "zombie")
            .expect("zombie must be bundled");
        assert_eq!(p.name, "Zombie");
        let pitch_of = |id: &str| {
            presets
                .iter()
                .find(|p| p.id == id)
                .and_then(|p| p.chain.iter().find(|e| e.id == "pitch"))
                .and_then(|e| e.vals.get("shift").copied())
        };
        let zshift = pitch_of("zombie").expect("zombie is pitched");
        assert!(zshift < 0.0, "zombie is pitched DOWN");
        // ...but SHALLOWER than Leviathan — an undead human, not a leviathan.
        if let Some(lev) = pitch_of("leviathan") {
            assert!(
                zshift > lev,
                "zombie pitch ({zshift}) must be shallower than leviathan ({lev})"
            );
        }
        // The slow moan.
        assert!(
            p.chain.iter().any(|e| e.id == "tremolo" && e.enabled),
            "the slow moan (tremolo)"
        );
        // Grit + an octave-down body (the growl fake), not depth alone.
        assert!(
            p.chain.iter().any(|e| e.id == "distortion" && e.enabled),
            "gravel (distortion)"
        );
        assert!(
            p.chain.iter().any(|e| e.id == "harmonizer" && e.enabled),
            "sub-octave body (harmonizer)"
        );
        // No tonal ring-mod (wrong for organic undead); no de-ess (keep body).
        assert!(p.chain.iter().all(|e| e.id != "robot"), "no robot ring-mod");
        assert!(p.chain.iter().all(|e| e.id != "deesser"), "no de-esser");
    }

    // v1.37.0: the Enderman voice — an honest OTHERWORLDLY approximation (not
    // literal reversed speech). Its identity is the new `warble` pitch-vibrato
    // (the wobble), pitched DOWN for the tall being, with a garbling ring-mod
    // and a big void reverb. Lock the warble, the down-pitch, and the space.
    #[test]
    fn enderman_is_a_warbling_otherworldly_voice() {
        let presets = bundled_presets();
        let p = presets
            .iter()
            .find(|p| p.id == "enderman")
            .expect("enderman must be bundled");
        assert_eq!(p.name, "Enderman");
        // The identity linchpin: the pitch-vibrato wobble (the new effect).
        assert!(
            p.chain.iter().any(|e| e.id == "warble" && e.enabled),
            "the otherworldly wobble (warble)"
        );
        // Pitched DOWN — a tall being from the void.
        let shift = p
            .chain
            .iter()
            .find(|e| e.id == "pitch")
            .and_then(|e| e.vals.get("shift").copied())
            .unwrap_or(0.0);
        assert!(shift < 0.0, "enderman is pitched down");
        // The void — a big reverb.
        assert!(
            p.chain.iter().any(|e| e.id == "reverb" && e.enabled),
            "the void (reverb)"
        );
    }

    // v1.38.0: the Ghast voice — the family's odd one out, pitched UP for the
    // high plaintive cat-cry (creeper/enderman/zombie all go DOWN). Its
    // identity is the airy `breath` exhale, a slow mournful `warble`, and a
    // big floating reverb. Lock the up-pitch, the breath, the waver, the space.
    #[test]
    fn ghast_is_a_high_breathy_wailing_cry() {
        let presets = bundled_presets();
        let p = presets
            .iter()
            .find(|p| p.id == "ghast")
            .expect("ghast must be bundled");
        assert_eq!(p.name, "Ghast");
        // Pitched UP — the invert of the other mobs (a high, piercing wail).
        let shift = p
            .chain
            .iter()
            .find(|e| e.id == "pitch")
            .and_then(|e| e.vals.get("shift").copied())
            .unwrap_or(0.0);
        assert!(shift > 0.0, "ghast is pitched UP (unlike the other mobs)");
        // The airy exhale + the mournful waver + the floating space.
        assert!(
            p.chain.iter().any(|e| e.id == "breath" && e.enabled),
            "the airy exhale (breath)"
        );
        assert!(
            p.chain.iter().any(|e| e.id == "warble" && e.enabled),
            "the mournful waver (warble)"
        );
        assert!(
            p.chain.iter().any(|e| e.id == "reverb" && e.enabled),
            "the floating cavern (reverb)"
        );
        // Organic cat-cry, not a robot.
        assert!(p.chain.iter().all(|e| e.id != "robot"), "no robot ring-mod");
    }
}
