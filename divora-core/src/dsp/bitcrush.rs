//! Bitcrusher (v1.48.0) — lo-fi digital destruction: amplitude quantisation
//! (bit-depth reduction) plus sample-and-hold decimation (rate reduction).
//!
//! Deliberately NOT another mode on [`super::Distortion`], which is a smooth
//! `tanh` waveshaper. A waveshaper's products are **harmonic** — they sit at
//! integer multiples of the speaker's pitch, so they read as "overdriven".
//! Decimating without a band-limiting filter folds everything above half the
//! target rate back down as mirror images, so those partials are
//! **inharmonic** and move DOWNWARD as the speaker's F0 rises. That contrary
//! motion is the entire "broken machine / 8-bit" cue, and a waveshaper cannot
//! produce it at any setting.
//!
//! ### The aliasing is the product, not a defect
//!
//! There is no anti-alias filter here and there should not be one. Filtering
//! the fold-down away removes exactly the thing being emulated (early digital
//! gear aliased, which is why it sounded like that). The few plugins that do
//! filter — Decimort, Kilohearts — are modelling specific vintage samplers
//! that had real reconstruction filters, and they sell it as a separate
//! feature. Please do not "fix" this.
//!
//! ### Rate is a target in Hz, never a sample divisor
//!
//! A naive bitcrusher holds every Nth sample. That makes the audible crush
//! frequency depend on the device rate — "divide by 4" is 12 kHz at 48 kHz but
//! 11.025 kHz at 44.1 kHz, so the same preset sounds different on different
//! hardware. This crate treats that as a defect (see the reverb's comb retune
//! and the `exp(-1 / (tau * sr))` envelope coefficients). So the rate is a
//! target frequency driving a fractional phase accumulator, and the hold
//! period is constant in *seconds* at any device rate. It needs no retune hook
//! at all, because the increment is re-derived from `sample_rate` every block.
//! A target at or above the device rate bypasses the decimation exactly (the
//! quantiser still applies, so the residual is at most half a step).
//!
//! Zero latency, no allocation, sample-by-sample: RT-safe.

use super::{AudioEffect, EffectKind};

/// Maximum pre-quantiser gain, in dB, at `drive` = 100.
///
/// Drive earns its place on a *voice* crusher specifically. Mid-tread
/// quantisation has a dead zone of ±delta/2 below which everything is silenced
/// — about −24 dBFS at 4 bits — and speech has ~40 dB of intra-utterance
/// range, so without pre-gain a low bit depth swallows whole syllables and
/// reads as a broken gate rather than as "crushed".
const DRIVE_MAX_DB: f32 = 24.0;

pub struct Bitcrush {
    enabled: bool,
    /// Quantiser resolution in whole bits (stored as f32 for the param API,
    /// but rounded on the way in — see `set_param`).
    bits: f32,
    /// Sample-and-hold target, in Hz.
    rate_hz: f32,
    /// Pre-quantiser gain, 0..1 → 0..`DRIVE_MAX_DB`.
    drive: f32,
    /// Dry/wet, 0..1.
    mix: f32,
    // --- state ---
    /// Phase accumulator; a hold fires when it reaches 1. Sits in (0, 1] after
    /// the first firing, except in the transparent case (target >= device
    /// rate) where it alternates 1.0 / 2.0 and fires every sample.
    phase: f32,
    /// The currently held (and quantised) sample.
    hold: f32,
}

impl Bitcrush {
    #[must_use]
    pub fn new() -> Self {
        Self {
            enabled: false,
            bits: 8.0,
            rate_hz: 8000.0,
            drive: 0.0,
            mix: 1.0,
            // Starts AT the trigger point, not at 0: with phase 0 the first
            // sample after enabling takes the else-branch and emits the stale
            // hold, so every toggle-on would leak a few samples of silence.
            phase: 1.0,
            hold: 0.0,
        }
    }
}

impl Default for Bitcrush {
    fn default() -> Self {
        Self::new()
    }
}

