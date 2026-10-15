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
fn full_at(frames: u64) -> String {
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
    /// ready to `write`. Desktop audio goes in before the timeline is asked
    /// what is ready, so a recording thread that was held up has nothing
    /// arrive "late" that was in fact waiting.
    pub fn pump(
        &mut self,
        start: Instant,
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
        let now = start.elapsed().as_secs_f64();
        for block in self.timeline.pump(now) {
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
        loop {
            self.pump(start, write)?;
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
    #![allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]

    use super::AudioTrack;
    use crate::stems::StemWriter;
    use crate::take::AudioFeed;
    use crate::timeline::{AudioBlock, AudioChunk, DesktopChunk, TrackOptions, BLOCK_FRAMES};
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
            .pump(start, &mut |b| {
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
            .pump(start, &mut |_| {
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
        t.pump(Instant::now(), &mut |_| Ok(())).unwrap();
        drop(t);
        assert_eq!(feed.polls, 1, "the track stopped with its stems");
    }
}
