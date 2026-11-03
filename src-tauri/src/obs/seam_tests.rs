//! v1.61.0 (track G, §10.2, §12's seams): the link through the app's own
//! parts, against the fake obs-websocket server (`fake.rs`). Every link here
//! is started by [`ObsShared`] — its status cell and generations, the slots
//! and due marks, the written set, the stored password — and Follow OBS
//! drives a real [`TimerShared`]. Where a timer's run matters, the timer
//! thread is the real [`run_timer_loop`] on a clock the test moves, handing
//! the feed through the app's [`TimerSinks`], and its hook runs reach
//! [`ObsShared::offer`] through the real action worker and a host whose
//! `obs` is `AppHost::obs`'s own line.
//!
//! Each test is on its own current-thread runtime under the fake's 10 s
//! guard ([`guarded`]), with §10.2's timings (100 ms units) and the fake on
//! 127.0.0.1 and a port the OS picked. The link's own cases with stubs are
//! `link.rs`'s (track B1); `ObsShared` with stub link ends is `mod.rs`'s
//! (track C).

// Test code: a panic fails the test (mod.rs: "Test modules allow these").
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic
)]

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::SyncSender;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use tokio::runtime::Handle;
use tokio::sync::watch;
use zeroize::Zeroizing;

use super::fake::{
    guarded, Accept, Config, Dir, FakeObs, Frame, Model, Output, OutputState as Out, Rule, BRB,
    GAME, JUST_CHATTING, TEXT_FT2, TEXT_V3,
};
use super::files::{self, Cipher, Dpapi, PASSWORD_FILE, RETURNS_FILE};
use super::machine::{ObsConfig, ObsState, ObsStatus, SkipOp, SkipWhy};
use super::scenes::{SceneRef, Slots};
use super::testing::{assert_none_in, scratch, secret_forms, MemCipher};
use super::{
    lock, wall_ms, Emit, Follow, FollowObs, LinkOp, LinkPeriods, LinkTiming, ObsFeed, ObsOp,
    ObsShared, ObsText, SlotStore, StatusSink, TimerControl, EXIT_WAIT, NOT_CONNECTED,
};
use crate::timer_actions::{ActionBatch, Planned};
use crate::timer_thread::{
    hand_off, run_action_worker, run_timer_loop, ActionHost, ChannelWaiter, SleepCheck, TextSlot,
    TimerClock, TimerEvents, TimerShared, TimerSinks, Waiter, Woke, ACTION_QUEUE,
};
use crate::timers::{TimerOp, TimerPhase, TimerSnapshot, TimerState, TICK};

const PASSWORD: &str = "correct horse \u{2713} 7Qz";

fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}

/// §10.2's timings: 100 ms units (the grace 2 s, a request 200 ms).
fn timing() -> LinkTiming {
    LinkTiming::scaled(ms(100)).unwrap()
}

fn game() -> SceneRef {
    SceneRef {
        uuid: GAME.into(),
        name: "Game".into(),
    }
}

fn brb() -> SceneRef {
    SceneRef {
        uuid: BRB.into(),
        name: "BRB".into(),
    }
}

fn scene_op(uuid: &str, name: &str) -> ObsOp {
    ObsOp::Scene {
        scene_uuid: uuid.into(),
        scene_name: name.into(),
    }
}

fn cfg(enabled: bool, host: &str, port: u16) -> ObsConfig {
    ObsConfig {
        enabled,
        host: host.into(),
        port,
    }
}

/// Switched on, at the fake.
fn on(fake: &FakeObs) -> ObsConfig {
    cfg(true, "127.0.0.1", fake.port())
}

// ---- ObsShared on the test's runtime ---------------------------------------------

/// Every status `obs-status` carried, in order.
type Seen = Arc<Mutex<Vec<ObsStatus>>>;

fn recording() -> (Emit, Seen) {
    let seen: Seen = Arc::default();
    let into = seen.clone();
    let emit: Emit = Box::new(move |s| into.lock().unwrap().push(s.clone()));
    (emit, seen)
}

/// A link that follows no timer.
struct NoTimers;

impl TimerControl for NoTimers {
    fn apply(&self, _id: &str, _op: TimerOp) -> bool {
        false
    }
}

/// `ObsShared` as `obs::init` builds it, but on this test's runtime, with
/// §10.2's timings: no timer thread, a feed the test sends.
struct Plain {
    obs: Arc<ObsShared>,
    seen: Seen,
    feed: watch::Sender<Arc<ObsFeed>>,
    dir: PathBuf,
}

fn plain(dir: PathBuf) -> Plain {
    plain_with(
        dir,
        Arc::new(MemCipher::default()),
        Arc::new(NoTimers),
        &timing(),
    )
}

fn plain_with(
    dir: PathBuf,
    cipher: Arc<dyn Cipher>,
    timers: Arc<dyn TimerControl>,
    timing: &LinkTiming,
) -> Plain {
    let (emit, seen) = recording();
    let (feed, feed_rx) = watch::channel(Arc::default());
    let obs = Arc::new(ObsShared::new(
        emit,
        Handle::current(),
        cipher,
        dir.clone(),
        timers,
        feed_rx,
        *timing,
    ));
    Plain {
        obs,
        seen,
        feed,
        dir,
    }
}

/// Waits up to `within` for `f`, checking every 2 ms (the link runs on this
/// runtime meanwhile).
async fn eventually(within: Duration, mut f: impl FnMut() -> bool) -> bool {
    let end = Instant::now() + within;
    loop {
        if f() {
            return true;
        }
        if Instant::now() >= end {
            return false;
        }
        tokio::time::sleep(ms(2)).await;
    }
}

async fn state(obs: &ObsShared, state: ObsState, within: Duration) -> bool {
    eventually(within, || obs.status().state == state).await
}

async fn connected(obs: &ObsShared) {
    assert!(
        state(obs, ObsState::Connected, ms(3_000)).await,
        "never connected: {:?}",
        obs.status()
    );
}

/// Waits for the slot saver to have written everything handed to it.
async fn saved(obs: &ObsShared) {
    assert!(eventually(ms(2_000), || obs.shared.saver.idle()).await);
}

/// Waits for `returns.json` in `dir` to read `json`, the saver then idle:
/// the state the file keeps. (A switch back's commit, and its save, come
/// after OBS answers the Set the fake has already recorded.)
async fn kept(obs: &ObsShared, dir: &std::path::Path, json: &str) {
    let file = dir.join(RETURNS_FILE);
    let reads = || std::fs::read_to_string(&file).is_ok_and(|text| text == json);
    assert!(
        eventually(ms(2_000), reads).await,
        "{:?}",
        std::fs::read_to_string(&file)
    );
    saved(obs).await;
    assert!(reads(), "a later write replaced it");
}

/// Every scene skip a status carried, without its time, in order, each
/// once.
fn skips(seen: &Seen) -> Vec<(SkipOp, SkipWhy, String)> {
    let mut out: Vec<(SkipOp, SkipWhy, String)> = Vec::new();
    let mut last = None;
    for s in seen.lock().unwrap().iter() {
        let skip = s
            .last_skip
            .as_ref()
            .map(|k| (k.op, k.why, k.timer.clone(), k.at_ms));
        if skip != last {
            if let Some(k) = &s.last_skip {
                out.push((k.op, k.why, k.timer.clone()));
            }
            last = skip;
        }
    }
    out
}

