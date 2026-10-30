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
//   • v1.59.0: Hourglass (countdowns and count-ups the backend keeps, after
//     Stream overlay); v1.60.0: a countdown's At start and At zero lists,
//     End now and Add break (BRB)

import {
  createEffect,
  createMemo,
  createSignal,
  createUniqueId,
  For,
  Index,
  on,
  onCleanup,
  onMount,
  Show,
  untrack,
  type JSX,
} from "solid-js";
import { Badge } from "../components/Badge";
import { Button } from "../components/Button";
import { Card } from "../components/Card";
import { HotkeyCapture } from "../components/HotkeyCapture";
import { IconButton } from "../components/IconButton";
import { DMark } from "../components/DMark";
import { Kbd } from "../components/Kbd";
import { resamplingLegs } from "../audio/api";
import { HMeter } from "../components/Meters";
import { Segmented } from "../components/Segmented";
import { Slider } from "../components/Slider";
import { LISTBOX_MOVES, listboxStep, revealOption, Select, type SelectOption } from "../components/Select";
import { Sigil, type SigilName } from "../components/Sigil";
import { Toggle } from "../components/Toggle";
import { blurOnPointer } from "../components/blurOnPointer";
import { clearWizardSeenFlag } from "../components/Wizard";
import { GlyphRecorder, GlyphPreview } from "../components/GlyphRecorder";
import { type DiagStatus } from "../audio/diagnostics";
import type { Point } from "../data/glyphs";
import type { OverlayBg } from "../overlay/state";
import { charmLabels, layoutScene, MAX_PICTURES, pictureCount } from "../overlay/scene";
import { openExternal } from "../lib/openExternal";
import { EFFECTS } from "../data/effects";
import {
  MYSTICAL_BALANCED,
  MYSTICAL_RICH,
  MYSTICAL_SUBTLE,
  useApp,
  type HotkeyAction,
} from "../stores/app";
import type {
  ChainEntry,
  GlyphAction,
  GlyphId,
  MidiActionKind,
  MidiMapping,
  TweaksState,
} from "../types";

import { acceleratorToKeys, keysToAccelerator } from "../data/hotkeys";
import {
  lengthText,
  MAX_NAME,
  MAX_TIMERS,
  parseLength,
  phaseWord,
  readTimerName,
  sameKey,
  timerAction,
  timerAnnouncement,
  type StoredTimer,
  type TimerEdges,
  type TimerVerb,
} from "../data/timers";
import type { TimerPhase, TimerState } from "../overlay/timerState";
import {
  ACTION_KINDS,
  actionsForSync,
  actionWords,
  canAddAction,
  pairingWarnings,
  readAction,
  readStoredActions,
  restores,
  revertsAtZero,
  timerVoiceLine,
  type ActionNames,
  type ActionRow,
  type PairingWarning,
  type TimerAction,
  type TimerActionKind,
  type TimerHook,
} from "../data/timerActions";
import { transforms } from "../data/ptm";
import {
  MAX_AV_OFFSET_MS,
  MAX_DESKTOP_GAIN_DB,
  MIN_DESKTOP_GAIN_DB,
} from "../data/screenRecording";
import {
  lengthTitle,
  memoryLabel,
  REPLAY_LENGTHS,
  replayOn,
  sanitizeReplaySeconds,
} from "../data/replay";
import type { ScreenQuality } from "../audio/api";

const GITHUB_URL = "https://github.com/NickSanft/DivoraVoiceMod";
const ISSUES_URL = "https://github.com/NickSanft/DivoraVoiceMod/issues/new";

