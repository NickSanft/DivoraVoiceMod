use thiserror::Error;

/// Why a capture or an encode could not happen.
#[derive(Debug, Error)]
pub enum CaptureError {
    /// This machine can't record the screen, and the message says what to
    /// do about it (shown to the user as-is).
    #[error("{0}")]
    Unavailable(String),
    /// v1.54.0: the directly driven encoder can't be used on this PC (it
    /// isn't asynchronous, ignores the keyframe interval, can't force a
    /// keyframe, …). Takes fall back to the sink writer; Replay says it
    /// can't use this PC's encoder. The text is for the log.
    #[error("can't use this video encoder: {0}")]
    Encoder(String),
    /// The monitor asked for doesn't exist (any more).
    #[error("that screen isn't connected")]
    NoSuchMonitor,
    /// A Windows call failed.
    #[cfg(windows)]
    #[error("{context}: {source}")]
    Windows {
        context: &'static str,
        #[source]
        source: windows::core::Error,
    },
    /// Writing the output file failed.
    #[error("{0}")]
    Io(#[from] std::io::Error),
}

#[cfg(windows)]
pub(crate) trait Context<T> {
    /// Attach what was being attempted to a Windows error.
    fn context(self, what: &'static str) -> Result<T, CaptureError>;
}

#[cfg(windows)]
impl<T> Context<T> for windows::core::Result<T> {
    fn context(self, what: &'static str) -> Result<T, CaptureError> {
        self.map_err(|source| CaptureError::Windows {
            context: what,
            source,
        })
    }
}
