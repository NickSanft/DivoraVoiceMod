//! H.264 and AAC into MP4 through a Media Foundation sink writer.
//!
//! Rung 1 of the plan's encoder ladder: the sink writer loads the encoders
//! itself, given a DXGI device manager so a hardware H.264 encoder can read
//! the NV12 textures straight from the GPU. Which video encoder it actually
//! loaded is read back rather than assumed. The audio is Windows' own AAC
//! encoder: AAC-LC, 48 kHz stereo, 192 kbps.

use std::ffi::c_void;
use std::path::Path;

use windows::core::{Interface, GUID, PWSTR};
use windows::Win32::Graphics::Direct3D11::ID3D11Texture2D;
use windows::Win32::Media::MediaFoundation::{
    eAVEncH264VProfile_High, IMF2DBuffer, IMFAttributes, IMFDXGIDeviceManager, IMFMediaType,
    IMFTransform, MFMediaType_Video, MFNominalRange_16_235, MFT_ENUM_HARDWARE_URL_Attribute,
    MFT_FRIENDLY_NAME_Attribute, MFVideoFormat_H264, MFVideoFormat_NV12,
    MFVideoInterlace_Progressive, MFVideoPrimaries_BT709, MFVideoTransFunc_709,
    MFVideoTransferMatrix_BT709, MF_MT_AVG_BITRATE, MF_MT_FRAME_RATE, MF_MT_FRAME_SIZE,
    MF_MT_INTERLACE_MODE, MF_MT_MAJOR_TYPE, MF_MT_MPEG2_PROFILE, MF_MT_PIXEL_ASPECT_RATIO,
    MF_MT_SUBTYPE, MF_MT_TRANSFER_FUNCTION, MF_MT_VIDEO_NOMINAL_RANGE, MF_MT_VIDEO_PRIMARIES,
    MF_MT_YUV_MATRIX, MF_READWRITE_ENABLE_HARDWARE_TRANSFORMS, MF_SINK_WRITER_D3D_MANAGER,
};
use windows::Win32::System::Com::CoTaskMemFree;

use super::d3d::Gpu;
use super::mf::MfSession;
use super::sink::SinkCore;
use crate::error::{CaptureError, Context};
use crate::timeline::AudioBlock;

/// Media Foundation packs two u32s into one u64 attribute (`MFSetAttributeSize`
/// / `MFSetAttributeRatio` are header-only helpers, not DLL exports).
fn pack(hi: u32, lo: u32) -> u64 {
    (u64::from(hi) << 32) | u64::from(lo)
}

/// An open MP4 being written.
pub struct TakeWriter {
    core: SinkCore,
    stream: u32,
    /// Held for the writer's life when frames come from the GPU.
    _manager: Option<IMFDXGIDeviceManager>,
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

        // Back-pressure stays on: when the encoder falls behind, WriteSample
        // blocks this thread, the pacer sees the lost time, and the ticks are
        // counted as dropped instead of queuing frames without bound.
        let mut core = SinkCore::create(mf, path, true, &|attrs: &IMFAttributes| {
            if let Some(manager) = &manager {
                // SAFETY: setters on a live attribute store with valid keys/values.
                unsafe {
                    attrs
                        .SetUINT32(&MF_READWRITE_ENABLE_HARDWARE_TRANSFORMS, 1)
                        .context("MF_READWRITE_ENABLE_HARDWARE_TRANSFORMS")?;
                    attrs
                        .SetUnknown(&MF_SINK_WRITER_D3D_MANAGER, manager)
                        .context("MF_SINK_WRITER_D3D_MANAGER")?;
                }
            }
            Ok(())
        })?;

        let out = video_type(api, MFVideoFormat_H264, size, fps)?;
        // SAFETY: setters on a live media type.
        unsafe {
            out.SetUINT32(&MF_MT_AVG_BITRATE, bitrate)
                .context("MF_MT_AVG_BITRATE")?;
            out.SetUINT32(&MF_MT_MPEG2_PROFILE, eAVEncH264VProfile_High.0 as u32)
                .context("MF_MT_MPEG2_PROFILE")?;
        }
        // SAFETY: live writer and type.
        let stream = unsafe { core.writer.AddStream(&out) }.context("AddStream")?;
        let input = video_type(api, MFVideoFormat_NV12, size, fps)?;
        // SAFETY: live writer and type; no encoding parameters.
        unsafe {
            core.writer
                .SetInputMediaType(stream, &input, None::<&IMFAttributes>)
        }
        .context("SetInputMediaType")?;

        if with_audio {
            core.add_aac_from_pcm()?;
        }
        core.begin()?;

        Ok(Self {
            core,
            stream,
            _manager: manager,
        })
    }

    /// Write one NV12 frame held in memory — [`crate::nv12::nv12_len`] bytes for this
    /// writer's size — at `pts` for `duration` (100-ns ticks).
    pub fn write_memory(&self, nv12: &[u8], pts: u64, duration: u64) -> Result<(), CaptureError> {
        let buffer = self.core.memory_buffer(nv12)?;
        self.core
            .write_sample(self.stream, &buffer, pts, duration, "WriteSample (frame)")
    }

    /// Write one block of the audio track. Ignored when the file has none.
    pub fn write_audio(&self, block: &AudioBlock) -> Result<(), CaptureError> {
        self.core.write_audio(block)
    }

    /// Write one NV12 frame at `pts` for `duration` (100-ns ticks).
    pub fn write(
        &self,
        nv12: &ID3D11Texture2D,
        pts: u64,
        duration: u64,
    ) -> Result<(), CaptureError> {
        let buffer = self.core.api().create_dxgi_surface_buffer(nv12)?;
        let two_d: IMF2DBuffer = buffer.cast().context("IMF2DBuffer")?;
        // SAFETY: live buffers; the length comes from the buffer itself.
        unsafe {
            let len = two_d.GetContiguousLength().context("GetContiguousLength")?;
            buffer.SetCurrentLength(len).context("SetCurrentLength")?;
        }
        self.core
            .write_sample(self.stream, &buffer, pts, duration, "WriteSample")
    }

    /// The encoder Media Foundation actually loaded: its friendly name, and
    /// whether it is a hardware MFT.
    pub fn encoder_info(&self) -> (Option<String>, bool) {
        let mut p: *mut c_void = std::ptr::null_mut();
        // SAFETY: valid out-pointer; asks for the stream's encoder MFT.
        let got = unsafe {
            self.core.writer.GetServiceForStream(
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
    pub fn finish(self) -> Result<(), CaptureError> {
        self.core.finish()
    }
}

pub(crate) fn video_type(
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
