//! Direct3D 11: the device everything shares, and the GPU colour converter.

use std::mem::ManuallyDrop;

use windows::core::{Interface, BOOL};
use windows::Graphics::DirectX::Direct3D11::IDirect3DDevice;
use windows::Win32::Foundation::{HMODULE, RECT};
use windows::Win32::Graphics::Direct3D::{D3D_DRIVER_TYPE_HARDWARE, D3D_FEATURE_LEVEL_11_0};
use windows::Win32::Graphics::Direct3D11::{
    D3D11CreateDevice, ID3D11Device, ID3D11DeviceContext, ID3D11Multithread, ID3D11Resource,
    ID3D11Texture2D, ID3D11VideoContext, ID3D11VideoContext1, ID3D11VideoDevice,
    ID3D11VideoProcessor, ID3D11VideoProcessorEnumerator, ID3D11VideoProcessorInputView,
    ID3D11VideoProcessorOutputView, D3D11_BIND_RENDER_TARGET, D3D11_CPU_ACCESS_READ,
    D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_CREATE_DEVICE_FLAG, D3D11_CREATE_DEVICE_VIDEO_SUPPORT,
    D3D11_MAPPED_SUBRESOURCE, D3D11_MAP_READ, D3D11_SDK_VERSION, D3D11_TEX2D_VPIV,
    D3D11_TEX2D_VPOV, D3D11_TEXTURE2D_DESC, D3D11_USAGE_DEFAULT, D3D11_USAGE_STAGING,
    D3D11_VIDEO_COLOR, D3D11_VIDEO_COLOR_0, D3D11_VIDEO_COLOR_RGBA,
    D3D11_VIDEO_FRAME_FORMAT_PROGRESSIVE, D3D11_VIDEO_PROCESSOR_CONTENT_DESC,
    D3D11_VIDEO_PROCESSOR_INPUT_VIEW_DESC, D3D11_VIDEO_PROCESSOR_INPUT_VIEW_DESC_0,
    D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC, D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC_0,
    D3D11_VIDEO_PROCESSOR_STREAM, D3D11_VIDEO_USAGE_PLAYBACK_NORMAL,
    D3D11_VPIV_DIMENSION_TEXTURE2D, D3D11_VPOV_DIMENSION_TEXTURE2D,
};
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_COLOR_SPACE_RGB_FULL_G22_NONE_P709, DXGI_COLOR_SPACE_YCBCR_STUDIO_G22_LEFT_P709,
    DXGI_FORMAT, DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_FORMAT_NV12, DXGI_RATIONAL, DXGI_SAMPLE_DESC,
};
use windows::Win32::Graphics::Dxgi::IDXGIDevice;
use windows::Win32::System::WinRT::Direct3D11::CreateDirect3D11DeviceFromDXGIDevice;

use crate::error::{CaptureError, Context};

/// One hardware Direct3D 11 device, shared by capture, conversion and the
/// encoder.
pub struct Gpu {
    pub device: ID3D11Device,
    pub context: ID3D11DeviceContext,
    /// The same device, as Windows Graphics Capture wants it.
    pub winrt: IDirect3DDevice,
    /// Whether the device was made with video support — what the GPU colour
    /// converter, and so the directly driven encoder, need (v1.54.0).
    pub video: bool,
}

