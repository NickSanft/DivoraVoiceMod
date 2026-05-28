// Mixer (Phase 2 — half-real).
//
// Phase 2 lights up the IN/OUT vertical meters (driven by real audio
// when the engine is running) and the right rail's voice status,
// push-to-modulate, and monitor toggle cards. The spell circle in the
// middle is a Phase 3 deliverable; until then a calm placeholder fills
// that space without breaking the layout from the design screenshots.

import { Show, type JSX } from "solid-js";
import { Badge } from "../components/Badge";
import { Kbd } from "../components/Kbd";
import { VMeter } from "../components/Meters";
import { Segmented } from "../components/Segmented";
import { Sigil } from "../components/Sigil";
import { Toggle } from "../components/Toggle";
import { statusMeta } from "../shell/statusMeta";
import { useApp } from "../stores/app";
import type { PtmMode } from "../types";

export function MixerScreen(): JSX.Element {
  const app = useApp();
  const meta = () => statusMeta(app.status());
  const activeCount = () => app.chain().filter((c) => c.enabled).length;
  const totalCount = () => app.chain().length;

  return (
    <div
      style={{
        height: "100%",
        display: "flex",
        "flex-direction": "column",
        padding: "20px 24px",
        gap: "var(--s5)",
      }}
    >
      <PresetHeader activeCount={activeCount()} totalCount={totalCount()} />
      <div
        style={{
          flex: 1,
          display: "flex",
          gap: "var(--s7)",
          "min-height": 0,
          "align-items": "stretch",
        }}
      >
        <div
          style={{
            display: "flex",
            "flex-direction": "column",
            "align-items": "center",
            "justify-content": "center",
            gap: "var(--s3)",
          }}
        >
          <VMeter
            level={app.inputLevels().rms}
            peak={app.inputLevels().peak}
            height={320}
            label="In"
          />
          <DbReadout levels={app.inputLevels()} />
        </div>
        <div
          style={{
            flex: 1,
            display: "flex",
            "align-items": "center",
            "justify-content": "center",
          }}
        >
          <SpellCirclePlaceholder color={meta().color} />
        </div>
        <div
          style={{
            display: "flex",
            "flex-direction": "column",
            "align-items": "center",
            "justify-content": "center",
            gap: "var(--s3)",
          }}
        >
          <VMeter
            level={app.outputLevels().rms}
            peak={app.outputLevels().peak}
            height={320}
            label="Out"
          />
          <DbReadout levels={app.outputLevels()} />
        </div>
        <RightRail />
      </div>
    </div>
  );
}

interface PresetHeaderProps {
  activeCount: number;
  totalCount: number;
}

function PresetHeader(props: PresetHeaderProps): JSX.Element {
  const app = useApp();
  return (
    <div
      style={{
        display: "flex",
        "align-items": "center",
        gap: "var(--s4)",
        flex: "none",
      }}
    >
      <div
        style={{
          width: "38px",
          height: "38px",
          "border-radius": "var(--r-md)",
          display: "grid",
          "place-items": "center",
          background: app.preset().color + "26",
          border: `1px solid ${app.preset().color}55`,
          color: app.preset().color,
        }}
      >
        <Sigil name={app.preset().glyph as import("../components/Sigil").SigilName} size={22} />
      </div>
      <div style={{ display: "flex", "flex-direction": "column", gap: "2px" }}>
        <div style={{ display: "flex", "align-items": "center", gap: "var(--s2)" }}>
          <h2
            class="display"
            style={{ "font-size": "26px", "font-weight": 700 }}
          >
            {app.preset().name}
          </h2>
          <Badge tone={app.preset().tag === "Bundled" ? "accent" : "info"}>
            {app.preset().tag}
          </Badge>
        </div>
        <div
          style={{
            "font-size": "var(--t-sm)",
            color: "var(--text-lo)",
          }}
        >
          {props.activeCount} of {props.totalCount} runes active
          <Show when={app.streamInfo()}>
            {(info) => (
              <span>
                {" "}· routed via {info().outputName}
              </span>
            )}
          </Show>
        </div>
      </div>
      <div style={{ flex: 1 }} />
      <div style={{ display: "flex", "align-items": "center", gap: "var(--s2)" }}>
        <span class="eyebrow">Compare</span>
        <Segmented
          options={["A", "B"]}
          value={app.ui.ab}
          onChange={(v) => app.setUi("ab", v as "A" | "B")}
          accent
        />
      </div>
    </div>
  );
}

function SpellCirclePlaceholder(props: { color: string }): JSX.Element {
  return (
    <div
      style={{
        width: "min(440px, 100%)",
        "aspect-ratio": "1 / 1",
        "border-radius": "50%",
        border: `1px dashed ${props.color}55`,
        position: "relative",
        display: "grid",
        "place-items": "center",
        background: `radial-gradient(circle at center, ${props.color}11, transparent 70%)`,
        animation: "breathe calc(3.2s / max(var(--motion), 0.001)) ease-in-out infinite",
      }}
    >
      <div style={{ "text-align": "center", color: "var(--text-lo)" }}>
        <div class="eyebrow" style={{ "margin-bottom": "8px" }}>
          Spell circle
        </div>
        <div
          style={{
            "font-family": "var(--font-display)",
            "font-size": "var(--t-h2)",
            color: "var(--text-mid)",
            "font-weight": 600,
          }}
        >
          Arrives in Phase 3
        </div>
        <div style={{ "font-size": "var(--t-xs)", "margin-top": "6px" }}>
          Effects orbit a glowing voice core, threaded with light.
        </div>
      </div>
    </div>
  );
}

