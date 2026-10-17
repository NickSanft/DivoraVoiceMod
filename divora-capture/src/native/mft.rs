//! The H.264 encoder, driven directly (v1.54.0).
//!
//! v1.52.0 let the sink writer load the encoder itself (rung 1 of the plan's
//! ladder). The sink writer never hands encoded samples back, so Replay — the
//! last moments kept in memory — needs the encoder driven by hand, its
//! output fanned out to a take's file and to the replay ring. This module is
//! that encoder, for both kinds Windows has:
//!
//! - a **hardware** encoder (NVIDIA's, AMD's, Intel's) is *asynchronous*: it
//!   says when it wants a frame (`METransformNeedInput`) and when it has
//!   output (`METransformHaveOutput`) through its event queue, which is read
//!   here without waiting, so the capture loop never blocks on it;
//! - Windows' own **software** encoder is *synchronous*: a frame in, then
//!   output until it asks for more. CI has only that one, so it is what the
//!   smoke test drives.
//!
//! What was measured on this machine (RTX 4090, "NVIDIA H.264 Encoder MFT",
//! and the inbox "H264 Encoder MFT") decides the order of every call below:
//! the keyframe interval is honoured only if set **before** the output type;
//! a forced keyframe is set just before the frame it applies to; each output
//! is one access unit, Annex B, with SPS and PPS inside every IDR.
//!
//! Every call returns a `Result`: release builds abort on a panic, and a
//! driver that misbehaves must not take the voice app with it.

use std::mem::ManuallyDrop;
use std::sync::Arc;
use std::time::{Duration, Instant};

use windows::core::Interface;
use windows::Win32::Media::MediaFoundation::{
    eAVEncH264VProfile_High, CODECAPI_AVEncCommonMeanBitRate, CODECAPI_AVEncCommonRateControlMode,
    CODECAPI_AVEncMPVDefaultBPictureCount, CODECAPI_AVEncMPVGOPSize,
    CODECAPI_AVEncVideoForceKeyFrame, CODECAPI_AVLowLatencyMode, ICodecAPI, IMF2DBuffer,
    IMFActivate, IMFAttributes, IMFMediaEventGenerator, IMFSample, IMFShutdown, IMFTransform,
    MEError, METransformDrainComplete, METransformHaveOutput, METransformNeedInput,
    MFMediaType_Video, MFSampleExtension_CleanPoint, MFSampleExtension_DecodeTimestamp,
    MFT_ENUM_HARDWARE_URL_Attribute, MFT_ENUM_HARDWARE_VENDOR_ID_Attribute,
    MFT_FRIENDLY_NAME_Attribute, MFT_GFX_DRIVER_VERSION_ID_Attribute, MFVideoFormat_H264,
    MFVideoFormat_NV12, MFT_CATEGORY_VIDEO_ENCODER, MFT_ENUM_FLAG_HARDWARE,
    MFT_ENUM_FLAG_SORTANDFILTER, MFT_ENUM_FLAG_SYNCMFT, MFT_MESSAGE_COMMAND_DRAIN,
    MFT_MESSAGE_NOTIFY_BEGIN_STREAMING, MFT_MESSAGE_NOTIFY_END_OF_STREAM,
    MFT_MESSAGE_NOTIFY_END_STREAMING, MFT_MESSAGE_NOTIFY_START_OF_STREAM,
    MFT_MESSAGE_SET_D3D_MANAGER, MFT_OUTPUT_DATA_BUFFER, MFT_OUTPUT_STREAM_CAN_PROVIDE_SAMPLES,
    MFT_OUTPUT_STREAM_INFO, MFT_OUTPUT_STREAM_PROVIDES_SAMPLES, MFT_REGISTER_TYPE_INFO,
    MF_EVENT_FLAG_NO_WAIT, MF_E_NOTACCEPTING, MF_E_NO_EVENTS_AVAILABLE,
    MF_E_TRANSFORM_NEED_MORE_INPUT, MF_E_TRANSFORM_STREAM_CHANGE, MF_MT_AVG_BITRATE,
    MF_MT_MPEG2_PROFILE, MF_MT_MPEG_SEQUENCE_HEADER, MF_SA_D3D11_AWARE, MF_TRANSFORM_ASYNC,
    MF_TRANSFORM_ASYNC_UNLOCK,
};

