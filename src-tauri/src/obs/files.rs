//! v1.61.0: what the OBS link keeps on disk, in the `obs/` folder
//! (`obs-dev/` for `tauri dev`): the password, encrypted with DPAPI for the
//! current Windows user (D28), and which scene each break replaced (D24).
//! Each file is written by a temp file and a rename; blocking work runs only
//! through [`blocking`](super::blocking).
//!
//! The folder is made at the first write (a saved password, or the first
//! slot), never at launch: someone who never turns OBS on gets none. Every
//! log line goes through [`log`](super::log), with an `io::ErrorKind` at
//! most: nothing of the password, its blob or a path.

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use tokio::runtime::Handle;
use zeroize::Zeroizing;

use super::scenes::Slots;
use super::{blocking, log, ENCRYPT_FAILED, KEEP_RETURNS, WRITE_FAILED};

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

/// D28: DPAPI for the current Windows user, with [`ENTROPY`]
/// (`divora_capture::dpapi`, which loads crypt32.dll at run time). The
/// plaintext it hands back is wrapped in `Zeroizing` at once.
pub struct Dpapi;

impl Cipher for Dpapi {
    fn protect(&self, plain: &[u8]) -> Result<Vec<u8>, String> {
        divora_capture::dpapi::protect(plain, ENTROPY).map_err(|e| e.to_string())
    }

    fn unprotect(&self, blob: &[u8]) -> Result<Zeroizing<Vec<u8>>, String> {
        divora_capture::dpapi::unprotect(blob, ENTROPY)
            .map(Zeroizing::new)
            .map_err(|e| e.to_string())
    }
}

/// D29: a stored password that won't decrypt (a profile reset, another
/// user's file): the status reads `passwordUnreadable`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Unreadable;

/// Why a password wasn't stored.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NotSaved {
    /// DPAPI refused to encrypt it.
    Encrypt,
    /// `password.bin` couldn't be written (the old one, if any, is kept).
    Write,
}

impl NotSaved {
    /// What `obs_set_password` answers (§4).
    #[must_use]
    pub fn message(self) -> &'static str {
        match self {
            Self::Encrypt => ENCRYPT_FAILED,
            Self::Write => WRITE_FAILED,
        }
    }
}

/// `path`'s temp: `.<name>.tmp` in the same folder, so the rename never
/// crosses a volume.
fn temp_of(path: &Path) -> PathBuf {
    let name = path
        .file_name()
        .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
    path.with_file_name(format!(".{name}.tmp"))
}

/// `bytes` into `path` by `.<name>.tmp` in the same folder (made if it is
/// missing): written, flushed to the disk, then renamed over `path`. A
/// failed write or rename removes the temp and leaves `path` as it was.
pub fn write_atomically(path: &Path, bytes: &[u8]) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let temp = temp_of(path);
    let written = fs::File::create(&temp).and_then(|mut file| {
        file.write_all(bytes)?;
        file.sync_all()
    });
    let landed = written.and_then(|()| fs::rename(&temp, path));
    if landed.is_err() {
        let _ = fs::remove_file(&temp);
    }
    landed
}

/// D28: `pw` encrypted by `cipher` into `<dir>/password.bin`, by
/// [`write_atomically`]. Blocking: called only through
/// [`blocking`](super::blocking).
pub fn save_password(
    dir: &Path,
    cipher: &dyn Cipher,
    pw: &Zeroizing<String>,
) -> Result<(), NotSaved> {
    let Ok(blob) = cipher.protect(pw.as_bytes()) else {
        log::password_not_saved("encrypt", None);
        return Err(NotSaved::Encrypt);
    };
    if let Err(e) = write_atomically(&dir.join(PASSWORD_FILE), &blob) {
        log::password_not_saved("write", Some(e.kind()));
        return Err(NotSaved::Write);
    }
    log::password_saved();
    Ok(())
}

/// D12, D29: the stored password, decrypted (wiped when dropped); `None`
/// when none is stored. A file that can't be read, or won't decrypt, is
/// [`Unreadable`]. Blocking: called only through
/// [`blocking`](super::blocking).
pub fn load_password(
    dir: &Path,
    cipher: &dyn Cipher,
) -> Result<Option<Zeroizing<Vec<u8>>>, Unreadable> {
    let blob = match fs::read(dir.join(PASSWORD_FILE)) {
        Ok(blob) => blob,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => {
            log::password_unreadable("read", Some(e.kind()));
            return Err(Unreadable);
        }
    };
    if let Ok(plain) = cipher.unprotect(&blob) {
        Ok(Some(plain))
    } else {
        log::password_unreadable("decrypt", None);
        Err(Unreadable)
    }
}

