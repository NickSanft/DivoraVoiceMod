//! v1.59.0: Hourglass — countdowns and count-ups on a hotkey, as a pure core.
//!
//! This module is the rules, and only the rules: the wire shapes and limits,
//! the timer id grammar, which `tm:` hotkey ids act, the text a timer shows,
//! how a sync entry is read, and [`TimerCore`], which owns every deadline.
//! Every reader — the timer thread, a command, a `tm:` press — reads the
//! clock once under the core's lock, checks that reading for a sleep, and
//! hands both in; nothing here reads a clock, waits, touches a file or calls
//! Tauri, so every rule is tested on any machine. `timer_thread.rs` is the
//! shell around it.
//!
//! The clock is a `Duration` since a process origin (monotonic). Wall time
//! (Unix ms) only sets the anchors a restart seeds from, and checks a sleep
//! the clock missed (D3, D5).
//!
//! The wire contract — names, keys, phases, kinds, limits, the id and
//! hotkey-id tables and the run samples — is `src/overlay/timer.fixture.json`,
//! which the frontend's tests read too, so a change on either side fails the
//! other's tests.

use std::collections::HashSet;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// The event every change goes out as, to every window. Payload:
/// [`TimerSnapshot`].
pub const TIMER_STATE_EVENT: &str = "timer-state";

/// The hotkey ids the backend acts on for timers: `tm:<id>:toggle` and
/// `tm:<id>:reset`. The prefix is reserved (`tm:<id>:finish` is v1.60.0's).
pub const HOTKEY_TIMER_PREFIX: &str = "tm:";

/// The timer thread's longest wait: sleep checks keep this cadence with
/// nothing running.
pub const TICK: Duration = Duration::from_millis(100);

/// D5 (b): after a sleep signal with no clock jump, a running timer whose
/// monotonic time disagrees with its wall anchor by more than this missed
/// the sleep.
pub const CLOCK_SLACK: Duration = Duration::from_secs(2);

/// D5: no (b) check this soon after an (a): that sleep was counted, and a
/// clock resync after the resume isn't another one.
pub const RECENT_SLEEP: Duration = Duration::from_secs(60);

/// D3: a wall-clock step this big (wall − monotonic, in ms) …
pub const ANCHOR_SLACK_MS: i64 = 1_000;

/// … held this long moves the running timers' anchors by the step.
pub const ANCHOR_HOLD: Duration = Duration::from_secs(10);

/// A fire later than this is logged at warn, with its lateness.
pub const LATE_LOG: Duration = Duration::from_secs(1);

/// Timers the core runs: the first this many known-kind entries of a sync,
/// in list order (D36). The page stores no more (D8).
pub const MAX_TIMERS: usize = 16;

/// Entries of one `sync_timers` call that are read; the rest are skipped.
pub const MAX_SYNC_ENTRIES: usize = 64;

/// A countdown's shortest length.
pub const MIN_LENGTH_MS: u64 = 1_000;

/// A countdown's longest length, and the most it can have left after +30 s.
pub const MAX_LENGTH_MS: u64 = 86_400_000;

/// A countdown's length when a sync entry's can't be read.
pub const DEFAULT_LENGTH_MS: u64 = 300_000;

/// A timer's longest name, in characters (Unicode scalar values) after
/// trimming.
pub const MAX_NAME_CHARS: usize = 40;

/// A timer id's longest length: it names `timers/<id>.txt`.
pub const MAX_ID_CHARS: usize = 32;

/// The most a count-up seeded at a restart has counted: 10 000 h (D8).
pub const MAX_SEED_ELAPSED_MS: u64 = 36_000_000_000;

/// A timer's kind. A stored kind this build doesn't know is kept, not run
/// ([`WireEntry::Kept`], D36).
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "lowercase")]
pub enum TimerKind {
    Countdown,
    Countup,
}

/// Where a timer is. `Done`: a countdown reached zero while the PC was awake
/// and Divora running, and fired (late, if Divora was held up). `Expired`: it
/// reached zero while the PC slept or Divora was closed, and did not fire. A
/// count-up is only ever idle, running or paused (D2).
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "lowercase")]
pub enum TimerPhase {
    Idle,
    Running,
    Paused,
    Done,
    Expired,
}

/// One timer as a sync entry defines it, read by [`entry_from_wire`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TimerDef {
    /// Passes [`is_timer_id`].
    pub id: String,
    /// 1 to [`MAX_NAME_CHARS`] characters, trimmed.
    pub name: String,
    pub kind: TimerKind,
    /// A countdown's length, [`MIN_LENGTH_MS`] to [`MAX_LENGTH_MS`]. Read
    /// the same way for a count-up, which doesn't use it.
    pub duration_ms: u64,
    /// The run the page stored for it, read by [`run_from_wire`]: applied
    /// only to an id the core has never accepted in this process (D10).
    pub resume: Option<TimerRun>,
}

/// What the page stores of a running, paused, done or ran-out timer, and
/// sends back as `resume` with every sync (§7.8). Absent keys stay absent on
/// the wire, as the page writes them.
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TimerRun {
    /// Never `Idle`: an idle timer stores no run.
    pub phase: TimerPhase,
    /// Paused: the value it stopped at (a countdown's left, a count-up's
    /// elapsed).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ms: Option<u64>,
    /// A countdown's total (length ± adjusts).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_ms: Option<u64>,
    /// Running: wall ms a countdown ends at, or a count-up effectively
    /// started at. Always there for a running run.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub anchor_ms: Option<i64>,
}

