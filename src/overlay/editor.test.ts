import { describe, expect, it } from "vitest";
import {
  beginMove,
  beginResize,
  finishGesture,
  keyAction,
  NUDGE_FLUSH_MS,
  shouldPreventKey,
  updateGesture,
} from "./editor";
import { moveRect, resizeRect, type Corner, type PxRect, type Size } from "./scene";
import { snapTargets } from "./snap";

const V: Size = { w: 1000, h: 800 };
const R: PxRect = { x: 300, y: 300, w: 300, h: 100 };
const CORNERS: Corner[] = ["nw", "ne", "sw", "se"];

/** +1 when the corner's x grows outward to the right, −1 to the left. */
const sx = (c: Corner) => (c === "ne" || c === "se" ? 1 : -1);
const sy = (c: Corner) => (c === "sw" || c === "se" ? 1 : -1);

/** The corner opposite the dragged one, which must never move. */
function anchor(r: PxRect, c: Corner): [number, number] {
  return [sx(c) > 0 ? r.x : r.x + r.w, sy(c) > 0 ? r.y : r.y + r.h];
}

describe("resizeRect", () => {
  // [name, dx along the corner's outward x, dy along its outward y, grows]
  const rows: Array<[string, number, number, boolean]> = [
    ["horizontal inward", -50, 0, false],
    ["vertical inward", 0, -50, false],
    ["horizontal inward with a small outward wobble", -150, 3, false],
    ["horizontal outward", 50, 0, true],
    ["vertical outward", 0, 50, true],
    ["horizontal outward with a small inward wobble", 150, -3, true],
  ];

  for (const c of CORNERS) {
    it.each(rows)(`${c}: %s`, (_name, ox, oy, grows) => {
      const r = resizeRect(R, c, ox * sx(c), oy * sy(c), V);
      const s = r.w / R.w;
      if (grows) expect(s).toBeGreaterThan(1);
      else expect(s).toBeLessThan(1);
      expect(r.w / r.h).toBeCloseTo(3, 12);
      expect(anchor(r, c)[0]).toBeCloseTo(anchor(R, c)[0], 9);
      expect(anchor(r, c)[1]).toBeCloseTo(anchor(R, c)[1], 9);
    });

    it(`${c}: along the exact diagonal the corner lands on the pointer`, () => {
      for (const k of [-0.5, -0.2, 0.1, 0.4]) {
        const r = resizeRect(R, c, k * R.w * sx(c), k * R.h * sy(c), V);
        expect(r.w).toBeCloseTo(R.w * (1 + k), 9);
        const corner = [sx(c) > 0 ? r.x + r.w : r.x, sy(c) > 0 ? r.y + r.h : r.y];
        const pointer = [(sx(c) > 0 ? R.x + R.w : R.x) + k * R.w * sx(c), (sy(c) > 0 ? R.y + R.h : R.y) + k * R.h * sy(c)];
        expect(Math.abs(corner[0]! - pointer[0]!)).toBeLessThan(1e-9);
        expect(Math.abs(corner[1]! - pointer[1]!)).toBeLessThan(1e-9);
      }
    });
  }

  it("the worked example: 300×100 from se", () => {
    expect(resizeRect(R, "se", -150, 0, V).w).toBeCloseTo(165, 9);
    expect(resizeRect(R, "se", -150, 0, V).h).toBeCloseTo(55, 9);
    expect(resizeRect(R, "se", -150, 3, V).w / R.w).toBeCloseTo(0.553, 9);
    expect(resizeRect(R, "se", 150, 0, V).w / R.w).toBeCloseTo(1.45, 9);
  });

  it("is continuous: a 1 px pointer step never jumps the size", () => {
    // The largest change a 1 px step can make, along the diagonal.
    const bound = (1 * Math.max(R.w, R.h)) / Math.hypot(R.w, R.h) + 1e-9;
    for (const c of CORNERS) {
      for (let dx = -200; dx <= 200; dx += 7) {
        for (let dy = -60; dy <= 60; dy += 5) {
          const a = resizeRect(R, c, dx, dy, V);
          for (const [ex, ey] of [
            [1, 0],
            [0, 1],
          ] as const) {
            const b = resizeRect(R, c, dx + ex, dy + ey, V);
            expect(Math.abs(b.w - a.w)).toBeLessThanOrEqual(bound);
          }
        }
      }
    }
  });

  it("stops at the 24 px minimum", () => {
    for (const c of CORNERS) {
      const r = resizeRect(R, c, -1000 * sx(c), -1000 * sy(c), V);
      expect(r.h).toBeCloseTo(24, 9);
      expect(r.w).toBeCloseTo(72, 9);
      expect(anchor(r, c)[0]).toBeCloseTo(anchor(R, c)[0], 9);
    }
  });

  it("stops at the viewport", () => {
    const r = resizeRect(R, "se", 5000, 5000, V);
    // Width reaches the right edge first (sMax = 700/300 vs 500/100).
    expect(r.x + r.w).toBeCloseTo(1000, 9);
    expect(r.w / r.h).toBeCloseTo(3, 12);
    const l = resizeRect(R, "nw", -5000, -5000, V);
    expect(l.x).toBeCloseTo(0, 9);
    expect(l.x + l.w).toBeCloseTo(600, 9);
  });

  it("below the minimum and unable to reach it: it can grow but not shrink further", () => {
    // 10×5 in a corner where it can grow at most 2×.
    const small = { x: 0, y: 0, w: 10, h: 5 };
    const v = { w: 20, h: 10 };
    expect(resizeRect(small, "se", -5, -5, v)).toEqual(small);
    const grown = resizeRect(small, "se", 50, 50, v);
    expect(grown.w).toBeCloseTo(20, 9);
    expect(grown.h).toBeCloseTo(10, 9);
  });

  it("a pointer past the anchor clamps instead of flipping", () => {
    const r = resizeRect(R, "se", -900, -900, V);
    expect(r.w).toBeGreaterThan(0);
    expect(r.x).toBe(R.x);
    expect(r.y).toBe(R.y);
  });
});

