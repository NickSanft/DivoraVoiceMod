//! v1.61.0 (§10.4): DPAPI for the current user, as the OBS password uses it,
//! through the public `divora_capture::dpapi`.
//!
//! User-scope DPAPI needs the user's profile loaded. Outside CI a protect
//! that fails prints "skipped: …" and returns, as the encoder smoke test
//! does for a machine without Media Foundation. With `CI` set every failure
//! fails the test, so the first CI run says whether the runner can use it;
//! its log shows whether these ran.

#![cfg(windows)]

use divora_capture::dpapi::{protect, unprotect};

/// The app's entropy (`src-tauri`'s `obs::files::ENTROPY`).
const ENTROPY: &[u8] = b"DivoraVoice OBS password v1";

/// A password with a non-ASCII character, so a UTF-16 copy differs.
const PASSWORD: &str = "correct horse \u{2713} 7Qz";

fn say(line: &str) {
    use std::io::Write;
    let _ = writeln!(std::io::stderr(), "dpapi: {line}");
}

/// `PASSWORD` protected, or `None` (said) when DPAPI can't run here outside
/// CI.
fn protected() -> Option<Vec<u8>> {
    match protect(PASSWORD.as_bytes(), ENTROPY) {
        Ok(blob) => Some(blob),
        Err(e) if std::env::var_os("CI").is_none() => {
            say(&format!("skipped: DPAPI can't protect here ({e})"));
            None
        }
        Err(e) => panic!("DPAPI refused to protect on CI: {e}"),
    }
}

fn contains(hay: &[u8], needle: &[u8]) -> bool {
    hay.windows(needle.len()).any(|w| w == needle)
}

#[test]
fn round_trip() {
    let Some(blob) = protected() else { return };
    say(&format!("ran: a {}-byte blob", blob.len()));
    assert_eq!(
        unprotect(&blob, ENTROPY).as_deref(),
        Ok(PASSWORD.as_bytes())
    );
    // Each protect is salted: the same password makes another blob.
    let again = protect(PASSWORD.as_bytes(), ENTROPY).unwrap();
    assert_ne!(again, blob);
    assert_eq!(
        unprotect(&again, ENTROPY).as_deref(),
        Ok(PASSWORD.as_bytes())
    );
}

#[test]
fn wrong_entropy_fails() {
    let Some(blob) = protected() else { return };
    assert!(unprotect(&blob, b"DivoraVoice OBS password v2").is_err());
    // No entropy at all, as a bare `CryptUnprotectData` would pass.
    assert!(unprotect(&blob, b"").is_err());
    // Nor does a blob that isn't DPAPI's open.
    let mut broken = blob.clone();
    let last = broken.len() - 1;
    broken[last] ^= 0xff;
    assert!(unprotect(&broken, ENTROPY).is_err());
    assert!(unprotect(b"not a blob", ENTROPY).is_err());
}

#[test]
fn blob_is_not_the_plaintext() {
    let Some(blob) = protected() else { return };
    let utf8 = PASSWORD.as_bytes();
    let utf16: Vec<u8> = PASSWORD.encode_utf16().flat_map(u16::to_le_bytes).collect();
    assert!(!contains(&blob, utf8), "the blob holds the UTF-8 password");
    assert!(
        !contains(&blob, &utf16),
        "the blob holds the UTF-16 password"
    );
    assert!(!contains(&blob, b"correct horse"));
}