impl AudioEffect for Bitcrush {
    fn process(&mut self, buffer: &mut [f32], sample_rate: u32) {
        #[allow(clippy::cast_precision_loss)]
        let sr = sample_rate.max(1) as f32;
        // Clamped to 1.0 so a target at or above the device rate bypasses the
        // DECIMATOR exactly — every sample triggers a fresh hold — rather than
        // through a special-cased branch that could drift from the main path.
        // Note the quantiser still runs, so the effect as a whole is never a
        // bit-identity; the residual is bounded by delta/2.
        let inc = (self.rate_hz / sr).min(1.0);
        // Full scale spans 2.0 (−1..1), so the step is 2/2^bits.
        let delta = 2.0 / self.bits.exp2();
        let drive_gain = 10_f32.powf(self.drive * DRIVE_MAX_DB / 20.0);

        for sample in buffer.iter_mut() {
            let x = if sample.is_finite() { *sample } else { 0.0 };

            self.phase += inc;
            if self.phase >= 1.0 {
                // One subtraction always suffices. For inc < 1 the phase lands
                // in (0, 1]; for inc == 1 (the transparent case) it simply
                // oscillates 2.0 -> 1.0 -> 2.0, re-firing on every sample,
                // which is exactly the wanted behaviour. Either way it never
                // needs a wrap loop or a `floor`.
                self.phase -= 1.0;
                // Clamping first models converter overload (which is what
                // makes `drive` musical) and bounds the output for free.
                let driven = (x * drive_gain).clamp(-1.0, 1.0);
                // Mid-tread: `round`, not `floor`. The classic musicdsp form
                // `floor(x * m) / m` biases every sample down by delta/2 —
                // a DC offset reaching HALF of full scale at 1 bit, which on a
                // live mic feed would thump speakers and confuse the gate.
                // Mid-riser is wrong here too: it emits a constant ±delta/2 on
                // silence, so every pause between words would carry DC.
                // Mid-tread maps 0 to exactly 0.
                self.hold = (driven / delta).round() * delta;
            }

            // Quantising inside the trigger branch is not an optimisation
            // detail — decimate-then-quantise and quantise-then-decimate are
            // the same expression, Q(x[m(n)]), so this is identical output at
            // 1/N the work. That equivalence dies the moment dither, error
            // feedback, or a filter is introduced between the two stages.
            let out = x.mul_add(1.0 - self.mix, self.hold * self.mix);
            *sample = if out.is_finite() { out } else { x };
        }
    }

    fn set_param(&mut self, key: &str, value: f32) {
        match key {
            // Rounded to whole bits, not merely clamped. With a fractional
            // depth `delta` stops being a power of two, so the quantiser can
            // land OUTSIDE [-1, 1] — measured +-1.23 at 1.7 bits — which breaks
            // the output bound the input clamp is supposed to guarantee. The
            // slider steps by 1 anyway; this closes the hand-edited-preset and
            // raw-SetParam paths.
            "bits" => self.bits = value.clamp(1.0, 16.0).round(),
            // Top of the range is 48000, not 44100: on a 48 kHz device a
            // 44100 target gives inc = 0.919, i.e. hold lengths alternating
            // aperiodically between 1 and 2 samples — an audible, hard-to-
            // attribute stutter. 48000 is an exact bypass on both common rates.
            "rate" => self.rate_hz = value.clamp(1000.0, 48_000.0),
            "drive" => self.drive = (value / 100.0).clamp(0.0, 1.0),
            "mix" => self.mix = (value / 100.0).clamp(0.0, 1.0),
            _ => {}
        }
    }

    fn enabled(&self) -> bool {
        self.enabled
    }

    fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
        if !enabled {
            // Back to the trigger point so a re-enable starts cleanly rather
            // than emitting a stale held sample.
            self.phase = 1.0;
            self.hold = 0.0;
        }
    }

    fn kind(&self) -> EffectKind {
        EffectKind::Bitcrush
    }
    // latency_samples: default 0 — sample-by-sample, no buffering.
}

