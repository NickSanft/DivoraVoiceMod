//! v1.61.0: Talk to OBS — Divora's own obs-websocket v5 client, off until
//! Settings → OBS turns it on.
//!
//! Turned on, Divora keeps one WebSocket connection to OBS's own server at
//! an IP address (D7, D16) and does three things with it: a timer's text in
//! an OBS text source (D18), a switch to a break scene and back (D21-D24),
//! and a count-up that follows OBS's stream or recording (D26). Divora never
//! listens, looks no name up, and sends only the nine requests of
//! [`proto::Request`] (D9).
//!
//! The parts, pure first:
//! - [`proto`]: the wire — Hello, Identify, the auth string, the nine
//!   requests, the six events, close codes, text sources.
//! - [`machine`]: the link's rules — the status and the other wire types,
//!   the status cell, hosts, backoff, Follow OBS, text pushes, request
//!   failures.
//! - [`scenes`]: which scene each break replaced, and the switch-backs due.
//! - [`link`]: a running link's two tasks (the driver and the session), on
//!   Tauri's runtime, only while switched on.
//! - [`files`]: the password (DPAPI) and the slots' file.
//! - [`log`]: the only file here that names a `tracing` macro (D40).
//!
//! This file holds what they share: [`ObsShared`], the state the commands,
//! the timer worker and every link generation reach (D3); the kill switches
//! (D41) and limits; and the link's channels, timings and traits, so the
//! link is tested against stubs (§10.2). Nothing waits on OBS: the timer
//! thread hands its feed through a `watch` (latest wins), and the timer
//! worker's ops go through [`ObsShared::offer`], which never waits (D17).
//!
//! The wire contract — the command and event names, the keys of the
//! config, the status and the inventory, the states, reasons and skip
//! words, the nine requests, the limits, the host table and the auth
//! vectors — is `src/data/obs.fixture.json`, which the frontend's tests
//! read too, so a change on either side fails the other's tests.

// D39: release builds abort on a panic, with no hook, so one in a link
// task would end Divora silently. Test modules allow these.
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic
)]

/// The fake obs-websocket v5 server the link's tests use (§10.2).
#[cfg(test)]
mod fake;
pub mod files;
pub mod link;
pub mod log;
pub mod machine;
pub mod proto;
pub mod scenes;
/// v1.61.0 (track G): the seam's tests — links started by `ObsShared`
/// against the fake, Follow OBS on a real `TimerShared` (§10.2, §12).
#[cfg(test)]
mod seam_tests;
/// What the OBS tests share: an in-memory cipher, a log capture.
#[cfg(test)]
pub(crate) mod testing;

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use tauri::{AppHandle, Emitter, Manager};
use tokio::runtime::Handle;
use tokio::sync::{mpsc, oneshot, watch};
use tokio::task::JoinHandle;
use zeroize::Zeroizing;

pub use crate::timer_actions::ObsOp;
pub use crate::timers::{Follow, FollowObs, ObsFeed, ObsText};

use crate::timer_thread::TimerShared;
use crate::timers::TimerOp;
use files::{Cipher, SlotSaver, Unreadable};
use machine::{
    parse_host, LastSkip, LinkConfig, ObsConfig, ObsInventory, ObsState, ObsStatus, SkipOp,
    SkipWhy, StatusCell,
};
use scenes::{BackPlan, SceneRef, Slots};

// ---- Kill switches (D41) ----------------------------------------------------

/// `false`: no link ever starts, the status stays `off`, and
/// [`ObsShared::offer`] does nothing.
pub const OBS_LINK: bool = true;

/// `false`: no text push is ever sent.
pub const OBS_TEXT: bool = true;

/// D23: how long a switch back waits for a connected link, counted from the
/// moment [`ObsShared::offer`] received it. `ZERO` is the plan's rule: a
/// switch back that finds OBS disconnected is dropped.
pub const SCENE_BACK_GRACE: Duration = Duration::from_secs(120);

/// D24: `false`: no `returns.json`. Slots still outlive a link, not a Divora
/// restart.
pub const KEEP_RETURNS: bool = true;

/// D8: no frame of any kind within this of a ping drops the connection
/// (`silent`). `None` pings with no deadline.
pub const KEEPALIVE_DEADLINE: Option<Duration> = Some(Duration::from_secs(10));

// ---- Limits (fixture `limits`) --------------------------------------------

/// D17: ops waiting for a connected link at most; one past it is a skip
/// (`queueFull`) or a due mark.
pub const OPS_QUEUE: usize = 16;

/// D10: decoded events waiting for the driver at most; a full queue ends
/// the connection (`protocol`).
pub const EVENTS_QUEUE: usize = 256;

/// D23: a `scene` the driver takes later than this after `offer` stamped it
/// is skipped (`late`), never run late.
pub const SCENE_MAX_AGE: Duration = Duration::from_secs(5);

/// D8, D9: a request still unanswered this long after it was sent ends the
/// connection (`noAnswer`).
pub const LATE_ANSWER_CAP: Duration = Duration::from_secs(15);

/// D8: a startup step answered 207 (not ready) is sent again after this …
pub const NOT_READY_WAIT: Duration = Duration::from_secs(1);

/// … for at most this in all; then the connection ends (`notReady`).
pub const NOT_READY_CAP: Duration = Duration::from_secs(30);

/// D15: `inventoryRev` changes at most once per this.
pub const REV_COALESCE: Duration = Duration::from_millis(250);

/// D20: a text source OBS couldn't find is tried again after this.
pub const MISSING_RECHECK: Duration = Duration::from_secs(60);

/// D7: the largest frame read once connected: 4 MiB.
pub const FRAME_CAP_BYTES: usize = 4 << 20;

/// D7: the most bytes read before the upgrade completes: 64 KiB.
pub const HANDSHAKE_CAP_BYTES: usize = 64 << 10;

/// D19: text sources an inventory lists at most (`truncated` past it).
pub const MAX_TEXT_SOURCES: usize = 100;

/// D19: scenes an inventory lists at most (`truncated` past it).
pub const MAX_SCENES: usize = 200;

/// §4: a password's longest, in characters.
pub const MAX_PASSWORD_CHARS: usize = 256;

/// D21: a scene name an `obs` action carries, at most this many characters
/// (it is only shown: the uuid switches).
pub const MAX_SCENE_NAME_CHARS: usize = 256;

/// D18, D21: a scene's or a text source's uuid, 1 to this many characters
/// (OBS's are 36).
pub const MAX_UUID_CHARS: usize = 128;

/// D5: how long the tray's Quit waits for the link to close its connection.
pub const EXIT_WAIT: Duration = Duration::from_millis(300);

// ---- Wire names and messages (§4) ------------------------------------------

/// The event every status change goes out as, to every window. Payload:
/// [`ObsStatus`]; on change only.
pub const OBS_STATUS_EVENT: &str = "obs-status";

/// `obs_configure`: a port of 0 (Tauri refuses anything that isn't a `u16`).
pub const BAD_PORT: &str = "Type a port from 1 to 65535.";

/// `obs_set_password`: an empty password.
pub const NO_PASSWORD: &str = "Paste a password first.";

/// `obs_set_password`: past [`MAX_PASSWORD_CHARS`].
pub const LONG_PASSWORD: &str = "The password can be up to 256 characters.";

/// `obs_set_password`: DPAPI refused.
pub const ENCRYPT_FAILED: &str = "Couldn't save the password: Windows couldn't encrypt it.";

/// `obs_set_password`: `password.bin` couldn't be written.
pub const WRITE_FAILED: &str = "Couldn't save the password: the file couldn't be written.";

/// `obs_inventory`: no connected link.
pub const NOT_CONNECTED: &str = "OBS isn't connected.";

/// `obs_inventory`: OBS took longer than the command's 15 s.
pub const NO_ANSWER: &str = "OBS didn't answer in time.";

/// `obs_inventory`: OBS answered 207 (not ready) for the command's 15 s.
pub const BUSY: &str = "OBS is busy. Try again in a moment.";

/// `obs_inventory`: OBS refused the scene or input list.
pub const REFUSED: &str = "OBS refused the request.";

/// The folder under the app's data folder that holds `password.bin` and
/// `returns.json`: `obs-dev` for `tauri dev`, which keeps its own, as the
/// timers' folder does (D28, K14).
#[must_use]
pub fn obs_dir_name(is_dev: bool) -> &'static str {
    if is_dev {
        "obs-dev"
    } else {
        "obs"
    }
}

// ---- The link's channels and timings (§5.5) --------------------------------

/// One op for a connected link, stamped when [`ObsShared::offer`] received
/// it: a `scene` older than [`SCENE_MAX_AGE`] when the driver takes it is
/// skipped (D23).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LinkOp {
    pub at: Instant,
    /// The timer id whose action it is.
    pub timer: String,
    pub op: ObsOp,
}

/// What a command asks of a running link. Dropping the sender is the stop:
/// every wait in the driver watches for it (D4, D6).
#[derive(Debug)]
pub enum LinkCmd {
    /// `obs_inventory`: scenes and text sources, one request per idle turn
    /// (D19); a second while one runs joins it.
    Inventory(oneshot::Sender<Result<ObsInventory, String>>),
    /// `obs_reconnect` while retrying: attempt now, the failure count kept
    /// (D4).
    Wake,
}

/// Every period the link waits by (D5, D8, D14, D18, D20, D23), as
/// [`LinkTiming::new`] checks them. Tests scale them
/// ([`LinkTiming::scaled`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LinkPeriods {
    /// D7: the TCP connect.
    pub connect: Duration,
    /// D8: the upgrade and Hello.
    pub handshake: Duration,
    /// D8: Identify → Identified.
    pub identify: Duration,
    /// D8: a request to a loopback address …
    pub request_loopback: Duration,
    /// … and to any other.
    pub request_other: Duration,
    /// D9: a timed-out request still outstanding this long after it was
    /// sent ends the connection.
    pub late_answer_cap: Duration,
    /// D8: the wait before a startup step answered 207 goes again …
    pub not_ready_wait: Duration,
    /// … for at most this in all.
    pub not_ready_cap: Duration,
    /// D8: a ping (empty payload) every this.
    pub ping: Duration,
    /// D8: no frame within this of a ping ends the connection; `None`, no
    /// deadline ([`KEEPALIVE_DEADLINE`]).
    pub keepalive_deadline: Option<Duration>,
    /// D14: the backoff's base steps are 1, 2, 4, 8, 15 and 30 of these.
    pub backoff_unit: Duration,
    /// D14: a connection Identified this long resets the failure count when
    /// it ends.
    pub reset_after: Duration,
    /// D23: how long a switch back stays due ([`SCENE_BACK_GRACE`]); may be
    /// zero.
    pub scene_back_grace: Duration,
    /// D23: [`SCENE_MAX_AGE`].
    pub scene_max_age: Duration,
    /// D18: a failed push is tried again after this, doubling to 30 of them.
    pub push_retry_base: Duration,
    /// D20: [`MISSING_RECHECK`].
    pub missing_recheck: Duration,
    /// D15: [`REV_COALESCE`].
    pub rev_coalesce: Duration,
    /// D5: the close frame's flush, at most.
    pub close_flush: Duration,
    /// D5: then reading to OBS's close, at most.
    pub close_echo: Duration,
}

