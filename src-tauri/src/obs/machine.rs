//! v1.61.0: the link's rules, as pure functions and plain data — the wire
//! types of §4 (the config, the status, the inventory), the status cell,
//! hosts, backoff, what an output event does to a count-up that follows it,
//! which text to push next, and what a refused request means. Nothing here
//! reads a clock, waits, logs or touches a socket: callers hand in the time,
//! so every rule is tested on any machine (§10.1).

use std::collections::{HashMap, HashSet};
use std::io;
use std::net::{IpAddr, SocketAddr};
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
    #[allow(unused_variables)] // S1: track A writes this body
    pub fn begin(&mut self, cfg: &LinkConfig) -> (u64, Option<ObsStatus>) {
        todo!("S1: track A")
    }

    /// A driver's write: ignored when `gen` is stale. `f` applied to a
    /// clone; if that differs, `seq` + 1, stored and returned.
    #[allow(unused_variables)] // S1: track A writes this body
    pub fn write(&mut self, gen: u64, f: impl FnOnce(&mut ObsStatus)) -> Option<ObsStatus> {
        todo!("S1: track A")
    }

    /// `ObsShared`'s own writes (skips, `heldBack`, `off`): always current.
    #[allow(unused_variables)] // S1: track A writes this body
    pub fn write_shared(&mut self, f: impl FnOnce(&mut ObsStatus)) -> Option<ObsStatus> {
        todo!("S1: track A")
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
#[allow(unused_variables)] // S1: track A writes this body
pub fn parse_host(host: &str) -> Result<IpAddr, HostError> {
    todo!("S1: track A")
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
/// once with its kind by the caller).
#[allow(unused_variables)] // S1: track A writes this body
pub fn connect_reason(kind: io::ErrorKind) -> ObsReason {
    todo!("S1: track A")
}

/// §7.3, D14: the wait in ms before attempt `failures` + 1: 1, 2, 4, 8, 15,
/// then 30 s (in ms), times `0.8 + 0.4·r`, then at most 30 000.
#[allow(unused_variables)] // S1: track A writes this body
pub fn backoff_ms(failures: u32, r: f64) -> u64 {
    todo!("S1: track A")
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
#[allow(unused_variables)] // S1: track A writes this body
pub fn follow_op(what: FollowObs, ev: &ObsEvent, since_ms: u64) -> Option<FollowAction> {
    todo!("S1: track A")
}

/// §7.7's second table (fixture `seedTable`): what OBS's status of `what`
/// does to the timer, given whether it is running now.
#[allow(unused_variables)] // S1: track A writes this body
pub fn join_seed(what: FollowObs, status: OutputStatus, running_now: bool) -> Option<TimerOp> {
    todo!("S1: track A")
}

/// D26: the `(timer id, what)` pairs `next` follows that `prev` didn't —
/// every pair when `sleeps` changed — each to be seeded.
#[allow(unused_variables)] // S1: track A writes this body
pub fn seed_pairs(prev: &ObsFeed, next: &ObsFeed) -> Vec<(String, FollowObs)> {
    todo!("S1: track A")
}

// ---- Text pushes (D18, §7.8) -----------------------------------------------

/// What OBS confirmed on this connection: source uuid → text. A new
/// connection starts empty.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Pushed(pub HashMap<String, String>);

/// §7.8: the next push, `(uuid, text)`: the first uuid in feed order whose
/// text differs from what OBS confirmed (not one OBS can't find), else the
/// first uuid Divora wrote that left the feed (`""`, a clear), else none.
#[allow(unused_variables, clippy::implicit_hasher)] // S1: track A writes this body
pub fn next_push(
    pushed: &Pushed,
    feed: &ObsFeed,
    missing: &HashSet<String>,
    written: &HashSet<String>,
) -> Option<(String, String)> {
    todo!("S1: track A")
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

/// §7.13 (fixture `failureTable`): `code`, OBS's error answer to `request`
/// sent for `work`.
#[allow(unused_variables)] // S1: track A writes this body
pub fn request_failure(work: Work, request: RequestKind, code: u64) -> Failure {
    todo!("S1: track A")
}

// ---- Status words, versions, coalescing (D15) ------------------------------

/// A `StreamStateChanged`'s state, as the status's `stream` word; `None`
/// leaves it as it was.
#[allow(unused_variables)] // S1: track A writes this body
pub fn stream_word(state: OutputState) -> Option<StreamState> {
    todo!("S1: track A")
}

/// A `RecordStateChanged`'s state, as the status's `record` word; `None`
/// leaves it as it was.
#[allow(unused_variables)] // S1: track A writes this body
pub fn record_word(state: OutputState) -> Option<RecordState> {
    todo!("S1: track A")
}

/// D15: a version OBS sent, kept only if it is at most 32 characters of
/// `[0-9A-Za-z.+-]`.
#[allow(unused_variables)] // S1: track A writes this body
pub fn sanitize_version(s: &str) -> Option<String> {
    todo!("S1: track A")
}

/// D15: scene events mark the inventory dirty; `inventoryRev` goes up at
/// most once per `period`, and no mark is lost.
#[derive(Clone, Copy, Debug)]
#[allow(dead_code)] // S1: track A's bodies read every field
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

    /// When the next bump may happen, while dirty.
    #[allow(unused_variables, clippy::unused_self)] // S1: track A writes this body
    pub fn due_at(&self, now: Instant) -> Option<Instant> {
        todo!("S1: track A")
    }

    /// Whether to bump `inventoryRev` now (dirty, and the period since the
    /// last bump has passed); clears the mark when it does.
    #[allow(unused_variables, clippy::unused_self)] // S1: track A writes this body
    pub fn take(&mut self, now: Instant) -> bool {
        todo!("S1: track A")
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
        HostError, LastSkip, ObsConfig, ObsInventory, ObsReason, ObsState, ObsStatus, RecordState,
        SkipOp, SkipWhy, StreamState, TextKind, TextSkip, TextSkipWhy, TextSource,
    };
    use crate::obs::scenes::SceneRef;
    use serde::Serialize;
    use serde_json::{json, Value};

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
}
