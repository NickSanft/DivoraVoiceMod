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
  Show,
  Switch,
  type JSX,
} from "solid-js";
import type { UnlistenFn } from "@tauri-apps/api/event";
import {
  subscribeGlobalShortcut,
  subscribeLevels,
  subscribeMidi,
  subscribeTtsProgress,
  type GlobalShortcutEvent,
  type LevelUpdate,
} from "./audio/api";
import { levelMute } from "./data/mute";
import { levelPtm, ptmKeyMatches } from "./data/ptm";
import { overlayPayload } from "./overlay/state";
import { overlayHeld, syncOverlay } from "./overlay/sync";
import { Toast } from "./components/Toast";
import { Wizard } from "./components/Wizard";
import { WhatsNewBanner, WhatsNewModal } from "./components/WhatsNew";
import { Sidebar } from "./shell/Sidebar";
import { Titlebar } from "./shell/Titlebar";
import { MixerScreen } from "./screens/MixerScreen";
import { CovenScreen } from "./screens/CovenScreen";
import { SoundboardScreen } from "./screens/SoundboardScreen";
import { SpeakScreen } from "./screens/SpeakScreen";
import { PresetsScreen } from "./screens/PresetsScreen";
import { SettingsScreen } from "./screens/SettingsScreen";
import { AppProvider, useApp, type AppState } from "./stores/app";

function applyRootAttr(name: string, value: string | null) {
  const root = document.documentElement;
  if (value === null) root.removeAttribute(name);
  else root.setAttribute(name, value);
}

// Global hotkeys. The backend emits `global-shortcut` events with the
// binding's id and pressed/released state; this turns them into store
// actions. It is the path every id takes, except that `overlay:arrange`
// (v1.55.0), `mute` (v1.56.0), `ptm` (v1.58.0) and the `tm:` timer keys
// (v1.59.0) are acted on by the backend; here the first only shows a notice
// while the overlay is off or starting, and the others do nothing. It runs
// while the window is hidden to the tray too:
//   ptm             → nothing: the backend holds push-to-modulate on the
//                     press and lets go on the release; acting here would
//                     do it twice (v1.58.0)
//   panic           → stop all soundboard clips on press
//   monitor         → toggle sidetone on press
//   rec:screen      → start / stop the screen recording (v1.52.0)
//   rec:wav         → start / stop the WAV recording (v1.52.0)
//   replay:save     → save what Replay has kept (v1.54.0)
//   replay:toggle   → turn Replay on / off (v1.54.0) — on goes through
//                     armReplay(), which refuses until the card's explainer
//                     has been read
//   overlay:arrange → the backend already switched Arrange / Locked, inside
//                     the hotkey's own message, while Windows still lets it
//                     take the foreground; never set_overlay_mode from here,
//                     or the press would toggle twice (v1.55.0)
//   mute            → nothing: the backend already flipped the mute on the
//                     press; acting here would flip it twice (v1.56.0)
//   tm:<id>:<verb>  → nothing: the backend already started, paused, resumed
//                     or reset the Hourglass timer on the press, or (v1.60.0,
//                     `finish`) ended it; acting here would do it twice
//                     (v1.59.0)
//   sb:<clip>       → play that soundboard tile
export function dispatchGlobalShortcut(app: AppState, event: GlobalShortcutEvent): void {
  if (event.id === "ptm") {
    // The backend already held or let go (v1.58.0); acting here would do it
    // twice. The next level tick brings the new state.
    return;
  }
  if (event.state !== "pressed") return;
  if (event.id === "panic") {
    void app.panicSoundboard();
  } else if (event.id === "monitor") {
    void app.toggleMonitor();
  } else if (event.id === "rec:screen") {
    void app.toggleScreenRecording();
  } else if (event.id === "rec:wav") {
    void app.toggleRecording();
  } else if (event.id === "replay:save") {
    void app.saveReplay();
  } else if (event.id === "replay:toggle") {
    void app.toggleReplay();
  } else if (event.id === "overlay:arrange") {
    app.noteOverlayHotkey();
  } else if (event.id === "mute") {
    // The backend already flipped it (v1.56.0); acting here would flip it
    // twice. The next level tick brings the new state.
  } else if (event.id.startsWith("tm:")) {
    // An Hourglass timer's key: the backend already acted on it (v1.59.0).
    // The next `timer-state` brings the new state.
  } else if (event.id.startsWith("sb:")) {
    // Per-tile hotkey: `sb:<clipId>` — fire the clip if the
    // current scan still has it.
    void app.playTileById(event.id.slice(3));
  }
}

