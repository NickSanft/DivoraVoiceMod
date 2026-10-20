import { describe, expect, it } from "vitest";
import type { InputMute } from "../audio/api";
import {
  adoptMute,
  levelMute,
  MUTE_CC_DOWN,
  MUTE_CC_UP,
  muteAnnouncement,
  muteCcStep,
  readInputMute,
} from "./mute";

const m = (muted: boolean, seq: number): InputMute => ({ muted, seq });

describe("adoptMute", () => {
  // The design's table (§7.3): the mirror only moves to a newer seq.
  it.each([
    [null, null, null],
    [null, m(true, 1), m(true, 1)],
    [m(true, 5), null, m(true, 5)],
    [m(true, 5), m(false, 4), m(true, 5)], // stale: a tick built before the answer
    [m(true, 5), m(true, 5), m(true, 5)],
    [m(true, 5), m(false, 6), m(false, 6)],
  ])("held %j, incoming %j → %j", (held, incoming, want) => {
    expect(adoptMute(held, incoming)).toEqual(want);
  });

  it("keeps the held object when nothing newer comes, so the store sets nothing", () => {
    const held = m(true, 5);
    expect(adoptMute(held, m(false, 5))).toBe(held);
    expect(adoptMute(held, null)).toBe(held);
  });
});

describe("readInputMute", () => {
  it("reads a command's answer", () => {
    expect(readInputMute({ muted: true, seq: 3 })).toEqual(m(true, 3));
    expect(readInputMute({ muted: false, seq: 0 })).toEqual(m(false, 0));
  });

  it.each([
    ["seq missing", { muted: true }],
    ["seq negative", { muted: true, seq: -1 }],
    ["seq fractional", { muted: true, seq: 1.5 }],
    ["seq NaN", { muted: true, seq: Number.NaN }],
    ["seq a string", { muted: true, seq: "3" }],
    ["muted missing", { seq: 3 }],
    ["muted not a boolean", { muted: 1, seq: 3 }],
    ["null (a mock's answer)", null],
    ["undefined", undefined],
    ["a number", 3],
  ])("%s → null", (_name, raw) => {
    expect(readInputMute(raw)).toBeNull();
  });
});

describe("levelMute", () => {
  it("reads a tick's pair", () => {
    expect(levelMute({ inputMuted: true, inputMuteSeq: 3 })).toEqual(m(true, 3));
    expect(levelMute({ inputMuted: false, inputMuteSeq: 0 })).toEqual(m(false, 0));
  });

  it("absent fields are no information, never unmuted", () => {
    // A test-emitted or e2e tick has neither field.
    expect(levelMute({ running: true })).toBeNull();
    expect(levelMute({ inputMuted: false })).toBeNull();
    expect(levelMute({ inputMuteSeq: 4 })).toBeNull();
  });

  it.each([
    ["seq negative", { inputMuted: true, inputMuteSeq: -1 }],
    ["seq fractional", { inputMuted: true, inputMuteSeq: 0.5 }],
    ["seq NaN", { inputMuted: true, inputMuteSeq: Number.NaN }],
    ["seq a string", { inputMuted: true, inputMuteSeq: "3" }],
    ["muted not a boolean", { inputMuted: "true", inputMuteSeq: 3 }],
  ])("%s → null", (_name, update) => {
    expect(levelMute(update as never)).toBeNull();
  });

  it("null, undefined or a number → null", () => {
    expect(levelMute(null as never)).toBeNull();
    expect(levelMute(undefined as never)).toBeNull();
    expect(levelMute(3 as never)).toBeNull();
  });
});

describe("muteAnnouncement", () => {
  it.each([
    [true, false, "Your voice is muted."],
    [true, true, "Your voice is muted."],
    [false, true, "Your voice is unmuted."],
    [false, false, ""], // nothing to say before the first mute
  ])("muted %s, everMuted %s → %j", (muted, everMuted, want) => {
    expect(muteAnnouncement(muted, everMuted)).toBe(want);
  });
});

describe("muteCcStep", () => {
  it("mutes at 64 and above, unmutes at 59 and below", () => {
    expect(MUTE_CC_UP).toBe(64);
    expect(MUTE_CC_DOWN).toBe(59);
  });

  // The design's table (§7.5). `act` is the mute to set, or null.
  it.each([
    // First message after load: only up acts.
    [undefined, 127, true, true],
    [undefined, 64, true, true],
    [undefined, 0, false, null], // a control resting low after load does nothing
    [undefined, 59, false, null],
    [undefined, 60, undefined, null],
    [undefined, 63, undefined, null],
    // Down: rising past halfway mutes; the band keeps it down.
    [false, 64, true, true],
    [false, 127, true, true],
    [false, 60, false, null],
    [false, 63, false, null],
    [false, 0, false, null],
    [false, 59, false, null],
    // Up: falling below the band unmutes; jitter in the band doesn't.
    [true, 63, true, null], // jitter below 64 doesn't unmute
    [true, 60, true, null],
    [true, 59, false, false],
    [true, 0, false, false],
    [true, 64, true, null], // a fader resting high doesn't re-mute
    [true, 127, true, null],
  ])("prevUp %s, value %i → up %s, act %s", (prevUp, value, up, act) => {
    expect(muteCcStep(prevUp, value)).toEqual({ up, act });
  });

  it("a knob jittering around halfway acts once, not on every message", () => {
    let side: boolean | undefined = false;
    const acts: boolean[] = [];
    for (const v of [62, 64, 63, 64, 62, 61, 64, 60, 63, 64]) {
      const s = muteCcStep(side, v);
      side = s.up;
      if (s.act !== null) acts.push(s.act);
    }
    expect(acts).toEqual([true]);
  });
});
