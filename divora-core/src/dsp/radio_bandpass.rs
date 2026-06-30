//! Vintage-radio band-pass (v1.32.0) — a *real* band-limiter for the
//! "through an old radio" sound the gentle [`super::Eq`] shelves can't make.
//!
//! Two cascaded high-pass + two cascaded low-pass biquads carve a steep
//! ~24 dB/oct pass-band (the limited-bandwidth "device" cue — a wall, not a
//! slope), and a movable high-Q peaking band adds the midrange "cone/horn"
//! resonance that reads as a physical speaker (and doubles as intelligibility
//! insurance, since it sits in the 1-3 kHz consonant band). Sample-by-sample
//! biquads → **zero latency**. Place it AFTER distortion so the low-pass
//! re-tames the saturator's harmonics like a real cone.

use biquad::{Biquad, Coefficients, DirectForm1, ToHertz, Type};

use super::{AudioEffect, EffectKind};

/// Q per cascaded pass stage. Two identical 0.707 stages give a steep
/// ~24 dB/oct rolloff with a touch of edge resonance — which is exactly how
/// a real radio's band edges behave.
const STAGE_Q: f32 = 0.707;

pub struct RadioBandpass {
    enabled: bool,
    hp_hz: f32,
    lp_hz: f32,
    peak_hz: f32,
    peak_db: f32,
    peak_q: f32,
    hp1: DirectForm1<f32>,
    hp2: DirectForm1<f32>,
    lp1: DirectForm1<f32>,
    lp2: DirectForm1<f32>,
    peak: DirectForm1<f32>,
    sample_rate: u32,
}

impl RadioBandpass {
    #[must_use]
    pub fn new() -> Self {
        let sr = 48_000_u32;
        let mut b = Self {
            enabled: false,
            hp_hz: 300.0,
            lp_hz: 3000.0,
            peak_hz: 1500.0,
            peak_db: 0.0,
            peak_q: 1.0,
            hp1: DirectForm1::<f32>::new(pass_coeffs(sr, Type::HighPass, 300.0)),
            hp2: DirectForm1::<f32>::new(pass_coeffs(sr, Type::HighPass, 300.0)),
            lp1: DirectForm1::<f32>::new(pass_coeffs(sr, Type::LowPass, 3000.0)),
            lp2: DirectForm1::<f32>::new(pass_coeffs(sr, Type::LowPass, 3000.0)),
            peak: DirectForm1::<f32>::new(peak_coeffs(sr, 1500.0, 0.0, 1.0)),
            sample_rate: sr,
        };
        b.rebuild();
        b
    }

    fn rebuild(&mut self) {
        let sr = self.sample_rate;
        let hp = pass_coeffs(sr, Type::HighPass, self.hp_hz);
        self.hp1.update_coefficients(hp);
        self.hp2.update_coefficients(hp);
        let lp = pass_coeffs(sr, Type::LowPass, self.lp_hz);
        self.lp1.update_coefficients(lp);
        self.lp2.update_coefficients(lp);
        self.peak
            .update_coefficients(peak_coeffs(sr, self.peak_hz, self.peak_db, self.peak_q));
    }
}

/// `biquad` 0.5.0 quirk: `from_params` normalizes as `f0 / (2·fs)` while using
/// `omega = π·normalized`, so the real corner lands at **`f0/4`**. We pass `4·f`
/// so the cutoff sits at the intended frequency. (The older `eq`/`deesser`
/// effects don't compensate — their nominal Hz are 4× the real ones — but a
/// band-pass needs its corners exactly where the label says.)
fn corner_f0(sr: u32, freq_hz: f32) -> f32 {
    #[allow(clippy::cast_precision_loss)]
    let nyq = (sr as f32) * 0.5;
    // Clamp the *intended* corner below Nyquist, then ×4 for the crate.
    freq_hz.clamp(20.0, nyq * 0.95) * 4.0
}

/// High-/low-pass stage coefficients (corner at the intended Hz; safe fallback).
fn pass_coeffs(sr: u32, ty: Type<f32>, freq_hz: f32) -> Coefficients<f32> {
    Coefficients::<f32>::from_params(ty, sr.hz(), corner_f0(sr, freq_hz).hz(), STAGE_Q)
        .or_else(|_| {
            Coefficients::<f32>::from_params(ty, 48_000_u32.hz(), 4000.0_f32.hz(), STAGE_Q)
        })
        .expect("band-pass stage coefficients should build at the fallback corner")
}

