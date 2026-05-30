//! Voice conversion via ONNX inference — Phase 12 lands the real-time
//! AI voice changer the plan has been pointing at since v0.1.
//!
//! ### What this is
//!
//! A streaming `AudioEffect` that:
//!
//!   1. Buffers native-rate input (typically 48 kHz from cpal).
//!   2. Resamples it down to **16 kHz** (the rate every published voice-
//!      conversion model assumes).
//!   3. Runs the loaded ONNX session over a chunk of 16 kHz samples.
//!   4. Resamples the converted output back to the native rate.
//!   5. Mixes the converted (wet) signal with a delay-matched dry copy
//!      so a user can dial in any wet/dry blend.
//!
//! The chunk size is 4096 samples at 16 kHz (≈ 256 ms). This matches
//! what the LLVC streaming variant expects and gives reasonable
//! end-to-end latency without overwhelming a typical CPU.
//!
//! ### Graceful degradation
//!
//! The effect is designed to compile and run on every machine,
//! whether or not a voice model is present:
//!
//!   - **No `onnxruntime.dll` on disk** → `Session` load fails →
//!     effect runs as passthrough; the rest of the chain is unaffected.
//!   - **No `.onnx` model file** → no session created → passthrough.
//!   - **Sample rate not 48 kHz** → bypass; downsampling at non-48k
//!     rates is currently out of scope (denoiser has the same
//!     constraint).
//!   - **Session loaded but inference fails** → that chunk passes
//!     through dry; the session stays loaded for the next chunk so
//!     transient errors don't lock the effect into bypass.
//!
//! ### Where models live
//!
//! The Tauri layer is the source of truth for the voice library
//! directory (`%APPDATA%/DivoraVoice/voices/` on Windows). This
//! effect doesn't know about that path — the model file path is
//! handed in via `set_model_path`, called from the audio engine when
//! the UI changes the active voice. Loading happens lazily on the
//! first `process` call after the path changes, so the audio thread
//! never blocks the user-facing UI.

#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]

use std::{
    collections::VecDeque,
    path::{Path, PathBuf},
    sync::{mpsc, OnceLock},
};

use ort::session::{builder::GraphOptimizationLevel, Session};
use rubato::{
    Resampler, SincFixedOut, SincInterpolationParameters, SincInterpolationType, WindowFunction,
};

use super::{AudioEffect, EffectKind};

/// Sample rate the model was trained at. Off-rate input bypasses.
pub const MODEL_RATE: u32 = 16_000;
/// Native rate the rest of the engine runs at.
pub const NATIVE_RATE: u32 = 48_000;
/// Inference chunk size in 16 kHz samples (≈ 256 ms). Matches LLVC's
/// streaming variant; large enough to give the model context, small
/// enough to keep round-trip latency under a third of a second.
pub const CHUNK_16K: usize = 4096;

/// In-place voice converter. Holds optional ONNX session, optional
/// resamplers, and the streaming buffers that bridge native-rate audio
/// callbacks to fixed-rate model chunks.
pub struct VoiceConverter {
    enabled: bool,
    mix: f32,
    /// Currently active voice name (matched against files in the voices
    /// directory). `None` means passthrough.
    voice: Option<String>,
    /// Absolute path of the active model, used to detect redundant
    /// reloads of the same file.
    loaded_path: Option<PathBuf>,
    /// Receiver for an in-flight background load. A voice change spawns
    /// a loader thread (so the audio thread never blocks on
    /// `Session::builder`) and stashes the rx here; `process` polls it
    /// with `try_recv` each callback until the session arrives.
    loader: Option<mpsc::Receiver<Option<Session>>>,
    /// ONNX session — `None` when no model is loaded or load failed.
    session: Option<Session>,
    /// 48 kHz → 16 kHz pre-inference resampler.
    down: Option<MonoSinc>,
    /// 16 kHz → 48 kHz post-inference resampler.
    up: Option<MonoSinc>,
    /// Native-rate input samples queued for downsampling.
    native_in: VecDeque<f32>,
    /// 16 kHz samples queued for the next inference chunk.
    chunk_in: VecDeque<f32>,
    /// 16 kHz samples produced by inference, queued for upsampling.
    chunk_out: VecDeque<f32>,
    /// Native-rate converted samples queued for the engine.
    native_out: VecDeque<f32>,
    /// Dry copy of native-rate input, delayed to align with the wet
    /// stream for a glitch-free wet/dry crossfade.
    dry_delay: VecDeque<f32>,
    /// Last sample rate observed; used to detect device changes that
    /// require a resampler rebuild.
    last_rate: u32,
}