describe("moveRect", () => {
  it("moves and stays inside the viewport", () => {
    expect(moveRect(R, 10, -20, V)).toEqual({ x: 310, y: 280, w: 300, h: 100 });
    expect(moveRect(R, -1000, -1000, V)).toEqual({ x: 0, y: 0, w: 300, h: 100 });
    expect(moveRect(R, 1000, 1000, V)).toEqual({ x: 700, y: 700, w: 300, h: 100 });
    // Larger than the viewport: pinned to its top-left.
    expect(moveRect({ x: 5, y: 5, w: 2000, h: 10 }, 30, 30, V)).toEqual({ x: 0, y: 35, w: 2000, h: 10 });
  });
});

describe("gestures", () => {
  it("a move previews from the start point and ends as one place", () => {
    let g = beginMove("p", 7, { x: 400, y: 350 }, R, V);
    g = updateGesture(g, { x: 410, y: 360 });
    g = updateGesture(g, { x: 450, y: 330 });
    expect(g.preview).toEqual({ x: 350, y: 280, w: 300, h: 100 });
    expect(finishGesture(g)).toEqual({ kind: "place", id: "p", rect: { x: 350, y: 280, w: 300, h: 100 }, viewport: V });
  });

  it("a resize uses resizeRect from its corner", () => {
    let g = beginResize("p", "se", 1, { x: 600, y: 400 }, R, V);
    g = updateGesture(g, { x: 450, y: 400 });
    expect(g.preview.w).toBeCloseTo(165, 9);
    expect(finishGesture(g)).toMatchObject({ kind: "place", id: "p", viewport: V });
  });

  it("under half a pixel of change sends nothing", () => {
    const g = updateGesture(beginMove("p", 1, { x: 0, y: 0 }, R, V), { x: 0.4, y: -0.4 });
    expect(finishGesture(g)).toBeNull();
    expect(finishGesture(beginMove("p", 1, { x: 0, y: 0 }, R, V))).toBeNull();
    const r = updateGesture(beginResize("p", "ne", 1, { x: 0, y: 0 }, R, V), { x: 0.2, y: 0 });
    expect(finishGesture(r)).toBeNull();
    const moved = updateGesture(beginMove("p", 1, { x: 0, y: 0 }, R, V), { x: 0.6, y: 0 });
    expect(finishGesture(moved)).not.toBeNull();
  });

  it("keeps the viewport the gesture began in", () => {
    const v0 = { w: 640, h: 480 };
    let g = beginMove("p", 1, { x: 0, y: 0 }, { x: 100, y: 100, w: 100, h: 100 }, v0);
    g = updateGesture(g, { x: 5000, y: 0 });
    // Clamped by the starting viewport, and placed in it.
    expect(g.preview.x).toBe(540);
    expect(finishGesture(g)).toMatchObject({ viewport: v0 });
  });
});

