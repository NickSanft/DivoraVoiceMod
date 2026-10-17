//! The decisions of the capture pipeline (v1.54.0), kept apart from Windows
//! so CI can drive them with a pretend encoder.
//!
//! The pipeline is one capture, one encoder and one audio timeline, shared
//! by whatever needs them: a take, the replay ring, or both. Its Windows
//! shell (`native::pipeline`) captures, converts and talks to the encoder;
//! everything it then has to decide is here:
//!
//! - [`OutputRouter`]: which frame each encoder output belongs to, and
//!   whether the encoder can be trusted (no re-stamping, no B-frames, the
//!   keyframes it was asked for);
//! - [`TakeCore`]: when a take's file opens (its first keyframe, never an
//!   earlier frame), what of the shared audio is the take's, and where the
//!   take ends — as messages for the take's own writing thread;
//! - [`PipelineCore`]: all of it together — the clock, the shared timeline,
//!   the open take, the replay ring and a pending Save — and what every
//!   command, output, block of audio, fault and sleep does to them. The
//!   shell reads back what to send the take's writer, which clips to save
//!   and what to do with its session ([`Out`]).
//!
//! Nothing a user asked for is ever dropped without an answer: every take
//! ends with exactly one `End`, every Save is cut (or refused) exactly once,
//! and every way out of the pipeline resolves both first.

use std::collections::VecDeque;
use std::sync::Arc;
use std::time::Duration;

use crate::annexb;
use crate::keyframes::{KeyframeFault, KeyframeWatchdog};
use crate::pacer::FramePacer;
use crate::pipeline::{
    EngineSignal, HeldReason, PipelineEvents, RingStatus, SaveRequest, TakeEvents, NOTHING_YET,
    REPLAY_OFF, STILL_SAVING, STILL_STOPPING,
};
use crate::replay::{
    sample_of, AccessUnit, CutState, PendingCut, Pushed, ReplayRing, RingConfig, RingFault,
    Snapshot,
};
use crate::submit::{EncoderLiveness, OutputMatch, SubmitQueue};
use crate::take::{take_clock, AudioStats, RecordStats};
use crate::take_cut::{frame_of, BlockTail, TakeAudioCut};
use crate::timeline::{
    AudioBlock, AudioChunk, AudioTimeline, DesktopChunk, StemBlock, TimelineStats, TrackOptions,
};

/// How long a take's end waits for its last frames and audio: v1.53.0's.
pub const END_WAIT: Duration = Duration::from_millis(1_500);

/// The pipeline's clock fit covers the newest 5 to 10 minutes: an engine
/// epoch lasts as long as the engine runs, and a fit over all of it drifts.
pub const FIT_WINDOW_SECONDS: u32 = 600;

/// How often a take and the ring report.
const REPORT_EVERY: Duration = Duration::from_secs(1);

/// What an encoder that ignores forced keyframes is held with.
pub const NO_KEYFRAMES: &str = "This PC's encoder doesn't make regular keyframes.";

/// A take asked for while the session is being built again after a sleep.
pub const NO_PICTURE_YET: &str = "The screen isn't sending a picture yet — try again in a moment.";

/// Why Replay turned itself off: a session that couldn't be built again.
pub const REBUILD_FAILED: &str = "Replay turned off: the video encoder couldn't be started again.";

/// Why a held ring that a sleep emptied turns off instead of staying held:
/// it has nothing to save, and nothing will bring it back.
pub const EMPTIED_BY_SLEEP: &str = "Replay turned off: it had kept nothing since the PC slept.";

/// One output of the encoder, copied out of Media Foundation.
#[derive(Debug, Clone)]
pub struct EncodedUnit {
    /// 100-ns ticks, as the input sample was stamped (the encoder copies it).
    pub pts: i64,
    pub dur: i64,
    pub dts: Option<i64>,
    /// The encoder's own keyframe flag — a cross-check only: the bitstream
    /// (NAL type 5) decides.
    pub clean_point: Option<bool>,
    /// Annex B.
    pub data: Arc<[u8]>,
}

/// What the encoder hands back.
#[derive(Debug, Clone)]
pub enum EncoderEvent {
    Unit(EncodedUnit),
    /// `MF_E_TRANSFORM_STREAM_CHANGE`. Microsoft's stream-change protocol
    /// "does not apply to encoders", so the session gives up and the owner
    /// builds a new one rather than renegotiating.
    Restart,
    /// The drain asked for has finished.
    Drained,
}

/// Why the encoder can't be trusted any more. Each ends a take at once and
/// holds the replay ring.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EncoderFault {
    /// An output whose time matches no frame submitted: the encoder stamps
    /// its own times, so nothing can be placed.
    Restamped { pts: i64 },
    /// An output decoded at another time than it is shown: B-frames, which
    /// the file and the ring are not built for.
    BFrames { pts: i64 },
}

impl std::fmt::Display for EncoderFault {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Restamped { pts } => {
                write!(f, "the encoder restamped a frame (output at {pts})")
            }
            Self::BFrames { pts } => write!(f, "the encoder made B-frames (output at {pts})"),
        }
    }
}

/// One encoder output, placed.
#[derive(Debug, Clone)]
pub struct Routed {
    pub au: AccessUnit,
    /// A keyframe that was asked for and came too late — the ring's fault
    /// to raise; a take just opens on the next keyframe.
    pub late_keyframe: Option<KeyframeFault>,
}

/// Matches outputs to the frames submitted, and watches the keyframes.
#[derive(Debug)]
pub struct OutputRouter {
    fps: u32,
    queue: SubmitQueue,
    watchdog: KeyframeWatchdog,
    newest: Option<u64>,
}

impl OutputRouter {
    pub fn new(fps: u32, gop_frames: u32) -> Self {
        Self {
            fps,
            queue: SubmitQueue::new(),
            watchdog: KeyframeWatchdog::new(gop_frames),
            newest: None,
        }
    }

    /// Whether frame `index` should be a forced keyframe: a keyframe
    /// interval (GOP) of input since the last one came out.
    pub fn should_force(&self, index: u64) -> bool {
        self.watchdog.should_force(index)
    }

    /// Frame `index` went into the encoder at `pts`.
    pub fn on_submitted(&mut self, index: u64, pts: i64, forced: bool, now: Duration) {
        self.queue.push(index, pts, forced, now);
        self.watchdog.on_submitted(index, forced);
    }

    /// Place one output. A dropped frame (an output for a later frame) is
    /// counted and passed over; a frame split into pieces keeps its index.
    pub fn route(
        &mut self,
        pts: i64,
        dts: Option<i64>,
        data: Arc<[u8]>,
    ) -> Result<Routed, EncoderFault> {
        if dts.is_some_and(|d| d != pts) {
            return Err(EncoderFault::BFrames { pts });
        }
        let m = self.queue.match_output(pts);
        let Some(index) = m.index() else {
            debug_assert!(matches!(m, OutputMatch::Restamped));
            return Err(EncoderFault::Restamped { pts });
        };
        let au = AccessUnit::new(index, self.fps, data);
        let late_keyframe = self.watchdog.on_output(index, au.key).err();
        self.newest = Some(self.newest.map_or(index, |n| n.max(index)));
        Ok(Routed { au, late_keyframe })
    }

    /// The newest frame that came out of the encoder.
    pub fn newest(&self) -> Option<u64> {
        self.newest
    }

    /// The last frame that went in.
    pub fn last_submitted(&self) -> Option<u64> {
        self.queue.last_submitted_index()
    }

    /// When the oldest frame still waiting for its output went in.
    pub fn oldest_unanswered(&self) -> Option<Duration> {
        self.queue.oldest_submitted_at()
    }

    /// Frames in the encoder.
    pub fn in_flight(&self) -> usize {
        self.queue.len()
    }

    /// Frames the encoder took and never gave back.
    pub fn encoder_dropped(&self) -> u64 {
        self.queue.encoder_dropped()
    }
}

/// What a take's writing thread is told, in order.
#[derive(Debug, Clone)]
pub enum MuxMsg {
    /// Open the file: its first keyframe is next. `seq` is that keyframe's
    /// own SPS and PPS when it carries them.
    Open {
        seq: Arc<[u8]>,
    },
    Video(AccessUnit),
    /// A block of the take's mixed track, timed from the take's start.
    Audio(AudioBlock),
    /// The same for the separate files.
    Stems(Vec<StemBlock>),
    /// The last message: finish the file (if one was opened) and report
    /// the take with these stats; the writer adds what only it knows.
    End {
        how: TakeEnd,
        stats: Box<RecordStats>,
    },
}

/// How a take ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TakeEnd {
    /// Stopped by the user (or a time limit).
    Clean,
    /// Ended by something else; the text says what, as the card shows it.
    /// The file, if any, is kept.
    Cut(String),
}

/// Why a take ended by itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TakeCut {
    Slept,
    ScreenLost,
    EncoderFailed,
    EncoderRestarted,
    Disk,
}

impl TakeCut {
    /// What the card says: the take ended `seconds` in, and `opened` says
    /// whether it had written anything.
    #[must_use]
    pub fn message(self, seconds: f64, opened: bool) -> String {
        let what = match self {
            Self::Slept => "The PC went to sleep",
            Self::ScreenLost => "That screen was disconnected",
            Self::EncoderFailed => "The video encoder stopped working",
            Self::EncoderRestarted => "The video encoder restarted",
            Self::Disk => "The disk couldn't keep up",
        };
        if opened {
            format!(
                "{what}, so the recording stopped at {}. What was recorded is saved.",
                take_clock(seconds)
            )
        } else {
            format!("{what}.")
        }
    }
}

/// One take on the pipeline: where it starts and ends, and what of the
/// shared video and audio is its own.
#[derive(Debug)]
pub struct TakeCore {
    fps: u32,
    /// The first frame the take may contain: the next one after Record was
    /// pressed. Moved later if the forced keyframe doesn't come on it.
    k0: u64,
    /// The sequence header to fall back on when the opening keyframe
    /// carries none.
    seq: Arc<[u8]>,
    /// `Some` once the take's first keyframe has gone out.
    cut: Option<TakeAudioCut>,
    /// The end frame (exclusive), once stopped.
    end: Option<u64>,
    /// Access units sent.
    written: u64,
    finished: bool,
    /// The pipeline sample the take's video reaches: the end of its newest
    /// frame sent. Its audio goes no further (but for the block that starts
    /// before it), so a take ended early — an encoder that stalls, a sleep —
    /// has no audio past its last frame. Normally the audio is behind the
    /// picture anyway, and nothing waits.
    video_until: u64,
    /// Audio and stems not yet sent: past the video, or past the end.
    held_audio: VecDeque<AudioBlock>,
    held_stems: VecDeque<StemBlock>,
}

impl TakeCore {
    /// A take whose first frame may be `k0` (a keyframe forced on it, or the
    /// next natural one).
    pub fn new(k0: u64, fps: u32, seq: Arc<[u8]>) -> Self {
        Self {
            fps,
            k0,
            seq,
            cut: None,
            end: None,
            written: 0,
            finished: false,
            video_until: 0,
            held_audio: VecDeque::new(),
            held_stems: VecDeque::new(),
        }
    }

    /// Start again from `k0` with `seq`: the pipeline's clock restarted
    /// (after a sleep) before the take's first keyframe came.
    pub fn rebase(&mut self, k0: u64, seq: Arc<[u8]>) {
        debug_assert!(self.cut.is_none(), "an open take never crosses a sleep");
        if self.cut.is_none() {
            self.k0 = k0;
            self.seq = seq;
        }
    }

    /// The take's first frame, once open.
    pub fn first_frame(&self) -> Option<u64> {
        self.cut.as_ref().map(|_| self.k0)
    }

    pub fn is_open(&self) -> bool {
        self.cut.is_some()
    }

    /// Access units sent to the file.
    pub fn written(&self) -> u64 {
        self.written
    }

    /// The end frame, once stopped.
    pub fn end(&self) -> Option<u64> {
        self.end
    }

    /// An encoded frame. Opens the take on its first keyframe at or after
    /// `k0` — never an earlier one, so nothing from before Record was
    /// pressed is in it — seeding its audio from `tail`.
    pub fn on_au(&mut self, au: &AccessUnit, tail: &BlockTail, out: &mut Vec<MuxMsg>) {
        if self.finished || au.index < self.k0 || self.end.is_some_and(|e| au.index >= e) {
            return;
        }
        if self.cut.is_none() {
            if !au.key {
                return;
            }
            // A keyframe later than asked for: the take starts there.
            self.k0 = au.index;
            let seq = annexb::parameter_sets(&au.data).unwrap_or_else(|| self.seq.clone());
            self.cut = Some(TakeAudioCut::new(sample_of(self.k0, self.fps), tail));
            out.push(MuxMsg::Open { seq });
        }
        out.push(MuxMsg::Video(au.clone()));
        self.written += 1;
        self.video_until = self.video_until.max(sample_of(au.index + 1, self.fps));
        self.release(out);
    }

