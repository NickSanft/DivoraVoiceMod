//! v1.59.0: the shell around Hourglass's [`TimerCore`]: its clock and sleep
//! check, the `divora-timers` thread that wakes at each running timer's next
//! second boundary (or every [`TICK`](crate::timers::TICK)) and emits
//! `timer-state`, the `tm:` hotkey step, and the `divora-timer-files` thread
//! that puts each timer's text in `timers/<id>.txt` for OBS.
//!
//! Every reader — the thread, a command, a `tm:` press — takes the core's
//! lock, reads the clock once, asks the one [`SleepCheck`] about that
//! reading, then acts (D5). The lock is a leaf: nothing else is locked while
//! it is held, and nothing slow happens under it. The core never logs: the
//! thread logs its transitions and notes after letting go, then emits and
//! hands the texts to the writer, and only the writer touches the timers'
//! files, so no disk wait can delay a deadline (D13, D21).
//!
//! Neither thread is joined or stopped: Divora has no exit hook, and both
//! run for the process (D14).
//!
//! v1.60.0: a hook run's backend half goes to `divora-timer-actions`, the
//! action worker, by a send that never waits ([`hand_off`]); the worker runs
//! each batch in order against an [`ActionHost`], so nothing that can wait
//! runs on `divora-timers` (D13, D14).

use std::collections::hash_map::Entry;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, SyncSender, TrySendError};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::Value;

use crate::timer_actions::{route, Action, ActionBatch, Hook, Planned, TimerActionsEvent};
use crate::timers::{
    entry_from_wire, hotkey_timer_request, CancelWhy, Note, TextUpdate, TimerCore, TimerOp,
    TimerSnapshot, Transition, LATE_LOG, MAX_SYNC_ENTRIES,
};

/// The clocks every reader takes one reading of.
pub trait TimerClock: Send + Sync {
    /// The monotonic clock: time since the process origin.
    fn now(&self) -> Duration;
    /// The wall clock, in Unix milliseconds.
    fn wall_ms(&self) -> i64;
}

/// The real clocks: [`Instant`] since an origin taken at startup, and
/// [`SystemTime`].
pub struct RealClock {
    origin: Instant,
}

impl RealClock {
    pub fn new() -> Self {
        Self {
            origin: Instant::now(),
        }
    }
}

impl Default for RealClock {
    fn default() -> Self {
        Self::new()
    }
}

impl TimerClock for RealClock {
    fn now(&self) -> Duration {
        self.origin.elapsed()
    }

    fn wall_ms(&self) -> i64 {
        match SystemTime::now().duration_since(UNIX_EPOCH) {
            Ok(after) => i64::try_from(after.as_millis()).unwrap_or(i64::MAX),
            Err(e) => i64::try_from(e.duration().as_millis()).map_or(i64::MIN, |ms| -ms),
        }
    }
}

/// v1.54.0's sleep check, asked with each reading: `Some(before)` when the
/// PC slept since the last reading, `before` being that reading.
pub trait SleepCheck: Send {
    fn jumped(&mut self, clock: Duration) -> Option<Duration>;
}

impl SleepCheck for divora_capture::SleepWatch {
    fn jumped(&mut self, clock: Duration) -> Option<Duration> {
        divora_capture::SleepWatch::jumped(self, clock)
    }
}

/// Where the timer thread sends each snapshot it builds: `timer-state`, to
/// every window; and (v1.60.0) each hook run it plans.
pub trait TimerEvents: Send + 'static {
    /// Never blocks. (Not `state`: `tauri::Manager::state` on the same
    /// `AppHandle` would then be ambiguous wherever this trait is in scope.)
    fn timer_state(&self, s: &TimerSnapshot);
    /// v1.60.0: one hook run, numbered `seq`. Never blocks: hands the
    /// backend half to the action worker, then emits the page's half
    /// (`timer-actions`) — in production, one [`hand_off`].
    fn timer_actions(&self, p: &Planned, seq: u64);
}

/// v1.60.0: batches waiting for the action worker at most. A batch past it
/// is dropped and warned (K3): the worker's own work never waits, so only a
/// stalled one falls this far behind.
pub const ACTION_QUEUE: usize = 32;

/// v1.60.0: what the action worker runs a batch against (D14). Every method
/// returns at once: the mute is an atomic op, Stop all a send that never
/// waits, a replay save goes to the blocking pool and a line to the line
/// thread.
pub trait ActionHost: Send + 'static {
    /// `SetMuted`: a plain set, never a toggle (D24).
    fn set_muted(&self, muted: bool);
    /// `Panic`: stop every clip and line, and tell the page (D23).
    fn stop_all(&self);
    /// `SaveReplay`: save the seconds up to `at` (D22).
    fn save_replay(&self, at: Instant);
    /// `Speak`: put `timer`'s line in the line slot, newest wins (D21).
    fn speak(&self, timer: &str, text: &str, voice_id: &str);
    /// Every end batch, before its actions: `timer`'s start line, waiting or
    /// being prepared, is stale (D21).
    fn end_run(&self, timer: &str);
}

/// v1.60.0: `b` to the action worker without waiting. `false` when it was
/// dropped — the queue full or the worker gone — warned once per 10 s at
/// most.
pub fn hand_batch(tx: &SyncSender<ActionBatch>, b: ActionBatch) -> bool {
    const WARN_EVERY: Duration = Duration::from_secs(10);
    static WARNED: Mutex<Option<Instant>> = Mutex::new(None);
    let (why, id) = match tx.try_send(b) {
        Ok(()) => return true,
        Err(TrySendError::Full(b)) => ("the action worker is behind", b.id),
        Err(TrySendError::Disconnected(b)) => ("the action worker isn't running", b.id),
    };
    let now = Instant::now();
    let warn = {
        let mut warned = lock(&WARNED);
        let due = warned.is_none_or(|at| now.saturating_duration_since(at) >= WARN_EVERY);
        if due {
            *warned = Some(now);
        }
        due
    };
    if warn {
        tracing::warn!(id = %id, "a timer's actions were dropped: {why}");
    }
    false
}

/// v1.60.0 (D13): one hook run's hand-off, in this order: [`route`], the
/// batch to the worker ([`hand_batch`]), then `emit` with the page's event —
/// so the batch never waits for the emit. `drained` is when the timer thread
/// drained the transition. A restart's re-mute hands a batch and emits
/// nothing.
#[doc(hidden)]
pub fn hand_off(
    p: &Planned,
    seq: u64,
    drained: Instant,
    tx: &SyncSender<ActionBatch>,
    emit: impl FnOnce(&TimerActionsEvent),
) {
    let (batch, event) = route(p, seq, drained);
    if let Some(b) = batch {
        hand_batch(tx, b);
    }
    if let Some(ev) = event {
        emit(&ev);
    }
}

/// v1.60.0 (D14): the `divora-timer-actions` loop. Each batch, in the order
/// handed: an end first retires its timer's start line (`end_run`), then
/// each action in list order (`panic` already first), then one log line —
/// after the actions, so a disk stall can't delay a mute. The page's kinds
/// never reach a batch. Returns once every sender is gone.
#[doc(hidden)]
pub fn run_action_worker(rx: Receiver<ActionBatch>, host: &dyn ActionHost) {
    for b in rx {
        if b.hook == Hook::End {
            host.end_run(&b.id);
        }
        for a in &b.actions {
            match a {
                Action::SetMuted { muted } => host.set_muted(*muted),
                Action::Panic => host.stop_all(),
                Action::SaveReplay => host.save_replay(b.at),
                Action::Speak { text, voice_id } => host.speak(&b.id, text, voice_id),
                Action::Preset { .. }
                | Action::RandomPreset
                | Action::RevertPreset
                | Action::Tile { .. } => {}
            }
        }
        let kinds: Vec<&str> = b.actions.iter().map(Action::kind).collect();
        tracing::info!(id = %b.id, hook = ?b.hook, why = ?b.why, ?kinds, "timer actions ran");
    }
}

/// v1.60.0: starts `divora-timer-actions` running [`run_action_worker`]. A
/// failed spawn is logged, and a timer's backend actions then never run
/// (each batch is dropped and warned).
pub fn spawn_action_worker(rx: Receiver<ActionBatch>, host: impl ActionHost) {
    let spawned = std::thread::Builder::new()
        .name("divora-timer-actions".into())
        .spawn(move || run_action_worker(rx, &host));
    if let Err(e) = spawned {
        tracing::error!(error = %e, "couldn't start the timer action worker; timers' actions won't run");
    }
}

/// Why a [`Waiter`] returned.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Woke {
    /// The wait ran out.
    Timeout,
    /// A reader changed something.
    Signal,
    /// Nothing can wake it again: the loop ends.
    Closed,
}

/// The timer thread's wait: until `d` has passed or a reader wakes it.
/// Tests drive the loop with a fake one.
pub trait Waiter: Send {
    fn wait(&mut self, d: Duration) -> Woke;
}

/// The real [`Waiter`]: the wake channel [`TimerShared::new`] returns. A
/// signal drains any other pending one.
pub struct ChannelWaiter(pub Receiver<()>);

impl Waiter for ChannelWaiter {
    fn wait(&mut self, d: Duration) -> Woke {
        match self.0.recv_timeout(d) {
            Ok(()) => {
                while self.0.try_recv().is_ok() {}
                Woke::Signal
            }
            Err(RecvTimeoutError::Timeout) => Woke::Timeout,
            Err(RecvTimeoutError::Disconnected) => Woke::Closed,
        }
    }
}

/// Where the timer thread hands the timers' texts. Never blocks: the writer
/// thread does the disk work.
pub trait TextSink: Send {
    fn update(&mut self, u: TextUpdate);
}

/// The latest texts for the writer thread: a newer set replaces one it
/// hasn't taken yet. Its lock is held only to swap a value in or out.
pub struct TextSlot {
    latest: Mutex<Option<TextUpdate>>,
    ready: Condvar,
}

impl TextSlot {
    pub fn new() -> Self {
        Self {
            latest: Mutex::new(None),
            ready: Condvar::new(),
        }
    }

    /// Replaces what's there and wakes the writer.
    pub fn put(&self, u: TextUpdate) {
        *lock(&self.latest) = Some(u);
        self.ready.notify_one();
    }

    /// The latest set, waiting up to `d` for one; `None` when `d` passed
    /// with nothing new.
    pub fn take_or_wait(&self, d: Duration) -> Option<TextUpdate> {
        let latest = lock(&self.latest);
        let (mut latest, _) = self
            .ready
            .wait_timeout_while(latest, d, |u| u.is_none())
            .unwrap_or_else(PoisonError::into_inner);
        latest.take()
    }
}

impl Default for TextSlot {
    fn default() -> Self {
        Self::new()
    }
}

impl TextSink for Arc<TextSlot> {
    fn update(&mut self, u: TextUpdate) {
        self.put(u);
    }
}

/// What the core's lock guards: the core, and the sleep check every
/// reading is asked about, so each check is ordered with its reading.
struct Guarded {
    core: TimerCore,
    sleep: Box<dyn SleepCheck>,
}

/// The core, its clock, the timer thread's wake and the timers' folder,
/// shared by the commands, the hotkey step and the timer thread.
pub struct TimerShared {
    inner: Mutex<Guarded>,
    clock: Arc<dyn TimerClock>,
    /// One pending wake is enough: the thread reads everything again when it
    /// wakes. Bounded, so a stopped thread can't make it grow.
    wake: SyncSender<()>,
    dir: PathBuf,
}

impl TimerShared {
    /// An empty core. The receiver is the timer thread's wake channel, for
    /// [`ChannelWaiter`].
    pub fn new(
        clock: Arc<dyn TimerClock>,
        sleep: Box<dyn SleepCheck>,
        dir: PathBuf,
    ) -> (Arc<Self>, Receiver<()>) {
        let (wake, woken) = mpsc::sync_channel(1);
        let shared = Self {
            inner: Mutex::new(Guarded {
                core: TimerCore::new(),
                sleep,
            }),
            clock,
            wake,
            dir,
        };
        (Arc::new(shared), woken)
    }

