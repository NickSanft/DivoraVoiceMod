// Segmented — small tab group. Selected = surface-4; with `accent`
// the active option uses the brand gradient instead.
//
// v1.54.0: the whole group can be `disabled`, and so can one option, with a
// `title` that says why (Replay's lengths over this PC's memory ceiling).

import type { JSX } from "solid-js";
import { For } from "solid-js";

export type SegmentedOption<T extends string> =
  | T
  | { value: T; label: string; disabled?: boolean; title?: string };

export interface SegmentedProps<T extends string> {
  options: SegmentedOption<T>[];
  value: T;
  onChange: (next: T) => void;
  accent?: boolean;
  disabled?: boolean;
}

export function Segmented<T extends string>(props: SegmentedProps<T>): JSX.Element {
  return (
    <div class="seg" role="tablist">
      <For each={props.options}>
        {(o) => {
          const value = typeof o === "string" ? o : o.value;
          const label = typeof o === "string" ? o : o.label;
          const off = () => !!props.disabled || (typeof o !== "string" && !!o.disabled);
          return (
            <button
              type="button"
              role="tab"
              aria-selected={props.value === value}
              class={`seg-btn ${props.value === value ? "is-on" : ""} ${
                props.accent ? "is-accent" : ""
              }`}
              disabled={off()}
              title={typeof o === "string" ? undefined : o.title}
              onClick={() => {
                if (!off()) props.onChange(value);
              }}
            >
              {label}
            </button>
          );
        }}
      </For>
    </div>
  );
}
