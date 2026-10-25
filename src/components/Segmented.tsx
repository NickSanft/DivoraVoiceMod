// Segmented — small tab group. Selected = surface-4; with `accent`
// the active option uses the brand gradient instead.
//
// v1.54.0: the whole group can be `disabled`, and so can one option, with a
// `title` that says why (Replay's lengths over this PC's memory ceiling).
// v1.58.0: `blurOnPointer` hands focus back to the page after a mouse or
// touch click, so Space reaches push-to-modulate again (Push to modulate's
// Hold to apply / Hold to bypass).

import type { JSX } from "solid-js";
import { For } from "solid-js";
import { blurOnPointer } from "./blurOnPointer";

export type SegmentedOption<T extends string> =
  | T
  | { value: T; label: string; disabled?: boolean; title?: string };

export interface SegmentedProps<T extends string> {
  options: SegmentedOption<T>[];
  value: T;
  onChange: (next: T) => void;
  accent?: boolean;
  disabled?: boolean;
  /** A pointer click leaves the focus on the page, not on the option. */
  blurOnPointer?: boolean;
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
              onClick={(e) => {
                if (props.blurOnPointer) blurOnPointer(e);
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
