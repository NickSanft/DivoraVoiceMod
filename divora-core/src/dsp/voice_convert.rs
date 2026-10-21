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
//! ### Streaming vs. non-streaming models (v1.3.0)
//!
//! A loaded model is detected as **streaming** when it exposes the LLVC
//! cache-tensor contract (an `enc_buf` input). Streaming models run tiny
//! 208-sample chunks (≈ 13 ms) and thread four cache tensors + a
//! front-context between calls, so latency is conversational: with the
//! resamplers' lookahead the voice comes through 18.4 ms late
//! ([`hold_frames`]), plus whatever the model adds inside itself.
//! Models that only expose the single `audio` → `output` contract fall
//! back to the original non-streaming path: one 4096-sample chunk
//! (≈ 256 ms) converted statelessly per call, 261.4 ms through.
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
//! directory (`%APPDATA%/com.divora.voicemod/voices/` on Windows). This
//! effect doesn't know about that path — it is handed in as a
//! [`VoiceModel`], built by the control thread that heard the UI change
//! the active voice. That thread derives the name and starts the ONNX
//! load; the audio thread's whole part is moving the new model in and
//! handing the old one back to be freed elsewhere. The session itself
//! lands on a later `process`, through `poll_loader`.

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

// The one resampler wrapper, shared with the engine's device-rate bridge. This
// module used to keep a private copy "so it compiles without a circular
// dependency" — modules within a crate may reference each other freely, so
// there was nothing to avoid, and the copy diverged exactly as duplicates do:
// it kept the fixed-chunk shape whose overshoot handling was the v1.51.2
// defect, discarding the excess outright rather than splicing it anywhere.
use crate::audio::MonoResampler;

use super::{AudioEffect, Displaced, EffectKind, Prepared};

/// The `DspCommand::SetResource` key that carries a voice-model path.
/// `DspEdit::prepare` matches on it to know a `VoiceModel` is what to build.
pub const MODEL_RESOURCE_KEY: &str = "model";

/// Sample rate the model was trained at. Off-rate input bypasses.
pub const MODEL_RATE: u32 = 16_000;
/// Native rate the rest of the engine runs at.
pub const NATIVE_RATE: u32 = 48_000;
/// Inference chunk size in 16 kHz samples (≈ 256 ms) for a NON-streaming
/// model: the whole chunk is converted in one stateless call. Used as a
/// fallback for models that only expose the single-tensor `audio` →
/// `output` contract. Also the most one downsampling round writes.
pub const CHUNK_16K: usize = 4096;

/// The most native frames one upsampling round writes, 16 kHz → native. A
/// ceiling, not a step: like the downsampler, the upsampler converts all the
/// input it has every callback (see [`VoiceConverter::upsample_queued`]).
pub const UP_CHUNK: usize = 256;

/// Native frames the converter delays the voice by, with an inference chunk of
/// `chunk_16k` samples at 16 kHz: what [`AudioEffect::latency_samples`]
/// reports, and how far [`VoiceConverter::run_phases`] holds both the wet and
/// the dry signal back.
///
/// The sample that waits longest is the first of an inference chunk: it waits
/// for the rest of the chunk, and both resamplers read
/// [`MonoResampler::LOOKAHEAD`] frames past the point they take a sample at —
/// the downsampler at 48 kHz, the upsampler at 16 kHz, where each frame is three
/// of ours. Holding the output back by that much means every converted sample
/// is ready by the time its slot comes round, whatever the device's buffer
/// size, so the delay is exactly this and never creeps. 883 frames (18.4 ms)
/// for a streaming model, 12 547 (261.4 ms) for a non-streaming one.
#[must_use]
pub const fn hold_frames(chunk_16k: usize) -> usize {
    let ratio = (NATIVE_RATE / MODEL_RATE) as usize;
    // The 1: rubato sums a round's input need in f32 and rounds it up, so an
    // upsampling round can ask for one 16 kHz sample more than it reads — and
    // at a chunk's end that one is the next chunk's, three of our frames later.
    ratio * (chunk_16k + MonoResampler::LOOKAHEAD + 1) + MonoResampler::LOOKAHEAD
}

// ---- v1.3.0: streaming LLVC contract -------------------------------------
//
// The streaming export threads four cache tensors between successive
// chunks and consumes a `2·L` front-context prepended to each chunk. The
// shapes below are fixed by `export_streaming_onnx.py` (validated ONNX vs
// PyTorch max|diff| ≈ 2e-7) and documented in `docs/STABLE-SURFACE.md`.
// A model is treated as streaming when its inputs include `enc_buf`.

/// Streaming inference chunk (`dec_chunk_size 13 · L 16`) ≈ 13 ms @ 16 kHz.
pub const STREAM_CHUNK_16K: usize = 208;
/// Front-context (`2·L`) prepended to each chunk for continuity.
const STREAM_FRONT: usize = 32;
/// Total model input length per chunk (front + chunk).
const STREAM_IN: usize = STREAM_FRONT + STREAM_CHUNK_16K; // 240
const ENC_BUF_SHAPE: [usize; 3] = [1, 512, 510];
const DEC_BUF_SHAPE: [usize; 4] = [1, 2, 13, 256];
const OUT_BUF_SHAPE: [usize; 3] = [1, 512, 4];
const CPC_SHAPE: [usize; 3] = [1, 1, 24];

/// Streaming cache state threaded between chunks. All tensors start at
/// zero (verified equivalent to the model's `None`-init path); `front`
/// holds the previous chunk's tail for the lookahead context.
struct StreamCaches {
    enc_buf: Vec<f32>,
    dec_buf: Vec<f32>,
    out_buf: Vec<f32>,
    cpc: Vec<f32>,
    front: Vec<f32>,
}

impl StreamCaches {
    /// Zero every tensor **without reallocating**.
    ///
    /// `zeros()` builds five `Vec`s, and `clear_pipeline` — which resets the
    /// caches — is reachable from `process`: a bypassed callback with buffered
    /// state clears the pipeline, so building fresh ones there would allocate
    /// (and free the old ones) in the audio callback.
    fn zero_in_place(&mut self) {
        for cache in [
            &mut self.enc_buf,
            &mut self.dec_buf,
            &mut self.out_buf,
            &mut self.cpc,
            &mut self.front,
        ] {
            cache.fill(0.0);
        }
    }

    fn zeros() -> Self {
        Self {
            enc_buf: vec![0.0; ENC_BUF_SHAPE.iter().product()],
            dec_buf: vec![0.0; DEC_BUF_SHAPE.iter().product()],
            out_buf: vec![0.0; OUT_BUF_SHAPE.iter().product()],
            cpc: vec![0.0; CPC_SHAPE.iter().product()],
            front: vec![0.0; STREAM_FRONT],
        }
    }
}

/// A voice-conversion model: which file, the load still in flight, and the
/// session once it has arrived.
///
/// One struct so that changing voice is a single `mem::replace` — the audio
/// thread swaps the new model in and hands this whole thing back for another
/// thread to drop. Freeing it means dropping an ONNX session, which is far
/// too much work for a callback (see [`Displaced`]).
///
/// Built by [`VoiceModel::start`] on a control thread, which is also where
/// the loader thread is spawned.
pub struct VoiceModel {
    /// Voice name shown in the UI, derived from the file stem. `None` means
    /// passthrough.
    voice: Option<String>,
    /// Path of the model, used to detect redundant reloads of the same file.
    path: Option<PathBuf>,
    /// Receiver for the background load. `Session::builder` can be slow
    /// (graph optimization) or, with a missing runtime DLL, can even block,
    /// so it never runs on the audio thread; `process` polls this with
    /// `try_recv` until the session arrives.
    ///
    /// Kept even once the load has landed: dropping the receiver frees the
    /// channel, and `poll_loader` runs on the audio thread. It goes when the
    /// whole model is retired, on the graveyard thread.
    loader: Option<mpsc::Receiver<Option<Box<Session>>>>,
    /// Whether `loader` may still produce a session — cleared on the value or
    /// the hang-up, so a settled load costs nothing per callback.
    loading: bool,
    /// ONNX session — `None` when no model is loaded or the load failed.
    /// Boxed so retiring one is a pointer move, not a copy of the session.
    session: Option<Box<Session>>,
    /// v1.3.0: true when the loaded model exposes the streaming
    /// cache-tensor contract (detected at load by an `enc_buf` input).
    streaming: bool,
}

