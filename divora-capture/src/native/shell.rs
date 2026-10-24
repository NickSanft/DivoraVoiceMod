//! v1.57.0: open a link, folder or program from the desktop's own Explorer,
//! so whatever starts is Explorer's child, not Divora's.
//!
//! "Everything except Divora" captures with
//! `PROCESS_LOOPBACK_MODE_EXCLUDE_TARGET_PROCESS_TREE` on Divora's PID, which
//! leaves out Divora and every process it started. A link opened with the
//! shell plugin's `open` is a `ShellExecuteExW` from Divora's own process: a
//! browser that wasn't running starts as Divora's child, and the sound of
//! everything played in it afterwards was left out of the recording until
//! the browser was restarted (`tests/launch_outside_tree.rs` measures both).
//!
//! The way out is Raymond Chen's: find the desktop's shell view in the
//! running Explorer, take its `Application` object (`IShellDispatch2`), and
//! call its `ShellExecute`, which runs inside Explorer. The new process is
//! Explorer's child and runs at Explorer's integrity level. Divora hands
//! Explorer its right to the foreground first, so the window still comes to
//! the front as it did when Divora opened it. Where there is no desktop
//! Explorer (another shell, Explorer restarting), where Explorer can't open
//! it, or where its desktop doesn't answer within [`DESKTOP_WAIT`], this
//! fails, and the caller opens it the old way.

use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{mpsc, Arc};
use std::time::Duration;

use windows::core::{Interface, BSTR};
use windows::Win32::Foundation::S_OK;
use windows::Win32::System::Com::{
    CoAllowSetForegroundWindow, CoCreateInstance, CoInitializeEx, CoUninitialize, IDispatch,
    IServiceProvider, CLSCTX_ALL, COINIT_APARTMENTTHREADED,
};
use windows::Win32::System::Variant::VARIANT;
use windows::Win32::UI::Shell::{
    IShellBrowser, IShellDispatch2, IShellFolderViewDual, IShellView, IShellWindows,
    SID_STopLevelBrowser, ShellWindows, CSIDL_DESKTOP, SVGIO_BACKGROUND, SWC_DESKTOP,
    SWFO_NEEDDISPATCH,
};

/// How long to wait for the desktop. The calls go to Explorer's desktop
/// thread and have no timeout of their own; normally they take tens of
/// milliseconds, so a desktop this slow is stuck (a hung shell extension, an
/// unreachable network item), and the caller opens it the old way instead.
pub const DESKTOP_WAIT: Duration = Duration::from_secs(3);

/// Who decides a slow open: the worker, by handing it to Explorer, or the
/// waiter, by giving up. One compare-and-swap, so it can't open twice.
const PENDING: u8 = 0;
const COMMITTED: u8 = 1;
const ABANDONED: u8 = 2;

/// How the started program's first window shows (`SW_*`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Show {
    /// `SW_SHOWNORMAL`: a link's browser, a folder's window.
    Normal,
    /// `SW_HIDE`: a helper the user needn't see (the tests' tone player).
    Hidden,
}

impl Show {
    const fn code(self) -> i32 {
        match self {
            Self::Normal => 1,
            Self::Hidden => 0,
        }
    }
}

/// Open `file` (a URL, a folder or a program, with `args` for a program)
/// through the desktop's Explorer, so it doesn't start inside Divora's
/// process tree. Blocks for the cross-process calls (tens of milliseconds,
/// at most [`DESKTOP_WAIT`]); call it off the main thread. An error names
/// the step that failed, and means nothing was opened — unless the desktop
/// answered so late that the open was already Explorer's (then `Ok`).
pub fn open_outside_tree(file: &str, args: Option<&str>, show: Show) -> Result<(), String> {
    let file = file.to_owned();
    let args = args.map(str::to_owned);
    bounded(DESKTOP_WAIT, move |state| {
        // SAFETY: initializes COM on this new thread; balanced below only
        // when it succeeded.
        let entered = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }.is_ok();
        let result = shell_execute_in_explorer(&file, args.as_deref(), show, state);
        if entered {
            // SAFETY: pairs with the successful CoInitializeEx above, after
            // every interface from this apartment is dropped.
            unsafe { CoUninitialize() };
        }
        result
    })
}

