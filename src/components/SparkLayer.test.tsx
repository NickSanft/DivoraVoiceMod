// v0.11.3 — SparkLayer's `isCastBlocker` predicate. The canvas-based
// cast surface listens at the window level with capture-phase, so it
// sees pointer events on every UI control before the control does.
// `isCastBlocker` decides whether to ignore a pointerdown (control)
// or capture it (empty space → start a cast).
//
// The predicate uses `Element.closest()` with a single CSS selector,
// which makes it easy to drive directly from jsdom without rendering
// the full Mixer.

import { describe, expect, it } from "vitest";
import { isCastBlocker } from "./SparkLayer";

/** Build a small detached DOM tree where `inner` is nested via `wrap`. */
function nest(inner: Element, wrap: Element[] = []): void {
  let parent: Element | null = null;
  for (const w of wrap) {
    if (parent) parent.appendChild(w);
    parent = w;
  }
  if (parent) parent.appendChild(inner);
}

describe("SparkLayer.isCastBlocker", () => {
  it("returns false for a plain non-interactive div (true empty space)", () => {
    const div = document.createElement("div");
    expect(isCastBlocker(div)).toBe(false);
  });

  it("returns false when given null (defensive)", () => {
    expect(isCastBlocker(null)).toBe(false);
  });

  it("returns false when target isn't an Element (e.g. document)", () => {
    expect(isCastBlocker(document as unknown as EventTarget)).toBe(false);
  });

  for (const tag of [
    "button",
    "a",
    "input",
    "select",
    "textarea",
  ] as const) {
    it(`returns true when target is <${tag}>`, () => {
      const el = document.createElement(tag);
      expect(isCastBlocker(el)).toBe(true);
    });

    it(`returns true when target is nested inside <${tag}>`, () => {
      const wrap = document.createElement(tag);
      const inner = document.createElement("span");
      nest(inner, [wrap]);
      expect(isCastBlocker(inner)).toBe(true);
    });
  }

  for (const role of [
    "button",
    "slider",
    "switch",
    "tab",
    "radio",
    "radiogroup",
  ] as const) {
    it(`returns true when target carries role="${role}"`, () => {
      const el = document.createElement("div");
      el.setAttribute("role", role);
      expect(isCastBlocker(el)).toBe(true);
    });
  }

  for (const cls of ["card", "seg", "kbd"] as const) {
    it(`returns true when target sits inside a .${cls}`, () => {
      const wrap = document.createElement("div");
      wrap.className = cls;
      const inner = document.createElement("span");
      nest(inner, [wrap]);
      expect(isCastBlocker(inner)).toBe(true);
    });
  }

  it("respects an explicit data-cast-block opt-out", () => {
    const wrap = document.createElement("div");
    wrap.setAttribute("data-cast-block", "");
    const inner = document.createElement("span");
    nest(inner, [wrap]);
    expect(isCastBlocker(inner)).toBe(true);
  });

  it("matches via deep nesting (icon inside button inside header)", () => {
    const header = document.createElement("header");
    const button = document.createElement("button");
    const icon = document.createElement("span");
    nest(icon, [header, button]);
    expect(isCastBlocker(icon)).toBe(true);
  });

  // v0.11.5: the custom titlebar surfaces are tagged
  // `data-tauri-drag-region` so the OS treats them as window chrome
  // for dragging. SparkLayer must NOT intercept pointerdown there,
  // otherwise the window stops being draggable on the Mixer (where
  // SparkLayer is mounted). Regression catch.
  it("returns true on a [data-tauri-drag-region] surface (titlebar)", () => {
    const dragRegion = document.createElement("div");
    dragRegion.setAttribute("data-tauri-drag-region", "");
    expect(isCastBlocker(dragRegion)).toBe(true);
  });

  it("returns true for a child of [data-tauri-drag-region] (wordmark span)", () => {
    const dragRegion = document.createElement("div");
    dragRegion.setAttribute("data-tauri-drag-region", "");
    const wordmark = document.createElement("span");
    nest(wordmark, [dragRegion]);
    expect(isCastBlocker(wordmark)).toBe(true);
  });
});
