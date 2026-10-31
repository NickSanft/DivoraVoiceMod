//! v1.59.0: Hourglass — countdowns and count-ups on a hotkey, as a pure core.
//!
//! This module is the rules, and only the rules: the wire shapes and limits,
//! the timer id grammar, which `tm:` hotkey ids act, the text a timer shows,
//! how a sync entry is read, and [`TimerCore`], which owns every deadline.
//! Every reader — the timer thread, a command, a `tm:` press — reads the
//! clock once under the core's lock, checks that reading for a sleep, and
//! hands both in; nothing here reads a clock, waits, touches a file, logs or
//! calls Tauri, so every rule is tested on any machine. `timer_thread.rs` is
//! the shell around it. What happens to a timer (a fire, an expiry, a sleep
//! pause, a seed; v1.60.0 adds a start and a cancel) is collected as a
//! [`Transition`], and the core's rare decisions — a sleep handled, a
//! wall-clock step followed, a duplicate id — as a [`Note`], for the timer
//! thread to drain; once it has let go of the lock it plans each
//! transition and hands its actions on (v1.60.0), and only then logs
//! (§8.1). Every transition but a sleep pause carries the timer's action
//! lists (`timer_actions.rs`).
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
use std::sync::Arc;
use std::time::Duration;

use divora_capture::sleep::Discontinuities;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::timer_actions::{hooks_from_wire, Hooks};

/// The event every change goes out as, to every window. Payload:
/// [`TimerSnapshot`].
pub const TIMER_STATE_EVENT: &str = "timer-state";

/// The hotkey ids the backend acts on for timers: `tm:<id>:toggle`,
/// `tm:<id>:reset` and `tm:<id>:finish` (v1.60.0's End now). The prefix is
/// reserved.
pub const HOTKEY_TIMER_PREFIX: &str = "tm:";

/// The timer thread's longest wait: sleep checks keep this cadence with
/// nothing running.
pub const TICK: Duration = Duration::from_millis(100);

/// D5 (b): after a sleep signal with no clock jump, a running timer whose
/// monotonic time disagrees with its wall anchor by more than this missed
/// the sleep.
pub const CLOCK_SLACK: Duration = Duration::from_secs(2);

/// D5: no (b) check this soon after an (a): that sleep was counted, and a
/// clock resync after the resume isn't another one. Nor, this soon after
/// one, does a gap of [`Discontinuities::COALESCE`] or less make another
/// (a): that is Windows' late notification meeting readers held up past
/// [`Discontinuities::THRESHOLD`], not a second sleep.
pub const RECENT_SLEEP: Duration = Duration::from_secs(60);

/// D5 (a): a sleep signal whose reading is more than this past the reading
/// before it means the clock counted the sleep. The sleep check's own bar.
const CLOCK_SLEEP: Duration = Discontinuities::THRESHOLD;

/// D3: a wall-clock step this big (wall − monotonic, in ms) …
pub const ANCHOR_SLACK_MS: i64 = 1_000;

/// … held this long moves the running timers' anchors by the step.
pub const ANCHOR_HOLD: Duration = Duration::from_secs(10);

/// A fire later than this is logged at warn, with its lateness.
pub const LATE_LOG: Duration = Duration::from_secs(1);

/// Timers the core runs: the first this many known-kind entries of a sync,
/// in list order (D36). The page adds no more (D8); more come only from a
/// hand edit or a newer Divora (after a downgrade), kept, not run (D36).
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
/// and Divora running, and fired (late, if Divora was held up), or (v1.60.0)
/// End now fired it before zero. `Expired`: it reached zero while the PC
/// slept or Divora was closed, and did not fire. A count-up is only ever
/// idle, running or paused (D2). Two edge cases break the split (D5): a
/// countdown due in a second sleep of 5 s or less within a minute of one
/// handled ends `Done`, and on a clock that doesn't count sleep, a resume
/// signal during a hold-up past 500 ms ends what came due in the hold-up
/// `Expired` with the PC awake.
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
    /// v1.60.0: a countdown's `onStart` and `onFire`, read by
    /// [`hooks_from_wire`]; a count-up's are empty (D32). Shared with every
    /// transition the timer records.
    pub hooks: Arc<Hooks>,
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
    /// v1.60.0's End now (D25): a running or paused countdown fires at once,
    /// through the one [`Transition::Fired`], not late. Anything else —
    /// idle, ended, a count-up — changes nothing.
    Finish,
}

/// v1.60.0: why an active countdown stopped without firing (D6, D10).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CancelWhy {
    /// A Reset, or a sync that changed its kind.
    Reset,
    /// A sync without its id: deleted.
    Removed,
}

/// Something that happened to a timer, wherever it happened: the thread's
/// tick, an op's or a status read's tick on the main thread, a sync's seed.
/// Collected by the core and drained only by the timer thread, so each is
/// reported exactly once (D34). v1.60.0: each that can run a timer's actions
/// carries its lists as they were when it happened (D6, D7), so a deleted
/// timer's still reach the plan.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Transition {
    /// A countdown reached zero awake and fired; `late_ms` past its deadline.
    /// v1.60.0: or End now ended it (`finished`, `late_ms` 0).
    Fired {
        id: String,
        fires: u32,
        late_ms: u64,
        finished: bool,
        hooks: Arc<Hooks>,
    },
    /// A countdown reached zero while the PC slept; not fired.
    Expired { id: String, hooks: Arc<Hooks> },
    /// A count-up paused where the PC went to sleep.
    SleepPaused { id: String },
    /// A timer the core had never accepted, seeded from its stored run.
    /// v1.60.0: `from` is the stored run's phase, so a run that ran out
    /// while Divora was closed (`running` → `expired`) is told from a stored
    /// `expired` seeded again at every launch.
    Seeded {
        id: String,
        phase: TimerPhase,
        from: TimerPhase,
        hooks: Arc<Hooks>,
    },
    /// v1.60.0: Arm took a countdown from idle, done or ran out to running
    /// (a Toggle that arms included). `fires` as it stands, for the log.
    Started {
        id: String,
        fires: u32,
        hooks: Arc<Hooks>,
    },
    /// v1.60.0: a running or paused countdown stopped without firing. A
    /// kind change's carries the lists from before the sync. `started` is
    /// the lists this run's start ran with — its Arm's, or the relaunch's
    /// that picked it up (P7) — so the undo is judged against what the start
    /// did, not an At start edited since (Q2, Q12).
    Cancelled {
        id: String,
        why: CancelWhy,
        hooks: Arc<Hooks>,
        started: Arc<Hooks>,
    },
}

/// v1.60.0: transitions held for the timer thread at most; more are dropped
/// (K13). Only a thread that never started leaves them there.
pub const MAX_TRANSITIONS: usize = 256;

/// `t` onto `transitions`, unless [`MAX_TRANSITIONS`] are waiting.
fn push_transition(transitions: &mut Vec<Transition>, t: Transition) {
    if transitions.len() < MAX_TRANSITIONS {
        transitions.push(t);
    }
}

/// One of the core's rare decisions, for the log. Collected by the core and
/// drained only by the timer thread, which logs it once it has let go of
/// the lock — so no reader, a `tm:` press inside `WM_HOTKEY` included,
/// writes the log file under it (§8.1, D34's way).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Note {
    /// D5 (a), with timers running: the PC slept `slept_ms` by the clock,
    /// which counted it; `expired` countdowns ran out in it and `paused`
    /// count-ups paused.
    ClockSlept {
        slept_ms: u64,
        running: usize,
        expired: usize,
        paused: usize,
    },
    /// D5 (b), with timers running: a sleep signal with no clock jump,
    /// each checked against the wall clock; `corrected` countdowns took the
    /// wall's left.
    WallChecked {
        running: usize,
        corrected: usize,
        expired: usize,
        paused: usize,
    },
    /// D3: a held wall-clock step of `step_ms` moved `id`'s anchor.
    AnchorFollowed { id: String, step_ms: i64 },
    /// D35: a sync entry whose id an earlier one had, skipped.
    DuplicateId { id: String },
}

/// Notes held for the timer thread at most; more are dropped. Only a thread
/// that never started leaves them there.
pub const MAX_NOTES: usize = 64;

