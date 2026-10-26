import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { render, renderHook } from "@solidjs/testing-library";
import App, { applyLevelUpdate, dispatchGlobalShortcut, Shell } from "./App";
import { AppProvider, useApp, type AppState } from "./stores/app";

const invokeCalls: Array<{ cmd: string; args?: unknown }> = [];
const onFocusChangedListeners: Array<(event: { payload: boolean }) => void> = [];
// v1.56.0: per-test answers for a command, and the events the page emits.
const invokeOverrides = new Map<string, (args?: unknown) => unknown>();
const emitted: Array<{ event: string; payload: unknown }> = [];

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(async (cmd: string, args?: unknown) => {
    invokeCalls.push({ cmd, args });
    const override = invokeOverrides.get(cmd);
    if (override) return override(args);
    switch (cmd) {
      case "list_audio_input_devices":
        return [];
      case "list_audio_output_devices":
        return [];
      case "audio_engine_status":
        return {
          running: false,
          monitoring: true,
          input: { rms: 0, peak: 0 },
          output: { rms: 0, peak: 0 },
        };
      case "list_presets":
        return [];
      default:
        return null;
    }
  }),
}));
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(async () => () => {
    /* unlisten no-op */
  }),
  emit: vi.fn(async (event: string, payload: unknown) => {
    emitted.push({ event, payload });
  }),
}));
vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: vi.fn(() => ({
    minimize: vi.fn(),
    maximize: vi.fn(),
    unmaximize: vi.fn(),
    close: vi.fn(),
    isMaximized: vi.fn(async () => false),
    // v0.11.4: register the focus listener so the device-refresh path
    // can drive it from tests.
    onFocusChanged: vi.fn(
      async (cb: (event: { payload: boolean }) => void) => {
        onFocusChangedListeners.push(cb);
        return () => {
          const idx = onFocusChangedListeners.indexOf(cb);
          if (idx >= 0) onFocusChangedListeners.splice(idx, 1);
        };
      },
    ),
  })),
}));

describe("App shell", () => {
  it("renders the DivoraVoice wordmark (titlebar + first-run wizard)", () => {
    const { getAllByText } = render(() => <App />);
    expect(getAllByText("DivoraVoice").length).toBeGreaterThanOrEqual(1);
  });

  it("shows the persistent LOCAL · NO ACCOUNT affirmation", () => {
    const { getByText } = render(() => <App />);
    expect(getByText(/LOCAL · NO ACCOUNT/)).toBeInTheDocument();
  });

  // v1.58.0: push-to-modulate is off by default, so the default preset's
  // effects are on (it said Clean while the call heard the preset).
  it("shows the Modulated status by default", () => {
    const { getByTestId } = render(() => <App />);
    expect(getByTestId("titlebar-status")).toHaveTextContent("Modulated");
  });
});

// v0.11.4: when the OS window regains focus, App should re-enumerate
// the audio devices so anything plugged in while DivoraVoice was in
// the background shows up immediately. Two paths are wired:
//
//   1. Tauri's `onFocusChanged({ payload: true })` (preferred — fires
//      even when the user clicks back into the chromeless window).
//   2. The browser-level `window.focus` event (fallback for preview /
//      future builds without the window plugin).
//
// We exercise both by clearing the recorded `invoke` calls AFTER mount
// completes, then firing each event and asserting the audio-list
// commands are issued.
describe("App shell — push-to-modulate key handling", () => {
  // The PTM listener swallows the hotkey so it never reaches the page. It
  // must not swallow it from a focused control: browsers activate a button
  // or radio on Space *keyup*, so preventing that everywhere silently broke
  // Space on every button in the app, including the Speak voice cards.
  function pressSpace(target: EventTarget): boolean {
    const up = new KeyboardEvent("keyup", {
      key: " ",
      code: "Space",
      bubbles: true,
      cancelable: true,
    });
    target.dispatchEvent(up);
    return up.defaultPrevented;
  }

  it("leaves Space alone when a control has focus", () => {
    render(() => <App />);
    const button = document.createElement("button");
    document.body.append(button);
    button.focus();
    expect(pressSpace(button)).toBe(false);
    button.remove();
  });

  it("still swallows Space for push-to-modulate when nothing has focus", () => {
    render(() => <App />);
    expect(pressSpace(document.body)).toBe(true);
  });
});

