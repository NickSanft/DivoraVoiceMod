//! Formant shift — Phase 3 ships a coloration effect built from three
//! parallel band-pass filters tuned to typical vowel formants. The
//! `shift` parameter scales the centre frequencies by 2^(shift/12).
//! Real LPC-based formant warping lands in a later phase.

use biquad::{Biquad, Coefficients, DirectForm1, ToHertz, Type};

use super::{AudioEffect, EffectKind};

const F1_HZ: f32 = 700.0;
const F2_HZ: f32 = 1220.0;
const F3_HZ: f32 = 2600.0;
const Q: f32 = 6.0;
const WET_MIX: f32 = 0.45;

pub struct FormantShift {
    enabled: bool,
    shift: f32,
    f1: DirectForm1<f32>,
    f2: DirectForm1<f32>,
    f3: DirectForm1<f32>,
    sample_rate: u32,
}

impl FormantShift {
    #[must_use]
    pub fn new() -> Self {
        let sr = 48000_u32;
        let mut f = Self {
            enabled: false,
            shift: 0.0,
            f1: DirectForm1::<f32>::new(neutral_coeffs(sr, F1_HZ)),
            f2: DirectForm1::<f32>::new(neutral_coeffs(sr, F2_HZ)),
            f3: DirectForm1::<f32>::new(neutral_coeffs(sr, F3_HZ)),
            sample_rate: sr,
        };
        f.rebuild();
        f
    }

    fn rebuild(&mut self) {
        let ratio = 2.0_f32.powf(self.shift / 12.0);
        let fs = self.sample_rate.hz();
        #[allow(clippy::cast_precision_loss)]
        let max_hz = (self.sample_rate / 2 - 100) as f32;
        let f1 = (F1_HZ * ratio).clamp(80.0, max_hz);
        let f2 = (F2_HZ * ratio).clamp(80.0, max_hz);
        let f3 = (F3_HZ * ratio).clamp(80.0, max_hz);
        for (filter, freq) in [(&mut self.f1, f1), (&mut self.f2, f2), (&mut self.f3, f3)] {
            if let Ok(c) = Coefficients::<f32>::from_params(Type::BandPass, fs, freq.hz(), Q) {
                filter.update_coefficients(c);
            }
        }
    }
}

impl Default for FormantShift {
    fn default() -> Self {
        Self::new()
    }
}

fn neutral_coeffs(sr: u32, freq: f32) -> Coefficients<f32> {
    Coefficients::<f32>::from_params(Type::BandPass, sr.hz(), freq.hz(), Q)
        .expect("default formant band-pass coefficients should always build")
}

impl AudioEffect for FormantShift {
    fn process(&mut self, buffer: &mut [f32], sample_rate: u32) {
        if sample_rate != self.sample_rate {
            self.sample_rate = sample_rate;
            self.rebuild();
        }
        for sample in buffer.iter_mut() {
            let dry = *sample;
            let wet = self.f1.run(dry) + self.f2.run(dry) + self.f3.run(dry);
            *sample = dry * (1.0 - WET_MIX) + wet * (WET_MIX / 3.0);
        }
    }

    fn set_param(&mut self, key: &str, value: f32) {
        if key == "shift" {
            self.shift = value.clamp(-24.0, 24.0);
            self.rebuild();
        }
    }

    fn enabled(&self) -> bool {
        self.enabled
    }

    fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }

    fn kind(&self) -> EffectKind {
        EffectKind::Formant
    }
}

#[cfg(test)]
mod tests {
    use super::{AudioEffect, FormantShift};

    #[test]
    fn processing_is_stable_at_zero_shift() {
        let mut f = FormantShift::new();
        f.set_enabled(true);
        let mut buf = vec![0.0_f32; 4096];
        for (i, s) in buf.iter_mut().enumerate() {
            #[allow(clippy::cast_precision_loss)]
            let t = i as f32 / 48000.0;
            *s = (2.0 * std::f32::consts::PI * 200.0 * t).sin() * 0.5;
        }
        f.process(&mut buf, 48000);
        for s in &buf {
            assert!(s.is_finite());
            assert!(s.abs() < 2.0);
        }
    }
}