/// `password.bin` (and any temp a cut-short save left) deleted; none there
/// is fine. Blocking.
pub fn forget_password(dir: &Path) -> io::Result<()> {
    let path = dir.join(PASSWORD_FILE);
    let _ = fs::remove_file(temp_of(&path));
    match fs::remove_file(&path) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => {
            log::password_not_forgotten(e.kind());
            return Err(e);
        }
    }
    log::password_forgotten();
    Ok(())
}

/// Whether `password.bin` exists (the status's `hasPassword`). Reads
/// nothing of it.
#[must_use]
pub fn has_password(dir: &Path) -> bool {
    dir.join(PASSWORD_FILE).is_file()
}

/// D24: the slots `<dir>/returns.json` holds, read at `wall_ms` (a slot
/// older than 24 h dropped, [`Slots::from_json`]). None there, or
/// [`KEEP_RETURNS`] off: none. A file that can't be read, or isn't a
/// version-1 returns file, is logged and read as none. Small: read once, on
/// the main thread, at setup.
#[must_use]
pub fn load_returns(dir: &Path, wall_ms: i64) -> Slots {
    if !KEEP_RETURNS {
        return Slots::default();
    }
    let text = match fs::read_to_string(dir.join(RETURNS_FILE)) {
        Ok(text) => text,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Slots::default(),
        Err(e) => {
            log::returns_unreadable(Some(e.kind()));
            return Slots::default();
        }
    };
    let readable = serde_json::from_str::<serde_json::Value>(&text)
        .is_ok_and(|v| v.get("version") == Some(&serde_json::json!(1)));
    if !readable {
        log::returns_unreadable(None);
    }
    Slots::from_json(&text, wall_ms)
}

/// What [`SlotSaver`] writes a value with.
pub type SlotWriter = Box<dyn Fn(&str) -> io::Result<()> + Send + Sync>;

/// The newest value not written yet, and its number.
#[derive(Default)]
struct Pending {
    seq: u64,
    json: Option<String>,
}

/// D24: the one writer of `returns.json`, latest wins. [`SlotSaver::save`]
/// stores the value and, unless a job is already writing, starts one job on
/// the blocking pool, which writes the pending value until none is left, so
/// two writes (and two temp files) never overlap and the link never waits
/// on the disk. Values carry the slots' change number, so of two saves that
/// race, the later state stays whatever order they arrive in.
pub struct SlotSaver {
    pending: Mutex<Pending>,
    /// A job is writing.
    busy: AtomicBool,
    write: SlotWriter,
    spawner: Handle,
}

impl SlotSaver {
    /// The app's: `<dir>/returns.json`, by [`write_atomically`], on
    /// `spawner`'s blocking pool.
    #[must_use]
    pub fn new(spawner: Handle, dir: PathBuf) -> Arc<Self> {
        let path = dir.join(RETURNS_FILE);
        Self::with_writer(
            spawner,
            Box::new(move |json| write_atomically(&path, json.as_bytes())),
        )
    }

    /// A saver that writes with `write` (tests: a slow one).
    #[must_use]
    pub fn with_writer(spawner: Handle, write: SlotWriter) -> Arc<Self> {
        Arc::new(Self {
            pending: Mutex::new(Pending::default()),
            busy: AtomicBool::new(false),
            write,
            spawner,
        })
    }

    /// `json`, the slots' state number `seq` (from 1), to be written soon;
    /// one no newer than a value already saved is dropped. Never waits on
    /// the disk. With [`KEEP_RETURNS`] off, nothing.
    pub fn save(self: &Arc<Self>, seq: u64, json: String) {
        if !KEEP_RETURNS {
            return;
        }
        {
            let mut pending = lock(&self.pending);
            if seq <= pending.seq {
                return;
            }
            pending.seq = seq;
            pending.json = Some(json);
        }
        if !self.busy.swap(true, Ordering::AcqRel) {
            self.start();
        }
    }

    /// Whether nothing is pending and no job is writing.
    #[must_use]
    pub fn idle(&self) -> bool {
        !self.busy.load(Ordering::Acquire) && lock(&self.pending).json.is_none()
    }

