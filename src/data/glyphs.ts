// Glyph classifier.
//
// Given a free-hand pointer trace (a list of {x, y} points captured
// during pointermove), decide which of the four glyphs the user drew:
//
//   triangle    (▲, apex up)
//   invtriangle (▽, apex down)
//   square      (□)
//   circle      (○)
//
// Returns `null` when the trace is too small, too short, or doesn't
// match any shape with reasonable confidence.
//
// Algorithm — a deliberately simple geometric classifier (no ML,
// no template-matching DLLs). The pipeline:
//
//   1. Trim trailing duplicates.
//   2. Bail if bounding box is too small (likely a click).
//   3. Resample the path uniformly along arc length to N=32 points so
//      the trace's natural speed variation doesn't bias the rest.
//   4. Smooth the resampled points (3-tap moving average).
//   5. Decide if the path is CLOSED (start ≈ end) — every glyph is
//      visually closed, so we expect this.
//   6. Compute the turning angle at each interior point. Sum the
//      absolute angles → total turning ≈ 2π for a closed shape.
//   7. Count "corners": points whose smoothed turning angle exceeds a
//      threshold (≈ 50° of direction change in a short window).
//   8. Classify:
//        - 0 corners → smooth = circle
//        - 4 corners → square
//        - 3 corners → triangle or invtriangle (decide by the apex's Y
//          position — apex is the corner with the most-extreme Y
//          coordinate; if it's at the top of the bbox, it's a triangle;
//          if at the bottom, it's an inverted triangle).
//        - anything else → null
//
// This is good enough for the four target shapes drawn by a human
// pointer; "tightening" it with template-matching is future polish.

import type { GlyphId } from "../types";

/** A single (x, y) sample from a pointer trace. */
export interface Point {
  x: number;
  y: number;
}

/** Tunables exposed for tests; tweak with care. */
export const CLASSIFIER_DEFAULTS = {
  /** Bounding-box minimum side (px) before we bother classifying. */
  minBoundingSide: 40,
  /** Number of resampled points along the path. */
  resampleCount: 48,
  /**
   * Moving-average half-window for smoothing the resampled path. 0 =
   * no smoothing. Natural pointer-sample timing already provides some
   * smoothing for real input; smoothing more dilutes the per-point
   * turning angles at corners, washing them out below threshold.
   */
  smoothHalfWindow: 0,
  /** Minimum points after deduping; below this we bail. */
  minPointCount: 8,
  /**
   * Corner detection: turning angles above this (radians) count as
   * a corner. ~35° is loose enough to catch corners that fall between
   * resampled points (where the angle gets split across two samples)
   * yet tight enough to ignore mild noise on a circle.
   */
  cornerAngleThreshold: (35 * Math.PI) / 180,
  /**
   * Closed-path tolerance: distance between start and end (as a
   * fraction of the bounding-box diagonal) below which the trace is
   * considered closed. We use this to decide whether a 3-corner trace
   * is a triangle (closed) or just an unfinished sketch (open).
   */
  closedTolerance: 0.35,
};

/** Remove consecutive duplicate points (zero-length segments). */
export function dedupe(points: Point[]): Point[] {
  const out: Point[] = [];
  for (const p of points) {
    const last = out[out.length - 1];
    if (!last || last.x !== p.x || last.y !== p.y) out.push(p);
  }
  return out;
}

export interface BoundingBox {
  xmin: number;
  ymin: number;
  xmax: number;
  ymax: number;
}

export function boundingBox(points: Point[]): BoundingBox {
  if (points.length === 0) return { xmin: 0, ymin: 0, xmax: 0, ymax: 0 };
  let xmin = points[0]!.x;
  let xmax = points[0]!.x;
  let ymin = points[0]!.y;
  let ymax = points[0]!.y;
  for (const p of points) {
    if (p.x < xmin) xmin = p.x;
    if (p.x > xmax) xmax = p.x;
    if (p.y < ymin) ymin = p.y;
    if (p.y > ymax) ymax = p.y;
  }
  return { xmin, ymin, xmax, ymax };
}

