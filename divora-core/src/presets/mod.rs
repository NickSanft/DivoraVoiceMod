//! Preset persistence — load bundled defaults from compile-time JSON,
//! read/write user presets to a base directory on disk.
//!
//! ### Why presets live in `divora-core`
//!
//! The audio engine doesn't actually need preset persistence (it only
//! ever sees `DspCommand::SetChain`), but bundling preset I/O alongside
//! the DSP keeps the schema in one place: bundled JSON files describe
//! the exact effects and parameters that `EffectChain::from_specs`
//! consumes.
//!
//! ### Bundled vs user
//!
//! - **Bundled** presets are embedded into the binary via `include_str!`.
//!   They are read-only at runtime; "saving" a bundled preset by name
//!   would refuse. To customise a bundled preset the UI offers
//!   *Save as…*, which writes a new user preset.
//! - **User** presets live as JSON files under the store's `base_dir`
//!   (the Tauri shell points this at `%APPDATA%\DivoraVoice\presets\`).
//!   The file name is `<id>.json`.

mod bundled;
mod store;

pub use bundled::{bundled_preset_ids, bundled_presets};
pub use store::{PresetStore, PresetStoreError};

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// Whether a preset is read-only (bundled with the app) or user-owned.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum PresetTag {
    Bundled,
    User,
}

/// One effect in a preset's chain. Wire format mirrors the frontend's
/// `ChainEntry` exactly.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PresetChainEntry {
    /// `EffectKind` lowercased. Kept as a string here so unknown future
    /// kinds don't fail deserialisation of older clients.
    pub id: String,
    pub enabled: bool,
    pub vals: HashMap<String, f32>,
}

/// Full preset record. `version` is incremented when the schema gains a
/// new required field; old clients can still load older files because
/// `serde(default)` is supplied on every optional addition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Preset {
    pub id: String,
    #[serde(default = "default_version")]
    pub version: u32,
    pub name: String,
    pub color: String,
    pub glyph: String,
    pub tag: PresetTag,
    pub desc: String,
    pub chain: Vec<PresetChainEntry>,
}

fn default_version() -> u32 {
    1
}

impl Preset {
    /// True if this preset cannot be saved or deleted.
    #[must_use]
    pub fn is_bundled(&self) -> bool {
        matches!(self.tag, PresetTag::Bundled)
    }
}