impl Gpu {
    /// The default adapter's device — with video support when the driver has
    /// it (the GPU colour converter needs it), without when it doesn't (a
    /// basic display adapter, some virtual machines), in which case the take
    /// converts on the CPU instead.
    pub fn new() -> Result<Self, CaptureError> {
        let with_video = D3D11_CREATE_DEVICE_BGRA_SUPPORT | D3D11_CREATE_DEVICE_VIDEO_SUPPORT;
        let ((device, context), video) = match create_device(with_video) {
            Ok(pair) => (pair, true),
            Err(e) => {
                tracing::info!(error = %e, "no Direct3D video support; converting on the CPU");
                (create_device(D3D11_CREATE_DEVICE_BGRA_SUPPORT)?, false)
            }
        };

        // The encoder (inside Media Foundation) and this thread both use the
        // device; protect it.
        let mt: ID3D11Multithread = context.cast().context("ID3D11Multithread")?;
        // SAFETY: plain setter on a live interface.
        unsafe {
            let _ = mt.SetMultithreadProtected(true);
        }

        let dxgi: IDXGIDevice = device.cast().context("IDXGIDevice")?;
        // SAFETY: wraps a live DXGI device for WinRT; no ownership transfer.
        let winrt: IDirect3DDevice = unsafe { CreateDirect3D11DeviceFromDXGIDevice(&dxgi) }
            .context("CreateDirect3D11DeviceFromDXGIDevice")?
            .cast()
            .context("IDirect3DDevice")?;
        Ok(Self {
            device,
            context,
            winrt,
            video,
        })
    }

    /// The PCI vendor id of the adapter this device runs on (0x10DE NVIDIA,
    /// 0x1002 AMD, 0x8086 Intel) — so the encoder picked is the one on the
    /// same GPU as the frames, on a PC with two (v1.54.0).
    pub fn vendor_id(&self) -> Result<u32, CaptureError> {
        let dxgi: IDXGIDevice = self.device.cast().context("IDXGIDevice")?;
        // SAFETY: plain getters on a live DXGI device and its adapter.
        let desc = unsafe { dxgi.GetAdapter().and_then(|a| a.GetDesc()) }
            .context("IDXGIAdapter::GetDesc")?;
        Ok(desc.VendorId)
    }

    /// A GPU texture of `format`, usable as a video-processor input or
    /// output and as an encoder input.
    pub fn texture(
        &self,
        size: (u32, u32),
        format: DXGI_FORMAT,
    ) -> Result<ID3D11Texture2D, CaptureError> {
        let desc = D3D11_TEXTURE2D_DESC {
            Width: size.0,
            Height: size.1,
            MipLevels: 1,
            ArraySize: 1,
            Format: format,
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            Usage: D3D11_USAGE_DEFAULT,
            BindFlags: D3D11_BIND_RENDER_TARGET.0 as u32,
            CPUAccessFlags: 0,
            MiscFlags: 0,
        };
        let mut tex = None;
        // SAFETY: valid descriptor and out-pointer; no initial data.
        unsafe {
            self.device
                .CreateTexture2D(&raw const desc, None, Some(&raw mut tex))
        }
        .context("CreateTexture2D")?;
        tex.ok_or_else(|| unavailable("CreateTexture2D returned nothing"))
    }
}

fn unavailable(why: &str) -> CaptureError {
    CaptureError::Unavailable(format!(
        "Screen recording needs a working graphics driver ({why})."
    ))
}

/// BGRA (the captured screen) → NV12 (what H.264 encoders take), scaled to
/// the output size, on the GPU.
pub struct Nv12Converter {
    vcontext: ID3D11VideoContext,
    vdevice: ID3D11VideoDevice,
    enumerator: ID3D11VideoProcessorEnumerator,
    processor: ID3D11VideoProcessor,
    input_view: ID3D11VideoProcessorInputView,
    out_size: (u32, u32),
    /// v1.54.0: one output view per pooled texture, by the texture's
    /// address, so converting into the encoder's pool makes no view per
    /// frame. A view belongs to this converter's enumerator, so the cache
    /// goes with it when the screen changes size.
    views: Vec<(usize, ID3D11VideoProcessorOutputView)>,
}

/// The most output views kept: the encoder's pool holds four textures.
const VIEW_CACHE: usize = 8;

