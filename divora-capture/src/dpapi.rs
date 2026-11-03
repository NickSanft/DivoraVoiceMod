//! v1.61.0: DPAPI for the current Windows user, which the OBS link's
//! password is stored with (`src-tauri`'s `obs::files`, D28): [`protect`]
//! and [`unprotect`]. The calls are in `native::dpapi`, which loads
//! crypt32.dll at run time, so the exe gains no DLL import; off Windows
//! both fail with [`DpapiError::Unavailable`].

/// Why a DPAPI call failed. Never holds any of the data.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DpapiError {
    /// crypt32.dll, or one of its two functions, couldn't be loaded (or this
    /// isn't Windows).
    Unavailable,
    /// More than a blob's 4 GiB.
    TooBig,
    /// Windows refused: its HRESULT (another user's blob, another entropy, a
    /// blob that isn't DPAPI's, no user profile loaded, …).
    Windows(i32),
}

impl std::fmt::Display for DpapiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unavailable => f.write_str("DPAPI isn't available"),
            Self::TooBig => f.write_str("too much data for DPAPI"),
            // The bits of the HRESULT, as Windows prints them.
            Self::Windows(code) => write!(f, "Windows error {:#010x}", code.cast_unsigned()),
        }
    }
}

impl std::error::Error for DpapiError {}

#[cfg(windows)]
pub use crate::native::dpapi::{protect, unprotect};

#[cfg(not(windows))]
pub use crate::elsewhere::dpapi::{protect, unprotect};
