//! Telling a sleep from a slow loop (v1.54.0).
//!
//! `Instant` on Windows is the performance counter, and it keeps counting
//! while the PC sleeps (measured). After a resume the pipeline's clock has
//! jumped by the whole sleep, and everything timed by it would take the jump
//! at its word: the pacer would count hours of frames as dropped, and the
//! audio timeline would write hours of overdue silence in one call. So the
//! pipeline checks for a jump before anything reads its clock, and ends or
//! rebuilds around it instead.
//!
//! A long gap between two loop iterations is not enough to tell: a debugger
//! pause or a stalled thread makes one too, with no sleep in it. Two signals
//! are: Windows' resume notification, and [`slept`] — the performance
//! counter's step less the unbiased interrupt time's, which leaves sleep out
//! and so moves with the counter while the PC is awake, however slow the
//! loop. Both see every resume, in either order and up to seconds apart, so
//! [`Discontinuities`] makes them one event: a resume handled twice would
//! rebuild the encoder twice.
//!
//! Pure arithmetic over readings the caller takes, so it is tested without
//! sleeping a PC.

use std::time::Duration;

/// How long the PC slept between two readings: `biased`, how far `Instant`
/// moved, less `unbiased_100ns`, how far `QueryUnbiasedInterruptTime` moved
/// (in its 100-ns units). The two are read one after the other, and the
/// unbiased time only moves on the clock interrupt, so with no sleep it can
/// be a little ahead: that is no sleep, not a negative one.
pub fn slept(biased: Duration, unbiased_100ns: u64) -> Duration {
    biased.saturating_sub(Duration::from_nanos(unbiased_100ns.saturating_mul(100)))
}

/// Turns the two sleep signals into one event per resume.
#[derive(Debug, Clone)]
pub struct Discontinuities {
    resumes_seen: u64,
    last_fired: Option<Duration>,
    coalesced: u64,
}

impl Discontinuities {
    /// How much sleep [`slept`] must report before it counts. The unbiased
    /// time moves in clock-interrupt steps (about 15.6 ms), so a few of
    /// those either way are reading noise; real sleep is seconds at least.
    pub const THRESHOLD: Duration = Duration::from_millis(500);

    /// A second signal this soon after one that was handled is the same
    /// resume: Windows' notification can arrive seconds after the jump.
    pub const COALESCE: Duration = Duration::from_secs(5);

    /// Watching from now. `resumes_now` is Windows' resume count at this
    /// moment: resumes before it are not this pipeline's to handle.
    pub fn new(resumes_now: u64) -> Self {
        Self {
            resumes_seen: resumes_now,
            last_fired: None,
            coalesced: 0,
        }
    }

    /// One loop iteration's evidence: Windows' resume count, and how long
    /// [`slept`] says the PC slept since the last iteration. True when a
    /// discontinuity is to be handled now; false when there is none, and for
    /// a signal within [`Self::COALESCE`] of one already handled.
    ///
    /// `now` is on a clock that counts sleep and that a rebuild does not
    /// restart — the encoder thread's own `Instant`, not the pipeline's
    /// clock, which starts again after one.
    pub fn poll(&mut self, now: Duration, resumes: u64, slept: Duration) -> bool {
        let woke = resumes != self.resumes_seen;
        // Seen whether or not it fires: a resume coalesced away must not
        // fire again on the next iteration.
        self.resumes_seen = resumes;
        if !woke && slept <= Self::THRESHOLD {
            return false;
        }
        if self
            .last_fired
            .is_some_and(|t| now.saturating_sub(t) < Self::COALESCE)
        {
            self.coalesced += 1;
            return false;
        }
        self.last_fired = Some(now);
        true
    }

    /// Signals folded into one already handled.
    pub fn coalesced(&self) -> u64 {
        self.coalesced
    }
}

/// Where a loop's clock was before the PC slept. Each iteration reads its
/// clock once, *then* checks for a sleep, and uses that one reading for
/// all its work; when a check finds a sleep, the moment to end things at is
/// the reading the previous check was handed — taken before the sleep,
/// wherever in that iteration (its work or its wait) the PC went down. A
/// reading at the end of an iteration would not do: the wait is where a
/// loop spends its time, so it is where the PC most often goes to sleep,
/// and a reading after the wait is after the sleep.
#[derive(Debug, Clone)]
pub struct LoopSleep {
    events: Discontinuities,
    last: Duration,
}

impl LoopSleep {
    pub fn new(resumes_now: u64) -> Self {
        Self {
            events: Discontinuities::new(resumes_now),
            last: Duration::ZERO,
        }
    }

