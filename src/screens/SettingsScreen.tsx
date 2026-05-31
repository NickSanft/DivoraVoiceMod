// Settings — Phase 6 fills in the remaining sections beyond the Phase 2
// Audio devices block:
//   • Virtual microphone (VB-Cable detection card + Discord/Zoom/OBS
//     instruction cards)
//   • Hotkeys (push-to-modulate / panic / toggle monitor — captured into
//     Tauri accelerator strings, registered via tauri-plugin-global-shortcut)
//   • Glyph casting (4 shapes → preset id mapping; the Phase 7 wizard reads
//     these to assign casts)
//   • Appearance (mood / accent / motion from Phase 1 + mystical / parchment
//     grain / vignette new in Phase 6)
//   • About (DMark + version + license + GitHub + three pillar cards +
//     Replay setup button that re-opens the first-run wizard)

import { createMemo, createSignal, For, onMount, Show, type JSX } from "solid-js";
import { Button } from "../components/Button";
import { Card } from "../components/Card";
import { HotkeyCapture } from "../components/HotkeyCapture";
import { DMark } from "../components/DMark";
import { HMeter } from "../components/Meters";
import { Segmented } from "../components/Segmented";
import { Select, type SelectOption } from "../components/Select";
import { Sigil, type SigilName } from "../components/Sigil";
import { Toggle } from "../components/Toggle";
import { clearWizardSeenFlag } from "../components/Wizard";
import {
  MYSTICAL_BALANCED,
  MYSTICAL_RICH,
  MYSTICAL_SUBTLE,
  useApp,
  type HotkeyAction,
} from "../stores/app";
import type { GlyphId, TweaksState } from "../types";

const VERSION = "v0.6.0";
const GITHUB_URL = "https://github.com/NickSanft/DivoraVoiceMod";

// Open a URL in the user's default browser. Lazy-loads the Tauri shell
// plugin so the screen still renders in a browser preview that doesn't
// have the Tauri bridge.
async function openExternal(url: string): Promise<void> {
  try {
    const { open } = await import("@tauri-apps/plugin-shell");
    await open(url);
  } catch (err) {
    console.warn("[settings] external open failed", err);
    // Fall back to a normal anchor in case we're running in a browser
    // preview without the Tauri shell plugin.
    if (typeof window !== "undefined") window.open(url, "_blank", "noopener");
  }
}

// HotkeyCapture takes/returns `string[]` (one chip per key). The store
// keeps a single Tauri accelerator string ("Ctrl+Shift+P", "Space", "").
function keysToAccelerator(keys: string[]): string {
  return keys.join("+");
}

function acceleratorToKeys(accelerator: string): string[] {
  if (!accelerator) return [];
  return accelerator.split("+").map((k) => k.trim()).filter(Boolean);
}

export function SettingsScreen(): JSX.Element {
  const app = useApp();
  const motionLabel = (m: number) =>
    m === 0 ? "functional" : m < 0.8 ? "ambient" : "rich";

  // Run a VB-Cable detection on first mount so the section can paint a
  // real status without the user having to click Re-scan. Also re-poll
  // audio devices so the Settings list reflects anything plugged in
  // since the app started — App-level focus refresh covers the alt-tab
  // case, but this catches the "user navigated to Settings *first*
  // thing after plugging in a device" path.
  onMount(() => {
    void app.refreshVirtualMicStatus();
    void app.refreshDevices();
    void app.refreshVoiceLibrary();
  });

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
  // Phase 13: monitor picker = "None" + every output device. "None"
  // (value "") means monitoring rides the main output (legacy behavior).
  const monitorOptions = createMemo<SelectOption[]>(() => [
    { value: "", label: "None — use main output", sub: "no separate monitor" },
    ...outputOptions(),
  ]);

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
        "max-width": "760px",
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
          Devices, virtual mic, hotkeys, glyph casting, and visual tweaks.
        </p>
      </div>

      <AudioDevicesSection
        inputOptions={inputOptions()}
        outputOptions={outputOptions()}
        monitorOptions={monitorOptions()}
        startStop={startStop}
      />

      <VoiceLibrarySection />

      <RecordingsSection />

      <VirtualMicSection />

      <HotkeysSection />

      <GlyphCastingSection />

      <AppearanceSection motionLabel={motionLabel} />

      <AboutSection />
    </div>
  );
}

