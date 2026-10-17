//! A take's audio track, between its feed and the file (v1.52.0; plain Rust
//! since v1.53.0, so that CI drives it): what arrives goes into the
//! [`AudioTimeline`], what the timeline finishes goes to the writer, and —
//! when asked — each source goes to its own WAV file beside the take.

#![cfg_attr(
    not(test),
    deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)
)]

use std::path::Path;
use std::time::{Duration, Instant};

use crate::error::CaptureError;
use crate::stems::{StemStats, StemWriter};
use crate::take::{AudioFeed, AudioStats};
use crate::timeline::{AudioBlock, AudioChunk, AudioTimeline, DesktopChunk, TrackOptions};

/// After the last frame, how long to wait for the audio to catch up to it:
/// the slowest voice chain (≈ 256 ms) plus the drain, with room.
const FLUSH_TIMEOUT: Duration = Duration::from_millis(1_500);

/// What the card says when the separate files reach a WAV file's limit,
/// `frames` (48 kHz) into the take.
pub(crate) fn full_at(frames: u64) -> String {
    let secs = frames / 48_000;
    format!(
        "a WAV file can only hold so much, so they stop at {}:{:02}:{:02}; the video goes on",
        secs / 3600,
        secs / 60 % 60,
        secs % 60
    )
}

/// The audio half of a take.
pub struct AudioTrack<'a> {
    feed: &'a mut dyn AudioFeed,
    timeline: AudioTimeline,
    chunks: Vec<AudioChunk>,
    desktop: Vec<DesktopChunk>,
    blocks: u64,
    stems: Option<StemWriter>,
    stems_error: Option<String>,
    stem_stats: Option<StemStats>,
    flush_timeout: Duration,
}

impl<'a> AudioTrack<'a> {
    /// A track from `feed`, made as `opts` says; with stems, their files go
    /// beside `take`.
    pub fn new(feed: &'a mut dyn AudioFeed, opts: TrackOptions, take: &Path) -> Self {
        let (stems, stems_error) = if opts.stems {
            match StemWriter::create(take, opts.desktop_gain_db.is_some()) {
                Ok(w) => (Some(w), None),
                Err(e) => {
                    tracing::warn!(error = %e, "couldn't start the separate audio files");
                    (None, Some(e.to_string()))
                }
            }
        } else {
            (None, None)
        };
        Self {
            feed,
            timeline: AudioTimeline::with_options(opts),
            chunks: Vec::new(),
            desktop: Vec::new(),
            blocks: 0,
            stems,
            stems_error,
            stem_stats: None,
            flush_timeout: FLUSH_TIMEOUT,
        }
    }

    pub fn stats(&self) -> AudioStats {
        AudioStats {
            timeline: self.timeline.stats(),
            blocks: self.blocks,
            stems: self.stem_stats.clone(),
            stems_error: self.stems_error.clone(),
        }
    }

    /// Take in everything that has arrived, then hand every block that is
    /// ready by `now` (the recording's clock, as its loop read it) to
    /// `write`. Desktop audio goes in before the timeline is asked what is
    /// ready, so a recording thread that was held up has nothing arrive
    /// "late" that was in fact waiting.
    pub fn pump(
        &mut self,
        start: Instant,
        now: Duration,
        write: &mut dyn FnMut(&AudioBlock) -> Result<(), CaptureError>,
    ) -> Result<(), CaptureError> {
        self.feed.poll(start, &mut self.chunks);
        for c in self.chunks.drain(..) {
            self.timeline.push(&c);
        }
        self.feed.poll_desktop(start, &mut self.desktop);
        for c in self.desktop.drain(..) {
            self.timeline.push_desktop(&c);
        }
        for block in self.timeline.pump(now.as_secs_f64()) {
            write(&block)?;
            self.blocks += 1;
        }
        self.write_stems();
        Ok(())
    }

