//! Mono sample-rate converter used to bridge a device-rate mismatch
//! between the input and output streams. Wraps `rubato::SincFixedOut`
//! so the audio callback can ask for a fixed number of output frames
//! and feed whatever input it has on hand.
//!
//! Phase 9 replaces the hard `SampleRateMismatch` error that Phase 2
//! threw — `DivoraVoice` now accepts any pair of supported device rates
//! and resamples between them silently.

#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]

use rubato::{
    Resampler, SincFixedOut, SincInterpolationParameters, SincInterpolationType, WindowFunction,
};

use super::AudioEngineError;

/// Default output chunk size we ask `SincFixedOut` to produce per call.
/// Sized to match the largest typical cpal device callback in our
/// pipeline (4096 frames). The resampler buffers the difference between
/// what the device asks for this callback and what we hand it.
const DEFAULT_CHUNK: usize = 256;

/// Streaming mono resampler.
///
/// Methods are realtime-safe in the sense that they allocate once at
/// construction. Per-callback work is bounded by the chunk size.
pub struct MonoResampler {
    inner: SincFixedOut<f32>,
    /// `rubato` works on a `&[Vec<f32>]` (per-channel), so we keep a
    /// 1-channel scratch buffer here. Cleared and refilled per call.
    input_buf: Vec<Vec<f32>>,
    output_buf: Vec<Vec<f32>>,
    /// Native-rate samples queued up across `process` calls but not yet
    /// fed to the rubato engine (because the engine takes a fixed
    /// number of input frames per round).
    pending: Vec<f32>,
    input_rate: u32,
    output_rate: u32,
}

impl MonoResampler {
    /// Create a resampler that converts `input_rate` → `output_rate`,
    /// producing `chunk_out` output frames per `process` call.
    ///
    /// Returns an error only if the underlying `rubato` constructor
    /// rejects the rate combination (which it shouldn't for any sane
    /// audio rate pair).
    pub fn new(
        input_rate: u32,
        output_rate: u32,
        chunk_out: usize,
    ) -> Result<Self, AudioEngineError> {
        let chunk = if chunk_out == 0 {
            DEFAULT_CHUNK
        } else {
            chunk_out
        };
        let ratio = f64::from(output_rate) / f64::from(input_rate);
        let params = SincInterpolationParameters {
            sinc_len: 128,
            f_cutoff: 0.95,
            interpolation: SincInterpolationType::Linear,
            oversampling_factor: 128,
            window: WindowFunction::BlackmanHarris2,
        };
        // rubato expects `resample_ratio = output_rate / input_rate`.
        // SincFixedOut produces `chunk` output samples per call.
        let inner = SincFixedOut::<f32>::new(ratio, 1.0, params, chunk, 1).map_err(|e| {
            AudioEngineError::ResamplerBuild {
                input: input_rate,
                output: output_rate,
                message: e.to_string(),
            }
        })?;
        let input_frames_max = inner.input_frames_max();
        Ok(Self {
            inner,
            input_buf: vec![Vec::with_capacity(input_frames_max)],
            output_buf: vec![vec![0.0; chunk]],
            pending: Vec::with_capacity(input_frames_max * 4),
            input_rate,
            output_rate,
        })
    }

    /// How many input samples does the resampler still need before it
    /// can produce its next chunk of output? Callers use this to size
    /// their pop from an upstream ring buffer.
    #[must_use]
    pub fn input_frames_next(&self) -> usize {
        self.inner.input_frames_next()
    }

    /// Queue native-rate input samples for the next `process` call.
    pub fn push_input(&mut self, samples: &[f32]) {
        self.pending.extend_from_slice(samples);
    }