// ---------- Audio devices ----------

interface AudioDevicesProps {
  inputOptions: SelectOption[];
  outputOptions: SelectOption[];
  monitorOptions: SelectOption[];
  startStop: () => void;
}

function AudioDevicesSection(props: AudioDevicesProps): JSX.Element {
  const app = useApp();
  const [refreshing, setRefreshing] = createSignal(false);
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

  /**
   * Manual re-enumeration. We mirror the focus-event path: the Settings
   * screen also re-polls on mount and on window focus, so this button
   * is the explicit fallback for the user who plugged in a device but
   * never lost focus (e.g. an internal switch that doesn't pop a system
   * tray, or running in a headless preview).
   *
   * Sets a transient `refreshing` flag so the button can render its
   * own progress affordance without locking up. cpal enumeration is
   * fast on Windows (~10 ms typical) but the network of awaits goes
   * through the IPC bridge, so the flag also guards against
   * double-clicks.
   */
  const onRefresh = async (): Promise<void> => {
    if (refreshing()) return;
    setRefreshing(true);
    try {
      await app.refreshDevices();
    } finally {
      setRefreshing(false);
    }
  };

  return (
    <section>
      <div
        style={{
          display: "flex",
          "align-items": "center",
          "justify-content": "space-between",
          gap: "var(--s3)",
          "margin-bottom": "var(--s3)",
        }}
      >
        <div class="eyebrow">Audio devices</div>
        <Button
          variant="ghost"
          size="sm"
          icon="refresh"
          onClick={() => void onRefresh()}
          disabled={refreshing()}
          title="Re-scan input + output devices (plugged in something?)"
        >
          {refreshing() ? "Scanning…" : "Refresh"}
        </Button>
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
        <FieldRow label="Input device">
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

        <FieldRow label="Output device">
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

        <FieldRow label="Monitor output (hear yourself)">
          <Select
            icon="monitor"
            value={app.selectedMonitor() ?? ""}
            options={props.monitorOptions}
            onChange={(v) => app.setSelectedMonitor(v === "" ? null : v)}
          />
          <div
            style={{
              "font-size": "var(--t-xs)",
              color: "var(--text-lo)",
              "margin-top": "6px",
              "line-height": 1.5,
            }}
          >
            Pick a second device (e.g. your headphones) to hear yourself
            while the main output routes elsewhere — set Output to CABLE
            Input for Discord/games and Monitor to your headphones. The
            Monitor toggle below mutes only this device, never the main
            send. Soundboard clips play here too.
          </div>
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

// ---------- Voice library (Phase 12) ----------

function fmtBytes(bytes: number): string {
  if (bytes >= 1_000_000) return `${(bytes / 1_000_000).toFixed(1)} MB`;
  if (bytes >= 1000) return `${Math.round(bytes / 1000)} KB`;
  return `${bytes} B`;
}

function VoiceLibrarySection(): JSX.Element {
  const app = useApp();
  const [refreshing, setRefreshing] = createSignal(false);
  const status = () => app.onnxStatus();
  const runtimeOk = () => status()?.runtimeAvailable ?? false;
  const dir = () => status()?.voicesDir ?? "";
  const voices = () => app.voices();

  const onRefresh = async (): Promise<void> => {
    if (refreshing()) return;
    setRefreshing(true);
    try {
      await app.refreshVoiceLibrary();
    } finally {
      setRefreshing(false);
    }
  };

  const openFolder = (): void => {
    const d = dir();
    if (d) void openExternal(d);
  };

  return (
    <section>
      <div
        style={{
          display: "flex",
          "align-items": "center",
          "justify-content": "space-between",
          gap: "var(--s3)",
          "margin-bottom": "var(--s3)",
        }}
      >
        <div class="eyebrow">Voice library</div>
        <Button
          variant="ghost"
          size="sm"
          icon="refresh"
          onClick={() => void onRefresh()}
          disabled={refreshing()}
          title="Re-scan the voices folder + ONNX runtime"
        >
          {refreshing() ? "Scanning…" : "Refresh"}
        </Button>
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
        {/* Runtime status */}
        <div
          style={{
            display: "flex",
            "align-items": "center",
            gap: "var(--s3)",
          }}
        >
          <span
            style={{
              width: "10px",
              height: "10px",
              "border-radius": "50%",
              flex: "none",
              background: runtimeOk() ? "var(--success)" : "var(--warning)",
              "box-shadow": `0 0 8px ${runtimeOk() ? "var(--success)" : "var(--warning)"}`,
            }}
          />
          <div style={{ flex: 1 }}>
            <div style={{ "font-size": "var(--t-sm)", color: "var(--text-mid)" }}>
              {runtimeOk()
                ? "ONNX Runtime detected — voice conversion can run."
                : "ONNX Runtime not installed — Voice Convert stays in passthrough."}
            </div>
            <Show when={!runtimeOk()}>
              <div
                style={{
                  "font-size": "var(--t-xs)",
                  color: "var(--text-lo)",
                  "margin-top": "2px",
                }}
              >
                Place <span class="mono">onnxruntime.dll</span> next to the app
                executable (or set <span class="mono">ORT_DYLIB_PATH</span>).
                Bundled installer support is coming in a later 0.12.x update.
              </div>
            </Show>
          </div>
        </div>

        {/* Active voice selection */}
        <FieldRow label="Active voice">
          <div
            style={{
              display: "flex",
              "flex-direction": "column",
              gap: "var(--s2)",
            }}
          >
            <VoiceRow
              label="None — pass my voice through"
              sub="Voice Convert acts as a bypass."
              selected={app.activeVoiceId() === null}
              onSelect={() => app.setActiveVoice(null)}
            />
            <For each={voices()}>
              {(v) => (
                <VoiceRow
                  label={v.name}
                  sub={`${fmtBytes(v.sizeBytes)} · ${v.id}.onnx`}
                  selected={app.activeVoiceId() === v.id}
                  onSelect={() => app.setActiveVoice(v.id)}
                />
              )}
            </For>
            <Show when={voices().length === 0}>
              <EmptyHint>
                No voice models found. Drop <span class="mono">.onnx</span> files
                into the voices folder, then Refresh.
              </EmptyHint>
            </Show>
          </div>
        </FieldRow>

        {/* Voices folder */}
        <div
          style={{
            display: "flex",
            "align-items": "center",
            "justify-content": "space-between",
            gap: "var(--s4)",
          }}
        >
          <div style={{ "min-width": 0 }}>
            <div class="eyebrow" style={{ "margin-bottom": "4px" }}>
              Voices folder
            </div>
            <div
              class="mono"
              style={{
                "font-size": "var(--t-xs)",
                color: "var(--text-lo)",
                "white-space": "nowrap",
                overflow: "hidden",
                "text-overflow": "ellipsis",
              }}
              title={dir()}
            >
              {dir() || "—"}
            </div>
          </div>
          <Button
            variant="secondary"
            size="sm"
            icon="folder"
            onClick={openFolder}
            disabled={!dir()}
          >
            Open folder
          </Button>
        </div>
      </div>
    </section>
  );
}

// ---------- Recordings (Phase 16) ----------

function RecordingsSection(): JSX.Element {
  const app = useApp();
  const [dir, setDir] = createSignal("");

  onMount(() => {
    void app
      .getRecordingsDir()
      .then(setDir)
      .catch((err) => console.warn("[settings] recordings dir unavailable", err));
  });

  const openFolder = (): void => {
    const d = dir();
    if (d) void openExternal(d);
  };

  return (
    <section>
      <div class="eyebrow" style={{ "margin-bottom": "var(--s3)" }}>
        Recordings
      </div>
      <div
        class="panel"
        style={{
          padding: "var(--s5)",
          display: "flex",
          "flex-direction": "column",
          gap: "var(--s4)",
        }}
      >
        <p style={{ "font-size": "var(--t-sm)", color: "var(--text-mid)", margin: 0 }}>
          Use the <strong>Record</strong> button on the Mixer to capture the
          modulated output (exactly what listeners hear) to a WAV file. Files
          land in this folder, timestamped.
        </p>
        <div
          style={{
            display: "flex",
            "align-items": "center",
            "justify-content": "space-between",
            gap: "var(--s4)",
          }}
        >
          <div style={{ "min-width": 0 }}>
            <div class="eyebrow" style={{ "margin-bottom": "4px" }}>
              Recordings folder
            </div>
            <div
              class="mono"
              style={{
                "font-size": "var(--t-xs)",
                color: "var(--text-lo)",
                "white-space": "nowrap",
                overflow: "hidden",
                "text-overflow": "ellipsis",
              }}
              title={dir()}
            >
              {dir() || "—"}
            </div>
          </div>
          <Button
            variant="secondary"
            size="sm"
            icon="folder"
            onClick={openFolder}
            disabled={!dir()}
          >
            Open folder
          </Button>
        </div>
        <Show when={app.recordingPath()}>
          {(path) => (
            <div
              style={{
                "font-size": "var(--t-xs)",
                color: "var(--text-lo)",
                "border-top": "1px solid var(--line)",
                "padding-top": "var(--s3)",
              }}
            >
              Last recording:{" "}
              <span class="mono" title={path()}>
                {path().split(/[\\/]/).pop()}
              </span>
            </div>
          )}
        </Show>
      </div>
    </section>
  );
}

interface VoiceRowProps {
  label: string;
  sub: string;
  selected: boolean;
  onSelect: () => void;
}

function VoiceRow(props: VoiceRowProps): JSX.Element {
  return (
    <button
      type="button"
      onClick={props.onSelect}
      style={{
        display: "flex",
        "align-items": "center",
        gap: "var(--s3)",
        padding: "var(--s3) var(--s4)",
        "border-radius": "var(--r-md)",
        border: props.selected
          ? "1px solid var(--line-glow)"
          : "1px solid var(--line)",
        background: props.selected ? "var(--accent-bg)" : "var(--surface-1)",
        cursor: "pointer",
        "text-align": "left",
        width: "100%",
      }}
    >
      <span
        style={{
          width: "16px",
          height: "16px",
          "border-radius": "50%",
          flex: "none",
          border: props.selected
            ? "5px solid var(--indigo)"
            : "2px solid var(--line-strong)",
          background: "var(--surface-0)",
          transition: "all 0.15s",
        }}
      />
      <span style={{ flex: 1, "min-width": 0 }}>
        <span
          style={{
            display: "block",
            "font-size": "var(--t-sm)",
            color: props.selected ? "var(--text-hi)" : "var(--text-mid)",
            "font-weight": props.selected ? 600 : 400,
          }}
        >
          {props.label}
        </span>
        <span
          style={{
            display: "block",
            "font-size": "var(--t-xs)",
            color: "var(--text-lo)",
            "white-space": "nowrap",
            overflow: "hidden",
            "text-overflow": "ellipsis",
          }}
        >
          {props.sub}
        </span>
      </span>
    </button>
  );
}

// ---------- Virtual microphone ----------

function VirtualMicSection(): JSX.Element {
  const app = useApp();
  const status = () => app.virtualMicStatus();
  const detected = () => status()?.detected ?? false;
  const downloadUrl = () => status()?.downloadUrl ?? "https://vb-audio.com/Cable/";

  return (
    <section>
      <div
        style={{
          display: "flex",
          "align-items": "center",
          "justify-content": "space-between",
          "margin-bottom": "var(--s3)",
        }}
      >
        <span class="eyebrow">Virtual microphone</span>
        <Button
          variant="ghost"
          size="sm"
          icon="refresh"
          onClick={() => void app.refreshVirtualMicStatus()}
        >
          Re-scan
        </Button>
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
        <div
          style={{
            display: "flex",
            "align-items": "flex-start",
            gap: "var(--s4)",
            padding: "var(--s4)",
            "border-radius": "var(--r-md)",
            background: detected() ? "var(--success-bg)" : "var(--warning-bg)",
            border: `1px solid ${
              detected() ? "rgba(52, 217, 160, 0.35)" : "rgba(233, 177, 76, 0.35)"
            }`,
          }}
        >
          <span
            style={{
              width: "32px",
              height: "32px",
              "border-radius": "var(--r-sm)",
              display: "grid",
              "place-items": "center",
              color: detected() ? "var(--success)" : "var(--warning)",
              background: detected()
                ? "rgba(52, 217, 160, 0.18)"
                : "rgba(233, 177, 76, 0.18)",
              "flex-shrink": 0,
            }}
          >
            <Sigil name={detected() ? "check" : "warning"} size={18} />
          </span>
          <div style={{ flex: 1, "min-width": 0 }}>
            <div
              style={{
                "font-size": "var(--t-sm)",
                "font-weight": 600,
                color: detected() ? "var(--success)" : "var(--warning)",
              }}
            >
              <Show when={detected()} fallback={"VB-Cable not detected"}>
                VB-Cable detected
              </Show>
            </div>
            <div
              style={{
                "font-size": "var(--t-xs)",
                color: "var(--text-mid)",
                "margin-top": "4px",
              }}
            >
              <Show
                when={detected()}
                fallback={
                  "Install VB-Cable to route DivoraVoice into Discord, Zoom, or OBS. After install, re-scan."
                }
              >
                <Show
                  when={status()?.cableInputDevice && status()?.cableOutputDevice}
                  fallback="Both ends of the cable are present."
                >
                  Route output to “{status()?.cableInputDevice?.name}” and tell
                  your call app to listen on “{status()?.cableOutputDevice?.name}”.
                </Show>
              </Show>
            </div>
          </div>
          <Show when={!detected()}>
            <Button
              variant="primary"
              size="sm"
              icon="download"
              onClick={() => void openExternal(downloadUrl())}
            >
              Download
            </Button>
          </Show>
        </div>

        <Show when={detected()}>
          <div
            style={{
              display: "grid",
              "grid-template-columns": "repeat(3, 1fr)",
              gap: "var(--s3)",
            }}
          >
            <CallAppCard
              name="Discord"
              detail="User Settings → Voice & Video → Input Device → CABLE Output"
            />
            <CallAppCard
              name="Zoom"
              detail="Settings → Audio → Microphone → CABLE Output"
            />
            <CallAppCard
              name="OBS"
              detail="Sources → Audio Input Capture → Device → CABLE Output"
            />
          </div>
        </Show>
      </div>
    </section>
  );
}

function CallAppCard(props: { name: string; detail: string }): JSX.Element {
  return (
    <Card
      style={{
        padding: "var(--s3) var(--s4)",
        display: "flex",
        "flex-direction": "column",
        gap: "4px",
      }}
    >
      <div
        style={{
          "font-size": "var(--t-sm)",
          "font-weight": 600,
          color: "var(--text-hi)",
        }}
      >
        {props.name}
      </div>
      <div style={{ "font-size": "var(--t-xs)", color: "var(--text-lo)" }}>
        {props.detail}
      </div>
    </Card>
  );
}

// ---------- Hotkeys ----------

interface HotkeyRowDef {
  action: HotkeyAction;
  label: string;
  desc: string;
  icon: SigilName;
}

const HOTKEY_ROWS: HotkeyRowDef[] = [
  {
    action: "ptm",
    label: "Push-to-modulate",
    desc: "Hold to apply the chain. Lifts back to clean.",
    icon: "bolt",
  },
  {
    action: "panic",
    label: "Panic — stop all clips",
    desc: "Instantly stops every playing soundboard clip.",
    icon: "panic",
  },
  {
    action: "monitor",
    label: "Toggle monitor",
    desc: "Mute / unmute the headphone sidetone without stopping the engine.",
    icon: "monitor",
  },
];

function HotkeysSection(): JSX.Element {
  const app = useApp();
  return (
    <section>
      <div class="eyebrow" style={{ "margin-bottom": "var(--s3)" }}>
        Hotkeys
      </div>
      <div
        class="panel"
        style={{
          padding: "var(--s5)",
          display: "flex",
          "flex-direction": "column",
          gap: "var(--s4)",
        }}
      >
        <For each={HOTKEY_ROWS}>
          {(row) => {
            const accelerator = () => app.hotkeyBindings[row.action];
            const keys = () => acceleratorToKeys(accelerator());
            return (
              <div
                style={{
                  display: "flex",
                  "align-items": "center",
                  gap: "var(--s4)",
                  "justify-content": "space-between",
                }}
              >
                <div
                  style={{
                    display: "flex",
                    "align-items": "flex-start",
                    gap: "var(--s3)",
                    "min-width": 0,
                  }}
                >
                  <span
                    style={{
                      width: "28px",
                      height: "28px",
                      "border-radius": "var(--r-sm)",
                      display: "grid",
                      "place-items": "center",
                      background: "var(--surface-3)",
                      color: "var(--text-mid)",
                      "flex-shrink": 0,
                    }}
                  >
                    <Sigil name={row.icon} size={15} />
                  </span>
                  <div style={{ "min-width": 0 }}>
                    <div
                      style={{
                        "font-size": "var(--t-sm)",
                        "font-weight": 600,
                        color: "var(--text-hi)",
                      }}
                    >
                      {row.label}
                    </div>
                    <div style={{ "font-size": "var(--t-xs)", color: "var(--text-lo)" }}>
                      {row.desc}
                    </div>
                  </div>
                </div>
                <div
                  style={{
                    display: "flex",
                    "align-items": "center",
                    gap: "var(--s2)",
                    "flex-shrink": 0,
                  }}
                >
                  <HotkeyCapture
                    value={keys()}
                    onChange={(next) =>
                      void app.setHotkeyBinding(row.action, keysToAccelerator(next))
                    }
                  />
                  <Show when={accelerator()}>
                    <Button
                      variant="ghost"
                      size="sm"
                      icon="x"
                      onClick={() => void app.clearHotkeyBinding(row.action)}
                      aria-label={`Clear ${row.label} hotkey`}
                    >
                      Clear
                    </Button>
                  </Show>
                </div>
              </div>
            );
          }}
        </For>
        <div
          style={{
            "font-size": "var(--t-xs)",
            color: "var(--text-lo)",
            "padding-top": "var(--s2)",
            "border-top": "1px dashed var(--line)",
          }}
        >
          Hotkeys are global — they fire even while another app is focused.
        </div>
      </div>
    </section>
  );
}

// ---------- Glyph casting ----------

interface GlyphRow {
  id: GlyphId;
  label: string;
  shape: JSX.Element;
}

const GLYPH_ROWS: GlyphRow[] = [
  {
    id: "triangle",
    label: "Triangle",
    shape: (
      <svg width="20" height="20" viewBox="0 0 24 24" aria-hidden="true">
        <path
          d="M12 4 L21 19 H3 Z"
          fill="none"
          stroke="currentColor"
          stroke-width="1.6"
          stroke-linejoin="round"
        />
      </svg>
    ),
  },
  {
    id: "invtriangle",
    label: "Inverted triangle",
    shape: (
      <svg width="20" height="20" viewBox="0 0 24 24" aria-hidden="true">
        <path
          d="M3 5 H21 L12 20 Z"
          fill="none"
          stroke="currentColor"
          stroke-width="1.6"
          stroke-linejoin="round"
        />
      </svg>
    ),
  },
  {
    id: "square",
    label: "Square",
    shape: (
      <svg width="20" height="20" viewBox="0 0 24 24" aria-hidden="true">
        <rect
          x="4"
          y="4"
          width="16"
          height="16"
          rx="2"
          fill="none"
          stroke="currentColor"
          stroke-width="1.6"
        />
      </svg>
    ),
  },
  {
    id: "circle",
    label: "Circle",
    shape: (
      <svg width="20" height="20" viewBox="0 0 24 24" aria-hidden="true">
        <circle
          cx="12"
          cy="12"
          r="8"
          fill="none"
          stroke="currentColor"
          stroke-width="1.6"
        />
      </svg>
    ),
  },
];

function GlyphCastingSection(): JSX.Element {
  const app = useApp();
  const presetOptions = createMemo<SelectOption[]>(() =>
    app.presets().map((p) => ({
      value: p.id,
      label: p.name,
      sub: p.tag,
    })),
  );

  return (
    <section>
      <div class="eyebrow" style={{ "margin-bottom": "var(--s3)" }}>
        Glyph casting
      </div>
      <div
        class="panel"
        style={{
          padding: "var(--s5)",
          display: "flex",
          "flex-direction": "column",
          gap: "var(--s4)",
        }}
      >
        <div style={{ "font-size": "var(--t-xs)", color: "var(--text-lo)" }}>
          Draw the shape on the Mixer to instantly switch to the bound preset.
        </div>
        <For each={GLYPH_ROWS}>
          {(row) => (
            <div
              style={{
                display: "flex",
                "align-items": "center",
                gap: "var(--s4)",
                "justify-content": "space-between",
              }}
            >
              <div
                style={{
                  display: "flex",
                  "align-items": "center",
                  gap: "var(--s3)",
                  "min-width": 0,
                  "flex-shrink": 0,
                  width: "180px",
                }}
              >
                <span
                  style={{
                    width: "34px",
                    height: "34px",
                    "border-radius": "var(--r-sm)",
                    display: "grid",
                    "place-items": "center",
                    background: "var(--surface-3)",
                    color: "var(--indigo)",
                    "flex-shrink": 0,
                  }}
                >
                  {row.shape}
                </span>
                <span
                  style={{
                    "font-size": "var(--t-sm)",
                    "font-weight": 600,
                    color: "var(--text-hi)",
                  }}
                >
                  {row.label}
                </span>
              </div>
              <div style={{ flex: 1, "min-width": 0 }}>
                <Select
                  icon="presets"
                  value={app.glyphs[row.id] ?? ""}
                  options={presetOptions()}
                  onChange={(v) => app.setGlyphs(row.id, v)}
                />
              </div>
            </div>
          )}
        </For>
      </div>
    </section>
  );
}

// ---------- Appearance ----------

interface AppearanceProps {
  motionLabel: (m: number) => string;
}

function AppearanceSection(props: AppearanceProps): JSX.Element {
  const app = useApp();
  // Threshold midpoints between the three discrete mystical values
  // (0.3 / 0.7 / 1.0) — so the segmented control snaps to the closest.
  const mysticalSegment = (): "subtle" | "balanced" | "rich" => {
    const m = app.tweaks.mystical;
    if (m <= 0.4) return "subtle";
    if (m <= 0.85) return "balanced";
    return "rich";
  };
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
        <Row label={`Mystical · ${mysticalSegment()}`}>
          <Segmented<"subtle" | "balanced" | "rich">
            options={[
              { value: "subtle", label: "Subtle" },
              { value: "balanced", label: "Balanced" },
              { value: "rich", label: "Rich" },
            ]}
            value={mysticalSegment()}
            onChange={(v) =>
              app.setTweaks(
                "mystical",
                v === "subtle"
                  ? MYSTICAL_SUBTLE
                  : v === "balanced"
                    ? MYSTICAL_BALANCED
                    : MYSTICAL_RICH,
              )
            }
          />
        </Row>
        <Row label="Parchment grain">
          <Toggle
            on={app.tweaks.grain}
            onChange={(next) => app.setTweaks("grain", next)}
            ariaLabel="Parchment grain"
          />
        </Row>
        <Row label="Vignette">
          <Toggle
            on={app.tweaks.vignette}
            onChange={(next) => app.setTweaks("vignette", next)}
            ariaLabel="Vignette"
          />
        </Row>
      </div>
    </section>
  );
}

// ---------- About ----------

interface PillarDef {
  icon: SigilName;
  title: string;
  body: string;
}

const PILLARS: PillarDef[] = [
  {
    icon: "shield",
    title: "No telemetry",
    body: "Nothing leaves your machine. Audio stays on device.",
  },
  {
    icon: "lock",
    title: "No account",
    body: "No sign-in, no cloud. Settings live in your AppData.",
  },
  {
    icon: "bolt",
    title: "Free forever",
    body: "MIT-licensed and open source. Read or fork the code.",
  },
];

function AboutSection(): JSX.Element {
  const app = useApp();
  return (
    <section>
      <div class="eyebrow" style={{ "margin-bottom": "var(--s3)" }}>
        About
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
        <div
          style={{
            display: "flex",
            "align-items": "center",
            gap: "var(--s4)",
          }}
        >
          <DMark size={48} radius={12} />
          <div style={{ flex: 1, "min-width": 0 }}>
            <div
              class="display"
              style={{ "font-size": "var(--t-h2)", "font-weight": 700 }}
            >
              DivoraVoice <span class="mono" style={{ color: "var(--text-lo)" }}>{VERSION}</span>
            </div>
            <div style={{ "font-size": "var(--t-xs)", color: "var(--text-lo)" }}>
              MIT License · Tauri + SolidJS
            </div>
          </div>
          <Button
            variant="secondary"
            icon="github"
            onClick={() => void openExternal(GITHUB_URL)}
          >
            View on GitHub
          </Button>
        </div>

        <div
          style={{
            display: "grid",
            "grid-template-columns": "repeat(3, 1fr)",
            gap: "var(--s3)",
          }}
        >
          <For each={PILLARS}>
            {(p) => (
              <Card
                style={{
                  padding: "var(--s4)",
                  display: "flex",
                  "flex-direction": "column",
                  gap: "var(--s2)",
                }}
              >
                <span
                  style={{
                    width: "30px",
                    height: "30px",
                    "border-radius": "var(--r-sm)",
                    display: "grid",
                    "place-items": "center",
                    background: "var(--accent-bg)",
                    color: "var(--indigo)",
                  }}
                >
                  <Sigil name={p.icon} size={17} />
                </span>
                <div
                  style={{
                    "font-size": "var(--t-sm)",
                    "font-weight": 600,
                    color: "var(--text-hi)",
                  }}
                >
                  {p.title}
                </div>
                <div style={{ "font-size": "var(--t-xs)", color: "var(--text-lo)" }}>
                  {p.body}
                </div>
              </Card>
            )}
          </For>
        </div>

        <div
          style={{
            display: "flex",
            "align-items": "center",
            "justify-content": "space-between",
            gap: "var(--s4)",
            "padding-top": "var(--s4)",
            "border-top": "1px dashed var(--line)",
          }}
        >
          <div>
            <div
              style={{
                "font-size": "var(--t-sm)",
                "font-weight": 600,
                color: "var(--text-hi)",
              }}
            >
              First-run setup
            </div>
            <div style={{ "font-size": "var(--t-xs)", color: "var(--text-lo)", "margin-top": "2px" }}>
              Re-open the welcome flow to pick devices, install VB-Cable, and bind glyphs.
            </div>
          </div>
          <Button
            variant="ghost"
            icon="refresh"
            onClick={() => {
              clearWizardSeenFlag();
              app.setWizardOpen(true);
            }}
          >
            Replay setup
          </Button>
        </div>
      </div>
    </section>
  );
}

// ---------- Shared layout helpers ----------

interface FieldRowProps {
  label: string;
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
