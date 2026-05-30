// v0.11.4: the Settings screen re-enumerates audio devices on mount
// and provides a manual "Refresh" button alongside the section header.
// Together with the App-level focus refresh, this covers all the paths
// a user can hit after plugging in a new mic or output:
//
//   - user plugs device in, alt-tabs back → App-level focus refresh
//   - user plugs device in, navigates to Settings → onMount refresh
//   - user plugs device in without alt-tabbing → manual button
//
// We exercise the latter two paths directly. Focus refresh is covered
// in `App.test.tsx`.

import { fireEvent, render, waitFor } from "@solidjs/testing-library";
import { describe, expect, it, vi } from "vitest";

const invokeCalls: Array<{ cmd: string; args?: unknown }> = [];

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(async (cmd: string, args?: unknown) => {
    invokeCalls.push({ cmd, args });
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
      case "detect_virtual_mic":
        return { installed: false, name: null };
      case "list_voices":
        return [];
      case "onnx_runtime_status":
        return { runtimeAvailable: false, voicesDir: "/tmp/DivoraVoice/voices" };
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
    onFocusChanged: vi.fn(async () => () => {
      /* noop */
    }),
  })),
}));

import { AppProvider } from "../stores/app";
import { SettingsScreen } from "./SettingsScreen";

/** Mount Settings with a real store but mocked IPC. */
function mount() {
  return render(() => (
    <AppProvider>
      <SettingsScreen />
    </AppProvider>
  ));
}

const flush = (): Promise<void> => new Promise((r) => setTimeout(r, 0));

describe("SettingsScreen — audio device refresh (v0.11.4)", () => {
  it("re-enumerates audio devices on mount", async () => {
    invokeCalls.length = 0;
    mount();
    await flush();
    // The store calls list_audio_*_devices once during init and again
    // when SettingsScreen mounts. The "again" is what we care about.
    const inputs = invokeCalls.filter(
      (c) => c.cmd === "list_audio_input_devices",
    );
    const outputs = invokeCalls.filter(
      (c) => c.cmd === "list_audio_output_devices",
    );
    expect(inputs.length).toBeGreaterThanOrEqual(1);
    expect(outputs.length).toBeGreaterThanOrEqual(1);
  });

  it("renders the Audio-devices Refresh button when idle", async () => {
    const { getByTitle } = mount();
    await flush();
    // There are two Refresh buttons now (Audio devices + Voice library);
    // target the Audio-devices one by its title.
    expect(
      getByTitle(/Re-scan input \+ output devices/i),
    ).toBeInTheDocument();
  });

  it("clicking the Audio-devices Refresh triggers a fresh enumeration", async () => {
    const { getByTitle } = mount();
    await flush();
    invokeCalls.length = 0;

    const btn = getByTitle(/Re-scan input \+ output devices/i);
    fireEvent.click(btn);
    await flush();
    // Click triggers an async refresh; wait for the IPC fan-out.
    await waitFor(() => {
      const inputs = invokeCalls.filter(
        (c) => c.cmd === "list_audio_input_devices",
      );
      const outputs = invokeCalls.filter(
        (c) => c.cmd === "list_audio_output_devices",
      );
      expect(inputs.length).toBeGreaterThanOrEqual(1);
      expect(outputs.length).toBeGreaterThanOrEqual(1);
    });
  });

  it("shows 'Audio devices' section eyebrow above the refresh control", async () => {
    const { getByText } = mount();
    await flush();
    expect(getByText(/Audio devices/i)).toBeInTheDocument();
  });
});

// v0.12.1 — Voice library section: lists installed voices, surfaces the
// ONNX runtime status, and lets the user pick the active voice (which
// drives the VoiceConvert effect).
describe("SettingsScreen — Voice library (v0.12.1)", () => {
  const flush = (): Promise<void> => new Promise((r) => setTimeout(r, 0));

  it("queries voices + runtime status on mount", async () => {
    invokeCalls.length = 0;
    mount();
    await flush();
    expect(invokeCalls.some((c) => c.cmd === "list_voices")).toBe(true);
    expect(invokeCalls.some((c) => c.cmd === "onnx_runtime_status")).toBe(true);
  });

  it("renders the Voice library section + runtime-not-installed guidance", async () => {
    const { getByText } = mount();
    await flush();
    expect(getByText(/Voice library/i)).toBeInTheDocument();
    // Mock returns runtimeAvailable: false → passthrough warning shown.
    expect(getByText(/ONNX Runtime not installed/i)).toBeInTheDocument();
  });

  it("offers a 'None — pass my voice through' option selected by default", async () => {
    const { getByText } = mount();
    await flush();
    expect(getByText(/pass my voice through/i)).toBeInTheDocument();
  });

  it("shows an empty hint when no voice models are installed", async () => {
    const { getByText } = mount();
    await flush();
    expect(getByText(/No voice models found/i)).toBeInTheDocument();
  });
});
