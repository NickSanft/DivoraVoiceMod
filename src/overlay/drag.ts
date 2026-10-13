// v1.51.3: which presses on the overlay move its window.
//
// Tauri's own `data-tauri-drag-region` would also do this, but its script
// toggles maximise on a double-click whenever the window is resizable
// (tauri 2.11.2 `window/scripts/drag.js` → `internal_toggle_maximize`, which
// checks `is_resizable` and nothing else). The overlay is resizable, so a
// double-click on the circle would blow a transparent always-on-top window
// up to the whole screen. This handler only ever starts a drag.

/** Elements that keep their own click; pressing them never moves the window. */
const INTERACTIVE = "button, a, input, select, textarea, [role='button']";

export interface PressLike {
  button: number;
  /** The click count: 1 for a single press, 2 for the second of a double. */
  detail: number;
  target: EventTarget | null;
}

/** True for a plain left press on anything that isn't interactive. */
export function startsWindowDrag(e: PressLike): boolean {
  if (e.button !== 0 || e.detail !== 1) return false;
  const t = e.target;
  if (typeof Element === "undefined" || !(t instanceof Element)) return false;
  return t.closest(INTERACTIVE) === null;
}
