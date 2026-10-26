// v1.58.0: the Mixer's Push to modulate card, which now drives the engine.
//
// The card renders from `divora.ptm` (on or off, and which way the hold
// goes), saved by the store only from the engine's answer, and from the
// store's mirror of the engine's word. Its Hold to switch button holds the
// "button" source, and lets go of it wherever a pointerup or keyup may never
// come: the pointer leaving or cancelled, the button's own blur, the window's
// blur or the page hidden, and the card unmounting. The mock below keeps the
// engine's word as Rust does (a held bit per source, `seq` bumped only by a
// change), so the card is tested through the real store and its answers.

import { beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render } from "@solidjs/testing-library";
import { createSignal, Show } from "solid-js";

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
// One test renders the whole app, for its push-to-modulate key listener.
vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: vi.fn(() => ({
    minimize: vi.fn(),
    maximize: vi.fn(),
    unmaximize: vi.fn(),
    close: vi.fn(),
    isMaximized: vi.fn(async () => false),
    onFocusChanged: vi.fn(async () => () => undefined),
  })),
}));

import App from "../App";
import type { PtmMode } from "../audio/api";
import { AppProvider, useApp } from "../stores/app";
import { MixerScreen } from "./MixerScreen";

type Source = "hotkey" | "key" | "button" | "midi";
type Word = { mode: PtmMode; held: Record<Source, boolean>; seq: number };

const IN_APP = " in Divora's window. To use it in a call, set a key in Settings → Hotkeys.";
const ON_TITLE =
  "Switches your effects for the call, your headphones, recordings and Replay while held.";
const OFF_TITLE = "Turn on Push to modulate to try it.";

function wordOf(mode: PtmMode = "always", held: Partial<Record<Source, boolean>> = {}): Word {
  return { mode, held: { hotkey: false, key: false, button: false, midi: false, ...held }, seq: 0 };
}

/** What the engine answers: the word as a `PtmState` (§7.1). */
function stateOf(w: Word) {
  const held = Object.values(w.held).some(Boolean);
  const bypassed = w.mode === "apply" ? !held : w.mode === "bypass" ? held : false;
  return { mode: w.mode, held, bypassed, seq: w.seq };
}

interface Setup {
  /** `divora.ptm` before the store is made. */
  setting?: { on: boolean; hold: "apply" | "bypass" };
  /** The engine's word at launch (a global key may already hold it). */
  engine?: Word;
  hotkeyBindings?: Record<string, string>;
  midiMappings?: unknown[];
  /** The whole app (App's key listener included) instead of the Mixer
   *  alone; it opens on the Mixer. */
  whole?: boolean;
}

/** The Mixer through the real store over a mock engine word. `shown` mounts
 *  and unmounts the Mixer while the store lives on, as leaving the screen
 *  does. */
