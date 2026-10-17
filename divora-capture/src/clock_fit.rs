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
//!
//! A take's line runs through every pair it has seen. A crystal's drift is
//! not quite constant, though — it moves with the temperature — and a
//! straight line through hours of a slowly changing drift misses the newest
//! audio by more the longer it runs: Δ·T/12 for a drift that moves by Δ over
//! T, 6 ms for 10 ppm over two hours. Replay's timeline lives as long as the
//! engine runs, so its fit forgets: [`ClockFit::windowed`] fits only the
//! newest few minutes.

/// Seconds of audio the fit needs before it trusts its own slope over the
/// nominal rate. Before then a handful of jittery pairs would swing it.
const MIN_FIT_SECONDS: f64 = 2.0;

/// How far the measured rate may stray from nominal, either way.
pub const MAX_DRIFT: f64 = 0.005;

/// A least-squares line `time = a + b · frame` through every pair so far —
/// or, windowed, through the newest ones — with the slope clamped near
/// nominal.
#[derive(Debug, Clone)]
pub struct ClockFit {
    /// Nominal frames per second.
    rate: f64,
    /// Every pair so far; for a windowed fit, those since it last turned over.
    pairs: Moments,
    /// A windowed fit's previous half-window, once it has rotated.
    older: Option<Moments>,
    /// A windowed fit's half-window, in frames; `None` fits every pair.
    half: Option<f64>,
    last_n: f64,
}

/// Welford's running means and co-moments of a set of pairs: stable over
/// the tens of millions of frames in a long take, where Σn² would not be.
#[derive(Debug, Clone, Copy, Default)]
struct Moments {
    count: u64,
    mean_n: f64,
    mean_t: f64,
    m2_n: f64,
    c_nt: f64,
    /// The first pair's frame.
    first_n: f64,
}

impl Moments {
    #[allow(clippy::cast_precision_loss)] // a count of pairs: exact far past any take
    fn add(&mut self, n: f64, t: f64) {
        if self.count == 0 {
            self.first_n = n;
        }
        self.count += 1;
        let k = self.count as f64;
        let dn = n - self.mean_n;
        self.mean_n += dn / k;
        self.mean_t += (t - self.mean_t) / k;
        // Welford: the co-moment uses the OLD mean for one factor and the new
        // one for the other.
        self.m2_n += dn * (n - self.mean_n);
        self.c_nt += dn * (t - self.mean_t);
    }

    /// Both sets of pairs as one, by Chan's pairwise merge: the means move
    /// toward the newer set by its share of the pairs, and each co-moment
    /// gains the part the two means' distance explains.
    #[allow(clippy::cast_precision_loss)] // as `add`
    fn merged(&self, newer: &Self) -> Self {
        if newer.count == 0 {
            return *self;
        }
        let (a, b) = (self.count as f64, newer.count as f64);
        let k = a + b;
        let dn = newer.mean_n - self.mean_n;
        let dt = newer.mean_t - self.mean_t;
        Self {
            count: self.count + newer.count,
            mean_n: self.mean_n + dn * b / k,
            mean_t: self.mean_t + dt * b / k,
            m2_n: self.m2_n + newer.m2_n + dn * dn * a * b / k,
            c_nt: self.c_nt + newer.c_nt + dn * dt * a * b / k,
            first_n: self.first_n,
        }
    }
}

impl ClockFit {
    /// A fit for audio nominally at `rate` frames per second.
    #[must_use]
    pub fn new(rate: u32) -> Self {
        Self {
            rate: f64::from(rate.max(1)),
            pairs: Moments::default(),
            older: None,
            half: None,
            last_n: 0.0,
        }
    }

    /// A fit through only the newest `window_seconds` of frames, for a clock
    /// watched for hours.
    ///
    /// It keeps two halves: every `window_seconds / 2` of frames the newer
    /// half becomes the older and the oldest is forgotten, and the line runs
    /// through both — so through the newest half to whole window. Until the
    /// first such turn it **is** [`ClockFit::new`], bit for bit: one set of
    /// pairs, no merge arithmetic. A window under twice the seconds a slope
    /// needs ([`MIN_FIT_SECONDS`]) is raised to that, so a half on its own
    /// still has one.
    #[must_use]
    pub fn windowed(rate: u32, window_seconds: u32) -> Self {
        let mut fit = Self::new(rate);
        let window = f64::from(window_seconds).max(2.0 * MIN_FIT_SECONDS);
        fit.half = Some(window / 2.0 * fit.rate);
        fit
    }

