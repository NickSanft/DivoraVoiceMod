// Mixer (Phase 1 placeholder).
//
// Phase 3 lands the real Mixer: the spell circle with effects orbiting
// the voice core, vertical IN/OUT meters, push-to-modulate card, etc.
// For now we show an EmptyState that names the active preset so the
// state plumbing is visible.

import type { JSX } from "solid-js";
import { EmptyState } from "../components/EmptyState";
import { useApp } from "../stores/app";

export function MixerScreen(): JSX.Element {
  const app = useApp();
  return (
    <div style={{ height: "100%", display: "grid", "place-items": "center" }}>
      <EmptyState icon="mixer" title="Mixer">
        Spell circle for{" "}
        <span style={{ color: "var(--text-mid)", "font-weight": 600 }}>
          {app.preset().name}
        </span>{" "}
        will appear here in Phase 3.
      </EmptyState>
    </div>
  );
}
