//! Matching an encoder's outputs to the frames submitted, and noticing when it stops answering (v1.54.0).
//!
//! The direct path hands Windows' H.264 encoder one frame per pacer tick and
//! gets access units back some time later, on the encoder's schedule. A unit
//! carries only the presentation time it was given, so the pipeline keeps the
//! frames it submitted, in order, and looks each output up by that time
//! ([`SubmitQueue`]). That is how a unit gets its pacer index back — and the
//! index, never a timestamp difference, is what takes and the replay ring cut
//! by.
//!
//! The same bookkeeping answers two other questions the encoder never raises
//! by itself: may a frame be submitted now ([`Credits`], for an asynchronous
//! encoder that says when it wants input), and has it gone quiet
//! ([`EncoderLiveness`]: a wedged encoder sends no error, it just stops).
//!
//! Pure bookkeeping over indices, times and counts, so it is tested without
//! an encoder.

use std::collections::VecDeque;
use std::fmt;
use std::time::Duration;

/// A frame handed to the encoder and not yet seen coming out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Submitted {
    /// The frame's pacer index on the pipeline clock.
    pub index: u64,
    /// The presentation time the sample was stamped with, in 100-ns ticks:
    /// what the encoder copies onto its output.
    pub pts: i64,
    /// Whether a keyframe was asked for with this frame.
    pub forced: bool,
    /// When it was submitted, on the pipeline clock.
    pub submitted_at: Duration,
}

/// Which submitted frame an output belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputMatch {
    /// The oldest frame in flight: the usual case. It is no longer in flight.
    Front(Submitted),
    /// The frame matched last: a second piece of the same frame, from an
    /// output the encoder split across calls. It keeps that frame's index,
    /// so the ring and the take merge the two pieces into one access unit
    /// (an MP4 sample has one presentation time).
    SamePiece(Submitted),
    /// A later frame. The `dropped` frames before it will never come out —
    /// the encoder dropped them — and are no longer in flight. A gap in
    /// the indices is only a dropped frame, so the take and the ring go on.
    AfterDropped { frame: Submitted, dropped: u64 },
    /// No frame in flight, nor the one matched last, has this time: the
    /// encoder re-stamped its output, so no unit's index can be trusted.
    /// An encoder failure.
    Restamped,
}

impl OutputMatch {
    /// The pacer index of the frame the output belongs to, if it belongs to one.
    pub fn index(&self) -> Option<u64> {
        match self {
            Self::Front(f) | Self::SamePiece(f) | Self::AfterDropped { frame: f, .. } => {
                Some(f.index)
            }
            Self::Restamped => None,
        }
    }
}

/// The frames handed to the encoder, oldest first, until their output comes.
#[derive(Debug, Clone, Default)]
pub struct SubmitQueue {
    in_flight: VecDeque<Submitted>,
    /// The frame the last output was matched to. A second piece of it comes
    /// after it has left `in_flight`, so it is kept here.
    last_matched: Option<Submitted>,
    last_submitted: Option<u64>,
    encoder_dropped: u64,
}

impl SubmitQueue {
    pub fn new() -> Self {
        Self::default()
    }

    /// The encoder accepted frame `index`, stamped `pts`.
    pub fn push(&mut self, index: u64, pts: i64, forced: bool, submitted_at: Duration) {
        self.in_flight.push_back(Submitted {
            index,
            pts,
            forced,
            submitted_at,
        });
        self.last_submitted = Some(index);
    }

    /// Which frame an output stamped `pts` belongs to. Checked in this order:
    /// the frame matched last (a second piece), the oldest in flight, a later
    /// one (the ones before it dropped), and otherwise none (re-stamped).
    /// The second piece is checked first so it is never mistaken for
    /// re-stamping, which would make the merge unreachable.
    pub fn match_output(&mut self, pts: i64) -> OutputMatch {
        if let Some(last) = self.last_matched.filter(|f| f.pts == pts) {
            return OutputMatch::SamePiece(last);
        }
        let Some(at) = self.in_flight.iter().position(|f| f.pts == pts) else {
            return OutputMatch::Restamped;
        };
        self.in_flight.drain(..at);
        let dropped = u64::try_from(at).unwrap_or(u64::MAX);
        let Some(frame) = self.in_flight.pop_front() else {
            // `at` was a position in the queue, so this can't happen; an
            // unmatched output is the safe answer if it ever did.
            return OutputMatch::Restamped;
        };
        self.last_matched = Some(frame);
        if dropped == 0 {
            OutputMatch::Front(frame)
        } else {
            self.encoder_dropped += dropped;
            OutputMatch::AfterDropped { frame, dropped }
        }
    }

