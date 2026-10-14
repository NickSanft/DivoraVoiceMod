//! The audio's clock, measured against the video's.
//!
//! Every engine buffer reaches the recorder with two numbers: how many
//! engine-rate frames came before it, and when its callback ran on the clock
//! the video uses. The audio device runs on its own crystal, so the two clocks
//! disagree — by up to about 100 ppm, which is 60 ms over ten minutes, more
//! than the whole sync budget. [`ClockFit`] fits a straight line through the
//! (frame, time) pairs so the timeline can resample the audio onto the video's
//! clock.
//!
//! It is a fit, not a follower, because each pair is noisy: a callback runs a
//! little early or late every time. Least squares averages that away. What
//! it must NOT average away is a **step** — the device stalled and the audio
//! really did stop for a while — so [`ClockFit::residual`] reports how far a
//! new pair is off the line, and the caller decides before adding it.
//!
//! Drift only: the slope is held to ±0.5 % of nominal, far outside any real
//! crystal, so a run of bad pairs can bend the line but never wildly.

/// Seconds of audio the fit needs before it trusts its own slope over the
/// nominal rate. Before then a handful of jittery pairs would swing it.
const MIN_FIT_SECONDS: f64 = 2.0;

/// How far the measured rate may stray from nominal, either way.
pub const MAX_DRIFT: f64 = 0.005;

/// A least-squares line `time = a + b · frame` through every pair so far, with
/// the slope clamped near nominal.
#[derive(Debug, Clone)]
pub struct ClockFit {
    /// Nominal frames per second.
    rate: f64,
    count: u64,
    // Welford's running means and co-moments: stable over the tens of
    // millions of frames in a long take, where Σn² would not be.
    mean_n: f64,
    mean_t: f64,
    m2_n: f64,
    c_nt: f64,
    first_n: f64,
    last_n: f64,
}

impl ClockFit {
    /// A fit for audio nominally at `rate` frames per second.
    #[must_use]
    pub fn new(rate: u32) -> Self {
        Self {
            rate: f64::from(rate.max(1)),
            count: 0,
            mean_n: 0.0,
            mean_t: 0.0,
            m2_n: 0.0,
            c_nt: 0.0,
            first_n: 0.0,
            last_n: 0.0,
        }
    }

    /// Whether any pair has been added.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    /// Seconds per frame: fitted once there is enough to fit, nominal before.
    fn slope(&self) -> f64 {
        let nominal = 1.0 / self.rate;
        let span = self.last_n - self.first_n;
        if self.count < 3 || span < MIN_FIT_SECONDS * self.rate || self.m2_n <= 0.0 {
            return nominal;
        }
        let fitted = self.c_nt / self.m2_n;
        fitted.clamp(nominal / (1.0 + MAX_DRIFT), nominal / (1.0 - MAX_DRIFT))
    }

    /// The time frame `n` sounds at, by the line. Meaningless while empty.
    #[must_use]
    pub fn time_of(&self, n: f64) -> f64 {
        self.mean_t + self.slope() * (n - self.mean_n)
    }

    /// The (fractional) frame sounding at time `t`.
    #[must_use]
    pub fn frame_at(&self, t: f64) -> f64 {
        self.mean_n + (t - self.mean_t) / self.slope()
    }

    /// Frames per second on the video's clock.
    #[must_use]
    pub fn measured_rate(&self) -> f64 {
        1.0 / self.slope()
    }

    /// How fast the audio clock runs against the video's, in parts per
    /// million: positive means it makes more frames per second than nominal.
    #[must_use]
    pub fn drift_ppm(&self) -> f64 {
        (self.measured_rate() / self.rate - 1.0) * 1e6
    }

    /// How far `(n, t)` lies off the line, in seconds — positive when it came
    /// later than the line says. `None` while there is no line yet.
    #[must_use]
    pub fn residual(&self, n: f64, t: f64) -> Option<f64> {
        (self.count > 0).then(|| t - self.time_of(n))
    }

    /// Add a pair.
    #[allow(clippy::cast_precision_loss)] // a count of pairs: exact far past any take
    pub fn add(&mut self, n: f64, t: f64) {
        if self.count == 0 {
            self.first_n = n;
        }
        self.count += 1;
        self.last_n = self.last_n.max(n);
        let k = self.count as f64;
        let dn = n - self.mean_n;
        self.mean_n += dn / k;
        self.mean_t += (t - self.mean_t) / k;
        // Welford: the co-moment uses the OLD mean for one factor and the new
        // one for the other.
        self.m2_n += dn * (n - self.mean_n);
        self.c_nt += dn * (t - self.mean_t);
    }
}

