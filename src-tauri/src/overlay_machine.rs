//! v1.55.0: the stream overlay's lifecycle and mode, as a pure machine.
//!
//! The backend owns the overlay window: it builds it hidden, shows it once
//! the page says it is listening, lets it be arranged only once the page has
//! painted, and locks it again when the page stops answering. Every input —
//! the overlay commands, the page's handshake and beats, the watchdog, the
//! lease, the recorder, the hotkey — goes through [`OverlayCore::step`], and
//! comes back as [`Effect`]s for the shell (`overlay_windows.rs`) to run on
//! the real window. Nothing here touches a window or calls Tauri, so every
//! rule is tested on any machine. [`OverlayStatus`] is what the windows are
//! told after each change.

use std::time::Duration;

use serde::{Deserialize, Serialize};

/// The overlay window's label. The capability and `getByLabel` use it.
pub const OVERLAY_LABEL: &str = "overlay";

/// The overlay window's title. OBS Window Capture sources match it, so it is
/// frozen.
pub const OVERLAY_TITLE: &str = "DivoraVoice Overlay";

/// The event every status change goes out as. Payload: [`OverlayStatus`].
pub const OVERLAY_STATUS_EVENT: &str = "overlay:status";

/// How long a new window may take to say it is listening before it is
/// destroyed.
pub const WATCHDOG: Duration = Duration::from_secs(10);

/// How long a capture waits for the overlay's protection to be applied
/// before it starts anyway.
pub const CAPTURE_ACK: Duration = Duration::from_millis(300);

/// How often the Arrange lease checks for the page's beats.
pub const LEASE_TICK: Duration = Duration::from_secs(1);

/// Ticks without a beat before the lease locks the overlay again.
pub const LEASE_MISSES: u8 = 3;

/// Which rung of the click-through ladder ships (0: tao as is; 1: also make
/// the layered window opaque). A code constant, never a setting.
pub const R1_RUNG: u8 = 0;

/// The tray menu's item for leaving Arrange, shown only while arranging.
pub const TRAY_LOCK_ID: &str = "overlay-lock";
pub const TRAY_LOCK_LABEL: &str = "Lock the overlay";

/// The hotkey that switches the open overlay between Arrange and Locked.
pub const HOTKEY_ARRANGE: &str = "overlay:arrange";

/// A debug-only switch: the overlay page skips its ready handshake, so the
/// watchdog can be checked. The shell passes it to the page as
/// [`NO_READY_GLOBAL`].
pub const PRETEND_NO_READY_ENV: &str = "DIVORA_PRETEND_OVERLAY_NO_READY";

/// The global an init script sets on the overlay page when
/// [`PRETEND_NO_READY_ENV`] is set.
pub const NO_READY_GLOBAL: &str = "__DIVORA_OVERLAY_NO_READY__";

/// Refusals, in the user's words.
pub const NOT_OPEN: &str = "The overlay isn't open.";
pub const STARTING: &str = "The overlay is still starting. Try again in a moment.";
pub const CLOSING: &str = "The overlay is still closing. Try again in a moment.";
pub const BAD_MODE: &str = "Unknown overlay mode.";
pub const BAD_STAGE: &str = "Unknown overlay stage.";

/// Why the watchdog closed a window that never said it was listening.
pub const NO_START: &str =
    "The overlay window didn't start, so Divora closed it. Try turning it on again.";

/// Why the lease locked the overlay: the page never beat in this Arrange.
pub const LEASE_NEVER: &str = "The overlay didn't respond, so it stayed locked.";

/// Why the lease locked the overlay: the page beat, then stopped.
pub const LEASE_LOST: &str = "The overlay stopped responding, so Divora locked it. Turn it off and on again if it looks wrong.";

/// Whether the overlay passes clicks through (Locked) or takes them
/// (Arrange). Never persisted: the overlay always opens Locked.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    Locked,
    Arrange,
}

/// What `set_overlay_mode` and the hotkey ask for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModeRequest {
    Locked,
    Arrange,
    /// Arrange when Locked, Locked when arranging.
    Toggle,
}

impl ModeRequest {
    /// `"locked"`, `"arrange"` or `"toggle"`; anything else is `None`.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "locked" => Some(Self::Locked),
            "arrange" => Some(Self::Arrange),
            "toggle" => Some(Self::Toggle),
            _ => None,
        }
    }
}

/// The overlay page's `overlay_handshake` stages.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HandshakeStage {
    /// The page is listening for its events.
    Ready,
    /// The page has drawn a frame since it was shown.
    Painted,
}

impl HandshakeStage {
    /// `"ready"` or `"painted"`; anything else is `None`.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "ready" => Some(Self::Ready),
            "painted" => Some(Self::Painted),
            _ => None,
        }
    }
}

/// A press of [`HOTKEY_ARRANGE`] is a [`ModeRequest::Toggle`]. Releases
/// (which arrive from global-hotkey's polling thread) and other ids are
/// `None`.
#[doc(hidden)]
pub fn hotkey_overlay_request(id: &str, pressed: bool) -> Option<ModeRequest> {
    (pressed && id == HOTKEY_ARRANGE).then_some(ModeRequest::Toggle)
}

/// Where the overlay window is in its life. `instance` numbers each window,
/// so a stale call from an earlier one never counts for a new one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Closed,
    /// Being built. `ready_early`: the page said ready before `Built`.
    /// `cancel`: closed while being built.
    Creating {
        instance: u32,
        ready_early: bool,
        cancel: bool,
    },
    /// Built, hidden, click-through set, placed; waiting for the ready
    /// handshake, with the watchdog armed.
    Starting {
        instance: u32,
    },
    /// Shown; waiting for the painted handshake.
    Shown {
        instance: u32,
    },
    /// Painted: Arrange is allowed.
    Painted {
        instance: u32,
    },
    Closing {
        instance: u32,
    },
}

/// [`Phase`] as the wire names it. A window being built reads as starting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum PhaseName {
    Closed,
    Starting,
    Shown,
    Painted,
    Closing,
}

/// Everything that can happen to the overlay.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Input {
    /// `open_overlay`, fitted to its monitor or not.
    Open {
        fit: bool,
    },
    /// The window was built, placed and made click-through.
    Built {
        instance: u32,
    },
    BuildFailed {
        instance: u32,
    },
    /// The page's `overlay_handshake` stages.
    Ready {
        instance: u32,
    },
    Painted {
        instance: u32,
    },
    /// The watchdog's time ran out.
    Watchdog {
        instance: u32,
    },
    /// `close_overlay`.
    Close,
    /// The window is closing (Alt+F4, or [`Effect::CloseWindow`]).
    CloseRequested,
    /// The window is gone.
    Destroyed,
    Mode(ModeRequest),
    /// The page's beat while arranging (`overlay_alive`).
    Alive {
        instance: u32,
        gen: u32,
    },
    /// The lease's tick.
    LeaseTick {
        instance: u32,
        gen: u32,
    },
    /// The Hide from recordings toggle.
    HideWanted(bool),
    /// A take is running, or Replay is on.
    Capturing(bool),
    /// `place_overlay` placed the window.
    Placed {
        fit: bool,
    },
}

/// What the shell does after a step, in order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Effect {
    /// Build the window (`open` does this itself).
    Build {
        instance: u32,
    },
    /// Apply [`OverlayCore::window`] to the live window.
    ApplyWindow,
    /// Apply the window state, then show the window, back to back.
    Show,
    /// Take the foreground, remembering who had it.
    Focus,
    /// Hand the foreground back to the window that had it before Arrange.
    ReturnFocus,
    ArmWatchdog {
        instance: u32,
    },
    ArmLease {
        instance: u32,
        gen: u32,
    },
    /// Close the window (it then sends [`Input::CloseRequested`]).
    CloseWindow,
    DestroyWindow,
    /// Send [`OverlayStatus`] on [`OVERLAY_STATUS_EVENT`].
    Emit,
    /// Rebuild the tray menu (the Lock item comes and goes with Arrange).
    RefreshTray,
}

/// Why a step was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    NotOpen,
    Starting,
    Closing,
    BadMode,
}

impl Refusal {
    /// The refusal in the user's words.
    pub fn message(self) -> &'static str {
        match self {
            Self::NotOpen => NOT_OPEN,
            Self::Starting => STARTING,
            Self::Closing => CLOSING,
            Self::BadMode => BAD_MODE,
        }
    }
}

/// The machine. The shell holds one behind a mutex and only ever changes it
/// through [`OverlayCore::step`].
#[derive(Debug, Clone)]
#[allow(clippy::struct_excessive_bools)] // independent facts the machine holds
pub struct OverlayCore {
    pub phase: Phase,
    pub mode: Mode,
    /// The latest window's number: the live one's, or the last one's; 0
    /// before any.
    pub last_instance: u32,
    /// Bumped on every entry to Arrange; the page's beats carry it.
    pub arrange_gen: u32,
    /// This Arrange has had its first beat, so click-through is off.
    pub arranged: bool,
    /// Lease ticks since the last beat.
    pub missed: u8,
    /// The window was placed fitted to its monitor.
    pub fit: bool,
    pub hide_wanted: bool,
    /// Windows can leave a window out of captures (the contract gate).
    pub hide_available: bool,
    pub capturing: bool,
    /// Why this window is closing abnormally, or why the lease ended
    /// Arrange.
    pub error: Option<&'static str>,
}

impl OverlayCore {
    /// Closed and Locked, with nothing wanted or running.
    pub fn new(hide_available: bool) -> Self {
        Self {
            phase: Phase::Closed,
            mode: Mode::Locked,
            last_instance: 0,
            arrange_gen: 0,
            arranged: false,
            missed: 0,
            fit: false,
            hide_wanted: false,
            hide_available,
            capturing: false,
            error: None,
        }
    }

