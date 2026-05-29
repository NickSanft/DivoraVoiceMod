// Inspector — the right-rail card that lets the user fine-tune the
// currently selected effect. Sigil + name + enable toggle along the
// top, description, then one slider per parameter (bipolar for signed
// params).

import { For, Show, type JSX } from "solid-js";
import { EFFECTS } from "../data/effects";
import { useApp } from "../stores/app";
import type { EffectId } from "../types";
import { Sigil, type SigilName } from "./Sigil";
import { Slider } from "./Slider";
import { Toggle } from "./Toggle";

export function Inspector(): JSX.Element {
  const app = useApp();

  const selectedDef = () => {
    const id = app.selectedEffect();
    return id ? EFFECTS[id] : null;
  };

  const selectedIndex = () => {
    const id = app.selectedEffect();
    if (!id) return -1;
    return app.chain().findIndex((c) => c.id === id);
  };

  const selectedEntry = () => {
    const idx = selectedIndex();
    return idx >= 0 ? app.chain()[idx] : undefined;
  };

  const formatValue = (v: number, unit: string): string => {
    if (Math.abs(v) >= 100 || Number.isInteger(v)) {
      return `${v}${unit}`;
    }
    return `${v.toFixed(1)}${unit}`;
  };

  return (
    <Show when={selectedDef() && selectedEntry()}>
      {(() => {
        const def = selectedDef()!;
        const entry = selectedEntry()!;
        const idx = selectedIndex();
        return (
          <div
            class="card"
            style={{
              padding: "var(--s4)",
              flex: 1,
              "min-height": 0,
              display: "flex",
              "flex-direction": "column",
            }}
          >
            <div
              style={{
                display: "flex",
                "align-items": "center",
                gap: "9px",
                "margin-bottom": "4px",
              }}
            >
              <span
                style={{
                  color: entry.enabled ? "var(--indigo)" : "var(--text-lo)",
                }}
              >
                <Sigil name={def.sigil as SigilName} size={18} />
              </span>
              <span
                style={{
                  "font-weight": 600,
                  "font-size": "var(--t-sm)",
                  flex: 1,
                }}
              >
                {def.name}
              </span>
              <Toggle
                on={entry.enabled}
                onChange={() => app.toggleEffectById(def.id as EffectId)}
                ariaLabel={`Enable ${def.name}`}
              />
            </div>
            <div
              style={{
                "font-size": "var(--t-xs)",
                color: "var(--text-lo)",
                "line-height": 1.4,
                "margin-bottom": "10px",
              }}
            >
              {def.desc}
            </div>
            <div
              style={{
                display: "flex",
                "flex-direction": "column",
                gap: "12px",
                opacity: entry.enabled ? 1 : 0.45,
                "pointer-events": entry.enabled ? "auto" : "none",
              }}
            >
              <For each={def.params}>
                {(p) => {
                  const value = () => entry.vals[p.key] ?? p.default;
                  return (
                    <div>
                      <div
                        style={{
                          display: "flex",
                          "justify-content": "space-between",
                          "margin-bottom": "5px",
                        }}
                      >
                        <span
                          style={{
                            "font-size": "var(--t-xs)",
                            color: "var(--text-mid)",
                          }}
                        >
                          {p.label}
                        </span>
                        <span
                          class="mono tnum"
                          style={{
                            "font-size": "var(--t-xs)",
                            color: "var(--text-hi)",
                          }}
                        >
                          {formatValue(value(), p.unit)}
                        </span>
                      </div>
                      <Slider
                        value={value()}
                        min={p.min}
                        max={p.max}
                        step={p.step}
                        bipolar={p.bipolar}
                        onChange={(v) => app.setChainParam(idx, p.key, v)}
                        ariaLabel={`${def.name} ${p.label}`}
                      />
                    </div>
                  );
                }}
              </For>
            </div>
          </div>
        );
      })()}
    </Show>
  );
}
