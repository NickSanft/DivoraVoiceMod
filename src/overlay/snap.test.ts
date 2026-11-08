import { readFileSync } from "node:fs";
import { join } from "node:path";
import ts from "typescript";
import { describe, expect, it } from "vitest";
import { beginMove, beginResize, finishGesture, updateGesture } from "./editor";
import {
  applyOp,
  isLive,
  layoutElement,
  layoutScene,
  liveElements,
  MIN_ELEMENT_PX,
  moveRect,
  normalizeRect,
  resizeRect,
  type Corner,
  type OverlayScene,
  type PxRect,
  type SceneElement,
  type Size,
} from "./scene";
import {
  SNAP_BYPASS,
  SNAP_PX,
  snapMove,
  snapOn,
  snapResize,
  snapTargets,
  type Guide,
  type SnapLine,
  type SnapTargets,
} from "./snap";

describe("snapping's switches (v1.62.0)", () => {
  it("snaps within 8 px, and Ctrl places freely", () => {
    expect(SNAP_PX).toBe(8);
    expect(SNAP_BYPASS).toBe("ctrl");
  });

  it("snapOn reads the bypass key only", () => {
    const keys = { ctrlKey: false, altKey: false, shiftKey: false };
    expect(snapOn(keys)).toBe(true);
    expect(snapOn({ ...keys, ctrlKey: true })).toBe(false);
    expect(snapOn({ ...keys, altKey: true })).toBe(true);
    expect(snapOn({ ...keys, shiftKey: true })).toBe(true);
    expect(snapOn({ ctrlKey: true, altKey: true, shiftKey: true })).toBe(false);
  });

  it("snap.ts has only type imports", () => {
    // Pure: it reads scene.ts's types, never its code.
    const source = readFileSync(join(process.cwd(), "src", "overlay", "snap.ts"), "utf8");
    const imports = source.split("\n").filter((l) => /^\s*import\b/.test(l));
    expect(imports.length).toBeGreaterThan(0);
    for (const line of imports) expect(line).toMatch(/^import type /);
  });

  it("SNAP_PX 0 turns snapping off, even for an edge already on a line (§11 K5)", () => {
    // The kill switch is a module constant, so this runs snap.ts itself with
    // only that line changed (its imports are types only, so it stands alone).
    const at = (px: number): typeof import("./snap") => {
      const source = readFileSync(join(process.cwd(), "src", "overlay", "snap.ts"), "utf8");
      const decl = "export const SNAP_PX = 8;";
      expect(source.split(decl)).toHaveLength(2);
      const js = ts.transpileModule(source.replace(decl, `export const SNAP_PX = ${px};`), {
        compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 },
      }).outputText;
      const mod = {};
      new Function("exports", js)(mod);
      return mod as typeof import("./snap");
    };
    const v = { w: 480, h: 480 };
    const laid = [{ el: { id: "o" }, rect: { x: 300, y: 100, w: 100, h: 50 } }];
    // Its left edge on the other charm's 300 (d 0), and its centre 4 from
    // the viewport's 240; a corner resize whose bottom edge is on 150.
    const onLine = { x: 300, y: 300, w: 100, h: 40 };
    const nearLine = { x: 186, y: 300, w: 100, h: 40 };
    const rect0 = { x: 100, y: 100, w: 100, h: 50 };
    for (const [px, snaps] of [
      [8, true],
      [0, false],
    ] as const) {
      const s = at(px);
      expect(s.SNAP_PX).toBe(px);
      const t = s.snapTargets(laid, "me", v);
      for (const preview of [onLine, nearLine]) {
        const out = s.snapMove(preview, t, v, true);
        if (snaps) expect(out.guides.length, `${px}`).toBeGreaterThan(0);
        else expect(out, `${px}`).toEqual({ rect: preview, guides: [] });
      }
      const grown = s.snapResize(rect0, rect0, "se", t, v, true);
      if (snaps) expect(grown.guides).toEqual([{ axis: "y", at: 150 }]);
      else expect(grown).toEqual({ rect: rect0, guides: [] });
    }
    // The copy at 8 snaps as the module the app imports does.
    const eight = at(8);
    expect(eight.snapMove(nearLine, eight.snapTargets(laid, "me", v), v, true)).toEqual(
      snapMove(nearLine, snapTargets(laid, "me", v), v, true),
    );
  });
});

// §7.5's worked tables: a 480 × 480 viewport and one other charm at
// {x: 300, y: 100, w: 100, h: 50} (lines x 300 / 350 / 400, y 100 / 125 / 150).
const V480: Size = { w: 480, h: 480 };
const OTHER: PxRect = { x: 300, y: 100, w: 100, h: 50 };
const CTRL = { ctrlKey: true, altKey: false, shiftKey: false };
const NONE = { ctrlKey: false, altKey: false, shiftKey: false };

/** The lines a gesture on "me" gets when `others` are laid out beside it. */
function targetsWith(...others: PxRect[]): SnapTargets {
  const laid = others.map((rect, i) => ({ el: { id: `o${i}` }, rect }));
  return snapTargets(laid, "me", V480);
}

/** Lines of one kind, by hand, for rules the viewport's own lines would
 *  otherwise hide. */
const lines = (...at: number[]): SnapLine[] => at.map((a) => ({ at: a, viewport: false }));

function closeRect(a: PxRect, b: PxRect, digits = 9): void {
  expect(a.x).toBeCloseTo(b.x, digits);
  expect(a.y).toBeCloseTo(b.y, digits);
  expect(a.w).toBeCloseTo(b.w, digits);
  expect(a.h).toBeCloseTo(b.h, digits);
}

