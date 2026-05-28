//! Audio engine for Divora.
//!
//! `AudioEngine` owns a dedicated OS thread that hosts the cpal streams
//! (which are `!Send`) and receives commands through an `mpsc` channel.
//! Shared state (current levels, monitor flag, running flag) lives in an
//! `Arc<EngineState>` with atomic fields so the UI thread can read them
//! without locking.
//!
//! Phase 2 scope: passthrough from a chosen input device to a chosen
//! output device, mono-mixed internally, with monitor on/off (output
//! silenced when monitor is off). The DSP graph slots in between input
//! and output in Phase 3.

mod devices;
mod engine;
mod level;
mod state;

pub use devices::{list_input_devices, list_output_devices, DeviceInfo};
pub use engine::{AudioEngine, StreamInfo};
pub use level::LevelMeter;
pub use state::Levels;

use thiserror::Error;

/// Errors that can occur while configuring or running the audio engine.
#[derive(Debug, Error)]
pub enum AudioEngineError {
    #[error("no input devices available")]
    NoInputDevice,

    #[error("no output devices available")]
    NoOutputDevice,

    #[error("input device '{0}' not found")]
    InputDeviceNotFound(String),

    #[error("output device '{0}' not found")]
    OutputDeviceNotFound(String),

    #[error(
        "sample-rate mismatch: input {input} Hz, output {output} Hz. Phase 2 requires matching rates; resampling lands in a later phase."
    )]
    SampleRateMismatch { input: u32, output: u32 },

    #[error("unsupported sample format on {device}: {format}")]
    UnsupportedSampleFormat { device: String, format: String },

    #[error("failed to query default config: {0}")]
    DefaultConfig(String),

    #[error("failed to build audio stream: {0}")]
    StreamBuild(String),

    #[error("failed to start audio stream: {0}")]
    StreamPlay(String),

    #[error("audio thread is not responding")]
    ThreadGone,
}