/// A sync entry that was read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WireEntry {
    /// A known kind: run if it is among the first [`MAX_TIMERS`].
    Def(TimerDef),
    /// A valid id of a kind this build doesn't know: kept, not run, and its
    /// file kept out of the sweep (D36).
    Kept(String),
}

/// One timer the core runs, as every window sees it.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TimerState {
    pub id: String,
    pub name: String,
    pub kind: TimerKind,
    pub phase: TimerPhase,
    /// At the snapshot: a countdown's left, rounded up to whole ms; a
    /// count-up's elapsed, rounded down. Always agrees with `text` (D22).
    pub ms: u64,
    /// A countdown's current total: its length ± adjusts since its start (it
    /// can pass 24 h). 0 for a count-up.
    pub total_ms: u64,
    /// [`timer_text`] of `kind` and `ms`.
    pub text: String,
    /// Running: wall ms a countdown ends at, or a count-up effectively
    /// started at. Otherwise `null`.
    pub anchor_ms: Option<i64>,
    /// Fires of this definition since its id entered the set (a re-added id
    /// starts again at 0).
    pub fires: u32,
}

/// The whole set the core runs, in sync order. Of two snapshots, the larger
/// `seq` is newer: it is taken under the core's lock every time one is
/// built (D17).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct TimerSnapshot {
    pub seq: u64,
    pub timers: Vec<TimerState>,
}

/// An op named an id the core doesn't run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NoTimer;

impl std::fmt::Display for NoTimer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("no such timer")
    }
}

/// What a command or a `tm:` press asks of one timer (§7.1, §7.3). The
/// labelled ones do only what they name; `Toggle` (the hotkey's) decides
/// from the core's own state (D6).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TimerOp {
    Arm,
    Pause,
    Resume,
    Reset,
    Toggle,
    /// Whole ms, within ±[`MAX_LENGTH_MS`] ([`adjust_delta`]).
    Adjust(i64),
}

/// Something that happened to a timer, wherever it happened: the thread's
/// tick, an op's or a status read's tick on the main thread, a sync's seed.
/// Collected by the core and drained only by the timer thread, so each is
/// reported exactly once (D34).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Transition {
    /// A countdown reached zero awake and fired; `late_ms` past its deadline.
    Fired {
        id: String,
        fires: u32,
        late_ms: u64,
    },
    /// A countdown reached zero while the PC slept; not fired.
    Expired { id: String },
    /// A count-up paused where the PC went to sleep.
    SleepPaused { id: String },
    /// A timer the core had never accepted, seeded from its stored run.
    Seeded { id: String, phase: TimerPhase },
}

/// What the file writer needs: each run timer's text, in list order, and
/// every id whose file stays (the ones run and the ones kept).
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct TextUpdate {
    pub texts: Vec<(String, String)>,
    pub keep: Vec<String>,
}

/// D35, §7.14: one `sync_timers` entry, read on its own so one bad field
/// can't fail the call. `Err(reason)`: skipped, and the caller logs why (no
/// valid id, or no kind). A bad name or length falls back; a bad `resume`
/// becomes none (the timer starts idle). Unknown keys are ignored: later
/// releases add fields (D33).
pub fn entry_from_wire(v: &Value) -> Result<WireEntry, String> {
    let Some(entry) = v.as_object() else {
        return Err("not an object".to_string());
    };
    let Some(id) = entry
        .get("id")
        .and_then(Value::as_str)
        .filter(|id| is_timer_id(id))
    else {
        return Err("no valid id".to_string());
    };
    let kind = match entry.get("kind").and_then(Value::as_str) {
        Some("countdown") => TimerKind::Countdown,
        Some("countup") => TimerKind::Countup,
        Some(_) => return Ok(WireEntry::Kept(id.to_string())),
        None => return Err(format!("{id}: no kind")),
    };
    let name = entry
        .get("name")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|name| (1..=MAX_NAME_CHARS).contains(&name.chars().count()))
        .map_or_else(|| default_name(kind).to_string(), str::to_string);
    Ok(WireEntry::Def(TimerDef {
        id: id.to_string(),
        name,
        kind,
        duration_ms: length_from_wire(entry.get("durationMs")),
        resume: entry.get("resume").and_then(run_from_wire),
    }))
}

/// The name a sync entry without a usable one runs under. The page sends a
/// valid name; this covers a hand-edited `divora.timers`.
fn default_name(kind: TimerKind) -> &'static str {
    match kind {
        TimerKind::Countdown => "Countdown",
        TimerKind::Countup => "Count-up",
    }
}

/// A finite number, rounded and clamped to [`MIN_LENGTH_MS`] ..
/// [`MAX_LENGTH_MS`]; anything else [`DEFAULT_LENGTH_MS`].
fn length_from_wire(v: Option<&Value>) -> u64 {
    match v.and_then(Value::as_f64) {
        Some(ms) if ms.is_finite() => {
            // A float-to-int cast saturates (a negative one gives 0), and
            // the clamp follows.
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let ms = ms.round() as u64;
            ms.clamp(MIN_LENGTH_MS, MAX_LENGTH_MS)
        }
        _ => DEFAULT_LENGTH_MS,
    }
}

