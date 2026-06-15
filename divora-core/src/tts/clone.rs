//! Voice cloning via OpenVoice v2 tone-color conversion (Phase 2).
//!
//! Two ONNX models (MIT, OpenVoice v2) recolor the *timbre* of Kokoro's
//! output toward a target speaker, leaving Kokoro's content/accent intact:
//!   - **extractor**: linear spectrogram `[1, T, 513]` → speaker embedding
//!     (SE) `[1, 256, 1]`.
//!   - **converter**: source spec `[1, 513, T]` + `audio_length` + `src_tone`
//!     + `dest_tone` (256-d SEs) + `tau` → converted audio @ 22.05 kHz.
//!
//! Pipeline (verified against the OpenVoice reference in the 2a spike):
//! `kokoro audio 24k → resample 22.05k → spectrogram(513) → converter(src SE,
//! target SE) → audio`. The spectrogram is the standard VITS linear magnitude
//! (`n_fft 1024`, `hop 256`, reflect-padded, `sqrt(re²+im²+1e-6)`), computed
//! here with `realfft` (same planner the DSP STFT uses). Sessions load via the
//! `dsp::voice_convert` gate, so a missing model/runtime degrades gracefully.

// "OpenVoice" (a product name) recurs in the prose here; and DSP code casts
// freely between int/float for sample indexing, FFT bins, and rates. Mute the
// noise like the other `dsp` modules do.
#![allow(
    clippy::doc_markdown,
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_possible_wrap
)]

use std::path::Path;

use ort::session::{builder::GraphOptimizationLevel, Session};
use realfft::RealFftPlanner;

/// OpenVoice converter sample rate (from its `configuration.json`).
pub const OV_SAMPLE_RATE: u32 = 22_050;
/// Speaker-embedding dimension.
pub const SE_DIM: usize = 256;

const NFFT: usize = 1024;
const HOP: usize = 256;
const BINS: usize = NFFT / 2 + 1; // 513
const PAD: usize = (NFFT - HOP) / 2; // 384

/// Hann window matching numpy's `hanning(NFFT+1)[:-1]` (== the DSP STFT's).
fn hann() -> Vec<f32> {
    (0..NFFT)
        .map(|i| 0.5 * (1.0 - (2.0 * std::f32::consts::PI * i as f32 / NFFT as f32).cos()))
        .collect()
}

/// Linear (reflect-padded) magnitude spectrogram → flat `frames * BINS` plus
/// the frame count. Mirrors the OpenVoice reference `spectrogram_numpy`.
fn spectrogram(samples: &[f32]) -> (Vec<f32>, usize) {
    let n = samples.len();
    if n < PAD + 2 {
        return (Vec::new(), 0);
    }
    // numpy reflect pad: [arr[PAD..1], arr, arr[n-2..n-1-PAD]].
    let mut y = Vec::with_capacity(n + 2 * PAD);
    for k in 0..PAD {
        y.push(samples[PAD - k]);
    }
    y.extend_from_slice(samples);
    for k in 0..PAD {
        y.push(samples[n - 2 - k]);
    }

    let frames = (y.len() - NFFT) / HOP + 1;
    let window = hann();
    let mut planner = RealFftPlanner::<f32>::new();
    let fft = planner.plan_fft_forward(NFFT);
    let mut frame_buf = fft.make_input_vec();
    let mut spec_buf = fft.make_output_vec();

    let mut out = vec![0f32; frames * BINS];
    for f in 0..frames {
        let start = f * HOP;
        for i in 0..NFFT {
            frame_buf[i] = y[start + i] * window[i];
        }
        if fft.process(&mut frame_buf, &mut spec_buf).is_err() {
            return (Vec::new(), 0);
        }
        for (b, c) in spec_buf.iter().enumerate() {
            out[f * BINS + b] = (c.re * c.re + c.im * c.im + 1e-6).sqrt();
        }
    }
    (out, frames)
}

