import { describe, expect, it } from "vitest";
import { render } from "@solidjs/testing-library";
import { Sigil, SIGIL_NAMES } from "./Sigil";

describe("Sigil", () => {
  it("renders a SVG of the requested size", () => {
    const { container } = render(() => <Sigil name="mixer" size={32} />);
    const svg = container.querySelector("svg");
    expect(svg).toBeInTheDocument();
    expect(svg!.getAttribute("width")).toBe("32");
    expect(svg!.getAttribute("height")).toBe("32");
  });

  it("defaults to size 20", () => {
    const { container } = render(() => <Sigil name="clean" />);
    expect(container.querySelector("svg")!.getAttribute("width")).toBe("20");
  });

  it("includes every effect name in the catalog", () => {
    for (const name of ["pitch", "formant", "reverb", "eq", "robot", "distortion", "echo", "gate"] as const) {
      expect(SIGIL_NAMES).toContain(name);
    }
  });

  it("includes every nav name in the catalog", () => {
    for (const name of ["mixer", "soundboard", "presets", "settings"] as const) {
      expect(SIGIL_NAMES).toContain(name);
    }
  });

  it("renders independent DOM trees when the same name is used twice (v0.7.1 regression)", () => {
    // In v0.7.0 the SIGILS map stored pre-evaluated JSX elements, so
    // two <Sigil> components with the same name SHARED a single DOM
    // subtree — Solid would move the node to the latest mount point
    // and the earlier site would blank out. The fix converts each
    // SIGILS entry to a factory (`() => JSX.Element`) so every use
    // creates its own subtree.
    const { container } = render(() => (
      <div>
        <Sigil name="presets" size={20} />
        <Sigil name="presets" size={20} />
      </div>
    ));
    const svgs = container.querySelectorAll("svg");
    expect(svgs).toHaveLength(2);
    // Both SVGs must contain their own children (the icon's <g> or
    // <path>). A shared node would leave one of them empty.
    for (const svg of svgs) {
      expect(svg.children.length).toBeGreaterThan(0);
    }
  });

  it("renders independent DOM trees for the mixer sigil too (covers the Wizard pillar + Sidebar collision)", () => {
    const { container } = render(() => (
      <div>
        <Sigil name="mixer" size={20} />
        <Sigil name="mixer" size={20} />
      </div>
    ));
    const svgs = container.querySelectorAll("svg");
    expect(svgs).toHaveLength(2);
    for (const svg of svgs) {
      expect(svg.children.length).toBeGreaterThan(0);
    }
  });

  it("renders different glyphs side-by-side without bleed", () => {
    const { container } = render(() => (
      <div>
        <Sigil name="mixer" size={20} />
        <Sigil name="settings" size={20} />
        <Sigil name="presets" size={20} />
        <Sigil name="soundboard" size={20} />
      </div>
    ));
    expect(container.querySelectorAll("svg")).toHaveLength(4);
    for (const svg of container.querySelectorAll("svg")) {
      expect(svg.children.length).toBeGreaterThan(0);
    }
  });
});
