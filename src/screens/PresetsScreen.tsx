// Presets (Phase 1 placeholder). Real browser/editor with drag-reorder
// chain cards, Export JSON, and A/B compare snapshots land in Phase 4.

import type { JSX } from "solid-js";
import { EmptyState } from "../components/EmptyState";
import { PRESETS } from "../data/presets";

export function PresetsScreen(): JSX.Element {
  return (
    <div style={{ height: "100%", display: "grid", "place-items": "center" }}>
      <EmptyState icon="presets" title="Presets">
        {PRESETS.length} presets loaded ({PRESETS.filter((p) => p.tag === "Bundled").length}{" "}
        bundled, {PRESETS.filter((p) => p.tag === "User").length} user). Full
        browser + editor in Phase 4.
      </EmptyState>
    </div>
  );
}
