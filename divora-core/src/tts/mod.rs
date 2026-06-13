//! On-device text-to-speech — the **"Speak"** feature.
//!
//! Pipeline: `text → espeak-ng phonemes → Kokoro token ids → Kokoro ONNX
//! (+ per-voice style vector) → f32 @ 24 kHz → soundboard mixer → output`.
//!
//! This module is the **scaffolding**. The pure tokenizer + chunker
//! ([`tokens`]) and the espeak-ng subprocess wrapper ([`phonemize`]) are
//! complete and unit-tested. [`synthesize`] wires them together but degrades
//! to [`TtsError::NotInstalled`] until the Kokoro model + voice pack + the
//! espeak-ng binary are staged next to the app — it never hangs and never
//! touches `ort` while the assets are absent (mirroring
//! [`crate::dsp`]'s `voice_convert` passthrough-on-missing-model behaviour).
//!
//! The final inference step (loading `kokoro-v1.0.int8.onnx` via `ort`,
//! looking up each voice's 256-dim style vector by token length, running the
//! model, concatenating the 24 kHz output) is implemented and
//! desktop-verified once `voice-assets-v2` carries the ~80 MB model. Gated
//! here so the rest of the feature — UI, command surface, tests — ships now.

pub mod kokoro;
pub mod phonemize;
pub mod tokens;

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Kokoro renders at a fixed 24 kHz; the engine resamples on mix-in.
pub const TTS_SAMPLE_RATE: u32 = 24_000;

/// Filenames for the staged Kokoro assets under the TTS asset directory.
const MODEL_FILE: &str = "kokoro-v1.0.int8.onnx";
/// Our compact `DVTS` voice pack (preset voices only) — see `kokoro::StylePack`.
const VOICES_FILE: &str = "voices-divora.bin";
const CONFIG_FILE: &str = "kokoro-config.json";

/// The curated preset voices shipped in Phase 1: `(id, display name, lang)`.
/// Display names are ours; the ids match Kokoro's voice-pack keys so the
/// style vector can be looked up from `voices-v1.0.bin` once assets land.
const PRESET_VOICES: &[(&str, &str, &str)] = &[
    ("af_heart", "Aria — warm (US)", "en-us"),
    ("af_bella", "Bella — bright (US)", "en-us"),
    ("af_aoede", "Aoede — clear (US)", "en-us"),
    ("am_michael", "Michael — deep (US)", "en-us"),
    ("am_puck", "Puck — lively (US)", "en-us"),
    ("bf_emma", "Emma — soft (UK)", "en-gb"),
    ("bm_george", "George — crisp (UK)", "en-gb"),
];

/// A synthesized utterance: mono f32 samples at [`TTS_SAMPLE_RATE`].
#[derive(Debug, Clone)]
pub struct TtsAudio {
    pub samples: Vec<f32>,
    pub sample_rate: u32,
}

/// A selectable preset voice. `installed` reflects whether the model assets
/// needed to actually synthesize are present on disk (so the UI can show a
/// clear "voice not installed" instead of failing on Speak).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TtsVoiceInfo {
    pub id: String,
    pub name: String,
    /// espeak language used to phonemize for this voice (e.g. `"en-us"`).
    pub lang: String,
    /// True once the Kokoro model, voice pack, config, and espeak-ng are all
    /// present — the same condition [`synthesize`] gates on.
    pub installed: bool,
}