fn skip_now(obs: &ObsShared) -> Option<(SkipOp, SkipWhy, String)> {
    obs.status()
        .last_skip
        .map(|k| (k.op, k.why, k.timer.clone()))
}

/// The scenes the fake was told to switch to, in order.
fn sets(fake: &FakeObs) -> Vec<String> {
    fake.requests_of("SetCurrentProgramScene")
        .iter()
        .map(|r| {
            r.data.as_ref().unwrap()["sceneUuid"]
                .as_str()
                .unwrap()
                .to_string()
        })
        .collect()
}

/// The pushes the fake received: `(connection, uuid, text)`.
fn pushes(fake: &FakeObs) -> Vec<(u64, String, String)> {
    fake.requests_of("SetInputSettings")
        .iter()
        .map(|r| {
            let d = r.data.as_ref().unwrap();
            (
                r.session,
                d["inputUuid"].as_str().unwrap().to_string(),
                d["inputSettings"]["text"].as_str().unwrap().to_string(),
            )
        })
        .collect()
}

/// The texts pushed to `uuid`, in order.
fn texts_to(fake: &FakeObs, uuid: &str) -> Vec<String> {
    pushes(fake)
        .into_iter()
        .filter(|(_, u, _)| u == uuid)
        .map(|(_, _, t)| t)
        .collect()
}

/// When the fake read a close from the client, and its code.
fn client_close(fake: &FakeObs) -> Option<(Instant, Option<u16>)> {
    fake.log().iter().find_map(|r| match (&r.dir, &r.frame) {
        (Dir::In, Frame::Close(code, _)) => Some((r.at, *code)),
        _ => None,
    })
}

fn texts(list: &[(&str, &str)]) -> Arc<ObsFeed> {
    Arc::new(ObsFeed {
        texts: list
            .iter()
            .map(|(uuid, text)| ObsText {
                uuid: (*uuid).into(),
                text: (*text).into(),
            })
            .collect(),
        ..ObsFeed::default()
    })
}

/// `obs.before_exit()` from a plain thread, as the tray's Quit calls it
/// from the main thread, while this runtime goes on driving the link: how
/// long it took, and when it returned.
async fn quit(obs: &Arc<ObsShared>) -> (Duration, Instant) {
    let obs = Arc::clone(obs);
    let thread = std::thread::spawn(move || {
        let start = Instant::now();
        obs.before_exit();
        (start.elapsed(), Instant::now())
    });
    while !thread.is_finished() {
        tokio::time::sleep(ms(1)).await;
    }
    thread.join().unwrap()
}

// ---- A real TimerShared and its threads -------------------------------------------

/// The timer core's clocks, moved by the test (the wall follows the
/// monotonic one from the real wall at the start). A sleep moves both and
/// is reported once, to the first reading at or past the wake, as v1.54.0's
/// `SleepWatch` reports one.
struct TestClock {
    state: Mutex<ClockState>,
    wall_at_zero: i64,
}

struct ClockState {
    now: Duration,
    /// A sleep not reported yet: the reading before it, and the wake.
    slept: Option<(Duration, Duration)>,
}

impl TestClock {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            state: Mutex::new(ClockState {
                now: Duration::ZERO,
                slept: None,
            }),
            wall_at_zero: wall_ms(),
        })
    }

    fn advance(&self, by: Duration) {
        lock(&self.state).now += by;
    }

    /// The PC sleeps for `by`.
    fn sleep(&self, by: Duration) {
        let mut s = lock(&self.state);
        let before = s.now;
        s.now += by;
        s.slept = Some((before, s.now));
    }
}

impl TimerClock for TestClock {
    fn now(&self) -> Duration {
        lock(&self.state).now
    }

    fn wall_ms(&self) -> i64 {
        self.wall_at_zero + i64::try_from(self.now().as_millis()).unwrap()
    }
}

/// [`TestClock`]'s sleep check.
struct TestSleep(Arc<TestClock>);

impl SleepCheck for TestSleep {
    fn jumped(&mut self, clock: Duration) -> Option<Duration> {
        let mut s = lock(&self.0.state);
        match s.slept {
            Some((before, woke)) if clock >= woke => {
                s.slept = None;
                Some(before)
            }
            _ => None,
        }
    }
}

/// The real wake channel's waiter, which ends the loop when the test is
/// done (the app's never ends: the core holds its sender for good). Waits
/// at most 20 ms at a time, which only ticks the core early.
struct Stoppable {
    woken: ChannelWaiter,
    stop: Arc<AtomicBool>,
}

impl Waiter for Stoppable {
    fn wait(&mut self, d: Duration) -> Woke {
        if self.stop.load(Ordering::SeqCst) {
            return Woke::Closed;
        }
        let woke = self.woken.wait(d.min(ms(20)));
        if self.stop.load(Ordering::SeqCst) {
            Woke::Closed
        } else {
            woke
        }
    }
}

/// The timer thread's outputs without a page: each hook run's batch handed
/// to the action worker as `TimerOutputs::timer_actions` hands it.
struct Outputs {
    actions: SyncSender<ActionBatch>,
}

impl TimerEvents for Outputs {
    fn timer_state(&self, _s: &TimerSnapshot) {}

    fn timer_actions(&self, p: &Planned, seq: u64) {
        hand_off(p, seq, Instant::now(), &self.actions, |_| {});
    }
}

/// `AppHost` once `try_state` has found the `ObsShared`: its `obs` is
/// `AppHost::obs`'s own line (`the_timer_host_hands_obs_ops_to_the_link`
/// reads it in `lib.rs`); a timer's other actions aren't the link's.
struct Host(Arc<ObsShared>);

impl ActionHost for Host {
    fn set_muted(&self, _muted: bool) {}

    fn stop_all(&self) {}

    fn save_replay(&self, _at: Instant) {}

    fn speak(&self, _timer: &str, _text: &str, _voice_id: &str) {}

    fn end_run(&self, _timer: &str) {}

    fn obs(&self, timer: &str, op: &ObsOp) {
        self.0.offer(timer, op);
    }
}

/// `ObsShared` with a real `TimerShared` (on [`TestClock`]), whose timer
/// thread hands the feed through `TimerSinks` and whose hook runs go
/// through the real action worker to `offer`: setup's wiring.
struct Rig {
    obs: Arc<ObsShared>,
    timers: Arc<TimerShared>,
    clock: Arc<TestClock>,
    dir: PathBuf,
    stop: Arc<AtomicBool>,
    threads: Vec<std::thread::JoinHandle<()>>,
}