describe("App shell — focus device refresh (v0.11.4)", () => {
  /** Wait one microtask cycle so onMount's async work settles. */
  const flush = (): Promise<void> => new Promise((r) => setTimeout(r, 0));

  it("re-enumerates devices when Tauri's onFocusChanged fires with focused=true", async () => {
    onFocusChangedListeners.length = 0;
    render(() => <App />);
    await flush();
    expect(onFocusChangedListeners.length).toBeGreaterThanOrEqual(1);

    invokeCalls.length = 0;
    onFocusChangedListeners.forEach((cb) => cb({ payload: true }));
    await flush();

    const inputsCalled = invokeCalls.some(
      (c) => c.cmd === "list_audio_input_devices",
    );
    const outputsCalled = invokeCalls.some(
      (c) => c.cmd === "list_audio_output_devices",
    );
    expect(inputsCalled).toBe(true);
    expect(outputsCalled).toBe(true);
  });

  it("ignores blur events (focused=false) — no extra enumeration", async () => {
    onFocusChangedListeners.length = 0;
    render(() => <App />);
    await flush();

    invokeCalls.length = 0;
    onFocusChangedListeners.forEach((cb) => cb({ payload: false }));
    await flush();

    const inputsCalled = invokeCalls.some(
      (c) => c.cmd === "list_audio_input_devices",
    );
    expect(inputsCalled).toBe(false);
  });

  it("falls back to window.focus when the Tauri path is unavailable", async () => {
    render(() => <App />);
    await flush();

    invokeCalls.length = 0;
    window.dispatchEvent(new Event("focus"));
    await flush();

    const inputsCalled = invokeCalls.some(
      (c) => c.cmd === "list_audio_input_devices",
    );
    const outputsCalled = invokeCalls.some(
      (c) => c.cmd === "list_audio_output_devices",
    );
    expect(inputsCalled).toBe(true);
    expect(outputsCalled).toBe(true);
  });
});

// v1.56.0: the mute's seams in the shell — the boot read's order, the
// hotkey the backend owns, the level tick, and the overlay push held until
// the mute is known.
describe("App shell — the mute (v1.56.0)", () => {
  const settle = async (): Promise<void> => {
    for (let i = 0; i < 10; i++) await Promise.resolve();
    await new Promise<void>((r) => setTimeout(r, 0));
  };
  afterEach(() => {
    invokeOverrides.clear();
    emitted.length = 0;
  });

  it("reads the mute before the boot enumerates devices", async () => {
    invokeCalls.length = 0;
    render(() => <App />);
    await settle();
    const cmds = invokeCalls.map((c) => c.cmd);
    const read = cmds.indexOf("input_mute");
    const enumerate = cmds.indexOf("list_audio_input_devices");
    expect(read).toBeGreaterThanOrEqual(0);
    expect(enumerate).toBeGreaterThanOrEqual(0);
    expect(read).toBeLessThan(enumerate);
    // The boot only reads: it never sends a mute.
    expect(cmds.filter((c) => c === "set_input_muted" || c === "toggle_input_muted")).toEqual([]);
  });

  it("a mute hotkey press or release invokes nothing (the backend already flipped it)", async () => {
    const { result } = renderHook(() => useApp(), { wrapper: AppProvider });
    await settle();
    invokeCalls.length = 0;
    dispatchGlobalShortcut(result, { id: "mute", accelerator: "F20", state: "pressed" });
    dispatchGlobalShortcut(result, { id: "mute", accelerator: "F20", state: "released" });
    await settle();
    expect(invokeCalls).toEqual([]);
    expect(result.muted()).toBe(false);
  });

  it("applyLevelUpdate takes a tick's levels and its mute; a tick without the mute leaves it", async () => {
    const { result } = renderHook(() => useApp(), { wrapper: AppProvider });
    await settle();
    const tick = {
      input: { rms: 0.1, peak: 0.2 },
      output: { rms: 0.3, peak: 0.4 },
      running: true,
      monitoring: false,
      dspLatencyMs: 12,
      recording: true,
      loudnessGainDb: 3,
      deviceLost: false,
      modEnv: 0.5,
    };
    applyLevelUpdate(result, tick);
    expect(result.inputLevels()).toEqual({ rms: 0.1, peak: 0.2 });
    expect(result.outputLevels()).toEqual({ rms: 0.3, peak: 0.4 });
    expect(result.engineRunning()).toBe(true);
    expect(result.engineMonitoring()).toBe(false);
    expect(result.dspLatencyMs()).toBe(12);
    expect(result.isRecording()).toBe(true);
    expect(result.loudnessGainDb()).toBe(3);
    expect(result.modEnv()).toBe(0.5);
    // No mute fields: no information, never "unmuted".
    expect(result.muteState()).toBeNull();

    applyLevelUpdate(result, { ...tick, inputMuted: true, inputMuteSeq: 1 });
    expect(result.muted()).toBe(true);
    expect(result.status()).toBe("muted");
    applyLevelUpdate(result, tick);
    expect(result.muted()).toBe(true);
    // An older tick is stale; a newer one is taken.
    applyLevelUpdate(result, { ...tick, inputMuted: false, inputMuteSeq: 0 });
    expect(result.muted()).toBe(true);
    applyLevelUpdate(result, { ...tick, inputMuted: false, inputMuteSeq: 2 });
    expect(result.muted()).toBe(false);
  });

  it("the overlay hears nothing until the mute is read, then Muted", async () => {
    // A reload while the overlay is open and the voice is muted.
    const open = {
      open: true,
      phase: "painted",
      instance: 1,
      mode: "locked",
      arrangeGen: 0,
      canArrange: true,
      fit: false,
      hideWanted: false,
      hideAvailable: true,
      capturing: false,
      error: null,
    };
    invokeOverrides.set("overlay_status", () => open);
    invokeOverrides.set("set_overlay_hide_from_recordings", () => open);
    let answer!: (v: unknown) => void;
    invokeOverrides.set("input_mute", () => new Promise((r) => (answer = r)));
    const states = () =>
      emitted
        .filter((e) => e.event === "overlay:state")
        .map((e) => (e.payload as { status: string }).status);

    render(() => <App />);
    await settle();
    await settle();
    // The overlay is open, but the mute isn't known: nothing goes (it would
    // say Clean).
    expect(invokeCalls.some((c) => c.cmd === "overlay_status")).toBe(true);
    expect(states()).toEqual([]);

    answer({ muted: true, seq: 3 });
    await settle();
    await settle();
    expect(states().length).toBeGreaterThan(0);
    expect(states().every((s) => s === "muted")).toBe(true);
  });
});