    /// `act` under the core's lock, with this reader's one clock reading
    /// (`now`, `wall_ms`) and that reading's sleep check, taken in that order
    /// before anything else (D5).
    fn with_core<T>(
        &self,
        act: impl FnOnce(&mut TimerCore, Duration, i64, Option<Duration>) -> T,
    ) -> T {
        let mut guarded = lock(&self.inner);
        let g = &mut *guarded;
        let now = self.clock.now();
        let wall_ms = self.clock.wall_ms();
        let slept_from = g.sleep.jumped(now);
        act(&mut g.core, now, wall_ms, slept_from)
    }

    /// `sync_timers`: the entries are read outside the lock, each on its own
    /// (an unreadable one is skipped and logged, and so is everything past
    /// the [`MAX_SYNC_ENTRIES`]th); then the core takes them, and the answer
    /// is the whole set. Never fails (D35).
    pub fn sync(&self, entries: Vec<Value>) -> TimerSnapshot {
        if entries.len() > MAX_SYNC_ENTRIES {
            tracing::warn!(
                skipped = entries.len() - MAX_SYNC_ENTRIES,
                "sync_timers: entries past the limit skipped"
            );
        }
        let mut read = Vec::with_capacity(entries.len().min(MAX_SYNC_ENTRIES));
        for entry in entries.iter().take(MAX_SYNC_ENTRIES) {
            match entry_from_wire(entry) {
                Ok(e) => read.push(e),
                Err(reason) => tracing::warn!(%reason, "sync_timers: an entry skipped"),
            }
        }
        let (snapshot, left) = self.with_core(|core, now, wall_ms, slept_from| {
            let left = core.sync(read, now, wall_ms, slept_from);
            (core.snapshot(now), left)
        });
        if !left.is_empty() {
            tracing::info!(?left, "timers removed");
        }
        self.wake_thread();
        snapshot
    }

    /// A command's `op` on `id`; the answer is the whole set, or "no such
    /// timer".
    pub fn apply(&self, id: &str, op: TimerOp) -> Result<TimerSnapshot, String> {
        let answer = self.with_core(|core, now, wall_ms, slept_from| {
            core.apply(id, op, now, wall_ms, slept_from)
                .map(|()| core.snapshot(now))
        });
        // Even a refused op ran the tick, which may have fired something.
        self.wake_thread();
        answer.map_err(|e| e.to_string())
    }

    /// `timer_status`: the whole set, ticked first. Wakes the thread only
    /// when the tick changed something.
    pub fn status(&self) -> TimerSnapshot {
        let (snapshot, dirty) = self.with_core(|core, now, wall_ms, slept_from| {
            core.tick(now, wall_ms, slept_from);
            let snapshot = core.snapshot(now);
            (snapshot, core.is_dirty())
        });
        if dirty {
            self.wake_thread();
        }
        snapshot
    }

    /// The timers' folder (`timers/`, `timers-dev/` for `tauri dev`).
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Never blocks. Full: a wake is already pending. Disconnected: there is
    /// no thread to wake.
    fn wake_thread(&self) {
        let _ = self.wake.try_send(());
    }
}

/// The poison-tolerant lock the tree uses: a panic elsewhere mustn't stop
/// the timers.
fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// D19: the shortcut handler's timer step. A press of `tm:<id>:toggle`,
/// `tm:<id>:reset` or (v1.60.0, End now) `tm:<id>:finish` acts on that
/// timer; a release, any other id, or an id the core doesn't run does
/// nothing. Runs inside `WM_HOTKEY` on the main thread, so it only takes
/// the core's lock for microseconds: it never registers, unregisters,
/// writes, logs or emits (the timer thread logs and emits).
pub fn timer_on_hotkey(shared: &TimerShared, id: &str, pressed: bool) {
    let Some((timer, op)) = hotkey_timer_request(id, pressed) else {
        return;
    };
    let _ = shared.with_core(|core, now, wall_ms, slept_from| {
        core.apply(timer, op, now, wall_ms, slept_from)
    });
    shared.wake_thread();
}

/// The `divora-timers` loop (D13): wait until the next wake instant (or a
/// reader's wake), then under the lock read the clock once, check for a
/// sleep, tick, build a snapshot if anything changed, take the texts (once
/// a sync has said which timers exist), compute the next wake instant from
/// that same reading, and drain the transitions and notes — any reader's,
/// so one a command or a press left is logged within a [`TICK`] at most;
/// then, unlocked, log them, emit, and hand the texts on. Returns when the
/// waiter reports [`Woke::Closed`]. Tests run it on their own thread with
/// fakes.
///
/// [`TICK`]: crate::timers::TICK
#[doc(hidden)]
pub fn run_timer_loop(
    shared: &TimerShared,
    waiter: &mut dyn Waiter,
    events: &dyn TimerEvents,
    sink: &mut dyn TextSink,
    cap: Duration,
) {
    let mut wake_at = Duration::ZERO;
    loop {
        if waiter.wait(wake_at.saturating_sub(shared.clock.now())) == Woke::Closed {
            return;
        }
        let (snapshot, update, transitions, notes) =
            shared.with_core(|core, now, wall_ms, slept_from| {
                core.tick(now, wall_ms, slept_from);
                let snapshot = core.take_dirty().then(|| core.snapshot(now));
                let update = (snapshot.is_some() && core.synced()).then(|| core.text_update());
                wake_at = now.saturating_add(core.next_wake(now, cap));
                (snapshot, update, core.take_transitions(), core.take_notes())
            });
        for n in &notes {
            log_note(n);
        }
        for t in &transitions {
            log_transition(t);
        }
        if let Some(s) = snapshot {
            events.timer_state(&s);
        }
        if let Some(u) = update {
            sink.update(u);
        }
    }
}

/// One line per transition (D34): what S2 reads back, and v1.60.0's place
/// to act from.
fn log_transition(t: &Transition) {
    match t {
        Transition::Fired {
            id,
            fires,
            finished: true,
            ..
        } => {
            tracing::info!(id = %id, fires, "a timer was ended now");
        }
        Transition::Fired {
            id, fires, late_ms, ..
        } => {
            if Duration::from_millis(*late_ms) > LATE_LOG {
                tracing::warn!(id = %id, fires, late_ms, "a timer finished late");
            } else {
                tracing::info!(id = %id, fires, late_ms, "a timer finished");
            }
        }
        Transition::Expired { id, .. } => {
            tracing::info!(id = %id, "a timer ran out while the PC slept; not finished");
        }
        Transition::SleepPaused { id } => {
            tracing::info!(id = %id, "a count-up paused where the PC slept");
        }
        Transition::Seeded {
            id, phase, from, ..
        } => {
            tracing::info!(id = %id, ?phase, ?from, "a timer picked up its stored run");
        }
        Transition::Started { id, fires, .. } => {
            tracing::info!(id = %id, fires, "a timer started");
        }
        Transition::Cancelled { id, why, .. } => match why {
            CancelWhy::Reset => tracing::info!(id = %id, "a running timer was reset"),
            CancelWhy::Removed => tracing::info!(id = %id, "a running timer was deleted"),
        },
    }
}

/// One line per note: the clock that decided a sleep (D5), a held wall
/// step followed (D3), a duplicate id skipped (D35).
fn log_note(n: &Note) {
    match n {
        Note::ClockSlept {
            slept_ms,
            running,
            expired,
            paused,
        } => tracing::info!(
            slept_ms,
            running,
            expired,
            paused,
            "timers: the PC slept, and the clock counted it"
        ),
        Note::WallChecked {
            running,
            corrected,
            expired,
            paused,
        } => tracing::info!(
            running,
            corrected,
            expired,
            paused,
            "timers: a sleep signal with no clock jump; checked against the wall clock"
        ),
        Note::AnchorFollowed { id, step_ms } => tracing::info!(
            id = %id,
            step_ms,
            "timers: the wall clock moved; the anchor follows"
        ),
        Note::DuplicateId { id } => {
            tracing::warn!(id = %id, "sync_timers: a duplicate id skipped");
        }
    }
}

/// Starts `divora-timers` running [`run_timer_loop`]. A failed spawn is
/// logged, and timers then never run (commands still answer).
pub fn spawn_timer_thread(
    shared: Arc<TimerShared>,
    mut waiter: impl Waiter + 'static,
    events: impl TimerEvents,
    mut sink: impl TextSink + 'static,
    cap: Duration,
) {
    let spawned = std::thread::Builder::new()
        .name("divora-timers".into())
        .spawn(move || run_timer_loop(&shared, &mut waiter, &events, &mut sink, cap));
    if let Err(e) = spawned {
        tracing::error!(error = %e, "couldn't start the timer thread; timers won't run");
    }
}

/// The timers' folder under the app data dir. `tauri dev` builds keep their
/// own timers (their own localStorage), so their own files: one build can't
/// sweep the other's.
#[doc(hidden)]
pub fn timers_dir_name(dev: bool) -> &'static str {
    if dev {
        "timers-dev"
    } else {
        "timers"
    }
}

/// K1's kill switch: `false` keeps `divora-timer-files` from starting, so no
/// timer file is written, removed or swept. The overlay and Settings don't
/// depend on the files.
pub const TEXT_FILES: bool = true;

/// A pending write's next try: it ends at the next system timer tick.
pub const RETRY_WAIT: Duration = Duration::from_millis(10);

/// A file failing this long is tried once a second (or on a new text).
pub const BACKOFF_AFTER: Duration = Duration::from_secs(1);

/// How often the writer checks that the files it wrote still exist.
pub const REPAIR_EVERY: Duration = Duration::from_secs(5);

/// After [`BACKOFF_AFTER`] of failures, how often a file is tried.
const BACKOFF_EVERY: Duration = Duration::from_secs(1);

/// The collision line is logged at most this often.
const COLLISION_LOG_EVERY: Duration = Duration::from_secs(60);

/// The writer thread's work, one pass at a time (§7.13).
pub trait TextWriter: Send {
    /// One pass: `u` is a new set from the slot, or `None` when the wait ran
    /// out. `now` is injected so tests drive time. Returns how long to wait
    /// before the next pass while something is pending (`None`: wait for a
    /// new set, or [`REPAIR_EVERY`]).
    fn apply(&mut self, u: Option<TextUpdate>, now: Instant) -> Option<Duration>;
}

/// What a failing file is waiting to have done to it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Op {
    Write,
    Remove,
}

/// An error, as far as "said once" goes: the OS code, or the kind when
/// there is none.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum ErrorKey {
    Raw(i32),
    Kind(io::ErrorKind),
}

impl ErrorKey {
    fn of(e: &io::Error) -> Self {
        e.raw_os_error().map_or(Self::Kind(e.kind()), Self::Raw)
    }
}

/// A write or removal that has failed since `since` and not yet landed.
#[derive(Debug)]
struct Failing {
    op: Op,
    since: Instant,
    /// Tried again at the first pass from then on: at once for the first
    /// [`BACKOFF_AFTER`], then once a second (or at once on a new text).
    next_try: Instant,
    /// The errors warned about: one line each.
    warned: Vec<ErrorKey>,
    /// A reader (or anything else [`retryable`] covers) held it at least
    /// once: a write that then lands counts as one that waited for a reader.
    collided: bool,
}

impl Failing {
    fn new(op: Op, now: Instant) -> Self {
        Self {
            op,
            since: now,
            next_try: now,
            warned: Vec::new(),
            collided: false,
        }
    }
}

/// The `.txt` files (D21, §7.13): written by temp file and rename, retried
/// while a reader holds one, removed when their timer goes, swept once at
/// the first set (which comes only after a sync), repaired every
/// [`REPAIR_EVERY`].
///
/// A write or removal that fails stays pending and is tried at every pass —
/// the writer thread comes back in [`RETRY_WAIT`], so about every system
/// timer tick — until it lands or a newer text replaces it. After
/// [`BACKOFF_AFTER`] of failures it is tried once a second (or at once on a
/// new text), with one `warn!` per (id, error) and one `info!` when it lands
/// again. An OBS read collides with a rename now and then and isn't an
/// error: it is said only once it has lasted [`BACKOFF_AFTER`], and counted
/// in the collision line, at most one a minute. Any other error is said at
/// once.
pub struct TextFiles {
    dir: PathBuf,
    /// Each run timer's latest text, in list order: what its file should
    /// hold.
    want: Vec<(String, String)>,
    /// What each file holds as far as this writer knows. Set only once a
    /// rename has landed, so a write that failed is tried again.
    written: HashMap<String, String>,
    /// Ids whose file is to be removed: their timer left, or the sweep found
    /// a `timer-<n>.txt` no timer has. An id that comes back leaves.
    gone: BTreeSet<String>,
    /// The last set's `keep`: an id that leaves it lost its timer, run or
    /// kept (D36) — a kept one's file was never written here, so neither
    /// `want` nor `written` has it.
    kept: HashSet<String>,
    failing: HashMap<String, Failing>,
    swept: bool,
    last_repair: Option<Instant>,
    /// Writes that waited for a reader since the last collision line, and
    /// the longest wait.
    collisions: u64,
    longest_wait: Duration,
    last_collision_log: Option<Instant>,
    /// Writes tried and writes failed, for the tests.
    tried: u64,
    failed: u64,
}