    /// Apply `input`: the effects to run, in order, or why it was refused.
    /// Anything for another window or another Arrange changes nothing.
    #[allow(clippy::too_many_lines)] // §7.1 in one match, an arm per cell
    pub fn step(&mut self, input: Input) -> Result<Vec<Effect>, Refusal> {
        use Effect::{
            ApplyWindow, ArmWatchdog, Build, CloseWindow, DestroyWindow, Emit, RefreshTray, Show,
        };
        let effects = match (input, self.phase) {
            (Input::Open { fit }, Phase::Closed) => {
                // Wrapping, so no input can panic under the shell's lock.
                let n = self.last_instance.wrapping_add(1);
                self.last_instance = n;
                self.phase = Phase::Creating {
                    instance: n,
                    ready_early: false,
                    cancel: false,
                };
                self.fit = fit;
                self.error = None;
                vec![Build { instance: n }, Emit]
            }
            // The shell waits for Closed and tries once more.
            (Input::Open { .. }, Phase::Closing { .. }) => return Err(Refusal::Closing),
            (
                Input::Built { instance: j },
                Phase::Creating {
                    instance,
                    ready_early,
                    cancel,
                },
            ) if j == instance => {
                if cancel {
                    self.phase = Phase::Closing { instance };
                    vec![CloseWindow, Emit]
                } else if ready_early {
                    // Click-through went on in `ApplyWindow`, before the show.
                    self.phase = Phase::Shown { instance };
                    vec![ApplyWindow, ArmWatchdog { instance }, Show, Emit]
                } else {
                    self.phase = Phase::Starting { instance };
                    vec![ApplyWindow, ArmWatchdog { instance }, Emit]
                }
            }
            (Input::BuildFailed { instance: j }, Phase::Creating { instance, .. })
                if j == instance =>
            {
                self.closed();
                vec![Emit]
            }
            (
                Input::Ready { instance: j },
                Phase::Creating {
                    instance, cancel, ..
                },
            ) if j == instance => {
                // Shown as soon as it is built.
                self.phase = Phase::Creating {
                    instance,
                    ready_early: true,
                    cancel,
                };
                Vec::new()
            }
            (Input::Ready { instance: j }, Phase::Starting { instance }) if j == instance => {
                self.phase = Phase::Shown { instance };
                vec![Show, Emit]
            }
            (Input::Ready { instance: j }, Phase::Shown { instance }) if j == instance => {
                vec![ApplyWindow, Emit]
            }
            (Input::Ready { instance: j }, Phase::Painted { instance }) if j == instance => {
                // The page reloaded: it paints again before it may arrange.
                self.phase = Phase::Shown { instance };
                if self.mode == Mode::Arrange {
                    self.lock_edge()
                } else {
                    vec![ApplyWindow, Emit]
                }
            }
            (Input::Painted { instance: j }, Phase::Shown { instance }) if j == instance => {
                self.phase = Phase::Painted { instance };
                vec![Emit]
            }
            (Input::Watchdog { instance: j }, Phase::Starting { instance }) if j == instance => {
                self.phase = Phase::Closing { instance };
                self.error = Some(NO_START);
                vec![DestroyWindow, Emit]
            }
            (
                Input::Close,
                Phase::Creating {
                    instance,
                    ready_early,
                    ..
                },
            ) => {
                // Closed once it is built.
                self.phase = Phase::Creating {
                    instance,
                    ready_early,
                    cancel: true,
                };
                Vec::new()
            }
            (
                Input::Close,
                Phase::Starting { instance }
                | Phase::Shown { instance }
                | Phase::Painted { instance },
            ) => {
                let mut effects = vec![CloseWindow, Emit];
                effects.extend(self.closing(instance));
                effects
            }
            (
                Input::CloseRequested,
                Phase::Starting { instance }
                | Phase::Shown { instance }
                | Phase::Painted { instance },
            ) => {
                let mut effects = vec![Emit];
                effects.extend(self.closing(instance));
                effects
            }
            (Input::Destroyed, _) if self.has_window() => {
                self.closed();
                vec![Emit, RefreshTray]
            }
            (Input::Mode(req), _) => return self.set_mode(req),
            (Input::Alive { instance, gen }, _) if self.in_lease(instance, gen) => {
                self.missed = 0;
                if self.arranged {
                    Vec::new()
                } else {
                    // The first beat: now the window takes clicks.
                    self.arranged = true;
                    vec![ApplyWindow]
                }
            }
            (Input::LeaseTick { instance, gen }, _) if self.in_lease(instance, gen) => {
                self.missed = self.missed.saturating_add(1);
                if self.missed < LEASE_MISSES {
                    Vec::new()
                } else {
                    self.error = Some(if self.arranged {
                        LEASE_LOST
                    } else {
                        LEASE_NEVER
                    });
                    self.lock_edge()
                }
            }
            (Input::HideWanted(on), _) => {
                let changed = self.hide_wanted != on;
                self.hide_wanted = on;
                self.fact_changed(changed)
            }
            (Input::Capturing(on), _) => {
                let changed = self.capturing != on;
                self.capturing = on;
                self.fact_changed(changed)
            }
            (Input::Placed { fit }, _) if self.live() => {
                self.fit = fit;
                vec![ApplyWindow, Emit]
            }
            (Input::Placed { .. }, Phase::Creating { .. }) => return Err(Refusal::Starting),
            (Input::Placed { .. }, _) => return Err(Refusal::NotOpen),
            // The table's "—": nothing changes, nothing to do.
            _ => Vec::new(),
        };
        Ok(effects)
    }

    /// `Mode(req)`. Arrange only once the page has painted; Locked never
    /// fails.
    fn set_mode(&mut self, req: ModeRequest) -> Result<Vec<Effect>, Refusal> {
        let arranging = self.mode == Mode::Arrange;
        let arrange = match req {
            ModeRequest::Locked => false,
            ModeRequest::Arrange => true,
            ModeRequest::Toggle => !arranging,
        };
        match (arrange, self.phase) {
            (false, Phase::Painted { .. }) if arranging => Ok(self.lock_edge()),
            (false, _) => Ok(Vec::new()),
            (true, Phase::Painted { .. }) if arranging => Ok(Vec::new()),
            (true, Phase::Painted { instance }) => {
                self.mode = Mode::Arrange;
                self.arrange_gen = self.arrange_gen.wrapping_add(1);
                self.arranged = false;
                self.missed = 0;
                self.error = None;
                // Click-through stays on until the page's first beat.
                Ok(vec![
                    Effect::Focus,
                    Effect::Emit,
                    Effect::RefreshTray,
                    Effect::ArmLease {
                        instance,
                        gen: self.arrange_gen,
                    },
                ])
            }
            (true, Phase::Closed | Phase::Closing { .. }) => Err(Refusal::NotOpen),
            (true, _) => Err(Refusal::Starting),
        }
    }

    /// Arrange → Locked: click-through back on, and the keyboard handed
    /// back.
    fn lock_edge(&mut self) -> Vec<Effect> {
        self.mode = Mode::Locked;
        self.arranged = false;
        self.missed = 0;
        vec![
            Effect::ApplyWindow,
            Effect::ReturnFocus,
            Effect::Emit,
            Effect::RefreshTray,
        ]
    }

    /// Into Closing, Locked. A closing window gets no window state
    /// (`reconcile_plan` is `None`) and the keyboard isn't handed back, so
    /// leaving Arrange here only takes the tray item away.
    fn closing(&mut self, instance: u32) -> Option<Effect> {
        let was_arranging = self.mode == Mode::Arrange;
        self.phase = Phase::Closing { instance };
        self.mode = Mode::Locked;
        self.arranged = false;
        self.missed = 0;
        was_arranging.then_some(Effect::RefreshTray)
    }

    /// Back to Closed: Locked, not fitted, no error.
    fn closed(&mut self) {
        self.phase = Phase::Closed;
        self.mode = Mode::Locked;
        self.arranged = false;
        self.missed = 0;
        self.fit = false;
        self.error = None;
    }

    /// The hide toggle or the capture state was set: re-apply a live window
    /// and tell the windows, if it changed.
    fn fact_changed(&self, changed: bool) -> Vec<Effect> {
        match (changed, self.live()) {
            (false, _) => Vec::new(),
            (true, true) => vec![Effect::ApplyWindow, Effect::Emit],
            (true, false) => vec![Effect::Emit],
        }
    }

    /// The current window's instance: being built, live or closing.
    pub fn instance(&self) -> Option<u32> {
        match self.phase {
            Phase::Closed => None,
            Phase::Creating { instance, .. }
            | Phase::Starting { instance }
            | Phase::Shown { instance }
            | Phase::Painted { instance }
            | Phase::Closing { instance } => Some(instance),
        }
    }

    /// A window exists: Starting, Shown, Painted or Closing.
    pub fn has_window(&self) -> bool {
        matches!(
            self.phase,
            Phase::Starting { .. }
                | Phase::Shown { .. }
                | Phase::Painted { .. }
                | Phase::Closing { .. }
        )
    }

    /// The window is given its window state (`reconcile_plan`) and `Placed`
    /// is accepted: Starting, Shown or Painted. A closing window still has
    /// its `NonRudeHWND` mark taken off and is closed or destroyed.
    pub fn live(&self) -> bool {
        matches!(
            self.phase,
            Phase::Starting { .. } | Phase::Shown { .. } | Phase::Painted { .. }
        )
    }

    /// Locked, or arranging before the page's first beat.
    pub fn effectively_locked(&self) -> bool {
        !(self.mode == Mode::Arrange && self.arranged)
    }

    /// Divora asks Windows to leave the window out of captures. Never for a
    /// window that isn't shown yet; still while it closes, so nothing lowers
    /// it while it is on screen.
    pub fn protected(&self) -> bool {
        let shown = matches!(
            self.phase,
            Phase::Shown { .. } | Phase::Painted { .. } | Phase::Closing { .. }
        );
        self.hide_wanted
            && self.hide_available
            && self.effectively_locked()
            && self.capturing
            && shown
    }

