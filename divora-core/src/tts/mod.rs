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

pub mod clone;
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
/// `OpenVoice` tone-color models for Phase 2 voice cloning — see [`clone`].
const CLONE_EXTRACTOR_FILE: &str = "openvoice-extractor.onnx";
const CLONE_CONVERTER_FILE: &str = "openvoice-converter.onnx";
/// v1.22.0: precomputed per-preset speaker embeddings (bundled JSON,
/// `{ voiceId: [256 f32] }`) used to auto-pick the closest base for a clone.
const BASE_SES_FILE: &str = "base-ses.json";
/// Fallback base voice when auto-selection can't run.
const DEFAULT_BASE_VOICE: &str = "am_puck";

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

/// Whether both `OpenVoice` cloning models are present in `clone_dir`. They're
/// no longer bundled (v1.21.0) — downloaded on demand into a user dir — so the
/// clone path resolves them separately from the bundled Kokoro assets.
#[must_use]
pub fn clone_models_present(clone_dir: &Path) -> bool {
    clone_dir.join(CLONE_EXTRACTOR_FILE).exists() && clone_dir.join(CLONE_CONVERTER_FILE).exists()
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

/// The short, human label for a preset voice id — the part before the `" — "`
/// in its display name (e.g. `am_puck` → `"Puck"`). `None` if `id` isn't a
/// preset. Used to show which base a cloned voice was matched to.
#[must_use]
pub fn preset_short_name(id: &str) -> Option<&'static str> {
    PRESET_VOICES
        .iter()
        .find(|&&(p, _, _)| p == id)
        .map(|&(_, name, _)| name.split(" — ").next().unwrap_or(name))
}

/// Load Kokoro (vocab + voice pack + session) and render each already-split
/// text chunk to 24 kHz audio for `voice_id` (lang `lang`), handing every
/// chunk's samples to `sink`. Shared by [`synthesize`] (raw) and
/// [`synthesize_cloned`] (tone-color converted). Assumes `paths.installed()`.
fn kokoro_render(
    chunks: Vec<String>,
    voice_id: &str,
    lang: &str,
    paths: &TtsPaths,
    mut sink: impl FnMut(Vec<f32>) -> Result<(), TtsError>,
) -> Result<(), TtsError> {
    let config = std::fs::read_to_string(&paths.config)
        .map_err(|e| TtsError::Inference(format!("read kokoro config: {e}")))?;
    let vocab = tokens::parse_vocab(&config)
        .map_err(|e| TtsError::Inference(format!("parse kokoro vocab: {e}")))?;
    let phonemizer = phonemize::Phonemizer::from_dir(&paths.espeak_dir);

    let pack_bytes = std::fs::read(&paths.voices)
        .map_err(|e| TtsError::Inference(format!("read voice pack: {e}")))?;
    let pack = kokoro::StylePack::parse(&pack_bytes)
        .ok_or_else(|| TtsError::Inference("invalid voice pack".to_string()))?;
    if !pack.has_voice(voice_id) {
        return Err(TtsError::UnknownVoice(voice_id.to_string()));
    }
    let mut session = kokoro::load_session(&paths.model)
        .ok_or_else(|| TtsError::Inference("Kokoro model/runtime unavailable".to_string()))?;

    for chunk in chunks {
        let phonemes = phonemizer.phonemize(&chunk, lang)?;
        let ids = tokens::phonemes_to_ids(&phonemes, &vocab);
        // Kokoro selects the style vector by the *unpadded* token length.
        let inner_len = ids.len().saturating_sub(2);
        let style = pack
            .style(voice_id, inner_len)
            .ok_or_else(|| TtsError::Inference("style lookup failed".to_string()))?;
        let audio = kokoro::infer(&mut session, &ids, style, 1.0)
            .ok_or_else(|| TtsError::Inference("Kokoro inference failed".to_string()))?;
        sink(audio)?;
    }
    Ok(())
}

/// Synthesize `text` with preset `voice_id`, returning 24 kHz mono audio ready
/// for the soundboard mixer.
///
/// Until the Kokoro model, voice pack, config, and espeak-ng are staged in
/// `asset_dir`, this returns [`TtsError::NotInstalled`] without ever calling
/// into `ort` (so it can't hang on the missing runtime).
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
    let mut samples: Vec<f32> = Vec::new();
    kokoro_render(chunks, voice.0, voice.2, &paths, |mut audio| {
        samples.append(&mut audio);
        Ok(())
    })?;
    if samples.is_empty() {
        return Err(TtsError::EmptyText);
    }
    Ok(TtsAudio {
        samples,
        sample_rate: TTS_SAMPLE_RATE,
    })
}

