//! The stream overlay's window calls (v1.55.0): the shell's `NonRudeHWND`
//! mark, whether Windows can leave a window out of captures, the foreground
//! without injected input, and the layered-window fix the click-through
//! ladder may need. All user32, plus one Windows Runtime query the crate
//! already makes, so the exe gains no DLL import.
//!
//! Windows are passed as integers (`HWND.0 as isize`), so the app above
//! needs neither `unsafe` nor the `windows` crate. 0 is no window, and
//! every call is a no-op or `false` for it.

use std::ffi::c_void;
use std::sync::OnceLock;

use windows::core::{w, HSTRING, PCWSTR};
use windows::Foundation::Metadata::ApiInformation;
use windows::Win32::Foundation::{COLORREF, HANDLE, HWND};
use windows::Win32::UI::WindowsAndMessaging::{
    GetForegroundWindow, GetWindowDisplayAffinity, IsIconic, IsWindow, IsWindowVisible,
    RemovePropW, SetForegroundWindow, SetLayeredWindowAttributes, SetPropW, LWA_ALPHA,
};

/// Set to anything to make this process behave as if Windows couldn't leave
/// a window out of captures, so the Hide from recordings row is hidden.
pub const PRETEND_NO_CAPTURE_EXCLUSION_ENV: &str = "DIVORA_PRETEND_NO_CAPTURE_EXCLUSION";

/// The `UniversalApiContract` major the toggle needs. 10 = Windows 10 2004,
/// the floor `WDA_EXCLUDEFROMCAPTURE` needs. Raised to 15 (Windows 11 22H2,
/// build 22621; the SDK's 10.0.22621.0 Platform.xml lists 15.0.0.0) if R3's
/// common-path check shows the overlay on Windows 10.
pub const MIN_CONTRACT: u16 = 10;

/// The shell's property: a window carrying it is never taken for a
/// fullscreen app (Raymond Chen, 2025-05-22).
const NON_RUDE: PCWSTR = w!("NonRudeHWND");

fn hwnd(h: isize) -> HWND {
    HWND(std::ptr::without_provenance_mut::<c_void>(
        h.cast_unsigned(),
    ))
}

/// `NonRudeHWND` (Raymond Chen, 2025-05-22): the shell won't treat this
/// window as a fullscreen app, so a fitted overlay doesn't hide the taskbar
/// or hold back notifications. Read when the window is shown, so set it
/// before the first show. Main thread.
pub fn mark_non_rude(window: isize) -> bool {
    if window == 0 {
        return false;
    }
    // Any non-null value marks it; the shell only checks the property is there.
    let one = HANDLE(std::ptr::without_provenance_mut::<c_void>(1));
    // SAFETY: a window handle (a stale one only makes the call fail) and a
    // static wide string.
    unsafe { SetPropW(hwnd(window), NON_RUDE, Some(one)) }.is_ok()
}

/// Remove [`mark_non_rude`]'s property, releasing its atom reference.
/// Never set, or the window gone: nothing happens.
pub fn unmark_non_rude(window: isize) {
    if window == 0 {
        return;
    }
    // SAFETY: as in `mark_non_rude`. The returned value is the 1 set there,
    // not a handle to free.
    let _ = unsafe { RemovePropW(hwnd(window), NON_RUDE) };
}

/// Whether Windows can leave a window out of captures here:
/// `IsApiContractPresentByMajor("Windows.Foundation.UniversalApiContract",
/// MIN_CONTRACT)`, as `loopback.rs` asks it. Checked once per process;
/// [`PRETEND_NO_CAPTURE_EXCLUSION_ENV`] answers false whenever it is set.
pub fn capture_exclusion_available() -> bool {
    static AVAILABLE: OnceLock<bool> = OnceLock::new();
    if std::env::var_os(PRETEND_NO_CAPTURE_EXCLUSION_ENV).is_some() {
        return false;
    }
    *AVAILABLE.get_or_init(|| {
        ApiInformation::IsApiContractPresentByMajor(
            &HSTRING::from("Windows.Foundation.UniversalApiContract"),
            MIN_CONTRACT,
        )
        .unwrap_or(false)
    })
}

/// `GetWindowDisplayAffinity`, for a log line only: it reports success even
/// where capture ignores the affinity.
pub fn display_affinity(window: isize) -> Option<u32> {
    if window == 0 {
        return None;
    }
    let mut affinity = 0u32;
    // SAFETY: a window handle and a valid out-pointer.
    unsafe { GetWindowDisplayAffinity(hwnd(window), &raw mut affinity) }
        .ok()
        .map(|()| affinity)
}

/// The window that has the foreground, as an integer; 0 when none.
pub fn foreground_window() -> isize {
    // SAFETY: no arguments.
    let h = unsafe { GetForegroundWindow() };
    h.0 as isize
}

