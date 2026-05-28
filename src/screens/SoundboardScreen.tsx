// Soundboard (Phase 1 placeholder). Real folder picker, tile grid,
// hotkeys, panic button, and progress rings land in Phase 5.

import type { JSX } from "solid-js";
import { EmptyState } from "../components/EmptyState";

export function SoundboardScreen(): JSX.Element {
  return (
    <div style={{ height: "100%", display: "grid", "place-items": "center" }}>
      <EmptyState icon="soundboard" title="Soundboard">
        Pick a folder of audio clips and they will appear here as
        clickable tiles. Coming together in Phase 5.
      </EmptyState>
    </div>
  );
}
