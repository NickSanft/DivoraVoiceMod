//! v1.61.0: a running link — the *driver* (connect, handshake, backoff, the
//! one outstanding request, ops, the feed, Follow OBS, commands, due marks)
//! and the *session* (owns the socket: reads, writes, keepalive, the close),
//! two tasks on [`LinkDeps::spawner`] (§5.5). Every wait also watches
//! `cmds`, whose end is the stop (D4, D6). The link connects only to
//! [`LinkDeps::cfg`]'s IP and port and looks no name up (D7).
//!
//! tokio's `select!` needs its `macros` feature, which stays off (D6), so
//! each wait is one hand-written [`poll_fn`] over its sources in a fixed
//! order, the command channel first: the driver's is `Driver::first_of`, the
//! session's `next_turn`. A source that has ended is never polled again
//! (D39): a closed feed is "no more feed", an op channel that closed is
//! dropped, and a reply, a session's end or a blocking job is taken out of
//! its slot the moment it completes.
//!
//! The order on the wire (§8.2): Hello, then Identify, then nothing before
//! Identified; then at most one request outstanding at OBS, a timed-out one
//! included, until its late answer or the late-answer cap (D9). Events are
//! handled in every state, as they are read (D10).

use std::collections::{HashMap, HashSet, VecDeque};
use std::convert::Infallible;
use std::future::{pending, poll_fn, Future};
use std::io;
use std::net::SocketAddr;
use std::pin::{pin, Pin};
use std::sync::mpsc::SyncSender;
use std::sync::Arc;
use std::task::{ready, Context, Poll};
use std::time::{Duration, Instant};

use futures_core::Stream;
use futures_sink::Sink;
use serde_json::Value;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::TcpStream;
use tokio::runtime::Handle;
use tokio::sync::mpsc::error::{SendError, TrySendError};
use tokio::sync::{mpsc, oneshot, watch};
use tokio::time::{sleep_until, timeout, Interval, MissedTickBehavior, Sleep};
use tokio_websockets::{ClientBuilder, CloseCode, Limits, Message, WebSocketStream};
use zeroize::Zeroizing;

use super::files::Unreadable;
use super::machine::{
    backoff, connect_reason, follow_op, join_seed, next_push, record_status_word, record_word,
    request_failure, sanitize_version, seed_pairs, stream_status_word, stream_word, Failure,
    FollowAction, LastSkip, LinkConfig, ObsInventory, ObsReason, ObsState, ObsStatus, OutputStatus,
    Pushed, RecordState, RevCoalesce, SkipOp, SkipWhy, Stop, StreamState, TextSkip, TextSkipWhy,
    TextSource, Unsupported, Work, NOT_READY,
};
use super::proto::{
    self, auth_string, close_outcome, gate, identify_json, parse_hello, parse_incoming,
    program_scene, reads_from_file, AuthChallenge, CloseOutcome, Hello, Incoming, ObsEvent,
    Request, RequestKind, TextSourceRef, REQUEST_TYPES,
};
use super::scenes::{BackPlan, SceneRef};
use super::{
    blocking, log, wall_ms, FollowObs, LinkCmd, LinkDeps, LinkOp, LinkTiming, ObsFeed, ObsOp,
    PasswordSource, SlotStore, StatusSink, TimerControl, TimerOp, BUSY, EVENTS_QUEUE,
    FRAME_CAP_BYTES, HANDSHAKE_CAP_BYTES, INVENTORY_WAIT, MAX_SCENES, MAX_TEXT_SOURCES,
    NOT_CONNECTED, NO_ANSWER, OBS_TEXT, REFUSED,
};

/// The socket once upgraded.
type Ws = WebSocketStream<Capped>;

/// What `obs_inventory` waits on.
type InventoryReply = oneshot::Sender<Result<ObsInventory, String>>;

/// §5.5: the session keeps at most this many requests waiting for their
/// answer (one is outstanding at a time; closed entries go first).
const REPLIES: usize = 64;

/// D18: a failed push is tried again after the base, doubling, at most 30
/// times the base (30 s in the app).
const PUSH_RETRY_MAX: u32 = 30;

// ---- The capped reader (D7) ---------------------------------------------------

/// D7: the TCP stream with a cap on what the upgrade may read. Past
/// [`HANDSHAKE_CAP_BYTES`] a read fails with `InvalidData` (`notObs`) until
/// [`Capped::lift`], so a peer whose upgrade answer never ends can't grow
/// memory: tokio-websockets reads that answer with no cap of its own. A read
/// never takes more than what is left under the cap. Writes pass through.
struct Capped {
    tcp: TcpStream,
    /// Bytes still allowed before the upgrade is done; `None` once lifted.
    left: Option<usize>,
}

impl Capped {
    fn new(tcp: TcpStream) -> Self {
        Self {
            tcp,
            left: Some(HANDSHAKE_CAP_BYTES),
        }
    }

    /// The upgrade is done: from here the 4 MiB frame cap applies.
    fn lift(&mut self) {
        self.left = None;
    }
}

impl AsyncRead for Capped {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        let Some(left) = this.left else {
            return Pin::new(&mut this.tcp).poll_read(cx, buf);
        };
        if left == 0 {
            return Poll::Ready(Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "the upgrade answer passed 64 KiB",
            )));
        }
        let room = buf.remaining().min(left);
        let n = {
            let mut part = ReadBuf::new(buf.initialize_unfilled_to(room));
            ready!(Pin::new(&mut this.tcp).poll_read(cx, &mut part))?;
            part.filled().len()
        };
        buf.advance(n);
        this.left = Some(left.saturating_sub(n));
        Poll::Ready(Ok(()))
    }
}

impl AsyncWrite for Capped {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bytes: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.get_mut().tcp).poll_write(cx, bytes)
    }

    fn poll_write_vectored(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bufs: &[io::IoSlice<'_>],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.get_mut().tcp).poll_write_vectored(cx, bufs)
    }

    fn is_write_vectored(&self) -> bool {
        self.tcp.is_write_vectored()
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().tcp).poll_flush(cx)
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().tcp).poll_shutdown(cx)
    }
}

// ---- Between the driver and the session ---------------------------------------

/// What the driver hands the session.
enum Out {
    /// One request: its id, its frame, and where its answer goes.
    Request {
        id: String,
        json: String,
        reply: oneshot::Sender<Answer>,
    },
    /// D5's close (1001), then the ack when the link is stopping.
    Close(Option<SyncSender<()>>),
}

/// An answer (op 7), routed by its `requestId`.
#[derive(Debug)]
struct Answer {
    ok: bool,
    code: u64,
    data: Option<Value>,
}

/// D10, D26: a decoded event and the moment the session read it, which a
/// `STARTED` counts from however late the driver gets to it.
#[derive(Debug)]
struct Stamped {
    at: Instant,
    event: ObsEvent,
}

/// §7.1: how a session ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SessionEnd {
    /// OBS's close frame and its code (1005 when the frame had none), or
    /// the stream's end with no close frame (`None`: EOF, an I/O error).
    Closed(Option<u16>),
    /// A frame over 4 MiB (the crate answers 1009).
    TooBig,
    /// A frame the crate refused: invalid UTF-8, a bad frame, a reserved
    /// close code.
    Protocol,
    /// No frame within the keepalive deadline of a ping (D8).
    Silent,
    /// The events queue was full (D10).
    Flooded,
    /// The driver asked, or went away.
    Ours,
}

// ---- The session (§5.5) --------------------------------------------------------

/// The periods the session keeps (D5, D8): [`LinkTiming`]'s, copied out.
#[derive(Clone, Copy, Debug)]
struct SessionTiming {
    ping: Duration,
    deadline: Option<Duration>,
    flush: Duration,
    echo: Duration,
}

impl SessionTiming {
    fn of(t: &LinkTiming) -> Self {
        Self {
            ping: t.ping,
            deadline: t.keepalive_deadline,
            flush: t.close_flush,
            echo: t.close_echo,
        }
    }
}

/// D8: the ping every period, and the deadline a ping arms: any frame read
/// disarms it.
struct Alive {
    ping: Interval,
    silent: Pin<Box<Sleep>>,
    armed: bool,
    deadline: Option<Duration>,
}

impl Alive {
    fn new(timing: SessionTiming) -> Self {
        let start = tokio::time::Instant::now();
        let mut ping = tokio::time::interval_at(start + timing.ping, timing.ping);
        ping.set_missed_tick_behavior(MissedTickBehavior::Delay);
        Self {
            ping,
            silent: Box::pin(sleep_until(start)),
            armed: false,
            deadline: timing.deadline,
        }
    }

    /// A frame came in: OBS is there.
    fn heard(&mut self) {
        self.armed = false;
    }

    /// A ping went out: no frame by the deadline is `silent`. A deadline
    /// already running keeps its time.
    fn pinged(&mut self) {
        if let (Some(d), false) = (self.deadline, self.armed) {
            self.silent.as_mut().reset(tokio::time::Instant::now() + d);
            self.armed = true;
        }
    }
}

/// What one turn of the session's wait found.
enum Turn {
    Out(Option<Out>),
    In(Option<Result<Message, tokio_websockets::Error>>),
    WriteFailed,
    Silent,
    Ping,
}

/// The session's wait, in order: what the driver sends (while the socket
/// takes a frame), the flush of what is queued, the next frame, the
/// keepalive deadline, the ping tick.
fn next_turn(
    cx: &mut Context<'_>,
    ws: &mut Ws,
    out: &mut mpsc::UnboundedReceiver<Out>,
    flushing: &mut bool,
    alive: &mut Alive,
) -> Poll<Turn> {
    match Pin::new(&mut *ws).poll_ready(cx) {
        Poll::Ready(Ok(())) => {
            if let Poll::Ready(sent) = out.poll_recv(cx) {
                return Poll::Ready(Turn::Out(sent));
            }
        }
        Poll::Ready(Err(_)) => return Poll::Ready(Turn::WriteFailed),
        Poll::Pending => {}
    }
    if *flushing {
        match Pin::new(&mut *ws).poll_flush(cx) {
            Poll::Ready(Ok(())) => *flushing = false,
            Poll::Ready(Err(_)) => return Poll::Ready(Turn::WriteFailed),
            Poll::Pending => {}
        }
    }
    if let Poll::Ready(frame) = Pin::new(&mut *ws).poll_next(cx) {
        return Poll::Ready(Turn::In(frame));
    }
    if alive.armed && alive.silent.as_mut().poll(cx).is_ready() {
        return Poll::Ready(Turn::Silent);
    }
    if alive.ping.poll_tick(cx).is_ready() {
        return Poll::Ready(Turn::Ping);
    }
    Poll::Pending
}

/// The session task: owns the socket until the connection ends, then closes
/// it (D5's close when the end is the driver's; otherwise the close or echo
/// the crate queued, flushed within the close's flush time), drops it, acks
/// when the driver handed it the ack, and reports how it ended.
async fn session(
    mut ws: Ws,
    mut out: mpsc::UnboundedReceiver<Out>,
    events: mpsc::Sender<Stamped>,
    end: oneshot::Sender<SessionEnd>,
    timing: SessionTiming,
) {
    let (how, ack) = serve(&mut ws, &mut out, &events, timing).await;
    if how == SessionEnd::Ours {
        close(&mut ws, CloseCode::GOING_AWAY, timing).await;
    } else {
        let _ = timeout(timing.flush, flush(&mut ws)).await;
    }
    drop(ws);
    if let Some(ack) = ack {
        let _ = ack.try_send(());
    }
    let _ = end.send(how);
}

/// The session's loop: requests out, answers routed by id, events stamped
/// and handed over (a full queue ends the connection, D10), pings and their
/// deadline. Returns how it ended, and the ack a stop handed it.
async fn serve(
    ws: &mut Ws,
    out: &mut mpsc::UnboundedReceiver<Out>,
    events: &mpsc::Sender<Stamped>,
    timing: SessionTiming,
) -> (SessionEnd, Option<SyncSender<()>>) {
    let mut replies: Vec<(String, oneshot::Sender<Answer>)> = Vec::new();
    let mut alive = Alive::new(timing);
    let mut flushing = false;
    loop {
        let turn = poll_fn(|cx| next_turn(cx, ws, out, &mut flushing, &mut alive)).await;
        match turn {
            Turn::Out(None) => return (SessionEnd::Ours, None),
            Turn::Out(Some(Out::Close(ack))) => return (SessionEnd::Ours, ack),
            Turn::Out(Some(Out::Request { id, json, reply })) => {
                replies.retain(|(_, r)| !r.is_closed());
                if replies.len() >= REPLIES {
                    replies.remove(0);
                }
                replies.push((id, reply));
                if Pin::new(&mut *ws).start_send(Message::text(json)).is_err() {
                    return (SessionEnd::Closed(None), None);
                }
                flushing = true;
            }
            Turn::In(None) | Turn::WriteFailed => return (SessionEnd::Closed(None), None),
            Turn::In(Some(Err(e))) => return (read_end(&e), None),
            Turn::In(Some(Ok(msg))) => {
                alive.heard();
                if let Some(how) = on_frame(&msg, &mut replies, events) {
                    return (how, None);
                }
                // The crate queued a pong: send it now.
                flushing |= msg.is_ping();
            }
            Turn::Silent => return (SessionEnd::Silent, None),
            Turn::Ping => {
                alive.pinged();
                // D8, D39: an empty payload (a ping asserts ≤ 125 bytes).
                if Pin::new(&mut *ws)
                    .start_send(Message::ping(Vec::new()))
                    .is_err()
                {
                    return (SessionEnd::Closed(None), None);
                }
                flushing = true;
            }
        }
    }
}

/// One frame read: an answer to its request, an event to the driver; a
/// close ends the session. Everything else is ignored (pings are answered
/// by the crate).
fn on_frame(
    msg: &Message,
    replies: &mut Vec<(String, oneshot::Sender<Answer>)>,
    events: &mpsc::Sender<Stamped>,
) -> Option<SessionEnd> {
    if let Some((code, _)) = msg.as_close() {
        return Some(SessionEnd::Closed(Some(code.into())));
    }
    match parse_incoming(msg.as_text()?) {
        Incoming::Response { id, ok, code, data } => {
            if let Some(at) = replies.iter().position(|(r, _)| *r == id) {
                let (_, reply) = replies.swap_remove(at);
                let _ = reply.send(Answer { ok, code, data });
            }
            None
        }
        Incoming::Event(event) => {
            let stamped = Stamped {
                at: Instant::now(),
                event,
            };
            match events.try_send(stamped) {
                Ok(()) => None,
                Err(TrySendError::Full(_)) => Some(SessionEnd::Flooded),
                Err(TrySendError::Closed(_)) => Some(SessionEnd::Ours),
            }
        }
        Incoming::Hello | Incoming::Identified | Incoming::Ignored => None,
    }
}

/// §7.1: a read the crate refused.
fn read_end(e: &tokio_websockets::Error) -> SessionEnd {
    match e {
        tokio_websockets::Error::PayloadTooLong { .. } => SessionEnd::TooBig,
        tokio_websockets::Error::Io(_) => SessionEnd::Closed(None),
        _ => SessionEnd::Protocol,
    }
}

/// D5: the link's own close: `code` (1001 for a stop), the flush within
/// `close_flush`, then reading to OBS's close or the end within
/// `close_echo`; never the sink's own close (1000, then an unbounded wait
/// for the echo). The caller drops the socket after.
async fn close(ws: &mut Ws, code: CloseCode, timing: SessionTiming) {
    // Refused once a close was queued or read: the crate's echo then goes
    // out in the flush.
    let _ = Pin::new(&mut *ws).start_send(Message::close(Some(code), ""));
    let _ = timeout(timing.flush, flush(ws)).await;
    let _ = timeout(timing.echo, async {
        while let Some(Ok(msg)) = poll_fn(|cx| Pin::new(&mut *ws).poll_next(cx)).await {
            if msg.is_close() {
                break;
            }
        }
    })
    .await;
}

/// Everything queued, written.
async fn flush(ws: &mut Ws) -> Result<(), tokio_websockets::Error> {
    poll_fn(|cx| Pin::new(&mut *ws).poll_flush(cx)).await
}

/// One text frame, written.
async fn send_text(ws: &mut Ws, text: String) -> Result<(), tokio_websockets::Error> {
    poll_fn(|cx| Pin::new(&mut *ws).poll_ready(cx)).await?;
    Pin::new(&mut *ws).start_send(Message::text(text))?;
    flush(ws).await
}

// ---- The driver's waits (D6, D39) ------------------------------------------------

/// What a link ends on, when it doesn't stop.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Outcome {
    /// `retrying` with this reason (and the I/O error's kind, for the log).
    Retry(ObsReason, Option<io::ErrorKind>),
    /// A stop state (D13).
    Stop(Stop),
}

/// How a connection attempt went.
enum Attempt {
    /// The link stopped (its commands closed) or went stale.
    Gone,
    Failed(Outcome),
    Identified(Box<Ws>),
}

/// Why the connected loop let go of its connection.
#[derive(Debug)]
enum Exit {
    /// The command sender was dropped: the stop.
    Stopped,
    /// A newer link generation runs (D4).
    Stale,
    /// The session ended on its own.
    Ended(SessionEnd),
    /// The driver ends it.
    Drop(Outcome),
}

/// What one of the driver's waits returned: its step's output, or the
/// first other source that was ready.
enum Got<T> {
    Step(T),
    /// The command channel closed: the stop.
    Stop,
    Cmd(LinkCmd),
    Op(LinkOp),
    Event(Stamped),
    End(SessionEnd),
    /// The outstanding request's answer; `None`: the session dropped it.
    Reply(Option<Answer>),
    /// The feed changed (`false`: its sender is gone, for good).
    Feed(bool),
    /// The wait's deadline passed.
    Timer,
}

/// Which sources a wait watches besides the commands, its step and its
/// deadline (§5.5).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Watch {
    /// Not connected: ops (each handled as *not connected*).
    Offline,
    /// One request outstanding: events, the session's end, the answer.
    Answer,
    /// Connected, waiting out a 207: events and the session's end.
    Pause,
    /// Connected and idle: ops, events, the session's end and the feed.
    Idle,
}

impl Watch {
    fn ops(self) -> bool {
        matches!(self, Self::Offline | Self::Idle)
    }

    fn session(self) -> bool {
        self != Self::Offline
    }

    fn reply(self) -> bool {
        self == Self::Answer
    }

    fn feed(self) -> bool {
        self == Self::Idle
    }
}

