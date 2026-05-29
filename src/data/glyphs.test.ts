import { describe, expect, it } from "vitest";
import {
  boundingBox,
  CLASSIFIER_DEFAULTS,
  classifyGlyph,
  dedupe,
  endpointGap,
  findCorners,
  pathLength,
  resamplePath,
  smooth,
  turningAngles,
  type Point,
} from "./glyphs";

/** Sample N points equally spaced around an arc of `theta` radians. */
function arcPoints(
  cx: number,
  cy: number,
  r: number,
  thetaStart: number,
  thetaEnd: number,
  n: number,
): Point[] {
  const out: Point[] = [];
  for (let i = 0; i < n; i++) {
    const t = thetaStart + ((thetaEnd - thetaStart) * i) / (n - 1);
    out.push({ x: cx + Math.cos(t) * r, y: cy + Math.sin(t) * r });
  }
  return out;
}

/** A polygon traced by walking around its vertices with `segPoints` per side. */
function polygon(vertices: Point[], segPoints: number, closed = true): Point[] {
  const out: Point[] = [];
  const loop = closed ? [...vertices, vertices[0]!] : vertices;
  for (let i = 0; i < loop.length - 1; i++) {
    const a = loop[i]!;
    const b = loop[i + 1]!;
    for (let s = 0; s < segPoints; s++) {
      const t = s / segPoints;
      out.push({ x: a.x + (b.x - a.x) * t, y: a.y + (b.y - a.y) * t });
    }
  }
  out.push(loop[loop.length - 1]!);
  return out;
}

describe("glyph classifier — geometry helpers", () => {
  it("dedupe drops consecutive identical points but keeps unique ones", () => {
    const out = dedupe([
      { x: 0, y: 0 },
      { x: 0, y: 0 },
      { x: 1, y: 1 },
      { x: 1, y: 1 },
      { x: 1, y: 1 },
      { x: 2, y: 2 },
    ]);
    expect(out).toEqual([
      { x: 0, y: 0 },
      { x: 1, y: 1 },
      { x: 2, y: 2 },
    ]);
  });

  it("boundingBox spans every point", () => {
    const bb = boundingBox([
      { x: 5, y: 7 },
      { x: 1, y: 10 },
      { x: 12, y: 2 },
    ]);
    expect(bb).toEqual({ xmin: 1, ymin: 2, xmax: 12, ymax: 10 });
  });

  it("pathLength sums segment lengths", () => {
    const len = pathLength([
      { x: 0, y: 0 },
      { x: 3, y: 4 },
      { x: 3, y: 0 },
    ]);
    expect(len).toBeCloseTo(5 + 4, 5);
  });

  it("resamplePath produces exactly N points along a straight line", () => {
    const out = resamplePath(
      [
        { x: 0, y: 0 },
        { x: 10, y: 0 },
      ],
      11,
    );
    expect(out).toHaveLength(11);
    expect(out[0]!.x).toBeCloseTo(0, 5);
    expect(out[10]!.x).toBeCloseTo(10, 5);
    // Points should be uniformly spaced.
    expect(out[5]!.x).toBeCloseTo(5, 5);
  });

  it("smooth with halfWindow=1 averages neighbours and dampens spikes", () => {
    const spiked: Point[] = [
      { x: 0, y: 0 },
      { x: 0, y: 100 },
      { x: 0, y: 0 },
    ];
    const out = smooth(spiked, 1);
    expect(out[1]!.y).toBeLessThan(100);
    expect(out[1]!.y).toBeGreaterThan(0);
  });

  it("turningAngles flag a 90° corner and zero out smooth segments", () => {
    const corner: Point[] = [
      { x: 0, y: 0 },
      { x: 10, y: 0 },
      { x: 10, y: 10 },
    ];
    const angles = turningAngles(corner);
    expect(angles[0]).toBe(0);
    expect(angles[1]).toBeCloseTo(Math.PI / 2, 4);
    expect(angles[2]).toBe(0);
  });

  it("findCorners picks the peak angle when above threshold", () => {
    // Angles where indices 4 and 12 are local maxima above threshold.
    const angles = new Array<number>(20).fill(0);
    angles[4] = 1.5;
    angles[12] = 1.2;
    const corners = findCorners(angles, 1.0, 3);
    expect(corners).toEqual([4, 12]);
  });

  it("endpointGap reports near-zero for a closed loop", () => {
    const closed: Point[] = [
      { x: 0, y: 0 },
      { x: 10, y: 0 },
      { x: 10, y: 10 },
      { x: 0, y: 10 },
      { x: 0, y: 0 },
    ];
    const bb = boundingBox(closed);
    expect(endpointGap(closed, bb)).toBeCloseTo(0, 5);
  });
});