impl TextFiles {
    pub fn new(dir: PathBuf) -> Self {
        Self {
            dir,
            want: Vec::new(),
            written: HashMap::new(),
            gone: BTreeSet::new(),
            kept: HashSet::new(),
            failing: HashMap::new(),
            swept: false,
            last_repair: None,
            collisions: 0,
            longest_wait: Duration::ZERO,
            last_collision_log: None,
            tried: 0,
            failed: 0,
        }
    }

    /// Writes tried so far, and how many of them failed.
    #[doc(hidden)]
    pub fn tries(&self) -> (u64, u64) {
        (self.tried, self.failed)
    }

    fn file(&self, id: &str) -> PathBuf {
        self.dir.join(format!("{id}.txt"))
    }

    /// A new set: the texts to write, and every id whose file stays.
    fn take(&mut self, u: TextUpdate, now: Instant) {
        let keep: HashSet<String> = u.keep.into_iter().collect();
        if !self.swept {
            self.swept = true;
            self.sweep(&keep);
        }
        // A file this writer wrote, or meant to, or that the last set kept,
        // whose timer left goes. An id back in the set keeps its file.
        let left: Vec<String> = self
            .want
            .iter()
            .map(|(id, _)| id)
            .chain(self.written.keys())
            .chain(&self.kept)
            .filter(|id| !keep.contains(*id))
            .cloned()
            .collect();
        self.gone.extend(left);
        self.gone.retain(|id| !keep.contains(id));
        // A new text for a failing file is tried at once.
        for (id, text) in &u.texts {
            let old = self.want.iter().find(|(was, _)| was == id);
            if old.is_none_or(|(_, was)| was != text) {
                if let Some(f) = self.failing.get_mut(id) {
                    f.next_try = now;
                }
            }
        }
        self.want = u.texts;
        // Nothing is pending for an id neither written nor removed any more
        // (its timer is kept, not run).
        let (want, gone) = (&self.want, &self.gone);
        self.failing
            .retain(|id, _| gone.contains(id) || want.iter().any(|(w, _)| w == id));
        self.kept = keep;
    }

    /// The one sweep, at the first set after the first sync (D21): every
    /// `timer-<n>.txt` no timer keeps goes (as a removal, retried like any
    /// other), and so does every `.timer-<n>.tmp` a write cut short left.
    /// Nothing else in the folder is touched ([`sweepable`]).
    fn sweep(&mut self, keep: &HashSet<String>) {
        let entries = match std::fs::read_dir(&self.dir) {
            Ok(entries) => entries,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return,
            Err(e) => {
                tracing::warn!(
                    path = %self.dir.display(),
                    error = %e,
                    "timer files: couldn't list the folder; nothing swept"
                );
                return;
            }
        };
        let (mut old, mut temps) = (0_usize, 0_usize);
        for entry in entries.flatten() {
            if !entry.file_type().is_ok_and(|t| t.is_file()) {
                continue;
            }
            let name = entry.file_name();
            let Some(name) = name.to_str() else {
                continue;
            };
            if !sweepable(name, keep) {
                continue;
            }
            if let Some(id) = name.strip_suffix(".txt") {
                self.gone.insert(id.to_string());
                old += 1;
            } else {
                match std::fs::remove_file(entry.path()) {
                    Ok(()) => temps += 1,
                    Err(e) if e.kind() == io::ErrorKind::NotFound => {}
                    Err(e) => {
                        tracing::warn!(
                            path = %entry.path().display(),
                            error = %e,
                            "timer files: couldn't remove a leftover temp"
                        );
                    }
                }
            }
        }
        if old + temps > 0 {
            tracing::info!(
                old,
                temps,
                "timer files: sweeping the ones no timer has, and leftover temps"
            );
        }
    }

    /// Every written file that has gone missing (deleted by hand, or with
    /// its folder) is forgotten, so this pass writes it again.
    fn repair(&mut self) {
        let missing: Vec<String> = self
            .written
            .keys()
            .filter(|id| !self.gone.contains(*id))
            .filter(|id| {
                std::fs::metadata(self.file(id)).is_err_and(|e| e.kind() == io::ErrorKind::NotFound)
            })
            .cloned()
            .collect();
        if !missing.is_empty() {
            tracing::info!(
                ?missing,
                "timer files: some went missing; writing them again"
            );
            for id in &missing {
                self.written.remove(id);
            }
        }
    }

    /// Whether `op` on `id` is tried in this pass.
    fn due(&self, id: &str, op: Op, now: Instant) -> bool {
        self.failing
            .get(id)
            .is_none_or(|f| f.op != op || now >= f.next_try)
    }

    /// Every text that isn't on disk yet, in list order.
    fn write_due(&mut self, now: Instant) {
        let want = std::mem::take(&mut self.want);
        for (id, text) in &want {
            if self.written.get(id) == Some(text) {
                // Nothing pending: a failure of an older text is over.
                self.failing.remove(id);
                continue;
            }
            if !self.due(id, Op::Write, now) {
                continue;
            }
            self.tried += 1;
            match write_text(&self.dir, id, text) {
                Ok(()) => {
                    self.written.insert(id.clone(), text.clone());
                    self.landed(id, Op::Write, now);
                }
                Err(e) => {
                    self.failed += 1;
                    self.note_failure(id, Op::Write, &e, now);
                }
            }
        }
        self.want = want;
    }

    /// Every file whose timer left; one that is already gone counts as
    /// removed. The id is forgotten once its file is.
    fn remove_due(&mut self, now: Instant) {
        for id in std::mem::take(&mut self.gone) {
            if !self.due(&id, Op::Remove, now) {
                self.gone.insert(id);
                continue;
            }
            match std::fs::remove_file(self.file(&id)) {
                Ok(()) => {}
                Err(e) if e.kind() == io::ErrorKind::NotFound => {}
                Err(e) => {
                    self.note_failure(&id, Op::Remove, &e, now);
                    self.gone.insert(id);
                    continue;
                }
            }
            self.written.remove(&id);
            self.landed(&id, Op::Remove, now);
        }
    }

    /// `op` on `id` landed: its failure is over.
    fn landed(&mut self, id: &str, op: Op, now: Instant) {
        // A failure of the other op (a removal whose id came back) just ends.
        let Some(f) = self.failing.remove(id).filter(|f| f.op == op) else {
            return;
        };
        let waited = now.saturating_duration_since(f.since);
        if op == Op::Write && f.collided {
            self.collisions += 1;
            self.longest_wait = self.longest_wait.max(waited);
        }
        if f.warned.is_empty() {
            return;
        }
        let (path, waited_ms) = (self.file(id), waited.as_millis());
        match op {
            Op::Write => tracing::info!(
                id = %id,
                path = %path.display(),
                waited_ms,
                "timer files: written again"
            ),
            Op::Remove => tracing::info!(
                id = %id,
                path = %path.display(),
                waited_ms,
                "timer files: removed at last"
            ),
        }
    }

    /// `op` on `id` failed with `e`: it stays pending (§7.13).
    fn note_failure(&mut self, id: &str, op: Op, e: &io::Error, now: Instant) {
        let retry = retryable(e);
        let path = self.file(id);
        let f = match self.failing.entry(id.to_string()) {
            Entry::Occupied(mut was) => {
                if was.get().op != op {
                    was.insert(Failing::new(op, now));
                }
                was.into_mut()
            }
            Entry::Vacant(none) => none.insert(Failing::new(op, now)),
        };
        f.collided |= retry;
        let failing_for = now.saturating_duration_since(f.since);
        f.next_try = if failing_for >= BACKOFF_AFTER {
            now.checked_add(BACKOFF_EVERY).unwrap_or(now)
        } else {
            now
        };
        // A reader's hold is said only once it has lasted; anything else at
        // once. Each error once.
        let key = ErrorKey::of(e);
        if (retry && failing_for < BACKOFF_AFTER) || f.warned.contains(&key) {
            return;
        }
        f.warned.push(key);
        let failing_ms = failing_for.as_millis();
        match op {
            Op::Write => tracing::warn!(
                id = %id,
                path = %path.display(),
                error = %e,
                failing_ms,
                "timer files: can't write it; trying again (once a second after a second)"
            ),
            Op::Remove => tracing::warn!(
                id = %id,
                path = %path.display(),
                error = %e,
                failing_ms,
                "timer files: can't remove it; trying again (once a second after a second)"
            ),
        }
    }

    /// H2's observable: at most one line a minute saying how many writes
    /// waited for a reader since the last one, and the longest wait.
    fn log_collisions(&mut self, now: Instant) {
        if self.collisions == 0
            || self
                .last_collision_log
                .is_some_and(|at| now.saturating_duration_since(at) < COLLISION_LOG_EVERY)
        {
            return;
        }
        tracing::info!(
            "timer files: {} writes waited for a reader (longest {} ms)",
            self.collisions,
            self.longest_wait.as_millis()
        );
        self.collisions = 0;
        self.longest_wait = Duration::ZERO;
        self.last_collision_log = Some(now);
    }

    /// How long until the next pass is needed: the soonest try of anything
    /// pending, never sooner than [`RETRY_WAIT`]. `None` with nothing
    /// pending.
    fn next_wait(&self, now: Instant) -> Option<Duration> {
        let writes = self
            .want
            .iter()
            .filter(|(id, text)| self.written.get(id) != Some(text))
            .map(|(id, _)| (id, Op::Write));
        let removals = self.gone.iter().map(|id| (id, Op::Remove));
        writes
            .chain(removals)
            .map(|(id, op)| match self.failing.get(id) {
                Some(f) if f.op == op => f.next_try.saturating_duration_since(now).max(RETRY_WAIT),
                _ => RETRY_WAIT,
            })
            .min()
    }
}

impl TextWriter for TextFiles {
    /// One pass (§7.13): take a new set (sweeping at the first), check the
    /// written files every [`REPAIR_EVERY`], write what's due, remove what's
    /// due, maybe log the collision line.
    fn apply(&mut self, u: Option<TextUpdate>, now: Instant) -> Option<Duration> {
        if let Some(u) = u {
            self.take(u, now);
        }
        if self
            .last_repair
            .is_none_or(|at| now.saturating_duration_since(at) >= REPAIR_EVERY)
        {
            self.last_repair = Some(now);
            self.repair();
        }
        self.write_due(now);
        self.remove_due(now);
        self.log_collisions(now);
        self.next_wait(now)
    }
}

/// Starts `divora-timer-files`: it takes the latest texts from `slot` and
/// hands them to `writer`, waiting as long as the writer asks ([`REPAIR_EVERY`]
/// when nothing is pending). It ends once nothing else holds the slot — in
/// the app, never: the timer thread holds it (D14). With [`TEXT_FILES`] off
/// it doesn't start.
pub fn spawn_text_writer(slot: &Arc<TextSlot>, mut writer: impl TextWriter + 'static) {
    if !TEXT_FILES {
        tracing::info!("timer files are off");
        return;
    }
    let slot = Arc::downgrade(slot);
    let spawned = std::thread::Builder::new()
        .name("divora-timer-files".into())
        .spawn(move || {
            let mut retry: Option<Duration> = None;
            while let Some(held) = slot.upgrade() {
                let u = held.take_or_wait(retry.unwrap_or(REPAIR_EVERY));
                drop(held);
                retry = writer.apply(u, Instant::now());
            }
        });
    if let Err(e) = spawned {
        tracing::error!(error = %e, "couldn't start the timer file writer; no timer files");
    }
}

