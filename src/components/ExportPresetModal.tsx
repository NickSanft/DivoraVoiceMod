// Export-preset modal. Shows the preset serialised as JSON with copy +
// "save .json" actions. Used by Presets and (future) Settings export.

import { createEffect, createSignal, Show, type JSX } from "solid-js";
import { Button } from "./Button";
import { IconButton } from "./IconButton";
import { Sigil } from "./Sigil";

export interface ExportPresetModalProps {
  open: boolean;
  presetName: string;
  json: string;
  onClose: () => void;
}

export function ExportPresetModal(props: ExportPresetModalProps): JSX.Element {
  const [copyState, setCopyState] = createSignal<"idle" | "copied" | "error">(
    "idle",
  );

  createEffect(() => {
    // Whenever the modal opens (json or open flips), reset the copy
    // feedback so it doesn't carry over.
    if (props.open) {
      setCopyState("idle");
    }
  });

  const copy = async (): Promise<void> => {
    try {
      if (
        typeof navigator !== "undefined" &&
        navigator.clipboard?.writeText
      ) {
        await navigator.clipboard.writeText(props.json);
        setCopyState("copied");
        setTimeout(() => setCopyState("idle"), 1600);
        return;
      }
      throw new Error("clipboard unavailable");
    } catch {
      setCopyState("error");
      setTimeout(() => setCopyState("idle"), 2000);
    }
  };

  const saveFile = (): void => {
    try {
      const blob = new Blob([props.json], { type: "application/json" });
      const url = URL.createObjectURL(blob);
      const a = document.createElement("a");
      a.href = url;
      a.download = `${props.presetName || "preset"}.json`;
      document.body.appendChild(a);
      a.click();
      document.body.removeChild(a);
      URL.revokeObjectURL(url);
    } catch (err) {
      console.warn("[export] save failed", err);
    }
  };

  return (
    <Show when={props.open}>
      <div
        role="dialog"
        aria-modal="true"
        aria-labelledby="export-preset-title"
        style={{
          position: "fixed",
          inset: 0,
          background: "rgba(7, 6, 13, 0.7)",
          "backdrop-filter": "blur(4px)",
          display: "grid",
          "place-items": "center",
          "z-index": 200,
        }}
        onClick={(e) => {
          if (e.currentTarget === e.target) props.onClose();
        }}
      >
        <div
          class="panel"
          style={{
            width: "640px",
            "max-width": "calc(100vw - 48px)",
            "max-height": "calc(100vh - 96px)",
            display: "flex",
            "flex-direction": "column",
            background: "var(--surface-1)",
            "box-shadow": "var(--shadow-3)",
          }}
        >
          <div
            style={{
              display: "flex",
              "align-items": "center",
              gap: "var(--s3)",
              padding: "var(--s4) var(--s5)",
              "border-bottom": "1px solid var(--line)",
            }}
          >
            <Sigil name="copy" size={18} style={{ color: "var(--indigo)" }} />
            <h3
              id="export-preset-title"
              class="display"
              style={{ "font-size": "var(--t-h3)", flex: 1, margin: 0 }}
            >
              Export {props.presetName}
            </h3>
            <IconButton icon="x" onClick={props.onClose} tip="Close" />
          </div>
          <pre
            class="mono"
            style={{
              flex: 1,
              "min-height": 0,
              "overflow": "auto",
              padding: "var(--s4) var(--s5)",
              background: "var(--surface-0)",
              "font-size": "var(--t-xs)",
              color: "var(--text-hi)",
              margin: 0,
            }}
          >
            {props.json}
          </pre>
          <div
            style={{
              display: "flex",
              "align-items": "center",
              gap: "var(--s3)",
              "justify-content": "flex-end",
              padding: "var(--s4) var(--s5)",
              "border-top": "1px solid var(--line)",
            }}
          >
            <Show when={copyState() === "copied"}>
              <span
                class="eyebrow"
                style={{ color: "var(--success)" }}
              >
                Copied
              </span>
            </Show>
            <Show when={copyState() === "error"}>
              <span class="eyebrow" style={{ color: "var(--danger)" }}>
                Copy failed
              </span>
            </Show>
            <Button variant="ghost" icon="download" onClick={saveFile}>
              Save .json
            </Button>
            <Button variant="primary" icon="copy" onClick={() => void copy()}>
              Copy
            </Button>
          </div>
        </div>
      </div>
    </Show>
  );
}
