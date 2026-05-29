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

describe("app store — Phase 4 preset actions", () => {
  beforeEach(() => {
    invokeMock.mockReset();
    invokeMock.mockResolvedValue(undefined);
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