/// §7.8, §7.14: a stored run. `None` when it is malformed — not an object,
/// a phase other than running, paused, done or expired, an `ms` or `totalMs`
/// that is present but not a non-negative integer, an `anchorMs` present but
/// not an integer (`null` included), or a running run without `anchorMs` —
/// and the timer then starts idle. Which fields a seed needs beyond these is
/// the seed's to say.
pub fn run_from_wire(v: &Value) -> Option<TimerRun> {
    let run = v.as_object()?;
    let phase = match run.get("phase")?.as_str()? {
        "running" => TimerPhase::Running,
        "paused" => TimerPhase::Paused,
        "done" => TimerPhase::Done,
        "expired" => TimerPhase::Expired,
        _ => return None,
    };
    let ms = field(run, "ms", Value::as_u64).ok()?;
    let total_ms = field(run, "totalMs", Value::as_u64).ok()?;
    let anchor_ms = field(run, "anchorMs", Value::as_i64).ok()?;
    if phase == TimerPhase::Running && anchor_ms.is_none() {
        return None;
    }
    Some(TimerRun {
        phase,
        ms,
        total_ms,
        anchor_ms,
    })
}

/// An optional field: `Ok(None)` absent, `Ok(Some(_))` read, `Err(())`
/// present but not what `read` takes.
fn field<T>(
    run: &Map<String, Value>,
    key: &str,
    read: fn(&Value) -> Option<T>,
) -> Result<Option<T>, ()> {
    run.get(key)
        .map_or(Ok(None), |v| read(v).map(Some).ok_or(()))
}

/// §7.6, D9: 1 to [`MAX_ID_CHARS`] of `a-z`, `0-9` and `-`, starting and
/// ending with a letter or digit, and not a Windows device name (`con`,
/// `prn`, `aux`, `nul`, `com0`-`com9`, `lpt0`-`lpt9`). The id sits between
/// two colons in `tm:<id>:<verb>` and names `timers/<id>.txt`, so it can hold
/// no `:`, `.`, slash or upper case (NTFS ignores case).
pub fn is_timer_id(id: &str) -> bool {
    let b = id.as_bytes();
    let (Some(&first), Some(&last)) = (b.first(), b.last()) else {
        return false;
    };
    b.len() <= MAX_ID_CHARS
        && b.iter()
            .all(|c| matches!(c, b'a'..=b'z' | b'0'..=b'9' | b'-'))
        && first != b'-'
        && last != b'-'
        && !is_device_name(id)
}

/// A name Windows keeps for a device, with any extension (`con.txt`).
fn is_device_name(id: &str) -> bool {
    matches!(id, "con" | "prn" | "aux" | "nul")
        || ((id.starts_with("com") || id.starts_with("lpt"))
            && id.len() == 4
            && id.as_bytes()[3].is_ascii_digit())
}

/// §7.7: what a shortcut event does to a timer. `Some((id, op))` for a press
/// of `tm:<id>:toggle` or `tm:<id>:reset` with a valid id; `None` for a
/// release, `tm:<id>:finish` (v1.60.0's), any other spelling and any other
/// id.
pub fn hotkey_timer_request(id: &str, pressed: bool) -> Option<(&str, TimerOp)> {
    if !pressed {
        return None;
    }
    let (timer, verb) = id.strip_prefix(HOTKEY_TIMER_PREFIX)?.split_once(':')?;
    let op = match verb {
        "toggle" => TimerOp::Toggle,
        "reset" => TimerOp::Reset,
        _ => return None,
    };
    is_timer_id(timer).then_some((timer, op))
}

/// §7.4, D22: the text a timer shows everywhere — overlay, `.txt`, Settings.
/// A countdown rounds up to whole seconds, a count-up down. Under an hour
/// `MM:SS` (`05:00`); from an hour `H:MM:SS` (`1:02:03`, `100:00:00`). No
/// sign, no overtime.
pub fn timer_text(kind: TimerKind, ms: u64) -> String {
    let s = match kind {
        TimerKind::Countdown => ms.div_ceil(1_000),
        TimerKind::Countup => ms / 1_000,
    };
    if s < 3_600 {
        format!("{:02}:{:02}", s / 60, s % 60)
    } else {
        format!("{}:{:02}:{:02}", s / 3_600, s / 60 % 60, s % 60)
    }
}

/// D7: `adjust_timer`'s `deltaMs`, checked: a whole number of ms within
/// ±[`MAX_LENGTH_MS`]. The UI sends ±30 000.
pub fn adjust_delta(delta_ms: f64) -> Result<i64, String> {
    const BAD: &str = "deltaMs must be a whole number of milliseconds, at most 24 hours either way";
    if !delta_ms.is_finite() || delta_ms.fract() != 0.0 {
        return Err(BAD.to_string());
    }
    // Whole and finite; a cast past i64's range saturates, and the range
    // check below refuses it.
    #[allow(clippy::cast_possible_truncation)]
    let delta = delta_ms as i64;
    if delta.unsigned_abs() > MAX_LENGTH_MS {
        return Err(BAD.to_string());
    }
    Ok(delta)
}

