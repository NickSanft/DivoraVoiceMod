import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { renderHook } from "@solidjs/testing-library";
import { makeTemplate } from "../data/unistroke";
import { FALLBACK_PRESETS } from "../data/presets";
import type { StoredTimer, StoredTimers, TimerDefWire } from "../data/timers";
import type { TimerSnapshot, TimerState } from "../overlay/timerState";

const invokeMock = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
}));
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(async () => () => {
    /* unlisten no-op */
  }),
}));

import { listen } from "@tauri-apps/api/event";
import { AppProvider, useApp } from "./app";
import { applyLevelUpdate, dispatchGlobalShortcut } from "../App";

function setupApp() {
  return renderHook(() => useApp(), { wrapper: AppProvider });
}

/** v1.59.0: invokes in flight at the same time can reach Rust out of send
 *  order (measured in the real app with 40 sent at once). Over this wire,
 *  each invoke of `cmds` stays in flight until `drain`, which hands the
 *  newest one in flight to the fake backend (the mock as it was) first, so
 *  what the page sends together lands reversed. `sent` is the page's order,
 *  `landed` the backend's; `peak` is the most in flight at once. */
function reversingWire(cmds: readonly string[]) {
  const backend = invokeMock.getMockImplementation()!;
  const inFlight: Array<() => void> = [];
  const sent: Array<[string, unknown]> = [];
  const landed: Array<[string, unknown]> = [];
  let peak = 0;
  invokeMock.mockImplementation((cmd: string, args?: unknown) => {
    if (!cmds.includes(cmd)) return backend(cmd, args);
    sent.push([cmd, args]);
    return new Promise((resolve, reject) => {
      inFlight.push(() => {
        landed.push([cmd, args]);
        Promise.resolve()
          .then(() => backend(cmd, args))
          .then(resolve, reject);
      });
      peak = Math.max(peak, inFlight.length);
    });
  });
  const settle = async () => {
    for (let i = 0; i < 10; i++) await Promise.resolve();
    await new Promise<void>((r) => setTimeout(r, 0));
  };
  return {
    sent,
    landed,
    peak: () => peak,
    /** Lands what is in flight, newest first, and whatever the page sends
     *  as each answers, until nothing is left in flight. */
    async drain() {
      while (inFlight.length > 0) {
        inFlight.pop()!();
        await settle();
      }
    },
  };
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

  // v1.58.0: push-to-modulate is off ("always") by default, so a preset with
  // effects on reads Modulated; the status follows the engine's mirror.
  it("defaults to modulated with an effect on (push-to-modulate off, not muted)", () => {
    const { result } = setupApp();
    expect(result.hasEnabled()).toBe(true);
    expect(result.ptmState()).toBeNull();
    expect(result.ptmBypassed()).toBe(false);
    expect(result.status()).toBe("modulated");
  });

  it("is clean while the engine's push-to-modulate bypasses the effects", () => {
    const { result } = setupApp();
    result.applyPtmState({ mode: "apply", held: false, bypassed: true, seq: 1 });
    expect(result.ptmBypassed()).toBe(true);
    expect(result.status()).toBe("clean");
    result.applyPtmState({ mode: "apply", held: true, bypassed: false, seq: 2 });
    expect(result.status()).toBe("modulated");
    result.applyPtmState({ mode: "bypass", held: true, bypassed: true, seq: 3 });
    expect(result.status()).toBe("clean");
  });

  it("becomes muted when the engine's mute is adopted, overriding push-to-modulate", () => {
    const { result } = setupApp();
    result.applyMuteState({ muted: true, seq: 1 });
    expect(result.status()).toBe("muted");
    result.applyPtmState({ mode: "apply", held: false, bypassed: true, seq: 1 });
    expect(result.status()).toBe("muted");
    result.applyPtmState({ mode: "apply", held: true, bypassed: false, seq: 2 });
    expect(result.status()).toBe("muted");
  });

  it("a chain with only the noise gate on reads clean (it passes speech at unity)", () => {
    const { result } = setupApp();
    result.usePreset("clean");
    expect(result.chain().map((c) => [c.id, c.enabled])).toEqual([["gate", true]]);
    expect(result.hasEnabled()).toBe(true);
    expect(result.status()).toBe("clean");
    // Any other effect on makes it modulated.
    result.usePreset("hollow-king");
    expect(result.status()).toBe("modulated");
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

  // ---- v1.52.0: screen recording ---------------------------------------

  const MONITORS = [
    { id: "\\\\.\\DISPLAY1", number: 1, x: 0, y: 0, width: 2560, height: 1440, primary: true },
    { id: "\\\\.\\DISPLAY2", number: 2, x: 2560, y: 0, width: 1920, height: 1080, primary: false },
  ];

  /** A backend with the engine and screen recording both working. */
  function screenBackend(
    status: {
      supported: boolean;
      reason: string | null;
      h264Encoder: string | null;
      hardware: boolean;
    } = { supported: true, reason: null, h264Encoder: "X", hardware: true },
  ) {
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
      if (cmd === "screen_capture_status") return status;
      if (cmd === "list_capture_monitors") return MONITORS;
      if (cmd === "list_desktop_audio_sources") {
        return [
          {
            id: "everything",
            label: "Everything except Divora",
            kind: "everything",
            isDefault: false,
            divoraPlays: "nothing",
          },
        ];
      }
      if (cmd === "start_screen_recording") return "C:/rec/divora-screen-test.mp4";
      if (cmd === "stop_screen_recording") return "C:/rec/divora-screen-test.mp4";
      return null;
    });
  }

  it("toggleScreenRecording refuses while the engine is stopped, as the WAV recorder does", async () => {
    screenBackend();
    const { result } = setupApp();
    await result.toggleScreenRecording();
    expect(result.engineError()).toBe("Start the engine before recording.");
    expect(invokeMock).not.toHaveBeenCalledWith("start_screen_recording", expect.anything());
  });

  it("toggleScreenRecording records the saved screen with the saved settings, then stops", async () => {
    // Settings saved by v1.52.0, before desktop audio: it stays off until
    // chosen, so a take from a hotkey records what it always did.
    window.localStorage.setItem(
      "divora.screenRecording",
      JSON.stringify({
        monitorId: "\\\\.\\DISPLAY2",
        fps: 60,
        quality: "high",
        cursor: false,
        avOffsetMs: -20,
      }),
    );
    screenBackend();
    const { result } = setupApp();
    await result.startEngine();
    await result.toggleScreenRecording();
    expect(invokeMock).toHaveBeenCalledWith("start_screen_recording", {
      monitorId: "\\\\.\\DISPLAY2",
      filename: expect.stringMatching(/^divora-screen-.*\.mp4$/),
      fps: 60,
      quality: "high",
      cursor: false,
      avOffsetMs: -20,
      desktopGainDb: -6,
      stems: false,
    });
    expect(result.screenRecording().state).toBe("starting");
    expect(result.screenRecordingPath()).toBe("C:/rec/divora-screen-test.mp4");

    await result.toggleScreenRecording();
    expect(invokeMock).toHaveBeenCalledWith("stop_screen_recording");
  });

  it("v1.53.0: a first run records desktop audio by default, and settles that choice", async () => {
    window.localStorage.removeItem("divora.wizardSeen");
    window.localStorage.removeItem("divora.whatsNewSeenVersion");
    screenBackend();
    const { result } = setupApp();
    expect(result.screenSettings().desktopAudio).toBe("everything");
    const saved = JSON.parse(window.localStorage.getItem("divora.screenRecording") ?? "null");
    expect(saved?.desktopAudio).toBe("everything");
    await result.startEngine();
    await result.toggleScreenRecording();
    expect(invokeMock).toHaveBeenCalledWith(
      "start_screen_recording",
      expect.objectContaining({ desktopAudio: "everything", desktopGainDb: -6, stems: false }),
    );
  });

  it("v1.53.0: someone who has used the app before starts with desktop audio off", async () => {
    window.localStorage.setItem("divora.wizardSeen", "true");
    window.localStorage.removeItem("divora.screenRecording");
    screenBackend();
    const { result } = setupApp();
    expect(result.screenSettings().desktopAudio).toBe("off");
    const saved = JSON.parse(window.localStorage.getItem("divora.screenRecording") ?? "null");
    expect(saved?.desktopAudio).toBe("off");
    await result.startEngine();
    await result.toggleScreenRecording();
    const call = invokeMock.mock.calls.find((c) => c[0] === "start_screen_recording");
    expect(call?.[1]).not.toHaveProperty("desktopAudio");
  });

  it("v1.53.0: a profile whose wizard flag was cleared (Replay setup) is not a first run", async () => {
    window.localStorage.removeItem("divora.wizardSeen");
    window.localStorage.removeItem("divora.screenRecording");
    window.localStorage.setItem("divora.whatsNewSeenVersion", "1.52.0");
    screenBackend();
    const { result } = setupApp();
    expect(result.screenSettings().desktopAudio).toBe("off");
  });

  it("v1.53.0: a saved source that isn't offered now is left out of the take", async () => {
    window.localStorage.setItem(
      "divora.screenRecording",
      JSON.stringify({ desktopAudio: "device:{headphones}" }),
    );
    screenBackend();
    const { result } = setupApp();
    await result.startEngine();
    await result.toggleScreenRecording();
    const call = invokeMock.mock.calls.find((c) => c[0] === "start_screen_recording");
    expect(call?.[1]).not.toHaveProperty("desktopAudio");
  });

  it("v1.53.0: the sources are asked for again when the engine starts", async () => {
    screenBackend();
    const { result } = setupApp();
    await result.refreshCapture();
    const before = invokeMock.mock.calls.filter((c) => c[0] === "list_desktop_audio_sources").length;
    await result.startEngine();
    await new Promise((r) => setTimeout(r, 0));
    const after = invokeMock.mock.calls.filter((c) => c[0] === "list_desktop_audio_sources").length;
    expect(after).toBeGreaterThan(before);
  });

  it("v1.53.0: the card's refresh asks where desktop audio can come from", async () => {
    screenBackend();
    const { result } = setupApp();
    expect(result.desktopSources()).toBeNull();
    await result.refreshCapture();
    expect(result.desktopSources()?.map((s) => s.id)).toEqual(["everything"]);
    // Never on a PC that can't record the screen at all.
    invokeMock.mockClear();
    screenBackend({ supported: false, reason: "no", h264Encoder: null, hardware: false });
    const second = setupApp().result;
    await second.refreshCapture();
    expect(invokeMock).not.toHaveBeenCalledWith("list_desktop_audio_sources");
  });

  it("a screen that is gone falls back to the main one", async () => {
    window.localStorage.setItem(
      "divora.screenRecording",
      JSON.stringify({ monitorId: "\\\\.\\DISPLAY9" }),
    );
    screenBackend();
    const { result } = setupApp();
    await result.startEngine();
    await result.toggleScreenRecording();
    expect(invokeMock).toHaveBeenCalledWith(
      "start_screen_recording",
      expect.objectContaining({ monitorId: "\\\\.\\DISPLAY1", fps: 30, quality: "balanced" }),
    );
  });

  it("an unsupported PC says why on the card and never starts a take", async () => {
    screenBackend({
      supported: false,
      reason: "Install the Media Feature Pack",
      h264Encoder: null,
      hardware: false,
    });
    const { result } = setupApp();
    await result.startEngine();
    await result.toggleScreenRecording();
    expect(result.screenRecording().error).toBe("Install the Media Feature Pack");
    expect(result.screenRecording().state).toBe("idle");
    expect(invokeMock).not.toHaveBeenCalledWith("start_screen_recording", expect.anything());
  });

  it("a refused start puts the card back to idle with the reason", async () => {
    screenBackend();
    const base = invokeMock.getMockImplementation()!;
    invokeMock.mockImplementation(async (cmd: string, args?: unknown) => {
      if (cmd === "start_screen_recording") throw "That screen isn't sending any picture";
      return base(cmd, args);
    });
    const { result } = setupApp();
    await result.startEngine();
    await result.toggleScreenRecording();
    expect(result.screenRecording()).toMatchObject({
      state: "idle",
      error: "That screen isn't sending any picture",
    });
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
    // v1.54.0: a restart, never a Stop — the start tears the old streams
    // down itself, and a Stop is what Replay holds its ring for.
    expect(invokeMock).not.toHaveBeenCalledWith("stop_audio_engine");
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
    expect(invokeMock).not.toHaveBeenCalledWith("stop_audio_engine");
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
    expect(invokeMock).not.toHaveBeenCalledWith("stop_audio_engine");
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
    expect(invokeMock).not.toHaveBeenCalledWith("stop_audio_engine");
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

  // v1.60.0: the engine's start sends the chain through the chain's queue
  // (D19), and a param queued behind it goes once it has answered.
  it("setChainParam forwards to backend when engine is running", async () => {
    const { result } = setupApp();
    result.setEngineRunning(true);
    await new Promise<void>((r) => setTimeout(r, 0));
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

  it("syncChain sends SetChain when engine is running", async () => {
    const { result } = setupApp();
    result.setEngineRunning(true);
    await new Promise<void>((r) => setTimeout(r, 0));
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
    // v1.51.4: bindings persist now, so one test's binding must not leak
    // into the next (the "default registers nothing" guard depends on it).
    window.localStorage.clear();
  });

  it("a binding survives a restart (v1.51.4)", async () => {
    const first = setupApp();
    await first.result.setHotkeyBinding("panic", "Ctrl+Shift+P");
    expect(JSON.parse(window.localStorage.getItem("divora.hotkeyBindings") ?? "{}")).toEqual({
      ptm: "",
      panic: "Ctrl+Shift+P",
      monitor: "",
      // v1.52.0: the recorders' keys, saved unbound.
      "rec:screen": "",
      "rec:wav": "",
      // v1.54.0: Replay's, likewise.
      "replay:save": "",
      "replay:toggle": "",
      // v1.55.0: the overlay's Arrange / Lock switch.
      "overlay:arrange": "",
      // v1.56.0: the mute.
      mute: "",
    });
    first.cleanup();

    // A fresh store is a restart: the binding is back, and it registers.
    const second = setupApp();
    expect(second.result.hotkeyBindings.panic).toBe("Ctrl+Shift+P");
    invokeMock.mockClear();
    await second.result.syncHotkeyBindings();
    const ids = invokeMock.mock.calls
      .filter((c) => c[0] === "register_global_shortcut")
      .map((c) => (c[1] as { id: string }).id);
    expect(ids).toEqual(["panic"]);
  });

  it("a push-to-modulate binding also restores the in-app key", () => {
    window.localStorage.setItem("divora.hotkeyBindings", JSON.stringify({ ptm: "Ctrl+Alt+M" }));
    const { result } = setupApp();
    expect(result.hotkeyBindings.ptm).toBe("Ctrl+Alt+M");
    expect(result.ui.ptmKey).toBe("Ctrl+Alt+M");
  });

  it("the Windows key registers as Super, including a binding saved as Win", async () => {
    window.localStorage.setItem("divora.hotkeyBindings", JSON.stringify({ monitor: "Ctrl+Win+M" }));
    const { result } = setupApp();
    invokeMock.mockClear();
    await result.syncHotkeyBindings();
    expect(invokeMock).toHaveBeenCalledWith("register_global_shortcut", {
      id: "monitor",
      accelerator: "Ctrl+Super+M",
    });

    invokeMock.mockClear();
    await result.setHotkeyBinding("panic", "Win+F9");
    expect(invokeMock).toHaveBeenCalledWith("register_global_shortcut", {
      id: "panic",
      accelerator: "Super+F9",
    });
  });

  it("a soundboard tile hotkey with the Windows key registers as Super", async () => {
    const { result } = setupApp();
    invokeMock.mockClear();
    await result.bindTileHotkey("clip-bell", ["Win", "1"]);
    expect(invokeMock).toHaveBeenCalledWith("register_global_shortcut", {
      id: "sb:clip-bell",
      accelerator: "Super+1",
    });
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
    expect(result.hotkeyBindings.mute).toBe("");
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

  // v1.50.0: Critter Chatter ids are engine-namespaced ("babble:*") and persist
  // like any other voice id.
  const WITH_BABBLE = [
    ...TWO_VOICES,
    {
      id: "babble:bright",
      name: "Bright",
      lang: "en-us",
      installed: true,
      engine: "babble",
    },
    {
      id: "babble:gruff",
      name: "Gruff",
      lang: "en-us",
      installed: true,
      engine: "babble",
    },
  ];

  it("refreshTtsVoices keeps a persisted Critter Chatter selection", async () => {
    window.localStorage.setItem("divora.ttsVoice", JSON.stringify("babble:gruff"));
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "list_tts_voices") return WITH_BABBLE;
      return null;
    });
    const { result } = setupApp();
    await result.refreshTtsVoices();
    // Not reset to the first (uninstalled Kokoro) voice, and still persisted.
    expect(result.selectedTtsVoice()).toBe("babble:gruff");
    expect(window.localStorage.getItem("divora.ttsVoice")).toContain("babble:gruff");
  });

  it("refreshTtsVoices still defaults a fresh install to the first Kokoro preset", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "list_tts_voices") return WITH_BABBLE;
      return null;
    });
    const { result } = setupApp();
    await result.refreshTtsVoices();
    expect(result.selectedTtsVoice()).toBe("af_heart");
  });

  it("speakText sends a Critter Chatter id to speak unchanged", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "speak") return 0.8;
      return null;
    });
    const { result } = setupApp();
    result.setSelectedTtsVoice("babble:bright");
    result.setTtsText("Hi there!");
    await result.speakText();
    expect(invokeMock).toHaveBeenCalledWith(
      "speak",
      expect.objectContaining({ text: "Hi there!", voiceId: "babble:bright" }),
    );
    expect(result.ttsError()).toBeNull();
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

  // ---- v1.47.0: tap-to-audition previews ----

  it("previewVoice plays a voice and marks it as auditioning (v1.47.0)", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "preview_voice") return 1.5;
      return undefined;
    });
    const { result } = setupApp();
    await result.previewVoice("af_heart");
    expect(invokeMock).toHaveBeenCalledWith("preview_voice", {
      voiceId: "af_heart",
      useGpu: false,
    });
    expect(result.previewingVoice()).toBe("af_heart");
  });

  it("previewing a second voice stops the first (v1.47.0)", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "preview_voice") return 1.5;
      return undefined;
    });
    const { result } = setupApp();
    await result.previewVoice("af_heart");
    // Let the store's reads at creation finish first (v1.57.0: the model
    // download read goes out only after its two listens have answered).
    await new Promise<void>((r) => setTimeout(r, 0));
    invokeMock.mockClear();
    await result.previewVoice("bm_george");
    // Assert the SEQUENCE, not just that both happened: moving the stop after
    // the play would silence every second tap, and a "was it called" assertion
    // passes for that too. The mixer takes an idle slot rather than replacing a
    // same-id clip, so without a stop-first both voices sound at once.
    expect(invokeMock.mock.calls.map((c) => c[0])).toEqual([
      "stop_preview_voice",
      "preview_voice",
    ]);
    expect(result.previewingVoice()).toBe("bm_george");
  });

  it("tapping the auditioning voice again stops it (v1.47.0)", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "preview_voice") return 1.5;
      return undefined;
    });
    const { result } = setupApp();
    await result.previewVoice("af_heart");
    await result.previewVoice("af_heart");
    expect(result.previewingVoice()).toBeNull();
    expect(invokeMock).toHaveBeenCalledWith("stop_preview_voice");
  });

  it("a superseded render does not steal the auditioning state back (v1.47.0)", async () => {
    // NB: suppressing the AUDIO of an abandoned render is the backend's job —
    // it bumps its own epoch and discards the result, because a spawn_blocking
    // synthesis cannot be aborted. This test covers the store's half: a late
    // result must not overwrite the voice the user has since chosen.
    let releaseSlow: ((v: number) => void) | null = null;
    invokeMock.mockImplementation(async (cmd: string, args?: any) => {
      if (cmd === "preview_voice") {
        if (args.voiceId === "slow-clone") {
          return new Promise<number>((res) => {
            releaseSlow = res;
          });
        }
        return 1.0;
      }
      return undefined;
    });
    const { result } = setupApp();
    const slow = result.previewVoice("slow-clone");
    await result.previewVoice("af_heart");
    expect(result.previewingVoice()).toBe("af_heart");

    // The abandoned render finally lands — it must not steal the state back.
    releaseSlow!(7.0);
    await slow;
    expect(result.previewingVoice()).toBe("af_heart");
    // And it issues no further backend traffic for the abandoned voice.
    const afterCalls = invokeMock.mock.calls.filter(
      (c) => c[0] === "preview_voice" && (c[1] as any).voiceId === "slow-clone",
    );
    expect(afterCalls).toHaveLength(1);
  });

  it("the auto-clear timer returns the card to its play state (v1.47.0)", async () => {
    vi.useFakeTimers();
    try {
      invokeMock.mockImplementation(async (cmd: string) => {
        if (cmd === "preview_voice") return 2.0;
        return undefined;
      });
      const { result } = setupApp();
      await result.previewVoice("af_heart");
      expect(result.previewingVoice()).toBe("af_heart");
      // Without this timer the card would sit on "Stop" forever and the next
      // tap would become a stop-toggle instead of a replay.
      vi.advanceTimersByTime(2100);
      expect(result.previewingVoice()).toBeNull();
    } finally {
      vi.useRealTimers();
    }
  });

  it("an old clip's timer cannot clear the newly-auditioning voice (v1.47.0)", async () => {
    vi.useFakeTimers();
    try {
      invokeMock.mockImplementation(async (cmd: string, args?: any) => {
        if (cmd === "preview_voice") {
          return args.voiceId === "short" ? 0.3 : 9.0;
        }
        return undefined;
      });
      const { result } = setupApp();
      await result.previewVoice("short");
      await result.previewVoice("long");
      // The short clip's timer must have been cancelled when it was superseded.
      vi.advanceTimersByTime(1000);
      expect(result.previewingVoice()).toBe("long");
    } finally {
      vi.useRealTimers();
    }
  });

  it("previewVoice forwards the GPU setting (v1.47.0)", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "preview_voice") return 1.0;
      return undefined;
    });
    const { result } = setupApp();
    result.setCloneGpu(true);
    await result.previewVoice("my-clone");
    // The backend shares one VoxCPM engine with Speak; asking for a different
    // mode tears it down and rebuilds ~1.6 GB of ONNX graphs.
    expect(invokeMock).toHaveBeenCalledWith("preview_voice", {
      voiceId: "my-clone",
      useGpu: true,
    });
  });

  it("a failed preview clears the state and surfaces the error (v1.47.0)", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "preview_voice") throw "voices are not installed";
      return undefined;
    });
    const { result } = setupApp();
    await result.previewVoice("af_heart");
    expect(result.previewingVoice()).toBeNull();
    expect(result.ttsError()).toBe("voices are not installed");
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

  it("the sentence to read is fixed when a take starts, as the backend's engine is (v1.57.0)", async () => {
    let voxcpm = false;
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "voxcpm_status") return { available: voxcpm, readPrompt: "Read this aloud." };
      // The backend's own decision, from the disk as the take starts.
      if (cmd === "start_voice_recording") return voxcpm ? "Read this aloud." : null;
      if (cmd === "stop_voice_recording") return { id: "me", name: "Me", baseName: "Puck" };
      if (cmd === "list_cloned_voices") return [];
      return undefined;
    });
    const { result } = setupApp();
    await result.refreshVoxcpmStatus();
    await result.startRecordingVoice();
    expect(result.recordingVoice()).toBe(true);
    expect(result.recordingReadPrompt()).toBe("");

    // VoxCPM's download ends mid-take. The backend saves this take with the
    // engine it picked at the start (free speech), so the hint stays too.
    voxcpm = true;
    await result.refreshVoxcpmStatus();
    expect(result.voxcpmAvailable()).toBe(true);
    expect(result.recordingReadPrompt()).toBe("");

    await result.stopRecordingVoice("Me");
    expect(result.recordingReadPrompt()).toBe("");
    // The next take starts with VoxCPM there: it is read from the sentence.
    await result.startRecordingVoice();
    expect(result.recordingReadPrompt()).toBe("Read this aloud.");
  });

  it("the hint is the start's answer, not the page's VoxCPM status (v1.57.0)", async () => {
    // The download has just ended: the backend sees all six files, the page
    // hasn't read VoxCPM's status since. The take is a VoxCPM voice, so the
    // page asks for the sentence anyway.
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "voxcpm_status") return { available: false, readPrompt: "Read this aloud." };
      if (cmd === "start_voice_recording") return "Read this aloud.";
      return undefined;
    });
    const { result } = setupApp();
    await result.refreshVoxcpmStatus();
    expect(result.voxcpmAvailable()).toBe(false);
    await result.startRecordingVoice();
    expect(result.recordingReadPrompt()).toBe("Read this aloud.");
  });
});

