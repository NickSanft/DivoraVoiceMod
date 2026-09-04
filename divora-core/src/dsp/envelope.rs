//! Shared one-pole envelope follower (v1.45.0) — the primitive five effects
//! had each grown their own copy of.
//!
//! Every follower in `dsp/` is the same branching one-pole:
//!
//! ```text
//! c   = exp(-1 / (tau_seconds * sample_rate))
//! env = c * env + (1 - c) * target
//! ```
//!
//! with `c` chosen per sample from an attack or a release time constant. What
//! actually differs between call sites is only *which direction counts as the
//! attack* — so pick the method by the direction the target moves, never by
//! the signal domain:
//!
//! * [`EnvelopeFollower::attack_on_rise`] — attack when the target moves UP.
//! * [`EnvelopeFollower::attack_on_fall`] — attack when the target moves DOWN.
//!
//! It happens that the five current callers split along domain lines — the
//! level followers (`Breath`, `VintageNoise`, `NoiseGate`) attack on a rise,
//! and the dB gain-reduction followers (`Compressor`, `DeEsser`) attack on a
//! fall, because "more reduction" drives their stored value down — but that
//! pairing is a coincidence of these call sites, not a rule. The makeup-gain
//! follower in `audio::loudness` is a counterexample: linear domain, attack on
//! fall.
//!
//! Hence two methods rather than one with a direction flag: each mirrors one
//! convention exactly, *including which coefficient a tie selects*, so lifting
//! an effect onto this type is bit-identical rather than merely equivalent.

/// Floor on a time constant so a zero (or negative) can never divide. Every
/// caller either passes a literal or a clamped parameter well above this, so
/// it is inert in practice — it exists because one of the five copies had
/// silently dropped its guard.
const MIN_TAU_SECS: f32 = 1e-5;

/// One-pole smoothing coefficient for a time constant in **seconds**.
#[must_use]
pub(crate) fn coeff_secs(seconds: f32, sample_rate: f32) -> f32 {
    (-1.0 / (seconds.max(MIN_TAU_SECS) * sample_rate)).exp()
}

/// One-pole smoothing coefficient for a time constant in **milliseconds**.
#[must_use]
pub(crate) fn coeff_ms(ms: f32, sample_rate: f32) -> f32 {
    coeff_secs(ms / 1000.0, sample_rate)
}

/// A single smoothed value with asymmetric attack/release timing.
///
/// One `f32` of state, no allocation, safe on the audio thread.
///
/// Deliberately **not** `Copy` and **not** `Default`. It is a mutable
/// accumulator: `let mut e = self.env;` — the natural borrow-checker
/// workaround — would silently take a *copy*, so every subsequent step would
/// mutate a temporary and the real follower would freeze at its last value.
/// That compiles cleanly and survives any single-buffer test. Without `Copy`
/// it is a compile error instead. `Default` is omitted for a related reason:
/// the correct rest value is call-site-specific (the gate's gain follower must
/// rest at 1.0, not 0.0, or a disabled gate would mute the signal).
#[derive(Debug, Clone)]
pub(crate) struct EnvelopeFollower {
    value: f32,
}

impl EnvelopeFollower {
    /// A follower resting at `initial`.
    pub(crate) const fn new(initial: f32) -> Self {
        Self { value: initial }
    }

    /// Force the stored value — used when an effect is disabled so a
    /// re-enable doesn't jump from stale state.
    pub(crate) const fn reset(&mut self, value: f32) {
        self.value = value;
    }

    /// Advance toward `target`, using `attack` when the target is **above**
    /// the stored value and `release` otherwise (ties take `release`).
    ///
    /// The linear-domain convention: louder means attack.
    pub(crate) fn attack_on_rise(&mut self, target: f32, attack: f32, release: f32) -> f32 {
        let c = if target > self.value { attack } else { release };
        self.step(target, c)
    }

    /// Advance toward `target`, using `attack` when the target is **below**
    /// the stored value and `release` otherwise (ties take `release`).
    ///
    /// The dB gain-reduction convention: more reduction means attack.
    pub(crate) fn attack_on_fall(&mut self, target: f32, attack: f32, release: f32) -> f32 {
        let c = if target < self.value { attack } else { release };
        self.step(target, c)
    }

    #[inline]
    fn step(&mut self, target: f32, c: f32) -> f32 {
        self.value = c * self.value + (1.0 - c) * target;
        self.value
    }
}