/// Run `work` on a thread of its own and wait for it at most `wait`. The
/// shell's objects want a single-threaded apartment, and the caller's thread
/// may already be in the multithreaded one (a blocking-pool thread that ran
/// a capture job). `work` claims the open with [`commit`] just before the
/// call that opens it; a waiter that gives up first gets an error and the
/// worker's claim fails, so it opens nothing, while a worker that has
/// claimed it makes a late waiter answer `Ok`. Not joined: when the desktop
/// hangs, the thread is left to finish on its own.
fn bounded<F>(wait: Duration, work: F) -> Result<(), String>
where
    F: FnOnce(&AtomicU8) -> Result<(), String> + Send + 'static,
{
    let state = Arc::new(AtomicU8::new(PENDING));
    let (tx, rx) = mpsc::sync_channel(1);
    let worker_state = Arc::clone(&state);
    std::thread::Builder::new()
        .name("divora-open-outside".into())
        .spawn(move || {
            let result = work(&worker_state);
            // The waiter may have given up and gone.
            let _ = tx.send(result);
        })
        .map_err(|e| format!("couldn't start a thread to open it: {e}"))?;
    match rx.recv_timeout(wait) {
        Ok(result) => result,
        Err(mpsc::RecvTimeoutError::Disconnected) => Err("opening it panicked".to_string()),
        Err(mpsc::RecvTimeoutError::Timeout) => {
            match state.compare_exchange(PENDING, ABANDONED, Ordering::AcqRel, Ordering::Acquire) {
                // Given up before the worker reached Explorer: it stops
                // there, and the caller opens it the old way.
                Ok(_) => Err("the desktop isn't responding".to_string()),
                // Explorer already has it, and opens it when it can.
                Err(_) => Ok(()),
            }
        }
    }
}

fn shell_execute_in_explorer(
    file: &str,
    args: Option<&str>,
    show: Show,
    state: &AtomicU8,
) -> Result<(), String> {
    let step = |what: &'static str| move |e: windows::core::Error| format!("{what}: {e}");
    // SAFETY: plain COM calls on interfaces this function owns; every
    // pointer passed is to a live local, and the raw `ShellExecute` call
    // below passes its arguments as the generated wrapper does.
    unsafe {
        let windows: IShellWindows =
            CoCreateInstance(&ShellWindows, None, CLSCTX_ALL).map_err(step("no shell windows"))?;
        // CSIDL_DESKTOP is 0: the desktop's own folder.
        let desktop = VARIANT::from(i32::try_from(CSIDL_DESKTOP).unwrap_or(0));
        let empty = VARIANT::default();
        let mut hwnd = 0i32;
        let found: IDispatch = windows
            .FindWindowSW(
                &raw const desktop,
                &raw const empty,
                SWC_DESKTOP,
                &raw mut hwnd,
                SWFO_NEEDDISPATCH,
            )
            .map_err(step("no desktop window"))?;
        let services: IServiceProvider = found.cast().map_err(step("no service provider"))?;
        let browser: IShellBrowser = services
            .QueryService(&SID_STopLevelBrowser)
            .map_err(step("no shell browser"))?;
        let view: IShellView = browser
            .QueryActiveShellView()
            .map_err(step("no active shell view"))?;
        // The view hands its background object out as `IDispatch` only; the
        // folder-view interface is a query on that object.
        let background: IShellFolderViewDual = view
            .GetItemObject::<IDispatch>(SVGIO_BACKGROUND)
            .and_then(|d| d.cast())
            .map_err(step("no desktop view object"))?;
        let app: IShellDispatch2 = background
            .Application()
            .and_then(|a| a.cast())
            .map_err(step("no shell application"))?;
        // The window Explorer starts may take the foreground only if the
        // foreground process (Divora, whose button was just clicked) lets
        // Explorer pass it on. Refused when Divora isn't in front: then
        // there's nothing to pass, and it opens anyway.
        let _ = CoAllowSetForegroundWindow(&app, None);
        // From here the open is Explorer's: claim it, unless the waiter has
        // given up (and the caller is opening it the old way).
        if !commit(state) {
            return Err("gave up waiting for the desktop".to_string());
        }
        let file = BSTR::from(file);
        let args = args.map_or_else(VARIANT::default, VARIANT::from);
        let none = VARIANT::default();
        let show = VARIANT::from(show.code());
        // Called through the vtable: the shell answers S_FALSE when its own
        // ShellExecuteEx fails (after showing its error box), which the
        // generated wrapper's `.ok()` would count as success.
        let hr = (Interface::vtable(&app).ShellExecute)(
            Interface::as_raw(&app),
            core::mem::transmute_copy(&file),
            core::mem::transmute_copy(&args),
            core::mem::transmute_copy(&none),
            core::mem::transmute_copy(&none),
            core::mem::transmute_copy(&show),
        );
        if hr == S_OK {
            Ok(())
        } else {
            Err(format!("Explorer couldn't open it ({hr:?})"))
        }
    }
}