// ---- v1.54.0: Replay ---------------------------------------------------

describe("app store — Replay (v1.54.0)", () => {
  type Handler = (e: { payload: unknown }) => void;
  const handlers = new Map<string, Handler[]>();
  const fire = (event: string, payload: unknown) => {
    for (const h of handlers.get(event) ?? []) h({ payload });
  };
  const settle = async () => {
    for (let i = 0; i < 10; i++) await Promise.resolve();
    await new Promise<void>((r) => setTimeout(r, 0));
  };
  const GATE = "Turn Replay on from the Record screen card first — it explains what Replay keeps.";
  const DISPLAY1 = "\\\\.\\DISPLAY1";
  const DISPLAY2 = "\\\\.\\DISPLAY2";
  const OFF = {
    supported: true,
    reason: null,
    state: "off",
    saving: false,
    seconds: 30,
    bufferedMs: 0,
    bytes: 0,
    capBytes: 0,
    monitorId: null,
    heldReason: null,
    armed: null,
  };
  /** A ring built with settings unlike Settings' defaults, so a test can
   *  tell which were sent. */
  const RING = {
    ...OFF,
    state: "armed",
    bufferedMs: 30_000,
    capBytes: 80_044_800,
    monitorId: DISPLAY2,
    armed: {
      fps: 60,
      quality: "high",
      cursor: false,
      desktopAudio: "everything",
      desktopGainDb: -12,
      avOffsetMs: 30,
    },
  };
  const armCalls = () => invokeMock.mock.calls.filter((c) => c[0] === "arm_replay_buffer");

  /** A backend with the engine, the screens and Replay all working. */
  function replayBackend(status: unknown = OFF) {
    invokeMock.mockImplementation(async (cmd: string, args?: Record<string, unknown>) => {
      switch (cmd) {
        case "start_audio_engine":
          return {
            inputName: "Mic",
            outputName: "CABLE Input",
            sampleRate: 48000,
            inputChannels: 1,
            outputChannels: 2,
          };
        case "screen_capture_status":
          return { supported: true, reason: null, h264Encoder: "X", hardware: true };
        case "list_capture_monitors":
          return [
            { id: DISPLAY1, number: 1, x: 0, y: 0, width: 2560, height: 1440, primary: true },
            { id: DISPLAY2, number: 2, x: 2560, y: 0, width: 1920, height: 1080, primary: false },
          ];
        case "list_desktop_audio_sources":
          return [
            {
              id: "everything",
              label: "Everything except Divora",
              kind: "everything",
              isDefault: false,
              divoraPlays: "nothing",
            },
          ];
        case "replay_buffer_status":
          return status;
        case "replay_memory":
          return {
            ceilingBytes: 1_073_741_824,
            lengths: [15, 30, 60, 120].map((s) => ({
              seconds: s,
              bytes: s * 1_000_000,
              capBytes: s * 1_250_000,
              allowed: true,
            })),
          };
        case "arm_replay_buffer":
          return {
            ...OFF,
            state: "armed",
            seconds: args?.seconds,
            monitorId: args?.monitorId,
            armed: {
              fps: args?.fps,
              quality: args?.quality,
              cursor: args?.cursor,
              desktopAudio: args?.desktopAudio ?? null,
              desktopGainDb: args?.desktopGainDb,
              avOffsetMs: args?.avOffsetMs,
            },
          };
        case "disarm_replay_buffer":
          return OFF;
        case "save_replay":
          throw "Replay is off.";
        case "start_screen_recording":
          return "C:/rec/divora-screen-test.mp4";
        case "screen_recording_status":
          return {
            state: "idle",
            path: null,
            elapsedMs: 0,
            droppedFrames: 0,
            driftPpm: 0,
            audioLostAt: null,
            error: null,
            desktopSourceId: null,
            desktopSource: null,
            desktopQuietSeconds: null,
            desktopDoubled: false,
            desktopLostAt: null,
            desktopError: null,
            stemsError: null,
          };
        default:
          return null;
      }
    });
  }

  const explained = (more: Record<string, unknown> = {}) =>
    window.localStorage.setItem(
      "divora.replayBuffer",
      JSON.stringify({ seconds: 30, explained: true, explainedDesktop: false, ...more }),
    );

  beforeEach(() => {
    invokeMock.mockReset();
    handlers.clear();
    vi.mocked(listen).mockImplementation((async (event: string, handler: Handler) => {
      handlers.set(event, [...(handlers.get(event) ?? []), handler]);
      return () => {};
    }) as unknown as typeof listen);
    window.localStorage.clear();
    // Someone who has used the app before: desktop audio starts off.
    window.localStorage.setItem("divora.wizardSeen", "true");
  });
  afterEach(() => {
    vi.mocked(listen).mockImplementation((async () => () => {}) as unknown as typeof listen);
    vi.restoreAllMocks();
  });

  it("is off at launch, whatever is stored", async () => {
    window.localStorage.setItem(
      "divora.replayBuffer",
      JSON.stringify({ seconds: 60, explained: true, armed: true }),
    );
    replayBackend();
    const { result } = setupApp();
    await settle();
    expect(result.replayStatus().state).toBe("off");
    expect(result.replaySettings()).toEqual({
      seconds: 60,
      explained: true,
      explainedDesktop: false,
    });
    expect(armCalls()).toEqual([]);
  });

  it("a window opened while Replay runs pulls its status", async () => {
    replayBackend(RING);
    const { result } = setupApp();
    await settle();
    expect(result.replayStatus()).toMatchObject({ state: "armed", monitorId: DISPLAY2 });
  });

  it("an arm is refused while the engine is stopped, in the recorders' words", async () => {
    explained();
    replayBackend();
    const { result } = setupApp();
    await result.armReplay();
    expect(result.engineError()).toBe("Start the engine before recording.");
    expect(armCalls()).toEqual([]);
  });

  it("an arm sends the screen settings and the length; an upgrader's desktop audio stays off", async () => {
    window.localStorage.setItem(
      "divora.screenRecording",
      JSON.stringify({ monitorId: DISPLAY2, fps: 60, quality: "high", cursor: false, avOffsetMs: -20 }),
    );
    explained({ seconds: 60 });
    replayBackend();
    const { result } = setupApp();
    await result.startEngine();
    await result.armReplay();
    expect(armCalls()).toHaveLength(1);
    expect(armCalls()[0]![1]).toEqual({
      monitorId: DISPLAY2,
      seconds: 60,
      fps: 60,
      quality: "high",
      cursor: false,
      avOffsetMs: -20,
      desktopGainDb: -6,
    });
    expect(result.replayStatus().state).toBe("armed");
  });

  it("with desktop audio on and its explainer read, the arm asks for it", async () => {
    window.localStorage.setItem(
      "divora.screenRecording",
      JSON.stringify({ desktopAudio: "everything", desktopGainDb: -9 }),
    );
    explained({ explainedDesktop: true });
    replayBackend();
    const { result } = setupApp();
    await result.startEngine();
    await result.armReplay();
    expect(armCalls()[0]![1]).toMatchObject({
      monitorId: DISPLAY1,
      desktopAudio: "everything",
      desktopGainDb: -9,
    });
  });

  it("armReplay() never invokes arm_replay_buffer before the explainer — whatever the caller", async () => {
    replayBackend();
    const { result } = setupApp();
    await result.startEngine();
    await settle();

    await result.armReplay();
    expect(result.toast()?.text).toBe(GATE);
    await result.toggleReplay();
    dispatchGlobalShortcut(result, { id: "replay:toggle", accelerator: "F10", state: "pressed" });
    await settle();
    // A running ring resized from Settings goes through the same gate.
    fire("replay-buffer", { ...RING, cursor: true, seconds: 30 });
    await result.setReplaySeconds(60);
    await settle();

    expect(armCalls()).toEqual([]);
    expect(result.toast()).toMatchObject({ text: GATE, tone: "error" });
    // Each refusal is its own toast, so each is announced.
    expect(result.toast()!.id).toBeGreaterThanOrEqual(4);
  });

  it("read with desktop audio off, the explainer is needed again once it is on", async () => {
    window.localStorage.setItem(
      "divora.screenRecording",
      JSON.stringify({ desktopAudio: "everything" }),
    );
    explained({ explainedDesktop: false });
    replayBackend();
    const { result } = setupApp();
    await result.startEngine();
    await result.toggleReplay();
    dispatchGlobalShortcut(result, { id: "replay:toggle", accelerator: "F10", state: "pressed" });
    await settle();
    expect(armCalls()).toEqual([]);
    expect(result.toast()?.text).toBe(GATE);

    // Read again from the card, with desktop audio on: now it arms.
    result.markReplayExplained(true);
    await result.armReplay();
    expect(armCalls()).toHaveLength(1);
  });

  it("once explained, the replay:toggle hotkey turns it on and off", async () => {
    explained();
    replayBackend();
    const { result } = setupApp();
    await result.startEngine();
    dispatchGlobalShortcut(result, { id: "replay:toggle", accelerator: "F10", state: "pressed" });
    await settle();
    expect(armCalls()).toHaveLength(1);
    expect(result.replayStatus().state).toBe("armed");
    dispatchGlobalShortcut(result, { id: "replay:toggle", accelerator: "F10", state: "pressed" });
    await settle();
    expect(invokeMock).toHaveBeenCalledWith("disarm_replay_buffer");
    expect(result.replayStatus().state).toBe("off");
    // A release does nothing.
    dispatchGlobalShortcut(result, { id: "replay:toggle", accelerator: "F10", state: "released" });
    await settle();
    expect(armCalls()).toHaveLength(1);
  });

  it("replay:save just asks; the replay-saved event reports, even a refusal", async () => {
    replayBackend();
    const { result } = setupApp();
    await settle();
    dispatchGlobalShortcut(result, { id: "replay:save", accelerator: "F11", state: "pressed" });
    await settle();
    expect(invokeMock).toHaveBeenCalledWith("save_replay");
    // No toast from the rejected invoke itself…
    expect(result.toast()).toBeNull();
    // …only from the event, which every attempt ends in.
    fire("replay-saved", { path: null, durationMs: 0, error: "Replay is off." });
    expect(result.toast()).toMatchObject({ text: "Replay is off.", tone: "error" });
  });

  it("a length change resizes a running ring with the ring's own settings", async () => {
    // The ring keeps desktop audio, so its explainer was read with it on.
    explained({ explainedDesktop: true });
    replayBackend();
    const { result } = setupApp();
    await result.startEngine();
    await settle();
    // Off: the length is only remembered.
    await result.setReplaySeconds(15);
    expect(armCalls()).toEqual([]);

    fire("replay-buffer", { ...RING, seconds: 15 });
    await result.setReplaySeconds(120);
    expect(armCalls()).toHaveLength(1);
    expect(armCalls()[0]![1]).toEqual({
      monitorId: DISPLAY2,
      seconds: 120,
      fps: 60,
      quality: "high",
      cursor: false,
      avOffsetMs: 30,
      desktopAudio: "everything",
      desktopGainDb: -12,
    });
  });

  it("a length picked while Replay is held applies once the ring runs again", async () => {
    explained({ explainedDesktop: true });
    replayBackend();
    const { result } = setupApp();
    await result.startEngine();
    await settle();
    fire("replay-buffer", { ...RING, seconds: 30 });
    fire("replay-buffer", { ...RING, state: "held", heldReason: "screen-lost", seconds: 30 });
    // Held: the length is only remembered…
    await result.setReplaySeconds(120);
    await settle();
    expect(armCalls()).toEqual([]);
    // …until the backend resumes the ring as it was.
    fire("replay-buffer", { ...RING, state: "arming", seconds: 30 });
    await settle();
    expect(armCalls()).toHaveLength(1);
    expect(armCalls()[0]![1]).toMatchObject({ monitorId: DISPLAY2, seconds: 120, fps: 60 });
    expect(result.replayStatus()).toMatchObject({ state: "armed", seconds: 120 });
    // Once: the ring's own news after that is not another resume.
    fire("replay-buffer", { ...RING, state: "armed", seconds: 120 });
    fire("replay-buffer", { ...RING, state: "armed", seconds: 120, bufferedMs: 31_000 });
    await settle();
    expect(armCalls()).toHaveLength(1);
  });

  it("held for an engine stop, the new length waits for the engine as well", async () => {
    explained({ explainedDesktop: true });
    replayBackend();
    const { result } = setupApp();
    await result.startEngine();
    await settle();
    fire("replay-buffer", { ...RING, seconds: 30 });
    await result.stopEngine();
    fire("replay-buffer", { ...RING, state: "held", heldReason: "engine-stopped", seconds: 30 });
    await result.setReplaySeconds(60);
    // The ring's news can beat the engine's: no arm yet, and no refusal.
    fire("replay-buffer", { ...RING, state: "arming", seconds: 30 });
    await settle();
    expect(armCalls()).toEqual([]);
    expect(result.engineError()).toBeNull();
    await result.startEngine();
    await settle();
    expect(armCalls()).toHaveLength(1);
    expect(armCalls()[0]![1]).toMatchObject({ seconds: 60 });
  });

  it("a ring that resumes at the length picked is left alone", async () => {
    explained({ explainedDesktop: true });
    replayBackend();
    const { result } = setupApp();
    await result.startEngine();
    await settle();
    fire("replay-buffer", { ...RING, seconds: 30 });
    fire("replay-buffer", { ...RING, state: "held", heldReason: "screen-lost", seconds: 30 });
    fire("replay-buffer", { ...RING, state: "arming", seconds: 30 });
    await settle();
    expect(armCalls()).toEqual([]);
    // A later pick on the running ring resizes it the usual way.
    await result.setReplaySeconds(15);
    expect(armCalls()).toHaveLength(1);
  });

  it("a resume's re-arm that is refused is not asked again with every status", async () => {
    explained({ explainedDesktop: true });
    replayBackend();
    const base = invokeMock.getMockImplementation()!;
    invokeMock.mockImplementation(async (cmd: string, args?: Record<string, unknown>) => {
      if (cmd === "arm_replay_buffer") throw "120 s needs more memory than this PC allows.";
      return base(cmd, args);
    });
    const { result } = setupApp();
    await result.startEngine();
    await settle();
    fire("replay-buffer", { ...RING, seconds: 30 });
    fire("replay-buffer", { ...RING, state: "held", heldReason: "screen-lost", seconds: 30 });
    await result.setReplaySeconds(120);
    fire("replay-buffer", { ...RING, state: "arming", seconds: 30 });
    await settle();
    expect(armCalls()).toHaveLength(1);
    expect(result.toast()).toMatchObject({ tone: "error" });
    // The ring runs on at the length it had; its news asks nothing more.
    for (const ms of [1_000, 2_000, 3_000]) {
      fire("replay-buffer", { ...RING, state: "armed", seconds: 30, bufferedMs: ms });
    }
    await settle();
    expect(armCalls()).toHaveLength(1);
  });

  it("divora.replayBuffer never stores whether Replay is on", async () => {
    explained();
    replayBackend();
    const { result } = setupApp();
    await result.startEngine();
    await result.armReplay();
    await result.setReplaySeconds(60);
    result.markReplayExplained(true);
    const stored = JSON.parse(window.localStorage.getItem("divora.replayBuffer") ?? "null");
    expect(Object.keys(stored).sort()).toEqual(["explained", "explainedDesktop", "seconds"]);
    expect(stored).toEqual({ seconds: 60, explained: true, explainedDesktop: true });
  });

  it("a take while Replay is on asks for exactly what the ring runs with", async () => {
    replayBackend();
    const { result } = setupApp();
    await result.startEngine();
    await settle();
    fire("replay-buffer", RING);
    await result.toggleScreenRecording();
    expect(invokeMock).toHaveBeenCalledWith("start_screen_recording", {
      monitorId: DISPLAY2,
      filename: expect.stringMatching(/^divora-screen-.*\.mp4$/),
      fps: 60,
      quality: "high",
      cursor: false,
      avOffsetMs: 30,
      desktopAudio: "everything",
      desktopGainDb: -12,
      stems: false,
    });
  });

  it("a device change while the engine runs restarts it without a Stop", async () => {
    replayBackend();
    const { result } = setupApp();
    result.setSelectedInput("Mic A");
    result.setSelectedOutput("Headphones");
    await result.startEngine();
    result.setIsRecording(true);
    invokeMock.mockClear();
    result.setSelectedOutput("CABLE Input (VB-Audio Virtual Cable)");
    await settle();
    await settle();
    expect(invokeMock).toHaveBeenCalledWith(
      "start_audio_engine",
      expect.objectContaining({ outputName: "CABLE Input (VB-Audio Virtual Cable)" }),
    );
    expect(invokeMock).not.toHaveBeenCalledWith("stop_audio_engine");
    // The start's own teardown finalized the WAV, as a stop does.
    expect(result.isRecording()).toBe(false);
  });

  it("the memory figures follow the chosen screen and settings", async () => {
    replayBackend();
    const { result } = setupApp();
    await result.refreshCapture();
    await settle();
    expect(invokeMock).toHaveBeenCalledWith("replay_memory", {
      monitorId: DISPLAY1,
      fps: 30,
      quality: "balanced",
    });
    expect(result.replayMemory()?.lengths).toHaveLength(4);
    invokeMock.mockClear();
    result.setScreenSettings({ fps: 60 });
    await settle();
    expect(invokeMock).toHaveBeenCalledWith("replay_memory", {
      monitorId: DISPLAY1,
      fps: 60,
      quality: "balanced",
    });
  });

  describe("the save toast names the preset only when the whole clip was disguised", () => {
    let now = 0;
    const saved = (durationMs: number) =>
      fire("replay-saved", { path: "C:/rec/divora-replay-x.mp4", durationMs, error: null });
    // v1.58.0: push-to-modulate is off by default, so the preset's effects
    // are on (modulated) from the store's creation. The mirror turns it on:
    // Hold to apply, the key up (bypassed, clean) or held (modulated).
    const ptm = (held: boolean, seq: number) =>
      ({ mode: "apply", held, bypassed: !held, seq }) as const;

    beforeEach(() => {
      now = 1_000;
      vi.spyOn(performance, "now").mockImplementation(() => now);
    });

    it("modulated with the preset since before the clip: named", async () => {
      replayBackend();
      const { result } = setupApp();
      await settle();
      expect(result.status()).toBe("modulated");
      expect(result.modulatedSince()).toBe(1_000);
      now = 40_000;
      saved(30_000);
      expect(result.toast()).toMatchObject({
        text: `Saved a 30 s replay · ${result.preset().name}`,
        tone: "success",
      });
    });

    it("modulated only partway (allowing for the save's 3 s): the call's voice", async () => {
      replayBackend();
      const { result } = setupApp();
      await settle();
      result.applyPtmState(ptm(false, 1));
      expect(result.status()).toBe("clean");
      expect(result.modulatedSince()).toBeNull();
      now = 1_000;
      result.applyPtmState(ptm(true, 2));
      expect(result.modulatedSince()).toBe(1_000);
      now = 33_999;
      saved(30_000);
      expect(result.toast()?.text).toBe("Saved a 30 s replay · your voice as your call heard it");
    });

    it("clean or muted: the call's voice", async () => {
      replayBackend();
      const { result } = setupApp();
      await settle();
      result.applyPtmState(ptm(false, 1));
      expect(result.status()).toBe("clean");
      now = 40_000;
      saved(30_000);
      expect(result.toast()?.text).toBe("Saved a 30 s replay · your voice as your call heard it");

      result.applyPtmState(ptm(true, 2));
      result.applyMuteState({ muted: true, seq: 1 });
      expect(result.status()).toBe("muted");
      expect(result.modulatedSince()).toBeNull();
      now = 90_000;
      saved(30_000);
      expect(result.toast()?.text).toBe("Saved a 30 s replay · your voice as your call heard it");
    });

    it("a preset change inside the clip: the call's voice", async () => {
      replayBackend();
      const { result } = setupApp();
      await settle();
      expect(result.status()).toBe("modulated");
      now = 20_000;
      result.usePreset("static-wraith");
      expect(result.status()).toBe("modulated");
      expect(result.modulatedSince()).toBe(20_000);
      now = 40_000;
      saved(30_000);
      expect(result.toast()?.text).toBe("Saved a 30 s replay · your voice as your call heard it");
      // Held long enough, the new preset is named.
      now = 60_000;
      saved(30_000);
      expect(result.toast()?.text).toBe("Saved a 30 s replay · Static Wraith");
    });

    it("a held ring's clip ends at the hold: a preset picked after it is not named", async () => {
      replayBackend();
      const { result } = setupApp();
      await settle();
      expect(result.status()).toBe("modulated");
      fire("replay-buffer", { ...RING, seconds: 30 });
      // The screen goes at 40 s; the clip ends there, under the first preset.
      now = 40_000;
      fire("replay-buffer", { ...RING, state: "held", heldReason: "screen-lost", seconds: 30 });
      now = 41_000;
      result.usePreset("static-wraith");
      now = 120_000;
      saved(30_000);
      expect(result.toast()?.text).toBe("Saved a 30 s replay · your voice as your call heard it");
      // Back to keeping: judged from the save again.
      fire("replay-buffer", { ...RING, state: "armed", seconds: 30 });
      now = 125_000;
      saved(30_000);
      expect(result.toast()?.text).toBe("Saved a 30 s replay · Static Wraith");
    });

    it("a held ring disguised from before the clip to the hold is named", async () => {
      replayBackend();
      const { result } = setupApp();
      await settle();
      expect(result.status()).toBe("modulated");
      now = 40_000;
      fire("replay-buffer", { ...RING, state: "held", heldReason: "screen-lost", seconds: 30 });
      now = 300_000;
      saved(30_000);
      expect(result.toast()?.text).toBe(`Saved a 30 s replay · ${result.preset().name}`);
    });
  });
});

// v1.55.0: the `overlay:arrange` hotkey is acted on by the backend, inside
// the hotkey's own message. The JS path only explains why nothing happened;
// it must never ask for the mode too, or one press would toggle twice.
describe("app store — stream overlay hotkey (v1.55.0)", () => {
  type Handler = (e: { payload: unknown }) => void;
  const handlers = new Map<string, Handler[]>();
  const fire = (event: string, payload: unknown) => {
    for (const h of handlers.get(event) ?? []) h({ payload });
  };
  const settle = async () => {
    for (let i = 0; i < 10; i++) await Promise.resolve();
    await new Promise<void>((r) => setTimeout(r, 0));
  };
  const status = (more: Record<string, unknown>) => ({
    open: false,
    phase: "closed",
    instance: 0,
    mode: "locked",
    arrangeGen: 0,
    canArrange: false,
    fit: false,
    hideWanted: false,
    hideAvailable: true,
    capturing: false,
    error: null,
    ...more,
  });
  const press = (result: ReturnType<typeof useApp>) =>
    dispatchGlobalShortcut(result, { id: "overlay:arrange", accelerator: "F8", state: "pressed" });
  const modeCalls = () => invokeMock.mock.calls.filter((c) => c[0] === "set_overlay_mode");

  beforeEach(() => {
    invokeMock.mockReset();
    invokeMock.mockImplementation(async (cmd: string) =>
      cmd === "overlay_status" || cmd === "set_overlay_hide_from_recordings" ? status({}) : null,
    );
    handlers.clear();
    vi.mocked(listen).mockImplementation((async (event: string, handler: Handler) => {
      handlers.set(event, [...(handlers.get(event) ?? []), handler]);
      return () => {};
    }) as unknown as typeof listen);
    window.localStorage.clear();
  });
  afterEach(() => {
    vi.mocked(listen).mockImplementation((async () => () => {}) as unknown as typeof listen);
  });

  it("closed: a notice, and never set_overlay_mode", async () => {
    const { result } = setupApp();
    await settle();
    press(result);
    expect(result.toast()).toMatchObject({
      text: "The overlay is off. Turn it on in Settings first.",
      tone: "success",
    });
    expect(modeCalls()).toEqual([]);
  });

  it("starting: says so; painted: nothing — the backend already switched it", async () => {
    const { result } = setupApp();
    await settle();
    fire("overlay:status", status({ open: true, phase: "starting", instance: 1 }));
    press(result);
    expect(result.toast()?.text).toBe("The overlay is still starting. Try again in a moment.");
    const shown = result.toast()!.id;

    fire("overlay:status", status({ open: true, phase: "painted", instance: 1, canArrange: true }));
    press(result);
    fire("overlay:status", status({ open: true, phase: "painted", instance: 1, canArrange: true, mode: "arrange", arrangeGen: 1 }));
    press(result);
    // A release does nothing either.
    dispatchGlobalShortcut(result, { id: "overlay:arrange", accelerator: "F8", state: "released" });
    await settle();
    expect(result.toast()!.id).toBe(shown);
    expect(modeCalls()).toEqual([]);
  });
});