export function pathLength(points: Point[]): number {
  let total = 0;
  for (let i = 1; i < points.length; i++) {
    const a = points[i - 1]!;
    const b = points[i]!;
    total += Math.hypot(b.x - a.x, b.y - a.y);
  }
  return total;
}

/**
 * Resample `points` to exactly `n` evenly-spaced points along arc length.
 * Returns the original (deduped) list if it has < 2 entries.
 */
export function resamplePath(points: Point[], n: number): Point[] {
  if (points.length < 2 || n < 2) return points.slice();
  const total = pathLength(points);
  if (total === 0) return points.slice();
  const step = total / (n - 1);
  const out: Point[] = [points[0]!];
  let acc = 0;
  let i = 1;
  while (out.length < n && i < points.length) {
    const a = points[i - 1]!;
    const b = points[i]!;
    const segLen = Math.hypot(b.x - a.x, b.y - a.y);
    const need = step - acc;
    if (segLen >= need && segLen > 0) {
      const t = need / segLen;
      const p: Point = { x: a.x + (b.x - a.x) * t, y: a.y + (b.y - a.y) * t };
      out.push(p);
      // Splice the new point into the path so the next iteration starts
      // from there.
      points = [
        ...points.slice(0, i),
        p,
        ...points.slice(i),
      ];
      acc = 0;
      i += 1;
    } else {
      acc += segLen;
      i += 1;
    }
  }
  while (out.length < n) out.push(points[points.length - 1]!);
  return out;
}

/** 3-tap moving average (the half-window k = 1 case touches neighbours only). */
export function smooth(points: Point[], halfWindow: number): Point[] {
  if (halfWindow <= 0 || points.length < 3) return points.slice();
  const out: Point[] = [];
  for (let i = 0; i < points.length; i++) {
    let sx = 0;
    let sy = 0;
    let count = 0;
    for (
      let j = Math.max(0, i - halfWindow);
      j <= Math.min(points.length - 1, i + halfWindow);
      j++
    ) {
      sx += points[j]!.x;
      sy += points[j]!.y;
      count += 1;
    }
    out.push({ x: sx / count, y: sy / count });
  }
  return out;
}

/**
 * For each point, return the absolute turning angle (radians) between
 * the incoming and outgoing direction vectors.
 *
 * If `cyclic` is true, the path is treated as a closed cycle so the
 * angle at index 0 uses `points[N-1]` as the predecessor and the angle
 * at index N-1 uses `points[0]` as the successor. The seam corner of
 * a triangle / square / circle then shows up as a real peak instead of
 * a forced zero. Non-cyclic mode keeps the endpoints at 0 (legacy).
 */
export function turningAngles(points: Point[], cyclic = false): number[] {
  const n = points.length;
  const angles = new Array<number>(n).fill(0);
  for (let i = 0; i < n; i++) {
    let prevIdx: number;
    let nextIdx: number;
    if (cyclic) {
      prevIdx = (i - 1 + n) % n;
      nextIdx = (i + 1) % n;
    } else {
      if (i === 0 || i === n - 1) continue;
      prevIdx = i - 1;
      nextIdx = i + 1;
    }
    const a = points[prevIdx]!;
    const b = points[i]!;
    const c = points[nextIdx]!;
    const v1x = b.x - a.x;
    const v1y = b.y - a.y;
    const v2x = c.x - b.x;
    const v2y = c.y - b.y;
    const m1 = Math.hypot(v1x, v1y);
    const m2 = Math.hypot(v2x, v2y);
    if (m1 === 0 || m2 === 0) continue;
    const cos = Math.max(-1, Math.min(1, (v1x * v2x + v1y * v2y) / (m1 * m2)));
    angles[i] = Math.acos(cos);
  }
  return angles;
}