impl VoiceModel {
    /// The empty model: passthrough, nothing loading.
    #[must_use]
    pub const fn none() -> Self {
        Self {
            voice: None,
            path: None,
            loader: None,
            loading: false,
            session: None,
            streaming: false,
        }
    }

    /// Name this model after `path` and **start loading it**, on a thread of
    /// its own.
    ///
    /// Call from a control thread; the point of the type is that the audio
    /// thread gets handed the result of this instead of doing it. An empty or
    /// absent path is the passthrough model, and spawns nothing.
    #[must_use]
    pub fn start(path: Option<String>) -> Self {
        let path = path.filter(|s| !s.is_empty()).map(PathBuf::from);
        let voice = path
            .as_ref()
            .and_then(|p| p.file_stem())
            .map(|s| s.to_string_lossy().into_owned());
        let loader = path.as_ref().map(|p| {
            let (tx, rx) = mpsc::channel();
            let p = p.clone();
            // Detached: if the user switches again before it finishes, the
            // receiver is retired with the old model and the send becomes a
            // no-op (rx hung up).
            std::thread::spawn(move || {
                let _ = tx.send(load_session(&p).map(Box::new));
            });
            rx
        });
        let loading = loader.is_some();
        Self {
            voice,
            path,
            loader,
            loading,
            session: None,
            streaming: false,
        }
    }
}

/// In-place voice converter. Holds optional ONNX session, optional
/// resamplers, and the streaming buffers that bridge native-rate audio
/// callbacks to fixed-rate model chunks.
pub struct VoiceConverter {
    enabled: bool,
    mix: f32,
    /// The model in use, swapped whole by [`VoiceConverter::install`].
    model: VoiceModel,
    /// 48 kHz → 16 kHz pre-inference resampler.
    ///
    /// Built in [`VoiceConverter::new`], i.e. on whichever control thread
    /// `DspEdit::prepare` runs on — never in the callback. Until v1.51.2 these
    /// were built lazily from `process`, and because a `SetChain` hands the
    /// callback a fresh converter with both slots empty, the pair was rebuilt
    /// *there* after every preset switch: 1060 allocations, 784 frees and
    /// ~0.84 ms inside a callback with a few milliseconds of budget — eight
    /// times the cost of the chain rebuild v1.51.1 moved off the thread.
    ///
    /// `Option` only because construction can in principle fail; it cannot in
    /// practice, since every parameter is a constant. `None` means passthrough
    /// for the life of this instance, and nothing tries to fill it later.
    down: Option<MonoResampler>,
    /// 16 kHz → 48 kHz post-inference resampler. Built with [`Self::down`].
    up: Option<MonoResampler>,
    /// Contiguous scratch for moving queued samples into a resampler, and for
    /// taking its output back out. Sized once, in `new`; the phases reuse them
    /// rather than allocating a batch and an output buffer per round.
    batch: Vec<f32>,
    resampled: Vec<f32>,
    /// Scratch for one inference chunk. Separate from [`Self::batch`] on
    /// purpose: `drain_chunks` runs BETWEEN the two resampling phases, and one
    /// buffer serving three uses in sequence is a correct-today arrangement
    /// that a later edit would quietly break.
    chunk_scratch: Vec<f32>,
    /// Native-rate input samples queued for downsampling.
    native_in: VecDeque<f32>,
    /// 16 kHz samples queued for the next inference chunk.
    chunk_in: VecDeque<f32>,
    /// 16 kHz samples produced by inference, queued for upsampling.
    chunk_out: VecDeque<f32>,
    /// Native-rate converted samples queued for the engine, behind the
    /// [`hold_frames`] of silence it starts with.
    native_out: VecDeque<f32>,
    /// Dry copy of native-rate input, delayed by the same hold so the wet/dry
    /// mix lines up.
    dry_delay: VecDeque<f32>,
    /// Whether the two output queues have had their hold put in front of them
    /// since the pipeline last cleared. Also what marks the pipeline as holding
    /// anything at all.
    primed: bool,
    /// Output samples written with no converted sample ready — 0 unless the
    /// hold is too short. Diagnostic only; never read by the callback.
    starved: u64,
    /// v1.3.0: streaming cache state, threaded between chunks. Reset to
    /// zero on (re)load and whenever the pipeline clears.
    caches: StreamCaches,
}

impl VoiceConverter {
    #[must_use]
    pub fn new() -> Self {
        // NATIVE_RATE and MODEL_RATE, not parameters: `process` bypasses at any
        // other input rate (see its `bypass` condition), so this pair is only
        // ever valid for one rate pair and there is nothing left to decide at
        // runtime. That is what lets them be built here, off the audio thread.
        let down = MonoResampler::new(NATIVE_RATE, MODEL_RATE, CHUNK_16K).ok();
        let up = MonoResampler::new(MODEL_RATE, NATIVE_RATE, UP_CHUNK).ok();
        let batch_len = down
            .as_ref()
            .map_or(0, MonoResampler::max_input_frames)
            .max(up.as_ref().map_or(0, MonoResampler::max_input_frames));
        Self {
            enabled: false,
            mix: 1.0,
            model: VoiceModel::none(),
            down,
            up,
            batch: vec![0.0; batch_len],
            resampled: vec![0.0; CHUNK_16K.max(UP_CHUNK)],
            chunk_scratch: vec![0.0; CHUNK_16K.max(STREAM_CHUNK_16K)],
            native_in: VecDeque::with_capacity(NATIVE_RATE as usize),
            chunk_in: VecDeque::with_capacity(CHUNK_16K * 2),
            chunk_out: VecDeque::with_capacity(CHUNK_16K * 2),
            // A second at 48 kHz: the non-streaming hold (12 547) plus the
            // largest buffer the engine hands the chain (4096) is well inside
            // it, so priming and queueing never reallocate in the callback.
            native_out: VecDeque::with_capacity(NATIVE_RATE as usize),
            dry_delay: VecDeque::with_capacity(NATIVE_RATE as usize),
            primed: false,
            starved: 0,
            caches: StreamCaches::zeros(),
        }
    }

    /// Drain `native_in` through the downsampler into `chunk_in`: everything
    /// the queued input can make (see [`Self::resample_rounds`]).
    ///
    /// Allocation-free: the batch and the output both live in scratch sized in
    /// [`Self::new`]. Its own method so a test can drive it on its own.
    pub(crate) fn downsample_queued(&mut self) {
        Self::resample_rounds(
            self.down.as_mut(),
            &mut self.native_in,
            &mut self.chunk_in,
            &mut self.batch,
            &mut self.resampled,
            CHUNK_16K,
        );
    }

    /// Drain `chunk_out` through the upsampler into `native_out`. See
    /// [`Self::downsample_queued`].
    pub(crate) fn upsample_queued(&mut self) {
        Self::resample_rounds(
            self.up.as_mut(),
            &mut self.chunk_out,
            &mut self.native_out,
            &mut self.batch,
            &mut self.resampled,
            UP_CHUNK,
        );
    }