impl VoiceConverter {
    #[must_use]
    pub fn new() -> Self {
        Self {
            enabled: false,
            mix: 1.0,
            voice: None,
            loaded_path: None,
            loader: None,
            session: None,
            down: None,
            up: None,
            native_in: VecDeque::with_capacity(NATIVE_RATE as usize),
            chunk_in: VecDeque::with_capacity(CHUNK_16K * 2),
            chunk_out: VecDeque::with_capacity(CHUNK_16K * 2),
            native_out: VecDeque::with_capacity(NATIVE_RATE as usize),
            dry_delay: VecDeque::with_capacity(NATIVE_RATE as usize),
            last_rate: 0,
        }
    }

    /// Switch to a different voice model. The ONNX session is built on
    /// a background thread — `load_session` can be slow (graph
    /// optimization) or, with a missing runtime DLL, can even block, so
    /// it must never run on the audio thread. The result lands via
    /// `try_recv` in `process`. Pass `None` to clear (passthrough).
    pub fn set_model_path(&mut self, voice: Option<String>, path: Option<PathBuf>) {
        self.voice = voice;
        // Drop the current session immediately so we don't keep
        // converting with the old voice while the new one loads.
        self.session = None;
        self.clear_pipeline();

        match path {
            Some(p) if Some(&p) != self.loaded_path.as_ref() => {
                self.loaded_path = Some(p.clone());
                let (tx, rx) = mpsc::channel();
                // Detached loader thread; if the user switches again
                // before it finishes, we just drop this rx and the
                // send becomes a no-op (rx hung up).
                std::thread::spawn(move || {
                    let _ = tx.send(load_session(&p));
                });
                self.loader = Some(rx);
            }
            Some(_) => {
                // Same path already loaded/loading — nothing to do.
            }
            None => {
                self.loaded_path = None;
                self.loader = None;
            }
        }
    }

    /// Poll the background loader (if any). Cheap `try_recv`; called at
    /// the top of every `process`.
    fn poll_loader(&mut self) {
        if let Some(rx) = self.loader.as_ref() {
            match rx.try_recv() {
                Ok(session) => {
                    self.session = session;
                    self.loader = None;
                }
                Err(mpsc::TryRecvError::Empty) => {
                    // Still loading; stay in passthrough this callback.
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    // Loader thread vanished without a value — give up.
                    self.loader = None;
                }
            }
        }
    }

    /// Name of the active voice, or `None` if passthrough.
    #[must_use]
    pub fn voice(&self) -> Option<&str> {
        self.voice.as_deref()
    }

    /// Whether an ONNX session is currently loaded and ready.
    #[must_use]
    pub fn is_model_loaded(&self) -> bool {
        self.session.is_some()
    }

    fn clear_pipeline(&mut self) {
        self.native_in.clear();
        self.chunk_in.clear();
        self.chunk_out.clear();
        self.native_out.clear();
        self.dry_delay.clear();
        if let Some(d) = self.down.as_mut() {
            d.reset();
        }
        if let Some(u) = self.up.as_mut() {
            u.reset();
        }
    }

    /// Ensure both resamplers exist + are sized for the engine's
    /// current sample rate. Called on rate change.
    fn ensure_resamplers(&mut self, rate: u32) {
        if self.last_rate == rate && self.down.is_some() && self.up.is_some() {
            return;
        }
        self.last_rate = rate;
        // Sized to produce one 16 kHz chunk per round when we have at
        // least one chunk's worth of native input queued. Output chunk
        // for upsampling matches the native callback size we typically
        // see (256 samples in our resampler module's default).
        self.down = MonoSinc::new(rate, MODEL_RATE, CHUNK_16K).ok();
        self.up = MonoSinc::new(MODEL_RATE, rate, 256).ok();
    }

