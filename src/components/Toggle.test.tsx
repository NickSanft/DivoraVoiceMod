import { describe, expect, it, vi } from "vitest";
import { fireEvent, render } from "@solidjs/testing-library";
import { Toggle } from "./Toggle";

describe("Toggle", () => {
  it("is a switch that asks for the other state", () => {
    const onChange = vi.fn();
    const { getByRole } = render(() => <Toggle on={false} onChange={onChange} ariaLabel="X" />);
    const sw = getByRole("switch", { name: "X" });
    expect(sw).toHaveAttribute("aria-checked", "false");
    fireEvent.click(sw);
    expect(onChange).toHaveBeenCalledWith(true);
  });

  // v1.54.0
  it("disabled blocks the change, and can say why", () => {
    const onChange = vi.fn();
    const { getByRole } = render(() => (
      <Toggle
        on
        onChange={onChange}
        ariaLabel="X"
        disabled
        title="Replay is on — turn it off to change these."
      />
    ));
    const sw = getByRole("switch", { name: "X" });
    expect(sw).toBeDisabled();
    expect(sw).toHaveAttribute("title", "Replay is on — turn it off to change these.");
    fireEvent.click(sw);
    expect(onChange).not.toHaveBeenCalled();
  });
});
