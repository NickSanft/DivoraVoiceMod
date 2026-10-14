//! The screen recorder's audio tap (v1.52.0).
//!
//! While a screen recording runs, the output callback copies two signals per
//! buffer, each into its own preallocated ring: the **voice** — the chain's
//! output, before the soundboard is folded in — and the **soundboard**. A third
//! ring takes one [`AvMark`] per buffer saying when that buffer ran and how
//! stale its voice was. A per-session drain thread turns marks and frames into
//! [`AvChunk`]s and hands them to the recorder.
//!
//! Two signals rather than the mix, because they are late by different
//! amounts. The voice came through the input device, the input ring and the
//! effect chain — a bring-your-own non-streaming voice model alone adds about
//! 256 ms — while a soundboard sting is rendered in the very buffer that plays
//! it. The recorder places each at its own time; mixed here, the sting would be
//! dragged late with the voice or the voice left early with the sting.
//!
//! Callback cost while recording: three ring pushes per buffer, plus one clock
//! read per take (the anchor that puts cpal's timestamps on the clock the
//! video uses). While not recording: nothing — the caller's one atomic load
//! decides. Nothing here allocates, frees, locks or makes a syscall on the
//! callback side; `the_tap_allocates_nothing` pins it.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, TryRecvError};
use std::sync::Arc;
use std::time::{Duration, Instant};

use ringbuf::traits::{Consumer, Observer, Producer, Split};
use ringbuf::HeapRb;

use super::state::EngineState;

/// Frames each of the voice and soundboard rings holds: about 680 ms at
/// 48 kHz, so a drain thread descheduled for a long moment still loses
/// nothing. Allocated per session whether or not anything records — 128 KiB
/// a ring.
const AV_RING_FRAMES: usize = 1 << 15;

/// Marks the side ring holds: one per callback, so 1024 covers the frame rings
/// above even at 1 ms device buffers.
const AV_MARKS: usize = 1024;

/// How often the drain thread forwards what the callback tapped.
const AV_DRAIN_INTERVAL_MS: u64 = 5;

/// A callback timestamp: cpal's `StreamInstant` in the engine. A trait only so
/// the tests can use something else — cpal has no public constructor for one.
pub trait Stamp: Copy + Send + 'static {
    /// How long after `earlier` this is; `None` if it is before it.
    fn since(&self, earlier: &Self) -> Option<Duration>;
}

impl Stamp for cpal::StreamInstant {
    fn since(&self, earlier: &Self) -> Option<Duration> {
        self.duration_since(earlier)
    }
}

impl Stamp for Duration {
    fn since(&self, earlier: &Self) -> Option<Duration> {
        self.checked_sub(*earlier)
    }
}

/// What the output callback knows about the buffer it is tapping.
#[derive(Debug, Clone, Copy)]
pub struct AvBuffer<S> {
    /// When cpal says this callback ran. On WASAPI that is the QPC reading
    /// `IAudioClock` took for this buffer, so it costs the callback nothing.
    pub at: S,
    /// Device frames this callback writes. Lets the recorder cancel
    /// scheduling jitter: a callback that runs late writes more.
    pub out_frames: u32,
    /// Native-rate frames in the input ring before this callback took any —
    /// how long the oldest voice frame in this buffer had been waiting.
    pub occupancy: u32,
    /// Frames the effect chain delays the voice by.
    pub chain_latency: u32,
}

/// One tapped buffer's bookkeeping, pushed after its frames.
#[derive(Debug, Clone, Copy)]
struct AvMark<S> {
    buf: AvBuffer<S>,
    /// Set on the first buffer of a run: the same moment as `buf.at`, on the
    /// clock the video uses. Everything later in the run is placed from this
    /// plus cpal's own differences, so this is the only clock read.
    anchor: Option<Instant>,
    /// Engine-rate frame index of this buffer's first frame within the run,
    /// counting frames that were dropped.
    index: u64,
    span: u32,
}

type FrameProducer = <HeapRb<f32> as Split>::Prod;
type FrameConsumer = <HeapRb<f32> as Split>::Cons;
type MarkProducer<S> = <HeapRb<AvMark<S>> as Split>::Prod;
type MarkConsumer<S> = <HeapRb<AvMark<S>> as Split>::Cons;