describe("gestures snap (v1.62.0)", () => {
  // §7.5's viewport and other charm: lines x 0 / 240 / 480 and 300 / 350 /
  // 400, y 0 / 240 / 480 and 100 / 125 / 150.
  const V480: Size = { w: 480, h: 480 };
  const T = snapTargets([{ el: { id: "o" }, rect: { x: 300, y: 100, w: 100, h: 50 } }], "p", V480);
  const P: PxRect = { x: 100, y: 300, w: 100, h: 40 };
  const R0: PxRect = { x: 100, y: 100, w: 100, h: 50 };

  it("updateGesture snaps, and not with snap off", () => {
    // Dragged 86 right: its centre at 236, 4 from the viewport's centre line.
    let g = beginMove("p", 1, { x: 150, y: 320 }, P, V480, T);
    expect(g.guides).toEqual([]);
    g = updateGesture(g, { x: 236, y: 320 });
    expect(g.preview).toEqual({ x: 190, y: 300, w: 100, h: 40 });
    expect(g.guides).toEqual([{ axis: "x", at: 240 }]);
    expect(finishGesture(g)).toEqual({ kind: "place", id: "p", rect: { x: 190, y: 300, w: 100, h: 40 }, viewport: V480 });
    // The bypass key held: where dropped, no guide.
    const free = updateGesture(g, { x: 236, y: 320 }, false);
    expect(free.preview).toEqual({ x: 186, y: 300, w: 100, h: 40 });
    expect(free.guides).toEqual([]);
    expect(finishGesture(free)).toEqual({ kind: "place", id: "p", rect: { x: 186, y: 300, w: 100, h: 40 }, viewport: V480 });
    // Released mid-gesture, the same point snaps again (Stage re-runs the
    // last point on a Control change).
    expect(updateGesture(free, { x: 236, y: 320 })).toEqual(g);
    // Out of reach of every line: the guide goes.
    const away = updateGesture(g, { x: 200, y: 320 });
    expect(away.preview).toEqual({ x: 150, y: 300, w: 100, h: 40 });
    expect(away.guides).toEqual([]);
  });

  it("a resize snaps its moving edge and keeps the shape, and not with snap off", () => {
    let g = beginResize("p", "se", 1, { x: 200, y: 150 }, R0, V480, T);
    g = updateGesture(g, { x: 236, y: 168 });
    expect(g.preview.x).toBe(100);
    expect(g.preview.y).toBe(100);
    expect(g.preview.w).toBeCloseTo(140, 9);
    expect(g.preview.h).toBeCloseTo(70, 9);
    expect(g.guides).toEqual([{ axis: "x", at: 240 }]);
    expect(finishGesture(g)).toMatchObject({ kind: "place", id: "p", viewport: V480 });
    const free = updateGesture(g, { x: 236, y: 168 }, false);
    expect(free.preview.w).toBeCloseTo(136, 9);
    expect(free.preview.h).toBeCloseTo(68, 9);
    expect(free.guides).toEqual([]);
  });

  it("a resize snaps against its press rect, not its last preview", () => {
    // 20 × 10 in a 30 × 15 viewport (it can't reach 24): first grown out of
    // reach of the line at 21 (right edge 29.5), then back to 22, where it
    // snaps to 21 × 10.5. Its press size, 20 × 10, is the floor, not 29.5's.
    const tiny = { x: 0, y: 0, w: 20, h: 10 };
    const v = { w: 30, h: 15 };
    let g = beginResize("p", "se", 1, { x: 20, y: 10 }, tiny, v, { xs: [{ at: 21, viewport: false }], ys: [] });
    g = updateGesture(g, { x: 29.5, y: 14.75 });
    expect(g.preview.w).toBeCloseTo(29.5, 9);
    expect(g.guides).toEqual([]);
    g = updateGesture(g, { x: 22, y: 11 });
    expect(g.preview.w).toBeCloseTo(21, 9);
    expect(g.preview.h).toBeCloseTo(10.5, 9);
    expect(g.guides).toEqual([{ axis: "x", at: 21 }]);
  });

  it("a gesture with no lines behaves as before v1.62.0", () => {
    // Begun without targets, as the existing callers and a nudge run do.
    let g = beginMove("p", 1, { x: 150, y: 320 }, P, V480);
    expect(g.targets.xs).toEqual([]);
    expect(g.targets.ys).toEqual([]);
    g = updateGesture(g, { x: 236, y: 320 });
    expect(g.preview).toEqual(moveRect(P, 86, 0, V480));
    expect(g.guides).toEqual([]);
    let r = beginResize("p", "se", 1, { x: 200, y: 150 }, R0, V480);
    r = updateGesture(r, { x: 236, y: 168 });
    expect(r.preview).toEqual(resizeRect(R0, "se", 36, 18, V480));
    expect(r.guides).toEqual([]);
  });

  it("snaps to the press's lines in the press's viewport", () => {
    // Lines from a 480 × 480 press; the gesture keeps them (and v0) however
    // far the pointer goes: the right edge reaches 480 and is held there.
    let g = beginMove("p", 1, { x: 0, y: 0 }, P, V480, T);
    g = updateGesture(g, { x: 5000, y: 0 });
    expect(g.preview.x).toBe(380);
    expect(g.guides).toEqual([{ axis: "x", at: 480 }]);
    expect(finishGesture(g)).toMatchObject({ viewport: V480 });
  });
});

