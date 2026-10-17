//! v1.54.0: Replay's side of the screen recorder — turning the ring on and
//! off, saving from it, and hearing from the pipeline — applying the rules
//! of [`crate::replay`].
//!
//! Replay and takes share the one pipeline in [`super::Shared`]'s slot: an
//! arm with no pipeline starts one, an arm during a take on a pipeline joins
//! it (adopting its settings), and the pipeline ends when neither needs it.
//! Every Save ends in exactly one `replay-saved` event, whatever happens to
//! it on the way.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{channel, Sender};
use std::sync::{Arc, PoisonError};
use std::time::{Duration, Instant};

use divora_capture::pipeline::SAVE_DROPPED;
use divora_capture::{
    HeldReason, PipelineCommand, PipelineEvents, RingStatus, SaveReply, SaveRequest, SavedClip,
    StartError,
};
use divora_core::audio::AudioEngine;
use tauri::{AppHandle, Emitter};

use super::{pipeline_settings, Phase, ScreenRecorder, Shared, StartRequest};
use crate::replay::{
    arm_check, memory_check, rearm_check, replay_filename, replay_support, ring_cap, save_check,
    short_failure, tray_view, ArmFacts, LastSave, ReplayInput, ReplaySaved, ReplayState,
    ReplayStatus, SharedSettings, SlotRequest, DEFAULT_SECONDS, EVENT_BUFFER, EVENT_SAVED,
    NO_DIRECT, QUIT_CANCEL_WAIT, QUIT_SAVING_TOOLTIP, REPLAY_OFF,
};
use divora_capture::sizing::DEFAULT_MAX;
use divora_capture::timeline::TrackOptions;
use divora_capture::RecordOptions;

/// How long `save_replay` waits for its answer: the longest a save may
/// write, and its cut.
const SAVE_WAIT: Duration = Duration::from_secs(90);

/// How long Quit waits for the pipeline to end after turning Replay off.
const QUIT_JOIN: Duration = Duration::from_secs(2);

/// Replay, as the recorder keeps it.
#[derive(Debug)]
pub(super) struct ReplayShared {
    pub(super) state: ReplayState,
    /// The length kept, or the last one asked for.
    pub(super) seconds: u32,
    /// The ring's latest report.
    pub(super) ring: RingStatus,
    pub(super) saving: bool,
    /// The last save's outcome, for the tray.
    pub(super) last_save: Option<LastSave>,
    /// The save in progress: its size once cut, and Quit's way to stop it.
    pub(super) save_size: Option<Arc<AtomicU64>>,
    pub(super) save_cancel: Option<Arc<AtomicBool>>,
}

impl Default for ReplayShared {
    fn default() -> Self {
        Self {
            state: ReplayState::Off,
            seconds: DEFAULT_SECONDS,
            ring: RingStatus::default(),
            saving: false,
            last_save: None,
            save_size: None,
            save_cancel: None,
        }
    }
}

/// What `arm_replay_buffer` asks for: the screen settings (as a take would
/// use them) and the length.
#[derive(Debug, Clone)]
pub struct ArmRequest {
    pub screen: StartRequest,
    pub seconds: u32,
}

impl ScreenRecorder {
    /// Replay now, for `replay_buffer_status`.
    pub fn replay_status(&self) -> ReplayStatus {
        status(&self.lock())
    }

