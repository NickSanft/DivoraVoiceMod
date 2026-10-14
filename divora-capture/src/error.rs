use thiserror::Error;

/// Why a capture or an encode could not happen.
#[derive(Debug, Error)]
pub enum CaptureError {
    /// This machine can't record the screen, and the message says what to
    /// do about it (shown to the user as-is).
    #[error("{0}")]
    Unavailable(String),
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
