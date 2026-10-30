import { describe, expect, it } from "vitest";
import fixture from "../overlay/timer.fixture.json";
import type { TimerPhase } from "../overlay/timerState";
import {
  ACTION_KINDS,
  actionsForSync,
  actionWords,
  canAddAction,
  FINISH_TIMER_CMD,
  LATE_ONE_SHOTS_MS,
  MAX_ACTIONS,
  MAX_LINE_CHARS,
  MAX_TARGET_CHARS,
  pairingWarnings,
  pickRandomPreset,
  readAction,
  readReturns,
  readStoredActions,
  readTimerActionsEvent,
  reapplies,
  restartNotice,
  restores,
  returnsAfterStart,
  revertPlan,
  SOUNDBOARD_STOPPED_EVENT,
  TIMER_ACTIONS_EVENT,
  TIMER_ACTIONS_HOOKS,
  TIMER_WHYS,
  withAdded,
  withRemoved,
  withReplaced,
  type ActionNames,
  type PresetReturn,
  type TimerAction,
} from "./timerActions";
import { readTimers } from "./timers";

const MUTE: TimerAction = { kind: "setMuted", muted: true };
const UNMUTE: TimerAction = { kind: "setMuted", muted: false };
const REVERT: TimerAction = { kind: "revertPreset" };
const LINE = (text: string): TimerAction => ({ kind: "speak", text, voiceId: "af_heart" });
const OBS = { kind: "obs", op: "sceneBack" };

describe("the wire is the fixture's (v1.60.0)", () => {
  it("names the event, the stop event and End now's command as the fixture does", () => {
    expect({
      events: { actions: TIMER_ACTIONS_EVENT, soundboardStopped: SOUNDBOARD_STOPPED_EVENT },
      commands: { finish: FINISH_TIMER_CMD },
    }).toEqual(fixture.actionWire);
  });

  it("has the fixture's limits", () => {
    expect({
      maxActions: MAX_ACTIONS,
      maxLineChars: MAX_LINE_CHARS,
      maxTargetChars: MAX_TARGET_CHARS,
      lateOneShotsMs: LATE_ONE_SHOTS_MS,
    }).toEqual(fixture.actionLimits);
  });

  it("has the fixture's whys and hooks, in the backend's order", () => {
    expect(TIMER_WHYS).toEqual(fixture.whys);
    expect(TIMER_ACTIONS_HOOKS).toEqual(fixture.hooks);
  });

  it("ACTION_KINDS is the fixture's actionKinds, in its order", () => {
    expect(Object.keys(ACTION_KINDS)).toEqual(fixture.actionKinds.map((k) => k.kind));
    for (const row of fixture.actionKinds) {
      expect(ACTION_KINDS[row.kind as TimerAction["kind"]], row.kind).toEqual({ runsIn: row.runsIn, at: row.at });
    }
  });
});

describe("readAction", () => {
  // The seam with the backend's `action_from_wire`, which reads the same
  // rows (timers.rs / timer_actions.rs `action_from_wire_table`).
  it("reads every valid fixture sample as stated, with where it runs and what it restores", () => {
    for (const row of fixture.actionSamples.valid) {
      const read = readAction(row.action);
      expect(read, row.name).toEqual(row.read);
      expect(ACTION_KINDS[read!.kind].runsIn, row.name).toBe(row.runsIn);
      expect(restores(read!), row.name).toBe(row.restores);
      expect(reapplies(read!), row.name).toBe(row.reapplies);
      // What it reads is what it sends: read again, it is the same.
      expect(readAction(row.read), row.name).toEqual(row.read);
      const fields = Object.keys(row.read).filter((k) => k !== "kind").sort();
      expect(fields, row.name).toEqual(fixture.actionKinds.find((k) => k.kind === read!.kind)!.fields);
    }
  });

  it("skips every skipped fixture sample, the page-only lone surrogate included", () => {
    for (const row of fixture.actionSamples.skipped) {
      let action: unknown = row.action;
      const field = row.appendLoneSurrogate;
      if (field !== undefined) {
        // JSON can't hold a lone surrogate, so the row holds a good line and
        // names the field to append one to: without it, the line reads.
        expect(readAction(action), row.name).not.toBeNull();
        const a = row.action as Record<string, unknown>;
        action = { ...a, [field]: `${String(a[field])}\uD800` };
      }
      expect(readAction(action), row.name).toBeNull();
    }
  });

  it("refuses a lone surrogate in any text, and keeps a pair", () => {
    expect(readAction({ kind: "preset", presetId: "villager\uDC00" })).toBeNull();
    expect(readAction({ kind: "tile", tileId: "\uD83D" })).toBeNull();
    expect(readAction({ kind: "speak", text: "Back.", voiceId: "af_heart\uD800" })).toBeNull();
    expect(readAction({ kind: "speak", text: "Back \u{1F642}", voiceId: "af_heart" })).toEqual(LINE("Back \u{1F642}"));
  });

  it("trims a line as the backend's str::trim does", () => {
    // U+0085 is White_Space (Rust trims it, JS's trim doesn't); U+FEFF
    // isn't (JS's trim trims it, Rust doesn't).
    expect(readAction(LINE("\u0085I'm back.\u0085"))).toEqual(LINE("I'm back."));
    expect(readAction(LINE("\u0085"))).toBeNull();
    expect(readAction(LINE("﻿I'm back."))).toEqual(LINE("﻿I'm back."));
  });
});