/// One write of `text` to `<dir>/<id>.txt`: `.<id>.tmp` is created (the
/// folder too, if it went missing, and the create tried once more),
/// written and closed, then renamed over the file. The file is never removed
/// first, so a reader never finds it missing or part-written (D21). A failed
/// write or rename removes the temp if it can (the next try makes it again;
/// one a crash leaves is swept). The temp can't be an id's file: ids never
/// start with `.`.
#[doc(hidden)]
pub fn write_text(dir: &Path, id: &str, text: &str) -> io::Result<()> {
    let temp = dir.join(format!(".{id}.tmp"));
    let mut file = match std::fs::File::create(&temp) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            std::fs::create_dir_all(dir)?;
            std::fs::File::create(&temp)?
        }
        created => created?,
    };
    let written = file.write_all(text.as_bytes());
    drop(file);
    let landed = written.and_then(|()| std::fs::rename(&temp, dir.join(format!("{id}.txt"))));
    if landed.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    landed
}

/// `ERROR_SHARING_VIOLATION`: the temp itself is held (a scanner, say).
const SHARING_VIOLATION: i32 = 32;

/// Whether a write or removal that failed this way is worth trying again
/// soon: a rename onto a file a reader holds (OBS's read) fails with
/// `ERROR_ACCESS_DENIED` (5, std's `PermissionDenied`), and a held temp with
/// a sharing violation (32), as measured here. Both pass once the reader
/// lets go.
#[doc(hidden)]
pub fn retryable(e: &io::Error) -> bool {
    e.kind() == io::ErrorKind::PermissionDenied
        || (cfg!(windows) && e.raw_os_error() == Some(SHARING_VIOLATION))
}

/// Whether the one sweep after the first sync removes `name` from the
/// timers' folder: only names Divora makes — `timer-<n>.txt` (n 1-99999)
/// whose id isn't in `keep`, and any `.timer-<n>.tmp` — so a user's own files
/// there are never touched.
#[doc(hidden)]
#[allow(clippy::implicit_hasher)] // the writer's own set
pub fn sweepable(name: &str, keep: &HashSet<String>) -> bool {
    if let Some(id) = name.strip_suffix(".txt") {
        numbered(id) && !keep.contains(id)
    } else {
        name.strip_prefix('.')
            .and_then(|temp| temp.strip_suffix(".tmp"))
            .is_some_and(numbered)
    }
}

/// `timer-<n>` with n from 1 to 99999, no leading zero: an id the page
/// picks (D9).
fn numbered(id: &str) -> bool {
    id.strip_prefix("timer-").is_some_and(|n| {
        (1..=5).contains(&n.len()) && !n.starts_with('0') && n.bytes().all(|b| b.is_ascii_digit())
    })
}

#[cfg(test)]
mod tests {
    use super::{
        retryable, run_timer_loop, spawn_text_writer, sweepable, timer_on_hotkey, timers_dir_name,
        write_text, ChannelWaiter, RealClock, SleepCheck, TextFiles, TextSink, TextSlot,
        TextWriter, TimerClock, TimerEvents, TimerShared, Waiter, Woke, BACKOFF_AFTER,
        REPAIR_EVERY, RETRY_WAIT,
    };
    use crate::timer_actions::Planned;
    use crate::timers::TimerPhase::{Done, Expired, Idle, Paused, Running};
    use crate::timers::{
        timer_text, TextUpdate, TimerKind, TimerOp, TimerPhase, TimerSnapshot, MAX_SYNC_ENTRIES,
        TICK,
    };
    use serde_json::{json, Value};
    use std::collections::{HashSet, VecDeque};
    use std::io;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    fn secs(n: u64) -> Duration {
        Duration::from_secs(n)
    }

    // ---- fakes and helpers ----------------------------------------------

    /// A clock the test moves.
    struct FakeClock {
        now: Mutex<Duration>,
        wall_at_zero: i64,
    }

    impl FakeClock {
        fn set(&self, to: Duration) {
            *self.now.lock().unwrap() = to;
        }
    }

    impl TimerClock for FakeClock {
        fn now(&self) -> Duration {
            *self.now.lock().unwrap()
        }

        fn wall_ms(&self) -> i64 {
            let ms = i64::try_from(self.now().as_millis()).unwrap();
            self.wall_at_zero + ms
        }
    }

    /// No sleep, ever.
    struct Awake;

    impl SleepCheck for Awake {
        fn jumped(&mut self, _clock: Duration) -> Option<Duration> {
            None
        }
    }

    /// One sleep, when the test says: reported to the next reading as the
    /// reading before it, the way v1.54.0's `SleepWatch` reports one.
    struct OneSleep {
        last: Option<Duration>,
        slept: Arc<AtomicBool>,
        reported: Arc<Mutex<Option<Duration>>>,
    }

    impl SleepCheck for OneSleep {
        fn jumped(&mut self, clock: Duration) -> Option<Duration> {
            let before = self.last.replace(clock);
            if !self.slept.swap(false, Ordering::SeqCst) {
                return None;
            }
            *self.reported.lock().unwrap() = before;
            before
        }
    }

    /// A command a [`FakeWaiter`] lands mid-wait.
    type Step = Box<dyn FnOnce(&TimerShared) + Send>;

    /// Records each wait with the clock it was asked at, and moves the clock
    /// by it — or to the next scripted command, which it runs (as a page or
    /// a hotkey would, from another thread) and reports as a wake. Closes
    /// after `left` waits, or once the clock reaches `until`.
    struct FakeWaiter {
        clock: Arc<FakeClock>,
        shared: Arc<TimerShared>,
        script: VecDeque<(Duration, Step)>,
        waits: Arc<Mutex<Vec<(Duration, Duration)>>>,
        left: usize,
        until: Duration,
    }

    impl FakeWaiter {
        fn new(shared: &Arc<TimerShared>, clock: &Arc<FakeClock>) -> Self {
            Self {
                clock: clock.clone(),
                shared: shared.clone(),
                script: VecDeque::new(),
                waits: Arc::default(),
                left: usize::MAX,
                until: Duration::MAX,
            }
        }

        fn left(mut self, left: usize) -> Self {
            self.left = left;
            self
        }

        fn until(mut self, until: Duration) -> Self {
            self.until = until;
            self
        }

        /// Runs `step` at clock `at` (in order).
        fn at(mut self, at: Duration, step: impl FnOnce(&TimerShared) + Send + 'static) -> Self {
            self.script.push_back((at, Box::new(step)));
            self
        }
    }

    impl Waiter for FakeWaiter {
        fn wait(&mut self, d: Duration) -> Woke {
            let now = self.clock.now();
            if self.left == 0 || now >= self.until {
                return Woke::Closed;
            }
            self.left -= 1;
            self.waits.lock().unwrap().push((now, d));
            let end = now.saturating_add(d).min(self.until);
            if self.script.front().is_some_and(|(at, _)| *at <= end) {
                let (at, step) = self.script.pop_front().unwrap();
                self.clock.set(at.max(now));
                step(&self.shared);
                return Woke::Signal;
            }
            self.clock.set(end);
            Woke::Timeout
        }
    }

    /// Every emit, with the fake clock it went out at.
    #[derive(Clone)]
    struct Stamped {
        clock: Arc<FakeClock>,
        seen: Arc<Mutex<Vec<(Duration, TimerSnapshot)>>>,
    }

    impl Stamped {
        fn new(clock: &Arc<FakeClock>) -> Self {
            Self {
                clock: clock.clone(),
                seen: Arc::default(),
            }
        }

        fn all(&self) -> Vec<(Duration, TimerSnapshot)> {
            self.seen.lock().unwrap().clone()
        }
    }

    impl TimerEvents for Stamped {
        fn timer_state(&self, s: &TimerSnapshot) {
            self.seen
                .lock()
                .unwrap()
                .push((self.clock.now(), s.clone()));
        }

        fn timer_actions(&self, _p: &Planned, _seq: u64) {}
    }

    /// Every set handed to the writer.
    #[derive(Default)]
    struct Handed(Vec<TextUpdate>);

    impl TextSink for Handed {
        fn update(&mut self, u: TextUpdate) {
            self.0.push(u);
        }
    }

    /// An empty core on a fake clock, and its wake channel.
    fn fake_with(sleep: Box<dyn SleepCheck>) -> (Arc<TimerShared>, Arc<FakeClock>, Receiver<()>) {
        let clock = Arc::new(FakeClock {
            now: Mutex::new(Duration::ZERO),
            wall_at_zero: 1_793_217_600_000,
        });
        let (shared, woken) = TimerShared::new(clock.clone(), sleep, unused_dir());
        (shared, clock, woken)
    }

    fn fake() -> (Arc<TimerShared>, Arc<FakeClock>, Receiver<()>) {
        fake_with(Box::new(Awake))
    }

    /// An empty core on the real clocks.
    fn real() -> (Arc<TimerShared>, Receiver<()>) {
        TimerShared::new(Arc::new(RealClock::new()), Box::new(Awake), unused_dir())
    }

    /// The folder the core is given; nothing in these tests writes it.
    fn unused_dir() -> PathBuf {
        std::env::temp_dir().join("divora-timers-unused")
    }

    fn countdown(id: &str, length_ms: u64) -> Value {
        json!({"id": id, "name": "BRB", "kind": "countdown", "durationMs": length_ms})
    }

    fn countup(id: &str) -> Value {
        json!({"id": id, "name": "Uptime", "kind": "countup", "durationMs": 300_000})
    }

    /// One pass of the real loop on the fake clock: the set it hands the
    /// writer, if any.
    fn one_pass(shared: &Arc<TimerShared>, clock: &Arc<FakeClock>) -> Option<TextUpdate> {
        let mut waiter = FakeWaiter::new(shared, clock).left(1);
        let mut handed = Handed::default();
        run_timer_loop(shared, &mut waiter, &Stamped::new(clock), &mut handed, TICK);
        handed.0.pop()
    }

    /// The real wait on the wake channel, closed after `run` of real time.
    struct Until {
        inner: ChannelWaiter,
        end: Instant,
    }

    impl Until {
        fn new(woken: Receiver<()>, run: Duration) -> Self {
            Self {
                inner: ChannelWaiter(woken),
                end: Instant::now() + run,
            }
        }
    }

