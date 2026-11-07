// v1.55.0: the arithmetic behind arranging charms, and which keys do what.
//
// Stage only wires events: a press starts a gesture here, each pointermove
// updates its preview, and pointerup turns it into at most one `place` edit.
// Nothing is sent while the pointer moves. A gesture keeps the viewport it
// began in, so a window resized mid-drag can't mix two sizes in one edit.
// Pure, so every rule is a table test.
//
// v1.62.0: a pointer gesture snaps (snap.ts). Its press fixes the lines it
// can snap to; each update snaps the preview and keeps the guides to draw,
// so what is drawn and what `place` stores is the snapped rectangle. Keys
// never snap: a nudge run starts a gesture with no lines.

import {
  MIN_ELEMENT_PX,
  moveRect,
  resizeRect,
  type Corner,
  type PxRect,
  type SceneOp,
  type Size,
} from "./scene";
import { snapMove, snapResize, type Guide, type SnapTargets } from "./snap";

/** How long after the last arrow nudge or keyboard resize its one edit is
 *  sent. */
export const NUDGE_FLUSH_MS = 300;

type Point = { x: number; y: number };

/** v1.62.0: a gesture with nothing to snap to: a nudge run, and any
 *  gesture begun without lines, which then behaves as before. */
const NO_TARGETS: SnapTargets = Object.freeze({ xs: Object.freeze([]), ys: Object.freeze([]) });

/** v1.62.0: `targets` are the lines fixed at the press (snap.ts's
 *  `snapTargets`, in `v0`), and `guides` the lines the preview is snapped
 *  to, drawn while the pointer is down. */
export type Gesture =
  | {
      kind: "move";
      id: string;
      pointerId: number;
      start: Point;
      rect0: PxRect;
      v0: Size;
      targets: SnapTargets;
      preview: PxRect;
      guides: Guide[];
    }
  | {
      kind: "resize";
      id: string;
      corner: Corner;
      pointerId: number;
      start: Point;
      rect0: PxRect;
      v0: Size;
      targets: SnapTargets;
      preview: PxRect;
      guides: Guide[];
    };

export function beginMove(
  id: string,
  pointerId: number,
  at: Point,
  rect: PxRect,
  v: Size,
  targets: SnapTargets = NO_TARGETS,
): Gesture {
  return { kind: "move", id, pointerId, start: at, rect0: rect, v0: v, targets, preview: rect, guides: [] };
}

export function beginResize(
  id: string,
  corner: Corner,
  pointerId: number,
  at: Point,
  rect: PxRect,
  v: Size,
  targets: SnapTargets = NO_TARGETS,
): Gesture {
  return { kind: "resize", id, corner, pointerId, start: at, rect0: rect, v0: v, targets, preview: rect, guides: [] };
}

/** The gesture with its preview for the pointer at `at`, from where it
 *  started, in the viewport it started in. v1.62.0: snapped to the press's
 *  lines unless `snap` is false (snap.ts's `snapOn`: the bypass key held).
 *  Re-run at the last pointer point when that key changes mid-gesture. */
export function updateGesture(g: Gesture, at: Point, snap = true): Gesture {
  const dx = at.x - g.start.x;
  const dy = at.y - g.start.y;
  if (g.kind === "move") {
    const { rect, guides } = snapMove(moveRect(g.rect0, dx, dy, g.v0), g.targets, g.v0, snap);
    return { ...g, preview: rect, guides };
  }
  const raw = resizeRect(g.rect0, g.corner, dx, dy, g.v0);
  const { rect, guides } = snapResize(raw, g.rect0, g.corner, g.targets, g.v0, snap, MIN_ELEMENT_PX);
  return { ...g, preview: rect, guides };
}

/** The edit a released gesture makes; null when it moved nothing (under
 *  half a pixel on every side). */
export function finishGesture(g: Gesture): SceneOp | null {
  const a = g.preview;
  const b = g.rect0;
  const still =
    Math.abs(a.x - b.x) < 0.5 && Math.abs(a.y - b.y) < 0.5 && Math.abs(a.w - b.w) < 0.5 && Math.abs(a.h - b.h) < 0.5;
  if (still) return null;
  return { kind: "place", id: g.id, rect: { ...a }, viewport: { w: g.v0.w, h: g.v0.h } };
}

export type KeyAction =
  | { kind: "lock" }
  | { kind: "cancel" }
  | { kind: "nudge"; dx: number; dy: number }
  | { kind: "scale"; dir: 1 | -1; big: boolean }
  | { kind: "remove" }
  | { kind: "order"; to: "front" | "back" | "forward" | "backward" }
  | { kind: "none" };

const NONE: KeyAction = { kind: "none" };

const ARROWS = new Map<string, [number, number]>([
  ["ArrowLeft", [-1, 0]],
  ["ArrowRight", [1, 0]],
  ["ArrowUp", [0, -1]],
  ["ArrowDown", [0, 1]],
]);

const ORDER_KEYS = new Map<string, "front" | "back" | "forward" | "backward">([
  ["]", "forward"],
  ["[", "backward"],
  ["}", "front"],
  ["{", "back"],
]);

/** What a key does in the overlay. Locked: nothing (Esc included). During a
 *  pointer gesture: Esc cancels it and nothing else acts. Otherwise Esc
 *  locks, and with an element selected: arrows nudge 1 px (Shift: 10),
 *  Delete removes (Backspace doesn't), = and numpad + grow a step, + grows
 *  10 %, - and numpad − shrink a step, _ shrinks 10 %, and [ ] { } restack. */
export function keyAction(
  e: { key: string; code: string; shiftKey: boolean },
  ctx: { arrange: boolean; gesture: boolean; selected: boolean },
): KeyAction {
  if (!ctx.arrange) return NONE;
  if (ctx.gesture) return e.key === "Escape" ? { kind: "cancel" } : NONE;
  if (e.key === "Escape") return { kind: "lock" };
  if (!ctx.selected) return NONE;
  const arrow = ARROWS.get(e.key);
  if (arrow) {
    const step = e.shiftKey ? 10 : 1;
    return { kind: "nudge", dx: arrow[0] * step, dy: arrow[1] * step };
  }
  if (e.key === "Delete") return { kind: "remove" };
  // The numpad's keys by `code`: Shift doesn't change what they mean.
  if (e.code === "NumpadAdd" || e.key === "=") return { kind: "scale", dir: 1, big: false };
  if (e.code === "NumpadSubtract" || e.key === "-") return { kind: "scale", dir: -1, big: false };
  if (e.key === "+") return { kind: "scale", dir: 1, big: true };
  if (e.key === "_") return { kind: "scale", dir: -1, big: true };
  const to = ORDER_KEYS.get(e.key);
  return to ? { kind: "order", to } : NONE;
}

/** Whether the overlay blocks a key's default. Everything is blocked —
 *  reload, print, find, zoom, DevTools, Alt+arrows and Alt+Space (the
 *  system menu, a popup capture exclusion can't cover) — except Tab and
 *  Shift+Tab, Alt+F4 (a focused overlay must still close), and Enter or
 *  Space on a button (so a keyboard can press Lock). */
export function shouldPreventKey(
  e: { key: string; altKey: boolean; ctrlKey: boolean; metaKey: boolean },
  onButton: boolean,
): boolean {
  if (e.key === "Tab" && !e.altKey && !e.ctrlKey && !e.metaKey) return false;
  if (e.key === "F4" && e.altKey && !e.ctrlKey && !e.metaKey) return false;
  if (onButton && (e.key === "Enter" || e.key === " ") && !e.altKey && !e.ctrlKey && !e.metaKey) return false;
  return true;
}
