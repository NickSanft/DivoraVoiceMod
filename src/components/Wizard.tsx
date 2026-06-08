// First-run wizard. Ported from docs/mockups/prototype/divora/screen_wizard.jsx
// — keeps the four-step ceremony (Welcome → Virtual cable → Devices →
// Ready) and the ceremonial left rail with the breathing sigil.
//
// Open state lives on the store as `wizardOpen()`. On mount we read the
// `divora.wizardSeen` localStorage key — if absent we let the wizard
// stay open (true by default). Once the user finishes or skips, we
// stash the flag so the next launch goes straight to the Mixer. The
// About → Replay setup button clears the flag and re-opens it.

import {
  createEffect,
  createMemo,
  createSignal,
  For,
  Match,
  on,
  onMount,
  Show,
  Switch,
  type JSX,
} from "solid-js";
import { Button } from "./Button";
import { DMark } from "./DMark";
import { HMeter } from "./Meters";
import { Kbd } from "./Kbd";
import { Select, type SelectOption } from "./Select";
import { Sigil, type SigilName } from "./Sigil";
import { useApp } from "../stores/app";

const STORAGE_KEY = "divora.wizardSeen";
const VB_CABLE_URL = "https://vb-audio.com/Cable/";

interface StepDef {
  key: string;
  title: string;
  icon: SigilName;
}

const BASE_STEPS: StepDef[] = [
  { key: "welcome", title: "Welcome", icon: "clean" },
  { key: "cable", title: "Virtual cable", icon: "output" },
  { key: "devices", title: "Devices", icon: "mic" },
  { key: "calibrate", title: "Calibrate", icon: "gate" },
  { key: "ready", title: "Ready", icon: "modulated" },
];

/** The wizard steps for the current state. Once the user has calibrated
 *  their mic the calibration step is dropped, so replaying setup doesn't
 *  nag. Pure + exported for unit testing. */
export function wizardSteps(calibrated: boolean): StepDef[] {
  return calibrated ? BASE_STEPS.filter((s) => s.key !== "calibrate") : BASE_STEPS;
}

interface PillarDef {
  icon: SigilName;
  title: string;
  body: string;
}

const PILLARS: PillarDef[] = [
  { icon: "shield", title: "Local-first", body: "All DSP runs on your machine." },
  { icon: "lock", title: "Private", body: "No telemetry. No tracking. Ever." },
  { icon: "bolt", title: "Free", body: "Open source under MIT." },
  { icon: "mixer", title: "Real-time", body: "Sub-20 ms processing." },
];

interface RoutingStep {
  text: string;
}

const DISCORD_ROUTING: RoutingStep[] = [
  { text: "Open Discord → User Settings → Voice & Video" },
  { text: "Under Input Device, choose CABLE Output (VB-Audio Virtual Cable)" },
  { text: "Speak — your modulated voice now travels through" },
];

/** Read the localStorage seen-flag once, defensively. */
function readSeenFlag(): boolean {
  if (typeof window === "undefined") return false;
  try {
    return window.localStorage.getItem(STORAGE_KEY) === "true";
  } catch {
    return false;
  }
}

function writeSeenFlag(seen: boolean): void {
  if (typeof window === "undefined") return;
  try {
    if (seen) {
      window.localStorage.setItem(STORAGE_KEY, "true");
    } else {
      window.localStorage.removeItem(STORAGE_KEY);
    }
  } catch {
    /* localStorage unavailable — surfaces nothing the user can act on. */
  }
}

/** Test-friendly: clear the seen flag so the wizard fires next mount. */
export function clearWizardSeenFlag(): void {
  writeSeenFlag(false);
}

