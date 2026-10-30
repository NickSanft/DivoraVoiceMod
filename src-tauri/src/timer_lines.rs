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
    /// The epoch the last Stop all made, for the log's outcome only.
    stopped_at: u64,
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
        self.stale(t).is_none()
    }

    /// Why `t` is no longer the line to say, for the log: its timer's run
    /// `ended`, a Stop all `stopped` it, or a newer line `replaced` it.
    /// `None` while it is wanted.
    fn stale(&self, t: &Ticket) -> Option<&'static str> {
        if self.run_of(&t.line.timer) != t.run {
            Some("ended")
        } else if self.epoch == t.epoch {
            None
        } else if self.epoch.wrapping_sub(self.stopped_at) < self.epoch.wrapping_sub(t.epoch) {
            Some("stopped")
        } else {
            Some("replaced")
        }
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
        inner.stopped_at = inner.epoch;
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
        self.play_or_why(t, send).is_none()
    }

    /// [`play_if_wanted`](Self::play_if_wanted), saying why it didn't play.
    fn play_or_why(&self, t: &Ticket, send: impl FnOnce()) -> Option<&'static str> {
        let inner = lock(&self.inner);
        let stale = inner.stale(t);
        if stale.is_none() {
            send();
        }
        stale
    }

    /// Why `t` went stale (`ended`, `stopped`, `replaced`), or `None`.
    fn stale(&self, t: &Ticket) -> Option<&'static str> {
        lock(&self.inner).stale(t)
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
/// taken, after it played or was dropped: its timer, voice, characters,
/// render time, outcome (`played`, `replaced`, `ended`, `stopped`,
/// `failed`) and whether the engine was running (`running`; a stopped
/// engine plays nothing). Returns once [`TimerLines::close`] is called.
#[doc(hidden)]
pub fn run_line_thread<R>(
    lines: &TimerLines,
    render: &mut dyn FnMut(&Line) -> Result<R, String>,
    play: &mut dyn FnMut(R),
    running: &dyn Fn() -> bool,
) {
    while !lines.is_closed() {
        let Some(ticket) = lines.take_or_wait(LINE_WAIT) else {
            continue;
        };
        let line = &ticket.line;
        let chars = line.text.chars().count();
        if let Some(outcome) = lines.stale(&ticket) {
            tracing::info!(timer = %line.timer, voice = %line.voice_id, chars, outcome, "timer line");
            continue;
        }
        let started = Instant::now();
        let rendered = render(line);
        let render_ms = started.elapsed().as_millis();
        match rendered {
            Ok(r) => {
                let outcome = lines.play_or_why(&ticket, || play(r)).unwrap_or("played");
                let engine_running = running();
                tracing::info!(timer = %line.timer, voice = %line.voice_id, chars, render_ms, outcome, engine_running, "timer line");
            }
            Err(error) => {
                tracing::info!(timer = %line.timer, voice = %line.voice_id, chars, render_ms, %error, outcome = "failed", "timer line");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{lock, run_line_thread, Line, Ticket, TimerLines};
    use crate::timer_actions::{plan, Action, ActionBatch, Hooks};
    use crate::timer_thread::{hand_off, run_action_worker, ActionHost, ACTION_QUEUE};
    use crate::timers::Transition;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::mpsc::{self, Receiver};
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    fn secs(n: u64) -> Duration {
        Duration::from_secs(n)
    }

    fn line(timer: &str, text: &str) -> Line {
        Line {
            timer: timer.to_string(),
            text: text.to_string(),
            voice_id: "babble:bright".to_string(),
        }
    }

    /// The line waiting now, without waiting for one.
    fn take(lines: &TimerLines) -> Option<Ticket> {
        lines.take_or_wait(Duration::ZERO)
    }

    /// Whether a line still waits for the thread to take it.
    fn waiting(lines: &TimerLines) -> bool {
        lock(&lines.inner).waiting.is_some()
    }

    fn speak(text: &str) -> Action {
        Action::Speak {
            text: text.to_string(),
            voice_id: "babble:bright".to_string(),
        }
    }

    fn hooks(on_start: Vec<Action>, on_fire: Vec<Action>) -> Arc<Hooks> {
        Arc::new(Hooks { on_start, on_fire })
    }

    fn started(id: &str, hooks: &Arc<Hooks>) -> Transition {
        Transition::Started {
            id: id.to_string(),
            fires: 0,
            hooks: hooks.clone(),
        }
    }

    fn fired(id: &str, hooks: &Arc<Hooks>, finished: bool) -> Transition {
        Transition::Fired {
            id: id.to_string(),
            fires: 1,
            late_ms: 0,
            finished,
            hooks: hooks.clone(),
        }
    }

    /// The worker's host as `AppHost` is for lines — `speak` puts, `end_run`
    /// retires, `stop_all` stops through the lines — over a real
    /// [`TimerLines`]; the rest isn't a line's business.
    struct LinesHost(Arc<TimerLines>);

    impl ActionHost for LinesHost {
        fn set_muted(&self, _muted: bool) {}

        fn stop_all(&self) {
            self.0.stop_all(|| {});
        }

        fn save_replay(&self, _at: Instant) {}

        fn speak(&self, timer: &str, text: &str, voice_id: &str) {
            self.0.put(Line {
                timer: timer.to_string(),
                text: text.to_string(),
                voice_id: voice_id.to_string(),
            });
        }

        fn end_run(&self, timer: &str) {
            self.0.end_run(timer);
        }
    }

    /// `transitions` planned in order and handed off as the timer thread
    /// hands them (the real [`hand_off`]), then run by the real worker
    /// against `lines`.
    fn through_the_worker(lines: &Arc<TimerLines>, transitions: &[Transition]) {
        let (tx, rx) = mpsc::sync_channel::<ActionBatch>(ACTION_QUEUE);
        for (t, seq) in transitions.iter().zip(1..) {
            let p = plan(t).expect("planned");
            hand_off(&p, seq, Instant::now(), &tx, |_| {});
        }
        drop(tx);
        run_action_worker(rx, &LinesHost(lines.clone()));
    }

    /// D21: newest wins. A line put while another waits replaces it; one put
    /// while another is being prepared makes that one stale, so it doesn't
    /// play after its render.
    #[test]
    fn newest_line_wins() {
        let lines = TimerLines::new();
        lines.put(line("timer-1", "A"));
        lines.put(line("timer-2", "B"));
        let b = take(&lines).expect("a line");
        assert_eq!(b.line.text, "B");
        assert!(take(&lines).is_none(), "A was replaced while it waited");

        lines.put(line("timer-1", "A"));
        let a = take(&lines).unwrap();
        lines.put(line("timer-2", "B"));
        let b = take(&lines).unwrap();
        let mut played = Vec::new();
        assert!(!lines.play_if_wanted(&a, || played.push("A")));
        assert!(lines.play_if_wanted(&b, || played.push("B")));
        assert_eq!(played, ["B"]);
        assert_eq!((lines.stale(&a), lines.stale(&b)), (Some("replaced"), None));
    }

    /// D21, ENG-9: an end retires its timer's start line — waiting or being
    /// prepared — and nobody else's; its own line, put after, plays. Through
    /// the worker too: End now with an empty At zero still hands a batch,
    /// whose `end_run` makes the break's "Be right back." stale.
    #[test]
    fn an_end_drops_its_timers_start_line() {
        let lines = TimerLines::new();
        lines.put(line("timer-x", "Be right back."));
        let x = take(&lines).unwrap();
        lines.end_run("timer-y");
        assert!(lines.wanted(&x), "another timer's end leaves it");
        lines.end_run("timer-x");
        assert!(!lines.wanted(&x));
        assert_eq!(lines.stale(&x), Some("ended"));
        assert!(!lines.play_if_wanted(&x, || panic!("a retired line played")));
        lines.put(line("timer-x", "I'm back."));
        let end_line = take(&lines).unwrap();
        assert!(lines.play_if_wanted(&end_line, || {}), "the end's own line");

        lines.put(line("timer-y", "Starting soon."));
        let y = take(&lines).unwrap();
        lines.end_run("timer-x");
        assert!(lines.wanted(&y), "X's end leaves Y's line");

        let lines = Arc::new(TimerLines::new());
        let brb = hooks(vec![speak("Be right back.")], vec![]);
        through_the_worker(&lines, &[started("timer-x", &brb)]);
        let start_line = take(&lines).expect("the start's line");
        assert_eq!(start_line.line.text, "Be right back.");
        through_the_worker(&lines, &[fired("timer-x", &brb, true)]);
        assert!(
            !lines.wanted(&start_line),
            "an end with nothing to run still retires the start's line"
        );
    }

    /// D23, ENG-5: Stop all's bump and its `StopAll` share the lines' lock
    /// with a line's check and its Stop/Play. A Stop all called while a line
    /// is inside its play waits for it: the mixer gets `[Stop, Play, StopAll]`,
    /// never a `StopAll` between them (which would leave the line playing
    /// after the Panic). The line's ticket is stale afterwards.
    #[test]
    fn panic_never_lands_between_a_lines_check_and_its_play() {
        let lines = TimerLines::new();
        lines.put(line("timer-1", "I'm back."));
        let ticket = take(&lines).unwrap();
        let sink: Mutex<Vec<&str>> = Mutex::new(Vec::new());
        let (inside_tx, inside) = mpsc::channel::<()>();
        let (sent_tx, sent) = mpsc::channel::<()>();
        let (lines, ticket, sink) = (&lines, &ticket, &sink);
        std::thread::scope(|s| {
            s.spawn(move || {
                lines.play_if_wanted(ticket, || {
                    sink.lock().unwrap().push("Stop");
                    inside_tx.send(()).unwrap();
                    // A Stop all that could get in would be sent by now.
                    let _ = sent.recv_timeout(ms(500));
                    sink.lock().unwrap().push("Play");
                });
            });
            s.spawn(move || {
                inside.recv_timeout(secs(5)).expect("the line is inside");
                lines.stop_all(|| {
                    sink.lock().unwrap().push("StopAll");
                    let _ = sent_tx.send(());
                });
            });
        });
        assert_eq!(*sink.lock().unwrap(), ["Stop", "Play", "StopAll"]);
        assert!(!lines.play_if_wanted(ticket, || panic!("played after Stop all")));
        assert_eq!(lines.stale(ticket), Some("stopped"));
    }

    /// D23: the reverse of the race above. A line put after a Stop all
    /// waits for its `StopAll` to go out, so the Stop all can't cut it off:
    /// the mixer gets `[StopAll, Stop, Play]`.
    #[test]
    fn a_line_put_after_a_stop_all_plays_after_it() {
        let lines = TimerLines::new();
        let sink: Mutex<Vec<&str>> = Mutex::new(Vec::new());
        let (sending_tx, sending) = mpsc::channel::<()>();
        let (played_tx, played) = mpsc::channel::<()>();
        let (lines, sink) = (&lines, &sink);
        std::thread::scope(|s| {
            s.spawn(move || {
                lines.stop_all(|| {
                    sending_tx.send(()).unwrap();
                    // A line that could get in would have played by now.
                    let _ = played.recv_timeout(ms(500));
                    sink.lock().unwrap().push("StopAll");
                });
            });
            s.spawn(move || {
                sending.recv_timeout(secs(5)).expect("Stop all is sending");
                lines.put(line("timer-1", "I'm back."));
                let ticket = take(lines).expect("the new line");
                lines.play_if_wanted(&ticket, || {
                    sink.lock().unwrap().extend(["Stop", "Play"]);
                });
                let _ = played_tx.send(());
            });
        });
        assert_eq!(*sink.lock().unwrap(), ["StopAll", "Stop", "Play"]);
    }

    /// What the fake renderer saw.
    #[derive(Default)]
    struct Renders {
        now: AtomicUsize,
        most: AtomicUsize,
        texts: Mutex<Vec<String>>,
    }

    /// D21, ENG-6: the line thread renders one line at a time. Five lines
    /// put while the first renders: the first goes stale and isn't played,
    /// only the newest of the five is rendered and played, and a line whose
    /// timer ended before its render started is never rendered.
    #[test]
    fn timer_lines_render_one_at_a_time() {
        let lines = TimerLines::new();
        let renders = Renders::default();
        let played: Mutex<Vec<String>> = Mutex::new(Vec::new());
        let (rendering_tx, rendering) = mpsc::channel::<String>();
        let (go_tx, go) = mpsc::channel::<()>();
        let next = |rendering: &Receiver<String>| {
            rendering
                .recv_timeout(secs(5))
                .expect("the thread rendered")
        };
        let (lines, renders, played) = (&lines, &renders, &played);
        std::thread::scope(|s| {
            s.spawn(move || {
                let mut render = |l: &Line| {
                    let now = renders.now.fetch_add(1, Ordering::SeqCst) + 1;
                    renders.most.fetch_max(now, Ordering::SeqCst);
                    renders.texts.lock().unwrap().push(l.text.clone());
                    rendering_tx.send(l.text.clone()).unwrap();
                    let _ = go.recv_timeout(secs(5));
                    renders.now.fetch_sub(1, Ordering::SeqCst);
                    Ok::<String, String>(l.text.clone())
                };
                let mut play = |text: String| played.lock().unwrap().push(text);
                run_line_thread(lines, &mut render, &mut play, &|| false);
            });
            lines.put(line("timer-1", "1"));
            assert_eq!(next(&rendering), "1");
            for text in ["2", "3", "4", "5", "6"] {
                lines.put(line("timer-1", text));
            }
            go_tx.send(()).unwrap();
            assert_eq!(next(&rendering), "6");
            go_tx.send(()).unwrap();
            while played.lock().unwrap().is_empty() {
                std::thread::sleep(ms(2));
            }
            // Hold the thread in a render while X's line is put and its run
            // ends, so it can only find that line already stale.
            lines.put(line("timer-y", "8"));
            assert_eq!(next(&rendering), "8");
            lines.put(line("timer-x", "7"));
            lines.end_run("timer-x");
            go_tx.send(()).unwrap();
            // "7" is taken and dropped unrendered; then the thread waits.
            while waiting(lines) {
                std::thread::sleep(ms(2));
            }
            lines.close();
        });
        assert_eq!(renders.most.load(Ordering::SeqCst), 1, "renders at once");
        assert_eq!(*renders.texts.lock().unwrap(), ["1", "6", "8"]);
        assert_eq!(*played.lock().unwrap(), ["6"]);
    }

    /// §7.1, AT2: a Toggle at the deadline drains `[Fired, Started]`. With a
    /// line in each list, the worker puts the end's "I'm back." and then the
    /// start's "Be right back.", and only the start's is said.
    #[test]
    fn a_toggle_at_the_deadline_says_only_the_start_line() {
        let lines = Arc::new(TimerLines::new());
        let brb = hooks(vec![speak("Be right back.")], vec![speak("I'm back.")]);
        through_the_worker(
            &lines,
            &[fired("timer-1", &brb, false), started("timer-1", &brb)],
        );
        let played: Mutex<Vec<String>> = Mutex::new(Vec::new());
        let (said_tx, said) = mpsc::channel::<()>();
        std::thread::scope(|s| {
            s.spawn(|| {
                let mut render = |l: &Line| Ok::<String, String>(l.text.clone());
                let mut play = |text: String| {
                    played.lock().unwrap().push(text);
                    let _ = said_tx.send(());
                };
                run_line_thread(&lines, &mut render, &mut play, &|| false);
            });
            said.recv_timeout(secs(5)).expect("a line was said");
            lines.close();
        });
        assert_eq!(*played.lock().unwrap(), ["Be right back."]);
    }

    /// D21: a render that fails (Kokoro not installed, an unknown voice)
    /// plays nothing, and the thread goes on to the next line.
    #[test]
    fn a_failed_render_plays_nothing() {
        let lines = TimerLines::new();
        let played: Mutex<Vec<String>> = Mutex::new(Vec::new());
        let (said_tx, said) = mpsc::channel::<()>();
        let (tried_tx, tried) = mpsc::channel::<()>();
        std::thread::scope(|s| {
            s.spawn(|| {
                let mut render = |l: &Line| {
                    let _ = tried_tx.send(());
                    if l.text == "bad" {
                        Err("not installed".to_string())
                    } else {
                        Ok(l.text.clone())
                    }
                };
                let mut play = |text: String| {
                    played.lock().unwrap().push(text);
                    let _ = said_tx.send(());
                };
                run_line_thread(&lines, &mut render, &mut play, &|| false);
            });
            lines.put(line("timer-1", "bad"));
            tried.recv_timeout(secs(5)).expect("the thread tried it");
            lines.put(line("timer-1", "good"));
            said.recv_timeout(secs(5)).expect("the next line was said");
            lines.close();
        });
        assert_eq!(*played.lock().unwrap(), ["good"]);
    }
}