    /// A block of the shared track; nothing until the take is open (the
    /// tail holds what it will need), and sent as far as the video goes.
    pub fn on_block(&mut self, block: &AudioBlock, out: &mut Vec<MuxMsg>) {
        if self.finished || self.cut.is_none() {
            return;
        }
        self.held_audio.push_back(block.clone());
        self.release(out);
    }

    /// Blocks of the separate files; the same rule.
    pub fn on_stems(&mut self, blocks: &[StemBlock], out: &mut Vec<MuxMsg>) {
        if self.finished || self.cut.is_none() {
            return;
        }
        self.held_stems.extend(blocks.iter().cloned());
        self.release(out);
    }

    /// Send what is held that starts before `limit`, in order: the video's
    /// reach, or the end once known, whichever is first.
    fn release_to(&mut self, limit: u64, out: &mut Vec<MuxMsg>) {
        let Some(cut) = self.cut.as_mut() else {
            return;
        };
        while self
            .held_audio
            .front()
            .is_some_and(|b| frame_of(b.pts) < limit)
        {
            if let Some(b) = self.held_audio.pop_front() {
                out.extend(cut.audio(&b).into_iter().map(MuxMsg::Audio));
            }
        }
        let mut stems = Vec::new();
        while self
            .held_stems
            .front()
            .is_some_and(|b| frame_of(b.pts) < limit)
        {
            if let Some(b) = self.held_stems.pop_front() {
                stems.extend(cut.stems(&b));
            }
        }
        if !stems.is_empty() {
            out.push(MuxMsg::Stems(stems));
        }
    }

    fn release(&mut self, out: &mut Vec<MuxMsg>) {
        let mut limit = self.video_until;
        if let Some(end) = self.end {
            limit = limit.min(sample_of(end, self.fps));
        }
        self.release_to(limit, out);
    }

    /// Stop after the last frame submitted: nothing the encoder hasn't been
    /// given is part of the take.
    pub fn stop(&mut self, last_submitted: Option<u64>) {
        if self.end.is_none() {
            self.end = Some(last_submitted.map_or(self.k0, |l| l + 1).max(self.k0));
        }
    }

    /// End no later than after frame `last` (inclusive), whatever a stop
    /// said: nothing after it will come out, or none of it is the take's.
    pub fn cut_short(&mut self, last: Option<u64>) {
        let end = last.map_or(self.k0, |l| l + 1).max(self.k0);
        self.end = Some(self.end.map_or(end, |e| e.min(end)));
    }

    /// Whether everything up to the end is in: its last frame came out of
    /// the encoder, and the timeline has handed out audio to its end.
    pub fn ready(&self, newest_au: Option<u64>, audio_until_sample: u64) -> bool {
        let Some(end) = self.end else {
            return false;
        };
        let video = end == 0 || newest_au.is_some_and(|n| n + 1 >= end);
        video && audio_until_sample >= sample_of(end, self.fps)
    }

    /// End the take, now — ready or out of time. The video ends at the
    /// stop point or after the newest frame received, whichever is first;
    /// the audio is filled out with silence to there. Returns the end
    /// frame, or `None` when the take never opened (there is no file). The
    /// caller sends the `End`.
    pub fn finish(&mut self, newest_au: Option<u64>, out: &mut Vec<MuxMsg>) -> Option<u64> {
        if self.finished {
            return None;
        }
        if self.cut.is_none() {
            self.finished = true;
            return None;
        }
        let received = newest_au.map_or(self.k0, |n| n + 1);
        let end = self.end.unwrap_or(received).min(received).max(self.k0);
        self.end = Some(end);
        // What is held, up to the end now known; the rest is past it.
        let end_sample = sample_of(end, self.fps);
        self.release_to(end_sample, out);
        self.held_audio.clear();
        self.held_stems.clear();
        self.finished = true;
        let cut = self.cut.as_mut()?;
        let frames = end_sample - sample_of(self.k0, self.fps);
        let (audio, stems) = cut.finish(frames);
        out.extend(audio.into_iter().map(MuxMsg::Audio));
        if !stems.is_empty() {
            out.push(MuxMsg::Stems(stems));
        }
        Some(end)
    }

    /// Frames of silence the take starts with (the tail didn't reach).
    pub fn lead_silence(&self) -> u64 {
        self.cut.as_ref().map_or(0, TakeAudioCut::lead_silence)
    }
}

/// What the shell must do with its session — the device, capture, encoder
/// and pool — after a call into the core.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionAction {
    /// Stop capturing (Windows' border goes) and drain the encoder, handing
    /// what comes back to [`PipelineCore::on_encoder`]: a take is ending and
    /// nothing else needs the screen.
    Drain,
    /// Let the session go, quiescing the encoder first when it can still be
    /// talked to.
    Release { quiesce: bool },
    /// The encoder restarted itself: build a new encoder and pool on the
    /// same capture and clock, then [`PipelineCore::encoder_rebuilt`].
    RebuildEncoder,
    /// The PC slept: build everything again, wait for a picture for as long
    /// as it takes, then [`PipelineCore::restarted`] on a new clock.
    Rebuild,
}

/// What one call into the core asks of the shell, in order.
#[derive(Default)]
pub struct Out {
    /// For the open take's writer.
    pub mux: Vec<MuxMsg>,
    /// Clips cut: each to a save thread of its own, which answers its
    /// request's reply.
    pub saves: Vec<(Snapshot, SaveRequest)>,
    pub session: Vec<SessionAction>,
    /// Nothing needs the pipeline any more: end the thread.
    pub exit: bool,
}

/// A frame the pacer says is due.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Due {
    pub index: u64,
    pub pts: u64,
    pub dur: u64,
    /// Must be a keyframe.
    pub force: bool,
}

/// Counts the shell keeps the core told of.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FrameCounters {
    /// Frames Windows Graphics Capture delivered.
    pub arrived: u64,
    /// Ticks with nothing new on screen.
    pub repeated: u64,
    /// Ticks whose frame was replaced before the encoder asked for one.
    pub no_credit: u64,
    /// Ticks with no free input surface.
    pub pool_empty: u64,
}

/// What a pipeline is built with, as the core needs it.
#[derive(Debug, Clone)]
pub struct CoreConfig {
    pub fps: u32,
    /// The encoded picture's size.
    pub size: (u32, u32),
    /// Desktop audio and its gain; stems are always made.
    pub track: TrackOptions,
    /// Whether there is an audio feed at all (the app always has one).
    pub with_audio: bool,
    /// The encoder, as takes report it.
    pub encoder: Option<String>,
    pub hardware: bool,
    /// The session's sequence header from the encoder's media type.
    pub seq: Option<Arc<[u8]>>,
}

/// The ring as the app sees it, for tests and the log.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplayView {
    Arming,
    Armed,
    Held(HeldReason),
}

/// Whether there is a picture to encode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Video {
    Live,
    /// Captured no more; the encoder drained for a take's end.
    Drained,
    /// A new encoder is being built on the same capture and clock.
    Rebuilding,
    /// After a sleep: everything is being built again, and the clock starts
    /// over with the first picture.
    Waiting,
    /// No session (the screen went, the encoder failed).
    Gone,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RingPhase {
    /// The next keyframe at or after `from` starts a segment.
    Arming {
        from: u64,
    },
    Armed,
    Held(HeldReason),
}

struct RingSide {
    ring: ReplayRing,
    phase: RingPhase,
    /// A segment has begun (its first keyframe is in); the ring takes
    /// nothing until then.
    segment: bool,
    cut: Option<(PendingCut, SaveRequest)>,
}

/// What a take started from, so it reports its own counts.
#[derive(Clone, Copy)]
struct TakeBase {
    frames: FrameCounters,
    dropped: u64,
    timeline: TimelineStats,
}

struct OpenTake {
    id: u64,
    core: TakeCore,
    events: Arc<dyn TakeEvents>,
    stems: bool,
    /// Stopping: when to give up waiting for the rest.
    deadline: Option<Duration>,
    /// Why, when it wasn't a stop.
    cut: Option<TakeCut>,
    /// Its last frame is known to have come out (or never will).
    video_done: bool,
    /// A frame converted but never submitted when it stopped.
    extra_dropped: u64,
    base: TakeBase,
}

/// The pipeline's decisions: see the module docs.
#[allow(clippy::struct_excessive_bools)] // independent facts about the session and the ring
pub struct PipelineCore {
    cfg: CoreConfig,
    events: Arc<dyn PipelineEvents>,
    pacer: FramePacer,
    router: OutputRouter,
    liveness: EncoderLiveness,
    /// The session's sequence header: the encoder's, or its first IDR's.
    seq: Option<Arc<[u8]>>,
    /// The session's first IDR has come out.
    first_idr: bool,
    header_adopted: bool,
    /// A keyframe is wanted (a take or a segment starts).
    force: bool,
    timeline: AudioTimeline,
    tail: BlockTail,
    /// Pipeline samples the timeline has handed out.
    emitted: u64,
    take: Option<OpenTake>,
    ring: Option<RingSide>,
    frames: FrameCounters,
    /// Drops counted by pacers and routers since replaced.
    retired_dropped: u64,
    video: Video,
    engine: EngineSignal,
    /// Disarm is waiting for a cut to be handed over.
    disarming: bool,
    /// Shutdown: nothing new is taken on.
    leaving: bool,
    last_report: Option<Duration>,
    /// The timeline's clock counted a sleep and nothing rebuilds it: it is
    /// never pumped again (it would write the whole sleep as silence).
    no_clock: bool,
    /// The encoder was found ignoring forced keyframes: said once, then
    /// only counted.
    keyframes_untrusted: bool,
    keyframe_faults: u64,
}

fn new_timeline(track: TrackOptions) -> AudioTimeline {
    // Stems always: a take started on a running pipeline gets its separate
    // files from its first sample, and they are drained every pump.
    AudioTimeline::with_options(TrackOptions {
        stems: true,
        ..track
    })
    .with_fit_window(FIT_WINDOW_SECONDS)
}

fn empty_seq() -> Arc<[u8]> {
    Arc::from(&[][..])
}

#[allow(clippy::cast_precision_loss)] // frames in a take: exact far past any length
fn seconds_between(first: Option<u64>, end: Option<u64>, fps: u32) -> f64 {
    match (first, end) {
        (Some(f), Some(e)) => e.saturating_sub(f) as f64 / f64::from(fps.max(1)),
        _ => 0.0,
    }
}

impl PipelineCore {
    /// A pipeline whose clock starts now, with no take and no ring.
    pub fn new(cfg: CoreConfig, events: Arc<dyn PipelineEvents>) -> Self {
        let fps = cfg.fps;
        Self {
            pacer: FramePacer::new(fps),
            router: OutputRouter::new(fps, 2 * fps),
            liveness: EncoderLiveness::default(),
            seq: cfg.seq.clone(),
            first_idr: false,
            header_adopted: false,
            force: false,
            timeline: new_timeline(cfg.track),
            tail: BlockTail::default(),
            emitted: 0,
            take: None,
            ring: None,
            frames: FrameCounters::default(),
            retired_dropped: 0,
            video: Video::Live,
            engine: EngineSignal::Running,
            disarming: false,
            leaving: false,
            last_report: None,
            no_clock: false,
            keyframes_untrusted: false,
            keyframe_faults: 0,
            cfg,
            events,
        }
    }

    // ---- frames -------------------------------------------------------

    /// The frame due at `now`, if any, and whether it must be a keyframe.
    /// `None` while there is no picture to encode.
    pub fn tick(&mut self, now: Duration) -> Option<Due> {
        if self.video != Video::Live {
            return None;
        }
        let e = self.pacer.due(now)?;
        Some(Due {
            index: e.index,
            pts: e.pts,
            dur: e.duration,
            force: self.force || self.router.should_force(e.index),
        })
    }

    /// When the next frame is due.
    pub fn next_deadline(&self) -> Duration {
        self.pacer.next_deadline()
    }

    /// Frame `index` went into the encoder.
    pub fn submitted(&mut self, index: u64, pts: i64, forced: bool, now: Duration) {
        self.router.on_submitted(index, pts, forced, now);
        if forced {
            self.force = false;
        }
    }

    /// The counts the shell keeps.
    pub fn counters(&mut self) -> &mut FrameCounters {
        &mut self.frames
    }

    /// Whether the encoder's outputs are wanted and its liveness checked.
    pub fn encoding(&self) -> bool {
        matches!(self.video, Video::Live | Video::Drained)
    }

    /// Whether the shell should be capturing and ticking.
    pub fn capturing(&self) -> bool {
        self.video == Video::Live
    }

    /// The encoder's first IDR brought its own SPS/PPS, unlike its media
    /// type's: they were adopted as the session's header.
    pub fn header_adopted(&self) -> bool {
        self.header_adopted
    }

    /// The ring, if Replay is on.
    pub fn replay(&self) -> Option<ReplayView> {
        self.ring.as_ref().map(|r| match r.phase {
            RingPhase::Arming { .. } => ReplayView::Arming,
            RingPhase::Armed => ReplayView::Armed,
            RingPhase::Held(h) => ReplayView::Held(h),
        })
    }

