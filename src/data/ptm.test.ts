import { describe, expect, it } from "vitest";
import type { PtmState } from "../audio/api";
import type { ChainEntry, EffectId } from "../types";
import {
  adoptPtm,
  defaultPtmSetting,
  levelPtm,
  ptmButtonLabel,
  ptmButtonTitle,
  ptmCardLine,
  ptmKeyMatches,
  ptmModeOf,
  readPtmSetting,
  readPtmState,
  settingOf,
  transforms,
} from "./ptm";

const s = (mode: PtmState["mode"], held: boolean, bypassed: boolean, seq: number): PtmState => ({
  mode,
  held,
  bypassed,
  seq,
});

describe("adoptPtm", () => {
  // The mirror only moves to a newer seq, as the mute's (adoptMute).
  it.each([
    [null, null, null],
    [null, s("apply", false, true, 1), s("apply", false, true, 1)],
    [s("apply", true, false, 5), null, s("apply", true, false, 5)],
    // stale: a tick built before the answer
    [s("apply", true, false, 5), s("apply", false, true, 4), s("apply", true, false, 5)],
    [s("apply", true, false, 5), s("apply", false, true, 5), s("apply", true, false, 5)],
    [s("apply", true, false, 5), s("bypass", false, false, 6), s("bypass", false, false, 6)],
  ])("held %j, incoming %j → %j", (held, incoming, want) => {
    expect(adoptPtm(held, incoming)).toEqual(want);
  });

  it("keeps the held object when nothing newer comes, so the store sets nothing", () => {
    const held = s("always", false, false, 5);
    expect(adoptPtm(held, s("apply", false, true, 5))).toBe(held);
    expect(adoptPtm(held, null)).toBe(held);
  });
});

describe("readPtmState", () => {
  it("reads a command's answer, every mode", () => {
    expect(readPtmState({ mode: "always", held: false, bypassed: false, seq: 0 })).toEqual(
      s("always", false, false, 0),
    );
    expect(readPtmState({ mode: "apply", held: false, bypassed: true, seq: 3 })).toEqual(
      s("apply", false, true, 3),
    );
    expect(readPtmState({ mode: "bypass", held: true, bypassed: true, seq: 9 })).toEqual(
      s("bypass", true, true, 9),
    );
  });

  it.each([
    ["mode missing", { held: false, bypassed: false, seq: 1 }],
    ["mode unknown", { mode: "toggle", held: false, bypassed: false, seq: 1 }],
    ["mode in another case", { mode: "Apply", held: false, bypassed: true, seq: 1 }],
    ["held missing", { mode: "apply", bypassed: true, seq: 1 }],
    ["held not a boolean", { mode: "apply", held: 0, bypassed: true, seq: 1 }],
    ["bypassed missing", { mode: "apply", held: false, seq: 1 }],
    ["bypassed not a boolean", { mode: "apply", held: false, bypassed: "true", seq: 1 }],
    ["seq missing", { mode: "apply", held: false, bypassed: true }],
    ["seq negative", { mode: "apply", held: false, bypassed: true, seq: -1 }],
    ["seq fractional", { mode: "apply", held: false, bypassed: true, seq: 1.5 }],
    ["seq NaN", { mode: "apply", held: false, bypassed: true, seq: Number.NaN }],
    ["seq a string", { mode: "apply", held: false, bypassed: true, seq: "3" }],
    ["null (a mock's answer)", null],
    ["undefined", undefined],
    ["a number", 3],
  ])("%s → null", (_name, raw) => {
    expect(readPtmState(raw)).toBeNull();
  });
});

describe("levelPtm", () => {
  it("reads a tick's ptm", () => {
    expect(levelPtm({ ptm: s("apply", true, false, 7) })).toEqual(s("apply", true, false, 7));
  });

  it("absent is no information, never always", () => {
    // A test-emitted or older tick has no ptm.
    expect(levelPtm({ running: true })).toBeNull();
    expect(levelPtm({ inputMuted: false, inputMuteSeq: 2 })).toBeNull();
  });

  it("a malformed ptm → null", () => {
    expect(levelPtm({ ptm: { mode: "apply", held: false } as never })).toBeNull();
    expect(levelPtm({ ptm: null as never })).toBeNull();
    expect(levelPtm(null as never)).toBeNull();
    expect(levelPtm(undefined as never)).toBeNull();
  });
});

describe("readPtmSetting", () => {
  it("absent or malformed reads as off, Hold to apply", () => {
    for (const raw of [
      null,
      undefined,
      "apply",
      3,
      [],
      {},
      { on: true },
      { hold: "bypass" },
      { on: "true", hold: "apply" },
      { on: true, hold: "always" },
      { on: true, hold: "Apply" },
    ]) {
      expect(readPtmSetting(raw)).toEqual({ on: false, hold: "apply" });
    }
  });

  it("reads a valid setting", () => {
    expect(readPtmSetting({ on: true, hold: "bypass" })).toEqual({ on: true, hold: "bypass" });
    expect(readPtmSetting({ on: false, hold: "bypass" })).toEqual({ on: false, hold: "bypass" });
    expect(readPtmSetting({ on: true, hold: "apply", extra: 1 })).toEqual({ on: true, hold: "apply" });
  });

  it("the default is a fresh object each time", () => {
    const a = defaultPtmSetting();
    a.on = true;
    expect(defaultPtmSetting()).toEqual({ on: false, hold: "apply" });
  });
});

