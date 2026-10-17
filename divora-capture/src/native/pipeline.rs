//! The capture pipeline on the directly driven encoder (v1.54.0).
//!
//! One capture, one encoder and one audio timeline on a thread of their own
//! (`divora-encoder`), shared by whatever needs them: a take, the replay
//! ring, or both ([`spawn_pipeline`]). The decisions are in
//! [`crate::pipeline_core::PipelineCore`]; this is the Windows shell around
//! it: it captures, converts, feeds the encoder, takes in audio, and does
//! what the core asks — hands a take's messages to its writer, starts a
//! save, drains or rebuilds or lets go of the session.
//!
//! [`record_monitor_direct`] runs the same loop on the calling thread for
//! one take, with v1.52.0's `record_monitor` signature: the day-4 gate and
//! `capture_spike` use it.
//!
//! The thread writes no recording or clip (its log lines are its only file
//! writes). Beyond one loop's wait (≤ 20 ms) it waits only for the first
//! picture at the start (up to 5 s); to drain the encoder (up to 500 ms)
//! when its output is wanted, or quiesce it (up to 100 ms) when it isn't;
//! while it builds a new encoder after a stream change or a new session
//! after a sleep; and, at the very end, for the desktop-audio thread to
//! stop (up to 2 s). It logs transitions and failures only — every log
//! line is a file write.

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use windows::Win32::Graphics::Direct3D11::ID3D11Texture2D;

use super::d3d::{latest_frame_texture, Gpu, Nv12Converter};
use super::find_monitor;
use super::mf::{self, MfSession};
use super::mft::{EncoderConfig, EncoderEngine, EncoderInput, EncoderSession, Submit};
use super::pool::{InputPool, MfDevice, PoolFrame};
use super::power::SleepWatch;
use super::record::ComScope;
use super::save;
use super::take_mux::{SendFail, TakeFile, TakeMux};
use super::wgc::{MonitorCapture, Wake};
use crate::error::CaptureError;
use crate::pipeline::{
    EngineSignal, HeldReason, PipelineCommand, PipelineEvents, PipelineHandle, PipelineSettings,
    RingStatus, StartError, TakeEvents, TakeRequest,
};
use crate::pipeline_core::{CoreConfig, EncoderEvent, MuxMsg, Out, PipelineCore, SessionAction};
use crate::sizing::output_size;
use crate::take::{seconds_since, AudioFeed, RecordOptions, RecordStats, TakeObserver};
use crate::timeline::{AudioChunk, DesktopChunk};

/// How long to wait for the very first frame (a monitor asleep sends none).
const FIRST_FRAME_TIMEOUT: Duration = Duration::from_secs(5);
/// The longest the loop goes without taking in audio.
const AUDIO_POLL: Duration = Duration::from_millis(20);
/// How long the encoder is given to hand back what it holds at the end.
const DRAIN: Duration = Duration::from_millis(500);
/// How long a teardown nobody waits on gives the encoder.
const QUIESCE: Duration = Duration::from_millis(100);
/// How often the graphics device is asked whether it is still there.
const DEVICE_CHECK: Duration = Duration::from_secs(1);
/// After a sleep: how often a failed build is tried again, and for how long.
const REBUILD_RETRY: Duration = Duration::from_secs(2);
const REBUILD_FOR: Duration = Duration::from_secs(30);
/// After a sleep: a capture that has sent nothing for this long is made
/// again (the lock screen is on a desktop it can't see; whether a session
/// survives that is unknown).
const RECAPTURE: Duration = Duration::from_secs(10);
/// Set to anything to end a direct take the way a teardown nobody waits on
/// does (`quiesce`, its output discarded) instead of draining it: the gate's
/// check that tearing down with frames in flight is safe.
pub const QUIESCE_ENV: &str = "DIVORA_DIRECT_QUIESCE";
/// Set to anything to have the direct encoder refused as if this PC
/// couldn't use it: how the fallback to the sink writer, with the same
/// audio feed, is checked on a PC that would never take it.
pub const PRETEND_NO_DIRECT_ENV: &str = "DIVORA_PRETEND_NO_DIRECT";

/// The device, capture and encoder one pipeline runs on. Field order is
/// drop order: the encoder first (it holds the pool's textures and the
/// device manager), the device last.
struct Session {
    encoder: EncoderSession,
    pool: InputPool,
    converter: Nv12Converter,
    latest: ID3D11Texture2D,
    capture: Option<MonitorCapture>,
    mf_device: MfDevice,
    gpu: Gpu,
    out_size: (u32, u32),
    fps: u32,
    bitrate: u32,
}