use super::d3d::Gpu;
use super::encoder::{string_attr, video_type};
use super::mf::{Mf, MfSession};
use super::pool::MfDevice;
use super::variant::{variant_as_u32, variant_bool, variant_u32};
use crate::error::{CaptureError, Context};
use crate::submit::Credits;

/// What to build.
#[derive(Debug, Clone)]
pub struct EncoderConfig {
    /// Even; the take's encoded size.
    pub size: (u32, u32),
    /// 30 or 60: a frame is then a whole number of 48 kHz samples.
    pub fps: u32,
    /// Bits per second.
    pub bitrate: u32,
    /// Frames between keyframes: 2 × fps. Never 0, which on NVIDIA means
    /// no keyframe after the first, ever.
    pub gop_frames: u32,
    /// `CODECAPI_AVLowLatencyMode`: no frames held for lookahead. The inbox
    /// encoder otherwise buffers 16 frames before its first output.
    pub low_latency: bool,
    pub engine: EncoderEngine,
}

/// Which kind of encoder to look for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EncoderEngine {
    /// The GPU's, preferring the one whose PCI vendor matches the device's.
    Hardware { vendor_id: Option<u32> },
    /// Windows' own synchronous encoder: the CI smoke test.
    Software,
}

/// What the encoder turned out to be. Logged once per session.
#[derive(Debug, Clone, Default)]
#[allow(clippy::struct_excessive_bools)] // a report of independent facts
pub struct EncoderInfo {
    pub name: Option<String>,
    pub hardware: bool,
    pub vendor: Option<String>,
    pub driver_version: Option<u64>,
    pub is_async: bool,
    pub d3d11_aware: bool,
    pub provides_samples: bool,
    /// The keyframe interval read back after configuring; `None` when the
    /// encoder wouldn't say (it is then watched instead).
    pub gop_readback: Option<u32>,
    /// HRESULTs of the settings that may be refused without harm.
    pub bframes_hr: Option<i32>,
    pub low_latency_hr: Option<i32>,
    pub rate_control_hr: Option<i32>,
    pub force_key_supported: bool,
}

/// Counted, never logged per event: one summary line at teardown.
#[derive(Debug, Clone, Default)]
pub struct EncoderCounters {
    /// `MF_E_NOTACCEPTING` despite a pending request (the count reset).
    pub not_accepting: u64,
    /// A forced keyframe the encoder refused to be told about.
    pub force_key_failed: u64,
    /// Events other than the four this module acts on.
    pub ignored_events: u64,
}

// What the session hands back is plain data, so the pipeline's decisions
// can be tested without Windows.
pub use crate::pipeline_core::{EncodedUnit, EncoderEvent};

/// Whether a frame went in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Submit {
    Accepted,
    /// The encoder isn't asking for a frame: keep it and try again (or
    /// replace it with a newer one).
    NotNow,
}