/// Load an OpenVoice ONNX session, gated like `dsp::voice_convert::load_session`
/// (only touches `ort` when the file + runtime dylib are present).
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

/// Resample arbitrary-rate mono samples to [`OV_SAMPLE_RATE`] (for extracting
/// an SE from a reference clip recorded at any rate).
#[must_use]
pub fn resample_to_ov(samples: &[f32], src_rate: u32) -> Vec<f32> {
    resample_linear(samples, src_rate, OV_SAMPLE_RATE)
}

/// Extract a 256-d speaker embedding from `samples` (expected at
/// [`OV_SAMPLE_RATE`]). `None` on any failure.
pub fn extract_se(extractor: &mut Session, samples: &[f32]) -> Option<Vec<f32>> {
    let (spec, frames) = spectrogram(samples);
    if frames == 0 {
        return None;
    }
    se_from_spec(extractor, &spec, frames)
}

fn se_from_spec(extractor: &mut Session, spec: &[f32], frames: usize) -> Option<Vec<f32>> {
    let arr = ndarray::Array3::from_shape_vec((1, frames, BINS), spec.to_vec()).ok()?;
    let input = ort::value::Tensor::from_array(arr).ok()?;
    let outputs = extractor.run(ort::inputs!["input" => input]).ok()?;
    let (_, value) = outputs.iter().next()?;
    let arr = value.try_extract_array::<f32>().ok()?;
    let se: Vec<f32> = arr.iter().take(SE_DIM).copied().collect();
    (se.len() == SE_DIM).then_some(se)
}

/// Recolor `audio_24k` (Kokoro output @ 24 kHz) toward `target_se`, returning
/// converted audio @ [`OV_SAMPLE_RATE`]. `tau` is the conversion temperature
/// (0.3 = OpenVoice default). `None` on any failure (caller falls back).
pub fn convert(
    extractor: &mut Session,
    converter: &mut Session,
    audio_24k: &[f32],
    target_se: &[f32],
    tau: f32,
) -> Option<Vec<f32>> {
    if target_se.len() != SE_DIM {
        return None;
    }
    let src = resample_linear(audio_24k, crate::tts::TTS_SAMPLE_RATE, OV_SAMPLE_RATE);
    let (spec, frames) = spectrogram(&src);
    if frames == 0 {
        return None;
    }
    // Source SE is extracted from the source itself (we don't bundle a base
    // SE — the converter morphs from Kokoro's actual tone to the target).
    let src_se = se_from_spec(extractor, &spec, frames)?;

    // Converter wants the spectrogram transposed to [1, BINS, frames].
    let mut transposed = vec![0f32; frames * BINS];
    for f in 0..frames {
        for b in 0..BINS {
            transposed[b * frames + f] = spec[f * BINS + b];
        }
    }
    let audio = ndarray::Array3::from_shape_vec((1, BINS, frames), transposed).ok()?;
    let src_tone = ndarray::Array3::from_shape_vec((1, SE_DIM, 1), src_se).ok()?;
    let dest_tone = ndarray::Array3::from_shape_vec((1, SE_DIM, 1), target_se.to_vec()).ok()?;
    let length = ndarray::Array1::from_vec(vec![frames as i64]);
    let tau_t = ndarray::Array1::from_vec(vec![tau]);

    let inputs = ort::inputs![
        "audio" => ort::value::Tensor::from_array(audio).ok()?,
        "audio_length" => ort::value::Tensor::from_array(length).ok()?,
        "src_tone" => ort::value::Tensor::from_array(src_tone).ok()?,
        "dest_tone" => ort::value::Tensor::from_array(dest_tone).ok()?,
        "tau" => ort::value::Tensor::from_array(tau_t).ok()?,
    ];
    let outputs = converter.run(inputs).ok()?;
    let (_, value) = outputs.iter().next()?;
    let arr = value.try_extract_array::<f32>().ok()?;
    Some(arr.iter().copied().collect())
}