impl Session {
    /// Everything for the monitor `settings` names, or
    /// `CaptureError::Encoder` when this PC can't use the direct path.
    fn build(
        mf: &MfSession,
        settings: &PipelineSettings,
        wake: &Arc<Wake>,
    ) -> Result<Self, CaptureError> {
        if std::env::var_os(PRETEND_NO_DIRECT_ENV).is_some() {
            return Err(CaptureError::Encoder(format!(
                "{PRETEND_NO_DIRECT_ENV} is set"
            )));
        }
        let fps = settings.fps;
        if fps == 0 || 48_000 % fps != 0 {
            return Err(CaptureError::Encoder(format!(
                "{fps} fps is not a whole number of audio samples per frame"
            )));
        }
        let monitor = find_monitor(&settings.monitor_id).ok_or(CaptureError::NoSuchMonitor)?;
        let gpu = Gpu::new()?;
        if !gpu.video {
            return Err(CaptureError::Encoder(
                "the graphics driver has no video support".to_string(),
            ));
        }
        // What only the direct path needs: any failure here means this PC
        // can't use it (an encoder whose sessions are all taken, a driver
        // that refuses a setting), and the take falls back to the sink
        // writer, which picks its own encoder and can convert on the CPU.
        let direct = |e: CaptureError| CaptureError::Encoder(e.to_string());
        let mf_device = MfDevice::new(mf.mf(), &gpu).map_err(direct)?;
        let capture = MonitorCapture::start(&gpu, monitor, settings.cursor, Arc::clone(wake))?;
        let in_size = capture.size();
        let out_size = output_size(in_size, settings.max_size);
        let encoder =
            new_encoder(mf, &mf_device, &gpu, out_size, fps, settings.bitrate).map_err(direct)?;
        let pool = InputPool::new(mf.mf(), &mf_device, out_size, fps).map_err(direct)?;
        let latest = latest_frame_texture(&gpu, in_size)?;
        let converter =
            Nv12Converter::new(&gpu, &latest, in_size, out_size, fps).map_err(direct)?;
        Ok(Self {
            encoder,
            pool,
            converter,
            latest,
            capture: Some(capture),
            mf_device,
            gpu,
            out_size,
            fps,
            bitrate: settings.bitrate,
        })
    }

    /// Take every frame waiting into `latest`; how many arrived. A monitor
    /// that changed size rebuilds the capture's buffers and the converter;
    /// the encoder's size, and its pool, stay put.
    fn drain(&mut self) -> Result<u64, CaptureError> {
        let Some(capture) = self.capture.as_mut() else {
            return Ok(0);
        };
        let d = capture.drain_into(&self.gpu, &self.latest)?;
        let Some(size) = d.resized else {
            return Ok(d.frames);
        };
        tracing::info!(width = size.0, height = size.1, "the screen changed size");
        capture.recreate(&self.gpu, size)?;
        self.latest = latest_frame_texture(&self.gpu, size)?;
        self.converter =
            Nv12Converter::new(&self.gpu, &self.latest, size, self.out_size, self.fps)?;
        Ok(0)
    }

    /// A new encoder and pool on the same device and capture.
    fn rebuild_encoder(&mut self, mf: &MfSession) -> Result<(), CaptureError> {
        self.encoder = new_encoder(
            mf,
            &self.mf_device,
            &self.gpu,
            self.out_size,
            self.fps,
            self.bitrate,
        )?;
        self.pool = InputPool::new(mf.mf(), &self.mf_device, self.out_size, self.fps)?;
        Ok(())
    }
}

fn new_encoder(
    mf: &MfSession,
    dev: &MfDevice,
    gpu: &Gpu,
    size: (u32, u32),
    fps: u32,
    bitrate: u32,
) -> Result<EncoderSession, CaptureError> {
    EncoderSession::new(
        mf,
        Some(dev),
        &EncoderConfig {
            size,
            fps,
            bitrate,
            gop_frames: 2 * fps,
            low_latency: true,
            engine: EncoderEngine::Hardware {
                vendor_id: gpu.vendor_id().ok(),
            },
        },
    )
}

/// Wait for the session's first frame, so a recording starts on real
/// content, not a black frame. `Ok(false)`: `stop` was set first.
fn first_frame(
    s: &mut Session,
    wake: &Wake,
    stop: Option<&AtomicBool>,
) -> Result<bool, CaptureError> {
    let since = Instant::now();
    loop {
        if stop.is_some_and(|s| s.load(Ordering::Acquire)) {
            return Ok(false);
        }
        if wake.wait(Duration::from_millis(50)) && s.drain()? > 0 {
            return Ok(true);
        }
        if since.elapsed() > FIRST_FRAME_TIMEOUT {
            return Err(CaptureError::Unavailable(
                "That screen isn't sending any picture (is it asleep?).".to_string(),
            ));
        }
    }
}

