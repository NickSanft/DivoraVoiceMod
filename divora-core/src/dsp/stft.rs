//! Streaming Short-Time Fourier Transform with overlap-add re-synthesis.
//!
//! Shared infrastructure for Phase 9's phase-vocoder pitch shifter and
//! formant warper. The user feeds in mono samples (typically a few
//! hundred at a time, matching the audio callback's chunk); the
//! processor windows the signal, runs a real-input FFT, hands the
//! caller a magnitude / phase spectrum to mutate, runs the inverse,
//! and overlap-adds back into an output buffer.
//!
//! The fixed `WINDOW` / `HOP` constants give 75 % overlap with a Hann
//! window — the standard COLA-compliant setup that lets us reconstruct
//! an unmodified signal back to itself within machine epsilon.
//!
//! Latency = `WINDOW` samples (1024 samples ≈ 21 ms @ 48 kHz).

#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::similar_names
)]

use realfft::num_complex::Complex32;
use realfft::{ComplexToReal, RealFftPlanner, RealToComplex};
use std::sync::Arc;

/// Number of samples in one STFT window.
pub const WINDOW: usize = 1024;
/// Hop between successive windows (75 % overlap; COLA-compliant for Hann).
pub const HOP: usize = 256;
/// Half-window + 1 = number of unique real-FFT bins.
pub const BINS: usize = WINDOW / 2 + 1;

/// Frame state handed to the spectrum-modification callback. Both
/// `mag` and `phase` are length-`BINS` slices.
pub struct Frame<'a> {
    /// Magnitude per bin.
    pub mag: &'a mut [f32],
    /// Phase per bin (radians).
    pub phase: &'a mut [f32],
    /// Bin index → frequency in Hz. `bin_hz * i = freq of bin i`.
    pub bin_hz: f32,
}

/// Streaming STFT processor. One per audio path (i.e. each `PitchShift`
/// instance owns one; each `FormantShift` owns one).
pub struct Stft {
    /// Pre-computed Hann window.
    window: [f32; WINDOW],
    /// `realfft` planner is just a factory; the actual FFT objects below.
    fft_fwd: Arc<dyn RealToComplex<f32>>,
    fft_inv: Arc<dyn ComplexToReal<f32>>,
    /// Input ring — holds the most recent `WINDOW` samples we've seen,
    /// shifted left by `HOP` each time a new frame is analysed.
    in_ring: [f32; WINDOW],
    /// How many samples are queued in `in_ring` since the last frame.
    in_fill: usize,
    /// Output overlap-add buffer. Each synthesised window is added at
    /// the start; once `HOP` samples have accumulated at the front we
    /// pop them and shift the rest left.
    out_ring: Vec<f32>,
    /// FFT scratch buffers (kept around to avoid allocating per call).
    time_buf: Vec<f32>,
    freq_buf: Vec<Complex32>,
    inv_scratch: Vec<Complex32>,
}

impl Stft {
    /// Build a fresh STFT processor. Allocations happen here; per-call
    /// work is allocation-free.
    pub fn new() -> Self {
        let mut planner = RealFftPlanner::<f32>::new();
        let fft_fwd = planner.plan_fft_forward(WINDOW);
        let fft_inv = planner.plan_fft_inverse(WINDOW);
        let inv_scratch = fft_inv.make_scratch_vec();
        let mut window = [0f32; WINDOW];
        for (i, w) in window.iter_mut().enumerate() {
            // Hann window. With HOP = WINDOW / 4 this gives a constant-
            // overlap-add factor of 1.5 (each output sample is the sum
            // of 4 overlapping windows each with average weight 0.375),
            // which we compensate for via OLA_GAIN below.
            let theta = 2.0 * std::f32::consts::PI * i as f32 / WINDOW as f32;
            *w = 0.5 * (1.0 - theta.cos());
        }
        Self {
            window,
            fft_fwd,
            fft_inv,
            in_ring: [0.0; WINDOW],
            in_fill: 0,
            out_ring: vec![0.0; WINDOW + HOP],
            time_buf: vec![0.0; WINDOW],
            freq_buf: vec![Complex32::default(); BINS],
            inv_scratch,
        }
    }

    /// Reset all internal state so the next call starts cleanly. The
    /// chain's `reset()` plumbing wires this in when the engine restarts.
    #[allow(dead_code)] // used by tests; not part of `AudioEffect::reset` yet
    pub fn reset(&mut self) {
        self.in_ring = [0.0; WINDOW];
        self.in_fill = 0;
        for s in &mut self.out_ring {
            *s = 0.0;
        }
    }

