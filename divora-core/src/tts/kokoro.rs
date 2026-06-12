//! Kokoro ONNX inference + the compact Divora voice pack.
//!
//! The model (`kokoro-v1.0.int8.onnx`) takes three inputs — `tokens`
//! (int64 `[1, N]`, the `[0, …ids, 0]`-framed phoneme ids), `style`
//! (f32 `[1, 256]`, the per-voice style vector chosen by token length), and
//! `speed` (f32 `[1]`) — and returns `audio` (f32 `[samples]` @ 24 kHz).
//! (Contract verified against the real model; see `scripts/prep-tts-voices.py`.)
//!
//! The style vectors ship in our own compact `voices-divora.bin` (the `DVTS`
//! format) — just the preset voices, parsed with zero extra dependencies
//! (vs. pulling a zip + npy reader in to read the upstream 27 MB NPZ).
//!
//! Session loading mirrors `dsp::voice_convert`: we never call into `ort`
//! unless both the model file and the runtime dylib are physically present,
//! so a missing runtime degrades to "not installed" rather than hanging.

use std::collections::HashMap;
use std::path::Path;

use ort::session::{builder::GraphOptimizationLevel, Session};

/// Kokoro's `style` vector dimension (the model's `style` input is `[1, 256]`).
pub const STYLE_DIM: usize = 256;

/// A parsed Divora voice pack: voice id → `rows × dim` style matrix (one
/// style row per phoneme-token length, `0..rows`).
pub struct StylePack {
    rows: usize,
    dim: usize,
    voices: HashMap<String, Vec<f32>>,
}

impl StylePack {
    /// Parse the compact `DVTS` format: `"DVTS"` · u32 version · u32 voiceCount
    /// · u32 rows · u32 dim, then per voice: u32 nameLen · name (utf8) ·
    /// `rows·dim` f32 (little-endian). `None` on any structural mismatch.
    #[must_use]
    pub fn parse(bytes: &[u8]) -> Option<Self> {
        if bytes.len() < 20 || &bytes[0..4] != b"DVTS" {
            return None;
        }
        let u32_at = |off: usize| -> Option<usize> {
            let s = bytes.get(off..off + 4)?;
            Some(u32::from_le_bytes([s[0], s[1], s[2], s[3]]) as usize)
        };
        let _version = u32_at(4)?;
        let n = u32_at(8)?;
        let rows = u32_at(12)?;
        let dim = u32_at(16)?;
        if rows == 0 || dim == 0 {
            return None;
        }
        let count = rows.checked_mul(dim)?;
        let need = count.checked_mul(4)?;
        let mut off = 20usize;
        let mut voices = HashMap::with_capacity(n);
        for _ in 0..n {
            let name_len = u32_at(off)?;
            off += 4;
            let name = std::str::from_utf8(bytes.get(off..off + name_len)?)
                .ok()?
                .to_string();
            off += name_len;
            let raw = bytes.get(off..off + need)?;
            off += need;
            let data = raw
                .chunks_exact(4)
                .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
                .collect();
            voices.insert(name, data);
        }
        Some(Self { rows, dim, voices })
    }

    /// The style vector for `voice_id` at phoneme-token length `token_len`
    /// (clamped to the last row). `None` if the voice isn't in the pack.
    #[must_use]
    pub fn style(&self, voice_id: &str, token_len: usize) -> Option<&[f32]> {
        let v = self.voices.get(voice_id)?;
        let row = token_len.min(self.rows - 1);
        let start = row * self.dim;
        v.get(start..start + self.dim)
    }

    /// Whether the pack contains `voice_id`.
    #[must_use]
    pub fn has_voice(&self, voice_id: &str) -> bool {
        self.voices.contains_key(voice_id)
    }
}

/// Attempt to load the Kokoro session. Returns `None` (→ caller degrades to
/// "not installed") when the model file or the ONNX runtime dylib is absent —
/// never calling into `ort` in that case, so it can't hang. Mirrors
/// `dsp::voice_convert::load_session`.
#[must_use]
pub fn load_session(model_path: &Path) -> Option<Session> {
    if !model_path.exists() || !crate::dsp::onnx_runtime_available() {
        return None;
    }
    let builder = Session::builder().ok()?;
    builder
        .with_optimization_level(GraphOptimizationLevel::Level3)
        .ok()?
        .commit_from_file(model_path)
        .ok()
}