    /// A beat or tick for `instance` and `gen` belongs to this Arrange.
    pub fn in_lease(&self, instance: u32, gen: u32) -> bool {
        self.phase == Phase::Painted { instance }
            && self.mode == Mode::Arrange
            && self.arrange_gen == gen
    }

    /// The state the live window should be in.
    pub fn window(&self) -> WindowState {
        WindowState {
            ignore_cursor: self.effectively_locked(),
            protected: self.protected(),
            resizable: !self.fit,
        }
    }

    /// The setters that bring the window to [`OverlayCore::window`], in
    /// order; `None` unless the window is live.
    pub fn reconcile_plan(&self, rung: u8) -> Option<Vec<Setter>> {
        self.live().then(|| setter_plan(self.window(), rung))
    }

    /// What the windows are told.
    pub fn status(&self) -> OverlayStatus {
        let phase = match self.phase {
            Phase::Closed => PhaseName::Closed,
            Phase::Creating { .. } | Phase::Starting { .. } => PhaseName::Starting,
            Phase::Shown { .. } => PhaseName::Shown,
            Phase::Painted { .. } => PhaseName::Painted,
            Phase::Closing { .. } => PhaseName::Closing,
        };
        OverlayStatus {
            open: phase != PhaseName::Closed,
            phase,
            instance: self.last_instance,
            mode: self.mode,
            arrange_gen: self.arrange_gen,
            can_arrange: phase == PhaseName::Painted,
            fit: self.fit,
            hide_wanted: self.hide_wanted,
            hide_available: self.hide_available,
            capturing: self.capturing,
            error: self.error.map(str::to_owned),
        }
    }
}

/// The live window's click-through, capture protection and resizability.
/// It is shown once and never hidden, so visibility isn't part of it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WindowState {
    pub ignore_cursor: bool,
    pub protected: bool,
    pub resizable: bool,
}

/// One window call the shell makes to apply a [`WindowState`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Setter {
    IgnoreCursor(bool),
    /// Rung 1 only: make the layered window opaque.
    LayerOpaque,
    Resizable(bool),
    /// The display affinity.
    Protect(bool),
}

/// The setters for `w`, in the order they must run.
#[doc(hidden)]
pub fn setter_plan(w: WindowState, rung: u8) -> Vec<Setter> {
    if w.ignore_cursor {
        // Locked: the affinity last, so it is set on this apply's final
        // styles. An unmeasured hedge for older Windows builds.
        let mut plan = vec![Setter::IgnoreCursor(true)];
        if rung == 1 {
            plan.push(Setter::LayerOpaque);
        }
        plan.extend([Setter::Resizable(w.resizable), Setter::Protect(w.protected)]);
        plan
    } else {
        // Arrange: the affinity off first, while the window is still layered.
        vec![
            Setter::Protect(false),
            Setter::IgnoreCursor(false),
            Setter::Resizable(w.resizable),
        ]
    }
}

/// Whether to set the display affinity, and to what: only once the window's
/// Show has begun (`shown` is the shell's cell, set just before the window
/// shows), and only when it changes.
#[doc(hidden)]
pub fn affinity_call(want: bool, shown: bool, applied: bool) -> Option<bool> {
    // A window never protected never gets WDA_NONE either.
    (shown && want != applied).then_some(want)
}

/// Whether Lock hands the foreground back to `saved` (window handles as
/// integers; 0 is none).
#[doc(hidden)]
pub fn should_return_focus(fg: isize, ours: isize, saved: isize, saved_ok: bool) -> bool {
    // Only while the overlay still has the keyboard: a Lock from Settings or
    // the tray leaves it where it is.
    ours != 0 && fg == ours && saved != 0 && saved != ours && saved_ok
}

/// What every overlay command returns and `overlay:status` carries.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
#[allow(clippy::struct_excessive_bools)] // flat status flags for one wire event
pub struct OverlayStatus {
    /// A window exists (any phase but closed).
    pub open: bool,
    pub phase: PhaseName,
    /// The current or last window's number; 0 before any.
    pub instance: u32,
    pub mode: Mode,
    /// Bumped on every entry to Arrange; the page echoes it in its beats.
    pub arrange_gen: u32,
    /// The phase is painted.
    pub can_arrange: bool,
    /// The window was placed fitted to its monitor.
    pub fit: bool,
    /// The Hide from recordings toggle, as the backend holds it.
    pub hide_wanted: bool,
    /// Windows can leave the overlay out of captures.
    pub hide_available: bool,
    /// A take is running, or Replay is on.
    pub capturing: bool,
    /// Why this window is closing abnormally (the watchdog), or why the
    /// lease ended Arrange.
    pub error: Option<String>,
}

/// A window rectangle in physical pixels, as main computes it.
#[derive(Debug, Clone, Copy, Deserialize)]
pub struct OverlayRect {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

impl OverlayRect {
    /// The rectangle, if each side is 1..=32768 px.
    pub fn sane(self) -> Option<Self> {
        let side = 1..=32_768;
        (side.contains(&self.width) && side.contains(&self.height)).then_some(self)
    }
}

/// Whether the window read back at `pos` and `size` is where it was put.
#[doc(hidden)]
pub fn placement_settled(want: OverlayRect, pos: (i32, i32), size: (u32, u32)) -> bool {
    // Exactly, as the JS placement did: WM_DPICHANGED may have rescaled it.
    pos == (want.x, want.y) && size == (want.width, want.height)
}

#[cfg(test)]
mod tests {
    use super::{
        affinity_call, hotkey_overlay_request, placement_settled, setter_plan, should_return_focus,
        Effect, HandshakeStage, Input, Mode, ModeRequest, OverlayCore, OverlayRect, Phase,
        PhaseName, Refusal, Setter, WindowState, HOTKEY_ARRANGE, LEASE_LOST, LEASE_MISSES,
        LEASE_NEVER, NO_READY_GLOBAL, NO_START, OVERLAY_LABEL, OVERLAY_STATUS_EVENT, OVERLAY_TITLE,
        PRETEND_NO_READY_ENV, TRAY_LOCK_ID, TRAY_LOCK_LABEL,
    };
    use serde_json::Value;
    use std::collections::{BTreeSet, HashSet};
    use Effect::{
        ApplyWindow, ArmLease, ArmWatchdog, Build, CloseWindow, DestroyWindow, Emit, Focus,
        RefreshTray, ReturnFocus, Show,
    };

    /// The window every test opens first.
    const I: u32 = 1;

    /// What every Arrange → Locked edge does.
    const LOCK_EDGE: [Effect; 4] = [ApplyWindow, ReturnFocus, Emit, RefreshTray];

    /// The wire contract both test suites read.
    fn fixture() -> Value {
        serde_json::from_str(include_str!("../../src/overlay/wire.fixture.json")).unwrap()
    }

    fn sorted_keys(v: &Value) -> Vec<String> {
        let mut keys: Vec<String> = v.as_object().unwrap().keys().cloned().collect();
        keys.sort();
        keys
    }

    /// Step through `inputs`, failing on a refusal; every effect, in order.
    fn step_all(core: &mut OverlayCore, inputs: &[Input]) -> Vec<Effect> {
        let mut effects = Vec::new();
        for &input in inputs {
            match core.step(input) {
                Ok(e) => effects.extend(e),
                Err(r) => panic!("{input:?} refused: {r:?}"),
            }
        }
        effects
    }

    /// Window 1, shown and painted, Locked; Hide from recordings offered.
    fn painted() -> OverlayCore {
        let mut core = OverlayCore::new(true);
        step_all(
            &mut core,
            &[
                Input::Open { fit: false },
                Input::Built { instance: I },
                Input::Ready { instance: I },
                Input::Painted { instance: I },
            ],
        );
        core
    }

    /// [`painted`], then in Arrange (generation 1) before the page's first
    /// beat.
    fn arranging() -> OverlayCore {
        let mut core = painted();
        step_all(&mut core, &[Input::Mode(ModeRequest::Arrange)]);
        core
    }

    /// The whole core, to check nothing else changed (it has no
    /// `PartialEq`).
    fn facts(core: &OverlayCore) -> String {
        format!("{core:?}")
    }

    /// `src/overlay/wire.fixture.json` is what `status.test.ts` reads too,
    /// so a change on either side fails the other side's tests.
    #[test]
    fn wire_fixture_matches_rust() {
        let f = fixture();
        assert_eq!(f["events"]["status"], OVERLAY_STATUS_EVENT);
        assert_eq!(f["debug"]["noReadyEnv"], PRETEND_NO_READY_ENV);
        assert_eq!(f["debug"]["noReadyGlobal"], NO_READY_GLOBAL);

        let closed = serde_json::to_value(OverlayCore::new(false).status()).unwrap();
        assert_eq!(closed, f["statusSamples"]["closed"]);
        let keys: Vec<Value> = sorted_keys(&closed).into_iter().map(Value::from).collect();
        assert_eq!(Value::from(keys), f["statusKeys"]);

        // The other samples, from the machine driven there.
        let mut arranging = OverlayCore::new(false);
        step_all(
            &mut arranging,
            &[
                Input::Open { fit: false },
                Input::Built { instance: I },
                Input::Ready { instance: I },
                Input::Painted { instance: I },
                Input::HideWanted(true),
                Input::Capturing(true),
                Input::Mode(ModeRequest::Arrange),
            ],
        );
        assert_eq!(
            serde_json::to_value(arranging.status()).unwrap(),
            f["statusSamples"]["arranging"]
        );
        let mut watchdog = OverlayCore::new(false);
        step_all(
            &mut watchdog,
            &[
                Input::Open { fit: false },
                Input::Built { instance: I },
                Input::Watchdog { instance: I },
            ],
        );
        assert_eq!(
            serde_json::to_value(watchdog.status()).unwrap(),
            f["statusSamples"]["watchdog"]
        );

        for s in f["modeRequests"].as_array().unwrap() {
            assert!(ModeRequest::parse(s.as_str().unwrap()).is_some(), "{s}");
        }
        for s in f["handshakeStages"].as_array().unwrap() {
            assert!(HandshakeStage::parse(s.as_str().unwrap()).is_some(), "{s}");
        }
        for junk in ["", "Locked", "lock", "ready ", "shown"] {
            assert_eq!(ModeRequest::parse(junk), None, "{junk}");
            assert_eq!(HandshakeStage::parse(junk), None, "{junk}");
        }

        // A window being built reads as starting.
        let mut creating = OverlayCore::new(false);
        creating.step(Input::Open { fit: false }).unwrap();
        let s = creating.status();
        assert_eq!((s.open, s.phase), (true, PhaseName::Starting));
    }

