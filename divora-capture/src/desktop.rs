//! Desktop audio's tap (v1.53.0): from the loopback capture thread to the
//! recorder.
//!
//! The `divora-loopback` thread reads Windows' packets — everything the PC
//! plays except Divora, or one playback device — and hands each one to
//! [`DesktopTap::ingest`], which copies it into preallocated rings. Nothing
//! there allocates, frees or locks: Windows keeps only about three 10 ms
//! packets for a process-loopback reader, and one that falls further behind
//! loses audio without being told. The recorder drains the rings into
//! [`DesktopChunk`]s on its own thread ([`DesktopDrain::drain`]).
//!
//! Every packet is timed by Windows: the performance-counter reading of its
//! first frame. [`packet_time`] puts that on the clock the video uses
//! (`Instant` reads the same counter), and falls back to the packet's arrival
//! when Windows flags the reading or it is plainly wrong.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::channel;
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use ringbuf::traits::{Consumer, Observer, Producer, Split};
use ringbuf::HeapRb;

use crate::error::CaptureError;
use crate::take::seconds_since;
use crate::timeline::{DesktopChunk, OUT_RATE};

/// Where a take's desktop audio comes from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DesktopSource {
    /// Everything the PC plays except Divora and the apps it started, on
    /// every device (process loopback). Never swapped for a device behind
    /// the user's back: a device can be playing Divora's own sound.
    Everything,
    /// One playback device, by its Windows endpoint id: everything it plays,
    /// Divora included. Where process loopback can't be had.
    Device(String),
}

/// What a desktop capture actually opened.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OpenedSource {
    Everything,
    Device { id: String, name: String },
}

/// A playback device desktop audio can be taken from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlaybackDevice {
    /// The Windows endpoint id: stable for the device.
    pub id: String,
    /// What Windows' Sound settings call it.
    pub name: String,
    /// The Windows default playback device.
    pub is_default: bool,
}

/// How long [`DesktopCapture::start`] waits for the device to open: longer
/// than the capture thread's own activation timeout, so its answer — even a
/// timeout — arrives first.
const OPEN_TIMEOUT: Duration = Duration::from_secs(3);

/// How long dropping a [`DesktopCapture`] waits for its thread. It looks at
/// its stop flag every 20 ms; this is only for a Windows call that hangs.
const STOP_TIMEOUT: Duration = Duration::from_secs(2);

/// Desktop audio being captured, on a `divora-loopback` thread of its own,
/// until this is dropped.
pub struct DesktopCapture {
    opened: OpenedSource,
    stop: Arc<AtomicBool>,
    lost: Arc<AtomicBool>,
    gave_up: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
    /// Signalled when the thread is about to end.
    done: std::sync::mpsc::Receiver<()>,
    drain: Option<DesktopDrain>,
}

impl DesktopCapture {
    /// Open `source` and start capturing. Waits for the device to open, so
    /// a failure is an `Err` here rather than silence later.
    pub fn start(source: &DesktopSource) -> Result<Self, CaptureError> {
        let (tap, drain) = desktop_tap();
        let stop = Arc::new(AtomicBool::new(false));
        let lost = Arc::new(AtomicBool::new(false));
        let gave_up = Arc::new(AtomicBool::new(false));
        let (tx, rx) = channel();
        let (done_tx, done) = channel();
        let thread = {
            let (source, stop) = (source.clone(), Arc::clone(&stop));
            let flags = (Arc::clone(&lost), Arc::clone(&gave_up));
            std::thread::Builder::new()
                .name("divora-loopback".into())
                .spawn(move || {
                    capture_thread(&source, tap, &stop, (&flags.0, &flags.1), &tx);
                    let _ = done_tx.send(());
                })?
        };
        match rx.recv_timeout(OPEN_TIMEOUT) {
            Ok(Ok(opened)) => Ok(Self {
                opened,
                stop,
                lost,
                gave_up,
                thread: Some(thread),
                done,
                drain: Some(drain),
            }),
            Ok(Err(e)) => {
                let _ = thread.join();
                Err(e)
            }
            Err(_) => {
                // Still opening: tell it to give up, and don't wait for it.
                stop.store(true, Ordering::Release);
                Err(CaptureError::Unavailable(
                    "Windows didn't start the desktop audio in time.".to_string(),
                ))
            }
        }
    }