describe("readStoredActions and actionsForSync", () => {
  // The seam with the backend's `hooks_from_wire`: the same rows.
  it("read every fixture listSample's lists as stated", () => {
    for (const row of fixture.listSamples) {
      const entry = row.entry as Record<string, unknown>;
      for (const hook of ["onStart", "onFire"] as const) {
        expect(readStoredActions(entry[hook]), `${row.name} ${hook}`).toEqual(row.rows[hook]);
        expect(actionsForSync(entry[hook]), `${row.name} ${hook}`).toEqual(row.read[hook]);
      }
    }
  });

  it("a list that isn't an array has no rows and sends nothing", () => {
    for (const raw of [undefined, null, 5, "x", {}, { 0: MUTE, length: 1 }]) {
      expect(readStoredActions(raw)).toEqual([]);
      expect(actionsForSync(raw)).toEqual([]);
    }
  });
});

describe("readTimerActionsEvent", () => {
  it("reads every fixture actionsEventSample, one per why, with exactly actionsEventKeys", () => {
    expect(fixture.actionsEventSamples.map((r) => r.event.why)).toEqual(fixture.whys);
    for (const row of fixture.actionsEventSamples) {
      const ev = readTimerActionsEvent(row.event);
      expect(ev, row.name).toEqual(row.event);
      expect(Object.keys(ev!).sort(), row.name).toEqual(fixture.actionsEventKeys);
    }
    const zero = fixture.actionsEventSamples.find((r) => r.event.why === "zero")!;
    expect(readTimerActionsEvent(zero.event)!.actions.map((a) => a.kind)).toEqual([
      "preset",
      "randomPreset",
      "revertPreset",
      "tile",
    ]);
  });

  const good = { seq: 3, id: "timer-1", hook: "end", why: "endNow", lateMs: 0, actions: [REVERT] };

  it.each([
    ["not an object", null],
    ["a list", [good]],
    ["seq negative", { ...good, seq: -1 }],
    ["seq fractional", { ...good, seq: 1.5 }],
    ["id not an id", { ...good, id: "Timer 1" }],
    ["hook unknown", { ...good, hook: "middle" }],
    ["why unknown", { ...good, why: "later" }],
    ["why restart (never on the wire)", { ...good, why: "restart" }],
    ["lateMs negative", { ...good, lateMs: -1 }],
    ["lateMs missing", { ...good, lateMs: undefined }],
    ["actions not a list", { ...good, actions: {} }],
  ])("%s → null", (_name, raw) => {
    expect(readTimerActionsEvent(raw)).toBeNull();
  });

  it("leaves out actions it can't read, and other keys", () => {
    const ev = readTimerActionsEvent({ ...good, fires: 2, actions: [OBS, REVERT, { kind: "tile" }] });
    expect(ev).toEqual({ ...good, actions: [REVERT] });
  });
});

describe("readReturns", () => {
  it("reads each return it can, the first per timer; none when it isn't a list", () => {
    const a = { timer: "timer-1", from: "hollow-king", to: "villager" };
    const b = { timer: "timer-2", from: "villager", to: "x".repeat(300) };
    expect(
      readReturns([
        a,
        { timer: "timer-1", from: "clean", to: "villager" },
        { timer: "Timer 3", from: "a", to: "b" },
        { timer: "timer-4", from: "", to: "b" },
        { timer: "timer-5", from: "a", to: 5 },
        "timer-6",
        { ...b, extra: true },
      ]),
    ).toEqual([a, b]);
    for (const raw of [undefined, null, "x", {}, 5]) expect(readReturns(raw)).toEqual([]);
  });
});

