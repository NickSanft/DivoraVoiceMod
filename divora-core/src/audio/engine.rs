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
    Soundboard(SoundboardCommand),
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

    /// Send a soundboard command (play / stop / stop-all). Forwarded
    /// through the engine thread to the live output callback.
    pub fn send_soundboard(&self, cmd: SoundboardCommand) {
        let _ = self.tx.send(Command::Soundboard(cmd));
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
    let mut sb_tx: Option<Sender<SoundboardCommand>> = None;
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
                drop(sb_tx.take());
                state.running.store(false, Ordering::Release);

                let result =
                    start_streams(input_name.as_deref(), output_name.as_deref(), state.clone());
                current = match result {
                    Ok((streams, info, new_dsp_tx, new_sb_tx)) => {
                        state.running.store(true, Ordering::Release);
                        dsp_tx = Some(new_dsp_tx);
                        sb_tx = Some(new_sb_tx);
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
                drop(sb_tx.take());
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
            Command::Soundboard(sb_cmd) => {
                if let Some(tx) = &sb_tx {
                    let _ = tx.send(sb_cmd);
                }
            }
            Command::Shutdown => {
                drop(current.take());
                drop(dsp_tx.take());
                drop(sb_tx.take());
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
#[allow(clippy::type_complexity)] // four-tuple is clearer than a struct here
fn start_streams(
    input_name: Option<&str>,
    output_name: Option<&str>,
    state: Arc<EngineState>,
) -> Result<
    (
        RunningStreams,
        StreamInfo,
        Sender<DspCommand>,
        Sender<SoundboardCommand>,
    ),
    AudioEngineError,
> {
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
        sb_tx,
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
    sb_rx: Receiver<SoundboardCommand>,
    input_rate: u32,
    output_rate: u32,
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

                // Run the DSP chain over the mic mono buffer first, so
                // effects apply only to the user's voice…
                // …then mix soundboard voices in alongside the
                // already-effected voice. Clips play "as-is" (no DSP).
                // Both write into the same `mono` buffer that the
                // resampler / fan-out consume below, so the soundboard
                // mix lands on whatever output device is selected —
                // including CABLE Input, which is what makes the clips
                // audible to call participants.
                mix_voice_and_soundboard(
                    &mut mono[..native_frames],
                    &mut chain,
                    &mut soundboard,
                    input_rate,
                );

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

                // Fan out mono -> all output channels, or silence if not
                // monitoring.
                for (i, frame) in data.chunks_exact_mut(channels).enumerate() {
                    if i >= written_out {
                        for s in frame {
                            *s = 0.0;
                        }
                        continue;
                    }
                    let sample = if monitoring { output_mono[i] } else { 0.0 };
                    for s in frame {
                        *s = sample;
                    }
                }
                // Meter reflects what we actually sent to the device.
                let metered = &output_mono[..written_out];
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

/// Run the DSP chain on the mic mono buffer, then mix soundboard
/// voices in over the top of the already-effected signal. Extracted
/// from the output callback so the order — "DSP first, then
/// soundboard on top" — is unit-testable in isolation. (The order
/// matters: it means effects apply only to the user's voice, and
/// clips play as-is on whatever output device is selected, including
/// the virtual mic.)
fn mix_voice_and_soundboard(
    mono: &mut [f32],
    chain: &mut EffectChain,
    soundboard: &mut SoundboardMixer,
    sample_rate: u32,
) {
    chain.process(mono, sample_rate);
    soundboard.mix_into(mono, sample_rate);
}

#[cfg(test)]
mod tests {
    use super::{mix_voice_and_soundboard, AudioEngine};
    use crate::dsp::EffectChain;
    use crate::soundboard::{SoundboardCommand, SoundboardMixer};
    use std::sync::Arc;

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
        });
        // Pretend the mic delivered a 480-sample buffer of constant 0.10.
        let mut mono = vec![0.10_f32; 480];
        mix_voice_and_soundboard(&mut mono, &mut chain, &mut sb, 48_000);
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
        mix_voice_and_soundboard(&mut mono, &mut chain, &mut sb, 48_000);
        for &s in &mono {
            assert!(
                (s - 0.10).abs() < 1e-6,
                "mic-only path mutated the sample, got {s}"
            );
        }
    }
}