describe("keyAction", () => {
  const key = (k: string, more: { code?: string; shiftKey?: boolean } = {}) => ({
    key: k,
    code: more.code ?? "",
    shiftKey: more.shiftKey ?? false,
  });
  const arrange = { arrange: true, gesture: false, selected: true };

  it("locked: nothing, Escape included", () => {
    for (const k of ["Escape", "ArrowLeft", "Delete", "+", "]"]) {
      expect(keyAction(key(k), { arrange: false, gesture: false, selected: true })).toEqual({ kind: "none" });
    }
  });

  it("during a pointer gesture only Escape acts, and it cancels", () => {
    const ctx = { arrange: true, gesture: true, selected: true };
    expect(keyAction(key("Escape"), ctx)).toEqual({ kind: "cancel" });
    for (const k of ["ArrowLeft", "Delete", "]", "+", "="]) expect(keyAction(key(k), ctx)).toEqual({ kind: "none" });
  });

  it("Escape locks, selected or not", () => {
    expect(keyAction(key("Escape"), arrange)).toEqual({ kind: "lock" });
    expect(keyAction(key("Escape"), { ...arrange, selected: false })).toEqual({ kind: "lock" });
  });

  it.each([
    ["ArrowLeft", {}, { kind: "nudge", dx: -1, dy: 0 }],
    ["ArrowRight", {}, { kind: "nudge", dx: 1, dy: 0 }],
    ["ArrowUp", {}, { kind: "nudge", dx: 0, dy: -1 }],
    ["ArrowDown", {}, { kind: "nudge", dx: 0, dy: 1 }],
    ["ArrowLeft", { shiftKey: true }, { kind: "nudge", dx: -10, dy: 0 }],
    ["ArrowDown", { shiftKey: true }, { kind: "nudge", dx: 0, dy: 10 }],
    ["Delete", {}, { kind: "remove" }],
    ["Backspace", {}, { kind: "none" }],
    ["=", { code: "Equal" }, { kind: "scale", dir: 1, big: false }],
    ["+", { code: "Equal", shiftKey: true }, { kind: "scale", dir: 1, big: true }],
    ["+", { code: "NumpadAdd" }, { kind: "scale", dir: 1, big: false }],
    ["+", { code: "NumpadAdd", shiftKey: true }, { kind: "scale", dir: 1, big: false }],
    ["-", { code: "Minus" }, { kind: "scale", dir: -1, big: false }],
    ["_", { code: "Minus", shiftKey: true }, { kind: "scale", dir: -1, big: true }],
    ["-", { code: "NumpadSubtract" }, { kind: "scale", dir: -1, big: false }],
    ["]", {}, { kind: "order", to: "forward" }],
    ["[", {}, { kind: "order", to: "backward" }],
    ["}", { shiftKey: true }, { kind: "order", to: "front" }],
    ["{", { shiftKey: true }, { kind: "order", to: "back" }],
    ["a", {}, { kind: "none" }],
    ["Enter", {}, { kind: "none" }],
    ["constructor", {}, { kind: "none" }],
  ])("selected: %s %o", (k, more, want) => {
    expect(keyAction(key(k, more), arrange)).toEqual(want);
  });

  it("with nothing selected, only Escape acts", () => {
    for (const k of ["ArrowLeft", "Delete", "+", "=", "-", "_", "]", "{"]) {
      expect(keyAction(key(k, { code: k === "+" ? "NumpadAdd" : "" }), { ...arrange, selected: false })).toEqual({ kind: "none" });
    }
  });

  it("a nudge or keyboard resize is sent 300 ms after the last key", () => {
    expect(NUDGE_FLUSH_MS).toBe(300);
  });
});