/// A frame converted and waiting for the encoder to ask for one.
struct Pending {
    frame: PoolFrame,
    index: u64,
    pts: u64,
    dur: u64,
    force: bool,
    /// When the frame first found no request; a newer tick that replaces it
    /// keeps this, so a stalled encoder is noticed.
    since: Duration,
}

/// After a sleep: building the session again, then waiting for a picture.
struct Rebuild {
    /// When to try building next.
    next_try: Instant,
    /// When to stop trying.
    give_up: Instant,
    /// When the capture of the session being waited on started.
    capture_since: Option<Instant>,
}

/// The open take's writer.
struct Writer {
    mux: TakeMux,
    /// Behind: everything from here on waits to be handed over at the end.
    behind: Option<Vec<MuxMsg>>,
}

/// The loop, owning everything Windows. `'m` is Media Foundation's
/// session, `'f` the audio feed's borrow.
struct Runner<'m, 'f> {
    mf: &'m MfSession,
    settings: PipelineSettings,
    s: Option<Session>,
    core: PipelineCore,
    feed: Option<&'f mut dyn AudioFeed>,
    wake: Arc<Wake>,
    /// The pipeline's clock: frame 0 is stamped 0 here.
    start: Instant,
    pending: Option<Pending>,
    encoded: Vec<EncoderEvent>,
    out: Out,
    writer: Option<Writer>,
    sleep: SleepWatch,
    /// This iteration's one reading of the clock, taken before the sleep
    /// check: what the timeline is pumped to. A sleep in the middle of the
    /// iteration is found by the next check before any later reading of
    /// the clock — which counts the sleep — reaches the timeline.
    clock: Duration,
    rebuild: Option<Rebuild>,
    chunks: Vec<AudioChunk>,
    desktop: Vec<DesktopChunk>,
    last_audio: Instant,
    last_device_check: Instant,
    changed_since_write: bool,
    /// The encoder was drained or quiesced: nothing more to give it.
    drained: bool,
    exiting: bool,
}

impl<'m, 'f> Runner<'m, 'f> {
    fn new(
        mf: &'m MfSession,
        s: Session,
        settings: PipelineSettings,
        feed: Option<&'f mut dyn AudioFeed>,
        events: Arc<dyn PipelineEvents>,
        wake: Arc<Wake>,
    ) -> Self {
        let info = s.encoder.info();
        tracing::info!(info = ?info, size = ?s.out_size, "the direct encoder is ready");
        let core = PipelineCore::new(
            CoreConfig {
                fps: s.fps,
                size: s.out_size,
                track: settings.track,
                with_audio: feed.is_some(),
                encoder: info.name.clone(),
                hardware: info.hardware,
                seq: s.encoder.sequence_header().cloned(),
            },
            events,
        );
        let now = Instant::now();
        Self {
            mf,
            settings,
            s: Some(s),
            core,
            feed,
            wake,
            start: now,
            pending: None,
            encoded: Vec::new(),
            out: Out::default(),
            writer: None,
            sleep: SleepWatch::new(),
            clock: Duration::ZERO,
            rebuild: None,
            chunks: Vec::new(),
            desktop: Vec::new(),
            last_audio: now,
            last_device_check: now,
            changed_since_write: true,
            drained: false,
            exiting: false,
        }
    }

    /// The clock, read live. Only the start of an iteration (and the wait's
    /// length) reads it: everything else uses that iteration's reading,
    /// `clock`, which the sleep check has seen.
    fn now(&self) -> Duration {
        self.start.elapsed()
    }

    /// One command from the app.
    fn command(&mut self, c: PipelineCommand) {
        let now = self.clock;
        match c {
            PipelineCommand::StartTake(req) => self.start_take(req),
            PipelineCommand::StopTake { take } => {
                let pending = self.pending.is_some();
                self.core.stop_take(take, pending, now, &mut self.out);
            }
            PipelineCommand::Arm { seconds, cap_bytes } => {
                self.core.arm(seconds, cap_bytes, &mut self.out);
            }
            PipelineCommand::Resize { seconds, cap_bytes } => {
                self.core.resize(seconds, cap_bytes);
            }
            PipelineCommand::Disarm => self.core.disarm(now, &mut self.out),
            PipelineCommand::Save(req) => {
                let at = seconds_since(self.start, req.at);
                self.core.save(req, at, now);
            }
            PipelineCommand::Shutdown => {
                let pending = self.pending.is_some();
                self.core.shutdown(pending, now, &mut self.out);
            }
        }
        self.flush();
    }

