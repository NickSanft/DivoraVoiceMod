//! v1.52.0: recording the screen with your voice. v1.53.0: and with what
//! the PC plays.
//!
//! [`ScreenRecorder`] lives in `AppState` and outlives every engine session.
//! A take runs on its own `divora-encoder` thread (`divora_capture` does the
//! capturing and encoding); its audio comes from the engine's tap — and,
//! when asked, from Windows' desktop audio — through [`EngineFeed`], and how
//! it goes is reported as `screen-recording` events.
//!
//! The take's phase moves Idle → Starting → Recording → Stopping → Idle, and
//! nothing else: [`Phase::next`] is the whole table, and the tests pin it.
//!
//! v1.54.0: a take runs on the capture pipeline it shares with Replay — one
//! capture, one encoder, one timeline on one `divora-encoder` thread
//! (`divora_capture::spawn_pipeline`) — or, where the encoder can't be driven
//! directly, on v1.53.0's own thread through the sink writer. The pipeline
//! is held in a [`Slot`], and Replay's side of it is in [`replay_ops`].

mod replay_ops;

pub use replay_ops::ArmRequest;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, sync_channel, Receiver, Sender};
use std::sync::{Arc, Condvar, Mutex, PoisonError};
use std::time::{Duration, Instant};

use divora_capture::desktop::{
    DesktopCapture, DesktopDrain, DesktopSource, OpenedSource, PlaybackDevice,
};
use divora_capture::pipeline::STILL_STOPPING;
use divora_capture::sizing::{default_bitrate, output_size, DEFAULT_MAX};
use divora_capture::timeline::{AudioChunk, DesktopChunk, TrackOptions};
use divora_capture::{
    seconds_since, AudioFeed, CaptureStatus, EngineSignal, MonitorInfo, PipelineCommand,
    PipelineHandle, PipelineSettings, RecordOptions, RecordStats, StartError, TakeEvents,
    TakeObserver, TakeRequest,
};
use divora_core::audio::{AudioEngine, AvChunk, StreamInfo};

use crate::replay::{take_check, EngineWatch, SharedSettings, Slot, SLOT_WAIT};
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

/// v1.54.0: how long a request waits for a pipeline being started — its
/// encoder built and its first frame in — before giving up on it.
const SPAWN_WAIT: Duration = Duration::from_secs(8);

/// v1.54.0: a take on a pipeline whose desktop audio had already stopped for
/// good: not "stopped at 0:00".
const GAVE_UP_BEFORE: &str =
    "Desktop audio stopped before this recording started and Windows didn't bring it back.";

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
    /// stopped: from there the voice and soundboard are silence (desktop
    /// audio, if any, goes on).
    pub audio_lost_at: Option<f64>,
    /// Why the take ended, when it ended badly.
    pub error: Option<String>,
    /// v1.53.0: the desktop audio being recorded — `everything` or
    /// `device:<id>` — and its name; `None` without desktop audio.
    pub desktop_source_id: Option<String>,
    pub desktop_source: Option<String>,
    /// How long the desktop audio has had nothing to hear, in seconds.
    pub desktop_quiet_seconds: Option<f64>,
    /// The running engine's devices and monitor switches put Divora's own
    /// voice or soundboard on the playback device being recorded, so it is
    /// recorded twice whenever it plays — set whether or not anything is
    /// playing right now; updated about once a second.
    pub desktop_doubled: bool,
    /// Seconds into the take at which desktop audio stopped coming, while it
    /// stays stopped and is being reopened (a playback device that went
    /// away, say, or Everything except Divora sending nothing for a second).
    pub desktop_lost_at: Option<f64>,
    /// Why the take has no desktop audio although it was asked for — from
    /// the start, from when it stopped for good, or while the device
    /// recorded carries your voice to the call (taken back when it no
    /// longer does).
    pub desktop_error: Option<String>,
    /// Why the separate WAV files stopped (an error, or a WAV file's size
    /// limit), never started, or couldn't be finished.
    pub stems_error: Option<String>,
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

/// Desktop audio's volume in a take, in dB: −6 by default, which halves its
/// amplitude before it is mixed with your voice; never above as captured.
pub const DEFAULT_DESKTOP_GAIN_DB: f32 = -6.0;
pub const MIN_DESKTOP_GAIN_DB: f32 = -30.0;
pub const MAX_DESKTOP_GAIN_DB: f32 = 0.0;

/// The desktop volume asked for, in range; anything that isn't a number is
/// the default.
#[must_use]
pub fn sanitize_desktop_gain_db(db: Option<f64>) -> f32 {
    match db {
        #[allow(clippy::cast_possible_truncation)] // clamped to a few tens of dB
        Some(v) if v.is_finite() => (v as f32).clamp(MIN_DESKTOP_GAIN_DB, MAX_DESKTOP_GAIN_DB),
        _ => DEFAULT_DESKTOP_GAIN_DB,
    }
}

/// The source id for "everything except Divora".
pub const EVERYTHING_ID: &str = "everything";
/// The prefix of a playback device's source id.
pub const DEVICE_PREFIX: &str = "device:";

/// The desktop audio asked for: `everything`, `device:<endpoint id>`, or —
/// for anything else, absent and `off` included — none, as in v1.52.0.
#[must_use]
pub fn parse_desktop_source(s: Option<&str>) -> Option<DesktopSource> {
    match s? {
        EVERYTHING_ID => Some(DesktopSource::Everything),
        other => other
            .strip_prefix(DEVICE_PREFIX)
            .filter(|id| !id.is_empty())
            .map(|id| DesktopSource::Device(id.to_string())),
    }
}

/// What Divora itself plays on a playback device — which decides whether
/// recording that device records your voice twice. Process loopback
/// ("everything") never has this problem: it leaves Divora out.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum DivoraPlays {
    /// Nothing of Divora's plays there.
    Nothing,
    /// It is where Divora sends your voice, with a separate device to hear
    /// yourself on: your voice and soundboard always play there.
    Send,
    /// It is the device you hear yourself on: your voice plays there while
    /// Monitor is on, the soundboard while its own monitor is.
    Monitor,
    /// It is Divora's only output: your voice and soundboard play there
    /// while Monitor is on (off silences it entirely).
    OnlyOutput,
    /// Can't tell: the engine is stopped, or its device names match no
    /// device here.
    Unknown,
}

/// What Divora plays on the playback device `id`, given the devices there
/// are and the engine's routing. The engine knows its devices by name — the
/// name Windows shows — so a name shared by two devices counts as both.
#[must_use]
pub fn divora_plays(
    id: &str,
    devices: &[PlaybackDevice],
    routing: Option<&StreamInfo>,
) -> DivoraPlays {
    let Some(r) = routing else {
        return DivoraPlays::Unknown;
    };
    let named = |name: &str| -> Vec<&str> {
        devices
            .iter()
            .filter(|d| d.name == name)
            .map(|d| d.id.as_str())
            .collect()
    };
    // Each role from its own match: a monitor device that has gone away must
    // not hide the device the voice is sent to.
    let output = named(&r.output_name);
    let monitor = r.monitor_name.as_deref().map(named);
    if output.contains(&id) {
        return if r.monitor_name.is_some() {
            DivoraPlays::Send
        } else {
            DivoraPlays::OnlyOutput
        };
    }
    if monitor.as_ref().is_some_and(|m| m.contains(&id)) {
        return DivoraPlays::Monitor;
    }
    if output.is_empty() || monitor.as_ref().is_some_and(Vec::is_empty) {
        return DivoraPlays::Unknown;
    }
    DivoraPlays::Nothing
}