    /// Drain `chunk_in` whenever a full 16k chunk has accumulated;
    /// run inference; push the converted samples onto `chunk_out`.
    fn drain_chunks(&mut self) {
        while self.chunk_in.len() >= CHUNK_16K {
            let mut chunk = vec![0f32; CHUNK_16K];
            for slot in chunk.iter_mut().take(CHUNK_16K) {
                *slot = self.chunk_in.pop_front().unwrap_or(0.0);
            }
            let converted = run_inference(self.session.as_mut(), &chunk);
            for s in converted {
                self.chunk_out.push_back(s);
            }
        }
    }
}

impl Default for VoiceConverter {
    fn default() -> Self {
        Self::new()
    }
}

impl AudioEffect for VoiceConverter {
    fn process(&mut self, buffer: &mut [f32], sample_rate: u32) {
        // Pick up a background-loaded session if one just finished.
        if self.loader.is_some() {
            self.poll_loader();
        }

        // Bypass conditions: disabled, no model, off-rate input.
        let bypass = !self.enabled
            || self.session.is_none()
            || sample_rate != NATIVE_RATE
            || self.mix < 1e-4;
        if bypass {
            // Drop any stale buffered state so re-engagement is clean.
            if !self.native_in.is_empty() || !self.native_out.is_empty() {
                self.clear_pipeline();
            }
            return;
        }

        self.ensure_resamplers(sample_rate);
        if self.down.is_none() || self.up.is_none() {
            return;
        }

        // Phase 1: queue native input + delayed dry copy.
        for &s in buffer.iter() {
            self.native_in.push_back(s);
            self.dry_delay.push_back(s);
        }

        // Phase 2: downsample as many 16k samples as we can. We borrow
        // the resampler in a tight scope so `drain_chunks()` (which
        // borrows `&mut self`) can run between phases without a
        // simultaneous-borrow conflict.
        {
            let Some(down) = self.down.as_mut() else {
                return;
            };
            loop {
                let needed = down.input_frames_next();
                if self.native_in.len() < needed {
                    break;
                }
                let mut batch = Vec::with_capacity(needed);
                for _ in 0..needed {
                    batch.push(self.native_in.pop_front().unwrap_or(0.0));
                }
                down.push(&batch);
                let mut out = vec![0f32; CHUNK_16K];
                let written = down.process(&mut out);
                for &s in out.iter().take(written) {
                    self.chunk_in.push_back(s);
                }
            }
        }

        // Phase 3: run inference on any complete 16k chunks.
        self.drain_chunks();

        // Phase 4: upsample 16k → native and queue to `native_out`.
        if !self.chunk_out.is_empty() {
            let Some(up) = self.up.as_mut() else {
                return;
            };
            loop {
                let needed = up.input_frames_next();
                if self.chunk_out.len() < needed {
                    break;
                }
                let mut batch = Vec::with_capacity(needed);
                for _ in 0..needed {
                    batch.push(self.chunk_out.pop_front().unwrap_or(0.0));
                }
                up.push(&batch);
                let mut out = vec![0f32; 256];
                let written = up.process(&mut out);
                for &s in out.iter().take(written) {
                    self.native_out.push_back(s);
                }
            }
        }

        // Phase 5: write `native_out` (wet) + `dry_delay` (dry) back
        // into the output buffer with the wet/dry mix. During warm-up
        // (when `native_out` is empty), output silence — playing the
        // dry signal here would make those samples repeat audibly once
        // the wet pipeline catches up.
        for slot in buffer.iter_mut() {
            if let (Some(wet), Some(dry)) =
                (self.native_out.pop_front(), self.dry_delay.pop_front())
            {
                *slot = self.mix * wet + (1.0 - self.mix) * dry;
            } else {
                *slot = 0.0;
            }
        }
    }

    fn set_param(&mut self, key: &str, value: f32) {
        if key == "mix" {
            // UI surfaces 0..100; store 0..1.
            self.mix = (value / 100.0).clamp(0.0, 1.0);
        }
    }

    fn enabled(&self) -> bool {
        self.enabled
    }

    fn set_enabled(&mut self, enabled: bool) {
        if !enabled {
            self.clear_pipeline();
        }
        self.enabled = enabled;
    }

    fn kind(&self) -> EffectKind {
        EffectKind::VoiceConvert
    }

