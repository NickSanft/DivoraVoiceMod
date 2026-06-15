//! Accent-preserving voice cloning via `VoxCPM`-0.5B (Apache-2.0) — Phase C.
//!
//! Unlike [`crate::tts::clone`] (OpenVoice tone-color = *timbre only*, accent
//! comes from the Kokoro base), `VoxCPM` is a reference-conditioned zero-shot TTS
//! model: it takes **text + a reference clip (+ its transcript)** and infers the
//! speaker's timbre *and* accent/prosody from the reference. So a cloned voice
//! keeps the user's accent. See `docs/research/voxcpm-port-blueprint.md`.
//!
//! Phase A validated the model end-to-end under **pure ONNX Runtime on CPU**
//! (no `PyTorch`): cloning `me.wav` produced a timbre cosine of 0.868 — on par
//! with OpenVoice — using the `bluryar` 4-graph export below. This module is the
//! **scaffold** for the Rust `ort` port: paths, the runtime gate, and the
//! constants the inference pipeline needs. The decode loop (prefill →
//! autoregressive `decode_step` with KV cache → VAE decode) lands in the next
//! increment, validated against the Python ORT oracle in `_spike/accent/bluryar`.
//!
//! Port target — the 4-graph decomposition:
//!   - `audio_vae_encoder`: prompt audio (int16 @ 16 kHz) → latent patches.
//!   - `voxcpm_prefill`: prompt ids + target ids + feat embed → hidden + KV seed
//!     + rotary + mask.
//!   - `voxcpm_decode_step`: KV cache + hidden + rotary → KV cache' + latent +
//!     stop token (one autoregressive step; the diffusion loop is inside it).
//!   - `audio_vae_decoder`: accumulated latents → waveform.

// "VoxCPM"/"OpenVoice"/"Kokoro" (product names) recur in the prose; DSP/inference
// code casts freely between int/float for sample indexing and tensor shapes.
#![allow(
    clippy::doc_markdown,
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_possible_wrap
)]

use std::path::{Path, PathBuf};

use ort::session::{builder::GraphOptimizationLevel, Session};
use tokenizers::Tokenizer;

/// `VoxCPM` operates at 16 kHz (the `bluryar` export's prompt + output rate).
pub const VOXCPM_SAMPLE_RATE: u32 = 16_000;
/// Autoregressive stop token id.
pub const STOP_TOKEN: i64 = 1;
/// Classifier-free-guidance scale (the export's default; higher = closer to the
/// reference's voice features, lower = more natural on long text).
pub const DEFAULT_CFG: f32 = 2.0;
/// Minimum generated length (patches) before the stop token is honored.
pub const MIN_DECODE_LEN: usize = 2;
/// Per-step decode budget factor relative to the text length.
pub const DECODE_LIMIT_FACTOR: usize = 6;

/// The fixed sentence the in-app recorder asks the user to read, so its
/// transcript is known without any ASR model (Phase B decision). Phonetically
/// balanced (Harvard-style) for a good ~12 s voice capture.
pub const READ_ALOUD_PROMPT: &str = "The birch canoe slid on the smooth planks. \
Glue the sheet to the dark blue background. These days a chicken leg is a rare \
dish. The juice of lemons makes fine punch.";

/// ONNX graph + tokenizer filenames under the `VoxCPM` model directory.
const PREFILL_FILE: &str = "voxcpm-prefill.onnx";
const DECODE_STEP_FILE: &str = "voxcpm-decode-step.onnx";
const VAE_ENCODER_FILE: &str = "voxcpm-vae-encoder.onnx";
const VAE_DECODER_FILE: &str = "voxcpm-vae-decoder.onnx";
const TOKENIZER_FILE: &str = "voxcpm-tokenizer.json";

/// Resolved paths to the staged `VoxCPM` assets, built from one model dir so the
/// Tauri layer can point at the downloaded user dir (Phase D hosts these
/// quantized — ~1.5 GB — on the release for download-on-demand, like the
/// OpenVoice models in v1.21.0).
#[derive(Debug, Clone)]
pub struct VoxCpmPaths {
    pub prefill: PathBuf,
    pub decode_step: PathBuf,
    pub vae_encoder: PathBuf,
    pub vae_decoder: PathBuf,
    pub tokenizer: PathBuf,
}

impl VoxCpmPaths {
    #[must_use]
    pub fn new(model_dir: &Path) -> Self {
        Self {
            prefill: model_dir.join(PREFILL_FILE),
            decode_step: model_dir.join(DECODE_STEP_FILE),
            vae_encoder: model_dir.join(VAE_ENCODER_FILE),
            vae_decoder: model_dir.join(VAE_DECODER_FILE),
            tokenizer: model_dir.join(TOKENIZER_FILE),
        }
    }