function setup(opts: Setup = {}) {
  localStorage.clear();
  if (opts.setting) localStorage.setItem("divora.ptm", JSON.stringify(opts.setting));
  if (opts.hotkeyBindings) {
    localStorage.setItem("divora.hotkeyBindings", JSON.stringify(opts.hotkeyBindings));
  }
  if (opts.midiMappings) localStorage.setItem("divora.midiMappings", JSON.stringify(opts.midiMappings));
  let w: Word = structuredClone(opts.engine ?? wordOf());
  invokeMock.mockImplementation(
    async (
      cmd: string,
      args?: { mode?: PtmMode; restore?: boolean; source?: Source; held?: boolean },
    ) => {
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
        case "audio_engine_status":
          return {
            running: false,
            monitoring: true,
            input: { rms: 0, peak: 0 },
            output: { rms: 0, peak: 0 },
          };
        // The engine's rules (ptm_transition): the card's set lets go of
        // every hold, the boot's restore keeps the global key's; a change
        // bumps seq and a set to what it has moves nothing.
        case "set_ptm_mode": {
          const held = args?.restore
            ? { ...wordOf().held, hotkey: w.held.hotkey }
            : { ...wordOf().held };
          const mode = args?.mode ?? "always";
          if (mode !== w.mode || JSON.stringify(held) !== JSON.stringify(w.held)) {
            w = { mode, held, seq: w.seq + 1 };
          }
          return stateOf(w);
        }
        case "set_ptm_held": {
          const source = args?.source as Source;
          if (w.held[source] !== !!args?.held) {
            w = { ...w, held: { ...w.held, [source]: !!args?.held }, seq: w.seq + 1 };
          }
          return stateOf(w);
        }
        default:
          return undefined;
      }
    },
  );

  const [shown, setShown] = createSignal(true);
  let api: ReturnType<typeof useApp> | null = null;
  function Harness() {
    api = useApp();
    return (
      <Show when={shown()}>
        <MixerScreen />
      </Show>
    );
  }
  if (opts.whole) localStorage.setItem("divora.wizardSeen", "true");
  const utils = render(() =>
    opts.whole ? (
      <App />
    ) : (
      <AppProvider>
        <Harness />
      </AppProvider>
    ),
  );
  return {
    ...utils,
    app: () => api!,
    word: () => w,
    setShown,
    toggle: () => utils.getByRole("switch", { name: "Push to modulate" }),
    tab: (name: "Hold to apply" | "Hold to bypass") => utils.getByRole("tab", { name }),
    hold: () => utils.getByTestId("ptm-hold") as HTMLButtonElement,
  };
}

const flush = async (): Promise<void> => {
  for (let i = 0; i < 10; i++) await Promise.resolve();
};

/** Every push-to-modulate command, in order, as `[cmd, args]`. */
const ptmCalls = (): unknown[][] =>
  invokeMock.mock.calls.filter(([cmd]) => cmd === "set_ptm_mode" || cmd === "set_ptm_held");

/** The card's line: the element the switch is described by. */
const line = (sw: HTMLElement): string =>
  document.getElementById(sw.getAttribute("aria-describedby") ?? "")?.textContent?.trim() ?? "";

// jsdom has no default actions, so these play Chromium's (WebView2's) part.
// A mouse press focuses a button unless mousedown is cancelled; the click
// counts it (detail 1). A keyboard click has detail 0.
function pointerPress(el: HTMLElement): void {
  fireEvent.pointerDown(el);
  if (fireEvent.mouseDown(el)) el.focus();
}

function pointerRelease(el: HTMLElement): void {
  fireEvent.pointerUp(el);
  fireEvent.mouseUp(el);
  fireEvent.click(el, { detail: 1 });
}

function mouseClick(el: HTMLElement): void {
  pointerPress(el);
  pointerRelease(el);
}

beforeEach(() => {
  invokeMock.mockReset();
  localStorage.clear();
});