    /// Turn Replay on, or change its length. Starts a pipeline if none runs
    /// (waiting for its first frame), joins a take's if one does (its
    /// settings win), resizes the ring if Replay is already on with the same
    /// settings. `Err` says why not, in the user's words.
    #[allow(clippy::too_many_lines)] // resize, join a take, or start a pipeline
    pub fn arm_replay(
        self: &Arc<Self>,
        app: &AppHandle,
        engine: &Arc<AudioEngine>,
        req: ArmRequest,
    ) -> Result<ReplayStatus, String> {
        let capture = divora_capture::capture_status();
        let asked = SharedSettings::from(&req.screen);
        let seconds = req.seconds;
        let mut s = self.wait_slot().map_err(str::to_string)?;
        if s.replay.state.is_on() {
            let Some(armed) = s.settings.clone() else {
                return Err(super::STILL_STOPPING.to_string());
            };
            rearm_check(&armed, &asked).map_err(str::to_string)?;
            let bitrate = bitrate(&armed)?;
            memory_check(seconds, bitrate, divora_capture::total_physical_memory())?;
            let cap_bytes = ring_cap(seconds, bitrate);
            if let Some(h) = s.slot.handle() {
                let _ = h.send(PipelineCommand::Resize { seconds, cap_bytes });
            }
            s.replay.seconds = seconds;
            s.replay.ring.seconds = seconds;
            s.replay.ring.cap_bytes = cap_bytes;
            return Ok(publish(app, &s));
        }
        // A take running on a pipeline: Replay joins it, on its settings.
        let adopt = s.slot.running().is_some();
        let settings = if adopt {
            s.settings.clone().unwrap_or(asked)
        } else {
            asked
        };
        let bitrate = bitrate(&settings)?;
        arm_check(&ArmFacts {
            capture: &capture,
            direct_failure: s.direct_failure.as_deref(),
            slot: s.slot.answer(SlotRequest::Arm),
            engine_running: engine.is_running(),
            rung1_take: s.phase != Phase::Idle && !s.pipeline_take,
            seconds,
            bitrate,
            total_phys: divora_capture::total_physical_memory(),
        })?;
        let cap_bytes = ring_cap(seconds, bitrate);
        s.replay.state = ReplayState::Arming;
        s.replay.seconds = seconds;
        s.replay.ring = RingStatus {
            seconds,
            cap_bytes,
            ..RingStatus::default()
        };
        if adopt {
            if let Some(h) = s.slot.running() {
                let _ = h.send(PipelineCommand::Arm { seconds, cap_bytes });
            }
            tracing::info!(seconds, "replay on, joining the take's pipeline");
            return Ok(publish(app, &s));
        }
        s.spawning = true;
        drop(s);

        let mut screen = req.screen;
        screen.stems = false;
        let started =
            self.open_feed(app, engine, &screen, false)
                .and_then(|(feed, desk, opened)| {
                    let opts = RecordOptions {
                        fps: screen.fps,
                        max_size: DEFAULT_MAX,
                        bitrate: Some(bitrate),
                        cursor: screen.cursor,
                        audio: TrackOptions {
                            desktop_gain_db: opened.then_some(screen.desktop_gain_db),
                            stems: false,
                        },
                    };
                    let feed = Box::new(feed.watch_engine(Arc::clone(engine)));
                    divora_capture::spawn_pipeline(
                        pipeline_settings(&screen.monitor_id, &opts),
                        feed,
                        self.pipeline_events(app, engine),
                        PipelineCommand::Arm { seconds, cap_bytes },
                    )
                    .map(|h| (h, desk))
                    .map_err(|e| {
                        engine.detach_av_tap();
                        match e {
                            StartError::Direct { why, .. } => {
                                tracing::warn!(why, "replay can't use the direct encoder");
                                self.lock().direct_failure = Some(why);
                                NO_DIRECT.to_string()
                            }
                            StartError::Other(e) => e.to_string(),
                        }
                    })
                });
        match started {
            Ok((h, desk)) => {
                self.spawned(Some((h, settings, desk, None)));
                tracing::info!(seconds, cap_bytes, "replay on");
                Ok(publish(app, &self.lock()))
            }
            Err(why) => {
                self.spawned(None);
                let mut s = self.lock();
                if let Some(next) = s.replay.state.next(ReplayInput::ArmFailed) {
                    s.replay.state = next;
                }
                publish(app, &s);
                Err(why)
            }
        }
    }