describe("glyph classifier — shape recognition", () => {
  it("classifies a clean circle as 'circle'", () => {
    const points = arcPoints(200, 200, 100, 0, 2 * Math.PI, 64);
    expect(classifyGlyph(points)).toBe("circle");
  });

  it("classifies a slightly noisy circle as 'circle'", () => {
    const base = arcPoints(200, 200, 100, 0, 2 * Math.PI, 64);
    const noisy: Point[] = base.map((p, i) => ({
      x: p.x + Math.sin(i) * 1.2,
      y: p.y + Math.cos(i * 0.7) * 1.2,
    }));
    expect(classifyGlyph(noisy)).toBe("circle");
  });

  it("classifies a square as 'square'", () => {
    const square: Point[] = polygon(
      [
        { x: 50, y: 50 },
        { x: 150, y: 50 },
        { x: 150, y: 150 },
        { x: 50, y: 150 },
      ],
      12,
    );
    expect(classifyGlyph(square)).toBe("square");
  });

  it("classifies an upright triangle as 'triangle' (apex up)", () => {
    const tri: Point[] = polygon(
      [
        { x: 100, y: 30 }, // apex at TOP
        { x: 170, y: 170 },
        { x: 30, y: 170 },
      ],
      12,
    );
    expect(classifyGlyph(tri)).toBe("triangle");
  });

  it("classifies an inverted triangle as 'invtriangle' (apex down)", () => {
    const invtri: Point[] = polygon(
      [
        { x: 30, y: 30 },
        { x: 170, y: 30 },
        { x: 100, y: 170 }, // apex at BOTTOM
      ],
      12,
    );
    expect(classifyGlyph(invtri)).toBe("invtriangle");
  });

  it("returns null for a tiny trace (under the bbox minimum)", () => {
    const tiny: Point[] = [];
    for (let i = 0; i < 20; i++) tiny.push({ x: 100 + i, y: 100 + i });
    expect(classifyGlyph(tiny)).toBeNull();
  });

  it("returns null when there are too few points", () => {
    expect(classifyGlyph([{ x: 0, y: 0 }, { x: 100, y: 100 }])).toBeNull();
  });

  it("returns null for a straight line (only one direction change)", () => {
    const line: Point[] = [];
    for (let i = 0; i <= 50; i++) line.push({ x: 50 + i * 3, y: 100 });
    // A line has no corners (smooth) — would classify as a 'circle'
    // *but* the bounding-box test rules it out because height < min.
    // Force the test more honestly: use a clearly non-shape stroke.
    expect(classifyGlyph(line)).toBeNull();
  });

  it("respects custom config overrides", () => {
    const tinyButValid: Point[] = arcPoints(50, 50, 20, 0, 2 * Math.PI, 48);
    // With the default minBoundingSide=40 this circle (40 px diameter) is
    // borderline — tighten the threshold and confirm it still classifies.
    expect(
      classifyGlyph(tinyButValid, {
        minBoundingSide: 20,
        cornerAngleThreshold: CLASSIFIER_DEFAULTS.cornerAngleThreshold,
      }),
    ).toBe("circle");
  });
});