    /// What is being recorded.
    #[must_use]
    pub fn opened(&self) -> &OpenedSource {
        &self.opened
    }

    /// The recorder's end of the rings; `None` once taken.
    pub fn take_drain(&mut self) -> Option<DesktopDrain> {
        self.drain.take()
    }

    /// The device stopped working: desktop audio is silence from then on —
    /// until it is opened again, unless [`Self::gave_up_flag`] says that
    /// won't happen this take.
    #[must_use]
    pub fn is_lost(&self) -> bool {
        self.lost.load(Ordering::Acquire)
    }

    /// The flag behind [`Self::is_lost`], for whoever reports on the take
    /// while the drain is elsewhere.
    #[must_use]
    pub fn lost_flag(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.lost)
    }

    /// Set once Windows stopped answering when the lost device was opened
    /// again: nothing more is tried this take.
    #[must_use]
    pub fn gave_up_flag(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.gave_up)
    }
}

impl Drop for DesktopCapture {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        let Some(t) = self.thread.take() else {
            return;
        };
        // Never wait without end: a stop must not hang on Windows.
        if self.done.recv_timeout(STOP_TIMEOUT).is_ok() {
            let _ = t.join();
        } else {
            tracing::warn!("desktop audio thread did not stop in time; left to finish");
        }
    }
}

#[cfg(windows)]
use crate::native::loopback::capture_thread;

/// Elsewhere there is nothing to capture with.
#[cfg(not(windows))]
fn capture_thread(
    _source: &DesktopSource,
    _tap: DesktopTap,
    _stop: &AtomicBool,
    _flags: (&AtomicBool, &AtomicBool),
    ready: &std::sync::mpsc::Sender<Result<OpenedSource, CaptureError>>,
) {
    let _ = ready.send(Err(CaptureError::Unavailable(
        "Desktop audio is only available on Windows.".to_string(),
    )));
}

/// Desktop audio is captured as 48 kHz stereo: Windows converts whatever the
/// device runs at.
pub const DESKTOP_RATE: u32 = OUT_RATE;
pub const DESKTOP_CHANNELS: usize = 2;

/// Samples the frame ring holds: about 10.9 s of 48 kHz stereo — what the
/// engine's own queue to the recorder holds — so a recording thread held up
/// by the encoder, or by a take's slow start, loses nothing. 4 MiB.
const RING_SAMPLES: usize = 1 << 20;

/// Packets the mark ring holds: Windows' packets are 10 ms, so ten seconds.
const MARKS: usize = 1024;

/// A packet whose timestamp is further than this from when it was read is
/// not believed.
const BELIEVABLE: Duration = Duration::from_secs(1);

/// One packet from Windows, as the capture thread hands it in.
#[derive(Debug, Clone, Copy)]
#[allow(clippy::struct_excessive_bools)] // Windows' per-packet flags, one each
pub struct DesktopPacket<'a> {
    /// Interleaved left/right, `frames` frames. Not read when `silent`.
    pub samples: &'a [f32],
    pub frames: u32,
    /// When the first frame was captured, on the `Instant` clock (see
    /// [`packet_time`]).
    pub at: Instant,
    /// When the capture thread read it.
    pub arrived: Instant,
    /// False when `at` is only the arrival estimate.
    pub timed: bool,
    /// Windows says to treat the packet as silence, whatever it holds.
    pub silent: bool,
    /// Windows says this packet does not follow on from the last one.
    pub discontinuity: bool,
    /// The first packet after the playback device went quiet (sent nothing
    /// for a wait): whatever gap lies before it was silence, not lost audio.
    pub resumed: bool,
}

