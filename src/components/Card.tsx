// Card / Panel surfaces. `Card` = surface-2 + line border; `Panel` =
// surface-1 + slightly larger radius (used for the sidebar's preset
// list and other persistent containers).

import type { JSX } from "solid-js";
import { splitProps } from "solid-js";

export interface CardProps extends JSX.HTMLAttributes<HTMLDivElement> {
  hover?: boolean;
}

export function Card(props: CardProps): JSX.Element {
  const [local, rest] = splitProps(props, ["hover", "class", "children"]);
  return (
    <div class={`card ${local.hover ? "is-hover" : ""} ${local.class ?? ""}`} {...rest}>
      {local.children}
    </div>
  );
}

export function Panel(props: JSX.HTMLAttributes<HTMLDivElement>): JSX.Element {
  const [local, rest] = splitProps(props, ["class", "children"]);
  return (
    <div class={`panel ${local.class ?? ""}`} {...rest}>
      {local.children}
    </div>
  );
}
