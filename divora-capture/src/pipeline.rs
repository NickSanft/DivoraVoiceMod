//! The capture pipeline's API (v1.54.0): what the app asks of it and what it
//! hears back.
//!
//! A pipeline is one capture, one encoder and one audio timeline on a thread
//! of its own (`divora-encoder`), shared by whatever needs them: a take, the
//! replay ring, or both. The app sends it [`PipelineCommand`]s through a
//! [`PipelineHandle`] and is told what happens through [`PipelineEvents`] and
//! each take's [`TakeEvents`]. The decisions it makes are in
//! [`crate::pipeline_core`]; the Windows side is `spawn_pipeline`.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64};
use std::sync::mpsc::Sender;
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crate::error::CaptureError;
use crate::take::{AudioFeed, RecordStats};
use crate::timeline::TrackOptions;

/// What a [`SaveReply`] nobody answered says: the cut it belonged to was
/// cleared away, or the pipeline had already gone.
pub const SAVE_DROPPED: &str =
    "The replay wasn't saved: Replay was turned off or the PC slept before it was ready.";

/// A Save before the ring holds a keyframe.
pub const NOTHING_YET: &str = "Replay hasn't kept anything yet — try again in a moment.";

/// A request that reached a pipeline on its way out.
pub const STILL_STOPPING: &str = "Still stopping the last recording — try again.";

/// A Save with Replay off.
pub const REPLAY_OFF: &str = "Replay is off.";

/// A Save while the last one is still being cut or written.
pub const STILL_SAVING: &str = "Still saving the last replay.";

/// A take stopped before its first frame: there is no file.
pub const NO_PICTURE: &str = "Stopped before the screen sent a picture.";

/// What a pipeline is built with. Everything a take and the ring share: a
/// take or an arm asking for something else is refused by the app.
#[derive(Debug, Clone)]
pub struct PipelineSettings {
    pub monitor_id: String,
    /// 30 or 60: a whole number of audio samples per frame.
    pub fps: u32,
    /// The largest encoded size ([`crate::sizing::DEFAULT_MAX`]).
    pub max_size: (u32, u32),
    /// Bits per second.
    pub bitrate: u32,
    pub cursor: bool,
    /// Desktop audio and its gain. `stems` is ignored: the pipeline's
    /// timeline always makes them, and each take decides.
    pub track: TrackOptions,
}

/// What the app asks of a running pipeline.
pub enum PipelineCommand {
    StartTake(TakeRequest),
    StopTake {
        take: u64,
    },
    /// Turn the ring on, keeping clips of `seconds` within `cap_bytes`.
    Arm {
        seconds: u32,
        cap_bytes: u64,
    },
    /// The same ring, another length.
    Resize {
        seconds: u32,
        cap_bytes: u64,
    },
    Disarm,
    Save(SaveRequest),
    /// End: a take still open is stopped first, so its file is finished.
    Shutdown,
}

impl std::fmt::Debug for PipelineCommand {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::StartTake(t) => write!(f, "StartTake({})", t.take),
            Self::StopTake { take } => write!(f, "StopTake({take})"),
            Self::Arm { seconds, cap_bytes } => write!(f, "Arm({seconds} s, {cap_bytes} B)"),
            Self::Resize { seconds, cap_bytes } => {
                write!(f, "Resize({seconds} s, {cap_bytes} B)")
            }
            Self::Disarm => f.write_str("Disarm"),
            Self::Save(s) => write!(f, "Save({})", s.name),
            Self::Shutdown => f.write_str("Shutdown"),
        }
    }
}

/// A take to record on the pipeline.
pub struct TakeRequest {
    /// The app's number for it; [`PipelineCommand::StopTake`] names it.
    pub take: u64,
    pub path: PathBuf,
    /// Also save the voice and the desktop audio as separate WAV files.
    pub stems: bool,
    pub events: Arc<dyn TakeEvents>,
}

/// A press of Save.
pub struct SaveRequest {
    /// The press: the clip ends on the frame showing then.
    pub at: Instant,
    /// The recordings folder.
    pub dir: PathBuf,
    /// The file name, e.g. `divora-replay-2026-10-16_21-04-05.mp4`.
    pub name: String,
    pub reply: SaveReply,
    /// The clip's size, set once it is cut (0 until then): what Quit waits
    /// for.
    pub size: Arc<AtomicU64>,
    /// Quit's last resort: the save stops, and leaves no file.
    pub cancel: Arc<AtomicBool>,
}

/// Whoever is waiting for a save's outcome.
type Answer = Box<dyn FnOnce(Result<SavedClip, String>) + Send>;

/// The answer to one Save, given exactly once: dropped unanswered — its cut
/// cleared by a rebuild, the pipeline gone — it answers [`SAVE_DROPPED`].
pub struct SaveReply(Option<Answer>);

impl SaveReply {
    pub fn new(f: impl FnOnce(Result<SavedClip, String>) + Send + 'static) -> Self {
        Self(Some(Box::new(f)))
    }

    pub fn send(mut self, r: Result<SavedClip, String>) {
        if let Some(f) = self.0.take() {
            f(r);
        }
    }
}

impl Drop for SaveReply {
    fn drop(&mut self) {
        if let Some(f) = self.0.take() {
            f(Err(SAVE_DROPPED.to_string()));
        }
    }
}

/// A saved replay.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SavedClip {
    pub path: PathBuf,
    pub duration_ms: u64,
    pub bytes: u64,
}

