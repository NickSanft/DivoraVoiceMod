//! What the `divora-loopback` thread does (v1.53.0), apart from talking to
//! Windows: open the source, copy every packet into the [`DesktopTap`], and
//! when the device stops working, say so and open it again.
//!
//! It is written against two small traits — [`PacketSource`] for an open
//! capture client, [`Opener`] for opening one — so that the tests below can
//! drive the real loop with scripted devices. The branches matter most on
//! machines this was not developed on: process loopback on Windows 11 sends
//! a packet every 10 ms forever and never fails, so here the loop's timeout,
//! liveness, watchdog and reopen paths would otherwise never run at all.

#![cfg_attr(
    not(test),
    deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)
)]

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::time::{Duration, Instant};

use crate::desktop::{
    packet_time, DesktopPacket, DesktopSource, DesktopTap, OpenedSource, QpcAnchor,
};
use crate::error::CaptureError;

/// How long the thread waits for a packet before looking around: process
/// loopback wakes it every 10 ms; an idle playback device never does.
pub const WAIT_MS: u32 = 20;

/// Packets taken per wake at most, so a device that floods the loop can't
/// keep it from looking at its stop flag.
const MAX_PACKETS_PER_WAKE: usize = 64;

/// Process loopback sends a packet every 10 ms even in silence (measured on
/// Windows 11). A second of nothing means the stream has died quietly.
const WATCHDOG: Duration = Duration::from_secs(1);

/// The first wait before opening a lost source again; it doubles up to
/// [`RETRY_MAX`] while the source stays unavailable.
const RETRY_FIRST: Duration = Duration::from_secs(1);
const RETRY_MAX: Duration = Duration::from_secs(30);

/// How often a source that stays unavailable is logged again.
const LOG_EVERY: Duration = Duration::from_secs(60);

/// One packet as the device hands it out, valid until released.
pub struct RawPacket<'a> {
    /// Interleaved stereo; `None` when it can't be read (the loop records
    /// silence for it).
    pub samples: Option<&'a [f32]>,
    pub frames: u32,
    /// Windows' timestamp of the first frame, in 100-ns units.
    pub qpc_100ns: u64,
    pub silent: bool,
    pub discontinuity: bool,
    pub timestamp_error: bool,
}

/// Whether the source has been lost, and whether it was given up on.
pub struct LossFlags<'a> {
    /// Set while the source is down; cleared when it is open again.
    pub lost: &'a AtomicBool,
    /// Set when Windows stopped answering a reopen: nothing more will be
    /// tried this take.
    pub gave_up: &'a AtomicBool,
}

/// An open, started capture client, as the loop sees it.
pub trait PacketSource {
    /// Wait up to `ms` for packets: `true` if woken, `false` on timeout.
    fn wait(&self, ms: u32) -> Result<bool, CaptureError>;
    /// The next packet waiting, if any.
    fn next(&self) -> Result<Option<RawPacket<'_>>, CaptureError>;
    /// Hand back the packet `next` gave out.
    fn release(&self, frames: u32) -> Result<(), CaptureError>;
    /// Whether a playback device is still there. An idle device sends
    /// nothing, so silence alone says nothing; this does.
    fn alive(&self) -> Result<(), CaptureError>;
    /// The (`Instant`, counter) pair this stream's timestamps are read by.
    fn anchor(&self) -> QpcAnchor;
    /// A playback device, as opposed to process loopback.
    fn is_device(&self) -> bool;
}

/// Why a source could not be opened.
#[derive(Debug)]
pub enum OpenError {
    /// It failed; it may work later.
    Failed(CaptureError),
    /// Windows never answered. Not tried again in this take: each try
    /// leaves a little behind with Windows, and it is unlikely to answer.
    TimedOut(CaptureError),
}

impl OpenError {
    fn into_error(self) -> CaptureError {
        match self {
            Self::Failed(e) | Self::TimedOut(e) => e,
        }
    }
}

/// Opens capture clients, on the capture thread.
pub trait Opener {
    type Stream: PacketSource;
    fn open(&mut self, source: &DesktopSource) -> Result<(Self::Stream, OpenedSource), OpenError>;
}

/// The clock and the sleep the thread uses: real ones in the app, a script
/// in the tests.
pub trait Clock {
    fn now(&self) -> Instant;
    /// Sleep for `d`, waking early when `stop` is set; `true` if it was.
    fn sleep(&self, d: Duration, stop: &AtomicBool) -> bool;
}