/// What Divora plays on the device it is recording, named `name` — by the
/// engine's own notion of a device, its name — or `Unknown` with no engine.
#[must_use]
pub fn device_role(name: &str, routing: Option<&StreamInfo>) -> DivoraPlays {
    let Some(r) = routing else {
        return DivoraPlays::Unknown;
    };
    if name == r.output_name {
        if r.monitor_name.is_some() {
            DivoraPlays::Send
        } else {
            DivoraPlays::OnlyOutput
        }
    } else if r.monitor_name.as_deref() == Some(name) {
        DivoraPlays::Monitor
    } else {
        DivoraPlays::Nothing
    }
}

/// Why a device Divora sends your voice to isn't recorded.
fn sends_your_voice(label: &str) -> String {
    format!(
        "Divora sends your voice to {label}, so recording it would record your voice twice \u{2014} pick another device."
    )
}

/// `0:07`, `12:34`, `1:02:03`, as the card writes times.
#[must_use]
pub fn clock(seconds: f64) -> String {
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)] // a take's length
    let total = seconds.max(0.0).floor() as u64;
    let (h, m, s) = (total / 3600, (total % 3600) / 60, total % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

/// Whether a device Divora plays on as `role` has Divora's sound on it right
/// now, given whether the engine runs and its monitor switches.
#[must_use]
pub fn doubles_now(
    role: DivoraPlays,
    running: bool,
    monitor: bool,
    speak_monitor: bool,
    monitor_gain: f32,
) -> bool {
    // A stopped engine plays nothing anywhere.
    if !running {
        return false;
    }
    match role {
        DivoraPlays::Send => true,
        DivoraPlays::OnlyOutput => monitor,
        DivoraPlays::Monitor => monitor_gain > 0.0 && (monitor || speak_monitor),
        DivoraPlays::Nothing | DivoraPlays::Unknown => false,
    }
}

/// One entry of `list_desktop_audio_sources`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DesktopSourceDto {
    /// `everything` or `device:<endpoint id>`: what `start_screen_recording`
    /// takes as `desktopAudio`.
    pub id: String,
    pub label: String,
    /// `everything` or `device`.
    pub kind: &'static str,
    /// The Windows default playback device.
    pub is_default: bool,
    pub divora_plays: DivoraPlays,
}

/// The desktop audio sources to offer: "everything except Divora" where
/// Windows can do it — it already covers every device, and leaves Divora
/// out — and otherwise each playback device, with what Divora plays there.
#[must_use]
pub fn desktop_source_dtos(
    everything: bool,
    devices: &[PlaybackDevice],
    routing: Option<&StreamInfo>,
) -> Vec<DesktopSourceDto> {
    if everything {
        return vec![DesktopSourceDto {
            id: EVERYTHING_ID.to_string(),
            label: EVERYTHING_LABEL.to_string(),
            kind: "everything",
            is_default: false,
            divora_plays: DivoraPlays::Nothing,
        }];
    }
    devices
        .iter()
        .map(|d| DesktopSourceDto {
            id: format!("{DEVICE_PREFIX}{}", d.id),
            label: d.name.clone(),
            kind: "device",
            is_default: d.is_default,
            divora_plays: divora_plays(&d.id, devices, routing),
        })
        .collect()
}

/// Whether the recorded playback device carries your voice to the call, as
/// the engine's routing changes. A pure step, so it is tested without an
/// engine.
#[derive(Debug)]
pub struct SendGuard {
    name: String,
    routing: Option<(String, Option<String>)>,
}

impl SendGuard {
    #[must_use]
    pub fn new(name: String) -> Self {
        Self {
            name,
            routing: None,
        }
    }

    /// `Some(sends)` when the routing differs from the last one seen; `None`
    /// when it is unchanged or the engine is stopped (the last answer stands).
    pub fn check(&mut self, routing: Option<&StreamInfo>) -> Option<bool> {
        let r = routing?;
        let key = (r.output_name.clone(), r.monitor_name.clone());
        if self.routing.as_ref() == Some(&key) {
            return None;
        }
        self.routing = Some(key);
        Some(device_role(&self.name, Some(r)) == DivoraPlays::Send)
    }
}

/// What "everything except Divora" is called on the card.
pub const EVERYTHING_LABEL: &str = "Everything except Divora";

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
    /// v1.53.0: desktop audio to mix in, if any, at this volume (dB).
    pub desktop: Option<DesktopSource>,
    pub desktop_gain_db: f32,
    /// Also save the voice and the desktop audio as separate WAV files.
    pub stems: bool,
}

/// The engine's tap, as the capture crate wants it: each [`AvChunk`] timed
/// against the take's start, its voice delay worked out. With desktop
/// audio, Windows' packets too, on the same clock and with the same A/V
/// offset — so the offset moves all the sound against the picture, never
/// one source against another.
pub struct EngineFeed {
    rx: Receiver<AvChunk>,
    offset: f64,
    desktop: Option<DesktopDrain>,
    /// For a playback device: the engine whose routing decides whether the
    /// device carries your voice to the call.
    guard: Option<(Arc<AudioEngine>, SendGuard)>,
    /// Set while the recorded device carries Divora's voice to the call:
    /// its packets go in as silence (timed as ever) until it no longer does.
    desktop_mute: Arc<AtomicBool>,
    /// Held so the capture runs for as long as the take; dropped (and so
    /// stopped) with the feed.
    capture: Option<DesktopCapture>,
    /// v1.54.0: what the engine is doing, as Replay's ring needs to know:
    /// a user's Stop holds it, a device rebuild doesn't.
    watch: Option<Watch>,
}

/// The engine, watched for the ring.
struct Watch {
    engine: Arc<AudioEngine>,
    stops: EngineWatch,
    since: Instant,
}

impl EngineFeed {
    #[must_use]
    pub fn new(rx: Receiver<AvChunk>, av_offset_ms: i32) -> Self {
        let ms = av_offset_ms.clamp(-MAX_AV_OFFSET_MS, MAX_AV_OFFSET_MS);
        Self {
            rx,
            offset: f64::from(ms) / 1000.0,
            desktop: None,
            guard: None,
            desktop_mute: Arc::new(AtomicBool::new(false)),
            capture: None,
            watch: None,
        }
    }

    /// v1.54.0: report the engine's stops to a pipeline (`AudioFeed::engine`).
    #[must_use]
    pub fn watch_engine(mut self, engine: Arc<AudioEngine>) -> Self {
        let stops = engine.user_stop_count();
        self.watch = Some(Watch {
            engine,
            stops: EngineWatch::new(stops),
            since: Instant::now(),
        });
        self
    }

    /// Watch the engine's routing for the playback device named `name`:
    /// whenever Divora sends your voice there, its packets go in as silence.
    /// Checked every time desktop audio is taken in, so a change of devices
    /// mid-take lets through at most what arrived before it.
    #[must_use]
    pub fn guard_device(mut self, engine: Arc<AudioEngine>, name: String) -> Self {
        self.guard = Some((engine, SendGuard::new(name)));
        self
    }

