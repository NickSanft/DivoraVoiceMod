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

pub mod annexb;
pub mod clock_fit;
pub mod desktop;
pub mod desktop_thread;
pub mod keyframes;
#[doc(hidden)]
pub mod mp4_box;
pub mod nv12;
pub mod pacer;
pub mod pipeline;
pub mod pipeline_core;
pub mod replay;
pub mod sizing;
pub mod sleep;
pub mod stems;
pub mod submit;
mod take;
pub mod take_cut;
pub mod timeline;
pub mod track;

mod error;
pub use error::CaptureError;
pub use pipeline::{
    EngineSignal, HeldReason, LocalTime, PipelineCommand, PipelineEvents, PipelineHandle,
    PipelineSettings, RingStatus, SaveReply, SaveRequest, SavedClip, StartError, TakeEvents,
    TakeRequest,
};
pub use take::{
    part_path, seconds_since, slept_message, take_clock, AudioFeed, AudioStats, CaptureStatus,
    MonitorInfo, RecordOptions, RecordStats, TakeObserver, FORCE_CPU_ENV, RUNG1_ENV,
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
pub use native::{
    list_monitors,
    loopback::{list_playback_devices, process_loopback_available},
    pipeline::{record_monitor_direct, spawn_pipeline},
    probe::capture_status,
    record::record_monitor,
    system::{free_bytes, local_time, total_physical_memory, PRETEND_FREE_ENV},
};

/// The real writer, for `tests/encoder_smoke.rs` to drive without a screen.
/// Not part of the API: hidden, and nothing outside the tests should call it.
#[cfg(windows)]
#[doc(hidden)]
pub mod smoke {
    pub use crate::native::d3d::Gpu;
    pub use crate::native::encoder::TakeWriter;
    pub use crate::native::mf::{api, Mf, MfSession};
    pub use crate::native::mft::{
        EncodedUnit, EncoderConfig, EncoderEngine, EncoderEvent, EncoderInput, EncoderSession,
        Submit,
    };
    pub use crate::native::mux::{Mp4Writer, VideoFormat};
    pub use crate::native::pool::{InputPool, MfDevice};
    pub use crate::native::probe::software_encoders;
    pub use crate::native::record::ComScope;
    pub use crate::native::save::write_clip;
}

#[cfg(not(windows))]
mod elsewhere;
#[cfg(not(windows))]
pub use elsewhere::{
    capture_status, free_bytes, list_monitors, list_playback_devices, local_time,
    process_loopback_available, record_monitor, record_monitor_direct, spawn_pipeline,
    total_physical_memory, PRETEND_FREE_ENV,
};
