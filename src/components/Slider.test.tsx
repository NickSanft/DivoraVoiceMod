import { describe, expect, it, vi } from "vitest";
import { render } from "@solidjs/testing-library";
import { Slider } from "./Slider";

/** jsdom lays nothing out: give the track a 100 px box at x = 0. */
function sized(el: Element): void {
  el.getBoundingClientRect = () =>
    ({ left: 0, width: 100, top: 0, height: 22, right: 100, bottom: 22, x: 0, y: 0 }) as DOMRect;
}

/** jsdom has no PointerEvent; a mouse event of that type carries clientX. */
function press(el: Element, clientX: number): void {
  el.dispatchEvent(new MouseEvent("pointerdown", { clientX, bubbles: true }));
  window.dispatchEvent(new MouseEvent("pointerup", { bubbles: true }));
}

describe("Slider", () => {
  it("a press sets the value under the pointer", () => {
    const onChange = vi.fn();
    const { getByRole } = render(() => <Slider value={0} onChange={onChange} ariaLabel="X" />);
    const slider = getByRole("slider", { name: "X" });
    sized(slider);
    press(slider, 40);
    expect(onChange).toHaveBeenCalledWith(40);
  });

  // v1.54.0: Settings' desktop volume and A/V offset while Replay is on.
  it("disabled blocks the change and says so", () => {
    const onChange = vi.fn();
    const { getByRole } = render(() => (
      <Slider value={0} onChange={onChange} ariaLabel="X" disabled />
    ));
    const slider = getByRole("slider", { name: "X" });
    sized(slider);
    expect(slider).toHaveAttribute("aria-disabled", "true");
    press(slider, 40);
    expect(onChange).not.toHaveBeenCalled();
  });
});
