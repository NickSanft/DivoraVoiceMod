//! Spectral centroid — the "brightness" number.
//!
//! ### Why a real FFT rather than something cheaper
//!
//! A filter bank or a zero-crossing estimate would be cheaper, but neither
//! computes the quantity the field is named after. Third-octave bands put all
//! of a band's energy at its centre, so a tone at a band edge is reported
//! 12 % off; the closed-form differentiator trick
//! (`f = fs/pi * asin(sqrt(E[dx^2]/4E[x^2]))`) is exact for a pure tone but
//! computes the *root-mean-square* frequency, which is strictly above the
//! first moment for anything broadband, by an amount that depends on the
//! spectrum's shape — i.e. an error that moves when the effect chain moves,
//! which is exactly when this readout is being looked at.
//!
//! An FFT is affordable here because this is the worker thread, not the audio
//! callback, and because brightness is a window statistic: one transform every
//! 40 ms is plenty, versus the 10 ms level framing.
//!
//! ### Accuracy
//!
//! Bin spacing is `sample_rate / n`, ~23 Hz at 48 kHz with `n = 2048`, so a
//! narrow-band signal is quantised to ±12 Hz. A Hann window trades that for
//! leakage: its main lobe is four bins wide, which on a tonal signal spreads a
//! little magnitude symmetrically and leaves the first moment essentially
//! unbiased, while without it the sidelobes of a rectangular window would drag
//! the centroid upward by hundreds of Hz. Bins below [`CENTROID_FLOOR_HZ`] are
//! excluded: DC offset and desk rumble carry real magnitude and would pull the
//! centroid of a quiet frame toward zero.

#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]

use std::sync::Arc;

use realfft::num_complex::Complex32;
use realfft::{RealFftPlanner, RealToComplex};

/// Lowest bin that counts toward the centroid, Hz. Below the bottom of the
/// pitch search range, so nothing voiced is discarded.
const CENTROID_FLOOR_HZ: f32 = 50.0;
/// Top of the integration band, Hz.
///
/// Without a ceiling the moment is taken to Nyquist, so the SAME physical
/// signal reads differently at 44.1, 48 and 96 kHz — the converter's own
/// noise floor occupies more bins at a higher rate and drags the centroid up
/// with it. Review measured a 3.2 semitone gap between 48 and 96 kHz on
/// speech at 30 dB SNR, against a descriptor threshold of 5, and pure noise
/// reading exactly Nyquist/2 at every rate. That breaks the project's
/// sample-rate-independence rule.
///
/// 10 kHz is above everything a voice puts there — sibilance peaks well
/// below it — so the band covers the signal and stops where only the floor
/// is left.
const CENTROID_CEIL_HZ: f32 = 10_000.0;

/// Windowed spectral centroid over a fixed frame length.
///
/// Buffers are allocated in [`Centroid::new`].
pub struct Centroid {
    fft: Arc<dyn RealToComplex<f32>>,
    /// Hann window, one coefficient per input sample.
    window: Vec<f32>,
    /// Zero-padded transform input.
    time: Vec<f32>,
    freq: Vec<Complex32>,
    bin_hz: f32,
    first_bin: usize,
    /// One past the last bin in the band; see [`CENTROID_CEIL_HZ`].
    last_bin: usize,
}

impl Centroid {
    /// `frame_len` is the number of real samples fed to [`Centroid::measure`];
    /// the transform size is the next power of two at or above it.
    pub fn new(sample_rate: f32, frame_len: usize) -> Self {
        let n = frame_len.max(2).next_power_of_two();
        let fft = RealFftPlanner::<f32>::new().plan_fft_forward(n);
        let freq = fft.make_output_vec();
        let window = (0..frame_len)
            .map(|i| {
                let phase = std::f32::consts::TAU * i as f32 / frame_len as f32;
                0.5 - 0.5 * phase.cos()
            })
            .collect();
        let bin_hz = sample_rate / n as f32;
        let bins = freq.len();
        Self {
            fft,
            window,
            time: vec![0.0; n],
            freq,
            bin_hz,
            first_bin: (CENTROID_FLOOR_HZ / bin_hz).ceil().max(1.0) as usize,
            last_bin: ((CENTROID_CEIL_HZ / bin_hz).ceil() as usize + 1).min(bins),
        }
    }

