// Settings — Phase 2 lights up the Audio devices section: input picker,
// live input confirmation meter, output picker, engine start/stop, and
// monitor toggle. The Appearance section that landed in Phase 1 stays
// as the Tweaks affordance. The remaining sections (Virtual mic,
// Hotkeys, Glyph casting, About) arrive in Phase 6.

import { createMemo, Show, type JSX } from "solid-js";
import { Button } from "../components/Button";
import { HMeter } from "../components/Meters";
import { Segmented } from "../components/Segmented";
import { Select, type SelectOption } from "../components/Select";
import { Sigil } from "../components/Sigil";
import { Toggle } from "../components/Toggle";
import { useApp } from "../stores/app";
import type { TweaksState } from "../types";

export function SettingsScreen(): JSX.Element {
  const app = useApp();
  const motionLabel = (m: number) =>
    m === 0 ? "functional" : m < 0.8 ? "ambient" : "rich";

  const inputOptions = createMemo<SelectOption[]>(() =>
    app.audioInputs().map((d) => ({
      value: d.name,
      label: d.name,
      sub: `${d.defaultSampleRate} Hz · ${d.channels === 1 ? "mono" : `${d.channels} ch`}${
        d.isDefault ? " · default" : ""
      }`,
    })),
  );
  const outputOptions = createMemo<SelectOption[]>(() =>
    app.audioOutputs().map((d) => ({
      value: d.name,
      label: d.name,
      sub: `${d.defaultSampleRate} Hz · ${d.channels === 1 ? "mono" : `${d.channels} ch`}${
        d.isDefault ? " · default" : ""
      }`,
    })),
  );

  const startStop = () => {
    if (app.engineRunning()) {
      void app.stopEngine();
    } else {
      void app.startEngine();
    }
  };

  return (
    <div
      style={{
        height: "100%",
        padding: "var(--s8) var(--s7)",
        overflow: "auto",
        display: "flex",
        "flex-direction": "column",
        gap: "var(--s7)",
        "max-width": "680px",
        margin: "0 auto",
      }}
    >
      <div>
        <h2
          class="display"
          style={{ "font-size": "var(--t-h1)", "margin-bottom": "var(--s2)" }}
        >
          Settings
        </h2>
        <p style={{ color: "var(--text-mid)", "font-size": "var(--t-sm)" }}>
          Audio devices is live in Phase 2. The remaining sections (virtual
          mic detection, hotkeys, glyph casting, about) arrive in Phase 6.
        </p>
      </div>

      <AudioDevicesSection
        inputOptions={inputOptions()}
        outputOptions={outputOptions()}
        startStop={startStop}
      />

      <AppearanceSection motionLabel={motionLabel} />
    </div>
  );
}

interface AudioDevicesProps {
  inputOptions: SelectOption[];
  outputOptions: SelectOption[];
  startStop: () => void;
}

function AudioDevicesSection(props: AudioDevicesProps): JSX.Element {
  const app = useApp();
  const inputCount = () => app.audioInputs().length;
  const outputCount = () => app.audioOutputs().length;
  const statusLabel = () => {
    if (app.engineError()) return `Error: ${app.engineError()}`;
    if (app.engineRunning()) {
      const info = app.streamInfo();
      if (info) return `Running at ${info.sampleRate} Hz`;
      return "Running";
    }
    return "Stopped";
  };

  return (
    <section>
      <div class="eyebrow" style={{ "margin-bottom": "var(--s3)" }}>
        Audio devices
      </div>
      <div
        class="panel"
        style={{
          padding: "var(--s5)",
          display: "flex",
          "flex-direction": "column",
          gap: "var(--s5)",
        }}
      >
        <FieldRow label="Input device" sigil="mic">
          <Show
            when={inputCount() > 0}
            fallback={<EmptyHint>No input devices detected.</EmptyHint>}
          >
            <Select
              icon="mic"
              value={app.selectedInput() ?? ""}
              options={props.inputOptions}
              onChange={(v) => app.setSelectedInput(v)}
            />
          </Show>
        </FieldRow>

        <div>
          <div class="field-label">Input level</div>
          <HMeter level={app.inputLevels().rms} peak={app.inputLevels().peak} />
          <div
            style={{
              "font-family": "var(--font-mono)",
              "font-size": "var(--t-xs)",
              color: "var(--text-lo)",
              "margin-top": "6px",
            }}
            class="tnum"
          >
            RMS {(app.inputLevels().rms * 100).toFixed(0)}% · Peak{" "}
            {(app.inputLevels().peak * 100).toFixed(0)}%
          </div>
        </div>

        <FieldRow label="Output device" sigil="output">
          <Show
            when={outputCount() > 0}
            fallback={<EmptyHint>No output devices detected.</EmptyHint>}
          >
            <Select
              icon="output"
              value={app.selectedOutput() ?? ""}
              options={props.outputOptions}
              onChange={(v) => app.setSelectedOutput(v)}
            />
          </Show>
        </FieldRow>

        <div
          style={{
            display: "flex",
            "align-items": "center",
            gap: "var(--s4)",
            "justify-content": "space-between",
          }}
        >
          <div>
            <div
              class="eyebrow"
              style={{ "margin-bottom": "4px", "letter-spacing": "0.16em" }}
            >
              Engine
            </div>
            <div style={{ "font-size": "var(--t-sm)", color: "var(--text-mid)" }}>
              {statusLabel()}
            </div>
          </div>
          <Button
            variant={app.engineRunning() ? "danger" : "primary"}
            onClick={props.startStop}
            icon={app.engineRunning() ? "stop" : "play"}
          >
            {app.engineRunning() ? "Stop" : "Start"}
          </Button>
        </div>

        <div
          style={{
            display: "flex",
            "align-items": "center",
            "justify-content": "space-between",
            gap: "var(--s4)",
          }}
        >
          <div>
            <div style={{ "font-size": "var(--t-sm)", color: "var(--text-mid)" }}>
              <Sigil
                name="monitor"
                size={15}
                style={{
                  display: "inline-block",
                  "vertical-align": "middle",
                  "margin-right": "6px",
                  color: "var(--text-lo)",
                }}
              />
              Monitor (hear yourself in headphones)
            </div>
            <div style={{ "font-size": "var(--t-xs)", color: "var(--text-lo)", "margin-top": "2px" }}>
              When off, the engine still meters but the output device is silent.
            </div>
          </div>
          <Toggle
            on={app.engineMonitoring()}
            onChange={(next) => void app.setMonitor(next)}
            ariaLabel="Sidetone monitor"
          />
        </div>
      </div>
    </section>
  );
}

