//! v1.54.0: Replay — the last 15, 30, 60 or 120 seconds of one screen, with
//! your voice and soundboard (and desktop audio, when the card records it),
//! kept in memory and saved when you ask.
//!
//! This module is the rules, and only the rules: what state Replay is in and
//! what moves it ([`ReplayState::next`]), who may use the one capture
//! pipeline ([`Slot`]), when turning Replay on or starting a take is refused
//! and in what words, the engine watch that tells a user's Stop from a
//! device rebuild ([`EngineWatch`]), the memory figures, the file name, the
//! tray's view and the wire shapes. Nothing here starts a thread, touches the
//! engine or calls Tauri, so every rule is tested on any machine; the
//! commands, events and tray wiring apply them.

use std::time::Duration;

use divora_capture::desktop::DesktopSource;
use divora_capture::pipeline::{NOTHING_YET, SAVE_DROPPED, STILL_STOPPING};
use divora_capture::replay::budget;
use divora_capture::replay::PendingCut;
use divora_capture::{CaptureStatus, EngineSignal, HeldReason, LocalTime, RingStatus, SavedClip};
use serde::{Serialize, Serializer};

use crate::screen::{
    parse_desktop_source, Quality, StartRequest, DEVICE_PREFIX, ENGINE_STOPPED, EVERYTHING_ID,
    MAX_AV_OFFSET_MS,
};

/// The event every state change, `saving` change and (while armed) 1 Hz
/// report goes out as. Payload: [`ReplayStatus`].
pub const EVENT_BUFFER: &str = "replay-buffer";

/// The event every save attempt ends in — refusals included, exactly once.
/// Payload: [`ReplaySaved`].
pub const EVENT_SAVED: &str = "replay-saved";

/// A Save with Replay off (the hotkey works whether or not it is on), and a
/// second Save while the first is still being written: one at a time. The
/// pipeline says the same words.
pub use divora_capture::pipeline::{REPLAY_OFF, STILL_SAVING};

/// Why Replay can't run on a PC whose only H.264 encoder is software:
/// Replay runs only on the directly driven encoder, and that is a hardware
/// one.
pub const NO_HARDWARE: &str =
    "Replay needs a hardware video encoder; this PC encodes with the CPU.";

/// Why Replay can't run once the hardware encoder failed to be driven
/// directly on this run (a fresh launch tries again).
pub const NO_DIRECT: &str = "Replay can't use this PC's video encoder.";

/// Screen recording is unsupported and the probe gave no reason — the words
/// a take is refused with in the same case.
pub const UNAVAILABLE: &str = "Screen recording isn't available.";

/// An arm while a take runs on the sink writer, which has no ring to share
/// (only reachable with `DIVORA_CAPTURE_RUNG1` set).
pub const RUNG1_TAKE: &str = "Stop the screen recording first, then turn Replay on.";

/// An arm while Replay is already on with other settings than the length.
pub const ALREADY_ON: &str =
    "Replay is already on for another screen or settings — turn it off first.";

/// A take while Replay is held for a lost screen: the pipeline has no
/// capture left to share.
pub const SCREEN_LOST: &str = "Replay lost its screen — turn Replay off to record.";

/// A take while Replay is held for a failed encoder: nothing to share.
pub const ENCODER_LOST: &str = "Replay lost its encoder — turn Replay off to record.";

/// How the save thread's no-space refusal begins (§D: "There isn't enough
/// free space on the drive to save the replay (it needs about N MB).").
pub const NO_SPACE_PREFIX: &str = "There isn't enough free space";

/// How long a request waits for a pipeline on its way out to be gone. It
/// covers the longest bounded exit: a take's end (1.5 s), the encoder's
/// drain (0.5 s) and its quiesce (0.1 s).
pub const SLOT_WAIT: Duration = Duration::from_secs(3);

/// The length Replay keeps when none (or one not offered) is asked for.
pub const DEFAULT_SECONDS: u32 = 30;

/// The tray's product name — its whole tooltip while Replay is off, as
/// `setup_tray` sets it.
pub const PRODUCT: &str = "DivoraVoice";

/// The tray's tooltip while Quit waits for a save to finish.
pub const QUIT_SAVING_TOOLTIP: &str = "DivoraVoice · saving replay…";

/// The tray menu's Replay items, shown while Replay is on or held. The tray
/// has no item to turn Replay on: that goes through the card's explainer.
pub const TRAY_SAVE_ID: &str = "replay-save";
pub const TRAY_SAVE_LABEL: &str = "Save replay";
pub const TRAY_OFF_ID: &str = "replay-off";
pub const TRAY_OFF_LABEL: &str = "Turn Replay off";

/// The most UTF-16 units a tray tooltip keeps: tray-icon copies at most 128
/// into Windows' buffer with no room for the terminator.
pub const TOOLTIP_MAX_UNITS: usize = 127;

/// The tray icon while Replay is on: the app icon with a red dot in the
/// lower right, as raw 32×32 RGBA (`scripts/make-tray-replay-icon.py` makes
/// it from `icons/32x32.png`). Raw, so the tray needs no PNG decoder.
pub const REPLAY_ICON: &[u8] = include_bytes!("../icons/tray-replay.rgba");
/// [`REPLAY_ICON`]'s width and height.
pub const REPLAY_ICON_SIZE: u32 = 32;

/// How fast Quit assumes a save writes, in bytes per second: a slow USB
/// stick. The largest clip (≈ 725 MB) then gets about 36 s.
pub const QUIT_SAVE_RATE: u64 = 20_000_000;
/// The least and most Quit waits for a save's writing.
pub const QUIT_WAIT_MIN: Duration = Duration::from_secs(5);
pub const QUIT_WAIT_MAX: Duration = Duration::from_secs(60);
/// After that, Quit cancels the save and waits up to this long for it to
/// stop and remove its `.part`. A save still writing stops within a few
/// writes; one already finishing its file can't be stopped, and has about
/// the writer's queue (4 MB, `QUEUE_CAP`) to flush — 8 s at 0.5 MB/s.
pub const QUIT_CANCEL_WAIT: Duration = Duration::from_secs(10);

/// Where Replay is.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ReplayState {
    #[default]
    Off,
    /// On, and waiting for the first keyframe of a segment: nothing to save
    /// yet.
    Arming,
    /// The ring holds a keyframe: Save cuts a clip.
    Armed,
    /// The ring keeps what it has but takes in nothing new, and why.
    Held(HeldReason),
}

/// What can happen to Replay.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplayInput {
    /// `arm_replay_buffer` passed its checks.
    Arm,
    /// The first keyframe of a segment is in the ring.
    Ready,
    /// The pipeline couldn't be started for the ring, or refused the Arm on
    /// its way out.
    ArmFailed,
    /// Turned off: the card, the tray, the hotkey or Quit.
    Disarm,
    /// The user stopped the engine, it couldn't bring its devices back, or
    /// it was down past [`EngineWatch::BACKSTOP`].
    EngineStopped,
    /// The engine started again after that.
    EngineStarted,
    /// The screen went away.
    ScreenLost,
    /// The encoder failed, stalled, or a rebuild of it failed.
    EncoderFailed,
    /// A sleep or an encoder restart: the session is rebuilt and the ring
    /// cleared.
    Rebuilding,
    /// That rebuild failed: Replay turns off, whatever it held (the
    /// owner's decision for v1.54.0).
    Failed,
    /// The pipeline's thread is gone.
    Ended,
}

impl ReplayState {
    /// The state after `input`, or `None` when `input` means nothing here.
    /// An Arm while Armed is `None` too: the caller resizes the ring.
    #[must_use]
    pub fn next(self, input: ReplayInput) -> Option<Self> {
        use HeldReason as Why;
        use ReplayInput as In;
        use ReplayState::{Armed, Arming, Held, Off};
        match (self, input) {
            (Off, In::Arm) => Some(Arming),
            (Arming, In::Ready) => Some(Armed),
            // Arming has nothing to keep, so whatever would hold a ring turns
            // it off instead.
            (Arming, In::ArmFailed | In::EngineStopped | In::ScreenLost | In::EncoderFailed)
            | (Arming | Armed | Held(_), In::Disarm | In::Failed | In::Ended) => Some(Off),
            (Arming | Armed, In::Rebuilding) | (Held(Why::EngineStopped), In::EngineStarted) => {
                Some(Arming)
            }
            (Armed, In::EngineStopped) => Some(Held(Why::EngineStopped)),
            // A lost screen or encoder outranks a stopped engine: the engine
            // coming back can't bring them back.
            (Armed | Held(Why::EngineStopped), In::ScreenLost) => Some(Held(Why::ScreenLost)),
            (Armed | Held(Why::EngineStopped), In::EncoderFailed) => Some(Held(Why::EncoderFailed)),
            // A rebuild clears a held ring but doesn't un-hold it: an
            // engine-stopped hold still waits for the engine.
            (Held(why), In::Rebuilding) => Some(Held(why)),
            _ => None,
        }
    }

    /// On in any form: arming, armed or held. The tray shows the Replay
    /// icon and items exactly then.
    #[must_use]
    pub fn is_on(self) -> bool {
        self != Self::Off
    }