    fn start_take(&mut self, req: TakeRequest) {
        if let Some(why) = self.core.take_refusal() {
            req.events
                .finished(Err(why.to_string()), RecordStats::default());
            return;
        }
        let size = self.s.as_ref().map_or((0, 0), |s| s.out_size);
        let file = TakeFile {
            path: req.path.clone(),
            size,
            fps: self.settings.fps,
            with_audio: self.feed.is_some(),
            stems: req
                .stems
                .then_some(self.settings.track.desktop_gain_db.is_some()),
        };
        match TakeMux::spawn(file, Arc::clone(&req.events)) {
            Ok(mux) => {
                self.writer = Some(Writer { mux, behind: None });
                self.core.start_take(req.take, req.stems, req.events);
            }
            Err(e) => req
                .events
                .finished(Err(e.to_string()), RecordStats::default()),
        }
    }

    /// One turn of the loop; false once the pipeline is done.
    fn step(&mut self) -> bool {
        if self.exiting {
            return false;
        }
        // A sleep is handled before the clock, which counts it, is used.
        let clock = self.now();
        self.clock = clock;
        if let Some(t_jump) = self.sleep.jumped(clock) {
            self.on_sleep(t_jump);
            // Ended by it (a take with no ring): nothing more of this turn —
            // the clock that counted the sleep must not reach the timeline.
            if self.exiting {
                return false;
            }
        }
        if self.rebuild.is_some() {
            self.rebuilding();
        } else if let Err(e) = self.video() {
            let now = self.clock;
            self.core
                .encoder_failed(&e.to_string(), false, false, now, &mut self.out);
        }
        self.audio();
        let now = self.clock;
        self.core.poll(now, &mut self.out);
        self.flush();
        if self.exiting {
            return false;
        }
        self.wait();
        true
    }

    /// The picture: the screen gone, the encoder's outputs, its health, the
    /// frame due, and into the encoder.
    fn video(&mut self) -> Result<(), CaptureError> {
        let now = self.clock;
        let Some(s) = self.s.as_mut() else {
            return Ok(());
        };
        if s.capture.as_ref().is_some_and(MonitorCapture::is_lost) {
            s.capture = None;
            self.pending = None;
            if !self.drained {
                self.drained = true;
                s.encoder.drain(DRAIN, &mut self.encoded)?;
                self.core.on_encoder(&mut self.encoded, now, &mut self.out);
            }
            self.core.on_screen_lost(now, &mut self.out);
            self.flush();
            return Ok(());
        }
        if self.core.encoding() && !self.drained {
            s.encoder.poll(&mut self.encoded)?;
            self.core.on_encoder(&mut self.encoded, now, &mut self.out);
            self.flush();
        }
        let Some(s) = self.s.as_mut() else {
            return Ok(());
        };
        if !self.core.capturing() {
            return Ok(());
        }
        if self.last_device_check.elapsed() >= DEVICE_CHECK {
            self.last_device_check = Instant::now();
            if !EncoderSession::device_ok(&s.gpu) {
                self.core.encoder_failed(
                    "the graphics device was lost",
                    false,
                    false,
                    now,
                    &mut self.out,
                );
                self.flush();
                return Ok(());
            }
        }
        self.core
            .check_liveness(now, self.pending.as_ref().map(|p| p.since), &mut self.out);
        if !self.core.capturing() {
            self.flush();
            return Ok(());
        }
        // The frame due: converted into a free texture to wait for the
        // encoder; one still waiting is replaced with the newer picture.
        if let Some(due) = self.core.tick(now) {
            if let Some(p) = self.pending.as_mut() {
                s.converter.convert_into(&p.frame.texture)?;
                self.core.counters().no_credit += 1;
                (p.index, p.pts, p.dur) = (due.index, due.pts, due.dur);
                p.force |= due.force;
            } else if let Some(frame) = s.pool.try_acquire()? {
                s.converter.convert_into(&frame.texture)?;
                self.pending = Some(Pending {
                    frame,
                    index: due.index,
                    pts: due.pts,
                    dur: due.dur,
                    force: due.force,
                    since: now,
                });
            } else {
                self.core.counters().pool_empty += 1;
            }
            if !self.changed_since_write {
                self.core.counters().repeated += 1;
            }
            self.changed_since_write = false;
        }
        // Into the encoder when it asks.
        if s.encoder.wants_input() {
            if let Some(p) = self.pending.take() {
                let pts = i64::try_from(p.pts).unwrap_or(i64::MAX);
                let dur = i64::try_from(p.dur).unwrap_or(1);
                match s
                    .encoder
                    .encode(&EncoderInput::Gpu(&p.frame.sample), pts, dur, p.force)?
                {
                    Submit::Accepted => self.core.submitted(p.index, pts, p.force, now),
                    Submit::NotNow => self.pending = Some(p),
                }
            }
        }
        Ok(())
    }

