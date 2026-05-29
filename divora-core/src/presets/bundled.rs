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
        id: "clean",
        json: include_str!("bundled/clean.json"),
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
    use crate::presets::PresetTag;

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
}
