import { describe, expect, it } from "vitest";
import fixture from "../overlay/timer.fixture.json";
import type { TimerPhase } from "../overlay/timerState";
import {
  ACTION_KINDS,
  actionsForSync,
  actionWords,
  breakTemplate,
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
  reconcileReturns,
  restartNotice,
  restores,
  returnsAfterStart,
  revertPlan,
  revertsAtZero,
  SOUNDBOARD_STOPPED_EVENT,
  TIMER_ACTIONS_EVENT,
  TIMER_ACTIONS_HOOKS,
  TIMER_WHYS,
  timerVoiceLine,
  withAdded,
  withRemoved,
  withReplaced,
  type ActionNames,
  type CallVoice,
  type PresetReturn,
  type TimerAction,
} from "./timerActions";
import { defsForSync, readTimers } from "./timers";

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

describe("revertsAtZero", () => {
  it("is a countdown whose At zero has a Back to the preset before that runs", () => {
    expect(revertsAtZero({ kind: "countdown", onFire: [UNMUTE, REVERT] })).toBe(true);
    expect(revertsAtZero({ kind: "countdown", onFire: [UNMUTE] })).toBe(false);
    // A count-up never reaches zero; the 9th entry is never read; a list
    // that isn't one is empty.
    expect(revertsAtZero({ kind: "countup", onFire: [REVERT] })).toBe(false);
    expect(revertsAtZero({ kind: "countdown", onFire: [...Array(8).fill(OBS), REVERT] })).toBe(false);
    expect(revertsAtZero({ kind: "countdown", onFire: { kind: "revertPreset" } })).toBe(false);
  });
});

describe("reconcileReturns (§7.6)", () => {
  const r = (timer: string, from: string, to: string): PresetReturn => ({ timer, from, to });
  const VILLAGER: TimerAction = { kind: "preset", presetId: "villager" };
  // timer-1 switches back at zero, timer-2 doesn't; timer-3 is a newer
  // kind; timer-4 is the 17th countdown, so it is over the limit.
  const fillers = Array.from({ length: 14 }, (_, i) => ({
    id: `timer-${i + 10}`,
    name: `T${i + 10}`,
    kind: "countdown",
    durationMs: 60_000,
  }));
  const stored = readTimers({
    version: 1,
    timers: [
      { id: "timer-1", name: "Villager", kind: "countdown", durationMs: 300_000, onStart: [VILLAGER], onFire: [REVERT] },
      { id: "timer-2", name: "One way", kind: "countdown", durationMs: 300_000, onStart: [VILLAGER], onFire: [] },
      { id: "timer-3", name: "Later", kind: "interval", durationMs: 1, onFire: [REVERT] },
      ...fillers,
      { id: "timer-4", name: "Over", kind: "countdown", durationMs: 300_000, onFire: [REVERT] },
    ],
  });
  const exists = (id: string) => id !== "gone";
  const phases = (m: Record<string, TimerPhase>) => (id: string) => m[id] ?? null;

  it.each([
    ["1. deleted: switches back", "timer-9", null, "villager", "hollow-king", []],
    ["1. deleted, you moved on: dropped", "timer-9", null, "clean", null, []],
    ["2. a newer kind: dropped, no switch", "timer-3", null, "villager", null, []],
    ["2. over the limit: dropped, no switch", "timer-4", null, "villager", null, []],
    ["3. running: kept", "timer-1", "running", "villager", null, [r("timer-1", "hollow-king", "villager")]],
    ["3. paused: kept", "timer-1", "paused", "villager", null, [r("timer-1", "hollow-king", "villager")]],
    ["4. done, switching back at zero", "timer-1", "done", "villager", "hollow-king", []],
    ["4. ran out, switching back at zero", "timer-1", "expired", "villager", "hollow-king", []],
    ["4. idle (a lost Reset), switching back at zero", "timer-1", "idle", "villager", "hollow-king", []],
    ["4. stored as run but not in the mirror: by its At zero", "timer-1", null, "villager", "hollow-king", []],
    ["4. done, no switch back at zero: dropped", "timer-2", "done", "villager", null, []],
    ["4. idle, no switch back at zero: dropped", "timer-2", "idle", "villager", null, []],
  ] as const)("%s", (_name, timer, phase, live, switchTo, returns) => {
    const m: Record<string, TimerPhase> = phase === null ? {} : { [timer]: phase };
    const rs = [r(timer, "hollow-king", "villager")];
    expect(reconcileReturns(rs, stored, phases(m), live, exists, true)).toEqual({ switchTo, returns });
  });

  it("a deleted timer's return whose preset before is gone switches nothing and goes", () => {
    expect(reconcileReturns([r("timer-9", "gone", "villager")], stored, phases({}), "villager", exists)).toEqual({
      switchTo: null,
      returns: [],
    });
  });

  it("Q2's alternative: a deleted or idle timer's return goes without switching", () => {
    const rs = [r("timer-9", "hollow-king", "villager")];
    expect(reconcileReturns(rs, stored, phases({}), "villager", exists, false)).toEqual({ switchTo: null, returns: [] });
    const idle = [r("timer-1", "hollow-king", "villager")];
    expect(reconcileReturns(idle, stored, phases({ "timer-1": "idle" }), "villager", exists, false)).toEqual({
      switchTo: null,
      returns: [],
    });
    // A run that really ended still switches back.
    expect(reconcileReturns(idle, stored, phases({ "timer-1": "done" }), "villager", exists, false)).toEqual({
      switchTo: "hollow-king",
      returns: [],
    });
  });

  it("goes oldest first, splicing into a return stacked on top, and keeps a running one", () => {
    // Hollow King →(timer-9, deleted) Villager →(timer-1, running) Glass
    // Oracle: the deleted one hands Hollow King to the running one.
    const rs = [r("timer-9", "hollow-king", "villager"), r("timer-1", "villager", "glass-oracle")];
    expect(reconcileReturns(rs, stored, phases({ "timer-1": "running" }), "glass-oracle", exists)).toEqual({
      switchTo: null,
      returns: [r("timer-1", "hollow-king", "glass-oracle")],
    });
  });

  it("judges each settle against the preset the switches before it left live, and sends only the last", () => {
    // timer-9's switch back makes Hollow King live, which is timer-1's: it
    // then switches back to Clean. Judged against Villager, timer-1's would
    // read as "you moved on".
    const rs = [r("timer-9", "hollow-king", "villager"), r("timer-1", "clean", "hollow-king")];
    expect(reconcileReturns(rs, stored, phases({ "timer-1": "done" }), "villager", exists)).toEqual({
      switchTo: "clean",
      returns: [],
    });
  });

  it("with no returns, changes nothing", () => {
    expect(reconcileReturns([], stored, phases({}), "villager", exists)).toEqual({ switchTo: null, returns: [] });
  });
});

