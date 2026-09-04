//! Speaker-verification reranker for `VoxCPM` best-of-N cloning.
//!
//! `VoxCPM` synthesis is stochastic (per-utterance diffusion noise): most seeds
//! clone the reference well, but a minority *ramble* (run to the decode cap) or
//! drift in timbre. The empirical spike on `me.wav` (see
//! `docs/research/accent-preserving-cloning.md`) showed the per-seed timbre
//! cosine swinging 0.75–0.87 with the occasional crater. The fix is **best-of-N**:
//! generate a few candidates and keep the one closest to the reference speaker.
//!
//! Picking the winner needs a *proper* speaker-verification model, not the
//! OpenVoice tone-color cosine we ship for [`crate::tts::clone`] — that cosine is
//! compressed (all candidates land 0.81–0.87) and, crucially, **rates a rambling
//! candidate near the top** (validated: a 12.5 s runaway scored 0.866 on OpenVoice
//! but 0.598 on the model here, by far the worst). So this module embeds with
//! **WeSpeaker ResNet34-LM** (VoxCeleb, ONNX, CC-BY-4.0): 80-dim Kaldi-`fbank`
//! features → a 256-d embedding; cosine of two embeddings is the SECS score.
//! Sanity on the spike: same-speaker 0.966, different-speaker 0.076 — a real
//! discriminative range.
//!
//! The `fbank` front-end is reproduced here to match `torchaudio.compliance.kaldi`
//! exactly (the model was trained on it): per 25 ms / 10 ms frame — int16 scaling,
//! DC removal, 0.97 pre-emphasis, Hamming window, 512-pt real FFT, power spectrum,
//! 80 triangular mel filters (20 Hz–8 kHz, `1127·ln(1+f/700)`), log, then
//! cepstral mean normalization. Parity vs the Python oracle (`dump_ref_emb.py`) is
//! the `#[ignore]` test below (Rust-vs-Python embedding cosine > 0.999).

// Product names in prose; DSP casts freely between int/float for indexing.
#![allow(
    clippy::doc_markdown,
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]

use std::path::Path;
use std::sync::Arc;

use realfft::num_complex::Complex32;
use realfft::{RealFftPlanner, RealToComplex};

use super::voxcpm::{load_session, VOXCPM_SAMPLE_RATE};
use ort::session::Session;

/// Filename of the staged WeSpeaker ONNX model under the `VoxCPM` model dir.
pub const SPEAKER_MODEL_FILE: &str = "voxcpm-speaker.onnx";

// Kaldi `fbank` front-end parameters (must match the WeSpeaker training config).
const FRAME_LEN: usize = 400; // 25 ms @ 16 kHz
const FRAME_SHIFT: usize = 160; // 10 ms @ 16 kHz
const NUM_MEL: usize = 80;
const FFT_SIZE: usize = 512; // next power of two ≥ FRAME_LEN
const FFT_BINS: usize = FFT_SIZE / 2 + 1; // 257 (rfft output)
const PREEMPH: f32 = 0.97;
const LOW_FREQ: f32 = 20.0;
const HIGH_FREQ: f32 = 8000.0; // Nyquist
const INT16_SCALE: f32 = 32768.0; // kaldi reads PCM at int16 magnitudes

/// Mel scale (kaldi `MelScale`): `1127·ln(1 + f/700)`.
fn mel_hz(f: f32) -> f32 {
    1127.0 * (1.0 + f / 700.0).ln()
}

/// Symmetric Hamming window, matching `torch.hamming_window(N, periodic=false)`:
/// `0.54 − 0.46·cos(2πn/(N−1))`.
fn hamming_window() -> Vec<f32> {
    let denom = (FRAME_LEN - 1) as f32;
    (0..FRAME_LEN)
        .map(|i| 0.54 - 0.46 * (std::f32::consts::TAU * i as f32 / denom).cos())
        .collect()
}

/// The 80 triangular mel filters as `[NUM_MEL][FFT_BINS]` weights, matching
/// `torchaudio`'s `get_mel_banks` (triangles in the mel domain over FFT bins
/// `0..FFT_SIZE/2`; the Nyquist bin is left at zero).
fn mel_banks() -> Vec<Vec<f32>> {
    let fft_bin_width = VOXCPM_SAMPLE_RATE as f32 / FFT_SIZE as f32; // 31.25 Hz
    let mel_low = mel_hz(LOW_FREQ);
    let mel_high = mel_hz(HIGH_FREQ);
    let delta = (mel_high - mel_low) / (NUM_MEL as f32 + 1.0);
    let mut banks = vec![vec![0f32; FFT_BINS]; NUM_MEL];
    for (m, row) in banks.iter_mut().enumerate() {
        let left = mel_low + m as f32 * delta;
        let center = mel_low + (m + 1) as f32 * delta;
        let right = mel_low + (m + 2) as f32 * delta;
        for (i, w) in row.iter_mut().enumerate().take(FFT_SIZE / 2) {
            let mel = mel_hz(fft_bin_width * i as f32);
            let up = (mel - left) / (center - left);
            let down = (right - mel) / (right - center);
            *w = up.min(down).max(0.0);
        }
    }
    banks
}