    /// OBS Window Capture sources match the title; the capability and
    /// `getByLabel` use the label.
    #[test]
    fn title_and_label_are_the_obs_contract() {
        assert_eq!(OVERLAY_TITLE, "DivoraVoice Overlay");
        assert_eq!(OVERLAY_LABEL, "overlay");
        assert_eq!(
            (TRAY_LOCK_ID, TRAY_LOCK_LABEL),
            ("overlay-lock", "Lock the overlay")
        );
    }

    /// A §7.1 cell: the table's "—", a refusal, or these effects with the
    /// core changed by the function (and nothing else changed).
    #[derive(Clone, Copy)]
    enum Want {
        Same,
        No(Refusal),
        Go(&'static [Effect], fn(&mut OverlayCore)),
    }

    /// A §7.1 row: an input, and its cells that aren't "—", by column.
    type Row<'a> = (Input, &'a [(&'a [&'a str], Want)]);

    /// Back to Locked from Arrange.
    fn locked(c: &mut OverlayCore) {
        c.mode = Mode::Locked;
        c.arranged = false;
        c.missed = 0;
    }

    /// Into Arrange from the columns' generation 5.
    fn arrange_6(c: &mut OverlayCore) {
        c.mode = Mode::Arrange;
        c.arrange_gen = 6;
        c.arranged = false;
        c.missed = 0;
        c.error = None;
    }

