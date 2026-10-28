//! v1.59.0: the shell around Hourglass's [`TimerCore`]: its clock and sleep
//! check, the `divora-timers` thread that wakes at each running timer's next
//! second boundary (or every [`TICK`](crate::timers::TICK)) and emits
//! `timer-state`, the `tm:` hotkey step, and the `divora-timer-files` thread
//! that puts each timer's text in `timers/<id>.txt` for OBS.
//!
//! Every reader — the thread, a command, a `tm:` press — takes the core's
//! lock, reads the clock once, asks the one [`SleepCheck`] about that
//! reading, then acts (D5). The lock is a leaf: nothing else is locked while
//! it is held, and nothing slow happens under it. Emits and the hand-off to
//! the writer happen after it is released, and only the writer touches the
//! disk, so no disk wait can delay a deadline (D13, D21).
//!
//! Neither thread is joined or stopped: Divora has no exit hook, and both
//! run for the process (D14).

use std::collections::HashSet;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, SyncSender};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::Value;

use crate::timers::{
    entry_from_wire, hotkey_timer_request, TextUpdate, TimerCore, TimerOp, TimerSnapshot,
    Transition, LATE_LOG, MAX_SYNC_ENTRIES,
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
/// every window.
pub trait TimerEvents: Send + 'static {
    /// Never blocks. (Not `state`: `tauri::Manager::state` on the same
    /// `AppHandle` would then be ambiguous wherever this trait is in scope.)
    fn timer_state(&self, s: &TimerSnapshot);
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

/// D19: the shortcut handler's timer step. A press of `tm:<id>:toggle` or
/// `tm:<id>:reset` acts on that timer; a release, any other id, or an id the
/// core doesn't run does nothing. Runs inside `WM_HOTKEY` on the main
/// thread, so it only takes the core's lock for microseconds: it never
/// registers, unregisters, writes or emits (the timer thread emits).
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
/// that same reading, and drain the transitions; then, unlocked, log the
/// transitions, emit, and hand the texts on. Returns when the waiter reports
/// [`Woke::Closed`]. Tests run it on their own thread with fakes.
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
        let (snapshot, update, transitions) = shared.with_core(|core, now, wall_ms, slept_from| {
            core.tick(now, wall_ms, slept_from);
            let snapshot = core.take_dirty().then(|| core.snapshot(now));
            let update = (snapshot.is_some() && core.synced()).then(|| core.text_update());
            wake_at = now.saturating_add(core.next_wake(now, cap));
            (snapshot, update, core.take_transitions())
        });
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
        Transition::Fired { id, fires, late_ms } => {
            if Duration::from_millis(*late_ms) > LATE_LOG {
                tracing::warn!(id = %id, fires, late_ms, "a timer finished late");
            } else {
                tracing::info!(id = %id, fires, late_ms, "a timer finished");
            }
        }
        Transition::Expired { id } => {
            tracing::info!(id = %id, "a timer ran out while the PC slept; not finished");
        }
        Transition::SleepPaused { id } => {
            tracing::info!(id = %id, "a count-up paused where the PC slept");
        }
        Transition::Seeded { id, phase } => {
            tracing::info!(id = %id, ?phase, "a timer picked up its stored run");
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

/// A pending write's next try: it ends at the next system timer tick.
pub const RETRY_WAIT: Duration = Duration::from_millis(10);

/// A file failing this long is tried once a second (or on a new text).
pub const BACKOFF_AFTER: Duration = Duration::from_secs(1);

/// How often the writer checks that the files it wrote still exist.
pub const REPAIR_EVERY: Duration = Duration::from_secs(5);

/// The writer thread's work, one pass at a time (§7.13).
pub trait TextWriter: Send {
    /// One pass: `u` is a new set from the slot, or `None` when the wait ran
    /// out. `now` is injected so tests drive time. Returns how long to wait
    /// before the next pass while something is pending (`None`: wait for a
    /// new set, or [`REPAIR_EVERY`]).
    fn apply(&mut self, u: Option<TextUpdate>, now: Instant) -> Option<Duration>;
}

/// The `.txt` files (D21): written by temp file and rename, retried while a
/// reader holds one, removed when their timer goes, swept once after the
/// first sync, repaired every [`REPAIR_EVERY`].
pub struct TextFiles {
    #[allow(dead_code)] // S1: track B's passes use it
    dir: PathBuf,
}

impl TextFiles {
    pub fn new(dir: PathBuf) -> Self {
        Self { dir }
    }
}

impl TextWriter for TextFiles {
    #[allow(unused_variables)] // S1: track B's body uses them
    fn apply(&mut self, u: Option<TextUpdate>, now: Instant) -> Option<Duration> {
        // S1: no set arrives before the core's first sync (track A), so
        // nothing has been written: nothing pending, nothing to repair.
        let _first = u?;
        todo!("track B: §7.13")
    }
}

/// Starts `divora-timer-files`: it takes the latest texts from `slot` and
/// hands them to `writer`, waiting as long as the writer asks.
pub fn spawn_text_writer(slot: Arc<TextSlot>, mut writer: impl TextWriter + 'static) {
    let spawned = std::thread::Builder::new()
        .name("divora-timer-files".into())
        .spawn(move || {
            let mut retry: Option<Duration> = None;
            loop {
                let u = slot.take_or_wait(retry.unwrap_or(REPAIR_EVERY));
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
/// first, so a reader never finds it missing or part-written (D21). The temp
/// can't be an id's file: ids never start with `.`.
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
    file.write_all(text.as_bytes())?;
    drop(file);
    std::fs::rename(&temp, dir.join(format!("{id}.txt")))
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
        retryable, run_timer_loop, sweepable, timer_on_hotkey, timers_dir_name, write_text,
        SleepCheck, TextFiles, TextSink, TextSlot, TextWriter, TimerClock, TimerEvents,
        TimerShared, Waiter, Woke,
    };
    use crate::timers::{TextUpdate, TimerOp, TimerSnapshot, TICK};
    use std::collections::HashSet;
    use std::io;
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    /// A clock the test moves.
    struct FakeClock {
        now: Mutex<Duration>,
        wall_at_zero: i64,
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

    /// Records each wait, moves the clock by it, and closes after `left`.
    struct FakeWaiter {
        clock: Arc<FakeClock>,
        waits: Arc<Mutex<Vec<Duration>>>,
        left: usize,
    }

    impl Waiter for FakeWaiter {
        fn wait(&mut self, d: Duration) -> Woke {
            if self.left == 0 {
                return Woke::Closed;
            }
            self.left -= 1;
            self.waits.lock().unwrap().push(d);
            *self.clock.now.lock().unwrap() += d;
            Woke::Timeout
        }
    }

    #[derive(Clone, Default)]
    struct Seen(Arc<Mutex<Vec<TimerSnapshot>>>);

    impl TimerEvents for Seen {
        fn timer_state(&self, s: &TimerSnapshot) {
            self.0.lock().unwrap().push(s.clone());
        }
    }

    #[derive(Default)]
    struct Handed(Vec<TextUpdate>);

    impl TextSink for Handed {
        fn update(&mut self, u: TextUpdate) {
            self.0.push(u);
        }
    }

    fn shared() -> (Arc<TimerShared>, Arc<FakeClock>) {
        let clock = Arc::new(FakeClock {
            now: Mutex::new(Duration::ZERO),
            wall_at_zero: 1_793_217_600_000,
        });
        let (shared, _woken) = TimerShared::new(
            clock.clone(),
            Box::new(Awake),
            std::env::temp_dir().join("divora-timers-unused"),
        );
        (shared, clock)
    }

    fn scratch(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("divora-timer-thread-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

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
        let (shared, clock) = shared();
        let waits = Arc::new(Mutex::new(Vec::new()));
        let mut waiter = FakeWaiter {
            clock,
            waits: waits.clone(),
            left: 30,
        };
        let seen = Seen::default();
        let mut handed = Handed::default();
        run_timer_loop(&shared, &mut waiter, &seen, &mut handed, TICK);
        let waits = waits.lock().unwrap();
        assert_eq!(waits.len(), 30);
        assert_eq!(waits[0], Duration::ZERO, "the first iteration runs at once");
        assert!(waits[1..].iter().all(|&w| w == TICK), "{waits:?}");
        assert_eq!(*seen.0.lock().unwrap(), Vec::new());
        assert_eq!(handed.0, Vec::new());
    }

    /// And every other reader of an empty core: a `tm:` press of any kind
    /// does nothing, an op answers "no such timer", and a status read is the
    /// empty set with a larger `seq` each time.
    #[test]
    fn an_empty_core_answers_every_reader() {
        let (shared, _clock) = shared();
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
        assert_eq!(slot.take_or_wait(Duration::from_millis(1)), None);
        let set = |text: &str| TextUpdate {
            texts: vec![("timer-1".into(), text.into())],
            keep: vec!["timer-1".into()],
        };
        let mut sink = slot.clone();
        sink.update(set("05:00"));
        sink.update(set("04:59"));
        assert_eq!(slot.take_or_wait(Duration::ZERO), Some(set("04:59")));
        assert_eq!(slot.take_or_wait(Duration::ZERO), None);
    }

    /// S1's writer: with nothing ever handed over, a pass that timed out
    /// has nothing pending.
    #[test]
    fn a_writer_with_nothing_handed_over_waits() {
        let mut files = TextFiles::new(scratch("idle"));
        assert_eq!(files.apply(None, Instant::now()), None);
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
}