export function Wizard(): JSX.Element {
  const app = useApp();

  // First mount: if the seen-flag is present, close the wizard right
  // away. The store defaults `wizardOpen` to true so first-launch users
  // see the wizard; this just suppresses repeat showings.
  onMount(() => {
    if (readSeenFlag()) {
      app.setWizardOpen(false);
    }
  });

  // Whenever the wizard opens (e.g. from About → Replay setup), kick a
  // fresh device + VB-Cable probe so the steps have current state.
  createEffect(
    on(app.wizardOpen, (open) => {
      if (!open) return;
      void app.refreshDevices();
      void app.refreshVirtualMicStatus();
    }),
  );

  const close = (markSeen: boolean): void => {
    if (markSeen) writeSeenFlag(true);
    app.setWizardOpen(false);
  };

  return (
    <Show when={app.wizardOpen()}>
      <WizardOverlay onFinish={() => close(true)} onSkip={() => close(true)} />
    </Show>
  );
}

interface OverlayProps {
  onFinish: () => void;
  onSkip: () => void;
}

function WizardOverlay(props: OverlayProps): JSX.Element {
  const app = useApp();
  const [step, setStep] = createSignal(0);
  const steps = createMemo<StepDef[]>(() => wizardSteps(app.calibrated()));
  const current = createMemo<StepDef | undefined>(() => steps()[step()]);

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

  const cableDetected = () => app.virtualMicStatus()?.detected ?? false;

  const next = (): void => {
    if (step() < steps().length - 1) {
      setStep(step() + 1);
    } else {
      props.onFinish();
    }
  };
  const back = (): void => {
    if (step() > 0) setStep(step() - 1);
  };

  const openDownload = (): void => {
    void (async () => {
      try {
        const { open } = await import("@tauri-apps/plugin-shell");
        await open(app.virtualMicStatus()?.downloadUrl ?? VB_CABLE_URL);
      } catch {
        if (typeof window !== "undefined") {
          window.open(VB_CABLE_URL, "_blank", "noopener");
        }
      }
    })();
  };

  return (
    <div
      role="dialog"
      aria-modal="true"
      aria-label="DivoraVoice first-run setup"
      style={{
        position: "absolute",
        inset: 0,
        "z-index": 100,
        display: "flex",
        background: "var(--surface-0)",
      }}
    >
      {/* Ceremonial left rail */}
      <div
        style={{
          width: "372px",
          flex: "none",
          position: "relative",
          overflow: "hidden",
          background:
            "radial-gradient(120% 90% at 30% 10%, rgba(79,70,229,0.28), transparent 60%), radial-gradient(110% 80% at 90% 100%, rgba(219,39,119,0.22), transparent 55%), var(--surface-1)",
          "border-right": "1px solid var(--line)",
          display: "flex",
          "flex-direction": "column",
          padding: "32px",
        }}
      >
        <div
          style={{
            position: "relative",
            "z-index": 2,
            display: "flex",
            "align-items": "center",
            gap: "11px",
          }}
        >
          <DMark size={30} radius={8} />
          <span class="display" style={{ "font-size": "18px" }}>
            DivoraVoice
          </span>
        </div>
        <div
          style={{
            flex: 1,
            display: "grid",
            "place-items": "center",
            position: "relative",
            "z-index": 2,
          }}
        >
          <div
            style={{
              width: "132px",
              height: "132px",
              "border-radius": "50%",
              display: "grid",
              "place-items": "center",
              background:
                "radial-gradient(circle, rgba(124,92,246,0.35), transparent 70%)",
            }}
          >
            <div
              style={{
                width: "92px",
                height: "92px",
                "border-radius": "50%",
                background: "var(--surface-0)",
                border: "1.5px solid var(--line-glow)",
                display: "grid",
                "place-items": "center",
                color: "#fff",
                "box-shadow": "0 0 40px rgba(124,92,246,0.5)",
              }}
            >
              <Sigil name={current()?.icon ?? "clean"} size={42} />
            </div>
          </div>
        </div>
        <div
          style={{
            position: "relative",
            "z-index": 2,
            display: "flex",
            "flex-direction": "column",
            gap: "7px",
          }}
        >
          <For each={steps()}>
            {(s, i) => (
              <StepRow
                index={i()}
                step={step()}
                label={s.title}
              />
            )}
          </For>
        </div>
      </div>

      {/* Right content */}
      <div
        style={{
          flex: 1,
          display: "flex",
          "flex-direction": "column",
          "min-width": 0,
        }}
      >
        <div
          style={{
            flex: 1,
            "overflow-y": "auto",
            "overflow-x": "hidden",
            padding: "44px 48px",
            display: "flex",
            "flex-direction": "column",
            "justify-content": "center",
          }}
        >
          <Switch>
            <Match when={current()?.key === "welcome"}>
              <WelcomeStep />
            </Match>
            <Match when={current()?.key === "cable"}>
              <CableStep
                detected={cableDetected()}
                cableInName={app.virtualMicStatus()?.cableInputDevice?.name ?? null}
                cableOutName={app.virtualMicStatus()?.cableOutputDevice?.name ?? null}
                onDownload={openDownload}
                onRescan={() => void app.refreshVirtualMicStatus()}
              />
            </Match>
            <Match when={current()?.key === "devices"}>
              <DevicesStep
                inputOptions={inputOptions()}
                outputOptions={outputOptions()}
              />
            </Match>
            <Match when={current()?.key === "calibrate"}>
              <CalibrateStep />
            </Match>
            <Match when={current()?.key === "ready"}>
              <ReadyStep />
            </Match>
          </Switch>
        </div>

        {/* Footer nav */}
        <div
          style={{
            flex: "none",
            padding: "16px 48px",
            "border-top": "1px solid var(--line)",
            display: "flex",
            "align-items": "center",
            gap: "12px",
          }}
        >
          <button
            type="button"
            onClick={props.onSkip}
            style={{
              "font-size": "12px",
              color: "var(--text-lo)",
              "font-weight": 600,
              background: "transparent",
              border: "none",
              cursor: "pointer",
              padding: "8px 0",
            }}
          >
            Skip setup
          </button>
          <div style={{ flex: 1 }} />
          <Show when={step() > 0}>
            <Button variant="ghost" icon="chevronL" onClick={back}>
              Back
            </Button>
          </Show>
          <Button
            variant="primary"
            iconR={step() < steps().length - 1 ? "chevronR" : "bolt"}
            onClick={next}
          >
            {step() < steps().length - 1 ? "Continue" : "Enter Divora"}
          </Button>
        </div>
      </div>
    </div>
  );
}