/// The callback's end of the tap.
pub struct AvTap<S> {
    voice: FrameProducer,
    sb: FrameProducer,
    marks: MarkProducer<S>,
    /// Whether the last armed buffer was taken. False means the next one
    /// starts a run: index 0, and an anchor.
    running: bool,
    index: u64,
}

impl<S: Copy> AvTap<S> {
    /// Tap one buffer. `voice` and `sb` are this buffer's engine-rate frames,
    /// the same length; `now` is read only on the first buffer of a run.
    ///
    /// All or nothing: a buffer goes into all three rings or none of them, so
    /// the drain can never pair a mark with another buffer's frames. A buffer
    /// that does not fit is counted in `dropped` and leaves a gap in the
    /// run's index, which the recorder fills with silence.
    pub fn tap(
        &mut self,
        armed: bool,
        buf: AvBuffer<S>,
        voice: &[f32],
        sb: &[f32],
        dropped: &AtomicU64,
        now: impl FnOnce() -> Instant,
    ) {
        if !armed {
            self.running = false;
            return;
        }
        let span = voice.len().min(sb.len());
        let first = !self.running;
        if first {
            self.index = 0;
        }
        let fits = self.marks.vacant_len() > 0
            && self.voice.vacant_len() >= span
            && self.sb.vacant_len() >= span;
        if fits {
            let _ = self.voice.push_slice(&voice[..span]);
            let _ = self.sb.push_slice(&sb[..span]);
            let _ = self.marks.try_push(AvMark {
                buf,
                anchor: first.then(now),
                index: self.index,
                span: u32::try_from(span).unwrap_or(u32::MAX),
            });
            self.running = true;
        } else {
            // A run begins with an anchored mark; if that one is the buffer
            // that did not fit, `running` stays false and the next buffer
            // starts the run instead.
            dropped.fetch_add(span as u64, Ordering::Relaxed);
        }
        if self.running {
            self.index += span as u64;
        }
    }
}

/// One output buffer's worth of audio for the screen recorder.
#[derive(Debug, Clone)]
pub struct AvChunk {
    /// Changes whenever the timeline restarts: a new take, or the same take
    /// after a device-loss rebuild. Process-unique.
    pub epoch: u64,
    /// When the callback that produced this ran.
    pub at: Instant,
    /// Engine-rate frame index of `voice[0]` / `sb[0]` within the epoch.
    /// A jump from one chunk to the next is frames that were dropped.
    pub index: u64,
    /// The engine rate (the input device's).
    pub rate: u32,
    /// Device frames the callback wrote, at `out_rate`.
    pub out_frames: u32,
    pub out_rate: u32,
    /// Native frames waiting in the input ring before this buffer took any.
    pub occupancy: u32,
    /// Frames the effect chain delays the voice by.
    pub chain_latency: u32,
    /// How old the newest captured frame was when it reached the ring, on
    /// average (measured by the input callback).
    pub input_latency: Duration,
    /// The chain's output — what the call hears from the mic.
    pub voice: Vec<f32>,
    /// The soundboard and Speak voices the call hears.
    pub sb: Vec<f32>,
}

/// Process-wide, so two sessions' drains can never hand out the same epoch.
static NEXT_EPOCH: AtomicU64 = AtomicU64::new(1);

/// The run the drain is currently placing: its epoch and its anchor pair.
struct Run<S> {
    epoch: u64,
    at: S,
    anchor: Instant,
}

/// The drain thread's end of the tap.
pub struct AvDrain<S> {
    voice: FrameConsumer,
    sb: FrameConsumer,
    marks: MarkConsumer<S>,
    run: Option<Run<S>>,
    rate: u32,
    out_rate: u32,
}

impl<S: Stamp> AvDrain<S> {
    /// Turn every buffer waiting into a chunk, oldest first.
    ///
    /// Buffers from before the current run's anchor — left over from an
    /// earlier take — are dropped rather than placed against a clock they
    /// were never measured on.
    pub fn drain(&mut self, input_latency: Duration, mut emit: impl FnMut(AvChunk)) {
        while let Some(mark) = self.marks.try_pop() {
            let span = mark.span as usize;
            let mut voice = vec![0.0; span];
            let mut sb = vec![0.0; span];
            // The callback pushed these before the mark, so they are here.
            let _ = self.voice.pop_slice(&mut voice);
            let _ = self.sb.pop_slice(&mut sb);
            if let Some(anchor) = mark.anchor {
                self.run = Some(Run {
                    epoch: NEXT_EPOCH.fetch_add(1, Ordering::Relaxed),
                    at: mark.buf.at,
                    anchor,
                });
            }
            let Some(run) = &self.run else {
                continue;
            };
            let at = run.anchor + mark.buf.at.since(&run.at).unwrap_or_default();
            emit(AvChunk {
                epoch: run.epoch,
                at,
                index: mark.index,
                rate: self.rate,
                out_frames: mark.buf.out_frames,
                out_rate: self.out_rate,
                occupancy: mark.buf.occupancy,
                chain_latency: mark.buf.chain_latency,
                input_latency,
                voice,
                sb,
            });
        }
    }