    /// One direction's worth of rounds: everything the queued input can make,
    /// up to `max_out` frames a round. An associated function taking the
    /// pieces rather than `&mut self`, because both phases need two different
    /// queues of the same struct mutably at once.
    ///
    /// Each round is sized to what is queued — [`MonoResampler::prepare_within`]
    /// with the queue's length as the budget — so a callback converts all its
    /// input bar the resampler's lookahead and a frame's fraction. Through v1.56.0
    /// the downsampler ran whole rounds of `CHUNK_16K`: it put out nothing
    /// until 4096 samples' worth (about 12 300 native) were queued, and then
    /// all of them at once. That held a streaming model's voice for about a
    /// quarter-second instead of the 13 ms its chunk takes, ran the ~20
    /// inferences a round feeds back to back in one callback (101–115 ms, one
    /// callback in 26, with the bundled model), and left the chunk's remainder
    /// waiting a whole extra round, so the output ran dry mid-stream and the
    /// delay crept up to 272–277 ms.
    fn resample_rounds(
        resampler: Option<&mut MonoResampler>,
        input: &mut VecDeque<f32>,
        output: &mut VecDeque<f32>,
        batch: &mut [f32],
        scratch: &mut [f32],
        max_out: usize,
    ) {
        let Some(r) = resampler else {
            return;
        };
        loop {
            let need = r.prepare_within(max_out, input.len().min(batch.len()));
            if need == 0 || input.len() < need {
                break;
            }
            for slot in &mut batch[..need] {
                *slot = input.pop_front().unwrap_or(0.0);
            }
            let wrote = r.run(batch, need, &mut scratch[..max_out]);
            if wrote == 0 {
                break;
            }
            output.extend(scratch[..wrote].iter().copied());
        }
    }

    /// Poll the background loader (if any). Cheap `try_recv`; called at
    /// the top of every `process`.
    fn poll_loader(&mut self) {
        let Some(rx) = self.model.loader.as_ref() else {
            return;
        };
        match rx.try_recv() {
            Ok(session) => {
                self.model.streaming = session.as_deref().is_some_and(session_is_streaming);
                self.model.session = session;
                self.model.loading = false;
                // Fresh model → fresh streaming state. In place: this runs on
                // the audio thread.
                self.caches.zero_in_place();
            }
            Err(mpsc::TryRecvError::Empty) => {
                // Still loading; stay in passthrough this callback.
            }
            Err(mpsc::TryRecvError::Disconnected) => {
                // Loader thread vanished without a value — give up. The
                // receiver stays until the model is retired; dropping it here
                // would free the channel on the audio thread.
                self.model.loading = false;
            }
        }
    }

    /// Name of the active voice, or `None` if passthrough.
    #[must_use]
    pub fn voice(&self) -> Option<&str> {
        self.model.voice.as_deref()
    }

    /// Whether an ONNX session is currently loaded and ready.
    #[must_use]
    pub fn is_model_loaded(&self) -> bool {
        self.model.session.is_some()
    }

    /// Samples per inference call: 208 for a streaming model, 4096 for a
    /// non-streaming one.
    fn chunk_len(&self) -> usize {
        if self.model.streaming {
            STREAM_CHUNK_16K
        } else {
            CHUNK_16K
        }
    }

    /// The [`hold_frames`] for this model: both what `run_phases` primes its
    /// outputs with and what `latency_samples` reports, so the two cannot
    /// part.
    fn hold(&self) -> usize {
        hold_frames(self.chunk_len())
    }

    /// Whether `process` converts at `sample_rate` rather than leaving the
    /// buffer alone. [`AudioEffect::latency_samples`] asks the same question,
    /// so the delay it reports is the one `process` adds, and none when it
    /// adds none.
    ///
    /// The resamplers were built in `new`, on a control thread; if either is
    /// missing this effect is passthrough for good, and the callback does not
    /// build one.
    fn engaged(&self, sample_rate: u32) -> bool {
        self.enabled
            && self.model.session.is_some()
            && sample_rate == NATIVE_RATE
            && self.mix >= 1e-4
            && self.down.is_some()
            && self.up.is_some()
    }

    fn clear_pipeline(&mut self) {
        self.native_in.clear();
        self.chunk_in.clear();
        self.chunk_out.clear();
        self.native_out.clear();
        self.dry_delay.clear();
        self.primed = false;
        // Reset streaming cache + lookahead so re-engagement starts clean.
        // In place: `process` reaches this when it bypasses with buffered
        // state, and fresh `Vec`s there would allocate in the callback.
        self.caches.zero_in_place();
        if let Some(d) = self.down.as_mut() {
            d.reset();
        }
        if let Some(u) = self.up.as_mut() {
            u.reset();
        }
    }

    /// Drain `chunk_in` whenever a full inference chunk has accumulated;
    /// run inference; push the converted samples onto `chunk_out`. The
    /// chunk size + inference path depend on whether the loaded model is
    /// streaming (small, stateful chunks) or non-streaming (one big
    /// stateless chunk).
    fn drain_chunks(
        &mut self,
        infer: &mut impl FnMut(&mut VoiceModel, &mut StreamCaches, &[f32], &mut VecDeque<f32>),
    ) {
        let chunk_len = self.chunk_len();
        while self.chunk_in.len() >= chunk_len {
            // Reuse the scratch rather than allocating a chunk per round.
            debug_assert!(chunk_len <= self.chunk_scratch.len());
            if chunk_len > self.chunk_scratch.len() {
                return;
            }
            let chunk = &mut self.chunk_scratch[..chunk_len];
            for slot in chunk.iter_mut() {
                *slot = self.chunk_in.pop_front().unwrap_or(0.0);
            }
            // Disjoint borrows of the model, the caches, the scratch and the
            // output queue.
            infer(
                &mut self.model,
                &mut self.caches,
                chunk,
                &mut self.chunk_out,
            );
        }
    }

    /// Phases 1–5 of [`AudioEffect::process`], for a converter that is not
    /// bypassed, with the inference step passed in.
    ///
    /// `process` passes [`convert_chunk`], the ONNX call. It is a parameter so
    /// a test can drive this, the real phase order, with a stand-in: CI has no
    /// ONNX runtime, so without one `process` never gets past its bypass.
    fn run_phases(
        &mut self,
        buffer: &mut [f32],
        mut infer: impl FnMut(&mut VoiceModel, &mut StreamCaches, &[f32], &mut VecDeque<f32>),
    ) {
        // Phase 0, on engaging: put the hold in front of both outputs. The
        // wet path's first sample is then due out `hold_frames` after it
        // went in, the longest any sample can take to come through, so none
        // is ever late and the delay never moves; the dry copy waits the
        // same, so the two line up at any mix. Within capacity: no allocation.
        if !self.primed {
            let hold = self.hold();
            self.native_out.extend(std::iter::repeat_n(0.0, hold));
            self.dry_delay.extend(std::iter::repeat_n(0.0, hold));
            self.primed = true;
        }

        // Phase 1: queue native input + delayed dry copy.
        for &s in buffer.iter() {
            self.native_in.push_back(s);
            self.dry_delay.push_back(s);
        }

        // Phase 2: downsample as many 16k samples as we can.
        self.downsample_queued();

        // Phase 3: run inference on any complete 16k chunks.
        self.drain_chunks(&mut infer);

        // Phase 4: upsample 16k → native and queue to `native_out`.
        self.upsample_queued();

        // Phase 5: write `native_out` (wet) + `dry_delay` (dry) back into the
        // output buffer with the wet/dry mix. Both start with the hold's
        // silence, so the first `hold_frames` out are silent at any mix and
        // the dry copy is exactly as late as the wet.
        //
        // Through v1.56.0 there was no hold: until the wet queue first filled,
        // this popped the dry copy and wrote silence in its place, so the dry
        // copy was never delayed at all — at the default 90 % mix the
        // unconverted voice played about a quarter-second ahead of the
        // converted one, 20 dB down.
        for slot in buffer.iter_mut() {
            let dry = self.dry_delay.pop_front().unwrap_or(0.0);
            let wet = if let Some(wet) = self.native_out.pop_front() {
                wet
            } else {
                self.starved = self.starved.saturating_add(1);
                0.0
            };
            *slot = self.mix * wet + (1.0 - self.mix) * dry;
        }
    }
}

/// The inference step [`VoiceConverter::process`] hands to
/// [`VoiceConverter::run_phases`]: one chunk through the loaded model, the
/// converted samples queued on `out`.
fn convert_chunk(
    model: &mut VoiceModel,
    caches: &mut StreamCaches,
    chunk: &[f32],
    out: &mut VecDeque<f32>,
) {
    let converted = if model.streaming {
        run_inference_streaming(model.session.as_deref_mut(), caches, chunk)
    } else {
        run_inference(model.session.as_deref_mut(), chunk)
    };
    out.extend(converted);
}