/// Errors from the TTS pipeline. [`TtsError::NotInstalled`] is the one the
/// scaffolding surfaces to every user until the assets are staged.
#[derive(Debug, thiserror::Error)]
pub enum TtsError {
    #[error("text-to-speech voices are not installed")]
    NotInstalled,
    #[error("unknown voice: {0}")]
    UnknownVoice(String),
    #[error("no speakable text")]
    EmptyText,
    #[error("phonemization failed: {0}")]
    Phonemize(#[from] phonemize::PhonemizeError),
    #[error("synthesis failed: {0}")]
    Inference(String),
}

/// Resolved paths to the staged TTS assets, built from a single asset dir so
/// the Tauri layer can point at the bundled resource dir (or a user dir).
#[derive(Debug, Clone)]
pub struct TtsPaths {
    pub model: PathBuf,
    pub voices: PathBuf,
    pub config: PathBuf,
    /// Directory holding `espeak-ng(.exe)` + `espeak-ng-data`.
    pub espeak_dir: PathBuf,
}

impl TtsPaths {
    #[must_use]
    pub fn new(asset_dir: &Path) -> Self {
        Self {
            model: asset_dir.join(MODEL_FILE),
            voices: asset_dir.join(VOICES_FILE),
            config: asset_dir.join(CONFIG_FILE),
            espeak_dir: asset_dir.to_path_buf(),
        }
    }