    /// Magnitude-weighted first moment of `frame`'s spectrum, Hz. `None` when
    /// the frame carries no magnitude to take a moment of.
    pub fn measure(&mut self, frame: &[f32]) -> Option<f32> {
        if frame.len() != self.window.len() {
            return None;
        }
        for (slot, (&x, &w)) in self.time.iter_mut().zip(frame.iter().zip(&self.window)) {
            *slot = x * w;
        }
        for slot in &mut self.time[frame.len()..] {
            *slot = 0.0;
        }
        self.fft.process(&mut self.time, &mut self.freq).ok()?;
        let mut weighted = 0.0f64;
        let mut total = 0.0f64;
        let band = self.first_bin..self.last_bin.max(self.first_bin + 1);
        for (k, bin) in self.freq.iter().enumerate().take(band.end).skip(band.start) {
            let mag = f64::from(bin.norm());
            weighted += mag * f64::from(k as f32 * self.bin_hz);
            total += mag;
        }
        if total <= f64::from(f32::MIN_POSITIVE) {
            return None;
        }
        Some((weighted / total) as f32)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Deterministic broadband noise, the content the old unbounded band was
    /// wrong about.
    fn noise(len: usize) -> Vec<f32> {
        let mut state = 0x2545_F491_4F6C_DD1D_u64;
        (0..len)
            .map(|_| {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                #[allow(clippy::cast_precision_loss)]
                let unit = (state >> 40) as f32 / (1u64 << 24) as f32;
                unit.mul_add(2.0, -1.0) * 0.2
            })
            .collect()
    }

    #[test]
    fn broadband_content_reads_the_same_at_every_sample_rate() {
        // The band used to run to Nyquist, so the same physical signal read
        // higher at a higher rate purely because the noise floor occupied
        // more bins. The suite missed it because its fixture was a clean
        // harmonic stack with nothing up there — the one signal class where
        // an unbounded band looks fine.
        let mut readings = Vec::new();
        for &rate in &[44_100.0f32, 48_000.0, 96_000.0] {
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let len = (rate * 0.025) as usize;
            let mut centroid = Centroid::new(rate, len);
            let hz = centroid.measure(&noise(len)).expect("noise has magnitude");
            readings.push((rate, hz));
        }
        let (_, reference) = readings[1];
        for &(rate, hz) in &readings {
            let st = 12.0 * (hz / reference).log2();
            assert!(
                st.abs() < 1.0,
                "{rate} Hz reads {hz} Hz, {st} semitones from the 48 kHz reading"
            );
        }
    }

    fn frame(rate: f32, len: usize, parts: &[(f32, f32)]) -> Vec<f32> {
        (0..len)
            .map(|i| {
                parts
                    .iter()
                    .map(|&(hz, amp)| amp * (std::f32::consts::TAU * hz * i as f32 / rate).sin())
                    .sum()
            })
            .collect()
    }

    #[test]
    fn a_pure_tone_sits_on_its_own_frequency() {
        let rate = 48_000.0f32;
        let len = 1920; // 40 ms
        let mut c = Centroid::new(rate, len);
        for &hz in &[500.0f32, 1_000.0, 4_000.0] {
            let got = c.measure(&frame(rate, len, &[(hz, 1.0)])).unwrap();
            // One bin is 23.4 Hz here; allow a couple of them for leakage.
            assert!(
                (got - hz).abs() < 60.0,
                "{hz} Hz tone read a centroid of {got} Hz"
            );
        }
    }

    #[test]
    fn two_tones_of_equal_amplitude_land_between_them() {
        let rate = 48_000.0f32;
        let len = 1920;
        let mut c = Centroid::new(rate, len);
        let got = c
            .measure(&frame(rate, len, &[(500.0, 1.0), (3_500.0, 1.0)]))
            .unwrap();
        assert!(
            (got - 2_000.0).abs() < 120.0,
            "expected ~2000 Hz, got {got} Hz"
        );
    }

    #[test]
    fn dc_offset_does_not_drag_the_centroid_down() {
        let rate = 48_000.0f32;
        let len = 1920;
        let mut c = Centroid::new(rate, len);
        let clean = c.measure(&frame(rate, len, &[(2_000.0, 1.0)])).unwrap();
        let mut offset = frame(rate, len, &[(2_000.0, 1.0)]);
        for s in &mut offset {
            *s += 0.5;
        }
        let got = c.measure(&offset).unwrap();
        assert!(
            (got - clean).abs() < 100.0,
            "a DC offset moved the centroid from {clean} Hz to {got} Hz"
        );
    }

    #[test]
    fn silence_measures_nothing() {
        let mut c = Centroid::new(48_000.0, 1920);
        assert!(c.measure(&vec![0.0; 1920]).is_none());
    }
}
