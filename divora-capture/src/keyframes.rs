//! When to force an IDR, and whether a forced one came back (v1.54.0).
//!
//! The replay ring keeps whole GOPs and cuts a clip at an IDR, and a take
//! opens on one, so the direct path needs an IDR at least every `2 · fps`
//! frames and one on demand. It asks the encoder for both: a GOP length set
//! before the output type, and a forced keyframe on the frames that must be
//! one. Not every encoder can be shown to honour either — a GOP length that
//! can't be read back is accepted unverified, and a forced keyframe that
//! fails is only counted — so [`KeyframeWatchdog`] watches what comes out:
//!
//! - **Proactively,** it asks for a keyframe once `gop_frames` frames have
//!   passed since the last IDR came out, whatever the encoder does by itself.
//! - **As a check,** a forced keyframe must come out within
//!   [`KeyframeWatchdog::LATE_FRAMES`] frames; otherwise the encoder doesn't
//!   make keyframes when asked, and the ring can't rely on it.
//!
//! Frames are counted in pacer indices, so ticks dropped before the encoder
//! count too: the ring's length bound is in time, not in frames encoded.

use std::collections::VecDeque;
use std::fmt;

/// A keyframe asked for at one frame had not come out a few frames later.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyframeFault {
    /// The frame the keyframe was asked for with.
    pub forced_at: u64,
    /// The output that showed it was late: a later frame, IDR or not.
    pub seen_at: u64,
}

impl fmt::Display for KeyframeFault {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "a keyframe asked for at frame {} had not come out by frame {}",
            self.forced_at, self.seen_at
        )
    }
}

/// Decides which input frames to force as keyframes, and checks that the
/// forced ones come out as IDRs.
#[derive(Debug, Clone)]
pub struct KeyframeWatchdog {
    gop_frames: u64,
    /// The index of the newest IDR that has come out.
    last_idr: Option<u64>,
    /// Forced frames whose IDR has not come out yet, oldest first.
    forced: VecDeque<u64>,
}

impl KeyframeWatchdog {
    /// How many frames after a forced one its IDR may come out and still be
    /// in time. A forced keyframe at frame `k` that comes out as frame `k + 3`
    /// is in time; one at `k + 4`, or any output past `k + 3` while it is
    /// still missing, is a [`KeyframeFault`]. The hardware encoder measured
    /// makes the forced frame itself the IDR.
    pub const LATE_FRAMES: u64 = 3;

    /// A watchdog for a GOP of `gop_frames` frames (`2 · fps`; at least 1).
    pub fn new(gop_frames: u32) -> Self {
        Self {
            gop_frames: u64::from(gop_frames.max(1)),
            last_idr: None,
            forced: VecDeque::new(),
        }
    }

    /// Whether frame `index` should be submitted as a forced keyframe: once
    /// `gop_frames` input frames have passed since the last IDR **came out**
    /// — counted from that IDR's own index, not from when its output arrived,
    /// so an encoder's latency doesn't stretch the GOP. Before any IDR has
    /// come out, the first frame is forced, so the encoder's first IDR is
    /// checked too. Never while a forced keyframe is still on its way: the
    /// one in flight is the IDR this would ask for.
    ///
    /// A query: the frame may be replaced before it is submitted, and only
    /// [`Self::on_submitted`] records what was sent.
    pub fn should_force(&self, index: u64) -> bool {
        if !self.forced.is_empty() {
            return false;
        }
        match self.last_idr {
            Some(idr) => index.saturating_sub(idr) >= self.gop_frames,
            None => true,
        }
    }

    /// Frame `index` was accepted by the encoder, forced or not.
    pub fn on_submitted(&mut self, index: u64, forced: bool) {
        if forced {
            self.forced.push_back(index);
        }
    }

    /// The output for frame `index` came out; `key` when it holds an IDR.
    /// An IDR settles every keyframe forced at or before its frame, whether
    /// it is the forced frame itself or one the encoder chose a little later
    /// — and, forced or not, it restarts the GOP count. An IDR at a frame
    /// before the forced one settles nothing.
    ///
    /// Errs once per late keyframe, at the first output that shows it late;
    /// an IDR that comes out late still counts as the newest IDR.
    pub fn on_output(&mut self, index: u64, key: bool) -> Result<(), KeyframeFault> {
        let mut fault = None;
        while let Some(&forced_at) = self.forced.front() {
            if index <= forced_at.saturating_add(Self::LATE_FRAMES) {
                break;
            }
            self.forced.pop_front();
            fault.get_or_insert(KeyframeFault {
                forced_at,
                seen_at: index,
            });
        }
        if key {
            self.last_idr = Some(self.last_idr.map_or(index, |idr| idr.max(index)));
            while self.forced.front().is_some_and(|&f| f <= index) {
                self.forced.pop_front();
            }
        }
        fault.map_or(Ok(()), Err)
    }