/// Cosine similarity of two embeddings — the SECS score (0 if either is zero).
#[must_use]
pub fn cosine(a: &[f32], b: &[f32]) -> f32 {
    let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
    let na = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let nb = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    if na == 0.0 || nb == 0.0 {
        0.0
    } else {
        dot / (na * nb)
    }
}

/// The Kaldi `fbank` front-end: precomputed Hamming window, mel filters, and FFT
/// plan. Split from the model session so it's unit-testable without ONNX.
struct Fbank {
    window: Vec<f32>,
    mel: Vec<Vec<f32>>,
    fft: Arc<dyn RealToComplex<f32>>,
}

impl Fbank {
    fn new() -> Self {
        let mut planner = RealFftPlanner::<f32>::new();
        Self {
            window: hamming_window(),
            mel: mel_banks(),
            fft: planner.plan_fft_forward(FFT_SIZE),
        }
    }

    /// 80-dim Kaldi `fbank` for 16 kHz `samples`, with cepstral mean
    /// normalization, returned flat row-major `[T, 80]` plus `T`. `None` if the
    /// clip is shorter than one frame.
    fn compute(&self, samples: &[f32]) -> Option<(Vec<f32>, usize)> {
        if samples.len() < FRAME_LEN {
            return None;
        }
        let n_frames = 1 + (samples.len() - FRAME_LEN) / FRAME_SHIFT; // snip_edges
        let mut feats = vec![0f32; n_frames * NUM_MEL];
        let mut time = vec![0f32; FFT_SIZE];
        let mut freq = vec![Complex32::default(); FFT_BINS];
        for fr in 0..n_frames {
            let start = fr * FRAME_SHIFT;
            // Scale to int16 + remove DC offset (mean of the scaled frame).
            let mut buf = [0f32; FRAME_LEN];
            let mut sum = 0f32;
            for (j, b) in buf.iter_mut().enumerate() {
                *b = samples[start + j] * INT16_SCALE;
                sum += *b;
            }
            let mean = sum / FRAME_LEN as f32;
            for b in &mut buf {
                *b -= mean;
            }
            // Pre-emphasis y[i] = x[i] − 0.97·x[i−1] (x[−1] = x[0]); from the end.
            for j in (1..FRAME_LEN).rev() {
                buf[j] -= PREEMPH * buf[j - 1];
            }
            buf[0] -= PREEMPH * buf[0];
            // Window into the zero-padded FFT buffer.
            for j in 0..FRAME_LEN {
                time[j] = buf[j] * self.window[j];
            }
            for t in time.iter_mut().skip(FRAME_LEN) {
                *t = 0.0;
            }
            let _ = self.fft.process(&mut time, &mut freq);
            // Power spectrum → mel → log (floor at f32 epsilon, like torchaudio).
            for (m, row) in self.mel.iter().enumerate() {
                let mut e = 0f32;
                for (i, &w) in row.iter().enumerate() {
                    e += w * (freq[i].re * freq[i].re + freq[i].im * freq[i].im);
                }
                feats[fr * NUM_MEL + m] = e.max(f32::EPSILON).ln();
            }
        }
        // Cepstral mean normalization: subtract each bin's mean over time.
        for m in 0..NUM_MEL {
            let mean =
                (0..n_frames).map(|fr| feats[fr * NUM_MEL + m]).sum::<f32>() / n_frames as f32;
            for fr in 0..n_frames {
                feats[fr * NUM_MEL + m] -= mean;
            }
        }
        Some((feats, n_frames))
    }
}

/// The loaded speaker-verification reranker: the WeSpeaker ONNX session plus the
/// precomputed `fbank` front-end. Held by the
/// [`crate::tts::voxcpm::VoxCpmEngine`] and reused across candidates.
pub struct SpeakerScorer {
    session: Session,
    fbank: Fbank,
}

impl SpeakerScorer {
    /// Load the WeSpeaker model from `model_path` and build the `fbank`
    /// front-end. `None` if the file or the ONNX runtime is absent (the engine
    /// then runs without reranking — best-of-1).
    #[must_use]
    pub fn load(model_path: &Path) -> Option<Self> {
        Some(Self {
            session: load_session(model_path)?,
            fbank: Fbank::new(),
        })
    }

