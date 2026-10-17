//! What a take is asked to do and reports back — the crate's API types,
//! shared by the Windows implementation and the stubs elsewhere.

use std::path::{Path, PathBuf};
use std::time::Instant;

use crate::stems::StemStats;
use crate::timeline::{AudioChunk, DesktopChunk, TimelineStats, TrackOptions};

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
    /// What the audio track is made of (v1.53.0): desktop audio and its
    /// volume, and whether each source is also saved on its own. The
    /// default is v1.52.0's track.
    pub audio: TrackOptions,
}

impl Default for RecordOptions {
    fn default() -> Self {
        Self {
            fps: 30,
            max_size: crate::sizing::DEFAULT_MAX,
            bitrate: None,
            cursor: true,
            audio: TrackOptions::default(),
        }
    }
}

/// What happened to a take's audio track.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct AudioStats {
    pub timeline: TimelineStats,
    /// 1024-frame blocks written.
    pub blocks: u64,
    /// The separate WAV files, once finished.
    pub stems: Option<StemStats>,
    /// Why the separate WAV files stopped, or were never started.
    pub stems_error: Option<String>,
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
    /// v1.54.0: why the take ended before it was stopped — the PC slept,
    /// the screen went away, the encoder failed — said as the card says it.
    /// The file is kept.
    pub cut: Option<String>,
}

/// Where a take's audio comes from. The app implements it over the audio
/// engine's tap; this crate never sees the engine.
pub trait AudioFeed {
    /// Move every chunk that has arrived into `out`, oldest first, with
    /// [`AudioChunk::at`] measured from `start` — the instant the take's
    /// first frame is stamped 0 (see [`seconds_since`]).
    fn poll(&mut self, start: Instant, out: &mut Vec<AudioChunk>);

    /// The same for desktop audio, on the same clock. A feed without any
    /// has nothing to add.
    fn poll_desktop(&mut self, _start: Instant, _out: &mut Vec<DesktopChunk>) {}

    /// Whether the engine is running (v1.54.0): a ring holds its clip while
    /// it isn't. A feed that can't tell says it is.
    fn engine(&mut self) -> crate::pipeline::EngineSignal {
        crate::pipeline::EngineSignal::Running
    }
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

/// Set to anything to record takes through the sink writer (v1.52.0's path)
/// even where the encoder could be driven directly (v1.54.0): for checking
/// one against the other.
pub const RUNG1_ENV: &str = "DIVORA_CAPTURE_RUNG1";

/// Set to anything to convert on the CPU even where the GPU could: how the
/// fallback is checked on a machine that would never take it. Only the sink
/// writer's path has a CPU converter, so it also means that path.
pub const FORCE_CPU_ENV: &str = "DIVORA_CAPTURE_CPU_CONVERT";

/// `m:ss`, or `h:mm:ss` past an hour: a time into a take, as the card shows
/// it (v1.54.0).
#[must_use]
pub fn take_clock(seconds: f64) -> String {
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)] // a take's length
    let total = seconds.max(0.0).floor() as u64;
    let (h, m, s) = (total / 3600, (total % 3600) / 60, total % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

/// What the card says when the PC slept during a take (v1.54.0): the take
/// ends at its last frame before the sleep.
#[must_use]
pub fn slept_message(seconds: f64) -> String {
    format!(
        "The PC went to sleep, so the recording stopped at {}. What was recorded is saved.",
        take_clock(seconds)
    )
}

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