    /// The index of the newest IDR that has come out.
    pub fn last_idr(&self) -> Option<u64> {
        self.last_idr
    }

    /// The oldest forced frame whose IDR has not come out yet.
    pub fn awaiting(&self) -> Option<u64> {
        self.forced.front().copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const GOP: u32 = 60;

    /// Submits frames `from..to` the way the pipeline does — forced when the
    /// watchdog says so — with outputs `lag` frames behind, none of them IDRs
    /// unless the encoder makes the forced frame one (`honour`).
    fn run(w: &mut KeyframeWatchdog, from: u64, to: u64, lag: u64, honour: bool) -> Vec<u64> {
        let mut forced = Vec::new();
        for i in from..to {
            let force = w.should_force(i);
            w.on_submitted(i, force);
            if force {
                forced.push(i);
            }
            if let Some(out) = i.checked_sub(lag) {
                let key = honour && forced.contains(&out);
                w.on_output(out, key).unwrap();
            }
        }
        forced
    }

    #[test]
    fn forces_after_two_gops_of_input() {
        let mut w = KeyframeWatchdog::new(GOP);
        // Nothing has come out yet: the first frame is forced, and nothing
        // else while it is on its way.
        assert!(w.should_force(0));
        w.on_submitted(0, true);
        assert!(!w.should_force(1));

        // Outputs run three frames behind input. The IDR at 0 comes out as
        // frame 3 goes in; by frame 60 only 57 outputs have come out since.
        for i in 1..60 {
            assert!(!w.should_force(i), "frame {i}");
            w.on_submitted(i, false);
            if i >= 3 {
                w.on_output(i - 3, i == 3).unwrap();
            }
        }
        assert_eq!(w.last_idr(), Some(0));
        assert!(!w.should_force(59), "one frame early");
        assert!(w.should_force(60), "exactly gop_frames after the IDR at 0");

        // The encoder makes an IDR of its own at 100, unasked (a scene cut,
        // or its GOP restarting after a force): the count restarts there.
        for i in 60..=103 {
            w.on_submitted(i, i == 60);
            w.on_output(i - 3, i - 3 == 60 || i - 3 == 100).unwrap();
        }
        assert_eq!(w.last_idr(), Some(100));
        for i in 104..160 {
            assert!(
                !w.should_force(i),
                "frame {i}: the natural IDR at 100 counts"
            );
        }
        assert!(w.should_force(160));

        // The whole loop: an encoder that honours every force and makes no
        // IDR of its own is forced exactly every gop_frames frames.
        let mut w = KeyframeWatchdog::new(GOP);
        let forced = run(&mut w, 0, 600, 2, true);
        assert_eq!(forced, (0..600).step_by(60).collect::<Vec<_>>());
    }

    #[test]
    fn a_late_forced_idr_is_a_fault() {
        let mut w = KeyframeWatchdog::new(GOP);
        w.on_submitted(0, true);
        w.on_output(0, true).unwrap();

        // Forced at k = 10; the encoder makes the IDR at k + 3: in time.
        for i in 10..=13 {
            w.on_submitted(i, i == 10);
        }
        assert_eq!(w.awaiting(), Some(10));
        for i in 10..13 {
            assert_eq!(w.on_output(i, false), Ok(()), "frame {i}");
        }
        assert_eq!(w.on_output(13, true), Ok(()));
        assert_eq!(w.awaiting(), None, "the IDR at k + 3 settles the force");
        assert_eq!(w.last_idr(), Some(13));
        for i in 14..40 {
            w.on_submitted(i, false);
            assert_eq!(w.on_output(i, false), Ok(()), "frame {i}");
        }

        // Forced at k = 70; the IDR comes at k + 4: late, once.
        w.on_submitted(70, true);
        for i in 70..74 {
            assert_eq!(w.on_output(i, false), Ok(()), "frame {i}");
        }
        let late = KeyframeFault {
            forced_at: 70,
            seen_at: 74,
        };
        assert_eq!(w.on_output(74, true), Err(late));
        assert_eq!(w.last_idr(), Some(74), "a late IDR is still an IDR");
        assert_eq!(w.on_output(75, false), Ok(()), "reported once");

        // Forced at k = 130, with an IDR just before it (k − 1): that one
        // settles nothing, and k + 4 with no IDR is late without one coming.
        w.on_submitted(129, false);
        w.on_submitted(130, true);
        assert_eq!(w.on_output(129, true), Ok(()));
        assert_eq!(w.awaiting(), Some(130));
        for i in 130..=133 {
            assert_eq!(w.on_output(i, false), Ok(()), "frame {i}");
        }
        assert_eq!(
            w.on_output(134, false),
            Err(KeyframeFault {
                forced_at: 130,
                seen_at: 134
            })
        );
        assert_eq!(w.awaiting(), None);
    }
}
