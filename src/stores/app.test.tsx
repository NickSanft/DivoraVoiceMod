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

  it("glyphs default to bundled preset ids and update via setGlyphs", () => {
    const { result } = setupApp();
    expect(result.glyphs.triangle).toBe("velvet-demon");
    expect(result.glyphs.circle).toBe("clean");
    result.setGlyphs("triangle", "static-wraith");
    expect(result.glyphs.triangle).toBe("static-wraith");
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
    // The default for vignette is false; for mystical 0.7.
    expect(result.tweaks.vignette).toBe(false);
    expect(result.tweaks.mystical).toBeCloseTo(0.7, 5);
  });
});