describe("shouldPreventKey", () => {
  type K = { key: string; altKey?: boolean; ctrlKey?: boolean; metaKey?: boolean; shiftKey?: boolean };
  const ev = (k: K) => ({ key: k.key, altKey: !!k.altKey, ctrlKey: !!k.ctrlKey, metaKey: !!k.metaKey });

  it.each([
    ["Tab", { key: "Tab" }, false],
    ["Shift+Tab", { key: "Tab", shiftKey: true }, false],
    ["Alt+F4", { key: "F4", altKey: true }, false],
    ["Alt+Shift+F4", { key: "F4", altKey: true, shiftKey: true }, false],
  ] as Array<[string, K, boolean]>)("%s is let through anywhere", (_n, k, want) => {
    expect(shouldPreventKey(ev(k), false)).toBe(want);
    expect(shouldPreventKey(ev(k), true)).toBe(want);
  });

  it.each([
    ["Enter", { key: "Enter" }],
    ["Space", { key: " " }],
    ["Shift+Enter", { key: "Enter", shiftKey: true }],
  ] as Array<[string, K]>)("%s on a button presses it", (_n, k) => {
    expect(shouldPreventKey(ev(k), true)).toBe(false);
    expect(shouldPreventKey(ev(k), false)).toBe(true);
  });

  it.each([
    ["F5", { key: "F5" }],
    ["Ctrl+R", { key: "r", ctrlKey: true }],
    ["Ctrl+Shift+R", { key: "R", ctrlKey: true, shiftKey: true }],
    ["Ctrl+P", { key: "p", ctrlKey: true }],
    ["Ctrl+F", { key: "f", ctrlKey: true }],
    ["F3", { key: "F3" }],
    ["F7", { key: "F7" }],
    ["F12", { key: "F12" }],
    ["Ctrl+Shift+I", { key: "I", ctrlKey: true, shiftKey: true }],
    ["Ctrl+Plus", { key: "+", ctrlKey: true }],
    ["Ctrl+Equal", { key: "=", ctrlKey: true }],
    ["Ctrl+Minus", { key: "-", ctrlKey: true }],
    ["Alt+Left", { key: "ArrowLeft", altKey: true }],
    ["Alt+Right", { key: "ArrowRight", altKey: true }],
    ["Alt+Space", { key: " ", altKey: true }],
    ["a bare Alt", { key: "Alt", altKey: true }],
    ["Ctrl+F4", { key: "F4", ctrlKey: true }],
    ["Ctrl+Alt+F4", { key: "F4", altKey: true, ctrlKey: true }],
    ["F4 alone", { key: "F4" }],
    ["Ctrl+Tab", { key: "Tab", ctrlKey: true }],
    ["Escape", { key: "Escape" }],
    ["ArrowLeft", { key: "ArrowLeft" }],
    ["Delete", { key: "Delete" }],
    ["Ctrl+Enter", { key: "Enter", ctrlKey: true }],
    ["Alt+Enter", { key: "Enter", altKey: true }],
    ["Meta+Space", { key: " ", metaKey: true }],
  ] as Array<[string, K]>)("%s is blocked, on a button too", (_n, k) => {
    expect(shouldPreventKey(ev(k), false)).toBe(true);
    expect(shouldPreventKey(ev(k), true)).toBe(true);
  });
});