    /// Whether a take is open (stopping included).
    pub fn has_take(&self) -> bool {
        self.take.is_some()
    }

    /// Pipeline samples the timeline has handed out.
    pub fn emitted(&self) -> u64 {
        self.emitted
    }

    pub fn tail(&self) -> &BlockTail {
        &self.tail
    }

    pub fn timeline_stats(&self) -> TimelineStats {
        self.timeline.stats()
    }

    /// Keyframes asked for that didn't come, for the summary at the end.
    pub fn keyframe_faults(&self) -> u64 {
        self.keyframe_faults
    }

    /// Stem blocks the timeline holds, not yet handed out.
    #[cfg(test)]
    fn stems_waiting(&self) -> usize {
        self.timeline.stems_waiting()
    }

    fn dropped_total(&self) -> u64 {
        self.retired_dropped
            + self.pacer.dropped()
            + self.router.encoder_dropped()
            + self.frames.no_credit
            + self.frames.pool_empty
    }

    /// Check that the encoder still answers (§A.2.8). `pending_since`: when
    /// the frame waiting to be submitted first found no request.
    pub fn check_liveness(
        &mut self,
        now: Duration,
        pending_since: Option<Duration>,
        out: &mut Out,
    ) {
        if self.video != Video::Live {
            return;
        }
        if let Err(f) = self.liveness.check(
            now,
            self.router.oldest_unanswered(),
            pending_since,
            self.router.in_flight(),
            2 * self.cfg.fps as usize,
        ) {
            self.encoder_failed(&f.to_string(), false, true, now, out);
        }
    }

    // ---- encoder output -----------------------------------------------

    /// What the encoder handed back: placed, checked, and given to the ring
    /// and the take.
    pub fn on_encoder(&mut self, events: &mut Vec<EncoderEvent>, now: Duration, out: &mut Out) {
        for ev in events.drain(..) {
            if !self.encoding() {
                // An encoder given up on, or being replaced: whatever else
                // it said is no one's.
                continue;
            }
            match ev {
                EncoderEvent::Unit(u) => match self.router.route(u.pts, u.dts, u.data) {
                    Ok(r) => self.on_unit(r, now, out),
                    Err(f) => self.encoder_failed(&f.to_string(), true, true, now, out),
                },
                EncoderEvent::Restart => self.restart_encoder(now, out),
                EncoderEvent::Drained => {}
            }
        }
    }

    fn on_unit(&mut self, r: Routed, now: Duration, out: &mut Out) {
        let au = r.au;
        if let Some(k) = r.late_keyframe {
            self.keyframes_ignored(&k.to_string(), now, out);
        }
        if au.key && annexb::has_parameter_sets(&au.data) {
            let differs = self
                .seq
                .as_ref()
                .is_none_or(|s| !annexb::same_parameter_sets(s, &au.data));
            if !self.first_idr {
                // The media type's header isn't documented as final before
                // streaming: the session's first IDR may bring its own.
                if differs {
                    if self.seq.is_some() {
                        self.header_adopted = true;
                        tracing::info!("the encoder's first keyframe brought its own header");
                    }
                    self.seq = annexb::parameter_sets(&au.data);
                }
            } else if differs && self.seq.is_some() {
                self.header_changed(&au, now, out);
            }
        }
        if au.key {
            self.first_idr = true;
        }
        self.push_ring(&au, now, out);
        if let Some(t) = self.take.as_mut() {
            t.core.on_au(&au, &self.tail, &mut out.mux);
        }
    }

    /// A later IDR with another SPS/PPS: a take's file has one header, so
    /// it ends before this frame; the ring starts a new segment on it.
    fn header_changed(&mut self, au: &AccessUnit, now: Duration, out: &mut Out) {
        tracing::info!(index = au.index, "the encoder's sequence header changed");
        self.seq = annexb::parameter_sets(&au.data);
        if self.take.as_ref().is_some_and(|t| t.core.is_open()) {
            self.end_take_waiting(
                TakeCut::EncoderRestarted,
                Some(au.index.saturating_sub(1)),
                now,
            );
        }
        let size = self.cfg.size;
        if let Some(rs) = self.ring.as_mut() {
            if rs.segment && !matches!(rs.phase, RingPhase::Held(_)) {
                resolve_cut(rs, size, out);
                rs.segment = false;
                rs.phase = RingPhase::Arming { from: au.index };
            }
        }
    }

    /// Give `au` to the ring: the first keyframe of a segment starts it.
    fn push_ring(&mut self, au: &AccessUnit, now: Duration, out: &mut Out) {
        let fps = self.cfg.fps;
        let size = self.cfg.size;
        let Some(rs) = self.ring.as_mut() else {
            return;
        };
        if let RingPhase::Arming { from } = rs.phase {
            if !(au.key && au.index >= from) {
                return;
            }
            start_segment(rs, self.seq.clone(), au.index, fps, &self.tail);
            rs.phase = RingPhase::Armed;
            tracing::info!(index = au.index, "replay is keeping the screen");
            self.events.ring_ready();
        }
        if !rs.segment {
            return;
        }
        let fault = match rs.ring.push_video(au.clone()) {
            Ok(Pushed::HeaderChanged) => {
                // Only a segment begun without any header can disagree with
                // the core's: start again on this keyframe.
                resolve_cut(rs, size, out);
                start_segment(
                    rs,
                    annexb::parameter_sets(&au.data),
                    au.index,
                    fps,
                    &self.tail,
                );
                let _ = rs.ring.push_video(au.clone());
                None
            }
            Ok(_) => None,
            Err(RingFault::NoKeyframe { frames_since }) => {
                Some(format!("no keyframe for {frames_since} frames"))
            }
        };
        if let Some(detail) = fault {
            self.keyframes_ignored(&detail, now, out);
        }
    }

    /// The encoder doesn't make the keyframes it is asked for: the ring
    /// can't rely on it, and the direct path isn't used again this run. A
    /// take carries on (it opened on a natural keyframe).
    fn keyframes_ignored(&mut self, detail: &str, now: Duration, out: &mut Out) {
        // The watchdog asks again a few frames later, so this repeats for as
        // long as a take keeps the encoder: said once, then counted.
        self.keyframe_faults += 1;
        if self.keyframes_untrusted {
            return;
        }
        self.keyframes_untrusted = true;
        tracing::warn!(detail, "the encoder ignores forced keyframes");
        self.events
            .direct_failed(format!("{NO_KEYFRAMES} ({detail})"));
        let at = self
            .router
            .newest()
            .map_or_else(|| self.pacer.next_index(), |n| n + 1);
        self.hold(
            HeldReason::EncoderFailed,
            Some(NO_KEYFRAMES.to_string()),
            at,
            out,
        );
        self.settle(now, out);
    }

    /// The encoder failed: a take ends at its last frame received, the ring
    /// is held, the session goes. `untrusted`: it misbehaved rather than
    /// broke (no direct path again this run). `quiesce`: it can still be
    /// talked to.
    pub fn encoder_failed(
        &mut self,
        detail: &str,
        untrusted: bool,
        quiesce: bool,
        now: Duration,
        out: &mut Out,
    ) {
        if matches!(self.video, Video::Gone | Video::Waiting) {
            return;
        }
        tracing::warn!(detail, untrusted, "the direct encoder failed");
        let newest = self.router.newest();
        self.end_take_waiting(TakeCut::EncoderFailed, newest, now);
        let at = newest.map_or_else(|| self.pacer.next_index(), |n| n + 1);
        self.hold(HeldReason::EncoderFailed, Some(detail.to_string()), at, out);
        if untrusted {
            self.events.direct_failed(detail.to_string());
        }
        self.video = Video::Gone;
        out.session.push(SessionAction::Release { quiesce });
        self.settle(now, out);
    }

    /// `MF_E_TRANSFORM_STREAM_CHANGE`: never renegotiated. A take ends; a
    /// ring gets a new encoder on the same capture and clock.
    fn restart_encoder(&mut self, now: Duration, out: &mut Out) {
        tracing::warn!("the encoder restarted itself");
        let newest = self.router.newest();
        self.end_take_waiting(TakeCut::EncoderRestarted, newest, now);
        let size = self.cfg.size;
        let next = self.pacer.next_index();
        let mut rebuild = false;
        if let Some(rs) = self.ring.as_mut() {
            resolve_cut(rs, size, out);
            match rs.phase {
                RingPhase::Armed | RingPhase::Arming { .. } => {
                    rs.segment = false;
                    rs.phase = RingPhase::Arming { from: next };
                    self.events.rebuilding();
                    rebuild = true;
                }
                // The capture runs on for the engine's return.
                RingPhase::Held(HeldReason::EngineStopped) => rebuild = true,
                RingPhase::Held(_) => {}
            }
        }
        if rebuild {
            self.video = Video::Rebuilding;
            out.session.push(SessionAction::RebuildEncoder);
        } else {
            self.video = Video::Gone;
            out.session.push(SessionAction::Release { quiesce: true });
        }
        self.settle(now, out);
    }

    /// A new encoder is ready on the same capture and clock. Ignored unless
    /// one was asked for and nothing has overtaken it (a sleep in between
    /// rebuilds everything instead).
    pub fn encoder_rebuilt(&mut self, seq: Option<Arc<[u8]>>) {
        if self.video != Video::Rebuilding {
            return;
        }
        self.retired_dropped += self.router.encoder_dropped();
        self.router = OutputRouter::new(self.cfg.fps, 2 * self.cfg.fps);
        self.liveness = EncoderLiveness::default();
        self.seq = seq;
        self.first_idr = false;
        self.force = true;
        self.video = Video::Live;
    }

    /// A rebuild (after a sleep or a stream change) couldn't be done: Replay
    /// turns off, whatever it held (the owner's decision), and a take still
    /// on the pipeline ends.
    pub fn rebuild_failed(&mut self, detail: &str, now: Duration, out: &mut Out) {
        tracing::warn!(detail, "the encoder couldn't be built again");
        // After a sleep the clock counted it: never pumped again.
        let slept = self.video == Video::Waiting;
        self.video = Video::Gone;
        if slept {
            self.no_clock = true;
        }
        let newest = self.router.newest();
        if slept {
            self.end_take_now(TakeCut::EncoderFailed, newest, now, out);
        } else {
            self.end_take_waiting(TakeCut::EncoderFailed, newest, now);
        }
        let size = self.cfg.size;
        if let Some(mut rs) = self.ring.take() {
            resolve_cut(&mut rs, size, out);
            tracing::info!(stats = ?rs.ring.stats(), "replay is off: no encoder");
            self.events.failed(format!("{REBUILD_FAILED} ({detail})"));
        }
        self.disarming = false;
        out.session.push(SessionAction::Release { quiesce: false });
        self.settle(now, out);
    }

    /// Windows said the screen is gone. The shell has already drained the
    /// encoder into [`Self::on_encoder`]: a take ends at its last frame, the
    /// ring is held there.
    pub fn on_screen_lost(&mut self, now: Duration, out: &mut Out) {
        tracing::info!("the screen went away");
        let newest = self.router.newest();
        self.end_take_waiting(TakeCut::ScreenLost, newest, now);
        let at = newest.map_or_else(|| self.pacer.next_index(), |n| n + 1);
        self.hold(HeldReason::ScreenLost, None, at, out);
        self.video = Video::Gone;
        out.session.push(SessionAction::Release { quiesce: false });
        self.settle(now, out);
    }

    // ---- audio --------------------------------------------------------

    /// What the feed brought, and whether the engine is running. Pumps the
    /// timeline to `now`, giving its blocks to the tail, the ring and the
    /// take.
    pub fn on_audio(
        &mut self,
        chunks: &[AudioChunk],
        desktop: &[DesktopChunk],
        engine: EngineSignal,
        now: Duration,
        out: &mut Out,
    ) {
        let live_clock = self.video != Video::Waiting && !self.no_clock;
        if live_clock && self.cfg.with_audio {
            for c in chunks {
                self.timeline.push(c);
            }
            for c in desktop {
                self.timeline.push_desktop(c);
            }
            self.pump(now, out);
        }
        if engine != self.engine {
            self.engine = engine;
            match engine {
                EngineSignal::Stopped => {
                    // The soundboard placed past the voice's cursor is
                    // released, so a held clip ends with it.
                    if live_clock && self.cfg.with_audio {
                        self.timeline.close_epoch();
                        self.pump(now, out);
                    }
                    let at = self.pacer.next_index();
                    self.hold(HeldReason::EngineStopped, None, at, out);
                    self.settle(now, out);
                }
                EngineSignal::Running => self.engine_back(out),
            }
        }
    }

    fn pump(&mut self, now: Duration, out: &mut Out) {
        for b in self.timeline.pump(now.as_secs_f64()) {
            let first = frame_of(b.pts);
            self.emitted = first + (b.samples.len() / 2) as u64;
            if let Some(rs) = self.ring.as_mut() {
                if rs.segment {
                    rs.ring.push_pcm(first, &b.samples);
                }
            }
            if let Some(t) = self.take.as_mut() {
                t.core.on_block(&b, &mut out.mux);
            }
            self.tail.push(b);
        }
        // Drained every pump, whether or not a take wants them.
        let stems = self.timeline.take_stems();
        if let Some(t) = self.take.as_mut() {
            if t.stems {
                t.core.on_stems(&stems, &mut out.mux);
            }
        }
        self.tail.push_stems(stems);
    }