    /// `key == "model"` sets the active ONNX model path. The voice name
    /// shown in the UI is derived from the file stem. `None` clears
    /// back to passthrough.
    fn set_resource(&mut self, key: &str, value: Option<&str>) {
        if key != "model" {
            return;
        }
        let path = value.filter(|s| !s.is_empty()).map(PathBuf::from);
        let voice = path
            .as_ref()
            .and_then(|p| p.file_stem())
            .map(|s| s.to_string_lossy().into_owned());
        self.set_model_path(voice, path);
    }
}

/// Thin wrapper around `SincFixedOut` matching the shape we use here.
/// Identical to `crate::audio::resampler::MonoResampler` but kept local
/// so this module compiles without a circular dependency.
struct MonoSinc {
    inner: SincFixedOut<f32>,
    input_buf: Vec<Vec<f32>>,
    output_buf: Vec<Vec<f32>>,
    pending: Vec<f32>,
}

impl MonoSinc {
    fn new(input_rate: u32, output_rate: u32, chunk_out: usize) -> Result<Self, String> {
        let ratio = f64::from(output_rate) / f64::from(input_rate);
        let params = SincInterpolationParameters {
            sinc_len: 128,
            f_cutoff: 0.95,
            interpolation: SincInterpolationType::Linear,
            oversampling_factor: 128,
            window: WindowFunction::BlackmanHarris2,
        };
        let inner = SincFixedOut::<f32>::new(ratio, 1.0, params, chunk_out, 1)
            .map_err(|e| e.to_string())?;
        let max_in = inner.input_frames_max();
        Ok(Self {
            inner,
            input_buf: vec![Vec::with_capacity(max_in)],
            output_buf: vec![vec![0f32; chunk_out]],
            pending: Vec::with_capacity(max_in * 4),
        })
    }

    fn input_frames_next(&self) -> usize {
        self.inner.input_frames_next()
    }

    fn push(&mut self, samples: &[f32]) {
        self.pending.extend_from_slice(samples);
    }

    fn process(&mut self, out: &mut [f32]) -> usize {
        let need = self.inner.input_frames_next();
        if self.pending.len() < need {
            return 0;
        }
        self.input_buf[0].clear();
        self.input_buf[0].extend_from_slice(&self.pending[..need]);
        self.pending.drain(..need);
        match self
            .inner
            .process_into_buffer(&self.input_buf, &mut self.output_buf, None)
        {
            Ok((_, frames_out)) => {
                let take = frames_out.min(out.len());
                out[..take].copy_from_slice(&self.output_buf[0][..take]);
                take
            }
            Err(_) => 0,
        }
    }

    fn reset(&mut self) {
        self.pending.clear();
    }
}

/// Attempt to load an ONNX session at `path`. Returns `None` on any
/// failure (missing file, missing runtime DLL, model arch mismatch).
///
/// ### Why the up-front presence checks matter
///
/// We build `ort` with the `load-dynamic` feature, which resolves the
/// ONNX Runtime shared library lazily on the FIRST call into the API
/// (`Session::builder()`). On a machine without `onnxruntime.dll` —
/// every CI runner, every test process, and any user who hasn't
/// installed the runtime — that first call can **block** rather than
/// return an error, hanging whatever thread touched it (including the
/// audio thread). So we never call into `ort` at all unless BOTH the
/// model file and the runtime dylib are physically present on disk.
/// With either missing we return `None` and the effect stays in
/// passthrough — no `ort` symbols are ever exercised.
fn load_session(path: &Path) -> Option<Session> {
    // Cheap filesystem gate first. No model file → passthrough, and
    // critically, no `ort` call (keeps tests + CI from ever loading
    // the runtime).
    if !path.exists() {
        return None;
    }
    // No runtime dylib we can point at → passthrough. Guarding here
    // means `Session::builder()` is only ever reached when a real
    // `onnxruntime.dll` exists, so the lazy load can't hang us.
    if !onnx_runtime_present() {
        return None;
    }
    let builder = Session::builder().ok()?;
    let mut builder = builder
        .with_optimization_level(GraphOptimizationLevel::Level3)
        .ok()?;
    builder.commit_from_file(path).ok()
}

