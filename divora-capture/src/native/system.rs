//! What Replay asks of Windows besides the screen (v1.54.0): how much memory
//! the PC has, how much room a drive has, and the time of day. All three
//! are kernel32, which the app already imports.

use std::os::windows::ffi::OsStrExt;
use std::path::Path;

use windows::core::PCWSTR;
use windows::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
use windows::Win32::System::SystemInformation::{
    GetLocalTime, GlobalMemoryStatusEx, MEMORYSTATUSEX,
};

use crate::pipeline::LocalTime;

/// Overrides [`free_bytes`] with this many bytes, for checking what a full
/// disk does without filling one.
pub const PRETEND_FREE_ENV: &str = "DIVORA_PRETEND_FREE_BYTES";

/// The memory available to Windows (`ullTotalPhys`): less than what is
/// installed, by what firmware and drivers keep. 0 if Windows won't say.
#[must_use]
pub fn total_physical_memory() -> u64 {
    let mut m = MEMORYSTATUSEX {
        dwLength: u32::try_from(std::mem::size_of::<MEMORYSTATUSEX>()).unwrap_or(0),
        ..MEMORYSTATUSEX::default()
    };
    // SAFETY: a correctly sized MEMORYSTATUSEX with its length set.
    match unsafe { GlobalMemoryStatusEx(&raw mut m) } {
        Ok(()) => m.ullTotalPhys,
        Err(_) => 0,
    }
}

/// The bytes this user may still write on the drive holding `dir` (quotas
/// counted), or `None` if Windows won't say.
#[must_use]
pub fn free_bytes(dir: &Path) -> Option<u64> {
    if let Some(n) = std::env::var_os(PRETEND_FREE_ENV)
        .and_then(|v| v.to_str().and_then(|s| s.trim().parse::<u64>().ok()))
    {
        return Some(n);
    }
    let wide: Vec<u16> = dir.as_os_str().encode_wide().chain(Some(0)).collect();
    let mut free = 0u64;
    // SAFETY: a NUL-terminated path and a valid out-pointer.
    unsafe { GetDiskFreeSpaceExW(PCWSTR(wide.as_ptr()), Some(&raw mut free), None, None) }
        .ok()
        .map(|()| free)
}

/// The time of day where the user is, for naming a saved replay.
#[must_use]
pub fn local_time() -> LocalTime {
    // SAFETY: no arguments; it cannot fail.
    let t = unsafe { GetLocalTime() };
    LocalTime {
        year: t.wYear,
        month: u8::try_from(t.wMonth).unwrap_or(1),
        day: u8::try_from(t.wDay).unwrap_or(1),
        hour: u8::try_from(t.wHour).unwrap_or(0),
        minute: u8::try_from(t.wMinute).unwrap_or(0),
        second: u8::try_from(t.wSecond).unwrap_or(0),
    }
}

#[cfg(test)]
mod tests {
    use super::{free_bytes, local_time, total_physical_memory};

    /// Each answers with something plausible on any PC that runs the tests.
    #[test]
    fn windows_answers() {
        assert!(total_physical_memory() > 256 * 1024 * 1024);
        let dir = std::env::temp_dir();
        assert!(free_bytes(&dir).is_some_and(|b| b > 0));
        let t = local_time();
        assert!(t.year >= 2024 && (1..=12).contains(&t.month) && (1..=31).contains(&t.day));
    }
}