impl Rig {
    fn new(dir: PathBuf) -> Self {
        let clock = TestClock::new();
        let (timers, woken) = TimerShared::new(
            clock.clone(),
            Box::new(TestSleep(clock.clone())),
            dir.join("timers-unused"),
        );
        // Setup's parts: the feed, `ObsShared` on its receiver, the action
        // worker and its host, the timer thread on the sender. `ObsShared`
        // comes first here because this host holds it; the app's finds it
        // by `try_state` once `obs::init` has managed it.
        let (feed, feed_rx) = watch::channel(Arc::default());
        let (actions, batches) = std::sync::mpsc::sync_channel(ACTION_QUEUE);
        let (emit, _seen) = recording();
        let obs = Arc::new(ObsShared::new(
            emit,
            Handle::current(),
            Arc::new(MemCipher::default()),
            dir.clone(),
            timers.clone(),
            feed_rx,
            timing(),
        ));
        let host = Host(obs.clone());
        let worker = std::thread::spawn(move || run_action_worker(batches, &host));
        let stop = Arc::new(AtomicBool::new(false));
        let (shared, until) = (timers.clone(), stop.clone());
        let ticker = std::thread::spawn(move || {
            let mut waiter = Stoppable {
                woken: ChannelWaiter(woken),
                stop: until,
            };
            let mut sink = TimerSinks {
                files: Arc::new(TextSlot::new()),
                obs: feed,
            };
            run_timer_loop(&shared, &mut waiter, &Outputs { actions }, &mut sink, TICK);
        });
        Self {
            obs,
            timers,
            clock,
            dir,
            stop,
            // The ticker first: its end drops the worker's sender.
            threads: vec![ticker, worker],
        }
    }

    /// `sync_timers` with these entries.
    fn sync(&self, entries: &[Value]) {
        let _ = self.timers.sync(entries.to_vec());
    }

    /// A command's op, as the page's buttons send it.
    fn apply(&self, id: &str, op: TimerOp) {
        self.timers.apply(id, op).unwrap();
    }

    /// `id` now, ticked (a reader: the thread wakes if it changed).
    fn timer(&self, id: &str) -> TimerState {
        self.timers
            .status()
            .timers
            .into_iter()
            .find(|t| t.id == id)
            .unwrap()
    }

    /// Moves the clock, and ticks.
    fn advance(&self, by: Duration) {
        self.clock.advance(by);
        let _ = self.timers.status();
    }

    /// Waits for `id` to pass `f`.
    async fn timer_until(&self, id: &str, f: impl Fn(&TimerState) -> bool) -> TimerState {
        let mut last = None;
        let ok = eventually(ms(2_000), || {
            let t = self.timer(id);
            let pass = f(&t);
            last = Some(t);
            pass
        })
        .await;
        let last = last.unwrap();
        assert!(ok, "{id}: {last:?}");
        last
    }
}

impl Drop for Rig {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        for t in self.threads.drain(..) {
            let _ = t.join();
        }
    }
}

/// A count-up sync entry, following `follow` ("stream", "record") if any.
fn countup(id: &str, follow: Option<&str>) -> Value {
    let mut entry = json!({"id": id, "name": "Uptime", "kind": "countup", "durationMs": 300_000});
    if let Some(what) = follow {
        entry["followObs"] = json!(what);
    }
    entry
}

/// With its text in OBS's `uuid` source.
fn mapped(mut entry: Value, uuid: &str) -> Value {
    entry["obsText"] = json!({"uuid": uuid, "name": "Timer"});
    entry
}

/// A 5 min break: Switch OBS scene to BRB at the start, Switch OBS back at
/// zero.
fn break_entry(id: &str) -> Value {
    json!({
        "id": id,
        "name": "BRB",
        "kind": "countdown",
        "durationMs": 300_000,
        "onStart": [{"kind": "obs", "op": "scene", "sceneUuid": BRB, "sceneName": "BRB"}],
        "onFire": [{"kind": "obs", "op": "sceneBack"}],
    })
}

/// The break, as the page stores it after it ran out while Divora was
/// closed: running, its end a minute before `wall_now`.
fn ran_out(id: &str, wall_now: i64) -> Value {
    let mut entry = break_entry(id);
    entry["resume"] =
        json!({"phase": "running", "totalMs": 300_000, "anchorMs": wall_now - 60_000});
    entry
}

/// `returns.json` in `dir` holding `timer`'s break from Game to BRB, as a
/// Divora that quit mid-break left it.
fn left_mid_break(dir: &std::path::Path, timer: &str) {
    let mut slots = Slots::default();
    assert!(slots.after_scene(timer, game(), brb(), wall_ms()));
    std::fs::create_dir_all(dir).unwrap();
    std::fs::write(dir.join(RETURNS_FILE), slots.to_json()).unwrap();
}

// ---- L26: a break end to end (seams 1, 2, 3) -------------------------------------

/// L26: the boot `sync_timers`, then `obs_configure`; the break's Start
/// switches OBS to BRB and its End now back to Game, each through the timer
/// thread, the action worker, `AppHost::obs`'s `offer` and the link; and the
/// break's seconds reach its text source through `TimerSinks`' feed.
#[test]
fn l26_a_break_end_to_end() {
    guarded(async {
        let fake = FakeObs::start(Config::default()).await;
        let rig = Rig::new(scratch("l26"));
        rig.sync(&[mapped(break_entry("brb"), TEXT_V3)]);
        rig.obs.configure(on(&fake)).unwrap();
        connected(&rig.obs).await;
        assert!(
            fake.wait(ms(1_000), |f| texts_to(f, TEXT_V3) == ["05:00"])
                .await
        );

        let started = Instant::now();
        rig.apply("brb", TimerOp::Arm);
        assert!(fake.wait(ms(1_000), |f| sets(f) == [BRB]).await);
        assert!(started.elapsed() < ms(500), "{:?}", started.elapsed());
        assert_eq!(fake.program(), BRB);
        let get = fake.requests_of("GetCurrentProgramScene");
        let set = fake.requests_of("SetCurrentProgramScene");
        assert!(get.len() == 1 && get[0].at <= set[0].at);

        rig.advance(ms(2_000));
        assert!(
            fake.wait(ms(1_000), |f| texts_to(f, TEXT_V3)
                .last()
                .map(String::as_str)
                == Some("04:58"))
                .await,
            "{:?}",
            texts_to(&fake, TEXT_V3)
        );

        rig.apply("brb", TimerOp::Finish);
        assert!(fake.wait(ms(1_000), |f| sets(f) == [BRB, GAME]).await);
        assert_eq!(fake.program(), GAME);
        assert!(
            fake.wait(ms(1_000), |f| texts_to(f, TEXT_V3)
                .last()
                .map(String::as_str)
                == Some("00:00"))
                .await
        );
        assert_eq!(rig.obs.status().held_back, 0);
        // The way back was used.
        kept(&rig.obs, &rig.dir, &Slots::default().to_json()).await;
        assert_eq!(fake.accepts(), 1);
        fake.assert_clean();
        std::fs::remove_dir_all(&rig.dir).ok();
    });
}

// ---- L32: a run-out at boot (seams 2, 5) -----------------------------------------

