//! Constant-frame-rate pacing for a change-driven capture source.
//!
//! Windows Graphics Capture delivers a frame only when the screen changes:
//! a static desktop produces nothing for seconds, a game produces a frame
//! per present. An MP4 wants one frame per tick. The pacer owns the tick
//! clock: each tick due is written once (the latest captured image, repeated
//! if nothing changed), and ticks that fell behind while the encoder thread
//! was busy are skipped and counted, never written late — a late frame
//! would push every later timestamp, and the audio, out of step.
//!
//! Pure arithmetic over durations since the take started, so it is tested
//! without a capture session.

use std::time::Duration;

/// 100-nanosecond units, the Media Foundation time base.
pub const TICKS_PER_SECOND: u64 = 10_000_000;

/// Presentation time of frame `index` at `fps`, in 100-ns ticks — the same
/// for a take, a clip and the replay ring, so their times never disagree
/// (v1.54.0; what [`FramePacer::pts_of`] computes).
pub fn pts_at(index: u64, fps: u32) -> u64 {
    // index * 10^7 / fps without overflow for any realistic take
    // (u128 keeps it exact past 58 000 years at 240 fps).
    let t = u128::from(index) * u128::from(TICKS_PER_SECOND) / u128::from(fps.max(1));
    u64::try_from(t).unwrap_or(u64::MAX)
}

/// One frame to write.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Emit {
    /// The frame's index on the constant-rate timeline (0, 1, 2, …).
    pub index: u64,
    /// Presentation time, in 100-ns ticks from the start of the take.
    pub pts: u64,
    /// Duration until the next frame, in 100-ns ticks (never zero).
    pub duration: u64,
    /// Ticks skipped since the previous emit because the writer was late.
    pub dropped_before: u64,
}

/// Turns "time since the take started" into frames to write.
#[derive(Debug, Clone)]
pub struct FramePacer {
    fps: u32,
    next_index: u64,
    dropped: u64,
}

impl FramePacer {
    /// A pacer at `fps` frames per second (clamped to 1..=240).
    pub fn new(fps: u32) -> Self {
        Self {
            fps: fps.clamp(1, 240),
            next_index: 0,
            dropped: 0,
        }
    }

    pub fn fps(&self) -> u32 {
        self.fps
    }

    /// Presentation time of frame `index`, in 100-ns ticks. Computed from
    /// the index, never accumulated, so it cannot drift (30 fps is not a
    /// whole number of ticks: 333 333.3…).
    pub fn pts_of(&self, index: u64) -> u64 {
        pts_at(index, self.fps)
    }

    /// When the next frame is due, as time since the start of the take.
    pub fn next_deadline(&self) -> Duration {
        ticks_to_duration(self.pts_of(self.next_index))
    }

    /// The frame to write at `elapsed` (time since the start of the take),
    /// or `None` if the next one isn't due yet. When several are due because
    /// the caller was late, only the newest is returned; the rest are counted
    /// in `dropped_before` and in [`Self::dropped`].
    pub fn due(&mut self, elapsed: Duration) -> Option<Emit> {
        let now = duration_to_ticks(elapsed);
        if now < self.pts_of(self.next_index) {
            return None;
        }
        // Newest index whose time has come: floor(now * fps / 10^7).
        let newest = u128::from(now) * u128::from(self.fps) / u128::from(TICKS_PER_SECOND);
        let newest = u64::try_from(newest)
            .unwrap_or(u64::MAX)
            .max(self.next_index);
        let dropped_before = newest - self.next_index;
        self.dropped += dropped_before;
        let pts = self.pts_of(newest);
        let duration = (self.pts_of(newest + 1) - pts).max(1);
        self.next_index = newest + 1;
        Some(Emit {
            index: newest,
            pts,
            duration,
            dropped_before,
        })
    }

    /// The index the next frame will have (v1.54.0: a take started now
    /// begins there).
    pub fn next_index(&self) -> u64 {
        self.next_index
    }

    /// Frames skipped so far because the writer fell behind.
    pub fn dropped(&self) -> u64 {
        self.dropped
    }

    /// Frames emitted so far.
    pub fn emitted(&self) -> u64 {
        self.next_index - self.dropped
    }