interface StepRowProps {
  index: number;
  step: number;
  label: string;
}

function StepRow(props: StepRowProps): JSX.Element {
  const done = () => props.index < props.step;
  const current = () => props.index === props.step;
  return (
    <div
      style={{
        display: "flex",
        "align-items": "center",
        gap: "10px",
        opacity: current() ? 1 : 0.5,
      }}
    >
      <span
        style={{
          width: "22px",
          height: "22px",
          "border-radius": "50%",
          flex: "none",
          display: "grid",
          "place-items": "center",
          "font-size": "11px",
          "font-weight": 700,
          "font-family": "var(--font-mono)",
          background: done()
            ? "var(--success)"
            : current()
              ? "var(--grad)"
              : "var(--surface-3)",
          color: props.index <= props.step ? "#fff" : "var(--text-lo)",
          border: current() ? "none" : "1px solid var(--line)",
        }}
      >
        <Show when={done()} fallback={String(props.index + 1)}>
          <Sigil name="check" size={13} />
        </Show>
      </span>
      <span
        style={{
          "font-size": "12.5px",
          "font-weight": current() ? 600 : 500,
          color: current() ? "var(--text-hi)" : "var(--text-lo)",
        }}
      >
        {props.label}
      </span>
    </div>
  );
}

// ---------- Step 0: Welcome ----------