    /// Audio, at most every 20 ms. While a rebuild waits for its first
    /// picture, what arrives is thrown away: it belongs to no clock.
    fn audio(&mut self) {
        if self.last_audio.elapsed() < AUDIO_POLL {
            return;
        }
        self.last_audio = Instant::now();
        let mut engine = EngineSignal::Running;
        if let Some(feed) = self.feed.as_deref_mut() {
            feed.poll(self.start, &mut self.chunks);
            feed.poll_desktop(self.start, &mut self.desktop);
            engine = feed.engine();
        }
        // The iteration's reading, checked for a sleep: never a later one.
        let now = self.clock;
        self.core
            .on_audio(&self.chunks, &self.desktop, engine, now, &mut self.out);
        self.chunks.clear();
        self.desktop.clear();
    }

    /// Throw away what the feed holds.
    fn discard_feed(&mut self) {
        discard(
            self.feed.as_deref_mut(),
            self.start,
            &mut self.chunks,
            &mut self.desktop,
        );
    }

    /// The PC slept: the feed's contents are quarantined, the encoder's last
    /// outputs placed, and everything resolved as of `t_jump`.
    fn on_sleep(&mut self, t_jump: Duration) {
        self.discard_feed();
        if let Some(s) = self.s.as_mut() {
            if self.core.encoding() && !self.drained && s.encoder.poll(&mut self.encoded).is_ok() {
                self.core
                    .on_encoder(&mut self.encoded, t_jump, &mut self.out);
            }
        }
        self.pending = None;
        self.core.on_sleep(t_jump, &mut self.out);
        self.flush();
    }

    /// After a sleep: build at once (again every 2 s, for 30 s), then wait
    /// for a picture for as long as it takes — the lock screen shows none.
    fn rebuilding(&mut self) {
        // Nothing arriving now has a clock to go on.
        self.discard_feed();
        let Some(r) = self.rebuild.as_mut() else {
            return;
        };
        if self.s.is_none() {
            if Instant::now() < r.next_try {
                return;
            }
            match Session::build(self.mf, &self.settings, &self.wake) {
                Ok(s) => {
                    self.s = Some(s);
                    self.drained = false;
                    r.capture_since = Some(Instant::now());
                }
                Err(e) if Instant::now() >= r.give_up => {
                    // Never recorded as "this PC can't": an encoder busy
                    // after a sleep says nothing about the next launch.
                    self.rebuild = None;
                    let now = self.clock;
                    self.core.rebuild_failed(&e.to_string(), now, &mut self.out);
                    self.flush();
                    return;
                }
                Err(e) => {
                    tracing::info!(error = %e, "the session couldn't be built yet");
                    r.next_try = Instant::now() + REBUILD_RETRY;
                    return;
                }
            }
        }
        let Some(s) = self.s.as_mut() else {
            return;
        };
        match s.drain() {
            Ok(n) if n > 0 => {
                // The first picture: the clock starts again here.
                self.start = Instant::now();
                self.rebuild = None;
                discard(
                    self.feed.as_deref_mut(),
                    self.start,
                    &mut self.chunks,
                    &mut self.desktop,
                );
                self.core.counters().arrived += n;
                self.changed_since_write = true;
                let (seq, size) = (s.encoder.sequence_header().cloned(), s.out_size);
                self.core.restarted(seq, size);
                self.clock = Duration::ZERO;
                self.sleep.restart(Duration::ZERO);
            }
            Ok(_) => {
                let stale = r.capture_since.is_some_and(|t| t.elapsed() >= RECAPTURE);
                if stale {
                    s.capture = None;
                    match find_monitor(&self.settings.monitor_id).map(|m| {
                        MonitorCapture::start(
                            &s.gpu,
                            m,
                            self.settings.cursor,
                            Arc::clone(&self.wake),
                        )
                    }) {
                        Some(Ok(c)) => s.capture = Some(c),
                        Some(Err(e)) => {
                            tracing::info!(error = %e, "the capture couldn't start yet");
                        }
                        None => tracing::info!("the screen isn't there yet"),
                    }
                    r.capture_since = Some(Instant::now());
                }
            }
            Err(e) => {
                tracing::info!(error = %e, "the capture isn't working yet");
                s.capture = None;
                r.capture_since = Instant::now().checked_sub(RECAPTURE);
            }
        }
    }

    /// Do what the core asked, until it asks nothing more.
    fn flush(&mut self) {
        loop {
            let out = std::mem::take(&mut self.out);
            let busy =
                !out.mux.is_empty() || !out.saves.is_empty() || !out.session.is_empty() || out.exit;
            if !busy {
                return;
            }
            for m in out.mux {
                self.send_mux(m);
            }
            for (snap, req) in out.saves {
                save::spawn(snap, req);
            }
            for a in out.session {
                self.session_action(a);
            }
            if out.exit {
                self.exiting = true;
            }
        }
    }

