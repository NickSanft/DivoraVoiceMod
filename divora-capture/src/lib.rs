//! Screen capture and MP4 encoding for Divora (v1.52.0).
//!
//! Windows Graphics Capture records a monitor; a Direct3D 11 video processor
//! converts each frame BGRA → NV12 on the GPU; a Media Foundation sink writer
//! encodes H.264 (and, with audio, AAC) into an MP4, using Windows' own
//! encoders — hardware ones where the machine has them.
//!
//! The pure parts — frame pacing, sizing — live outside `native` and are
//! tested on any machine. Everything that touches COM is in `native`, the
//! crate's only `unsafe` scope.

pub mod clock_fit;
pub mod nv12;
pub mod pacer;
pub mod sizing;
mod take;
pub mod timeline;

mod error;
pub use error::CaptureError;
pub use take::{
    part_path, seconds_since, AudioFeed, AudioStats, CaptureStatus, MonitorInfo, RecordOptions,
    RecordStats, TakeObserver,
};

// The Windows Graphics Capture, Direct3D 11 and Media Foundation calls are
// COM, and the `windows` crate exposes almost all of them as `unsafe fn`;
// there is no safe binding to write this against. The unsafe code is kept
// to this module tree, each block says why it holds, and no COM failure
// panics (release builds use `panic = "abort"`, so a panic here would take
// the whole app down mid-call): every call returns a `Result`.
#[cfg(windows)]
#[allow(unsafe_code)]
mod native;

#[cfg(windows)]
pub use native::{list_monitors, probe::capture_status, record::record_monitor};

/// The real writer, for `tests/encoder_smoke.rs` to drive without a screen.
/// Not part of the API: hidden, and nothing outside the tests should call it.
#[cfg(windows)]
#[doc(hidden)]
pub mod smoke {
    pub use crate::native::encoder::TakeWriter;
    pub use crate::native::mf::{api, Mf, MfSession};
    pub use crate::native::probe::software_encoders;
    pub use crate::native::record::ComScope;
}

#[cfg(not(windows))]
mod elsewhere;
#[cfg(not(windows))]
pub use elsewhere::{capture_status, list_monitors, record_monitor};