/// The link's periods, built only through [`LinkTiming::new`], which refuses
/// a zero period (`tokio::time::interval` panics on one, D39). Reads as its
/// [`LinkPeriods`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LinkTiming(LinkPeriods);

/// D5, D8, D14, D18, D20, D23's numbers.
const PRODUCTION: LinkPeriods = LinkPeriods {
    connect: Duration::from_secs(3),
    handshake: Duration::from_secs(5),
    identify: Duration::from_secs(5),
    request_loopback: Duration::from_secs(3),
    request_other: Duration::from_secs(5),
    late_answer_cap: LATE_ANSWER_CAP,
    not_ready_wait: NOT_READY_WAIT,
    not_ready_cap: NOT_READY_CAP,
    ping: Duration::from_secs(15),
    keepalive_deadline: KEEPALIVE_DEADLINE,
    backoff_unit: Duration::from_secs(1),
    reset_after: Duration::from_secs(10),
    scene_back_grace: SCENE_BACK_GRACE,
    scene_max_age: SCENE_MAX_AGE,
    push_retry_base: Duration::from_secs(1),
    missing_recheck: MISSING_RECHECK,
    rev_coalesce: REV_COALESCE,
    close_flush: Duration::from_secs(1),
    close_echo: Duration::from_millis(250),
};

impl LinkTiming {
    /// `p`, unless a period is zero: `Err` names it. Only the switch-back
    /// grace may be zero (its kill switch); a keepalive deadline may be
    /// `None`, not zero.
    pub fn new(p: LinkPeriods) -> Result<Self, &'static str> {
        let periods = [
            ("connect", p.connect),
            ("handshake", p.handshake),
            ("identify", p.identify),
            ("request_loopback", p.request_loopback),
            ("request_other", p.request_other),
            ("late_answer_cap", p.late_answer_cap),
            ("not_ready_wait", p.not_ready_wait),
            ("not_ready_cap", p.not_ready_cap),
            ("ping", p.ping),
            ("backoff_unit", p.backoff_unit),
            ("reset_after", p.reset_after),
            ("scene_max_age", p.scene_max_age),
            ("push_retry_base", p.push_retry_base),
            ("missing_recheck", p.missing_recheck),
            ("rev_coalesce", p.rev_coalesce),
            ("close_flush", p.close_flush),
            ("close_echo", p.close_echo),
        ];
        if let Some((name, _)) = periods.iter().find(|(_, d)| d.is_zero()) {
            return Err(name);
        }
        if p.keepalive_deadline.is_some_and(|d| d.is_zero()) {
            return Err("keepalive_deadline");
        }
        Ok(Self(p))
    }

    /// The app's: D5, D8, D14, D18, D20, D23's numbers, with the kill
    /// switches' values.
    #[must_use]
    pub fn production() -> Self {
        Self(PRODUCTION)
    }

    /// §10.2's test timings, in `unit`s (100 ms gives §10.2's numbers):
    /// connect 3, handshake and Identify 5 each, a request 2, the late-answer
    /// cap 10, the 207 wait 1 and its cap 10, the ping 2 and its deadline 2,
    /// the backoff unit 2, the reset 4, the grace 20, a scene's age 5, the
    /// push retry 1, the missing re-check 10, the rev 1, the close's flush 2
    /// and its echo 1. The kill switches hold: no deadline stays none, a zero
    /// grace stays zero.
    pub fn scaled(unit: Duration) -> Result<Self, &'static str> {
        let n = |times: u32| unit.saturating_mul(times);
        Self::new(LinkPeriods {
            connect: n(3),
            handshake: n(5),
            identify: n(5),
            request_loopback: n(2),
            request_other: n(2),
            late_answer_cap: n(10),
            not_ready_wait: n(1),
            not_ready_cap: n(10),
            ping: n(2),
            keepalive_deadline: KEEPALIVE_DEADLINE.map(|_| n(2)),
            backoff_unit: n(2),
            reset_after: n(4),
            scene_back_grace: if SCENE_BACK_GRACE.is_zero() {
                Duration::ZERO
            } else {
                n(20)
            },
            scene_max_age: n(5),
            push_retry_base: n(1),
            missing_recheck: n(10),
            rev_coalesce: n(1),
            close_flush: n(2),
            close_echo: n(1),
        })
    }
}

impl std::ops::Deref for LinkTiming {
    type Target = LinkPeriods;

    fn deref(&self) -> &LinkPeriods {
        &self.0
    }
}

// ---- The link's traits (§5.5): `ObsShared` in the app, stubs in tests -------

/// Where the link writes the status. In the app, `ObsShared`'s status cell,
/// whose change bumps `seq` and emits `obs-status` under its lock (D15).
pub trait StatusSink: Send + Sync {
    /// Applies `f` to the status if `gen` is the cell's generation; `false`
    /// (and nothing written) when it is stale: a newer link has started.
    fn write(&self, gen: u64, f: &mut dyn FnMut(&mut ObsStatus)) -> bool;
    /// The cell's generation now. A driver whose own is older ends quietly
    /// (D4).
    fn current_gen(&self) -> u64;
    /// Whether the last config was `enabled` (a skip is shown only then).
    fn enabled(&self) -> bool;
}

/// The slots and due marks (D22-D24), and the uuids Divora wrote (D18). In
/// the app, `ObsShared`'s: they outlive every link. A change is saved inside,
/// through the slot saver; the link never waits on the disk.
pub trait SlotStore: Send + Sync {
    /// §7.6: `scene {to}` found `cur`; whether to send the switch.
    fn after_scene(&self, timer: &str, cur: SceneRef, to: SceneRef) -> bool;
    /// §7.6: what `timer`'s switch back does with `cur_uuid` on air. Changes
    /// nothing.
    fn plan_back(&self, timer: &str, cur_uuid: &str) -> BackPlan;
    /// §7.6: the plan done (its Set answered, or none needed).
    fn commit_back(&self, timer: &str, plan: BackPlan);
    /// D23: `timer`'s switch back is due, stamped `at` (an earlier stamp
    /// stays).
    fn mark_due(&self, timer: &str, at: Instant);
    /// D23: the due mark to run next, and the timers whose marks the grace
    /// let go.
    fn take_due(&self, now: Instant) -> (Option<String>, Vec<String>);
    /// D23: the timers whose marks the grace let go at `now`, taking none
    /// to run: what a link that isn't connected does when
    /// [`next_expiry`](Self::next_expiry) comes. v1.61.0 (track C): by
    /// default through `take_due`, the one it takes marked due again (it
    /// keeps its first stamp); `ObsShared`'s takes nothing out.
    fn expire_due(&self, now: Instant) -> Vec<String> {
        let (next, expired) = self.take_due(now);
        if let Some(timer) = next {
            self.mark_due(&timer, now);
        }
        expired
    }
    /// D23: when the next due mark's grace runs out.
    fn next_expiry(&self) -> Option<Instant>;
    /// D18: the source uuids Divora wrote in this process.
    fn written(&self) -> HashSet<String>;
    /// D18: OBS confirmed a push to `uuid`: into the written set, or out of
    /// it for an empty text (the clear).
    fn note_written(&self, uuid: &str, text_empty: bool);
}

/// D26: how the link sets a timer that follows OBS. In the app,
/// [`TimerShared::apply`]: a leaf lock, held for microseconds.
pub trait TimerControl: Send + Sync {
    /// `op` on the timer `id`; `false` when no timer has that id.
    fn apply(&self, id: &str, op: TimerOp) -> bool;
}

/// D12, D29: the stored password, decrypted only when OBS's Hello asks for
/// auth. Blocking: the link calls it only through [`blocking`].
pub trait PasswordSource: Send + Sync {
    /// `None`: no password stored. [`Unreadable`]: a blob that won't
    /// decrypt.
    fn load(&self) -> Result<Option<Zeroizing<Vec<u8>>>, Unreadable>;
}

impl TimerControl for TimerShared {
    fn apply(&self, id: &str, op: TimerOp) -> bool {
        TimerShared::apply(self, id, op).is_ok()
    }
}

/// Everything one link generation runs with (§5.5): `link::run` takes it
/// whole. The receivers are this generation's own; their senders are in
/// [`ObsShared`]'s handle, and dropping the command sender stops it.
pub struct LinkDeps {
    /// The canonical address, as [`machine::parse_host`] made it.
    pub cfg: LinkConfig,
    /// This link's generation (D4): every write checks it.
    pub gen: u64,
    pub status: Arc<dyn StatusSink>,
    pub slots: Arc<dyn SlotStore>,
    pub timers: Arc<dyn TimerControl>,
    pub password: Arc<dyn PasswordSource>,
    /// The timer thread's latest feed (D17, D18): read by `borrow_and_update`
    /// and cloned at once, never held across an `.await`.
    pub feed: watch::Receiver<Arc<ObsFeed>>,
    /// [`ObsShared::offer`]'s ops, [`OPS_QUEUE`] at most.
    pub ops: mpsc::Receiver<LinkOp>,
    /// Commands; `None` from it (the sender dropped) is the stop.
    pub cmds: mpsc::Receiver<LinkCmd>,
    pub timing: LinkTiming,
    /// The backoff's draw, in [0, 1): `fastrand::f64` in the app, fixed in
    /// tests (D14).
    pub rand: fn() -> f64,
    /// Where the link's tasks and blocking work run: Tauri's runtime in the
    /// app, the test's own in tests (D3).
    pub spawner: Handle,
    /// Sent once the link has stopped: at once with no connection open, else
    /// after the close (D5).
    pub closed_ack: std::sync::mpsc::SyncSender<()>,
}

// ---- ObsShared (D3) ----------------------------------------------------------

