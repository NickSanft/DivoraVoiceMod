// Sidebar — 80px icon rail. Four nav items (Mixer / Board / Presets /
// Settings); active item gets a 3px gradient bar + glow. Footer has a
// "replay first-run" bolt button and a green "Local-first · no
// telemetry" shield badge.

import { For, Show, type JSX } from "solid-js";
import { Sigil, type SigilName } from "../components/Sigil";
import { Tooltip } from "../components/Tooltip";
import { useApp } from "../stores/app";
import type { NavId } from "../types";

interface NavItem {
  id: NavId;
  label: string;
  sigil: SigilName;
}

const NAV: NavItem[] = [
  { id: "mixer", label: "Mixer", sigil: "mixer" },
  { id: "coven", label: "Coven", sigil: "modulated" },
  { id: "soundboard", label: "Board", sigil: "soundboard" },
  { id: "presets", label: "Presets", sigil: "presets" },
  { id: "settings", label: "Settings", sigil: "settings" },
];

export function Sidebar(): JSX.Element {
  const app = useApp();
  return (
    <div class="sidebar">
      <For each={NAV}>
        {(n) => (
          <button
            type="button"
            class={`sidebar-nav ${app.nav() === n.id ? "is-active" : ""}`}
            onClick={() => app.setNav(n.id)}
            aria-current={app.nav() === n.id ? "page" : undefined}
          >
            <Show when={app.nav() === n.id}>
              <span class="sidebar-nav-bar" />
            </Show>
            <span class="sidebar-nav-sigil">
              <Sigil name={n.sigil} size={23} />
            </span>
            <span class="sidebar-nav-label">{n.label}</span>
          </button>
        )}
      </For>
      <div style={{ flex: 1 }} />
      <Tooltip label="Replay first-run setup">
        <button
          type="button"
          class="icon-btn"
          onClick={() => app.setWizardOpen(true)}
          aria-label="Replay first-run setup"
        >
          <Sigil name="bolt" size={18} />
        </button>
      </Tooltip>
      <div class="sidebar-shield" title="Local-first · no telemetry">
        <Sigil name="shield" size={18} />
      </div>
    </div>
  );
}