describe("snapTargets (v1.62.0)", () => {
  it("the viewport's six lines, then every other charm's six, never the dragged one's", () => {
    const laid = [
      { el: { id: "a" }, rect: { x: 10, y: 20, w: 100, h: 40 } },
      { el: { id: "me" }, rect: { x: 200, y: 300, w: 60, h: 30 } },
      { el: { id: "b" }, rect: { x: 50, y: 60, w: 20, h: 10 } },
    ];
    const t = snapTargets(laid, "me", { w: 640, h: 360 });
    expect(t.xs).toEqual([
      { at: 0, viewport: true },
      { at: 320, viewport: true },
      { at: 640, viewport: true },
      { at: 10, viewport: false },
      { at: 60, viewport: false },
      { at: 110, viewport: false },
      { at: 50, viewport: false },
      { at: 60, viewport: false },
      { at: 70, viewport: false },
    ]);
    expect(t.ys).toEqual([
      { at: 0, viewport: true },
      { at: 180, viewport: true },
      { at: 360, viewport: true },
      { at: 20, viewport: false },
      { at: 40, viewport: false },
      { at: 60, viewport: false },
      { at: 60, viewport: false },
      { at: 65, viewport: false },
      { at: 70, viewport: false },
    ]);
    // The dragged charm's own edges and centre (200 / 230 / 260, 300 / 315 / 330).
    for (const at of [200, 230, 260]) expect(t.xs.some((l) => l.at === at)).toBe(false);
    for (const at of [300, 315, 330]) expect(t.ys.some((l) => l.at === at)).toBe(false);
    // Alone, a charm has the viewport's lines only.
    expect(snapTargets([laid[1]!], "me", { w: 640, h: 360 }).xs.map((l) => l.at)).toEqual([0, 320, 640]);
  });

  it("uses the charms as laid out in the press's viewport, not as stored", () => {
    // Designed at 480 × 480; laid out in 960 × 540 the picture keeps its
    // size and its centre fraction, so its lines are the layout's.
    const s: OverlayScene = {
      version: 1,
      designSize: { w: 480, h: 480 },
      elements: [
        { id: "c", type: "spellcircle", x: 0.25, y: 0.25, w: 0.5, h: 0.5, z: 0 },
        { id: "p", type: "image", x: 0.5, y: 0.5, w: 0.25, h: 0.125, z: 1, src: "0123456789abcdef.png", nw: 240, nh: 120 },
      ],
    };
    const v = { w: 960, h: 720 };
    const laid = layoutScene(s, v);
    const pic = laid.find((e) => e.el.id === "p")!.rect;
    // As stored it would be 240, 240, 120 × 60.
    expect(pic).toEqual({ x: 540, y: 375, w: 120, h: 60 });
    const t = snapTargets(laid, "c", v);
    expect(t.xs.map((l) => l.at)).toEqual([0, 480, 960, 540, 600, 660]);
    expect(t.ys.map((l) => l.at)).toEqual([0, 360, 720, 375, 405, 435]);
  });
});

describe("snapMove (v1.62.0, §7.5)", () => {
  const T = targetsWith(OTHER);

  it.each([
    ["centre 236 → 240 (d 4); top 10 → 0 is 10 > 8", { x: 186, y: 10, w: 100, h: 40 }, { x: 190, y: 10 }, [{ axis: "x", at: 240 }]],
    [
      "left → 300, centre → 350, right → 400 all d 8, all element lines: the lowest wins",
      { x: 292, y: 300, w: 100, h: 40 },
      { x: 300, y: 300 },
      [{ axis: "x", at: 300 }],
    ],
    ["d 8.01: no snap", { x: 291.99, y: 300, w: 100, h: 40 }, { x: 291.99, y: 300 }, []],
    ["already on a line: a guide, no move", { x: 236, y: 300, w: 8, h: 8 }, { x: 236, y: 300 }, [{ axis: "x", at: 240 }]],
    ["right 476 → 480 (d 4) ends exactly at the edge: kept", { x: 376, y: 300, w: 100, h: 40 }, { x: 380, y: 300 }, [{ axis: "x", at: 480 }]],
    // moveRect clamped a drag to x 384 back to 380.
    ["the preview already touches the edge: a guide, no move", moveRect({ x: 370, y: 300, w: 100, h: 40 }, 14, 0, V480), { x: 380, y: 300 }, [{ axis: "x", at: 480 }]],
    [
      "each axis on its own: top 96 → 100 (d 4) too",
      { x: 186, y: 96, w: 100, h: 40 },
      { x: 190, y: 100 },
      [
        { axis: "x", at: 240 },
        { axis: "y", at: 100 },
      ],
    ],
  ] as Array<[string, PxRect, { x: number; y: number }, Guide[]]>)("%s", (_why, preview, want, guides) => {
    const out = snapMove(preview, T, V480, true);
    expect(out.rect).toEqual({ ...want, w: preview.w, h: preview.h });
    expect(out.guides).toEqual(guides);
    // Ctrl: the preview itself, no guide (D13).
    const free = snapMove(preview, T, V480, snapOn(CTRL));
    expect(free.rect).toEqual(preview);
    expect(free.guides).toEqual([]);
    // Alt and Shift don't bypass.
    expect(snapMove(preview, T, V480, snapOn({ ...NONE, altKey: true }))).toEqual(out);
    expect(snapMove(preview, T, V480, snapOn({ ...NONE, shiftKey: true }))).toEqual(out);
  });

  it("a tie goes to the viewport's line before an element's, even a lower one", () => {
    // Centre 236: the element line 232 and the viewport's 240 are both 4 away.
    const t = targetsWith({ x: 182, y: 400, w: 50, h: 20 });
    expect(t.xs.some((l) => l.at === 232 && !l.viewport)).toBe(true);
    const out = snapMove({ x: 186, y: 300, w: 100, h: 40 }, t, V480, true);
    expect(out.rect.x).toBe(190);
    expect(out.guides).toEqual([{ axis: "x", at: 240 }]);
  });

  it("then the lower line, then left before centre before right", () => {
    // Left 100 and centre 104 are both 2 from the element line 102.
    const t = targetsWith({ x: 102, y: 400, w: 60, h: 20 });
    const out = snapMove({ x: 100, y: 300, w: 8, h: 8 }, t, V480, true);
    expect(out.rect.x).toBe(102);
    expect(out.guides).toEqual([{ axis: "x", at: 102 }]);
    // The same tie between two lines: the lower one (left 100 → 96 and
    // right 116 → 120 are both 4 away).
    const two = snapMove({ x: 100, y: 300, w: 16, h: 8 }, { xs: lines(120, 96), ys: [] }, V480, true);
    expect(two.rect.x).toBe(96);
    expect(two.guides).toEqual([{ axis: "x", at: 96 }]);
  });

  it("a nearer snap that would leave the viewport is passed over", () => {
    // Centre 12 → 8 (d −4) would put the left edge at −2; left 2 → 8 (d 6)
    // keeps it inside. (The viewport's own line 0 is left out here: with it,
    // the left edge is always nearer.)
    const out = snapMove({ x: 2, y: 300, w: 20, h: 20 }, { xs: lines(8), ys: [] }, V480, true);
    expect(out.rect.x).toBe(8);
    expect(out.guides).toEqual([{ axis: "x", at: 8 }]);
    // Wider than the viewport: no shift keeps it inside, so no x guide.
    expect(snapMove({ x: 0, y: 0, w: 600, h: 40 }, T, V480, true).guides).toEqual([{ axis: "y", at: 0 }]);
  });

  it("never changes the size", () => {
    const out = snapMove({ x: 186.3, y: 96.7, w: 100.25, h: 40.5 }, T, V480, true);
    expect(out.rect.w).toBe(100.25);
    expect(out.rect.h).toBe(40.5);
    expect(out.guides.length).toBe(2);
  });
});

