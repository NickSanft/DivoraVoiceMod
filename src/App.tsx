// App shell. Composes the custom titlebar, sidebar nav rail, and the
// active screen. Wires the Tweaks system into root-level data-mood,
// data-accent, and data-motion attributes so the design-token swap
// works without rerendering anything. Phase 2 adds the audio-engine
// bootstrap: refresh devices, auto-start, and subscribe to live level
// events.

import {
  createEffect,
  Match,
  onCleanup,
  onMount,
  Switch,
  type JSX,
} from "solid-js";
import type { UnlistenFn } from "@tauri-apps/api/event";
import { subscribeGlobalShortcut, subscribeLevels } from "./audio/api";
import { Wizard } from "./components/Wizard";
import { Sidebar } from "./shell/Sidebar";
import { Titlebar } from "./shell/Titlebar";
import { MixerScreen } from "./screens/MixerScreen";
import { SoundboardScreen } from "./screens/SoundboardScreen";
import { PresetsScreen } from "./screens/PresetsScreen";
import { SettingsScreen } from "./screens/SettingsScreen";
import { AppProvider, useApp } from "./stores/app";

function applyRootAttr(name: string, value: string | null) {
  const root = document.documentElement;
  if (value === null) root.removeAttribute(name);
  else root.setAttribute(name, value);
}

function Shell(): JSX.Element {
  const app = useApp();

  // Reflect Tweaks state on the root so CSS variable cascades fire.
  createEffect(() => {
    applyRootAttr(
      "data-mood",
      app.tweaks.mood === "violet" ? null : app.tweaks.mood,
    );
  });
  createEffect(() => {
    applyRootAttr(
      "data-accent",
      app.tweaks.accent === "brand" ? null : app.tweaks.accent,
    );
  });
  createEffect(() => {
    const m = app.tweaks.motion;
    applyRootAttr(
      "data-motion",
      m === 0 ? "functional" : m < 0.8 ? "ambient" : "rich",
    );
    // Mark that the user has chosen a motion level so we don't override
    // it via the prefers-reduced-motion media query.
    applyRootAttr("data-motion-user", "true");
  });
  createEffect(() => {
    document.body.classList.toggle("grain", app.tweaks.grain);
    document.body.classList.toggle("vignette", app.tweaks.vignette);
  });

  // Push-to-modulate fallback (in-app keyboard). The system-level
  // tauri-plugin-global-shortcut wiring lives in the next onMount and
  // fires even while the app is unfocused — this listener covers the
  // case where the window IS focused, since global-shortcut on Windows
  // does NOT swallow the key from the focused app.
  onMount(() => {
    const matches = (e: KeyboardEvent): boolean => {
      const target = app.ui.ptmKey || "Space";
      // Tauri accelerators look like "Space", "Ctrl+Shift+P". We only
      // need to match the trailing key for the in-app fallback.
      const parts = target.split("+");
      const last = parts[parts.length - 1] ?? "Space";
      if (last === "Space") return e.code === "Space";
      if (last.length === 1) return e.key.toUpperCase() === last.toUpperCase();
      return e.key === last;
    };
    const down = (e: KeyboardEvent) => {
      if (matches(e) && e.target === document.body) {
        e.preventDefault();
        if (!app.ui.pressed) app.setUi("pressed", true);
      }
    };
    const up = (e: KeyboardEvent) => {
      if (matches(e)) {
        e.preventDefault();
        app.setUi("pressed", false);
      }
    };
    window.addEventListener("keydown", down);
    window.addEventListener("keyup", up);
    onCleanup(() => {
      window.removeEventListener("keydown", down);
      window.removeEventListener("keyup", up);
    });
  });

  // Global hotkey subscription. The backend emits `global-shortcut`
  // events with the binding's id ("ptm" | "panic" | "monitor") and
  // pressed/released state. We translate those into store mutations:
  //   ptm     → set ui.pressed on press, clear on release
  //   panic   → stop all soundboard clips on press
  //   monitor → toggle sidetone on press
  onMount(() => {
    let unlisten: UnlistenFn | null = null;
    let cancelled = false;
    void (async () => {
      try {
        unlisten = await subscribeGlobalShortcut((event) => {
          if (event.id === "ptm") {
            app.setUi("pressed", event.state === "pressed");
            return;
          }
          if (event.state !== "pressed") return;
          if (event.id === "panic") {
            void app.panicSoundboard();
          } else if (event.id === "monitor") {
            void app.toggleMonitor();
          }
        });
      } catch (err) {
        // Browser preview without the Tauri bridge — the in-app
        // fallback above still works.
        console.warn("[hotkey] subscribe failed", err);
      }
      if (cancelled && unlisten) {
        unlisten();
        unlisten = null;
        return;
      }
      // Push any persisted bindings into the backend so they survive
      // a restart. Defaults: PTM = Space, others empty (so this is a
      // no-op on first boot).
      await app.syncHotkeyBindings();
    })();
    onCleanup(() => {
      cancelled = true;
      if (unlisten) unlisten();
    });
  });

  // Audio engine bootstrap.
  // 1) Enumerate devices.
  // 2) Subscribe to `audio-levels` events from the backend.
  // 3) Refresh the preset list from the backend (bundled + user JSON).
  // 4) Attempt to auto-start with the selected (default) devices.
  // Each step is wrapped to fail gracefully — we may be running in a
  // browser preview without the Tauri bridge.
  onMount(() => {
    let unlisten: UnlistenFn | null = null;
    let cancelled = false;

    void (async () => {
      try {
        await app.refreshDevices();
      } catch (err) {
        app.setEngineError(`device enumeration failed: ${String(err)}`);
      }

      try {
        unlisten = await subscribeLevels((update) => {
          app.setInputLevels(update.input);
          app.setOutputLevels(update.output);
          app.setEngineRunning(update.running);
          app.setEngineMonitoring(update.monitoring);
        });
      } catch (err) {
        app.setEngineError(`level subscription failed: ${String(err)}`);
      }

      // Pull the live preset list (bundled + user) from the backend.
      // Failures keep the fallback bundled list seeded in the store.
      await app.refreshPresets();

      if (cancelled) return;
      // Only auto-start when we have something to talk to.
      if (app.selectedInput() && app.selectedOutput()) {
        await app.startEngine();
      }
    })();

    onCleanup(() => {
      cancelled = true;
      if (unlisten) unlisten();
      void app.stopEngine();
    });
  });

  return (
    <div
      style={{
        display: "flex",
        "flex-direction": "column",
        height: "100%",
        background: "var(--surface-0)",
      }}
    >
      <Titlebar />
      <div style={{ display: "flex", flex: 1, "min-height": 0 }}>
        <Sidebar />
        <div
          style={{
            flex: 1,
            "min-width": 0,
            position: "relative",
            background: "var(--surface-0)",
            overflow: "hidden",
          }}
        >
          <Switch>
            <Match when={app.nav() === "mixer"}>
              <MixerScreen />
            </Match>
            <Match when={app.nav() === "soundboard"}>
              <SoundboardScreen />
            </Match>
            <Match when={app.nav() === "presets"}>
              <PresetsScreen />
            </Match>
            <Match when={app.nav() === "settings"}>
              <SettingsScreen />
            </Match>
          </Switch>
          <Wizard />
        </div>
      </div>
    </div>
  );
}

export default function App(): JSX.Element {
  return (
    <AppProvider>
      <Shell />
    </AppProvider>
  );
}
