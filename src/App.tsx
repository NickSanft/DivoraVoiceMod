// App shell. Composes the custom titlebar, sidebar nav rail, and the
// active screen. Wires the Tweaks system into root-level data-mood,
// data-accent, and data-motion attributes so the design-token swap
// works without rerendering anything.

import { createEffect, Match, onCleanup, onMount, Switch, type JSX } from "solid-js";
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
    // Grain and vignette overlays live on the shell root rather than a
    // CSS-variable attribute. We toggle via a class.
    document.body.classList.toggle("grain", app.tweaks.grain);
    document.body.classList.toggle("vignette", app.tweaks.vignette);
  });

  // Push-to-modulate placeholder: hold the bound key, set ui.pressed.
  // The global hotkey system (works while backgrounded) ships in Phase 3.
  onMount(() => {
    const down = (e: KeyboardEvent) => {
      if (e.code === "Space" && e.target === document.body) {
        e.preventDefault();
        if (!app.ui.pressed) app.setUi("pressed", true);
      }
    };
    const up = (e: KeyboardEvent) => {
      if (e.code === "Space") {
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
