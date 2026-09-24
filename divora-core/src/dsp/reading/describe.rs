//! Metrics against the baseline, turned into at most [`MAX_DESCRIPTORS`] words
//! from the closed vocabulary.
//!
//! Every word here describes the **signal**. Nothing in this file may grow a
//! branch that names a feeling or addresses the speaker — see the module docs
//! on [`super::super::reading`] for why that line is the whole feature.
//!
//! ### Thresholds
//!
//! Each axis has an ON threshold in its own unit, chosen to sit above both the
//! estimator's own noise and the drift a speaker produces without meaning
//! anything by it:
//!
//! | axis | unit | ON | why that number |
//! |---|---|---|---|
//! | level | dB | 6 | a doubling of amplitude, and well over the ±2–3 dB a head moving in front of a mic produces |
//! | level movement | dB | 4 | the span itself runs ~10–20 dB on speech, so this is a ~25 % change; below it the p90−p10 estimate's own scatter dominates |
//! | pitch height | semitones | 2 | phrase-to-phrase median F0 drifts ~1 st with no change of register; 2 st is a step a listener would place |
//! | pitch range | semitones | 3 | conversational F0 spread runs ~4–8 st, so this is about half of it |
//! | pace | onsets/s | 1.5 | a 2.5 s window holds ~10 onsets, so the count's own scatter is already ~±1.3/s — anything smaller would be reporting noise |
//! | brightness | semitones | 5 | ~0.42 octave; far outside the ±12 Hz bin quantisation, and about what a real change of vocal effort moves a centroid |
//!
//! ### Hysteresis
//!
//! A threshold with no band makes the phrase flicker between "steady" and a
//! word every window whenever a speaker sits near it. Each axis therefore
//! switches on at its threshold and off only at [`HYSTERESIS`] of it, and an
//! axis already on screen gets a [`SHOWN_BONUS`] advantage when the top three
//! are chosen, so a newcomer has to be clearly more salient to displace it
//! rather than merely tie.

#![allow(clippy::cast_precision_loss)]

use super::super::{Descriptor, Metrics, MAX_DESCRIPTORS};
use super::baseline::AXES;

/// Fraction of the ON threshold at which an axis switches off again.
const HYSTERESIS: f32 = 0.6;
/// Salience advantage held by a descriptor that is already on screen.
const SHOWN_BONUS: f32 = 1.15;

/// ON thresholds, in each axis's own unit, in [`AXES`] order.
const THRESHOLDS: [f32; AXES] = [6.0, 4.0, 2.0, 3.0, 1.5, 5.0];
/// The word for a deviation below the baseline, per axis.
const BELOW: [Descriptor; AXES] = [
    Descriptor::Quiet,
    Descriptor::Flat,
    Descriptor::Low,
    Descriptor::Narrow,
    Descriptor::Slow,
    Descriptor::Dark,
];
/// The word for a deviation above it.
const ABOVE: [Descriptor; AXES] = [
    Descriptor::Loud,
    Descriptor::Dynamic,
    Descriptor::High,
    Descriptor::Wide,
    Descriptor::Fast,
    Descriptor::Bright,
];

/// Ratio between two positive quantities, in semitones. Used for the two axes
/// whose unit is multiplicative (pitch, spectral centroid); `None` when either
/// side is zero, which is how an unvoiced window declines to be compared.
fn semitones(value: f32, reference: f32) -> Option<f32> {
    if value > 0.0 && reference > 0.0 {
        Some(12.0 * (value / reference).log2())
    } else {
        None
    }
}

/// The six comparisons, in [`AXES`] order. `None` for an axis this window
/// cannot speak to.
pub fn deviations(m: &Metrics, base: &[f32; AXES]) -> [Option<f32>; AXES] {
    [
        m.energy_dbfs.is_finite().then(|| m.energy_dbfs - base[0]),
        Some(m.energy_range_db - base[1]),
        semitones(m.f0_hz, base[2]),
        (m.f0_hz > 0.0).then(|| m.f0_range_st - base[3]),
        Some(m.pace_ops - base[4]),
        semitones(m.brightness_hz, base[5]),
    ]
}

