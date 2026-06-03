// Mixer — Phase 3 lights up the spell circle and the selected-rune
// inspector. Effects orbit a glowing voice core; threads of light
// connect enabled effects to the core; the inspector lets the user
// tweak the focused effect live.
//
// v0.11.3: the explicit Cast button is gone. The user drags anywhere
// on the Mixer (except on UI controls) to draw a glyph; the always-
// mounted SparkLayer renders the spark trail + canvas-based "omen"
// (shape outline + preset name + "◆ SPELL CAST ◆"). Matches the
// prototype's headline gesture: the Mixer *is* the casting surface.

import { createSignal, onCleanup, Show, type JSX } from "solid-js";
import { Badge } from "../components/Badge";
import { Inspector } from "../components/Inspector";
import { Kbd } from "../components/Kbd";
import { VMeter } from "../components/Meters";
import { Segmented } from "../components/Segmented";
import { Sigil, type SigilName } from "../components/Sigil";
import { SparkLayer } from "../components/SparkLayer";
import { SpellCircle } from "../components/SpellCircle";
import { Toggle } from "../components/Toggle";
import { statusMeta } from "../shell/statusMeta";
import { useApp } from "../stores/app";
import type { EffectId, GlyphId, Preset, PtmMode } from "../types";

export function MixerScreen(): JSX.Element {
  const app = useApp();
  const activeCount = () => app.chain().filter((c) => c.enabled).length;
  const totalCount = () => app.chain().length;
  const [castMessage, setCastMessage] = createSignal<string | null>(null);
  let messageTimeout: number | undefined;

  const flashMessage = (text: string): void => {
    setCastMessage(text);
    if (messageTimeout !== undefined) {
      window.clearTimeout(messageTimeout);
    }
    messageTimeout = window.setTimeout(() => {
      setCastMessage(null);
      messageTimeout = undefined;
    }, 2400);
  };

  /** Look up the preset bound to a recognised glyph. Returns null when
   *  the glyph has no binding — the SparkLayer then surfaces a flash. */
  const resolvePreset = (glyph: GlyphId): Preset | null => {
    const presetId = app.glyphs[glyph];
    return app.presets().find((p) => p.id === presetId) ?? null;
  };

  /** Apply the cast — switch to the bound preset. The SparkLayer has
   *  already started the omen animation by the time we land here. */
  const onCast = (preset: Preset): void => {
    app.usePreset(preset.id);
  };

  onCleanup(() => {
    if (messageTimeout !== undefined) {
      window.clearTimeout(messageTimeout);
    }
  });

  return (
    <div
      style={{
        height: "100%",
        display: "flex",
        "flex-direction": "column",
        padding: "20px 24px",
        gap: "var(--s5)",
        position: "relative",
      }}
    >
      <PresetHeader activeCount={activeCount()} totalCount={totalCount()} />
      <SparkLayer
        resolvePreset={resolvePreset}
        onCast={onCast}
        onMessage={flashMessage}
      />
      <Show when={castMessage()} keyed>
        {(text) => <CastFlash text={text} />}
      </Show>
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
          <SpellCircle
            chain={app.chain()}
            status={app.status()}
            motion={app.tweaks.motion}
            mystical={app.tweaks.mystical}
            selected={app.selectedEffect()}
            onSelect={(id) => app.setSelectedEffect(id)}
            onToggle={(id) => app.toggleEffectById(id)}
          />
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

function CastFlash(props: { text: string }): JSX.Element {
  return (
    <div
      style={{
        position: "absolute",
        bottom: "var(--s7)",
        left: "50%",
        transform: "translateX(-50%)",
        "z-index": 80,
        padding: "var(--s3) var(--s5)",
        "border-radius": "var(--r-pill)",
        background: "var(--surface-2)",
        border: "1px solid var(--line-glow)",
        "box-shadow": "var(--shadow-2)",
        "font-size": "var(--t-sm)",
        color: "var(--text-hi)",
        "pointer-events": "none",
      }}
    >
      {props.text}
    </div>
  );
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
        <Sigil name={app.preset().glyph as SigilName} size={22} />
      </div>
      <div style={{ display: "flex", "flex-direction": "column", gap: "2px" }}>
        <div style={{ display: "flex", "align-items": "center", gap: "var(--s2)" }}>
          <h2 class="display" style={{ "font-size": "26px", "font-weight": 700 }}>
            {app.preset().name}
          </h2>
          <Badge tone={app.preset().tag === "Bundled" ? "accent" : "info"}>
            {app.preset().tag}
          </Badge>
        </div>
        <div style={{ "font-size": "var(--t-sm)", color: "var(--text-lo)" }}>
          {props.activeCount} of {props.totalCount} runes active
          <Show when={app.streamInfo()}>
            {(info) => <span> · routed via {info().outputName}</span>}
          </Show>
          <Show when={app.engineRunning() && app.dspLatencyMs() >= 0.5}>
            <span title="Latency added by the active effects (e.g. Voice Convert ≈ 256 ms, Denoiser ≈ 10 ms)">
              {" "}
              · +{Math.round(app.dspLatencyMs())} ms latency
            </span>
          </Show>
        </div>
      </div>
      <div style={{ flex: 1 }} />
      <div style={{ display: "flex", "align-items": "center", gap: "var(--s2)" }}>
        <span class="eyebrow">Compare</span>
        <Segmented
          options={["A", "B"]}
          value={app.ui.ab}
          onChange={(v) => app.setAbSlot(v as "A" | "B")}
          accent
        />
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
      <RecordCard />
      <Inspector />
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
        class="btn btn-secondary btn-block"
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
  const pct = () => Math.round(app.monitorGain() * 100);
  return (
    <div
      class="card"
      style={{
        padding: "var(--s4)",
        display: "flex",
        "flex-direction": "column",
        gap: "var(--s3)",
      }}
    >
      <div
        style={{
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

      {/* v1.6.0: monitor volume — boost/cut how loud you hear yourself
          (applies to the separate monitor device). */}
      <div style={{ display: "flex", "align-items": "center", gap: "var(--s3)" }}>
        <span
          class="eyebrow"
          style={{ color: "var(--text-lo)", "flex-shrink": 0 }}
        >
          Volume
        </span>
        <input
          type="range"
          min="0"
          max="2"
          step="0.05"
          value={app.monitorGain()}
          disabled={!app.engineMonitoring()}
          onInput={(e) => app.setMonitorGain(parseFloat(e.currentTarget.value))}
          aria-label="Monitor volume"
          style={{ flex: 1, opacity: app.engineMonitoring() ? 1 : 0.5 }}
        />
        <span
          class="tnum"
          style={{
            "font-size": "var(--t-xs)",
            color: "var(--text-lo)",
            width: "40px",
            "text-align": "right",
          }}
        >
          {pct()}%
        </span>
      </div>
    </div>
  );
}

// Phase 16: one-click capture of the post-chain output to a WAV file.
// Recording rides the same processed mono the main output sends, so the
// file is exactly what call participants hear. Only available while the
// engine is running (the writer thread lives for the session).
function RecordCard(): JSX.Element {
  const app = useApp();
  const recording = () => app.isRecording();
  const canRecord = () => app.engineRunning();
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
        <span
          aria-hidden="true"
          style={{
            width: "14px",
            height: "14px",
            "border-radius": "var(--r-pill)",
            flex: "none",
            background: recording() ? "var(--danger)" : "var(--surface-3)",
            border: recording() ? "none" : "1px solid var(--line)",
            "box-shadow": recording() ? "0 0 10px var(--danger)" : "none",
            animation: recording() ? "breathe 1.4s ease-in-out infinite" : "none",
          }}
        />
        <div>
          <div style={{ "font-size": "var(--t-sm)", "font-weight": 600 }}>
            {recording() ? "Recording…" : "Record"}
          </div>
          <div style={{ "font-size": "var(--t-xs)", color: "var(--text-lo)" }}>
            {recording()
              ? "Capturing the modulated output"
              : "Save the modulated output to a WAV"}
          </div>
        </div>
      </div>
      <button
        type="button"
        class="btn btn-secondary"
        disabled={!canRecord() && !recording()}
        title={
          canRecord() || recording()
            ? "Toggle recording the modulated output"
            : "Start the engine to record"
        }
        style={{
          height: "38px",
          ...(recording()
            ? {
                background: "var(--danger-bg)",
                "border-color": "rgba(242, 86, 122, 0.5)",
                color: "var(--danger)",
              }
            : {}),
        }}
        onClick={() => void app.toggleRecording()}
      >
        {recording() ? "Stop" : "Record"}
      </button>
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

// Silence unused-import lint when Phase 3 hasn't wired ID inference yet.
void (null as EffectId | null);