/**
 * Find "corners" — local maxima in the smoothed turning-angle series
 * that exceed `threshold` and aren't too close to each other.
 *
 * Cyclic mode wraps neighbour lookups at the seam so a corner that
 * straddles index 0 / N-1 still registers.
 *
 * Returns indices into the resampled point array.
 */
export function findCorners(
  angles: number[],
  threshold: number,
  minSeparation: number,
  cyclic = false,
): number[] {
  const n = angles.length;
  const corners: number[] = [];
  const lo = cyclic ? 0 : 1;
  const hi = cyclic ? n : n - 1;
  for (let i = lo; i < hi; i++) {
    const a = angles[i]!;
    const prev = angles[cyclic ? (i - 1 + n) % n : i - 1]!;
    const next = angles[cyclic ? (i + 1) % n : i + 1]!;
    if (a < threshold) continue;
    if (a < prev || a < next) continue;
    const last = corners[corners.length - 1];
    if (last !== undefined && i - last < minSeparation) {
      if (a > angles[last]!) corners[corners.length - 1] = i;
      continue;
    }
    corners.push(i);
  }
  // Cyclic: dedupe corners that wrap (last corner and first corner
  // within minSeparation distance around the seam).
  if (cyclic && corners.length >= 2) {
    const first = corners[0]!;
    const last = corners[corners.length - 1]!;
    const wrapDist = first + (n - last);
    if (wrapDist < minSeparation) {
      // Keep whichever is sharper.
      if (angles[first]! >= angles[last]!) corners.pop();
      else corners.shift();
    }
  }
  return corners;
}

/** Distance from start to end as a fraction of the bbox diagonal. */
export function endpointGap(points: Point[], bbox: BoundingBox): number {
  const first = points[0];
  const last = points[points.length - 1];
  if (!first || !last) return Infinity;
  const diag = Math.hypot(bbox.xmax - bbox.xmin, bbox.ymax - bbox.ymin);
  if (diag === 0) return Infinity;
  return Math.hypot(last.x - first.x, last.y - first.y) / diag;
}

/**
 * Classify a trace into a `GlyphId`, or `null` if it's too small / open /
 * unrecognisable.
 */
export function classifyGlyph(
  raw: Point[],
  config: Partial<typeof CLASSIFIER_DEFAULTS> = {},
): GlyphId | null {
  const cfg = { ...CLASSIFIER_DEFAULTS, ...config };
  const points = dedupe(raw);
  if (points.length < cfg.minPointCount) return null;

  const bbox = boundingBox(points);
  const w = bbox.xmax - bbox.xmin;
  const h = bbox.ymax - bbox.ymin;
  if (w < cfg.minBoundingSide || h < cfg.minBoundingSide) return null;

  let resampled = resamplePath(points, cfg.resampleCount);
  const smoothed = smooth(resampled, cfg.smoothHalfWindow);

  // If start ≈ end, the trace is closed. Drop the duplicate seam point
  // and process the rest cyclically so the seam corner registers like
  // any other corner.
  const cyclic = endpointGap(smoothed, bbox) <= cfg.closedTolerance;
  let workPoints = smoothed;
  if (cyclic && workPoints.length > 2) {
    const first = workPoints[0]!;
    const last = workPoints[workPoints.length - 1]!;
    const dx = Math.abs(last.x - first.x);
    const dy = Math.abs(last.y - first.y);
    // Tighter check: if the gap is tiny (<1 px in both axes), the last
    // sample is a literal duplicate of the first — drop it.
    if (dx < 1 && dy < 1) {
      workPoints = workPoints.slice(0, -1);
    }
  }

  const angles = turningAngles(workPoints, cyclic);
  const corners = findCorners(
    angles,
    cfg.cornerAngleThreshold,
    Math.max(2, Math.floor(cfg.resampleCount / 8)),
    cyclic,
  );

  if (!cyclic) {
    // Open path — we can't reliably classify a glyph since the user
    // didn't close the shape. Be strict.
    return null;
  }

  if (corners.length === 0) return "circle";
  if (corners.length === 4) return "square";
  if (corners.length === 3) {
    // Apex = the corner whose Y is the outlier (furthest from the
    // median Y of the three corner points). For ▲ the apex Y is the
    // smallest (above the two base corners); for ▽ the apex Y is the
    // largest (below the two top corners). Sorting by Y and picking
    // the median gives a robust reference even when sample noise has
    // shifted things around.
    const ys = corners.map((i) => workPoints[i]!.y);
    const sortedYs = [...ys].sort((a, b) => a - b);
    const medianY = sortedYs[1]!;
    let apexIdx = corners[0]!;
    let maxDistance = -Infinity;
    for (const idx of corners) {
      const distance = Math.abs(workPoints[idx]!.y - medianY);
      if (distance > maxDistance) {
        maxDistance = distance;
        apexIdx = idx;
      }
    }
    const apex = workPoints[apexIdx]!;
    return apex.y < medianY ? "triangle" : "invtriangle";
  }

  // 1, 2, or 5+ corners — not a glyph we recognise.
  return null;
}