    /// The one job (`busy` is set).
    fn start(self: &Arc<Self>) {
        let me = Arc::clone(self);
        // Detached: it ends when nothing is left to write.
        drop(blocking(&self.spawner, move || me.drain()));
    }

    /// Writes what is pending until none is left, then lets go of `busy`
    /// and looks once more: a save that came between the last take and the
    /// letting go found `busy` set and started nothing, so its value is
    /// taken here (no lost wake-up).
    fn drain(&self) {
        loop {
            loop {
                let next = lock(&self.pending).json.take();
                let Some(json) = next else {
                    break;
                };
                if let Err(e) = (self.write)(&json) {
                    log::returns_not_saved(e.kind());
                }
            }
            self.busy.store(false, Ordering::Release);
            if lock(&self.pending).json.is_none() || self.busy.swap(true, Ordering::AcqRel) {
                return;
            }
        }
    }
}

/// A leaf lock, poisoned or not.
fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic
)]
mod tests {
    use super::{
        forget_password, has_password, load_password, load_returns, save_password,
        write_atomically, Cipher, Dpapi, NotSaved, SlotSaver, Unreadable, ENTROPY, PASSWORD_FILE,
        RETURNS_FILE,
    };
    use crate::obs::scenes::{SceneRef, Slots};
    use crate::obs::testing::{assert_none_in, scratch, secret_forms, traced, MemCipher};
    use crate::obs::{blocking, ENCRYPT_FAILED, WRITE_FAILED};
    use std::path::Path;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::time::{Duration, Instant};
    use zeroize::Zeroizing;

    const PASSWORD: &str = "correct horse \u{2713} 7Qz";

    fn pw(s: &str) -> Zeroizing<String> {
        Zeroizing::new(s.to_string())
    }