#[cfg(test)]
mod tests {
    use super::{AudioEffect, Bitcrush};

    fn crusher(bits: f32, rate: f32, mix: f32) -> Bitcrush {
        let mut b = Bitcrush::new();
        b.set_enabled(true);
        b.set_param("bits", bits);
        b.set_param("rate", rate);
        b.set_param("mix", mix);
        b
    }

    /// Count how many times the output value changes — i.e. how many distinct
    /// held segments were produced.
    fn holds(buf: &[f32]) -> usize {
        let mut n = 0;
        let mut last = f32::NAN;
        for &s in buf {
            // Exact comparison is deliberate: a sample-and-hold repeats the
            // SAME bits until the next trigger, so "changed" means literally
            // a different value, not "differs by more than epsilon".
            #[allow(clippy::float_cmp)]
            let changed = s != last;
            if changed {
                n += 1;
                last = s;
            }
        }
        n
    }

    /// THE headline test: the crush rate is a frequency, so the number of held
    /// segments PER SECOND must equal the target regardless of device rate.
    ///
    /// A naive integer-divisor implementation fails this — which is the entire
    /// point of parameterising in Hz.
    #[test]
    fn the_crush_rate_is_the_same_in_hz_at_any_device_rate() {
        let target = 6000.0_f32;
        let segments_per_second = |sr: u32| {
            let mut b = crusher(16.0, target, 100.0);
            // Exactly one second of a ramp, so consecutive holds differ.
            let n = sr as usize;
            #[allow(clippy::cast_precision_loss)]
            let mut buf: Vec<f32> = (0..n).map(|i| (i as f32 / n as f32) * 2.0 - 1.0).collect();
            b.process(&mut buf, sr);
            holds(&buf)
        };
        let at_44 = segments_per_second(44_100);
        let at_48 = segments_per_second(48_000);
        #[allow(clippy::cast_precision_loss)]
        let drift = (at_44 as f32 - at_48 as f32).abs();
        assert!(
            drift <= 2.0,
            "hold rate must be device-independent: {at_44} vs {at_48} segments/s"
        );
        // And it must actually be the requested frequency, not merely equal.
        #[allow(clippy::cast_precision_loss)]
        let err = (at_48 as f32 - target).abs();
        assert!(err <= 2.0, "expected ~{target} segments/s, got {at_48}");
    }

    /// A target at or above the device rate is an exact identity, not an
    /// approximation — every input sample triggers a fresh hold.
    #[test]
    fn a_target_at_the_device_rate_is_transparent() {
        let mut b = crusher(16.0, 48_000.0, 100.0);
        #[allow(clippy::cast_precision_loss)]
        let input: Vec<f32> = (0..512).map(|i| (i as f32 * 0.01).sin() * 0.7).collect();
        let mut buf = input.clone();
        b.process(&mut buf, 48_000);
        // HALF a step is the true mid-tread bound, and asserting the real
        // bound rather than a loose one earns a second guard for free: a
        // `floor` quantiser errs by up to a full step and fails this.
        let half_step = 1.0 / 16.0_f32.exp2();
        for (i, (&a, &c)) in input.iter().zip(&buf).enumerate() {
            assert!(
                (a - c).abs() <= half_step,
                "sample {i} should pass through: {a} vs {c}"
            );
        }
    }

    /// Mid-tread quantisation maps zero to exactly zero. Mid-riser would put a
    /// constant ±delta/2 on silence — DC on every pause between words, into a
    /// live virtual-mic feed.
    #[test]
    fn silence_stays_silent_with_no_dc_offset() {
        let mut b = crusher(3.0, 4000.0, 100.0);
        let mut buf = vec![0.0_f32; 512];
        b.process(&mut buf, 48_000);
        for &s in &buf {
            assert!(s.abs() < 1e-9, "silence must not acquire DC, got {s}");
        }
    }