    /// The engine started again: a ring held for its stop arms again on a
    /// new segment.
    fn engine_back(&mut self, out: &mut Out) {
        let size = self.cfg.size;
        let next = self.pacer.next_index();
        let Some(rs) = self.ring.as_mut() else {
            return;
        };
        if rs.phase != RingPhase::Held(HeldReason::EngineStopped) {
            return;
        }
        resolve_cut(rs, size, out);
        rs.segment = false;
        rs.phase = RingPhase::Arming { from: next };
        self.force = true;
        tracing::info!("the engine is back; replay is arming again");
        self.events.resumed();
    }

    // ---- sleep --------------------------------------------------------

    /// The PC slept. The shell has thrown away what the feed held and given
    /// the encoder's last outputs to [`Self::on_encoder`]; nothing has read
    /// the clock since. Everything is resolved as of `t_jump`, the last
    /// moment before the sleep: a take ends there, a pending Save is cut
    /// there. Then a ring waits for a rebuilt session.
    pub fn on_sleep(&mut self, t_jump: Duration, out: &mut Out) {
        tracing::info!(at = ?t_jump, "the PC slept");
        if self.cfg.with_audio && self.video != Video::Waiting && !self.no_clock {
            self.pump(t_jump, out);
        }
        let newest = self.router.newest();
        self.end_take_now(TakeCut::Slept, newest, t_jump, out);
        let size = self.cfg.size;
        let mut rebuild = false;
        let mut emptied = false;
        if let Some(rs) = self.ring.as_mut() {
            resolve_cut(rs, size, out);
            // Nothing from before the sleep is kept.
            rs.ring.start_segment(None, 0);
            rs.segment = false;
            match rs.phase {
                RingPhase::Armed | RingPhase::Arming { .. } => {
                    rs.phase = RingPhase::Arming { from: 0 };
                    rebuild = true;
                }
                // Its capture and encoder were running.
                RingPhase::Held(HeldReason::EngineStopped) => rebuild = true,
                // No screen or encoder to keep with again, and now nothing
                // to save.
                RingPhase::Held(_) => emptied = true,
            }
        }
        if emptied {
            tracing::info!("replay is off: a sleep emptied a ring held for good");
            self.ring = None;
            self.disarming = false;
            self.events.failed(EMPTIED_BY_SLEEP.to_string());
        } else if self.ring.is_some() {
            // An armed ring is Arming again; one held for the engine stays
            // held, emptied, until the engine is back.
            self.events.rebuilding();
        }
        if rebuild {
            self.video = Video::Waiting;
            out.session.push(SessionAction::Rebuild);
        } else {
            // Nothing rebuilds the clock: the timeline from before the sleep
            // is never pumped again.
            self.no_clock = true;
        }
        self.settle(t_jump, out);
    }

    /// A rebuilt session's first picture is in: the clock starts again at
    /// zero, with a new timeline and an empty tail (the old ones were on
    /// the old clock).
    pub fn restarted(&mut self, seq: Option<Arc<[u8]>>, size: (u32, u32)) {
        let fps = self.cfg.fps;
        self.retired_dropped += self.pacer.dropped() + self.router.encoder_dropped();
        self.pacer = FramePacer::new(fps);
        self.router = OutputRouter::new(fps, 2 * fps);
        self.liveness = EncoderLiveness::default();
        self.cfg.size = size;
        self.seq = seq;
        self.first_idr = false;
        self.timeline = new_timeline(self.cfg.track);
        self.tail.clear();
        self.emitted = 0;
        self.force = true;
        self.video = Video::Live;
        self.no_clock = false;
        self.last_report = None;
        if let Some(rs) = self.ring.as_mut() {
            if let RingPhase::Arming { .. } = rs.phase {
                rs.phase = RingPhase::Arming { from: 0 };
            }
        }
        let fresh = self.timeline.stats();
        let seq = self.seq.clone().unwrap_or_else(empty_seq);
        if let Some(t) = self.take.as_mut() {
            t.core.rebase(0, seq);
            t.base.timeline = fresh;
        }
        tracing::info!(width = size.0, height = size.1, "the capture is back");
    }

    // ---- commands -----------------------------------------------------

    /// Why a take can't start now, if it can't.
    pub fn take_refusal(&self) -> Option<&'static str> {
        if self.take.is_some() {
            Some("A screen recording is already running.")
        } else if self.leaving || matches!(self.video, Video::Drained | Video::Gone) {
            Some(STILL_STOPPING)
        } else if matches!(self.video, Video::Waiting | Video::Rebuilding) {
            // No session, no clock, no size yet: after a sleep, until the
            // screen sends a picture.
            Some(NO_PICTURE_YET)
        } else {
            None
        }
    }

    /// Start a take on the next frame, a keyframe forced there. The shell
    /// has its writer ready; check [`Self::take_refusal`] first.
    pub fn start_take(&mut self, id: u64, stems: bool, events: Arc<dyn TakeEvents>) {
        debug_assert!(self.take_refusal().is_none());
        let k0 = self.pacer.next_index();
        self.force = true;
        let base = TakeBase {
            frames: self.frames,
            dropped: self.dropped_total(),
            timeline: self.timeline.stats(),
        };
        let seq = self.seq.clone().unwrap_or_else(empty_seq);
        self.take = Some(OpenTake {
            id,
            core: TakeCore::new(k0, self.cfg.fps, seq),
            events,
            stems,
            deadline: None,
            cut: None,
            video_done: false,
            extra_dropped: 0,
            base,
        });
    }

    /// Stop take `id` after the last frame submitted, then wait (up to
    /// [`END_WAIT`]) for that frame and its audio. With Replay off nothing
    /// else needs the screen: it is let go now and the encoder drained.
    /// `frame_pending`: a frame was converted but not yet submitted — it is
    /// not the take's, and counts as dropped.
    pub fn stop_take(&mut self, id: u64, frame_pending: bool, now: Duration, out: &mut Out) {
        let last = self.router.last_submitted();
        let Some(t) = self.take.as_mut().filter(|t| t.id == id) else {
            return;
        };
        if t.deadline.is_some() {
            return;
        }
        t.core.stop(last);
        if frame_pending {
            t.extra_dropped += 1;
        }
        t.deadline = Some(now + END_WAIT);
        if self.ring.is_none() && self.video == Video::Live {
            self.video = Video::Drained;
            out.session.push(SessionAction::Drain);
        }
    }

    /// Turn the ring on (or resize the one there is). A ring still waiting
    /// to hand over a cut before a Disarm drops it is finished with first:
    /// this arm starts a fresh one, which says when it is ready.
    pub fn arm(&mut self, seconds: u32, cap_bytes: u64, out: &mut Out) {
        if self.disarming {
            let size = self.cfg.size;
            if let Some(mut rs) = self.ring.take() {
                resolve_cut(&mut rs, size, out);
            }
            self.disarming = false;
        }
        if let Some(rs) = self.ring.as_mut() {
            rs.ring.resize(seconds, cap_bytes);
            return;
        }
        if self.leaving || matches!(self.video, Video::Drained | Video::Gone) {
            self.events.arm_refused(STILL_STOPPING.to_string());
            return;
        }
        let from = self.pacer.next_index();
        self.ring = Some(RingSide {
            ring: ReplayRing::new(RingConfig {
                fps: self.cfg.fps,
                seconds,
                cap_bytes,
            }),
            phase: RingPhase::Arming { from },
            segment: false,
            cut: None,
        });
        self.force = true;
        self.disarming = false;
        tracing::info!(seconds, cap_bytes, "replay is arming");
    }

    pub fn resize(&mut self, seconds: u32, cap_bytes: u64) {
        if let Some(rs) = self.ring.as_mut() {
            rs.ring.resize(seconds, cap_bytes);
        }
    }

    /// Turn the ring off — once a pending Save has been cut from it.
    pub fn disarm(&mut self, now: Duration, out: &mut Out) {
        match self.ring.as_ref() {
            None => {}
            Some(rs) if rs.cut.is_some() => self.disarming = true,
            Some(_) => self.drop_ring(),
        }
        self.settle(now, out);
    }

    /// Cut a clip ending at the frame showing at `at` (seconds on the
    /// pipeline's clock). It is handed over once its audio is in: see
    /// [`Self::poll`].
    pub fn save(&mut self, req: SaveRequest, at: f64, now: Duration) {
        let fps = self.cfg.fps;
        let Some(rs) = self.ring.as_mut() else {
            req.reply.send(Err(REPLAY_OFF.to_string()));
            return;
        };
        if self.disarming || self.leaving {
            // Dropped: the reply says Replay was turned off.
            return;
        }
        if rs.cut.is_some() {
            req.reply.send(Err(STILL_SAVING.to_string()));
            return;
        }
        if !rs.segment {
            req.reply.send(Err(NOTHING_YET.to_string()));
            return;
        }
        // The frame showing at the press is in the clip.
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)] // ≥ 0, a frame count
        let mut end = (at.max(0.0) * f64::from(fps)).floor() as u64 + 1;
        if let Some(f) = rs.ring.frozen() {
            end = end.min(f.video_index);
        }
        let Some(w) = rs.ring.window(end) else {
            req.reply.send(Err(NOTHING_YET.to_string()));
            return;
        };
        rs.ring.pin(Some(w.first_index));
        rs.cut = Some((
            PendingCut {
                window: w,
                registered_at: now,
            },
            req,
        ));
    }

    /// End: a take still open is stopped (its file finished), a pending
    /// Save is cut, then the thread ends.
    pub fn shutdown(&mut self, frame_pending: bool, now: Duration, out: &mut Out) {
        self.leaving = true;
        if let Some(id) = self.take.as_ref().map(|t| t.id) {
            self.stop_take(id, frame_pending, now, out);
        }
        let cut = self.ring.as_ref().is_some_and(|r| r.cut.is_some());
        if (self.take.is_some() || cut) && self.video == Video::Live {
            self.video = Video::Drained;
            out.session.push(SessionAction::Drain);
        }
        self.disarm(now, out);
    }

    /// The take's writer can't keep up: the take ends now, its file kept.
    pub fn take_disk_full(&mut self, now: Duration, out: &mut Out) {
        let newest = self.router.newest();
        self.end_take_now(TakeCut::Disk, newest, now, out);
        self.settle(now, out);
    }

    // ---- each iteration -----------------------------------------------

    /// Deadlines and reports: a take whose end is in (or out of time), a
    /// cut whose audio is in (or out of time), once-a-second progress.
    pub fn poll(&mut self, now: Duration, out: &mut Out) {
        let newest = self.router.newest();
        let audio_in = if self.cfg.with_audio {
            self.emitted
        } else {
            u64::MAX
        };
        let done = self.take.as_ref().is_some_and(|t| {
            t.deadline.is_some_and(|d| {
                let video = if t.video_done {
                    t.core.end().map(|e| e.saturating_sub(1))
                } else {
                    newest
                };
                now >= d || t.core.ready(video, audio_in)
            })
        });
        if done {
            let why = self.take.as_ref().and_then(|t| t.cut);
            self.finish_take(why, now, out);
        }
        let size = self.cfg.size;
        if let Some(rs) = self.ring.as_mut() {
            if let Some((cut, _)) = &rs.cut {
                let state = cut.poll(now, rs.ring.newest_index(), rs.ring.pcm_end_sample());
                if state != CutState::Wait {
                    resolve_cut(rs, size, out);
                }
            }
        }
        if self.disarming && self.ring.as_ref().is_some_and(|r| r.cut.is_none()) {
            self.drop_ring();
        }
        self.report(now);
        self.settle(now, out);
    }

    // ---- inside -------------------------------------------------------

    /// Hold the ring for `reason`, frozen at frame `at`. A ring that hadn't
    /// kept anything yet is turned off instead (the app's table: Arming →
    /// Off), with the same event.
    fn hold(&mut self, reason: HeldReason, detail: Option<String>, at: u64, out: &mut Out) {
        let fps = self.cfg.fps;
        let size = self.cfg.size;
        let Some(rs) = self.ring.as_mut() else {
            return;
        };
        match rs.phase {
            RingPhase::Arming { .. } => {
                resolve_cut(rs, size, out);
                tracing::info!(?reason, "replay stopped before it kept anything");
                self.ring = None;
                self.disarming = false;
                self.events.held(reason, detail);
            }
            RingPhase::Held(HeldReason::ScreenLost | HeldReason::EncoderFailed) => {}
            RingPhase::Held(HeldReason::EngineStopped) if reason == HeldReason::EngineStopped => {}
            // Held for the engine and emptied by a sleep since: nothing to
            // save, and the engine's return can't bring back a lost screen
            // or encoder — off, as a ring that hadn't kept anything is.
            RingPhase::Held(HeldReason::EngineStopped) if !rs.segment => {
                resolve_cut(rs, size, out);
                tracing::info!(?reason, "replay stopped with nothing kept since a sleep");
                self.ring = None;
                self.disarming = false;
                self.events
                    .failed(format!("{EMPTIED_BY_SLEEP} ({reason:?})"));
            }
            RingPhase::Armed | RingPhase::Held(HeldReason::EngineStopped) => {
                rs.ring.freeze(at, sample_of(at, fps));
                // A pending cut ends where the ring does.
                if let (Some((cut, _)), Some(f)) = (rs.cut.as_mut(), rs.ring.frozen()) {
                    if cut.window.end_index > f.video_index
                        && f.video_index > cut.window.first_index
                    {
                        cut.window.end_index = f.video_index;
                        cut.window.end_sample = sample_of(f.video_index, fps);
                    }
                }
                rs.phase = RingPhase::Held(reason);
                tracing::info!(?reason, detail = detail.as_deref(), "replay is held");
                self.events.held(reason, detail);
            }
        }
    }

    fn drop_ring(&mut self) {
        if let Some(rs) = self.ring.take() {
            debug_assert!(rs.cut.is_none());
            tracing::info!(stats = ?rs.ring.stats(), "replay is off");
        }
        self.disarming = false;
    }

    /// End the take now: what it has, padded to its end. Sleep and a full
    /// disk can't wait.
    fn end_take_now(&mut self, why: TakeCut, last: Option<u64>, now: Duration, out: &mut Out) {
        let Some(t) = self.take.as_mut() else {
            return;
        };
        t.cut.get_or_insert(why);
        t.core.cut_short(last);
        self.finish_take(Some(why), now, out);
    }

    /// End the take at frame `last` (inclusive), waiting as a stop does for
    /// its audio; its video is complete.
    fn end_take_waiting(&mut self, why: TakeCut, last: Option<u64>, now: Duration) {
        let Some(t) = self.take.as_mut() else {
            return;
        };
        t.core.cut_short(last);
        t.cut.get_or_insert(why);
        t.video_done = true;
        if t.deadline.is_none() {
            t.deadline = Some(now + END_WAIT);
        }
    }

    fn finish_take(&mut self, why: Option<TakeCut>, now: Duration, out: &mut Out) {
        let Some(mut t) = self.take.take() else {
            return;
        };
        let newest = if t.video_done {
            t.core.end().map(|e| e.saturating_sub(1))
        } else {
            self.router.newest()
        };
        let first = t.core.first_frame();
        let end = t.core.finish(newest, &mut out.mux);
        let seconds = seconds_between(first, end, self.cfg.fps);
        let how = match why {
            None => TakeEnd::Clean,
            Some(cut) => TakeEnd::Cut(cut.message(seconds, end.is_some())),
        };
        let mut stats = self.take_stats(&t, now);
        if let TakeEnd::Cut(text) = &how {
            stats.cut = Some(text.clone());
        }
        tracing::info!(take = t.id, ?how, frames = t.core.written(), "take ended");
        out.mux.push(MuxMsg::End {
            how,
            stats: Box::new(stats),
        });
    }

    /// The take's counts so far: counters since it started, gauges as they
    /// are (a drift is not a difference).
    fn take_stats(&self, t: &OpenTake, now: Duration) -> RecordStats {
        let fps = self.cfg.fps;
        let tl = self.timeline.stats();
        let b = t.base.timeline;
        #[allow(clippy::cast_precision_loss)] // ticks: exact far past any take
        let elapsed = t.core.first_frame().map_or(0.0, |k0| {
            (now.as_secs_f64() - crate::pacer::pts_at(k0, fps) as f64 / 1e7).max(0.0)
        });
        let audio = self.cfg.with_audio.then(|| AudioStats {
            timeline: TimelineStats {
                steps: tl.steps.saturating_sub(b.steps),
                waited_frames: tl.waited_frames.saturating_sub(b.waited_frames),
                drift_ppm: tl.drift_ppm,
                voice_delay_ms: tl.voice_delay_ms,
                desktop_drift_ppm: tl.desktop_drift_ppm,
                desktop_runs: tl.desktop_runs.saturating_sub(b.desktop_runs),
                desktop_gaps: tl.desktop_gaps.saturating_sub(b.desktop_gaps),
                desktop_gap_seconds: (tl.desktop_gap_seconds - b.desktop_gap_seconds).max(0.0),
                desktop_late_frames: tl.desktop_late_frames.saturating_sub(b.desktop_late_frames),
                desktop_waited_frames: tl
                    .desktop_waited_frames
                    .saturating_sub(b.desktop_waited_frames),
                desktop_quiet_seconds: tl.desktop_quiet_seconds.map(|q| q.min(elapsed)),
            },
            ..AudioStats::default()
        });
        RecordStats {
            frames_arrived: self.frames.arrived.saturating_sub(t.base.frames.arrived),
            frames_written: t.core.written(),
            frames_repeated: self.frames.repeated.saturating_sub(t.base.frames.repeated),
            frames_dropped: self.dropped_total().saturating_sub(t.base.dropped) + t.extra_dropped,
            width: self.cfg.size.0,
            height: self.cfg.size.1,
            encoder: self.cfg.encoder.clone(),
            hardware: self.cfg.hardware,
            audio,
            cut: None,
        }
    }

    fn report(&mut self, now: Duration) {
        if self.last_report.is_some_and(|t| now < t + REPORT_EVERY) {
            return;
        }
        self.last_report = Some(now);
        if let Some(t) = &self.take {
            if t.core.is_open() && t.deadline.is_none() {
                t.events.progress(&self.take_stats(t, now));
            }
        }
        if let Some(rs) = &self.ring {
            if !matches!(rs.phase, RingPhase::Arming { .. }) {
                let s = rs.ring.stats();
                self.events.ring(RingStatus {
                    buffered_ms: if rs.segment { s.buffered_ms } else { 0 },
                    bytes: s.bytes,
                    cap_bytes: s.cap_bytes,
                    seconds: rs.ring.config().seconds,
                });
            }
        }
    }

    /// What is still needed: with no take and no ring, nothing — the thread
    /// ends; with no take and a ring that can't take in pictures, not the
    /// session.
    fn settle(&mut self, _now: Duration, out: &mut Out) {
        if self.take.is_some() {
            return;
        }
        match self.ring.as_ref().map(|r| r.phase) {
            None => out.exit = true,
            Some(RingPhase::Held(HeldReason::ScreenLost | HeldReason::EncoderFailed))
                if matches!(self.video, Video::Live | Video::Drained) =>
            {
                self.video = Video::Gone;
                out.session.push(SessionAction::Release { quiesce: true });
            }
            Some(_) => {}
        }
    }
}

