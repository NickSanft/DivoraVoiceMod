//! Audio engine — owns a dedicated thread that hosts cpal streams.
//!
//! The thread receives commands through a `mpsc` channel and reports
//! results either via reply channels (for Start) or via shared atomic
//! state (for level meters, running flag).
//!
//! Phase 2 scope: passthrough only. Input frames are mono-mixed,
//! enqueued into an SPSC ring buffer, and dequeued by the output stream
//! which fans them back out across the output channels (or writes zero
//! when monitor is off). DSP graph slots in between in Phase 3.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, sync_channel, Receiver, Sender, SyncSender, TryRecvError};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{Device, SampleFormat, Stream, StreamConfig};
use ringbuf::traits::{Consumer, Observer, Producer, Split};
use ringbuf::HeapRb;
use serde::{Deserialize, Serialize};

use super::level::LevelMeter;
use super::loudness::{LoudnessNormalizer, DEFAULT_TARGET_DBFS};
use super::resampler::MonoResampler;
use super::state::{EngineState, Levels};
use super::AudioEngineError;
use crate::dsp::{
    Analyzer, Displaced, DspCommand, DspEdit, EffectChain, InputFacts, Metrics, ReactiveConfig,
    ReactiveModulator, ReadingState, ResolvedReactive, VoiceReading,
};
use crate::soundboard::{SoundboardCommand, SoundboardMixer};

/// Capacity of the SPSC ring buffer used between input and output
/// callbacks. ~170 ms at 48 kHz; ample headroom for OS scheduling jitter.
const RING_BUFFER_FRAMES: usize = 8192;

/// Max samples processed in a single callback. Realistic cpal buffers
/// stay well under this; the array sits on the stack so no allocation
/// happens in the audio thread.
const MAX_FRAMES_PER_CALLBACK: usize = 4096;

/// Capacity of the bounded UI→audio soundboard command queue. The realtime
/// output callback drains it every buffer, so in normal use it holds only a
/// handful of messages. The cap exists purely as a safety valve: if the
/// output callback ever stops draining — a dead/disconnected output device —
/// the queue cannot grow without bound and OOM the whole process; excess
/// plays are dropped (`try_send`) instead. (v1.33.0)
const SB_CHANNEL_CAPACITY: usize = 256;

/// Capacity of the bounded engine→audio DSP edit queue.
///
/// Bounded so the callback's `try_recv` never frees anything: an unbounded
/// `mpsc` allocates its blocks on the sending side and frees them on the
/// RECEIVING one, which here is the audio thread. Deep enough for any burst
/// of UI edits between two buffers; overflow drops the edit, which needs a
/// callback that has stopped draining — the case device-loss recovery exists
/// for.
const DSP_CHANNEL_CAPACITY: usize = 64;

/// Capacity of the graveyard ring — chains (and voice models) the audio
/// callback displaced, for [`graveyard_thread`] to free. Deep enough that
/// someone mashing preset buttons cannot outrun the drain interval below.
const GRAVEYARD_CAPACITY: usize = 16;

/// How often the graveyard thread drains. It polls instead of blocking on a
/// channel because a parked receiver has to be woken, and the only sender is
/// the audio callback — the one thread that must not make a syscall. Freeing
/// a displaced chain up to this late costs nothing but the memory.
const GRAVEYARD_DRAIN_INTERVAL_MS: u64 = 50;

/// Max automatic session rebuilds (after stream errors) between manual
/// starts. Caps recovery so a permanently-flapping device can't loop
/// forever; a manual `Start` resets the budget. (v1.33.0)
const MAX_AUTO_RECOVERIES: u32 = 8;

/// How often the voice-reading worker drains its taps. Short enough that a
/// `RING_BUFFER_FRAMES` ring cannot fill between passes even at 96 kHz (where
/// 8192 frames is ~85 ms), long enough that an idle worker costs nothing.
const READING_DRAIN_INTERVAL_MS: u64 = 20;

/// How long a held reading stays on screen before the panel drops the numbers
/// and shows only the state. A reading from four minutes ago is not a stale
/// reading, it is a different conversation. In seconds, so it behaves
/// identically at 44.1, 48 and 96 kHz.
const READING_HOLD_SECS: u64 = 30;

/// How long a `Speaking` verdict survives with no further analysis windows
/// before it is demoted. A stalled output device keeps the engine's `running`
/// flag true while producing no callbacks at all; without this the panel would
/// sit on "live" indefinitely. In milliseconds, so it is the same wall-clock
/// window at every sample rate.
const READING_LIVE_TIMEOUT_MS: u64 = 2_000;

/// How far from an exact octave a pitch jump can land and still be treated as
/// a possible tracker error, in semitones. Also the window within which two
/// consecutive readings count as agreeing with each other.
const OCTAVE_TOLERANCE_ST: f32 = 1.5;

/// Information about the live engine session.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StreamInfo {
    pub input_name: String,
    pub output_name: String,
    /// Phase 13: separate monitor ("hear yourself") output device, if
    /// one is active. `None` when monitoring rides the main output.
    pub monitor_name: Option<String>,
    pub sample_rate: u32,
    pub input_channels: u16,
    pub output_channels: u16,
}

enum Command {
    Start {
        input_name: Option<String>,
        output_name: Option<String>,
        monitor_name: Option<String>,
        reply: Sender<Result<StreamInfo, AudioEngineError>>,
    },
    Stop,
    SetMonitor(bool),
    /// A chain edit the caller already prepared (built the chain, started the
    /// model load) — see [`DspEdit::prepare`].
    Dsp(DspEdit),
    /// v1.46.0: replace the whole reactive-modulation configuration.
    /// Sent as one message so the audio thread can never observe a
    /// half-applied state (e.g. new routes against an old intensity).
    Reactive(Box<ReactiveConfig>),
    Soundboard(SoundboardCommand),
    StartRecording {
        path: PathBuf,
    },
    StopRecording,
    /// v1.23.0: begin capturing the dry input to `path` for voice cloning.
    StartReferenceRecording {
        path: PathBuf,
    },
    /// v1.23.0: stop the dry capture; `reply` fires once the WAV is finalized.
    StopReferenceRecording {
        reply: Sender<()>,
    },
    /// v1.33.0: an output stream's error callback fired (device unplugged,
    /// disabled, or the default device changed). The engine thread tears the
    /// session down and rebuilds on the same devices — device-loss recovery.
    /// Coalesced via [`EngineState::stream_error_pending`].
    StreamError,
    Shutdown,
}

/// Commands to the recording writer thread (drains the recording ring).
enum RecordingCommand {
    Start {
        path: PathBuf,
    },
    /// Finalize the open WAV. `reply` (v1.23.0) lets a caller block until the
    /// file is fully flushed to disk — used by the reference recorder so the
    /// clip can be decoded immediately after stopping. `None` for the
    /// fire-and-forget output recorder.
    Stop {
        reply: Option<Sender<()>>,
    },
}

/// Public handle to the audio engine. Construct once at app startup;
/// drop to shut down the audio thread cleanly.
pub struct AudioEngine {
    tx: Sender<Command>,
    state: Arc<EngineState>,
    /// Voice-reading gate + latest snapshot. Separate from `state` because
    /// only this feature reads or writes it.
    reading: Arc<ReadingTap>,
    handle: Option<JoinHandle<()>>,
}

impl AudioEngine {
    /// Spawn the audio thread.
    #[must_use]
    pub fn new() -> Self {
        let (tx, rx) = channel();
        let state = Arc::new(EngineState::default());
        // Monitor defaults to on so first-time users hear themselves
        // immediately when they start the engine.
        state.monitor.store(true, Ordering::Release);
        // Speak/soundboard monitor defaults to on so TTS previews are audible
        // even if mic monitoring is later turned off (v1.29.0).
        state.monitor_soundboard.store(true, Ordering::Release);
        // Monitor gain defaults to unity (Default would leave it 0.0).
        state.store_monitor_gain(1.0);
        // v1.7.0: loudness normalization is opt-in (enabled defaults false);
        // seed the target so the readout/stage have a sane value if enabled.
        state.store_loudness_target(DEFAULT_TARGET_DBFS);
        let state_clone = state.clone();
        let reading = Arc::new(ReadingTap::default());
        let reading_clone = reading.clone();
        // The engine thread keeps a Command sender so a stream's error
        // callback can ask it to rebuild the session after a device loss.
        let cmd_tx = tx.clone();
        let handle = std::thread::Builder::new()
            .name("divora-audio".into())
            .spawn(move || engine_thread(rx, cmd_tx, state_clone, reading_clone))
            .expect("spawning the audio thread should not fail");
        Self {
            tx,
            state,
            reading,
            handle: Some(handle),
        }
    }

    /// Start passthrough using the named input/output devices (or the
    /// host defaults when `None`). Blocks until the audio thread reports
    /// success or failure.
    pub fn start(
        &self,
        input_name: Option<&str>,
        output_name: Option<&str>,
        monitor_name: Option<&str>,
    ) -> Result<StreamInfo, AudioEngineError> {
        let (reply_tx, reply_rx) = channel();
        self.tx
            .send(Command::Start {
                input_name: input_name.map(str::to_owned),
                output_name: output_name.map(str::to_owned),
                monitor_name: monitor_name.map(str::to_owned),
                reply: reply_tx,
            })
            .map_err(|_| AudioEngineError::ThreadGone)?;
        reply_rx.recv().map_err(|_| AudioEngineError::ThreadGone)?
    }

    /// Tear down the live streams. Idempotent.
    pub fn stop(&self) {
        let _ = self.tx.send(Command::Stop);
    }

    /// Toggle sidetone monitoring. When false, the output stream emits
    /// silence even while the engine is running and metering input.
    pub fn set_monitor(&self, enabled: bool) {
        let _ = self.tx.send(Command::SetMonitor(enabled));
    }

    /// v1.29.0: toggle hearing Speak/soundboard "monitor-only" voices (TTS
    /// previews) in the monitor — independent of [`set_monitor`](Self::set_monitor)
    /// (which governs hearing your own mic). Stored directly; the output callback
    /// picks it up next buffer.
    pub fn set_speak_monitor(&self, enabled: bool) {
        self.state
            .monitor_soundboard
            .store(enabled, Ordering::Release);
    }

    /// v1.29.0: whether Speak/soundboard previews are routed to the monitor.
    #[must_use]
    pub fn is_speak_monitoring(&self) -> bool {
        self.state.monitor_soundboard.load(Ordering::Acquire)
    }

    /// v1.6.0: set the gain applied to the separate monitor ("hear
    /// yourself") stream. 1.0 = unity. Clamped to a safe range. Stored in
    /// shared state and picked up by the monitor callback next buffer; no
    /// engine restart needed.
    pub fn set_monitor_gain(&self, gain: f32) {
        self.state.store_monitor_gain(gain.clamp(0.0, 4.0));
    }

    /// Send a DSP command (chain build, parameter sweep, etc.). The
    /// audio thread drains these at the top of each output buffer.
    ///
    /// The command is lowered to a [`DspEdit`] **here, on the calling
    /// thread**: a `SetChain` boxes and constructs every effect, and a voice
    /// model starts an ONNX load, neither of which the audio callback may do.
    /// Every caller keeps sending plain `DspCommand`s, so the whole fix lives
    /// at this one seam.
    pub fn send_dsp(&self, cmd: DspCommand) {
        if let Some(edit) = DspEdit::prepare(cmd) {
            let _ = self.tx.send(Command::Dsp(edit));
        }
    }

    /// v1.46.0: replace the reactive-modulation configuration.
    pub fn send_reactive(&self, cfg: ReactiveConfig) {
        let _ = self.tx.send(Command::Reactive(Box::new(cfg)));
    }

    /// Send a soundboard command (play / stop / stop-all). Forwarded
    /// through the engine thread to the live output callback.
    pub fn send_soundboard(&self, cmd: SoundboardCommand) {
        let _ = self.tx.send(Command::Soundboard(cmd));
    }

    /// Phase 16: begin recording the modulated output to a WAV file at
    /// `path`. No-op (until next start) if the engine isn't running.
    pub fn start_recording(&self, path: PathBuf) {
        let _ = self.tx.send(Command::StartRecording { path });
    }

    /// Stop the current recording and finalize the WAV file.
    pub fn stop_recording(&self) {
        let _ = self.tx.send(Command::StopRecording);
    }

    #[must_use]
    pub fn is_recording(&self) -> bool {
        self.state.recording.load(Ordering::Acquire)
    }

    /// v1.23.0: begin recording the **dry input** (raw mic, before effects)
    /// to a WAV at `path` — the reference clip for voice cloning. Independent
    /// of [`start_recording`](Self::start_recording) (which captures the
    /// modulated output). No-op (until next start) if the engine isn't running.
    pub fn start_reference_recording(&self, path: PathBuf) {
        let _ = self.tx.send(Command::StartReferenceRecording { path });
    }

    /// v1.23.0: stop the dry-input reference recording and **block until the
    /// WAV is finalized** on disk, so the caller can decode it immediately.
    /// Returns `false` only if the audio thread is gone.
    pub fn stop_reference_recording(&self) -> bool {
        let (reply_tx, reply_rx) = channel();
        if self
            .tx
            .send(Command::StopReferenceRecording { reply: reply_tx })
            .is_err()
        {
            return false;
        }
        reply_rx.recv().is_ok()
    }

    #[must_use]
    pub fn is_reference_recording(&self) -> bool {
        self.state.reference_recording.load(Ordering::Acquire)
    }

    #[must_use]
    pub fn is_running(&self) -> bool {
        self.state.running.load(Ordering::Acquire)
    }

    /// v1.39.0: true when device-loss auto-recovery gave up and the engine is
    /// stopped with no audio (drives the UI's "device lost — restart" banner).
    #[must_use]
    pub fn device_recovery_failed(&self) -> bool {
        self.state.recovery_failed.load(Ordering::Acquire)
    }

    #[must_use]
    pub fn is_monitoring(&self) -> bool {
        self.state.monitor.load(Ordering::Acquire)
    }

    /// v1.6.0: current monitor ("hear yourself") gain. 1.0 = unity.
    #[must_use]
    pub fn monitor_gain(&self) -> f32 {
        self.state.load_monitor_gain()
    }

    /// v1.7.0: enable/disable output loudness normalization (auto-gain +
    /// limiter). Stored in shared state and picked up by the output
    /// callback next buffer; no engine restart needed.
    pub fn set_loudness_enabled(&self, enabled: bool) {
        self.state.store_loudness_enabled(enabled);
    }

    /// v1.7.0: set the loudness target level, dBFS. The normalizer clamps
    /// it to its supported window.
    pub fn set_loudness_target(&self, dbfs: f32) {
        self.state.store_loudness_target(dbfs);
    }

    /// v1.7.0: whether loudness normalization is currently enabled.
    #[must_use]
    pub fn loudness_enabled(&self) -> bool {
        self.state.load_loudness_enabled()
    }

    /// v1.7.0: current loudness target, dBFS.
    #[must_use]
    pub fn loudness_target(&self) -> f32 {
        self.state.load_loudness_target()
    }

    /// v1.7.0: makeup gain the normalizer is currently applying, in dB
    /// (0 dB while disabled or stopped). For the live UI readout.
    #[must_use]
    pub fn loudness_gain_db(&self) -> f32 {
        self.state.load_loudness_gain_db()
    }

    #[must_use]
    pub fn input_levels(&self) -> Levels {
        self.state.load_input()
    }

    #[must_use]
    pub fn output_levels(&self) -> Levels {
        self.state.load_output()
    }

    /// Phase 14: latency added by the active DSP chain, in milliseconds.
    /// 0 when stopped or when no latency-adding effects are enabled.
    #[must_use]
    pub fn dsp_latency_ms(&self) -> f32 {
        self.state.load_dsp_latency_ms()
    }

    /// v1.46.0: current reactive-modulation depth, 0..=1. 0 when the
    /// feature is off or the engine is stopped.
    #[must_use]
    pub fn reactive_depth(&self) -> f32 {
        self.state.load_reactive_depth()
    }

    /// Turn the voice-reading analysis on or off.
    ///
    /// Off is the default, and off means off: the audio callback copies
    /// nothing into the taps and the worker runs no analysis. Switching it on
    /// starts a fresh baseline — nothing carries over from the last time,
    /// and nothing is written to disk.
    pub fn set_reading_enabled(&self, enabled: bool) {
        self.reading.enabled.store(enabled, Ordering::Release);
    }

    #[must_use]
    pub fn reading_enabled(&self) -> bool {
        self.reading.is_enabled()
    }

    /// Tell the reading the input is muted.
    ///
    /// A fact handed in, not a level guess — see [`ReadingFacts`]. Deliberately
    /// NOT wired to push-to-modulate: with the key up the mic is still open
    /// and only the chain is bypassed, so the dry reading stays valid and only
    /// the after-effects half changes.
    pub fn set_reading_muted(&self, muted: bool) {
        self.reading.muted.store(muted, Ordering::Release);
    }

    /// The latest voice reading. Idle (measuring nothing) while the feature is
    /// off, and held-but-flagged rather than decayed during pauses.
    #[must_use]
    pub fn voice_reading(&self) -> ReadingSnapshot {
        self.reading.load()
    }
}

