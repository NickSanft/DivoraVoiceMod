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

  // v1.58.0: Push to modulate's Hold to apply / Hold to bypass. A focused
  // option takes Space, the in-app push-to-modulate key, so a pointer click
  // hands the focus back to the page; a keyboard click (detail 0) keeps it.
  it("blurOnPointer: a pointer click leaves the focus on the page, the keyboard keeps it", () => {
    const onChange = vi.fn();
    const { getByRole } = render(() => (
      <Segmented options={["A", "B"]} value="A" onChange={onChange} blurOnPointer />
    ));
    const b = getByRole("tab", { name: "B" });
    b.focus();
    fireEvent.click(b, { detail: 1 });
    expect(onChange).toHaveBeenCalledWith("B");
    expect(document.activeElement).toBe(document.body);

    b.focus();
    fireEvent.click(b, { detail: 0 });
    expect(onChange).toHaveBeenCalledTimes(2);
    expect(document.activeElement).toBe(b);
  });

  it("without blurOnPointer a pointer click keeps the focus, as before", () => {
    const { getByRole } = render(() => (
      <Segmented options={["A", "B"]} value="A" onChange={vi.fn()} />
    ));
    const b = getByRole("tab", { name: "B" });
    b.focus();
    fireEvent.click(b, { detail: 1 });
    expect(document.activeElement).toBe(b);
  });
});
