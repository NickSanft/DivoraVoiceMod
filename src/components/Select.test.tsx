// v1.60.0: Select names its value, never shows a value it doesn't list as
// its first option, and works from the keyboard by the active-descendant
// model (D28). The hosts (Settings, the Mixer's recording card, the
// first-run Wizard) run their own suites unchanged.

import { createSignal } from "solid-js";
import { fireEvent, render } from "@solidjs/testing-library";
import { describe, expect, it, vi } from "vitest";
import { listboxStep, Select, type SelectOption } from "./Select";

const OPTIONS: SelectOption[] = [
  { value: "hollow-king", label: "Hollow King" },
  { value: "villager", label: "Villager", sub: "Bundled" },
  { value: "seraph", label: "Seraph" },
];

/** A Select that keeps what it is given, as its hosts do. */
function mountSelect(more: { value?: string; ariaLabel?: string; missing?: string; blurOnPointer?: boolean } = {}) {
  const onChange = vi.fn();
  const view = render(() => {
    const [value, setValue] = createSignal(more.value ?? "villager");
    return (
      <Select
        icon="presets"
        value={value()}
        options={OPTIONS}
        onChange={(v) => {
          onChange(v);
          setValue(v);
        }}
        ariaLabel={more.ariaLabel}
        missing={more.missing}
        blurOnPointer={more.blurOnPointer}
      />
    );
  });
  const trigger = view.container.querySelector<HTMLButtonElement>("button.select")!;
  /** The option the trigger points at, by its words. */
  const highlighted = (): string | null => {
    const id = trigger.getAttribute("aria-activedescendant");
    return id ? (document.getElementById(id)?.textContent ?? null) : null;
  };
  return { view, trigger, onChange, highlighted };
}

