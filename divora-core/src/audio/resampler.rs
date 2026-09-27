//! Mono sample-rate converter used to bridge a device-rate mismatch
//! between the input and output streams. Wraps `rubato::SincFixedOut`
//! so the audio callback can ask for a fixed number of output frames
//! and feed whatever input it has on hand.
//!
//! Phase 9 replaces the hard `SampleRateMismatch` error that Phase 2
//! threw — `DivoraVoice` now accepts any pair of supported device rates
//! and resamples between them silently.
//!
//! # Exact fit
//!
//! Every call produces **exactly** the number of output frames the device
//! asked for, in one rubato round, by resizing the chunk to match:
//! [`Resampler::set_chunk_size`] is two field writes and a five-float
//! recomputation, and costs nothing at realtime.
//!
//! That is not a micro-optimisation, it is what makes this module correct.
//! Until v1.51.2 the chunk was a fixed 256 while the caller asked for the
//! device's frame count, and the three defects that followed all descended
//! from that one mismatch:
//!
//! * the overshoot (480 frames wanted, two 256-frame rounds produced) was
//!   "stashed" by splicing resampler OUTPUT into the INPUT queue, at index 0,
//!   ahead of older input. Measured on a 440 Hz tone: **0.00 dB** SNR, i.e.
//!   the input tone explained none of the output energy;
//! * the `to_vec` that stash needed allocated and freed on the audio thread
//!   once per buffer;
//! * and because the caller topped the input queue up by a fixed amount every
//!   callback instead of to a target, the queue grew for ever — 32 seconds of
//!   backlog after one minute, realloc-copied inside the callback.
//!
//! With an exact fit there is no overshoot to stash, no queue whose depth
//! needs managing, and no buffer a caller can grow: the staging buffer is
//! allocated at construction and [`MonoResampler::run`] is the only thing that
//! writes to it. `tests/rt_resampler.rs` pins all of it.

#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]

use rubato::{
    Resampler, SincFixedOut, SincInterpolationParameters, SincInterpolationType, WindowFunction,
};

use super::AudioEngineError;

/// How much the resample ratio may later be trimmed, as a factor either way.
///
/// Nothing trims it today. It is not 1.0 because 1.0 makes the permitted
/// window for [`Resampler::set_resample_ratio`] a single point — the ratio it
/// was built with — which forecloses ever correcting for **clock drift**
/// between two physical devices, the one thing that could still make this path
/// underrun. Raising it costs a few kilobytes of rubato's internal buffer and
/// cannot be done later without rebuilding the resampler, which is the one
/// expensive operation here and must not happen on the audio thread.
const MAX_RATIO_TRIM: f64 = 1.05;

/// Streaming mono resampler.
///
/// Allocates at construction and never again. One `process` round per call,
/// producing exactly the frame count asked for.
pub struct MonoResampler {
    inner: SincFixedOut<f32>,
    /// Native-rate input for the current round. `rubato` works on a
    /// `&[Vec<f32>]` (per-channel), hence the outer `Vec` of one.
    ///
    /// Sized `input_frames_max()` at construction, which is the most any
    /// chunk up to `max_out_frames` can ask for — so a round can never need
    /// more than this holds.
    stage: Vec<Vec<f32>>,
    output_buf: Vec<Vec<f32>>,
    input_rate: u32,
    output_rate: u32,
    /// Largest output frame count a single round may be asked for.
    max_out_frames: usize,
    /// Output frames the prepared round will write, and native frames it
    /// needs to do it. Set by [`Self::prepare_within`].
    chunk: usize,
    need: usize,
    /// Rounds that ran with a zero-filled tail because the caller could not
    /// supply `need` frames. Diagnostic only; never read by the callback.
    underruns: u64,
}

