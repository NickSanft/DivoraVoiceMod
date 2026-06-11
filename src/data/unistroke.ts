// v1.15.0: a $1-style unistroke recognizer for CUSTOM glyphs.
//
// The built-in 4 shapes keep their tuned geometric detector
// (`detectShape` in glyphs.ts). Custom user-recorded glyphs need
// arbitrary single-stroke matching, which is what this does: resample a
// drawn path to a fixed point count, normalize position + scale, and
// compare to stored templates by average point distance.
//
// Deliberately ORIENTATION-PRESERVING — we do NOT rotate to an indicative
// angle (the classic $1 step), because for glyphs a triangle (▲) and an
// inverted triangle (▽) are different gestures. Resample + uniform-scale +
// recenter only. Matching is start-point sensitive, which is fine for a
// personal glyph you draw the same way you recorded it.

import type { Point } from "./glyphs";

/** Points per normalized template. */
export const RESAMPLE_N = 64;

/** Half-diagonal of the unit reference box — distance → score scale. */
const HALF_DIAG = Math.SQRT1_2; // 0.7071…

function pathLength(pts: Point[]): number {
  let len = 0;
  for (let i = 1; i < pts.length; i++) {
    len += Math.hypot(pts[i]!.x - pts[i - 1]!.x, pts[i]!.y - pts[i - 1]!.y);
  }
  return len;
}

/** Resample `pts` to `n` points spaced equally along arc length. */
export function resample(pts: Point[], n = RESAMPLE_N): Point[] {
  if (pts.length === 0) return [];
  if (pts.length === 1) return Array.from({ length: n }, () => ({ ...pts[0]! }));
  const interval = pathLength(pts) / (n - 1);
  if (!(interval > 0)) return Array.from({ length: n }, () => ({ ...pts[0]! }));

  const out: Point[] = [{ ...pts[0]! }];
  let acc = 0;
  const work = pts.map((p) => ({ ...p }));
  for (let i = 1; i < work.length; i++) {
    const prev = work[i - 1]!;
    const cur = work[i]!;
    let d = Math.hypot(cur.x - prev.x, cur.y - prev.y);
    if (acc + d >= interval && d > 0) {
      const t = (interval - acc) / d;
      const np = { x: prev.x + t * (cur.x - prev.x), y: prev.y + t * (cur.y - prev.y) };
      out.push(np);
      work.splice(i, 0, np); // reconsider the segment from the new point
      acc = 0;
    } else {
      acc += d;
    }
  }
  // Floating-point slack can leave us one short — pad with the last point.
  while (out.length < n) out.push({ ...pts[pts.length - 1]! });
  return out.slice(0, n);
}

function centroid(pts: Point[]): Point {
  let sx = 0;
  let sy = 0;
  for (const p of pts) {
    sx += p.x;
    sy += p.y;
  }
  return { x: sx / pts.length, y: sy / pts.length };
}

/** Recenter to the centroid and uniformly scale by the larger bbox side, so
 *  the stroke lands in roughly [-0.5, 0.5] while preserving aspect (a tall
 *  stroke stays tall) and orientation. */
export function normalize(pts: Point[]): Point[] {
  if (pts.length === 0) return [];
  const c = centroid(pts);
  let minX = Infinity;
  let minY = Infinity;
  let maxX = -Infinity;
  let maxY = -Infinity;
  for (const p of pts) {
    if (p.x < minX) minX = p.x;
    if (p.y < minY) minY = p.y;
    if (p.x > maxX) maxX = p.x;
    if (p.y > maxY) maxY = p.y;
  }
  const scale = Math.max(maxX - minX, maxY - minY) || 1;
  return pts.map((p) => ({ x: (p.x - c.x) / scale, y: (p.y - c.y) / scale }));
}

/** Resample + normalize a raw drawn stroke into a comparable template. */
export function makeTemplate(raw: Point[], n = RESAMPLE_N): Point[] {
  return normalize(resample(raw, n));
}

/** Average point-to-point distance between two equal-length point lists. */
function pathDistance(a: Point[], b: Point[]): number {
  const len = Math.min(a.length, b.length);
  if (len === 0) return Infinity;
  let sum = 0;
  for (let i = 0; i < len; i++) {
    sum += Math.hypot(a[i]!.x - b[i]!.x, a[i]!.y - b[i]!.y);
  }
  return sum / len;
}

export interface UnistrokeTemplate {
  id: string;
  /** Normalized points (output of `makeTemplate`). */
  points: Point[];
}

export interface UnistrokeMatch {
  id: string;
  /** 0..1, higher = closer. */
  score: number;
}

/** Default acceptance threshold (tuned against the tests). */
export const DEFAULT_MIN_SCORE = 0.8;

/**
 * Recognize a raw drawn `path` against `templates`. Returns the best match
 * with a score in [0, 1], or null when nothing clears `minScore` (so an
 * unrecognized scribble casts nothing).
 */
export function recognize(
  path: Point[],
  templates: UnistrokeTemplate[],
  minScore = DEFAULT_MIN_SCORE,
): UnistrokeMatch | null {
  if (path.length < 4 || templates.length === 0) return null;
  const cand = makeTemplate(path);
  let best: UnistrokeMatch | null = null;
  for (const t of templates) {
    if (t.points.length !== cand.length) continue;
    const d = pathDistance(cand, t.points);
    const score = Math.max(0, 1 - d / HALF_DIAG);
    if (!best || score > best.score) best = { id: t.id, score };
  }
  return best && best.score >= minScore ? best : null;
}