#[cfg(test)]
thread_local! {
    /// L35: how many times a driver's wait was polled on this thread (each
    /// test's link runs on its own current-thread runtime).
    static POLLS: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

/// L35: the driver's wait polls on this thread so far.
#[cfg(test)]
fn polls() -> u64 {
    POLLS.with(std::cell::Cell::get)
}

fn count_poll() {
    #[cfg(test)]
    POLLS.with(|p| p.set(p.get().saturating_add(1)));
}

/// `now + d`; `now` in the impossible case it overflows.
fn after(d: Duration) -> Instant {
    let now = Instant::now();
    now.checked_add(d).unwrap_or(now)
}

fn ms_u64(d: Duration) -> u64 {
    u64::try_from(d.as_millis()).unwrap_or(u64::MAX)
}

fn ms_i64(d: Duration) -> i64 {
    i64::try_from(d.as_millis()).unwrap_or(i64::MAX)
}

/// The connection's own part of a wait: events first (so a session's last
/// events come out before its end), then the answer, then the end.
fn poll_session<T>(c: &mut Conn, reply: bool, cx: &mut Context<'_>) -> Option<Got<T>> {
    if let Poll::Ready(Some(e)) = c.events.poll_recv(cx) {
        return Some(Got::Event(e));
    }
    if reply {
        if let Some(rx) = c.reply.as_mut() {
            if let Poll::Ready(answer) = Pin::new(rx).poll(cx) {
                c.reply = None;
                return Some(Got::Reply(answer.ok()));
            }
        }
    }
    if let Some(end) = c.end.as_mut() {
        if let Poll::Ready(how) = Pin::new(end).poll(cx) {
            c.end = None;
            return Some(Got::End(how.unwrap_or(SessionEnd::Closed(None))));
        }
    }
    None
}

/// A command that finds no connection: an inventory hears "not connected";
/// a wake outside `retrying` does nothing (a stop state restarts as a new
/// generation, D4).
fn refuse(cmd: LinkCmd) {
    if let LinkCmd::Inventory(reply) = cmd {
        let _ = reply.send(Err(NOT_CONNECTED.to_string()));
    }
}

/// §7.5: a stop's state, reason and detail.
fn stop_fields(stop: Stop) -> (ObsState, Option<ObsReason>, Option<&'static str>) {
    match stop {
        Stop::NeedsPassword => (ObsState::NeedsPassword, None, None),
        Stop::AuthFailed => (ObsState::AuthFailed, None, None),
        Stop::PasswordUnreadable => (ObsState::PasswordUnreadable, None, None),
        Stop::TooOld => (ObsState::TooOld, None, None),
        Stop::NeedsNewerDivora => (ObsState::NeedsNewerDivora, None, None),
        Stop::Kicked => (ObsState::Kicked, None, None),
        Stop::Unsupported(Unsupported::BadVersion) => {
            (ObsState::Unsupported, Some(ObsReason::BadVersion), None)
        }
        Stop::Unsupported(Unsupported::MissingRequest(name)) => (
            ObsState::Unsupported,
            Some(ObsReason::MissingRequest),
            Some(name),
        ),
    }
}

/// §7.1: a close code as an outcome; `gen` for the log.
fn closed_with(gen: u64, code: Option<u16>) -> Outcome {
    log::closed_by_obs(gen, code);
    match close_outcome(code) {
        CloseOutcome::Stop(stop) => Outcome::Stop(stop),
        CloseOutcome::Retry(reason) => Outcome::Retry(reason, None),
    }
}

/// §7.1: a read the crate refused before Identified.
fn read_reason(e: &tokio_websockets::Error) -> ObsReason {
    match read_end(e) {
        SessionEnd::TooBig => ObsReason::TooBig,
        SessionEnd::Closed(_) => ObsReason::Closed,
        _ => ObsReason::Protocol,
    }
}

/// §7.1: an upgrade that failed: past the 64 KiB cap, refused, a bad
/// accept or anything that isn't an HTTP answer is `notObs`; the socket
/// closing under it is `closed`.
fn upgrade_reason(e: &tokio_websockets::Error) -> ObsReason {
    match e {
        tokio_websockets::Error::Io(io) if io.kind() != io::ErrorKind::InvalidData => {
            ObsReason::Closed
        }
        _ => ObsReason::NotObs,
    }
}

fn retry(reason: ObsReason) -> Attempt {
    Attempt::Failed(Outcome::Retry(reason, None))
}

fn stop(stop: Stop) -> Attempt {
    Attempt::Failed(Outcome::Stop(stop))
}

// ---- The driver (§5.5) --------------------------------------------------------------

/// What one link generation keeps across its connections.
struct Driver {
    cfg: LinkConfig,
    gen: u64,
    status: Arc<dyn StatusSink>,
    slots: Arc<dyn SlotStore>,
    timers: Arc<dyn TimerControl>,
    password: Arc<dyn PasswordSource>,
    feed: watch::Receiver<Arc<ObsFeed>>,
    /// `false` once the timer thread's sender is gone: the feed is never
    /// waited on again (D39).
    feed_open: bool,
    /// `None` once its sender is gone (it never comes back).
    ops: Option<mpsc::Receiver<LinkOp>>,
    cmds: mpsc::Receiver<LinkCmd>,
    timing: LinkTiming,
    rand: fn() -> f64,
    spawner: Handle,
    /// Sent once the link has stopped; handed to a session that closes.
    ack: Option<SyncSender<()>>,
    /// A newer generation runs: no more side effects (D4).
    stale: bool,
    /// D14: failures in a row.
    failures: u32,
    /// The last Hello: whether it asked for auth, and its version,
    /// sanitized (D15).
    hello: Option<(bool, Option<String>)>,
    /// D23: switch backs OBS answered 207: `notReady` if their grace ends.
    busy_backs: HashSet<String>,
}

/// The driver (§5.5): runs until `deps.cmds` closes, then acks on
/// `deps.closed_ack` — at once with no connection open, else from the
/// session after D5's close.
pub async fn run(deps: LinkDeps) {
    let mut driver = Driver::new(deps);
    driver.drive().await;
    if let Some(ack) = driver.ack.take() {
        let _ = ack.try_send(());
    }
}

impl Driver {
    fn new(deps: LinkDeps) -> Self {
        let LinkDeps {
            cfg,
            gen,
            status,
            slots,
            timers,
            password,
            feed,
            ops,
            cmds,
            timing,
            rand,
            spawner,
            closed_ack,
        } = deps;
        Self {
            cfg,
            gen,
            status,
            slots,
            timers,
            password,
            feed,
            feed_open: true,
            ops: Some(ops),
            cmds,
            timing,
            rand,
            spawner,
            ack: Some(closed_ack),
            stale: false,
            failures: 0,
            hello: None,
            busy_backs: HashSet::new(),
        }
    }

    /// D6's "first ready of": the commands (always, first), the
    /// connection's events, answer and end (per `watch`), `step`, the ops,
    /// the feed, and `until`. Each source that has ended stays out (D39).
    async fn first_of<F: Future>(
        &mut self,
        mut conn: Option<&mut Conn>,
        mut step: Pin<&mut F>,
        watch: Watch,
        until: Option<Instant>,
    ) -> Got<F::Output> {
        let feed_on = watch.feed() && self.feed_open;
        let feed = &mut self.feed;
        let mut changed = pin!(async move {
            if feed_on {
                feed.changed().await.is_ok()
            } else {
                pending().await
            }
        });
        let mut timer = pin!(async move {
            match until {
                Some(at) => sleep_until(tokio::time::Instant::from_std(at)).await,
                None => pending().await,
            }
        });
        let cmds = &mut self.cmds;
        let ops = &mut self.ops;
        poll_fn(|cx| {
            count_poll();
            if let Poll::Ready(cmd) = cmds.poll_recv(cx) {
                return Poll::Ready(cmd.map_or(Got::Stop, Got::Cmd));
            }
            if let (true, Some(c)) = (watch.session(), conn.as_deref_mut()) {
                if let Some(got) = poll_session(c, watch.reply(), cx) {
                    return Poll::Ready(got);
                }
            }
            if let Poll::Ready(out) = step.as_mut().poll(cx) {
                return Poll::Ready(Got::Step(out));
            }
            if watch.ops() {
                if let Some(rx) = ops.as_mut() {
                    match rx.poll_recv(cx) {
                        Poll::Ready(Some(op)) => return Poll::Ready(Got::Op(op)),
                        Poll::Ready(None) => *ops = None,
                        Poll::Pending => {}
                    }
                }
            }
            if let Poll::Ready(open) = changed.as_mut().poll(cx) {
                return Poll::Ready(Got::Feed(open));
            }
            if timer.as_mut().poll(cx).is_ready() {
                return Poll::Ready(Got::Timer);
            }
            Poll::Pending
        })
        .await
    }

    /// [`first_of`](Self::first_of) with no step.
    async fn wait(
        &mut self,
        conn: Option<&mut Conn>,
        watch: Watch,
        until: Option<Instant>,
    ) -> Got<Infallible> {
        self.first_of(conn, pin!(pending::<Infallible>()), watch, until)
            .await
    }

    /// Whether this is still the newest generation (D4); once stale, for
    /// good.
    fn fresh(&mut self) -> bool {
        if !self.stale && self.status.current_gen() != self.gen {
            self.stale = true;
        }
        !self.stale
    }

    /// A status write, unless stale; a write the cell refuses makes it so.
    fn write(&mut self, mut f: impl FnMut(&mut ObsStatus)) -> bool {
        if self.stale {
            return false;
        }
        if !self.status.write(self.gen, &mut f) {
            self.stale = true;
        }
        !self.stale
    }

    /// §7.5: a change of state, which clears what belonged to a connection
    /// and carries what the last Hello said.
    fn write_state(
        &mut self,
        state: ObsState,
        reason: Option<ObsReason>,
        detail: Option<&'static str>,
        retry_at_ms: Option<i64>,
    ) -> bool {
        let hello = self.hello.clone();
        self.write(|s| {
            s.state = state;
            s.reason = reason;
            s.detail = detail;
            s.retry_at_ms = retry_at_ms;
            s.stream = None;
            s.record = None;
            s.missing.clear();
            if let Some((auth, version)) = &hello {
                s.auth_required = Some(*auth);
                s.websocket_version.clone_from(version);
            }
        })
    }

    /// §7.5: a scene skip (replacing the last).
    fn skip(&mut self, op: SkipOp, why: SkipWhy, name: Option<&str>, timer: &str) {
        let at_ms = wall_ms();
        self.write(|s| {
            s.last_skip = Some(LastSkip {
                op,
                why,
                name: name.map(str::to_string),
                timer: timer.to_string(),
                at_ms,
            });
        });
    }

    /// A switch that went through clears the last scene skip.
    fn clear_skip(&mut self) {
        self.write(|s| s.last_skip = None);
    }

    /// §5.5's *not connected*: a `scene` is a skip (when the link is
    /// switched on; it is never run late, D23), a `sceneBack` due with its
    /// stamp.
    fn not_connected(&mut self, op: LinkOp) {
        match op.op {
            ObsOp::Scene { scene_name, .. } => {
                log::op_dropped(&op.timer, "scene", SkipWhy::NotConnected);
                if self.status.enabled() {
                    self.skip(
                        SkipOp::Scene,
                        SkipWhy::NotConnected,
                        Some(&scene_name),
                        &op.timer,
                    );
                }
            }
            ObsOp::SceneBack => self.due_again(&op.timer, Some(op.at)),
        }
    }

    /// D23: `timer`'s switch back is due (again), keeping its first stamp:
    /// `stamp` for one from the queue, the one the slots kept for a due
    /// mark taken. Never once OBS is switched off (`off` drops due marks).
    fn due_again(&mut self, timer: &str, stamp: Option<Instant>) {
        if self.status.enabled() {
            self.slots
                .mark_due(timer, stamp.unwrap_or_else(Instant::now));
        }
    }

    /// D23: due marks whose grace ran out: logged, and each a `lastSkip
    /// {sceneBack, notConnected}` — or `notReady` when OBS kept answering
    /// 207 — while the link is switched on.
    fn expired(&mut self, timers: Vec<String>) {
        for timer in timers {
            log::scene_back_expired(&timer);
            let why = if self.busy_backs.remove(&timer) {
                SkipWhy::NotReady
            } else {
                SkipWhy::NotConnected
            };
            if self.status.enabled() {
                self.skip(SkipOp::SceneBack, why, None, &timer);
            }
        }
    }

    /// A source a wait outside a connection returned: the stop or
    /// staleness end the attempt (`Err`); an op is *not connected*; a
    /// command is refused.
    fn offline<T>(&mut self, got: Got<T>) -> Result<(), Attempt> {
        match got {
            Got::Stop => return Err(Attempt::Gone),
            Got::Op(op) => self.not_connected(op),
            Got::Cmd(cmd) => refuse(cmd),
            Got::Feed(false) => self.feed_open = false,
            _ => {}
        }
        if self.stale {
            Err(Attempt::Gone)
        } else {
            Ok(())
        }
    }

    // ---- Attempts, backoff, stop states (D13, D14, §7.1) ------------------

    /// Connect, wait out a failure, again, until the stop.
    async fn drive(&mut self) {
        loop {
            let outcome = match self.attempt().await {
                Attempt::Gone => return,
                Attempt::Failed(outcome) => outcome,
                Attempt::Identified(ws) => {
                    let identified = Instant::now();
                    let Some(outcome) = self.connection(*ws).await else {
                        return;
                    };
                    // D14: a connection that held resets the run.
                    if identified.elapsed() >= self.timing.reset_after {
                        self.failures = 0;
                    }
                    outcome
                }
            };
            let again = match outcome {
                Outcome::Retry(reason, kind) => self.retry(reason, kind).await,
                Outcome::Stop(stop) => {
                    self.park(stop).await;
                    false
                }
            };
            if !again || !self.write_state(ObsState::Connecting, None, None, None) {
                return;
            }
        }
    }

    /// §7.3: `retrying` until the backoff's attempt, or Try again's `Wake`
    /// (an attempt now, the count kept). Ops are *not connected*; due marks
    /// expire at their grace. `false`: stopped or stale.
    async fn retry(&mut self, reason: ObsReason, kind: Option<io::ErrorKind>) -> bool {
        self.failures = self.failures.saturating_add(1);
        let delay = backoff(self.failures, (self.rand)(), self.timing.backoff_unit);
        let at = after(delay);
        let at_ms = wall_ms().saturating_add(ms_i64(delay));
        if !self.write_state(ObsState::Retrying, Some(reason), None, Some(at_ms)) {
            return false;
        }
        log::retrying(self.gen, reason, self.failures, kind);
        loop {
            let until = self.slots.next_expiry().map_or(at, |e| e.min(at));
            match self.wait(None, Watch::Offline, Some(until)).await {
                Got::Stop => return false,
                Got::Cmd(LinkCmd::Wake) => return true,
                Got::Timer if Instant::now() >= at => return true,
                Got::Timer => {
                    let expired = self.slots.expire_due(Instant::now());
                    self.expired(expired);
                }
                got => {
                    if self.offline(got).is_err() {
                        return false;
                    }
                }
            }
            if self.stale {
                return false;
            }
        }
    }

    /// §5.5 step 2: a stop state, parked until the stop: ops are *not
    /// connected*, due marks stay and expire at their grace (D23). A restart
    /// is a new generation (D4), so this driver only ends.
    async fn park(&mut self, stop: Stop) {
        let (state, reason, detail) = stop_fields(stop);
        if !self.write_state(state, reason, detail, None) {
            return;
        }
        log::stopped_trying(self.gen, state);
        loop {
            let until = self.slots.next_expiry();
            match self.wait(None, Watch::Offline, until).await {
                Got::Timer => {
                    let expired = self.slots.expire_due(Instant::now());
                    self.expired(expired);
                }
                got => {
                    if self.offline(got).is_err() {
                        return;
                    }
                }
            }
        }
    }

    // ---- Connect and handshake (D7, D8, D11, D12) ----------------------------

    /// One attempt: TCP, the upgrade, Hello, the gate, Identify, Identified.
    async fn attempt(&mut self) -> Attempt {
        let (ws, until) = match self.dial().await {
            Ok(dialed) => dialed,
            Err(attempt) => return attempt,
        };
        self.handshake(ws, until).await
    }

    /// D7: the TCP connect to the IP literal (never a name) within
    /// `connect`, no Nagle, then the upgrade on the capped stream within
    /// `handshake`, with the 4 MiB frame cap. The upgrade's deadline goes on
    /// to cover Hello.
    async fn dial(&mut self) -> Result<(Ws, Instant), Attempt> {
        let addr = SocketAddr::new(self.cfg.ip, self.cfg.port);
        let until = after(self.timing.connect);
        let mut dial = pin!(TcpStream::connect(addr));
        let tcp = loop {
            match self
                .first_of(None, dial.as_mut(), Watch::Offline, Some(until))
                .await
            {
                Got::Step(Ok(tcp)) => break tcp,
                Got::Step(Err(e)) => {
                    let kind = e.kind();
                    return Err(Attempt::Failed(Outcome::Retry(
                        connect_reason(kind),
                        Some(kind),
                    )));
                }
                Got::Timer => {
                    return Err(Attempt::Failed(Outcome::Retry(
                        ObsReason::Unreachable,
                        Some(io::ErrorKind::TimedOut),
                    )));
                }
                got => self.offline(got)?,
            }
        };
        // Nagle could hold a small frame for a delayed ACK (D7).
        let _ = tcp.set_nodelay(true);
        let until = after(self.timing.handshake);
        let Ok(builder) = ClientBuilder::new().uri(&self.cfg.uri()) else {
            return Err(retry(ObsReason::Protocol));
        };
        let builder = builder.limits(Limits::default().max_payload_len(Some(FRAME_CAP_BYTES)));
        let mut upgrade = pin!(builder.connect_on(Capped::new(tcp)));
        let mut ws = loop {
            match self
                .first_of(None, upgrade.as_mut(), Watch::Offline, Some(until))
                .await
            {
                Got::Step(Ok((ws, _))) => break ws,
                Got::Step(Err(e)) => return Err(retry(upgrade_reason(&e))),
                Got::Timer => return Err(retry(ObsReason::NoAnswer)),
                got => self.offline(got)?,
            }
        };
        ws.get_mut().lift();
        Ok((ws, until))
    }

    /// D11, D12: Hello by `until`, the gate before any Identify, the
    /// password only when Hello asks, then Identify (RPC 1, mask 69) and
    /// Identified within `identify` (which counts the password's load too).
    /// A failure after the upgrade closes with 1000; a stop drops the socket
    /// (nothing is sent before Identified).
    async fn handshake(&mut self, mut ws: Ws, until: Instant) -> Attempt {
        let hello = match self.read_hello(&mut ws, until).await {
            Ok(hello) => hello,
            Err(attempt) => return self.give_up(ws, attempt).await,
        };
        let version = hello.ws_version.as_str().and_then(sanitize_version);
        self.hello = Some((hello.auth.is_some(), version));
        if let Err(stopped) = gate(&hello) {
            return self.give_up(ws, stop(stopped)).await;
        }
        let until = after(self.timing.identify);
        let auth = match hello.auth {
            None => None,
            Some(challenge) => match self.auth(challenge, until).await {
                Ok(auth) => Some(auth),
                Err(attempt) => return self.give_up(ws, attempt).await,
            },
        };
        let identify = identify_json(auth.as_ref().map(|a| a.as_str()));
        // Divora's copy of the auth string goes now; the frame is the
        // crate's.
        drop(auth);
        match self.identify(&mut ws, identify, until).await {
            Ok(()) => Attempt::Identified(Box::new(ws)),
            Err(attempt) => self.give_up(ws, attempt).await,
        }
    }

    /// The handshake failed: a stop or staleness drops the socket at once;
    /// anything else closes it (1000) within the close's bounds first.
    async fn give_up(&mut self, mut ws: Ws, attempt: Attempt) -> Attempt {
        if !matches!(attempt, Attempt::Gone) {
            let timing = SessionTiming::of(&self.timing);
            close(&mut ws, CloseCode::NORMAL_CLOSURE, timing).await;
        }
        attempt
    }

    /// The next data frame by `until`, control frames skipped; a close is
    /// §7.1's outcome for its code.
    async fn next_frame(&mut self, ws: &mut Ws, until: Instant) -> Result<Message, Attempt> {
        loop {
            let mut next = pin!(poll_fn(|cx| Pin::new(&mut *ws).poll_next(cx)));
            match self
                .first_of(None, next.as_mut(), Watch::Offline, Some(until))
                .await
            {
                Got::Step(None) => return Err(retry(ObsReason::Closed)),
                Got::Step(Some(Err(e))) => return Err(retry(read_reason(&e))),
                Got::Step(Some(Ok(msg))) => {
                    if let Some((code, _)) = msg.as_close() {
                        return Err(Attempt::Failed(closed_with(self.gen, Some(code.into()))));
                    }
                    if !msg.is_ping() && !msg.is_pong() {
                        return Ok(msg);
                    }
                }
                Got::Timer => return Err(retry(ObsReason::NoAnswer)),
                got => self.offline(got)?,
            }
        }
    }

    /// D11: the first data frame must be a Hello (`notObs` otherwise).
    async fn read_hello(&mut self, ws: &mut Ws, until: Instant) -> Result<Hello, Attempt> {
        let msg = self.next_frame(ws, until).await?;
        msg.as_text()
            .and_then(|text| parse_hello(text).ok())
            .ok_or_else(|| retry(ObsReason::NotObs))
    }

    /// D12: the stored password, decrypted on the blocking pool and turned
    /// into the auth string there, so the plaintext and the secret never
    /// leave that closure and are wiped in it. A stop abandons the job: its
    /// result is wiped when dropped.
    async fn auth(
        &mut self,
        challenge: AuthChallenge,
        until: Instant,
    ) -> Result<Zeroizing<String>, Attempt> {
        let source = Arc::clone(&self.password);
        let mut job = blocking(&self.spawner, move || {
            source.load().map(|password| {
                password.map(|pw| auth_string(&pw, &challenge.salt, &challenge.challenge))
            })
        });
        loop {
            match self
                .first_of(None, Pin::new(&mut job), Watch::Offline, Some(until))
                .await
            {
                Got::Step(Ok(Ok(Some(auth)))) => return Ok(auth),
                Got::Step(Ok(Ok(None))) => return Err(stop(Stop::NeedsPassword)),
                Got::Step(Ok(Err(Unreadable)) | Err(_)) => {
                    return Err(stop(Stop::PasswordUnreadable));
                }
                Got::Timer => return Err(retry(ObsReason::NoAnswer)),
                got => self.offline(got)?,
            }
        }
    }

    /// Identify, then Identified by `until`; any other frame before it is
    /// skipped.
    async fn identify(&mut self, ws: &mut Ws, json: String, until: Instant) -> Result<(), Attempt> {
        {
            let mut sent = pin!(send_text(ws, json));
            loop {
                match self
                    .first_of(None, sent.as_mut(), Watch::Offline, Some(until))
                    .await
                {
                    Got::Step(Ok(())) => break,
                    Got::Step(Err(e)) => return Err(retry(read_reason(&e))),
                    Got::Timer => return Err(retry(ObsReason::NoAnswer)),
                    got => self.offline(got)?,
                }
            }
        }
        loop {
            let msg = self.next_frame(ws, until).await?;
            if matches!(
                msg.as_text().map(parse_incoming),
                Some(Incoming::Identified)
            ) {
                return Ok(());
            }
        }
    }
}

// ---- One connection (§5.5's connected loop) -----------------------------------------

/// A request that timed out and is still outstanding (D9): nothing else is
/// sent until its late answer or the late-answer cap.
struct Wedge {
    sent: Instant,
    /// A push's `(uuid, text)`: its late success is still recorded (§7.13).
    push: Option<(String, String)>,
}

/// D19: `obs_inventory`'s run, one request per idle turn: the scene list,
/// the input list, then each text source's settings for its file flag.
struct Inventory {
    /// Every caller waiting; a second `obs_inventory` joins.
    repliers: Vec<InventoryReply>,
    began: Instant,
    /// After a 207: not before this.
    hold: Option<Instant>,
    /// The program scene and the scenes, top first, once read.
    scenes: Option<(Option<SceneRef>, Vec<SceneRef>)>,
    sources: Option<Vec<TextSourceRef>>,
    /// Each source's "reads a file", in `sources`' order, as read so far.
    flags: Vec<bool>,
    truncated: bool,
}

impl Inventory {
    fn new(reply: InventoryReply) -> Self {
        Self {
            repliers: vec![reply],
            began: Instant::now(),
            hold: None,
            scenes: None,
            sources: None,
            flags: Vec::new(),
            truncated: false,
        }
    }

    /// The next step's request; `None`: every step is done.
    fn next_request(&self) -> Option<Request> {
        if self.scenes.is_none() {
            return Some(Request::GetSceneList);
        }
        let Some(sources) = &self.sources else {
            return Some(Request::GetInputList);
        };
        sources
            .get(self.flags.len())
            .map(|s| Request::GetInputSettings {
                uuid: s.uuid.clone(),
            })
    }

    /// The text source whose settings the next step reads.
    fn next_source(&self) -> Option<&TextSourceRef> {
        self.sources.as_ref()?.get(self.flags.len())
    }

    /// A step's answer. Nothing from a source's settings is kept but its
    /// file flag (D19).
    fn take(&mut self, kind: RequestKind, data: &Value) {
        match kind {
            RequestKind::GetSceneList => {
                let (program, mut scenes) = proto::scene_list(data);
                if scenes.len() > MAX_SCENES {
                    scenes.truncate(MAX_SCENES);
                    self.truncated = true;
                }
                self.scenes = Some((program, scenes));
            }
            RequestKind::GetInputList => {
                let mut sources = proto::input_list(data);
                if sources.len() > MAX_TEXT_SOURCES {
                    sources.truncate(MAX_TEXT_SOURCES);
                    self.truncated = true;
                }
                self.sources = Some(sources);
            }
            _ => {
                let reads = self.next_source().is_some_and(|s| {
                    reads_from_file(s.kind, data.get("inputSettings").unwrap_or(&Value::Null))
                });
                self.flags.push(reads);
            }
        }
    }

    /// Every step done: the answer, at `rev`.
    fn done(&self, rev: u64) -> Option<ObsInventory> {
        if self.next_request().is_some() {
            return None;
        }
        let (program, scenes) = self.scenes.clone()?;
        let text_sources = self
            .sources
            .iter()
            .flatten()
            .zip(&self.flags)
            .map(|(s, &reads_from_file)| TextSource {
                uuid: s.uuid.clone(),
                name: s.name.clone(),
                kind: s.kind,
                reads_from_file,
            })
            .collect();
        Some(ObsInventory {
            rev,
            scenes,
            text_sources,
            program_scene_uuid: program.map(|p| p.uuid),
            truncated: self.truncated,
        })
    }

    /// Every caller gets `answer`.
    fn answer(self, answer: &Result<ObsInventory, String>) {
        for reply in self.repliers {
            let _ = reply.send(answer.clone());
        }
    }
}

/// What a request got.
enum Reply {
    /// Success, with `responseData`.
    Ok(Option<Value>),
    /// An error answer's code.
    Code(u64),
    /// No answer in time: the connection is wedged until the late answer
    /// or the cap (D9).
    TimedOut,
}

/// One Identified connection: the session's ends, and what the driver
/// knows of OBS on it. A new connection starts from nothing (D18).
struct Conn {
    out: mpsc::UnboundedSender<Out>,
    events: mpsc::Receiver<Stamped>,
    end: Option<oneshot::Receiver<SessionEnd>>,
    /// The outstanding request's answer.
    reply: Option<oneshot::Receiver<Answer>>,
    wedge: Option<Wedge>,
    next_id: u64,
    /// `connected` is published.
    live: bool,
    /// D8: OBS said `ExitStarted`; its next 207 ends the connection.
    exiting: bool,
    /// The connection is to end as this before the next request.
    end_now: Option<Outcome>,
    /// Ops taken while a request was outstanding, run next.
    next_ops: VecDeque<LinkOp>,
    stream: Option<StreamState>,
    record: Option<RecordState>,
    /// D18: what OBS confirmed on this connection.
    pushed: Pushed,
    /// D20: sources OBS can't find, and since when.
    missing: HashMap<String, Instant>,
    /// D18: failed pushes `uuid → text`, not sent again with that text
    /// until the retry.
    held: HashMap<String, String>,
    push_fails: u32,
    push_retry: Option<Instant>,
    /// D26: follows to seed, by `(timer id, what)`.
    seeds: Vec<(String, FollowObs)>,
    seeds_hold: Option<Instant>,
    /// The feed the last turn read: new follows are seeded against it.
    prev_feed: Arc<ObsFeed>,
    rev: RevCoalesce,
    /// `inventoryRev` as last written.
    inventory_rev: u64,
    inventory: Option<Inventory>,
    /// D23: after a switch back's 207, due marks wait for this.
    due_hold: Option<Instant>,
}

impl Conn {
    fn new(
        out: mpsc::UnboundedSender<Out>,
        events: mpsc::Receiver<Stamped>,
        end: oneshot::Receiver<SessionEnd>,
        rev_coalesce: Duration,
    ) -> Self {
        Self {
            out,
            events,
            end: Some(end),
            reply: None,
            wedge: None,
            next_id: 0,
            live: false,
            exiting: false,
            end_now: None,
            next_ops: VecDeque::new(),
            stream: None,
            record: None,
            pushed: Pushed::default(),
            missing: HashMap::new(),
            held: HashMap::new(),
            push_fails: 0,
            push_retry: None,
            seeds: Vec::new(),
            seeds_hold: None,
            prev_feed: Arc::default(),
            rev: RevCoalesce::new(rev_coalesce),
            inventory_rev: 0,
            inventory: None,
            due_hold: None,
        }
    }

    /// A command while connected: an inventory starts or joins the run
    /// (D19), once `connected` is published; a wake does nothing.
    fn on_cmd(&mut self, cmd: LinkCmd) {
        match cmd {
            LinkCmd::Wake => {}
            LinkCmd::Inventory(reply) if self.live => match self.inventory.as_mut() {
                Some(run) => run.repliers.push(reply),
                None => self.inventory = Some(Inventory::new(reply)),
            },
            LinkCmd::Inventory(reply) => refuse(LinkCmd::Inventory(reply)),
        }
    }

    /// D26: a seed for `(id, what)`, once.
    fn queue_seed(&mut self, id: &str, what: FollowObs) {
        if !self.seeds.iter().any(|(i, w)| i == id && *w == what) {
            self.seeds.push((id.to_string(), what));
        }
    }

    /// The earliest of the connected loop's own timers past `now`.
    fn next_timer(&self, now: Instant, recheck: Duration) -> Option<Instant> {
        let recheck_at = self
            .missing
            .values()
            .min()
            .and_then(|at| at.checked_add(recheck));
        [
            self.rev.due_at(now),
            self.push_retry,
            recheck_at,
            self.due_hold.filter(|t| *t > now),
            self.seeds_hold
                .filter(|t| *t > now && !self.seeds.is_empty()),
            self.inventory
                .as_ref()
                .and_then(|i| i.hold)
                .filter(|t| *t > now),
        ]
        .into_iter()
        .flatten()
        .min()
    }
}

impl Driver {
    /// §5.5 step 4: the session spawned on the socket, the connected loop
    /// run until the connection ends, then the session closed. `None`: the
    /// link stopped or went stale (the session acks after its close).
    async fn connection(&mut self, ws: Ws) -> Option<Outcome> {
        let (out, out_rx) = mpsc::unbounded_channel();
        let (events_tx, events) = mpsc::channel(EVENTS_QUEUE);
        let (end_tx, end) = oneshot::channel();
        drop(self.spawner.spawn(session(
            ws,
            out_rx,
            events_tx,
            end_tx,
            SessionTiming::of(&self.timing),
        )));
        let mut conn = Conn::new(out, events, end, self.timing.rev_coalesce);
        let exit = self.connected(&mut conn).await;
        match exit {
            Exit::Stopped | Exit::Stale => {
                // D5: the session sends 1001 and then acks; if it had already
                // gone, the driver acks.
                let ack = self.ack.take();
                if let Err(SendError(Out::Close(ack))) = conn.out.send(Out::Close(ack)) {
                    self.ack = ack;
                }
                None
            }
            Exit::Ended(end) => Some(self.end_outcome(&conn, end)),
            Exit::Drop(outcome) => {
                let _ = conn.out.send(Out::Close(None));
                if let Some(end) = conn.end.take() {
                    let bound = self
                        .timing
                        .close_flush
                        .saturating_mul(2)
                        .saturating_add(self.timing.close_echo);
                    let _ = timeout(bound, end).await;
                }
                Some(outcome)
            }
        }
    }

    /// §7.1: how a session's own end reads; `closed` after `ExitStarted` is
    /// `obsExiting`.
    fn end_outcome(&self, conn: &Conn, end: SessionEnd) -> Outcome {
        let outcome = match end {
            SessionEnd::Closed(code) => closed_with(self.gen, code),
            SessionEnd::TooBig => Outcome::Retry(ObsReason::TooBig, None),
            SessionEnd::Protocol | SessionEnd::Flooded => Outcome::Retry(ObsReason::Protocol, None),
            SessionEnd::Silent => Outcome::Retry(ObsReason::Silent, None),
            SessionEnd::Ours => Outcome::Retry(ObsReason::Closed, None),
        };
        match outcome {
            Outcome::Retry(ObsReason::Closed, kind) if conn.exiting => {
                Outcome::Retry(ObsReason::ObsExiting, kind)
            }
            other => other,
        }
    }

    /// The startup steps, `connected`, then one turn at a time until the
    /// connection ends.
    async fn connected(&mut self, conn: &mut Conn) -> Exit {
        if let Err(exit) = self.startup(conn).await {
            return exit;
        }
        loop {
            if let Some(outcome) = conn.end_now.take() {
                return Exit::Drop(outcome);
            }
            if !self.fresh() {
                return Exit::Stale;
            }
            let turn = if conn.wedge.is_some() {
                self.unwedge(conn).await
            } else {
                self.turn(conn).await
            };
            if let Err(exit) = turn {
                return exit;
            }
        }
    }

    /// A source a connected wait returned, besides the step and the answer:
    /// the stop, a command, an event, the session's end, the feed.
    fn connected_got<T>(&mut self, conn: &mut Conn, got: Got<T>) -> Result<(), Exit> {
        match got {
            Got::Stop => return Err(Exit::Stopped),
            Got::End(end) => return Err(Exit::Ended(end)),
            Got::Cmd(cmd) => conn.on_cmd(cmd),
            Got::Event(e) => self.on_event(conn, &e),
            Got::Op(op) => conn.next_ops.push_back(op),
            Got::Feed(open) => self.feed_open &= open,
            Got::Reply(_) | Got::Timer | Got::Step(_) => {}
        }
        if self.stale {
            Err(Exit::Stale)
        } else {
            Ok(())
        }
    }

    /// The session dropped an answer or the out channel: its end, which is
    /// on its way.
    async fn session_gone(&mut self, conn: &mut Conn) -> Exit {
        loop {
            if conn.end.is_none() {
                return Exit::Ended(SessionEnd::Closed(None));
            }
            match self.wait(Some(&mut *conn), Watch::Pause, None).await {
                Got::End(end) => return Exit::Ended(end),
                got => {
                    if let Err(exit) = self.connected_got(conn, got) {
                        return exit;
                    }
                }
            }
        }
    }

    /// D9: one request, its answer within the request timeout. Events,
    /// commands and the session's end are handled meanwhile. A timeout
    /// leaves it outstanding (the connection wedged).
    async fn request(&mut self, conn: &mut Conn, req: &Request) -> Result<Reply, Exit> {
        if let Some(outcome) = conn.end_now {
            return Err(Exit::Drop(outcome));
        }
        if !self.fresh() {
            return Err(Exit::Stale);
        }
        conn.next_id = conn.next_id.wrapping_add(1);
        let id = format!("divora-{}", conn.next_id);
        let (reply, answer) = oneshot::channel();
        let json = req.request_json(&id);
        if conn.out.send(Out::Request { id, json, reply }).is_err() {
            return Err(self.session_gone(conn).await);
        }
        conn.reply = Some(answer);
        let sent = Instant::now();
        let until = sent
            .checked_add(self.cfg.request_timeout(&self.timing))
            .unwrap_or(sent);
        loop {
            match self
                .wait(Some(&mut *conn), Watch::Answer, Some(until))
                .await
            {
                Got::Reply(Some(a)) => return Ok(Self::answered(conn, a)),
                Got::Reply(None) => return Err(self.session_gone(conn).await),
                Got::Timer if Instant::now() >= until => {
                    log::request_timed_out(self.gen, req.request_type());
                    conn.wedge = Some(Wedge { sent, push: None });
                    return Ok(Reply::TimedOut);
                }
                got => self.connected_got(conn, got)?,
            }
        }
    }

    /// An answer as a [`Reply`]. D8: a 207 after `ExitStarted` ends the
    /// connection (`obsExiting`) before the next request.
    fn answered(conn: &mut Conn, a: Answer) -> Reply {
        if a.ok {
            return Reply::Ok(a.data);
        }
        if a.code == NOT_READY && conn.exiting {
            conn.end_now = Some(Outcome::Retry(ObsReason::ObsExiting, None));
        }
        Reply::Code(a.code)
    }

    /// D9: wedged: nothing is sent until the late answer — used only for a
    /// push's success — or the late-answer cap, which ends the connection
    /// (`noAnswer`).
    async fn unwedge(&mut self, conn: &mut Conn) -> Result<(), Exit> {
        let Some(wedge) = conn.wedge.take() else {
            return Ok(());
        };
        let until = wedge
            .sent
            .checked_add(self.timing.late_answer_cap)
            .unwrap_or(wedge.sent);
        loop {
            match self
                .wait(Some(&mut *conn), Watch::Answer, Some(until))
                .await
            {
                Got::Reply(Some(a)) => {
                    if let (true, Some((uuid, text))) = (a.ok, wedge.push) {
                        self.pushed_ok(conn, uuid, text);
                    }
                    return Ok(());
                }
                Got::Reply(None) => return Err(self.session_gone(conn).await),
                Got::Timer if Instant::now() >= until => {
                    return Err(Exit::Drop(Outcome::Retry(ObsReason::NoAnswer, None)));
                }
                got => self.connected_got(conn, got)?,
            }
        }
    }

    /// Connected, waiting `d` (a 207's wait): events and commands go on.
    async fn pause(&mut self, conn: &mut Conn, d: Duration) -> Result<(), Exit> {
        let until = after(d);
        loop {
            match self.wait(Some(&mut *conn), Watch::Pause, Some(until)).await {
                Got::Timer if Instant::now() >= until => return Ok(()),
                got => self.connected_got(conn, got)?,
            }
        }
    }

    // ---- Startup (D8, D11, D26) ------------------------------------------------

    /// After Identified, in order: `GetVersion` (every one of the nine
    /// offered, or `unsupported`), `GetStreamStatus`, `GetRecordStatus`;
    /// the follows seeded from those answers; then `connected`.
    async fn startup(&mut self, conn: &mut Conn) -> Result<(), Exit> {
        let began = Instant::now();
        let version = self.startup_step(conn, &Request::GetVersion, began).await?;
        let (obs_version, available) = proto::version(&version);
        if let Some(name) = REQUEST_TYPES
            .iter()
            .find(|t| !available.iter().any(|a| a == *t))
        {
            let missing = Unsupported::MissingRequest(name);
            return Err(Exit::Drop(Outcome::Stop(Stop::Unsupported(missing))));
        }
        let stream = proto::stream_status(
            &self
                .startup_step(conn, &Request::GetStreamStatus, began)
                .await?,
        );
        let record = proto::record_status(
            &self
                .startup_step(conn, &Request::GetRecordStatus, began)
                .await?,
        );
        conn.stream = Some(stream_status_word(stream));
        conn.record = Some(record_status_word(record));
        let feed = self.take_feed(conn);
        for follow in &feed.follows {
            let status = match follow.what {
                FollowObs::Stream => stream,
                FollowObs::Record => record,
            };
            if let Some(op) = join_seed(follow.what, status, true) {
                self.apply(&follow.id, op)?;
            }
        }
        self.go_live(conn, obs_version.as_deref())
    }

    /// D8, §7.13: a startup step, sent again after `not_ready_wait` on a
    /// 207 for at most `not_ready_cap` from the first step (`notReady`);
    /// any other error answer is `protocol`, a timeout `noAnswer`.
    async fn startup_step(
        &mut self,
        conn: &mut Conn,
        req: &Request,
        began: Instant,
    ) -> Result<Value, Exit> {
        loop {
            match self.request(conn, req).await? {
                Reply::Ok(data) => return Ok(data.unwrap_or(Value::Null)),
                Reply::TimedOut => {
                    return Err(Exit::Drop(Outcome::Retry(ObsReason::NoAnswer, None)));
                }
                Reply::Code(code) => {
                    if let Some(outcome) = conn.end_now {
                        return Err(Exit::Drop(outcome));
                    }
                    if request_failure(Work::Startup, req.kind(), code) != Failure::WaitAndRetry {
                        return Err(Exit::Drop(Outcome::Retry(ObsReason::Protocol, None)));
                    }
                    if began.elapsed() >= self.timing.not_ready_cap {
                        return Err(Exit::Drop(Outcome::Retry(ObsReason::NotReady, None)));
                    }
                    self.pause(conn, self.timing.not_ready_wait).await?;
                }
            }
        }
    }

    /// §7.5: `connected`, with the versions, the output words and
    /// `inventoryRev` + 1.
    fn go_live(&mut self, conn: &mut Conn, obs_version: Option<&str>) -> Result<(), Exit> {
        let obs = obs_version.and_then(sanitize_version);
        let (auth, websocket) = self.hello.clone().unwrap_or_default();
        let (stream, record, exiting) = (conn.stream, conn.record, conn.exiting);
        let mut rev = 0;
        let live = self.write(|s| {
            s.state = ObsState::Connected;
            s.reason = exiting.then_some(ObsReason::ObsExiting);
            s.detail = None;
            s.retry_at_ms = None;
            s.stream = stream;
            s.record = record;
            s.missing.clear();
            s.auth_required = Some(auth);
            s.obs_version.clone_from(&obs);
            s.websocket_version.clone_from(&websocket);
            s.inventory_rev = s.inventory_rev.saturating_add(1);
            rev = s.inventory_rev;
        });
        if !live {
            return Err(Exit::Stale);
        }
        conn.live = true;
        conn.inventory_rev = rev;
        log::connected(self.gen, obs.as_deref(), websocket.as_deref());
        Ok(())
    }

    // ---- The idle turn (§5.5, §8.2) ---------------------------------------------

    /// One idle turn: at most one request, the first work there is, in
    /// order: an op, a due switch back, a follow seed, one text push, one
    /// inventory step; with none, a wait for anything that makes some.
    async fn turn(&mut self, conn: &mut Conn) -> Result<(), Exit> {
        let now = Instant::now();
        self.timers_due(conn, now);
        let queued = conn
            .next_ops
            .pop_front()
            .or_else(|| self.ops.as_mut().and_then(|rx| rx.try_recv().ok()));
        if let Some(op) = queued {
            return self.run_op(conn, op).await;
        }
        if conn.due_hold.is_some_and(|t| t > now) {
            let expired = self.slots.expire_due(now);
            self.expired(expired);
        } else {
            let (next, expired) = self.slots.take_due(now);
            self.expired(expired);
            if let Some(timer) = next {
                return self.scene_back(conn, &timer, None).await;
            }
        }
        let feed = self.take_feed(conn);
        if !conn.seeds.is_empty() && conn.seeds_hold.is_none_or(|t| t <= now) {
            return self.seed(conn).await;
        }
        if let Some((uuid, text)) = self.next_text(conn, &feed) {
            return self.push(conn, uuid, text).await;
        }
        if conn
            .inventory
            .as_ref()
            .is_some_and(|i| i.hold.is_none_or(|t| t <= now))
        {
            return self.inventory_step(conn).await;
        }
        let until = [
            conn.next_timer(now, self.timing.missing_recheck),
            self.slots.next_expiry(),
        ]
        .into_iter()
        .flatten()
        .min();
        match self.wait(Some(&mut *conn), Watch::Idle, until).await {
            Got::Op(op) => self.run_op(conn, op).await,
            got => self.connected_got(conn, got),
        }
    }

    /// The connected loop's own timers: a coalesced `inventoryRev` bump
    /// (D15), missing sources re-checked (D20), failed pushes released for
    /// their retry (D18).
    fn timers_due(&mut self, conn: &mut Conn, now: Instant) {
        if conn.rev.take(now) {
            let mut rev = conn.inventory_rev;
            if self.write(|s| {
                s.inventory_rev = s.inventory_rev.saturating_add(1);
                rev = s.inventory_rev;
            }) {
                conn.inventory_rev = rev;
            }
            // D20: a scene change may bring a source back.
            conn.missing.clear();
            self.write_missing(conn);
        }
        let recheck = self.timing.missing_recheck;
        let before = conn.missing.len();
        conn.missing
            .retain(|_, at| now.saturating_duration_since(*at) < recheck);
        if conn.missing.len() != before {
            self.write_missing(conn);
        }
        if conn.push_retry.is_some_and(|t| t <= now) {
            conn.push_retry = None;
            conn.held.clear();
        }
    }

    /// `status.missing`: the uuids, sorted.
    fn write_missing(&mut self, conn: &Conn) {
        let mut missing: Vec<String> = conn.missing.keys().cloned().collect();
        missing.sort();
        self.write(|s| s.missing.clone_from(&missing));
    }

    /// The latest feed, cloned out of the `watch` at once (D17); while
    /// connected, follows new to it are queued for a seed (D26).
    fn take_feed(&mut self, conn: &mut Conn) -> Arc<ObsFeed> {
        let next = Arc::clone(&*self.feed.borrow_and_update());
        if conn.live {
            for (id, what) in seed_pairs(&conn.prev_feed, &next) {
                conn.queue_seed(&id, what);
            }
        }
        conn.prev_feed = Arc::clone(&next);
        next
    }

    /// D26: `op` on the timer `id`, unless stale.
    fn apply(&mut self, id: &str, op: TimerOp) -> Result<(), Exit> {
        if !self.fresh() {
            return Err(Exit::Stale);
        }
        let _ = self.timers.apply(id, op);
        Ok(())
    }

    // ---- Events (D10, D15, D26) ------------------------------------------------

    /// An event, handled as it is read, in any state: output words and
    /// Follow OBS, scene changes into the coalesced `inventoryRev`,
    /// `ExitStarted` remembered.
    fn on_event(&mut self, conn: &mut Conn, e: &Stamped) {
        match &e.event {
            ObsEvent::ExitStarted => {
                conn.exiting = true;
                log::obs_exiting(self.gen);
                if conn.live {
                    self.write(|s| s.reason = Some(ObsReason::ObsExiting));
                }
            }
            ObsEvent::SceneCreated | ObsEvent::SceneRemoved | ObsEvent::SceneNameChanged => {
                conn.rev.mark();
            }
            ObsEvent::Stream(state) => {
                if let Some(word) = stream_word(*state) {
                    conn.stream = Some(word);
                    self.write_words(conn);
                }
                self.follow(conn, FollowObs::Stream, e);
            }
            ObsEvent::Record(state) => {
                if let Some(word) = record_word(*state) {
                    conn.record = Some(word);
                    self.write_words(conn);
                }
                self.follow(conn, FollowObs::Record, e);
            }
        }
    }

    /// The output words, once `connected` is published.
    fn write_words(&mut self, conn: &Conn) {
        if conn.live {
            let (stream, record) = (conn.stream, conn.record);
            self.write(|s| {
                s.stream = stream;
                s.record = record;
            });
        }
    }

    /// D26, §7.7: an output event for every count-up following that
    /// output: an op at once (a `STARTED` counts from when the event was
    /// read), or a seed queued.
    fn follow(&mut self, conn: &mut Conn, what: FollowObs, e: &Stamped) {
        let since = ms_u64(e.at.elapsed());
        let feed = Arc::clone(&*self.feed.borrow());
        for f in feed.follows.iter().filter(|f| f.what == what) {
            match follow_op(what, &e.event, since) {
                Some(FollowAction::Op(op)) => {
                    if self.apply(&f.id, op).is_err() {
                        return;
                    }
                }
                Some(FollowAction::Seed) => conn.queue_seed(&f.id, what),
                None => {}
            }
        }
    }

    /// D26: one seed request for the first queued output; every follow of
    /// that output queued takes its answer (`join_seed`). A 207 waits
    /// `not_ready_wait`; another refusal drops them, logged.
    async fn seed(&mut self, conn: &mut Conn) -> Result<(), Exit> {
        let Some(&(_, what)) = conn.seeds.first() else {
            return Ok(());
        };
        let req = match what {
            FollowObs::Stream => Request::GetStreamStatus,
            FollowObs::Record => Request::GetRecordStatus,
        };
        match self.request(conn, &req).await? {
            Reply::Ok(data) => {
                let data = data.unwrap_or(Value::Null);
                let status = match what {
                    FollowObs::Stream => proto::stream_status(&data),
                    FollowObs::Record => proto::record_status(&data),
                };
                self.set_word(conn, what, status);
                let (now, later): (Vec<_>, Vec<_>) =
                    conn.seeds.drain(..).partition(|(_, w)| *w == what);
                conn.seeds = later;
                for (id, _) in now {
                    if let Some(op) = join_seed(what, status, true) {
                        self.apply(&id, op)?;
                    }
                }
            }
            // Still queued: it goes again after the late answer.
            Reply::TimedOut => {}
            Reply::Code(NOT_READY) => conn.seeds_hold = Some(after(self.timing.not_ready_wait)),
            Reply::Code(code) => {
                log::seed_refused(self.gen, req.request_type(), code);
                conn.seeds.retain(|(_, w)| *w != what);
            }
        }
        Ok(())
    }

    /// A seed's answer as the output's word.
    fn set_word(&mut self, conn: &mut Conn, what: FollowObs, status: OutputStatus) {
        match what {
            FollowObs::Stream => conn.stream = Some(stream_status_word(status)),
            FollowObs::Record => conn.record = Some(record_status_word(status)),
        }
        self.write_words(conn);
    }

    // ---- Text pushes (D18, D20, §7.8) -------------------------------------------

    /// §7.8: the next push against `feed`, past sources OBS can't find and
    /// failed pushes waiting for their retry with the same text.
    fn next_text(&self, conn: &Conn, feed: &ObsFeed) -> Option<(String, String)> {
        if !OBS_TEXT {
            return None;
        }
        let mut skip: HashSet<String> = conn.missing.keys().cloned().collect();
        for (uuid, text) in &conn.held {
            let now = feed
                .texts
                .iter()
                .find(|t| t.uuid == *uuid)
                .map_or("", |t| t.text.as_str());
            if now == text {
                skip.insert(uuid.clone());
            }
        }
        next_push(&conn.pushed, feed, &skip, &self.slots.written())
    }

    /// One push, and what its answer means (§7.13).
    async fn push(&mut self, conn: &mut Conn, uuid: String, text: String) -> Result<(), Exit> {
        let req = Request::SetText {
            uuid: uuid.clone(),
            text: text.clone(),
        };
        match self.request(conn, &req).await? {
            Reply::Ok(_) => self.pushed_ok(conn, uuid, text),
            Reply::TimedOut => {
                self.text_skip(TextSkipWhy::TimedOut);
                self.push_failed(conn, &uuid, &text);
                if let Some(wedge) = conn.wedge.as_mut() {
                    wedge.push = Some((uuid, text));
                }
            }
            Reply::Code(code) => {
                match request_failure(Work::Push, RequestKind::SetInputSettings, code) {
                    Failure::Missing => {
                        log::source_missing(self.gen, &uuid, code);
                        conn.missing.insert(uuid, Instant::now());
                        self.write_missing(conn);
                    }
                    Failure::TextSkipFailed => {
                        log::push_refused(self.gen, code);
                        self.text_skip(TextSkipWhy::Failed);
                        self.push_failed(conn, &uuid, &text);
                    }
                    _ => {
                        log::push_refused(self.gen, code);
                        self.push_failed(conn, &uuid, &text);
                    }
                }
            }
        }
        Ok(())
    }

    /// OBS confirmed a push (§7.13): recorded, the uuid into or out of the
    /// written set, `textSkip` cleared.
    fn pushed_ok(&mut self, conn: &mut Conn, uuid: String, text: String) {
        if !self.fresh() {
            return;
        }
        self.slots.note_written(&uuid, text.is_empty());
        conn.held.remove(&uuid);
        if conn.held.is_empty() {
            conn.push_fails = 0;
            conn.push_retry = None;
        }
        conn.pushed.0.insert(uuid, text);
        self.write(|s| s.text_skip = None);
    }

    /// D18: a push that didn't land waits for the retry (the base, doubling,
    /// at most 30 times it), which runs the diff again even if the feed
    /// hasn't changed; a new text for it goes at once.
    fn push_failed(&mut self, conn: &mut Conn, uuid: &str, text: &str) {
        conn.held.insert(uuid.to_string(), text.to_string());
        conn.push_fails = conn.push_fails.saturating_add(1);
        let factor = 1_u32
            .checked_shl(conn.push_fails.saturating_sub(1))
            .unwrap_or(u32::MAX)
            .min(PUSH_RETRY_MAX);
        conn.push_retry = Some(after(self.timing.push_retry_base.saturating_mul(factor)));
    }

    fn text_skip(&mut self, why: TextSkipWhy) {
        let at_ms = wall_ms();
        self.write(|s| s.text_skip = Some(TextSkip { why, at_ms }));
    }

    // ---- Scene ops (D22, D23, §7.6, §7.13) ------------------------------------------

    /// An op from the queue: a `scene` older than `scene_max_age` is
    /// skipped (`late`), never run late (D23).
    async fn run_op(&mut self, conn: &mut Conn, op: LinkOp) -> Result<(), Exit> {
        match op.op {
            ObsOp::Scene {
                scene_uuid,
                scene_name,
            } => {
                if op.at.elapsed() > self.timing.scene_max_age {
                    log::op_dropped(&op.timer, "scene", SkipWhy::Late);
                    self.skip(SkipOp::Scene, SkipWhy::Late, Some(&scene_name), &op.timer);
                    return Ok(());
                }
                let to = SceneRef {
                    uuid: scene_uuid,
                    name: scene_name,
                };
                self.scene(conn, &op.timer, to).await
            }
            ObsOp::SceneBack => self.scene_back(conn, &op.timer, Some(op.at)).await,
        }
    }

    /// D22: `scene {to}`: the program now, the slot recorded (before the
    /// Set, so the switch back works even if a timed-out Set lands later),
    /// then the Set unless OBS is on `to` already. Cut off by the session's
    /// end: before the Set `notConnected`, after it `timedOut`.
    async fn scene(&mut self, conn: &mut Conn, timer: &str, to: SceneRef) -> Result<(), Exit> {
        let name = to.name.clone();
        let got = match self.request(conn, &Request::GetCurrentProgramScene).await {
            Ok(got) => got,
            Err(exit) => {
                if matches!(exit, Exit::Ended(_) | Exit::Drop(_)) {
                    self.skip(SkipOp::Scene, SkipWhy::NotConnected, Some(&name), timer);
                }
                return Err(exit);
            }
        };
        let cur = match got {
            Reply::Ok(data) => data.as_ref().and_then(program_scene),
            Reply::TimedOut => {
                self.skip(SkipOp::Scene, SkipWhy::TimedOut, Some(&name), timer);
                return Ok(());
            }
            Reply::Code(code) => {
                let why = scene_why(Work::Scene, RequestKind::GetCurrentProgramScene, code);
                self.skip(SkipOp::Scene, why, Some(&name), timer);
                return Ok(());
            }
        };
        let Some(cur) = cur else {
            self.skip(SkipOp::Scene, SkipWhy::Failed, Some(&name), timer);
            return Ok(());
        };
        if !self.fresh() {
            return Err(Exit::Stale);
        }
        let set = Request::SetCurrentProgramScene {
            uuid: to.uuid.clone(),
        };
        if !self.slots.after_scene(timer, cur, to) {
            self.clear_skip();
            return Ok(());
        }
        match self.request(conn, &set).await {
            Ok(Reply::Ok(_)) => self.clear_skip(),
            Ok(Reply::TimedOut) => self.skip(SkipOp::Scene, SkipWhy::TimedOut, Some(&name), timer),
            Ok(Reply::Code(code)) => {
                let why = scene_why(Work::Scene, RequestKind::SetCurrentProgramScene, code);
                self.skip(SkipOp::Scene, why, Some(&name), timer);
            }
            Err(exit) => {
                if matches!(exit, Exit::Ended(_) | Exit::Drop(_)) {
                    self.skip(SkipOp::Scene, SkipWhy::TimedOut, Some(&name), timer);
                }
                return Err(exit);
            }
        }
        Ok(())
    }

    /// D22: `sceneBack`: the program now, the plan (still on the scene the
    /// break set, or stacked, or moved on), the Set if any, and the plan
    /// committed once the Set is answered or none is needed. What can't
    /// finish (a Get timeout, a 207, the session's end or a stop before an
    /// answer) is due again with its first stamp. `stamp`: an op's from the
    /// queue; `None` for a due mark taken.
    async fn scene_back(
        &mut self,
        conn: &mut Conn,
        timer: &str,
        stamp: Option<Instant>,
    ) -> Result<(), Exit> {
        let got = match self.request(conn, &Request::GetCurrentProgramScene).await {
            Ok(got) => got,
            Err(exit) => {
                self.due_again(timer, stamp);
                return Err(exit);
            }
        };
        let cur = match got {
            Reply::Ok(data) => data.as_ref().and_then(program_scene),
            Reply::TimedOut => {
                self.due_again(timer, stamp);
                return Ok(());
            }
            Reply::Code(code) => {
                let get = RequestKind::GetCurrentProgramScene;
                return self.back_refused(conn, timer, stamp, get, code, BackPlan::default());
            }
        };
        let Some(cur) = cur else {
            self.skip(SkipOp::SceneBack, SkipWhy::Failed, None, timer);
            return self.commit(timer, BackPlan::default());
        };
        let plan = self.slots.plan_back(timer, &cur.uuid);
        let Some(to) = plan.to.clone() else {
            return self.commit(timer, plan);
        };
        let set = Request::SetCurrentProgramScene { uuid: to.uuid };
        match self.request(conn, &set).await {
            Ok(Reply::Ok(_)) => {
                self.clear_skip();
                self.commit(timer, plan)
            }
            Ok(Reply::TimedOut) => {
                self.skip(SkipOp::SceneBack, SkipWhy::TimedOut, None, timer);
                self.commit(timer, plan)
            }
            Ok(Reply::Code(code)) => {
                let kind = RequestKind::SetCurrentProgramScene;
                self.back_refused(conn, timer, stamp, kind, code, plan)
            }
            Err(exit) => {
                self.due_again(timer, stamp);
                Err(exit)
            }
        }
    }

    /// §7.13: a switch back's Get or Set refused: a 207 is due again (not
    /// taken for `not_ready_wait`); a missing scene or another code is a
    /// skip, the plan committed.
    fn back_refused(
        &mut self,
        conn: &mut Conn,
        timer: &str,
        stamp: Option<Instant>,
        kind: RequestKind,
        code: u64,
        plan: BackPlan,
    ) -> Result<(), Exit> {
        match request_failure(Work::SceneBack, kind, code) {
            Failure::DueAgain => {
                self.due_again(timer, stamp);
                self.busy_backs.insert(timer.to_string());
                conn.due_hold = Some(after(self.timing.not_ready_wait));
                Ok(())
            }
            failure => {
                let why = if failure == Failure::SkipNotFound {
                    SkipWhy::NotFound
                } else {
                    SkipWhy::Failed
                };
                self.skip(SkipOp::SceneBack, why, None, timer);
                self.commit(timer, plan)
            }
        }
    }

    /// §7.6: the plan done, unless stale.
    fn commit(&mut self, timer: &str, plan: BackPlan) -> Result<(), Exit> {
        if !self.fresh() {
            return Err(Exit::Stale);
        }
        self.busy_backs.remove(timer);
        self.slots.commit_back(timer, plan);
        Ok(())
    }

    // ---- The inventory (D19) -------------------------------------------------------

    /// One inventory step; the run is dropped when every caller has gone.
    async fn inventory_step(&mut self, conn: &mut Conn) -> Result<(), Exit> {
        let Some(run) = conn.inventory.as_mut() else {
            return Ok(());
        };
        run.repliers.retain(|r| !r.is_closed());
        let req = run.next_request();
        if run.repliers.is_empty() {
            conn.inventory = None;
            return Ok(());
        }
        let Some(req) = req else {
            self.finish_inventory(conn);
            return Ok(());
        };
        let kind = req.kind();
        let reply = self.request(conn, &req).await?;
        let Some(run) = conn.inventory.as_mut() else {
            return Ok(());
        };
        run.hold = None;
        match reply {
            Reply::Ok(data) => run.take(kind, &data.unwrap_or(Value::Null)),
            Reply::TimedOut => {
                if let Some(run) = conn.inventory.take() {
                    run.answer(&Err(NO_ANSWER.to_string()));
                }
            }
            Reply::Code(code) => match request_failure(Work::Inventory, kind, code) {
                Failure::WaitAndRetry => {
                    let wait = self.timing.not_ready_wait;
                    if run.began.elapsed().saturating_add(wait) >= INVENTORY_WAIT {
                        if let Some(run) = conn.inventory.take() {
                            run.answer(&Err(BUSY.to_string()));
                        }
                    } else {
                        run.hold = Some(after(wait));
                    }
                }
                Failure::ListedNotReadingFile => {
                    if let Some(source) = run.next_source() {
                        log::settings_unread(self.gen, &source.uuid, code);
                    }
                    run.flags.push(false);
                }
                _ => {
                    if let Some(run) = conn.inventory.take() {
                        run.answer(&Err(REFUSED.to_string()));
                    }
                }
            },
        }
        self.finish_inventory(conn);
        Ok(())
    }

    /// The run's answer once every step is done; a source it lists is no
    /// longer missing (D20).
    fn finish_inventory(&mut self, conn: &mut Conn) {
        let Some(inventory) = conn
            .inventory
            .as_ref()
            .and_then(|run| run.done(conn.inventory_rev))
        else {
            return;
        };
        let before = conn.missing.len();
        for source in &inventory.text_sources {
            conn.missing.remove(&source.uuid);
        }
        if conn.missing.len() != before {
            self.write_missing(conn);
        }
        if let Some(run) = conn.inventory.take() {
            run.answer(&Ok(inventory));
        }
    }
}

/// §7.13: a scene op's error answer as its skip.
fn scene_why(work: Work, kind: RequestKind, code: u64) -> SkipWhy {
    match request_failure(work, kind, code) {
        Failure::SkipNotReady => SkipWhy::NotReady,
        Failure::SkipNotFound => SkipWhy::NotFound,
        _ => SkipWhy::Failed,
    }
}

/// v1.61.0 (§10.2, track B1): the link against the fake obs-websocket server
/// (B2's, written from P576 and obs-websocket's sources by another hand),
/// through [`run`] with in-memory stubs for the status, the slots and the
/// timers, and the real stored password ([`StoredPassword`] over an
/// in-memory cipher). Every test runs on its own current-thread runtime
/// under the fake's 10 s guard, with §10.2's timings (`LinkTiming::scaled`
/// at 100 ms), the backoff's draw fixed at 0.5, and the fake on
/// 127.0.0.1:0.
#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic
)]
mod tests {
    use super::super::fake::{
        self, guarded, Accept, Config, Dir, End, FakeObs, Frame, Model, Output, OutputState as Out,
        Rule, Stage, BRB, BROWSER, GAME, JUST_CHATTING, SOURCE_TEXT, TEXT_FROM_FILE, TEXT_FT2,
        TEXT_V0, TEXT_V3,
    };
    use super::super::files::{self, Cipher};
    use super::super::machine::{
        backoff_ms, LinkConfig, ObsInventory, ObsReason, ObsState, ObsStatus, SkipOp, SkipWhy,
        StatusCell, TextKind, TextSkipWhy,
    };
    use super::super::scenes::{BackPlan, SceneRef, Slots};
    use super::super::testing::{assert_none_in, scratch, secret_forms, traced, MemCipher};
    use super::super::{
        wall_ms, Follow, FollowObs, LinkCmd, LinkDeps, LinkOp, LinkTiming, ObsFeed, ObsOp, ObsText,
        SlotStore, StatusSink, StoredPassword, TimerControl, OPS_QUEUE,
    };
    use super::{polls, run};
    use crate::timers::TimerOp;
    use std::collections::HashSet;
    use std::net::{IpAddr, Ipv4Addr};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};
    use tokio::runtime::Handle;
    use tokio::sync::{mpsc, oneshot, watch};
    use zeroize::Zeroizing;

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    /// §10.2's timings: 100 ms units.
    fn timing() -> LinkTiming {
        LinkTiming::scaled(ms(100)).unwrap()
    }

    /// §10.2: the backoff's draw, fixed.
    fn half() -> f64 {
        0.5
    }

    /// `ObsShared`'s status cell, every change recorded with when (both
    /// clocks) it was written.
    struct Statuses {
        cell: Mutex<StatusCell>,
        seen: Mutex<Vec<(Instant, i64, ObsStatus)>>,
        enabled: AtomicBool,
    }

    impl StatusSink for Statuses {
        fn write(&self, gen: u64, f: &mut dyn FnMut(&mut ObsStatus)) -> bool {
            let mut cell = self.cell.lock().unwrap();
            if cell.gen() != gen {
                return false;
            }
            if let Some(s) = cell.write(gen, |s| f(s)) {
                self.seen
                    .lock()
                    .unwrap()
                    .push((Instant::now(), wall_ms(), s));
            }
            true
        }

        fn current_gen(&self) -> u64 {
            self.cell.lock().unwrap().gen()
        }

        fn enabled(&self) -> bool {
            self.enabled.load(Ordering::SeqCst)
        }
    }

    /// `ObsShared`'s slots, due marks and written set, on the real
    /// [`Slots`].
    struct Store {
        slots: Mutex<Slots>,
        grace: Duration,
        written: Mutex<HashSet<String>>,
    }

    impl SlotStore for Store {
        fn after_scene(&self, timer: &str, cur: SceneRef, to: SceneRef) -> bool {
            self.slots
                .lock()
                .unwrap()
                .after_scene(timer, cur, to, wall_ms())
        }

        fn plan_back(&self, timer: &str, cur_uuid: &str) -> BackPlan {
            self.slots.lock().unwrap().plan_back(timer, cur_uuid)
        }

        fn commit_back(&self, timer: &str, plan: BackPlan) {
            self.slots.lock().unwrap().commit_back(timer, plan);
        }

        fn mark_due(&self, timer: &str, at: Instant) {
            self.slots.lock().unwrap().mark_due(timer, at);
        }

        fn take_due(&self, now: Instant) -> (Option<String>, Vec<String>) {
            self.slots.lock().unwrap().take_due(now, self.grace)
        }

        fn expire_due(&self, now: Instant) -> Vec<String> {
            self.slots.lock().unwrap().expire_due(now, self.grace)
        }

        fn next_expiry(&self) -> Option<Instant> {
            self.slots.lock().unwrap().next_expiry(self.grace)
        }

        fn written(&self) -> HashSet<String> {
            self.written.lock().unwrap().clone()
        }

        fn note_written(&self, uuid: &str, text_empty: bool) {
            let mut written = self.written.lock().unwrap();
            if text_empty {
                written.remove(uuid);
            } else {
                written.insert(uuid.to_string());
            }
        }
    }

    /// Every op the link applied to a timer, with when.
    #[derive(Default)]
    struct Timers(Mutex<Vec<(String, TimerOp, Instant)>>);

    impl TimerControl for Timers {
        fn apply(&self, id: &str, op: TimerOp) -> bool {
            self.0
                .lock()
                .unwrap()
                .push((id.to_string(), op, Instant::now()));
            true
        }
    }

    impl Timers {
        fn ops(&self) -> Vec<(String, TimerOp)> {
            self.0
                .lock()
                .unwrap()
                .iter()
                .map(|(id, op, _)| (id.clone(), *op))
                .collect()
        }
    }

    /// A running link and the test's ends of it.
    struct Link {
        status: Arc<Statuses>,
        store: Arc<Store>,
        timers: Arc<Timers>,
        feed: watch::Sender<Arc<ObsFeed>>,
        ops: mpsc::Sender<LinkOp>,
        cmds: Option<mpsc::Sender<LinkCmd>>,
        acked: std::sync::mpsc::Receiver<()>,
        cipher: Arc<MemCipher>,
    }

    /// A link to `fake`, with `password` stored (if any) and `feed` first.
    fn start(fake: &FakeObs, password: Option<&str>, feed: ObsFeed) -> Link {
        start_at(fake.port(), password, feed)
    }

    fn start_at(port: u16, password: Option<&str>, feed: ObsFeed) -> Link {
        let cfg = LinkConfig {
            ip: IpAddr::V4(Ipv4Addr::LOCALHOST),
            port,
        };
        let mut cell = StatusCell::new();
        let (gen, _) = cell.begin(&cfg);
        let status = Arc::new(Statuses {
            cell: Mutex::new(cell),
            seen: Mutex::default(),
            enabled: AtomicBool::new(true),
        });
        let dir = scratch("link");
        let cipher = Arc::new(MemCipher::default());
        if let Some(pw) = password {
            std::fs::create_dir_all(&dir).unwrap();
            let pw = Zeroizing::new(pw.to_string());
            files::save_password(&dir, &*cipher, &pw).unwrap();
        }
        let store = Arc::new(Store {
            slots: Mutex::default(),
            grace: timing().scene_back_grace,
            written: Mutex::default(),
        });
        let timers = Arc::new(Timers::default());
        let (feed_tx, feed_rx) = watch::channel(Arc::new(feed));
        let (ops_tx, ops) = mpsc::channel(OPS_QUEUE);
        let (cmds_tx, cmds) = mpsc::channel(8);
        let (closed_ack, acked) = std::sync::mpsc::sync_channel(1);
        let as_cipher: Arc<dyn Cipher> = cipher.clone();
        drop(tokio::spawn(run(LinkDeps {
            cfg,
            gen,
            status: status.clone(),
            slots: store.clone(),
            timers: timers.clone(),
            password: Arc::new(StoredPassword {
                dir,
                cipher: as_cipher,
            }),
            feed: feed_rx,
            ops,
            cmds,
            timing: timing(),
            rand: half,
            spawner: Handle::current(),
            closed_ack,
        })));
        Link {
            status,
            store,
            timers,
            feed: feed_tx,
            ops: ops_tx,
            cmds: Some(cmds_tx),
            acked,
            cipher,
        }
    }

    impl Link {
        fn now(&self) -> ObsStatus {
            self.status.cell.lock().unwrap().status().clone()
        }

        /// Every status written, oldest first.
        fn seen(&self) -> Vec<ObsStatus> {
            self.status
                .seen
                .lock()
                .unwrap()
                .iter()
                .map(|(_, _, s)| s.clone())
                .collect()
        }

        /// Waits up to `within` for the status now to pass `f`.
        async fn until(&self, within: Duration, f: impl Fn(&ObsStatus) -> bool) -> bool {
            let end = Instant::now() + within;
            loop {
                if f(&self.now()) {
                    return true;
                }
                if Instant::now() >= end {
                    return false;
                }
                tokio::time::sleep(ms(2)).await;
            }
        }

        async fn state(&self, state: ObsState, within: Duration) -> bool {
            self.until(within, |s| s.state == state).await
        }

        async fn connected(&self) {
            assert!(
                self.state(ObsState::Connected, ms(3_000)).await,
                "never connected: {:?}",
                self.now()
            );
        }

        fn op(&self, timer: &str, op: ObsOp) {
            self.op_at(timer, op, Instant::now());
        }

        fn op_at(&self, timer: &str, op: ObsOp, at: Instant) {
            self.ops
                .try_send(LinkOp {
                    at,
                    timer: timer.into(),
                    op,
                })
                .unwrap();
        }

        /// The feed: these `(uuid, text)` pairs, no follows.
        fn texts(&self, texts: &[(&str, &str)]) {
            self.feed.send_replace(Arc::new(texts_feed(texts)));
        }

        fn cmd(&self, cmd: LinkCmd) {
            self.cmds.as_ref().unwrap().try_send(cmd).unwrap();
        }

        /// `obs_inventory`'s request, its answer to come.
        fn inventory(&self) -> oneshot::Receiver<Result<ObsInventory, String>> {
            let (reply, answer) = oneshot::channel();
            self.cmd(LinkCmd::Inventory(reply));
            answer
        }

        /// The stop: the command sender dropped.
        fn stop(&mut self) {
            self.cmds = None;
        }

        /// Waits (yielding, so the moment is exact) up to `within` for the
        /// ack; when it came.
        async fn ack(&self, within: Duration) -> Option<Instant> {
            let end = Instant::now() + within;
            while Instant::now() < end {
                if self.acked.try_recv().is_ok() {
                    return Some(Instant::now());
                }
                tokio::task::yield_now().await;
            }
            None
        }
    }

    fn texts_feed(texts: &[(&str, &str)]) -> ObsFeed {
        ObsFeed {
            texts: texts
                .iter()
                .map(|(uuid, text)| ObsText {
                    uuid: (*uuid).into(),
                    text: (*text).into(),
                })
                .collect(),
            ..ObsFeed::default()
        }
    }

    fn scene_op(uuid: &str, name: &str) -> ObsOp {
        ObsOp::Scene {
            scene_uuid: uuid.into(),
            scene_name: name.into(),
        }
    }

    fn scene(uuid: &str, name: &str) -> SceneRef {
        SceneRef {
            uuid: uuid.into(),
            name: name.into(),
        }
    }

    /// The pushes the fake received: `(uuid, text, when)`.
    fn pushes(fake: &FakeObs) -> Vec<(String, String, Instant)> {
        fake.requests_of("SetInputSettings")
            .iter()
            .map(|r| {
                let d = r.data.as_ref().unwrap();
                (
                    d["inputUuid"].as_str().unwrap().to_string(),
                    d["inputSettings"]["text"].as_str().unwrap().to_string(),
                    r.at,
                )
            })
            .collect()
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

    fn types(fake: &FakeObs) -> Vec<String> {
        fake.requests()
            .iter()
            .map(|r| r.request_type.clone())
            .collect()
    }

    /// The last status's skip, without its time.
    fn skip_of(s: &ObsStatus) -> Option<(SkipOp, SkipWhy, String)> {
        s.last_skip.as_ref().map(|k| (k.op, k.why, k.timer.clone()))
    }

    // ---- The handshake (L1-L5, L27, L28) -----------------------------------

    /// L1: Hello, Identify (RPC 1, mask 69, the auth string), Identified,
    /// then `GetVersion` → `GetStreamStatus` → `GetRecordStatus`, and `connected`
    /// with the versions; nothing before Identified (the fake would record
    /// a 4007).
    #[test]
    fn l1_happy_handshake() {
        guarded(async {
            let fake = FakeObs::start(Config::default().with_password("pw ✓")).await;
            let link = start(&fake, Some("pw ✓"), ObsFeed::default());
            link.connected().await;
            let s = link.now();
            assert_eq!(s.obs_version.as_deref(), Some("32.2.2"));
            assert_eq!(s.websocket_version.as_deref(), Some("5.7.6"));
            assert_eq!(s.auth_required, Some(true));
            assert_eq!(s.inventory_rev, 1);
            assert_eq!(s.reason, None);
            let session = &fake.sessions()[0];
            assert_eq!(session.subscriptions, Some(69));
            assert_eq!(session.rpc_version, Some(1));
            assert_eq!(
                types(&fake)[..3],
                ["GetVersion", "GetStreamStatus", "GetRecordStatus"]
            );
            assert_eq!(fake.identifies().len(), 1);
            assert_eq!(fake.accepts(), 1);
            fake.assert_clean();
        });
    }

    /// L2: a wrong password: one attempt, `authFailed`, no retry.
    #[test]
    fn l2_a_wrong_password_stops_after_one_attempt() {
        guarded(async {
            let fake = FakeObs::start(Config::default().with_password("right")).await;
            let link = start(&fake, Some("wrong"), ObsFeed::default());
            assert!(link.state(ObsState::AuthFailed, ms(3_000)).await);
            tokio::time::sleep(ms(1_000)).await;
            assert_eq!(fake.accepts(), 1);
            assert_eq!(link.now().state, ObsState::AuthFailed);
            assert_eq!(link.now().auth_required, Some(true));
        });
    }

    /// L3: OBS asks for a password and none is stored: no Identify,
    /// `needsPassword`. OBS asks for none and one is stored: no decrypt.
    #[test]
    fn l3_the_password_is_read_only_when_hello_asks() {
        guarded(async {
            let fake = FakeObs::start(Config::default().with_password("pw")).await;
            let link = start(&fake, None, ObsFeed::default());
            assert!(link.state(ObsState::NeedsPassword, ms(3_000)).await);
            tokio::time::sleep(ms(300)).await;
            assert_eq!(fake.identifies(), []);
            assert_eq!(fake.accepts(), 1);

            let open = FakeObs::start(Config::default()).await;
            let link = start(&open, Some("pw"), ObsFeed::default());
            link.connected().await;
            assert_eq!(link.cipher.unprotects(), 0);
            assert_eq!(link.now().auth_required, Some(false));
            open.assert_clean();
        });
    }

    /// L4a: a 5.3.3 Hello: refused before any Identify.
    #[test]
    fn l4a_an_old_server_gets_no_identify() {
        guarded(async {
            let fake = FakeObs::start(Config::default().ws_version("5.3.3")).await;
            let link = start(&fake, None, ObsFeed::default());
            assert!(link.state(ObsState::TooOld, ms(3_000)).await);
            assert_eq!(link.now().websocket_version.as_deref(), Some("5.3.3"));
            tokio::time::sleep(ms(300)).await;
            assert_eq!(fake.identifies(), []);
            assert_eq!(fake.accepts(), 1);
        });
    }

    /// L4b: Hello announces RPC 2 and the server serves only 2: Divora
    /// Identifies with 1 once, and 4010 is `needsNewerDivora`, no retry.
    #[test]
    fn l4b_a_server_without_rpc_1_needs_a_newer_divora() {
        guarded(async {
            let config = Config::default().hello_rpc(2).accept_rpc(&[2]);
            let fake = FakeObs::start(config).await;
            let link = start(&fake, None, ObsFeed::default());
            assert!(link.state(ObsState::NeedsNewerDivora, ms(3_000)).await);
            tokio::time::sleep(ms(1_000)).await;
            let identifies = fake.identifies();
            assert_eq!(identifies.len(), 1);
            assert_eq!(identifies[0].1["rpcVersion"], 1);
            assert_eq!(fake.accepts(), 1);
        });
    }

    /// L4c: Hello announces RPC 2 and the server still serves 1: connected.
    #[test]
    fn l4c_a_server_announcing_rpc_2_that_serves_1_connects() {
        guarded(async {
            let config = Config::default().hello_rpc(2).accept_rpc(&[1, 2]);
            let fake = FakeObs::start(config).await;
            let link = start(&fake, None, ObsFeed::default());
            link.connected().await;
            assert_eq!(fake.sessions()[0].rpc_version, Some(1));
            fake.assert_clean();
        });
    }

    /// L5: a Kick (4011): `kicked`, and the link never reconnects on its
    /// own.
    #[test]
    fn l5_a_kick_is_never_retried() {
        guarded(async {
            let fake = FakeObs::start(Config::default()).await;
            let link = start(&fake, None, ObsFeed::default());
            link.connected().await;
            fake.close_all(4011, "kicked");
            assert!(link.state(ObsState::Kicked, ms(2_000)).await);
            tokio::time::sleep(ms(1_000)).await;
            assert_eq!(fake.accepts(), 1);
            assert_eq!(link.now().state, ObsState::Kicked);
        });
    }

    /// L27: no Hello, or no Identified: `noAnswer` after the scaled
    /// handshake or Identify timeout; an upgrade answer that never ends:
    /// `notObs` once 64 KiB is read, before the timeout.
    #[test]
    fn l27_handshake_failures() {
        guarded(async {
            for (stage, hello) in [("hello", true), ("identified", false)] {
                let fake = FakeObs::start(Config::default()).await;
                if hello {
                    fake.set_hello(Stage::Never);
                } else {
                    fake.set_identified(Stage::Never);
                }
                let began = Instant::now();
                let link = start(&fake, None, ObsFeed::default());
                assert!(link.state(ObsState::Retrying, ms(2_000)).await, "{stage}");
                let took = began.elapsed();
                assert_eq!(link.now().reason, Some(ObsReason::NoAnswer), "{stage}");
                assert!(took >= ms(450) && took < ms(1_500), "{stage}: {took:?}");
            }

            let fake = FakeObs::start(Config::default()).await;
            fake.set_accept(Accept::EndlessHeader);
            let began = Instant::now();
            let link = start(&fake, None, ObsFeed::default());
            assert!(link.state(ObsState::Retrying, ms(2_000)).await);
            assert_eq!(link.now().reason, Some(ObsReason::NotObs));
            assert!(began.elapsed() < ms(450), "{:?}", began.elapsed());
            assert!(fake.sessions()[0].header_bytes >= 64 << 10);
        });
    }

    /// L28: `GetVersion`'s list lacks one of the nine: `unsupported`,
    /// `missingRequest`, its name; no retry.
    #[test]
    fn l28_a_missing_request_stops() {
        guarded(async {
            let fake = FakeObs::start(Config::default().unavailable("SetInputSettings")).await;
            let link = start(&fake, None, ObsFeed::default());
            assert!(link.state(ObsState::Unsupported, ms(3_000)).await);
            let s = link.now();
            assert_eq!(s.reason, Some(ObsReason::MissingRequest));
            assert_eq!(s.detail, Some("SetInputSettings"));
            tokio::time::sleep(ms(1_000)).await;
            assert_eq!(fake.accepts(), 1);
            fake.assert_clean();
        });
    }

    // ---- Backoff (L6, L6b, L7) ------------------------------------------------

    /// The `retrying` statuses written, with when (both clocks).
    fn retries(link: &Link) -> Vec<(Instant, i64, ObsStatus)> {
        link.status
            .seen
            .lock()
            .unwrap()
            .iter()
            .filter(|(_, _, s)| s.state == ObsState::Retrying)
            .cloned()
            .collect()
    }

    /// §7.3 at r = 0.5, in 200 ms units (§10.2's scaled unit).
    fn nominal(n: u32) -> Duration {
        ms(backoff_ms(n, 0.5) * 200 / 1_000)
    }

    /// L6: OBS goes away (accept-and-close) through five `retrying`
    /// statuses: each one's `retryAtMs` is §7.3's step at r = 0.5 from when
    /// it was written, no attempt comes before it, each comes within 10× the
    /// gap; then OBS is back, `connected`, and the mapped text is pushed
    /// again on the new connection (`pushed` starts empty).
    #[test]
    fn l6_backoff_follows_the_table_and_a_reconnect_pushes_again() {
        guarded(async {
            let fake = FakeObs::start(Config::default()).await;
            let link = start(&fake, None, texts_feed(&[(TEXT_V3, "1:00")]));
            link.connected().await;
            assert!(fake.wait(ms(1_000), |f| pushes(f).len() == 1).await);
            fake.set_accept(Accept::CloseAtOnce);
            fake.close_all(1001, "");
            assert!(fake.wait_accepts(5, ms(8_000)).await);
            fake.set_accept(Accept::Normal);
            assert!(fake.wait(ms(8_000), |f| pushes(f).len() == 2).await);
            assert!(link.state(ObsState::Connected, ms(1_000)).await);
            let tries = retries(&link);
            assert_eq!(tries.len(), 5, "{tries:?}");
            let sessions = fake.sessions();
            for (n, (at, wall, s)) in tries.iter().enumerate() {
                let n32 = u32::try_from(n + 1).unwrap();
                let gap = nominal(n32);
                let said = s.retry_at_ms.unwrap() - wall;
                let gap_ms = i64::try_from(gap.as_millis()).unwrap();
                assert!(
                    (said - gap_ms).abs() <= 5,
                    "step {n32}: {said} ms, not {gap_ms}"
                );
                let next = sessions[n + 1].accepted_at;
                let waited = next.saturating_duration_since(*at);
                assert!(
                    next + ms(2) >= *at + gap,
                    "step {n32}: an attempt before retryAtMs"
                );
                assert!(waited <= gap * 10, "step {n32}: {waited:?}");
            }
            let again = pushes(&fake);
            assert_eq!(again[1].0, TEXT_V3);
            assert_eq!(again[1].1, "1:00");
            fake.assert_clean();
        });
    }

    /// L6b: nothing listening (a port bound, then let go): `retrying`,
    /// `refused` or `unreachable`; the test prints which and how long the
    /// attempt took (Windows' refused connect, D8).
    #[test]
    fn l6b_nothing_listening() {
        guarded(async {
            let port = {
                let held = std::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
                held.local_addr().unwrap().port()
            };
            let began = Instant::now();
            let link = start_at(port, None, ObsFeed::default());
            assert!(link.state(ObsState::Retrying, ms(3_000)).await);
            let reason = link.now().reason;
            assert!(
                matches!(reason, Some(ObsReason::Refused | ObsReason::Unreachable)),
                "{reason:?}"
            );
            eprintln!(
                "l6b: a connect to a closed loopback port: {reason:?} after {:?}",
                began.elapsed()
            );
        });
    }

    /// L7: a connection that stayed up past `reset_after` resets the run:
    /// after two failures, a connection held 600 ms, then closed, waits
    /// step 1, not step 3.
    #[test]
    fn l7_a_connection_that_held_resets_the_backoff() {
        guarded(async {
            let fake = FakeObs::start(Config::default()).await;
            fake.set_accept(Accept::CloseAtOnce);
            let link = start(&fake, None, ObsFeed::default());
            assert!(fake.wait_accepts(2, ms(3_000)).await);
            fake.set_accept(Accept::Normal);
            link.connected().await;
            assert_eq!(retries(&link).len(), 2);
            tokio::time::sleep(ms(600)).await;
            fake.close_all(1001, "");
            assert!(link.state(ObsState::Retrying, ms(2_000)).await);
            let tries = retries(&link);
            assert_eq!(tries.len(), 3);
            let (_, wall, s) = tries.last().unwrap().clone();
            let said = s.retry_at_ms.unwrap() - wall;
            assert!((said - 200).abs() <= 5, "{said} ms, not step 1's 200");
        });
    }

    // ---- One request at a time (L8-L13, L34) ----------------------------------

    /// L8: a push that times out: `textSkip {timedOut}` at the request
    /// timeout; nothing else is sent until its late answer (600 ms); then
    /// the next push. Never two outstanding.
    #[test]
    fn l8_an_unanswered_push_wedges_until_its_late_answer() {
        guarded(async {
            let fake = FakeObs::start(Config::default()).await;
            fake.script(Rule::on("SetInputSettings").times(1).delay(ms(600)));
            let link = start(&fake, None, texts_feed(&[(TEXT_V3, "0:01")]));
            link.connected().await;
            assert!(fake.wait(ms(1_000), |f| pushes(f).len() == 1).await);
            let sent = pushes(&fake)[0].2;
            link.texts(&[(TEXT_V3, "0:02")]);
            assert!(
                link.until(ms(1_000), |s| s
                    .text_skip
                    .as_ref()
                    .is_some_and(|t| t.why == TextSkipWhy::TimedOut))
                    .await
            );
            let skipped = sent.elapsed();
            assert!(skipped >= ms(150) && skipped < ms(450), "{skipped:?}");
            assert!(fake.wait(ms(2_000), |f| pushes(f).len() == 2).await);
            let all = fake.requests_of("SetInputSettings");
            let answered = all[0].answer.unwrap().0;
            assert!(
                all[1].at >= answered,
                "the next push went before the late answer"
            );
            assert!(all[0].answer.unwrap().0 >= sent + ms(550));
            assert_eq!(pushes(&fake)[1].1, "0:02");
            // The late success counts: the next confirmed push clears the skip.
            assert!(link.until(ms(1_000), |s| s.text_skip.is_none()).await);
            assert_eq!(fake.max_outstanding(), 1);
            fake.assert_clean();
        });
    }

    /// L8b: an answer that never comes: the late-answer cap (1 s) ends the
    /// connection `noAnswer`, and the link reconnects.
    #[test]
    fn l8b_the_late_answer_cap_ends_the_connection() {
        guarded(async {
            let fake = FakeObs::start(Config::default()).await;
            fake.script(Rule::on("SetInputSettings").times(1).drop_answer());
            let link = start(&fake, None, texts_feed(&[(TEXT_V3, "0:01")]));
            link.connected().await;
            assert!(fake.wait(ms(1_000), |f| pushes(f).len() == 1).await);
            let sent = pushes(&fake)[0].2;
            assert!(link.state(ObsState::Retrying, ms(3_000)).await);
            assert_eq!(link.now().reason, Some(ObsReason::NoAnswer));
            let took = sent.elapsed();
            assert!(took >= ms(950) && took < ms(1_600), "{took:?}");
            assert!(fake.wait_identified(2, ms(2_000)).await);
            assert!(link.state(ObsState::Connected, ms(2_000)).await);
            assert_eq!(fake.max_outstanding(), 1);
        });
    }

    /// L8c: two `scene`s queued behind a 900 ms stall: the one queued 100 ms
    /// in is dropped when taken (`late`), the one queued 100 ms before the
    /// release runs.
    #[test]
    fn l8c_a_scene_behind_a_stall_is_never_run_late() {
        guarded(async {
            let fake = FakeObs::start(Config::default()).await;
            fake.script(Rule::on("SetInputSettings").times(1).delay(ms(900)));
            let link = start(&fake, None, texts_feed(&[(TEXT_V3, "0:01")]));
            link.connected().await;
            assert!(fake.wait(ms(1_000), |f| pushes(f).len() == 1).await);
            let stalled = pushes(&fake)[0].2;
            tokio::time::sleep_until((stalled + ms(100)).into()).await;
            link.op("early", scene_op(BRB, "BRB"));
            tokio::time::sleep_until((stalled + ms(800)).into()).await;
            link.op("fresh", scene_op(JUST_CHATTING, "Just Chatting"));
            assert!(fake.wait(ms(2_000), |f| sets(f).len() == 1).await);
            tokio::time::sleep(ms(200)).await;
            assert_eq!(sets(&fake), [JUST_CHATTING]);
            assert!(link
                .seen()
                .iter()
                .any(|s| skip_of(s) == Some((SkipOp::Scene, SkipWhy::Late, "early".into()))));
            assert_eq!(link.now().last_skip, None, "the switch that ran clears it");
            fake.assert_clean();
        });
    }

    /// L9: ops and pushes queued together, every answer slowed: the fake
    /// never sees two outstanding.
    #[test]
    fn l9_one_request_at_a_time() {
        guarded(async {
            let fake = FakeObs::start(Config::default()).await;
            let link = start(&fake, None, ObsFeed::default());
            link.connected().await;
            fake.script(Rule::any().delay(ms(15)));
            for i in 0..10 {
                let to = if i % 2 == 0 { BRB } else { GAME };
                link.op(&format!("t{i}"), scene_op(to, "x"));
                link.texts(&[(TEXT_V3, &format!("{i}")), (TEXT_FT2, &format!("-{i}"))]);
            }
            assert!(
                fake.wait(ms(5_000), |f| f.requests_of("GetCurrentProgramScene").len()
                    >= 10)
                    .await
            );
            tokio::time::sleep(ms(300)).await;
            assert!(pushes(&fake).len() >= 2);
            assert_eq!(fake.max_outstanding(), 1);
            fake.assert_clean();
        });
    }

    /// L10: 50 feed changes while a push stalls 150 ms (under the timeout):
    /// at most the outstanding push and the latest reach the fake, the last
    /// carrying the last text.
    #[test]
    fn l10_feeds_coalesce_behind_a_slow_push() {
        guarded(async {
            let fake = FakeObs::start(Config::default()).await;
            let link = start(&fake, None, texts_feed(&[(TEXT_V3, "t0")]));
            link.connected().await;
            assert!(fake.wait(ms(1_000), |f| pushes(f).len() == 1).await);
            fake.script(Rule::on("SetInputSettings").delay(ms(150)));
            link.texts(&[(TEXT_V3, "t1")]);
            assert!(fake.wait(ms(1_000), |f| pushes(f).len() == 2).await);
            for i in 2..=50 {
                link.texts(&[(TEXT_V3, &format!("t{i}"))]);
            }
            tokio::time::sleep(ms(700)).await;
            let after: Vec<String> = pushes(&fake)[1..].iter().map(|p| p.1.clone()).collect();
            assert!(after.len() <= 2, "{after:?}");
            assert_eq!(after.last().map(String::as_str), Some("t50"));
            fake.assert_clean();
        });
    }

    /// L10b: three mapped sources whose texts change every 40 ms, the first
    /// push stalled past the timeout and every other taking 60 ms: never two
    /// outstanding, and no push carries a text older than the newest feed
    /// when it was sent (each push reads the feed again).
    #[test]
    fn l10b_each_push_reads_the_newest_feed() {
        guarded(async {
            let fake = FakeObs::start(Config::default()).await;
            let link = start(&fake, None, ObsFeed::default());
            link.connected().await;
            fake.script(Rule::on("SetInputSettings").times(1).delay(ms(400)));
            fake.script(Rule::on("SetInputSettings").delay(ms(60)));
            let mut sent: Vec<(Instant, u32)> = Vec::new();
            for n in 1..=30_u32 {
                let t = n.to_string();
                link.texts(&[(TEXT_V0, &t), (TEXT_V3, &t), (TEXT_FT2, &t)]);
                sent.push((Instant::now(), n));
                tokio::time::sleep(ms(40)).await;
            }
            tokio::time::sleep(ms(500)).await;
            let all = pushes(&fake);
            assert!(all.len() >= 6, "{}", all.len());
            for (uuid, text, at) in &all {
                let n: u32 = text.parse().unwrap();
                let newest = sent
                    .iter()
                    .filter(|(t, _)| *t + ms(25) <= *at)
                    .map(|(_, n)| *n)
                    .max()
                    .unwrap_or(0);
                assert!(n >= newest, "{uuid} got {n} after {newest} was out");
            }
            assert_eq!(fake.max_outstanding(), 1);
            fake.assert_clean();
        });
    }

    /// L11: a feed sent 5 times unchanged: one push.
    #[test]
    fn l11_an_unchanged_text_is_pushed_once() {
        guarded(async {
            let fake = FakeObs::start(Config::default()).await;
            let link = start(&fake, None, ObsFeed::default());
            link.connected().await;
            for _ in 0..5 {
                link.texts(&[(TEXT_V3, "5:00")]);
                tokio::time::sleep(ms(30)).await;
            }
            tokio::time::sleep(ms(300)).await;
            assert_eq!(pushes(&fake).len(), 1);
        });
    }

    /// L12: three 207s at startup: each step waits and goes again, then
    /// `connected`.
    #[test]
    fn l12_a_few_207s_at_startup_are_waited_out() {
        guarded(async {
            let fake = FakeObs::start(Config::default()).await;
            fake.script(Rule::any().times(3).not_ready());
            let link = start(&fake, None, ObsFeed::default());
            link.connected().await;
            assert_eq!(fake.requests_of("GetVersion").len(), 4);
            assert_eq!(fake.accepts(), 1);
        });
    }

    /// L12b: 207 for ever at startup: `retrying`/`notReady` within the 1 s
    /// cap (+ 200 ms), then the backoff's next attempt.
    #[test]
    fn l12b_207_for_ever_at_startup_is_not_ready() {
        guarded(async {
            let fake = FakeObs::start(Config::default()).await;
            fake.set_ready(false);
            let link = start(&fake, None, ObsFeed::default());
            assert!(fake.wait_identified(1, ms(2_000)).await);
            let identified = Instant::now();
            assert!(link.state(ObsState::Retrying, ms(3_000)).await);
            assert_eq!(link.now().reason, Some(ObsReason::NotReady));
            let took = identified.elapsed();
            assert!(took >= ms(900) && took <= ms(1_300), "{took:?}");
            assert!(fake.wait_accepts(2, ms(1_000)).await);
        });
    }

    /// L12c: after `ExitStarted`, the first 207 ends the connection at once
    /// (`obsExiting`).
    #[test]
    fn l12c_a_207_after_exit_started_ends_it() {
        guarded(async {
            let fake = FakeObs::start(Config::default()).await;
            let link = start(&fake, None, ObsFeed::default());
            link.connected().await;
            fake.exit_started();
            assert!(
                link.until(ms(1_000), |s| s.reason == Some(ObsReason::ObsExiting))
                    .await
            );
            assert_eq!(link.now().state, ObsState::Connected);
            link.texts(&[(TEXT_V3, "0:30")]);
            assert!(link.state(ObsState::Retrying, ms(500)).await);
            assert_eq!(link.now().reason, Some(ObsReason::ObsExiting));
            assert_eq!(pushes(&fake).len(), 1);
        });
    }

    /// L13: 600 and 602 make a source missing: no push to it until the
    /// re-check (1 s); 702 is a failure, `textSkip {failed}`, retried after
    /// 100 ms with the feed unchanged.
    #[test]
    fn l13_a_missing_source_waits_for_its_recheck() {
        guarded(async {
            for code in [600, 602] {
                let fake = FakeObs::start(Config::default()).await;
                fake.script(Rule::on("SetInputSettings").times(1).code(code));
                let link = start(&fake, None, ObsFeed::default());
                link.connected().await;
                link.texts(&[(TEXT_V3, "0")]);
                assert!(
                    link.until(ms(1_000), |s| s.missing == [TEXT_V3]).await,
                    "{code}"
                );
                assert_eq!(link.now().text_skip, None);
                let refused = pushes(&fake)[0].2;
                for i in 1..=8 {
                    link.texts(&[(TEXT_V3, &i.to_string())]);
                    tokio::time::sleep(ms(100)).await;
                }
                assert!(
                    fake.wait(ms(1_000), |f| pushes(f).len() >= 2).await,
                    "{code}"
                );
                let again = pushes(&fake)[1].2;
                assert!(
                    again >= refused + ms(950),
                    "{code}: again after {:?}",
                    again - refused
                );
                assert!(link.until(ms(500), |s| s.missing.is_empty()).await);
            }

            let fake = FakeObs::start(Config::default()).await;
            fake.script(Rule::on("SetInputSettings").times(1).code(702));
            let link = start(&fake, None, ObsFeed::default());
            link.connected().await;
            link.texts(&[(TEXT_V3, "0:09")]);
            assert!(fake.wait(ms(1_000), |f| pushes(f).len() == 2).await);
            let all = pushes(&fake);
            let gap = all[1].2 - all[0].2;
            assert!(gap >= ms(90) && gap < ms(400), "{gap:?}");
            assert_eq!(all[1].1, "0:09");
            assert!(link.seen().iter().any(|s| s
                .text_skip
                .as_ref()
                .is_some_and(|t| t.why == TextSkipWhy::Failed)));
            assert!(link.seen().iter().all(|s| s.missing.is_empty()));
        });
    }

    /// L34: the last push of a paused timer refused (207, then 702): with
    /// the feed unchanged, the text still reaches OBS within the retry.
    #[test]
    fn l34_a_refused_final_push_is_retried() {
        guarded(async {
            for refusal in [
                Rule::on("SetInputSettings").times(1).not_ready(),
                Rule::on("SetInputSettings").times(1).code(702),
            ] {
                let fake = FakeObs::start(Config::default()).await;
                fake.script(refusal);
                let link = start(&fake, None, texts_feed(&[(TEXT_V3, "4:59")]));
                link.connected().await;
                assert!(fake.wait(ms(1_000), |f| pushes(f).len() == 2).await);
                let all = pushes(&fake);
                assert_eq!(all[1].1, "4:59");
                assert!(all[1].2 - all[0].2 < ms(400));
                assert_eq!(
                    fake.model(|m| m.input(TEXT_V3).unwrap().settings["text"].clone()),
                    "4:59"
                );
                tokio::time::sleep(ms(300)).await;
                assert_eq!(pushes(&fake).len(), 2, "confirmed: not pushed again");
            }
        });
    }

    // ---- Scene ops (L14-L17e) ------------------------------------------------------

    /// L14: `scene` then `sceneBack`: Get, Set(BRB), the slot stored between
    /// them; then Get, Set(Game), the slot gone.
    #[test]
    fn l14_a_break_switches_and_switches_back() {
        guarded(async {
            let fake = FakeObs::start(Config::default()).await;
            let link = start(&fake, None, ObsFeed::default());
            link.connected().await;
            link.op("brb", scene_op(BRB, "BRB"));
            assert!(fake.wait(ms(1_000), |f| sets(f).len() == 1).await);
            assert_eq!(fake.program(), BRB);
            let plan = link.store.plan_back("brb", BRB);
            assert_eq!(plan.to, Some(scene(GAME, "Game")));
            link.op("brb", ObsOp::SceneBack);
            assert!(fake.wait(ms(1_000), |f| sets(f).len() == 2).await);
            assert_eq!(sets(&fake), [BRB, GAME]);
            assert_eq!(fake.program(), GAME);
            assert_eq!(
                types(&fake)[3..],
                [
                    "GetCurrentProgramScene",
                    "SetCurrentProgramScene",
                    "GetCurrentProgramScene",
                    "SetCurrentProgramScene"
                ]
            );
            tokio::time::sleep(ms(50)).await;
            assert_eq!(link.store.plan_back("brb", BRB), BackPlan::default());
            assert_eq!(link.now().last_skip, None);
            fake.assert_clean();
        });
    }

    /// L15: the streamer moved on during the break: no switch back.
    #[test]
    fn l15_a_switch_back_leaves_a_scene_you_moved_to() {
        guarded(async {
            let fake = FakeObs::start(Config::default()).await;
            let link = start(&fake, None, ObsFeed::default());
            link.connected().await;
            link.op("brb", scene_op(BRB, "BRB"));
            assert!(fake.wait(ms(1_000), |f| sets(f).len() == 1).await);
            assert!(fake.switch_program(JUST_CHATTING));
            link.op("brb", ObsOp::SceneBack);
            assert!(
                fake.wait_requests("GetCurrentProgramScene", 2, ms(1_000))
                    .await
            );
            tokio::time::sleep(ms(200)).await;
            assert_eq!(sets(&fake), [BRB]);
            assert_eq!(fake.program(), JUST_CHATTING);
            assert_eq!(
                link.store.plan_back("brb", JUST_CHATTING),
                BackPlan::default()
            );
        });
    }

    /// L16: nested breaks: A: Game → BRB; B: BRB → Just Chatting; A's back
    /// while on Just Chatting switches nothing and hands Game to B; B's back
    /// → Game.
    #[test]
    fn l16_a_stacked_break_inherits_the_way_back() {
        guarded(async {
            let fake = FakeObs::start(Config::default()).await;
            let link = start(&fake, None, ObsFeed::default());
            link.connected().await;
            link.op("a", scene_op(BRB, "BRB"));
            link.op("b", scene_op(JUST_CHATTING, "Just Chatting"));
            assert!(fake.wait(ms(1_000), |f| sets(f).len() == 2).await);
            link.op("a", ObsOp::SceneBack);
            assert!(
                fake.wait_requests("GetCurrentProgramScene", 3, ms(1_000))
                    .await
            );
            tokio::time::sleep(ms(100)).await;
            assert_eq!(sets(&fake).len(), 2);
            link.op("b", ObsOp::SceneBack);
            assert!(fake.wait(ms(1_000), |f| sets(f).len() == 3).await);
            assert_eq!(sets(&fake), [BRB, JUST_CHATTING, GAME]);
        });
    }

    /// L17: ops while `retrying`: a `scene` is a skip (`notConnected`); a
    /// `sceneBack` runs once OBS is back within the grace; past the grace
    /// it is dropped, `lastSkip {sceneBack, notConnected}`.
    #[test]
    fn l17_ops_while_disconnected() {
        guarded(async {
            let fake = FakeObs::start(Config::default()).await;
            fake.model(|m| m.program = BRB.into());
            fake.set_accept(Accept::CloseAtOnce);
            let link = start(&fake, None, ObsFeed::default());
            assert!(link
                .store
                .after_scene("brb", scene(GAME, "Game"), scene(BRB, "BRB")));
            assert!(link.state(ObsState::Retrying, ms(2_000)).await);
            link.op("live", scene_op(JUST_CHATTING, "Just Chatting"));
            assert!(
                link.until(ms(500), |s| skip_of(s)
                    == Some((SkipOp::Scene, SkipWhy::NotConnected, "live".into())))
                    .await
            );
            link.op("brb", ObsOp::SceneBack);
            tokio::time::sleep(ms(50)).await;
            fake.set_accept(Accept::Normal);
            assert!(fake.wait(ms(2_000), |f| sets(f).len() == 1).await);
            assert_eq!(sets(&fake), [GAME]);

            let fake = FakeObs::start(Config::default()).await;
            fake.model(|m| m.program = BRB.into());
            fake.set_accept(Accept::CloseAtOnce);
            let link = start(&fake, None, ObsFeed::default());
            assert!(link
                .store
                .after_scene("brb", scene(GAME, "Game"), scene(BRB, "BRB")));
            assert!(link.state(ObsState::Retrying, ms(2_000)).await);
            let due = Instant::now();
            link.op_at("brb", ObsOp::SceneBack, due);
            assert!(
                link.until(ms(3_000), |s| skip_of(s)
                    == Some((SkipOp::SceneBack, SkipWhy::NotConnected, "brb".into())))
                    .await
            );
            let after = due.elapsed();
            assert!(after >= ms(2_000) && after < ms(2_500), "{after:?}");
            fake.set_accept(Accept::Normal);
            link.cmd(LinkCmd::Wake);
            link.connected().await;
            tokio::time::sleep(ms(200)).await;
            assert_eq!(sets(&fake), [""; 0]);
        });
    }

    /// L17b: Try again (`Wake`) while a switch back is due: the attempt
    /// comes at once, and the back runs at the connected moment.
    #[test]
    fn l17b_try_again_runs_a_due_switch_back() {
        guarded(async {
            let fake = FakeObs::start(Config::default()).await;
            fake.model(|m| m.program = BRB.into());
            fake.set_accept(Accept::CloseAtOnce);
            let link = start(&fake, None, ObsFeed::default());
            assert!(link
                .store
                .after_scene("brb", scene(GAME, "Game"), scene(BRB, "BRB")));
            assert!(fake.wait_accepts(3, ms(3_000)).await);
            assert!(
                link.until(ms(500), |s| s.state == ObsState::Retrying
                    && retries(&link).len() == 3)
                    .await
            );
            link.op("brb", ObsOp::SceneBack);
            fake.set_accept(Accept::Normal);
            let woke = Instant::now();
            link.cmd(LinkCmd::Wake);
            assert!(fake.wait(ms(2_000), |f| sets(f).len() == 1).await);
            assert!(
                woke.elapsed() < ms(500),
                "{:?}: not at once",
                woke.elapsed()
            );
            assert_eq!(sets(&fake), [GAME]);
        });
    }

    /// L17d: a `scene` offered while the handshake waits for Hello: never
    /// switched (D23), `lastSkip {scene, notConnected}`.
    #[test]
    fn l17d_a_scene_during_the_handshake_is_never_run() {
        guarded(async {
            let fake = FakeObs::start(Config::default()).await;
            fake.set_hello(Stage::After(ms(300)));
            let link = start(&fake, None, ObsFeed::default());
            tokio::time::sleep(ms(100)).await;
            link.op("brb", scene_op(BRB, "BRB"));
            link.connected().await;
            tokio::time::sleep(ms(300)).await;
            assert_eq!(sets(&fake), [""; 0]);
            assert!(fake.requests_of("GetCurrentProgramScene").is_empty());
            assert_eq!(
                skip_of(&link.now()),
                Some((SkipOp::Scene, SkipWhy::NotConnected, "brb".into()))
            );
        });
    }

    /// L17e: a switch back whose Get OBS stalls, then OBS closes: the back
    /// is due again with its first stamp, and runs after the reconnect.
    #[test]
    fn l17e_a_cut_off_switch_back_keeps_its_slot_and_stamp() {
        guarded(async {
            let fake = FakeObs::start(Config::default()).await;
            fake.model(|m| m.program = BRB.into());
            let link = start(&fake, None, ObsFeed::default());
            assert!(link
                .store
                .after_scene("brb", scene(GAME, "Game"), scene(BRB, "BRB")));
            link.connected().await;
            fake.script(Rule::on("GetCurrentProgramScene").times(1).drop_answer());
            let stamp = Instant::now();
            link.op_at("brb", ObsOp::SceneBack, stamp);
            assert!(
                fake.wait_requests("GetCurrentProgramScene", 1, ms(1_000))
                    .await
            );
            tokio::time::sleep(ms(300)).await;
            let grace = timing().scene_back_grace;
            let expiry = link.store.next_expiry().unwrap();
            assert!(
                expiry <= stamp + grace + ms(1),
                "re-marked with a later stamp"
            );
            fake.close_all(1001, "");
            assert!(fake.wait(ms(3_000), |f| sets(f).len() == 1).await);
            assert_eq!(sets(&fake), [GAME]);
            assert_eq!(fake.program(), GAME);

            // A Set cut off by the session's end commits nothing: the back
            // is due again, and goes after the reconnect.
            let fake = FakeObs::start(Config::default()).await;
            fake.model(|m| m.program = BRB.into());
            let link = start(&fake, None, ObsFeed::default());
            assert!(link
                .store
                .after_scene("brb", scene(GAME, "Game"), scene(BRB, "BRB")));
            link.connected().await;
            fake.script(Rule::on("SetCurrentProgramScene").times(1).close(1001));
            link.op("brb", ObsOp::SceneBack);
            assert!(fake.wait_identified(2, ms(3_000)).await);
            assert!(fake.wait(ms(2_000), |f| sets(f).len() == 2).await);
            assert_eq!(sets(&fake), [GAME, GAME]);
            assert_eq!(fake.program(), GAME);
        });
    }

    /// D4 (the link's half of L33): once a newer generation has started, a
    /// driver applies nothing more: an output event's op is dropped, and the
    /// driver lets its connection go.
    #[test]
    fn a_stale_driver_applies_nothing() {
        guarded(async {
            let fake = FakeObs::start(Config::default()).await;
            let link = start(&fake, None, follows(&[("up", FollowObs::Stream)], 0));
            link.connected().await;
            assert!(ops_count(&link, 1).await);
            let cfg = LinkConfig {
                ip: IpAddr::V4(Ipv4Addr::LOCALHOST),
                port: fake.port(),
            };
            let _ = link.status.cell.lock().unwrap().begin(&cfg);
            fake.stream_state(Out::Started);
            assert!(fake.wait_ended(1, ms(1_000)).await);
            tokio::time::sleep(ms(100)).await;
            assert_eq!(link.timers.ops().len(), 1, "{:?}", link.timers.ops());
            assert_eq!(fake.accepts(), 1);
        });
    }

    // ---- Follow OBS (L18, and the seeds' link half) ---------------------------------

    fn follows(list: &[(&str, FollowObs)], sleeps: u64) -> ObsFeed {
        ObsFeed {
            follows: list
                .iter()
                .map(|(id, what)| Follow {
                    id: (*id).into(),
                    what: *what,
                })
                .collect(),
            sleeps,
            ..ObsFeed::default()
        }
    }

    /// Waits for the timer ops to number `n`.
    async fn ops_count(link: &Link, n: usize) -> bool {
        let end = Instant::now() + ms(1_000);
        while Instant::now() < end {
            if link.timers.ops().len() >= n {
                return true;
            }
            tokio::time::sleep(ms(2)).await;
        }
        false
    }

    /// L18: output events drive the count-ups that follow them (stub
    /// timers): at connect each idle output pauses its follow (a pause the
    /// core ignores unless it runs); STARTED from 0, RECONNECTING nothing,
    /// RECONNECTED a `GetStreamStatus` seed, STOPPED a pause; a recording's
    /// PAUSED and RESUMED; and a STARTED read while a push stalls applies at
    /// once.
    #[test]
    fn l18_follow_obs_applies_events_as_they_come() {
        guarded(async {
            let fake = FakeObs::start(Config::default()).await;
            let feed = follows(&[("up", FollowObs::Stream), ("rec", FollowObs::Record)], 0);
            let link = start(&fake, None, feed);
            link.connected().await;
            assert!(ops_count(&link, 2).await);
            assert_eq!(
                link.timers.ops(),
                [
                    ("up".into(), TimerOp::Pause),
                    ("rec".into(), TimerOp::Pause)
                ]
            );
            fake.stream_state(Out::Started);
            assert!(ops_count(&link, 3).await);
            let (id, op) = link.timers.ops()[2].clone();
            assert_eq!(id, "up");
            assert!(
                matches!(op, TimerOp::SetElapsed { ms, running: true } if ms < 100),
                "{op:?}"
            );
            assert_eq!(link.now().stream, Some(super::StreamState::Live));
            fake.stream_state(Out::Reconnecting);
            tokio::time::sleep(ms(100)).await;
            assert_eq!(link.timers.ops().len(), 3);
            fake.model(|m| m.stream.duration_ms = 61_000);
            fake.stream_state(Out::Reconnected);
            assert!(ops_count(&link, 4).await);
            assert_eq!(fake.requests_of("GetStreamStatus").len(), 2);
            assert_eq!(
                link.timers.ops()[3],
                (
                    "up".into(),
                    TimerOp::SetElapsed {
                        ms: 61_000,
                        running: true
                    }
                )
            );
            fake.stream_state(Out::Stopped);
            assert!(ops_count(&link, 5).await);
            assert_eq!(link.timers.ops()[4], ("up".into(), TimerOp::Pause));
            fake.record_state(Out::Paused);
            fake.record_state(Out::Resumed);
            assert!(ops_count(&link, 7).await);
            assert_eq!(
                link.timers.ops()[5..],
                [
                    ("rec".into(), TimerOp::Pause),
                    ("rec".into(), TimerOp::Resume)
                ]
            );

            fake.script(Rule::on("SetInputSettings").times(1).delay(ms(500)));
            let mut feed = follows(&[("up", FollowObs::Stream), ("rec", FollowObs::Record)], 0);
            feed.texts = texts_feed(&[(TEXT_V3, "1")]).texts;
            link.feed.send_replace(Arc::new(feed));
            assert!(fake.wait(ms(1_000), |f| pushes(f).len() == 1).await);
            let stalled = Instant::now();
            fake.record_state(Out::Started);
            assert!(ops_count(&link, 8).await, "{:?}", link.timers.ops());
            let applied = link.timers.0.lock().unwrap()[7].2;
            assert!(
                applied < stalled + ms(150),
                "applied {:?} into the stall",
                applied - stalled
            );
        });
    }

    /// D26 (the link half of L19, stub timers): the startup's answers seed
    /// every follow; a follow new to the feed is seeded by its status
    /// request; a change of the feed's sleep count re-seeds them all.
    #[test]
    fn follows_are_seeded_at_connect_on_a_new_follow_and_after_a_sleep() {
        guarded(async {
            let mut model = Model::standard();
            model.record = Output {
                active: true,
                duration_ms: 125_000,
                ..Output::default()
            };
            let fake = FakeObs::start(Config::default().model(model)).await;
            let link = start(&fake, None, follows(&[("rec", FollowObs::Record)], 0));
            link.connected().await;
            assert!(ops_count(&link, 1).await);
            assert_eq!(
                link.timers.ops()[0],
                (
                    "rec".into(),
                    TimerOp::SetElapsed {
                        ms: 125_000,
                        running: true
                    }
                )
            );
            assert_eq!(link.now().record, Some(super::RecordState::Recording));
            let feed = follows(&[("rec", FollowObs::Record), ("up", FollowObs::Stream)], 0);
            link.feed.send_replace(Arc::new(feed));
            assert!(fake.wait_requests("GetStreamStatus", 2, ms(1_000)).await);
            assert!(ops_count(&link, 2).await);
            assert_eq!(link.timers.ops()[1], ("up".into(), TimerOp::Pause));
            let slept = follows(&[("rec", FollowObs::Record), ("up", FollowObs::Stream)], 1);
            link.feed.send_replace(Arc::new(slept));
            assert!(fake.wait_requests("GetRecordStatus", 2, ms(1_000)).await);
            assert!(fake.wait_requests("GetStreamStatus", 3, ms(1_000)).await);
            assert!(ops_count(&link, 4).await);
            let ops = link.timers.ops();
            assert_eq!(ops.len(), 4);
            assert!(ops[2..].contains(&(
                "rec".into(),
                TimerOp::SetElapsed {
                    ms: 125_000,
                    running: true
                }
            )));
            assert!(ops[2..].contains(&("up".into(), TimerOp::Pause)));
        });
    }

    // ---- How a connection ends (L20, L21, L30, L31) --------------------------------

    /// L20: OBS answers no ping and sends nothing: `silent` 400 ms to 1 s
    /// in. The variant: no pongs, but an event every 100 ms keeps it up (any
    /// frame counts, D8).
    #[test]
    fn l20_keepalive_counts_any_frame() {
        guarded(async {
            let fake = FakeObs::start(Config::default()).await;
            fake.set_ignore_pings(true);
            let link = start(&fake, None, ObsFeed::default());
            assert!(fake.wait_identified(1, ms(2_000)).await);
            let identified = Instant::now();
            assert!(link.state(ObsState::Retrying, ms(2_000)).await);
            assert_eq!(link.now().reason, Some(ObsReason::Silent));
            let took = identified.elapsed();
            assert!(took >= ms(380) && took <= ms(1_000), "{took:?}");
            assert!(fake.pongs().1 >= 1, "the fake held a pong back");

            let fake = FakeObs::start(Config::default()).await;
            fake.set_ignore_pings(true);
            let link = start(&fake, None, ObsFeed::default());
            link.connected().await;
            for i in 0..15 {
                fake.rename_scene(JUST_CHATTING, &format!("Chat {i}"));
                tokio::time::sleep(ms(100)).await;
            }
            assert_eq!(link.now().state, ObsState::Connected);
            assert_eq!(fake.accepts(), 1);
        });
    }

    /// L21: a 5 MiB frame: `tooBig`, then a reconnect.
    #[test]
    fn l21_a_frame_over_4_mib_ends_the_connection() {
        guarded(async {
            let fake = FakeObs::start(Config::default()).await;
            let link = start(&fake, None, ObsFeed::default());
            link.connected().await;
            fake.send_big_frame(5 << 20);
            assert!(link.state(ObsState::Retrying, ms(3_000)).await);
            assert_eq!(link.now().reason, Some(ObsReason::TooBig));
            assert!(fake.wait_identified(2, ms(2_000)).await);
            assert!(link.state(ObsState::Connected, ms(2_000)).await);
        });
    }

    /// L30: `ExitStarted`, then OBS's server stops (1001): `obsExiting`.
    #[test]
    fn l30_obs_exiting() {
        guarded(async {
            let fake = FakeObs::start(Config::default()).await;
            let link = start(&fake, None, ObsFeed::default());
            link.connected().await;
            fake.exit_started();
            assert!(
                link.until(ms(1_000), |s| s.reason == Some(ObsReason::ObsExiting))
                    .await
            );
            fake.close_all(1001, "");
            assert!(link.state(ObsState::Retrying, ms(2_000)).await);
            assert_eq!(link.now().reason, Some(ObsReason::ObsExiting));
        });
    }

    /// L31: 10 000 `SceneNameChanged` while a request stalls: the queue (256)
    /// fills and the connection ends `protocol`; `inventoryRev` never moved
    /// more than once per coalesce period (100 ms).
    #[test]
    fn l31_an_event_flood_ends_the_connection() {
        guarded(async {
            let fake = FakeObs::start(Config::default()).await;
            let link = start(&fake, None, ObsFeed::default());
            link.connected().await;
            fake.script(Rule::on("SetInputSettings").times(1).drop_answer());
            link.texts(&[(TEXT_V3, "1")]);
            assert!(fake.wait(ms(1_000), |f| pushes(f).len() == 1).await);
            fake.flood_scene_renames(10_000);
            assert!(link.state(ObsState::Retrying, ms(900)).await);
            assert_eq!(link.now().reason, Some(ObsReason::Protocol));
            let seen = link.status.seen.lock().unwrap().clone();
            let bumps: Vec<Instant> = seen
                .windows(2)
                .filter(|w| w[1].2.inventory_rev != w[0].2.inventory_rev)
                .map(|w| w[1].0)
                .collect();
            for w in bumps.windows(2) {
                assert!(w[1] - w[0] >= ms(90), "{:?} apart", w[1] - w[0]);
            }
            assert!(bumps.len() <= 2, "{} bumps", bumps.len());
        });
    }

    // ---- The stop (L23, L23b, L23c, L35) -------------------------------------

    /// When the fake read the client's close, and its code.
    fn client_close(fake: &FakeObs) -> Option<(Instant, Option<u16>)> {
        fake.log().iter().find_map(|r| match (&r.dir, &r.frame) {
            (Dir::In, Frame::Close(code, _)) => Some((r.at, *code)),
            _ => None,
        })
    }

    /// L23: the stop: OBS receives close 1001, and the ack comes after it
    /// did.
    #[test]
    fn l23_a_stop_closes_with_1001_then_acks() {
        guarded(async {
            let fake = FakeObs::start(Config::default()).await;
            let mut link = start(&fake, None, ObsFeed::default());
            link.connected().await;
            link.stop();
            let acked = link.ack(ms(1_000)).await.expect("an ack");
            let (closed, code) = client_close(&fake).expect("a close frame");
            assert_eq!(code, Some(1001));
            assert!(acked >= closed, "acked before OBS read the close");
            assert!(fake.wait_ended(1, ms(500)).await);
            assert_eq!(fake.sessions()[0].end, Some(End::ClientClosed(Some(1001))));
        });
    }

    /// L23b: a stop when OBS neither echoes nor closes: the link is done
    /// within the flush and the echo's bounds (300 ms scaled); OBS sees the
    /// 1001, then the end of the stream.
    #[test]
    fn l23b_a_stop_never_waits_on_obs() {
        guarded(async {
            let fake = FakeObs::start(Config::default()).await;
            fake.set_close_echo(false);
            let mut link = start(&fake, None, ObsFeed::default());
            link.connected().await;
            let stopped = Instant::now();
            link.stop();
            let acked = link.ack(ms(1_000)).await.expect("an ack");
            assert!(acked - stopped <= ms(400), "{:?}", acked - stopped);
            assert!(
                fake.wait(ms(1_000), |f| f.log().iter().any(|r| r.frame == Frame::Eof))
                    .await
            );
            let ins: Vec<Frame> = fake
                .log()
                .into_iter()
                .filter(|r| r.dir == Dir::In && matches!(r.frame, Frame::Close(..) | Frame::Eof))
                .map(|r| r.frame)
                .collect();
            assert_eq!(ins, [Frame::Close(Some(1001), String::new()), Frame::Eof]);
        });
    }

    /// L23c: a stop inside a stalled request, an inventory, the startup's
    /// 207 loop and the handshake: within 300 ms (+ slack) a close 1001 —
    /// or, before Identified, no frame — and nothing sent after it.
    #[test]
    fn l23c_a_stop_is_seen_inside_every_wait() {
        guarded(async {
            for case in ["request", "inventory", "207", "handshake"] {
                let fake = FakeObs::start(Config::default()).await;
                match case {
                    "request" => fake.script(Rule::on("SetInputSettings").drop_answer()),
                    "inventory" => fake.script(Rule::on("GetInputList").delay(ms(3_000))),
                    "207" => fake.set_ready(false),
                    _ => fake.set_hello(Stage::Never),
                }
                let feed = if case == "request" {
                    texts_feed(&[(TEXT_V3, "1")])
                } else {
                    ObsFeed::default()
                };
                let mut link = start(&fake, None, feed);
                let _answer;
                match case {
                    "request" => {
                        link.connected().await;
                        assert!(fake.wait(ms(1_000), |f| pushes(f).len() == 1).await);
                    }
                    "inventory" => {
                        link.connected().await;
                        _answer = link.inventory();
                        assert!(fake.wait_requests("GetInputList", 1, ms(1_000)).await);
                    }
                    "207" => assert!(fake.wait_requests("GetVersion", 3, ms(1_000)).await),
                    _ => assert!(
                        fake.wait(ms(1_000), |f| f
                            .sessions()
                            .first()
                            .is_some_and(|s| s.upgraded))
                            .await
                    ),
                }
                let stopped = Instant::now();
                link.stop();
                let acked = link.ack(ms(1_000)).await.expect("an ack");
                assert!(acked - stopped <= ms(400), "{case}: {:?}", acked - stopped);
                assert!(fake.wait_ended(1, ms(1_000)).await, "{case}");
                let log = fake.log();
                let ins: Vec<&Frame> = log
                    .iter()
                    .filter(|r| r.dir == Dir::In)
                    .map(|r| &r.frame)
                    .collect();
                let at = ins
                    .iter()
                    .position(|f| matches!(f, Frame::Close(..) | Frame::Eof | Frame::Error(_)))
                    .unwrap_or_else(|| panic!("{case}: {ins:?} {:?}", fake.sessions()));
                assert!(
                    ins[at + 1..]
                        .iter()
                        .all(|f| matches!(f, Frame::Eof | Frame::Error(_))),
                    "{case}: {ins:?}"
                );
                if case == "handshake" {
                    // The socket dropped: a FIN, or a reset.
                    assert!(
                        matches!(ins[at], Frame::Eof | Frame::Error(_)),
                        "{case}: {ins:?}"
                    );
                } else {
                    assert_eq!(*ins[at], Frame::Close(Some(1001), String::new()), "{case}");
                }
                tokio::time::sleep(ms(200)).await;
                assert_eq!(fake.accepts(), 1, "{case}");
            }
        });
    }

    /// L35: the feed's sender dropped (the timer thread never started): the
    /// driver keeps serving ops, and its wait isn't polled in a loop.
    #[test]
    fn l35_a_closed_feed_never_spins_the_driver() {
        guarded(async {
            let fake = FakeObs::start(Config::default()).await;
            let mut link = start(&fake, None, ObsFeed::default());
            link.connected().await;
            let (gone, _) = watch::channel(Arc::new(ObsFeed::default()));
            drop(std::mem::replace(&mut link.feed, gone));
            tokio::time::sleep(ms(50)).await;
            let before = polls();
            tokio::time::sleep(ms(500)).await;
            let idle = polls() - before;
            assert!(idle < 200, "{idle} polls in 500 ms");
            link.op("brb", scene_op(BRB, "BRB"));
            assert!(fake.wait(ms(1_000), |f| sets(f).len() == 1).await);
        });
    }

    // ---- The inventory (L24, L24b) -----------------------------------------------

    /// L24: the text sources — v0, v3, `FreeType` 2 v2 — listed, the browser
    /// source not, the file-reading one marked; scenes top first; with and
    /// without `unversionedInputKind`; nothing of a source's text or path.
    #[test]
    fn l24_the_inventory_lists_text_sources_and_scenes() {
        guarded(async {
            for model in [
                Model::standard(),
                Model::standard().without_unversioned_kinds(),
            ] {
                let fake = FakeObs::start(Config::default().model(model)).await;
                let link = start(&fake, None, ObsFeed::default());
                link.connected().await;
                let inventory = link.inventory().await.unwrap().unwrap();
                let names: Vec<&str> = inventory.scenes.iter().map(|s| s.name.as_str()).collect();
                assert_eq!(names, ["Game", "BRB", "Just Chatting"]);
                assert_eq!(inventory.program_scene_uuid.as_deref(), Some(GAME));
                let sources: Vec<(&str, TextKind, bool)> = inventory
                    .text_sources
                    .iter()
                    .map(|s| (s.uuid.as_str(), s.kind, s.reads_from_file))
                    .collect();
                assert_eq!(
                    sources,
                    [
                        (TEXT_V0, TextKind::Gdiplus, false),
                        (TEXT_V3, TextKind::Gdiplus, false),
                        (TEXT_FROM_FILE, TextKind::Gdiplus, true),
                        (TEXT_FT2, TextKind::Ft2, false),
                    ]
                );
                assert!(!inventory.text_sources.iter().any(|s| s.uuid == BROWSER));
                assert!(!inventory.truncated);
                assert_eq!(inventory.rev, link.now().inventory_rev);
                let json = serde_json::to_string(&inventory).unwrap();
                assert!(!json.contains(SOURCE_TEXT) && !json.contains("zz-user"));
                fake.assert_clean();
            }
        });
    }

    /// L24b: an inventory goes one request per turn: a `scene` offered
    /// during a 100-source run reaches OBS within one request; a second
    /// caller joins the run; a run whose callers have all gone stops.
    #[test]
    fn l24b_the_inventory_interleaves() {
        guarded(async {
            let model = Model::standard().with_text_sources(100);
            let fake = FakeObs::start(Config::default().model(model)).await;
            let link = start(&fake, None, ObsFeed::default());
            link.connected().await;
            fake.script(Rule::on("GetInputSettings").delay(ms(5)));
            let first = link.inventory();
            let second = link.inventory();
            assert!(fake.wait_requests("GetInputSettings", 10, ms(2_000)).await);
            let before = fake.requests_of("GetInputSettings").len();
            link.op("brb", scene_op(BRB, "BRB"));
            assert!(
                fake.wait_requests("GetCurrentProgramScene", 1, ms(1_000))
                    .await
            );
            let between = fake.requests_of("GetInputSettings").len() - before;
            assert!(between <= 1, "{between} settings requests went first");
            let (a, b) = (
                first.await.unwrap().unwrap(),
                second.await.unwrap().unwrap(),
            );
            assert_eq!(a, b);
            assert!(a.truncated);
            assert_eq!(a.text_sources.len(), 100);
            assert_eq!(fake.requests_of("GetSceneList").len(), 1);
            assert_eq!(fake.requests_of("GetInputSettings").len(), 100);
            assert_eq!(sets(&fake), [BRB]);

            let gone = link.inventory();
            assert!(fake.wait_requests("GetInputSettings", 110, ms(2_000)).await);
            drop(gone);
            tokio::time::sleep(ms(100)).await;
            let stopped = fake.requests_of("GetInputSettings").len();
            tokio::time::sleep(ms(300)).await;
            assert_eq!(fake.requests_of("GetInputSettings").len(), stopped);
            assert!(stopped < 200);
            fake.assert_clean();
        });
    }

    // ---- Clears (L25, L25b) ---------------------------------------------------

    /// L25: a source that leaves the feed while connected gets `""` once.
    #[test]
    fn l25_an_unmapped_source_is_cleared_once() {
        guarded(async {
            let fake = FakeObs::start(Config::default()).await;
            let link = start(&fake, None, texts_feed(&[(TEXT_V3, "2:00")]));
            link.connected().await;
            assert!(fake.wait(ms(1_000), |f| pushes(f).len() == 1).await);
            assert!(link.store.written().contains(TEXT_V3));
            link.texts(&[]);
            assert!(fake.wait(ms(1_000), |f| pushes(f).len() == 2).await);
            tokio::time::sleep(ms(300)).await;
            let all = pushes(&fake);
            assert_eq!(all.len(), 2);
            assert_eq!((all[1].0.as_str(), all[1].1.as_str()), (TEXT_V3, ""));
            assert!(link.store.written().is_empty());
        });
    }

    /// L25b (the link's half): a source unmapped while OBS was away is
    /// cleared once at the next connection.
    #[test]
    fn l25b_a_source_unmapped_while_disconnected_is_cleared_on_reconnect() {
        guarded(async {
            let fake = FakeObs::start(Config::default()).await;
            let link = start(&fake, None, texts_feed(&[(TEXT_V3, "2:00")]));
            link.connected().await;
            assert!(fake.wait(ms(1_000), |f| pushes(f).len() == 1).await);
            fake.set_accept(Accept::CloseAtOnce);
            fake.close_all(1001, "");
            assert!(link.state(ObsState::Retrying, ms(1_000)).await);
            link.texts(&[(TEXT_FT2, "0:01")]);
            fake.set_accept(Accept::Normal);
            assert!(fake.wait_identified(2, ms(3_000)).await);
            assert!(fake.wait(ms(1_000), |f| pushes(f).len() == 3).await);
            tokio::time::sleep(ms(300)).await;
            let again: Vec<(String, String)> = pushes(&fake)[1..]
                .iter()
                .map(|(u, t, _)| (u.clone(), t.clone()))
                .collect();
            assert_eq!(
                again,
                [
                    (TEXT_FT2.into(), "0:01".into()),
                    (TEXT_V3.into(), String::new())
                ]
            );
        });
    }

    // ---- Secrets (§10.3) -----------------------------------------------------------

    /// §10.3: L1's and L2's flows, a push and an inventory, under a TRACE
    /// capture (the blocking pool's lines included): the password, its
    /// encodings, the secret, the auth strings, the pushed text and the
    /// source's text and path appear in no log line, status or answer.
    #[test]
    fn secrets_never_reach_a_log_line_or_a_status() {
        const PASSWORD: &str = "correct horse \u{2713} 7Qz";
        let ((collected, mut needles), log) = traced(|| {
            guarded(async {
                let mut needles = secret_forms(PASSWORD);
                let mut collected = String::new();
                for (stored, connects) in [(PASSWORD, true), ("not it", false)] {
                    let fake = FakeObs::start(Config::default().with_password(PASSWORD)).await;
                    let link = start(&fake, Some(stored), ObsFeed::default());
                    if connects {
                        link.connected().await;
                        link.texts(&[(TEXT_V3, "zz-pushed-zz")]);
                        assert!(fake.wait(ms(1_000), |f| pushes(f).len() == 1).await);
                        let inventory = link.inventory().await.unwrap().unwrap();
                        collected.push_str(&serde_json::to_string(&inventory).unwrap());
                    } else {
                        assert!(link.state(ObsState::AuthFailed, ms(3_000)).await);
                    }
                    for s in link.seen() {
                        collected.push_str(&serde_json::to_string(&s).unwrap());
                    }
                    let secret = fake::secret(PASSWORD, &fake.salt());
                    for session in fake.sessions() {
                        if let Some(challenge) = &session.challenge {
                            needles.push(fake::auth(&secret, challenge));
                        }
                    }
                    needles.push(secret);
                }
                (collected, needles)
            })
        });
        needles.extend(["zz-pushed-zz", SOURCE_TEXT, "zz-user"].map(String::from));
        assert!(log.contains("obs: connected"), "the capture saw the link");
        assert_none_in(&log, &needles);
        assert_none_in(&collected, &needles);
    }
}