describe("app store — the mute (v1.56.0)", () => {
  const settle = async () => {
    for (let i = 0; i < 10; i++) await Promise.resolve();
    await new Promise<void>((r) => setTimeout(r, 0));
  };

  /** The engine's word, as the backend keeps it: a change bumps `seq`, a set
   *  to the value it already has moves nothing. */
  function engineWord(start = { muted: false, seq: 0 }) {
    let word = { ...start };
    const set = (muted: boolean) => {
      if (word.muted !== muted) word = { muted, seq: word.seq + 1 };
      return { ...word };
    };
    invokeMock.mockImplementation(async (cmd: string, args?: { muted?: boolean }) => {
      if (cmd === "input_mute") return { ...word };
      if (cmd === "set_input_muted") return set(args!.muted!);
      if (cmd === "toggle_input_muted") return set(!word.muted);
      return undefined;
    });
    return {
      get: () => ({ ...word }),
      /** A change made elsewhere (the hotkey, in Rust): the store hears of it
       *  only through a tick. */
      elsewhere: (muted: boolean) => set(muted),
    };
  }

  const muteCalls = () =>
    invokeMock.mock.calls
      .filter((c) => c[0] === "set_input_muted" || c[0] === "toggle_input_muted")
      .map((c) => (c[0] === "set_input_muted" ? c[1] : c[0]));

  beforeEach(() => {
    invokeMock.mockReset();
    invokeMock.mockResolvedValue(undefined);
    window.localStorage.clear();
  });

  it("defaults to unmuted and modulated; muteKnown waits for the read", async () => {
    let answer!: (v: unknown) => void;
    invokeMock.mockImplementation((cmd: string) =>
      cmd === "input_mute" ? new Promise((r) => (answer = r)) : Promise.resolve(undefined),
    );
    const { result } = setupApp();
    expect(result.muted()).toBe(false);
    expect(result.muteState()).toBeNull();
    // v1.58.0: push-to-modulate is off by default, so the preset's effects
    // are on.
    expect(result.status()).toBe("modulated");
    await settle();
    expect(result.muteKnown()).toBe(false);

    answer({ muted: false, seq: 0 });
    await settle();
    expect(result.muteKnown()).toBe(true);
    expect(result.muteState()).toEqual({ muted: false, seq: 0 });
    expect(result.status()).toBe("modulated");
  });

  it("the boot read adopts a mute before any tick (a reload during a hotkey mute)", async () => {
    engineWord({ muted: true, seq: 4 });
    const { result } = setupApp();
    await settle();
    expect(result.muted()).toBe(true);
    expect(result.muteState()).toEqual({ muted: true, seq: 4 });
    expect(result.status()).toBe("muted");
    expect(result.muteKnown()).toBe(true);
  });

  it("muteKnown comes true when the read fails (off Tauri)", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "input_mute") throw new Error("no tauri");
      return undefined;
    });
    const { result } = setupApp();
    await settle();
    expect(result.muteKnown()).toBe(true);
    expect(result.muted()).toBe(false);
    expect(result.muteState()).toBeNull();
  });

  it("muteKnown comes true after 500 ms when the read never answers", () => {
    vi.useFakeTimers();
    try {
      invokeMock.mockImplementation((cmd: string) =>
        cmd === "input_mute" ? new Promise(() => {}) : Promise.resolve(undefined),
      );
      const { result } = setupApp();
      vi.advanceTimersByTime(499);
      expect(result.muteKnown()).toBe(false);
      vi.advanceTimersByTime(1);
      expect(result.muteKnown()).toBe(true);
      expect(result.muted()).toBe(false);
    } finally {
      vi.useRealTimers();
    }
  });

  it("setMuted sends the value every time and adopts the answer", async () => {
    const word = engineWord();
    const { result } = setupApp();
    await settle();
    await result.setMuted(true);
    await result.setMuted(true);
    expect(muteCalls()).toEqual([{ muted: true }, { muted: true }]);
    expect(result.muteState()).toEqual({ muted: true, seq: 1 });
    expect(word.get()).toEqual({ muted: true, seq: 1 });
    expect(result.status()).toBe("muted");
  });

  it("toggleMuted sends one toggle and adopts the answer", async () => {
    engineWord();
    const { result } = setupApp();
    await settle();
    await result.toggleMuted();
    expect(muteCalls()).toEqual(["toggle_input_muted"]);
    expect(result.muteState()).toEqual({ muted: true, seq: 1 });
  });

  it("a stale tick after an answer is ignored; a newer one is taken", async () => {
    invokeMock.mockImplementation(async (cmd: string) =>
      cmd === "set_input_muted" ? { muted: true, seq: 3 } : undefined,
    );
    const { result } = setupApp();
    await settle();
    await result.setMuted(true);
    // A tick built just before the command landed arrives after its answer.
    result.applyMuteState({ muted: false, seq: 2 });
    expect(result.muted()).toBe(true);
    result.applyMuteState({ muted: false, seq: 4 });
    expect(result.muted()).toBe(false);
  });

  it("a tick without the mute fields changes nothing", async () => {
    const { result } = setupApp();
    await settle();
    const tick = {
      input: { rms: 0.1, peak: 0.2 },
      output: { rms: 0, peak: 0 },
      running: true,
      monitoring: true,
      dspLatencyMs: 0,
      recording: false,
      loudnessGainDb: 0,
      deviceLost: false,
      modEnv: 0,
    };
    applyLevelUpdate(result, tick);
    expect(result.muteState()).toBeNull();
    applyLevelUpdate(result, { ...tick, inputMuted: true, inputMuteSeq: 1 });
    expect(result.muted()).toBe(true);
    applyLevelUpdate(result, tick);
    expect(result.muteState()).toEqual({ muted: true, seq: 1 });
  });

  it("the store sends no mute of its own: only the read, at creation; nothing on start", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "input_mute") return { muted: true, seq: 2 };
      if (cmd === "start_audio_engine") {
        return { inputName: "Mic", outputName: "Speakers", monitorName: null, sampleRate: 48000, inputChannels: 1, outputChannels: 2 };
      }
      return undefined;
    });
    const { result } = setupApp();
    result.setSelectedInput("Mic");
    result.setSelectedOutput("Speakers");
    await result.startEngine();
    await result.stopEngine();
    await result.startEngine();
    await settle();
    expect(result.engineRunning()).toBe(true);
    expect(invokeMock.mock.calls.filter((c) => c[0] === "input_mute")).toHaveLength(1);
    expect(muteCalls()).toEqual([]);
    expect(result.muted()).toBe(true);
  });

  describe("the glyph sends what its omen says", () => {
    it("Mute while unmuted, Unmute while muted", async () => {
      const word = engineWord();
      const { result } = setupApp();
      await settle();
      result.setGlyphBinding("circle", { kind: "mute" });

      expect(result.glyphOutcome("circle")?.label).toBe("Mute");
      result.dispatchGlyphAction({ kind: "mute" });
      await settle();
      expect(muteCalls()).toEqual([{ muted: true }]);
      expect(result.muted()).toBe(true);

      expect(result.glyphOutcome("circle")?.label).toBe("Unmute");
      result.dispatchGlyphAction({ kind: "mute" });
      await settle();
      expect(muteCalls()).toEqual([{ muted: true }, { muted: false }]);
      expect(word.get()).toEqual({ muted: false, seq: 2 });
      expect(result.glyphOutcome("circle")?.label).toBe("Mute");
    });

    it("a stale omen is a no-op, never the opposite", async () => {
      // The hotkey muted in Rust and the mirror hasn't heard yet. The omen
      // says Mute, and the cast sends exactly that: the engine stays muted.
      const word = engineWord();
      const { result } = setupApp();
      await settle();
      word.elsewhere(true);
      result.setGlyphBinding("circle", { kind: "mute" });
      expect(result.glyphOutcome("circle")?.label).toBe("Mute");
      result.dispatchGlyphAction({ kind: "mute" });
      await settle();
      expect(muteCalls()).toEqual([{ muted: true }]);
      expect(word.get()).toEqual({ muted: true, seq: 1 });
      expect(result.muteState()).toEqual({ muted: true, seq: 1 });
    });
  });

  describe("MIDI, through the real handlers", () => {
    const noteOn = (n: number) => ({ channel: 0, kind: "note-on" as const, data1: n, data2: 100 });
    const noteOff = (n: number) => ({ channel: 0, kind: "note-off" as const, data1: n, data2: 0 });
    const cc = (controller: number, value: number) => ({
      channel: 0,
      kind: "control-change" as const,
      data1: controller,
      data2: value,
    });
    type Msg = ReturnType<typeof noteOn> | ReturnType<typeof noteOff> | ReturnType<typeof cc>;

    /** A store with one learned `mute` mapping per trigger given. */
    async function withMappings(...triggers: Array<{ kind: "note" | "cc"; data1: number }>) {
      const word = engineWord();
      const { result } = setupApp();
      await settle();
      for (const t of triggers) {
        result.addMidiMapping({ action: "mute", trigger: { ...t, channel: 0 } });
      }
      const send = async (...msgs: Msg[]) => {
        for (const m of msgs) {
          result.handleMidiMessage(m);
          await settle();
        }
      };
      return { result, word, send };
    }

    it("a pad toggles on note-on; note-off does nothing", async () => {
      const { result, send } = await withMappings({ kind: "note", data1: 48 });
      await send(noteOn(48), noteOff(48));
      expect(muteCalls()).toEqual(["toggle_input_muted"]);
      expect(result.muted()).toBe(true);
      await send(noteOn(48), noteOff(48));
      expect(muteCalls()).toEqual(["toggle_input_muted", "toggle_input_muted"]);
      expect(result.muted()).toBe(false);
    });

    it("a CC resting low as the first message does nothing", async () => {
      const { send } = await withMappings({ kind: "cc", data1: 21 });
      await send(cc(21, 0), cc(21, 3), cc(21, 62));
      expect(muteCalls()).toEqual([]);
    });

    it("a fader swept up then down mutes, then unmutes", async () => {
      const { result, send } = await withMappings({ kind: "cc", data1: 21 });
      await send(cc(21, 10), cc(21, 70));
      expect(muteCalls()).toEqual([{ muted: true }]);
      expect(result.muted()).toBe(true);
      await send(cc(21, 10));
      expect(muteCalls()).toEqual([{ muted: true }, { muted: false }]);
      expect(result.muted()).toBe(false);
    });

    it("a fader resting low doesn't undo a hotkey mute when touched", async () => {
      const { result, word, send } = await withMappings({ kind: "cc", data1: 21 });
      // Fresh: touching it low does nothing.
      await send(cc(21, 10), cc(21, 11), cc(21, 12));
      expect(muteCalls()).toEqual([]);
      // Used once (up, then back down): it is known to be down.
      await send(cc(21, 70), cc(21, 10));
      expect(muteCalls()).toHaveLength(2);
      // The hotkey mutes in Rust; a tick brings it.
      result.applyMuteState(word.elsewhere(true));
      expect(result.muted()).toBe(true);
      await send(cc(21, 10), cc(21, 11), cc(21, 12));
      expect(muteCalls()).toHaveLength(2);
      expect(word.get().muted).toBe(true);
      expect(result.muted()).toBe(true);
    });

    it("jitter at halfway doesn't flap: one set, not one per message", async () => {
      const { send } = await withMappings({ kind: "cc", data1: 21 });
      await send(cc(21, 64), cc(21, 63), cc(21, 64), cc(21, 62), cc(21, 64));
      expect(muteCalls()).toEqual([{ muted: true }]);
    });

    it("each mapping keeps its own side", async () => {
      const { send } = await withMappings({ kind: "cc", data1: 21 }, { kind: "cc", data1: 22 });
      await send(cc(21, 70));
      expect(muteCalls()).toEqual([{ muted: true }]);
      // The second control was never up, so its low first message doesn't
      // unmute.
      await send(cc(22, 10));
      expect(muteCalls()).toEqual([{ muted: true }]);
    });

    it("a mapping that changes forgets its side", async () => {
      const { result, send } = await withMappings({ kind: "cc", data1: 21 });
      await send(cc(21, 70));
      expect(muteCalls()).toEqual([{ muted: true }]);
      const id = result.midiMappings()[0]!.id;
      // Re-learned onto another control: its first message acts only when up.
      result.updateMidiMapping(id, { trigger: { kind: "cc", data1: 23, channel: 0 } });
      await send(cc(23, 10));
      expect(muteCalls()).toEqual([{ muted: true }]);
      await send(cc(23, 90));
      expect(muteCalls()).toEqual([{ muted: true }, { muted: true }]);
    });

    it.each([
      ["another device", ["Deck B"]],
      ["None, then the same device", [null, "Deck A"]],
    ] as const)(
      "picking the MIDI input (%s) forgets every side, so a fader resting low there can't undo a hotkey mute",
      async (_, picks) => {
        const { result, word, send } = await withMappings({ kind: "cc", data1: 7 });
        await result.selectMidiInput("Deck A");
        // Pushed up: mute. Unmuted from the Mixer; the fader stays up.
        await send(cc(7, 10), cc(7, 70));
        await result.setMuted(false);
        await settle();
        expect(muteCalls()).toEqual([{ muted: true }, { muted: false }]);

        // The input changes, and CC 7 there rests low.
        for (const name of picks) await result.selectMidiInput(name);
        // The hotkey mutes in Rust; a tick brings it.
        result.applyMuteState(word.elsewhere(true));
        await send(cc(7, 5), cc(7, 6));
        expect(muteCalls()).toEqual([{ muted: true }, { muted: false }]);
        expect(word.get().muted).toBe(true);
        expect(result.muted()).toBe(true);
      },
    );
  });

  describe("v1.59.0: one mute command at a time", () => {
    afterEach(() => {
      vi.restoreAllMocks();
    });

    /** engineWord, with each set's and toggle's answer held until the test
     *  lets it go (`go`, and only then does the engine change) or refuses
     *  it. */
    function heldWord() {
      const word = engineWord();
      const real = invokeMock.getMockImplementation()!;
      const held: Array<{ go: () => void; refuse: (e: Error) => void }> = [];
      invokeMock.mockImplementation((cmd: string, a?: unknown) => {
        if (cmd !== "set_input_muted" && cmd !== "toggle_input_muted") return real(cmd, a);
        return new Promise((resolve, reject) => {
          held.push({ go: () => resolve(real(cmd, a)), refuse: reject });
        });
      });
      return { word, held };
    }

    it("a labelled set sent while one is slow to answer goes out only after it; the engine ends at the last", async () => {
      const { word, held } = heldWord();
      const { result } = setupApp();
      await settle();
      const on = result.setMuted(true);
      const off = result.setMuted(false);
      await settle();
      expect(muteCalls()).toEqual([{ muted: true }]);
      held.shift()!.go();
      await settle();
      expect(muteCalls()).toEqual([{ muted: true }, { muted: false }]);
      expect(result.muted()).toBe(true);
      held.shift()!.go();
      await Promise.all([on, off]);
      expect(word.get()).toEqual({ muted: false, seq: 2 });
      expect(result.muteState()).toEqual({ muted: false, seq: 2 });
    });

    it("a toggle in flight holds a set back too: both go through one queue", async () => {
      const { word, held } = heldWord();
      const { result } = setupApp();
      await settle();
      const toggle = result.toggleMuted();
      const off = result.setMuted(false);
      await settle();
      expect(muteCalls()).toEqual(["toggle_input_muted"]);
      held.shift()!.go();
      await settle();
      expect(muteCalls()).toEqual(["toggle_input_muted", { muted: false }]);
      held.shift()!.go();
      await Promise.all([toggle, off]);
      expect(word.get()).toEqual({ muted: false, seq: 2 });
      expect(result.muted()).toBe(false);
    });

    it("a set that fails doesn't hold back the next; each caller gets its own outcome", async () => {
      const { word, held } = heldWord();
      const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
      const { result } = setupApp();
      await settle();
      const refused = result.setMuted(true);
      const next = result.setMuted(true);
      await settle();
      expect(muteCalls()).toEqual([{ muted: true }]);
      held.shift()!.refuse(new Error("refused"));
      await settle();
      expect(muteCalls()).toEqual([{ muted: true }, { muted: true }]);
      held.shift()!.go();
      await Promise.all([refused, next]);
      expect(warn).toHaveBeenCalledTimes(1);
      expect(warn).toHaveBeenCalledWith("[mute] set_input_muted failed:", expect.any(Error));
      expect(word.get()).toEqual({ muted: true, seq: 1 });
      expect(result.muteState()).toEqual({ muted: true, seq: 1 });
    });

    it("20 labelled sets, alternating, over a wire that lands what is in flight together reversed: the engine takes them in send order", async () => {
      const word = engineWord();
      const { result } = setupApp();
      await settle();
      const wire = reversingWire(["set_input_muted", "toggle_input_muted"]);
      const done = Array.from({ length: 20 }, (_, i) => result.setMuted(i % 2 === 0));
      await wire.drain();
      await Promise.all(done);
      const want = Array.from({ length: 20 }, (_, i) => ["set_input_muted", { muted: i % 2 === 0 }]);
      expect(wire.sent).toEqual(want);
      expect(wire.landed).toEqual(want);
      expect(wire.peak()).toBe(1);
      expect(word.get()).toEqual({ muted: false, seq: 20 });
      expect(result.muteState()).toEqual({ muted: false, seq: 20 });
    });
  });

  it("Test my setup names the mute when it is why no voice reached the output", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "input_mute") return { muted: true, seq: 1 };
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
    const { result } = setupApp();
    await settle();
    expect(result.muted()).toBe(true);
    result.setSelectedInput("Mic");
    result.setSelectedOutput("Speakers");
    // The OUT meter stays at zero, so the 700 ms signal poll runs out.
    const checks = await result.runDiagnostics();
    const signal = checks.find((c) => c.id === "signal");
    expect(signal?.status).toBe("warn");
    expect(signal?.detail).toBe("Your voice is muted in Divora. Unmute it, then run the test again.");
  });
});

// ---- v1.57.0: the model downloads --------------------------------------

