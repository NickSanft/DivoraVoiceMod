//! v1.52.0: recording the screen with your voice.
//!
//! [`ScreenRecorder`] lives in `AppState` and outlives every engine session.
//! A take runs on its own `divora-encoder` thread (`divora_capture` does the
//! capturing and encoding); its audio comes from the engine's tap through
//! [`EngineFeed`], and how it goes is reported as `screen-recording` events.
//!
//! The take's phase moves Idle → Starting → Recording → Stopping → Idle, and
//! nothing else: [`Phase::next`] is the whole table, and the tests pin it.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, sync_channel, Receiver, Sender};
use std::sync::{Arc, Condvar, Mutex, PoisonError};
use std::time::{Duration, Instant};

use divora_capture::sizing::{default_bitrate, output_size, DEFAULT_MAX};
use divora_capture::timeline::AudioChunk;
use divora_capture::{
    seconds_since, AudioFeed, CaptureStatus, MonitorInfo, RecordOptions, RecordStats, TakeObserver,
};
use divora_core::audio::{AudioEngine, AvChunk};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter};

/// The event every phase change and progress report goes out as.
pub const EVENT: &str = "screen-recording";

/// What the WAV recorder says when the engine is off; the screen recorder
/// refuses with the same words.
pub const ENGINE_STOPPED: &str = "Start the engine before recording.";

/// Engine buffers queued for the take: about ten seconds of 10 ms buffers,
/// far more than the recording thread ever falls behind by. A full queue
/// drops buffers (counted, and silent in the file) rather than block the
/// engine's drain.
const AV_QUEUE: usize = 1024;

/// How long Start waits for the first frame before reporting failure. The
/// capture itself gives up after five seconds of no picture.
const START_TIMEOUT: Duration = Duration::from_secs(10);

/// How long Stop waits for the file to be finished.
const STOP_TIMEOUT: Duration = Duration::from_secs(20);

/// Where a take is.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Phase {
    #[default]
    Idle,
    Starting,
    Recording,
    Stopping,
}

/// What can happen to a take.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Input {
    /// The user asked for a take.
    Start,
    /// The first frame is in and the file is open.
    Started,
    /// The user asked to stop.
    Stop,
    /// The recording thread is done, for whatever reason.
    Finished,
}

impl Phase {
    /// The phase after `input`, or `None` when `input` means nothing here
    /// (a second Start, a Stop with nothing running).
    #[must_use]
    pub fn next(self, input: Input) -> Option<Self> {
        match (self, input) {
            (Self::Idle, Input::Start) => Some(Self::Starting),
            (Self::Starting, Input::Started) => Some(Self::Recording),
            (Self::Starting | Self::Recording, Input::Stop) => Some(Self::Stopping),
            (Self::Starting | Self::Recording | Self::Stopping, Input::Finished) => {
                Some(Self::Idle)
            }
            _ => None,
        }
    }
}

/// Payload of the `screen-recording` event.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScreenRecordingUpdate {
    pub state: Phase,
    /// The file being (or just) written.
    pub path: Option<String>,
    /// How long the take has been recording.
    pub elapsed_ms: u64,
    /// Frame ticks skipped because the encoder fell behind.
    pub dropped_frames: u64,
    /// The audio clock against the video's, in ppm (0 until measured).
    pub drift_ppm: f64,
    /// Seconds into the take at which the engine stopped, while it stays
    /// stopped: from there the take's audio is silence.
    pub audio_lost_at: Option<f64>,
    /// Why the take ended, when it ended badly.
    pub error: Option<String>,
}

/// Payload of `screen_capture_status`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptureStatusDto {
    pub supported: bool,
    pub reason: Option<String>,
    pub h264_encoder: Option<String>,
    pub hardware: bool,
}

impl From<CaptureStatus> for CaptureStatusDto {
    fn from(s: CaptureStatus) -> Self {
        Self {
            supported: s.supported,
            reason: s.reason,
            h264_encoder: s.h264_encoder,
            hardware: s.hardware,
        }
    }
}

/// One entry of `list_capture_monitors`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MonitorDto {
    pub id: String,
    /// 1-based, in the list's order (the main screen first).
    pub number: u32,
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub primary: bool,
}

