// EmptyState — dashed-ring glyph + display title + helper copy +
// optional trailing action. Used by every screen during Phase 1 while
// the real content hasn't landed yet.

import type { JSX } from "solid-js";
import { Show } from "solid-js";
import { Sigil, type SigilName } from "./Sigil";

export interface EmptyStateProps {
  icon?: SigilName;
  title: string;
  children?: JSX.Element;
  action?: JSX.Element;
}

export function EmptyState(props: EmptyStateProps): JSX.Element {
  const icon = () => props.icon ?? "presets";
  return (
    <div class="empty">
      <div class="empty-glyph">
        <Sigil name={icon()} size={28} />
      </div>
      <div>
        <h3 class="empty-title">{props.title}</h3>
        <Show when={props.children}>
          <p class="empty-body">{props.children}</p>
        </Show>
      </div>
      <Show when={props.action}>{props.action}</Show>
    </div>
  );
}