impl Nv12Converter {
    /// A converter reading from `source` (a BGRA texture of `in_size`) and
    /// producing `out_size` NV12 frames.
    pub fn new(
        dev: &Gpu,
        source: &ID3D11Texture2D,
        in_size: (u32, u32),
        out_size: (u32, u32),
        fps: u32,
    ) -> Result<Self, CaptureError> {
        let vdevice: ID3D11VideoDevice = dev.device.cast().context("ID3D11VideoDevice")?;
        let vcontext: ID3D11VideoContext = dev.context.cast().context("ID3D11VideoContext")?;
        let rate = DXGI_RATIONAL {
            Numerator: fps,
            Denominator: 1,
        };
        let desc = D3D11_VIDEO_PROCESSOR_CONTENT_DESC {
            InputFrameFormat: D3D11_VIDEO_FRAME_FORMAT_PROGRESSIVE,
            InputFrameRate: rate,
            InputWidth: in_size.0,
            InputHeight: in_size.1,
            OutputFrameRate: rate,
            OutputWidth: out_size.0,
            OutputHeight: out_size.1,
            Usage: D3D11_VIDEO_USAGE_PLAYBACK_NORMAL,
        };
        // SAFETY: valid descriptor on a live video device.
        let enumerator = unsafe { vdevice.CreateVideoProcessorEnumerator(&raw const desc) }
            .context("CreateVideoProcessorEnumerator")?;
        // SAFETY: as above.
        let processor = unsafe { vdevice.CreateVideoProcessor(&enumerator, 0) }
            .context("CreateVideoProcessor")?;

        // Screen pixels are full-range sRGB; H.264 players expect studio-
        // range BT.709. Without this the recording looks washed out or
        // crushed depending on the driver's default.
        if let Ok(vc1) = vcontext.cast::<ID3D11VideoContext1>() {
            // SAFETY: setters on live objects with valid enum values.
            unsafe {
                vc1.VideoProcessorSetStreamColorSpace1(
                    &processor,
                    0,
                    DXGI_COLOR_SPACE_RGB_FULL_G22_NONE_P709,
                );
                vc1.VideoProcessorSetOutputColorSpace1(
                    &processor,
                    DXGI_COLOR_SPACE_YCBCR_STUDIO_G22_LEFT_P709,
                );
            }
        }
        let full_in = rect(in_size);
        let full_out = rect(out_size);
        // The picture keeps its shape: a screen that changed mode mid-take
        // is letterboxed into the take's fixed size, on black.
        let (fx, fy, fw, fh) = crate::sizing::fit_rect(in_size, out_size);
        let fitted = RECT {
            left: i32::try_from(fx).unwrap_or(0),
            top: i32::try_from(fy).unwrap_or(0),
            right: i32::try_from(fx + fw).unwrap_or(i32::MAX),
            bottom: i32::try_from(fy + fh).unwrap_or(i32::MAX),
        };
        let black = D3D11_VIDEO_COLOR {
            Anonymous: D3D11_VIDEO_COLOR_0 {
                RGBA: D3D11_VIDEO_COLOR_RGBA {
                    R: 0.0,
                    G: 0.0,
                    B: 0.0,
                    A: 1.0,
                },
            },
        };
        // SAFETY: setters on live objects; the rects are valid for the sizes
        // the processor was created with.
        unsafe {
            vcontext.VideoProcessorSetStreamFrameFormat(
                &processor,
                0,
                D3D11_VIDEO_FRAME_FORMAT_PROGRESSIVE,
            );
            vcontext.VideoProcessorSetStreamSourceRect(
                &processor,
                0,
                true,
                Some(&raw const full_in),
            );
            vcontext.VideoProcessorSetStreamDestRect(&processor, 0, true, Some(&raw const fitted));
            vcontext.VideoProcessorSetOutputBackgroundColor(&processor, false, &raw const black);
            vcontext.VideoProcessorSetOutputTargetRect(&processor, true, Some(&raw const full_out));
            vcontext.VideoProcessorSetStreamAutoProcessingMode(&processor, 0, false);
        }

        let in_desc = D3D11_VIDEO_PROCESSOR_INPUT_VIEW_DESC {
            FourCC: 0,
            ViewDimension: D3D11_VPIV_DIMENSION_TEXTURE2D,
            Anonymous: D3D11_VIDEO_PROCESSOR_INPUT_VIEW_DESC_0 {
                Texture2D: D3D11_TEX2D_VPIV {
                    MipSlice: 0,
                    ArraySlice: 0,
                },
            },
        };
        let mut input_view = None;
        // SAFETY: valid descriptor and out-pointer; `source` is alive.
        unsafe {
            vdevice.CreateVideoProcessorInputView(
                source,
                &enumerator,
                &raw const in_desc,
                Some(&raw mut input_view),
            )
        }
        .context("CreateVideoProcessorInputView")?;
        let input_view = input_view.ok_or_else(|| unavailable("no video input view"))?;

        Ok(Self {
            vcontext,
            vdevice,
            enumerator,
            processor,
            input_view,
            out_size,
            views: Vec::new(),
        })
    }

