import { describe, expect, it } from "vitest";
import { localPoint } from "./GlyphCastOverlay";

/**
 * `localPoint(svg, e)` must subtract the SVG's `getBoundingClientRect()`
 * top-left from the event's viewport coordinates. In Phase 7 we shipped
 * with raw `clientX/clientY` and a window-sized viewBox, which left the
 * trace offset by exactly the titlebar height (~36 px) and sidebar
 * width (~80 px). This is the regression catch.
 */

function fakeSvg(left: number, top: number): SVGSVGElement {
  return {
    getBoundingClientRect: () =>
      ({ left, top, right: 0, bottom: 0, width: 0, height: 0, x: left, y: top }) as DOMRect,
  } as unknown as SVGSVGElement;
}

describe("GlyphCastOverlay.localPoint", () => {
  it("subtracts the SVG's bounding-rect top-left from the event coords", () => {
    const svg = fakeSvg(80, 36);
    const pt = localPoint(svg, { clientX: 200, clientY: 150 });
    expect(pt).toEqual({ x: 120, y: 114 });
  });

  it("is the identity when the SVG sits at the viewport origin", () => {
    const svg = fakeSvg(0, 0);
    const pt = localPoint(svg, { clientX: 200, clientY: 150 });
    expect(pt).toEqual({ x: 200, y: 150 });
  });

  it("falls back to client coords when no SVG ref is available", () => {
    const pt = localPoint(null, { clientX: 200, clientY: 150 });
    expect(pt).toEqual({ x: 200, y: 150 });
  });

  it("handles fractional offsets without rounding", () => {
    const svg = fakeSvg(80.5, 36.25);
    const pt = localPoint(svg, { clientX: 200, clientY: 150 });
    expect(pt.x).toBeCloseTo(119.5, 5);
    expect(pt.y).toBeCloseTo(113.75, 5);
  });
});