/** v1.58.0: lets go of everything the page holds of push-to-modulate, the
 *  in-app key and the Mixer's Hold to switch button, where no keyup or
 *  pointerup will come: the window lost the focus, or the page is hidden.
 *  Each sends only if the page held it, so a second path sends nothing.
 *  MIDI isn't the page's: a pad keeps holding while Divora is behind. */
function letGoOfPagePtm(app: AppState): void {
  app.ptmUp("key");
  app.ptmUp("button");
}

/** One `audio-levels` tick into the store. v1.56.0 adds the mute: the tick's
 *  pair is adopted only when it is newer than what the mirror holds, and a
 *  tick without it changes nothing. v1.58.0 adds push-to-modulate's state,
 *  by the same rule. */
export function applyLevelUpdate(app: AppState, update: LevelUpdate): void {
  app.setInputLevels(update.input);
  app.setOutputLevels(update.output);
  app.setEngineRunning(update.running);
  app.setEngineMonitoring(update.monitoring);
  app.setDspLatencyMs(update.dspLatencyMs);
  app.setIsRecording(update.recording);
  app.setLoudnessGainDb(update.loudnessGainDb);
  app.setDeviceLost(update.deviceLost);
  app.setModEnv(update.modEnv);
  app.applyMuteState(levelMute(update));
  app.applyPtmState(levelPtm(update));
}

/** The shell inside the store's provider. Exported so its tests can render
 *  it in a provider of their own and reach the store it drives. */