/// One timer the core runs.
///
/// A running timer holds `since` (the clock reading its run was last set
/// at) and `base_ms` (a countdown's left at `since`, rounded up; a
/// count-up's elapsed at `since`, rounded down), so nothing is ever a
/// negative `Duration`; a countdown's deadline is `since + base_ms`. Paused,
/// `base_ms` is the value it stopped at; idle, a countdown's length or a
/// count-up's 0; done or expired, 0.
#[allow(dead_code)] // S1: built by track A's `sync`; until then the set stays empty
#[derive(Debug)]
struct Timer {
    id: String,
    name: String,
    kind: TimerKind,
    /// The definition's length (a countdown's).
    length_ms: u64,
    phase: TimerPhase,
    since: Duration,
    base_ms: u64,
    /// A countdown's total since its start (length ± adjusts); 0 for a
    /// count-up.
    total_ms: u64,
    /// Set at each transition that changes the run (Arm, Resume, Adjust, a
    /// seed, a (b) correction), cleared when it stops running.
    anchor_ms: Option<i64>,
    /// `wall − clock` in ms when the anchor was set: a held wall step moves
    /// both (§7.2).
    anchor_offset_ms: i64,
    fires: u32,
    /// The text as last computed: every transition recomputes it, and a
    /// running timer's at each tick and snapshot.
    text: String,
}

impl Timer {
    /// Its `ms` at `now` (D22): a running countdown's left rounded up, a
    /// running count-up's elapsed rounded down; otherwise `base_ms`.
    fn ms_at(&self, now: Duration) -> u64 {
        if self.phase != TimerPhase::Running {
            return self.base_ms;
        }
        match self.kind {
            TimerKind::Countdown => ceil_ms(self.deadline().saturating_sub(now)),
            TimerKind::Countup => self
                .base_ms
                .saturating_add(floor_ms(now.saturating_sub(self.since))),
        }
    }

    /// A running countdown's deadline on the core's clock.
    fn deadline(&self) -> Duration {
        self.since
            .saturating_add(Duration::from_millis(self.base_ms))
    }

    /// §7.5: how long until a running timer's text next changes — a
    /// countdown's deadline being its last boundary — computed in ns.
    fn wake_in(&self, now: Duration) -> Duration {
        const SECOND_NS: u128 = 1_000_000_000;
        let ns = match self.kind {
            TimerKind::Countdown => match self.deadline().saturating_sub(now).as_nanos() {
                0 => 0,
                left => (left - 1) % SECOND_NS + 1,
            },
            TimerKind::Countup => {
                let elapsed = Duration::from_millis(self.base_ms)
                    .saturating_add(now.saturating_sub(self.since))
                    .as_nanos();
                SECOND_NS - elapsed % SECOND_NS
            }
        };
        Duration::from_nanos(u64::try_from(ns).unwrap_or(u64::MAX))
    }

    /// How every window sees it, with `ms` from [`Timer::ms_at`].
    fn state(&self, ms: u64) -> TimerState {
        TimerState {
            id: self.id.clone(),
            name: self.name.clone(),
            kind: self.kind,
            phase: self.phase,
            ms,
            total_ms: self.total_ms,
            text: self.text.clone(),
            anchor_ms: self.anchor_ms,
            fires: self.fires,
        }
    }
}

/// Whole ms in `d`, rounded up.
fn ceil_ms(d: Duration) -> u64 {
    u64::try_from(d.as_nanos().div_ceil(1_000_000)).unwrap_or(u64::MAX)
}

/// Whole ms in `d`, rounded down.
fn floor_ms(d: Duration) -> u64 {
    u64::try_from(d.as_millis()).unwrap_or(u64::MAX)
}

/// The timers and every deadline (D4-D11, D34-D36).
///
/// Every entry point runs [`TimerCore::tick`] first, with the one clock
/// reading and the sleep check its caller took under the core's lock:
/// `now` (the monotonic clock), `wall_ms` (Unix ms) and `slept_from`
/// (`Some(before)` when the PC slept since the last reader: `before` is the
/// last reading anyone took awake). Every arithmetic step saturates: a panic
/// aborts the app.
///
/// S1: the bodies an empty core needs — the timer thread ticks one from
/// startup until the page's first sync — are real; [`TimerCore::sync`],
/// and `tick` and `apply` on a timer, are track A's.
#[derive(Debug)]
pub struct TimerCore {
    /// In sync order.
    timers: Vec<Timer>,
    /// Ids of entries kept, not run (D36): their files stay.
    kept: Vec<String>,
    /// Ids accepted this process: only these ignore a stored run (D10).
    #[allow(dead_code)] // S1: track A's `sync` reads it
    seen: HashSet<String>,
    transitions: Vec<Transition>,
    seq: u64,
    dirty: bool,
    synced: bool,
    /// When a clock-counted sleep (D5 (a)) was last handled: no (b) for
    /// [`RECENT_SLEEP`] after it.
    #[allow(dead_code)] // S1: track A's `tick` reads it
    last_clock_sleep: Option<Duration>,
}

impl TimerCore {
    pub fn new() -> Self {
        Self {
            timers: Vec::new(),
            kept: Vec::new(),
            seen: HashSet::new(),
            transitions: Vec::new(),
            seq: 0,
            dirty: false,
            synced: false,
            last_clock_sleep: None,
        }
    }

    /// D5, D3, D4 (§7.2), for running timers, in order: the sleep rule
    /// ((a) a clock-counted sleep, (b) a signal the clock missed, checked
    /// against the wall anchors), held wall steps, settle (fires each due
    /// countdown once, however late, `fires` +1, [`Transition::Fired`]),
    /// then texts (dirty when one changed). With nothing running there is
    /// nothing to do.
    #[allow(unused_variables)] // S1: track A's body uses them
    pub fn tick(&mut self, now: Duration, wall_ms: i64, slept_from: Option<Duration>) {
        if self.timers.is_empty() {
            return;
        }
        todo!("track A: §7.2")
    }

