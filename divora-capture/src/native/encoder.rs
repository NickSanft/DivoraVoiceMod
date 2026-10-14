//! H.264 and AAC into MP4 through a Media Foundation sink writer.
//!
//! Rung 1 of the plan's encoder ladder: the sink writer loads the encoders
//! itself, given a DXGI device manager so a hardware H.264 encoder can read
//! the NV12 textures straight from the GPU. Which video encoder it actually
//! loaded is read back rather than assumed. The audio is Windows' own AAC
//! encoder: AAC-LC, 48 kHz stereo, 192 kbps.

use std::ffi::c_void;
use std::path::Path;

use windows::core::{Interface, GUID, HSTRING, PWSTR};
use windows::Win32::Graphics::Direct3D11::ID3D11Texture2D;
use windows::Win32::Media::MediaFoundation::{
    eAVEncH264VProfile_High, IMF2DBuffer, IMFAttributes, IMFDXGIDeviceManager, IMFMediaBuffer,
    IMFMediaType, IMFSinkWriter, IMFTransform, MFAudioFormat_AAC, MFAudioFormat_PCM,
    MFMediaType_Audio, MFMediaType_Video, MFNominalRange_16_235, MFT_ENUM_HARDWARE_URL_Attribute,
    MFT_FRIENDLY_NAME_Attribute, MFTranscodeContainerType_MPEG4, MFVideoFormat_H264,
    MFVideoFormat_NV12, MFVideoInterlace_Progressive, MFVideoPrimaries_BT709, MFVideoTransFunc_709,
    MFVideoTransferMatrix_BT709, MF_MT_AUDIO_AVG_BYTES_PER_SECOND, MF_MT_AUDIO_BITS_PER_SAMPLE,
    MF_MT_AUDIO_BLOCK_ALIGNMENT, MF_MT_AUDIO_NUM_CHANNELS, MF_MT_AUDIO_SAMPLES_PER_SECOND,
    MF_MT_AVG_BITRATE, MF_MT_FRAME_RATE, MF_MT_FRAME_SIZE, MF_MT_INTERLACE_MODE, MF_MT_MAJOR_TYPE,
    MF_MT_MPEG2_PROFILE, MF_MT_PIXEL_ASPECT_RATIO, MF_MT_SUBTYPE, MF_MT_TRANSFER_FUNCTION,
    MF_MT_VIDEO_NOMINAL_RANGE, MF_MT_VIDEO_PRIMARIES, MF_MT_YUV_MATRIX,
    MF_READWRITE_ENABLE_HARDWARE_TRANSFORMS, MF_SINK_WRITER_D3D_MANAGER,
    MF_SINK_WRITER_DISABLE_THROTTLING, MF_TRANSCODE_CONTAINERTYPE,
};
use windows::Win32::System::Com::CoTaskMemFree;

use super::d3d::Gpu;
use super::mf::MfSession;
use crate::error::{CaptureError, Context};
use crate::timeline::{AudioBlock, OUT_RATE};

/// AAC-LC at 192 kbps: one of the four rates Windows' AAC encoder takes,
/// in bytes per second.
const AAC_BYTES_PER_SECOND: u32 = 24_000;
const AUDIO_CHANNELS: u32 = 2;

/// Media Foundation packs two u32s into one u64 attribute (`MFSetAttributeSize`
/// / `MFSetAttributeRatio` are header-only helpers, not DLL exports).
fn pack(hi: u32, lo: u32) -> u64 {
    (u64::from(hi) << 32) | u64::from(lo)
}

/// An open MP4 being written.
pub struct TakeWriter {
    writer: IMFSinkWriter,
    stream: u32,
    audio: Option<u32>,
    session_mf: &'static super::mf::Mf,
    /// Held for the writer's life when frames come from the GPU.
    _manager: Option<IMFDXGIDeviceManager>,
    finished: bool,
}

