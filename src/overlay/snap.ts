// v1.62.0 (Text charms): snapping a charm while it is dragged in Arrange.
//
// A press fixes the lines a gesture can snap to: the viewport's left,
// centre and right and top, middle and bottom, and the same six of every
// other live charm as laid out at the press, in the viewport the gesture
// keeps. Each pointermove snaps the preview editor.ts computes, so what is
// drawn, and what the release sends as one `place`, is the snapped
// rectangle. A move snaps each axis on its own; a corner resize snaps the
// moving edge nearer a line and keeps the shape. Within SNAP_PX only, and
// never while the bypass key is held, as in OBS's own preview. Keys never
// snap: arrows are the precise tool. Pure, with type-only imports.

import type { Corner, PxRect, Size } from "./scene";

/** How near a line, in CSS px, a dragged edge or centre snaps to it. The
 *  kill switch: 0 turns snapping off. */
export const SNAP_PX = 8;

/** The keys that can turn snapping off while held. */
export type SnapBypass = "ctrl" | "alt" | "shift";

/** The key that places a charm freely while held: Ctrl, as in OBS's
 *  preview (Alt there means crop, and Alt released alone can start
 *  Windows' menu mode). */
export const SNAP_BYPASS: SnapBypass = "ctrl";

const BYPASS_FLAG = { ctrl: "ctrlKey", alt: "altKey", shift: "shiftKey" } as const;

/** The shortest side a resize gives an element: scene.ts's MIN_ELEMENT_PX,
 *  which editor.ts passes (this file imports types only). */
const MIN_PX = 24;

/** Rounding noise in CSS px. Distances closer than this tie, and an edge
 *  this far past the viewport still counts as on it. Far below anything a
 *  pointer can do, far above float error at 16384 px. */
const EPS = 1e-6;

/** The modifier flags every pointer and key event carries. */
export interface SnapKeys {
  ctrlKey: boolean;
  altKey: boolean;
  shiftKey: boolean;
}

/** Whether a gesture snaps at this event: unless SNAP_BYPASS's key is held.
 *  No other key matters. */
export function snapOn(e: SnapKeys): boolean {
  return !e[BYPASS_FLAG[SNAP_BYPASS]];
}

/** A guide drawn while a gesture is snapped: a vertical line (`x`) or a
 *  horizontal one (`y`), at `at` CSS px. */
export interface Guide {
  axis: "x" | "y";
  at: number;
}

/** One line a gesture can snap to, in CSS px. A tie goes to the viewport's
 *  line before an element's. */
export interface SnapLine {
  at: number;
  viewport: boolean;
}

/** The lines a gesture can snap to, fixed at its press: vertical lines
 *  (`xs`) and horizontal ones (`ys`). */
export interface SnapTargets {
  xs: readonly SnapLine[];
  ys: readonly SnapLine[];
}

/** A snapped preview and its guides (none when nothing snapped). */
export interface Snapped {
  rect: PxRect;
  guides: Guide[];
}

function finite(n: unknown): n is number {
  return typeof n === "number" && Number.isFinite(n);
}

/** A line's three places on one axis: its start, centre and end. */
function thirds(start: number, size: number, viewport: boolean): SnapLine[] {
  return [start, start + size / 2, start + size].map((at) => ({ at, viewport }));
}

/** The lines for a gesture on `dragged`: the viewport's six, and the six of
 *  every other charm in `laid` (the stage's layout at the press, in `v`).
 *  The dragged charm's own lines are never targets: they move with it. */
export function snapTargets(
  laid: ReadonlyArray<{ el: { id: string }; rect: PxRect }>,
  dragged: string,
  v: Size,
): SnapTargets {
  const xs: SnapLine[] = [];
  const ys: SnapLine[] = [];
  if (finite(v.w) && finite(v.h)) {
    xs.push(...thirds(0, v.w, true));
    ys.push(...thirds(0, v.h, true));
  }
  for (const { el, rect } of laid) {
    if (el.id === dragged) continue;
    if (!finite(rect.x) || !finite(rect.y) || !finite(rect.w) || !finite(rect.h)) continue;
    xs.push(...thirds(rect.x, rect.w, false));
    ys.push(...thirds(rect.y, rect.h, false));
  }
  return { xs, ys };
}

/** A candidate snap on one axis: the line, how far the snapped edge or
 *  centre moves to reach it, and which of start, centre and end (0, 1, 2)
 *  it is. */
interface Hit {
  line: SnapLine;
  d: number;
  k: number;
}

/** Whether `a` wins over `b`: the nearer line; on a tie the viewport's line,
 *  then the lower line, then start before centre before end. */
function wins(a: Hit, b: Hit): boolean {
  const da = Math.abs(a.d);
  const db = Math.abs(b.d);
  if (Math.abs(da - db) > EPS) return da < db;
  if (a.line.viewport !== b.line.viewport) return a.line.viewport;
  if (Math.abs(a.line.at - b.line.at) > EPS) return a.line.at < b.line.at;
  return a.k < b.k;
}

/** Within SNAP_PX of the line (a hair over still counts: rounding). */
function near(d: number): boolean {
  return Math.abs(d) <= SNAP_PX + EPS;
}

function snapping(on: boolean): boolean {
  return on && SNAP_PX > 0;
}

/** A move's best snap on one axis: the span `[start, start + size]` with
 *  its start, centre or end on a line within SNAP_PX, the shifted span
 *  still inside `[0, extent]`. Its new start, or null. */