function RightRail(): JSX.Element {
  const app = useApp();
  return (
    <div
      style={{
        width: "290px",
        flex: "none",
        display: "flex",
        "flex-direction": "column",
        gap: "var(--s3)",
        overflow: "auto",
      }}
    >
      <VoiceStatusCard />
      <PushToModulateCard />
      <MonitorCard />
      <Show when={app.engineError()}>
        {(err) => (
          <div
            class="card"
            style={{
              padding: "var(--s4)",
              "border-color": "rgba(242, 86, 122, 0.4)",
              background: "var(--danger-bg)",
              color: "var(--danger)",
              "font-size": "var(--t-xs)",
              "line-height": 1.5,
            }}
          >
            <div
              class="eyebrow"
              style={{ "margin-bottom": "4px", color: "var(--danger)" }}
            >
              Engine error
            </div>
            {err()}
          </div>
        )}
      </Show>
      <Show when={!app.engineRunning() && !app.engineError()}>
        <div
          class="card"
          style={{
            padding: "var(--s4)",
            "font-size": "var(--t-xs)",
            color: "var(--text-lo)",
            "line-height": 1.5,
          }}
        >
          <div class="eyebrow" style={{ "margin-bottom": "4px" }}>
            Engine offline
          </div>
          Go to{" "}
          <button
            type="button"
            class="btn btn-ghost btn-sm"
            style={{ padding: "0 4px", height: "auto", display: "inline" }}
            onClick={() => app.setNav("settings")}
          >
            Settings
          </button>{" "}
          to pick devices and start passthrough.
        </div>
      </Show>
    </div>
  );
}

function VoiceStatusCard(): JSX.Element {
  const app = useApp();
  const meta = () => statusMeta(app.status());
  return (
    <div
      class="card"
      style={{
        padding: "var(--s4)",
        display: "flex",
        "align-items": "center",
        gap: "var(--s3)",
        background: meta().bg,
        "border-color": meta().line,
      }}
    >
      <div
        style={{
          width: "38px",
          height: "38px",
          "border-radius": "var(--r-md)",
          display: "grid",
          "place-items": "center",
          color: meta().color,
          background: "var(--surface-2)",
        }}
      >
        <Sigil name={meta().sigil} size={22} />
      </div>
      <div>
        <div
          style={{
            "font-family": "var(--font-display)",
            "font-size": "var(--t-h3)",
            "font-weight": 700,
            color: meta().color,
          }}
        >
          {meta().label}
        </div>
        <div style={{ "font-size": "var(--t-xs)", color: "var(--text-lo)" }}>
          {meta().sub}
        </div>
      </div>
    </div>
  );
}

function PushToModulateCard(): JSX.Element {
  const app = useApp();
  const pressed = () => app.ui.pressed;
  return (
    <div class="card" style={{ padding: "var(--s4)" }}>
      <div
        style={{
          display: "flex",
          "align-items": "center",
          "justify-content": "space-between",
          gap: "var(--s2)",
          "margin-bottom": "var(--s3)",
        }}
      >
        <span class="eyebrow">Push to modulate</span>
        <Kbd>{app.ui.ptmKey}</Kbd>
      </div>
      <Segmented<PtmMode>
        options={[
          { value: "apply", label: "Hold to apply" },
          { value: "bypass", label: "Hold to bypass" },
        ]}
        value={app.ui.ptmMode}
        onChange={(v) => app.setUi("ptmMode", v)}
      />
      <button
        type="button"
        class={`btn btn-secondary btn-block ${pressed() ? "" : ""}`}
        style={{
          "margin-top": "var(--s3)",
          height: "44px",
          background: pressed() ? "var(--accent-bg)" : undefined,
          "border-color": pressed() ? "var(--line-glow)" : undefined,
          color: pressed() ? "var(--indigo)" : undefined,
        }}
        onPointerDown={() => app.setUi("pressed", true)}
        onPointerUp={() => app.setUi("pressed", false)}
        onPointerLeave={() => app.setUi("pressed", false)}
      >
        {pressed()
          ? app.ui.ptmMode === "apply"
            ? "APPLYING"
            : "BYPASSED"
          : `Hold to test · press ${app.ui.ptmKey}`}
      </button>
    </div>
  );
}

function MonitorCard(): JSX.Element {
  const app = useApp();
  return (
    <div
      class="card"
      style={{
        padding: "var(--s4)",
        display: "flex",
        "align-items": "center",
        "justify-content": "space-between",
        gap: "var(--s3)",
      }}
    >
      <div
        style={{
          display: "flex",
          "align-items": "center",
          gap: "var(--s3)",
          color: "var(--text-mid)",
        }}
      >
        <Sigil name="monitor" size={20} style={{ color: "var(--indigo)" }} />
        <div>
          <div style={{ "font-size": "var(--t-sm)", "font-weight": 600 }}>
            Monitor
          </div>
          <div style={{ "font-size": "var(--t-xs)", color: "var(--text-lo)" }}>
            Hear yourself in headphones
          </div>
        </div>
      </div>
      <Toggle
        on={app.engineMonitoring()}
        onChange={(next) => void app.setMonitor(next)}
        ariaLabel="Sidetone monitor"
      />
    </div>
  );
}

interface DbReadoutProps {
  levels: { rms: number; peak: number };
}

function DbReadout(props: DbReadoutProps): JSX.Element {
  const fmt = (v: number) => {
    if (v <= 1e-6) return "−∞";
    const db = 20 * Math.log10(v);
    return `${db.toFixed(0)} dB`;
  };
  return (
    <div
      style={{
        "font-family": "var(--font-mono)",
        "font-size": "var(--t-xs)",
        color: "var(--text-lo)",
      }}
      class="tnum"
    >
      {fmt(props.levels.peak)}
    </div>
  );
}