/// What an `obs-status` emit goes through: `app.emit` in the app.
pub type Emit = Box<dyn Fn(&ObsStatus) + Send + Sync>;

/// §4: how long `obs_inventory` waits for the link's answer.
pub const INVENTORY_WAIT: Duration = Duration::from_secs(15);

/// Commands waiting for a driver at most. It takes each at once, in every
/// state, so only a stalled one fills this: `obs_inventory` then answers
/// busy, and Try again's wake is dropped (the backoff's attempt still comes).
const CMDS_QUEUE: usize = 8;

/// The running link's ends, kept by [`ObsShared`]: only the newest
/// generation's (D4).
struct LinkHandle {
    /// Its generation, and the canonical address it connects to: a
    /// configure with the same address keeps it.
    gen: u64,
    at: LinkConfig,
    ops: mpsc::Sender<LinkOp>,
    cmds: mpsc::Sender<LinkCmd>,
    /// The link's [`LinkDeps::closed_ack`].
    closed: std::sync::mpsc::Receiver<()>,
}

/// D16: what the last `obs_configure` said, in memory only: whether the
/// link is switched on, and the canonical address.
#[derive(Clone, Copy, Debug)]
struct LastConfig {
    enabled: bool,
    at: LinkConfig,
}

/// The slots and due marks, and the number of the slots' last change: the
/// saver keeps the newest file, whatever order two saves arrive in.
struct SlotState {
    slots: Slots,
    changes: u64,
}

/// The parts every link generation shares with [`ObsShared`], each behind
/// its own leaf lock (D3): handed to a driver as its [`StatusSink`] and
/// [`SlotStore`].
struct Shared {
    /// The status, its `seq` and the link generation: the only copy of it.
    status: Mutex<StatusCell>,
    emit: Emit,
    /// `None` before the first `obs_configure`.
    config: Mutex<Option<LastConfig>>,
    /// D24: the slots and due marks, for the life of the process.
    slots: Mutex<SlotState>,
    /// D23: `heldBack`, as the slots' last change left it: set under their
    /// lock and read by every status write, so no write takes both locks.
    held: AtomicU32,
    /// D18: the source uuids Divora wrote in this process.
    written: Mutex<HashSet<String>>,
    /// D24: `returns.json`'s one writer.
    saver: Arc<SlotSaver>,
    /// D23: how long a switch back stays due.
    grace: Duration,
}

impl Shared {
    /// `ObsShared`'s own write — a skip, `heldBack`, `hasPassword`, `off` —
    /// always current, emitted under the cell's lock if it changed (D15).
    fn write_shared(&self, f: impl FnOnce(&mut ObsStatus)) {
        let mut cell = lock(&self.status);
        let held = self.held.load(Ordering::Acquire);
        if let Some(s) = cell.write_shared(|s| {
            f(s);
            s.held_back = held;
        }) {
            (self.emit)(&s);
        }
    }

    /// `f` on the slots, under their lock, `heldBack` set with them. `save`:
    /// `f` may change a slot, so the file goes to the saver. The status
    /// follows once the lock is let go.
    fn with_slots<T>(&self, save: bool, f: impl FnOnce(&mut Slots) -> T) -> T {
        let (out, saved, held_moved) = {
            let mut st = lock(&self.slots);
            let out = f(&mut st.slots);
            let held = st.slots.due_count();
            let held_moved = self.held.swap(held, Ordering::AcqRel) != held;
            let saved = save.then(|| {
                st.changes = st.changes.saturating_add(1);
                (st.changes, st.slots.to_json())
            });
            (out, saved, held_moved)
        };
        if let Some((seq, json)) = saved {
            self.saver.save(seq, json);
        }
        if held_moved {
            self.write_shared(|_| {});
        }
        out
    }

    /// D17: whether `timer` has a slot — whether a switch back due for it
    /// would count in `heldBack` — asked of a copy of the slots.
    fn has_slot(&self, timer: &str) -> bool {
        let mut probe = lock(&self.slots).slots.clone();
        probe.drop_due();
        probe.mark_due(timer, Instant::now());
        probe.due_count() == 1
    }

    /// The last config, if any.
    fn last(&self) -> Option<LastConfig> {
        *lock(&self.config)
    }
}

impl StatusSink for Shared {
    fn write(&self, gen: u64, f: &mut dyn FnMut(&mut ObsStatus)) -> bool {
        let mut cell = lock(&self.status);
        if cell.gen() != gen {
            return false;
        }
        let held = self.held.load(Ordering::Acquire);
        if let Some(s) = cell.write(gen, |s| {
            f(s);
            s.held_back = held;
        }) {
            (self.emit)(&s);
        }
        true
    }

    fn current_gen(&self) -> u64 {
        lock(&self.status).gen()
    }

    fn enabled(&self) -> bool {
        self.last().is_some_and(|c| c.enabled)
    }
}

impl SlotStore for Shared {
    fn after_scene(&self, timer: &str, cur: SceneRef, to: SceneRef) -> bool {
        let wall = wall_ms();
        self.with_slots(true, |s| s.after_scene(timer, cur, to, wall))
    }

    fn plan_back(&self, timer: &str, cur_uuid: &str) -> BackPlan {
        lock(&self.slots).slots.plan_back(timer, cur_uuid)
    }

    fn commit_back(&self, timer: &str, plan: BackPlan) {
        self.with_slots(true, |s| s.commit_back(timer, plan));
    }

    fn mark_due(&self, timer: &str, at: Instant) {
        self.with_slots(false, |s| s.mark_due(timer, at));
    }

    fn take_due(&self, now: Instant) -> (Option<String>, Vec<String>) {
        let grace = self.grace;
        self.with_slots(false, |s| s.take_due(now, grace))
    }

    fn expire_due(&self, now: Instant) -> Vec<String> {
        let grace = self.grace;
        self.with_slots(false, |s| s.expire_due(now, grace))
    }

    fn next_expiry(&self) -> Option<Instant> {
        lock(&self.slots).slots.next_expiry(self.grace)
    }

    fn written(&self) -> HashSet<String> {
        lock(&self.written).clone()
    }

    fn note_written(&self, uuid: &str, text_empty: bool) {
        let mut written = lock(&self.written);
        if text_empty {
            written.remove(uuid);
        } else {
            written.insert(uuid.to_string());
        }
    }
}

/// D12: the stored password, which a link decrypts (on the blocking pool)
/// only when OBS's Hello asks for one.
struct StoredPassword {
    dir: PathBuf,
    cipher: Arc<dyn Cipher>,
}

impl PasswordSource for StoredPassword {
    fn load(&self) -> Result<Option<Zeroizing<Vec<u8>>>, Unreadable> {
        files::load_password(&self.dir, &*self.cipher)
    }
}

/// §7.5, D13: the states a link stays in until something changes: an
/// address, a password, or Try again.
fn is_stop(state: ObsState) -> bool {
    matches!(
        state,
        ObsState::NeedsPassword
            | ObsState::AuthFailed
            | ObsState::PasswordUnreadable
            | ObsState::TooOld
            | ObsState::NeedsNewerDivora
            | ObsState::Kicked
            | ObsState::Unsupported
    )
}

/// The wall clock in Unix ms: a skip's `atMs`, a slot's `setAtMs`.
fn wall_ms() -> i64 {
    match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(after) => i64::try_from(after.as_millis()).unwrap_or(i64::MAX),
        Err(e) => i64::try_from(e.duration().as_millis()).map_or(i64::MIN, |ms| -ms),
    }
}

/// Where [`ObsShared::offer`] got an op to.
enum Handed {
    /// Into the running link's queue.
    Queued,
    /// The queue was full.
    Full,
    /// No link runs (or its driver had ended).
    NoLink,
}

/// D3: the OBS link's state, managed by the app once `init` has built it,
/// separately from `AppState` (it must exist before the timer thread, which
/// `AppState` comes after). The commands, the timer worker's
/// [`ObsShared::offer`] and every link generation reach it. No lock is held
/// across an `.await`, and every lock is a leaf but `link`'s: a start or a
/// stop holds it while it takes the status cell's (D4: the generation,
/// `connecting`, then the spawn), so two can't interleave; nothing takes
/// `link`'s while holding another.
pub struct ObsShared {
    shared: Arc<Shared>,
    /// The running link's ends; `None` while none runs.
    link: Mutex<Option<LinkHandle>>,
    feed: watch::Receiver<Arc<ObsFeed>>,
    timers: Arc<dyn TimerControl>,
    cipher: Arc<dyn Cipher>,
    /// The `obs/` folder (`obs-dev/` for `tauri dev`).
    dir: PathBuf,
    spawner: Handle,
    timing: LinkTiming,
}

impl ObsShared {
    /// D3: off, no link, no config, with the slots `dir`'s `returns.json`
    /// holds (D24) and `hasPassword` from `password.bin`, both read here,
    /// once (small, on the main thread at setup). Makes no folder.
    /// `spawner` is Tauri's runtime in the app and the test's own in tests.
    #[must_use]
    pub fn new(
        emit: Emit,
        spawner: Handle,
        cipher: Arc<dyn Cipher>,
        dir: PathBuf,
        timers: Arc<dyn TimerControl>,
        feed: watch::Receiver<Arc<ObsFeed>>,
        timing: LinkTiming,
    ) -> Self {
        let slots = files::load_returns(&dir, wall_ms());
        let has_password = files::has_password(&dir);
        let mut cell = StatusCell::new();
        // Before anything listens: not emitted.
        let _ = cell.write_shared(|s| s.has_password = has_password);
        Self {
            shared: Arc::new(Shared {
                status: Mutex::new(cell),
                emit,
                config: Mutex::new(None),
                held: AtomicU32::new(slots.due_count()),
                slots: Mutex::new(SlotState { slots, changes: 0 }),
                written: Mutex::new(HashSet::new()),
                saver: SlotSaver::new(spawner.clone(), dir.clone()),
                grace: timing.scene_back_grace,
            }),
            link: Mutex::new(None),
            feed,
            timers,
            cipher,
            dir,
            spawner,
            timing,
        }
    }

    /// `obs_status`: the status now.
    #[must_use]
    pub fn status(&self) -> ObsStatus {
        lock(&self.shared.status).status().clone()
    }