/// Synthesize `text` in a **cloned voice**: Kokoro generates with the preset
/// `base_voice_id`, then `OpenVoice` recolors each chunk toward `target_se` (a
/// 256-d speaker embedding from the user's reference clip). Returns audio at
/// [`clone::OV_SAMPLE_RATE`] when cloning is active; if the `OpenVoice` models
/// aren't installed it degrades to the plain base voice (24 kHz).
pub fn synthesize_cloned(
    text: &str,
    base_voice_id: &str,
    target_se: &[f32],
    asset_dir: &Path,
    clone_dir: &Path,
    tau: f32,
) -> Result<TtsAudio, TtsError> {
    let voice = PRESET_VOICES
        .iter()
        .find(|&&(id, _, _)| id == base_voice_id)
        .ok_or_else(|| TtsError::UnknownVoice(base_voice_id.to_string()))?;
    if target_se.len() != clone::SE_DIM {
        return Err(TtsError::Inference("invalid speaker embedding".to_string()));
    }
    let chunks = tokens::split_text(text, tokens::MAX_PHONEME_LENGTH);
    if chunks.is_empty() {
        return Err(TtsError::EmptyText);
    }
    let paths = TtsPaths::new(asset_dir);
    if !paths.installed() {
        return Err(TtsError::NotInstalled);
    }
    // Load the converter pair from the (downloaded) clone dir if present;
    // otherwise fall back to the plain base voice.
    let mut clone_sessions = if clone_models_present(clone_dir) {
        match (
            clone::load_session(&clone_dir.join(CLONE_EXTRACTOR_FILE)),
            clone::load_session(&clone_dir.join(CLONE_CONVERTER_FILE)),
        ) {
            (Some(ext), Some(conv)) => Some((ext, conv)),
            _ => None,
        }
    } else {
        None
    };
    let cloning = clone_sessions.is_some();

    let mut samples: Vec<f32> = Vec::new();
    kokoro_render(chunks, voice.0, voice.2, &paths, |kaudio| {
        if let Some((ext, conv)) = clone_sessions.as_mut() {
            let converted = clone::convert(ext, conv, &kaudio, target_se, tau)
                .ok_or_else(|| TtsError::Inference("tone-color conversion failed".to_string()))?;
            samples.extend(converted);
        } else {
            samples.extend(kaudio);
        }
        Ok(())
    })?;
    if samples.is_empty() {
        return Err(TtsError::EmptyText);
    }
    Ok(TtsAudio {
        samples,
        sample_rate: if cloning {
            clone::OV_SAMPLE_RATE
        } else {
            TTS_SAMPLE_RATE
        },
    })
}

/// Extract a 256-d speaker embedding from a reference clip's mono `samples` at
/// `sample_rate`, for storing a cloned voice. Requires the `OpenVoice` extractor.
pub fn extract_voice_se(
    samples: &[f32],
    sample_rate: u32,
    clone_dir: &Path,
) -> Result<Vec<f32>, TtsError> {
    let extractor_path = clone_dir.join(CLONE_EXTRACTOR_FILE);
    if !extractor_path.exists() {
        return Err(TtsError::NotInstalled);
    }
    let mut extractor = clone::load_session(&extractor_path)
        .ok_or_else(|| TtsError::Inference("OpenVoice extractor unavailable".to_string()))?;
    let resampled = clone::resample_to_ov(samples, sample_rate);
    clone::extract_se(&mut extractor, &resampled)
        .ok_or_else(|| TtsError::Inference("speaker-embedding extraction failed".to_string()))
}

/// Cosine similarity of two equal-length vectors (0.0 if either is degenerate).
fn cosine(a: &[f32], b: &[f32]) -> f32 {
    let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
    let na = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let nb = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    if na == 0.0 || nb == 0.0 {
        0.0
    } else {
        dot / (na * nb)
    }
}

