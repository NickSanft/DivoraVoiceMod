//! The speaker's own rolling baseline.
//!
//! "Loud" only means anything next to how this person usually sounds on this
//! device in this room, so every descriptor is a comparison against a baseline
//! built from the session's own speaking windows.
//!
//! ### Why a median and not a mean
//!
//! A cough, a door, a laugh or a chair scrape is a 20–30 dB outlier. A mean
//! over a minute of speech takes minutes to shed one; the median ignores
//! anything short of half the observations, so a handful of outliers move it
//! by nothing at all. The same argument applies to F0 (one octave-error frame
//! that survives the guard) and to pace.
//!
//! ### Why it is a ring and not an average-so-far
//!
//! It has to be *rolling*. A speaker who leans in, changes headset position or
//! moves to a quieter room has genuinely changed baseline; an accumulate-
//! forever statistic would keep calling their new normal "loud" for the rest of
//! the session. [`RING`] observations is the memory.
//!
//! ### What invalidates it
//!
//! Nothing here persists — the ring lives in the analyzer, which the engine
//! owns and drops. [`Baseline::reset`] is called on an engine restart and on an
//! input-device change, because both mean the gain structure in front of the
//! mic may have changed and none of that is the speaker. A preset switch does
//! **not** reset it: the baseline is built from the dry tap, which the chain
//! never touches.

#![allow(clippy::cast_precision_loss)]

/// The quantities a descriptor axis compares against.
pub const AXES: usize = 6;

/// Observations retained. One is taken every [`super::analyzer::STRIDE_EMITS`] emits, so
/// this is ~90 s of *speaking* time — long enough to be a baseline, short
/// enough to follow a speaker who moves.
const RING: usize = 90;
/// Observations needed before the comparison is worth showing. Consecutive
/// observations overlap (a 2.5 s window sampled every 1 s), so 12 of them is
/// ~12 s of speech but only ~5 independent windows — about the least that can
/// carry a p90−p10 spread.
const MIN_OBS: usize = 12;

/// Fixed-size ring of speaking-window observations, summarised by median.
pub struct Baseline {
    ring: [[f32; AXES]; RING],
    pos: usize,
    len: usize,
    /// Sort scratch, so summarising allocates nothing after construction.
    scratch: Vec<f32>,
}

impl Baseline {
    pub fn new() -> Self {
        Self {
            ring: [[0.0; AXES]; RING],
            pos: 0,
            len: 0,
            scratch: Vec::with_capacity(RING),
        }
    }

    pub fn push(&mut self, obs: [f32; AXES]) {
        self.ring[self.pos] = obs;
        self.pos = (self.pos + 1) % RING;
        self.len = (self.len + 1).min(RING);
    }

    pub fn calibrated(&self) -> bool {
        self.len >= MIN_OBS
    }

    /// Per-axis median, or `None` until there is enough to mean anything.
    pub fn summary(&mut self) -> Option<[f32; AXES]> {
        if !self.calibrated() {
            return None;
        }
        let mut out = [0.0f32; AXES];
        for (axis, slot) in out.iter_mut().enumerate() {
            self.scratch.clear();
            self.scratch
                .extend(self.ring[..self.len].iter().map(|o| o[axis]));
            self.scratch
                .sort_unstable_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
            *slot = self.scratch[self.scratch.len() / 2];
        }
        Some(out)
    }

    pub fn reset(&mut self) {
        self.pos = 0;
        self.len = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_says_so_until_it_has_enough() {
        let mut b = Baseline::new();
        for i in 0..MIN_OBS - 1 {
            assert!(!b.calibrated(), "calibrated after {i} observations");
            assert!(b.summary().is_none());
            b.push([1.0; AXES]);
        }
        b.push([1.0; AXES]);
        assert!(b.calibrated());
        assert!(b.summary().is_some());
    }

    #[test]
    fn one_cough_does_not_move_it() {
        let mut b = Baseline::new();
        for _ in 0..30 {
            b.push([-30.0, 10.0, 120.0, 5.0, 4.0, 1_500.0]);
        }
        let before = b.summary().unwrap();
        // A door slam: 25 dB over, an octave up, wildly bright.
        b.push([-5.0, 40.0, 240.0, 30.0, 12.0, 9_000.0]);
        let after = b.summary().unwrap();
        for axis in 0..AXES {
            assert!(
                (after[axis] - before[axis]).abs() < f32::EPSILON,
                "axis {axis} moved from {} to {}",
                before[axis],
                after[axis]
            );
        }
    }

    #[test]
    fn it_follows_a_speaker_who_moves() {
        let mut b = Baseline::new();
        for _ in 0..RING {
            b.push([-30.0, 10.0, 120.0, 5.0, 4.0, 1_500.0]);
        }
        assert!((b.summary().unwrap()[0] - -30.0).abs() < 0.01);
        // They lean in: a full ring of the new normal.
        for _ in 0..RING {
            b.push([-20.0, 10.0, 120.0, 5.0, 4.0, 1_500.0]);
        }
        assert!((b.summary().unwrap()[0] - -20.0).abs() < 0.01);
    }

    #[test]
    fn a_reset_forgets_everything() {
        let mut b = Baseline::new();
        for _ in 0..RING {
            b.push([-30.0; AXES]);
        }
        b.reset();
        assert!(!b.calibrated());
        assert!(b.summary().is_none());
    }
}
