// v1.51.3: where the stream overlay window goes, and what `divora.overlay`
// remembers about it.
//
// Tauri reports window and monitor rectangles in PHYSICAL pixels, while the
// `x`/`y`/`width`/`height` a window is created with are LOGICAL. Mixing the
// two misplaces the overlay on any display that isn't at 100 % scaling, so
// everything here is physical: the rectangle is saved as `outerPosition()` /
// `innerSize()` report it and restored with `setPosition(PhysicalPosition)` /
// `setSize(PhysicalSize)` once the window exists. Pure, so the arithmetic is
// tested without a window.

import { OVERLAY_BG_COLOR, type OverlayBg } from "./state";

/** A rectangle in physical pixels. */
export interface Rect {
  x: number;
  y: number;
  width: number;
  height: number;
}

/** A monitor as `availableMonitors()` reports it, flattened (physical px). */
export interface MonitorInfo extends Rect {
  name: string | null;
  scaleFactor: number;
}

/** What `divora.overlay` remembers about the window, besides `bg`. All
 *  optional on disk — a v1.16.0 `{ bg }` blob simply has no geometry. */
export interface OverlayGeometry {
  x: number;
  y: number;
  w: number;
  h: number;
  /** The monitor the window was on, so it goes back there. */
  monitor?: string;
  /** That monitor's scale factor when saved, so the window keeps its
   *  apparent size if the display's scaling changed in between. */
  scale?: number;
}

/** Smallest overlay we will restore to, in physical pixels: below this the
 *  circle is unreadable and the window hard to grab again. */
export const OVERLAY_MIN_SIZE = 120;

/** Windows parks minimised windows at (-32000, -32000). A position that far
 *  out is not a place anyone put the overlay, so it is never saved. */
const PARKED = -30000;

const GEOMETRY_KEYS = ["x", "y", "w", "h", "monitor", "scale"] as const;

function finite(n: unknown): n is number {
  return typeof n === "number" && Number.isFinite(n);
}

/** The background mode from a stored blob, defaulting like v1.16.0 did. */
export function readOverlayBg(raw: unknown): OverlayBg {
  // An own-property check: `"toString" in OVERLAY_BG_COLOR` is true through
  // the prototype, and would hand the overlay a function for a colour.
  return typeof raw === "string" && Object.prototype.hasOwnProperty.call(OVERLAY_BG_COLOR, raw)
    ? (raw as OverlayBg)
    : "transparent";
}

/** Geometry from a stored blob, or null when there is none or it is not
 *  something we would ever have written. */
export function readGeometry(raw: unknown): OverlayGeometry | null {
  if (typeof raw !== "object" || raw === null) return null;
  const o = raw as Record<string, unknown>;
  const { x, y, w, h } = o;
  if (!finite(x) || !finite(y) || !finite(w) || !finite(h)) return null;
  if (w <= 0 || h <= 0 || x <= PARKED || y <= PARKED) return null;
  const g: OverlayGeometry = { x, y, w, h };
  if (typeof o.monitor === "string" && o.monitor.length > 0) g.monitor = o.monitor;
  if (finite(o.scale) && o.scale > 0) g.scale = o.scale;
  return g;
}

/** Whether a freshly read window position is worth saving. */
export function isSavablePosition(x: number, y: number): boolean {
  return finite(x) && finite(y) && x > PARKED && y > PARKED;
}

/** The blob to write: what was stored, with `bg` and the geometry replaced.
 *  Keys this build doesn't know are kept (the surface is additive-only, so a
 *  newer build's fields must survive an older one saving). Null geometry
 *  removes the geometry keys rather than leaving stale ones behind. */
export function mergeOverlayPrefs(
  stored: unknown,
  bg: OverlayBg,
  geometry: OverlayGeometry | null,
): Record<string, unknown> {
  const base: Record<string, unknown> =
    typeof stored === "object" && stored !== null && !Array.isArray(stored)
      ? { ...(stored as Record<string, unknown>) }
      : {};
  for (const k of GEOMETRY_KEYS) delete base[k];
  base.bg = bg;
  if (geometry) {
    base.x = geometry.x;
    base.y = geometry.y;
    base.w = geometry.w;
    base.h = geometry.h;
    if (geometry.monitor !== undefined) base.monitor = geometry.monitor;
    if (geometry.scale !== undefined) base.scale = geometry.scale;
  }
  return base;
}

function overlap(a: Rect, b: Rect): number {
  const w = Math.min(a.x + a.width, b.x + b.width) - Math.max(a.x, b.x);
  const h = Math.min(a.y + a.height, b.y + b.height) - Math.max(a.y, b.y);
  return w > 0 && h > 0 ? w * h : 0;
}

function clamp(n: number, lo: number, hi: number): number {
  return Math.min(Math.max(n, lo), Math.max(lo, hi));
}

/** Where to put the overlay, given where it was and the monitors that exist
 *  now; null when there are no monitors to place it on.
 *
 *  1. Back on the monitor it was on, if that monitor still exists.
 *  2. Otherwise on whichever monitor the old rectangle overlaps most.
 *  3. Otherwise on the primary monitor (an unplugged second screen).
 *
 *  If that monitor's scale factor differs from the one saved, the size is
 *  scaled by the ratio so the window keeps its apparent size. The result is
 *  then clamped to fit entirely on that monitor. */
export function placeOverlay(
  saved: OverlayGeometry,
  monitors: MonitorInfo[],
  primaryName: string | null = null,
): Rect | null {
  if (monitors.length === 0) return null;
  const was: Rect = { x: saved.x, y: saved.y, width: saved.w, height: saved.h };

  let target: MonitorInfo | undefined;
  if (saved.monitor !== undefined) {
    target = monitors.find((m) => m.name === saved.monitor);
  }
  if (!target) {
    let best = 0;
    for (const m of monitors) {
      const a = overlap(was, m);
      if (a > best) {
        best = a;
        target = m;
      }
    }
  }
  if (!target && primaryName !== null) {
    target = monitors.find((m) => m.name === primaryName);
  }
  target ??= monitors[0]!;

  let width = saved.w;
  let height = saved.h;
  if (saved.scale !== undefined && target.scaleFactor > 0 && target.scaleFactor !== saved.scale) {
    const k = target.scaleFactor / saved.scale;
    width *= k;
    height *= k;
  }
  width = Math.round(clamp(width, Math.min(OVERLAY_MIN_SIZE, target.width), target.width));
  height = Math.round(clamp(height, Math.min(OVERLAY_MIN_SIZE, target.height), target.height));
  const x = Math.round(clamp(saved.x, target.x, target.x + target.width - width));
  const y = Math.round(clamp(saved.y, target.y, target.y + target.height - height));
  return { x, y, width, height };
}
