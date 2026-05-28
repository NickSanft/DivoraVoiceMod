// Toggle — 40x23 switch with sliding knob. Gradient track when on;
// `tone` lets you swap the on-tint to danger or success.

import type { JSX } from "solid-js";

export interface ToggleProps {
  on: boolean;
  onChange?: (next: boolean) => void;
  tone?: "" | "danger" | "success";
  ariaLabel?: string;
}

export function Toggle(props: ToggleProps): JSX.Element {
  return (
    <button
      type="button"
      class={`toggle ${props.on ? "is-on" : ""} ${props.tone ? `tone-${props.tone}` : ""}`}
      role="switch"
      aria-checked={props.on}
      aria-label={props.ariaLabel}
      onClick={() => props.onChange?.(!props.on)}
    >
      <span class="toggle-knob" />
    </button>
  );
}
