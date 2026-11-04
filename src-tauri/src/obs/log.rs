//! v1.61.0 (D40): the only file under `obs/` that names a `tracing` macro.
//! Each helper takes only fixed codes, versions, uuids, timer ids, counts
//! and `io::ErrorKind`s — never the password, the secret, the auth string, a
//! pushed text, a source's settings, or a request's or an event's body — so
//! a source test can check that no secret reaches a log line on any thread
//! (§10.5).
//!
//! The log writer is synchronous on the calling thread (a tokio worker
//! blocks for it), so lines are few: state changes at info, the first of a
//! run of failures at warn, and the lines that could repeat at most once per
//! [`DROPPED_EVERY`].

use std::io;
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

use super::machine::SkipWhy;

/// D40: a dropped scene op is said at most once per this.
pub const DROPPED_EVERY: Duration = Duration::from_secs(10);

/// A line said at most once per a period: when it was last said.
struct Every(Mutex<Option<Instant>>);

impl Every {
    const fn new() -> Self {
        Self(Mutex::new(None))
    }

    /// Whether to say it now; if so, now is when it was last said.
    fn due(&self, period: Duration) -> bool {
        let now = Instant::now();
        let mut last = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        if last.is_some_and(|at| now.saturating_duration_since(at) < period) {
            return false;
        }
        *last = Some(now);
        true
    }
}

static DROPPED: Every = Every::new();

// ---- `ObsShared` (track C) ---------------------------------------------------

/// D4: a link generation started, for a loopback address or not (the
/// address itself isn't logged).
pub fn link_started(gen: u64, loopback: bool) {
    tracing::info!(gen, loopback, "obs link started");
}

/// D4, D5: a link generation was told to stop: `why` is `"off"`,
/// `"restart"` or `"quit"`.
pub fn link_stopped(gen: u64, why: &'static str) {
    tracing::info!(gen, why, "obs link stopped");
}

/// D17: the running link's queue had closed (its driver had ended): the
/// handle is dropped, and the next configure starts a new link.
pub fn link_gone(gen: u64) {
    tracing::warn!(
        gen,
        "obs: the link had ended; it starts again at the next change"
    );
}

/// D17, D23: a scene op that didn't switch, and why; at most one line per
/// [`DROPPED_EVERY`].
pub fn op_dropped(timer: &str, op: &'static str, why: SkipWhy) {
    if DROPPED.due(DROPPED_EVERY) {
        tracing::warn!(timer = %timer, op, why = ?why, "obs: a scene op was dropped");
    }
}

/// D24: `returns.json` couldn't be read (`kind`) or held no slots Divora can
/// read (`None`); the slots start empty.
pub fn returns_unreadable(kind: Option<io::ErrorKind>) {
    tracing::warn!(kind = ?kind, "obs: returns file unreadable; starting empty");
}

/// D24: `returns.json` couldn't be written; the next change tries again.
pub fn returns_not_saved(kind: io::ErrorKind) {
    tracing::warn!(kind = ?kind, "obs: couldn't save the returns file");
}

/// D28: a password was stored (encrypted). Nothing of it is logged.
pub fn password_saved() {
    tracing::info!("obs: password saved");
}

/// D28: a password wasn't stored: `stage` is `"encrypt"` (DPAPI refused) or
/// `"write"` (with the file's error).
pub fn password_not_saved(stage: &'static str, kind: Option<io::ErrorKind>) {
    tracing::warn!(stage, kind = ?kind, "obs: couldn't save the password");
}

/// The stored password was deleted (or there was none).
pub fn password_forgotten() {
    tracing::info!("obs: password forgotten");
}

/// `password.bin` couldn't be deleted.
pub fn password_not_forgotten(kind: io::ErrorKind) {
    tracing::warn!(kind = ?kind, "obs: couldn't delete the password file");
}

/// D29: the stored password can't be used: `stage` is `"read"` (with the
/// file's error) or `"decrypt"`. The status reads `passwordUnreadable`.
pub fn password_unreadable(stage: &'static str, kind: Option<io::ErrorKind>) {
    tracing::warn!(stage, kind = ?kind, "obs: the saved password can't be read");
}

// ---- The link (track B1) ------------------------------------------------------

/// D14: after a run of failures' first line, "still can't reach OBS" is said
/// at most once per this.
pub const STILL_EVERY: Duration = Duration::from_secs(300);