    /// The flag that silences the desktop audio, for whoever watches the
    /// engine's routing during the take.
    #[must_use]
    pub fn desktop_mute(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.desktop_mute)
    }

    /// Add desktop audio from `drain`, keeping `capture` alive with it.
    #[must_use]
    pub fn with_desktop(mut self, drain: DesktopDrain, capture: Option<DesktopCapture>) -> Self {
        self.desktop = Some(drain);
        self.capture = capture;
        self
    }

    /// What the desktop capture saw, for the log.
    fn desktop_report(&self) -> Option<String> {
        let d = self.desktop.as_ref()?;
        let (packets, untimed) = d.packets();
        Some(format!(
            "{packets} packets ({untimed} untimed), {} frames dropped, stamp lag {:?}",
            d.dropped_frames(),
            d.stamp_lag()
        ))
    }
}

impl AudioFeed for EngineFeed {
    fn poll(&mut self, start: Instant, out: &mut Vec<AudioChunk>) {
        while let Ok(c) = self.rx.try_recv() {
            out.push(to_audio_chunk(c, start, self.offset));
        }
    }

    fn engine(&mut self) -> EngineSignal {
        match self.watch.as_mut() {
            Some(w) => w.stops.poll(
                w.since.elapsed(),
                w.engine.is_running(),
                w.engine.user_stop_count(),
                w.engine.device_recovery_failed(),
            ),
            None => EngineSignal::Running,
        }
    }

    fn poll_desktop(&mut self, start: Instant, out: &mut Vec<DesktopChunk>) {
        if let Some((engine, guard)) = self.guard.as_mut() {
            if let Some(send) = guard.check(engine.stream_info().as_ref()) {
                self.desktop_mute.store(send, Ordering::Release);
            }
        }
        if let Some(d) = self.desktop.as_mut() {
            let first = out.len();
            d.drain(start, self.offset, out);
            if self.desktop_mute.load(Ordering::Acquire) {
                for c in &mut out[first..] {
                    c.frames.fill(0.0);
                }
            }
        }
    }
}