/// The thread's whole life: open `source` and report it on `ready`, then
/// copy packets until `stop`, reopening the source — with `flags.lost` set
/// meanwhile, and a new stream each time — when it stops working. If
/// Windows stops answering, it says so in `flags.gave_up` and ends.
pub fn run<O: Opener, C: Clock>(
    opener: &mut O,
    clock: &C,
    source: &DesktopSource,
    tap: &mut DesktopTap,
    stop: &AtomicBool,
    flags: &LossFlags<'_>,
    ready: &Sender<Result<OpenedSource, CaptureError>>,
) {
    let lost = flags.lost;
    let mut stream = match opener.open(source) {
        Ok((stream, what)) => {
            let _ = ready.send(Ok(what));
            stream
        }
        Err(e) => {
            let _ = ready.send(Err(e.into_error()));
            return;
        }
    };
    loop {
        tap.begin_stream();
        match pump(&stream, clock, tap, stop) {
            Ok(()) => return,
            Err(e) => {
                tracing::warn!(error = %e, "desktop audio lost; opening it again");
                lost.store(true, Ordering::Release);
            }
        }
        drop(stream);
        let mut wait = RETRY_FIRST;
        let mut logged: Option<Instant> = None;
        stream = loop {
            if clock.sleep(wait, stop) {
                return;
            }
            match opener.open(source) {
                Ok((s, _)) => break s,
                Err(OpenError::TimedOut(e)) => {
                    tracing::warn!(error = %e, "desktop audio did not come back; giving up for this take");
                    flags.gave_up.store(true, Ordering::Release);
                    return;
                }
                Err(OpenError::Failed(e)) => {
                    let now = clock.now();
                    if logged.is_none_or(|t| now.saturating_duration_since(t) >= LOG_EVERY) {
                        tracing::info!(error = %e, "desktop audio still unavailable");
                        logged = Some(now);
                    }
                    wait = (wait * 2).min(RETRY_MAX);
                }
            }
        };
        lost.store(false, Ordering::Release);
        tracing::info!("desktop audio back");
    }
}

