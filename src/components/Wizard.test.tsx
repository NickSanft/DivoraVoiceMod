import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { render } from "@solidjs/testing-library";

const invokeMock = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
}));
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(async () => () => {
    /* unlisten no-op */
  }),
}));
vi.mock("@tauri-apps/plugin-shell", () => ({
  open: vi.fn(async () => undefined),
}));

import { AppProvider, useApp } from "../stores/app";
import { Wizard, clearWizardSeenFlag } from "./Wizard";
import type { JSX } from "solid-js";

function Harness(props: { children?: JSX.Element }): JSX.Element {
  return <AppProvider>{props.children}</AppProvider>;
}

function setup() {
  let capturedApp: ReturnType<typeof useApp> | null = null;
  function Inner() {
    capturedApp = useApp();
    return <Wizard />;
  }
  const utils = render(() => (
    <Harness>
      <Inner />
    </Harness>
  ));
  return { ...utils, app: () => capturedApp! };
}

describe("Wizard", () => {
  beforeEach(() => {
    invokeMock.mockReset();
    invokeMock.mockResolvedValue(undefined);
    // Default mock returns for the auto-refresh side effects.
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "list_audio_input_devices") return [];
      if (cmd === "list_audio_output_devices") return [];
      if (cmd === "list_presets") return [];
      if (cmd === "detect_virtual_mic") {
        return {
          detected: false,
          cableInputDevice: null,
          cableOutputDevice: null,
          downloadUrl: "https://vb-audio.com/Cable/",
        };
      }
      return null;
    });
    try {
      window.localStorage.clear();
    } catch {
      /* jsdom should always have localStorage */
    }
  });

  afterEach(() => {
    try {
      window.localStorage.clear();
    } catch {
      /* fine */
    }
  });

  it("renders on first launch (no seen flag) with the welcome step", () => {
    const { queryByText } = setup();
    expect(
      queryByText(/transmuted in real time/i),
    ).not.toBeNull();
  });

  it("auto-closes on mount when the seen flag is set", () => {
    window.localStorage.setItem("divora.wizardSeen", "true");
    const { queryByText, app } = setup();
    expect(app().wizardOpen()).toBe(false);
    expect(queryByText(/transmuted in real time/i)).toBeNull();
  });

  it("clearWizardSeenFlag erases the localStorage entry", () => {
    window.localStorage.setItem("divora.wizardSeen", "true");
    clearWizardSeenFlag();
    expect(window.localStorage.getItem("divora.wizardSeen")).toBeNull();
  });

  it("Continue advances through the four steps and Finish closes + sets seen", async () => {
    const { getByText, queryByText, app } = setup();
    expect(queryByText(/transmuted in real time/i)).not.toBeNull();

    // Step 0 → 1
    getByText("Continue").click();
    expect(queryByText(/A virtual cable carries your voice/i)).not.toBeNull();

    // Step 1 → 2
    getByText("Continue").click();
    expect(queryByText(/Pick your devices/i)).not.toBeNull();

    // Step 2 → 3
    getByText("Continue").click();
    expect(queryByText(/You.{0,3}re ready/i)).not.toBeNull();

    // Step 3 → finish
    getByText("Enter Divora").click();
    expect(app().wizardOpen()).toBe(false);
    expect(window.localStorage.getItem("divora.wizardSeen")).toBe("true");
  });

  it("Back walks the user back to the previous step", () => {
    const { getByText, queryByText } = setup();
    getByText("Continue").click();
    getByText("Continue").click();
    expect(queryByText(/Pick your devices/i)).not.toBeNull();
    getByText("Back").click();
    expect(queryByText(/A virtual cable carries your voice/i)).not.toBeNull();
  });

  it("Skip setup closes the wizard and marks it seen", () => {
    const { getByText, app } = setup();
    getByText("Skip setup").click();
    expect(app().wizardOpen()).toBe(false);
    expect(window.localStorage.getItem("divora.wizardSeen")).toBe("true");
  });

  it("opening the wizard re-runs the device + virtual-mic refresh", async () => {
    window.localStorage.setItem("divora.wizardSeen", "true");
    const { app } = setup();
    expect(app().wizardOpen()).toBe(false);
    invokeMock.mockClear();
    // Re-open via the store (mimicking About → Replay setup).
    clearWizardSeenFlag();
    app().setWizardOpen(true);
    // Settle microtasks for the createEffect.
    await Promise.resolve();
    await Promise.resolve();
    expect(invokeMock).toHaveBeenCalledWith("list_audio_input_devices");
    expect(invokeMock).toHaveBeenCalledWith("detect_virtual_mic");
  });
});