    /// Wait (briefly) for the audio to reach `end`, then write the rest of
    /// the track through it.
    pub fn finish(
        &mut self,
        start: Instant,
        end: f64,
        write: &mut dyn FnMut(&AudioBlock) -> Result<(), CaptureError>,
    ) -> Result<(), CaptureError> {
        let deadline = Instant::now() + self.flush_timeout;
        // Never past the end by more than the timeline's own wait: a sleep
        // during this wait must not become audio.
        let ceiling = Duration::from_secs_f64(end.max(0.0)) + Duration::from_secs(2);
        loop {
            self.pump(start, start.elapsed().min(ceiling), write)?;
            let now = start.elapsed().as_secs_f64();
            if self.timeline.ready_until(now).is_none_or(|t| t >= end) || Instant::now() >= deadline
            {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        for block in self.timeline.finish(end) {
            write(&block)?;
            self.blocks += 1;
        }
        self.write_stems();
        Ok(())
    }

    /// End the track at `end` across a jump in the clock: the PC slept, and
    /// `start.elapsed()` now counts the sleep. `t_jump` is the take's clock
    /// as it last read before the jump.
    ///
    /// [`finish`](Self::finish) would pump with the live clock first, and
    /// the timeline writes every frame older than `now − MAX_WAIT_SECONDS`
    /// as silence: the whole sleep, hours of it, in one call. Instead,
    /// nothing from after the jump goes in — whatever the feed holds now is
    /// thrown away: at most the last poll's worth from before it, and what
    /// came after — the timeline is pumped as of `t_jump`, and finished at
    /// `end` with whatever has not arrived written as silence. The live
    /// clock is never read again.
    pub fn finish_at(
        &mut self,
        start: Instant,
        end: f64,
        t_jump: Duration,
        write: &mut dyn FnMut(&AudioBlock) -> Result<(), CaptureError>,
    ) -> Result<(), CaptureError> {
        self.feed.poll(start, &mut self.chunks);
        self.chunks.clear();
        self.feed.poll_desktop(start, &mut self.desktop);
        self.desktop.clear();
        let mut blocks = self.timeline.pump(t_jump.as_secs_f64());
        blocks.extend(self.timeline.finish(end));
        for block in &blocks {
            write(block)?;
            self.blocks += 1;
        }
        self.write_stems();
        Ok(())
    }

    /// Finish the separate files and give them their names: called once the
    /// take's own file has its name, so the stems never outlive a take that
    /// failed.
    pub fn finish_stems(&mut self) {
        if let Some(w) = self.stems.take() {
            match w.finish() {
                Ok(stats) => self.stem_stats = Some(stats),
                Err(e) => {
                    tracing::warn!(error = %e, "couldn't finish the separate audio files");
                    self.stems_error = Some(e.to_string());
                }
            }
        }
    }

    /// Stems never fail the take: an error stops them, says why, and the
    /// recording goes on.
    fn write_stems(&mut self) {
        let blocks = self.timeline.take_stems();
        let Some(w) = self.stems.as_mut() else {
            return;
        };
        for b in &blocks {
            if let Err(e) = w.write(b) {
                tracing::warn!(error = %e, "the separate audio files stopped");
                self.stems_error = Some(e.to_string());
                self.stems = None;
                return;
            }
        }
        // Past a WAV file's limit the files stop and the video goes on:
        // said once, so a long take doesn't end with stems cut short and
        // nothing on the card.
        if let (Some(frames), None) = (w.stopped_at(), &self.stems_error) {
            tracing::warn!(
                frames,
                "the separate audio files reached a WAV file's limit"
            );
            self.stems_error = Some(full_at(frames));
        }
    }

    #[cfg(test)]
    fn with_stems(mut self, w: StemWriter) -> Self {
        self.stems = Some(w);
        self
    }

    #[cfg(test)]
    fn with_flush_timeout(mut self, d: Duration) -> Self {
        self.flush_timeout = d;
        self
    }
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss
    )]

    use super::AudioTrack;
    use crate::stems::StemWriter;
    use crate::take::AudioFeed;
    use crate::timeline::{AudioBlock, AudioChunk, DesktopChunk, TrackOptions, BLOCK_FRAMES};
    use std::collections::VecDeque;
    use std::path::Path;
    use std::time::{Duration, Instant};

    /// A feed whose engine and desktop audio are whatever the test queued,
    /// stamped on the take's clock as they are polled.
    #[derive(Default)]
    struct Fake {
        engine: Vec<AudioChunk>,
        desktop: Vec<DesktopChunk>,
        polls: u32,
    }

    impl AudioFeed for Fake {
        fn poll(&mut self, _start: Instant, out: &mut Vec<AudioChunk>) {
            self.polls += 1;
            out.append(&mut self.engine);
        }
        fn poll_desktop(&mut self, _start: Instant, out: &mut Vec<DesktopChunk>) {
            out.append(&mut self.desktop);
        }
    }

    fn engine_chunk(index: u64, at: f64) -> AudioChunk {
        AudioChunk {
            epoch: 1,
            at,
            index,
            rate: 48_000,
            out_frames: 480,
            out_rate: 48_000,
            voice_wait: 0.0,
            chain_latency: 0,
            voice: vec![0.0; 480],
            sb: vec![0.0; 480],
        }
    }

