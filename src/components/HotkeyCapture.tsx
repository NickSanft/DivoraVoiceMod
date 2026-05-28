// HotkeyCapture — dashed field that captures the next keychord into
// Kbd chips. Click to arm; Esc cancels. Used by Settings → Hotkeys.

import type { JSX } from "solid-js";
import { createSignal, For, onCleanup, Show } from "solid-js";
import { Kbd } from "./Kbd";

export interface HotkeyCaptureProps {
  value: string[] | null;
  onChange: (next: string[]) => void;
}

const MODIFIER_KEYS = new Set(["Control", "Alt", "Shift", "Meta"]);

function formatKey(e: KeyboardEvent): string[] {
  const parts: string[] = [];
  if (e.ctrlKey) parts.push("Ctrl");
  if (e.altKey) parts.push("Alt");
  if (e.shiftKey) parts.push("Shift");
  if (e.metaKey) parts.push("Win");
  let k = e.key;
  if (k === " ") k = "Space";
  else if (k.length === 1) k = k.toUpperCase();
  if (!MODIFIER_KEYS.has(e.key)) parts.push(k);
  return parts;
}

export function HotkeyCapture(props: HotkeyCaptureProps): JSX.Element {
  const [capturing, setCapturing] = createSignal(false);

  const onArm = () => {
    if (capturing()) {
      setCapturing(false);
      return;
    }
    setCapturing(true);
    const handler = (e: KeyboardEvent) => {
      e.preventDefault();
      if (e.key === "Escape") {
        setCapturing(false);
        window.removeEventListener("keydown", handler);
        return;
      }
      if (MODIFIER_KEYS.has(e.key)) return;
      props.onChange(formatKey(e));
      setCapturing(false);
      window.removeEventListener("keydown", handler);
    };
    window.addEventListener("keydown", handler);
    onCleanup(() => window.removeEventListener("keydown", handler));
  };

  return (
    <button
      type="button"
      class={`hotkey-capture ${capturing() ? "is-capturing" : ""}`}
      onClick={onArm}
    >
      <Show
        when={capturing()}
        fallback={
          <Show
            when={props.value && props.value.length > 0}
            fallback={<span class="hotkey-capture-hint">Click to set</span>}
          >
            <For each={props.value ?? []}>{(k) => <Kbd>{k}</Kbd>}</For>
          </Show>
        }
      >
        <span class="hotkey-capture-hint">Press a key… (Esc to cancel)</span>
      </Show>
    </button>
  );
}