    /// At one bit the output can only be −1, 0 or +1. This is the canary for a
    /// future switch to `floor` (which would bias everything down by half a
    /// step) or to mid-riser (which would drop the zero level).
    #[test]
    fn one_bit_output_is_only_minus_one_zero_or_plus_one() {
        let mut b = crusher(1.0, 48_000.0, 100.0);
        #[allow(clippy::cast_precision_loss)]
        let mut buf: Vec<f32> = (0..400).map(|i| (i as f32 * 0.05).sin()).collect();
        b.process(&mut buf, 48_000);
        for &s in &buf {
            let ok = (s + 1.0).abs() < 1e-6 || s.abs() < 1e-6 || (s - 1.0).abs() < 1e-6;
            assert!(ok, "1-bit output must be -1/0/+1, got {s}");
        }
    }

    /// The real mid-tread canary. At 2 bits (delta = 0.5) an input of 0.3
    /// rounds to 0.5; `floor` would give 0.0 and mid-riser 0.25. The 1-bit
    /// test above does NOT catch a switch to `floor` — at 1 bit floor yields
    /// {-1, 0}, which satisfies its assertion — so this is the one that pins
    /// the rounding mode.
    #[test]
    fn quantisation_rounds_to_nearest_rather_than_down() {
        let mut b = crusher(2.0, 48_000.0, 100.0);
        let mut buf = [0.3_f32, -0.3, 0.1, -0.1];
        b.process(&mut buf, 48_000);
        assert!((buf[0] - 0.5).abs() < 1e-6, "0.3 -> 0.5, got {}", buf[0]);
        assert!((buf[1] + 0.5).abs() < 1e-6, "-0.3 -> -0.5, got {}", buf[1]);
        // And symmetric about zero, which mid-riser would break.
        assert!((buf[2]).abs() < 1e-6, "0.1 -> 0, got {}", buf[2]);
        assert!((buf[3]).abs() < 1e-6, "-0.1 -> 0, got {}", buf[3]);
    }

    /// The hold must carry ACROSS `process()` calls. Real audio arrives in
    /// 128–512 sample callbacks, and an implementation that reset `phase` at
    /// the top of each block — a plausible refactor, since the coefficients
    /// already are — would pass every other test here while adding a spurious
    /// trigger at every buffer boundary (an audible whine at the block rate).
    #[test]
    fn the_hold_carries_across_buffer_boundaries() {
        let render = |chunk: usize| {
            let mut b = crusher(16.0, 6000.0, 100.0);
            #[allow(clippy::cast_precision_loss)]
            let mut buf: Vec<f32> = (0..48_000)
                .map(|i| (i as f32 / 48_000.0) * 2.0 - 1.0)
                .collect();
            let mut at = 0;
            while at < buf.len() {
                let end = (at + chunk).min(buf.len());
                b.process(&mut buf[at..end], 48_000);
                at = end;
            }
            buf
        };
        let whole = render(48_000);
        let chunked = render(128);
        assert_eq!(whole.len(), chunked.len());
        for (i, (&a, &c)) in whole.iter().zip(&chunked).enumerate() {
            assert!(
                (a - c).abs() < 1e-9,
                "block size must not change the output at sample {i}: {a} vs {c}"
            );
        }
    }

    /// Coarser quantisation must actually be coarser — fewer distinct output
    /// values for the same input.
    #[test]
    fn fewer_bits_produce_fewer_distinct_levels() {
        let distinct = |bits: f32| {
            let mut b = crusher(bits, 48_000.0, 100.0);
            #[allow(clippy::cast_precision_loss)]
            let mut buf: Vec<f32> = (0..2000).map(|i| (i as f32 * 0.01).sin() * 0.9).collect();
            b.process(&mut buf, 48_000);
            let mut seen: Vec<f32> = buf.clone();
            seen.sort_by(|a, c| a.partial_cmp(c).unwrap());
            seen.dedup_by(|a, c| (*a - *c).abs() < 1e-9);
            seen.len()
        };
        let coarse = distinct(3.0);
        let fine = distinct(10.0);
        assert!(
            coarse < fine,
            "3 bits should quantise harder than 10 ({coarse} vs {fine} levels)"
        );
    }