function snapSpan(
  start: number,
  size: number,
  lines: readonly SnapLine[],
  extent: number,
): { hit: Hit; start: number } | null {
  let best: { hit: Hit; start: number } | null = null;
  for (let k = 0; k < 3; k++) {
    const c = start + (k * size) / 2;
    for (const line of lines) {
      const d = line.at - c;
      if (!near(d)) continue;
      const at = line.at - (k * size) / 2;
      if (at < -EPS || at + size > extent + EPS) continue;
      const hit = { line, d, k };
      if (!best || wins(hit, best.hit)) best = { hit, start: at };
    }
  }
  return best;
}

/** A move's preview `r`, snapped on each axis on its own to the nearest
 *  line within SNAP_PX that keeps it inside `v`, with a guide for each axis
 *  that snapped. Its size never changes, so the reducer stores a move. `r`
 *  itself, and no guides, when `on` is false. */
export function snapMove(r: PxRect, t: SnapTargets, v: Size, on: boolean): Snapped {
  if (!snapping(on)) return { rect: r, guides: [] };
  const x = snapSpan(r.x, r.w, t.xs, v.w);
  const y = snapSpan(r.y, r.h, t.ys, v.h);
  if (!x && !y) return { rect: r, guides: [] };
  const guides: Guide[] = [];
  if (x) guides.push({ axis: "x", at: x.hit.line.at });
  if (y) guides.push({ axis: "y", at: y.hit.line.at });
  // Inside by the check above; the clamp only removes rounding noise.
  const inside = (n: number, size: number, extent: number) => Math.min(Math.max(n, 0), Math.max(0, extent - size));
  return {
    rect: {
      x: x ? inside(x.start, r.w, v.w) : r.x,
      y: y ? inside(y.start, r.h, v.h) : r.y,
      w: r.w,
      h: r.h,
    },
    guides,
  };
}

/** A corner resize's preview `r` (from `rect0`, the opposite corner held
 *  still, as scene.ts's `resizeRect` makes it), snapped on one axis: the
 *  moving x edge or y edge onto the nearest line within SNAP_PX, the other
 *  side taken from `rect0`'s shape. Only a size `resizeRect` itself could
 *  give is kept: its short side at least `min` (or, for an element that is
 *  already smaller and can't reach `min` here, at least its own) and
 *  inside `v`, and only one whose other side, too, moves at most SNAP_PX
 *  from the preview's (so a wide charm's x edge, or a tall one's y edge,
 *  never jumps by the shape); a line that breaks any of these is passed
 *  over for the next nearest. The nearer of the two axes' snaps wins, x on
 *  a tie. One guide, for the snapped axis. `r` itself, and no guides, when
 *  nothing snaps or `on` is false. */
export function snapResize(
  r: PxRect,
  rect0: PxRect,
  corner: Corner,
  t: SnapTargets,
  v: Size,
  on: boolean,
  min = MIN_PX,
): Snapped {
  if (!snapping(on) || !(rect0.w > 0) || !(rect0.h > 0)) return { rect: r, guides: [] };
  const sx = corner === "ne" || corner === "se" ? 1 : -1;
  const sy = corner === "sw" || corner === "se" ? 1 : -1;
  // The corner that stays still.
  const ox = sx > 0 ? rect0.x : rect0.x + rect0.w;
  const oy = sy > 0 ? rect0.y : rect0.y + rect0.h;
  const aspect = rect0.w / rect0.h;
  // resizeRect's own floor on the short side, so a snap never gives what a
  // drag can't (the reducer would grow it about its centre, off the line).
  const short0 = Math.min(rect0.w, rect0.h);
  const sMax = Math.min((sx > 0 ? v.w - ox : ox) / rect0.w, (sy > 0 ? v.h - oy : oy) / rect0.h);
  const sFloor = min / short0;
  const floor = short0 * (sFloor > sMax ? Math.min(1, sMax) : sFloor);

  /** The rect with this width and height, anchored at the still corner;
   *  null when it is below the floor, outside `v`, or either side is more
   *  than SNAP_PX from the preview's. The last keeps the side the shape
   *  derives from jumping: snapping a 10 : 1 text's bottom edge 8 px would
   *  move its right edge 80 px, and hold it there across 160 px of drag. */
  const sized = (w: number, h: number): PxRect | null => {
    if (!(w > 0) || !(h > 0) || Math.min(w, h) < floor - EPS) return null;
    if (!near(w - r.w) || !near(h - r.h)) return null;
    const x = sx > 0 ? ox : ox - w;
    const y = sy > 0 ? oy : oy - h;
    if (x < -EPS || y < -EPS || x + w > v.w + EPS || y + h > v.h + EPS) return null;
    return { x, y, w, h };
  };

  /** The nearest line within SNAP_PX of the moving `edge` that gives a size
   *  `sized` keeps. */
  const best = (
    edge: number,
    lines: readonly SnapLine[],
    size: (at: number) => PxRect | null,
  ): { hit: Hit; rect: PxRect } | null => {
    let out: { hit: Hit; rect: PxRect } | null = null;
    for (const line of lines) {
      const d = line.at - edge;
      if (!near(d)) continue;
      const hit = { line, d, k: 0 };
      if (out && !wins(hit, out.hit)) continue;
      const rect = size(line.at);
      if (rect) out = { hit, rect };
    }
    return out;
  };

  const x = best(sx > 0 ? r.x + r.w : r.x, t.xs, (at) => {
    const w = sx * (at - ox);
    return sized(w, w / aspect);
  });
  const y = best(sy > 0 ? r.y + r.h : r.y, t.ys, (at) => {
    const h = sy * (at - oy);
    return sized(h * aspect, h);
  });
  if (x && (!y || Math.abs(x.hit.d) <= Math.abs(y.hit.d) + EPS)) {
    return { rect: x.rect, guides: [{ axis: "x", at: x.hit.line.at }] };
  }
  if (y) return { rect: y.rect, guides: [{ axis: "y", at: y.hit.line.at }] };
  return { rect: r, guides: [] };
}
