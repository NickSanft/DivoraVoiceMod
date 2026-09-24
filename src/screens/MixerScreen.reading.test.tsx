// Voice reading panel — the parts only a rendered DOM can prove.
//
// Three things are load-bearing here and each has a test that fails loudly if
// it regresses:
//
//   1. OFF BY DEFAULT. Nothing is analyzed until somebody asks.
//   2. EVERY STATE IS EXPLICIT. Stopped, muted, too quiet, still listening and
//      live are five different things that all look like "no numbers"; a panel
//      that blurred them would quietly describe a signal it no longer has.
//      A pause holds the last window and says it is held.
//   3. NO HIGH-FREQUENCY VALUE IN A LIVE REGION. `ReactiveCard` pairs
//      role="meter" with an aria-live region safely because that region holds
//      rarely-changing guardrail text. A value updating several times a second
//      in a live region makes a screen reader talk continuously and renders
//      the app unusable, so the live values live in role="meter" and the one
//      live region holds a sentence that changes rarely by construction.
//
// Plus the honesty guardrail, mirroring the Rust one: no word on this card
// names a feeling or addresses the user in the second person about their
// state.

import { readFileSync } from "node:fs";
import { join } from "node:path";
import { beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render } from "@solidjs/testing-library";

// The Mixer always mounts the cast-trail canvas; jsdom has no 2D context and
// would otherwise fill the run with "not implemented" noise unrelated to the
// panel under test.
beforeAll(() => {
  HTMLCanvasElement.prototype.getContext = (() => null) as never;
});

const invokeMock = vi.fn();
let readingHandler: ((payload: unknown) => void) | null = null;

vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
}));
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(async (event: string, cb: (e: { payload: unknown }) => void) => {
    if (event === "voice-reading") {
      readingHandler = (payload: unknown) => cb({ payload });
    }
    return () => {
      readingHandler = null;
    };
  }),
}));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn(async () => null) }));
vi.mock("@tauri-apps/plugin-shell", () => ({
  open: vi.fn(async () => undefined),
}));

import { AppProvider, useApp } from "../stores/app";
import { readingAnnouncement, readingView } from "./MixerScreen";
import type { ReadingMetrics, VoiceReadingUpdate } from "../audio/api";

const SILENT: ReadingMetrics = {
  energyDbfs: -120,
  energyRangeDb: 0,
  f0Hz: 0,
  f0RangeSt: 0,
  voicedRatio: 0,
  paceOps: 0,
  brightnessHz: 0,
};

const VOICED: ReadingMetrics = {
  energyDbfs: -17.4,
  energyRangeDb: 9.2,
  f0Hz: 118,
  f0RangeSt: 7.3,
  voicedRatio: 0.62,
  paceOps: 4.1,
  brightnessHz: 2280,
};

function update(over: Partial<VoiceReadingUpdate> = {}): VoiceReadingUpdate {
  return {
    state: "speaking",
    dry: VOICED,
    wet: { ...VOICED, f0Hz: 84, brightnessHz: 1450 },
    words: ["bright", "wide range", "fast"],
    calibrated: true,
    wetBypassed: false,
    stale: false,
    ageMs: 0,
    ...over,
  };
}

/** Render the Mixer's right rail through the real store. A handle exposes the
 *  store so a test can drive the switch and push an event payload. */
function setup() {
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
      case "voice_reading":
        return update({ state: "quiet", words: [], calibrated: false });
      default:
        return undefined;
    }
  });

  let api: ReturnType<typeof useApp> | null = null;
  function Harness() {
    api = useApp();
    return <MixerScreenUnderTest />;
  }
  const utils = render(() => (
    <AppProvider>
      <Harness />
    </AppProvider>
  ));
  return { ...utils, app: () => api! };
}

// The card lives inside MixerScreen's right rail; rendering the whole screen
// pulls in the spell circle's canvas work, so drive the card through the same
// store the screen uses and assert on the rail itself.
import { MixerScreen } from "./MixerScreen";
const MixerScreenUnderTest = MixerScreen;

/** The banned words, read from the Rust source rather than copied.
 *
 *  A hand-kept copy drifted: review found this list 29 entries against Rust's
 *  56, missing `bored` — the exact word that defeated the first version of the
 *  Rust guard. Reading the one list keeps them in step, the way
 *  `src/data/presets.mirror.test.ts` mirrors the bundled presets. */