    /// Why it is held, if it is.
    #[must_use]
    pub fn held_reason(self) -> Option<HeldReason> {
        match self {
            Self::Held(why) => Some(why),
            _ => None,
        }
    }

    /// The state as the wire names it.
    #[must_use]
    pub fn wire(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Arming => "arming",
            Self::Armed => "armed",
            Self::Held(_) => "held",
        }
    }
}

/// On the wire the state is its name; why it is held goes out separately,
/// as [`ReplayStatus::held_reason`].
impl Serialize for ReplayState {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(self.wire())
    }
}

/// Why the ring is held, as the wire names it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum HeldReasonDto {
    EngineStopped,
    ScreenLost,
    EncoderFailed,
}

impl From<HeldReason> for HeldReasonDto {
    fn from(why: HeldReason) -> Self {
        match why {
            HeldReason::EngineStopped => Self::EngineStopped,
            HeldReason::ScreenLost => Self::ScreenLost,
            HeldReason::EncoderFailed => Self::EncoderFailed,
        }
    }
}

/// Whether a Save can go ahead, or the words it is refused with. Saving is
/// a flag beside the state, not a state: one save at a time, in Armed or
/// Held.
pub fn save_check(state: ReplayState, saving: bool) -> Result<(), &'static str> {
    match state {
        ReplayState::Off => Err(REPLAY_OFF),
        _ if saving => Err(STILL_SAVING),
        ReplayState::Arming => Err(NOTHING_YET),
        ReplayState::Armed | ReplayState::Held(_) => Ok(()),
    }
}

/// The app's one capture pipeline, as the `Shared` lock holds it: free, in
/// use, or on its way out. `T` is the pipeline's handle.
#[derive(Debug, Default)]
pub enum Slot<T> {
    #[default]
    None,
    Running(T),
    /// A command after which the pipeline exits has been sent; its thread
    /// hasn't gone yet. Nothing new may start until it has.
    Ending(T),
}

/// What can be asked of the slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SlotRequest {
    StartTake,
    Arm,
    Save,
    Disarm,
    Shutdown,
}

/// What the caller does with a request, given the slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SlotAnswer {
    /// The slot is free: start a pipeline for it.
    Spawn,
    /// Send it to the running pipeline.
    Send,
    /// No pipeline runs to send it to: the caller answers it (a Save with
    /// "Replay is off.", a Disarm with the off status).
    Nothing,
    /// A pipeline is on its way out: wait for the slot to be free, up to
    /// [`SLOT_WAIT`], then ask again.
    Wait,
}

impl SlotAnswer {
    /// The answer once the wait is over: still [`SlotAnswer::Wait`] means
    /// the old pipeline outlived [`SLOT_WAIT`], and the request is refused.
    pub fn after_wait(self) -> Result<Self, &'static str> {
        match self {
            Self::Wait => Err(STILL_STOPPING),
            other => Ok(other),
        }
    }
}

impl<T> Slot<T> {
    /// What to do with `request` now.
    #[must_use]
    pub fn answer(&self, request: SlotRequest) -> SlotAnswer {
        match (self, request) {
            (Self::None, SlotRequest::StartTake | SlotRequest::Arm) => SlotAnswer::Spawn,
            (Self::None, SlotRequest::Save | SlotRequest::Disarm | SlotRequest::Shutdown) => {
                SlotAnswer::Nothing
            }
            (Self::Running(_), _) => SlotAnswer::Send,
            (Self::Ending(_), _) => SlotAnswer::Wait,
        }
    }

    /// A pipeline was started: it runs in the slot. Only a free slot takes
    /// one; otherwise the handle comes back and the slot is unchanged.
    pub fn spawned(&mut self, handle: T) -> Result<(), T> {
        if matches!(self, Self::None) {
            *self = Self::Running(handle);
            Ok(())
        } else {
            Err(handle)
        }
    }

    /// Running → Ending, in the same critical section that sends the
    /// command after which the pipeline exits: a `StopTake` with Replay off,
    /// a `Disarm` with no take open, or `Shutdown`. So no request slips in
    /// between "the pipeline decided to exit" and "the app knows". Whether
    /// it moved.
    pub fn exiting(&mut self) -> bool {
        match std::mem::replace(self, Self::None) {
            Self::Running(handle) => {
                *self = Self::Ending(handle);
                true
            }
            other => {
                *self = other;
                false
            }
        }
    }

    /// The pipeline's thread is gone (after the engine's tap is detached):
    /// the slot is free, and the handle comes back to be joined.
    pub fn ended(&mut self) -> Option<T> {
        match std::mem::replace(self, Self::None) {
            Self::Running(handle) | Self::Ending(handle) => Some(handle),
            Self::None => None,
        }
    }

    /// The handle, while the thread is alive (running or on its way out):
    /// a take still open on a pipeline that is ending is stopped through it.
    #[must_use]
    pub fn handle(&self) -> Option<&T> {
        match self {
            Self::Running(handle) | Self::Ending(handle) => Some(handle),
            Self::None => None,
        }
    }

    /// The handle to send to, while the pipeline runs.
    #[must_use]
    pub fn running(&self) -> Option<&T> {
        match self {
            Self::Running(handle) => Some(handle),
            _ => None,
        }
    }

    #[must_use]
    pub fn is_free(&self) -> bool {
        matches!(self, Self::None)
    }
}

/// The desktop volumes closer than this are the same volume.
const GAIN_TOLERANCE_DB: f32 = 0.01;

/// What a pipeline was built with, and so what a take and the ring on it
/// share. A take's stems and file name are its own.
#[derive(Debug, Clone, PartialEq)]
pub struct SharedSettings {
    pub monitor_id: String,
    pub fps: u32,
    pub quality: Quality,
    pub cursor: bool,
    /// The desktop audio ASKED for — `everything` or `device:<id>` — not
    /// whether it opened: a device that failed to open is still the one
    /// the user picked.
    pub desktop: Option<String>,
    pub desktop_gain_db: f32,
    /// Clamped to ±[`MAX_AV_OFFSET_MS`], as the feed applies it.
    pub av_offset_ms: i32,
}

impl SharedSettings {
    /// The first setting `other` differs in, by the name the user knows it
    /// by, or `None` when the two can share a pipeline. The desktop volume
    /// counts only with desktop audio: without it the volume changes
    /// nothing.
    #[must_use]
    pub fn mismatch(&self, other: &Self) -> Option<&'static str> {
        if self.monitor_id != other.monitor_id {
            Some("screen")
        } else if self.fps != other.fps {
            Some("frame rate")
        } else if self.quality != other.quality {
            Some("quality")
        } else if self.cursor != other.cursor {
            Some("pointer")
        } else if self.desktop != other.desktop {
            Some("desktop audio")
        } else if self.desktop.is_some()
            && (self.desktop_gain_db - other.desktop_gain_db).abs() > GAIN_TOLERANCE_DB
        {
            Some("desktop volume")
        } else if self.av_offset_ms != other.av_offset_ms {
            Some("A/V offset")
        } else {
            None
        }
    }
}

/// The settings a take would build its pipeline with.
impl From<&StartRequest> for SharedSettings {
    fn from(req: &StartRequest) -> Self {
        Self {
            monitor_id: req.monitor_id.clone(),
            fps: req.fps,
            quality: req.quality,
            cursor: req.cursor,
            desktop: req.desktop.as_ref().map(desktop_id),
            desktop_gain_db: req.desktop_gain_db,
            av_offset_ms: req.av_offset_ms.clamp(-MAX_AV_OFFSET_MS, MAX_AV_OFFSET_MS),
        }
    }
}

/// A desktop source as the frontend names it: `everything` or
/// `device:<endpoint id>`.
#[must_use]
pub fn desktop_id(source: &DesktopSource) -> String {
    match source {
        DesktopSource::Everything => EVERYTHING_ID.to_string(),
        DesktopSource::Device(id) => format!("{DEVICE_PREFIX}{id}"),
    }
}

/// The desktop audio asked for, as [`SharedSettings::desktop`] keeps it:
/// anything `parse_desktop_source` doesn't take (absent, `off`, a later
/// version's value) is none, so an arm and a take saying "none" in
/// different words still match.
#[must_use]
pub fn requested_desktop(raw: Option<&str>) -> Option<String> {
    parse_desktop_source(raw).as_ref().map(desktop_id)
}

/// The quality as the wire names it (what `quality` takes).
#[must_use]
pub fn quality_name(quality: Quality) -> &'static str {
    match quality {
        Quality::Small => "small",
        Quality::Balanced => "balanced",
        Quality::High => "high",
    }
}

/// Whether this PC can run Replay, and if not, why. Three cases kept apart,
/// so a PC without screen recording is never told about its encoder:
/// screen recording unsupported (the row is hidden); no hardware encoder
/// (`hardware` is false in every unsupported status, so it is only read
/// once `supported` holds); the direct encoder failed on this run. An
/// unsupported answer always carries a reason.
#[must_use]
pub fn replay_support(
    capture: &CaptureStatus,
    direct_failure: Option<&str>,
) -> (bool, Option<String>) {
    if !capture.supported {
        let why = capture.reason.clone().unwrap_or_else(|| UNAVAILABLE.into());
        (false, Some(why))
    } else if !capture.hardware {
        (false, Some(NO_HARDWARE.into()))
    } else if direct_failure.is_some() {
        (false, Some(NO_DIRECT.into()))
    } else {
        (true, None)
    }
}