/// Run one framed token sequence through Kokoro → 24 kHz f32 samples.
/// `tokens` is the padded `[0, …ids, 0]` sequence; `style` is a
/// [`STYLE_DIM`]-length vector; `speed` is `1.0` for normal rate. `None` on
/// any inference failure (the caller surfaces a clear error, never crashes).
pub fn infer(session: &mut Session, tokens: &[i64], style: &[f32], speed: f32) -> Option<Vec<f32>> {
    let tokens_t = ndarray::Array2::from_shape_vec((1, tokens.len()), tokens.to_vec()).ok()?;
    let style_t = ndarray::Array2::from_shape_vec((1, style.len()), style.to_vec()).ok()?;
    let speed_t = ndarray::Array1::from_vec(vec![speed]);
    let tok = ort::value::Tensor::from_array(tokens_t).ok()?;
    let sty = ort::value::Tensor::from_array(style_t).ok()?;
    let spd = ort::value::Tensor::from_array(speed_t).ok()?;
    let inputs = ort::inputs!["tokens" => tok, "style" => sty, "speed" => spd];
    let outputs = session.run(inputs).ok()?;
    let (_, value) = outputs.iter().next()?;
    let arr = value.try_extract_array::<f32>().ok()?;
    Some(arr.iter().copied().collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A synthetic pack: 1 voice "x", rows=2, dim=3.
    fn synth_pack() -> Vec<u8> {
        let mut b = Vec::new();
        b.extend_from_slice(b"DVTS");
        b.extend_from_slice(&1u32.to_le_bytes()); // version
        b.extend_from_slice(&1u32.to_le_bytes()); // voice count
        b.extend_from_slice(&2u32.to_le_bytes()); // rows
        b.extend_from_slice(&3u32.to_le_bytes()); // dim
        b.extend_from_slice(&1u32.to_le_bytes()); // name_len
        b.extend_from_slice(b"x");
        for f in [0.0f32, 1.0, 2.0, 10.0, 11.0, 12.0] {
            b.extend_from_slice(&f.to_le_bytes());
        }
        b
    }

    #[test]
    fn parse_indexes_rows_and_clamps() {
        let p = StylePack::parse(&synth_pack()).unwrap();
        assert!(p.has_voice("x"));
        assert_eq!(p.style("x", 0).unwrap(), &[0.0, 1.0, 2.0]);
        assert_eq!(p.style("x", 1).unwrap(), &[10.0, 11.0, 12.0]);
        // Beyond the last row clamps to it.
        assert_eq!(p.style("x", 99).unwrap(), &[10.0, 11.0, 12.0]);
        assert!(p.style("missing", 0).is_none());
    }

    #[test]
    fn parse_rejects_bad_input() {
        assert!(StylePack::parse(b"XXXXxxxxxxxxxxxxxxxx").is_none());
        assert!(StylePack::parse(b"DV").is_none());
        assert!(StylePack::parse(&[]).is_none());
    }

    /// Real end-to-end synthesis through `ort` — proves the Rust tensor
    /// wiring matches the verified Python contract. Requires the staged
    /// assets + `onnxruntime.dll`, so it's `#[ignore]`d (local-only; CI has
    /// no model). Run with: `cargo test -p divora-core -- --ignored kokoro`.
    #[test]
    #[ignore = "requires staged TTS assets + onnxruntime.dll (local only)"]
    fn real_synthesis_produces_audio() {
        let res = concat!(env!("CARGO_MANIFEST_DIR"), "/../src-tauri/resources");
        std::env::set_var("ORT_DYLIB_PATH", format!("{res}/onnxruntime.dll"));
        let pack_bytes = std::fs::read(format!("{res}/tts/voices-divora.bin")).unwrap();
        let pack = StylePack::parse(&pack_bytes).unwrap();
        // Build the session directly (bypassing the cached runtime probe).
        let mut session = Session::builder()
            .unwrap()
            .with_optimization_level(GraphOptimizationLevel::Level3)
            .unwrap()
            .commit_from_file(format!("{res}/tts/kokoro-v1.0.int8.onnx"))
            .unwrap();
        // A handful of in-vocab phoneme ids, framed with pad 0.
        let ids: Vec<i64> = vec![0, 50, 83, 54, 156, 57, 135, 0];
        let style = pack.style("af_heart", ids.len() - 2).unwrap();
        let audio = infer(&mut session, &ids, style, 1.0).unwrap();
        assert!(
            audio.len() > 1000,
            "expected real audio, got {}",
            audio.len()
        );
        assert!(audio.iter().all(|s| s.is_finite()));
    }
}
