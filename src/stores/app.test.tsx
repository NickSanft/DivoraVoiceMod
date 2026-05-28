import { beforeEach, describe, expect, it, vi } from "vitest";
import { renderHook } from "@solidjs/testing-library";

const invokeMock = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
}));
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(async () => () => {
    /* unlisten no-op */
  }),
}));

import { AppProvider, useApp } from "./app";

function setupApp() {
  return renderHook(() => useApp(), { wrapper: AppProvider });
}

describe("app store — Phase 1", () => {
  beforeEach(() => {
    invokeMock.mockReset();
  });

  it("starts on the Mixer tab", () => {
    const { result } = setupApp();
    expect(result.nav()).toBe("mixer");
  });

  it("switches nav when setNav is called", () => {
    const { result } = setupApp();
    result.setNav("settings");
    expect(result.nav()).toBe("settings");
  });

  it("starts on a preset that exists", () => {
    const { result } = setupApp();
    expect(result.preset().id).toBe(result.presetId());
  });

  it("defaults to clean status (no PTM pressed, not muted)", () => {
    const { result } = setupApp();
    expect(result.status()).toBe("clean");
  });

  it("becomes modulated when PTM is pressed (apply mode) and there are enabled effects", () => {
    const { result } = setupApp();
    expect(result.hasEnabled()).toBe(true);
    result.setUi("pressed", true);
    expect(result.status()).toBe("modulated");
  });

  it("becomes muted when muted flag is set, overriding PTM", () => {
    const { result } = setupApp();
    result.setUi("pressed", true);
    result.setUi("muted", true);
    expect(result.status()).toBe("muted");
  });

  it("PTM bypass mode inverts the pressed semantics", () => {
    const { result } = setupApp();
    result.setUi("ptmMode", "bypass");
    expect(result.status()).toBe("modulated");
    result.setUi("pressed", true);
    expect(result.status()).toBe("clean");
  });
});

describe("app store — Phase 2 audio actions", () => {
  beforeEach(() => {
    invokeMock.mockReset();
  });

  it("refreshDevices populates input + output lists and pre-selects defaults", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "list_audio_input_devices") {
        return [
          { name: "Mic A", isDefault: false, defaultSampleRate: 48000, channels: 1 },
          { name: "Mic B", isDefault: true, defaultSampleRate: 48000, channels: 1 },
        ];
      }
      if (cmd === "list_audio_output_devices") {
        return [
          { name: "Headphones", isDefault: true, defaultSampleRate: 48000, channels: 2 },
        ];
      }
      return null;
    });
    const { result } = setupApp();
    await result.refreshDevices();
    expect(result.audioInputs()).toHaveLength(2);
    expect(result.audioOutputs()).toHaveLength(1);
    expect(result.selectedInput()).toBe("Mic B");
    expect(result.selectedOutput()).toBe("Headphones");
  });

  it("startEngine records StreamInfo + sets running on success", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "list_audio_input_devices") return [];
      if (cmd === "list_audio_output_devices") return [];
      if (cmd === "start_audio_engine") {
        return {
          inputName: "Mic A",
          outputName: "Headphones",
          sampleRate: 48000,
          inputChannels: 1,
          outputChannels: 2,
        };
      }
      return null;
    });
    const { result } = setupApp();
    await result.startEngine();
    expect(result.engineRunning()).toBe(true);
    expect(result.streamInfo()?.sampleRate).toBe(48000);
    expect(result.engineError()).toBeNull();
  });

  it("startEngine records the error and stays stopped on failure", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "start_audio_engine") {
        throw new Error("no input device");
      }
      return null;
    });
    const { result } = setupApp();
    await result.startEngine();
    expect(result.engineRunning()).toBe(false);
    expect(result.engineError()).toContain("no input device");
  });

  it("stopEngine resets running flag and zeros levels", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "start_audio_engine") {
        return {
          inputName: "Mic",
          outputName: "Headphones",
          sampleRate: 48000,
          inputChannels: 1,
          outputChannels: 2,
        };
      }
      return null;
    });
    const { result } = setupApp();
    await result.startEngine();
    result.setInputLevels({ rms: 0.5, peak: 0.7 });
    result.setOutputLevels({ rms: 0.4, peak: 0.6 });
    await result.stopEngine();
    expect(result.engineRunning()).toBe(false);
    expect(result.inputLevels()).toEqual({ rms: 0, peak: 0 });
    expect(result.outputLevels()).toEqual({ rms: 0, peak: 0 });
  });

  it("setMonitor forwards to backend and updates local flag", async () => {
    invokeMock.mockResolvedValue(undefined);
    const { result } = setupApp();
    await result.setMonitor(false);
    expect(invokeMock).toHaveBeenCalledWith("set_audio_monitor", { enabled: false });
    expect(result.engineMonitoring()).toBe(false);
  });

  it("toggleMonitor flips the current state", async () => {
    invokeMock.mockResolvedValue(undefined);
    const { result } = setupApp();
    expect(result.engineMonitoring()).toBe(true);
    await result.toggleMonitor();
    expect(result.engineMonitoring()).toBe(false);
    await result.toggleMonitor();
    expect(result.engineMonitoring()).toBe(true);
  });
});