/// A cap in whole megabytes (10⁶ bytes), rounded up: "up to" is never
/// below what the ring may use. The frontend's `capMb` rounds the same way.
#[must_use]
pub fn cap_megabytes(bytes: u64) -> u64 {
    bytes.div_ceil(1_000_000)
}

/// A ceiling in whole megabytes, rounded down: a length refused against it
/// always reads larger than it. The frontend's `ceilingMb` agrees, so the
/// refusal and the greyed-out option quote the same two numbers.
#[must_use]
pub fn ceiling_megabytes(bytes: u64) -> u64 {
    bytes / 1_000_000
}

/// The ring's cap for `seconds` at `bitrate`: what an Arm sends, and the
/// figure the UI and the refusal show.
#[must_use]
pub fn ring_cap(seconds: u32, bitrate: u32) -> u64 {
    budget::cap_bytes(budget::estimate_bytes(seconds, bitrate))
}

/// The refusal for a length this PC can't hold.
#[must_use]
pub fn over_ceiling(seconds: u32, cap_bytes: u64, ceiling_bytes: u64) -> String {
    format!(
        "Replay of {seconds} s at these settings needs up to {} MB of memory; this PC allows {} MB.",
        cap_megabytes(cap_bytes),
        ceiling_megabytes(ceiling_bytes)
    )
}

/// Whether `seconds` at `bitrate` fits a PC that gives Windows `total_phys`
/// bytes, or the words it is refused with: part of every arm, and all a
/// resize checks again.
pub fn memory_check(seconds: u32, bitrate: u32, total_phys: u64) -> Result<(), String> {
    if budget::allowed(seconds, bitrate, total_phys) {
        Ok(())
    } else {
        Err(over_ceiling(
            seconds,
            ring_cap(seconds, bitrate),
            budget::ceiling(total_phys),
        ))
    }
}

/// What decides whether Replay may turn on. The settings behind `bitrate`
/// are the ones the ring will use: a running take's, when there is one (an
/// arm adopts them).
#[derive(Debug, Clone, Copy)]
pub struct ArmFacts<'a> {
    pub capture: &'a CaptureStatus,
    pub direct_failure: Option<&'a str>,
    /// The slot's answer to the Arm once any wait for it is over.
    pub slot: SlotAnswer,
    pub engine_running: bool,
    /// A take is running on the sink writer, which has no ring to share.
    pub rung1_take: bool,
    pub seconds: u32,
    pub bitrate: u32,
    /// `MEMORYSTATUSEX.ullTotalPhys`, read for this arm.
    pub total_phys: u64,
}

/// Whether Replay may turn on from off, or the words it is refused with. A
/// pipeline still on its way out comes first (nothing else is known until
/// it has gone), then what this PC can't do at all, then what the user can
/// change.
pub fn arm_check(f: &ArmFacts) -> Result<(), String> {
    f.slot.after_wait()?;
    if let (false, why) = replay_support(f.capture, f.direct_failure) {
        return Err(why.unwrap_or_else(|| UNAVAILABLE.into()));
    }
    if f.rung1_take {
        return Err(RUNG1_TAKE.into());
    }
    if !f.engine_running {
        return Err(ENGINE_STOPPED.into());
    }
    memory_check(f.seconds, f.bitrate, f.total_phys)
}

/// An arm while Replay is already on: the same settings resize the ring
/// (the length is not a shared setting; [`memory_check`] the new one), any
/// other is refused.
pub fn rearm_check(armed: &SharedSettings, asked: &SharedSettings) -> Result<(), &'static str> {
    match armed.mismatch(asked) {
        Some(_) => Err(ALREADY_ON),
        None => Ok(()),
    }
}

/// The refusal for a take asking for other settings than Replay's.
#[must_use]
pub fn different_settings(field: &str) -> String {
    format!(
        "Replay is on with a different {field}. A screen recording while Replay is on uses Replay's settings — turn Replay off to record with others."
    )
}

/// Whether a take may start given Replay's state and the settings its
/// pipeline was built with (`None` when none runs), or the words it is
/// refused with. Off, Replay asks nothing of a take.
pub fn take_check(
    state: ReplayState,
    pipeline: Option<&SharedSettings>,
    asked: &SharedSettings,
) -> Result<(), String> {
    match state {
        ReplayState::Off => Ok(()),
        ReplayState::Held(HeldReason::ScreenLost) => Err(SCREEN_LOST.into()),
        ReplayState::Held(HeldReason::EncoderFailed) => Err(ENCODER_LOST.into()),
        ReplayState::Held(HeldReason::EngineStopped) => Err(ENGINE_STOPPED.into()),
        ReplayState::Arming | ReplayState::Armed => {
            match pipeline.and_then(|p| p.mismatch(asked)) {
                Some(field) => Err(different_settings(field)),
                None => Ok(()),
            }
        }
    }
}

/// Watches the engine for the ring: a stop the user made (or a recovery
/// that gave up) holds it; a device-loss rebuild — during which the engine
/// reads as not running — doesn't.
#[derive(Debug, Clone)]
pub struct EngineWatch {
    stops_seen: u64,
    stopped: bool,
    down_since: Option<Duration>,
}

impl EngineWatch {
    /// Not running this long is a stop whatever the reason: a rebuild never
    /// takes that long, and a stop nobody counted must not leave the ring
    /// "armed" with silence.
    pub const BACKSTOP: Duration = Duration::from_secs(5);

    /// `user_stops_now` is the engine's count of user stops when the
    /// pipeline starts: only later ones count.
    #[must_use]
    pub fn new(user_stops_now: u64) -> Self {
        Self {
            stops_seen: user_stops_now,
            stopped: false,
            down_since: None,
        }
    }

    /// The engine as the ring should see it at `now` (the pipeline's
    /// clock). Stopped for a user stop (the count moved) or a failed
    /// recovery while not running, or after [`Self::BACKSTOP`] of not
    /// running for any reason. A count that moved while the engine is
    /// already running again (a fast Stop → Start) reports Stopped then
    /// Running on consecutive polls, so the ring is still cleared.
    pub fn poll(
        &mut self,
        now: Duration,
        running: bool,
        user_stops: u64,
        recovery_failed: bool,
    ) -> EngineSignal {
        let moved = user_stops != self.stops_seen;
        self.stops_seen = user_stops;
        if running {
            self.down_since = None;
            if moved && !self.stopped {
                self.stopped = true;
                return EngineSignal::Stopped;
            }
            self.stopped = false;
            return EngineSignal::Running;
        }
        let since = *self.down_since.get_or_insert(now);
        if moved || recovery_failed || now.saturating_sub(since) >= Self::BACKSTOP {
            self.stopped = true;
        }
        if self.stopped {
            EngineSignal::Stopped
        } else {
            EngineSignal::Running
        }
    }
}

/// Payload of `replay_buffer_status` and the `replay-buffer` event. Every
/// key is always present; absent values are `null`.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReplayStatus {
    /// Whether this PC can run Replay ([`replay_support`]).
    pub supported: bool,
    /// Why not, when it can't.
    pub reason: Option<String>,
    /// `off`, `arming`, `armed` or `held`.
    pub state: ReplayState,
    /// A save is being cut or written.
    pub saving: bool,
    /// The length kept (or, off, the one an arm would ask for).
    pub seconds: u32,
    /// How much the ring holds now.
    pub buffered_ms: u64,
    pub bytes: u64,
    /// The most the ring may hold.
    pub cap_bytes: u64,
    /// The screen being kept.
    pub monitor_id: Option<String>,
    /// `engine-stopped`, `screen-lost` or `encoder-failed`, while held.
    pub held_reason: Option<HeldReasonDto>,
    /// What the running ring uses, so the card can lock its pickers to it
    /// and a take on it can ask for the same.
    pub armed: Option<ReplayArmed>,
}

impl ReplayStatus {
    /// Replay off: nothing kept, nothing locked.
    #[must_use]
    pub fn off(supported: bool, reason: Option<String>, seconds: u32) -> Self {
        Self {
            supported,
            reason,
            state: ReplayState::Off,
            saving: false,
            seconds,
            buffered_ms: 0,
            bytes: 0,
            cap_bytes: 0,
            monitor_id: None,
            held_reason: None,
            armed: None,
        }
    }

    /// Replay on — arming, armed or held — with what its pipeline was built
    /// with and the ring's latest report (before the first, the `seconds`
    /// and `cap_bytes` it was armed with, the rest zero). A running ring
    /// means this PC can run it. `Off` gives [`Self::off`].
    #[must_use]
    pub fn on(state: ReplayState, ring: RingStatus, settings: &SharedSettings) -> Self {
        if state == ReplayState::Off {
            return Self::off(true, None, ring.seconds);
        }
        Self {
            supported: true,
            reason: None,
            state,
            saving: false,
            seconds: ring.seconds,
            buffered_ms: ring.buffered_ms,
            bytes: ring.bytes,
            cap_bytes: ring.cap_bytes,
            monitor_id: Some(settings.monitor_id.clone()),
            held_reason: state.held_reason().map(HeldReasonDto::from),
            armed: Some(ReplayArmed::from(settings)),
        }
    }

