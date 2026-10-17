// Toast — v1.54.0: one short message at a time, top-centre under the
// titlebar, for things that happen away from the screen the user is on (a
// replay saved from a hotkey, a hotkey that was refused).
//
// The live region is ALWAYS mounted and only its text changes: a region
// inserted together with its text is commonly missed by screen readers.
// Each toast first clears the text and sets it on the next animation frame,
// so the same words twice ("Still saving the last replay.") are announced
// twice.
//
// A toast that arrives while the window can't be seen — a hidden document,
// or Divora hidden to the tray — is held, not shown, and its time starts
// when the window is next shown or focused: a save made from a hotkey while
// hidden is reported when the user comes back, for its full time. Whether
// WebView2 reports a tray-hidden window as `hidden` is unverified, so the
// window's own visibility is asked too.
//
// Static: no animation, so nothing here needs a reduced-motion rule.

import { createEffect, createSignal, on, onCleanup, onMount, type JSX } from "solid-js";

export type ToastTone = "success" | "error";

export interface ToastMessage {
  /** Distinct per toast, so the same words twice are shown twice. */
  id: number;
  text: string;
  tone: ToastTone;
}

/** How long a success shows. */
export const TOAST_MS = 3000;
/** An error also gets reading time for its length: the ≈ 95-character
 *  disk-space message stays ≈ 7.8 s. */
export const TOAST_MS_PER_CHAR = 50;

export function toastDuration(m: Pick<ToastMessage, "text" | "tone">): number {
  return m.tone === "error" ? TOAST_MS + TOAST_MS_PER_CHAR * m.text.length : TOAST_MS;
}

/** Whether nobody can see the window right now. */
async function windowHidden(): Promise<boolean> {
  if (document.visibilityState !== "visible") return true;
  try {
    const { getCurrentWindow } = await import("@tauri-apps/api/window");
    return !(await getCurrentWindow().isVisible());
  } catch {
    // Browser preview or tests: the document's own state is all there is.
    return false;
  }
}

export interface ToastProps {
  /** The latest toast; a new object (a new `id`) shows again. */
  message: ToastMessage | null;
}

export function Toast(props: ToastProps): JSX.Element {
  const [text, setText] = createSignal("");
  const [tone, setTone] = createSignal<ToastTone>("success");
  // The newest message's id: an older one whose visibility check resolves
  // late gives way to it.
  let latest = 0;
  let held: ToastMessage | null = null;
  let frame: number | undefined;
  let timer: number | undefined;

  const hide = (): void => {
    if (frame !== undefined) cancelAnimationFrame(frame);
    if (timer !== undefined) window.clearTimeout(timer);
    frame = undefined;
    timer = undefined;
    setText("");
  };

  // Clear now, set on the next frame, and only then start the clock — so a
  // frame that waits for the window to be painted again waits with it.
  const show = (m: ToastMessage): void => {
    hide();
    frame = requestAnimationFrame(() => {
      frame = undefined;
      setTone(m.tone);
      setText(m.text);
      timer = window.setTimeout(() => {
        timer = undefined;
        setText("");
      }, toastDuration(m));
    });
  };

  createEffect(
    on(
      () => props.message,
      (m) => {
        if (!m || m.id === latest) return;
        latest = m.id;
        held = null;
        hide();
        void windowHidden().then((hidden) => {
          if (m.id !== latest) return;
          if (hidden) held = m;
          else show(m);
        });
      },
    ),
  );

  /** The window can be seen again: show the held toast for its full time. */
  const release = (): void => {
    const m = held;
    held = null;
    if (m && m.id === latest) show(m);
  };

  onMount(() => {
    const onVisibility = (): void => {
      if (document.visibilityState === "visible") release();
    };
    const onKey = (e: KeyboardEvent): void => {
      if (e.key === "Escape" && text()) hide();
    };
    document.addEventListener("visibilitychange", onVisibility);
    window.addEventListener("focus", release);
    window.addEventListener("keydown", onKey);

    // The OS window's focus too, which is what showing it from the tray
    // gives; the browser-level focus event above covers previews.
    let unlistenFocus: (() => void) | null = null;
    let cancelled = false;
    void (async () => {
      try {
        const { getCurrentWindow } = await import("@tauri-apps/api/window");
        const fn = await getCurrentWindow().onFocusChanged(({ payload: focused }) => {
          if (focused) release();
        });
        if (cancelled) fn();
        else unlistenFocus = fn;
      } catch {
        /* browser preview without the Tauri bridge */
      }
    })();

    onCleanup(() => {
      cancelled = true;
      document.removeEventListener("visibilitychange", onVisibility);
      window.removeEventListener("focus", release);
      window.removeEventListener("keydown", onKey);
      if (unlistenFocus) unlistenFocus();
      hide();
    });
  });

  return (
    <div
      role="status"
      aria-live="polite"
      aria-atomic="true"
      data-testid="toast"
      class={`toast${text() ? " is-shown" : ""}${tone() === "error" ? " is-error" : ""}`}
      onClick={hide}
    >
      {text()}
    </div>
  );
}
