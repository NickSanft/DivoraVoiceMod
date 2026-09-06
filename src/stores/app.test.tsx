import { beforeEach, describe, expect, it, vi } from "vitest";
import { renderHook } from "@solidjs/testing-library";
import { makeTemplate } from "../data/unistroke";

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
    // Device selections now persist to localStorage; clear it so a
    // selection set in one test doesn't leak into the next.
    try {
      window.localStorage.clear();
    } catch {
      /* jsdom always has localStorage */
    }
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

  // v1.1.1: device selections persist across restarts (bug fix).
  it("persists device selections to localStorage", () => {
    const { result } = setupApp();
    result.setSelectedInput("Mic A");
    result.setSelectedOutput("Headphones");
    expect(window.localStorage.getItem("divora.inputDevice")).toContain("Mic A");
    expect(window.localStorage.getItem("divora.outputDevice")).toContain(
      "Headphones",
    );
  });

  it("restores persisted device selections on a fresh store", () => {
    window.localStorage.setItem("divora.inputDevice", JSON.stringify("Saved Mic"));
    window.localStorage.setItem("divora.outputDevice", JSON.stringify("Saved Out"));
    const { result } = setupApp();
    expect(result.selectedInput()).toBe("Saved Mic");
    expect(result.selectedOutput()).toBe("Saved Out");
  });

  it("refreshDevices keeps a valid restored device, falls back when it's gone", async () => {
    window.localStorage.setItem("divora.inputDevice", JSON.stringify("Mic A"));
    window.localStorage.setItem(
      "divora.outputDevice",
      JSON.stringify("Unplugged Output"),
    );
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "list_audio_input_devices") {
        return [
          { name: "Mic A", isDefault: false, defaultSampleRate: 48000, channels: 1 },
          { name: "Mic B", isDefault: true, defaultSampleRate: 48000, channels: 1 },
        ];
      }
      if (cmd === "list_audio_output_devices") {
        return [
          { name: "Speakers", isDefault: true, defaultSampleRate: 48000, channels: 2 },
        ];
      }
      return null;
    });
    const { result } = setupApp();
    await result.refreshDevices();
    expect(result.selectedInput()).toBe("Mic A"); // still present → kept
    expect(result.selectedOutput()).toBe("Speakers"); // gone → default
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

  // ---- v1.6.0: monitor volume ------------------------------------------

  it("setMonitorGain clamps, persists, and forwards to the backend", () => {
    invokeMock.mockResolvedValue(undefined);
    const { result } = setupApp();
    expect(result.monitorGain()).toBe(1.0);

    result.setMonitorGain(1.5);
    expect(result.monitorGain()).toBe(1.5);
    expect(invokeMock).toHaveBeenCalledWith("set_monitor_gain", { gain: 1.5 });
    expect(window.localStorage.getItem("divora.monitorGain")).toBe("1.5");

    // Above the 4.0 ceiling clamps down; below 0 clamps up.
    result.setMonitorGain(99);
    expect(result.monitorGain()).toBe(4);
    result.setMonitorGain(-1);
    expect(result.monitorGain()).toBe(0);
  });

  // ---- v1.7.0: loudness normalization ----------------------------------

  it("setLoudnessEnabled persists and forwards to the backend", () => {
    invokeMock.mockResolvedValue(undefined);
    const { result } = setupApp();
    expect(result.loudnessEnabled()).toBe(false); // opt-in default

    result.setLoudnessEnabled(true);
    expect(result.loudnessEnabled()).toBe(true);
    expect(invokeMock).toHaveBeenCalledWith("set_loudness_enabled", {
      enabled: true,
    });
    expect(window.localStorage.getItem("divora.loudnessEnabled")).toBe("true");
  });

  it("setLoudnessTarget clamps to [-30, -6], persists, and forwards", () => {
    invokeMock.mockResolvedValue(undefined);
    const { result } = setupApp();
    expect(result.loudnessTarget()).toBe(-18); // default target

    result.setLoudnessTarget(-22);
    expect(result.loudnessTarget()).toBe(-22);
    expect(invokeMock).toHaveBeenCalledWith("set_loudness_target", {
      dbfs: -22,
    });
    expect(window.localStorage.getItem("divora.loudnessTarget")).toBe("-22");

    // Below the −30 floor clamps up; above the −6 ceiling clamps down.
    result.setLoudnessTarget(-100);
    expect(result.loudnessTarget()).toBe(-30);
    result.setLoudnessTarget(0);
    expect(result.loudnessTarget()).toBe(-6);
  });

  // ---- Phase 16: recording ---------------------------------------------

  it("toggleRecording refuses to start while the engine is stopped", async () => {
    invokeMock.mockResolvedValue(undefined);
    const { result } = setupApp();
    expect(result.engineRunning()).toBe(false);
    await result.toggleRecording();
    expect(result.isRecording()).toBe(false);
    expect(result.engineError()).toContain("Start the engine");
    expect(invokeMock).not.toHaveBeenCalledWith("start_recording", expect.anything());
  });

  it("toggleRecording starts then stops recording while the engine runs", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "start_audio_engine") {
        return {
          inputName: "Mic",
          outputName: "CABLE Input",
          sampleRate: 48000,
          inputChannels: 1,
          outputChannels: 2,
        };
      }
      if (cmd === "start_recording") return "C:/rec/divora-test.wav";
      return null;
    });
    const { result } = setupApp();
    await result.startEngine();
    expect(result.engineRunning()).toBe(true);

    await result.toggleRecording();
    expect(invokeMock).toHaveBeenCalledWith(
      "start_recording",
      expect.objectContaining({
        filename: expect.stringMatching(/^divora-.*\.wav$/),
      }),
    );
    expect(result.isRecording()).toBe(true);
    expect(result.recordingPath()).toBe("C:/rec/divora-test.wav");

    await result.toggleRecording();
    expect(invokeMock).toHaveBeenCalledWith("stop_recording");
    expect(result.isRecording()).toBe(false);
  });

  // ---- Phase 11: live device switching ---------------------------------

  it("changing selectedInput while running restarts the engine on the new device", async () => {
    invokeMock.mockImplementation(async (cmd: string, args?: unknown) => {
      if (cmd === "start_audio_engine") {
        const a = args as { inputName: string | null; outputName: string | null };
        return {
          inputName: a.inputName ?? "?",
          outputName: a.outputName ?? "?",
          sampleRate: 48000,
          inputChannels: 1,
          outputChannels: 2,
        };
      }
      return null;
    });
    const { result } = setupApp();
    result.setSelectedInput("Mic A");
    result.setSelectedOutput("Headphones");
    await result.startEngine();
    expect(result.engineRunning()).toBe(true);

    invokeMock.mockClear();
    result.setSelectedInput("Mic B");
    // Settle the createEffect microtask + the inner async chain.
    await new Promise<void>((r) => setTimeout(r, 0));
    await new Promise<void>((r) => setTimeout(r, 0));
    expect(invokeMock).toHaveBeenCalledWith("stop_audio_engine");
    expect(invokeMock).toHaveBeenCalledWith(
      "start_audio_engine",
      expect.objectContaining({ inputName: "Mic B" }),
    );
  });

  it("changing selectedOutput while running restarts the engine on the new device", async () => {
    invokeMock.mockImplementation(async (cmd: string, args?: unknown) => {
      if (cmd === "start_audio_engine") {
        const a = args as { inputName: string | null; outputName: string | null };
        return {
          inputName: a.inputName ?? "?",
          outputName: a.outputName ?? "?",
          sampleRate: 48000,
          inputChannels: 1,
          outputChannels: 2,
        };
      }
      return null;
    });
    const { result } = setupApp();
    result.setSelectedInput("Mic A");
    result.setSelectedOutput("Headphones");
    await result.startEngine();
    invokeMock.mockClear();
    result.setSelectedOutput("CABLE Input (VB-Audio Virtual Cable)");
    await new Promise<void>((r) => setTimeout(r, 0));
    await new Promise<void>((r) => setTimeout(r, 0));
    expect(invokeMock).toHaveBeenCalledWith("stop_audio_engine");
    expect(invokeMock).toHaveBeenCalledWith(
      "start_audio_engine",
      expect.objectContaining({
        outputName: "CABLE Input (VB-Audio Virtual Cable)",
      }),
    );
  });

  it("restarts on the FIRST device change of the session while running (defer prev-undefined regression)", async () => {
    // Regression (v1.4.1): with on(..., { defer: true }), Solid passes
    // prev=undefined on the first post-mount change. The old guard
    // `if (prev === undefined) return` swallowed the very first device
    // switch — so picking a monitor took effect only on the *second* try.
    // No device is changed before startEngine here, so the monitor change
    // below IS the session's first — it must restart the engine.
    invokeMock.mockImplementation(async (cmd: string, args?: unknown) => {
      if (cmd === "start_audio_engine") {
        const a = args as { monitorName: string | null };
        return {
          inputName: "in",
          outputName: "out",
          monitorName: a.monitorName,
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
    invokeMock.mockClear();
    result.setSelectedMonitor("Headphones"); // first device change since mount
    await new Promise<void>((r) => setTimeout(r, 0));
    await new Promise<void>((r) => setTimeout(r, 0));
    expect(invokeMock).toHaveBeenCalledWith("stop_audio_engine");
    expect(invokeMock).toHaveBeenCalledWith(
      "start_audio_engine",
      expect.objectContaining({ monitorName: "Headphones" }),
    );
  });

  it("startEngine passes the selected monitor device (Phase 13)", async () => {
    invokeMock.mockImplementation(async (cmd: string, args?: unknown) => {
      if (cmd === "start_audio_engine") {
        const a = args as {
          inputName: string | null;
          outputName: string | null;
          monitorName: string | null;
        };
        return {
          inputName: a.inputName ?? "?",
          outputName: a.outputName ?? "?",
          monitorName: a.monitorName,
          sampleRate: 48000,
          inputChannels: 1,
          outputChannels: 2,
        };
      }
      return null;
    });
    const { result } = setupApp();
    result.setSelectedInput("Mic A");
    result.setSelectedOutput("CABLE Input (VB-Audio Virtual Cable)");
    result.setSelectedMonitor("Headphones");
    await result.startEngine();
    expect(invokeMock).toHaveBeenCalledWith(
      "start_audio_engine",
      expect.objectContaining({
        outputName: "CABLE Input (VB-Audio Virtual Cable)",
        monitorName: "Headphones",
      }),
    );
  });

  it("changing selectedMonitor while running restarts the engine (Phase 13)", async () => {
    invokeMock.mockImplementation(async (cmd: string, args?: unknown) => {
      if (cmd === "start_audio_engine") {
        const a = args as { monitorName: string | null };
        return {
          inputName: "Mic A",
          outputName: "CABLE",
          monitorName: a.monitorName,
          sampleRate: 48000,
          inputChannels: 1,
          outputChannels: 2,
        };
      }
      return null;
    });
    const { result } = setupApp();
    result.setSelectedInput("Mic A");
    result.setSelectedOutput("CABLE");
    // Reset to a known baseline first — localStorage persists across
    // tests in this file, so a prior test may have left a monitor set.
    result.setSelectedMonitor(null);
    await result.startEngine();
    invokeMock.mockClear();
    result.setSelectedMonitor("Headphones");
    await new Promise<void>((r) => setTimeout(r, 0));
    await new Promise<void>((r) => setTimeout(r, 0));
    expect(invokeMock).toHaveBeenCalledWith("stop_audio_engine");
    expect(invokeMock).toHaveBeenCalledWith(
      "start_audio_engine",
      expect.objectContaining({ monitorName: "Headphones" }),
    );
  });

  it("setSelectedMonitor persists to localStorage (Phase 13)", () => {
    const { result } = setupApp();
    result.setSelectedMonitor("Headphones");
    expect(window.localStorage.getItem("divora.monitorDevice")).toContain(
      "Headphones",
    );
    // Clearing back to null persists null (no separate monitor).
    result.setSelectedMonitor(null);
    expect(window.localStorage.getItem("divora.monitorDevice")).toBe("null");
  });

  it("device-change effect is a no-op when the engine is stopped", async () => {
    invokeMock.mockImplementation(async () => null);
    const { result } = setupApp();
    result.setSelectedInput("Mic A");
    // Engine has never been started; engineRunning() is false.
    invokeMock.mockClear();
    result.setSelectedInput("Mic B");
    await new Promise<void>((r) => setTimeout(r, 0));
    await new Promise<void>((r) => setTimeout(r, 0));
    expect(invokeMock).not.toHaveBeenCalledWith("stop_audio_engine");
    expect(invokeMock).not.toHaveBeenCalledWith(
      "start_audio_engine",
      expect.anything(),
    );
  });

  it("device-change effect is a no-op when nothing actually changed", async () => {
    invokeMock.mockImplementation(async (cmd: string, args?: unknown) => {
      if (cmd === "start_audio_engine") {
        const a = args as { inputName: string | null; outputName: string | null };
        return {
          inputName: a.inputName ?? "?",
          outputName: a.outputName ?? "?",
          sampleRate: 48000,
          inputChannels: 1,
          outputChannels: 2,
        };
      }
      return null;
    });
    const { result } = setupApp();
    result.setSelectedInput("Mic A");
    await result.startEngine();
    invokeMock.mockClear();
    // Re-set the SAME value: no restart should happen.
    result.setSelectedInput("Mic A");
    await new Promise<void>((r) => setTimeout(r, 0));
    await new Promise<void>((r) => setTimeout(r, 0));
    expect(invokeMock).not.toHaveBeenCalledWith("stop_audio_engine");
  });
});

describe("app store — Phase 4 preset actions", () => {
  beforeEach(() => {
    invokeMock.mockReset();
    invokeMock.mockResolvedValue(undefined);
    // The active preset now persists to localStorage; clear it so a
    // `usePreset` in one test doesn't restore into the next.
    try {
      window.localStorage.clear();
    } catch {
      /* jsdom always has localStorage */
    }
  });

  it("refreshPresets replaces the list with the backend response", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "list_presets") {
        return [
          {
            id: "my-voice",
            version: 1,
            name: "My Voice",
            color: "#34D9A0",
            glyph: "eq",
            tag: "User",
            desc: "Custom.",
            chain: [
              { id: "gate", enabled: true, vals: { thresh: -45 } },
            ],
          },
        ];
      }
      return null;
    });
    const { result } = setupApp();
    expect(result.presetsLoaded()).toBe(false);
    await result.refreshPresets();
    expect(result.presetsLoaded()).toBe(true);
    expect(result.presets()).toHaveLength(1);
    expect(result.presets()[0]!.id).toBe("my-voice");
  });

  it("refreshPresets keeps the fallback when the backend response is empty", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "list_presets") return [];
      return null;
    });
    const { result } = setupApp();
    const fallbackCount = result.presets().length;
    await result.refreshPresets();
    expect(result.presets().length).toBe(fallbackCount);
    expect(result.presetsLoaded()).toBe(false);
  });

  it("usePreset switches the active id and resets A/B to A", () => {
    const { result } = setupApp();
    const other = result
      .presets()
      .find((p) => p.id !== result.presetId());
    expect(other).toBeDefined();
    result.setUi("ab", "B");
    result.usePreset(other!.id);
    expect(result.presetId()).toBe(other!.id);
    expect(result.ui.ab).toBe("A");
  });

  // ---- Active preset persists across restarts (bug fix) ----------------

  it("persists the active preset to localStorage", () => {
    const { result } = setupApp();
    const other = result.presets().find((p) => p.id !== result.presetId());
    expect(other).toBeDefined();
    result.usePreset(other!.id);
    expect(window.localStorage.getItem("divora.activePreset")).toContain(
      other!.id,
    );
  });

  it("restores the persisted active preset on a fresh store", () => {
    // Discover a real, non-default bundled preset id (defer means mounting
    // the probe store doesn't itself write to localStorage).
    const probe = setupApp();
    const target = probe.result
      .presets()
      .find((p) => p.id !== probe.result.presetId());
    expect(target).toBeDefined();
    window.localStorage.setItem(
      "divora.activePreset",
      JSON.stringify(target!.id),
    );
    const { result } = setupApp();
    expect(result.presetId()).toBe(target!.id);
    expect(result.viewedId()).toBe(target!.id); // editor opens on the active one
  });

  it("falls back when the persisted active preset no longer exists", async () => {
    window.localStorage.setItem(
      "divora.activePreset",
      JSON.stringify("ghost-deleted-user-preset"),
    );
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "list_presets") {
        return [
          {
            id: "my-voice",
            version: 1,
            name: "My Voice",
            color: "#34D9A0",
            glyph: "eq",
            tag: "User",
            desc: "Custom.",
            chain: [{ id: "gate", enabled: true, vals: { thresh: -45 } }],
          },
        ];
      }
      return null;
    });
    const { result } = setupApp();
    expect(result.presetId()).toBe("ghost-deleted-user-preset"); // optimistic restore
    await result.refreshPresets();
    // The id isn't in the real list → fall back to a valid preset so the
    // engine never runs an empty chain.
    expect(result.presets().some((p) => p.id === result.presetId())).toBe(true);
    expect(result.presetId()).not.toBe("ghost-deleted-user-preset");
    expect(result.viewedId()).toBe(result.presetId());
  });

  // ---- Presets: select vs. use (v1.6.0) --------------------------------

  it("viewPreset previews without changing the active preset; Use applies", () => {
    const { result } = setupApp();
    const active = result.presetId();
    const other = result.presets().find((p) => p.id !== active)!;
    result.viewPreset(other.id);
    expect(result.viewedId()).toBe(other.id); // editor shows the previewed one
    expect(result.presetId()).toBe(active); // live voice unchanged
    result.usePreset(other.id); // the "Use" button
    expect(result.presetId()).toBe(other.id);
    expect(result.viewedId()).toBe(other.id);
  });

  it("editing a previewed (non-active) preset does not touch the engine", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "start_audio_engine") {
        return {
          inputName: "i",
          outputName: "o",
          sampleRate: 48000,
          inputChannels: 1,
          outputChannels: 2,
        };
      }
      return null;
    });
    const { result } = setupApp();
    await result.startEngine();
    const active = result.presetId();
    const other = result.presets().find((p) => p.id !== active)!;
    result.viewPreset(other.id);
    invokeMock.mockClear();
    result.setChainParam(0, "thresh", -33);
    // The previewed preset's chain updated locally...
    expect(result.viewedChain()[0]!.vals.thresh).toBe(-33);
    // ...but the live engine was not told (it isn't the active preset).
    expect(invokeMock).not.toHaveBeenCalledWith("set_effect_param", expect.anything());
  });

  it("leaving the Presets screen resyncs the viewed preset to the active one", async () => {
    const { result } = setupApp();
    const active = result.presetId();
    const other = result.presets().find((p) => p.id !== active)!;
    result.setNav("presets");
    result.viewPreset(other.id);
    expect(result.viewedId()).toBe(other.id);
    result.setNav("mixer"); // leaving Presets
    await new Promise<void>((r) => setTimeout(r, 0));
    expect(result.viewedId()).toBe(active); // back in sync with the live voice
  });

  // ---- The Coven (v1.1.0) ----------------------------------------------

  it("summon applies a DSP cast member's preset and clears any active model", () => {
    const { result } = setupApp();
    result.setActiveVoice("some-model"); // pretend a model was selected
    result.summon("velvet-demon"); // DSP character → no model
    expect(result.presetId()).toBe("velvet-demon");
    expect(result.activeVoiceId()).toBeNull();
  });

  it("summon applies the narrator preset and loads its conversion model", () => {
    const { result } = setupApp();
    result.summon("deep-narrator-ai", "llvc-narrator");
    expect(result.presetId()).toBe("deep-narrator-ai");
    expect(result.activeVoiceId()).toBe("llvc-narrator");
  });

  it("setAbSlot snapshots the current chain into the old slot and swaps", () => {
    const { result } = setupApp();
    const idx = result.chain().findIndex((c) => c.id === "gate");
    // Edit in slot A
    result.setChainParam(idx, "thresh", -30);
    expect(result.chain()[idx]!.vals.thresh).toBe(-30);
    // Switch to slot B — A snapshot captures the -30 edit
    result.setAbSlot("B");
    expect(result.ui.ab).toBe("B");
    // The chain shown is now slot B (which started equal to the
    // preset defaults), so the gate threshold is back to the bundled
    // value, not -30.
    expect(result.chain()[idx]!.vals.thresh).not.toBe(-30);
    // Edit in slot B
    result.setChainParam(idx, "thresh", -20);
    // Toggle back to A — restores -30
    result.setAbSlot("A");
    expect(result.chain()[idx]!.vals.thresh).toBe(-30);
  });

  it("savePreset rejects bundled presets and forwards user ones", async () => {
    const { result } = setupApp();
    const bundled = result.presets().find((p) => p.tag === "Bundled")!;
    await expect(result.savePreset(bundled)).rejects.toThrow(/bundled/i);

    const user = {
      ...bundled,
      id: "my-copy",
      name: "My Copy",
      tag: "User" as const,
    };
    await result.savePreset(user);
    expect(invokeMock).toHaveBeenCalledWith(
      "save_user_preset",
      expect.objectContaining({
        preset: expect.objectContaining({ id: "my-copy", tag: "User" }),
      }),
    );
    expect(result.presets().some((p) => p.id === "my-copy")).toBe(true);
  });

  it("duplicatePreset writes a new user preset with a slugged id", async () => {
    const { result } = setupApp();
    const before = new Set(result.presets().map((p) => p.id));
    const source = result.preset();
    const copy = await result.duplicatePreset(source.id);
    expect(copy).not.toBeNull();
    expect(copy!.tag).toBe("User");
    expect(copy!.name.toLowerCase()).toContain("copy");
    expect(before.has(copy!.id)).toBe(false);
    expect(result.presets().some((p) => p.id === copy!.id)).toBe(true);
  });

  it("deletePreset rejects bundled and removes user", async () => {
    const { result } = setupApp();
    const bundled = result.presets().find((p) => p.tag === "Bundled")!;
    await expect(result.deletePreset(bundled.id)).rejects.toThrow(/bundled/i);

    // Add a user preset, then delete it
    const user = {
      ...bundled,
      id: "deletable",
      name: "Deletable",
      tag: "User" as const,
    };
    await result.savePreset(user);
    expect(result.presets().some((p) => p.id === "deletable")).toBe(true);
    await result.deletePreset("deletable");
    expect(invokeMock).toHaveBeenCalledWith("delete_user_preset", {
      id: "deletable",
    });
    expect(result.presets().some((p) => p.id === "deletable")).toBe(false);
  });

  it("exportPreset returns the backend-serialised JSON", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "export_preset_json") return "{ exported }";
      return null;
    });
    const { result } = setupApp();
    const out = await result.exportPreset(result.preset());
    expect(invokeMock).toHaveBeenCalledWith(
      "export_preset_json",
      expect.objectContaining({ preset: expect.any(Object) }),
    );
    expect(out).toBe("{ exported }");
  });

  it("exportPreset falls back to JSON.stringify when backend is unreachable", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "export_preset_json") throw new Error("no tauri");
      return null;
    });
    const { result } = setupApp();
    const out = await result.exportPreset(result.preset());
    expect(out).toContain(result.preset().id);
    expect(out).toContain(result.preset().name);
  });

  it("reorderChainEntries swaps positions in the active chain", () => {
    const { result } = setupApp();
    const before = result.chain().map((c) => c.id);
    if (before.length < 2) return; // safety
    result.reorderChainEntries(0, 1);
    const after = result.chain().map((c) => c.id);
    expect(after[0]).toBe(before[1]);
    expect(after[1]).toBe(before[0]);
  });

  it("reorderChainEntries is a no-op when from === to", () => {
    const { result } = setupApp();
    const before = result.chain().map((c) => c.id);
    result.reorderChainEntries(1, 1);
    const after = result.chain().map((c) => c.id);
    expect(after).toEqual(before);
  });

  it("reorderChainEntries clamps out-of-range indices to a no-op", () => {
    const { result } = setupApp();
    const before = result.chain().map((c) => c.id);
    result.reorderChainEntries(-1, 0);
    result.reorderChainEntries(0, 99);
    const after = result.chain().map((c) => c.id);
    expect(after).toEqual(before);
  });

  it("resetAbSlots makes both slots match the current chain", () => {
    const { result } = setupApp();
    const id = result.presetId();
    const idx = result.chain().findIndex((c) => c.id === "gate");
    result.setChainParam(idx, "thresh", -25);
    result.resetAbSlots();
    expect(result.abSlots[id]!.A[idx]!.vals.thresh).toBe(-25);
    expect(result.abSlots[id]!.B[idx]!.vals.thresh).toBe(-25);
    expect(result.ui.ab).toBe("A");
  });
});