    /// Embed `samples` (at `rate`, resampled to 16 kHz if needed) into a 256-d
    /// WeSpeaker speaker embedding. `None` on too-short audio or model failure.
    #[must_use]
    pub fn embed(&mut self, samples: &[f32], rate: u32) -> Option<Vec<f32>> {
        let resampled;
        let mono16k: &[f32] = if rate == VOXCPM_SAMPLE_RATE {
            samples
        } else {
            resampled = crate::tts::clone::resample_linear(samples, rate, VOXCPM_SAMPLE_RATE);
            &resampled
        };
        let (feats, t) = self.fbank.compute(mono16k)?;
        let input = ndarray::Array3::from_shape_vec((1, t, NUM_MEL), feats).ok()?;
        let inputs = ort::inputs![
            "input_features" => ort::value::Tensor::from_array(input).ok()?,
        ];
        let outputs = self.session.run(inputs).ok()?;
        let (_, value) = outputs.iter().next()?;
        let arr = value.try_extract_array::<f32>().ok()?; // [1, 256]
        Some(arr.iter().copied().collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[allow(clippy::float_cmp)] // exact small-integer arithmetic
    fn cosine_basic() {
        assert_eq!(cosine(&[1.0, 0.0], &[1.0, 0.0]), 1.0);
        assert_eq!(cosine(&[1.0, 0.0], &[0.0, 1.0]), 0.0);
        assert_eq!(cosine(&[1.0, 0.0], &[-1.0, 0.0]), -1.0);
        assert_eq!(cosine(&[0.0, 0.0], &[1.0, 1.0]), 0.0); // zero vector
    }

    #[test]
    fn hamming_window_symmetric_endpoints() {
        let w = hamming_window();
        assert_eq!(w.len(), FRAME_LEN);
        assert!((w[0] - 0.08).abs() < 1e-5); // 0.54 - 0.46
        assert!((w[FRAME_LEN - 1] - 0.08).abs() < 1e-5);
        assert!(w[FRAME_LEN / 2] > 0.99); // peak near the middle
    }

    #[test]
    #[allow(clippy::float_cmp)] // weights set to exactly 0.0 by construction
    fn mel_banks_shape_nonneg_and_covered() {
        let banks = mel_banks();
        assert_eq!(banks.len(), NUM_MEL);
        for row in &banks {
            assert_eq!(row.len(), FFT_BINS);
            assert!(row.iter().all(|&w| w >= 0.0));
            assert!(row.iter().sum::<f32>() > 0.0, "each filter has support");
            assert_eq!(row[FFT_BINS - 1], 0.0, "Nyquist bin is zero");
        }
        assert_eq!(banks[0][0], 0.0, "DC bin excluded (below 20 Hz)");
    }

    #[test]
    fn fbank_frame_count_and_finite() {
        // A 1 s 16 kHz tone: n_frames = 1 + (16000-400)/160 = 98.
        let n = VOXCPM_SAMPLE_RATE as usize;
        let samples: Vec<f32> = (0..n)
            .map(|i| 0.2 * (std::f32::consts::TAU * 220.0 * i as f32 / n as f32).sin())
            .collect();
        let (feats, t) = Fbank::new().compute(&samples).expect("fbank");
        assert_eq!(t, 98);
        assert_eq!(feats.len(), 98 * NUM_MEL);
        assert!(feats.iter().all(|x| x.is_finite()));
    }

    /// Rust `fbank` + WeSpeaker must reproduce the Python oracle embedding
    /// (`dump_ref_emb.py` on the staged ref) — proving the front-end matches
    /// `torchaudio.compliance.kaldi`. Requires the local WeSpeaker model, the ref
    /// wav, the dumped fixture, and `onnxruntime.dll`, so it's `#[ignore]`d. Run:
    /// `cargo test -p divora-core embed_matches_python_oracle -- --ignored`.
    #[test]
    #[ignore = "requires staged speaker model + ref wav + emb fixture + onnxruntime.dll (local only)"]
    fn embed_matches_python_oracle() {
        let res = concat!(env!("CARGO_MANIFEST_DIR"), "/../src-tauri/resources");
        std::env::set_var("ORT_DYLIB_PATH", format!("{res}/onnxruntime.dll"));

        let mut scorer = SpeakerScorer::load(Path::new(&format!(
            "{res}/_spike/accent/spkr/onnx/model.onnx"
        )))
        .expect("speaker model");
        let reader = hound::WavReader::open(format!("{res}/tts/voxcpm-ref-16k.wav")).unwrap();
        let samples: Vec<f32> = reader.into_samples::<f32>().map(Result::unwrap).collect();

        let emb = scorer.embed(&samples, VOXCPM_SAMPLE_RATE).expect("embed");
        assert_eq!(emb.len(), 256);

        // Python oracle aggregates (dump_ref_emb.py): sum -4.41559, norm 3.57866.
        let sum: f32 = emb.iter().sum();
        let norm = emb.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!((sum - (-4.415_59)).abs() < 0.05, "sum {sum}");
        assert!((norm - 3.578_66).abs() < 0.05, "norm {norm}");

        // Strongest check: cosine vs the dumped Python embedding > 0.999.
        let bytes = std::fs::read(format!("{res}/_spike/accent/spkr/ref16k.emb.f32")).unwrap();
        let (quads, _tail) = bytes.as_chunks::<4>();
        let py: Vec<f32> = quads.iter().copied().map(f32::from_le_bytes).collect();
        assert_eq!(py.len(), 256);
        let cos = cosine(&emb, &py);
        assert!(cos > 0.999, "rust-vs-python embedding cosine {cos}");
    }
}