/// Whether an ONNX Runtime shared library is locatable on this machine.
///
/// `load-dynamic` resolves the runtime from `ORT_DYLIB_PATH` if set,
/// otherwise from a platform-default filename next to the executable.
/// We probe exactly those locations so we never hand `ort` a load it
/// can't satisfy (which on some platforms hangs instead of erroring).
///
/// Cached after the first probe — the answer can't change within a
/// process lifetime, and we never want to re-walk the filesystem on
/// the audio thread.
/// Public probe of [`onnx_runtime_present`] for the Tauri layer, so the
/// Settings → Voice library panel can tell the user whether voice
/// conversion can actually run on this machine.
#[must_use]
pub fn onnx_runtime_available() -> bool {
    onnx_runtime_present()
}

fn onnx_runtime_present() -> bool {
    static PRESENT: OnceLock<bool> = OnceLock::new();
    *PRESENT.get_or_init(|| {
        if let Ok(p) = std::env::var("ORT_DYLIB_PATH") {
            if !p.is_empty() && Path::new(&p).exists() {
                return true;
            }
        }
        let name = if cfg!(target_os = "windows") {
            "onnxruntime.dll"
        } else if cfg!(target_os = "macos") {
            "libonnxruntime.dylib"
        } else {
            "libonnxruntime.so"
        };
        if let Ok(exe) = std::env::current_exe() {
            if let Some(dir) = exe.parent() {
                if dir.join(name).exists() {
                    return true;
                }
            }
        }
        false
    })
}

/// Run a single chunk of 16 kHz audio through the loaded model.
///
/// Returns the model's `CHUNK_16K`-length output. On failure (or no
/// session), echoes the input — the streaming-merge logic still keeps
/// the wet/dry buffers aligned.
///
/// **Important**: the actual input/output tensor names + shapes are
/// model-specific. For LLVC the model exposes `audio` (1 × N) → `audio`
/// (1 × N). This implementation tries that shape; if it doesn't match
/// the loaded model, the inference fails and we echo. v0.12.x patches
/// will firm up the supported model contract once we bundle one.
fn run_inference(session: Option<&mut Session>, chunk: &[f32]) -> Vec<f32> {
    let Some(session) = session else {
        return chunk.to_vec();
    };
    // Try the LLVC streaming shape: `(1, N)` f32 audio.
    let shape = (1_usize, chunk.len());
    let Ok(tensor) = ndarray::Array2::from_shape_vec(shape, chunk.to_vec()) else {
        return chunk.to_vec();
    };
    let Ok(value) = ort::value::Tensor::from_array(tensor) else {
        return chunk.to_vec();
    };
    let inputs = ort::inputs!["audio" => value];
    let Ok(outputs) = session.run(inputs) else {
        return chunk.to_vec();
    };
    // First output, named `audio` per the LLVC convention.
    let Some((_, value)) = outputs.iter().next() else {
        return chunk.to_vec();
    };
    let Ok(t) = value.try_extract_array::<f32>() else {
        return chunk.to_vec();
    };
    t.iter().copied().collect()
}

#[cfg(test)]
#[allow(clippy::float_cmp)] // bypass paths are bit-exact passthroughs
mod tests {
    use super::{AudioEffect, EffectKind, VoiceConverter, NATIVE_RATE};

    fn run(vc: &mut VoiceConverter, buf: &mut [f32]) {
        vc.process(buf, NATIVE_RATE);
    }

    #[test]
    fn kind_is_voice_convert() {
        let vc = VoiceConverter::new();
        assert_eq!(vc.kind(), EffectKind::VoiceConvert);
    }

    #[test]
    fn defaults_to_disabled() {
        let vc = VoiceConverter::new();
        assert!(!vc.enabled());
        assert!(vc.voice().is_none());
        assert!(!vc.is_model_loaded());
    }

    #[test]
    fn passthrough_when_disabled() {
        let mut vc = VoiceConverter::new();
        let mut buf = [0.5_f32; 64];
        let before = buf;
        run(&mut vc, &mut buf);
        // Disabled bypass is bit-exact: the engine writes nothing
        // and the upstream sample stays in place.
        assert_eq!(buf, before);
    }