/// Pick the preset base voice whose natural timbre is closest to `target_se`
/// (highest speaker-embedding cosine), reading the bundled [`BASE_SES_FILE`]
/// under `asset_dir`.
///
/// This makes a cloned voice inherit the accent/gender of the nearest preset
/// instead of always starting from one fixed base, so e.g. a UK-sounding
/// reference clones from a UK base. Only ids that are real presets are
/// considered. Falls back to [`DEFAULT_BASE_VOICE`] when the file is missing,
/// unparseable, or yields no usable match (so cloning still works before the
/// asset is staged).
#[must_use]
pub fn pick_base_voice(target_se: &[f32], asset_dir: &Path) -> String {
    if target_se.len() != clone::SE_DIM {
        return DEFAULT_BASE_VOICE.to_string();
    }
    let Ok(text) = std::fs::read_to_string(asset_dir.join(BASE_SES_FILE)) else {
        return DEFAULT_BASE_VOICE.to_string();
    };
    let Ok(map) = serde_json::from_str::<std::collections::BTreeMap<String, Vec<f32>>>(&text)
    else {
        return DEFAULT_BASE_VOICE.to_string();
    };
    let mut best_id = DEFAULT_BASE_VOICE.to_string();
    let mut best_cos = f32::MIN;
    for (id, se) in map {
        if se.len() != target_se.len() || !PRESET_VOICES.iter().any(|&(p, _, _)| p == id) {
            continue;
        }
        let c = cosine(&se, target_se);
        if c > best_cos {
            best_cos = c;
            best_id = id;
        }
    }
    best_id
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

    #[test]
    fn pick_base_voice_defaults_without_file() {
        // No base-ses.json staged → documented fallback to the default base.
        let got = pick_base_voice(&vec![0.1_f32; clone::SE_DIM], &empty_assets());
        assert_eq!(got, DEFAULT_BASE_VOICE);
    }

    #[test]
    fn pick_base_voice_wrong_dim_defaults() {
        // A target SE of the wrong dimension can't be matched → fallback.
        let got = pick_base_voice(&[0.1, 0.2, 0.3], &empty_assets());
        assert_eq!(got, DEFAULT_BASE_VOICE);
    }

    #[test]
    #[allow(clippy::cast_precision_loss)]
    fn pick_base_voice_chooses_highest_cosine_among_presets() {
        use std::io::Write;
        let dir = std::env::temp_dir().join("divora-pick-base-test");
        std::fs::create_dir_all(&dir).unwrap();

        let target: Vec<f32> = (0..clone::SE_DIM).map(|i| i as f32 + 1.0).collect();
        // am_michael: target with one sign flip → cosine just under 1.0.
        let mut michael = target.clone();
        michael[0] = -michael[0];
        // af_heart: reversed → markedly lower cosine.
        let mut heart = target.clone();
        heart.reverse();

        let mut map = std::collections::BTreeMap::<String, Vec<f32>>::new();
        map.insert("am_michael".to_string(), michael);
        map.insert("af_heart".to_string(), heart);
        // A non-preset id collinear with the target (cosine 1.0): it would win
        // outright if the preset filter were broken, so it guards that filter.
        map.insert("zzz_not_a_preset".to_string(), target.clone());

        let json = serde_json::to_string(&map).unwrap();
        std::fs::File::create(dir.join(BASE_SES_FILE))
            .unwrap()
            .write_all(json.as_bytes())
            .unwrap();

        assert_eq!(pick_base_voice(&target, &dir), "am_michael");
        let _ = std::fs::remove_dir_all(&dir);
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

    /// Full cloned-voice path — extract an SE from a reference clip, then
    /// synthesize text in that cloned voice (Kokoro base → `OpenVoice`
    /// convert). `#[ignore]`d (needs staged TTS + `OpenVoice` models + a
    /// reference clip). Run:
    /// `cargo test -p divora-core cloned_synthesis -- --ignored --test-threads=1`.
    #[test]
    #[ignore = "requires staged TTS + OpenVoice models + me.wav (local only)"]
    fn cloned_synthesis_produces_audio() {
        let res = concat!(env!("CARGO_MANIFEST_DIR"), "/../src-tauri/resources");
        std::env::set_var("ORT_DYLIB_PATH", format!("{res}/onnxruntime.dll"));
        let assets = PathBuf::from(format!("{res}/tts"));
        let clip =
            crate::soundboard::decode_clip(Path::new(&format!("{res}/_spike/me.wav"))).unwrap();
        let se = extract_voice_se(&clip.samples, clip.sample_rate, &assets).unwrap();
        assert_eq!(se.len(), clone::SE_DIM);
        // In dev the OpenVoice models live in resources/tts too, so the same
        // dir doubles as the Kokoro asset dir and the clone-model dir.
        let audio = synthesize_cloned(
            "Hello, this is my cloned voice.",
            "am_puck",
            &se,
            &assets,
            &assets,
            0.3,
        )
        .unwrap();
        assert_eq!(audio.sample_rate, clone::OV_SAMPLE_RATE);
        assert!(audio.samples.len() > 10_000 && audio.samples.iter().all(|s| s.is_finite()));
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
