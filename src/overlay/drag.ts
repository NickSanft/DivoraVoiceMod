// v1.51.3: which presses on the overlay move its window.
//
// Tauri's own `data-tauri-drag-region` would also do this, but its script
// toggles maximise on a double-click whenever the window is resizable
// (tauri 2.11.2 `window/scripts/drag.js` → `internal_toggle_maximize`, which
// checks `is_resizable` and nothing else). The overlay is resizable, so a
// double-click on the circle would blow a transparent always-on-top window
// up to the whole screen. This handler only ever starts a drag.

/** Elements that keep their own click; pressing them never moves the window.
 *  v1.55.0: charms, their handles and the Arrange chrome too (the chrome's
 *  Move window grip starts its own drag). */
const INTERACTIVE = "button, a, input, select, textarea, [role='button'], [data-charm], [data-handle], [data-chrome]";

export interface PressLike {
  button: number;
  /** The click count: 1 for a single press, 2 for the second of a double. */
  detail: number;
  target: EventTarget | null;
}

/** v1.55.0: a single press of the left button — the only press that moves
 *  the window (the second press of a double-click never does). */
export function isPlainLeftPress(e: { button: number; detail: number }): boolean {
  return e.button === 0 && e.detail === 1;
}

/** True for a plain left press on anything that isn't interactive. */
export function startsWindowDrag(e: PressLike): boolean {
  if (!isPlainLeftPress(e)) return false;
  const t = e.target;
  if (typeof Element === "undefined" || !(t instanceof Element)) return false;
  return t.closest(INTERACTIVE) === null;
}