    /// 10 ms of stereo at a constant `level`.
    fn desktop_chunk(at: f64, level: f32) -> DesktopChunk {
        DesktopChunk {
            stream: 1,
            at,
            timed: true,
            discontinuity: false,
            resumed: false,
            rate: 48_000,
            frames: vec![level; 2 * 480],
        }
    }

    /// Desktop audio that sat in the queue while the recording thread was
    /// held up still lands in the track — it goes in before the timeline is
    /// asked what is ready — and the stems match the track block for block,
    /// finish's padding included.
    #[test]
    fn desktop_audio_reaches_the_track_and_the_stems() {
        let dir = std::env::temp_dir().join(format!("divora-track-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let take = dir.join("take.mp4");
        let mut feed = Fake::default();
        // Two seconds of both, all of it already 0.4 s old by the first poll.
        let start = Instant::now()
            .checked_sub(Duration::from_millis(2_400))
            .unwrap();
        for k in 0..200u64 {
            let at = k as f64 * 0.01;
            feed.engine.push(engine_chunk(k * 480, at));
            feed.desktop.push(desktop_chunk(at, 0.25));
        }
        let opts = TrackOptions {
            desktop_gain_db: Some(0.0),
            stems: true,
        };
        let mut written: Vec<AudioBlock> = Vec::new();
        let mut track =
            AudioTrack::new(&mut feed, opts, &take).with_flush_timeout(Duration::from_millis(50));
        track
            .pump(start, start.elapsed(), &mut |b| {
                written.push(b.clone());
                Ok(())
            })
            .unwrap();
        track
            .finish(start, 2.0, &mut |b| {
                written.push(b.clone());
                Ok(())
            })
            .unwrap();
        track.finish_stems();
        let stats = track.stats();
        assert_eq!(stats.timeline.desktop_late_frames, 0);
        assert_eq!(stats.blocks, written.len() as u64);
        assert!(stats.stems_error.is_none(), "{:?}", stats.stems_error);
        let stems = stats.stems.expect("stems");
        assert_eq!(stems.frames, (written.len() * BLOCK_FRAMES) as u64);
        // The desktop's 0.25 is in the middle of the take.
        let mid = &written[40].samples;
        assert!(
            mid.iter().all(|&s| (i32::from(s) - 8_191).abs() <= 1),
            "{:?}",
            &mid[..4]
        );
        let desk = hound::WavReader::open(dir.join("take.desktop.wav")).unwrap();
        assert_eq!(u64::from(desk.duration()), stems.frames);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// Stems that reach a WAV file's limit stop there and say so, while the
    /// track goes on.
    #[test]
    fn stems_that_fill_up_say_where_they_stopped() {
        let dir = std::env::temp_dir().join(format!("divora-full-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let take = dir.join("take.mp4");
        let mut feed = Fake::default();
        let start = Instant::now()
            .checked_sub(Duration::from_millis(2_400))
            .unwrap();
        for k in 0..200u64 {
            feed.engine.push(engine_chunk(k * 480, k as f64 * 0.01));
        }
        // Room for three blocks of mono float.
        let cap = 3 * BLOCK_FRAMES as u64 * 4;
        let w = StemWriter::with_cap(&take, false, cap).unwrap();
        let mut blocks = 0;
        let opts = TrackOptions {
            desktop_gain_db: None,
            stems: true,
        };
        // The track's own stems go aside; the small ones replace them.
        let mut track = AudioTrack::new(&mut feed, opts, &dir.join("aside.mp4"))
            .with_stems(w)
            .with_flush_timeout(Duration::from_millis(50));
        track
            .pump(start, start.elapsed(), &mut |_| {
                blocks += 1;
                Ok(())
            })
            .unwrap();
        let said = track.stats().stems_error.expect("the card is told");
        assert!(said.contains("0:00:00") && said.contains("video"), "{said}");
        assert!(blocks > 3, "the track went on: {blocks} blocks");
        track.finish_stems();
        assert_eq!(track.stats().stems.unwrap().frames, 3 * BLOCK_FRAMES as u64);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// The earliest instant at most `want` ago that this machine's clock can
    /// say: `Instant` counts from boot on Windows, and a CI machine may not
    /// have been up for hours.
    fn long_ago(want: Duration) -> Instant {
        let mut back = want;
        loop {
            if let Some(t) = Instant::now().checked_sub(back) {
                return t;
            }
            back /= 2;
            assert!(
                back >= Duration::from_secs(60),
                "the clock can't reach back"
            );
        }
    }

    /// A feed that hands over one queued batch of engine chunks per poll.
    struct Batches(VecDeque<Vec<AudioChunk>>);

    impl AudioFeed for Batches {
        fn poll(&mut self, _start: Instant, out: &mut Vec<AudioChunk>) {
            if let Some(mut batch) = self.0.pop_front() {
                out.append(&mut batch);
            }
        }
    }

    /// `n` chunks of the soundboard at `level`, from frame `first`, the
    /// first stamped `at`.
    fn level_chunks(first: u64, at: f64, n: u64, level: f32) -> Vec<AudioChunk> {
        (0..n)
            .map(|k| {
                let mut c = engine_chunk((first + k) * 480, at + k as f64 * 0.01);
                c.sb = vec![level; 480];
                c
            })
            .collect()
    }

    /// A take across a sleep: two seconds recorded, then the PC slept eight
    /// hours with the engine's stream open. The live clock now counts the
    /// sleep, and the feed holds chunks stamped after it. Finished as of the
    /// jump, the track is the two seconds and no more — never the sleep as
    /// silence — and the post-sleep audio is not in it.
    #[test]
    fn finish_at_ignores_the_live_clock() {
        let mut feed = Batches(VecDeque::from([
            level_chunks(0, 0.0, 200, 0.25),
            // The same epoch's stream, carrying on after the sleep.
            level_chunks(200, 8.0 * 3600.0, 50, 0.9),
        ]));
        // The take's start as the clock read it before the sleep, 2.4 s in,
        // and as it reads it after: `start` is one instant, but the clock
        // counts the sleep.
        let before = Instant::now()
            .checked_sub(Duration::from_millis(2_400))
            .unwrap();
        let after = long_ago(Duration::from_secs(8 * 3600));
        let mut written: Vec<AudioBlock> = Vec::new();
        let mut track = AudioTrack::new(&mut feed, TrackOptions::default(), Path::new("take.mp4"));
        track
            .pump(before, before.elapsed(), &mut |b| {
                written.push(b.clone());
                Ok(())
            })
            .unwrap();
        let end = 2.0;
        track
            .finish_at(after, end, Duration::from_millis(2_400), &mut |b| {
                written.push(b.clone());
                Ok(())
            })
            .unwrap();
        let most = (end * 48_000.0 / BLOCK_FRAMES as f64).ceil() as usize + 1;
        assert!(
            written.len() <= most,
            "{} blocks for a {end} s take",
            written.len()
        );
        assert_eq!(track.stats().blocks, written.len() as u64);
        let level = |b: &AudioBlock| i32::from(b.samples[0]);
        assert!((level(&written[40]) - 8_191).abs() <= 1, "the take's audio");
        assert!(written.iter().all(|b| level(b) < 9_000), "post-sleep audio");
    }

    /// A stop, then the PC sleeps while the track waits for its last audio:
    /// the clock counts the sleep, but the wait never pumps more than a
    /// moment past the take's end.
    #[test]
    fn finish_never_pumps_a_sleep() {
        let mut feed = Batches(VecDeque::from([level_chunks(0, 0.0, 200, 0.25)]));
        let start = long_ago(Duration::from_secs(600));
        let mut written: Vec<AudioBlock> = Vec::new();
        let mut track = AudioTrack::new(&mut feed, TrackOptions::default(), Path::new("take.mp4"))
            .with_flush_timeout(Duration::from_millis(50));
        let end = 2.0;
        track
            .finish(start, end, &mut |b| {
                written.push(b.clone());
                Ok(())
            })
            .unwrap();
        let most = ((end + 2.0) * 48_000.0 / BLOCK_FRAMES as f64).ceil() as usize + 1;
        assert!(
            written.len() <= most,
            "{} blocks for a {end} s take",
            written.len()
        );
    }

    /// A take with no stems writes none, and a stem file that can't be made
    /// is reported, not fatal: the track goes on.
    #[test]
    fn stems_are_optional_and_never_fatal() {
        let mut feed = Fake::default();
        let take = std::env::temp_dir()
            .join("divora-no-stems")
            .join("take.mp4");
        let t = AudioTrack::new(&mut feed, TrackOptions::default(), &take);
        assert!(t.stats().stems_error.is_none() && t.stats().stems.is_none());
        drop(t);
        let mut feed = Fake::default();
        feed.engine.push(engine_chunk(0, 0.0));
        let nowhere = std::env::temp_dir()
            .join(format!("divora-missing-{}", std::process::id()))
            .join("take.mp4");
        let opts = TrackOptions {
            desktop_gain_db: None,
            stems: true,
        };
        let mut t = AudioTrack::new(&mut feed, opts, &nowhere);
        assert!(t.stats().stems_error.is_some());
        let start = Instant::now();
        t.pump(start, start.elapsed(), &mut |_| Ok(())).unwrap();
        drop(t);
        assert_eq!(feed.polls, 1, "the track stopped with its stems");
    }
}
