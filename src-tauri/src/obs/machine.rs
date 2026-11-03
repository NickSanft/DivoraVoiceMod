//! v1.61.0: the link's rules, as pure functions and plain data — the wire
//! types of §4 (the config, the status, the inventory), the status cell,
//! hosts, backoff, what an output event does to a count-up that follows it,
//! which text to push next, and what a refused request means. Nothing here
//! reads a clock, waits, logs or touches a socket: callers hand in the time,
//! so every rule is tested on any machine (§10.1).

use std::collections::{HashMap, HashSet};
use std::io;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use super::proto::{ObsEvent, OutputState, RequestKind};
use super::scenes::SceneRef;
use super::LinkTiming;
use crate::timers::{FollowObs, ObsFeed, TimerOp};

// ---- The wire (§4) -----------------------------------------------------------

/// `obs_configure`'s `config`, and what the page stores as `divora.obs`
/// (less its `breakScene`). A `port` that isn't a `u16` is refused by
/// Tauri before Rust sees it; 0 by [`BAD_PORT`](super::BAD_PORT).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ObsConfig {
    pub enabled: bool,
    /// An IP literal or `localhost`, as typed: [`parse_host`] reads it.
    pub host: String,
    pub port: u16,
}

/// Where the link is (§7.5). The stop states — `needsPassword` through
/// `unsupported` — last until something changes (D13).
#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ObsState {
    /// Switched off, or never configured.
    Off,
    Connecting,
    /// Identified, and the startup steps done.
    Connected,
    /// Waiting to try again (`reason`, `retryAtMs`).
    Retrying,
    /// OBS asks for a password and none is stored.
    NeedsPassword,
    /// OBS closed with 4009.
    AuthFailed,
    /// The stored password won't decrypt.
    PasswordUnreadable,
    /// OBS's WebSocket server is older than 5.4.0.
    TooOld,
    /// OBS closed with 4010: it no longer serves RPC 1.
    NeedsNewerDivora,
    /// OBS closed with 4011 (a kick).
    Kicked,
    /// An unreadable version, or a request missing (`reason`, `detail`).
    Unsupported,
}

impl ObsState {
    /// Every state, in the fixture's `states` order.
    pub const ALL: [Self; 11] = [
        Self::Off,
        Self::Connecting,
        Self::Connected,
        Self::Retrying,
        Self::NeedsPassword,
        Self::AuthFailed,
        Self::PasswordUnreadable,
        Self::TooOld,
        Self::NeedsNewerDivora,
        Self::Kicked,
        Self::Unsupported,
    ];
}

/// Why the link is retrying or stopped (§7.1); fixed codes, never OBS's
/// text (D15).
#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ObsReason {
    Refused,
    Unreachable,
    NoAnswer,
    NotObs,
    NotReady,
    Closed,
    ObsExiting,
    Silent,
    TooBig,
    Protocol,
    BadVersion,
    MissingRequest,
}

impl ObsReason {
    /// Every reason, in the fixture's `reasons` order.
    pub const ALL: [Self; 12] = [
        Self::Refused,
        Self::Unreachable,
        Self::NoAnswer,
        Self::NotObs,
        Self::NotReady,
        Self::Closed,
        Self::ObsExiting,
        Self::Silent,
        Self::TooBig,
        Self::Protocol,
        Self::BadVersion,
        Self::MissingRequest,
    ];
}

/// The op a scene skip names.
#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum SkipOp {
    Scene,
    SceneBack,
}

impl SkipOp {
    /// Both, in the fixture's `skipOps` order.
    pub const ALL: [Self; 2] = [Self::Scene, Self::SceneBack];
}

/// Why a scene op didn't switch (§7.13, D23). A `sceneBack` is never
/// `queueFull` or `late`: a full queue or a slow OBS makes it due.
#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum SkipWhy {
    NotConnected,
    QueueFull,
    NotFound,
    NotReady,
    TimedOut,
    Failed,
    Late,
}

impl SkipWhy {
    /// Every why, in the fixture's `skipWhys` order.
    pub const ALL: [Self; 7] = [
        Self::NotConnected,
        Self::QueueFull,
        Self::NotFound,
        Self::NotReady,
        Self::TimedOut,
        Self::Failed,
        Self::Late,
    ];
}

/// Why a text push didn't land (§7.13).
#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum TextSkipWhy {
    Failed,
    TimedOut,
}

impl TextSkipWhy {
    /// Both, in the fixture's `textSkipWhys` order.
    pub const ALL: [Self; 2] = [Self::Failed, Self::TimedOut];
}

/// OBS's stream, as the status shows it.
#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum StreamState {
    Off,
    Starting,
    Live,
    Reconnecting,
    Stopping,
}

impl StreamState {
    /// Every word, in the fixture's `streamStates` order.
    pub const ALL: [Self; 5] = [
        Self::Off,
        Self::Starting,
        Self::Live,
        Self::Reconnecting,
        Self::Stopping,
    ];
}

/// OBS's recording, as the status shows it.
#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum RecordState {
    Off,
    Starting,
    Recording,
    Paused,
    Stopping,
}

impl RecordState {
    /// Every word, in the fixture's `recordStates` order.
    pub const ALL: [Self; 5] = [
        Self::Off,
        Self::Starting,
        Self::Recording,
        Self::Paused,
        Self::Stopping,
    ];
}

/// The last scene op that didn't switch: replaced by the next scene skip,
/// cleared by the next switch that succeeds (§7.5).
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct LastSkip {
    pub op: SkipOp,
    pub why: SkipWhy,
    /// The scene's name from the action (the user's own); `None` for a
    /// `sceneBack`.
    pub name: Option<String>,
    /// The timer id whose action it was.
    pub timer: String,
    /// Wall ms.
    pub at_ms: i64,
}

/// The last text push that didn't land: cleared by the next confirmed one.
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TextSkip {
    pub why: TextSkipWhy,
    /// Wall ms.
    pub at_ms: i64,
}

/// §4, D15: the link's whole state, sent as `obs-status` on change only and
/// answered by every OBS command. Of two, the larger `seq` is newer. No
/// field holds a secret or free text from OBS: versions are sanitized,
/// reasons are codes, names are the user's own.
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ObsStatus {
    /// +1 per change in this process.
    pub seq: u64,
    pub state: ObsState,
    /// The canonical IP literal in use; `""` before any configure.
    pub host: String,
    pub port: u16,
    /// The canonical address's `is_loopback()`.
    pub loopback: bool,
    /// `password.bin` exists; never the password.
    pub has_password: bool,
    /// From the last Hello; `None` before one.
    pub auth_required: Option<bool>,
    /// `GetVersion`'s `obsVersion`, sanitized ([`sanitize_version`]).
    pub obs_version: Option<String>,
    /// Hello's `obsWebSocketVersion`, sanitized.
    pub websocket_version: Option<String>,
    /// `None` while not connected.
    pub stream: Option<StreamState>,
    pub record: Option<RecordState>,
    /// Why retrying or stopped.
    pub reason: Option<ObsReason>,
    /// Only for `missingRequest`: one of the nine request names.
    pub detail: Option<&'static str>,
    /// Wall ms of the next attempt, while retrying.
    pub retry_at_ms: Option<i64>,
    /// Mapped text-source uuids OBS can't find on this connection (D20).
    pub missing: Vec<String>,
    pub last_skip: Option<LastSkip>,
    pub text_skip: Option<TextSkip>,
    /// Switch-backs due, waiting for a connected link (D23).
    pub held_back: u32,
    /// +1 per connection and per burst of scene changes (≤ 1 per 250 ms).
    pub inventory_rev: u64,
}

impl ObsStatus {
    /// Before anything: off, never configured (no host, port 0), nothing
    /// known. The fixture's first `obsStatusSamples` row.
    #[must_use]
    pub fn off() -> Self {
        Self {
            seq: 0,
            state: ObsState::Off,
            host: String::new(),
            port: 0,
            loopback: false,
            has_password: false,
            auth_required: None,
            obs_version: None,
            websocket_version: None,
            stream: None,
            record: None,
            reason: None,
            detail: None,
            retry_at_ms: None,
            missing: Vec::new(),
            last_skip: None,
            text_skip: None,
            held_back: 0,
            inventory_rev: 0,
        }
    }
}

/// Which text source a source is (§7.11); nothing else of its kind leaves
/// the link.
#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum TextKind {
    /// Text (GDI+): `text_gdiplus`.
    Gdiplus,
    /// Text (`FreeType` 2): `text_ft2_source`.
    Ft2,
}

impl TextKind {
    /// Both, in the fixture's `textSourceKinds` order.
    pub const ALL: [Self; 2] = [Self::Gdiplus, Self::Ft2];
}

/// One text source an inventory lists (D19).
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TextSource {
    pub uuid: String,
    pub name: String,
    pub kind: TextKind,
    /// The source's own "Read from file" is on: listed, not pickable.
    pub reads_from_file: bool,
}

/// §4, D19: `obs_inventory`'s answer. Nothing from a source's settings but
/// its file flag.
#[derive(Serialize, Clone, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct ObsInventory {
    /// The `inventoryRev` it was read at.
    pub rev: u64,
    /// OBS's top-to-bottom order.
    pub scenes: Vec<SceneRef>,
    pub text_sources: Vec<TextSource>,
    pub program_scene_uuid: Option<String>,
    /// Past [`MAX_SCENES`](super::MAX_SCENES) scenes or
    /// [`MAX_TEXT_SOURCES`](super::MAX_TEXT_SOURCES) text sources.
    pub truncated: bool,
}

// ---- The status cell (D4, D15) ---------------------------------------------

/// The status and the link generation, under one lock in `ObsShared`: a
/// write that changes the status bumps `seq` and returns the new status for
/// the caller to emit under the same lock, so emits go out in `seq` order.
#[derive(Clone, Debug)]
pub struct StatusCell {
    status: ObsStatus,
    gen: u64,
}

impl StatusCell {
    /// [`ObsStatus::off`], generation 0.
    #[must_use]
    pub fn new() -> Self {
        Self {
            status: ObsStatus::off(),
            gen: 0,
        }
    }

    /// The status now.
    #[must_use]
    pub fn status(&self) -> &ObsStatus {
        &self.status
    }

