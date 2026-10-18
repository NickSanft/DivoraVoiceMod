// v1.16.0: shared contract between the main window (emitter) and the
// stream-overlay window (consumer). The overlay is a separate Tauri
// WebviewWindow with its own JS context, so it can't read the store — the
// main window pushes the slice it needs over the Tauri event bus.

import type { ChainEntry, TweaksState, VoiceStatus } from "../types";
import type { OverlayScene, SceneOp, Size } from "./scene";

/** The Tauri event the main window emits and the overlay listens for. */
export const OVERLAY_EVENT = "overlay:state";

/** v1.51.3: the overlay emits this (no payload) once its `overlay:state`
 *  listener is registered; the main window answers with the current state.
 *  Without it the push sent when the window opened could arrive before the
 *  overlay's page was listening. */
export const OVERLAY_READY_EVENT = "overlay:ready";

/** v1.55.0: the backend sends the whole `OverlayStatus` (status.ts) on this
 *  after every change, to both windows. */
export const OVERLAY_STATUS_EVENT = "overlay:status";

/** v1.55.0: main → overlay, the scene to draw (`ScenePush`). Its own event,
 *  not part of `overlay:state`, so a status flip never snaps a dragged
 *  picture back. */
export const OVERLAY_SCENE_EVENT = "overlay:scene";

/** v1.55.0: overlay → main, one finished edit (`SceneEdit`). */
export const OVERLAY_EDIT_EVENT = "overlay:edit";

/** Background mode for the overlay window's content. The window itself is
 *  always created transparent; this controls what sits behind the circle:
 *  nothing (alpha, for OBS Window Capture with transparency) or a solid
 *  chroma colour the streamer keys out. */
export type OverlayBg = "transparent" | "green" | "magenta";

/** CSS colour for each chroma mode (keyed-out in OBS). */
export const OVERLAY_BG_COLOR: Record<OverlayBg, string> = {
  transparent: "transparent",
  green: "#00b140", // standard chroma green
  magenta: "#ff00ff",
};

/** Everything the overlay needs to mirror the Mixer's spell circle. */
export interface OverlayState {
  chain: ChainEntry[];
  status: VoiceStatus;
  motion: number;
  mystical: number;
  mood: TweaksState["mood"];
  accent: TweaksState["accent"];
  theme: TweaksState["theme"];
  bg: OverlayBg;
}

/** v1.55.0: the `overlay:scene` payload. */
export interface ScenePush {
  /** Main's page load: a new one means main reloaded, and the overlay drops
   *  the edits it was waiting on. */
  session: string;
  /** Bumped on every scene change; the overlay never goes back to a lower
   *  one within a session. */
  revision: number;
  scene: OverlayScene;
  /** The overlay's latest edit this push includes; null in the push that
   *  answers `overlay:ready`. */
  ack: { instance: number; seq: number } | null;
  /** Set only by a Fit change: keep drawing the previous scene until the
   *  viewport is this size, for at most a second. */
  expectViewport: Size | null;
}

/** v1.55.0: the `overlay:edit` payload. */
export interface SceneEdit {
  /** The overlay window's number (`OverlayStatus.instance`). */
  instance: number;
  /** Numbers this window's edits from 0, in the order sent. */
  seq: number;
  op: SceneOp;
}

/** Bundle the live store values into an overlay payload (pure, testable). */
export function overlayPayload(input: {
  chain: ChainEntry[];
  status: VoiceStatus;
  motion: number;
  mystical: number;
  mood: TweaksState["mood"];
  accent: TweaksState["accent"];
  theme: TweaksState["theme"];
  bg: OverlayBg;
}): OverlayState {
  return {
    chain: input.chain,
    status: input.status,
    motion: input.motion,
    mystical: input.mystical,
    mood: input.mood,
    accent: input.accent,
    theme: input.theme,
    bg: input.bg,
  };
}