/// The observation this window contributes to the baseline, same axis order.
pub fn observation(m: &Metrics) -> [f32; AXES] {
    [
        m.energy_dbfs,
        m.energy_range_db,
        m.f0_hz,
        m.f0_range_st,
        m.pace_ops,
        m.brightness_hz,
    ]
}

/// Per-axis on/off state, carried between windows so the phrase is stable.
pub struct Describer {
    /// -1 below, 0 off, +1 above.
    active: [i8; AXES],
    /// Which axes were in the last phrase.
    shown: [bool; AXES],
}

impl Describer {
    pub fn new() -> Self {
        Self {
            active: [0; AXES],
            shown: [false; AXES],
        }
    }

    pub fn reset(&mut self) {
        self.active = [0; AXES];
        self.shown = [false; AXES];
    }

    /// Update the axes and write the phrase into `out`, most salient first.
    pub fn describe(&mut self, deviations: &[Option<f32>; AXES], out: &mut Vec<Descriptor>) {
        out.clear();
        // Salience is "how many thresholds out", so axes in different units
        // can be ranked against each other at all.
        let mut ranked: [(f32, usize); AXES] = [(0.0, 0); AXES];
        for (axis, slot) in ranked.iter_mut().enumerate() {
            let dev = deviations[axis].unwrap_or(0.0);
            if deviations[axis].is_none() {
                self.active[axis] = 0;
            } else {
                self.active[axis] = next_state(self.active[axis], dev, THRESHOLDS[axis]);
            }
            let salience = if self.active[axis] == 0 {
                0.0
            } else {
                let raw = dev.abs() / THRESHOLDS[axis];
                if self.shown[axis] {
                    raw * SHOWN_BONUS
                } else {
                    raw
                }
            };
            *slot = (salience, axis);
        }
        // Descending salience; ties break on axis order, so the phrase is a
        // pure function of the metrics.
        ranked.sort_by(|a, b| {
            b.0.partial_cmp(&a.0)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(a.1.cmp(&b.1))
        });
        self.shown = [false; AXES];
        for &(salience, axis) in &ranked {
            if out.len() == MAX_DESCRIPTORS || salience <= 0.0 {
                break;
            }
            out.push(if self.active[axis] < 0 {
                BELOW[axis]
            } else {
                ABOVE[axis]
            });
            self.shown[axis] = true;
        }
        if out.is_empty() {
            out.push(Descriptor::Steady);
        }
    }
}