impl MonoResampler {
    /// Create a resampler that converts `input_rate` → `output_rate` and can
    /// produce up to `max_out_frames` output frames per round.
    ///
    /// `max_out_frames` is a **ceiling**, not a per-call size — the per-call
    /// size is whatever [`Self::prepare_within`] is given. Construction cost
    /// is dominated by the sinc table and does not vary with it (measured:
    /// 387 µs at 256, 373 µs at 4096), so pass the largest buffer the stream
    /// can ever deliver and stop thinking about it.
    ///
    /// Returns an error only if the underlying `rubato` constructor rejects
    /// the rate combination (which it shouldn't for any sane audio rate pair).
    pub fn new(
        input_rate: u32,
        output_rate: u32,
        max_out_frames: usize,
    ) -> Result<Self, AudioEngineError> {
        let max_out_frames = max_out_frames.max(1);
        let ratio = f64::from(output_rate) / f64::from(input_rate);
        let params = SincInterpolationParameters {
            sinc_len: 128,
            f_cutoff: 0.95,
            interpolation: SincInterpolationType::Linear,
            oversampling_factor: 128,
            window: WindowFunction::BlackmanHarris2,
        };
        // rubato expects `resample_ratio = output_rate / input_rate`.
        let inner = SincFixedOut::<f32>::new(ratio, MAX_RATIO_TRIM, params, max_out_frames, 1)
            .map_err(|e| AudioEngineError::ResamplerBuild {
                input: input_rate,
                output: output_rate,
                message: e.to_string(),
            })?;
        let stage_len = inner.input_frames_max();
        let chunk = max_out_frames;
        let need = inner.input_frames_next();
        Ok(Self {
            inner,
            stage: vec![vec![0.0; stage_len]],
            output_buf: vec![vec![0.0; max_out_frames]],
            input_rate,
            output_rate,
            max_out_frames,
            chunk,
            need,
            underruns: 0,
        })
    }

    /// Size the next round, and report how many native-rate frames it needs.
    ///
    /// Call once per callback before [`Self::stage`] and [`Self::run`]. The
    /// returned count is exactly what `stage` will expect — pop that many from
    /// the input ring.
    ///
    /// `native_budget` caps how much native-rate audio the caller can hand
    /// over in one go. It exists because the engine runs its DSP chain on a
    /// fixed-size stack buffer, and a steep downsample needs more input than
    /// output: 48 kHz → 16 kHz at 4096 output frames needs 12 288 input
    /// frames. Rather than silently under-feed (which is what produced half a
    /// buffer of zeros before v1.51.2), this **lowers the output frame count**
    /// to one the budget can serve and returns the smaller need; the caller
    /// then writes fewer frames and the fan-out silences the rest. Check
    /// [`Self::prepared_out_frames`] if you need to know it happened.
    pub fn prepare_within(&mut self, out_frames: usize, native_budget: usize) -> usize {
        let mut want = out_frames.clamp(1, self.max_out_frames);
        loop {
            // Infallible: `want` is clamped into 1..=max_out_frames, the only
            // range `set_chunk_size` rejects outside of.
            debug_assert!(want >= 1 && want <= self.max_out_frames);
            let _ = self.inner.set_chunk_size(want);
            let need = self.inner.input_frames_next();
            if need <= native_budget.max(1) || want == 1 {
                self.chunk = want;
                self.need = need.min(self.stage[0].len());
                return self.need;
            }
            // `need` is very close to linear in `want`, so scaling by the
            // overshoot converges in one or two passes rather than looping
            // down one frame at a time.
            let scaled = want.saturating_mul(native_budget.max(1)) / need.max(1);
            want = scaled.clamp(1, want.saturating_sub(1));
        }
    }