    /// Whether every asset needed to synthesize is present on disk. Pure
    /// filesystem checks — never touches `ort`, so it's safe to call freely.
    #[must_use]
    pub fn installed(&self) -> bool {
        self.model.exists()
            && self.voices.exists()
            && self.config.exists()
            && phonemize::Phonemizer::from_dir(&self.espeak_dir).is_available()
    }
}

/// List the preset voices, each flagged with whether its assets are installed.
#[must_use]
pub fn list_voices(asset_dir: &Path) -> Vec<TtsVoiceInfo> {
    let installed = TtsPaths::new(asset_dir).installed();
    PRESET_VOICES
        .iter()
        .map(|&(id, name, lang)| TtsVoiceInfo {
            id: id.to_string(),
            name: name.to_string(),
            lang: lang.to_string(),
            installed,
        })
        .collect()
}

/// Synthesize `text` with preset `voice_id`, returning 24 kHz mono audio ready
/// for the soundboard mixer.
///
/// Until the Kokoro model, voice pack, config, and espeak-ng are staged in
/// `asset_dir`, this returns [`TtsError::NotInstalled`] without ever calling
/// into `ort` (so it can't hang on the missing runtime). Once the assets land,
/// the gated branch runs the real phonemize → tokenize → Kokoro pipeline.
pub fn synthesize(text: &str, voice_id: &str, asset_dir: &Path) -> Result<TtsAudio, TtsError> {
    let voice = PRESET_VOICES
        .iter()
        .find(|&&(id, _, _)| id == voice_id)
        .ok_or_else(|| TtsError::UnknownVoice(voice_id.to_string()))?;

    let chunks = tokens::split_text(text, tokens::MAX_PHONEME_LENGTH);
    if chunks.is_empty() {
        return Err(TtsError::EmptyText);
    }

    let paths = TtsPaths::new(asset_dir);
    if !paths.installed() {
        return Err(TtsError::NotInstalled);
    }

    // Assets are present → run the full pipeline. Load the runtime vocab
    // (from the model's config.json) and the compact voice pack, open the
    // Kokoro session, then phonemize → tokenize → infer per chunk.
    let config = std::fs::read_to_string(&paths.config)
        .map_err(|e| TtsError::Inference(format!("read kokoro config: {e}")))?;
    let vocab = tokens::parse_vocab(&config)
        .map_err(|e| TtsError::Inference(format!("parse kokoro vocab: {e}")))?;
    let phonemizer = phonemize::Phonemizer::from_dir(&paths.espeak_dir);

    let pack_bytes = std::fs::read(&paths.voices)
        .map_err(|e| TtsError::Inference(format!("read voice pack: {e}")))?;
    let pack = kokoro::StylePack::parse(&pack_bytes)
        .ok_or_else(|| TtsError::Inference("invalid voice pack".to_string()))?;
    if !pack.has_voice(voice.0) {
        return Err(TtsError::UnknownVoice(voice.0.to_string()));
    }
    let mut session = kokoro::load_session(&paths.model)
        .ok_or_else(|| TtsError::Inference("Kokoro model/runtime unavailable".to_string()))?;

    let mut samples: Vec<f32> = Vec::new();
    for chunk in chunks {
        let phonemes = phonemizer.phonemize(&chunk, voice.2)?;
        let ids = tokens::phonemes_to_ids(&phonemes, &vocab);
        // Kokoro selects the style vector by the *unpadded* token length.
        let inner_len = ids.len().saturating_sub(2);
        let style = pack
            .style(voice.0, inner_len)
            .ok_or_else(|| TtsError::Inference("style lookup failed".to_string()))?;
        let mut audio = kokoro::infer(&mut session, &ids, style, 1.0)
            .ok_or_else(|| TtsError::Inference("Kokoro inference failed".to_string()))?;
        samples.append(&mut audio);
    }
    if samples.is_empty() {
        return Err(TtsError::EmptyText);
    }
    Ok(TtsAudio {
        samples,
        sample_rate: TTS_SAMPLE_RATE,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An asset dir guaranteed to contain none of the staged files.
    fn empty_assets() -> PathBuf {
        std::env::temp_dir().join("divora-tts-tests-no-assets")
    }

    #[test]
    fn list_voices_returns_presets_uninstalled_without_assets() {
        let voices = list_voices(&empty_assets());
        assert_eq!(voices.len(), PRESET_VOICES.len());
        assert!(voices.iter().all(|v| !v.installed));
        assert!(voices.iter().any(|v| v.id == "af_heart"));
        // ids are unique.
        let mut ids: Vec<_> = voices.iter().map(|v| v.id.clone()).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), PRESET_VOICES.len());
    }

    #[test]
    fn synthesize_without_assets_is_not_installed() {
        let err = synthesize("Hello there.", "af_heart", &empty_assets()).unwrap_err();
        assert!(matches!(err, TtsError::NotInstalled), "got {err:?}");
    }

    #[test]
    fn synthesize_unknown_voice_errors_before_install_check() {
        let err = synthesize("Hello", "no_such_voice", &empty_assets()).unwrap_err();
        assert!(matches!(err, TtsError::UnknownVoice(_)), "got {err:?}");
    }

    #[test]
    fn synthesize_empty_text_errors() {
        let err = synthesize("   \n  ", "af_heart", &empty_assets()).unwrap_err();
        assert!(matches!(err, TtsError::EmptyText), "got {err:?}");
    }

    #[test]
    fn paths_not_installed_for_empty_dir() {
        assert!(!TtsPaths::new(&empty_assets()).installed());
    }

    /// Full real pipeline — espeak-ng subprocess → tokens → Kokoro → audio.
    /// Requires the staged assets + `onnxruntime.dll`, so it's `#[ignore]`d
    /// (local-only). Run with:
    /// `cargo test -p divora-core full_pipeline -- --ignored --test-threads=1`.
    #[test]
    #[ignore = "requires staged TTS assets + espeak-ng + onnxruntime.dll (local only)"]
    fn full_pipeline_synthesizes_audio() {
        let res = concat!(env!("CARGO_MANIFEST_DIR"), "/../src-tauri/resources");
        std::env::set_var("ORT_DYLIB_PATH", format!("{res}/onnxruntime.dll"));
        let assets = PathBuf::from(format!("{res}/tts"));
        // Sanity: the gate sees everything installed.
        assert!(TtsPaths::new(&assets).installed(), "assets not all present");
        let audio = synthesize("Speak, and the coven echoes.", "af_heart", &assets)
            .expect("synthesis should succeed with staged assets");
        assert_eq!(audio.sample_rate, TTS_SAMPLE_RATE);
        assert!(
            audio.samples.len() > 10_000,
            "expected real audio, got {} samples",
            audio.samples.len()
        );
        assert!(audio.samples.iter().all(|s| s.is_finite()));
    }

    #[test]
    fn voice_info_serializes_camel_case_stable_keys() {
        let json = serde_json::to_string(&TtsVoiceInfo {
            id: "af_heart".into(),
            name: "Aria".into(),
            lang: "en-us".into(),
            installed: false,
        })
        .unwrap();
        assert!(json.contains("\"id\":\"af_heart\""));
        assert!(json.contains("\"name\":\"Aria\""));
        assert!(json.contains("\"lang\":\"en-us\""));
        assert!(json.contains("\"installed\":false"));
    }
}
