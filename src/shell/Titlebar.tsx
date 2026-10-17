// Titlebar — custom frameless titlebar with DMark + wordmark + live
// status pill + persistent "LOCAL · NO ACCOUNT" affirmation + window
// controls. The whole bar is an OS drag region via the
// `data-tauri-drag-region` attribute (which Tauri 2 wires up natively
// for frameless windows).

import { Show, type JSX } from "solid-js";
import { DMark } from "../components/DMark";
import { Sigil } from "../components/Sigil";
import { chipLabel, chipTitle, replayOn } from "../data/replay";
import { useApp } from "../stores/app";
import { statusMeta } from "./statusMeta";

async function tauriWindow() {
  try {
    const mod = await import("@tauri-apps/api/window");
    return mod.getCurrentWindow();
  } catch {
    return null;
  }
}

const onMinimize = async () => (await tauriWindow())?.minimize();
const onMaximize = async () => {
  const w = await tauriWindow();
  if (!w) return;
  const max = await w.isMaximized();
  if (max) await w.unmaximize();
  else await w.maximize();
};
const onClose = async () => (await tauriWindow())?.close();

export function Titlebar(): JSX.Element {
  const app = useApp();
  const meta = () => statusMeta(app.status());
  // The kept screen's number, once the screens are known.
  const replayScreen = (): number | null =>
    app.captureMonitors().find((m) => m.id === app.replayStatus().monitorId)?.number ?? null;

  return (
    <div
      data-tauri-drag-region
      style={{
        height: "40px",
        flex: "none",
        display: "flex",
        "align-items": "center",
        gap: "12px",
        padding: "0 14px",
        background: "var(--surface-1)",
        "border-bottom": "1px solid var(--line)",
        position: "relative",
        "z-index": 30,
      }}
    >
      <DMark size={20} radius={6} />
      <span class="titlebar-wordmark">DivoraVoice</span>
      <span class="titlebar-divider" />
      <span class="titlebar-status">
        <span
          class="titlebar-status-dot"
          style={{
            background: meta().color,
            "box-shadow": `0 0 7px ${meta().color}`,
            animation: app.status() === "modulated" ? "shimmer 1.4s infinite" : "none",
          }}
        />
        {meta().label}
      </span>
      <div style={{ flex: 1 }} />
      {/* v1.54.0: Replay is on. One of its indicators, with the tray icon and
          Windows' border; the title says what is kept, calls included. No
          animation in any state. */}
      <Show when={replayOn(app.replayStatus())}>
        <span
          class={`titlebar-replay${
            app.replayStatus().state === "arming"
              ? " is-arming"
              : app.replayStatus().state === "held"
                ? " is-held"
                : ""
          }`}
          data-testid="titlebar-replay"
          title={chipTitle(app.replayStatus(), replayScreen())}
        >
          <Sigil name="replay" size={13} /> {chipLabel(app.replayStatus())}
        </span>
      </Show>
      <span class="titlebar-local">
        <Sigil name="lock" size={13} /> LOCAL · NO ACCOUNT
      </span>
      <div style={{ display: "flex", gap: "4px", "margin-left": "6px" }}>
        <button
          type="button"
          class="winbtn"
          onClick={onMinimize}
          aria-label="Minimize"
          data-tauri-drag-region={false}
        >
          <div
            style={{
              width: "9px",
              height: "1.5px",
              background: "currentColor",
            }}
          />
        </button>
        <button
          type="button"
          class="winbtn"
          onClick={onMaximize}
          aria-label="Maximize"
          data-tauri-drag-region={false}
        >
          <div
            style={{
              width: "9px",
              height: "9px",
              border: "1.5px solid currentColor",
              "border-radius": "2px",
            }}
          />
        </button>
        <button
          type="button"
          class="winbtn winbtn-close"
          onClick={onClose}
          aria-label="Close"
          data-tauri-drag-region={false}
        >
          <Sigil name="x" size={13} />
        </button>
      </div>
    </div>
  );
}
