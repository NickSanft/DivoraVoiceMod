//! Output loudness normalization (auto-gain) + safety limiter — v1.7.0.
//!
//! ### What this is
//!
//! A global, **post-chain** output stage that keeps the *perceived* level
//! of the modulated voice steady regardless of which preset is active and
//! whether Voice Convert is engaged, then guarantees the result never
//! clips. It is deliberately **not** a chain effect: a chain effect would
//! be saved into each preset and reset on every preset switch — the exact
//! opposite of "stay level across presets." Instead the engine owns one
//! `LoudnessNormalizer` and applies it after the effect chain (see
//! `engine::mix_voice_and_soundboard`), controlled by persisted app-level
//! settings, exactly like the monitor / soundboard master gains.
//!
//! ### Signal flow (per sample)
//!
//!   1. **Measure**: an exponential moving average of `x²` tracks the
//!      short-term mean-square (≈ 300 ms window); its square root is the
//!      running RMS.
//!   2. **Auto-gain**: the makeup gain wanted to hit the target RMS is
//!      `target / rms`, clamped to a sane window (−12…+18 dB). It is
//!      smoothed with a fast *attack* (turn down quickly when the voice
//!      gets loud) and a slow *release* (turn up gently when it gets
//!      quiet), so speech doesn't pump. Below a noise floor the gain is
//!      **held** rather than cranked, so silence and room hiss aren't
//!      amplified.
//!   3. **Limiter**: a feed-forward brick-wall limiter with instantaneous
//!      attack and a short release pins peaks under a −1 dBFS ceiling. A
//!      final hard clamp to [−1, 1] is the last-resort safety net.
//!
//! ### Latency
//!
//! Zero. There is no look-ahead — the limiter reacts within the same
//! sample (and the hard clamp backstops it), so the stage adds nothing to
//! the engine's reported DSP latency.

#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]

/// Default target level, dBFS RMS. −18 dBFS is a comfortable speech target
/// that leaves headroom for transients under the limiter ceiling.
pub const DEFAULT_TARGET_DBFS: f32 = -18.0;
/// Lower bound the UI target is clamped to (very loud).
pub const MIN_TARGET_DBFS: f32 = -30.0;
/// Upper bound the UI target is clamped to (very quiet / gentle).
pub const MAX_TARGET_DBFS: f32 = -6.0;

/// Limiter ceiling, dBFS — peaks are pinned just under full scale.
const LIMITER_CEILING_DBFS: f32 = -1.0;
/// RMS below this (dBFS) is treated as silence/noise: the makeup gain is
/// held steady rather than boosting hiss into the target.
const NOISE_FLOOR_DBFS: f32 = -45.0;
/// Makeup-gain window. We never cut more than −12 dB or boost more than
/// +18 dB, so a momentary dropout can't slam the gain to an extreme.
const MIN_GAIN: f32 = 0.251_188_64; // 10^(-12/20)
const MAX_GAIN: f32 = 7.943_282_5; //  10^(+18/20)

// Smoothing time constants, in seconds.
const MS_TC: f32 = 0.30; // loudness measurement window
const GAIN_ATTACK_TC: f32 = 0.030; // turn the gain down quickly
const GAIN_RELEASE_TC: f32 = 0.400; // turn the gain up slowly
const LIM_RELEASE_TC: f32 = 0.100; // limiter recovery

/// Convert a dBFS value to a linear amplitude.
fn db_to_lin(db: f32) -> f32 {
    10.0_f32.powf(db / 20.0)
}

/// One-pole smoothing coefficient for a given time constant at `rate`.
/// `v += coeff * (target − v)` then approaches `target` with ~`tc` seconds.
fn one_pole(tc: f32, rate: u32) -> f32 {
    let rate = rate.max(1) as f32;
    1.0 - (-1.0 / (tc * rate)).exp()
}

/// Real-time auto-gain + brick-wall limiter. Engine-owned; lives on the
/// audio thread and is fed its parameters (enabled, target) once per
/// callback from the shared `EngineState` atomics.
pub struct LoudnessNormalizer {
    enabled: bool,
    /// Target RMS as a linear amplitude (derived from the dBFS target).
    target_rms: f32,
    /// Limiter ceiling, linear.
    ceiling: f32,
    /// Noise floor, linear — below this RMS the makeup gain is held.
    noise_floor: f32,

    // ---- state (audio-thread owned) ----
    /// Smoothed mean-square (envelope of x²).
    ms: f32,
    /// Current smoothed makeup gain.
    gain: f32,
    /// Current limiter gain reduction (≤ 1.0).
    limiter_gain: f32,

    // ---- sample-rate-dependent coefficients ----
    sample_rate: u32,
    ms_coeff: f32,
    gain_attack: f32,
    gain_release: f32,
    lim_release: f32,
}

