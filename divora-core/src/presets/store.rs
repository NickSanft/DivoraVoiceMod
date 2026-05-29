//! On-disk store for user presets. Bundled presets do not pass through
//! this module — they live as `include_str!`'d JSON inside the binary.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use thiserror::Error;

use super::{Preset, PresetTag};

#[derive(Debug, Error)]
pub enum PresetStoreError {
    #[error("preset id is empty or contains path separators")]
    InvalidId,

    #[error("preset '{0}' not found")]
    NotFound(String),

    #[error("refusing to overwrite bundled preset '{0}' — use Save as to create a user copy")]
    BundledIsReadOnly(String),

    #[error("preset directory I/O failed: {0}")]
    Io(#[from] io::Error),

    #[error("preset JSON parse failed: {0}")]
    Parse(#[from] serde_json::Error),
}

/// File-backed user preset store. Tauri shell constructs one of these
/// pointing at `%APPDATA%\DivoraVoice\presets\` (or the platform
/// equivalent); divora-core itself stays path-agnostic so tests can use
/// a `tempfile` directory.
pub struct PresetStore {
    base_dir: PathBuf,
}

impl PresetStore {
    /// Create a store pointing at `base_dir`. The directory is created
    /// if missing.
    pub fn new(base_dir: PathBuf) -> io::Result<Self> {
        fs::create_dir_all(&base_dir)?;
        Ok(Self { base_dir })
    }

    /// Where this store keeps its files. Useful for diagnostics.
    #[must_use]
    pub fn base_dir(&self) -> &Path {
        &self.base_dir
    }

    /// Enumerate every user preset on disk. Files that fail to parse
    /// are skipped (logged); a corrupt preset shouldn't take down the
    /// whole list.
    pub fn list_user(&self) -> Result<Vec<Preset>, PresetStoreError> {
        let mut out = Vec::new();
        let entries = match fs::read_dir(&self.base_dir) {
            Ok(e) => e,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(out),
            Err(e) => return Err(e.into()),
        };
        for entry in entries {
            let entry = entry?;
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) != Some("json") {
                continue;
            }
            let bytes = match fs::read(&path) {
                Ok(b) => b,
                Err(err) => {
                    tracing::warn!(?path, ?err, "failed to read user preset; skipping");
                    continue;
                }
            };
            match serde_json::from_slice::<Preset>(&bytes) {
                Ok(mut p) => {
                    // Reject anything claiming to be Bundled — only
                    // embedded presets are allowed to use that tag.
                    if matches!(p.tag, PresetTag::Bundled) {
                        tracing::warn!(?path, "user-dir preset has Bundled tag; treating as User");
                        p.tag = PresetTag::User;
                    }
                    out.push(p);
                }
                Err(err) => {
                    tracing::warn!(?path, ?err, "failed to parse user preset; skipping");
                }
            }
        }
        out.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(out)
    }

    /// Persist a user preset. Fails for bundled presets or invalid ids.
    pub fn save(&self, preset: &Preset) -> Result<(), PresetStoreError> {
        if preset.is_bundled() {
            return Err(PresetStoreError::BundledIsReadOnly(preset.id.clone()));
        }
        if !is_safe_id(&preset.id) {
            return Err(PresetStoreError::InvalidId);
        }
        let mut buf = serde_json::to_vec_pretty(preset)?;
        buf.push(b'\n');
        let path = self.base_dir.join(format!("{}.json", preset.id));
        fs::write(path, buf)?;
        Ok(())
    }

    /// Delete a user preset by id. Returns `NotFound` if no such file.
    pub fn delete(&self, id: &str) -> Result<(), PresetStoreError> {
        if !is_safe_id(id) {
            return Err(PresetStoreError::InvalidId);
        }
        let path = self.base_dir.join(format!("{id}.json"));
        match fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                Err(PresetStoreError::NotFound(id.to_owned()))
            }
            Err(e) => Err(e.into()),
        }
    }
}

/// Allow `[a-z0-9_-]` only. The file name is derived from the id, so
/// we have to keep path separators and parent-traversal sequences out.
fn is_safe_id(id: &str) -> bool {
    !id.is_empty()
        && id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
}

#[cfg(test)]
mod tests {
    use super::{is_safe_id, PresetStore, PresetStoreError};
    use crate::presets::{Preset, PresetChainEntry, PresetTag};
    use std::collections::HashMap;

    fn sample_user(id: &str) -> Preset {
        Preset {
            id: id.into(),
            version: 1,
            name: format!("Test {id}"),
            color: "#34D9A0".into(),
            glyph: "eq".into(),
            tag: PresetTag::User,
            desc: "Test description".into(),
            chain: vec![PresetChainEntry {
                id: "gate".into(),
                enabled: true,
                vals: {
                    let mut m = HashMap::new();
                    m.insert("thresh".into(), -45.0);
                    m
                },
            }],
        }
    }