    /// D15, D35, D36: the set becomes `entries`, in order. `tick` first. The
    /// first [`MAX_TIMERS`] definitions run; the rest, and every
    /// [`WireEntry::Kept`], are kept, not run; duplicates after the first are
    /// skipped. A timer the core already runs keeps its run (a rename or a
    /// new length changes only what it names; a kind change makes it idle);
    /// an id it has never accepted is seeded from its `resume` (§7.8,
    /// [`Transition::Seeded`]). Marks dirty and synced. Returns the ids that
    /// left.
    #[allow(unused_variables, clippy::unused_self)] // S1: track A's body
    pub fn sync(
        &mut self,
        entries: Vec<WireEntry>,
        now: Duration,
        wall_ms: i64,
        slept_from: Option<Duration>,
    ) -> Vec<String> {
        todo!("track A: D15, D35, D36, §7.8")
    }

    /// D6, D7 (§7.1, §7.3): `op` on the timer `id`. `tick` first, so a due
    /// countdown fires before the op. A labelled op that doesn't fit the
    /// phase changes nothing. `Err` for an id the core doesn't run.
    #[allow(unused_variables)] // S1: track A's body uses them
    pub fn apply(
        &mut self,
        id: &str,
        op: TimerOp,
        now: Duration,
        wall_ms: i64,
        slept_from: Option<Duration>,
    ) -> Result<(), NoTimer> {
        self.tick(now, wall_ms, slept_from);
        let Some(timer) = self.timers.iter_mut().find(|t| t.id == id) else {
            return Err(NoTimer);
        };
        todo!("track A: §7.1, §7.3")
    }

    /// `seq` + 1, then the set at `now`. A running timer's text is
    /// recomputed at `now`, so every snapshot's `ms` and `text` agree; one
    /// that changed marks dirty, so the thread still emits that second.
    /// Leaves dirty as it was otherwise: an answer's snapshot doesn't stop the
    /// thread's emit.
    pub fn snapshot(&mut self, now: Duration) -> TimerSnapshot {
        self.seq = self.seq.saturating_add(1);
        let mut timers = Vec::with_capacity(self.timers.len());
        for t in &mut self.timers {
            let ms = t.ms_at(now);
            if t.phase == TimerPhase::Running {
                let text = timer_text(t.kind, ms);
                if text != t.text {
                    t.text = text;
                    self.dirty = true;
                }
            }
            timers.push(t.state(ms));
        }
        TimerSnapshot {
            seq: self.seq,
            timers,
        }
    }

    /// Whether something changed since the last [`TimerCore::take_dirty`].
    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    /// Whether something changed since the last call; clears it.
    pub fn take_dirty(&mut self) -> bool {
        std::mem::take(&mut self.dirty)
    }

    /// D34: every transition since the last call, in order. Only the timer
    /// thread calls it.
    pub fn take_transitions(&mut self) -> Vec<Transition> {
        std::mem::take(&mut self.transitions)
    }

    /// Whether a sync has happened in this process. Until then the set of
    /// timers is unknown, so the writer neither writes nor sweeps.
    pub fn synced(&self) -> bool {
        self.synced
    }

    /// Each run timer's text, in list order, and every id whose file stays.
    pub fn text_update(&self) -> TextUpdate {
        TextUpdate {
            texts: self
                .timers
                .iter()
                .map(|t| (t.id.clone(), t.text.clone()))
                .collect(),
            keep: self
                .timers
                .iter()
                .map(|t| t.id.clone())
                .chain(self.kept.iter().cloned())
                .collect(),
        }
    }

    /// §7.5: how long from `now` until the next running timer's text changes
    /// (a countdown's last boundary being its deadline), at most `cap`; `cap`
    /// with nothing running. 0 for a due countdown (unreachable after a
    /// tick, which fires every due one).
    pub fn next_wake(&self, now: Duration, cap: Duration) -> Duration {
        self.timers
            .iter()
            .filter(|t| t.phase == TimerPhase::Running)
            .map(|t| t.wake_in(now))
            .fold(cap, Ord::min)
    }
}

impl Default for TimerCore {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::{
        adjust_delta, entry_from_wire, hotkey_timer_request, is_timer_id, run_from_wire,
        timer_text, NoTimer, TextUpdate, TimerCore, TimerDef, TimerKind, TimerOp, TimerPhase,
        TimerRun, TimerSnapshot, TimerState, WireEntry, DEFAULT_LENGTH_MS, MAX_ID_CHARS,
        MAX_LENGTH_MS, MAX_NAME_CHARS, MAX_SYNC_ENTRIES, MAX_TIMERS, MIN_LENGTH_MS, TICK,
        TIMER_STATE_EVENT,
    };
    use serde_json::{json, Value};
    use std::time::Duration;