impl TakeWriter {
    /// Open `path` for writing `size` H.264 at `fps`, `bitrate` bits/s, and
    /// an AAC track when `with_audio`. The container is chosen by attribute,
    /// not by the file's extension, so `path` can be a `.part` file renamed
    /// afterwards.
    ///
    /// With `dev`, frames arrive as GPU textures ([`Self::write`]) and a
    /// hardware encoder may take them directly. Without, they arrive as
    /// bytes ([`Self::write_memory`]) for whichever encoder Windows picks.
    pub fn new(
        mf: &MfSession,
        dev: Option<&Gpu>,
        path: &Path,
        size: (u32, u32),
        fps: u32,
        bitrate: u32,
        with_audio: bool,
    ) -> Result<Self, CaptureError> {
        let api = mf.mf();
        let manager = match dev {
            Some(dev) => {
                let (token, manager) = api.create_dxgi_device_manager()?;
                // SAFETY: the device is alive; the token is the one just issued.
                unsafe { manager.ResetDevice(&dev.device, token) }.context("ResetDevice")?;
                Some(manager)
            }
            None => None,
        };

        let attrs = api.create_attributes(4)?;
        // SAFETY: setters on a live attribute store with valid keys/values.
        unsafe {
            if let Some(manager) = &manager {
                attrs
                    .SetUINT32(&MF_READWRITE_ENABLE_HARDWARE_TRANSFORMS, 1)
                    .context("MF_READWRITE_ENABLE_HARDWARE_TRANSFORMS")?;
                attrs
                    .SetUnknown(&MF_SINK_WRITER_D3D_MANAGER, manager)
                    .context("MF_SINK_WRITER_D3D_MANAGER")?;
            }
            attrs
                .SetGUID(&MF_TRANSCODE_CONTAINERTYPE, &MFTranscodeContainerType_MPEG4)
                .context("MF_TRANSCODE_CONTAINERTYPE")?;
            // Back-pressure stays on: when the encoder falls behind,
            // WriteSample blocks this thread, the pacer sees the lost time,
            // and the ticks are counted as dropped instead of queuing frames
            // without bound.
            attrs
                .SetUINT32(&MF_SINK_WRITER_DISABLE_THROTTLING, 0)
                .context("MF_SINK_WRITER_DISABLE_THROTTLING")?;
        }
        let path_w = HSTRING::from(path.as_os_str());
        let writer = api.create_sink_writer(&path_w, &attrs)?;

        let out = video_type(api, MFVideoFormat_H264, size, fps)?;
        // SAFETY: setters on a live media type.
        unsafe {
            out.SetUINT32(&MF_MT_AVG_BITRATE, bitrate)
                .context("MF_MT_AVG_BITRATE")?;
            out.SetUINT32(&MF_MT_MPEG2_PROFILE, eAVEncH264VProfile_High.0 as u32)
                .context("MF_MT_MPEG2_PROFILE")?;
        }
        // SAFETY: live writer and type.
        let stream = unsafe { writer.AddStream(&out) }.context("AddStream")?;
        let input = video_type(api, MFVideoFormat_NV12, size, fps)?;
        // SAFETY: live writer and type; no encoding parameters.
        unsafe { writer.SetInputMediaType(stream, &input, None::<&IMFAttributes>) }
            .context("SetInputMediaType")?;

        let audio = if with_audio {
            let out = audio_type(api, MFAudioFormat_AAC, AAC_BYTES_PER_SECOND)?;
            // SAFETY: live writer and type.
            let audio = unsafe { writer.AddStream(&out) }.context("AddStream (AAC)")?;
            let pcm = audio_type(api, MFAudioFormat_PCM, OUT_RATE * AUDIO_CHANNELS * 2)?;
            // SAFETY: setter on a live media type.
            unsafe { pcm.SetUINT32(&MF_MT_AUDIO_BLOCK_ALIGNMENT, AUDIO_CHANNELS * 2) }
                .context("MF_MT_AUDIO_BLOCK_ALIGNMENT")?;
            // SAFETY: live writer and type; no encoding parameters.
            unsafe { writer.SetInputMediaType(audio, &pcm, None::<&IMFAttributes>) }
                .context("SetInputMediaType (PCM)")?;
            Some(audio)
        } else {
            None
        };

        // SAFETY: configured above.
        unsafe { writer.BeginWriting() }.context("BeginWriting")?;

        Ok(Self {
            writer,
            stream,
            audio,
            session_mf: api,
            _manager: manager,
            finished: false,
        })
    }

    /// Write one NV12 frame held in memory — [`crate::nv12::nv12_len`] bytes for this
    /// writer's size — at `pts` for `duration` (100-ns ticks).
    pub fn write_memory(&self, nv12: &[u8], pts: u64, duration: u64) -> Result<(), CaptureError> {
        let buffer = self.memory_buffer(nv12)?;
        self.write_sample(self.stream, &buffer, pts, duration, "WriteSample (frame)")
    }