/// `n` onto `notes`, unless [`MAX_NOTES`] are waiting.
fn push_note(notes: &mut Vec<Note>, n: Note) {
    if notes.len() < MAX_NOTES {
        notes.push(n);
    }
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
/// releases add fields (D33). v1.60.0: a countdown's `onStart` and `onFire`
/// are read by [`hooks_from_wire`], an action at a time (D4); a count-up's
/// are dropped (D32).
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
    let hooks = match kind {
        TimerKind::Countdown => hooks_from_wire(entry),
        TimerKind::Countup => Hooks::default(),
    };
    Ok(WireEntry::Def(TimerDef {
        id: id.to_string(),
        name,
        kind,
        duration_ms: length_from_wire(entry.get("durationMs")),
        resume: entry.get("resume").and_then(run_from_wire),
        hooks: Arc::new(hooks),
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
/// of `tm:<id>:toggle`, `tm:<id>:reset` or (v1.60.0, End now)
/// `tm:<id>:finish` with a valid id; `None` for a release, any other
/// spelling and any other id.
pub fn hotkey_timer_request(id: &str, pressed: bool) -> Option<(&str, TimerOp)> {
    if !pressed {
        return None;
    }
    let (timer, verb) = id.strip_prefix(HOTKEY_TIMER_PREFIX)?.split_once(':')?;
    let op = match verb {
        "toggle" => TimerOp::Toggle,
        "reset" => TimerOp::Reset,
        "finish" => TimerOp::Finish,
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

/// What a stored run seeds (§7.8): the phase, the value (a countdown's left
/// or a count-up's elapsed) and a countdown's total.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Seed {
    phase: TimerPhase,
    ms: u64,
    total_ms: u64,
}

/// §7.8, D11: what `run` seeds, at `wall_ms`, for a timer of `kind` and
/// length `length_ms` that the core has never accepted. Time passed while
/// Divora was closed: a running countdown has `anchorMs − now` left (at most
/// its total and 24 h, so a clock set back adds time, never past the total),
/// or is `expired` if that is 0 or less; a running count-up has
/// `now − anchorMs` (0 to 10 000 h). Paused, done and expired come back as
/// they were.
///
/// `None` — the timer starts idle, its definition kept (D35) — for a run
/// that can't be one: a running or paused countdown without `totalMs` (a
/// running one with a total of 0), a paused countdown outside 1 ms .. 24 h, a
/// paused count-up without `ms`, a count-up done or expired. A done or
/// expired countdown needs no total: it takes its length's.
///
/// One helper, so v1.61.0's `SetElapsed` can reuse the arithmetic (D33).
fn seed(kind: TimerKind, length_ms: u64, run: &TimerRun, wall_ms: i64) -> Option<Seed> {
    let (phase, ms, total_ms) = match (kind, run.phase) {
        (TimerKind::Countdown, TimerPhase::Running) => {
            let total = run.total_ms.filter(|&total| total > 0)?;
            let left = i128::from(run.anchor_ms?) - i128::from(wall_ms);
            if left <= 0 {
                (TimerPhase::Expired, 0, total)
            } else {
                let left = u64::try_from(left).unwrap_or(u64::MAX);
                (
                    TimerPhase::Running,
                    left.min(total).min(MAX_LENGTH_MS),
                    total,
                )
            }
        }
        (TimerKind::Countdown, TimerPhase::Paused) => {
            let ms = run.ms.filter(|ms| (1..=MAX_LENGTH_MS).contains(ms))?;
            (TimerPhase::Paused, ms, run.total_ms?.max(ms))
        }
        (TimerKind::Countdown, TimerPhase::Done | TimerPhase::Expired) => {
            (run.phase, 0, run.total_ms.unwrap_or(length_ms))
        }
        (TimerKind::Countup, TimerPhase::Running) => {
            let elapsed = i128::from(wall_ms) - i128::from(run.anchor_ms?);
            let elapsed = u64::try_from(elapsed.max(0)).unwrap_or(u64::MAX);
            (TimerPhase::Running, elapsed.min(MAX_SEED_ELAPSED_MS), 0)
        }
        (TimerKind::Countup, TimerPhase::Paused) => {
            (TimerPhase::Paused, run.ms?.min(MAX_SEED_ELAPSED_MS), 0)
        }
        _ => return None,
    };
    Some(Seed {
        phase,
        ms,
        total_ms,
    })
}

/// One timer the core runs.
///
/// A running timer holds `since` (the clock reading its run was last set
/// at) and `base_ms` (a countdown's left at `since`, rounded up; a
/// count-up's elapsed at `since`, rounded down), so nothing is ever a
/// negative `Duration`; a countdown's deadline is `since + base_ms`. Paused,
/// `base_ms` is the value it stopped at; idle, a countdown's length or a
/// count-up's 0; done or expired, 0.
#[derive(Debug)]
struct Timer {
    id: String,
    name: String,
    kind: TimerKind,
    /// The definition's length (a countdown's), [`MIN_LENGTH_MS`] to
    /// [`MAX_LENGTH_MS`].
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
    /// When a wall step past [`ANCHOR_SLACK_MS`] was first seen, while it
    /// lasts.
    step_since: Option<Duration>,
    fires: u32,
    /// The text as last computed: every transition recomputes it, and a
    /// running timer's at each tick and snapshot.
    text: String,
    /// v1.60.0: the definition's action lists, the latest sync's (D7); each
    /// transition takes a share.
    hooks: Arc<Hooks>,
    /// v1.60.0: the lists this run started with — set by a countdown's Arm
    /// and by a seed, kept by a sync — which a cancel's undo is judged
    /// against (Q2: Reset and delete undo only what the start did). Empty
    /// before any start.
    started: Arc<Hooks>,
}

impl Timer {
    /// Idle, as `def` defines it, with `fires` from 0: its id just entered
    /// the set.
    fn new(def: &TimerDef) -> Self {
        let mut timer = Self {
            id: def.id.clone(),
            name: def.name.clone(),
            kind: def.kind,
            length_ms: def.duration_ms.clamp(MIN_LENGTH_MS, MAX_LENGTH_MS),
            phase: TimerPhase::Idle,
            since: Duration::ZERO,
            base_ms: 0,
            total_ms: 0,
            anchor_ms: None,
            anchor_offset_ms: 0,
            step_since: None,
            fires: 0,
            text: String::new(),
            hooks: Arc::clone(&def.hooks),
            started: Arc::default(),
        };
        timer.reset(Duration::ZERO);
        timer
    }

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

    /// Recomputes its text at `now`; whether it changed.
    fn retext(&mut self, now: Duration) -> bool {
        let text = timer_text(self.kind, self.ms_at(now));
        let changed = text != self.text;
        self.text = text;
        changed
    }

    /// Running from `ms` (a countdown's left, a count-up's elapsed) at
    /// `now`, with a new anchor: the wall ms a countdown ends at, or a
    /// count-up effectively started at (D3).
    fn run_from(&mut self, ms: u64, now: Duration, wall_ms: i64) {
        self.phase = TimerPhase::Running;
        self.since = now;
        self.base_ms = ms;
        self.anchor_ms = Some(match self.kind {
            TimerKind::Countdown => wall_ms.saturating_add(signed(ms)),
            TimerKind::Countup => wall_ms.saturating_sub(signed(ms)),
        });
        self.anchor_offset_ms = wall_ms.saturating_sub(clock_ms(now));
        self.step_since = None;
        self.retext(now);
    }

    /// Stopped at `ms` in `phase` (idle, paused, done, expired): no anchor.
    fn stop(&mut self, phase: TimerPhase, ms: u64, now: Duration) {
        self.phase = phase;
        self.base_ms = ms;
        self.anchor_ms = None;
        self.step_since = None;
        self.retext(now);
    }

    /// Idle: a countdown at its length, which is its total too; a count-up
    /// at 0.
    fn reset(&mut self, now: Duration) {
        let ms = match self.kind {
            TimerKind::Countdown => self.length_ms,
            TimerKind::Countup => 0,
        };
        self.total_ms = ms;
        self.stop(TimerPhase::Idle, ms, now);
    }

    /// D4: the one way a countdown gets to `done` (a seed only puts a stored
    /// `done` back, `fires` 0) — a running one found at or past its
    /// deadline, however late, or (v1.60.0, D25) a running or paused one
    /// `finished` by End now. `fires` + 1, so "fires once" covers both. A
    /// finish is never late: a paused countdown's deadline is stale.
    fn fire(&mut self, now: Duration, finished: bool) -> Transition {
        let late_ms = if finished {
            0
        } else {
            floor_ms(now.saturating_sub(self.deadline()))
        };
        self.fires = self.fires.saturating_add(1);
        self.stop(TimerPhase::Done, 0, now);
        Transition::Fired {
            id: self.id.clone(),
            fires: self.fires,
            late_ms,
            finished,
            hooks: Arc::clone(&self.hooks),
        }
    }

    /// D2, D5: a countdown that ran out while the PC slept: `expired`, not
    /// fired.
    fn expire(&mut self, now: Duration) -> Transition {
        self.stop(TimerPhase::Expired, 0, now);
        Transition::Expired {
            id: self.id.clone(),
            hooks: Arc::clone(&self.hooks),
        }
    }

    /// D5, Q2: a count-up paused at `before`, the last reading before the PC
    /// slept.
    fn sleep_pause(&mut self, before: Duration, now: Duration) -> Transition {
        let ms = self.ms_at(before);
        self.stop(TimerPhase::Paused, ms, now);
        Transition::SleepPaused {
            id: self.id.clone(),
        }
    }

    /// The seed of a stored run (§7.8). v1.60.0: a run seeded running or
    /// paused started, as far as this process goes, with the lists it has
    /// now — the ones its restart re-applies (P7).
    fn take_seed(&mut self, seed: Seed, now: Duration, wall_ms: i64) {
        self.started = Arc::clone(&self.hooks);
        self.total_ms = seed.total_ms;
        if seed.phase == TimerPhase::Running {
            self.run_from(seed.ms, now, wall_ms);
        } else {
            self.stop(seed.phase, seed.ms, now);
        }
    }

    /// D15: a sync's definition for a timer the core already runs. A new
    /// name or length changes only what it names — an idle countdown shows
    /// its new length; one that runs, is paused or has ended keeps its run
    /// until it starts again — and a new kind makes it idle. v1.60.0: the
    /// lists are the definition's from now on, and a change to them keeps
    /// the run too (D7) — and what the run started with, which a cancel
    /// undoes. A new kind on an active countdown cancels it as a Reset
    /// does, with the lists it had before (D6): the new def's are a
    /// count-up's, which has none (D32).
    fn redefine(&mut self, def: &TimerDef, now: Duration) -> Option<Transition> {
        self.name.clone_from(&def.name);
        self.length_ms = def.duration_ms.clamp(MIN_LENGTH_MS, MAX_LENGTH_MS);
        let mut cancelled = None;
        if def.kind != self.kind || self.phase == TimerPhase::Idle {
            if def.kind != self.kind && self.active() {
                cancelled = Some(self.cancel(CancelWhy::Reset));
            }
            self.kind = def.kind;
            self.reset(now);
        }
        self.hooks = Arc::clone(&def.hooks);
        cancelled
    }

    /// v1.60.0: a countdown that has started and not ended — running or
    /// paused — whose end has an undo to run (D10).
    fn active(&self) -> bool {
        self.kind == TimerKind::Countdown
            && matches!(self.phase, TimerPhase::Running | TimerPhase::Paused)
    }

    /// v1.60.0: what stopping it without a fire records, with its lists as
    /// they are and the ones its run started with.
    fn cancel(&self, why: CancelWhy) -> Transition {
        Transition::Cancelled {
            id: self.id.clone(),
            why,
            hooks: Arc::clone(&self.hooks),
            started: Arc::clone(&self.started),
        }
    }

    /// D6 (§7.1): `op` at `now`, after the tick. Labelled ops do only what
    /// they name; Toggle decides from the phase. Whether anything changed.
    /// Never given `Finish`: the core fires it (D25). What a start or a
    /// cancel records is the core's too (§7.8).
    fn apply(&mut self, op: TimerOp, now: Duration, wall_ms: i64) -> bool {
        use TimerPhase::{Done, Expired, Idle, Paused, Running};
        let op = match (op, self.phase) {
            (TimerOp::Toggle, Idle | Done | Expired) => TimerOp::Arm,
            (TimerOp::Toggle, Running) => TimerOp::Pause,
            (TimerOp::Toggle, Paused) => TimerOp::Resume,
            (op, _) => op,
        };
        match (op, self.phase) {
            (TimerOp::Arm, Idle | Done | Expired) => {
                // From the full length (a count-up from 0), its total reset.
                self.reset(now);
                self.run_from(self.base_ms, now, wall_ms);
            }
            (TimerOp::Pause, Running) => {
                let ms = self.ms_at(now);
                self.stop(Paused, ms, now);
            }
            (TimerOp::Resume, Paused) => self.run_from(self.base_ms, now, wall_ms),
            (TimerOp::Reset, Running | Paused | Done | Expired) => self.reset(now),
            (TimerOp::Adjust(delta_ms), Running | Paused) => {
                return self.adjust(delta_ms, now, wall_ms);
            }
            _ => return false,
        }
        true
    }

    /// D7 (§7.3): a running or paused timer moved by `delta_ms`. A countdown
    /// keeps at most 24 h left and, going down, at least 1 s — a left
    /// already under 1 s stays as it is — so −30 s never ends one (End now
    /// is v1.60.0's deliberate way); its total moves by what its left moved,
    /// so it can pass 24 h. A count-up stops at 0. A running timer runs on
    /// from the new value, rounded as D22 says, with a new anchor. Whether
    /// anything changed.
    fn adjust(&mut self, delta_ms: i64, now: Duration, wall_ms: i64) -> bool {
        let ms = self.ms_at(now);
        let by = delta_ms.unsigned_abs();
        let next = match (self.kind, delta_ms >= 0) {
            // Never lower than it was, whatever the cap.
            (TimerKind::Countdown, true) => ms.saturating_add(by).min(MAX_LENGTH_MS).max(ms),
            // The shortest length is the floor; under it already, no lower.
            (TimerKind::Countdown, false) => ms.saturating_sub(by).max(ms.min(MIN_LENGTH_MS)),
            (TimerKind::Countup, true) => ms.saturating_add(by),
            (TimerKind::Countup, false) => ms.saturating_sub(by),
        };
        if next == ms {
            return false;
        }
        if self.kind == TimerKind::Countdown {
            self.total_ms = if next > ms {
                self.total_ms.saturating_add(next - ms)
            } else {
                self.total_ms.saturating_sub(ms - next)
            };
        }
        if self.phase == TimerPhase::Running {
            self.run_from(next, now, wall_ms);
        } else {
            self.base_ms = next;
            self.retext(now);
        }
        true
    }

    /// §7.2, D3: a wall-clock step past [`ANCHOR_SLACK_MS`] — the wall less
    /// the clock, against what it was when the anchor was set — that still
    /// holds [`ANCHOR_HOLD`] after it was first seen moves the anchor by the
    /// step, so a restart doesn't move the timer by a step taken while it ran
    /// (a resync after a resume, the user setting the clock). The hold lets
    /// a late sleep signal's (b) go first. A step that goes back within the
    /// slack is forgotten. The live time runs on the clock and doesn't move.
    /// The step the anchor moved by, if it moved.
    fn follow_wall(&mut self, now: Duration, wall_ms: i64) -> Option<i64> {
        let (TimerPhase::Running, Some(anchor)) = (self.phase, self.anchor_ms) else {
            return None;
        };
        let step = wall_ms
            .saturating_sub(clock_ms(now))
            .saturating_sub(self.anchor_offset_ms);
        if step.unsigned_abs() <= ANCHOR_SLACK_MS.unsigned_abs() {
            self.step_since = None;
            return None;
        }
        match self.step_since {
            None => {
                self.step_since = Some(now);
                None
            }
            Some(seen) if now.saturating_sub(seen) >= ANCHOR_HOLD => {
                self.anchor_ms = Some(anchor.saturating_add(step));
                self.anchor_offset_ms = self.anchor_offset_ms.saturating_add(step);
                self.step_since = None;
                Some(step)
            }
            Some(_) => None,
        }
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

/// `ms` as a signed count, saturating.
fn signed(ms: u64) -> i64 {
    i64::try_from(ms).unwrap_or(i64::MAX)
}

/// The clock in whole ms, to set against the wall's.
fn clock_ms(now: Duration) -> i64 {
    signed(floor_ms(now))
}

/// `d` in ns, signed, for the (b) comparisons.
fn nanos(d: Duration) -> i128 {
    i128::try_from(d.as_nanos()).unwrap_or(i128::MAX)
}

/// The timers and every deadline (D4-D11, D34-D36).
///
/// Every entry point runs [`TimerCore::tick`] first, with the one clock
/// reading and the sleep check its caller took under the core's lock:
/// `now` (the monotonic clock), `wall_ms` (Unix ms) and `slept_from`
/// (`Some(before)` when the PC slept since the last reader: `before` is the
/// last reading anyone took awake). The sleep check reports each sleep once,
/// to whichever reader comes first, and the core acts on it then — with no
/// timers, only noting it for [`RECENT_SLEEP`] — so a sleep never reaches a
/// timer added after it. Every arithmetic step saturates: a panic aborts the
/// app.
#[derive(Debug)]
pub struct TimerCore {
    /// In sync order.
    timers: Vec<Timer>,
    /// Ids of entries kept, not run (D36): their files stay.
    kept: Vec<String>,
    /// Ids accepted this process: only these ignore a stored run (D10).
    seen: HashSet<String>,
    transitions: Vec<Transition>,
    /// At most [`MAX_NOTES`].
    notes: Vec<Note>,
    seq: u64,
    dirty: bool,
    synced: bool,
    /// When a clock-counted sleep (D5 (a)) was last handled: no (b), and no
    /// (a) from a gap of [`Discontinuities::COALESCE`] or less, for
    /// [`RECENT_SLEEP`] after it.
    last_clock_sleep: Option<Duration>,
}

impl TimerCore {
    pub fn new() -> Self {
        Self {
            timers: Vec::new(),
            kept: Vec::new(),
            seen: HashSet::new(),
            transitions: Vec::new(),
            notes: Vec::new(),
            seq: 0,
            dirty: false,
            synced: false,
            last_clock_sleep: None,
        }
    }

    /// D5, D3, D4 (§7.2), for running timers, in order: the sleep rule
    /// ((a) a clock-counted sleep, (b) a signal the clock missed, checked
    /// against the wall anchors), held wall steps (only with no sleep),
    /// settle (fires each due countdown once, however late, `fires` +1,
    /// [`Transition::Fired`]), then texts (dirty when one changed). With no
    /// timers it still notes an (a), so the 60 s rule covers timers added
    /// soon after.
    pub fn tick(&mut self, now: Duration, wall_ms: i64, slept_from: Option<Duration>) {
        match slept_from {
            Some(before) if self.clock_counted(now, before) => {
                self.clock_slept(now, before);
            }
            Some(before) => {
                // Within a minute of an (a), that sleep was counted, and a
                // clock resync after the resume isn't another one.
                if self
                    .last_clock_sleep
                    .is_none_or(|at| now.saturating_sub(at) >= RECENT_SLEEP)
                {
                    self.wall_slept(now, wall_ms, before);
                }
            }
            None => {
                for t in &mut self.timers {
                    if let Some(step_ms) = t.follow_wall(now, wall_ms) {
                        self.dirty = true;
                        let id = t.id.clone();
                        push_note(&mut self.notes, Note::AnchorFollowed { id, step_ms });
                    }
                }
            }
        }
        for t in &mut self.timers {
            if t.phase != TimerPhase::Running {
                continue;
            }
            if t.kind == TimerKind::Countdown && t.deadline() <= now {
                push_transition(&mut self.transitions, t.fire(now, false));
                self.dirty = true;
            } else if t.retext(now) {
                self.dirty = true;
            }
        }
    }

    /// D5 (a): whether a sleep signal at `now`, whose reading before was
    /// `before`, means the clock counted a sleep: a gap past
    /// [`CLOCK_SLEEP`], except — within [`RECENT_SLEEP`] of the last (a) —
    /// one of [`Discontinuities::COALESCE`] or less. Windows' notification
    /// can come more than 5 s after the jump the sleep check reported, as a
    /// second signal; if the readers were held up past [`CLOCK_SLEEP`] just
    /// then (a PC busy after its resume), the gap is that hold, not a sleep,
    /// and taking it for one would expire countdowns and pause count-ups
    /// with the PC awake. A real second sleep that short counts as awake
    /// time instead: its countdowns fire late, its count-ups count it.
    fn clock_counted(&self, now: Duration, before: Duration) -> bool {
        let gap = now.saturating_sub(before);
        gap > CLOCK_SLEEP
            && self.last_clock_sleep.is_none_or(|at| {
                now.saturating_sub(at) >= RECENT_SLEEP || gap > Discontinuities::COALESCE
            })
    }

    /// D5 (a): the PC slept from `before`, and the clock counted it. A
    /// countdown whose deadline fell in the sleep expires, unfired (the
    /// check at `before` would have fired one due by then); one still ahead
    /// keeps running, the sleep counted; a count-up pauses at `before` (Q2).
    fn clock_slept(&mut self, now: Duration, before: Duration) {
        self.last_clock_sleep = Some(now);
        let (mut running, mut expired, mut paused) = (0_usize, 0_usize, 0_usize);
        for t in &mut self.timers {
            if t.phase != TimerPhase::Running {
                continue;
            }
            running += 1;
            let ended = match t.kind {
                TimerKind::Countdown if t.deadline() <= now => {
                    expired += 1;
                    t.expire(now)
                }
                TimerKind::Countdown => continue,
                TimerKind::Countup => {
                    paused += 1;
                    t.sleep_pause(before, now)
                }
            };
            push_transition(&mut self.transitions, ended);
            self.dirty = true;
        }
        if running > 0 {
            let slept_ms = floor_ms(now.saturating_sub(before));
            push_note(
                &mut self.notes,
                Note::ClockSlept {
                    slept_ms,
                    running,
                    expired,
                    paused,
                },
            );
        }
    }

    /// D5 (b): a sleep signal with no clock jump — Windows' notification
    /// arriving late, or a clock that didn't count the sleep. Each running
    /// timer is checked against its wall anchor. A countdown with more than
    /// [`CLOCK_SLACK`] more left on the clock than on the wall missed the
    /// sleep: `expired` if the wall says it has ended, else running on with
    /// the wall's left. A count-up the wall says has counted more than
    /// [`CLOCK_SLACK`] beyond the clock pauses at `before`. Agreement changes
    /// nothing.
    fn wall_slept(&mut self, now: Duration, wall_ms: i64, before: Duration) {
        const NS_PER_MS: i128 = 1_000_000;
        let slack = nanos(CLOCK_SLACK);
        let (mut running, mut corrected, mut expired, mut paused) =
            (0_usize, 0_usize, 0_usize, 0_usize);
        for t in &mut self.timers {
            let (TimerPhase::Running, Some(anchor)) = (t.phase, t.anchor_ms) else {
                continue;
            };
            running += 1;
            match t.kind {
                TimerKind::Countdown => {
                    let clock_left = nanos(t.deadline()) - nanos(now);
                    let wall_left_ms = i128::from(anchor) - i128::from(wall_ms);
                    if clock_left - wall_left_ms * NS_PER_MS <= slack {
                        continue;
                    }
                    if wall_left_ms <= 0 {
                        expired += 1;
                        push_transition(&mut self.transitions, t.expire(now));
                    } else {
                        corrected += 1;
                        let left = u64::try_from(wall_left_ms).unwrap_or(u64::MAX);
                        t.run_from(left, now, wall_ms);
                    }
                }
                TimerKind::Countup => {
                    let clock_elapsed = nanos(
                        Duration::from_millis(t.base_ms)
                            .saturating_add(now.saturating_sub(t.since)),
                    );
                    let wall_elapsed = (i128::from(wall_ms) - i128::from(anchor)) * NS_PER_MS;
                    if wall_elapsed - clock_elapsed <= slack {
                        continue;
                    }
                    paused += 1;
                    push_transition(&mut self.transitions, t.sleep_pause(before, now));
                }
            }
            self.dirty = true;
        }
        if running > 0 {
            push_note(
                &mut self.notes,
                Note::WallChecked {
                    running,
                    corrected,
                    expired,
                    paused,
                },
            );
        }
    }

    /// D15, D35, D36: the set becomes `entries`, in order. `tick` first. The
    /// first [`MAX_TIMERS`] definitions run; the rest, and every
    /// [`WireEntry::Kept`], are kept, not run; duplicates after the first are
    /// skipped, each with a [`Note::DuplicateId`]. A timer the core already
    /// runs keeps its run (a rename or a new length changes only what it
    /// names; a kind change makes it idle); an id it doesn't run starts idle
    /// with `fires` 0, or —
    /// if the core has never accepted that id in this process — from its
    /// `resume` (§7.8, [`Transition::Seeded`]). Marks dirty and synced.
    /// Returns the ids that left, in their old order.
    ///
    /// v1.60.0: every definition's lists replace the ones its timer had,
    /// and a change to them records nothing (D7). An active countdown that
    /// stops running — its kind changed, or its id gone (deleted, or no
    /// longer among the first [`MAX_TIMERS`]) — records a
    /// [`Transition::Cancelled`] with the lists it had and the ones its run
    /// started with (D6, D10).
    pub fn sync(
        &mut self,
        entries: Vec<WireEntry>,
        now: Duration,
        wall_ms: i64,
        slept_from: Option<Duration>,
    ) -> Vec<String> {
        self.tick(now, wall_ms, slept_from);
        let mut before = std::mem::take(&mut self.timers);
        let mut ids = HashSet::new();
        self.kept.clear();
        for entry in entries {
            let id = match &entry {
                WireEntry::Def(def) => &def.id,
                WireEntry::Kept(id) => id,
            };
            if !ids.insert(id.clone()) {
                let id = id.clone();
                push_note(&mut self.notes, Note::DuplicateId { id });
                continue;
            }
            let def = match entry {
                WireEntry::Def(def) if self.timers.len() < MAX_TIMERS => def,
                // Past the 16th: kept, and not accepted, so its stored run
                // still seeds it if it comes to run.
                WireEntry::Def(TimerDef { id, .. }) | WireEntry::Kept(id) => {
                    self.kept.push(id);
                    continue;
                }
            };
            let timer = match before.iter().position(|t| t.id == def.id) {
                Some(i) => {
                    let mut timer = before.remove(i);
                    if let Some(cancelled) = timer.redefine(&def, now) {
                        push_transition(&mut self.transitions, cancelled);
                    }
                    timer
                }
                None => self.accept(&def, now, wall_ms),
            };
            self.timers.push(timer);
        }
        for t in before.iter().filter(|t| t.active()) {
            push_transition(&mut self.transitions, t.cancel(CancelWhy::Removed));
        }
        self.dirty = true;
        self.synced = true;
        before.into_iter().map(|t| t.id).collect()
    }

    /// A definition whose id the set didn't have: idle with `fires` 0 (D33:
    /// fires count per definition), unless this is the core's first
    /// acceptance of the id in this process and it carries a stored run that
    /// seeds one (D10). An id accepted before — deleted and added again —
    /// ignores its run: Rust wins.
    fn accept(&mut self, def: &TimerDef, now: Duration, wall_ms: i64) -> Timer {
        let mut timer = Timer::new(def);
        let first = self.seen.insert(def.id.clone());
        let seeded = def.resume.as_ref().filter(|_| first).and_then(|run| {
            seed(timer.kind, timer.length_ms, run, wall_ms).map(|seeded| (run.phase, seeded))
        });
        if let Some((from, seeded)) = seeded {
            timer.take_seed(seeded, now, wall_ms);
            push_transition(
                &mut self.transitions,
                Transition::Seeded {
                    id: timer.id.clone(),
                    phase: timer.phase,
                    from,
                    hooks: Arc::clone(&timer.hooks),
                },
            );
        }
        timer
    }

    /// D6, D7 (§7.1, §7.3): `op` on the timer `id`. `tick` first, so a due
    /// countdown fires before the op (a Toggle then starts it again), and a
    /// sleep is handled before anything starts. A labelled op that doesn't
    /// fit the phase changes nothing. `Err` for an id the core doesn't run.
    ///
    /// v1.60.0 (D25): `Finish` fires a running or paused countdown through
    /// the one `fire()`, `finished` and not late. The tick comes
    /// first, so a Finish on one already past a deadline no reader saw
    /// finds that fire taken — a plain `Fired` with its real lateness — and
    /// adds nothing.
    ///
    /// v1.60.0 (D6, §7.8): a countdown armed from idle, done or ran out —
    /// Start, Start again, a Toggle that arms — records
    /// [`Transition::Started`], and its lists are what this run started
    /// with; a Resume doesn't, since it isn't a start. A
    /// Reset of a running or paused countdown records
    /// [`Transition::Cancelled`]; of an ended one, nothing (it already
    /// ended). A Toggle that finds its countdown due records the tick's
    /// `Fired`, then `Started`. A count-up records neither (D32).
    pub fn apply(
        &mut self,
        id: &str,
        op: TimerOp,
        now: Duration,
        wall_ms: i64,
        slept_from: Option<Duration>,
    ) -> Result<(), NoTimer> {
        use TimerPhase::{Done, Expired, Idle, Paused, Running};
        self.tick(now, wall_ms, slept_from);
        let timer = self.timers.iter_mut().find(|t| t.id == id).ok_or(NoTimer)?;
        if op == TimerOp::Finish {
            if timer.active() {
                push_transition(&mut self.transitions, timer.fire(now, true));
                self.dirty = true;
            }
            return Ok(());
        }
        let before = timer.phase;
        if !timer.apply(op, now, wall_ms) {
            return Ok(());
        }
        self.dirty = true;
        if timer.kind != TimerKind::Countdown {
            return Ok(());
        }
        let recorded = match (before, timer.phase) {
            (Idle | Done | Expired, Running) => {
                timer.started = Arc::clone(&timer.hooks);
                Some(Transition::Started {
                    id: timer.id.clone(),
                    fires: timer.fires,
                    hooks: Arc::clone(&timer.hooks),
                })
            }
            (Running | Paused, Idle) => Some(timer.cancel(CancelWhy::Reset)),
            _ => None,
        };
        if let Some(t) = recorded {
            push_transition(&mut self.transitions, t);
        }
        Ok(())
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

    /// Every note since the last call (up to [`MAX_NOTES`]), in order. Only
    /// the timer thread calls it, and logs them unlocked.
    pub fn take_notes(&mut self) -> Vec<Note> {
        std::mem::take(&mut self.notes)
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
    use super::TimerPhase::{Done, Expired, Idle, Paused, Running};
    use super::{
        adjust_delta, entry_from_wire, hotkey_timer_request, is_timer_id, run_from_wire,
        timer_text, CancelWhy, NoTimer, Note, TextUpdate, TimerCore, TimerDef, TimerKind, TimerOp,
        TimerPhase, TimerRun, TimerSnapshot, TimerState, Transition, WireEntry, DEFAULT_LENGTH_MS,
        MAX_ID_CHARS, MAX_LENGTH_MS, MAX_NAME_CHARS, MAX_NOTES, MAX_SEED_ELAPSED_MS,
        MAX_SYNC_ENTRIES, MAX_TIMERS, MAX_TRANSITIONS, MIN_LENGTH_MS, TICK, TIMER_STATE_EVENT,
    };
    use crate::timer_actions::{
        hooks_from_wire, plan, replay_at, Action, Hook, Hooks, TimerActionsEvent, Why,
        LATE_ONE_SHOTS_MS, MAX_ACTIONS, MAX_LINE_CHARS, MAX_TARGET_CHARS, SOUNDBOARD_STOPPED_EVENT,
        TIMER_ACTIONS_EVENT,
    };
    use serde_json::{json, Value};
    use std::sync::Arc;
    use std::time::{Duration, Instant};

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
    /// as the rules give them, and each run reads back unchanged. v1.60.0's
    /// names, keys and limits too.
    #[test]
    #[allow(clippy::too_many_lines)] // one fixture, every key
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

        // v1.60.0's names, keys and limits, each under a key of its own so
        // the v1.59.0 objects above stay as `timerState.test.ts` pins them.
        // (`actionWire.commands.finish` is checked against the registered
        // commands, beside `commands.status`.)
        let wire = &f["actionWire"];
        assert_eq!(wire["events"]["actions"], TIMER_ACTIONS_EVENT);
        assert_eq!(
            wire["events"]["soundboardStopped"],
            SOUNDBOARD_STOPPED_EVENT
        );
        let event = TimerActionsEvent {
            seq: 1,
            id: "timer-1".into(),
            hook: Hook::End,
            why: Why::Zero,
            late_ms: 0,
            actions: vec![Action::RevertPreset.to_wire()],
        };
        assert_eq!(
            sorted_keys(&serde_json::to_value(&event).unwrap()),
            strings(&f["actionsEventKeys"])
        );
        // Every `why` the page can hear, in order; a restart's re-mute is
        // never sent.
        let whys = [
            Why::Armed,
            Why::Zero,
            Why::EndNow,
            Why::Late,
            Why::RanOut,
            Why::Reset,
            Why::Deleted,
        ]
        .map(|w| serde_json::to_value(w).unwrap());
        assert_eq!(Value::from(whys.to_vec()), f["whys"]);
        assert!(!f["whys"]
            .as_array()
            .unwrap()
            .contains(&serde_json::to_value(Why::Restart).unwrap()));
        let hooks = [Hook::Start, Hook::End].map(|h| serde_json::to_value(h).unwrap());
        assert_eq!(Value::from(hooks.to_vec()), f["hooks"]);
        let action_limits = &f["actionLimits"];
        assert_eq!(action_limits["maxActions"], MAX_ACTIONS);
        assert_eq!(action_limits["maxLineChars"], MAX_LINE_CHARS);
        assert_eq!(action_limits["maxTargetChars"], MAX_TARGET_CHARS);
        assert_eq!(action_limits["lateOneShotsMs"], LATE_ONE_SHOTS_MS);

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
                    Some("finish") => TimerOp::Finish,
                    other => panic!("{id}: verb {other:?}"),
                };
                (timer, op)
            });
            assert_eq!(hotkey_timer_request(id, true), want, "{id:?} pressed");
            assert_eq!(hotkey_timer_request(id, false), None, "{id:?} released");
        }
        // v1.60.0: End now's key is in the table, and acts.
        assert!(f["hotkeyIds"]
            .as_array()
            .unwrap()
            .iter()
            .any(|row| row["verb"] == "finish"));
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
            hooks: Arc::default(),
        }))
    }

    /// §7.14, an entry at a time: what is skipped (and why), what is kept
    /// unrun, and a name that falls back. One sync of every kind of bad
    /// entry beside a good one, through `sync_timers`'s own reader, is
    /// `timer_thread`'s `sync_skips_bad_entries_and_stops_at_64`.
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
    /// v1.60.0: an action of a kind this build doesn't know is skipped, and
    /// a count-up's lists are dropped (D32), so both defs have none.
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
                json!({
                    "id": "timer-1", "kind": "countup", "obsText": "x", "followObs": true,
                    "onStart": [{"kind": "setMuted", "muted": true}],
                    "onFire": [{"kind": "setMuted", "muted": false}],
                }),
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
    /// every field read, none fallen back. v1.60.0: a countdown, since only
    /// a countdown's lists are sent and read (D32), with a backend action at
    /// the start and a page action at zero.
    #[test]
    fn sync_entry_keys_are_the_fixtures() {
        let entry = json!({
            "id": "brb",
            "name": "Intro",
            "kind": "countdown",
            "durationMs": 90_000,
            "resume": {"phase": "paused", "ms": 5_000, "totalMs": 90_000},
            "onStart": [{"kind": "setMuted", "muted": true}],
            "onFire": [{"kind": "preset", "presetId": "x"}],
        });
        assert_eq!(sorted_keys(&entry), strings(&fixture()["defKeys"]));
        assert_eq!(
            entry_from_wire(&entry),
            Ok(WireEntry::Def(TimerDef {
                id: "brb".into(),
                name: "Intro".into(),
                kind: TimerKind::Countdown,
                duration_ms: 90_000,
                resume: Some(TimerRun {
                    phase: TimerPhase::Paused,
                    ms: Some(5_000),
                    total_ms: Some(90_000),
                    anchor_ms: None,
                }),
                hooks: Arc::new(Hooks {
                    on_start: vec![Action::SetMuted { muted: true }],
                    on_fire: vec![Action::Preset {
                        preset_id: "x".into()
                    }],
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

    /// The timer thread's every launch: an empty core — until the page's
    /// first sync, or for good with no `divora.timers` — ticks (a sleep
    /// included), snapshots, drains and answers quietly, and asks for a wait
    /// of `cap`.
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

    // ---- The core (§10.1). Times are on the core's clock; the wall is
    // ---- `WALL0` at clock 0 and keeps step with it unless a test says.

    /// The fixture's T0: the wall clock when the core's clock reads 0.
    const WALL0: i64 = 1_793_217_600_000;

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    fn secs(n: u64) -> Duration {
        Duration::from_secs(n)
    }

    /// The wall clock of an awake PC nobody set: in step with the core's.
    fn wall(now: Duration) -> i64 {
        WALL0 + i64::try_from(now.as_millis()).unwrap()
    }

    /// A countdown sync entry named after its id.
    fn countdown(id: &str, length_ms: u64) -> WireEntry {
        WireEntry::Def(TimerDef {
            id: id.into(),
            name: id.into(),
            kind: TimerKind::Countdown,
            duration_ms: length_ms,
            resume: None,
            hooks: Arc::default(),
        })
    }

    /// A count-up sync entry named after its id.
    fn countup(id: &str) -> WireEntry {
        WireEntry::Def(TimerDef {
            id: id.into(),
            name: id.into(),
            kind: TimerKind::Countup,
            duration_ms: DEFAULT_LENGTH_MS,
            resume: None,
            hooks: Arc::default(),
        })
    }

    /// `entry`, its definition changed.
    fn edit(entry: WireEntry, change: impl FnOnce(&mut TimerDef)) -> WireEntry {
        let WireEntry::Def(mut def) = entry else {
            panic!("a definition");
        };
        change(&mut def);
        WireEntry::Def(def)
    }

    /// A stored run.
    fn stored(
        phase: TimerPhase,
        ms: Option<u64>,
        total: Option<u64>,
        anchor: Option<i64>,
    ) -> TimerRun {
        TimerRun {
            phase,
            ms,
            total_ms: total,
            anchor_ms: anchor,
        }
    }

    /// A core running `entries`, synced at 0, the sync's dirt and seeds
    /// already taken.
    fn core_with(entries: Vec<WireEntry>) -> TimerCore {
        let mut core = TimerCore::new();
        core.sync(entries, Duration::ZERO, WALL0, None);
        core.take_dirty();
        core.take_transitions();
        core
    }

    /// An awake reader's tick at `now`.
    fn tick(core: &mut TimerCore, now: Duration) {
        core.tick(now, wall(now), None);
    }

    /// An awake reader's `op` at `now` on a timer the core runs.
    fn op(core: &mut TimerCore, id: &str, op: TimerOp, now: Duration) {
        core.apply(id, op, now, wall(now), None).unwrap();
    }

    /// `id` as a snapshot at `now` shows it.
    fn state(core: &mut TimerCore, now: Duration, id: &str) -> TimerState {
        core.snapshot(now)
            .timers
            .into_iter()
            .find(|t| t.id == id)
            .unwrap_or_else(|| panic!("{id} runs"))
    }

    /// `id`'s phase, `ms` and text at `now`.
    fn shown(core: &mut TimerCore, now: Duration, id: &str) -> (TimerPhase, u64, String) {
        let s = state(core, now, id);
        (s.phase, s.ms, s.text)
    }

    /// A fire at a deadline (not End now) of a timer with no lists.
    fn fired(id: &str, fires: u32, late_ms: u64) -> Transition {
        Transition::Fired {
            id: id.into(),
            fires,
            late_ms,
            finished: false,
            hooks: Arc::default(),
        }
    }

    /// An expiry of a timer with no lists.
    fn expired(id: &str) -> Transition {
        Transition::Expired {
            id: id.into(),
            hooks: Arc::default(),
        }
    }

    /// A seed of a timer with no lists, from a stored run in `from`.
    fn seeded_as(id: &str, phase: TimerPhase, from: TimerPhase) -> Transition {
        Transition::Seeded {
            id: id.into(),
            phase,
            from,
            hooks: Arc::default(),
        }
    }

    const MUTE: Action = Action::SetMuted { muted: true };
    const UNMUTE: Action = Action::SetMuted { muted: false };

    /// A countdown's two lists.
    fn lists(on_start: Vec<Action>, on_fire: Vec<Action>) -> Arc<Hooks> {
        Arc::new(Hooks { on_start, on_fire })
    }

    /// Add break (BRB)'s lists: mute at the start, unmute at zero (D26).
    fn brb() -> Arc<Hooks> {
        lists(vec![MUTE], vec![UNMUTE])
    }

    /// `entry` with these lists.
    fn with_hooks(entry: WireEntry, hooks: &Arc<Hooks>) -> WireEntry {
        edit(entry, |d| d.hooks = hooks.clone())
    }

    /// A start, carrying `hooks`.
    fn started_with(id: &str, fires: u32, hooks: &Arc<Hooks>) -> Transition {
        Transition::Started {
            id: id.into(),
            fires,
            hooks: hooks.clone(),
        }
    }

    /// A start of a timer with no lists.
    fn started(id: &str, fires: u32) -> Transition {
        started_with(id, fires, &Arc::default())
    }

    /// A cancel, carrying `hooks`, of a run that started with them.
    fn cancelled_with(id: &str, why: CancelWhy, hooks: &Arc<Hooks>) -> Transition {
        Transition::Cancelled {
            id: id.into(),
            why,
            hooks: hooks.clone(),
            started: hooks.clone(),
        }
    }

    /// A fire, carrying `hooks`.
    fn fired_with(
        id: &str,
        fires: u32,
        late_ms: u64,
        finished: bool,
        hooks: &Arc<Hooks>,
    ) -> Transition {
        Transition::Fired {
            id: id.into(),
            fires,
            late_ms,
            finished,
            hooks: hooks.clone(),
        }
    }

    /// v1.60.0: Arm on `id` at `now`, which records its start — a
    /// countdown's `Started`, a count-up's nothing (D32) — and nothing else.
    /// Drained here, so what a test takes next is what it is about.
    fn arm(core: &mut TimerCore, id: &str, now: Duration) {
        op(core, id, TimerOp::Arm, now);
        let t = core.timers.iter().find(|t| t.id == id).unwrap();
        let want = match t.kind {
            TimerKind::Countdown => vec![started(id, t.fires)],
            TimerKind::Countup => vec![],
        };
        assert_eq!(core.take_transitions(), want, "{id} armed at {now:?}");
    }

    /// D4: armed for 5 s and ticked every 10 ms to 20 s, a countdown fires
    /// once — done, `fires` 1, one `Fired` — and never again.
    #[test]
    fn fires_exactly_once() {
        let mut core = core_with(vec![countdown("timer-1", 5_000)]);
        arm(&mut core, "timer-1", Duration::ZERO);
        let mut now = Duration::ZERO;
        while now < secs(20) {
            now += ms(10);
            tick(&mut core, now);
        }
        let s = state(&mut core, now, "timer-1");
        assert_eq!(
            (s.phase, s.ms, s.text.as_str(), s.fires),
            (Done, 0, "00:00", 1)
        );
        assert_eq!(core.take_transitions(), vec![fired("timer-1", 1, 0)]);
    }

    /// D4, D22: a nanosecond before its deadline a countdown runs with 1 ms
    /// left and shows `00:01`; at the deadline it is done and shows `00:00`.
    #[test]
    fn never_fires_early() {
        let mut core = core_with(vec![countdown("timer-1", 5_000)]);
        arm(&mut core, "timer-1", secs(1));
        let almost = secs(6).saturating_sub(Duration::from_nanos(1));
        tick(&mut core, almost);
        assert_eq!(
            shown(&mut core, almost, "timer-1"),
            (Running, 1, "00:01".into())
        );
        assert_eq!(core.take_transitions(), vec![]);
        tick(&mut core, secs(6));
        assert_eq!(
            shown(&mut core, secs(6), "timer-1"),
            (Done, 0, "00:00".into())
        );
        assert_eq!(core.take_transitions(), vec![fired("timer-1", 1, 0)]);
    }

    /// D4: each Arm allows one fire; a countdown started again from `done`
    /// fires again, once. v1.60.0: each Arm is a start.
    #[test]
    fn a_rearmed_countdown_fires_again_once() {
        let mut core = core_with(vec![countdown("timer-1", 5_000)]);
        arm(&mut core, "timer-1", Duration::ZERO);
        tick(&mut core, secs(5));
        op(&mut core, "timer-1", TimerOp::Arm, secs(6));
        assert_eq!(
            shown(&mut core, secs(6), "timer-1"),
            (Running, 5_000, "00:05".into())
        );
        for t in 7..=20 {
            tick(&mut core, secs(t));
        }
        let s = state(&mut core, secs(20), "timer-1");
        assert_eq!((s.phase, s.fires), (Done, 2));
        assert_eq!(
            core.take_transitions(),
            vec![
                fired("timer-1", 1, 0),
                started("timer-1", 1),
                fired("timer-1", 2, 0)
            ]
        );
    }

    /// D34: a fire an op's own tick takes — a Pause 1.5 s past the deadline
    /// with no tick between — is recorded where it happens and reported once,
    /// at the next drain. The Pause then finds the timer done and changes
    /// nothing, so nothing ever pauses at 0.
    #[test]
    fn a_fire_taken_by_an_op_is_reported_once() {
        let mut core = core_with(vec![countdown("timer-1", 5_000)]);
        arm(&mut core, "timer-1", Duration::ZERO);
        op(&mut core, "timer-1", TimerOp::Pause, ms(6_500));
        let s = state(&mut core, ms(6_500), "timer-1");
        assert_eq!(
            (s.phase, s.ms, s.text.as_str(), s.fires),
            (Done, 0, "00:00", 1)
        );
        assert_eq!(core.take_transitions(), vec![fired("timer-1", 1, 1_500)]);
        tick(&mut core, secs(7));
        op(&mut core, "timer-1", TimerOp::Pause, secs(8));
        assert_eq!(core.take_transitions(), vec![]);
    }

    /// D6: a Toggle that finds its countdown due fires it first, then starts
    /// it again from its length: `fires` + 1, running. v1.60.0 (§7.1):
    /// `[Fired, Started]` in that order — P2, then P1 — so a break started
    /// again at its deadline unmutes, then mutes again.
    #[test]
    fn a_toggle_at_the_deadline_fires_then_starts() {
        let brb = brb();
        let mut core = core_with(vec![with_hooks(countdown("timer-1", 5_000), &brb)]);
        op(&mut core, "timer-1", TimerOp::Arm, Duration::ZERO);
        assert_eq!(
            core.take_transitions(),
            vec![started_with("timer-1", 0, &brb)]
        );
        op(&mut core, "timer-1", TimerOp::Toggle, ms(5_001));
        let s = state(&mut core, ms(5_001), "timer-1");
        assert_eq!(
            (s.phase, s.ms, s.text.as_str(), s.fires),
            (Running, 5_000, "00:05", 1)
        );
        let recorded = core.take_transitions();
        assert_eq!(
            recorded,
            vec![
                fired_with("timer-1", 1, 1, false, &brb),
                started_with("timer-1", 1, &brb)
            ]
        );
        let planned: Vec<(Why, Vec<Action>)> = recorded
            .iter()
            .map(|t| plan(t).map(|p| (p.why, p.backend)).unwrap())
            .collect();
        assert_eq!(
            planned,
            [(Why::Zero, vec![UNMUTE]), (Why::Armed, vec![MUTE])]
        );
    }

    /// End now's fire, of a timer with no lists.
    fn finished(id: &str, fires: u32) -> Transition {
        Transition::Fired {
            id: id.into(),
            fires,
            late_ms: 0,
            finished: true,
            hooks: Arc::default(),
        }
    }

    /// v1.60.0 (D25): End now fires a running countdown once, through the
    /// one `fire()` — done, `fires` 1, one `Fired {late 0, finished}` — and
    /// a second Finish, or a tick past the old deadline, adds nothing.
    #[test]
    fn finish_fires_once() {
        let mut core = core_with(vec![countdown("timer-1", 5_000)]);
        arm(&mut core, "timer-1", Duration::ZERO);
        op(&mut core, "timer-1", TimerOp::Finish, secs(2));
        assert!(core.take_dirty());
        let s = state(&mut core, secs(2), "timer-1");
        assert_eq!(
            (s.phase, s.ms, s.text.as_str(), s.fires, s.anchor_ms),
            (Done, 0, "00:00", 1, None)
        );
        assert_eq!(core.take_transitions(), vec![finished("timer-1", 1)]);
        op(&mut core, "timer-1", TimerOp::Finish, secs(3));
        assert!(!core.take_dirty());
        tick(&mut core, secs(6));
        assert_eq!(state(&mut core, secs(6), "timer-1").fires, 1);
        assert_eq!(core.take_transitions(), vec![]);
    }

    /// v1.60.0 (D25): a paused countdown's deadline is stale — armed at 0
    /// for 5:00 and paused at 100 s, it would read 300 s late at 500 s — so
    /// End now is never late.
    #[test]
    fn finish_a_paused_countdown_is_not_late() {
        let mut core = core_with(vec![countdown("timer-1", 300_000)]);
        arm(&mut core, "timer-1", Duration::ZERO);
        op(&mut core, "timer-1", TimerOp::Pause, secs(100));
        op(&mut core, "timer-1", TimerOp::Finish, secs(500));
        let s = state(&mut core, secs(500), "timer-1");
        assert_eq!((s.phase, s.ms, s.fires), (Done, 0, 1));
        assert_eq!(core.take_transitions(), vec![finished("timer-1", 1)]);
    }

    /// v1.60.0 (D25): End now on a countdown already past a deadline no
    /// reader saw finds the fire taken by its own tick — the deadline's,
    /// with its real lateness, not `finished` — and adds nothing. So it
    /// plans as that fire: 0.5 s late is P2 (`zero`, all of At zero); 40 s
    /// late is P4 (`late`, its restoring actions only: the clip is skipped).
    #[test]
    fn finish_after_an_unticked_deadline_is_a_zero_fire() {
        let tile = Action::Tile {
            tile_id: "9f2c4e1a7b3d5f60".into(),
        };
        let hooks = lists(vec![MUTE], vec![UNMUTE, tile.clone()]);
        for (at, late_ms, why, page) in [
            (ms(5_500), 500, Why::Zero, vec![tile]),
            (secs(45), 40_000, Why::Late, vec![]),
        ] {
            let mut core = core_with(vec![with_hooks(countdown("timer-1", 5_000), &hooks)]);
            op(&mut core, "timer-1", TimerOp::Arm, Duration::ZERO);
            core.take_transitions();
            op(&mut core, "timer-1", TimerOp::Finish, at);
            let s = state(&mut core, at, "timer-1");
            assert_eq!((s.phase, s.fires), (Done, 1), "{at:?}");
            let recorded = core.take_transitions();
            assert_eq!(
                recorded,
                vec![fired_with("timer-1", 1, late_ms, false, &hooks)],
                "{at:?}"
            );
            let p = plan(&recorded[0]).unwrap();
            assert_eq!(
                (p.why, p.late_ms, p.backend, p.page),
                (why, late_ms, vec![UNMUTE], page),
                "{at:?}"
            );
        }
    }

    /// v1.60.0 (D25, D32): End now on an idle or ended countdown, or on a
    /// count-up in any phase, changes nothing and records nothing.
    #[test]
    fn finish_leaves_idle_ended_and_countups() {
        use TimerKind::{Countdown, Countup};
        for (kind, phase) in [
            (Countdown, Idle),
            (Countdown, Done),
            (Countdown, Expired),
            (Countup, Idle),
            (Countup, Running),
            (Countup, Paused),
        ] {
            let (mut core, at) = in_phase(kind, phase);
            let before = state(&mut core, at, "timer-1");
            core.take_dirty();
            op(&mut core, "timer-1", TimerOp::Finish, at);
            assert!(!core.take_dirty(), "{kind:?} {phase:?}");
            assert_eq!(
                state(&mut core, at, "timer-1"),
                before,
                "{kind:?} {phase:?}"
            );
            assert_eq!(core.take_transitions(), vec![], "{kind:?} {phase:?}");
        }
    }

    /// D5 (a): a countdown whose deadline fell inside a sleep the clock
    /// counted is `expired` at the first reading after it, never fired. The
    /// core notes which clock decided, for the thread to log.
    #[test]
    fn a_countdown_that_ran_out_asleep_expires_unfired() {
        let mut core = core_with(vec![countdown("timer-1", 60_000)]);
        arm(&mut core, "timer-1", Duration::ZERO);
        tick(&mut core, secs(30));
        // Down after the reading at 30 s; back ten minutes later.
        let back = secs(30 + 600);
        core.tick(back, wall(back), Some(secs(30)));
        let s = state(&mut core, back, "timer-1");
        assert_eq!(
            (s.phase, s.ms, s.text.as_str(), s.fires, s.anchor_ms),
            (Expired, 0, "00:00", 0, None)
        );
        tick(&mut core, back + secs(1));
        assert_eq!(core.take_transitions(), vec![expired("timer-1")]);
        assert_eq!(
            core.take_notes(),
            vec![Note::ClockSlept {
                slept_ms: 600_000,
                running: 1,
                expired: 1,
                paused: 0
            }]
        );
    }

    /// D5 (a): a countdown due after the resume keeps running, and the sleep
    /// counts: 120 s, 10.1 s awake and 60 s asleep leave 49.9 s.
    #[test]
    fn a_countdown_counts_the_sleep() {
        let mut core = core_with(vec![countdown("timer-1", 120_000)]);
        arm(&mut core, "timer-1", Duration::ZERO);
        tick(&mut core, secs(10));
        let back = ms(70_100);
        core.tick(back, wall(back), Some(secs(10)));
        assert_eq!(
            shown(&mut core, back, "timer-1"),
            (Running, 49_900, "00:50".into())
        );
        assert_eq!(core.take_transitions(), vec![]);
    }

    /// D5 (a), Q2: a count-up pauses at the last reading before the sleep,
    /// and stays there.
    #[test]
    fn a_countup_pauses_where_the_pc_slept() {
        let mut core = core_with(vec![countup("timer-2")]);
        arm(&mut core, "timer-2", Duration::ZERO);
        tick(&mut core, ms(10_250));
        let back = ms(10_250) + secs(3_600);
        core.tick(back, wall(back), Some(ms(10_250)));
        assert_eq!(
            shown(&mut core, back, "timer-2"),
            (Paused, 10_250, "00:10".into())
        );
        let later = back + secs(5);
        tick(&mut core, later);
        assert_eq!(
            shown(&mut core, later, "timer-2"),
            (Paused, 10_250, "00:10".into())
        );
        assert_eq!(
            core.take_transitions(),
            vec![Transition::SleepPaused {
                id: "timer-2".into()
            }]
        );
    }

    /// D5, Q9: held up for 10 s with the PC awake (a debugger, a starved
    /// CPU), a countdown due 5 s ago fires late, its lateness reported.
    #[test]
    fn a_late_countdown_fires_late_when_awake() {
        let mut core = core_with(vec![countdown("timer-1", 5_000)]);
        arm(&mut core, "timer-1", Duration::ZERO);
        tick(&mut core, secs(10));
        let s = state(&mut core, secs(10), "timer-1");
        assert_eq!((s.phase, s.fires), (Done, 1));
        assert_eq!(core.take_transitions(), vec![fired("timer-1", 1, 5_000)]);
    }

    /// D5, WF5: Windows' notification can come more than 5 s after the jump
    /// the sleep check already reported, as a second signal with `before`
    /// one reading old. After an (a): a count-up running again, and a
    /// countdown due between the last two readings. The count-up runs on and
    /// the countdown fires; nothing expires or pauses.
    #[test]
    fn a_late_second_signal_changes_nothing() {
        let mut core = core_with(vec![countup("timer-1"), countdown("timer-2", 3_050)]);
        arm(&mut core, "timer-1", Duration::ZERO);
        tick(&mut core, secs(1));
        let back = secs(1 + 3_600);
        core.tick(back, wall(back), Some(secs(1)));
        assert_eq!(state(&mut core, back, "timer-1").phase, Paused);
        core.take_transitions();
        op(&mut core, "timer-1", TimerOp::Resume, back + secs(3));
        arm(&mut core, "timer-2", back + secs(3));
        let mut now = back + secs(3);
        while now < back + secs(6) {
            now += ms(100);
            tick(&mut core, now);
        }
        let late = back + ms(6_100);
        core.tick(late, wall(late), Some(now));
        assert_eq!(
            shown(&mut core, late, "timer-1"),
            (Running, 4_100, "00:04".into())
        );
        let s = state(&mut core, late, "timer-2");
        assert_eq!((s.phase, s.fires), (Done, 1));
        assert_eq!(core.take_transitions(), vec![fired("timer-2", 1, 50)]);
    }

    /// D5, WF5: the same late second signal, met by readers held up 600 ms
    /// (past (a)'s 500 ms bar; a busy PC just after its resume). The PC
    /// never slept in that gap, so the count-up runs on and the countdown
    /// due in it fires late; nothing expires, pauses or is noted. A real
    /// second sleep longer than 5 s within the minute is still an (a).
    #[test]
    fn a_late_second_signal_after_a_stall_changes_nothing() {
        let mut core = core_with(vec![countup("timer-1"), countdown("timer-2", 3_050)]);
        arm(&mut core, "timer-1", Duration::ZERO);
        tick(&mut core, secs(1));
        let back = secs(1 + 3_600);
        core.tick(back, wall(back), Some(secs(1)));
        assert_eq!(state(&mut core, back, "timer-1").phase, Paused);
        core.take_transitions();
        assert_eq!(
            core.take_notes(),
            vec![Note::ClockSlept {
                slept_ms: 3_600_000,
                running: 1,
                expired: 0,
                paused: 1
            }]
        );
        op(&mut core, "timer-1", TimerOp::Resume, back + secs(3));
        arm(&mut core, "timer-2", back + secs(3));
        let mut now = back + secs(3);
        while now < back + secs(6) {
            now += ms(100);
            tick(&mut core, now);
        }
        let late = now + ms(600);
        core.tick(late, wall(late), Some(now));
        assert_eq!(
            shown(&mut core, late, "timer-1"),
            (Running, 4_600, "00:04".into())
        );
        let s = state(&mut core, late, "timer-2");
        assert_eq!((s.phase, s.fires), (Done, 1));
        assert_eq!(core.take_transitions(), vec![fired("timer-2", 1, 550)]);
        assert_eq!(core.take_notes(), vec![]);

        // 20 s after the first, the PC sleeps 30 s more.
        let down = back + secs(20);
        tick(&mut core, down);
        let again = down + secs(30);
        core.tick(again, wall(again), Some(down));
        assert_eq!(
            shown(&mut core, again, "timer-1"),
            (Paused, 18_000, "00:18".into())
        );
        assert_eq!(
            core.take_transitions(),
            vec![Transition::SleepPaused {
                id: "timer-1".into()
            }]
        );
    }

    /// The thread drains the notes; until it does — one that never started
    /// — they stop at [`MAX_NOTES`].
    #[test]
    fn notes_wait_for_the_thread_up_to_a_cap() {
        let mut core = TimerCore::new();
        for k in 0..100 {
            let now = ms(k);
            core.sync(
                vec![countdown("timer-1", 60_000), countdown("timer-1", 60_000)],
                now,
                wall(now),
                None,
            );
        }
        let notes = core.take_notes();
        assert_eq!(notes.len(), MAX_NOTES);
        assert!(notes.iter().all(|n| *n
            == Note::DuplicateId {
                id: "timer-1".into()
            }));
        assert_eq!(core.take_notes(), vec![]);
    }

    /// K13: the thread drains the transitions; until it does — one that
    /// never started — they stop at [`MAX_TRANSITIONS`], the oldest kept,
    /// and the timers run on regardless. 300 starts and End nows offer 600.
    #[test]
    fn transitions_are_capped() {
        assert_eq!(MAX_TRANSITIONS, 256);
        let mut core = core_with(vec![countdown("timer-1", 60_000)]);
        for k in 0..300 {
            op(&mut core, "timer-1", TimerOp::Arm, secs(2 * k));
            op(&mut core, "timer-1", TimerOp::Finish, secs(2 * k + 1));
        }
        assert_eq!(state(&mut core, secs(600), "timer-1").fires, 300);
        let kept = core.take_transitions();
        assert_eq!(kept.len(), MAX_TRANSITIONS);
        assert_eq!(kept[..2], [started("timer-1", 0), finished("timer-1", 1)]);
        assert_eq!(kept[MAX_TRANSITIONS - 1], finished("timer-1", 128));
        // Drained, they are recorded again.
        arm(&mut core, "timer-1", secs(600));
    }

    /// D5 (b), K2: the clock stood still through a 10-minute sleep while the
    /// wall moved on, and the signal came three readings late with a 100 ms
    /// gap. Each running timer's wall anchor decides: the countdown due
    /// inside the sleep expires, the one due after it gets the wall's left,
    /// the count-up pauses at its reading before the signal.
    #[test]
    fn a_clock_that_missed_the_sleep_is_caught_by_the_wall() {
        let mut core = core_with(vec![
            countdown("timer-1", 300_000),
            countdown("timer-2", 900_000),
            countup("timer-3"),
        ]);
        for id in ["timer-1", "timer-2", "timer-3"] {
            arm(&mut core, id, Duration::ZERO);
        }
        let mut now = Duration::ZERO;
        while now < secs(10) {
            now += ms(100);
            tick(&mut core, now);
        }
        let woke = |now: Duration| wall(now) + 600_000;
        for _ in 0..3 {
            now += ms(100);
            core.tick(now, woke(now), None);
        }
        let before = now;
        now += ms(100);
        core.tick(now, woke(now), Some(before));
        let s1 = state(&mut core, now, "timer-1");
        assert_eq!(
            (s1.phase, s1.fires, s1.text.as_str()),
            (Expired, 0, "00:00")
        );
        // The wall's left: 15:00 − 10:10.4.
        let s2 = state(&mut core, now, "timer-2");
        assert_eq!(
            (s2.phase, s2.ms, s2.text.as_str(), s2.anchor_ms),
            (Running, 289_600, "04:50", Some(WALL0 + 900_000))
        );
        assert_eq!(
            shown(&mut core, now, "timer-3"),
            (Paused, 10_300, "00:10".into())
        );
        assert_eq!(
            core.take_transitions(),
            vec![
                expired("timer-1"),
                Transition::SleepPaused {
                    id: "timer-3".into()
                }
            ]
        );
        assert_eq!(
            core.take_notes(),
            vec![Note::WallChecked {
                running: 3,
                corrected: 1,
                expired: 1,
                paused: 1
            }]
        );
        // From there it runs on the clock, its anchor where it was.
        let later = now + secs(10);
        core.tick(later, woke(later), None);
        let s2 = state(&mut core, later, "timer-2");
        assert_eq!((s2.ms, s2.anchor_ms), (279_600, Some(WALL0 + 900_000)));
    }

    /// D5: the same sleep with the clock counting it is (a)'s, at the first
    /// reading. Windows then resyncs the wall by +3 s, and its notification
    /// lands 6 s after the jump: skipped by the 60 s rule, so the countdown
    /// keeps the clock's time and isn't shortened by the resync.
    #[test]
    fn instant_counted_the_sleep_is_left_to_a() {
        let mut core = core_with(vec![countdown("timer-1", 900_000)]);
        arm(&mut core, "timer-1", Duration::ZERO);
        let mut now = Duration::ZERO;
        while now < secs(10) {
            now += ms(100);
            tick(&mut core, now);
        }
        let resumed = now + secs(600) + ms(100);
        core.tick(resumed, wall(resumed), Some(now));
        assert_eq!(state(&mut core, resumed, "timer-1").ms, 289_900);
        let resynced = |now: Duration| {
            if now >= resumed + secs(3) {
                wall(now) + 3_000
            } else {
                wall(now)
            }
        };
        now = resumed;
        while now < resumed + ms(5_900) {
            now += ms(100);
            core.tick(now, resynced(now), None);
        }
        let signal = now + ms(100);
        core.tick(signal, resynced(signal), Some(now));
        let s = state(&mut core, signal, "timer-1");
        assert_eq!(
            (s.phase, s.ms, s.anchor_ms),
            (Running, 283_900, Some(WALL0 + 900_000))
        );
        assert_eq!(core.take_transitions(), vec![]);
    }

    /// D5 (b): a signal with no jump and no earlier (a), the wall agreeing
    /// with the clock: nothing pauses or expires, and a countdown due in the
    /// 100 ms since the last reading fires.
    #[test]
    fn a_signal_without_a_jump_changes_nothing_when_the_clocks_agree() {
        let mut core = core_with(vec![
            countdown("timer-1", 5_000),
            countup("timer-2"),
            countdown("timer-3", 60_000),
        ]);
        for id in ["timer-1", "timer-2", "timer-3"] {
            arm(&mut core, id, Duration::ZERO);
        }
        let mut now = Duration::ZERO;
        while now < ms(4_950) {
            now += ms(50);
            tick(&mut core, now);
        }
        let signal = now + ms(100);
        core.tick(signal, wall(signal), Some(now));
        let s1 = state(&mut core, signal, "timer-1");
        assert_eq!((s1.phase, s1.fires), (Done, 1));
        assert_eq!(
            shown(&mut core, signal, "timer-2"),
            (Running, 5_050, "00:05".into())
        );
        assert_eq!(
            shown(&mut core, signal, "timer-3"),
            (Running, 54_950, "00:55".into())
        );
        assert_eq!(core.take_transitions(), vec![fired("timer-1", 1, 50)]);
    }

    /// D5: a count-up started by the last reader before the sleep (its
    /// `since` is that reading) pauses there: at 0.
    #[test]
    fn an_op_just_before_a_sleep_pauses_the_countup_at_its_reading() {
        let mut core = core_with(vec![countup("timer-2")]);
        tick(&mut core, secs(5));
        op(&mut core, "timer-2", TimerOp::Arm, secs(7));
        let back = secs(7 + 3_600);
        core.tick(back, wall(back), Some(secs(7)));
        assert_eq!(
            shown(&mut core, back, "timer-2"),
            (Paused, 0, "00:00".into())
        );
        assert_eq!(
            core.take_transitions(),
            vec![Transition::SleepPaused {
                id: "timer-2".into()
            }]
        );
    }

    /// D5, D16: a command that is the first reader after a resume sees the
    /// sleep itself: a countdown due inside it expires (not fired), and the
    /// Pause then changes nothing.
    #[test]
    fn a_command_after_a_resume_sees_the_sleep() {
        let mut core = core_with(vec![countdown("timer-1", 60_000)]);
        arm(&mut core, "timer-1", Duration::ZERO);
        tick(&mut core, secs(30));
        let back = secs(30 + 600);
        core.apply("timer-1", TimerOp::Pause, back, wall(back), Some(secs(30)))
            .unwrap();
        let s = state(&mut core, back, "timer-1");
        assert_eq!(
            (s.phase, s.ms, s.text.as_str(), s.fires),
            (Expired, 0, "00:00", 0)
        );
        assert_eq!(core.take_transitions(), vec![expired("timer-1")]);
    }

    /// D5: an Arm that is the first reader after a resume handles the sleep
    /// first, so the count-up it starts runs on from the Arm.
    #[test]
    fn a_countup_started_after_the_resume_runs_on() {
        let mut core = core_with(vec![countup("timer-2")]);
        tick(&mut core, secs(5));
        let back = secs(5 + 3_600);
        core.apply("timer-2", TimerOp::Arm, back, wall(back), Some(secs(5)))
            .unwrap();
        let later = back + secs(2);
        tick(&mut core, later);
        assert_eq!(
            shown(&mut core, later, "timer-2"),
            (Running, 2_000, "00:02".into())
        );
        assert_eq!(core.take_transitions(), vec![]);
    }

    /// The sleep check reports a sleep once, to whichever reader comes
    /// first — often the thread, with no timers yet. An empty core spends it
    /// then: a count-up added after the resume runs on, and the 60 s rule
    /// still covers a second signal 20 s after the jump, landing 5 s into a
    /// +3 s wall resync (before the anchors follow it).
    #[test]
    fn a_sleep_seen_with_no_timers_still_counts() {
        let mut core = TimerCore::new();
        tick(&mut core, secs(10));
        let back = secs(10 + 600);
        core.tick(back, wall(back), Some(secs(10)));
        let added = back + secs(1);
        core.sync(
            vec![countup("timer-1"), countdown("timer-2", 900_000)],
            added,
            wall(added),
            None,
        );
        arm(&mut core, "timer-1", added);
        arm(&mut core, "timer-2", added);
        let resynced = |now: Duration| {
            if now >= back + secs(15) {
                wall(now) + 3_000
            } else {
                wall(now)
            }
        };
        let mut now = added;
        while now < back + ms(19_900) {
            now += ms(100);
            core.tick(now, resynced(now), None);
        }
        let signal = now + ms(100);
        core.tick(signal, resynced(signal), Some(now));
        assert_eq!(
            shown(&mut core, signal, "timer-1"),
            (Running, 19_000, "00:19".into())
        );
        assert_eq!(state(&mut core, signal, "timer-2").ms, 881_000);
        assert_eq!(core.take_transitions(), vec![]);
    }

    /// D3, §7.2: a +5 s wall step while a countdown runs leaves its anchor
    /// for 9.9 s, then moves it by exactly the step, once, and marks dirty
    /// (the page stores the new anchor). The live time never moves. A step
    /// that goes back within 10 s moves nothing.
    #[test]
    fn a_wall_step_moves_the_anchor_after_10_s() {
        let armed = Some(WALL0 + 3_600_050);
        let stepped = |now: Duration| {
            if now >= secs(5) {
                wall(now) + 5_000
            } else {
                wall(now)
            }
        };
        let mut core = core_with(vec![countdown("timer-1", 3_600_000)]);
        // Off the 100 ms grid, so no second boundary falls on a tick.
        op(&mut core, "timer-1", TimerOp::Arm, ms(50));
        let mut now = Duration::ZERO;
        while now < ms(14_900) {
            now += ms(100);
            core.tick(now, stepped(now), None);
            assert_eq!(state(&mut core, now, "timer-1").anchor_ms, armed, "{now:?}");
        }
        assert_eq!(core.take_notes(), vec![]);
        core.take_dirty();
        now += ms(100);
        core.tick(now, stepped(now), None);
        assert!(core.take_dirty());
        assert_eq!(
            state(&mut core, now, "timer-1").anchor_ms,
            Some(WALL0 + 3_605_050)
        );
        assert_eq!(
            core.take_notes(),
            vec![Note::AnchorFollowed {
                id: "timer-1".into(),
                step_ms: 5_000
            }]
        );
        while now < secs(30) {
            now += ms(100);
            core.tick(now, stepped(now), None);
        }
        let s = state(&mut core, now, "timer-1");
        assert_eq!((s.anchor_ms, s.ms), (Some(WALL0 + 3_605_050), 3_570_050));

        let undone = |now: Duration| {
            if (secs(5)..secs(12)).contains(&now) {
                wall(now) + 5_000
            } else {
                wall(now)
            }
        };
        let mut core = core_with(vec![countdown("timer-1", 3_600_000)]);
        op(&mut core, "timer-1", TimerOp::Arm, ms(50));
        let mut now = Duration::ZERO;
        while now < secs(30) {
            now += ms(100);
            core.tick(now, undone(now), None);
            assert_eq!(state(&mut core, now, "timer-1").anchor_ms, armed, "{now:?}");
        }
        assert_eq!(core.take_notes(), vec![]);
    }

    /// D3, D10: an anchor is set at the transition that sets the run, not
    /// recomputed after: with the wall drifting 1 ms ahead of the clock at
    /// each tick (50 ms in all, under the step bar) every snapshot has the
    /// same anchors. An Adjust and a Resume set new ones.
    #[test]
    fn the_anchor_is_set_at_transitions() {
        let mut core = core_with(vec![countdown("timer-1", 300_000), countup("timer-2")]);
        op(&mut core, "timer-1", TimerOp::Arm, Duration::ZERO);
        op(&mut core, "timer-2", TimerOp::Arm, Duration::ZERO);
        let anchors = |core: &mut TimerCore, now: Duration| -> Vec<Option<i64>> {
            core.snapshot(now)
                .timers
                .iter()
                .map(|t| t.anchor_ms)
                .collect()
        };
        let armed = anchors(&mut core, Duration::ZERO);
        assert_eq!(armed, vec![Some(WALL0 + 300_000), Some(WALL0)]);
        let mut now = Duration::ZERO;
        for drift in 1..=50 {
            now += ms(100);
            core.tick(now, wall(now) + drift, None);
            assert_eq!(anchors(&mut core, now), armed, "tick {drift}");
        }
        let at_adjust = wall(now) + 50;
        core.apply("timer-1", TimerOp::Adjust(30_000), now, at_adjust, None)
            .unwrap();
        core.apply("timer-2", TimerOp::Pause, now, at_adjust, None)
            .unwrap();
        let resumed = now + secs(1);
        let at_resume = wall(resumed) + 50;
        core.apply("timer-2", TimerOp::Resume, resumed, at_resume, None)
            .unwrap();
        // 4:55 left + 30 s; 5 s counted.
        assert_eq!(
            anchors(&mut core, resumed),
            vec![Some(at_adjust + 325_000), Some(at_resume - 5_000)]
        );
    }

    /// A core with `timer-1` — a 5:00 countdown or a count-up — brought to
    /// `phase`, and the time to act at. Running: armed at 1 s, acted on at
    /// 11.5 s (4:49.5 left, 10.5 s counted). Paused: paused there, acted on
    /// at 20 s. Done: fired at 301 s. Expired: ran out in a sleep from 200 s
    /// to 400 s.
    fn in_phase(kind: TimerKind, phase: TimerPhase) -> (TimerCore, Duration) {
        let entry = match kind {
            TimerKind::Countdown => countdown("timer-1", 300_000),
            TimerKind::Countup => countup("timer-1"),
        };
        in_phase_of(entry, phase)
    }

    /// [`in_phase`] for `entry` (a 5:00 countdown or a count-up, `timer-1`),
    /// its lists and all.
    fn in_phase_of(entry: WireEntry, phase: TimerPhase) -> (TimerCore, Duration) {
        let mut core = core_with(vec![entry]);
        if phase != Idle {
            op(&mut core, "timer-1", TimerOp::Arm, secs(1));
        }
        let at = match phase {
            Idle => secs(1),
            Running => ms(11_500),
            Paused => {
                op(&mut core, "timer-1", TimerOp::Pause, ms(11_500));
                secs(20)
            }
            Done => {
                tick(&mut core, secs(301));
                secs(310)
            }
            Expired => {
                tick(&mut core, secs(200));
                core.tick(secs(400), wall(secs(400)), Some(secs(200)));
                secs(410)
            }
        };
        assert_eq!(state(&mut core, at, "timer-1").phase, phase);
        core.take_dirty();
        core.take_transitions();
        (core, at)
    }

    /// What an op records (§7.8): nothing, a start, a Reset's cancel, End
    /// now's fire.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum Rec {
        Nothing,
        Started,
        Cancelled,
        Ended,
    }

    /// §7.1: every op on every phase of both kinds, by `ms`, text and total.
    /// A row with no result changes nothing: the answer shows the timer as
    /// it was, and nothing is marked dirty. v1.60.0 (§7.8): what every op,
    /// Toggle and Finish included, records on every phase — a countdown
    /// armed from idle, done or ran out starts (a Resume doesn't), a Reset
    /// cancels only a running or paused one, End now fires one, a count-up
    /// records nothing — and on a countdown past a deadline no reader saw,
    /// the op's tick takes the fire first.
    #[test]
    #[allow(clippy::too_many_lines)] // §7.1 in one table, a row per cell
    fn op_table() {
        use TimerKind::{Countdown, Countup};
        use TimerOp::{Adjust, Arm, Pause, Reset, Resume};
        type Want = Option<(TimerPhase, u64, &'static str, u64)>;
        let up = Adjust(30_000);
        let rows: [(TimerKind, TimerPhase, TimerOp, Want); 40] = [
            (
                Countdown,
                Idle,
                Arm,
                Some((Running, 300_000, "05:00", 300_000)),
            ),
            (Countdown, Idle, Pause, None),
            (Countdown, Idle, Resume, None),
            (Countdown, Idle, Reset, None),
            (Countdown, Idle, up, None),
            (Countdown, Running, Arm, None),
            (
                Countdown,
                Running,
                Pause,
                Some((Paused, 289_500, "04:50", 300_000)),
            ),
            (Countdown, Running, Resume, None),
            (
                Countdown,
                Running,
                Reset,
                Some((Idle, 300_000, "05:00", 300_000)),
            ),
            (
                Countdown,
                Running,
                up,
                Some((Running, 319_500, "05:20", 330_000)),
            ),
            (Countdown, Paused, Arm, None),
            (Countdown, Paused, Pause, None),
            (
                Countdown,
                Paused,
                Resume,
                Some((Running, 289_500, "04:50", 300_000)),
            ),
            (
                Countdown,
                Paused,
                Reset,
                Some((Idle, 300_000, "05:00", 300_000)),
            ),
            (
                Countdown,
                Paused,
                up,
                Some((Paused, 319_500, "05:20", 330_000)),
            ),
            (
                Countdown,
                Done,
                Arm,
                Some((Running, 300_000, "05:00", 300_000)),
            ),
            (Countdown, Done, Pause, None),
            (Countdown, Done, Resume, None),
            (
                Countdown,
                Done,
                Reset,
                Some((Idle, 300_000, "05:00", 300_000)),
            ),
            (Countdown, Done, up, None),
            (
                Countdown,
                Expired,
                Arm,
                Some((Running, 300_000, "05:00", 300_000)),
            ),
            (Countdown, Expired, Pause, None),
            (Countdown, Expired, Resume, None),
            (
                Countdown,
                Expired,
                Reset,
                Some((Idle, 300_000, "05:00", 300_000)),
            ),
            (Countdown, Expired, up, None),
            (Countup, Idle, Arm, Some((Running, 0, "00:00", 0))),
            (Countup, Idle, Pause, None),
            (Countup, Idle, Resume, None),
            (Countup, Idle, Reset, None),
            (Countup, Idle, up, None),
            (Countup, Running, Arm, None),
            (Countup, Running, Pause, Some((Paused, 10_500, "00:10", 0))),
            (Countup, Running, Resume, None),
            (Countup, Running, Reset, Some((Idle, 0, "00:00", 0))),
            (Countup, Running, up, Some((Running, 40_500, "00:40", 0))),
            (Countup, Paused, Arm, None),
            (Countup, Paused, Pause, None),
            (Countup, Paused, Resume, Some((Running, 10_500, "00:10", 0))),
            (Countup, Paused, Reset, Some((Idle, 0, "00:00", 0))),
            (Countup, Paused, up, Some((Paused, 40_500, "00:40", 0))),
        ];
        for (kind, phase, top, want) in rows {
            let what = format!("{kind:?} {phase:?} {top:?}");
            let (mut core, at) = in_phase(kind, phase);
            let before = state(&mut core, at, "timer-1");
            core.take_dirty();
            op(&mut core, "timer-1", top, at);
            let after = state(&mut core, at, "timer-1");
            let Some((phase, value, text, total)) = want else {
                assert_eq!(after, before, "{what}");
                assert!(!core.is_dirty(), "{what}");
                continue;
            };
            assert_eq!(
                (after.phase, after.ms, after.text.as_str(), after.total_ms),
                (phase, value, text, total),
                "{what}"
            );
            assert_eq!(after.anchor_ms.is_some(), phase == Running, "{what}");
            assert_eq!(after.fires, before.fires, "{what}");
            assert!(core.is_dirty(), "{what}");
        }

        let (fin, tog) = (TimerOp::Finish, TimerOp::Toggle);
        let records: [(TimerKind, TimerPhase, TimerOp, Rec); 56] = [
            (Countdown, Idle, Arm, Rec::Started),
            (Countdown, Idle, Pause, Rec::Nothing),
            (Countdown, Idle, Resume, Rec::Nothing),
            (Countdown, Idle, Reset, Rec::Nothing),
            (Countdown, Idle, up, Rec::Nothing),
            (Countdown, Idle, tog, Rec::Started),
            (Countdown, Idle, fin, Rec::Nothing),
            (Countdown, Running, Arm, Rec::Nothing),
            (Countdown, Running, Pause, Rec::Nothing),
            (Countdown, Running, Resume, Rec::Nothing),
            (Countdown, Running, Reset, Rec::Cancelled),
            (Countdown, Running, up, Rec::Nothing),
            (Countdown, Running, tog, Rec::Nothing),
            (Countdown, Running, fin, Rec::Ended),
            (Countdown, Paused, Arm, Rec::Nothing),
            (Countdown, Paused, Pause, Rec::Nothing),
            (Countdown, Paused, Resume, Rec::Nothing),
            (Countdown, Paused, Reset, Rec::Cancelled),
            (Countdown, Paused, up, Rec::Nothing),
            (Countdown, Paused, tog, Rec::Nothing),
            (Countdown, Paused, fin, Rec::Ended),
            (Countdown, Done, Arm, Rec::Started),
            (Countdown, Done, Pause, Rec::Nothing),
            (Countdown, Done, Resume, Rec::Nothing),
            (Countdown, Done, Reset, Rec::Nothing),
            (Countdown, Done, up, Rec::Nothing),
            (Countdown, Done, tog, Rec::Started),
            (Countdown, Done, fin, Rec::Nothing),
            (Countdown, Expired, Arm, Rec::Started),
            (Countdown, Expired, Pause, Rec::Nothing),
            (Countdown, Expired, Resume, Rec::Nothing),
            (Countdown, Expired, Reset, Rec::Nothing),
            (Countdown, Expired, up, Rec::Nothing),
            (Countdown, Expired, tog, Rec::Started),
            (Countdown, Expired, fin, Rec::Nothing),
            (Countup, Idle, Arm, Rec::Nothing),
            (Countup, Idle, Pause, Rec::Nothing),
            (Countup, Idle, Resume, Rec::Nothing),
            (Countup, Idle, Reset, Rec::Nothing),
            (Countup, Idle, up, Rec::Nothing),
            (Countup, Idle, tog, Rec::Nothing),
            (Countup, Idle, fin, Rec::Nothing),
            (Countup, Running, Arm, Rec::Nothing),
            (Countup, Running, Pause, Rec::Nothing),
            (Countup, Running, Resume, Rec::Nothing),
            (Countup, Running, Reset, Rec::Nothing),
            (Countup, Running, up, Rec::Nothing),
            (Countup, Running, tog, Rec::Nothing),
            (Countup, Running, fin, Rec::Nothing),
            (Countup, Paused, Arm, Rec::Nothing),
            (Countup, Paused, Pause, Rec::Nothing),
            (Countup, Paused, Resume, Rec::Nothing),
            (Countup, Paused, Reset, Rec::Nothing),
            (Countup, Paused, up, Rec::Nothing),
            (Countup, Paused, tog, Rec::Nothing),
            (Countup, Paused, fin, Rec::Nothing),
        ];
        for (kind, phase, top, rec) in records {
            let what = format!("{kind:?} {phase:?} {top:?}");
            let (mut core, at) = in_phase(kind, phase);
            let fires = state(&mut core, at, "timer-1").fires;
            op(&mut core, "timer-1", top, at);
            let want = match rec {
                Rec::Nothing => vec![],
                Rec::Started => vec![started("timer-1", fires)],
                Rec::Cancelled => {
                    vec![cancelled_with("timer-1", CancelWhy::Reset, &Arc::default())]
                }
                Rec::Ended => vec![finished("timer-1", fires + 1)],
            };
            assert_eq!(core.take_transitions(), want, "{what}");
            assert_eq!(
                state(&mut core, at, "timer-1").fires,
                fires + u32::from(rec == Rec::Ended),
                "{what}"
            );
        }

        // Running, 1 s past a deadline no reader saw: the op's tick fires
        // it (`fires` 1), then the op meets a done countdown.
        for (top, then, phase) in [
            (Arm, Rec::Started, Running),
            (Reset, Rec::Nothing, Idle),
            (tog, Rec::Started, Running),
            (fin, Rec::Nothing, Done),
        ] {
            let mut core = core_with(vec![countdown("timer-1", 300_000)]);
            arm(&mut core, "timer-1", secs(1));
            op(&mut core, "timer-1", top, secs(302));
            let mut want = vec![fired("timer-1", 1, 1_000)];
            if then == Rec::Started {
                want.push(started("timer-1", 1));
            }
            assert_eq!(core.take_transitions(), want, "{top:?}");
            let s = state(&mut core, secs(302), "timer-1");
            assert_eq!((s.phase, s.fires), (phase, 1), "{top:?}");
        }
    }

    /// §7.1's Toggle column (the hotkey's): idle, done or expired → Arm;
    /// running → Pause; paused → Resume.
    #[test]
    fn toggle_table() {
        use TimerKind::{Countdown, Countup};
        for (kind, phase, want) in [
            (Countdown, Idle, (Running, 300_000, "05:00")),
            (Countdown, Running, (Paused, 289_500, "04:50")),
            (Countdown, Paused, (Running, 289_500, "04:50")),
            (Countdown, Done, (Running, 300_000, "05:00")),
            (Countdown, Expired, (Running, 300_000, "05:00")),
            (Countup, Idle, (Running, 0, "00:00")),
            (Countup, Running, (Paused, 10_500, "00:10")),
            (Countup, Paused, (Running, 10_500, "00:10")),
        ] {
            let (mut core, at) = in_phase(kind, phase);
            op(&mut core, "timer-1", TimerOp::Toggle, at);
            assert_eq!(
                shown(&mut core, at, "timer-1"),
                (want.0, want.1, want.2.to_string()),
                "{kind:?} {phase:?}"
            );
        }
    }

    /// §7.3, D7: ±δ on running and paused timers, with text and total; −30 s
    /// stops at 1 s and keeps a left already under it; idle, done and
    /// expired don't move.
    #[test]
    #[allow(clippy::too_many_lines)] // §7.3's rows, then the phases that don't move
    fn adjust_table() {
        use TimerOp::{Arm, Pause};
        // (timer, ops before as (op, at ms), δ, at ms) → (phase, ms, text,
        // total, changed).
        type Row = (WireEntry, &'static [(TimerOp, u64)], i64, u64);
        type Want = (TimerPhase, u64, &'static str, u64, bool);
        let rows: [(Row, Want); 10] = [
            // 5:00 +30 → 5:30.
            (
                (countdown("timer-1", 300_000), &[(Arm, 0)], 30_000, 0),
                (Running, 330_000, "05:30", 330_000, true),
            ),
            // 23:59:50 +30 → 24:00:00.
            (
                (
                    countdown("timer-1", 86_400_000),
                    &[(Arm, 0), (Pause, 10_000)],
                    30_000,
                    20_000,
                ),
                (Paused, 86_400_000, "24:00:00", 86_410_000, true),
            ),
            // A 24 h countdown run 1 h, +30: left 23:00:30, total 24:00:30.
            (
                (
                    countdown("timer-1", 86_400_000),
                    &[(Arm, 0)],
                    30_000,
                    3_600_000,
                ),
                (Running, 82_830_000, "23:00:30", 86_430_000, true),
            ),
            // 0:12 −30 → 0:01.
            (
                (countdown("timer-1", 60_000), &[(Arm, 0)], -30_000, 48_000),
                (Running, 1_000, "00:01", 49_000, true),
            ),
            // 0:00.4 −30 → 0:00.4: it stays, paused or running.
            (
                (
                    countdown("timer-1", 1_000),
                    &[(Arm, 0), (Pause, 600)],
                    -30_000,
                    700,
                ),
                (Paused, 400, "00:01", 1_000, false),
            ),
            (
                (countdown("timer-1", 1_000), &[(Arm, 0)], -30_000, 600),
                (Running, 400, "00:01", 1_000, false),
            ),
            // 5:00 −30 → 4:30.
            (
                (
                    countdown("timer-1", 300_000),
                    &[(Arm, 0), (Pause, 0)],
                    -30_000,
                    0,
                ),
                (Paused, 270_000, "04:30", 270_000, true),
            ),
            // A count-up: 0:10 −30 → 0:00; 1:00 +30 → 1:30.
            (
                (countup("timer-1"), &[(Arm, 0)], -30_000, 10_000),
                (Running, 0, "00:00", 0, true),
            ),
            (
                (
                    countup("timer-1"),
                    &[(Arm, 0), (Pause, 60_000)],
                    30_000,
                    61_000,
                ),
                (Paused, 90_000, "01:30", 0, true),
            ),
            // Idle: nothing.
            (
                (countdown("timer-1", 300_000), &[], -30_000, 0),
                (Idle, 300_000, "05:00", 300_000, false),
            ),
        ];
        for ((entry, steps, delta, at), (phase, want_ms, text, total, changed)) in rows {
            let what = format!("{steps:?} {delta} at {at}");
            let mut core = core_with(vec![entry]);
            for &(step, step_at) in steps {
                op(&mut core, "timer-1", step, ms(step_at));
            }
            let at = ms(at);
            tick(&mut core, at);
            let before = state(&mut core, at, "timer-1");
            core.take_dirty();
            op(&mut core, "timer-1", TimerOp::Adjust(delta), at);
            let after = state(&mut core, at, "timer-1");
            assert_eq!(
                (after.phase, after.ms, after.text.as_str(), after.total_ms),
                (phase, want_ms, text, total),
                "{what}"
            );
            assert_eq!(core.is_dirty(), changed, "{what}");
            if !changed {
                assert_eq!(after, before, "{what}");
            } else if phase == Running {
                // Running on from the new value, with a new anchor.
                let anchor = match after.kind {
                    TimerKind::Countdown => wall(at) + i64::try_from(want_ms).unwrap(),
                    TimerKind::Countup => wall(at) - i64::try_from(want_ms).unwrap(),
                };
                assert_eq!(after.anchor_ms, Some(anchor), "{what}");
            }
        }
        // A sub-second left kept running still ends on its old deadline.
        let mut core = core_with(vec![countdown("timer-1", 1_000)]);
        arm(&mut core, "timer-1", Duration::ZERO);
        op(&mut core, "timer-1", TimerOp::Adjust(-30_000), ms(600));
        tick(&mut core, ms(1_000));
        assert_eq!(core.take_transitions(), vec![fired("timer-1", 1, 0)]);
        // Done and expired don't move either.
        for phase in [Done, Expired] {
            let (mut core, at) = in_phase(TimerKind::Countdown, phase);
            let before = state(&mut core, at, "timer-1");
            op(&mut core, "timer-1", TimerOp::Adjust(-30_000), at);
            assert_eq!(state(&mut core, at, "timer-1"), before, "{phase:?}");
        }
    }

    /// §5.2: every transition recomputes its timer's text and marks dirty —
    /// a Reset, an Adjust on a paused timer, a fire, a seed — so a stopped
    /// timer's snapshot never shows a stale text.
    #[test]
    fn texts_follow_every_transition() {
        let check = |core: &mut TimerCore, now: Duration, id: &str, text: &str| {
            assert!(core.take_dirty(), "{id} {text}");
            let s = state(core, now, id);
            assert_eq!(
                (s.text.as_str(), timer_text(s.kind, s.ms).as_str()),
                (text, text)
            );
        };
        let mut core = core_with(vec![
            countdown("timer-1", 300_000),
            countdown("timer-2", 300_000),
        ]);
        op(&mut core, "timer-1", TimerOp::Arm, Duration::ZERO);
        tick(&mut core, secs(11));
        core.take_dirty();
        op(&mut core, "timer-1", TimerOp::Reset, secs(11));
        check(&mut core, secs(11), "timer-1", "05:00");

        op(&mut core, "timer-2", TimerOp::Arm, secs(11));
        op(&mut core, "timer-2", TimerOp::Pause, secs(20));
        assert_eq!(state(&mut core, secs(20), "timer-2").text, "04:51");
        core.take_dirty();
        op(&mut core, "timer-2", TimerOp::Adjust(30_000), secs(25));
        check(&mut core, secs(25), "timer-2", "05:21");

        op(&mut core, "timer-1", TimerOp::Arm, secs(30));
        tick(&mut core, ms(329_900));
        core.take_dirty();
        tick(&mut core, secs(330));
        check(&mut core, secs(330), "timer-1", "00:00");

        let resumed = edit(countdown("timer-3", 120_000), |d| {
            d.resume = Some(stored(Paused, Some(61_000), Some(120_000), None));
        });
        let now = secs(331);
        core.sync(
            vec![
                countdown("timer-1", 300_000),
                countdown("timer-2", 300_000),
                resumed,
            ],
            now,
            wall(now),
            None,
        );
        check(&mut core, now, "timer-3", "01:01");
    }

    /// A fresh core's seed of `run` for `id`, a `kind` timer of `length_ms`,
    /// at `wall_ms`: the state it comes back in, and what was recorded.
    fn seeded(
        id: &str,
        kind: TimerKind,
        length_ms: u64,
        run: TimerRun,
        wall_ms: i64,
    ) -> (TimerState, Vec<Transition>) {
        let mut core = TimerCore::new();
        let now = secs(3);
        let def = TimerDef {
            id: id.into(),
            name: "Seeded".into(),
            kind,
            duration_ms: length_ms,
            resume: Some(run),
            hooks: Arc::default(),
        };
        core.sync(vec![WireEntry::Def(def)], now, wall_ms, None);
        (state(&mut core, now, id), core.take_transitions())
    }

    /// §7.8, D11, K3: every fixture `runSample` seeds its `seededPhase` and
    /// `seededMs` at its `nowWallMs` (the page's `runOf` test reads the same
    /// rows); then the rules beyond the fixture, and the runs that can't
    /// seed one and start idle. A pause 0.3 ms before the deadline stores
    /// `ms` 1 and comes back paused at 1 ms.
    #[test]
    #[allow(clippy::too_many_lines)] // the fixture's rows, then §7.8's own
    fn seed_table() {
        use TimerKind::{Countdown, Countup};
        for row in fixture()["runSamples"].as_array().unwrap() {
            let name = row_name(row);
            let s: TimerState = serde_json::from_value(row["state"].clone()).unwrap();
            let run = run_from_wire(&row["run"]).unwrap_or_else(|| panic!("{name}"));
            let length = match s.kind {
                Countdown => s.total_ms.min(MAX_LENGTH_MS),
                Countup => DEFAULT_LENGTH_MS,
            };
            let want: TimerPhase = serde_json::from_value(row["seededPhase"].clone()).unwrap();
            let want_ms = row["seededMs"].as_u64().unwrap();
            let wall_ms = row["nowWallMs"].as_i64().unwrap();
            let from = run.phase;
            let (got, recorded) = seeded(&s.id, s.kind, length, run, wall_ms);
            assert_eq!((got.phase, got.ms), (want, want_ms), "{name}");
            assert_eq!(got.text, timer_text(s.kind, want_ms), "{name}");
            assert_eq!((got.fires, got.total_ms), (0, s.total_ms), "{name}");
            assert_eq!(recorded, vec![seeded_as(&s.id, want, from)], "{name}");
        }

        let at = WALL0;
        for (what, kind, run, want) in [
            (
                "a clock set back: no more than the total",
                Countdown,
                stored(Running, None, Some(300_000), Some(at + 400_000)),
                Some((Running, 300_000, 300_000)),
            ),
            (
                "… and no more than 24 h",
                Countdown,
                stored(Running, None, Some(90_000_000), Some(at + 90_000_000)),
                Some((Running, MAX_LENGTH_MS, 90_000_000)),
            ),
            (
                "a clock set forward: expired",
                Countdown,
                stored(Running, None, Some(300_000), Some(at - 1)),
                Some((Expired, 0, 300_000)),
            ),
            (
                "paused, a total under its left",
                Countdown,
                stored(Paused, Some(60_000), Some(30_000), None),
                Some((Paused, 60_000, 60_000)),
            ),
            (
                "paused, a total past 24 h",
                Countdown,
                stored(Paused, Some(82_830_000), Some(86_430_000), None),
                Some((Paused, 82_830_000, 86_430_000)),
            ),
            (
                "done with no total: its length's",
                Countdown,
                stored(Done, None, None, None),
                Some((Done, 0, 120_000)),
            ),
            (
                "expired with no total: its length's",
                Countdown,
                stored(Expired, None, None, None),
                Some((Expired, 0, 120_000)),
            ),
            (
                "a count-up closed past 10 000 h",
                Countup,
                stored(Running, None, None, Some(at - 36_000_000_001)),
                Some((Running, MAX_SEED_ELAPSED_MS, 0)),
            ),
            (
                "a count-up started after now",
                Countup,
                stored(Running, None, None, Some(at + 5_000)),
                Some((Running, 0, 0)),
            ),
            (
                "a count-up paused past 10 000 h",
                Countup,
                stored(Paused, Some(u64::MAX), None, None),
                Some((Paused, MAX_SEED_ELAPSED_MS, 0)),
            ),
            // Runs that can't be one: idle, the definition kept (D35).
            (
                "running, no anchor",
                Countdown,
                stored(Running, None, Some(300_000), None),
                None,
            ),
            (
                "running, no total",
                Countdown,
                stored(Running, None, None, Some(at + 60_000)),
                None,
            ),
            (
                "running, a total of 0",
                Countdown,
                stored(Running, None, Some(0), Some(at + 60_000)),
                None,
            ),
            (
                "paused, no total",
                Countdown,
                stored(Paused, Some(60_000), None, None),
                None,
            ),
            (
                "paused, no ms",
                Countdown,
                stored(Paused, None, Some(300_000), None),
                None,
            ),
            (
                "paused at 0",
                Countdown,
                stored(Paused, Some(0), Some(300_000), None),
                None,
            ),
            (
                "paused past 24 h",
                Countdown,
                stored(Paused, Some(86_400_001), Some(90_000_000), None),
                None,
            ),
            (
                "a count-up paused, no ms",
                Countup,
                stored(Paused, None, None, None),
                None,
            ),
            (
                "a count-up done",
                Countup,
                stored(Done, None, Some(1), None),
                None,
            ),
            (
                "a count-up expired",
                Countup,
                stored(Expired, None, None, None),
                None,
            ),
        ] {
            let from = run.phase;
            let (got, recorded) = seeded("timer-1", kind, 120_000, run, at);
            let Some((phase, value, total)) = want else {
                let idle = if kind == Countdown { 120_000 } else { 0 };
                assert_eq!(
                    (got.phase, got.ms, got.total_ms),
                    (Idle, idle, idle),
                    "{what}"
                );
                assert_eq!(recorded, vec![], "{what}");
                continue;
            };
            assert_eq!(
                (got.phase, got.ms, got.total_ms),
                (phase, value, total),
                "{what}"
            );
            assert_eq!(got.text, timer_text(kind, value), "{what}");
            assert_eq!(recorded, vec![seeded_as("timer-1", phase, from)], "{what}");
        }

        // The pause at deadline − 0.3 ms, stored as the page's `runOf`
        // stores it, and seeded after a restart.
        let mut core = core_with(vec![countdown("timer-1", 300_000)]);
        op(&mut core, "timer-1", TimerOp::Arm, Duration::ZERO);
        let paused_at = secs(300).saturating_sub(Duration::from_micros(300));
        op(&mut core, "timer-1", TimerOp::Pause, paused_at);
        let s = state(&mut core, paused_at, "timer-1");
        assert_eq!((s.phase, s.ms, s.text.as_str()), (Paused, 1, "00:01"));
        let run = stored(Paused, Some(s.ms), Some(s.total_ms), None);
        let (back, _) = seeded("timer-1", Countdown, 300_000, run, WALL0 + 3_600_000);
        assert_eq!(
            (back.phase, back.ms, back.text.as_str()),
            (Paused, 1, "00:01")
        );
    }

    /// D10: a stored run seeds an id only at the core's first acceptance of
    /// it in this process. A reload's sync with another run changes nothing;
    /// a timer deleted and added again is a new definition: idle, `fires`
    /// from 0, its run ignored.
    #[test]
    fn rust_wins_while_it_knows_a_timer() {
        let running = |anchor: i64| stored(Running, None, Some(300_000), Some(anchor));
        let brb = |anchor: i64| {
            edit(countdown("timer-1", 300_000), |d| {
                d.resume = Some(running(anchor));
            })
        };
        let mut core = TimerCore::new();
        core.sync(
            vec![brb(WALL0 + 120_000), countup("timer-2")],
            Duration::ZERO,
            WALL0,
            None,
        );
        assert_eq!(
            shown(&mut core, Duration::ZERO, "timer-1"),
            (Running, 120_000, "02:00".into())
        );
        let now = secs(1);
        core.sync(
            vec![brb(WALL0 + 60_000), countup("timer-2")],
            now,
            wall(now),
            None,
        );
        assert_eq!(state(&mut core, now, "timer-1").ms, 119_000);
        tick(&mut core, secs(120));
        assert_eq!(state(&mut core, secs(120), "timer-1").fires, 1);

        let now = secs(121);
        let left = core.sync(vec![countup("timer-2")], now, wall(now), None);
        assert_eq!(left, vec!["timer-1".to_string()]);
        let now = secs(122);
        core.sync(
            vec![countup("timer-2"), brb(wall(now) + 30_000)],
            now,
            wall(now),
            None,
        );
        let s = state(&mut core, now, "timer-1");
        assert_eq!((s.phase, s.ms, s.fires), (Idle, 300_000, 0));
        assert_eq!(
            core.take_transitions(),
            vec![
                seeded_as("timer-1", Running, Running),
                fired("timer-1", 1, 0)
            ]
        );
    }

    /// D6, D11, §7.1 P6-P8, from the fixture's `runSamples`: each stored run
    /// seeds a `Seeded` that says the phase it was stored in (`from`), with
    /// the lists of the sync that seeded it — the current ones (Q12). Its
    /// plan follows: a break that was running or paused mutes again
    /// (backend only); one that ran out while Divora was closed unmutes; a
    /// stored ended run runs nothing. A count-up's lists were dropped by the
    /// reader (D32), so it plans nothing.
    #[test]
    fn seeded_says_where_it_came_from() {
        use TimerPhase::{Done, Expired, Paused, Running};
        for row in fixture()["runSamples"].as_array().unwrap() {
            let name = row_name(row);
            let s: TimerState = serde_json::from_value(row["state"].clone()).unwrap();
            let entry = json!({
                "id": s.id,
                "name": "BRB",
                "kind": s.kind,
                "durationMs": s.total_ms.clamp(MIN_LENGTH_MS, MAX_LENGTH_MS),
                "resume": row["run"],
                "onStart": [{"kind": "setMuted", "muted": true}],
                "onFire": [{"kind": "setMuted", "muted": false}],
            });
            let mut core = TimerCore::new();
            let now = secs(3);
            let wall_ms = row["nowWallMs"].as_i64().unwrap();
            core.sync(vec![entry_from_wire(&entry).unwrap()], now, wall_ms, None);
            let recorded = core.take_transitions();
            let [Transition::Seeded {
                phase, from, hooks, ..
            }] = recorded.as_slice()
            else {
                panic!("{name}: {recorded:?}");
            };
            let stored: TimerPhase = serde_json::from_value(row["run"]["phase"].clone()).unwrap();
            let seeded: TimerPhase = serde_json::from_value(row["seededPhase"].clone()).unwrap();
            assert_eq!((*phase, *from), (seeded, stored), "{name}");
            let countdown = s.kind == TimerKind::Countdown;
            let want_hooks = if countdown { brb() } else { Arc::default() };
            assert_eq!(*hooks, want_hooks, "{name}");
            let want = match (countdown, stored, seeded) {
                (true, Running | Paused, Running | Paused) => Some((Why::Restart, MUTE)),
                (true, Running, Expired) => Some((Why::RanOut, UNMUTE)),
                (true, Done | Expired, _) | (false, ..) => None,
                other => panic!("{name}: {other:?}"),
            };
            let planned = plan(&recorded[0]).map(|p| {
                assert!(p.page.is_empty(), "{name}");
                (p.why, p.backend)
            });
            assert_eq!(planned, want.map(|(why, a)| (why, vec![a])), "{name}");
        }
    }

    /// D15, D36: what a sync changes — an add, a rename, a new length while
    /// idle (its text follows) and while running (its run stands until it
    /// starts again), a new kind (idle), a removal (returned), a duplicate
    /// (the first stands) — and what it keeps without running: an unknown
    /// kind and every known one past the 16th, whose files stay. A kept
    /// 17th was never accepted, so once it runs its stored run seeds it.
    #[test]
    #[allow(clippy::too_many_lines)] // one story, a step per kind of change
    fn sync_table() {
        let named = |entry: WireEntry, name: &str| edit(entry, |d| d.name = name.into());
        let ids = |core: &mut TimerCore, now: Duration| -> Vec<String> {
            core.snapshot(now)
                .timers
                .into_iter()
                .map(|t| t.id)
                .collect()
        };
        let mut core = TimerCore::new();
        let sync = |core: &mut TimerCore, entries: Vec<WireEntry>, now: Duration| {
            let left = core.sync(entries, now, wall(now), None);
            assert!(core.take_dirty() && core.synced());
            left
        };
        assert!(!core.synced());
        let left = sync(&mut core, vec![countdown("timer-1", 300_000)], secs(0));
        assert_eq!(left, Vec::<String>::new());
        assert_eq!(
            shown(&mut core, secs(0), "timer-1"),
            (Idle, 300_000, "05:00".into())
        );

        sync(
            &mut core,
            vec![
                named(countdown("timer-1", 300_000), "BRB"),
                countup("timer-2"),
            ],
            secs(1),
        );
        let titles: Vec<String> = core
            .snapshot(secs(1))
            .timers
            .into_iter()
            .map(|t| t.name)
            .collect();
        assert_eq!(titles, vec!["BRB", "timer-2"]);

        sync(
            &mut core,
            vec![
                named(countdown("timer-1", 600_000), "BRB"),
                countup("timer-2"),
            ],
            secs(2),
        );
        let s = state(&mut core, secs(2), "timer-1");
        assert_eq!(
            (s.phase, s.ms, s.text.as_str(), s.total_ms),
            (Idle, 600_000, "10:00", 600_000)
        );

        op(&mut core, "timer-1", TimerOp::Arm, secs(3));
        sync(
            &mut core,
            vec![
                named(countdown("timer-1", 120_000), "BRB"),
                countup("timer-2"),
            ],
            secs(4),
        );
        let s = state(&mut core, secs(4), "timer-1");
        assert_eq!((s.phase, s.ms, s.total_ms), (Running, 599_000, 600_000));
        op(&mut core, "timer-1", TimerOp::Reset, secs(5));
        assert_eq!(
            shown(&mut core, secs(5), "timer-1"),
            (Idle, 120_000, "02:00".into())
        );
        op(&mut core, "timer-1", TimerOp::Arm, secs(5));

        sync(
            &mut core,
            vec![countup("timer-1"), countup("timer-2")],
            secs(6),
        );
        let s = state(&mut core, secs(6), "timer-1");
        assert_eq!(
            (
                s.kind,
                s.phase,
                s.ms,
                s.text.as_str(),
                s.total_ms,
                s.anchor_ms
            ),
            (TimerKind::Countup, Idle, 0, "00:00", 0, None)
        );

        let left = sync(&mut core, vec![countup("timer-2")], secs(7));
        assert_eq!(left, vec!["timer-1".to_string()]);
        assert_eq!(ids(&mut core, secs(7)), vec!["timer-2"]);

        sync(
            &mut core,
            vec![
                named(countup("timer-2"), "A"),
                named(countup("timer-2"), "B"),
                countdown("timer-3", 60_000),
            ],
            secs(8),
        );
        assert_eq!(ids(&mut core, secs(8)), vec!["timer-2", "timer-3"]);
        assert_eq!(state(&mut core, secs(8), "timer-2").name, "A");
        assert_eq!(
            core.take_notes(),
            vec![Note::DuplicateId {
                id: "timer-2".into()
            }]
        );

        // Kept, not run.
        let later = stored(Running, None, Some(60_000), Some(WALL0 + 30_000));
        let seventeenth = || {
            edit(countdown("timer-17", 60_000), |d| {
                d.resume = Some(later.clone());
            })
        };
        let mut entries = vec![WireEntry::Kept("brb".into())];
        entries.extend((1..=16).map(|n| countdown(&format!("timer-{n}"), 60_000)));
        entries.push(seventeenth());
        let mut core = TimerCore::new();
        core.sync(entries, Duration::ZERO, WALL0, None);
        let run = ids(&mut core, Duration::ZERO);
        assert_eq!(run.len(), MAX_TIMERS);
        assert!(!run.contains(&"timer-17".to_string()));
        let update = core.text_update();
        assert_eq!(update.texts.len(), MAX_TIMERS);
        assert_eq!(update.keep.len(), MAX_TIMERS + 2);
        assert!(["brb", "timer-17"]
            .iter()
            .all(|id| update.keep.contains(&(*id).to_string())));
        assert_eq!(core.take_transitions(), vec![]);

        let mut entries: Vec<WireEntry> = (2..=16)
            .map(|n| countdown(&format!("timer-{n}"), 60_000))
            .collect();
        entries.push(seventeenth());
        let now = secs(1);
        let left = core.sync(entries, now, wall(now), None);
        assert_eq!(left, vec!["timer-1".to_string()]);
        assert_eq!(
            shown(&mut core, now, "timer-17"),
            (Running, 29_000, "00:29".into())
        );
        assert_eq!(core.text_update().keep.len(), MAX_TIMERS);
    }

    /// D6, D10: a Reset cancels a running or paused countdown —
    /// `Cancelled {Reset}`, carrying its lists — and an idle or ended one
    /// records nothing. A sync without an active countdown's id (deleted,
    /// or no longer among the first [`MAX_TIMERS`]) records `Cancelled
    /// {Removed}` with the lists it had; one that changes its kind records
    /// `Cancelled {Reset}` with its lists from before that sync, not the
    /// count-up's it becomes (D32). Ended and idle countdowns, and
    /// count-ups, record nothing either way.
    #[test]
    #[allow(clippy::too_many_lines)] // Reset, delete, the 17th, a kind change
    fn reset_and_delete_cancel_only_active_countdowns() {
        let brb = brb();
        let entry = || with_hooks(countdown("timer-1", 300_000), &brb);
        let phases = [
            (Running, true),
            (Paused, true),
            (Idle, false),
            (Done, false),
            (Expired, false),
        ];
        let cancelled = |active: bool, why| -> Vec<Transition> {
            if active {
                vec![cancelled_with("timer-1", why, &brb)]
            } else {
                vec![]
            }
        };
        for (phase, active) in phases {
            // Reset.
            let (mut core, at) = in_phase_of(entry(), phase);
            op(&mut core, "timer-1", TimerOp::Reset, at);
            assert_eq!(
                core.take_transitions(),
                cancelled(active, CancelWhy::Reset),
                "Reset {phase:?}"
            );
            assert_eq!(state(&mut core, at, "timer-1").phase, Idle, "{phase:?}");

            // Deleted: a sync without it, beside a timer that stays.
            let (mut core, at) = in_phase_of(entry(), phase);
            let left = core.sync(vec![countup("timer-2")], at, wall(at), None);
            assert_eq!(left, ["timer-1"], "{phase:?}");
            assert_eq!(
                core.take_transitions(),
                cancelled(active, CancelWhy::Removed),
                "deleted {phase:?}"
            );

            // A kind change: idle, a count-up with no lists.
            let (mut core, at) = in_phase_of(entry(), phase);
            core.sync(vec![countup("timer-1")], at, wall(at), None);
            assert_eq!(
                core.take_transitions(),
                cancelled(active, CancelWhy::Reset),
                "kind {phase:?}"
            );
            let s = state(&mut core, at, "timer-1");
            assert_eq!((s.kind, s.phase), (TimerKind::Countup, Idle), "{phase:?}");
            assert_eq!(core.timers[0].hooks, Arc::default(), "{phase:?}");
        }

        // Past the 16th: kept, not run, so a running one is cancelled too.
        let (mut core, at) = in_phase_of(entry(), Running);
        let mut entries: Vec<WireEntry> = (2..=17)
            .map(|n| countdown(&format!("timer-{n}"), 60_000))
            .collect();
        entries.push(entry());
        let left = core.sync(entries, at, wall(at), None);
        assert_eq!(left, ["timer-1"]);
        assert_eq!(core.take_transitions(), cancelled(true, CancelWhy::Removed));

        // A count-up, running or paused, records nothing either way.
        for phase in [Running, Paused] {
            let (mut core, at) = in_phase(TimerKind::Countup, phase);
            op(&mut core, "timer-1", TimerOp::Reset, at);
            let (mut gone, at) = in_phase(TimerKind::Countup, phase);
            gone.sync(vec![], at, wall(at), None);
            let (mut changed, at) = in_phase(TimerKind::Countup, phase);
            changed.sync(vec![countdown("timer-1", 60_000)], at, wall(at), None);
            for c in [&mut core, &mut gone, &mut changed] {
                assert_eq!(c.take_transitions(), vec![], "{phase:?}");
            }
        }
    }

    /// D7, TP4: a sync's lists replace the timer's. Synced with none, then
    /// with a break's, a countdown's Start carries the break's. On a running
    /// countdown, a list change, a rename and a new length each record
    /// nothing and keep the run, and At zero changed mid-run counts for this
    /// run's end (Q12).
    #[test]
    fn a_sync_takes_new_hooks() {
        let plain = || countdown("timer-1", 60_000);
        let mut core = core_with(vec![plain()]);
        let brb = brb();
        let resync = |core: &mut TimerCore, entry: WireEntry, now: Duration| {
            core.sync(vec![entry], now, wall(now), None);
            assert_eq!(core.take_transitions(), vec![], "{now:?}");
        };
        resync(&mut core, with_hooks(plain(), &brb), secs(1));
        op(&mut core, "timer-1", TimerOp::Arm, secs(2));
        assert_eq!(
            core.take_transitions(),
            vec![started_with("timer-1", 0, &brb)]
        );

        let line = Action::Speak {
            text: "I'm back.".into(),
            voice_id: "af_heart".into(),
        };
        let newer = lists(vec![MUTE], vec![UNMUTE, line]);
        let running = state(&mut core, secs(3), "timer-1");
        resync(&mut core, with_hooks(plain(), &newer), secs(3));
        let renamed = edit(with_hooks(plain(), &newer), |d| {
            d.name = "Break".into();
            d.duration_ms = 120_000;
        });
        resync(&mut core, renamed, secs(3));
        let s = state(&mut core, secs(3), "timer-1");
        assert_eq!(
            (s.phase, s.ms, s.total_ms, s.anchor_ms, s.name.as_str()),
            (
                running.phase,
                running.ms,
                running.total_ms,
                running.anchor_ms,
                "Break"
            )
        );
        tick(&mut core, secs(62));
        assert_eq!(
            core.take_transitions(),
            vec![fired_with("timer-1", 1, 0, false, &newer)]
        );
    }

    /// Q2, Q12 (D7, D10): a Reset, a kind change and a delete undo what this
    /// run's start did, not what At start says by then — an At start edit
    /// counts from the next start. A break whose mute is edited out mid-run
    /// still unmutes when cancelled; an "unmute me in 5 minutes" with a mute
    /// edited in mid-run unmutes nothing. A Start again takes At start as it
    /// is then, and a run picked up at a relaunch started with the lists
    /// its restart re-applied (P7).
    #[test]
    fn a_cancel_undoes_what_its_start_did() {
        let entry = |hooks: &Arc<Hooks>| with_hooks(countdown("timer-1", 300_000), hooks);
        let brb = brb();
        let unmute_later = lists(vec![], vec![UNMUTE]);
        // What the recorded transitions run in the backend, and why.
        let runs = |core: &mut TimerCore| -> Vec<(Why, Vec<Action>)> {
            core.take_transitions()
                .iter()
                .filter_map(plan)
                .map(|p| (p.why, p.backend))
                .collect()
        };
        // (started with, At start edited to, what a cancel undoes)
        let rows = [
            (&brb, &unmute_later, vec![UNMUTE]),
            (&unmute_later, &brb, vec![]),
        ];
        for (start, edited, undo) in &rows {
            for how in ["Reset", "kind change", "deleted"] {
                let mut core = core_with(vec![entry(start)]);
                op(&mut core, "timer-1", TimerOp::Arm, secs(1));
                assert_eq!(
                    runs(&mut core),
                    [(Why::Armed, start.on_start.clone())],
                    "{how}"
                );
                core.sync(vec![entry(edited)], secs(2), wall(secs(2)), None);
                assert_eq!(runs(&mut core), [], "{how}: an edit runs nothing");
                let at = secs(3);
                let why = match how {
                    "Reset" => {
                        op(&mut core, "timer-1", TimerOp::Reset, at);
                        Why::Reset
                    }
                    "kind change" => {
                        core.sync(vec![countup("timer-1")], at, wall(at), None);
                        Why::Reset
                    }
                    _ => {
                        core.sync(vec![], at, wall(at), None);
                        Why::Deleted
                    }
                };
                assert_eq!(runs(&mut core), [(why, undo.clone())], "{how}");
            }
        }

        // Start again: a break run to zero, its mute edited out, started
        // again by a Toggle, the mute edited back in, Reset: this run's
        // start muted nothing.
        let mut core = core_with(vec![entry(&brb)]);
        op(&mut core, "timer-1", TimerOp::Arm, secs(1));
        tick(&mut core, secs(301));
        assert_eq!(
            runs(&mut core),
            [(Why::Armed, vec![MUTE]), (Why::Zero, vec![UNMUTE])]
        );
        core.sync(vec![entry(&unmute_later)], secs(302), wall(secs(302)), None);
        op(&mut core, "timer-1", TimerOp::Toggle, secs(303));
        core.sync(vec![entry(&brb)], secs(304), wall(secs(304)), None);
        op(&mut core, "timer-1", TimerOp::Reset, secs(305));
        assert_eq!(
            runs(&mut core),
            [(Why::Armed, vec![]), (Why::Reset, vec![])],
            "Start again"
        );

        // A relaunch picks up a paused run with the lists it has then and
        // re-applies their mute; an edit after it counts from the next start.
        for (start, edited, undo) in &rows {
            let mut core = TimerCore::new();
            let paused = edit(entry(start), |d| {
                d.resume = Some(stored(Paused, Some(120_000), Some(300_000), None));
            });
            core.sync(vec![paused], secs(1), wall(secs(1)), None);
            let restart: Vec<Action> = start.on_start.clone();
            let restarted = if restart.is_empty() {
                vec![]
            } else {
                vec![(Why::Restart, restart)]
            };
            assert_eq!(runs(&mut core), restarted, "relaunch");
            assert_eq!(state(&mut core, secs(1), "timer-1").phase, Paused);
            core.sync(vec![entry(edited)], secs(2), wall(secs(2)), None);
            op(&mut core, "timer-1", TimerOp::Reset, secs(3));
            assert_eq!(runs(&mut core), [(Why::Reset, undo.clone())], "relaunch");
        }
    }

    /// D17: every snapshot's `seq` is larger than the one before, whoever
    /// builds it: the thread after a change, a command's answer, a status
    /// read that changed nothing.
    #[test]
    fn snapshots_count_up() {
        let mut core = core_with(vec![countdown("timer-1", 10_000)]);
        let mut last = 0;
        let mut next = |snapshot: TimerSnapshot, what: &str| {
            assert!(snapshot.seq > last, "{what}: {} after {last}", snapshot.seq);
            last = snapshot.seq;
        };
        for i in 1..=40 {
            let now = ms(250 * i);
            core.tick(now, wall(now), None);
            if core.take_dirty() {
                next(core.snapshot(now), "the thread's");
            }
            if i % 3 == 0 {
                op(&mut core, "timer-1", TimerOp::Toggle, now);
                next(core.snapshot(now), "an answer");
            }
            if i % 5 == 0 {
                core.tick(now, wall(now), None);
                next(core.snapshot(now), "a status read");
            }
        }
    }

    /// D18: the thread emits on change only. A tick inside one second
    /// changes nothing; one that crosses a boundary does; a snapshot that
    /// crosses one marks dirty too (the thread still emits that second); a
    /// paused timer and an op that changes nothing never do.
    #[test]
    fn dirty_only_on_change() {
        let mut core = core_with(vec![countdown("timer-1", 10_000), countup("timer-2")]);
        op(&mut core, "timer-1", TimerOp::Arm, Duration::ZERO);
        op(&mut core, "timer-2", TimerOp::Arm, Duration::ZERO);
        assert!(core.take_dirty());
        for t in [100, 400, 999] {
            tick(&mut core, ms(t));
            assert!(!core.take_dirty(), "{t}");
        }
        tick(&mut core, secs(1));
        assert!(core.take_dirty());
        tick(&mut core, ms(1_500));
        assert!(!core.take_dirty());
        core.snapshot(secs(2));
        assert!(core.take_dirty());
        core.snapshot(ms(2_100));
        assert!(!core.take_dirty());
        op(&mut core, "timer-1", TimerOp::Pause, ms(2_500));
        op(&mut core, "timer-2", TimerOp::Pause, ms(2_500));
        assert!(core.take_dirty());
        tick(&mut core, secs(10));
        op(&mut core, "timer-1", TimerOp::Pause, secs(11));
        assert!(!core.take_dirty());
    }

    /// §7.5: the wait to the next text change, in ns from the tick's own
    /// reading, uncapped and capped at [`TICK`]; the deadline is a
    /// countdown's last boundary.
    #[test]
    fn next_wake_table() {
        let countdown_left = |left: Duration| {
            let mut core = core_with(vec![countdown("timer-1", 10_000)]);
            op(&mut core, "timer-1", TimerOp::Arm, Duration::ZERO);
            (core, secs(10).saturating_sub(left))
        };
        let countup_at = |elapsed: Duration| {
            let mut core = core_with(vec![countup("timer-2")]);
            op(&mut core, "timer-2", TimerOp::Arm, Duration::ZERO);
            (core, elapsed)
        };
        let micros = Duration::from_micros;
        for (what, (core, now), wake) in [
            ("countdown, 4 500 left", countdown_left(ms(4_500)), ms(500)),
            (
                "countdown, 4 000 left",
                countdown_left(ms(4_000)),
                ms(1_000),
            ),
            ("countdown, 800 left", countdown_left(ms(800)), ms(800)),
            ("countdown, 30 left", countdown_left(ms(30)), ms(30)),
            (
                "countdown, 0.3 left",
                countdown_left(micros(300)),
                micros(300),
            ),
            (
                "countdown, due, unfired",
                countdown_left(Duration::ZERO),
                Duration::ZERO,
            ),
            ("count-up, 2 300 counted", countup_at(ms(2_300)), ms(700)),
            ("count-up, 2 970 counted", countup_at(ms(2_970)), ms(30)),
        ] {
            assert_eq!(core.next_wake(now, secs(5)), wake, "{what}");
            assert_eq!(core.next_wake(now, TICK), wake.min(TICK), "{what}");
        }
        // Nothing running: the cap, so sleep checks keep their cadence.
        let core = core_with(vec![countdown("timer-1", 10_000), countup("timer-2")]);
        assert_eq!(core.next_wake(secs(1), TICK), TICK);
        // Two running: the nearer change.
        let mut core = core_with(vec![countdown("timer-1", 10_000), countup("timer-2")]);
        op(&mut core, "timer-1", TimerOp::Arm, Duration::ZERO);
        op(&mut core, "timer-2", TimerOp::Arm, ms(2_530));
        assert_eq!(core.next_wake(ms(5_500), secs(5)), ms(30));
    }

    /// D33: fields a later release sends (`obsText`, `followObs`, actions of
    /// a kind this build doesn't know) don't stop a definition: it runs.
    /// v1.60.0: its readable actions are read, the rest skipped (D4).
    #[test]
    fn later_fields_are_ignored() {
        let sent = json!({
            "id": "timer-1", "name": "BRB", "kind": "countdown", "durationMs": 300_000,
            "obsText": "BRB text", "followObs": true,
            "onStart": [{"kind": "setMuted", "muted": true}],
            "onFire": [{"kind": "somethingNew", "x": 1}, {"kind": "speak", "text": "back"}],
        });
        let read = entry_from_wire(&sent).unwrap();
        let WireEntry::Def(def) = &read else {
            panic!("a definition");
        };
        // `somethingNew` is skipped, and so is a line with no voice.
        let hooks = lists(vec![MUTE], vec![]);
        assert_eq!(def.hooks, hooks);
        let mut core = TimerCore::new();
        core.sync(vec![read], Duration::ZERO, WALL0, None);
        op(&mut core, "timer-1", TimerOp::Arm, Duration::ZERO);
        assert_eq!(
            shown(&mut core, Duration::ZERO, "timer-1"),
            (Running, 300_000, "05:00".into())
        );
        assert_eq!(
            core.take_transitions(),
            vec![started_with("timer-1", 0, &hooks)]
        );
    }

    /// Every arithmetic step saturates (a panic aborts the app): the largest
    /// deltas through Adjust, lengths of 0 and `u64::MAX`, clocks and walls
    /// at their extremes, and seeds from anchors at `i64::MIN` / `i64::MAX`.
    /// v1.60.0: End now among the ops, every transition planned, lists with
    /// a 10 000-character line and preset id, and `replay_at` at its edges.
    #[test]
    #[allow(clippy::too_many_lines)] // every edge, then v1.60.0's
    fn nothing_panics_at_the_edges() {
        use TimerKind::{Countdown, Countup};
        let long = "a".repeat(10_000);
        let wide = json!([
            {"kind": "speak", "text": long, "voiceId": long},
            {"kind": "preset", "presetId": long},
            {"kind": "setMuted", "muted": true},
            {"kind": "tile", "tileId": long},
        ]);
        let hooks = Arc::new(hooks_from_wire(
            json!({"onStart": wide, "onFire": wide})
                .as_object()
                .unwrap(),
        ));
        // The line and the clip are past their caps; a preset id has none.
        let kept = vec![
            Action::Preset {
                preset_id: long.clone(),
            },
            MUTE,
        ];
        assert_eq!(
            *hooks,
            Hooks {
                on_start: kept.clone(),
                on_fire: kept,
            }
        );
        let edge = |id: &str, kind, duration_ms| {
            WireEntry::Def(TimerDef {
                id: id.into(),
                name: id.into(),
                kind,
                duration_ms,
                resume: None,
                hooks: hooks.clone(),
            })
        };
        let clocks = [
            Duration::ZERO,
            Duration::from_nanos(1),
            secs(u64::MAX / 2),
            Duration::MAX,
        ];
        let walls = [i64::MIN, -1, 0, WALL0, i64::MAX];
        let ops = [
            TimerOp::Arm,
            TimerOp::Adjust(i64::MAX),
            TimerOp::Adjust(i64::MIN),
            TimerOp::Pause,
            TimerOp::Adjust(i64::MIN),
            TimerOp::Adjust(i64::MAX),
            TimerOp::Resume,
            TimerOp::Toggle,
            TimerOp::Finish,
            TimerOp::Reset,
            TimerOp::Toggle,
            TimerOp::Finish,
        ];
        let ids = ["timer-1", "timer-2", "timer-3"];
        let drained = Instant::now();
        for wall_ms in walls {
            let mut core = TimerCore::new();
            core.sync(
                vec![
                    edge("timer-1", Countdown, 0),
                    edge("timer-2", Countdown, u64::MAX),
                    edge("timer-3", Countup, u64::MAX),
                ],
                Duration::ZERO,
                wall_ms,
                None,
            );
            for now in clocks {
                for other_wall in walls {
                    for id in ids {
                        for top in ops {
                            core.apply(id, top, now, other_wall, None).unwrap();
                            core.apply(id, top, now, other_wall, Some(Duration::ZERO))
                                .unwrap();
                            core.apply(id, top, now, other_wall, Some(now)).unwrap();
                        }
                        core.apply(id, TimerOp::Arm, now, other_wall, None).unwrap();
                    }
                    core.tick(now, other_wall, None);
                    core.tick(now, wall_ms, None);
                    core.tick(now, other_wall, Some(Duration::ZERO));
                    core.tick(now, other_wall, Some(now));
                    let _ = core.snapshot(now);
                    let _ = core.next_wake(now, TICK);
                    let _ = core.text_update();
                    for t in core.take_transitions() {
                        if let Some(p) = plan(&t) {
                            let _ = replay_at(drained, p.late_ms);
                            let _ = serde_json::to_value(TimerActionsEvent::of(&p, u64::MAX));
                        }
                    }
                }
            }
            for anchor in [i64::MIN, -1, 0, i64::MAX] {
                for phase in [Running, Paused, Done, Expired] {
                    for kind in [Countdown, Countup] {
                        let run = stored(phase, Some(u64::MAX), Some(u64::MAX), Some(anchor));
                        let (s, recorded) = seeded("timer-1", kind, u64::MAX, run, wall_ms);
                        assert_eq!(s.text, timer_text(kind, s.ms));
                        for t in &recorded {
                            let _ = plan(t);
                        }
                    }
                }
            }
        }
        for late_ms in [0, 1, u64::MAX] {
            for at in [Instant::now(), drained] {
                assert!(replay_at(at, late_ms) <= at);
            }
        }
    }
}
