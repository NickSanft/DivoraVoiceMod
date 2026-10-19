//! v1.55.0: the stream overlay's shell — the glue that runs
//! [`OverlayCore`]'s effects on the real window: building, placing, showing
//! and closing it, its click-through and capture protection, the page's
//! handshake and beats, the hotkey, the watchdog, the lease, and the
//! recorder's capture hooks.
//!
//! Threads and locks. `core` is a leaf: while holding it, code only
//! computes, emits, and (off the main thread) posts. Window calls run on the
//! main thread — posted as one closure, or inline when already there, with
//! `core` released first, because an inline setter can send a window event
//! straight back into [`on_window_event`], which takes `core`. Getters
//! (`hwnd`, `outer_position`, `inner_size`) run only on worker threads,
//! never under `core` or the recorder's lock: `open` and `place` call them
//! under the placement lock, which the main thread never takes. Main-thread
//! code reads the stored HWND. The page's handshake and beats are commands,
//! never event listeners: tauri parks Rust event listeners on lock
//! contention (event/listener.rs:137-143, 196-213).
//!
//! Focus is taken with plain `SetForegroundWindow`, never tauri's or tao's
//! focus call, which injects an Alt key press when Windows refuses
//! (`the_overlay_never_calls_set_focus`).

use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
use std::sync::{mpsc, Mutex, MutexGuard, OnceLock, PoisonError};
use std::thread::{self, ThreadId};
use std::time::{Duration, Instant};

use tauri::{
    AppHandle, Emitter, Manager, PhysicalPosition, PhysicalSize, State, WebviewUrl, WebviewWindow,
    WebviewWindowBuilder, WindowEvent,
};

use crate::overlay_machine::{
    affinity_call, hotkey_overlay_request, placement_settled, should_return_focus, Effect,
    HandshakeStage, Input, Mode, ModeRequest, OverlayCore, OverlayRect, OverlayStatus, Phase,
    Refusal, Setter, CAPTURE_ACK, CLOSING, LEASE_TICK, NOT_OPEN, NO_READY_GLOBAL, OVERLAY_LABEL,
    OVERLAY_STATUS_EVENT, OVERLAY_TITLE, PRETEND_NO_READY_ENV, R1_RUNG, STARTING, WATCHDOG,
};

/// How long `open` waits for a closing window to go before it gives up.
const CLOSE_WAIT: Duration = Duration::from_secs(2);
const CLOSE_POLL: Duration = Duration::from_millis(50);

/// The overlay's state, shared by the commands, the window events, the
/// watchdog, the lease, the hotkey and the recorder.
pub struct OverlayShared {
    /// The machine. A leaf lock (see the module comment).
    core: Mutex<OverlayCore>,
    /// The live window's HWND as an integer; 0 when none.
    hwnd: AtomicIsize,
    /// The core is in Arrange, for the tray menu, which mustn't take `core`.
    arranging: AtomicBool,
    /// The foreground window before Arrange took it; 0 when none.
    return_to: AtomicIsize,
    /// The live window has been shown. Main thread only while there is a
    /// window; `open` clears it on its worker when there is none (`adopt`
    /// before the build, `forget` if the build fails).
    shown: AtomicBool,
    /// The display affinity last set on the live window. The same threads
    /// as `shown`.
    applied_protect: AtomicBool,
    /// True from just before the window is built until `CloseRequested` or
    /// `DestroyWindow`; the `charm` scheme answers 503 otherwise.
    overlay_live: AtomicBool,
    /// Held by `open` from build to `Built`, and by `place` from placing to
    /// `Placed`, so two placements never interleave.
    placement: tauri::async_runtime::Mutex<()>,
}

impl OverlayShared {
    fn new(hide_available: bool) -> Self {
        Self {
            core: Mutex::new(OverlayCore::new(hide_available)),
            hwnd: AtomicIsize::new(0),
            arranging: AtomicBool::new(false),
            return_to: AtomicIsize::new(0),
            shown: AtomicBool::new(false),
            applied_protect: AtomicBool::new(false),
            overlay_live: AtomicBool::new(false),
            placement: tauri::async_runtime::Mutex::new(()),
        }
    }

    fn core(&self) -> MutexGuard<'_, OverlayCore> {
        self.core.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// The thread the event loop runs on, set at setup.
static MAIN_THREAD: OnceLock<ThreadId> = OnceLock::new();

fn on_main_thread() -> bool {
    MAIN_THREAD.get() == Some(&thread::current().id())
}

fn shared(app: &AppHandle) -> Option<State<'_, OverlayShared>> {
    app.try_state::<OverlayShared>()
}

/// Proof that [`before_capture`] ran. Only it makes one, and the recorder's
/// `open_feed` takes one, so a capture can't start without the overlay
/// being asked to hide first.
#[must_use]
pub(crate) struct CaptureCleared(());

/// At setup, on the main thread.
pub fn init(app: &tauri::App) {
    let _ = MAIN_THREAD.set(thread::current().id());
    let hide_available = divora_capture::capture_exclusion_available();
    tracing::info!(hide_available, "overlay shell ready");
    app.manage(OverlayShared::new(hide_available));
}