    /// §7.1's columns: a core in each phase, holding the facts its rows
    /// need. Each window is instance 1, in arrange generation 5, fitted.
    fn columns() -> [(&'static str, OverlayCore); 10] {
        let at = |phase: Phase| {
            let mut core = OverlayCore::new(true);
            core.phase = phase;
            core.last_instance = 1;
            core.arrange_gen = 5;
            core.fit = phase != Phase::Closed;
            core
        };
        let creating = |ready_early, cancel| {
            at(Phase::Creating {
                instance: 1,
                ready_early,
                cancel,
            })
        };
        let mut painted = at(Phase::Painted { instance: 1 });
        // The lease locked it earlier, so the next Arrange clears an error.
        painted.error = Some(LEASE_NEVER);
        let mut arranging = at(Phase::Painted { instance: 1 });
        arranging.mode = Mode::Arrange;
        let mut arranged = arranging.clone();
        arranged.arranged = true;
        arranged.missed = LEASE_MISSES - 1;
        let mut closing = at(Phase::Closing { instance: 1 });
        closing.error = Some(NO_START);
        [
            ("closed", at(Phase::Closed)),
            ("creating", creating(false, false)),
            ("early", creating(true, false)),
            ("cancelled", creating(false, true)),
            ("starting", at(Phase::Starting { instance: 1 })),
            ("shown", at(Phase::Shown { instance: 1 })),
            ("painted", painted),
            // Arrange before the first beat, and after it with the lease
            // one tick from running out.
            ("arranging", arranging),
            ("arranged", arranged),
            ("closing", closing),
        ]
    }

    /// §7.1, every input against every phase. Each row lists its cells that
    /// aren't "—"; every other cell is checked to be "—". Instance 9 and
    /// generation 4 are stale.
    #[test]
    #[allow(clippy::too_many_lines)] // the table, one entry per cell
    fn machine_table_is_exhaustive() {
        use Want::{Go, No};
        const CREATING: &[&str] = &["creating", "early", "cancelled"];
        const BEFORE_PAINT: &[&str] = &["creating", "early", "cancelled", "starting", "shown"];
        const NOT_LIVE: &[&str] = &["closed", "creating", "early", "cancelled", "closing"];
        const LIVE: &[&str] = &["starting", "shown", "painted", "arranging", "arranged"];
        const ARRANGE: &[&str] = &["arranging", "arranged"];
        const ARRANGE_EFFECTS: &[Effect] = &[
            Focus,
            Emit,
            RefreshTray,
            ArmLease {
                instance: 1,
                gen: 6,
            },
        ];
        let rows: &[Row<'_>] = &[
            (
                Input::Open { fit: true },
                &[
                    (
                        &["closed"],
                        Go(&[Build { instance: 2 }, Emit], |c| {
                            c.phase = Phase::Creating {
                                instance: 2,
                                ready_early: false,
                                cancel: false,
                            };
                            c.last_instance = 2;
                            c.fit = true;
                        }),
                    ),
                    (&["closing"], No(Refusal::Closing)),
                ],
            ),
            (
                Input::Built { instance: 1 },
                &[
                    (
                        &["creating"],
                        Go(&[ApplyWindow, ArmWatchdog { instance: 1 }, Emit], |c| {
                            c.phase = Phase::Starting { instance: 1 };
                        }),
                    ),
                    (
                        &["early"],
                        Go(
                            &[ApplyWindow, ArmWatchdog { instance: 1 }, Show, Emit],
                            |c| c.phase = Phase::Shown { instance: 1 },
                        ),
                    ),
                    (
                        &["cancelled"],
                        Go(&[CloseWindow, Emit], |c| {
                            c.phase = Phase::Closing { instance: 1 };
                        }),
                    ),
                ],
            ),
            (Input::Built { instance: 9 }, &[]),
            (
                Input::BuildFailed { instance: 1 },
                &[(
                    CREATING,
                    Go(&[Emit], |c| {
                        c.phase = Phase::Closed;
                        c.fit = false;
                    }),
                )],
            ),
            (Input::BuildFailed { instance: 9 }, &[]),
            (
                Input::Ready { instance: 1 },
                &[
                    (
                        &["creating"],
                        Go(&[], |c| {
                            c.phase = Phase::Creating {
                                instance: 1,
                                ready_early: true,
                                cancel: false,
                            };
                        }),
                    ),
                    (
                        &["cancelled"],
                        Go(&[], |c| {
                            c.phase = Phase::Creating {
                                instance: 1,
                                ready_early: true,
                                cancel: true,
                            };
                        }),
                    ),
                    (
                        &["starting"],
                        Go(&[Show, Emit], |c| c.phase = Phase::Shown { instance: 1 }),
                    ),
                    // The page reloaded.
                    (&["shown"], Go(&[ApplyWindow, Emit], |_| {})),
                    (
                        &["painted"],
                        Go(&[ApplyWindow, Emit], |c| {
                            c.phase = Phase::Shown { instance: 1 };
                        }),
                    ),
                    (
                        ARRANGE,
                        Go(&LOCK_EDGE, |c| {
                            c.phase = Phase::Shown { instance: 1 };
                            locked(c);
                        }),
                    ),
                ],
            ),
            (Input::Ready { instance: 9 }, &[]),
            (
                Input::Painted { instance: 1 },
                &[(
                    &["shown"],
                    Go(&[Emit], |c| c.phase = Phase::Painted { instance: 1 }),
                )],
            ),
            (Input::Painted { instance: 9 }, &[]),
            (
                Input::Watchdog { instance: 1 },
                &[(
                    &["starting"],
                    Go(&[DestroyWindow, Emit], |c| {
                        c.phase = Phase::Closing { instance: 1 };
                        c.error = Some(NO_START);
                    }),
                )],
            ),
            (Input::Watchdog { instance: 9 }, &[]),
            (
                Input::Close,
                &[
                    (
                        &["creating"],
                        Go(&[], |c| {
                            c.phase = Phase::Creating {
                                instance: 1,
                                ready_early: false,
                                cancel: true,
                            };
                        }),
                    ),
                    (
                        &["early"],
                        Go(&[], |c| {
                            c.phase = Phase::Creating {
                                instance: 1,
                                ready_early: true,
                                cancel: true,
                            };
                        }),
                    ),
                    (
                        &["starting", "shown", "painted"],
                        Go(&[CloseWindow, Emit], |c| {
                            c.phase = Phase::Closing { instance: 1 };
                        }),
                    ),
                    (
                        ARRANGE,
                        Go(&[CloseWindow, Emit, RefreshTray], |c| {
                            c.phase = Phase::Closing { instance: 1 };
                            locked(c);
                        }),
                    ),
                ],
            ),
            (
                Input::CloseRequested,
                &[
                    (
                        &["starting", "shown", "painted"],
                        Go(&[Emit], |c| c.phase = Phase::Closing { instance: 1 }),
                    ),
                    (
                        ARRANGE,
                        Go(&[Emit, RefreshTray], |c| {
                            c.phase = Phase::Closing { instance: 1 };
                            locked(c);
                        }),
                    ),
                ],
            ),
            (
                Input::Destroyed,
                &[(
                    &[
                        "starting",
                        "shown",
                        "painted",
                        "arranging",
                        "arranged",
                        "closing",
                    ],
                    Go(&[Emit, RefreshTray], |c| {
                        c.phase = Phase::Closed;
                        c.fit = false;
                        c.error = None;
                        locked(c);
                    }),
                )],
            ),
            (
                Input::Mode(ModeRequest::Arrange),
                &[
                    (&["closed", "closing"], No(Refusal::NotOpen)),
                    (BEFORE_PAINT, No(Refusal::Starting)),
                    (&["painted"], Go(ARRANGE_EFFECTS, arrange_6)),
                ],
            ),
            (
                Input::Mode(ModeRequest::Locked),
                &[(ARRANGE, Go(&LOCK_EDGE, locked))],
            ),
            (
                Input::Mode(ModeRequest::Toggle),
                &[
                    (&["closed", "closing"], No(Refusal::NotOpen)),
                    (BEFORE_PAINT, No(Refusal::Starting)),
                    (&["painted"], Go(ARRANGE_EFFECTS, arrange_6)),
                    (ARRANGE, Go(&LOCK_EDGE, locked)),
                ],
            ),
            (
                Input::Alive {
                    instance: 1,
                    gen: 5,
                },
                &[
                    (&["arranging"], Go(&[ApplyWindow], |c| c.arranged = true)),
                    (&["arranged"], Go(&[], |c| c.missed = 0)),
                ],
            ),
            (
                Input::Alive {
                    instance: 9,
                    gen: 5,
                },
                &[],
            ),
            (
                Input::Alive {
                    instance: 1,
                    gen: 4,
                },
                &[],
            ),
            (
                Input::LeaseTick {
                    instance: 1,
                    gen: 5,
                },
                &[
                    (&["arranging"], Go(&[], |c| c.missed = 1)),
                    (
                        &["arranged"],
                        Go(&LOCK_EDGE, |c| {
                            c.error = Some(LEASE_LOST);
                            locked(c);
                        }),
                    ),
                ],
            ),
            (
                Input::LeaseTick {
                    instance: 9,
                    gen: 5,
                },
                &[],
            ),
            (
                Input::LeaseTick {
                    instance: 1,
                    gen: 4,
                },
                &[],
            ),
            (
                Input::HideWanted(true),
                &[
                    (NOT_LIVE, Go(&[Emit], |c| c.hide_wanted = true)),
                    (LIVE, Go(&[ApplyWindow, Emit], |c| c.hide_wanted = true)),
                ],
            ),
            (Input::HideWanted(false), &[]),
            (
                Input::Capturing(true),
                &[
                    (NOT_LIVE, Go(&[Emit], |c| c.capturing = true)),
                    (LIVE, Go(&[ApplyWindow, Emit], |c| c.capturing = true)),
                ],
            ),
            (Input::Capturing(false), &[]),
            (
                Input::Placed { fit: false },
                &[
                    (&["closed", "closing"], No(Refusal::NotOpen)),
                    (CREATING, No(Refusal::Starting)),
                    (LIVE, Go(&[ApplyWindow, Emit], |c| c.fit = false)),
                ],
            ),
        ];

        let columns = columns();
        let names: Vec<&str> = columns.iter().map(|&(name, _)| name).collect();
        for &(input, cells) in rows {
            for &(cols, _) in cells {
                for col in cols {
                    assert!(names.contains(col), "{input:?}: no column {col}");
                }
            }
            for (name, start) in &columns {
                let mut wants = cells
                    .iter()
                    .filter(|(cols, _)| cols.contains(name))
                    .map(|&(_, want)| want);
                let want = wants.next().unwrap_or(Want::Same);
                assert!(wants.next().is_none(), "{input:?}: {name} given twice");
                let mut core = start.clone();
                let got = core.step(input);
                let case = format!("{input:?} in {name}");
                let after = match want {
                    Want::Same => {
                        assert_eq!(got, Ok(Vec::new()), "{case}");
                        start.clone()
                    }
                    Want::No(refusal) => {
                        assert_eq!(got, Err(refusal), "{case}");
                        start.clone()
                    }
                    Want::Go(effects, change) => {
                        assert_eq!(got, Ok(effects.to_vec()), "{case}");
                        let mut after = start.clone();
                        change(&mut after);
                        after
                    }
                };
                assert_eq!(facts(&core), facts(&after), "{case}");
            }
        }

        // Every kind of input has a row and every phase a column: a new one
        // doesn't compile here until it is added.
        let kinds: BTreeSet<u8> = rows
            .iter()
            .map(|(input, _)| match input {
                Input::Open { .. } => 0,
                Input::Built { .. } => 1,
                Input::BuildFailed { .. } => 2,
                Input::Ready { .. } => 3,
                Input::Painted { .. } => 4,
                Input::Watchdog { .. } => 5,
                Input::Close => 6,
                Input::CloseRequested => 7,
                Input::Destroyed => 8,
                Input::Mode(ModeRequest::Locked) => 9,
                Input::Mode(ModeRequest::Arrange) => 10,
                Input::Mode(ModeRequest::Toggle) => 11,
                Input::Alive { .. } => 12,
                Input::LeaseTick { .. } => 13,
                Input::HideWanted(_) => 14,
                Input::Capturing(_) => 15,
                Input::Placed { .. } => 16,
            })
            .collect();
        assert_eq!(kinds.len(), 17);
        let phases: BTreeSet<u8> = columns
            .iter()
            .map(|(_, core)| match core.phase {
                Phase::Closed => 0,
                Phase::Creating { .. } => 1,
                Phase::Starting { .. } => 2,
                Phase::Shown { .. } => 3,
                Phase::Painted { .. } => 4,
                Phase::Closing { .. } => 5,
            })
            .collect();
        assert_eq!(phases.len(), 6);
    }

    #[test]
    fn cannot_arrange_before_painted() {
        let arrange = Input::Mode(ModeRequest::Arrange);
        let toggle = Input::Mode(ModeRequest::Toggle);
        let mut core = OverlayCore::new(true);
        assert_eq!(core.step(arrange), Err(Refusal::NotOpen));
        assert_eq!(core.step(toggle), Err(Refusal::NotOpen));
        for input in [
            Input::Open { fit: false },
            Input::Built { instance: I },
            Input::Ready { instance: I },
        ] {
            core.step(input).unwrap();
            assert_eq!(
                core.step(arrange),
                Err(Refusal::Starting),
                "after {input:?}"
            );
            assert_eq!(core.step(toggle), Err(Refusal::Starting), "after {input:?}");
            assert!(!core.status().can_arrange, "after {input:?}");
            // Locking never fails.
            assert_eq!(core.step(Input::Mode(ModeRequest::Locked)), Ok(Vec::new()));
        }
        core.step(Input::Painted { instance: I }).unwrap();
        assert!(core.status().can_arrange);
        assert!(core.step(arrange).unwrap().contains(&Focus));
        assert_eq!(core.mode, Mode::Arrange);
        // Closing: not open any more.
        core.step(Input::Close).unwrap();
        assert_eq!(core.step(arrange), Err(Refusal::NotOpen));
        assert_eq!(core.mode, Mode::Locked);
    }

    #[test]
    fn painted_for_another_instance_is_ignored() {
        let mut core = OverlayCore::new(true);
        step_all(
            &mut core,
            &[
                Input::Open { fit: false },
                Input::Built { instance: I },
                Input::Ready { instance: I },
            ],
        );
        for other in [0, 2, u32::MAX] {
            let before = facts(&core);
            assert_eq!(
                core.step(Input::Painted { instance: other }),
                Ok(Vec::new())
            );
            assert_eq!(facts(&core), before, "painted from {other}");
        }
        // The old window's page, after the next window opened.
        step_all(
            &mut core,
            &[
                Input::Close,
                Input::Destroyed,
                Input::Open { fit: false },
                Input::Built { instance: 2 },
                Input::Ready { instance: 2 },
            ],
        );
        assert_eq!(core.step(Input::Painted { instance: I }), Ok(Vec::new()));
        assert_eq!(core.status().phase, PhaseName::Shown);
        assert_eq!(core.step(Input::Painted { instance: 2 }), Ok(vec![Emit]));
        assert_eq!(core.status().phase, PhaseName::Painted);
    }

    #[test]
    fn ready_for_another_instance_is_ignored() {
        // While building: another page's ready doesn't make it show early.
        let mut core = OverlayCore::new(true);
        step_all(
            &mut core,
            &[Input::Open { fit: false }, Input::Ready { instance: 2 }],
        );
        assert_eq!(
            core.step(Input::Built { instance: I }),
            Ok(vec![ApplyWindow, ArmWatchdog { instance: I }, Emit])
        );
        // Starting: no show.
        assert_eq!(core.step(Input::Ready { instance: 2 }), Ok(Vec::new()));
        assert_eq!(core.status().phase, PhaseName::Starting);
        assert_eq!(
            core.step(Input::Ready { instance: I }),
            Ok(vec![Show, Emit])
        );
        // Shown and arranging: no reload for another page.
        assert_eq!(core.step(Input::Ready { instance: 0 }), Ok(Vec::new()));
        step_all(
            &mut core,
            &[
                Input::Painted { instance: I },
                Input::Mode(ModeRequest::Arrange),
            ],
        );
        let before = facts(&core);
        assert_eq!(core.step(Input::Ready { instance: 0 }), Ok(Vec::new()));
        assert_eq!(facts(&core), before);
    }

    #[test]
    fn ready_before_built_shows_after_built() {
        let mut core = OverlayCore::new(true);
        core.step(Input::Open { fit: false }).unwrap();
        // The page can load before `open` steps Built: nothing to show yet.
        assert_eq!(core.step(Input::Ready { instance: I }), Ok(Vec::new()));
        assert_eq!(core.status().phase, PhaseName::Starting);
        // Built: click-through first, then shown.
        assert_eq!(
            core.step(Input::Built { instance: I }),
            Ok(vec![ApplyWindow, ArmWatchdog { instance: I }, Show, Emit])
        );
        assert_eq!(core.status().phase, PhaseName::Shown);
        // Shown once: a reload re-applies, never shows again.
        assert_eq!(
            core.step(Input::Ready { instance: I }),
            Ok(vec![ApplyWindow, Emit])
        );
        // The watchdog it armed finds it shown.
        assert_eq!(core.step(Input::Watchdog { instance: I }), Ok(Vec::new()));
        assert_eq!(core.step(Input::Painted { instance: I }), Ok(vec![Emit]));
    }

    #[test]
    fn ready_in_painted_relocks_reload() {
        // Locked: back to Shown, re-applied, the keyboard left alone.
        let mut core = painted();
        assert_eq!(
            core.step(Input::Ready { instance: I }),
            Ok(vec![ApplyWindow, Emit])
        );
        assert_eq!(core.status().phase, PhaseName::Shown);
        assert_eq!(
            core.step(Input::Mode(ModeRequest::Arrange)),
            Err(Refusal::Starting)
        );

        // Arranging after a beat: locked, and the keyboard handed back.
        let mut core = arranging();
        core.step(Input::Alive {
            instance: I,
            gen: 1,
        })
        .unwrap();
        assert_eq!(
            core.step(Input::Ready { instance: I }),
            Ok(LOCK_EDGE.to_vec())
        );
        assert_eq!(
            (core.status().phase, core.mode),
            (PhaseName::Shown, Mode::Locked)
        );
        assert!(core.window().ignore_cursor);
        // That Arrange's lease ends with it.
        assert!(!core.in_lease(I, 1));
        // The reloaded page paints again before it may arrange.
        assert_eq!(
            core.step(Input::Mode(ModeRequest::Arrange)),
            Err(Refusal::Starting)
        );
        core.step(Input::Painted { instance: I }).unwrap();
        assert!(core.step(Input::Mode(ModeRequest::Arrange)).is_ok());
    }

    #[test]
    fn watchdog_fires_only_while_starting() {
        let watchdog = Input::Watchdog { instance: I };
        let ignored = |core: &mut OverlayCore| {
            let before = facts(core);
            assert_eq!(core.step(watchdog), Ok(Vec::new()), "{before}");
            assert_eq!(facts(core), before);
        };
        let mut core = OverlayCore::new(true);
        ignored(&mut core);
        core.step(Input::Open { fit: false }).unwrap();
        ignored(&mut core);
        core.step(Input::Built { instance: I }).unwrap();
        // Another window's watchdog.
        assert_eq!(core.step(Input::Watchdog { instance: 2 }), Ok(Vec::new()));
        // Ready in time: shown, painted or closing when it fires.
        let mut ready = core.clone();
        ready.step(Input::Ready { instance: I }).unwrap();
        ignored(&mut ready);
        ready.step(Input::Painted { instance: I }).unwrap();
        ignored(&mut ready);
        ready.step(Input::Close).unwrap();
        ignored(&mut ready);
        // Never ready: destroyed, with the reason.
        assert_eq!(core.step(watchdog), Ok(vec![DestroyWindow, Emit]));
        assert_eq!(core.status().phase, PhaseName::Closing);
        assert_eq!(core.status().error.as_deref(), Some(NO_START));
    }

    /// The watchdog's error rides only the closing statuses, so a main
    /// window that reloads later doesn't toast it again.
    #[test]
    fn watchdog_error_rides_only_closing() {
        let mut core = OverlayCore::new(true);
        step_all(
            &mut core,
            &[
                Input::Open { fit: false },
                Input::Built { instance: I },
                Input::Watchdog { instance: I },
            ],
        );
        assert_eq!(core.status().error.as_deref(), Some(NO_START));
        assert_eq!(core.step(Input::CloseRequested), Ok(Vec::new()));
        assert_eq!(core.status().error.as_deref(), Some(NO_START));
        core.step(Input::Destroyed).unwrap();
        assert_eq!(core.status().phase, PhaseName::Closed);
        assert_eq!(core.status().error, None);
        for input in [Input::HideWanted(true), Input::Capturing(true)] {
            assert_eq!(core.step(input), Ok(vec![Emit]));
            assert_eq!(core.status().error, None, "{input:?}");
        }
    }

    #[test]
    fn close_while_creating_closes_after_build() {
        for ready_early in [false, true] {
            let mut core = OverlayCore::new(true);
            core.step(Input::Open { fit: true }).unwrap();
            if ready_early {
                core.step(Input::Ready { instance: I }).unwrap();
            }
            // Nothing to close yet: remembered until it is built.
            assert_eq!(core.step(Input::Close), Ok(Vec::new()));
            assert_eq!(core.step(Input::Close), Ok(Vec::new()));
            assert_eq!(core.status().phase, PhaseName::Starting);
            // Built: closed at once, never shown, no watchdog.
            assert_eq!(
                core.step(Input::Built { instance: I }),
                Ok(vec![CloseWindow, Emit])
            );
            assert_eq!(core.status().phase, PhaseName::Closing);
            assert_eq!(core.step(Input::Ready { instance: I }), Ok(Vec::new()));
            assert_eq!(core.step(Input::CloseRequested), Ok(Vec::new()));
            assert_eq!(core.step(Input::Destroyed), Ok(vec![Emit, RefreshTray]));
            let s = core.status();
            assert_eq!((s.phase, s.fit), (PhaseName::Closed, false));
            // And it opens again as the next window.
            assert_eq!(
                core.step(Input::Open { fit: false }),
                Ok(vec![Build { instance: 2 }, Emit])
            );
        }
    }

    #[test]
    fn close_while_arranging_locks_and_refreshes_tray() {
        // `close_overlay`, and Alt+F4 on the focused overlay.
        for (input, want) in [
            (Input::Close, vec![CloseWindow, Emit, RefreshTray]),
            (Input::CloseRequested, vec![Emit, RefreshTray]),
        ] {
            for beat in [false, true] {
                let mut core = arranging();
                if beat {
                    core.step(Input::Alive {
                        instance: I,
                        gen: 1,
                    })
                    .unwrap();
                }
                // A closing window gets no window state and no ReturnFocus
                // (`Close` only closes it), so the keyboard goes wherever
                // Windows puts it.
                assert_eq!(core.step(input), Ok(want.clone()), "{input:?}");
                assert_eq!(
                    (core.status().phase, core.mode),
                    (PhaseName::Closing, Mode::Locked)
                );
                // The lease ends.
                assert!(!core.in_lease(I, 1));
                let tick = Input::LeaseTick {
                    instance: I,
                    gen: 1,
                };
                assert_eq!(core.step(tick), Ok(Vec::new()));
                assert_eq!(core.step(Input::CloseRequested), Ok(Vec::new()));
                assert_eq!(core.step(Input::Destroyed), Ok(vec![Emit, RefreshTray]));
            }
        }
        // Locked: the tray has nothing to drop.
        let mut core = painted();
        assert_eq!(core.step(Input::Close), Ok(vec![CloseWindow, Emit]));
    }

    #[test]
    fn arrange_waits_for_first_beat() {
        let mut core = painted();
        step_all(
            &mut core,
            &[Input::HideWanted(true), Input::Capturing(true)],
        );
        assert!(core.protected());
        assert_eq!(
            core.step(Input::Mode(ModeRequest::Arrange)),
            Ok(vec![
                Focus,
                Emit,
                RefreshTray,
                ArmLease {
                    instance: I,
                    gen: 1
                }
            ])
        );
        // Still click-through, and still hidden, until the page beats.
        let still = WindowState {
            ignore_cursor: true,
            protected: true,
            resizable: true,
        };
        assert_eq!(core.window(), still);
        assert!(core.effectively_locked());
        assert_eq!(
            core.step(Input::Alive {
                instance: I,
                gen: 1
            }),
            Ok(vec![ApplyWindow])
        );
        let open = WindowState {
            ignore_cursor: false,
            protected: false,
            resizable: true,
        };
        assert_eq!(core.window(), open);
        // Later beats only keep the lease.
        assert_eq!(
            core.step(Input::Alive {
                instance: I,
                gen: 1
            }),
            Ok(Vec::new())
        );
        assert_eq!(core.window(), open);
    }

    #[test]
    fn lease_relocks_after_three_silent_ticks() {
        assert_eq!(LEASE_MISSES, 3);
        let tick = Input::LeaseTick {
            instance: I,
            gen: 1,
        };
        let beat = Input::Alive {
            instance: I,
            gen: 1,
        };

        // Never beat: the page never drew its chrome.
        let mut core = arranging();
        assert_eq!(step_all(&mut core, &[tick, tick]), Vec::new());
        assert_eq!(core.mode, Mode::Arrange);
        assert_eq!(core.step(tick), Ok(LOCK_EDGE.to_vec()));
        assert_eq!(core.mode, Mode::Locked);
        assert_eq!(core.status().error.as_deref(), Some(LEASE_NEVER));
        // The lease's thread stops here.
        assert!(!core.in_lease(I, 1));

        // Beat, then stopped. A beat starts the count again.
        let mut core = arranging();
        assert_eq!(
            step_all(&mut core, &[beat, tick, tick, beat, tick, tick]),
            vec![ApplyWindow]
        );
        assert_eq!(core.mode, Mode::Arrange);
        assert!(!core.window().ignore_cursor);
        assert_eq!(core.step(tick), Ok(LOCK_EDGE.to_vec()));
        assert_eq!(core.status().error.as_deref(), Some(LEASE_LOST));
        assert!(core.window().ignore_cursor);

        // The next Arrange starts clean.
        core.step(Input::Mode(ModeRequest::Arrange)).unwrap();
        assert_eq!(core.status().error, None);
        assert_eq!(core.status().arrange_gen, 2);
    }

    #[test]
    fn stale_gen_tick_ignored() {
        // Arrange, Lock, Arrange again: the first lease's thread may tick
        // once more before it notices.
        let mut core = arranging();
        step_all(
            &mut core,
            &[
                Input::Mode(ModeRequest::Locked),
                Input::Mode(ModeRequest::Arrange),
            ],
        );
        assert_eq!(core.arrange_gen, 2);
        let before = facts(&core);
        for stale in [
            Input::LeaseTick {
                instance: I,
                gen: 1,
            },
            Input::LeaseTick {
                instance: 2,
                gen: 2,
            },
            Input::LeaseTick {
                instance: 0,
                gen: 2,
            },
        ] {
            for _ in 0..LEASE_MISSES {
                assert_eq!(core.step(stale), Ok(Vec::new()), "{stale:?}");
            }
        }
        assert_eq!(facts(&core), before);
        // Locked: even this generation's ticks do nothing.
        core.step(Input::Mode(ModeRequest::Locked)).unwrap();
        let before = facts(&core);
        for _ in 0..LEASE_MISSES {
            let tick = Input::LeaseTick {
                instance: I,
                gen: 2,
            };
            assert_eq!(core.step(tick), Ok(Vec::new()));
        }
        assert_eq!(facts(&core), before);
    }

    #[test]
    fn stale_gen_beat_ignored() {
        let mut core = arranging();
        step_all(
            &mut core,
            &[
                Input::Mode(ModeRequest::Locked),
                Input::Mode(ModeRequest::Arrange),
                Input::LeaseTick {
                    instance: I,
                    gen: 2,
                },
            ],
        );
        // A beat from the first Arrange's page, or from another window's.
        for stale in [
            Input::Alive {
                instance: I,
                gen: 1,
            },
            Input::Alive {
                instance: 2,
                gen: 2,
            },
        ] {
            assert_eq!(core.step(stale), Ok(Vec::new()), "{stale:?}");
            assert!(core.window().ignore_cursor, "{stale:?}");
            // ... and it doesn't hold the lease either.
            assert_eq!(core.missed, 1, "{stale:?}");
        }
        assert_eq!(
            core.step(Input::Alive {
                instance: I,
                gen: 2
            }),
            Ok(vec![ApplyWindow])
        );
        assert!(!core.window().ignore_cursor);
    }

    #[test]
    fn focus_and_return_focus_only_on_edges() {
        let focus = |effects: &[Effect]| {
            (
                effects.iter().filter(|&&e| e == Focus).count(),
                effects.iter().filter(|&&e| e == ReturnFocus).count(),
            )
        };
        // A whole life without Arrange: no focus effects.
        let mut core = OverlayCore::new(true);
        let life = step_all(
            &mut core,
            &[
                Input::Open { fit: false },
                Input::HideWanted(true),
                Input::Built { instance: I },
                Input::Ready { instance: I },
                Input::Painted { instance: I },
                Input::Placed { fit: true },
                Input::Capturing(true),
                Input::Ready { instance: I },
                Input::Painted { instance: I },
            ],
        );
        assert_eq!(focus(&life), (0, 0));
        // Locked → Locked: nothing to hand back.
        assert_eq!(core.step(Input::Mode(ModeRequest::Locked)), Ok(Vec::new()));
        // Locked → Arrange: focus, once.
        let e = core.step(Input::Mode(ModeRequest::Toggle)).unwrap();
        assert_eq!(focus(&e), (1, 0));
        // Arrange → Arrange, and the first beat: nothing.
        assert_eq!(core.step(Input::Mode(ModeRequest::Arrange)), Ok(Vec::new()));
        let e = core
            .step(Input::Alive {
                instance: I,
                gen: 1,
            })
            .unwrap();
        assert_eq!(focus(&e), (0, 0));
        // Arrange → Locked: handed back, once.
        let e = core.step(Input::Mode(ModeRequest::Toggle)).unwrap();
        assert_eq!(focus(&e), (0, 1));
        // A second Lock (Esc, then the button): nothing more.
        assert_eq!(core.step(Input::Mode(ModeRequest::Locked)), Ok(Vec::new()));
        // Closing while arranging: nothing either.
        core.step(Input::Mode(ModeRequest::Arrange)).unwrap();
        let e = step_all(&mut core, &[Input::Close, Input::Destroyed]);
        assert_eq!(focus(&e), (0, 0));
    }

    /// §7.3 for every hide toggle × offered × mode × beat × capturing ×
    /// phase, on real cores. Creating is also tried ready early and
    /// cancelled.
    #[test]
    fn protected_partition_all_192() {
        let phases = [
            Phase::Closed,
            Phase::Creating {
                instance: I,
                ready_early: false,
                cancel: false,
            },
            Phase::Creating {
                instance: I,
                ready_early: true,
                cancel: false,
            },
            Phase::Creating {
                instance: I,
                ready_early: false,
                cancel: true,
            },
            Phase::Starting { instance: I },
            Phase::Shown { instance: I },
            Phase::Painted { instance: I },
            Phase::Closing { instance: I },
        ];
        let (mut cases, mut yes) = (0, 0);
        for wanted in [false, true] {
            for available in [false, true] {
                for mode in [Mode::Locked, Mode::Arrange] {
                    for arranged in [false, true] {
                        for capturing in [false, true] {
                            for phase in phases {
                                let mut core = OverlayCore::new(available);
                                core.hide_wanted = wanted;
                                core.mode = mode;
                                core.arranged = arranged;
                                core.capturing = capturing;
                                core.phase = phase;
                                // The table's rows: off, not offered, Arrange
                                // after a beat, nothing capturing, or not
                                // shown → no.
                                let beat = mode == Mode::Arrange && arranged;
                                let shown = matches!(
                                    phase,
                                    Phase::Shown { .. }
                                        | Phase::Painted { .. }
                                        | Phase::Closing { .. }
                                );
                                let want = wanted && available && !beat && capturing && shown;
                                let case = format!("{core:?}");
                                assert_eq!(core.protected(), want, "{case}");
                                assert_eq!(core.window().protected, want, "{case}");
                                cases += 1;
                                yes += usize::from(want);
                            }
                        }
                    }
                }
            }
        }
        // 192 combinations over the six phases, 64 more for Creating's
        // flags; yes only in 3 phases × 3 effectively-locked mode/beat pairs.
        assert_eq!((cases, yes), (192 + 64, 9));
    }

    #[test]
    fn reconcile_plan_only_while_live() {
        // phase, has_window, live, instance
        let rows = [
            (Phase::Closed, false, false, None),
            (
                Phase::Creating {
                    instance: I,
                    ready_early: true,
                    cancel: false,
                },
                false,
                false,
                Some(I),
            ),
            (Phase::Starting { instance: I }, true, true, Some(I)),
            (Phase::Shown { instance: I }, true, true, Some(I)),
            (Phase::Painted { instance: I }, true, true, Some(I)),
            (Phase::Closing { instance: I }, true, false, Some(I)),
        ];
        for (phase, has_window, live, instance) in rows {
            let mut core = OverlayCore::new(true);
            core.phase = phase;
            // Protection due, so a plan in Closing would carry it.
            core.hide_wanted = true;
            core.capturing = true;
            assert_eq!(core.has_window(), has_window, "{phase:?}");
            assert_eq!(core.live(), live, "{phase:?}");
            assert_eq!(core.instance(), instance, "{phase:?}");
            for rung in [0, 1] {
                let plan = live.then(|| setter_plan(core.window(), rung));
                assert_eq!(core.reconcile_plan(rung), plan, "{phase:?} rung {rung}");
            }
        }
    }

    #[test]
    fn setter_plan_orders() {
        use Setter::{IgnoreCursor, LayerOpaque, Protect, Resizable};
        for resizable in [false, true] {
            for protected in [false, true] {
                // Locked: click-through first, the affinity last, after
                // rung 1's layered attributes and the resizable style.
                let locked = WindowState {
                    ignore_cursor: true,
                    protected,
                    resizable,
                };
                assert_eq!(
                    setter_plan(locked, 0),
                    vec![IgnoreCursor(true), Resizable(resizable), Protect(protected)]
                );
                assert_eq!(
                    setter_plan(locked, 1),
                    vec![
                        IgnoreCursor(true),
                        LayerOpaque,
                        Resizable(resizable),
                        Protect(protected)
                    ]
                );
                // Arrange: the affinity off first, while still layered.
                let arrange = WindowState {
                    ignore_cursor: false,
                    protected,
                    resizable,
                };
                for rung in [0, 1] {
                    assert_eq!(
                        setter_plan(arrange, rung),
                        vec![Protect(false), IgnoreCursor(false), Resizable(resizable)],
                        "rung {rung}"
                    );
                }
            }
        }
    }

    #[test]
    fn affinity_call_table() {
        // shown, want, applied → the call
        let rows = [
            (false, false, false, None),
            (false, false, true, None),
            (false, true, false, None),
            (false, true, true, None),
            // WDA_NONE never reaches a window that was never protected.
            (true, false, false, None),
            (true, false, true, Some(false)),
            (true, true, false, Some(true)),
            (true, true, true, None),
        ];
        for (shown, want, applied, call) in rows {
            assert_eq!(
                affinity_call(want, shown, applied),
                call,
                "shown {shown}, want {want}, applied {applied}"
            );
        }
    }

    #[test]
    fn should_return_focus_table() {
        const OURS: isize = 0x10;
        const GAME: isize = 0x20;
        const MAIN: isize = 0x30;
        const TRAY: isize = 0x40;
        // fg, ours, saved, saved_ok → hand it back
        let rows = [
            // The hotkey in a game, Arrange, then Esc.
            (OURS, OURS, GAME, true, true),
            // Settings' Arrange, then Esc.
            (OURS, OURS, MAIN, true, true),
            // Settings' Lock: main has the foreground.
            (MAIN, OURS, MAIN, true, false),
            // The tray's Lock: the tray's hidden window has it.
            (TRAY, OURS, GAME, true, false),
            // The game was closed meanwhile.
            (OURS, OURS, GAME, false, false),
            // Nothing saved.
            (OURS, OURS, 0, true, false),
            (OURS, OURS, 0, false, false),
            // Saved is the overlay itself.
            (OURS, OURS, OURS, true, false),
            // No overlay window.
            (0, 0, GAME, true, false),
            (GAME, 0, GAME, true, false),
        ];
        for (fg, ours, saved, saved_ok, want) in rows {
            assert_eq!(
                should_return_focus(fg, ours, saved, saved_ok),
                want,
                "fg {fg}, ours {ours}, saved {saved}, ok {saved_ok}"
            );
        }
    }

    #[test]
    fn hotkey_overlay_request_table() {
        assert_eq!(HOTKEY_ARRANGE, "overlay:arrange");
        let rows = [
            ("overlay:arrange", true, Some(ModeRequest::Toggle)),
            // Releases arrive from global-hotkey's polling thread.
            ("overlay:arrange", false, None),
            ("replay:save", true, None),
            ("sb:x", true, None),
            ("", true, None),
            ("overlay:arrange ", true, None),
            ("Overlay:arrange", true, None),
            ("overlay:", true, None),
        ];
        for (id, pressed, want) in rows {
            assert_eq!(
                hotkey_overlay_request(id, pressed),
                want,
                "{id:?} {pressed}"
            );
        }
    }

    #[test]
    fn placement_settled_cases() {
        // On a monitor left of the primary.
        let want = OverlayRect {
            x: -1920,
            y: 40,
            width: 480,
            height: 360,
        };
        assert!(placement_settled(want, (-1920, 40), (480, 360)));
        for (pos, size) in [
            ((-1919, 40), (480, 360)),
            ((-1920, 41), (480, 360)),
            ((-1920, 40), (481, 360)),
            ((-1920, 40), (480, 359)),
            // Rescaled by WM_DPICHANGED.
            ((-1920, 40), (720, 540)),
            ((40, -1920), (360, 480)),
        ] {
            assert!(!placement_settled(want, pos, size), "{pos:?} {size:?}");
        }
    }

    /// A seeded linear congruential generator: the same walks every run.
    struct Lcg(u64);

    impl Lcg {
        fn below(&mut self, n: usize) -> usize {
            self.0 = self
                .0
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            usize::try_from(self.0 >> 33).unwrap() % n
        }
    }

    /// A random input, mostly for the current window and Arrange, sometimes
    /// for the one before or after. Weighted so walks reach Arrange and
    /// both of the lease's relocks often.
    fn random_input(rng: &mut Lcg, core: &OverlayCore) -> Input {
        let near =
            |rng: &mut Lcg, n: u32| [n, n, n, n.wrapping_sub(1), n.wrapping_add(1)][rng.below(5)];
        let instance = near(rng, core.last_instance);
        let gen = near(rng, core.arrange_gen);
        let on = rng.below(2) == 0;
        if core.mode == Mode::Arrange && on {
            return if rng.below(3) == 0 {
                Input::Alive { instance, gen }
            } else {
                Input::LeaseTick { instance, gen }
            };
        }
        match rng.below(32) {
            0 | 1 => Input::Open { fit: on },
            2 | 3 => Input::Built { instance },
            4 => Input::BuildFailed { instance },
            5 | 6 => Input::Ready { instance },
            7 | 8 => Input::Painted { instance },
            9 => Input::Watchdog { instance },
            10 => Input::Close,
            11 => Input::CloseRequested,
            12 => Input::Destroyed,
            13 => Input::Mode(ModeRequest::Locked),
            14 => Input::Mode(ModeRequest::Arrange),
            15 | 16 => Input::Mode(ModeRequest::Toggle),
            17..=19 => Input::Alive { instance, gen },
            20..=25 => Input::LeaseTick { instance, gen },
            26 | 27 => Input::HideWanted(on),
            28 | 29 => Input::Capturing(on),
            _ => Input::Placed { fit: on },
        }
    }

    /// How often the walks got somewhere, so they can't pass by staying in
    /// Closed.
    #[derive(Debug, Default)]
    struct Reached {
        shows: usize,
        focus: usize,
        return_focus: usize,
        watchdog: usize,
        lease_never: usize,
        lease_lost: usize,
        protected: usize,
        clicks_taken: usize,
        refusals: BTreeSet<String>,
    }

    /// One step of a walk: the input, the core before and after, and the
    /// answer.
    struct Walked<'a> {
        input: Input,
        before: &'a OverlayCore,
        after: &'a OverlayCore,
        result: &'a Result<Vec<Effect>, Refusal>,
        case: &'a str,
    }

