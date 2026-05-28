import { describe, expect, it, vi } from "vitest";
import { render } from "@solidjs/testing-library";
import App from "./App";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(async () => "pong"),
}));

describe("App", () => {
  it("renders the Divora heading", () => {
    const { getByText } = render(() => <App />);
    expect(getByText("Divora")).toBeInTheDocument();
  });

  it("renders the Phase 0 description", () => {
    const { getByText } = render(() => <App />);
    expect(getByText(/Phase 0 scaffold/)).toBeInTheDocument();
  });
});
