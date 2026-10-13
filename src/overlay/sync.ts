// v1.51.3: the main window's side of the overlay link.
//
// v1.16.0 pushed `overlay:state` from an effect that re-ran on every change,
// including the moment the overlay opened. But "opened" meant the native
// window existed (`tauri://created`), which is before the overlay's page has
// loaded and registered its listener — so that first push landed on nobody,
// and the overlay showed defaults until the next change. Now the overlay
// emits `overlay:ready` once it is listening, and that re-sends the current
// state. Taking `emit`/`listen` as arguments lets the test drive the same
// function App.tsx calls, rather than a copy of it.

import { createEffect, createSignal, onCleanup } from "solid-js";
import { OVERLAY_EVENT, OVERLAY_READY_EVENT, type OverlayState } from "./state";

export interface OverlaySyncDeps {
  /** Whether the overlay window is open. Nothing is sent while it isn't. */
  isOpen: () => boolean;
  /** The state to send. Read only while open, so it is tracked only then. */
  payload: () => OverlayState;
  emit: (event: string, payload: OverlayState) => Promise<void>;
  listen: (event: string, handler: () => void) => Promise<() => void>;
}

/** Keep the overlay in step with the main window: send the state whenever it
 *  changes, and again whenever the overlay says it is ready to hear it. Call
 *  inside a reactive owner (a component, or `createRoot` in tests); the
 *  listener is removed when that owner is disposed. */
export function syncOverlay(deps: OverlaySyncDeps): void {
  const [readyCount, setReadyCount] = createSignal(0);

  let unlisten: (() => void) | null = null;
  let disposed = false;
  void deps
    .listen(OVERLAY_READY_EVENT, () => setReadyCount((n) => n + 1))
    .then((fn) => {
      if (disposed) fn();
      else unlisten = fn;
    })
    .catch(() => {
      /* not under Tauri */
    });
  onCleanup(() => {
    disposed = true;
    unlisten?.();
  });

  createEffect(() => {
    readyCount();
    if (!deps.isOpen()) return;
    const payload = deps.payload();
    void deps.emit(OVERLAY_EVENT, payload).catch(() => {
      /* not under Tauri */
    });
  });
}
