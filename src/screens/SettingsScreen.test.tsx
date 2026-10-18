// v0.11.4: the Settings screen re-enumerates audio devices on mount
// and provides a manual "Refresh" button alongside the section header.
// Together with the App-level focus refresh, this covers all the paths
// a user can hit after plugging in a new mic or output:
//
//   - user plugs device in, alt-tabs back → App-level focus refresh
//   - user plugs device in, navigates to Settings → onMount refresh
//   - user plugs device in without alt-tabbing → manual button
//
// We exercise the latter two paths directly. Focus refresh is covered
// in `App.test.tsx`.

import { fireEvent, render, waitFor } from "@solidjs/testing-library";
import { beforeEach, describe, expect, it, vi } from "vitest";

const invokeCalls: Array<{ cmd: string; args?: unknown }> = [];
// v1.55.0: what the backend says about the overlay, and the listeners the
// store registered, so a test can send it a status.
let overlayReply: unknown = null;
const eventHandlers = new Map<string, Array<(e: { payload: unknown }) => void>>();

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(async (cmd: string, args?: unknown) => {
    invokeCalls.push({ cmd, args });
    switch (cmd) {
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
      case "detect_virtual_mic":
        return { installed: false, name: null };
      case "list_voices":
        return [];
      case "onnx_runtime_status":
        return { runtimeAvailable: false, voicesDir: "/tmp/DivoraVoice/voices" };
      case "overlay_status":
      case "set_overlay_hide_from_recordings":
      case "set_overlay_mode":
        return overlayReply;
      default:
        return null;
    }
  }),
}));
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(async (event: string, handler: (e: { payload: unknown }) => void) => {
    eventHandlers.set(event, [...(eventHandlers.get(event) ?? []), handler]);
    return () => {
      eventHandlers.set(
        event,
        (eventHandlers.get(event) ?? []).filter((h) => h !== handler),
      );
    };
  }),
}));
vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: vi.fn(() => ({
    onFocusChanged: vi.fn(async () => () => {
      /* noop */
    }),
  })),
}));

import { AppProvider } from "../stores/app";
import { SettingsScreen } from "./SettingsScreen";

/** Mount Settings with a real store but mocked IPC. */
function mount() {
  return render(() => (
    <AppProvider>
      <SettingsScreen />
    </AppProvider>
  ));
}

const flush = (): Promise<void> => new Promise((r) => setTimeout(r, 0));

describe("SettingsScreen — audio device refresh (v0.11.4)", () => {
  it("re-enumerates audio devices on mount", async () => {
    invokeCalls.length = 0;
    mount();
    await flush();
    // The store calls list_audio_*_devices once during init and again
    // when SettingsScreen mounts. The "again" is what we care about.
    const inputs = invokeCalls.filter(
      (c) => c.cmd === "list_audio_input_devices",
    );
    const outputs = invokeCalls.filter(
      (c) => c.cmd === "list_audio_output_devices",
    );
    expect(inputs.length).toBeGreaterThanOrEqual(1);
    expect(outputs.length).toBeGreaterThanOrEqual(1);
  });

  it("renders the Audio-devices Refresh button when idle", async () => {
    const { getByTitle } = mount();
    await flush();
    // There are two Refresh buttons now (Audio devices + Voice library);
    // target the Audio-devices one by its title.
    expect(
      getByTitle(/Re-scan input \+ output devices/i),
    ).toBeInTheDocument();
  });

  it("clicking the Audio-devices Refresh triggers a fresh enumeration", async () => {
    const { getByTitle } = mount();
    await flush();
    invokeCalls.length = 0;

    const btn = getByTitle(/Re-scan input \+ output devices/i);
    fireEvent.click(btn);
    await flush();
    // Click triggers an async refresh; wait for the IPC fan-out.
    await waitFor(() => {
      const inputs = invokeCalls.filter(
        (c) => c.cmd === "list_audio_input_devices",
      );
      const outputs = invokeCalls.filter(
        (c) => c.cmd === "list_audio_output_devices",
      );
      expect(inputs.length).toBeGreaterThanOrEqual(1);
      expect(outputs.length).toBeGreaterThanOrEqual(1);
    });
  });

  it("shows 'Audio devices' section eyebrow above the refresh control", async () => {
    const { getByText } = mount();
    await flush();
    expect(getByText(/Audio devices/i)).toBeInTheDocument();
  });
});

