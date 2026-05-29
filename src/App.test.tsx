import { describe, expect, it, vi } from "vitest";
import { render } from "@solidjs/testing-library";
import App from "./App";

const invokeCalls: Array<{ cmd: string; args?: unknown }> = [];
const onFocusChangedListeners: Array<(event: { payload: boolean }) => void> = [];

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(async (cmd: string, args?: unknown) => {
    invokeCalls.push({ cmd, args });
    switch (cmd) {
      case "ping":
        return "pong";
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