    /// Frames the encoder has dropped so far: submitted, and passed over by
    /// an output for a later one.
    pub fn encoder_dropped(&self) -> u64 {
        self.encoder_dropped
    }

    /// When the oldest frame still in flight was submitted: how long the
    /// encoder has owed an output (see [`EncoderLiveness`]).
    pub fn oldest_submitted_at(&self) -> Option<Duration> {
        self.in_flight.front().map(|f| f.submitted_at)
    }

    /// The index of the last frame submitted, whether or not it has come out
    /// yet. A take stopped now ends after it: a frame still waiting to be
    /// submitted is not part of the take.
    pub fn last_submitted_index(&self) -> Option<u64> {
        self.last_submitted
    }

    /// Frames in flight.
    pub fn len(&self) -> usize {
        self.in_flight.len()
    }

    pub fn is_empty(&self) -> bool {
        self.in_flight.is_empty()
    }
}

/// How many frames an asynchronous encoder has asked for and not yet been
/// given: one per `METransformNeedInput` event.
///
/// A synchronous encoder (Windows' own software one) sends no events and
/// takes input whenever it is given some, so for it the count is never
/// touched: it stays 0 and [`Credits::wants_input`] is always true. Taking one
/// from a count of 0 would underflow — a panic in debug builds and CI, and in
/// release a count of four billion that submits into an encoder that asked
/// for nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Credits {
    is_async: bool,
    count: u32,
}

impl Credits {
    /// Credits for an encoder that is asynchronous (`MF_TRANSFORM_ASYNC`) or not.
    pub fn new(is_async: bool) -> Self {
        Self { is_async, count: 0 }
    }

    pub fn is_async(&self) -> bool {
        self.is_async
    }

    /// Requests waiting: always 0 for a synchronous encoder.
    pub fn count(&self) -> u32 {
        self.count
    }

    /// Whether a frame may be submitted now. Submitting early would use up a
    /// request that is still queued and skew the count from then on.
    pub fn wants_input(&self) -> bool {
        !self.is_async || self.count > 0
    }

    /// The encoder asked for a frame.
    pub fn on_need_input(&mut self) {
        if self.is_async {
            self.count = self.count.saturating_add(1);
        }
    }

    /// The encoder took a frame.
    pub fn on_accepted(&mut self) {
        if self.is_async {
            self.count = self.count.saturating_sub(1);
        }
    }

    /// The encoder refused a frame (`MF_E_NOTACCEPTING`) despite a request.
    /// Its own count is the one that holds: Microsoft documents this answer
    /// for input sent without a pending request, so there is none.
    pub fn on_not_accepting(&mut self) {
        if self.is_async {
            self.count = 0;
        }
    }
}

/// An encoder that has gone quiet: one more way it can fail, and the only
/// one that raises no event or error by itself (an encoder wedged after a
/// resume, a hang without a driver reset, a lost request for input). Without
/// this check every tick would just count as a dropped frame, and the replay
/// ring would say "on" over a frozen picture.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LivenessFault {
    /// A submitted frame has had no output for longer than the stall limit.
    OutputStalled,
    /// A frame has waited for the encoder to ask for input for longer than
    /// the stall limit.
    InputStalled,
    /// More frames are in flight than the cap (two GOPs).
    QueueOverflow,
}

impl fmt::Display for LivenessFault {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::OutputStalled => "the video encoder stopped returning frames",
            Self::InputStalled => "the video encoder stopped asking for frames",
            Self::QueueOverflow => "the video encoder is holding more than two GOPs of frames",
        })
    }
}

/// Checks, every loop iteration, that the encoder is still answering. It
/// can't wait for output to judge by, as the keyframe watchdog and the ring's
/// own fault do: a quiet encoder makes none.
///
/// Not checked while there is no capture (the screen was lost, or a rebuild
/// is waiting for its first picture) or after a drain: then nothing is owed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EncoderLiveness {
    /// How long an answer may take. The hardware encoder measured (NVIDIA's)
    /// answers in about a frame; a second is far past that.
    pub stall: Duration,
}

