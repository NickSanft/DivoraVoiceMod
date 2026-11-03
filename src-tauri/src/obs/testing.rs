//! v1.61.0: what the OBS tests share (test builds only): an in-memory
//! cipher that counts its decrypts, a TRACE log capture, the forms a
//! password could take in a log line (§10.3), and a fresh scratch folder.

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use zeroize::Zeroizing;

use super::files::Cipher;

/// What [`MemCipher`]'s blobs start with.
const MEM_BLOB: &[u8] = b"mem-blob-1:";

/// A cipher that needs no Windows profile: the bytes masked behind a
/// marker. It counts the decrypts asked of it (L3: none when OBS asks for
/// no password), and can be told to refuse every encrypt.
#[derive(Default)]
pub struct MemCipher {
    unprotects: AtomicUsize,
    refuse: bool,
}

impl MemCipher {
    /// One that refuses to encrypt (DPAPI refusing).
    #[must_use]
    pub fn refusing() -> Self {
        Self {
            refuse: true,
            ..Self::default()
        }
    }

    /// The decrypts asked of it so far.
    pub fn unprotects(&self) -> usize {
        self.unprotects.load(Ordering::SeqCst)
    }
}

impl Cipher for MemCipher {
    fn protect(&self, plain: &[u8]) -> Result<Vec<u8>, String> {
        if self.refuse {
            return Err("refused".into());
        }
        Ok(MEM_BLOB
            .iter()
            .copied()
            .chain(plain.iter().map(|b| b ^ 0xa5))
            .collect())
    }

    fn unprotect(&self, blob: &[u8]) -> Result<Zeroizing<Vec<u8>>, String> {
        self.unprotects.fetch_add(1, Ordering::SeqCst);
        let masked = blob.strip_prefix(MEM_BLOB).ok_or("not a blob")?;
        Ok(Zeroizing::new(masked.iter().map(|b| b ^ 0xa5).collect()))
    }
}

/// `f`'s log lines at every level, TRACE included, on this thread and on
/// the blocking pool through [`blocking`](super::blocking), formatted as the
/// app's log is, without times or colours.
pub fn traced<T>(f: impl FnOnce() -> T) -> (T, String) {
    struct Buf(Arc<Mutex<Vec<u8>>>);
    impl std::io::Write for Buf {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let buf: Arc<Mutex<Vec<u8>>> = Arc::default();
    let into = buf.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(move || Buf(into.clone()))
        .with_ansi(false)
        .without_time()
        .with_max_level(tracing::Level::TRACE)
        .finish();
    // tracing-core, with exactly one dispatcher registered, computes a
    // callsite's interest from the default of whichever thread hits it
    // first (`Rebuilder::JustOne`): a parallel test with none would cache
    // "never" for a line this capture is waiting for. A second live
    // dispatcher makes every callsite ask all of them, this one included.
    let _second = tracing::Dispatch::new(tracing_subscriber::registry());
    let out = tracing::subscriber::with_default(subscriber, f);
    let bytes = buf
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone();
    (out, String::from_utf8_lossy(&bytes).into_owned())
}

/// §10.3: every form `password` could take in a log line, an event or an
/// answer: itself, its UTF-8 and UTF-16LE bytes in standard base64, its
/// bytes as a Rust `Debug` array, and in lowercase hex.
#[must_use]
pub fn secret_forms(password: &str) -> Vec<String> {
    let utf8 = password.as_bytes();
    let utf16: Vec<u8> = password.encode_utf16().flat_map(u16::to_le_bytes).collect();
    let hex = utf8.iter().fold(String::new(), |mut hex, b| {
        use std::fmt::Write;
        let _ = write!(hex, "{b:02x}");
        hex
    });
    vec![
        password.to_string(),
        STANDARD.encode(utf8),
        STANDARD.encode(&utf16),
        format!("{utf8:?}"),
        hex,
    ]
}

/// Fails if `text` holds any of `needles`, saying which form (its index in
/// [`secret_forms`]) and where — never the text or the needle, so a failing
/// test prints no password either.
pub fn assert_none_in(text: &str, needles: &[String]) {
    for (form, needle) in needles.iter().enumerate() {
        let at = text.find(needle.as_str());
        assert!(
            at.is_none(),
            "form {form} of the secret found at byte {at:?} of {} bytes",
            text.len()
        );
    }
}

/// A fresh, empty folder under the temp folder, for one test.
#[must_use]
pub fn scratch(name: &str) -> PathBuf {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let n = NEXT.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("divora-obs-{}-{name}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}
