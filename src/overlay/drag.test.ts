import { describe, expect, it } from "vitest";
import { isPlainLeftPress, startsWindowDrag } from "./drag";

function tree() {
  const root = document.createElement("div");
  root.setAttribute("data-stage", "");
  const button = document.createElement("button");
  const inside = document.createElement("span");
  button.appendChild(inside);
  const roleButton = document.createElement("div");
  roleButton.setAttribute("role", "button");
  // v1.55.0: the circle is a charm; its drawing sits inside the row.
  const charm = document.createElement("div");
  charm.setAttribute("data-charm", "");
  const svg = document.createElementNS("http://www.w3.org/2000/svg", "svg");
  const line = document.createElementNS("http://www.w3.org/2000/svg", "line");
  svg.appendChild(line);
  const hit = document.createElement("div");
  hit.setAttribute("data-hit", "");
  const handle = document.createElement("div");
  handle.setAttribute("data-handle", "");
  charm.append(svg, hit, handle);
  const chrome = document.createElement("div");
  chrome.setAttribute("data-chrome", "");
  const grip = document.createElement("div");
  grip.setAttribute("data-grip", "");
  chrome.appendChild(grip);
  root.append(button, roleButton, charm, chrome);
  return { root, line, button, inside, roleButton, charm, hit, handle, chrome, grip };
}

describe("isPlainLeftPress (v1.55.0)", () => {
  it.each([
    [0, 1, true],
    [0, 2, false],
    [0, 3, false],
    [0, 0, false],
    [1, 1, false],
    [2, 1, false],
  ])("button %i, detail %i → %s", (button, detail, want) => {
    expect(isPlainLeftPress({ button, detail })).toBe(want);
  });
});

describe("startsWindowDrag", () => {
  it("a plain left press on the empty stage drags", () => {
    const t = tree();
    expect(startsWindowDrag({ button: 0, detail: 1, target: t.root })).toBe(true);
  });

  it("v1.55.0: charms, their handles and the Arrange chrome never drag the window", () => {
    // v1.51.3 let the circle's drawing drag; now pressing a charm moves the
    // charm, and the window moves from the grip or an empty spot.
    const t = tree();
    for (const target of [t.charm, t.line, t.hit, t.handle, t.chrome, t.grip]) {
      expect(startsWindowDrag({ button: 0, detail: 1, target })).toBe(false);
    }
  });

  it("buttons keep their click, including whatever is inside them", () => {
    const t = tree();
    expect(startsWindowDrag({ button: 0, detail: 1, target: t.button })).toBe(false);
    expect(startsWindowDrag({ button: 0, detail: 1, target: t.inside })).toBe(false);
    expect(startsWindowDrag({ button: 0, detail: 1, target: t.roleButton })).toBe(false);
  });

  it("the second press of a double-click never drags (and never maximises)", () => {
    const t = tree();
    expect(startsWindowDrag({ button: 0, detail: 2, target: t.root })).toBe(false);
  });

  it("only the left button", () => {
    const t = tree();
    expect(startsWindowDrag({ button: 1, detail: 1, target: t.root })).toBe(false);
    expect(startsWindowDrag({ button: 2, detail: 1, target: t.root })).toBe(false);
  });

  it("no element target, no drag", () => {
    expect(startsWindowDrag({ button: 0, detail: 1, target: null })).toBe(false);
    expect(startsWindowDrag({ button: 0, detail: 1, target: document })).toBe(false);
  });
});