impl LoudnessNormalizer {
    #[must_use]
    pub fn new() -> Self {
        let mut s = Self {
            enabled: false,
            target_rms: db_to_lin(DEFAULT_TARGET_DBFS),
            ceiling: db_to_lin(LIMITER_CEILING_DBFS),
            noise_floor: db_to_lin(NOISE_FLOOR_DBFS),
            ms: 0.0,
            gain: 1.0,
            limiter_gain: 1.0,
            sample_rate: 0,
            ms_coeff: 0.0,
            gain_attack: 0.0,
            gain_release: 0.0,
            lim_release: 0.0,
        };
        s.recompute(48_000);
        s
    }

    /// Recompute the smoothing coefficients for a new sample rate.
    fn recompute(&mut self, rate: u32) {
        self.sample_rate = rate;
        self.ms_coeff = one_pole(MS_TC, rate);
        self.gain_attack = one_pole(GAIN_ATTACK_TC, rate);
        self.gain_release = one_pole(GAIN_RELEASE_TC, rate);
        self.lim_release = one_pole(LIM_RELEASE_TC, rate);
    }

    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }

    /// Set the target level in dBFS (clamped to the supported window).
    pub fn set_target_dbfs(&mut self, dbfs: f32) {
        let clamped = dbfs.clamp(MIN_TARGET_DBFS, MAX_TARGET_DBFS);
        self.target_rms = db_to_lin(clamped);
    }

    #[must_use]
    pub fn enabled(&self) -> bool {
        self.enabled
    }

    /// The makeup gain currently applied, in dB. Drives the UI "it's
    /// working" readout (positive = boosting a quiet voice, negative =
    /// taming a loud one). Reports 0 dB while disabled.
    #[must_use]
    pub fn gain_db(&self) -> f32 {
        if self.enabled {
            20.0 * self.gain.max(1e-6).log10()
        } else {
            0.0
        }
    }

    /// Normalize `buffer` in place. No-op (bit-exact passthrough) while
    /// disabled. Self-heals on a sample-rate change.
    pub fn process(&mut self, buffer: &mut [f32], sample_rate: u32) {
        if sample_rate != self.sample_rate {
            self.recompute(sample_rate);
        }
        if !self.enabled {
            return;
        }

        for slot in buffer.iter_mut() {
            // Sanitize so a stray NaN/inf can never poison the running
            // mean-square (which would otherwise stick forever).
            let x = if slot.is_finite() { *slot } else { 0.0 };

            // 1. Measurement: EMA of x².
            self.ms += self.ms_coeff * (x * x - self.ms);
            let rms = self.ms.max(0.0).sqrt();

            // 2. Auto-gain toward the target, held below the noise floor.
            let desired = if rms > self.noise_floor {
                (self.target_rms / rms).clamp(MIN_GAIN, MAX_GAIN)
            } else {
                self.gain
            };
            // Fast attack when reducing, slow release when boosting.
            let coeff = if desired < self.gain {
                self.gain_attack
            } else {
                self.gain_release
            };
            self.gain += coeff * (desired - self.gain);
            let mut y = x * self.gain;

            // 3. Limiter: instant attack, smoothed release, hard backstop.
            let peak = y.abs();
            let lim_target = if peak > self.ceiling {
                self.ceiling / peak
            } else {
                1.0
            };
            if lim_target < self.limiter_gain {
                self.limiter_gain = lim_target;
            } else {
                self.limiter_gain += self.lim_release * (lim_target - self.limiter_gain);
            }
            y *= self.limiter_gain;

            *slot = y.clamp(-1.0, 1.0);
        }
    }
}

impl Default for LoudnessNormalizer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
#[allow(clippy::float_cmp)] // disabled path is a bit-exact passthrough
mod tests {
    use super::{db_to_lin, LoudnessNormalizer, DEFAULT_TARGET_DBFS};

    const SR: u32 = 48_000;

    /// RMS of a slice.
    fn rms(buf: &[f32]) -> f32 {
        if buf.is_empty() {
            return 0.0;
        }
        (buf.iter().map(|&s| s * s).sum::<f32>() / buf.len() as f32).sqrt()
    }

    /// Drive a steady sine of a given amplitude through the normalizer for
    /// `secs` seconds, returning the final 100 ms of output (after the
    /// gain has settled).
    fn settle(norm: &mut LoudnessNormalizer, amp: f32, secs: f32) -> Vec<f32> {
        let total = (SR as f32 * secs) as usize;
        let mut tail = Vec::new();
        let tail_start = total.saturating_sub(SR as usize / 10);
        let mut block = vec![0f32; 256];
        let mut n = 0usize;
        while n < total {
            for (i, s) in block.iter_mut().enumerate() {
                let t = (n + i) as f32 / SR as f32;
                *s = amp * (2.0 * std::f32::consts::PI * 220.0 * t).sin();
            }
            norm.process(&mut block, SR);
            for (i, &s) in block.iter().enumerate() {
                if n + i >= tail_start {
                    tail.push(s);
                }
            }
            n += block.len();
        }
        tail
    }

    #[test]
    fn disabled_is_bit_exact_passthrough() {
        let mut norm = LoudnessNormalizer::new();
        let mut buf = [0.5_f32, -0.5, 0.25, -0.25, 0.9];
        let before = buf;
        norm.process(&mut buf, SR);
        assert_eq!(buf, before);
    }

