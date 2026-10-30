//! v1.60.0: a timer's spoken lines (D21) — the latest line and what makes
//! one stale, and the `divora-timer-lines` loop that renders one at a time
//! and plays it in the mixer's own [`TIMER_LINE_CLIP_ID`] slot.
//!
//! The action worker puts a line ([`TimerLines::put`]) and never waits. The
//! line thread takes the newest, renders it, and plays it only if nothing
//! made it stale meanwhile: a newer line from any timer, its timer's end
//! ([`TimerLines::end_run`]) or a Stop all ([`TimerLines::stop_all`]). The
//! check and the mixer sends share one small lock, held only around field
//! reads and sends that never wait — never across a render — so a Stop all
//! can't land between a line's check and its Play.

use std::collections::HashMap;
use std::sync::{Condvar, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

/// The mixer slot a timer's line plays in: its own, so a timer's line never
/// cuts off one you say on the Speak screen, nor the other way round.
pub const TIMER_LINE_CLIP_ID: &str = "timer-line";

/// How long the line thread waits for a line before it checks whether it
/// was closed.
const LINE_WAIT: Duration = Duration::from_secs(1);

/// One line a timer's `speak` action says.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Line {
    /// The timer whose hook put it.
    pub timer: String,
    /// Trimmed, 1 to `MAX_LINE_CHARS` characters.
    pub text: String,
    pub voice_id: String,
}

/// A line as it was put, with what it takes to still be wanted: no line put
/// and no Stop all since (`epoch`), and its timer's run not ended since
/// (`run`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ticket {
    pub epoch: u64,
    pub run: u64,
    pub line: Line,
}

/// What the lock guards.
#[derive(Default)]
struct LinesInner {
    /// + 1 at each put and each Stop all.
    epoch: u64,
    /// Each timer id's run counter, + 1 at each of its ends: one per id seen
    /// (at most a few dozen; ids are reused).
    runs: HashMap<String, u64>,
    /// The newest line the thread hasn't taken.
    waiting: Option<Ticket>,
    closed: bool,
}

impl LinesInner {
    fn run_of(&self, timer: &str) -> u64 {
        self.runs.get(timer).copied().unwrap_or(0)
    }

    fn wanted(&self, t: &Ticket) -> bool {
        self.epoch == t.epoch && self.run_of(&t.line.timer) == t.run
    }
}

/// The latest line and what makes one stale. Its lock is held only to read
/// or change these fields and around sends that never wait — never across a
/// render. `AppState` holds the one there is (D23), so the page's Stop all
/// and a timer's line share it.
pub struct TimerLines {
    inner: Mutex<LinesInner>,
    ready: Condvar,
}

impl TimerLines {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(LinesInner::default()),
            ready: Condvar::new(),
        }
    }

    /// `line` replaces any line waiting, and makes one being rendered stale
    /// (newest wins); it is wanted while its timer's run lasts.
    pub fn put(&self, line: Line) {
        {
            let mut inner = lock(&self.inner);
            inner.epoch = inner.epoch.wrapping_add(1);
            let run = inner.run_of(&line.timer);
            let epoch = inner.epoch;
            inner.waiting = Some(Ticket { epoch, run, line });
        }
        self.ready.notify_one();
    }

    /// `timer`'s run ended: its line waiting or being rendered is stale. A
    /// line put after this is wanted (an end's own line).
    pub fn end_run(&self, timer: &str) {
        let mut inner = lock(&self.inner);
        let run = inner.runs.entry(timer.to_string()).or_insert(0);
        *run = run.wrapping_add(1);
    }

    /// Stop all: every line waiting or being rendered is stale, and `send`
    /// (the mixer's `StopAll`) goes out under the same lock, after the bump —
    /// so no line checked before it can play after it.
    pub fn stop_all(&self, send: impl FnOnce()) {
        let mut inner = lock(&self.inner);
        inner.epoch = inner.epoch.wrapping_add(1);
        inner.waiting = None;
        send();
    }

    /// The newest line, waiting up to `d` for one; `None` when `d` passed
    /// with none, or once closed.
    pub fn take_or_wait(&self, d: Duration) -> Option<Ticket> {
        let inner = lock(&self.inner);
        let (mut inner, _) = self
            .ready
            .wait_timeout_while(inner, d, |i| i.waiting.is_none() && !i.closed)
            .unwrap_or_else(PoisonError::into_inner);
        if inner.closed {
            return None;
        }
        inner.waiting.take()
    }

    /// Whether `t` is still the line to say.
    pub fn wanted(&self, t: &Ticket) -> bool {
        lock(&self.inner).wanted(t)
    }

    /// `send` (the line's Stop then Play) under the lock, if `t` is still
    /// wanted; whether it was.
    pub fn play_if_wanted(&self, t: &Ticket, send: impl FnOnce()) -> bool {
        let inner = lock(&self.inner);
        let wanted = inner.wanted(t);
        if wanted {
            send();
        }
        wanted
    }

    /// The line thread returns at its next look (tests).
    pub fn close(&self) {
        lock(&self.inner).closed = true;
        self.ready.notify_all();
    }

    fn is_closed(&self) -> bool {
        lock(&self.inner).closed
    }
}

impl Default for TimerLines {
    fn default() -> Self {
        Self::new()
    }
}

/// The poison-tolerant lock the tree uses.
fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The `divora-timer-lines` loop: take the newest line; skip it if it went
/// stale while it waited; else `render` it — outside the lock, one at a
/// time — and `play` the result inside [`TimerLines::play_if_wanted`], so
/// only a line still wanted after its render plays. One log line per line
/// taken. Returns once [`TimerLines::close`] is called.
#[doc(hidden)]
pub fn run_line_thread<R>(
    lines: &TimerLines,
    render: &mut dyn FnMut(&Line) -> Result<R, String>,
    play: &mut dyn FnMut(R),
) {
    while !lines.is_closed() {
        let Some(ticket) = lines.take_or_wait(LINE_WAIT) else {
            continue;
        };
        let line = &ticket.line;
        let chars = line.text.chars().count();
        if !lines.wanted(&ticket) {
            tracing::info!(timer = %line.timer, voice = %line.voice_id, chars, outcome = "stale", "timer line");
            continue;
        }
        let started = Instant::now();
        let rendered = render(line);
        let render_ms = started.elapsed().as_millis();
        match rendered {
            Ok(r) => {
                let outcome = if lines.play_if_wanted(&ticket, || play(r)) {
                    "played"
                } else {
                    "stale"
                };
                tracing::info!(timer = %line.timer, voice = %line.voice_id, chars, render_ms, outcome, "timer line");
            }
            Err(error) => {
                tracing::info!(timer = %line.timer, voice = %line.voice_id, chars, render_ms, %error, outcome = "failed", "timer line");
            }
        }
    }
}