describe("Select (v1.60.0)", () => {
  it("is named for what it picks and the value it shows", () => {
    const { view, trigger } = mountSelect({ ariaLabel: "Preset BRB switches to at the start" });
    expect(view.getByRole("button", { name: "Preset BRB switches to at the start: Villager" })).toBe(trigger);
    expect(trigger).toHaveAttribute("aria-haspopup", "listbox");
    expect(trigger).toHaveAttribute("aria-expanded", "false");
    // The value is still what shows.
    expect(trigger.querySelector(".select-main")).toHaveTextContent("Villager");
    expect(trigger.querySelector(".select-sub")).toHaveTextContent("Bundled");
  });

  it("without ariaLabel its name is what it shows, as before", () => {
    const { trigger } = mountSelect();
    expect(trigger).not.toHaveAttribute("aria-label");
    expect(trigger.querySelector(".select-main")).toHaveTextContent("Villager");
  });

  it("a value it doesn't list shows `missing`, never the first option", () => {
    const { trigger } = mountSelect({ value: "gone", ariaLabel: "Preset BRB switches to at zero", missing: "Missing preset" });
    expect(trigger.querySelector(".select-main")).toHaveTextContent("Missing preset");
    expect(trigger).toHaveAttribute("aria-label", "Preset BRB switches to at zero: Missing preset");
    expect(trigger.textContent).not.toContain("Hollow King");
    // Without `missing`, a dash.
    const plain = mountSelect({ value: "gone" });
    expect(plain.trigger.querySelector(".select-main")).toHaveTextContent("—");
    expect(plain.trigger.textContent).not.toContain("Hollow King");
  });

  it("Enter, Space and Down open it on the chosen option; the focus stays on the trigger", () => {
    for (const key of ["Enter", " ", "ArrowDown"]) {
      const { view, trigger, highlighted } = mountSelect({ ariaLabel: "Pick" });
      trigger.focus();
      fireEvent.keyDown(trigger, { key });
      expect(trigger, key).toHaveAttribute("aria-expanded", "true");
      const list = view.getByRole("listbox");
      expect(trigger).toHaveAttribute("aria-controls", list.id);
      // Each option has its own id, and the chosen one is selected.
      const options = view.getAllByRole("option");
      expect(new Set(options.map((o) => o.id)).size).toBe(3);
      expect(options.map((o) => o.getAttribute("aria-selected"))).toEqual(["false", "true", "false"]);
      expect(highlighted()).toContain("Villager");
      expect(document.activeElement).toBe(trigger);
      view.unmount();
    }
  });

  it("Up, Down, Home and End move the highlight, stopping at the ends", () => {
    const { trigger, highlighted } = mountSelect();
    trigger.focus();
    fireEvent.keyDown(trigger, { key: "Enter" });
    const steps: Array<[string, string]> = [
      ["ArrowDown", "Seraph"],
      ["ArrowDown", "Seraph"],
      ["Home", "Hollow King"],
      ["ArrowUp", "Hollow King"],
      ["End", "Seraph"],
      ["ArrowUp", "Villager"],
    ];
    for (const [key, words] of steps) {
      fireEvent.keyDown(trigger, { key });
      expect(highlighted(), key).toContain(words);
    }
    expect(document.activeElement).toBe(trigger);
  });

  it("Enter and Space pick the highlighted option and close; the focus stays", () => {
    const { trigger, onChange, highlighted } = mountSelect({ ariaLabel: "Pick" });
    trigger.focus();
    fireEvent.keyDown(trigger, { key: "Enter" });
    fireEvent.keyDown(trigger, { key: "ArrowDown" });
    fireEvent.keyDown(trigger, { key: "Enter" });
    expect(onChange.mock.calls).toEqual([["seraph"]]);
    expect(trigger).toHaveAttribute("aria-expanded", "false");
    expect(trigger).not.toHaveAttribute("aria-activedescendant");
    expect(trigger).toHaveAttribute("aria-label", "Pick: Seraph");
    expect(document.activeElement).toBe(trigger);

    fireEvent.keyDown(trigger, { key: " " });
    expect(highlighted()).toContain("Seraph");
    fireEvent.keyDown(trigger, { key: "Home" });
    fireEvent.keyDown(trigger, { key: " " });
    expect(onChange.mock.calls).toEqual([["seraph"], ["hollow-king"]]);
    expect(trigger).toHaveAttribute("aria-label", "Pick: Hollow King");
    expect(document.activeElement).toBe(trigger);
  });

  it("a key's open and every move scroll the highlight into view, the least it takes; a pointer's hover doesn't", () => {
    // jsdom has no scrollIntoView: a spy that notes which element it was
    // asked of.
    // The options it was asked of, each as the id the trigger pointed at.
    const scrolled: string[] = [];
    const before = Object.getOwnPropertyDescriptor(Element.prototype, "scrollIntoView");
    const spy = vi.fn(function (this: Element, arg?: unknown) {
      expect(arg).toEqual({ block: "nearest" });
      expect(this.getAttribute("role")).toBe("option");
      scrolled.push(this.id);
    });
    Element.prototype.scrollIntoView = spy as unknown as Element["scrollIntoView"];
    try {
      const { view, trigger, highlighted } = mountSelect({ value: "seraph" });
      const pointedAt = (): string => trigger.getAttribute("aria-activedescendant") ?? "";
      trigger.focus();
      // Opened on the chosen option, the last: shown at once.
      fireEvent.keyDown(trigger, { key: "Enter" });
      expect(highlighted()).toContain("Seraph");
      expect(scrolled).toEqual([pointedAt()]);
      for (const [key, words] of [
        ["Home", "Hollow King"],
        ["ArrowDown", "Villager"],
        ["End", "Seraph"],
        ["ArrowUp", "Villager"],
      ] as const) {
        const n = scrolled.length;
        fireEvent.keyDown(trigger, { key });
        expect(highlighted(), key).toContain(words);
        expect(scrolled.slice(n), key).toEqual([pointedAt()]);
      }
      // A hover highlights, and scrolls nothing.
      fireEvent.pointerEnter(view.getByRole("option", { name: /Hollow King/ }));
      expect(scrolled).toHaveLength(5);
      // Nor does a pointer's open.
      fireEvent.keyDown(trigger, { key: "Escape" });
      fireEvent.click(trigger, { detail: 1 });
      expect(view.getByRole("listbox")).toBeInTheDocument();
      expect(scrolled).toHaveLength(5);
    } finally {
      if (before) Object.defineProperty(Element.prototype, "scrollIntoView", before);
      else delete (Element.prototype as { scrollIntoView?: unknown }).scrollIntoView;
    }
  });

  it("Esc closes without picking and keeps the focus; Tab closes", () => {
    const { view, trigger, onChange } = mountSelect();
    trigger.focus();
    fireEvent.keyDown(trigger, { key: "Enter" });
    fireEvent.keyDown(trigger, { key: "ArrowDown" });
    fireEvent.keyDown(trigger, { key: "Escape" });
    expect(view.queryByRole("listbox")).toBeNull();
    expect(onChange).not.toHaveBeenCalled();
    expect(document.activeElement).toBe(trigger);
    fireEvent.keyDown(trigger, { key: "Enter" });
    fireEvent.keyDown(trigger, { key: "Tab" });
    expect(view.queryByRole("listbox")).toBeNull();
    expect(onChange).not.toHaveBeenCalled();
  });

  it("a click opens it and a click on an option picks it, as before", () => {
    const { view, trigger, onChange } = mountSelect();
    fireEvent.click(trigger, { detail: 1 });
    fireEvent.click(view.getByRole("option", { name: /Hollow King/ }), { detail: 1 });
    expect(onChange.mock.calls).toEqual([["hollow-king"]]);
    expect(view.queryByRole("listbox")).toBeNull();
    // A pointer down outside closes it.
    fireEvent.click(trigger, { detail: 1 });
    fireEvent.pointerDown(document.body);
    expect(view.queryByRole("listbox")).toBeNull();
  });

  it("blurOnPointer: a pointer's open and pick leave the focus on the page; the keyboard keeps it", () => {
    const { view, trigger, onChange } = mountSelect({ blurOnPointer: true });
    trigger.focus();
    fireEvent.click(trigger, { detail: 1 });
    expect(document.activeElement).toBe(document.body);
    trigger.focus();
    fireEvent.click(view.getByRole("option", { name: /Seraph/ }), { detail: 1 });
    expect(onChange.mock.calls).toEqual([["seraph"]]);
    expect(document.activeElement).toBe(document.body);

    // From the keyboard (a click with detail 0, or the keys), it stays.
    trigger.focus();
    fireEvent.click(trigger, { detail: 0 });
    expect(document.activeElement).toBe(trigger);
    fireEvent.keyDown(trigger, { key: "Enter" });
    expect(document.activeElement).toBe(trigger);
  });

  it("without blurOnPointer a pointer click keeps the focus, as before", () => {
    const { trigger } = mountSelect();
    trigger.focus();
    fireEvent.click(trigger, { detail: 1 });
    expect(document.activeElement).toBe(trigger);
  });

  it("listboxStep", () => {
    expect(listboxStep("ArrowDown", 0, 3)).toBe(1);
    expect(listboxStep("ArrowDown", 2, 3)).toBe(2);
    expect(listboxStep("ArrowUp", 0, 3)).toBe(0);
    expect(listboxStep("ArrowUp", 2, 3)).toBe(1);
    expect(listboxStep("Home", 2, 3)).toBe(0);
    expect(listboxStep("End", 0, 3)).toBe(2);
    expect(listboxStep("a", 1, 3)).toBe(1);
    expect(listboxStep("End", 0, 0)).toBe(0);
  });
});