/// One packet's bookkeeping, pushed after its samples.
#[derive(Debug, Clone, Copy)]
struct Mark {
    stream: u64,
    frames: u32,
    at: Instant,
    arrived: Instant,
    timed: bool,
    discontinuity: bool,
    resumed: bool,
}

type SampleProducer = <HeapRb<f32> as Split>::Prod;
type SampleConsumer = <HeapRb<f32> as Split>::Cons;
type MarkProducer = <HeapRb<Mark> as Split>::Prod;
type MarkConsumer = <HeapRb<Mark> as Split>::Cons;

/// The capture thread's end.
pub struct DesktopTap {
    samples: SampleProducer,
    marks: MarkProducer,
    /// Which opening of the device this packet came from. Each new one is a
    /// fresh run on the recorder's side.
    stream: u64,
    dropped: Arc<AtomicU64>,
}

impl DesktopTap {
    /// A new opening of the capture device begins: what follows does not
    /// continue what came before.
    pub fn begin_stream(&mut self) {
        self.stream += 1;
    }

    /// Take one packet. All or nothing: its samples and its mark go into
    /// the rings together, or it is counted in the dropped frames and the
    /// recorder sees the gap in its timestamps.
    pub fn ingest(&mut self, p: &DesktopPacket<'_>) {
        let n = p.frames as usize * DESKTOP_CHANNELS;
        let fits = self.marks.vacant_len() > 0 && self.samples.vacant_len() >= n;
        if !fits {
            self.dropped
                .fetch_add(u64::from(p.frames), Ordering::Relaxed);
            return;
        }
        if p.silent || p.samples.len() < n {
            // Silent, or (never seen) short: zeros, never Windows' bytes.
            let _ = self.samples.push_iter(std::iter::repeat_n(0.0, n));
        } else {
            let _ = self.samples.push_slice(&p.samples[..n]);
        }
        let _ = self.marks.try_push(Mark {
            stream: self.stream,
            frames: p.frames,
            at: p.at,
            arrived: p.arrived,
            timed: p.timed,
            discontinuity: p.discontinuity,
            resumed: p.resumed,
        });
    }
}

/// Packets per window of the stamp-lag measurement: ten seconds of 10 ms
/// packets.
const LAG_WINDOW: u32 = 1_000;

/// How far behind Windows' timestamps the packets were read, on average, in
/// milliseconds: over the stream's first ten seconds, and over the latest
/// ten. Windows times process-loopback packets by counting frames, so a
/// device whose clock did not follow the counter would show only here, as
/// the two drifting apart.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct StampLag {
    pub first_ms: Option<f64>,
    pub latest_ms: Option<f64>,
}

/// The recorder's end. `Send`: it moves to the recording thread.
pub struct DesktopDrain {
    samples: SampleConsumer,
    marks: MarkConsumer,
    dropped: Arc<AtomicU64>,
    /// The stream the lag windows belong to, and the window being filled.
    lag_stream: u64,
    lag_sum: f64,
    lag_count: u32,
    lag: StampLag,
    untimed: u64,
    packets: u64,
}

impl DesktopDrain {
    /// Every packet taken so far, as chunks timed from `start` (the take's
    /// first frame) plus `offset` seconds.
    pub fn drain(&mut self, start: Instant, offset: f64, out: &mut Vec<DesktopChunk>) {
        while let Some(m) = self.marks.try_pop() {
            let n = m.frames as usize * DESKTOP_CHANNELS;
            let mut frames = vec![0.0; n];
            // Pushed before the mark, so all of it is there.
            let _ = self.samples.pop_slice(&mut frames);
            self.measure(&m);
            out.push(DesktopChunk {
                stream: m.stream,
                at: seconds_since(start, m.at) + offset,
                timed: m.timed,
                discontinuity: m.discontinuity,
                resumed: m.resumed,
                rate: DESKTOP_RATE,
                frames,
            });
        }
    }