/// What the desktop capture saw goes in the log when the feed goes — the
/// take's, or the pipeline's.
impl Drop for EngineFeed {
    fn drop(&mut self) {
        if let Some(report) = self.desktop_report() {
            tracing::info!(desktop = %report, "desktop audio for the recording");
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

/// Which way a take is encoded (v1.54.0).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TakePath {
    /// The hardware encoder driven directly: Replay's path, shared with it.
    Direct,
    /// The sink writer, as v1.52.0 recorded, and why.
    Rung1(&'static str),
}

/// What decides a take's path.
#[derive(Debug, Clone, Copy, Default)]
#[allow(clippy::struct_excessive_bools)] // independent facts, each a reason
pub struct PathFacts {
    /// `DIVORA_CAPTURE_RUNG1` is set.
    pub rung1_env: bool,
    /// `DIVORA_CAPTURE_CPU_CONVERT` is set.
    pub cpu_env: bool,
    /// The PC has a hardware H.264 encoder.
    pub hardware: bool,
    /// The direct encoder already failed on this run.
    pub direct_failed: bool,
}

/// The direct path, unless a switch asks for the old one, the PC has no
/// hardware encoder, or the direct encoder already failed on this run (a
/// fresh launch tries it again).
#[must_use]
pub fn take_path(f: PathFacts) -> TakePath {
    if f.rung1_env {
        TakePath::Rung1("DIVORA_CAPTURE_RUNG1 is set")
    } else if f.cpu_env {
        // Only the sink writer's path has the CPU converter.
        TakePath::Rung1("DIVORA_CAPTURE_CPU_CONVERT is set")
    } else if !f.hardware {
        TakePath::Rung1("no hardware video encoder")
    } else if f.direct_failed {
        TakePath::Rung1("the direct encoder failed earlier on this run")
    } else {
        TakePath::Direct
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
    /// v1.54.0: why the direct encoder couldn't be used, once it failed on
    /// this run; later takes use the sink writer.
    direct_failure: Option<String>,
    /// v1.54.0: the capture pipeline, if one runs — Replay's ring, a take,
    /// or both.
    slot: Slot<PipelineHandle>,
    /// A pipeline is being started (its encoder built, its first frame
    /// awaited): nothing else may start one meanwhile.
    spawning: bool,
    /// What the running pipeline was built with: a take on it and the ring
    /// share these.
    settings: Option<SharedSettings>,
    /// Its desktop audio, as each take on it reports it.
    desktop: Option<DesktopReport>,
    /// The take running is on the pipeline (not the sink writer).
    pipeline_take: bool,
    /// The pipeline being started ended before its start was over: its
    /// handle is not put in the slot.
    spawn_ended: bool,
    replay: replay_ops::ReplayShared,
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
    #[allow(clippy::too_many_lines)] // the take's three paths, in the order tried
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
        // v1.54.0: with Replay on, the take goes on its pipeline, with its
        // settings — or is refused.
        let asked = SharedSettings::from(&req);
        let route = self.route(&capture);
        let mut s = match self.wait_slot() {
            Ok(s) => s,
            Err(why) => return refuse(why.to_string()),
        };
        // Stopped while it waited for the slot: no take, no file.
        if s.take == take && s.phase == Phase::Stopping {
            drop(s);
            return refuse(divora_capture::pipeline::NO_PICTURE.to_string());
        }
        if let Err(why) = take_check(s.replay.state, s.settings.as_ref(), &asked) {
            drop(s);
            return refuse(why);
        }
        if s.slot.running().is_some() {
            let desk = s.desktop.clone().unwrap_or_default();
            s.last.desktop_source_id.clone_from(&desk.id);
            s.last.desktop_source.clone_from(&desk.label);
            s.last.desktop_error.clone_from(&desk.error);
            emit(app, &s.last);
            let (events, start_rx) = self.take_events(app, engine, take, &req.path, desk);
            s.pipeline_take = true;
            let start = PipelineCommand::StartTake(TakeRequest {
                take,
                path: req.path.clone(),
                stems: req.stems,
                events,
            });
            let sent = s.slot.running().is_some_and(|h| h.send(start).is_ok());
            drop(s);
            if !sent {
                return refuse(STILL_STOPPING.to_string());
            }
            return wait_started(&start_rx, req.path);
        }
        let direct = route == TakePath::Direct;
        s.spawning = direct;
        drop(s);

        // Desktop audio, if asked for. Opened here, before the take's own
        // thread, so its time to open never counts against the first frame;
        // and never a reason to fail the take — without it, the take goes on
        // as v1.52.0 recorded it, saying why.
        let (feed, desk, opened) = match self.open_feed(app, engine, &req, true) {
            Ok(f) => f,
            Err(why) => {
                if direct {
                    self.spawned(None);
                }
                return refuse(why);
            }
        };
        let opts = RecordOptions {
            fps: req.fps,
            max_size: DEFAULT_MAX,
            bitrate: Some(
                req.quality
                    .bitrate((monitor.width, monitor.height), req.fps),
            ),
            cursor: req.cursor,
            audio: TrackOptions {
                desktop_gain_db: opened.then_some(req.desktop_gain_db),
                stems: req.stems,
            },
        };
        let mut feed: Box<dyn AudioFeed + Send> = Box::new(feed.watch_engine(Arc::clone(engine)));
        if direct {
            let (events, start_rx) = self.take_events(app, engine, take, &req.path, desk.clone());
            let first = PipelineCommand::StartTake(TakeRequest {
                take,
                path: req.path.clone(),
                stems: req.stems,
                events,
            });
            match divora_capture::spawn_pipeline(
                pipeline_settings(&req.monitor_id, &opts),
                feed,
                self.pipeline_events(app, engine),
                first,
            ) {
                Ok(h) => {
                    self.spawned(Some((h, asked, desk.clone(), Some(take))));
                    return wait_started(&start_rx, req.path);
                }
                Err(StartError::Direct { why, feed: back }) => {
                    // Built before any audio was taken in, so the feed is
                    // whole: the same take goes on through the sink writer.
                    tracing::warn!(
                        why,
                        "the direct encoder can't be used here; using the sink writer"
                    );
                    self.lock().direct_failure = Some(why);
                    self.spawned(None);
                    // Replay can't run here now: the card says so at once.
                    self.publish_replay(app);
                    feed = back;
                }
                Err(StartError::Other(e)) => {
                    // The tap goes before the slot frees: a request waiting
                    // for it may attach its own the moment it does.
                    engine.detach_av_tap();
                    self.spawned(None);
                    return refuse(e.to_string());
                }
            }
        }

        let (start_tx, start_rx) = channel::<Result<(), String>>();
        let path = req.path.clone();
        let run = TakeRun {
            recorder: Arc::clone(self),
            app: app.clone(),
            engine: Arc::clone(engine),
            req,
            opts,
            feed,
            desk,
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
        wait_started(&start_rx, path)
    }

    /// The engine's tap and, if asked for, the desktop audio, as a feed for
    /// a take or a pipeline; for a take (`publish`), the desktop source goes
    /// out at once. `Err` when the engine isn't running. Returns whether the
    /// desktop audio opened.
    fn open_feed(
        &self,
        app: &AppHandle,
        engine: &Arc<AudioEngine>,
        req: &StartRequest,
        publish: bool,
    ) -> Result<(EngineFeed, DesktopReport, bool), String> {
        let (tx, rx) = sync_channel::<AvChunk>(AV_QUEUE);
        if !engine.attach_av_tap(tx) {
            return Err(ENGINE_STOPPED.to_string());
        }
        let desktop = req.desktop.as_ref().map(|s| open_desktop(s, engine));
        if publish {
            if let Some(d) = &desktop {
                let mut s = self.lock();
                s.last.desktop_source_id.clone_from(&d.id);
                s.last.desktop_source.clone_from(&d.label);
                s.last.desktop_error.clone_from(&d.error);
                emit(app, &s.last);
            }
        }
        let opened = desktop.as_ref().is_some_and(|d| d.capture.is_some());
        let (feed, desk) = build_feed(rx, req.av_offset_ms, desktop, engine);
        Ok((feed, desk, opened))
    }

    /// Wait while a pipeline is being started or is on its way out (§C.1):
    /// the lock, once the slot can be used.
    fn wait_slot(&self) -> Result<std::sync::MutexGuard<'_, Shared>, &'static str> {
        let since = Instant::now();
        let mut s = self.lock();
        loop {
            let ending = matches!(s.slot, Slot::Ending(_));
            if !ending && !s.spawning {
                return Ok(s);
            }
            let limit = if s.spawning { SPAWN_WAIT } else { SLOT_WAIT };
            let Some(left) = limit.checked_sub(since.elapsed()) else {
                return Err(STILL_STOPPING);
            };
            s = self
                .changed
                .wait_timeout(s, left.min(Duration::from_millis(100)))
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
    }

    /// A pipeline start is over: running with these settings — for take
    /// `take`, or for Replay — or not started. Whatever the user did while
    /// it started is carried out on it now: a Stop of its take, Replay turned
    /// off. A pipeline that already ended (its first moments failed) is not
    /// put in the slot.
    fn spawned(&self, ok: Option<(PipelineHandle, SharedSettings, DesktopReport, Option<u64>)>) {
        let mut s = self.lock();
        s.spawning = false;
        let ended = std::mem::take(&mut s.spawn_ended);
        if let Some((h, settings, desk, take)) = ok {
            if ended {
                tracing::info!("the pipeline ended as it started");
            } else if let Err(h) = s.slot.spawned(h) {
                // Never: only this start could fill the slot.
                tracing::warn!(?h, "a second pipeline was started; it is dropped");
            } else {
                s.settings = Some(settings);
                s.desktop = Some(desk);
                match take {
                    // Its take, unless that already finished.
                    Some(t) if s.take == t && s.phase != Phase::Idle => {
                        s.pipeline_take = true;
                        if s.phase == Phase::Stopping {
                            if let Some(h) = s.slot.handle() {
                                let _ = h.send(PipelineCommand::StopTake { take: t });
                            }
                            if !s.replay.state.is_on() {
                                s.slot.exiting();
                            }
                        }
                    }
                    // Replay's — turned off while it started.
                    None if !s.replay.state.is_on() => {
                        if let Some(h) = s.slot.handle() {
                            let _ = h.send(PipelineCommand::Disarm);
                        }
                        if !s.pipeline_take {
                            s.slot.exiting();
                        }
                    }
                    Some(_) | None => {}
                }
            }
        }
        drop(s);
        self.changed.notify_all();
    }

    /// What a take on the pipeline tells the app, and how Start hears that
    /// it began (or didn't).
    fn take_events(
        self: &Arc<Self>,
        app: &AppHandle,
        engine: &Arc<AudioEngine>,
        take: u64,
        path: &Path,
        mut desk: DesktopReport,
    ) -> (Arc<dyn TakeEvents>, Receiver<Result<(), String>>) {
        let (tx, rx) = channel();
        let gave_up = desk
            .gave_up
            .as_ref()
            .is_some_and(|g| g.load(Ordering::Acquire));
        if gave_up && desk.error.is_none() {
            desk.error = Some(GAVE_UP_BEFORE.to_string());
        }
        let observer = AppObserver {
            app: app.clone(),
            recorder: Arc::clone(self),
            engine: Arc::clone(engine),
            path: path.to_path_buf(),
            started: None,
            start_tx: Some(tx),
            lost_at: None,
            desk,
        };
        let events = AppTakeEvents {
            observer: Mutex::new(observer),
            take,
        };
        (Arc::new(events), rx)
    }

    /// Which path the next take uses (v1.54.0).
    fn route(&self, capture: &divora_capture::CaptureStatus) -> TakePath {
        let route = take_path(PathFacts {
            rung1_env: std::env::var_os(divora_capture::RUNG1_ENV).is_some(),
            cpu_env: std::env::var_os(divora_capture::FORCE_CPU_ENV).is_some(),
            hardware: capture.hardware,
            direct_failed: self.lock().direct_failure.is_some(),
        });
        if let TakePath::Rung1(why) = route {
            tracing::info!(why, "this take uses the sink writer");
        }
        route
    }

    /// Stop the take and wait for its file to be finished. Returns the file.
    pub fn stop(&self, app: &AppHandle) -> Result<PathBuf, String> {
        let mut s = self.lock();
        let take = s.take;
        match s.phase.next(Input::Stop) {
            Some(next) => {
                s.phase = next;
                if s.pipeline_take {
                    if let Some(h) = s.slot.handle() {
                        let _ = h.send(PipelineCommand::StopTake { take });
                    }
                    // With Replay off the pipeline ends with the take: say
                    // so in the same breath, so nothing slips in between.
                    if !s.replay.state.is_on() {
                        s.slot.exiting();
                    }
                } else if let Some(stop) = &s.stop {
                    stop.store(true, Ordering::Release);
                }
                s.last.state = next;
                emit(app, &s.last);
            }
            None if s.phase == Phase::Stopping => {
                // Asked again: the pipeline is told again (a take stops once).
                if s.pipeline_take {
                    if let Some(h) = s.slot.handle() {
                        let _ = h.send(PipelineCommand::StopTake { take });
                    }
                }
            }
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
        s.pipeline_take = false;
        let error = outcome.as_ref().err().cloned();
        let audio = stats.and_then(|st| st.audio.as_ref());
        s.last = ScreenRecordingUpdate {
            state: s.phase,
            path: s.path.as_ref().map(|p| p.to_string_lossy().into_owned()),
            elapsed_ms: s.last.elapsed_ms,
            dropped_frames: stats.map_or(s.last.dropped_frames, |st| st.frames_dropped),
            drift_ppm: audio.map_or(s.last.drift_ppm, |a| a.timeline.drift_ppm),
            audio_lost_at: s.last.audio_lost_at,
            error,
            desktop_source_id: s.last.desktop_source_id.take(),
            desktop_source: s.last.desktop_source.take(),
            desktop_quiet_seconds: audio.map_or(s.last.desktop_quiet_seconds, |a| {
                a.timeline.desktop_quiet_seconds
            }),
            desktop_doubled: false,
            desktop_lost_at: s.last.desktop_lost_at,
            desktop_error: s.last.desktop_error.take(),
            stems_error: audio
                .and_then(|a| a.stems_error.clone())
                .or_else(|| s.last.stems_error.take()),
        };
        s.outcome = Some(outcome);
        emit(app, &s.last);
        drop(s);
        self.changed.notify_all();
    }
}

/// A take's desktop audio, as opened: the capture, what it is, what Divora
/// plays there — or why there is none.
struct TakeDesktop {
    capture: Option<DesktopCapture>,
    id: Option<String>,
    label: Option<String>,
    role: DivoraPlays,
    error: Option<String>,
}

/// Open the desktop audio for a take. A playback device Divora sends your
/// voice to is not recorded: it would carry your voice twice, whatever the
/// monitor switches say.
fn open_desktop(source: &DesktopSource, engine: &AudioEngine) -> TakeDesktop {
    let none = |error: String| TakeDesktop {
        capture: None,
        id: None,
        label: None,
        role: DivoraPlays::Nothing,
        error: Some(error),
    };
    let cap = match DesktopCapture::start(source) {
        Ok(cap) => cap,
        Err(e) => {
            tracing::warn!(error = %e, ?source, "desktop audio didn't start");
            return none(format!("Desktop audio couldn't start: {e}"));
        }
    };
    let (id, label, role) = match cap.opened() {
        OpenedSource::Everything => (
            EVERYTHING_ID.to_string(),
            EVERYTHING_LABEL.to_string(),
            DivoraPlays::Nothing,
        ),
        OpenedSource::Device { id, name } => {
            let role = device_role(name, engine.stream_info().as_ref());
            (format!("{DEVICE_PREFIX}{id}"), name.clone(), role)
        }
    };
    if role == DivoraPlays::Send {
        return none(sends_your_voice(&label));
    }
    TakeDesktop {
        capture: Some(cap),
        id: Some(id),
        label: Some(label),
        role,
        error: None,
    }
}

/// The engine's tap and the desktop capture, as one feed, and what a take
/// says about its desktop audio.
fn build_feed(
    rx: Receiver<AvChunk>,
    av_offset_ms: i32,
    desktop: Option<TakeDesktop>,
    engine: &Arc<AudioEngine>,
) -> (EngineFeed, DesktopReport) {
    let mut feed = EngineFeed::new(rx, av_offset_ms);
    let mut desk = DesktopReport::default();
    if let Some(d) = desktop {
        desk.id = d.id;
        desk.error = d.error;
        if let Some(mut cap) = d.capture {
            if matches!(cap.opened(), OpenedSource::Device { .. }) {
                desk.device_name.clone_from(&d.label);
            }
            desk.role = Some(d.role);
            desk.lost = Some(cap.lost_flag());
            desk.gave_up = Some(cap.gave_up_flag());
            desk.mute = Some(feed.desktop_mute());
            if let Some(name) = desk.device_name.clone() {
                feed = feed.guard_device(Arc::clone(engine), name);
            }
            if let Some(drain) = cap.take_drain() {
                feed = feed.with_desktop(drain, Some(cap));
            }
        }
        desk.label = d.label;
    }
    (feed, desk)
}

/// What a pipeline for `monitor_id` is built with, from a take's options.
fn pipeline_settings(monitor_id: &str, opts: &RecordOptions) -> PipelineSettings {
    PipelineSettings {
        monitor_id: monitor_id.to_string(),
        fps: opts.fps,
        max_size: opts.max_size,
        bitrate: opts
            .bitrate
            .unwrap_or_else(|| default_bitrate(opts.max_size, opts.fps)),
        cursor: opts.cursor,
        track: TrackOptions {
            desktop_gain_db: opts.audio.desktop_gain_db,
            stems: false,
        },
    }
}

/// Start's answer: the take began (its first frame is in), or why not.
fn wait_started(rx: &Receiver<Result<(), String>>, path: PathBuf) -> Result<PathBuf, String> {
    match rx.recv_timeout(START_TIMEOUT) {
        Ok(Ok(())) => Ok(path),
        Ok(Err(why)) => Err(why),
        Err(_) => Err("The screen recording didn't start.".to_string()),
    }
}

/// Everything a v1.53.0 take's `divora-encoder` thread needs (v1.54.0: the
/// sink writer's path, where the encoder can't be driven directly).
struct TakeRun {
    recorder: Arc<ScreenRecorder>,
    app: AppHandle,
    engine: Arc<AudioEngine>,
    req: StartRequest,
    opts: RecordOptions,
    feed: Box<dyn AudioFeed + Send>,
    desk: DesktopReport,
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
            mut feed,
            desk,
            stop,
            start_tx,
            take,
        } = self;
        let mut observer = AppObserver {
            app: app.clone(),
            recorder: Arc::clone(&recorder),
            engine: Arc::clone(&engine),
            path: req.path.clone(),
            started: None,
            start_tx: Some(start_tx),
            lost_at: None,
            desk,
        };
        let audio: &mut dyn AudioFeed = &mut *feed;
        let result = divora_capture::record_monitor(
            &req.monitor_id,
            &req.path,
            &opts,
            &stop,
            None,
            Some(audio),
            &mut observer,
        );
        // Stops the desktop capture too (waiting a moment at most).
        drop(feed);
        engine.detach_av_tap();
        let take_stats = result.as_ref().ok().cloned();
        let outcome = match result {
            // Ended by itself (a sleep): the file is kept and named, and the
            // card says why.
            Ok(st) if req.path.exists() => st.cut.clone().map_or_else(|| Ok(req.path.clone()), Err),
            Ok(_) => Err(divora_capture::pipeline::NO_PICTURE.to_string()),
            Err(e) => Err(e.to_string()),
        };
        // A take that never started still owes Start its answer.
        if let Some(tx) = observer.start_tx.take() {
            let _ = tx.send(outcome.as_ref().map(|_| ()).map_err(Clone::clone));
        }
        if let Some(st) = &take_stats {
            log_finished(st, false);
        }
        recorder.finish(&app, take, outcome, take_stats.as_ref());
    }
}

/// The take's numbers, once, in the log.
fn log_finished(st: &RecordStats, direct: bool) {
    tracing::info!(
        direct,
        frames = st.frames_written,
        dropped = st.frames_dropped,
        encoder = ?st.encoder,
        hardware = st.hardware,
        audio = ?st.audio,
        "screen recording finished"
    );
}

/// A take on the pipeline: the same observer a v1.53.0 take has, called
/// from the pipeline's threads.
struct AppTakeEvents {
    observer: Mutex<AppObserver>,
    take: u64,
}

impl AppTakeEvents {
    fn observer(&self) -> std::sync::MutexGuard<'_, AppObserver> {
        self.observer.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl TakeEvents for AppTakeEvents {
    fn started(&self) {
        self.observer().started();
    }

    fn progress(&self, stats: &RecordStats) {
        self.observer().progress(stats);
    }

    fn finished(&self, outcome: Result<PathBuf, String>, stats: RecordStats) {
        let mut o = self.observer();
        if let Some(tx) = o.start_tx.take() {
            let _ = tx.send(outcome.as_ref().map(|_| ()).map_err(Clone::clone));
        }
        let (recorder, app) = (Arc::clone(&o.recorder), o.app.clone());
        drop(o);
        log_finished(&stats, true);
        recorder.finish(&app, self.take, outcome, Some(&stats));
    }
}

fn emit(app: &AppHandle, update: &ScreenRecordingUpdate) {
    let _ = app.emit(EVENT, update);
}

/// What the take reports about its desktop audio. v1.54.0: a pipeline's is
/// made once and copied into every take on it.
#[derive(Debug, Clone, Default)]
struct DesktopReport {
    id: Option<String>,
    label: Option<String>,
    error: Option<String>,
    /// The playback device being recorded, by name; `None` for everything
    /// except Divora (which Divora's routing can't touch).
    device_name: Option<String>,
    /// What Divora plays on the device being recorded; `None` without one.
    role: Option<DivoraPlays>,
    /// The engine's routing the role was worked out for.
    routing: Option<(String, Option<String>)>,
    mute: Option<Arc<AtomicBool>>,
    lost: Option<Arc<AtomicBool>>,
    gave_up: Option<Arc<AtomicBool>>,
    lost_at: Option<f64>,
}

impl DesktopReport {
    /// Follow the engine's routing: if it changed, what Divora plays on the
    /// recorded device is worked out again, and a device that now carries
    /// your voice to the call says why it went silent (the feed silences it,
    /// see [`EngineFeed::guard_device`]) until it no longer does. With the
    /// engine stopped the last answer stands.
    fn follow(&mut self, routing: Option<&StreamInfo>) {
        let (Some(name), Some(r)) = (self.device_name.as_deref(), routing) else {
            return;
        };
        let key = (r.output_name.clone(), r.monitor_name.clone());
        if self.routing.as_ref() == Some(&key) {
            return;
        }
        self.routing = Some(key);
        let role = device_role(name, Some(r));
        self.role = Some(role);
        let said = sends_your_voice(name);
        if role == DivoraPlays::Send {
            self.error = Some(said);
        } else if self.error.as_deref() == Some(said.as_str()) {
            // Only its own message is taken back; "stopped for good" stays.
            self.error = None;
        }
    }
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
    desk: DesktopReport,
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
        // The engine stopped: the take goes on, and its voice and soundboard
        // are silence from here until the engine is started again (which
        // re-attaches); desktop audio, if any, goes on.
        let running = self.engine.is_running();
        if running {
            self.lost_at = None;
        } else if self.lost_at.is_none() {
            self.lost_at = Some(elapsed.as_secs_f64());
        }
        self.desk.follow(self.engine.stream_info().as_ref());
        // Desktop audio that stopped coming: being reopened — or, if Windows
        // stopped answering, gone for the rest of the take, which is said
        // once, as an error, rather than "trying again" for ever.
        match &self.desk.lost {
            Some(lost) if lost.load(Ordering::Acquire) => {
                self.desk.lost_at.get_or_insert(elapsed.as_secs_f64());
            }
            _ => self.desk.lost_at = None,
        }
        let gave_up = self
            .desk
            .gave_up
            .as_ref()
            .is_some_and(|g| g.load(Ordering::Acquire));
        if gave_up {
            let at = self.desk.lost_at.take().unwrap_or(elapsed.as_secs_f64());
            if self.desk.error.is_none() {
                self.desk.error = Some(format!(
                    "Desktop audio stopped at {} and Windows didn't bring it back.",
                    clock(at)
                ));
            }
        }
        let muted = self
            .desk
            .mute
            .as_ref()
            .is_some_and(|m| m.load(Ordering::Acquire));
        let doubled = !muted
            && self.desk.role.is_some_and(|role| {
                doubles_now(
                    role,
                    running,
                    self.engine.is_monitoring(),
                    self.engine.is_speak_monitoring(),
                    self.engine.monitor_gain(),
                )
            });
        let audio = stats.audio.as_ref();
        self.recorder.progress(
            &self.app,
            ScreenRecordingUpdate {
                state: Phase::Recording,
                path: Some(self.path.to_string_lossy().into_owned()),
                elapsed_ms: u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX),
                dropped_frames: stats.frames_dropped,
                drift_ppm: audio.map_or(0.0, |a| a.timeline.drift_ppm),
                audio_lost_at: self.lost_at,
                error: None,
                desktop_source_id: self.desk.id.clone(),
                desktop_source: self.desk.label.clone(),
                desktop_quiet_seconds: audio.and_then(|a| a.timeline.desktop_quiet_seconds),
                desktop_doubled: doubled,
                desktop_lost_at: self.desk.lost_at,
                desktop_error: self.desk.error.clone(),
                stems_error: audio.and_then(|a| a.stems_error.clone()),
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::{
        clock, desktop_source_dtos, device_role, divora_plays, doubles_now, monitor_dtos,
        parse_desktop_source, sanitize_desktop_gain_db, sanitize_fps, screen_recording_path,
        to_audio_chunk, CaptureStatusDto, DesktopReport, DivoraPlays, EngineFeed, Input,
        MonitorDto, Phase, Quality, ScreenRecordingUpdate, SendGuard, DEFAULT_DESKTOP_GAIN_DB,
    };
    use divora_capture::desktop::{desktop_tap, DesktopPacket, DesktopSource, PlaybackDevice};
    use divora_capture::{AudioFeed, MonitorInfo};
    use divora_core::audio::{AvChunk, StreamInfo};
    use std::path::Path;
    use std::sync::atomic::Ordering;
    use std::sync::mpsc::sync_channel;
    use std::time::{Duration, Instant};

    /// The whole transition table: every pair, so a new arm cannot slip in
    /// unnoticed.
    /// The direct encoder unless a switch, the hardware or an earlier
    /// failure says otherwise — and never with the CPU converter forced,
    /// which only the sink writer's path has.
    #[test]
    fn takes_choose_their_path() {
        use super::{take_path, PathFacts, TakePath};
        let ok = PathFacts {
            hardware: true,
            ..PathFacts::default()
        };
        assert_eq!(take_path(ok), TakePath::Direct);
        for f in [
            PathFacts {
                rung1_env: true,
                ..ok
            },
            PathFacts {
                cpu_env: true,
                ..ok
            },
            PathFacts {
                hardware: false,
                ..ok
            },
            PathFacts {
                direct_failed: true,
                ..ok
            },
        ] {
            assert!(matches!(take_path(f), TakePath::Rung1(_)), "{f:?}");
        }
    }

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
                "desktopDoubled",
                "desktopError",
                "desktopLostAt",
                "desktopQuietSeconds",
                "desktopSource",
                "desktopSourceId",
                "driftPpm",
                "droppedFrames",
                "elapsedMs",
                "error",
                "path",
                "state",
                "stemsError"
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
        let source = serde_json::to_value(&desktop_source_dtos(true, &[], None)[0]).unwrap();
        let mut keys: Vec<_> = source.as_object().unwrap().keys().cloned().collect();
        keys.sort();
        assert_eq!(keys, ["divoraPlays", "id", "isDefault", "kind", "label"]);
        for (role, s) in [
            (DivoraPlays::Nothing, "nothing"),
            (DivoraPlays::Send, "send"),
            (DivoraPlays::Monitor, "monitor"),
            (DivoraPlays::OnlyOutput, "onlyOutput"),
            (DivoraPlays::Unknown, "unknown"),
        ] {
            assert_eq!(serde_json::to_value(role).unwrap(), s);
        }
    }

    /// `everything` and `device:<id>`; everything else — absent, `off`, a
    /// value from a later version, an empty id — is no desktop audio, as
    /// v1.52.0 recorded.
    #[test]
    fn desktop_sources_parse_strictly() {
        assert_eq!(
            parse_desktop_source(Some("everything")),
            Some(DesktopSource::Everything)
        );
        assert_eq!(
            parse_desktop_source(Some("device:{0.0.0.00000000}.{abc}")),
            Some(DesktopSource::Device("{0.0.0.00000000}.{abc}".into()))
        );
        for none in [
            None,
            Some("off"),
            Some(""),
            Some("device:"),
            Some("app:123"),
            Some("Everything"),
        ] {
            assert_eq!(parse_desktop_source(none), None, "{none:?}");
        }
    }

    #[test]
    fn the_desktop_volume_is_clamped() {
        assert!((sanitize_desktop_gain_db(None) - DEFAULT_DESKTOP_GAIN_DB).abs() < 1e-6);
        assert!((sanitize_desktop_gain_db(Some(f64::NAN)) - DEFAULT_DESKTOP_GAIN_DB).abs() < 1e-6);
        assert!((sanitize_desktop_gain_db(Some(12.0)) - 0.0).abs() < 1e-6);
        assert!((sanitize_desktop_gain_db(Some(-99.0)) + 30.0).abs() < 1e-6);
        assert!((sanitize_desktop_gain_db(Some(-12.5)) + 12.5).abs() < 1e-6);
    }

    fn device(id: &str, name: &str) -> PlaybackDevice {
        PlaybackDevice {
            id: id.into(),
            name: name.into(),
            is_default: false,
        }
    }

    fn routing(output: &str, monitor: Option<&str>) -> StreamInfo {
        StreamInfo {
            input_name: "CABLE Output".into(),
            output_name: output.into(),
            monitor_name: monitor.map(Into::into),
            sample_rate: 48_000,
            output_rate: 48_000,
            monitor_rate: monitor.map(|_| 48_000),
            input_channels: 1,
            output_channels: 2,
        }
    }

    /// What Divora plays on each device, for every routing that matters —
    /// and whether that puts your voice in the recording twice, for every
    /// combination of the monitor switches.
    #[test]
    fn what_divora_plays_where() {
        let devices = [
            device("cable", "CABLE Input"),
            device("phones", "Headphones"),
            device("spk", "Speakers"),
        ];
        // The recommended setup: the call on the cable, you on headphones.
        let recommended = routing("CABLE Input", Some("Headphones"));
        assert_eq!(
            divora_plays("cable", &devices, Some(&recommended)),
            DivoraPlays::Send
        );
        assert_eq!(
            divora_plays("phones", &devices, Some(&recommended)),
            DivoraPlays::Monitor
        );
        assert_eq!(
            divora_plays("spk", &devices, Some(&recommended)),
            DivoraPlays::Nothing
        );
        // One output only.
        let single = routing("Speakers", None);
        assert_eq!(
            divora_plays("spk", &devices, Some(&single)),
            DivoraPlays::OnlyOutput
        );
        assert_eq!(
            divora_plays("phones", &devices, Some(&single)),
            DivoraPlays::Nothing
        );
        // Stopped, or a device name that matches nothing here: can't tell.
        assert_eq!(divora_plays("spk", &devices, None), DivoraPlays::Unknown);
        let renamed = routing("Old Name", None);
        assert_eq!(
            divora_plays("spk", &devices, Some(&renamed)),
            DivoraPlays::Unknown
        );
        let renamed_monitor = routing("CABLE Input", Some("Gone"));
        assert_eq!(
            divora_plays("phones", &devices, Some(&renamed_monitor)),
            DivoraPlays::Unknown
        );
        // Two devices with one name: both count — each by its own match,
        // even when the output's name resolves to nothing here.
        let twins = [device("a", "USB Audio"), device("b", "USB Audio")];
        let r = routing("CABLE Input", Some("USB Audio"));
        assert_eq!(divora_plays("b", &twins, Some(&r)), DivoraPlays::Monitor);
        let twins = [
            device("cable", "CABLE Input"),
            device("a", "USB Audio"),
            device("b", "USB Audio"),
        ];
        assert_eq!(divora_plays("a", &twins, Some(&r)), DivoraPlays::Monitor);
        assert_eq!(divora_plays("b", &twins, Some(&r)), DivoraPlays::Monitor);

        for monitor in [false, true] {
            for speak in [false, true] {
                for gain in [0.0, 1.0] {
                    let case = format!("monitor {monitor}, speak {speak}, gain {gain}");
                    assert!(
                        doubles_now(DivoraPlays::Send, true, monitor, speak, gain),
                        "{case}"
                    );
                    assert_eq!(
                        doubles_now(DivoraPlays::OnlyOutput, true, monitor, speak, gain),
                        monitor,
                        "{case}"
                    );
                    assert_eq!(
                        doubles_now(DivoraPlays::Monitor, true, monitor, speak, gain),
                        gain > 0.0 && (monitor || speak),
                        "{case}"
                    );
                    assert!(
                        !doubles_now(DivoraPlays::Nothing, true, monitor, speak, gain),
                        "{case}"
                    );
                    assert!(
                        !doubles_now(DivoraPlays::Unknown, true, monitor, speak, gain),
                        "{case}"
                    );
                }
            }
        }
    }

    /// A stopped engine plays nothing, so nothing is recorded twice whatever
    /// the switches say; and a monitor device that has gone away does not
    /// hide the device the voice is sent to.
    #[test]
    fn doubling_needs_a_running_engine_and_survives_a_lost_monitor() {
        for role in [
            DivoraPlays::Send,
            DivoraPlays::OnlyOutput,
            DivoraPlays::Monitor,
        ] {
            assert!(!doubles_now(role, false, true, true, 1.0), "{role:?}");
        }
        let devices = [device("cable", "CABLE Input"), device("spk", "Speakers")];
        let gone = routing("CABLE Input", Some("Headphones"));
        assert_eq!(
            divora_plays("cable", &devices, Some(&gone)),
            DivoraPlays::Send
        );
        assert_eq!(
            divora_plays("spk", &devices, Some(&gone)),
            DivoraPlays::Unknown
        );
        assert_eq!(
            divora_plays("cable", &[], Some(&gone)),
            DivoraPlays::Unknown
        );
    }

    /// The device being recorded, by the engine's own name for it.
    #[test]
    fn the_recorded_device_s_role_by_name() {
        let r = routing("CABLE Input", Some("Headphones"));
        assert_eq!(device_role("CABLE Input", Some(&r)), DivoraPlays::Send);
        assert_eq!(device_role("Headphones", Some(&r)), DivoraPlays::Monitor);
        assert_eq!(device_role("Speakers", Some(&r)), DivoraPlays::Nothing);
        let single = routing("Speakers", None);
        assert_eq!(
            device_role("Speakers", Some(&single)),
            DivoraPlays::OnlyOutput
        );
        assert_eq!(device_role("Speakers", None), DivoraPlays::Unknown);
    }

    /// Mid-take, the engine's routing changes: the role follows it, and a
    /// device that comes to carry the voice to the call says why it went
    /// silent — until it no longer does. A stop keeps the answer. (The
    /// silencing itself is the feed's: see the guard's test below.)
    #[test]
    fn the_role_follows_the_routing_mid_take() {
        let mut desk = DesktopReport {
            device_name: Some("Speakers".into()),
            role: Some(DivoraPlays::Nothing),
            ..DesktopReport::default()
        };
        desk.follow(Some(&routing("CABLE Input", Some("Headphones"))));
        assert_eq!(desk.role, Some(DivoraPlays::Nothing));
        // The monitor moves onto the recorded device.
        desk.follow(Some(&routing("CABLE Input", Some("Speakers"))));
        assert_eq!(desk.role, Some(DivoraPlays::Monitor));
        assert!(desk.error.is_none());
        // It becomes where the voice is sent, with a monitor elsewhere.
        desk.follow(Some(&routing("Speakers", Some("Headphones"))));
        assert_eq!(desk.role, Some(DivoraPlays::Send));
        assert!(desk
            .error
            .as_deref()
            .unwrap_or("")
            .contains("Divora sends your voice to Speakers"));
        // The engine stops: the last answer stands.
        desk.follow(None);
        assert_eq!(desk.role, Some(DivoraPlays::Send));
        // And back.
        desk.follow(Some(&routing("CABLE Input", None)));
        assert_eq!(desk.role, Some(DivoraPlays::Nothing));
        assert!(desk.error.is_none());
        // Another message (a capture that gave up) isn't taken back.
        desk.error = Some("Desktop audio stopped at 0:05".into());
        desk.follow(Some(&routing("CABLE Input", Some("Speakers"))));
        assert_eq!(desk.error.as_deref(), Some("Desktop audio stopped at 0:05"));
        // Everything except Divora is never touched by Divora's routing.
        let mut everything = DesktopReport::default();
        everything.follow(Some(&routing("Speakers", None)));
        assert_eq!(everything.role, None);
    }

    /// The feed's guard: an answer whenever the routing changes, none while
    /// it doesn't (or the engine is stopped).
    #[test]
    fn the_feed_s_guard_answers_on_every_routing_change() {
        let mut g = SendGuard::new("Speakers".into());
        assert_eq!(
            g.check(Some(&routing("CABLE Input", Some("Headphones")))),
            Some(false)
        );
        assert_eq!(
            g.check(Some(&routing("CABLE Input", Some("Headphones")))),
            None
        );
        assert_eq!(g.check(None), None);
        assert_eq!(
            g.check(Some(&routing("Speakers", Some("Headphones")))),
            Some(true)
        );
        assert_eq!(g.check(Some(&routing("Speakers", None))), Some(false));
    }

    #[test]
    fn times_read_like_the_card_s() {
        assert_eq!(clock(7.9), "0:07");
        assert_eq!(clock(754.0), "12:34");
        assert_eq!(clock(3_723.0), "1:02:03");
        assert_eq!(clock(-1.0), "0:00");
    }

    /// Where "everything except Divora" works it is the one source offered;
    /// elsewhere each device is, as `device:<id>`, with its role.
    #[test]
    fn the_sources_offered() {
        let everything = desktop_source_dtos(true, &[device("x", "X")], None);
        assert_eq!(everything.len(), 1);
        assert_eq!(
            (everything[0].id.as_str(), everything[0].kind),
            ("everything", "everything")
        );
        assert_eq!(everything[0].divora_plays, DivoraPlays::Nothing);
        let mut spk = device("spk", "Speakers");
        spk.is_default = true;
        let devices = [device("cable", "CABLE Input"), spk];
        let r = routing("CABLE Input", None);
        let list = desktop_source_dtos(false, &devices, Some(&r));
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].id, "device:cable");
        assert_eq!(list[0].divora_plays, DivoraPlays::OnlyOutput);
        assert_eq!(
            (list[1].id.as_str(), list[1].kind, list[1].is_default),
            ("device:spk", "device", true)
        );
        assert_eq!(list[1].divora_plays, DivoraPlays::Nothing);
    }

    /// Desktop audio reaches the take on the same clock as the voice, with
    /// the same A/V offset: the offset moves all the sound, never one
    /// source against the other.
    #[test]
    fn desktop_audio_is_timed_like_the_voice() {
        let (_tx, rx) = sync_channel::<AvChunk>(1);
        let (mut tap, drain) = desktop_tap();
        let start = Instant::now();
        let samples = [0.5f32; 960];
        let at = start + Duration::from_millis(250);
        tap.ingest(&DesktopPacket {
            samples: &samples,
            frames: 480,
            at,
            arrived: at,
            timed: true,
            silent: false,
            discontinuity: false,
            resumed: false,
        });
        let mut feed = EngineFeed::new(rx, 40).with_desktop(drain, None);
        let mut out = Vec::new();
        feed.poll_desktop(start, &mut out);
        assert_eq!(out.len(), 1);
        assert!((out[0].at - 0.29).abs() < 1e-9, "at {}", out[0].at);
        // The engine's buffers get the same 40 ms (to_audio_chunk, above).
        let voice = to_audio_chunk(
            AvChunk {
                epoch: 1,
                at,
                index: 0,
                rate: 48_000,
                out_frames: 480,
                out_rate: 48_000,
                occupancy: 0,
                chain_latency: 0,
                input_latency: Duration::ZERO,
                voice: vec![0.0; 480],
                sb: vec![0.0; 480],
            },
            start,
            0.04,
        );
        assert!((voice.at - out[0].at).abs() < 1e-9);
        // Muted (the device came to carry the voice to the call): the same
        // packets, on time, as silence.
        assert!(out[0].frames.iter().all(|&s| s != 0.0));
        let mute = feed.desktop_mute();
        mute.store(true, Ordering::Release);
        let (_tx2, rx2) = sync_channel::<AvChunk>(1);
        let (mut tap2, drain2) = desktop_tap();
        tap2.ingest(&DesktopPacket {
            samples: &samples,
            frames: 480,
            at,
            arrived: at,
            timed: true,
            silent: false,
            discontinuity: false,
            resumed: false,
        });
        let mut muted_feed = EngineFeed::new(rx2, 40).with_desktop(drain2, None);
        muted_feed.desktop_mute().store(true, Ordering::Release);
        let mut out2 = Vec::new();
        muted_feed.poll_desktop(start, &mut out2);
        assert_eq!(out2.len(), 1);
        assert!((out2[0].at - 0.29).abs() < 1e-9);
        assert!(
            out2[0].frames.iter().all(|&s| s == 0.0),
            "muted audio still sounds"
        );
    }
}