fn peak_coeffs(sr: u32, freq_hz: f32, gain_db: f32, q: f32) -> Coefficients<f32> {
    let q = q.clamp(0.2, 12.0);
    Coefficients::<f32>::from_params(
        Type::PeakingEQ(gain_db),
        sr.hz(),
        corner_f0(sr, freq_hz).hz(),
        q,
    )
    .or_else(|_| {
        Coefficients::<f32>::from_params(
            Type::PeakingEQ(0.0),
            48_000_u32.hz(),
            6000.0_f32.hz(),
            1.0,
        )
    })
    .expect("peak coefficients should build at the fallback")
}

impl Default for RadioBandpass {
    fn default() -> Self {
        Self::new()
    }
}

impl AudioEffect for RadioBandpass {
    fn process(&mut self, buffer: &mut [f32], sample_rate: u32) {
        if sample_rate != self.sample_rate {
            self.sample_rate = sample_rate;
            self.rebuild();
        }
        for sample in buffer.iter_mut() {
            let x = if sample.is_finite() { *sample } else { 0.0 };
            let y = self
                .peak
                .run(self.lp2.run(self.lp1.run(self.hp2.run(self.hp1.run(x)))));
            *sample = if y.is_finite() { y } else { 0.0 };
        }
    }

    fn set_param(&mut self, key: &str, value: f32) {
        match key {
            "hp" => self.hp_hz = value.clamp(50.0, 2000.0),
            "lp" => self.lp_hz = value.clamp(800.0, 12_000.0),
            "peak" => self.peak_hz = value.clamp(100.0, 6000.0),
            // v1.34.0: allow NEGATIVE gain so a second instance can CUT a
            // resonance (a true notch) — e.g. the Villager voice's ~1.05 kHz
            // nasal antiformant. PeakingEQ coefficients accept negative dB.
            "gain" => self.peak_db = value.clamp(-18.0, 18.0),
            "q" => self.peak_q = value.clamp(0.2, 12.0),
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
        EffectKind::RadioBandpass
    }
    // latency_samples: default 0 — sample-by-sample biquads, no look-ahead.
}

#[cfg(test)]
mod tests {
    use super::{AudioEffect, RadioBandpass};

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

    /// A tone in the pass-band survives; tones below the high-pass and above
    /// the low-pass are strongly attenuated (the band-limit "wall").
    #[test]
    fn band_limits_outside_the_passband() {
        let mut b = RadioBandpass::new();
        b.set_enabled(true);
        b.set_param("hp", 300.0);
        b.set_param("lp", 3000.0);
        b.set_param("gain", 0.0);
        let mut mid = |f: f32| {
            let mut buf = sine(f, 0.5, 24_000);
            b.process(&mut buf, 48_000);
            rms(&buf[12_000..]) // settled
        };
        let pass = mid(1000.0);
        let low = mid(80.0);
        let high = mid(8000.0);
        assert!(pass > 0.2, "1 kHz should pass (rms {pass})");
        assert!(
            low < pass * 0.25,
            "80 Hz cut hard (low {low} vs pass {pass})"
        );
        assert!(
            high < pass * 0.25,
            "8 kHz cut hard (high {high} vs pass {pass})"
        );
    }

    /// The peaking band lifts its centre frequency.
    #[test]
    fn resonant_peak_boosts_its_band() {
        let centre = 1500.0;
        let measure = |gain: f32| {
            let mut b = RadioBandpass::new();
            b.set_enabled(true);
            b.set_param("hp", 200.0);
            b.set_param("lp", 5000.0);
            b.set_param("peak", centre);
            b.set_param("q", 3.0);
            b.set_param("gain", gain);
            let mut buf = sine(centre, 0.3, 24_000);
            b.process(&mut buf, 48_000);
            rms(&buf[12_000..])
        };
        assert!(
            measure(10.0) > measure(0.0) * 1.3,
            "peak should boost its band"
        );
    }

    #[test]
    fn stays_finite_on_extreme_input() {
        let mut b = RadioBandpass::new();
        b.set_enabled(true);
        let mut buf = [f32::NAN, f32::INFINITY, -f32::INFINITY, 1e30, -1e30, 0.0];
        b.process(&mut buf, 48_000);
        for s in buf {
            assert!(s.is_finite(), "output must stay finite, got {s}");
        }
    }

    #[test]
    fn adds_no_latency() {
        assert_eq!(RadioBandpass::new().latency_samples(48_000), 0);
    }

    #[test]
    fn handles_sample_rate_change() {
        let mut b = RadioBandpass::new();
        b.set_enabled(true);
        let mut buf = sine(1000.0, 0.4, 512);
        b.process(&mut buf, 48_000);
        let mut buf2 = sine(1000.0, 0.4, 512);
        b.process(&mut buf2, 44_100);
        for s in buf2 {
            assert!(s.is_finite());
        }
    }
}