    fn tmp_dir() -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "divora-presets-test-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn is_safe_id_accepts_kebab_and_snake() {
        assert!(is_safe_id("hollow-king"));
        assert!(is_safe_id("my_custom_1"));
        assert!(is_safe_id("a"));
    }

    #[test]
    fn is_safe_id_rejects_traversal_and_separators() {
        assert!(!is_safe_id(""));
        assert!(!is_safe_id(".."));
        assert!(!is_safe_id("a/b"));
        assert!(!is_safe_id("a\\b"));
        assert!(!is_safe_id("HollowKing")); // uppercase
        assert!(!is_safe_id("with space"));
    }

    #[test]
    fn empty_dir_lists_empty() {
        let dir = tmp_dir();
        let store = PresetStore::new(dir.clone()).unwrap();
        assert!(store.list_user().unwrap().is_empty());
    }

    #[test]
    fn save_then_list_round_trips() {
        let dir = tmp_dir();
        let store = PresetStore::new(dir.clone()).unwrap();
        let p = sample_user("round-trip");
        store.save(&p).unwrap();
        let back = store.list_user().unwrap();
        assert_eq!(back.len(), 1);
        assert_eq!(back[0].id, "round-trip");
        assert_eq!(back[0].chain.len(), 1);
    }

    #[test]
    fn save_overwrites_existing_id() {
        let dir = tmp_dir();
        let store = PresetStore::new(dir.clone()).unwrap();
        let mut p = sample_user("overwrite");
        store.save(&p).unwrap();
        p.name = "Updated".into();
        store.save(&p).unwrap();
        let back = store.list_user().unwrap();
        assert_eq!(back.len(), 1);
        assert_eq!(back[0].name, "Updated");
    }

    #[test]
    fn save_refuses_bundled_tag() {
        let dir = tmp_dir();
        let store = PresetStore::new(dir.clone()).unwrap();
        let mut p = sample_user("refuse-bundled");
        p.tag = PresetTag::Bundled;
        let res = store.save(&p);
        assert!(matches!(res, Err(PresetStoreError::BundledIsReadOnly(_))));
    }

    #[test]
    fn save_refuses_invalid_id() {
        let dir = tmp_dir();
        let store = PresetStore::new(dir.clone()).unwrap();
        let mut p = sample_user("ok");
        p.id = "../escape".into();
        let res = store.save(&p);
        assert!(matches!(res, Err(PresetStoreError::InvalidId)));
    }

    #[test]
    fn delete_removes_file() {
        let dir = tmp_dir();
        let store = PresetStore::new(dir.clone()).unwrap();
        let p = sample_user("delete-me");
        store.save(&p).unwrap();
        store.delete("delete-me").unwrap();
        assert!(store.list_user().unwrap().is_empty());
    }

    #[test]
    fn delete_returns_not_found_for_missing() {
        let dir = tmp_dir();
        let store = PresetStore::new(dir.clone()).unwrap();
        let res = store.delete("nope");
        assert!(matches!(res, Err(PresetStoreError::NotFound(_))));
    }

    #[test]
    fn list_skips_non_json_files() {
        let dir = tmp_dir();
        let store = PresetStore::new(dir.clone()).unwrap();
        std::fs::write(dir.join("readme.txt"), "ignore me").unwrap();
        std::fs::write(dir.join("not-a-preset.bin"), [0_u8; 4]).unwrap();
        let p = sample_user("valid");
        store.save(&p).unwrap();
        let back = store.list_user().unwrap();
        assert_eq!(back.len(), 1);
    }

    #[test]
    fn list_skips_corrupt_files_without_failing() {
        let dir = tmp_dir();
        let store = PresetStore::new(dir.clone()).unwrap();
        std::fs::write(dir.join("garbage.json"), "{ not valid json").unwrap();
        let p = sample_user("good");
        store.save(&p).unwrap();
        let back = store.list_user().unwrap();
        assert_eq!(back.len(), 1);
        assert_eq!(back[0].id, "good");
    }

    #[test]
    fn list_rewrites_bundled_tag_on_disk_to_user() {
        // A user preset file claiming Bundled tag should be treated as
        // user-owned anyway; bundled is reserved for embedded presets.
        let dir = tmp_dir();
        let store = PresetStore::new(dir.clone()).unwrap();
        let mut p = sample_user("liar");
        p.tag = PresetTag::Bundled;
        // Bypass `save`'s safety by writing the JSON directly.
        let raw = serde_json::to_string(&p).unwrap();
        std::fs::write(dir.join("liar.json"), raw).unwrap();
        let back = store.list_user().unwrap();
        assert_eq!(back.len(), 1);
        assert!(matches!(back[0].tag, PresetTag::User));
    }
}