describe("app store — Phase 5 soundboard actions", () => {
  beforeEach(() => {
    invokeMock.mockReset();
    invokeMock.mockResolvedValue(undefined);
  });

  function sampleTile(id: string, label = `clip-${id}`) {
    return {
      id,
      path: `/tmp/${label}.wav`,
      label,
      extension: "wav",
      sizeBytes: 1024,
    };
  }

  it("scanCurrentSoundboardFolder is a no-op when no folder is picked", async () => {
    const { result } = setupApp();
    invokeMock.mockClear();
    await result.scanCurrentSoundboardFolder();
    expect(invokeMock).not.toHaveBeenCalledWith(
      "scan_soundboard_folder",
      expect.anything(),
    );
  });

  it("scanCurrentSoundboardFolder records tiles + clears error", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "scan_soundboard_folder") {
        return [sampleTile("a"), sampleTile("b")];
      }
      return null;
    });
    const { result } = setupApp();
    result.setSoundboardFolder("/tmp/sb");
    result.setSoundboardError("stale");
    await result.scanCurrentSoundboardFolder();
    expect(result.soundboardTiles()).toHaveLength(2);
    expect(result.soundboardError()).toBeNull();
  });

  it("scanCurrentSoundboardFolder records error on failure", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "scan_soundboard_folder") throw new Error("no folder");
      return null;
    });
    const { result } = setupApp();
    result.setSoundboardFolder("/tmp/sb");
    await result.scanCurrentSoundboardFolder();
    expect(result.soundboardError()).toContain("no folder");
    expect(result.soundboardTiles()).toEqual([]);
  });

  it("playClip records a PlayingClip with duration", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "play_soundboard_clip") return 4.2;
      return null;
    });
    const { result } = setupApp();
    const tile = sampleTile("bell");
    await result.playClip(tile);
    expect(invokeMock).toHaveBeenCalledWith("play_soundboard_clip", {
      clipId: "bell",
      path: tile.path,
      gain: 1,
    });
    expect(result.playingClips[tile.id]).toBeDefined();
    expect(result.playingClips[tile.id]!.durationSecs).toBe(4.2);
  });

  it("playClip surfaces backend errors via soundboardError", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "play_soundboard_clip") throw new Error("file missing");
      return null;
    });
    const { result } = setupApp();
    await result.playClip(sampleTile("ghost"));
    expect(result.soundboardError()).toContain("file missing");
  });

  it("stopClip clears the playingClips entry even if backend errors", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "play_soundboard_clip") return 1.0;
      if (cmd === "stop_soundboard_clip") throw new Error("nope");
      return null;
    });
    const { result } = setupApp();
    const tile = sampleTile("z");
    await result.playClip(tile);
    expect(result.playingClips[tile.id]).toBeDefined();
    await result.stopClip(tile.id);
    expect(result.playingClips[tile.id]).toBeUndefined();
  });

  it("panicSoundboard clears every playing clip", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "play_soundboard_clip") return 1.0;
      if (cmd === "stop_all_soundboard_clips") return undefined;
      return null;
    });
    const { result } = setupApp();
    await result.playClip(sampleTile("a"));
    await result.playClip(sampleTile("b"));
    await result.playClip(sampleTile("c"));
    expect(Object.keys(result.playingClips).length).toBe(3);
    await result.panicSoundboard();
    expect(Object.keys(result.playingClips).length).toBe(0);
  });

  it("markClipFinished removes a single clip", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "play_soundboard_clip") return 1.0;
      return null;
    });
    const { result } = setupApp();
    await result.playClip(sampleTile("a"));
    expect(result.playingClips["a"]).toBeDefined();
    result.markClipFinished("a");
    expect(result.playingClips["a"]).toBeUndefined();
  });

  it("bindTileHotkey records the binding; clear removes it", async () => {
    const { result } = setupApp();
    await result.bindTileHotkey("a", ["F", "1"]);
    expect(result.tileHotkeys["a"]).toEqual(["F", "1"]);
    await result.clearTileHotkey("a");
    expect(result.tileHotkeys["a"]).toBeUndefined();
  });
});