/// Copy packets from `s` into `tap` until `stop` is set (`Ok`) or the
/// source stops working (`Err`).
pub fn pump<S: PacketSource, C: Clock>(
    s: &S,
    clock: &C,
    tap: &mut DesktopTap,
    stop: &AtomicBool,
) -> Result<(), CaptureError> {
    let mut last_packet = clock.now();
    // A playback device sends nothing while idle; the first packet after a
    // quiet wait follows silence, not lost audio.
    let mut idle = false;
    while !stop.load(Ordering::Acquire) {
        let woke = s.wait(WAIT_MS)?;
        if !woke && s.is_device() {
            s.alive()?;
        }
        let mut got = false;
        // Looked at on every wake, timeouts included: that is where a
        // stream that has died quietly reports it.
        for _ in 0..MAX_PACKETS_PER_WAKE {
            let Some(p) = s.next()? else {
                break;
            };
            let frames = p.frames;
            let arrived = clock.now();
            let (at, timed) =
                packet_time(s.anchor(), p.qpc_100ns, p.timestamp_error, arrived, frames);
            tap.ingest(&DesktopPacket {
                samples: p.samples.unwrap_or(&[]),
                frames,
                at,
                arrived,
                timed,
                silent: p.silent || p.samples.is_none(),
                discontinuity: p.discontinuity,
                resumed: idle,
            });
            s.release(frames)?;
            last_packet = arrived;
            idle = false;
            got = true;
        }
        if !woke && !got && s.is_device() {
            idle = true;
        }
        if !s.is_device() && clock.now().saturating_duration_since(last_packet) >= WATCHDOG {
            return Err(CaptureError::Unavailable(
                "Windows stopped sending desktop audio.".to_string(),
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{run, Clock, LossFlags, OpenError, Opener, PacketSource, RawPacket, WATCHDOG};
    use crate::desktop::{desktop_tap, DesktopDrain, DesktopSource, OpenedSource, QpcAnchor};
    use crate::error::CaptureError;
    use std::cell::{Cell, RefCell};
    use std::collections::VecDeque;
    use std::rc::Rc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::mpsc::channel;
    use std::time::{Duration, Instant};

    /// Time that moves only when the loop waits or sleeps.
    #[derive(Clone)]
    struct FakeClock {
        t0: Instant,
        elapsed: Rc<Cell<Duration>>,
    }

    impl FakeClock {
        fn new() -> Self {
            Self {
                t0: Instant::now(),
                elapsed: Rc::new(Cell::new(Duration::ZERO)),
            }
        }
        fn advance(&self, d: Duration) {
            self.elapsed.set(self.elapsed.get() + d);
        }
    }

    impl Clock for FakeClock {
        fn now(&self) -> Instant {
            self.t0 + self.elapsed.get()
        }
        fn sleep(&self, d: Duration, stop: &AtomicBool) -> bool {
            self.advance(d);
            stop.load(Ordering::Acquire)
        }
    }

    /// What a scripted device does on each wait.
    #[derive(Clone, Copy, Debug)]
    enum Step {
        /// Wakes with this many 10 ms packets waiting.
        Packets(u32),
        /// Times out with nothing.
        Quiet,
        /// The next call fails: the device went away.
        Fail,
    }

    /// The tests' record of what the loop did to a device.
    #[derive(Default)]
    struct Calls {
        gets: Cell<u32>,
        releases: Cell<u32>,
        alive: Cell<u32>,
    }

    struct FakeStream {
        device: bool,
        clock: FakeClock,
        script: RefCell<VecDeque<Step>>,
        waiting: Cell<u32>,
        failed: Cell<bool>,
        samples: Vec<f32>,
        qpc: Cell<u64>,
        calls: Rc<Calls>,
        stop_when_done: Rc<AtomicBool>,
    }

    impl PacketSource for FakeStream {
        fn wait(&self, ms: u32) -> Result<bool, CaptureError> {
            self.clock
                .advance(Duration::from_millis(u64::from(ms.min(10))));
            match self.script.borrow_mut().pop_front() {
                Some(Step::Packets(n)) => {
                    self.waiting.set(n);
                    Ok(true)
                }
                Some(Step::Quiet) => {
                    self.clock.advance(Duration::from_millis(u64::from(ms)));
                    Ok(false)
                }
                Some(Step::Fail) => {
                    self.failed.set(true);
                    Ok(false)
                }
                None => {
                    self.stop_when_done.store(true, Ordering::Release);
                    Ok(false)
                }
            }
        }
        fn next(&self) -> Result<Option<RawPacket<'_>>, CaptureError> {
            if self.failed.get() && !self.device {
                return Err(CaptureError::Unavailable("service stopped".into()));
            }
            if self.waiting.get() == 0 {
                return Ok(None);
            }
            self.waiting.set(self.waiting.get() - 1);
            self.calls.gets.set(self.calls.gets.get() + 1);
            let q = self.qpc.get();
            self.qpc.set(q + 100_000);
            Ok(Some(RawPacket {
                samples: Some(&self.samples),
                frames: 480,
                qpc_100ns: q,
                silent: false,
                discontinuity: false,
                timestamp_error: false,
            }))
        }
        fn release(&self, frames: u32) -> Result<(), CaptureError> {
            assert_eq!(frames, 480);
            self.calls.releases.set(self.calls.releases.get() + 1);
            Ok(())
        }
        fn alive(&self) -> Result<(), CaptureError> {
            self.calls.alive.set(self.calls.alive.get() + 1);
            if self.failed.get() {
                Err(CaptureError::Unavailable("unplugged".into()))
            } else {
                Ok(())
            }
        }
        fn anchor(&self) -> QpcAnchor {
            QpcAnchor {
                at: self.clock.t0,
                qpc_100ns: 0,
            }
        }
        fn is_device(&self) -> bool {
            self.device
        }
    }

    /// Hands out scripted devices in turn; an empty queue is a failure.
    struct FakeOpener {
        device: bool,
        clock: FakeClock,
        scripts: VecDeque<Result<Vec<Step>, bool>>,
        opens: u32,
        calls: Rc<Calls>,
        stop: Rc<AtomicBool>,
    }

    impl Opener for FakeOpener {
        type Stream = FakeStream;
        fn open(&mut self, _: &DesktopSource) -> Result<(FakeStream, OpenedSource), OpenError> {
            self.opens += 1;
            match self.scripts.pop_front() {
                Some(Ok(script)) => Ok((
                    FakeStream {
                        device: self.device,
                        clock: self.clock.clone(),
                        script: RefCell::new(script.into()),
                        waiting: Cell::new(0),
                        failed: Cell::new(false),
                        samples: vec![0.5; 960],
                        qpc: Cell::new(0),
                        calls: Rc::clone(&self.calls),
                        stop_when_done: Rc::clone(&self.stop),
                    },
                    OpenedSource::Everything,
                )),
                Some(Err(true)) => Err(OpenError::TimedOut(CaptureError::Unavailable(
                    "hung".into(),
                ))),
                _ => Err(OpenError::Failed(CaptureError::Unavailable(
                    "not there".into(),
                ))),
            }
        }
    }

    struct Outcome {
        opener: FakeOpener,
        drain: DesktopDrain,
        lost: bool,
        gave_up: bool,
        ready_ok: bool,
    }

    fn drive(device: bool, scripts: Vec<Result<Vec<Step>, bool>>) -> Outcome {
        let clock = FakeClock::new();
        let stop = Rc::new(AtomicBool::new(false));
        let mut opener = FakeOpener {
            device,
            clock: clock.clone(),
            scripts: scripts.into(),
            opens: 0,
            calls: Rc::new(Calls::default()),
            stop: Rc::clone(&stop),
        };
        let (mut tap, drain) = desktop_tap();
        let lost = AtomicBool::new(false);
        let gave_up = AtomicBool::new(false);
        let (tx, rx) = channel();
        run(
            &mut opener,
            &clock,
            &DesktopSource::Everything,
            &mut tap,
            &stop,
            &LossFlags {
                lost: &lost,
                gave_up: &gave_up,
            },
            &tx,
        );
        Outcome {
            opener,
            drain,
            lost: lost.load(Ordering::Acquire),
            gave_up: gave_up.load(Ordering::Acquire),
            ready_ok: matches!(rx.try_recv(), Ok(Ok(_))),
        }
    }

    fn streams(drain: &mut DesktopDrain) -> Vec<u64> {
        let mut out = Vec::new();
        drain.drain(Instant::now(), 0.0, &mut out);
        let mut s: Vec<u64> = out.iter().map(|c| c.stream).collect();
        s.dedup();
        s
    }

    /// Process loopback that times out now and then is fine: the playback
    /// device check is never made (a process client answers it `E_NOTIMPL`),
    /// and nothing is reported lost.
    #[test]
    fn process_loopback_timeouts_are_not_a_lost_device() {
        let mut o = drive(
            false,
            vec![Ok(vec![
                Step::Packets(2),
                Step::Quiet,
                Step::Quiet,
                Step::Packets(1),
            ])],
        );
        assert!(o.ready_ok);
        assert_eq!(
            o.opener.calls.alive.get(),
            0,
            "asked a process client for padding"
        );
        assert!(!o.lost);
        assert_eq!(o.opener.opens, 1);
        assert_eq!(streams(&mut o.drain), [1]);
    }

    /// Every packet taken is handed back exactly once — even when the rings
    /// are full and the packet is dropped.
    #[test]
    fn every_packet_is_released_once() {
        // 3000 packets: past the mark ring, so some are dropped.
        let script: Vec<Step> = (0..50).map(|_| Step::Packets(60)).collect();
        let o = drive(false, vec![Ok(script)]);
        assert_eq!(o.opener.calls.gets.get(), 3_000);
        assert_eq!(o.opener.calls.releases.get(), 3_000);
        assert!(o.drain.dropped_frames() > 0, "the rings never filled");
    }

    /// A playback device that goes away is noticed while idle, reported
    /// lost, and opened again as a new stream once it is back.
    #[test]
    fn a_lost_device_is_reopened_as_a_new_stream() {
        let mut o = drive(
            true,
            vec![
                Ok(vec![Step::Packets(3), Step::Quiet, Step::Fail]),
                Err(false), // still unplugged at the first retry
                Ok(vec![Step::Packets(2)]),
            ],
        );
        assert!(o.ready_ok);
        assert!(o.opener.calls.alive.get() >= 2);
        assert_eq!(o.opener.opens, 3);
        assert!(!o.lost, "back, but still reported lost");
        assert!(!o.gave_up);
        assert_eq!(streams(&mut o.drain), [1, 2]);
    }

    /// A playback device that goes quiet and plays again marks its first
    /// packet back as a resume: what lies before it was silence, not audio
    /// lost — and only that packet.
    #[test]
    fn a_device_back_from_idle_says_so() {
        let mut o = drive(
            true,
            vec![Ok(vec![
                Step::Packets(2),
                Step::Quiet,
                Step::Quiet,
                Step::Packets(2),
                Step::Packets(1),
            ])],
        );
        let mut out = Vec::new();
        o.drain.drain(Instant::now(), 0.0, &mut out);
        let resumed: Vec<bool> = out.iter().map(|c| c.resumed).collect();
        assert_eq!(resumed, [false, false, true, false, false]);
        // Process loopback never idles, so never says so.
        let mut o = drive(
            false,
            vec![Ok(vec![Step::Packets(1), Step::Quiet, Step::Packets(1)])],
        );
        out.clear();
        o.drain.drain(Instant::now(), 0.0, &mut out);
        assert!(out.iter().all(|c| !c.resumed));
    }

    /// Process loopback that dies without an error — no packets for a
    /// second — is caught by the watchdog and opened again.
    #[test]
    fn a_silent_process_stream_is_caught_and_reopened() {
        let quiet = (WATCHDOG.as_millis() / 20 + 2) as usize;
        let mut first = vec![Step::Packets(1)];
        first.extend(std::iter::repeat_n(Step::Quiet, quiet));
        let mut o = drive(false, vec![Ok(first), Ok(vec![Step::Packets(1)])]);
        assert_eq!(o.opener.opens, 2);
        assert_eq!(streams(&mut o.drain), [1, 2]);
    }

    /// A process client whose next call fails is lost and reopened too.
    #[test]
    fn a_failing_process_stream_is_reopened() {
        let o = drive(
            false,
            vec![Ok(vec![Step::Packets(1), Step::Fail]), Ok(vec![])],
        );
        assert_eq!(o.opener.opens, 2);
        assert!(!o.lost);
    }

    /// Windows not answering at all is not tried again: the take goes on
    /// without desktop audio, reported lost.
    #[test]
    fn a_hung_reopen_is_not_retried() {
        let o = drive(false, vec![Ok(vec![Step::Fail]), Err(true), Ok(vec![])]);
        assert_eq!(o.opener.opens, 2);
        assert!(o.lost);
        assert!(o.gave_up, "gave up without saying so");
    }

    /// A source that can't be opened at all says so to whoever started it.
    #[test]
    fn a_source_that_never_opens_is_an_error() {
        let o = drive(true, vec![Err(false)]);
        assert!(!o.ready_ok);
        assert_eq!(o.opener.opens, 1);
    }

    /// A clock that sets the stop flag on its fifth sleep.
    struct StopAfter(FakeClock, Cell<u32>);

    impl Clock for StopAfter {
        fn now(&self) -> Instant {
            self.0.now()
        }
        fn sleep(&self, d: Duration, stop: &AtomicBool) -> bool {
            self.0.advance(d);
            self.1.set(self.1.get() + 1);
            if self.1.get() == 5 {
                stop.store(true, Ordering::Release);
            }
            stop.load(Ordering::Acquire)
        }
    }

    /// A stop during the retry waits ends the thread there, and the waits
    /// grow while the device stays away.
    #[test]
    fn stop_ends_the_retries() {
        let clock = FakeClock::new();
        let stop = Rc::new(AtomicBool::new(false));
        let mut opener = FakeOpener {
            device: true,
            clock: clock.clone(),
            scripts: vec![Ok(vec![Step::Fail])].into(),
            opens: 0,
            calls: Rc::new(Calls::default()),
            stop: Rc::clone(&stop),
        };
        let (mut tap, _drain) = desktop_tap();
        let lost = AtomicBool::new(false);
        let gave_up = AtomicBool::new(false);
        let (tx, _rx) = channel();
        run(
            &mut opener,
            &StopAfter(clock.clone(), Cell::new(0)),
            &DesktopSource::Everything,
            &mut tap,
            &stop,
            &LossFlags {
                lost: &lost,
                gave_up: &gave_up,
            },
            &tx,
        );
        assert!(!gave_up.load(Ordering::Acquire));
        // Four failed reopens, backing off 1, 2, 4, 8 s before the fifth wait.
        assert_eq!(opener.opens, 5);
        assert!(clock.elapsed.get() >= Duration::from_secs(1 + 2 + 4 + 8 + 16));
    }
}
