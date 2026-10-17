//! The directly driven encoder's device manager and input pool (v1.54.0).
//!
//! A hardware encoder reads its input straight from the GPU and keeps the
//! last texture it was given until the next one arrives (measured on NVIDIA:
//! a pool of one deadlocks, two never ran dry). So the frames go through a
//! small pool of NV12 textures handed out by Media Foundation's sample
//! allocator, which takes a texture back when the encoder lets it go. A pool
//! with nothing free is a dropped frame, never a wait: asking an empty
//! allocator for a sample blocks for about a second.

use windows::core::Interface;
use windows::Win32::Graphics::Direct3D11::{
    ID3D11Texture2D, D3D11_BIND_RENDER_TARGET, D3D11_BIND_VIDEO_ENCODER, D3D11_USAGE_DEFAULT,
};
use windows::Win32::Media::MediaFoundation::{
    IMFDXGIBuffer, IMFDXGIDeviceManager, IMFSample, IMFVideoSampleAllocatorCallback,
    IMFVideoSampleAllocatorEx, MFVideoFormat_NV12, MF_SA_D3D11_BINDFLAGS, MF_SA_D3D11_USAGE,
};

use super::d3d::Gpu;
use super::mf::Mf;
use crate::error::{CaptureError, Context};

/// One Direct3D 11 device's DXGI device manager, shared by the encoder and
/// the pool, so both reach the same GPU the frames are converted on.
pub struct MfDevice {
    pub manager: IMFDXGIDeviceManager,
}

impl MfDevice {
    pub fn new(api: &Mf, gpu: &Gpu) -> Result<Self, CaptureError> {
        let (token, manager) = api.create_dxgi_device_manager()?;
        // SAFETY: the device is alive; the token is the one just issued.
        unsafe { manager.ResetDevice(&gpu.device, token) }.context("ResetDevice")?;
        Ok(Self { manager })
    }
}

/// One frame's texture from the pool, and the sample wrapping it that goes
/// to the encoder.
pub struct PoolFrame {
    pub sample: IMFSample,
    pub texture: ID3D11Texture2D,
}

/// A fixed set of NV12 textures for the encoder's input.
pub struct InputPool {
    allocator: IMFVideoSampleAllocatorEx,
    callback: IMFVideoSampleAllocatorCallback,
    /// The bind flags the textures were made with, for the log.
    pub bind_flags: u32,
}

impl InputPool {
    /// How many textures: NVIDIA needs two; four leaves room for an encoder
    /// that looks further ahead.
    pub const SAMPLES: u32 = 4;

    /// A pool of [`Self::SAMPLES`] textures of `size`, made for an encoder
    /// to read (`D3D11_BIND_VIDEO_ENCODER`) and the converter to write
    /// (`D3D11_BIND_RENDER_TARGET`) — or, where a driver refuses that pair,
    /// render target only, which NVIDIA was measured fine with.
    pub fn new(api: &Mf, dev: &MfDevice, size: (u32, u32), fps: u32) -> Result<Self, CaptureError> {
        let nv12 = super::encoder::video_type(api, MFVideoFormat_NV12, size, fps)?;
        let both = (D3D11_BIND_RENDER_TARGET.0 | D3D11_BIND_VIDEO_ENCODER.0) as u32;
        let target = D3D11_BIND_RENDER_TARGET.0 as u32;
        let mut last = None;
        for bind_flags in [both, target] {
            match Self::with_flags(api, dev, &nv12, bind_flags) {
                Ok(pool) => return Ok(pool),
                Err(e) => {
                    tracing::info!(bind_flags, error = %e, "encoder input pool refused these flags");
                    last = Some(e);
                }
            }
        }
        Err(CaptureError::Encoder(format!(
            "no input pool ({})",
            last.map_or_else(String::new, |e| e.to_string())
        )))
    }

    fn with_flags(
        api: &Mf,
        dev: &MfDevice,
        nv12: &windows::Win32::Media::MediaFoundation::IMFMediaType,
        bind_flags: u32,
    ) -> Result<Self, CaptureError> {
        let allocator = api.create_video_sample_allocator_ex()?;
        let attrs = api.create_attributes(2)?;
        // SAFETY: live objects, valid keys and values; the allocator keeps
        // its own references to the manager and the type.
        unsafe {
            allocator
                .SetDirectXManager(&dev.manager)
                .context("SetDirectXManager")?;
            attrs
                .SetUINT32(&MF_SA_D3D11_BINDFLAGS, bind_flags)
                .context("MF_SA_D3D11_BINDFLAGS")?;
            attrs
                .SetUINT32(&MF_SA_D3D11_USAGE, D3D11_USAGE_DEFAULT.0 as u32)
                .context("MF_SA_D3D11_USAGE")?;
            allocator
                .InitializeSampleAllocatorEx(Self::SAMPLES, Self::SAMPLES, &attrs, nv12)
                .context("InitializeSampleAllocatorEx")?;
        }
        let callback: IMFVideoSampleAllocatorCallback = allocator
            .cast()
            .context("IMFVideoSampleAllocatorCallback")?;
        Ok(Self {
            allocator,
            callback,
            bind_flags,
        })
    }

    /// How many textures are free right now (sub-microsecond).
    pub fn free(&self) -> Result<u32, CaptureError> {
        // SAFETY: plain getter on a live allocator.
        let n = unsafe { self.callback.GetFreeSampleCount() }.context("GetFreeSampleCount")?;
        Ok(u32::try_from(n).unwrap_or(0))
    }

    /// A free texture, or `None` when the encoder holds them all: that tick
    /// is a dropped frame. Never waits.
    pub fn try_acquire(&self) -> Result<Option<PoolFrame>, CaptureError> {
        if self.free()? == 0 {
            return Ok(None);
        }
        // SAFETY: a sample is free (checked just above, on the one thread
        // that allocates), so this returns at once.
        let sample = unsafe { self.allocator.AllocateSample() }.context("AllocateSample")?;
        // SAFETY: live sample; the allocator's samples have one buffer.
        let buffer = unsafe { sample.GetBufferByIndex(0) }.context("GetBufferByIndex")?;
        let dxgi: IMFDXGIBuffer = buffer.cast().context("IMFDXGIBuffer")?;
        let mut p = std::ptr::null_mut();
        // SAFETY: the IID matches the interface asked for; `p` owns a
        // reference on success.
        unsafe { dxgi.GetResource(&ID3D11Texture2D::IID, &raw mut p) }
            .context("IMFDXGIBuffer::GetResource")?;
        if p.is_null() {
            return Err(CaptureError::Encoder(
                "the input pool gave no texture".to_string(),
            ));
        }
        // SAFETY: GetResource returned an owned ID3D11Texture2D.
        let texture = unsafe { ID3D11Texture2D::from_raw(p) };
        Ok(Some(PoolFrame { sample, texture }))
    }
}
