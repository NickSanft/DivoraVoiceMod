import { beforeEach, describe, expect, it, vi } from "vitest";

const invokeMock = vi.fn();
const listenMock = vi.fn();

vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
}));
vi.mock("@tauri-apps/api/event", () => ({
  listen: (...args: unknown[]) => listenMock(...args),
}));

import {
  clearEffectChain,
  closeMidiInput,
  deleteUserPreset,
  detectVirtualMic,
  exportPresetJson,
  getEngineStatus,
  listInputDevices,
  listMidiInputs,
  listOutputDevices,
  listPresets,
  openMidiInput,
  playSoundboardClip,
  presetStorePath,
  recordingsDir,
  registerGlobalShortcut,
  saveUserPreset,
  scanSoundboardFolder,
  setAudioMonitor,
  setEffectChain,
  setEffectEnabled,
  setEffectParam,
  speak,
  startAudioEngine,
  startRecording,
  listTtsVoices,
  stopAllSoundboardClips,
  stopAudioEngine,
  stopRecording,
  stopSoundboardClip,
  stopSpeak,
  subscribeGlobalShortcut,
  subscribeLevels,
  subscribeMidi,
  unregisterAllGlobalShortcuts,
  unregisterGlobalShortcut,
} from "./api";

describe("audio api", () => {
  beforeEach(() => {
    invokeMock.mockReset();
    listenMock.mockReset();
  });

  it("listInputDevices invokes list_audio_input_devices", async () => {
    invokeMock.mockResolvedValueOnce([
      { name: "Mic 1", isDefault: true, defaultSampleRate: 48000, channels: 1 },
    ]);
    const devices = await listInputDevices();
    expect(invokeMock).toHaveBeenCalledWith("list_audio_input_devices");
    expect(devices).toHaveLength(1);
  });

  it("listOutputDevices invokes list_audio_output_devices", async () => {
    invokeMock.mockResolvedValueOnce([]);
    await listOutputDevices();
    expect(invokeMock).toHaveBeenCalledWith("list_audio_output_devices");
  });

  it("startAudioEngine passes input/output names to the backend", async () => {
    invokeMock.mockResolvedValueOnce({
      inputName: "Mic 1",
      outputName: "Headphones",
      sampleRate: 48000,
      inputChannels: 1,
      outputChannels: 2,
    });
    await startAudioEngine("Mic 1", "Headphones", "Speakers");
    expect(invokeMock).toHaveBeenCalledWith("start_audio_engine", {
      inputName: "Mic 1",
      outputName: "Headphones",
      monitorName: "Speakers",
    });
  });

  it("startAudioEngine passes nulls when devices are not specified", async () => {
    invokeMock.mockResolvedValueOnce({
      inputName: "",
      outputName: "",
      sampleRate: 0,
      inputChannels: 0,
      outputChannels: 0,
    });
    await startAudioEngine();
    expect(invokeMock).toHaveBeenCalledWith("start_audio_engine", {
      inputName: null,
      outputName: null,
      monitorName: null,
    });
  });

  it("stopAudioEngine invokes stop_audio_engine", async () => {
    invokeMock.mockResolvedValueOnce(undefined);
    await stopAudioEngine();
    expect(invokeMock).toHaveBeenCalledWith("stop_audio_engine");
  });

  it("setAudioMonitor passes enabled flag", async () => {
    invokeMock.mockResolvedValueOnce(undefined);
    await setAudioMonitor(true);
    expect(invokeMock).toHaveBeenCalledWith("set_audio_monitor", { enabled: true });
  });

  it("getEngineStatus invokes audio_engine_status", async () => {
    invokeMock.mockResolvedValueOnce({
      running: true,
      monitoring: true,
      input: { rms: 0, peak: 0 },
      output: { rms: 0, peak: 0 },
    });
    const status = await getEngineStatus();
    expect(invokeMock).toHaveBeenCalledWith("audio_engine_status");
    expect(status.running).toBe(true);
  });

  it("setEffectChain forwards the spec list", async () => {
    invokeMock.mockResolvedValueOnce(undefined);
    await setEffectChain([
      { kind: "gate", enabled: true, params: { thresh: -48 } },
      { kind: "reverb", enabled: false, params: { size: 40, mix: 25 } },
    ]);
    expect(invokeMock).toHaveBeenCalledWith("set_effect_chain", {
      specs: [
        { kind: "gate", enabled: true, params: { thresh: -48 } },
        { kind: "reverb", enabled: false, params: { size: 40, mix: 25 } },
      ],
    });
  });

  it("setEffectParam forwards index/key/value", async () => {
    invokeMock.mockResolvedValueOnce(undefined);
    await setEffectParam(2, "shift", -5);
    expect(invokeMock).toHaveBeenCalledWith("set_effect_param", {
      index: 2,
      key: "shift",
      value: -5,
    });
  });

  it("setEffectEnabled forwards index + enabled flag", async () => {
    invokeMock.mockResolvedValueOnce(undefined);
    await setEffectEnabled(1, false);
    expect(invokeMock).toHaveBeenCalledWith("set_effect_enabled", {
      index: 1,
      enabled: false,
    });
  });

  it("clearEffectChain invokes clear_effect_chain", async () => {
    invokeMock.mockResolvedValueOnce(undefined);
    await clearEffectChain();
    expect(invokeMock).toHaveBeenCalledWith("clear_effect_chain");
  });

  it("listPresets invokes list_presets and returns wire presets", async () => {
    invokeMock.mockResolvedValueOnce([
      {
        id: "hollow-king",
        version: 1,
        name: "Hollow King",
        color: "#7C5CF6",
        glyph: "reverb",
        tag: "Bundled",
        desc: "Cavernous.",
        chain: [
          { id: "gate", enabled: true, vals: { thresh: -48 } },
        ],
      },
    ]);
    const presets = await listPresets();
    expect(invokeMock).toHaveBeenCalledWith("list_presets");
    expect(presets).toHaveLength(1);
    expect(presets[0]!.tag).toBe("Bundled");
  });

  it("saveUserPreset forwards the preset", async () => {
    invokeMock.mockResolvedValueOnce(undefined);
    const preset = {
      id: "my-voice",
      version: 1,
      name: "My Voice",
      color: "#34D9A0",
      glyph: "eq",
      tag: "User" as const,
      desc: "Custom.",
      chain: [],
    };
    await saveUserPreset(preset);
    expect(invokeMock).toHaveBeenCalledWith("save_user_preset", { preset });
  });

  it("deleteUserPreset forwards the id", async () => {
    invokeMock.mockResolvedValueOnce(undefined);
    await deleteUserPreset("my-voice");
    expect(invokeMock).toHaveBeenCalledWith("delete_user_preset", {
      id: "my-voice",
    });
  });

  it("exportPresetJson returns the backend's serialised string", async () => {
    invokeMock.mockResolvedValueOnce("{ pretty json }");
    const preset = {
      id: "x",
      version: 1,
      name: "X",
      color: "#fff",
      glyph: "clean",
      tag: "User" as const,
      desc: "",
      chain: [],
    };
    const json = await exportPresetJson(preset);
    expect(invokeMock).toHaveBeenCalledWith("export_preset_json", { preset });
    expect(json).toBe("{ pretty json }");
  });

  it("presetStorePath invokes preset_store_path", async () => {
    invokeMock.mockResolvedValueOnce("C:\\Users\\nick\\AppData\\presets");
    const p = await presetStorePath();
    expect(invokeMock).toHaveBeenCalledWith("preset_store_path");
    expect(p).toContain("presets");
  });

  it("scanSoundboardFolder forwards the folder path", async () => {
    invokeMock.mockResolvedValueOnce([
      {
        id: "abc",
        path: "C:/clips/bell.wav",
        label: "bell",
        extension: "wav",
        sizeBytes: 12_345,
      },
    ]);
    const tiles = await scanSoundboardFolder("C:/clips");
    expect(invokeMock).toHaveBeenCalledWith("scan_soundboard_folder", {
      folder: "C:/clips",
    });
    expect(tiles).toHaveLength(1);
    expect(tiles[0]!.label).toBe("bell");
  });

  it("playSoundboardClip forwards id + path and returns duration", async () => {
    invokeMock.mockResolvedValueOnce(3.5);
    const d = await playSoundboardClip("abc", "C:/clips/bell.wav", 0.5);
    expect(invokeMock).toHaveBeenCalledWith("play_soundboard_clip", {
      clipId: "abc",
      path: "C:/clips/bell.wav",
      gain: 0.5,
    });
    expect(d).toBe(3.5);
  });

  it("stopSoundboardClip forwards clipId", async () => {
    invokeMock.mockResolvedValueOnce(undefined);
    await stopSoundboardClip("abc");
    expect(invokeMock).toHaveBeenCalledWith("stop_soundboard_clip", {
      clipId: "abc",
    });
  });

  it("stopAllSoundboardClips invokes stop_all_soundboard_clips", async () => {
    invokeMock.mockResolvedValueOnce(undefined);
    await stopAllSoundboardClips();
    expect(invokeMock).toHaveBeenCalledWith("stop_all_soundboard_clips");
  });

  it("recordingsDir invokes recordings_dir", async () => {
    invokeMock.mockResolvedValueOnce("C:/Users/me/AppData/Roaming/DivoraVoice/recordings");
    const dir = await recordingsDir();
    expect(invokeMock).toHaveBeenCalledWith("recordings_dir");
    expect(dir).toContain("recordings");
  });

  it("startRecording forwards filename and returns the destination path", async () => {
    invokeMock.mockResolvedValueOnce("C:/rec/divora-2026-05-31_14-30-00.wav");
    const dest = await startRecording("divora-2026-05-31_14-30-00.wav");
    expect(invokeMock).toHaveBeenCalledWith("start_recording", {
      filename: "divora-2026-05-31_14-30-00.wav",
    });
    expect(dest).toMatch(/\.wav$/);
  });

  it("stopRecording invokes stop_recording", async () => {
    invokeMock.mockResolvedValueOnce(undefined);
    await stopRecording();
    expect(invokeMock).toHaveBeenCalledWith("stop_recording");
  });

  it("detectVirtualMic invokes detect_virtual_mic and returns the status", async () => {
    invokeMock.mockResolvedValueOnce({
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
    });
    const status = await detectVirtualMic();
    expect(invokeMock).toHaveBeenCalledWith("detect_virtual_mic");
    expect(status.detected).toBe(true);
    expect(status.cableInputDevice?.name).toContain("CABLE Input");
    expect(status.cableOutputDevice?.name).toContain("CABLE Output");
  });

  it("detectVirtualMic forwards a missing-cable status untouched", async () => {
    invokeMock.mockResolvedValueOnce({
      detected: false,
      cableInputDevice: null,
      cableOutputDevice: null,
      downloadUrl: "https://vb-audio.com/Cable/",
    });
    const status = await detectVirtualMic();
    expect(status.detected).toBe(false);
    expect(status.cableInputDevice).toBeNull();
    expect(status.cableOutputDevice).toBeNull();
    expect(status.downloadUrl).toMatch(/vb-audio/);
  });

  it("registerGlobalShortcut forwards id + accelerator", async () => {
    invokeMock.mockResolvedValueOnce(undefined);
    await registerGlobalShortcut("ptm", "Space");
    expect(invokeMock).toHaveBeenCalledWith("register_global_shortcut", {
      id: "ptm",
      accelerator: "Space",
    });
  });

  it("registerGlobalShortcut surfaces backend errors for bad accelerators", async () => {
    invokeMock.mockRejectedValueOnce(new Error("invalid accelerator: ZZZ"));
    await expect(registerGlobalShortcut("ptm", "ZZZ")).rejects.toThrow(
      /invalid accelerator/,
    );
  });

  it("unregisterGlobalShortcut forwards the id", async () => {
    invokeMock.mockResolvedValueOnce(undefined);
    await unregisterGlobalShortcut("ptm");
    expect(invokeMock).toHaveBeenCalledWith("unregister_global_shortcut", {
      id: "ptm",
    });
  });

  it("unregisterAllGlobalShortcuts invokes unregister_all_global_shortcuts", async () => {
    invokeMock.mockResolvedValueOnce(undefined);
    await unregisterAllGlobalShortcuts();
    expect(invokeMock).toHaveBeenCalledWith("unregister_all_global_shortcuts");
  });

  it("subscribeGlobalShortcut listens on global-shortcut and forwards payloads", async () => {
    let captured: unknown = null;
    listenMock.mockImplementationOnce(
      async (
        _event: string,
        handler: (e: { payload: unknown }) => void,
      ) => {
        handler({
          payload: { id: "ptm", accelerator: "Space", state: "pressed" },
        });
        return () => {
          /* unlisten */
        };
      },
    );
    await subscribeGlobalShortcut((event) => {
      captured = event;
    });
    expect(listenMock).toHaveBeenCalled();
    expect(listenMock.mock.calls[0]?.[0]).toBe("global-shortcut");
    expect(captured).toMatchObject({ id: "ptm", state: "pressed" });
  });

  it("subscribeLevels listens on audio-levels and forwards payloads", async () => {
    let captured: unknown = null;
    listenMock.mockImplementationOnce(
      async (
        _event: string,
        handler: (e: { payload: unknown }) => void,
      ) => {
        handler({
          payload: {
            input: { rms: 0.5, peak: 0.6 },
            output: { rms: 0.1, peak: 0.2 },
            running: true,
            monitoring: true,
          },
        });
        return () => {
          /* unlisten */
        };
      },
    );
    await subscribeLevels((update) => {
      captured = update;
    });
    expect(listenMock).toHaveBeenCalled();
    expect(listenMock.mock.calls[0]?.[0]).toBe("audio-levels");
    expect(captured).toMatchObject({ running: true });
  });

  it("listMidiInputs invokes list_midi_inputs", async () => {
    invokeMock.mockResolvedValueOnce([{ id: "Launchpad", name: "Launchpad" }]);
    const inputs = await listMidiInputs();
    expect(invokeMock).toHaveBeenCalledWith("list_midi_inputs");
    expect(inputs[0]!.name).toBe("Launchpad");
  });

  it("openMidiInput forwards the port name", async () => {
    invokeMock.mockResolvedValueOnce(undefined);
    await openMidiInput("Launchpad");
    expect(invokeMock).toHaveBeenCalledWith("open_midi_input", { name: "Launchpad" });
  });

  it("closeMidiInput invokes close_midi_input", async () => {
    invokeMock.mockResolvedValueOnce(undefined);
    await closeMidiInput();
    expect(invokeMock).toHaveBeenCalledWith("close_midi_input");
  });

  it("listTtsVoices invokes list_tts_voices", async () => {
    invokeMock.mockResolvedValueOnce([
      { id: "af_heart", name: "Aria", lang: "en-us", installed: false },
    ]);
    const voices = await listTtsVoices();
    expect(invokeMock).toHaveBeenCalledWith("list_tts_voices");
    expect(voices[0]!.id).toBe("af_heart");
    expect(voices[0]!.installed).toBe(false);
  });

  it("speak forwards text + voiceId (default gain/preview) and returns the duration", async () => {
    invokeMock.mockResolvedValueOnce(1.8);
    const d = await speak("Hello there.", "af_heart");
    expect(invokeMock).toHaveBeenCalledWith("speak", {
      text: "Hello there.",
      voiceId: "af_heart",
      gain: 1.0,
      previewOnly: false,
    });
    expect(d).toBe(1.8);
  });

  it("speak forwards an explicit gain + previewOnly", async () => {
    invokeMock.mockResolvedValueOnce(2.0);
    await speak("Hi", "bm_george", 0.5, true);
    expect(invokeMock).toHaveBeenCalledWith("speak", {
      text: "Hi",
      voiceId: "bm_george",
      gain: 0.5,
      previewOnly: true,
    });
  });

  it("speak surfaces a backend 'not installed' error", async () => {
    invokeMock.mockRejectedValueOnce("text-to-speech voices are not installed");
    await expect(speak("hi", "af_heart")).rejects.toBe(
      "text-to-speech voices are not installed",
    );
  });

  it("stopSpeak invokes stop_speak", async () => {
    invokeMock.mockResolvedValueOnce(undefined);
    await stopSpeak();
    expect(invokeMock).toHaveBeenCalledWith("stop_speak");
  });

  it("subscribeMidi listens on midi-message and forwards payloads", async () => {
    let captured: unknown = null;
    listenMock.mockImplementationOnce(
      async (
        _event: string,
        handler: (e: { payload: unknown }) => void,
      ) => {
        handler({
          payload: { channel: 0, kind: "note-on", data1: 60, data2: 100 },
        });
        return () => {
          /* unlisten */
        };
      },
    );
    await subscribeMidi((msg) => {
      captured = msg;
    });
    expect(listenMock.mock.calls[0]?.[0]).toBe("midi-message");
    expect(captured).toMatchObject({ kind: "note-on", data1: 60 });
  });
});
