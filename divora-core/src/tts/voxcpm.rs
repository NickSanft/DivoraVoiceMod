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
/// Token appended after the text ids to mark the start of audio.
pub const AUDIO_START_TOKEN: i64 = 101;

/// Audio VAE latent dimension.
const VAE_LATENT_DIM: usize = 64;
/// Audio samples per latent step.
const CHUNK_SIZE: usize = 640;
/// Latent steps grouped into one patch.
const PATCH_SIZE: usize = 2;
/// Audio samples per patch (the VAE input length must be a multiple of this).
const PATCH_LEN: usize = PATCH_SIZE * CHUNK_SIZE;

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

/// Encode a reference clip into `VoxCPM` latent patches, flattened row-major as
/// `[T-1, PATCH_SIZE, 64]` — the prompt conditioning the prefill graph consumes.
/// `samples`/`sample_rate` is the reference clip at any rate (resampled to
/// 16 kHz, linearly, matching the reference pipeline). `None` on any failure.
///
/// Mirrors `bluryar`'s `encode_audio_to_patches`: pad to a multiple of
/// [`PATCH_LEN`], run `audio_data` `[1, 1, T]` → latent `[1, 64, L]`, regroup to
/// `[T, PATCH_SIZE, 64]` and drop the trailing patch.
#[must_use]
pub fn encode_prompt(
    vae_encoder: &mut Session,
    samples: &[f32],
    sample_rate: u32,
) -> Option<Vec<f32>> {
    let mut audio = crate::tts::clone::resample_linear(samples, sample_rate, VOXCPM_SAMPLE_RATE);
    if audio.len() < PATCH_LEN {
        return None;
    }
    let rem = audio.len() % PATCH_LEN;
    if rem != 0 {
        audio.resize(audio.len() + (PATCH_LEN - rem), 0.0);
    }
    let t = audio.len();
    let input = ndarray::Array3::from_shape_vec((1, 1, t), audio).ok()?;
    let inputs = ort::inputs![
        "audio_data" => ort::value::Tensor::from_array(input).ok()?,
    ];
    let outputs = vae_encoder.run(inputs).ok()?;
    let (_, value) = outputs.iter().next()?;
    let arr = value.try_extract_array::<f32>().ok()?;
    let arr = arr.into_dimensionality::<ndarray::Ix3>().ok()?; // [1, 64, L]
    let (_, dim, len) = arr.dim();
    if dim != VAE_LATENT_DIM || !len.is_multiple_of(PATCH_SIZE) {
        return None;
    }
    let keep = (len / PATCH_SIZE).saturating_sub(1); // drop the trailing patch
    let mut patches = Vec::with_capacity(keep * PATCH_SIZE * VAE_LATENT_DIM);
    for ti in 0..keep {
        for p in 0..PATCH_SIZE {
            let col = ti * PATCH_SIZE + p;
            for d in 0..VAE_LATENT_DIM {
                patches.push(arr[[0, d, col]]);
            }
        }
    }
    Some(patches)
}

/// Output names of the prefill graph, in order. The first/last are the model
/// state we read; the four middle ones are the opaque KV cache.
const PREFILL_OUTPUTS: [&str; 6] = [
    "dit_hidden",
    "base_next_keys",
    "base_next_values",
    "residual_next_keys",
    "residual_next_values",
    "prefix_feat_cond",
];

/// The four flattened prefill input tensors + sequence length `S`
/// (= text positions + audio patches).
#[derive(Debug, Clone)]
pub struct PrefillInputs {
    /// `[1, S]` i64: text ids + `AUDIO_START_TOKEN` + zero-pad over audio.
    pub text_token: Vec<i64>,
    /// `[1, S]` i32: 1 over text positions, 0 over audio.
    pub text_mask: Vec<i32>,
    /// `[1, S, PATCH_SIZE, 64]` f32: zeros over text positions, then patches.
    pub feat: Vec<f32>,
    /// `[1, S]` i32: 0 over text positions, 1 over audio (inverse of text mask).
    pub feat_mask: Vec<i32>,
    pub seq_len: usize,
}

/// One model output/state tensor (shape + row-major data) — the form the decode
/// loop threads between steps.
#[derive(Debug, Clone)]
pub struct StateTensor {
    pub shape: Vec<usize>,
    pub data: Vec<f32>,
}