describe("editing a list by index", () => {
  const seven = [MUTE, OBS, UNMUTE, REVERT, { kind: "saveReplay" }, { kind: "panic" }, LINE("Hi.")];

  it("canAddAction counts every stored entry, readable or not, and one line per list", () => {
    expect(canAddAction(seven, "tile")).toBe("ok");
    expect(canAddAction([...seven, OBS], "tile")).toBe("full");
    expect(canAddAction([...seven, OBS], "speak")).toBe("full");
    expect(canAddAction(seven, "speak")).toBe("hasLine");
    // An unreadable line doesn't use the list's one line up.
    expect(canAddAction([{ kind: "speak", text: "", voiceId: "af_heart" }], "speak")).toBe("ok");
    expect(canAddAction(5, "speak")).toBe("ok");
  });

  it("adds Stop all clips and lines at the top and anything else at the end, keeping every entry", () => {
    expect(withAdded([OBS, MUTE], UNMUTE)).toEqual([OBS, MUTE, UNMUTE]);
    expect(withAdded([OBS, MUTE], { kind: "panic" })).toEqual([{ kind: "panic" }, OBS, MUTE]);
    expect(withAdded({}, MUTE)).toEqual([MUTE]);
  });

  it("replaces and removes by index, every other entry in place; no such index changes nothing", () => {
    const list = [MUTE, OBS, UNMUTE];
    expect(withReplaced(list, 2, REVERT)).toEqual([MUTE, OBS, REVERT]);
    expect(withRemoved(list, 0)).toEqual([OBS, UNMUTE]);
    expect(withReplaced(list, 3, REVERT)).toEqual(list);
    expect(withReplaced(list, -1, REVERT)).toEqual(list);
    expect(withRemoved(list, 7)).toEqual(list);
    // The list it was given is never changed.
    expect(list).toEqual([MUTE, OBS, UNMUTE]);
  });
});

describe("pairingWarnings (§7.9)", () => {
  const VILLAGER: TimerAction = { kind: "preset", presetId: "villager" };
  it.each([
    ["the Break template", [MUTE], [UNMUTE], []],
    ["the Villager flow", [VILLAGER], [REVERT], []],
    ["a mute with no unmute", [MUTE], [], ["muteNoUnmute"]],
    ["a switch with no switch back", [VILLAGER], [UNMUTE], ["switchNoRevert"]],
    ["a random switch with no switch back", [{ kind: "randomPreset" }], [], ["switchNoRevert"]],
    ["a switch back with no switch", [], [REVERT], ["revertNoSwitch"]],
    ["a mute and a switch with nothing at zero", [MUTE, VILLAGER], [], ["muteNoUnmute", "switchNoRevert"]],
    ["only actions that don't run count", [MUTE, VILLAGER], [OBS, ...Array(7).fill(OBS), UNMUTE, REVERT], [
      "muteNoUnmute",
      "switchNoRevert",
    ]],
    ["lists that aren't arrays", 5, {}, []],
  ])("%s", (_name, onStart, onFire, want) => {
    expect(pairingWarnings(onStart, onFire)).toEqual(want);
  });
});

describe("returns (§7.3)", () => {
  const exists = (id: string) => id !== "gone";
  const r = (timer: string, from: string, to: string): PresetReturn => ({ timer, from, to });

  it("a start notes a return, and a second switch keeps where it came from", () => {
    const one = returnsAfterStart([], "timer-1", "hollow-king", "villager");
    expect(one).toEqual([r("timer-1", "hollow-king", "villager")]);
    expect(returnsAfterStart(one, "timer-1", "villager", "glass-oracle")).toEqual([
      r("timer-1", "hollow-king", "glass-oracle"),
    ]);
  });

  it.each([
    ["R1 no return", [r("b", "x", "y")], "a", "y", true, null, [r("b", "x", "y")]],
    ["R2 no revert", [r("a", "hk", "v")], "a", "v", false, null, []],
    ["R3 still ours", [r("a", "hk", "v")], "a", "v", true, "hk", []],
    ["R4 the one before is gone", [r("a", "gone", "v")], "a", "v", true, null, []],
    ["R5 stacked on", [r("a", "hk", "v"), r("b", "v", "go")], "a", "go", true, null, [r("b", "hk", "go")]],
    ["R6 moved on", [r("a", "hk", "v")], "a", "clean", true, null, []],
  ] as const)("%s", (_name, rs, timer, live, revert, switchTo, returns) => {
    expect(revertPlan([...rs], timer, live, exists, revert)).toEqual({ switchTo, returns });
  });

  it("two timers that both switch back nest; mixed ones as the design says", () => {
    const both = returnsAfterStart(returnsAfterStart([], "a", "hk", "v"), "b", "v", "go");
    const aEnds = revertPlan(both, "a", "go", exists, true);
    expect(aEnds).toEqual({ switchTo: null, returns: [r("b", "hk", "go")] });
    expect(revertPlan(aEnds.returns, "b", "go", exists, true)).toEqual({ switchTo: "hk", returns: [] });
    // Only A switches back: B ends without, and you stay on Glass Oracle.
    expect(revertPlan(aEnds.returns, "b", "go", exists, false)).toEqual({ switchTo: null, returns: [] });
    // Only B switches back: A drops its return unspliced; B goes back to Villager.
    const aDrops = revertPlan(both, "a", "go", exists, false);
    expect(aDrops).toEqual({ switchTo: null, returns: [r("b", "v", "go")] });
    expect(revertPlan(aDrops.returns, "b", "go", exists, true)).toEqual({ switchTo: "v", returns: [] });
  });
});