describe("app store — model downloads (v1.57.0)", () => {
  type Handler = (e: { payload: unknown }) => void;
  type App = ReturnType<typeof useApp>;
  const handlers = new Map<string, Handler[]>();
  const fire = (event: string, payload: unknown) => {
    for (const h of handlers.get(event) ?? []) h({ payload });
  };
  const settle = async () => {
    for (let i = 0; i < 10; i++) await Promise.resolve();
    await new Promise<void>((r) => setTimeout(r, 0));
  };
  const BUSY = "A model download is already running.";

  const FLOWS = [
    {
      kind: "clone",
      cmd: "download_clone_models",
      status: "clone_models_status",
      start: (r: App) => r.downloadCloneModels(),
      present: (r: App) => r.cloneModelsReady(),
    },
    {
      kind: "voxcpm",
      cmd: "download_voxcpm_models",
      status: "voxcpm_status",
      start: (r: App) => r.downloadVoxcpmModels(),
      present: (r: App) => r.voxcpmAvailable(),
    },
    {
      kind: "voxcpmGpu",
      cmd: "download_voxcpm_gpu_model",
      status: "voxcpm_gpu_status",
      start: (r: App) => r.downloadVoxcpmGpuModel(),
      present: (r: App) => r.voxcpmGpuModelPresent(),
    },
  ] as const;
  type Flow = (typeof FLOWS)[number];
  const DOWNLOAD_CMDS: readonly string[] = FLOWS.map((f) => f.cmd);

  /** The backend: a download command stays open until the test ends it (as
   *  the real one stays open for minutes), each status says whether its set
   *  is there, and a command in `held` waits until the test releases it. */
  function backend(opts: { status?: unknown; held?: string[] } = {}) {
    const present = { clone: false, voxcpm: false, voxcpmGpu: false };
    const open = new Map<string, { resolve: (v: unknown) => void; reject: (e: unknown) => void }>();
    const waiting = new Map<string, Array<() => void>>();
    const answer = (cmd: string): unknown => {
      switch (cmd) {
        case "model_download_status":
          return opts.status ?? null;
        case "clone_models_status":
          return { ready: present.clone };
        case "voxcpm_status":
          return { available: present.voxcpm, readPrompt: "Read this aloud." };
        case "voxcpm_gpu_status":
          return { supported: true, modelPresent: present.voxcpmGpu };
        default:
          return undefined;
      }
    };
    invokeMock.mockImplementation((cmd: string) => {
      if (DOWNLOAD_CMDS.includes(cmd)) {
        return new Promise((resolve, reject) => open.set(cmd, { resolve, reject }));
      }
      if (opts.held?.includes(cmd)) {
        return new Promise((resolve) =>
          waiting.set(cmd, [...(waiting.get(cmd) ?? []), () => resolve(answer(cmd))]),
        );
      }
      return Promise.resolve(answer(cmd));
    });
    return {
      present,
      /** Answer every waiting call of a held command, with what it says now. */
      release(cmd: string) {
        for (const go of waiting.get(cmd)?.splice(0) ?? []) go();
      },
      /** End a download as the backend does: the set is there (on success),
       *  the done event goes out, then the command answers. */
      end(flow: Flow, error: string | null = null) {
        if (error === null) present[flow.kind] = true;
        fire("clone-model-download-done", { kind: flow.kind, error });
        const call = open.get(flow.cmd);
        open.delete(flow.cmd);
        if (error === null) call?.resolve(null);
        else call?.reject(error);
      },
      /** Only the command's answer, with no event: a refused start (nothing
       *  ran), or the late answer to a download whose done event the test
       *  already sent. */
      answer(flow: Flow, error: string | null = null) {
        const call = open.get(flow.cmd);
        open.delete(flow.cmd);
        if (error === null) call?.resolve(null);
        else call?.reject(error);
      },
    };
  }
  const calls = (cmd: string) => invokeMock.mock.calls.filter((c) => c[0] === cmd).length;
  const progress = (more: Record<string, unknown>) => ({
    file: 1,
    fileCount: 1,
    received: 0,
    total: 0,
    ...more,
  });

  beforeEach(() => {
    invokeMock.mockReset();
    handlers.clear();
    vi.mocked(listen).mockClear();
    vi.mocked(listen).mockImplementation((async (event: string, handler: Handler) => {
      handlers.set(event, [...(handlers.get(event) ?? []), handler]);
      return () => {};
    }) as unknown as typeof listen);
    window.localStorage.clear();
  });
  afterEach(() => {
    vi.mocked(listen).mockImplementation((async () => () => {}) as unknown as typeof listen);
  });

  describe.each(FLOWS)("$kind", (flow) => {
    it("a second start, of any of the three, waits for the first", async () => {
      const be = backend();
      const { result } = setupApp();
      await settle();
      const first = flow.start(result);
      expect(result.cloneDownloading()).toBe(true);
      for (const f of FLOWS) void f.start(result);
      await settle();
      for (const f of FLOWS) expect(calls(f.cmd)).toBe(f === flow ? 1 : 0);

      be.end(flow);
      await first;
      await settle();
      expect(result.cloneDownloading()).toBe(false);
      void flow.start(result);
      expect(calls(flow.cmd)).toBe(2);
    });

    it("the % comes from received over total; a payload with no length keeps it", async () => {
      backend();
      const { result } = setupApp();
      await settle();
      void flow.start(result);
      expect(result.modelDownload()).toEqual({ kind: flow.kind, pct: 0 });
      fire("clone-model-download", progress({ kind: flow.kind, received: 50, total: 200 }));
      expect(result.cloneDownloadPct()).toBe(25);
      fire("clone-model-download", progress({ kind: flow.kind, received: 60, total: 0 }));
      expect(result.cloneDownloadPct()).toBe(25);
      expect(result.modelDownload()).toEqual({ kind: flow.kind, pct: 25 });
    });

    it("success reads its own status, and the set shows as there", async () => {
      const be = backend();
      const { result } = setupApp();
      await settle();
      const run = flow.start(result);
      fire("clone-model-download", progress({ kind: flow.kind, received: 10, total: 10 }));
      be.end(flow);
      await run;
      await settle();
      expect(result.cloneDownloading()).toBe(false);
      expect(result.cloneError()).toBeNull();
      expect(flow.present(result)).toBe(true);
      for (const f of FLOWS) expect(calls(f.status) > 0).toBe(f === flow);
    });

    it("a failure lands in cloneError, and the button comes back", async () => {
      const be = backend();
      const { result } = setupApp();
      await settle();
      const run = flow.start(result);
      fire("clone-model-download", progress({ kind: flow.kind, received: 1, total: 10 }));
      be.end(flow, "downloading x.onnx: ended early: got 1 of 10 bytes");
      await run;
      await settle();
      expect(result.cloneError()).toBe("downloading x.onnx: ended early: got 1 of 10 bytes");
      expect(result.cloneDownloading()).toBe(false);
      expect(flow.present(result)).toBe(false);
    });
  });

  it("the progress and done events are listened to once, at creation, not per download", async () => {
    const be = backend();
    const { result } = setupApp();
    await settle();
    for (const flow of [FLOWS[0], FLOWS[1]]) {
      const run = flow.start(result);
      be.end(flow);
      await run;
      await settle();
    }
    const heard = vi.mocked(listen).mock.calls.map((c) => c[0]);
    expect(heard.filter((e) => e === "clone-model-download")).toHaveLength(1);
    expect(heard.filter((e) => e === "clone-model-download-done")).toHaveLength(1);
  });

  it("a download another page started shows here, and its done event ends it", async () => {
    const be = backend();
    const { result } = setupApp();
    await settle();
    fire(
      "clone-model-download",
      progress({ kind: "voxcpm", file: 3, fileCount: 6, received: 10, total: 40 }),
    );
    expect(result.modelDownload()).toEqual({ kind: "voxcpm", pct: 25 });
    expect(result.cloneDownloading()).toBe(true);

    be.present.voxcpm = true;
    fire("clone-model-download-done", { kind: "voxcpm", error: null });
    await settle();
    expect(result.cloneDownloading()).toBe(false);
    expect(result.voxcpmAvailable()).toBe(true);
    for (const f of FLOWS) expect(calls(f.cmd)).toBe(0);

    // A failure there shows here too.
    fire("clone-model-download", progress({ kind: "clone", received: 1, total: 2 }));
    fire("clone-model-download-done", { kind: "clone", error: "downloading a.onnx: timed out" });
    await settle();
    expect(result.cloneDownloading()).toBe(false);
    expect(result.cloneError()).toBe("downloading a.onnx: timed out");
  });

  it("the read at creation shows a download already running (a reload)", async () => {
    const be = backend({
      status: { file: 1, fileCount: 1, received: 300, total: 1200, kind: "voxcpmGpu" },
    });
    const { result } = setupApp();
    await settle();
    expect(result.modelDownload()).toEqual({ kind: "voxcpmGpu", pct: 25 });
    void result.downloadVoxcpmGpuModel();
    expect(calls("download_voxcpm_gpu_model")).toBe(0);

    fire("clone-model-download", progress({ kind: "voxcpmGpu", received: 600, total: 1200 }));
    expect(result.cloneDownloadPct()).toBe(50);
    be.present.voxcpmGpu = true;
    fire("clone-model-download-done", { kind: "voxcpmGpu", error: null });
    await settle();
    expect(result.cloneDownloading()).toBe(false);
    expect(result.voxcpmGpuModelPresent()).toBe(true);
  });

  it.each([
    [
      "progress",
      () => fire("clone-model-download", progress({ kind: "clone", received: 90, total: 100 })),
      { kind: "clone", pct: 90 },
    ],
    ["done", () => fire("clone-model-download-done", { kind: "clone", error: null }), null],
  ] as const)(
    "the read at creation is dropped when a %s event came while it was on its way",
    async (_name, event, want) => {
      const be = backend({
        status: { file: 1, fileCount: 2, received: 10, total: 100, kind: "clone" },
        held: ["model_download_status"],
      });
      const { result } = setupApp();
      await settle();
      event();
      be.release("model_download_status");
      await settle();
      expect(result.modelDownload()).toEqual(want);
    },
  );

  /** Tauri's listen: the listener is registered (and an event can reach it)
   *  only when the test lets its listen answer. Other events listen at once. */
  function heldListens() {
    const pending = new Map<string, () => void>();
    const HELD = ["clone-model-download", "clone-model-download-done"];
    vi.mocked(listen).mockImplementation(((event: string, handler: Handler) => {
      const register = () => {
        handlers.set(event, [...(handlers.get(event) ?? []), handler]);
        return () => {};
      };
      if (!HELD.includes(event)) return Promise.resolve(register());
      return new Promise((resolve) => pending.set(event, () => resolve(register())));
    }) as unknown as typeof listen);
    return {
      asked: () => [...pending.keys()],
      registered: (event: string) => pending.get(event)!(),
    };
  }

  it("the read at creation waits for both listeners, the done one first", async () => {
    backend({ status: { file: 1, fileCount: 2, received: 40, total: 100, kind: "clone" } });
    const tauri = heldListens();
    const { result } = setupApp();
    await settle();
    // Progress and the read are what can show a download here, so neither
    // goes out before the done event that ends it is sure to arrive.
    expect(tauri.asked()).toEqual(["clone-model-download-done"]);
    expect(calls("model_download_status")).toBe(0);

    tauri.registered("clone-model-download-done");
    await settle();
    expect(tauri.asked()).toEqual(["clone-model-download-done", "clone-model-download"]);
    expect(calls("model_download_status")).toBe(0);

    tauri.registered("clone-model-download");
    await settle();
    expect(calls("model_download_status")).toBe(1);
    expect(result.modelDownload()).toEqual({ kind: "clone", pct: 40 });
  });

  it("a download that ends before the listeners are in isn't shown for good (a reload at its end)", async () => {
    const opts: { status: unknown } = {
      status: { file: 2, fileCount: 2, received: 99, total: 100, kind: "clone" },
    };
    backend(opts);
    const tauri = heldListens();
    const { result } = setupApp();
    await settle();
    // It ends now: the slot is free, and its done event reaches no one.
    opts.status = null;
    fire("clone-model-download-done", { kind: "clone", error: null });
    tauri.registered("clone-model-download-done");
    await settle();
    tauri.registered("clone-model-download");
    await settle();
    expect(result.modelDownload()).toBeNull();
    expect(result.cloneDownloading()).toBe(false);
  });

  it("a failed read at creation leaves nothing shown", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "model_download_status") throw "unknown command";
      return undefined;
    });
    const { result } = setupApp();
    await settle();
    expect(result.modelDownload()).toBeNull();
    expect(result.cloneError()).toBeNull();
  });

  it("a read of none running never clears this page's own start", async () => {
    const be = backend({ held: ["model_download_status"] });
    const { result } = setupApp();
    await settle();
    void result.downloadCloneModels();
    be.release("model_download_status");
    await settle();
    expect(result.modelDownload()).toEqual({ kind: "clone", pct: 0 });
  });

  it("a late answer doesn't clear a newer download", async () => {
    const be = backend();
    const { result } = setupApp();
    await settle();
    // The command answers after its done event, so its answer can land
    // after the next download (another page's) has begun.
    const run = result.downloadCloneModels();
    fire("clone-model-download", progress({ kind: "clone", received: 5, total: 10 }));
    be.present.clone = true;
    fire("clone-model-download-done", { kind: "clone", error: null });
    await settle();
    expect(result.modelDownload()).toBeNull();
    fire("clone-model-download", progress({ kind: "voxcpm", received: 1, total: 10 }));
    be.answer(FLOWS[0]);
    await run;
    await settle();
    expect(result.modelDownload()).toEqual({ kind: "voxcpm", pct: 10 });
  });

  it.each([
    // The end with no progress before it: another process finished the
    // files between the backend's presence check and its loop. (A set
    // already on disk sends no done event at all; the command just answers.)
    ["another process finished the files", null],
    // Or the first file failed before its first MiB (a 404, a refused
    // connection): the done event and the answer carry the same words.
    ["the first file failed", "downloading a.onnx: connection refused"],
  ] as const)(
    "a late answer to a start that saw no progress doesn't clear a newer download (%s)",
    async (_case, error) => {
      const be = backend();
      const { result } = setupApp();
      await settle();
      const run = result.downloadCloneModels();
      if (error === null) be.present.clone = true;
      fire("clone-model-download-done", { kind: "clone", error });
      await settle();
      expect(result.modelDownload()).toBeNull();
      fire("clone-model-download", progress({ kind: "voxcpm", received: 3, total: 10 }));
      be.answer(FLOWS[0], error);
      await run;
      await settle();
      expect(result.modelDownload()).toEqual({ kind: "voxcpm", pct: 30 });
      expect(result.cloneError()).toBe(error);
      expect(result.cloneModelsReady()).toBe(error === null);
    },
  );

  it("a start the backend refuses shows its words as-is", async () => {
    const be = backend();
    const { result } = setupApp();
    await settle();
    const run = result.downloadVoxcpmModels();
    be.answer(FLOWS[1], BUSY);
    await run;
    await settle();
    expect(result.cloneError()).toBe(BUSY);
    expect(result.cloneDownloading()).toBe(false);
  });

  it("a start refused under another page's download keeps showing that download", async () => {
    const be = backend({
      status: { file: 2, fileCount: 6, received: 5, total: 10, kind: "voxcpm" },
      held: ["model_download_status"],
    });
    const { result } = setupApp();
    await settle();
    // Clicked before the read at creation answered: the read is older than
    // the start, but it is this page's only word on the download that
    // refuses it.
    const run = result.downloadCloneModels();
    expect(calls("download_clone_models")).toBe(1);
    be.release("model_download_status");
    await settle();
    be.answer(FLOWS[0], BUSY);
    await run;
    await settle();
    expect(result.cloneError()).toBe(BUSY);
    expect(result.modelDownload()).toEqual({ kind: "voxcpm", pct: 50 });
  });

  it("the end clears only once the status says the set is there", async () => {
    const be = backend({ held: ["clone_models_status"] });
    const { result } = setupApp();
    await settle();
    const run = result.downloadCloneModels();
    fire("clone-model-download", progress({ kind: "clone", received: 10, total: 10 }));
    be.end(FLOWS[0]);
    await settle();
    // Still "Downloading… 100%", not the idle label, while the status is out.
    expect(result.cloneDownloading()).toBe(true);
    expect(result.cloneDownloadPct()).toBe(100);
    be.release("clone_models_status");
    await run;
    await settle();
    expect(result.cloneModelsReady()).toBe(true);
    expect(result.cloneDownloading()).toBe(false);
  });
});

// ---- v1.58.0: push-to-modulate ------------------------------------------