/// Build the prefill inputs from `prompt_text + target_text` (tokenized
/// together) and the encoded prompt `patches` (flattened `[P, PATCH_SIZE, 64]`).
/// Pure — no model needed. Mirrors `bluryar`'s `build_inputs_with_patches`.
#[must_use]
pub fn build_prefill_inputs(
    tokenizer: &Tokenizer,
    prompt_text: &str,
    target_text: &str,
    patches: &[f32],
) -> Option<PrefillInputs> {
    let mut text_seed = tokenize(tokenizer, &format!("{prompt_text}{target_text}"))?;
    text_seed.push(AUDIO_START_TOKEN);
    assemble_prefill(text_seed, patches)
}

/// Pure assembly of the prefill tensors from the text token seed (text ids +
/// `AUDIO_START_TOKEN`) and the prompt `patches`. Split out so the
/// padding/masking is unit-testable without a tokenizer or model.
fn assemble_prefill(text_seed: Vec<i64>, patches: &[f32]) -> Option<PrefillInputs> {
    let text_length = text_seed.len();
    let stride = PATCH_SIZE * VAE_LATENT_DIM;
    if stride == 0 || !patches.len().is_multiple_of(stride) {
        return None;
    }
    let audio_length = patches.len() / stride;
    let seq_len = text_length + audio_length;

    let mut text_token = text_seed;
    text_token.resize(seq_len, 0); // zero-pad over the audio positions

    let mut text_mask = vec![1i32; text_length];
    text_mask.resize(seq_len, 0);
    let mut feat_mask = vec![0i32; text_length];
    feat_mask.resize(seq_len, 1);

    let mut feat = vec![0f32; text_length * stride];
    feat.extend_from_slice(patches);

    Some(PrefillInputs {
        text_token,
        text_mask,
        feat,
        feat_mask,
        seq_len,
    })
}

/// Run the prefill graph, returning its 6 outputs (see [`PREFILL_OUTPUTS`]) as
/// owned [`StateTensor`]s — `dit_hidden`, the four KV-cache tensors, and
/// `prefix_feat_cond` — the seed the decode loop iterates from.
#[must_use]
pub fn run_prefill(prefill: &mut Session, inp: &PrefillInputs) -> Option<Vec<StateTensor>> {
    let s = inp.seq_len;
    let text = ndarray::Array2::from_shape_vec((1, s), inp.text_token.clone()).ok()?;
    let text_mask = ndarray::Array2::from_shape_vec((1, s), inp.text_mask.clone()).ok()?;
    let feat_mask = ndarray::Array2::from_shape_vec((1, s), inp.feat_mask.clone()).ok()?;
    let feat =
        ndarray::Array4::from_shape_vec((1, s, PATCH_SIZE, VAE_LATENT_DIM), inp.feat.clone())
            .ok()?;

    let inputs = ort::inputs![
        "text" => ort::value::Tensor::from_array(text).ok()?,
        "text_mask" => ort::value::Tensor::from_array(text_mask).ok()?,
        "feat" => ort::value::Tensor::from_array(feat).ok()?,
        "feat_mask" => ort::value::Tensor::from_array(feat_mask).ok()?,
    ];
    let outputs = prefill.run(inputs).ok()?;
    let mut state = Vec::with_capacity(PREFILL_OUTPUTS.len());
    for name in PREFILL_OUTPUTS {
        let arr = outputs[name].try_extract_array::<f32>().ok()?;
        state.push(StateTensor {
            shape: arr.shape().to_vec(),
            data: arr.iter().copied().collect(),
        });
    }
    Some(state)
}

/// Output names of the decode_step graph, in order: `pred_feat`, the 5 new
/// states (dit_hidden + 4 KV), then `stop_flag` (bool).
const DECODE_OUTPUTS: [&str; 7] = [
    "pred_feat",
    "new_dit_hidden",
    "new_base_next_keys",
    "new_base_next_values",
    "new_residual_next_keys",
    "new_residual_next_values",
    "stop_flag",
];