    #[test]
    fn quiet_input_is_boosted_toward_target() {
        let mut norm = LoudnessNormalizer::new();
        norm.set_enabled(true);
        let target = db_to_lin(DEFAULT_TARGET_DBFS);
        // A −36 dBFS-ish sine (amp 0.02 → RMS ≈ 0.0141) should be lifted
        // up toward the −18 dBFS target.
        let tail = settle(&mut norm, 0.02, 3.0);
        let out = rms(&tail);
        let quiet_in = 0.02 / std::f32::consts::SQRT_2;
        assert!(
            out > quiet_in * 2.0,
            "should boost a quiet voice (out={out})"
        );
        // Lands in the target neighborhood (within ~3 dB).
        assert!(
            out > target * 0.7 && out < target * 1.4,
            "near target (out={out}, target={target})"
        );
        assert!(norm.gain_db() > 6.0, "reports positive makeup gain");
    }

    #[test]
    fn loud_input_is_attenuated_toward_target() {
        let mut norm = LoudnessNormalizer::new();
        norm.set_enabled(true);
        let target = db_to_lin(DEFAULT_TARGET_DBFS);
        // A hot −6 dBFS sine (amp 0.5 → RMS ≈ 0.354) should be pulled down.
        let tail = settle(&mut norm, 0.5, 3.0);
        let out = rms(&tail);
        let loud_in = 0.5 / std::f32::consts::SQRT_2;
        assert!(out < loud_in * 0.8, "should tame a loud voice (out={out})");
        assert!(
            out > target * 0.6 && out < target * 1.5,
            "near target (out={out}, target={target})"
        );
        assert!(norm.gain_db() < 0.0, "reports negative makeup gain");
    }

    #[test]
    fn limiter_keeps_peaks_under_full_scale() {
        let mut norm = LoudnessNormalizer::new();
        norm.set_enabled(true);
        // Way-too-hot input: the makeup gain bottoms out at −12 dB but the
        // signal is still over full scale, so the limiter must catch it.
        let tail = settle(&mut norm, 4.0, 2.0);
        let max_peak = tail.iter().fold(0.0_f32, |m, &s| m.max(s.abs()));
        assert!(
            max_peak <= 1.0,
            "never exceeds full scale (peak={max_peak})"
        );
    }

    #[test]
    fn silence_is_not_boosted() {
        let mut norm = LoudnessNormalizer::new();
        norm.set_enabled(true);
        // Pure silence: gain must stay held at unity (no hiss-cranking).
        let mut buf = vec![0f32; SR as usize]; // 1 s of zeros
        norm.process(&mut buf, SR);
        assert!(buf.iter().all(|&s| s == 0.0), "silence stays silent");
        // Makeup gain held at its starting unity, not slammed to +18 dB.
        assert!(
            norm.gain_db().abs() < 1e-3,
            "gain held at unity over silence"
        );
    }

    #[test]
    fn stays_finite_on_extreme_and_nonfinite_input() {
        let mut norm = LoudnessNormalizer::new();
        norm.set_enabled(true);
        // DC, huge values, and a NaN/inf mixed in must not poison state.
        let mut buf = [1.0_f32, 1.0, 1e9, -1e9, f32::NAN, f32::INFINITY, 0.3];
        for _ in 0..10 {
            norm.process(&mut buf, SR);
            // refresh the poisoned slots the way a real stream would
            buf = [1.0, 1.0, 1e9, -1e9, f32::NAN, f32::INFINITY, 0.3];
        }
        norm.process(&mut buf, SR);
        assert!(
            buf.iter().all(|s| s.is_finite()),
            "output is always finite: {buf:?}"
        );
        assert!(buf.iter().all(|&s| s.abs() <= 1.0), "and bounded to [-1,1]");
    }

    #[test]
    fn recomputes_on_sample_rate_change_without_panic() {
        let mut norm = LoudnessNormalizer::new();
        norm.set_enabled(true);
        let mut buf = [0.1_f32; 128];
        norm.process(&mut buf, 44_100);
        let mut buf2 = [0.1_f32; 128];
        norm.process(&mut buf2, 48_000);
        assert!(buf2.iter().all(|s| s.is_finite()));
    }

    #[test]
    fn target_clamps_to_supported_window() {
        let mut norm = LoudnessNormalizer::new();
        // Absurd targets clamp into [-30, -6] dBFS internally; no panic and
        // a finite target_rms either way.
        norm.set_target_dbfs(100.0);
        norm.set_enabled(true);
        let mut buf = [0.2_f32; 256];
        norm.process(&mut buf, SR);
        assert!(buf.iter().all(|s| s.is_finite()));
        norm.set_target_dbfs(-200.0);
        let mut buf2 = [0.2_f32; 256];
        norm.process(&mut buf2, SR);
        assert!(buf2.iter().all(|s| s.is_finite()));
    }
}