    /// Hand one message to the take's writer. One that can't keep up ends
    /// the take; everything after waits for the end and goes on a helper
    /// thread, so this one never waits.
    fn send_mux(&mut self, m: MuxMsg) {
        let end = matches!(m, MuxMsg::End { .. });
        let Some(w) = self.writer.as_mut() else {
            return;
        };
        if let Some(behind) = w.behind.as_mut() {
            behind.push(m);
        } else {
            match w.mux.send(m) {
                Ok(()) => {}
                Err(SendFail::Full(m)) => {
                    tracing::warn!("a take's writer can't keep up; ending the take");
                    w.behind = Some(vec![m]);
                    if !end {
                        let now = self.clock;
                        self.core.take_disk_full(now, &mut self.out);
                    }
                }
                // Its file failed, and it has said so: the take is over.
                Err(SendFail::Gone) => {
                    if !end {
                        let now = self.clock;
                        self.core.take_disk_full(now, &mut self.out);
                        self.out.mux.clear();
                    }
                    self.writer = None;
                    return;
                }
            }
        }
        if end {
            if let Some(w) = self.writer.take() {
                if let Some(rest) = w.behind {
                    w.mux.finish_behind(rest);
                }
            }
        }
    }

    fn session_action(&mut self, a: SessionAction) {
        let now = self.clock;
        match a {
            SessionAction::Drain => {
                self.pending = None;
                let Some(s) = self.s.as_mut() else {
                    return;
                };
                // The border goes now.
                s.capture = None;
                if self.drained {
                    return;
                }
                self.drained = true;
                if std::env::var_os(QUIESCE_ENV).is_some() {
                    s.encoder.quiesce(QUIESCE);
                    return;
                }
                if let Err(e) = s.encoder.drain(DRAIN, &mut self.encoded) {
                    tracing::warn!(error = %e, "the encoder couldn't be drained");
                }
                self.core.on_encoder(&mut self.encoded, now, &mut self.out);
            }
            SessionAction::Release { quiesce } => {
                self.pending = None;
                if let Some(mut s) = self.s.take() {
                    if quiesce && !self.drained && EncoderSession::device_ok(&s.gpu) {
                        s.encoder.quiesce(QUIESCE);
                    }
                    tracing::info!(counters = ?s.encoder.counters(), "the session was let go");
                }
                self.drained = false;
            }
            SessionAction::RebuildEncoder => {
                self.pending = None;
                let Some(s) = self.s.as_mut() else {
                    return;
                };
                match s.rebuild_encoder(self.mf) {
                    Ok(()) => {
                        self.drained = false;
                        let seq = s.encoder.sequence_header().cloned();
                        self.core.encoder_rebuilt(seq);
                    }
                    Err(e) => {
                        self.core.rebuild_failed(&e.to_string(), now, &mut self.out);
                    }
                }
            }
            SessionAction::Rebuild => {
                self.pending = None;
                if let Some(mut s) = self.s.take() {
                    s.capture = None;
                    if !self.drained && EncoderSession::device_ok(&s.gpu) {
                        s.encoder.quiesce(QUIESCE);
                    }
                }
                self.drained = false;
                let at = Instant::now();
                self.rebuild = Some(Rebuild {
                    next_try: at,
                    give_up: at + REBUILD_FOR,
                    capture_since: None,
                });
            }
        }
    }

    /// Until the next tick, audio poll, frame or command.
    fn wait(&mut self) {
        let mut until = AUDIO_POLL;
        if self.core.capturing() {
            until = until.min(self.core.next_deadline().saturating_sub(self.now()));
        }
        if self.pending.is_some() {
            until = until.min(Duration::from_millis(2));
        }
        if self.wake.wait(until) && self.rebuild.is_none() {
            if let Some(s) = self.s.as_mut() {
                match s.drain() {
                    Ok(n) if n > 0 => {
                        self.core.counters().arrived += n;
                        self.changed_since_write = true;
                    }
                    Ok(_) => {}
                    Err(e) => {
                        let now = self.clock;
                        self.core
                            .encoder_failed(&e.to_string(), false, false, now, &mut self.out);
                        self.flush();
                    }
                }
            }
        }
    }

    /// The end: what is still in the encoder is no one's.
    fn teardown(mut self) {
        self.pending = None;
        if let Some(mut s) = self.s.take() {
            s.capture = None;
            if !self.drained && EncoderSession::device_ok(&s.gpu) {
                s.encoder.quiesce(QUIESCE);
            }
            tracing::info!(
                counters = ?s.encoder.counters(),
                keyframe_faults = self.core.keyframe_faults(),
                "the pipeline ended"
            );
        }
    }
}