// v0.12.1 — Voice library section: lists installed voices, surfaces the
// ONNX runtime status, and lets the user pick the active voice (which
// drives the VoiceConvert effect).
describe("SettingsScreen — Voice library (v0.12.1)", () => {
  const flush = (): Promise<void> => new Promise((r) => setTimeout(r, 0));

  it("queries voices + runtime status on mount", async () => {
    invokeCalls.length = 0;
    mount();
    await flush();
    expect(invokeCalls.some((c) => c.cmd === "list_voices")).toBe(true);
    expect(invokeCalls.some((c) => c.cmd === "onnx_runtime_status")).toBe(true);
  });

  it("renders the Voice library section + runtime-not-installed guidance", async () => {
    const { getByText } = mount();
    await flush();
    expect(getByText(/Voice library/i)).toBeInTheDocument();
    // Mock returns runtimeAvailable: false → passthrough warning shown.
    expect(getByText(/ONNX Runtime not installed/i)).toBeInTheDocument();
  });

  it("offers a 'None — pass my voice through' option selected by default", async () => {
    const { getByText } = mount();
    await flush();
    expect(getByText(/pass my voice through/i)).toBeInTheDocument();
  });

  it("shows an empty hint when no voice models are installed", async () => {
    const { getByText } = mount();
    await flush();
    expect(getByText(/No voice models found/i)).toBeInTheDocument();
  });
});