impl Default for AudioEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for AudioEngine {
    fn drop(&mut self) {
        let _ = self.tx.send(Command::Shutdown);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

/// Currently-running cpal streams. Holding these alive keeps audio flowing.
struct RunningStreams {
    _input: Stream,
    _output: Stream,
    /// Phase 13: optional second output to a separate monitor device.
    _monitor: Option<Stream>,
}

/// Everything `start_streams` hands back: the live streams plus the
/// command senders + the recording writer's join handle.
struct StartedStreams {
    streams: RunningStreams,
    info: StreamInfo,
    dsp_tx: SyncSender<DspEdit>,
    reactive_tx: Sender<ResolvedReactive>,
    sb_tx: SyncSender<SoundboardCommand>,
    recording_tx: Sender<RecordingCommand>,
    writer: JoinHandle<()>,
    /// v1.23.0: the dry-input reference recorder's channel + writer thread.
    reference_tx: Sender<RecordingCommand>,
    reference_writer: JoinHandle<()>,
    /// The voice-reading worker. Dropping `reading_tx` disconnects its
    /// keep-alive channel, which is how it learns the session is over.
    reading_tx: Sender<()>,
    reading_worker: JoinHandle<()>,
    /// The graveyard thread, which frees the chains the callback displaces.
    /// Same keep-alive shutdown as the workers above.
    graveyard_tx: Sender<()>,
    graveyard: JoinHandle<()>,
}

#[allow(clippy::needless_pass_by_value)] // owns values for the thread's lifetime
#[allow(clippy::too_many_lines)] // one command dispatch loop; clearer inline
fn engine_thread(
    rx: Receiver<Command>,
    cmd_tx: Sender<Command>,
    state: Arc<EngineState>,
    reading: Arc<ReadingTap>,
) {
    let mut current: Option<RunningStreams> = None;
    let mut dsp_tx: Option<SyncSender<DspEdit>> = None;
    let mut reactive_tx: Option<Sender<ResolvedReactive>> = None;
    let mut sb_tx: Option<SyncSender<SoundboardCommand>> = None;
    let mut recording_tx: Option<Sender<RecordingCommand>> = None;
    let mut writer: Option<JoinHandle<()>> = None;
    // v1.23.0: the parallel dry-input reference recorder (its own ring + writer).
    let mut reference_tx: Option<Sender<RecordingCommand>> = None;
    let mut reference_writer: Option<JoinHandle<()>> = None;
    // The voice-reading worker + its keep-alive sender. Per-session, which is
    // what makes an engine restart or a device change invalidate the baseline.
    let mut reading_tx: Option<Sender<()>> = None;
    let mut reading_worker: Option<JoinHandle<()>> = None;
    // The graveyard thread + its keep-alive sender. Per-session like the
    // workers, so an engine restart cannot leave one behind.
    let mut graveyard_tx: Option<Sender<()>> = None;
    let mut graveyard: Option<JoinHandle<()>> = None;
    // v1.33.0: the devices of the live session, kept so a stream error can
    // rebuild on the same ones; and a counter that caps automatic recoveries
    // between manual starts so a flapping device can't loop forever.
    let mut last_start: Option<(Option<String>, Option<String>, Option<String>)> = None;
    let mut stream_recoveries: u32 = 0;
    // v1.39.0: remember the soundboard master gain so a device-loss rebuild can
    // restore it — a rebuild constructs a fresh mixer at unity, which would
    // otherwise silently jump the user's chosen volume back to 100%.
    let mut last_master_gain: f32 = 1.0;
    // v1.46.0: remember the reactive config for the same reason — a
    // device-loss rebuild constructs a fresh modulator with no routes, which
    // would silently switch the feature off mid-call.
    let mut last_reactive = ReactiveConfig::default();

    // Tear down the current session's streams + recording writers.
    // Dropping a writer's `tx` disconnects its channel, which makes it
    // finalize any open WAV and exit; we then join it.
    macro_rules! teardown {
        () => {{
            drop(current.take());
            drop(dsp_tx.take());
            drop(sb_tx.take());
            state.recording.store(false, Ordering::Release);
            state.reference_recording.store(false, Ordering::Release);
            drop(recording_tx.take());
            drop(reactive_tx.take());
            drop(reference_tx.take());
            if let Some(h) = writer.take() {
                let _ = h.join();
            }
            if let Some(h) = reference_writer.take() {
                let _ = h.join();
            }
            drop(reading_tx.take());
            if let Some(h) = reading_worker.take() {
                let _ = h.join();
            }
            // Last, and only after `current` dropped the streams above: the
            // callback is gone by now, so the graveyard's final pass sees
            // everything it was handed.
            drop(graveyard_tx.take());
            if let Some(h) = graveyard.take() {
                let _ = h.join();
            }
            state.running.store(false, Ordering::Release);
            // The meter must not freeze at the last depth once the engine
            // is down — `reactive_depth()` documents 0 when stopped.
            state.store_reactive_depth(0.0);
            // Same for the reading: the worker is gone, so nothing would
            // re-label the held window and the panel would go on showing a
            // reading for a session that no longer exists. The baseline dies
            // with the worker, which is the invalidation rule on an engine
            // restart or a device change.
            reading.store(ReadingSnapshot::idle(ReadingState::Stopped));
        }};
    }

    // Bring a session up on the given devices, wiring the command senders +
    // `running` flag into the engine thread's locals. Shared by the user
    // `Start` path and the automatic post-error recovery; returns the
    // `StreamInfo` on success, the build error on failure.
    macro_rules! bring_up {
        ($in:expr, $out:expr, $mon:expr) => {{
            match start_streams(
                $in,
                $out,
                $mon,
                state.clone(),
                reading.clone(),
                cmd_tx.clone(),
            ) {
                Ok(started) => {
                    state.running.store(true, Ordering::Release);
                    dsp_tx = Some(started.dsp_tx);
                    reactive_tx = Some(started.reactive_tx);
                    sb_tx = Some(started.sb_tx);
                    recording_tx = Some(started.recording_tx);
                    writer = Some(started.writer);
                    reference_tx = Some(started.reference_tx);
                    reference_writer = Some(started.reference_writer);
                    reading_tx = Some(started.reading_tx);
                    reading_worker = Some(started.reading_worker);
                    graveyard_tx = Some(started.graveyard_tx);
                    graveyard = Some(started.graveyard);
                    current = Some(started.streams);
                    // Restore the soundboard master gain onto the fresh (unity)
                    // mixer so a rebuild doesn't reset the user's volume.
                    if let Some(tx) = &sb_tx {
                        let _ = tx.try_send(SoundboardCommand::SetMasterGain(last_master_gain));
                    }
                    // Same for reactive modulation: re-apply onto the fresh
                    // modulator so a rebuild doesn't drop the routing.
                    if let Some(tx) = &reactive_tx {
                        let _ = tx.send(last_reactive.resolve());
                    }
                    Ok(started.info)
                }
                Err(e) => Err(e),
            }
        }};
    }

    while let Ok(cmd) = rx.recv() {
        match cmd {
            Command::Start {
                input_name,
                output_name,
                monitor_name,
                reply,
            } => {
                teardown!();
                let result = bring_up!(
                    input_name.as_deref(),
                    output_name.as_deref(),
                    monitor_name.as_deref()
                );
                if result.is_ok() {
                    // Remember the devices for post-error recovery and reset
                    // the auto-recovery budget for this fresh session.
                    last_start = Some((input_name, output_name, monitor_name));
                    stream_recoveries = 0;
                    state.stream_error_pending.store(false, Ordering::Release);
                    // A manual start clears any prior "recovery gave up" banner.
                    state.recovery_failed.store(false, Ordering::Release);
                }
                let _ = reply.send(result);
            }
            Command::Stop => {
                teardown!();
                state.store_input(Levels::default());
                state.store_output(Levels::default());
                state.store_dsp_latency_ms(0.0);
                // A user Stop must win over any in-flight StreamError: a device
                // error that fired just before Stop has already queued a
                // `Command::StreamError` behind us. Clear the recovery intent
                // so that queued error can't resurrect the session the user
                // just stopped (with `last_start` None its rebuild is skipped).
                last_start = None;
                stream_recoveries = 0;
                state.stream_error_pending.store(false, Ordering::Release);
                // A deliberate Stop also dismisses any "recovery gave up" state.
                state.recovery_failed.store(false, Ordering::Release);
            }
            Command::StreamError => {
                // An output stream died — most likely the device was
                // unplugged/disabled or the default device changed. Tear the
                // session down (which drops `sb_tx`, so the UI→audio queue
                // stops growing) and rebuild on the same devices so audio
                // comes back without a manual restart. `stream_error_pending`
                // is re-armed only AFTER teardown, so the now-dropped stream
                // can't re-signal; a capped budget stops a genuinely failing
                // device from looping forever (a manual Start resets it).
                teardown!();
                state.store_input(Levels::default());
                state.store_output(Levels::default());
                state.store_dsp_latency_ms(0.0);
                state.stream_error_pending.store(false, Ordering::Release);
                stream_recoveries += 1;
                if stream_recoveries > MAX_AUTO_RECOVERIES {
                    // Give up — surface it so the UI can prompt a restart
                    // instead of showing a silent, metering-but-dead session.
                    state.recovery_failed.store(true, Ordering::Release);
                    tracing::error!(
                        attempts = stream_recoveries,
                        "audio stream kept failing; auto-recovery gave up — \
                         the engine is stopped, restart to try again"
                    );
                } else if let Some((i, o, m)) = last_start.clone() {
                    match bring_up!(i.as_deref(), o.as_deref(), m.as_deref()) {
                        Ok(info) => {
                            state.recovery_failed.store(false, Ordering::Release);
                            tracing::warn!(
                                attempt = stream_recoveries,
                                output = %info.output_name,
                                "audio stream error — recovered by rebuilding the session"
                            );
                        }
                        Err(e) => {
                            state.recovery_failed.store(true, Ordering::Release);
                            tracing::error!(
                                error = ?e,
                                "audio stream error — rebuild failed; engine stopped"
                            );
                        }
                    }
                }
            }
            Command::SetMonitor(enabled) => {
                state.monitor.store(enabled, Ordering::Release);
            }
            Command::Dsp(edit) => {
                if let Some(tx) = &dsp_tx {
                    // Bounded + non-blocking, for the same reason as the
                    // soundboard queue below: the audio callback is the only
                    // consumer, and the engine thread must never block on it.
                    // A rejected edit is dropped HERE, off the audio thread.
                    if tx.try_send(edit).is_err() {
                        tracing::warn!(
                            "DSP edit queue full — dropped a chain edit; \
                             the audio callback is not draining"
                        );
                    }
                }
            }
            Command::Reactive(cfg) => {
                // Resolve HERE, on the engine thread — the audio callback must
                // not run the whitelist filter_map or drop the config's owned
                // Strings. It receives a ready-made route table instead.
                if let Some(tx) = &reactive_tx {
                    let _ = tx.send(cfg.resolve());
                }
                last_reactive = *cfg;
            }
            Command::Soundboard(sb_cmd) => {
                // Snoop the master gain so a device-loss rebuild can restore it
                // to the fresh (unity) mixer (v1.39.0).
                if let SoundboardCommand::SetMasterGain(g) = &sb_cmd {
                    last_master_gain = *g;
                }
                if let Some(tx) = &sb_tx {
                    // Bounded + non-blocking. The realtime output callback is
                    // the only consumer, draining this every buffer. If it has
                    // stalled (a dead output device), `try_send` DROPS the play
                    // rather than let the queue grow without bound and OOM the
                    // whole app. The engine thread must never block here, so
                    // we never use the blocking `send`.
                    let _ = tx.try_send(sb_cmd);
                }
            }
            Command::StartRecording { path } => {
                // Only if a session is live (the writer exists).
                if let Some(tx) = &recording_tx {
                    state.recording.store(true, Ordering::Release);
                    let _ = tx.send(RecordingCommand::Start { path });
                }
            }
            Command::StopRecording => {
                state.recording.store(false, Ordering::Release);
                if let Some(tx) = &recording_tx {
                    let _ = tx.send(RecordingCommand::Stop { reply: None });
                }
            }
            Command::StartReferenceRecording { path } => {
                // Only if a session is live (the reference writer exists).
                if let Some(tx) = &reference_tx {
                    state.reference_recording.store(true, Ordering::Release);
                    let _ = tx.send(RecordingCommand::Start { path });
                }
            }
            Command::StopReferenceRecording { reply } => {
                state.reference_recording.store(false, Ordering::Release);
                if let Some(tx) = &reference_tx {
                    // The writer fires `reply` once the WAV is finalized.
                    let _ = tx.send(RecordingCommand::Stop { reply: Some(reply) });
                } else {
                    // No live session/writer — nothing to finalize; unblock now.
                    let _ = reply.send(());
                }
            }
            Command::Shutdown => {
                teardown!();
                break;
            }
        }
    }
}

#[derive(Clone, Copy)]
enum Direction {
    Input,
    Output,
}

fn find_device(direction: Direction, name: Option<&str>) -> Result<Device, AudioEngineError> {
    let host = cpal::default_host();
    match direction {
        Direction::Input => match name {
            Some(target) => {
                let devices = host
                    .input_devices()
                    .map_err(|e| AudioEngineError::DefaultConfig(e.to_string()))?;
                devices
                    .into_iter()
                    .find(|d| d.name().is_ok_and(|n| n == target))
                    .ok_or_else(|| AudioEngineError::InputDeviceNotFound(target.to_owned()))
            }
            None => host
                .default_input_device()
                .ok_or(AudioEngineError::NoInputDevice),
        },
        Direction::Output => match name {
            Some(target) => {
                let devices = host
                    .output_devices()
                    .map_err(|e| AudioEngineError::DefaultConfig(e.to_string()))?;
                devices
                    .into_iter()
                    .find(|d| d.name().is_ok_and(|n| n == target))
                    .ok_or_else(|| AudioEngineError::OutputDeviceNotFound(target.to_owned()))
            }
            None => host
                .default_output_device()
                .ok_or(AudioEngineError::NoOutputDevice),
        },
    }
}

#[allow(clippy::needless_pass_by_value)] // state is cloned into the stream closures
#[allow(clippy::too_many_lines)] // linear device/stream setup; splitting hurts readability
#[allow(clippy::similar_names)] // parallel recording/reference ring locals read clearer paired
fn start_streams(
    input_name: Option<&str>,
    output_name: Option<&str>,
    monitor_name: Option<&str>,
    state: Arc<EngineState>,
    // The voice-reading gate + snapshot slot, shared with the panel.
    reading: Arc<ReadingTap>,
    // v1.33.0: handed to the output stream's error callback so a device loss
    // can signal the engine thread to rebuild the session.
    cmd_tx: Sender<Command>,
) -> Result<StartedStreams, AudioEngineError> {
    let input_device = find_device(Direction::Input, input_name)?;
    let output_device = find_device(Direction::Output, output_name)?;

    let input_name_str = input_device.name().unwrap_or_default();
    let output_name_str = output_device.name().unwrap_or_default();

    // Phase 13: resolve the optional monitor device. Skip it when it's
    // unset or names the same device as the main output (there's nothing
    // to gain from a second stream on the same device — the main output
    // already plays it).
    let monitor_device = match monitor_name {
        Some(n) if !n.is_empty() && n != output_name_str => {
            Some(find_device(Direction::Output, Some(n))?)
        }
        _ => None,
    };
    let monitor_name_str = monitor_device
        .as_ref()
        .map(|d| d.name().unwrap_or_default());
    let has_monitor = monitor_device.is_some();

    let input_default = input_device
        .default_input_config()
        .map_err(|e| AudioEngineError::DefaultConfig(e.to_string()))?;
    let output_default = output_device
        .default_output_config()
        .map_err(|e| AudioEngineError::DefaultConfig(e.to_string()))?;

    let input_rate = input_default.sample_rate().0;
    let output_rate = output_default.sample_rate().0;
    // Phase 9 replaces the hard SampleRateMismatch error with a
    // `MonoResampler` in the output callback when the two rates
    // disagree. DSP runs at `input_rate` (the engine rate); the
    // resampler bridges to `output_rate` just before fan-out.

    let input_channels = input_default.channels();
    let output_channels = output_default.channels();
    let input_format = input_default.sample_format();
    let output_format = output_default.sample_format();

    let input_config: StreamConfig = input_default.into();
    let output_config: StreamConfig = output_default.into();

    let rb = HeapRb::<f32>::new(RING_BUFFER_FRAMES);
    let (producer, consumer) = rb.split();

    // Monitor tap ring: the main output callback pushes the processed,
    // input-rate mono (voice + soundboard, post-DSP) into it; the
    // monitor stream resamples that to its own device rate. Only created
    // when a separate monitor device is active.
    let (monitor_producer, monitor_consumer) = if has_monitor {
        let mrb = HeapRb::<f32>::new(RING_BUFFER_FRAMES);
        let (mp, mc) = mrb.split();
        (Some(mp), Some(mc))
    } else {
        (None, None)
    };

    // Phase 16: recording tap ring. Always created; the output callback
    // pushes the processed input-rate mono into it only while
    // `state.recording` is set. A dedicated writer thread drains it to
    // a WAV file so no file I/O ever touches the audio thread.
    let rec_rb = HeapRb::<f32>::new(RING_BUFFER_FRAMES);
    let (recording_producer, recording_consumer) = rec_rb.split();
    let (recording_tx, recording_rx) = channel::<RecordingCommand>();

    // v1.23.0: parallel dry-input reference ring. The input callback pushes
    // the raw (pre-effects) mono into it only while `state.reference_recording`
    // is set; its own writer thread drains it to a WAV used as a clone
    // reference. Separate from the modulated-output recorder above.
    let reference_rb = HeapRb::<f32>::new(RING_BUFFER_FRAMES);
    let (reference_producer, reference_consumer) = reference_rb.split();
    let (reference_tx, reference_rx) = channel::<RecordingCommand>();

    // Voice-reading taps: the DRY mic and the chain's OUTPUT, on their own
    // rings so the worker can compare the same moment on both. Always wired —
    // the panel can be switched on mid-session — but the callback copies into
    // them only while the panel is on, so off costs one atomic load a buffer.
    let reading_dry_rb = HeapRb::<f32>::new(RING_BUFFER_FRAMES);
    let (reading_dry_producer, reading_dry_consumer) = reading_dry_rb.split();
    let reading_wet_rb = HeapRb::<f32>::new(RING_BUFFER_FRAMES);
    let (reading_wet_producer, reading_wet_consumer) = reading_wet_rb.split();
    let (reading_tx, reading_alive) = channel::<()>();

    // The graveyard ring. Chains the callback swaps out leave this way, to a
    // thread that can afford to drop them. A lock-free, preallocated SPSC ring
    // rather than a channel: `try_send` on a bounded channel may have to wake a
    // parked receiver, and that is a syscall in the callback.
    let grave_rb = HeapRb::<Displaced>::new(GRAVEYARD_CAPACITY);
    let (grave_producer, grave_consumer) = grave_rb.split();
    let (graveyard_tx, graveyard_alive) = channel::<()>();

    let (dsp_tx, dsp_rx) = sync_channel::<DspEdit>(DSP_CHANNEL_CAPACITY);
    let (reactive_tx, reactive_rx) = channel::<ResolvedReactive>();
    let (sb_tx, sb_rx) = sync_channel::<SoundboardCommand>(SB_CHANNEL_CAPACITY);

    let input_stream = build_input_stream(
        &input_device,
        &input_config,
        input_format,
        input_channels,
        producer,
        state.clone(),
        reference_producer,
    )?;
    let output_stream = build_output_stream(
        &output_device,
        &output_config,
        output_format,
        output_channels,
        consumer,
        state.clone(),
        dsp_rx,
        reactive_rx,
        sb_rx,
        input_rate,
        output_rate,
        monitor_producer,
        has_monitor,
        recording_producer,
        ReadingTaps {
            dry: reading_dry_producer,
            wet: reading_wet_producer,
        },
        reading.clone(),
        grave_producer,
        cmd_tx,
    )?;

    // Build the monitor stream last so the tap producer is already wired
    // into the main output above.
    let monitor_stream = if let (Some(md), Some(mc)) = (monitor_device, monitor_consumer) {
        let m_default = md
            .default_output_config()
            .map_err(|e| AudioEngineError::DefaultConfig(e.to_string()))?;
        let m_rate = m_default.sample_rate().0;
        let m_channels = m_default.channels();
        let m_format = m_default.sample_format();
        let m_config: StreamConfig = m_default.into();
        let stream = build_monitor_stream(
            &md,
            &m_config,
            m_format,
            m_channels,
            mc,
            state.clone(),
            input_rate,
            m_rate,
        )?;
        stream
            .play()
            .map_err(|e| AudioEngineError::StreamPlay(e.to_string()))?;
        Some(stream)
    } else {
        None
    };

    input_stream
        .play()
        .map_err(|e| AudioEngineError::StreamPlay(e.to_string()))?;
    output_stream
        .play()
        .map_err(|e| AudioEngineError::StreamPlay(e.to_string()))?;

    let info = StreamInfo {
        input_name: input_name_str,
        output_name: output_name_str,
        monitor_name: monitor_name_str,
        sample_rate: input_rate,
        input_channels,
        output_channels,
    };
    // Phase 16: spawn the recording writer thread. It owns the ring's
    // consumer end + the command channel, parks draining nothing until a
    // `Start { path }` arrives, and exits when `recording_tx` drops at
    // teardown (the channel disconnect is its shutdown signal).
    let writer = std::thread::Builder::new()
        .name("divora-recording".into())
        .spawn(move || recording_writer(recording_consumer, recording_rx, input_rate))
        .map_err(|e| AudioEngineError::StreamBuild(e.to_string()))?;

    // v1.23.0: the reference (dry-input) writer — same drain-to-WAV body,
    // its own ring + channel, captures at the native input rate.
    let reference_writer = std::thread::Builder::new()
        .name("divora-reference".into())
        .spawn(move || recording_writer(reference_consumer, reference_rx, input_rate))
        .map_err(|e| AudioEngineError::StreamBuild(e.to_string()))?;

    // The graveyard. Per-session like the writers: it exits when
    // `graveyard_tx` drops at teardown, after the streams are gone, so its
    // last pass sees everything the callback handed over.
    let graveyard = std::thread::Builder::new()
        .name("divora-graveyard".into())
        .spawn(move || graveyard_thread(grave_consumer, graveyard_alive))
        .map_err(|e| AudioEngineError::StreamBuild(e.to_string()))?;

    // The reading worker. Per-session like the writers above: it exits when
    // `reading_tx` drops at teardown, and the rolling baseline it built goes
    // with it, which is exactly the invalidation an engine restart or a device
    // change should cause.
    let reading_worker_handle = std::thread::Builder::new()
        .name("divora-reading".into())
        .spawn(move || {
            reading_worker(
                reading_dry_consumer,
                reading_wet_consumer,
                reading,
                state,
                reading_alive,
                input_rate,
            );
        })
        .map_err(|e| AudioEngineError::StreamBuild(e.to_string()))?;

    Ok(StartedStreams {
        streams: RunningStreams {
            _input: input_stream,
            _output: output_stream,
            _monitor: monitor_stream,
        },
        info,
        dsp_tx,
        reactive_tx,
        sb_tx,
        recording_tx,
        writer,
        reference_tx,
        reference_writer,
        reading_tx,
        reading_worker: reading_worker_handle,
        graveyard_tx,
        graveyard,
    })
}

type RingProducer = <HeapRb<f32> as Split>::Prod;
type RingConsumer = <HeapRb<f32> as Split>::Cons;

/// The graveyard ring's ends: what the audio callback displaced, on its way
/// to the thread that frees it.
type GraveProducer = <HeapRb<Displaced> as Split>::Prod;
type GraveConsumer = <HeapRb<Displaced> as Split>::Cons;

/// Free what the audio callback displaced.
///
/// A preset switch replaces the whole effect chain. Building the new one
/// already happens on the control thread (see [`DspEdit::prepare`]); this is
/// the other half — dropping the OLD one, which means freeing every boxed
/// effect and everything in it: STFT rings, reverb combs, harmonizer state,
/// an ONNX session. The callback hands it over through the ring and moves on.
///
/// Polls rather than blocking on a channel, because a parked receiver has to
/// be woken and the producer is the audio thread. Sleeping between passes
/// costs a wake every [`GRAVEYARD_DRAIN_INTERVAL_MS`] while a session is up.
#[allow(clippy::needless_pass_by_value)] // owns the keep-alive for its lifetime
fn graveyard_thread(mut bin: GraveConsumer, alive: Receiver<()>) {
    loop {
        // Dropping each value IS the work; nothing else to do with it.
        while bin.try_pop().is_some() {}
        if matches!(alive.try_recv(), Err(TryRecvError::Disconnected)) {
            // Teardown drops the streams before this channel, so the callback
            // has already stopped and the pass above emptied the ring.
            return;
        }
        std::thread::sleep(Duration::from_millis(GRAVEYARD_DRAIN_INTERVAL_MS));
    }
}

/// Apply every pending chain edit, at the top of an output buffer.
///
/// Its own function so a test can run exactly what the callback runs: each
/// edit applied, everything it displaces handed to the graveyard rather than
/// freed here, and the voice reading told the chain moved under it.
fn drain_dsp_edits(
    rx: &Receiver<DspEdit>,
    chain: &mut EffectChain,
    graveyard: &mut GraveProducer,
    reading: &ReadingTap,
) {
    while let Ok(edit) = rx.try_recv() {
        if let Some(displaced) = chain.apply(edit) {
            // Getting this out of the callback is the whole point. `try_push`
            // hands it back on a full ring — the graveyard thread has not woken
            // while several chains were replaced — and letting it drop right
            // here is the honest fallback: one bounded, rare hitch rather than
            // a leak or a blocked callback.
            let _ = graveyard.try_push(displaced);
        }
        // The reading worker runs on another thread and cannot see the chain,
        // so it has to be told. Without this the after-effects half kept
        // showing the previous preset's numbers for over a second while the
        // card already named the new one.
        reading.note_chain_changed();
    }
}

fn build_input_stream(
    device: &Device,
    config: &StreamConfig,
    sample_format: SampleFormat,
    channels: u16,
    mut producer: RingProducer,
    state: Arc<EngineState>,
    // v1.23.0: the dry (pre-effects) mono is tapped into this ring while
    // `state.reference_recording` is set; a writer thread drains it to a WAV
    // used as a voice-clone reference. Always wired — capture can begin
    // mid-session.
    mut reference_producer: RingProducer,
) -> Result<Stream, AudioEngineError> {
    let device_name = device.name().unwrap_or_default();
    if sample_format != SampleFormat::F32 {
        // Phase 2 supports the default F32 shared-mode config that
        // virtually all modern Windows devices expose. Non-F32 formats
        // come in a later phase along with rubato resampling.
        return Err(AudioEngineError::UnsupportedSampleFormat {
            device: device_name,
            format: format!("{sample_format:?}"),
        });
    }
    let err_label = device_name;
    let err_fn = move |err: cpal::StreamError| {
        tracing::error!(?err, device = %err_label, "input stream error");
    };

    let mut meter = LevelMeter::new();
    let channels = channels as usize;

    let stream = device
        .build_input_stream(
            config,
            move |data: &[f32], _info| {
                if data.is_empty() || channels == 0 {
                    return;
                }
                let mut mono = [0f32; MAX_FRAMES_PER_CALLBACK];
                let mut written = 0;
                for frame in data.chunks_exact(channels) {
                    if written >= mono.len() {
                        break;
                    }
                    let mut sum = 0f32;
                    for s in frame {
                        sum += *s;
                    }
                    #[allow(clippy::cast_precision_loss)]
                    let avg = sum / channels as f32;
                    mono[written] = avg;
                    written += 1;
                }
                let slice = &mono[..written];
                let _ = producer.push_slice(slice);
                // v1.23.0: tap the dry mono for a clone reference when armed.
                if state.reference_recording.load(Ordering::Acquire) {
                    let _ = reference_producer.push_slice(slice);
                }
                meter.process(slice);
                state.store_input(Levels {
                    rms: meter.rms(),
                    peak: meter.peak(),
                });
            },
            err_fn,
            None,
        )
        .map_err(|e| AudioEngineError::StreamBuild(e.to_string()))?;
    Ok(stream)
}

#[allow(clippy::too_many_arguments)] // builder-style; each arg is necessary
#[allow(clippy::too_many_lines)] // single realtime callback; clearer inline
fn build_output_stream(
    device: &Device,
    config: &StreamConfig,
    sample_format: SampleFormat,
    channels: u16,
    mut consumer: RingConsumer,
    state: Arc<EngineState>,
    dsp_rx: Receiver<DspEdit>,
    reactive_rx: Receiver<ResolvedReactive>,
    sb_rx: Receiver<SoundboardCommand>,
    input_rate: u32,
    output_rate: u32,
    // Phase 13: when a separate monitor device is active, the processed
    // input-rate mono is tapped into this ring for the monitor stream,
    // and the main output plays unconditionally (it's the send to e.g.
    // VB-Cable). `has_monitor` therefore both selects that gating and
    // signals the tap is wired.
    mut monitor_producer: Option<RingProducer>,
    has_monitor: bool,
    // Phase 16: the same processed, input-rate mono is tapped into this
    // ring while `state.recording` is set; a writer thread drains it to
    // a WAV file. Always wired — recording can begin mid-session.
    mut recording_producer: RingProducer,
    // The voice-reading taps. The callback's only job here is two O(n) copies
    // into preallocated rings, gated on an atomic; all analysis happens on the
    // worker thread that drains them.
    mut reading_taps: ReadingTaps,
    reading: Arc<ReadingTap>,
    // Where a displaced chain goes instead of being dropped here. See
    // [`graveyard_thread`].
    mut graveyard: GraveProducer,
    // v1.33.0: lets the stream's error callback ask the engine thread to
    // rebuild the session after a device loss (see `Command::StreamError`).
    cmd_tx: Sender<Command>,
) -> Result<Stream, AudioEngineError> {
    let device_name = device.name().unwrap_or_default();
    if sample_format != SampleFormat::F32 {
        return Err(AudioEngineError::UnsupportedSampleFormat {
            device: device_name,
            format: format!("{sample_format:?}"),
        });
    }
    let err_label = device_name;
    let err_state = state.clone();
    let err_fn = move |err: cpal::StreamError| {
        tracing::error!(?err, device = %err_label, "output stream error");
        // Coalesce the burst of repeated errors a dying stream emits into a
        // single recovery request; the engine thread re-arms this after it
        // tears the session down. Without recovery a dead output callback
        // would stop draining the soundboard queue (see SB_CHANNEL_CAPACITY)
        // and the app would go silently dead until a manual restart.
        if !err_state.stream_error_pending.swap(true, Ordering::AcqRel) {
            let _ = cmd_tx.send(Command::StreamError);
        }
    };

    let mut meter = LevelMeter::new();
    let mut chain = EffectChain::new();
    // v1.46.0: reactive modulation lives BESIDE the chain, never inside it —
    // `SetChain` replaces the chain wholesale on every preset switch, which
    // would invalidate any routing stored within it.
    let mut modulator = ReactiveModulator::new();
    let mut soundboard = SoundboardMixer::new();
    // v1.7.0: post-chain output loudness normalizer (auto-gain + limiter).
    let mut loudness = LoudnessNormalizer::new();
    let channels = channels as usize;
    let state_for_callback = state.clone();

    // When input + output rates disagree we drop a streaming
    // `MonoResampler` into the callback. It buffers native-rate samples
    // from the engine and produces output-rate samples on demand.
    let mut resampler: Option<MonoResampler> = if input_rate == output_rate {
        None
    } else {
        Some(MonoResampler::new(input_rate, output_rate, 256)?)
    };

    let stream = device
        .build_output_stream(
            config,
            move |data: &mut [f32], _info| {
                if data.is_empty() || channels == 0 {
                    return;
                }
                // Drain any pending DSP + soundboard commands before
                // processing audio.
                // v1.46.0: reactive config, drained BEFORE the DSP drain.
                // `configure` restores the outgoing routes to their authored
                // values, and those bases were authored against the CURRENT
                // chain — so the restore has to land before a `SetChain` in the
                // same buffer replaces it.
                while let Ok(cfg) = reactive_rx.try_recv() {
                    modulator.configure(&cfg, &mut chain);
                }
                // Structural changes arrive already built (`DspEdit`) and
                // what they displace leaves through the graveyard, so nothing
                // in this drain allocates or frees.
                drain_dsp_edits(&dsp_rx, &mut chain, &mut graveyard, &reading);
                while let Ok(cmd) = sb_rx.try_recv() {
                    soundboard.apply(cmd);
                }

                // Phase 14: publish the chain's added latency (ms) for
                // the UI readout. Cheap — just sums enabled effects'
                // fixed delays — and reflects the live chain, so it
                // moves the moment Voice Convert / denoiser toggle.
                #[allow(clippy::cast_precision_loss)]
                let lat_ms = chain.latency_samples(input_rate) as f32 / input_rate as f32 * 1000.0;
                state_for_callback.store_dsp_latency_ms(lat_ms);

                let monitoring = state_for_callback.monitor.load(Ordering::Acquire);
                let out_frames = data.len() / channels;
                let out_frames = out_frames.min(MAX_FRAMES_PER_CALLBACK);
                let mut mono = [0f32; MAX_FRAMES_PER_CALLBACK];

                // How many native-rate frames do we need this round?
                // With no resampler: out_frames (1:1). With a
                // resampler: ceil(out_frames * input_rate / output_rate)
                // — but we read in chunks of `resampler.input_frames_next()`
                // so the cumulative size approaches the right total.
                let native_frames = if let Some(r) = resampler.as_ref() {
                    // Aim for slightly more native frames than strictly
                    // needed so the resampler always has a fresh chunk
                    // ready. We size by the input ratio plus the
                    // resampler's own next-needed count.
                    let ratio_num = u64::from(input_rate);
                    let ratio_den = u64::from(output_rate);
                    let scaled = u64::try_from(out_frames)
                        .unwrap_or(u64::MAX)
                        .saturating_mul(ratio_num)
                        / ratio_den.max(1);
                    let approx = usize::try_from(scaled).unwrap_or(MAX_FRAMES_PER_CALLBACK)
                        + r.input_frames_next();
                    approx.min(MAX_FRAMES_PER_CALLBACK)
                } else {
                    out_frames
                };

                let popped = consumer.pop_slice(&mut mono[..native_frames]);
                let zero_from = popped;
                for slot in &mut mono[zero_from..native_frames] {
                    *slot = 0.0;
                }

                // v1.7.0: refresh the loudness stage's params from shared
                // state (cheap atomic loads) so the Mixer toggle/slider
                // take effect next buffer without an engine restart.
                loudness.set_enabled(state_for_callback.load_loudness_enabled());
                loudness.set_target_dbfs(state_for_callback.load_loudness_target());

                // Run the DSP chain over the mic mono buffer first, so
                // effects apply only to the user's voice; normalize the
                // voice's loudness post-chain (steady level across presets,
                // never clipping); then mix soundboard voices in alongside
                // the already-effected voice. Clips play "as-is" (no DSP,
                // no normalization). All write into the same `mono` buffer
                // that the resampler / fan-out consume below, so the mix
                // lands on whatever output device is selected — including
                // CABLE Input, which is what makes the clips audible to
                // call participants.
                // v1.29.0: soundboard `Play` voices (incl. "Speak" + saved
                // clips) render into their OWN buffer so the monitor can gate
                // them by the Speak monitor, independent of the mic monitor.
                // Folded back into `mono` for the main send below.
                let mut sb = [0f32; MAX_FRAMES_PER_CALLBACK];

                // Voice reading, DRY half. `mono` is the raw mic here — the
                // chain rewrites it in place below, so this is the one point
                // on this thread where the untouched signal exists. Room is
                // reserved in BOTH rings now so the WET copy after the chain
                // cannot be the one that gets dropped, which would leave the
                // two taps permanently out of step.
                let tapping = reading_taps.armed(reading.is_enabled(), native_frames);
                if tapping {
                    reading_taps.push_dry(&mono[..native_frames]);
                }

                mix_voice_and_soundboard(
                    &mut mono[..native_frames],
                    &mut sb[..native_frames],
                    &mut chain,
                    &mut modulator,
                    &mut loudness,
                    &mut soundboard,
                    input_rate,
                );
                // Publish the makeup gain for the UI "it's working" readout.
                state_for_callback.store_loudness_gain_db(loudness.gain_db());
                state_for_callback.store_reactive_depth(modulator.depth());

                // Voice reading, WET half — the chain's output, which is what
                // the call hears from the mic. Taken HERE, before the
                // soundboard is folded in below, so Speak / Critter Chatter /
                // clip audio is on neither tap: those are mixed in after the
                // chain and are not the user's voice.
                if tapping {
                    reading_taps.push_wet(&mono[..native_frames]);
                }

                // Phase 13: tap the processed, input-rate mono into the
                // monitor ring (if a monitor device is active) BEFORE this
                // device's own resample. The monitor stream then resamples it
                // to the headphone device's rate. Dropping samples on a full
                // ring is fine — the monitor just underruns to silence.
                //
                // v1.18.0: monitor-only voices (e.g. a TTS "preview") are
                // mixed in HERE — into the monitor signal only — so the main
                // send (e.g. CABLE → the call) never carries them. With no
                // separate monitor device they ride the single output instead
                // so they stay audible locally. Either branch calls
                // `mix_monitor_into` exactly once per callback, so those
                // voices advance.
                let mic_mon = state_for_callback.monitor.load(Ordering::Acquire);
                let speak_mon = state_for_callback
                    .monitor_soundboard
                    .load(Ordering::Acquire);
                if let Some(mp) = monitor_producer.as_mut() {
                    // v1.29.0: build the monitor mix from INDEPENDENTLY-gated
                    // sources — mic by the Mixer monitor, soundboard/Speak (the
                    // `sb` Play voices + monitor-only previews) by the Speak
                    // monitor — so muting one never silences the other. The ring
                    // is then played ungated in the monitor stream.
                    let mut mon = [0f32; MAX_FRAMES_PER_CALLBACK];
                    if mic_mon {
                        mon[..native_frames].copy_from_slice(&mono[..native_frames]);
                    }
                    if speak_mon {
                        for (m, &s) in mon[..native_frames].iter_mut().zip(&sb[..native_frames]) {
                            *m += s;
                        }
                        soundboard.mix_monitor_into(&mut mon[..native_frames], input_rate);
                    } else {
                        // Still advance the monitor-only voices so clips play through.
                        let mut scratch = [0f32; MAX_FRAMES_PER_CALLBACK];
                        soundboard.mix_monitor_into(&mut scratch[..native_frames], input_rate);
                    }
                    let _ = mp.push_slice(&mon[..native_frames]);
                } else {
                    // No separate monitor device: the single output carries the
                    // monitor-only voices too (gated downstream by the monitor
                    // toggle, exactly as before).
                    soundboard.mix_monitor_into(&mut mono[..native_frames], input_rate);
                }

                // Fold the soundboard `Play` voices back into `mono` for the main
                // send (the call always hears Speak/soundboard output) and the
                // recording tap below.
                for (m, &s) in mono[..native_frames].iter_mut().zip(&sb[..native_frames]) {
                    *m += s;
                }

                // Phase 16: tap the same processed mono into the
                // recording ring while recording is active. The writer
                // thread drains it to a WAV; a full ring just drops
                // samples (a brief gap in the file) rather than stalling
                // the audio thread with file I/O.
                if state_for_callback.recording.load(Ordering::Acquire) {
                    let _ = recording_producer.push_slice(&mono[..native_frames]);
                }

                // Now hand the native-rate buffer to the output
                // pipeline. Either a passthrough (rates match) or via
                // the resampler.
                let mut output_mono = [0f32; MAX_FRAMES_PER_CALLBACK];
                let written_out = if let Some(r) = resampler.as_mut() {
                    r.push_input(&mono[..native_frames]);
                    r.process(&mut output_mono[..out_frames])
                } else {
                    let n = native_frames.min(out_frames);
                    output_mono[..n].copy_from_slice(&mono[..n]);
                    n
                };

                // Phase 13 gating: when a separate monitor device is
                // active, the main output is the *send* (e.g. to
                // VB-Cable) and plays unconditionally so the far end
                // always hears the voice — the monitor toggle then only
                // governs the headphone (monitor) stream. With no
                // separate monitor device, the toggle gates this output
                // directly, exactly as before.
                let play = main_output_plays(has_monitor, monitoring);

                // Fan out mono -> all output channels, or silence if not
                // playing.
                for (i, frame) in data.chunks_exact_mut(channels).enumerate() {
                    if i >= written_out {
                        for s in frame {
                            *s = 0.0;
                        }
                        continue;
                    }
                    let sample = if play { output_mono[i] } else { 0.0 };
                    for s in frame {
                        *s = sample;
                    }
                }
                // Meter reflects what we actually sent to the device.
                let metered = &output_mono[..written_out];
                if play {
                    meter.process(metered);
                } else {
                    meter.process(&[]);
                }
                state.store_output(Levels {
                    rms: meter.rms(),
                    peak: meter.peak(),
                });
            },
            err_fn,
            None,
        )
        .map_err(|e| AudioEngineError::StreamBuild(e.to_string()))?;

    Ok(stream)
}

/// Phase 13: the monitor ("hear yourself") output stream. Unlike the
/// main output, it does NO DSP or soundboard work — it only drains the
/// monitor tap ring (already-processed, input-rate mono fed by the main
/// output callback), resamples it to this device's rate, and fans it
/// out. Gated by the monitor atomic, so the toggle mutes only the
/// headphones, never the main send.
#[allow(clippy::too_many_arguments)] // builder-style; each arg is necessary
fn build_monitor_stream(
    device: &Device,
    config: &StreamConfig,
    sample_format: SampleFormat,
    channels: u16,
    mut consumer: RingConsumer,
    state: Arc<EngineState>,
    input_rate: u32,
    monitor_rate: u32,
) -> Result<Stream, AudioEngineError> {
    let device_name = device.name().unwrap_or_default();
    if sample_format != SampleFormat::F32 {
        return Err(AudioEngineError::UnsupportedSampleFormat {
            device: device_name,
            format: format!("{sample_format:?}"),
        });
    }
    let err_label = device_name;
    let err_fn = move |err: cpal::StreamError| {
        tracing::error!(?err, device = %err_label, "monitor stream error");
    };

    let channels = channels as usize;
    let mut resampler: Option<MonoResampler> = if input_rate == monitor_rate {
        None
    } else {
        Some(MonoResampler::new(input_rate, monitor_rate, 256)?)
    };

    let stream = device
        .build_output_stream(
            config,
            move |data: &mut [f32], _info| {
                if data.is_empty() || channels == 0 {
                    return;
                }
                // v1.29.0: the ring is already gated per-source by the main
                // output callback (mic by `monitor`, preview by
                // `monitor_soundboard`), so this stream no longer gates on
                // `monitor` — it plays whatever it's fed.
                // v1.6.0: user-set monitor volume, applied over the whole mix.
                let monitor_gain = state.load_monitor_gain();
                let out_frames = (data.len() / channels).min(MAX_FRAMES_PER_CALLBACK);

                // Match the main output's native-frame sizing so the
                // resampler is fed a consistent amount per round.
                let native_frames = if let Some(r) = resampler.as_ref() {
                    let scaled = u64::try_from(out_frames)
                        .unwrap_or(u64::MAX)
                        .saturating_mul(u64::from(input_rate))
                        / u64::from(monitor_rate).max(1);
                    let approx = usize::try_from(scaled).unwrap_or(MAX_FRAMES_PER_CALLBACK)
                        + r.input_frames_next();
                    approx.min(MAX_FRAMES_PER_CALLBACK)
                } else {
                    out_frames
                };

                let mut mono = [0f32; MAX_FRAMES_PER_CALLBACK];
                let popped = consumer.pop_slice(&mut mono[..native_frames]);
                for slot in &mut mono[popped..native_frames] {
                    *slot = 0.0;
                }

                let mut output_mono = [0f32; MAX_FRAMES_PER_CALLBACK];
                let written_out = if let Some(r) = resampler.as_mut() {
                    r.push_input(&mono[..native_frames]);
                    r.process(&mut output_mono[..out_frames])
                } else {
                    let n = native_frames.min(out_frames);
                    output_mono[..n].copy_from_slice(&mono[..n]);
                    n
                };

                for (i, frame) in data.chunks_exact_mut(channels).enumerate() {
                    let sample = if i < written_out {
                        output_mono[i] * monitor_gain
                    } else {
                        0.0
                    };
                    for s in frame {
                        *s = sample;
                    }
                }
            },
            err_fn,
            None,
        )
        .map_err(|e| AudioEngineError::StreamBuild(e.to_string()))?;

    Ok(stream)
}

/// Whether the MAIN output device should emit audio this callback.
///
/// Phase 13 semantics: with a separate monitor device active
/// (`has_monitor`), the main output is the *send* (e.g. to VB-Cable)
/// and always plays so the far end keeps hearing the voice — the
/// monitor toggle only governs the headphone stream. With no separate
/// monitor device, the toggle gates this output directly (legacy
/// behavior). Extracted so the truth table is unit-tested.
#[must_use]
fn main_output_plays(has_monitor: bool, monitoring: bool) -> bool {
    has_monitor || monitoring
}

/// Run the DSP chain on the mic mono buffer and normalize the voice's
/// loudness (v1.7.0), then render the soundboard's regular (`Play`) voices into
/// a **separate** `soundboard_out` buffer. Keeping them apart lets the monitor
/// tap gate the mic and the soundboard/Speak output independently (v1.29.0);
/// the caller folds `soundboard_out` back into `mono` afterwards for the main
/// send. Effects + loudness apply only to the user's voice; clips play as-is.
/// Extracted so the order is unit-testable in isolation.
fn mix_voice_and_soundboard(
    mono: &mut [f32],
    soundboard_out: &mut [f32],
    chain: &mut EffectChain,
    modulator: &mut ReactiveModulator,
    loudness: &mut LoudnessNormalizer,
    soundboard: &mut SoundboardMixer,
    sample_rate: u32,
) {
    // v1.46.0: `mono` is the DRY, pre-effects mic signal at exactly this point
    // — the only place on this thread where that is true. Measuring here is
    // what makes the modulation feedback-free: following the chain's own
    // output would mean louder -> more drive -> louder. It must also run
    // before `loudness`, whose entire job is removing the level variation the
    // follower needs.
    modulator.apply(mono, chain, sample_rate);
    chain.process(mono, sample_rate);
    loudness.process(mono, sample_rate);
    soundboard.mix_into(soundboard_out, sample_rate);
}

// ---------------------------------------------------------------------------
// Voice reading — the taps, the worker, and the seam the analysis plugs into.
//
// What this half owns: getting the DRY mic and the chain's OUTPUT off the
// audio thread without allocating, deciding which of the four states the panel
// is in, and holding the last speaking window rather than letting a readout
// decay through every pause. What it deliberately does NOT own: the analysis
// itself. See `crate::dsp::reading` for what a reading is and is not.
// ---------------------------------------------------------------------------

/// Facts the reading is **told** rather than left to infer from a quiet
/// buffer.
///
/// Stopped, muted and quiet-but-present all look the same at the meter, and
/// guessing between them is how a readout ends up publishing a verdict on the
/// speaker every time they stop to breathe.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReadingFacts {
    /// The engine is live. False means there is no signal to read, not a
    /// quiet one.
    pub running: bool,
    /// Something outside the analysis knows the input is muted — an OS or
    /// device mute. Push-to-modulate is deliberately not this: with the key
    /// up the mic is still open and only the chain is bypassed.
    pub muted: bool,
    /// Every sample of a whole analysis window was exactly zero. A device or
    /// OS mute produces that; a quiet room never does, which is what makes it
    /// a fact about the input rather than a level threshold. A brief ring
    /// underrun cannot trigger it either — it takes a window's worth.
    pub digital_silence: bool,
}

impl ReadingFacts {
    /// Resolve the state the panel shows.
    ///
    /// `measured` is the analyzer's verdict about the *signal*, so only
    /// `Speaking` is taken from it; everything else collapses to `Quiet` and
    /// the engine facts decide from there. An analyzer that could return
    /// `Stopped` or `Muted` would be inferring exactly what this split exists
    /// to keep it from inferring.
    #[must_use]
    fn resolve(self, measured: ReadingState) -> ReadingState {
        if !self.running {
            ReadingState::Stopped
        } else if self.muted || self.digital_silence {
            ReadingState::Muted
        } else if matches!(measured, ReadingState::Speaking) {
            ReadingState::Speaking
        } else {
            ReadingState::Quiet
        }
    }
}

/// The seam the voice-reading analysis plugs into.
///
/// A worker thread owns one of these and feeds it aligned dry/wet blocks;
/// pitch tracking, spectra and the rolling baseline all live behind it, off
/// the audio thread. Two things are contractual:
///
/// * `push` returns `Some` exactly when an analysis window completes, so the
///   panel updates at the analysis rate rather than the callback rate.
/// * the returned `state` is a verdict about the signal — `Speaking` or
///   `Quiet`. The engine supplies stopped and muted (see [`ReadingFacts`]).
pub trait ReadingAnalyzer: Send {
    /// Feed one aligned pair of blocks, `dry` before the chain and `wet`
    /// after it, both `sample_rate` Hz and the same length.
    fn push(&mut self, dry: &[f32], wet: &[f32], sample_rate: u32) -> Option<VoiceReading>;