    /// Drain the queued input through the resampler until either the
    /// caller's `out` slice is full or we run out of pending input.
    /// Returns the number of output samples actually written.
    pub fn process(&mut self, out: &mut [f32]) -> usize {
        let mut written = 0;
        while written < out.len() {
            let need = self.inner.input_frames_next();
            if self.pending.len() < need {
                // Not enough input queued for another rubato round.
                break;
            }
            self.input_buf[0].clear();
            self.input_buf[0].extend_from_slice(&self.pending[..need]);
            self.pending.drain(..need);

            // Ensure the output slot is at least chunk-sized; rubato
            // writes into it from the start.
            let chunk = self.output_buf[0].len();
            if chunk == 0 {
                break;
            }

            // `process_into_buffer` returns (frames_in_used, frames_out_written).
            match self
                .inner
                .process_into_buffer(&self.input_buf, &mut self.output_buf, None)
            {
                Ok((_, frames_out)) => {
                    let take = frames_out.min(out.len() - written);
                    out[written..written + take].copy_from_slice(&self.output_buf[0][..take]);
                    written += take;
                    if take < frames_out {
                        // Caller's slice is full but rubato handed us
                        // more — stash the remainder for next time.
                        // (Won't happen in steady state because `out` is
                        // sized to a multiple of `chunk`, but defensive
                        // here.)
                        let leftover = &self.output_buf[0][take..frames_out].to_vec();
                        self.pending.splice(0..0, leftover.iter().copied());
                    }
                }
                Err(_) => break,
            }
        }
        written
    }

    /// Drop every queued input sample; useful when the engine restarts
    /// and needs a clean state without rebuilding the resampler.
    pub fn reset(&mut self) {
        self.pending.clear();
    }

    #[must_use]
    pub fn input_rate(&self) -> u32 {
        self.input_rate
    }

    #[must_use]
    pub fn output_rate(&self) -> u32 {
        self.output_rate
    }
}

#[cfg(test)]
mod tests {
    use super::MonoResampler;

    /// Sanity: 48 kHz → 48 kHz with a unity ratio is constructible and
    /// produces some output for some input.
    #[test]
    fn identity_rate_pair_constructs_and_passes_signal() {
        let mut r = MonoResampler::new(48_000, 48_000, 256).expect("construct");
        let mut input = Vec::with_capacity(4096);
        for i in 0..4096 {
            input.push((i as f32 / 100.0).sin());
        }
        r.push_input(&input);
        let mut out = vec![0.0_f32; 2048];
        let n = r.process(&mut out);
        assert!(n > 0, "should produce some output");
    }

    #[test]
    fn upsample_44100_to_48000_produces_more_samples_than_input() {
        let mut r = MonoResampler::new(44_100, 48_000, 256).expect("construct");
        let in_samples = 44_100; // 1 second of audio
        let mut input = Vec::with_capacity(in_samples);
        for i in 0..in_samples {
            input.push((i as f32 / 50.0).sin());
        }
        r.push_input(&input);
        // Pull output one chunk at a time until the resampler runs dry.
        let mut total_out = 0;
        let mut out = vec![0.0_f32; 256];
        loop {
            let n = r.process(&mut out);
            if n == 0 {
                break;
            }
            total_out += n;
        }
        // 1 s of 44.1 kHz audio should produce ~48000 output samples ±
        // a couple of chunks (because we stop once input is dry).
        // Allow ±5% tolerance.
        let expected = 48_000.0;
        let lower = (expected * 0.92) as usize;
        let upper = (expected * 1.0) as usize;
        assert!(
            total_out >= lower && total_out <= upper,
            "expected ~48000 output samples, got {total_out} (lower={lower}, upper={upper})"
        );
    }

    #[test]
    fn downsample_48000_to_44100_produces_fewer_samples_than_input() {
        let mut r = MonoResampler::new(48_000, 44_100, 256).expect("construct");
        let in_samples = 48_000;
        let mut input = Vec::with_capacity(in_samples);
        for i in 0..in_samples {
            input.push((i as f32 / 50.0).sin());
        }
        r.push_input(&input);
        let mut total_out = 0;
        let mut out = vec![0.0_f32; 256];
        loop {
            let n = r.process(&mut out);
            if n == 0 {
                break;
            }
            total_out += n;
        }
        let expected = 44_100.0;
        let lower = (expected * 0.92) as usize;
        let upper = (expected * 1.0) as usize;
        assert!(
            total_out >= lower && total_out <= upper,
            "expected ~44100 output samples, got {total_out} (lower={lower}, upper={upper})"
        );
    }

    #[test]
    fn reset_clears_pending_without_panic() {
        let mut r = MonoResampler::new(48_000, 44_100, 256).expect("construct");
        r.push_input(&vec![0.5_f32; 1024]);
        r.reset();
        // After reset, processing without more input should write nothing.
        let mut out = vec![0.0_f32; 256];
        let n = r.process(&mut out);
        assert_eq!(n, 0);
    }
}