describe("ptmModeOf and settingOf", () => {
  it("on asks for its hold, off for always", () => {
    expect(ptmModeOf({ on: true, hold: "apply" })).toBe("apply");
    expect(ptmModeOf({ on: true, hold: "bypass" })).toBe("bypass");
    expect(ptmModeOf({ on: false, hold: "apply" })).toBe("always");
    expect(ptmModeOf({ on: false, hold: "bypass" })).toBe("always");
  });

  // The design's §7.5 rows.
  it("an answer's mode → the setting to save; always keeps the hold", () => {
    expect(settingOf("always", { on: true, hold: "bypass" })).toEqual({ on: false, hold: "bypass" });
    expect(settingOf("always", { on: false, hold: "apply" })).toEqual({ on: false, hold: "apply" });
    for (const prev of [
      { on: false, hold: "bypass" as const },
      { on: true, hold: "apply" as const },
    ]) {
      expect(settingOf("apply", prev)).toEqual({ on: true, hold: "apply" });
      expect(settingOf("bypass", prev)).toEqual({ on: true, hold: "bypass" });
    }
  });

  it("round-trips: the mode a setting asks for gives that setting back", () => {
    for (const on of [true, false]) {
      for (const hold of ["apply", "bypass"] as const) {
        expect(settingOf(ptmModeOf({ on, hold }), { on, hold })).toEqual({ on, hold });
      }
    }
  });
});

describe("the words that follow the state (§7.5)", () => {
  // The in-app key holds only while no control in the window has the
  // keyboard, so the line says so.
  const IN_APP =
    " in Divora's window, when no control in it has the keyboard. To use it in a call, set a key in Settings → Hotkeys.";
  // No global key, but a learned MIDI control: the key the card names still
  // works only in Divora's window; the control works in a call.
  const OR_MIDI =
    " in Divora's window, when no control in it has the keyboard, or with your MIDI control.";

  it.each([
    [{ on: false, hold: "apply" }, "key", "Off · your effects are always on."],
    [{ on: false, hold: "bypass" }, null, "Off · your effects are always on."],
    [{ on: false, hold: "apply" }, "midi", "Off · your effects are always on."],
    [{ on: true, hold: "apply" }, "key", "Your effects are on only while you hold Space."],
    [{ on: true, hold: "apply" }, "midi", `Your effects are on only while you hold Space${OR_MIDI}`],
    [{ on: true, hold: "apply" }, null, `Your effects are on only while you hold Space${IN_APP}`],
    [{ on: true, hold: "bypass" }, "key", "Your effects are off while you hold Space."],
    [{ on: true, hold: "bypass" }, "midi", `Your effects are off while you hold Space${OR_MIDI}`],
    [{ on: true, hold: "bypass" }, null, `Your effects are off while you hold Space${IN_APP}`],
  ] as const)("ptmCardLine(%j, Space, in call: %s)", (setting, inCall, want) => {
    expect(ptmCardLine(setting, "Space", inCall)).toBe(want);
  });

  it("ptmButtonLabel", () => {
    expect(ptmButtonLabel(false, false, "Space")).toBe("Hold to switch · or press Space");
    expect(ptmButtonLabel(false, true, "Ctrl+M")).toBe("Hold to switch · or press Ctrl+M");
    expect(ptmButtonLabel(true, false, "Space")).toBe("Effects on");
    expect(ptmButtonLabel(true, true, "Space")).toBe("Effects off");
  });

  it("ptmButtonTitle", () => {
    expect(ptmButtonTitle(false)).toBe("Turn on Push to modulate to try it.");
    expect(ptmButtonTitle(true)).toBe(
      "Switches your effects for the call, your headphones, recordings and Replay while held.",
    );
  });

  it("no copy says the chain, instant or at once, or names a Stream Deck", () => {
    const copy = [
      ptmCardLine({ on: false, hold: "apply" }, "Space", "key"),
      ptmCardLine({ on: true, hold: "apply" }, "Space", null),
      ptmCardLine({ on: true, hold: "bypass" }, "Space", null),
      ptmCardLine({ on: true, hold: "apply" }, "Space", "midi"),
      ptmCardLine({ on: true, hold: "bypass" }, "Space", "midi"),
      ptmButtonLabel(false, false, "Space"),
      ptmButtonLabel(true, false, "Space"),
      ptmButtonLabel(true, true, "Space"),
      ptmButtonTitle(false),
      ptmButtonTitle(true),
    ].join(" ");
    expect(copy).not.toMatch(/\bchain\b|instant|at once|stream deck|bypass/i);
  });
});