    /// A media buffer holding a copy of `bytes`.
    fn memory_buffer(&self, bytes: &[u8]) -> Result<IMFMediaBuffer, CaptureError> {
        let len = u32::try_from(bytes.len()).unwrap_or(u32::MAX);
        let buffer = self.session_mf.create_memory_buffer(len)?;
        // SAFETY: Lock hands out `max` writable bytes, at least `len` as
        // asked; exactly `bytes.len()` are copied in before Unlock.
        unsafe {
            let mut data: *mut u8 = std::ptr::null_mut();
            let mut max = 0u32;
            buffer
                .Lock(&raw mut data, Some(&raw mut max), None)
                .context("IMFMediaBuffer::Lock")?;
            if data.is_null() || (max as usize) < bytes.len() {
                let _ = buffer.Unlock();
                return Err(CaptureError::Unavailable(
                    "Media Foundation gave a short buffer.".to_string(),
                ));
            }
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), data, bytes.len());
            buffer.Unlock().context("IMFMediaBuffer::Unlock")?;
            buffer.SetCurrentLength(len).context("SetCurrentLength")?;
        }
        Ok(buffer)
    }

    /// Wrap `buffer` in a sample at `pts` for `duration` and write it.
    fn write_sample(
        &self,
        stream: u32,
        buffer: &IMFMediaBuffer,
        pts: u64,
        duration: u64,
        what: &'static str,
    ) -> Result<(), CaptureError> {
        let sample = self.session_mf.create_sample()?;
        // SAFETY: live sample and buffer; times are non-negative.
        unsafe {
            sample.AddBuffer(buffer).context("AddBuffer")?;
            sample
                .SetSampleTime(i64::try_from(pts).unwrap_or(i64::MAX))
                .context("SetSampleTime")?;
            sample
                .SetSampleDuration(i64::try_from(duration).unwrap_or(1).max(1))
                .context("SetSampleDuration")?;
            self.writer.WriteSample(stream, &sample).context(what)?;
        }
        Ok(())
    }

    /// Write one block of the audio track. Ignored when the file has none.
    pub fn write_audio(&self, block: &AudioBlock) -> Result<(), CaptureError> {
        let Some(stream) = self.audio else {
            return Ok(());
        };
        let bytes: Vec<u8> = block.samples.iter().flat_map(|s| s.to_le_bytes()).collect();
        let buffer = self.memory_buffer(&bytes)?;
        self.write_sample(
            stream,
            &buffer,
            block.pts,
            block.duration,
            "WriteSample (audio)",
        )
    }

    /// Write one NV12 frame at `pts` for `duration` (100-ns ticks).
    pub fn write(
        &self,
        nv12: &ID3D11Texture2D,
        pts: u64,
        duration: u64,
    ) -> Result<(), CaptureError> {
        let buffer = self.session_mf.create_dxgi_surface_buffer(nv12)?;
        let two_d: IMF2DBuffer = buffer.cast().context("IMF2DBuffer")?;
        // SAFETY: live buffers; the length comes from the buffer itself.
        unsafe {
            let len = two_d.GetContiguousLength().context("GetContiguousLength")?;
            buffer.SetCurrentLength(len).context("SetCurrentLength")?;
        }
        self.write_sample(self.stream, &buffer, pts, duration, "WriteSample")
    }

    /// The encoder Media Foundation actually loaded: its friendly name, and
    /// whether it is a hardware MFT.
    pub fn encoder_info(&self) -> (Option<String>, bool) {
        let mut p: *mut c_void = std::ptr::null_mut();
        // SAFETY: valid out-pointer; asks for the stream's encoder MFT.
        let got = unsafe {
            self.writer.GetServiceForStream(
                self.stream,
                &GUID::zeroed(),
                &IMFTransform::IID,
                &raw mut p,
            )
        };
        if got.is_err() || p.is_null() {
            return (None, false);
        }
        // SAFETY: GetServiceForStream returned an owned IMFTransform.
        let transform = unsafe { IMFTransform::from_raw(p) };
        // SAFETY: live transform.
        let Ok(attrs) = (unsafe { transform.GetAttributes() }) else {
            return (None, false);
        };
        let name = string_attr(&attrs, &MFT_FRIENDLY_NAME_Attribute);
        let hardware = string_attr(&attrs, &MFT_ENUM_HARDWARE_URL_Attribute).is_some();
        (name, hardware)
    }

    /// Finish the file (writes the MP4 index). Must be called for the file
    /// to be playable.
    pub fn finish(mut self) -> Result<(), CaptureError> {
        self.finished = true;
        // SAFETY: live writer; called once.
        unsafe { self.writer.Finalize() }.context("Finalize")
    }
}