describe("app store — push-to-modulate (v1.58.0)", () => {
  const settle = async () => {
    for (let i = 0; i < 10; i++) await Promise.resolve();
    await new Promise<void>((r) => setTimeout(r, 0));
  };

  type Bits = Partial<Record<"hotkey" | "key" | "button" | "midi", boolean>>;

  /** The engine's word, as the backend keeps it: a held bit per source, a
   *  change bumps `seq`, a set to what it has moves nothing. `set_ptm_mode`
   *  lets go of every hold; with `restore`, of the page's only. The engine
   *  changes when the call arrives; `gate` holds its answer back until
   *  released, `hang` never answers, `fail` refuses. */
  function ptmEngine(start: { mode?: string; bits?: Bits; seq?: number } = {}) {
    let mode = start.mode ?? "always";
    let bits: Bits = { ...start.bits };
    let seq = start.seq ?? 0;
    const state = () => {
      const any = Object.values(bits).some(Boolean);
      const bypassed = mode === "apply" ? !any : mode === "bypass" ? any : false;
      return { mode, held: any, bypassed, seq };
    };
    const write = (m: string, b: Bits) => {
      const sources = ["hotkey", "key", "button", "midi"] as const;
      const same = m === mode && sources.every((k) => !!b[k] === !!bits[k]);
      if (!same) {
        mode = m;
        bits = b;
        seq += 1;
      }
      return state();
    };
    const ctl = {
      gate: null as Array<() => void> | null,
      hang: false,
      fail: null as Error | null,
    };
    invokeMock.mockImplementation(async (cmd: string, a?: Record<string, unknown>) => {
      if (cmd !== "set_ptm_mode" && cmd !== "set_ptm_held") return undefined;
      if (ctl.hang) return new Promise(() => {});
      if (ctl.fail) throw ctl.fail;
      const answer =
        cmd === "set_ptm_mode"
          ? write(a!.mode as string, a!.restore === true ? { hotkey: !!bits.hotkey } : {})
          : write(mode, { ...bits, [a!.source as string]: a!.held as boolean });
      if (ctl.gate) await new Promise<void>((r) => ctl.gate!.push(r));
      return answer;
    });
    return {
      ctl,
      state,
      /** The sources the engine holds. */
      held: () => Object.keys(bits).filter((k) => bits[k as keyof Bits]),
      /** The global key, in Rust: the page hears of it only by a tick. */
      hotkey: (on: boolean) => write(mode, { ...bits, hotkey: on }),
    };
  }

  /** What the page sent the engine, in order. */
  const ptmCalls = () =>
    invokeMock.mock.calls
      .filter((c) => String(c[0]).startsWith("set_ptm"))
      .map((c) => [c[0], c[1]]);
  const heldCall = (source: string, on: boolean) => ["set_ptm_held", { source, held: on }];
  const modeCall = (mode: string, restore = false) => ["set_ptm_mode", { mode, restore }];

  beforeEach(() => {
    invokeMock.mockReset();
    window.localStorage.clear();
  });
  afterEach(() => {
    vi.useRealTimers();
    vi.restoreAllMocks();
  });

  describe("the boot restore", () => {
    it("sends the saved mode with restore at creation; off reads always", async () => {
      ptmEngine();
      const { result } = setupApp();
      // Sent while the store is created: nothing has been awaited yet.
      expect(ptmCalls()).toEqual([modeCall("always", true)]);
      expect(result.ptmKnown()).toBe(false);
      expect(result.ptmSetting()).toEqual({ on: false, hold: "apply" });
      await settle();
      expect(result.ptmKnown()).toBe(true);
      expect(result.ptmState()).toEqual({ mode: "always", held: false, bypassed: false, seq: 0 });
      expect(result.status()).toBe("modulated");
    });

    it.each([
      [{ on: true, hold: "apply" }, "apply"],
      [{ on: true, hold: "bypass" }, "bypass"],
      [{ on: false, hold: "bypass" }, "always"],
      [{ on: "yes", hold: "apply" }, "always"],
      ["apply", "always"],
    ])("restores %j as %s", (saved, mode) => {
      ptmEngine();
      window.localStorage.setItem("divora.ptm", JSON.stringify(saved));
      setupApp();
      expect(ptmCalls()).toEqual([modeCall(mode, true)]);
    });

    it("a global key held across the reload stays held; the page's key is let go", async () => {
      const engine = ptmEngine({ mode: "apply", bits: { hotkey: true, key: true }, seq: 4 });
      window.localStorage.setItem("divora.ptm", JSON.stringify({ on: true, hold: "apply" }));
      const { result } = setupApp();
      await settle();
      expect(engine.held()).toEqual(["hotkey"]);
      expect(result.ptmState()).toEqual({ mode: "apply", held: true, bypassed: false, seq: 5 });
      expect(result.status()).toBe("modulated");
    });

    it("a tick adopted before the answer updates the mirror but leaves ptmKnown false", async () => {
      // The previous page held the in-app key.
      const engine = ptmEngine({ mode: "apply", bits: { key: true }, seq: 7 });
      engine.ctl.gate = [];
      window.localStorage.setItem("divora.ptm", JSON.stringify({ on: true, hold: "apply" }));
      const { result } = setupApp();
      // A tick from before the restore landed: the key was held, so applied.
      result.applyPtmState({ mode: "apply", held: true, bypassed: false, seq: 7 });
      await settle();
      expect(result.ptmState()?.seq).toBe(7);
      expect(result.status()).toBe("modulated");
      expect(result.ptmKnown()).toBe(false);
      // The restore let go of it.
      engine.ctl.gate.shift()!();
      await settle();
      expect(result.ptmKnown()).toBe(true);
      expect(result.ptmState()).toEqual({ mode: "apply", held: false, bypassed: true, seq: 8 });
      expect(result.status()).toBe("clean");
    });

    it("ptmKnown comes true after 500 ms when the restore never answers", () => {
      vi.useFakeTimers();
      const engine = ptmEngine();
      engine.ctl.hang = true;
      const { result } = setupApp();
      vi.advanceTimersByTime(499);
      expect(result.ptmKnown()).toBe(false);
      vi.advanceTimersByTime(1);
      expect(result.ptmKnown()).toBe(true);
      expect(result.ptmState()).toBeNull();
    });

    it("ptmKnown comes true when the restore fails (off Tauri), with a warning", async () => {
      const engine = ptmEngine();
      engine.ctl.fail = new Error("no bridge");
      const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
      const { result } = setupApp();
      await settle();
      expect(result.ptmKnown()).toBe(true);
      expect(result.ptmState()).toBeNull();
      expect(warn).toHaveBeenCalledWith("[ptm] boot restore failed", expect.any(Error));
    });
  });

  describe("the card's set", () => {
    it("setPtmOn(true) sends apply; divora.ptm and the card follow only the answer", async () => {
      const engine = ptmEngine();
      const { result } = setupApp();
      await settle();
      invokeMock.mockClear();
      engine.ctl.gate = [];
      const done = result.setPtmOn(true);
      expect(ptmCalls()).toEqual([modeCall("apply")]);
      await settle();
      // In flight: nothing saved, the card unchanged.
      expect(window.localStorage.getItem("divora.ptm")).toBeNull();
      expect(result.ptmSetting()).toEqual({ on: false, hold: "apply" });
      engine.ctl.gate.shift()!();
      await done;
      expect(result.ptmSetting()).toEqual({ on: true, hold: "apply" });
      expect(JSON.parse(window.localStorage.getItem("divora.ptm")!)).toEqual({
        on: true,
        hold: "apply",
      });
      // Key up, Hold to apply: the effects are off.
      expect(result.status()).toBe("clean");
    });

    it("a refused set_ptm_mode leaves divora.ptm and the card as they were", async () => {
      const engine = ptmEngine();
      window.localStorage.setItem("divora.ptm", JSON.stringify({ on: true, hold: "bypass" }));
      const { result } = setupApp();
      await settle();
      engine.ctl.fail = new Error("refused");
      const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
      await result.setPtmOn(false);
      await result.setPtmHold("apply");
      expect(result.ptmSetting()).toEqual({ on: true, hold: "bypass" });
      expect(JSON.parse(window.localStorage.getItem("divora.ptm")!)).toEqual({
        on: true,
        hold: "bypass",
      });
      expect(warn).toHaveBeenCalledWith("[ptm] set_ptm_mode failed:", expect.any(Error));
    });

    it("setPtmHold('bypass') while on sends bypass; setPtmOn(false) sends always and keeps the hold", async () => {
      ptmEngine();
      const { result } = setupApp();
      await settle();
      await result.setPtmOn(true);
      invokeMock.mockClear();
      await result.setPtmHold("bypass");
      expect(ptmCalls()).toEqual([modeCall("bypass")]);
      expect(result.ptmSetting()).toEqual({ on: true, hold: "bypass" });
      // Bypass with nothing held: the effects are on.
      expect(result.status()).toBe("modulated");
      await result.setPtmOn(false);
      expect(ptmCalls()).toEqual([modeCall("bypass"), modeCall("always")]);
      expect(result.ptmSetting()).toEqual({ on: false, hold: "bypass" });
    });

    it("an off sent while a hold pick is in flight keeps the pick", async () => {
      // The card still shows the old hold until bypass's answer lands, so
      // the off is built from Hold to apply. v1.59.0: it goes out only once
      // the pick has answered.
      const engine = ptmEngine();
      const { result } = setupApp();
      await settle();
      await result.setPtmOn(true);
      invokeMock.mockClear();
      engine.ctl.gate = [];
      const pick = result.setPtmHold("bypass");
      const off = result.setPtmOn(false);
      await settle();
      expect(ptmCalls()).toEqual([modeCall("bypass")]);
      engine.ctl.gate.shift()!();
      await settle();
      expect(ptmCalls()).toEqual([modeCall("bypass"), modeCall("always")]);
      expect(result.ptmSetting()).toEqual({ on: true, hold: "bypass" });
      engine.ctl.gate.shift()!();
      await Promise.all([pick, off]);
      expect(result.ptmSetting()).toEqual({ on: false, hold: "bypass" });
      expect(JSON.parse(window.localStorage.getItem("divora.ptm")!)).toEqual({
        on: false,
        hold: "bypass",
      });
      // Turning it back on asks for the pick.
      invokeMock.mockClear();
      engine.ctl.gate = null;
      await result.setPtmOn(true);
      expect(ptmCalls()).toEqual([modeCall("bypass")]);
    });

    it("a hold picked while off is remembered; the engine stays at always", async () => {
      ptmEngine();
      const { result } = setupApp();
      await settle();
      invokeMock.mockClear();
      await result.setPtmHold("bypass");
      expect(ptmCalls()).toEqual([modeCall("always")]);
      expect(result.ptmSetting()).toEqual({ on: false, hold: "bypass" });
      await result.setPtmOn(true);
      expect(ptmCalls()).toEqual([modeCall("always"), modeCall("bypass")]);
    });

    it("re-picking the current hold sends set_ptm_mode and forgets the page's holds", async () => {
      const engine = ptmEngine();
      const { result } = setupApp();
      await settle();
      await result.setPtmOn(true);
      result.ptmDown("key");
      result.ptmDown("button");
      await settle();
      expect(engine.held()).toEqual(["key", "button"]);
      invokeMock.mockClear();
      await result.setPtmHold("apply");
      expect(ptmCalls()).toEqual([modeCall("apply")]);
      expect(engine.held()).toEqual([]);
      expect(result.ptmState()?.held).toBe(false);
      // The page holds nothing now: a later release sends nothing.
      result.ptmUp("key");
      result.ptmUp("button");
      expect(ptmCalls()).toEqual([modeCall("apply")]);
    });

    it("a stale answer or tick after a newer one is ignored", async () => {
      const engine = ptmEngine();
      const { result } = setupApp();
      await settle();
      await result.setPtmOn(true);
      const now = engine.state();
      expect(result.ptmState()).toEqual(now);
      result.applyPtmState({ ...now, mode: "always", bypassed: false, seq: now.seq - 1 });
      expect(result.ptmState()).toEqual(now);
      expect(result.status()).toBe("clean");
    });
  });

  describe("the page's holds", () => {
    it("ptmDown twice sends one hold; ptmUp without a down sends nothing", async () => {
      const engine = ptmEngine();
      const { result } = setupApp();
      await settle();
      invokeMock.mockClear();
      result.ptmUp("key");
      result.ptmUp("button");
      expect(ptmCalls()).toEqual([]);
      result.ptmDown("key");
      result.ptmDown("key");
      expect(ptmCalls()).toEqual([heldCall("key", true)]);
      result.ptmUp("key");
      result.ptmUp("key");
      // v1.59.0: the release goes once the press has answered.
      await settle();
      expect(ptmCalls()).toEqual([heldCall("key", true), heldCall("key", false)]);
      expect(engine.held()).toEqual([]);
    });

    it("changing the push-to-modulate binding lets go of the key before the key changes", async () => {
      const engine = ptmEngine();
      const { result } = setupApp();
      await settle();
      result.ptmDown("key");
      await settle();
      expect(engine.held()).toEqual(["key"]);
      // What the in-app key was when the release went out.
      const keyAtRelease: string[] = [];
      const real = invokeMock.getMockImplementation()!;
      invokeMock.mockImplementation((cmd: string, a?: Record<string, unknown>) => {
        if (cmd === "set_ptm_held") keyAtRelease.push(result.ui.ptmKey);
        return real(cmd, a);
      });
      invokeMock.mockClear();
      await result.setHotkeyBinding("ptm", "F14");
      expect(ptmCalls()).toEqual([heldCall("key", false)]);
      expect(keyAtRelease).toEqual(["Space"]);
      expect(result.ui.ptmKey).toBe("F14");
      await settle();
      expect(engine.held()).toEqual([]);
      // Nothing held: binding another key sends nothing.
      await result.setHotkeyBinding("ptm", "");
      expect(ptmCalls()).toEqual([heldCall("key", false)]);
    });
  });

  describe("MIDI, through the real handlers", () => {
    const noteOn = (n: number) => ({ channel: 0, kind: "note-on" as const, data1: n, data2: 100 });
    const noteOff = (n: number) => ({ channel: 0, kind: "note-off" as const, data1: n, data2: 0 });
    const cc = (controller: number, value: number) => ({
      channel: 0,
      kind: "control-change" as const,
      data1: controller,
      data2: value,
    });

    /** A store with one learned `ptm` mapping per trigger given, and its ids
     *  in the same order. */
    async function withMappings(...triggers: Array<{ kind: "note" | "cc"; data1: number }>) {
      const engine = ptmEngine();
      const { result } = setupApp();
      await settle();
      await result.setPtmOn(true);
      for (const t of triggers) {
        result.addMidiMapping({ action: "ptm", trigger: { ...t, channel: 0 } });
      }
      const ids = result.midiMappings().map((m) => m.id);
      invokeMock.mockClear();
      return { result, engine, ids };
    }

    it("two pads hold until both are up", async () => {
      const { result, engine } = await withMappings(
        { kind: "note", data1: 36 },
        { kind: "note", data1: 37 },
      );
      result.handleMidiMessage(noteOn(36));
      result.handleMidiMessage(noteOn(37));
      result.handleMidiMessage(noteOff(36));
      // Pad B is still down: no release.
      expect(ptmCalls()).toEqual([heldCall("midi", true)]);
      await settle();
      expect(engine.held()).toEqual(["midi"]);
      result.handleMidiMessage(noteOff(37));
      expect(ptmCalls()).toEqual([heldCall("midi", true), heldCall("midi", false)]);
      // A stray note-off sends nothing.
      result.handleMidiMessage(noteOff(37));
      expect(ptmCalls()).toHaveLength(2);
    });

    it("a CC holds at 64 or more and lets go at 59 or less; jitter at halfway doesn't flap", async () => {
      const { result } = await withMappings({ kind: "cc", data1: 20 });
      for (const v of [63, 64, 62, 64, 60, 63]) result.handleMidiMessage(cc(20, v));
      expect(ptmCalls()).toEqual([heldCall("midi", true)]);
      result.handleMidiMessage(cc(20, 59));
      await settle();
      expect(ptmCalls()).toEqual([heldCall("midi", true), heldCall("midi", false)]);
    });

    it.each([
      ["another device", "Deck B"],
      ["None", null],
    ] as const)("picking the input (%s) while a pad is held lets go", async (_, pick) => {
      const { result, engine } = await withMappings({ kind: "note", data1: 36 });
      result.handleMidiMessage(noteOn(36));
      await result.selectMidiInput(pick);
      await settle();
      expect(ptmCalls()).toEqual([heldCall("midi", true), heldCall("midi", false)]);
      expect(engine.held()).toEqual([]);
      // The old port's note-off, if it ever came, sends nothing.
      result.handleMidiMessage(noteOff(36));
      expect(ptmCalls()).toHaveLength(2);
    });

    it("entering learn on another mapping while a pad is held lets go", async () => {
      const { result, engine, ids } = await withMappings(
        { kind: "note", data1: 36 },
        { kind: "note", data1: 37 },
      );
      result.handleMidiMessage(noteOn(36));
      // Learn on mapping Y; learn swallows pad X's note-off.
      result.setMidiLearnId(ids[1]!);
      await settle();
      expect(ptmCalls()).toEqual([heldCall("midi", true), heldCall("midi", false)]);
      result.handleMidiMessage(noteOff(36));
      expect(result.midiLearnId()).toBeNull();
      await settle();
      expect(engine.held()).toEqual([]);
      // Cancelling learn lets go of nothing more.
      result.handleMidiMessage(noteOn(36));
      result.setMidiLearnId(null);
      await settle();
      expect(ptmCalls()).toEqual([
        heldCall("midi", true),
        heldCall("midi", false),
        heldCall("midi", true),
      ]);
    });

    it("editing or removing the held mapping lets go; editing another doesn't", async () => {
      const { result, ids } = await withMappings(
        { kind: "note", data1: 36 },
        { kind: "note", data1: 37 },
      );
      result.handleMidiMessage(noteOn(36));
      result.updateMidiMapping(ids[1]!, { trigger: { kind: "note", data1: 38, channel: 0 } });
      await settle();
      expect(ptmCalls()).toEqual([heldCall("midi", true)]);
      result.updateMidiMapping(ids[0]!, { trigger: { kind: "note", data1: 40, channel: 0 } });
      await settle();
      expect(ptmCalls()).toEqual([heldCall("midi", true), heldCall("midi", false)]);

      invokeMock.mockClear();
      result.handleMidiMessage(noteOn(40));
      result.removeMidiMapping(ids[0]!);
      await settle();
      expect(ptmCalls()).toEqual([heldCall("midi", true), heldCall("midi", false)]);
    });

    it("after a mode change, a CC resting up doesn't hold until it goes down and back up", async () => {
      const { result, engine } = await withMappings({ kind: "cc", data1: 20 });
      result.handleMidiMessage(cc(20, 70));
      expect(ptmCalls()).toEqual([heldCall("midi", true)]);
      // The card's set lets go of every hold; the CC stays up.
      await result.setPtmHold("bypass");
      expect(engine.held()).toEqual([]);
      invokeMock.mockClear();
      result.handleMidiMessage(cc(20, 70));
      result.handleMidiMessage(cc(20, 72));
      expect(ptmCalls()).toEqual([]);
      // Down: the set is already empty, so nothing to let go.
      result.handleMidiMessage(cc(20, 50));
      expect(ptmCalls()).toEqual([]);
      result.handleMidiMessage(cc(20, 70));
      expect(ptmCalls()).toEqual([heldCall("midi", true)]);
    });
  });

  describe("v1.59.0: one command at a time", () => {
    const noteOn = (n: number) => ({ channel: 0, kind: "note-on" as const, data1: n, data2: 100 });
    const noteOff = (n: number) => ({ channel: 0, kind: "note-off" as const, data1: n, data2: 0 });

    it.each(["key", "button", "midi"] as const)(
      "%s: a release sent while its press is slow to answer goes out only after it",
      async (source) => {
        const engine = ptmEngine();
        const { result } = setupApp();
        await settle();
        await result.setPtmOn(true);
        result.addMidiMapping({ action: "ptm", trigger: { kind: "note", data1: 36, channel: 0 } });
        const press = () =>
          source === "midi" ? result.handleMidiMessage(noteOn(36)) : result.ptmDown(source);
        const release = () =>
          source === "midi" ? result.handleMidiMessage(noteOff(36)) : result.ptmUp(source);
        invokeMock.mockClear();
        engine.ctl.gate = [];
        press();
        release();
        await settle();
        expect(ptmCalls()).toEqual([heldCall(source, true)]);
        engine.ctl.gate.shift()!();
        await settle();
        expect(ptmCalls()).toEqual([heldCall(source, true), heldCall(source, false)]);
        engine.ctl.gate.shift()!();
        await settle();
        expect(engine.held()).toEqual([]);
        expect(result.ptmState()).toMatchObject({ held: false, bypassed: true });
      },
    );

    it("a press that fails doesn't hold back its release", async () => {
      const engine = ptmEngine();
      const { result } = setupApp();
      await settle();
      const real = invokeMock.getMockImplementation()!;
      let refuse = true;
      invokeMock.mockImplementation(async (cmd: string, a?: Record<string, unknown>) => {
        if (cmd === "set_ptm_held" && refuse) {
          refuse = false;
          throw new Error("refused");
        }
        return real(cmd, a);
      });
      const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
      invokeMock.mockClear();
      result.ptmDown("key");
      result.ptmUp("key");
      expect(ptmCalls()).toEqual([heldCall("key", true)]);
      await settle();
      expect(ptmCalls()).toEqual([heldCall("key", true), heldCall("key", false)]);
      expect(warn).toHaveBeenCalledTimes(1);
      expect(warn).toHaveBeenCalledWith("[ptm] set_ptm_held failed:", expect.any(Error));
      expect(engine.held()).toEqual([]);
    });

    it("a press made while the card's set is in flight goes after it, so the set can't let go of it", async () => {
      const engine = ptmEngine();
      const { result } = setupApp();
      await settle();
      invokeMock.mockClear();
      engine.ctl.gate = [];
      const on = result.setPtmOn(true);
      result.ptmDown("key");
      await settle();
      expect(ptmCalls()).toEqual([modeCall("apply")]);
      engine.ctl.gate.shift()!();
      await settle();
      expect(ptmCalls()).toEqual([modeCall("apply"), heldCall("key", true)]);
      engine.ctl.gate.shift()!();
      await on;
      await settle();
      expect(engine.held()).toEqual(["key"]);
      expect(result.status()).toBe("modulated");
    });

    it("a press made before the boot restore answers lands after it, over a wire that lands what is in flight together reversed", async () => {
      const engine = ptmEngine({ mode: "apply", seq: 3 });
      window.localStorage.setItem("divora.ptm", JSON.stringify({ on: true, hold: "apply" }));
      const wire = reversingWire(["set_ptm_mode", "set_ptm_held"]);
      const { result } = setupApp();
      result.ptmDown("key");
      expect(wire.sent).toEqual([modeCall("apply", true)]);
      await wire.drain();
      expect(wire.landed).toEqual([modeCall("apply", true), heldCall("key", true)]);
      expect(wire.peak()).toBe(1);
      // The restore let go of nothing the page holds now.
      expect(engine.held()).toEqual(["key"]);
      expect(result.ptmKnown()).toBe(true);
      expect(result.status()).toBe("modulated");
    });

    it("20 presses and releases, alternating, over a wire that lands what is in flight together reversed: the engine takes them in send order", async () => {
      const engine = ptmEngine();
      const { result } = setupApp();
      await settle();
      await result.setPtmOn(true);
      const wire = reversingWire(["set_ptm_mode", "set_ptm_held"]);
      for (let i = 0; i < 20; i++) {
        if (i % 2 === 0) result.ptmDown("button");
        else result.ptmUp("button");
      }
      await wire.drain();
      const want = Array.from({ length: 20 }, (_, i) => heldCall("button", i % 2 === 0));
      expect(wire.sent).toEqual(want);
      expect(wire.landed).toEqual(want);
      expect(wire.peak()).toBe(1);
      expect(engine.held()).toEqual([]);
      expect(result.ptmState()).toMatchObject({ held: false, bypassed: true });
    });

    it("each feature has its own queue: a press goes out at once while a mute and a timer command go unanswered, and so does each of them", async () => {
      const engine = ptmEngine();
      const real = invokeMock.getMockImplementation()!;
      invokeMock.mockImplementation((cmd: string, a?: Record<string, unknown>) =>
        cmd === "set_input_muted" || cmd === "pause_timer" ? new Promise(() => {}) : real(cmd, a),
      );
      const { result } = setupApp();
      await settle();
      invokeMock.mockClear();
      engine.ctl.hang = true;
      const sent = () => invokeMock.mock.calls.map((c) => c[0]);
      // Each in the caller's own turn, with the ones before it unanswered.
      result.ptmDown("key");
      expect(sent()).toEqual(["set_ptm_held"]);
      void result.setMuted(true);
      expect(sent()).toEqual(["set_ptm_held", "set_input_muted"]);
      void result.pauseTimer("timer-1");
      expect(sent()).toEqual(["set_ptm_held", "set_input_muted", "pause_timer"]);
    });
  });
});