    /// A refusal, or an input for another window or another Arrange,
    /// changes nothing.
    fn check_nothing_stale(s: &Walked<'_>, reached: &mut Reached) {
        let (before, case) = (s.before, s.case);
        let stale = match s.input {
            Input::Built { instance }
            | Input::BuildFailed { instance }
            | Input::Ready { instance }
            | Input::Painted { instance }
            | Input::Watchdog { instance } => before.instance() != Some(instance),
            Input::Alive { instance, gen } | Input::LeaseTick { instance, gen } => {
                before.phase != (Phase::Painted { instance })
                    || before.mode != Mode::Arrange
                    || before.arrange_gen != gen
            }
            _ => false,
        };
        if stale {
            assert_eq!(*s.result, Ok(Vec::new()), "{case}");
        }
        if let Err(refusal) = s.result {
            reached.refusals.insert(format!("{refusal:?}"));
        }
        if stale || s.result.is_err() {
            assert_eq!(facts(s.after), facts(before), "{case}");
        }
    }

    /// The invariants on the state.
    fn check_state(s: &Walked<'_>, reached: &mut Reached) {
        let (after, case) = (s.after, s.case);
        // Arrange only while painted; a beat only in Arrange.
        if after.mode == Mode::Arrange {
            assert!(matches!(after.phase, Phase::Painted { .. }), "{case}");
        }
        if after.arranged {
            assert_eq!(after.mode, Mode::Arrange, "{case}");
        }
        // Closed: Locked, not fitted, no error.
        if after.phase == Phase::Closed {
            assert_eq!(
                (after.mode, after.fit, after.error),
                (Mode::Locked, false, None),
                "{case}"
            );
        }
        // Clicks are taken only in Arrange, after a beat, inside the lease,
        // and only a beat starts taking them.
        if !after.window().ignore_cursor {
            reached.clicks_taken += 1;
            assert!(
                after.mode == Mode::Arrange && after.arranged && after.missed < LEASE_MISSES,
                "{case}"
            );
            if s.before.window().ignore_cursor {
                assert!(matches!(s.input, Input::Alive { .. }), "{case}");
            }
        }
        // Protection only for a shown window.
        if after.protected() {
            reached.protected += 1;
            assert!(
                matches!(
                    after.phase,
                    Phase::Shown { .. } | Phase::Painted { .. } | Phase::Closing { .. }
                ),
                "{case}"
            );
        }
    }

