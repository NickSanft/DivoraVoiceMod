// v1.56.0: the Voice status card's Mute / Unmute button — the on-screen way
// out of a hotkey or timer mute.
//
// The button sends the state its label names, never a toggle. Its label comes
// from the store's mirror, which can lag the engine (a hotkey mute reaches it
// on the next tick; the boot read may not have answered). A stale "Mute" that
// toggled would unmute the call; one that sends `muted: true` is a no-op whose
// answer corrects the label. The mock below keeps the engine's word, so a test
// can make the mirror stale on purpose.

import { beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render } from "@solidjs/testing-library";

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
// v1.58.0: one test renders the whole app, for its push-to-modulate key.
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
import { AppProvider, useApp } from "../stores/app";
import { MixerScreen } from "./MixerScreen";

type Word = { muted: boolean; seq: number };

/** Render the Mixer through the real store over a mock engine word. With
 *  `bootReads: false` the boot read answers nothing, so the mirror stays
 *  unknown whatever the word says. With `whole`, the whole app (its
 *  push-to-modulate key listener included), on the Mixer. */
function setup(start: Word, bootReads: boolean, whole = false) {
  let word: Word = { ...start };
  invokeMock.mockImplementation(async (cmd: string, args?: { muted?: boolean }) => {
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
      // The engine's rules: a change bumps seq; a set to the value it
      // already has moves nothing.
      case "input_mute":
        return bootReads ? word : undefined;
      case "set_input_muted":
        if (args?.muted !== word.muted) word = { muted: !!args?.muted, seq: word.seq + 1 };
        return word;
      case "toggle_input_muted":
        word = { muted: !word.muted, seq: word.seq + 1 };
        return word;
      case "audio_engine_status":
        return {
          running: false,
          monitoring: true,
          input: { rms: 0, peak: 0 },
          output: { rms: 0, peak: 0 },
        };
      default:
        return undefined;
    }
  });

  if (whole) {
    // Past the first-run wizard; the shell opens on the Mixer.
    localStorage.setItem("divora.wizardSeen", "true");
    const utils = render(() => <App />);
    return { ...utils, word: () => word };
  }

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
  return { ...utils, app: () => api!, word: () => word };
}

const flush = async (): Promise<void> => {
  for (let i = 0; i < 10; i++) await Promise.resolve();
};

const muteCalls = (): unknown[][] =>
  invokeMock.mock.calls.filter(([cmd]) => cmd === "set_input_muted" || cmd === "toggle_input_muted");

const ptmHeldCalls = (): unknown[][] =>
  invokeMock.mock.calls.filter(([cmd]) => cmd === "set_ptm_held");

// jsdom has no default actions for the mouse or keys, so these two play
// Chromium's (WebView2's) part. A mouse press focuses a button unless
// mousedown is cancelled, and the click counts it (detail 1). Keys go to the
// focused element, or the page when nothing is; a focused button clicks on
// Enter's keydown and on Space's keyup, with detail 0.
function mouseClick(el: HTMLElement): void {
  if (fireEvent.mouseDown(el)) el.focus();
  fireEvent.mouseUp(el);
  fireEvent.click(el, { detail: 1 });
}

function press(key: "Space" | "Enter"): void {
  const target = (document.activeElement as HTMLElement | null) ?? document.body;
  const init = { key: key === "Space" ? " " : "Enter", code: key };
  const down = fireEvent.keyDown(target, init);
  const isButton = target instanceof HTMLButtonElement;
  if (key === "Enter" && down && isButton) fireEvent.click(target, { detail: 0 });
  const up = fireEvent.keyUp(target, init);
  if (key === "Space" && up && isButton) fireEvent.click(target, { detail: 0 });
}

beforeEach(() => {
  invokeMock.mockReset();
  localStorage.clear();
});