    /// The same status with `saving` set — a save can outlive Replay being
    /// turned off.
    #[must_use]
    pub fn with_saving(mut self, saving: bool) -> Self {
        self.saving = saving;
        self
    }
}

/// The settings a running ring uses.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReplayArmed {
    pub fps: u32,
    /// `small`, `balanced` or `high`.
    pub quality: &'static str,
    pub cursor: bool,
    /// `everything` or `device:<id>`, as asked for; `null` without.
    pub desktop_audio: Option<String>,
    pub desktop_gain_db: f32,
    pub av_offset_ms: i32,
}

impl From<&SharedSettings> for ReplayArmed {
    fn from(s: &SharedSettings) -> Self {
        Self {
            fps: s.fps,
            quality: quality_name(s.quality),
            cursor: s.cursor,
            desktop_audio: s.desktop.clone(),
            desktop_gain_db: s.desktop_gain_db,
            av_offset_ms: s.av_offset_ms,
        }
    }
}

/// Payload of the `replay-saved` event: the outcome of one save attempt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReplaySaved {
    /// The clip, when it was saved.
    pub path: Option<String>,
    pub duration_ms: u64,
    /// Why it wasn't.
    pub error: Option<String>,
}

impl ReplaySaved {
    #[must_use]
    pub fn saved(clip: &SavedClip) -> Self {
        Self {
            path: Some(clip.path.to_string_lossy().into_owned()),
            duration_ms: clip.duration_ms,
            error: None,
        }
    }

    #[must_use]
    pub fn failed(error: impl Into<String>) -> Self {
        Self {
            path: None,
            duration_ms: 0,
            error: Some(error.into()),
        }
    }

    #[must_use]
    pub fn from_outcome(outcome: &Result<SavedClip, String>) -> Self {
        match outcome {
            Ok(clip) => Self::saved(clip),
            Err(e) => Self::failed(e.as_str()),
        }
    }
}

/// Payload of `replay_memory`: what each length needs, and what this PC
/// allows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReplayMemory {
    pub ceiling_bytes: u64,
    pub lengths: Vec<ReplayLength>,
}

/// One length offered.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReplayLength {
    pub seconds: u32,
    /// What it is expected to hold.
    pub bytes: u64,
    /// The most it may hold: the figure shown, as "up to … MB".
    pub cap_bytes: u64,
    /// Whether the cap is within the ceiling.
    pub allowed: bool,
}

/// What each length needs on `monitor` at `fps` (as `sanitize_fps` gives
/// it) and `quality`, from the bitrate a take there would use, against a PC
/// that gives Windows `total_phys` bytes.
#[must_use]
pub fn replay_memory(
    monitor: (u32, u32),
    fps: u32,
    quality: Quality,
    total_phys: u64,
) -> ReplayMemory {
    let bitrate = quality.bitrate(monitor, fps);
    ReplayMemory {
        ceiling_bytes: budget::ceiling(total_phys),
        lengths: budget::LENGTHS
            .iter()
            .map(|&seconds| ReplayLength {
                seconds,
                bytes: budget::estimate_bytes(seconds, bitrate),
                cap_bytes: ring_cap(seconds, bitrate),
                allowed: budget::allowed(seconds, bitrate, total_phys),
            })
            .collect(),
    }
}

/// The lengths offered; anything else is the default.
#[must_use]
pub fn sanitize_seconds(seconds: Option<u32>) -> u32 {
    match seconds {
        Some(s) if budget::LENGTHS.contains(&s) => s,
        _ => DEFAULT_SECONDS,
    }
}

/// `divora-replay-2026-10-16_21-04-05.mp4`, from local time: the same shape
/// as screen recordings' names, so the folder sorts by time. Named here,
/// not in the frontend, because the tray saves with no window open.
#[must_use]
pub fn replay_filename(t: LocalTime) -> String {
    format!(
        "divora-replay-{:04}-{:02}-{:02}_{:02}-{:02}-{:02}.mp4",
        t.year, t.month, t.day, t.hour, t.minute, t.second
    )
}

/// `21:04`, as the tray says when a save was made.
#[must_use]
pub fn hh_mm(t: LocalTime) -> String {
    format!("{:02}:{:02}", t.hour, t.minute)
}

/// How long Quit waits for a save in progress before cancelling it: the
/// clip's writing at [`QUIT_SAVE_RATE`], within [`QUIT_WAIT_MIN`] and
/// [`QUIT_WAIT_MAX`], plus the cut's own wait for its audio. `size_bytes`
/// is 0 until the clip is cut.
#[must_use]
pub fn quit_wait(size_bytes: u64) -> Duration {
    let writing = Duration::from_millis(size_bytes / (QUIT_SAVE_RATE / 1000));
    writing.clamp(QUIT_WAIT_MIN, QUIT_WAIT_MAX) + PendingCut::TIMEOUT
}

/// Which icon the tray shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayIconKind {
    /// The app's own.
    Normal,
    /// [`REPLAY_ICON`].
    Replay,
}

/// The last save's outcome, as the tray tooltip reports it — how a save
/// made while Divora is hidden says how it went.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LastSave {
    Saved {
        seconds: u32,
        hh_mm: String,
    },
    /// A short reason ([`short_failure`]).
    Failed(&'static str),
    Saving,
}

impl LastSave {
    /// A clip of `duration_ms` saved at `at`, to the nearest second.
    #[must_use]
    pub fn saved(duration_ms: u64, at: LocalTime) -> Self {
        Self::Saved {
            seconds: u32::try_from(duration_ms.saturating_add(500) / 1000).unwrap_or(u32::MAX),
            hh_mm: hh_mm(at),
        }
    }

    /// A save's outcome, finished at `at`.
    #[must_use]
    pub fn from_outcome(outcome: &Result<SavedClip, String>, at: LocalTime) -> Self {
        match outcome {
            Ok(clip) => Self::saved(clip.duration_ms, at),
            Err(e) => Self::Failed(short_failure(e)),
        }
    }
}

/// A failed save's full words, cut down to what fits the tooltip.
#[must_use]
pub fn short_failure(error: &str) -> &'static str {
    if error.starts_with(NO_SPACE_PREFIX) {
        "not enough disk space"
    } else if error == NOTHING_YET {
        "nothing kept yet"
    } else if error == STILL_SAVING {
        "already saving"
    } else if [REPLAY_OFF, SAVE_DROPPED, STILL_STOPPING].contains(&error) {
        "Replay was turned off"
    } else {
        "couldn't write the file"
    }
}

/// What the tray shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrayView {
    pub tooltip: String,
    /// Show "Save replay" and "Turn Replay off".
    pub replay_items: bool,
    pub icon: TrayIconKind,
}

/// The tray for Replay in `state`, keeping `seconds`, with or without
/// desktop audio, and the last save's outcome. Off, the tray is exactly
/// v1.53.0's; on or held, it says so in its icon, tooltip and menu.
#[must_use]
pub fn tray_view(
    state: ReplayState,
    seconds: u32,
    desktop_audio: bool,
    last: Option<&LastSave>,
) -> TrayView {
    let replay = match state {
        ReplayState::Off => {
            return TrayView {
                tooltip: PRODUCT.to_string(),
                replay_items: false,
                icon: TrayIconKind::Normal,
            };
        }
        ReplayState::Arming | ReplayState::Armed if desktop_audio => {
            format!("Replay on ({seconds} s, with desktop audio)")
        }
        ReplayState::Arming | ReplayState::Armed => format!("Replay on ({seconds} s)"),
        ReplayState::Held(_) => "Replay held".to_string(),
    };
    let last = match last {
        Some(LastSave::Saving) => " · saving…".to_string(),
        Some(LastSave::Saved { seconds, hh_mm }) => format!(" · saved {seconds} s at {hh_mm}"),
        Some(LastSave::Failed(short)) => format!(" · last save failed: {short}"),
        None => String::new(),
    };
    TrayView {
        tooltip: fit_tooltip(format!("{PRODUCT} · {replay}{last}")),
        replay_items: true,
        icon: TrayIconKind::Replay,
    }
}

/// `s` cut to [`TOOLTIP_MAX_UNITS`] UTF-16 units, ending in "…" when cut.
/// Every tooltip Divora makes fits already; this keeps one that someday
/// doesn't from being cut mid-character by tray-icon.
fn fit_tooltip(mut s: String) -> String {
    if s.encode_utf16().count() <= TOOLTIP_MAX_UNITS {
        return s;
    }
    let mut units = 0;
    let mut end = 0;
    for (i, c) in s.char_indices() {
        // Room for the "…" (one unit).
        if units + c.len_utf16() > TOOLTIP_MAX_UNITS - 1 {
            break;
        }
        units += c.len_utf16();
        end = i + c.len_utf8();
    }
    s.truncate(end);
    s.push('…');
    s
}