/// A frame for the encoder.
pub enum EncoderInput<'a> {
    /// A sample from the [`super::pool::InputPool`] (a GPU texture).
    Gpu(&'a IMFSample),
    /// NV12 bytes (the software encoder).
    Memory(&'a [u8]),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SessionState {
    Streaming,
    Draining,
    Drained,
    Failed,
}

/// Owns the activated encoder from the line after it is created, so every
/// early return and every drop runs the documented shutdown: `IMFShutdown`
/// (asynchronous encoders), then `IMFActivate::ShutdownObject`.
struct MftGuard {
    activate: IMFActivate,
    mft: IMFTransform,
    shutdown: Option<IMFShutdown>,
}

impl Drop for MftGuard {
    fn drop(&mut self) {
        // SAFETY: live objects; called once, every result ignored — this is
        // teardown, and nothing more can be done about a failure here.
        unsafe {
            if let Some(s) = &self.shutdown {
                let _ = s.Shutdown();
            }
            let _ = self.activate.ShutdownObject();
        }
    }
}

/// One H.264 encoder, configured and streaming.
pub struct EncoderSession {
    events: Option<IMFMediaEventGenerator>,
    codec: ICodecAPI,
    manager_set: bool,
    in_id: u32,
    out_id: u32,
    out_info: MFT_OUTPUT_STREAM_INFO,
    caller_allocates: bool,
    credits: Credits,
    state: SessionState,
    seq_header: Option<Arc<[u8]>>,
    info: EncoderInfo,
    counters: EncoderCounters,
    api: &'static Mf,
    // Last: dropped after `Drop::drop` has ended streaming.
    guard: MftGuard,
}

fn hr(r: &windows::core::Result<()>) -> i32 {
    match r {
        Ok(()) => 0,
        Err(e) => e.code().0,
    }
}

fn enc(why: impl Into<String>) -> CaptureError {
    CaptureError::Encoder(why.into())
}

impl EncoderSession {
    /// Find, configure and start an encoder: enumerate → activate → unlock
    /// (asynchronous) → device manager → settings → output type → input
    /// type → read back → start streaming. `dev` is required for hardware.
    pub fn new(
        mf: &MfSession,
        dev: Option<&MfDevice>,
        cfg: &EncoderConfig,
    ) -> Result<Self, CaptureError> {
        let api = mf.mf();
        if cfg.gop_frames == 0 || cfg.fps == 0 {
            return Err(enc("no keyframe interval"));
        }
        let (activate, mut info) = pick(api, cfg.engine)?;
        // SAFETY: a live activate from MFTEnumEx; it owns the object made.
        let mft: IMFTransform = unsafe { activate.ActivateObject() }.context("ActivateObject")?;
        let mut guard = MftGuard {
            activate,
            mft: mft.clone(),
            shutdown: None,
        };

        // SAFETY: plain getters/setters on the encoder's attribute store.
        let attrs = unsafe { mft.GetAttributes() }.context("IMFTransform::GetAttributes")?;
        // SAFETY: as above.
        info.is_async = unsafe { attrs.GetUINT32(&MF_TRANSFORM_ASYNC) }.unwrap_or(0) != 0;
        // SAFETY: as above.
        info.d3d11_aware = unsafe { attrs.GetUINT32(&MF_SA_D3D11_AWARE) }.unwrap_or(0) != 0;
        let hardware = matches!(cfg.engine, EncoderEngine::Hardware { .. });
        let events = if info.is_async {
            // An asynchronous encoder refuses every call until unlocked.
            // SAFETY: as above.
            unsafe { attrs.SetUINT32(&MF_TRANSFORM_ASYNC_UNLOCK, 1) }
                .context("MF_TRANSFORM_ASYNC_UNLOCK")?;
            guard.shutdown = mft.cast::<IMFShutdown>().ok();
            Some(
                mft.cast::<IMFMediaEventGenerator>()
                    .context("IMFMediaEventGenerator")?,
            )
        } else if hardware {
            return Err(enc("the hardware encoder isn't asynchronous"));
        } else {
            None
        };

        let (in_id, out_id) = stream_ids(&mft);

        let mut manager_set = false;
        if hardware {
            if !info.d3d11_aware {
                return Err(enc("the hardware encoder isn't Direct3D 11 aware"));
            }
            let Some(dev) = dev else {
                return Err(enc("no Direct3D device for the hardware encoder"));
            };
            // The encoder takes its own reference; passing ours as a raw
            // pointer leaks nothing.
            // SAFETY: a live manager, valid for the encoder's life (it holds
            // a reference until SET_D3D_MANAGER(0) in Drop).
            unsafe {
                mft.ProcessMessage(MFT_MESSAGE_SET_D3D_MANAGER, dev.manager.as_raw() as usize)
            }
            .context("MFT_MESSAGE_SET_D3D_MANAGER")?;
            manager_set = true;
        }

        let codec: ICodecAPI = mft.cast().map_err(|_| enc("no ICodecAPI"))?;
        configure(&codec, cfg, &mut info)?;

        let out = video_type(api, MFVideoFormat_H264, cfg.size, cfg.fps)?;
        // SAFETY: setters on a live media type; live encoder and type.
        unsafe {
            out.SetUINT32(&MF_MT_AVG_BITRATE, cfg.bitrate)
                .context("MF_MT_AVG_BITRATE")?;
            out.SetUINT32(&MF_MT_MPEG2_PROFILE, eAVEncH264VProfile_High.0 as u32)
                .context("MF_MT_MPEG2_PROFILE")?;
            mft.SetOutputType(out_id, &out, 0)
                .context("IMFTransform::SetOutputType")?;
        }
        let input = video_type(api, MFVideoFormat_NV12, cfg.size, cfg.fps)?;
        // SAFETY: live encoder and type.
        unsafe { mft.SetInputType(in_id, &input, 0) }.context("IMFTransform::SetInputType")?;
        let seq_header = sequence_header(&mft, out_id);

        let out_info = read_back(&codec, &mft, out_id, cfg, &mut info)?;
        let caller_allocates = !info.provides_samples;

        // SAFETY: a configured encoder; plain messages.
        unsafe {
            mft.ProcessMessage(MFT_MESSAGE_NOTIFY_BEGIN_STREAMING, 0)
                .context("MFT_MESSAGE_NOTIFY_BEGIN_STREAMING")?;
            mft.ProcessMessage(MFT_MESSAGE_NOTIFY_START_OF_STREAM, 0)
                .context("MFT_MESSAGE_NOTIFY_START_OF_STREAM")?;
        }

        Ok(Self {
            events,
            codec,
            manager_set,
            in_id,
            out_id,
            out_info,
            caller_allocates,
            credits: Credits::new(info.is_async),
            state: SessionState::Streaming,
            seq_header,
            info,
            counters: EncoderCounters::default(),
            api,
            guard,
        })
    }

    pub fn info(&self) -> &EncoderInfo {
        &self.info
    }

    pub fn counters(&self) -> &EncoderCounters {
        &self.counters
    }

    /// The SPS and PPS from the output type, Annex B; `None` if the encoder
    /// gave none (the first IDR's are used then).
    pub fn sequence_header(&self) -> Option<&Arc<[u8]>> {
        self.seq_header.as_ref()
    }

    /// Whether a frame may be submitted now.
    pub fn wants_input(&self) -> bool {
        self.state == SessionState::Streaming && self.credits.wants_input()
    }

    /// Collect whatever the encoder has: outputs, requests for input, a
    /// finished drain. Never waits.
    pub fn poll(&mut self, out: &mut Vec<EncoderEvent>) -> Result<(), CaptureError> {
        if self.state == SessionState::Failed {
            return Ok(());
        }
        let Some(events) = self.events.clone() else {
            // Synchronous: output until it wants more input.
            while self.process_output(out)? {}
            return Ok(());
        };
        loop {
            // SAFETY: a live event generator; NO_WAIT returns at once.
            let ev = match unsafe { events.GetEvent(MF_EVENT_FLAG_NO_WAIT) } {
                Ok(ev) => ev,
                Err(e) if e.code() == MF_E_NO_EVENTS_AVAILABLE => return Ok(()),
                Err(e) => {
                    self.state = SessionState::Failed;
                    return Err(CaptureError::Windows {
                        context: "IMFMediaEventGenerator::GetEvent",
                        source: e,
                    });
                }
            };
            // SAFETY: plain getters on a live event.
            let (kind, status) = unsafe { (ev.GetType(), ev.GetStatus()) };
            let kind = kind.context("IMFMediaEvent::GetType")?;
            let status = status.context("IMFMediaEvent::GetStatus")?;
            if status.is_err() {
                self.state = SessionState::Failed;
                return Err(enc(format!("the encoder reported {:#010x}", status.0)));
            }
            let kind = i32::try_from(kind).unwrap_or(-1);
            if kind == METransformNeedInput.0 {
                self.credits.on_need_input();
            } else if kind == METransformHaveOutput.0 {
                self.process_output(out)?;
            } else if kind == METransformDrainComplete.0 {
                self.state = SessionState::Drained;
                out.push(EncoderEvent::Drained);
            } else if kind == MEError.0 {
                self.state = SessionState::Failed;
                return Err(enc("the encoder reported an error"));
            } else {
                self.counters.ignored_events += 1;
            }
            if self.state == SessionState::Failed {
                return Ok(());
            }
        }
    }

    /// Give the encoder one frame, stamped `pts` for `dur` (100-ns ticks),
    /// forcing it to be a keyframe if `force_key`. Never waits.
    pub fn encode(
        &mut self,
        input: &EncoderInput<'_>,
        pts: i64,
        dur: i64,
        force_key: bool,
    ) -> Result<Submit, CaptureError> {
        if self.state != SessionState::Streaming {
            return Err(enc("the encoder isn't taking frames"));
        }
        if !self.credits.wants_input() {
            return Ok(Submit::NotNow);
        }
        let sample = match input {
            EncoderInput::Gpu(sample) => {
                // The pool's samples carry their texture's length already on
                // the measured driver; set it anyway, as Microsoft's samples do.
                // SAFETY: a live sample with one DXGI buffer.
                unsafe {
                    let buffer = sample.GetBufferByIndex(0).context("GetBufferByIndex")?;
                    if let Ok(two_d) = buffer.cast::<IMF2DBuffer>() {
                        let len = two_d.GetContiguousLength().context("GetContiguousLength")?;
                        buffer.SetCurrentLength(len).context("SetCurrentLength")?;
                    }
                }
                (*sample).clone()
            }
            EncoderInput::Memory(bytes) => {
                let buffer = memory_buffer(self.api, bytes)?;
                let sample = self.api.create_sample()?;
                // SAFETY: live sample and buffer.
                unsafe { sample.AddBuffer(&buffer) }.context("AddBuffer")?;
                sample
            }
        };
        // SAFETY: plain setters on a live sample.
        unsafe {
            sample.SetSampleTime(pts).context("SetSampleTime")?;
            sample
                .SetSampleDuration(dur.max(1))
                .context("SetSampleDuration")?;
        }
        if force_key {
            // SAFETY: a live interface, a static GUID, a scalar VARIANT.
            let forced = unsafe {
                self.codec
                    .SetValue(&CODECAPI_AVEncVideoForceKeyFrame, &variant_u32(1))
            };
            if forced.is_err() {
                // Counted, not fatal: the keyframe watchdog decides.
                self.counters.force_key_failed += 1;
            }
        }
        // SAFETY: a live encoder and sample on its input stream.
        match unsafe { self.guard.mft.ProcessInput(self.in_id, &sample, 0) } {
            Ok(()) => {
                self.credits.on_accepted();
                Ok(Submit::Accepted)
            }
            Err(e) if e.code() == MF_E_NOTACCEPTING => {
                self.credits.on_not_accepting();
                self.counters.not_accepting += 1;
                Ok(Submit::NotNow)
            }
            Err(e) => {
                self.state = SessionState::Failed;
                Err(CaptureError::Windows {
                    context: "IMFTransform::ProcessInput",
                    source: e,
                })
            }
        }
    }

    /// Ask for everything still inside the encoder, collecting it into `out`
    /// for at most `timeout`. A drained session takes no more frames.
    pub fn drain(
        &mut self,
        timeout: Duration,
        out: &mut Vec<EncoderEvent>,
    ) -> Result<(), CaptureError> {
        if self.state != SessionState::Streaming {
            return Ok(());
        }
        // SAFETY: plain messages to a streaming encoder.
        unsafe {
            self.guard
                .mft
                .ProcessMessage(MFT_MESSAGE_NOTIFY_END_OF_STREAM, 0)
                .context("MFT_MESSAGE_NOTIFY_END_OF_STREAM")?;
            self.guard
                .mft
                .ProcessMessage(MFT_MESSAGE_COMMAND_DRAIN, 0)
                .context("MFT_MESSAGE_COMMAND_DRAIN")?;
        }
        self.state = SessionState::Draining;
        if self.events.is_none() {
            while self.process_output(out)? {}
            self.state = SessionState::Drained;
            out.push(EncoderEvent::Drained);
            return Ok(());
        }
        let deadline = Instant::now() + timeout;
        while self.state == SessionState::Draining && Instant::now() < deadline {
            self.poll(out)?;
            if self.state == SessionState::Draining {
                std::thread::sleep(Duration::from_millis(1));
            }
        }
        Ok(())
    }

    /// Before a teardown whose output nobody wants: drain for at most
    /// `timeout` and discard it, so the encoder isn't released mid-frame —
    /// some drivers crash if it is. Skipped on a failed session.
    pub fn quiesce(&mut self, timeout: Duration) {
        let mut discard = Vec::new();
        if let Err(e) = self.drain(timeout, &mut discard) {
            tracing::info!(error = %e, "the encoder didn't drain before teardown");
        }
    }

    /// Whether the GPU is still there (a driver reset or an unplugged eGPU
    /// removes it). Polled about once a second.
    pub fn device_ok(dev: &Gpu) -> bool {
        // SAFETY: plain getter on a live device.
        unsafe { dev.device.GetDeviceRemovedReason() }.is_ok()
    }

    /// One `ProcessOutput`. `Ok(true)` when it produced something and a
    /// synchronous encoder may have more.
    fn process_output(&mut self, out: &mut Vec<EncoderEvent>) -> Result<bool, CaptureError> {
        let mine = if self.caller_allocates {
            let sample = self.api.create_sample()?;
            let buffer = self.api.create_memory_buffer(self.out_info.cbSize)?;
            // SAFETY: live sample and buffer.
            unsafe { sample.AddBuffer(&buffer) }.context("AddBuffer (output)")?;
            Some(sample)
        } else {
            None
        };
        let mut buf = MFT_OUTPUT_DATA_BUFFER {
            dwStreamID: self.out_id,
            pSample: ManuallyDrop::new(mine),
            dwStatus: 0,
            pEvents: ManuallyDrop::new(None),
        };
        let mut status = 0u32;
        // SAFETY: one output buffer for the encoder's one output stream.
        let result = unsafe {
            self.guard
                .mft
                .ProcessOutput(0, std::slice::from_mut(&mut buf), &raw mut status)
        };
        // The binding never drops these: take them on every path, or each
        // frame leaks a sample.
        // SAFETY: taken exactly once each, after the call.
        let (sample, _events) = unsafe {
            (
                ManuallyDrop::take(&mut buf.pSample),
                ManuallyDrop::take(&mut buf.pEvents),
            )
        };
        match result {
            Ok(()) => {
                let Some(sample) = sample else {
                    return Ok(false);
                };
                out.push(EncoderEvent::Unit(unit(&sample)?));
                Ok(true)
            }
            Err(e) if e.code() == MF_E_TRANSFORM_NEED_MORE_INPUT => Ok(false),
            Err(e) if e.code() == MF_E_TRANSFORM_STREAM_CHANGE => {
                self.state = SessionState::Failed;
                out.push(EncoderEvent::Restart);
                Ok(false)
            }
            Err(e) => {
                self.state = SessionState::Failed;
                Err(CaptureError::Windows {
                    context: "IMFTransform::ProcessOutput",
                    source: e,
                })
            }
        }
    }
}

impl Drop for EncoderSession {
    fn drop(&mut self) {
        // SAFETY: plain messages; every result ignored (teardown). The
        // guard field then shuts the encoder down.
        unsafe {
            let _ = self
                .guard
                .mft
                .ProcessMessage(MFT_MESSAGE_NOTIFY_END_STREAMING, 0);
            if self.manager_set {
                let _ = self
                    .guard
                    .mft
                    .ProcessMessage(MFT_MESSAGE_SET_D3D_MANAGER, 0);
            }
        }
    }
}

/// What the configured encoder says back: the keyframe interval (refused if
/// it ignored ours), whether it can force a keyframe (refused if not), and
/// its output stream — refused if it wants buffers from us of no stated size.
fn read_back(
    codec: &ICodecAPI,
    mft: &IMFTransform,
    out_id: u32,
    cfg: &EncoderConfig,
    info: &mut EncoderInfo,
) -> Result<MFT_OUTPUT_STREAM_INFO, CaptureError> {
    // SAFETY: a live interface and a static GUID; the VARIANT returned
    // is a scalar that owns nothing.
    match unsafe { codec.GetValue(&CODECAPI_AVEncMPVGOPSize) }
        .ok()
        .as_ref()
        .and_then(variant_as_u32)
    {
        Some(g) if g == cfg.gop_frames => info.gop_readback = Some(g),
        Some(g) => {
            return Err(enc(format!(
                "it ignores the keyframe interval ({g} instead of {})",
                cfg.gop_frames
            )))
        }
        None => tracing::info!("the encoder's keyframe interval can't be read back"),
    }
    // SAFETY: as above.
    info.force_key_supported =
        unsafe { codec.IsSupported(&CODECAPI_AVEncVideoForceKeyFrame) }.is_ok();
    if !info.force_key_supported {
        return Err(enc("it can't force a keyframe"));
    }

    // SAFETY: a live encoder with both types set.
    let out_info = unsafe { mft.GetOutputStreamInfo(out_id) }.context("GetOutputStreamInfo")?;
    let provides =
        (MFT_OUTPUT_STREAM_PROVIDES_SAMPLES.0 | MFT_OUTPUT_STREAM_CAN_PROVIDE_SAMPLES.0) as u32;
    info.provides_samples = out_info.dwFlags & provides != 0;
    if !info.provides_samples && out_info.cbSize == 0 {
        return Err(enc("it gives no output buffer size"));
    }
    Ok(out_info)
}

/// Everything the encoder is told before its output type: NVIDIA accepts
/// the keyframe interval later and silently ignores it.
fn configure(
    codec: &ICodecAPI,
    cfg: &EncoderConfig,
    info: &mut EncoderInfo,
) -> Result<(), CaptureError> {
    // SAFETY (each SetValue): a live interface, a static GUID and a
    // scalar VARIANT that owns nothing.
    unsafe {
        info.rate_control_hr = Some(hr(&codec.SetValue(
            &CODECAPI_AVEncCommonRateControlMode,
            &variant_u32(0), // CBR
        )));
        codec
            .SetValue(&CODECAPI_AVEncCommonMeanBitRate, &variant_u32(cfg.bitrate))
            .context("CODECAPI_AVEncCommonMeanBitRate")?;
        codec
            .SetValue(&CODECAPI_AVEncMPVGOPSize, &variant_u32(cfg.gop_frames))
            .context("CODECAPI_AVEncMPVGOPSize")?;
        // NVIDIA refuses this even with 0 and makes no B-frames anyway:
        // B-frames are caught on the output (DTS ≠ PTS) instead.
        info.bframes_hr = Some(hr(
            &codec.SetValue(&CODECAPI_AVEncMPVDefaultBPictureCount, &variant_u32(0))
        ));
        if cfg.low_latency {
            info.low_latency_hr = Some(hr(
                &codec.SetValue(&CODECAPI_AVLowLatencyMode, &variant_bool(true))
            ));
        }
    }
    Ok(())
}

/// The encoder to use, and what its registration says about it.
fn pick(api: &Mf, engine: EncoderEngine) -> Result<(IMFActivate, EncoderInfo), CaptureError> {
    let h264 = MFT_REGISTER_TYPE_INFO {
        guidMajorType: MFMediaType_Video,
        guidSubtype: MFVideoFormat_H264,
    };
    let kind = match engine {
        EncoderEngine::Hardware { .. } => MFT_ENUM_FLAG_HARDWARE.0,
        EncoderEngine::Software => MFT_ENUM_FLAG_SYNCMFT.0,
    };
    let flags = u32::try_from(kind | MFT_ENUM_FLAG_SORTANDFILTER.0).unwrap_or(0);
    let mut list = api.tenum_ex(MFT_CATEGORY_VIDEO_ENCODER, flags, None, Some(&h264))?;
    if list.is_empty() {
        return Err(enc("no H.264 encoder of that kind"));
    }
    let wanted = match engine {
        EncoderEngine::Hardware { vendor_id: Some(v) } => Some(format!("VEN_{v:04X}")),
        _ => None,
    };
    let at = wanted
        .as_deref()
        .and_then(|w| {
            list.iter().position(|a| {
                string_attr(a, &MFT_ENUM_HARDWARE_VENDOR_ID_Attribute).as_deref() == Some(w)
            })
        })
        .unwrap_or_else(|| {
            if wanted.is_some() {
                tracing::info!("no encoder on the device's own GPU vendor; using the first");
            }
            0
        });
    let activate = list.swap_remove(at);
    let info = EncoderInfo {
        name: string_attr(&activate, &MFT_FRIENDLY_NAME_Attribute),
        hardware: string_attr(&activate, &MFT_ENUM_HARDWARE_URL_Attribute).is_some(),
        vendor: string_attr(&activate, &MFT_ENUM_HARDWARE_VENDOR_ID_Attribute),
        // SAFETY: plain getter on the activate's attribute store.
        driver_version: unsafe { activate.GetUINT64(&MFT_GFX_DRIVER_VERSION_ID_Attribute) }.ok(),
        ..EncoderInfo::default()
    };
    Ok((activate, info))
}

/// The encoder's input and output stream ids: 0 and 0 when it doesn't number
/// them (`E_NOTIMPL`, which Microsoft allows for fixed streams).
fn stream_ids(mft: &IMFTransform) -> (u32, u32) {
    let (mut ins, mut outs) = (0u32, 0u32);
    // SAFETY: valid out-pointers on a live encoder.
    if unsafe { mft.GetStreamCount(&raw mut ins, &raw mut outs) }.is_err() || ins == 0 || outs == 0
    {
        return (0, 0);
    }
    let mut in_ids = vec![0u32; ins as usize];
    let mut out_ids = vec![0u32; outs as usize];
    // SAFETY: arrays sized to the counts just read.
    match unsafe { mft.GetStreamIDs(&mut in_ids, &mut out_ids) } {
        Ok(()) => (in_ids[0], out_ids[0]),
        Err(_) => (0, 0),
    }
}

/// The output type's SPS and PPS, if it carries them.
fn sequence_header(mft: &IMFTransform, out_id: u32) -> Option<Arc<[u8]>> {
    // SAFETY: plain getters on a configured encoder and its media type.
    unsafe {
        let t = mft.GetOutputCurrentType(out_id).ok()?;
        let n = t.GetBlobSize(&MF_MT_MPEG_SEQUENCE_HEADER).ok()?;
        let mut v = vec![0u8; n as usize];
        t.GetBlob(&MF_MT_MPEG_SEQUENCE_HEADER, &mut v, None).ok()?;
        (!v.is_empty()).then(|| Arc::from(v))
    }
}

/// A media buffer holding a copy of `bytes`.
fn memory_buffer(
    api: &Mf,
    bytes: &[u8],
) -> Result<windows::Win32::Media::MediaFoundation::IMFMediaBuffer, CaptureError> {
    let len = u32::try_from(bytes.len()).map_err(|_| enc("a frame too large"))?;
    let buffer = api.create_memory_buffer(len)?;
    // SAFETY: Lock hands out at least `len` writable bytes; exactly
    // `bytes.len()` are copied in before Unlock.
    unsafe {
        let mut data: *mut u8 = std::ptr::null_mut();
        let mut max = 0u32;
        buffer
            .Lock(&raw mut data, Some(&raw mut max), None)
            .context("IMFMediaBuffer::Lock")?;
        if data.is_null() || (max as usize) < bytes.len() {
            let _ = buffer.Unlock();
            return Err(enc("Media Foundation gave a short buffer"));
        }
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), data, bytes.len());
        buffer.Unlock().context("IMFMediaBuffer::Unlock")?;
        buffer.SetCurrentLength(len).context("SetCurrentLength")?;
    }
    Ok(buffer)
}