    /// `obs_configure` (D4): validates the address (`Err`: its message);
    /// `enabled:false` stops any link and drops every due mark, and never
    /// connects; `enabled:true` starts a link, or keeps the one running for
    /// the same canonical address (a reload changes nothing, a Kick
    /// included), or restarts it for another. Rust keeps the config in
    /// memory only (D16).
    #[allow(clippy::needless_pass_by_value)] // the command's own value
    pub fn configure(&self, cfg: ObsConfig) -> Result<ObsStatus, String> {
        let ip = parse_host(&cfg.host).map_err(|e| e.to_string())?;
        if cfg.port == 0 {
            return Err(BAD_PORT.to_string());
        }
        let at = LinkConfig { ip, port: cfg.port };
        let enabled = cfg.enabled && OBS_LINK;
        let mut link = lock(&self.link);
        *lock(&self.shared.config) = Some(LastConfig { enabled, at });
        if enabled {
            self.start(&mut link, at, false);
        } else {
            self.shared.with_slots(false, Slots::drop_due);
            self.turn_off(&mut link, at);
        }
        drop(link);
        Ok(self.status())
    }

    /// `obs_reconnect` (D4): retrying, an attempt now (the failure count
    /// kept); a stop state, a restart (a new generation); a link whose
    /// driver had ended, a new one; otherwise, or while off, nothing.
    #[must_use]
    pub fn reconnect(&self) -> ObsStatus {
        let mut link = lock(&self.link);
        if let Some(last) = self.shared.last().filter(|c| c.enabled) {
            let state = self.status().state;
            match link.as_ref() {
                Some(h) if !h.cmds.is_closed() => {
                    if state == ObsState::Retrying {
                        let _ = h.cmds.try_send(LinkCmd::Wake);
                    } else if is_stop(state) {
                        let at = h.at;
                        self.start(&mut link, at, true);
                    }
                }
                _ => self.start(&mut link, last.at, true),
            }
        }
        drop(link);
        self.status()
    }

    /// `obs_set_password` (D28): 1 to [`MAX_PASSWORD_CHARS`] characters,
    /// encrypted and written to `password.bin` on the blocking pool (Divora's
    /// copy wiped there once written), then a link that is on restarts — a
    /// stopped one included — so it uses it. Works while off. Never echoes
    /// the password.
    pub async fn set_password(&self, password: Zeroizing<String>) -> Result<ObsStatus, String> {
        let chars = password.chars().count();
        if chars == 0 {
            return Err(NO_PASSWORD.to_string());
        }
        if chars > MAX_PASSWORD_CHARS {
            return Err(LONG_PASSWORD.to_string());
        }
        let dir = self.dir.clone();
        let cipher = Arc::clone(&self.cipher);
        let saved = blocking(&self.spawner, move || {
            files::save_password(&dir, &*cipher, &password)
        })
        .await;
        match saved {
            Ok(Ok(())) => {}
            Ok(Err(why)) => return Err(why.message().to_string()),
            Err(_) => return Err(WRITE_FAILED.to_string()),
        }
        self.shared.write_shared(|s| s.has_password = true);
        self.restart_if_on();
        Ok(self.status())
    }

    /// `obs_forget_password`: deletes `password.bin` (none there is fine) on
    /// the blocking pool, and a link that is on restarts. Works while off.
    /// `hasPassword` says what is on the disk after: a file that couldn't be
    /// deleted still reads as saved (and is logged).
    pub async fn forget_password(&self) -> Result<ObsStatus, String> {
        let dir = self.dir.clone();
        let has = blocking(&self.spawner, move || {
            let _ = files::forget_password(&dir);
            files::has_password(&dir)
        })
        .await
        .unwrap_or(true);
        self.shared.write_shared(|s| s.has_password = has);
        self.restart_if_on();
        Ok(self.status())
    }

    /// `obs_inventory` (D19): the scenes and text sources, from a connected
    /// link, within [`INVENTORY_WAIT`]; a second call while one runs joins it
    /// (the link's). Holds no sender while it waits, so a stop isn't held up.
    pub async fn inventory(&self) -> Result<ObsInventory, String> {
        let (reply, answer) = oneshot::channel();
        let sent = {
            let link = lock(&self.link);
            match link.as_ref() {
                Some(h) if self.status().state == ObsState::Connected => h
                    .cmds
                    .try_send(LinkCmd::Inventory(reply))
                    .map_err(|e| matches!(e, mpsc::error::TrySendError::Full(_))),
                _ => Err(false),
            }
        };
        match sent {
            Ok(()) => {}
            Err(true) => return Err(BUSY.to_string()),
            Err(false) => return Err(NOT_CONNECTED.to_string()),
        }
        match tokio::time::timeout(INVENTORY_WAIT, answer).await {
            Ok(Ok(answer)) => answer,
            // The link stopped (or reconnected) before it answered.
            Ok(Err(_)) => Err(NOT_CONNECTED.to_string()),
            Err(_) => Err(NO_ANSWER.to_string()),
        }
    }

    /// D17: `timer`'s `obs` action, from the timer worker. Never waits:
    /// stamps it now, then, while the status is `connected`, hands it to the
    /// link (`try_send`). Otherwise — or with the queue full, or the link's
    /// driver gone — a `scene` is a skip (`queueFull`, or `notConnected`
    /// when the link is switched on; a log line either way: it is never run
    /// late, D23), and a `sceneBack` is due in the slots (D23), with its
    /// stamp: always on a full queue (the `scene` ahead of it may make its
    /// slot); otherwise only if its timer has a slot (its `scene` ran) and
    /// OBS isn't switched off (`off` drops due marks; before the first
    /// configure — a launch — it is kept). So a switch back is never
    /// `queueFull` or `late`.
    pub fn offer(&self, timer: &str, op: &ObsOp) {
        if !OBS_LINK {
            return;
        }
        let at = Instant::now();
        let connected = lock(&self.shared.status).status().state == ObsState::Connected;
        let handed = if connected {
            self.hand_over(LinkOp {
                at,
                timer: timer.to_string(),
                op: op.clone(),
            })
        } else {
            Handed::NoLink
        };
        let full = match handed {
            Handed::Queued => return,
            Handed::Full => true,
            Handed::NoLink => false,
        };
        match op {
            ObsOp::Scene { scene_name, .. } => {
                let why = if full {
                    SkipWhy::QueueFull
                } else {
                    SkipWhy::NotConnected
                };
                log::op_dropped(timer, op.op(), why);
                if full || self.shared.last().is_some_and(|c| c.enabled) {
                    let at_ms = wall_ms();
                    self.shared.write_shared(|s| {
                        s.last_skip = Some(LastSkip {
                            op: SkipOp::Scene,
                            why,
                            name: Some(scene_name.clone()),
                            timer: timer.to_string(),
                            at_ms,
                        });
                    });
                }
            }
            ObsOp::SceneBack => {
                let on = self.shared.last().is_none_or(|c| c.enabled);
                if full || (on && self.shared.has_slot(timer)) {
                    self.shared.mark_due(timer, at);
                }
            }
        }
    }

    /// D5: the tray's Quit. With no link, returns at once; otherwise stops
    /// it and waits up to [`EXIT_WAIT`] for it to close its connection.
    pub fn before_exit(&self) {
        let Some(link) = lock(&self.link).take() else {
            return;
        };
        log::link_stopped(link.gen, "quit");
        let LinkHandle {
            ops, cmds, closed, ..
        } = link;
        drop(ops);
        drop(cmds);
        let _ = closed.recv_timeout(EXIT_WAIT);
    }

    /// `op` into the running link's queue, if one runs. A queue whose
    /// driver had ended (it can't take anything) drops the handle, so the
    /// next change starts a new link.
    fn hand_over(&self, op: LinkOp) -> Handed {
        let mut link = lock(&self.link);
        let Some(h) = link.as_ref() else {
            return Handed::NoLink;
        };
        match h.ops.try_send(op) {
            Ok(()) => Handed::Queued,
            Err(mpsc::error::TrySendError::Full(_)) => Handed::Full,
            Err(mpsc::error::TrySendError::Closed(_)) => {
                log::link_gone(h.gen);
                *link = None;
                Handed::NoLink
            }
        }
    }

    /// D4, under `link`'s lock: a link for `at`. One already running for
    /// `at`, whose driver hasn't ended, stays, unless `restart`. A start:
    /// under the status cell's lock, the generation + 1 and `connecting`
    /// (emitted), then the driver spawned with that generation; then the old
    /// handle is dropped, so the old driver, already stale, sees its
    /// commands close and stops. The handle keeps only the new ends.
    fn start(&self, link: &mut Option<LinkHandle>, at: LinkConfig, restart: bool) {
        let running = link
            .as_ref()
            .is_some_and(|h| h.at == at && !h.cmds.is_closed());
        if running && !restart {
            return;
        }
        let (ops_tx, ops) = mpsc::channel(OPS_QUEUE);
        let (cmds_tx, cmds) = mpsc::channel(CMDS_QUEUE);
        let (closed_ack, closed) = std::sync::mpsc::sync_channel(1);
        let gen = {
            let mut cell = lock(&self.shared.status);
            let (gen, began) = cell.begin(&at);
            if let Some(s) = began {
                (self.shared.emit)(&s);
            }
            let status: Arc<dyn StatusSink> = self.shared.clone();
            let slots: Arc<dyn SlotStore> = self.shared.clone();
            drop(self.spawner.spawn(link::run(LinkDeps {
                cfg: at,
                gen,
                status,
                slots,
                timers: Arc::clone(&self.timers),
                password: Arc::new(StoredPassword {
                    dir: self.dir.clone(),
                    cipher: Arc::clone(&self.cipher),
                }),
                feed: self.feed.clone(),
                ops,
                cmds,
                timing: self.timing,
                rand: fastrand::f64,
                spawner: self.spawner.clone(),
                closed_ack,
            })));
            gen
        };
        let old = link.replace(LinkHandle {
            gen,
            at,
            ops: ops_tx,
            cmds: cmds_tx,
            closed,
        });
        if let Some(old) = old {
            log::link_stopped(old.gen, "restart");
        }
        log::link_started(gen, at.loopback());
    }

