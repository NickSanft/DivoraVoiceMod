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
use std::sync::atomic::Ordering;
use std::sync::mpsc::{channel, Receiver, Sender, TryRecvError};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{Device, SampleFormat, Stream, StreamConfig};
use ringbuf::traits::{Consumer, Producer, Split};
use ringbuf::HeapRb;
use serde::{Deserialize, Serialize};

use super::level::LevelMeter;
use super::loudness::{LoudnessNormalizer, DEFAULT_TARGET_DBFS};
use super::resampler::MonoResampler;
use super::state::{EngineState, Levels};
use super::AudioEngineError;
use crate::dsp::{DspCommand, EffectChain};
use crate::soundboard::{SoundboardCommand, SoundboardMixer};

/// Capacity of the SPSC ring buffer used between input and output
/// callbacks. ~170 ms at 48 kHz; ample headroom for OS scheduling jitter.
const RING_BUFFER_FRAMES: usize = 8192;

/// Max samples processed in a single callback. Realistic cpal buffers
/// stay well under this; the array sits on the stack so no allocation
/// happens in the audio thread.
const MAX_FRAMES_PER_CALLBACK: usize = 4096;

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
    Dsp(DspCommand),
    Soundboard(SoundboardCommand),
    StartRecording {
        path: PathBuf,
    },
    StopRecording,
    Shutdown,
}

/// Commands to the recording writer thread (drains the recording ring).
enum RecordingCommand {
    Start { path: PathBuf },
    Stop,
}