    impl Waiter for Until {
        fn wait(&mut self, d: Duration) -> Woke {
            let left = self.end.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Woke::Closed;
            }
            self.inner.wait(d.min(left))
        }
    }

    /// Every emit, with the real time it was heard.
    #[derive(Clone, Default)]
    struct Heard(Arc<Mutex<Vec<(Instant, TimerSnapshot)>>>);

    impl TimerEvents for Heard {
        fn timer_state(&self, s: &TimerSnapshot) {
            self.0.lock().unwrap().push((Instant::now(), s.clone()));
        }

        fn timer_actions(&self, _p: &Planned, _seq: u64) {}
    }

    impl Heard {
        /// The first emit `pred` accepts, waiting up to `limit` for it.
        fn within(
            &self,
            limit: Duration,
            pred: impl Fn(&TimerSnapshot) -> bool,
        ) -> Option<(Instant, TimerSnapshot)> {
            let end = Instant::now() + limit;
            loop {
                let found = self
                    .0
                    .lock()
                    .unwrap()
                    .iter()
                    .find(|(_, s)| pred(s))
                    .cloned();
                if found.is_some() || Instant::now() >= end {
                    return found;
                }
                std::thread::sleep(ms(2));
            }
        }
    }

    /// Whether `f`, run on its own thread, returns within a second.
    fn returns_within<'scope, T: Send + 'scope>(
        s: &'scope std::thread::Scope<'scope, '_>,
        f: impl FnOnce() -> T + Send + 'scope,
    ) -> bool {
        let (done, finished) = mpsc::channel();
        s.spawn(move || {
            let _ = done.send(f());
        });
        finished.recv_timeout(secs(1)).is_ok()
    }

    /// Holds whoever passes it until the test says go.
    struct Gate {
        name: &'static str,
        entered: Sender<&'static str>,
        go: Receiver<()>,
    }

    impl Gate {
        /// The gate, and its go. Passing it sends `name` to `entered`.
        fn new(name: &'static str, entered: &Sender<&'static str>) -> (Self, Sender<()>) {
            let (go_tx, go) = mpsc::channel();
            let entered = entered.clone();
            (Self { name, entered, go }, go_tx)
        }

        fn pass(self) {
            let _ = self.entered.send(self.name);
            let _ = self.go.recv_timeout(secs(10));
        }
    }

    /// Blocks its first emit — a snapshot or (v1.60.0) a hook run — at a
    /// gate.
    struct GatedEvents(Mutex<Option<Gate>>);

    impl GatedEvents {
        fn pass(&self) {
            let gate = self.0.lock().unwrap().take();
            if let Some(gate) = gate {
                gate.pass();
            }
        }
    }

    impl TimerEvents for GatedEvents {
        fn timer_state(&self, _s: &TimerSnapshot) {
            self.pass();
        }

        fn timer_actions(&self, _p: &Planned, _seq: u64) {
            self.pass();
        }
    }

    /// A log file whose first line containing `at` is held at a gate, as a
    /// disk that is still waking up would hold it.
    struct GatedLog {
        at: &'static str,
        gate: Arc<Mutex<Option<Gate>>>,
    }

    impl io::Write for GatedLog {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            if String::from_utf8_lossy(bytes).contains(self.at) {
                let gate = self.gate.lock().unwrap().take();
                if let Some(gate) = gate {
                    gate.pass();
                }
            }
            Ok(bytes.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    /// v1.54.0's backstop: a reading more than 500 ms past the one before
    /// is a sleep the clock counted, reported as that reading before.
    struct Backstop(Option<Duration>);

    impl SleepCheck for Backstop {
        fn jumped(&mut self, clock: Duration) -> Option<Duration> {
            let before = self.0.replace(clock)?;
            (clock.saturating_sub(before) > ms(500)).then_some(before)
        }
    }

    /// Blocks its first hand-off at a gate.
    struct GatedSink(Option<Gate>);

    impl TextSink for GatedSink {
        fn update(&mut self, _u: TextUpdate) {
            if let Some(gate) = self.0.take() {
                gate.pass();
            }
        }
    }

    /// The log lines `f` writes on this thread at INFO and up, formatted as
    /// the app's log is, without times or colours.
    fn logged<T>(f: impl FnOnce() -> T) -> (T, String) {
        let buf: Arc<Mutex<Vec<u8>>> = Arc::default();
        let into = buf.clone();
        let subscriber = tracing_subscriber::fmt()
            .with_writer(move || LogBuf(into.clone()))
            .with_ansi(false)
            .without_time()
            .with_max_level(tracing::Level::INFO)
            .finish();
        let out = tracing::subscriber::with_default(subscriber, f);
        let text = String::from_utf8(buf.lock().unwrap().clone()).unwrap();
        (out, text)
    }

    struct LogBuf(Arc<Mutex<Vec<u8>>>);

    impl io::Write for LogBuf {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    /// Lines of `log` at `level` that contain `needle`.
    fn lines(log: &str, level: &str, needle: &str) -> usize {
        log.lines()
            .filter(|l| l.contains(level) && l.contains(needle))
            .count()
    }

    fn scratch(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("divora-timer-thread-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    fn put(dir: &Path, name: &str, text: &str) {
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(dir.join(name), text).unwrap();
    }

    fn read(dir: &Path, name: &str) -> Option<String> {
        std::fs::read_to_string(dir.join(name)).ok()
    }

    /// A set whose files are exactly its texts'.
    fn set(texts: &[(&str, &str)]) -> TextUpdate {
        TextUpdate {
            texts: texts
                .iter()
                .map(|&(id, text)| (id.to_string(), text.to_string()))
                .collect(),
            keep: texts.iter().map(|&(id, _)| id.to_string()).collect(),
        }
    }

    /// Opens `path` the way OBS's text source reads it: `_wfopen(…, "rb")`
    /// shares read and write but not delete, so while it is open a rename
    /// onto the file fails (5) and so does removing it (32).
    #[cfg(windows)]
    fn obs_open(path: &Path) -> io::Result<std::fs::File> {
        use std::os::windows::fs::OpenOptionsExt;
        const FILE_SHARE_READ: u32 = 1;
        const FILE_SHARE_WRITE: u32 = 2;
        std::fs::OpenOptions::new()
            .read(true)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
            .open(path)
    }

    #[cfg(windows)]
    fn set_read_only(path: &Path, on: bool) {
        let mut permissions = std::fs::metadata(path).unwrap().permissions();
        permissions.set_readonly(on);
        std::fs::set_permissions(path, permissions).unwrap();
    }

    // ---- S1: the empty core, the slot, the helpers -----------------------

    #[test]
    fn timers_dir_name_table() {
        assert_eq!(timers_dir_name(false), "timers");
        assert_eq!(timers_dir_name(true), "timers-dev");
    }

    /// S1's promise to every launch: with no timers (no `divora.timers`, or
    /// before the page's first sync) the loop waits a whole [`TICK`] each
    /// time, emits nothing and hands the writer nothing.
    #[test]
    fn an_empty_core_keeps_the_loop_quiet() {
        let (shared, clock, _woken) = fake();
        let mut waiter = FakeWaiter::new(&shared, &clock).left(30);
        let waits = waiter.waits.clone();
        let seen = Stamped::new(&clock);
        let mut handed = Handed::default();
        run_timer_loop(&shared, &mut waiter, &seen, &mut handed, TICK);
        let waits = waits.lock().unwrap();
        assert_eq!(waits.len(), 30);
        assert_eq!(
            waits[0].1,
            Duration::ZERO,
            "the first iteration runs at once"
        );
        assert!(waits[1..].iter().all(|w| w.1 == TICK), "{waits:?}");
        assert_eq!(seen.all(), Vec::new());
        assert_eq!(handed.0, Vec::new());
    }

    /// And every other reader of an empty core: a `tm:` press of any kind
    /// does nothing, an op answers "no such timer", and a status read is the
    /// empty set with a larger `seq` each time.
    #[test]
    fn an_empty_core_answers_every_reader() {
        let (shared, _clock, _woken) = fake();
        for (id, pressed) in [
            ("tm:timer-1:toggle", true),
            ("tm:timer-1:reset", true),
            ("tm:timer-1:toggle", false),
            ("mute", true),
            ("ptm", false),
        ] {
            timer_on_hotkey(&shared, id, pressed);
        }
        assert_eq!(
            shared.apply("timer-1", TimerOp::Arm),
            Err("no such timer".to_string())
        );
        let first = shared.status();
        let second = shared.status();
        assert_eq!(first.timers, Vec::new());
        assert!(second.seq > first.seq);
    }

    /// The slot keeps only the latest set, and a wait with nothing in it
    /// ends empty.
    #[test]
    fn the_slot_keeps_the_latest_set() {
        let slot = Arc::new(TextSlot::new());
        assert_eq!(slot.take_or_wait(ms(1)), None);
        let mut sink = slot.clone();
        sink.update(set(&[("timer-1", "05:00")]));
        sink.update(set(&[("timer-1", "04:59")]));
        assert_eq!(
            slot.take_or_wait(Duration::ZERO),
            Some(set(&[("timer-1", "04:59")]))
        );
        assert_eq!(slot.take_or_wait(Duration::ZERO), None);
    }

    /// A writer that was never handed a set has nothing pending, and makes
    /// no folder: the writer makes none for someone who never adds a timer.
    #[test]
    fn a_writer_with_nothing_handed_over_waits() {
        let dir = scratch("idle");
        let mut files = TextFiles::new(dir.clone());
        let t0 = Instant::now();
        assert_eq!(files.apply(None, t0), None);
        assert_eq!(files.apply(None, t0 + REPAIR_EVERY), None);
        assert!(!dir.exists());
    }

    /// One write replaces the file by rename, leaves no temp, and brings a
    /// missing folder back.
    #[test]
    fn write_text_lands_by_rename() {
        let dir = scratch("write");
        write_text(&dir, "timer-1", "05:00").unwrap();
        write_text(&dir, "timer-1", "04:59").unwrap();
        assert_eq!(std::fs::read(dir.join("timer-1.txt")).unwrap(), b"04:59");
        assert!(!dir.join(".timer-1.tmp").exists());
        std::fs::remove_dir_all(&dir).unwrap();
        write_text(&dir, "timer-1", "1:00:00").unwrap();
        assert_eq!(std::fs::read(dir.join("timer-1.txt")).unwrap(), b"1:00:00");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn retryable_table() {
        assert!(retryable(&io::Error::from(io::ErrorKind::PermissionDenied)));
        assert!(!retryable(&io::Error::from(io::ErrorKind::NotFound)));
        #[cfg(windows)]
        {
            assert!(retryable(&io::Error::from_raw_os_error(5)));
            assert!(retryable(&io::Error::from_raw_os_error(32)));
            assert!(!retryable(&io::Error::from_raw_os_error(2)));
        }
    }

    /// Only Divora's own names are swept.
    #[test]
    fn sweepable_table() {
        let keep: HashSet<String> = ["timer-1".to_string(), "timer-7".to_string()].into();
        for name in [
            "timer-2.txt",
            "timer-99999.txt",
            ".timer-1.tmp",
            ".timer-3.tmp",
        ] {
            assert!(sweepable(name, &keep), "{name}");
        }
        for name in [
            "timer-1.txt",
            "timer-7.txt",
            "notes.txt",
            "brb.txt",
            ".x.tmp",
            "notes.md",
            "Timer-3.txt",
            "con.txt.bak",
            "timer-0.txt",
            "timer-01.txt",
            "timer-100000.txt",
            "timer-.txt",
            ".timer-1.txt",
            "timer-1.tmp",
            ".brb.tmp",
        ] {
            assert!(!sweepable(name, &keep), "{name}");
        }
    }

    // ---- v1.59.0 (track B): the timer thread -----------------------------

    /// D13, §7.5: each wait the loop asks for is `next_wake` from that
    /// pass's own reading — a timer's next text change, at most a [`TICK`]
    /// — so every emit lands exactly on a command or a boundary, once. A
    /// 2.5 s countdown armed at 237 ms changes at 737 ms and 1.737 s and
    /// fires at 2.737 s; a count-up armed at 610 ms changes each whole second
    /// after it.
    #[test]
    fn the_thread_waits_next_wake() {
        let (shared, clock, _woken) = fake();
        let (armed_down, armed_up) = (ms(237), ms(610));
        let mut waiter = FakeWaiter::new(&shared, &clock)
            .until(ms(4_000))
            .at(ms(37), |s| {
                s.sync(vec![countdown("timer-1", 2_500), countup("timer-2")]);
            })
            .at(armed_down, |s| {
                s.apply("timer-1", TimerOp::Arm).unwrap();
            })
            .at(armed_up, |s| {
                s.apply("timer-2", TimerOp::Arm).unwrap();
            });
        let waits = waiter.waits.clone();
        let events = Stamped::new(&clock);
        run_timer_loop(&shared, &mut waiter, &events, &mut Handed::default(), TICK);

        let down = [
            armed_down + ms(500),
            armed_down + ms(1_500),
            armed_down + ms(2_500),
        ];
        let up: Vec<Duration> = (1..=3).map(|k| armed_up + secs(k)).collect();
        let waits = waits.lock().unwrap();
        assert_eq!(
            waits[0],
            (Duration::ZERO, Duration::ZERO),
            "the first pass runs at once"
        );
        for &(at, asked) in &waits[1..] {
            let mut next = TICK;
            if at >= armed_down && at < down[2] {
                let boundary = down.iter().find(|&&b| b > at).unwrap();
                next = next.min(boundary.saturating_sub(at));
            }
            if at >= armed_up {
                let whole = at.saturating_sub(armed_up).as_secs() + 1;
                next = next.min((armed_up + secs(whole)).saturating_sub(at));
            }
            assert_eq!(asked, next, "the wait asked at {at:?}");
        }
        let mut expected = vec![ms(37), armed_down, armed_up];
        expected.extend(down);
        expected.extend(up);
        expected.sort();
        let stamps: Vec<Duration> = events.all().iter().map(|(at, _)| *at).collect();
        assert_eq!(stamps, expected);
    }

    /// Seam 1, core ↔ thread: a 2.5 s countdown through the real loop emits
    /// `00:03` → `00:02` → `00:01` → `00:00` once each, on its boundaries;
    /// it fires once (one log line); the writer is handed each text in order.
    #[test]
    fn a_short_countdown_through_the_thread() {
        let (shared, clock, _woken) = fake();
        let mut waiter = FakeWaiter::new(&shared, &clock)
            .until(ms(4_000))
            .at(ms(50), |s| {
                s.sync(vec![countdown("timer-1", 2_500)]);
            })
            .at(ms(100), |s| {
                s.apply("timer-1", TimerOp::Arm).unwrap();
            });
        let events = Stamped::new(&clock);
        let mut handed = Handed::default();
        let ((), log) = logged(|| run_timer_loop(&shared, &mut waiter, &events, &mut handed, TICK));

        let emitted: Vec<_> = events
            .all()
            .into_iter()
            .map(|(at, s)| {
                let t = &s.timers[0];
                (at, t.phase, t.text.clone(), t.fires)
            })
            .collect();
        let expected = [
            (ms(50), Idle, "00:03", 0),
            (ms(100), Running, "00:03", 0),
            (ms(600), Running, "00:02", 0),
            (ms(1_600), Running, "00:01", 0),
            (ms(2_600), Done, "00:00", 1),
        ]
        .map(|(at, phase, text, fires)| (at, phase, text.to_string(), fires));
        assert_eq!(emitted, expected);
        let texts: Vec<&str> = handed
            .0
            .iter()
            .map(|u| {
                assert_eq!(u.keep, ["timer-1"]);
                u.texts[0].1.as_str()
            })
            .collect();
        assert_eq!(texts, ["00:03", "00:03", "00:02", "00:01", "00:00"]);
        assert_eq!(
            lines(&log, "INFO", "a timer finished id=timer-1 fires=1"),
            1,
            "{log}"
        );
        assert_eq!(lines(&log, "WARN", ""), 0, "{log}");
    }

    /// D5 (a) through the thread: the PC sleeps an hour 10 s into a 5:00
    /// countdown and a count-up; Windows' resume wakes the thread, whose
    /// first reading sees the sleep: the countdown ran out asleep (expired,
    /// not fired), the count-up pauses where the PC went to sleep, and both
    /// are logged once.
    #[test]
    fn the_thread_expires_across_a_simulated_sleep() {
        let slept = Arc::new(AtomicBool::new(false));
        let reported = Arc::new(Mutex::new(None));
        let (shared, clock, _woken) = fake_with(Box::new(OneSleep {
            last: None,
            slept: slept.clone(),
            reported: reported.clone(),
        }));
        let hour = secs(3_600);
        let sleeper = clock.clone();
        let mut waiter = FakeWaiter::new(&shared, &clock)
            .until(hour + ms(12_000))
            .at(ms(50), |s| {
                s.sync(vec![countdown("timer-1", 300_000), countup("timer-2")]);
            })
            .at(ms(100), |s| {
                s.apply("timer-1", TimerOp::Arm).unwrap();
                s.apply("timer-2", TimerOp::Arm).unwrap();
            })
            .at(ms(10_000), move |_| {
                sleeper.set(ms(10_000) + hour);
                slept.store(true, Ordering::SeqCst);
            });
        let events = Stamped::new(&clock);
        let ((), log) = logged(|| {
            run_timer_loop(&shared, &mut waiter, &events, &mut Handed::default(), TICK);
        });

        let before = reported.lock().unwrap().expect("the sleep was reported");
        assert!(before > ms(9_800) && before <= ms(10_000), "{before:?}");
        let (at, last) = events.all().pop().unwrap();
        assert_eq!(
            at,
            ms(10_000) + hour,
            "handled at the first reading after it"
        );
        let (down, up) = (&last.timers[0], &last.timers[1]);
        assert_eq!(
            (
                down.phase,
                down.fires,
                down.ms,
                down.text.as_str(),
                down.anchor_ms
            ),
            (Expired, 0, 0, "00:00", None)
        );
        let counted = u64::try_from(before.saturating_sub(ms(100)).as_millis()).unwrap();
        assert_eq!((up.phase, up.ms, up.anchor_ms), (Paused, counted, None));
        assert_eq!(up.text, timer_text(TimerKind::Countup, counted));
        assert_eq!(
            lines(
                &log,
                "INFO",
                "ran out while the PC slept; not finished id=timer-1"
            ),
            1,
            "{log}"
        );
        assert_eq!(
            lines(&log, "INFO", "paused where the PC slept id=timer-2"),
            1,
            "{log}"
        );
        assert_eq!(lines(&log, "", "a timer finished"), 0, "{log}");
        // The core's note of the clock that decided, logged by the thread.
        assert_eq!(lines(&log, "", "the clock counted it"), 1, "{log}");
        assert_eq!(
            lines(&log, "INFO", "running=2 expired=1 paused=1"),
            1,
            "{log}"
        );
    }

    /// §8.1, D34: the core never logs under its lock; the thread logs its
    /// notes after letting go. The PC sleeps an hour with a count-up
    /// running, and the thread is the first reader after it: while its
    /// "the PC slept" line is stuck in the log file's write, a command and a
    /// `tm:` press from another thread each return within a second.
    #[test]
    fn a_slow_log_write_never_holds_the_core() {
        let (shared, clock, woken) = fake_with(Box::new(Backstop(None)));
        shared.sync(vec![countup("timer-1")]);
        shared.apply("timer-1", TimerOp::Arm).unwrap();
        let (entered_tx, entered) = mpsc::channel();
        let (gate, go) = Gate::new("log", &entered_tx);
        let gate = Arc::new(Mutex::new(Some(gate)));
        let subscriber = tracing_subscriber::fmt()
            .with_writer(move || GatedLog {
                at: "the clock counted it",
                gate: gate.clone(),
            })
            .with_ansi(false)
            .without_time()
            .with_max_level(tracing::Level::INFO)
            .finish();
        std::thread::scope(|s| {
            let mut waiter = Until::new(woken, ms(3_000));
            let looped = &shared;
            s.spawn(move || {
                tracing::subscriber::with_default(subscriber, || {
                    let (events, mut sink) = (Heard::default(), Handed::default());
                    run_timer_loop(looped, &mut waiter, &events, &mut sink, TICK);
                });
            });
            clock.set(secs(3_600));
            let at = entered
                .recv_timeout(secs(2))
                .expect("the thread logged the sleep");
            let core = &shared;
            let readers = (
                returns_within(s, move || core.apply("timer-1", TimerOp::Toggle)),
                returns_within(s, move || timer_on_hotkey(core, "tm:timer-1:toggle", true)),
            );
            go.send(()).unwrap();
            // (true, true): both returned while the line was being written.
            assert_eq!((at, readers), ("log", (true, true)));
        });
    }

    /// D13: a command wakes the thread at once. With nothing running and a
    /// 5 s cap, a loop that ignored the wake would emit only at 5 s.
    #[test]
    fn a_command_wakes_the_thread() {
        let (shared, woken) = real();
        let heard = Heard::default();
        std::thread::scope(|s| {
            let mut waiter = Until::new(woken, ms(2_500));
            let (looped, events) = (&shared, heard.clone());
            s.spawn(move || {
                run_timer_loop(
                    looped,
                    &mut waiter,
                    &events,
                    &mut Handed::default(),
                    secs(5),
                );
            });
            // Its first pass is done: it waits out the 5 s cap.
            std::thread::sleep(ms(100));
            let answer = shared.sync(vec![countdown("timer-1", 300_000)]);
            let newer = |s: &TimerSnapshot, phase| {
                s.seq > answer.seq && s.timers.first().is_some_and(|t| t.phase == phase)
            };
            assert!(heard.within(secs(1), |s| newer(s, Idle)).is_some(), "sync");
            std::thread::sleep(ms(100));
            let answer = shared.apply("timer-1", TimerOp::Arm).unwrap();
            let newer = |s: &TimerSnapshot, phase| {
                s.seq > answer.seq && s.timers.first().is_some_and(|t| t.phase == phase)
            };
            assert!(
                heard.within(secs(1), |s| newer(s, Running)).is_some(),
                "Arm"
            );
            let answer = shared.apply("timer-1", TimerOp::Pause).unwrap();
            let newer = |s: &TimerSnapshot| {
                s.seq > answer.seq && s.timers.first().is_some_and(|t| t.phase == Paused)
            };
            assert!(heard.within(secs(1), newer).is_some(), "Pause");
        });
    }

    /// D13, D16: the thread emits, then hands texts over, only after it lets
    /// go of the core's lock. While its emit blocks, and then while its
    /// hand-off blocks, a command and a status read from another thread each
    /// answer within a second.
    #[test]
    fn commands_never_wait_for_the_sink() {
        let (shared, woken) = real();
        let (entered_tx, entered) = mpsc::channel();
        let (gate, emit_go) = Gate::new("emit", &entered_tx);
        let events = GatedEvents(Mutex::new(Some(gate)));
        let (gate, hand_go) = Gate::new("hand-off", &entered_tx);
        let mut sink = GatedSink(Some(gate));
        std::thread::scope(|s| {
            let mut waiter = Until::new(woken, ms(3_000));
            let (looped, events) = (&shared, &events);
            s.spawn(move || run_timer_loop(looped, &mut waiter, events, &mut sink, TICK));
            shared.sync(vec![countdown("timer-1", 300_000)]);
            let mut blocked = Vec::new();
            for op in [TimerOp::Arm, TimerOp::Pause] {
                let at = entered.recv_timeout(secs(2)).expect("the thread blocked");
                let core = &shared;
                let readers = (
                    returns_within(s, move || core.apply("timer-1", op)),
                    returns_within(s, || shared.status()),
                );
                let go = if at == "emit" { &emit_go } else { &hand_go };
                go.send(()).unwrap();
                blocked.push((at, readers));
            }
            // (true, true): both readers answered while the thread was held.
            assert_eq!(
                blocked,
                [("emit", (true, true)), ("hand-off", (true, true))]
            );
        });
    }

    /// Takes its first set, then sits on it for 3 s, as a stuck disk would.
    struct Stalls(Option<Sender<()>>);

    impl TextWriter for Stalls {
        fn apply(&mut self, u: Option<TextUpdate>, _now: Instant) -> Option<Duration> {
            if u.is_some() {
                if let Some(taken) = self.0.take() {
                    let _ = taken.send(());
                    std::thread::sleep(secs(3));
                }
            }
            None
        }
    }

    /// D13, D21: the disk is the writer thread's, never the timer thread's.
    /// With the writer stuck 3 s on its first set, a 1.5 s countdown still
    /// emits `00:01` and fires on time (each within a second of its
    /// boundary), once.
    #[test]
    fn a_slow_writer_never_delays_a_countdown() {
        let (shared, woken) = real();
        let slot = Arc::new(TextSlot::new());
        let (taken, writing) = mpsc::channel();
        spawn_text_writer(&slot, Stalls(Some(taken)));
        let heard = Heard::default();
        std::thread::scope(|s| {
            let mut waiter = Until::new(woken, ms(2_800));
            let (looped, events, mut sink) = (&shared, heard.clone(), slot.clone());
            s.spawn(move || run_timer_loop(looped, &mut waiter, &events, &mut sink, TICK));
            shared.sync(vec![countdown("timer-1", 1_500)]);
            writing
                .recv_timeout(secs(1))
                .expect("the writer took the sync's texts");
            let armed = Instant::now();
            shared.apply("timer-1", TimerOp::Arm).unwrap();
            let first = |s: &TimerSnapshot| s.timers.first().cloned();
            let one = heard
                .within(secs(2), |s| first(s).is_some_and(|t| t.text == "00:01"))
                .expect("00:01");
            let done = heard
                .within(secs(3), |s| first(s).is_some_and(|t| t.phase == Done))
                .expect("done");
            let (one_in, done_in) = (one.0 - armed, done.0 - armed);
            assert!(
                one_in >= ms(500) && one_in <= ms(1_500),
                "00:01 after {one_in:?}"
            );
            assert!(
                done_in >= ms(1_500) && done_in <= ms(2_500),
                "done after {done_in:?}"
            );
            assert_eq!(
                (done.1.timers[0].fires, done.1.timers[0].text.as_str()),
                (1, "00:00")
            );
        });
    }

    /// D19, seam 3: the handler's step through the real core. A press of
    /// `tm:timer-1:toggle` starts it and wakes the thread; another pauses
    /// it where it is; a release does nothing (and wakes nothing); reset
    /// makes it idle; an id the core doesn't run, other keys and (v1.60.0)
    /// `finish` on an idle timer, which has nothing to end, do nothing.
    #[test]
    fn the_timer_hotkey_toggles_through_the_core() {
        let (shared, clock, woken) = fake();
        shared.sync(vec![countdown("timer-1", 60_000)]);
        let drain = || while woken.try_recv().is_ok() {};
        let state = || {
            let t = shared.status().timers[0].clone();
            (t.phase, t.ms)
        };
        drain();
        timer_on_hotkey(&shared, "tm:timer-1:toggle", true);
        assert!(woken.try_recv().is_ok(), "a press wakes the thread");
        assert_eq!(state(), (Running, 60_000));
        clock.set(secs(2));
        timer_on_hotkey(&shared, "tm:timer-1:toggle", true);
        assert_eq!(state(), (Paused, 58_000));
        drain();
        timer_on_hotkey(&shared, "tm:timer-1:toggle", false);
        assert!(woken.try_recv().is_err(), "a release does nothing");
        assert_eq!(state(), (Paused, 58_000));
        timer_on_hotkey(&shared, "tm:timer-1:reset", true);
        assert_eq!(state(), (Idle, 60_000));
        for id in [
            "tm:timer-9:toggle",
            "tm:timer-1:finish",
            "mute",
            "ptm",
            "sb:timer-1",
        ] {
            timer_on_hotkey(&shared, id, true);
            assert_eq!(state(), (Idle, 60_000), "{id}");
        }
    }

    // ---- v1.59.0 (track B): the text files -------------------------------

    /// K1, seam 2: a rename onto a file OBS is reading fails with 5
    /// (`PermissionDenied`, measured), which is retryable, as a held temp's
    /// 32 is; `NotFound` isn't. The write stays pending without a warning (a
    /// reader's hold isn't an error), lands within a second of the reader
    /// letting go, and is counted in the collision line S2's H2 looks for.
    #[cfg(windows)]
    #[test]
    fn a_write_while_obs_reads_is_retried() {
        let dir = scratch("obs");
        let mut files = TextFiles::new(dir.clone());
        let ((), log) = logged(|| {
            assert_eq!(
                files.apply(Some(set(&[("timer-1", "05:00")])), Instant::now()),
                None
            );
            let reader = obs_open(&dir.join("timer-1.txt")).unwrap();
            let pending = files.apply(Some(set(&[("timer-1", "04:59")])), Instant::now());
            assert_eq!(pending, Some(RETRY_WAIT));
            assert_eq!(files.tries(), (2, 1), "the writer's first failed attempt");
            assert_eq!(read(&dir, "timer-1.txt").as_deref(), Some("05:00"));
            drop(reader);
            let released = Instant::now();
            while read(&dir, "timer-1.txt").as_deref() != Some("04:59")
                && released.elapsed() < secs(1)
            {
                let wait = files.apply(None, Instant::now());
                std::thread::sleep(wait.unwrap_or(RETRY_WAIT));
            }
        });
        assert_eq!(
            read(&dir, "timer-1.txt").as_deref(),
            Some("04:59"),
            "within 1 s"
        );
        assert_eq!(files.apply(None, Instant::now()), None, "nothing pending");
        assert_eq!(lines(&log, "WARN", ""), 0, "{log}");
        assert_eq!(
            lines(&log, "INFO", "timer files: 1 writes waited for a reader"),
            1,
            "{log}"
        );
        assert!(retryable(&io::Error::from_raw_os_error(5)));
        assert!(retryable(&io::Error::from_raw_os_error(32)));
        assert!(!retryable(&io::Error::from(io::ErrorKind::NotFound)));
        std::fs::remove_dir_all(&dir).ok();
    }

    /// D21: what was written is remembered only once the rename lands, so a
    /// paused timer's one new text, held up 500 ms by a reader with no set
    /// after it, still reaches the disk when the reader lets go.
    #[cfg(windows)]
    #[test]
    fn an_unchanged_text_lands_after_a_long_hold() {
        let dir = scratch("long-hold");
        let mut files = TextFiles::new(dir.clone());
        let t0 = Instant::now();
        assert_eq!(files.apply(Some(set(&[("timer-1", "03:00")])), t0), None);
        let reader = obs_open(&dir.join("timer-1.txt")).unwrap();
        let mut t = t0 + ms(1);
        assert_eq!(
            files.apply(Some(set(&[("timer-1", "02:59")])), t),
            Some(RETRY_WAIT)
        );
        while t < t0 + ms(500) {
            t += RETRY_WAIT;
            assert_eq!(
                files.apply(None, t),
                Some(RETRY_WAIT),
                "still pending at {:?}",
                t - t0
            );
        }
        assert_eq!(read(&dir, "timer-1.txt").as_deref(), Some("03:00"));
        drop(reader);
        assert_eq!(files.apply(None, t + RETRY_WAIT), None);
        assert_eq!(read(&dir, "timer-1.txt").as_deref(), Some("02:59"));
        std::fs::remove_dir_all(&dir).ok();
    }

    /// D21: a deleted folder comes back at the next write, and, with no new
    /// text (a paused timer), at the next repair pass.
    #[test]
    fn the_folder_comes_back() {
        let dir = scratch("folder");
        let mut files = TextFiles::new(dir.clone());
        let t0 = Instant::now();
        assert_eq!(files.apply(Some(set(&[("timer-1", "05:00")])), t0), None);
        assert_eq!(read(&dir, "timer-1.txt").as_deref(), Some("05:00"));
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(
            files.apply(Some(set(&[("timer-1", "04:59")])), t0 + secs(1)),
            None
        );
        assert_eq!(read(&dir, "timer-1.txt").as_deref(), Some("04:59"));
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(files.apply(None, t0 + REPAIR_EVERY), None);
        assert_eq!(read(&dir, "timer-1.txt").as_deref(), Some("04:59"));
        std::fs::remove_dir_all(&dir).ok();
    }

    /// D21: a paused timer's file deleted by hand is back at the next repair
    /// pass, [`REPAIR_EVERY`] after the last.
    #[test]
    fn a_file_deleted_behind_our_back_is_rewritten() {
        let dir = scratch("behind");
        let mut files = TextFiles::new(dir.clone());
        let t0 = Instant::now();
        assert_eq!(files.apply(Some(set(&[("timer-1", "02:00")])), t0), None);
        std::fs::remove_file(dir.join("timer-1.txt")).unwrap();
        assert_eq!(files.apply(None, t0 + ms(4_990)), None);
        assert_eq!(
            read(&dir, "timer-1.txt"),
            None,
            "only the repair pass looks"
        );
        assert_eq!(files.apply(None, t0 + REPAIR_EVERY), None);
        assert_eq!(read(&dir, "timer-1.txt").as_deref(), Some("02:00"));
        std::fs::remove_dir_all(&dir).ok();
    }

    /// D21: a timer deleted while OBS holds its file, and added again before
    /// OBS lets go, keeps its file, with the new timer's text.
    #[cfg(windows)]
    #[test]
    fn a_readded_id_keeps_its_new_file() {
        let dir = scratch("readded");
        let mut files = TextFiles::new(dir.clone());
        let t0 = Instant::now();
        assert_eq!(files.apply(Some(set(&[("timer-1", "03:00")])), t0), None);
        let reader = obs_open(&dir.join("timer-1.txt")).unwrap();
        let deleted = files.apply(Some(TextUpdate::default()), t0 + ms(10));
        assert_eq!(
            deleted,
            Some(RETRY_WAIT),
            "the removal waits for the reader"
        );
        assert_eq!(read(&dir, "timer-1.txt").as_deref(), Some("03:00"));
        let added = files.apply(Some(set(&[("timer-1", "05:00")])), t0 + ms(20));
        assert_eq!(added, Some(RETRY_WAIT), "the write waits for the reader");
        drop(reader);
        for k in 3..6 {
            assert_eq!(files.apply(None, t0 + ms(10 * k)), None);
            assert_eq!(read(&dir, "timer-1.txt").as_deref(), Some("05:00"));
        }
        std::fs::remove_dir_all(&dir).ok();
    }

    /// K21, D21: a read-only file fails every rename (5). It is tried each
    /// [`RETRY_WAIT`] for [`BACKOFF_AFTER`], then once a second, with one
    /// warning; once it can be written again it lands, with one line saying
    /// so.
    #[cfg(windows)]
    #[test]
    fn a_read_only_file_backs_off_with_one_warning() {
        let dir = scratch("read-only");
        let file = dir.join("timer-1.txt");
        let mut files = TextFiles::new(dir.clone());
        let t0 = Instant::now();
        let (counts, log) = logged(|| {
            assert_eq!(files.apply(Some(set(&[("timer-1", "05:00")])), t0), None);
            set_read_only(&file, true);
            let mut t = t0;
            let mut wait = files.apply(Some(set(&[("timer-1", "04:59")])), t);
            let mut by_one_second = None;
            while t < t0 + secs(5) {
                t += wait.expect("still pending");
                if by_one_second.is_none() && t >= t0 + BACKOFF_AFTER {
                    by_one_second = Some(files.tries().0);
                }
                wait = files.apply(None, t);
            }
            let failing = files.tries();
            set_read_only(&file, false);
            let landed = files.apply(None, t + wait.unwrap());
            (by_one_second.unwrap(), failing, landed)
        });
        let (by_one_second, (tried, failed), landed) = counts;
        // `05:00`, then `04:59` each 10 ms for the first second …
        assert!((96..=101).contains(&(by_one_second - 1)), "{by_one_second}");
        // … then once a second.
        assert!(
            (3..=5).contains(&(tried - by_one_second)),
            "{tried} {by_one_second}"
        );
        assert_eq!(failed, tried - 1);
        assert_eq!(landed, None);
        assert_eq!(read(&dir, "timer-1.txt").as_deref(), Some("04:59"));
        assert_eq!(lines(&log, "WARN", "timer-1"), 1, "{log}");
        assert_eq!(lines(&log, "WARN", "os error 5"), 1, "{log}");
        assert_eq!(
            lines(&log, "INFO", "timer files: written again id=timer-1"),
            1,
            "{log}"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    /// D21, WF10: a reader opening, reading and closing the file as OBS does
    /// (in a loop) while 2 000 writes alternate `59:59` and `1:00:00` only
    /// ever reads one of them: never an empty or partial file. An open can
    /// fail in a rename's microseconds (raw 32), but never with `NotFound`: the
    /// file is never removed before the rename.
    #[cfg(windows)]
    #[test]
    fn a_reader_never_sees_a_partial_file() {
        use std::collections::BTreeMap;
        use std::io::Read;
        let dir = scratch("reader");
        write_text(&dir, "timer-1", "59:59").unwrap();
        let file = dir.join("timer-1.txt");
        let stop = AtomicBool::new(false);
        let (reads, odd, failed) = std::thread::scope(|s| {
            let reader = s.spawn(|| {
                let (mut reads, mut odd) = (0_u64, Vec::new());
                let mut failed: BTreeMap<Option<i32>, u64> = BTreeMap::new();
                while !stop.load(Ordering::Relaxed) {
                    match obs_open(&file) {
                        Ok(mut f) => {
                            let mut text = String::new();
                            f.read_to_string(&mut text).unwrap();
                            reads += 1;
                            if text != "59:59" && text != "1:00:00" {
                                odd.push(text);
                            }
                        }
                        Err(e) => *failed.entry(e.raw_os_error()).or_default() += 1,
                    }
                }
                (reads, odd, failed)
            });
            for i in 0..2_000 {
                let text = if i % 2 == 0 { "1:00:00" } else { "59:59" };
                loop {
                    match write_text(&dir, "timer-1", text) {
                        Ok(()) => break,
                        Err(e) if retryable(&e) => std::thread::yield_now(),
                        Err(e) => panic!("write {i}: {e}"),
                    }
                }
            }
            stop.store(true, Ordering::Relaxed);
            reader.join().unwrap()
        });
        assert!(
            odd.is_empty(),
            "read {} odd texts, e.g. {:?}",
            odd.len(),
            odd.first()
        );
        assert!(reads >= 100, "only {reads} reads");
        assert!(
            failed.keys().all(|&code| code == Some(32)),
            "failed opens: {failed:?}"
        );
        assert_eq!(read(&dir, "timer-1.txt").as_deref(), Some("59:59"));
        std::fs::remove_dir_all(&dir).ok();
    }

    /// D21: before the first sync the set of timers is unknown, so nothing
    /// is swept: the thread runs 300 ms unsynced and hands the writer
    /// nothing, and the writer's own passes (a repair pass included) touch
    /// nothing. The first sync's texts then sweep.
    #[test]
    fn nothing_is_swept_before_the_first_sync() {
        let dir = scratch("unsynced");
        let names = ["timer-1.txt", "timer-2.txt", ".timer-3.tmp", "notes.txt"];
        for name in names {
            put(&dir, name, "old");
        }
        let (shared, clock, _woken) = fake();
        let mut waiter = FakeWaiter::new(&shared, &clock).until(ms(300));
        let mut handed = Handed::default();
        run_timer_loop(
            &shared,
            &mut waiter,
            &Stamped::new(&clock),
            &mut handed,
            TICK,
        );
        assert_eq!(
            handed.0,
            Vec::new(),
            "the thread handed texts over unsynced"
        );
        let mut files = TextFiles::new(dir.clone());
        let t0 = Instant::now();
        for k in 0..=60 {
            assert_eq!(files.apply(None, t0 + ms(100 * k)), None);
        }
        for name in names {
            assert_eq!(read(&dir, name).as_deref(), Some("old"), "{name}");
        }
        shared.sync(vec![countdown("timer-1", 300_000)]);
        let first = one_pass(&shared, &clock).expect("the sync's texts");
        assert_eq!(files.apply(Some(first), t0 + secs(7)), None);
        assert_eq!(read(&dir, "timer-1.txt").as_deref(), Some("05:00"));
        assert_eq!(read(&dir, "timer-2.txt"), None);
        assert_eq!(read(&dir, ".timer-3.tmp"), None);
        assert_eq!(read(&dir, "notes.txt").as_deref(), Some("old"));
        std::fs::remove_dir_all(&dir).ok();
    }

    /// D9, D21, D36: the one sweep removes only the names Divora makes — a
    /// `timer-<n>.txt` no timer has and any `.timer-<n>.tmp` — and keeps a
    /// kept timer's file (a kind this build doesn't run) and every file of
    /// the user's own.
    #[test]
    fn the_sweep_removes_only_divoras_names() {
        let dir = scratch("sweep");
        let kept = [
            "timer-7.txt",
            "notes.txt",
            "brb.txt",
            ".x.tmp",
            "notes.md",
            "Timer-3.txt",
            "con.txt.bak",
        ];
        for name in kept
            .iter()
            .chain(&["timer-1.txt", "timer-2.txt", ".timer-1.tmp"])
        {
            put(&dir, name, "old");
        }
        let (shared, clock, _woken) = fake();
        shared.sync(vec![
            countdown("timer-1", 300_000),
            json!({"id": "timer-7", "name": "Laps", "kind": "interval"}),
        ]);
        let first = one_pass(&shared, &clock).expect("the sync's texts");
        assert_eq!(first.keep, ["timer-1", "timer-7"]);
        let mut files = TextFiles::new(dir.clone());
        let ((), log) = logged(|| assert_eq!(files.apply(Some(first), Instant::now()), None));
        let mut left: Vec<String> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        left.sort();
        let mut expected: Vec<String> = kept.iter().map(ToString::to_string).collect();
        expected.push("timer-1.txt".to_string());
        expected.sort();
        assert_eq!(left, expected);
        assert_eq!(read(&dir, "timer-1.txt").as_deref(), Some("05:00"));
        for name in kept {
            assert_eq!(read(&dir, name).as_deref(), Some("old"), "{name}");
        }
        assert_eq!(lines(&log, "INFO", "old=1 temps=1"), 1, "{log}");
        std::fs::remove_dir_all(&dir).ok();
    }

    /// A sync, then the pass of the real loop that hands its texts over,
    /// then the writer's pass at `t`.
    fn sync_and_write(
        shared: &Arc<TimerShared>,
        clock: &Arc<FakeClock>,
        files: &mut TextFiles,
        timers: Vec<Value>,
        t: Instant,
    ) -> Option<Duration> {
        shared.sync(timers);
        files.apply(one_pass(shared, clock), t)
    }

    /// D21: a timer deleted by a sync loses its file; one OBS is holding
    /// (no delete sharing) stays while held and goes once it is let go.
    #[test]
    fn a_deleted_timers_file_goes() {
        let dir = scratch("deleted-timer");
        let (shared, clock, _woken) = fake();
        let mut files = TextFiles::new(dir.clone());
        let t0 = Instant::now();
        let both = || vec![countdown("timer-1", 300_000), countup("timer-2")];
        assert_eq!(
            sync_and_write(&shared, &clock, &mut files, both(), t0),
            None
        );
        assert_eq!(read(&dir, "timer-1.txt").as_deref(), Some("05:00"));
        let one = || vec![countup("timer-2")];
        assert_eq!(
            sync_and_write(&shared, &clock, &mut files, one(), t0 + ms(10)),
            None
        );
        assert_eq!(read(&dir, "timer-1.txt"), None);
        assert_eq!(read(&dir, "timer-2.txt").as_deref(), Some("00:00"));
        #[cfg(windows)]
        {
            assert_eq!(
                sync_and_write(&shared, &clock, &mut files, both(), t0 + ms(20)),
                None
            );
            let reader = obs_open(&dir.join("timer-1.txt")).unwrap();
            let held = sync_and_write(&shared, &clock, &mut files, one(), t0 + ms(30));
            assert_eq!(held, Some(RETRY_WAIT));
            assert_eq!(files.apply(None, t0 + ms(40)), Some(RETRY_WAIT));
            assert_eq!(
                read(&dir, "timer-1.txt").as_deref(),
                Some("05:00"),
                "kept while held"
            );
            drop(reader);
            assert_eq!(files.apply(None, t0 + ms(50)), None);
            assert_eq!(read(&dir, "timer-1.txt"), None, "removed once released");
        }
        std::fs::remove_dir_all(&dir).ok();
    }

    /// D21, D36: a kept timer's file — a kind this build doesn't run, its id
    /// `timer-<n>` or not — stays while a sync keeps it, and goes when a
    /// sync deletes it, as a run timer's does.
    #[test]
    fn a_deleted_kept_timers_file_goes() {
        let dir = scratch("deleted-kept");
        put(&dir, "timer-7.txt", "12:34");
        put(&dir, "brb.txt", "1:02:03");
        let (shared, clock, _woken) = fake();
        let mut files = TextFiles::new(dir.clone());
        let t0 = Instant::now();
        let laps = |id: &str| json!({"id": id, "name": "Laps", "kind": "interval"});
        let all = vec![countdown("timer-1", 300_000), laps("timer-7"), laps("brb")];
        assert_eq!(sync_and_write(&shared, &clock, &mut files, all, t0), None);
        assert_eq!(read(&dir, "timer-7.txt").as_deref(), Some("12:34"));
        assert_eq!(read(&dir, "brb.txt").as_deref(), Some("1:02:03"));
        let one = vec![countdown("timer-1", 300_000)];
        assert_eq!(
            sync_and_write(&shared, &clock, &mut files, one, t0 + ms(10)),
            None
        );
        assert_eq!(read(&dir, "timer-7.txt"), None);
        assert_eq!(read(&dir, "brb.txt"), None);
        assert_eq!(read(&dir, "timer-1.txt").as_deref(), Some("05:00"));
        std::fs::remove_dir_all(&dir).ok();
    }

    /// D35, §7.14 through `sync_timers`'s own reader: one sync of every kind
    /// of bad entry beside a good one answers the whole set — the good timer
    /// running from its stored run, each bad field fallen back, the unknown
    /// kind kept, the unreadable ones skipped (not the whole sync). Then
    /// entries past the 64th aren't read: a countdown 65th, after 64 kept
    /// ones, neither runs nor keeps a file, as the page's `timerRoles`
    /// shows it (`src/data/timers.ts`).
    #[test]
    fn sync_skips_bad_entries_and_stops_at_64() {
        let (shared, clock, _woken) = fake();
        let wall0 = clock.wall_ms();
        let sent = vec![
            json!({"id": 5, "kind": "countdown"}),
            json!({"id": "timer-2", "name": null, "kind": "countdown", "durationMs": 60_000}),
            json!({"id": "timer-3", "name": "Below", "kind": "countdown", "durationMs": -1}),
            json!({"id": "timer-4", "name": "Half", "kind": "countdown", "durationMs": 1.5}),
            json!({"id": "timer-5", "name": "Text", "kind": "countdown", "durationMs": "300000"}),
            json!({"id": "timer-6", "name": "Empty", "kind": "countdown", "durationMs": 60_000,
                   "resume": {}}),
            json!({"id": "timer-7", "name": "Bad", "kind": "countdown", "durationMs": 60_000,
                   "resume": {"phase": "running", "anchorMs": "x"}}),
            json!({"id": "timer-8", "name": "Later", "kind": "interval"}),
            json!({"id": "timer-9", "name": "No kind"}),
            json!({"id": "timer-1", "name": "BRB", "kind": "countdown", "durationMs": 300_000,
                   "resume": {"phase": "running", "totalMs": 300_000,
                              "anchorMs": wall0 + 120_000}}),
        ];
        let shown: Vec<(String, String, TimerPhase, String)> = shared
            .sync(sent)
            .timers
            .into_iter()
            .map(|t| (t.id, t.name, t.phase, t.text))
            .collect();
        let row = |id: &str, name: &str, phase, text: &str| {
            (id.to_string(), name.to_string(), phase, text.to_string())
        };
        assert_eq!(
            shown,
            vec![
                row("timer-2", "Countdown", Idle, "01:00"),
                row("timer-3", "Below", Idle, "00:01"),
                row("timer-4", "Half", Idle, "00:01"),
                row("timer-5", "Text", Idle, "05:00"),
                row("timer-6", "Empty", Idle, "01:00"),
                row("timer-7", "Bad", Idle, "01:00"),
                row("timer-1", "BRB", Running, "02:00"),
            ]
        );
        let keep = one_pass(&shared, &clock).expect("the sync's texts").keep;
        assert!(keep.contains(&"timer-8".to_string()) && !keep.contains(&"timer-9".to_string()));

        let mut many: Vec<Value> = (2..=65)
            .map(|n| json!({"id": format!("timer-{n}"), "kind": "interval"}))
            .collect();
        many.push(countdown("timer-1", 300_000));
        let answer = shared.sync(many);
        assert_eq!(answer.timers, Vec::new(), "the 65th entry ran");
        let keep = one_pass(&shared, &clock).expect("the sync's texts").keep;
        assert_eq!(keep.len(), MAX_SYNC_ENTRIES);
        assert!(!keep.contains(&"timer-1".to_string()));
    }

    /// Records each pass (whether it brought a set) and asks for the waits
    /// in `waits`; sits 500 ms on every set after the first.
    struct Asks {
        passes: Sender<bool>,
        waits: VecDeque<Duration>,
        sets: u32,
    }

    impl TextWriter for Asks {
        fn apply(&mut self, u: Option<TextUpdate>, _now: Instant) -> Option<Duration> {
            let _ = self.passes.send(u.is_some());
            if u.is_some() {
                self.sets += 1;
                if self.sets > 1 {
                    std::thread::sleep(ms(500));
                }
            }
            self.waits.pop_front()
        }
    }

    /// The writer thread comes back as soon as its writer asks (a pending
    /// write), otherwise waits for a set (or [`REPAIR_EVERY`]); and it ends
    /// once nothing else holds the slot (D14), dropping its writer.
    #[test]
    fn the_writer_thread_waits_as_asked() {
        let slot = Arc::new(TextSlot::new());
        let (tell, passed) = mpsc::channel();
        let asks = Asks {
            passes: tell,
            waits: [RETRY_WAIT, RETRY_WAIT].into(),
            sets: 0,
        };
        spawn_text_writer(&slot, asks);
        slot.put(TextUpdate::default());
        let first: Vec<_> = (0..3).map(|_| passed.recv_timeout(secs(1))).collect();
        assert_eq!(first, [Ok(true), Ok(false), Ok(false)]);
        assert_eq!(passed.recv_timeout(ms(300)), Err(RecvTimeoutError::Timeout));
        slot.put(TextUpdate::default());
        drop(slot);
        assert_eq!(passed.recv_timeout(secs(1)), Ok(true));
        // At once when the drop lands inside that set's 500 ms. If the test
        // thread was held up longer, the writer is already in its
        // REPAIR_EVERY wait holding the slot: that wait ends in one empty
        // pass, and then the thread ends.
        let mut end = passed.recv_timeout(REPAIR_EVERY + secs(2));
        if end == Ok(false) {
            end = passed.recv_timeout(secs(1));
        }
        assert_eq!(end, Err(RecvTimeoutError::Disconnected));
    }
}