describe("snapResize (v1.62.0, §7.5)", () => {
  // rect0 {100, 100, 100, 50} from se: 36, 18 along the diagonal is × 1.36.
  const R0: PxRect = { x: 100, y: 100, w: 100, h: 50 };
  const P0 = resizeRect(R0, "se", 36, 18, V480);
  const SMALL: PxRect = { x: 100, y: 100, w: 48, h: 24 };
  const P1 = resizeRect(SMALL, "se", 2, 1, V480);

  it("the previews are the table's", () => {
    closeRect(P0, { x: 100, y: 100, w: 136, h: 68 });
    closeRect(P1, { x: 100, y: 100, w: 50, h: 25 });
  });

  it.each([
    ["x 240 (d 4)", R0, P0, [OTHER], { x: 100, y: 100, w: 140, h: 70 }, [{ axis: "x", at: 240 }]],
    [
      "x 240 (d 4), y 169.5 (d 1.5): y first, the smaller d",
      R0,
      P0,
      [OTHER, { x: 20, y: 169.5, w: 20, h: 20 }],
      { x: 100, y: 100, w: 139, h: 69.5 },
      [{ axis: "y", at: 169.5 }],
    ],
    [
      "x 145 (d −5) gives 45 × 22.5, below 24: refused; no y line: unsnapped",
      SMALL,
      P1,
      [{ x: 145, y: 300, w: 40, h: 40 }],
      P1,
      [],
    ],
  ] as Array<[string, PxRect, PxRect, PxRect[], PxRect, Guide[]]>)("se: %s", (_why, rect0, preview, others, want, guides) => {
    const t = targetsWith(...others);
    const out = snapResize(preview, rect0, "se", t, V480, true);
    closeRect(out.rect, want);
    expect(out.guides).toEqual(guides);
    // Ctrl: unsnapped, no guide.
    const free = snapResize(preview, rect0, "se", t, V480, snapOn(CTRL));
    expect(free.rect).toEqual(preview);
    expect(free.guides).toEqual([]);
  });

  it.each([
    // nw: the left and top edges move; (300, 250) stays.
    ["nw", { x: 200, y: 200, w: 100, h: 50 }, [-36, -18], { x: 60, y: 400, w: 100, h: 50 }, { x: 160, y: 180, w: 140, h: 70 }, { axis: "x", at: 160 }],
    // ne: the right and top edges move; (100, 250) stays.
    ["ne", { x: 100, y: 200, w: 100, h: 50 }, [36, -18], { x: 400, y: 400, w: 40, h: 40 }, { x: 100, y: 180, w: 140, h: 70 }, { axis: "x", at: 240 }],
    // sw: the left and bottom edges move; (300, 100) stays.
    ["sw", { x: 200, y: 100, w: 100, h: 50 }, [-36, 18], { x: 400, y: 170, w: 40, h: 40 }, { x: 160, y: 100, w: 140, h: 70 }, { axis: "y", at: 170 }],
  ] as Array<[Corner, PxRect, [number, number], PxRect, PxRect, Guide]>)(
    "%s: the moving edge snaps, the opposite corner stays",
    (corner, rect0, [dx, dy], other, want, guide) => {
      const preview = resizeRect(rect0, corner, dx, dy, V480);
      expect(preview.w).toBeCloseTo(136, 9);
      const out = snapResize(preview, rect0, corner, targetsWith(other), V480, true);
      closeRect(out.rect, want);
      expect(out.guides).toEqual([guide]);
    },
  );

  it("the still corner's edges never snap", () => {
    // From se the left edge 236 sits 4 from 240, but it doesn't move.
    const rect0 = { x: 236, y: 300, w: 100, h: 50 };
    const preview = resizeRect(rect0, "se", 20, 10, V480);
    closeRect(preview, { x: 236, y: 300, w: 120, h: 60 });
    const out = snapResize(preview, rect0, "se", targetsWith(), V480, true);
    expect(out).toEqual({ rect: preview, guides: [] });
    // A move of the same rect would snap it.
    expect(snapMove(preview, targetsWith(), V480, true).guides).toEqual([{ axis: "x", at: 240 }]);
  });

  it("a line that breaks the minimum gives way to the other axis, or to a farther line", () => {
    // x 147 (d −3) gives 47 × 23.5: refused. y 129 (d 4) gives 58 × 29.
    const other = snapResize(P1, SMALL, "se", { xs: lines(147), ys: lines(129) }, V480, true);
    closeRect(other.rect, { x: 100, y: 100, w: 58, h: 29 });
    expect(other.guides).toEqual([{ axis: "y", at: 129 }]);
    // x 145 refused; x 156 (d 6) gives 56 × 28.
    const farther = snapResize(P1, SMALL, "se", { xs: lines(145, 156), ys: [] }, V480, true);
    closeRect(farther.rect, { x: 100, y: 100, w: 56, h: 28 });
    expect(farther.guides).toEqual([{ axis: "x", at: 156 }]);
  });

  it("the minimum is scene.ts's, and an element already below it can't shrink by snapping", () => {
    // Exactly 24 is kept: x 148 gives 48 × 24.
    const exact = snapResize(P1, SMALL, "se", { xs: lines(148), ys: [] }, V480, true);
    closeRect(exact.rect, SMALL);
    expect(exact.guides).toEqual([{ axis: "x", at: 148 }]);
    for (const at of [140, 145, 147.9, 148, 150, 156]) {
      const t = { xs: lines(at), ys: [] };
      expect(snapResize(P1, SMALL, "se", t, V480, true)).toEqual(
        snapResize(P1, SMALL, "se", t, V480, true, MIN_ELEMENT_PX),
      );
    }
    // 20 × 10 in a 30 × 15 viewport can't reach 24: as with resizeRect,
    // snapping may grow it, or shrink it back to its own size, never below.
    const tiny = { x: 0, y: 0, w: 20, h: 10 };
    const v = { w: 30, h: 15 };
    const grown = resizeRect(tiny, "se", 2, 1, v);
    closeRect(grown, { x: 0, y: 0, w: 22, h: 11 });
    expect(snapResize(grown, tiny, "se", { xs: lines(19), ys: [] }, v, true).guides).toEqual([]);
    closeRect(snapResize(grown, tiny, "se", { xs: lines(21), ys: [] }, v, true).rect, { x: 0, y: 0, w: 21, h: 10.5 });
    const ok = snapResize(grown, tiny, "se", { xs: lines(24), ys: [] }, v, true);
    closeRect(ok.rect, { x: 0, y: 0, w: 24, h: 12 });
  });

  it("a snap that would leave the viewport is refused", () => {
    // rect0's bottom can grow only to 480 (× 1.36): x 240 would need × 1.4.
    const rect0 = { x: 100, y: 412, w: 100, h: 50 };
    const preview = resizeRect(rect0, "se", 36, 18, V480);
    closeRect(preview, { x: 100, y: 412, w: 136, h: 68 });
    const out = snapResize(preview, rect0, "se", { xs: [{ at: 240, viewport: true }], ys: [] }, V480, true);
    expect(out).toEqual({ rect: preview, guides: [] });
    // With the viewport's lines, its bottom is on 480 already: a guide, no change.
    const all = snapResize(preview, rect0, "se", targetsWith(), V480, true);
    closeRect(all.rect, preview);
    expect(all.guides).toEqual([{ axis: "y", at: 480 }]);
  });

  it("an element below the minimum that can reach it snaps no smaller than a drag can make it", () => {
    // 24 × 12: resizeRect never gives it less than 48 × 24 here, and the
    // reducer grows anything smaller about its centre, so a snap to 41 × 20.5
    // would leave its line at once.
    const rect0 = { x: 100, y: 100, w: 24, h: 12 };
    const preview = resizeRect(rect0, "se", 10, 5, V480);
    closeRect(preview, { x: 100, y: 100, w: 48, h: 24 });
    const out = snapResize(preview, rect0, "se", { xs: lines(141), ys: [] }, V480, true);
    expect(out).toEqual({ rect: preview, guides: [] });
    const stored = normalizeRect({ x: 100, y: 100, w: 41, h: 20.5 }, 2, V480);
    expect(stored.x + stored.w).not.toBeCloseTo(141, 0);
    // A line it can reach at 24 or more still snaps.
    const ok = snapResize(preview, rect0, "se", { xs: lines(152), ys: [] }, V480, true);
    closeRect(ok.rect, { x: 100, y: 100, w: 52, h: 26 });
    expect(ok.guides).toEqual([{ axis: "x", at: 152 }]);
  });

  it("a tie between the axes goes to x", () => {
    // Right 136 → 140 and bottom 68 → 72 are both 4 away (rect0 2 : 1 at 0, 0).
    const rect0 = { x: 0, y: 0, w: 100, h: 50 };
    const preview = resizeRect(rect0, "se", 36, 18, V480);
    const out = snapResize(preview, rect0, "se", { xs: lines(140), ys: lines(72) }, V480, true);
    closeRect(out.rect, { x: 0, y: 0, w: 140, h: 70 });
    expect(out.guides).toEqual([{ axis: "x", at: 140 }]);
  });

  it("a wide charm's y snap never moves its x edge more than 8 px: a 5 : 1 light", () => {
    // The light's default shape, 150 × 30 at (100, 200), alone. Its bottom
    // edge 8 px above the middle line (y 240) would snap to 200 × 40: the
    // right edge 40 px from the pointer, then held there across 80 px.
    const rect0 = { x: 100, y: 200, w: 150, h: 30 };
    const t = targetsWith();
    const preview = resizeRect(rect0, "se", 10, 2, V480);
    closeRect(preview, { x: 100, y: 200, w: 160, h: 32 });
    expect(snapResize(preview, rect0, "se", t, V480, true)).toEqual({ rect: preview, guides: [] });
    // Within 8 / 5 px of the line it snaps, the right edge moving 4.
    const close = resizeRect(rect0, "se", 46, 9.2, V480);
    closeRect(close, { x: 100, y: 200, w: 196, h: 39.2 });
    const out = snapResize(close, rect0, "se", t, V480, true);
    closeRect(out.rect, { x: 100, y: 200, w: 200, h: 40 });
    expect(out.guides).toEqual([{ axis: "y", at: 240 }]);
    // A sweep of the corner along its diagonal: neither edge ever lands
    // more than 8 px from where the pointer puts it.
    for (let k = 0; k <= 200; k += 0.5) {
      const p = resizeRect(rect0, "se", k, k / 5, V480);
      const o = snapResize(p, rect0, "se", t, V480, true).rect;
      expect(Math.abs(o.w - p.w), `k ${k}`).toBeLessThanOrEqual(SNAP_PX + 1e-9);
      expect(Math.abs(o.h - p.h), `k ${k}`).toBeLessThanOrEqual(SNAP_PX + 1e-9);
    }
  });

  it("a tall charm's x snap never moves its y edge more than 8 px", () => {
    // 30 × 300 (1 : 10) at (200, 20), grown to 34 × 340: its right edge 6 px
    // short of x 240 would make it 40 × 400, its bottom edge 60 px from the
    // pointer. Refused; a line 0.5 px away still snaps (the bottom moves 5).
    const rect0 = { x: 200, y: 20, w: 30, h: 300 };
    const preview = resizeRect(rect0, "se", 4, 40, V480);
    closeRect(preview, { x: 200, y: 20, w: 34, h: 340 });
    const out = snapResize(preview, rect0, "se", { xs: lines(240), ys: [] }, V480, true);
    expect(out).toEqual({ rect: preview, guides: [] });
    const near = snapResize(preview, rect0, "se", { xs: lines(234.5), ys: [] }, V480, true);
    closeRect(near.rect, { x: 200, y: 20, w: 34.5, h: 345 });
    expect(near.guides).toEqual([{ axis: "x", at: 234.5 }]);
  });
});