/// Start a segment on the keyframe `index`, its audio seeded from the tail
/// (the timeline can be ahead of the picture by the A/V offset).
fn start_segment(
    rs: &mut RingSide,
    seq: Option<Arc<[u8]>>,
    index: u64,
    fps: u32,
    tail: &BlockTail,
) {
    debug_assert!(
        rs.cut.is_none(),
        "a cut is resolved before a segment starts"
    );
    rs.ring.start_segment(seq, sample_of(index, fps));
    for b in tail.blocks() {
        rs.ring.push_pcm(frame_of(b.pts), &b.samples);
    }
    rs.segment = true;
}

/// Cut a pending Save now — ready or not: what audio hasn't come is
/// silence — and hand it over.
fn resolve_cut(rs: &mut RingSide, size: (u32, u32), out: &mut Out) {
    let Some((cut, req)) = rs.cut.take() else {
        return;
    };
    let fps = rs.ring.config().fps;
    let mut w = cut.window;
    // Frames that never came out are not in the clip.
    if let Some(n) = rs.ring.newest_index() {
        if n + 1 < w.end_index && n >= w.first_index {
            w.end_index = n + 1;
            w.end_sample = sample_of(n + 1, fps);
        }
    }
    let mut snap = rs.ring.snapshot(&w);
    rs.ring.pin(None);
    snap.size = size;
    if snap.padded_frames > 0 {
        tracing::info!(
            frames = snap.padded_frames,
            "a replay's last audio was silence"
        );
    }
    req.size
        .store(snap.bytes(), std::sync::atomic::Ordering::Release);
    out.saves.push((snap, req));
}

#[cfg(test)]
mod tests {
    use super::{EncoderFault, MuxMsg, OutputRouter, TakeCore};
    use crate::replay::{sample_of, AccessUnit};
    use crate::take_cut::BlockTail;
    use crate::timeline::{AudioBlock, BLOCK_FRAMES};
    use std::sync::Arc;
    use std::time::Duration;

    const FPS: u32 = 30;

    /// An access unit: an IDR (AUD, SPS, PPS, IDR) or a P-frame (AUD, P).
    fn au_bytes(key: bool) -> Arc<[u8]> {
        let mut v = vec![0, 0, 0, 1, 0x09, 0xf0];
        if key {
            v.extend([
                0, 0, 0, 1, 0x67, 1, 2, 3, 0, 0, 0, 1, 0x68, 4, 5, 0, 0, 0, 1, 0x65, 9,
            ]);
        } else {
            v.extend([0, 0, 0, 1, 0x41, 9]);
        }
        Arc::from(v)
    }

    fn pts(i: u64) -> i64 {
        i64::try_from(crate::pacer::pts_at(i, FPS)).unwrap()
    }

    fn block(first_sample: u64, value: i16) -> AudioBlock {
        let ticks = |s: u64| s * 10_000_000 / 48_000;
        AudioBlock {
            pts: ticks(first_sample),
            duration: ticks(first_sample + BLOCK_FRAMES as u64) - ticks(first_sample),
            samples: vec![value; 2 * BLOCK_FRAMES],
        }
    }

    /// Outputs land on the frames they were submitted as; a re-stamped one
    /// and a B-frame are faults; a late forced keyframe is reported, not a
    /// fault.
    #[test]
    fn outputs_are_placed_and_untrustworthy_encoders_caught() {
        let mut r = OutputRouter::new(FPS, 60);
        for i in 0..3 {
            r.on_submitted(i, pts(i), i == 0, Duration::ZERO);
        }
        let a = r.route(pts(0), Some(pts(0)), au_bytes(true)).unwrap();
        assert_eq!((a.au.index, a.au.key), (0, true));
        // Frame 1 never comes out: 2's output passes over it.
        assert_eq!(r.route(pts(2), None, au_bytes(false)).unwrap().au.index, 2);
        assert_eq!(r.encoder_dropped(), 1);
        assert_eq!(r.newest(), Some(2));
        assert_eq!(
            r.route(pts(7), None, au_bytes(false)).unwrap_err(),
            EncoderFault::Restamped { pts: pts(7) }
        );
        r.on_submitted(3, pts(3), false, Duration::ZERO);
        assert_eq!(
            r.route(pts(3), Some(pts(2)), au_bytes(false)).unwrap_err(),
            EncoderFault::BFrames { pts: pts(3) }
        );
    }

    /// A take opens on its first keyframe at or after the frame Record was
    /// pressed on — a later keyframe moves its start, an earlier one is
    /// never used — and its audio starts on that keyframe's sample.
    #[test]
    fn a_take_opens_on_its_own_first_keyframe() {
        let mut tail = BlockTail::default();
        // Audio from sample 0, value = the block number, already handed out.
        for b in 0..40u64 {
            tail.push(block(b * BLOCK_FRAMES as u64, i16::try_from(b).unwrap()));
        }
        let mut t = TakeCore::new(10, FPS, au_bytes(true));
        let mut out = Vec::new();
        // A keyframe before k0, and P-frames at and after it: nothing.
        t.on_au(&AccessUnit::new(5, FPS, au_bytes(true)), &tail, &mut out);
        t.on_au(&AccessUnit::new(10, FPS, au_bytes(false)), &tail, &mut out);
        t.on_au(&AccessUnit::new(11, FPS, au_bytes(false)), &tail, &mut out);
        assert!(out.is_empty() && !t.is_open());
        // The forced keyframe came late, at 12: the take starts there.
        t.on_au(&AccessUnit::new(12, FPS, au_bytes(true)), &tail, &mut out);
        assert!(matches!(out[0], MuxMsg::Open { .. }));
        assert!(matches!(&out[1], MuxMsg::Video(au) if au.index == 12));
        assert_eq!(t.first_frame(), Some(12));
        out.clear();
        // The video goes on past the next block…
        for i in 13..=26 {
            t.on_au(&AccessUnit::new(i, FPS, au_bytes(false)), &tail, &mut out);
        }
        // …and with it the take's audio, which starts on sample_of(12).
        t.on_block(&block(40 * BLOCK_FRAMES as u64, 40), &mut out);
        let first = out.iter().find_map(|m| match m {
            MuxMsg::Audio(b) => Some(b.clone()),
            _ => None,
        });
        let first = first.expect("the take's first block");
        assert_eq!(first.pts, 0, "timed from the take's start");
        let s0 = sample_of(12, FPS);
        let expect = i16::try_from(s0 / BLOCK_FRAMES as u64).unwrap();
        assert_eq!(
            first.samples[0], expect,
            "the tail's sample at the keyframe"
        );
        assert_eq!(t.lead_silence(), 0);
    }

