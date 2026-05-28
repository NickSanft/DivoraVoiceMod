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
});