describe("app store — Phase 8 soundboard polish", () => {
  beforeEach(() => {
    invokeMock.mockReset();
    invokeMock.mockResolvedValue(undefined);
    try {
      window.localStorage.clear();
    } catch {
      /* jsdom always has localStorage */
    }
  });

  function sampleTile(id: string, label = `clip-${id}`) {
    return {
      id,
      path: `/tmp/${label}.wav`,
      label,
      extension: "wav",
      sizeBytes: 1024,
    };
  }

  // --- sortedTiles + reorderTiles --------------------------------------

  it("sortedTiles passes the scan through unchanged when no tileOrder entry exists for the folder", () => {
    const { result } = setupApp();
    result.setSoundboardFolder("/sb");
    result.setSoundboardTiles([sampleTile("a"), sampleTile("b"), sampleTile("c")]);
    expect(result.sortedTiles().map((t) => t.id)).toEqual(["a", "b", "c"]);
  });

  it("reorderTiles moves a tile within the active folder and sortedTiles reflects it", () => {
    const { result } = setupApp();
    result.setSoundboardFolder("/sb");
    result.setSoundboardTiles([sampleTile("a"), sampleTile("b"), sampleTile("c")]);
    result.reorderTiles("/sb", 0, 2);
    expect(result.sortedTiles().map((t) => t.id)).toEqual(["b", "c", "a"]);
  });

  it("sortedTiles appends new tiles (not in the saved order) to the end", () => {
    const { result } = setupApp();
    result.setSoundboardFolder("/sb");
    // Seed an order saved by a prior session.
    result.setSoundboardTiles([sampleTile("a"), sampleTile("b")]);
    result.reorderTiles("/sb", 0, 1); // a → end → [b, a]
    // New file (id "c") shows up in this scan.
    result.setSoundboardTiles([
      sampleTile("a"),
      sampleTile("b"),
      sampleTile("c"),
    ]);
    expect(result.sortedTiles().map((t) => t.id)).toEqual(["b", "a", "c"]);
  });

  it("reorderTiles persists per folder so it can be restored on next session", () => {
    const { result } = setupApp();
    result.setSoundboardFolder("/sb");
    result.setSoundboardTiles([sampleTile("a"), sampleTile("b")]);
    result.reorderTiles("/sb", 0, 1);
    expect(window.localStorage.getItem("divora.tileOrder")).toContain("/sb");
  });

  it("reorderTiles is a no-op for out-of-range / equal indices", () => {
    const { result } = setupApp();
    result.setSoundboardFolder("/sb");
    result.setSoundboardTiles([sampleTile("a"), sampleTile("b")]);
    result.reorderTiles("/sb", -1, 0);
    result.reorderTiles("/sb", 1, 1);
    result.reorderTiles("/sb", 0, 99);
    expect(result.sortedTiles().map((t) => t.id)).toEqual(["a", "b"]);
  });

  // --- setTileColor ----------------------------------------------------

  it("setTileColor stores a color override and clears it on null", () => {
    const { result } = setupApp();
    result.setTileColor("clip-x", "#34D9A0");
    expect(result.tileColors["clip-x"]).toBe("#34D9A0");
    result.setTileColor("clip-x", null);
    expect(result.tileColors["clip-x"]).toBeUndefined();
  });

  it("setTileColor persists to localStorage", () => {
    const { result } = setupApp();
    result.setTileColor("clip-x", "#EC4899");
    expect(window.localStorage.getItem("divora.tileColors")).toContain("#EC4899");
  });

  // --- Phase 15: folder persistence + soundboard volume ---------------

  it("setSoundboardFolder persists to localStorage", () => {
    const { result } = setupApp();
    result.setSoundboardFolder("C:/clips");
    expect(window.localStorage.getItem("divora.soundboardFolder")).toContain(
      "C:/clips",
    );
  });

  it("tileGain defaults to 1.0 and setTileGain persists", () => {
    const { result } = setupApp();
    expect(result.tileGain("clip-x")).toBe(1.0);
    result.setTileGain("clip-x", 0.5);
    expect(result.tileGain("clip-x")).toBe(0.5);
    expect(window.localStorage.getItem("divora.tileGains")).toContain("0.5");
  });

  it("setSoundboardMasterGain persists + sends the backend command", () => {
    invokeMock.mockResolvedValue(undefined);
    const { result } = setupApp();
    result.setSoundboardMasterGain(0.75);
    expect(result.soundboardMasterGain()).toBe(0.75);
    expect(window.localStorage.getItem("divora.soundboardMasterGain")).toContain(
      "0.75",
    );
    expect(invokeMock).toHaveBeenCalledWith("set_soundboard_master_gain", {
      gain: 0.75,
    });
  });

  it("playClip passes the tile's gain to the backend", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "play_soundboard_clip") return 1.5; // duration
      return undefined;
    });
    const { result } = setupApp();
    result.setTileGain("clip-x", 0.25);
    await result.playClip({
      id: "clip-x",
      path: "C:/clips/x.wav",
      label: "x",
      extension: "wav",
      sizeBytes: 1,
    });
    expect(invokeMock).toHaveBeenCalledWith("play_soundboard_clip", {
      clipId: "clip-x",
      path: "C:/clips/x.wav",
      gain: 0.25,
    });
  });

  // --- recent folders --------------------------------------------------

  it("pushRecentFolder prepends and caps at 5", () => {
    const { result } = setupApp();
    for (const folder of ["A", "B", "C", "D", "E", "F"]) {
      result.pushRecentFolder(folder);
    }
    expect(result.recentFolders()).toEqual(["F", "E", "D", "C", "B"]);
  });

  it("pushRecentFolder moves an existing folder to the front (no duplicates)", () => {
    const { result } = setupApp();
    result.pushRecentFolder("A");
    result.pushRecentFolder("B");
    result.pushRecentFolder("A");
    expect(result.recentFolders()).toEqual(["A", "B"]);
  });

  it("removeRecentFolder drops the entry", () => {
    const { result } = setupApp();
    result.pushRecentFolder("A");
    result.pushRecentFolder("B");
    result.removeRecentFolder("A");
    expect(result.recentFolders()).toEqual(["B"]);
  });

  it("useRecentFolder switches the active folder and re-pushes it to the top of recents", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "scan_soundboard_folder") return [];
      return null;
    });
    const { result } = setupApp();
    result.pushRecentFolder("A");
    result.pushRecentFolder("B");
    await result.useRecentFolder("A");
    expect(result.soundboardFolder()).toBe("A");
    expect(result.recentFolders()).toEqual(["A", "B"]);
  });

  it("pickSoundboardFolder pushes the chosen folder onto recents", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "scan_soundboard_folder") return [];
      return null;
    });
    const { result } = setupApp();
    // Simulate the dialog plugin returning a path. The wrapper is
    // mocked by the dialog plugin's `open` returning the path; here
    // we shortcut by directly invoking the scan + push.
    result.setSoundboardFolder("/pick/me");
    result.pushRecentFolder("/pick/me");
    await result.scanCurrentSoundboardFolder();
    expect(result.recentFolders()).toEqual(["/pick/me"]);
  });

  // --- global tile hotkey ---------------------------------------------

  it("bindTileHotkey registers a global shortcut with the sb: prefix", async () => {
    const { result } = setupApp();
    invokeMock.mockClear();
    await result.bindTileHotkey("clip-bell", ["Ctrl", "1"]);
    expect(invokeMock).toHaveBeenCalledWith("register_global_shortcut", {
      id: "sb:clip-bell",
      accelerator: "Ctrl+1",
    });
  });

  it("bindTileHotkey with empty keys unregisters via the sb: prefix", async () => {
    const { result } = setupApp();
    await result.bindTileHotkey("clip-bell", ["Ctrl", "1"]);
    invokeMock.mockClear();
    await result.bindTileHotkey("clip-bell", []);
    expect(invokeMock).toHaveBeenCalledWith("unregister_global_shortcut", {
      id: "sb:clip-bell",
    });
    expect(result.tileHotkeys["clip-bell"]).toEqual([]);
  });

  it("clearTileHotkey unregisters the global shortcut", async () => {
    const { result } = setupApp();
    await result.bindTileHotkey("clip-bell", ["F2"]);
    invokeMock.mockClear();
    await result.clearTileHotkey("clip-bell");
    expect(invokeMock).toHaveBeenCalledWith("unregister_global_shortcut", {
      id: "sb:clip-bell",
    });
  });

  it("syncHotkeyBindings re-registers persisted tile hotkeys on startup", async () => {
    window.localStorage.setItem(
      "divora.tileHotkeys",
      JSON.stringify({ "clip-a": ["F1"], "clip-b": ["F2"] }),
    );
    const { result } = setupApp();
    invokeMock.mockClear();
    await result.syncHotkeyBindings();
    const calls = invokeMock.mock.calls.filter(
      (c) => c[0] === "register_global_shortcut",
    );
    const ids = calls.map((c) => (c[1] as { id: string }).id).sort();
    expect(ids).toEqual(["sb:clip-a", "sb:clip-b"]);
  });

  it("playTileById looks up the tile by id and forwards to playClip", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "play_soundboard_clip") return 2.0;
      return null;
    });
    const { result } = setupApp();
    result.setSoundboardTiles([sampleTile("clip-bell")]);
    await result.playTileById("clip-bell");
    expect(invokeMock).toHaveBeenCalledWith("play_soundboard_clip", {
      clipId: "clip-bell",
      path: "/tmp/clip-clip-bell.wav",
      gain: 1,
    });
  });

  it("playTileById is a silent no-op when no matching tile exists", async () => {
    const { result } = setupApp();
    invokeMock.mockClear();
    await result.playTileById("not-there");
    expect(invokeMock).not.toHaveBeenCalledWith(
      "play_soundboard_clip",
      expect.anything(),
    );
  });
});