    /// One iteration's check. `clock` is its reading of the loop's clock,
    /// taken before this call; `origin`, `resumes` and `slept` are as for
    /// [`Discontinuities::poll`]. `Some(before)` when the PC slept since the
    /// last check: `before` is the clock that check was handed.
    pub fn check(
        &mut self,
        clock: Duration,
        origin: Duration,
        resumes: u64,
        slept: Duration,
    ) -> Option<Duration> {
        let before = std::mem::replace(&mut self.last, clock);
        self.events.poll(origin, resumes, slept).then_some(before)
    }

    /// The loop's clock starts again at `clock` (a rebuilt session's first
    /// picture): a later sleep's moment is on the new clock.
    pub fn restart(&mut self, clock: Duration) {
        self.last = clock;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A loop iterating every 20 ms sleeps for eight hours in its wait. The
    /// moment it gets back is the reading before the sleep, never one
    /// taken after the wait — the eight hours must not reach the timeline.
    #[test]
    fn the_moment_before_a_sleep_is_read_before_it() {
        let mut w = LoopSleep::new(0);
        for i in 0..=5u64 {
            assert_eq!(w.check(ms(20 * i), ms(20 * i), 0, ms(20)), None);
        }
        // Down in the wait after the reading at 100 ms; back 8 h later.
        let back = ms(120) + s(8 * 3600);
        assert_eq!(w.check(back, back, 0, s(8 * 3600)), Some(ms(100)));
        // Windows' notification a moment later is the same sleep.
        assert_eq!(w.check(back + ms(20), back + ms(20), 1, ms(20)), None);
        // Down in the work of an iteration, after its reading: the same.
        assert_eq!(w.check(back + ms(40), back + ms(40), 1, ms(20)), None);
        let again = back + ms(60) + s(3600);
        assert_eq!(
            w.check(again, again, 1, s(3600)),
            Some(back + ms(40)),
            "the reading of the iteration it went down in"
        );
    }

    /// After a rebuilt session the loop's clock starts again; a sleep then
    /// ends things on the new clock, not the old one.
    #[test]
    fn a_restarted_clock_is_the_one_a_sleep_ends_on() {
        let mut w = LoopSleep::new(0);
        assert_eq!(w.check(s(90), s(90), 0, ms(20)), None);
        w.restart(Duration::ZERO);
        assert_eq!(
            w.check(s(100) + s(600), s(200) + s(600), 0, s(600)),
            Some(Duration::ZERO)
        );
    }

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    fn s(n: u64) -> Duration {
        Duration::from_secs(n)
    }

    #[test]
    fn slept_is_biased_minus_unbiased() {
        // Awake, one ordinary iteration: both clocks moved 20 ms.
        assert_eq!(slept(ms(20), 200_000), Duration::ZERO);
        // Awake, but the loop stalled for 3 s (a debugger, a stuck call):
        // both clocks moved 3 s. A slow loop is not a sleep.
        assert_eq!(slept(s(3), 30_000_000), Duration::ZERO);
        // Eight hours asleep inside a 20 ms iteration.
        assert_eq!(slept(s(8 * 3600) + ms(20), 200_000), s(8 * 3600));
        // The unit is 100 ns: 2 500 000 of them is 250 ms.
        assert_eq!(slept(s(1), 2_500_000), ms(750));
        // The unbiased reading a clock tick ahead: no sleep, and no panic.
        assert_eq!(slept(ms(15), 156_250), Duration::ZERO);
        assert_eq!(slept(Duration::ZERO, u64::MAX), Duration::ZERO);
    }

    #[test]
    fn two_triggers_within_five_seconds_are_one() {
        // Four resumes happened before this pipeline: none of its business.
        let mut d = Discontinuities::new(4);
        assert!(!d.poll(s(10), 4, ms(20)));
        assert!(!d.poll(s(11), 4, ms(500)), "at the threshold is not yet");
        assert!(d.poll(s(12), 4, ms(501)));

        // A sleep: the detector sees the jump first, Windows' notification
        // lands 2 s later. One event.
        assert!(d.poll(s(100), 4, s(3600)));
        assert!(!d.poll(s(102), 5, ms(20)));
        // The count it bumped does not fire later either.
        assert!(!d.poll(s(108), 5, ms(20)));
        assert!(!d.poll(s(200), 5, ms(20)));

        // The other order: the notification first, the detector a second
        // later.
        assert!(d.poll(s(500), 6, ms(20)));
        assert!(!d.poll(s(501), 6, s(60)));
        assert_eq!(d.coalesced(), 2);

        // Five seconds on, a signal is a new event again.
        assert!(!d.poll(s(504), 6, ms(20)));
        assert!(d.poll(s(505), 7, ms(20)));
        assert_eq!(d.coalesced(), 2);
    }
}
