// v1.16.0: stream-overlay entry point.
//
// A standalone Solid app in a separate transparent Tauri window, so it never
// bootstraps a second audio engine / store. It mirrors the Mixer's circle by
// listening for the `overlay:state` event the main window emits, and
// re-applies the theme data-attributes on its own root so mood / motion /
// mystical match.
//
// v1.51.3: loads the same fonts as the main window, tells the main window
// when it is listening (`overlay:ready`), and moves its window when dragged.
//
// v1.55.0 (Charms): draws the scene main pushes on `overlay:scene` through
// Stage, which also arranges it (and now moves the window only in Arrange);
// each edit goes back on `overlay:edit`. The backend shows the window once
// this page says it is listening (the ready handshake) and allows Arrange
// once it has drawn since being shown (painted). Both are commands, never
// events the backend listens for: tauri can park a Rust listener's event on
// lock contention.

import "@fontsource-variable/bricolage-grotesque";
import "@fontsource-variable/space-grotesk";
import "@fontsource/space-mono/400.css";
import "@fontsource/space-mono/700.css";

import { render } from "solid-js/web";
import { createMemo, createSignal, onCleanup, onMount, type JSX } from "solid-js";
import { emptyMirror, HOLD_MS, proposeEdit, receiveScene, renderedScene } from "./overlay/mirror";
import { defaultScene, type SceneOp, type Size } from "./overlay/scene";
import { Stage, twoFrames } from "./overlay/Stage";
import {
  OVERLAY_EDIT_EVENT,
  OVERLAY_EVENT,
  OVERLAY_READY_EVENT,
  OVERLAY_SCENE_EVENT,
  OVERLAY_STATUS_EVENT,
  type OverlayState,
  type SceneEdit,
  type ScenePush,
} from "./overlay/state";
import {
  closedStatus,
  NO_READY_GLOBAL,
  readOverlayStatus,
  sendHandshake,
  type OverlayStatus,
} from "./overlay/status";
import "./styles.css";

/** The browser preview (not under Tauri) draws the default circle. */
const PREVIEW_STATE: OverlayState = {
  chain: [],
  status: "clean",
  motion: 1,
  mystical: 0.7,
  mood: "violet",
  accent: "brand",
  theme: "dark",
  bg: "transparent",
};

/** This window's number, from `overlay.html?instance=<n>`; 0 without one. */
export function pageInstance(search: string): number {
  const n = Number(new URLSearchParams(search).get("instance"));
  return Number.isInteger(n) && n > 0 ? n : 0;
}

function applyTheme(s: OverlayState): void {
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
}

export function OverlayApp(): JSX.Element {
  const instance = pageInstance(location.search);
  // Nothing draws until main's state (the circle) and scene (the pictures)
  // arrive: a first frame of defaults would be captured by OBS.
  const [state, setState] = createSignal<OverlayState | null>(null);
  const [mirror, setMirror] = createSignal(emptyMirror());
  const [status, setStatus] = createSignal<OverlayStatus>(closedStatus());
  const [viewport, setViewport] = createSignal<Size>({ w: window.innerWidth, h: window.innerHeight });
  // Moved on when a held scene's time runs out.
  const [now, setNow] = createSignal(performance.now());
  const scene = createMemo(() => renderedScene(mirror(), viewport(), now()));

  let painted = false;
  let send: ((event: string, payload: unknown) => Promise<void>) | null = null;
  // One chain, so edits reach main in the order they were made.
  let edits: Promise<void> = Promise.resolve();
  const emitEdit = (edit: SceneEdit): void => {
    const emit = send;
    if (!emit) return;
    edits = edits
      .then(() => emit(OVERLAY_EDIT_EVENT, edit))
      .catch(() => {
        /* lost: main's next session drops it */
      });
  };

  const onEdit = (op: SceneOp): void => {
    const { mirror: next, edit } = proposeEdit(mirror(), op, instance);
    setMirror(next);
    emitEdit(edit);
  };

  const onScene = (push: ScenePush): void => {
    const t = performance.now();
    setMirror((m) => receiveScene(m, push, instance, t, viewport()));
    // A Fit change holds the old scene until the window has its new size,
    // or HOLD_MS.
    if (push && push.expectViewport) setTimeout(() => setNow((n) => Math.max(n, t + HOLD_MS)), HOLD_MS);
  };

  const onStatus = (raw: unknown): void => {
    const s = readOverlayStatus(raw);
    if (!s || s.instance !== instance) return;
    setStatus(s);
    // Painted means drawn since the window was shown: after the first status
    // that says it is shown, and two frames.
    if (!painted && s.phase === "shown") {
      painted = true;
      twoFrames(() => void sendHandshake(instance, "painted"));
    }
  };

  onMount(() => {
    document.documentElement.style.background = "transparent";
    document.body.style.background = "transparent";

    let frame = 0;
    const onResize = (): void => {
      if (frame) return;
      frame = requestAnimationFrame(() => {
        frame = 0;
        setViewport({ w: window.innerWidth, h: window.innerHeight });
      });
    };
    window.addEventListener("resize", onResize);

    let gone = false;
    const unlisteners: Array<() => void> = [];
    const keep = (unlisten: () => void): void => {
      if (gone) unlisten();
      else unlisteners.push(unlisten);
    };
    onCleanup(() => {
      gone = true;
      window.removeEventListener("resize", onResize);
      cancelAnimationFrame(frame);
      for (const u of unlisteners) u();
    });

    void (async () => {
      try {
        const { emit, listen } = await import("@tauri-apps/api/event");
        keep(
          await listen<OverlayState>(OVERLAY_EVENT, (e) => {
            setState(e.payload);
            applyTheme(e.payload);
          }),
        );
        keep(await listen<ScenePush>(OVERLAY_SCENE_EVENT, (e) => onScene(e.payload)));
        keep(await listen<unknown>(OVERLAY_STATUS_EVENT, (e) => onStatus(e.payload)));
        if (gone) return;
        send = (event, payload) => emit(event, payload);
        // Only now can a push reach us, so only now ask for one. Deliberately
        // not after a paint: the window stays hidden until the backend hears
        // the ready handshake.
        await emit(OVERLAY_READY_EVENT);
        // The debug switch that lets the watchdog be checked.
        if ((window as unknown as Record<string, unknown>)[NO_READY_GLOBAL] !== true) {
          void sendHandshake(instance, "ready");
        }
      } catch {
        // Not under Tauri (the browser preview): the default circle.
        setState(PREVIEW_STATE);
        setMirror((m) => (m.server ? m : { ...m, server: defaultScene() }));
      }
    })();
  });

  return (
    <Stage
      instance={instance}
      state={state()}
      scene={scene()}
      status={status()}
      viewport={viewport()}
      // A fitted window covers its monitor: never a colour there.
      background={status().fit ? "transparent" : state()?.bg}
      onEdit={onEdit}
    />
  );
}

const root = document.getElementById("root");
if (root) render(() => <OverlayApp />, root);
