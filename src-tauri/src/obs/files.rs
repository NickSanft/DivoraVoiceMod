//! v1.61.0: what the OBS link keeps on disk, in the `obs/` folder
//! (`obs-dev/` for `tauri dev`): the password, encrypted with DPAPI for the
//! current Windows user (D28), and which scene each break replaced (D24).
//! Each file is written by a temp file and a rename; blocking work runs only
//! through [`blocking`](super::blocking).

use zeroize::Zeroizing;

/// D28: the password's file.
pub const PASSWORD_FILE: &str = "password.bin";

/// D24: the slots' file.
pub const RETURNS_FILE: &str = "returns.json";

/// D28: DPAPI's optional entropy, so another program running as the same
/// user can't open the blob with a bare `CryptUnprotectData`. It is in the
/// binary, so it is no secret. Changing it orphans every saved password.
pub const ENTROPY: &[u8] = b"DivoraVoice OBS password v1";

/// D28: what encrypts the password: DPAPI in the app, an in-memory one in
/// tests.
pub trait Cipher: Send + Sync {
    fn protect(&self, plain: &[u8]) -> Result<Vec<u8>, String>;
    /// The plaintext, wiped when dropped.
    fn unprotect(&self, blob: &[u8]) -> Result<Zeroizing<Vec<u8>>, String>;
}

/// D29: a stored password that won't decrypt (a profile reset, another
/// user's file): the status reads `passwordUnreadable`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Unreadable;