    /// Frames that did not fit and were lost.
    #[must_use]
    pub fn dropped_frames(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }

    /// How late packets were read against their timestamps (see
    /// [`StampLag`]).
    #[must_use]
    pub fn stamp_lag(&self) -> StampLag {
        self.lag
    }

    /// Packets drained, and how many of them Windows could not time.
    #[must_use]
    pub fn packets(&self) -> (u64, u64) {
        (self.packets, self.untimed)
    }

    fn measure(&mut self, m: &Mark) {
        self.packets += 1;
        if !m.timed {
            self.untimed += 1;
            return;
        }
        if m.stream != self.lag_stream {
            self.lag_stream = m.stream;
            self.lag = StampLag::default();
            self.lag_sum = 0.0;
            self.lag_count = 0;
        }
        self.lag_sum += seconds_since(m.at, m.arrived) * 1000.0;
        self.lag_count += 1;
        if self.lag_count == LAG_WINDOW {
            let mean = self.lag_sum / f64::from(LAG_WINDOW);
            self.lag.first_ms.get_or_insert(mean);
            self.lag.latest_ms = Some(mean);
            self.lag_sum = 0.0;
            self.lag_count = 0;
        }
    }
}

/// A tap and its drain, sharing preallocated rings.
#[must_use]
pub fn desktop_tap() -> (DesktopTap, DesktopDrain) {
    let (sp, sc) = HeapRb::<f32>::new(RING_SAMPLES).split();
    let (mp, mc) = HeapRb::<Mark>::new(MARKS).split();
    let dropped = Arc::new(AtomicU64::new(0));
    (
        DesktopTap {
            samples: sp,
            marks: mp,
            stream: 0,
            dropped: Arc::clone(&dropped),
        },
        DesktopDrain {
            samples: sc,
            marks: mc,
            dropped,
            lag_stream: 0,
            lag_sum: 0.0,
            lag_count: 0,
            lag: StampLag::default(),
            untimed: 0,
            packets: 0,
        },
    )
}

/// A performance-counter reading in the 100-ns units Windows stamps audio
/// packets in, as Windows computes it: `ticks × 10^7 / frequency`, in 128
/// bits so that no uptime overflows it.
#[must_use]
pub fn qpc_to_100ns(ticks: i64, frequency: i64) -> u64 {
    let v = i128::from(ticks) * 10_000_000 / i128::from(frequency.max(1));
    u64::try_from(v.max(0)).unwrap_or(u64::MAX)
}

/// One reading of the performance counter, in 100-ns units, taken together
/// with an `Instant`: the pair that puts packet timestamps on the `Instant`
/// clock.
#[derive(Debug, Clone, Copy)]
pub struct QpcAnchor {
    pub at: Instant,
    pub qpc_100ns: u64,
}

