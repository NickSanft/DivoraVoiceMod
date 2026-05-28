// Select — 42px field with a leading sigil and a popover dropdown.
// Used for device pickers, preset glyph dropdowns, etc.

import type { JSX } from "solid-js";
import { createSignal, For, onCleanup, onMount, Show } from "solid-js";
import { Sigil, type SigilName } from "./Sigil";

export interface SelectOption {
  value: string;
  label: string;
  sub?: string;
}

export interface SelectProps {
  icon?: SigilName;
  value: string;
  options: SelectOption[];
  onChange: (next: string) => void;
  /** Subtitle shown when the current option has no sub of its own. */
  sub?: string;
}

export function Select(props: SelectProps): JSX.Element {
  const [open, setOpen] = createSignal(false);
  let rootRef: HTMLDivElement | undefined;

  onMount(() => {
    const handler = (e: PointerEvent) => {
      if (rootRef && !rootRef.contains(e.target as Node)) setOpen(false);
    };
    document.addEventListener("pointerdown", handler);
    onCleanup(() => document.removeEventListener("pointerdown", handler));
  });

  const current = () =>
    props.options.find((o) => o.value === props.value) ?? props.options[0];
  const icon = () => props.icon ?? "mic";

  return (
    <div ref={rootRef} style={{ position: "relative" }}>
      <button
        type="button"
        class={`select ${open() ? "is-open" : ""}`}
        onClick={() => setOpen(!open())}
        aria-haspopup="listbox"
        aria-expanded={open()}
      >
        <span class="select-icon">
          <Sigil name={icon()} size={18} />
        </span>
        <span class="select-text">
          <div class="select-main">{current()?.label ?? "—"}</div>
          <Show when={current()?.sub ?? props.sub}>
            <div class="select-sub">{current()?.sub ?? props.sub}</div>
          </Show>
        </span>
        <Sigil
          name="chevronD"
          size={16}
          style={{
            color: "var(--text-lo)",
            transform: open() ? "rotate(180deg)" : "none",
            transition: "transform 0.16s",
          }}
        />
      </button>
      <Show when={open()}>
        <div
          class="dropdown"
          role="listbox"
          style={{
            position: "absolute",
            top: "calc(100% + 6px)",
            left: 0,
            right: 0,
            "z-index": 50,
          }}
        >
          <For each={props.options}>
            {(o) => (
              <div
                class={`dropdown-opt ${o.value === props.value ? "is-selected" : ""}`}
                role="option"
                aria-selected={o.value === props.value}
                onClick={() => {
                  props.onChange(o.value);
                  setOpen(false);
                }}
              >
                <Sigil
                  name={icon()}
                  size={16}
                  style={{
                    color: o.value === props.value ? "var(--indigo)" : "var(--text-lo)",
                  }}
                />
                <div style={{ flex: 1 }}>
                  <div style={{ "font-size": "var(--t-sm)", "font-weight": 600 }}>
                    {o.label}
                  </div>
                  <Show when={o.sub}>
                    <div style={{ "font-size": "var(--t-xs)", color: "var(--text-lo)" }}>
                      {o.sub}
                    </div>
                  </Show>
                </div>
                <Show when={o.value === props.value}>
                  <Sigil name="check" size={15} style={{ color: "var(--indigo)" }} />
                </Show>
              </div>
            )}
          </For>
        </div>
      </Show>
    </div>
  );
}