describe("app store — Phase 3 DSP chain", () => {
  beforeEach(() => {
    invokeMock.mockReset();
    invokeMock.mockResolvedValue(undefined);
  });

  it("setChainParam updates the local chain entry", () => {
    const { result } = setupApp();
    const beforeIdx = result.chain().findIndex((c) => c.id === "gate");
    expect(beforeIdx).toBeGreaterThanOrEqual(0);
    result.setChainParam(beforeIdx, "thresh", -30);
    const after = result.chain()[beforeIdx]!;
    expect(after.vals.thresh).toBe(-30);
  });

  it("setChainParam does NOT call backend when engine is stopped", () => {
    const { result } = setupApp();
    invokeMock.mockClear();
    result.setChainParam(0, "thresh", -40);
    expect(invokeMock).not.toHaveBeenCalledWith(
      "set_effect_param",
      expect.anything(),
    );
  });

  it("setChainParam forwards to backend when engine is running", () => {
    const { result } = setupApp();
    result.setEngineRunning(true);
    invokeMock.mockClear();
    result.setChainParam(0, "thresh", -42);
    expect(invokeMock).toHaveBeenCalledWith("set_effect_param", {
      index: 0,
      key: "thresh",
      value: -42,
    });
  });

  it("setChainEnabled flips the local flag", () => {
    const { result } = setupApp();
    // Store entries are reactive proxies; snapshot the primitive before
    // mutating so the comparison isn't reading the post-mutation value.
    const wasEnabled = result.chain()[0]!.enabled;
    result.setChainEnabled(0, !wasEnabled);
    expect(result.chain()[0]!.enabled).toBe(!wasEnabled);
  });

  it("toggleEffectById finds by id and flips", () => {
    const { result } = setupApp();
    const gateIdx = result.chain().findIndex((c) => c.id === "gate");
    const wasEnabled = result.chain()[gateIdx]!.enabled;
    result.toggleEffectById("gate");
    expect(result.chain()[gateIdx]!.enabled).toBe(!wasEnabled);
  });

  it("syncChain is a no-op when engine is stopped", () => {
    const { result } = setupApp();
    invokeMock.mockClear();
    result.syncChain();
    expect(invokeMock).not.toHaveBeenCalledWith(
      "set_effect_chain",
      expect.anything(),
    );
  });

  it("syncChain sends SetChain when engine is running", () => {
    const { result } = setupApp();
    result.setEngineRunning(true);
    invokeMock.mockClear();
    result.syncChain();
    expect(invokeMock).toHaveBeenCalledWith(
      "set_effect_chain",
      expect.objectContaining({ specs: expect.any(Array) }),
    );
  });

  it("changing presetId resets the inspector selection", () => {
    const { result } = setupApp();
    result.setSelectedEffect("reverb");
    expect(result.selectedEffect()).toBe("reverb");
    // Pick the second preset; its first effect should be the new selection.
    const next = result.preset(); // current
    // Pick any other preset id.
    const otherId = "static-wraith";
    result.setPresetId(otherId);
    // Selection updates via the createEffect on presetId.
    // The first effect of static-wraith is "gate".
    expect(result.selectedEffect()).not.toBe("reverb");
    expect(next.id).not.toBe(otherId);
  });
});

