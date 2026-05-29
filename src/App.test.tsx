import { describe, expect, it, vi } from "vitest";
import { render } from "@solidjs/testing-library";
import App from "./App";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(async (cmd: string) => {
    switch (cmd) {
      case "ping":
        return "pong";
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
    minimize: vi.fn(),
    maximize: vi.fn(),
    unmaximize: vi.fn(),
    close: vi.fn(),
    isMaximized: vi.fn(async () => false),
  })),
}));

describe("App shell", () => {
  it("renders the DivoraVoice wordmark (titlebar + first-run wizard)", () => {
    const { getAllByText } = render(() => <App />);
    expect(getAllByText("DivoraVoice").length).toBeGreaterThanOrEqual(1);
  });

  it("shows the persistent LOCAL · NO ACCOUNT affirmation", () => {
    const { getByText } = render(() => <App />);
    expect(getByText(/LOCAL · NO ACCOUNT/)).toBeInTheDocument();
  });

  it("shows the Clean status by default", () => {
    const { getAllByText } = render(() => <App />);
    expect(getAllByText(/Clean/).length).toBeGreaterThanOrEqual(1);
  });
});