    /// The invariants on the effects.
    fn check_effects(s: &Walked<'_>, effects: &[Effect], reached: &mut Reached) {
        let (before, after, case) = (s.before, s.after, s.case);
        // Focus only on Locked → Arrange, with a lease; ReturnFocus only on
        // Arrange → Locked in Painted, or the reload to Shown.
        let entered = before.mode == Mode::Locked && after.mode == Mode::Arrange;
        let left = before.mode == Mode::Arrange
            && after.mode == Mode::Locked
            && matches!(after.phase, Phase::Painted { .. } | Phase::Shown { .. });
        assert_eq!(effects.contains(&Focus), entered, "{case}");
        assert_eq!(effects.contains(&ReturnFocus), left, "{case}");
        let lease = ArmLease {
            instance: after.last_instance,
            gen: after.arrange_gen,
        };
        assert_eq!(effects.contains(&lease), entered, "{case}");
        reached.focus += usize::from(entered);
        reached.return_focus += usize::from(left);
        // Beats never cause status traffic.
        if matches!(s.input, Input::Alive { .. }) {
            assert!(!effects.contains(&Emit), "{case}");
        }
        // Every status change is told; every mode change reaches the tray.
        if after.status() != before.status() {
            assert!(effects.contains(&Emit), "{case}");
        }
        if after.mode != before.mode {
            assert!(effects.contains(&RefreshTray), "{case}");
        }
        if matches!(s.input, Input::LeaseTick { .. }) && effects == LOCK_EDGE {
            match after.error {
                Some(LEASE_NEVER) => reached.lease_never += 1,
                Some(LEASE_LOST) => reached.lease_lost += 1,
                other => panic!("relocked with {other:?}: {case}"),
            }
        }
    }