describe("app store — Phase 6 virtual mic + hotkeys", () => {
  beforeEach(() => {
    invokeMock.mockReset();
    invokeMock.mockResolvedValue(undefined);
  });

  it("refreshVirtualMicStatus stores the backend status verbatim", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "detect_virtual_mic") {
        return {
          detected: true,
          cableInputDevice: {
            name: "CABLE Input (VB-Audio Virtual Cable)",
            isDefault: false,
            defaultSampleRate: 48000,
            channels: 2,
          },
          cableOutputDevice: {
            name: "CABLE Output (VB-Audio Virtual Cable)",
            isDefault: false,
            defaultSampleRate: 48000,
            channels: 2,
          },
          downloadUrl: "https://vb-audio.com/Cable/",
        };
      }
      return null;
    });
    const { result } = setupApp();
    expect(result.virtualMicStatus()).toBeNull();
    await result.refreshVirtualMicStatus();
    expect(result.virtualMicStatus()?.detected).toBe(true);
    expect(result.virtualMicStatus()?.cableInputDevice?.name).toContain(
      "CABLE Input",
    );
  });

  it("refreshVirtualMicStatus swallows backend errors and leaves status null", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "detect_virtual_mic") throw new Error("backend exploded");
      return null;
    });
    const { result } = setupApp();
    await result.refreshVirtualMicStatus();
    expect(result.virtualMicStatus()).toBeNull();
  });

  it("hotkeyBindings default to empty for every action so we don't steal keys system-wide", () => {
    // v0.7.1 fix: the previous Space-as-default PTM was registered via
    // tauri-plugin-global-shortcut and captured Space from every other
    // app on Windows. The in-app focused-window listener still handles
    // Space (via ui.ptmKey, which stays "Space") so PTM works in-app.
    const { result } = setupApp();
    expect(result.hotkeyBindings.ptm).toBe("");
    expect(result.hotkeyBindings.panic).toBe("");
    expect(result.hotkeyBindings.monitor).toBe("");
  });

  it("ui.ptmKey defaults to Space so the in-app focused-window PTM still works", () => {
    const { result } = setupApp();
    expect(result.ui.ptmKey).toBe("Space");
  });

  it("setHotkeyBinding stores the accelerator and forwards to the backend", async () => {
    const { result } = setupApp();
    invokeMock.mockClear();
    await result.setHotkeyBinding("panic", "Ctrl+Shift+P");
    expect(result.hotkeyBindings.panic).toBe("Ctrl+Shift+P");
    expect(invokeMock).toHaveBeenCalledWith("register_global_shortcut", {
      id: "panic",
      accelerator: "Ctrl+Shift+P",
    });
  });

  it("setHotkeyBinding for PTM also updates ui.ptmKey for the in-app fallback", async () => {
    const { result } = setupApp();
    await result.setHotkeyBinding("ptm", "Ctrl+Space");
    expect(result.hotkeyBindings.ptm).toBe("Ctrl+Space");
    expect(result.ui.ptmKey).toBe("Ctrl+Space");
  });

  it("setHotkeyBinding with empty accelerator unregisters instead of registering", async () => {
    const { result } = setupApp();
    invokeMock.mockClear();
    await result.setHotkeyBinding("panic", "");
    expect(result.hotkeyBindings.panic).toBe("");
    expect(invokeMock).toHaveBeenCalledWith("unregister_global_shortcut", {
      id: "panic",
    });
    expect(invokeMock).not.toHaveBeenCalledWith(
      "register_global_shortcut",
      expect.anything(),
    );
  });

  it("clearHotkeyBinding is equivalent to setHotkeyBinding(action, '')", async () => {
    const { result } = setupApp();
    await result.setHotkeyBinding("monitor", "F8");
    expect(result.hotkeyBindings.monitor).toBe("F8");
    invokeMock.mockClear();
    await result.clearHotkeyBinding("monitor");
    expect(result.hotkeyBindings.monitor).toBe("");
    expect(invokeMock).toHaveBeenCalledWith("unregister_global_shortcut", {
      id: "monitor",
    });
  });

  it("setHotkeyBinding swallows backend register failures so the UI stays consistent", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "register_global_shortcut") throw new Error("bad accelerator");
      return undefined;
    });
    const { result } = setupApp();
    await result.setHotkeyBinding("panic", "F19");
    // Even though the backend rejected, the local binding still records
    // what the user picked — the next sync attempt will retry.
    expect(result.hotkeyBindings.panic).toBe("F19");
  });

  it("syncHotkeyBindings registers every non-empty binding (and skips empty ones)", async () => {
    const { result } = setupApp();
    // Bind panic + leave PTM and monitor empty (defaults are empty in v0.7.1).
    await result.setHotkeyBinding("panic", "Ctrl+Shift+P");
    invokeMock.mockClear();
    await result.syncHotkeyBindings();
    const calls = invokeMock.mock.calls.filter(
      (c) => c[0] === "register_global_shortcut",
    );
    const ids = calls.map((c) => (c[1] as { id: string }).id).sort();
    expect(ids).toEqual(["panic"]);
  });

  it("syncHotkeyBindings is a no-op when every binding is empty (the default)", async () => {
    // v0.7.1 fix: makes sure the default state never registers any
    // system-wide hotkey. Otherwise the bug where Space gets captured
    // from every other app would silently come back.
    const { result } = setupApp();
    invokeMock.mockClear();
    await result.syncHotkeyBindings();
    expect(invokeMock).not.toHaveBeenCalledWith(
      "register_global_shortcut",
      expect.anything(),
    );
  });

  it("tweaks default to mystical=0.7 (balanced, per prototype), grain=false, vignette=false", () => {
    // v0.11.1 aligned the mystical default with the prototype's
    // `balanced = 0.7`. Earlier code shipped `1` which was the wrong
    // baseline (always rich; segmented control felt non-functional).
    const { result } = setupApp();
    expect(result.tweaks.mystical).toBeCloseTo(0.7, 5);
    expect(result.tweaks.grain).toBe(false);
    expect(result.tweaks.vignette).toBe(false);
  });

  it("setTweaks updates the new Phase 6 fields", () => {
    const { result } = setupApp();
    result.setTweaks("mystical", 0);
    result.setTweaks("grain", true);
    result.setTweaks("vignette", true);
    expect(result.tweaks.mystical).toBe(0);
    expect(result.tweaks.grain).toBe(true);
    expect(result.tweaks.vignette).toBe(true);
  });

  it("theme defaults to dark and toggles to light (v1.11.0)", () => {
    const { result } = setupApp();
    expect(result.tweaks.theme).toBe("dark");
    result.setTweaks("theme", "light");
    expect(result.tweaks.theme).toBe("light");
  });

  it("built-in glyphs default to their preset bindings and rebind via setGlyphBinding", () => {
    const { result } = setupApp();
    expect(result.glyphBindings.triangle).toEqual({ kind: "preset", presetId: "velvet-demon" });
    expect(result.glyphBindings.circle).toEqual({ kind: "preset", presetId: "clean" });
    // A glyph can now bind to any action, e.g. the monitor toggle.
    result.setGlyphBinding("triangle", { kind: "monitor" });
    expect(result.glyphBindings.triangle).toEqual({ kind: "monitor" });
  });
});

describe("app store — Phase 11.1 tweak persistence + mystical values", () => {
  beforeEach(() => {
    invokeMock.mockReset();
    invokeMock.mockResolvedValue(undefined);
    try {
      window.localStorage.clear();
    } catch {
      /* fine */
    }
  });

  it("default mystical is 0.7 ('balanced'), matching the prototype default", () => {
    const { result } = setupApp();
    expect(result.tweaks.mystical).toBeCloseTo(0.7, 5);
  });

  it("setTweaks persists Tweaks state to localStorage", () => {
    const { result } = setupApp();
    result.setTweaks("mystical", 0.3);
    result.setTweaks("mood", "midnight");
    const raw = window.localStorage.getItem("divora.tweaks");
    expect(raw).not.toBeNull();
    const parsed = JSON.parse(raw!);
    expect(parsed.mystical).toBeCloseTo(0.3, 5);
    expect(parsed.mood).toBe("midnight");
  });

  it("persisted Tweaks survive a re-init of the store", () => {
    // Set in one instance, then create another to mimic an app restart.
    const a = setupApp();
    a.result.setTweaks("mystical", 1.0);
    a.result.setTweaks("accent", "ember");
    const b = setupApp();
    expect(b.result.tweaks.mystical).toBeCloseTo(1.0, 5);
    expect(b.result.tweaks.accent).toBe("ember");
  });

  it("the light/dark theme tweak persists across a re-init (v1.11.0)", () => {
    const a = setupApp();
    a.result.setTweaks("theme", "light");
    expect(JSON.parse(window.localStorage.getItem("divora.tweaks")!).theme).toBe(
      "light",
    );
    const b = setupApp();
    expect(b.result.tweaks.theme).toBe("light");
  });

  it("persisted payload is partial-merged onto defaults — new tweak fields don't blow up old data", () => {
    // Pretend an older app version saved only `mood` and never knew
    // about `vignette`. The current store should fill in defaults for
    // every missing field.
    window.localStorage.setItem(
      "divora.tweaks",
      JSON.stringify({ mood: "ink" }),
    );
    const { result } = setupApp();
    expect(result.tweaks.mood).toBe("ink");
    // The default for vignette is false; for mystical 0.7; theme dark.
    expect(result.tweaks.vignette).toBe(false);
    expect(result.tweaks.mystical).toBeCloseTo(0.7, 5);
    expect(result.tweaks.theme).toBe("dark");
  });
});