/// Throw away what `feed` holds.
fn discard<'a>(
    feed: Option<&mut (dyn AudioFeed + 'a)>,
    start: Instant,
    chunks: &mut Vec<AudioChunk>,
    desktop: &mut Vec<DesktopChunk>,
) {
    if let Some(feed) = feed {
        feed.poll(start, chunks);
        feed.poll_desktop(start, desktop);
    }
    chunks.clear();
    desktop.clear();
}

/// Start a pipeline on `settings.monitor_id` for `first` (a take or an
/// arm). Returns once the encoder is built and the first frame is in; a PC
/// whose encoder can't be driven directly gets the feed back, for the sink
/// writer.
pub fn spawn_pipeline(
    settings: PipelineSettings,
    feed: Box<dyn AudioFeed + Send>,
    events: Arc<dyn PipelineEvents>,
    first: PipelineCommand,
) -> Result<PipelineHandle, StartError> {
    let (tx, rx) = mpsc::channel::<PipelineCommand>();
    let (ready_tx, ready_rx) = mpsc::channel::<Result<(), StartError>>();
    let wake = Arc::new(Wake::default());
    let for_thread = Arc::clone(&wake);
    let thread = std::thread::Builder::new()
        .name("divora-encoder".into())
        .spawn(move || pipeline_thread(settings, feed, &events, first, &rx, for_thread, &ready_tx))
        .map_err(|e| StartError::Other(CaptureError::Io(e)))?;
    match ready_rx.recv() {
        Ok(Ok(())) => Ok(PipelineHandle::new(
            tx,
            Arc::new(move || wake.notify()),
            thread,
        )),
        Ok(Err(e)) => {
            let _ = thread.join();
            Err(e)
        }
        Err(_) => {
            let _ = thread.join();
            Err(StartError::Other(CaptureError::Unavailable(
                "The screen recording didn't start.".to_string(),
            )))
        }
    }
}

fn pipeline_thread(
    settings: PipelineSettings,
    mut feed: Box<dyn AudioFeed + Send>,
    events: &Arc<dyn PipelineEvents>,
    first: PipelineCommand,
    rx: &Receiver<PipelineCommand>,
    wake: Arc<Wake>,
    ready: &Sender<Result<(), StartError>>,
) {
    // First, so they drop last: COM and Media Foundation outlive the MFT.
    let _com = ComScope::enter();
    let mf = match mf::api().and_then(mf::Mf::session) {
        Ok(mf) => mf,
        Err(e) => {
            let _ = ready.send(Err(StartError::Other(e)));
            return;
        }
    };
    let mut s = match Session::build(&mf, &settings, &wake) {
        Ok(s) => s,
        Err(CaptureError::Encoder(why)) => {
            let _ = ready.send(Err(StartError::Direct { why, feed }));
            return;
        }
        Err(e) => {
            let _ = ready.send(Err(StartError::Other(e)));
            return;
        }
    };
    if let Err(e) = first_frame(&mut s, &wake, None) {
        let _ = ready.send(Err(StartError::Other(e)));
        return;
    }
    let mut r = Runner::new(&mf, s, settings, Some(&mut *feed), Arc::clone(events), wake);
    r.command(first);
    let _ = ready.send(Ok(()));
    loop {
        while let Ok(c) = rx.try_recv() {
            r.command(c);
        }
        if !r.step() {
            break;
        }
    }
    r.teardown();
    drop(feed);
    events.ended(None);
    // `ended` freed the app's slot, and with it the only sender: every
    // command that will ever come is here now. Those that came too late
    // answer for themselves — a take is told, a Save's reply answers when
    // dropped. An Arm needs nothing: `ended` already turned Replay off, and
    // refusing it now could turn off a newer pipeline's arm.
    while let Ok(c) = rx.try_recv() {
        if let PipelineCommand::StartTake(req) = c {
            req.events.finished(
                Err(crate::pipeline::STILL_STOPPING.to_string()),
                RecordStats::default(),
            );
        }
    }
}

/// What `record_monitor_direct`'s take tells the calling thread.
enum Note {
    Started,
    Progress(Box<RecordStats>),
    Finished(Result<std::path::PathBuf, String>, Box<RecordStats>),
}

struct Notes(Mutex<Sender<Note>>);

impl Notes {
    fn send(&self, n: Note) {
        let _ = self
            .0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .send(n);
    }
}

impl TakeEvents for Notes {
    fn started(&self) {
        self.send(Note::Started);
    }
    fn progress(&self, stats: &RecordStats) {
        self.send(Note::Progress(Box::new(stats.clone())));
    }
    fn finished(&self, outcome: Result<std::path::PathBuf, String>, stats: RecordStats) {
        self.send(Note::Finished(outcome, Box::new(stats)));
    }
}

/// A take with no ring has nothing to tell the app about one.
struct NoRing;

