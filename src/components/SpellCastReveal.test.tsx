import { describe, expect, it, vi } from "vitest";
import { render } from "@solidjs/testing-library";

import { SpellCastReveal, REVEAL_DURATION_MS } from "./SpellCastReveal";

describe("SpellCastReveal", () => {
  it("renders the preset name + tag + SPELL CAST eyebrow", () => {
    const { getByText, queryByText } = render(() => (
      <SpellCastReveal
        glyph="reverb"
        name="Hollow King"
        color="#7C5CF6"
        tag="Bundled"
        onDone={() => {}}
      />
    ));
    expect(getByText("Hollow King")).toBeInTheDocument();
    expect(getByText("Bundled")).toBeInTheDocument();
    // Decorative ◆ characters in the eyebrow.
    expect(queryByText(/SPELL CAST/)).not.toBeNull();
  });

  it("applies the preset's brand colour to the name", () => {
    const { getByText } = render(() => (
      <SpellCastReveal
        glyph="distortion"
        name="Static Wraith"
        color="#58C6F2"
        tag="Bundled"
        onDone={() => {}}
      />
    ));
    const heading = getByText("Static Wraith");
    expect((heading as HTMLElement).style.color).toBe("rgb(88, 198, 242)");
  });

  it("fires onDone after REVEAL_DURATION_MS", () => {
    vi.useFakeTimers();
    const onDone = vi.fn();
    render(() => (
      <SpellCastReveal
        glyph="reverb"
        name="Hollow King"
        color="#7C5CF6"
        tag="Bundled"
        onDone={onDone}
      />
    ));
    expect(onDone).not.toHaveBeenCalled();
    vi.advanceTimersByTime(REVEAL_DURATION_MS - 1);
    expect(onDone).not.toHaveBeenCalled();
    vi.advanceTimersByTime(1);
    expect(onDone).toHaveBeenCalledTimes(1);
    vi.useRealTimers();
  });

  it("is keyboard-accessible via role=status + aria-live=polite", () => {
    const { container } = render(() => (
      <SpellCastReveal
        glyph="reverb"
        name="Hollow King"
        color="#7C5CF6"
        tag="Bundled"
        onDone={() => {}}
      />
    ));
    const root = container.querySelector('[role="status"]');
    expect(root).not.toBeNull();
    expect(root!.getAttribute("aria-live")).toBe("polite");
    expect(root!.getAttribute("aria-label")).toContain("Hollow King");
  });
});
