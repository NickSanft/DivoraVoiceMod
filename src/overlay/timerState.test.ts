import { readFileSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import fixture from "./timer.fixture.json";
import {
  adoptTimers,
  fitTimerFont,
  isTimerId,
  isTimerKind,
  MAX_TIMER_ID_CHARS,
  readTimerSnapshot,
  TIMER_KINDS,
  TIMER_PHASES,
  TIMER_STATE_EVENT,
  TIMER_STATUS_CMD,
  type TimerSnapshot,
} from "./timerState";

// timer.fixture.json is Hourglass's wire contract, read by the backend's
// tests too (timers.rs `timer_wire_fixture_matches_rust`, `timer_id_table`):
// a change on either side fails the other side's suite.

const SAMPLES = [fixture.samples.idle, fixture.samples.running, fixture.samples.done];
const sorted = (o: object): string[] => Object.keys(o).sort();

describe("the timer wire (v1.59.0)", () => {
  it("names the event and the command the overlay page uses", () => {
    expect({ state: TIMER_STATE_EVENT }).toEqual(fixture.events);
    expect({ status: TIMER_STATUS_CMD }).toEqual(fixture.commands);
  });

  it("has the frozen phases, kinds and id length", () => {
    expect(TIMER_PHASES).toEqual(fixture.phases);
    expect(TIMER_KINDS).toEqual(fixture.kinds);
    expect(MAX_TIMER_ID_CHARS).toBe(fixture.limits.maxIdChars);
    for (const k of fixture.kinds) expect(isTimerKind(k)).toBe(true);
    for (const k of ["interval", "Countdown", "", 1, null]) expect(isTimerKind(k)).toBe(false);
  });

  it("the samples have exactly the frozen keys", () => {
    for (const s of SAMPLES) {
      expect(sorted(s)).toEqual(fixture.snapshotKeys);
      for (const t of s.timers) expect(sorted(t)).toEqual(fixture.stateKeys);
    }
    for (const r of fixture.runSamples) expect(sorted(r.state)).toEqual(fixture.stateKeys);
  });

  it("the module is overlay-importable: it imports nothing", () => {
    // The overlay page can't import src/audio/api.ts (status.ts), and this
    // file is shared with it.
    const source = readFileSync(join(process.cwd(), "src", "overlay", "timerState.ts"), "utf8");
    expect(source).not.toMatch(/^\s*import\b/m);
  });
});

describe("readTimerSnapshot", () => {
  it("reads every sample as it is", () => {
    for (const s of SAMPLES) expect(readTimerSnapshot(s)).toEqual(s);
    for (const r of fixture.runSamples) {
      expect(readTimerSnapshot({ seq: 9, timers: [r.state] })).toEqual({ seq: 9, timers: [r.state] });
    }
  });

  it.each([
    ["null (the e2e mock's answer)", null],
    ["undefined", undefined],
    ["a number", 3],
    ["a list", [fixture.samples.idle]],
    ["seq missing", { timers: [] }],
    ["seq negative", { seq: -1, timers: [] }],
    ["seq fractional", { seq: 1.5, timers: [] }],
    ["seq NaN", { seq: Number.NaN, timers: [] }],
    ["seq a string", { seq: "3", timers: [] }],
    ["timers missing", { seq: 1 }],
    ["timers not a list", { seq: 1, timers: { a: 1 } }],
  ])("%s → null", (_name, raw) => {
    expect(readTimerSnapshot(raw)).toBeNull();
  });

  it("drops an entry it can't read and keeps the rest", () => {
    const good = fixture.samples.running.timers[0]!;
    const bad: Array<[string, unknown]> = [
      ["an unknown phase", { ...good, id: "a", phase: "overtime" }],
      ["an unknown kind", { ...good, id: "b", kind: "interval" }],
      ["a bad id", { ...good, id: "Timer-1" }],
      ["a device name for an id", { ...good, id: "con" }],
      ["no name", { ...good, id: "c", name: undefined }],
      ["ms negative", { ...good, id: "d", ms: -1 }],
      ["ms fractional", { ...good, id: "e", ms: 1.5 }],
      ["totalMs a string", { ...good, id: "f", totalMs: "300000" }],
      ["fires missing", { ...good, id: "g", fires: undefined }],
      ["anchorMs fractional", { ...good, id: "h", anchorMs: 1.5 }],
      ["anchorMs missing", { ...good, id: "i", anchorMs: undefined }],
      ["text not a string", { ...good, id: "j", text: 5 }],
      ["not an object", "timer-9"],
      ["null", null],
    ];
    for (const [name, entry] of bad) {
      const read = readTimerSnapshot({ seq: 4, timers: [entry, good] });
      expect(read, name).toEqual({ seq: 4, timers: [good] });
    }
  });

  it("keeps the first of two entries with one id, and leaves out keys it doesn't know", () => {
    const [a, b] = fixture.samples.idle.timers;
    const read = readTimerSnapshot({
      seq: 2,
      later: true,
      timers: [{ ...a, colour: "red" }, b, { ...a, name: "Again" }],
    });
    expect(read).toEqual({ seq: 2, timers: [a, b] });
  });

  it("reads a negative anchor (a wall clock before 1970) and a count-up past 10 000 hours", () => {
    const t = { ...fixture.samples.running.timers[1]!, anchorMs: -5, ms: 36_000_000_000 };
    expect(readTimerSnapshot({ seq: 1, timers: [t] })?.timers).toEqual([t]);
  });
});

describe("adoptTimers", () => {
  const snap = (seq: number): TimerSnapshot => ({ seq, timers: [] });

  // §7.10: the holder only moves to a newer seq.
  it.each([
    ["nothing held", null, snap(3), 3],
    ["a newer one", snap(5), snap(6), 6],
    ["the same seq", snap(5), snap(5), 5],
    ["an older one (a late answer)", snap(5), snap(4), 5],
    ["null (unreadable, or the mock's null)", snap(5), null, 5],
  ])("%s", (_name, held, incoming, want) => {
    expect(adoptTimers(held, incoming)?.seq).toBe(want);
  });

  it("keeps the held object when nothing newer comes", () => {
    const held = snap(5);
    expect(adoptTimers(held, snap(5))).toBe(held);
    expect(adoptTimers(held, null)).toBe(held);
    expect(adoptTimers(null, null)).toBeNull();
  });
});

describe("isTimerId", () => {
  it.each(fixture.ids.valid)("accepts %j", (id) => {
    expect(isTimerId(id)).toBe(true);
  });

  it.each(fixture.ids.invalid)("refuses %j", (id) => {
    expect(isTimerId(id)).toBe(false);
  });

  it("refuses anything that isn't a string", () => {
    for (const v of [null, undefined, 1, ["timer-1"], { id: "timer-1" }]) expect(isTimerId(v)).toBe(false);
  });

  it("the fixture's longest valid id is the limit", () => {
    expect(Math.max(...fixture.ids.valid.map((s) => s.length))).toBe(MAX_TIMER_ID_CHARS);
  });
});

describe("fitTimerFont", () => {
  // min(0.78·h, w / (0.62·chars + 0.4)): `05:00` is 5 characters,
  // `1:00:00` 7, `100:00:00` 9.
  it.each([
    [300, 100, 5, 78],
    [300, 100, 7, 300 / (0.62 * 7 + 0.4)],
    [300, 100, 9, 300 / (0.62 * 9 + 0.4)],
    [90, 30, 5, 0.78 * 30],
    [90, 30, 7, 90 / (0.62 * 7 + 0.4)],
    [90, 30, 9, 90 / (0.62 * 9 + 0.4)],
  ])("%d×%d, %d characters → %d px", (w, h, chars, want) => {
    expect(fitTimerFont(w, h, chars)).toBeCloseTo(want, 9);
  });

  it("never lets the text grow wider than the box", () => {
    for (const [w, h] of [
      [300, 100],
      [90, 30],
      [40, 400],
    ] as const) {
      for (const chars of [5, 7, 8, 9]) {
        const px = fitTimerFont(w, h, chars);
        expect((0.62 * chars + 0.4) * px).toBeLessThanOrEqual(w + 1e-9);
        expect(px).toBeLessThanOrEqual(0.78 * h + 1e-9);
      }
    }
  });

  it("an empty box gets 0", () => {
    expect(fitTimerFont(0, 100, 5)).toBe(0);
    expect(fitTimerFont(300, -1, 5)).toBe(0);
    expect(fitTimerFont(Number.NaN, 100, 5)).toBe(0);
  });
});