/// What a take on a pipeline tells the app.
pub trait TakeEvents: Send + Sync {
    /// The file is open with its first frame: the take is recording. Not
    /// called for a take stopped before its first frame.
    fn started(&self);
    /// About once a second, the take so far.
    fn progress(&self, stats: &RecordStats);
    /// The take is over: its file, or why there is none. A take that ended
    /// by itself (a sleep, the screen gone) is `Err` with its file kept.
    /// Called exactly once.
    fn finished(&self, outcome: Result<PathBuf, String>, stats: RecordStats);
}

/// Why the ring stopped taking in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HeldReason {
    EngineStopped,
    ScreenLost,
    EncoderFailed,
}

/// The ring, as the app shows it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RingStatus {
    pub buffered_ms: u64,
    pub bytes: u64,
    pub cap_bytes: u64,
    pub seconds: u32,
}

/// What a pipeline tells the app, from its thread.
pub trait PipelineEvents: Send + Sync {
    /// The first keyframe of a segment is in the ring: Armed.
    fn ring_ready(&self);
    /// About once a second while armed or held.
    fn ring(&self, status: RingStatus);
    fn held(&self, reason: HeldReason, detail: Option<String>);
    /// The engine is back after a hold: Arming again, the ring cleared.
    fn resumed(&self);
    /// The session is being rebuilt (the PC slept, or the encoder restarted
    /// itself): an armed ring is Arming again, cleared; a held one stays
    /// held.
    fn rebuilding(&self);
    /// An Arm reached a pipeline on its way out.
    fn arm_refused(&self, why: String);
    /// The encoder proved it can't be trusted here (it re-stamps, makes
    /// B-frames, or ignores forced keyframes): no direct path again this run.
    fn direct_failed(&self, why: String);
    /// Replay turned itself off: its session couldn't be built again after
    /// a sleep or the encoder restarting itself. Off from any state.
    fn failed(&self, why: String);
    /// The thread has exited. The app detaches the engine's tap, then
    /// frees the slot.
    fn ended(&self, error: Option<String>);
}

/// Whether the audio engine is running, as a feed sees it. A device-loss
/// rebuild is not a stop.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum EngineSignal {
    #[default]
    Running,
    Stopped,
}

/// The wall clock, for naming files.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LocalTime {
    pub year: u16,
    pub month: u8,
    pub day: u8,
    pub hour: u8,
    pub minute: u8,
    pub second: u8,
}

/// Why a pipeline didn't start.
pub enum StartError {
    /// The direct path can't be used here. The feed comes back untouched, so
    /// the same take can go on through the sink writer.
    Direct {
        why: String,
        feed: Box<dyn AudioFeed + Send>,
    },
    Other(CaptureError),
}

impl std::fmt::Debug for StartError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Direct { why, .. } => write!(f, "Direct({why})"),
            Self::Other(e) => write!(f, "Other({e})"),
        }
    }
}

impl std::fmt::Display for StartError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Direct { why, .. } => write!(f, "can't use this video encoder: {why}"),
            Self::Other(e) => e.fmt(f),
        }
    }
}

/// A running pipeline.
pub struct PipelineHandle {
    tx: Sender<PipelineCommand>,
    wake: Arc<dyn Fn() + Send + Sync>,
    thread: Option<JoinHandle<()>>,
}

impl PipelineHandle {
    #[allow(dead_code)] // spawn_pipeline's; not built off Windows
    pub(crate) fn new(
        tx: Sender<PipelineCommand>,
        wake: Arc<dyn Fn() + Send + Sync>,
        thread: JoinHandle<()>,
    ) -> Self {
        Self {
            tx,
            wake,
            thread: Some(thread),
        }
    }

    /// Send `c` and wake the thread. A thread that has gone hands the
    /// command back, so the app can answer it (a Save's reply answers
    /// itself when dropped).
    pub fn send(&self, c: PipelineCommand) -> Result<(), PipelineCommand> {
        self.tx.send(c).map_err(|e| e.0)?;
        (self.wake)();
        Ok(())
    }

    /// Wait up to `timeout` for the thread to exit; whether it did.
    pub fn join(mut self, timeout: Duration) -> bool {
        let Some(thread) = self.thread.take() else {
            return true;
        };
        let until = Instant::now() + timeout;
        while !thread.is_finished() {
            if Instant::now() >= until {
                return false;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        let _ = thread.join();
        true
    }
}

impl std::fmt::Debug for PipelineHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PipelineHandle")
            .field(
                "running",
                &self.thread.as_ref().is_some_and(|t| !t.is_finished()),
            )
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::{SaveReply, SAVE_DROPPED};
    use std::sync::{Arc, Mutex};

    /// A reply answers once: as sent, or, dropped unsent, with the
    /// dropped text — so `saving` always clears.
    #[test]
    fn a_save_reply_answers_exactly_once() {
        let got = Arc::new(Mutex::new(Vec::new()));
        let g = Arc::clone(&got);
        SaveReply::new(move |r| g.lock().unwrap().push(r.map(|c| c.duration_ms))).send(Ok(
            super::SavedClip {
                path: "a.mp4".into(),
                duration_ms: 31_000,
                bytes: 1,
            },
        ));
        let g = Arc::clone(&got);
        drop(SaveReply::new(move |r| {
            g.lock().unwrap().push(r.map(|c| c.duration_ms));
        }));
        let got = got.lock().unwrap();
        assert_eq!(got.len(), 2);
        assert_eq!(got[0], Ok(31_000));
        assert_eq!(got[1], Err(SAVE_DROPPED.to_string()));
    }
}