/// `open_overlay`: build the window at `rect` (Windows' default spot when
/// `None`) and show it once its page is ready.
pub async fn open(
    app: AppHandle,
    rect: Option<OverlayRect>,
    fit: bool,
) -> Result<OverlayStatus, String> {
    let (status, effects) = match step_effects(&app, Input::Open { fit }) {
        Err(Refusal::Closing) => {
            // The last window is still going: wait for it, then try once more.
            let h = app.clone();
            let closed = tauri::async_runtime::spawn_blocking(move || {
                shared(&h).is_some_and(|s| wait_closed(&s, CLOSE_WAIT, CLOSE_POLL))
            })
            .await
            .unwrap_or(false);
            if !closed {
                return Err(CLOSING.to_owned());
            }
            step_effects(&app, Input::Open { fit }).map_err(|r| r.message().to_owned())?
        }
        other => other.map_err(|r| r.message().to_owned())?,
    };
    let build = effects.iter().find_map(|e| match *e {
        Effect::Build { instance } => Some(instance),
        _ => None,
    });
    match build {
        Some(instance) => build_window(&app, instance, rect, fit).await,
        // Open, or still opening: nothing to build.
        None => Ok(status),
    }
}

/// Build window `instance` hidden and click-through, place it, and step
/// `Built`. Holds the placement lock throughout; never `core` across a
/// window call.
async fn build_window(
    app: &AppHandle,
    instance: u32,
    rect: Option<OverlayRect>,
    fit: bool,
) -> Result<OverlayStatus, String> {
    let Some(shared) = shared(app) else {
        return Err(NOT_OPEN.to_owned());
    };
    let placing = shared.placement.lock().await;
    let url = WebviewUrl::App(overlay_url(instance).into());
    let mut builder = WebviewWindowBuilder::new(app, OVERLAY_LABEL, url)
        .title(OVERLAY_TITLE)
        .inner_size(480.0, 480.0)
        .min_inner_size(64.0, 48.0)
        .transparent(true)
        .decorations(false)
        // An undecorated transparent window keeps a 1 px border unless the
        // shadow is off (tauri #8308).
        .shadow(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .resizable(!fit)
        .maximizable(false)
        .minimizable(false)
        // Hidden until the page is ready, and it never takes focus on open.
        .visible(false)
        .focused(false);
    let pretend = cfg!(debug_assertions) && std::env::var_os(PRETEND_NO_READY_ENV).is_some();
    if let Some(script) = no_ready_script(pretend) {
        tracing::warn!("{PRETEND_NO_READY_ENV} is set: the overlay page won't say it is ready");
        builder = builder.initialization_script(script);
    }
    // Fresh cells, live before the page can load: its pictures may be asked
    // for before this thread is back from the round trips below.
    adopt(&shared, 0);
    let window = match builder.build() {
        Ok(w) => w,
        Err(e) => {
            forget(&shared);
            let _ = step(app, Input::BuildFailed { instance });
            return Err(format!("Couldn't open the overlay window: {e}"));
        }
    };
    // Click-through before anything can show it. Posted; the getter below
    // waits behind it.
    let _ = window.set_ignore_cursor_events(true);
    let hwnd = window_id(&window);
    shared.hwnd.store(hwnd, Ordering::SeqCst);
    let _ = app.run_on_main_thread(move || {
        if !divora_capture::mark_non_rude(hwnd) {
            tracing::warn!("couldn't mark the overlay window NonRudeHWND");
        }
    });
    if let Some(rect) = rect.and_then(OverlayRect::sane) {
        place_window(&window, rect);
    }
    let status = step(app, Input::Built { instance }).map_err(|r| r.message().to_owned());
    drop(placing);
    status
}

/// `overlay.html?instance=<n>`: the page echoes `n` in its handshake and
/// beats, so a stale call from an earlier window never counts.
fn overlay_url(instance: u32) -> String {
    format!("overlay.html?instance={instance}")
}

/// The init script that makes the page skip its ready handshake, when
/// [`PRETEND_NO_READY_ENV`] is set in a debug build.
fn no_ready_script(pretend: bool) -> Option<String> {
    pretend.then(|| format!("window.{NO_READY_GLOBAL} = true;"))
}

/// The window's HWND as an integer. A main-thread round trip: worker
/// threads only, never under `core` (`build_window` calls it under the
/// placement lock, which the main thread never takes).
fn window_id(window: &WebviewWindow) -> isize {
    #[cfg(windows)]
    {
        window.hwnd().map_or(0, |h| h.0 as isize)
    }
    #[cfg(not(windows))]
    {
        let _ = window;
        0
    }
}

/// The cells for a new window: its HWND, nothing shown, protected or
/// remembered yet, and live for the `charm` scheme.
fn adopt(shared: &OverlayShared, hwnd: isize) {
    shared.hwnd.store(hwnd, Ordering::SeqCst);
    shared.shown.store(false, Ordering::SeqCst);
    shared.applied_protect.store(false, Ordering::SeqCst);
    shared.return_to.store(0, Ordering::SeqCst);
    shared.overlay_live.store(true, Ordering::SeqCst);
}

/// The window is going (closing, being destroyed, or the app quitting): the
/// `charm` scheme stops serving it, and the shell's mark comes off.
fn going(shared: &OverlayShared, n: &impl Native) {
    shared.overlay_live.store(false, Ordering::SeqCst);
    n.unmark_non_rude(shared.hwnd.load(Ordering::SeqCst));
}

/// The window is gone: no HWND, and nothing shown, protected or remembered.
fn forget(shared: &OverlayShared) {
    shared.hwnd.store(0, Ordering::SeqCst);
    shared.return_to.store(0, Ordering::SeqCst);
    shared.shown.store(false, Ordering::SeqCst);
    shared.applied_protect.store(false, Ordering::SeqCst);
    shared.overlay_live.store(false, Ordering::SeqCst);
}

/// Whether the core stepped back to Closed within `within`.
fn wait_closed(shared: &OverlayShared, within: Duration, poll: Duration) -> bool {
    let until = Instant::now() + within;
    loop {
        if shared.core().phase == Phase::Closed {
            return true;
        }
        if Instant::now() >= until {
            return false;
        }
        thread::sleep(poll);
    }
}

/// `close_overlay`.
pub fn close(app: &AppHandle) -> OverlayStatus {
    step(app, Input::Close).unwrap_or_else(|_| status(app))
}

/// `place_overlay`: move and size the open window.
pub async fn place(app: AppHandle, rect: OverlayRect, fit: bool) -> Result<OverlayStatus, String> {
    let Some(shared) = shared(&app) else {
        return Err(NOT_OPEN.to_owned());
    };
    // Before the window is touched: one being built is `open`'s to place.
    let phase = shared.core().phase;
    placeable(phase)?;
    let Some(rect) = rect.sane() else {
        // Main's rectangles always have a usable size; nothing moves.
        tracing::warn!(?rect, "overlay placement ignored: no usable size");
        return Ok(status(&app));
    };
    let placing = shared.placement.lock().await;
    if let Some(window) = app.get_webview_window(OVERLAY_LABEL) {
        place_window(&window, rect);
    }
    let status = step(&app, Input::Placed { fit }).map_err(|r| r.message().to_owned());
    drop(placing);
    status
}

/// Whether `place_overlay` may touch the window in `phase`, or why not.
fn placeable(phase: Phase) -> Result<(), String> {
    match phase {
        Phase::Starting { .. } | Phase::Shown { .. } | Phase::Painted { .. } => Ok(()),
        Phase::Creating { .. } => Err(STARTING.to_owned()),
        Phase::Closed | Phase::Closing { .. } => Err(NOT_OPEN.to_owned()),
    }
}

/// Size, move, read back; at most three passes. Windows rescales a window
/// (`WM_DPICHANGED`) when most of it crosses onto a monitor with another
/// scale, on the resize as well as the move, so a second pass puts it right.
/// Worker threads only: the reads are main-thread round trips.
fn place_window(window: &WebviewWindow, rect: OverlayRect) {
    let size = PhysicalSize::new(rect.width, rect.height);
    let pos = PhysicalPosition::new(rect.x, rect.y);
    for _ in 0..3 {
        let _ = window.set_size(size);
        let _ = window.set_position(pos);
        let (Ok(p), Ok(s)) = (window.outer_position(), window.inner_size()) else {
            return;
        };
        if placement_settled(rect, (p.x, p.y), (s.width, s.height)) {
            return;
        }
    }
}

/// `set_overlay_mode`, the tray's Lock and the hotkey.
pub fn set_mode(app: &AppHandle, req: ModeRequest) -> Result<OverlayStatus, String> {
    step(app, Input::Mode(req)).map_err(|r| r.message().to_owned())
}

/// `overlay_handshake`, from the overlay page. A command rather than a
/// listener: tauri parks Rust event listeners on lock contention
/// (event/listener.rs:137-143, 196-213); keep this on commands.
pub fn handshake(app: &AppHandle, instance: u32, stage: HandshakeStage) -> OverlayStatus {
    let input = match stage {
        HandshakeStage::Ready => Input::Ready { instance },
        HandshakeStage::Painted => Input::Painted { instance },
    };
    step(app, input).unwrap_or_else(|_| status(app))
}

/// `overlay_alive`, the page's beat while arranging. It only steps: a beat
/// emits nothing. A command for the same reason as [`handshake`]: tauri
/// parks Rust event listeners on lock contention (event/listener.rs:137-143,
/// 196-213); keep this on commands.
pub fn alive(app: &AppHandle, instance: u32, gen: u32) {
    let _ = step(app, Input::Alive { instance, gen });
}

/// A global shortcut's press or release, from `global_shortcut_handler`
/// (the main thread, inside `WM_HOTKEY`, so the change and its focus run
/// inline while the hotkey's right to the foreground is fresh). Never
/// registers or unregisters a shortcut: the plugin may hold its store's lock.
pub fn hotkey(app: &AppHandle, id: &str, pressed: bool) {
    if let Some(req) = hotkey_overlay_request(id, pressed) {
        if let Err(e) = set_mode(app, req) {
            tracing::debug!(error = %e, "overlay hotkey refused");
        }
    }
}

/// `set_overlay_hide_from_recordings`.
pub fn set_hide(app: &AppHandle, on: bool) -> OverlayStatus {
    step(app, Input::HideWanted(on)).unwrap_or_else(|_| status(app))
}

/// `overlay_status`.
pub fn status(app: &AppHandle) -> OverlayStatus {
    shared(app).map_or_else(closed, |s| s.core().status())
}

/// The overlay window's events (the caller checks the label). Main thread.
pub fn on_window_event(app: &AppHandle, event: &WindowEvent) {
    let Some(shared) = shared(app) else {
        return;
    };
    match event {
        WindowEvent::CloseRequested { .. } => {
            going(&shared, &Desktop(None));
            let _ = step(app, Input::CloseRequested);
        }
        WindowEvent::Destroyed => {
            forget(&shared);
            let _ = step(app, Input::Destroyed);
        }
        _ => {}
    }
}

/// Before a take or Replay starts capturing: apply protection if it is
/// due, waiting at most `CAPTURE_ACK`. Never under the recorder's lock.
pub(crate) fn before_capture(app: &AppHandle) -> CaptureCleared {
    let _ = step(app, Input::Capturing(true));
    // Not due: the toggle is off or unavailable, the overlay is arranging,
    // or there is no shown window (a hidden one isn't captured).
    if !shared(app).is_some_and(|s| s.core().protected()) {
        return CaptureCleared(());
    }
    if on_main_thread() {
        reconcile(app);
        return CaptureCleared(());
    }
    // Runs after anything posted before it, the step above included. Before
    // the window is shown it leaves the affinity alone; the Show closure
    // protects it before showing it.
    let (tx, rx) = mpsc::channel();
    let h = app.clone();
    let posted = app.run_on_main_thread(move || {
        reconcile(&h);
        let _ = tx.send(());
    });
    if posted.is_err() || rx.recv_timeout(CAPTURE_ACK).is_err() {
        tracing::warn!("overlay protection not confirmed before capture");
    }
    CaptureCleared(())
}

/// A take started or ended, or Replay changed. May be called under the
/// recorder's lock: `core` is a leaf, and the window work is posted (or, on
/// the main thread, runs setters that never take the recorder's lock).
pub fn capture_changed(app: &AppHandle, active: bool) {
    let _ = step(app, Input::Capturing(active));
}

/// The overlay is in Arrange, for the tray menu.
pub fn arranging(app: &AppHandle) -> bool {
    shared(app).is_some_and(|s| s.arranging.load(Ordering::SeqCst))
}

/// The overlay window is open and not closing, for the `charm` scheme.
pub fn overlay_live(app: &AppHandle) -> bool {
    shared(app).is_some_and(|s| s.overlay_live.load(Ordering::SeqCst))
}

/// From the tray's Quit, before the app exits: the shell's mark comes off
/// (the window isn't closed through `CloseRequested` then).
pub fn before_exit(app: &AppHandle) {
    if let Some(shared) = shared(app) {
        going(&shared, &Desktop(None));
    }
}

fn closed() -> OverlayStatus {
    OverlayCore::new(false).status()
}

/// Step the core with `input`, emit, and run the effects.
fn step(app: &AppHandle, input: Input) -> Result<OverlayStatus, Refusal> {
    step_effects(app, input).map(|(status, _)| status)
}

/// [`step`], also handing back the effects (`open` builds the window
/// itself).
fn step_effects(app: &AppHandle, input: Input) -> Result<(OverlayStatus, Vec<Effect>), Refusal> {
    let Some(shared) = shared(app) else {
        return Err(Refusal::NotOpen);
    };
    let on_main = on_main_thread();
    let mut core = shared.core();
    let effects = core.step(input)?;
    let status = core.status();
    if effects.contains(&Effect::Emit) {
        // Under `core`, so statuses go out in the order the state changed,
        // as the recorder's `publish` does.
        let _ = app.emit(OVERLAY_STATUS_EVENT, &status);
    }
    shared
        .arranging
        .store(core.mode == Mode::Arrange, Ordering::SeqCst);
    if on_main {
        // The window calls run inline, and may come straight back here.
        drop(core);
        dispatch(app, &effects, true);
    } else {
        // Posting doesn't wait, and under `core` keeps the order across
        // threads.
        dispatch(app, &effects, false);
        drop(core);
    }
    Ok((status, effects))
}

/// Start the effects' threads, and run their window work on the main
/// thread: inline when already there, else posted as one closure (the
/// event loop runs closures in the order posted).
fn dispatch(app: &AppHandle, effects: &[Effect], on_main: bool) {
    for effect in effects {
        match *effect {
            Effect::ArmWatchdog { instance } => arm_watchdog(app, instance),
            Effect::ArmLease { instance, gen } => arm_lease(app, instance, gen),
            _ => {}
        }
    }
    let work = main_thread_effects(effects);
    if work.is_empty() {
        return;
    }
    let h = app.clone();
    let run = move || run_on_main(&h, &work);
    if on_main {
        run();
    } else if app.run_on_main_thread(run).is_err() {
        tracing::debug!("overlay effects not run: the app is closing");
    }
}

/// The effects that are window or tray work, in order. `Build` is `open`'s,
/// `Emit` went out in `step`, and the two timers are threads.
fn main_thread_effects(effects: &[Effect]) -> Vec<Effect> {
    effects
        .iter()
        .copied()
        .filter(|e| {
            !matches!(
                e,
                Effect::Build { .. }
                    | Effect::Emit
                    | Effect::ArmWatchdog { .. }
                    | Effect::ArmLease { .. }
            )
        })
        .collect()
}

/// Main thread, `core` released.
fn run_on_main(app: &AppHandle, effects: &[Effect]) {
    let Some(shared) = shared(app) else {
        return;
    };
    let desktop = Desktop::of(app);
    for &effect in effects {
        if effect == Effect::RefreshTray {
            crate::refresh_tray_menu(app);
        } else {
            window_effect(&shared, effect, &desktop);
        }
    }
}

/// One window effect, on the main thread with `core` released.
fn window_effect(shared: &OverlayShared, effect: Effect, n: &impl Native) {
    match effect {
        Effect::ApplyWindow => {
            reconcile_on(shared, n);
        }
        Effect::Show => show(shared, n),
        Effect::Focus => focus(shared, n),
        Effect::ReturnFocus => return_focus(shared, n),
        Effect::CloseWindow => n.close(),
        Effect::DestroyWindow => {
            going(shared, n);
            n.destroy();
        }
        Effect::Build { .. }
        | Effect::ArmWatchdog { .. }
        | Effect::ArmLease { .. }
        | Effect::Emit
        | Effect::RefreshTray => {}
    }
}

/// Bring the live window to the core's state now. Main thread only.
fn reconcile(app: &AppHandle) {
    if let Some(shared) = shared(app) {
        reconcile_on(&shared, &Desktop::of(app));
    }
}

/// Apply the latest window state, if the window is live; whether it was.
/// A closure that runs late reads the state as it is then, so it never
/// lowers protection on a closing window.
fn reconcile_on(shared: &OverlayShared, n: &impl Native) -> bool {
    let plan = shared.core().reconcile_plan(R1_RUNG);
    let Some(plan) = plan else {
        return false;
    };
    apply_setters(shared, &plan, n);
    true
}

/// Run `plan` in its order. The affinity reaches only a shown window, and
/// only when it changes (`affinity_call`); tao skips the flags that didn't
/// change, so a repeat apply makes no Win32 call.
fn apply_setters(shared: &OverlayShared, plan: &[Setter], n: &impl Native) {
    let hwnd = shared.hwnd.load(Ordering::SeqCst);
    for setter in plan {
        match *setter {
            Setter::IgnoreCursor(on) => n.set_ignore_cursor(on),
            Setter::LayerOpaque => n.make_layered_opaque(hwnd),
            Setter::Resizable(on) => n.set_resizable(on),
            Setter::Protect(want) => {
                let shown = shared.shown.load(Ordering::SeqCst);
                let applied = shared.applied_protect.load(Ordering::SeqCst);
                if let Some(on) = affinity_call(want, shown, applied) {
                    n.set_protected(on, hwnd);
                    shared.applied_protect.store(on, Ordering::SeqCst);
                }
            }
        }
    }
}

/// Apply the window state, then show it, back to back: no message pump in
/// between, so protection (when due) lands before the first frame is
/// composed. A window no longer live isn't shown.
fn show(shared: &OverlayShared, n: &impl Native) {
    shared.shown.store(true, Ordering::SeqCst);
    if reconcile_on(shared, n) {
        n.show();
    } else {
        shared.shown.store(false, Ordering::SeqCst);
    }
}

/// Take the foreground for Arrange, remembering who had it (unless it was
/// the overlay already).
fn focus(shared: &OverlayShared, n: &impl Native) {
    let ours = shared.hwnd.load(Ordering::SeqCst);
    let fg = n.foreground();
    if fg != 0 && fg != ours {
        shared.return_to.store(fg, Ordering::SeqCst);
    }
    let ok = n.try_foreground(ours);
    tracing::info!(ok, "overlay focus");
}

/// On Lock: if the overlay still has the foreground, give it back to the
/// window that had it before Arrange.
fn return_focus(shared: &OverlayShared, n: &impl Native) {
    let ours = shared.hwnd.load(Ordering::SeqCst);
    let saved = shared.return_to.swap(0, Ordering::SeqCst);
    if should_return_focus(n.foreground(), ours, saved, n.can_take_foreground(saved)) {
        let ok = n.try_foreground(saved);
        tracing::debug!(ok, "overlay focus handed back");
    }
}

/// `Watchdog(instance)` once `WATCHDOG` has passed.
fn arm_watchdog(app: &AppHandle, instance: u32) {
    let h = app.clone();
    let spawned = thread::Builder::new()
        .name("divora-overlay-watchdog".into())
        .spawn(move || {
            thread::sleep(WATCHDOG);
            let _ = step(&h, Input::Watchdog { instance });
        });
    if let Err(e) = spawned {
        tracing::warn!(error = %e, "couldn't start the overlay watchdog");
    }
}

/// A `LeaseTick` every `LEASE_TICK` while this Arrange lasts.
fn arm_lease(app: &AppHandle, instance: u32, gen: u32) {
    let h = app.clone();
    let spawned = thread::Builder::new()
        .name("divora-overlay-lease".into())
        .spawn(move || loop {
            thread::sleep(LEASE_TICK);
            let _ = step(&h, Input::LeaseTick { instance, gen });
            if !shared(&h).is_some_and(|s| s.core().in_lease(instance, gen)) {
                break;
            }
        });
    if let Err(e) = spawned {
        tracing::warn!(error = %e, "couldn't start the overlay lease");
    }
}

/// The calls the effects make on the window and the desktop. A trait so
/// the order the shell makes them in is tested without a window.
trait Native {
    fn set_ignore_cursor(&self, on: bool);
    fn make_layered_opaque(&self, hwnd: isize);
    fn set_resizable(&self, on: bool);
    fn set_protected(&self, on: bool, hwnd: isize);
    fn show(&self);
    fn close(&self);
    fn destroy(&self);
    fn foreground(&self) -> isize;
    fn try_foreground(&self, hwnd: isize) -> bool;
    fn can_take_foreground(&self, hwnd: isize) -> bool;
    fn unmark_non_rude(&self, hwnd: isize);
}

/// The real overlay window, if it still exists, and the desktop through
/// divora-capture.
struct Desktop(Option<WebviewWindow>);

impl Desktop {
    fn of(app: &AppHandle) -> Self {
        Self(app.get_webview_window(OVERLAY_LABEL))
    }
}

impl Native for Desktop {
    fn set_ignore_cursor(&self, on: bool) {
        if let Some(w) = &self.0 {
            let _ = w.set_ignore_cursor_events(on);
        }
    }

    fn make_layered_opaque(&self, hwnd: isize) {
        if !divora_capture::make_layered_opaque(hwnd) {
            tracing::debug!("overlay layered attributes refused");
        }
    }

    fn set_resizable(&self, on: bool) {
        if let Some(w) = &self.0 {
            let _ = w.set_resizable(on);
        }
    }

    fn set_protected(&self, on: bool, hwnd: isize) {
        if let Some(w) = &self.0 {
            let _ = w.set_content_protected(on);
        }
        // Windows doesn't say whether capture honours it; this is for the log.
        let affinity = divora_capture::display_affinity(hwnd);
        tracing::debug!(on, ?affinity, "overlay display affinity");
    }

    fn show(&self) {
        if let Some(w) = &self.0 {
            let _ = w.show();
        }
    }

    fn close(&self) {
        if let Some(w) = &self.0 {
            let _ = w.close();
        }
    }

    fn destroy(&self) {
        if let Some(w) = &self.0 {
            let _ = w.destroy();
        }
    }

    fn foreground(&self) -> isize {
        divora_capture::foreground_window()
    }

    fn try_foreground(&self, hwnd: isize) -> bool {
        divora_capture::try_foreground(hwnd)
    }

    fn can_take_foreground(&self, hwnd: isize) -> bool {
        divora_capture::can_take_foreground(hwnd)
    }

    fn unmark_non_rude(&self, hwnd: isize) {
        divora_capture::unmark_non_rude(hwnd);
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::sync::atomic::Ordering;
    use std::time::Duration;

    use super::{
        adopt, apply_setters, forget, going, main_thread_effects, no_ready_script, overlay_url,
        placeable, return_focus, show, wait_closed, window_effect, Native, OverlayShared,
    };
    use crate::overlay_machine::{
        affinity_call, should_return_focus, Effect, Phase, Setter, NOT_OPEN, R1_RUNG, STARTING,
    };

    const OURS: isize = 7;
    const GAME: isize = 42;

    /// Records the shell's calls, in order.
    #[derive(Default)]
    struct Fake {
        calls: RefCell<Vec<String>>,
        foreground: isize,
        can_take: bool,
    }

    impl Fake {
        fn log(&self, call: String) {
            self.calls.borrow_mut().push(call);
        }

        fn calls(&self) -> Vec<String> {
            self.calls.borrow().clone()
        }
    }

    impl Native for Fake {
        fn set_ignore_cursor(&self, on: bool) {
            self.log(format!("ignore {on}"));
        }
        fn make_layered_opaque(&self, hwnd: isize) {
            self.log(format!("layered {hwnd}"));
        }
        fn set_resizable(&self, on: bool) {
            self.log(format!("resizable {on}"));
        }
        fn set_protected(&self, on: bool, hwnd: isize) {
            self.log(format!("protect {on} {hwnd}"));
        }
        fn show(&self) {
            self.log("show".into());
        }
        fn close(&self) {
            self.log("close".into());
        }
        fn destroy(&self) {
            self.log("destroy".into());
        }
        fn foreground(&self) -> isize {
            self.foreground
        }
        fn try_foreground(&self, hwnd: isize) -> bool {
            self.log(format!("foreground {hwnd}"));
            true
        }
        fn can_take_foreground(&self, _hwnd: isize) -> bool {
            self.can_take
        }
        fn unmark_non_rude(&self, hwnd: isize) {
            self.log(format!("unmark {hwnd}"));
        }
    }

    fn with_window(phase: Phase) -> OverlayShared {
        let shared = OverlayShared::new(true);
        adopt(&shared, OURS);
        shared.core().phase = phase;
        shared
    }

    /// A source scan, the honest limit without a window: tauri's and tao's
    /// focus call injects an Alt key press when Windows refuses, so the shell
    /// must never make it. The needles are built from halves so this test
    /// doesn't find itself, and are calls (a method or a path), so its own
    /// name doesn't either.
    #[test]
    fn the_overlay_never_calls_set_focus() {
        let src = include_str!("overlay_windows.rs");
        for needle in [concat!(".set_", "focus("), concat!("::set_", "focus(")] {
            assert!(
                !src.contains(needle),
                "the overlay takes the foreground with try_foreground, never {needle}"
            );
        }
    }

    /// The window and tray work keeps its order; the window is built by
    /// `open`, the status went out in `step`, and the timers are threads.
    #[test]
    fn main_thread_effects_keep_order_and_drop_the_rest() {
        let effects = [
            Effect::Build { instance: 1 },
            Effect::ApplyWindow,
            Effect::ArmWatchdog { instance: 1 },
            Effect::Emit,
            Effect::Show,
            Effect::Focus,
            Effect::ArmLease {
                instance: 1,
                gen: 2,
            },
            Effect::RefreshTray,
            Effect::ReturnFocus,
            Effect::CloseWindow,
            Effect::DestroyWindow,
        ];
        assert_eq!(
            main_thread_effects(&effects),
            vec![
                Effect::ApplyWindow,
                Effect::Show,
                Effect::Focus,
                Effect::RefreshTray,
                Effect::ReturnFocus,
                Effect::CloseWindow,
                Effect::DestroyWindow,
            ]
        );
        assert_eq!(
            main_thread_effects(&[Effect::Emit, Effect::Build { instance: 3 }]),
            []
        );
    }

    /// Setters run in the plan's order, on the stored HWND. (A hidden,
    /// never-protected window gets no affinity call either way.)
    #[test]
    fn setters_run_in_the_plan_order() {
        let shared = with_window(Phase::Starting { instance: 1 });
        let locked = [
            Setter::IgnoreCursor(true),
            Setter::LayerOpaque,
            Setter::Resizable(false),
            Setter::Protect(false),
        ];
        let fake = Fake::default();
        apply_setters(&shared, &locked, &fake);
        assert_eq!(
            fake.calls(),
            ["ignore true", "layered 7", "resizable false"]
        );

        let arrange = [
            Setter::Protect(false),
            Setter::IgnoreCursor(false),
            Setter::Resizable(true),
        ];
        let fake = Fake::default();
        apply_setters(&shared, &arrange, &fake);
        assert_eq!(fake.calls(), ["ignore false", "resizable true"]);
    }

    /// The affinity goes through `affinity_call` with the window's own
    /// cells, and the cell remembers what was set.
    #[test]
    fn protect_goes_through_affinity_call() {
        for want in [false, true] {
            for shown in [false, true] {
                for applied in [false, true] {
                    let shared = with_window(Phase::Shown { instance: 1 });
                    shared.shown.store(shown, Ordering::SeqCst);
                    shared.applied_protect.store(applied, Ordering::SeqCst);
                    let fake = Fake::default();
                    apply_setters(&shared, &[Setter::Protect(want)], &fake);
                    let call = affinity_call(want, shown, applied);
                    let expected: Vec<String> = call
                        .map(|on| format!("protect {on} 7"))
                        .into_iter()
                        .collect();
                    let case = format!("want {want} shown {shown} applied {applied}");
                    assert_eq!(fake.calls(), expected, "{case}");
                    assert_eq!(
                        shared.applied_protect.load(Ordering::SeqCst),
                        call.unwrap_or(applied),
                        "{case}"
                    );
                }
            }
        }
    }

    /// Show applies the state and shows only a live window; a window that
    /// closed before the posted Show ran is never shown.
    #[test]
    fn show_only_while_live() {
        for phase in [
            Phase::Closed,
            Phase::Creating {
                instance: 1,
                ready_early: true,
                cancel: false,
            },
            Phase::Closing { instance: 1 },
        ] {
            let shared = with_window(phase);
            let fake = Fake::default();
            show(&shared, &fake);
            assert_eq!(fake.calls(), Vec::<String>::new(), "{phase:?}");
            assert!(!shared.shown.load(Ordering::SeqCst), "{phase:?}");
        }
        // Live: shown last, after whatever the plan applies.
        let shared = with_window(Phase::Shown { instance: 1 });
        let live = shared.core().reconcile_plan(R1_RUNG).is_some();
        let fake = Fake::default();
        show(&shared, &fake);
        assert_eq!(
            fake.calls().last().map(String::as_str) == Some("show"),
            live
        );
        assert_eq!(shared.shown.load(Ordering::SeqCst), live);
    }

    /// Opened while a capture runs with Hide on: the affinity goes on before
    /// the window shows, so no frame of it is captured.
    #[test]
    fn show_protects_before_it_shows() {
        let shared = with_window(Phase::Shown { instance: 1 });
        {
            let mut c = shared.core();
            c.hide_wanted = true;
            c.capturing = true;
        }
        let fake = Fake::default();
        show(&shared, &fake);
        let calls = fake.calls();
        let protect = calls.iter().position(|c| c == "protect true 7");
        let shown = calls.iter().position(|c| c == "show");
        assert!(
            matches!((protect, shown), (Some(p), Some(s)) if p < s),
            "{calls:?}"
        );
        assert_eq!(shown, Some(calls.len() - 1), "{calls:?}");
        assert!(shared.applied_protect.load(Ordering::SeqCst));
        assert!(shared.shown.load(Ordering::SeqCst));
    }

    /// Focus remembers who had the foreground, unless it was nobody or the
    /// overlay itself, and takes it for the overlay.
    #[test]
    fn focus_remembers_who_had_the_foreground() {
        for (fg, kept) in [(GAME, GAME), (OURS, 0), (0, 0)] {
            let shared = with_window(Phase::Painted { instance: 1 });
            let fake = Fake {
                foreground: fg,
                ..Fake::default()
            };
            window_effect(&shared, Effect::Focus, &fake);
            assert_eq!(shared.return_to.load(Ordering::SeqCst), kept, "fg {fg}");
            assert_eq!(fake.calls(), ["foreground 7"], "fg {fg}");
        }
    }

    /// Lock hands the foreground back exactly when `should_return_focus`
    /// says, and forgets the saved window either way.
    #[test]
    fn return_focus_follows_should_return_focus() {
        for fg in [OURS, GAME, 0] {
            for saved in [GAME, OURS, 0] {
                for can_take in [false, true] {
                    let shared = with_window(Phase::Painted { instance: 1 });
                    shared.return_to.store(saved, Ordering::SeqCst);
                    let fake = Fake {
                        foreground: fg,
                        can_take,
                        ..Fake::default()
                    };
                    return_focus(&shared, &fake);
                    let back = should_return_focus(fg, OURS, saved, can_take);
                    let expected: Vec<String> = if back {
                        vec![format!("foreground {saved}")]
                    } else {
                        Vec::new()
                    };
                    let case = format!("fg {fg} saved {saved} can_take {can_take}");
                    assert_eq!(fake.calls(), expected, "{case}");
                    assert_eq!(shared.return_to.load(Ordering::SeqCst), 0, "{case}");
                }
            }
        }
    }

    /// Destroying stops the `charm` scheme and takes the mark off first;
    /// closing goes through the window's own close.
    #[test]
    fn destroy_unmarks_first_and_close_closes() {
        let shared = with_window(Phase::Starting { instance: 1 });
        let fake = Fake::default();
        window_effect(&shared, Effect::DestroyWindow, &fake);
        assert_eq!(fake.calls(), ["unmark 7", "destroy"]);
        assert!(!shared.overlay_live.load(Ordering::SeqCst));

        let fake = Fake::default();
        window_effect(&shared, Effect::CloseWindow, &fake);
        assert_eq!(fake.calls(), ["close"]);
    }

    /// A new window starts live with clean cells; a closing one stops being
    /// served and loses its mark; a destroyed one leaves nothing behind.
    #[test]
    fn the_cells_follow_the_window() {
        let shared = OverlayShared::new(true);
        shared.shown.store(true, Ordering::SeqCst);
        shared.applied_protect.store(true, Ordering::SeqCst);
        shared.return_to.store(GAME, Ordering::SeqCst);
        adopt(&shared, OURS);
        assert_eq!(shared.hwnd.load(Ordering::SeqCst), OURS);
        assert!(!shared.shown.load(Ordering::SeqCst));
        assert!(!shared.applied_protect.load(Ordering::SeqCst));
        assert_eq!(shared.return_to.load(Ordering::SeqCst), 0);
        assert!(shared.overlay_live.load(Ordering::SeqCst));

        let fake = Fake::default();
        going(&shared, &fake);
        assert_eq!(fake.calls(), ["unmark 7"]);
        assert!(!shared.overlay_live.load(Ordering::SeqCst));

        shared.shown.store(true, Ordering::SeqCst);
        shared.applied_protect.store(true, Ordering::SeqCst);
        shared.return_to.store(GAME, Ordering::SeqCst);
        forget(&shared);
        assert_eq!(shared.hwnd.load(Ordering::SeqCst), 0);
        assert!(!shared.shown.load(Ordering::SeqCst));
        assert!(!shared.applied_protect.load(Ordering::SeqCst));
        assert_eq!(shared.return_to.load(Ordering::SeqCst), 0);
        assert!(!shared.overlay_live.load(Ordering::SeqCst));
    }

    /// `place_overlay` refuses before touching anything: a window being
    /// built is still starting; none, or a closing one, isn't open.
    #[test]
    fn place_checks_the_phase_first() {
        let creating = Phase::Creating {
            instance: 1,
            ready_early: false,
            cancel: false,
        };
        assert_eq!(placeable(creating), Err(STARTING.to_owned()));
        assert_eq!(placeable(Phase::Closed), Err(NOT_OPEN.to_owned()));
        assert_eq!(
            placeable(Phase::Closing { instance: 1 }),
            Err(NOT_OPEN.to_owned())
        );
        for phase in [
            Phase::Starting { instance: 1 },
            Phase::Shown { instance: 1 },
            Phase::Painted { instance: 1 },
        ] {
            assert_eq!(placeable(phase), Ok(()), "{phase:?}");
        }
    }

    /// `open` waits for a closing window to finish, but not forever.
    #[test]
    fn open_waits_for_closed_but_not_forever() {
        let poll = Duration::from_millis(5);
        let shared = OverlayShared::new(true);
        assert!(wait_closed(&shared, Duration::ZERO, poll));

        shared.core().phase = Phase::Closing { instance: 1 };
        assert!(!wait_closed(&shared, Duration::from_millis(30), poll));

        let shared = std::sync::Arc::new(shared);
        let s = std::sync::Arc::clone(&shared);
        let closer = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(20));
            s.core().phase = Phase::Closed;
        });
        assert!(wait_closed(&shared, Duration::from_secs(5), poll));
        closer.join().unwrap();
    }

    /// The page reads its instance from the query, and the debug switch
    /// sets the global the page checks.
    #[test]
    fn the_page_url_and_the_debug_switch() {
        assert_eq!(overlay_url(3), "overlay.html?instance=3");
        assert_eq!(no_ready_script(false), None);
        assert_eq!(
            no_ready_script(true).as_deref(),
            Some("window.__DIVORA_OVERLAY_NO_READY__ = true;")
        );
    }
}