interface FieldRowProps {
  label: string;
  sigil?: import("../components/Sigil").SigilName;
  children: JSX.Element;
}

function FieldRow(props: FieldRowProps): JSX.Element {
  return (
    <div>
      <div class="field-label">{props.label}</div>
      {props.children}
    </div>
  );
}

function EmptyHint(props: { children: JSX.Element }): JSX.Element {
  return (
    <div
      style={{
        "font-size": "var(--t-sm)",
        color: "var(--text-lo)",
        padding: "var(--s3) 0",
      }}
    >
      {props.children}
    </div>
  );
}

interface AppearanceProps {
  motionLabel: (m: number) => string;
}

function AppearanceSection(props: AppearanceProps): JSX.Element {
  const app = useApp();
  return (
    <section>
      <div class="eyebrow" style={{ "margin-bottom": "var(--s3)" }}>
        Appearance
      </div>
      <div
        class="panel"
        style={{
          padding: "var(--s5)",
          display: "flex",
          "flex-direction": "column",
          gap: "var(--s5)",
        }}
      >
        <Row label="Color mood">
          <Segmented<TweaksState["mood"]>
            options={[
              { value: "violet", label: "Dusk Violet" },
              { value: "ink", label: "Ink + Candle" },
              { value: "midnight", label: "Midnight" },
            ]}
            value={app.tweaks.mood}
            onChange={(v) => app.setTweaks("mood", v)}
          />
        </Row>
        <Row label="Accent">
          <Segmented<TweaksState["accent"]>
            options={[
              { value: "brand", label: "Brand" },
              { value: "abyssal", label: "Abyssal" },
              { value: "ember", label: "Ember" },
            ]}
            value={app.tweaks.accent}
            onChange={(v) => app.setTweaks("accent", v)}
            accent
          />
        </Row>
        <Row label={`Motion · ${props.motionLabel(app.tweaks.motion)}`}>
          <Segmented<"functional" | "ambient" | "rich">
            options={[
              { value: "functional", label: "Functional" },
              { value: "ambient", label: "Ambient" },
              { value: "rich", label: "Rich" },
            ]}
            value={
              app.tweaks.motion === 0
                ? "functional"
                : app.tweaks.motion < 0.8
                  ? "ambient"
                  : "rich"
            }
            onChange={(v) =>
              app.setTweaks(
                "motion",
                v === "functional" ? 0 : v === "ambient" ? 0.6 : 1,
              )
            }
          />
        </Row>
      </div>
    </section>
  );
}

interface RowProps {
  label: string;
  children: JSX.Element;
}

function Row(props: RowProps): JSX.Element {
  return (
    <div
      style={{
        display: "flex",
        "align-items": "center",
        "justify-content": "space-between",
        gap: "var(--s4)",
      }}
    >
      <span style={{ "font-size": "var(--t-sm)", color: "var(--text-mid)" }}>
        {props.label}
      </span>
      {props.children}
    </div>
  );
}