describe("timerVoiceLine (§7.7)", () => {
  const ret: PresetReturn = { timer: "timer-1", from: "hollow-king", to: "villager" };
  const name = (id: string) => (id === "villager" ? "Villager" : null);
  const heard: CallVoice = { muted: false, transforms: true, bypassed: false, mode: null, key: "Space", inCall: null };
  const HEAD = "Your preset is Villager until this ends.";
  const line = (v: Partial<CallVoice>, revertsAtEnd = true) =>
    timerVoiceLine(ret, "villager", name, revertsAtEnd, { ...heard, ...v });

  it("says nothing without a return, once you moved on, or for a preset with no name", () => {
    expect(timerVoiceLine(null, "villager", name, true, heard)).toBeNull();
    expect(timerVoiceLine(ret, "clean", name, true, heard)).toBeNull();
    expect(timerVoiceLine({ ...ret, to: "gone" }, "gone", name, true, heard)).toBeNull();
  });

  it("names the preset, and says when it stays after the end", () => {
    expect(line({})).toBe(HEAD);
    expect(line({}, false)).toBe("Your preset is Villager, and it stays after this ends.");
    expect(line({ muted: true }, false)).toBe(
      "Your preset is Villager, and it stays after this ends. You're muted, so your call hears none of your voice.",
    );
  });

  it("the mute comes first: your call hears none of your voice", () => {
    expect(line({ muted: true })).toBe(`${HEAD} You're muted, so your call hears none of your voice.`);
    expect(line({ muted: true, transforms: false, bypassed: true, mode: "apply" })).toBe(
      `${HEAD} You're muted, so your call hears none of your voice.`,
    );
  });

  // The status reads Clean both ways; the line tells them apart (UX4).
  it.each([
    ["off", null, false],
    ["Hold to apply, not held", "apply", true],
    ["Hold to apply, held", "apply", false],
    ["Hold to bypass, held", "bypass", true],
    ["Hold to bypass, not held", "bypass", false],
  ] as const)("a preset that doesn't change the voice says so, push-to-modulate %s", (_name, mode, bypassed) => {
    expect(line({ transforms: false, mode, bypassed })).toBe(`${HEAD} It doesn't change your voice.`);
  });

  it.each([
    ["key", "."],
    ["midi", " in Divora's window, when no control in it has the keyboard, or with your MIDI control."],
    [
      null,
      " in Divora's window, when no control in it has the keyboard. To use it in a call, set a key in Settings → Hotkeys.",
    ],
  ] as const)("Hold to apply, not held: until you hold the key, where push-to-modulate's card says (%s)", (inCall, where) => {
    expect(line({ bypassed: true, mode: "apply", key: "F13", inCall })).toBe(
      `${HEAD} Push-to-modulate keeps it off your call until you hold F13${where}`,
    );
  });

  it("Hold to bypass, held: your call hears your own voice", () => {
    expect(line({ bypassed: true, mode: "bypass" })).toBe(
      `${HEAD} Push-to-modulate is holding it off, so your call hears your own voice.`,
    );
  });

  it("push-to-modulate letting the effects through says only the head", () => {
    expect(line({ bypassed: false, mode: "apply" })).toBe(HEAD);
    expect(line({ bypassed: false, mode: "bypass" })).toBe(HEAD);
  });
});

describe("breakTemplate (D26)", () => {
  it("is BRB, 5:00, muting at the start and unmuting at zero, and syncs as both lists", () => {
    const b = breakTemplate();
    expect(b).toEqual({
      name: "BRB",
      durationMs: 300_000,
      onStart: [{ kind: "setMuted", muted: true }],
      onFire: [{ kind: "setMuted", muted: false }],
    });
    expect(pairingWarnings(b.onStart, b.onFire)).toEqual([]);
    const s = readTimers({ version: 1, timers: [{ id: "timer-1", kind: "countdown", hotkeys: {}, ...b }] });
    expect(defsForSync(s)).toEqual([
      { id: "timer-1", name: "BRB", kind: "countdown", durationMs: 300_000, onStart: [MUTE], onFire: [UNMUTE] },
    ]);
    // Each call's lists are its own.
    expect(breakTemplate().onStart).not.toBe(b.onStart);
  });
});