export function Shell(): JSX.Element {
  const app = useApp();

  // Reflect Tweaks state on the root so CSS variable cascades fire.
  // Theme polarity is orthogonal to mood: `data-theme="light"` flips the
  // surface/text ramp, and the mood blocks compose on top via specificity
  // (`[data-theme="light"][data-mood="ink"]`). Dark is the default, so we
  // omit the attribute for it (mirrors the `violet` mood default).
  createEffect(() => {
    applyRootAttr(
      "data-theme",
      app.tweaks.theme === "light" ? "light" : null,
    );
  });
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
  // Phase 11.1: surface the mystical level as both a data attribute
  // (for category-style CSS rules) and a `--mystical` numeric CSS
  // variable (for things that want a smooth scalar like opacity).
  createEffect(() => {
    const m = app.tweaks.mystical;
    const tier = m <= 0.4 ? "subtle" : m <= 0.85 ? "balanced" : "rich";
    applyRootAttr("data-mystical", tier);
    document.documentElement.style.setProperty("--mystical", String(m));
  });
  createEffect(() => {
    document.body.classList.toggle("grain", app.tweaks.grain);
    document.body.classList.toggle("vignette", app.tweaks.vignette);
  });

  // v1.16.0: while the stream overlay is open, push the spell-circle state
  // to it over the Tauri event bus on every change (status / chain / tweaks
  // / bg mode). The overlay animates on its own from there — no per-frame
  // traffic. v1.51.3: and again whenever the overlay says it is listening
  // (`overlay:ready`), so the push made as it opens is never lost. No-op off
  // Tauri. v1.55.0: the charms travel on their own event, `overlay:scene`,
  // which the store's overlay controller pushes. v1.56.0: held until the
  // store knows the mute or 500 ms pass (`muteKnown`), so a reload during a
  // mute shows the stream unmuted only if the boot read takes longer than
  // that, and then only until the read or a level tick lands. v1.58.0: and
  // until push-to-modulate's boot restore settles or 500 ms pass
  // (`ptmKnown`; a tick doesn't count), so a reload with it on and the key
  // up doesn't show the stream Modulated over a clean voice.
  syncOverlay({
    isOpen: app.overlayOpen,
    held: () => overlayHeld(app),
    payload: () =>
      overlayPayload({
        chain: app.chain(),
        status: app.status(),
        motion: app.tweaks.motion,
        mystical: app.tweaks.mystical,
        mood: app.tweaks.mood,
        accent: app.tweaks.accent,
        theme: app.tweaks.theme,
        bg: app.overlayBg(),
      }),
    emit: async (event, payload) => {
      const { emit } = await import("@tauri-apps/api/event");
      await emit(event, payload);
    },
    listen: async (event, handler) => {
      const { listen } = await import("@tauri-apps/api/event");
      return listen(event, handler);
    },
  });

  // Push-to-modulate's in-app key: the default input (Space), which works
  // only while Divora's window has the focus. A global key is the
  // backend's own source (it acts on the press and the release); a key
  // registered globally takes the key-down from the focused app (CHANGELOG
  // v0.7.1) while the key-up still arrives here. v1.58.0: this listener
  // holds its own source in the engine ("key"), so either path may fire
  // and neither releases the other's hold. A press needs the bound key's
  // modifiers exactly and ignores autorepeat; a release matches the
  // trailing key alone, or the physical key that made the press (its
  // `code`), wherever focus ended up. The second is for a character that
  // changes with Shift or AltGr: Shift+1 binds as "Shift+!", and letting go
  // of Shift first makes the 1's keyup say "1". Where no keyup or
  // pointerup will come, the page lets go of the key and the Mixer's
  // button: the window losing the focus (`blur`, and Tauri's own focus
  // event below), or the page hidden (`visibilitychange`). Without that, a
  // key held across Alt-Tab stays held.
  onMount(() => {
    const key = (): string => app.ui.ptmKey || "Space";
    // Set by every press before it holds, so while the page holds the key
    // this is the key that held it. A stale one is harmless: `ptmUp` sends
    // nothing unless the page holds the key.
    let heldCode: string | null = null;
    const down = (e: KeyboardEvent) => {
      if (ptmKeyMatches(key(), e, "down") && e.target === document.body) {
        e.preventDefault();
        if (!e.repeat) {
          heldCode = e.code || null;
          app.ptmDown("key");
        }
      }
    };
    const up = (e: KeyboardEvent) => {
      if (!ptmKeyMatches(key(), e, "up") && (heldCode === null || e.code !== heldCode)) return;
      heldCode = null;
      // Only swallow the key when nothing else wants it. Browsers activate a
      // focused button on Space *keyup*, so preventing it unconditionally
      // silently broke Space on every button and radio in the app. Releasing
      // still lets go wherever focus ended up.
      if (e.target === document.body) e.preventDefault();
      app.ptmUp("key");
    };
    const blur = (): void => {
      heldCode = null;
      letGoOfPagePtm(app);
    };
    const visibility = (): void => {
      if (document.visibilityState === "hidden") {
        heldCode = null;
        letGoOfPagePtm(app);
      }
    };
    window.addEventListener("keydown", down);
    window.addEventListener("keyup", up);
    window.addEventListener("blur", blur);
    document.addEventListener("visibilitychange", visibility);
    onCleanup(() => {
      window.removeEventListener("keydown", down);
      window.removeEventListener("keyup", up);
      window.removeEventListener("blur", blur);
      document.removeEventListener("visibilitychange", visibility);
    });
  });

  // Global hotkey subscription: see dispatchGlobalShortcut above.
  onMount(() => {
    let unlisten: UnlistenFn | null = null;
    let cancelled = false;
    void (async () => {
      try {
        unlisten = await subscribeGlobalShortcut((event) => dispatchGlobalShortcut(app, event));
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
      // Push the persisted bindings (`divora.hotkeyBindings`, v1.51.4)
      // into the backend so they survive a restart. Every default is empty,
      // so this is a no-op on first boot.
      await app.syncHotkeyBindings();
    })();
    onCleanup(() => {
      cancelled = true;
      if (unlisten) unlisten();
    });
  });

  // v1.9.0: MIDI control surfaces. Subscribe to `midi-message` events and
  // feed them to the store router (which learns a trigger if armed, else
  // dispatches to the bound action). Then enumerate the ports and re-open
  // the persisted device so bindings work right after a restart — mirrors
  // how the audio devices auto-start below.
  onMount(() => {
    let unlisten: UnlistenFn | null = null;
    let cancelled = false;
    void (async () => {
      try {
        unlisten = await subscribeMidi((msg) => app.handleMidiMessage(msg));
      } catch (err) {
        // Browser preview without the Tauri bridge — MIDI is desktop-only.
        console.warn("[midi] subscribe failed", err);
      }
      if (cancelled) {
        if (unlisten) unlisten();
        unlisten = null;
        return;
      }
      await app.refreshMidiInputs();
      const saved = app.selectedMidiInput();
      if (saved) await app.selectMidiInput(saved);
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
  // The levels subscription stays after enumeration: subscribed first, a
  // reload's early `running: true` tick would set engineRunning, and a
  // default pick in refreshDevices (a saved device gone) would then fire
  // the device effect's restart beside the boot's own start. The mute
  // doesn't wait for ticks: the store read it at creation, before this
  // (v1.56.0).
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
        unlisten = await subscribeLevels((update) => applyLevelUpdate(app, update));
      } catch (err) {
        app.setEngineError(`level subscription failed: ${String(err)}`);
      }

      // Pull the live preset list (bundled + user) from the backend.
      // Failures keep the fallback bundled list seeded in the store.
      await app.refreshPresets();

      // v1.12.0: fire-and-forget a lightweight update check (opt-out,
      // no telemetry; no-ops off Tauri / in dev). Never blocks startup.
      void app.checkUpdates();

      // v1.49.0: decide whether this launch has anything to announce.
      // Purely local (notes are compiled in), runs once per process, and
      // stays silent on a brand-new install and on dev builds.
      void app.checkWhatsNew();

      // Phase 15: re-scan the persisted soundboard folder (restored from
      // localStorage) so its tiles + per-tile hotkeys are live without
      // the user re-picking the folder each launch.
      if (app.soundboardFolder()) {
        await app.scanCurrentSoundboardFolder();
      }

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

  // v1.27.0: per-take TTS synthesis progress → the Speak panel, so a multi-take
  // (best-of-N) VoxCPM generation shows "Take 2 of 6" instead of a frozen UI.
  onMount(() => {
    let unlisten: UnlistenFn | null = null;
    let cancelled = false;
    void (async () => {
      try {
        unlisten = await subscribeTtsProgress((p) => app.setTtsProgress(p));
      } catch (err) {
        console.warn("[tts] progress subscribe failed", err);
      }
      if (cancelled && unlisten) unlisten();
    })();
    onCleanup(() => {
      cancelled = true;
      if (unlisten) unlisten();
    });
  });

  // v0.11.4: re-enumerate audio devices whenever the user returns focus
  // to the app. cpal's `Host::devices()` only reflects the system list
  // at the moment of the call — there's no cross-platform "device
  // arrived" notification we can subscribe to without dropping into
  // WASAPI's `IMMNotificationClient` (Windows-only, COM-heavy). The
  // common workflow when plugging in a mic is:
  //
  //   1) user plugs the device in
  //   2) user alt-tabs back to DivoraVoice
  //   3) user opens Settings to pick it
  //
  // Refreshing on every focus catches step 2 transparently. The Tauri
  // window-level `onFocusChanged` event is the source of truth on the
  // OS-window level; we also listen to the browser-level `focus` event
  // as a belt-and-suspenders fallback (covers browser preview / older
  // Tauri builds where the plugin import fails). v1.58.0: losing the focus
  // lets go of the page's push-to-modulate holds here too, beside the DOM's
  // `blur` above; whichever comes second sends nothing.
  onMount(() => {
    let unlistenTauri: UnlistenFn | null = null;
    let cancelled = false;

    void (async () => {
      try {
        const mod = await import("@tauri-apps/api/window");
        const w = mod.getCurrentWindow();
        const fn = await w.onFocusChanged(({ payload: focused }) => {
          if (!focused) {
            letGoOfPagePtm(app);
            return;
          }
          void app.refreshDevices();
        });
        if (cancelled) {
          fn();
        } else {
          unlistenTauri = fn;
        }
      } catch (err) {
        // Browser preview or older Tauri without the window plugin —
        // the window-level "focus" listener below still fires.
        console.warn("[devices] Tauri focus subscription failed", err);
      }
    })();

    const onFocus = (): void => {
      void app.refreshDevices();
    };
    window.addEventListener("focus", onFocus);

    onCleanup(() => {
      cancelled = true;
      window.removeEventListener("focus", onFocus);
      if (unlistenTauri) unlistenTauri();
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
      {/* v1.39.0: device-loss recovery gave up — the engine is stopped with no
          audio. Surface it (instead of a silent metering-but-dead state) with a
          one-click restart. */}
      <Show when={app.deviceLost()}>
        <div
          role="alert"
          style={{
            display: "flex",
            "align-items": "center",
            gap: "0.75rem",
            padding: "0.5rem 1rem",
            background: "var(--danger, #b4232a)",
            color: "#fff",
            "font-size": "0.85rem",
          }}
        >
          <span style={{ flex: 1 }}>
            Audio device lost — the engine stopped and couldn't recover on its
            own (a headset unplugged, or the output device changed).
          </span>
          <button
            type="button"
            onClick={() => void app.startEngine()}
            style={{
              padding: "0.3rem 0.8rem",
              "border-radius": "6px",
              border: "1px solid rgba(255,255,255,0.6)",
              background: "rgba(255,255,255,0.15)",
              color: "#fff",
              "font-weight": 600,
              cursor: "pointer",
            }}
          >
            Restart audio
          </button>
        </div>
      </Show>
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
            <Match when={app.nav() === "coven"}>
              <CovenScreen />
            </Match>
            <Match when={app.nav() === "soundboard"}>
              <SoundboardScreen />
            </Match>
            <Match when={app.nav() === "speak"}>
              <SpeakScreen />
            </Match>
            <Match when={app.nav() === "presets"}>
              <PresetsScreen />
            </Match>
            <Match when={app.nav() === "settings"}>
              <SettingsScreen />
            </Match>
          </Switch>
          <WhatsNewBanner />
          {/* v1.54.0: top-centre, clear of the banner's bottom slot. Always
              mounted: it is the live region. */}
          <Toast message={app.toast()} />
          <Wizard />
        </div>
      </div>
      <WhatsNewModal />
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