// ---- properties ----

/** mulberry32: a small seeded generator, so every run sees the same cases. */
function seeded(seed: number): () => number {
  let a = seed >>> 0;
  return () => {
    a = (a + 0x6d2b79f5) >>> 0;
    let t = a;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

type Rand = () => number;
const between = (r: Rand, lo: number, hi: number): number => lo + (hi - lo) * r();
const pick = <T>(r: Rand, xs: readonly T[]): T => xs[Math.floor(r() * xs.length)]!;

const CORNERS: Corner[] = ["nw", "ne", "sw", "se"];
const sxOf = (c: Corner) => (c === "ne" || c === "se" ? 1 : -1);
const syOf = (c: Corner) => (c === "sw" || c === "se" ? 1 : -1);

/** A rect's start, centre and end on one axis. */
const spans = (r: PxRect, axis: "x" | "y"): number[] =>
  axis === "x" ? [r.x, r.x + r.w / 2, r.x + r.w] : [r.y, r.y + r.h / 2, r.y + r.h];

/** The edge a corner resize moves on one axis. */
const movingEdge = (r: PxRect, c: Corner, axis: "x" | "y"): number =>
  axis === "x" ? (sxOf(c) > 0 ? r.x + r.w : r.x) : syOf(c) > 0 ? r.y + r.h : r.y;

/** The corner a resize from `c` holds still. */
const still = (r: PxRect, c: Corner): [number, number] => [
  sxOf(c) > 0 ? r.x : r.x + r.w,
  syOf(c) > 0 ? r.y : r.y + r.h,
];

function inside(r: PxRect, v: Size, tol = 1e-9): boolean {
  return r.x >= -tol && r.y >= -tol && r.x + r.w <= v.w + tol && r.y + r.h <= v.h + tol;
}

/** `t` without the viewport's own lines: the other charms' alone. */
const charmsOnly = (t: SnapTargets): SnapTargets => ({
  xs: t.xs.filter((l) => !l.viewport),
  ys: t.ys.filter((l) => !l.viewport),
});

/** A random viewport, charms laid out in it and a dragged rect `me`. */
function arena(r: Rand): { v: Size; laid: Array<{ el: { id: string }; rect: PxRect }>; me: PxRect } {
  const v = { w: Math.round(between(r, 240, 1920)), h: Math.round(between(r, 240, 1080)) };
  const box = (): PxRect => {
    const w = between(r, 24, v.w * 0.5);
    const h = between(r, 24, v.h * 0.5);
    return { x: between(r, 0, v.w - w), y: between(r, 0, v.h - h), w, h };
  };
  const others = Array.from({ length: Math.floor(between(r, 0, 5)) }, (_, i) => ({ el: { id: `o${i}` }, rect: box() }));
  const me = box();
  return { v, laid: [...others, { el: { id: "me" }, rect: me }], me };
}

/** A pointer offset on one axis: mostly aimed so `from` lands within ±10 px
 *  of one of `ls`, else anywhere. */
function aim(r: Rand, from: number, ls: readonly SnapLine[]): number {
  return r() < 0.75 ? pick(r, ls).at - from + between(r, -10, 10) : between(r, -300, 300);
}

/** A pointer offset along a corner's diagonal (resizeRect projects onto it):
 *  mostly aimed so one moving edge lands within ±10 px of one of `t`'s
 *  lines, else any scale from 0.3 to 3. */
function aimCorner(r: Rand, rect: PxRect, c: Corner, t: SnapTargets): { dx: number; dy: number } {
  const [ox, oy] = still(rect, c);
  let s = between(r, 0.3, 3);
  if (r() < 0.75) {
    const x = r() < 0.5;
    const to = pick(r, x ? t.xs : t.ys).at + between(r, -10, 10);
    s = x ? (sxOf(c) * (to - ox)) / rect.w : (syOf(c) * (to - oy)) / rect.h;
  }
  return { dx: sxOf(c) * (s - 1) * rect.w, dy: syOf(c) * (s - 1) * rect.h };
}

describe("snapping properties (v1.62.0, 500 seeded cases each)", () => {
  it("move: inside, at most 8 px an axis, to the nearest line that fits, guides on the result", () => {
    const r = seeded(0x5a9);
    let snapped = 0;
    for (let i = 0; i < 500; i++) {
      const { v, laid, me } = arena(r);
      const t = snapTargets(laid, "me", v);
      const k = Math.floor(r() * 3);
      const dx = aim(r, spans(me, "x")[k]!, t.xs);
      const dy = aim(r, spans(me, "y")[Math.floor(r() * 3)]!, t.ys);
      const preview = moveRect(me, dx, dy, v);
      const on = r() < 0.8;
      // The viewport's own edge line is always nearer than, or tied with,
      // a shift that would leave it, and wins the tie: with it, nothing
      // here can tell whether a shift out of the viewport is refused. So
      // each case also runs on the other charms' lines alone.
      for (const tt of [t, charmsOnly(t)]) {
        const out = snapMove(preview, tt, v, on);
        expect(inside(out.rect, v)).toBe(true);
        expect(out.rect.w).toBe(preview.w);
        expect(out.rect.h).toBe(preview.h);
        if (!on) {
          expect(out).toEqual({ rect: preview, guides: [] });
          continue;
        }
        if (tt === t && out.guides.length > 0) snapped += 1;
        for (const axis of ["x", "y"] as const) {
          const ls = axis === "x" ? tt.xs : tt.ys;
          const extent = axis === "x" ? v.w : v.h;
          const size = axis === "x" ? preview.w : preview.h;
          const from = axis === "x" ? preview.x : preview.y;
          const to = axis === "x" ? out.rect.x : out.rect.y;
          // Every (candidate, line) pair within 8 whose shift keeps it inside.
          const fits = spans(preview, axis).flatMap((c, j) =>
            ls
              .map((l) => ({ d: l.at - c, start: l.at - (j * size) / 2 }))
              .filter((p) => Math.abs(p.d) <= SNAP_PX && p.start >= -1e-9 && p.start + size <= extent + 1e-9),
          );
          const guides = out.guides.filter((g) => g.axis === axis);
          expect(Math.abs(to - from)).toBeLessThanOrEqual(SNAP_PX + 1e-6);
          if (fits.length === 0) {
            expect(to).toBe(from);
            expect(guides).toEqual([]);
            continue;
          }
          const nearest = Math.min(...fits.map((p) => Math.abs(p.d)));
          expect(guides.length).toBe(1);
          expect(Math.abs(Math.abs(to - from) - nearest)).toBeLessThan(1e-6);
          // The guide is a target and lies on an edge or the centre of the result.
          expect(ls.some((l) => l.at === guides[0]!.at)).toBe(true);
          expect(Math.min(...spans(out.rect, axis).map((c) => Math.abs(c - guides[0]!.at)))).toBeLessThan(1e-9);
        }
      }
    }
    expect(snapped).toBeGreaterThan(200);
  });

  it("resize: inside, the shape and the still corner kept, each side at most 8 px from the preview, on its guide", () => {
    const r = seeded(0x2e5);
    let snapped = 0;
    let pushed = 0;
    for (let i = 0; i < 500; i++) {
      const { v, laid, me } = arena(r);
      const t = snapTargets(laid, "me", v);
      const c = pick(r, CORNERS);
      const a = me.w / me.h;
      const { dx, dy } = aimCorner(r, me, c, t);
      const preview = resizeRect(me, c, dx, dy, v);
      const on = r() < 0.8;
      const out = snapResize(preview, me, c, t, v, on);
      const o = out.rect;
      expect(inside(o, v)).toBe(true);
      expect(Math.abs(o.w / o.h - a)).toBeLessThan(1e-9 * Math.max(1, a));
      const [ax, ay] = still(o, c);
      const [bx, by] = still(me, c);
      expect(Math.abs(ax - bx)).toBeLessThan(1e-9);
      expect(Math.abs(ay - by)).toBeLessThan(1e-9);
      expect(Math.min(o.w, o.h)).toBeGreaterThanOrEqual(MIN_ELEMENT_PX - 1e-9);
      expect(out.guides.length).toBeLessThanOrEqual(1);
      // §7.5: each axis moves at most 8 from the preview, the side the
      // shape derives as well as the snapped one.
      expect(Math.abs(o.w - preview.w)).toBeLessThanOrEqual(SNAP_PX + 1e-6);
      expect(Math.abs(o.h - preview.h)).toBeLessThanOrEqual(SNAP_PX + 1e-6);
      // Pressed against the viewport: dragged far past it, the preview stops
      // where the first wall binds. A line 1 to 8 px past the other axis's
      // moving edge can only be reached by pushing the bound side out, so it
      // is never taken (the aimed cases above rarely press a wall).
      const pressed = resizeRect(me, c, sxOf(c) * 4 * v.w, syOf(c) * 4 * v.h, v);
      const gap = (axis: "x" | "y"): number => {
        const wall = axis === "x" ? (sxOf(c) > 0 ? v.w : 0) : syOf(c) > 0 ? v.h : 0;
        return Math.abs(movingEdge(pressed, c, axis) - wall);
      };
      const free = gap("x") >= gap("y") ? "x" : "y";
      const past = 1 + (i % 8);
      const line = { at: movingEdge(pressed, c, free) + (free === "x" ? sxOf(c) : syOf(c)) * past, viewport: false };
      expect(snapResize(pressed, me, c, free === "x" ? { xs: [line], ys: [] } : { xs: [], ys: [line] }, v, true)).toEqual({
        rect: pressed,
        guides: [],
      });
      // Within 8 on the bound side too, so only the inside rule refuses it.
      if ((free === "x" ? past / a : past * a) <= SNAP_PX) pushed += 1;
      if (!on) {
        expect(out).toEqual({ rect: preview, guides: [] });
        continue;
      }
      // Every line within 8 of a moving edge that gives a size resizeRect
      // could (at least 24 on the short side, inside) with neither side
      // more than 8 from the preview's.
      const [ox, oy] = still(me, c);
      const sx = sxOf(c);
      const sy = syOf(c);
      const fits = [
        ...t.xs.map((l) => {
          const w = sx * (l.at - ox);
          return { d: l.at - movingEdge(preview, c, "x"), w, h: w / a };
        }),
        ...t.ys.map((l) => {
          const h = sy * (l.at - oy);
          return { d: l.at - movingEdge(preview, c, "y"), w: h * a, h };
        }),
      ].filter((p) => {
        if (Math.abs(p.d) > SNAP_PX || !(Math.min(p.w, p.h) >= MIN_ELEMENT_PX - 1e-9)) return false;
        if (Math.abs(p.w - preview.w) > SNAP_PX + 1e-9 || Math.abs(p.h - preview.h) > SNAP_PX + 1e-9) return false;
        return inside({ x: sx > 0 ? ox : ox - p.w, y: sy > 0 ? oy : oy - p.h, w: p.w, h: p.h }, v, 1e-9);
      });
      if (fits.length === 0) {
        expect(out).toEqual({ rect: preview, guides: [] });
        continue;
      }
      snapped += 1;
      const g = out.guides[0]!;
      const edge = movingEdge(o, c, g.axis);
      const was = movingEdge(preview, c, g.axis);
      expect(Math.abs(edge - g.at)).toBeLessThan(1e-9);
      expect(Math.abs(g.at - was)).toBeLessThanOrEqual(SNAP_PX + 1e-6);
      expect(Math.abs(Math.abs(g.at - was) - Math.min(...fits.map((p) => Math.abs(p.d))))).toBeLessThan(1e-6);
      expect((g.axis === "x" ? t.xs : t.ys).some((l) => l.at === g.at)).toBe(true);
    }
    expect(snapped).toBeGreaterThan(150);
    expect(pushed).toBeGreaterThan(100);
  });
});

// ---- through the reducer ----

const IMG = "0123456789abcdef.png";

/** Fractions of `d` for a box of `w × h` px whose top-left is at `x, y` px. */
const frac = (d: Size, x: number, y: number, w: number, h: number) => ({
  x: Math.round((x / d.w) * 1e6) / 1e6,
  y: Math.round((y / d.h) * 1e6) / 1e6,
  w: Math.round((w / d.w) * 1e6) / 1e6,
  h: Math.round((h / d.h) * 1e6) / 1e6,
});

/** Drag `id` in `s` laid out in `v`, snapping, then store the release and lay
 *  it out again. Returns the gesture and the stored element's new layout. */
function roundTrip(
  s: OverlayScene,
  v: Size,
  id: string,
  how: { corner?: Corner; dx: number; dy: number },
): { guides: Guide[]; preview: PxRect; after: PxRect } {
  const laid = layoutScene(s, v);
  const rect0 = laid.find((e) => e.el.id === id)!.rect;
  const targets = snapTargets(laid, id, v);
  const at = { x: 500, y: 500 };
  let g = how.corner ? beginResize(id, how.corner, 1, at, rect0, v, targets) : beginMove(id, 1, at, rect0, v, targets);
  g = updateGesture(g, { x: at.x + how.dx, y: at.y + how.dy });
  const op = finishGesture(g);
  const s2 = op ? applyOp(s, op) : s;
  const el = liveElements(s2).find((e) => e.id === id)!;
  return { guides: g.guides, preview: g.preview, after: layoutElement(el, s2.designSize, v)! };
}

/** The snapped edge or centre of `after`, the one that was on `g` in the
 *  preview (a move), or the moving edge (a resize). */
function onGuide(g: Guide, preview: PxRect, after: PxRect, corner?: Corner): number {
  if (corner) return movingEdge(after, corner, g.axis);
  const j = spans(preview, g.axis).findIndex((c) => Math.abs(c - g.at) < 1e-6);
  expect(j).toBeGreaterThanOrEqual(0);
  return spans(after, g.axis)[j]!;
}

describe("place round trip (v1.62.0)", () => {
  // One of each kind, each alone in its scene, so its only lines are the
  // viewport's. Designed at 480 × 480, laid out larger (designSize grows on
  // the place) and smaller.
  const D = { w: 480, h: 480 };
  const kinds: Array<[string, SceneElement]> = [
    ["circle", { id: "circle", type: "spellcircle", ...frac(D, 60, 60, 120, 120), z: 0 }],
    ["picture", { id: "p", type: "image", ...frac(D, 60, 60, 120, 90), z: 0, src: IMG, nw: 400, nh: 300 }],
    ["timer", { id: "f", type: "timer", ...frac(D, 60, 60, 150, 50), z: 0, timer: "brb" }],
    [
      "text",
      {
        id: "t",
        type: "text",
        ...frac(D, 60, 60, 144, 48),
        z: 0,
        text: "BRB",
        font: "sans",
        bold: true,
        color: "#ffffff",
        outline: true,
        align: "center",
        nw: 186,
        nh: 62,
      },
    ],
    ["light", { id: "rec", type: "rec", ...frac(D, 60, 60, 150, 30), z: 0 }],
  ];

  for (const v of [
    { w: 960, h: 540 },
    { w: 400, h: 360 },
  ]) {
    it.each(kinds)(`%s in ${v.w} × ${v.h}: a move's snapped centre, and a resize's snapped edge, stay on their line`, (_k, el) => {
      const s: OverlayScene = { version: 1, designSize: D, elements: [el] };
      // The element must be live here, or the place changes nothing.
      expect(isLive(s, el.id)).toBe(true);
      const rect0 = layoutScene(s, v)[0]!.rect;
      // Its centre to 5 px left of the viewport's vertical centre line.
      const cx = rect0.x + rect0.w / 2;
      const move = roundTrip(s, v, el.id, { dx: v.w / 2 - 5 - cx, dy: 0 });
      expect(move.guides).toEqual([{ axis: "x", at: v.w / 2 }]);
      expect(Math.abs(onGuide(move.guides[0]!, move.preview, move.after) - v.w / 2)).toBeLessThanOrEqual(0.5);
      // Its se corner along the diagonal until the right edge is 5 px short
      // of the centre line.
      const k = (v.w / 2 - 5 - rect0.x) / rect0.w - 1;
      const grow = roundTrip(s, v, el.id, { corner: "se", dx: k * rect0.w, dy: k * rect0.h });
      expect(grow.guides).toEqual([{ axis: "x", at: v.w / 2 }]);
      expect(grow.after.w).toBeGreaterThan(rect0.w + 1);
      expect(Math.abs(onGuide(grow.guides[0]!, grow.preview, grow.after, "se") - v.w / 2)).toBeLessThanOrEqual(0.5);
    });
  }

  it("500 seeded drags of every kind among others: the snapped line holds", () => {
    const r = seeded(0x7a1);
    let checked = 0;
    for (let i = 0; i < 500; i++) {
      const d = { w: Math.round(between(r, 320, 1920)), h: Math.round(between(r, 240, 1080)) };
      const v = r() < 0.4 ? d : { w: Math.round(between(r, 320, 1920)), h: Math.round(between(r, 240, 1080)) };
      const elements: SceneElement[] = [];
      // Short side 30 px or more, long side at most half the smaller of
      // the design and the viewport, so it lays out unshrunk.
      const box = (a: number): ReturnType<typeof frac> => {
        const short = between(r, 30, 0.2 * Math.min(d.w, d.h, v.w, v.h));
        const w = a >= 1 ? short * a : short;
        const h = a >= 1 ? short : short / a;
        return frac(d, between(r, 0, d.w - w), between(r, 0, d.h - h), w, h);
      };
      if (r() < 0.5) elements.push({ id: "circle", type: "spellcircle", ...box(1), z: elements.length });
      for (let j = 0; j < 1 + Math.floor(r() * 3); j++) {
        const nw = Math.round(between(r, 60, 2000));
        const nh = Math.round(nw / between(r, 0.4, 2.5));
        elements.push({ id: `p${j}`, type: "image", ...box(nw / nh), z: elements.length, src: IMG, nw, nh });
      }
      if (r() < 0.6) elements.push({ id: "f", type: "timer", ...box(between(r, 1.5, 4)), z: elements.length, timer: "brb" });
      if (r() < 0.6) {
        const nw = Math.round(between(r, 40, 900));
        const nh = Math.round(between(r, 60, 300));
        elements.push({ id: "t", type: "text", ...box(nw / nh), z: elements.length, text: "Be right back", nw, nh });
      }
      if (r() < 0.5) elements.push({ id: "rec", type: "rec", ...box(between(r, 3, 6)), z: elements.length });
      const s: OverlayScene = { version: 1, designSize: d, elements };
      expect(liveElements(s).length).toBe(elements.length);
      const laid = layoutScene(s, v);
      const { el, rect: rect0 } = pick(r, laid);
      if (Math.min(rect0.w, rect0.h) < MIN_ELEMENT_PX + 1) continue;
      const t = snapTargets(laid, el.id, v);
      const corner = r() < 0.5 ? pick(r, CORNERS) : undefined;
      const { dx, dy } = corner
        ? aimCorner(r, rect0, corner, t)
        : {
            dx: aim(r, spans(rect0, "x")[Math.floor(r() * 3)]!, t.xs),
            dy: aim(r, spans(rect0, "y")[Math.floor(r() * 3)]!, t.ys),
          };
      const out = roundTrip(s, v, el.id, { corner, dx, dy });
      for (const g of out.guides) {
        expect(Math.abs(onGuide(g, out.preview, out.after, corner) - g.at)).toBeLessThanOrEqual(0.5);
        checked += 1;
      }
    }
    expect(checked).toBeGreaterThan(200);
  });
});