    /// D4, under `link`'s lock: `off` at `at`. A running link's generation
    /// is retired first, under the status cell's lock (it moves as a start's
    /// would), so its driver, on its way out, can't write over `off`; then
    /// its handle is dropped and the driver stops. The status keeps
    /// `hasPassword` and `inventoryRev`, and the address it was given.
    fn turn_off(&self, link: &mut Option<LinkHandle>, at: LinkConfig) {
        {
            let mut cell = lock(&self.shared.status);
            if let Some(h) = link.as_ref() {
                let _ = cell.begin(&h.at);
            }
            let held = self.shared.held.load(Ordering::Acquire);
            if let Some(s) = cell.write_shared(|s| {
                *s = ObsStatus {
                    host: at.ip.to_string(),
                    port: at.port,
                    loopback: at.loopback(),
                    has_password: s.has_password,
                    held_back: held,
                    inventory_rev: s.inventory_rev,
                    ..ObsStatus::off()
                };
            }) {
                (self.shared.emit)(&s);
            }
        }
        if let Some(old) = link.take() {
            log::link_stopped(old.gen, "off");
        }
    }

    /// D4: a saved or forgotten password restarts the link while it is on,
    /// whatever its state: it may need the password, or not any more.
    fn restart_if_on(&self) {
        let mut link = lock(&self.link);
        if let Some(last) = self.shared.last().filter(|c| c.enabled) {
            self.start(&mut link, last.at, true);
        }
    }
}

/// D3: at setup, after `spawn_timer_thread` and before `AppState` is
/// managed: builds [`ObsShared`] on Tauri's runtime — its slots read from
/// `dir`, DPAPI for the password, the production timings — and manages it.
/// Nothing starts: no link runs and no folder is made until the page
/// switches OBS on.
pub fn init(
    app: &AppHandle,
    feed: watch::Receiver<Arc<ObsFeed>>,
    timers: Arc<TimerShared>,
    dir: PathBuf,
) {
    let emitter = app.clone();
    let emit: Emit = Box::new(move |s: &ObsStatus| {
        let _ = emitter.emit(OBS_STATUS_EVENT, s);
    });
    let spawner = tauri::async_runtime::handle().inner().clone();
    app.manage(ObsShared::new(
        emit,
        spawner,
        Arc::new(files::Dpapi),
        dir,
        timers,
        feed,
        LinkTiming::production(),
    ));
}

/// D5: the tray's Quit, after the overlay's: [`ObsShared::before_exit`],
/// if it is managed.
pub fn before_exit(app: &AppHandle) {
    if let Some(obs) = app.try_state::<ObsShared>() {
        obs.before_exit();
    }
}

/// D6: the only way blocking work under `obs/` runs (the password, the slot
/// saver): on `spawner`'s blocking pool, under the caller's `tracing`
/// dispatcher, so a line it logs reaches the same subscriber (a test's
/// capture included).
pub fn blocking<F, R>(spawner: &Handle, f: F) -> JoinHandle<R>
where
    F: FnOnce() -> R + Send + 'static,
    R: Send + 'static,
{
    let dispatch = tracing::dispatcher::get_default(tracing::Dispatch::clone);
    spawner.spawn_blocking(move || tracing::dispatcher::with_default(&dispatch, f))
}

/// A leaf lock, poisoned or not: every value under one is whole between
/// statements.
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
        blocking, obs_dir_name, LinkPeriods, LinkTiming, EVENTS_QUEUE, FRAME_CAP_BYTES,
        HANDSHAKE_CAP_BYTES, LATE_ANSWER_CAP, MAX_PASSWORD_CHARS, MAX_SCENES, MAX_SCENE_NAME_CHARS,
        MAX_TEXT_SOURCES, MAX_UUID_CHARS, MISSING_RECHECK, NOT_READY_CAP, NOT_READY_WAIT,
        OBS_STATUS_EVENT, OPS_QUEUE, PRODUCTION, REV_COALESCE, SCENE_BACK_GRACE, SCENE_MAX_AGE,
    };
    use serde_json::{json, Value};
    use std::time::Duration;

    fn fixture() -> Value {
        serde_json::from_str(include_str!("../../../src/data/obs.fixture.json")).unwrap()
    }

    fn ms(d: Duration) -> u64 {
        u64::try_from(d.as_millis()).unwrap()
    }

    /// §4: the fixture's `limits` (the page and the e2e mock read the same)
    /// are this module's, value for value, and no key is left unread.
    #[test]
    fn limits_are_the_fixtures() {
        let f = fixture();
        let ours = json!({
            "opsQueue": OPS_QUEUE,
            "eventsQueue": EVENTS_QUEUE,
            "sceneBackGraceMs": ms(SCENE_BACK_GRACE),
            "sceneMaxAgeMs": ms(SCENE_MAX_AGE),
            "lateAnswerCapMs": ms(LATE_ANSWER_CAP),
            "notReadyWaitMs": ms(NOT_READY_WAIT),
            "notReadyCapMs": ms(NOT_READY_CAP),
            "revCoalesceMs": ms(REV_COALESCE),
            "missingRecheckMs": ms(MISSING_RECHECK),
            "frameCapBytes": FRAME_CAP_BYTES,
            "handshakeCapBytes": HANDSHAKE_CAP_BYTES,
            "maxTextSources": MAX_TEXT_SOURCES,
            "maxScenes": MAX_SCENES,
            "maxPasswordChars": MAX_PASSWORD_CHARS,
            "maxSceneNameChars": MAX_SCENE_NAME_CHARS,
            "maxUuidChars": MAX_UUID_CHARS,
        });
        assert_eq!(f["limits"], ours);
        assert_eq!(f["obsWire"]["events"]["status"], OBS_STATUS_EVENT);
    }

    /// D8, D39: a zero period is refused by name; the production timings
    /// and §10.2's scaled ones pass, the latter at §10.2's numbers.
    #[test]
    fn link_timing_refuses_zero() {
        assert_eq!(LinkTiming::new(PRODUCTION), Ok(LinkTiming::production()));
        let t = LinkTiming::production();
        assert_eq!(t.connect, Duration::from_secs(3));
        assert_eq!(t.late_answer_cap, Duration::from_secs(15));
        let zero = |change: fn(&mut LinkPeriods)| {
            let mut p = PRODUCTION;
            change(&mut p);
            LinkTiming::new(p)
        };
        assert_eq!(zero(|p| p.ping = Duration::ZERO), Err("ping"));
        assert_eq!(zero(|p| p.connect = Duration::ZERO), Err("connect"));
        assert_eq!(zero(|p| p.close_echo = Duration::ZERO), Err("close_echo"));
        assert_eq!(
            zero(|p| p.keepalive_deadline = Some(Duration::ZERO)),
            Err("keepalive_deadline")
        );
        // The kill switches' values pass.
        assert!(zero(|p| p.keepalive_deadline = None).is_ok());
        assert!(zero(|p| p.scene_back_grace = Duration::ZERO).is_ok());
        assert_eq!(LinkTiming::scaled(Duration::ZERO), Err("connect"));

        let s = LinkTiming::scaled(Duration::from_millis(100)).unwrap();
        let ms = Duration::from_millis;
        assert_eq!(
            [
                s.connect,
                s.request_loopback,
                s.request_other,
                s.late_answer_cap,
                s.ping,
                s.keepalive_deadline.unwrap(),
                s.backoff_unit,
                s.reset_after,
                s.scene_back_grace,
                s.scene_max_age,
                s.not_ready_wait,
                s.not_ready_cap,
                s.push_retry_base,
                s.missing_recheck,
                s.rev_coalesce,
                s.close_flush,
                s.close_echo,
            ],
            [
                ms(300),
                ms(200),
                ms(200),
                ms(1_000),
                ms(200),
                ms(200),
                ms(200),
                ms(400),
                ms(2_000),
                ms(500),
                ms(100),
                ms(1_000),
                ms(100),
                ms(1_000),
                ms(100),
                ms(200),
                ms(100),
            ]
        );
    }

    /// D28, K14: `tauri dev` keeps its own folder.
    #[test]
    fn obs_dir_names() {
        assert_eq!(obs_dir_name(false), "obs");
        assert_eq!(obs_dir_name(true), "obs-dev");
    }

    /// A layer only this test's dispatcher has.
    struct Marker;

    impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for Marker {}

    /// D6: a blocking job runs under the dispatcher of the code that started
    /// it, so a test's capture sees its log lines; a job started outside it
    /// doesn't.
    #[test]
    fn blocking_carries_the_dispatcher() {
        use tracing_subscriber::layer::SubscriberExt;
        let rt = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let marked = || tracing::dispatcher::get_default(tracing::Dispatch::is::<Marker>);
        let dispatch = tracing::Dispatch::new(tracing_subscriber::registry().with(Marker));
        let job = tracing::dispatcher::with_default(&dispatch, || blocking(rt.handle(), marked));
        assert!(rt.block_on(job).unwrap());
        let job = blocking(rt.handle(), marked);
        assert!(!rt.block_on(job).unwrap());
    }
}