#[must_use]
pub fn monitor_dtos(monitors: Vec<MonitorInfo>) -> Vec<MonitorDto> {
    monitors
        .into_iter()
        .zip(1u32..)
        .map(|(m, number)| MonitorDto {
            id: m.id,
            number,
            x: m.x,
            y: m.y,
            width: m.width,
            height: m.height,
            primary: m.primary,
        })
        .collect()
}

/// Picture quality, as the Settings row names it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Quality {
    /// Smaller files; fine for talking over a mostly still screen.
    Small,
    #[default]
    Balanced,
    /// Fast motion stays sharp; files about 1.6× larger.
    High,
}

impl Quality {
    /// Unknown names fall back to Balanced rather than fail a take.
    #[must_use]
    pub fn parse(s: Option<&str>) -> Self {
        match s {
            Some("small") => Self::Small,
            Some("high") => Self::High,
            _ => Self::Balanced,
        }
    }

    /// Bits per second for a take of `monitor` at `fps`.
    #[must_use]
    pub fn bitrate(self, monitor: (u32, u32), fps: u32) -> u32 {
        let base = f64::from(default_bitrate(output_size(monitor, DEFAULT_MAX), fps));
        let scale = match self {
            Self::Small => 0.6,
            Self::Balanced => 1.0,
            Self::High => 1.6,
        };
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)] // ≤ 1.6 × a u32
        let bits = (base * scale).round().min(f64::from(u32::MAX)) as u32;
        bits
    }
}

/// The frame rates offered: 30 (the default) and 60.
#[must_use]
pub fn sanitize_fps(fps: Option<u32>) -> u32 {
    if fps == Some(60) {
        60
    } else {
        30
    }
}

/// Where a take named `filename` goes: its final path component only, in
/// `dir`, as an `.mp4`. Nothing the frontend sends can escape `dir`.
#[must_use]
pub fn screen_recording_path(dir: &Path, filename: &str) -> PathBuf {
    let name = Path::new(filename)
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .filter(|s| !s.is_empty() && s != "." && s != "..")
        .unwrap_or_else(|| "divora-screen.mp4".to_string());
    let mut dest = dir.join(name);
    dest.set_extension("mp4");
    dest
}

/// The A/V offset the user may set, in milliseconds either way.
pub const MAX_AV_OFFSET_MS: i32 = 500;

/// What `start_screen_recording` was asked for.
#[derive(Debug, Clone)]
pub struct StartRequest {
    pub monitor_id: String,
    pub path: PathBuf,
    pub fps: u32,
    pub quality: Quality,
    pub cursor: bool,
    /// Added to every audio timestamp: positive moves the sound later.
    pub av_offset_ms: i32,
}

/// The engine's tap, as the capture crate wants it: each [`AvChunk`] timed
/// against the take's start, its voice delay worked out.
pub struct EngineFeed {
    rx: Receiver<AvChunk>,
    offset: f64,
}

impl EngineFeed {
    #[must_use]
    pub fn new(rx: Receiver<AvChunk>, av_offset_ms: i32) -> Self {
        let ms = av_offset_ms.clamp(-MAX_AV_OFFSET_MS, MAX_AV_OFFSET_MS);
        Self {
            rx,
            offset: f64::from(ms) / 1000.0,
        }
    }
}

impl AudioFeed for EngineFeed {
    fn poll(&mut self, start: Instant, out: &mut Vec<AudioChunk>) {
        while let Ok(c) = self.rx.try_recv() {
            out.push(to_audio_chunk(c, start, self.offset));
        }
    }
}

/// One engine buffer on the take's clock. The voice had been on its way for
/// the input latency plus however long it sat in the input ring.
#[must_use]
pub fn to_audio_chunk(c: AvChunk, start: Instant, offset: f64) -> AudioChunk {
    let rate = f64::from(c.rate.max(1));
    AudioChunk {
        epoch: c.epoch,
        at: seconds_since(start, c.at) + offset,
        index: c.index,
        rate: c.rate,
        out_frames: c.out_frames,
        out_rate: c.out_rate,
        voice_wait: c.input_latency.as_secs_f64() + f64::from(c.occupancy) / rate,
        chain_latency: c.chain_latency,
        voice: c.voice,
        sb: c.sb,
    }
}