/// D40: a request timeout (after the first of a run), a refused push, a
/// refused seed are each said at most once per this.
pub const REQUEST_EVERY: Duration = Duration::from_secs(10);

static STILL: Every = Every::new();
static TIMED_OUT: Every = Every::new();
static PUSH_REFUSED: Every = Every::new();

/// D14, §7.1: an attempt failed, or a connection ended, and the link waits
/// to try again. The first of a run of failures (`failures` 1) at warn, with
/// the I/O error's kind if there was one; after it, at most one "still"
/// line per [`STILL_EVERY`].
pub fn retrying(
    gen: u64,
    reason: super::machine::ObsReason,
    failures: u32,
    kind: Option<io::ErrorKind>,
) {
    if failures <= 1 {
        // The first line of a run starts the "still" clock.
        let _ = STILL.due(STILL_EVERY);
        tracing::warn!(gen, reason = ?reason, kind = ?kind, "obs: can't reach OBS; trying again");
    } else if STILL.due(STILL_EVERY) {
        tracing::warn!(gen, reason = ?reason, failures, "obs: still can't reach OBS");
    }
}

/// D13: the link stopped trying (`state` is a stop state) until the address,
/// the password or Try again changes something.
pub fn stopped_trying(gen: u64, state: super::machine::ObsState) {
    tracing::warn!(gen, state = ?state, "obs: stopped trying until something changes");
}

/// D11, D40: Identified and the startup steps done. The versions are the
/// sanitized ones the status shows (D15), never OBS's raw text.
pub fn connected(gen: u64, obs_version: Option<&str>, websocket_version: Option<&str>) {
    tracing::info!(gen, obs = ?obs_version, websocket = ?websocket_version, "obs: connected");
}

/// D10: OBS said it is quitting.
pub fn obs_exiting(gen: u64) {
    tracing::info!(gen, "obs: OBS is closing");
}

/// §7.1: OBS closed the connection with `code` (`None`: no close frame).
/// A code a correct Divora never earns (4002-4008, `protocol`) is a warn.
pub fn closed_by_obs(gen: u64, code: Option<u16>) {
    if code.is_some_and(|c| (4002..=4008).contains(&c)) {
        tracing::warn!(gen, code = ?code, "obs: OBS closed the connection as a client bug");
    } else {
        tracing::info!(gen, code = ?code, "obs: OBS closed the connection");
    }
}

/// D8, D9: a request (`request`, one of the nine names) got no answer in
/// time: the first of a run, then at most once per [`REQUEST_EVERY`].
pub fn request_timed_out(gen: u64, request: &'static str) {
    if TIMED_OUT.due(REQUEST_EVERY) {
        tracing::warn!(gen, request, "obs: OBS didn't answer in time");
    }
}

/// D20: OBS refused a text push with `code` (207 included); at most once
/// per [`REQUEST_EVERY`]. The text isn't logged.
pub fn push_refused(gen: u64, code: u64) {
    if PUSH_REFUSED.due(REQUEST_EVERY) {
        tracing::warn!(gen, code, "obs: OBS refused a timer's text");
    }
}

/// D20: OBS can't find the text source `uuid` (`code` 600-699); pushes to it
/// wait for a re-check.
pub fn source_missing(gen: u64, uuid: &str, code: u64) {
    tracing::warn!(gen, source = %uuid, code, "obs: OBS can't find a text source");
}

/// §7.13: `GetInputSettings` for `uuid` was refused (`code`): the source is
/// listed as not reading a file.
pub fn settings_unread(gen: u64, uuid: &str, code: u64) {
    tracing::warn!(gen, source = %uuid, code, "obs: couldn't read a text source's file flag");
}

/// D26: a follow seed's status request was refused (`code`, not 207): the
/// seed is dropped until the next connection, change or wake.
pub fn seed_refused(gen: u64, request: &'static str, code: u64) {
    tracing::warn!(gen, request, code, "obs: couldn't read OBS's output status");
}

/// D23: `timer` started a new run (a `scene`) while an earlier run's
/// switch back was due: that switch back was dropped.
pub fn scene_back_superseded(timer: &str) {
    tracing::info!(timer = %timer, "obs: a break started again; its earlier switch back was dropped");
}

/// D23: `timer`'s switch back waited out its grace and was dropped.
pub fn scene_back_expired(timer: &str) {
    tracing::warn!(timer = %timer, "obs: a switch back waited too long and was dropped");
}
