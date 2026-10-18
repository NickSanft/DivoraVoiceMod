// v1.55.0: the stream overlay's status, as the backend reports it.
//
// The backend owns the overlay's lifecycle and mode
// (src-tauri/src/overlay_machine.rs). It sends the whole status on
// `overlay:status` after every change, and every overlay window command
// returns the same shape, so both windows read it from here. The overlay
// page never imports src/audio/api.ts, so the commands it calls are named
// here too.

/** `OverlayStatus` in overlay_machine.rs; its keys are frozen in
 *  wire.fixture.json. */
export interface OverlayStatus {
  /** A window exists (any phase but closed). */
  open: boolean;
  phase: "closed" | "starting" | "shown" | "painted" | "closing";
  /** The current or last window's number; 0 before any. */
  instance: number;
  mode: "locked" | "arrange";
  /** Bumped on every entry to Arrange; the page echoes it in `overlay_alive`. */
  arrangeGen: number;
  /** The phase is painted. */
  canArrange: boolean;
  /** The window was placed fitted to its monitor. */
  fit: boolean;
  /** The Hide from recordings toggle, as the backend holds it. */
  hideWanted: boolean;
  /** Windows can leave the overlay out of captures. */
  hideAvailable: boolean;
  /** A take is running, or Replay is on in any state but off. */
  capturing: boolean;
  /** Why this window is closing abnormally (the watchdog; null once it is
   *  destroyed or another opens), or why the lease ended Arrange (null
   *  again at the next Arrange, open or close). */
  error: string | null;
}

/** What `set_overlay_mode` takes. Locking never fails. */
export type OverlayModeRequest = "locked" | "arrange" | "toggle";

/** What `overlay_handshake` takes: the page is listening, or has drawn
 *  since it was shown. */
export type HandshakeStage = "ready" | "painted";

export const SET_OVERLAY_MODE_CMD = "set_overlay_mode";
export const HANDSHAKE_CMD = "overlay_handshake";
export const ALIVE_CMD = "overlay_alive";

/** Set on the overlay page by an init script when the backend runs with
 *  `DIVORA_PRETEND_OVERLAY_NO_READY` (debug only): the page then skips its
 *  ready handshake, so the watchdog can be checked. */
export const NO_READY_GLOBAL = "__DIVORA_OVERLAY_NO_READY__";

const PHASES: readonly string[] = ["closed", "starting", "shown", "painted", "closing"];
const MODES: readonly string[] = ["locked", "arrange"];

function count(n: unknown): n is number {
  return typeof n === "number" && Number.isInteger(n) && n >= 0;
}

/** A status off the wire, with every key checked; null when any is missing
 *  or the wrong type (the e2e mock answers `null`). Keys a later backend
 *  adds are left out. */
export function readOverlayStatus(raw: unknown): OverlayStatus | null {
  if (typeof raw !== "object" || raw === null) return null;
  const o = raw as Record<string, unknown>;
  const flags = [o.open, o.canArrange, o.fit, o.hideWanted, o.hideAvailable, o.capturing];
  if (!flags.every((b) => typeof b === "boolean")) return null;
  if (typeof o.phase !== "string" || !PHASES.includes(o.phase)) return null;
  if (typeof o.mode !== "string" || !MODES.includes(o.mode)) return null;
  if (!count(o.instance) || !count(o.arrangeGen)) return null;
  if (o.error !== null && typeof o.error !== "string") return null;
  return {
    open: o.open as boolean,
    phase: o.phase as OverlayStatus["phase"],
    instance: o.instance,
    mode: o.mode as OverlayStatus["mode"],
    arrangeGen: o.arrangeGen,
    canArrange: o.canArrange as boolean,
    fit: o.fit as boolean,
    hideWanted: o.hideWanted as boolean,
    hideAvailable: o.hideAvailable as boolean,
    capturing: o.capturing as boolean,
    error: o.error,
  };
}

async function invokeCmd(cmd: string, args: Record<string, unknown>): Promise<unknown> {
  const { invoke } = await import("@tauri-apps/api/core");
  return invoke(cmd, args);
}

/** Lock the overlay (Esc, the chrome's Lock button). A failure, such as a
 *  capability refusing the command, is logged, never thrown. */
export async function requestLock(): Promise<void> {
  try {
    await invokeCmd(SET_OVERLAY_MODE_CMD, { mode: "locked" });
  } catch (e) {
    console.warn("overlay: set_overlay_mode(locked) failed", e);
  }
}

/** Tell the backend the page is listening (`ready`) or has drawn since it
 *  was shown (`painted`). Retried once; each failure is logged; never
 *  throws. */
export async function sendHandshake(instance: number, stage: HandshakeStage): Promise<void> {
  for (let attempt = 0; attempt < 2; attempt += 1) {
    try {
      await invokeCmd(HANDSHAKE_CMD, { instance, stage });
      return;
    } catch (e) {
      console.warn(`overlay: overlay_handshake(${stage}) failed`, e);
    }
  }
}

let beatWarned: string | null = null;

/** One Arrange beat: the page is alive, so the backend may keep the overlay
 *  clickable. False when the call failed; never throws. A failure is logged
 *  once per arrange generation, and only under Tauri, so the browser
 *  preview stays quiet. */
export async function sendAlive(instance: number, gen: number): Promise<boolean> {
  try {
    await invokeCmd(ALIVE_CMD, { instance, gen });
    return true;
  } catch (e) {
    const key = `${instance}:${gen}`;
    if (beatWarned !== key && typeof window !== "undefined" && "__TAURI_INTERNALS__" in window) {
      beatWarned = key;
      console.warn("overlay: overlay_alive failed", e);
    }
    return false;
  }
}

/** The status before any has arrived: closed. */
export function closedStatus(): OverlayStatus {
  return {
    open: false,
    phase: "closed",
    instance: 0,
    mode: "locked",
    arrangeGen: 0,
    canArrange: false,
    fit: false,
    hideWanted: false,
    hideAvailable: false,
    capturing: false,
    error: null,
  };
}