    /// A take ends after its last submitted frame; it is ready once that
    /// frame and the audio to it are in; finishing pads the audio, once. A
    /// take that never opened has no file.
    #[test]
    fn a_take_ends_after_its_last_submitted_frame() {
        let tail = BlockTail::default();
        let mut t = TakeCore::new(0, FPS, au_bytes(true));
        let mut out = Vec::new();
        for i in 0..30u64 {
            t.on_au(&AccessUnit::new(i, FPS, au_bytes(i == 0)), &tail, &mut out);
        }
        t.stop(Some(32));
        assert_eq!(t.end(), Some(33));
        assert!(
            !t.ready(Some(29), sample_of(33, FPS)),
            "frames 30–32 still in the encoder"
        );
        assert!(
            !t.ready(Some(32), sample_of(33, FPS) - 1),
            "the audio isn't there"
        );
        assert!(t.ready(Some(32), sample_of(33, FPS)));
        // Frames past the end are not the take's.
        out.clear();
        t.on_au(&AccessUnit::new(33, FPS, au_bytes(false)), &tail, &mut out);
        assert!(out.is_empty());
        // Out of time with frames 30–32 lost: the take ends at 30.
        let end = t.finish(Some(29), &mut out);
        assert_eq!(end, Some(30));
        let audio: u64 = out
            .iter()
            .filter_map(|m| match m {
                MuxMsg::Audio(b) => Some((b.samples.len() / 2) as u64),
                _ => None,
            })
            .sum();
        assert_eq!(
            audio,
            sample_of(30, FPS).div_ceil(BLOCK_FRAMES as u64) * BLOCK_FRAMES as u64,
            "padded to whole blocks, to the end"
        );
        assert!(t.finish(Some(40), &mut out).is_none(), "once");

        // Audio goes no further than the video: blocks past where the take
        // finally ends never go out, however long the wait for frames was.
        let mut t = TakeCore::new(0, FPS, au_bytes(true));
        let mut out = Vec::new();
        t.on_au(&AccessUnit::new(0, FPS, au_bytes(true)), &tail, &mut out);
        t.stop(Some(59)); // two seconds submitted
        for b in 0..200u64 {
            t.on_block(&block(b * BLOCK_FRAMES as u64, 1), &mut out);
        }
        let sent: u64 = out
            .iter()
            .filter_map(|m| match m {
                MuxMsg::Audio(b) => Some((b.samples.len() / 2) as u64),
                _ => None,
            })
            .sum();
        assert!(
            sent <= sample_of(1, FPS).div_ceil(BLOCK_FRAMES as u64) * BLOCK_FRAMES as u64,
            "only the blocks under the one frame out: {sent}"
        );
        // Only frame 0 ever came out: the take is one frame long.
        t.finish(Some(0), &mut out);
        let audio: u64 = out
            .iter()
            .filter_map(|m| match m {
                MuxMsg::Audio(b) => Some((b.samples.len() / 2) as u64),
                _ => None,
            })
            .sum();
        assert_eq!(
            audio,
            sample_of(1, FPS).div_ceil(BLOCK_FRAMES as u64) * BLOCK_FRAMES as u64,
            "one frame's audio, padded to whole blocks — not two seconds of it"
        );

        let mut never = TakeCore::new(5, FPS, au_bytes(true));
        never.stop(None);
        let mut out = Vec::new();
        assert!(never.finish(None, &mut out).is_none());
        assert!(out.is_empty(), "no file");
    }
}