// v1.59.0: Hourglass. Rust keeps the time; the store keeps `divora.timers`
// and a mirror of Rust's snapshot by `seq`. These drive the real store
// against a fake backend that answers as Rust does: every answer and event
// is the whole set, a sync seeds a timer from its `resume` only for an id it
// has never accepted, and an event reaches only a listener already in.
describe("app store — Hourglass (v1.59.0)", () => {
  type Handler = (e: { payload: unknown }) => void;
  const handlers = new Map<string, Handler[]>();
  /** Every command and listen the page made, in order. */
  const log: string[] = [];
  /** Every command with its arguments and `divora.timers` as it stood when
   *  the command went out. */
  const calls: Array<{ cmd: string; args: unknown; stored: string | null }> = [];
  const settle = async () => {
    for (let i = 0; i < 10; i++) await Promise.resolve();
    await new Promise<void>((r) => setTimeout(r, 0));
  };

  const KEY = "divora.timers";
  const ENDS = 1_793_217_900_000;

  /** A stored countdown "BRB" of 5:00 unless `more` says otherwise. */
  const entry = (id: string, more: Record<string, unknown> = {}) => ({
    id,
    name: "BRB",
    kind: "countdown",
    durationMs: 300_000,
    ...more,
  });
  const store = (...timers: unknown[]) =>
    window.localStorage.setItem(KEY, JSON.stringify({ version: 1, timers }));
  const saved = (): StoredTimer[] => {
    const raw = window.localStorage.getItem(KEY);
    return raw === null ? [] : (JSON.parse(raw) as StoredTimers).timers;
  };
  const savedRun = (id: string) => saved().find((t) => t.id === id)?.run;
  /** A timer as a snapshot has it: an idle 5:00 countdown "BRB" unless
   *  `more` says otherwise. */
  const timer = (id: string, more: Partial<TimerState> = {}): TimerState => ({
    id,
    name: "BRB",
    kind: "countdown",
    phase: "idle",
    ms: 300_000,
    totalMs: 300_000,
    text: "05:00",
    anchorMs: null,
    fires: 0,
    ...more,
  });
  /** The calls that are Hourglass's. */
  const timerLog = () =>
    log.filter(
      (c) =>
        c === "listen timer-state" ||
        c === "sync_timers" ||
        c === "timer_status" ||
        c === "timers_dir" ||
        c.endsWith("_timer"),
    );
  const argsOf = (cmd: string) => calls.filter((c) => c.cmd === cmd).map((c) => c.args);
  const synced = () => argsOf("sync_timers").map((a) => (a as { timers: TimerDefWire[] }).timers);
  const keyCalls = () =>
    calls
      .filter((c) => c.cmd.endsWith("register_global_shortcut"))
      .map((c) => [c.cmd, c.args]);

  /** Rust's side. A command changes the set when it arrives; `hold(cmd)`
   *  keeps its answer back until `release(cmd)`. `elsewhere` is a change
   *  made in Rust (a hotkey, a fire), emitted as the timer thread does: it
   *  reaches only listeners already registered. */
  function backend() {
    let seq = 0;
    let timers: TimerState[] = [];
    const seen = new Set<string>();
    const held = new Map<string, Array<() => void>>();
    const snapshot = (): TimerSnapshot => ({ seq: ++seq, timers: timers.map((t) => ({ ...t })) });
    const emit = (s: TimerSnapshot): void => {
      for (const h of handlers.get("timer-state") ?? []) h({ payload: s });
    };
    const seed = (d: TimerDefWire, first: boolean): TimerState => {
      const countdown = d.kind === "countdown";
      const r = first ? d.resume : undefined;
      const total = countdown ? (r?.totalMs ?? d.durationMs) : 0;
      return {
        id: d.id,
        name: d.name,
        kind: d.kind as TimerState["kind"],
        phase: r?.phase ?? "idle",
        ms: r?.phase === "paused" ? r.ms! : countdown ? total : 0,
        totalMs: total,
        text: "05:00",
        anchorMs: r?.phase === "running" ? r.anchorMs! : null,
        fires: 0,
      };
    };
    const change = (id: string, patch: Partial<TimerState>): void => {
      if (!timers.some((t) => t.id === id)) throw new Error("no such timer");
      timers = timers.map((t) => (t.id === id ? { ...t, ...patch } : t));
    };
    invokeMock.mockImplementation(async (cmd: string, a?: Record<string, unknown>) => {
      log.push(cmd);
      calls.push({ cmd, args: a, stored: window.localStorage.getItem(KEY) });
      let answer: unknown = null;
      const id = a?.id as string;
      switch (cmd) {
        case "sync_timers": {
          const defs = (a!.timers as TimerDefWire[]).filter(
            (d) => d.kind === "countdown" || d.kind === "countup",
          );
          timers = defs.slice(0, 16).map((d) => {
            const known = timers.find((t) => t.id === d.id);
            const first = !seen.has(d.id);
            seen.add(d.id);
            return known ? { ...known, name: d.name } : seed(d, first);
          });
          answer = snapshot();
          break;
        }
        case "timer_status":
          answer = snapshot();
          break;
        case "arm_timer":
          change(id, { phase: "running", anchorMs: ENDS + seq, text: "05:00" });
          answer = snapshot();
          break;
        case "pause_timer":
          change(id, { phase: "paused", ms: 200_000, anchorMs: null, text: "03:20" });
          answer = snapshot();
          break;
        case "resume_timer":
          change(id, { phase: "running", anchorMs: ENDS + seq });
          answer = snapshot();
          break;
        case "reset_timer":
          change(id, { phase: "idle", ms: 300_000, anchorMs: null, text: "05:00" });
          answer = snapshot();
          break;
        case "adjust_timer":
          change(id, { ms: 170_000, text: "02:50" });
          answer = snapshot();
          break;
        case "finish_timer": {
          const t = timers.find((x) => x.id === id);
          if (t?.kind === "countdown" && (t.phase === "running" || t.phase === "paused")) {
            change(id, { phase: "done", ms: 0, anchorMs: null, text: "00:00", fires: t.fires + 1 });
          } else {
            change(id, {});
          }
          answer = snapshot();
          break;
        }
        case "timers_dir":
          answer = "C:\\Users\\me\\AppData\\Roaming\\com.divora.voice\\timers";
          break;
      }
      const gate = held.get(cmd);
      if (gate) await new Promise<void>((r) => gate.push(r));
      return answer;
    });
    return {
      state: (id: string) => timers.find((t) => t.id === id),
      hold: (cmd: string) => held.set(cmd, []),
      release: (cmd: string) => {
        for (const r of held.get(cmd) ?? []) r();
        held.delete(cmd);
      },
      /** A change made in Rust, then the thread's emit. */
      elsewhere: (id: string, patch: Partial<TimerState>): TimerSnapshot => {
        change(id, patch);
        const s = snapshot();
        emit(s);
        return s;
      },
    };
  }

  /** Tauri's listen: with `holdTimerListen`, `timer-state`'s registers only
   *  when the test lets it; every other one at once. */
  let holdTimerListen = false;
  const pendingListens: Array<() => void> = [];

  beforeEach(() => {
    invokeMock.mockReset();
    handlers.clear();
    log.length = 0;
    calls.length = 0;
    holdTimerListen = false;
    pendingListens.length = 0;
    vi.mocked(listen).mockImplementation(((event: string, handler: Handler) => {
      log.push(`listen ${event}`);
      const register = () => {
        handlers.set(event, [...(handlers.get(event) ?? []), handler]);
        return () => handlers.set(event, (handlers.get(event) ?? []).filter((h) => h !== handler));
      };
      if (event === "timer-state" && holdTimerListen) {
        return new Promise((resolve) => pendingListens.push(() => resolve(register())));
      }
      return Promise.resolve(register());
    }) as unknown as typeof listen);
    window.localStorage.clear();
  });
  afterEach(() => {
    vi.mocked(listen).mockImplementation((async () => () => {}) as unknown as typeof listen);
    vi.restoreAllMocks();
    window.localStorage.clear();
  });

  describe("the boot", () => {
    it("without divora.timers: no timer command, no listener, and the key stays absent", async () => {
      backend();
      const { result } = setupApp();
      await settle();
      expect(timerLog()).toEqual([]);
      expect(result.timerStore().timers).toEqual([]);
      expect(result.timerSnapshot()).toBeNull();
      expect(window.localStorage.getItem(KEY)).toBeNull();
    });

    it("with it: sync_timers with each run as resume at creation, then the listener, then timer_status", async () => {
      const run = { phase: "running", totalMs: 300_000, anchorMs: ENDS };
      store(entry("timer-1", { run }), entry("timer-2", { name: "Uptime", kind: "countup" }));
      backend();
      const { result } = setupApp();
      // Sent while the store is created: nothing has been awaited yet.
      expect(timerLog()).toEqual(["sync_timers", "listen timer-state"]);
      expect(synced()).toEqual([
        [
          { id: "timer-1", name: "BRB", kind: "countdown", durationMs: 300_000, resume: run },
          { id: "timer-2", name: "Uptime", kind: "countup", durationMs: 300_000 },
        ],
      ]);
      await settle();
      expect(timerLog()).toEqual(["sync_timers", "listen timer-state", "timer_status"]);
      // Both answers adopted by seq: the pull's is the newer.
      expect(result.timerSnapshot()?.seq).toBe(2);
      expect(result.timerSnapshot()?.timers.map((t) => [t.id, t.phase])).toEqual([
        ["timer-1", "running"],
        ["timer-2", "idle"],
      ]);
      // The run Rust seeded is the one stored, so nothing was rewritten.
      expect(savedRun("timer-1")).toEqual(run);
    });

    it("a done emitted before the listener is in still lands, adopted and saved, through timer_status", async () => {
      store(entry("timer-1", { run: { phase: "running", totalMs: 300_000, anchorMs: ENDS } }));
      const be = backend();
      holdTimerListen = true;
      const { result } = setupApp();
      await settle();
      expect(result.timerSnapshot()?.timers[0]?.phase).toBe("running");
      // The restored countdown runs out while the listen is still on its way:
      // the event reaches nobody.
      be.elsewhere("timer-1", { phase: "done", ms: 0, text: "00:00", anchorMs: null, fires: 1 });
      expect(result.timerSnapshot()?.timers[0]?.phase).toBe("running");
      expect(timerLog()).toEqual(["sync_timers", "listen timer-state"]);
      pendingListens.shift()!();
      await settle();
      expect(timerLog()).toEqual(["sync_timers", "listen timer-state", "timer_status"]);
      expect(result.timerSnapshot()?.timers[0]).toMatchObject({ phase: "done", fires: 1 });
      expect(savedRun("timer-1")).toEqual({ phase: "done", totalMs: 300_000 });
    });
  });

  describe("the mirror and the stored runs", () => {
    it("an answer older than an event already adopted is ignored", async () => {
      store(entry("timer-1"));
      const be = backend();
      const { result } = setupApp();
      await settle();
      await result.startTimer("timer-1");
      expect(result.timerSnapshot()?.timers[0]?.phase).toBe("running");
      // Pause lands in Rust (its answer built), then the hotkey resumes it in
      // Rust and the thread emits; the pause's answer arrives last.
      be.hold("pause_timer");
      const pause = result.pauseTimer("timer-1");
      await settle();
      const resumed = be.elsewhere("timer-1", { phase: "running", anchorMs: ENDS + 99 });
      expect(result.timerSnapshot()).toEqual(resumed);
      be.release("pause_timer");
      await pause;
      expect(result.timerSnapshot()).toEqual(resumed);
      expect(savedRun("timer-1")).toEqual({ phase: "running", totalMs: 300_000, anchorMs: ENDS + 99 });
    });

    it("divora.timers is written on a transition, never for a text-only snapshot", async () => {
      store(entry("timer-1"));
      const be = backend();
      const { result } = setupApp();
      await settle();
      const setItem = vi.spyOn(Storage.prototype, "setItem");
      const writes = () => setItem.mock.calls.filter((c) => c[0] === KEY).length;

      await result.startTimer("timer-1");
      expect(writes()).toBe(1);
      const anchor = be.state("timer-1")!.anchorMs!;
      expect(savedRun("timer-1")).toEqual({ phase: "running", totalMs: 300_000, anchorMs: anchor });

      // Five seconds of a running countdown: the text changes, the run doesn't.
      for (let s = 1; s <= 5; s++) {
        be.elsewhere("timer-1", { ms: 300_000 - s * 1000, text: `04:${String(60 - s).padStart(2, "0")}` });
      }
      expect(result.timerSnapshot()?.timers[0]?.text).toBe("04:55");
      expect(writes()).toBe(1);

      await result.pauseTimer("timer-1");
      expect(writes()).toBe(2);
      expect(savedRun("timer-1")).toEqual({ phase: "paused", ms: 200_000, totalMs: 300_000 });
      await result.resetTimer("timer-1");
      expect(writes()).toBe(3);
      expect(saved()[0]).not.toHaveProperty("run");
      // An idle timer's snapshot again: nothing to write.
      await result.resetTimer("timer-1");
      expect(writes()).toBe(3);
    });

    it("a late snapshot never writes a deleted timer back", async () => {
      store(entry("timer-1"), entry("timer-2", { name: "Intro" }));
      backend();
      const { result } = setupApp();
      await settle();
      await result.deleteTimer("timer-1");
      expect(saved().map((t) => t.id)).toEqual(["timer-2"]);
      // A snapshot that still has timer-1 running, newer than anything held.
      const running = { phase: "running", ms: 299_000, text: "04:59", anchorMs: ENDS } as const;
      const late = {
        seq: result.timerSnapshot()!.seq + 1,
        timers: [timer("timer-1", running), timer("timer-2", { ...running, name: "Intro" })],
      };
      result.adoptTimerSnapshot(late);
      expect(result.timerSnapshot()?.seq).toBe(late.seq);
      expect(saved().map((t) => [t.id, t.run?.phase])).toEqual([["timer-2", "running"]]);
      expect(result.timerStore().timers.map((t) => t.id)).toEqual(["timer-2"]);
    });
  });

  describe("adding, renaming, lengths, deleting", () => {
    it("add: the lowest free timer-<n>, a default name, saved and synced; the first add starts the listener", async () => {
      backend();
      const { result } = setupApp();
      await settle();
      expect(timerLog()).toEqual([]);

      expect(await result.addTimer("countdown")).toBe("timer-1");
      await settle();
      expect(timerLog()).toEqual(["sync_timers", "listen timer-state", "timer_status"]);
      expect(saved()).toEqual([
        { id: "timer-1", name: "Countdown", kind: "countdown", durationMs: 300_000, hotkeys: {} },
      ]);
      expect(result.timerSnapshot()?.timers.map((t) => [t.id, t.phase])).toEqual([["timer-1", "idle"]]);

      expect(await result.addTimer("countup")).toBe("timer-2");
      expect(await result.addTimer("countdown")).toBe("timer-3");
      await result.deleteTimer("timer-1");
      expect(await result.addTimer("countdown")).toBe("timer-1");
      await settle();
      expect(saved().map((t) => [t.id, t.name])).toEqual([
        ["timer-2", "Count-up"],
        ["timer-3", "Countdown 2"],
        ["timer-1", "Countdown"],
      ]);
      // One listener for the page, however many adds.
      expect(log.filter((c) => c === "listen timer-state")).toHaveLength(1);
      expect(log.filter((c) => c === "timer_status")).toHaveLength(1);
      expect(synced().at(-1)?.map((d) => d.id)).toEqual(["timer-2", "timer-3", "timer-1"]);
    });

    it("refuses a 17th stored timer, a newer kind's counted too", async () => {
      store(
        ...Array.from({ length: 15 }, (_, i) => entry(`timer-${i + 1}`, { name: `T${i + 1}` })),
        entry("timer-16", { name: "Later", kind: "interval" }),
      );
      backend();
      const { result } = setupApp();
      await settle();
      const before = window.localStorage.getItem(KEY);
      log.length = 0;
      expect(await result.addTimer("countdown")).toBeNull();
      expect(timerLog()).toEqual([]);
      expect(window.localStorage.getItem(KEY)).toBe(before);
      expect(result.timerStore().timers).toHaveLength(16);
    });

    it("rename and a new length sync every stored run as resume; an unchanged commit sends nothing", async () => {
      const run = { phase: "running", totalMs: 300_000, anchorMs: ENDS };
      store(
        entry("timer-1", { run }),
        entry("timer-2", { name: "Intro", durationMs: 60_000 }),
        entry("timer-3", { name: "Uptime", kind: "countup" }),
      );
      backend();
      const { result } = setupApp();
      await settle();
      calls.length = 0;

      await result.renameTimer("timer-2", "  Starting soon ");
      expect(synced()).toHaveLength(1);
      expect(synced()[0]![0]).toEqual({
        id: "timer-1",
        name: "BRB",
        kind: "countdown",
        durationMs: 300_000,
        resume: run,
      });
      expect(synced()[0]![1]).toEqual({
        id: "timer-2",
        name: "Starting soon",
        kind: "countdown",
        durationMs: 60_000,
      });
      await result.renameTimer("timer-2", "Starting soon");
      await result.renameTimer("timer-2", "   ");
      await result.renameTimer("timer-9", "Nobody");
      expect(synced()).toHaveLength(1);

      await result.setTimerLength("timer-2", 600_000);
      expect(synced()).toHaveLength(2);
      expect(synced()[1]![1]).toMatchObject({ id: "timer-2", durationMs: 600_000 });
      expect(saved()[1]).toMatchObject({ name: "Starting soon", durationMs: 600_000 });
      // The same length again, a count-up, lengths no field gives: nothing.
      await result.setTimerLength("timer-2", 600_000);
      await result.setTimerLength("timer-3", 600_000);
      await result.setTimerLength("timer-2", 0);
      await result.setTimerLength("timer-2", 1500.5);
      await result.setTimerLength("timer-2", 86_400_001);
      expect(synced()).toHaveLength(2);
    });

    // v1.60.0: three keys, End now's included.
    it("delete unregisters its three keys first, then syncs without the timer", async () => {
      store(entry("timer-1", { hotkeys: { toggle: "F9" } }), entry("timer-2", { name: "Intro" }));
      backend();
      const { result } = setupApp();
      await settle();
      calls.length = 0;
      await result.deleteTimer("timer-1");
      expect(calls.map((c) => [c.cmd, c.args])).toEqual([
        ["unregister_global_shortcut", { id: "tm:timer-1:toggle" }],
        ["unregister_global_shortcut", { id: "tm:timer-1:reset" }],
        ["unregister_global_shortcut", { id: "tm:timer-1:finish" }],
        [
          "sync_timers",
          { timers: [{ id: "timer-2", name: "Intro", kind: "countdown", durationMs: 300_000 }] },
        ],
      ]);
      expect(saved().map((t) => t.id)).toEqual(["timer-2"]);
      expect(result.timerSnapshot()?.timers.map((t) => t.id)).toEqual(["timer-2"]);
    });

    it("an Add while the delete's keys are still on their way takes the id fresh: idle, fires 0, no run, its face kept", async () => {
      store(entry("timer-1"));
      const be = backend();
      const { result } = setupApp();
      await settle();
      await result.startTimer("timer-1");
      be.elsewhere("timer-1", { fires: 2, text: "03:12" });
      expect(savedRun("timer-1")?.phase).toBe("running");
      // The main thread is busy (a device enumeration): the unregisters'
      // answers wait. The user clicks Add countdown meanwhile.
      be.hold("unregister_global_shortcut");
      calls.length = 0;
      const deleted = result.deleteTimer("timer-1");
      await settle();
      const added = result.addTimer("countdown");
      await settle();
      result.setTimerOnOverlay("timer-1", true);
      be.release("unregister_global_shortcut");
      await deleted;
      expect(await added).toBe("timer-1");
      await settle();
      // The backend saw timer-1 go before the new one came.
      expect(calls.map((c) => c.cmd)).toEqual([
        "unregister_global_shortcut",
        "unregister_global_shortcut",
        "unregister_global_shortcut",
        "sync_timers",
        "sync_timers",
      ]);
      expect(synced()).toEqual([
        [],
        [{ id: "timer-1", name: "Countdown", kind: "countdown", durationMs: 300_000 }],
      ]);
      expect(result.timerSnapshot()?.timers).toEqual([
        expect.objectContaining({ id: "timer-1", name: "Countdown", phase: "idle", anchorMs: null, fires: 0 }),
      ]);
      expect(saved()).toEqual([
        { id: "timer-1", name: "Countdown", kind: "countdown", durationMs: 300_000, hotkeys: {} },
      ]);
      // The face turned on for the new timer meanwhile stays.
      expect(result.timerOnOverlay("timer-1")).toBe(true);
    });

    it("a delete that brings a timer under the limit registers its stored keys, after the sync", async () => {
      store(
        ...Array.from({ length: 16 }, (_, i) => entry(`timer-${i + 1}`, { name: `T${i + 1}` })),
        entry("timer-17", { name: "Over", hotkeys: { toggle: "F13", reset: "", finish: "F14" } }),
      );
      backend();
      const { result } = setupApp();
      await settle();
      await result.syncHotkeyBindings();
      expect(result.timerRoles().get("timer-17")).toBe("over");
      expect(keyCalls()).toEqual([]);
      calls.length = 0;
      await result.deleteTimer("timer-1");
      expect(result.timerRoles().get("timer-17")).toBe("run");
      expect(calls.map((c) => c.cmd)).toEqual([
        "unregister_global_shortcut",
        "unregister_global_shortcut",
        "unregister_global_shortcut",
        "sync_timers",
        "register_global_shortcut",
        "register_global_shortcut",
      ]);
      // v1.60.0: its End now key (`finish`) is registered too.
      expect(keyCalls()).toEqual([
        ["unregister_global_shortcut", { id: "tm:timer-1:toggle" }],
        ["unregister_global_shortcut", { id: "tm:timer-1:reset" }],
        ["unregister_global_shortcut", { id: "tm:timer-1:finish" }],
        ["register_global_shortcut", { id: "tm:timer-17:toggle", accelerator: "F13" }],
        ["register_global_shortcut", { id: "tm:timer-17:finish", accelerator: "F14" }],
      ]);
      expect(synced()[0]!.map((d) => d.id)).toContain("timer-17");
      // A timer that already ran registers nothing again.
      calls.length = 0;
      await result.deleteTimer("timer-2");
      expect(keyCalls()).toEqual([
        ["unregister_global_shortcut", { id: "tm:timer-2:toggle" }],
        ["unregister_global_shortcut", { id: "tm:timer-2:reset" }],
        ["unregister_global_shortcut", { id: "tm:timer-2:finish" }],
      ]);
    });

    // Needs track D's controller bodies (the scene's timer elements).
    it("delete takes the timer off the overlay", async () => {
      store(entry("timer-1"), entry("timer-2", { name: "Intro" }));
      backend();
      const { result } = setupApp();
      await settle();
      result.setTimerOnOverlay("timer-1", true);
      result.setTimerOnOverlay("timer-2", true);
      expect(result.timerOnOverlay("timer-1")).toBe(true);
      await result.deleteTimer("timer-1");
      expect(result.timerOnOverlay("timer-1")).toBe(false);
      expect(result.timerOnOverlay("timer-2")).toBe(true);
      const faces = result.overlayScene().elements.filter((e) => e.type === "timer");
      expect(faces.map((e) => e.timer)).toEqual(["timer-2"]);
    });

    // Needs track D's controller bodies: the scene is the toggle's only record.
    it("Show on overlay follows an Arrange delete", async () => {
      store(entry("timer-1"));
      backend();
      const { result } = setupApp();
      await settle();
      result.setTimerOnOverlay("timer-1", true);
      expect(result.timerOnOverlay("timer-1")).toBe(true);
      const face = result.overlayScene().elements.find((e) => e.type === "timer")!;
      for (const h of handlers.get("overlay:edit") ?? []) {
        h({ payload: { instance: 1, seq: 0, op: { kind: "remove", id: face.id } } });
      }
      expect(result.timerOnOverlay("timer-1")).toBe(false);
      result.setTimerOnOverlay("timer-1", true);
      expect(result.timerOnOverlay("timer-1")).toBe(true);
      result.setTimerOnOverlay("timer-1", false);
      expect(result.timerOnOverlay("timer-1")).toBe(false);
      expect(result.overlayScene().elements.filter((e) => e.type === "timer")).toEqual([]);
    });
  });

  describe("the buttons", () => {
    it("Start sends what its label says, from the mirror; nothing without state", async () => {
      invokeMock.mockResolvedValue(null);
      const { result } = setupApp();
      await settle();
      invokeMock.mockClear();
      await result.startTimer("timer-1");
      expect(invokeMock).not.toHaveBeenCalled();
      const sent: unknown[] = [];
      let seq = 0;
      for (const phase of ["idle", "running", "paused", "done", "expired"] as const) {
        result.adoptTimerSnapshot({
          seq: ++seq,
          timers: [timer("timer-1", { phase, anchorMs: phase === "running" ? ENDS : null })],
        });
        invokeMock.mockClear();
        await result.startTimer("timer-1");
        sent.push(...invokeMock.mock.calls);
      }
      expect(sent).toEqual([
        ["arm_timer", { id: "timer-1" }],
        ["pause_timer", { id: "timer-1" }],
        ["resume_timer", { id: "timer-1" }],
        ["arm_timer", { id: "timer-1" }],
        ["arm_timer", { id: "timer-1" }],
      ]);
    });

    it("Pause, Reset and ±30 s send their commands and adopt the answers", async () => {
      store(entry("timer-1"));
      backend();
      const { result } = setupApp();
      await settle();
      calls.length = 0;
      await result.startTimer("timer-1");
      await result.pauseTimer("timer-1");
      await result.adjustTimer("timer-1", -30_000);
      expect(result.timerSnapshot()?.timers[0]).toMatchObject({
        phase: "paused",
        ms: 170_000,
        text: "02:50",
      });
      await result.adjustTimer("timer-1", 30_000);
      await result.resetTimer("timer-1");
      expect(calls.map((c) => [c.cmd, c.args])).toEqual([
        ["arm_timer", { id: "timer-1" }],
        ["pause_timer", { id: "timer-1" }],
        ["adjust_timer", { id: "timer-1", deltaMs: -30_000 }],
        ["adjust_timer", { id: "timer-1", deltaMs: 30_000 }],
        ["reset_timer", { id: "timer-1" }],
      ]);
      expect(result.timerSnapshot()?.timers[0]).toMatchObject({ phase: "idle", text: "05:00" });
    });

    it("a refused command warns and changes nothing", async () => {
      store(entry("timer-1"));
      backend();
      const { result } = setupApp();
      await settle();
      const held = result.timerSnapshot();
      const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
      await result.pauseTimer("timer-9");
      expect(warn).toHaveBeenCalledWith("[hourglass] pause_timer failed:", expect.any(Error));
      expect(result.timerSnapshot()).toBe(held);
    });

    it("timersDir asks once, then remembers the folder", async () => {
      backend();
      const { result } = setupApp();
      const want = "C:\\Users\\me\\AppData\\Roaming\\com.divora.voice\\timers";
      expect(await result.timersDir()).toBe(want);
      expect(await result.timersDir()).toBe(want);
      expect(timerLog()).toEqual(["timers_dir"]);
    });

    it("timersDir is null when the folder can't be had, and asks again next time", async () => {
      invokeMock.mockImplementation(async (cmd: string) => {
        log.push(cmd);
        if (cmd === "timers_dir") throw new Error("no app data");
        return null;
      });
      const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
      const { result } = setupApp();
      expect(await result.timersDir()).toBeNull();
      expect(await result.timersDir()).toBeNull();
      expect(timerLog()).toEqual(["timers_dir", "timers_dir"]);
      expect(warn).toHaveBeenCalledWith("[hourglass] timers_dir failed:", expect.any(Error));
    });
  });

  describe("hotkeys", () => {
    it("a bind saves its verb first, then registers tm:<id>:<verb>; a clear saves, then unregisters; other verbs stay", async () => {
      store(entry("timer-1", { hotkeys: { finish: "F10" } }));
      backend();
      const { result } = setupApp();
      await settle();
      calls.length = 0;
      /** The timer's keys as stored when the i-th call went out. */
      const storedAt = (i: number) =>
        (JSON.parse(calls[i]!.stored!) as StoredTimers).timers[0]!.hotkeys;

      await result.bindTimerHotkey("timer-1", "toggle", "Ctrl+Alt+F13");
      expect(storedAt(0)).toEqual({ finish: "F10", toggle: "Ctrl+Alt+F13" });
      await result.bindTimerHotkey("timer-1", "reset", "Win+F14");
      expect(saved()[0]!.hotkeys).toEqual({
        finish: "F10",
        toggle: "Ctrl+Alt+F13",
        reset: "Super+F14",
      });
      expect(keyCalls()).toEqual([
        ["register_global_shortcut", { id: "tm:timer-1:toggle", accelerator: "Ctrl+Alt+F13" }],
        ["register_global_shortcut", { id: "tm:timer-1:reset", accelerator: "Super+F14" }],
      ]);

      calls.length = 0;
      await result.bindTimerHotkey("timer-1", "toggle", "");
      expect(saved()[0]!.hotkeys).toEqual({ finish: "F10", reset: "Super+F14" });
      expect(keyCalls()).toEqual([["unregister_global_shortcut", { id: "tm:timer-1:toggle" }]]);
      expect(storedAt(0)).toEqual({ finish: "F10", reset: "Super+F14" });
      // A bind syncs nothing: the backend's set has no keys in it.
      expect(synced()).toEqual([]);
    });

    it("a bind for a timer no longer stored, or one the backend doesn't run, saves and sends nothing", async () => {
      // timer-1 and 16 more countdowns: after the delete, the 17th known one
      // (timer-19) is still over the limit.
      store(
        entry("timer-1"),
        entry("timer-2", { name: "Later", kind: "interval" }),
        ...Array.from({ length: 17 }, (_, i) => entry(`timer-${i + 3}`, { name: `T${i + 3}` })),
      );
      backend();
      const { result } = setupApp();
      await settle();
      await result.deleteTimer("timer-1");
      expect(result.timerRoles().get("timer-2")).toBe("newer");
      expect(result.timerRoles().get("timer-19")).toBe("over");
      const before = window.localStorage.getItem(KEY);
      calls.length = 0;
      await result.bindTimerHotkey("timer-1", "toggle", "F9");
      await result.bindTimerHotkey("timer-2", "toggle", "F9");
      await result.bindTimerHotkey("timer-19", "reset", "F9");
      await result.bindTimerHotkey("timer-99", "toggle", "F9");
      expect(calls).toEqual([]);
      expect(window.localStorage.getItem(KEY)).toBe(before);
    });

    // v1.60.0: End now's key (`finish`) is registered when bound.
    it("syncHotkeyBindings registers the timers the backend runs after the actions and tiles, End now's key included; never a newer kind's or one over the limit", async () => {
      window.localStorage.setItem("divora.hotkeyBindings", JSON.stringify({ panic: "F1" }));
      window.localStorage.setItem("divora.tileHotkeys", JSON.stringify({ "clip-a": ["F2"] }));
      store(
        entry("timer-1", { hotkeys: { toggle: "F9", reset: "F10", finish: "F11" } }),
        entry("timer-2", { name: "Later", kind: "interval", hotkeys: { toggle: "F12" } }),
        ...Array.from({ length: 15 }, (_, i) => entry(`timer-${i + 3}`, { name: `T${i + 3}` })),
        entry("timer-18", { name: "Over", hotkeys: { toggle: "F13" } }),
        entry("timer-19", { name: "Uptime", kind: "countup", hotkeys: { reset: "" } }),
      );
      backend();
      const { result } = setupApp();
      await settle();
      calls.length = 0;
      await result.syncHotkeyBindings();
      expect(keyCalls()).toEqual([
        ["register_global_shortcut", { id: "panic", accelerator: "F1" }],
        ["register_global_shortcut", { id: "sb:clip-a", accelerator: "F2" }],
        ["register_global_shortcut", { id: "tm:timer-1:toggle", accelerator: "F9" }],
        ["register_global_shortcut", { id: "tm:timer-1:reset", accelerator: "F10" }],
        ["register_global_shortcut", { id: "tm:timer-1:finish", accelerator: "F11" }],
      ]);
    });
  });

  describe("v1.59.0: one command at a time", () => {
    const ONE = { id: "timer-1" };

    it("a command sent while another is slow to answer goes out only after it", async () => {
      store(entry("timer-1"));
      const be = backend();
      const { result } = setupApp();
      await settle();
      await result.startTimer("timer-1");
      calls.length = 0;
      be.hold("pause_timer");
      const pause = result.pauseTimer("timer-1");
      const adjust = result.adjustTimer("timer-1", 30_000);
      await settle();
      expect(calls.map((c) => c.cmd)).toEqual(["pause_timer"]);
      be.release("pause_timer");
      await Promise.all([pause, adjust]);
      expect(calls.map((c) => [c.cmd, c.args])).toEqual([
        ["pause_timer", ONE],
        ["adjust_timer", { ...ONE, deltaMs: 30_000 }],
      ]);
      expect(result.timerSnapshot()?.timers[0]).toMatchObject({ phase: "paused", ms: 170_000 });
    });

    it("a refused command doesn't hold back the next; each caller gets its own outcome", async () => {
      store(entry("timer-1"));
      backend();
      const { result } = setupApp();
      await settle();
      const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
      calls.length = 0;
      const refused = result.pauseTimer("timer-9");
      const next = result.adjustTimer("timer-1", -30_000);
      expect(calls.map((c) => c.cmd)).toEqual(["pause_timer"]);
      await Promise.all([refused, next]);
      expect(calls.map((c) => c.cmd)).toEqual(["pause_timer", "adjust_timer"]);
      expect(warn).toHaveBeenCalledTimes(1);
      expect(warn).toHaveBeenCalledWith("[hourglass] pause_timer failed:", expect.any(Error));
      expect(result.timerSnapshot()?.timers[0]).toMatchObject({ ms: 170_000, text: "02:50" });
    });

    it("20 resumes and pauses, alternating, through the store's actions over a wire that lands what is in flight together reversed: Rust takes them in send order", async () => {
      store(entry("timer-1"));
      const be = backend();
      const { result } = setupApp();
      await settle();
      await result.startTimer("timer-1");
      // Paused in the mirror, so Start sends resume_timer.
      await result.pauseTimer("timer-1");
      const wire = reversingWire(["pause_timer", "resume_timer"]);
      const done = Array.from({ length: 20 }, (_, i) =>
        i % 2 === 0 ? result.startTimer("timer-1") : result.pauseTimer("timer-1"),
      );
      await wire.drain();
      await Promise.all(done);
      const want = Array.from({ length: 20 }, (_, i) => [i % 2 === 0 ? "resume_timer" : "pause_timer", ONE]);
      expect(wire.sent).toEqual(want);
      expect(wire.landed).toEqual(want);
      expect(wire.peak()).toBe(1);
      expect(be.state("timer-1")?.phase).toBe("paused");
      expect(result.timerSnapshot()?.timers[0]?.phase).toBe("paused");
      expect(savedRun("timer-1")?.phase).toBe("paused");
    });

    it("a delete, an Add and a bind made while a command is slow to answer go after it, in that order", async () => {
      store(entry("timer-1", { hotkeys: { toggle: "F9" } }));
      const be = backend();
      const { result } = setupApp();
      await settle();
      await result.startTimer("timer-1");
      calls.length = 0;
      be.hold("pause_timer");
      const pause = result.pauseTimer("timer-1");
      const deleted = result.deleteTimer("timer-1");
      const added = result.addTimer("countdown");
      // The Add took the id; a key bound for it must outlive the delete's
      // unregister.
      const bound = result.bindTimerHotkey("timer-1", "toggle", "F10");
      await settle();
      expect(calls.map((c) => c.cmd)).toEqual(["pause_timer"]);
      be.release("pause_timer");
      await Promise.all([pause, deleted, added, bound]);
      await settle();
      expect(calls.map((c) => [c.cmd, c.args])).toEqual([
        ["pause_timer", ONE],
        ["unregister_global_shortcut", { id: "tm:timer-1:toggle" }],
        ["unregister_global_shortcut", { id: "tm:timer-1:reset" }],
        ["unregister_global_shortcut", { id: "tm:timer-1:finish" }],
        ["sync_timers", { timers: [] }],
        ["sync_timers", { timers: [{ ...ONE, name: "Countdown", kind: "countdown", durationMs: 300_000 }] }],
        ["register_global_shortcut", { id: "tm:timer-1:toggle", accelerator: "F10" }],
      ]);
      // The backend saw the id go before it came back: the new timer is
      // fresh.
      expect(result.timerSnapshot()?.timers).toEqual([
        expect.objectContaining({ ...ONE, name: "Countdown", phase: "idle", anchorMs: null, fires: 0 }),
      ]);
      expect(saved()).toEqual([
        { ...ONE, name: "Countdown", kind: "countdown", durationMs: 300_000, hotkeys: { toggle: "F10" } },
      ]);
    });

    it("a key cleared while its register is slow to answer is unregistered only after it", async () => {
      store(entry("timer-1"));
      const be = backend();
      const { result } = setupApp();
      await settle();
      calls.length = 0;
      be.hold("register_global_shortcut");
      const bound = result.bindTimerHotkey("timer-1", "toggle", "F10");
      const cleared = result.bindTimerHotkey("timer-1", "toggle", "");
      await settle();
      expect(keyCalls()).toEqual([["register_global_shortcut", { id: "tm:timer-1:toggle", accelerator: "F10" }]]);
      be.release("register_global_shortcut");
      await Promise.all([bound, cleared]);
      // Landing first, the unregister would leave F10 registered for a
      // timer whose card shows no key.
      expect(keyCalls()).toEqual([
        ["register_global_shortcut", { id: "tm:timer-1:toggle", accelerator: "F10" }],
        ["unregister_global_shortcut", { id: "tm:timer-1:toggle" }],
      ]);
      expect(saved()[0]?.hotkeys?.toggle).toBeUndefined();
    });

    it("the timers' keys App registers go after the boot sync; timer_status doesn't wait for it", async () => {
      store(entry("timer-1", { hotkeys: { toggle: "F9" } }));
      const be = backend();
      be.hold("sync_timers");
      const { result } = setupApp();
      const keys = result.syncHotkeyBindings();
      await settle();
      // The catch-up read isn't queued: it went while the sync is held.
      expect(timerLog()).toEqual(["sync_timers", "listen timer-state", "timer_status"]);
      const timerKeys = () => keyCalls().filter((c) => (c[1] as { id: string }).id.startsWith("tm:"));
      expect(timerKeys()).toEqual([]);
      be.release("sync_timers");
      await keys;
      expect(timerKeys()).toEqual([["register_global_shortcut", { id: "tm:timer-1:toggle", accelerator: "F9" }]]);
    });
  });

  // v1.60.0: End now (`finish_timer`) is one of the timers' commands.
  describe("v1.60.0: End now", () => {
    const ONE = { id: "timer-1" };

    it("End now sends finish_timer and adopts the answer", async () => {
      store(entry("timer-1"));
      backend();
      const { result } = setupApp();
      await settle();
      await result.startTimer("timer-1");
      calls.length = 0;
      await result.finishTimer("timer-1");
      expect(calls.map((c) => [c.cmd, c.args])).toEqual([["finish_timer", ONE]]);
      expect(result.timerSnapshot()?.timers[0]).toMatchObject({ phase: "done", fires: 1, text: "00:00" });
      expect(savedRun("timer-1")).toEqual({ phase: "done", totalMs: 300_000 });
    });

    it("an End now clicked while a Pause is slow to answer goes out only after it", async () => {
      store(entry("timer-1"));
      const be = backend();
      const { result } = setupApp();
      await settle();
      await result.startTimer("timer-1");
      calls.length = 0;
      be.hold("pause_timer");
      const pause = result.pauseTimer("timer-1");
      const finish = result.finishTimer("timer-1");
      await settle();
      expect(calls.map((c) => c.cmd)).toEqual(["pause_timer"]);
      be.release("pause_timer");
      await Promise.all([pause, finish]);
      expect(calls.map((c) => [c.cmd, c.args])).toEqual([
        ["pause_timer", ONE],
        ["finish_timer", ONE],
      ]);
      expect(result.timerSnapshot()?.timers[0]).toMatchObject({ phase: "done", fires: 1 });
    });

    it("a refused End now warns and changes nothing", async () => {
      store(entry("timer-1"));
      backend();
      const { result } = setupApp();
      await settle();
      const held = result.timerSnapshot();
      const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
      await result.finishTimer("timer-9");
      expect(warn).toHaveBeenCalledWith("[hourglass] finish_timer failed:", expect.any(Error));
      expect(result.timerSnapshot()).toBe(held);
    });

    it("timerReturn reads the stored return for a timer, and null without one", async () => {
      window.localStorage.setItem(
        KEY,
        JSON.stringify({
          version: 1,
          timers: [entry("timer-1"), entry("timer-2", { name: "Intro" })],
          presetReturns: [{ timer: "timer-1", from: "hollow-king", to: "villager" }, { timer: 5 }],
        }),
      );
      backend();
      const { result } = setupApp();
      await settle();
      expect(result.timerReturn("timer-1")).toEqual({ timer: "timer-1", from: "hollow-king", to: "villager" });
      expect(result.timerReturn("timer-2")).toBeNull();
    });
  });

  // ---- v1.60.0: timers that change your voice ----------------------------

  describe("v1.60.0: what a countdown's start and end do on the page", () => {
    const VILLAGER = { kind: "preset", presetId: "villager" } as const;
    const ORACLE = { kind: "preset", presetId: "the-oracle" } as const;
    const REVERT = { kind: "revertPreset" } as const;
    const MUTE = { kind: "setMuted", muted: true } as const;
    const UNMUTE = { kind: "setMuted", muted: false } as const;
    const OBS = { kind: "obs", op: "sceneBack" };
    const TILE = { id: "9f2c4e1a7b3d5f60", path: "C:\\sb\\intro.wav", label: "intro", extension: "wav", sizeBytes: 1 };
    const PLAY_TILE = { kind: "tile", tileId: TILE.id } as const;
    const LINE = (text: string) => ({ kind: "speak" as const, text, voiceId: "babble:bright" });
    const RUNNING = { phase: "running", totalMs: 300_000, anchorMs: ENDS };
    const VOICES = [
      { id: "af_heart", name: "Heart", lang: "en-us", installed: true, engine: "kokoro" },
      { id: "babble:bright", name: "Bright", lang: "en-us", installed: true, engine: "babble" },
    ];
    /** A user's own preset, with a chain no bundled one has. */
    const MY_KING = {
      id: "my-king",
      name: "My King",
      color: "#000000",
      glyph: "crown",
      tag: "User" as const,
      desc: "",
      chain: [
        { id: "gate" as const, enabled: true, vals: { thresh: -50 } },
        { id: "pitch" as const, enabled: true, vals: { shift: 9 } },
      ],
    };
    const ALL = [...FALLBACK_PRESETS, MY_KING];
    /** `list_presets`' answer: the bundled presets and My King. */
    const wirePresets = () => ALL.map((p) => ({ ...p, chain: p.chain.map((c) => ({ ...c, vals: { ...c.vals } })) }));
    const specsOf = (p: (typeof ALL)[number]) =>
      JSON.stringify(p.chain.map((c) => ({ kind: c.id, enabled: c.enabled, params: { ...c.vals } })));
    /** The preset each `set_effect_chain` sent, in order. */
    const switches = () =>
      argsOf("set_effect_chain").map((a) => {
        const sent = JSON.stringify((a as { specs: unknown }).specs);
        return ALL.find((p) => specsOf(p) === sent)?.id ?? "?";
      });
    /** Every command an action could send, as "cmd target". */
    const ACTION_CMDS = [
      "set_effect_chain",
      "set_voice_model",
      "play_soundboard_clip",
      "stop_soundboard_clip",
      "stop_all_soundboard_clips",
      "set_input_muted",
      "toggle_input_muted",
      "speak",
      "save_replay",
      "set_audio_monitor",
    ];
    const actionCmds = () => {
      let n = 0;
      return calls
        .filter((c) => ACTION_CMDS.includes(c.cmd))
        .map((c) => {
          if (c.cmd === "set_effect_chain") return `set_effect_chain ${switches()[n++]}`;
          if (c.cmd === "play_soundboard_clip") return `play_soundboard_clip ${(c.args as { clipId: string }).clipId}`;
          return c.cmd;
        });
    };
    const returnsStored = () => {
      const raw = window.localStorage.getItem(KEY);
      return raw === null ? undefined : (JSON.parse(raw) as StoredTimers).presetReturns;
    };
    const ret = (timer: string, from: string, to: string) => ({ timer, from, to });
    /** The Villager flow (§1.2 A): Villager at the start, back at zero. */
    const villagerTimer = (more: Record<string, unknown> = {}) =>
      entry("timer-1", { name: "Villager", onStart: [VILLAGER], onFire: [REVERT], ...more });

    let seqN = 0;
    /** The timer thread's `timer-actions`, to the listeners registered. */
    const emitActions = (id: string, hook: "start" | "end", why: string, actions: unknown[]): void => {
      seqN += 1;
      const payload = { seq: seqN, id, hook, why, lateMs: 0, actions };
      for (const h of handlers.get("timer-actions") ?? []) h({ payload });
    };
    beforeEach(() => {
      seqN = 0;
    });

    /** Divora with `timers` (and `returns`) stored, live on `live`, the
     *  engine running and a soundboard of one clip. The preset list loads at
     *  once unless `holdPresets` (then `releasePresets`); `holdSync` keeps the
     *  boot sync from reaching the backend until `releaseSync`, so the pull
     *  lands first. The mute's word answers `muted`. Every command so far is
     *  then forgotten. */
    async function boot(
      o: {
        timers?: unknown[];
        returns?: unknown;
        live?: string;
        holdPresets?: boolean;
        holdSync?: boolean;
        muted?: boolean;
        noKey?: boolean;
        voices?: () => unknown;
        wait?: () => Promise<void>;
      } = {},
    ) {
      const wait = o.wait ?? settle;
      if (!o.noKey) {
        const stored: Record<string, unknown> = { version: 1, timers: o.timers ?? [] };
        if (o.returns !== undefined) stored.presetReturns = o.returns;
        window.localStorage.setItem(KEY, JSON.stringify(stored));
      }
      window.localStorage.setItem("divora.activePreset", JSON.stringify(o.live ?? "hollow-king"));
      const be = backend();
      const real = invokeMock.getMockImplementation()!;
      let openPresets: () => void = () => {};
      const presetGate = o.holdPresets ? new Promise<void>((r) => (openPresets = r)) : null;
      let openSync: () => void = () => {};
      let syncGate = o.holdSync ? new Promise<void>((r) => (openSync = r)) : null;
      let word = { muted: o.muted ?? false, seq: 1 };
      invokeMock.mockImplementation(async (cmd: string, a?: Record<string, unknown>) => {
        if (cmd === "sync_timers" && syncGate) {
          const gate = syncGate;
          syncGate = null;
          await gate;
        }
        const answer = await real(cmd, a);
        switch (cmd) {
          case "list_presets":
            if (presetGate) await presetGate;
            return wirePresets();
          case "input_mute":
            return { ...word };
          case "set_input_muted":
            if (word.muted !== a!.muted) word = { muted: a!.muted as boolean, seq: word.seq + 1 };
            return { ...word };
          case "play_soundboard_clip":
            return 3;
          case "list_tts_voices":
            return o.voices ? o.voices() : VOICES;
          default:
            return answer;
        }
      });
      const { result } = setupApp();
      result.setEngineRunning(true);
      result.setSoundboardTiles([TILE]);
      const loaded = result.refreshPresets();
      await wait();
      calls.length = 0;
      log.length = 0;
      return {
        result,
        be,
        releasePresets: async () => {
          openPresets();
          await loaded;
          await wait();
        },
        releaseSync: async () => {
          openSync();
          await wait();
        },
      };
    }

    it("a profile without timers listens to none of the three; the first add listens to all three, then pulls", async () => {
      const { result } = await boot({ noKey: true });
      const listens = () =>
        log.filter((c) =>
          ["listen timer-state", "listen timer-actions", "listen soundboard-stopped", "timer_status"].includes(c),
        );
      expect(listens()).toEqual([]);
      await result.addTimer("countdown");
      await settle();
      expect(listens()).toEqual([
        "listen timer-state",
        "listen timer-actions",
        "listen soundboard-stopped",
        "timer_status",
      ]);
    });

    describe("the dispatch table", () => {
      // Each kind a timer's event can carry, at the hook Rust sends it from:
      // what the page sends for it, and through which queue (a command of
      // that queue in flight holds it back).
      it.each([
        ["Switch preset", "start", [VILLAGER], "chain", ["set_effect_chain villager"]],
        ["Random preset (any that changes the voice but the live one)", "start", [{ kind: "randomPreset" }], "chain", [
          "set_effect_chain static-wraith",
        ]],
        ["Back to the preset before, at an end", "end", [REVERT], "chain", ["set_effect_chain hollow-king"]],
        ["Play a clip", "start", [PLAY_TILE], "board", [`play_soundboard_clip ${TILE.id}`]],
        ["Mute my voice (the backend's)", "start", [MUTE], null, []],
        ["Unmute my voice (the backend's)", "end", [UNMUTE], null, []],
        ["Speak a line (the backend's)", "end", [LINE("I'm back.")], null, []],
        ["Save a replay (the backend's)", "end", [{ kind: "saveReplay" }], null, []],
        ["Stop all clips and lines (the backend's)", "end", [{ kind: "panic" }], null, []],
        ["the glyph's toggles, which aren't timer actions", "end", [{ kind: "monitor" }, { kind: "mute" }], null, []],
        ["Back to the preset before at a start (never run there)", "start", [REVERT], null, []],
      ] as const)("%s", async (_name, hook, actions, queue, want) => {
        vi.spyOn(Math, "random").mockReturnValue(0);
        // Live on Villager with a return to Hollow King, so a revert has
        // somewhere to go; a start's switches come from Hollow King.
        const live = hook === "end" ? "villager" : "hollow-king";
        const { result, be } = await boot({
          timers: [villagerTimer({ run: RUNNING })],
          returns: hook === "end" ? [ret("timer-1", "hollow-king", "villager")] : undefined,
          live,
        });
        if (queue === "chain") {
          be.hold("set_effect_param");
          result.setChainParam(0, "thresh", -30);
        } else if (queue === "board") {
          be.hold("stop_soundboard_clip");
          void result.stopClip("another");
        }
        emitActions("timer-1", hook, hook === "start" ? "armed" : "zero", [...actions]);
        await settle();
        const first = queue === "board" ? ["stop_soundboard_clip"] : [];
        expect(actionCmds()).toEqual(first);
        be.release("set_effect_param");
        be.release("stop_soundboard_clip");
        await settle();
        expect(actionCmds()).toEqual([...first, ...want]);
      });

      it("dispatchAction from a timer runs the same way; from a glyph, as it always has", async () => {
        const { result } = await boot({ timers: [villagerTimer({ run: RUNNING })] });
        result.dispatchAction({ kind: "monitor" }, { kind: "timer", id: "timer-1", hook: "start" });
        result.dispatchAction({ kind: "mute" }, { kind: "timer", id: "timer-1", hook: "start" });
        result.dispatchAction({ kind: "panic" }, { kind: "timer", id: "timer-1", hook: "start" });
        result.dispatchAction({ kind: "setMuted", muted: true }, { kind: "timer", id: "timer-1", hook: "start" });
        await settle();
        expect(actionCmds()).toEqual([]);
        result.dispatchAction(VILLAGER, { kind: "timer", id: "timer-1", hook: "start" });
        await settle();
        expect(actionCmds()).toEqual(["set_effect_chain villager"]);
        expect(returnsStored()).toEqual([ret("timer-1", "hollow-king", "villager")]);

        // The glyph's five, unchanged (D1).
        calls.length = 0;
        result.dispatchGlyphAction({ kind: "preset", presetId: "clean" });
        result.dispatchGlyphAction({ kind: "tile", tileId: TILE.id });
        result.dispatchAction({ kind: "monitor" }, { kind: "glyph" });
        result.dispatchAction({ kind: "mute" }, { kind: "glyph" });
        result.dispatchAction({ kind: "panic" }, { kind: "glyph" });
        await settle();
        expect(actionCmds()).toEqual([
          "set_effect_chain clean",
          `play_soundboard_clip ${TILE.id}`,
          "set_audio_monitor",
          "set_input_muted",
          "stop_all_soundboard_clips",
        ]);
        expect(argsOf("set_input_muted")).toEqual([{ muted: true }]);
        // A glyph's switch notes no return.
        expect(returnsStored()).toEqual([ret("timer-1", "hollow-king", "villager")]);
      });
    });

    describe("switching preset and back (D17)", () => {
      it("the Villager flow: the start switches and notes where from; zero switches back and forgets it", async () => {
        const { result } = await boot({ timers: [villagerTimer()] });
        emitActions("timer-1", "start", "armed", [VILLAGER]);
        await settle();
        expect(switches()).toEqual(["villager"]);
        expect(result.presetId()).toBe("villager");
        expect(returnsStored()).toEqual([ret("timer-1", "hollow-king", "villager")]);
        expect(result.timerReturn("timer-1")).toEqual(ret("timer-1", "hollow-king", "villager"));
        emitActions("timer-1", "end", "zero", [REVERT]);
        await settle();
        expect(switches()).toEqual(["villager", "hollow-king"]);
        expect(result.presetId()).toBe("hollow-king");
        expect(returnsStored()).toBeUndefined();
        expect(result.timerReturn("timer-1")).toBeNull();
      });

      it("you moved on: no switch back, and the return goes", async () => {
        const { result } = await boot({ timers: [villagerTimer()] });
        emitActions("timer-1", "start", "armed", [VILLAGER]);
        await settle();
        result.usePreset("clean");
        emitActions("timer-1", "end", "zero", [REVERT]);
        await settle();
        expect(switches()).toEqual(["villager", "clean"]);
        expect(result.presetId()).toBe("clean");
        expect(returnsStored()).toBeUndefined();
      });

      it("an end with no Back to the preset before stays on the timer's preset and forgets the return", async () => {
        const { result } = await boot({ timers: [villagerTimer()] });
        emitActions("timer-1", "start", "armed", [VILLAGER]);
        emitActions("timer-1", "end", "reset", []);
        await settle();
        expect(switches()).toEqual(["villager"]);
        expect(result.presetId()).toBe("villager");
        expect(returnsStored()).toBeUndefined();
      });

      it("two timers that both switch back nest: the second to end goes back to before the first", async () => {
        const { result } = await boot({
          timers: [villagerTimer(), entry("timer-2", { name: "Oracle", onStart: [ORACLE], onFire: [REVERT] })],
        });
        emitActions("timer-1", "start", "armed", [VILLAGER]);
        emitActions("timer-2", "start", "armed", [ORACLE]);
        await settle();
        expect(returnsStored()).toEqual([
          ret("timer-1", "hollow-king", "villager"),
          ret("timer-2", "villager", "the-oracle"),
        ]);
        emitActions("timer-1", "end", "zero", [REVERT]);
        await settle();
        expect(result.presetId()).toBe("the-oracle");
        expect(returnsStored()).toEqual([ret("timer-2", "hollow-king", "the-oracle")]);
        emitActions("timer-2", "end", "zero", [REVERT]);
        await settle();
        expect(switches()).toEqual(["villager", "the-oracle", "hollow-king"]);
        expect(returnsStored()).toBeUndefined();
      });

      it("an event already seen does nothing again", async () => {
        const { result } = await boot({ timers: [villagerTimer()] });
        emitActions("timer-1", "start", "armed", [VILLAGER]);
        await settle();
        // The same seq again, and an older one.
        for (const h of handlers.get("timer-actions") ?? []) {
          h({ payload: { seq: 1, id: "timer-1", hook: "start", why: "armed", lateMs: 0, actions: [ORACLE] } });
          h({ payload: { seq: 0, id: "timer-1", hook: "end", why: "zero", lateMs: 0, actions: [REVERT] } });
        }
        await settle();
        expect(switches()).toEqual(["villager"]);
        expect(result.presetId()).toBe("villager");
      });

      it("on the Presets screen a timer's switch keeps the preset you are editing in view; elsewhere the view follows", async () => {
        const { result } = await boot({ timers: [villagerTimer()] });
        result.setNav("presets");
        result.viewPreset("clean");
        result.setAbSlot("B");
        emitActions("timer-1", "start", "armed", [VILLAGER]);
        await settle();
        expect(result.presetId()).toBe("villager");
        expect(result.viewedId()).toBe("clean");
        // Compare back on A, as for any switch (D18).
        expect(result.ui.ab).toBe("A");
        result.setNav("mixer");
        expect(result.viewedId()).toBe("villager");
        emitActions("timer-1", "end", "zero", [REVERT]);
        await settle();
        expect(result.presetId()).toBe("hollow-king");
        expect(result.viewedId()).toBe("hollow-king");
        // Your own Use on the Presets screen still shows what it made live.
        result.setNav("presets");
        result.usePreset("clean");
        expect(result.viewedId()).toBe("clean");
      });
    });

    describe("preset work waits for the real preset list (D16)", () => {
      it("a revert that arrives before the list loads switches back to a user's preset once it has", async () => {
        const { result, releasePresets } = await boot({
          timers: [villagerTimer({ run: RUNNING })],
          returns: [ret("timer-1", "my-king", "villager")],
          live: "villager",
          holdPresets: true,
        });
        // Ran out while Divora was closed (P6): its event reaches the page
        // before the list does. Judged against the bundled list, My King
        // would read as gone and the return would be dropped.
        emitActions("timer-1", "end", "ranOut", [REVERT]);
        await settle();
        expect(switches()).toEqual([]);
        expect(returnsStored()).toEqual([ret("timer-1", "my-king", "villager")]);
        await releasePresets();
        expect(switches()).toEqual(["my-king"]);
        expect(result.presetId()).toBe("my-king");
        expect(returnsStored()).toBeUndefined();
      });

      it("reconcile waits for it too: a stored ended run's return is settled once it has loaded", async () => {
        const { result, releasePresets } = await boot({
          timers: [villagerTimer({ run: { phase: "done", totalMs: 300_000 } })],
          returns: [ret("timer-1", "my-king", "villager")],
          live: "villager",
          holdPresets: true,
        });
        expect(result.timerSnapshot()?.timers[0]?.phase).toBe("done");
        expect(switches()).toEqual([]);
        expect(returnsStored()).toEqual([ret("timer-1", "my-king", "villager")]);
        await releasePresets();
        expect(switches()).toEqual(["my-king"]);
        expect(returnsStored()).toBeUndefined();
      });

      it("a clip in the same event plays at once; the switch waits, in the order it came", async () => {
        const { result, releasePresets } = await boot({ timers: [villagerTimer()], holdPresets: true });
        emitActions("timer-1", "start", "armed", [VILLAGER, PLAY_TILE]);
        await settle();
        expect(actionCmds()).toEqual([`play_soundboard_clip ${TILE.id}`]);
        // The mirror hasn't caught up with the start (its `timer-state` is
        // still on its way): the reconcile the list lets run must not read
        // the timer as idle and undo the switch it just made.
        expect(result.timerSnapshot()?.timers[0]?.phase).toBe("idle");
        await releasePresets();
        expect(actionCmds()).toEqual([`play_soundboard_clip ${TILE.id}`, "set_effect_chain villager"]);
        expect(result.presetId()).toBe("villager");
        expect(returnsStored()).toEqual([ret("timer-1", "hollow-king", "villager")]);
      });
    });

    describe("reconcile (§7.6)", () => {
      it("runs once, and only after the boot sync answered: a pull that lands first with no timers settles nothing", async () => {
        const { result, be, releaseSync } = await boot({
          timers: [villagerTimer({ run: RUNNING })],
          returns: [ret("timer-1", "hollow-king", "villager")],
          live: "villager",
          holdSync: true,
        });
        // The pull reached the backend before the boot sync: no timers yet.
        expect(result.timerSnapshot()?.timers).toEqual([]);
        expect(result.presetsLoaded()).toBe(true);
        expect(switches()).toEqual([]);
        expect(returnsStored()).toEqual([ret("timer-1", "hollow-king", "villager")]);
        await releaseSync();
        // Running: kept for its end.
        expect(result.timerSnapshot()?.timers[0]?.phase).toBe("running");
        expect(switches()).toEqual([]);
        expect(returnsStored()).toEqual([ret("timer-1", "hollow-king", "villager")]);
        // Once per page: a later list and a later end with no event change
        // nothing here.
        be.elsewhere("timer-1", { phase: "done", ms: 0, text: "00:00", anchorMs: null, fires: 1 });
        await result.refreshPresets();
        await settle();
        expect(switches()).toEqual([]);
        expect(returnsStored()).toEqual([ret("timer-1", "hollow-king", "villager")]);
      });

      it("settles a deleted timer's return by switching back, and drops a newer kind's", async () => {
        const { result } = await boot({
          timers: [entry("timer-2", { name: "Later", kind: "interval" })],
          returns: [ret("timer-1", "hollow-king", "villager"), ret("timer-2", "clean", "hollow-king")],
          live: "villager",
        });
        // The deleted one's switch back makes Hollow King live, but the
        // newer kind's return is dropped, never switched.
        expect(result.presetId()).toBe("hollow-king");
        expect(returnsStored()).toBeUndefined();
      });
    });

    describe("End now (AT8)", () => {
      it("an answer that lands before the end event settles nothing; the event does", async () => {
        const { result } = await boot({ timers: [villagerTimer()] });
        await result.startTimer("timer-1");
        emitActions("timer-1", "start", "armed", [VILLAGER]);
        await settle();
        await result.finishTimer("timer-1");
        await settle();
        expect(result.timerSnapshot()?.timers[0]).toMatchObject({ phase: "done", fires: 1 });
        expect(result.presetId()).toBe("villager");
        expect(returnsStored()).toEqual([ret("timer-1", "hollow-king", "villager")]);
        emitActions("timer-1", "end", "endNow", [REVERT]);
        await settle();
        expect(switches()).toEqual(["villager", "hollow-king"]);
        expect(returnsStored()).toBeUndefined();
      });
    });

    describe("Add break (BRB) (D26)", () => {
      it("BRB, 5:00, muting at the start and unmuting at zero: one save and one sync carrying both lists", async () => {
        const { result } = await boot({ noKey: true });
        expect(await result.addTimer("countdown", "break")).toBe("timer-1");
        const brb = { id: "timer-1", name: "BRB", kind: "countdown", durationMs: 300_000 };
        expect(synced()).toEqual([[{ ...brb, onStart: [MUTE], onFire: [UNMUTE] }]]);
        expect(saved()).toEqual([{ ...brb, hotkeys: {}, onStart: [MUTE], onFire: [UNMUTE] }]);
        expect(await result.addTimer("countdown", "break")).toBe("timer-2");
        expect(saved()[1]).toMatchObject({ id: "timer-2", name: "BRB 2" });
        expect(synced()).toHaveLength(2);
        // A count-up has no lists: no break.
        expect(await result.addTimer("countup", "break")).toBeNull();
        expect(synced()).toHaveLength(2);
      });
    });

    describe("editing the lists by index (D27)", () => {
      const lists = () => synced().map((defs) => defs[0]?.onStart ?? null);

      it("an action this build can't read survives adding and removing around it, and is never sent", async () => {
        const { result } = await boot({ timers: [entry("timer-1", { onStart: [OBS, MUTE] })] });
        await result.addTimerAction("timer-1", "onStart", PLAY_TILE);
        expect(saved()[0]!.onStart).toEqual([OBS, MUTE, PLAY_TILE]);
        await result.removeTimerAction("timer-1", "onStart", 1);
        expect(saved()[0]!.onStart).toEqual([OBS, PLAY_TILE]);
        expect(lists()).toEqual([[MUTE, PLAY_TILE], [PLAY_TILE]]);
        // Removing the one it can't read takes only that one.
        await result.removeTimerAction("timer-1", "onStart", 0);
        expect(saved()[0]!.onStart).toEqual([PLAY_TILE]);
      });

      it("Add is refused at 8 stored entries, the one it can't read counted", async () => {
        const seven = [UNMUTE, REVERT, PLAY_TILE, { kind: "saveReplay" }, { kind: "panic" }, LINE("Back."), VILLAGER];
        const { result } = await boot({ timers: [entry("timer-1", { onFire: [...seven, OBS] })] });
        const before = window.localStorage.getItem(KEY);
        await result.addTimerAction("timer-1", "onFire", { kind: "randomPreset" });
        expect(window.localStorage.getItem(KEY)).toBe(before);
        expect(synced()).toEqual([]);
      });

      it("a second line in a list is refused; Stop all clips and lines goes to the top; each list offers its kinds", async () => {
        const { result } = await boot({
          timers: [entry("timer-1", { onFire: [LINE("I'm back.")] }), entry("timer-2", { kind: "countup" })],
        });
        await result.addTimerAction("timer-1", "onFire", LINE("Hello."));
        expect(saved()[0]!.onFire).toEqual([LINE("I'm back.")]);
        await result.addTimerAction("timer-1", "onFire", { kind: "panic" });
        expect(saved()[0]!.onFire).toEqual([{ kind: "panic" }, LINE("I'm back.")]);
        // Back to the preset before only at zero; a line that can't be
        // read; a count-up.
        await result.addTimerAction("timer-1", "onStart", REVERT);
        await result.addTimerAction("timer-1", "onStart", LINE("   "));
        await result.addTimerAction("timer-2", "onStart", MUTE);
        expect(saved()[0]!.onStart).toBeUndefined();
        expect(saved()[1]!.onStart).toBeUndefined();
        expect(synced()).toHaveLength(1);
        // A line is stored as read: trimmed.
        await result.addTimerAction("timer-1", "onStart", LINE("  Be right back.  "));
        expect(saved()[0]!.onStart).toEqual([LINE("Be right back.")]);
      });

      it("Set changes a running row's target or line, keeping the entry's other keys; never its kind or a row that doesn't run", async () => {
        const { result } = await boot({
          timers: [entry("timer-1", { onStart: [{ kind: "tile", tileId: "a", later: 1 }, OBS, LINE("Be right back.")] })],
        });
        await result.setTimerAction("timer-1", "onStart", 0, { kind: "tile", tileId: "b" });
        await result.setTimerAction("timer-1", "onStart", 2, { kind: "speak", text: " Back soon. ", voiceId: "af_heart" });
        expect(saved()[0]!.onStart).toEqual([
          { kind: "tile", tileId: "b", later: 1 },
          OBS,
          { kind: "speak", text: "Back soon.", voiceId: "af_heart" },
        ]);
        expect(synced()).toHaveLength(2);
        await result.setTimerAction("timer-1", "onStart", 0, { kind: "tile", tileId: "b" });
        await result.setTimerAction("timer-1", "onStart", 0, MUTE);
        await result.setTimerAction("timer-1", "onStart", 1, MUTE);
        await result.setTimerAction("timer-1", "onStart", 3, MUTE);
        expect(synced()).toHaveLength(2);
      });
    });

    describe("deleting a timer with a return (D17)", () => {
      it("running, switching back at zero: kept for the cancel's event, which switches back", async () => {
        const { result } = await boot({ timers: [villagerTimer()] });
        await result.startTimer("timer-1");
        emitActions("timer-1", "start", "armed", [VILLAGER]);
        await settle();
        await result.deleteTimer("timer-1");
        expect(returnsStored()).toEqual([ret("timer-1", "hollow-king", "villager")]);
        expect(result.presetId()).toBe("villager");
        emitActions("timer-1", "end", "deleted", [REVERT]);
        await settle();
        expect(switches()).toEqual(["villager", "hollow-king"]);
        expect(returnsStored()).toBeUndefined();
      });

      it("running, no switch back at zero: dropped at once, and you stay", async () => {
        const { result } = await boot({ timers: [villagerTimer({ onFire: [] })] });
        await result.startTimer("timer-1");
        emitActions("timer-1", "start", "armed", [VILLAGER]);
        await settle();
        await result.deleteTimer("timer-1");
        expect(returnsStored()).toBeUndefined();
        expect(switches()).toEqual(["villager"]);
        expect(result.presetId()).toBe("villager");
      });

      it("ended with its end unheard: settled at once, switching back only when its At zero does", async () => {
        const { result, be } = await boot({
          timers: [villagerTimer(), entry("timer-2", { name: "One way", onStart: [ORACLE] })],
        });
        await result.startTimer("timer-1");
        emitActions("timer-1", "start", "armed", [VILLAGER]);
        await result.startTimer("timer-2");
        emitActions("timer-2", "start", "armed", [ORACLE]);
        await settle();
        // Both end while the page hears no `timer-actions` (a reload's gap).
        be.elsewhere("timer-1", { phase: "done", ms: 0, text: "00:00", anchorMs: null, fires: 1 });
        be.elsewhere("timer-2", { phase: "done", ms: 0, text: "00:00", anchorMs: null, fires: 1 });
        expect(returnsStored()).toEqual([
          ret("timer-1", "hollow-king", "villager"),
          ret("timer-2", "villager", "the-oracle"),
        ]);
        await result.deleteTimer("timer-2");
        expect(result.presetId()).toBe("the-oracle");
        expect(returnsStored()).toEqual([ret("timer-1", "hollow-king", "villager")]);
        // timer-1's Villager isn't live any more (the one-way timer left
        // the Oracle on): you moved on, so nothing switches.
        await result.deleteTimer("timer-1");
        expect(result.presetId()).toBe("the-oracle");
        expect(returnsStored()).toBeUndefined();
      });

      it("ended, switching back at zero, and still live: switches back at once", async () => {
        const { result, be } = await boot({ timers: [villagerTimer()] });
        await result.startTimer("timer-1");
        emitActions("timer-1", "start", "armed", [VILLAGER]);
        await settle();
        be.elsewhere("timer-1", { phase: "done", ms: 0, text: "00:00", anchorMs: null, fires: 1 });
        await result.deleteTimer("timer-1");
        await settle();
        expect(switches()).toEqual(["villager", "hollow-king"]);
        expect(returnsStored()).toBeUndefined();
      });
    });

    describe("the restart notice (D36)", () => {
      const fsettle = async () => {
        for (let i = 0; i < 10; i++) await Promise.resolve();
        await vi.advanceTimersByTimeAsync(0);
      };
      const brb = (run: unknown, onFire: unknown[] = [UNMUTE], id = "timer-1") =>
        entry(id, { onStart: [MUTE], onFire, run });
      beforeEach(() => {
        vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
      });
      afterEach(() => {
        vi.useRealTimers();
      });

      it("muted at launch by a running break: named once", async () => {
        const { result } = await boot({ timers: [brb(RUNNING)], muted: true, wait: fsettle });
        expect(result.toast()).toEqual({
          id: expect.any(Number),
          text: "BRB is running, so you're muted until it ends. Press End now or Reset to unmute.",
          tone: "error",
        });
        const shown = result.toast();
        await result.setMuted(false);
        await result.setMuted(true);
        await fsettle();
        expect(result.toast()).toBe(shown);
      });

      it("a paused break, and one whose At zero doesn't unmute, each say so", async () => {
        const paused = await boot({
          timers: [brb({ phase: "paused", ms: 100_000, totalMs: 300_000 })],
          muted: true,
          wait: fsettle,
        });
        expect(paused.result.toast()?.text).toBe(
          "BRB is paused, so you stay muted. Press End now or Reset to unmute.",
        );
      });

      it("At zero that doesn't unmute: the notice says where to unmute", async () => {
        const { result } = await boot({ timers: [brb(RUNNING, [])], muted: true, wait: fsettle });
        expect(result.toast()?.text).toBe(
          "BRB muted you when it started, and nothing at zero unmutes you. Unmute from the Mixer.",
        );
      });

      it("a mute that comes within 3 s of the reconcile is named; one after isn't", async () => {
        const late = await boot({ timers: [brb(RUNNING)], wait: fsettle });
        expect(late.result.toast()).toBeNull();
        await vi.advanceTimersByTimeAsync(2_000);
        await late.result.setMuted(true);
        await fsettle();
        expect(late.result.toast()?.text).toBe(
          "BRB is running, so you're muted until it ends. Press End now or Reset to unmute.",
        );
      });

      it("after 3 s, a mute is yours: nothing is said", async () => {
        const { result } = await boot({ timers: [brb(RUNNING)], wait: fsettle });
        await vi.advanceTimersByTimeAsync(3_001);
        await result.setMuted(true);
        await fsettle();
        expect(result.toast()).toBeNull();
      });

      it("muted with no muting countdown running: nothing is said", async () => {
        const { result } = await boot({
          timers: [
            entry("timer-1", { onFire: [UNMUTE], run: RUNNING }),
            brb({ phase: "done", totalMs: 300_000 }, [UNMUTE], "timer-2"),
          ],
          muted: true,
          wait: fsettle,
        });
        expect(result.toast()).toBeNull();
      });
    });

    it("soundboard-stopped: nothing shows as playing, and nothing is sent", async () => {
      const { result } = await boot({ timers: [] });
      await result.playClip(TILE);
      expect(Object.keys(result.playingClips)).toEqual([TILE.id]);
      for (const h of handlers.get("soundboard-stopped") ?? []) h({ payload: null });
      expect(Object.keys(result.playingClips)).toEqual([]);
      expect(log).not.toContain("stop_all_soundboard_clips");
    });

    describe("ensureTtsVoices", () => {
      it("fills the voices once and leaves Speak's choice alone", async () => {
        window.localStorage.setItem("divora.ttsVoice", JSON.stringify("clone-1"));
        const { result } = await boot({ timers: [] });
        expect(result.ttsVoicesLoaded()).toBe(false);
        await Promise.all([result.ensureTtsVoices(), result.ensureTtsVoices()]);
        await result.ensureTtsVoices();
        expect(log.filter((c) => c === "list_tts_voices")).toHaveLength(1);
        expect(result.ttsVoices().map((v) => v.id)).toEqual(["af_heart", "babble:bright"]);
        expect(result.selectedTtsVoice()).toBe("clone-1");
        expect(result.ttsVoicesLoaded()).toBe(true);
      });

      it("a list that fails still ends the loading, and the next call asks again", async () => {
        let fail = true;
        const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
        const { result } = await boot({
          timers: [],
          voices: () => {
            if (fail) throw new Error("no voices");
            return VOICES;
          },
        });
        await result.ensureTtsVoices();
        expect(result.ttsVoicesLoaded()).toBe(true);
        expect(result.ttsVoices()).toEqual([]);
        expect(warn).toHaveBeenCalledWith("[tts] voice list for a timer failed", expect.any(Error));
        fail = false;
        await result.ensureTtsVoices();
        expect(result.ttsVoices()).toHaveLength(2);
      });
    });
  });
});