/// `SetForegroundWindow`, and nothing else: unlike tao's `set_focus` it
/// never injects input when Windows refuses. False for 0 or a refusal.
pub fn try_foreground(window: isize) -> bool {
    if window == 0 {
        return false;
    }
    // SAFETY: a window handle; a stale one only makes the call fail.
    unsafe { SetForegroundWindow(hwnd(window)) }.as_bool()
}

/// Whether `window` may take the foreground back: it exists, is visible
/// and isn't minimised.
pub fn can_take_foreground(window: isize) -> bool {
    if window == 0 {
        return false;
    }
    let h = hwnd(window);
    // SAFETY: plain queries on a window handle; a stale one answers false.
    unsafe { IsWindow(Some(h)).as_bool() && IsWindowVisible(h).as_bool() && !IsIconic(h).as_bool() }
}

/// R1 rung 1 only: `SetLayeredWindowAttributes(window, 0, 255, LWA_ALPHA)`,
/// so a layered window that nobody gave attributes still draws. False when
/// Windows refuses (a window that isn't layered).
pub fn make_layered_opaque(window: isize) -> bool {
    if window == 0 {
        return false;
    }
    // SAFETY: a window handle; a stale or unlayered one only makes it fail.
    unsafe { SetLayeredWindowAttributes(hwnd(window), COLORREF(0), 255, LWA_ALPHA) }.is_ok()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::{
        can_take_foreground, capture_exclusion_available, display_affinity, foreground_window,
        make_layered_opaque, mark_non_rude, try_foreground, unmark_non_rude, NON_RUDE,
        PRETEND_NO_CAPTURE_EXCLUSION_ENV,
    };
    use windows::core::w;
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DestroyWindow, GetPropW, WINDOW_EX_STYLE, WS_EX_LAYERED, WS_POPUP,
    };

    /// A hidden window of the predefined STATIC class (nothing to register),
    /// destroyed on drop.
    struct Hidden(HWND);

    impl Hidden {
        fn new(ex: WINDOW_EX_STYLE) -> Self {
            // SAFETY: a predefined class, no parent, menu or creation data.
            let h = unsafe {
                CreateWindowExW(
                    ex,
                    w!("STATIC"),
                    w!("divora window test"),
                    WS_POPUP,
                    0,
                    0,
                    16,
                    16,
                    None,
                    None,
                    None,
                    None,
                )
            }
            .unwrap();
            Self(h)
        }

        fn id(&self) -> isize {
            self.0 .0 as isize
        }

        fn non_rude(&self) -> bool {
            // SAFETY: our own live window and a static wide string.
            !unsafe { GetPropW(self.0, NON_RUDE) }.is_invalid()
        }
    }

    impl Drop for Hidden {
        fn drop(&mut self) {
            // SAFETY: our own window, destroyed once on the thread that made it.
            let _ = unsafe { DestroyWindow(self.0) };
        }
    }

    /// No window is a no-op or false, never a call on handle 0.
    #[test]
    fn no_window_is_nothing() {
        assert!(!mark_non_rude(0));
        unmark_non_rude(0);
        assert_eq!(display_affinity(0), None);
        assert!(!try_foreground(0));
        assert!(!can_take_foreground(0));
        assert!(!make_layered_opaque(0));
    }

    /// The property goes on and comes off the real window.
    #[test]
    fn non_rude_marks_and_unmarks() {
        let win = Hidden::new(WINDOW_EX_STYLE(0));
        assert!(!win.non_rude());
        assert!(mark_non_rude(win.id()));
        assert!(win.non_rude());
        unmark_non_rude(win.id());
        assert!(!win.non_rude());
        // Twice is fine.
        unmark_non_rude(win.id());
    }

    /// A new window has no affinity; a hidden one can't take the foreground
    /// back; only a layered window takes layered attributes.
    #[test]
    fn queries_on_a_real_window() {
        let plain = Hidden::new(WINDOW_EX_STYLE(0));
        assert_eq!(display_affinity(plain.id()), Some(0));
        assert!(!can_take_foreground(plain.id()), "hidden");
        assert!(!make_layered_opaque(plain.id()), "not layered");
        let layered = Hidden::new(WS_EX_LAYERED);
        assert!(make_layered_opaque(layered.id()));
        // Ours is hidden, so it is never the foreground.
        assert_ne!(foreground_window(), plain.id());
    }

    /// The test machines (Windows 11, Windows Server 2025) have the
    /// contract; the switch answers false while it is set, cached or not.
    #[test]
    fn capture_exclusion_is_available_here() {
        assert!(capture_exclusion_available());
        std::env::set_var(PRETEND_NO_CAPTURE_EXCLUSION_ENV, "1");
        let pretended = capture_exclusion_available();
        std::env::remove_var(PRETEND_NO_CAPTURE_EXCLUSION_ENV);
        assert!(!pretended);
        assert!(capture_exclusion_available());
    }
}