/// L32: a break that ran out while Divora was closed. The first Divora's
/// break switched to BRB and saved its way back to `returns.json`; a new
/// `ObsShared` (the restart) reads it, the boot `sync_timers` plans the
/// run-out's switch back before any link exists (it is due), and once
/// `obs_configure` connects: the program asked, then Set(Game), within the
/// grace. Then the reverse order: `obs_configure` first.
#[test]
fn l32_a_run_out_at_boot_switches_back() {
    guarded(async {
        // The first Divora: a break, then Quit mid-break.
        let fake = FakeObs::start(Config::default()).await;
        let dir = scratch("l32");
        let first = Rig::new(dir.clone());
        first.sync(&[break_entry("brb")]);
        first.obs.configure(on(&fake)).unwrap();
        connected(&first.obs).await;
        first.apply("brb", TimerOp::Arm);
        assert!(fake.wait(ms(1_000), |f| sets(f) == [BRB]).await);
        saved(&first.obs).await;
        let (took, _) = quit(&first.obs).await;
        assert!(took < EXIT_WAIT, "{took:?}");
        drop(first);
        assert_eq!(
            files::load_returns(&dir, wall_ms())
                .plan_back("brb", BRB)
                .to,
            Some(game()),
            "returns.json kept the way back"
        );

        // The restart: sync first.
        let rig = Rig::new(dir.clone());
        let synced = Instant::now();
        rig.sync(&[ran_out("brb", rig.clock.wall_ms())]);
        assert!(
            eventually(ms(1_000), || rig.obs.status().held_back == 1).await,
            "the run-out's switch back is due before any link: {:?}",
            rig.obs.status()
        );
        assert_eq!(rig.timer("brb").phase, TimerPhase::Expired);
        let before = fake.requests().len();
        rig.obs.configure(on(&fake)).unwrap();
        assert!(fake.wait(ms(2_000), |f| sets(f) == [BRB, GAME]).await);
        let set = fake.requests_of("SetCurrentProgramScene")[1].clone();
        assert!(
            set.at - synced < timing().scene_back_grace,
            "{:?}",
            set.at - synced
        );
        let after: Vec<String> = fake.requests()[before..]
            .iter()
            .map(|r| r.request_type.clone())
            .collect();
        assert_eq!(
            after[3..],
            ["GetCurrentProgramScene", "SetCurrentProgramScene"],
            "{after:?}"
        );
        assert_eq!(fake.program(), GAME);
        assert_eq!(rig.obs.status().held_back, 0);
        fake.assert_clean();
        drop(rig);
        std::fs::remove_dir_all(&dir).ok();

        // The reverse order: configure, then the boot sync.
        let fake = FakeObs::start(Config::default()).await;
        fake.model(|m| m.program = BRB.into());
        let dir = scratch("l32-reverse");
        left_mid_break(&dir, "brb");
        let rig = Rig::new(dir.clone());
        rig.obs.configure(on(&fake)).unwrap();
        let synced = Instant::now();
        rig.sync(&[ran_out("brb", rig.clock.wall_ms())]);
        assert!(fake.wait(ms(2_000), |f| sets(f) == [GAME]).await);
        let get = fake.requests_of("GetCurrentProgramScene");
        let set = fake.requests_of("SetCurrentProgramScene");
        assert!(get.len() == 1 && get[0].at <= set[0].at);
        assert!(set[0].at - synced < timing().scene_back_grace);
        assert_eq!(fake.program(), GAME);
        fake.assert_clean();
        drop(rig);
        std::fs::remove_dir_all(&dir).ok();
    });
}

/// L32 (the grace from `offer`): the boot's run-out switch back while OBS,
/// just started, answers 207 to every program request: it is due again
/// after each, with its first stamp, so it is dropped when the grace has run
/// from the boot (`lastSkip {sceneBack, notReady}`), never switched late.
#[test]
fn l32_a_run_out_obs_never_readies_for_ends_at_its_grace() {
    guarded(async {
        let fake = FakeObs::start(Config::default()).await;
        fake.model(|m| m.program = BRB.into());
        fake.script(Rule::on("GetCurrentProgramScene").not_ready());
        let dir = scratch("l32-not-ready");
        left_mid_break(&dir, "brb");
        let rig = Rig::new(dir.clone());
        let synced = Instant::now();
        rig.sync(&[ran_out("brb", rig.clock.wall_ms())]);
        assert!(eventually(ms(1_000), || rig.obs.status().held_back == 1).await);
        rig.obs.configure(on(&fake)).unwrap();
        connected(&rig.obs).await;
        let grace = timing().scene_back_grace;
        assert!(
            eventually(grace + ms(1_500), || skip_now(&rig.obs)
                == Some((SkipOp::SceneBack, SkipWhy::NotReady, "brb".into())))
            .await,
            "{:?}",
            rig.obs.status()
        );
        let ended = synced.elapsed();
        assert!(ended >= grace && ended < grace + ms(700), "{ended:?}");
        assert!(fake.requests_of("GetCurrentProgramScene").len() >= 3);
        assert_eq!(sets(&fake), [""; 0]);
        assert_eq!(rig.obs.status().held_back, 0);
        fake.assert_clean();
        drop(rig);
        std::fs::remove_dir_all(&dir).ok();
    });
}

// ---- L18, L19: Follow OBS on a real TimerShared (seams 1, 4) ---------------------

/// L18: output events drive the count-ups that follow them, in the core:
/// at connect both outputs are off and the idle timers stay idle; a
/// stream's STARTED runs `up` from 0, RECONNECTING leaves it, RECONNECTED
/// asks `GetStreamStatus` and sets it to OBS's duration, STOPPED pauses it; a
/// recording's STARTED, PAUSED and RESUMED run, pause and resume `rec`; and
/// a STARTED read while a push stalls is applied at once.
#[test]
fn l18_follow_obs_runs_the_core_s_count_ups() {
    guarded(async {
        let fake = FakeObs::start(Config::default()).await;
        let rig = Rig::new(scratch("l18"));
        rig.sync(&[
            mapped(countup("up", Some("stream")), TEXT_V3),
            mapped(countup("rec", Some("record")), TEXT_FT2),
        ]);
        rig.obs.configure(on(&fake)).unwrap();
        connected(&rig.obs).await;
        assert!(fake.wait(ms(1_000), |f| pushes(f).len() == 2).await);
        assert_eq!(rig.timer("up").phase, TimerPhase::Idle);
        assert_eq!(rig.timer("rec").phase, TimerPhase::Idle);

        fake.stream_state(Out::Started);
        let up = rig
            .timer_until("up", |t| t.phase == TimerPhase::Running)
            .await;
        assert!(up.ms < 200, "from 0: {up:?}");

        fake.stream_state(Out::Reconnecting);
        tokio::time::sleep(ms(150)).await;
        assert_eq!(rig.timer("up"), up, "a reconnecting stream changes nothing");
        assert_eq!(fake.requests_of("GetStreamStatus").len(), 1);

        fake.model(|m| m.stream.duration_ms = 61_000);
        fake.stream_state(Out::Reconnected);
        let up = rig.timer_until("up", |t| t.ms == 61_000).await;
        assert_eq!((up.phase, up.text.as_str()), (TimerPhase::Running, "01:01"));
        assert_eq!(fake.requests_of("GetStreamStatus").len(), 2);

        fake.stream_state(Out::Stopped);
        let up = rig
            .timer_until("up", |t| t.phase == TimerPhase::Paused)
            .await;
        assert_eq!(up.ms, 61_000, "STOPPED keeps the last value");

        fake.record_state(Out::Started);
        rig.timer_until("rec", |t| t.phase == TimerPhase::Running)
            .await;
        fake.record_state(Out::Paused);
        rig.timer_until("rec", |t| t.phase == TimerPhase::Paused)
            .await;
        fake.record_state(Out::Resumed);
        rig.timer_until("rec", |t| t.phase == TimerPhase::Running)
            .await;

        // Every text pushed, then a push OBS takes 500 ms to answer.
        let shown = |rig: &Rig, fake: &FakeObs| {
            texts_to(fake, TEXT_V3).last().cloned() == Some(rig.timer("up").text)
                && texts_to(fake, TEXT_FT2).last().cloned() == Some(rig.timer("rec").text)
        };
        assert!(eventually(ms(1_000), || shown(&rig, &fake)).await);
        fake.script(Rule::on("SetInputSettings").times(1).delay(ms(500)));
        let pushed = pushes(&fake).len();
        rig.advance(ms(1_000));
        assert!(fake.wait(ms(1_000), |f| pushes(f).len() > pushed).await);
        let stalled = Instant::now();
        fake.stream_state(Out::Started);
        rig.timer_until("up", |t| t.phase == TimerPhase::Running)
            .await;
        let applied = stalled.elapsed();
        assert!(applied < ms(150), "applied {applied:?} into the stall");
        let answer = fake.requests_of("SetInputSettings")[pushed].answer;
        assert!(
            answer.is_none_or(|(at, _)| at + ms(1) > stalled + applied),
            "the push was answered first"
        );
        fake.assert_clean();
        std::fs::remove_dir_all(&rig.dir).ok();
    });
}

