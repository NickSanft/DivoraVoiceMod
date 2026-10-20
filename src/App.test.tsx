import { afterEach, describe, expect, it, vi } from "vitest";
import { render, renderHook } from "@solidjs/testing-library";
import App, { applyLevelUpdate, dispatchGlobalShortcut } from "./App";
import { AppProvider, useApp } from "./stores/app";

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

  it("shows the Clean status by default", () => {
    const { getAllByText } = render(() => <App />);
    expect(getAllByText(/Clean/).length).toBeGreaterThanOrEqual(1);
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