describe("pickRandomPreset (§7.4)", () => {
  const changes = (id: string) => id !== "clean";
  it.each([
    ["none to pick", ["clean", "live"], 0.5, null],
    ["one", ["live", "a"], 0, "a"],
    ["one, rand 0.999", ["a", "live"], 0.999, "a"],
    ["three, rand 0", ["a", "b", "c"], 0, "a"],
    ["three, rand 0.5", ["a", "live", "b", "clean", "c"], 0.5, "b"],
    ["three, rand 0.999", ["a", "b", "c"], 0.999, "c"],
    ["three, rand 1 held to the last", ["a", "b", "c"], 1, "c"],
  ] as const)("%s", (_name, ids, rand, want) => {
    expect(pickRandomPreset(ids, changes, "live", () => rand)).toBe(want);
  });
});

describe("restartNotice (D36)", () => {
  const stored = (onFire: unknown[], kind = "countdown") =>
    readTimers({ version: 1, timers: [{ id: "timer-1", name: "BRB", kind, durationMs: 300_000, onStart: [MUTE], onFire }] });
  const at = (phase: TimerPhase | null) => () => phase;

  it("names a muting countdown that runs or is paused, by what its zero does", () => {
    expect(restartNotice(stored([UNMUTE]), at("running"), true)).toBe(
      "BRB is running, so you're muted until it ends. Press End now or Reset to unmute.",
    );
    expect(restartNotice(stored([UNMUTE]), at("paused"), true)).toBe(
      "BRB is paused, so you stay muted. Press End now or Reset to unmute.",
    );
    expect(restartNotice(stored([]), at("running"), true)).toBe(
      "BRB muted you when it started, and nothing at zero unmutes you. Unmute from the Mixer.",
    );
  });

  it("says nothing when unmuted, ended, not run, or a count-up", () => {
    expect(restartNotice(stored([UNMUTE]), at("running"), false)).toBeNull();
    expect(restartNotice(stored([UNMUTE]), at("done"), true)).toBeNull();
    expect(restartNotice(stored([UNMUTE]), at(null), true)).toBeNull();
    expect(restartNotice(stored([UNMUTE], "countup"), at("running"), true)).toBeNull();
  });
});

describe("actionWords (§9)", () => {
  const names: ActionNames = {
    preset: (id) => (id === "villager" ? "Villager" : null),
    tile: (id) => (id === "intro" ? "Intro" : null),
    voice: (id) => (id === "babble:bright" ? "Bright" : null),
  };
  it.each([
    [{ kind: "preset", presetId: "villager" }, "Switch to Villager"],
    [{ kind: "preset", presetId: "gone" }, "Switch to Missing preset"],
    [{ kind: "randomPreset" }, "Switch to a random preset"],
    [REVERT, "Back to the preset before"],
    [MUTE, "Mute my voice"],
    [UNMUTE, "Unmute my voice"],
    [{ kind: "tile", tileId: "intro" }, "Play Intro"],
    [{ kind: "tile", tileId: "gone" }, "Play Missing clip"],
    [{ kind: "speak", text: "I'm back.", voiceId: "babble:bright" }, "Say “I'm back.” in Bright"],
    [LINE("I'm back."), "Say “I'm back.” in Voice not available"],
    [{ kind: "saveReplay" }, "Save a replay"],
    [{ kind: "panic" }, "Stop all clips and lines"],
  ] as const)("%j → %s", (a, want) => {
    expect(actionWords(a as TimerAction, names)).toBe(want);
  });
});