#[derive(Debug, Default)]
struct Shared {
    phase: Phase,
    path: Option<PathBuf>,
    stop: Option<Arc<AtomicBool>>,
    /// Counts takes, so a Stop waiting on one can tell it has finished.
    take: u64,
    /// The last take's file, or why there is none.
    outcome: Option<Result<PathBuf, String>>,
    /// The latest report, for `screen_recording_status`.
    last: ScreenRecordingUpdate,
}

/// The app's screen recorder. One take at a time.
#[derive(Debug, Default)]
pub struct ScreenRecorder {
    shared: Mutex<Shared>,
    changed: Condvar,
}

impl ScreenRecorder {
    fn lock(&self) -> std::sync::MutexGuard<'_, Shared> {
        self.shared.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Whether a take is under way in any phase.
    pub fn is_busy(&self) -> bool {
        self.lock().phase != Phase::Idle
    }

    /// The latest report.
    pub fn status(&self) -> ScreenRecordingUpdate {
        self.lock().last.clone()
    }

    /// Record `req.monitor_id` with the engine's voice and soundboard.
    /// Blocks until the first frame is in (or the take fails), and returns
    /// the file it is writing.
    pub fn start(
        self: &Arc<Self>,
        app: &AppHandle,
        engine: &Arc<AudioEngine>,
        req: StartRequest,
    ) -> Result<PathBuf, String> {
        let (take, stop) = {
            let mut s = self.lock();
            let Some(next) = s.phase.next(Input::Start) else {
                return Err("A screen recording is already running.".to_string());
            };
            s.phase = next;
            s.take += 1;
            s.path = Some(req.path.clone());
            s.outcome = None;
            let stop = Arc::new(AtomicBool::new(false));
            s.stop = Some(Arc::clone(&stop));
            s.last = ScreenRecordingUpdate {
                state: next,
                path: Some(req.path.to_string_lossy().into_owned()),
                ..ScreenRecordingUpdate::default()
            };
            emit(app, &s.last);
            (s.take, stop)
        };

        let refuse = |why: String| -> Result<PathBuf, String> {
            self.finish(app, take, Err(why.clone()), None);
            Err(why)
        };
        if !engine.is_running() {
            return refuse(ENGINE_STOPPED.to_string());
        }
        let capture = divora_capture::capture_status();
        if !capture.supported {
            return refuse(
                capture
                    .reason
                    .unwrap_or_else(|| "Screen recording isn't available.".into()),
            );
        }
        let Some(monitor) = divora_capture::list_monitors()
            .into_iter()
            .find(|m| m.id == req.monitor_id)
        else {
            return refuse("That screen isn't connected any more — pick another.".to_string());
        };
        let (tx, rx) = sync_channel::<AvChunk>(AV_QUEUE);
        if !engine.attach_av_tap(tx) {
            return refuse(ENGINE_STOPPED.to_string());
        }

        let opts = RecordOptions {
            fps: req.fps,
            max_size: DEFAULT_MAX,
            bitrate: Some(
                req.quality
                    .bitrate((monitor.width, monitor.height), req.fps),
            ),
            cursor: req.cursor,
        };
        let (start_tx, start_rx) = channel::<Result<(), String>>();
        let path = req.path.clone();
        let run = TakeRun {
            recorder: Arc::clone(self),
            app: app.clone(),
            engine: Arc::clone(engine),
            req,
            opts,
            rx,
            stop,
            start_tx,
            take,
        };
        let spawned = std::thread::Builder::new()
            .name("divora-encoder".into())
            .spawn(move || run.run());
        if let Err(e) = spawned {
            engine.detach_av_tap();
            return refuse(format!("Couldn't start recording: {e}"));
        }
        match start_rx.recv_timeout(START_TIMEOUT) {
            Ok(Ok(())) => Ok(path),
            Ok(Err(why)) => Err(why),
            Err(_) => Err("The screen recording didn't start.".to_string()),
        }
    }