/**
 * v0.11.3: rich shape detection used by the canvas-based SparkLayer.
 *
 * Returns NOT just the glyph id but also the centroid, characteristic
 * size (bounding-box diagonal), and either the polygon corners (for
 * triangle/inv-triangle/square) or the mean radius (for circle). That
 * geometry is what the omen-render code uses to outline the shape in
 * sparks at the actual stroke's center + scale.
 *
 * Ported 1:1 from `docs/mockups/prototype/divora/spark_canvas.jsx`
 * (`detectShape`). The prototype's algorithm uses Ramer–Douglas–Peucker
 * line simplification + radial uniformity + corner-count voting. It is
 * a deliberately different algorithm from `classifyGlyph` (which is a
 * turning-angle classifier with a different threshold scheme). They
 * coexist — `classifyGlyph` keeps its existing unit tests since other
 * code paths (and the `localPoint` helper) still use it; `detectShape`
 * is purpose-built for the cast pipeline.
 */
export interface DetectedShape {
  type: GlyphId;
  /** Centroid X (mean of trace samples). */
  cx: number;
  /** Centroid Y. */
  cy: number;
  /** Bounding-box diagonal (px). */
  size: number;
  /** Polygon corners after RDP simplification; absent for circles. */
  corners?: Point[];
  /** Mean radius from the centroid; only set for circles. */
  r?: number;
}

/** Perpendicular distance from `p` to the line through `a` and `b`. */
function perpDist(p: Point, a: Point, b: Point): number {
  const dx = b.x - a.x;
  const dy = b.y - a.y;
  const len = Math.hypot(dx, dy) || 1;
  return Math.abs((p.x - a.x) * dy - (p.y - a.y) * dx) / len;
}

/**
 * Ramer–Douglas–Peucker line simplification. Returns a subset of `pts`
 * that approximates the same path with perpendicular error ≤ `eps`.
 */
export function rdp(pts: Point[], eps: number): Point[] {
  if (pts.length < 3) return pts.slice();
  let dmax = 0;
  let idx = 0;
  const a = pts[0]!;
  const b = pts[pts.length - 1]!;
  for (let i = 1; i < pts.length - 1; i++) {
    const d = perpDist(pts[i]!, a, b);
    if (d > dmax) {
      dmax = d;
      idx = i;
    }
  }
  if (dmax > eps) {
    const l = rdp(pts.slice(0, idx + 1), eps);
    const r = rdp(pts.slice(idx), eps);
    return l.slice(0, -1).concat(r);
  }
  return [a, b];
}

/** Absolute polygon area via the shoelace formula. */
function polyArea(p: Point[]): number {
  let s = 0;
  for (let i = 0; i < p.length; i++) {
    const q = p[(i + 1) % p.length]!;
    s += p[i]!.x * q.y - q.x * p[i]!.y;
  }
  return Math.abs(s / 2);
}