impl PipelineEvents for NoRing {
    fn ring_ready(&self) {}
    fn ring(&self, _status: RingStatus) {}
    fn held(&self, _reason: HeldReason, _detail: Option<String>) {}
    fn resumed(&self) {}
    fn rebuilding(&self) {}
    fn arm_refused(&self, _why: String) {}
    fn direct_failed(&self, _why: String) {}
    fn failed(&self, _why: String) {}
    fn ended(&self, _error: Option<String>) {}
}

/// Record the monitor `monitor_id` to `path` on the directly driven encoder,
/// until `stop` is set or `max_duration` has been recorded — v1.52.0's
/// [`super::record::record_monitor`], on the v1.54.0 pipeline, run on this
/// thread. An `Err(CaptureError::Encoder(_))` before anything was written
/// means this PC can't use the direct path. A take that ended by itself (a
/// sleep, the screen gone) is `Ok` with [`RecordStats::cut`] saying why.
pub fn record_monitor_direct(
    monitor_id: &str,
    path: &Path,
    opts: &RecordOptions,
    stop: &AtomicBool,
    max_duration: Option<Duration>,
    audio: Option<&mut dyn AudioFeed>,
    observer: &mut dyn TakeObserver,
) -> Result<RecordStats, CaptureError> {
    let _com = ComScope::enter();
    let api = mf::api()?;
    let mf = api.session()?;
    let mut settings = PipelineSettings {
        monitor_id: monitor_id.to_string(),
        fps: opts.fps,
        max_size: opts.max_size,
        bitrate: opts.bitrate.unwrap_or(0),
        cursor: opts.cursor,
        track: opts.audio,
    };
    if settings.bitrate == 0 {
        settings.bitrate = default_bitrate(&settings);
    }
    let wake = Arc::new(Wake::default());
    let mut s = Session::build(&mf, &settings, &wake)?;
    if !first_frame(&mut s, &wake, Some(stop))? {
        return Ok(RecordStats {
            width: s.out_size.0,
            height: s.out_size.1,
            ..RecordStats::default()
        });
    }
    let (tx, rx) = mpsc::channel();
    let notes: Arc<dyn TakeEvents> = Arc::new(Notes(Mutex::new(tx)));
    let mut r = Runner::new(&mf, s, settings, audio, Arc::new(NoRing), wake);
    r.command(PipelineCommand::StartTake(TakeRequest {
        take: 1,
        path: path.to_path_buf(),
        stems: opts.audio.stems,
        events: notes,
    }));
    let mut stopping = false;
    let mut finished = None;
    loop {
        let out_of_time = max_duration.is_some_and(|m| r.now() >= m);
        if !stopping && (stop.load(Ordering::Acquire) || out_of_time) {
            stopping = true;
            r.command(PipelineCommand::StopTake { take: 1 });
        }
        let more = r.step();
        tell(&rx, observer, &mut finished);
        if !more {
            break;
        }
    }
    r.teardown();
    // The take's writer finishes the file on its own thread.
    let (outcome, stats) = match finished {
        Some(f) => f,
        None => loop {
            match rx.recv_timeout(Duration::from_secs(60)) {
                Ok(Note::Finished(o, st)) => break (o, *st),
                Ok(Note::Started) => observer.started(),
                Ok(Note::Progress(st)) => observer.progress(&st),
                Err(_) => {
                    return Err(CaptureError::Unavailable(
                        "The recording's writer stopped unexpectedly.".to_string(),
                    ))
                }
            }
        },
    };
    match outcome {
        Ok(_) => Ok(stats),
        // Ended by itself, its file kept.
        Err(why) if path.exists() => Ok(RecordStats {
            cut: Some(why),
            ..stats
        }),
        Err(why) => Err(CaptureError::Unavailable(why)),
    }
}

/// Pass what the take said on to the observer, keeping its last word.
fn tell(
    rx: &Receiver<Note>,
    observer: &mut dyn TakeObserver,
    finished: &mut Option<(Result<std::path::PathBuf, String>, RecordStats)>,
) {
    while let Ok(n) = rx.try_recv() {
        match n {
            Note::Started => observer.started(),
            Note::Progress(st) => observer.progress(&st),
            Note::Finished(o, st) => *finished = Some((o, *st)),
        }
    }
}

/// The sizing default bitrate for the monitor `settings` names: its output
/// size is the capture's, which the monitor's own size gives.
fn default_bitrate(settings: &PipelineSettings) -> u32 {
    let size = super::list_monitors()
        .into_iter()
        .find(|m| m.id == settings.monitor_id)
        .map_or(settings.max_size, |m| {
            output_size((m.width, m.height), settings.max_size)
        });
    crate::sizing::default_bitrate(size, settings.fps)
}
