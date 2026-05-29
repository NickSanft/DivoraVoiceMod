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

use std::sync::atomic::Ordering;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;
use std::thread::JoinHandle;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{Device, SampleFormat, Stream, StreamConfig};
use ringbuf::traits::{Consumer, Producer, Split};
use ringbuf::HeapRb;
use serde::{Deserialize, Serialize};

use super::level::LevelMeter;
use super::state::{EngineState, Levels};
use super::AudioEngineError;
use crate::dsp::{DspCommand, EffectChain};

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
    pub sample_rate: u32,
    pub input_channels: u16,
    pub output_channels: u16,
}

enum Command {
    Start {
        input_name: Option<String>,
        output_name: Option<String>,
        reply: Sender<Result<StreamInfo, AudioEngineError>>,
    },
    Stop,
    SetMonitor(bool),
    Dsp(DspCommand),
    Shutdown,
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
    ) -> Result<StreamInfo, AudioEngineError> {
        let (reply_tx, reply_rx) = channel();
        self.tx
            .send(Command::Start {
                input_name: input_name.map(str::to_owned),
                output_name: output_name.map(str::to_owned),
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

    /// Send a DSP command (chain build, parameter sweep, etc.). The
    /// audio thread drains these at the top of each output buffer.
    pub fn send_dsp(&self, cmd: DspCommand) {
        let _ = self.tx.send(Command::Dsp(cmd));
    }

    #[must_use]
    pub fn is_running(&self) -> bool {
        self.state.running.load(Ordering::Acquire)
    }

    #[must_use]
    pub fn is_monitoring(&self) -> bool {
        self.state.monitor.load(Ordering::Acquire)
    }

    #[must_use]
    pub fn input_levels(&self) -> Levels {
        self.state.load_input()
    }

    #[must_use]
    pub fn output_levels(&self) -> Levels {
        self.state.load_output()
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
}

#[allow(clippy::needless_pass_by_value)] // owns values for the thread's lifetime
fn engine_thread(rx: Receiver<Command>, state: Arc<EngineState>) {
    let mut current: Option<RunningStreams> = None;
    let mut dsp_tx: Option<Sender<DspCommand>> = None;
    while let Ok(cmd) = rx.recv() {
        match cmd {
            Command::Start {
                input_name,
                output_name,
                reply,
            } => {
                // Drop any existing streams before building new ones.
                drop(current.take());
                drop(dsp_tx.take());
                state.running.store(false, Ordering::Release);

                let result =
                    start_streams(input_name.as_deref(), output_name.as_deref(), state.clone());
                current = match result {
                    Ok((streams, info, new_dsp_tx)) => {
                        state.running.store(true, Ordering::Release);
                        dsp_tx = Some(new_dsp_tx);
                        let _ = reply.send(Ok(info));
                        Some(streams)
                    }
                    Err(e) => {
                        let _ = reply.send(Err(e));
                        None
                    }
                };
            }
            Command::Stop => {
                drop(current.take());
                drop(dsp_tx.take());
                state.running.store(false, Ordering::Release);
                state.store_input(Levels::default());
                state.store_output(Levels::default());
            }
            Command::SetMonitor(enabled) => {
                state.monitor.store(enabled, Ordering::Release);
            }
            Command::Dsp(dsp_cmd) => {
                if let Some(tx) = &dsp_tx {
                    let _ = tx.send(dsp_cmd);
                }
            }
            Command::Shutdown => {
                drop(current.take());
                drop(dsp_tx.take());
                state.running.store(false, Ordering::Release);
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

#[allow(clippy::needless_pass_by_value)] // state is cloned into both stream closures
fn start_streams(
    input_name: Option<&str>,
    output_name: Option<&str>,
    state: Arc<EngineState>,
) -> Result<(RunningStreams, StreamInfo, Sender<DspCommand>), AudioEngineError> {
    let input_device = find_device(Direction::Input, input_name)?;
    let output_device = find_device(Direction::Output, output_name)?;

    let input_name_str = input_device.name().unwrap_or_default();
    let output_name_str = output_device.name().unwrap_or_default();

    let input_default = input_device
        .default_input_config()
        .map_err(|e| AudioEngineError::DefaultConfig(e.to_string()))?;
    let output_default = output_device
        .default_output_config()
        .map_err(|e| AudioEngineError::DefaultConfig(e.to_string()))?;

    let input_rate = input_default.sample_rate().0;
    let output_rate = output_default.sample_rate().0;
    if input_rate != output_rate {
        return Err(AudioEngineError::SampleRateMismatch {
            input: input_rate,
            output: output_rate,
        });
    }

    let input_channels = input_default.channels();
    let output_channels = output_default.channels();
    let input_format = input_default.sample_format();
    let output_format = output_default.sample_format();

    let input_config: StreamConfig = input_default.into();
    let output_config: StreamConfig = output_default.into();

    let rb = HeapRb::<f32>::new(RING_BUFFER_FRAMES);
    let (producer, consumer) = rb.split();

    let (dsp_tx, dsp_rx) = channel::<DspCommand>();

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
        input_rate,
    )?;

    input_stream
        .play()
        .map_err(|e| AudioEngineError::StreamPlay(e.to_string()))?;
    output_stream
        .play()
        .map_err(|e| AudioEngineError::StreamPlay(e.to_string()))?;

    let info = StreamInfo {
        input_name: input_name_str,
        output_name: output_name_str,
        sample_rate: input_rate,
        input_channels,
        output_channels,
    };
    Ok((
        RunningStreams {
            _input: input_stream,
            _output: output_stream,
        },
        info,
        dsp_tx,
    ))
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
fn build_output_stream(
    device: &Device,
    config: &StreamConfig,
    sample_format: SampleFormat,
    channels: u16,
    mut consumer: RingConsumer,
    state: Arc<EngineState>,
    dsp_rx: Receiver<DspCommand>,
    sample_rate: u32,
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
    let channels = channels as usize;
    let state_for_callback = state.clone();

    let stream = device
        .build_output_stream(
            config,
            move |data: &mut [f32], _info| {
                if data.is_empty() || channels == 0 {
                    return;
                }
                // Drain any pending DSP commands before processing audio.
                while let Ok(cmd) = dsp_rx.try_recv() {
                    chain.apply(cmd);
                }
                let monitoring = state_for_callback.monitor.load(Ordering::Acquire);
                let frames = data.len() / channels;
                let mut mono = [0f32; MAX_FRAMES_PER_CALLBACK];
                let frames = frames.min(mono.len());
                let popped = consumer.pop_slice(&mut mono[..frames]);
                let zero_from = popped;
                for slot in &mut mono[zero_from..frames] {
                    *slot = 0.0;
                }
                // Run the DSP chain over the mono buffer.
                chain.process(&mut mono[..frames], sample_rate);
                // Fan out mono -> all output channels, or silence if not
                // monitoring.
                for (i, frame) in data.chunks_exact_mut(channels).enumerate() {
                    if i >= frames {
                        for s in frame {
                            *s = 0.0;
                        }
                        continue;
                    }
                    let sample = if monitoring { mono[i] } else { 0.0 };
                    for s in frame {
                        *s = sample;
                    }
                }
                // Meter reflects what we actually sent to the device.
                let metered = &mono[..frames];
                if monitoring {
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

#[cfg(test)]
mod tests {
    use super::AudioEngine;

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
}
