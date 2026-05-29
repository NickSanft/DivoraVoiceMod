import { render } from "@solidjs/testing-library";
import { describe, expect, it, vi } from "vitest";
import { GlyphCastOverlay, localPoint } from "./GlyphCastOverlay";

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

/**
 * v0.11.2 — seedPointer prop. The Mixer screen now forwards the
 * originating pointerdown event when the user starts dragging on
 * empty space, so the overlay can take pointer capture and start
 * drawing immediately. We assert:
 *
 *   1. The overlay calls setPointerCapture(pointerId) on its root
 *      with the seed's pointerId.
 *   2. The instruction blurb ("Cast a glyph") is hidden because the
 *      overlay enters drawing mode immediately.
 *   3. Without a seed, the overlay still mounts in idle state with
 *      the instructions visible (existing behavior preserved).
 */
describe("GlyphCastOverlay seedPointer (v0.11.2)", () => {
  /**
   * jsdom doesn't implement `Element.setPointerCapture` by default —
   * the spec is part of the Pointer Events API and v0.4-era jsdom
   * stubs it. Provide one before mount so the overlay's onMount can
   * call it and we can observe the call.
   */
  function stubPointerCapture(): ReturnType<typeof vi.fn> {
    const spy = vi.fn();
    Element.prototype.setPointerCapture = spy as typeof Element.prototype.setPointerCapture;
    // releasePointerCapture is also missing — stub it as a no-op so the
    // pointerup handler doesn't throw if it ever fires in jsdom.
    if (typeof Element.prototype.releasePointerCapture !== "function") {
      Element.prototype.releasePointerCapture = (() => {
        /* noop */
      }) as typeof Element.prototype.releasePointerCapture;
    }
    return spy;
  }

  it("captures the seed pointer id on mount", async () => {
    const spy = stubPointerCapture();
    render(() => (
      <GlyphCastOverlay
        onClassified={vi.fn()}
        onCancel={vi.fn()}
        seedPointer={{ pointerId: 42, clientX: 200, clientY: 150 }}
      />
    ));
    // onMount runs synchronously in Solid; the capture call should have
    // already happened by the time render() returns.
    expect(spy).toHaveBeenCalledWith(42);
  });

  it("enters drawing mode immediately when seeded (hides the idle blurb)", () => {
    stubPointerCapture();
    const { queryByText } = render(() => (
      <GlyphCastOverlay
        onClassified={vi.fn()}
        onCancel={vi.fn()}
        seedPointer={{ pointerId: 1, clientX: 80, clientY: 60 }}
      />
    ));
    // The "Cast a glyph" heading is only shown when !drawing && no
    // points — so a seeded overlay must hide it.
    expect(queryByText(/Cast a glyph/i)).toBeNull();
  });

  it("does NOT call setPointerCapture when no seed is provided", () => {
    const spy = stubPointerCapture();
    render(() => (
      <GlyphCastOverlay onClassified={vi.fn()} onCancel={vi.fn()} />
    ));
    expect(spy).not.toHaveBeenCalled();
  });

  it("still shows the idle blurb without a seed (existing behavior)", () => {
    stubPointerCapture();
    const { getByText } = render(() => (
      <GlyphCastOverlay onClassified={vi.fn()} onCancel={vi.fn()} />
    ));
    expect(getByText(/Cast a glyph/i)).toBeTruthy();
  });

  it("survives a setPointerCapture failure (pointer already released)", () => {
    Element.prototype.setPointerCapture = (() => {
      throw new Error("InvalidStateError");
    }) as typeof Element.prototype.setPointerCapture;
    if (typeof Element.prototype.releasePointerCapture !== "function") {
      Element.prototype.releasePointerCapture = (() => {
        /* noop */
      }) as typeof Element.prototype.releasePointerCapture;
    }
    // Should not throw — the overlay catches and logs a warning instead
    // so the user can still interact normally.
    expect(() =>
      render(() => (
        <GlyphCastOverlay
          onClassified={vi.fn()}
          onCancel={vi.fn()}
          seedPointer={{ pointerId: 7, clientX: 10, clientY: 10 }}
        />
      )),
    ).not.toThrow();
  });
});
