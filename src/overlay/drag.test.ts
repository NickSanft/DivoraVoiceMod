import { describe, expect, it } from "vitest";
import { startsWindowDrag } from "./drag";

function tree() {
  const root = document.createElement("div");
  const svg = document.createElementNS("http://www.w3.org/2000/svg", "svg");
  const line = document.createElementNS("http://www.w3.org/2000/svg", "line");
  svg.appendChild(line);
  const button = document.createElement("button");
  const inside = document.createElement("span");
  button.appendChild(inside);
  const roleButton = document.createElement("div");
  roleButton.setAttribute("role", "button");
  root.append(svg, button, roleButton);
  return { root, line, button, inside, roleButton };
}

describe("startsWindowDrag", () => {
  it("a plain left press on the background or the circle's drawing drags", () => {
    const t = tree();
    expect(startsWindowDrag({ button: 0, detail: 1, target: t.root })).toBe(true);
    // SVG elements are not HTMLElements; Tauri's own script skips them.
    expect(startsWindowDrag({ button: 0, detail: 1, target: t.line })).toBe(true);
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