    /// The current link generation.
    #[must_use]
    pub fn gen(&self) -> u64 {
        self.gen
    }

    /// D4: a link starts for `cfg`: the generation + 1 and `connecting` in
    /// one step, before the driver is spawned with that generation. The new
    /// generation, and the status if it changed.
    ///
    /// The status gets `cfg`'s canonical host, port and loopback, and loses
    /// what belonged to the last connection (`reason`, `detail`,
    /// `retryAtMs`, `stream`, `record`, `missing`). What the last Hello and
    /// `GetVersion` said is kept for the same address and cleared for a new
    /// one. Skips, `heldBack`, `hasPassword` and `inventoryRev` are
    /// `ObsShared`'s and stay.
    pub fn begin(&mut self, cfg: &LinkConfig) -> (u64, Option<ObsStatus>) {
        self.gen = self.gen.saturating_add(1);
        let host = cfg.ip.to_string();
        let changed = self.write_shared(|s| {
            if s.host != host || s.port != cfg.port {
                s.auth_required = None;
                s.obs_version = None;
                s.websocket_version = None;
            }
            s.state = ObsState::Connecting;
            s.host = host;
            s.port = cfg.port;
            s.loopback = cfg.loopback();
            s.reason = None;
            s.detail = None;
            s.retry_at_ms = None;
            s.stream = None;
            s.record = None;
            s.missing.clear();
        });
        (self.gen, changed)
    }

    /// A driver's write: ignored when `gen` is stale. `f` applied to a
    /// clone; if that differs, `seq` + 1, stored and returned.
    pub fn write(&mut self, gen: u64, f: impl FnOnce(&mut ObsStatus)) -> Option<ObsStatus> {
        if gen != self.gen {
            return None;
        }
        self.write_shared(f)
    }

    /// `ObsShared`'s own writes (skips, `heldBack`, `off`): always current.
    /// `seq` is the cell's: whatever `f` does to it is put back.
    pub fn write_shared(&mut self, f: impl FnOnce(&mut ObsStatus)) -> Option<ObsStatus> {
        let mut next = self.status.clone();
        f(&mut next);
        next.seq = self.status.seq;
        if next == self.status {
            return None;
        }
        next.seq = next.seq.saturating_add(1);
        self.status.clone_from(&next);
        Some(next)
    }
}

impl Default for StatusCell {
    fn default() -> Self {
        Self::new()
    }
}

// ---- Hosts (D16, §7.4) -----------------------------------------------------

/// Why a typed address was refused; its message is what `obs_configure`
/// answers (fixture `hostErrors`).
#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum HostError {
    /// A name, a URL, a port inside the host, an unspecified, multicast or
    /// broadcast address, or nothing.
    NotAnIp,
    /// IPv6 link-local (`fe80::/10`), or any `%` zone id.
    LinkLocalV6,
}

impl std::fmt::Display for HostError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::NotAnIp => "Type an IP address, like 127.0.0.1, or localhost.",
            Self::LinkLocalV6 => {
                "Divora can't use a link-local IPv6 address (fe80::…). Type the PC's IPv4 \
                 address, like 192.168.1.20."
            }
        })
    }
}

/// §7.4: the address a typed host names, canonical (`to_canonical()`
/// before every check): trimmed; `localhost` (any case) is 127.0.0.1; one
/// pair of brackets stripped; a `%` anywhere is the link-local refusal; then
/// `IpAddr`'s own parse, and the refusals of D16. Never a name lookup.
///
/// The `%` check comes first because std refuses `fe80::1%3` as a plain
/// parse error, which would get the other message. Canonical first, so
/// `::ffff:127.0.0.1` is loopback and `::ffff:0.0.0.0` unspecified.
pub fn parse_host(host: &str) -> Result<IpAddr, HostError> {
    let typed = host.trim();
    if typed.contains('%') {
        return Err(HostError::LinkLocalV6);
    }
    if typed.eq_ignore_ascii_case("localhost") {
        return Ok(IpAddr::V4(Ipv4Addr::LOCALHOST));
    }
    let bare = typed
        .strip_prefix('[')
        .and_then(|t| t.strip_suffix(']'))
        .unwrap_or(typed);
    let ip = bare
        .parse::<IpAddr>()
        .map_err(|_| HostError::NotAnIp)?
        .to_canonical();
    // `fe80::/10` names no interface without a zone id.
    if matches!(ip, IpAddr::V6(v6) if v6.is_unicast_link_local()) {
        return Err(HostError::LinkLocalV6);
    }
    let broadcast = matches!(ip, IpAddr::V4(v4) if v4.is_broadcast());
    if ip.is_unspecified() || ip.is_multicast() || broadcast {
        return Err(HostError::NotAnIp);
    }
    Ok(ip)
}

/// D7: what a link connects to: only ever an IP and a port.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LinkConfig {
    /// Canonical ([`parse_host`]).
    pub ip: IpAddr,
    pub port: u16,
}

impl LinkConfig {
    /// The canonical address's `is_loopback()`.
    #[must_use]
    pub fn loopback(&self) -> bool {
        self.ip.is_loopback()
    }

    /// The upgrade's URI: `ws://127.0.0.1:4455/`, `ws://[::1]:4455/`.
    #[must_use]
    pub fn uri(&self) -> String {
        format!("ws://{}/", SocketAddr::new(self.ip, self.port))
    }

    /// D8: a request's timeout here: 3 s to a loopback address, 5 s to any
    /// other (`t`'s).
    #[must_use]
    pub fn request_timeout(&self, t: &LinkTiming) -> Duration {
        if self.loopback() {
            t.request_loopback
        } else {
            t.request_other
        }
    }
}

// ---- Ends, backoff (D13, D14, §7.1, §7.3) ----------------------------------

/// §7.1: why a link stops trying until something changes (D13).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stop {
    NeedsPassword,
    AuthFailed,
    PasswordUnreadable,
    TooOld,
    NeedsNewerDivora,
    Kicked,
    Unsupported(Unsupported),
}

/// What `unsupported` is about.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unsupported {
    /// Hello's version isn't a string with a leading `x.y.z`.
    BadVersion,
    /// `GetVersion`'s `availableRequests` lacks this one of the nine.
    MissingRequest(&'static str),
}

/// §7.1's connect rows: an I/O error while connecting, by its kind —
/// refused, unreachable, and every other kind `unreachable` too (logged
/// once with its kind by the caller). Never `closed`: nothing was open.
pub fn connect_reason(kind: io::ErrorKind) -> ObsReason {
    match kind {
        io::ErrorKind::ConnectionRefused => ObsReason::Refused,
        // `TimedOut`, `HostUnreachable`, `NetworkUnreachable`, and every
        // other kind (`AddrNotAvailable`, `PermissionDenied` when outbound
        // is blocked, a reset or an abort, `InvalidInput`, …).
        _ => ObsReason::Unreachable,
    }
}

/// D14's base steps, ms, for 1 to 6 failures; 30 s after.
const BACKOFF_STEPS_MS: [u32; 6] = [1_000, 2_000, 4_000, 8_000, 15_000, 30_000];

/// D14: no wait is longer, jitter included.
pub const BACKOFF_CAP_MS: u64 = 30_000;

/// §7.3, D14: the wait in ms before attempt `failures` + 1: 1, 2, 4, 8, 15,
/// then 30 s (in ms), times `0.8 + 0.4·r`, then at most 30 000 (the cap
/// after the jitter, so no wait passes 30 s). `r` is clamped to [0, 1] (not
/// a number: 0.5); 0 failures waits as 1 does.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)] // in [800, 30 000]
pub fn backoff_ms(failures: u32, r: f64) -> u64 {
    let step = usize::try_from(failures.saturating_sub(1)).unwrap_or(usize::MAX);
    let base = BACKOFF_STEPS_MS
        .get(step)
        .or(BACKOFF_STEPS_MS.last())
        .copied()
        .unwrap_or(30_000);
    let r = if r.is_nan() { 0.5 } else { r.clamp(0.0, 1.0) };
    let ms = (f64::from(base) * 0.4f64.mul_add(r, 0.8)).round();
    (ms as u64).min(BACKOFF_CAP_MS)
}

/// [`backoff_ms`] in units of `unit` (1 s in the app: the ms themselves;
/// §10.2's tests scale it): the backoff sleep's length.
#[must_use]
pub fn backoff(failures: u32, r: f64, unit: Duration) -> Duration {
    // ≤ 30 000, so the u32 holds it; `unit × ms / 1000`.
    let ms = u32::try_from(backoff_ms(failures, r)).unwrap_or(u32::MAX);
    unit.saturating_mul(ms) / 1_000
}

// ---- Follow OBS (D26, §7.7) ------------------------------------------------

/// What an output event does to a count-up that follows it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FollowAction {
    /// Applied at once through `TimerControl`.
    Op(TimerOp),
    /// Ask OBS (`GetStreamStatus` or `GetRecordStatus`), then [`join_seed`].
    Seed,
}

/// `GetStreamStatus`'s or `GetRecordStatus`'s answer, as [`join_seed`] reads
/// it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct OutputStatus {
    /// `outputActive`.
    pub active: bool,
    /// A stream's `outputReconnecting`; false for a recording.
    pub reconnecting: bool,
    /// A recording's `outputPaused`; false for a stream.
    pub paused: bool,
    /// `outputDuration`, ms.
    pub duration_ms: u64,
}

/// §7.7's first table (fixture `followTable`): `ev` for a count-up that
/// follows `what`; `since_ms` is the time since the event was read, which a
/// `STARTED` counts from.
///
/// Keyed on `outputState` alone, never `outputActive` (false while a stream
/// reconnects or a recording pauses). Only the states OBS sends for each
/// output count (EH:341-398, :659-671): a stream's PAUSED or RESUMED and a
/// recording's RECONNECTING or RECONNECTED, which OBS never sends, do
/// nothing, as STARTING, STOPPING and UNKNOWN do.
pub fn follow_op(what: FollowObs, ev: &ObsEvent, since_ms: u64) -> Option<FollowAction> {
    use OutputState::{Paused, Reconnected, Resumed, Started, Stopped};
    let started = FollowAction::Op(TimerOp::SetElapsed {
        ms: since_ms,
        running: true,
    });
    match (what, ev) {
        (FollowObs::Stream, ObsEvent::Stream(Started))
        | (FollowObs::Record, ObsEvent::Record(Started)) => Some(started),
        // Stopped keeps the last value: the card shows Resume.
        (FollowObs::Stream, ObsEvent::Stream(Stopped))
        | (FollowObs::Record, ObsEvent::Record(Stopped | Paused)) => {
            Some(FollowAction::Op(TimerOp::Pause))
        }
        (FollowObs::Record, ObsEvent::Record(Resumed)) => Some(FollowAction::Op(TimerOp::Resume)),
        (FollowObs::Stream, ObsEvent::Stream(Reconnected)) => Some(FollowAction::Seed),
        _ => None,
    }
}