    fn runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
    }

    fn names_in(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    /// D28: changing the entropy orphans every saved password: no blob
    /// written before opens after.
    #[test]
    fn entropy_is_pinned() {
        assert_eq!(ENTROPY, b"DivoraVoice OBS password v1");
    }

    /// D28: the app's cipher passes the entropy, so a bare
    /// `CryptUnprotectData` (no entropy) can't open its blob, and another
    /// entropy can't either. DPAPI needs the user's profile: outside CI a
    /// refusal to protect is said and skipped; on CI it fails (§10.4).
    #[test]
    fn dpapi_protect_needs_the_entropy() {
        let blob = match Dpapi.protect(PASSWORD.as_bytes()) {
            Ok(blob) => blob,
            Err(e) if std::env::var_os("CI").is_none() => {
                eprintln!(
                    "dpapi_protect_needs_the_entropy skipped: DPAPI can't protect here ({e})"
                );
                return;
            }
            Err(e) => panic!("DPAPI refused to protect on CI: {e}"),
        };
        assert!(divora_capture::dpapi::unprotect(&blob, b"").is_err());
        assert!(divora_capture::dpapi::unprotect(&blob, b"DivoraVoice OBS password v2").is_err());
        assert_eq!(
            Dpapi.unprotect(&blob).unwrap().as_slice(),
            PASSWORD.as_bytes()
        );
    }

    /// D28, §4: the password goes to `password.bin` by a temp file and a
    /// rename. A write that fails (its temp can't be made) leaves the old
    /// file as it was and no temp; a refused encrypt writes nothing. Each
    /// answers its message.
    #[test]
    fn password_file_is_written_by_rename() {
        let dir = scratch("password-rename");
        let cipher = MemCipher::default();
        save_password(&dir, &cipher, &pw("first")).unwrap();
        let file = dir.join(PASSWORD_FILE);
        let first = std::fs::read(&file).unwrap();
        assert_eq!(names_in(&dir), [PASSWORD_FILE]);
        // A folder where the temp goes: the temp can't be created.
        let temp = dir.join(format!(".{PASSWORD_FILE}.tmp"));
        std::fs::create_dir(&temp).unwrap();
        assert_eq!(
            save_password(&dir, &cipher, &pw("second")),
            Err(NotSaved::Write)
        );
        assert_eq!(std::fs::read(&file).unwrap(), first, "the old file stays");
        std::fs::remove_dir(&temp).unwrap();
        assert_eq!(
            save_password(&dir, &MemCipher::refusing(), &pw("third")),
            Err(NotSaved::Encrypt)
        );
        assert_eq!(std::fs::read(&file).unwrap(), first);
        save_password(&dir, &cipher, &pw("fourth")).unwrap();
        assert_eq!(
            load_password(&dir, &cipher).unwrap().unwrap().as_slice(),
            b"fourth"
        );
        assert_eq!(names_in(&dir), [PASSWORD_FILE], "no temp left");
        assert_eq!(NotSaved::Write.message(), WRITE_FAILED);
        assert_eq!(NotSaved::Encrypt.message(), ENCRYPT_FAILED);
        std::fs::remove_dir_all(&dir).ok();
    }

    /// D29: a stored blob that won't decrypt (another user's, a profile
    /// reset, a broken file) or can't be read is `Unreadable`; none stored
    /// is `None`, and so is a folder that was never made.
    #[test]
    fn a_blob_that_wont_decrypt_reads_unreadable() {
        let dir = scratch("password-unreadable");
        let cipher = MemCipher::default();
        assert_eq!(load_password(&dir, &cipher), Ok(None));
        assert!(!dir.exists(), "reading makes no folder");
        assert_eq!(cipher.unprotects(), 0, "nothing to decrypt");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(PASSWORD_FILE), b"not a blob").unwrap();
        assert_eq!(load_password(&dir, &cipher), Err(Unreadable));
        assert_eq!(cipher.unprotects(), 1);
        std::fs::remove_file(dir.join(PASSWORD_FILE)).unwrap();
        // A folder in its place: the read fails.
        std::fs::create_dir(dir.join(PASSWORD_FILE)).unwrap();
        assert_eq!(load_password(&dir, &cipher), Err(Unreadable));
        std::fs::remove_dir_all(&dir).ok();
    }

    /// Forget password deletes the file (none there is fine), and with it
    /// `hasPassword`.
    #[test]
    fn forget_removes_the_file() {
        let dir = scratch("password-forget");
        let cipher = MemCipher::default();
        forget_password(&dir).unwrap();
        assert!(!has_password(&dir));
        save_password(&dir, &cipher, &pw(PASSWORD)).unwrap();
        assert!(has_password(&dir));
        forget_password(&dir).unwrap();
        assert!(!has_password(&dir));
        assert_eq!(load_password(&dir, &cipher), Ok(None));
        assert_eq!(names_in(&dir), Vec::<String>::new());
        std::fs::remove_dir_all(&dir).ok();
    }

    /// §10.3: the real save, load and forget, through every branch — saved,
    /// refused, unwritable, loaded, unreadable, unreadable by I/O — on the
    /// blocking pool as the commands and the link run them, logging at
    /// TRACE: no form of the password reaches a line.
    #[test]
    fn load_password_logs_nothing_secret() {
        let rt = runtime();
        let dir = scratch("password-logs");
        let cipher: Arc<dyn Cipher> = Arc::new(MemCipher::default());
        let ((), log) = traced(|| {
            let run = |f: Box<dyn FnOnce() + Send>| {
                rt.block_on(blocking(rt.handle(), f)).unwrap();
            };
            let (d, c) = (dir.clone(), cipher.clone());
            run(Box::new(move || {
                save_password(&d, &*c, &pw(PASSWORD)).unwrap();
                let loaded = load_password(&d, &*c).unwrap().unwrap();
                assert_eq!(loaded.as_slice(), PASSWORD.as_bytes());
            }));
            let d = dir.clone();
            run(Box::new(move || {
                let temp = d.join(format!(".{PASSWORD_FILE}.tmp"));
                let _ = save_password(&d, &MemCipher::refusing(), &pw(PASSWORD));
                std::fs::create_dir(&temp).unwrap();
                let _ = save_password(&d, &MemCipher::default(), &pw(PASSWORD));
                std::fs::remove_dir(&temp).unwrap();
            }));
            let (d, c) = (dir.clone(), cipher.clone());
            run(Box::new(move || {
                std::fs::write(d.join(PASSWORD_FILE), PASSWORD).unwrap();
                assert_eq!(load_password(&d, &*c), Err(Unreadable));
                std::fs::remove_file(d.join(PASSWORD_FILE)).unwrap();
                std::fs::create_dir(d.join(PASSWORD_FILE)).unwrap();
                assert_eq!(load_password(&d, &*c), Err(Unreadable));
                std::fs::remove_dir(d.join(PASSWORD_FILE)).unwrap();
                forget_password(&d).unwrap();
            }));
        });
        // The lines are there (the capture reached the pool) …
        for line in [
            "obs: password saved",
            "obs: couldn't save the password",
            "obs: the saved password can't be read",
            "obs: password forgotten",
        ] {
            assert!(log.contains(line), "{line}: not logged");
        }
        // … and nothing of the password is in them.
        assert_none_in(&log, &secret_forms(PASSWORD));
        std::fs::remove_dir_all(&dir).ok();
    }

    /// D24: the slots' file as `ObsShared` reads it at launch: none there is
    /// none (and makes no folder); one written from the slots reads back, a
    /// slot older than 24 h dropped; one that isn't a version-1 returns file
    /// reads as none, and is logged.
    #[test]
    fn returns_file_round_trip_and_prune() {
        let dir = scratch("returns");
        let wall = 1_793_217_600_000_i64;
        let empty = Slots::default().to_json();
        assert_eq!(load_returns(&dir, wall).to_json(), empty);
        assert!(!dir.exists(), "reading makes no folder");
        let scene = |name: &str| SceneRef {
            uuid: format!("5d3f1c9e-{name}"),
            name: name.into(),
        };
        let mut slots = Slots::default();
        slots.after_scene("timer-1", scene("Game"), scene("BRB"), wall - 1_000);
        slots.after_scene("timer-2", scene("Game"), scene("Chat"), wall - 90_000_000);
        write_atomically(&dir.join(RETURNS_FILE), slots.to_json().as_bytes()).unwrap();
        let read = load_returns(&dir, wall);
        assert_eq!(
            read.plan_back("timer-1", "5d3f1c9e-BRB").to,
            Some(scene("Game"))
        );
        assert_eq!(
            read.plan_back("timer-2", "5d3f1c9e-Chat").to,
            None,
            "past 24 h"
        );
        let ((), log) = traced(|| {
            for text in ["{not json", r#"{"version":2,"returns":[]}"#] {
                std::fs::write(dir.join(RETURNS_FILE), text).unwrap();
                assert_eq!(load_returns(&dir, wall).to_json(), empty);
            }
        });
        assert_eq!(
            log.matches("obs: returns file unreadable; starting empty")
                .count(),
            2,
            "{log}"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    /// D24: the saver writes one value at a time, latest wins. Saves from
    /// four threads, faster than a slow writer: never two writes at once
    /// (so never two temp files), the last state on disk, no temp left, and
    /// a save older than one already taken never lands.
    #[test]
    fn two_saves_leave_the_later_state() {
        let rt = runtime();
        let dir = scratch("saver");
        let path = dir.join(RETURNS_FILE);
        let now = Arc::new(AtomicUsize::new(0));
        let most = Arc::new(AtomicUsize::new(0));
        let writes = Arc::new(AtomicUsize::new(0));
        let saver = {
            let (now, most, writes, path) =
                (now.clone(), most.clone(), writes.clone(), path.clone());
            SlotSaver::with_writer(
                rt.handle().clone(),
                Box::new(move |json| {
                    let at_once = now.fetch_add(1, Ordering::SeqCst) + 1;
                    most.fetch_max(at_once, Ordering::SeqCst);
                    std::thread::sleep(Duration::from_millis(15));
                    let landed = write_atomically(&path, json.as_bytes());
                    writes.fetch_add(1, Ordering::SeqCst);
                    now.fetch_sub(1, Ordering::SeqCst);
                    landed
                }),
            )
        };
        let seq = AtomicUsize::new(0);
        std::thread::scope(|s| {
            for _ in 0..4 {
                s.spawn(|| {
                    for _ in 0..25 {
                        let n = seq.fetch_add(1, Ordering::SeqCst) + 1;
                        saver.save(n as u64, format!("{{\"n\":{n}}}"));
                        std::thread::sleep(Duration::from_millis(2));
                    }
                });
            }
        });
        let deadline = Instant::now() + Duration::from_secs(5);
        while !saver.idle() {
            assert!(Instant::now() < deadline, "the saver never finished");
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(most.load(Ordering::SeqCst), 1, "two writes at once");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "{\"n\":100}");
        let wrote = writes.load(Ordering::SeqCst);
        assert!(wrote < 100, "latest wins: {wrote} writes for 100 saves");
        assert_eq!(names_in(&dir), [RETURNS_FILE], "no temp left");
        // An older state than one already saved is dropped.
        saver.save(50, "{\"n\":50}".into());
        assert!(saver.idle());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "{\"n\":100}");
        drop(rt);
        std::fs::remove_dir_all(&dir).ok();
    }
}