#[cfg(test)]
mod tests {
    use super::{coeff_ms, coeff_secs, EnvelopeFollower};

    #[test]
    fn coefficients_sit_strictly_inside_zero_and_one() {
        for &sr in &[8_000.0_f32, 44_100.0, 48_000.0, 96_000.0] {
            for &secs in &[0.001_f32, 0.008, 0.05, 0.18, 1.0] {
                let c = coeff_secs(secs, sr);
                assert!(
                    c > 0.0 && c < 1.0,
                    "coeff {c} out of range for {secs}s @ {sr}"
                );
            }
        }
    }

    #[test]
    fn seconds_and_milliseconds_agree() {
        let a = coeff_secs(0.06, 48_000.0);
        let b = coeff_ms(60.0, 48_000.0);
        assert!((a - b).abs() < 1e-9, "{a} vs {b}");
    }

    /// A zero or negative time constant must not divide by zero and produce a
    /// non-finite coefficient — the guard one of the original copies lacked.
    #[test]
    fn degenerate_time_constants_stay_finite() {
        for &bad in &[0.0_f32, -1.0, -0.0] {
            let c = coeff_secs(bad, 48_000.0);
            assert!(c.is_finite(), "coeff for {bad}s must stay finite, got {c}");
            assert!(
                (0.0..1.0).contains(&c),
                "coeff for {bad}s out of range: {c}"
            );
        }
    }

    /// A longer time constant must move less per sample than a short one.
    #[test]
    fn longer_time_constant_moves_more_slowly() {
        let fast = coeff_secs(0.002, 48_000.0);
        let slow = coeff_secs(0.200, 48_000.0);
        assert!(
            slow > fast,
            "slow {slow} should retain more than fast {fast}"
        );
    }

    #[test]
    fn attack_on_rise_picks_attack_going_up_and_release_coming_down() {
        // Attack fully open (c = 0 → snap to target), release fully closed
        // (c = 1 → hold), so which coefficient was used is unambiguous.
        let mut f = EnvelopeFollower::new(0.0);
        assert!(
            (f.attack_on_rise(1.0, 0.0, 1.0) - 1.0).abs() < 1e-6,
            "rise must snap"
        );
        // Now falling: release holds the value.
        assert!(
            (f.attack_on_rise(0.0, 0.0, 1.0) - 1.0).abs() < 1e-6,
            "fall must hold"
        );
    }

    #[test]
    fn attack_on_fall_inverts_the_branch() {
        let mut f = EnvelopeFollower::new(0.0);
        // Falling target uses `attack` (0.0 → snap).
        assert!(
            (f.attack_on_fall(-1.0, 0.0, 1.0) + 1.0).abs() < 1e-6,
            "fall must snap"
        );
        // Rising target uses `release` (1.0 → hold).
        assert!(
            (f.attack_on_fall(0.0, 0.0, 1.0) + 1.0).abs() < 1e-6,
            "rise must hold"
        );
    }

    /// A tie resolves to the same stored value under either method, so the two
    /// conventions cannot disagree when target == value.
    #[test]
    fn a_tie_leaves_the_value_untouched() {
        let mut up = EnvelopeFollower::new(0.5);
        let mut down = EnvelopeFollower::new(0.5);
        let a = up.attack_on_rise(0.5, 0.1, 0.9);
        let b = down.attack_on_fall(0.5, 0.1, 0.9);
        assert!((a - b).abs() < 1e-6, "tie must agree: {a} vs {b}");
    }

    #[test]
    fn converges_toward_a_held_target() {
        let c = coeff_secs(0.005, 48_000.0);
        let mut f = EnvelopeFollower::new(0.0);
        let mut last = 0.0;
        for _ in 0..4_800 {
            last = f.attack_on_rise(1.0, c, c);
        }
        assert!((last - 1.0).abs() < 0.01, "should approach 1.0, got {last}");
    }

    #[test]
    fn reset_forces_the_stored_value() {
        let mut f = EnvelopeFollower::new(0.0);
        f.attack_on_rise(1.0, 0.0, 1.0); // snap up to 1.0
        f.reset(0.0);
        // A coefficient of 1.0 holds, so the step reports the stored value.
        let held = f.attack_on_rise(9.0, 1.0, 1.0);
        assert!(held.abs() < 1e-9, "reset should have zeroed it, got {held}");
    }
}
