//! Folder enumeration for the soundboard. Walks a directory, picks out
//! files with a supported audio extension, and returns a tile for each.

use std::path::Path;
use std::time::UNIX_EPOCH;

use serde::{Deserialize, Serialize};
use walkdir::WalkDir;

use super::SoundboardError;

/// One soundboard slot returned by `scan_folder`. The id is a hash of
/// the absolute path; the frontend treats it as an opaque key.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SoundboardTile {
    /// Stable opaque id (hash of the canonicalised path).
    pub id: String,
    /// Absolute path on disk. Returned to the frontend so it can be
    /// passed back to `play_clip`.
    pub path: String,
    /// Display label — file stem without extension.
    pub label: String,
    /// File extension (lowercased, no dot).
    pub extension: String,
    /// File size in bytes, for diagnostics + sorting.
    pub size_bytes: u64,
    /// Unix-epoch seconds when the file was last modified, if known.
    pub modified_secs: Option<u64>,
}

const SUPPORTED: &[&str] = &["wav", "mp3", "ogg", "oga", "flac", "opus"];

/// Walk `folder` (not recursively — only direct children for now) and
/// build a tile list. Files with unsupported extensions are silently
/// skipped; an unreadable folder fails with `FolderNotFound`.
pub fn scan_folder(folder: &Path) -> Result<Vec<SoundboardTile>, SoundboardError> {
    if !folder.is_dir() {
        return Err(SoundboardError::FolderNotFound(
            folder.display().to_string(),
        ));
    }

    let mut tiles = Vec::new();
    let walker = WalkDir::new(folder).max_depth(1).follow_links(false);
    for entry in walker.into_iter().flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let Some(extension) = path
            .extension()
            .and_then(|e| e.to_str())
            .map(str::to_lowercase)
        else {
            continue;
        };
        if !SUPPORTED.contains(&extension.as_str()) {
            continue;
        }
        let label = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("clip")
            .to_owned();
        let canonical = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
        let id = hash_path(&canonical);
        let metadata = entry.metadata().ok();
        let size_bytes = metadata.as_ref().map_or(0, std::fs::Metadata::len);
        let modified_secs = metadata.and_then(|m| {
            m.modified()
                .ok()
                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                .map(|d| d.as_secs())
        });
        tiles.push(SoundboardTile {
            id,
            path: canonical.display().to_string(),
            label,
            extension,
            size_bytes,
            modified_secs,
        });
    }
    tiles.sort_by_key(|t| t.label.to_lowercase());
    Ok(tiles)
}

/// Stable hex hash of the path. Not cryptographic — just a label.
fn hash_path(path: &Path) -> String {
    use std::hash::{DefaultHasher, Hasher};
    let mut hasher = DefaultHasher::new();
    hasher.write(path.as_os_str().to_string_lossy().as_bytes());
    format!("{:016x}", hasher.finish())
}

#[cfg(test)]
mod tests {
    use super::{scan_folder, SoundboardError};
    use std::fs;
    use std::path::PathBuf;

    fn tmp_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "divora-soundboard-test-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn touch(dir: &std::path::Path, name: &str, bytes: &[u8]) {
        fs::write(dir.join(name), bytes).unwrap();
    }

    #[test]
    fn missing_folder_errors() {
        let res = scan_folder(&PathBuf::from("/definitely/nope/divora/test"));
        assert!(matches!(res, Err(SoundboardError::FolderNotFound(_))));
    }

    #[test]
    fn empty_folder_returns_empty_list() {
        let dir = tmp_dir();
        let tiles = scan_folder(&dir).unwrap();
        assert!(tiles.is_empty());
    }

    #[test]
    fn picks_up_supported_extensions_and_skips_others() {
        let dir = tmp_dir();
        touch(&dir, "bell.wav", b"RIFF");
        touch(&dir, "owl.mp3", b"ID3");
        touch(&dir, "thunder.ogg", b"OggS");
        touch(&dir, "rune.flac", b"fLaC");
        touch(&dir, "whisper.opus", b"OpusHead");
        touch(&dir, "readme.txt", b"hi");
        touch(&dir, "image.png", b"PNG");
        let tiles = scan_folder(&dir).unwrap();
        assert_eq!(tiles.len(), 5);
        for t in &tiles {
            assert!(["wav", "mp3", "ogg", "flac", "opus"].contains(&t.extension.as_str()));
        }
    }

    #[test]
    fn tiles_are_sorted_by_label() {
        let dir = tmp_dir();
        touch(&dir, "z-last.wav", b"a");
        touch(&dir, "a-first.wav", b"a");
        touch(&dir, "m-middle.wav", b"a");
        let tiles = scan_folder(&dir).unwrap();
        let labels: Vec<_> = tiles.iter().map(|t| t.label.as_str()).collect();
        assert_eq!(labels, vec!["a-first", "m-middle", "z-last"]);
    }

    #[test]
    fn id_is_stable_across_scans() {
        let dir = tmp_dir();
        touch(&dir, "stable.wav", b"RIFF");
        let first = scan_folder(&dir).unwrap();
        let second = scan_folder(&dir).unwrap();
        assert_eq!(first[0].id, second[0].id);
    }

    #[test]
    fn label_strips_extension() {
        let dir = tmp_dir();
        touch(&dir, "Demon Laugh.wav", b"a");
        let tiles = scan_folder(&dir).unwrap();
        assert_eq!(tiles[0].label, "Demon Laugh");
    }

    #[test]
    fn ignores_subdirectories_at_depth_two() {
        let dir = tmp_dir();
        let sub = dir.join("nested");
        fs::create_dir_all(&sub).unwrap();
        touch(&sub, "buried.wav", b"a");
        touch(&dir, "top.wav", b"a");
        let tiles = scan_folder(&dir).unwrap();
        let labels: Vec<_> = tiles.iter().map(|t| t.label.as_str()).collect();
        assert_eq!(labels, vec!["top"]);
    }
}