function WelcomeStep(): JSX.Element {
  return (
    <div>
      <div class="eyebrow" style={{ "margin-bottom": "12px" }}>
        First run · no account needed
      </div>
      <h1
        class="display"
        style={{
          "font-size": "34px",
          "line-height": 1.08,
          "margin-bottom": "14px",
        }}
      >
        Your voice,
        <br />
        transmuted in real time.
      </h1>
      <p
        style={{
          "font-size": "14px",
          color: "var(--text-mid)",
          "line-height": 1.6,
          "max-width": "440px",
          "margin-bottom": "26px",
        }}
      >
        DivoraVoice bends your microphone through layered spells — pitch,
        formant, reverb and more — then routes the result into Discord,
        Zoom, OBS or any game. Three quick steps and you’re ready.
      </p>
      <div
        style={{
          display: "grid",
          "grid-template-columns": "1fr 1fr",
          gap: "12px",
          "max-width": "460px",
        }}
      >
        <For each={PILLARS}>
          {(p) => (
            <div
              class="card"
              style={{
                padding: "14px",
                display: "flex",
                gap: "11px",
                "align-items": "flex-start",
              }}
            >
              <span style={{ color: "var(--indigo)", "flex-shrink": 0 }}>
                <Sigil name={p.icon} size={19} />
              </span>
              <div>
                <div style={{ "font-size": "13px", "font-weight": 600 }}>
                  {p.title}
                </div>
                <div
                  style={{
                    "font-size": "11px",
                    color: "var(--text-lo)",
                    "margin-top": "2px",
                    "line-height": 1.4,
                  }}
                >
                  {p.body}
                </div>
              </div>
            </div>
          )}
        </For>
      </div>
    </div>
  );
}

// ---------- Step 1: Virtual cable ----------

interface CableStepProps {
  detected: boolean;
  cableInName: string | null;
  cableOutName: string | null;
  onDownload: () => void;
  onRescan: () => void;
}

function CableStep(props: CableStepProps): JSX.Element {
  return (
    <div style={{ "max-width": "480px" }}>
      <div class="eyebrow" style={{ "margin-bottom": "12px" }}>
        Step 2 · the bridge
      </div>
      <h1
        class="display"
        style={{ "font-size": "28px", "margin-bottom": "12px" }}
      >
        A virtual cable carries your voice
      </h1>
      <p
        style={{
          "font-size": "13.5px",
          color: "var(--text-mid)",
          "line-height": 1.6,
          "margin-bottom": "22px",
        }}
      >
        Apps can’t read DivoraVoice directly.{" "}
        <strong style={{ color: "var(--text-hi)" }}>VB-Cable</strong> is a
        free virtual audio device that acts as the microphone other apps
        see — DivoraVoice pours your modulated voice into it.
      </p>
      <div
        class="card"
        style={{
          padding: "16px",
          display: "flex",
          "align-items": "center",
          gap: "14px",
          "border-color": props.detected
            ? "rgba(52,217,160,0.3)"
            : "rgba(233,177,76,0.3)",
          background: props.detected
            ? "var(--success-bg)"
            : "var(--warning-bg)",
        }}
      >
        <span
          style={{
            color: props.detected ? "var(--success)" : "var(--warning)",
            "flex-shrink": 0,
          }}
        >
          <Sigil name={props.detected ? "check" : "warning"} size={26} />
        </span>
        <div style={{ flex: 1, "min-width": 0 }}>
          <div style={{ "font-size": "14px", "font-weight": 600 }}>
            {props.detected ? "VB-Cable is installed" : "VB-Cable not found"}
          </div>
          <div
            style={{
              "font-size": "11.5px",
              color: "var(--text-lo)",
              "margin-top": "2px",
            }}
          >
            <Show
              when={props.detected}
              fallback={"Free download, ~1 MB. Install, then click Re-scan."}
            >
              Detected — modulated voice will route into{" "}
              <span class="mono" style={{ color: "var(--text-mid)" }}>
                {props.cableInName ?? "CABLE Input"}
              </span>
              .
            </Show>
          </div>
        </div>
        <Show when={!props.detected}>
          <Button
            variant="primary"
            size="sm"
            iconR="external"
            onClick={props.onDownload}
          >
            Download
          </Button>
        </Show>
      </div>
      <div
        style={{
          "margin-top": "12px",
          display: "flex",
          gap: "12px",
          "align-items": "center",
        }}
      >
        <Button variant="ghost" size="sm" icon="refresh" onClick={props.onRescan}>
          Re-scan
        </Button>
        <span
          style={{
            "font-size": "11.5px",
            color: "var(--text-lo)",
          }}
        >
          Click after installing to confirm detection.
        </span>
      </div>
    </div>
  );
}