/// Linear-interpolation resample (matches the spike's `np.interp`; the engine
/// resamples the 22.05 kHz result to 48 kHz on playback anyway). Shared with
/// [`crate::tts::voxcpm`] (its reference-clip resample to 16 kHz also linear).
pub(crate) fn resample_linear(input: &[f32], src: u32, dst: u32) -> Vec<f32> {
    if src == dst || input.len() < 2 {
        return input.to_vec();
    }
    let ratio = f64::from(dst) / f64::from(src);
    let n = (input.len() as f64 * ratio) as usize;
    let last = input.len() - 1;
    (0..n)
        .map(|i| {
            let pos = i as f64 / ratio;
            let idx = pos as usize;
            let frac = (pos - idx as f64) as f32;
            let a = input[idx.min(last)];
            let b = input[(idx + 1).min(last)];
            a * (1.0 - frac) + b * frac
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spectrogram_shape_and_finite() {
        let samples = vec![0.1f32; 24_000]; // ~1s
        let (spec, frames) = spectrogram(&samples);
        assert!(frames > 0);
        assert_eq!(spec.len(), frames * BINS);
        assert!(spec.iter().all(|x| x.is_finite() && *x >= 0.0));
    }

    #[test]
    fn spectrogram_rejects_too_short() {
        assert_eq!(spectrogram(&[0.0; 10]).1, 0);
    }

    #[test]
    fn resample_changes_length_proportionally() {
        let input = vec![0.0f32; 24_000];
        let out = resample_linear(&input, 24_000, 22_050);
        assert!((out.len() as i64 - 22_050).abs() < 4);
    }

    /// Real OpenVoice clone through `ort` — proves the Rust port matches the
    /// verified Python spike. Needs the staged models + a reference clip, so
    /// it's `#[ignore]`d (local only). Run:
    /// `cargo test -p divora-core clone -- --ignored --test-threads=1`.
    #[test]
    #[ignore = "requires staged OpenVoice models + onnxruntime.dll (local only)"]
    fn real_clone_moves_timbre_toward_target() {
        let res = concat!(env!("CARGO_MANIFEST_DIR"), "/../src-tauri/resources");
        std::env::set_var("ORT_DYLIB_PATH", format!("{res}/onnxruntime.dll"));
        let mut ext = Session::builder()
            .unwrap()
            .commit_from_file(format!("{res}/tts/openvoice-extractor.onnx"))
            .unwrap();
        let mut conv = Session::builder()
            .unwrap()
            .commit_from_file(format!("{res}/tts/openvoice-converter.onnx"))
            .unwrap();

        let cos = |a: &[f32], b: &[f32]| -> f32 {
            let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
            let na: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
            let nb: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
            dot / (na * nb)
        };
        let read = |name: &str| {
            let c = crate::soundboard::decode_clip(std::path::Path::new(&format!(
                "{res}/_spike/{name}"
            )))
            .unwrap();
            resample_linear(&c.samples, c.sample_rate, OV_SAMPLE_RATE)
        };
        let target = read("target.wav");
        let source = read("src2.wav"); // already ~22k speech
        let target_se = extract_se(&mut ext, &target).unwrap();
        let raw_se = extract_se(&mut ext, &source).unwrap();
        // convert() expects 24k input; feed 24k-resampled source.
        let src24 = resample_linear(&source, OV_SAMPLE_RATE, crate::tts::TTS_SAMPLE_RATE);
        let out = convert(&mut ext, &mut conv, &src24, &target_se, 0.3).unwrap();
        assert!(out.len() > 1000 && out.iter().all(|s| s.is_finite()));
        let out22 = out; // already 22k
        let out_se = extract_se(&mut ext, &out22).unwrap();
        let before = cos(&raw_se, &target_se);
        let after = cos(&out_se, &target_se);
        println!("clone SE cosine to target: raw {before:.3} -> converted {after:.3}");
        assert!(
            after > before + 0.1,
            "conversion should move timbre toward target (raw {before:.3}, after {after:.3})"
        );
    }
}
