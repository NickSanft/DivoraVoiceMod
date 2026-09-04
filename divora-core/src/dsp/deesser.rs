//! De-esser (v1.8.0). A split-band dynamic processor that tames
//! sibilance ("sss" / "shh") — the harsh high-frequency energy that
//! pitch and formant shifting tend to exaggerate.
//!
//! It isolates a high band with a high-pass biquad, tracks that band's
//! level, and when the band exceeds the threshold it subtracts a portion
//! of it back out of the full signal. Only the *high* band is ducked, so
//! the body of the voice is untouched. **Zero look-ahead → no latency.**

use biquad::{Biquad, Coefficients, DirectForm1, ToHertz, Type};

use super::envelope::{coeff_ms, EnvelopeFollower};
use super::{AudioEffect, EffectKind};

const Q: f32 = 0.707;

pub struct DeEsser {
    enabled: bool,
    freq_hz: f32,
    threshold_db: f32,
    /// Maximum attenuation applied to the high band, in dB.
    range_db: f32,
    hp: DirectForm1<f32>,
    /// Smoothed gain reduction (dB, ≤ 0) applied to the high band.
    env_db: EnvelopeFollower,
    sample_rate: u32,
}

impl DeEsser {
    #[must_use]
    pub fn new() -> Self {
        let sr = 48_000_u32;
        let mut d = Self {
            enabled: false,
            freq_hz: 6000.0,
            threshold_db: -30.0,
            range_db: 12.0,
            hp: DirectForm1::<f32>::new(hp_coeffs(sr, 6000.0)),
            env_db: EnvelopeFollower::new(0.0),
            sample_rate: sr,
        };
        d.rebuild();
        d
    }

    fn rebuild(&mut self) {
        self.hp
            .update_coefficients(hp_coeffs(self.sample_rate, self.freq_hz));
    }
}

fn hp_coeffs(sr: u32, freq_hz: f32) -> Coefficients<f32> {
    // Clamp the corner below Nyquist so coefficient construction can't fail.
    #[allow(clippy::cast_precision_loss)]
    let nyq = (sr as f32) * 0.5;
    let f = freq_hz.clamp(1000.0, nyq * 0.95);
    Coefficients::<f32>::from_params(Type::HighPass, sr.hz(), f.hz(), Q)
        .or_else(|_| {
            Coefficients::<f32>::from_params(Type::HighPass, 48_000_u32.hz(), 6000.0_f32.hz(), Q)
        })
        .expect("high-pass coefficients should build at the fallback corner")
}

impl Default for DeEsser {
    fn default() -> Self {
        Self::new()
    }
}

impl AudioEffect for DeEsser {
    fn process(&mut self, buffer: &mut [f32], sample_rate: u32) {
        if sample_rate != self.sample_rate {
            self.sample_rate = sample_rate;
            self.rebuild();
        }
        #[allow(clippy::cast_precision_loss)]
        let sr = sample_rate.max(1) as f32;
        // De-essing is fast: short attack, moderate release.
        let attack_coeff = coeff_ms(1.0, sr);
        let release_coeff = coeff_ms(60.0, sr);

        for sample in buffer.iter_mut() {
            let x = if sample.is_finite() { *sample } else { 0.0 };
            // Isolate the high band.
            let high = self.hp.run(x);
            let level_db = 20.0 * high.abs().max(1e-9).log10();

            // How far over the threshold the sibilant band is.
            let over = (level_db - self.threshold_db).max(0.0);
            let target_db = -over.min(self.range_db); // reduction, ≤ 0

            let env_db = self
                .env_db
                .attack_on_fall(target_db, attack_coeff, release_coeff);

            // gain ∈ (0, 1]; subtract the attenuated fraction of the band.
            let gain = 10_f32.powf(env_db / 20.0);
            let out = x - high * (1.0 - gain);
            *sample = if out.is_finite() { out } else { 0.0 };
        }
    }

    fn set_param(&mut self, key: &str, value: f32) {
        match key {
            "freq" => {
                self.freq_hz = value.clamp(1000.0, 16_000.0);
                self.rebuild();
            }
            "thresh" => self.threshold_db = value.clamp(-80.0, 0.0),
            "range" => self.range_db = value.clamp(0.0, 36.0),
            _ => {}
        }
    }

    fn enabled(&self) -> bool {
        self.enabled
    }

    fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
        if !enabled {
            self.env_db.reset(0.0);
        }
    }

    fn kind(&self) -> EffectKind {
        EffectKind::Deesser
    }
    // latency_samples: default 0 — split-band, no look-ahead.
}

#[cfg(test)]
mod tests {
    use super::{AudioEffect, DeEsser};

    fn sine(freq: f32, amp: f32, n: usize) -> Vec<f32> {
        (0..n)
            .map(|i| {
                #[allow(clippy::cast_precision_loss)]
                let t = i as f32 / 48_000.0;
                (2.0 * std::f32::consts::PI * freq * t).sin() * amp
            })
            .collect()
    }

    fn rms(buf: &[f32]) -> f32 {
        #[allow(clippy::cast_precision_loss)]
        let n = buf.len() as f32;
        (buf.iter().map(|s| s * s).sum::<f32>() / n).sqrt()
    }

    /// A low-frequency tone is below the de-esser's band and passes
    /// through essentially untouched.
    #[test]
    fn low_frequency_passes_through() {
        let mut d = DeEsser::new();
        d.set_enabled(true);
        d.set_param("freq", 6000.0);
        let mut buf = sine(200.0, 0.5, 9600);
        let before = rms(&buf);
        d.process(&mut buf, 48_000);
        let after = rms(&buf);
        assert!(
            (after - before).abs() / before < 0.1,
            "low-freq tone should pass (before {before}, after {after})"
        );
    }

    /// A loud high-frequency tone in the sibilance band gets attenuated.
    #[test]
    fn sibilant_high_frequency_is_attenuated() {
        let mut d = DeEsser::new();
        d.set_enabled(true);
        d.set_param("freq", 6000.0);
        d.set_param("thresh", -30.0);
        d.set_param("range", 18.0);
        let mut buf = sine(8000.0, 0.5, 48_000);
        let before = rms(&buf);
        d.process(&mut buf, 48_000);
        // Settled second half.
        let after = rms(&buf[24_000..]);
        assert!(
            after < before * 0.8,
            "sibilant band should be attenuated (before {before}, after {after})"
        );
    }

    #[test]
    fn stays_finite_on_extreme_input() {
        let mut d = DeEsser::new();
        d.set_enabled(true);
        let mut buf = [f32::NAN, f32::INFINITY, -f32::INFINITY, 1e30, -1e30, 0.0];
        d.process(&mut buf, 48_000);
        for s in buf {
            assert!(s.is_finite(), "output must stay finite, got {s}");
        }
    }

    #[test]
    fn adds_no_latency() {
        let d = DeEsser::new();
        assert_eq!(d.latency_samples(48_000), 0);
    }

    #[test]
    fn handles_sample_rate_change() {
        let mut d = DeEsser::new();
        d.set_enabled(true);
        let mut buf = sine(8000.0, 0.4, 512);
        d.process(&mut buf, 48_000);
        let mut buf2 = sine(8000.0, 0.4, 512);
        d.process(&mut buf2, 44_100);
        for s in buf2 {
            assert!(s.is_finite());
        }
    }
}