    /// Turn Replay off. A Save already pressed is still cut and saved; the
    /// pipeline ends with it unless a take is still on it.
    pub fn disarm_replay(&self, app: &AppHandle) -> ReplayStatus {
        let mut s = self.lock();
        if !s.replay.state.is_on() {
            return status(&s);
        }
        if let Some(h) = s.slot.handle() {
            let _ = h.send(PipelineCommand::Disarm);
        }
        if !s.pipeline_take {
            s.slot.exiting();
        }
        if let Some(next) = s.replay.state.next(ReplayInput::Disarm) {
            s.replay.state = next;
        }
        tracing::info!("replay off");
        publish(app, &s)
    }

    /// Save the last `seconds` up to `at` (the press) into `dir`. Every
    /// call ends in exactly one `replay-saved` event; this also waits for
    /// it and returns the same outcome.
    pub fn save_replay(
        self: &Arc<Self>,
        app: &AppHandle,
        dir: PathBuf,
        at: Instant,
    ) -> Result<SavedClip, String> {
        let (tx, rx) = channel::<Result<SavedClip, String>>();
        let mut s = self.lock();
        let refused = save_check(s.replay.state, s.replay.saving)
            .err()
            .or_else(|| s.slot.handle().is_none().then_some(REPLAY_OFF));
        if let Some(why) = refused {
            if s.replay.state.is_on() {
                s.replay.last_save = Some(LastSave::Failed(short_failure(why)));
            }
            let _ = app.emit(EVENT_SAVED, ReplaySaved::failed(why));
            crate::sync_tray(app, tray(&s));
            return Err(why.to_string());
        }
        let size = Arc::new(AtomicU64::new(0));
        let cancel = Arc::new(AtomicBool::new(false));
        s.replay.saving = true;
        s.replay.last_save = Some(LastSave::Saving);
        s.replay.save_size = Some(Arc::clone(&size));
        s.replay.save_cancel = Some(Arc::clone(&cancel));
        let me = Arc::clone(self);
        let to = app.clone();
        let req = SaveRequest {
            at,
            dir,
            name: replay_filename(divora_capture::local_time()),
            reply: SaveReply::new(move |r| me.saved(&to, r, &tx)),
            size,
            cancel,
        };
        let sent = s.slot.handle().map(|h| h.send(PipelineCommand::Save(req)));
        publish(app, &s);
        drop(s);
        // A Save the pipeline hands back is dropped here, with the lock
        // free: its reply answers that it wasn't saved.
        drop(sent);
        rx.recv_timeout(SAVE_WAIT)
            .unwrap_or_else(|_| Err(SAVE_DROPPED.to_string()))
    }

    /// A save's answer: `saving` clears, the event goes out, the tray says
    /// how it went, and whoever waits hears.
    fn saved(
        &self,
        app: &AppHandle,
        outcome: Result<SavedClip, String>,
        tx: &Sender<Result<SavedClip, String>>,
    ) {
        let mut s = self.lock();
        s.replay.saving = false;
        s.replay.save_size = None;
        s.replay.save_cancel = None;
        s.replay.last_save = Some(LastSave::from_outcome(
            &outcome,
            divora_capture::local_time(),
        ));
        let _ = app.emit(EVENT_SAVED, ReplaySaved::from_outcome(&outcome));
        publish(app, &s);
        drop(s);
        self.changed.notify_all();
        let _ = tx.send(outcome);
    }