/**
 * Detect one of the four target shapes (triangle, invtriangle, square,
 * circle) in a free-hand pointer trace and return its geometry, or
 * `null` if the trace is too short, too small, or doesn't match.
 *
 * The prototype's tuning constants are preserved verbatim so the cast
 * gesture feels identical to the mockup:
 *
 *   - bounding-box diagonal ≥ 110 px (anything smaller is "not a draw")
 *   - endpoint gap ≤ 30 % of diag (must be closed)
 *   - RDP epsilon = 9 % of diag
 *   - circle test: 5+ corners after RDP, radial stddev / mean < 0.17,
 *     and aspect ratio > 0.7
 *   - triangle: 3 RDP corners with polygon area ≥ 16 % of bbox area
 *   - square: 4 RDP corners with polygon area ≥ 40 % of bbox area
 *   - fallback circle (looser): 5+ corners, circ < 0.24, aspect > 0.6
 */
export function detectShape(path: Point[]): DetectedShape | null {
  if (path.length < 12) return null;
  // Defensive copy; we also strip any trailing duplicate close-loop
  // samples (some pointer devices repeat the seam point).
  let pts = path.slice();
  while (
    pts.length > 3 &&
    Math.hypot(
      pts[0]!.x - pts[pts.length - 1]!.x,
      pts[0]!.y - pts[pts.length - 1]!.y,
    ) < 3
  ) {
    pts.pop();
  }
  const xs = pts.map((p) => p.x);
  const ys = pts.map((p) => p.y);
  const minX = Math.min(...xs);
  const maxX = Math.max(...xs);
  const minY = Math.min(...ys);
  const maxY = Math.max(...ys);
  const w = maxX - minX;
  const h = maxY - minY;
  const diag = Math.hypot(w, h);
  if (diag < 110) return null;
  const s = pts[0]!;
  const e = pts[pts.length - 1]!;
  if (Math.hypot(e.x - s.x, e.y - s.y) > diag * 0.3) return null;
  const cx = xs.reduce((a, b) => a + b, 0) / xs.length;
  const cy = ys.reduce((a, b) => a + b, 0) / ys.length;
  // Radial uniformity — low stdR/meanR → circular.
  const radii = pts.map((p) => Math.hypot(p.x - cx, p.y - cy));
  const meanR = radii.reduce((a, b) => a + b, 0) / radii.length;
  const stdR = Math.sqrt(
    radii.reduce((a, b) => a + (b - meanR) ** 2, 0) / radii.length,
  );
  const circ = meanR ? stdR / meanR : 1;
  const aspect = Math.min(w, h) / Math.max(w, h);
  // RDP corner count.
  let simp = rdp(pts, diag * 0.09);
  while (
    simp.length > 3 &&
    Math.hypot(
      simp[0]!.x - simp[simp.length - 1]!.x,
      simp[0]!.y - simp[simp.length - 1]!.y,
    ) <
      diag * 0.2
  ) {
    simp.pop();
  }
  const n = simp.length;

  if (n >= 5 && circ < 0.17 && aspect > 0.7) {
    return { type: "circle", cx, cy, size: diag, r: meanR };
  }
  if (n === 3) {
    if (polyArea(simp) < w * h * 0.16) return null;
    const tcy = (simp[0]!.y + simp[1]!.y + simp[2]!.y) / 3;
    const above = simp.filter((p) => p.y < tcy).length;
    return {
      type: above === 1 ? "triangle" : "invtriangle",
      cx,
      cy,
      size: diag,
      corners: simp,
    };
  }
  if (n === 4) {
    if (polyArea(simp) < w * h * 0.4) return null;
    return { type: "square", cx, cy, size: diag, corners: simp };
  }
  if (n >= 5 && circ < 0.24 && aspect > 0.6) {
    return { type: "circle", cx, cy, size: diag, r: meanR };
  }
  return null;
}
