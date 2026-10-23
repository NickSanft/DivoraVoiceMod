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
//! Explorer's child and runs at Explorer's integrity level. Where there is no
//! desktop Explorer (another shell, Explorer restarting) this fails, and the
//! caller falls back to opening it the old way.

use windows::core::{Interface, BSTR};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoUninitialize, IDispatch, IServiceProvider, CLSCTX_ALL,
    COINIT_APARTMENTTHREADED,
};
use windows::Win32::System::Variant::VARIANT;
use windows::Win32::UI::Shell::{
    IShellBrowser, IShellDispatch2, IShellFolderViewDual, IShellView, IShellWindows,
    SID_STopLevelBrowser, ShellWindows, CSIDL_DESKTOP, SVGIO_BACKGROUND, SWC_DESKTOP,
    SWFO_NEEDDISPATCH,
};

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
/// process tree. Blocks for the cross-process call (tens of milliseconds);
/// call it off the main thread. Errors name the step that failed.
pub fn open_outside_tree(file: &str, args: Option<&str>, show: Show) -> Result<(), String> {
    let file = file.to_owned();
    let args = args.map(str::to_owned);
    // A thread of its own: the shell's objects want a single-threaded
    // apartment, and the caller's thread may already be in the
    // multithreaded one (a blocking-pool thread that ran a capture job).
    std::thread::Builder::new()
        .name("divora-open-outside".into())
        .spawn(move || {
            // SAFETY: initializes COM on this new thread; balanced below
            // only when it succeeded.
            let entered = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }.is_ok();
            let result = shell_execute_in_explorer(&file, args.as_deref(), show);
            if entered {
                // SAFETY: pairs with the successful CoInitializeEx above,
                // after every interface from this apartment is dropped.
                unsafe { CoUninitialize() };
            }
            result
        })
        .map_err(|e| format!("couldn't start a thread to open it: {e}"))?
        .join()
        .map_err(|_| "opening it panicked".to_string())?
}

fn shell_execute_in_explorer(file: &str, args: Option<&str>, show: Show) -> Result<(), String> {
    let step = |what: &'static str| move |e: windows::core::Error| format!("{what}: {e}");
    // SAFETY: plain COM calls on interfaces this function owns; every
    // pointer passed is to a live local.
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
        let args = args.map_or_else(VARIANT::default, VARIANT::from);
        app.ShellExecute(
            &BSTR::from(file),
            &args,
            &VARIANT::default(),
            &VARIANT::default(),
            &VARIANT::from(show.code()),
        )
        .map_err(step("Explorer couldn't open it"))
    }
}