#[cfg(test)]
mod tests {
    use super::{
        arm_check, cap_megabytes, ceiling_megabytes, desktop_id, different_settings, memory_check,
        over_ceiling, quality_name, quit_wait, rearm_check, replay_filename, replay_memory,
        replay_support, requested_desktop, sanitize_seconds, save_check, short_failure, take_check,
        tray_view, ArmFacts, EngineWatch, HeldReasonDto, LastSave, ReplayArmed, ReplayInput,
        ReplayLength, ReplayMemory, ReplaySaved, ReplayState, ReplayStatus, SharedSettings, Slot,
        SlotAnswer, SlotRequest, TrayIconKind, ALREADY_ON, ENCODER_LOST, NO_DIRECT, NO_HARDWARE,
        PRODUCT, QUIT_SAVING_TOOLTIP, REPLAY_ICON, REPLAY_ICON_SIZE, REPLAY_OFF, RUNG1_TAKE,
        SCREEN_LOST, STILL_SAVING, TOOLTIP_MAX_UNITS, UNAVAILABLE,
    };
    use crate::screen::{Quality, StartRequest, ENGINE_STOPPED};
    use divora_capture::desktop::DesktopSource;
    use divora_capture::pipeline::{NOTHING_YET, SAVE_DROPPED, STILL_STOPPING};
    use divora_capture::replay::budget::LENGTHS;
    use divora_capture::{
        CaptureStatus, EngineSignal, HeldReason, LocalTime, RingStatus, SavedClip,
    };
    use std::time::Duration;

    const ENGINE: ReplayState = ReplayState::Held(HeldReason::EngineStopped);
    const SCREEN: ReplayState = ReplayState::Held(HeldReason::ScreenLost);
    const ENCODER: ReplayState = ReplayState::Held(HeldReason::EncoderFailed);
    const STATES: [ReplayState; 6] = [
        ReplayState::Off,
        ReplayState::Arming,
        ReplayState::Armed,
        ENGINE,
        SCREEN,
        ENCODER,
    ];

    /// §C.7's table, every pair, so a new arm can't slip in unnoticed; and
    /// every request against every slot.
    #[test]
    fn the_replay_table() {
        use ReplayInput::{
            Arm, ArmFailed, Disarm, EncoderFailed, Ended, EngineStarted, EngineStopped, Failed,
            Ready, Rebuilding, ScreenLost,
        };
        use ReplayState::{Armed, Arming, Off};
        let inputs = [
            Arm,
            Ready,
            ArmFailed,
            Disarm,
            EngineStopped,
            EngineStarted,
            ScreenLost,
            EncoderFailed,
            Rebuilding,
            Failed,
            Ended,
        ];
        // Rows in the table's order; columns in `inputs`' order. `no` is the
        // table's "–" (an Arm while Armed included: the caller resizes).
        let no: Option<ReplayState> = None;
        let (off, arming, armed) = (Some(Off), Some(Arming), Some(Armed));
        let (engine, screen, encoder) = (Some(ENGINE), Some(SCREEN), Some(ENCODER));
        let table = [
            (Off, [arming, no, no, no, no, no, no, no, no, no, no]),
            (
                Arming,
                [no, armed, off, off, off, no, off, off, arming, off, off],
            ),
            (
                Armed,
                [
                    no, no, no, off, engine, no, screen, encoder, arming, off, off,
                ],
            ),
            (
                ENGINE,
                [
                    no, no, no, off, no, arming, screen, encoder, engine, off, off,
                ],
            ),
            (SCREEN, [no, no, no, off, no, no, no, no, screen, off, off]),
            (
                ENCODER,
                [no, no, no, off, no, no, no, no, encoder, off, off],
            ),
        ];
        assert_eq!(table.map(|(s, _)| s), STATES);
        for (state, row) in table {
            for (input, want) in inputs.into_iter().zip(row) {
                assert_eq!(state.next(input), want, "{state:?} + {input:?}");
            }
        }
        // The cases that matter in words: a stopped engine can't override a
        // lost screen or encoder, and only a Disarm, an end or a rebuild
        // reaches those holds at all.
        assert_eq!(SCREEN.next(EngineStarted), None);
        assert_eq!(ENCODER.next(EngineStopped), None);
        assert_eq!(ENGINE.next(ScreenLost), screen);

        the_slot_table();
    }

    fn the_slot_table() {
        use SlotAnswer::{Nothing, Send, Spawn, Wait};
        use SlotRequest::{Arm, Disarm, Save, Shutdown, StartTake};
        let requests = [StartTake, Arm, Save, Disarm, Shutdown];
        let none: Slot<u8> = Slot::None;
        let want = [Spawn, Spawn, Nothing, Nothing, Nothing];
        assert_eq!(requests.map(|r| none.answer(r)), want);
        assert_eq!(requests.map(|r| Slot::Running(1).answer(r)), [Send; 5]);
        assert_eq!(requests.map(|r| Slot::Ending(1).answer(r)), [Wait; 5]);

        // Spawned only from None; a second handle comes back untouched.
        let mut slot = Slot::default();
        assert!(slot.is_free());
        assert_eq!(slot.spawned(1), Ok(()));
        assert_eq!(slot.running(), Some(&1));
        assert_eq!(slot.spawned(2), Err(2));
        // Running → Ending, once; nothing new starts while it ends.
        assert!(slot.exiting());
        assert!(!slot.exiting());
        assert_eq!(slot.running(), None);
        assert_eq!(slot.spawned(3), Err(3));
        assert!(requests.iter().all(|&r| slot.answer(r) == Wait));
        // Ending → ended → None: the handle back, the slot free again.
        assert_eq!(slot.ended(), Some(1));
        assert!(slot.is_free());
        assert_eq!(slot.ended(), None);
        assert_eq!(slot.answer(Arm), Spawn);
        // A pipeline that ended by itself frees the slot straight from
        // Running.
        assert_eq!(slot.spawned(4), Ok(()));
        assert_eq!(slot.ended(), Some(4));
        assert!(!slot.exiting());

        // Waited out: the request goes ahead, or is refused in words.
        for a in [Spawn, Send, Nothing] {
            assert_eq!(a.after_wait(), Ok(a));
        }
        assert_eq!(Wait.after_wait(), Err(STILL_STOPPING));
    }

    /// Saving is a flag beside the state: one at a time, Armed or Held.
    #[test]
    fn saves_are_refused_in_words() {
        assert_eq!(save_check(ReplayState::Off, false), Err(REPLAY_OFF));
        // A save that outlived a Disarm: Replay is still off.
        assert_eq!(save_check(ReplayState::Off, true), Err(REPLAY_OFF));
        assert_eq!(save_check(ReplayState::Arming, false), Err(NOTHING_YET));
        for state in [ReplayState::Armed, ENGINE, SCREEN, ENCODER] {
            assert_eq!(save_check(state, false), Ok(()), "{state:?}");
            assert_eq!(save_check(state, true), Err(STILL_SAVING), "{state:?}");
        }
        assert_eq!(save_check(ReplayState::Arming, true), Err(STILL_SAVING));
    }

    fn settings() -> SharedSettings {
        SharedSettings {
            monitor_id: r"\\.\DISPLAY1".into(),
            fps: 30,
            quality: Quality::Balanced,
            cursor: true,
            desktop: Some("everything".into()),
            desktop_gain_db: -6.0,
            av_offset_ms: 0,
        }
    }

    fn start_request(path: &str, stems: bool) -> StartRequest {
        StartRequest {
            monitor_id: r"\\.\DISPLAY1".into(),
            path: path.into(),
            fps: 30,
            quality: Quality::Balanced,
            cursor: true,
            av_offset_ms: 0,
            desktop: Some(DesktopSource::Everything),
            desktop_gain_db: -6.0,
            stems,
        }
    }

    /// `base` with one change.
    fn with(base: &SharedSettings, change: impl FnOnce(&mut SharedSettings)) -> SharedSettings {
        let mut s = base.clone();
        change(&mut s);
        s
    }