// ---------- Step 2: Devices ----------

interface DevicesStepProps {
  inputOptions: SelectOption[];
  outputOptions: SelectOption[];
}

function DevicesStep(props: DevicesStepProps): JSX.Element {
  const app = useApp();
  return (
    <div style={{ "max-width": "480px" }}>
      <div class="eyebrow" style={{ "margin-bottom": "12px" }}>
        Step 3 · channels
      </div>
      <h1
        class="display"
        style={{ "font-size": "28px", "margin-bottom": "20px" }}
      >
        Pick your devices
      </h1>
      <div class="field-label">Microphone in</div>
      <Show
        when={props.inputOptions.length > 0}
        fallback={<EmptyDevices kind="input" />}
      >
        <Select
          icon="mic"
          value={app.selectedInput() ?? ""}
          options={props.inputOptions}
          onChange={(v) => app.setSelectedInput(v)}
        />
      </Show>
      <div
        style={{
          margin: "14px 0 4px",
          display: "flex",
          "align-items": "center",
          gap: "10px",
        }}
      >
        <span class="eyebrow" style={{ flex: "none" }}>
          Hearing you
        </span>
        <div style={{ flex: 1 }}>
          <HMeter
            level={app.inputLevels().rms}
            peak={app.inputLevels().peak}
          />
        </div>
      </div>
      <div class="field-label" style={{ "margin-top": "18px" }}>
        Modulated out
      </div>
      <Show
        when={props.outputOptions.length > 0}
        fallback={<EmptyDevices kind="output" />}
      >
        <Select
          icon="output"
          value={app.selectedOutput() ?? ""}
          options={props.outputOptions}
          onChange={(v) => app.setSelectedOutput(v)}
        />
      </Show>
      <div
        style={{
          "margin-top": "12px",
          "font-size": "11.5px",
          color: "var(--text-mid)",
          display: "flex",
          gap: "8px",
          "align-items": "center",
        }}
      >
        <Sigil
          name="info"
          size={15}
          style={{ color: "var(--info)", "flex-shrink": 0 }}
        />
        Send to <Kbd>CABLE Input</Kbd> so other apps can pick it up.
      </div>
    </div>
  );
}

function EmptyDevices(props: { kind: "input" | "output" }): JSX.Element {
  return (
    <div
      style={{
        padding: "var(--s3) var(--s4)",
        border: "1px dashed var(--line)",
        "border-radius": "var(--r-md)",
        "font-size": "var(--t-xs)",
        color: "var(--text-lo)",
      }}
    >
      No {props.kind} devices detected yet. They’ll appear once the engine
      enumerates them.
    </div>
  );
}

// ---------- Step: Calibrate your mic ----------