/// Copy one output sample out of Media Foundation.
fn unit(sample: &IMFSample) -> Result<EncodedUnit, CaptureError> {
    // SAFETY: plain getters on a live sample; the buffer is locked, copied
    // and unlocked before anything else touches it.
    unsafe {
        let pts = sample.GetSampleTime().context("GetSampleTime")?;
        let dur = sample.GetSampleDuration().unwrap_or(0);
        let attrs: &IMFAttributes = sample;
        let dts = attrs
            .GetUINT64(&MFSampleExtension_DecodeTimestamp)
            .ok()
            .and_then(|d| i64::try_from(d).ok());
        let clean_point = attrs
            .GetUINT32(&MFSampleExtension_CleanPoint)
            .ok()
            .map(|c| c != 0);
        let buffer = sample
            .ConvertToContiguousBuffer()
            .context("ConvertToContiguousBuffer")?;
        let mut p: *mut u8 = std::ptr::null_mut();
        let mut len = 0u32;
        buffer
            .Lock(&raw mut p, None, Some(&raw mut len))
            .context("IMFMediaBuffer::Lock (output)")?;
        let data: Arc<[u8]> = if p.is_null() {
            Arc::from(Vec::new())
        } else {
            Arc::from(std::slice::from_raw_parts(p, len as usize))
        };
        buffer.Unlock().context("IMFMediaBuffer::Unlock (output)")?;
        Ok(EncodedUnit {
            pts,
            dur,
            dts,
            clean_point,
            data,
        })
    }
}
