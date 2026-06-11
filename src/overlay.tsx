// v1.16.0: stream-overlay entry point.
//
// A standalone Solid app rendering ONLY the spell circle — it runs in a
// separate transparent Tauri window so it never bootstraps a second audio
// engine / store. It mirrors the Mixer's circle by listening for the
// `overlay:state` event the main window emits, and re-applies the theme
// data-attributes on its own root so mood / motion / mystical match.

import { render } from "solid-js/web";
import { createSignal, onMount, type JSX } from "solid-js";
import { SpellCircle } from "./components/SpellCircle";
import {
  OVERLAY_BG_COLOR,
  OVERLAY_EVENT,
  type OverlayState,
} from "./overlay/state";
import type { ChainEntry, VoiceStatus } from "./types";
import "./styles.css";

function OverlayApp(): JSX.Element {
  const [chain, setChain] = createSignal<ChainEntry[]>([]);
  const [status, setStatus] = createSignal<VoiceStatus>("clean");
  const [motion, setMotion] = createSignal(1);
  const [mystical, setMystical] = createSignal(0.7);

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
        const { listen } = await import("@tauri-apps/api/event");
        await listen<OverlayState>(OVERLAY_EVENT, (e) => {
          const s = e.payload;
          setChain(s.chain);
          setStatus(s.status);
          setMotion(s.motion);
          setMystical(s.mystical);
          applyTheme(s);
        });
      } catch {
        /* not under Tauri (browser preview) — render defaults */
      }
    })();
  });

  return (
    <div
      style={{
        width: "100vw",
        height: "100vh",
        display: "grid",
        "place-items": "center",
        overflow: "hidden",
      }}
    >
      <SpellCircle
        chain={chain()}
        status={status()}
        motion={motion()}
        mystical={mystical()}
        selected={null}
        onSelect={() => {}}
        onToggle={() => {}}
      />
    </div>
  );
}

const root = document.getElementById("root");
if (root) render(() => <OverlayApp />, root);