    #[test]
    fn a_mismatched_take_is_refused_while_armed() {
        let base = settings();
        let changed = [
            (
                "screen",
                with(&base, |s| s.monitor_id = r"\\.\DISPLAY2".into()),
            ),
            ("frame rate", with(&base, |s| s.fps = 60)),
            ("quality", with(&base, |s| s.quality = Quality::High)),
            ("pointer", with(&base, |s| s.cursor = false)),
            (
                "desktop audio",
                with(&base, |s| s.desktop = Some("device:{x}".into())),
            ),
            ("desktop volume", with(&base, |s| s.desktop_gain_db = -12.0)),
            ("A/V offset", with(&base, |s| s.av_offset_ms = 40)),
        ];
        for (name, other) in &changed {
            assert_eq!(base.mismatch(other), Some(*name));
            assert_eq!(other.mismatch(&base), Some(*name));
        }
        assert_eq!(base.mismatch(&base), None);
        // No desktop audio at all is a different desktop audio...
        let none = with(&base, |s| s.desktop = None);
        assert_eq!(base.mismatch(&none), Some("desktop audio"));
        // ...and without it the volume changes nothing.
        let quieter = with(&none, |s| s.desktop_gain_db = -20.0);
        assert_eq!(none.mismatch(&quieter), None);

        // A take's stems and file name are its own.
        let a = SharedSettings::from(&start_request("a.mp4", false));
        let b = SharedSettings::from(&start_request("b.mp4", true));
        assert_eq!(a, base);
        assert_eq!(a.mismatch(&b), None);
        // The offset as the feed applies it: clamped.
        let mut far = start_request("a.mp4", false);
        far.av_offset_ms = 9_000;
        assert_eq!(SharedSettings::from(&far).av_offset_ms, 500);

        // "None" in any words is none; the ids round-trip.
        assert_eq!(requested_desktop(Some("off")), None);
        assert_eq!(requested_desktop(None), None);
        assert_eq!(
            requested_desktop(Some("everything")).as_deref(),
            Some("everything")
        );
        assert_eq!(
            requested_desktop(Some("device:{a}")).as_deref(),
            Some("device:{a}")
        );
        assert_eq!(
            desktop_id(&DesktopSource::Device("{a}".into())),
            "device:{a}"
        );

        // A take on an armed pipeline, in words.
        let other = &changed[1].1;
        assert_eq!(take_check(ReplayState::Off, Some(&base), other), Ok(()));
        for state in [ReplayState::Arming, ReplayState::Armed] {
            assert_eq!(take_check(state, Some(&base), &base), Ok(()));
            assert_eq!(
                take_check(state, Some(&base), other),
                Err("Replay is on with a different frame rate. A screen recording while Replay is on uses Replay's settings — turn Replay off to record with others.".into())
            );
        }
        assert_eq!(
            different_settings("pointer"),
            "Replay is on with a different pointer. A screen recording while Replay is on uses Replay's settings — turn Replay off to record with others."
        );
        // Held: nothing to share, or the engine to start first.
        assert_eq!(
            take_check(SCREEN, Some(&base), &base),
            Err(SCREEN_LOST.into())
        );
        assert_eq!(
            take_check(ENCODER, Some(&base), &base),
            Err(ENCODER_LOST.into())
        );
        assert_eq!(
            take_check(ENGINE, Some(&base), &base),
            Err(ENGINE_STOPPED.into())
        );
        assert_eq!(
            SCREEN_LOST,
            "Replay lost its screen — turn Replay off to record."
        );
        assert_eq!(
            ENCODER_LOST,
            "Replay lost its encoder — turn Replay off to record."
        );

        // An arm while on: the length alone resizes; anything else is
        // refused.
        assert_eq!(rearm_check(&base, &base), Ok(()));
        assert_eq!(rearm_check(&base, other), Err(ALREADY_ON));
        assert_eq!(
            ALREADY_ON,
            "Replay is already on for another screen or settings — turn it off first."
        );
    }

    const GIB: u64 = 1 << 30;

    fn capture(supported: bool, hardware: bool) -> CaptureStatus {
        CaptureStatus {
            supported,
            reason: (!supported).then(|| "Windows 10 1903 or later is needed.".to_string()),
            h264_encoder: Some("NVIDIA H.264 Encoder MFT".into()),
            hardware,
        }
    }

    /// `arm_check` of `ok` with one change.
    fn arm_with<'a>(
        ok: ArmFacts<'a>,
        change: impl FnOnce(&mut ArmFacts<'a>),
    ) -> Result<(), String> {
        let mut f = ok;
        change(&mut f);
        arm_check(&f)
    }

    #[test]
    fn arming_is_refused_when_it_must_be() {
        let (fine, unsupported, cpu) = (
            capture(true, true),
            capture(false, false),
            capture(true, false),
        );
        let ok = ArmFacts {
            capture: &fine,
            direct_failure: None,
            slot: SlotAnswer::Spawn,
            engine_running: true,
            rung1_take: false,
            seconds: 30,
            bitrate: 14_376_960,
            total_phys: 96 * GIB,
        };
        assert_eq!(arm_check(&ok), Ok(()));
        // On a take's pipeline: sent to it.
        assert_eq!(arm_with(ok, |f| f.slot = SlotAnswer::Send), Ok(()));

        let refused = |why: &str| Err(why.to_string());
        assert_eq!(
            arm_with(ok, |f| f.engine_running = false),
            refused(ENGINE_STOPPED)
        );
        assert_eq!(
            arm_with(ok, |f| f.capture = &unsupported),
            refused("Windows 10 1903 or later is needed.")
        );
        assert_eq!(arm_with(ok, |f| f.capture = &cpu), refused(NO_HARDWARE));
        assert_eq!(
            arm_with(ok, |f| f.direct_failure = Some("DTS != PTS")),
            refused(NO_DIRECT)
        );
        assert_eq!(arm_with(ok, |f| f.rung1_take = true), refused(RUNG1_TAKE));
        // The old pipeline outlived the wait.
        assert_eq!(
            arm_with(ok, |f| f.slot = SlotAnswer::Wait),
            refused(STILL_STOPPING)
        );
        // 4 GB: 120 s at 1440p60 is past an eighth of the RAM. The figures
        // are the labels': the cap rounded up, the ceiling down.
        let at_1440p60 = |seconds| {
            move |f: &mut ArmFacts| {
                (f.seconds, f.bitrate, f.total_phys) = (seconds, 28_753_920, 4 * GIB);
            }
        };
        assert_eq!(
            arm_with(ok, at_1440p60(120)),
            refused("Replay of 120 s at these settings needs up to 578 MB of memory; this PC allows 536 MB.")
        );
        // 60 s at the same settings fits.
        assert_eq!(arm_with(ok, at_1440p60(60)), Ok(()));
        // A resize checks the memory alone, with the same words.
        assert_eq!(memory_check(60, 28_753_920, 4 * GIB), Ok(()));
        assert_eq!(
            memory_check(120, 28_753_920, 4 * GIB),
            arm_with(ok, at_1440p60(120))
        );
        // A PC that can't do it at all hears that first, engine or no.
        assert_eq!(
            arm_with(ok, |f| (f.capture, f.engine_running) = (&cpu, false)),
            refused(NO_HARDWARE)
        );

        // Caps round up, ceilings down — as the frontend's labels do.
        assert_eq!(cap_megabytes(80_044_800), 81);
        assert_eq!(cap_megabytes(577_881_600), 578);
        assert_eq!(cap_megabytes(2_000_000), 2);
        assert_eq!(ceiling_megabytes(536_870_912), 536);
        assert_eq!(ceiling_megabytes(1_999_999), 1);
        assert_eq!(
            over_ceiling(15, 1_000_001, 1_999_999),
            "Replay of 15 s at these settings needs up to 2 MB of memory; this PC allows 1 MB."
        );
    }

    #[test]
    fn support_says_why() {
        let reason = "Windows 10 1903 or later is needed.".to_string();
        // Unsupported: the capture's own reason, never the encoder's (an
        // unsupported status always says hardware = false).
        assert_eq!(
            replay_support(&capture(false, false), None),
            (false, Some(reason.clone()))
        );
        assert_eq!(
            replay_support(&capture(false, false), Some("x")),
            (false, Some(reason))
        );
        let silent = CaptureStatus {
            reason: None,
            ..capture(false, false)
        };
        assert_eq!(
            replay_support(&silent, None),
            (false, Some(UNAVAILABLE.into()))
        );
        assert_eq!(
            replay_support(&capture(true, false), None),
            (false, Some(NO_HARDWARE.into()))
        );
        assert_eq!(
            replay_support(&capture(true, true), Some("re-stamps")),
            (false, Some(NO_DIRECT.into()))
        );
        assert_eq!(replay_support(&capture(true, true), None), (true, None));
        assert_eq!(
            NO_HARDWARE,
            "Replay needs a hardware video encoder; this PC encodes with the CPU."
        );
        assert_eq!(NO_DIRECT, "Replay can't use this PC's video encoder.");
    }

    fn s(secs: f64) -> Duration {
        Duration::from_secs_f64(secs)
    }

    #[test]
    fn a_device_rebuild_does_not_hold_the_ring() {
        use EngineSignal::{Running, Stopped};

        // A rebuild: not running for a moment, no stop counted.
        let mut w = EngineWatch::new(7);
        assert_eq!(w.poll(s(0.0), true, 7, false), Running);
        assert_eq!(w.poll(s(1.0), false, 7, false), Running);
        assert_eq!(w.poll(s(5.9), false, 7, false), Running);
        assert_eq!(w.poll(s(6.0), true, 7, false), Running);

        // The user's Stop: the count moved.
        let mut w = EngineWatch::new(7);
        assert_eq!(w.poll(s(0.0), true, 7, false), Running);
        assert_eq!(w.poll(s(0.02), false, 8, false), Stopped);
        assert_eq!(w.poll(s(3.0), false, 8, false), Stopped);
        assert_eq!(w.poll(s(9.0), true, 8, false), Running);

        // A recovery that gave up (the device in use unplugged).
        let mut w = EngineWatch::new(0);
        assert_eq!(w.poll(s(0.0), false, 0, false), Running);
        assert_eq!(w.poll(s(0.5), false, 0, true), Stopped);
        assert_eq!(w.poll(s(0.7), false, 0, false), Stopped);

        // Stop then Start between two polls: Stopped, then Running, so the
        // ring is still held and cleared.
        let mut w = EngineWatch::new(2);
        assert_eq!(w.poll(s(0.0), true, 2, false), Running);
        assert_eq!(w.poll(s(0.02), true, 3, false), Stopped);
        assert_eq!(w.poll(s(0.04), true, 3, false), Running);

        // The backstop: not running for 5 s with nothing counted.
        let mut w = EngineWatch::new(0);
        assert_eq!(w.poll(s(10.0), false, 0, false), Running);
        assert_eq!(w.poll(s(14.99), false, 0, false), Running);
        assert_eq!(w.poll(s(15.0), false, 0, false), Stopped);
        assert_eq!(w.poll(s(15.02), true, 0, false), Running);
        // Each time down is timed afresh.
        assert_eq!(w.poll(s(16.0), false, 0, false), Running);
        assert_eq!(w.poll(s(20.0), false, 0, false), Running);
        assert_eq!(EngineWatch::BACKSTOP, Duration::from_secs(5));
    }

