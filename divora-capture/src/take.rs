//! What a take is asked to do and reports back — the crate's API types,
//! shared by the Windows implementation and the stubs elsewhere.

use std::path::{Path, PathBuf};
use std::time::Instant;

use crate::timeline::{AudioChunk, TimelineStats};

/// A monitor that can be recorded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MonitorInfo {
    /// Stable for the session and across restarts unless the display
    /// configuration changes (`\\.\DISPLAY1`).
    pub id: String,
    /// The desktop rectangle, in the calling process's coordinates.
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub primary: bool,
}

/// Whether this machine can record the screen, and if not, why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaptureStatus {
    pub supported: bool,
    /// What to tell the user when `supported` is false.
    pub reason: Option<String>,
    /// Friendly name of the H.264 encoder Media Foundation will try first.
    pub h264_encoder: Option<String>,
    /// Whether that encoder is a hardware one.
    pub hardware: bool,
}

/// What to record and how.
#[derive(Debug, Clone)]
pub struct RecordOptions {
    /// Frames per second (30 by default).
    pub fps: u32,
    /// Largest output size; the monitor is scaled down to fit.
    pub max_size: (u32, u32),
    /// Average bitrate in bits per second; `None` picks one from the size.
    pub bitrate: Option<u32>,
    /// Draw the mouse pointer into the recording.
    pub cursor: bool,
}

impl Default for RecordOptions {
    fn default() -> Self {
        Self {
            fps: 30,
            max_size: crate::sizing::DEFAULT_MAX,
            bitrate: None,
            cursor: true,
        }
    }
}

/// What happened to a take's audio track.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct AudioStats {
    pub timeline: TimelineStats,
    /// 1024-frame blocks written.
    pub blocks: u64,
}

/// What happened during a take.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RecordStats {
    /// Frames Windows Graphics Capture delivered (one per screen change).
    pub frames_arrived: u64,
    /// Frames written to the file.
    pub frames_written: u64,
    /// Frames written while nothing on screen had changed.
    pub frames_repeated: u64,
    /// Constant-rate ticks skipped because the writer fell behind.
    pub frames_dropped: u64,
    /// The encoded size.
    pub width: u32,
    pub height: u32,
    /// The encoder Media Foundation actually loaded, as it names itself.
    pub encoder: Option<String>,
    /// Whether that encoder is a hardware one.
    pub hardware: bool,
    /// The audio track, when the take had one.
    pub audio: Option<AudioStats>,
}

/// Where a take's audio comes from. The app implements it over the audio
/// engine's tap; this crate never sees the engine.
pub trait AudioFeed {
    /// Move every chunk that has arrived into `out`, oldest first, with
    /// [`AudioChunk::at`] measured from `start` — the instant the take's
    /// first frame is stamped 0 (see [`seconds_since`]).
    fn poll(&mut self, start: Instant, out: &mut Vec<AudioChunk>);
}

/// Told how a take is going, from the recording thread.
pub trait TakeObserver {
    /// The first frame is in and the file is open: the take has begun.
    fn started(&mut self) {}
    /// About once a second while recording, with the take so far.
    fn progress(&mut self, _stats: &RecordStats) {}
}

/// An observer that wants to know nothing.
impl TakeObserver for () {}

/// Seconds from `start` to `at` on the same clock, negative when `at` is
/// earlier.
#[must_use]
pub fn seconds_since(start: Instant, at: Instant) -> f64 {
    match at.checked_duration_since(start) {
        Some(d) => d.as_secs_f64(),
        None => -start.duration_since(at).as_secs_f64(),
    }
}

/// The file actually written while recording: `<path>.part`, renamed to
/// `path` only once the MP4 has been finalized, so a crash mid-take leaves
/// an obviously incomplete file rather than a broken `.mp4`.
#[must_use]
pub fn part_path(path: &Path) -> PathBuf {
    let mut s = path.as_os_str().to_owned();
    s.push(".part");
    PathBuf::from(s)
}

#[cfg(test)]
mod tests {
    use super::{part_path, seconds_since};
    use std::path::Path;
    use std::time::{Duration, Instant};

    #[test]
    fn seconds_since_is_signed() {
        let start = Instant::now();
        let later = start + Duration::from_millis(1_500);
        assert!((seconds_since(start, later) - 1.5).abs() < 1e-9);
        assert!((seconds_since(later, start) + 1.5).abs() < 1e-9);
    }

    #[test]
    fn the_part_file_sits_beside_the_take() {
        assert_eq!(
            part_path(Path::new(r"C:\rec\divora-screen.mp4")),
            Path::new(r"C:\rec\divora-screen.mp4.part")
        );
    }
}
