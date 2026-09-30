// The rate-mismatch indication — the parts only a rendered DOM can prove.
//
// This exists because of the history, not because the markup is tricky. Until
// v1.51.2 `StreamInfo` carried a single sample rate, so the app could not
// report — or even represent — two devices disagreeing, while the code that
// bridged the gap was destroying the audio: the input tone ended up 41 dB below
// the noise it was buried in. A user could sit in that state indefinitely with
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
      // The exact string, so a badge that renders the wrong pair of rates
      // cannot pass. A lone output leg needs no label.
      expect(container.textContent).toContain("resampling 44100 → 48000 Hz");
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

  it("names the monitor when only the monitor disagrees", async () => {
    // The monitor converts from the ENGINE rate, independently of the main
    // output. The first version of this badge hard-coded the engine→output pair,
    // so this case rendered "resampling 48000 → 48000 Hz" — two identical
    // numbers — with a tooltip asserting the output was resampled when it was
    // not, and the 44100 Hz monitor appeared nowhere.
    //
    // That shipped past an earlier version of this very test, which asserted
    // only `toContain("resampling")`. Hence the exact string.
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
      expect(container.textContent).toContain("resampling 48000 → 44100 Hz monitor");
    });
    expect(container.textContent).not.toContain("48000 → 48000");
  });

  it("names both legs when the output and the monitor each disagree", async () => {
    const { app, container } = setup({
      sampleRate: 44_100,
      outputRate: 48_000,
      monitorRate: 96_000,
    });
    await app().startEngine();
    await waitFor(() => {
      expect(app().engineRunning()).toBe(true);
    });
    await waitFor(() => {
      // Both, and labelled, because which device to change is the actionable part.
      expect(container.textContent).toContain("44100 → 48000 Hz output");
      expect(container.textContent).toContain("44100 → 96000 Hz monitor");
    });
  });
});