const BANNED: string[] = (() => {
  const source = readFileSync(
    join(process.cwd(), "divora-core", "src", "dsp", "reading.rs"),
    "utf8",
  );
  const block = /const EMOTIONS: &\[&str\] = &\[([\s\S]*?)\];/.exec(source);
  if (!block) throw new Error("could not find EMOTIONS in reading.rs");
  const words = [...block[1]!.matchAll(/"([^"]+)"/g)].map((w) => w[1]!);
  if (words.length < 40) {
    throw new Error(`only ${words.length} banned words parsed from reading.rs`);
  }
  return words;
})();

describe("voice reading — view resolution (pure)", () => {
  it("shows the engine state rather than guessing from a quiet window", () => {
    expect(readingView(null, false).kind).toBe("stopped");
    expect(readingView(null, true).kind).toBe("listening");
    expect(readingView(update({ state: "stopped" }), true).kind).toBe("stopped");
    expect(readingView(update({ state: "muted" }), true).kind).toBe("muted");
  });

  it("separates 'too quiet' from 'not calibrated yet'", () => {
    const quiet = update({ state: "quiet", calibrated: true, words: [] });
    expect(readingView(quiet, true).kind).toBe("quiet");
    const fresh = update({ state: "quiet", calibrated: false, words: [] });
    expect(readingView(fresh, true).kind).toBe("listening");
    // Speaking but not yet calibrated: the numbers are real, only the
    // comparison isn't.
    const warming = update({ calibrated: false, words: [] });
    expect(readingView(warming, true).kind).toBe("listening");
    expect(readingView(warming, true).hasNumbers).toBe(true);
  });

  it("holds the last window through a pause instead of decaying", () => {
    const held = update({ state: "quiet", stale: true, ageMs: 4200 });
    const v = readingView(held, true);
    expect(v.kind).toBe("held");
    expect(v.stale).toBe(true);
    expect(v.hasNumbers).toBe(true);
  });

  it("collapses live / quiet / held into one announcement", () => {
    // These three alternate every second or two while somebody talks. A
    // screen reader narrating that is the defect this guards against.
    const spoken = (
      k: "live" | "quiet" | "held",
    ): string => readingAnnouncement({ kind: k, hasNumbers: true, stale: false });
    expect(spoken("live")).toBe(spoken("quiet"));
    expect(spoken("quiet")).toBe(spoken("held"));
    // The rare states do get their own line.
    expect(
      readingAnnouncement({ kind: "stopped", hasNumbers: false, stale: false }),
    ).not.toBe(spoken("live"));
    expect(
      readingAnnouncement({ kind: "muted", hasNumbers: false, stale: false }),
    ).not.toBe(spoken("live"));
  });

  it("takes the passing-through answer from the engine, not from the numbers", () => {
    // The engine correlates the two taps and puts the answer on the wire.
    // The panel used to re-derive it here and required the levels to match,
    // which is wrong whenever loudness normalization is on: the loudness
    // stage sits between the chain and the wet tap, so a bypassed chain
    // still arrives at a different level.
    const bypassed = update({
      wetBypassed: true,
      wet: { ...VOICED, energyDbfs: VOICED.energyDbfs + 6 },
    });
    expect(bypassed.wetBypassed).toBe(true);
    expect(readingView(bypassed, true).hasNumbers).toBe(true);
  });
});

