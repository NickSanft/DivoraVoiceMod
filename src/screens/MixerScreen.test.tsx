// v0.11.2: pointerdown on the Mixer's empty space starts the cast
// gesture directly, matching the mockup's "drag anywhere to cast"
// behavior. The Cast button + G hotkey still work; this is an
// additive third invocation path.
//
// The screen's pointerdown handler uses `isInteractiveAncestor` to
// filter out events that started on a button, slider, switch, card,
// or anything with `data-cast-block`. The unit test below drives
// that predicate directly so we don't need to mount the whole app
// store + audio bootstrap just to assert UI-control filtering.

import { describe, expect, it } from "vitest";
import { isInteractiveAncestor } from "./MixerScreen";

/** Build a small detached DOM tree rooted at `root` containing `inner`
 *  nested via the optional `wrap` chain (outermost first). */
function tree(root: HTMLElement, inner: HTMLElement, wrap: HTMLElement[] = []) {
  let parent: HTMLElement = root;
  for (const w of wrap) {
    parent.appendChild(w);
    parent = w;
  }
  parent.appendChild(inner);
  return { root, inner };
}

describe("isInteractiveAncestor", () => {
  it("returns false when target is the root itself (true empty space)", () => {
    const root = document.createElement("div");
    expect(isInteractiveAncestor(root, root)).toBe(false);
  });

  it("returns false for plain non-interactive descendants", () => {
    const root = document.createElement("div");
    const span = document.createElement("span");
    tree(root, span, [document.createElement("div")]);
    expect(isInteractiveAncestor(span, root)).toBe(false);
  });

  it("returns true when target is a <button>", () => {
    const root = document.createElement("div");
    const btn = document.createElement("button");
    tree(root, btn);
    expect(isInteractiveAncestor(btn, root)).toBe(true);
  });

  it("returns true when target nests inside a <button> (e.g. icon span)", () => {
    const root = document.createElement("div");
    const icon = document.createElement("span");
    tree(root, icon, [document.createElement("button")]);
    expect(isInteractiveAncestor(icon, root)).toBe(true);
  });

  for (const tag of ["INPUT", "SELECT", "TEXTAREA", "A"] as const) {
    it(`returns true when target is <${tag.toLowerCase()}>`, () => {
      const root = document.createElement("div");
      const el = document.createElement(tag.toLowerCase()) as HTMLElement;
      tree(root, el);
      expect(isInteractiveAncestor(el, root)).toBe(true);
    });
  }

  for (const role of ["button", "slider", "switch"] as const) {
    it(`returns true when target carries role="${role}"`, () => {
      const root = document.createElement("div");
      const el = document.createElement("div");
      el.setAttribute("role", role);
      tree(root, el);
      expect(isInteractiveAncestor(el, root)).toBe(true);
    });
  }

  it("returns true when target sits inside a .card surface", () => {
    const root = document.createElement("div");
    const child = document.createElement("span");
    const card = document.createElement("div");
    card.className = "card";
    tree(root, child, [card]);
    expect(isInteractiveAncestor(child, root)).toBe(true);
  });

  it("respects an explicit data-cast-block opt-out", () => {
    const root = document.createElement("div");
    const child = document.createElement("span");
    const wrap = document.createElement("div");
    wrap.setAttribute("data-cast-block", "");
    tree(root, child, [wrap]);
    expect(isInteractiveAncestor(child, root)).toBe(true);
  });

  it("treats contenteditable hosts as interactive", () => {
    const root = document.createElement("div");
    const editable = document.createElement("div");
    // Use setAttribute (jsdom's `el.contentEditable = "true"` does
    // not propagate to the attribute or the isContentEditable getter).
    // The implementation reads the attribute as a fallback for exactly
    // this reason; a real browser populates the property automatically.
    editable.setAttribute("contenteditable", "true");
    const child = document.createElement("span");
    tree(root, child, [editable]);
    expect(isInteractiveAncestor(child, root)).toBe(true);
  });

  it("ignores contenteditable=\"false\" (explicit non-edit zone)", () => {
    const root = document.createElement("div");
    const wrap = document.createElement("div");
    wrap.setAttribute("contenteditable", "false");
    const child = document.createElement("span");
    tree(root, child, [wrap]);
    expect(isInteractiveAncestor(child, root)).toBe(false);
  });

  it("stops walking at the cast root (does not check the root's own ancestry)", () => {
    // The MixerScreen container is itself wrapped in interactive-looking
    // chrome (titlebar, sidebar buttons). If we walked past `stopAt` we'd
    // false-positive every click. The walk MUST terminate at the root.
    const outerButton = document.createElement("button");
    const root = document.createElement("div");
    const child = document.createElement("span");
    outerButton.appendChild(root);
    root.appendChild(child);
    expect(isInteractiveAncestor(child, root)).toBe(false);
  });

  it("returns false when start is null (defensive)", () => {
    const root = document.createElement("div");
    expect(isInteractiveAncestor(null, root)).toBe(false);
  });
});