impl Default for VoiceConverter {
    fn default() -> Self {
        Self::new()
    }
}

impl AudioEffect for VoiceConverter {
    fn process(&mut self, buffer: &mut [f32], sample_rate: u32) {
        // Pick up a background-loaded session if one just finished.
        if self.model.loading {
            self.poll_loader();
        }

        // Bypass conditions: disabled, no model, off-rate input, no mix.
        if !self.engaged(sample_rate) {
            // Drop any stale buffered state so re-engagement is clean.
            if self.primed {
                self.clear_pipeline();
            }
            return;
        }

        self.run_phases(buffer, convert_chunk);
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

    /// Take the voice model prepared for us and hand back the one it
    /// replaces. Both are single moves; the freeing happens elsewhere.
    fn install(&mut self, resource: Prepared) -> Option<Displaced> {
        let Prepared::VoiceModel(incoming) = resource;
        // The same file arrives again routinely: the UI re-points the active
        // voice after every preset switch, because a `SetChain` builds a
        // VoiceConvert with no model. On a chain that was NOT rebuilt, taking
        // that at face value would tear a working session down and reload it
        // — a second of passthrough for no reason — so keep what we have and
        // retire the redundant load instead.
        if incoming.path.is_some() && incoming.path == self.model.path {
            return Some(Displaced::VoiceModel(incoming));
        }
        // One move each way. The old session, the load still in flight and
        // the strings naming it all leave together.
        let displaced = std::mem::replace(&mut self.model, incoming);
        // A new voice must not inherit the previous one's buffered audio.
        self.clear_pipeline();
        Some(Displaced::VoiceModel(displaced))
    }

    /// The hold `run_phases` delays the voice by ([`hold_frames`]): 883
    /// frames (18.4 ms) with a streaming model, 12 547 (261.4 ms) with a
    /// non-streaming one — and nothing whenever `process` would pass the
    /// buffer through, which includes any rate but 48 kHz. Not counted: any
    /// delay inside the model itself.
    ///
    /// Through v1.56.0 this reported one inference chunk, 13 ms for the
    /// bundled streaming model, while the downsampler's whole rounds held the
    /// voice for 272–277 ms; the Mixer readout and the screen recorder, which
    /// moves the voice back by this to meet the picture, both took the 13.
    fn latency_samples(&self, sample_rate: u32) -> usize {
        if self.engaged(sample_rate) {
            self.hold()
        } else {
            0
        }
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
/// Returns the model's converted samples. On failure (or no session),
/// echoes the input — the streaming-merge logic still keeps the wet/dry
/// buffers aligned.
///
/// ### Model I/O contract (LLVC)
///
/// We exported `KoeAI`'s LLVC `Net` (non-streaming forward) to ONNX with:
///   - input  `audio`     : f32 `[1, 1, T]` (batch, 1 channel, samples)
///   - output `audio_out` : f32 `[1, 1, T]` (same length)
///
/// `T` is dynamic but the model self-pads to a multiple of its internal
/// stride (L = 16); we always feed `CHUNK_16K` (4096), a clean multiple,
/// so no boundary padding is introduced. Each chunk is converted
/// independently (the non-streaming forward re-inits its internal
/// buffers per call); LLVC's tiny lookahead keeps seam artifacts
/// negligible at this chunk size.
///
/// If the loaded model's shape/names don't match (a different model
/// dropped into the voices folder), inference errors and we echo —
/// the effect degrades to passthrough rather than crashing the engine.
fn run_inference(session: Option<&mut Session>, chunk: &[f32]) -> Vec<f32> {
    let Some(session) = session else {
        return chunk.to_vec();
    };
    // LLVC wants `[1, 1, T]` (batch, mono channel, samples).
    let shape = (1_usize, 1_usize, chunk.len());
    let Ok(tensor) = ndarray::Array3::from_shape_vec(shape, chunk.to_vec()) else {
        return chunk.to_vec();
    };
    let Ok(value) = ort::value::Tensor::from_array(tensor) else {
        return chunk.to_vec();
    };
    let inputs = ort::inputs!["audio" => value];
    let Ok(outputs) = session.run(inputs) else {
        return chunk.to_vec();
    };
    // First (only) output — `[1, 1, T]`, flattened back to a sample run.
    let Some((_, value)) = outputs.iter().next() else {
        return chunk.to_vec();
    };
    let Ok(t) = value.try_extract_array::<f32>() else {
        return chunk.to_vec();
    };
    t.iter().copied().collect()
}

/// Whether a loaded session uses the v1.3.0 streaming contract — detected
/// by an input named `enc_buf` (the first of the four threaded caches).
fn session_is_streaming(session: &Session) -> bool {
    session.inputs().iter().any(|i| i.name() == "enc_buf")
}

/// Pull a named f32 output out of a session run as an owned `Vec`.
fn extract_named(outputs: &ort::session::SessionOutputs, name: &str) -> Option<Vec<f32>> {
    let (_, value) = outputs.iter().find(|(n, _)| *n == name)?;
    let arr = value.try_extract_array::<f32>().ok()?;
    Some(arr.iter().copied().collect())
}

/// Run one streaming chunk: prepend the front-context, feed the audio +
/// four cache tensors, collect the converted samples, and thread the
/// updated caches forward. On any failure the chunk passes through dry
/// and the caches are left untouched (so the stream stays consistent).
///
/// ### Streaming model I/O contract (LLVC, v1.3.0)
///   inputs : `audio` `[1,1,240]`, `enc_buf` `[1,512,510]`,
///            `dec_buf` `[1,2,13,256]`, `out_buf` `[1,512,4]`,
///            `convnet_pre_ctx` `[1,1,24]`
///   outputs: `output` `[1,1,208]` + the four `*_out` caches (same shapes)
fn run_inference_streaming(
    session: Option<&mut Session>,
    caches: &mut StreamCaches,
    chunk: &[f32],
) -> Vec<f32> {
    let Some(session) = session else {
        return chunk.to_vec();
    };
    if chunk.len() != STREAM_CHUNK_16K {
        return chunk.to_vec();
    }

    // audio = front-context ++ chunk → [1, 1, 240]
    let mut audio = Vec::with_capacity(STREAM_IN);
    audio.extend_from_slice(&caches.front);
    audio.extend_from_slice(chunk);

    let built = (|| {
        let audio_t = ndarray::Array3::from_shape_vec((1, 1, STREAM_IN), audio).ok()?;
        let enc_t = ndarray::Array3::from_shape_vec(
            (ENC_BUF_SHAPE[0], ENC_BUF_SHAPE[1], ENC_BUF_SHAPE[2]),
            caches.enc_buf.clone(),
        )
        .ok()?;
        let dec_t = ndarray::Array4::from_shape_vec(
            (
                DEC_BUF_SHAPE[0],
                DEC_BUF_SHAPE[1],
                DEC_BUF_SHAPE[2],
                DEC_BUF_SHAPE[3],
            ),
            caches.dec_buf.clone(),
        )
        .ok()?;
        let out_t = ndarray::Array3::from_shape_vec(
            (OUT_BUF_SHAPE[0], OUT_BUF_SHAPE[1], OUT_BUF_SHAPE[2]),
            caches.out_buf.clone(),
        )
        .ok()?;
        let cpc_t = ndarray::Array3::from_shape_vec(
            (CPC_SHAPE[0], CPC_SHAPE[1], CPC_SHAPE[2]),
            caches.cpc.clone(),
        )
        .ok()?;
        Some((
            ort::value::Tensor::from_array(audio_t).ok()?,
            ort::value::Tensor::from_array(enc_t).ok()?,
            ort::value::Tensor::from_array(dec_t).ok()?,
            ort::value::Tensor::from_array(out_t).ok()?,
            ort::value::Tensor::from_array(cpc_t).ok()?,
        ))
    })();
    let Some((audio_v, enc_v, dec_v, out_v, cpc_v)) = built else {
        return chunk.to_vec();
    };

    let inputs = ort::inputs![
        "audio" => audio_v,
        "enc_buf" => enc_v,
        "dec_buf" => dec_v,
        "out_buf" => out_v,
        "convnet_pre_ctx" => cpc_v,
    ];
    let Ok(outputs) = session.run(inputs) else {
        return chunk.to_vec();
    };

    // Extract everything first; only commit the new caches if the full
    // set is present, so a partial failure can't corrupt the stream.
    let (Some(out), Some(enc), Some(dec), Some(ob), Some(cpc)) = (
        extract_named(&outputs, "output"),
        extract_named(&outputs, "enc_buf_out"),
        extract_named(&outputs, "dec_buf_out"),
        extract_named(&outputs, "out_buf_out"),
        extract_named(&outputs, "convnet_pre_ctx_out"),
    ) else {
        return chunk.to_vec();
    };

    caches.enc_buf = enc;
    caches.dec_buf = dec;
    caches.out_buf = ob;
    caches.cpc = cpc;
    // In place: `front` is already `STREAM_FRONT` long and this runs on the
    // audio thread, so assigning a fresh `to_vec` here allocated one and freed
    // the previous one on every streaming chunk.
    caches
        .front
        .copy_from_slice(&chunk[chunk.len() - STREAM_FRONT..]);
    out
}

#[cfg(test)]
#[allow(clippy::float_cmp)] // bypass paths are bit-exact passthroughs
mod tests {
    use super::{
        hold_frames, AudioEffect, Displaced, EffectKind, MonoResampler, Prepared, VoiceConverter,
        VoiceModel, CHUNK_16K, MODEL_RATE, NATIVE_RATE, STREAM_CHUNK_16K,
    };

    fn run(vc: &mut VoiceConverter, buf: &mut [f32]) {
        vc.process(buf, NATIVE_RATE);
    }

    /// Point the effect at a model the way the engine does: prepared on this
    /// (control) thread, then installed. Returns what the change displaced.
    fn install(vc: &mut VoiceConverter, path: Option<&str>) -> Option<Displaced> {
        vc.install(Prepared::VoiceModel(VoiceModel::start(
            path.map(str::to_owned),
        )))
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
    fn installing_no_model_keeps_passthrough() {
        let mut vc = VoiceConverter::new();
        vc.set_enabled(true);
        let _ = install(&mut vc, None);
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
        let _ = install(&mut vc, Some("/tmp/does-not-exist.onnx"));
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
    fn installing_a_model_derives_the_voice_name_from_the_file_stem() {
        let mut vc = VoiceConverter::new();
        let _ = install(&mut vc, Some("/voices/deep-narrator.onnx"));
        assert_eq!(vc.voice(), Some("deep-narrator"));
    }

    #[test]
    fn installing_no_model_clears_the_voice() {
        let mut vc = VoiceConverter::new();
        let _ = install(&mut vc, Some("/voices/whisper.onnx"));
        assert_eq!(vc.voice(), Some("whisper"));
        let _ = install(&mut vc, None);
        assert!(vc.voice().is_none());
    }

    #[test]
    fn an_empty_path_clears_the_voice() {
        let mut vc = VoiceConverter::new();
        let _ = install(&mut vc, Some("/voices/whisper.onnx"));
        let _ = install(&mut vc, Some(""));
        assert!(vc.voice().is_none());
    }

    /// The model the change displaced comes back out, so the graveyard thread
    /// frees the ONNX session rather than the audio callback.
    #[test]
    fn install_hands_the_previous_model_back() {
        let mut vc = VoiceConverter::new();
        let _ = install(&mut vc, Some("/voices/whisper.onnx"));
        let displaced = install(&mut vc, Some("/voices/sage.onnx"));
        let Some(Displaced::VoiceModel(old)) = displaced else {
            panic!("the previous model must be handed back, not dropped");
        };
        assert_eq!(old.voice.as_deref(), Some("whisper"));
        assert_eq!(vc.voice(), Some("sage"));
    }

    /// The UI re-points the active voice after every preset switch, so the
    /// same path arrives again on a chain that was not rebuilt. That must not
    /// reload: it is the live session that would go away.
    #[test]
    fn installing_the_same_path_keeps_the_model_it_has() {
        let mut vc = VoiceConverter::new();
        let _ = install(&mut vc, Some("/voices/sage.onnx"));
        let displaced = install(&mut vc, Some("/voices/sage.onnx"));
        let Some(Displaced::VoiceModel(redundant)) = displaced else {
            panic!("the redundant load must be handed back");
        };
        // What came back is the INCOMING model (the one we didn't install),
        // and the effect kept the one it already had.
        assert_eq!(redundant.voice.as_deref(), Some("sage"));
        assert_eq!(vc.voice(), Some("sage"));
    }

    #[test]
    fn a_missing_file_stays_passthrough_and_never_hangs() {
        let mut vc = VoiceConverter::new();
        vc.set_enabled(true);
        let _ = install(&mut vc, Some("/tmp/does-not-exist.onnx"));
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

    // Real end-to-end check against the exported STREAMING LLVC model.
    // `#[ignore]` so it never runs on CI or a plain `cargo test` (which
    // have no model + no onnxruntime.dll). Run on a dev box that did the
    // export with:
    //
    //   cargo test -p divora-core --all-features -- --ignored \
    //       llvc_streaming_model_converts_chunks
    //
    // Points `ORT_DYLIB_PATH` at the workspace `target/debug` DLL, loads
    // the model from the app's voices dir, confirms it's detected as
    // streaming, then threads the caches through several 208-sample
    // chunks and asserts each comes back one-chunk-long, finite, and the
    // stream overall *changed*.
    #[test]
    #[ignore = "needs the exported streaming LLVC model + onnxruntime.dll installed locally"]
    // `std::env::set_var` is unsafe in edition 2024; this is a single-
    // threaded, ignored, dev-only integration test setting the dylib path
    // before any ort init — no data race possible.
    #[allow(unsafe_code)]
    fn llvc_streaming_model_converts_chunks() {
        use std::path::PathBuf;

        let home = std::env::var("USERPROFILE")
            .or_else(|_| std::env::var("HOME"))
            .expect("home dir");
        let model = PathBuf::from(&home)
            .join("AppData")
            .join("Roaming")
            .join("com.divora.voicemod")
            .join("voices")
            .join("llvc-narrator.onnx");
        if !model.exists() {
            eprintln!("skip: {} not present", model.display());
            return;
        }
        let dll = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("target")
            .join("debug")
            .join("onnxruntime.dll");
        if dll.exists() {
            // SAFETY: single-threaded test setup before any ort init.
            unsafe {
                std::env::set_var("ORT_DYLIB_PATH", &dll);
            }
        }

        let mut session = super::load_session(&model);
        assert!(
            session.is_some(),
            "streaming LLVC model should load via ort"
        );
        assert!(
            super::session_is_streaming(session.as_ref().unwrap()),
            "the bundled model must be detected as streaming"
        );

        // Thread the caches through 8 successive 208-sample chunks of a
        // continuous tone, exactly as the engine would stream them.
        let mut caches = super::StreamCaches::zeros();
        let mut total_diff = 0.0_f32;
        let mut total = 0usize;
        for k in 0..8usize {
            let base = k * super::STREAM_CHUNK_16K;
            let chunk: Vec<f32> = (0..super::STREAM_CHUNK_16K)
                .map(|i| ((base + i) as f32 * 0.05).sin() * 0.3)
                .collect();
            let out = super::run_inference_streaming(session.as_mut(), &mut caches, &chunk);
            assert_eq!(out.len(), super::STREAM_CHUNK_16K, "one chunk out");
            assert!(out.iter().all(|s| s.is_finite()), "all output finite");
            let max_abs = out.iter().fold(0.0_f32, |m, &s| m.max(s.abs()));
            assert!(max_abs <= 1.5, "no gross clipping (max {max_abs})");
            for (a, b) in out.iter().zip(&chunk) {
                total_diff += (a - b).abs();
            }
            total += out.len();
        }
        let mean = total_diff / total as f32;
        assert!(
            mean > 1e-4,
            "streaming output must differ from input (mean|d|={mean})"
        );
        eprintln!("streaming LLVC OK: mean|delta|={mean:.4}");
    }

    // ---- the resampling phases: RT-safety and correctness ------------------
    //
    // Until v1.51.2 `VoiceConverter::new` left both resamplers unbuilt and
    // `process` filled them in lazily, ON THE AUDIO THREAD. Because a
    // `SetChain` hands the callback a brand-new converter with both slots
    // empty, the pair was rebuilt there after every preset switch: measured
    // 1060 allocations, 784 frees and ~0.84 ms inside a callback with a few
    // milliseconds of budget — eight times the cost of the chain rebuild
    // v1.51.1 moved off the thread. Each round then allocated twice more, a
    // batch and an output buffer: 98 allocations and 98 frees per inference
    // chunk, deterministically — two per round, 49 rounds (one down, 48 up).
    //
    // These tests live in the crate rather than in `tests/` so they can reach
    // `pub(crate)` phases and private queues without adding test-only public
    // API to a production type. The counting allocator is per-thread, which is
    // what makes it safe in the shared test binary — see `crate::alloc_probe`.
    //
    // NOT covered here: inference. It needs a loaded ONNX session and CI has
    // no ONNX runtime, so below it is a copy standing in (`identity`). The real
    // path still allocates (ort builds tensors per call) and blocks the
    // callback for about 5 ms a chunk with the bundled streaming model — one
    // chunk about three callbacks in four at 480 frames, which a 256-frame
    // buffer can't fit (`the_bundled_model_in_the_callback`, ignored, measures
    // it where the model is installed). Taking it off the callback needs its
    // own thread and a latency-compensating queue; that is deliberately out of
    // scope, and these zeros say nothing about it.

    use crate::alloc_probe::{arm, disarm};

    /// Stand in for inference so phase 4 has input without an ONNX session:
    /// 16 kHz in becomes 16 kHz out, unchanged.
    fn pass_chunks_through(vc: &mut VoiceConverter) {
        while let Some(s) = vc.chunk_in.pop_front() {
            vc.chunk_out.push_back(s);
        }
    }

    fn queue_native(vc: &mut VoiceConverter, block: &[f32]) {
        for &s in block {
            vc.native_in.push_back(s);
        }
    }

    /// Building a converter is where the cost belongs. Measuring it also proves
    /// the constructor really does the work — a `new` that had gone lazy again
    /// would read as suspiciously cheap.
    #[test]
    fn the_resamplers_are_built_when_the_converter_is_constructed() {
        let warm = VoiceConverter::new();
        assert!(warm.down.is_some() && warm.up.is_some(), "built eagerly");
        drop(warm);

        arm();
        let vc = VoiceConverter::new();
        let (allocs, frees) = disarm();
        assert!(
            vc.down.is_some() && vc.up.is_some(),
            "a fresh converter must arrive with both resamplers ready, so the \
             audio thread never has to build one"
        );
        assert!(
            allocs > 200,
            "only {allocs} allocations to construct a converter — two 128-tap \
             sinc tables at 128x oversampling cost far more than that, so the \
             build has probably gone lazy again"
        );
        eprintln!("VoiceConverter::new: {allocs} allocations, {frees} frees");
    }

    /// The callback's half allocates and frees nothing.
    #[test]
    fn the_resampling_phases_allocate_nothing() {
        let mut vc = VoiceConverter::new();

        // Prime one round of each direction outside the window, so first-call
        // effects are not attributed to the steady state.
        queue_native(&mut vc, &vec![0.1_f32; CHUNK_16K * 4]);
        vc.downsample_queued();
        pass_chunks_through(&mut vc);
        vc.upsample_queued();
        vc.native_out.clear();

        let block = vec![0.25_f32; 480];
        let mut total = (0_usize, 0_usize);
        for _ in 0..2000 {
            // Queueing is the caller's business; what is measured is the phases.
            queue_native(&mut vc, &block);
            arm();
            vc.downsample_queued();
            let (a1, f1) = disarm();
            pass_chunks_through(&mut vc);
            arm();
            vc.upsample_queued();
            let (a2, f2) = disarm();
            total.0 += a1 + a2;
            total.1 += f1 + f2;
            // Keep the output queue from growing without bound and reallocating
            // inside a measured window for reasons that are the test's fault.
            vc.native_out.clear();
        }
        assert_eq!(
            total,
            (0, 0),
            "{} allocations and {} frees across 2000 callbacks of resampling",
            total.0,
            total.1
        );
    }

    /// And the phases are still correct: the rates come out right and a tone
    /// survives the round trip. A zero-allocation path that produced silence
    /// would pass the test above.
    #[test]
    fn the_phases_convert_at_the_right_rates_and_keep_the_signal() {
        let mut vc = VoiceConverter::new();
        let n = NATIVE_RATE as usize;
        let input: Vec<f32> = (0..n)
            .map(|i| {
                (f64::from(i as u32) * 440.0 * std::f64::consts::TAU / f64::from(NATIVE_RATE)).sin()
                    as f32
                    * 0.5
            })
            .collect();
        queue_native(&mut vc, &input);
        vc.downsample_queued();

        // One second in gives one second at the model rate, short only of the
        // downsampler's lookahead: rounds are sized to what is queued, so
        // nothing waits for a whole round to fill.
        let down = vc.chunk_in.len();
        let want = MODEL_RATE as usize;
        let short = MonoResampler::LOOKAHEAD.div_ceil(3);
        assert!(
            down <= want && down + short >= want,
            "1 s at {NATIVE_RATE} Hz gave {down} samples at {MODEL_RATE} Hz, \
             wanted {want} less at most {short}"
        );

        // And back up, short only of the upsampler's lookahead, which is three
        // of our frames for each of its.
        pass_chunks_through(&mut vc);
        vc.upsample_queued();
        let up = vc.native_out.len();
        assert!(
            up <= down * 3 && up + 3 * (MonoResampler::LOOKAHEAD + 2) >= down * 3,
            "{down} samples at {MODEL_RATE} Hz gave back {up} at {NATIVE_RATE} Hz"
        );

        let out: Vec<f32> = vc.native_out.iter().copied().collect();
        let body = &out[out.len() / 4..];
        let peak = body.iter().fold(0.0_f32, |m, &v| m.max(v.abs()));
        assert!((peak - 0.5).abs() < 0.05, "peak {peak:.4}, expected ~0.5");
        let worst = body
            .windows(2)
            .map(|w| (w[1] - w[0]).abs())
            .fold(0.0_f32, f32::max);
        assert!(
            worst < 0.1,
            "worst adjacent step {worst:.4} — discontinuous"
        );
    }

    // ---- the hold: how long the voice takes to come through ------------------
    //
    // Through v1.56.0 the downsampler ran whole rounds of `CHUNK_16K`, and
    // `latency_samples` reported one inference chunk. Driven through the phase
    // order below with a streaming model's chunk at 480-frame buffers, the old
    // pipeline put out its first converted sample after 12 000 samples (the
    // 26th buffer), ran dry for 1 056 more mid-stream, and settled at 13 056
    // (272 ms; 277 ms at 256 frames) — against a reported 624 (13 ms).
    //
    // Inference is a copy here (`identity`): CI has no ONNX runtime. So these
    // pin the pipeline's delay and nothing inside a model; the bundled model
    // adds about 5 ms of its own (`the_bundled_model_in_the_callback`).

    /// The inference stand-in: 16 kHz in becomes 16 kHz out, unchanged, and
    /// queued without allocating while `chunk_out` has room.
    fn identity(
        _: &mut VoiceModel,
        _: &mut super::StreamCaches,
        chunk: &[f32],
        out: &mut std::collections::VecDeque<f32>,
    ) {
        out.extend(chunk.iter().copied());
    }

    /// `input` through the real phase order, `frames` at a time, as the
    /// callback would hand it over.
    fn drive(vc: &mut VoiceConverter, input: &[f32], frames: usize) -> Vec<f32> {
        let mut out = Vec::with_capacity(input.len());
        let mut buf = vec![0.0_f32; frames];
        for block in input.chunks(frames) {
            let buf = &mut buf[..block.len()];
            buf.copy_from_slice(block);
            vc.run_phases(buf, identity);
            out.extend_from_slice(buf);
        }
        out
    }

    /// Low-passed noise on a DC offset, so no sample of it is exactly 0.
    fn signal(n: usize) -> Vec<f32> {
        let mut seed = 0x2545_f491_u32;
        let mut lp = [0.0_f32; 4];
        (0..n)
            .map(|_| {
                seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                let mut x = (seed >> 8) as f32 / (1u32 << 24) as f32 * 2.0 - 1.0;
                for y in &mut lp {
                    *y += 0.18 * (x - *y);
                    x = *y;
                }
                0.3 + 2.0 * x
            })
            .collect()
    }

    /// The worst difference between `out` and `input` moved `lag` later,
    /// once the resamplers' start-up ringing has passed.
    fn worst_from(input: &[f32], out: &[f32], lag: usize) -> f32 {
        out[lag + 2000..]
            .iter()
            .zip(&input[2000..])
            .map(|(o, i)| (o - i).abs())
            .fold(0.0, f32::max)
    }

    #[test]
    fn the_hold_is_18_ms_streaming_and_261_ms_not() {
        assert_eq!(hold_frames(STREAM_CHUNK_16K), 883);
        assert_eq!(hold_frames(CHUNK_16K), 12_547);
    }

    /// The downsampler holds nothing back for a round to fill: fed 480- or
    /// 256-frame buffers, it puts out the first buffer's worth bar its
    /// lookahead at once, and each buffer after that brings its own third.
    #[test]
    fn the_downsampler_puts_out_each_buffer_as_it_comes() {
        for frames in [480_usize, 256] {
            let mut vc = VoiceConverter::new();
            let block = vec![0.25_f32; frames];
            for k in 1..=100 {
                queue_native(&mut vc, &block);
                vc.downsample_queued();
                let fed = k * frames;
                assert_eq!(
                    vc.chunk_in.len(),
                    (fed - MonoResampler::LOOKAHEAD) / 3,
                    "{frames}-frame buffers: after {k} ({fed} samples in)"
                );
                assert!(
                    vc.native_in.len() < 3,
                    "{frames}-frame buffers: {} samples left over after {k}",
                    vc.native_in.len()
                );
            }
        }
    }

    /// Through the real phase order, the voice comes out exactly
    /// `hold_frames` late — what `latency_samples` reports — at every buffer
    /// size from 1 to 64 and a spread up to the engine's largest, and no
    /// output sample ever goes without a converted one to play. Over this
    /// sweep the hold has two frames to spare: one or two shorter never
    /// starved, three shorter did (at 1-, 3- and 5-frame buffers).
    #[test]
    fn the_voice_comes_out_exactly_the_hold_late_at_any_buffer_size() {
        let input = signal(NATIVE_RATE as usize * 3);
        let mut sizes: Vec<usize> = (1..=64).collect();
        sizes.extend([
            97, 128, 191, 256, 333, 441, 479, 480, 481, 512, 640, 1000, 1023, 1024, 1056, 1999,
            2048, 3001, 4096,
        ]);
        for (streaming, chunk) in [(true, STREAM_CHUNK_16K), (false, CHUNK_16K)] {
            for &frames in &sizes {
                let mut vc = VoiceConverter::new();
                vc.model.streaming = streaming;
                // What `latency_samples` reports whenever `process` runs the
                // phases; with no ONNX session here, it is asked directly.
                let hold = vc.hold();
                let out = drive(&mut vc, &input, frames);
                // The input is never exactly 0, so the first output sample
                // that isn't is the voice's first.
                assert_eq!(
                    out.iter().position(|&s| s != 0.0),
                    Some(hold),
                    "chunk {chunk}, {frames}-frame buffers: first sound out"
                );
                assert_eq!(
                    vc.starved, 0,
                    "chunk {chunk}, {frames}-frame buffers: samples out with nothing converted"
                );
                let worst = worst_from(&input, &out, hold);
                assert!(
                    worst < 0.02,
                    "chunk {chunk}, {frames}-frame buffers: off the input by up to {worst:.4} \
                     at a delay of {hold}"
                );
            }
        }
    }

    /// The dry copy waits the same hold, so at any mix the two halves line up
    /// and the output is the input, `hold` late, silent until then. Through
    /// v1.56.0 the dry copy was never delayed: at the default 90 % mix the
    /// unconverted voice ran a quarter-second ahead of the converted one.
    #[test]
    fn the_dry_copy_lines_up_with_the_wet() {
        let input = signal(NATIVE_RATE as usize);
        for (streaming, chunk) in [(true, STREAM_CHUNK_16K), (false, CHUNK_16K)] {
            for mix in [90.0, 50.0, 10.0] {
                let mut vc = VoiceConverter::new();
                vc.model.streaming = streaming;
                vc.set_param("mix", mix);
                let hold = vc.hold();
                let out = drive(&mut vc, &input, 480);
                assert!(
                    out[..hold].iter().all(|&s| s == 0.0),
                    "chunk {chunk}, mix {mix}: sound before the hold was up"
                );
                let worst = worst_from(&input, &out, hold);
                assert!(
                    worst < 0.02,
                    "chunk {chunk}, mix {mix}: off the input by up to {worst:.4}"
                );
            }
        }
    }

    /// A bypass drops the hold with everything else, so the next engagement
    /// primes again from silence rather than playing what was held.
    #[test]
    fn bypassing_drops_the_hold() {
        let mut vc = VoiceConverter::new();
        vc.model.streaming = true;
        drive(&mut vc, &signal(4800), 480);
        assert!(vc.primed && !vc.native_out.is_empty());

        // No session: `process` bypasses, and passes the buffer through.
        let mut buf = [0.5_f32; 480];
        vc.set_enabled(true);
        vc.process(&mut buf, NATIVE_RATE);
        assert_eq!(buf, [0.5_f32; 480]);
        assert!(!vc.primed);
        assert!(vc.native_out.is_empty() && vc.dry_delay.is_empty() && vc.native_in.is_empty());
        assert_eq!(vc.latency_samples(NATIVE_RATE), 0);
    }

    /// The whole phase order allocates nothing around inference, the
    /// first callback's priming included. Inference itself is the stand-in's
    /// copy into the queue here, NOT what ort does: that builds tensors on
    /// every call, and nothing in CI reaches it.
    #[test]
    fn the_phases_around_inference_allocate_nothing() {
        for streaming in [true, false] {
            let mut vc = VoiceConverter::new();
            vc.model.streaming = streaming;
            let input = signal(NATIVE_RATE as usize * 2);
            let mut buf = vec![0.0_f32; 480];
            let mut total = (0_usize, 0_usize);
            for block in input.as_chunks::<480>().0 {
                buf.copy_from_slice(block);
                arm();
                vc.run_phases(&mut buf, identity);
                let (allocs, frees) = disarm();
                total.0 += allocs;
                total.1 += frees;
            }
            assert!(vc.primed);
            assert_eq!(
                total,
                (0, 0),
                "streaming {streaming}: {} allocations and {} frees across 2 s of callbacks",
                total.0,
                total.1
            );
        }
    }

    /// Speech-shaped test input: a 140 Hz pulse train through a one-pole
    /// low-pass, in bursts of 80–300 ms with gaps of 60–250 ms.
    fn bursts(n: usize) -> Vec<f32> {
        let mut seed = 0x9e37_79b9_u32;
        let mut next = |lo: usize, hi: usize| {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            lo + (seed >> 8) as usize % (hi - lo)
        };
        let mut out = Vec::with_capacity(n);
        let mut lp = 0.0_f32;
        let mut phase = 0.0_f32;
        while out.len() < n {
            let on = next(80, 300) * 48;
            let off = next(60, 250) * 48;
            for i in 0..on {
                phase = (phase + 140.0 / 48_000.0).fract();
                let fade = (i.min(on - i) as f32 / 480.0).min(1.0);
                lp += 0.2 * ((phase * 2.0 - 1.0) - lp);
                out.push(0.4 * fade * lp);
            }
            out.extend(std::iter::repeat_n(0.0, off));
        }
        out.truncate(n);
        out
    }

    /// The delay, in ms to a quarter, at which `out`'s loudness best follows
    /// `input`'s: RMS over 12-sample hops, correlated over lags up to 600 ms.
    /// A model changes the waveform, so the waveform can't be lined up, but
    /// it keeps the bursts where they are.
    fn envelope_lag_ms(input: &[f32], out: &[f32]) -> f64 {
        let envelope = |x: &[f32]| -> Vec<f32> {
            x.as_chunks::<12>()
                .0
                .iter()
                .map(|c| (c.iter().map(|s| s * s).sum::<f32>() / 12.0).sqrt())
                .collect()
        };
        let (a, b) = (envelope(input), envelope(out));
        let mean = |v: &[f32]| v.iter().sum::<f32>() / v.len() as f32;
        let lag = (0..=2400)
            .map(|lag| {
                let (x, y) = (&a[..b.len() - lag], &b[lag..]);
                let (mx, my) = (mean(x), mean(y));
                let num: f32 = x.iter().zip(y).map(|(p, q)| (p - mx) * (q - my)).sum();
                let dx: f32 = x.iter().map(|p| (p - mx) * (p - mx)).sum();
                let dy: f32 = y.iter().map(|q| (q - my) * (q - my)).sum();
                (lag, num / (dx * dy).sqrt().max(1e-12))
            })
            .max_by(|p, q| p.1.total_cmp(&q.1))
            .map_or(0, |(lag, _)| lag);
        lag as f64 / 4.0
    }

    /// The envelope measure, checked where the answer is known: through the
    /// identity pipeline it reads the hold to within its quarter-millisecond.
    #[test]
    fn the_envelope_measure_reads_the_identity_hold() {
        let input = bursts(NATIVE_RATE as usize * 4);
        for (streaming, chunk) in [(true, STREAM_CHUNK_16K), (false, CHUNK_16K)] {
            let mut vc = VoiceConverter::new();
            vc.model.streaming = streaming;
            let hold_ms = vc.hold() as f64 / 48.0;
            let out = drive(&mut vc, &input, 480);
            let lag = envelope_lag_ms(&input, &out);
            assert!(
                (lag - hold_ms).abs() <= 0.25,
                "chunk {chunk}: envelope says {lag} ms, the hold is {hold_ms:.2}"
            );
        }
    }

    // The callback with the real bundled model: how long each `process` takes
    // against the buffer's own length, and where the converted voice lands.
    // `#[ignore]`: CI has neither the model nor the runtime. Run on a box with
    // both, pointing `ORT_DYLIB_PATH` at the runtime:
    //
    //   ORT_DYLIB_PATH=<path>/onnxruntime.dll cargo test -p divora-core \
    //       --all-features -- --ignored the_bundled_model_in_the_callback --nocapture
    //
    // Measured on the dev box, 20 s of `bursts` a run, the converter alone (no
    // other effect in the chain), test profile (opt-level 1; ort is native):
    //
    //   - through v1.56.0: at 480 frames, 78 of 2 000 callbacks over the 10 ms
    //     budget, at 101–115 ms each (one a round); at 256 frames, 78 of 3 750,
    //     up to 111 ms. The voice landed 280 ms late, `latency_samples` said 13.
    //   - now: at 480 frames, 0 of 2 000 over, worst 7.8–9.4 ms in two runs,
    //     mean 4.0–4.2 ms; at 256 frames (a 5.33 ms budget), 500–628 of 3 750
    //     over, worst 8.2 ms — one ~5 ms inference doesn't fit. The voice lands
    //     23.25 ms late: the 18.33 ms hold and about 5 ms inside the model.
    #[test]
    #[ignore = "needs the bundled streaming LLVC model + onnxruntime.dll (ORT_DYLIB_PATH)"]
    fn the_bundled_model_in_the_callback() {
        use std::time::{Duration, Instant};

        let home = std::env::var("USERPROFILE")
            .or_else(|_| std::env::var("HOME"))
            .expect("home dir");
        let model = std::path::PathBuf::from(&home)
            .join("AppData")
            .join("Roaming")
            .join("com.divora.voicemod")
            .join("voices")
            .join("llvc-narrator.onnx");
        if !model.exists() || !super::onnx_runtime_available() {
            eprintln!("skip: no model at {} or no runtime", model.display());
            return;
        }
        let input = bursts(NATIVE_RATE as usize * 20);
        for frames in [480_usize, 256] {
            let mut vc = VoiceConverter::new();
            vc.set_enabled(true);
            vc.set_param("mix", 100.0);
            let _ = install(&mut vc, model.to_str());
            let started = Instant::now();
            let mut silence = vec![0.0_f32; frames];
            while !vc.is_model_loaded() {
                vc.process(&mut silence, NATIVE_RATE);
                assert!(
                    started.elapsed() < Duration::from_secs(60),
                    "model never loaded"
                );
                std::thread::sleep(Duration::from_millis(5));
            }
            assert!(
                vc.model.streaming,
                "the bundled model is the streaming export"
            );
            let hold = vc.latency_samples(NATIVE_RATE);
            assert_eq!(hold, hold_frames(STREAM_CHUNK_16K));
            let hold_ms = hold as f64 / 48.0;

            let mut out = Vec::with_capacity(input.len());
            let mut times = Vec::with_capacity(input.len() / frames);
            for block in input.chunks_exact(frames) {
                let mut buf = block.to_vec();
                let t = Instant::now();
                vc.process(&mut buf, NATIVE_RATE);
                times.push(t.elapsed().as_secs_f64() * 1000.0);
                out.extend_from_slice(&buf);
            }
            let budget = frames as f64 / f64::from(NATIVE_RATE) * 1000.0;
            let over = times.iter().filter(|&&t| t > budget).count();
            let mean = times.iter().sum::<f64>() / times.len() as f64;
            let mut sorted = times.clone();
            sorted.sort_by(f64::total_cmp);
            let p99 = sorted[(sorted.len() - 1) * 99 / 100];
            let lag = envelope_lag_ms(&input, &out);
            eprintln!(
                "{frames} frames ({budget:.2} ms budget): {} callbacks, {over} over budget; \
                 mean {mean:.2} ms, p99 {p99:.2}, max {:.2}; the voice lands {lag} ms late, \
                 latency_samples says {hold_ms:.2}",
                times.len(),
                sorted[sorted.len() - 1],
            );
            // Timing is reported, not asserted: it moves with whatever else
            // the machine is doing. Where the voice lands does not.
            assert!(
                lag >= hold_ms - 0.25 && lag <= hold_ms + 10.0,
                "the voice lands {lag} ms late against a reported {hold_ms:.2}"
            );
        }
    }

    /// The preset-switch case specifically: a brand-new converter's FIRST
    /// resampling round must cost nothing. This is the one that used to pay
    /// 1060 allocations.
    #[test]
    fn a_fresh_converters_first_round_costs_nothing() {
        let mut vc = VoiceConverter::new();
        queue_native(&mut vc, &vec![0.2_f32; CHUNK_16K * 4]);

        arm();
        vc.downsample_queued();
        let (allocs, frees) = disarm();

        assert!(
            !vc.chunk_in.is_empty(),
            "the first round produced nothing, so this proves nothing"
        );
        assert_eq!(
            (allocs, frees),
            (0, 0),
            "a fresh converter's first resampling round cost {allocs} \
             allocations and {frees} frees — the build is back in the callback"
        );
    }
}
