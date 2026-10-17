//! Off Windows there is nothing to capture with: the same API, reporting
//! that plainly, so the app above compiles and degrades the same way it does
//! on a Windows machine without Media Foundation.

use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use crate::desktop::PlaybackDevice;
use crate::pipeline::{
    LocalTime, PipelineCommand, PipelineEvents, PipelineHandle, PipelineSettings, StartError,
};
use crate::take::{
    AudioFeed, CaptureStatus, MonitorInfo, RecordOptions, RecordStats, TakeObserver,
};
use crate::CaptureError;
use std::sync::Arc;

/// See the Windows build's.
pub const PRETEND_FREE_ENV: &str = "DIVORA_PRETEND_FREE_BYTES";

const WINDOWS_ONLY: &str = "Screen recording is only available on Windows.";

#[must_use]
pub fn capture_status() -> CaptureStatus {
    CaptureStatus {
        supported: false,
        reason: Some(WINDOWS_ONLY.to_string()),
        h264_encoder: None,
        hardware: false,
    }
}

#[must_use]
pub fn list_monitors() -> Vec<MonitorInfo> {
    Vec::new()
}

#[must_use]
pub fn process_loopback_available() -> bool {
    false
}

pub fn list_playback_devices() -> Result<Vec<PlaybackDevice>, CaptureError> {
    Ok(Vec::new())
}

/// Always fails: there is no capture API here.
pub fn record_monitor_direct(
    monitor_id: &str,
    path: &Path,
    opts: &RecordOptions,
    stop: &AtomicBool,
    max_duration: Option<Duration>,
    audio: Option<&mut dyn AudioFeed>,
    observer: &mut dyn TakeObserver,
) -> Result<RecordStats, CaptureError> {
    record_monitor(monitor_id, path, opts, stop, max_duration, audio, observer)
}

/// Always fails: there is no capture API here.
pub fn record_monitor(
    _monitor_id: &str,
    _path: &Path,
    _opts: &RecordOptions,
    _stop: &AtomicBool,
    _max_duration: Option<Duration>,
    _audio: Option<&mut dyn AudioFeed>,
    _observer: &mut dyn TakeObserver,
) -> Result<RecordStats, CaptureError> {
    Err(CaptureError::Unavailable(WINDOWS_ONLY.to_string()))
}

/// Always fails: there is no capture API here.
pub fn spawn_pipeline(
    _settings: PipelineSettings,
    _feed: Box<dyn AudioFeed + Send>,
    _events: Arc<dyn PipelineEvents>,
    _first: PipelineCommand,
) -> Result<PipelineHandle, StartError> {
    Err(StartError::Other(CaptureError::Unavailable(
        WINDOWS_ONLY.to_string(),
    )))
}

/// Unknown here.
#[must_use]
pub fn total_physical_memory() -> u64 {
    0
}

/// Unknown here.
#[must_use]
pub fn free_bytes(_dir: &Path) -> Option<u64> {
    None
}

/// Unknown here: the epoch.
#[must_use]
pub fn local_time() -> LocalTime {
    LocalTime {
        year: 1970,
        month: 1,
        day: 1,
        ..LocalTime::default()
    }
}