/// §7.7's second table (fixture `seedTable`): what OBS's status of `what`
/// does to the timer, given whether it is running now. A reconnecting
/// stream sets nothing, whatever `outputActive` says; a stream's `paused`
/// and a recording's `reconnecting` aren't read.
pub fn join_seed(what: FollowObs, status: OutputStatus, running_now: bool) -> Option<TimerOp> {
    let (reconnecting, paused) = match what {
        FollowObs::Stream => (status.reconnecting, false),
        FollowObs::Record => (false, status.paused),
    };
    if reconnecting {
        None
    } else if status.active {
        Some(TimerOp::SetElapsed {
            ms: status.duration_ms,
            running: !paused,
        })
    } else if running_now {
        Some(TimerOp::Pause)
    } else {
        None
    }
}

/// D26: the `(timer id, what)` pairs `next` follows that `prev` didn't —
/// every pair when `sleeps` changed — each to be seeded, in `next`'s order.
/// A follow switched between Streaming and Recording is a new pair.
pub fn seed_pairs(prev: &ObsFeed, next: &ObsFeed) -> Vec<(String, FollowObs)> {
    let slept = prev.sleeps != next.sleeps;
    next.follows
        .iter()
        .filter(|f| slept || !prev.follows.contains(f))
        .map(|f| (f.id.clone(), f.what))
        .collect()
}

// ---- Text pushes (D18, §7.8) -----------------------------------------------

/// What OBS confirmed on this connection: source uuid → text. A new
/// connection starts empty.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Pushed(pub HashMap<String, String>);

/// §7.8: the next push, `(uuid, text)`: the first uuid in feed order whose
/// text differs from what OBS confirmed (not one OBS can't find), else the
/// first uuid Divora wrote that left the feed (`""`, a clear), else none.
///
/// Only a uuid's first entry in the feed counts (the feed keeps one per
/// uuid already). "First" among the clears is the smallest uuid, so the
/// order doesn't depend on the set's hashing. A clear isn't sent to a
/// source OBS can't find, nor to one OBS already confirmed empty.
#[allow(clippy::implicit_hasher)] // the link's sets are std's
pub fn next_push(
    pushed: &Pushed,
    feed: &ObsFeed,
    missing: &HashSet<String>,
    written: &HashSet<String>,
) -> Option<(String, String)> {
    let mut mapped: HashSet<&str> = HashSet::with_capacity(feed.texts.len());
    for t in &feed.texts {
        // Recorded before the missing check: a missing source is still
        // mapped, so it is never cleared.
        if !mapped.insert(t.uuid.as_str()) || missing.contains(&t.uuid) {
            continue;
        }
        if pushed.0.get(&t.uuid) != Some(&t.text) {
            return Some((t.uuid.clone(), t.text.clone()));
        }
    }
    written
        .iter()
        .filter(|uuid| {
            !mapped.contains(uuid.as_str())
                && !missing.contains(*uuid)
                && pushed.0.get(*uuid).is_none_or(|t| !t.is_empty())
        })
        .min()
        .map(|uuid| (uuid.clone(), String::new()))
}

// ---- Request failures (D20, D22, §7.13) ------------------------------------

/// What a request was sent for (fixture `failureTable`'s `work`).
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum Work {
    /// A text push.
    Push,
    /// A `scene` op's Get or Set.
    Scene,
    /// A `sceneBack`'s Get or Set.
    SceneBack,
    /// `GetVersion`, `GetStreamStatus` or `GetRecordStatus` after Identified.
    Startup,
    /// One of `obs_inventory`'s steps.
    Inventory,
}

/// §7.13: what an error answer means for its work (fixture
/// `failureResults`, whose text says each in full).
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum Failure {
    /// A push: the uuid into `missing`, no text skip.
    Missing,
    /// A push: the retry armed, no text skip.
    RetryLater,
    /// A push: `textSkip {failed}`, the retry armed.
    TextSkipFailed,
    /// A scene op: `lastSkip {scene, notReady}`.
    SkipNotReady,
    /// `lastSkip {op, notFound}`; a switch back's plan committed.
    SkipNotFound,
    /// `lastSkip {op, failed}`; a switch back's plan committed.
    SkipFailed,
    /// A switch back: due again with its first stamp.
    DueAgain,
    /// A startup or inventory step: wait, then again.
    WaitAndRetry,
    /// A startup step: the connection ends (`protocol`).
    EndProtocol,
    /// The inventory's scene or input list: "OBS refused the request."
    Refused,
    /// The inventory's settings of one source: listed, not reading a file.
    ListedNotReadingFile,
}

/// `RequestStatus` 207 `NotReady`: OBS is starting, closing or changing scene
/// collection (P576:889-895).
pub const NOT_READY: u64 = 207;

/// §7.13 (fixture `failureTable`): `code`, OBS's error answer to `request`
/// sent for `work`. A success, a timeout, a session's end and a 207 after
/// `ExitStarted` are the driver's, not this table's.
///
/// The not-found codes differ on purpose: a push treats any resource code
/// (600-699) as a source OBS can't find, while a scene op treats only 600
/// `ResourceNotFound` and 602 `InvalidResourceType` as a missing scene (604, a
/// scene off the main canvas, is `failed`); a 7xx is a processing failure,
/// never a missing source.
pub fn request_failure(work: Work, request: RequestKind, code: u64) -> Failure {
    let not_ready = code == NOT_READY;
    match work {
        Work::Push if not_ready => Failure::RetryLater,
        Work::Push if (600..=699).contains(&code) => Failure::Missing,
        Work::Push => Failure::TextSkipFailed,
        Work::Scene if not_ready => Failure::SkipNotReady,
        Work::SceneBack if not_ready => Failure::DueAgain,
        Work::Scene | Work::SceneBack if matches!(code, 600 | 602) => Failure::SkipNotFound,
        Work::Scene | Work::SceneBack => Failure::SkipFailed,
        Work::Startup | Work::Inventory if not_ready => Failure::WaitAndRetry,
        Work::Startup => Failure::EndProtocol,
        Work::Inventory if request == RequestKind::GetInputSettings => {
            Failure::ListedNotReadingFile
        }
        Work::Inventory => Failure::Refused,
    }
}

// ---- Status words, versions, coalescing (D15) ------------------------------

/// A `StreamStateChanged`'s state, as the status's `stream` word; `None`
/// leaves it as it was (UNKNOWN, and PAUSED and RESUMED, which only a
/// recording has).
pub fn stream_word(state: OutputState) -> Option<StreamState> {
    match state {
        OutputState::Starting => Some(StreamState::Starting),
        OutputState::Started | OutputState::Reconnected => Some(StreamState::Live),
        OutputState::Reconnecting => Some(StreamState::Reconnecting),
        OutputState::Stopping => Some(StreamState::Stopping),
        OutputState::Stopped => Some(StreamState::Off),
        OutputState::Paused | OutputState::Resumed | OutputState::Unknown => None,
    }
}

/// A `RecordStateChanged`'s state, as the status's `record` word; `None`
/// leaves it as it was (UNKNOWN, and RECONNECTING and RECONNECTED, which
/// only a stream has).
pub fn record_word(state: OutputState) -> Option<RecordState> {
    match state {
        OutputState::Starting => Some(RecordState::Starting),
        OutputState::Started | OutputState::Resumed => Some(RecordState::Recording),
        OutputState::Paused => Some(RecordState::Paused),
        OutputState::Stopping => Some(RecordState::Stopping),
        OutputState::Stopped => Some(RecordState::Off),
        OutputState::Reconnecting | OutputState::Reconnected | OutputState::Unknown => None,
    }
}

/// `GetStreamStatus`' answer as the `stream` word at a connected moment,
/// read as [`join_seed`] reads it: reconnecting whatever `outputActive`
/// says, else live or off.
pub fn stream_status_word(s: OutputStatus) -> StreamState {
    if s.reconnecting {
        StreamState::Reconnecting
    } else if s.active {
        StreamState::Live
    } else {
        StreamState::Off
    }
}

/// `GetRecordStatus`' answer as the `record` word at a connected moment.
pub fn record_status_word(s: OutputStatus) -> RecordState {
    match (s.active, s.paused) {
        (true, true) => RecordState::Paused,
        (true, false) => RecordState::Recording,
        (false, _) => RecordState::Off,
    }
}

/// D15: a version string is kept only up to this many characters.
const MAX_VERSION_CHARS: usize = 32;

/// D15: a version OBS sent, kept only if it is at most 32 characters of
/// `[0-9A-Za-z.+-]` (and not empty): no other text from OBS reaches the
/// status.
pub fn sanitize_version(s: &str) -> Option<String> {
    let ok = !s.is_empty()
        && s.len() <= MAX_VERSION_CHARS
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'+' | b'-'));
    ok.then(|| s.to_string())
}

/// D15: scene events mark the inventory dirty; `inventoryRev` goes up at
/// most once per `period`, and no mark is lost.
#[derive(Clone, Copy, Debug)]
pub struct RevCoalesce {
    dirty: bool,
    last: Option<Instant>,
    period: Duration,
}

impl RevCoalesce {
    /// Clean, never bumped.
    #[must_use]
    pub fn new(period: Duration) -> Self {
        Self {
            dirty: false,
            last: None,
            period,
        }
    }

    /// A scene was created, removed or renamed.
    pub fn mark(&mut self) {
        self.dirty = true;
    }

    /// When the next bump may happen, while dirty: `now` if the period since
    /// the last one has passed (or there was none), else when it does.
    pub fn due_at(&self, now: Instant) -> Option<Instant> {
        if !self.dirty {
            return None;
        }
        Some(self.next_allowed().filter(|&at| at > now).unwrap_or(now))
    }