    /// The effect chain changed, so anything measured from the wet tap
    /// describes a chain that no longer exists.
    fn note_chain_changed(&mut self);

    /// Forget the rolling baseline.
    ///
    /// Called when the panel is switched on, and implied by a new session
    /// (the worker is per-session), so a reading is never relative to a
    /// baseline built on a different mic. The baseline is per-session by
    /// design and is never written to disk — it derives from the user's
    /// voice, and nothing derived from the user's voice is persisted.
    fn reset(&mut self);
}

/// Bridges the analysis engine to the worker's seam.
///
/// Two things the seam doesn't carry and this fills in:
///
/// * the sample rate, which [`Analyzer`] wants once at construction while the
///   seam passes it per block — so the analyzer is built on the first block
///   and rebuilt if the device rate ever changes under it;
/// * whether the chain was a passthrough over the window. Nothing in the
///   engine holds a "bypassed" flag: push-to-modulate toggles effects one at
///   a time, and a Clean preset is bypass by another name. So this measures
///   it instead of trusting plumbing — see [`is_passthrough`] — which is
///   right for every cause rather than the one we remembered to wire.
struct LiveAnalyzer {
    /// The rate it was built for, and the analyzer itself.
    inner: Option<(u32, Analyzer)>,
    /// Every block since the last reading was a passthrough. Reset when a
    /// reading is emitted, so the flag describes that window and not just
    /// its final block.
    bypassed: bool,
}

impl Default for LiveAnalyzer {
    fn default() -> Self {
        Self {
            inner: None,
            bypassed: true,
        }
    }
}

impl ReadingAnalyzer for LiveAnalyzer {
    fn push(&mut self, dry: &[f32], wet: &[f32], sample_rate: u32) -> Option<VoiceReading> {
        if let Some(passthrough) = is_passthrough(dry, wet) {
            self.bypassed &= passthrough;
        }
        let bypassed = self.bypassed;
        let analyzer = match &mut self.inner {
            Some((rate, analyzer)) if *rate == sample_rate => analyzer,
            slot => {
                *slot = Some((sample_rate, Analyzer::new(sample_rate)));
                &mut slot.as_mut().expect("just set").1
            }
        };
        // `engine_running` and `input_muted` are the engine's to answer, and
        // it layers them on afterwards through `ReadingFacts::resolve`; what
        // the analyzer decides here is only whether the signal is speech.
        let facts = InputFacts {
            engine_running: true,
            input_muted: false,
            chain_bypassed: bypassed,
        };
        let out = analyzer.observe(dry, wet, facts);
        if out.is_some() {
            self.bypassed = true;
        }
        out
    }