    /// Whether every graph + the tokenizer are present on disk. Pure filesystem
    /// checks — never touches `ort` — so it's safe to call freely (mirrors
    /// [`crate::tts::clone_models_present`]).
    #[must_use]
    pub fn present(&self) -> bool {
        self.prefill.exists()
            && self.decode_step.exists()
            && self.vae_encoder.exists()
            && self.vae_decoder.exists()
            && self.tokenizer.exists()
    }
}

/// Whether the `VoxCPM` models are available in `model_dir` (so the UI can prompt
/// a one-time download before routing a cloned voice through the accent engine).
#[must_use]
pub fn models_present(model_dir: &Path) -> bool {
    VoxCpmPaths::new(model_dir).present()
}

/// Load one `VoxCPM` ONNX graph, degrading to `None` if the file or the ONNX
/// runtime is absent (same gate as [`crate::tts::clone::load_session`]).
#[must_use]
pub fn load_session(path: &Path) -> Option<Session> {
    if !path.exists() || !crate::dsp::onnx_runtime_available() {
        return None;
    }
    Session::builder()
        .ok()?
        .with_optimization_level(GraphOptimizationLevel::Level3)
        .ok()?
        .commit_from_file(path)
        .ok()
}

/// Load `VoxCPM`'s Llama BPE tokenizer from its `tokenizer.json`.
#[must_use]
pub fn load_tokenizer(path: &Path) -> Option<Tokenizer> {
    Tokenizer::from_file(path).ok()
}

/// Tokenize `text` into `VoxCPM` token ids. No special tokens are added — this
/// matches the reference pipeline (`LlamaTokenizerFast.tokenize` +
/// `convert_tokens_to_ids`); the prefill graph adds the structural tokens.
#[must_use]
pub fn tokenize(tokenizer: &Tokenizer, text: &str) -> Option<Vec<i64>> {
    let encoding = tokenizer.encode(text, false).ok()?;
    Some(encoding.get_ids().iter().map(|&id| i64::from(id)).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_resolve_under_model_dir() {
        let p = VoxCpmPaths::new(Path::new("/models/vox"));
        assert!(p.prefill.ends_with("voxcpm-prefill.onnx"));
        assert!(p.decode_step.ends_with("voxcpm-decode-step.onnx"));
        assert!(p.vae_encoder.ends_with("voxcpm-vae-encoder.onnx"));
        assert!(p.vae_decoder.ends_with("voxcpm-vae-decoder.onnx"));
        assert!(p.tokenizer.ends_with("voxcpm-tokenizer.json"));
    }

    #[test]
    fn not_present_for_empty_dir() {
        let dir = std::env::temp_dir().join("divora-voxcpm-absent");
        assert!(!models_present(&dir));
    }

    #[test]
    fn read_aloud_prompt_is_nonempty_ascii() {
        // The fixed transcript must be stable + plain so tokenization matches.
        assert!(READ_ALOUD_PROMPT.len() > 40);
        assert!(READ_ALOUD_PROMPT.is_ascii());
    }

    /// The Rust tokenizer must produce the exact ids the Python reference
    /// (`LlamaTokenizerFast`) does, or the prefill graph gets wrong inputs.
    /// Captured from the `bluryar` ORT oracle. Requires the staged
    /// `voxcpm-tokenizer.json`, so it's `#[ignore]`d (local only). Run:
    /// `cargo test -p divora-core voxcpm_tokenizer -- --ignored`.
    #[test]
    #[ignore = "requires staged voxcpm-tokenizer.json (local only)"]
    fn tokenizer_matches_reference_ids() {
        let res = concat!(env!("CARGO_MANIFEST_DIR"), "/../src-tauri/resources");
        let tok = load_tokenizer(Path::new(&format!("{res}/tts/voxcpm-tokenizer.json")))
            .expect("voxcpm-tokenizer.json should load");
        let cases: &[(&str, &[i64])] = &[
            ("Welcome to Divora.", &[27841, 1385, 7340, 8390, 72]),
            (
                "The quick brown fox jumps over the lazy dog.",
                &[1507, 4766, 13329, 49712, 43384, 1865, 1358, 29117, 6595, 72],
            ),
            ("Hello there, world!", &[21045, 1887, 59342, 2809, 73]),
        ];
        for (text, expect) in cases {
            assert_eq!(
                tokenize(&tok, text).unwrap(),
                *expect,
                "token id mismatch for {text:?}"
            );
        }
    }
}
