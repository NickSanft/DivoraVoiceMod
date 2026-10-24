//! Everything that talks to Windows: monitors, Direct3D 11, Windows
//! Graphics Capture and Media Foundation. See the crate root for why this
//! is the crate's only `unsafe` scope.

pub mod d3d;
pub mod encoder;
pub mod loopback;
pub mod mf;
pub mod mft;
pub mod mux;
pub mod pipeline;
pub mod pool;
pub mod power;
pub mod probe;
pub mod record;
pub mod save;
pub mod shell;
pub mod sink;
pub mod system;
pub mod take_mux;
pub mod variant;
pub mod wgc;
pub mod window;

use windows::core::BOOL;
use windows::Win32::Foundation::{LPARAM, RECT};
use windows::Win32::Graphics::Gdi::{
    EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITORINFO, MONITORINFOEXW,
};

use crate::take::MonitorInfo;

/// The monitors attached right now, primary first.
pub fn list_monitors() -> Vec<MonitorInfo> {
    let mut handles: Vec<HMONITOR> = Vec::new();
    // SAFETY: the callback only pushes into the Vec whose address it is
    // given, and runs synchronously inside EnumDisplayMonitors.
    unsafe {
        let _ = EnumDisplayMonitors(
            Some(HDC::default()),
            None,
            Some(collect_monitor),
            LPARAM(std::ptr::from_mut(&mut handles) as isize),
        );
    }
    let mut out: Vec<MonitorInfo> = handles.into_iter().filter_map(monitor_info).collect();
    out.sort_by_key(|m| !m.primary);
    out
}

/// The HMONITOR for a monitor id from [`list_monitors`].
pub fn find_monitor(id: &str) -> Option<HMONITOR> {
    let mut handles: Vec<HMONITOR> = Vec::new();
    // SAFETY: as in `list_monitors`.
    unsafe {
        let _ = EnumDisplayMonitors(
            Some(HDC::default()),
            None,
            Some(collect_monitor),
            LPARAM(std::ptr::from_mut(&mut handles) as isize),
        );
    }
    handles
        .into_iter()
        .find(|h| monitor_info(*h).is_some_and(|m| m.id == id))
}

unsafe extern "system" fn collect_monitor(
    monitor: HMONITOR,
    _dc: HDC,
    _rect: *mut RECT,
    data: LPARAM,
) -> BOOL {
    // SAFETY: `data` is the `&mut Vec<HMONITOR>` passed by the caller, alive
    // for the whole enumeration.
    let handles = unsafe { &mut *(data.0 as *mut Vec<HMONITOR>) };
    handles.push(monitor);
    BOOL(1)
}

fn monitor_info(monitor: HMONITOR) -> Option<MonitorInfo> {
    let mut info = MONITORINFOEXW::default();
    info.monitorInfo.cbSize = u32::try_from(std::mem::size_of::<MONITORINFOEXW>()).ok()?;
    // SAFETY: `info` is a correctly sized MONITORINFOEXW, passed as its
    // MONITORINFO header, as the API documents.
    let ok = unsafe { GetMonitorInfoW(monitor, (&raw mut info).cast::<MONITORINFO>()) };
    if !ok.as_bool() {
        return None;
    }
    let len = info
        .szDevice
        .iter()
        .position(|&c| c == 0)
        .unwrap_or(info.szDevice.len());
    let id = String::from_utf16_lossy(&info.szDevice[..len]);
    let r = info.monitorInfo.rcMonitor;
    Some(MonitorInfo {
        id,
        x: r.left,
        y: r.top,
        width: u32::try_from(r.right - r.left).unwrap_or(0),
        height: u32::try_from(r.bottom - r.top).unwrap_or(0),
        // MONITORINFOF_PRIMARY
        primary: info.monitorInfo.dwFlags & 1 != 0,
    })
}