/// L19: seeds, in the core. At connect, a recording 125 s in sets `rec` to
/// 2:05; a follow added while connected asks `GetRecordStatus` and is set;
/// one switched from Streaming to Recording is set; a PC sleep — the core
/// pauses its count-ups and bumps `sleeps` — sets them all again from OBS;
/// and a stream that is reconnecting pauses nothing, where one that is off
/// pauses a running follow.
#[test]
fn l19_seeds_set_the_core_s_count_ups() {
    guarded(async {
        let mut model = Model::standard();
        model.record = Output {
            active: true,
            duration_ms: 125_000,
            ..Output::default()
        };
        let fake = FakeObs::start(Config::default().model(model)).await;
        let rig = Rig::new(scratch("l19"));
        rig.sync(&[
            countup("rec", Some("record")),
            countup("up", Some("stream")),
        ]);
        rig.obs.configure(on(&fake)).unwrap();
        connected(&rig.obs).await;
        let rec = rig
            .timer_until("rec", |t| t.phase == TimerPhase::Running)
            .await;
        assert!(rec.ms.abs_diff(125_000) <= 1_000, "{rec:?}");
        assert_eq!(rec.text, "02:05");
        assert_eq!(rig.timer("up").phase, TimerPhase::Idle);
        assert_eq!(fake.requests_of("GetRecordStatus").len(), 1);

        // A follow added while connected.
        rig.sync(&[
            countup("rec", Some("record")),
            countup("up", Some("stream")),
            countup("rec-2", Some("record")),
        ]);
        assert!(fake.wait_requests("GetRecordStatus", 2, ms(1_000)).await);
        let rec2 = rig
            .timer_until("rec-2", |t| t.phase == TimerPhase::Running)
            .await;
        assert_eq!(rec2.text, "02:05");

        // A follow switched from Streaming to Recording.
        rig.sync(&[
            countup("rec", Some("record")),
            countup("up", Some("record")),
            countup("rec-2", Some("record")),
        ]);
        assert!(fake.wait_requests("GetRecordStatus", 3, ms(1_000)).await);
        let up = rig
            .timer_until("up", |t| t.phase == TimerPhase::Running)
            .await;
        assert_eq!(up.text, "02:05");

        // A sleep: the core pauses all three; OBS sets them again.
        fake.model(|m| m.record.duration_ms = 200_000);
        rig.clock.sleep(Duration::from_secs(120));
        let _ = rig.timers.status();
        assert!(fake.wait_requests("GetRecordStatus", 4, ms(1_000)).await);
        for id in ["rec", "up", "rec-2"] {
            let t = rig.timer_until(id, |t| t.ms == 200_000).await;
            assert_eq!((t.phase, t.text.as_str()), (TimerPhase::Running, "03:20"));
        }
        assert_eq!(fake.requests_of("GetRecordStatus").len(), 4, "one request");

        // A stream that is reconnecting pauses nothing...
        fake.model(|m| {
            m.stream = Output {
                active: true,
                reconnecting: true,
                duration_ms: 30_000,
                ..Output::default()
            };
        });
        let mut list = vec![
            countup("rec", Some("record")),
            countup("up", Some("record")),
            countup("rec-2", Some("record")),
            countup("live", None),
        ];
        rig.sync(&list);
        rig.apply("live", TimerOp::Arm);
        list[3] = countup("live", Some("stream"));
        rig.sync(&list);
        assert!(fake.wait_requests("GetStreamStatus", 2, ms(1_000)).await);
        tokio::time::sleep(ms(100)).await;
        let live = rig.timer("live");
        assert_eq!((live.phase, live.ms), (TimerPhase::Running, 0), "{live:?}");

        // ...where one that is off pauses a running follow.
        fake.model(|m| m.stream = Output::default());
        list.push(countup("live-2", None));
        rig.sync(&list);
        rig.apply("live-2", TimerOp::Arm);
        list[4] = countup("live-2", Some("stream"));
        rig.sync(&list);
        assert!(fake.wait_requests("GetStreamStatus", 3, ms(1_000)).await);
        rig.timer_until("live-2", |t| t.phase == TimerPhase::Paused)
            .await;
        assert_eq!(rig.timer("live").phase, TimerPhase::Running);
        fake.assert_clean();
        std::fs::remove_dir_all(&rig.dir).ok();
    });
}

// ---- L17c, L22, L29: ops and commands through ObsShared ----------------------------

/// L17c: parked in a stop state (`needsPassword`), 20 ops in a row: each
/// `scene` is a `notConnected` skip at once (never `queueFull`: a parked
/// driver is handed nothing), each `sceneBack` with a slot is due; Save
/// password within the grace restarts the link, which runs them.
#[test]
fn l17c_a_stop_state_keeps_the_switch_backs() {
    guarded(async {
        let fake = FakeObs::start(Config::default().with_password(PASSWORD)).await;
        fake.model(|m| m.program = BRB.into());
        let p = plain(scratch("l17c"));
        p.obs.configure(on(&fake)).unwrap();
        assert!(state(&p.obs, ObsState::NeedsPassword, ms(2_000)).await);
        for i in 0..10 {
            assert!(p
                .obs
                .shared
                .after_scene(&format!("back-{i}"), game(), brb()));
        }
        let offered = Instant::now();
        for i in 0..10 {
            let timer = format!("live-{i}");
            p.obs
                .offer(&timer, &scene_op(JUST_CHATTING, "Just Chatting"));
            assert_eq!(
                skip_now(&p.obs),
                Some((SkipOp::Scene, SkipWhy::NotConnected, timer))
            );
            p.obs.offer(&format!("back-{i}"), &ObsOp::SceneBack);
        }
        assert_eq!(p.obs.status().held_back, 10);
        assert!(skips(&p.seen)
            .iter()
            .all(|(_, why, _)| *why == SkipWhy::NotConnected));
        assert_eq!(p.obs.status().state, ObsState::NeedsPassword);

        p.obs
            .set_password(Zeroizing::new(PASSWORD.into()))
            .await
            .unwrap();
        assert!(fake.wait(ms(2_000), |f| sets(f) == [GAME]).await);
        assert!(offered.elapsed() < timing().scene_back_grace);
        assert!(eventually(ms(500), || p.obs.status().held_back == 0).await);
        assert_eq!(p.obs.shared.next_expiry(), None);
        assert_eq!(fake.program(), GAME);
        assert!(fake.requests_of("GetCurrentProgramScene").len() >= 10);
        assert!(!fake.sessions()[0].identified, "parked: never identified");
        fake.assert_clean();
        std::fs::remove_dir_all(&p.dir).ok();
    });
}