/// Claim the open for the worker; false when the waiter already gave up.
fn commit(state: &AtomicU8) -> bool {
    state
        .compare_exchange(PENDING, COMMITTED, Ordering::AcqRel, Ordering::Acquire)
        .is_ok()
}

#[cfg(test)]
mod tests {
    use super::{bounded, commit};
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    const WAIT: Duration = Duration::from_millis(200);

    #[test]
    fn a_quick_answer_is_the_answer() {
        assert_eq!(
            bounded(WAIT, |s| if commit(s) { Ok(()) } else { Err("no".into()) }),
            Ok(())
        );
        let refused = bounded(WAIT, |_| Err("Explorer couldn't open it".to_string()));
        assert_eq!(refused, Err("Explorer couldn't open it".to_string()));
    }

    /// A desktop stuck before the open: the waiter gives up in about `WAIT`,
    /// and the worker, when it gets there, opens nothing.
    #[test]
    fn a_stuck_desktop_is_given_up_and_never_opens_late() {
        let opened = Arc::new(AtomicU32::new(0));
        let done = Arc::new(AtomicU32::new(0));
        let (o, d) = (Arc::clone(&opened), Arc::clone(&done));
        let began = Instant::now();
        let r = bounded(WAIT, move |s| {
            std::thread::sleep(WAIT * 3);
            if commit(s) {
                o.fetch_add(1, Ordering::SeqCst);
            }
            d.fetch_add(1, Ordering::SeqCst);
            Ok(())
        });
        assert!(r.is_err(), "{r:?}");
        assert!(began.elapsed() < WAIT * 2, "waited {:?}", began.elapsed());
        while done.load(Ordering::SeqCst) == 0 {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(
            opened.load(Ordering::SeqCst),
            0,
            "it opened after the caller gave up"
        );
    }

    /// A desktop slow only in the open itself: the worker has claimed it, so
    /// the late waiter answers Ok and the caller doesn't open it a second
    /// time.
    #[test]
    fn a_claimed_open_answers_ok_when_slow() {
        let r = bounded(WAIT, |s| {
            assert!(commit(s));
            std::thread::sleep(WAIT * 3);
            Ok(())
        });
        assert_eq!(r, Ok(()));
    }

    #[test]
    fn a_worker_that_panics_is_an_error() {
        let r = bounded(WAIT, |_| panic!("boom"));
        assert!(r.is_err(), "{r:?}");
    }
}