    #[test]
    fn zero_mix_is_passthrough() {
        let mut b = crusher(2.0, 2000.0, 0.0);
        let input: Vec<f32> = vec![0.31, -0.62, 0.09, 0.88, -0.44];
        let mut buf = input.clone();
        b.process(&mut buf, 48_000);
        for (&a, &c) in input.iter().zip(&buf) {
            assert!((a - c).abs() < 1e-6, "mix 0 must pass dry: {a} vs {c}");
        }
    }

    #[test]
    fn drive_pushes_more_signal_over_the_dead_zone() {
        // A quiet signal at a coarse depth sits inside the mid-tread dead zone
        // and is silenced; drive is what rescues it. This is the reason a voice
        // crusher needs the control at all.
        let energy = |drive: f32| {
            let mut b = crusher(4.0, 48_000.0, 100.0);
            b.set_param("drive", drive);
            #[allow(clippy::cast_precision_loss)]
            let mut buf: Vec<f32> = (0..1000).map(|i| (i as f32 * 0.02).sin() * 0.03).collect();
            b.process(&mut buf, 48_000);
            buf.iter().map(|s| s.abs()).sum::<f32>()
        };
        assert!(
            energy(0.0) < 1e-6,
            "quiet input should fall in the dead zone"
        );
        assert!(energy(100.0) > 0.1, "drive should lift it back out");
    }

    #[test]
    fn stays_finite_on_extreme_input() {
        let mut b = crusher(4.0, 6000.0, 100.0);
        let mut buf = [f32::NAN, f32::INFINITY, -f32::INFINITY, 1e30, -1e30, 0.4];
        b.process(&mut buf, 48_000);
        for s in buf {
            assert!(s.is_finite(), "output must stay finite, got {s}");
        }
    }

    #[test]
    fn disabling_clears_the_held_sample() {
        let mut b = crusher(4.0, 4000.0, 100.0);
        let mut buf = vec![0.8_f32; 256];
        b.process(&mut buf, 48_000);
        b.set_enabled(false);
        b.set_enabled(true);
        // The first sample after a re-enable is freshly held, not stale.
        let mut next = vec![0.1_f32; 8];
        b.process(&mut next, 48_000);
        assert!(
            next[0].abs() < 0.5,
            "re-enable must not emit the previous hold, got {}",
            next[0]
        );
    }

    /// A rate change must not need a retune hook: the increment is re-derived
    /// from `sample_rate` every block.
    ///
    /// This MEASURES the adaptation rather than asserting `is_finite` — the
    /// output guard makes finiteness unconditional, so an is-finite assertion
    /// here would pass even for an implementation that ignored `sample_rate`
    /// entirely.
    #[test]
    fn a_device_rate_change_is_absorbed() {
        let holds_at = |sr: u32| {
            let mut b = crusher(16.0, 6000.0, 100.0);
            // Same SAMPLE COUNT at both rates, so a rate-ignoring
            // implementation would produce the same hold count; a correct one
            // produces half as many at double the rate.
            #[allow(clippy::cast_precision_loss)]
            let mut buf: Vec<f32> = (0..9600).map(|i| (i as f32 / 9600.0) * 2.0 - 1.0).collect();
            b.process(&mut buf, sr);
            holds(&buf)
        };
        let at_48 = holds_at(48_000);
        let at_96 = holds_at(96_000);
        #[allow(clippy::cast_precision_loss)]
        let ratio = at_48 as f32 / at_96 as f32;
        assert!(
            (ratio - 2.0).abs() < 0.1,
            "doubling the device rate should halve the holds per buffer              ({at_48} vs {at_96}, ratio {ratio})"
        );
    }
}
