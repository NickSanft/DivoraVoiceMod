import { describe, expect, it, vi } from "vitest";
import { render } from "@solidjs/testing-library";
import App from "./App";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(async () => "pong"),
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
  it("renders the DivoraVoice wordmark in the titlebar", () => {
    const { getByText } = render(() => <App />);
    expect(getByText("DivoraVoice")).toBeInTheDocument();
  });

  it("shows the persistent LOCAL · NO ACCOUNT affirmation", () => {
    const { getByText } = render(() => <App />);
    expect(getByText(/LOCAL · NO ACCOUNT/)).toBeInTheDocument();
  });

  it("shows the Clean status by default", () => {
    const { getAllByText } = render(() => <App />);
    // The status pill in the titlebar and the eventual status card both
    // say "Clean" — at least the titlebar pill must be present.
    expect(getAllByText(/Clean/).length).toBeGreaterThanOrEqual(1);
  });
});