/// Public handle to the audio engine. Construct once at app startup;
/// drop to shut down the audio thread cleanly.
pub struct AudioEngine {
    tx: Sender<Command>,
    state: Arc<EngineState>,
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
        // Monitor gain defaults to unity (Default would leave it 0.0).
        state.store_monitor_gain(1.0);
        // v1.7.0: loudness normalization is opt-in (enabled defaults false);
        // seed the target so the readout/stage have a sane value if enabled.
        state.store_loudness_target(DEFAULT_TARGET_DBFS);
        let state_clone = state.clone();
        let handle = std::thread::Builder::new()
            .name("divora-audio".into())
            .spawn(move || engine_thread(rx, state_clone))
            .expect("spawning the audio thread should not fail");
        Self {
            tx,
            state,
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

    /// v1.6.0: set the gain applied to the separate monitor ("hear
    /// yourself") stream. 1.0 = unity. Clamped to a safe range. Stored in
    /// shared state and picked up by the monitor callback next buffer; no
    /// engine restart needed.
    pub fn set_monitor_gain(&self, gain: f32) {
        self.state.store_monitor_gain(gain.clamp(0.0, 4.0));
    }

    /// Send a DSP command (chain build, parameter sweep, etc.). The
    /// audio thread drains these at the top of each output buffer.
    pub fn send_dsp(&self, cmd: DspCommand) {
        let _ = self.tx.send(Command::Dsp(cmd));
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

    #[must_use]
    pub fn is_running(&self) -> bool {
        self.state.running.load(Ordering::Acquire)
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
    dsp_tx: Sender<DspCommand>,
    sb_tx: Sender<SoundboardCommand>,
    recording_tx: Sender<RecordingCommand>,
    writer: JoinHandle<()>,
}

#[allow(clippy::needless_pass_by_value)] // owns values for the thread's lifetime
fn engine_thread(rx: Receiver<Command>, state: Arc<EngineState>) {
    let mut current: Option<RunningStreams> = None;
    let mut dsp_tx: Option<Sender<DspCommand>> = None;
    let mut sb_tx: Option<Sender<SoundboardCommand>> = None;
    let mut recording_tx: Option<Sender<RecordingCommand>> = None;
    let mut writer: Option<JoinHandle<()>> = None;

    // Tear down the current session's streams + recording writer.
    // Dropping `recording_tx` disconnects the writer's channel, which
    // makes it finalize any open WAV and exit; we then join it.
    macro_rules! teardown {
        () => {{
            drop(current.take());
            drop(dsp_tx.take());
            drop(sb_tx.take());
            state.recording.store(false, Ordering::Release);
            drop(recording_tx.take());
            if let Some(h) = writer.take() {
                let _ = h.join();
            }
            state.running.store(false, Ordering::Release);
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
                let result = start_streams(
                    input_name.as_deref(),
                    output_name.as_deref(),
                    monitor_name.as_deref(),
                    state.clone(),
                );
                current = match result {
                    Ok(started) => {
                        state.running.store(true, Ordering::Release);
                        dsp_tx = Some(started.dsp_tx);
                        sb_tx = Some(started.sb_tx);
                        recording_tx = Some(started.recording_tx);
                        writer = Some(started.writer);
                        let _ = reply.send(Ok(started.info));
                        Some(started.streams)
                    }
                    Err(e) => {
                        let _ = reply.send(Err(e));
                        None
                    }
                };
            }
            Command::Stop => {
                teardown!();
                state.store_input(Levels::default());
                state.store_output(Levels::default());
                state.store_dsp_latency_ms(0.0);
            }
            Command::SetMonitor(enabled) => {
                state.monitor.store(enabled, Ordering::Release);
            }
            Command::Dsp(dsp_cmd) => {
                if let Some(tx) = &dsp_tx {
                    let _ = tx.send(dsp_cmd);
                }
            }
            Command::Soundboard(sb_cmd) => {
                if let Some(tx) = &sb_tx {
                    let _ = tx.send(sb_cmd);
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
                    let _ = tx.send(RecordingCommand::Stop);
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
fn start_streams(
    input_name: Option<&str>,
    output_name: Option<&str>,
    monitor_name: Option<&str>,
    state: Arc<EngineState>,
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

    let (dsp_tx, dsp_rx) = channel::<DspCommand>();
    let (sb_tx, sb_rx) = channel::<SoundboardCommand>();

    let input_stream = build_input_stream(
        &input_device,
        &input_config,
        input_format,
        input_channels,
        producer,
        state.clone(),
    )?;
    let output_stream = build_output_stream(
        &output_device,
        &output_config,
        output_format,
        output_channels,
        consumer,
        state.clone(),
        dsp_rx,
        sb_rx,
        input_rate,
        output_rate,
        monitor_producer,
        has_monitor,
        recording_producer,
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

    Ok(StartedStreams {
        streams: RunningStreams {
            _input: input_stream,
            _output: output_stream,
            _monitor: monitor_stream,
        },
        info,
        dsp_tx,
        sb_tx,
        recording_tx,
        writer,
    })
}

type RingProducer = <HeapRb<f32> as Split>::Prod;
type RingConsumer = <HeapRb<f32> as Split>::Cons;

fn build_input_stream(
    device: &Device,
    config: &StreamConfig,
    sample_format: SampleFormat,
    channels: u16,
    mut producer: RingProducer,
    state: Arc<EngineState>,
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
    dsp_rx: Receiver<DspCommand>,
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
        tracing::error!(?err, device = %err_label, "output stream error");
    };

    let mut meter = LevelMeter::new();
    let mut chain = EffectChain::new();
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
                while let Ok(cmd) = dsp_rx.try_recv() {
                    chain.apply(cmd);
                }
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
                mix_voice_and_soundboard(
                    &mut mono[..native_frames],
                    &mut chain,
                    &mut loudness,
                    &mut soundboard,
                    input_rate,
                );
                // Publish the makeup gain for the UI "it's working" readout.
                state_for_callback.store_loudness_gain_db(loudness.gain_db());

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
                if let Some(mp) = monitor_producer.as_mut() {
                    let mut mon = [0f32; MAX_FRAMES_PER_CALLBACK];
                    mon[..native_frames].copy_from_slice(&mono[..native_frames]);
                    soundboard.mix_monitor_into(&mut mon[..native_frames], input_rate);
                    let _ = mp.push_slice(&mon[..native_frames]);
                } else {
                    soundboard.mix_monitor_into(&mut mono[..native_frames], input_rate);
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
                let monitoring = state.monitor.load(Ordering::Acquire);
                // v1.6.0: user-set monitor volume ("hear yourself" level).
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
                    let sample = if i < written_out && monitoring {
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

/// Run the DSP chain on the mic mono buffer, normalize the voice's
/// loudness (v1.7.0), then mix soundboard voices in over the top of the
/// already-effected, level-matched signal. Extracted from the output
/// callback so the order — "DSP → normalize voice → soundboard on top" —
/// is unit-testable in isolation. (The order matters: effects + loudness
/// normalization apply only to the user's voice, while clips play as-is
/// on whatever output device is selected, including the virtual mic.)
fn mix_voice_and_soundboard(
    mono: &mut [f32],
    chain: &mut EffectChain,
    loudness: &mut LoudnessNormalizer,
    soundboard: &mut SoundboardMixer,
    sample_rate: u32,
) {
    chain.process(mono, sample_rate);
    loudness.process(mono, sample_rate);
    soundboard.mix_into(mono, sample_rate);
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
                Ok(RecordingCommand::Stop) => {
                    if let Some(mut w) = writer.take() {
                        drain_recording(&mut consumer, &mut buf, &mut w);
                        let _ = w.finalize();
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
        drain_recording, main_output_plays, mix_voice_and_soundboard, AudioEngine,
        LoudnessNormalizer, StreamInfo, MAX_FRAMES_PER_CALLBACK, RING_BUFFER_FRAMES,
    };
    use crate::dsp::EffectChain;
    use crate::soundboard::{SoundboardCommand, SoundboardMixer};
    use ringbuf::traits::{Producer, Split};
    use ringbuf::HeapRb;
    use std::sync::Arc;

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
        engine.stop();
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

    /// Phase 11 regression — confirm that the engine's output
    /// callback mixes the soundboard ON TOP of the DSP-processed mic
    /// buffer (rather than into a separate stream or only the monitor
    /// path). The function is small but the assertion is the property
    /// users care about: if the user routes the engine output into
    /// CABLE Input, the clips reach the call.
    #[test]
    fn soundboard_clips_land_in_the_same_output_buffer_as_the_mic() {
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
        // Loudness normalizer defaults to disabled → bit-exact passthrough,
        // so the mic + clip sum below is unaffected.
        let mut loudness = LoudnessNormalizer::new();
        mix_voice_and_soundboard(&mut mono, &mut chain, &mut loudness, &mut sb, 48_000);
        // Each output sample should now hold the sum: 0.10 (mic, passed
        // through the empty chain) + 0.25 (clip mixed in) = 0.35.
        // Linear-interpolation between same-value neighbours is exact.
        for (i, &s) in mono.iter().enumerate() {
            assert!(
                (s - 0.35).abs() < 1e-5,
                "expected mic + clip to sum at i={i}, got {s}"
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
        let mut loudness = LoudnessNormalizer::new();
        mix_voice_and_soundboard(&mut mono, &mut chain, &mut loudness, &mut sb, 48_000);
        for &s in &mono {
            assert!(
                (s - 0.10).abs() < 1e-6,
                "mic-only path mutated the sample, got {s}"
            );
        }
    }
}