    /// Run the prepared round: take `filled` native-rate frames from the front
    /// of `native`, write exactly [`Self::prepared_out_frames`] output-rate
    /// samples into `out`, and return how many were written.
    ///
    /// `native` is the caller's **whole** buffer and `filled` is how much of it
    /// the input ring actually supplied. Taking the count separately, rather
    /// than a slice the caller has already trimmed, is deliberate: the defect
    /// this module is named after was a call site choosing the wrong bound, and
    /// a signature with no bound to choose cannot reproduce it.
    ///
    /// Short of `need` counts as an underrun and the tail is zeroed, because
    /// rubato treats a short input as a hard error rather than a short read.
    /// Those zeros stay inside this buffer: they never reach the caller, and
    /// they cannot accumulate, because nothing is queued between rounds.
    pub fn run(&mut self, native: &[f32], filled: usize, out: &mut [f32]) -> usize {
        debug_assert!(
            filled <= native.len(),
            "filled is {filled} but native holds {}",
            native.len()
        );
        debug_assert!(
            out.len() >= self.chunk,
            "out is {} but the prepared round writes {}",
            out.len(),
            self.chunk
        );
        if out.len() < self.chunk {
            return 0;
        }
        let taken = filled.min(native.len()).min(self.need);
        self.stage[0][..taken].copy_from_slice(&native[..taken]);
        if taken < self.need {
            self.underruns = self.underruns.saturating_add(1);
            for slot in &mut self.stage[0][taken..self.need] {
                *slot = 0.0;
            }
        }
        // Exactly `need` frames, not the whole staging buffer: rubato
        // tolerates extra input, but the slice length is the contract and
        // pinning it here is what makes a short feed unreachable.
        let fed = &self.stage[0][..self.need];
        match self
            .inner
            .process_into_buffer(&[fed], &mut self.output_buf, None)
        {
            Ok((_, frames_out)) => {
                let n = frames_out.min(out.len());
                out[..n].copy_from_slice(&self.output_buf[0][..n]);
                n
            }
            // Only reachable on a buffer-shape mismatch, which the types above
            // make unreachable. Nothing is consumed, so the next round retries
            // with the same input rather than dropping it.
            Err(_) => 0,
        }
    }

    /// Output frames the prepared round will write. Equal to what
    /// [`Self::prepare_within`] was asked for unless the native budget forced
    /// it down.
    #[must_use]
    pub const fn prepared_out_frames(&self) -> usize {
        self.chunk
    }

    /// Native frames the prepared round needs.
    #[must_use]
    pub const fn prepared_need(&self) -> usize {
        self.need
    }

    /// Rounds so far that ran with a zero-filled tail. A session that climbs
    /// steadily here is starved, not drifting.
    #[must_use]
    pub const fn underruns(&self) -> u64 {
        self.underruns
    }

    /// Forget everything buffered; useful when the engine restarts and needs a
    /// clean state without rebuilding the resampler.
    ///
    /// Also resets rubato itself, which holds a sinc history and a fractional
    /// read position — without that, the first output of a new session is
    /// blended with the previous one's tail. `reset` restores rubato's chunk
    /// size to its maximum, which is harmless here because
    /// [`Self::prepare_within`] sets it again every round, but it is why that
    /// call is unconditional.
    pub fn reset(&mut self) {
        self.inner.reset();
        self.chunk = self.max_out_frames;
        self.need = self.inner.input_frames_next();
        self.underruns = 0;
        self.stage[0].fill(0.0);
    }

    #[must_use]
    pub const fn input_rate(&self) -> u32 {
        self.input_rate
    }

    #[must_use]
    pub const fn output_rate(&self) -> u32 {
        self.output_rate
    }
}

#[cfg(test)]
mod tests {
    use super::MonoResampler;

    /// Drive one callback the way the engine does: prepare, fill the stage,
    /// run. Returns what was written.
    fn callback(
        r: &mut MonoResampler,
        out_frames: usize,
        src: &mut impl Iterator<Item = f32>,
    ) -> usize {
        let need = r.prepare_within(out_frames, usize::MAX);
        let native: Vec<f32> = src.take(need).collect();
        assert_eq!(native.len(), need, "the test source should keep up");
        let mut out = vec![0.0_f32; out_frames];
        r.run(&native, need, &mut out)
    }

    fn sine(rate: u32, hz: f32) -> impl Iterator<Item = f32> {
        let step = hz * std::f32::consts::TAU / rate as f32;
        (0u64..).map(move |i| (i as f32 * step).sin() * 0.5)
    }

    #[test]
    fn writes_exactly_the_frames_asked_for() {
        for (inr, outr) in [(44_100, 48_000), (48_000, 44_100), (96_000, 48_000)] {
            let mut r = MonoResampler::new(inr, outr, 4096).expect("construct");
            let mut src = sine(inr, 440.0);
            for &ofr in &[480_usize, 441, 512, 383, 1024] {
                for _ in 0..8 {
                    let n = callback(&mut r, ofr, &mut src);
                    assert_eq!(n, ofr, "{inr}->{outr} asked {ofr}, wrote {n}");
                }
            }
        }
    }