    fn note_chain_changed(&mut self) {
        if let Some((_, analyzer)) = &mut self.inner {
            analyzer.note_chain_changed();
        }
        self.bypassed = true;
    }

    fn reset(&mut self) {
        if let Some((_, analyzer)) = &mut self.inner {
            analyzer.reset_session();
        }
        self.bypassed = true;
    }
}

/// Drop what is queued on both taps without losing their pairing.
///
/// Sized by WET, dropping the same count from each. Two independent drains
/// race the callback: empty DRY, and if the panel is switched on before WET
/// is emptied, one buffer's DRY half survives while its WET half is eaten.
/// The taps are then misaligned by that buffer for the rest of the session
/// and nothing resyncs them. DRY is pushed first, so `dry >= wet` always
/// holds and equal skips keep the halves paired whatever the callback does
/// in between; a DRY buffer whose WET half has not arrived yet is left
/// alone, which is exactly right — its partner is on the way.
fn drop_paired(
    dry: &mut RingConsumer,
    wet: &mut RingConsumer,
    dry_buf: &mut [f32],
    wet_buf: &mut [f32],
) {
    let mut skip = wet.occupied_len();
    while skip > 0 {
        let want = skip.min(wet_buf.len()).min(dry_buf.len());
        let took = wet.pop_slice(&mut wet_buf[..want]);
        if took == 0 {
            break;
        }
        dry.pop_slice(&mut dry_buf[..took]);
        skip -= took;
    }
}

/// Whether `wet` is `dry` passed through, ignoring level.
///
/// Scale-invariant on purpose: the wet tap sits after the loudness stage, so
/// a bypassed chain still arrives at a different level. `None` when the block
/// is too quiet to tell, so silence never votes either way.
fn is_passthrough(dry: &[f32], wet: &[f32]) -> Option<bool> {
    const QUIET: f64 = 1e-9;
    /// Correlation above this is the same waveform. Any real effect — even a
    /// gentle EQ — lands far below it.
    const SAME: f64 = 0.999;

    let n = dry.len().min(wet.len());
    let (mut dd, mut ww, mut dw) = (0.0_f64, 0.0_f64, 0.0_f64);
    for i in 0..n {
        let (d, w) = (f64::from(dry[i]), f64::from(wet[i]));
        dd += d * d;
        ww += w * w;
        dw += d * w;
    }
    if dd < QUIET || ww < QUIET {
        return None;
    }
    Some(dw / (dd.sqrt() * ww.sqrt()) >= SAME)
}

/// Constructs the analyzer the reading worker runs.
///
/// The only place the analysis implementation is named. The taps, the worker,
/// the freeze and the wire shape are all written against [`ReadingAnalyzer`].
fn new_reading_analyzer() -> Box<dyn ReadingAnalyzer> {
    Box::new(LiveAnalyzer::default())
}

/// What the panel shows: a reading, plus whether it is still live.
///
/// `stale` is the answer to this feature's worst failure mode. Most of a
/// session is not speech, and a readout that decayed through the pauses toward
/// "quiet, flat, narrow" would be delivering a verdict on the speaker several
/// times a minute without printing a word. Instead the last speaking window is
/// **held** and flagged, and `state` says why it is being held.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReadingSnapshot {
    /// The reading itself. Its own keys stay as `crate::dsp::reading` defines
    /// them (`energy_dbfs`, `f0_hz`, …) — only this wrapper is camelCase.
    pub reading: VoiceReading,
    /// The metrics come from an earlier window, not from now.
    pub stale: bool,
    /// How long ago that window was measured, in ms. 0 while live.
    pub age_ms: u32,
}