    /// §7.1's invariants, over 50 seeded walks of 10,000 inputs each.
    #[test]
    fn invariants_hold_on_random_walks() {
        let mut reached = Reached::default();
        for seed in 0..50 {
            let mut rng = Lcg(seed);
            let mut core = OverlayCore::new(seed % 4 != 0);
            // Instances whose Built applied the window, and that were shown.
            let mut applied = HashSet::new();
            let mut shown = HashSet::new();
            for n in 0..10_000 {
                let input = random_input(&mut rng, &core);
                let before = core.clone();
                let result = core.step(input);
                let case = format!("seed {seed} step {n}: {input:?} on {before:?} → {core:?}");
                let s = Walked {
                    input,
                    before: &before,
                    after: &core,
                    result: &result,
                    case: &case,
                };
                let effects = result.clone().unwrap_or_default();
                check_nothing_stale(&s, &mut reached);
                check_state(&s, &mut reached);
                check_effects(&s, &effects, &mut reached);
                // Shown at most once per window, and only after its Built
                // applied click-through.
                for effect in &effects {
                    let i = core.instance();
                    match effect {
                        ApplyWindow if matches!(input, Input::Built { .. }) => {
                            applied.insert(i.unwrap());
                        }
                        Show => {
                            let i = i.unwrap();
                            assert_eq!(core.phase, Phase::Shown { instance: i }, "{case}");
                            assert!(applied.contains(&i), "shown before applied: {case}");
                            assert!(shown.insert(i), "shown twice: {case}");
                            reached.shows += 1;
                        }
                        DestroyWindow => reached.watchdog += 1,
                        _ => {}
                    }
                }
            }
        }
        let all = ["Closing", "NotOpen", "Starting"].map(str::to_owned);
        assert!(
            reached.shows > 0
                && reached.focus > 0
                && reached.return_focus > 0
                && reached.watchdog > 0
                && reached.lease_never > 0
                && reached.lease_lost > 0
                && reached.protected > 0
                && reached.clicks_taken > 0
                && reached.refusals == BTreeSet::from(all),
            "{reached:?}"
        );
    }
}
