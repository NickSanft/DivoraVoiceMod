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

import { fireEvent, render, waitFor, within } from "@solidjs/testing-library";
import { beforeEach, describe, expect, it, vi } from "vitest";

const invokeCalls: Array<{ cmd: string; args?: unknown }> = [];
// v1.55.0: what the backend says about the overlay, and the listeners the
// store registered, so a test can send it a status.
let overlayReply: unknown = null;
const eventHandlers = new Map<string, Array<(e: { payload: unknown }) => void>>();
// v1.56.0: the output devices the backend lists (none unless a test says).
let outputDevices: unknown[] = [];
// v1.59.0: what the backend answers Hourglass's commands (null unless a test
// says, as the e2e mock answers).
let timerReply: (cmd: string, args: unknown) => unknown = () => null;

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(async (cmd: string, args?: unknown) => {
    invokeCalls.push({ cmd, args });
    switch (cmd) {
      case "list_audio_input_devices":
        return [];
      case "list_audio_output_devices":
        return outputDevices;
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
      case "sync_timers":
      case "arm_timer":
      case "pause_timer":
      case "resume_timer":
      case "reset_timer":
      case "adjust_timer":
      case "timer_status":
      case "timers_dir":
        return timerReply(cmd, args);
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

import { Sigil } from "../components/Sigil";
import { muteCcStep } from "../data/mute";
import { AppProvider, useApp } from "../stores/app";
import { HOURGLASS_SAID_MS, SettingsScreen } from "./SettingsScreen";

/** Mount Settings with a real store but mocked IPC. */
function mount() {
  return render(() => (
    <AppProvider>
      <SettingsScreen />
    </AppProvider>
  ));
}

const flush = (): Promise<void> => new Promise((r) => setTimeout(r, 0));

beforeEach(() => {
  outputDevices = [];
});

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

// v1.56.0: the mute's rows and the words around it. Mute is your voice
// everywhere Divora sends it; Monitor is whether your headphones get a copy
// (with no monitor device, Divora's whole output).
describe("SettingsScreen — the mute (v1.56.0)", () => {
  const flush = async (): Promise<void> => {
    for (let i = 0; i < 10; i++) await Promise.resolve();
    await new Promise((r) => setTimeout(r, 0));
  };
  const squash = (s: string | null | undefined): string => (s ?? "").replace(/\s+/g, " ").trim();
  const MUTE_DESC =
    "Mutes or unmutes your voice for the call, your headphones, WAV and screen recordings and Replay. Clips and Speak still play.";
  const MONITOR_DESC =
    "Turns hearing yourself on or off. With no separate monitor device, it turns Divora's whole output on or off, the call included.";

  beforeEach(() => {
    window.localStorage.clear();
    overlayReply = null;
    invokeCalls.length = 0;
  });

  it("the Mute your voice hotkey row follows Toggle monitor and names headphones and recordings", async () => {
    const view = mount();
    await flush();
    const desc = view.getByText(MUTE_DESC);
    expect(desc.textContent).toContain("headphones");
    expect(desc.textContent).toContain("recordings");
    expect(desc.previousElementSibling?.textContent).toBe("Mute your voice");
    // Right after Toggle monitor, before the recorders.
    const rows = desc.closest("section")!.textContent ?? "";
    const at = rows.indexOf(MUTE_DESC);
    expect(rows.indexOf(MONITOR_DESC)).toBeGreaterThan(-1);
    expect(rows.indexOf(MONITOR_DESC)).toBeLessThan(at);
    expect(rows.indexOf("Record screen")).toBeGreaterThan(at);
  });

  it("the row is bound to the mute hotkey", async () => {
    window.localStorage.setItem("divora.hotkeyBindings", JSON.stringify({ mute: "F20" }));
    const view = mount();
    await flush();
    fireEvent.click(view.getByRole("button", { name: "Clear Mute your voice hotkey" }));
    await flush();
    expect(invokeCalls.filter((c) => c.cmd === "unregister_global_shortcut").map((c) => c.args)).toEqual([
      { id: "mute" },
    ]);
  });

  it("the Monitor row says what Monitor does, and never 'mute'", async () => {
    const view = mount();
    await flush();
    const desc = view.getByText(MONITOR_DESC);
    expect(desc.textContent).toContain("the call included");
    expect(desc.textContent).not.toMatch(/mute/i);
    expect(view.queryByText(/headphone sidetone/i)).toBeNull();
    expect(squash(document.body.textContent)).not.toContain("mutes only this device");
  });

  it("the monitor-device note says what the Monitor toggle does with the device picked", async () => {
    const SEPARATE =
      "The Monitor toggle below turns your voice on or off on this device only, never on the main send. Clips and Speak here follow the Monitor checkbox on Speak.";
    const NONE =
      "With no separate monitor device, the Monitor toggle below turns Divora's whole output on or off, the call included.";
    outputDevices = [
      { name: "CABLE Input", isDefault: true, defaultSampleRate: 48000, channels: 2 },
      { name: "Headphones", isDefault: false, defaultSampleRate: 48000, channels: 2 },
    ];
    const view = mount();
    await flush();
    const note = view.getByText(/Pick a second device/);
    const field = note.parentElement!;
    const pickMonitor = async (name: RegExp) => {
      fireEvent.click(field.querySelector("button.select")!);
      fireEvent.click(within(field).getByRole("option", { name }));
      await flush();
    };

    // None (the default): the toggle gates the main output, the call included.
    expect(squash(note.textContent)).toContain(NONE);
    expect(squash(note.textContent)).not.toContain(SEPARATE);

    // A second device: the toggle gates only your voice on it.
    await pickMonitor(/Headphones/);
    expect(squash(note.textContent)).toContain(SEPARATE);
    expect(squash(note.textContent)).not.toContain(NONE);

    // The main output picked again as the monitor: the engine skips it, so
    // there is no separate monitor device.
    await pickMonitor(/CABLE Input/);
    expect(squash(note.textContent)).toContain(NONE);

    await pickMonitor(/None/);
    expect(squash(note.textContent)).toContain(NONE);
  });

  it("MIDI: a Mute your voice mapping, and the intro says how a pad and a CC act", async () => {
    window.localStorage.setItem(
      "divora.midiMappings",
      JSON.stringify([{ id: "m1", action: "preset", trigger: { kind: "cc", data1: 7, channel: 0 } }]),
    );
    const view = mount();
    await flush();
    const section = view.getByText("Control surfaces").closest("section")!;
    expect(squash(section.textContent)).toContain(
      "Mute your voice toggles with each press of a pad that sends a note (its mapping shows Note), unless the pad is set to latch: then every second press just ends the note and does nothing. On a knob, fader or CC button it mutes when the control goes past halfway and unmutes when it comes back a little below halfway (59 or less); a control that stays on one side never changes it, except that its first move after launch (or after you pick the device or change its mapping) mutes if it is past halfway.",
    );
    // The sentence against the rule it describes (data/mute.ts). Its first
    // move mutes past halfway and does nothing below it...
    expect(muteCcStep(undefined, 100).act).toBe(true);
    expect(muteCcStep(undefined, 10).act).toBeNull();
    // ...then staying on a side never acts, and crossing does.
    expect(muteCcStep(true, 100).act).toBeNull();
    expect(muteCcStep(false, 10).act).toBeNull();
    expect(muteCcStep(false, 100).act).toBe(true);
    expect(muteCcStep(true, 10).act).toBe(false);
    // "A little below halfway (59 or less)": 60..63 is below halfway and
    // still keeps it muted.
    expect(muteCcStep(true, 63).act).toBeNull();
    expect(muteCcStep(true, 60).act).toBeNull();
    expect(muteCcStep(true, 59).act).toBe(false);

    // Pick it from the action list: the mapping is saved as kind "mute".
    fireEvent.click(view.getByText("Summon preset"));
    fireEvent.click(view.getByRole("option", { name: /Mute your voice/ }));
    await flush();
    const saved = JSON.parse(window.localStorage.getItem("divora.midiMappings") ?? "[]") as Array<{
      action: string;
    }>;
    expect(saved.map((m) => m.action)).toEqual(["mute"]);
    const shown = Array.from(section.querySelectorAll(".select-main")).map((e) => e.textContent);
    expect(shown).toContain("Mute your voice");
  });

  it("the glyph option reads Mute / unmute", async () => {
    window.localStorage.setItem("divora.glyphBindings", JSON.stringify({ triangle: { kind: "mute" } }));
    const view = mount();
    await flush();
    const section = view.getByText("Glyph casting").closest("section")!;
    const shown = Array.from(section.querySelectorAll(".select-main")).map((e) => e.textContent);
    expect(shown).toContain("Mute / unmute");
  });

  it("the Stream Deck card names Mute your voice", async () => {
    const view = mount();
    await flush();
    const card = view.getByText("Using a Stream Deck").closest(".card");
    expect(card).not.toBeNull();
    const text = squash(card!.textContent);
    // v1.59.0: the sentence goes on to a timer's keys (pinned in the
    // Hourglass block below).
    expect(text).toContain(
      "the same key you bound under Settings → Hotkeys (for example Mute your voice, panic or a recorder) or to a per-tile soundboard hotkey",
    );
    expect(text).not.toMatch(/push-to-modulate/i);
  });
});

describe("SettingsScreen — push-to-modulate (v1.58.0)", () => {
  const flush = async (): Promise<void> => {
    for (let i = 0; i < 10; i++) await Promise.resolve();
    await new Promise((r) => setTimeout(r, 0));
  };
  const squash = (s: string | null | undefined): string => (s ?? "").replace(/\s+/g, " ").trim();
  const PTM_DESC =
    "Hold to switch your effects on or off, as the Mixer's Push to modulate card is set. Switches nothing while that card is off, but other apps still don't get the key.";
  const PTM_MIDI =
    "Hold push-to-modulate holds while a pad's note is down, or while a knob, fader or CC button is past halfway (it lets go at 59 or less); with two pads on it, it holds until both are up. It does nothing while the Mixer's Push to modulate card is off. Turning Push to modulate on or off on the Mixer, or picking its hold, lets go: a pad then holds again on its next press, and a control left past halfway once it comes back down and up again.";

  beforeEach(() => {
    window.localStorage.clear();
    overlayReply = null;
    invokeCalls.length = 0;
  });

  it("the Push-to-modulate hotkey row says it switches your effects as the Mixer card is set", async () => {
    const view = mount();
    await flush();
    const desc = view.getByText(PTM_DESC);
    expect(desc.previousElementSibling?.textContent).toBe("Push-to-modulate");
    // The old words promised a switch that didn't happen; the copy rules.
    const text = desc.textContent ?? "";
    expect(text).not.toMatch(/chain|clean|bypass|instant|at once/i);
    expect(view.queryByText(/Hold to apply the chain/)).toBeNull();
    // Still the first row.
    const rows = desc.closest("section")!.textContent ?? "";
    expect(rows.indexOf(PTM_DESC)).toBeLessThan(rows.indexOf("Panic — stop all clips"));
  });

  it("MIDI: the intro says how a Hold push-to-modulate pad and CC hold", async () => {
    const view = mount();
    await flush();
    const section = view.getByText("Control surfaces").closest("section")!;
    const intro = squash(section.querySelector("p")?.textContent);
    expect(intro).toContain(PTM_MIDI);
    // After the mute's sentences, not in the middle of them.
    expect(intro.indexOf(PTM_MIDI)).toBeGreaterThan(intro.indexOf("mutes if it is past halfway."));
    // The sentence against the rule the store holds a CC by (data/mute.ts,
    // as the mute's): past halfway holds, 60..63 keeps, 59 or less lets go.
    // Its exception, a control left past halfway across the card's set
    // holding only after it crosses again, is the store's own behaviour,
    // pinned in app.test.tsx ("after a mode change, a CC resting up doesn't
    // hold until it goes down and back up").
    expect(muteCcStep(undefined, 64).act).toBe(true);
    expect(muteCcStep(false, 64).act).toBe(true);
    expect(muteCcStep(true, 63).act).toBeNull();
    expect(muteCcStep(true, 60).act).toBeNull();
    expect(muteCcStep(true, 59).act).toBe(false);
    expect(muteCcStep(false, 63).act).toBeNull();
    // Nothing in the section promises a switch with no delay.
    expect(squash(section.textContent)).not.toMatch(/instant|at once/i);
  });

  it("MIDI: the action is still called Hold push-to-modulate and saves as kind ptm", async () => {
    window.localStorage.setItem(
      "divora.midiMappings",
      JSON.stringify([{ id: "m1", action: "preset", trigger: { kind: "note", data1: 36, channel: 0 } }]),
    );
    const view = mount();
    await flush();
    const section = view.getByText("Control surfaces").closest("section")!;
    fireEvent.click(view.getByText("Summon preset"));
    fireEvent.click(view.getByRole("option", { name: /Hold push-to-modulate/ }));
    await flush();
    const saved = JSON.parse(window.localStorage.getItem("divora.midiMappings") ?? "[]") as Array<{
      action: string;
    }>;
    expect(saved.map((m) => m.action)).toEqual(["ptm"]);
    const shown = Array.from(section.querySelectorAll(".select-main")).map((e) => e.textContent);
    expect(shown).toContain("Hold push-to-modulate");
  });
});

// v1.59.0: Settings → Hourglass. The store is real (track C's bodies); the
// backend's snapshots come through the mocked event bus, and its answers
// are null unless a test says, as the e2e mock's are. The section never
// decides a timer's state: it shows the newest snapshot and sends what a
// control says.
describe("SettingsScreen — Hourglass (v1.59.0)", () => {
  const flush = async (): Promise<void> => {
    for (let i = 0; i < 10; i++) await Promise.resolve();
    await new Promise((r) => setTimeout(r, 0));
  };
  const squash = (s: string | null | undefined): string => (s ?? "").replace(/\s+/g, " ").trim();
  const KEY = "divora.timers";
  const ENDS = 1_793_217_900_000;

  /** A stored countdown "BRB" of 5:00 unless `more` says otherwise. */
  const entry = (id: string, more: Record<string, unknown> = {}) => ({
    id,
    name: "BRB",
    kind: "countdown",
    durationMs: 300_000,
    ...more,
  });
  const store = (...timers: unknown[]) =>
    window.localStorage.setItem(KEY, JSON.stringify({ version: 1, timers }));
  const saved = (): Array<Record<string, unknown>> =>
    (JSON.parse(window.localStorage.getItem(KEY) ?? '{"timers":[]}') as { timers: Array<Record<string, unknown>> })
      .timers;
  /** A timer as a snapshot has it: an idle 5:00 countdown "BRB" unless
   *  `more` says otherwise. */
  const state = (id: string, more: Record<string, unknown> = {}) => ({
    id,
    name: "BRB",
    kind: "countdown",
    phase: "idle",
    ms: 300_000,
    totalMs: 300_000,
    text: "05:00",
    anchorMs: null,
    fires: 0,
    ...more,
  });
  /** The backend's `timer-state`, heard by the store's listener. */
  const emit = async (seq: number, ...timers: unknown[]) => {
    for (const h of eventHandlers.get("timer-state") ?? []) h({ payload: { seq, timers } });
    await flush();
  };
  const sent = (cmd: string) => invokeCalls.filter((c) => c.cmd === cmd).map((c) => c.args);
  const timerCmds = () =>
    invokeCalls
      .map((c) => c.cmd)
      .filter((c) => c === "sync_timers" || c === "timer_status" || c.endsWith("_timer") || c.includes("global_shortcut"));
  const card = (id: string): HTMLElement => {
    const el = document.querySelector<HTMLElement>(`[data-timer-card="${id}"]`);
    if (!el) throw new Error(`no card for ${id}`);
    return el;
  };
  const section = (view: ReturnType<typeof mount>): HTMLElement =>
    view.getByText("Hourglass", { selector: ".eyebrow" }).closest("section")!;
  const announcer = (view: ReturnType<typeof mount>) => view.getByTestId("hourglass-announcer");
  /** Every button, switch and text field in a card, by its name. */
  const controls = (el: HTMLElement): string[] =>
    Array.from(el.querySelectorAll("button, input")).map(
      (c) => c.getAttribute("aria-label") ?? squash(c.textContent),
    );

  /** Settings with the store's toast shown beside it. */
  function ToastProbe() {
    const app = useApp();
    return (
      <div data-testid="toast-probe" data-tone={app.toast()?.tone}>
        {app.toast()?.text}
      </div>
    );
  }
  const mountWithToast = () =>
    render(() => (
      <AppProvider>
        <SettingsScreen />
        <ToastProbe />
      </AppProvider>
    ));

  beforeEach(() => {
    window.localStorage.clear();
    overlayReply = null;
    timerReply = () => null;
    invokeCalls.length = 0;
    eventHandlers.delete("timer-state");
  });

  it("follows Stream overlay, says what it is, and starts empty without asking the backend", async () => {
    const view = mount();
    await flush();
    const eyebrows = Array.from(document.querySelectorAll("section > .eyebrow")).map((e) => squash(e.textContent));
    expect(eyebrows.indexOf("Hourglass")).toBe(eyebrows.indexOf("Stream overlay") + 1);
    expect(eyebrows.indexOf("About")).toBe(eyebrows.indexOf("Hourglass") + 1);
    const hourglass = section(view);
    expect(squash(hourglass.querySelector("p")?.textContent)).toBe(
      "Countdowns and count-ups that keep time while Divora is in the tray. Start, pause and reset them here or with their own hotkeys, and show them on your overlay or in OBS. If your PC sleeps, a count-up pauses, and a countdown that runs out meanwhile says so instead of finishing late.",
    );
    expect(view.getByText("No timers yet — add one below.")).toBeInTheDocument();
    expect(view.getByRole("button", { name: "Add countdown" })).not.toBeDisabled();
    expect(view.getByRole("button", { name: "Add count-up" })).not.toBeDisabled();
    expect(squash(view.getByTestId("hourglass-hotkey-note").textContent)).toBe(
      "Keys you set here work in every app, and the app you're in doesn't get them while Divora runs: set to Space, you can't type a space anywhere. Pick keys you don't otherwise use, such as F13–F24 from a Stream Deck, or Ctrl+Alt with a letter on a US English keyboard. A key works only with exactly its modifiers: set to F9, pressing it while you hold Shift or Ctrl does nothing. A key that's already set in Divora runs what it's set for and can't be picked here; clear it there first. Some keys can't be set, and Divora doesn't say so: the row still shows the key. A key Windows or another app already holds is one; Shift with a number or symbol key is another (Shift+1 shows as !), and so, on some keyboards other than US English, is Ctrl+Alt with some letters. To check one, start its timer, press the key with Divora in the tray, then look at the timer here.",
    );
    expect(view.getByText("Timer files for OBS")).toBeInTheDocument();
    expect(squash(view.getByText(/pick a timer's file from this folder/).textContent)).toBe(
      "In OBS, add a Text (GDI+) source, tick Read from file and pick a timer's file from this folder. OBS checks the file about once a second, so its text can run up to about 2 seconds behind the overlay, and now and then it may show nothing for a second or two. While Divora is closed, a file keeps the last time it showed.",
    );
    // A fresh profile asks the backend nothing about timers, Settings open
    // or not.
    expect(timerCmds()).toEqual([]);
    expect(eventHandlers.get("timer-state") ?? []).toHaveLength(0);
    expect(hourglass).not.toHaveAttribute("data-timer-seq");
  });

  it("Add countdown syncs and shows a card that says Starting… until the backend answers", async () => {
    const view = mount();
    await flush();
    fireEvent.click(view.getByRole("button", { name: "Add countdown" }));
    await flush();
    expect(sent("sync_timers")).toEqual([
      { timers: [{ id: "timer-1", name: "Countdown", kind: "countdown", durationMs: 300_000 }] },
    ]);
    const c = card("timer-1");
    expect(squash(c.textContent)).toContain("Countdown · 5:00");
    expect(within(c).getByText("Starting…")).toBeInTheDocument();
    // No state yet: its name, its length, Rename and Delete only.
    expect(controls(c)).toEqual(["Rename Countdown", "Delete Countdown"]);
    expect(view.queryByText("No timers yet — add one below.")).toBeNull();

    fireEvent.click(view.getByRole("button", { name: "Add count-up" }));
    await flush();
    expect(squash(card("timer-2").textContent)).toContain("Count-up");
    expect(within(card("timer-2")).getByText("Starting…")).toBeInTheDocument();

    // The backend's first snapshot: the cards come alive.
    await emit(1, state("timer-1", { name: "Countdown" }), state("timer-2", { name: "Count-up", kind: "countup", ms: 0, totalMs: 0, text: "00:00" }));
    expect(within(card("timer-1")).getByText("Ready")).toBeInTheDocument();
    expect(within(card("timer-1")).getByText("05:00")).toBeInTheDocument();
    expect(within(card("timer-1")).getByRole("button", { name: "Start Countdown" })).toBeInTheDocument();
    expect(within(card("timer-2")).getByRole("button", { name: "Start Count-up" })).toBeInTheDocument();
    // A count-up has no Length.
    expect(within(card("timer-2")).queryByRole("textbox")).toBeNull();
    expect(within(card("timer-1")).getByRole("textbox", { name: "Length of Countdown" })).toBeInTheDocument();
  });

  it("at 16 stored timers, a newer kind counted, Add is off with the reason", async () => {
    store(
      entry("timer-1", { kind: "interval", name: "Laps" }),
      ...Array.from({ length: 15 }, (_, i) => entry(`timer-${i + 2}`, { name: `T${i + 2}` })),
    );
    const view = mount();
    await flush();
    expect(view.getByRole("button", { name: "Add countdown" })).toBeDisabled();
    expect(view.getByRole("button", { name: "Add count-up" })).toBeDisabled();
    expect(view.getByText("Hourglass holds up to 16 timers. Delete one to add another.")).toBeInTheDocument();
  });

  it("a newer kind and a timer over the limit are read-only rows with Delete only", async () => {
    store(
      entry("timer-1", { kind: "interval", name: "Laps", hotkeys: { toggle: "F7" } }),
      ...Array.from({ length: 16 }, (_, i) => entry(`timer-${i + 2}`, { name: `T${i + 2}` })),
      entry("timer-18", { name: "Extra", hotkeys: { toggle: "F8" } }),
    );
    const view = mount();
    await flush();
    await emit(1, ...Array.from({ length: 16 }, (_, i) => state(`timer-${i + 2}`, { name: `T${i + 2}` })));
    expect(squash(within(card("timer-1")).getByText(/Made by a newer Divora/).textContent)).toBe(
      "Laps · Made by a newer Divora. Delete it here, or use it in that version.",
    );
    expect(controls(card("timer-1"))).toEqual(["Delete Laps"]);
    expect(squash(within(card("timer-18")).getByText(/Over the 16-timer limit/).textContent)).toBe(
      "Extra · Over the 16-timer limit: Divora runs the first 16.",
    );
    expect(controls(card("timer-18"))).toEqual(["Delete Extra"]);
    // No hotkey rows for them, and their keys never show.
    expect(view.queryByRole("button", { name: /Laps .*hotkey/ })).toBeNull();
    expect(view.queryByRole("button", { name: /Extra .*hotkey/ })).toBeNull();
    expect(within(card("timer-2")).getByRole("button", { name: "Start T2" })).toBeInTheDocument();
    // Delete still asks twice there, and goes.
    fireEvent.click(view.getByRole("button", { name: "Delete Laps" }));
    fireEvent.click(view.getByRole("button", { name: "Click again to delete Laps" }));
    await flush();
    expect(saved().map((t) => t.id)).not.toContain("timer-1");
  });

  it("the main button follows the phase and sends what it says; ±30 s only while running or paused", async () => {
    store(entry("timer-1"));
    const view = mount();
    await flush();
    const button = (name: RegExp) => within(card("timer-1")).getByRole("button", { name });
    const phases: Array<[Record<string, unknown>, string, string, string]> = [
      [{ phase: "idle" }, "Start", "Ready", "arm_timer"],
      [{ phase: "running", ms: 298_400, text: "04:59", anchorMs: ENDS }, "Pause", "Running", "pause_timer"],
      [{ phase: "paused", ms: 200_000, text: "03:20" }, "Resume", "Paused", "resume_timer"],
      [{ phase: "done", ms: 0, text: "00:00", fires: 1 }, "Start again", "Done", "arm_timer"],
      [{ phase: "expired", ms: 0, text: "00:00", fires: 1 }, "Start again", "Ran out while away", "arm_timer"],
    ];
    let seq = 0;
    for (const [more, label, word, cmd] of phases) {
      await emit(++seq, state("timer-1", more));
      const main = within(card("timer-1")).getByRole("button", { name: `${label} BRB` });
      expect(main).toHaveTextContent(label);
      expect(within(card("timer-1")).getByText(word)).toBeInTheDocument();
      invokeCalls.length = 0;
      fireEvent.click(main);
      await flush();
      expect(timerCmds()).toEqual([cmd]);
      expect(sent(cmd)).toEqual([{ id: "timer-1" }]);

      const adjustable = more.phase === "running" || more.phase === "paused";
      for (const [name, delta] of [
        [/^Take 30 seconds off BRB$/, -30_000],
        [/^Add 30 seconds to BRB$/, 30_000],
      ] as const) {
        const b = button(name);
        if (!adjustable) {
          expect(b).toBeDisabled();
          continue;
        }
        expect(b).not.toBeDisabled();
        invokeCalls.length = 0;
        fireEvent.click(b);
        await flush();
        expect(sent("adjust_timer")).toEqual([{ id: "timer-1", deltaMs: delta }]);
      }
      invokeCalls.length = 0;
      fireEvent.click(button(/^Reset BRB$/));
      await flush();
      expect(sent("reset_timer")).toEqual([{ id: "timer-1" }]);
    }
    expect(button(/^Take 30 seconds off BRB$/)).toHaveTextContent("−30 s");
    expect(button(/^Add 30 seconds to BRB$/)).toHaveTextContent("+30 s");
    // Ran out while away says why it didn't finish.
    expect(
      within(card("timer-1")).getByText(
        "It reached 00:00 while your PC was asleep or Divora was closed, so it didn't finish.",
      ),
    ).toBeInTheDocument();
  });

  it("Length: typing sends nothing; Enter or leaving applies it and shows it in full; Esc puts it back; one it can't read stays marked and sends nothing", async () => {
    store(entry("timer-1"));
    const view = mount();
    await flush();
    await emit(1, state("timer-1"));
    const field = view.getByRole("textbox", { name: "Length of BRB" }) as HTMLInputElement;
    expect(field.value).toBe("5:00");
    expect(field).toHaveAttribute("placeholder", "5:00");
    invokeCalls.length = 0;

    for (const typed of ["1", "10"]) fireEvent.input(field, { target: { value: typed } });
    await flush();
    expect(sent("sync_timers")).toEqual([]);
    expect(saved()[0]!.durationMs).toBe(300_000);

    fireEvent.keyDown(field, { key: "Enter" });
    await flush();
    expect(sent("sync_timers")).toEqual([
      { timers: [{ id: "timer-1", name: "BRB", kind: "countdown", durationMs: 600_000 }] },
    ]);
    expect(field.value).toBe("10:00");
    expect(squash(card("timer-1").textContent)).toContain("Countdown · 10:00");
    expect(saved()[0]!.durationMs).toBe(600_000);

    // One it can't read: the draft stays, marked, with its reason; nothing goes.
    invokeCalls.length = 0;
    fireEvent.input(field, { target: { value: "1:2" } });
    fireEvent.keyDown(field, { key: "Enter" });
    await flush();
    expect(field.value).toBe("1:2");
    expect(field).toHaveAttribute("aria-invalid", "true");
    const reason = view.getByText("Enter whole minutes, m:ss or h:mm:ss, from 1 second to 24 hours.");
    expect(field.getAttribute("aria-describedby")?.split(" ")).toContain(reason.id);
    fireEvent.blur(field);
    await flush();
    expect(field.value).toBe("1:2");
    expect(sent("sync_timers")).toEqual([]);

    // Esc puts the stored one back.
    fireEvent.keyDown(field, { key: "Escape" });
    await flush();
    expect(field.value).toBe("10:00");
    expect(field).not.toHaveAttribute("aria-invalid");
    expect(view.queryByText("Enter whole minutes, m:ss or h:mm:ss, from 1 second to 24 hours.")).toBeNull();
    expect(sent("sync_timers")).toEqual([]);

    // Leaving the field applies it too.
    fireEvent.input(field, { target: { value: " 90 " } });
    fireEvent.blur(field);
    await flush();
    expect(field.value).toBe("1:30:00");
    expect(sent("sync_timers").map((a) => (a as { timers: Array<{ durationMs: number }> }).timers[0]!.durationMs)).toEqual([
      5_400_000,
    ]);

    // The same length again sends nothing, and still reads in full.
    invokeCalls.length = 0;
    fireEvent.input(field, { target: { value: "90:00" } });
    fireEvent.keyDown(field, { key: "Enter" });
    await flush();
    expect(field.value).toBe("1:30:00");
    expect(sent("sync_timers")).toEqual([]);

    // While it runs, a new length waits for the next start, and says so.
    await emit(2, state("timer-1", { phase: "running", ms: 5_000_000, totalMs: 5_400_000, text: "1:23:20", anchorMs: ENDS }));
    const next = view.getByText("A new length counts from the next start.");
    expect(field.getAttribute("aria-describedby")?.split(" ")).toContain(next.id);
  });

  it("Rename: Enter keeps it, Esc cancels, leaving keeps it, and an emptied name puts the old one back", async () => {
    store(entry("timer-1"));
    const view = mount();
    await flush();
    await emit(1, state("timer-1"));
    invokeCalls.length = 0;
    const rename = async (typed: string, end: "Enter" | "Escape" | "blur") => {
      fireEvent.click(within(card("timer-1")).getByRole("button", { name: /^Rename / }));
      await flush();
      const field = within(card("timer-1")).getByRole("textbox", { name: /^Name of / }) as HTMLInputElement;
      expect(document.activeElement).toBe(field);
      fireEvent.input(field, { target: { value: typed } });
      if (end === "blur") fireEvent.blur(field);
      else fireEvent.keyDown(field, { key: end });
      await flush();
    };

    await rename("   ", "Enter");
    expect(within(card("timer-1")).getByText("BRB")).toBeInTheDocument();
    await rename("Intro", "Escape");
    expect(within(card("timer-1")).getByText("BRB")).toBeInTheDocument();
    expect(sent("sync_timers")).toEqual([]);
    expect(saved()[0]!.name).toBe("BRB");

    await rename("  Intro ", "Enter");
    expect(sent("sync_timers")).toEqual([
      { timers: [{ id: "timer-1", name: "Intro", kind: "countdown", durationMs: 300_000 }] },
    ]);
    expect(saved()[0]!.name).toBe("Intro");
    expect(within(card("timer-1")).getByRole("button", { name: "Start Intro" })).toBeInTheDocument();

    await rename("Break", "blur");
    expect(saved()[0]!.name).toBe("Break");
    expect(within(card("timer-1")).getByRole("button", { name: "Rename Break" })).toBeInTheDocument();
  });

  it("the hotkey rows: named for the timer, Clear named, binding and clearing through the store", async () => {
    store(entry("timer-1", { hotkeys: { toggle: "Ctrl+Alt+F13", finish: "F14" } }));
    const view = mount();
    await flush();
    await emit(1, state("timer-1"));
    const c = card("timer-1");
    expect(within(c).getByText("Start or pause")).toBeInTheDocument();
    expect(within(c).getByText("Starts it, pauses it, resumes it, or starts it again once it's done or has run out.")).toBeInTheDocument();
    expect(within(c).getByText("Back to the start, stopped.")).toBeInTheDocument();
    expect(view.getByRole("button", { name: "BRB start or pause hotkey: Ctrl Alt F13" })).toBeInTheDocument();
    const reset = view.getByRole("button", { name: "BRB reset hotkey: not set" });
    expect(view.queryByRole("button", { name: "Clear BRB reset hotkey" })).toBeNull();
    // e2e still finds Settings → Hotkeys' captures by "Click to set"; these
    // are named for their timer instead.
    expect(within(c).queryByRole("button", { name: "Click to set" })).toBeNull();

    invokeCalls.length = 0;
    fireEvent.click(reset, { detail: 1 });
    expect(view.getByRole("button", { name: "BRB reset hotkey: press a key, Esc to cancel" })).toBe(reset);
    window.dispatchEvent(new KeyboardEvent("keydown", { key: "F9", cancelable: true }));
    await flush();
    expect(sent("register_global_shortcut")).toEqual([{ id: "tm:timer-1:reset", accelerator: "F9" }]);
    expect(view.getByRole("button", { name: "BRB reset hotkey: F9" })).toBeInTheDocument();

    invokeCalls.length = 0;
    fireEvent.click(view.getByRole("button", { name: "Clear BRB start or pause hotkey" }));
    await flush();
    expect(sent("unregister_global_shortcut")).toEqual([{ id: "tm:timer-1:toggle" }]);
    expect(view.getByRole("button", { name: "BRB start or pause hotkey: not set" })).toBeInTheDocument();
    // A later build's verb is kept.
    expect(saved()[0]!.hotkeys).toEqual({ reset: "F9", finish: "F14" });
  });

  it("a key set twice says where else it's set: an action, a tile not in the scan, another timer row", async () => {
    window.localStorage.setItem("divora.hotkeyBindings", JSON.stringify({ mute: "F9" }));
    window.localStorage.setItem("divora.tileHotkeys", JSON.stringify({ "clip-gone": ["Ctrl", "Alt", "F13"] }));
    store(
      entry("timer-1", { hotkeys: { toggle: "Alt+Ctrl+F13", reset: "F9" } }),
      entry("timer-2", { name: "Intro", hotkeys: { toggle: "f9", reset: "F10" } }),
    );
    mount();
    await flush();
    await emit(1, state("timer-1"), state("timer-2", { name: "Intro" }));
    const lines = (id: string) =>
      Array.from(card(id).querySelectorAll("div"))
        .map((d) => squash(d.textContent))
        .filter((t) => t.startsWith("Also set for"));
    expect(lines("timer-1")).toEqual([
      "Also set for a soundboard tile.",
      "Also set for Mute your voice and Intro start or pause.",
    ]);
    expect(lines("timer-2")).toEqual(["Also set for Mute your voice and BRB reset."]);
  });

  it("Show on overlay adds the timer's element; its help, and the Hide-from-recordings line only with both flags on", async () => {
    store(entry("timer-1"));
    const first = mount();
    await flush();
    await emit(1, state("timer-1"));
    const toggle = first.getByRole("switch", { name: "Show BRB on the overlay" });
    expect(toggle).toHaveAttribute("aria-checked", "false");
    const help = "Move and resize it in Arrange, like a picture. It shows while the overlay is open.";
    expect(first.queryByText(help)).toBeNull();
    fireEvent.click(toggle);
    await flush();
    expect(toggle).toHaveAttribute("aria-checked", "true");
    expect(first.getByText(help)).toBeInTheDocument();
    expect(first.queryByText(/Hide from recordings is on/)).toBeNull();
    const scene = JSON.parse(window.localStorage.getItem("divora.overlayScene") ?? "{}") as {
      elements: Array<{ type: string; timer?: string }>;
    };
    expect(scene.elements.filter((e) => e.type === "timer").map((e) => e.timer)).toEqual(["timer-1"]);
    // The Charms list has it, named for the timer.
    expect(within(first.getByTestId("overlay-charms")).getByText("Timer · BRB")).toBeInTheDocument();
    // Still the one "Stream overlay" e2e finds the section by.
    expect(first.getAllByText(/Stream overlay/i)).toHaveLength(1);
    first.unmount();

    const LINE =
      "Hide from recordings is on: while Divora records any screen or Replay is on, OBS, Discord and recordings may not show the overlay, this timer included. Its text file shows it in OBS either way.";
    for (const [available, on, shown] of [
      [true, true, true],
      [true, false, false],
      [false, true, false],
    ] as const) {
      overlayReply = { open: false, phase: "closed", instance: 0, mode: "locked", arrangeGen: 0, canArrange: false, fit: false, hideWanted: on, hideAvailable: available, capturing: false, error: null };
      window.localStorage.setItem("divora.overlay", JSON.stringify({ bg: "transparent", hideFromRecordings: on }));
      eventHandlers.delete("timer-state");
      const view = mount();
      await flush();
      await emit(1, state("timer-1"));
      expect(view.getByRole("switch", { name: "Show BRB on the overlay" })).toHaveAttribute("aria-checked", "true");
      expect(view.queryAllByText(LINE)).toHaveLength(shown ? 1 : 0);
      view.unmount();
    }
  });

  it("the Charms list names a timer's row Timer · {name}, with the hourglass", async () => {
    store(entry("timer-1"));
    window.localStorage.setItem(
      "divora.overlayScene",
      JSON.stringify({
        version: 1,
        designSize: { w: 480, h: 480 },
        frame: "window",
        elements: [{ id: "c-9", type: "timer", x: 0.1, y: 0.1, w: 0.3, h: 0.1, z: 1, timer: "timer-1" }],
      }),
    );
    const view = mount();
    await flush();
    const row = view.getByTestId("overlay-charms").querySelector<HTMLElement>('[data-charm-row="c-9"]')!;
    expect(within(row).getByText("Timer · BRB")).toBeInTheDocument();
    expect(within(row).getByRole("button", { name: "Remove BRB" })).toBeInTheDocument();
    const glass = render(() => <Sigil name="hourglass" size={16} />).container.querySelector("svg")!;
    expect(row.querySelector("svg")!.innerHTML).toBe(glass.innerHTML);
  });

  // v1.60.0: three keys, End now's included.
  it("Delete asks twice: one click sends nothing and reads Confirm?; the second unregisters its three keys, then syncs without it", async () => {
    store(entry("timer-1", { hotkeys: { toggle: "F9" } }), entry("timer-2", { name: "Intro" }));
    const view = mount();
    await flush();
    await emit(1, state("timer-1"), state("timer-2", { name: "Intro" }));
    invokeCalls.length = 0;
    fireEvent.click(view.getByRole("button", { name: "Delete BRB" }));
    await flush();
    expect(timerCmds()).toEqual([]);
    const confirm = view.getByRole("button", { name: "Click again to delete BRB" });
    expect(confirm).toHaveTextContent("Confirm?");
    expect(card("timer-1")).toBeInTheDocument();
    fireEvent.click(confirm);
    await flush();
    expect(timerCmds()).toEqual([
      "unregister_global_shortcut",
      "unregister_global_shortcut",
      "unregister_global_shortcut",
      "sync_timers",
    ]);
    expect(sent("unregister_global_shortcut")).toEqual([
      { id: "tm:timer-1:toggle" },
      { id: "tm:timer-1:reset" },
      { id: "tm:timer-1:finish" },
    ]);
    expect(sent("sync_timers")).toEqual([
      { timers: [{ id: "timer-2", name: "Intro", kind: "countdown", durationMs: 300_000 }] },
    ]);
    expect(document.querySelector('[data-timer-card="timer-1"]')).toBeNull();
    expect(card("timer-2")).toBeInTheDocument();
  });

  it("Delete's Confirm? goes back after 3 s", async () => {
    store(entry("timer-1"));
    const view = mount();
    await flush();
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
    try {
      fireEvent.click(view.getByRole("button", { name: "Delete BRB" }));
      expect(view.getByRole("button", { name: "Click again to delete BRB" })).toHaveTextContent("Confirm?");
      vi.advanceTimersByTime(2_999);
      expect(view.getByRole("button", { name: "Click again to delete BRB" })).toBeInTheDocument();
      vi.advanceTimersByTime(1);
      expect(view.getByRole("button", { name: "Delete BRB" })).toHaveTextContent("Delete");
    } finally {
      vi.useRealTimers();
    }
    invokeCalls.length = 0;
    fireEvent.click(view.getByRole("button", { name: "Delete BRB" }));
    await flush();
    expect(timerCmds()).toEqual([]);
    expect(card("timer-1")).toBeInTheDocument();
  });

  it("a pointer click leaves the focus on the page, so Space reaches push-to-modulate; a keyboard click keeps it", async () => {
    store(entry("timer-1"));
    const view = mount();
    await flush();
    await emit(1, state("timer-1", { phase: "running", ms: 298_400, text: "04:59", anchorMs: ENDS }));
    const names = [
      "Pause BRB",
      "Reset BRB",
      "Take 30 seconds off BRB",
      "Add 30 seconds to BRB",
      "Copy the path of BRB's file",
      "Open the timer files folder",
      "Add countdown",
      "Add count-up",
      "Delete BRB",
    ];
    for (const name of names) {
      const b = view.getByRole("button", { name });
      b.focus();
      expect(document.activeElement).toBe(b);
      fireEvent.click(b, { detail: 1 });
      expect(document.activeElement, name).toBe(document.body);
      await flush();
    }
    const toggle = view.getByRole("switch", { name: "Show BRB on the overlay" });
    toggle.focus();
    fireEvent.click(toggle, { detail: 1 });
    expect(document.activeElement).toBe(document.body);
    await flush();

    // From the keyboard (Tab, then Space or Enter), the focus stays.
    const reset = view.getByRole("button", { name: "Reset BRB" });
    reset.focus();
    fireEvent.click(reset, { detail: 0 });
    expect(document.activeElement).toBe(reset);
  });

  it("the live region: present from the first render, says done on a fires rise (twice for two), ran out on expired, nothing for the first snapshot, never a time", async () => {
    store(entry("timer-1"), entry("timer-2", { name: "Intro" }));
    const view = mount();
    await flush();
    const region = announcer(view);
    expect(region).toHaveAttribute("aria-live", "polite");
    expect(region).toHaveTextContent("");
    vi.useFakeTimers({
      toFake: ["setTimeout", "clearTimeout", "requestAnimationFrame", "cancelAnimationFrame"],
    });
    try {
      const heard: string[] = [];
      const now = (): string => region.textContent ?? "";
      /** A snapshot from the backend, adopted at once; no frame drawn yet. */
      const step = async (seq: number, ...timers: unknown[]) => {
        for (const h of eventHandlers.get("timer-state") ?? []) h({ payload: { seq, timers } });
        for (let i = 0; i < 10; i++) await Promise.resolve();
        heard.push(now());
        // The same node throughout: a region inserted with its text is missed.
        expect(announcer(view)).toBe(region);
      };
      /** The next frame is drawn: what it shows is what a screen reader gets. */
      const frame = (): string => {
        vi.advanceTimersToNextFrame();
        heard.push(now());
        return now();
      };
      const run = (id: string, more: Record<string, unknown> = {}) =>
        state(id, { phase: "running", ms: 1_000, text: "00:01", anchorMs: ENDS, ...more });
      const done = (id: string, fires: number, more: Record<string, unknown> = {}) =>
        state(id, { phase: "done", ms: 0, text: "00:00", fires, ...more });
      const intro = { name: "Intro" };

      // The first snapshot after mount says nothing, though BRB is done in it.
      await step(1, done("timer-1", 1), run("timer-2", intro));
      expect(frame()).toBe("");
      await step(2, run("timer-1", { fires: 1 }), run("timer-2", intro));
      expect(frame()).toBe("");
      await step(3, done("timer-1", 2), run("timer-2", intro));
      // Intro's next second lands a millisecond later, before the frame: the
      // words still reach it, and stay after it.
      vi.advanceTimersByTime(1);
      await step(4, done("timer-1", 2), run("timer-2", { ...intro, text: "00:00" }));
      expect(frame()).toBe("BRB is done.");
      expect(frame()).toBe("BRB is done.");
      // A press that fires and re-arms in one go, while the words are still
      // up: no snapshot shows done. Emptied, then the same words again on the
      // next frame, so they are spoken again.
      await step(5, run("timer-1", { fires: 3, text: "05:00" }), run("timer-2", intro));
      expect(now()).toBe("");
      expect(frame()).toBe("BRB is done.");
      // An older snapshot isn't adopted, and changes nothing.
      await step(4, run("timer-1", { fires: 2 }), run("timer-2", intro));
      expect(frame()).toBe("BRB is done.");
      // Emptied once HOURGLASS_SAID_MS has gone by (the next test times it).
      vi.advanceTimersByTime(HOURGLASS_SAID_MS);
      expect(now()).toBe("");
      await step(6, done("timer-1", 4), state("timer-2", { ...intro, phase: "expired", ms: 0, text: "00:00" }));
      expect(frame()).toBe("BRB is done. Intro ran out while away.");
      await step(7, done("timer-1", 4), state("timer-2", { ...intro, phase: "expired", ms: 0, text: "00:00" }));
      expect(frame()).toBe("BRB is done. Intro ran out while away.");
      vi.advanceTimersByTime(HOURGLASS_SAID_MS);
      expect(now()).toBe("");
      // Two edges in two snapshots before one frame: both said, once each.
      await step(8, done("timer-1", 5), state("timer-2", { ...intro, phase: "idle" }));
      await step(9, done("timer-1", 5), state("timer-2", { ...intro, phase: "done", ms: 0, text: "00:00", fires: 1 }));
      expect(frame()).toBe("BRB is done. Intro is done.");
      for (const text of heard) expect(text).not.toMatch(/\d+:\d\d/);
    } finally {
      vi.useRealTimers();
    }
    // The time itself is on the card, outside any live region.
    const time = within(card("timer-1")).getByText("00:00");
    expect(time.closest("[aria-live]")).toBeNull();
  });

  it("the live region holds its words for HOURGLASS_SAID_MS, then empties; a section gone meanwhile leaves nothing pending", async () => {
    store(entry("timer-1"));
    const view = mount();
    await flush();
    const region = announcer(view);
    vi.useFakeTimers({
      toFake: ["setTimeout", "clearTimeout", "requestAnimationFrame", "cancelAnimationFrame"],
    });
    try {
      const fire = async (seq: number, fires: number) => {
        for (const h of eventHandlers.get("timer-state") ?? []) {
          h({ payload: { seq, timers: [state("timer-1", { phase: "done", ms: 0, text: "00:00", fires })] } });
        }
        for (let i = 0; i < 10; i++) await Promise.resolve();
      };
      await fire(1, 0);
      await fire(2, 1);
      vi.advanceTimersToNextFrame();
      expect(region).toHaveTextContent("BRB is done.");
      vi.advanceTimersByTime(HOURGLASS_SAID_MS - 1);
      expect(region).toHaveTextContent("BRB is done.");
      vi.advanceTimersByTime(1);
      expect(region).toHaveTextContent("");
      // An edge, then the section goes before its frame: nothing is left to run.
      await fire(3, 2);
      view.unmount();
      expect(vi.getTimerCount()).toBe(0);
    } finally {
      vi.useRealTimers();
    }
  });

  it("data-timer-seq on the section follows the adopted seq", async () => {
    store(entry("timer-1"));
    const view = mount();
    await flush();
    const hourglass = section(view);
    expect(hourglass).not.toHaveAttribute("data-timer-seq");
    await emit(4, state("timer-1"));
    expect(hourglass).toHaveAttribute("data-timer-seq", "4");
    await emit(2, state("timer-1", { phase: "running", ms: 298_400, text: "04:59", anchorMs: ENDS }));
    expect(hourglass).toHaveAttribute("data-timer-seq", "4");
    expect(within(card("timer-1")).getByText("Ready")).toBeInTheDocument();
    await emit(5, state("timer-1", { phase: "running", ms: 298_400, text: "04:59", anchorMs: ENDS }));
    expect(hourglass).toHaveAttribute("data-timer-seq", "5");
    expect(within(card("timer-1")).getByText("Running")).toBeInTheDocument();
  });

  it("a snapshot that saves a run doesn't rebuild a card mid-edit", async () => {
    store(entry("timer-1"));
    const view = mount();
    await flush();
    await emit(1, state("timer-1"));
    const field = view.getByRole("textbox", { name: "Length of BRB" }) as HTMLInputElement;
    const before = card("timer-1");
    fireEvent.input(field, { target: { value: "1:3" } });
    // A start: the store saves the run, a new entry for timer-1.
    await emit(2, state("timer-1", { phase: "running", ms: 298_400, text: "04:59", anchorMs: ENDS }));
    expect(saved()[0]!.run).toEqual({ phase: "running", totalMs: 300_000, anchorMs: ENDS });
    expect(card("timer-1")).toBe(before);
    expect(view.getByRole("textbox", { name: "Length of BRB" })).toBe(field);
    expect(field.value).toBe("1:3");
  });

  it("Copy path copies the file's full path and says so; a failure says to open the folder", async () => {
    store(entry("timer-1"));
    const DIR = "C:\\Users\\me\\AppData\\Roaming\\com.divora.voice\\timers";
    timerReply = (cmd) => (cmd === "timers_dir" ? DIR : null);
    const writeText = vi.fn(async (_: string) => {});
    Object.defineProperty(navigator, "clipboard", { value: { writeText }, configurable: true });
    try {
      const view = mountWithToast();
      await flush();
      await emit(1, state("timer-1"));
      expect(within(card("timer-1")).getByText("timer-1.txt")).toBeInTheDocument();
      expect(squash(within(card("timer-1")).getByText(/OBS text file:/).textContent)).toBe("OBS text file: timer-1.txt");
      const copy = view.getByRole("button", { name: "Copy the path of BRB's file" });
      expect(copy).toHaveTextContent("Copy path");
      fireEvent.click(copy);
      await flush();
      expect(writeText.mock.calls).toEqual([[`${DIR}\\timer-1.txt`]]);
      expect(view.getByTestId("toast-probe")).toHaveTextContent("Copied the file's path.");
      expect(view.getByTestId("toast-probe")).toHaveAttribute("data-tone", "success");

      writeText.mockRejectedValueOnce(new Error("denied"));
      fireEvent.click(copy);
      await flush();
      expect(view.getByTestId("toast-probe")).toHaveTextContent("Couldn't copy. Open the folder instead.");
      expect(view.getByTestId("toast-probe")).toHaveAttribute("data-tone", "error");

      // Open folder opens that folder.
      invokeCalls.length = 0;
      const open = view.getByRole("button", { name: "Open the timer files folder" });
      expect(open).toHaveTextContent("Open folder");
      fireEvent.click(open);
      await flush();
      expect(sent("open_external")).toEqual([{ target: DIR }]);
      // Asked once, then remembered.
      expect(sent("timers_dir")).toEqual([]);
    } finally {
      Reflect.deleteProperty(navigator, "clipboard");
    }
  });

  it("every control on a card is named for its timer", async () => {
    store(entry("timer-1", { hotkeys: { toggle: "F9" } }), entry("timer-2", { name: "Uptime", kind: "countup" }));
    mount();
    await flush();
    await emit(
      1,
      state("timer-1", { phase: "running", ms: 298_400, text: "04:59", anchorMs: ENDS }),
      state("timer-2", { name: "Uptime", kind: "countup", phase: "paused", ms: 60_000, totalMs: 0, text: "01:00" }),
    );
    for (const [id, name] of [
      ["timer-1", "BRB"],
      ["timer-2", "Uptime"],
    ] as const) {
      const named = controls(card(id));
      expect(named.length).toBeGreaterThan(8);
      for (const n of named) expect(n, n).toContain(name);
    }
    expect(controls(card("timer-1"))).toEqual([
      "Pause BRB",
      "Reset BRB",
      "Take 30 seconds off BRB",
      "Add 30 seconds to BRB",
      "Length of BRB",
      "BRB start or pause hotkey: F9",
      "Clear BRB start or pause hotkey",
      "BRB reset hotkey: not set",
      "Show BRB on the overlay",
      "Copy the path of BRB's file",
      "Rename BRB",
      "Delete BRB",
    ]);
  });

  it("the Stream Deck card names a timer's keys in Settings → Hourglass", async () => {
    const view = mount();
    await flush();
    const card = view.getByText("Using a Stream Deck").closest(".card");
    expect(squash(card!.textContent)).toContain(
      "or to a per-tile soundboard hotkey, or to one of a timer's keys in Settings → Hourglass. The button then fires",
    );
  });
});
