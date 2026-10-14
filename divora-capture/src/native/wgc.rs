//! Windows Graphics Capture of one monitor.

use std::sync::{Arc, Condvar, Mutex, PoisonError};
use std::time::Duration;

use windows::core::{Interface, HSTRING};
use windows::Foundation::Metadata::ApiInformation;
use windows::Foundation::TypedEventHandler;
use windows::Graphics::Capture::{
    Direct3D11CaptureFramePool, GraphicsCaptureItem, GraphicsCaptureSession,
};
use windows::Graphics::DirectX::DirectXPixelFormat;
use windows::Graphics::SizeInt32;
use windows::Win32::Graphics::Direct3D11::{ID3D11Resource, ID3D11Texture2D, D3D11_BOX};
use windows::Win32::Graphics::Gdi::HMONITOR;
use windows::Win32::System::WinRT::Direct3D11::IDirect3DDxgiInterfaceAccess;
use windows::Win32::System::WinRT::Graphics::Capture::IGraphicsCaptureItemInterop;

use super::d3d::Gpu;
use crate::error::{CaptureError, Context};

/// Set by the capture thread pool when a frame arrives; waited on by the
/// recording thread. `FrameArrived` does nothing else: all Direct3D work
/// happens on the recording thread.
#[derive(Default)]
struct FrameSignal {
    pending: Mutex<bool>,
    cv: Condvar,
}

impl FrameSignal {
    fn notify(&self) {
        *self.pending.lock().unwrap_or_else(PoisonError::into_inner) = true;
        self.cv.notify_one();
    }

    /// Wait up to `timeout` for a frame; true if one arrived. Clears the flag.
    fn wait(&self, timeout: Duration) -> bool {
        let guard = self.pending.lock().unwrap_or_else(PoisonError::into_inner);
        let (mut guard, _) = self
            .cv
            .wait_timeout_while(guard, timeout, |p| !*p)
            .unwrap_or_else(PoisonError::into_inner);
        std::mem::replace(&mut *guard, false)
    }
}

/// What the latest drain found.
pub struct Drained {
    /// Frames taken from the pool (only the newest was kept).
    pub frames: u64,
    /// The monitor is now this size, not the pool's: nothing was copied,
    /// and the pool must be recreated ([`MonitorCapture::recreate`]) before
    /// frames are whole again.
    pub resized: Option<(u32, u32)>,
}

/// A running capture of one monitor.
pub struct MonitorCapture {
    _item: GraphicsCaptureItem,
    pool: Direct3D11CaptureFramePool,
    session: GraphicsCaptureSession,
    token: i64,
    signal: Arc<FrameSignal>,
    size: (u32, u32),
}

impl MonitorCapture {
    /// Start capturing `monitor`. Windows draws its yellow capture border
    /// around the monitor until this is dropped.
    pub fn start(dev: &Gpu, monitor: HMONITOR, cursor: bool) -> Result<Self, CaptureError> {
        let interop = windows::core::factory::<GraphicsCaptureItem, IGraphicsCaptureItemInterop>()
            .context("IGraphicsCaptureItemInterop")?;
        // SAFETY: `monitor` came from EnumDisplayMonitors in this process.
        let item: GraphicsCaptureItem =
            unsafe { interop.CreateForMonitor(monitor) }.context("CreateForMonitor")?;
        let size = item.Size().context("GraphicsCaptureItem::Size")?;
        let pool = Direct3D11CaptureFramePool::CreateFreeThreaded(
            &dev.winrt,
            DirectXPixelFormat::B8G8R8A8UIntNormalized,
            2,
            size,
        )
        .context("CreateFreeThreaded")?;

        let signal = Arc::new(FrameSignal::default());
        let for_handler = Arc::clone(&signal);
        let token = pool
            .FrameArrived(&TypedEventHandler::new(move |_, _| {
                for_handler.notify();
                Ok(())
            }))
            .context("FrameArrived")?;

        let session = pool
            .CreateCaptureSession(&item)
            .context("CreateCaptureSession")?;
        // Windows 10 2004+: the cursor can be left out. Older builds always
        // draw it, and the property doesn't exist to set.
        if ApiInformation::IsPropertyPresent(
            &HSTRING::from("Windows.Graphics.Capture.GraphicsCaptureSession"),
            &HSTRING::from("IsCursorCaptureEnabled"),
        )
        .unwrap_or(false)
        {
            session
                .SetIsCursorCaptureEnabled(cursor)
                .context("SetIsCursorCaptureEnabled")?;
        }
        session.StartCapture().context("StartCapture")?;

        Ok(Self {
            _item: item,
            pool,
            session,
            token,
            signal,
            size: (
                u32::try_from(size.Width).unwrap_or(0),
                u32::try_from(size.Height).unwrap_or(0),
            ),
        })
    }