/// One axis's hysteresis: on at the threshold, off at [`HYSTERESIS`] of it,
/// and a sign flip has to clear the full threshold rather than the band.
fn next_state(current: i8, deviation: f32, threshold: f32) -> i8 {
    let magnitude = deviation.abs();
    let sign = if deviation < 0.0 { -1 } else { 1 };
    if sign == current {
        // Already saying this: it takes a drop below the band to stop.
        if magnitude >= threshold * HYSTERESIS {
            current
        } else {
            0
        }
    } else if magnitude >= threshold {
        // Off, or pointing the other way: the full threshold, either way.
        sign
    } else {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NONE: [Option<f32>; AXES] = [None; AXES];

    fn dev(axis: usize, value: f32) -> [Option<f32>; AXES] {
        let mut d = [Some(0.0); AXES];
        d[axis] = Some(value);
        d
    }

    #[test]
    fn nothing_out_of_the_ordinary_is_steady() {
        let mut d = Describer::new();
        let mut out = Vec::new();
        d.describe(&[Some(0.0); AXES], &mut out);
        assert_eq!(out, vec![Descriptor::Steady]);
    }

    #[test]
    fn an_axis_with_nothing_to_say_says_nothing() {
        let mut d = Describer::new();
        let mut out = Vec::new();
        d.describe(&NONE, &mut out);
        assert_eq!(out, vec![Descriptor::Steady]);
    }

    #[test]
    fn each_axis_reaches_both_of_its_words() {
        for axis in 0..AXES {
            let mut d = Describer::new();
            let mut out = Vec::new();
            d.describe(&dev(axis, THRESHOLDS[axis] * 1.5), &mut out);
            assert_eq!(out, vec![ABOVE[axis]], "axis {axis} above");
            let mut d = Describer::new();
            d.describe(&dev(axis, -THRESHOLDS[axis] * 1.5), &mut out);
            assert_eq!(out, vec![BELOW[axis]], "axis {axis} below");
        }
    }

    #[test]
    fn at_most_three_words() {
        let mut d = Describer::new();
        let mut out = Vec::new();
        let all: [Option<f32>; AXES] =
            std::array::from_fn(|a| Some(THRESHOLDS[a] * (2.0 + a as f32)));
        d.describe(&all, &mut out);
        assert_eq!(out.len(), MAX_DESCRIPTORS);
        // Most salient first: axis 5 is furthest out.
        assert_eq!(out[0], ABOVE[5]);
        assert_eq!(out[1], ABOVE[4]);
        assert_eq!(out[2], ABOVE[3]);
    }

    #[test]
    fn a_word_survives_a_dip_back_toward_the_threshold() {
        let mut d = Describer::new();
        let mut out = Vec::new();
        d.describe(&dev(0, -THRESHOLDS[0] * 1.05), &mut out);
        assert_eq!(out, vec![Descriptor::Quiet]);
        // Just under the ON threshold but inside the hysteresis band: the
        // word must not blink out and back.
        for _ in 0..5 {
            d.describe(&dev(0, -THRESHOLDS[0] * 0.8), &mut out);
            assert_eq!(out, vec![Descriptor::Quiet], "the phrase flickered");
        }
        // Clearly back to normal: it goes.
        d.describe(&dev(0, -THRESHOLDS[0] * 0.4), &mut out);
        assert_eq!(out, vec![Descriptor::Steady]);
    }

    #[test]
    fn a_word_does_not_appear_from_inside_the_band() {
        let mut d = Describer::new();
        let mut out = Vec::new();
        for _ in 0..5 {
            d.describe(&dev(0, THRESHOLDS[0] * 0.8), &mut out);
            assert_eq!(out, vec![Descriptor::Steady]);
        }
    }

    #[test]
    fn a_shown_word_is_not_displaced_by_a_marginal_newcomer() {
        let mut d = Describer::new();
        let mut out = Vec::new();
        // Three axes on screen at 2.0 thresholds out.
        let mut base = [Some(0.0); AXES];
        for axis in 0..3 {
            base[axis] = Some(THRESHOLDS[axis] * 2.0);
        }
        d.describe(&base, &mut out);
        assert_eq!(out.len(), 3);
        // A fourth axis arrives 5 % more salient — not enough.
        let mut challenged = base;
        challenged[4] = Some(THRESHOLDS[4] * 2.1);
        d.describe(&challenged, &mut out);
        assert!(
            !out.contains(&ABOVE[4]),
            "a 5 % more salient newcomer displaced a shown word: {out:?}"
        );
        // Clearly more salient — it takes the slot.
        let mut clear = base;
        clear[4] = Some(THRESHOLDS[4] * 4.0);
        d.describe(&clear, &mut out);
        assert!(out.contains(&ABOVE[4]), "a clear newcomer was kept out");
    }

    #[test]
    fn a_reset_forgets_the_phrase() {
        let mut d = Describer::new();
        let mut out = Vec::new();
        d.describe(&dev(0, -THRESHOLDS[0] * 1.5), &mut out);
        assert_eq!(out, vec![Descriptor::Quiet]);
        d.reset();
        d.describe(&dev(0, -THRESHOLDS[0] * 0.8), &mut out);
        assert_eq!(out, vec![Descriptor::Steady]);
    }

    #[test]
    fn every_word_in_the_vocabulary_is_reachable() {
        // If a descriptor exists but nothing can produce it, the vocabulary
        // audit is guarding a word the UI never shows.
        let mut seen = std::collections::HashSet::new();
        for axis in 0..AXES {
            seen.insert(ABOVE[axis]);
            seen.insert(BELOW[axis]);
        }
        seen.insert(Descriptor::Steady);
        assert_eq!(
            seen.len(),
            Descriptor::COUNT,
            "the mapping reaches {} of {} descriptors",
            seen.len(),
            Descriptor::COUNT
        );
    }
}
