import { describe, expect, it } from "vitest";
import type { Point } from "./glyphs";
import {
  makeTemplate,
  normalize,
  recognize,
  resample,
  RESAMPLE_N,
  type UnistrokeTemplate,
} from "./unistroke";

/** Build a polyline by sampling `perSeg` points along each edge. */
function poly(verts: Point[], perSeg = 12): Point[] {
  const out: Point[] = [];
  for (let i = 0; i < verts.length - 1; i++) {
    const a = verts[i]!;
    const b = verts[i + 1]!;
    for (let s = 0; s < perSeg; s++) {
      out.push({ x: a.x + ((b.x - a.x) * s) / perSeg, y: a.y + ((b.y - a.y) * s) / perSeg });
    }
  }
  out.push(verts[verts.length - 1]!);
  return out;
}

const SQUARE = poly([
  { x: 0, y: 0 }, { x: 100, y: 0 }, { x: 100, y: 100 }, { x: 0, y: 100 }, { x: 0, y: 0 },
]);
const L_SHAPE = poly([{ x: 0, y: 0 }, { x: 0, y: 100 }, { x: 100, y: 100 }]);
const CARET_UP = poly([{ x: 0, y: 100 }, { x: 50, y: 0 }, { x: 100, y: 100 }]);
const CARET_DOWN = poly([{ x: 0, y: 0 }, { x: 50, y: 100 }, { x: 100, y: 0 }]);

/** Deterministic pseudo-random jitter (so the test is stable). */
function jitter(pts: Point[], amt: number, seed = 7): Point[] {
  let s = seed;
  const rnd = (): number => {
    s = (s * 1103515245 + 12345) & 0x7fffffff;
    return (s / 0x7fffffff) * 2 - 1;
  };
  return pts.map((p) => ({ x: p.x + rnd() * amt, y: p.y + rnd() * amt }));
}

describe("unistroke recognizer", () => {
  it("resample yields exactly N points", () => {
    expect(resample(SQUARE).length).toBe(RESAMPLE_N);
    expect(resample(L_SHAPE, 32).length).toBe(32);
  });

  it("normalize recenters near origin and bounds to ~unit", () => {
    const n = normalize(resample(SQUARE));
    const cx = n.reduce((sum, p) => sum + p.x, 0) / n.length;
    const cy = n.reduce((sum, p) => sum + p.y, 0) / n.length;
    expect(Math.abs(cx)).toBeLessThan(0.05);
    expect(Math.abs(cy)).toBeLessThan(0.05);
    for (const p of n) {
      expect(Math.abs(p.x)).toBeLessThanOrEqual(0.75);
      expect(Math.abs(p.y)).toBeLessThanOrEqual(0.75);
    }
  });

  it("a stroke matches itself with a near-perfect score", () => {
    const tpl: UnistrokeTemplate = { id: "square", points: makeTemplate(SQUARE) };
    const m = recognize(SQUARE, [tpl]);
    expect(m?.id).toBe("square");
    expect(m!.score).toBeGreaterThan(0.97);
  });

  it("matches a noisy redraw of the same stroke", () => {
    const tpl: UnistrokeTemplate = { id: "square", points: makeTemplate(SQUARE) };
    expect(recognize(jitter(SQUARE, 4), [tpl])?.id).toBe("square");
  });

  it("rejects a clearly different stroke", () => {
    const tpl: UnistrokeTemplate = { id: "square", points: makeTemplate(SQUARE) };
    expect(recognize(L_SHAPE, [tpl])).toBeNull();
  });

  it("is orientation-preserving (^ is not v)", () => {
    const up: UnistrokeTemplate = { id: "caret-up", points: makeTemplate(CARET_UP) };
    const down: UnistrokeTemplate = { id: "caret-down", points: makeTemplate(CARET_DOWN) };
    expect(recognize(CARET_UP, [up, down])?.id).toBe("caret-up");
    expect(recognize(CARET_DOWN, [up])).toBeNull();
  });

  it("returns null with no templates or too-short paths", () => {
    expect(recognize(SQUARE, [])).toBeNull();
    expect(
      recognize([{ x: 0, y: 0 }, { x: 1, y: 1 }], [{ id: "x", points: makeTemplate(SQUARE) }]),
    ).toBeNull();
  });
});