    fn fixture() -> Value {
        serde_json::from_str(include_str!("../../src/overlay/timer.fixture.json")).unwrap()
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

    /// A `runSamples` row's name, for a failure message.
    fn row_name(row: &Value) -> &str {
        row["name"].as_str().unwrap_or("?")
    }

    /// `src/overlay/timer.fixture.json` is what `timerState.test.ts` and
    /// `timers.test.ts` read too, so a change on either side fails the
    /// other side's tests: the names, every key, the phases and kinds in
    /// order, the limits, and every sample — each snapshot and state reads
    /// into Rust's types and back unchanged, with its text, anchor and total
    /// as the rules give them, and each run reads back unchanged.
    #[test]
    fn timer_wire_fixture_matches_rust() {
        let f = fixture();
        assert_eq!(f["events"]["state"], TIMER_STATE_EVENT);

        let state = TimerState {
            id: "timer-1".into(),
            name: "BRB".into(),
            kind: TimerKind::Countdown,
            phase: TimerPhase::Running,
            ms: 1,
            total_ms: 1_000,
            text: "00:01".into(),
            anchor_ms: Some(1),
            fires: 0,
        };
        let snapshot = TimerSnapshot {
            seq: 1,
            timers: vec![state.clone()],
        };
        let full_run = TimerRun {
            phase: TimerPhase::Running,
            ms: Some(1),
            total_ms: Some(1),
            anchor_ms: Some(1),
        };
        for (value, keys) in [
            (serde_json::to_value(&snapshot).unwrap(), "snapshotKeys"),
            (serde_json::to_value(&state).unwrap(), "stateKeys"),
            (serde_json::to_value(&full_run).unwrap(), "runKeys"),
        ] {
            assert_eq!(sorted_keys(&value), strings(&f[keys]), "{keys}");
        }

        let phases = [
            TimerPhase::Idle,
            TimerPhase::Running,
            TimerPhase::Paused,
            TimerPhase::Done,
            TimerPhase::Expired,
        ]
        .map(|p| serde_json::to_value(p).unwrap());
        assert_eq!(Value::from(phases.to_vec()), f["phases"]);
        let kinds =
            [TimerKind::Countdown, TimerKind::Countup].map(|k| serde_json::to_value(k).unwrap());
        assert_eq!(Value::from(kinds.to_vec()), f["kinds"]);

        let limits = &f["limits"];
        assert_eq!(limits["maxTimers"], MAX_TIMERS);
        assert_eq!(limits["minLengthMs"], MIN_LENGTH_MS);
        assert_eq!(limits["maxLengthMs"], MAX_LENGTH_MS);
        assert_eq!(limits["defaultLengthMs"], DEFAULT_LENGTH_MS);
        assert_eq!(limits["maxName"], MAX_NAME_CHARS);
        assert_eq!(limits["maxIdChars"], MAX_ID_CHARS);
        assert_eq!(limits["maxSyncEntries"], MAX_SYNC_ENTRIES);

        // A state as the rules shape it: its text from its ms, an anchor
        // only while running, no total for a count-up.
        let check = |s: &TimerState, what: &str| {
            assert_eq!(s.text, timer_text(s.kind, s.ms), "{what}");
            assert_eq!(
                s.anchor_ms.is_some(),
                s.phase == TimerPhase::Running,
                "{what}"
            );
            if s.kind == TimerKind::Countup {
                assert_eq!(s.total_ms, 0, "{what}");
                assert!(
                    matches!(
                        s.phase,
                        TimerPhase::Idle | TimerPhase::Running | TimerPhase::Paused
                    ),
                    "{what}"
                );
            }
        };
        let samples = f["samples"].as_object().unwrap();
        for (name, sample) in samples {
            let snap: TimerSnapshot = serde_json::from_value(sample.clone()).unwrap();
            assert_eq!(serde_json::to_value(&snap).unwrap(), *sample, "{name}");
            for s in &snap.timers {
                check(s, name);
            }
        }
        // In story order, so a test can send an older one after a newer.
        let seq = |name: &str| samples[name]["seq"].as_u64().unwrap();
        assert!(samples.len() == 3 && seq("idle") < seq("running") && seq("running") < seq("done"));
        for row in f["runSamples"].as_array().unwrap() {
            let name = row_name(row);
            let s: TimerState = serde_json::from_value(row["state"].clone()).unwrap();
            assert_eq!(serde_json::to_value(&s).unwrap(), row["state"], "{name}");
            check(&s, name);
            let run = run_from_wire(&row["run"]).unwrap_or_else(|| panic!("{name}"));
            assert_eq!(serde_json::to_value(&run).unwrap(), row["run"], "{name}");
            assert_eq!(run.phase, s.phase, "{name}");
            let seeded: TimerPhase = serde_json::from_value(row["seededPhase"].clone()).unwrap();
            assert_ne!(seeded, TimerPhase::Idle, "{name}");
            assert!(
                row["nowWallMs"].is_i64() && row["seededMs"].is_u64(),
                "{name}"
            );
        }
    }

    /// §7.6, from the fixture's `ids` table (the frontend's `isTimerId` reads
    /// the same rows).
    #[test]
    fn timer_id_table() {
        let f = fixture();
        let valid = strings(&f["ids"]["valid"]);
        let invalid = strings(&f["ids"]["invalid"]);
        for id in &valid {
            assert!(is_timer_id(id), "{id:?} should be valid");
        }
        for id in &invalid {
            assert!(!is_timer_id(id), "{id:?} should be invalid");
        }
        // The length bound is in the table on both sides of it.
        assert!(valid.contains(&"a".repeat(MAX_ID_CHARS)));
        assert!(invalid.contains(&"a".repeat(MAX_ID_CHARS + 1)));
        // Every device name, with every digit.
        for digit in 0..=9 {
            assert!(!is_timer_id(&format!("com{digit}")));
            assert!(!is_timer_id(&format!("lpt{digit}")));
        }
    }

    /// §7.7, from the fixture's `hotkeyIds` table: each row's press, and no
    /// release ever acts.
    #[test]
    fn hotkey_timer_request_table() {
        let f = fixture();
        for row in f["hotkeyIds"].as_array().unwrap() {
            let id = row["id"].as_str().unwrap();
            let want = row["timer"].as_str().map(|timer| {
                let op = match row["verb"].as_str() {
                    Some("toggle") => TimerOp::Toggle,
                    Some("reset") => TimerOp::Reset,
                    other => panic!("{id}: verb {other:?}"),
                };
                (timer, op)
            });
            assert_eq!(hotkey_timer_request(id, true), want, "{id:?} pressed");
            assert_eq!(hotkey_timer_request(id, false), None, "{id:?} released");
        }
    }

    /// §7.4: rounding up for a countdown, down for a count-up, and the hour
    /// switch.
    #[test]
    fn timer_text_table() {
        use TimerKind::{Countdown, Countup};
        for (kind, ms, text) in [
            (Countdown, 300_000, "05:00"),
            (Countdown, 299_999, "05:00"),
            (Countdown, 299_000, "04:59"),
            (Countdown, 1, "00:01"),
            (Countdown, 0, "00:00"),
            (Countdown, 3_600_000, "1:00:00"),
            (Countdown, 3_599_001, "1:00:00"),
            (Countdown, 3_599_000, "59:59"),
            (Countdown, 86_400_000, "24:00:00"),
            (Countup, 999, "00:00"),
            (Countup, 1_000, "00:01"),
            (Countup, 3_599_999, "59:59"),
            (Countup, 3_600_000, "1:00:00"),
            (Countup, 360_000_000, "100:00:00"),
        ] {
            assert_eq!(timer_text(kind, ms), text, "{kind:?} {ms}");
        }
        // No overflow at the edge.
        assert_eq!(timer_text(Countdown, u64::MAX), "5124095576030:25:52");
        assert_eq!(timer_text(Countup, u64::MAX), "5124095576030:25:51");
    }

    /// A sync entry like `good` with `key` set to `value`.
    fn entry_with(key: &str, value: Value) -> Value {
        let mut v =
            json!({"id": "timer-1", "name": "BRB", "kind": "countdown", "durationMs": 600_000});
        v[key] = value;
        v
    }

    /// `timer-1`, defined: a table row's answer.
    #[allow(clippy::unnecessary_wraps)] // the shape `entry_from_wire` answers in
    fn def(name: &str, kind: TimerKind, duration_ms: u64) -> Result<WireEntry, String> {
        Ok(WireEntry::Def(TimerDef {
            id: "timer-1".into(),
            name: name.into(),
            kind,
            duration_ms,
            resume: None,
        }))
    }

    /// §7.14, an entry at a time: what is skipped (and why), what is kept
    /// unrun, and a name that falls back. (Track A adds the sync of them
    /// all.)
    #[test]
    fn entry_from_wire_table() {
        let skipped = |why: &str| Err(why.to_string());
        let countdown = TimerKind::Countdown;
        for (entry, want) in [
            (
                entry_with("name", json!("BRB")),
                def("BRB", countdown, 600_000),
            ),
            (json!(5), skipped("not an object")),
            (
                json!({"id": 5, "kind": "countdown"}),
                skipped("no valid id"),
            ),
            (entry_with("id", json!("Timer-1")), skipped("no valid id")),
            (entry_with("id", json!("con")), skipped("no valid id")),
            (json!({"id": "timer-1"}), skipped("timer-1: no kind")),
            (entry_with("kind", json!(7)), skipped("timer-1: no kind")),
            (
                entry_with("kind", json!("interval")),
                Ok(WireEntry::Kept("timer-1".into())),
            ),
            (
                entry_with("name", Value::Null),
                def("Countdown", countdown, 600_000),
            ),
            (
                entry_with("name", json!("   ")),
                def("Countdown", countdown, 600_000),
            ),
            (
                entry_with("name", json!("  BRB  ")),
                def("BRB", countdown, 600_000),
            ),
            (
                entry_with("name", json!("é".repeat(MAX_NAME_CHARS))),
                def(&"é".repeat(MAX_NAME_CHARS), countdown, 600_000),
            ),
            (
                entry_with("name", json!("a".repeat(MAX_NAME_CHARS + 1))),
                def("Countdown", countdown, 600_000),
            ),
        ] {
            assert_eq!(entry_from_wire(&entry), want, "{entry}");
        }
    }

    /// §7.14 again: a bad length or a bad `resume` falls back without failing
    /// its definition, and fields a later release adds are ignored (D33).
    #[test]
    fn entry_from_wire_falls_back_field_by_field() {
        let countdown = TimerKind::Countdown;
        for (entry, want) in [
            (
                entry_with("durationMs", json!(-1)),
                def("BRB", countdown, MIN_LENGTH_MS),
            ),
            (
                entry_with("durationMs", json!(1.5)),
                def("BRB", countdown, MIN_LENGTH_MS),
            ),
            (
                entry_with("durationMs", json!(90_000.4)),
                def("BRB", countdown, 90_000),
            ),
            (
                entry_with("durationMs", json!(1e300)),
                def("BRB", countdown, MAX_LENGTH_MS),
            ),
            (
                entry_with("durationMs", json!("300000")),
                def("BRB", countdown, DEFAULT_LENGTH_MS),
            ),
            (
                entry_with("resume", json!({})),
                def("BRB", countdown, 600_000),
            ),
            (
                entry_with("resume", json!({"phase": "running", "anchorMs": "x"})),
                def("BRB", countdown, 600_000),
            ),
            (
                entry_with("onFire", json!([{"kind": "somethingNew"}])),
                def("BRB", countdown, 600_000),
            ),
            (
                json!({"id": "timer-1", "kind": "countup", "obsText": "x", "followObs": true}),
                def("Count-up", TimerKind::Countup, DEFAULT_LENGTH_MS),
            ),
        ] {
            assert_eq!(entry_from_wire(&entry), want, "{entry}");
        }
        let paused = json!({"phase": "paused", "ms": 5_000, "totalMs": 600_000});
        let Ok(WireEntry::Def(read)) = entry_from_wire(&entry_with("resume", paused)) else {
            panic!("a definition");
        };
        assert_eq!(
            read.resume,
            Some(TimerRun {
                phase: TimerPhase::Paused,
                ms: Some(5_000),
                total_ms: Some(600_000),
                anchor_ms: None,
            })
        );
    }

    /// The seam with the page's `defsForSync` (`timers.test.ts` reads the
    /// same `defKeys`): an entry spelled with exactly the fixture's keys has
    /// every field read, none fallen back.
    #[test]
    fn sync_entry_keys_are_the_fixtures() {
        let entry = json!({
            "id": "brb",
            "name": "Intro",
            "kind": "countup",
            "durationMs": 90_000,
            "resume": {"phase": "paused", "ms": 5_000},
        });
        assert_eq!(sorted_keys(&entry), strings(&fixture()["defKeys"]));
        assert_eq!(
            entry_from_wire(&entry),
            Ok(WireEntry::Def(TimerDef {
                id: "brb".into(),
                name: "Intro".into(),
                kind: TimerKind::Countup,
                duration_ms: 90_000,
                resume: Some(TimerRun {
                    phase: TimerPhase::Paused,
                    ms: Some(5_000),
                    total_ms: None,
                    anchor_ms: None,
                }),
            }))
        );
    }

    /// §7.8, §7.14: what makes a stored run malformed (the timer then starts
    /// idle).
    #[test]
    fn run_from_wire_table() {
        for bad in [
            json!(null),
            json!("running"),
            json!({}),
            json!({"phase": "idle"}),
            json!({"phase": "Running", "anchorMs": 1}),
            json!({"phase": "running"}),
            json!({"phase": "running", "anchorMs": null}),
            json!({"phase": "running", "anchorMs": "x"}),
            json!({"phase": "running", "anchorMs": 1.5}),
            json!({"phase": "paused", "ms": -1}),
            json!({"phase": "paused", "ms": 1.5}),
            json!({"phase": "paused", "ms": "1"}),
            json!({"phase": "done", "totalMs": -300_000}),
        ] {
            assert_eq!(run_from_wire(&bad), None, "{bad}");
        }
        assert_eq!(
            run_from_wire(&json!({"phase": "running", "anchorMs": -5, "later": 1})),
            Some(TimerRun {
                phase: TimerPhase::Running,
                ms: None,
                total_ms: None,
                anchor_ms: Some(-5),
            })
        );
    }

    /// D7's range, which `adjust_timer` checks before the core sees it.
    #[test]
    fn adjust_delta_table() {
        for bad in [
            1.5,
            -0.5,
            f64::NAN,
            f64::INFINITY,
            86_400_001.0,
            -86_400_001.0,
            1e300,
        ] {
            assert!(adjust_delta(bad).is_err(), "{bad}");
        }
        for (ok, want) in [
            (-30_000.0, -30_000),
            (30_000.0, 30_000),
            (0.0, 0),
            (86_400_000.0, 86_400_000),
            (-86_400_000.0, -86_400_000),
        ] {
            assert_eq!(adjust_delta(ok), Ok(want), "{ok}");
        }
    }

    /// S1's promise to the timer thread: an empty core — every launch, until
    /// the page's first sync — ticks, snapshots, drains and answers without
    /// reaching track A's bodies, and asks for a wait of `cap`.
    #[test]
    fn an_empty_core_is_quiet() {
        let mut core = TimerCore::default();
        let now = Duration::from_secs(5);
        core.tick(now, 1_793_217_600_000, None);
        core.tick(now, 1_793_217_600_000, Some(Duration::from_secs(1)));
        assert!(!core.take_dirty() && !core.is_dirty() && !core.synced());
        assert_eq!(core.take_transitions(), Vec::new());
        assert_eq!(core.next_wake(now, TICK), TICK);
        assert_eq!(core.text_update(), TextUpdate::default());
        assert_eq!(
            core.apply("timer-1", TimerOp::Toggle, now, 0, None),
            Err(NoTimer)
        );
        let first = core.snapshot(now);
        let second = core.snapshot(now);
        assert_eq!(first.timers, Vec::new());
        assert!(second.seq > first.seq);
        assert_eq!(NoTimer.to_string(), "no such timer");
    }
}
