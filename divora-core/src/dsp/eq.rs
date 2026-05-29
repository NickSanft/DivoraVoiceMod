//! Three-band parametric EQ. Low-shelf at 200 Hz, peaking at 1 kHz,
//! high-shelf at 5 kHz. Each band's gain is bipolar (±12 dB).

use biquad::{Biquad, Coefficients, DirectForm1, ToHertz, Type};

use super::{AudioEffect, EffectKind};

const F_LOW_HZ: f32 = 200.0;
const F_MID_HZ: f32 = 1000.0;
const F_HIGH_HZ: f32 = 5000.0;
const Q: f32 = 0.7;

pub struct Eq {
    enabled: bool,
    low_db: f32,
    mid_db: f32,
    high_db: f32,
    low: DirectForm1<f32>,
    mid: DirectForm1<f32>,
    high: DirectForm1<f32>,
    sample_rate: u32,
}

impl Eq {
    #[must_use]
    pub fn new() -> Self {
        let sr = 48000_u32;
        let mut eq = Self {
            enabled: false,
            low_db: 0.0,
            mid_db: 0.0,
            high_db: 0.0,
            low: DirectForm1::<f32>::new(neutral_coeffs(sr)),
            mid: DirectForm1::<f32>::new(neutral_coeffs(sr)),
            high: DirectForm1::<f32>::new(neutral_coeffs(sr)),
            sample_rate: sr,
        };
        eq.rebuild();
        eq
    }

    fn rebuild(&mut self) {
        let fs = self.sample_rate.hz();
        if let Ok(c) =
            Coefficients::<f32>::from_params(Type::LowShelf(self.low_db), fs, F_LOW_HZ.hz(), Q)
        {
            self.low.update_coefficients(c);
        }
        if let Ok(c) =
            Coefficients::<f32>::from_params(Type::PeakingEQ(self.mid_db), fs, F_MID_HZ.hz(), Q)
        {
            self.mid.update_coefficients(c);
        }
        if let Ok(c) =
            Coefficients::<f32>::from_params(Type::HighShelf(self.high_db), fs, F_HIGH_HZ.hz(), Q)
        {
            self.high.update_coefficients(c);
        }
    }
}

impl Default for Eq {
    fn default() -> Self {
        Self::new()
    }
}

fn neutral_coeffs(sr: u32) -> Coefficients<f32> {
    Coefficients::<f32>::from_params(Type::PeakingEQ(0.0), sr.hz(), F_MID_HZ.hz(), Q)
        .expect("neutral peaking EQ coefficients should always build")
}

impl AudioEffect for Eq {
    fn process(&mut self, buffer: &mut [f32], sample_rate: u32) {
        if sample_rate != self.sample_rate {
            self.sample_rate = sample_rate;
            self.rebuild();
        }
        for sample in buffer.iter_mut() {
            *sample = self.high.run(self.mid.run(self.low.run(*sample)));
        }
    }

    fn set_param(&mut self, key: &str, value: f32) {
        let v = value.clamp(-24.0, 24.0);
        match key {
            "low" => self.low_db = v,
            "mid" => self.mid_db = v,
            "high" => self.high_db = v,
            _ => return,
        }
        self.rebuild();
    }

    fn enabled(&self) -> bool {
        self.enabled
    }

    fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }

    fn kind(&self) -> EffectKind {
        EffectKind::Eq
    }
}

#[cfg(test)]
mod tests {
    use super::{AudioEffect, Eq};

    #[test]
    fn zero_gain_is_near_passthrough() {
        let mut eq = Eq::new();
        eq.set_enabled(true);
        let mut buf = [0.5_f32; 64];
        eq.process(&mut buf, 48000);
        for s in buf {
            assert!((s - 0.5).abs() < 0.1);
        }
    }

    #[test]
    fn positive_low_gain_boosts_low_freq_content() {
        let mut neutral = Eq::new();
        let mut boosted = Eq::new();
        neutral.set_enabled(true);
        boosted.set_enabled(true);
        boosted.set_param("low", 12.0);

        // A 100 Hz sine at 48 kHz: should be boosted by the low shelf.
        let mut a = vec![0_f32; 4800];
        let mut b = vec![0_f32; 4800];
        for i in 0..a.len() {
            #[allow(clippy::cast_precision_loss)]
            let t = i as f32 / 48000.0;
            let s = (2.0 * std::f32::consts::PI * 100.0 * t).sin() * 0.5;
            a[i] = s;
            b[i] = s;
        }
        neutral.process(&mut a, 48000);
        boosted.process(&mut b, 48000);
        let rms_a: f32 = a.iter().map(|s| s * s).sum::<f32>().sqrt();
        let rms_b: f32 = b.iter().map(|s| s * s).sum::<f32>().sqrt();
        assert!(rms_b > rms_a);
    }
}