impl Drop for TakeWriter {
    fn drop(&mut self) {
        if !self.finished {
            // An abandoned take: finalize what there is rather than leave an
            // index-less file, and ignore failure (the file is a `.part`).
            // SAFETY: live writer.
            let _ = unsafe { self.writer.Finalize() };
        }
    }
}

fn video_type(
    api: &super::mf::Mf,
    subtype: GUID,
    size: (u32, u32),
    fps: u32,
) -> Result<IMFMediaType, CaptureError> {
    let t = api.create_media_type()?;
    // SAFETY: setters on a live media type with valid keys/values.
    unsafe {
        t.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)
            .context("MF_MT_MAJOR_TYPE")?;
        t.SetGUID(&MF_MT_SUBTYPE, &raw const subtype)
            .context("MF_MT_SUBTYPE")?;
        t.SetUINT32(&MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive.0 as u32)
            .context("MF_MT_INTERLACE_MODE")?;
        t.SetUINT64(&MF_MT_FRAME_SIZE, pack(size.0, size.1))
            .context("MF_MT_FRAME_SIZE")?;
        t.SetUINT64(&MF_MT_FRAME_RATE, pack(fps, 1))
            .context("MF_MT_FRAME_RATE")?;
        t.SetUINT64(&MF_MT_PIXEL_ASPECT_RATIO, pack(1, 1))
            .context("MF_MT_PIXEL_ASPECT_RATIO")?;
        // Say what the video processor produced (studio-range BT.709, see
        // d3d.rs), so the H.264 stream carries it and players don't guess.
        t.SetUINT32(&MF_MT_VIDEO_PRIMARIES, MFVideoPrimaries_BT709.0 as u32)
            .context("MF_MT_VIDEO_PRIMARIES")?;
        t.SetUINT32(&MF_MT_TRANSFER_FUNCTION, MFVideoTransFunc_709.0 as u32)
            .context("MF_MT_TRANSFER_FUNCTION")?;
        t.SetUINT32(&MF_MT_YUV_MATRIX, MFVideoTransferMatrix_BT709.0 as u32)
            .context("MF_MT_YUV_MATRIX")?;
        t.SetUINT32(&MF_MT_VIDEO_NOMINAL_RANGE, MFNominalRange_16_235.0 as u32)
            .context("MF_MT_VIDEO_NOMINAL_RANGE")?;
    }
    Ok(t)
}

/// 48 kHz stereo 16-bit audio of `subtype`, at `bytes_per_second`.
fn audio_type(
    api: &super::mf::Mf,
    subtype: GUID,
    bytes_per_second: u32,
) -> Result<IMFMediaType, CaptureError> {
    let t = api.create_media_type()?;
    // SAFETY: setters on a live media type with valid keys/values.
    unsafe {
        t.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Audio)
            .context("MF_MT_MAJOR_TYPE")?;
        t.SetGUID(&MF_MT_SUBTYPE, &raw const subtype)
            .context("MF_MT_SUBTYPE")?;
        t.SetUINT32(&MF_MT_AUDIO_BITS_PER_SAMPLE, 16)
            .context("MF_MT_AUDIO_BITS_PER_SAMPLE")?;
        t.SetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND, OUT_RATE)
            .context("MF_MT_AUDIO_SAMPLES_PER_SECOND")?;
        t.SetUINT32(&MF_MT_AUDIO_NUM_CHANNELS, AUDIO_CHANNELS)
            .context("MF_MT_AUDIO_NUM_CHANNELS")?;
        t.SetUINT32(&MF_MT_AUDIO_AVG_BYTES_PER_SECOND, bytes_per_second)
            .context("MF_MT_AUDIO_AVG_BYTES_PER_SECOND")?;
    }
    Ok(t)
}

/// A string attribute, if present.
pub(crate) fn string_attr(attrs: &IMFAttributes, key: &GUID) -> Option<String> {
    let mut p = PWSTR::null();
    let mut len = 0u32;
    // SAFETY: valid out-pointers; on success `p` is CoTaskMemAlloc'd.
    unsafe { attrs.GetAllocatedString(key, &raw mut p, &raw mut len) }.ok()?;
    if p.is_null() {
        return None;
    }
    // SAFETY: `p` is a NUL-terminated wide string of `len` characters.
    let s = unsafe { p.to_string() }.ok();
    // SAFETY: allocated by GetAllocatedString with CoTaskMemAlloc.
    unsafe { CoTaskMemFree(Some(p.0.cast_const().cast())) };
    s
}