impl ReadingSnapshot {
    /// A snapshot that measures nothing and holds nothing.
    #[must_use]
    pub fn idle(state: ReadingState) -> Self {
        Self {
            reading: VoiceReading::idle(state),
            stale: false,
            age_ms: 0,
        }
    }
}

/// Shared state for the voice-reading panel.
///
/// Lives beside [`EngineState`] rather than inside it because only this
/// feature touches it. The audio callback reads `enabled` and nothing else —
/// there is no lock on the realtime path. The `Mutex` sits between the worker
/// thread and the UI thread, neither of which is realtime.
#[derive(Debug)]
pub struct ReadingTap {
    enabled: AtomicBool,
    muted: AtomicBool,
    /// The chain changed since the worker last looked.
    ///
    /// Set on the audio thread (one relaxed store beside the command it
    /// already applied) and taken by the worker, because the two run on
    /// different threads and the worker cannot see the chain.
    chain_changed: AtomicBool,
    snapshot: Mutex<ReadingSnapshot>,
}

impl Default for ReadingTap {
    fn default() -> Self {
        Self {
            // Off by default: no analysis runs unless the user asks for it.
            enabled: AtomicBool::new(false),
            muted: AtomicBool::new(false),
            chain_changed: AtomicBool::new(false),
            snapshot: Mutex::new(ReadingSnapshot::idle(ReadingState::Stopped)),
        }
    }
}

impl ReadingTap {
    fn is_enabled(&self) -> bool {
        self.enabled.load(Ordering::Acquire)
    }

    fn is_muted(&self) -> bool {
        self.muted.load(Ordering::Acquire)
    }

    /// Tell the reading the effect chain changed under it.
    pub fn note_chain_changed(&self) {
        self.chain_changed.store(true, Ordering::Release);
    }

    /// Whether the chain changed since this was last asked, clearing the flag.
    fn take_chain_changed(&self) -> bool {
        self.chain_changed.swap(false, Ordering::AcqRel)
    }

    /// A poisoned lock means the worker panicked mid-write. The value behind
    /// it is a plain snapshot, so carrying on with it beats taking the UI
    /// thread down as well.
    fn store(&self, snapshot: ReadingSnapshot) {
        *self.snapshot.lock().unwrap_or_else(PoisonError::into_inner) = snapshot;
    }

