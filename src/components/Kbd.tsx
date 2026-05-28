// Kbd — keycap chip. Used for hotkey indicators and keyboard hints.

import type { JSX } from "solid-js";

export interface KbdProps {
  children: JSX.Element;
}

export function Kbd(props: KbdProps): JSX.Element {
  return <span class="kbd">{props.children}</span>;
}