// ---- v1.60.0: the effect chain's and the soundboard's commands ----------

describe("app store — one chain and one soundboard command at a time (v1.60.0)", () => {
  const settle = async () => {
    for (let i = 0; i < 10; i++) await Promise.resolve();
    await new Promise<void>((r) => setTimeout(r, 0));
  };
  const CHAIN = ["set_effect_chain", "set_effect_param", "set_effect_enabled", "set_voice_model"];
  const BOARD = ["play_soundboard_clip", "stop_soundboard_clip", "stop_all_soundboard_clips"];
  const TILE = { id: "a1b2c3d4e5f60718", path: "C:\\sb\\a.wav", label: "a", extension: "wav", sizeBytes: 1 };
  const CLIP = { id: "c1", text: "Hi.", voice: "af_heart", createdAt: 0, durationSecs: 1, path: "C:\\speak\\c1.wav" };
  /** Every chain and soundboard command, in the order invoked. */
  let sent: string[] = [];
  /** `holding`'s answers, each kept until the test lets it go or fails it. */
  let held: Array<{ go: () => void; fail: (e: Error) => void }> = [];
  let holding: string | null = null;

  beforeEach(() => {
    sent = [];
    held = [];
    holding = null;
    invokeMock.mockReset();
    invokeMock.mockImplementation((cmd: string) => {
      if (CHAIN.includes(cmd) || BOARD.includes(cmd)) sent.push(cmd);
      const answer = cmd === "play_soundboard_clip" ? 2 : undefined;
      if (cmd === holding) return new Promise((r, f) => held.push({ go: () => r(answer), fail: f }));
      return Promise.resolve(answer);
    });
  });
  afterEach(() => {
    vi.restoreAllMocks();
    window.localStorage.clear();
  });

  /** A page live on `live` with the engine running, every send so far
   *  answered and forgotten. */
  async function page(live = "hollow-king") {
    window.localStorage.setItem("divora.activePreset", JSON.stringify(live));
    const { result } = setupApp();
    result.setEngineRunning(true);
    await settle();
    sent = [];
    return result;
  }

  type App = ReturnType<typeof useApp>;
  // Every sender §8.4 names, with the command it sends: the first command
  // of its queue is held, and it goes only once that one has answered; the
  // other queue isn't held up meanwhile.
  it.each<[string, "chain" | "board", string, string[], (a: App) => unknown]>([
    ["setChainParam", "chain", "hollow-king", ["set_effect_param"], (a) => a.setChainParam(0, "thresh", -30)],
    ["setChainEnabled", "chain", "hollow-king", ["set_effect_enabled"], (a) => a.setChainEnabled(0, false)],
    ["setActiveVoice (applyActiveVoice)", "chain", "deep-narrator-ai", ["set_voice_model"], (a) => a.setActiveVoice(null)],
    ["setAbSlot", "chain", "hollow-king", ["set_effect_chain"], (a) => a.setAbSlot("B")],
    ["syncChain", "chain", "deep-narrator-ai", ["set_effect_chain", "set_voice_model"], (a) => a.syncChain()],
    ["reorderChainEntries", "chain", "hollow-king", ["set_effect_chain"], (a) => a.reorderChainEntries(0, 1)],
    ["the active preset's effect (a switch)", "chain", "hollow-king", ["set_effect_chain"], (a) => a.usePreset("villager")],
    ["playClip", "board", "hollow-king", ["play_soundboard_clip"], (a) => a.playClip(TILE)],
    ["stopClip", "board", "hollow-king", ["stop_soundboard_clip"], (a) => a.stopClip(TILE.id)],
    ["panicSoundboard", "board", "hollow-king", ["stop_all_soundboard_clips"], (a) => a.panicSoundboard()],
    ["playSpeakClip", "board", "hollow-king", ["play_soundboard_clip"], (a) => a.playSpeakClip(CLIP)],
  ])("%s goes through the %s queue", async (_name, queue, live, cmds, send) => {
    const app = await page(live);
    // The queue's first command, slow to answer.
    if (queue === "chain") {
      holding = "set_effect_param";
      app.setChainParam(1, "thresh", -31);
    } else {
      holding = "stop_soundboard_clip";
      void app.stopClip("held");
    }
    const first = holding;
    void send(app);
    await settle();
    expect(sent).toEqual([first]);
    // The other queue goes at once.
    if (queue === "chain") void app.playClip(TILE);
    else app.setChainEnabled(1, false);
    await settle();
    const other = queue === "chain" ? "play_soundboard_clip" : "set_effect_enabled";
    expect(sent).toEqual([first, other]);
    holding = null;
    held.shift()!.go();
    await settle();
    expect(sent).toEqual([first, other, ...cmds]);
  });

  it("a switch's chain slow to answer keeps its voice model behind it", async () => {
    const app = await page();
    holding = "set_effect_chain";
    app.usePreset("deep-narrator-ai");
    await settle();
    expect(sent).toEqual(["set_effect_chain"]);
    holding = null;
    held.shift()!.go();
    await settle();
    expect(sent).toEqual(["set_effect_chain", "set_voice_model"]);
  });

  it("one chain per switch, then the voice model, and no chain after it (V1b)", async () => {
    const app = await page();
    invokeMock.mockClear();
    app.usePreset("deep-narrator-ai");
    await settle();
    const narrator = FALLBACK_PRESETS.find((p) => p.id === "deep-narrator-ai")!;
    expect(invokeMock.mock.calls.filter((c) => CHAIN.includes(c[0] as string))).toEqual([
      ["set_effect_chain", { specs: narrator.chain.map((c) => ({ kind: c.id, enabled: c.enabled, params: c.vals })) }],
      ["set_voice_model", { index: 2, path: null }],
    ]);
    // A preset without a model: one chain only. The live one again: none.
    sent = [];
    app.usePreset("villager");
    await settle();
    expect(sent).toEqual(["set_effect_chain"]);
    sent = [];
    app.usePreset("villager");
    await settle();
    expect(sent).toEqual([]);
  });

  it("a chain command that fails is logged and holds nothing back", async () => {
    const app = await page();
    const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
    holding = "set_effect_param";
    app.setChainParam(0, "thresh", -30);
    app.setChainEnabled(0, false);
    await settle();
    expect(sent).toEqual(["set_effect_param"]);
    holding = null;
    held.shift()!.fail(new Error("refused"));
    await settle();
    expect(warn).toHaveBeenCalledTimes(1);
    expect(warn).toHaveBeenCalledWith("[chain] set_effect_param failed:", expect.any(Error));
    expect(sent).toEqual(["set_effect_param", "set_effect_enabled"]);
  });

  it("20 params and 20 clips, alternating, over a wire that lands what is in flight together reversed: Rust takes each queue's in send order", async () => {
    const app = await page();
    const wire = reversingWire(["set_effect_param", "play_soundboard_clip", "stop_soundboard_clip"]);
    const done: unknown[] = [];
    for (let i = 0; i < 20; i++) {
      app.setChainParam(0, "thresh", -40 + i);
      done.push(i % 2 === 0 ? app.playClip(TILE) : app.stopClip(TILE.id));
    }
    await wire.drain();
    await Promise.all(done);
    const chainOrder = (l: Array<[string, unknown]>) =>
      l.filter(([c]) => c === "set_effect_param").map(([, a]) => (a as { value: number }).value);
    const boardOrder = (l: Array<[string, unknown]>) => l.filter(([c]) => c !== "set_effect_param").map(([c]) => c);
    const values = Array.from({ length: 20 }, (_, i) => -40 + i);
    const clips = Array.from({ length: 20 }, (_, i) => (i % 2 === 0 ? "play_soundboard_clip" : "stop_soundboard_clip"));
    expect(chainOrder(wire.landed)).toEqual(values);
    expect(boardOrder(wire.landed)).toEqual(clips);
    // One of each queue in flight at once, never more.
    expect(wire.peak()).toBe(2);
  });
});
