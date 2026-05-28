// IconButton — square 34px button containing one Sigil. Optionally
// wrapped in a Tooltip via the `tip` prop. `active` swaps to the accent
// state (used by the Tweaks panel toggles, etc.).

import type { JSX } from "solid-js";
import { Show, splitProps } from "solid-js";
import { Sigil, type SigilName } from "./Sigil";

export interface IconButtonProps extends JSX.ButtonHTMLAttributes<HTMLButtonElement> {
  icon: SigilName;
  size?: number;
  active?: boolean;
  tip?: string;
}

export function IconButton(props: IconButtonProps): JSX.Element {
  const [local, rest] = splitProps(props, ["icon", "size", "active", "tip", "class"]);
  const button = (
    <button
      type="button"
      class={`icon-btn ${local.active ? "is-active" : ""} ${local.class ?? ""}`}
      {...rest}
    >
      <Sigil name={local.icon} size={local.size ?? 19} />
    </button>
  );
  return (
    <Show when={local.tip} fallback={button}>
      <span class="tip" style={{ display: "inline-flex" }}>
        {button}
        <span class="tip-body">{local.tip}</span>
      </span>
    </Show>
  );
}