    /// Process a block of samples in-place. The caller's closure
    /// receives the magnitude + phase spectrum once per `HOP` input
    /// samples and may mutate them; on return the synthesised frame is
    /// overlap-added into the output buffer and the leading `HOP`
    /// samples are written back into `buf`.
    ///
    /// Total added latency is `WINDOW` samples — the output samples
    /// returned for input `n` correspond to the audio that arrived
    /// `WINDOW` samples earlier.
    pub fn process<F>(&mut self, buf: &mut [f32], sample_rate: u32, mut modify: F)
    where
        F: FnMut(&mut Frame<'_>),
    {
        let bin_hz = sample_rate as f32 / WINDOW as f32;
        for slot in buf.iter_mut() {
            // 1. Shift the new sample into the input ring.
            for i in 0..WINDOW - 1 {
                self.in_ring[i] = self.in_ring[i + 1];
            }
            self.in_ring[WINDOW - 1] = *slot;
            self.in_fill += 1;

            // 2. Every HOP samples in, run a full STFT frame.
            if self.in_fill >= HOP {
                self.in_fill = 0;
                self.run_frame(bin_hz, &mut modify);
            }

            // 3. Pop the head of the output ring as this sample's output.
            *slot = self.out_ring[0] * OLA_GAIN;
            // Shift the output ring left by one.
            for i in 0..self.out_ring.len() - 1 {
                self.out_ring[i] = self.out_ring[i + 1];
            }
            *self.out_ring.last_mut().expect("non-empty out_ring") = 0.0;
        }
    }

    fn run_frame<F: FnMut(&mut Frame<'_>)>(&mut self, bin_hz: f32, modify: &mut F) {
        // Window the input.
        for (i, slot) in self.time_buf.iter_mut().enumerate() {
            *slot = self.in_ring[i] * self.window[i];
        }
        // Forward FFT.
        let _ = self.fft_fwd.process(&mut self.time_buf, &mut self.freq_buf);

        // Decompose into magnitude + phase, hand to the user.
        let mut mag = [0f32; BINS];
        let mut phase = [0f32; BINS];
        for i in 0..BINS {
            let c = self.freq_buf[i];
            mag[i] = (c.re * c.re + c.im * c.im).sqrt();
            phase[i] = c.im.atan2(c.re);
        }
        let mut frame = Frame {
            mag: &mut mag,
            phase: &mut phase,
            bin_hz,
        };
        modify(&mut frame);

        // Re-combine and inverse FFT.
        for i in 0..BINS {
            let m = mag[i];
            let p = phase[i];
            self.freq_buf[i] = Complex32::new(m * p.cos(), m * p.sin());
        }
        let _ = self.fft_inv.process_with_scratch(
            &mut self.freq_buf,
            &mut self.time_buf,
            &mut self.inv_scratch,
        );
        // Normalise (rustfft's inverse is unscaled), window again, OLA into out_ring.
        let scale = 1.0 / WINDOW as f32;
        for (i, slot) in self.out_ring[..WINDOW].iter_mut().enumerate() {
            *slot += self.time_buf[i] * scale * self.window[i];
        }
    }
}

impl Default for Stft {
    fn default() -> Self {
        Self::new()
    }
}

/// Compensation for Hann-on-Hann overlap-add at HOP = WINDOW/4. The
/// constant comes from the sum of overlapping Hann² values at each
/// output sample.
const OLA_GAIN: f32 = 2.0 / 3.0;

#[cfg(test)]
mod tests {
    use super::Stft;

    fn sine(len: usize, freq: f32, sample_rate: f32) -> Vec<f32> {
        (0..len)
            .map(|i| {
                let t = i as f32 / sample_rate;
                (2.0 * std::f32::consts::PI * freq * t).sin() * 0.5
            })
            .collect()
    }

    /// A passthrough modifier (identity on the spectrum) must reconstruct
    /// the input within a small epsilon after the warm-up window.
    #[test]
    fn identity_modifier_reconstructs_input_after_warmup() {
        let sr = 48_000_u32;
        let input = sine(8192, 440.0, sr as f32);
        let mut buf = input.clone();
        let mut stft = Stft::new();
        stft.process(&mut buf, sr, |_frame| {
            // No modification — should round-trip.
        });
        // First WINDOW samples are warm-up (filled with zeros / partial
        // overlap). After that, the output should match the input
        // shifted by WINDOW.
        let warm = super::WINDOW;
        let mut max_err: f32 = 0.0;
        for i in warm..buf.len() {
            let diff = (buf[i] - input[i - warm]).abs();
            if diff > max_err {
                max_err = diff;
            }
        }
        assert!(
            max_err < 0.05,
            "identity STFT diverged from input by max {max_err}"
        );
    }

    #[test]
    fn dc_signal_stays_finite() {
        let sr = 48_000_u32;
        let mut buf = vec![0.42_f32; 8192];
        let mut stft = Stft::new();
        stft.process(&mut buf, sr, |_| {});
        for s in &buf {
            assert!(s.is_finite(), "DC processing produced non-finite sample");
        }
    }

    #[test]
    fn zeroing_the_spectrum_silences_output() {
        let sr = 48_000_u32;
        let mut buf = sine(8192, 1000.0, sr as f32);
        let mut stft = Stft::new();
        stft.process(&mut buf, sr, |frame| {
            for m in frame.mag.iter_mut() {
                *m = 0.0;
            }
        });
        for s in &buf[super::WINDOW..] {
            assert!(s.abs() < 1e-3, "zeroed spectrum still produced sample {s}");
        }
    }

    #[test]
    fn reset_brings_output_back_to_zero() {
        let sr = 48_000_u32;
        let mut buf = sine(4096, 440.0, sr as f32);
        let mut stft = Stft::new();
        stft.process(&mut buf, sr, |_| {});
        stft.reset();
        let mut quiet = vec![0.0_f32; 2048];
        stft.process(&mut quiet, sr, |_| {});
        for s in &quiet {
            assert!(s.abs() < 1e-3, "post-reset output not silent; got {s}");
        }
    }
}