#[cfg(test)]
mod tests {
    use super::ClockFit;

    /// A small deterministic generator: the tests must not depend on a seed
    /// that changes between runs, or a marginal one would flake.
    struct Lcg(u64);

    impl Lcg {
        /// Uniform in [-1, 1).
        #[allow(clippy::cast_precision_loss)]
        fn next(&mut self) -> f64 {
            self.0 = self
                .0
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            ((self.0 >> 11) as f64 / (1u64 << 53) as f64) * 2.0 - 1.0
        }
    }

    /// Ten-millisecond buffers at `rate`, a clock `ppm` fast, every callback
    /// up to `jitter_s` early or late, for `seconds`.
    #[allow(
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss
    )]
    fn feed(fit: &mut ClockFit, rate: f64, ppm: f64, jitter_s: f64, seconds: f64, rng: &mut Lcg) {
        let buffer = (rate / 100.0).round();
        let callbacks = (seconds * 100.0) as u64;
        for k in 0..callbacks {
            let n = k as f64 * buffer;
            let t = n / (rate * (1.0 + ppm * 1e-6)) + jitter_s * rng.next();
            fit.add(n, t);
        }
    }

    #[test]
    fn it_starts_at_the_nominal_rate() {
        let mut fit = ClockFit::new(48_000);
        fit.add(0.0, 1.0);
        assert!((fit.time_of(48_000.0) - 2.0).abs() < 1e-12);
        assert!((fit.frame_at(1.5) - 24_000.0).abs() < 1e-6);
        assert!(fit.drift_ppm().abs() < 1e-9);
    }

    /// The plan's bar: +80 ppm found through ±5 ms of jitter on every pair.
    #[test]
    fn it_converges_on_80_ppm_through_5_ms_of_jitter() {
        let mut fit = ClockFit::new(48_000);
        let mut rng = Lcg(7);
        feed(&mut fit, 48_000.0, 80.0, 0.005, 60.0, &mut rng);
        let ppm = fit.drift_ppm();
        assert!((ppm - 80.0).abs() < 10.0, "found {ppm:.1} ppm");
        // And it keeps converging: ten minutes in, within a few ppm.
        let mut long = ClockFit::new(48_000);
        feed(&mut long, 48_000.0, 80.0, 0.005, 600.0, &mut rng);
        let ppm = long.drift_ppm();
        assert!(
            (ppm - 80.0).abs() < 2.0,
            "found {ppm:.2} ppm after ten minutes"
        );
        // Which is what matters: where the last frame sounds, to well under a
        // millisecond, despite every individual pair being up to 5 ms off.
        let n = 600.0 * 480.0 * 100.0;
        let truth = n / (48_000.0 * (1.0 + 80e-6));
        assert!((long.time_of(n) - truth).abs() < 0.001);
    }

    #[test]
    fn it_finds_slow_clocks_too_at_other_rates() {
        let mut fit = ClockFit::new(44_100);
        let mut rng = Lcg(11);
        feed(&mut fit, 44_100.0, -120.0, 0.002, 120.0, &mut rng);
        let ppm = fit.drift_ppm();
        assert!((ppm + 120.0).abs() < 5.0, "found {ppm:.1} ppm");
    }

    /// A 100 ms stall stands out from 5 ms of jitter by a wide margin, so the
    /// caller can tell one from the other.
    #[test]
    fn it_flags_a_100_ms_step() {
        let mut fit = ClockFit::new(48_000);
        let mut rng = Lcg(3);
        feed(&mut fit, 48_000.0, 0.0, 0.005, 10.0, &mut rng);
        let n = 10.0 * 48_000.0;
        let on_time = fit.residual(n, n / 48_000.0).unwrap();
        let stalled = fit.residual(n, n / 48_000.0 + 0.1).unwrap();
        assert!(on_time.abs() < 0.003, "on-time residual {on_time}");
        assert!((stalled - 0.1).abs() < 0.003, "stalled residual {stalled}");
    }

    /// Garbage in cannot drag the rate past the clamp.
    #[test]
    fn the_slope_is_clamped() {
        let mut fit = ClockFit::new(48_000);
        for k in 0..1_000u32 {
            let n = f64::from(k) * 480.0;
            fit.add(n, n / 48_000.0 * 1.5);
        }
        assert!(
            (fit.drift_ppm() + 5_000.0).abs() < 1.0,
            "{}",
            fit.drift_ppm()
        );
    }

    #[test]
    fn an_empty_fit_has_no_residual() {
        assert!(ClockFit::new(48_000).residual(0.0, 0.0).is_none());
    }
}