    /// Quit: a save in progress is waited for (as long as its size needs,
    /// then cancelled), then Replay is turned off and its pipeline given a
    /// moment to end. Runs on the main thread, after any take was stopped.
    pub fn quit_replay(&self, app: &AppHandle) {
        let pressed = Instant::now();
        let mut s = self.lock();
        if s.replay.saving {
            if let Some(tray) = app.tray_by_id(crate::TRAY_ID) {
                let _ = tray.set_tooltip(Some(QUIT_SAVING_TOOLTIP));
            }
        }
        // The clip's size is known once it is cut, which may be after the
        // press: the limit is worked out again as the wait goes, from then.
        let mut sized_at: Option<Instant> = None;
        while s.replay.saving {
            let size = s
                .replay
                .save_size
                .as_ref()
                .map_or(0, |n| n.load(Ordering::Acquire));
            if size > 0 && sized_at.is_none() {
                sized_at = Some(Instant::now());
            }
            let deadline = sized_at.unwrap_or(pressed) + crate::replay::quit_wait(size);
            let Some(left) = deadline.checked_duration_since(Instant::now()) else {
                tracing::warn!(size, "quitting during a slow save; cancelling it");
                if let Some(c) = &s.replay.save_cancel {
                    c.store(true, Ordering::Release);
                }
                // A cancelled save stops within a few writes and removes its
                // file; one already finishing its file flushes its queue
                // first (see QUIT_CANCEL_WAIT). Either way it answers.
                s = self
                    .changed
                    .wait_timeout_while(s, QUIT_CANCEL_WAIT, |s| s.replay.saving)
                    .unwrap_or_else(PoisonError::into_inner)
                    .0;
                break;
            };
            s = self
                .changed
                .wait_timeout(s, left.min(Duration::from_millis(200)))
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
        if s.replay.state.is_on() {
            drop(s);
            self.disarm_replay(app);
        } else {
            drop(s);
        }
        let s = self.lock();
        let _ = self
            .changed
            .wait_timeout_while(s, QUIT_JOIN, |s| !s.slot.is_free())
            .unwrap_or_else(PoisonError::into_inner);
    }

    /// The pipeline's events, for its thread.
    pub(super) fn pipeline_events(
        self: &Arc<Self>,
        app: &AppHandle,
        engine: &Arc<AudioEngine>,
    ) -> Arc<dyn PipelineEvents> {
        Arc::new(AppPipelineEvents {
            recorder: Arc::clone(self),
            app: app.clone(),
            engine: Arc::clone(engine),
        })
    }

    /// Replay moved (or didn't): the state, the event and the tray.
    fn replay_input(&self, app: &AppHandle, input: ReplayInput) {
        let mut s = self.lock();
        let Some(next) = s.replay.state.next(input) else {
            return;
        };
        if next == s.replay.state && input != ReplayInput::Rebuilding {
            return;
        }
        s.replay.state = next;
        if input == ReplayInput::Rebuilding || input == ReplayInput::EngineStarted {
            s.replay.ring.buffered_ms = 0;
        }
        publish(app, &s);
    }

    /// The ring's once-a-second report: armed, it goes out each time; held,
    /// only its transitions do.
    fn ring_report(&self, app: &AppHandle, ring: RingStatus) {
        let mut s = self.lock();
        s.replay.ring = ring;
        if s.replay.state == ReplayState::Armed {
            let _ = app.emit(EVENT_BUFFER, status(&s));
        }
    }

    /// The pipeline's thread is gone; the engine's tap was let go first, so
    /// a new pipeline's tap can only come after. One that ended while its
    /// start was still going on is never put in the slot.
    fn pipeline_ended(&self, app: &AppHandle) {
        let mut s = self.lock();
        if s.spawning && s.slot.is_free() {
            s.spawn_ended = true;
        } else {
            let _ = s.slot.ended();
            s.settings = None;
            s.desktop = None;
        }
        if let Some(next) = s.replay.state.next(ReplayInput::Ended) {
            s.replay.state = next;
        }
        publish(app, &s);
        drop(s);
        self.changed.notify_all();
    }

    /// Replay's status out now (the card shows "can't use this PC's
    /// encoder" as soon as that is known).
    pub(super) fn publish_replay(&self, app: &AppHandle) {
        publish(app, &self.lock());
    }
}

/// Replay's status out as an event, and the tray to match — under the
/// recorder's lock, so they go out in the order the state changed. Neither
/// waits: the event is queued, the tray posted to the main thread.
fn publish(app: &AppHandle, s: &Shared) -> ReplayStatus {
    let st = status(s);
    let _ = app.emit(EVENT_BUFFER, &st);
    crate::sync_tray(app, tray(s));
    st
}

/// Replay's status from what the recorder holds.
fn status(s: &Shared) -> ReplayStatus {
    let st = match (&s.settings, s.replay.state) {
        (Some(settings), state) if state.is_on() => {
            ReplayStatus::on(state, s.replay.ring, settings)
        }
        _ => {
            let (supported, reason) = replay_support(
                &divora_capture::capture_status(),
                s.direct_failure.as_deref(),
            );
            ReplayStatus::off(supported, reason, s.replay.seconds)
        }
    };
    // Arming before its pipeline is known still says Arming.
    let st = if s.replay.state.is_on() && s.settings.is_none() {
        ReplayStatus {
            state: s.replay.state,
            seconds: s.replay.seconds,
            cap_bytes: s.replay.ring.cap_bytes,
            ..st
        }
    } else {
        st
    };
    st.with_saving(s.replay.saving)
}

/// The tray for what the recorder holds.
fn tray(s: &Shared) -> crate::replay::TrayView {
    let desktop = s.settings.as_ref().is_some_and(|x| x.desktop.is_some());
    tray_view(
        s.replay.state,
        s.replay.seconds,
        desktop,
        s.replay.last_save.as_ref(),
    )
}

/// The bitrate a pipeline with `settings` encodes at: a take's, from its
/// screen's size.
fn bitrate(settings: &SharedSettings) -> Result<u32, String> {
    divora_capture::list_monitors()
        .into_iter()
        .find(|m| m.id == settings.monitor_id)
        .map(|m| settings.quality.bitrate((m.width, m.height), settings.fps))
        .ok_or_else(|| "That screen isn't connected any more — pick another.".to_string())
}

/// What a pipeline tells the app, from its thread.
struct AppPipelineEvents {
    recorder: Arc<ScreenRecorder>,
    app: AppHandle,
    engine: Arc<AudioEngine>,
}

impl PipelineEvents for AppPipelineEvents {
    fn ring_ready(&self) {
        self.recorder.replay_input(&self.app, ReplayInput::Ready);
    }

