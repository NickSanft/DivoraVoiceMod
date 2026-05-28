// Button — primary, secondary, ghost, danger variants in three sizes.
// Optional leading/trailing sigil. The styling lives in styles.css under
// `@layer components` — see the `.btn` family.

import type { JSX } from "solid-js";
import { Show, splitProps } from "solid-js";
import { Sigil, type SigilName } from "./Sigil";

export type ButtonVariant = "primary" | "secondary" | "ghost" | "danger";
export type ButtonSize = "sm" | "default" | "lg";

export interface ButtonProps extends JSX.ButtonHTMLAttributes<HTMLButtonElement> {
  variant?: ButtonVariant;
  size?: ButtonSize;
  icon?: SigilName;
  iconR?: SigilName;
  block?: boolean;
  /** Use the `solid` modifier on the danger variant. */
  solid?: boolean;
}

export function Button(props: ButtonProps): JSX.Element {
  const [local, rest] = splitProps(props, [
    "variant",
    "size",
    "icon",
    "iconR",
    "block",
    "solid",
    "children",
    "class",
  ]);
  const variant = (): ButtonVariant => local.variant ?? "secondary";
  const size = (): ButtonSize => local.size ?? "default";
  const iconSize = () => (size() === "sm" ? 15 : 17);
  return (
    <button
      type="button"
      class={`btn btn-${variant()} ${size() === "sm" ? "btn-sm" : ""} ${
        size() === "lg" ? "btn-lg" : ""
      } ${local.block ? "btn-block" : ""} ${local.solid ? "btn-solid" : ""} ${
        local.class ?? ""
      }`}
      {...rest}
    >
      <Show when={local.icon}>
        <Sigil name={local.icon as SigilName} size={iconSize()} />
      </Show>
      {local.children}
      <Show when={local.iconR}>
        <Sigil name={local.iconR as SigilName} size={iconSize()} />
      </Show>
    </button>
  );
}