    /// Convert the current contents of the source texture into a fresh NV12
    /// texture. A new texture per frame: the encoder may still be reading
    /// the previous one asynchronously.
    pub fn convert(&self, dev: &Gpu) -> Result<ID3D11Texture2D, CaptureError> {
        let out = dev.texture(self.out_size, DXGI_FORMAT_NV12)?;
        let out_view = self.output_view(&out)?;
        self.blt(&out_view)?;
        Ok(out)
    }

    /// Convert the current contents of the source texture into `out`, an
    /// NV12 texture of the output size from the encoder's pool (v1.54.0).
    pub fn convert_into(&mut self, out: &ID3D11Texture2D) -> Result<(), CaptureError> {
        let key = out.as_raw() as usize;
        let view = if let Some((_, v)) = self.views.iter().find(|(k, _)| *k == key) {
            v.clone()
        } else {
            let v = self.output_view(out)?;
            if self.views.len() >= VIEW_CACHE {
                self.views.remove(0);
            }
            self.views.push((key, v.clone()));
            v
        };
        self.blt(&view)
    }

    /// A video-processor output view of `out`.
    fn output_view(
        &self,
        out: &ID3D11Texture2D,
    ) -> Result<ID3D11VideoProcessorOutputView, CaptureError> {
        let out_desc = D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC {
            ViewDimension: D3D11_VPOV_DIMENSION_TEXTURE2D,
            Anonymous: D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC_0 {
                Texture2D: D3D11_TEX2D_VPOV { MipSlice: 0 },
            },
        };
        let mut out_view = None;
        // SAFETY: valid descriptor and out-pointer; `out` is alive.
        unsafe {
            self.vdevice.CreateVideoProcessorOutputView(
                out,
                &self.enumerator,
                &raw const out_desc,
                Some(&raw mut out_view),
            )
        }
        .context("CreateVideoProcessorOutputView")?;
        out_view.ok_or_else(|| unavailable("no video output view"))
    }

    /// Run the video processor from the source texture into `out_view`.
    fn blt(&self, out_view: &ID3D11VideoProcessorOutputView) -> Result<(), CaptureError> {
        let mut stream = D3D11_VIDEO_PROCESSOR_STREAM {
            Enable: BOOL(1),
            pInputSurface: ManuallyDrop::new(Some(self.input_view.clone())),
            ..Default::default()
        };
        // SAFETY: the processor, views and stream are live for the call.
        let result = unsafe {
            self.vcontext.VideoProcessorBlt(
                &self.processor,
                out_view,
                0,
                std::slice::from_ref(&stream),
            )
        };
        // The stream struct holds its input view in a ManuallyDrop; release
        // the reference we cloned into it.
        // SAFETY: taken exactly once, after the call that borrowed it.
        unsafe { ManuallyDrop::drop(&mut stream.pInputSurface) };
        result.context("VideoProcessorBlt")
    }
}

/// The CPU fallback's way back from the GPU: each frame is copied into a
/// staging texture, mapped, and converted to NV12 in memory.
pub struct Readback {
    staging: ID3D11Texture2D,
    in_size: (u32, u32),
    out_size: (u32, u32),
}