describe("Push to modulate card (v1.58.0)", () => {
  it("is off by default: its line says so, and nothing else on it can be used", async () => {
    const { toggle, tab, hold } = setup();
    await flush();
    // The boot restored the saved mode (none: "always") and held nothing.
    expect(ptmCalls()).toEqual([["set_ptm_mode", { mode: "always", restore: true }]]);

    expect(toggle()).toHaveAttribute("aria-checked", "false");
    expect(line(toggle())).toBe("Off · your effects are always on.");
    expect(tab("Hold to apply")).toBeDisabled();
    expect(tab("Hold to apply")).toHaveAttribute("aria-selected", "true");
    expect(tab("Hold to bypass")).toBeDisabled();
    expect(hold()).toBeDisabled();
    expect(hold().title).toBe(OFF_TITLE);
    expect(hold().textContent?.trim()).toBe("Hold to switch · or press Space");
    expect(hold()).toHaveAttribute("aria-pressed", "false");

    // A disabled button holds nothing.
    pointerPress(hold());
    pointerRelease(hold());
    await flush();
    expect(ptmCalls()).toHaveLength(1);
  });

  it("the switch turns it on as Hold to apply, and the card follows the engine's answer", async () => {
    const { toggle, tab, hold } = setup();
    await flush();
    invokeMock.mockClear();

    fireEvent.click(toggle());
    expect(ptmCalls()).toEqual([["set_ptm_mode", { mode: "apply", restore: false }]]);
    await flush();
    expect(toggle()).toHaveAttribute("aria-checked", "true");
    expect(JSON.parse(localStorage.getItem("divora.ptm") ?? "null")).toEqual({
      on: true,
      hold: "apply",
    });
    expect(line(toggle())).toBe(`Your effects are on only while you hold Space${IN_APP}`);
    expect(tab("Hold to apply")).toBeEnabled();
    expect(tab("Hold to bypass")).toBeEnabled();
    expect(hold()).toBeEnabled();
    expect(hold().title).toBe(ON_TITLE);
    expect(hold().textContent?.trim()).toBe("Hold to switch · or press Space");

    fireEvent.click(tab("Hold to bypass"));
    await flush();
    expect(ptmCalls().at(-1)).toEqual(["set_ptm_mode", { mode: "bypass", restore: false }]);
    expect(tab("Hold to bypass")).toHaveAttribute("aria-selected", "true");
    expect(line(toggle())).toBe(`Your effects are off while you hold Space${IN_APP}`);

    // Off keeps the hold it had, so on again comes back as Hold to bypass.
    fireEvent.click(toggle());
    await flush();
    expect(ptmCalls().at(-1)).toEqual(["set_ptm_mode", { mode: "always", restore: false }]);
    expect(line(toggle())).toBe("Off · your effects are always on.");
    expect(hold()).toBeDisabled();
    expect(hold().title).toBe(OFF_TITLE);
    expect(JSON.parse(localStorage.getItem("divora.ptm") ?? "null")).toEqual({
      on: false,
      hold: "bypass",
    });
    fireEvent.click(toggle());
    await flush();
    expect(ptmCalls().at(-1)).toEqual(["set_ptm_mode", { mode: "bypass", restore: false }]);
  });

  it("picking the hold it already has sends it again, which lets go of a stuck hold", async () => {
    // A global key the engine still thinks is down (its release was lost).
    const { tab, app, word } = setup({
      setting: { on: true, hold: "apply" },
      engine: wordOf("apply", { hotkey: true }),
    });
    await flush();
    // The boot's restore keeps a held global key.
    expect(app().ptmState()?.held).toBe(true);
    invokeMock.mockClear();

    fireEvent.click(tab("Hold to apply"));
    await flush();
    expect(ptmCalls()).toEqual([["set_ptm_mode", { mode: "apply", restore: false }]]);
    expect(word().held.hotkey).toBe(false);
    expect(app().ptmState()?.held).toBe(false);
    expect(app().ptmBypassed()).toBe(true);
  });

  it("the line says the key works only in Divora's window unless a global key is bound, and names a learned MIDI control", async () => {
    const note = { kind: "note", data1: 36, channel: 0 };
    const cases: Array<[Setup, string]> = [
      [{ setting: { on: true, hold: "apply" } }, `Your effects are on only while you hold Space${IN_APP}`],
      [{ setting: { on: true, hold: "bypass" } }, `Your effects are off while you hold Space${IN_APP}`],
      // A global key: the card shows it, and it works in a call.
      [
        { setting: { on: true, hold: "apply" }, hotkeyBindings: { ptm: "F13" } },
        "Your effects are on only while you hold F13.",
      ],
      [
        { setting: { on: true, hold: "bypass" }, hotkeyBindings: { ptm: "Ctrl+Shift+M" } },
        "Your effects are off while you hold Ctrl+Shift+M.",
      ],
      // A learned `ptm` pad works in a call; Space still works only in
      // Divora's window, so the line says so and names the pad.
      [
        { setting: { on: true, hold: "apply" }, midiMappings: [{ id: "p", action: "ptm", trigger: note }] },
        "Your effects are on only while you hold Space in Divora's window, or with your MIDI control.",
      ],
      [
        { setting: { on: true, hold: "bypass" }, midiMappings: [{ id: "p", action: "ptm", trigger: note }] },
        "Your effects are off while you hold Space in Divora's window, or with your MIDI control.",
      ],
      // A global key and a pad: the key works in a call.
      [
        {
          setting: { on: true, hold: "apply" },
          hotkeyBindings: { ptm: "F13" },
          midiMappings: [{ id: "p", action: "ptm", trigger: note }],
        },
        "Your effects are on only while you hold F13.",
      ],
      // One that hasn't learned its pad yet can't fire; a global key on
      // another action doesn't count.
      [
        { setting: { on: true, hold: "apply" }, midiMappings: [{ id: "p", action: "ptm", trigger: null }] },
        `Your effects are on only while you hold Space${IN_APP}`,
      ],
      [
        {
          setting: { on: true, hold: "apply" },
          hotkeyBindings: { mute: "F20" },
          midiMappings: [{ id: "m", action: "mute", trigger: note }],
        },
        `Your effects are on only while you hold Space${IN_APP}`,
      ],
      // Off says nothing about keys.
      [{ setting: { on: false, hold: "apply" }, hotkeyBindings: { ptm: "F13" } }, "Off · your effects are always on."],
    ];
    for (const [opts, want] of cases) {
      const view = setup(opts);
      await flush();
      expect(line(view.toggle()), JSON.stringify(opts)).toBe(want);
      view.unmount();
      invokeMock.mockReset();
    }
  });

  it("Hold to switch holds on pointerdown; pointerup, pointerleave and pointercancel each let go", async () => {
    const { hold, word } = setup({ setting: { on: true, hold: "apply" } });
    await flush();
    for (const end of ["pointerUp", "pointerLeave", "pointerCancel"] as const) {
      invokeMock.mockClear();
      fireEvent.pointerDown(hold());
      await flush();
      expect(ptmCalls(), end).toEqual([["set_ptm_held", { source: "button", held: true }]]);
      expect(word().held.button).toBe(true);
      expect(hold()).toHaveAttribute("aria-pressed", "true");
      // Hold to apply, held: the engine applies the effects.
      expect(hold().textContent?.trim()).toBe("Effects on");

      fireEvent[end](hold());
      await flush();
      expect(ptmCalls(), end).toEqual([
        ["set_ptm_held", { source: "button", held: true }],
        ["set_ptm_held", { source: "button", held: false }],
      ]);
      expect(word().held.button).toBe(false);
      expect(hold()).toHaveAttribute("aria-pressed", "false");
      expect(hold().textContent?.trim()).toBe("Hold to switch · or press Space");
    }
  });

  it("held in Hold to bypass it says Effects off", async () => {
    const { hold } = setup({ setting: { on: true, hold: "bypass" } });
    await flush();
    fireEvent.pointerDown(hold());
    await flush();
    expect(hold().textContent?.trim()).toBe("Effects off");
    fireEvent.pointerUp(hold());
    await flush();
    expect(hold().textContent?.trim()).toBe("Hold to switch · or press Space");
  });

  it("after a pointer click on the switch, a hold or Hold to switch, the focus is back on the page", async () => {
    const { toggle, tab, hold } = setup();
    await flush();

    mouseClick(toggle());
    await flush();
    expect(toggle()).toHaveAttribute("aria-checked", "true");
    expect(document.activeElement).toBe(document.body);

    mouseClick(tab("Hold to bypass"));
    await flush();
    expect(tab("Hold to bypass")).toHaveAttribute("aria-selected", "true");
    expect(document.activeElement).toBe(document.body);

    invokeMock.mockClear();
    mouseClick(hold());
    await flush();
    expect(ptmCalls()).toEqual([
      ["set_ptm_held", { source: "button", held: true }],
      ["set_ptm_held", { source: "button", held: false }],
    ]);
    expect(document.activeElement).toBe(document.body);

    // Pressed and dragged off the button: it lets go and hands the focus
    // back all the same.
    invokeMock.mockClear();
    pointerPress(hold());
    expect(document.activeElement).toBe(hold());
    fireEvent.pointerLeave(hold());
    await flush();
    expect(ptmCalls()).toEqual([
      ["set_ptm_held", { source: "button", held: true }],
      ["set_ptm_held", { source: "button", held: false }],
    ]);
    expect(document.activeElement).toBe(document.body);
  });

  it("from the keyboard the switch and the holds keep the focus", async () => {
    const { toggle, tab } = setup();
    await flush();
    toggle().focus();
    fireEvent.click(toggle(), { detail: 0 });
    await flush();
    expect(toggle()).toHaveAttribute("aria-checked", "true");
    expect(document.activeElement).toBe(toggle());

    tab("Hold to bypass").focus();
    fireEvent.click(tab("Hold to bypass"), { detail: 0 });
    await flush();
    expect(tab("Hold to bypass")).toHaveAttribute("aria-selected", "true");
    expect(document.activeElement).toBe(tab("Hold to bypass"));
  });

  it("Enter or Space on the focused button holds while down; autorepeat sends nothing more", async () => {
    const { hold } = setup({ setting: { on: true, hold: "apply" } });
    await flush();
    for (const key of ["Enter", " "]) {
      invokeMock.mockClear();
      hold().focus();
      fireEvent.keyDown(hold(), { key });
      fireEvent.keyDown(hold(), { key, repeat: true });
      fireEvent.keyDown(hold(), { key, repeat: true });
      await flush();
      expect(ptmCalls(), key).toEqual([["set_ptm_held", { source: "button", held: true }]]);
      expect(hold()).toHaveAttribute("aria-pressed", "true");
      expect(hold().textContent?.trim()).toBe("Effects on");

      fireEvent.keyUp(hold(), { key });
      await flush();
      expect(ptmCalls(), key).toEqual([
        ["set_ptm_held", { source: "button", held: true }],
        ["set_ptm_held", { source: "button", held: false }],
      ]);
      expect(hold()).toHaveAttribute("aria-pressed", "false");
      // The keyboard keeps the focus on it.
      expect(document.activeElement).toBe(hold());
    }
  });

  it("a key still down after the hold was let go must be pressed again: its autorepeat doesn't hold", async () => {
    const { hold } = setup({ setting: { on: true, hold: "apply" } });
    await flush();
    invokeMock.mockClear();
    hold().focus();
    fireEvent.keyDown(hold(), { key: "Enter" });
    await flush();
    // Alt-Tab away and back with Enter still down.
    fireEvent.blur(window);
    await flush();
    fireEvent.keyDown(hold(), { key: "Enter", repeat: true });
    fireEvent.keyDown(hold(), { key: "Enter", repeat: true });
    await flush();
    expect(ptmCalls()).toEqual([
      ["set_ptm_held", { source: "button", held: true }],
      ["set_ptm_held", { source: "button", held: false }],
    ]);
    expect(hold()).toHaveAttribute("aria-pressed", "false");
    // A fresh press holds again.
    fireEvent.keyUp(hold(), { key: "Enter" });
    fireEvent.keyDown(hold(), { key: "Enter" });
    await flush();
    expect(ptmCalls().at(-1)).toEqual(["set_ptm_held", { source: "button", held: true }]);
  });

  it("held from the keyboard, it lets go when the focus moves; a pointer passing over doesn't", async () => {
    const { hold, toggle, word } = setup({ setting: { on: true, hold: "apply" } });
    await flush();
    invokeMock.mockClear();
    hold().focus();
    fireEvent.keyDown(hold(), { key: "Enter" });
    await flush();
    expect(word().held.button).toBe(true);

    // The mouse crossing the button is no pointer press of its own.
    fireEvent.pointerLeave(hold());
    await flush();
    expect(word().held.button).toBe(true);

    // Tab (or a dialog) takes the focus: the keyup will land elsewhere.
    toggle().focus();
    await flush();
    expect(ptmCalls()).toEqual([
      ["set_ptm_held", { source: "button", held: true }],
      ["set_ptm_held", { source: "button", held: false }],
    ]);
    expect(word().held.button).toBe(false);
    expect(hold()).toHaveAttribute("aria-pressed", "false");
    expect(hold().textContent?.trim()).toBe("Hold to switch · or press Space");

    // That keyup, landing elsewhere, sends nothing more.
    fireEvent.keyUp(toggle(), { key: "Enter" });
    await flush();
    expect(ptmCalls()).toHaveLength(2);
  });

  it("the window losing the focus, or the page hidden, lets go and clears the look", async () => {
    const { hold, word } = setup({ setting: { on: true, hold: "apply" } });
    await flush();

    invokeMock.mockClear();
    fireEvent.pointerDown(hold());
    await flush();
    fireEvent.blur(window);
    await flush();
    expect(ptmCalls()).toEqual([
      ["set_ptm_held", { source: "button", held: true }],
      ["set_ptm_held", { source: "button", held: false }],
    ]);
    expect(word().held.button).toBe(false);
    expect(hold()).toHaveAttribute("aria-pressed", "false");
    expect(hold().textContent?.trim()).toBe("Hold to switch · or press Space");
    // The pointerup that may follow sends nothing more.
    fireEvent.pointerUp(hold());
    await flush();
    expect(ptmCalls()).toHaveLength(2);

    invokeMock.mockClear();
    fireEvent.pointerDown(hold());
    await flush();
    const visibility = Object.getOwnPropertyDescriptor(document, "visibilityState");
    Object.defineProperty(document, "visibilityState", { configurable: true, get: () => "hidden" });
    try {
      fireEvent(document, new Event("visibilitychange"));
    } finally {
      if (visibility) Object.defineProperty(document, "visibilityState", visibility);
      else delete (document as { visibilityState?: unknown }).visibilityState;
    }
    await flush();
    expect(ptmCalls()).toEqual([
      ["set_ptm_held", { source: "button", held: true }],
      ["set_ptm_held", { source: "button", held: false }],
    ]);
    expect(hold()).toHaveAttribute("aria-pressed", "false");
  });

  it("leaving the Mixer while it is held lets go", async () => {
    const { hold, setShown, word } = setup({ setting: { on: true, hold: "apply" } });
    await flush();
    invokeMock.mockClear();
    hold().focus();
    fireEvent.keyDown(hold(), { key: "Enter" });
    await flush();
    expect(word().held.button).toBe(true);

    // A removed element gets no keyup, pointerup or blur.
    setShown(false);
    await flush();
    expect(ptmCalls()).toEqual([
      ["set_ptm_held", { source: "button", held: true }],
      ["set_ptm_held", { source: "button", held: false }],
    ]);
    expect(word().held.button).toBe(false);
  });

  it("through the whole app, after pointer clicks on the card, Space is push-to-modulate's key", async () => {
    const { toggle, tab, hold } = setup({ whole: true });
    await flush();
    mouseClick(toggle());
    await flush();
    mouseClick(tab("Hold to bypass"));
    await flush();
    mouseClick(hold());
    await flush();
    expect(document.activeElement).toBe(document.body);
    invokeMock.mockClear();

    // Aimed at the page, so App's listener holds its own source; nothing
    // on the card takes it.
    fireEvent.keyDown(document.body, { key: " ", code: "Space" });
    await flush();
    expect(ptmCalls()).toEqual([["set_ptm_held", { source: "key", held: true }]]);
    expect(hold().textContent?.trim()).toBe("Hold to switch · or press Space");
    fireEvent.keyUp(document.body, { key: " ", code: "Space" });
    await flush();
    expect(ptmCalls()).toEqual([
      ["set_ptm_held", { source: "key", held: true }],
      ["set_ptm_held", { source: "key", held: false }],
    ]);
  });
});