    /// Sorted object keys of `v`.
    fn keys(v: &impl serde::Serialize) -> Vec<String> {
        let v = serde_json::to_value(v).unwrap();
        let mut keys: Vec<_> = v.as_object().unwrap().keys().cloned().collect();
        keys.sort();
        keys
    }

    /// The wire shapes, frozen: these keys and strings are what the frontend
    /// reads.
    #[test]
    fn replay_wire_keys_are_frozen() {
        let off = ReplayStatus::off(true, None, 30);
        assert_eq!(
            keys(&off),
            [
                "armed",
                "bufferedMs",
                "bytes",
                "capBytes",
                "heldReason",
                "monitorId",
                "reason",
                "saving",
                "seconds",
                "state",
                "supported"
            ]
        );
        // Absent is null, never missing.
        let v = serde_json::to_value(&off).unwrap();
        for k in ["armed", "heldReason", "monitorId", "reason"] {
            assert!(v[k].is_null(), "{k}");
        }
        let armed = ReplayArmed::from(&settings());
        assert_eq!(
            keys(&armed),
            [
                "avOffsetMs",
                "cursor",
                "desktopAudio",
                "desktopGainDb",
                "fps",
                "quality"
            ]
        );
        assert_eq!(
            keys(&ReplaySaved::failed("x")),
            ["durationMs", "error", "path"]
        );
        let memory = ReplayMemory {
            ceiling_bytes: 1,
            lengths: vec![ReplayLength {
                seconds: 30,
                bytes: 1,
                cap_bytes: 1,
                allowed: true,
            }],
        };
        assert_eq!(keys(&memory), ["ceilingBytes", "lengths"]);
        assert_eq!(
            keys(&memory.lengths[0]),
            ["allowed", "bytes", "capBytes", "seconds"]
        );

        for (state, wire) in [
            (ReplayState::Off, "off"),
            (ReplayState::Arming, "arming"),
            (ReplayState::Armed, "armed"),
            (ENGINE, "held"),
            (SCREEN, "held"),
            (ENCODER, "held"),
        ] {
            assert_eq!(serde_json::to_value(state).unwrap(), wire);
        }
        for (why, wire) in [
            (HeldReason::EngineStopped, "engine-stopped"),
            (HeldReason::ScreenLost, "screen-lost"),
            (HeldReason::EncoderFailed, "encoder-failed"),
        ] {
            assert_eq!(
                serde_json::to_value(HeldReasonDto::from(why)).unwrap(),
                wire
            );
        }
        for (q, wire) in [
            (Quality::Small, "small"),
            (Quality::Balanced, "balanced"),
            (Quality::High, "high"),
        ] {
            assert_eq!(quality_name(q), wire);
        }
    }

    /// The constructors put the pure state on the wire.
    #[test]
    fn statuses_carry_the_state() {
        let ring = RingStatus {
            buffered_ms: 12_000,
            bytes: 25_000_000,
            cap_bytes: 80_044_800,
            seconds: 30,
        };
        let held = ReplayStatus::on(SCREEN, ring, &settings());
        let v = serde_json::to_value(&held).unwrap();
        assert_eq!(v["state"], "held");
        assert_eq!(v["heldReason"], "screen-lost");
        assert_eq!(v["monitorId"], r"\\.\DISPLAY1");
        assert_eq!(v["armed"]["desktopAudio"], "everything");
        assert_eq!(v["armed"]["quality"], "balanced");
        assert_eq!(
            (v["seconds"].as_u64(), v["bufferedMs"].as_u64()),
            (Some(30), Some(12_000))
        );
        assert_eq!(v["capBytes"], 80_044_800);
        assert_eq!(v["supported"], true);
        assert!(v["reason"].is_null());

        let armed = ReplayStatus::on(ReplayState::Armed, ring, &settings()).with_saving(true);
        assert!(armed.saving);
        assert_eq!(armed.held_reason, None);
        assert_eq!(
            ReplayStatus::on(ReplayState::Off, ring, &settings()),
            ReplayStatus::off(true, None, 30)
        );
        let off = ReplayStatus::off(false, Some(NO_HARDWARE.into()), 60);
        assert_eq!(
            (off.state, off.seconds, off.saving),
            (ReplayState::Off, 60, false)
        );

        let clip = SavedClip {
            path: r"C:\rec\divora-replay-2026-10-16_21-04-05.mp4".into(),
            duration_ms: 31_000,
            bytes: 1,
        };
        assert_eq!(
            ReplaySaved::from_outcome(&Ok(clip)),
            ReplaySaved {
                path: Some(r"C:\rec\divora-replay-2026-10-16_21-04-05.mp4".into()),
                duration_ms: 31_000,
                error: None,
            }
        );
        assert_eq!(
            ReplaySaved::from_outcome(&Err(REPLAY_OFF.into())),
            ReplaySaved {
                path: None,
                duration_ms: 0,
                error: Some(REPLAY_OFF.into()),
            }
        );
    }

    /// §B.9's table through `Quality::bitrate`: (monitor, fps, quality, the
    /// bits/s it gives, and per length the estimate and cap in tenths of an
    /// MB).
    #[test]
    fn replay_memory_table_is_frozen() {
        type Row = ((u32, u32), u32, Quality, u32, [(u64, u64); 4]);
        let rows: [Row; 8] = [
            (
                (1920, 1080),
                30,
                Quality::Balanced,
                8_087_040,
                [(208, 260), (389, 486), (750, 937), (1471, 1839)],
            ),
            (
                (1920, 1080),
                60,
                Quality::Balanced,
                16_174_080,
                [(380, 475), (712, 890), (1376, 1720), (2705, 3381)],
            ),
            (
                (2560, 1440),
                30,
                Quality::Balanced,
                14_376_960,
                [(342, 427), (640, 800), (1237, 1546), (2431, 3038)],
            ),
            (
                (2560, 1440),
                60,
                Quality::Balanced,
                28_753_920,
                [(648, 809), (1215, 1519), (2351, 2939), (4623, 5779)],
            ),
            (
                (3440, 1440),
                30,
                Quality::Balanced,
                10_682_880,
                [(263, 329), (493, 616), (951, 1189), (1867, 2334)],
            ),
            (
                (3440, 1440),
                60,
                Quality::Balanced,
                21_365_760,
                [(491, 613), (920, 1150), (1779, 2223), (3496, 4370)],
            ),
            (
                (2560, 1440),
                60,
                Quality::High,
                46_006_272,
                [(1014, 1268), (1906, 2382), (3688, 4610), (7254, 9068)],
            ),
            (
                (2560, 1440),
                30,
                Quality::Small,
                8_626_176,
                [(220, 275), (410, 513), (791, 989), (1554, 1942)],
            ),
        ];
        let tenths = |bytes: u64| (bytes + 50_000) / 100_000;
        for (monitor, fps, quality, bitrate, want) in rows {
            assert_eq!(
                quality.bitrate(monitor, fps),
                bitrate,
                "{monitor:?} {fps} {quality:?}"
            );
            let m = replay_memory(monitor, fps, quality, 96 * GIB);
            assert_eq!(m.ceiling_bytes, GIB);
            assert_eq!(
                m.lengths.iter().map(|l| l.seconds).collect::<Vec<_>>(),
                LENGTHS
            );
            for (l, (estimate, cap)) in m.lengths.iter().zip(want) {
                assert_eq!(
                    (tenths(l.bytes), tenths(l.cap_bytes)),
                    (estimate, cap),
                    "{monitor:?} {fps} {quality:?} {} s",
                    l.seconds
                );
                assert!(l.allowed);
            }
        }
        // 4K is encoded at 2560×1440: the same figures.
        assert_eq!(Quality::High.bitrate((3840, 2160), 60), 46_006_272);

        // 4 GB: the two 120 s lengths over 536 MB are refused, and their
        // labels read above it.
        let m = replay_memory((2560, 1440), 60, Quality::High, 4 * GIB);
        assert_eq!(m.ceiling_bytes, 536_870_912);
        let allowed: Vec<_> = m.lengths.iter().map(|l| l.allowed).collect();
        assert_eq!(allowed, [true, true, true, false]);
        for l in &m.lengths {
            assert_eq!(
                l.allowed,
                cap_megabytes(l.cap_bytes) <= ceiling_megabytes(m.ceiling_bytes)
            );
        }
    }