impl Readback {
    pub fn new(dev: &Gpu, in_size: (u32, u32), out_size: (u32, u32)) -> Result<Self, CaptureError> {
        let desc = D3D11_TEXTURE2D_DESC {
            Width: in_size.0,
            Height: in_size.1,
            MipLevels: 1,
            ArraySize: 1,
            Format: DXGI_FORMAT_B8G8R8A8_UNORM,
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            Usage: D3D11_USAGE_STAGING,
            BindFlags: 0,
            CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32,
            MiscFlags: 0,
        };
        let mut tex = None;
        // SAFETY: valid descriptor and out-pointer; no initial data.
        unsafe {
            dev.device
                .CreateTexture2D(&raw const desc, None, Some(&raw mut tex))
        }
        .context("CreateTexture2D (staging)")?;
        Ok(Self {
            staging: tex.ok_or_else(|| unavailable("CreateTexture2D returned nothing"))?,
            in_size,
            out_size,
        })
    }

    /// Convert `latest` (BGRA, the input size) into `out`, one NV12 frame of
    /// the output size.
    pub fn convert_into(
        &self,
        dev: &Gpu,
        latest: &ID3D11Texture2D,
        out: &mut [u8],
    ) -> Result<(), CaptureError> {
        let dst: ID3D11Resource = self.staging.cast().context("ID3D11Resource")?;
        let src: ID3D11Resource = latest.cast().context("ID3D11Resource")?;
        let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
        // SAFETY: same size and format; the map is released below before the
        // staging texture is touched again.
        unsafe {
            dev.context.CopyResource(&dst, &src);
            dev.context
                .Map(&dst, 0, D3D11_MAP_READ, 0, Some(&raw mut mapped))
                .context("Map (staging)")?;
        }
        let pitch = mapped.RowPitch as usize;
        let len = pitch * self.in_size.1 as usize;
        if mapped.pData.is_null() {
            // SAFETY: mapped above.
            unsafe { dev.context.Unmap(&dst, 0) };
            return Err(unavailable("Map returned no data"));
        }
        // SAFETY: a successful Map of a staging texture exposes `RowPitch ×
        // height` readable bytes until Unmap.
        let bytes = unsafe { std::slice::from_raw_parts(mapped.pData.cast::<u8>(), len) };
        crate::nv12::bgra_to_nv12(bytes, pitch, self.in_size, self.out_size, out);
        // SAFETY: balances the Map above; `bytes` is not used past here.
        unsafe { dev.context.Unmap(&dst, 0) };
        Ok(())
    }
}

/// A hardware Direct3D 11 device at feature level 11.0 with `flags`.
fn create_device(
    flags: D3D11_CREATE_DEVICE_FLAG,
) -> Result<(ID3D11Device, ID3D11DeviceContext), CaptureError> {
    let mut device = None;
    let mut context = None;
    // SAFETY: valid out-pointers; no adapter (default), hardware driver.
    unsafe {
        D3D11CreateDevice(
            None,
            D3D_DRIVER_TYPE_HARDWARE,
            HMODULE::default(),
            flags,
            Some(&[D3D_FEATURE_LEVEL_11_0]),
            D3D11_SDK_VERSION,
            Some(&raw mut device),
            None,
            Some(&raw mut context),
        )
    }
    .context("D3D11CreateDevice")?;
    Ok((
        device.ok_or_else(|| unavailable("no Direct3D 11 device"))?,
        context.ok_or_else(|| unavailable("no Direct3D 11 context"))?,
    ))
}

/// The BGRA texture the converter reads from: each captured frame is copied
/// into it, so the capture pool gets its buffer back at once.
pub fn latest_frame_texture(dev: &Gpu, size: (u32, u32)) -> Result<ID3D11Texture2D, CaptureError> {
    dev.texture(size, DXGI_FORMAT_B8G8R8A8_UNORM)
}

fn rect(size: (u32, u32)) -> RECT {
    RECT {
        left: 0,
        top: 0,
        right: i32::try_from(size.0).unwrap_or(i32::MAX),
        bottom: i32::try_from(size.1).unwrap_or(i32::MAX),
    }
}