/// L22: off, through `ObsShared`: before any configure and after
/// `obs_configure {enabled:false}`, `offer`, Try again, the inventory, an
/// address edit and the password commands open no connection and spawn no
/// task in 500 ms; and a link turned off doesn't come back.
#[test]
fn l22_off_opens_nothing() {
    guarded(async {
        let fake = FakeObs::start(Config::default()).await;
        let p = plain(scratch("l22"));
        let tasks = Handle::current().metrics().num_alive_tasks();
        let port = fake.port();
        p.obs.offer("brb", &scene_op(BRB, "BRB"));
        p.obs.offer("brb", &ObsOp::SceneBack);
        assert_eq!(p.obs.reconnect().state, ObsState::Off);
        for host in ["127.0.0.1", "localhost", "::1", "127.0.0.1"] {
            assert_eq!(
                p.obs.configure(cfg(false, host, port)).unwrap().state,
                ObsState::Off
            );
        }
        p.obs.offer("brb", &scene_op(BRB, "BRB"));
        p.obs.offer("brb", &ObsOp::SceneBack);
        assert_eq!(p.obs.reconnect().state, ObsState::Off);
        assert_eq!(p.obs.inventory().await, Err(NOT_CONNECTED.to_string()));
        assert!(
            p.obs
                .set_password(Zeroizing::new(PASSWORD.into()))
                .await
                .unwrap()
                .has_password
        );
        assert!(!p.obs.forget_password().await.unwrap().has_password);
        assert_eq!(
            Handle::current().metrics().num_alive_tasks(),
            tasks,
            "nothing spawned while off"
        );
        tokio::time::sleep(ms(500)).await;
        assert_eq!(fake.accepts(), 0);
        assert!(lock(&p.obs.link).is_none());
        assert_eq!(p.obs.status().state, ObsState::Off);
        assert!(skips(&p.seen).is_empty(), "no skip while off");

        // On, then off: the link stops, and nothing opens after.
        p.obs.configure(on(&fake)).unwrap();
        connected(&p.obs).await;
        p.obs.configure(cfg(false, "127.0.0.1", port)).unwrap();
        assert!(fake.wait_ended(1, ms(1_000)).await);
        p.obs.offer("brb", &scene_op(BRB, "BRB"));
        assert_eq!(p.obs.reconnect().state, ObsState::Off);
        tokio::time::sleep(ms(500)).await;
        assert_eq!(fake.accepts(), 1);
        assert_eq!(p.obs.status().state, ObsState::Off);
        fake.assert_clean();
        std::fs::remove_dir_all(&p.dir).ok();
    });
}

/// L29: 20 `offer`s from a plain thread while the link waits on a stalled
/// request: each returns in under 5 ms; 16 are queued, and past them a
/// `scene` is `queueFull` and a `sceneBack` is due.
#[test]
fn l29_offer_never_waits_on_a_stalled_link() {
    guarded(async {
        let fake = FakeObs::start(Config::default()).await;
        let p = plain(scratch("l29"));
        p.obs.configure(on(&fake)).unwrap();
        connected(&p.obs).await;
        fake.script(Rule::on("GetCurrentProgramScene").times(1).drop_answer());
        p.obs.offer("first", &scene_op(BRB, "BRB"));
        assert!(
            fake.wait_requests("GetCurrentProgramScene", 1, ms(1_000))
                .await
        );
        assert!(p.obs.shared.after_scene("op-18", game(), brb()));
        let obs = &p.obs;
        let took: Vec<Duration> = std::thread::scope(|s| {
            s.spawn(|| {
                (1..=20)
                    .map(|i| {
                        let op = if i % 2 == 1 {
                            scene_op(BRB, "BRB")
                        } else {
                            ObsOp::SceneBack
                        };
                        let started = Instant::now();
                        obs.offer(&format!("op-{i}"), &op);
                        started.elapsed()
                    })
                    .collect()
            })
            .join()
            .unwrap()
        });
        assert!(took.iter().all(|d| *d < ms(5)), "{took:?}");
        let full = lock(&p.obs.link).as_ref().unwrap().ops.capacity() == 0;
        assert!(full, "16 queued");
        assert_eq!(
            skips(&p.seen),
            [
                (SkipOp::Scene, SkipWhy::QueueFull, "op-17".to_string()),
                (SkipOp::Scene, SkipWhy::QueueFull, "op-19".to_string()),
            ]
        );
        let s = p.obs.status();
        assert_eq!((s.state, s.held_back), (ObsState::Connected, 1), "{s:?}");
        assert!(p.obs.shared.next_expiry().is_some(), "op-20 due, no slot");
        assert_eq!(fake.max_outstanding(), 1);
        fake.assert_clean();
        std::fs::remove_dir_all(&p.dir).ok();
    });
}

// ---- L25b, L33: across link generations and Divora restarts (seam 5) -------------

/// L25b's restart half: the written set is `ObsShared`'s. A source unmapped
/// while OBS is down is cleared once by the next link, a restarted one
/// included; a new `ObsShared` (a Divora restart) never wrote it, so it
/// clears nothing.
#[test]
fn l25b_a_restart_clears_only_what_this_divora_wrote() {
    guarded(async {
        let fake = FakeObs::start(Config::default()).await;
        let dir = scratch("l25b");
        let p = plain(dir.clone());
        p.feed.send_replace(texts(&[(TEXT_V3, "01:00")]));
        p.obs.configure(on(&fake)).unwrap();
        connected(&p.obs).await;
        assert!(
            fake.wait(ms(1_000), |f| texts_to(f, TEXT_V3) == ["01:00"])
                .await
        );

        // OBS goes down; meanwhile the timer is unmapped and the link
        // restarts (Save password).
        fake.set_accept(Accept::CloseAtOnce);
        fake.close_all(1001, "");
        assert!(state(&p.obs, ObsState::Retrying, ms(1_000)).await);
        p.feed.send_replace(texts(&[]));
        let gen = p.obs.shared.current_gen();
        p.obs
            .set_password(Zeroizing::new("pw".into()))
            .await
            .unwrap();
        assert!(p.obs.shared.current_gen() > gen);
        fake.set_accept(Accept::Normal);
        connected(&p.obs).await;
        assert!(
            fake.wait(ms(1_000), |f| texts_to(f, TEXT_V3).len() == 2)
                .await
        );
        tokio::time::sleep(ms(300)).await;
        assert_eq!(texts_to(&fake, TEXT_V3), ["01:00", ""], "cleared once");

        // Mapped again, then Divora quits and starts with it unmapped.
        p.feed.send_replace(texts(&[(TEXT_V3, "02:00")]));
        assert!(
            fake.wait(ms(1_000), |f| texts_to(f, TEXT_V3).len() == 3)
                .await
        );
        let _ = quit(&p.obs).await;
        let sessions = fake.accepts();
        drop(p);
        let again = plain(dir.clone());
        again.obs.configure(on(&fake)).unwrap();
        connected(&again.obs).await;
        tokio::time::sleep(ms(500)).await;
        let theirs: Vec<_> = pushes(&fake)
            .into_iter()
            .filter(|(session, _, _)| *session > sessions as u64)
            .collect();
        assert!(theirs.is_empty(), "a new Divora clears nothing: {theirs:?}");
        fake.assert_clean();
        std::fs::remove_dir_all(&dir).ok();
    });
}