describe("voice reading — the card", () => {
  beforeEach(() => {
    invokeMock.mockReset();
    readingHandler = null;
    try {
      window.localStorage.clear();
    } catch {
      /* fine */
    }
  });

  it("is off by default and asks the backend for no analysis", async () => {
    const { getByLabelText } = setup();
    const toggle = getByLabelText("Voice reading");
    expect(toggle).toHaveAttribute("aria-checked", "false");
    // Nothing on the card body is rendered, and — the point of the test —
    // the app never asked the backend to start analyzing.
    const asked = invokeMock.mock.calls.filter(
      ([cmd]) => cmd === "set_voice_reading_enabled",
    );
    expect(asked.every(([, args]) => (args as { enabled: boolean }).enabled === false)).toBe(
      true,
    );
    expect(document.querySelector("[data-testid='reading-dry']")).toBeNull();
  });

  it("turning it on asks the backend once, and off again turns it off", async () => {
    const { getByLabelText } = setup();
    fireEvent.click(getByLabelText("Voice reading"));
    await Promise.resolve();
    expect(invokeMock).toHaveBeenCalledWith("set_voice_reading_enabled", {
      enabled: true,
    });
    fireEvent.click(getByLabelText("Voice reading"));
    await Promise.resolve();
    expect(invokeMock).toHaveBeenCalledWith("set_voice_reading_enabled", {
      enabled: false,
    });
  });

  it("renders every state explicitly", async () => {
    const { getByLabelText, getByTestId, queryByTestId } = setup();
    fireEvent.click(getByLabelText("Voice reading"));
    await Promise.resolve();

    const push = (u: VoiceReadingUpdate) => {
      expect(readingHandler).not.toBeNull();
      readingHandler!(u);
    };

    push(update({ state: "stopped", words: [], calibrated: false }));
    expect(getByTestId("reading-state")).toHaveTextContent(/engine stopped/i);
    expect(queryByTestId("reading-phrase")).toBeNull();

    push(update({ state: "muted", words: [], calibrated: false }));
    expect(getByTestId("reading-state")).toHaveTextContent(/input silent/i);

    push(update({ state: "quiet", words: [], calibrated: true }));
    expect(getByTestId("reading-state")).toHaveTextContent(/too quiet/i);

    push(update({ state: "quiet", words: [], calibrated: false }));
    expect(getByTestId("reading-state")).toHaveTextContent(/still listening/i);

    push(update());
    expect(getByTestId("reading-state")).toHaveTextContent(/live/i);
  });

  it("a pause shows the held window as visibly stale, with its age", async () => {
    const { getByLabelText, getByTestId } = setup();
    fireEvent.click(getByLabelText("Voice reading"));
    await Promise.resolve();

    readingHandler!(update({ state: "quiet", stale: true, ageMs: 4200 }));
    expect(getByTestId("reading-state")).toHaveTextContent(/paused/i);
    expect(getByTestId("reading-state")).toHaveTextContent(/4\.2 s ago/i);
    // The numbers are still there — held, not decayed toward a quieter,
    // flatter, narrower version of the speaker — but dimmed.
    const dry = getByTestId("reading-dry");
    expect(dry).toHaveTextContent("118 Hz");
    expect(dry.style.opacity).toBe("0.6");
  });

  it("renders the phrase and the numbers for both halves", async () => {
    const { getByLabelText, getByTestId } = setup();
    fireEvent.click(getByLabelText("Voice reading"));
    await Promise.resolve();
    readingHandler!(update());

    expect(getByTestId("reading-phrase")).toHaveTextContent(
      "bright, wide range, fast",
    );
    const dry = getByTestId("reading-dry");
    expect(dry).toHaveTextContent("118 Hz");
    expect(dry).toHaveTextContent("7.3 st");
    expect(dry).toHaveTextContent("-17.4 dB");
    // Whole onsets, with the unit spelled out: the estimator's own scatter
    // is about ±1.3/s, so a tenth would be showing noise as measurement, and
    // a bare "/s" reads as syllables per second, which this is not.
    expect(dry).toHaveTextContent("4 onsets/s");

    // The after-effects half is unmistakably labelled as the chain's output,
    // and carries the chain's numbers, not the mic's.
    const wet = getByTestId("reading-wet");
    expect(wet).toHaveTextContent(/after effects/i);
    expect(wet).toHaveTextContent(/chain's output/i);
    expect(wet).toHaveTextContent(/preset change moves these numbers/i);
    expect(wet).toHaveTextContent("84 Hz");
  });

  it("says when the chain is only passing the signal through", async () => {
    const { getByLabelText, getByTestId, queryByTestId } = setup();
    fireEvent.click(getByLabelText("Voice reading"));
    await Promise.resolve();

    readingHandler!(update());
    expect(queryByTestId("reading-passthrough")).toBeNull();

    // Numbers that happen to match are NOT the signal: two summaries can look
    // alike while the chain is doing plenty, and with loudness normalization
    // on a real passthrough arrives at a different level. The engine decides
    // by correlating the taps, and the panel believes it.
    readingHandler!(update({ wet: VOICED }));
    expect(queryByTestId("reading-passthrough")).toBeNull();

    // Push-to-modulate held up, or a preset with nothing enabled. Saying so
    // is what stops it reading as "the preset stopped working" — and it still
    // says so when the levels differ, which is the case that used to fail.
    readingHandler!(
      update({
        wetBypassed: true,
        wet: { ...VOICED, energyDbfs: VOICED.energyDbfs + 6 },
      }),
    );
    expect(getByTestId("reading-passthrough")).toHaveTextContent(
      /passing the signal through/i,
    );
  });

  it("carries one line saying what this is", async () => {
    const { getByLabelText, getByTestId } = setup();
    fireEvent.click(getByLabelText("Voice reading"));
    await Promise.resolve();
    readingHandler!(update());
    expect(getByTestId("reading-disclaimer")).toHaveTextContent(
      /measured properties of the sound/i,
    );
    expect(getByTestId("reading-disclaimer")).toHaveTextContent(
      /not a reading of the person/i,
    );
  });
});

describe("voice reading — accessibility structure", () => {
  beforeEach(() => {
    invokeMock.mockReset();
    readingHandler = null;
    try {
      window.localStorage.clear();
    } catch {
      /* fine */
    }
  });

  it("puts no high-frequency value inside a live region", async () => {
    const { getByLabelText } = setup();
    fireEvent.click(getByLabelText("Voice reading"));
    await Promise.resolve();
    readingHandler!(update());

    const regions = Array.from(document.querySelectorAll("[aria-live]"));
    expect(regions.length).toBeGreaterThan(0);
    for (const region of regions) {
      const text = region.textContent ?? "";
      // No number that moves with the signal.
      expect(text).not.toMatch(/\d+(\.\d+)?\s*(Hz|st|dB|\/s)/i);
      // No descriptor phrase either — it changes once a window.
      expect(text).not.toMatch(/bright|wide range|fast/i);
      // And no meter may be nested inside one.
      expect(region.querySelector("[role='meter']")).toBeNull();
      expect(region.querySelector("[aria-valuenow]")).toBeNull();
    }
  });

  it("exposes the live values as meters with a spoken value", async () => {
    const { getByLabelText, getByRole } = setup();
    fireEvent.click(getByLabelText("Voice reading"));
    await Promise.resolve();
    readingHandler!(update());

    const pitch = getByRole("meter", { name: "Pitch, your voice" });
    expect(pitch).toHaveAttribute("aria-valuenow", "118");
    expect(pitch).toHaveAttribute("aria-valuetext", "118 hertz");
    // Both halves are separately addressable, so a screen-reader user can
    // tell the mic from the chain's output without relying on reading order.
    expect(getByRole("meter", { name: "Pitch, after effects" })).toHaveAttribute(
      "aria-valuenow",
      "84",
    );
    expect(
      getByRole("meter", { name: "Pitch range, your voice" }),
    ).toHaveAttribute("aria-valuetext", "7.3 semitones");
    expect(getByRole("meter", { name: "Level, your voice" })).toHaveAttribute(
      "aria-valuetext",
      "-17.4 decibels",
    );
    expect(getByRole("meter", { name: "Pace, your voice" })).toHaveAttribute(
      "aria-valuetext",
      "4 onsets per second",
    );
  });

  it("names no feeling and makes no claim about the person", async () => {
    const { getByLabelText } = setup();
    const toggle = getByLabelText("Voice reading");
    const card = toggle.closest(".card");
    expect(card).not.toBeNull();
    fireEvent.click(toggle);
    await Promise.resolve();

    // Sweep every state, so a string that only appears in one of them is
    // still audited.
    const states: VoiceReadingUpdate[] = [
      update(),
      update({ state: "quiet", stale: true, ageMs: 9000 }),
      update({ state: "muted", words: [], calibrated: false }),
      update({ state: "stopped", words: [], calibrated: false }),
      update({ state: "quiet", words: [], calibrated: false }),
      update({ wet: VOICED }),
    ];
    for (const s of states) {
      readingHandler!(s);
      const text = (card!.textContent ?? "").toLowerCase();
      expect(text.length).toBeGreaterThan(50);
      for (const word of BANNED) {
        expect(text, `"${word}" reached the panel`).not.toContain(word);
      }
      // The accessible value text is an attribute, not body text, so sweep it
      // separately rather than let a spoken string slip the audit.
      for (const meter of Array.from(card!.querySelectorAll("[role='meter']"))) {
        const spoken = `${meter.getAttribute("aria-label") ?? ""} ${
          meter.getAttribute("aria-valuetext") ?? ""
        }`.toLowerCase();
        for (const word of BANNED) {
          expect(spoken, `"${word}" reached a meter`).not.toContain(word);
        }
      }
    }
  });
});