    /// Stop the take and wait for its file to be finished. Returns the file.
    pub fn stop(&self, app: &AppHandle) -> Result<PathBuf, String> {
        let mut s = self.lock();
        let take = s.take;
        match s.phase.next(Input::Stop) {
            Some(next) => {
                s.phase = next;
                if let Some(stop) = &s.stop {
                    stop.store(true, Ordering::Release);
                }
                s.last.state = next;
                emit(app, &s.last);
            }
            None if s.phase == Phase::Stopping => {}
            None => return Err("No screen recording is running.".to_string()),
        }
        let (s, timeout) = self
            .changed
            .wait_timeout_while(s, STOP_TIMEOUT, |s| {
                s.take == take && s.phase != Phase::Idle
            })
            .unwrap_or_else(PoisonError::into_inner);
        if timeout.timed_out() {
            return Err("The recording is taking a long time to finish.".to_string());
        }
        s.outcome
            .clone()
            .unwrap_or_else(|| Err("The recording ended without a file.".to_string()))
    }

    /// The first frame is in: Starting → Recording (unless a Stop got there
    /// first).
    fn started(&self, app: &AppHandle) {
        let mut s = self.lock();
        if let Some(next) = s.phase.next(Input::Started) {
            s.phase = next;
            s.last.state = next;
            emit(app, &s.last);
        }
    }

    /// A progress report from the recording thread.
    fn progress(&self, app: &AppHandle, update: ScreenRecordingUpdate) {
        let mut s = self.lock();
        if s.phase == Phase::Recording {
            s.last = ScreenRecordingUpdate {
                state: s.phase,
                ..update
            };
            emit(app, &s.last);
        }
    }

    /// The take `take` is over.
    fn finish(
        &self,
        app: &AppHandle,
        take: u64,
        outcome: Result<PathBuf, String>,
        stats: Option<&RecordStats>,
    ) {
        let mut s = self.lock();
        if s.take != take {
            return;
        }
        s.phase = s.phase.next(Input::Finished).unwrap_or(Phase::Idle);
        s.stop = None;
        let error = outcome.as_ref().err().cloned();
        s.last = ScreenRecordingUpdate {
            state: s.phase,
            path: s.path.as_ref().map(|p| p.to_string_lossy().into_owned()),
            elapsed_ms: s.last.elapsed_ms,
            dropped_frames: stats.map_or(s.last.dropped_frames, |st| st.frames_dropped),
            drift_ppm: stats
                .and_then(|st| st.audio.as_ref())
                .map_or(s.last.drift_ppm, |a| a.timeline.drift_ppm),
            audio_lost_at: s.last.audio_lost_at,
            error,
        };
        s.outcome = Some(outcome);
        emit(app, &s.last);
        drop(s);
        self.changed.notify_all();
    }
}

/// Everything one take's `divora-encoder` thread needs.
struct TakeRun {
    recorder: Arc<ScreenRecorder>,
    app: AppHandle,
    engine: Arc<AudioEngine>,
    req: StartRequest,
    opts: RecordOptions,
    rx: Receiver<AvChunk>,
    stop: Arc<AtomicBool>,
    start_tx: Sender<Result<(), String>>,
    take: u64,
}

impl TakeRun {
    /// The take, start to finish: record, let go of the engine's tap, report.
    fn run(self) {
        let Self {
            recorder,
            app,
            engine,
            req,
            opts,
            rx,
            stop,
            start_tx,
            take,
        } = self;
        let mut feed = EngineFeed::new(rx, req.av_offset_ms);
        let mut observer = AppObserver {
            app: app.clone(),
            recorder: Arc::clone(&recorder),
            engine: Arc::clone(&engine),
            path: req.path.clone(),
            started: None,
            start_tx: Some(start_tx),
            lost_at: None,
        };
        let result = divora_capture::record_monitor(
            &req.monitor_id,
            &req.path,
            &opts,
            &stop,
            None,
            Some(&mut feed),
            &mut observer,
        );
        engine.detach_av_tap();
        let take_stats = result.as_ref().ok().cloned();
        let outcome = match result {
            Ok(_) if req.path.exists() => Ok(req.path.clone()),
            Ok(_) => Err("Stopped before the screen sent a picture.".to_string()),
            Err(e) => Err(e.to_string()),
        };
        // A take that never started still owes Start its answer.
        if let Some(tx) = observer.start_tx.take() {
            let _ = tx.send(outcome.as_ref().map(|_| ()).map_err(Clone::clone));
        }
        if let Some(st) = &take_stats {
            tracing::info!(
                frames = st.frames_written,
                dropped = st.frames_dropped,
                encoder = ?st.encoder,
                hardware = st.hardware,
                audio = ?st.audio,
                "screen recording finished"
            );
        }
        recorder.finish(&app, take, outcome, take_stats.as_ref());
    }
}

