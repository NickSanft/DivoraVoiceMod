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
//
// v1.56.0: the status says Muted to an audience that relies on it, so the
// pushes go out in order (one emit at a time, as `syncOverlayScene` does),
// and none goes out until the main window knows the mute or stops waiting
// for it (`held`; at worst 500 ms).

import { batch, createEffect, createSignal, onCleanup } from "solid-js";
import {
  OVERLAY_EVENT,
  OVERLAY_READY_EVENT,
  OVERLAY_SCENE_EVENT,
  type OverlayState,
  type ScenePush,
} from "./state";

export interface OverlaySyncDeps {
  /** Whether the overlay window is open. Nothing is sent while it isn't. */
  isOpen: () => boolean;
  /** The state to send. Read only while open, so it is tracked only then. */
  payload: () => OverlayState;
  /** v1.56.0: while true nothing is sent; when it turns false, the current
   *  state goes once. App.tsx holds until the store knows the mute, or its
   *  read of it fails, or 500 ms pass, so the hold always ends; after a
   *  failure or the 500 ms, what goes may say not muted. Absent = never held. */
  held?: () => boolean;
  emit: (event: string, payload: OverlayState) => Promise<void>;
  listen: (event: string, handler: () => void) => Promise<() => void>;
}

/** Keep the overlay in step with the main window: send the state whenever it
 *  changes, and again whenever the overlay says it is ready to hear it. Call
 *  inside a reactive owner (a component, or `createRoot` in tests); the
 *  listener is removed when that owner is disposed.
 *
 *  Each Tauri `emit` is its own async command, so two sent back to back can
 *  arrive in either order, and the overlay shows whichever lands last. Every
 *  emit here waits for the one before it to finish. */
export function syncOverlay(deps: OverlaySyncDeps): void {
  const [readyCount, setReadyCount] = createSignal(0);
  let tail: Promise<void> = Promise.resolve();

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
    if (deps.held?.() === true || !deps.isOpen()) return;
    const payload = deps.payload();
    tail = tail
      .then(() => deps.emit(OVERLAY_EVENT, payload))
      .catch(() => {
        /* not under Tauri; the next push still goes */
      });
  });
}

export interface SceneSyncDeps {
  /** Whether the overlay window is open. Nothing is sent while it isn't. */
  isOpen: () => boolean;
  /** The push to send. Read only while open, so it is tracked only then. */
  push: () => ScenePush;
  emit: (event: string, payload: ScenePush) => Promise<void>;
  listen: (event: string, handler: () => void) => Promise<() => void>;
  /** Called on `overlay:ready`, before the re-push: a reloaded page numbers
   *  its edits from 0 again, so main forgets its ack. */
  onReady: () => void;
}

/** v1.55.0: keep the overlay's scene in step, as `syncOverlay` does its
 *  state: send the push whenever it changes, and again on `overlay:ready`.
 *
 *  Each Tauri `emit` is its own async command, so two sent back to back can
 *  arrive in either order and an older scene would win. Every emit here
 *  waits for the one before it to finish; `flushed()` resolves once
 *  everything queued so far has gone. Call inside a reactive owner; the
 *  listener goes when that owner does, or on `dispose()`. */
export function syncOverlayScene(deps: SceneSyncDeps): { dispose: () => void; flushed: () => Promise<void> } {
  const [readyCount, setReadyCount] = createSignal(0);
  let tail: Promise<void> = Promise.resolve();
  let unlisten: (() => void) | null = null;
  let disposed = false;
  const dispose = (): void => {
    if (disposed) return;
    disposed = true;
    unlisten?.();
    unlisten = null;
  };

  void deps
    .listen(OVERLAY_READY_EVENT, () => {
      if (disposed) return;
      // One re-push, with the ack already reset.
      batch(() => {
        deps.onReady();
        setReadyCount((n) => n + 1);
      });
    })
    .then((fn) => {
      if (disposed) fn();
      else unlisten = fn;
    })
    .catch(() => {
      /* not under Tauri */
    });
  onCleanup(dispose);

  createEffect(() => {
    readyCount();
    if (disposed || !deps.isOpen()) return;
    const payload = deps.push();
    tail = tail
      .then(() => deps.emit(OVERLAY_SCENE_EVENT, payload))
      .catch(() => {
        /* not under Tauri; the next push still goes */
      });
  });

  return { dispose, flushed: () => tail };
}