// v1.58.0: push-to-modulate's seams in the shell — the boot restore's order,
// the global key the backend owns, the in-app key and its releases, the level
// tick, and the overlay push held until the restore settles.
describe("App shell — push-to-modulate (v1.58.0)", () => {
  const settle = async (): Promise<void> => {
    for (let i = 0; i < 10; i++) await Promise.resolve();
    await new Promise<void>((r) => setTimeout(r, 0));
  };

  /** The engine's word, as the backend keeps it (a held bit per source; a
   *  change bumps `seq`; a set to what it has moves nothing), behind both
   *  commands. */
  function ptmEngine() {
    let mode = "always";
    let bits: Record<string, boolean> = {};
    let seq = 0;
    const state = () => {
      const any = Object.values(bits).some(Boolean);
      const bypassed = mode === "apply" ? !any : mode === "bypass" ? any : false;
      return { mode, held: any, bypassed, seq };
    };
    const write = (m: string, b: Record<string, boolean>) => {
      const same =
        m === mode && ["hotkey", "key", "button", "midi"].every((k) => !!b[k] === !!bits[k]);
      if (!same) {
        mode = m;
        bits = b;
        seq += 1;
      }
      return state();
    };
    invokeOverrides.set("set_ptm_mode", (a) => {
      const { mode: m, restore } = a as { mode: string; restore: boolean };
      return write(m, restore ? { hotkey: !!bits.hotkey } : {});
    });
    invokeOverrides.set("set_ptm_held", (a) => {
      const { source, held } = a as { source: string; held: boolean };
      return write(mode, { ...bits, [source]: held });
    });
    /** The sources held, as the engine has them. */
    const held = () => Object.keys(bits).filter((k) => bits[k]);
    return { held, state };
  }

  /** What the page sent the engine, in order. */
  const ptmCalls = () =>
    invokeCalls.filter((c) => c.cmd.startsWith("set_ptm")).map((c) => [c.cmd, c.args]);
  const heldCall = (source: string, on: boolean) => ["set_ptm_held", { source, held: on }];

  /** The real shell in a provider, with a handle on the store it drives. */
  function renderShell(): AppState {
    let app!: AppState;
    const Probe = (): null => {
      app = useApp();
      return null;
    };
    render(() => (
      <AppProvider>
        <Shell />
        <Probe />
      </AppProvider>
    ));
    return app;
  }

  const key = (
    type: "keydown" | "keyup",
    init: KeyboardEventInit,
    target: EventTarget = document.body,
  ): boolean => {
    const e = new KeyboardEvent(type, { bubbles: true, cancelable: true, ...init });
    target.dispatchEvent(e);
    return e.defaultPrevented;
  };
  const space = { key: " ", code: "Space" };

  const setVisibility = (v: DocumentVisibilityState): void => {
    Object.defineProperty(document, "visibilityState", { configurable: true, get: () => v });
  };

  const tick = {
    input: { rms: 0, peak: 0 },
    output: { rms: 0, peak: 0 },
    running: true,
    monitoring: false,
    dspLatencyMs: 0,
    recording: false,
    loudnessGainDb: 0,
    deviceLost: false,
    modEnv: 0,
  };

  beforeEach(() => {
    window.localStorage.clear();
    invokeOverrides.clear();
    onFocusChangedListeners.length = 0;
  });
  afterEach(() => {
    invokeOverrides.clear();
    emitted.length = 0;
    window.localStorage.clear();
    setVisibility("visible");
  });

  it("restores the saved mode before the boot enumerates devices", async () => {
    ptmEngine();
    invokeCalls.length = 0;
    render(() => <App />);
    await settle();
    const cmds = invokeCalls.map((c) => c.cmd);
    expect(cmds.indexOf("set_ptm_mode")).toBeGreaterThanOrEqual(0);
    expect(cmds.indexOf("set_ptm_mode")).toBeLessThan(cmds.indexOf("list_audio_input_devices"));
    // Off by default: the restore is "always", with `restore`, sent once.
    expect(ptmCalls()).toEqual([["set_ptm_mode", { mode: "always", restore: true }]]);
  });

  it("restores a saved Hold to apply as apply, and the pill says Clean with the key up", async () => {
    ptmEngine();
    window.localStorage.setItem("divora.ptm", JSON.stringify({ on: true, hold: "apply" }));
    invokeCalls.length = 0;
    const { getByTestId } = render(() => <App />);
    await settle();
    expect(ptmCalls()).toEqual([["set_ptm_mode", { mode: "apply", restore: true }]]);
    expect(getByTestId("titlebar-status")).toHaveTextContent("Clean");
  });

  it("a ptm hotkey press or release invokes nothing (the backend holds and lets go)", async () => {
    ptmEngine();
    const { result } = renderHook(() => useApp(), { wrapper: AppProvider });
    await settle();
    invokeCalls.length = 0;
    dispatchGlobalShortcut(result, { id: "ptm", accelerator: "F13", state: "pressed" });
    dispatchGlobalShortcut(result, { id: "ptm", accelerator: "F13", state: "released" });
    await settle();
    expect(invokeCalls).toEqual([]);
  });

  it("Space on the page holds once, ignores autorepeat and lets go on keyup", async () => {
    const engine = ptmEngine();
    renderShell();
    await settle();
    invokeCalls.length = 0;
    expect(key("keydown", space)).toBe(true);
    expect(key("keydown", { ...space, repeat: true })).toBe(true);
    expect(key("keydown", { ...space, repeat: true })).toBe(true);
    expect(ptmCalls()).toEqual([heldCall("key", true)]);
    expect(engine.held()).toEqual(["key"]);
    key("keyup", space);
    expect(ptmCalls()).toEqual([heldCall("key", true), heldCall("key", false)]);
    // A keyup with nothing held sends nothing.
    key("keyup", space);
    expect(ptmCalls()).toHaveLength(2);
  });

  it("autorepeat after a mode set doesn't hold again: the set let go of the key", async () => {
    const engine = ptmEngine();
    const app = renderShell();
    await settle();
    await app.setPtmOn(true);
    key("keydown", space);
    await settle();
    expect(engine.held()).toEqual(["key"]);
    // Re-picking the hold on the card lets go of every hold, Space still down.
    await app.setPtmHold("apply");
    expect(engine.held()).toEqual([]);
    invokeCalls.length = 0;
    key("keydown", { ...space, repeat: true });
    key("keydown", { ...space, repeat: true });
    key("keyup", space);
    await settle();
    expect(ptmCalls()).toEqual([]);
    // A fresh press holds.
    key("keydown", space);
    expect(ptmCalls()).toEqual([heldCall("key", true)]);
  });

  it("window blur lets go of the key", async () => {
    ptmEngine();
    renderShell();
    await settle();
    invokeCalls.length = 0;
    key("keydown", space);
    window.dispatchEvent(new Event("blur"));
    expect(ptmCalls()).toEqual([heldCall("key", true), heldCall("key", false)]);
    // Nothing held: a blur sends nothing.
    window.dispatchEvent(new Event("blur"));
    expect(ptmCalls()).toHaveLength(2);
  });

  it("the page hidden lets go of the key; shown sends nothing", async () => {
    ptmEngine();
    renderShell();
    await settle();
    key("keydown", space);
    invokeCalls.length = 0;
    setVisibility("visible");
    document.dispatchEvent(new Event("visibilitychange"));
    expect(ptmCalls()).toEqual([]);
    setVisibility("hidden");
    document.dispatchEvent(new Event("visibilitychange"));
    expect(ptmCalls()).toEqual([heldCall("key", false)]);
  });

  it("Tauri's focus lost lets go of the key; gaining it holds nothing", async () => {
    ptmEngine();
    renderShell();
    await settle();
    expect(onFocusChangedListeners.length).toBeGreaterThanOrEqual(1);
    key("keydown", space);
    invokeCalls.length = 0;
    onFocusChangedListeners.forEach((cb) => cb({ payload: false }));
    expect(ptmCalls()).toEqual([heldCall("key", false)]);
    onFocusChangedListeners.forEach((cb) => cb({ payload: true }));
    await settle();
    expect(ptmCalls()).toHaveLength(1);
  });

  it.each([
    ["window blur", () => window.dispatchEvent(new Event("blur"))],
    [
      "the page hidden",
      () => {
        setVisibility("hidden");
        document.dispatchEvent(new Event("visibilitychange"));
      },
    ],
    ["Tauri's focus lost", () => onFocusChangedListeners.forEach((cb) => cb({ payload: false }))],
  ])("%s lets go of the Mixer's button, from the shell too", async (_, lose) => {
    ptmEngine();
    const app = renderShell();
    await settle();
    // Off the Mixer, so this is the shell's own path: the card lets go of
    // its button on blur and when hidden as well, while it is mounted.
    app.setNav("presets");
    await settle();
    app.ptmDown("button");
    invokeCalls.length = 0;
    lose();
    expect(ptmCalls()).toEqual([heldCall("button", false)]);
  });

  it("a bound key with modifiers needs them on the press, not on the release", async () => {
    ptmEngine();
    window.localStorage.setItem("divora.hotkeyBindings", JSON.stringify({ ptm: "Ctrl+Shift+M" }));
    renderShell();
    await settle();
    invokeCalls.length = 0;
    // A bare M, or M with one of the two, doesn't switch the call.
    expect(key("keydown", { key: "m", code: "KeyM" })).toBe(false);
    key("keyup", { key: "m", code: "KeyM" });
    key("keydown", { key: "M", code: "KeyM", shiftKey: true });
    key("keyup", { key: "M", code: "KeyM" });
    expect(ptmCalls()).toEqual([]);
    // Space is no longer the key.
    key("keydown", space);
    expect(ptmCalls()).toEqual([]);

    expect(key("keydown", { key: "M", code: "KeyM", ctrlKey: true, shiftKey: true })).toBe(true);
    expect(ptmCalls()).toEqual([heldCall("key", true)]);
    // Ctrl and Shift let go first: M's keyup alone still lets go.
    key("keyup", { key: "m", code: "KeyM" });
    expect(ptmCalls()).toEqual([heldCall("key", true), heldCall("key", false)]);
  });

  it.each([
    // Settings captures Shift+1 as "Shift+!" (e.key); letting go of Shift
    // first makes the 1's keyup say "1".
    [
      "Shift+!",
      { key: "!", code: "Digit1", shiftKey: true },
      [{ key: "Shift", code: "ShiftLeft" }],
      { key: "1", code: "Digit1" },
    ],
    // AltGr+Q is "@" on a German layout, with Ctrl and Alt; Q alone is "q".
    [
      "Ctrl+Alt+@",
      { key: "@", code: "KeyQ", ctrlKey: true, altKey: true },
      [{ key: "AltGraph", code: "AltRight" }],
      { key: "q", code: "KeyQ" },
    ],
    // An unshifted 1, with Shift pressed during the hold: its keyup says "!".
    ["1", { key: "1", code: "Digit1" }, [], { key: "!", code: "Digit1", shiftKey: true }],
  ] as const)(
    "%s held lets go on the same physical key's keyup, whatever character it says",
    async (binding, press, before, release) => {
      ptmEngine();
      window.localStorage.setItem("divora.hotkeyBindings", JSON.stringify({ ptm: binding }));
      renderShell();
      await settle();
      invokeCalls.length = 0;
      expect(key("keydown", press)).toBe(true);
      expect(ptmCalls()).toEqual([heldCall("key", true)]);
      for (const k of before) key("keyup", k);
      expect(ptmCalls()).toEqual([heldCall("key", true)]);
      // Another key's keyup doesn't let go.
      key("keyup", { key: "2", code: "Digit2" });
      expect(ptmCalls()).toEqual([heldCall("key", true)]);
      key("keyup", release);
      expect(ptmCalls()).toEqual([heldCall("key", true), heldCall("key", false)]);
      // And it holds again on the next press.
      key("keydown", press);
      expect(ptmCalls()).toEqual([heldCall("key", true), heldCall("key", false), heldCall("key", true)]);
    },
  );

  it("applyLevelUpdate adopts a tick's ptm when newer; a tick without it leaves the mirror", async () => {
    const { result } = renderHook(() => useApp(), { wrapper: AppProvider });
    await settle();
    const bypassed = { mode: "apply" as const, held: false, bypassed: true, seq: 5 };
    applyLevelUpdate(result, { ...tick, ptm: bypassed });
    expect(result.ptmState()).toEqual(bypassed);
    expect(result.status()).toBe("clean");
    applyLevelUpdate(result, tick);
    expect(result.ptmState()).toEqual(bypassed);
    // An older tick is stale; a newer one is taken.
    applyLevelUpdate(result, { ...tick, ptm: { ...bypassed, held: true, bypassed: false, seq: 4 } });
    expect(result.status()).toBe("clean");
    applyLevelUpdate(result, { ...tick, ptm: { ...bypassed, held: true, bypassed: false, seq: 6 } });
    expect(result.status()).toBe("modulated");
  });

  it("the overlay hears nothing until the boot restore settles, a tick before it included; then Modulated", async () => {
    const open = {
      open: true,
      phase: "painted",
      instance: 1,
      mode: "locked",
      arrangeGen: 0,
      canArrange: true,
      fit: false,
      hideWanted: false,
      hideAvailable: true,
      capturing: false,
      error: null,
    };
    invokeOverrides.set("overlay_status", () => open);
    invokeOverrides.set("set_overlay_hide_from_recordings", () => open);
    invokeOverrides.set("input_mute", () => ({ muted: false, seq: 0 }));
    let answer!: (v: unknown) => void;
    invokeOverrides.set("set_ptm_mode", () => new Promise((r) => (answer = r)));
    const states = () =>
      emitted
        .filter((e) => e.event === "overlay:state")
        .map((e) => (e.payload as { status: string }).status);

    const app = renderShell();
    await settle();
    await settle();
    expect(invokeCalls.some((c) => c.cmd === "overlay_status")).toBe(true);
    expect(app.muteKnown()).toBe(true);
    expect(states()).toEqual([]);

    // A tick from before the restore (it may carry the previous page's
    // hold): the mirror takes it, but it isn't the boot's answer.
    applyLevelUpdate(app, { ...tick, ptm: { mode: "always", held: false, bypassed: false, seq: 3 } });
    await settle();
    expect(app.ptmState()?.seq).toBe(3);
    expect(app.ptmKnown()).toBe(false);
    expect(states()).toEqual([]);

    answer({ mode: "always", held: false, bypassed: false, seq: 3 });
    await settle();
    await settle();
    // Off by default, with the default preset's effects on.
    expect(states().length).toBeGreaterThan(0);
    expect(states().every((s) => s === "modulated")).toBe(true);
  });
});