// v1.55.0: Settings → Stream overlay. The store is real; the backend's
// overlay status comes through the mocked event bus.
describe("SettingsScreen — Stream overlay (v1.55.0)", () => {
  const flush = async (): Promise<void> => {
    for (let i = 0; i < 10; i++) await Promise.resolve();
    await new Promise((r) => setTimeout(r, 0));
  };
  const closed = {
    open: false,
    phase: "closed",
    instance: 0,
    mode: "locked",
    arrangeGen: 0,
    canArrange: false,
    fit: false,
    hideWanted: false,
    hideAvailable: false,
    capturing: false,
    error: null,
  };
  const emitStatus = (more: Record<string, unknown>) => {
    for (const h of eventHandlers.get("overlay:status") ?? []) h({ payload: { ...closed, ...more } });
  };
  const painted = { open: true, phase: "painted", instance: 1, canArrange: true };
  const wave = (id: string, z: number) => ({
    id,
    type: "image",
    x: 0.6,
    y: 0.6,
    w: 0.3,
    h: 0.168675,
    z,
    src: "a1b2c3d4e5f60718.gif",
    nw: 498,
    nh: 280,
    anim: true,
    bytes: 1834221,
    name: "wave.gif",
  });
  const storeScene = (elements: unknown[]) =>
    window.localStorage.setItem(
      "divora.overlayScene",
      JSON.stringify({ version: 1, designSize: { w: 480, h: 480 }, frame: "window", elements }),
    );
  const circle = { id: "circle", type: "spellcircle", x: 0.041667, y: 0.041667, w: 0.916667, h: 0.916667, z: 0 };

  beforeEach(() => {
    window.localStorage.clear();
    overlayReply = closed;
    invokeCalls.length = 0;
  });

  it("the rows render, and Hide from recordings only where Windows offers it", async () => {
    const first = mount();
    await flush();
    expect(first.getByRole("switch", { name: "Show stream overlay window" })).toBeInTheDocument();
    expect(first.getByRole("switch", { name: "Fit the overlay to its monitor" })).toBeInTheDocument();
    expect(first.getByTestId("overlay-arrange")).toBeInTheDocument();
    expect(first.getByTestId("overlay-charms")).toBeInTheDocument();
    expect(first.queryByRole("switch", { name: "Hide the overlay from recordings" })).toBeNull();
    // Still the words e2e looks for.
    expect(first.getByText(/capture it in OBS/i)).toBeInTheDocument();
    first.unmount();

    overlayReply = { ...closed, hideAvailable: true };
    const second = mount();
    await flush();
    expect(second.getByRole("switch", { name: "Hide the overlay from recordings" })).toBeInTheDocument();
    expect(second.getByText(/Windows is asked to leave the\s+locked overlay out/)).toBeInTheDocument();
  });

  it("Fit is off and says why on a colour background; colours are off while fitted", async () => {
    window.localStorage.setItem("divora.overlay", JSON.stringify({ bg: "green" }));
    const green = mount();
    await flush();
    expect(green.getByRole("switch", { name: "Fit the overlay to its monitor" })).toBeDisabled();
    expect(green.getByText("Fit to monitor needs the Transparent background.")).toBeInTheDocument();
    green.unmount();

    window.localStorage.setItem("divora.overlay", JSON.stringify({ bg: "transparent", fit: true }));
    const fitted = mount();
    await flush();
    expect(fitted.getByRole("switch", { name: "Fit the overlay to its monitor" })).not.toBeDisabled();
    expect(fitted.queryByText("Fit to monitor needs the Transparent background.")).toBeNull();
    for (const name of ["Green", "Magenta"]) {
      const tab = fitted.getByRole("tab", { name });
      expect(tab).toBeDisabled();
      expect(tab).toHaveAttribute("title", "Turn off Fit to monitor to use a colour background.");
    }
    expect(fitted.getByRole("tab", { name: "Transparent" })).not.toBeDisabled();
  });

  it("the Arrange button follows the backend: closed, starting, painted, arranging", async () => {
    const view = mount();
    await flush();
    const arrange = view.getByRole("button", { name: "Arrange the overlay" });
    expect(arrange).toBeDisabled();
    expect(arrange).toBe(view.getByTestId("overlay-arrange"));

    emitStatus({ open: true, phase: "starting", instance: 1 });
    const starting = view.getByTestId("overlay-arrange");
    expect(starting).toBeDisabled();
    expect(starting).toHaveTextContent("Starting…");
    expect(starting).not.toHaveAttribute("aria-label");
    expect(starting).not.toHaveAttribute("title");
    expect(view.getByRole("button", { name: "Starting…" })).toBe(starting);

    emitStatus(painted);
    overlayReply = { ...closed, ...painted, mode: "arrange", arrangeGen: 1 };
    fireEvent.click(view.getByRole("button", { name: "Arrange the overlay" }));
    await flush();
    expect(invokeCalls.filter((c) => c.cmd === "set_overlay_mode").map((c) => c.args)).toEqual([
      { mode: "arrange" },
    ]);

    const lock = view.getByRole("button", { name: "Lock the overlay" });
    expect(lock).toHaveTextContent("Lock");
    expect(lock).not.toHaveAttribute("aria-pressed");
    fireEvent.click(lock);
    await flush();
    expect(invokeCalls.filter((c) => c.cmd === "set_overlay_mode").map((c) => c.args)).toEqual([
      { mode: "arrange" },
      { mode: "locked" },
    ]);
  });

  it("the Charms list: the circle, each picture, distinct Remove names, and no pictures loaded", async () => {
    storeScene([circle, wave("c-1", 1), wave("c-2", 2)]);
    const view = mount();
    await flush();
    const charms = view.getByTestId("overlay-charms");
    // Front-most first.
    const rows = Array.from(charms.querySelectorAll("[data-charm-row]")).map((r) => r.getAttribute("data-charm-row"));
    expect(rows).toEqual(["c-2", "c-1", "circle"]);
    expect(view.getByText("Spell circle")).toBeInTheDocument();
    expect(view.getByText("wave.gif · 498×280 · animated")).toBeInTheDocument();
    expect(view.getByText("wave.gif (2) · 498×280 · animated")).toBeInTheDocument();
    expect(view.getByRole("button", { name: "Remove Spell circle" })).toBeInTheDocument();
    expect(view.getByRole("button", { name: "Remove wave.gif" })).toBeInTheDocument();
    expect(view.getByRole("button", { name: "Remove wave.gif (2)" })).toBeInTheDocument();
    expect(charms.querySelector("img")).toBeNull();
    expect(view.queryByRole("button", { name: "Add the spell circle" })).toBeNull();
    expect(view.getByText(/If you use OBS, add images in OBS/)).toBeInTheDocument();
  });

  it("Remove takes a charm out; the circle can come back", async () => {
    storeScene([circle, wave("c-1", 1)]);
    const view = mount();
    await flush();
    fireEvent.click(view.getByRole("button", { name: "Remove Spell circle" }));
    await flush();
    expect(view.queryByText("Spell circle")).toBeNull();
    fireEvent.click(view.getByRole("button", { name: "Add the spell circle" }));
    await flush();
    expect(view.getByText("Spell circle")).toBeInTheDocument();
    expect(view.queryByRole("button", { name: "Add the spell circle" })).toBeNull();
  });

  it("Add picture is off at 16, with the reason", async () => {
    storeScene([circle, ...Array.from({ length: 16 }, (_, i) => wave(`c-${i}`, i + 1))]);
    const view = mount();
    await flush();
    expect(view.getByRole("button", { name: /Add picture/ })).toBeDisabled();
    expect(view.getByText("The overlay holds up to 16 pictures.")).toBeInTheDocument();
  });

  it("the hotkey row is there, and doesn't say 'Stream overlay'", async () => {
    const view = mount();
    await flush();
    expect(view.getByText("Arrange or lock the overlay")).toBeInTheDocument();
    expect(view.getByText("Switches the overlay between Arrange and Locked while it's open.")).toBeInTheDocument();
    expect(view.getAllByText(/Stream overlay/i)).toHaveLength(1);
  });
});