fn emit(app: &AppHandle, update: &ScreenRecordingUpdate) {
    let _ = app.emit(EVENT, update);
}

/// Hears from the recording thread and tells the app.
struct AppObserver {
    app: AppHandle,
    recorder: Arc<ScreenRecorder>,
    engine: Arc<AudioEngine>,
    path: PathBuf,
    started: Option<Instant>,
    start_tx: Option<Sender<Result<(), String>>>,
    lost_at: Option<f64>,
}

impl TakeObserver for AppObserver {
    fn started(&mut self) {
        self.started = Some(Instant::now());
        self.recorder.started(&self.app);
        if let Some(tx) = self.start_tx.take() {
            let _ = tx.send(Ok(()));
        }
    }

    fn progress(&mut self, stats: &RecordStats) {
        let elapsed = self.started.map_or(Duration::ZERO, |t| t.elapsed());
        // The engine stopped: the take goes on, and its audio is silence
        // from here until the engine is started again (which re-attaches).
        if self.engine.is_running() {
            self.lost_at = None;
        } else if self.lost_at.is_none() {
            self.lost_at = Some(elapsed.as_secs_f64());
        }
        self.recorder.progress(
            &self.app,
            ScreenRecordingUpdate {
                state: Phase::Recording,
                path: Some(self.path.to_string_lossy().into_owned()),
                elapsed_ms: u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX),
                dropped_frames: stats.frames_dropped,
                drift_ppm: stats.audio.as_ref().map_or(0.0, |a| a.timeline.drift_ppm),
                audio_lost_at: self.lost_at,
                error: None,
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::{
        monitor_dtos, sanitize_fps, screen_recording_path, to_audio_chunk, CaptureStatusDto, Input,
        MonitorDto, Phase, Quality, ScreenRecordingUpdate,
    };
    use divora_capture::MonitorInfo;
    use divora_core::audio::AvChunk;
    use std::path::Path;
    use std::time::{Duration, Instant};

    /// The whole transition table: every pair, so a new arm cannot slip in
    /// unnoticed.
    #[test]
    fn the_phase_table() {
        use Input::{Finished, Start, Started, Stop};
        use Phase::{Idle, Recording, Starting, Stopping};
        let expect = |p: Phase, i: Input| -> Option<Phase> {
            match (p, i) {
                (Idle, Start) => Some(Starting),
                (Starting, Started) => Some(Recording),
                (Starting | Recording, Stop) => Some(Stopping),
                (Starting | Recording | Stopping, Finished) => Some(Idle),
                _ => None,
            }
        };
        for p in [Idle, Starting, Recording, Stopping] {
            for i in [Start, Started, Stop, Finished] {
                assert_eq!(p.next(i), expect(p, i), "{p:?} + {i:?}");
            }
        }
        // The cases that matter in words: one take at a time, and a stop
        // with nothing running does nothing.
        assert_eq!(Recording.next(Start), None);
        assert_eq!(Idle.next(Stop), None);
        // A Started that arrives after the user already hit Stop does not
        // put the take back to Recording.
        assert_eq!(Stopping.next(Started), None);
    }

    #[test]
    fn filenames_cannot_leave_the_recordings_folder() {
        let dir = Path::new(r"C:\rec");
        assert_eq!(
            screen_recording_path(dir, r"..\..\evil.exe"),
            dir.join("evil.mp4")
        );
        assert_eq!(
            screen_recording_path(dir, "a/b/clip.wav"),
            dir.join("clip.mp4")
        );
        assert_eq!(
            screen_recording_path(dir, ""),
            dir.join("divora-screen.mp4")
        );
        assert_eq!(
            screen_recording_path(dir, ".."),
            dir.join("divora-screen.mp4")
        );
        assert_eq!(
            screen_recording_path(dir, "divora-screen-2026-10-14_12-00-00.mp4"),
            dir.join("divora-screen-2026-10-14_12-00-00.mp4")
        );
    }

    #[test]
    fn only_30_and_60_fps() {
        assert_eq!(sanitize_fps(None), 30);
        assert_eq!(sanitize_fps(Some(60)), 60);
        assert_eq!(sanitize_fps(Some(144)), 30);
        assert_eq!(sanitize_fps(Some(0)), 30);
    }

    #[test]
    fn quality_scales_the_bitrate() {
        let m = (1920, 1080);
        let balanced = Quality::Balanced.bitrate(m, 30);
        assert_eq!(balanced, 8_087_040);
        assert!(Quality::Small.bitrate(m, 30) < balanced);
        assert!(Quality::High.bitrate(m, 30) > balanced);
        assert_eq!(Quality::parse(Some("nonsense")), Quality::Balanced);
        assert_eq!(Quality::parse(Some("high")), Quality::High);
    }

    /// The voice's wait is the input latency plus the ring's depth at the
    /// engine rate, and the offset moves the whole buffer.
    #[test]
    fn engine_buffers_are_timed_against_the_take() {
        let start = Instant::now();
        let chunk = AvChunk {
            epoch: 3,
            at: start + Duration::from_millis(250),
            index: 4_800,
            rate: 48_000,
            out_frames: 480,
            out_rate: 48_000,
            occupancy: 960,
            chain_latency: 12_288,
            input_latency: Duration::from_millis(7),
            voice: vec![0.5; 480],
            sb: vec![0.25; 480],
        };
        let c = to_audio_chunk(chunk, start, 0.04);
        assert!((c.at - 0.29).abs() < 1e-9);
        assert!((c.voice_wait - 0.027).abs() < 1e-9);
        assert_eq!((c.epoch, c.index, c.chain_latency), (3, 4_800, 12_288));
        assert_eq!(c.voice.len(), 480);
    }

    /// The wire shapes, frozen: these keys are what the frontend reads.
    #[test]
    fn wire_keys_are_frozen() {
        let update = serde_json::to_value(ScreenRecordingUpdate::default()).unwrap();
        let mut keys: Vec<_> = update.as_object().unwrap().keys().cloned().collect();
        keys.sort();
        assert_eq!(
            keys,
            [
                "audioLostAt",
                "driftPpm",
                "droppedFrames",
                "elapsedMs",
                "error",
                "path",
                "state"
            ]
        );
        for (p, s) in [
            (Phase::Idle, "idle"),
            (Phase::Starting, "starting"),
            (Phase::Recording, "recording"),
            (Phase::Stopping, "stopping"),
        ] {
            assert_eq!(serde_json::to_value(p).unwrap(), s);
        }
        let status = serde_json::to_value(CaptureStatusDto {
            supported: true,
            reason: None,
            h264_encoder: Some("x".into()),
            hardware: true,
        })
        .unwrap();
        let mut keys: Vec<_> = status.as_object().unwrap().keys().cloned().collect();
        keys.sort();
        assert_eq!(keys, ["h264Encoder", "hardware", "reason", "supported"]);
        let monitors: Vec<MonitorDto> = monitor_dtos(vec![MonitorInfo {
            id: r"\\.\DISPLAY1".into(),
            x: 0,
            y: 0,
            width: 1920,
            height: 1080,
            primary: true,
        }]);
        let m = serde_json::to_value(&monitors[0]).unwrap();
        let mut keys: Vec<_> = m.as_object().unwrap().keys().cloned().collect();
        keys.sort();
        assert_eq!(
            keys,
            ["height", "id", "number", "primary", "width", "x", "y"]
        );
        assert_eq!(monitors[0].number, 1);
    }
}