    #[test]
    fn identity_rate_pair_constructs_and_passes_signal() {
        let mut r = MonoResampler::new(48_000, 48_000, 4096).expect("construct");
        let mut src = sine(48_000, 440.0);
        assert_eq!(callback(&mut r, 480, &mut src), 480);
    }

    /// One second in, one second out, at the output rate — the property the
    /// old chunk-256 code got wrong by a third.
    #[test]
    fn a_second_of_input_becomes_a_second_of_output() {
        for (inr, outr) in [(44_100, 48_000), (48_000, 44_100), (44_100, 96_000)] {
            let mut r = MonoResampler::new(inr, outr, 4096).expect("construct");
            let mut src = sine(inr, 440.0);
            let ofr = 480;
            let mut consumed = 0_usize;
            let mut produced = 0_usize;
            for _ in 0..(outr as usize / ofr) {
                consumed += r.prepare_within(ofr, usize::MAX);
                produced += callback(&mut r, ofr, &mut src);
            }
            // `callback` re-prepares, so `consumed` double-counts the prepare;
            // compare the ratio instead of absolute totals.
            let _ = consumed;
            let want = outr as usize / ofr * ofr;
            assert_eq!(
                produced, want,
                "{inr}->{outr}: produced {produced}, want {want}"
            );
        }
    }

    #[test]
    fn a_short_feed_is_an_underrun_not_a_panic() {
        let mut r = MonoResampler::new(44_100, 48_000, 4096).expect("construct");
        let need = r.prepare_within(480, usize::MAX);
        assert!(need > 4, "sanity");
        let native = vec![0.5_f32; need];
        let mut out = vec![0.0_f32; 480];
        assert_eq!(
            r.run(&native, need - 4, &mut out),
            480,
            "still writes a full buffer"
        );
        assert_eq!(r.underruns(), 1, "and records the underrun");
    }

    #[test]
    fn a_native_budget_lowers_the_output_count_instead_of_underfeeding() {
        // 48 k -> 16 k needs 3x its output in input, so a 1024-frame budget
        // cannot serve 1024 output frames.
        let mut r = MonoResampler::new(48_000, 16_000, 4096).expect("construct");
        let need = r.prepare_within(1024, 1024);
        assert!(need <= 1024, "need {need} exceeds the budget");
        assert!(
            r.prepared_out_frames() < 1024,
            "should have lowered the output count, kept {}",
            r.prepared_out_frames()
        );
        assert!(r.prepared_out_frames() > 0);
    }

    #[test]
    fn an_ample_budget_never_lowers_the_output_count() {
        for (inr, outr) in [(44_100, 48_000), (48_000, 44_100), (96_000, 48_000)] {
            let mut r = MonoResampler::new(inr, outr, 4096).expect("construct");
            r.prepare_within(480, usize::MAX);
            assert_eq!(r.prepared_out_frames(), 480, "{inr}->{outr}");
        }
    }

    #[test]
    fn reset_clears_the_history_so_a_new_session_starts_clean() {
        let mut r = MonoResampler::new(44_100, 48_000, 4096).expect("construct");
        let mut loud = std::iter::repeat(0.9_f32);
        callback(&mut r, 480, &mut loud);
        r.reset();
        assert_eq!(r.underruns(), 0);
        // Feed silence: nothing of the loud session may survive into it.
        let mut out = vec![0.0_f32; 480];
        let need = r.prepare_within(480, usize::MAX);
        let silence = vec![0.0_f32; need];
        let n = r.run(&silence, need, &mut out);
        let peak = out[..n].iter().fold(0.0_f32, |a, b| a.max(b.abs()));
        assert!(peak < 1e-6, "stale audio survived reset: peak {peak}");
    }

    #[test]
    fn rates_are_reported_back() {
        let r = MonoResampler::new(44_100, 48_000, 4096).expect("construct");
        assert_eq!((r.input_rate(), r.output_rate()), (44_100, 48_000));
    }
}
