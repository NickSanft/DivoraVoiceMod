//! Off Windows there is nothing to capture with: the same API, reporting
//! that plainly, so the app above compiles and degrades the same way it does
//! on a Windows machine without Media Foundation.

use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use crate::take::{
    AudioFeed, CaptureStatus, MonitorInfo, RecordOptions, RecordStats, TakeObserver,
};
use crate::CaptureError;

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