    /// The monitor's size in pixels, as captured.
    pub fn size(&self) -> (u32, u32) {
        self.size
    }

    /// Wait up to `timeout` for the next frame to arrive.
    pub fn wait(&self, timeout: Duration) -> bool {
        self.signal.wait(timeout)
    }

    /// Take every frame waiting in the pool and copy the newest into
    /// `latest`, which must be a BGRA texture of [`Self::size`]. The pool's
    /// buffers go straight back to Windows.
    pub fn drain_into(&self, dev: &Gpu, latest: &ID3D11Texture2D) -> Result<Drained, CaptureError> {
        let mut frames = 0u64;
        let mut newest = None;
        // TryGetNextFrame yields an error once the pool is empty.
        while let Ok(frame) = self.pool.TryGetNextFrame() {
            frames += 1;
            newest = Some(frame);
        }
        let Some(frame) = newest else {
            return Ok(Drained {
                frames,
                resized: None,
            });
        };
        let surface = frame.Surface().context("CaptureFrame::Surface")?;
        let access: IDirect3DDxgiInterfaceAccess =
            surface.cast().context("IDirect3DDxgiInterfaceAccess")?;
        // SAFETY: the surface is alive; GetInterface AddRefs the texture.
        let tex: ID3D11Texture2D = unsafe { access.GetInterface() }
            .context("IDirect3DDxgiInterfaceAccess::GetInterface")?;

        // The pool's buffers are the monitor size it was created with. A
        // mode change (a game switching resolution, a DPI change) makes the
        // content a different size: say so, so the caller can recreate the
        // pool, rather than copy a cropped or padded picture.
        let content = frame.ContentSize().context("CaptureFrame::ContentSize")?;
        let now = (
            u32::try_from(content.Width).unwrap_or(0),
            u32::try_from(content.Height).unwrap_or(0),
        );
        if now != self.size && now.0 > 0 && now.1 > 0 {
            return Ok(Drained {
                frames,
                resized: Some(now),
            });
        }
        let (w, h) = (now.0.min(self.size.0), now.1.min(self.size.1));
        let src: ID3D11Resource = tex.cast().context("ID3D11Resource")?;
        let dst: ID3D11Resource = latest.cast().context("ID3D11Resource")?;
        if (w, h) == self.size {
            // SAFETY: same size and format (both BGRA, monitor-sized).
            unsafe { dev.context.CopyResource(&dst, &src) };
        } else {
            let region = D3D11_BOX {
                left: 0,
                top: 0,
                front: 0,
                right: w,
                bottom: h,
                back: 1,
            };
            // SAFETY: the box lies inside both textures.
            unsafe {
                dev.context.CopySubresourceRegion(
                    &dst,
                    0,
                    0,
                    0,
                    0,
                    &src,
                    0,
                    Some(&raw const region),
                );
            }
        }
        // `frame` drops here, returning its buffer to the pool.
        Ok(Drained {
            frames,
            resized: None,
        })
    }

    /// Recreate the pool's buffers at `size`, after a [`Drained::resized`].
    pub fn recreate(&mut self, dev: &Gpu, size: (u32, u32)) -> Result<(), CaptureError> {
        let s = SizeInt32 {
            Width: i32::try_from(size.0).unwrap_or(i32::MAX),
            Height: i32::try_from(size.1).unwrap_or(i32::MAX),
        };
        self.pool
            .Recreate(&dev.winrt, DirectXPixelFormat::B8G8R8A8UIntNormalized, 2, s)
            .context("Direct3D11CaptureFramePool::Recreate")?;
        self.size = size;
        Ok(())
    }
}

impl Drop for MonitorCapture {
    fn drop(&mut self) {
        let _ = self.pool.RemoveFrameArrived(self.token);
        let _ = self.session.Close();
        let _ = self.pool.Close();
    }
}