/// L33: the link restarts mid-break. A break switched to BRB, OBS went
/// down, and the switch back came while the link retried (due); then two
/// Save passwords back to back restarted the link twice: the later password
/// is the one on disk (OBS takes only it), and the newest link, reading the
/// same slots and due marks, switches back. `returns.json` was saved by
/// both generations — the slot, then its use — and holds the later state.
#[test]
fn l33_a_restart_mid_break_still_switches_back() {
    guarded(async {
        let fake = FakeObs::start(Config::default().with_password(PASSWORD)).await;
        let dir = scratch("l33");
        let p = plain(dir.clone());
        p.obs
            .set_password(Zeroizing::new(PASSWORD.into()))
            .await
            .unwrap();
        p.obs.configure(on(&fake)).unwrap();
        connected(&p.obs).await;
        p.obs.offer("brb", &scene_op(BRB, "BRB"));
        assert!(fake.wait(ms(1_000), |f| sets(f) == [BRB]).await);
        saved(&p.obs).await;
        assert_eq!(
            files::load_returns(&dir, wall_ms())
                .plan_back("brb", BRB)
                .to,
            Some(game())
        );

        fake.set_accept(Accept::CloseAtOnce);
        fake.close_all(1001, "");
        assert!(state(&p.obs, ObsState::Retrying, ms(1_000)).await);
        p.obs.offer("brb", &ObsOp::SceneBack);
        assert_eq!(p.obs.status().held_back, 1);
        let gen = p.obs.shared.current_gen();
        p.obs
            .set_password(Zeroizing::new("an older try".into()))
            .await
            .unwrap();
        let restarted = p
            .obs
            .set_password(Zeroizing::new(PASSWORD.into()))
            .await
            .unwrap();
        assert_eq!(restarted.state, ObsState::Connecting);
        assert!(p.obs.shared.current_gen() >= gen + 2, "two new generations");
        assert_eq!(restarted.held_back, 1, "the restarts kept the mark");
        let blob = std::fs::read(dir.join(PASSWORD_FILE)).unwrap();
        assert_eq!(
            MemCipher::default().unprotect(&blob).unwrap().as_slice(),
            PASSWORD.as_bytes(),
            "the later save"
        );
        fake.set_accept(Accept::Normal);
        assert!(fake.wait(ms(2_000), |f| sets(f) == [BRB, GAME]).await);
        let back = fake.requests_of("SetCurrentProgramScene")[1].session;
        assert!(back > fake.requests_of("SetCurrentProgramScene")[0].session);
        assert_eq!(fake.program(), GAME);
        assert!(eventually(ms(500), || p.obs.status().held_back == 0).await);
        // The later state.
        kept(&p.obs, &dir, &Slots::default().to_json()).await;
        fake.assert_clean();
        std::fs::remove_dir_all(&dir).ok();
    });
}

/// L33 (the stale half): a driver that outlives its generation — the test
/// holds its command sender, as the moment between a restart and the old
/// driver seeing its stop does — applies nothing. An output event reaching
/// it moves no timer of the real core, a `scene` in its queue sends
/// nothing and records no slot, and it lets its connection go; the new
/// generation's status stands.
#[test]
fn l33_a_stale_driver_applies_nothing() {
    guarded(async {
        let fake = FakeObs::start(Config::default().with_password(PASSWORD)).await;
        let dir = scratch("l33-stale");
        let clock = TestClock::new();
        let (timers, _woken) = TimerShared::new(
            clock.clone(),
            Box::new(TestSleep(clock.clone())),
            dir.join("timers-unused"),
        );
        let _ = timers.sync(vec![countup("up", Some("stream"))]);
        let p = plain_with(
            dir.clone(),
            Arc::new(MemCipher::default()),
            timers.clone(),
            &timing(),
        );
        p.feed.send_replace(Arc::new(ObsFeed {
            follows: vec![Follow {
                id: "up".into(),
                what: FollowObs::Stream,
            }],
            ..ObsFeed::default()
        }));
        let save = |pw: &str| p.obs.set_password(Zeroizing::new(pw.into()));
        save(PASSWORD).await.unwrap();
        p.obs.configure(on(&fake)).unwrap();
        connected(&p.obs).await;

        // A follow op: the old driver kept alive past a restart that parks
        // the new one (a wrong password).
        let held = lock(&p.obs.link).as_ref().unwrap().cmds.clone();
        save("wrong").await.unwrap();
        assert!(state(&p.obs, ObsState::AuthFailed, ms(2_000)).await);
        assert!(fake.session(1).unwrap().end.is_none(), "still connected");
        let after = p.obs.status();
        fake.stream_state(Out::Started);
        assert!(
            fake.wait(ms(1_000), |f| f.session(1).unwrap().end.is_some())
                .await
        );
        tokio::time::sleep(ms(50)).await;
        let up = timers.status().timers[0].clone();
        assert_eq!(up.phase, TimerPhase::Idle, "{up:?}");
        assert_eq!(p.obs.status(), after, "no stale write");
        drop(held);

        // A scene op in an old driver's queue.
        save(PASSWORD).await.unwrap();
        connected(&p.obs).await;
        let live = fake.accepts();
        let (held, ops) = {
            let link = lock(&p.obs.link);
            let h = link.as_ref().unwrap();
            (h.cmds.clone(), h.ops.clone())
        };
        save("wrong").await.unwrap();
        assert!(state(&p.obs, ObsState::AuthFailed, ms(2_000)).await);
        let after = p.obs.status();
        ops.try_send(LinkOp {
            at: Instant::now(),
            timer: "late".into(),
            op: scene_op(BRB, "BRB"),
        })
        .unwrap();
        let id = u64::try_from(live).unwrap();
        assert!(
            fake.wait(ms(1_000), |f| f.session(id).unwrap().end.is_some())
                .await
        );
        tokio::time::sleep(ms(50)).await;
        assert!(fake.requests_of("GetCurrentProgramScene").is_empty());
        assert_eq!(sets(&fake), [""; 0]);
        assert_eq!(p.obs.shared.plan_back("late", BRB).to, None, "no slot");
        assert_eq!(p.obs.status(), after, "no stale write");
        drop((held, ops));
        fake.assert_clean();
        std::fs::remove_dir_all(&dir).ok();
    });
}

// ---- A reload, Try again, Quit (D4, D5) --------------------------------------------