    /// Throw away everything waiting and forget the run. The next chunk
    /// comes from the next anchored buffer.
    pub fn discard(&mut self) {
        while self.marks.try_pop().is_some() {}
        self.voice.clear();
        self.sb.clear();
        self.run = None;
    }
}

/// Both ends of a fresh tap for a session at `rate` (engine) and `out_rate`
/// (output device).
#[must_use]
pub fn av_tap<S: Copy>(rate: u32, out_rate: u32) -> (AvTap<S>, AvDrain<S>) {
    let (voice_p, voice_c) = HeapRb::<f32>::new(AV_RING_FRAMES).split();
    let (sb_p, sb_c) = HeapRb::<f32>::new(AV_RING_FRAMES).split();
    let (marks_p, marks_c) = HeapRb::<AvMark<S>>::new(AV_MARKS).split();
    (
        AvTap {
            voice: voice_p,
            sb: sb_p,
            marks: marks_p,
            running: false,
            index: 0,
        },
        AvDrain {
            voice: voice_c,
            sb: sb_c,
            marks: marks_c,
            run: None,
            rate,
            out_rate,
        },
    )
}

/// Engine thread → a session's drain thread.
pub(crate) enum AvControl {
    /// Start forwarding to this recorder.
    Attach(SyncSender<AvChunk>),
    /// Forward what is waiting, then stop and let go of the recorder.
    Detach,
}

/// Body of a session's `divora-av-tap` thread.
///
/// Idle — no recorder attached — it blocks on its control channel and costs
/// nothing. Attached, it forwards every [`AV_DRAIN_INTERVAL_MS`]. It exits
/// when the session's control sender drops at teardown; the recorder's sender
/// lives on the engine thread, outside the session, which is how a
/// device-loss rebuild re-attaches the same take instead of silencing it.
#[allow(clippy::needless_pass_by_value)] // owns its ends for the thread's lifetime
pub(crate) fn av_drain_thread<S: Stamp>(
    mut drain: AvDrain<S>,
    ctl: Receiver<AvControl>,
    state: Arc<EngineState>,
) {
    let mut sink: Option<SyncSender<AvChunk>> = None;
    loop {
        let Some(tx) = sink.as_ref() else {
            match ctl.recv() {
                Ok(AvControl::Attach(tx)) => sink = Some(tx),
                Ok(AvControl::Detach) => drain.discard(),
                Err(_) => return,
            }
            continue;
        };
        forward(&mut drain, tx, &state);
        match ctl.try_recv() {
            Ok(AvControl::Attach(tx)) => sink = Some(tx),
            Ok(AvControl::Detach) => {
                // `forward` above already sent what the callback had
                // pushed; anything later belongs to no take.
                sink = None;
                drain.discard();
            }
            Err(TryRecvError::Empty) => {
                std::thread::sleep(Duration::from_millis(AV_DRAIN_INTERVAL_MS));
            }
            Err(TryRecvError::Disconnected) => return,
        }
    }
}