/// v1.61.0 (track C, §10.1, §10.3): `ObsShared` on its own — the commands'
/// bodies and `offer` — with a link handle the test holds the other ends of,
/// so nothing here depends on a connection: no driver ever sees these ends.
/// Where a test lets `configure` start a link, its address is a listener the
/// test holds and never accepts on (never OBS's 4455).
#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic
)]
mod shared_tests {
    use super::files::{Cipher, PASSWORD_FILE};
    use super::machine::{HostError, LastSkip, ObsConfig, ObsState, ObsStatus, SkipOp, SkipWhy};
    use super::scenes::SceneRef;
    use super::testing::{assert_none_in, scratch, secret_forms, traced, MemCipher};
    use super::{
        lock, Emit, LastConfig, LinkCmd, LinkConfig, LinkHandle, LinkOp, LinkTiming, ObsOp,
        ObsShared, SlotStore, StatusSink, TimerControl, BAD_PORT, BUSY, CMDS_QUEUE, ENCRYPT_FAILED,
        LONG_PASSWORD, MAX_PASSWORD_CHARS, NOT_CONNECTED, NO_ANSWER, NO_PASSWORD, OPS_QUEUE,
        REFUSED, WRITE_FAILED,
    };
    use crate::timers::TimerOp;
    use serde_json::Value;
    use std::net::{IpAddr, Ipv4Addr, TcpListener};
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};
    use tokio::sync::{mpsc, watch};
    use zeroize::Zeroizing;

    const PASSWORD: &str = "correct horse \u{2713} 7Qz";

    struct NoTimers;

    impl TimerControl for NoTimers {
        fn apply(&self, _id: &str, _op: TimerOp) -> bool {
            false
        }
    }

    fn runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
    }

    /// Every status `obs-status` carried.
    type Seen = Arc<Mutex<Vec<ObsStatus>>>;

    fn shared_with(
        rt: &tokio::runtime::Runtime,
        dir: PathBuf,
        cipher: MemCipher,
    ) -> (ObsShared, Seen) {
        let seen: Seen = Arc::default();
        let into = seen.clone();
        let emit: Emit = Box::new(move |s| into.lock().unwrap().push(s.clone()));
        let (_feed_tx, feed) = watch::channel(Arc::default());
        let obs = ObsShared::new(
            emit,
            rt.handle().clone(),
            Arc::new(cipher),
            dir,
            Arc::new(NoTimers),
            feed,
            LinkTiming::scaled(Duration::from_millis(100)).unwrap(),
        );
        (obs, seen)
    }

    fn shared(rt: &tokio::runtime::Runtime, name: &str) -> (ObsShared, Seen, PathBuf) {
        let dir = scratch(name);
        let (obs, seen) = shared_with(rt, dir.clone(), MemCipher::default());
        (obs, seen, dir)
    }

    /// An address nothing will answer on: a listener this test holds and
    /// never accepts from. Never OBS's.
    fn black_hole() -> (TcpListener, u16) {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        listener.set_nonblocking(true).unwrap();
        let port = listener.local_addr().unwrap().port();
        (listener, port)
    }

    /// Whether anything connected to `listener`.
    fn dialed(listener: &TcpListener) -> bool {
        listener.accept().is_ok()
    }

    fn config(enabled: bool, host: &str, port: u16) -> ObsConfig {
        ObsConfig {
            enabled,
            host: host.into(),
            port,
        }
    }

    fn at(port: u16) -> LinkConfig {
        LinkConfig {
            ip: IpAddr::V4(Ipv4Addr::LOCALHOST),
            port,
        }
    }

    /// The other ends of a link handle `obs` holds as its running link, for
    /// the last config `enabled` at `port`: no driver runs.
    struct Stub {
        gen: u64,
        ops: mpsc::Receiver<LinkOp>,
        cmds: mpsc::Receiver<LinkCmd>,
        _ack: std::sync::mpsc::SyncSender<()>,
    }

    fn stub_link(obs: &ObsShared, enabled: bool, port: u16) -> Stub {
        *lock(&obs.shared.config) = Some(LastConfig {
            enabled,
            at: at(port),
        });
        let (ops_tx, ops) = mpsc::channel(OPS_QUEUE);
        let (cmds_tx, cmds) = mpsc::channel(CMDS_QUEUE);
        let (ack, closed) = std::sync::mpsc::sync_channel(1);
        let (gen, _) = lock(&obs.shared.status).begin(&at(port));
        *lock(&obs.link) = Some(LinkHandle {
            gen,
            at: at(port),
            ops: ops_tx,
            cmds: cmds_tx,
            closed,
        });
        Stub {
            gen,
            ops,
            cmds,
            _ack: ack,
        }
    }

    /// The stub's driver says it is connected.
    fn connected(obs: &ObsShared, stub: &Stub) {
        assert!(obs
            .shared
            .write(stub.gen, &mut |s| s.state = ObsState::Connected));
    }

    fn scene(name: &str) -> SceneRef {
        SceneRef {
            uuid: format!("5d3f1c9e-{name}"),
            name: name.into(),
        }
    }

    fn brb() -> ObsOp {
        ObsOp::Scene {
            scene_uuid: scene("BRB").uuid,
            scene_name: "BRB".into(),
        }
    }

    /// `timer`'s break switched Game to BRB: it has a slot.
    fn give_slot(obs: &ObsShared, timer: &str) {
        assert!(obs.shared.after_scene(timer, scene("Game"), scene("BRB")));
    }

    /// The scene skip now, without its time.
    fn skip(obs: &ObsShared) -> Option<(SkipOp, SkipWhy, Option<String>, String)> {
        obs.status().last_skip.map(
            |LastSkip {
                 op,
                 why,
                 name,
                 timer,
                 ..
             }| (op, why, name, timer),
        )
    }

    fn queued(stub: &mut Stub) -> Vec<(String, ObsOp)> {
        let mut out = Vec::new();
        while let Ok(op) = stub.ops.try_recv() {
            out.push((op.timer, op.op));
        }
        out
    }

    /// D17, §5.6: `offer` in every state, for both ops — handed to a
    /// connected link, a skip, a due mark, or nothing — and never waits. A
    /// switch back is never `queueFull` or `late` (the page's reader drops a
    /// status that says so): a full queue makes it due. (`OBS_LINK` off, the
    /// one row not here, is a constant: `offer` returns first thing.)
    #[test]
    #[allow(clippy::too_many_lines)] // one table, a row per state
    fn offer_table() {
        let rt = runtime();
        let back = ObsOp::SceneBack;

        // Before the first configure (a launch): no skip line, no link; a
        // switch back whose timer has a slot is due, one without is nothing.
        let (obs, _seen, dir) = shared(&rt, "offer-unknown");
        give_slot(&obs, "timer-1");
        obs.offer("timer-2", &brb());
        assert_eq!(skip(&obs), None, "no skip while the config is unknown");
        obs.offer("timer-1", &back);
        obs.offer("timer-3", &back);
        assert_eq!(obs.status().held_back, 1);
        assert_eq!(
            obs.shared.take_due(Instant::now()).0.as_deref(),
            Some("timer-1")
        );
        assert_eq!(
            obs.shared.take_due(Instant::now()),
            (None, vec![]),
            "no mark for timer-3"
        );
        std::fs::remove_dir_all(&dir).ok();

        // On, not connected (connecting, retrying, a stop state): a scene is
        // a `notConnected` skip, a switch back with a slot is due, none goes
        // to the link.
        for state in [
            ObsState::Connecting,
            ObsState::Retrying,
            ObsState::NeedsPassword,
        ] {
            let (obs, _seen, dir) = shared(&rt, "offer-not-connected");
            let mut stub = stub_link(&obs, true, 4460);
            assert!(obs.shared.write(stub.gen, &mut |s| s.state = state));
            give_slot(&obs, "timer-1");
            obs.offer("timer-2", &brb());
            assert_eq!(
                skip(&obs),
                Some((
                    SkipOp::Scene,
                    SkipWhy::NotConnected,
                    Some("BRB".into()),
                    "timer-2".into()
                )),
                "{state:?}"
            );
            obs.offer("timer-1", &back);
            obs.offer("timer-3", &back);
            assert_eq!(obs.status().held_back, 1, "{state:?}");
            assert_eq!(
                queued(&mut stub),
                vec![],
                "{state:?}: a parked driver gets nothing"
            );
            std::fs::remove_dir_all(&dir).ok();
        }

        // Connected: both go to the link, stamped, in order, a switch back
        // whether its timer has a slot or not (the `scene` ahead may make
        // one); nothing else changes.
        let (obs, seen, dir) = shared(&rt, "offer-connected");
        let mut stub = stub_link(&obs, true, 4460);
        connected(&obs, &stub);
        let emits = seen.lock().unwrap().len();
        let before = Instant::now();
        obs.offer("timer-1", &brb());
        obs.offer("timer-1", &back);
        obs.offer("timer-2", &back);
        let got: Vec<LinkOp> = std::iter::from_fn(|| stub.ops.try_recv().ok()).collect();
        assert_eq!(
            got.iter()
                .map(|o| (o.timer.as_str(), o.op.clone()))
                .collect::<Vec<_>>(),
            vec![
                ("timer-1", brb()),
                ("timer-1", back.clone()),
                ("timer-2", back.clone())
            ]
        );
        assert!(got.iter().all(|o| o.at >= before && o.at <= Instant::now()));
        assert_eq!(seen.lock().unwrap().len(), emits, "nothing to say");
        assert_eq!((skip(&obs), obs.status().held_back), (None, 0));

        // The queue full: a scene is `queueFull`; a switch back is due, even
        // with no slot yet, and never `queueFull`.
        for n in 0..OPS_QUEUE {
            obs.offer(&format!("timer-{n}"), &brb());
        }
        obs.offer("timer-20", &brb());
        assert_eq!(
            skip(&obs),
            Some((
                SkipOp::Scene,
                SkipWhy::QueueFull,
                Some("BRB".into()),
                "timer-20".into()
            ))
        );
        obs.offer("timer-21", &back);
        assert_eq!(
            skip(&obs).unwrap().3,
            "timer-20",
            "no skip for a switch back"
        );
        assert!(obs.shared.next_expiry().is_some(), "due without a slot");
        assert_eq!(
            obs.status().held_back,
            0,
            "a mark without a slot isn't held back"
        );
        give_slot(&obs, "timer-21");
        assert_eq!(obs.status().held_back, 1, "its scene made one");
        assert_eq!(queued(&mut stub).len(), OPS_QUEUE);

        // The driver gone (its queue closed): the handle is dropped, and the
        // op is handled as not connected.
        drop(stub);
        obs.offer("timer-22", &brb());
        assert!(lock(&obs.link).is_none(), "a dead link's handle goes");
        assert_eq!(skip(&obs).unwrap().1, SkipWhy::NotConnected);
        std::fs::remove_dir_all(&dir).ok();

        // Off: no skip, and a switch back is nothing, slot or not.
        let (obs, _seen, dir) = shared(&rt, "offer-off");
        give_slot(&obs, "timer-1");
        obs.configure(config(false, "127.0.0.1", 4460)).unwrap();
        obs.offer("timer-1", &brb());
        obs.offer("timer-1", &back);
        assert_eq!((skip(&obs), obs.status().held_back), (None, 0));
        assert_eq!(obs.shared.next_expiry(), None);
        std::fs::remove_dir_all(&dir).ok();
    }

    /// D17: `offer` never waits, whatever the link does: 2 000 offers from
    /// a plain thread against a full queue that nothing drains.
    #[test]
    fn offer_never_waits() {
        let rt = runtime();
        let (obs, _seen, dir) = shared(&rt, "offer-fast");
        let stub = stub_link(&obs, true, 4460);
        connected(&obs, &stub);
        let started = Instant::now();
        std::thread::scope(|s| {
            s.spawn(|| {
                for n in 0..1_000 {
                    obs.offer(&format!("timer-{n}"), &brb());
                    obs.offer(&format!("timer-{n}"), &ObsOp::SceneBack);
                }
            });
        });
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "{:?}",
            started.elapsed()
        );
        drop(stub);
        std::fs::remove_dir_all(&dir).ok();
    }

    /// D4: `obs_configure {enabled:false}` drops every due mark (the slots
    /// stay, so a later break's switch back still works), stops a running
    /// link — its generation moves first, so its driver can't write over
    /// `off` — and answers `off` at the canonical address.
    #[test]
    fn configure_off_drops_due_marks() {
        let rt = runtime();
        let (obs, seen, dir) = shared(&rt, "configure-off");
        give_slot(&obs, "timer-1");
        obs.offer("timer-1", &ObsOp::SceneBack);
        assert_eq!(obs.status().held_back, 1);
        let mut stub = stub_link(&obs, true, 4460);
        connected(&obs, &stub);
        let off = obs.configure(config(false, " localhost ", 4460)).unwrap();
        assert_eq!(
            (
                off.state,
                off.held_back,
                off.host.as_str(),
                off.port,
                off.loopback
            ),
            (ObsState::Off, 0, "127.0.0.1", 4460, true)
        );
        assert_eq!(obs.shared.next_expiry(), None);
        assert!(lock(&obs.link).is_none());
        assert!(matches!(
            stub.cmds.try_recv(),
            Err(mpsc::error::TryRecvError::Disconnected)
        ));
        assert!(
            !obs.shared
                .write(stub.gen, &mut |s| s.state = ObsState::Connected),
            "stale"
        );
        assert_eq!(obs.status().state, ObsState::Off);
        assert_eq!(seen.lock().unwrap().last(), Some(&off), "off was said");
        // The slot stays.
        assert_eq!(
            obs.shared.plan_back("timer-1", &scene("BRB").uuid).to,
            Some(scene("Game"))
        );
        // Off again changes nothing, and says nothing.
        let said = seen.lock().unwrap().len();
        assert_eq!(
            obs.configure(config(false, "127.0.0.1", 4460)).unwrap(),
            off
        );
        assert_eq!(seen.lock().unwrap().len(), said);
        std::fs::remove_dir_all(&dir).ok();
    }

    /// D4, D16, D30: while off, every command works and nothing connects —
    /// configure (any address it takes), offer, Try again, the inventory, a
    /// saved and a forgotten password — and nothing makes a folder but a
    /// saved password. An address Rust refuses is answered with its message
    /// (the fixture's), and changes nothing.
    #[test]
    fn configure_while_off_validates_and_connects_nothing() {
        let rt = runtime();
        let (listener, port) = black_hole();
        let (obs, _seen, dir) = shared(&rt, "configure-while-off");
        for (host, canonical) in [
            ("127.0.0.1", "127.0.0.1"),
            ("localhost", "127.0.0.1"),
            ("::ffff:127.0.0.1", "127.0.0.1"),
            ("[::1]", "::1"),
        ] {
            let s = obs.configure(config(false, host, port)).unwrap();
            assert_eq!(
                (s.state, s.host.as_str(), s.port),
                (ObsState::Off, canonical, port),
                "{host}"
            );
        }
        let before = obs.status();
        for (host, port, error) in [
            ("obs.local", port, HostError::NotAnIp.to_string()),
            ("http://127.0.0.1", port, HostError::NotAnIp.to_string()),
            ("fe80::1", port, HostError::LinkLocalV6.to_string()),
            ("127.0.0.1", 0, BAD_PORT.to_string()),
        ] {
            for enabled in [false, true] {
                assert_eq!(
                    obs.configure(config(enabled, host, port)),
                    Err(error.clone()),
                    "{host}:{port}"
                );
            }
        }
        assert_eq!(obs.status(), before, "a refused address changes nothing");
        obs.offer("timer-1", &brb());
        assert_eq!(obs.reconnect().state, ObsState::Off);
        assert_eq!(rt.block_on(obs.inventory()), Err(NOT_CONNECTED.to_string()));
        assert!(!dir.exists(), "nothing made a folder");
        let saved = rt
            .block_on(obs.set_password(Zeroizing::new(PASSWORD.into())))
            .unwrap();
        assert!(saved.has_password);
        assert!(dir.join(PASSWORD_FILE).is_file());
        let forgot = rt.block_on(obs.forget_password()).unwrap();
        assert!(!forgot.has_password);
        assert_eq!(forgot.state, ObsState::Off);
        assert!(lock(&obs.link).is_none(), "no link ever started");
        rt.block_on(async { tokio::time::sleep(Duration::from_millis(50)).await });
        assert!(!dialed(&listener), "nothing connected");
        std::fs::remove_dir_all(&dir).ok();
    }

    /// D4: switched on, `configure` moves the generation and says
    /// `connecting` for the canonical address before the driver can write,
    /// and keeps the config for `enabled()`. (Whether a reload's configure
    /// keeps a live link is the seam's: it needs a driver that stays.)
    #[test]
    fn configure_on_starts_a_generation() {
        let rt = runtime();
        let (_listener, port) = black_hole();
        let (obs, seen, dir) = shared(&rt, "configure-on");
        assert!(!obs.shared.enabled());
        let gen = obs.shared.current_gen();
        let s = rt
            .block_on(async { obs.configure(config(true, "localhost", port)) })
            .unwrap();
        assert_eq!(
            (s.state, s.host.as_str(), s.port, s.loopback),
            (ObsState::Connecting, "127.0.0.1", port, true)
        );
        assert_eq!(obs.shared.current_gen(), gen + 1);
        assert!(obs.shared.enabled());
        assert_eq!(
            seen.lock().unwrap().first().map(|s| s.state),
            Some(ObsState::Connecting)
        );
        let handle = lock(&obs.link).as_ref().map(|h| (h.gen, h.at));
        assert_eq!(handle, Some((gen + 1, at(port))));
        // Off: the generation moves again, so that driver is stale.
        obs.configure(config(false, "127.0.0.1", port)).unwrap();
        assert!(obs.shared.current_gen() > gen + 1);
        std::fs::remove_dir_all(&dir).ok();
    }

    /// D24, D28: a new `ObsShared` (a Divora launch) reads the slots
    /// `returns.json` holds and whether `password.bin` exists, and nothing
    /// else; a slot change is saved there.
    #[test]
    fn new_reads_the_slots_and_the_password() {
        let rt = runtime();
        let (obs, _seen, dir) = shared(&rt, "new-reads");
        assert!(!obs.status().has_password);
        give_slot(&obs, "timer-1");
        rt.block_on(obs.set_password(Zeroizing::new("pw".into())))
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while !obs.shared.saver.idle() {
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(5));
        }
        drop(obs);
        let (again, _seen) = shared_with(&rt, dir.clone(), MemCipher::default());
        assert!(again.status().has_password);
        assert_eq!(
            again.shared.plan_back("timer-1", &scene("BRB").uuid).to,
            Some(scene("Game"))
        );
        // At launch, before any configure, a switch back for it is due.
        again.offer("timer-1", &ObsOp::SceneBack);
        assert_eq!(again.status().held_back, 1);
        std::fs::remove_dir_all(&dir).ok();
    }

    /// §4: every message a command can refuse with is the fixture's
    /// `commandErrors` (which the e2e mock answers), each one produced here
    /// by the real command body; and the fixture lists no other.
    #[test]
    fn command_errors_are_the_fixtures() {
        let fixture: Value =
            serde_json::from_str(include_str!("../../../src/data/obs.fixture.json")).unwrap();
        let errors = &fixture["commandErrors"];
        let commands = fixture["obsWire"]["commands"].as_object().unwrap();
        let mut keys: Vec<&String> = errors.as_object().unwrap().keys().collect();
        keys.sort();
        let mut wanted: Vec<&String> = commands.keys().collect();
        wanted.sort();
        assert_eq!(keys, wanted, "one entry per command");
        let rust = serde_json::json!({
            "configure": {
                "notAnIp": HostError::NotAnIp.to_string(),
                "linkLocalV6": HostError::LinkLocalV6.to_string(),
                "badPort": BAD_PORT,
            },
            "reconnect": {},
            "setPassword": {
                "empty": NO_PASSWORD,
                "tooLong": LONG_PASSWORD,
                "encryptFailed": ENCRYPT_FAILED,
                "writeFailed": WRITE_FAILED,
            },
            "forgetPassword": {},
            "status": {},
            "inventory": {
                "notConnected": NOT_CONNECTED,
                "noAnswer": NO_ANSWER,
                "busy": BUSY,
                "refused": REFUSED,
            },
        });
        assert_eq!(*errors, rust);
        assert_eq!(
            errors["configure"]["notAnIp"],
            fixture["hostErrors"]["notAnIp"]
        );
        assert_eq!(
            errors["configure"]["linkLocalV6"],
            fixture["hostErrors"]["linkLocalV6"]
        );

        let said = |key: &str, field: &str| errors[key][field].as_str().unwrap().to_string();
        let rt = runtime();
        let (obs, _seen, dir) = shared(&rt, "command-errors");
        let refuse = |host: &str, port| obs.configure(config(true, host, port)).unwrap_err();
        assert_eq!(refuse("obs.local", 4460), said("configure", "notAnIp"));
        assert_eq!(refuse("fe80::1", 4460), said("configure", "linkLocalV6"));
        assert_eq!(refuse("127.0.0.1", 0), said("configure", "badPort"));
        let set = |obs: &ObsShared, pw: String| rt.block_on(obs.set_password(Zeroizing::new(pw)));
        assert_eq!(
            set(&obs, String::new()).unwrap_err(),
            said("setPassword", "empty")
        );
        assert!(
            set(&obs, "é".repeat(MAX_PASSWORD_CHARS)).is_ok(),
            "256 characters, 512 bytes"
        );
        assert_eq!(
            set(&obs, "a".repeat(MAX_PASSWORD_CHARS + 1)).unwrap_err(),
            said("setPassword", "tooLong")
        );
        let (refusing, _seen) = shared_with(
            &rt,
            scratch("command-errors-refusing"),
            MemCipher::refusing(),
        );
        assert_eq!(
            set(&refusing, "pw".into()).unwrap_err(),
            said("setPassword", "encryptFailed")
        );
        // A file where the folder goes: nothing can be written in it.
        let blocked = scratch("command-errors-blocked");
        std::fs::write(&blocked, "a file").unwrap();
        let (unwritable, _seen) = shared_with(&rt, blocked.clone(), MemCipher::default());
        assert_eq!(
            set(&unwritable, "pw".into()).unwrap_err(),
            said("setPassword", "writeFailed")
        );
        assert_eq!(
            rt.block_on(obs.inventory()).unwrap_err(),
            said("inventory", "notConnected")
        );
        // Connected, the driver's command queue full: busy.
        let stub = stub_link(&obs, true, 4460);
        connected(&obs, &stub);
        for _ in 0..CMDS_QUEUE {
            lock(&obs.link)
                .as_ref()
                .unwrap()
                .cmds
                .try_send(LinkCmd::Wake)
                .unwrap();
        }
        assert_eq!(
            rt.block_on(obs.inventory()).unwrap_err(),
            said("inventory", "busy")
        );
        drop(stub);
        // "OBS didn't answer in time." and "OBS refused the request." come
        // from a link that is asked: the seam's tests (§10.2 L24).
        std::fs::remove_file(&blocked).ok();
        std::fs::remove_dir_all(&dir).ok();
    }

    /// §10.3: `obs_set_password`'s body, through success, a refused encrypt
    /// and a file that can't be written, then a forget, logging at TRACE: no
    /// form of the password reaches a log line, an `obs-status` or an
    /// answer.
    #[test]
    fn set_password_logs_nothing_secret() {
        let rt = runtime();
        let (obs, seen, dir) = shared(&rt, "password-secret");
        let (refusing, refused_seen) = shared_with(
            &rt,
            scratch("password-secret-refusing"),
            MemCipher::refusing(),
        );
        let blocked = scratch("password-secret-blocked");
        std::fs::write(&blocked, "a file").unwrap();
        let (unwritable, blocked_seen) = shared_with(&rt, blocked.clone(), MemCipher::default());
        let (answers, log) = traced(|| {
            rt.block_on(async {
                let mut answers = Vec::new();
                for o in [&obs, &refusing, &unwritable] {
                    let pw = Zeroizing::new(PASSWORD.to_string());
                    answers.push(format!("{:?}", o.set_password(pw).await));
                    answers.push(format!("{:?}", o.forget_password().await));
                    answers.push(serde_json::to_string(&o.status()).unwrap());
                }
                answers
            })
        });
        assert!(log.contains("obs: password saved"), "not logged");
        assert!(
            log.contains("obs: couldn't save the password"),
            "not logged"
        );
        let mut all = log;
        for s in [&seen, &refused_seen, &blocked_seen] {
            for status in s.lock().unwrap().iter() {
                all.push_str(&serde_json::to_string(status).unwrap());
            }
        }
        all.push_str(&answers.join("\n"));
        assert_none_in(&all, &secret_forms(PASSWORD));
        // The blob on disk isn't the password either.
        rt.block_on(obs.set_password(Zeroizing::new(PASSWORD.into())))
            .unwrap();
        let blob = std::fs::read(dir.join(PASSWORD_FILE)).unwrap();
        assert_none_in(&String::from_utf8_lossy(&blob), &secret_forms(PASSWORD));
        assert_eq!(
            MemCipher::default().unprotect(&blob).unwrap().as_slice(),
            PASSWORD.as_bytes()
        );
        std::fs::remove_file(&blocked).ok();
        std::fs::remove_dir_all(&dir).ok();
    }
}

