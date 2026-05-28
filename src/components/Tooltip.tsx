// Tooltip — wraps any element with a hover-revealed surface-4 bubble.

import type { JSX } from "solid-js";

export interface TooltipProps {
  label: string;
  children: JSX.Element;
}

export function Tooltip(props: TooltipProps): JSX.Element {
  return (
    <span class="tip" style={{ display: "inline-flex" }}>
      {props.children}
      <span class="tip-body">{props.label}</span>
    </span>
  );
}
