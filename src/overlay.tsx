// v1.16.0: stream-overlay entry point.
//
// A standalone Solid app rendering ONLY the spell circle — it runs in a
// separate transparent Tauri window so it never bootstraps a second audio
// engine / store. It mirrors the Mixer's circle by listening for the
// `overlay:state` event the main window emits, and re-applies the theme
// data-attributes on its own root so mood / motion / mystical match.
//
// v1.51.3: loads the same fonts as the main window, tells the main window
// when it is listening (`overlay:ready`), and moves its window when dragged.

import "@fontsource-variable/bricolage-grotesque";
import "@fontsource-variable/space-grotesk";
import "@fontsource/space-mono/400.css";
import "@fontsource/space-mono/700.css";

import { render } from "solid-js/web";
import { createSignal, onMount, Show, type JSX } from "solid-js";
import { SpellCircle } from "./components/SpellCircle";
import { startsWindowDrag } from "./overlay/drag";
import {
  OVERLAY_BG_COLOR,
  OVERLAY_EVENT,
  OVERLAY_READY_EVENT,
  type OverlayState,
} from "./overlay/state";
import type { ChainEntry, VoiceStatus } from "./types";
import "./styles.css";

function OverlayApp(): JSX.Element {
  const [chain, setChain] = createSignal<ChainEntry[]>([]);
  const [status, setStatus] = createSignal<VoiceStatus>("clean");
  const [motion, setMotion] = createSignal(1);
  const [mystical, setMystical] = createSignal(0.7);
  // Draw nothing until the main window's state arrives. The window can be
  // shown before then (it is placed and shown as soon as it exists), and a
  // first frame of the default circle would be captured by OBS.
  const [received, setReceived] = createSignal(false);

  const applyTheme = (s: OverlayState): void => {
    const r = document.documentElement;
    const set = (k: string, v: string | null): void => {
      if (v === null) r.removeAttribute(k);
      else r.setAttribute(k, v);
    };
    set("data-theme", s.theme === "light" ? "light" : null);
    set("data-mood", s.mood === "violet" ? null : s.mood);
    set("data-accent", s.accent === "brand" ? null : s.accent);
    set("data-motion", s.motion === 0 ? "functional" : s.motion < 0.8 ? "ambient" : "rich");
    set("data-motion-user", "true");
    set("data-mystical", s.mystical <= 0.4 ? "subtle" : s.mystical <= 0.85 ? "balanced" : "rich");
    r.style.setProperty("--mystical", String(s.mystical));
    // Transparent (window alpha) or a solid chroma fill to key out in OBS.
    document.body.style.background = OVERLAY_BG_COLOR[s.bg];
  };

  onMount(() => {
    document.documentElement.style.background = "transparent";
    document.body.style.background = "transparent";
    void (async () => {
      try {
        const { emit, listen } = await import("@tauri-apps/api/event");
        await listen<OverlayState>(OVERLAY_EVENT, (e) => {
          const s = e.payload;
          setChain(s.chain);
          setStatus(s.status);
          setMotion(s.motion);
          setMystical(s.mystical);
          applyTheme(s);
          setReceived(true);
        });
        // Only now can a push reach us, so only now ask for one. Deliberately
        // not after a paint: the window may still be hidden while the main
        // window places it, and a hidden page may not run animation frames.
        await emit(OVERLAY_READY_EVENT);
      } catch {
        /* not under Tauri (browser preview) — render defaults */
        setReceived(true);
      }
    })();
  });

  // Press anywhere that isn't a button and drag to move the window.
  const onMouseDown = (e: MouseEvent): void => {
    if (!startsWindowDrag(e)) return;
    e.preventDefault();
    void (async () => {
      try {
        const { getCurrentWindow } = await import("@tauri-apps/api/window");
        await getCurrentWindow().startDragging();
      } catch {
        /* not under Tauri */
      }
    })();
  };

  return (
    <div
      onMouseDown={onMouseDown}
      style={{
        width: "100vw",
        height: "100vh",
        display: "grid",
        "place-items": "center",
        overflow: "hidden",
      }}
    >
      <Show when={received()}>
        <SpellCircle
          chain={chain()}
          status={status()}
          motion={motion()}
          mystical={mystical()}
          selected={null}
          onSelect={() => {}}
          onToggle={() => {}}
        />
      </Show>
    </div>
  );
}

const root = document.getElementById("root");
if (root) render(() => <OverlayApp />, root);