    #[test]
    fn passthrough_when_no_model_even_if_enabled() {
        let mut vc = VoiceConverter::new();
        vc.set_enabled(true);
        let mut buf = [0.5_f32; 64];
        let before = buf;
        run(&mut vc, &mut buf);
        // No model → bypass path → buffer untouched.
        assert_eq!(buf, before);
    }

    #[test]
    fn set_model_path_to_none_keeps_passthrough() {
        let mut vc = VoiceConverter::new();
        vc.set_enabled(true);
        vc.set_model_path(None, None);
        let mut buf = [0.5_f32; 64];
        let before = buf;
        run(&mut vc, &mut buf);
        assert_eq!(buf, before);
        assert!(!vc.is_model_loaded());
    }

    #[test]
    fn missing_file_does_not_panic() {
        let mut vc = VoiceConverter::new();
        vc.set_enabled(true);
        vc.set_model_path(
            Some("does-not-exist".into()),
            Some(std::path::PathBuf::from("/tmp/does-not-exist.onnx")),
        );
        let mut buf = [0.5_f32; 64];
        // No panic; the effect just stays in passthrough.
        run(&mut vc, &mut buf);
        assert!(!vc.is_model_loaded());
    }

    #[test]
    fn off_rate_input_bypasses_without_buffering() {
        let mut vc = VoiceConverter::new();
        vc.set_enabled(true);
        let mut buf = [0.5_f32; 64];
        let before = buf;
        vc.process(&mut buf, 44_100);
        assert_eq!(buf, before);
    }

    #[test]
    fn mix_param_clamps_to_0_to_1_internal() {
        let mut vc = VoiceConverter::new();
        vc.set_param("mix", 50.0);
        // Indirect check: at very-low mix we early-bail in process()
        vc.set_enabled(true);
        let mut buf = [0.5_f32; 64];
        vc.process(&mut buf, NATIVE_RATE);
        // No model, so we bypass anyway. Just verify no panic on the
        // clamp path.
        vc.set_param("mix", 9999.0);
        vc.set_param("mix", -50.0);
        vc.process(&mut buf, NATIVE_RATE);
    }

    #[test]
    fn set_enabled_false_clears_pipeline() {
        let mut vc = VoiceConverter::new();
        vc.set_enabled(true);
        // Toggle off — internal buffers should be cleared via the
        // public path; no panic.
        vc.set_enabled(false);
        assert!(!vc.enabled());
    }

    #[test]
    fn set_resource_model_derives_voice_name_from_file_stem() {
        let mut vc = VoiceConverter::new();
        vc.set_resource("model", Some("/voices/deep-narrator.onnx"));
        assert_eq!(vc.voice(), Some("deep-narrator"));
    }

    #[test]
    fn set_resource_model_none_clears_voice() {
        let mut vc = VoiceConverter::new();
        vc.set_resource("model", Some("/voices/whisper.onnx"));
        assert_eq!(vc.voice(), Some("whisper"));
        vc.set_resource("model", None);
        assert!(vc.voice().is_none());
    }

    #[test]
    fn set_resource_empty_string_clears_voice() {
        let mut vc = VoiceConverter::new();
        vc.set_resource("model", Some("/voices/whisper.onnx"));
        vc.set_resource("model", Some(""));
        assert!(vc.voice().is_none());
    }

    #[test]
    fn set_resource_unknown_key_is_a_no_op() {
        let mut vc = VoiceConverter::new();
        vc.set_resource("model", Some("/voices/sage.onnx"));
        vc.set_resource("nonsense", Some("/voices/other.onnx"));
        // The bogus key must not change the active voice.
        assert_eq!(vc.voice(), Some("sage"));
    }

    #[test]
    fn set_resource_missing_file_stays_passthrough_and_never_hangs() {
        let mut vc = VoiceConverter::new();
        vc.set_enabled(true);
        vc.set_resource("model", Some("/tmp/does-not-exist.onnx"));
        // Spin a few callbacks so the background loader thread (which
        // returns None for a missing file) has time to land. The poll
        // must never block; the effect stays in passthrough.
        let mut buf = [0.25_f32; 64];
        for _ in 0..50 {
            vc.process(&mut buf, NATIVE_RATE);
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        assert!(!vc.is_model_loaded());
    }
}
