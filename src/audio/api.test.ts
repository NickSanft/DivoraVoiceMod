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
  getEngineStatus,
  listInputDevices,
  listOutputDevices,
  setAudioMonitor,
  startAudioEngine,
  stopAudioEngine,
  subscribeLevels,
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
    await startAudioEngine("Mic 1", "Headphones");
    expect(invokeMock).toHaveBeenCalledWith("start_audio_engine", {
      inputName: "Mic 1",
      outputName: "Headphones",
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
});
