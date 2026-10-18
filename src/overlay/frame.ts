// v1.55.0: where the overlay window goes when it opens or Fit changes, and
// whether the scene has to be re-expressed for it first.
//
// The scene records which frame its `designSize` was measured against: the
// window, or the whole monitor while Fit to monitor is on. When the frame
// the window is about to get differs from the scene's (Fit was turned on or
// off, perhaps while the overlay was closed), main commits a `reframe` from
// the old rectangle to the new one before placing the window, so nothing
// moves on screen. Pure: main reads the monitors and passes them in.

import {
  canvasRects,
  fitRect,
  pickMonitor,
  placeOverlay,
  type MonitorInfo,
  type OverlayGeometry,
  type Rect,
} from "./geometry";
import { frameOf, type Frame, type OverlayScene, type SceneOp, type Size } from "./scene";

/** The monitors as main read them; null when they couldn't be read. */
export interface MonitorsRead {
  monitors: MonitorInfo[];
  primary: string | null;
}

export interface FramePlan {
  /** Where to put the window (physical px); null leaves it to Windows. */
  rect: Rect | null;
  /** Whether that rect is the monitor's: what `open_overlay` and
   *  `place_overlay` are told, never the stored flag. */
  fit: boolean;
  /** Commit this before placing, when the scene's frame differs. */
  reframe: SceneOp | null;
  /** The window's CSS size once placed, for `expectViewport`. */
  viewport: Size | null;
}

/** A window never moved has no saved place: stand in a 480-logical square
 *  at the monitor's top-left. It only affects a window that was never
 *  moved. */
function standIn(m: MonitorInfo): Rect {
  const s = m.scaleFactor > 0 ? m.scaleFactor : 1;
  return {
    x: m.x,
    y: m.y,
    width: Math.min(m.width, Math.round(480 * s)),
    height: Math.min(m.height, Math.round(480 * s)),
  };
}

/** Plan the window for an open or a Fit change.
 *
 *  - Fitted only when Fit is wanted and the monitors were read; otherwise
 *    the saved place (`placeOverlay`), or null without one.
 *  - A reframe when the scene's frame differs from that target's and the
 *    monitors are known. The window rect is `live` (the open, unfitted
 *    window) when it is the one being left, else where the window is about
 *    to go.
 *  - The target's CSS size, when there is a rect. */
export function planFrame(input: {
  scene: OverlayScene;
  wantFit: boolean;
  monitors: MonitorsRead | null;
  saved: OverlayGeometry | null;
  live: Rect | null;
}): FramePlan {
  const { scene, wantFit, saved, live } = input;
  const list = input.monitors?.monitors ?? [];
  const primary = input.monitors?.primary ?? null;
  const fitted = wantFit ? fitRect(saved, list, primary) : null;
  const rect = fitted ?? (saved ? placeOverlay(saved, list, primary) : null);
  const fit = fitted !== null;
  const monitor = pickMonitor(saved, list, primary);
  if (!monitor) return { rect, fit, reframe: null, viewport: null };

  const s = monitor.scaleFactor > 0 ? monitor.scaleFactor : 1;
  const viewport = rect ? { w: rect.width / s, h: rect.height / s } : null;
  const target: Frame = fit ? "monitor" : "window";
  if (frameOf(scene) === target) return { rect, fit, reframe: null, viewport };

  // The window side: the open window being fitted (`live`), else where the
  // unfitted window goes or would go.
  const windowRect =
    (fit ? live : null) ?? (saved ? placeOverlay(saved, list, primary) : null) ?? standIn(monitor);
  const canvas = canvasRects(windowRect, monitor);
  const reframe: SceneOp = fit
    ? { kind: "reframe", from: canvas.window, to: canvas.monitor, frame: "monitor" }
    : { kind: "reframe", from: canvas.monitor, to: canvas.window, frame: "window" };
  return { rect, fit, reframe, viewport };
}