/// The pipeline's decisions driven end to end with a pretend encoder, a
/// pretend engine and a pretend shell (design tests 28, 45–50, 61, 62).
#[cfg(test)]
#[allow(clippy::struct_excessive_bools, clippy::unchecked_time_subtraction)] // a pretend shell's switches; test clocks only move forward
mod core_tests {
    use super::{
        CoreConfig, EncodedUnit, EncoderEvent, MuxMsg, Out, PipelineCore, ReplayView,
        SessionAction, TakeEnd,
    };
    use crate::pipeline::{
        EngineSignal, HeldReason, PipelineEvents, RingStatus, SaveReply, SaveRequest, SavedClip,
        TakeEvents, SAVE_DROPPED,
    };
    use crate::replay::{budget, sample_of, Snapshot};
    use crate::take::RecordStats;
    use crate::take_cut::TAIL_FRAMES;
    use crate::timeline::tests::{tone, Engine};
    use crate::timeline::{TrackOptions, BLOCK_FRAMES, OUT_RATE};
    use std::collections::VecDeque;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicBool, AtomicU64};
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    const HEADER_A: &[u8] = &[0, 0, 0, 1, 0x67, 1, 2, 3, 0, 0, 0, 1, 0x68, 4, 5];
    const HEADER_B: &[u8] = &[0, 0, 0, 1, 0x67, 7, 7, 7, 0, 0, 0, 1, 0x68, 4, 5];
    const HEADER_C: &[u8] = &[0, 0, 0, 1, 0x67, 9, 9, 9, 0, 0, 0, 1, 0x68, 4, 5];

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    /// What the pipeline told the app.
    #[derive(Default)]
    struct Events(Mutex<Vec<String>>);

    impl Events {
        fn push(&self, s: impl Into<String>) {
            self.0.lock().unwrap().push(s.into());
        }
        fn count(&self, what: &str) -> usize {
            self.0.lock().unwrap().iter().filter(|s| *s == what).count()
        }
        fn has(&self, what: &str) -> bool {
            self.count(what) > 0
        }
    }

    impl PipelineEvents for Events {
        fn ring_ready(&self) {
            self.push("ready");
        }
        fn ring(&self, _status: RingStatus) {}
        fn held(&self, reason: HeldReason, _detail: Option<String>) {
            self.push(format!("held:{reason:?}"));
        }
        fn resumed(&self) {
            self.push("resumed");
        }
        fn rebuilding(&self) {
            self.push("rebuilding");
        }
        fn arm_refused(&self, _why: String) {
            self.push("arm-refused");
        }
        fn direct_failed(&self, _why: String) {
            self.push("direct-failed");
        }
        fn failed(&self, _why: String) {
            self.push("failed");
        }
        fn ended(&self, _error: Option<String>) {}
    }

    struct Quiet;

    impl TakeEvents for Quiet {
        fn started(&self) {}
        fn progress(&self, _stats: &RecordStats) {}
        fn finished(&self, _outcome: Result<PathBuf, String>, _stats: RecordStats) {}
    }

    /// A pretend encoder (outputs one step after input, an IDR when forced
    /// or every GOP), a pretend engine, and a shell that does what the core
    /// asks.
    struct Fake {
        core: PipelineCore,
        events: Arc<Events>,
        now: Duration,
        /// Submitted, not yet out: (index, pts, forced).
        queue: VecDeque<(u64, i64, bool)>,
        /// Outputs come out.
        answering: bool,
        /// Input is asked for.
        asking: bool,
        /// A drain hands back what was in flight.
        drain_gives: bool,
        gop: u64,
        last_idr: Option<u64>,
        /// Forced frames come out as keyframes.
        honour_force: bool,
        /// The SPS/PPS every IDR carries.
        inband: &'static [u8],
        received: Option<u64>,
        engine: Engine,
        /// Buffers keep coming (false: the engine went quiet, unannounced).
        audio_on: bool,
        signal: EngineSignal,
        /// Added to every chunk's time: the app's A/V offset.
        offset: f64,
        voice: Box<dyn Fn(f64) -> f32>,
        sb: Box<dyn Fn(f64) -> f32>,
        pending_since: Option<Duration>,
        mux: Vec<MuxMsg>,
        clips: Vec<Snapshot>,
        actions: Vec<SessionAction>,
        exited: bool,
        replies: Arc<Mutex<Vec<Result<u64, String>>>>,
    }

    impl Fake {
        fn new(fps: u32, track: TrackOptions) -> Self {
            let events = Arc::new(Events::default());
            let core = PipelineCore::new(
                CoreConfig {
                    fps,
                    size: (320, 240),
                    track,
                    with_audio: true,
                    encoder: Some("pretend".into()),
                    hardware: true,
                    seq: Some(Arc::from(HEADER_A)),
                },
                Arc::clone(&events) as Arc<dyn PipelineEvents>,
            );
            Self {
                core,
                events,
                now: Duration::ZERO,
                queue: VecDeque::new(),
                answering: true,
                asking: true,
                drain_gives: true,
                gop: 2 * u64::from(fps),
                last_idr: None,
                honour_force: true,
                inband: HEADER_A,
                received: None,
                engine: Engine::new(48_000, 0.0),
                audio_on: true,
                signal: EngineSignal::Running,
                offset: 0.0,
                voice: Box::new(tone(300.0, 0.4)),
                sb: Box::new(|_| 0.0),
                pending_since: None,
                mux: Vec::new(),
                clips: Vec::new(),
                actions: Vec::new(),
                exited: false,
                replies: Arc::new(Mutex::new(Vec::new())),
            }
        }

        fn bytes(&mut self, index: u64, forced: bool) -> Arc<[u8]> {
            let key = (forced && self.honour_force)
                || self.last_idr.is_none_or(|l| index >= l + self.gop);
            let mut v = vec![0, 0, 0, 1, 0x09, 0xf0];
            if key {
                self.last_idr = Some(index);
                v.extend_from_slice(self.inband);
                v.extend([0, 0, 0, 1, 0x65, 0x88]);
            } else {
                v.extend([0, 0, 0, 1, 0x41, 0x9a]);
            }
            Arc::from(v)
        }

        fn unit(&mut self, (index, pts, forced): (u64, i64, bool)) -> EncoderEvent {
            self.received = Some(self.received.map_or(index, |r| r.max(index)));
            EncoderEvent::Unit(EncodedUnit {
                pts,
                dur: 0,
                dts: None,
                clean_point: None,
                data: self.bytes(index, forced),
            })
        }

        /// One turn of the loop, `dt` after the last.
        fn step(&mut self, dt: Duration) {
            self.now += dt;
            let now = self.now;
            let mut out = Out::default();
            let mut evs = Vec::new();
            while self.answering {
                let Some(q) = self.queue.pop_front() else {
                    break;
                };
                evs.push(self.unit(q));
            }
            self.core.on_encoder(&mut evs, now, &mut out);
            self.core.check_liveness(now, self.pending_since, &mut out);
            if let Some(due) = self.core.tick(now) {
                if self.asking {
                    let pts = i64::try_from(due.pts).unwrap();
                    self.core.submitted(due.index, pts, due.force, now);
                    self.queue.push_back((due.index, pts, due.force));
                    self.pending_since = None;
                } else {
                    self.pending_since.get_or_insert(now);
                }
            }
            let mut chunks = Vec::new();
            if self.audio_on && self.signal == EngineSignal::Running {
                let until = now.as_secs_f64();
                while self.engine.time(self.engine.index) - self.offset <= until {
                    let mut c = self.engine.chunk(&*self.sb, &*self.voice);
                    c.at += self.offset;
                    chunks.push(c);
                }
            }
            self.core.on_audio(&chunks, &[], self.signal, now, &mut out);
            self.core.poll(now, &mut out);
            self.absorb(out);
        }

        fn run(&mut self, d: Duration) {
            let end = self.now + d;
            while self.now < end {
                self.step(ms(10));
            }
        }

        /// Do as the core asks.
        fn absorb(&mut self, out: Out) {
            self.mux.extend(out.mux);
            for (snap, req) in out.saves {
                let bytes = snap.bytes();
                req.reply.send(Ok(SavedClip {
                    path: req.name.clone().into(),
                    duration_ms: snap.duration_ms(),
                    bytes,
                }));
                self.clips.push(snap);
            }
            for a in out.session {
                self.actions.push(a);
                let mut more = Out::default();
                match a {
                    SessionAction::Drain => {
                        let mut evs = Vec::new();
                        while let Some(q) = self.queue.pop_front() {
                            if self.drain_gives {
                                evs.push(self.unit(q));
                            }
                        }
                        self.core.on_encoder(&mut evs, self.now, &mut more);
                        self.answering = false;
                        self.asking = false;
                    }
                    SessionAction::Release { .. } | SessionAction::Rebuild => {
                        self.queue.clear();
                        self.answering = false;
                        self.asking = false;
                    }
                    SessionAction::RebuildEncoder => {
                        self.queue.clear();
                        self.last_idr = None;
                        self.answering = true;
                        self.asking = true;
                        self.core.encoder_rebuilt(Some(Arc::from(self.inband)));
                    }
                }
                self.absorb(more);
            }
            self.exited |= out.exit;
        }

        fn request(&self) -> SaveRequest {
            let replies = Arc::clone(&self.replies);
            SaveRequest {
                at: Instant::now(),
                dir: PathBuf::from("recordings"),
                name: "divora-replay-test.mp4".into(),
                reply: SaveReply::new(move |r| {
                    replies.lock().unwrap().push(r.map(|c| c.duration_ms));
                }),
                size: Arc::new(AtomicU64::new(0)),
                cancel: Arc::new(AtomicBool::new(false)),
            }
        }

        /// Press Save now.
        fn save(&mut self) {
            let req = self.request();
            self.core.save(req, self.now.as_secs_f64(), self.now);
        }

        fn arm(&mut self, seconds: u32) {
            let mut out = Out::default();
            self.core.arm(
                seconds,
                budget::cap_bytes(budget::estimate_bytes(seconds, 1_000_000)),
                &mut out,
            );
            self.absorb(out);
        }

        fn start_take(&mut self, id: u64, stems: bool) {
            assert!(self.core.take_refusal().is_none());
            self.core.start_take(id, stems, Arc::new(Quiet));
        }

        fn stop_take(&mut self, id: u64) {
            let mut out = Out::default();
            self.core.stop_take(id, false, self.now, &mut out);
            self.absorb(out);
        }

        fn ends(&self) -> Vec<(TakeEnd, RecordStats)> {
            self.mux
                .iter()
                .filter_map(|m| match m {
                    MuxMsg::End { how, stats } => Some((how.clone(), (**stats).clone())),
                    _ => None,
                })
                .collect()
        }

        fn take_video(&self) -> Vec<u64> {
            self.mux
                .iter()
                .filter_map(|m| match m {
                    MuxMsg::Video(au) => Some(au.index),
                    _ => None,
                })
                .collect()
        }

        fn take_audio(&self) -> Vec<i16> {
            self.mux
                .iter()
                .filter_map(|m| match m {
                    MuxMsg::Audio(b) => Some(b.samples.clone()),
                    _ => None,
                })
                .flatten()
                .collect()
        }

        fn replies(&self) -> Vec<Result<u64, String>> {
            self.replies.lock().unwrap().clone()
        }
    }

    #[allow(clippy::cast_precision_loss)] // a test signal's length
    fn rms(s: &[i16]) -> f64 {
        let n = s.len().max(1) as f64;
        (s.iter().map(|&x| f64::from(x).powi(2)).sum::<f64>() / n).sqrt()
    }

    /// Frames of `n` audio frames padded to whole blocks.
    fn blocks_of(n: u64) -> u64 {
        n.div_ceil(BLOCK_FRAMES as u64) * BLOCK_FRAMES as u64
    }

    /// Test 28: with no take open the stems never pile up — the tail holds
    /// about 600 ms of them, whatever the run's length — and a stemmed take
    /// started mid-run has them from its first sample.
    #[test]
    fn stems_are_drained_every_pump() {
        let mut f = Fake::new(30, TrackOptions::default());
        let mut most = 0;
        for _ in 0..(10 * 60 * 1000 / 20) {
            f.step(ms(20));
            most = most.max(f.core.tail().stem_frames());
            assert_eq!(f.core.stems_waiting(), 0, "stems left in the timeline");
        }
        assert!(
            most <= TAIL_FRAMES + BLOCK_FRAMES as u64,
            "the stems held {most} frames"
        );
        f.start_take(1, true);
        f.run(Duration::from_secs(2));
        let stems: Vec<_> = f
            .mux
            .iter()
            .filter_map(|m| match m {
                MuxMsg::Stems(b) => Some(b.clone()),
                _ => None,
            })
            .flatten()
            .collect();
        let first = stems.first().expect("the take's stems");
        assert_eq!(first.pts, 0, "from the take's first sample");
        assert!(
            first.voice[..64].iter().any(|v| v.abs() > 0.05),
            "the voice is there from the first sample, not a block later"
        );
        let audio_frames = (f.take_audio().len() / 2) as u64;
        let stem_frames: u64 = stems.iter().map(|s| s.voice.len() as u64).sum();
        assert_eq!(stem_frames, audio_frames, "stems and the mix in step");
    }

    /// Test 45: an encoder that goes quiet — no outputs, or no requests
    /// for input — is caught within a second: the ring is held, a take ends
    /// with its file kept.
    #[test]
    fn a_quiet_encoder_faults_within_a_second() {
        for no_input in [false, true] {
            let mut f = Fake::new(30, TrackOptions::default());
            f.arm(30);
            f.start_take(1, false);
            f.run(Duration::from_secs(4));
            assert_eq!(f.core.replay(), Some(ReplayView::Armed));
            let quiet_at = f.now;
            if no_input {
                f.asking = false;
            } else {
                f.answering = false;
            }
            while !f.events.has("held:EncoderFailed") {
                f.step(ms(10));
                assert!(
                    f.now - quiet_at < ms(1_200),
                    "no fault after {:?}",
                    f.now - quiet_at
                );
            }
            assert!(
                f.now - quiet_at <= ms(1_100),
                "faulted after {:?}",
                f.now - quiet_at
            );
            assert_eq!(
                f.core.replay(),
                Some(ReplayView::Held(HeldReason::EncoderFailed))
            );
            f.run(Duration::from_secs(2));
            let ends = f.ends();
            assert_eq!(ends.len(), 1);
            let TakeEnd::Cut(why) = &ends[0].0 else {
                panic!("a clean end");
            };
            assert!(
                why.starts_with("The video encoder stopped working")
                    && why.ends_with("What was recorded is saved."),
                "{why}"
            );
            assert!(
                !f.events.has("direct-failed"),
                "a stall says nothing of the next launch"
            );
        }
    }

    /// Test 46: a stopped take ends within 1.5 s however little comes after
    /// the stop — the encoder gone, no more outputs, the engine stopped —
    /// exactly once, its video ending after the last frame received and its
    /// audio padded to there.
    #[test]
    fn a_take_always_ends_within_its_deadline() {
        for case in 0..3 {
            let mut f = Fake::new(30, TrackOptions::default());
            f.start_take(1, false);
            f.run(Duration::from_secs(3));
            match case {
                // The encoder is dropped: nothing in flight comes back.
                0 => f.drain_gives = false,
                // It just stops answering.
                1 => f.answering = false,
                // The engine stops with the take.
                _ => f.signal = EngineSignal::Stopped,
            }
            let stopped = f.now;
            f.stop_take(1);
            while f.ends().is_empty() {
                f.step(ms(10));
                assert!(f.now - stopped <= ms(1_510), "case {case}: no end in time");
            }
            f.run(Duration::from_secs(2));
            let ends = f.ends();
            assert_eq!(ends.len(), 1, "case {case}: one end");
            assert_eq!(ends[0].0, TakeEnd::Clean);
            let video = f.take_video();
            let k0 = video[0];
            let last = *video.last().unwrap();
            assert_eq!(
                Some(last),
                f.received,
                "case {case}: up to the last frame received"
            );
            let frames = sample_of(last + 1, 30) - sample_of(k0, 30);
            assert_eq!(
                (f.take_audio().len() / 2) as u64,
                blocks_of(frames),
                "case {case}: audio padded to the end"
            );
            assert!(
                f.exited,
                "case {case}: a take-only pipeline ends with its take"
            );
        }
    }

    /// Test 47: a Save waiting for its audio is answered exactly once
    /// whatever comes first — and the sleep, the engine's return and Disarm
    /// answer with the clip.
    #[test]
    fn every_save_answers_once() {
        for case in [
            "disarm", "shutdown", "sleep", "engine", "header", "failure", "dead",
        ] {
            let mut f = Fake::new(30, TrackOptions::default());
            f.arm(30);
            f.run(Duration::from_secs(4));
            // The audio stops coming, so the cut waits for it.
            f.audio_on = false;
            f.step(ms(10));
            f.save();
            let mut out = Out::default();
            let now = f.now;
            match case {
                "disarm" => f.core.disarm(now, &mut out),
                "shutdown" => f.core.shutdown(false, now, &mut out),
                "sleep" => f.core.on_sleep(now, &mut out),
                "engine" => {
                    f.signal = EngineSignal::Stopped;
                    f.step(ms(10));
                    f.signal = EngineSignal::Running;
                }
                "header" => {
                    f.inband = HEADER_B;
                    f.gop = 1;
                }
                "failure" => f.core.encoder_failed("gone", false, false, now, &mut out),
                _ => {
                    // The pipeline has gone: the app drops the Save it hands back.
                    f.core.disarm(now, &mut out);
                    f.absorb(out);
                    f.run(Duration::from_secs(2));
                    assert!(f.exited);
                    let replies = Arc::clone(&f.replies);
                    drop(SaveReply::new(move |r| {
                        replies.lock().unwrap().push(r.map(|c| c.duration_ms));
                    }));
                    out = Out::default();
                }
            }
            f.absorb(out);
            f.run(Duration::from_secs(2));
            let replies = f.replies();
            if case == "dead" {
                assert_eq!(replies.len(), 2, "{case}");
                assert_eq!(replies[1], Err(SAVE_DROPPED.to_string()), "{case}");
                continue;
            }
            assert_eq!(replies.len(), 1, "{case}: {replies:?}");
            assert!(replies[0].is_ok(), "{case}: {replies:?}");
            if case == "sleep" {
                assert!(f.clips[0].padded_frames > 0, "the missing audio is silence");
            }
        }
    }

    /// Test 48: with the A/V offset ahead of the voice's delay, the audio
    /// for a take's first frame was handed out before the take began; the
    /// tail still has it, so the take has sound from frame 0, in its track
    /// and its voice stem.
    #[test]
    fn a_take_on_an_offset_pipeline_has_its_head() {
        let mut f = Fake::new(30, TrackOptions::default());
        f.offset = 0.3;
        f.engine.wait = 0.05;
        f.arm(30);
        f.run(Duration::from_secs(10));
        f.start_take(1, true);
        f.run(Duration::from_secs(2));
        let audio = f.take_audio();
        assert!(
            rms(&audio[..2 * 256]) > 1_000.0,
            "sound from the first sample"
        );
        let stem = f
            .mux
            .iter()
            .find_map(|m| match m {
                MuxMsg::Stems(b) => b.first().cloned(),
                _ => None,
            })
            .expect("a voice stem");
        assert!(stem.voice[..256].iter().any(|v| v.abs() > 0.05));
    }

    /// Test 49: a take on a running pipeline reports the pipeline's drift
    /// as it is (a gauge), and its own counts (counters since it began).
    #[test]
    fn take_stats_pass_gauges_through() {
        let mut f = Fake::new(
            30,
            TrackOptions {
                desktop_gain_db: Some(0.0),
                stems: false,
            },
        );
        f.engine.ppm = 80.0;
        for _ in 0..6_000 {
            f.step(ms(10));
            f.core.counters().arrived += 1;
        }
        f.start_take(1, false);
        for _ in 0..500 {
            f.step(ms(10));
            f.core.counters().arrived += 1;
        }
        f.stop_take(1);
        f.run(Duration::from_secs(2));
        let (_, stats) = f.ends().pop().unwrap();
        let a = stats.audio.unwrap();
        assert!(
            (a.timeline.drift_ppm.abs() - 80.0).abs() < 5.0,
            "drift {} ppm",
            a.timeline.drift_ppm
        );
        assert!(
            stats.frames_arrived <= 520,
            "arrived counts the take's, not the run's: {}",
            stats.frames_arrived
        );
        let quiet = a.timeline.desktop_quiet_seconds.unwrap();
        assert!(quiet <= 5.5, "quiet for {quiet} s in a 5 s take");
    }

    /// Test 50: across a sleep nothing is pumped on the clock that counted
    /// it — the take ends at the jump with a few blocks, a pending Save is
    /// cut there, and chunks from after the jump never reach the timeline.
    #[test]
    fn a_sleep_never_floods_the_timeline() {
        let mut f = Fake::new(30, TrackOptions::default());
        f.arm(30);
        f.start_take(1, false);
        f.run(Duration::from_secs(2));
        f.save();
        let t_jump = f.now;
        let emitted = f.core.emitted();
        let mut out = Out::default();
        f.core.on_sleep(t_jump, &mut out);
        f.absorb(out);
        assert!(f.actions.contains(&SessionAction::Rebuild));
        // Eight hours later, before any picture: a chunk of the same epoch.
        let later = t_jump + Duration::from_secs(8 * 3600);
        f.engine.t0 += 8.0 * 3600.0;
        let c = f.engine.chunk(tone(300.0, 0.4), tone(300.0, 0.4));
        let mut out = Out::default();
        f.core
            .on_audio(&[c], &[], EngineSignal::Running, later, &mut out);
        f.core.poll(later, &mut out);
        f.absorb(out);
        assert!(
            f.core.emitted() <= emitted + 2 * u64::from(OUT_RATE),
            "pumped past the jump: {} frames",
            f.core.emitted()
        );
        let ends = f.ends();
        assert_eq!(ends.len(), 1);
        assert!(matches!(&ends[0].0, TakeEnd::Cut(w) if w.starts_with("The PC went to sleep")));
        let end = f.take_video().last().unwrap() + 1;
        let frames = (f.take_audio().len() / 2) as u64;
        assert!(
            frames <= blocks_of(sample_of(end, 30)) + BLOCK_FRAMES as u64,
            "{frames} frames for a {end}-frame take"
        );
        assert_eq!(
            f.replies().len(),
            1,
            "the Save pressed before the sleep is cut"
        );
        assert_eq!(f.core.replay(), Some(ReplayView::Arming));
        assert!(f.events.has("rebuilding"));
        // The picture is back: a new clock, the ring arms again.
        f.core.restarted(Some(Arc::from(HEADER_A)), (320, 240));
        f.now = Duration::ZERO;
        f.engine = Engine::new(48_000, 0.0);
        f.answering = true;
        f.asking = true;
        f.last_idr = None;
        f.run(Duration::from_secs(1));
        assert_eq!(f.core.replay(), Some(ReplayView::Armed));
    }

    /// Test 61: the encoder restarting itself is never renegotiated: a take
    /// ends ("restarted"), the encoder is rebuilt, and the ring starts over
    /// at the new encoder's first keyframe.
    #[test]
    fn a_stream_change_rebuilds() {
        let mut f = Fake::new(30, TrackOptions::default());
        f.arm(30);
        f.start_take(1, false);
        f.run(Duration::from_secs(3));
        let restart_at = f.received.unwrap();
        let mut out = Out::default();
        let now = f.now;
        f.core
            .on_encoder(&mut vec![EncoderEvent::Restart], now, &mut out);
        f.absorb(out);
        assert!(f.actions.contains(&SessionAction::RebuildEncoder));
        assert!(f.events.has("rebuilding"));
        f.run(Duration::from_secs(3));
        let ends = f.ends();
        assert_eq!(ends.len(), 1);
        assert!(
            matches!(&ends[0].0, TakeEnd::Cut(w) if w.starts_with("The video encoder restarted"))
        );
        assert_eq!(f.events.count("ready"), 2, "armed again on the new encoder");
        f.save();
        f.run(Duration::from_millis(500));
        let clip = f.clips.last().expect("a clip");
        assert!(
            clip.window.first_index > restart_at,
            "nothing from before the restart is kept"
        );
    }

    /// Test 62: an encoder whose first IDR brings other SPS/PPS than its
    /// media type had is not a header change — the take opens on the
    /// in-band sets — but a later change is.
    #[test]
    fn the_first_idr_may_bring_its_own_header() {
        let mut f = Fake::new(30, TrackOptions::default());
        f.inband = HEADER_B;
        f.arm(30);
        f.start_take(1, false);
        f.run(Duration::from_secs(3));
        assert!(f.core.header_adopted());
        assert!(f.ends().is_empty(), "the take goes on");
        let open = f
            .mux
            .iter()
            .find_map(|m| match m {
                MuxMsg::Open { seq } => Some(seq.clone()),
                _ => None,
            })
            .unwrap();
        assert!(crate::annexb::same_parameter_sets(&open, HEADER_B));
        assert_eq!(f.events.count("ready"), 1);
        f.save();
        f.run(Duration::from_millis(500));
        assert!(crate::annexb::same_parameter_sets(
            &f.clips[0].seq,
            HEADER_B
        ));
        // Later the encoder changes its sets.
        f.inband = HEADER_C;
        f.run(Duration::from_secs(3));
        let ends = f.ends();
        assert_eq!(ends.len(), 1);
        assert!(
            matches!(&ends[0].0, TakeEnd::Cut(w) if w.starts_with("The video encoder restarted"))
        );
        assert_eq!(f.events.count("ready"), 2, "a new segment on the new sets");
    }

    /// The engine's stop holds an armed ring — after its closed epoch's last
    /// audio — and its return arms it again on a cleared ring. A stop before
    /// anything was kept turns Replay off.
    #[test]
    fn an_engine_stop_holds_and_its_return_rearms() {
        let mut f = Fake::new(30, TrackOptions::default());
        // A non-streaming voice model's delay, and a soundboard click in its
        // last 256 ms before the stop: placed past the voice's cursor, it is
        // only written because the stop closes the epoch.
        f.engine.wait = 0.256;
        f.voice = Box::new(|_| 0.0);
        f.sb = Box::new(crate::timeline::tests::click(2.9));
        f.arm(30);
        f.run(Duration::from_secs(3));
        f.signal = EngineSignal::Stopped;
        f.run(Duration::from_secs(2));
        assert_eq!(
            f.core.replay(),
            Some(ReplayView::Held(HeldReason::EngineStopped))
        );
        f.save();
        f.run(Duration::from_millis(100));
        assert_eq!(f.replies().len(), 1, "a held ring's clip is ready at once");
        let clip = &f.clips[0];
        assert!(clip.window.end_index <= 3 * 30 + 2, "it ends at the stop");
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)] // 2.9 s in samples
        let at =
            usize::try_from((2.9 * f64::from(OUT_RATE)) as u64 - clip.window.first_sample).unwrap();
        let around = &clip.pcm[2 * (at - 480)..2 * (at + 480)];
        assert!(
            around.iter().any(|&s| s.abs() > 10_000),
            "the soundboard's last click is in the held clip"
        );
        f.signal = EngineSignal::Running;
        f.run(Duration::from_secs(1));
        assert!(f.events.has("resumed"));
        assert_eq!(f.core.replay(), Some(ReplayView::Armed));

        let mut g = Fake::new(30, TrackOptions::default());
        g.arm(30);
        g.signal = EngineSignal::Stopped;
        g.step(ms(10));
        assert_eq!(g.core.replay(), None);
        assert!(g.exited);
    }

    /// The review's sleep cases: a take-only pipeline and a ring held for a
    /// lost screen go through a sleep with nothing to rebuild — and nothing
    /// pumps the old timeline with the clock that counted it.
    #[test]
    fn a_sleep_without_a_rebuild_never_floods_the_timeline() {
        // A take, no ring.
        let mut f = Fake::new(30, TrackOptions::default());
        f.start_take(1, false);
        f.run(Duration::from_secs(2));
        let mut out = Out::default();
        let t_jump = f.now;
        f.core.on_sleep(t_jump, &mut out);
        f.absorb(out);
        let emitted = f.core.emitted();
        let mut out = Out::default();
        let later = t_jump + Duration::from_secs(8 * 3600);
        f.core
            .on_audio(&[], &[], EngineSignal::Running, later, &mut out);
        f.absorb(out);
        assert!(f.exited, "a take-only pipeline ends with its take");
        assert_eq!(f.core.emitted(), emitted, "pumped across the sleep");

        // A ring held for a lost screen.
        let mut g = Fake::new(30, TrackOptions::default());
        g.arm(30);
        g.run(Duration::from_secs(3));
        let mut out = Out::default();
        let now = g.now;
        g.core.on_screen_lost(now, &mut out);
        g.absorb(out);
        g.run(Duration::from_secs(2));
        let mut out = Out::default();
        let t_jump = g.now;
        g.core.on_sleep(t_jump, &mut out);
        g.absorb(out);
        let emitted = g.core.emitted();
        let mut out = Out::default();
        g.core.on_audio(
            &[],
            &[],
            EngineSignal::Running,
            t_jump + Duration::from_secs(8 * 3600),
            &mut out,
        );
        g.absorb(out);
        assert_eq!(g.core.emitted(), emitted, "pumped across the sleep");
        assert_eq!(g.core.replay(), None, "emptied, it is off");
        assert!(g.events.has("failed"));
        assert!(!g.events.has("rebuilding"));
        assert!(g.exited, "nothing left to run for");
    }

    /// A ring held for the engine and emptied by a sleep has nothing to
    /// save: a screen or encoder lost after the rebuild turns Replay off
    /// rather than holding nothing for good.
    #[test]
    fn an_emptied_hold_that_loses_its_screen_turns_off() {
        for screen in [true, false] {
            let mut f = Fake::new(30, TrackOptions::default());
            f.arm(30);
            f.run(Duration::from_secs(3));
            f.signal = EngineSignal::Stopped;
            f.run(Duration::from_secs(1));
            let mut out = Out::default();
            let now = f.now;
            f.core.on_sleep(now, &mut out);
            f.absorb(out);
            f.core.restarted(Some(Arc::from(HEADER_A)), (320, 240));
            f.now = Duration::ZERO;
            f.engine = Engine::new(48_000, 0.0);
            f.last_idr = None;
            f.run(Duration::from_secs(1));
            assert_eq!(
                f.core.replay(),
                Some(ReplayView::Held(HeldReason::EngineStopped)),
                "held for the engine, emptied"
            );
            let mut out = Out::default();
            let now = f.now;
            if screen {
                f.core.on_screen_lost(now, &mut out);
            } else {
                f.core
                    .encoder_failed("stalled", false, false, now, &mut out);
            }
            f.absorb(out);
            assert_eq!(f.core.replay(), None, "screen {screen}: Replay is off");
            assert!(f.events.has("failed"));
            assert!(f.exited);
        }
        // Held with something kept, the same loss holds it.
        let mut f = Fake::new(30, TrackOptions::default());
        f.arm(30);
        f.run(Duration::from_secs(3));
        f.signal = EngineSignal::Stopped;
        f.run(Duration::from_secs(1));
        let mut out = Out::default();
        let now = f.now;
        f.core.on_screen_lost(now, &mut out);
        f.absorb(out);
        assert_eq!(
            f.core.replay(),
            Some(ReplayView::Held(HeldReason::ScreenLost))
        );
        assert!(!f.events.has("failed"));
    }

    /// A rebuild that fails turns Replay off — armed or held — and a take
    /// can't start while the session is being built again.
    #[test]
    fn a_failed_rebuild_turns_replay_off() {
        for held in [false, true] {
            let mut f = Fake::new(30, TrackOptions::default());
            f.arm(30);
            f.run(Duration::from_secs(3));
            if held {
                f.signal = EngineSignal::Stopped;
                f.run(Duration::from_secs(1));
                assert_eq!(
                    f.core.replay(),
                    Some(ReplayView::Held(HeldReason::EngineStopped))
                );
            }
            let mut out = Out::default();
            let now = f.now;
            f.core.on_sleep(now, &mut out);
            f.absorb(out);
            assert_eq!(
                f.core.take_refusal(),
                Some(super::NO_PICTURE_YET),
                "no take while the session is rebuilt"
            );
            let mut out = Out::default();
            f.core.rebuild_failed("busy", now, &mut out);
            f.absorb(out);
            assert_eq!(f.core.replay(), None, "held {held}: Replay is off");
            assert!(f.events.has("failed"));
            assert!(f.exited);
        }
    }

    /// Turning Replay back on while a Disarm waits to hand over a cut starts
    /// a fresh ring that says when it is ready — the cut is still saved.
    #[test]
    fn re_arming_during_a_disarm_starts_afresh() {
        let mut f = Fake::new(30, TrackOptions::default());
        f.arm(30);
        f.start_take(1, false);
        f.run(Duration::from_secs(3));
        f.audio_on = false;
        f.step(ms(10));
        f.save();
        let mut out = Out::default();
        let now = f.now;
        f.core.disarm(now, &mut out);
        f.absorb(out);
        f.arm(30);
        f.audio_on = true;
        f.run(Duration::from_secs(3));
        assert_eq!(f.replies().len(), 1, "the cut is saved");
        assert_eq!(f.core.replay(), Some(ReplayView::Armed));
        assert_eq!(f.events.count("ready"), 2, "the fresh ring is announced");
    }

    /// A take stopped by the user, then cut short by a failing encoder
    /// before its last frames came out, ends after the last frame that did.
    #[test]
    fn a_failure_after_a_stop_ends_at_the_last_frame_received() {
        let mut f = Fake::new(30, TrackOptions::default());
        f.arm(30);
        f.start_take(1, false);
        f.run(Duration::from_secs(3));
        // Frames are in flight and stop coming.
        f.answering = false;
        f.step(ms(40));
        f.step(ms(40));
        f.stop_take(1);
        let mut out = Out::default();
        let now = f.now;
        f.core
            .encoder_failed("stalled", false, false, now, &mut out);
        f.absorb(out);
        f.run(Duration::from_secs(2));
        let ends = f.ends();
        assert_eq!(ends.len(), 1);
        let last = *f.take_video().last().unwrap();
        assert_eq!(Some(last), f.received);
        let k0 = f.take_video()[0];
        assert_eq!(
            (f.take_audio().len() / 2) as u64,
            blocks_of(sample_of(last + 1, 30) - sample_of(k0, 30)),
            "the audio ends with the last frame received"
        );
    }

    /// An encoder that ignores every forced keyframe is reported once, not
    /// on every fault.
    #[test]
    fn ignored_keyframes_are_reported_once() {
        let mut f = Fake::new(30, TrackOptions::default());
        f.start_take(1, false);
        // Only the first keyframe ever comes: no GOP, forced ones ignored.
        f.gop = u64::MAX / 2;
        f.honour_force = false;
        f.run(Duration::from_secs(12));
        assert_eq!(f.events.count("direct-failed"), 1);
        assert!(f.core.keyframe_faults() > 1);
    }
}
