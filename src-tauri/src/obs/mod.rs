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

pub mod files;
pub mod link;
pub mod log;
pub mod machine;
pub mod proto;
pub mod scenes;

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use tauri::{AppHandle, Manager};
use tokio::runtime::Handle;
use tokio::sync::{mpsc, oneshot, watch};
use tokio::task::JoinHandle;
use zeroize::Zeroizing;

pub use crate::timer_actions::ObsOp;
pub use crate::timers::{Follow, FollowObs, ObsFeed, ObsText};

use crate::timer_thread::TimerShared;
use crate::timers::TimerOp;
use files::{Cipher, Unreadable};
use machine::{LinkConfig, ObsConfig, ObsInventory, ObsStatus, StatusCell};
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

/// The running link's ends, kept by [`ObsShared`]: only the newest
/// generation's (D4).
struct LinkHandle {
    ops: mpsc::Sender<LinkOp>,
    cmds: mpsc::Sender<LinkCmd>,
    /// The link's [`LinkDeps::closed_ack`].
    closed: std::sync::mpsc::Receiver<()>,
}

/// The parts every link generation shares with [`ObsShared`], each behind
/// its own leaf lock (D3): handed to a driver as its [`StatusSink`] and
/// [`SlotStore`].
// S1: track C's bodies read every field.
#[allow(dead_code)]
struct Shared {
    /// The status, its `seq` and the link generation: the only copy of it.
    status: Mutex<StatusCell>,
    emit: Emit,
    /// D24: the slots and due marks, for the life of the process.
    slots: Mutex<Slots>,
    /// D18: the source uuids Divora wrote in this process.
    written: Mutex<HashSet<String>>,
}

/// D3: the OBS link's state, managed by the app once `init` has built it,
/// separately from `AppState` (it must exist before the timer thread, which
/// `AppState` comes after). The commands, the timer worker's
/// [`ObsShared::offer`] and every link generation reach it. No lock is held
/// while another is taken, or across an `.await`.
// S1: track C's bodies read every field (the commands, `offer`, the link's
// start) and may regroup them.
#[allow(dead_code)]
pub struct ObsShared {
    shared: Arc<Shared>,
    /// The last config `obs_configure` received, in memory only; `None`
    /// before the first (D16).
    config: Mutex<Option<ObsConfig>>,
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
    /// D3: off, no link, no config. `spawner` is Tauri's runtime in the app
    /// and the test's own in tests.
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
        // S1: track C loads the slots from `dir`'s `returns.json` (D24) and
        // `hasPassword` from `password.bin`.
        Self {
            shared: Arc::new(Shared {
                status: Mutex::new(StatusCell::new()),
                emit,
                slots: Mutex::new(Slots::default()),
                written: Mutex::new(HashSet::new()),
            }),
            config: Mutex::new(None),
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
    /// connects; `enabled:true` starts, keeps or restarts the link.
    pub fn configure(&self, cfg: ObsConfig) -> Result<ObsStatus, String> {
        // S1: track C writes this body. Until then no link starts.
        let _ = cfg;
        Ok(self.status())
    }

    /// `obs_reconnect` (D4): retrying, an attempt now; a stop state, a
    /// restart; otherwise nothing.
    #[must_use]
    pub fn reconnect(&self) -> ObsStatus {
        // S1: track C writes this body.
        self.status()
    }

    /// `obs_set_password` (D28): encrypts and writes `password.bin` on the
    /// blocking pool, then restarts a link in a stop state or a running
    /// one. Works while off. Never echoes the password.
    #[allow(clippy::unused_async, clippy::unused_async_trait_impl)] // S1: track C's body awaits the blocking pool
    pub async fn set_password(&self, password: Zeroizing<String>) -> Result<ObsStatus, String> {
        // S1: track C writes this body. Until then nothing is stored.
        drop(password);
        Err(WRITE_FAILED.to_string())
    }

    /// `obs_forget_password`: deletes `password.bin` (absent is fine) and
    /// restarts a running link. Works while off.
    #[allow(clippy::unused_async, clippy::unused_async_trait_impl)] // S1: track C's body awaits the blocking pool
    pub async fn forget_password(&self) -> Result<ObsStatus, String> {
        // S1: track C writes this body.
        Ok(self.status())
    }

    /// `obs_inventory` (D19): the scenes and text sources, from a connected
    /// link, within 15 s; a second call while one runs joins it.
    #[allow(clippy::unused_async, clippy::unused_async_trait_impl)] // S1: track C's body awaits the link's answer
    pub async fn inventory(&self) -> Result<ObsInventory, String> {
        // S1: track C writes this body.
        Err(NOT_CONNECTED.to_string())
    }

    /// D17: `timer`'s `obs` action, from the timer worker. Never waits:
    /// stamps it now, then hands it to a connected link (`try_send`), or
    /// records a skip, or marks a switch back due.
    pub fn offer(&self, timer: &str, op: &ObsOp) {
        // S1: track C writes this body. Until then an op does nothing.
        let _ = (timer, op);
    }

    /// D5: the tray's Quit. With no link, returns at once; otherwise stops
    /// it and waits up to [`EXIT_WAIT`] for it to close its connection.
    pub fn before_exit(&self) {
        let Some(link) = lock(&self.link).take() else {
            return;
        };
        let LinkHandle { ops, cmds, closed } = link;
        drop(ops);
        drop(cmds);
        let _ = closed.recv_timeout(EXIT_WAIT);
    }
}

/// D3: at setup, after `spawn_timer_thread` and before `AppState` is
/// managed: builds [`ObsShared`] on Tauri's runtime, loads its slots from
/// `dir`, and manages it.
#[allow(unused_variables)] // S1: track C writes this body
pub fn init(
    app: &AppHandle,
    feed: watch::Receiver<Arc<ObsFeed>>,
    timers: Arc<TimerShared>,
    dir: PathBuf,
) {
    // S1: track C writes this body; nothing calls it yet, so nothing is
    // managed and no `obs/` folder is touched.
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
