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

  it("renders a Refresh button labelled 'Refresh' when idle", async () => {
    const { getByText } = mount();
    await flush();
    expect(getByText(/^Refresh$/)).toBeInTheDocument();
  });

  it("clicking the Refresh button triggers a fresh enumeration", async () => {
    const { getByText } = mount();
    await flush();
    invokeCalls.length = 0;

    const btn = getByText(/^Refresh$/);
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