function CalibrateStep(): JSX.Element {
  const app = useApp();
  const result = () => app.calibrationResult();
  return (
    <div style={{ "max-width": "480px" }}>
      <div class="eyebrow" style={{ "margin-bottom": "12px" }}>
        Step 4 · a clean signal
      </div>
      <h1 class="display" style={{ "font-size": "28px", "margin-bottom": "12px" }}>
        Calibrate your mic
      </h1>
      <p
        style={{
          "font-size": "13.5px",
          color: "var(--text-mid)",
          "line-height": 1.6,
          "margin-bottom": "20px",
        }}
      >
        Stay quiet for a couple of seconds and DivoraVoice measures your
        room’s noise floor, then sets the noise gate so it opens for your
        voice but not the hum of the room.
      </p>
      <div
        style={{
          margin: "0 0 14px",
          display: "flex",
          "align-items": "center",
          gap: "10px",
        }}
      >
        <span class="eyebrow" style={{ flex: "none" }}>
          Input
        </span>
        <div style={{ flex: 1 }}>
          <HMeter level={app.inputLevels().rms} peak={app.inputLevels().peak} />
        </div>
      </div>
      <Button
        variant="primary"
        icon="gate"
        onClick={() => void app.runCalibration()}
        disabled={app.calibrating()}
      >
        {app.calibrating() ? "Listening… stay quiet" : "Calibrate (2s)"}
      </Button>
      <Show when={result()}>
        {(r) => (
          <div
            class="card"
            style={{
              "margin-top": "14px",
              padding: "14px",
              display: "flex",
              "align-items": "center",
              gap: "12px",
              "border-color": "rgba(52,217,160,0.3)",
              background: "var(--success-bg)",
            }}
          >
            <span style={{ color: "var(--success)", "flex-shrink": 0 }}>
              <Sigil name="check" size={22} />
            </span>
            <div
              style={{
                "font-size": "12.5px",
                color: "var(--text-mid)",
                "line-height": 1.5,
              }}
            >
              Noise floor{" "}
              <strong style={{ color: "var(--text-hi)" }}>
                {r().noiseFloorDb.toFixed(0)} dB
              </strong>{" "}
              → gate set to{" "}
              <strong style={{ color: "var(--text-hi)" }}>
                {r().gateThreshDb} dB
              </strong>
              <Show when={r().denoiserMix > 0}>
                {`, denoiser ${r().denoiserMix}%`}
              </Show>
              . You can fine-tune it in Settings.
            </div>
          </div>
        )}
      </Show>
      <p
        style={{
          "margin-top": "14px",
          "font-size": "11.5px",
          color: "var(--text-lo)",
        }}
      >
        Optional — skip this and adjust the gate later in Settings → Audio
        devices.
      </p>
    </div>
  );
}

// ---------- Step 3: Ready ----------

function ReadyStep(): JSX.Element {
  return (
    <div style={{ "max-width": "480px" }}>
      <div
        style={{
          width: "56px",
          height: "56px",
          "border-radius": "50%",
          display: "grid",
          "place-items": "center",
          "margin-bottom": "18px",
          background: "var(--success-bg)",
          color: "var(--success)",
          border: "1px solid rgba(52,217,160,0.3)",
        }}
      >
        <Sigil name="check" size={30} />
      </div>
      <h1
        class="display"
        style={{ "font-size": "30px", "margin-bottom": "12px" }}
      >
        You’re ready.
      </h1>
      <p
        style={{
          "font-size": "13.5px",
          color: "var(--text-mid)",
          "line-height": 1.6,
          "margin-bottom": "22px",
        }}
      >
        Pick a spell from the Mixer and hold <Kbd>Space</Kbd> to modulate.
        One last thing — point your chat app at DivoraVoice:
      </p>
      <div
        class="card"
        style={{ padding: "18px", background: "var(--surface-1)" }}
      >
        <div
          style={{
            display: "flex",
            "align-items": "center",
            gap: "8px",
            "margin-bottom": "14px",
          }}
        >
          <Sigil name="output" size={17} style={{ color: "var(--indigo)" }} />
          <span style={{ "font-size": "13px", "font-weight": 600 }}>
            Route into Discord
          </span>
        </div>
        <For each={DISCORD_ROUTING}>
          {(s, i) => (
            <div
              style={{
                display: "flex",
                gap: "11px",
                "align-items": "flex-start",
                "margin-bottom":
                  i() < DISCORD_ROUTING.length - 1 ? "11px" : 0,
              }}
            >
              <span
                style={{
                  width: "20px",
                  height: "20px",
                  "border-radius": "50%",
                  flex: "none",
                  display: "grid",
                  "place-items": "center",
                  "font-size": "11px",
                  "font-weight": 700,
                  "font-family": "var(--font-mono)",
                  background: "var(--accent-bg)",
                  color: "var(--indigo)",
                  border: "1px solid var(--line-glow)",
                }}
              >
                {i() + 1}
              </span>
              <span
                style={{
                  "font-size": "12.5px",
                  color: "var(--text-mid)",
                  "line-height": 1.45,
                  "padding-top": "1px",
                }}
              >
                {s.text}
              </span>
            </div>
          )}
        </For>
      </div>
    </div>
  );
}
