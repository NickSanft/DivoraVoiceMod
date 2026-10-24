import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const invokeMock = vi.fn();
const shellOpen = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
}));
vi.mock("@tauri-apps/plugin-shell", () => ({
  open: (...args: unknown[]) => shellOpen(...args),
}));

import { openExternal } from "./openExternal";

// v1.57.0: links and folders open from the Windows desktop first, so a
// browser Divora starts isn't in its process tree; the old ways stay as the
// fallbacks.
describe("openExternal", () => {
  const windowOpen = vi.fn();
  beforeEach(() => {
    invokeMock.mockReset();
    shellOpen.mockReset();
    windowOpen.mockReset();
    vi.spyOn(window, "open").mockImplementation((...args) => {
      windowOpen(...args);
      return null;
    });
    vi.spyOn(console, "warn").mockImplementation(() => {});
  });
  afterEach(() => {
    vi.restoreAllMocks();
  });

  it("asks the backend to open it from the desktop, and nothing else", async () => {
    invokeMock.mockResolvedValue(undefined);
    await openExternal("https://vb-audio.com/Cable/");
    expect(invokeMock).toHaveBeenCalledTimes(1);
    expect(invokeMock).toHaveBeenCalledWith("open_external", {
      target: "https://vb-audio.com/Cable/",
    });
    expect(shellOpen).not.toHaveBeenCalled();
    expect(windowOpen).not.toHaveBeenCalled();
  });

  it("falls back to the shell plugin when the desktop can't open a link", async () => {
    invokeMock.mockRejectedValue("no desktop window: …");
    shellOpen.mockResolvedValue(undefined);
    await openExternal("https://github.com/NickSanft/DivoraVoiceMod/issues");
    expect(shellOpen).toHaveBeenCalledWith("https://github.com/NickSanft/DivoraVoiceMod/issues");
    expect(windowOpen).not.toHaveBeenCalled();
  });

  it("falls back to a plain anchor when neither can (a browser preview)", async () => {
    invokeMock.mockRejectedValue(new Error("no Tauri"));
    shellOpen.mockRejectedValue(new Error("no plugin"));
    await openExternal("https://github.com/NickSanft/DivoraVoiceMod");
    expect(windowOpen).toHaveBeenCalledWith(
      "https://github.com/NickSanft/DivoraVoiceMod",
      "_blank",
      "noopener",
    );
  });
});