describe("transforms (§7.5)", () => {
  const e = (id: EffectId, enabled: boolean): ChainEntry => ({ id, enabled, vals: {} });
  it.each([
    [[], false],
    [[e("gate", true)], false],
    [[e("pitch", false)], false],
    [[e("gate", true), e("pitch", false)], false],
    [[e("gate", true), e("pitch", true)], true],
    [[e("denoiser", true)], true],
    [[e("gate", false), e("reverb", true)], true],
  ] as const)("%j → %s", (chain, want) => {
    expect(transforms(chain)).toBe(want);
  });
});

describe("ptmKeyMatches (§7.7)", () => {
  type Ev = Parameters<typeof ptmKeyMatches>[1];
  const ev = (
    key: string,
    code: string,
    mods: Partial<Pick<Ev, "ctrlKey" | "altKey" | "shiftKey" | "metaKey">> = {},
  ): Ev => ({
    key,
    code,
    ctrlKey: false,
    altKey: false,
    shiftKey: false,
    metaKey: false,
    ...mods,
  });

  it.each([
    ["Space", ev(" ", "Space"), "down", true],
    ["Space", ev(" ", "Space", { shiftKey: true }), "down", false],
    ["Ctrl+Shift+M", ev("M", "KeyM"), "down", false],
    ["Ctrl+Shift+M", ev("M", "KeyM", { ctrlKey: true, shiftKey: true }), "down", true],
    ["Ctrl+Shift+M", ev("m", "KeyM"), "up", true],
    ["CommandOrControl+F13", ev("F13", "F13", { ctrlKey: true }), "down", true],
    ["Super+K", ev("k", "KeyK", { metaKey: true }), "down", true],
    ["Ctrl+Alt+Shift+F24", ev("F24", "F24"), "down", false],
  ] as const)("%s, %j, %s → %s", (accelerator, e, phase, want) => {
    expect(ptmKeyMatches(accelerator, e, phase)).toBe(want);
  });

  it("a press needs the modifiers exactly: none missing, none extra", () => {
    const acc = "Ctrl+Alt+Shift+F24";
    const all = { ctrlKey: true, altKey: true, shiftKey: true };
    expect(ptmKeyMatches(acc, ev("F24", "F24", all), "down")).toBe(true);
    expect(ptmKeyMatches(acc, ev("F24", "F24", { ...all, metaKey: true }), "down")).toBe(false);
    expect(ptmKeyMatches(acc, ev("F24", "F24", { ctrlKey: true, altKey: true }), "down")).toBe(
      false,
    );
    expect(ptmKeyMatches("F13", ev("F13", "F13", { ctrlKey: true }), "down")).toBe(false);
  });

  it("every modifier name the parser takes, in any case", () => {
    const k = (mods: Partial<Pick<Ev, "ctrlKey" | "altKey" | "shiftKey" | "metaKey">>) =>
      ev("K", "KeyK", mods);
    for (const name of ["Ctrl", "Control", "CommandOrControl", "CmdOrCtrl", "ctrl", "CONTROL"]) {
      expect(ptmKeyMatches(`${name}+K`, k({ ctrlKey: true }), "down")).toBe(true);
    }
    for (const name of ["Alt", "Option", "alt"]) {
      expect(ptmKeyMatches(`${name}+K`, k({ altKey: true }), "down")).toBe(true);
    }
    expect(ptmKeyMatches("shift+K", k({ shiftKey: true }), "down")).toBe(true);
    for (const name of ["Super", "Meta", "Cmd", "Command", "Win"]) {
      expect(ptmKeyMatches(`${name}+K`, k({ metaKey: true }), "down")).toBe(true);
    }
  });

  it("a modifier name it doesn't know never matches a press", () => {
    expect(ptmKeyMatches("Hyper+K", ev("K", "KeyK"), "down")).toBe(false);
    // A "+" key leaves empty tokens; it never matched and still doesn't.
    expect(ptmKeyMatches("Shift++", ev("+", "Equal", { shiftKey: true }), "down")).toBe(false);
  });

  it("the trailing key matches as it always has", () => {
    // Space by code (its key is " "), one character in any case, others exactly.
    expect(ptmKeyMatches("Space", ev(" ", "KeyX"), "down")).toBe(false);
    expect(ptmKeyMatches("M", ev("m", "KeyM"), "down")).toBe(true);
    expect(ptmKeyMatches("F13", ev("F13", "F13"), "down")).toBe(true);
    expect(ptmKeyMatches("F13", ev("f13", "F13"), "down")).toBe(false);
    expect(ptmKeyMatches("M", ev("N", "KeyN"), "up")).toBe(false);
  });

  it("a release matches the trailing key whatever the modifiers", () => {
    expect(ptmKeyMatches("Ctrl+Shift+M", ev("M", "KeyM", { ctrlKey: true }), "up")).toBe(true);
    expect(ptmKeyMatches("Space", ev(" ", "Space", { shiftKey: true }), "up")).toBe(true);
  });
});