    fn load(&self) -> ReadingSnapshot {
        self.snapshot
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

/// The producer ends of the two taps, owned by the output callback.
///
/// Both or neither. The DRY copy is taken before the chain overwrites the
/// buffer in place and the WET copy after it, so a push that landed on one
/// ring and was truncated on the other would slide the two taps out of step
/// for the rest of the session — the later readings would compare a speaker
/// against a different moment of themselves.
struct ReadingTaps {
    dry: RingProducer,
    wet: RingProducer,
}

impl ReadingTaps {
    /// Whether this callback taps. Checked once, before the DRY copy, and the
    /// answer still holds at the WET copy: only this thread pushes, and the
    /// worker only ever frees space, so the room reserved here cannot vanish.
    fn armed(&self, enabled: bool, frames: usize) -> bool {
        enabled && frames > 0 && self.dry.vacant_len() >= frames && self.wet.vacant_len() >= frames
    }

    fn push_dry(&mut self, block: &[f32]) {
        let _ = self.dry.push_slice(block);
    }

    fn push_wet(&mut self, block: &[f32]) {
        let _ = self.wet.push_slice(block);
    }
}

/// Semitone distance from `a` to `b`.
fn semitones_between(a: f32, b: f32) -> f32 {
    12.0 * (b / a).log2()
}

/// Whether `b` sits within [`OCTAVE_TOLERANCE_ST`] of an exact octave (or two)
/// from `a` — the shape of an f0 tracker locking onto the wrong harmonic.
fn is_octave_jump(a: f32, b: f32) -> bool {
    if !(a.is_finite() && b.is_finite()) || a <= 0.0 || b <= 0.0 {
        return false;
    }
    let st = semitones_between(a, b).abs();
    (st - 12.0).abs() <= OCTAVE_TOLERANCE_ST || (st - 24.0).abs() <= OCTAVE_TOLERANCE_ST
}

/// Whether two windows landed close enough to be the same answer twice.
fn pitches_agree(a: f32, b: f32) -> bool {
    a > 0.0 && b > 0.0 && semitones_between(a, b).abs() <= OCTAVE_TOLERANCE_ST
}

/// Holds back the pitch jump an f0 tracker makes when it locks onto the wrong
/// harmonic.
///
/// Octave errors are what users actually see. On a gaming headset behind a
/// gate and a denoiser the tracker will halve or double from time to time, and
/// a pitch readout flickering between two answers is worse than no readout at
/// all. So a move landing near an exact octave is not shown until a second
/// window agrees with it: a real change of register survives one window's
/// delay, a tracker error does not repeat cleanly.
///
/// DRY only. The WET half is *supposed* to jump an octave the moment a preset
/// says so — that is the entire point of the after-effects reading — and a
/// confirm-before-showing rule there would fight the feature.
#[derive(Debug, Default)]
struct OctaveGuard {
    /// The last pitch pair actually shown.
    last: Option<(f32, f32)>,
    /// An unconfirmed jump, waiting for a second window to agree.
    pending: Option<f32>,
}

impl OctaveGuard {
    fn reset(&mut self) {
        self.last = None;
        self.pending = None;
    }

    /// Replace this window's pitch pair with the last accepted one when the
    /// jump looks like a tracker error. The guard only ever holds a value
    /// back; it never invents one.
    fn apply(&mut self, metrics: &mut Metrics) {
        let f0 = metrics.f0_hz;
        if !f0.is_finite() || f0 <= 0.0 {
            // Nothing voiced. A pause is not a confirmation, so drop any
            // pending jump rather than let silence vouch for it.
            self.pending = None;
            return;
        }
        let Some((last_f0, last_range)) = self.last else {
            self.accept(metrics);
            return;
        };
        if !is_octave_jump(last_f0, f0) {
            self.accept(metrics);
            return;
        }
        if self.pending.is_some_and(|p| pitches_agree(p, f0)) {
            self.accept(metrics);
        } else {
            self.pending = Some(f0);
            metrics.f0_hz = last_f0;
            metrics.f0_range_st = last_range;
        }
    }

    fn accept(&mut self, metrics: &Metrics) {
        self.last = Some((metrics.f0_hz, metrics.f0_range_st));
        self.pending = None;
    }
}

/// Build the snapshot to publish: the held window, re-labelled with the
/// current state, or nothing once it is too old to mean anything.
///
/// Never a decayed version of it. Freezing is the point — a readout that
/// drifted toward "quiet, flat, narrow" through every pause would be passing
/// judgement on the speaker rather than reporting a measurement.
fn reading_snapshot(
    held: Option<&(VoiceReading, Instant)>,
    state: ReadingState,
) -> ReadingSnapshot {
    let Some((reading, at)) = held else {
        return ReadingSnapshot::idle(state);
    };
    let age = at.elapsed();
    if age.as_secs() >= READING_HOLD_SECS {
        // Dropping the stale NUMBERS does not un-learn the baseline. Returning
        // a bare idle here made the panel announce "still listening — not
        // enough speech yet to compare against" after any 30 second pause,
        // which is false: the analyzer stays calibrated for the session. The
        // same silence at 29 seconds correctly read "too quiet to measure",
        // so the panel got less truthful the longer it waited.
        let mut expired = ReadingSnapshot::idle(state);
        expired.reading.calibrated = reading.calibrated;
        return expired;
    }
    let held_over = !matches!(state, ReadingState::Speaking);
    let mut reading = reading.clone();
    reading.state = state;
    // Keep the flag honest. This path re-labels the last *speaking* reading
    // rather than using the analyzer's own held output — which is what keeps
    // the phrase on screen through a pause instead of blinking it out — but
    // it left `held` reading false on a snapshot whose `stale` was true, on a
    // surface the docs describe as frozen.
    reading.held = held_over;
    ReadingSnapshot {
        reading,
        stale: held_over,
        age_ms: if held_over {
            u32::try_from(age.as_millis()).unwrap_or(u32::MAX)
        } else {
            0
        },
    }
}

/// Whether a `Speaking` verdict has gone stale because no analysis window has
/// completed in a while.
///
/// The same failure as a decaying readout wearing a different costume: a
/// stalled device stops producing callbacks while the engine still reports
/// itself running, and the panel would go on claiming a live reading of a
/// signal that stopped arriving.
fn live_verdict_expired(measured: ReadingState, since_last_window: Duration) -> bool {
    matches!(measured, ReadingState::Speaking)
        && since_last_window >= Duration::from_millis(READING_LIVE_TIMEOUT_MS)
}

/// The voice-reading worker thread.
///
/// Owns the consumer ends of both taps plus the analyzer, and publishes a
/// snapshot the UI thread reads. Everything expensive — pitch tracking, FFTs,
/// the baseline — happens here and never in the callback. Exits when `alive`
/// disconnects at teardown, which is also what invalidates the baseline: a new
/// session gets a new worker and therefore a new one.
#[allow(clippy::needless_pass_by_value)] // owns everything for the thread's lifetime
fn reading_worker(
    mut dry: RingConsumer,
    mut wet: RingConsumer,
    tap: Arc<ReadingTap>,
    state: Arc<EngineState>,
    alive: Receiver<()>,
    sample_rate: u32,
) {
    let mut analyzer = new_reading_analyzer();
    let mut guard = OctaveGuard::default();
    let mut held: Option<(VoiceReading, Instant)> = None;
    let mut measured = ReadingState::Quiet;
    let mut last_window = Instant::now();
    let mut was_enabled = false;
    let mut all_zero = true;
    let mut dry_buf = [0f32; MAX_FRAMES_PER_CALLBACK];
    let mut wet_buf = [0f32; MAX_FRAMES_PER_CALLBACK];

    loop {
        if matches!(alive.try_recv(), Err(TryRecvError::Disconnected)) {
            return;
        }
        let enabled = tap.is_enabled();
        if enabled != was_enabled {
            was_enabled = enabled;
            // Switching the panel on starts a fresh baseline; switching it off
            // throws away what it had. Neither survives to disk.
            analyzer.reset();
            guard.reset();
            held = None;
            measured = ReadingState::Quiet;
            last_window = Instant::now();
            all_zero = true;
        }

        let mut facts = ReadingFacts {
            running: state.running.load(Ordering::Acquire),
            muted: tap.is_muted(),
            digital_silence: false,
        };

        // Drop the wet window whenever the chain changed under it: those
        // samples describe a chain that no longer exists, and holding them
        // meant the card named the new preset beside the old one's numbers.
        // Taken every pass, enabled or not, so the flag never goes stale.
        if tap.take_chain_changed() {
            analyzer.note_chain_changed();
        }

        if !enabled {
            // Discard anything the callback pushed around the edge of the
            // toggle, so stale audio never lands at the head of a new window.
            //
            // Drop the SAME COUNT from each, sized by WET. Two independent
            // unbounded drains race the callback: empty DRY, and if the panel
            // is switched on before WET is emptied, one buffer's DRY half
            // survives while its WET half is eaten. The taps are then
            // misaligned by that buffer for the rest of the session, and
            // nothing resyncs them — review reproduced it. DRY is pushed
            // first, so `dry_occupied >= wet_occupied` always holds and equal
            // skips keep the halves paired whatever the callback does in
            // between.
            drop_paired(&mut dry, &mut wet, &mut dry_buf, &mut wet_buf);
            tap.store(ReadingSnapshot::idle(facts.resolve(ReadingState::Quiet)));
            std::thread::sleep(Duration::from_millis(READING_DRAIN_INTERVAL_MS));
            continue;
        }

        let mut published = false;
        loop {
            // Size the drain by WET. It is pushed after the chain runs, so it
            // trails DRY by at most one buffer; draining by DRY would eat
            // samples whose WET half has not been written yet and the two taps
            // would never line up again.
            let want = wet.occupied_len().min(MAX_FRAMES_PER_CALLBACK);
            if want == 0 {
                break;
            }
            let n_wet = wet.pop_slice(&mut wet_buf[..want]);
            let n = dry.pop_slice(&mut dry_buf[..n_wet]).min(n_wet);
            if n == 0 {
                break;
            }
            all_zero &= dry_buf[..n].iter().all(|s| *s == 0.0);
            if let Some(mut reading) = analyzer.push(&dry_buf[..n], &wet_buf[..n], sample_rate) {
                facts.digital_silence = all_zero;
                all_zero = true;
                measured = reading.state;
                let resolved = facts.resolve(measured);
                if matches!(resolved, ReadingState::Speaking) {
                    guard.apply(&mut reading.dry);
                    reading.state = resolved;
                    held = Some((reading, Instant::now()));
                }
                tap.store(reading_snapshot(held.as_ref(), resolved));
                last_window = Instant::now();
                published = true;
            }
        }

        if !published {
            // Keep the state honest even when no audio arrives at all: the
            // engine stopping mid-pause produces no windows, and the panel
            // must not go on showing a reading for a session that is down.
            // This also keeps `age_ms` ticking while a held reading is shown.
            if live_verdict_expired(measured, last_window.elapsed()) {
                measured = ReadingState::Quiet;
            }
            facts.digital_silence = false;
            tap.store(reading_snapshot(held.as_ref(), facts.resolve(measured)));
        }

        std::thread::sleep(Duration::from_millis(READING_DRAIN_INTERVAL_MS));
    }
}

/// The WAV writer the recording thread holds while a file is open.
type RecordingFile = hound::WavWriter<std::io::BufWriter<std::fs::File>>;

/// Pop everything currently in the recording ring and write it as
/// 16-bit PCM. f32 samples are clamped to [-1, 1] before scaling so a
/// hot signal saturates cleanly instead of wrapping.
fn drain_recording(
    consumer: &mut RingConsumer,
    buf: &mut [f32; MAX_FRAMES_PER_CALLBACK],
    writer: &mut RecordingFile,
) {
    loop {
        let n = consumer.pop_slice(buf);
        if n == 0 {
            break;
        }
        for &s in &buf[..n] {
            #[allow(clippy::cast_possible_truncation)] // clamped to i16 range
            let v = (s.clamp(-1.0, 1.0) * f32::from(i16::MAX)) as i16;
            let _ = writer.write_sample(v);
        }
    }
}

/// Phase 16: recording writer thread body. Owns the consumer end of the
/// recording ring plus a command channel. On `Start { path }` it opens a
/// 16-bit PCM mono WAV at `sample_rate`; while a file is open it drains
/// the ring (f32 → i16) into it. `Stop` flushes the tail and finalizes.
/// A disconnected command channel (engine teardown) finalizes any open
/// file and exits. All file I/O lives here so the audio callback never
/// touches the disk.
#[allow(clippy::needless_pass_by_value)] // owns the receiver for the thread's lifetime
fn recording_writer(mut consumer: RingConsumer, rx: Receiver<RecordingCommand>, sample_rate: u32) {
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut writer: Option<RecordingFile> = None;
    let mut buf = [0f32; MAX_FRAMES_PER_CALLBACK];

    loop {
        // Apply all pending commands first.
        loop {
            match rx.try_recv() {
                Ok(RecordingCommand::Start { path }) => {
                    // Finalize any in-progress file before starting anew.
                    if let Some(w) = writer.take() {
                        let _ = w.finalize();
                    }
                    match hound::WavWriter::create(&path, spec) {
                        Ok(w) => writer = Some(w),
                        Err(e) => {
                            tracing::error!(
                                error = %e,
                                path = %path.display(),
                                "failed to create recording WAV"
                            );
                        }
                    }
                }
                Ok(RecordingCommand::Stop { reply }) => {
                    if let Some(mut w) = writer.take() {
                        drain_recording(&mut consumer, &mut buf, &mut w);
                        let _ = w.finalize();
                    }
                    // v1.23.0: signal the caller the file is fully on disk.
                    if let Some(r) = reply {
                        let _ = r.send(());
                    }
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    // Engine teardown: flush the tail, finalize, exit.
                    if let Some(mut w) = writer.take() {
                        drain_recording(&mut consumer, &mut buf, &mut w);
                        let _ = w.finalize();
                    }
                    return;
                }
            }
        }

        if let Some(w) = writer.as_mut() {
            drain_recording(&mut consumer, &mut buf, w);
        } else {
            // Not recording: discard ring contents so stale audio never
            // leaks into the next file.
            while consumer.pop_slice(&mut buf) > 0 {}
        }

        std::thread::sleep(Duration::from_millis(15));
    }
}

#[cfg(test)]
mod tests {
    use super::{
        drain_dsp_edits, drain_recording, drop_paired, graveyard_thread, is_octave_jump,
        is_passthrough, live_verdict_expired, main_output_plays, mix_voice_and_soundboard,
        new_reading_analyzer, reading_snapshot, AudioEngine, LoudnessNormalizer, OctaveGuard,
        ReadingFacts, ReadingSnapshot, ReadingTap, ReadingTaps, StreamInfo,
        MAX_FRAMES_PER_CALLBACK, READING_HOLD_SECS, RING_BUFFER_FRAMES,
    };
    use crate::dsp::witness::witnessed_chain;
    use crate::dsp::{
        Descriptor, Displaced, DspEdit, EffectChain, Metrics, ReactiveModulator, ReadingState,
        VoiceReading,
    };
    use crate::soundboard::{SoundboardCommand, SoundboardMixer};
    use ringbuf::traits::{Observer, Producer, Split};
    use ringbuf::HeapRb;
    use std::sync::atomic::Ordering;
    use std::sync::mpsc::{channel, sync_channel};
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    /// Phase 13 gating truth table: the main output (the "send") always
    /// plays when a separate monitor device exists, so routing to
    /// VB-Cable keeps reaching Discord regardless of the monitor toggle;
    /// with no monitor device the toggle gates the output (legacy).
    #[test]
    fn main_output_gating_truth_table() {
        // No separate monitor → toggle gates the output (legacy).
        assert!(!main_output_plays(false, false));
        assert!(main_output_plays(false, true));
        // Separate monitor → main output is the send, always plays.
        assert!(main_output_plays(true, false));
        assert!(main_output_plays(true, true));
    }

    /// v1.0 freeze: `StreamInfo` is returned from `start_audio_engine`
    /// and consumed by the frontend, so its camelCase JSON keys are a
    /// stable contract. See `docs/STABLE-SURFACE.md`.
    #[test]
    fn stream_info_json_keys_are_frozen() {
        let info = StreamInfo {
            input_name: "in".into(),
            output_name: "out".into(),
            monitor_name: Some("mon".into()),
            sample_rate: 48_000,
            input_channels: 1,
            output_channels: 2,
        };
        let v = serde_json::to_value(&info).unwrap();
        let mut keys: Vec<String> = v.as_object().unwrap().keys().cloned().collect();
        keys.sort();
        assert_eq!(
            keys,
            [
                "inputChannels",
                "inputName",
                "monitorName",
                "outputChannels",
                "outputName",
                "sampleRate"
            ]
        );
    }

    #[test]
    fn engine_starts_and_stops_cleanly_even_without_audio_hardware() {
        // CI runners have no real devices, so start() will likely fail.
        // What we're verifying here is that constructing and dropping
        // the engine (which spins up and joins the audio thread) doesn't
        // hang or panic.
        let engine = AudioEngine::new();
        assert!(!engine.is_running());
        // v1.39.0: a fresh engine has not "failed recovery" (the banner flag
        // defaults off); a plain Stop leaves it off too.
        assert!(!engine.device_recovery_failed());
        engine.stop();
        assert!(!engine.device_recovery_failed());
        engine.set_monitor(false);
        drop(engine);
    }

    /// Phase 16: the recording drain converts f32 → 16-bit PCM, clamping
    /// hot samples to the i16 range instead of wrapping, and writes a
    /// valid mono WAV that hound can read back. This is the property
    /// users care about: a saved take matches what they heard (and a
    /// clipped signal saturates cleanly rather than glitching).
    #[test]
    fn drain_recording_writes_clamped_pcm() {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("divora-rec-{nanos}"));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("take.wav");

        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: 48_000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut writer = hound::WavWriter::create(&path, spec).unwrap();

        let rb = HeapRb::<f32>::new(RING_BUFFER_FRAMES);
        let (mut prod, mut cons) = rb.split();
        let mut buf = [0f32; MAX_FRAMES_PER_CALLBACK];

        // Zero, normal, and over-unity samples (the last two must clamp).
        let pushed = prod.push_slice(&[0.0, 0.5, -0.5, 2.0, -2.0]);
        assert_eq!(pushed, 5);

        drain_recording(&mut cons, &mut buf, &mut writer);
        writer.finalize().unwrap();

        let reader = hound::WavReader::open(&path).unwrap();
        assert_eq!(reader.spec().channels, 1);
        assert_eq!(reader.spec().sample_rate, 48_000);
        assert_eq!(reader.spec().bits_per_sample, 16);
        let samples: Vec<i16> = reader.into_samples::<i16>().map(Result::unwrap).collect();
        // 0.5 * 32767 = 16383.5 → truncates to 16383; ±2.0 clamps to ±1.0.
        assert_eq!(samples, vec![0, 16383, -16383, 32767, -32767]);

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// v1.23.0: the dry-input reference recorder is idle on a fresh engine,
    /// arming it without a live session is a no-op (stays idle), and stopping
    /// returns promptly (the engine thread replies even with no writer, so the
    /// synchronous stop never deadlocks).
    #[test]
    fn reference_recording_idle_without_session_and_stop_never_hangs() {
        let engine = AudioEngine::new();
        assert!(!engine.is_reference_recording());
        // No live streams → the writer doesn't exist; arming can't take effect.
        engine.start_reference_recording(std::env::temp_dir().join("divora-ref-noop.wav"));
        assert!(!engine.is_reference_recording());
        // Synchronous stop must still return (engine thread is alive).
        assert!(engine.stop_reference_recording());
        assert!(!engine.is_reference_recording());
    }

    /// v1.29.0: the Speak/soundboard monitor defaults on (TTS previews audible)
    /// and toggles independently — disabling the mic monitor can't silence it.
    #[test]
    fn speak_monitor_defaults_on_and_toggles() {
        let engine = AudioEngine::new();
        assert!(engine.is_speak_monitoring(), "previews audible by default");
        engine.set_speak_monitor(false);
        assert!(!engine.is_speak_monitoring());
        engine.set_speak_monitor(true);
        assert!(engine.is_speak_monitoring());
    }

    /// v1.6.0: monitor gain defaults to unity and `set_monitor_gain`
    /// clamps to a safe range.
    #[test]
    #[allow(clippy::float_cmp)]
    fn monitor_gain_defaults_to_unity_and_clamps() {
        let engine = AudioEngine::new();
        assert_eq!(engine.monitor_gain(), 1.0);
        engine.set_monitor_gain(1.8);
        assert!((engine.monitor_gain() - 1.8).abs() < 1e-6);
        engine.set_monitor_gain(99.0);
        assert_eq!(engine.monitor_gain(), 4.0); // clamped high
        engine.set_monitor_gain(-3.0);
        assert_eq!(engine.monitor_gain(), 0.0); // clamped low
    }

    /// v1.7.0: loudness normalization is off by default with a sane target,
    /// and the setters round-trip through shared state.
    #[test]
    #[allow(clippy::float_cmp)]
    fn loudness_defaults_off_with_sane_target_and_setters_roundtrip() {
        let engine = AudioEngine::new();
        assert!(!engine.loudness_enabled(), "opt-in: defaults off");
        assert_eq!(engine.loudness_target(), super::DEFAULT_TARGET_DBFS);
        assert_eq!(engine.loudness_gain_db(), 0.0, "0 dB readout while idle");
        engine.set_loudness_enabled(true);
        assert!(engine.loudness_enabled());
        engine.set_loudness_target(-22.0);
        assert!((engine.loudness_target() - (-22.0)).abs() < 1e-6);
    }

    #[test]
    #[allow(clippy::float_cmp)] // bit-exact zero on a fresh engine
    fn engine_levels_default_to_zero() {
        let engine = AudioEngine::new();
        let input = engine.input_levels();
        let output = engine.output_levels();
        assert_eq!(input.rms, 0.0);
        assert_eq!(input.peak, 0.0);
        assert_eq!(output.rms, 0.0);
        assert_eq!(output.peak, 0.0);
    }

    /// Phase 11 regression (updated v1.29.0) — the mic stays in `mono` and the
    /// soundboard clips render into the SEPARATE `soundboard_out` buffer (so the
    /// monitor can gate them independently). The property users care about still
    /// holds: their SUM is what reaches the output (the caller folds them
    /// together), so routing into CABLE Input still carries clips to the call.
    #[test]
    fn soundboard_clips_render_into_their_own_buffer_then_sum_with_mic() {
        let mut chain = EffectChain::new(); // empty → mic samples pass through
        let mut sb = SoundboardMixer::new();
        // Inject a 4096-sample clip of constant 0.25 at 48 kHz so the
        // voice doesn't run dry within the 480-sample mix window.
        let clip = Arc::new(vec![0.25_f32; 4096]);
        sb.apply(SoundboardCommand::Play {
            clip_id: "test".to_string(),
            samples: clip,
            sample_rate: 48_000,
            gain: 1.0,
        });
        // Pretend the mic delivered a 480-sample buffer of constant 0.10.
        let mut mono = vec![0.10_f32; 480];
        let mut sb_out = vec![0.0_f32; 480];
        // Loudness normalizer defaults to disabled → bit-exact passthrough.
        let mut loudness = LoudnessNormalizer::new();
        // Disabled by default, so it is inert here.
        let mut modulator = ReactiveModulator::new();
        mix_voice_and_soundboard(
            &mut mono,
            &mut sb_out,
            &mut chain,
            &mut modulator,
            &mut loudness,
            &mut sb,
            48_000,
        );
        for (i, (&m, &s)) in mono.iter().zip(&sb_out).enumerate() {
            assert!(
                (m - 0.10).abs() < 1e-6,
                "mic stays in mono at i={i}, got {m}"
            );
            assert!(
                (s - 0.25).abs() < 1e-5,
                "clip lands in sb_out at i={i}, got {s}"
            );
            assert!(
                (m + s - 0.35).abs() < 1e-5,
                "their sum is the output at i={i}, got {}",
                m + s
            );
        }
    }

    /// Same scenario without a playing clip: the mic samples reach the
    /// output untouched by the (empty) soundboard mix.
    #[test]
    fn mic_only_passes_through_when_no_clip_is_playing() {
        let mut chain = EffectChain::new();
        let mut sb = SoundboardMixer::new();
        let mut mono = vec![0.10_f32; 480];
        let mut sb_out = vec![0.0_f32; 480];
        let mut loudness = LoudnessNormalizer::new();
        // Disabled by default, so it is inert here.
        let mut modulator = ReactiveModulator::new();
        mix_voice_and_soundboard(
            &mut mono,
            &mut sb_out,
            &mut chain,
            &mut modulator,
            &mut loudness,
            &mut sb,
            48_000,
        );
        for (&m, &s) in mono.iter().zip(&sb_out) {
            assert!(
                (m - 0.10).abs() < 1e-6,
                "mic-only path mutated the mic, got {m}"
            );
            assert!(s.abs() < 1e-9, "no clip → empty soundboard buffer, got {s}");
        }
    }
    // ---- v1.46.0: reactive modulation wiring ----

    /// A phase-continuous 440 Hz tone at `amp`. Must be AC: the modulator's
    /// detector high-passes at 110 Hz, so a DC buffer reads as silence (which
    /// is correct, and makes a constant-value test signal useless here).
    struct Tone {
        phase: f32,
    }

    impl Tone {
        fn new() -> Self {
            Self { phase: 0.0 }
        }
        fn block(&mut self, amp: f32, frames: usize) -> Vec<f32> {
            let inc = std::f32::consts::TAU * 440.0 / 48_000.0;
            (0..frames)
                .map(|_| {
                    self.phase = (self.phase + inc) % std::f32::consts::TAU;
                    amp * self.phase.sin()
                })
                .collect()
        }
    }

    /// Build a chain of one Distortion so a route has something to target.
    fn distortion_chain(drive: f32) -> EffectChain {
        use crate::dsp::{EffectKind, EffectSpec};
        let mut params = std::collections::HashMap::new();
        params.insert("drive".to_string(), drive);
        EffectChain::from_specs(&[EffectSpec {
            kind: EffectKind::Distortion,
            enabled: true,
            params,
        }])
    }

    fn rage_config(enabled: bool, base: f32) -> crate::dsp::ResolvedReactive {
        use crate::dsp::{EffectKind, ReactiveConfig, ReactiveRouteSpec};
        ReactiveConfig {
            enabled,
            intensity: 1.0,
            routes: vec![ReactiveRouteSpec {
                kind: EffectKind::Distortion,
                nth: 0,
                key: "drive".to_string(),
                base,
                depth: 45.0,
            }],
            ..ReactiveConfig::default()
        }
        .resolve()
    }

    /// The load-bearing property: the modulator observes the DRY buffer, so
    /// what the chain does to the signal cannot feed back into the modulation.
    ///
    /// Two things this test learned the hard way. It must drive
    /// `mix_voice_and_soundboard`, because the call order there IS the property
    /// under test. And the two chains must differ in a way that survives the
    /// comparison: an earlier version used distortion at drive 10 vs 100, but
    /// `tanh` saturates both to a similar level, so inverting the production
    /// order still passed. Comparing a PASSTHROUGH chain against a heavily
    /// saturating one makes the observed levels differ enormously if the
    /// modulator ever sees post-chain audio.
    ///
    /// Neither modulator carries routes: this isolates what they OBSERVE from
    /// what they write.
    #[test]
    fn modulation_source_is_dry_so_it_cannot_self_feed() {
        use crate::dsp::{ReactiveConfig, ReactiveModulator};
        let observe_only = ReactiveConfig {
            enabled: true,
            intensity: 1.0,
            routes: vec![],
            ..ReactiveConfig::default()
        }
        .resolve();

        let mut passthrough = EffectChain::new();
        let mut saturating = distortion_chain(90.0);
        let mut m_dry = ReactiveModulator::new();
        let mut m_hot = ReactiveModulator::new();
        m_dry.configure(&observe_only, &mut passthrough);
        m_hot.configure(&observe_only, &mut saturating);

        let mut loud1 = LoudnessNormalizer::new();
        let mut loud2 = LoudnessNormalizer::new();
        let mut sb1 = SoundboardMixer::new();
        let mut sb2 = SoundboardMixer::new();

        let mut t1 = Tone::new();
        let mut t2 = Tone::new();
        for _ in 0..60 {
            let mut a = t1.block(0.15, 480);
            let mut b = t2.block(0.15, 480);
            let mut o1 = vec![0.0_f32; 480];
            let mut o2 = vec![0.0_f32; 480];
            mix_voice_and_soundboard(
                &mut a,
                &mut o1,
                &mut passthrough,
                &mut m_dry,
                &mut loud1,
                &mut sb1,
                48_000,
            );
            mix_voice_and_soundboard(
                &mut b,
                &mut o2,
                &mut saturating,
                &mut m_hot,
                &mut loud2,
                &mut sb2,
                48_000,
            );
        }

        // Must actually be modulating, or this compares two zeros and would
        // pass with the feature entirely broken.
        assert!(
            m_dry.depth() > 0.1 && m_dry.depth() < 0.99,
            "test signal must land inside the window, got {}",
            m_dry.depth()
        );
        assert!(
            (m_dry.depth() - m_hot.depth()).abs() < 1e-6,
            "depth must follow the DRY input only — a saturating chain must not \
             raise it (passthrough {}, saturating {})",
            m_dry.depth(),
            m_hot.depth()
        );
    }

    /// Switching the feature off must put every routed parameter back to its
    /// authored value, not leave it frozen mid-sweep.
    #[test]
    fn disabling_restores_the_authored_base() {
        use crate::dsp::ReactiveModulator;
        let mut chain = distortion_chain(10.0);
        let mut m = ReactiveModulator::new();
        m.configure(&rage_config(true, 10.0), &mut chain);

        // Drive it loud so the parameter is well away from its base.
        let mut tone = Tone::new();
        for _ in 0..80 {
            let buf = tone.block(0.4, 480);
            m.apply(&buf, &mut chain, 48_000);
        }
        assert!(m.depth() > 0.5, "should be modulating, got {}", m.depth());

        // Disable, then run one more block so the restore fires.
        m.configure(&rage_config(false, 10.0), &mut chain);
        let buf = tone.block(0.4, 480);
        m.apply(&buf, &mut chain, 48_000);

        // Assert the PARAMETER came back, not merely that depth reads zero.
        // `configure` zeroes depth by itself, so a depth-only assertion stays
        // green even with `restore_bases` deleted entirely. `AudioEffect` has
        // no getter, so compare behaviour against a pristine chain.
        assert!(
            chains_agree(&mut chain, &mut distortion_chain(10.0)),
            "disabling must restore the authored drive"
        );
    }

    /// Push a ramp through both chains and report whether they behave
    /// identically — the only way to observe a parameter, since effects expose
    /// no getters.
    fn chains_agree(a: &mut EffectChain, b: &mut EffectChain) -> bool {
        let mut pa = vec![0.0_f32; 64];
        let mut pb = vec![0.0_f32; 64];
        for (i, (x, y)) in pa.iter_mut().zip(pb.iter_mut()).enumerate() {
            #[allow(clippy::cast_precision_loss)]
            let v = ((i as f32) / 32.0 - 1.0) * 0.5;
            *x = v;
            *y = v;
        }
        a.process(&mut pa, 48_000);
        b.process(&mut pb, 48_000);
        let diff: f32 = pa.iter().zip(&pb).map(|(x, y)| (x - y).abs()).sum();
        diff < 1e-4
    }

    /// Re-pointing a route at a different parameter must release the old one.
    /// `restore_bases` can only iterate the routes it currently holds, so the
    /// outgoing table has to be restored BEFORE it is replaced — otherwise the
    /// abandoned target keeps whatever the modulation last wrote, forever.
    #[test]
    fn repointing_a_route_releases_the_previous_target() {
        use crate::dsp::{EffectKind, ReactiveConfig, ReactiveModulator, ReactiveRouteSpec};
        let mut chain = distortion_chain(10.0);
        let mut m = ReactiveModulator::new();
        m.configure(&rage_config(true, 10.0), &mut chain);

        let mut tone = Tone::new();
        for _ in 0..80 {
            let buf = tone.block(0.4, 480);
            m.apply(&buf, &mut chain, 48_000);
        }
        assert!(m.depth() > 0.5, "should be modulating, got {}", m.depth());

        // Re-point at Reverb.mix; Distortion.drive must return to 10.
        let repointed = ReactiveConfig {
            enabled: true,
            intensity: 1.0,
            routes: vec![ReactiveRouteSpec {
                kind: EffectKind::Reverb,
                nth: 0,
                key: "mix".to_string(),
                base: 20.0,
                depth: 30.0,
            }],
            ..ReactiveConfig::default()
        }
        .resolve();
        m.configure(&repointed, &mut chain);

        assert!(
            chains_agree(&mut chain, &mut distortion_chain(10.0)),
            "re-pointing must release the old target"
        );
    }

    /// Disabling by clearing the route list must still restore. A UI turning
    /// the panel off may plausibly send `{enabled: false, routes: []}`, which
    /// leaves `restore_bases` nothing to iterate unless the outgoing table is
    /// flushed first.
    #[test]
    fn disabling_with_an_empty_route_list_still_restores() {
        use crate::dsp::{ReactiveConfig, ReactiveModulator};
        let mut chain = distortion_chain(10.0);
        let mut m = ReactiveModulator::new();
        m.configure(&rage_config(true, 10.0), &mut chain);

        let mut tone = Tone::new();
        for _ in 0..80 {
            let buf = tone.block(0.4, 480);
            m.apply(&buf, &mut chain, 48_000);
        }
        assert!(m.depth() > 0.5, "should be modulating, got {}", m.depth());

        let cleared = ReactiveConfig {
            enabled: false,
            routes: vec![],
            ..ReactiveConfig::default()
        }
        .resolve();
        m.configure(&cleared, &mut chain);

        assert!(
            chains_agree(&mut chain, &mut distortion_chain(10.0)),
            "clearing the routes must still restore the authored drive"
        );
    }

    /// A buggy or hostile client cannot poison the audio path with an absurd
    /// depth: `1e39` arrives from JSON as `f32::INFINITY`, and at rest
    /// `inf.mul_add(0.0, base)` is NaN — which would propagate to the device.
    #[test]
    fn an_absurd_depth_cannot_produce_nan() {
        use crate::dsp::{EffectKind, ReactiveConfig, ReactiveRouteSpec};
        let cfg = ReactiveConfig {
            enabled: true,
            routes: vec![ReactiveRouteSpec {
                kind: EffectKind::Distortion,
                nth: 0,
                key: "drive".to_string(),
                base: 10.0,
                // What a JSON `1e39` actually becomes after the f64 -> f32
                // narrowing serde performs (rustc rejects the literal itself).
                depth: f32::INFINITY,
            }],
            ..ReactiveConfig::default()
        }
        .resolve();
        assert_eq!(cfg.routes.len(), 1);
        for shaped in [0.0_f32, 0.5, 1.0] {
            let v = cfg.routes[0].value_at(shaped);
            assert!(v.is_finite(), "value_at({shaped}) must be finite, got {v}");
        }
    }

    /// A route naming a parameter the whitelist does not allow is dropped, so
    /// a stale or hostile config cannot reach a click-prone parameter.
    #[test]
    fn routes_outside_the_whitelist_are_dropped() {
        use crate::dsp::{EffectKind, ReactiveConfig, ReactiveRouteSpec};
        let cfg = ReactiveConfig {
            enabled: true,
            routes: vec![
                // Allowed.
                ReactiveRouteSpec {
                    kind: EffectKind::Distortion,
                    nth: 0,
                    key: "drive".to_string(),
                    base: 10.0,
                    depth: 20.0,
                },
                // Delay-time: moving it jumps the read head. Never routable.
                ReactiveRouteSpec {
                    kind: EffectKind::Echo,
                    nth: 0,
                    key: "time".to_string(),
                    base: 100.0,
                    depth: 50.0,
                },
                // Filter frequency: rebuilds stateful biquads. Never routable.
                ReactiveRouteSpec {
                    kind: EffectKind::Eq,
                    nth: 0,
                    key: "low".to_string(),
                    base: 0.0,
                    depth: 6.0,
                },
            ],
            ..ReactiveConfig::default()
        };
        let resolved = cfg.resolve_routes();
        assert_eq!(resolved.len(), 1, "only the whitelisted route survives");
        assert_eq!(resolved[0].key, "drive");
    }

    // -----------------------------------------------------------------
    // Voice reading
    // -----------------------------------------------------------------

    /// A tap pair wired to real rings, so the tests exercise the same code
    /// the audio callback runs rather than a stand-in for it.
    fn taps() -> (ReadingTaps, super::RingConsumer, super::RingConsumer) {
        let (dp, dc) = HeapRb::<f32>::new(RING_BUFFER_FRAMES).split();
        let (wp, wc) = HeapRb::<f32>::new(RING_BUFFER_FRAMES).split();
        (ReadingTaps { dry: dp, wet: wp }, dc, wc)
    }

    /// Run one callback's worth of tapping exactly as the output callback
    /// does: arm once, copy DRY, then copy WET after the chain would have run.
    fn tap_one_buffer(t: &mut ReadingTaps, enabled: bool, frames: usize) -> bool {
        let block = vec![0.5f32; frames];
        let armed = t.armed(enabled, frames);
        if armed {
            t.push_dry(&block);
            t.push_wet(&block);
        }
        armed
    }

    /// Off means off. The switch defaults off, and while it is off the audio
    /// callback copies NOTHING — no samples reach the worker, so no analysis
    /// of the user's voice can happen at all.
    #[test]
    fn a_disabled_panel_taps_nothing() {
        let (mut t, dry, wet) = taps();
        for _ in 0..8 {
            assert!(!tap_one_buffer(&mut t, false, 256), "tapped while disabled");
        }
        assert_eq!(dry.occupied_len(), 0, "dry samples reached the worker");
        assert_eq!(wet.occupied_len(), 0, "wet samples reached the worker");

        // And the same taps do carry audio once it is switched on, so the
        // assertion above is about the gate and not about broken plumbing.
        assert!(tap_one_buffer(&mut t, true, 256));
        assert_eq!(dry.occupied_len(), 256);
        assert_eq!(wet.occupied_len(), 256);
    }

    /// The two taps are pushed together or not at all. A buffer that landed
    /// on one ring and was truncated on the other would leave the worker
    /// comparing a speaker against a different moment of themselves for the
    /// rest of the session.
    #[test]
    fn the_taps_drop_a_buffer_together_or_not_at_all() {
        let (mut t, dry, wet) = taps();
        // Fill the rings to just under capacity, then ask for more than fits.
        let frames = RING_BUFFER_FRAMES - 64;
        assert!(tap_one_buffer(&mut t, true, frames));
        assert!(
            !tap_one_buffer(&mut t, true, 256),
            "tapped without room for both halves"
        );
        assert_eq!(dry.occupied_len(), wet.occupied_len());
        // A zero-length buffer is not a tap either (it would arm the WET copy
        // for a DRY copy that never happened).
        assert!(!t.armed(true, 0));
    }

    /// Stopped, muted and quiet-but-present all look the same at the meter.
    /// The engine hands the first two in as facts; only `Speaking` comes from
    /// the measurement, and the facts always win.
    #[test]
    fn the_state_comes_from_facts_not_from_a_quiet_buffer() {
        let facts = |running, muted, silence| ReadingFacts {
            running,
            muted,
            digital_silence: silence,
        };
        // Engine down beats everything, including a measurement that somehow
        // still claims speech.
        assert_eq!(
            facts(false, false, false).resolve(ReadingState::Speaking),
            ReadingState::Stopped
        );
        assert_eq!(
            facts(false, true, true).resolve(ReadingState::Quiet),
            ReadingState::Stopped
        );
        // Muted is told, and digital silence (what a device mute produces)
        // counts as the same fact.
        assert_eq!(
            facts(true, true, false).resolve(ReadingState::Speaking),
            ReadingState::Muted
        );
        assert_eq!(
            facts(true, false, true).resolve(ReadingState::Speaking),
            ReadingState::Muted
        );
        // Running, not muted, signal present: now the measurement decides.
        assert_eq!(
            facts(true, false, false).resolve(ReadingState::Speaking),
            ReadingState::Speaking
        );
        assert_eq!(
            facts(true, false, false).resolve(ReadingState::Quiet),
            ReadingState::Quiet
        );
        // An analyzer that tried to claim an engine fact is not believed.
        assert_eq!(
            facts(true, false, false).resolve(ReadingState::Stopped),
            ReadingState::Quiet
        );
        assert_eq!(
            facts(true, false, false).resolve(ReadingState::Muted),
            ReadingState::Quiet
        );
    }

    fn speaking_window() -> VoiceReading {
        VoiceReading {
            state: ReadingState::Speaking,
            dry: Metrics {
                energy_dbfs: -18.0,
                energy_range_db: 9.0,
                f0_hz: 120.0,
                f0_range_st: 7.0,
                voiced_ratio: 0.6,
                pace_ops: 4.0,
                brightness_hz: 2200.0,
            },
            wet: Metrics::silent(),
            descriptors: vec![Descriptor::Bright, Descriptor::Wide],
            calibrated: true,
            held: false,
            held_descriptors: Vec::new(),
            wet_bypassed: false,
            wet_settled: true,
        }
    }

    /// A pause holds the last speaking window instead of decaying toward
    /// "quiet, flat, narrow" — which would amount to a verdict on the speaker
    /// several times a minute — and the snapshot says it is being held.
    #[test]
    fn a_pause_freezes_the_last_speaking_window() {
        let held = (speaking_window(), Instant::now());

        let live = reading_snapshot(Some(&held), ReadingState::Speaking);
        assert!(!live.stale);
        assert_eq!(live.age_ms, 0);
        assert_eq!(live.reading.state, ReadingState::Speaking);

        for state in [
            ReadingState::Quiet,
            ReadingState::Muted,
            ReadingState::Stopped,
        ] {
            let frozen = reading_snapshot(Some(&held), state);
            assert!(frozen.stale, "{state:?} showed a held window as live");
            assert_eq!(frozen.reading.state, state, "{state:?} lost its state");
            // The numbers are the SAME ones, unchanged — not decayed toward a
            // quieter, flatter, narrower version of the speaker.
            assert_eq!(frozen.reading.dry, held.0.dry);
            assert_eq!(frozen.reading.descriptors, held.0.descriptors);
        }
    }

    /// With nothing measured yet there is nothing to hold, so the panel shows
    /// the state and no numbers at all rather than an empty-looking reading
    /// dressed up as live.
    #[test]
    fn nothing_measured_yet_is_not_a_stale_reading() {
        let snap = reading_snapshot(None, ReadingState::Quiet);
        assert!(!snap.stale);
        assert!(snap.reading.descriptors.is_empty());
        assert!(!snap.reading.calibrated);
    }

    /// A held window expires. A reading from four minutes ago is not a stale
    /// reading, it is a different conversation.
    #[test]
    fn a_held_window_expires_instead_of_ageing_forever() {
        let stale_at = Instant::now()
            .checked_sub(Duration::from_secs(READING_HOLD_SECS + 1))
            .expect("the clock is far enough past boot to step back 31 s");
        let old = (speaking_window(), stale_at);
        let snap = reading_snapshot(Some(&old), ReadingState::Quiet);
        assert!(!snap.stale, "an expired window was shown as merely stale");
        assert!(snap.reading.descriptors.is_empty());
        assert_eq!(snap.reading.state, ReadingState::Quiet);
    }

    /// Octave errors are what users actually see on a gaming headset behind a
    /// gate and a denoiser. A jump that lands near an exact octave is held
    /// back until a second window agrees with it; a real change of register
    /// costs one window, a tracker glitch never lands twice.
    #[test]
    fn an_unconfirmed_octave_jump_is_not_shown() {
        assert!(is_octave_jump(120.0, 240.0));
        assert!(is_octave_jump(240.0, 120.0));
        assert!(!is_octave_jump(120.0, 150.0));

        let mut guard = OctaveGuard::default();
        let mut first = Metrics {
            f0_hz: 120.0,
            f0_range_st: 6.0,
            ..Metrics::silent()
        };
        guard.apply(&mut first);
        assert!(
            (first.f0_hz - 120.0).abs() < 1e-3,
            "the first window stands"
        );

        // A doubled window is held back, pitch AND range: a mid-window flip
        // inflates the range too, so showing one without the other would still
        // flicker.
        let mut flip = Metrics {
            f0_hz: 241.0,
            f0_range_st: 19.0,
            ..Metrics::silent()
        };
        guard.apply(&mut flip);
        assert!(
            (flip.f0_hz - 120.0).abs() < 1e-3,
            "an octave flip was shown"
        );
        assert!((flip.f0_range_st - 6.0).abs() < 1e-3);

        // A second window agreeing with the jump makes it real.
        let mut confirm = Metrics {
            f0_hz: 239.0,
            f0_range_st: 8.0,
            ..Metrics::silent()
        };
        guard.apply(&mut confirm);
        assert!(
            (confirm.f0_hz - 239.0).abs() < 1e-3,
            "a real jump never landed"
        );

        // An ordinary move is never delayed.
        let mut ordinary = Metrics {
            f0_hz: 250.0,
            f0_range_st: 5.0,
            ..Metrics::silent()
        };
        guard.apply(&mut ordinary);
        assert!((ordinary.f0_hz - 250.0).abs() < 1e-3);
    }

    /// An unvoiced window does not vouch for a pending jump: a pause between
    /// two glitched windows must not be read as confirmation.
    #[test]
    fn a_pause_does_not_confirm_an_octave_jump() {
        let mut guard = OctaveGuard::default();
        let mut base = Metrics {
            f0_hz: 110.0,
            f0_range_st: 5.0,
            ..Metrics::silent()
        };
        guard.apply(&mut base);
        let mut flip = Metrics {
            f0_hz: 220.0,
            f0_range_st: 15.0,
            ..Metrics::silent()
        };
        guard.apply(&mut flip);
        assert!((flip.f0_hz - 110.0).abs() < 1e-3);

        // Nothing voiced (f0 == 0 per the Metrics contract).
        let mut pause = Metrics::silent();
        guard.apply(&mut pause);

        let mut again = Metrics {
            f0_hz: 220.0,
            f0_range_st: 15.0,
            ..Metrics::silent()
        };
        guard.apply(&mut again);
        assert!(
            (again.f0_hz - 110.0).abs() < 1e-3,
            "a pause confirmed an unconfirmed jump"
        );
    }

    /// A device that stalls without clearing the engine's `running` flag
    /// produces no further windows. The last `Speaking` verdict must not
    /// outlive the audio that justified it.
    #[test]
    fn a_live_verdict_does_not_outlive_the_audio() {
        assert!(!live_verdict_expired(
            ReadingState::Speaking,
            Duration::from_millis(500)
        ));
        assert!(live_verdict_expired(
            ReadingState::Speaking,
            Duration::from_secs(5)
        ));
        // Only a live verdict expires; the quiet states are already honest.
        for state in [
            ReadingState::Quiet,
            ReadingState::Muted,
            ReadingState::Stopped,
        ] {
            assert!(!live_verdict_expired(state, Duration::from_secs(60)));
        }
    }

    /// A fresh engine reads nothing and the feature is off. This is the
    /// default a user who never opens the panel lives with.
    #[test]
    fn a_fresh_engine_has_the_reading_switched_off() {
        let engine = AudioEngine::new();
        assert!(!engine.reading_enabled());
        let snap = engine.voice_reading();
        assert_eq!(snap.reading.state, ReadingState::Stopped);
        assert!(snap.reading.descriptors.is_empty());
        assert!(!snap.reading.calibrated);
        assert!(!snap.stale);
        engine.set_reading_enabled(true);
        assert!(engine.reading_enabled());
        engine.set_reading_enabled(false);
        assert!(!engine.reading_enabled());
        // The muted fact is an input, never inferred; it defaults off.
        engine.set_reading_muted(true);
        drop(engine);
    }

    /// The reading crosses the Tauri bridge, so its wrapper keys are a
    /// contract. See `docs/STABLE-SURFACE.md`.
    #[test]
    fn reading_snapshot_json_keys_are_frozen() {
        let snap = ReadingSnapshot::idle(ReadingState::Quiet);
        let v = serde_json::to_value(&snap).unwrap();
        let mut keys: Vec<String> = v.as_object().unwrap().keys().cloned().collect();
        keys.sort();
        assert_eq!(keys, ["ageMs", "reading", "stale"]);
        // The state serializes as the lowercase word the UI switches on.
        assert_eq!(v["reading"]["state"], "quiet");
    }

    // ---- the chain swap: built off-thread, freed off-thread ----

    /// The graveyard frees what the callback hands it — and then stops, when
    /// the session does.
    #[test]
    fn the_graveyard_frees_a_displaced_chain_and_then_exits() {
        let (mut handover, bin) = HeapRb::<Displaced>::new(4).split();
        let (alive_tx, alive_rx) = channel::<()>();
        let (chain, freed) = witnessed_chain();
        assert!(handover.try_push(Displaced::Chain(chain)).is_ok());

        let worker = std::thread::spawn(move || graveyard_thread(bin, alive_rx));
        let start = Instant::now();
        while !freed.load(Ordering::SeqCst) && start.elapsed() < Duration::from_secs(5) {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(
            freed.load(Ordering::SeqCst),
            "the graveyard never freed the chain it was handed"
        );

        // Teardown: dropping the keep-alive is the only shutdown signal, and
        // it must be enough — a leaked thread per session is a leak per
        // device-loss rebuild.
        drop(alive_tx);
        worker.join().expect("the graveyard thread should exit");
    }

    /// A full graveyard must not block the callback. The overflow is freed
    /// inline instead — a bounded, rare hitch rather than a leak.
    #[test]
    fn a_full_graveyard_falls_back_to_freeing_inline() {
        // Room for exactly one, and nothing draining it.
        let (mut handover, _bin) = HeapRb::<Displaced>::new(1).split();
        let (tx, rx) = sync_channel::<DspEdit>(4);
        let reading = ReadingTap::default();

        let (live, first_freed) = witnessed_chain();
        let (second, second_freed) = witnessed_chain();
        let (third, third_freed) = witnessed_chain();
        let mut chain = live;
        tx.send(DspEdit::ReplaceChain(second)).expect("queued");
        tx.send(DspEdit::ReplaceChain(third)).expect("queued");

        // Two replacements, one slot: this returns rather than blocking.
        drain_dsp_edits(&rx, &mut chain, &mut handover, &reading);

        assert_eq!(handover.occupied_len(), 1, "the one slot should be taken");
        assert!(
            !first_freed.load(Ordering::SeqCst),
            "the chain that reached the graveyard must not be freed by the callback"
        );
        assert!(
            second_freed.load(Ordering::SeqCst),
            "the overflow chain should have been freed inline, not leaked"
        );
        assert!(
            !third_freed.load(Ordering::SeqCst),
            "the live chain must be left alone"
        );
    }

    /// v1.51.0 wired the voice reading to the command drain: the wet tap
    /// describes a chain that no longer exists once one is swapped in.
    #[test]
    fn a_chain_replacement_still_tells_the_reading() {
        let (mut handover, _bin) = HeapRb::<Displaced>::new(4).split();
        let (tx, rx) = sync_channel::<DspEdit>(4);
        let reading = ReadingTap::default();
        let mut chain = EffectChain::new();
        assert!(
            !reading.take_chain_changed(),
            "nothing has changed the chain yet"
        );

        tx.send(DspEdit::ReplaceChain(distortion_chain(20.0)))
            .expect("queued");
        drain_dsp_edits(&rx, &mut chain, &mut handover, &reading);

        assert_eq!(chain.len(), 1, "the prebuilt chain should be live");
        assert!(
            reading.take_chain_changed(),
            "the reading was never told the chain moved under it"
        );
    }

    /// Routes live BESIDE the chain and address effects by kind + occurrence,
    /// so a replacement has to keep working — including when the effect they
    /// name has moved to a different index in the new chain.
    #[test]
    fn reactive_routes_resolve_onto_a_swapped_in_chain() {
        use crate::dsp::{EffectKind, EffectSpec};
        use std::collections::HashMap;

        // Routed at base drive 0, so anything the modulation writes shows up.
        let mut chain = distortion_chain(0.0);
        let mut modulator = ReactiveModulator::new();
        modulator.configure(&rage_config(true, 0.0), &mut chain);

        // The replacement puts a (disabled, so inert) gate in front: the
        // distortion the route names is now at index 1.
        let built = || {
            let mut params = HashMap::new();
            params.insert("drive".to_string(), 0.0);
            EffectChain::from_specs(&[
                EffectSpec {
                    kind: EffectKind::Gate,
                    enabled: false,
                    params: HashMap::new(),
                },
                EffectSpec {
                    kind: EffectKind::Distortion,
                    enabled: true,
                    params,
                },
            ])
        };
        let displaced = chain.apply(DspEdit::ReplaceChain(built()));
        assert!(displaced.is_some(), "the old chain must come back");
        assert_eq!(chain.index_of_kind(EffectKind::Distortion, 0), Some(1));

        // Drive it loud enough to modulate, writing into the NEW chain.
        let mut tone = Tone::new();
        for _ in 0..60 {
            let buf = tone.block(0.3, 480);
            modulator.apply(&buf, &mut chain, 48_000);
        }
        assert!(
            modulator.depth() > 0.1,
            "test signal must actually modulate, got {}",
            modulator.depth()
        );

        // Same input through the modulated chain and an untouched copy of it.
        let mut modulated = tone.block(0.3, 480);
        let mut untouched = modulated.clone();
        chain.process(&mut modulated, 48_000);
        built().process(&mut untouched, 48_000);
        let delta = modulated
            .iter()
            .zip(&untouched)
            .fold(0.0_f32, |m, (a, b)| m.max((a - b).abs()));
        assert!(
            delta > 1e-3,
            "the route never reached the swapped-in chain's distortion (delta {delta})"
        );
    }

    /// Routes address targets by kind + occurrence, so reordering the chain
    /// re-resolves rather than pointing at whatever now sits at an old index.
    #[test]
    fn routes_follow_the_effect_across_a_reorder() {
        use crate::dsp::{EffectKind, EffectSpec};
        let spec = |kind: EffectKind| EffectSpec {
            kind,
            enabled: true,
            params: std::collections::HashMap::new(),
        };
        let a = EffectChain::from_specs(&[spec(EffectKind::Reverb), spec(EffectKind::Distortion)]);
        let b = EffectChain::from_specs(&[spec(EffectKind::Distortion), spec(EffectKind::Reverb)]);
        assert_eq!(a.index_of_kind(EffectKind::Distortion, 0), Some(1));
        assert_eq!(b.index_of_kind(EffectKind::Distortion, 0), Some(0));
        // A kind that isn't present resolves to nothing rather than index 0.
        assert_eq!(a.index_of_kind(EffectKind::Echo, 0), None);
    }

    /// Feed the seam real speech-like audio and require a real measurement.
    ///
    /// This is the test that would have caught shipping the placeholder: it
    /// returned `Quiet` with silent metrics for every input, so the panel
    /// would have rendered "still listening" forever while every other test
    /// passed. Anything that measures nothing fails here.
    #[test]
    fn the_wired_analyzer_actually_measures_the_signal() {
        const RATE: u32 = 48_000;
        const F0: f64 = 150.0;
        let mut analyzer = new_reading_analyzer();
        let mut reading = None;
        let mut phase = 0.0_f64;
        // A buzz, not a sine: harmonics are what a pitch tracker keys on, and
        // a bare sine would let a weak implementation look good.
        for _ in 0..400 {
            let mut block = [0.0_f32; 512];
            for s in &mut block {
                phase = (phase + F0 / f64::from(RATE)) % 1.0;
                let buzz = (1..=6)
                    .map(|h| (std::f64::consts::TAU * phase * f64::from(h)).sin() / f64::from(h));
                #[allow(clippy::cast_possible_truncation)]
                {
                    *s = (buzz.sum::<f64>() * 0.2) as f32;
                }
            }
            if let Some(r) = analyzer.push(&block, &block, RATE) {
                reading = Some(r);
            }
        }
        let r = reading.expect("a window of speech-like audio produced no reading");
        assert_eq!(
            r.state,
            ReadingState::Speaking,
            "steady voiced buzz read as {:?}",
            r.state
        );
        assert!(
            (r.dry.f0_hz - 150.0).abs() < 15.0,
            "pitch read as {} Hz, expected about {F0}",
            r.dry.f0_hz
        );
        assert!(
            r.dry.voiced_ratio > 0.5,
            "voiced ratio {}",
            r.dry.voiced_ratio
        );
        assert!(r.dry.energy_dbfs.is_finite() && r.dry.energy_dbfs < 0.0);
    }

    #[test]
    fn a_long_pause_does_not_claim_the_baseline_was_never_learned() {
        // After READING_HOLD_SECS the held numbers are dropped, and the panel
        // renders "still listening" from `calibrated: false`. That is a claim
        // about what the app knows, and it is untrue — the baseline lives in
        // the analyzer until the session ends.
        let mut reading = speaking_window();
        reading.calibrated = true;
        let expired = Instant::now()
            .checked_sub(Duration::from_secs(READING_HOLD_SECS + 1))
            .expect("a clock this far from the epoch");
        let snap = reading_snapshot(Some(&(reading, expired)), ReadingState::Quiet);
        assert!(
            snap.reading.calibrated,
            "a long pause un-learned the baseline"
        );
        assert_eq!(snap.reading.state, ReadingState::Quiet);

        // A session that never heard speech still reports uncalibrated.
        let fresh = reading_snapshot(None, ReadingState::Quiet);
        assert!(!fresh.reading.calibrated);
    }

    #[test]
    fn a_chain_change_is_signalled_across_the_thread_boundary() {
        // The analyzer has always had note_chain_changed; nothing called it.
        // The audio thread applies the command and the reading runs on
        // another thread, so the flag is the only way it can learn. Review
        // measured the consequence: the after-effects half kept the previous
        // preset's numbers for 1.25 s while the card already named the new
        // preset, and wet_settled could never go false.
        let tap = ReadingTap::default();
        assert!(!tap.take_chain_changed(), "nothing has changed yet");

        tap.note_chain_changed();
        assert!(tap.take_chain_changed(), "the change did not cross");
        assert!(!tap.take_chain_changed(), "the flag was not cleared");

        // Several commands before the worker looks are still one change.
        tap.note_chain_changed();
        tap.note_chain_changed();
        assert!(tap.take_chain_changed());
        assert!(!tap.take_chain_changed());
    }

    #[test]
    fn discarding_a_stale_tap_keeps_the_two_halves_paired() {
        // The panel-toggle race. The old code drained each tap to empty in
        // turn, so a callback landing between the two drains left one
        // buffer's DRY half alive with its WET half eaten -- the taps then
        // paired WET sample k with DRY sample k-N for the rest of the
        // session, and nothing resynced them.
        use ringbuf::traits::{Consumer, Observer, Producer, Split};
        use ringbuf::HeapRb;

        let (mut dry_p, mut dry_c) = HeapRb::<f32>::new(RING_BUFFER_FRAMES).split();
        let (mut wet_p, mut wet_c) = HeapRb::<f32>::new(RING_BUFFER_FRAMES).split();
        let (mut dry_buf, mut wet_buf) = ([0.0_f32; 256], [0.0_f32; 256]);

        // Three paired buffers, then a DRY half whose WET half has not been
        // written yet -- exactly what the callback leaves mid-buffer.
        for tag in [1.0_f32, 2.0, 3.0] {
            dry_p.push_slice(&[tag; 64]);
            wet_p.push_slice(&[tag; 64]);
        }
        dry_p.push_slice(&[4.0; 64]);

        drop_paired(&mut dry_c, &mut wet_c, &mut dry_buf, &mut wet_buf);

        // The unpaired DRY half survives, because its partner is on the way.
        assert_eq!(wet_c.occupied_len(), 0, "wet should be empty");
        assert_eq!(
            dry_c.occupied_len(),
            64,
            "the unpaired dry half must remain"
        );
        // And it is the right one: the next wet buffer belongs with tag 4.
        let n = dry_c.pop_slice(&mut dry_buf);
        assert_eq!(n, 64);
        assert!(
            dry_buf[..n].iter().all(|&s| (s - 4.0).abs() < f32::EPSILON),
            "kept the wrong buffer: {:?}",
            &dry_buf[..4]
        );
    }

    #[test]
    #[allow(clippy::cast_possible_truncation)]
    fn a_bypassed_chain_is_detected_by_measurement_not_plumbing() {
        // Nothing in the engine holds a "bypassed" flag, so this is measured.
        // Identical buffers are a passthrough whatever caused it; a gained
        // copy still is (the wet tap sits after the loudness stage); a
        // different waveform is not; silence votes neither way.
        let dry: Vec<f32> = (0..512)
            .map(|i| ((f64::from(i) * 0.05).sin() * 0.3) as f32)
            .collect();
        let gained: Vec<f32> = dry.iter().map(|s| s * 0.4).collect();
        let shifted: Vec<f32> = (0..512)
            .map(|i| ((f64::from(i) * 0.09).sin() * 0.3) as f32)
            .collect();
        assert_eq!(is_passthrough(&dry, &dry), Some(true));
        assert_eq!(
            is_passthrough(&dry, &gained),
            Some(true),
            "level alone is not processing"
        );
        assert_eq!(is_passthrough(&dry, &shifted), Some(false));
        assert_eq!(
            is_passthrough(&[0.0; 512], &[0.0; 512]),
            None,
            "silence must abstain"
        );
    }
}
