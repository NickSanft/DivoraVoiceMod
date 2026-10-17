import { describe, expect, it, vi } from "vitest";
import { fireEvent, render } from "@solidjs/testing-library";
import { Segmented } from "./Segmented";

describe("Segmented", () => {
  it("picks an option", () => {
    const onChange = vi.fn();
    const { getByRole } = render(() => (
      <Segmented options={["A", "B"]} value="A" onChange={onChange} />
    ));
    fireEvent.click(getByRole("tab", { name: "B" }));
    expect(onChange).toHaveBeenCalledWith("B");
  });

  // v1.54.0
  it("a disabled group changes nothing", () => {
    const onChange = vi.fn();
    const { getByRole } = render(() => (
      <Segmented options={["A", "B"]} value="A" onChange={onChange} disabled />
    ));
    const b = getByRole("tab", { name: "B" });
    expect(b).toBeDisabled();
    fireEvent.click(b);
    expect(onChange).not.toHaveBeenCalled();
  });

  it("one option can be disabled, with a title saying why", () => {
    const onChange = vi.fn();
    const { getByRole } = render(() => (
      <Segmented<string>
        options={[
          { value: "60", label: "60 s" },
          {
            value: "120",
            label: "120 s",
            disabled: true,
            title: "Needs up to 578 MB; this PC allows 536 MB.",
          },
        ]}
        value="60"
        onChange={onChange}
      />
    ));
    const over = getByRole("tab", { name: "120 s" });
    expect(over).toBeDisabled();
    expect(over).toHaveAttribute("title", "Needs up to 578 MB; this PC allows 536 MB.");
    fireEvent.click(over);
    expect(onChange).not.toHaveBeenCalled();

    const ok = getByRole("tab", { name: "60 s" });
    expect(ok).toBeEnabled();
    expect(ok).not.toHaveAttribute("title");
    fireEvent.click(ok);
    expect(onChange).toHaveBeenCalledWith("60");
  });
});