/// v1.61.0 (§10.5): rules every file under `obs/` keeps, read from the
/// sources, since no capture sees every thread and no test can watch a name
/// lookup that never happens.
#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic
)]
mod source_tests {
    /// Every `.rs` file under `src/obs/`, by name, `\r` dropped.
    fn sources() -> Vec<(String, String)> {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("src")
            .join("obs");
        let mut out: Vec<(String, String)> = std::fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().path())
            .filter(|p| p.extension().is_some_and(|x| x == "rs"))
            .map(|p| {
                let name = p.file_name().unwrap().to_string_lossy().into_owned();
                (name, std::fs::read_to_string(&p).unwrap().replace('\r', ""))
            })
            .collect();
        out.sort();
        assert!(
            out.iter().any(|(n, _)| n == "log.rs") && out.len() >= 8,
            "{out:?}"
        );
        out
    }

    /// D40: `log.rs` is the only file under `obs/` that names a `tracing`
    /// macro, so every line logged — on a tokio worker, the blocking pool or
    /// the timer worker — goes through a helper that takes no secret.
    #[test]
    fn obs_logs_only_through_its_log_module() {
        let macros: Vec<String> = [
            "trace",
            "debug",
            "info",
            "warn",
            "error",
            "event",
            "span",
            "trace_span",
            "debug_span",
            "info_span",
            "warn_span",
            "error_span",
        ]
        .iter()
        .map(|m| format!("{m}{}", '!'))
        .chain([
            format!("#[{}", "instrument"),
            format!("#[tracing::{}", "instrument"),
        ])
        .collect();
        let mut seen_in_log = 0;
        for (name, text) in sources() {
            for m in &macros {
                let n = text.matches(m.as_str()).count();
                if name == "log.rs" {
                    seen_in_log += n;
                } else {
                    assert_eq!(n, 0, "{name} names {m}: log through obs::log");
                }
            }
        }
        assert!(seen_in_log > 0, "log.rs logs");
    }

    /// D6: blocking work under `obs/` runs only through `obs::blocking`,
    /// which carries the caller's dispatcher; nothing is spawned on Tauri's
    /// global runtime (the spawner is handed in, so tests see their link).
    #[test]
    fn obs_blocking_work_carries_the_dispatcher() {
        let pool = format!("spawn{}", "_blocking");
        let global = format!("async_runtime::{}", "spawn");
        let mut found = Vec::new();
        for (name, text) in sources() {
            assert!(!text.contains(&global), "{name}: {global}");
            for (at, _) in text.match_indices(&pool) {
                found.push((name.clone(), at));
            }
        }
        assert_eq!(found.len(), 1, "{found:?}");
        let mod_rs = &sources()
            .into_iter()
            .find(|(n, _)| n == "mod.rs")
            .unwrap()
            .1;
        let head = format!("pub fn {}<F, R>(", "blocking");
        let body = &mod_rs[mod_rs.find(&head).unwrap()..];
        let body = &body[..body.find("\n}\n").unwrap()];
        assert!(body.contains(&pool), "{pool} is in obs::blocking");
        assert!(body.contains(&format!("dispatcher::{}", "get_default")));
        assert!(body.contains(&format!("dispatcher::{}", "with_default")));
    }

    /// D7, D37: the OBS link looks no name up: under `obs/`, nothing calls
    /// tokio-websockets' resolving connect, tokio's host lookup, std's
    /// socket-address conversion (the function or its trait) or the crate's
    /// getaddrinfo resolver; an address is only ever an `IpAddr` and a port.
    /// v1.61.0 (gate): and the link's one TCP connect is handed the
    /// `SocketAddr` made from them, never a string, which tokio's connect
    /// would take and resolve (its argument is generic over the conversion
    /// trait, so the needles above can't see one). The fake's own test
    /// clients are the fake's.
    #[test]
    fn the_obs_link_never_resolves_a_name() {
        let needles = [
            format!(".{}()", "connect"),
            format!("lookup_{}", "host"),
            format!("to_socket_{}", "addrs"),
            format!("ToSocket{}", "Addrs"),
            format!("{}::", "Gai"),
            format!("{}<", "Gai"),
            format!("{} ", "Gai"),
        ];
        for (name, text) in sources() {
            for needle in &needles {
                assert!(!text.contains(needle.as_str()), "{name}: {needle}");
            }
        }
        let (dial, by_addr, addr) = (
            format!("TcpStream::{}(", "connect"),
            format!("TcpStream::{}(addr)", "connect"),
            format!("let addr = SocketAddr::{}(", "new"),
        );
        let count = |needle: &str| {
            sources()
                .iter()
                .filter(|(name, _)| name != "fake.rs")
                .map(|(_, text)| text.matches(needle).count())
                .sum::<usize>()
        };
        assert_eq!(count(&dial), 1, "{dial}");
        assert_eq!(count(&by_addr), 1, "{by_addr}");
        assert_eq!(count(&addr), 1, "{addr}");
    }

    /// D7: the link's one connect path: tokio-websockets' `connect_on` (no
    /// name, no timeout of its own) called exactly once under `obs/`, and
    /// no-delay switched on once, on that stream. (This comment names
    /// neither call with its parenthesis, so it doesn't count itself.)
    #[test]
    fn the_obs_link_connects_one_way() {
        let (on, nodelay) = (
            format!("connect_{}(", "on"),
            format!("set_{}(true)", "nodelay"),
        );
        let count = |needle: &str| {
            sources()
                .iter()
                .filter(|(name, _)| name != "fake.rs")
                .map(|(_, text)| text.matches(needle).count())
                .sum::<usize>()
        };
        assert_eq!(count(&on), 1, "{on}");
        assert_eq!(count(&nodelay), 1, "{nodelay}");
    }

    /// D5: the link closes a connection itself (1001, a bounded flush and a
    /// bounded read to OBS's close), never through the sink's own close or
    /// the futures extension trait's, which send 1000 and wait for the echo
    /// unbounded. The fake server's own side is the fake's.
    #[test]
    fn the_obs_link_closes_itself() {
        let close = format!("poll_{}", "close");
        let sink = format!("Sink{}", "Ext");
        for (name, text) in sources() {
            if name == "fake.rs" {
                continue;
            }
            assert!(!text.contains(&close), "{name}: {close}");
            assert!(!text.contains(&sink), "{name}: {sink}");
        }
    }

    /// §10.2: no test under `obs/` dials 4455, where a real OBS may listen.
    /// The files that open sockets, start links or hold `ObsShared` tests
    /// never name it; the pure modules that do (in configs and statuses,
    /// as data) open no socket; and the fake binds only `127.0.0.1:0`.
    #[test]
    fn obs_tests_never_dial_4455() {
        let port = format!("{}{}", 44, 55);
        for (name, text) in sources() {
            let pure = matches!(name.as_str(), "machine.rs" | "proto.rs" | "scenes.rs");
            if pure {
                for socket in [
                    "TcpStream",
                    "TcpListener",
                    "UdpSocket",
                    "link::run",
                    "ObsShared::new",
                ] {
                    assert!(!text.contains(socket), "{name}: {socket}");
                }
                continue;
            }
            for (i, line) in text.lines().enumerate() {
                let code = line.split("//").next().unwrap();
                let this_test = line.contains("fn obs_tests_never_dial_");
                assert!(
                    this_test || !code.contains(&port),
                    "{name}:{}: {line}",
                    i + 1
                );
            }
            for (i, line) in text.lines().enumerate() {
                if line.contains(&format!("{}::bind(", "TcpListener")) {
                    assert!(
                        line.contains("LOCALHOST, 0)") || line.contains("127.0.0.1:0"),
                        "{name}:{}: {line}",
                        i + 1
                    );
                }
            }
        }
    }
}
