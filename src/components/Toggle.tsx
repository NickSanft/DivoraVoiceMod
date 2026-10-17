// Toggle — 40x23 switch with sliding knob. Gradient track when on;
// `tone` lets you swap the on-tint to danger or success.
//
// v1.54.0: `disabled` (Settings' screen-recording rows while Replay is on).

import type { JSX } from "solid-js";

export interface ToggleProps {
  on: boolean;
  onChange?: (next: boolean) => void;
  tone?: "" | "danger" | "success";
  ariaLabel?: string;
  disabled?: boolean;
  title?: string;
}

export function Toggle(props: ToggleProps): JSX.Element {
  return (
    <button
      type="button"
      class={`toggle ${props.on ? "is-on" : ""} ${props.tone ? `tone-${props.tone}` : ""}`}
      role="switch"
      aria-checked={props.on}
      aria-label={props.ariaLabel}
      disabled={props.disabled}
      title={props.title}
      onClick={() => {
        if (!props.disabled) props.onChange?.(!props.on);
      }}
    >
      <span class="toggle-knob" />
    </button>
  );
}