describe("the Voice status card's mute button (v1.56.0)", () => {
  it("says Mute, never disabled, and its title names the call, headphones, recordings and Replay", async () => {
    const { getByTestId } = setup({ muted: false, seq: 0 }, true);
    await flush();
    const button = getByTestId("mute-button");
    expect(button.textContent?.trim()).toBe("Mute");
    expect(button).not.toBeDisabled();
    expect(button).toHaveAttribute("type", "button");
    expect(button.title).toBe(
      "Mute your voice for the call, your headphones, WAV and screen recordings and Replay. Clips and Speak still play.",
    );
    expect(button.title).toContain("headphones");
    expect(button.title).toContain("screen recordings");
    expect(button.title).toContain("Replay");
  });

  it("with the engine muted and the mirror not yet, a click sends mute — not a toggle", async () => {
    // A hotkey muted the engine; the mirror hasn't heard (the boot read
    // answered nothing), so the button still reads Mute.
    const { getByTestId, word } = setup({ muted: true, seq: 3 }, false);
    await flush();
    const button = () => getByTestId("mute-button");
    expect(button().textContent?.trim()).toBe("Mute");

    fireEvent.click(button());
    await flush();
    expect(muteCalls()).toEqual([["set_input_muted", { muted: true }]]);
    // The engine stays muted, and the answer corrects the label.
    expect(word()).toEqual({ muted: true, seq: 3 });
    expect(button().textContent?.trim()).toBe("Unmute");
    expect(button().title).toBe("Unmute your voice");

    fireEvent.click(button());
    await flush();
    expect(muteCalls()).toEqual([
      ["set_input_muted", { muted: true }],
      ["set_input_muted", { muted: false }],
    ]);
    expect(word()).toEqual({ muted: false, seq: 4 });
    expect(button().textContent?.trim()).toBe("Mute");
  });

  it("while muted the card says the voice is off and clips still play", async () => {
    const { getByTestId, getByText, queryByText } = setup({ muted: false, seq: 0 }, true);
    await flush();
    expect(queryByText("Your voice is off · clips and Speak still play")).toBeNull();

    fireEvent.click(getByTestId("mute-button"));
    await flush();
    expect(getByText("Your voice is off · clips and Speak still play")).toBeInTheDocument();
    // Danger colours on the button while muted.
    expect(getByTestId("mute-button")).toHaveClass("btn-danger");
  });

  it("a mute the boot read finds shows at once, and the click unmutes", async () => {
    const { getByTestId, word } = setup({ muted: true, seq: 7 }, true);
    await flush();
    expect(getByTestId("mute-button").textContent?.trim()).toBe("Unmute");
    // The boot only read: it sent nothing.
    expect(muteCalls()).toEqual([]);

    fireEvent.click(getByTestId("mute-button"));
    await flush();
    expect(muteCalls()).toEqual([["set_input_muted", { muted: false }]]);
    expect(word()).toEqual({ muted: false, seq: 8 });
    expect(getByTestId("mute-button").textContent?.trim()).toBe("Mute");
  });

  it("after a mouse click, Space (push-to-modulate) and Enter don't press it again", async () => {
    const { getByTestId, word } = setup({ muted: false, seq: 0 }, true);
    await flush();
    const button = getByTestId("mute-button");

    mouseClick(button);
    await flush();
    expect(muteCalls()).toEqual([["set_input_muted", { muted: true }]]);
    // Focus is back on the page, where App's push-to-modulate fallback
    // listens (it acts only on keys whose target is document.body).
    expect(document.activeElement).toBe(document.body);

    press("Space");
    press("Enter");
    await flush();
    expect(muteCalls()).toEqual([["set_input_muted", { muted: true }]]);
    expect(word()).toEqual({ muted: true, seq: 1 });
    expect(button.textContent?.trim()).toBe("Unmute");
  });

  // v1.58.0: and that Space now reaches push-to-modulate. Through the whole
  // app, so App's own key listener (which acts only on keys aimed at the
  // page) is the one that hears it.
  it("through the whole app, Space after a Mute click holds push-to-modulate and doesn't unmute", async () => {
    const { getByTestId, word } = setup({ muted: false, seq: 0 }, true, true);
    await flush();
    const button = getByTestId("mute-button");

    mouseClick(button);
    await flush();
    expect(muteCalls()).toEqual([["set_input_muted", { muted: true }]]);
    expect(document.activeElement).toBe(document.body);

    press("Space");
    await flush();
    expect(ptmHeldCalls()).toEqual([
      ["set_ptm_held", { source: "key", held: true }],
      ["set_ptm_held", { source: "key", held: false }],
    ]);
    expect(muteCalls()).toEqual([["set_input_muted", { muted: true }]]);
    expect(word()).toEqual({ muted: true, seq: 1 });
  });

  it("from the keyboard it keeps focus, so Space and Enter keep working it", async () => {
    const { getByTestId, word } = setup({ muted: false, seq: 0 }, true);
    await flush();
    const button = getByTestId("mute-button");
    button.focus();

    press("Space");
    await flush();
    expect(muteCalls()).toEqual([["set_input_muted", { muted: true }]]);
    expect(document.activeElement).toBe(button);

    press("Space");
    await flush();
    press("Enter");
    await flush();
    expect(muteCalls()).toEqual([
      ["set_input_muted", { muted: true }],
      ["set_input_muted", { muted: false }],
      ["set_input_muted", { muted: true }],
    ]);
    expect(word()).toEqual({ muted: true, seq: 3 });
    expect(document.activeElement).toBe(button);
  });
});