    /// Whether to bump `inventoryRev` now (dirty, and the period since the
    /// last bump has passed); clears the mark when it does. A mark made
    /// before the period is up stays for the next call after it.
    pub fn take(&mut self, now: Instant) -> bool {
        if !self.dirty || self.next_allowed().is_some_and(|at| at > now) {
            return false;
        }
        self.dirty = false;
        self.last = Some(now);
        true
    }

    /// The last bump plus the period; `None` before any.
    fn next_allowed(&self) -> Option<Instant> {
        self.last.map(|l| l.checked_add(self.period).unwrap_or(l))
    }
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
        backoff, backoff_ms, connect_reason, follow_op, join_seed, next_push, parse_host,
        record_status_word, record_word, request_failure, sanitize_version, seed_pairs,
        stream_status_word, stream_word, Failure, FollowAction, HostError, LastSkip, LinkConfig,
        ObsConfig, ObsInventory, ObsReason, ObsState, ObsStatus, OutputStatus, Pushed, RecordState,
        RevCoalesce, SkipOp, SkipWhy, StatusCell, StreamState, TextKind, TextSkip, TextSkipWhy,
        TextSource, Work, BACKOFF_CAP_MS, NOT_READY,
    };
    use crate::obs::proto::{ObsEvent, OutputState, RequestKind, REQUEST_TYPES};
    use crate::obs::scenes::SceneRef;
    use crate::obs::LinkTiming;
    use crate::timers::{Follow, FollowObs, ObsFeed, ObsText, TimerOp};
    use serde::Serialize;
    use serde_json::{json, Value};
    use std::collections::HashSet;
    use std::time::{Duration, Instant};

    fn fixture() -> Value {
        serde_json::from_str(include_str!("../../../src/data/obs.fixture.json")).unwrap()
    }

    fn sorted_keys(v: &Value) -> Vec<String> {
        let mut keys: Vec<String> = v.as_object().unwrap().keys().cloned().collect();
        keys.sort();
        keys
    }

    fn strings(v: &Value) -> Vec<String> {
        v.as_array()
            .unwrap()
            .iter()
            .map(|s| s.as_str().unwrap().to_string())
            .collect()
    }

    /// Each of `all` as its wire word, in order.
    fn words<T: Serialize>(all: &[T]) -> Vec<String> {
        all.iter()
            .map(|w| {
                serde_json::to_value(w)
                    .unwrap()
                    .as_str()
                    .unwrap()
                    .to_string()
            })
            .collect()
    }

    /// The matches make a new variant fail to compile here until it has its
    /// place in `ALL` (and its fixture word).
    #[test]
    fn every_word_is_listed() {
        for (i, s) in ObsState::ALL.iter().enumerate() {
            let at = match s {
                ObsState::Off => 0,
                ObsState::Connecting => 1,
                ObsState::Connected => 2,
                ObsState::Retrying => 3,
                ObsState::NeedsPassword => 4,
                ObsState::AuthFailed => 5,
                ObsState::PasswordUnreadable => 6,
                ObsState::TooOld => 7,
                ObsState::NeedsNewerDivora => 8,
                ObsState::Kicked => 9,
                ObsState::Unsupported => 10,
            };
            assert_eq!(at, i, "{s:?}");
        }
        for (i, r) in ObsReason::ALL.iter().enumerate() {
            let at = match r {
                ObsReason::Refused => 0,
                ObsReason::Unreachable => 1,
                ObsReason::NoAnswer => 2,
                ObsReason::NotObs => 3,
                ObsReason::NotReady => 4,
                ObsReason::Closed => 5,
                ObsReason::ObsExiting => 6,
                ObsReason::Silent => 7,
                ObsReason::TooBig => 8,
                ObsReason::Protocol => 9,
                ObsReason::BadVersion => 10,
                ObsReason::MissingRequest => 11,
            };
            assert_eq!(at, i, "{r:?}");
        }
        for (i, w) in SkipWhy::ALL.iter().enumerate() {
            let at = match w {
                SkipWhy::NotConnected => 0,
                SkipWhy::QueueFull => 1,
                SkipWhy::NotFound => 2,
                SkipWhy::NotReady => 3,
                SkipWhy::TimedOut => 4,
                SkipWhy::Failed => 5,
                SkipWhy::Late => 6,
            };
            assert_eq!(at, i, "{w:?}");
        }
        for (i, s) in StreamState::ALL.iter().enumerate() {
            let at = match s {
                StreamState::Off => 0,
                StreamState::Starting => 1,
                StreamState::Live => 2,
                StreamState::Reconnecting => 3,
                StreamState::Stopping => 4,
            };
            assert_eq!(at, i, "{s:?}");
        }
        for (i, s) in RecordState::ALL.iter().enumerate() {
            let at = match s {
                RecordState::Off => 0,
                RecordState::Starting => 1,
                RecordState::Recording => 2,
                RecordState::Paused => 3,
                RecordState::Stopping => 4,
            };
            assert_eq!(at, i, "{s:?}");
        }
        let two = |a: bool| usize::from(!a);
        for (i, o) in SkipOp::ALL.iter().enumerate() {
            assert_eq!(two(matches!(o, SkipOp::Scene)), i, "{o:?}");
        }
        for (i, w) in TextSkipWhy::ALL.iter().enumerate() {
            assert_eq!(two(matches!(w, TextSkipWhy::Failed)), i, "{w:?}");
        }
        for (i, k) in TextKind::ALL.iter().enumerate() {
            assert_eq!(two(matches!(k, TextKind::Gdiplus)), i, "{k:?}");
        }
    }

    /// §4: the config's keys are the fixture's `configKeys`, it reads as the
    /// page sends it, and a port that isn't a `u16` fails the whole invoke
    /// (Tauri's reader, as `serde_json`'s here).
    #[test]
    fn obs_config_keys_are_frozen() {
        let f = fixture();
        let sent = json!({"enabled": true, "host": "127.0.0.1", "port": 4455});
        assert_eq!(sorted_keys(&sent), strings(&f["configKeys"]));
        let read: ObsConfig = serde_json::from_value(sent.clone()).unwrap();
        assert_eq!(
            read,
            ObsConfig {
                enabled: true,
                host: "127.0.0.1".into(),
                port: 4455,
            }
        );
        assert_eq!(serde_json::to_value(&read).unwrap(), sent);
        for port in [json!(65_536), json!(4455.5), json!("4455"), json!(-1)] {
            let bad = json!({"enabled": true, "host": "127.0.0.1", "port": port});
            assert!(serde_json::from_value::<ObsConfig>(bad).is_err(), "{port}");
        }
        // Port 0 reads: `obs_configure` refuses it with its message.
        let zero = json!({"enabled": false, "host": "", "port": 0});
        assert!(serde_json::from_value::<ObsConfig>(zero).is_ok());
    }

    /// The fixture's `obsStatusSamples`, built here: each row's status is
    /// [`ObsStatus::off`] with the fields its story changes, `seq` its index.
    #[allow(clippy::too_many_lines)] // a row per sample, in the fixture's order
    fn samples() -> Vec<(&'static str, ObsStatus)> {
        let at = |seq: u64, f: &dyn Fn(&mut ObsStatus)| {
            let mut s = ObsStatus::off();
            s.seq = seq;
            f(&mut s);
            s
        };
        // Configured for OBS on this PC, a password saved.
        let local = |s: &mut ObsStatus| {
            s.host = "127.0.0.1".into();
            s.port = 4455;
            s.loopback = true;
            s.has_password = true;
        };
        // What the last Hello and GetVersion said.
        let hello = |s: &mut ObsStatus, version: Option<&str>| {
            s.auth_required = Some(true);
            s.websocket_version = version.map(str::to_string);
        };
        let known = |s: &mut ObsStatus| {
            hello(s, Some("5.5.2"));
            s.obs_version = Some("30.2.2".into());
            s.inventory_rev = 1;
        };
        let connected = |s: &mut ObsStatus| {
            local(s);
            known(s);
            s.state = ObsState::Connected;
            s.stream = Some(StreamState::Off);
            s.record = Some(RecordState::Off);
        };
        let retrying = |s: &mut ObsStatus, reason: ObsReason, retry_at_ms: i64| {
            local(s);
            s.state = ObsState::Retrying;
            s.reason = Some(reason);
            s.retry_at_ms = Some(retry_at_ms);
        };
        let stopped = |s: &mut ObsStatus, state: ObsState, version: Option<&str>| {
            local(s);
            hello(s, version);
            s.state = state;
        };
        vec![
            ("off, never configured: ObsStatus::off()", at(0, &|_| {})),
            (
                "off, configured with enabled:false, a password saved",
                at(1, &local),
            ),
            (
                "connecting, the first attempt",
                at(2, &|s| {
                    local(s);
                    s.state = ObsState::Connecting;
                }),
            ),
            ("connected", at(3, &connected)),
            (
                "connected, live and recording, a source missing, a scene skip and a text skip",
                at(4, &|s| {
                    connected(s);
                    s.stream = Some(StreamState::Live);
                    s.record = Some(RecordState::Recording);
                    s.missing = vec!["a7e2c94b-1f3d-4b8a-9c6e-0d5f7b2e4a91".into()];
                    s.last_skip = Some(LastSkip {
                        op: SkipOp::Scene,
                        why: SkipWhy::NotFound,
                        name: Some("BRB".into()),
                        timer: "timer-1".into(),
                        at_ms: 1_793_217_840_000,
                    });
                    s.text_skip = Some(TextSkip {
                        why: TextSkipWhy::Failed,
                        at_ms: 1_793_217_845_000,
                    });
                    s.inventory_rev = 3;
                }),
            ),
            (
                "connected, OBS is closing (ExitStarted)",
                at(5, &|s| {
                    connected(s);
                    s.reason = Some(ObsReason::ObsExiting);
                }),
            ),
            (
                "retrying, refused; a switch back ran out of time",
                at(6, &|s| {
                    retrying(s, ObsReason::Refused, 1_793_217_900_800);
                    s.last_skip = Some(LastSkip {
                        op: SkipOp::SceneBack,
                        why: SkipWhy::NotConnected,
                        name: None,
                        timer: "timer-1".into(),
                        at_ms: 1_793_217_840_000,
                    });
                }),
            ),
            (
                "retrying, unreachable, OBS on another PC",
                at(7, &|s| {
                    retrying(s, ObsReason::Unreachable, 1_793_217_901_600);
                    s.host = "192.168.1.20".into();
                    s.loopback = false;
                }),
            ),
            (
                "retrying, noAnswer",
                at(8, &|s| retrying(s, ObsReason::NoAnswer, 1_793_217_903_200)),
            ),
            (
                "retrying, notObs",
                at(9, &|s| retrying(s, ObsReason::NotObs, 1_793_217_906_400)),
            ),
            (
                "retrying, notReady",
                at(10, &|s| {
                    retrying(s, ObsReason::NotReady, 1_793_217_912_000);
                    hello(s, Some("5.5.2"));
                }),
            ),
            (
                "retrying, closed; a switch back held back",
                at(11, &|s| {
                    retrying(s, ObsReason::Closed, 1_793_217_901_000);
                    known(s);
                    s.held_back = 1;
                }),
            ),
            (
                "retrying, obsExiting",
                at(12, &|s| {
                    retrying(s, ObsReason::ObsExiting, 1_793_217_901_000);
                    known(s);
                }),
            ),
            (
                "retrying, silent; a text push timed out",
                at(13, &|s| {
                    retrying(s, ObsReason::Silent, 1_793_217_901_000);
                    known(s);
                    s.text_skip = Some(TextSkip {
                        why: TextSkipWhy::TimedOut,
                        at_ms: 1_793_217_870_000,
                    });
                }),
            ),
            (
                "retrying, tooBig",
                at(14, &|s| {
                    retrying(s, ObsReason::TooBig, 1_793_217_901_000);
                    known(s);
                }),
            ),
            (
                "retrying, protocol",
                at(15, &|s| {
                    retrying(s, ObsReason::Protocol, 1_793_217_901_000);
                    known(s);
                }),
            ),
            (
                "needsPassword",
                at(16, &|s| {
                    stopped(s, ObsState::NeedsPassword, Some("5.5.2"));
                    s.has_password = false;
                }),
            ),
            (
                "authFailed",
                at(17, &|s| stopped(s, ObsState::AuthFailed, Some("5.5.2"))),
            ),
            (
                "passwordUnreadable",
                at(18, &|s| {
                    stopped(s, ObsState::PasswordUnreadable, Some("5.5.2"));
                }),
            ),
            (
                "tooOld",
                at(19, &|s| stopped(s, ObsState::TooOld, Some("5.3.3"))),
            ),
            (
                "needsNewerDivora",
                at(20, &|s| {
                    stopped(s, ObsState::NeedsNewerDivora, Some("6.0.0"));
                }),
            ),
            (
                "kicked",
                at(21, &|s| {
                    stopped(s, ObsState::Kicked, Some("5.5.2"));
                    known(s);
                }),
            ),
            (
                "unsupported, badVersion",
                at(22, &|s| {
                    stopped(s, ObsState::Unsupported, None);
                    s.reason = Some(ObsReason::BadVersion);
                }),
            ),
            (
                "unsupported, missingRequest",
                at(23, &|s| {
                    stopped(s, ObsState::Unsupported, Some("5.5.2"));
                    known(s);
                    s.inventory_rev = 0;
                    s.reason = Some(ObsReason::MissingRequest);
                    s.detail = Some("GetInputSettings");
                }),
            ),
        ]
    }

    /// §4: a status's keys are the fixture's `statusKeys` (19), a skip's its
    /// `lastSkipKeys` and `textSkipKeys`; the states, reasons and words are
    /// its lists, in order; and `ObsStatus::off()` is its first sample.
    #[test]
    fn obs_status_keys_are_frozen() {
        let f = fixture();
        let off = serde_json::to_value(ObsStatus::off()).unwrap();
        assert_eq!(sorted_keys(&off), strings(&f["statusKeys"]));
        assert_eq!(strings(&f["statusKeys"]).len(), 19);
        assert_eq!(off, f["obsStatusSamples"][0]["status"]);
        // Nothing is left out when absent: every key is there, `null` or not.
        let all = samples();
        let full = serde_json::to_value(&all[4].1).unwrap();
        assert_eq!(sorted_keys(&full), strings(&f["statusKeys"]));
        assert_eq!(sorted_keys(&full["lastSkip"]), strings(&f["lastSkipKeys"]));
        assert_eq!(sorted_keys(&full["textSkip"]), strings(&f["textSkipKeys"]));
        assert_eq!(words(&ObsState::ALL), strings(&f["states"]));
        assert_eq!(words(&ObsReason::ALL), strings(&f["reasons"]));
        assert_eq!(words(&SkipOp::ALL), strings(&f["skipOps"]));
        assert_eq!(words(&SkipWhy::ALL), strings(&f["skipWhys"]));
        assert_eq!(words(&TextSkipWhy::ALL), strings(&f["textSkipWhys"]));
        assert_eq!(words(&StreamState::ALL), strings(&f["streamStates"]));
        assert_eq!(words(&RecordState::ALL), strings(&f["recordStates"]));
        // A switch back's whys are scene's less the two that make it due.
        let back = strings(&f["skipWhysByOp"]["sceneBack"]);
        let scene = strings(&f["skipWhysByOp"]["scene"]);
        assert_eq!(scene, words(&SkipWhy::ALL));
        let mut want = words(&SkipWhy::ALL);
        want.retain(|w| w != "queueFull" && w != "late");
        assert_eq!(back, want);
    }

    /// Seam 6: the fixture's `obsStatusSamples` (the page's `obsStatusLine`
    /// and the e2e mock read them) are Rust's statuses, byte for byte, by
    /// name and in order; together they hold every state and every reason.
    #[test]
    fn obs_status_samples_are_rusts() {
        let f = fixture();
        let rows = f["obsStatusSamples"].as_array().unwrap();
        let ours = samples();
        assert_eq!(rows.len(), ours.len());
        for (row, (name, status)) in rows.iter().zip(&ours) {
            assert_eq!(row["name"], *name);
            assert_eq!(
                serde_json::to_value(status).unwrap(),
                row["status"],
                "{name}"
            );
        }
        for state in ObsState::ALL {
            assert!(ours.iter().any(|(_, s)| s.state == state), "{state:?}");
        }
        for reason in ObsReason::ALL {
            assert!(
                ours.iter().any(|(_, s)| s.reason == Some(reason)),
                "{reason:?}"
            );
        }
    }

    /// §4: an inventory's keys, a scene's and a text source's are the
    /// fixture's `inventoryKeys`, `sceneKeys` and `textSourceKeys`; a text
    /// source's kinds its `textSourceKinds`.
    #[test]
    fn obs_inventory_keys_are_frozen() {
        let f = fixture();
        let inv = ObsInventory {
            rev: 2,
            scenes: vec![SceneRef {
                uuid: "5d3f1c9e-7a2b-4e6d-8f10-2c4b6a8e0d13".into(),
                name: "BRB".into(),
            }],
            text_sources: vec![TextSource {
                uuid: "a7e2c94b-1f3d-4b8a-9c6e-0d5f7b2e4a91".into(),
                name: "Timer".into(),
                kind: TextKind::Gdiplus,
                reads_from_file: false,
            }],
            program_scene_uuid: None,
            truncated: false,
        };
        let v = serde_json::to_value(&inv).unwrap();
        assert_eq!(sorted_keys(&v), strings(&f["inventoryKeys"]));
        assert_eq!(sorted_keys(&v["scenes"][0]), strings(&f["sceneKeys"]));
        assert_eq!(
            sorted_keys(&v["textSources"][0]),
            strings(&f["textSourceKeys"])
        );
        assert_eq!(v["programSceneUuid"], Value::Null);
        assert_eq!(words(&TextKind::ALL), strings(&f["textSourceKinds"]));
        // An empty one too: every key there.
        let empty = serde_json::to_value(ObsInventory::default()).unwrap();
        assert_eq!(sorted_keys(&empty), strings(&f["inventoryKeys"]));
    }

    /// §4: a refused address's message, byte for byte, by the fixture's
    /// `hostErrors` code, which is the error's own wire name.
    #[test]
    fn host_errors_are_the_fixtures() {
        let f = fixture();
        let errors = f["hostErrors"].as_object().unwrap();
        assert_eq!(errors.len(), 2);
        for e in [HostError::NotAnIp, HostError::LinkLocalV6] {
            let code = serde_json::to_value(e).unwrap();
            assert_eq!(errors[code.as_str().unwrap()], e.to_string(), "{e:?}");
        }
    }

    /// §7.3: each row of the table, and its edges: 0 failures waits as 1,
    /// `r` outside [0, 1) is clamped (not a number: the middle), and the
    /// unit scales it for §10.2's tests.
    #[test]
    fn backoff_table() {
        let near_one = 1.0 - f64::EPSILON;
        for (n, at_0, at_half, at_1) in [
            (1, 800, 1_000, 1_200),
            (2, 1_600, 2_000, 2_400),
            (3, 3_200, 4_000, 4_800),
            (4, 6_400, 8_000, 9_600),
            (5, 12_000, 15_000, 18_000),
            (6, 24_000, 30_000, 30_000),
            (7, 24_000, 30_000, 30_000),
            (1_000, 24_000, 30_000, 30_000),
            (u32::MAX, 24_000, 30_000, 30_000),
            (0, 800, 1_000, 1_200),
        ] {
            assert_eq!(backoff_ms(n, 0.0), at_0, "n {n}, r 0");
            assert_eq!(backoff_ms(n, 0.5), at_half, "n {n}, r 0.5");
            assert_eq!(backoff_ms(n, near_one), at_1, "n {n}, r → 1");
        }
        assert_eq!(backoff_ms(1, -3.0), 800);
        assert_eq!(backoff_ms(1, 7.0), 1_200);
        assert_eq!(backoff_ms(1, f64::NAN), 1_000);
        assert_eq!(backoff_ms(1, f64::INFINITY), 1_200);
        assert_eq!(backoff_ms(5, 0.25), 13_500);
        // In units: 1 s is the ms themselves; §10.2's 200 ms unit a fifth.
        assert_eq!(
            backoff(3, 0.5, Duration::from_secs(1)),
            Duration::from_millis(4_000)
        );
        assert_eq!(
            backoff(1, 0.0, Duration::from_millis(200)),
            Duration::from_millis(160)
        );
        assert_eq!(
            backoff(9, 0.99, Duration::from_millis(200)),
            Duration::from_millis(6_000)
        );
    }

    /// D14, W5: the cap applies after the jitter, so no wait passes 30 s,
    /// and none is shorter than 0.8 s, for any failure count and draw.
    #[test]
    fn backoff_never_passes_30_s() {
        let mut longest = 0;
        for n in (0..=40).chain([u32::MAX - 1, u32::MAX]) {
            for i in 0..=1_000 {
                let r = f64::from(i) / 1_000.0;
                for r in [r, (r - f64::EPSILON).max(0.0)] {
                    let ms = backoff_ms(n, r);
                    assert!(ms <= BACKOFF_CAP_MS, "n {n}, r {r}: {ms}");
                    assert!(ms >= 800, "n {n}, r {r}: {ms}");
                    longest = longest.max(ms);
                }
            }
        }
        assert_eq!(longest, 30_000);
        assert_eq!(BACKOFF_CAP_MS, 30_000);
    }

    /// §7.4 (fixture `hostSamples`): every typed address, its canonical IP
    /// and loopback, or the error whose message `obs_configure` answers.
    #[test]
    fn host_table() {
        let f = fixture();
        let rows = f["hostSamples"].as_array().unwrap();
        assert_eq!(rows.len(), 32);
        for row in rows {
            let typed = row["typed"].as_str().unwrap();
            match parse_host(typed) {
                Ok(ip) => {
                    assert_eq!(ip.to_string(), row["ip"], "{typed:?}");
                    let cfg = LinkConfig { ip, port: 4455 };
                    assert_eq!(cfg.loopback(), row["loopback"], "{typed:?}");
                    assert_eq!(row["error"], Value::Null, "{typed:?}");
                }
                Err(e) => {
                    assert_eq!(serde_json::to_value(e).unwrap(), row["error"], "{typed:?}");
                    assert_eq!(row["ip"], Value::Null, "{typed:?}");
                    assert_eq!(
                        f["hostErrors"][row["error"].as_str().unwrap()],
                        e.to_string(),
                        "{typed:?}"
                    );
                }
            }
        }
        // Past the table: one pair of brackets only, a zone id anywhere,
        // the rest of fe80::/10, other whitespace.
        for (typed, want) in [
            ("[[::1]]", Err(HostError::NotAnIp)),
            ("[::1", Err(HostError::NotAnIp)),
            ("::1]", Err(HostError::NotAnIp)),
            ("[127.0.0.1]", Ok("127.0.0.1")),
            ("\t::1\n", Ok("::1")),
            ("localhost.", Err(HostError::NotAnIp)),
            ("[localhost]", Err(HostError::NotAnIp)),
            ("127.0.0.1%3", Err(HostError::LinkLocalV6)),
            ("febf::1", Err(HostError::LinkLocalV6)),
            ("fec0::1", Ok("fec0::1")),
            ("FE80::1", Err(HostError::LinkLocalV6)),
            ("2001:DB8::1", Ok("2001:db8::1")),
            ("255.255.255.254", Ok("255.255.255.254")),
            ("239.255.255.250", Err(HostError::NotAnIp)),
        ] {
            let got = parse_host(typed).map(|ip| ip.to_string());
            assert_eq!(got.as_deref().map_err(|e| *e), want, "{typed:?}");
        }
        // The upgrade's URI brackets IPv6; the request timeout follows
        // loopback (D8).
        let t = LinkTiming::production();
        let local = LinkConfig {
            ip: parse_host("localhost").unwrap(),
            port: 4455,
        };
        assert_eq!(local.uri(), "ws://127.0.0.1:4455/");
        assert_eq!(local.request_timeout(&t), Duration::from_secs(3));
        let v6 = LinkConfig {
            ip: parse_host("[::1]").unwrap(),
            port: 4455,
        };
        assert_eq!(v6.uri(), "ws://[::1]:4455/");
        assert_eq!(v6.request_timeout(&t), Duration::from_secs(3));
        let lan = LinkConfig {
            ip: parse_host("::ffff:192.168.1.20").unwrap(),
            port: 4456,
        };
        assert_eq!(lan.uri(), "ws://192.168.1.20:4456/");
        assert_eq!(lan.request_timeout(&t), Duration::from_secs(5));
    }

    /// §7.1's connect rows: refused is `refused`; a timeout, an unreachable
    /// host or network, and every other kind are `unreachable` — never
    /// `closed`, since nothing was open.
    #[test]
    fn connect_error_table() {
        use std::io::ErrorKind;
        assert_eq!(
            connect_reason(ErrorKind::ConnectionRefused),
            ObsReason::Refused
        );
        for kind in [
            ErrorKind::TimedOut,
            ErrorKind::HostUnreachable,
            ErrorKind::NetworkUnreachable,
            ErrorKind::AddrNotAvailable,
            ErrorKind::PermissionDenied,
            ErrorKind::ConnectionReset,
            ErrorKind::ConnectionAborted,
            ErrorKind::InvalidInput,
            ErrorKind::NetworkDown,
            ErrorKind::UnexpectedEof,
            ErrorKind::Interrupted,
            ErrorKind::Other,
        ] {
            assert_eq!(connect_reason(kind), ObsReason::Unreachable, "{kind:?}");
        }
    }

    /// A link config for `host`.
    fn cfg(host: &str, port: u16) -> LinkConfig {
        LinkConfig {
            ip: parse_host(host).unwrap(),
            port,
        }
    }

    /// D15, D4: a write that changes nothing emits nothing and keeps `seq`;
    /// one that changes something bumps it by one; `f` can't move `seq`; a
    /// driver's write with a stale (or a future) generation is ignored,
    /// while `ObsShared`'s own writes always land.
    #[test]
    fn status_cell_bumps_seq_only_on_change_and_ignores_stale_generations() {
        let mut cell = StatusCell::new();
        assert_eq!((cell.gen(), cell.status()), (0, &ObsStatus::off()));
        assert_eq!(cell.write_shared(|_| {}), None);
        assert_eq!(cell.write_shared(|s| s.seq = 99), None);
        assert_eq!(cell.status().seq, 0);

        let saved = cell.write_shared(|s| s.has_password = true).unwrap();
        assert_eq!((saved.seq, saved.has_password), (1, true));
        assert_eq!(cell.status(), &saved);
        assert_eq!(cell.write_shared(|s| s.has_password = true), None);
        assert_eq!(cell.status().seq, 1);

        // Generation 0's writes land until a link begins.
        let at0 = cell.write(0, |s| s.held_back = 2).unwrap();
        assert_eq!(at0.seq, 2);
        let (gen, began) = cell.begin(&cfg("127.0.0.1", 4455));
        assert_eq!(gen, 1);
        assert_eq!(began.unwrap().seq, 3);
        let before = cell.status().clone();
        assert_eq!(cell.write(0, |s| s.state = ObsState::Retrying), None);
        assert_eq!(cell.write(2, |s| s.state = ObsState::Retrying), None);
        assert_eq!(cell.status(), &before);

        let retrying = cell
            .write(1, |s| {
                s.state = ObsState::Retrying;
                s.reason = Some(ObsReason::Refused);
            })
            .unwrap();
        assert_eq!((retrying.seq, retrying.state), (4, ObsState::Retrying));
        // The same write again changes nothing.
        assert_eq!(
            cell.write(1, |s| {
                s.state = ObsState::Retrying;
                s.reason = Some(ObsReason::Refused);
            }),
            None
        );

        // A newer link: generation 1 is stale from now on, and a skip from
        // `ObsShared` still lands.
        let (gen, _) = cell.begin(&cfg("127.0.0.1", 4455));
        assert_eq!(gen, 2);
        let seq = cell.status().seq;
        assert_eq!(cell.write(1, |s| s.state = ObsState::Connected), None);
        assert_eq!(cell.status().state, ObsState::Connecting);
        let skipped = cell.write_shared(|s| s.held_back = 1).unwrap();
        assert_eq!(skipped.seq, seq + 1);
        // Every emit is newer than the last.
        let next = cell.write(2, |s| s.state = ObsState::Connected).unwrap();
        assert_eq!(next.seq, seq + 2);
    }

    /// D4: `begin` moves the generation and writes `connecting` in one step,
    /// so a driver spawned with the generation it returns can write at once
    /// and keep what it wrote, and the stopped driver's late write is
    /// ignored. What the last connection said goes; what `ObsShared` keeps
    /// stays; versions stay only for the same address.
    #[test]
    fn begin_writes_connecting_before_the_driver_can() {
        let mut cell = StatusCell::new();
        let (old, _) = cell.begin(&cfg("127.0.0.1", 4455));
        cell.write(old, |s| {
            s.state = ObsState::AuthFailed;
            s.auth_required = Some(true);
            s.websocket_version = Some("5.5.2".into());
            s.obs_version = Some("30.2.2".into());
            s.reason = Some(ObsReason::MissingRequest);
            s.detail = Some("GetInputSettings");
            s.retry_at_ms = Some(1);
            s.stream = Some(StreamState::Live);
            s.record = Some(RecordState::Paused);
            s.missing = vec!["u-1".into()];
            s.has_password = true;
            s.held_back = 1;
            s.inventory_rev = 3;
            s.last_skip = Some(LastSkip {
                op: SkipOp::Scene,
                why: SkipWhy::Late,
                name: Some("BRB".into()),
                timer: "timer-1".into(),
                at_ms: 5,
            });
            s.text_skip = Some(TextSkip {
                why: TextSkipWhy::Failed,
                at_ms: 6,
            });
        })
        .unwrap();
        let kept = cell.status().clone();

        // A password saved: the same address starts again.
        let (gen, began) = cell.begin(&cfg("127.0.0.1", 4455));
        assert_eq!(gen, old + 1);
        let began = began.unwrap();
        assert_eq!(began.seq, kept.seq + 1);
        let mut want = kept.clone();
        want.seq = began.seq;
        want.state = ObsState::Connecting;
        want.reason = None;
        want.detail = None;
        want.retry_at_ms = None;
        want.stream = None;
        want.record = None;
        want.missing = Vec::new();
        assert_eq!(began, want);
        assert_eq!(cell.status(), &want);

        // The stopped driver's late write lands nowhere; the new one's
        // first write is kept, and nothing puts `connecting` back over it.
        assert_eq!(cell.write(old, |s| s.state = ObsState::Retrying), None);
        let connected = cell.write(gen, |s| s.state = ObsState::Connected).unwrap();
        assert_eq!(connected.state, ObsState::Connected);
        assert_eq!(cell.status(), &connected);

        // A new address: what the last OBS said goes with it.
        let (newer, began) = cell.begin(&cfg("192.168.1.20", 4455));
        assert_eq!(newer, gen + 1);
        let began = began.unwrap();
        assert_eq!(
            (
                began.state,
                began.host.as_str(),
                began.port,
                began.loopback,
                began.auth_required,
                began.websocket_version.as_deref(),
                began.obs_version.as_deref()
            ),
            (
                ObsState::Connecting,
                "192.168.1.20",
                4455,
                false,
                None,
                None,
                None
            )
        );
        assert_eq!(
            (began.has_password, began.held_back, began.inventory_rev),
            (true, 1, 3)
        );
        assert!(began.last_skip.is_some() && began.text_skip.is_some());
        // A new port is a new address too; the host is the canonical one.
        let (_, began) = cell.begin(&cfg("::ffff:192.168.1.20", 4456));
        assert_eq!(began.unwrap().host, "192.168.1.20");

        // Already connecting there: the generation still moves, nothing is
        // emitted.
        let seq = cell.status().seq;
        let (again, none) = cell.begin(&cfg("192.168.1.20", 4456));
        assert_eq!((again, none), (newer + 2, None));
        assert_eq!(cell.status().seq, seq);
    }

    /// The fixture's `follows`/`event` word.
    fn follow_of(v: &Value) -> FollowObs {
        serde_json::from_value(v.clone()).unwrap()
    }

    /// §7.7's first table (fixture `followTable`), and the rows it leaves
    /// out: a stream's PAUSED and RESUMED and a recording's RECONNECTING
    /// and RECONNECTED, which OBS never sends, do nothing. A STARTED counts
    /// from when the event was read.
    #[test]
    fn follow_table() {
        let f = fixture();
        let rows = f["followTable"].as_array().unwrap();
        assert_eq!(rows.len(), 23);
        for row in rows {
            let what = follow_of(&row["follows"]);
            let state = OutputState::from_wire(row["outputState"].as_str().unwrap());
            let ev = match row["event"].as_str().unwrap() {
                "stream" => ObsEvent::Stream(state),
                "record" => ObsEvent::Record(state),
                other => panic!("event {other}"),
            };
            let want = match row["action"].as_str() {
                None => None,
                Some("setElapsed") => Some(FollowAction::Op(TimerOp::SetElapsed {
                    ms: 1_234,
                    running: true,
                })),
                Some("pause") => Some(FollowAction::Op(TimerOp::Pause)),
                Some("resume") => Some(FollowAction::Op(TimerOp::Resume)),
                Some("seed") => Some(FollowAction::Seed),
                Some(other) => panic!("action {other}"),
            };
            assert_eq!(follow_op(what, &ev, 1_234), want, "{row}");
        }
        // Left out of the fixture, decided here.
        for ev in [
            ObsEvent::Stream(OutputState::Paused),
            ObsEvent::Stream(OutputState::Resumed),
            ObsEvent::Record(OutputState::Reconnecting),
            ObsEvent::Record(OutputState::Reconnected),
            ObsEvent::ExitStarted,
            ObsEvent::SceneCreated,
            ObsEvent::SceneRemoved,
            ObsEvent::SceneNameChanged,
        ] {
            for what in [FollowObs::Stream, FollowObs::Record] {
                assert_eq!(follow_op(what, &ev, 5), None, "{what:?} {ev:?}");
            }
        }
        assert_eq!(
            follow_op(
                FollowObs::Record,
                &ObsEvent::Record(OutputState::Started),
                0
            ),
            Some(FollowAction::Op(TimerOp::SetElapsed {
                ms: 0,
                running: true
            }))
        );
    }

    /// §7.7's second table (fixture `seedTable`): a reconnecting stream sets
    /// nothing whatever `outputActive` says; a paused recording is set
    /// paused; an inactive output pauses only a running timer.
    #[test]
    fn seed_table() {
        let f = fixture();
        let rows = f["seedTable"].as_array().unwrap();
        assert_eq!(rows.len(), 20);
        for row in rows {
            let what = follow_of(&row["follows"]);
            let s = &row["status"];
            let status = OutputStatus {
                active: s["outputActive"].as_bool().unwrap(),
                reconnecting: s["outputReconnecting"].as_bool().unwrap_or(false),
                paused: s["outputPaused"].as_bool().unwrap_or(false),
                duration_ms: s["outputDuration"].as_u64().unwrap(),
            };
            let running_now = match row["timerNow"].as_str().unwrap() {
                "running" => true,
                "idle" | "paused" => false,
                other => panic!("timerNow {other}"),
            };
            let want = match row["op"]["op"].as_str() {
                None => None,
                Some("setElapsed") => Some(TimerOp::SetElapsed {
                    ms: row["op"]["ms"].as_u64().unwrap(),
                    running: row["op"]["running"].as_bool().unwrap(),
                }),
                Some("pause") => Some(TimerOp::Pause),
                Some(other) => panic!("op {other}"),
            };
            assert_eq!(join_seed(what, status, running_now), want, "{row}");
        }
        // A stream's `paused` and a recording's `reconnecting` aren't read.
        let odd = OutputStatus {
            active: true,
            reconnecting: true,
            paused: true,
            duration_ms: 9,
        };
        assert_eq!(
            join_seed(FollowObs::Record, odd, false),
            Some(TimerOp::SetElapsed {
                ms: 9,
                running: false
            })
        );
        let odd = OutputStatus {
            reconnecting: false,
            ..odd
        };
        assert_eq!(
            join_seed(FollowObs::Stream, odd, false),
            Some(TimerOp::SetElapsed {
                ms: 9,
                running: true
            })
        );
    }

    /// A feed of `follows` and `sleeps`.
    fn feed(follows: &[(&str, FollowObs)], sleeps: u64) -> ObsFeed {
        ObsFeed {
            texts: Vec::new(),
            follows: follows
                .iter()
                .map(|(id, what)| Follow {
                    id: (*id).to_string(),
                    what: *what,
                })
                .collect(),
            sleeps,
        }
    }

    /// D26: a follow new in the feed, or switched between Streaming and
    /// Recording, is seeded; a feed whose sleep count changed seeds every
    /// follow (the PC slept with OBS still on); otherwise nothing.
    #[test]
    fn seed_pairs_table() {
        use FollowObs::{Record, Stream};
        let pairs = |v: &[(&str, FollowObs)]| -> Vec<(String, FollowObs)> {
            v.iter().map(|(id, w)| ((*id).to_string(), *w)).collect()
        };
        for (name, prev, next, want) in [
            (
                "a new follow",
                feed(&[], 0),
                feed(&[("a", Stream)], 0),
                pairs(&[("a", Stream)]),
            ),
            (
                "unchanged",
                feed(&[("a", Stream)], 0),
                feed(&[("a", Stream)], 0),
                pairs(&[]),
            ),
            (
                "switched to Recording",
                feed(&[("a", Stream)], 0),
                feed(&[("a", Record)], 0),
                pairs(&[("a", Record)]),
            ),
            (
                "one added among others, in the feed's order",
                feed(&[("a", Stream), ("b", Record)], 3),
                feed(&[("c", Stream), ("a", Stream), ("b", Record)], 3),
                pairs(&[("c", Stream)]),
            ),
            (
                "one dropped",
                feed(&[("a", Stream), ("b", Record)], 0),
                feed(&[("b", Record)], 0),
                pairs(&[]),
            ),
            (
                "a sleep: every follow",
                feed(&[("a", Stream), ("b", Record)], 0),
                feed(&[("a", Stream), ("b", Record)], 1),
                pairs(&[("a", Stream), ("b", Record)]),
            ),
            (
                "a sleep with nothing following",
                feed(&[], 4),
                feed(&[], 5),
                pairs(&[]),
            ),
        ] {
            assert_eq!(seed_pairs(&prev, &next), want, "{name}");
        }
    }

    /// A feed of `(uuid, text)`s.
    fn texts(v: &[(&str, &str)]) -> ObsFeed {
        ObsFeed {
            texts: v
                .iter()
                .map(|(uuid, text)| ObsText {
                    uuid: (*uuid).to_string(),
                    text: (*text).to_string(),
                })
                .collect(),
            follows: Vec::new(),
            sleeps: 0,
        }
    }

    fn set(v: &[&str]) -> HashSet<String> {
        v.iter().map(|s| (*s).to_string()).collect()
    }

    fn confirmed(v: &[(&str, &str)]) -> Pushed {
        Pushed(
            v.iter()
                .map(|(u, t)| ((*u).to_string(), (*t).to_string()))
                .collect(),
        )
    }

    /// §7.8, a row at a time, then a run: one push per call, in feed order,
    /// then the clears, each recorded only once OBS confirms it, and then
    /// nothing until the feed changes.
    #[test]
    #[allow(clippy::too_many_lines)] // a row per case
    fn next_push_table() {
        let some = |u: &str, t: &str| Some((u.to_string(), t.to_string()));
        for (name, pushed, feed, missing, written, want) in [
            (
                "a new mapping (or a new connection)",
                confirmed(&[]),
                texts(&[("u", "04:59")]),
                set(&[]),
                set(&[]),
                some("u", "04:59"),
            ),
            (
                "OBS has it",
                confirmed(&[("u", "04:59")]),
                texts(&[("u", "04:59")]),
                set(&[]),
                set(&["u"]),
                None,
            ),
            (
                "a new time",
                confirmed(&[("u", "05:00")]),
                texts(&[("u", "04:59")]),
                set(&[]),
                set(&["u"]),
                some("u", "04:59"),
            ),
            (
                "OBS can't find it",
                confirmed(&[]),
                texts(&[("u", "04:59")]),
                set(&["u"]),
                set(&[]),
                None,
            ),
            (
                "unmapped after Divora wrote it: a clear",
                confirmed(&[("u", "04:59")]),
                texts(&[]),
                set(&[]),
                set(&["u"]),
                some("u", ""),
            ),
            (
                "unmapped, never written in this process",
                confirmed(&[]),
                texts(&[]),
                set(&[]),
                set(&[]),
                None,
            ),
            (
                "unmapped and missing: no clear",
                confirmed(&[]),
                texts(&[]),
                set(&["u"]),
                set(&["u"]),
                None,
            ),
            (
                "unmapped and already confirmed empty",
                confirmed(&[("u", "")]),
                texts(&[]),
                set(&[]),
                set(&["u"]),
                None,
            ),
            (
                "mapped but missing is never cleared",
                confirmed(&[("v", "1")]),
                texts(&[("u", "1"), ("v", "1")]),
                set(&["u"]),
                set(&["u", "v"]),
                None,
            ),
            (
                "two timers, one uuid: the first only",
                confirmed(&[("u", "first")]),
                texts(&[("u", "first"), ("u", "second")]),
                set(&[]),
                set(&["u"]),
                None,
            ),
            (
                "feed order before clears",
                confirmed(&[("a", "1")]),
                texts(&[("a", "1"), ("b", "2")]),
                set(&[]),
                set(&["a", "z"]),
                some("b", "2"),
            ),
            (
                "the first clear is the smallest uuid",
                confirmed(&[]),
                texts(&[]),
                set(&[]),
                set(&["m", "c", "x"]),
                some("c", ""),
            ),
        ] {
            assert_eq!(
                next_push(&pushed, &feed, &missing, &written),
                want,
                "{name}"
            );
        }

        // A run: what the link does, OBS confirming each push.
        let feed = texts(&[("b", "2"), ("a", "1"), ("b", "dup")]);
        let mut pushed = confirmed(&[("old", "9")]);
        let mut written = set(&["old", "gone"]);
        let missing = set(&[]);
        let mut sent = Vec::new();
        while let Some((uuid, text)) = next_push(&pushed, &feed, &missing, &written) {
            assert!(sent.len() < 10, "{sent:?}");
            sent.push((uuid.clone(), text.clone()));
            if text.is_empty() {
                written.remove(&uuid);
            } else {
                written.insert(uuid.clone());
            }
            pushed.0.insert(uuid, text);
        }
        let want: Vec<(String, String)> = [("b", "2"), ("a", "1"), ("gone", ""), ("old", "")]
            .iter()
            .map(|(u, t)| ((*u).to_string(), (*t).to_string()))
            .collect();
        assert_eq!(sent, want);
        assert_eq!(written, set(&["a", "b"]));
        // Unconfirmed, a push is offered again (the retry re-runs it).
        let unconfirmed = confirmed(&[]);
        assert_eq!(
            next_push(&unconfirmed, &feed, &missing, &set(&[])),
            some("b", "2")
        );
        assert_eq!(
            next_push(&unconfirmed, &feed, &missing, &set(&[])),
            some("b", "2")
        );
    }

    /// §7.13 (fixture `failureTable`): each work's answer to 207, the
    /// not-found codes, 702 and another code. A push treats any 6xx as
    /// missing and a 7xx as a refusal; a scene op only 600 and 602 as not
    /// found (604 is `failed`). The fixture's `failureResults` name every
    /// result.
    #[test]
    fn request_failure_table() {
        let f = fixture();
        let results = f["failureResults"].as_object().unwrap();
        let mut words: Vec<String> = results.keys().cloned().collect();
        words.sort();
        let mut ours: Vec<String> = [
            Failure::Missing,
            Failure::RetryLater,
            Failure::TextSkipFailed,
            Failure::SkipNotReady,
            Failure::SkipNotFound,
            Failure::SkipFailed,
            Failure::DueAgain,
            Failure::WaitAndRetry,
            Failure::EndProtocol,
            Failure::Refused,
            Failure::ListedNotReadingFile,
        ]
        .iter()
        .map(|r| {
            serde_json::to_value(r)
                .unwrap()
                .as_str()
                .unwrap()
                .to_string()
        })
        .collect();
        ours.sort();
        assert_eq!(words, ours);

        let rows = f["failureTable"].as_array().unwrap();
        assert_eq!(rows.len(), 59);
        for row in rows {
            let work: Work = serde_json::from_value(row["work"].clone()).unwrap();
            let name = row["request"].as_str().unwrap();
            let at = REQUEST_TYPES.iter().position(|t| *t == name).unwrap();
            let request = RequestKind::ALL[at];
            let code = row["code"].as_u64().unwrap();
            let want: Failure = serde_json::from_value(row["result"].clone()).unwrap();
            assert_eq!(request_failure(work, request, code), want, "{row}");
        }
        // The edges of the push's range, and of the scene ops' two codes.
        for (code, want) in [
            (599, Failure::TextSkipFailed),
            (600, Failure::Missing),
            (699, Failure::Missing),
            (700, Failure::TextSkipFailed),
            (703, Failure::TextSkipFailed),
            (207, Failure::RetryLater),
        ] {
            let got = request_failure(Work::Push, RequestKind::SetInputSettings, code);
            assert_eq!(got, want, "push {code}");
        }
        for code in [601, 603, 605, 607, 699] {
            for work in [Work::Scene, Work::SceneBack] {
                let got = request_failure(work, RequestKind::SetCurrentProgramScene, code);
                assert_eq!(got, Failure::SkipFailed, "{work:?} {code}");
            }
        }
        assert_eq!(NOT_READY, 207);
    }

    /// D15: scene events coalesce: the first bump comes at once, the next
    /// no sooner than the period after it, and a mark made in between is
    /// kept for that bump, never lost.
    #[test]
    fn rev_coalesce_table() {
        let period = Duration::from_millis(250);
        let ms = Duration::from_millis;
        let t0 = Instant::now();
        let mut rev = RevCoalesce::new(period);
        assert_eq!(rev.due_at(t0), None);
        assert!(!rev.take(t0));

        rev.mark();
        assert_eq!(rev.due_at(t0), Some(t0));
        assert!(rev.take(t0));
        assert_eq!(rev.due_at(t0), None);
        assert!(!rev.take(t0 + ms(1)));

        // Inside the period: held, not lost.
        rev.mark();
        assert_eq!(rev.due_at(t0 + ms(10)), Some(t0 + period));
        assert!(!rev.take(t0 + ms(10)));
        assert!(!rev.take(t0 + ms(249)));
        assert!(rev.take(t0 + period));
        // Past the period: at once.
        rev.mark();
        assert_eq!(rev.due_at(t0 + ms(900)), Some(t0 + ms(900)));
        assert!(rev.take(t0 + ms(900)));

        // A flood: a mark every 5 ms for 2 s, the driver taking at each
        // mark and at each `due_at`. At most one bump per period, and the
        // last mark is bumped for.
        let mut rev = RevCoalesce::new(period);
        let mut bumps = Vec::new();
        let mut timer: Option<Instant> = None;
        for i in 0..400_u32 {
            let now = t0 + ms(u64::from(i) * 5);
            if let Some(at) = timer.filter(|at| *at <= now) {
                if rev.take(at) {
                    bumps.push(at);
                }
            }
            rev.mark();
            if rev.take(now) {
                bumps.push(now);
            }
            timer = rev.due_at(now);
        }
        let last = t0 + ms(399 * 5);
        let at = timer.unwrap();
        assert!(at > last);
        assert!(rev.take(at));
        bumps.push(at);
        assert!(rev.due_at(at).is_none());
        for pair in bumps.windows(2) {
            assert!(pair[1] - pair[0] >= period, "{pair:?}");
        }
        assert_eq!(bumps.len(), 9);
    }

    /// D15: only short version strings of `[0-9A-Za-z.+-]` reach the status.
    #[test]
    fn sanitize_version_table() {
        let max = "1".repeat(32);
        for (v, kept) in [
            ("5.5.2", true),
            ("30.2.2", true),
            ("32.0.0-beta1", true),
            ("1.2.3+build.5", true),
            (max.as_str(), true),
            ("", false),
            ("5.5.2 ", false),
            ("5.5.2 (x)", false),
            ("<b>5</b>", false),
            ("5.5.2\n", false),
            ("5_5_2", false),
            ("5.5.2/../", false),
            ("ünïcode", false),
            ("5.5.2\u{0}", false),
        ] {
            let want = kept.then(|| v.to_string());
            assert_eq!(sanitize_version(v), want, "{v:?}");
        }
        let long = "1".repeat(33);
        assert_eq!(sanitize_version(&long), None);
    }

    /// The status's stream and record words, from an event's state and from
    /// a status answer.
    #[test]
    fn status_words_table() {
        use OutputState::{
            Paused, Reconnected, Reconnecting, Resumed, Started, Starting, Stopped, Stopping,
            Unknown,
        };
        for (state, stream, record) in [
            (
                Starting,
                Some(StreamState::Starting),
                Some(RecordState::Starting),
            ),
            (
                Started,
                Some(StreamState::Live),
                Some(RecordState::Recording),
            ),
            (
                Stopping,
                Some(StreamState::Stopping),
                Some(RecordState::Stopping),
            ),
            (Stopped, Some(StreamState::Off), Some(RecordState::Off)),
            (Reconnecting, Some(StreamState::Reconnecting), None),
            (Reconnected, Some(StreamState::Live), None),
            (Paused, None, Some(RecordState::Paused)),
            (Resumed, None, Some(RecordState::Recording)),
            (Unknown, None, None),
        ] {
            assert_eq!(stream_word(state), stream, "{state:?}");
            assert_eq!(record_word(state), record, "{state:?}");
        }
        let status = |active, reconnecting, paused| OutputStatus {
            active,
            reconnecting,
            paused,
            duration_ms: 0,
        };
        assert_eq!(
            stream_status_word(status(true, false, false)),
            StreamState::Live
        );
        assert_eq!(
            stream_status_word(status(false, true, false)),
            StreamState::Reconnecting
        );
        assert_eq!(
            stream_status_word(status(false, false, false)),
            StreamState::Off
        );
        assert_eq!(
            record_status_word(status(true, false, true)),
            RecordState::Paused
        );
        assert_eq!(
            record_status_word(status(true, false, false)),
            RecordState::Recording
        );
        assert_eq!(
            record_status_word(status(false, false, true)),
            RecordState::Off
        );
    }
}
