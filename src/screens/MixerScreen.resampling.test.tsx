// The rate-mismatch indication — the parts only a rendered DOM can prove.
//
// This exists because of the history, not because the markup is tricky. Until
// v1.51.2 `StreamInfo` carried a single sample rate, so the app could not
// report — or even represent — two devices disagreeing, while the code that
// bridged the gap was destroying the audio: measured 0.00 dB of the input tone
// surviving to the output. A user could sit in that state indefinitely with
// nothing on screen to suggest why their voice sounded wrong.
//
// So the indication is load-bearing, and a badge that silently stops rendering
// would put us back where we started. Two tests: it appears when the rates
// differ, and it stays away when they match.

import { beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { render, waitFor } from "@solidjs/testing-library";

// The Mixer always mounts the cast-trail canvas; jsdom has no 2D context.
beforeAll(() => {
  HTMLCanvasElement.prototype.getContext = (() => null) as never;
});

const invokeMock = vi.fn();

vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
}));
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(async () => () => undefined),
}));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn(async () => null) }));
vi.mock("@tauri-apps/plugin-shell", () => ({
  open: vi.fn(async () => undefined),
}));

import { AppProvider, useApp } from "../stores/app";
import { MixerScreen } from "./MixerScreen";

type Rates = {
  sampleRate: number;
  outputRate: number;
  monitorRate: number | null;
};

function setup(rates: Rates) {
  invokeMock.mockImplementation(async (cmd: string) => {
    switch (cmd) {
      case "list_audio_input_devices":
      case "list_audio_output_devices":
      case "list_presets":
      case "list_voices":
      case "list_midi_inputs":
      case "list_tts_voices":
      case "list_cloned_voices":
      case "list_speak_clips":
        return [];
      case "start_audio_engine":
        return {
          inputName: "Focusrite",
          outputName: "CABLE Input",
          monitorName: rates.monitorRate === null ? null : "Headphones",
          inputChannels: 1,
          outputChannels: 2,
          ...rates,
        };
      default:
        return undefined;
    }
  });

  let api: ReturnType<typeof useApp> | null = null;
  function Harness() {
    api = useApp();
    return <MixerScreen />;
  }
  const utils = render(() => (
    <AppProvider>
      <Harness />
    </AppProvider>
  ));
  return { ...utils, app: () => api! };
}

beforeEach(() => {
  invokeMock.mockReset();
  localStorage.clear();
});

describe("the rate-mismatch indication", () => {
  it("names both rates when the devices disagree", async () => {
    const { app, container } = setup({
      sampleRate: 44_100,
      outputRate: 48_000,
      monitorRate: null,
    });
    await app().startEngine();
    await waitFor(() => {
      expect(app().engineRunning()).toBe(true);
    });
    await waitFor(() => {
      // Both rates, so the user can see WHICH pair to reconcile in Windows.
      expect(container.textContent).toContain("resampling");
      expect(container.textContent).toContain("44100");
      expect(container.textContent).toContain("48000");
    });
  });

  it("says nothing when both devices agree", async () => {
    const { app, container } = setup({
      sampleRate: 48_000,
      outputRate: 48_000,
      monitorRate: 48_000,
    });
    await app().startEngine();
    await waitFor(() => {
      expect(app().engineRunning()).toBe(true);
    });
    expect(container.textContent).not.toContain("resampling");
  });

  it("notices a monitor device that disagrees even when the main output does not", async () => {
    // The monitor stream resamples independently, and it is fed BY the output
    // callback rather than by the input device — a case the engine has to size
    // separately and this indication has to cover.
    const { app, container } = setup({
      sampleRate: 48_000,
      outputRate: 48_000,
      monitorRate: 44_100,
    });
    await app().startEngine();
    await waitFor(() => {
      expect(app().engineRunning()).toBe(true);
    });
    await waitFor(() => {
      expect(container.textContent).toContain("resampling");
    });
  });
});