// HotkeyCapture takes/returns `string[]` (one chip per key). The store
// keeps a single Tauri accelerator string ("Ctrl+Shift+P", "Space", "").
// The converters live in data/hotkeys.ts (v1.51.4): the "Win" chip has to
// become "Super" in the accelerator, or the registration fails.

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

      <DiagnosticsSection />

      <VoiceLibrarySection />

      <RecordingsSection />

      <VirtualMicSection />

      <HotkeysSection />

      <ControlSurfacesSection />

      <GlyphCastingSection />

      <AppearanceSection motionLabel={motionLabel} />

      <StreamOverlaySection />

      <HourglassSection />

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
  // v1.56.0: the engine's rule (engine.rs, Phase 13): a monitor device only
  // counts when it isn't the main output. With none, the Monitor toggle
  // gates the main output itself (`main_output_plays`), so the note below
  // must not say it never touches the send.
  const separateMonitor = (): boolean => {
    const monitor = app.selectedMonitor();
    return !!monitor && monitor !== app.selectedOutput();
  };
  const statusLabel = () => {
    if (app.engineError()) return `Error: ${app.engineError()}`;
    if (app.engineRunning()) {
      const info = app.streamInfo();
      if (!info) return "Running";
      // Name the conversions that are actually happening. Before v1.51.2 only
      // one rate was carried here, so a mismatch was invisible — and the path
      // that handled it was wrecking the audio. Windows sets each device's rate
      // on its own, so a user can land here without touching anything.
      //
      // Not a chain: the monitor converts from the ENGINE rate, not from the
      // output's, so `in → out → monitor` described a pipeline that does not
      // exist, with a leg that was not a conversion at all when the output
      // matched.
      const legs = resamplingLegs(info);
      if (legs.length === 0) return `Running at ${info.sampleRate} Hz`;
      const each = legs
        .map((l) => `${l.from} → ${l.to} Hz to the ${l.device}`)
        .join(", ");
      return `Running, resampling: ${each}`;
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

        <div>
          <div class="field-label">Noise gate calibration</div>
          <div
            style={{
              display: "flex",
              "align-items": "center",
              gap: "var(--s3)",
              "flex-wrap": "wrap",
            }}
          >
            <Button
              variant="secondary"
              size="sm"
              icon="gate"
              onClick={() => void app.runCalibration()}
              disabled={app.calibrating()}
            >
              {app.calibrating() ? "Listening… stay quiet" : "Auto-calibrate gate"}
            </Button>
            <Show
              when={app.calibrationResult()}
              fallback={
                <span style={{ "font-size": "var(--t-xs)", color: "var(--text-lo)" }}>
                  Stay quiet for ~2 s; sets the active preset’s gate to your
                  room’s measured noise floor.
                </span>
              }
            >
              {(r) => (
                <span
                  class="tnum"
                  style={{ "font-size": "var(--t-xs)", color: "var(--text-mid)" }}
                >
                  Floor {r().noiseFloorDb.toFixed(0)} dB → gate {r().gateThreshDb} dB
                  {r().denoiserMix > 0 ? ` · denoiser ${r().denoiserMix}%` : ""}
                </span>
              )}
            </Show>
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
            Input for Discord/games and Monitor to your headphones.
            Soundboard clips play here too.{" "}
            {separateMonitor()
              ? "The Monitor toggle below turns your voice on or off on this device only, never on the main send. Clips and Speak here follow the Monitor checkbox on Speak."
              : "With no separate monitor device, the Monitor toggle below turns Divora's whole output on or off, the call included."}
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
              When off, you stop hearing yourself; the engine keeps metering
              your mic. With no separate monitor device, Divora's whole output
              is silent, the call included.
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
          modulated output (exactly what listeners hear) to a WAV file, or
          <strong> Record screen</strong> to capture a screen as an MP4 with your
          modulated voice and soundboard. Files land in this folder, timestamped.
        </p>
        <ScreenRecordingRows />
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
        {/* v1.54.0: below the folder row, which its copy points back up to —
            replays go to the same folder; there is no second one. */}
        <ReplayRows />
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

/** v1.52.0: how the screen recorder records. v1.54.0: while Replay is on, a
 *  take shares its capture and encoder, so the settings they were built with
 *  are locked until it is turned off. The separate files are the take's
 *  own and stay free. */
function ScreenRecordingRows(): JSX.Element {
  const app = useApp();
  const [advanced, setAdvanced] = createSignal(false);
  const s = () => app.screenSettings();
  const locked = () => replayOn(app.replayStatus());
  onMount(() => {
    if (!app.captureStatus()) void app.refreshCapture();
  });
  return (
    <div
      data-testid="screen-recording-settings"
      style={{
        display: "flex",
        "flex-direction": "column",
        gap: "var(--s3)",
        "border-top": "1px solid var(--line)",
        "padding-top": "var(--s4)",
      }}
    >
      <div class="eyebrow">Screen recording</div>
      <Show when={locked()}>
        <div
          data-testid="screen-recording-locked"
          style={{ "font-size": "var(--t-xs)", color: "var(--warning)", "line-height": 1.5 }}
        >
          Replay is on — turn it off to change these.
        </div>
      </Show>
      <Row label="Frame rate">
        <Segmented<"30" | "60">
          options={[
            { value: "30", label: "30 fps" },
            { value: "60", label: "60 fps" },
          ]}
          value={s().fps === 60 ? "60" : "30"}
          disabled={locked()}
          onChange={(v) => app.setScreenSettings({ fps: v === "60" ? 60 : 30 })}
        />
      </Row>
      <Row label="Quality">
        <Segmented<ScreenQuality>
          options={[
            { value: "small", label: "Smaller files" },
            { value: "balanced", label: "Balanced" },
            { value: "high", label: "Sharpest" },
          ]}
          value={s().quality}
          disabled={locked()}
          onChange={(v) => app.setScreenSettings({ quality: v })}
        />
      </Row>
      <Row label="Show the mouse pointer">
        <Toggle
          on={s().cursor}
          ariaLabel="Show the mouse pointer in screen recordings"
          disabled={locked()}
          onChange={(v) => app.setScreenSettings({ cursor: v })}
        />
      </Row>
      <Row label={`Desktop audio volume · ${s().desktopGainDb} dB`}>
        <div style={{ width: "220px" }} data-testid="desktop-audio-volume">
          <Slider
            value={s().desktopGainDb}
            min={MIN_DESKTOP_GAIN_DB}
            max={MAX_DESKTOP_GAIN_DB}
            step={1}
            ariaLabel="Desktop audio volume in decibels"
            disabled={locked()}
            onChange={(v) => app.setScreenSettings({ desktopGainDb: v })}
          />
        </div>
      </Row>
      <Row label="Also save separate audio files">
        <Toggle
          on={s().stems}
          ariaLabel="Also save your voice and the desktop audio as separate WAV files"
          onChange={(v) => app.setScreenSettings({ stems: v })}
        />
      </Row>
      <div style={{ "font-size": "var(--t-xs)", color: "var(--text-lo)", "line-height": 1.5 }}>
        Desktop audio — what your other apps play — is picked on the Record screen card. Its
        volume applies from the next recording and turns down the desktop audio in the video's
        sound only — not your voice, and not the separate desktop file, which keeps its full
        level; the default, −6 dB, puts it in the video at about half its level. Separate files put your
        voice and soundboard, and any desktop audio, in 48 kHz WAV files beside the video, for
        editing — about 35 MB a minute with desktop audio, about 12 MB without.
      </div>
      <div style={{ "font-size": "var(--t-xs)", color: "var(--text-lo)", "line-height": 1.5 }}>
        Screens bigger than 2560×1440 are scaled down to fit, keeping their shape.
        Windows' own encoders do the work — a graphics card's when Windows offers one.
        <Show when={app.captureStatus()?.h264Encoder}>
          {(name) => (
            <>
              {" "}
              First choice here: <span class="mono">{name()}</span>.
            </>
          )}
        </Show>
      </div>
      <button
        type="button"
        class="btn btn-ghost btn-sm"
        style={{ "align-self": "flex-start", padding: "0 4px", height: "auto" }}
        aria-expanded={advanced()}
        onClick={() => setAdvanced(!advanced())}
      >
        Advanced
      </button>
      <Show when={advanced()}>
        <Row label={`A/V offset · ${s().avOffsetMs > 0 ? "+" : ""}${s().avOffsetMs} ms`}>
          <div style={{ width: "220px" }}>
            <Slider
              value={s().avOffsetMs}
              min={-MAX_AV_OFFSET_MS}
              max={MAX_AV_OFFSET_MS}
              step={5}
              bipolar
              ariaLabel="A/V offset in milliseconds"
              disabled={locked()}
              onChange={(v) => app.setScreenSettings({ avOffsetMs: v })}
            />
          </div>
        </Row>
        <div style={{ "font-size": "var(--t-xs)", color: "var(--text-lo)", "line-height": 1.5 }}>
          Only if your recordings' sound lands early or late against the picture:
          positive moves the sound later. Leave it at 0 otherwise.
        </div>
      </Show>
    </div>
  );
}

/** v1.54.0: Replay's length. Turning it on — and its explainer — live on the
 *  Record screen card; this is where the length and its memory are seen at
 *  a glance. */
function ReplayRows(): JSX.Element {
  const app = useApp();
  onMount(() => {
    if (!app.captureStatus()) void app.refreshCapture();
    void app.refreshReplayMemory();
  });
  const options = createMemo(() => {
    const memory = app.replayMemory();
    return REPLAY_LENGTHS.map((secs) => {
      const len = memory?.lengths.find((l) => l.seconds === secs);
      const why = len && memory ? lengthTitle(len, memory.ceilingBytes) : null;
      return {
        value: String(secs),
        label: len ? `${secs} s · ${memoryLabel(len.capBytes)}` : `${secs} s`,
        disabled: why !== null,
        title: why ?? undefined,
      };
    });
  });
  return (
    <div
      data-testid="replay-settings"
      style={{
        display: "flex",
        "flex-direction": "column",
        gap: "var(--s3)",
        "border-top": "1px solid var(--line)",
        "padding-top": "var(--s4)",
      }}
    >
      <div class="eyebrow">Replay</div>
      <Row label="Length">
        <div data-testid="replay-settings-length">
          <Segmented<string>
            options={options()}
            value={String(app.replaySettings().seconds)}
            onChange={(v) => void app.setReplaySeconds(sanitizeReplaySeconds(Number(v)))}
          />
        </div>
      </Row>
      <div style={{ "font-size": "var(--t-xs)", color: "var(--text-lo)", "line-height": 1.5 }}>
        Save with its hotkey (Settings → Hotkeys), from the tray, or on the Record screen card.
        Replays are saved in the Recordings folder above.
      </div>
    </div>
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
  // v1.58.0: it really switches the effects now, and which way follows the
  // Mixer card (off by default, when the key switches nothing, though a bound
  // key stays registered, so other apps still don't get it). The backend acts
  // on this key itself, press and release, so it works while Divora is
  // hidden.
  {
    action: "ptm",
    label: "Push-to-modulate",
    desc: "Hold to switch your effects on or off, as the Mixer's Push to modulate card is set. Switches nothing while that card is off, but other apps still don't get the key.",
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
    // v1.56.0: never "mute" here, beside the real mute. With no monitor
    // device the toggle gates the whole output, so it says so.
    desc: "Turns hearing yourself on or off. With no separate monitor device, it turns Divora's whole output on or off, the call included.",
    icon: "monitor",
  },
  // v1.56.0. The backend acts on this key itself (a toggle on press), so it
  // works while Divora is hidden.
  {
    action: "mute",
    label: "Mute your voice",
    desc: "Mutes or unmutes your voice for the call, your headphones, WAV and screen recordings and Replay. Clips and Speak still play.",
    icon: "muted",
  },
  {
    action: "rec:screen",
    label: "Record screen",
    desc: "Start or stop recording a screen with your voice and soundboard.",
    icon: "screen",
  },
  {
    action: "rec:wav",
    label: "Record voice (WAV)",
    desc: "Start or stop recording the modulated output to a WAV.",
    icon: "record",
  },
  {
    action: "replay:save",
    label: "Save replay",
    desc: "Save what Replay has kept as an MP4, even while Divora is hidden.",
    icon: "replay",
  },
  {
    action: "replay:toggle",
    label: "Turn Replay on or off",
    desc: "Works once you've turned Replay on from the Record screen card.",
    icon: "replay",
  },
  // v1.55.0. Not "Stream overlay": e2e finds that section by the phrase.
  {
    action: "overlay:arrange",
    label: "Arrange or lock the overlay",
    desc: "Switches the overlay between Arrange and Locked while it's open.",
    icon: "unlock",
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

// ---------- Control surfaces (MIDI + Stream Deck) ----------

const MIDI_ACTIONS: { value: MidiActionKind; label: string }[] = [
  { value: "preset", label: "Summon preset" },
  { value: "tile", label: "Play soundboard tile" },
  { value: "ptm", label: "Hold push-to-modulate" },
  { value: "monitor", label: "Toggle monitor" },
  { value: "param", label: "Sweep effect param" },
  // v1.56.0: a pad toggles; a CC acts when it crosses (see the intro).
  { value: "mute", label: "Mute your voice" },
];

interface ParamOption {
  value: string; // `${effectIndex}:${key}`
  label: string;
  effectIndex: number;
  key: string;
  min: number;
  max: number;
}

/** Flatten the active (viewed) chain into "Effect · Param" options a CC
 *  can sweep. Each carries the param's range so the router can scale. */
function paramOptionsFor(chain: ChainEntry[]): ParamOption[] {
  const out: ParamOption[] = [];
  chain.forEach((entry, idx) => {
    const def = EFFECTS[entry.id];
    for (const p of def.params) {
      out.push({
        value: `${idx}:${p.key}`,
        label: `${def.name} · ${p.label}`,
        effectIndex: idx,
        key: p.key,
        min: p.min,
        max: p.max,
      });
    }
  });
  return out;
}

function triggerLabel(m: MidiMapping): string {
  if (!m.trigger) return "—";
  return `${m.trigger.kind === "note" ? "Note" : "CC"} ${m.trigger.data1}`;
}

function ControlSurfacesSection(): JSX.Element {
  const app = useApp();

  onMount(() => {
    void app.refreshMidiInputs();
  });

  const deviceOptions = createMemo<SelectOption[]>(() => [
    { value: "", label: "None — disconnected", sub: "no control surface" },
    ...app.midiInputs().map((d) => ({ value: d.id, label: d.name, sub: "MIDI input" })),
  ]);
  const paramOpts = createMemo<ParamOption[]>(() => paramOptionsFor(app.viewedChain()));
  const learning = (id: string): boolean => app.midiLearnId() === id;

  // Switching a mapping's action resets the target fields to a sensible
  // default for the new action (the store persists the result). v1.60.0:
  // picking the action it already has changes nothing: Enter to open the
  // picker and Enter again to close it picks the highlighted, current one
  // (Select's keys), and that mustn't retarget the mapping.
  const setAction = (m: MidiMapping, action: MidiActionKind): void => {
    if (action === m.action) return;
    const patch: Partial<MidiMapping> = {
      action,
      presetId: undefined,
      tileId: undefined,
      effectIndex: undefined,
      paramKey: undefined,
      min: undefined,
      max: undefined,
    };
    if (action === "preset") {
      patch.presetId = app.presets()[0]?.id;
    } else if (action === "tile") {
      patch.tileId = app.soundboardTiles()[0]?.id;
    } else if (action === "param") {
      const opt = paramOpts()[0];
      if (opt) {
        patch.effectIndex = opt.effectIndex;
        patch.paramKey = opt.key;
        patch.min = opt.min;
        patch.max = opt.max;
      }
    }
    app.updateMidiMapping(m.id, patch);
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
        <div class="eyebrow">Control surfaces</div>
        <Button
          variant="ghost"
          size="sm"
          icon="refresh"
          onClick={() => void app.refreshMidiInputs()}
          title="Re-scan MIDI input ports"
        >
          Rescan
        </Button>
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
          Bind a MIDI controller's pads and knobs to DivoraVoice. Pick your
          device, add a mapping, then click <strong>Learn</strong> and move a
          pad or knob to capture it. Mappings are saved locally and restored on
          the next launch. <strong>Mute your voice</strong> toggles with each
          press of a pad that sends a note (its mapping shows{" "}
          <strong>Note</strong>), unless the pad is set to latch: then every
          second press just ends the note and does nothing. On a knob, fader or CC button it mutes when the control goes past
          halfway and unmutes when it comes back a little below halfway (59
          or less); a control that stays on one side never changes it,
          except that its first move after launch (or after you pick the
          device or change its mapping) mutes if it is past halfway.{" "}
          {/* v1.58.0: push-to-modulate's mappings hold for real, with the
              mute's dead band (data/mute.ts muteCcStep). The card's set lets
              go of every hold and keeps each CC's side (D19), so a control
              left past halfway holds only after it crosses again. */}
          <strong>Hold push-to-modulate</strong> holds while a pad's note is
          down, or while a knob, fader or CC button is past halfway (it lets go
          at 59 or less); with two pads on it, it holds until both are up. It
          does nothing while the Mixer's Push to modulate card is off. Turning
          Push to modulate on or off on the Mixer, or picking its hold, lets go: a pad then holds again on its next press, and a control left
          past halfway once it comes back down and up again.
        </p>

        <FieldRow label="MIDI input">
          <Select
            icon="keyboard"
            value={app.selectedMidiInput() ?? ""}
            options={deviceOptions()}
            onChange={(v) => void app.selectMidiInput(v || null)}
          />
        </FieldRow>

        <div class="field-label" style={{ "margin-top": "var(--s2)" }}>
          Mappings
        </div>
        <Show
          when={app.midiMappings().length > 0}
          fallback={
            <div style={{ "font-size": "var(--t-sm)", color: "var(--text-lo)" }}>
              No mappings yet — add one below.
            </div>
          }
        >
          <div style={{ display: "flex", "flex-direction": "column", gap: "var(--s3)" }}>
            <For each={app.midiMappings()}>
              {(m) => (
                <div
                  style={{
                    display: "flex",
                    "align-items": "center",
                    gap: "var(--s2)",
                    "flex-wrap": "wrap",
                  }}
                >
                  <div style={{ "min-width": "168px", flex: "1 1 168px" }}>
                    <Select
                      icon="bolt"
                      value={m.action}
                      options={MIDI_ACTIONS.map((a) => ({ value: a.value, label: a.label }))}
                      onChange={(v) => setAction(m, v as MidiActionKind)}
                    />
                  </div>

                  <Show when={m.action === "preset"}>
                    <div style={{ "min-width": "168px", flex: "1 1 168px" }}>
                      <Select
                        icon="presets"
                        value={m.presetId ?? ""}
                        options={app.presets().map((p) => ({ value: p.id, label: p.name }))}
                        onChange={(v) => app.updateMidiMapping(m.id, { presetId: v })}
                      />
                    </div>
                  </Show>
                  <Show when={m.action === "tile"}>
                    <div style={{ "min-width": "168px", flex: "1 1 168px" }}>
                      <Select
                        icon="soundboard"
                        value={m.tileId ?? ""}
                        options={
                          app.soundboardTiles().length > 0
                            ? app.soundboardTiles().map((t) => ({ value: t.id, label: t.label }))
                            : [{ value: "", label: "No tiles — pick a folder first" }]
                        }
                        onChange={(v) => app.updateMidiMapping(m.id, { tileId: v })}
                      />
                    </div>
                  </Show>
                  <Show when={m.action === "param"}>
                    <div style={{ "min-width": "188px", flex: "1 1 188px" }}>
                      <Select
                        icon="mixer"
                        value={
                          m.effectIndex !== undefined && m.paramKey
                            ? `${m.effectIndex}:${m.paramKey}`
                            : ""
                        }
                        options={
                          paramOpts().length > 0
                            ? paramOpts().map((o) => ({ value: o.value, label: o.label }))
                            : [{ value: "", label: "Active preset has no effects" }]
                        }
                        onChange={(v) => {
                          const opt = paramOpts().find((o) => o.value === v);
                          if (opt) {
                            app.updateMidiMapping(m.id, {
                              effectIndex: opt.effectIndex,
                              paramKey: opt.key,
                              min: opt.min,
                              max: opt.max,
                            });
                          }
                        }}
                      />
                    </div>
                  </Show>

                  <div
                    style={{
                      display: "flex",
                      "align-items": "center",
                      gap: "var(--s2)",
                      "margin-left": "auto",
                    }}
                  >
                    <Show when={learning(m.id)} fallback={<Kbd>{triggerLabel(m)}</Kbd>}>
                      <Badge tone="accent">Listening…</Badge>
                    </Show>
                    <Button
                      variant={learning(m.id) ? "primary" : "secondary"}
                      size="sm"
                      onClick={() => app.setMidiLearnId(learning(m.id) ? null : m.id)}
                    >
                      {learning(m.id) ? "Cancel" : "Learn"}
                    </Button>
                    <Button
                      variant="ghost"
                      size="sm"
                      icon="trash"
                      onClick={() => app.removeMidiMapping(m.id)}
                      title="Remove mapping"
                    />
                  </div>
                </div>
              )}
            </For>
          </div>
        </Show>

        <div>
          <Button
            variant="secondary"
            size="sm"
            icon="plus"
            onClick={() =>
              app.addMidiMapping({
                action: "preset",
                presetId: app.presets()[0]?.id,
                trigger: null,
              })
            }
          >
            Add mapping
          </Button>
        </div>

        <Card>
          <div
            style={{
              padding: "var(--s4)",
              display: "flex",
              "flex-direction": "column",
              gap: "var(--s2)",
            }}
          >
            <div style={{ display: "flex", "align-items": "center", gap: "var(--s2)" }}>
              <Sigil name="bolt" size={15} />
              <div style={{ "font-weight": 600, "font-size": "var(--t-sm)" }}>
                Using a Stream Deck
              </div>
            </div>
            <p style={{ "font-size": "var(--t-xs)", color: "var(--text-mid)", margin: 0 }}>
              No plugin needed. In the Stream Deck app, add a <strong>Hotkey</strong>{" "}
              action to a button and set it to the same key you bound under{" "}
              <strong>Settings → Hotkeys</strong> (for example{" "}
              <strong>Mute your voice</strong>, panic or a recorder) or to a
              per-tile soundboard hotkey, or to one of a timer's keys in{" "}
              <strong>Settings → Hourglass</strong>. The button then fires
              DivoraVoice's existing global shortcut — even while a game or
              Discord is focused.
            </p>
          </div>
        </Card>
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

const GLYPH_ACTION_KINDS: SelectOption[] = [
  { value: "preset", label: "Preset" },
  { value: "tile", label: "Clip" },
  { value: "monitor", label: "Monitor" },
  // v1.56.0: really mutes now, and still flips it.
  { value: "mute", label: "Mute / unmute" },
  { value: "panic", label: "Panic" },
];

/** Pick what a glyph casts: an action kind + (for preset/tile) a target. */
function GlyphActionPicker(props: {
  action: GlyphAction;
  onChange: (a: GlyphAction) => void;
  presetOptions: SelectOption[];
  tileOptions: SelectOption[];
}): JSX.Element {
  // v1.60.0: the kind it already has changes nothing (Enter, Enter from
  // the keyboard picks the current one), so its preset or clip stays.
  const setKind = (k: GlyphAction["kind"]): void => {
    if (k === props.action.kind) return;
    if (k === "preset") {
      props.onChange({ kind: "preset", presetId: props.presetOptions[0]?.value ?? "" });
    } else if (k === "tile") {
      props.onChange({ kind: "tile", tileId: props.tileOptions[0]?.value ?? "" });
    } else {
      props.onChange({ kind: k } as GlyphAction);
    }
  };
  return (
    <div
      style={{
        display: "flex",
        gap: "var(--s2)",
        "align-items": "center",
        flex: 1,
        "min-width": 0,
      }}
    >
      <div style={{ width: "120px", "flex-shrink": 0 }}>
        <Select
          icon="bolt"
          value={props.action.kind}
          options={GLYPH_ACTION_KINDS}
          onChange={(v) => setKind(v as GlyphAction["kind"])}
        />
      </div>
      <Show when={props.action.kind === "preset"}>
        <div style={{ flex: 1, "min-width": 0 }}>
          <Select
            icon="presets"
            value={props.action.kind === "preset" ? props.action.presetId : ""}
            options={props.presetOptions}
            onChange={(v) => props.onChange({ kind: "preset", presetId: v })}
          />
        </div>
      </Show>
      <Show when={props.action.kind === "tile"}>
        <div style={{ flex: 1, "min-width": 0 }}>
          <Select
            icon="soundboard"
            value={props.action.kind === "tile" ? props.action.tileId : ""}
            options={props.tileOptions}
            onChange={(v) => props.onChange({ kind: "tile", tileId: v })}
          />
        </div>
      </Show>
    </div>
  );
}

function GlyphCastingSection(): JSX.Element {
  const app = useApp();
  const [recorderOpen, setRecorderOpen] = createSignal(false);
  const presetOptions = createMemo<SelectOption[]>(() =>
    app.presets().map((p) => ({ value: p.id, label: p.name, sub: p.tag })),
  );
  const tileOptions = createMemo<SelectOption[]>(() =>
    app.soundboardTiles().map((t) => ({ value: t.id, label: t.label })),
  );

  const onRecorded = (name: string, template: Point[]): void => {
    const id = `glyph-${Date.now().toString(36)}-${Math.floor(Math.random() * 1e6).toString(36)}`;
    app.addCustomGlyph({
      id,
      name,
      template,
      action: { kind: "preset", presetId: presetOptions()[0]?.value ?? "" },
    });
  };

  const chip = (inner: JSX.Element): JSX.Element => (
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
      {inner}
    </span>
  );
  const labelStyle = {
    "font-size": "var(--t-sm)",
    "font-weight": 600,
    color: "var(--text-hi)",
    overflow: "hidden",
    "text-overflow": "ellipsis",
    "white-space": "nowrap",
  } as const;
  const idCol = {
    display: "flex",
    "align-items": "center",
    gap: "var(--s3)",
    width: "164px",
    "flex-shrink": 0,
    "min-width": 0,
  } as const;

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
          Draw a glyph on the Mixer to fire its action. Bind the built-in
          shapes — or record your own.
        </div>
        <For each={GLYPH_ROWS}>
          {(row) => (
            <div style={{ display: "flex", "align-items": "center", gap: "var(--s3)" }}>
              <div style={idCol}>
                {chip(row.shape)}
                <span style={labelStyle}>{row.label}</span>
              </div>
              <GlyphActionPicker
                action={app.glyphBindings[row.id] ?? { kind: "preset", presetId: "" }}
                onChange={(a) => app.setGlyphBinding(row.id, a)}
                presetOptions={presetOptions()}
                tileOptions={tileOptions()}
              />
            </div>
          )}
        </For>

        <div class="eyebrow" style={{ "margin-top": "var(--s2)" }}>
          Custom glyphs
        </div>
        <Show
          when={app.customGlyphs().length > 0}
          fallback={
            <div style={{ "font-size": "var(--t-xs)", color: "var(--text-lo)" }}>
              None yet — record one below.
            </div>
          }
        >
          <For each={app.customGlyphs()}>
            {(g) => (
              <div style={{ display: "flex", "align-items": "center", gap: "var(--s3)" }}>
                <div style={idCol}>
                  {chip(<GlyphPreview template={g.template} size={22} />)}
                  <span style={labelStyle}>{g.name}</span>
                </div>
                <GlyphActionPicker
                  action={g.action}
                  onChange={(a) => app.setCustomGlyphAction(g.id, a)}
                  presetOptions={presetOptions()}
                  tileOptions={tileOptions()}
                />
                <IconButton
                  icon="trash"
                  tip="Delete"
                  aria-label={`Delete ${g.name}`}
                  onClick={() => app.removeCustomGlyph(g.id)}
                />
              </div>
            )}
          </For>
        </Show>
        <div>
          <Button
            variant="secondary"
            size="sm"
            icon="plus"
            onClick={() => setRecorderOpen(true)}
          >
            Record a glyph
          </Button>
        </div>
      </div>
      <GlyphRecorder
        open={recorderOpen()}
        onClose={() => setRecorderOpen(false)}
        onSave={onRecorded}
      />
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
        <Row label="Theme">
          <Segmented<TweaksState["theme"]>
            options={[
              { value: "dark", label: "Dark" },
              { value: "light", label: "Light" },
            ]}
            value={app.tweaks.theme}
            onChange={(v) => app.setTweaks("theme", v)}
          />
        </Row>
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

// ---------- Diagnostics (v1.13.0) ----------

function DiagnosticsSection(): JSX.Element {
  const app = useApp();
  const iconFor = (s: DiagStatus): SigilName =>
    s === "pass" ? "check" : s === "warn" ? "warning" : "x";
  const colorFor = (s: DiagStatus): string =>
    s === "pass"
      ? "var(--success)"
      : s === "warn"
        ? "var(--warning)"
        : "var(--danger)";
  return (
    <section>
      <div class="eyebrow" style={{ "margin-bottom": "var(--s3)" }}>
        Diagnostics
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
        <div
          style={{
            display: "flex",
            "align-items": "center",
            "justify-content": "space-between",
            gap: "var(--s4)",
          }}
        >
          <div
            style={{
              "font-size": "var(--t-sm)",
              color: "var(--text-mid)",
              "line-height": 1.5,
            }}
          >
            Check your mic, output, engine, and VB-Cable routing in one go.
          </div>
          <Button
            variant="secondary"
            icon="bolt"
            onClick={() => void app.runDiagnostics()}
            disabled={app.diagnosing()}
          >
            {app.diagnosing() ? "Testing…" : "Test my setup"}
          </Button>
        </div>
        <Show when={app.diagnostics()}>
          {(checks) => (
            <div
              style={{
                display: "flex",
                "flex-direction": "column",
                gap: "var(--s2)",
              }}
            >
              <For each={checks()}>
                {(c) => (
                  <div
                    class="card"
                    style={{
                      padding: "var(--s3)",
                      display: "flex",
                      gap: "var(--s3)",
                      "align-items": "flex-start",
                    }}
                  >
                    <span
                      style={{
                        color: colorFor(c.status),
                        "flex-shrink": 0,
                        "margin-top": "1px",
                      }}
                    >
                      <Sigil name={iconFor(c.status)} size={16} />
                    </span>
                    <div style={{ "min-width": 0 }}>
                      <div
                        style={{
                          "font-size": "var(--t-sm)",
                          "font-weight": 600,
                          color: "var(--text-hi)",
                        }}
                      >
                        {c.label}
                      </div>
                      <div
                        style={{
                          "font-size": "var(--t-xs)",
                          color: "var(--text-lo)",
                          "margin-top": "1px",
                          "line-height": 1.4,
                        }}
                      >
                        {c.detail}
                      </div>
                    </div>
                  </div>
                )}
              </For>
            </div>
          )}
        </Show>
      </div>
    </section>
  );
}

// ---------- Stream overlay (v1.16.0; charms v1.55.0) ----------

const helpStyle: JSX.CSSProperties = {
  "font-size": "var(--t-xs)",
  color: "var(--text-lo)",
  "line-height": 1.5,
};
const reasonStyle: JSX.CSSProperties = {
  "font-size": "var(--t-xs)",
  color: "var(--warning)",
  "line-height": 1.5,
};

/** v1.55.0: the overlay opens Locked; Arrange makes its charms movable. The
 *  backend says when Arrange is possible (`canArrange`, once the overlay has
 *  drawn), and the button follows its mode. */
function OverlayArrangeButton(): JSX.Element {
  const app = useApp();
  const s = () => app.overlayStatus();
  const arranging = () => s().mode === "arrange";
  // Starting, the visible text is the reason, and the name.
  const starting = () => s().open && s().phase !== "closing" && !s().canArrange;
  return (
    <Button
      size="sm"
      data-testid="overlay-arrange"
      icon={arranging() ? "lock" : "unlock"}
      aria-label={starting() ? undefined : arranging() ? "Lock the overlay" : "Arrange the overlay"}
      disabled={!s().canArrange}
      onClick={() => void app.setOverlayMode(arranging() ? "locked" : "arrange")}
    >
      {starting() ? "Starting…" : arranging() ? "Lock" : "Arrange"}
    </Button>
  );
}

/** v1.55.0: what the overlay shows, front-most first. No thumbnails:
 *  Settings never loads a picture. */
function OverlayCharmRows(): JSX.Element {
  const app = useApp();
  const scene = () => app.overlayScene();
  // v1.59.0: a timer's row is named for the timer it shows.
  const timerName = (id: string): string | undefined =>
    app.timerStore().timers.find((t) => t.id === id)?.name;
  const labels = createMemo(() => charmLabels(scene(), timerName));
  // `layoutScene` draws back to front; the list reads front to back.
  const charms = createMemo(() =>
    layoutScene(scene(), scene().designSize)
      .map((r) => r.el)
      .reverse(),
  );
  const full = () => pictureCount(scene()) >= MAX_PICTURES;
  const hasCircle = () => charms().some((e) => e.type === "spellcircle");
  return (
    <div
      data-testid="overlay-charms"
      style={{
        display: "flex",
        "flex-direction": "column",
        gap: "var(--s3)",
        "border-top": "1px solid var(--line)",
        "padding-top": "var(--s4)",
      }}
    >
      <div class="eyebrow">Charms</div>
      <For each={charms()}>
        {(el) => {
          const label = () => labels().get(el.id) ?? "Picture";
          return (
            <div
              data-charm-row={el.id}
              style={{
                display: "flex",
                "align-items": "center",
                "justify-content": "space-between",
                gap: "var(--s4)",
              }}
            >
              <span
                style={{
                  display: "inline-flex",
                  "align-items": "center",
                  gap: "var(--s2)",
                  "min-width": 0,
                  "font-size": "var(--t-sm)",
                  color: "var(--text-mid)",
                }}
              >
                <Sigil
                  name={
                    el.type === "spellcircle" ? "modulated" : el.type === "timer" ? "hourglass" : "image"
                  }
                  size={16}
                />
                <span style={{ overflow: "hidden", "text-overflow": "ellipsis", "white-space": "nowrap" }}>
                  {el.type === "image"
                    ? `${label()} · ${el.nw}×${el.nh}${el.anim === true ? " · animated" : ""}`
                    : el.type === "timer" && timerName(el.timer) !== undefined
                      ? `Timer · ${label()}`
                      : label()}
                </span>
              </span>
              <Button
                variant="ghost"
                size="sm"
                aria-label={`Remove ${label()}`}
                onClick={() => void app.removeOverlayElement(el.id)}
              >
                Remove
              </Button>
            </div>
          );
        }}
      </For>
      <div style={{ display: "flex", gap: "var(--s3)", "flex-wrap": "wrap" }}>
        <Button
          size="sm"
          icon="image"
          data-testid="overlay-add-picture"
          disabled={full()}
          onClick={() => void app.addOverlayPicture()}
        >
          Add picture…
        </Button>
        <Show when={!hasCircle()}>
          <Button size="sm" icon="modulated" onClick={() => app.addSpellCircle()}>
            Add the spell circle
          </Button>
        </Show>
      </div>
      <Show when={full()}>
        <div style={reasonStyle}>The overlay holds up to 16 pictures.</div>
      </Show>
      <div style={helpStyle}>
        If you use OBS, add images in OBS. Charms are for the window OBS captures and for
        recordings without OBS.
      </div>
    </div>
  );
}

function StreamOverlaySection(): JSX.Element {
  const app = useApp();
  // A colour background on a window the size of the monitor would paint the
  // whole screen, so the two exclude each other.
  const fitted = () => app.overlayFit() || app.overlayStatus().fit;
  const chroma = (value: OverlayBg, label: string) => ({
    value,
    label,
    disabled: fitted(),
    title: fitted() ? "Turn off Fit to monitor to use a colour background." : undefined,
  });
  return (
    <section>
      <div class="eyebrow" style={{ "margin-bottom": "var(--s3)" }}>
        Stream overlay
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
        <Row label="Show overlay window">
          <Toggle
            on={app.overlayOpen()}
            onChange={(v) => void (v ? app.openOverlay() : app.closeOverlay())}
            ariaLabel="Show stream overlay window"
          />
        </Row>
        <Row label="Background">
          <Segmented<OverlayBg>
            options={[
              { value: "transparent", label: "Transparent" },
              chroma("green", "Green"),
              chroma("magenta", "Magenta"),
            ]}
            value={app.overlayBg()}
            onChange={(v) => void app.setOverlayBg(v)}
          />
        </Row>
        <Row label="Move and resize" icon="unlock">
          <OverlayArrangeButton />
        </Row>
        <div style={helpStyle}>
          Locked, clicks go through the overlay to whatever is under it. Choose Arrange to move
          and resize what's on it and, unless it's fitted to the monitor, drag Move window at
          the overlay's top left to move the window. Press Esc or Lock when you're done.
        </div>
        <Row label="Fit to monitor" icon="expand">
          <Toggle
            on={app.overlayFit()}
            disabled={app.overlayBg() !== "transparent"}
            onChange={(v) => void app.setOverlayFit(v)}
            ariaLabel="Fit the overlay to its monitor"
          />
        </Row>
        <Show when={app.overlayBg() !== "transparent"}>
          <div data-testid="overlay-fit-reason" style={reasonStyle}>
            Fit to monitor needs the Transparent background.
          </div>
        </Show>
        <div style={helpStyle}>
          The overlay covers the whole monitor it's on, so pictures can go anywhere on it.
        </div>
        <Show when={app.overlayStatus().hideAvailable}>
          <Row label="Hide from recordings">
            <Toggle
              on={app.overlayHide()}
              onChange={(v) => void app.setOverlayHide(v)}
              ariaLabel="Hide the overlay from recordings"
            />
          </Row>
          <div style={helpStyle}>
            While Divora records any screen or Replay is on, Windows is asked to leave the
            locked overlay out. On Windows 10 and the first Windows 11 release it may still
            show. While it's hidden, apps that capture your screen or the overlay window, OBS
            and Discord included, may not show it either. With Replay on, that lasts as long as
            Replay does. It shows while you arrange.
          </div>
        </Show>
        <OverlayCharmRows />
        <div style={helpStyle}>
          A frameless, always-on-top window shows your charms — capture it in
          OBS. Use{" "}
          <strong style={{ color: "var(--text-mid)" }}>Transparent</strong> with
          a Window Capture that supports alpha, or pick{" "}
          <strong style={{ color: "var(--text-mid)" }}>Green</strong> /{" "}
          <strong style={{ color: "var(--text-mid)" }}>Magenta</strong> and add
          a Chroma Key filter. It reopens where you left it.
        </div>
      </div>
    </section>
  );
}

// ---------- Hourglass (v1.59.0) ----------
//
// Countdowns and count-ups. The backend keeps the time (src-tauri/src/
// timers.rs): this section never decides a timer's state. It shows the
// store's mirror of the backend's newest snapshot and sends what a control
// says. The cards are the stored timers (`divora.timers`) in list order,
// keyed by id, so a storage write (a run saved at a transition, a rename)
// never rebuilds a card mid-edit; each is joined by id with the snapshot.

/** Each timer's hotkeys, as their rows and names say them. v1.60.0: End
 *  now's, a countdown's only (a count-up never reaches zero). */
const TIMER_KEY_ROWS: Array<{ verb: TimerVerb; label: string; desc: string; spoken: string }> = [
  {
    verb: "toggle",
    label: "Start or pause",
    desc: "Starts it, pauses it, resumes it, or starts it again once it's done or has run out.",
    spoken: "start or pause",
  },
  { verb: "reset", label: "Reset", desc: "Back to the start, stopped.", spoken: "reset" },
  { verb: "finish", label: "End now", desc: "Ends it at once and runs what's set for zero.", spoken: "end now" },
];

const LENGTH_REASON = "Enter whole minutes, m:ss or h:mm:ss, from 1 second to 24 hours.";

// ---- v1.60.0: what a countdown does when it starts and at zero ----
//
// Rust decides what runs (timer_actions.rs `plan`); the card edits the two
// stored lists and says, ahead of time, what each action will and won't do.
// The lists are the stored arrays, shown entry by entry in stored order: an
// entry this build can't run is a row of its own with its reason and Remove
// only, and every edit goes through the store by index, so it stays where it
// is (D27). Count-ups have no lists (D32).

/** A start-or-pause press on a break pauses it (D12). */
const BREAK_PAUSE_KEY = "It pauses a break and doesn't end it; End now's key ends it.";
/** Reset's row on a countdown whose At zero puts something back (D10),
 *  naming only what its At zero has: the undo runs no other (§7.1 P9).
 *  With both, §9's sentence as it was. */
function resetUndoes(t: StoredTimer): string | null {
  const parts = restoringNames(t).map((n) => (n === "Unmute my voice" ? `${n} if At start mutes you` : n));
  return parts.length > 0
    ? `Back to the start, stopped. While it's running or paused, this also undoes its start with its At zero list: ${parts.join(", and ")}.`
    : null;
}

/** The words for each list, in its title, its help and its controls' names. */
const LIST_WORDS: Record<TimerHook, { title: string; help: string; at: string; from: string; add: string }> = {
  onStart: {
    title: "At start",
    help: "Runs when it starts from the beginning: Start, Start again, or its start-or-pause key. Pause and Resume don't run it again. A change here counts from its next start, and if Divora restarts while it's running or paused, its Mute my voice runs again.",
    at: "at the start",
    from: "from the start of",
    add: "at the start of",
  },
  onFire: {
    title: "At zero",
    help: "Runs at 00:00 and when you press End now. If it ran out while your PC slept or Divora was closed, or finished more than 30 seconds late, only Unmute my voice and Back to the preset before run. If you reset or delete it while it's running or paused, they run only to undo its start: Unmute my voice only if At start mutes you. A change here counts for the run that's going on.",
    at: "at zero",
    from: "from zero of",
    add: "at zero of",
  },
};

/** Said once under a countdown's lists when either has an action: the one
 *  general limit, with its cause, examples and a way to check (D37). */
const ACTIONS_LIMIT =
  "An action that can't run when its moment comes is skipped, not tried later: for example its preset, clip or voice is gone, Replay is off (it's off after every launch) or already saving, the engine is stopped, or Divora's window was reloading. To try a list, give it a 0:05 countdown and watch the titlebar while it runs; a clip or a line in it plays on your call too.";

const LIST_FULL = "Up to 8 actions here. Remove one to add another.";

const PAIRING_WORDS: Record<PairingWarning, string> = {
  muteNoUnmute:
    "This mutes you when it starts, and nothing at zero unmutes you, so you stay muted after it ends, is reset or is deleted. Add Unmute my voice at zero.",
  switchNoRevert:
    "This switches your preset when it starts, and nothing at zero switches back, so you stay on that preset after it ends, is reset or is deleted. Add Back to the preset before at zero to switch back.",
  revertNoSwitch: "The start doesn't switch preset, so there's nothing to go back to.",
};

/** Why a stored entry doesn't run, under its row. `startRevert`: Back to the
 *  preset before stored in At start (a hand edit): Rust reads it and never
 *  runs it there, so the card says so rather than show it as working. */
const NOT_RUN_WORDS: Record<"unreadable" | "secondLine" | "pastEight" | "startRevert", string> = {
  unreadable: "Made by a newer Divora, or can't be read here. It stays saved and doesn't run.",
  secondLine: "Only the first line in a list is said. This one stays saved and isn't said.",
  pastEight: "Only the first 8 actions in a list run. This one stays saved and doesn't run.",
  startRevert: "Back to the preset before runs only at zero. This one stays saved and doesn't run.",
};

const LINE_REASON = "Type a line of up to 200 characters. A long line takes longer to prepare.";
const LINE_HELP =
  "Said on your call even while you're muted, at full volume (Speak's Volume slider doesn't change it), and not kept in Saved clips. One line per list. A newer line from any timer replaces one still being prepared or playing, and Stop all clips and lines stops it; Speak's Stop doesn't.";
/** A new line's words (§9). */
const NEW_LINE: Record<TimerHook, string> = { onStart: "Be right back.", onFire: "I'm back." };
/** Always there: Critter Chatter needs no download. */
const FALLBACK_VOICE = "babble:bright";

/** Add action's choices, in the listbox's order; the kinds a list doesn't
 *  offer (Back to the preset before at a start) are left out per list. */
type AddChoice = "preset" | "randomPreset" | "revertPreset" | "mute" | "unmute" | "tile" | "speak" | "saveReplay" | "panic";
const ADD_CHOICES: Array<{ choice: AddChoice; kind: TimerActionKind; label: string }> = [
  { choice: "preset", kind: "preset", label: "Switch preset" },
  { choice: "randomPreset", kind: "randomPreset", label: "Random preset" },
  { choice: "revertPreset", kind: "revertPreset", label: "Back to the preset before" },
  { choice: "mute", kind: "setMuted", label: "Mute my voice" },
  { choice: "unmute", kind: "setMuted", label: "Unmute my voice" },
  { choice: "tile", kind: "tile", label: "Play a clip" },
  { choice: "speak", kind: "speak", label: "Speak a line" },
  { choice: "saveReplay", kind: "saveReplay", label: "Save a replay" },
  { choice: "panic", kind: "panic", label: "Stop all clips and lines" },
];

/** The sigil a row leads with. */
function actionSigil(a: TimerAction): SigilName {
  switch (a.kind) {
    case "preset":
      return "presets";
    case "randomPreset":
      return "shuffle";
    case "revertPreset":
      return "undo";
    case "tile":
      return "soundboard";
    case "speak":
      return "wave";
    case "saveReplay":
      return "replay";
    case "setMuted":
      return a.muted ? "muted" : "mic";
    case "panic":
      return "panic";
  }
}

/** At start mutes you (its runnable actions, as Rust reads them). */
const mutesAtStart = (t: StoredTimer): boolean =>
  actionsForSync(t.onStart).some((a) => a.kind === "setMuted" && a.muted);
/** At zero unmutes you. */
const unmutesAtZero = (t: StoredTimer): boolean =>
  actionsForSync(t.onFire).some((a) => a.kind === "setMuted" && !a.muted);
/** At zero's putting-back actions, as the expired line names them: what a
 *  run-out runs (§7.1 P5, P6). */
function restoringNames(t: StoredTimer): string[] {
  const kept = actionsForSync(t.onFire).filter(restores);
  const names: string[] = [];
  if (kept.some((a) => a.kind === "setMuted")) names.push("Unmute my voice");
  if (kept.some((a) => a.kind === "revertPreset")) names.push("Back to the preset before");
  return names;
}

/** A key row's words on this timer's card. v1.60.0: on a countdown that
 *  mutes at its start, start-or-pause says it pauses a break; on one whose
 *  At zero puts something back, Reset says it undoes the start with what
 *  that list has. */
function keyRowDesc(row: (typeof TIMER_KEY_ROWS)[number], t: StoredTimer): string {
  if (t.kind !== "countdown") return row.desc;
  if (row.verb === "toggle" && mutesAtStart(t)) return `${row.desc} ${BREAK_PAUSE_KEY}`;
  if (row.verb === "reset") return resetUndoes(t) ?? row.desc;
  return row.desc;
}

/** How long Hourglass's live region holds an edge's words before it is
 *  emptied again. */
export const HOURGLASS_SAID_MS = 2_000;

const PHASE_TONE: Record<TimerPhase, "" | "accent" | "success" | "warning"> = {
  idle: "",
  running: "accent",
  paused: "warning",
  done: "success",
  expired: "warning",
};

/** "a", "a and b", "a, b and c". */
function listed(names: readonly string[]): string {
  if (names.length <= 1) return names[0] ?? "";
  return `${names.slice(0, -1).join(", ")} and ${names[names.length - 1]}`;
}

/** A file in a folder the backend named, with the folder's own separator. */
function inFolder(dir: string, file: string): string {
  if (dir.endsWith("\\") || dir.endsWith("/")) return dir + file;
  return `${dir}${dir.includes("\\") ? "\\" : "/"}${file}`;
}

const fieldStyle: JSX.CSSProperties = {
  height: "30px",
  padding: "0 var(--s2)",
  "border-radius": "var(--r-sm)",
  border: "1px solid var(--line-strong)",
  background: "var(--surface-1)",
  color: "var(--text-hi)",
  "font-size": "var(--t-sm)",
};

function HourglassSection(): JSX.Element {
  const app = useApp();
  // By id, not by entry: a new entry object for the same id keeps its card.
  const ids = createMemo(() => app.timerStore().timers.map((t) => t.id));
  const full = () => app.timerStore().timers.length >= MAX_TIMERS;

  // The live region speaks edges only, never a time (a clock in a live
  // region would talk every second): a timer done, or one that ran out
  // while away. Nothing for the first snapshot after the section mounts.
  // A snapshot without an edge leaves the words alone: another timer's
  // tick, or the thread's emit right after a command's answer, often lands
  // within milliseconds, and words set and cleared inside one frame never
  // reach a screen reader. So an edge empties the region and puts its words
  // up on the next frame, as the toast does (the same words again are
  // spoken again); edges before that frame are joined; and the words are
  // emptied HOURGLASS_SAID_MS later.
  const [said, setSaid] = createSignal("");
  let held: TimerEdges | null = null;
  let waiting: string[] = [];
  let frame: number | undefined;
  let emptyLater: ReturnType<typeof setTimeout> | undefined;
  onCleanup(() => {
    if (frame !== undefined) cancelAnimationFrame(frame);
    clearTimeout(emptyLater);
  });
  createEffect(
    on(app.timerSnapshot, (snap) => {
      if (!snap) return;
      const next = timerAnnouncement(held, snap);
      held = next.held;
      if (next.text === "") return;
      // Each words once: a frame held back (a window not drawn) can't
      // pile up repeats.
      if (!waiting.includes(next.text)) waiting.push(next.text);
      if (frame !== undefined) return;
      clearTimeout(emptyLater);
      setSaid("");
      frame = requestAnimationFrame(() => {
        frame = undefined;
        setSaid(waiting.join(" "));
        waiting = [];
        emptyLater = setTimeout(() => setSaid(""), HOURGLASS_SAID_MS);
      });
    }),
  );

  const openFolder = async (): Promise<void> => {
    const dir = await app.timersDir();
    if (dir) await openExternal(dir);
  };

  return (
    <section data-timer-seq={app.timerSnapshot()?.seq}>
      <div class="eyebrow" style={{ "margin-bottom": "var(--s3)" }}>
        Hourglass
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
          Countdowns and count-ups that keep time while Divora is in the tray. Start, pause and
          reset them here or with their own hotkeys, and show them on your overlay or in OBS. A
          countdown can also do things when it starts and when it reaches zero: switch your
          preset, mute you for a break, play a clip, speak a line or save a replay. If your PC
          sleeps, a count-up pauses, and a countdown that runs out meanwhile says so instead of
          finishing late.
        </p>
        {/* Always mounted: a region inserted together with its text is
            commonly missed. */}
        <span class="sr-only" role="status" aria-live="polite" data-testid="hourglass-announcer">
          {said()}
        </span>
        <Show when={ids().length === 0}>
          <EmptyHint>No timers yet — add one below.</EmptyHint>
        </Show>
        <For each={ids()}>{(id) => <TimerCard id={id} />}</For>
        <div style={{ display: "flex", gap: "var(--s3)", "flex-wrap": "wrap" }}>
          <Button
            size="sm"
            icon="plus"
            disabled={full()}
            onClick={(e) => {
              blurOnPointer(e);
              void app.addTimer("countdown");
            }}
          >
            Add countdown
          </Button>
          <Button
            size="sm"
            icon="plus"
            disabled={full()}
            onClick={(e) => {
              blurOnPointer(e);
              void app.addTimer("countup");
            }}
          >
            Add count-up
          </Button>
          {/* v1.60.0: a 5:00 countdown "BRB" that mutes you at its start and
              unmutes you at zero, in one save and one sync (D26). */}
          <Button
            size="sm"
            icon="plus"
            disabled={full()}
            onClick={(e) => {
              blurOnPointer(e);
              void app.addTimer("countdown", "break");
            }}
          >
            Add break (BRB)
          </Button>
        </div>
        <Show when={full()}>
          <div style={reasonStyle}>Hourglass holds up to 16 timers. Delete one to add another.</div>
        </Show>
        <div
          data-testid="hourglass-hotkey-note"
          style={{
            ...helpStyle,
            "padding-top": "var(--s2)",
            "border-top": "1px dashed var(--line)",
          }}
        >
          Keys you set here work in every app, and the app you're in doesn't get them while Divora
          runs: set to Space, you can't type a space anywhere. Pick keys you don't otherwise use,
          such as F13–F24 from a Stream Deck, or Ctrl+Alt with a letter on a US English keyboard. A
          key works only with exactly its modifiers: set to F9, pressing it while you hold Shift
          or Ctrl does nothing.
          A key that's already set in Divora runs what it's set for and can't be picked here; clear
          it there first. Some keys can't be set, and Divora doesn't say so: the row still shows
          the key. A key Windows or another app already holds is one; Shift with a number or
          symbol key is another (Shift+1 shows as !), and so, on some keyboards other than US
          English, is Ctrl+Alt with some letters. To check one, use a countdown with nothing in At
          start or At zero: start it, press the key with Divora in the tray, then look at the timer
          here.
        </div>
        <div
          style={{
            display: "flex",
            "align-items": "center",
            "justify-content": "space-between",
            gap: "var(--s4)",
            "border-top": "1px solid var(--line)",
            "padding-top": "var(--s4)",
          }}
        >
          <div style={{ "min-width": 0 }}>
            <div class="eyebrow" style={{ "margin-bottom": "4px" }}>
              Timer files for OBS
            </div>
            <div style={helpStyle}>
              In OBS, add a <strong>Text (GDI+)</strong> source, tick{" "}
              <strong>Read from file</strong> and pick a timer's file from this folder. OBS checks
              the file about once a second, so its text can run up to about 2 seconds behind the
              overlay, and now and then it may show nothing for a second or two. While Divora is
              closed, a file keeps the last time it showed.
            </div>
          </div>
          {/* Named apart from Recordings' "Open folder" on the same page. */}
          <Button
            variant="secondary"
            size="sm"
            icon="folder"
            aria-label="Open the timer files folder"
            style={{ "flex-shrink": 0 }}
            onClick={(e) => {
              blurOnPointer(e);
              void openFolder();
            }}
          >
            Open folder
          </Button>
        </div>
      </div>
    </section>
  );
}

/** Delete asks twice: the first click turns it into "Confirm?" for 3 s
 *  (the Presets screen's rule). A delete stops a timer that may be on
 *  stream and blanks its OBS source. */
function TimerDeleteButton(props: { id: string; name: string }): JSX.Element {
  const app = useApp();
  const [confirming, setConfirming] = createSignal(false);
  let reset: ReturnType<typeof setTimeout> | undefined;
  onCleanup(() => clearTimeout(reset));
  return (
    <Button
      variant="danger"
      size="sm"
      icon="trash"
      aria-label={confirming() ? `Click again to delete ${props.name}` : `Delete ${props.name}`}
      onClick={(e) => {
        blurOnPointer(e);
        clearTimeout(reset);
        if (confirming()) {
          setConfirming(false);
          void app.deleteTimer(props.id);
          return;
        }
        setConfirming(true);
        reset = setTimeout(() => setConfirming(false), 3000);
      }}
    >
      {confirming() ? "Confirm?" : "Delete"}
    </Button>
  );
}

function TimerCard(props: { id: string }): JSX.Element {
  const app = useApp();
  // The entry as stored (the last one seen while the card is going).
  const stored = createMemo<StoredTimer | undefined>(
    (prev) => app.timerStore().timers.find((t) => t.id === props.id) ?? prev,
    undefined,
  );
  const name = () => stored()?.name ?? "";
  const role = () => app.timerRoles().get(props.id) ?? "run";
  const state = () => app.timerSnapshot()?.timers.find((t) => t.id === props.id);
  const kindLine = () =>
    stored()?.kind === "countdown" ? `Countdown · ${lengthText(stored()?.durationMs ?? 0)}` : "Count-up";

  // Rename: Enter or leaving the field keeps it, Esc cancels, and a name
  // that trims to nothing puts the old one back.
  const [renaming, setRenaming] = createSignal(false);
  const [nameDraft, setNameDraft] = createSignal("");
  const commitRename = (): void => {
    if (!renaming()) return;
    setRenaming(false);
    const next = readTimerName(nameDraft());
    if (next !== null && next !== name()) void app.renameTimer(props.id, next);
  };

  return (
    <Card
      data-timer-card={props.id}
      style={{ padding: "var(--s4)", display: "flex", "flex-direction": "column", gap: "var(--s3)" }}
    >
      <Show
        when={role() === "run"}
        fallback={
          // A timer this build doesn't run: kept as it is, with Delete only.
          <div
            style={{
              display: "flex",
              "align-items": "center",
              "justify-content": "space-between",
              gap: "var(--s4)",
            }}
          >
            <div style={{ "font-size": "var(--t-sm)", color: "var(--text-mid)", "min-width": 0 }}>
              <span style={{ "font-weight": 600, color: "var(--text-hi)" }}>{name()}</span>
              {role() === "newer"
                ? " · Made by a newer Divora. Delete it here, or use it in that version."
                : " · Over the 16-timer limit: Divora runs the first 16."}
            </div>
            <TimerDeleteButton id={props.id} name={name()} />
          </div>
        }
      >
        <div
          style={{
            display: "flex",
            "align-items": "flex-start",
            "justify-content": "space-between",
            gap: "var(--s4)",
          }}
        >
          <div style={{ "min-width": 0, flex: 1 }}>
            <Show
              when={renaming()}
              fallback={
                <div style={{ "font-size": "var(--t-sm)", "font-weight": 600, color: "var(--text-hi)" }}>
                  {name()}
                </div>
              }
            >
              <input
                type="text"
                value={nameDraft()}
                maxlength={MAX_NAME}
                aria-label={`Name of ${name()}`}
                onInput={(e) => setNameDraft(e.currentTarget.value)}
                onKeyDown={(e) => {
                  if (e.key === "Enter") {
                    e.preventDefault();
                    commitRename();
                  } else if (e.key === "Escape") {
                    e.preventDefault();
                    setRenaming(false);
                  }
                }}
                onBlur={commitRename}
                ref={(el) => {
                  // Focus and select, so typing replaces the old name.
                  queueMicrotask(() => {
                    el.focus();
                    el.select();
                  });
                }}
                style={{ ...fieldStyle, width: "100%", "max-width": "260px" }}
              />
            </Show>
            <div style={{ "font-size": "var(--t-xs)", color: "var(--text-lo)", "margin-top": "2px" }}>
              {kindLine()}
            </div>
          </div>
          <div style={{ display: "flex", "align-items": "center", gap: "var(--s3)", "flex-shrink": 0 }}>
            {/* The time is never in a live region: it changes every second. */}
            <Show when={state()}>
              {(s) => (
                <span
                  class="mono tnum"
                  style={{ "font-size": "var(--t-h2)", "font-weight": 700, color: "var(--text-hi)" }}
                >
                  {s().text}
                </span>
              )}
            </Show>
            <Badge tone={PHASE_TONE[state()?.phase ?? "idle"]}>{phaseWord(state()?.phase ?? null)}</Badge>
          </div>
        </div>
        <Show when={state()}>
          {(s) => <TimerControls id={props.id} name={name()} timer={stored()!} state={s()} />}
        </Show>
        <div style={{ display: "flex", gap: "var(--s2)", "justify-content": "flex-end" }}>
          <Show when={!renaming()}>
            <Button
              variant="ghost"
              size="sm"
              icon="pencil"
              aria-label={`Rename ${name()}`}
              onClick={() => {
                setNameDraft(name());
                setRenaming(true);
              }}
            >
              Rename
            </Button>
          </Show>
          <TimerDeleteButton id={props.id} name={name()} />
        </div>
      </Show>
    </Card>
  );
}

/** A timer the backend runs, once it has said how it is: the buttons, the
 *  Length, the hotkeys, the overlay toggle and the file. v1.60.0: on a
 *  countdown, End now, its At start and At zero lists, a third hotkey (End
 *  now's), and the lines that say what its lists did or are doing. */
function TimerControls(props: {
  id: string;
  name: string;
  timer: StoredTimer;
  state: TimerState;
}): JSX.Element {
  const app = useApp();
  const action = () => timerAction(props.state);
  // ±30 s changes a running or paused timer only; Length is for the rest.
  // v1.60.0: End now too, the backend's own rule (`TimerOp::Finish`).
  const adjustable = () => props.state.phase === "running" || props.state.phase === "paused";
  const countdown = () => props.timer.kind === "countdown";
  const keyRows = () => TIMER_KEY_ROWS.filter((r) => r.verb !== "finish" || countdown());

  // v1.60.0: what a run-out while away runs (§7.1 P5, P6): At zero's
  // putting-back actions only, named; v1.59.0's line when there are none.
  // Said of the list as it is now, never as what ran: the card keeps no
  // record of that, and the list can be edited after it ran out (or it ran
  // out before lists existed).
  const expiredLine = (): string => {
    const runs = restoringNames(props.timer);
    const head = "It reached 00:00 while your PC was asleep or Divora was closed, so it didn't finish.";
    if (!countdown() || runs.length === 0) return head;
    return `${head} When it runs out like this, only ${listed(runs)} from its At zero list ${runs.length > 1 ? "run" : "runs"}.`;
  };
  // A paused break keeps you muted (D12): the way out, said while it holds.
  const pausedBreak = (): boolean =>
    props.state.phase === "paused" &&
    countdown() &&
    mutesAtStart(props.timer) &&
    unmutesAtZero(props.timer) &&
    app.muted();
  // While its switch is the live preset, what the call hears (§7.7, D29),
  // from the facts the titlebar's status is made from, each on its own.
  const voiceLine = (): string | null => {
    if (!adjustable() || !countdown()) return null;
    const setting = app.ptmSetting();
    return timerVoiceLine(
      app.timerReturn(props.id),
      app.presetId(),
      actionNames(app).preset,
      revertsAtZero(props.timer),
      {
        muted: app.muted(),
        transforms: transforms(app.chain()),
        bypassed: app.ptmBypassed(),
        mode: setting.on ? setting.hold : null,
        key: acceleratorToKeys(app.ui.ptmKey).join("+") || "Space",
        inCall: ptmInCall(),
      },
    );
  };
  // Where push-to-modulate's key works from a call: the Mixer card's rule
  // (MixerScreen.tsx `PushToModulateCard`), so the two lines agree.
  const ptmInCall = (): "key" | "midi" | null =>
    app.hotkeyBindings.ptm !== ""
      ? "key"
      : !!app.selectedMidiInput() && app.midiMappings().some((m) => m.action === "ptm" && m.trigger !== null)
        ? "midi"
        : null;

  // Length: the draft while typing (null shows the length stored). It is
  // applied on Enter or on leaving the field, never per keystroke, which
  // would put every prefix of "1:30:00" on stream and in storage. Esc puts
  // the stored one back; an entry that can't be read stays, marked, and
  // nothing is sent.
  const [lengthDraft, setLengthDraft] = createSignal<string | null>(null);
  const [lengthBad, setLengthBad] = createSignal(false);
  const reasonId = `timer-length-reason-${createUniqueId()}`;
  const nextStartId = `timer-length-next-${createUniqueId()}`;
  const commitLength = (): void => {
    const draft = lengthDraft();
    if (draft === null) return;
    const ms = parseLength(draft);
    if (ms === null) {
      setLengthBad(true);
      return;
    }
    void app.setTimerLength(props.id, ms);
    setLengthBad(false);
    setLengthDraft(null);
  };
  const describedBy = () =>
    [lengthBad() ? reasonId : "", adjustable() ? nextStartId : ""].filter(Boolean).join(" ") ||
    undefined;

  const onOverlay = () => app.timerOnOverlay(props.id);

  const copyPath = async (): Promise<void> => {
    const dir = await app.timersDir();
    try {
      if (!dir || !navigator.clipboard?.writeText) throw new Error("no folder or no clipboard");
      await navigator.clipboard.writeText(inFolder(dir, `${props.id}.txt`));
      app.showToast("Copied the file's path.", "success");
    } catch (err) {
      console.warn("[hourglass] copying the file's path failed", err);
      app.showToast("Couldn't copy. Open the folder instead.", "error");
    }
  };

  return (
    <>
      <Show when={props.state.phase === "expired"}>
        <div style={helpStyle}>{expiredLine()}</div>
      </Show>
      <Show when={pausedBreak()}>
        <div style={helpStyle}>
          {props.name} is paused, so you stay muted until you resume it and it reaches zero, or
          press End now or Reset.
        </div>
      </Show>
      <Show when={voiceLine()}>
        {(line) => (
          <div data-testid="timer-voice-line" style={helpStyle}>
            {line()}
          </div>
        )}
      </Show>
      <div style={{ display: "flex", gap: "var(--s2)", "flex-wrap": "wrap" }}>
        <Button
          variant="primary"
          size="sm"
          icon={action().cmd === "pause" ? "pause" : "play"}
          aria-label={`${action().label} ${props.name}`}
          onClick={(e) => {
            blurOnPointer(e);
            void app.startTimer(props.id);
          }}
        >
          {action().label}
        </Button>
        <Button
          size="sm"
          icon="refresh"
          aria-label={`Reset ${props.name}`}
          onClick={(e) => {
            blurOnPointer(e);
            void app.resetTimer(props.id);
          }}
        >
          Reset
        </Button>
        {/* v1.60.0: finishes it now and runs At zero; a count-up never
            reaches zero, so it has none. */}
        <Show when={countdown()}>
          <Button
            size="sm"
            icon="endNow"
            disabled={!adjustable()}
            aria-label={`End ${props.name} now`}
            onClick={(e) => {
              blurOnPointer(e);
              void app.finishTimer(props.id);
            }}
          >
            End now
          </Button>
        </Show>
        <Button
          variant="ghost"
          size="sm"
          disabled={!adjustable()}
          aria-label={`Take 30 seconds off ${props.name}`}
          onClick={(e) => {
            blurOnPointer(e);
            void app.adjustTimer(props.id, -30_000);
          }}
        >
          −30 s
        </Button>
        <Button
          variant="ghost"
          size="sm"
          disabled={!adjustable()}
          aria-label={`Add 30 seconds to ${props.name}`}
          onClick={(e) => {
            blurOnPointer(e);
            void app.adjustTimer(props.id, 30_000);
          }}
        >
          +30 s
        </Button>
      </div>

      <Show when={props.timer.kind === "countdown"}>
        <FieldRow label="Length">
          <div style={{ display: "flex", "flex-direction": "column", gap: "var(--s1)" }}>
            <input
              type="text"
              class="mono tnum"
              value={lengthDraft() ?? lengthText(props.timer.durationMs)}
              placeholder="5:00"
              aria-label={`Length of ${props.name}`}
              aria-invalid={lengthBad() ? "true" : undefined}
              aria-describedby={describedBy()}
              onInput={(e) => {
                setLengthDraft(e.currentTarget.value);
                setLengthBad(false);
              }}
              onKeyDown={(e) => {
                if (e.key === "Enter") {
                  e.preventDefault();
                  commitLength();
                } else if (e.key === "Escape") {
                  e.preventDefault();
                  setLengthDraft(null);
                  setLengthBad(false);
                }
              }}
              onBlur={commitLength}
              style={{
                ...fieldStyle,
                width: "110px",
                "border-color": lengthBad() ? "var(--warning)" : "var(--line-strong)",
              }}
            />
            <Show when={lengthBad()}>
              <div id={reasonId} style={reasonStyle}>
                {LENGTH_REASON}
              </div>
            </Show>
            <Show when={adjustable()}>
              <div id={nextStartId} style={helpStyle}>
                A new length counts from the next start.
              </div>
            </Show>
          </div>
        </FieldRow>
        <TimerActions id={props.id} name={props.name} timer={props.timer} />
      </Show>

      <For each={keyRows()}>
        {(row) => <TimerHotkeyRow id={props.id} name={props.name} timer={props.timer} row={row} />}
      </For>

      <Row label="Show on overlay">
        <Toggle
          on={onOverlay()}
          onChange={(v) => app.setTimerOnOverlay(props.id, v)}
          ariaLabel={`Show ${props.name} on the overlay`}
          blurOnPointer
        />
      </Row>
      <Show when={onOverlay()}>
        <div style={helpStyle}>
          Move and resize it in Arrange, like a picture. It shows while the overlay is open.
        </div>
        <Show when={app.overlayStatus().hideAvailable && app.overlayHide()}>
          <div style={helpStyle}>
            Hide from recordings is on: while Divora records any screen or Replay is on, OBS,
            Discord and recordings may not show the overlay, this timer included. Its text file
            shows it in OBS either way.
          </div>
        </Show>
      </Show>

      <div
        style={{
          display: "flex",
          "align-items": "center",
          "justify-content": "space-between",
          gap: "var(--s4)",
        }}
      >
        <span style={{ "font-size": "var(--t-xs)", color: "var(--text-lo)", "min-width": 0 }}>
          OBS text file: <span class="mono">{props.id}.txt</span>
        </span>
        <Button
          variant="ghost"
          size="sm"
          icon="copy"
          aria-label={`Copy the path of ${props.name}'s file`}
          onClick={(e) => {
            blurOnPointer(e);
            void copyPath();
          }}
        >
          Copy path
        </Button>
      </div>
    </>
  );
}

/** One of a timer's hotkeys: the capture, Clear while it's set, and a
 *  line when another stored binding has the same key. A key Divora has
 *  registered never reaches the capture (Windows hands it to Divora), so a
 *  shared key comes from storage or from one that didn't register; which of
 *  the two then runs isn't promised, so the line says only that it's
 *  shared. */
function TimerHotkeyRow(props: {
  id: string;
  name: string;
  timer: StoredTimer;
  row: (typeof TIMER_KEY_ROWS)[number];
}): JSX.Element {
  const app = useApp();
  const accelerator = () => props.timer.hotkeys?.[props.row.verb] ?? "";
  const keys = () => acceleratorToKeys(accelerator());
  const spoken = () => `${props.name} ${props.row.spoken} hotkey`;
  const others = createMemo(() => {
    const key = accelerator();
    if (!key) return "";
    const names = new Set<string>();
    for (const row of HOTKEY_ROWS) {
      if (sameKey(app.hotkeyBindings[row.action] ?? "", key)) names.add(row.label);
    }
    for (const [clipId, chips] of Object.entries(app.tileHotkeys)) {
      if (!chips || chips.length === 0 || !sameKey(keysToAccelerator(chips), key)) continue;
      // Tile names come from the folder scanned now.
      names.add(app.soundboardTiles().find((t) => t.id === clipId)?.label ?? "a soundboard tile");
    }
    for (const t of app.timerStore().timers) {
      if (app.timerRoles().get(t.id) !== "run") continue;
      for (const row of TIMER_KEY_ROWS) {
        if (t.id === props.id && row.verb === props.row.verb) continue;
        if (sameKey(t.hotkeys?.[row.verb] ?? "", key)) names.add(`${t.name} ${row.spoken}`);
      }
    }
    return listed([...names]);
  });
  return (
    <div style={{ display: "flex", "flex-direction": "column", gap: "var(--s1)" }}>
      <div
        style={{
          display: "flex",
          "align-items": "center",
          gap: "var(--s4)",
          "justify-content": "space-between",
        }}
      >
        <div style={{ "min-width": 0 }}>
          <div style={{ "font-size": "var(--t-sm)", "font-weight": 600, color: "var(--text-hi)" }}>
            {props.row.label}
          </div>
          <div style={{ "font-size": "var(--t-xs)", color: "var(--text-lo)" }}>
            {keyRowDesc(props.row, props.timer)}
          </div>
        </div>
        <div style={{ display: "flex", "align-items": "center", gap: "var(--s2)", "flex-shrink": 0 }}>
          <HotkeyCapture
            value={keys()}
            onChange={(next) =>
              void app.bindTimerHotkey(props.id, props.row.verb, keysToAccelerator(next))
            }
            ariaLabel={(s) =>
              s === "set"
                ? `${spoken()}: ${keys().join(" ")}`
                : s === "unset"
                  ? `${spoken()}: not set`
                  : `${spoken()}: press a key, Esc to cancel`
            }
          />
          <Show when={accelerator()}>
            <Button
              variant="ghost"
              size="sm"
              icon="x"
              aria-label={`Clear ${spoken()}`}
              onClick={() => void app.bindTimerHotkey(props.id, props.row.verb, "")}
            >
              Clear
            </Button>
          </Show>
        </div>
      </div>
      <Show when={others()}>
        <div style={reasonStyle}>Also set for {others()}.</div>
      </Show>
    </div>
  );
}

/** v1.60.0: a countdown's At start and At zero lists, then what they say
 *  together: the pairing warnings (§7.9) and, once either list has an
 *  action, the one general limit (D37). */
function TimerActions(props: { id: string; name: string; timer: StoredTimer }): JSX.Element {
  const app = useApp();
  const warnings = () => pairingWarnings(props.timer.onStart, props.timer.onFire);
  const rows = () => [...readStoredActions(props.timer.onStart), ...readStoredActions(props.timer.onFire)];
  // A line's voice picker needs the voices, which only Speak's screen asked
  // for before. Asked once; Speak's own choice is left alone.
  createEffect(() => {
    if (rows().some((r) => r.action?.kind === "speak")) untrack(() => void app.ensureTtsVoices());
  });
  return (
    <div
      style={{
        display: "flex",
        "flex-direction": "column",
        gap: "var(--s3)",
        "padding-top": "var(--s2)",
        "border-top": "1px solid var(--line)",
      }}
    >
      <TimerActionList id={props.id} name={props.name} timer={props.timer} hook="onStart" />
      <TimerActionList id={props.id} name={props.name} timer={props.timer} hook="onFire" />
      <For each={warnings()}>{(w) => <div style={reasonStyle}>{PAIRING_WORDS[w]}</div>}</For>
      <Show when={rows().length > 0}>
        <div style={helpStyle}>{ACTIONS_LIMIT}</div>
      </Show>
    </div>
  );
}

/** How the card shows a stored entry: a row that runs, or why not. */
type ShownRow =
  | { index: number; runs: true; action: TimerAction }
  | { index: number; runs: false; action: TimerAction | null; why: keyof typeof NOT_RUN_WORDS };

function shownRow(row: ActionRow, hook: TimerHook): ShownRow {
  if (row.runs && hook === "onStart" && !ACTION_KINDS[row.action.kind].at.includes("start")) {
    return { index: row.index, runs: false, action: row.action, why: "startRevert" };
  }
  return row;
}

/** What keeps a row mounted: its kind while it runs, else why it doesn't. */
const rowKey = (r: ShownRow): string => (r.runs ? `run:${r.action.kind}` : `not:${r.why}`);

/** One list: its title and help, a row per stored entry, and Add action.
 *  Rows are kept by place (`Index`) and rebuilt only when the kind at that
 *  place changes, so a picker keeps the focus a key gave it while the store
 *  saves its change, and a line being typed survives a snapshot that saves
 *  a run. */
function TimerActionList(props: { id: string; name: string; timer: StoredTimer; hook: TimerHook }): JSX.Element {
  const words = () => LIST_WORDS[props.hook];
  const rows = createMemo(() => readStoredActions(props.timer[props.hook]).map((r) => shownRow(r, props.hook)));
  // Every stored entry counts toward the 8, one that doesn't run included.
  const full = () => canAddAction(props.timer[props.hook], "panic") === "full";
  return (
    <div data-timer-list={props.hook} style={{ display: "flex", "flex-direction": "column", gap: "var(--s2)" }}>
      <div>
        <div style={{ "font-size": "var(--t-sm)", "font-weight": 600, color: "var(--text-hi)" }}>
          {words().title}
        </div>
        <div style={helpStyle}>{words().help}</div>
      </div>
      <Show when={rows().length > 0} fallback={<div style={helpStyle}>Nothing yet.</div>}>
        <Index each={rows()}>
          {(row) => (
            <Show when={rowKey(row())} keyed>
              {(key) =>
                key.startsWith("run:") ? (
                  <TimerActionRow
                    id={props.id}
                    name={props.name}
                    hook={props.hook}
                    index={row().index}
                    action={row().action!}
                  />
                ) : (
                  <TimerActionNotRun
                    id={props.id}
                    name={props.name}
                    hook={props.hook}
                    index={row().index}
                    row={row()}
                  />
                )
              }
            </Show>
          )}
        </Index>
      </Show>
      <div>
        <AddActionMenu id={props.id} name={props.name} hook={props.hook} timer={props.timer} disabled={full()} />
      </div>
      <Show when={full()}>
        <div style={reasonStyle}>{LIST_FULL}</div>
      </Show>
    </div>
  );
}

const rowTextStyle: JSX.CSSProperties = {
  "font-size": "var(--t-sm)",
  color: "var(--text-hi)",
};
const rowStyle: JSX.CSSProperties = {
  display: "flex",
  "flex-direction": "column",
  gap: "var(--s1)",
  padding: "var(--s2) 0",
  "border-top": "1px dashed var(--line)",
};
const rowLineStyle: JSX.CSSProperties = {
  display: "flex",
  "align-items": "center",
  gap: "var(--s2)",
  "flex-wrap": "wrap",
};
const pickerBox: JSX.CSSProperties = { flex: "1 1 180px", "min-width": "160px", "max-width": "280px" };

/** A target's name as the store has it now, or null when it matches
 *  nothing: a preset in the list, a clip in the folder scanned, a voice
 *  Speak lists. */
function actionNames(app: ReturnType<typeof useApp>): ActionNames {
  return {
    preset: (id) => app.presets().find((p) => p.id === id)?.name ?? null,
    tile: (id) => app.soundboardTiles().find((t) => t.id === id)?.label ?? null,
    voice: (id) => app.ttsVoices().find((v) => v.id === id)?.name ?? null,
  };
}

/** A row that runs: its sigil and words, the target's picker (a preset, a
 *  clip, a voice and the line's text), Remove, and what it will and won't
 *  do. A target that matches nothing says so (D18, UX8). */
function TimerActionRow(props: {
  id: string;
  name: string;
  hook: TimerHook;
  index: number;
  action: TimerAction;
}): JSX.Element {
  const app = useApp();
  const place = () => LIST_WORDS[props.hook];
  const names = actionNames(app);
  const words = () => actionWords(props.action, names);
  const set = (a: TimerAction): void => void app.setTimerAction(props.id, props.hook, props.index, a);

  const preset = () => (props.action.kind === "preset" ? props.action : null);
  const tile = () => (props.action.kind === "tile" ? props.action : null);
  const line = () => (props.action.kind === "speak" ? props.action : null);
  const presetOptions = (): SelectOption[] => app.presets().map((p) => ({ value: p.id, label: p.name, sub: p.tag }));
  const tileOptions = (): SelectOption[] => app.soundboardTiles().map((t) => ({ value: t.id, label: t.label }));
  const voiceOptions = (): SelectOption[] =>
    app.ttsVoices().map((v) => ({
      value: v.id,
      label: v.name,
      sub: v.engine === "babble" ? "Critter Chatter" : v.installed ? "Kokoro" : "Kokoro · not installed",
    }));
  // Judged against the real list only: until it loads, a preset of your own
  // isn't in the bundled one and would read as gone (D16).
  const presetGone = () => {
    const p = preset();
    return p !== null && app.presetsLoaded() && names.preset(p.presetId) === null;
  };
  // The same for clips: until the folder's scan has finished, a clip in it
  // isn't in the list yet.
  const tileGone = () => {
    const t = tile();
    return t !== null && app.soundboardScanned() && names.tile(t.tileId) === null;
  };
  const voice = () => {
    const l = line();
    return l ? app.ttsVoices().find((v) => v.id === l.voiceId) : undefined;
  };

  // The line: the draft while typing (null shows the line stored), kept on
  // Enter or leaving the field, never per keystroke; Esc puts the stored one
  // back; one that can't be read stays, marked, and nothing is sent.
  const [draft, setDraft] = createSignal<string | null>(null);
  const [bad, setBad] = createSignal(false);
  const reasonId = `timer-line-reason-${createUniqueId()}`;
  const commitLine = (): void => {
    const d = draft();
    const l = line();
    if (d === null || l === null) return;
    const next = readAction({ kind: "speak", text: d, voiceId: l.voiceId });
    if (next === null) {
      setBad(true);
      return;
    }
    setBad(false);
    setDraft(null);
    if (next.kind === "speak" && next.text !== l.text) set(next);
  };

  return (
    <div data-action-row={props.index} style={rowStyle}>
      <div style={rowLineStyle}>
        <Sigil name={actionSigil(props.action)} size={16} style={{ color: "var(--indigo)", "flex-shrink": 0 }} />
        <Show when={preset()}>
          {(p) => (
            <>
              <span style={rowTextStyle}>Switch to</span>
              <div style={pickerBox}>
                <Select
                  icon="presets"
                  value={p().presetId}
                  options={presetOptions()}
                  onChange={(v) => set({ kind: "preset", presetId: v })}
                  ariaLabel={`Preset ${props.name} switches to ${place().at}`}
                  missing={app.presetsLoaded() ? "Missing preset" : "Loading presets…"}
                  blurOnPointer
                />
              </div>
            </>
          )}
        </Show>
        <Show when={tile()}>
          {(t) => (
            <>
              <span style={rowTextStyle}>Play</span>
              <div style={pickerBox}>
                <Select
                  icon="soundboard"
                  value={t().tileId}
                  options={tileOptions()}
                  onChange={(v) => set({ kind: "tile", tileId: v })}
                  ariaLabel={`Clip ${props.name} plays ${place().at}`}
                  missing={app.soundboardScanned() ? "Missing clip" : "Loading clips…"}
                  blurOnPointer
                />
              </div>
            </>
          )}
        </Show>
        <Show when={line()}>
          {(l) => (
            <>
              <span style={rowTextStyle}>Say</span>
              <input
                type="text"
                value={draft() ?? l().text}
                aria-label={`Line ${props.name} says ${place().at}`}
                aria-invalid={bad() ? "true" : undefined}
                aria-describedby={bad() ? reasonId : undefined}
                onInput={(e) => {
                  setDraft(e.currentTarget.value);
                  setBad(false);
                }}
                onKeyDown={(e) => {
                  if (e.key === "Enter") {
                    e.preventDefault();
                    commitLine();
                  } else if (e.key === "Escape") {
                    e.preventDefault();
                    setDraft(null);
                    setBad(false);
                  }
                }}
                onBlur={commitLine}
                style={{
                  ...fieldStyle,
                  flex: "1 1 200px",
                  "min-width": "160px",
                  "border-color": bad() ? "var(--warning)" : "var(--line-strong)",
                }}
              />
              <span style={rowTextStyle}>in</span>
              <div style={pickerBox}>
                <Select
                  icon="wave"
                  value={l().voiceId}
                  options={voiceOptions()}
                  onChange={(v) => set({ kind: "speak", text: l().text, voiceId: v })}
                  ariaLabel={`Voice of ${props.name}'s line ${place().at}`}
                  missing={app.ttsVoicesLoaded() ? "Voice not available" : "Loading voices…"}
                  blurOnPointer
                />
              </div>
            </>
          )}
        </Show>
        <Show when={!preset() && !tile() && !line()}>
          <span style={rowTextStyle}>{words()}</span>
        </Show>
        <div style={{ "margin-left": "auto" }}>
          <Button
            variant="ghost"
            size="sm"
            icon="x"
            aria-label={`Remove ${words()} ${place().from} ${props.name}`}
            onClick={(e) => {
              blurOnPointer(e);
              void app.removeTimerAction(props.id, props.hook, props.index);
            }}
          >
            Remove
          </Button>
        </div>
      </div>
      <Show when={presetGone()}>
        <div style={reasonStyle}>That preset is gone. Pick another, or remove this.</div>
      </Show>
      <Show when={tileGone()}>
        <div style={reasonStyle}>That clip isn't in your soundboard folder now, so nothing plays.</div>
      </Show>
      <Show when={line()}>
        <Show when={bad()}>
          <div id={reasonId} style={reasonStyle}>
            {LINE_REASON}
          </div>
        </Show>
        <Show when={voice() && !voice()!.installed}>
          <div style={reasonStyle}>
            {voice()!.name} isn't installed in this build, so nothing is said. Pick a Critter Chatter voice.
          </div>
        </Show>
        <div style={helpStyle}>{LINE_HELP}</div>
      </Show>
      <Show when={props.action.kind === "setMuted" && !props.action.muted}>
        <div style={helpStyle}>
          It unmutes you even if you muted yourself before or during the countdown, or another timer is
          still muting you.
        </div>
      </Show>
      <Show when={props.action.kind === "panic"}>
        <div style={helpStyle}>
          Runs first in its list: it stops what was playing before, and a line still being prepared.
          This list's own clip and line play after it.
        </div>
      </Show>
      <Show when={props.action.kind === "saveReplay"}>
        <Show when={!replayOn(app.replayStatus())}>
          <div style={reasonStyle}>
            Replay is off right now, so this saves nothing. Turn it on from the Record screen card; it's
            off after every launch.
          </div>
        </Show>
        <div style={helpStyle}>
          {props.hook === "onFire"
            ? "Saves the seconds up to the moment it reached zero."
            : "Saves the seconds up to the moment it started."}{" "}
          If a replay is already being saved, this one is refused with a notice.
        </div>
      </Show>
    </div>
  );
}

/** A stored entry that doesn't run: its words when it can be read, why it
 *  doesn't run, and Remove only. It stays saved and counts toward the 8
 *  (UX15). */
function TimerActionNotRun(props: {
  id: string;
  name: string;
  hook: TimerHook;
  index: number;
  row: ShownRow;
}): JSX.Element {
  const app = useApp();
  const names = actionNames(app);
  const why = () => (props.row.runs ? "unreadable" : props.row.why);
  return (
    <div data-action-row={props.index} style={rowStyle}>
      <div style={rowLineStyle}>
        <Sigil name="warning" size={16} style={{ color: "var(--warning)", "flex-shrink": 0 }} />
        <span style={{ ...rowTextStyle, color: "var(--text-mid)" }}>
          {props.row.action ? actionWords(props.row.action, names) : "An action this build can't run"}
        </span>
        <div style={{ "margin-left": "auto" }}>
          <Button
            variant="ghost"
            size="sm"
            icon="x"
            aria-label={`Remove an action that doesn't run ${LIST_WORDS[props.hook].from} ${props.name}`}
            onClick={(e) => {
              blurOnPointer(e);
              void app.removeTimerAction(props.id, props.hook, props.index);
            }}
          >
            Remove
          </Button>
        </div>
      </div>
      <div style={reasonStyle}>{NOT_RUN_WORDS[why()]}</div>
    </div>
  );
}

/** Add action: a button that opens a listbox of what this list offers
 *  (UX7), by Select's keys (D28): Enter, Space or Down opens it, Up, Down,
 *  Home and End move, Enter or Space adds, Esc closes and Tab closes and
 *  moves on, all with the focus kept on Add action. An option that can't be
 *  added now stays in the list, `aria-disabled`, its reason in its words. */
function AddActionMenu(props: {
  id: string;
  name: string;
  hook: TimerHook;
  timer: StoredTimer;
  disabled: boolean;
}): JSX.Element {
  const app = useApp();
  const [open, setOpen] = createSignal(false);
  const [active, setActive] = createSignal(0);
  let rootRef: HTMLDivElement | undefined;
  let trigger: HTMLButtonElement | undefined;
  const uid = createUniqueId();
  const listId = `add-action-${uid}`;
  const optionId = (i: number): string => `add-action-${uid}-${i}`;

  onMount(() => {
    const handler = (e: PointerEvent) => {
      if (rootRef && !rootRef.contains(e.target as Node)) setOpen(false);
    };
    document.addEventListener("pointerdown", handler);
    onCleanup(() => document.removeEventListener("pointerdown", handler));
  });

  const choices = createMemo(() => {
    const at = props.hook === "onStart" ? "start" : "zero";
    const noTiles = app.soundboardTiles().length === 0;
    // Why there's no clip to add: no folder, its scan not finished yet, or
    // none found in it (an empty folder, or one that couldn't be read).
    const noTilesWhy = !app.soundboardFolder()
      ? "Play a clip — pick a soundboard folder first"
      : !app.soundboardScanned()
        ? "Play a clip — loading your clips…"
        : "Play a clip — no clips found in your soundboard folder";
    const hasLine = canAddAction(props.timer[props.hook], "speak") === "hasLine";
    return ADD_CHOICES.filter((c) => ACTION_KINDS[c.kind].at.includes(at)).map((c) => {
      const off = (c.choice === "tile" && noTiles) || (c.choice === "speak" && hasLine);
      const label =
        c.choice === "tile" && noTiles
          ? noTilesWhy
          : c.choice === "speak" && hasLine
            ? "Speak a line — this list already has one"
            : c.label;
      return { ...c, label, off };
    });
  });

  /** The action a choice adds, with a first target the user can change. */
  const actionFor = async (choice: AddChoice): Promise<TimerAction | null> => {
    switch (choice) {
      case "preset": {
        // One other than the live preset, so it switches something.
        const p = app.presets().find((x) => x.id !== app.presetId()) ?? app.presets()[0];
        return p ? { kind: "preset", presetId: p.id } : null;
      }
      case "randomPreset":
        return { kind: "randomPreset" };
      case "revertPreset":
        return { kind: "revertPreset" };
      case "mute":
        return { kind: "setMuted", muted: true };
      case "unmute":
        return { kind: "setMuted", muted: false };
      case "tile": {
        const t = app.soundboardTiles()[0];
        return t ? { kind: "tile", tileId: t.id } : null;
      }
      case "speak": {
        // The voice is chosen once the list has answered: the first
        // installed Kokoro voice, else Critter Chatter's Bright.
        await app.ensureTtsVoices();
        const kokoro = app.ttsVoices().find((v) => v.engine !== "babble" && v.installed);
        return { kind: "speak", text: NEW_LINE[props.hook], voiceId: kokoro?.id ?? FALLBACK_VOICE };
      }
      case "saveReplay":
        return { kind: "saveReplay" };
      case "panic":
        return { kind: "panic" };
    }
  };

  const add = async (i: number): Promise<void> => {
    const c = choices()[i];
    if (!c || c.off) return;
    setOpen(false);
    const a = await actionFor(c.choice);
    if (a) await app.addTimerAction(props.id, props.hook, a);
  };

  const onKeyDown = (e: KeyboardEvent): void => {
    if (!open()) {
      if (e.key === "Enter" || e.key === " " || e.key === "ArrowDown") {
        // Handled here, so the button's own click doesn't open it again.
        e.preventDefault();
        setActive(0);
        setOpen(true);
        revealOption(optionId(active()));
      }
      return;
    }
    if (LISTBOX_MOVES.includes(e.key)) {
      e.preventDefault();
      setActive(listboxStep(e.key, active(), choices().length));
      // Into view from a key only, as Select's own (revealOption).
      revealOption(optionId(active()));
    } else if (e.key === "Enter" || e.key === " ") {
      e.preventDefault();
      void add(active());
    } else if (e.key === "Escape") {
      e.preventDefault();
      setOpen(false);
    } else if (e.key === "Tab") {
      setOpen(false);
    }
  };

  return (
    <div ref={rootRef} style={{ position: "relative", display: "inline-block" }}>
      <button
        ref={trigger}
        type="button"
        class="btn btn-secondary btn-sm"
        disabled={props.disabled}
        aria-label={`Add an action ${LIST_WORDS[props.hook].add} ${props.name}`}
        aria-haspopup="listbox"
        aria-expanded={open()}
        aria-controls={open() ? listId : undefined}
        aria-activedescendant={open() ? optionId(active()) : undefined}
        onClick={(e) => {
          blurOnPointer(e);
          if (open()) {
            setOpen(false);
          } else {
            setActive(0);
            setOpen(true);
          }
        }}
        onKeyDown={onKeyDown}
        // A Space released on the button would click it once more.
        onKeyUp={(e) => {
          if (e.key === " ") e.preventDefault();
        }}
      >
        <Sigil name="plus" size={15} />
        Add action
      </button>
      <Show when={open() && !props.disabled}>
        <div
          id={listId}
          class="dropdown"
          role="listbox"
          aria-label={`Actions to add ${LIST_WORDS[props.hook].add} ${props.name}`}
          style={{ position: "absolute", top: "calc(100% + 6px)", left: 0, "min-width": "300px", "z-index": 50 }}
        >
          <For each={choices()}>
            {(c, i) => (
              <div
                id={optionId(i())}
                class="dropdown-opt"
                role="option"
                aria-selected={false}
                aria-disabled={c.off ? "true" : undefined}
                style={{
                  ...(i() === active() ? { outline: "1px solid var(--line-glow)", "outline-offset": "-1px" } : {}),
                  ...(c.off ? { opacity: 0.55, cursor: "default" } : {}),
                  "font-size": "var(--t-sm)",
                  "font-weight": 600,
                }}
                onPointerEnter={() => setActive(i())}
                onClick={(e) => {
                  if (c.off) return;
                  // A pointer's pick leaves the focus on the page.
                  if (e.detail > 0) trigger?.blur();
                  void add(i());
                }}
              >
                {c.label}
              </div>
            )}
          </For>
        </div>
      </Show>
    </div>
  );
}

// ---------- About ----------

function AboutSection(): JSX.Element {
  const app = useApp();
  // Pull the real app version stamped into the build (release.yml writes
  // the tag's version into tauri.conf.json; dev builds report 0.0.0).
  // Lazy-import + catch so the browser preview (no Tauri bridge) and the
  // jsdom test env degrade to a blank version instead of throwing.
  const [version, setVersion] = createSignal("");
  const [logsPath, setLogsPath] = createSignal("");
  onMount(() => {
    void (async () => {
      try {
        const { getVersion } = await import("@tauri-apps/api/app");
        setVersion(`v${await getVersion()}`);
      } catch (err) {
        console.warn("[settings] app version unavailable", err);
      }
    })();
    // v1.14.0: the logs dir (for the "Open logs folder" support button).
    void app.getLogsDir().then(setLogsPath).catch(() => {
      /* not on Tauri (browser preview / E2E) — hide the button */
    });
  });
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
              DivoraVoice
              <Show when={version()}>
                {" "}
                <span class="mono" style={{ color: "var(--text-lo)" }}>{version()}</span>
              </Show>
            </div>
            <div style={{ "font-size": "var(--t-xs)", color: "var(--text-lo)" }}>
              MIT License · Tauri + SolidJS
            </div>
          </div>
          <Button
            variant="secondary"
            icon="bolt"
            onClick={app.openWhatsNew}
          >
            What&rsquo;s new
          </Button>
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

        {/* Updates (v1.12.0) — lightweight, opt-out, no telemetry. */}
        <div
          style={{
            "padding-top": "var(--s4)",
            "border-top": "1px dashed var(--line)",
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
              gap: "var(--s4)",
            }}
          >
            <div style={{ "min-width": 0 }}>
              <div
                style={{
                  "font-size": "var(--t-sm)",
                  "font-weight": 600,
                  color: "var(--text-hi)",
                }}
              >
                Updates
              </div>
              <div
                style={{
                  "font-size": "var(--t-xs)",
                  color: "var(--text-lo)",
                  "margin-top": "2px",
                  "line-height": 1.5,
                }}
              >
                Check GitHub for a newer release on launch. One-way version
                check — no account, no telemetry.
              </div>
            </div>
            <div
              style={{
                display: "flex",
                "align-items": "center",
                gap: "var(--s3)",
                "flex-shrink": 0,
              }}
            >
              <Toggle
                on={app.updateCheckEnabled()}
                onChange={(v) => app.setUpdateCheckEnabled(v)}
                ariaLabel="Check for updates on launch"
              />
              <Button
                variant="secondary"
                size="sm"
                icon="refresh"
                onClick={() => void app.checkUpdates()}
                disabled={app.updateChecking()}
              >
                {app.updateChecking() ? "Checking…" : "Check now"}
              </Button>
            </div>
          </div>
          <Show when={app.updateAvailable()}>
            {(info) => (
              <div
                class="card"
                style={{
                  padding: "var(--s4)",
                  display: "flex",
                  "align-items": "center",
                  gap: "var(--s3)",
                  "border-color": "var(--line-glow)",
                  background: "var(--accent-bg)",
                }}
              >
                <span style={{ color: "var(--indigo)", "flex-shrink": 0 }}>
                  <Sigil name="bolt" size={20} />
                </span>
                <div
                  style={{
                    flex: 1,
                    "min-width": 0,
                    "font-size": "var(--t-sm)",
                    color: "var(--text-mid)",
                  }}
                >
                  <strong style={{ color: "var(--text-hi)" }}>
                    v{info().latest}
                  </strong>{" "}
                  is available{version() ? ` — you're on ${version()}` : ""}.
                </div>
                <Button
                  variant="primary"
                  size="sm"
                  iconR="external"
                  onClick={() => void openExternal(info().url)}
                >
                  Download
                </Button>
                <Button
                  variant="ghost"
                  size="sm"
                  onClick={() => app.dismissUpdate()}
                >
                  Dismiss
                </Button>
              </div>
            )}
          </Show>
        </div>

        {/* Support — logs + report a problem (v1.14.0). */}
        <div
          style={{
            "padding-top": "var(--s4)",
            "border-top": "1px dashed var(--line)",
            display: "flex",
            "align-items": "center",
            "justify-content": "space-between",
            gap: "var(--s4)",
          }}
        >
          <div style={{ "min-width": 0 }}>
            <div
              style={{
                "font-size": "var(--t-sm)",
                "font-weight": 600,
                color: "var(--text-hi)",
              }}
            >
              Support
            </div>
            <div
              style={{
                "font-size": "var(--t-xs)",
                color: "var(--text-lo)",
                "margin-top": "2px",
                "line-height": 1.5,
              }}
            >
              Logs stay on your machine — open the folder to attach them to a
              bug report.
            </div>
          </div>
          <div style={{ display: "flex", gap: "var(--s2)", "flex-shrink": 0 }}>
            <Show when={logsPath()}>
              <Button
                variant="ghost"
                size="sm"
                icon="folder"
                onClick={() => void openExternal(logsPath())}
              >
                Open logs folder
              </Button>
            </Show>
            <Button
              variant="secondary"
              size="sm"
              icon="external"
              onClick={() => void openExternal(ISSUES_URL)}
            >
              Report a problem
            </Button>
          </div>
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
  /** v1.55.0: a sigil before the label. */
  icon?: SigilName;
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
      <span
        style={{
          display: "inline-flex",
          "align-items": "center",
          gap: "var(--s2)",
          "font-size": "var(--t-sm)",
          color: "var(--text-mid)",
        }}
      >
        <Show when={props.icon}>{(name) => <Sigil name={name()} size={16} />}</Show>
        {props.label}
      </span>
      {props.children}
    </div>
  );
}
