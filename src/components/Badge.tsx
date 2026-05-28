// Badge — mono uppercase pill with semantic tone variants.

import type { JSX } from "solid-js";
import { Show } from "solid-js";
import { Sigil, type SigilName } from "./Sigil";
import type { Tone } from "../types";

export interface BadgeProps {
  tone?: Tone;
  icon?: SigilName;
  children: JSX.Element;
}

export function Badge(props: BadgeProps): JSX.Element {
  return (
    <span class={`badge ${props.tone ? `tone-${props.tone}` : ""}`}>
      <Show when={props.icon}>
        <Sigil name={props.icon as SigilName} size={11} />
      </Show>
      {props.children}
    </span>
  );
}
