//! Soundboard — folder-driven sample player that mixes polyphonic
//! voices into the audio engine's output.
//!
//! ### Pipeline
//!
//! ```text
//! mic   ──► input ring ──► mono mixdown ──► DSP chain ──┐
//!                                                       │
//! soundboard voices ──► SoundboardMixer.mix_into(…) ────┴──► output
//!                                                          (fan-out to
//!                                                           device channels)
//! ```
//!
//! Decoded clips live as `Arc<Vec<f32>>` so the UI side can cache them
//! across plays without copying. The mixer plays whatever the buffer's
//! native sample rate is; voices interpolate against the engine's
//! current rate so a 44.1 kHz clip on a 48 kHz engine plays at the
//! right pitch instead of the chipmunk it would be without correction.

mod decoder;
mod mixer;
mod scanner;

pub use decoder::{decode_clip, DecodedClip};
pub use mixer::{SoundboardCommand, SoundboardMixer, MAX_VOICES};
pub use scanner::{scan_folder, SoundboardTile};

use thiserror::Error;

#[derive(Debug, Error)]
pub enum SoundboardError {
    #[error("folder not found or unreadable: {0}")]
    FolderNotFound(String),

    #[error("file extension '{0}' is not a supported audio format")]
    UnsupportedFormat(String),

    #[error("decode failed for {path}: {message}")]
    Decode { path: String, message: String },

    #[error("audio thread is not responding")]
    ThreadGone,
}