/// When a packet's first frame was captured, and whether that is Windows'
/// own timestamp (`true`) or an estimate from when it was read (`false`).
///
/// Windows' reading is used unless it is flagged as uncertain, missing, or
/// more than a second from `arrived` — then the packet is taken to have just
/// finished: `arrived` less its own length.
#[must_use]
pub fn packet_time(
    anchor: QpcAnchor,
    qpc_100ns: u64,
    timestamp_error: bool,
    arrived: Instant,
    frames: u32,
) -> (Instant, bool) {
    let estimate = || {
        let length =
            Duration::from_nanos(u64::from(frames) * 1_000_000_000 / u64::from(DESKTOP_RATE));
        arrived.checked_sub(length).unwrap_or(arrived)
    };
    if timestamp_error || qpc_100ns == 0 {
        return (estimate(), false);
    }
    let at = if qpc_100ns >= anchor.qpc_100ns {
        anchor.at.checked_add(Duration::from_nanos(
            (qpc_100ns - anchor.qpc_100ns).saturating_mul(100),
        ))
    } else {
        anchor.at.checked_sub(Duration::from_nanos(
            (anchor.qpc_100ns - qpc_100ns).saturating_mul(100),
        ))
    };
    match at {
        Some(at) if seconds_since(arrived, at).abs() <= BELIEVABLE.as_secs_f64() => (at, true),
        _ => (estimate(), false),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        desktop_tap, packet_time, qpc_to_100ns, DesktopPacket, QpcAnchor, MARKS, RING_SAMPLES,
    };
    use std::time::{Duration, Instant};

    fn packet(samples: &[f32], at: Instant) -> DesktopPacket<'_> {
        DesktopPacket {
            samples,
            frames: u32::try_from(samples.len() / 2).unwrap(),
            at,
            arrived: at,
            timed: true,
            silent: false,
            discontinuity: false,
            resumed: false,
        }
    }

    /// What the capture thread pushes is what the recorder gets: the same
    /// samples in order, timed against the take's start, stream by stream.
    #[test]
    fn packets_reach_the_recorder_intact() {
        let (mut tap, mut drain) = desktop_tap();
        let start = Instant::now();
        let samples: Vec<f32> = (0..960u16).map(|i| f32::from(i) / 960.0).collect();
        tap.begin_stream();
        tap.ingest(&packet(&samples, start + Duration::from_millis(5)));
        tap.ingest(&DesktopPacket {
            discontinuity: true,
            timed: false,
            ..packet(&samples, start + Duration::from_millis(15))
        });
        tap.begin_stream();
        let early = start.checked_sub(Duration::from_millis(20)).unwrap();
        tap.ingest(&packet(&samples, early));
        let mut out = Vec::new();
        drain.drain(start, 0.25, &mut out);
        assert_eq!(out.len(), 3);
        for c in &out {
            assert_eq!(c.frames, samples, "samples altered");
            assert_eq!(c.rate, 48_000);
        }
        assert!((out[0].at - 0.255).abs() < 1e-9);
        assert!((out[1].at - 0.265).abs() < 1e-9);
        assert!(
            (out[2].at - 0.23).abs() < 1e-9,
            "before the take is negative"
        );
        assert!(out[0].timed && !out[1].timed);
        assert!(!out[0].discontinuity && out[1].discontinuity);
        assert_eq!(out[0].stream, out[1].stream);
        assert_ne!(out[1].stream, out[2].stream);
        assert_eq!(drain.dropped_frames(), 0);
    }

    /// The stamp lag is the mean of (read − stamped) over ten-second
    /// windows: the first one kept, the latest one updated; untimed packets
    /// don't count, and a new stream starts over.
    #[test]
    fn the_stamp_lag_is_measured_per_window() {
        let (mut tap, mut drain) = desktop_tap();
        let t0 = Instant::now();
        let samples = [0.0f32; 960];
        let mut out = Vec::new();
        tap.begin_stream();
        for i in 0..2_500u64 {
            let at = t0 + Duration::from_millis(10 * i);
            // 12 ms late for the first ten seconds, 14 ms after.
            let late = if i < 1_000 { 12 } else { 14 };
            tap.ingest(&DesktopPacket {
                arrived: at + Duration::from_millis(late),
                timed: i < 1_000 || i % 100 != 7,
                ..packet(&samples, at)
            });
            drain.drain(t0, 0.0, &mut out);
        }
        let lag = drain.stamp_lag();
        assert!((lag.first_ms.unwrap() - 12.0).abs() < 1e-6, "{lag:?}");
        assert!((lag.latest_ms.unwrap() - 14.0).abs() < 1e-6, "{lag:?}");
        assert_eq!(drain.packets(), (2_500, 15));
        tap.begin_stream();
        tap.ingest(&packet(&samples, t0));
        drain.drain(t0, 0.0, &mut out);
        assert_eq!(drain.stamp_lag(), super::StampLag::default());
    }

    /// A packet Windows calls silent is zeros, whatever its buffer holds.
    #[test]
    fn a_silent_packet_is_zeros() {
        let (mut tap, mut drain) = desktop_tap();
        let junk = [0.7f32; 960];
        tap.ingest(&DesktopPacket {
            silent: true,
            ..packet(&junk, Instant::now())
        });
        let mut out = Vec::new();
        drain.drain(Instant::now(), 0.0, &mut out);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].frames.len(), 960);
        assert!(out[0].frames.iter().all(|&s| s == 0.0));
    }

    /// Full rings drop whole packets — counted — and never split one, so the
    /// recorder can never pair a mark with another packet's samples.
    #[test]
    fn a_full_ring_drops_whole_packets() {
        let (mut tap, mut drain) = desktop_tap();
        let at = Instant::now();
        let samples = [0.5f32; 960];
        let fit = (RING_SAMPLES / 960).min(MARKS);
        for _ in 0..fit + 3 {
            tap.ingest(&packet(&samples, at));
        }
        assert_eq!(drain.dropped_frames(), 3 * 480);
        let mut out = Vec::new();
        drain.drain(at, 0.0, &mut out);
        assert_eq!(out.len(), fit);
        assert!(out
            .iter()
            .all(|c| c.frames.len() == 960 && c.frames[959].to_bits() == 0.5f32.to_bits()));
        // Room again: the next packet goes in whole.
        tap.ingest(&packet(&samples, at));
        out.clear();
        drain.drain(at, 0.0, &mut out);
        assert_eq!(out.len(), 1);
    }

    /// Counter readings convert as Windows converts them, at the usual
    /// frequencies, and weeks of uptime do not overflow.
    #[test]
    fn counter_readings_convert_without_overflow() {
        assert_eq!(qpc_to_100ns(10_000_000, 10_000_000), 10_000_000);
        assert_eq!(qpc_to_100ns(3_579_545, 3_579_545), 10_000_000);
        assert_eq!(qpc_to_100ns(14_318_180 * 3, 14_318_180), 30_000_000);
        // Sixty days at 10 MHz: far past where 64 bits would overflow once
        // multiplied by 10^7.
        let sixty_days: i64 = 60 * 86_400 * 10_000_000;
        assert_eq!(
            qpc_to_100ns(sixty_days, 10_000_000),
            60 * 86_400 * 10_000_000
        );
        assert_eq!(qpc_to_100ns(-5, 10_000_000), 0);
    }

    /// Windows' timestamp is believed when it is there and sane — either
    /// side of the anchor — and replaced by the arrival estimate otherwise.
    #[test]
    fn packet_times_come_from_windows_or_from_arrival() {
        let t0 = Instant::now() + Duration::from_secs(10);
        let anchor = QpcAnchor {
            at: t0,
            qpc_100ns: 5_000_000_000,
        };
        let arrived = t0 + Duration::from_millis(30);
        // 20 ms after the anchor.
        let (at, timed) = packet_time(anchor, 5_000_200_000, false, arrived, 480);
        assert!(timed);
        assert_eq!(at, t0 + Duration::from_millis(20));
        // Before the anchor works too.
        let (at, timed) = packet_time(anchor, 4_999_900_000, false, t0, 480);
        assert!(timed);
        assert_eq!(Some(at), t0.checked_sub(Duration::from_millis(10)));
        // Flagged, missing, or absurd: the packet just finished arriving.
        let estimate = arrived.checked_sub(Duration::from_millis(10)).unwrap();
        for (qpc, flagged) in [(5_000_200_000, true), (0, false), (5_100_000_000, false)] {
            let (at, timed) = packet_time(anchor, qpc, flagged, arrived, 480);
            assert!(!timed, "qpc {qpc} flagged {flagged}");
            assert_eq!(at, estimate);
        }
    }
}