/// Build an `ort` f32 tensor from a shape + row-major data (handles the
/// varying-rank KV-cache tensors and the 0-d `cfg` scalar uniformly).
fn array_tensor(shape: &[usize], data: Vec<f32>) -> Option<ort::value::Tensor<f32>> {
    let arr = ndarray::ArrayD::from_shape_vec(ndarray::IxDyn(shape), data).ok()?;
    ort::value::Tensor::from_array(arr).ok()
}

/// Run the autoregressive decode loop from the prefill `state` (the 6
/// [`run_prefill`] outputs), returning the sequence of `pred_feat` latent
/// patches (`[1, PATCH_SIZE, 64]` each). Each step feeds the 6 state tensors +
/// `noise_fn(prefix_feat_cond.shape)` + `cfg`; the next `prefix_feat_cond` is the
/// step's `pred_feat`, and `dit_hidden` + the 4 KV tensors are replaced by the
/// step's outputs. Stops on the model's `stop_flag` once more than `min_len`
/// patches exist, or at `max_len`. `noise_fn` is the diffusion noise source
/// (random in production; deterministic zeros for parity tests).
#[must_use]
pub fn run_decode(
    decode_step: &mut Session,
    mut state: Vec<StateTensor>,
    cfg: f32,
    min_len: usize,
    max_len: usize,
    mut noise_fn: impl FnMut(&[usize]) -> Vec<f32>,
) -> Option<Vec<StateTensor>> {
    if state.len() != 6 {
        return None;
    }
    let mut preds: Vec<StateTensor> = Vec::new();
    for _ in 0..max_len {
        let noise_shape = state[5].shape.clone();
        let noise = noise_fn(&noise_shape);
        let inputs = ort::inputs![
            "dit_hidden" => array_tensor(&state[0].shape, state[0].data.clone())?,
            "base_next_keys" => array_tensor(&state[1].shape, state[1].data.clone())?,
            "base_next_values" => array_tensor(&state[2].shape, state[2].data.clone())?,
            "residual_next_keys" => array_tensor(&state[3].shape, state[3].data.clone())?,
            "residual_next_values" => array_tensor(&state[4].shape, state[4].data.clone())?,
            "prefix_feat_cond" => array_tensor(&state[5].shape, state[5].data.clone())?,
            "noise" => array_tensor(&noise_shape, noise)?,
            "cfg_value" => array_tensor(&[], vec![cfg])?,
        ];
        let outputs = decode_step.run(inputs).ok()?;
        let get = |name: &str| -> Option<StateTensor> {
            let arr = outputs[name].try_extract_array::<f32>().ok()?;
            Some(StateTensor {
                shape: arr.shape().to_vec(),
                data: arr.iter().copied().collect(),
            })
        };
        let pred = get(DECODE_OUTPUTS[0])?;
        let new_state: Vec<StateTensor> = DECODE_OUTPUTS[1..6]
            .iter()
            .map(|&n| get(n))
            .collect::<Option<_>>()?;
        let stop = preds.len() + 1 > min_len
            && outputs[DECODE_OUTPUTS[6]]
                .try_extract_array::<bool>()
                .ok()
                .and_then(|a| a.iter().next().copied())
                .unwrap_or(false);

        preds.push(pred.clone());
        state[0] = new_state[0].clone();
        state[1] = new_state[1].clone();
        state[2] = new_state[2].clone();
        state[3] = new_state[3].clone();
        state[4] = new_state[4].clone();
        state[5] = pred; // next prefix_feat_cond is this step's pred_feat
        if stop {
            break;
        }
    }
    Some(preds)
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

    /// The Rust VAE-encoder port must reproduce the Python ORT oracle's latent
    /// patches for the same 16 kHz input (`bluryar`'s `encode_audio_to_patches`
    /// on `me.wav`). Requires the staged encoder + ref wav + `onnxruntime.dll`,
    /// so it's `#[ignore]`d. Run:
    /// `cargo test -p divora-core encode_prompt_matches_oracle -- --ignored`.
    #[test]
    #[ignore = "requires staged voxcpm-vae-encoder.onnx + ref wav + onnxruntime.dll (local only)"]
    fn encode_prompt_matches_oracle() {
        let res = concat!(env!("CARGO_MANIFEST_DIR"), "/../src-tauri/resources");
        std::env::set_var("ORT_DYLIB_PATH", format!("{res}/onnxruntime.dll"));

        let reader = hound::WavReader::open(format!("{res}/tts/voxcpm-ref-16k.wav"))
            .expect("staged 16 kHz ref wav");
        let samples: Vec<f32> = reader.into_samples::<f32>().map(Result::unwrap).collect();
        assert_eq!(samples.len(), 192_000);

        let mut sess = load_session(Path::new(&format!("{res}/tts/voxcpm-vae-encoder.onnx")))
            .expect("vae encoder session");
        let patches = encode_prompt(&mut sess, &samples, VOXCPM_SAMPLE_RATE).expect("encode");

        // Oracle: shape (149, 2, 64), sum 535.058, spot values.
        assert_eq!(patches.len(), 149 * PATCH_SIZE * VAE_LATENT_DIM);
        let sum: f32 = patches.iter().sum();
        assert!((sum - 535.058_f32).abs() < 1.0, "sum was {sum}");

        let approx = |a: f32, b: f32| assert!((a - b).abs() < 1e-2, "{a} vs {b}");
        let first5: [f32; 5] = [0.44306, 0.73455, 0.94602, -0.04608, -0.17029];
        for (i, &e) in first5.iter().enumerate() {
            approx(patches[i], e);
        }
        // patches[-1, 1, -5:] → flat base for (ti=148, p=1, d=59..63).
        let last5: [f32; 5] = [0.67470, 0.14353, -0.14778, -0.29811, -1.12688];
        let base = (148 * PATCH_SIZE + 1) * VAE_LATENT_DIM + (VAE_LATENT_DIM - 5);
        for (i, &e) in last5.iter().enumerate() {
            approx(patches[base + i], e);
        }
    }

    #[test]
    fn assemble_prefill_pads_and_masks() {
        let stride = PATCH_SIZE * VAE_LATENT_DIM;
        let patches = vec![0.5f32; 2 * stride]; // 2 audio patches
        let inp = assemble_prefill(vec![10, 20, 30], &patches).unwrap();
        assert_eq!(inp.seq_len, 5); // 3 text + 2 audio
        assert_eq!(inp.text_token, vec![10, 20, 30, 0, 0]);
        assert_eq!(inp.text_mask, vec![1, 1, 1, 0, 0]);
        assert_eq!(inp.feat_mask, vec![0, 0, 0, 1, 1]); // inverse of text_mask
        assert_eq!(inp.feat.len(), 5 * stride);
        assert!(inp.feat[..3 * stride].iter().all(|&x| x == 0.0)); // text positions zeroed
        assert!(inp.feat[3 * stride..]
            .iter()
            .all(|&x| (x - 0.5).abs() < 1e-9));
    }

    /// The prefill graph port must reproduce the Python ORT oracle's 6 outputs
    /// (deterministic — no noise) for the same inputs: `dit_hidden`, the 4
    /// KV-cache tensors, and `prefix_feat_cond`. Requires the staged tokenizer +
    /// VAE encoder + prefill graph + `onnxruntime.dll`, so it's `#[ignore]`d.
    /// Run: `cargo test -p divora-core prefill_matches_oracle -- --ignored`.
    #[test]
    #[ignore = "requires staged voxcpm models + onnxruntime.dll (local only)"]
    fn prefill_matches_oracle() {
        let res = concat!(env!("CARGO_MANIFEST_DIR"), "/../src-tauri/resources");
        std::env::set_var("ORT_DYLIB_PATH", format!("{res}/onnxruntime.dll"));

        let tok = load_tokenizer(Path::new(&format!("{res}/tts/voxcpm-tokenizer.json"))).unwrap();
        let reader = hound::WavReader::open(format!("{res}/tts/voxcpm-ref-16k.wav")).unwrap();
        let samples: Vec<f32> = reader.into_samples::<f32>().map(Result::unwrap).collect();
        let mut ve =
            load_session(Path::new(&format!("{res}/tts/voxcpm-vae-encoder.onnx"))).unwrap();
        let patches = encode_prompt(&mut ve, &samples, VOXCPM_SAMPLE_RATE).unwrap();

        // Exact strings used for the oracle capture.
        let prompt_text = "Prosecutors have opened a massive investigation into \
allegations of fixing games and illegal betting. Different telescope designs \
perform differently and have different strengths and weaknesses.";
        let target_text = "Welcome to Divora. This is my own voice, speaking the words I typed.";
        let inp = build_prefill_inputs(&tok, prompt_text, target_text, &patches).unwrap();
        assert_eq!(inp.seq_len, 196);

        // Staged with bluryar's original name (external .onnx.data ref).
        let mut pf = load_session(Path::new(&format!("{res}/tts/voxcpm_prefill.onnx"))).unwrap();
        let state = run_prefill(&mut pf, &inp).unwrap();

        // (shape, sum) per output, from the Python ORT oracle.
        let expect: [(&[usize], f64); 6] = [
            (&[1, 1024], -37.88686),
            (&[1, 24, 2, 196, 64], -1063.36394),
            (&[1, 24, 2, 196, 64], 2595.34526),
            (&[1, 6, 2, 196, 64], -1796.92019),
            (&[1, 6, 2, 196, 64], 65.49082),
            (&[1, 2, 64], 14.83514),
        ];
        for (i, (shape, sum)) in expect.iter().enumerate() {
            assert_eq!(state[i].shape, *shape, "shape mismatch out {i}");
            let got: f64 = state[i].data.iter().map(|&x| f64::from(x)).sum();
            assert!((got - sum).abs() < 1.0, "out {i} sum {got} vs {sum}");
        }
    }

    /// The decode loop port must reproduce the Python ORT oracle step-for-step
    /// under **fixed (zeros) noise** — proving the 6-tensor state threading +
    /// the `prefix_feat_cond := pred_feat` update are correct (the real loop
    /// uses random noise, validated later by audio quality). Requires the staged
    /// models + `onnxruntime.dll`, so it's `#[ignore]`d. Run:
    /// `cargo test -p divora-core decode_step_matches_oracle -- --ignored`.
    #[test]
    #[ignore = "requires staged voxcpm models + onnxruntime.dll (local only)"]
    fn decode_step_matches_oracle() {
        let res = concat!(env!("CARGO_MANIFEST_DIR"), "/../src-tauri/resources");
        std::env::set_var("ORT_DYLIB_PATH", format!("{res}/onnxruntime.dll"));

        let tok = load_tokenizer(Path::new(&format!("{res}/tts/voxcpm-tokenizer.json"))).unwrap();
        let reader = hound::WavReader::open(format!("{res}/tts/voxcpm-ref-16k.wav")).unwrap();
        let samples: Vec<f32> = reader.into_samples::<f32>().map(Result::unwrap).collect();
        let mut ve =
            load_session(Path::new(&format!("{res}/tts/voxcpm-vae-encoder.onnx"))).unwrap();
        let patches = encode_prompt(&mut ve, &samples, VOXCPM_SAMPLE_RATE).unwrap();

        let prompt_text = "Prosecutors have opened a massive investigation into \
allegations of fixing games and illegal betting. Different telescope designs \
perform differently and have different strengths and weaknesses.";
        let target_text = "Welcome to Divora. This is my own voice, speaking the words I typed.";
        let inp = build_prefill_inputs(&tok, prompt_text, target_text, &patches).unwrap();
        let mut pf = load_session(Path::new(&format!("{res}/tts/voxcpm_prefill.onnx"))).unwrap();
        let state = run_prefill(&mut pf, &inp).unwrap();

        let mut ds =
            load_session(Path::new(&format!("{res}/tts/voxcpm_decode_step.onnx"))).unwrap();
        // Zeros noise, 3 steps, min_len huge so the stop flag never fires.
        let preds = run_decode(&mut ds, state, DEFAULT_CFG, 1000, 3, |shape| {
            vec![0.0f32; shape.iter().product()]
        })
        .unwrap();

        let expect = [12.30785f64, 0.51888, 5.78393];
        assert_eq!(preds.len(), 3);
        for (i, (p, &e)) in preds.iter().zip(expect.iter()).enumerate() {
            assert_eq!(p.shape, vec![1, PATCH_SIZE, VAE_LATENT_DIM]);
            let s: f64 = p.data.iter().map(|&x| f64::from(x)).sum();
            assert!((s - e).abs() < 0.05, "step {i} sum {s} vs {e}");
        }
    }
}