describe("app store — MIDI control surfaces (v1.9.0)", () => {
  beforeEach(() => {
    invokeMock.mockReset();
    invokeMock.mockResolvedValue(undefined);
    try {
      window.localStorage.clear();
    } catch {
      /* fine */
    }
  });

  const cc = (controller: number, value: number) => ({
    channel: 0,
    kind: "control-change",
    data1: controller,
    data2: value,
  });

  it("addMidiMapping persists to localStorage", () => {
    const { result } = setupApp();
    result.addMidiMapping({ action: "monitor", trigger: null });
    expect(result.midiMappings()).toHaveLength(1);
    const raw = window.localStorage.getItem("divora.midiMappings");
    expect(raw).not.toBeNull();
    expect(JSON.parse(raw!)[0].action).toBe("monitor");
  });

  it("mappings survive a re-init of the store (persist + restore)", () => {
    const a = setupApp();
    a.result.addMidiMapping({
      action: "preset",
      presetId: "clean",
      trigger: { kind: "note", data1: 60, channel: 0 },
    });
    const b = setupApp();
    expect(b.result.midiMappings()).toHaveLength(1);
    expect(b.result.midiMappings()[0]?.presetId).toBe("clean");
    expect(b.result.midiMappings()[0]?.trigger?.data1).toBe(60);
  });

  it("handleMidiMessage in learn mode captures the trigger and disarms", () => {
    const { result } = setupApp();
    result.addMidiMapping({ action: "ptm", trigger: null });
    const id = result.midiMappings()[0]!.id;
    result.setMidiLearnId(id);
    result.handleMidiMessage(cc(7, 64));
    expect(result.midiMappings()[0]?.trigger).toEqual({
      kind: "cc",
      data1: 7,
      channel: 0,
    });
    expect(result.midiLearnId()).toBeNull();
  });

  it("handleMidiMessage routes a learned preset mapping to the active voice", () => {
    const { result } = setupApp();
    const target = result.presets().find((p) => p.id !== result.presetId());
    expect(target).toBeTruthy();
    result.addMidiMapping({
      action: "preset",
      presetId: target!.id,
      trigger: { kind: "note", data1: 36, channel: 0 },
    });
    result.handleMidiMessage({ channel: 0, kind: "note-on", data1: 36, data2: 100 });
    expect(result.presetId()).toBe(target!.id);
  });

  it("removeMidiMapping drops it and persists the removal", () => {
    const { result } = setupApp();
    result.addMidiMapping({ action: "monitor", trigger: null });
    const id = result.midiMappings()[0]!.id;
    result.removeMidiMapping(id);
    expect(result.midiMappings()).toHaveLength(0);
    expect(JSON.parse(window.localStorage.getItem("divora.midiMappings")!)).toEqual([]);
  });
});

describe("app store — guided mic calibration (v1.10.0)", () => {
  beforeEach(() => {
    invokeMock.mockReset();
    invokeMock.mockResolvedValue(undefined);
    try {
      window.localStorage.clear();
    } catch {
      /* fine */
    }
  });

  it("runCalibration writes the gate threshold into the active chain", async () => {
    const { result } = setupApp();
    const id = result.viewedId();
    result.setChains(id, [
      { id: "gate", enabled: true, vals: { thresh: -52 } },
      { id: "denoiser", enabled: true, vals: { mix: 0 } },
    ]);
    // A −40 dB room (amp 0.01) → gate −32. durationMs 0 = one sample.
    result.setInputLevels({ rms: 0.01, peak: 0.01 });
    await result.runCalibration(0);
    expect(result.chains[id]![0]!.vals.thresh).toBe(-32);
    expect(result.calibrated()).toBe(true);
    expect(result.calibrationResult()?.gateThreshDb).toBe(-32);
  });

  it("writes a denoiser mix through when the floor is noisy", async () => {
    const { result } = setupApp();
    const id = result.viewedId();
    result.setChains(id, [
      { id: "gate", enabled: true, vals: { thresh: -52 } },
      { id: "denoiser", enabled: true, vals: { mix: 0 } },
    ]);
    // A loud room (amp 0.02 ≈ −34 dB) → denoiser maxed.
    result.setInputLevels({ rms: 0.02, peak: 0.02 });
    await result.runCalibration(0);
    expect(result.chains[id]![1]!.vals.mix).toBeGreaterThan(0);
  });

  it("persists the calibrated flag and restores it across a re-init", () => {
    const a = setupApp();
    a.result.setCalibrated(true);
    expect(window.localStorage.getItem("divora.calibrated")).toBe("true");
    const b = setupApp();
    expect(b.result.calibrated()).toBe(true);
  });
});

describe("app store — in-app update check (v1.12.0)", () => {
  beforeEach(() => {
    invokeMock.mockReset();
    invokeMock.mockResolvedValue(undefined);
    try {
      window.localStorage.clear();
    } catch {
      /* fine */
    }
  });

  it("is opt-in by default and persists when toggled off", () => {
    const a = setupApp();
    expect(a.result.updateCheckEnabled()).toBe(true);
    a.result.setUpdateCheckEnabled(false);
    expect(window.localStorage.getItem("divora.updateCheckEnabled")).toBe(
      "false",
    );
    const b = setupApp();
    expect(b.result.updateCheckEnabled()).toBe(false);
  });

  it("checkUpdates is a no-op when disabled and never throws", async () => {
    const { result } = setupApp();
    result.setUpdateCheckEnabled(false);
    await result.checkUpdates();
    expect(result.updateAvailable()).toBeNull();
    expect(result.updateChecking()).toBe(false);
  });

  it("checkUpdates stays null without a real version (jsdom) and dismiss is safe", async () => {
    const { result } = setupApp();
    await result.checkUpdates(); // enabled, but getVersion is unavailable here
    expect(result.updateAvailable()).toBeNull();
    result.dismissUpdate();
    expect(result.updateAvailable()).toBeNull();
  });
});

describe("app store — setup diagnostic (v1.13.0)", () => {
  beforeEach(() => {
    invokeMock.mockReset();
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "list_audio_input_devices")
        return [{ name: "Mic", isDefault: true, defaultSampleRate: 48000, channels: 1 }];
      if (cmd === "list_audio_output_devices")
        return [{ name: "Speakers", isDefault: true, defaultSampleRate: 48000, channels: 2 }];
      if (cmd === "detect_virtual_mic")
        return { detected: false, cableInputDevice: null, cableOutputDevice: null, downloadUrl: "x" };
      if (cmd === "start_audio_engine")
        return { inputName: "Mic", outputName: "Speakers", monitorName: null, sampleRate: 48000, inputChannels: 1, outputChannels: 2 };
      return undefined;
    });
    try {
      window.localStorage.clear();
    } catch {
      /* fine */
    }
  });

  it("runs the checklist, starts the engine for the test, then restores stopped state", async () => {
    const { result } = setupApp();
    result.setSelectedInput("Mic");
    result.setSelectedOutput("Speakers");
    result.setOutputLevels({ rms: 0.5, peak: 0.6 }); // skip the 700ms signal poll
    const checks = await result.runDiagnostics();
    expect(checks.length).toBeGreaterThanOrEqual(4);
    expect(result.diagnostics()).not.toBeNull();
    // The engine was started for the test, then restored to stopped.
    expect(result.engineRunning()).toBe(false);
    expect(result.diagnosing()).toBe(false);
    expect(checks.map((c) => c.id)).toEqual(
      expect.arrayContaining(["input", "output", "engine", "cable"]),
    );
  });

  it("reports a failure (and never starts the engine) when no devices exist", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "list_audio_input_devices") return [];
      if (cmd === "list_audio_output_devices") return [];
      if (cmd === "detect_virtual_mic")
        return { detected: false, cableInputDevice: null, cableOutputDevice: null, downloadUrl: "x" };
      return undefined;
    });
    const { result } = setupApp();
    const checks = await result.runDiagnostics();
    expect(checks.find((c) => c.id === "input")?.status).toBe("fail");
    expect(result.engineRunning()).toBe(false);
  });
});

describe("app store — custom glyph casting (v1.15.0)", () => {
  beforeEach(() => {
    invokeMock.mockReset();
    invokeMock.mockResolvedValue(undefined);
    try {
      window.localStorage.clear();
    } catch {
      /* fine */
    }
  });

  // A clean square stroke (raw pointer path) for the recognizer.
  const squarePath = (): { x: number; y: number }[] => {
    const verts = [
      { x: 0, y: 0 }, { x: 100, y: 0 }, { x: 100, y: 100 }, { x: 0, y: 100 }, { x: 0, y: 0 },
    ];
    const out: { x: number; y: number }[] = [];
    for (let i = 0; i < verts.length - 1; i++) {
      const a = verts[i]!;
      const b = verts[i + 1]!;
      for (let s = 0; s < 12; s++) {
        out.push({ x: a.x + ((b.x - a.x) * s) / 12, y: a.y + ((b.y - a.y) * s) / 12 });
      }
    }
    return out;
  };

  it("a built-in glyph resolves its preset binding to colour + name", () => {
    const { result } = setupApp();
    const o = result.glyphOutcome("triangle");
    expect(o?.label).toBe("Velvet Demon");
    expect(o?.action).toEqual({ kind: "preset", presetId: "velvet-demon" });
  });

  it("a built-in glyph can rebind to a non-preset action", () => {
    const { result } = setupApp();
    result.setGlyphBinding("circle", { kind: "panic" });
    expect(result.glyphOutcome("circle")).toEqual({
      color: expect.any(String),
      label: "Panic",
      action: { kind: "panic" },
    });
    // And it survives a re-init.
    const b = setupApp();
    expect(b.result.glyphBindings.circle).toEqual({ kind: "panic" });
  });

  it("custom glyphs add, resolve, recognise, persist, and remove", () => {
    const a = setupApp();
    const template = makeTemplate(squarePath());
    a.result.addCustomGlyph({ id: "g1", name: "Box", template, action: { kind: "monitor" } });
    expect(a.result.glyphOutcome("g1")?.label).toBe("Monitor");
    // Drawing the same stroke recognises it.
    expect(a.result.recognizeCustomGlyph(squarePath())?.id).toBe("g1");
    // Persists across a re-init.
    const b = setupApp();
    expect(b.result.customGlyphs().map((g) => g.id)).toContain("g1");
    b.result.removeCustomGlyph("g1");
    expect(b.result.customGlyphs()).toHaveLength(0);
  });
});