    #[test]
    fn seconds_and_names() {
        assert_eq!(sanitize_seconds(None), 30);
        for s in LENGTHS {
            assert_eq!(sanitize_seconds(Some(s)), s);
        }
        for s in [0, 1, 29, 45, 90, 240, u32::MAX] {
            assert_eq!(sanitize_seconds(Some(s)), 30, "{s}");
        }

        let t = |year, month, day, hour, minute, second| LocalTime {
            year,
            month,
            day,
            hour,
            minute,
            second,
        };
        assert_eq!(
            replay_filename(t(2026, 10, 16, 21, 4, 5)),
            "divora-replay-2026-10-16_21-04-05.mp4"
        );
        assert_eq!(
            replay_filename(t(2027, 1, 2, 3, 4, 5)),
            "divora-replay-2027-01-02_03-04-05.mp4"
        );
        // In time order, the names sort the same way.
        let times = [
            t(2026, 9, 30, 23, 59, 59),
            t(2026, 10, 1, 0, 0, 0),
            t(2026, 10, 1, 0, 0, 9),
            t(2026, 10, 1, 0, 0, 10),
            t(2026, 10, 1, 9, 0, 0),
            t(2026, 10, 1, 10, 0, 0),
            t(2026, 12, 9, 0, 0, 0),
            t(2027, 1, 1, 0, 0, 0),
        ];
        let names: Vec<_> = times.iter().map(|&x| replay_filename(x)).collect();
        let mut sorted = names.clone();
        sorted.sort();
        assert_eq!(names, sorted);
    }

    /// Every state × desktop audio on and off × every last save.
    #[test]
    fn tray_view_for_each_state() {
        let mut lasts = vec![
            None,
            Some(LastSave::Saving),
            Some(LastSave::saved(
                31_400,
                LocalTime {
                    hour: 21,
                    minute: 4,
                    ..LocalTime::default()
                },
            )),
            Some(LastSave::Saved {
                seconds: u32::MAX,
                hh_mm: "23:59".into(),
            }),
        ];
        for short in [
            "not enough disk space",
            "nothing kept yet",
            "already saving",
            "Replay was turned off",
            "couldn't write the file",
        ] {
            lasts.push(Some(LastSave::Failed(short)));
        }
        for state in STATES {
            for desktop in [false, true] {
                for seconds in LENGTHS {
                    for last in &lasts {
                        let v = tray_view(state, seconds, desktop, last.as_ref());
                        let at = format!("{state:?} {desktop} {seconds} {last:?}");
                        assert!(
                            v.tooltip.encode_utf16().count() <= TOOLTIP_MAX_UNITS,
                            "{at}"
                        );
                        assert_eq!(v.icon == TrayIconKind::Replay, state.is_on(), "{at}");
                        assert_eq!(v.replay_items, state.is_on(), "{at}");
                        if state == ReplayState::Off {
                            assert_eq!(v.tooltip, PRODUCT, "{at}");
                        }
                        // Nothing Divora makes needs cutting to fit.
                        let cut = v.tooltip.ends_with('…') && last != &Some(LastSave::Saving);
                        assert!(!cut, "{at}");
                    }
                }
            }
        }
        let view = |state, desktop, last: Option<LastSave>| {
            tray_view(state, 30, desktop, last.as_ref()).tooltip
        };
        assert_eq!(PRODUCT, "DivoraVoice");
        assert_eq!(
            view(ReplayState::Off, true, Some(LastSave::Saving)),
            "DivoraVoice"
        );
        assert_eq!(
            view(ReplayState::Arming, false, None),
            "DivoraVoice · Replay on (30 s)"
        );
        assert_eq!(
            view(ReplayState::Armed, true, None),
            "DivoraVoice · Replay on (30 s, with desktop audio)"
        );
        assert_eq!(view(ENGINE, true, None), "DivoraVoice · Replay held");
        assert_eq!(
            view(ReplayState::Armed, false, Some(LastSave::Saving)),
            "DivoraVoice · Replay on (30 s) · saving…"
        );
        assert_eq!(
            view(SCREEN, false, lasts[2].clone()),
            "DivoraVoice · Replay held · saved 31 s at 21:04"
        );
        assert_eq!(
            view(
                ReplayState::Armed,
                false,
                Some(LastSave::Failed("not enough disk space"))
            ),
            "DivoraVoice · Replay on (30 s) · last save failed: not enough disk space"
        );
        // The longest Divora makes.
        let longest = tray_view(
            ReplayState::Armed,
            120,
            true,
            Some(&LastSave::Failed("couldn't write the file")),
        );
        assert_eq!(
            longest.tooltip,
            "DivoraVoice · Replay on (120 s, with desktop audio) · last save failed: couldn't write the file"
        );
        assert_eq!(longest.tooltip.encode_utf16().count(), 95);
        assert!(QUIT_SAVING_TOOLTIP.encode_utf16().count() <= TOOLTIP_MAX_UNITS);
        // One that someday doesn't fit is cut, whole characters only.
        let long = LastSave::Saved {
            seconds: 1,
            hh_mm: "é".repeat(200),
        };
        let cut = tray_view(ReplayState::Armed, 30, true, Some(&long)).tooltip;
        assert_eq!(cut.encode_utf16().count(), TOOLTIP_MAX_UNITS);
        assert!(cut.ends_with("é…"));
    }

    /// Each failure's full words map to the tooltip's short reason.
    #[test]
    fn failures_are_short_in_the_tray() {
        for (error, short) in [
            (
                "There isn't enough free space on the drive to save the replay (it needs about 81 MB).",
                "not enough disk space",
            ),
            (NOTHING_YET, "nothing kept yet"),
            (STILL_SAVING, "already saving"),
            (REPLAY_OFF, "Replay was turned off"),
            (SAVE_DROPPED, "Replay was turned off"),
            (STILL_STOPPING, "Replay was turned off"),
            ("MF_E_SINK_NO_STREAMS (0xC00D4A3F)", "couldn't write the file"),
        ] {
            assert_eq!(short_failure(error), short, "{error}");
        }
        let at = LocalTime {
            hour: 9,
            minute: 5,
            ..LocalTime::default()
        };
        assert_eq!(
            LastSave::from_outcome(&Err(NOTHING_YET.into()), at),
            LastSave::Failed("nothing kept yet")
        );
        let clip = SavedClip {
            path: "a.mp4".into(),
            duration_ms: 12_499,
            bytes: 1,
        };
        assert_eq!(
            LastSave::from_outcome(&Ok(clip), at),
            LastSave::Saved {
                seconds: 12,
                hh_mm: "09:05".into()
            }
        );
    }

    #[test]
    fn quit_waits_by_size() {
        let cut = Duration::from_millis(1_200);
        assert_eq!(quit_wait(0), Duration::from_secs(5) + cut);
        // The default clip (≈ 64 MB) still gets the least.
        assert_eq!(quit_wait(64_035_840), Duration::from_secs(5) + cut);
        // The largest (120 s at 1440p60 High, ≈ 725 MB): about 36 s.
        assert_eq!(quit_wait(725_403_648), Duration::from_millis(36_270) + cut);
        assert_eq!(quit_wait(1_200_000_000), Duration::from_secs(60) + cut);
        assert_eq!(quit_wait(u64::MAX), Duration::from_secs(60) + cut);
    }

    /// The committed tray icon: 32×32 RGBA, with the red dot in the lower
    /// right and its light ring.
    #[test]
    fn the_replay_icon_has_its_dot() {
        let size = usize::try_from(REPLAY_ICON_SIZE).unwrap();
        assert_eq!(REPLAY_ICON.len(), 4096);
        assert_eq!(REPLAY_ICON.len(), size * size * 4);
        let px = |x: usize, y: usize| {
            let i = (y * size + x) * 4;
            (
                REPLAY_ICON[i],
                REPLAY_ICON[i + 1],
                REPLAY_ICON[i + 2],
                REPLAY_ICON[i + 3],
            )
        };
        // The dot's middle (centre 24.5, 24.5): opaque and red.
        for y in 22..27 {
            for x in 22..27 {
                let (r, g, b, a) = px(x, y);
                assert_eq!(a, 255, "({x}, {y})");
                assert!(
                    u16::from(r) > 2 * u16::from(g) && u16::from(r) > 2 * u16::from(b),
                    "({x}, {y})"
                );
            }
        }
        // The ring, between 6 and 7.5 px out: light, so the dot reads on a
        // dark taskbar.
        for (x, y) in [(31, 24), (24, 31), (17, 24), (24, 17)] {
            let (r, g, b, a) = px(x, y);
            assert_eq!(a, 255, "({x}, {y})");
            assert!(r > 200 && g > 200 && b > 200, "({x}, {y})");
        }
        // The top left is the app icon's, not red.
        let (r, g, b, _) = px(2, 2);
        assert!(!(u16::from(r) > 2 * u16::from(g) && u16::from(r) > 2 * u16::from(b)));
    }
}