/// D4: a second `obs_configure` with the same address — the page's boot
/// configure after a reload, the address as typed or canonical — keeps the
/// connection (one accept, the same generation) and, while retrying, the
/// backoff (no attempt before `retryAtMs`); after a 4011 it opens nothing
/// in 1 s, and Try again then connects.
#[test]
fn configure_with_the_same_address_keeps_the_link() {
    guarded(async {
        let fake = FakeObs::start(Config::default()).await;
        let port = fake.port();
        let p = plain(scratch("same-address"));
        p.obs.configure(on(&fake)).unwrap();
        connected(&p.obs).await;
        let gen = p.obs.shared.current_gen();
        for host in ["127.0.0.1", " localhost ", "::ffff:127.0.0.1"] {
            let s = p.obs.configure(cfg(true, host, port)).unwrap();
            assert_eq!(s.state, ObsState::Connected, "{host}");
        }
        tokio::time::sleep(ms(300)).await;
        assert_eq!(fake.accepts(), 1);
        assert!(fake.session(1).unwrap().end.is_none());
        assert_eq!(p.obs.shared.current_gen(), gen);

        // OBS down: the link retrying, three failures in.
        fake.set_accept(Accept::CloseAtOnce);
        fake.close_all(1001, "");
        let retries = || {
            p.seen
                .lock()
                .unwrap()
                .iter()
                .filter(|s| s.state == ObsState::Retrying)
                .count()
        };
        assert!(eventually(ms(3_000), || retries() == 3).await);
        let before = p.obs.status();
        let accepts = fake.accepts();
        let retry_at = before.retry_at_ms.unwrap();
        assert!(retry_at - wall_ms() > 300, "{before:?}");
        assert_eq!(p.obs.configure(on(&fake)).unwrap(), before, "a reload");
        assert_eq!(p.obs.shared.current_gen(), gen);
        assert!(eventually(ms(2_000), || wall_ms() >= retry_at - 50).await);
        assert_eq!(fake.accepts(), accepts, "no attempt before retryAtMs");
        assert!(fake.wait_accepts(accepts + 1, ms(1_000)).await);
        let attempt = u64::try_from(accepts + 1).unwrap();
        assert!(
            fake.wait(ms(1_000), |f| f
                .session(attempt)
                .is_some_and(|s| s.end.is_some()))
                .await,
            "the attempt met OBS still down"
        );

        // OBS back; Try again while retrying: an attempt now.
        fake.set_accept(Accept::Normal);
        assert!(eventually(ms(1_000), || retries() == 4).await);
        assert_eq!(p.obs.reconnect().state, ObsState::Retrying);
        connected(&p.obs).await;

        // A kick: a reload opens nothing; Try again connects.
        fake.close_all(4011, "");
        assert!(state(&p.obs, ObsState::Kicked, ms(1_000)).await);
        let accepts = fake.accepts();
        for _ in 0..3 {
            assert_eq!(p.obs.configure(on(&fake)).unwrap().state, ObsState::Kicked);
        }
        tokio::time::sleep(ms(1_000)).await;
        assert_eq!(fake.accepts(), accepts, "nothing opened after the kick");
        assert_eq!(p.obs.status().state, ObsState::Kicked);
        assert_eq!(p.obs.reconnect().state, ObsState::Connecting);
        connected(&p.obs).await;
        assert_eq!(fake.accepts(), accepts + 1);
        fake.assert_clean();
        std::fs::remove_dir_all(&p.dir).ok();
    });
}

/// L36: the tray's Quit, `before_exit`, from a plain thread while the link
/// runs on another runtime: with no link, and with a link but no
/// connection, at once; with a connection, OBS reads close 1001 and Quit
/// returns once it is closed, inside 300 ms; and when OBS neither echoes
/// nor closes and the link would wait 2 s for the echo, Quit still returns
/// at 300 ms.
#[test]
fn l36_quit_closes_the_connection_in_time() {
    guarded(async {
        let none = plain(scratch("l36-none"));
        let (took, _) = quit(&none.obs).await;
        assert!(took < ms(50), "no link: {took:?}");

        let down = FakeObs::start(Config::default()).await;
        down.set_accept(Accept::CloseAtOnce);
        let retrying = plain(scratch("l36-retrying"));
        retrying.obs.configure(on(&down)).unwrap();
        assert!(state(&retrying.obs, ObsState::Retrying, ms(1_000)).await);
        let (took, _) = quit(&retrying.obs).await;
        assert!(took < ms(100), "no connection: {took:?}");

        let fake = FakeObs::start(Config::default()).await;
        let p = plain(scratch("l36"));
        p.obs.configure(on(&fake)).unwrap();
        connected(&p.obs).await;
        let (took, returned) = quit(&p.obs).await;
        let (closed, code) = client_close(&fake).expect("a close");
        assert_eq!(code, Some(1001));
        assert!(
            closed <= returned,
            "Quit returned before OBS read the close"
        );
        assert!(took < EXIT_WAIT, "{took:?}");

        let silent = FakeObs::start(Config::default()).await;
        silent.set_close_echo(false);
        let periods = LinkPeriods {
            close_echo: Duration::from_secs(2),
            ..*timing()
        };
        let slow = plain_with(
            scratch("l36-silent"),
            Arc::new(MemCipher::default()),
            Arc::new(NoTimers),
            &LinkTiming::new(periods).unwrap(),
        );
        slow.obs.configure(on(&silent)).unwrap();
        connected(&slow.obs).await;
        let (took, _) = quit(&slow.obs).await;
        assert_eq!(
            client_close(&silent).map(|(_, code)| code),
            Some(Some(1001))
        );
        assert!(
            took + ms(20) >= EXIT_WAIT && took < EXIT_WAIT + ms(150),
            "{took:?}"
        );
        for (f, dir) in [
            (&down, &retrying.dir),
            (&fake, &p.dir),
            (&silent, &slow.dir),
        ] {
            f.assert_clean();
            std::fs::remove_dir_all(dir).ok();
        }
    });
}

// ---- Seam 8: DPAPI → files → the handshake ----------------------------------------

/// Seam 8: the password `obs_set_password` saves (through the app's DPAPI
/// cipher) is the one the link answers OBS's challenge with: none saved
/// parks it (`needsPassword`); saved, it connects; of two saves back to
/// back the later is used; forgotten, the file is gone and the link parks
/// again; a wrong one is `authFailed`. The file isn't the password. DPAPI
/// needs the user's profile: outside CI a refusal is said and skipped, on
/// CI it fails (§10.4).
#[test]
fn a_saved_password_is_the_one_the_link_sends() {
    if let Err(e) = Dpapi.protect(b"probe") {
        if std::env::var_os("CI").is_none() {
            eprintln!("a_saved_password_is_the_one_the_link_sends skipped: DPAPI can't protect here ({e})");
            return;
        }
        panic!("DPAPI refused to protect on CI: {e}");
    }
    guarded(async {
        let fake = FakeObs::start(Config::default().with_password(PASSWORD)).await;
        let dir = scratch("seam-8");
        let p = plain_with(dir.clone(), Arc::new(Dpapi), Arc::new(NoTimers), &timing());
        let save = |pw: &str| p.obs.set_password(Zeroizing::new(pw.into()));
        p.obs.configure(on(&fake)).unwrap();
        assert!(state(&p.obs, ObsState::NeedsPassword, ms(2_000)).await);
        save("not it").await.unwrap();
        save(PASSWORD).await.unwrap();
        connected(&p.obs).await;
        let blob = std::fs::read(dir.join(PASSWORD_FILE)).unwrap();
        assert_none_in(&String::from_utf8_lossy(&blob), &secret_forms(PASSWORD));
        assert!(p.obs.status().has_password);

        let forgot = p.obs.forget_password().await.unwrap();
        assert!(!forgot.has_password);
        assert!(!dir.join(PASSWORD_FILE).exists());
        assert!(state(&p.obs, ObsState::NeedsPassword, ms(2_000)).await);

        save("wrong").await.unwrap();
        assert!(state(&p.obs, ObsState::AuthFailed, ms(2_000)).await);
        let identified = fake.sessions().iter().filter(|s| s.identified).count();
        assert_eq!(identified, 1, "only the saved password got in");
        fake.assert_clean();
        std::fs::remove_dir_all(&dir).ok();
    });
}