    /// Whether any pair has been added.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.pairs.count == 0 && self.older.is_none()
    }

    /// The pairs the line runs through.
    fn line(&self) -> Moments {
        match &self.older {
            Some(older) => older.merged(&self.pairs),
            None => self.pairs,
        }
    }

    /// Seconds per frame through `m`: fitted once there is enough to fit,
    /// nominal before.
    fn slope_of(&self, m: &Moments) -> f64 {
        let nominal = 1.0 / self.rate;
        let span = self.last_n - m.first_n;
        if m.count < 3 || span < MIN_FIT_SECONDS * self.rate || m.m2_n <= 0.0 {
            return nominal;
        }
        let fitted = m.c_nt / m.m2_n;
        fitted.clamp(nominal / (1.0 + MAX_DRIFT), nominal / (1.0 - MAX_DRIFT))
    }

    fn slope(&self) -> f64 {
        self.slope_of(&self.line())
    }

    /// The time frame `n` sounds at, by the line. Meaningless while empty.
    #[must_use]
    pub fn time_of(&self, n: f64) -> f64 {
        let m = self.line();
        m.mean_t + self.slope_of(&m) * (n - m.mean_n)
    }

    /// The (fractional) frame sounding at time `t`.
    #[must_use]
    pub fn frame_at(&self, t: f64) -> f64 {
        let m = self.line();
        m.mean_n + (t - m.mean_t) / self.slope_of(&m)
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
        (!self.is_empty()).then(|| t - self.time_of(n))
    }

    /// Add a pair.
    pub fn add(&mut self, n: f64, t: f64) {
        // A windowed fit whose newer half is full turns over first, so the
        // newest pair always lands in the newer half.
        if let Some(half) = self.half {
            if self.pairs.count > 0 && n - self.pairs.first_n >= half {
                self.older = Some(std::mem::take(&mut self.pairs));
            }
        }
        self.last_n = self.last_n.max(n);
        self.pairs.add(n, t);
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
        assert!(ClockFit::windowed(48_000, 600).residual(0.0, 0.0).is_none());
    }

    /// Where frame `n` of a 48 kHz clock sounds when it runs 100 ppm fast,
    /// and 10 ppm faster still by the end of two hours: the slow change a
    /// warming crystal makes.
    fn ramp(n: f64) -> f64 {
        let u = n / 48_000.0;
        u - 1e-6 * (100.0 * u + 5.0 * u * u / 7_200.0)
    }

    /// A windowed fit is the global one until it first turns over, to the
    /// bit — so an epoch as short as a take, every golden included, is
    /// untouched by it — and over two hours of a drift that moves, it stays
    /// on the clock where the global line slides 6 ms off (Δ·T/12).
    #[test]
    #[allow(clippy::cast_precision_loss)]
    fn windowed_is_global_until_it_rotates() {
        let bits = |x: Option<f64>| x.map(f64::to_bits);
        let mut global = ClockFit::new(48_000);
        let mut windowed = ClockFit::windowed(48_000, 600);
        let mut rng = Lcg(5);
        // 290 s of +80 ppm through 5 ms of jitter: short of the turn at 300 s.
        for k in 0..29_000u32 {
            let n = f64::from(k) * 480.0;
            let t = n / (48_000.0 * (1.0 + 80e-6)) + 0.005 * rng.next();
            assert_eq!(
                bits(global.residual(n, t)),
                bits(windowed.residual(n, t)),
                "residual at pair {k}"
            );
            global.add(n, t);
            windowed.add(n, t);
            for probe in [0.0, n, n + 4_800.0] {
                assert_eq!(
                    global.time_of(probe).to_bits(),
                    windowed.time_of(probe).to_bits(),
                    "time_of({probe}) after pair {k}"
                );
            }
            assert_eq!(
                global.frame_at(t + 0.01).to_bits(),
                windowed.frame_at(t + 0.01).to_bits(),
                "frame_at after pair {k}"
            );
            assert_eq!(global.drift_ppm().to_bits(), windowed.drift_ppm().to_bits());
        }

        // Two hours of the ramp, a pair every 10 ms. The window turns every
        // 30 000 pairs (300 s), so at pair p its line runs through the pairs
        // from the start of the half before p's.
        let pairs: Vec<(f64, f64)> = (0..720_000u32)
            .map(|k| {
                let n = f64::from(k) * 480.0;
                (n, ramp(n))
            })
            .collect();
        let mut global = ClockFit::new(48_000);
        let mut windowed = ClockFit::windowed(48_000, 600);
        for (p, &(n, t)) in pairs.iter().enumerate() {
            global.add(n, t);
            windowed.add(n, t);
            if p % 60_000 != 59_999 {
                continue;
            }
            let off = windowed.time_of(n) - t;
            assert!(
                off.abs() < 1e-4,
                "{:.0} s in, the window is {:.3} ms off",
                t,
                off * 1e3
            );
            // The merge is exact: the same line as a fit fed only the
            // window's pairs.
            let from = (p / 30_000).saturating_sub(1) * 30_000;
            let mut direct = ClockFit::new(48_000);
            for &(n, t) in &pairs[from..=p] {
                direct.add(n, t);
            }
            let apart = windowed.time_of(n) - direct.time_of(n);
            assert!(apart.abs() < 1e-7, "the merge is {apart:e} s off at {p}");
        }
        // The control: the global line, through all two hours of it.
        let (n, t) = pairs[pairs.len() - 1];
        let off = global.time_of(n) - t;
        assert!(
            off.abs() > 0.005,
            "the global fit should be ~6 ms off: {:.3} ms",
            off * 1e3
        );
    }
}