    /// Where the video ends: the end of the last frame written, in 100-ns
    /// ticks. Skipped ticks are part of the timeline, so this is not
    /// `pts_of(emitted())` — that falls a frame short for every skip.
    pub fn end_pts(&self) -> u64 {
        self.pts_of(self.next_index)
    }
}

fn duration_to_ticks(d: Duration) -> u64 {
    let t = d.as_nanos() / 100;
    u64::try_from(t).unwrap_or(u64::MAX)
}

fn ticks_to_duration(t: u64) -> Duration {
    Duration::from_nanos(t.saturating_mul(100))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    #[test]
    fn nothing_is_due_before_the_first_deadline_except_frame_zero_at_zero() {
        let mut p = FramePacer::new(30);
        let e = p.due(Duration::ZERO).expect("frame 0 is due at 0");
        assert_eq!((e.index, e.pts, e.dropped_before), (0, 0, 0));
        assert_eq!(p.due(ms(10)), None);
        assert_eq!(p.next_deadline(), Duration::from_nanos(33_333_300));
    }

    #[test]
    fn on_time_writer_gets_every_frame_with_strictly_increasing_pts() {
        for fps in [24, 30, 60] {
            let mut p = FramePacer::new(fps);
            let mut last: Option<Emit> = None;
            for _ in 0..(fps * 10) {
                let at = p.next_deadline();
                let e = p.due(at).expect("due at its own deadline");
                assert_eq!(e.dropped_before, 0);
                if let Some(prev) = last {
                    assert_eq!(e.index, prev.index + 1);
                    assert!(e.pts > prev.pts);
                    assert_eq!(
                        prev.pts + prev.duration,
                        e.pts,
                        "durations tile the timeline"
                    );
                }
                last = Some(e);
            }
            assert_eq!(p.dropped(), 0);
            assert_eq!(p.emitted(), u64::from(fps) * 10);
        }
    }

    #[test]
    fn timeline_does_not_drift_over_an_hour() {
        // 30 fps is 333 333.33 ticks a frame; accumulating rounded durations
        // would drift. Frame 108 000 (one hour) must sit exactly at 3600 s.
        let p = FramePacer::new(30);
        assert_eq!(p.pts_of(108_000), 3600 * TICKS_PER_SECOND);
        let p = FramePacer::new(60);
        assert_eq!(p.pts_of(216_000), 3600 * TICKS_PER_SECOND);
    }

    #[test]
    fn a_late_writer_skips_ticks_and_counts_them() {
        let mut p = FramePacer::new(30);
        p.due(Duration::ZERO).unwrap();
        // The encoder stalled for 200 ms: frames 1..=6 are due; only 6 is
        // written, 1..=5 are dropped.
        let e = p.due(ms(200)).unwrap();
        assert_eq!(e.index, 6);
        assert_eq!(e.dropped_before, 5);
        assert_eq!(e.pts, p.pts_of(6));
        assert_eq!(p.dropped(), 5);
        assert_eq!(p.emitted(), 2);
        // And it carries on from there with nothing more dropped.
        let e = p.due(p.next_deadline()).unwrap();
        assert_eq!((e.index, e.dropped_before), (7, 0));
        // The video ends after frame 7, skips and all: the audio track is
        // cut here, and once ended a frame short for every skip.
        assert_eq!(p.end_pts(), p.pts_of(8));
        assert_ne!(p.end_pts(), p.pts_of(p.emitted()));
    }

    #[test]
    fn repeated_polls_between_deadlines_emit_nothing() {
        let mut p = FramePacer::new(60);
        p.due(Duration::ZERO).unwrap();
        for t in [1, 5, 10, 16] {
            assert_eq!(p.due(ms(t)), None, "at {t} ms");
        }
        assert!(p.due(ms(17)).is_some());
    }

    #[test]
    fn duration_is_never_zero_and_fps_is_clamped() {
        let p = FramePacer::new(0);
        assert_eq!(p.fps(), 1);
        let p = FramePacer::new(10_000);
        assert_eq!(p.fps(), 240);
        let mut p = FramePacer::new(240);
        let e = p.due(Duration::ZERO).unwrap();
        assert!(e.duration > 0);
    }
}