    fn ring(&self, status: RingStatus) {
        self.recorder.ring_report(&self.app, status);
    }

    fn held(&self, reason: HeldReason, detail: Option<String>) {
        tracing::info!(?reason, detail, "replay held");
        let input = match reason {
            HeldReason::EngineStopped => ReplayInput::EngineStopped,
            HeldReason::ScreenLost => ReplayInput::ScreenLost,
            HeldReason::EncoderFailed => ReplayInput::EncoderFailed,
        };
        self.recorder.replay_input(&self.app, input);
    }

    fn resumed(&self) {
        self.recorder
            .replay_input(&self.app, ReplayInput::EngineStarted);
    }

    fn rebuilding(&self) {
        self.recorder
            .replay_input(&self.app, ReplayInput::Rebuilding);
    }

    fn arm_refused(&self, why: String) {
        tracing::info!(why, "replay's arm reached a pipeline on its way out");
        self.recorder
            .replay_input(&self.app, ReplayInput::ArmFailed);
    }

    fn direct_failed(&self, why: String) {
        tracing::warn!(why, "the direct encoder can't be trusted on this PC");
        let mut s = self.recorder.lock();
        s.direct_failure.get_or_insert(why);
        publish(&self.app, &s);
    }

    fn failed(&self, why: String) {
        tracing::warn!(why, "replay turned itself off");
        self.recorder.replay_input(&self.app, ReplayInput::Failed);
    }

    fn ended(&self, error: Option<String>) {
        if let Some(e) = &error {
            tracing::warn!(error = e, "the capture pipeline ended");
        }
        // The tap first: a new pipeline attaches only once the slot is free,
        // and so after this.
        self.engine.detach_av_tap();
        self.recorder.pipeline_ended(&self.app);
    }
}