/// Send everything waiting to the recorder. A full channel — a recorder that
/// stopped reading — drops the chunk and counts its frames, rather than
/// blocking a thread the callback depends on to keep its rings empty.
fn forward<S: Stamp>(drain: &mut AvDrain<S>, tx: &SyncSender<AvChunk>, state: &EngineState) {
    let latency = Duration::from_micros(u64::from(state.load_input_latency_us()));
    drain.drain(latency, |chunk| {
        let frames = chunk.voice.len() as u64;
        if tx.try_send(chunk).is_err() {
            state.av_dropped.fetch_add(frames, Ordering::Relaxed);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::{av_tap, AvBuffer, AvChunk, AvDrain, AvTap, AV_MARKS, AV_RING_FRAMES};
    use crate::alloc_probe::{arm, disarm};
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{Duration, Instant};

    fn buf(at_ms: u64) -> AvBuffer<Duration> {
        AvBuffer {
            at: Duration::from_millis(at_ms),
            out_frames: 480,
            occupancy: 96,
            chain_latency: 0,
        }
    }

    fn drain_all(drain: &mut AvDrain<Duration>) -> Vec<AvChunk> {
        let mut out = Vec::new();
        drain.drain(Duration::ZERO, |c| out.push(c));
        out
    }

    fn tap_n(
        tap: &mut AvTap<Duration>,
        n: usize,
        span: usize,
        first_ms: u64,
        dropped: &AtomicU64,
        anchor: Instant,
    ) {
        let voice = vec![0.25f32; span];
        let sb = vec![-0.5f32; span];
        for i in 0..n {
            tap.tap(
                true,
                buf(first_ms + 10 * i as u64),
                &voice,
                &sb,
                dropped,
                || anchor,
            );
        }
    }

    /// The callback's half, measured: no allocation and no free, on the
    /// anchoring buffer, on the ones after it, disarmed, and when the rings
    /// are full and the buffer is dropped.
    #[test]
    fn the_tap_allocates_nothing() {
        let (mut tap, _drain) = av_tap::<Duration>(48_000, 48_000);
        let dropped = AtomicU64::new(0);
        let anchor = Instant::now();
        let voice = [0.1f32; 480];
        let sb = [0.2f32; 480];

        // The counter is live: the same window around a real allocation is
        // not zero, so the zeros below are a measurement, not a dead hook.
        arm();
        drop(std::hint::black_box(vec![0u8; 16]));
        assert_eq!(disarm(), (1, 1), "the allocation probe is not counting");

        arm();
        tap.tap(true, buf(0), &voice, &sb, &dropped, || anchor);
        tap.tap(true, buf(10), &voice, &sb, &dropped, || anchor);
        tap.tap(false, buf(20), &voice, &sb, &dropped, || anchor);
        let (allocs, frees) = disarm();
        assert_eq!((allocs, frees), (0, 0), "tapping allocated or freed");

        // Fill the rings, then measure the dropping path too.
        for i in 0..(AV_RING_FRAMES / 480 + 2) {
            tap.tap(true, buf(30 + 10 * i as u64), &voice, &sb, &dropped, || {
                anchor
            });
        }
        assert!(
            dropped.load(Ordering::Relaxed) > 0,
            "the rings never filled"
        );
        arm();
        tap.tap(true, buf(99_000), &voice, &sb, &dropped, || anchor);
        let (allocs, frees) = disarm();
        assert_eq!((allocs, frees), (0, 0), "dropping allocated or freed");
    }

    /// The seam: what the callback pushes is what the recorder receives —
    /// the same frames, in order, indexed and timed — not merely "a push
    /// happened".
    #[test]
    fn tapped_buffers_reach_the_recorder_intact() {
        let (mut tap, mut drain) = av_tap::<Duration>(44_100, 48_000);
        let dropped = AtomicU64::new(0);
        let anchor = Instant::now();
        let voice: Vec<f32> = (0..441u16).map(|i| f32::from(i) / 441.0).collect();
        let sb: Vec<f32> = (0..441u16).map(|i| -f32::from(i) / 441.0).collect();
        for i in 0..3u64 {
            let mut b = buf(1_000 + 10 * i);
            b.chain_latency = 7;
            tap.tap(true, b, &voice, &sb, &dropped, || anchor);
        }
        let chunks = drain_all(&mut drain);
        assert_eq!(chunks.len(), 3);
        for (i, c) in chunks.iter().enumerate() {
            assert_eq!(c.voice, voice, "voice frames altered");
            assert_eq!(c.sb, sb, "soundboard frames altered");
            assert_eq!(c.index, 441 * i as u64);
            assert_eq!(c.rate, 44_100);
            assert_eq!(c.out_rate, 48_000);
            assert_eq!(c.chain_latency, 7);
            assert_eq!(c.at, anchor + Duration::from_millis(10 * i as u64));
            assert_eq!(c.epoch, chunks[0].epoch);
        }
        assert_eq!(dropped.load(Ordering::Relaxed), 0);
    }

    /// A buffer that does not fit is dropped from all three rings, counted,
    /// and leaves a gap in the index exactly its size.
    #[test]
    fn a_dropped_buffer_leaves_an_exact_gap() {
        let (mut tap, mut drain) = av_tap::<Duration>(48_000, 48_000);
        let dropped = AtomicU64::new(0);
        let anchor = Instant::now();
        let fill = AV_RING_FRAMES / 1024;
        tap_n(&mut tap, fill, 1024, 0, &dropped, anchor);
        assert_eq!(
            dropped.load(Ordering::Relaxed),
            0,
            "the rings should hold exactly this"
        );
        // One more cannot fit.
        tap_n(&mut tap, 1, 1024, 10 * fill as u64, &dropped, anchor);
        assert_eq!(dropped.load(Ordering::Relaxed), 1024);
        let first = drain_all(&mut drain);
        assert_eq!(first.len(), fill);
        // Room again: the next buffer lands one span after the dropped one.
        tap_n(&mut tap, 1, 1024, 10 * (fill as u64 + 1), &dropped, anchor);
        let next = drain_all(&mut drain);
        assert_eq!(next.len(), 1);
        assert_eq!(next[0].index, 1024 * (fill as u64 + 1));
        assert_eq!(next[0].epoch, first[0].epoch, "a drop is not a new run");
    }

    /// The marks ring is the limit at small device buffers, and it drops the
    /// same way.
    #[test]
    fn the_mark_ring_drops_whole_buffers_too() {
        let (mut tap, mut drain) = av_tap::<Duration>(48_000, 48_000);
        let dropped = AtomicU64::new(0);
        tap_n(&mut tap, AV_MARKS + 3, 16, 0, &dropped, Instant::now());
        assert_eq!(dropped.load(Ordering::Relaxed), 3 * 16);
        let chunks = drain_all(&mut drain);
        assert_eq!(chunks.len(), AV_MARKS);
        assert!(chunks.iter().all(|c| c.voice.len() == 16));
    }

    /// Disarming ends the run; arming again starts a new epoch at index 0,
    /// anchored afresh.
    #[test]
    fn re_arming_starts_a_new_epoch() {
        let (mut tap, mut drain) = av_tap::<Duration>(48_000, 48_000);
        let dropped = AtomicU64::new(0);
        let a1 = Instant::now();
        tap_n(&mut tap, 2, 480, 0, &dropped, a1);
        tap.tap(false, buf(20), &[0.0; 480], &[0.0; 480], &dropped, || a1);
        let a2 = a1 + Duration::from_secs(5);
        tap_n(&mut tap, 2, 480, 5_000, &dropped, a2);
        let chunks = drain_all(&mut drain);
        assert_eq!(chunks.len(), 4);
        assert_ne!(chunks[2].epoch, chunks[0].epoch);
        assert_eq!(chunks[2].index, 0);
        assert_eq!(chunks[2].at, a2);
        assert_eq!(chunks[3].at, a2 + Duration::from_millis(10));
    }

    /// A first buffer that is dropped does not start the run: the next one
    /// does, at index 0, with its own anchor.
    #[test]
    fn a_dropped_first_buffer_moves_the_anchor() {
        let (mut tap, mut drain) = av_tap::<Duration>(48_000, 48_000);
        let dropped = AtomicU64::new(0);
        let anchor = Instant::now();
        tap_n(&mut tap, AV_RING_FRAMES / 1024, 1024, 0, &dropped, anchor);
        tap.tap(false, buf(0), &[], &[], &dropped, || anchor);
        // Rings full of the old run: the new run's first buffer is dropped.
        let late = anchor + Duration::from_secs(1);
        tap_n(&mut tap, 1, 1024, 2_000, &dropped, late);
        drain.discard();
        let later = anchor + Duration::from_secs(2);
        tap_n(&mut tap, 1, 1024, 3_000, &dropped, later);
        let chunks = drain_all(&mut drain);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].index, 0);
        assert_eq!(chunks[0].at, later);
    }

    /// Buffers left over from an earlier take, without the anchor that began
    /// them, are never placed.
    #[test]
    fn leftovers_without_an_anchor_are_skipped() {
        let (mut tap, mut drain) = av_tap::<Duration>(48_000, 48_000);
        let dropped = AtomicU64::new(0);
        let anchor = Instant::now();
        tap_n(&mut tap, 3, 480, 0, &dropped, anchor);
        // The drain forgets the run, as a Detach does; one more buffer of the
        // old run arrives after it.
        drain.discard();
        tap_n(&mut tap, 1, 480, 30, &dropped, anchor);
        assert!(drain_all(&mut drain).is_empty());
    }
}