impl Default for EncoderLiveness {
    fn default() -> Self {
        Self { stall: Self::STALL }
    }
}

impl EncoderLiveness {
    /// The stall limit in production.
    pub const STALL: Duration = Duration::from_secs(1);

    /// `oldest_unanswered`: when the oldest frame in flight was submitted
    /// ([`SubmitQueue::oldest_submitted_at`]). `pending_since`: when the
    /// frame waiting to be submitted first found no request for input — a
    /// newer frame that replaces it keeps that time, so a stream of fresh
    /// frames can't hide a stall. `queue_len` against `cap`: the frames in
    /// flight against two GOPs (`2 · fps`). Times are on the pipeline
    /// clock; waiting exactly `stall` is still in time.
    pub fn check(
        &self,
        now: Duration,
        oldest_unanswered: Option<Duration>,
        pending_since: Option<Duration>,
        queue_len: usize,
        cap: usize,
    ) -> Result<(), LivenessFault> {
        let waited = |since: Option<Duration>| since.map(|t| now.saturating_sub(t));
        if waited(oldest_unanswered).is_some_and(|w| w > self.stall) {
            return Err(LivenessFault::OutputStalled);
        }
        if waited(pending_since).is_some_and(|w| w > self.stall) {
            return Err(LivenessFault::InputStalled);
        }
        if queue_len > cap {
            return Err(LivenessFault::QueueOverflow);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    /// The pts the pipeline stamps frame `i` with at 30 fps (`pacer::pts_of`).
    fn pts(i: u64) -> i64 {
        i64::try_from(i * 10_000_000 / 30).unwrap()
    }

    fn queue_of(indices: std::ops::Range<u64>) -> SubmitQueue {
        let mut q = SubmitQueue::new();
        for i in indices {
            q.push(i, pts(i), i == 0, ms(i * 33));
        }
        q
    }

    #[test]
    fn outputs_find_their_frames() {
        let mut q = queue_of(0..6);
        assert_eq!(q.last_submitted_index(), Some(5));

        // The front: popped, its index.
        let m = q.match_output(pts(0));
        assert!(matches!(m, OutputMatch::Front(f) if f.index == 0 && f.forced));
        assert_eq!(q.len(), 5);

        // The pts of the frame just popped again: the same index, nothing
        // popped, and not re-stamping.
        assert_eq!(q.match_output(pts(0)).index(), Some(0));
        assert!(matches!(q.match_output(pts(0)), OutputMatch::SamePiece(_)));
        assert_eq!(q.len(), 5);

        // A later entry: frames 1 and 2 never came out. They are counted and
        // gone, and the output gets frame 3's index.
        let m = q.match_output(pts(3));
        assert_eq!(
            m,
            OutputMatch::AfterDropped {
                frame: Submitted {
                    index: 3,
                    pts: pts(3),
                    forced: false,
                    submitted_at: ms(99),
                },
                dropped: 2,
            }
        );
        assert_eq!(q.encoder_dropped(), 2);
        assert_eq!(q.len(), 2);
        assert_eq!(q.oldest_submitted_at(), Some(ms(4 * 33)));

        // A time no frame in flight has — not even a dropped one, which can't
        // come out after a later frame — is re-stamping, and changes nothing.
        for stray in [pts(1), pts(3) + 1, pts(9), -1] {
            assert_eq!(q.match_output(stray), OutputMatch::Restamped, "{stray}");
        }
        assert_eq!((q.len(), q.encoder_dropped()), (2, 2));

        // And matching carries on from where it was.
        assert_eq!(q.match_output(pts(4)).index(), Some(4));
        assert_eq!(q.match_output(pts(5)).index(), Some(5));
        assert!(q.is_empty());
        assert_eq!(q.oldest_submitted_at(), None);
        assert_eq!(q.last_submitted_index(), Some(5));
    }

    #[test]
    fn a_second_piece_of_a_frame_keeps_its_index() {
        // Frame 7's output comes in two pieces with one pts, the second after
        // frame 8 has been submitted.
        let mut q = queue_of(7..8);
        assert_eq!(q.match_output(pts(7)).index(), Some(7));
        q.push(8, pts(8), false, ms(300));
        let second = q.match_output(pts(7));
        assert!(
            matches!(second, OutputMatch::SamePiece(f) if f.index == 7),
            "{second:?}"
        );
        // Frame 8 is still the next one out, and nothing was dropped.
        assert_eq!(q.len(), 1);
        let next = q.match_output(pts(8));
        assert!(
            matches!(next, OutputMatch::Front(f) if f.index == 8),
            "{next:?}"
        );
        assert_eq!(q.encoder_dropped(), 0);
        // Now frame 8 is the one a second piece would belong to, not 7.
        assert_eq!(q.match_output(pts(7)), OutputMatch::Restamped);
        assert_eq!(q.match_output(pts(8)).index(), Some(8));
    }

    #[test]
    fn credits_are_async_only() {
        // Synchronous: nothing moves the count, and it always wants input.
        // Taking one from 0 here would panic in this (debug) build.
        let mut sync = Credits::new(false);
        for _ in 0..3 {
            sync.on_accepted();
            assert_eq!(sync.count(), 0);
            assert!(sync.wants_input());
        }
        sync.on_need_input();
        sync.on_not_accepting();
        assert_eq!(sync.count(), 0);
        assert!(sync.wants_input());

        // Asynchronous: one per request, one less per frame taken.
        let mut a = Credits::new(true);
        assert!(a.is_async());
        assert!(!a.wants_input(), "nothing asked for yet");
        a.on_need_input();
        a.on_need_input();
        a.on_need_input();
        assert_eq!(a.count(), 3);
        a.on_accepted();
        assert_eq!(a.count(), 2);
        assert!(a.wants_input());
        // Refused despite the count: the encoder's count holds, which is none.
        a.on_not_accepting();
        assert_eq!(a.count(), 0);
        assert!(!a.wants_input());
        // And it never goes below 0.
        a.on_accepted();
        assert_eq!(a.count(), 0);
        a.on_need_input();
        assert_eq!(a.count(), 1);
    }

    const CAP: usize = 60;

    #[test]
    fn a_frame_with_no_output_for_over_a_second_is_a_stall() {
        let live = EncoderLiveness::default();
        assert_eq!(live.stall, ms(1_000));
        let submitted = Some(ms(5_000));
        assert_eq!(live.check(ms(5_400), submitted, None, 1, CAP), Ok(()));
        assert_eq!(live.check(ms(6_000), submitted, None, 1, CAP), Ok(()));
        assert_eq!(
            live.check(ms(6_001), submitted, None, 1, CAP),
            Err(LivenessFault::OutputStalled)
        );
        // Nothing in flight is owed nothing, however long ago anything was.
        assert_eq!(live.check(ms(60_000), None, None, 0, CAP), Ok(()));
    }

    #[test]
    fn a_frame_waiting_for_input_for_over_a_second_is_a_stall() {
        let live = EncoderLiveness::default();
        let pending = Some(ms(2_000));
        assert_eq!(live.check(ms(3_000), None, pending, 0, CAP), Ok(()));
        assert_eq!(
            live.check(ms(3_001), None, pending, 0, CAP),
            Err(LivenessFault::InputStalled)
        );
    }

    #[test]
    fn more_than_two_gops_in_flight_is_an_overflow() {
        let live = EncoderLiveness::default();
        let recent = Some(ms(990));
        assert_eq!(live.check(ms(1_000), recent, None, CAP, CAP), Ok(()));
        assert_eq!(
            live.check(ms(1_000), recent, None, CAP + 1, CAP),
            Err(LivenessFault::QueueOverflow)
        );
    }

    #[test]
    fn an_encoder_that_answers_is_never_faulted() {
        // Ten minutes at 30 fps through the real queue, with two frames of
        // latency and an output split in two now and then: no fault, ever.
        let live = EncoderLiveness::default();
        let mut q = SubmitQueue::new();
        for i in 0..18_000u64 {
            let now = Duration::from_nanos(i * 1_000_000_000 / 30);
            q.push(i, pts(i), i % 60 == 0, now);
            if i >= 2 {
                assert_eq!(q.match_output(pts(i - 2)).index(), Some(i - 2));
                if i % 97 == 0 {
                    assert_eq!(q.match_output(pts(i - 2)).index(), Some(i - 2));
                }
            }
            let oldest = q.oldest_submitted_at();
            assert_eq!(live.check(now, oldest, None, q.len(), CAP), Ok(()), "{i}");
        }
        assert_eq!(q.encoder_dropped(), 0);
    }
}