describe("app store — v1.17.0 text-to-speech (Speak)", () => {
  beforeEach(() => {
    invokeMock.mockReset();
    invokeMock.mockResolvedValue(undefined);
    try {
      window.localStorage.clear();
    } catch {
      /* jsdom always has localStorage */
    }
  });

  const TWO_VOICES = [
    { id: "af_heart", name: "Aria", lang: "en-us", installed: false },
    { id: "bm_george", name: "George", lang: "en-gb", installed: false },
  ];

  it("refreshTtsVoices populates the list and defaults the selection to the first", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "list_tts_voices") return TWO_VOICES;
      return null;
    });
    const { result } = setupApp();
    expect(result.selectedTtsVoice()).toBeNull();
    await result.refreshTtsVoices();
    expect(result.ttsVoices()).toHaveLength(2);
    expect(result.selectedTtsVoice()).toBe("af_heart");
  });

  it("refreshTtsVoices keeps a valid persisted selection", async () => {
    window.localStorage.setItem("divora.ttsVoice", JSON.stringify("bm_george"));
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "list_tts_voices") return TWO_VOICES;
      return null;
    });
    const { result } = setupApp();
    expect(result.selectedTtsVoice()).toBe("bm_george");
    await result.refreshTtsVoices();
    expect(result.selectedTtsVoice()).toBe("bm_george"); // unchanged
  });

  it("setSelectedTtsVoice persists to localStorage", () => {
    const { result } = setupApp();
    result.setSelectedTtsVoice("af_heart");
    expect(window.localStorage.getItem("divora.ttsVoice")).toContain("af_heart");
  });

  it("speakText is a no-op when the text is blank", async () => {
    const { result } = setupApp();
    result.setSelectedTtsVoice("af_heart");
    result.setTtsText("   ");
    invokeMock.mockClear();
    await result.speakText();
    expect(invokeMock).not.toHaveBeenCalledWith("speak", expect.anything());
  });

  it("speakText forwards text + voice and registers a 'tts' playing clip on success", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "speak") return 2.0; // duration
      return null;
    });
    const { result } = setupApp();
    result.setSelectedTtsVoice("af_heart");
    result.setTtsText("Hello there.");
    await result.speakText();
    expect(invokeMock).toHaveBeenCalledWith("speak", {
      text: "Hello there.",
      voiceId: "af_heart",
      gain: 1.0,
      previewOnly: false,
      candidates: 3,
      useGpu: false,
    });
    expect(result.playingClips["tts"]).toBeDefined();
    expect(result.playingClips["tts"]!.durationSecs).toBe(2.0);
    expect(result.synthesizing()).toBe(false);
    expect(result.ttsError()).toBeNull();
  });

  it("speakText records a 'not installed' error and leaves nothing playing", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "speak") throw "text-to-speech voices are not installed";
      return null;
    });
    const { result } = setupApp();
    result.setSelectedTtsVoice("af_heart");
    result.setTtsText("Hello");
    await result.speakText();
    expect(result.ttsError()).toBe("text-to-speech voices are not installed");
    expect(result.playingClips["tts"]).toBeUndefined();
    expect(result.synthesizing()).toBe(false);
  });

  it("stopSpeaking clears the 'tts' clip and invokes stop_speak", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "speak") return 5.0;
      return undefined;
    });
    const { result } = setupApp();
    result.setSelectedTtsVoice("af_heart");
    result.setTtsText("A long passage.");
    await result.speakText();
    expect(result.playingClips["tts"]).toBeDefined();
    invokeMock.mockClear();
    await result.stopSpeaking();
    expect(result.playingClips["tts"]).toBeUndefined();
    expect(invokeMock).toHaveBeenCalledWith("stop_speak");
  });

  it("ttsVolume defaults to 1.0, clamps to [0,2], and persists", () => {
    const { result } = setupApp();
    expect(result.ttsVolume()).toBe(1.0);
    result.setTtsVolume(0.5);
    expect(result.ttsVolume()).toBe(0.5);
    expect(window.localStorage.getItem("divora.ttsVolume")).toBe("0.5");
    result.setTtsVolume(99);
    expect(result.ttsVolume()).toBe(2);
    result.setTtsVolume(-1);
    expect(result.ttsVolume()).toBe(0);
  });

  it("ttsPreviewOnly defaults to false and persists", () => {
    const { result } = setupApp();
    expect(result.ttsPreviewOnly()).toBe(false);
    result.setTtsPreviewOnly(true);
    expect(result.ttsPreviewOnly()).toBe(true);
    expect(window.localStorage.getItem("divora.ttsPreviewOnly")).toBe("true");
  });

  it("speakText passes the current volume + preview-only to the backend", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "speak") return 1.0;
      return undefined;
    });
    const { result } = setupApp();
    result.setSelectedTtsVoice("af_heart");
    result.setTtsText("Hello");
    result.setTtsVolume(0.4);
    result.setTtsPreviewOnly(true);
    await result.speakText();
    expect(invokeMock).toHaveBeenCalledWith("speak", {
      text: "Hello",
      voiceId: "af_heart",
      gain: 0.4,
      previewOnly: true,
      candidates: 3,
      useGpu: false,
    });
  });

  it("cloneQuality defaults to 3 (balanced), clamps to [1,8], and persists", () => {
    const { result } = setupApp();
    expect(result.cloneQuality()).toBe(3);
    result.setCloneQuality(6);
    expect(result.cloneQuality()).toBe(6);
    expect(window.localStorage.getItem("divora.cloneQuality")).toBe("6");
    result.setCloneQuality(99);
    expect(result.cloneQuality()).toBe(8);
    result.setCloneQuality(0);
    expect(result.cloneQuality()).toBe(1);
  });

  it("ttsProgress starts null, is settable, and clears after speakText", async () => {
    invokeMock.mockImplementation(async (cmd: string) =>
      cmd === "speak" ? 1.0 : undefined,
    );
    const { result } = setupApp();
    expect(result.ttsProgress()).toBeNull();
    result.setTtsProgress({ done: 2, total: 6 });
    expect(result.ttsProgress()).toEqual({ done: 2, total: 6 });
    result.setSelectedTtsVoice("af_heart");
    result.setTtsText("Hello");
    await result.speakText();
    expect(result.ttsProgress()).toBeNull(); // cleared in the finally block
  });

  it("speakText forwards the selected clone quality as best-of-N candidates", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "speak") return 1.0;
      return undefined;
    });
    const { result } = setupApp();
    result.setSelectedTtsVoice("af_heart");
    result.setTtsText("Hello");
    result.setCloneQuality(6);
    await result.speakText();
    expect(invokeMock).toHaveBeenCalledWith(
      "speak",
      expect.objectContaining({ candidates: 6 }),
    );
  });

  it("refreshClonedVoices populates the cloned-voice list", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "list_cloned_voices")
        return [{ id: "my-voice", name: "My Voice", baseName: "Puck" }];
      return undefined;
    });
    const { result } = setupApp();
    await result.refreshClonedVoices();
    expect(result.clonedVoices()).toHaveLength(1);
    expect(result.clonedVoices()[0]!.id).toBe("my-voice");
  });

  it("removeClonedVoice deletes, refreshes, and resets a deleted selection", async () => {
    let cloned = [{ id: "my-voice", name: "My Voice", baseName: "Puck" }];
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "list_cloned_voices") return cloned;
      if (cmd === "delete_cloned_voice") {
        cloned = [];
        return undefined;
      }
      if (cmd === "list_tts_voices") return TWO_VOICES;
      return undefined;
    });
    const { result } = setupApp();
    await result.refreshTtsVoices(); // presets, for the fallback
    await result.refreshClonedVoices();
    result.setSelectedTtsVoice("my-voice");
    await result.removeClonedVoice("my-voice");
    expect(invokeMock).toHaveBeenCalledWith("delete_cloned_voice", {
      id: "my-voice",
    });
    expect(result.clonedVoices()).toHaveLength(0);
    expect(result.selectedTtsVoice()).toBe("af_heart"); // fell back to first preset
  });

  // ---- v1.43.0: renaming a cloned voice ----

  it("renameClonedVoice renames, refreshes, and keeps the selection (v1.43.0)", async () => {
    let cloned = [{ id: "my-voice", name: "My Voice", baseName: "Puck" }];
    invokeMock.mockImplementation(async (cmd: string, args?: unknown) => {
      if (cmd === "list_cloned_voices") return cloned;
      if (cmd === "rename_cloned_voice") {
        const { id, name } = args as { id: string; name: string };
        cloned = cloned.map((v) => (v.id === id ? { ...v, name } : v));
        return undefined;
      }
      if (cmd === "list_tts_voices") return TWO_VOICES;
      return undefined;
    });
    const { result } = setupApp();
    await result.refreshClonedVoices();
    result.setSelectedTtsVoice("my-voice");

    // Leading/trailing whitespace is trimmed before it reaches the backend.
    const ok = await result.renameClonedVoice("my-voice", "  Renamed  ");
    expect(ok).toBe(true);
    expect(invokeMock).toHaveBeenCalledWith("rename_cloned_voice", {
      id: "my-voice",
      name: "Renamed",
    });
    expect(result.clonedVoices()[0]!.name).toBe("Renamed");
    // The id is stable, so the selection survives the rename.
    expect(result.selectedTtsVoice()).toBe("my-voice");
    expect(result.cloneError()).toBeNull();
  });

  it("renameClonedVoice surfaces a backend rejection and keeps the old name (v1.43.0)", async () => {
    const cloned = [{ id: "my-voice", name: "My Voice", baseName: "Puck" }];
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "list_cloned_voices") return cloned;
      if (cmd === "rename_cloned_voice") throw "name cannot be empty";
      return undefined;
    });
    const { result } = setupApp();
    await result.refreshClonedVoices();

    const ok = await result.renameClonedVoice("my-voice", "Whatever");
    expect(ok).toBe(false);
    expect(result.cloneError()).toBe("name cannot be empty");
    expect(result.clonedVoices()[0]!.name).toBe("My Voice");
  });

  it("renameClonedVoice skips empty names and no-op renames (v1.43.0)", async () => {
    const cloned = [{ id: "my-voice", name: "My Voice", baseName: "Puck" }];
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "list_cloned_voices") return cloned;
      return undefined;
    });
    const { result } = setupApp();
    await result.refreshClonedVoices();

    // Whitespace-only is refused without touching the backend.
    expect(await result.renameClonedVoice("my-voice", "   ")).toBe(false);
    // Renaming to the SAME name is a no-op success (no pointless write).
    expect(await result.renameClonedVoice("my-voice", "My Voice")).toBe(true);
    expect(invokeMock).not.toHaveBeenCalledWith(
      "rename_cloned_voice",
      expect.anything(),
    );
  });

  // ---- v1.46.0: reactive effects ----

  it("reactive routes take their base from the ACTIVE preset (v1.46.0)", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "list_tts_voices") return TWO_VOICES;
      return undefined;
    });
    const { result } = setupApp();
    result.setEngineRunning(true);
    // A chain whose distortion is authored at drive 18.
    result.setChains(result.viewedId(), [
      { id: "distortion", enabled: true, vals: { drive: 18 } },
      // 42, deliberately NOT the catalog default (25) — otherwise this
      // assertion would pass even if the base fell through to the default.
      { id: "reverb", enabled: true, vals: { size: 40, mix: 42 } },
    ]);
    result.setReactiveEnabled(true);

    const call = invokeMock.mock.calls
      .filter((c) => c[0] === "set_reactive_config")
      .pop();
    expect(call).toBeDefined();
    const cfg = (call![1] as { config: any }).config;
    expect(cfg.enabled).toBe(true);
    const drive = cfg.routes.find((r: any) => r.key === "drive");
    expect(drive).toBeDefined();
    // The base is the preset's authored value, NOT the catalog default —
    // otherwise the modulation would push away from the wrong starting point.
    expect(drive.base).toBe(18);
    const mix = cfg.routes.find((r: any) => r.key === "mix");
    expect(mix.base).toBe(42);
  });

  it("reactive routes skip effects the preset does not use (v1.46.0)", async () => {
    invokeMock.mockResolvedValue(undefined);
    const { result } = setupApp();
    result.setEngineRunning(true);
    result.setChains(result.viewedId(), [
      { id: "eq", enabled: true, vals: { low: 0 } },
    ]);
    result.setReactiveEnabled(true);

    const call = invokeMock.mock.calls
      .filter((c) => c[0] === "set_reactive_config")
      .pop();
    const cfg = (call![1] as { config: any }).config;
    expect(cfg.routes).toHaveLength(0);
    // And the UI can tell the user why enabling it would do nothing.
    expect(result.reactiveHasTarget()).toBe(false);
  });

  it("reactiveHasTarget requires the target to be ENABLED (v1.46.0)", () => {
    invokeMock.mockResolvedValue(undefined);
    const { result } = setupApp();
    const id = result.viewedId();
    result.setChains(id, [
      { id: "distortion", enabled: false, vals: { drive: 18 } },
    ]);
    expect(result.reactiveHasTarget()).toBe(false);
    result.setChains(id, [
      { id: "distortion", enabled: true, vals: { drive: 18 } },
    ]);
    expect(result.reactiveHasTarget()).toBe(true);
  });

  it("intensity is sent as a 0..1 fraction and clamped (v1.46.0)", async () => {
    invokeMock.mockResolvedValue(undefined);
    const { result } = setupApp();
    result.setEngineRunning(true);
    result.setChains(result.viewedId(), [
      { id: "distortion", enabled: true, vals: { drive: 18 } },
    ]);
    result.setReactiveEnabled(true);
    result.setReactiveIntensity(50);

    const cfg = (
      invokeMock.mock.calls
        .filter((c) => c[0] === "set_reactive_config")
        .pop()![1] as { config: any }
    ).config;
    expect(cfg.intensity).toBeCloseTo(0.5, 6);

    // Out-of-range input is clamped rather than forwarded.
    result.setReactiveIntensity(999);
    expect(result.reactiveIntensity()).toBe(100);
    result.setReactiveIntensity(-20);
    expect(result.reactiveIntensity()).toBe(0);
  });

  it("disabling reactive zeroes the live meter (v1.46.0)", () => {
    invokeMock.mockResolvedValue(undefined);
    const { result } = setupApp();
    result.setReactiveEnabled(true);
    result.setModEnv(0.8);
    expect(result.modEnv()).toBeCloseTo(0.8, 6);
    result.setReactiveEnabled(false);
    // Otherwise the meter would sit pinned at its last value after switch-off.
    expect(result.modEnv()).toBe(0);
  });

  it("sends the config when the ENGINE STARTS, not just on toggle (v1.46.0)", async () => {
    invokeMock.mockResolvedValue(undefined);
    const { result } = setupApp();
    // Enable while the engine is DOWN — the mirror of a persisted enabled:true
    // being restored at launch before the engine comes up.
    result.setEngineRunning(false);
    result.setChains(result.viewedId(), [
      { id: "distortion", enabled: true, vals: { drive: 18 } },
    ]);
    result.setReactiveEnabled(true);
    expect(
      invokeMock.mock.calls.filter((c) => c[0] === "set_reactive_config"),
    ).toHaveLength(0);

    // Starting the engine must push it, or the UI would read ON against a
    // backend that never received a config and never modulates.
    result.setEngineRunning(true);
    const sent = invokeMock.mock.calls.filter(
      (c) => c[0] === "set_reactive_config",
    );
    expect(sent.length).toBeGreaterThan(0);
    expect((sent.pop()![1] as { config: any }).config.enabled).toBe(true);
  });

  it("re-sends with fresh bases when the live chain changes (v1.46.0)", async () => {
    invokeMock.mockResolvedValue(undefined);
    const { result } = setupApp();
    const id = result.viewedId();
    result.setEngineRunning(true);
    result.setChains(id, [
      { id: "distortion", enabled: true, vals: { drive: 18 } },
    ]);
    result.setReactiveEnabled(true);

    const driveBase = (): number => {
      const cfg = (
        invokeMock.mock.calls
          .filter((c) => c[0] === "set_reactive_config")
          .pop()![1] as { config: any }
      ).config;
      return cfg.routes.find((r: any) => r.key === "drive").base;
    };
    expect(driveBase()).toBe(18);

    // A chain edit (Inspector slider, preset switch, A/B swap — all land here)
    // must refresh the base. Otherwise the backend keeps offsetting from the
    // old authored value and overwrites the new one on the next buffer.
    result.setChains(id, [
      { id: "distortion", enabled: true, vals: { drive: 70 } },
    ]);
    expect(driveBase()).toBe(70);
  });

  it("addresses the first ENABLED effect by its true occurrence index (v1.46.0)", async () => {
    invokeMock.mockResolvedValue(undefined);
    const { result } = setupApp();
    result.setEngineRunning(true);
    // Two distortions: the audible one is the second occurrence.
    result.setChains(result.viewedId(), [
      { id: "distortion", enabled: false, vals: { drive: 5 } },
      { id: "distortion", enabled: true, vals: { drive: 60 } },
    ]);
    result.setReactiveEnabled(true);

    const cfg = (
      invokeMock.mock.calls
        .filter((c) => c[0] === "set_reactive_config")
        .pop()![1] as { config: any }
    ).config;
    const drive = cfg.routes.find((r: any) => r.key === "drive");
    // The backend counts occurrences regardless of enabled, so nth must be 1 —
    // sending 0 would modulate the silent, disabled entry instead.
    expect(drive.nth).toBe(1);
    expect(drive.base).toBe(60);
  });

  it("stops re-sending once disabled, and re-sends after a restart (v1.46.0)", async () => {
    invokeMock.mockResolvedValue(undefined);
    const { result } = setupApp();
    const id = result.viewedId();
    result.setEngineRunning(true);
    result.setChains(id, [
      { id: "distortion", enabled: true, vals: { drive: 18 } },
    ]);
    result.setReactiveEnabled(true);
    result.setReactiveEnabled(false);
    const afterDisable = invokeMock.mock.calls.filter(
      (c) => c[0] === "set_reactive_config",
    ).length;

    // Chain churn while off must not produce traffic — the disabled config is
    // constant, so the dedupe swallows it.
    result.setChains(id, [
      { id: "distortion", enabled: true, vals: { drive: 55 } },
    ]);
    expect(
      invokeMock.mock.calls.filter((c) => c[0] === "set_reactive_config").length,
    ).toBe(afterDisable);

    // But a fresh engine session has no config, so stopping and starting must
    // re-send rather than dedupe itself into silence.
    result.setEngineRunning(false);
    result.setEngineRunning(true);
    expect(
      invokeMock.mock.calls.filter((c) => c[0] === "set_reactive_config").length,
    ).toBeGreaterThan(afterDisable);
  });

  it("refreshCloneModelsStatus reflects backend readiness", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "clone_models_status") return { ready: true };
      return undefined;
    });
    const { result } = setupApp();
    expect(result.cloneModelsReady()).toBe(false);
    await result.refreshCloneModelsStatus();
    expect(result.cloneModelsReady()).toBe(true);
  });

  it("startRecordingVoice then stopRecordingVoice clones, selects, and clears recording (v1.23.0)", async () => {
    let started = false;
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "start_voice_recording") {
        started = true;
        return undefined;
      }
      if (cmd === "stop_voice_recording")
        return { id: "me", name: "Me", baseName: "Puck" };
      if (cmd === "list_cloned_voices")
        return started ? [{ id: "me", name: "Me", baseName: "Puck" }] : [];
      return undefined;
    });
    const { result } = setupApp();
    await result.startRecordingVoice();
    expect(result.recordingVoice()).toBe(true);

    await result.stopRecordingVoice("Me");
    expect(result.recordingVoice()).toBe(false);
    expect(invokeMock).toHaveBeenCalledWith("stop_voice_recording", {
      name: "Me",
    });
    expect(result.selectedTtsVoice()).toBe("me"); // auto-selected the new voice
  });

  it("startRecordingVoice surfaces an error and stays idle when the engine is off (v1.23.0)", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "start_voice_recording")
        throw "start the engine first, then record your voice";
      return undefined;
    });
    const { result } = setupApp();
    await result.startRecordingVoice();
    expect(result.recordingVoice()).toBe(false);
    expect(result.cloneError()).toBe(
      "start the engine first, then record your voice",
    );
  });
});
