import { describe, expect, it } from "vitest";
import fixture from "../overlay/timer.fixture.json";
import type { TimerPhase, TimerSnapshot, TimerState } from "../overlay/timerState";
import {
  DEFAULT_LENGTH_MS,
  defaultName,
  defsForSync,
  lengthText,
  MAX_LENGTH_MS,
  MAX_NAME,
  MAX_SYNC_ENTRIES,
  MAX_TIMERS,
  MIN_LENGTH_MS,
  nextTimerId,
  parseLength,
  parseTimerHotkeyId,
  phaseWord,
  readRun,
  readTimers,
  runOf,
  sameKey,
  sameRun,
  timerAction,
  timerAnnouncement,
  timerHotkeyId,
  timerRoles,
  timersForSave,
  TIMER_VERBS,
  type StoredTimer,
  type StoredTimers,
  type TimerEdges,
  type TimerRun,
  type TimerVerb,
} from "./timers";

const cd = (id: string, extra: Record<string, unknown> = {}): Record<string, unknown> => ({
  id,
  name: id.toUpperCase(),
  kind: "countdown",
  durationMs: 300_000,
  ...extra,
});

/** One entry read on its own. */
const one = (entry: Record<string, unknown>): StoredTimer | undefined =>
  readTimers({ version: 1, timers: [entry] }).timers[0];

const stored = (timers: Array<Record<string, unknown>>): StoredTimers => readTimers({ version: 1, timers });

const state = (over: Partial<TimerState> = {}): TimerState => ({
  id: "timer-1",
  name: "BRB",
  kind: "countdown",
  phase: "idle",
  ms: 300_000,
  totalMs: 300_000,
  text: "05:00",
  anchorMs: null,
  fires: 0,
  ...over,
});

describe("the limits are the fixture's (v1.59.0)", () => {
  it("matches timer.fixture.json", () => {
    // The id length is timerState.ts's, checked there.
    const { maxIdChars: _ids, ...limits } = fixture.limits;
    expect({
      maxTimers: MAX_TIMERS,
      minLengthMs: MIN_LENGTH_MS,
      maxLengthMs: MAX_LENGTH_MS,
      defaultLengthMs: DEFAULT_LENGTH_MS,
      maxName: MAX_NAME,
      maxSyncEntries: MAX_SYNC_ENTRIES,
    }).toEqual(limits);
  });
});

describe("readTimers", () => {
  it.each([
    ["missing", null],
    ["undefined", undefined],
    ["a string", "timers"],
    ["a number", 3],
    ["a list", [cd("timer-1")]],
  ])("%s → no timers, version 1", (_name, raw) => {
    expect(readTimers(raw)).toEqual({ version: 1, timers: [] });
  });

  it("no list of timers → none, and the other keys are kept", () => {
    expect(readTimers({ version: 1, timers: "x", later: { a: 1 } })).toEqual({
      version: 1,
      timers: [],
      later: { a: 1 },
    });
  });

  it("reads a good entry as it is, with an empty hotkeys object", () => {
    expect(one(cd("timer-1"))).toEqual({ ...cd("timer-1"), hotkeys: {} });
  });

  it("keeps unknown keys, at the top and in an entry", () => {
    const s = readTimers({ version: 1, theme: "x", timers: [cd("timer-1", { onFire: [{ kind: "speak" }] })] });
    expect(s.theme).toBe("x");
    expect(s.timers[0]!.onFire).toEqual([{ kind: "speak" }]);
  });

  it("keeps a kind it doesn't know, with its fields", () => {
    const t = one({ id: "timer-3", name: "Laps", kind: "interval", durationMs: 172_800_000, laps: [1, 2] });
    expect(t).toEqual({
      id: "timer-3",
      name: "Laps",
      kind: "interval",
      durationMs: 172_800_000,
      laps: [1, 2],
      hotkeys: {},
    });
  });

  it("drops entries nothing can address, and later duplicates", () => {
    const s = readTimers({
      version: 1,
      timers: [
        "timer-1",
        null,
        [cd("timer-1")],
        { id: 5, name: "x", kind: "countdown", durationMs: 1000 },
        cd("Timer-2"),
        cd("con"),
        cd("timer:2"),
        cd("timer-2", { kind: undefined }),
        cd("timer-2", { kind: 3 }),
        cd("timer-2"),
        cd("timer-2", { name: "Again" }),
      ],
    });
    expect(s.timers.map((t) => [t.id, t.name])).toEqual([["timer-2", "TIMER-2"]]);
  });

  it.each([
    ["null", null],
    ["a number", 5],
    ["empty", ""],
    ["only spaces", "   "],
    ["41 characters", "x".repeat(41)],
  ])("a name that is %s gets the default", (_name, name) => {
    expect(one(cd("timer-1", { name }))?.name).toBe("Countdown");
    expect(one(cd("timer-1", { kind: "countup", name }))?.name).toBe("Count-up");
    expect(one(cd("timer-1", { kind: "interval", name }))?.name).toBe("Timer");
  });

  it("trims a name, and counts characters as the backend does", () => {
    expect(one(cd("timer-1", { name: "  BRB  " }))?.name).toBe("BRB");
    expect(one(cd("timer-1", { name: "x".repeat(40) }))?.name).toBe("x".repeat(40));
    // 40 emoji: 80 UTF-16 units, 40 characters.
    const emoji = "\u{1F570}".repeat(40);
    expect(one(cd("timer-1", { name: emoji }))?.name).toBe(emoji);
    expect(one(cd("timer-1", { name: emoji + "\u{1F570}" }))?.name).toBe("Countdown");
  });

  it("gives default names last, so one never takes a later timer's name", () => {
    const s = stored([cd("timer-1", { name: null }), cd("timer-2", { name: "Countdown" }), cd("timer-3", { name: 7 })]);
    expect(s.timers.map((t) => t.name)).toEqual(["Countdown 2", "Countdown", "Countdown 3"]);
  });

  it.each([
    ["-1", -1, MIN_LENGTH_MS],
    ["1.5", 1.5, MIN_LENGTH_MS],
    ["0", 0, MIN_LENGTH_MS],
    ["299 999.6", 299_999.6, 300_000],
    ["over 24 hours", 90_000_000, MAX_LENGTH_MS],
    ["a string", "300000", DEFAULT_LENGTH_MS],
    ["null", null, DEFAULT_LENGTH_MS],
    ["missing", undefined, DEFAULT_LENGTH_MS],
  ])("a known kind's length of %s reads as %d", (_name, durationMs, want) => {
    expect(one(cd("timer-1", { durationMs }))?.durationMs).toBe(want);
    expect(one(cd("timer-1", { kind: "countup", durationMs }))?.durationMs).toBe(want);
  });

  it("a kind it doesn't know keeps any finite length; anything else is 5:00", () => {
    expect(one(cd("timer-1", { kind: "interval", durationMs: 1.5 }))?.durationMs).toBe(1.5);
    expect(one(cd("timer-1", { kind: "interval", durationMs: "x" }))?.durationMs).toBe(DEFAULT_LENGTH_MS);
  });

  it("keeps a run it can read, and drops one it can't (the timer stays)", () => {
    const run = { phase: "paused", ms: 154_001, totalMs: 300_000 };
    expect(one(cd("timer-1", { run }))?.run).toEqual(run);
    for (const bad of [{}, { phase: "running", anchorMs: "x" }, { phase: "overtime" }, "running", null]) {
      const t = one(cd("timer-1", { run: bad }));
      expect(t?.id).toBe("timer-1");
      expect(t && "run" in t).toBe(false);
    }
  });

  it("keeps every hotkey verb, drops values that aren't strings, and spells Win as Super", () => {
    const t = one(cd("timer-1", { hotkeys: { toggle: "Ctrl+Alt+F13", reset: "Win+F14", finish: "F15", later: 5 } }));
    expect(t?.hotkeys).toEqual({ toggle: "Ctrl+Alt+F13", reset: "Super+F14", finish: "F15" });
    expect(one(cd("timer-1", { hotkeys: "F13" }))?.hotkeys).toEqual({});
    expect(one(cd("timer-1", { hotkeys: ["F13"] }))?.hotkeys).toEqual({});
  });

  it("reads a larger version with these rules, and settles a bad one at 1", () => {
    expect(readTimers({ version: 2, timers: [cd("timer-1")] })).toEqual({
      version: 2,
      timers: [{ ...cd("timer-1"), hotkeys: {} }],
    });
    for (const version of [0, -1, 1.5, "2", null, undefined]) {
      expect(readTimers({ version, timers: [] }).version).toBe(1);
    }
  });

  // v1.60.0: the lists and the returns are read only by data/timerActions.ts;
  // storage keeps whatever is there, readable or not.
  it("keeps onStart, onFire and presetReturns as written, whatever they hold", () => {
    const raw = {
      version: 1,
      presetReturns: "x",
      timers: [
        cd("timer-1", { onStart: {}, onFire: 5 }),
        cd("timer-2", { onStart: [{ kind: "obs", op: "scene" }, { kind: "setMuted", muted: true }], onFire: null }),
      ],
    };
    const s = readTimers(raw);
    expect(s.presetReturns).toBe("x");
    expect(s.timers[0]).toMatchObject({ onStart: {}, onFire: 5 });
    expect(s.timers[1]).toMatchObject({ onStart: raw.timers[1]!.onStart, onFire: null });
    expect(readTimers(JSON.parse(JSON.stringify(timersForSave(s))))).toEqual(s);
    // Nothing unreadable is sent; the readable mute is.
    expect(defsForSync(s)).toEqual([
      { id: "timer-1", name: "TIMER-1", kind: "countdown", durationMs: 300_000 },
      {
        id: "timer-2",
        name: "TIMER-2",
        kind: "countdown",
        durationMs: 300_000,
        onStart: [{ kind: "setMuted", muted: true }],
      },
    ]);
  });

  it("keeps entries past the 64th (the backend skips them)", () => {
    const many = Array.from({ length: 70 }, (_, i) => cd(`timer-${i + 1}`));
    expect(stored(many).timers).toHaveLength(70);
  });
});

describe("timersForSave", () => {
  it("never lowers the version", () => {
    expect(timersForSave({ version: 3, timers: [] }).version).toBe(3);
    expect(timersForSave({ version: 0, timers: [] }).version).toBe(1);
    expect(timersForSave({ version: Number.NaN, timers: [] }).version).toBe(1);
  });

  it("round-trips through storage unchanged", () => {
    const raw = {
      version: 2,
      later: [1],
      presetReturns: [{ timer: "timer-1", from: "hollow-king", to: "villager" }, { timer: 5 }],
      timers: [
        cd("timer-1", {
          hotkeys: { toggle: "F13", finish: "F15" },
          run: { phase: "running", totalMs: 300_000, anchorMs: 5 },
          onStart: [{ kind: "preset", presetId: "villager" }, { kind: "obs", op: "scene" }],
          onFire: [{ kind: "revertPreset" }],
        }),
        { id: "timer-2", name: "Laps", kind: "interval", durationMs: 9, laps: [1] },
        cd("timer-3", { kind: "countup", run: { phase: "paused", ms: 754_321 } }),
      ],
    };
    const once = readTimers(raw);
    const again = readTimers(JSON.parse(JSON.stringify(timersForSave(once))));
    expect(again).toEqual(once);
  });
});

describe("defsForSync", () => {
  it("sends every stored timer in list order, validated fields only, with its run as resume", () => {
    const s = stored([
      cd("timer-2", { colour: "red", hotkeys: { toggle: "F13" }, run: { phase: "paused", ms: 1, totalMs: 300_000 } }),
      { id: "timer-7", name: "Laps", kind: "interval", durationMs: 9, laps: [1] },
      cd("timer-1", { kind: "countup" }),
    ]);
    expect(defsForSync(s)).toEqual([
      {
        id: "timer-2",
        name: "TIMER-2",
        kind: "countdown",
        durationMs: 300_000,
        resume: { phase: "paused", ms: 1, totalMs: 300_000 },
      },
      { id: "timer-7", name: "Laps", kind: "interval", durationMs: 9 },
      { id: "timer-1", name: "TIMER-1", kind: "countup", durationMs: 300_000 },
    ]);
  });

  it("checks the fields again, whatever the store holds", () => {
    const s: StoredTimers = {
      version: 1,
      timers: [
        { id: "timer-1", name: "  BRB ", kind: "countdown", durationMs: -4, run: { phase: "running" } as TimerRun },
        { id: "Bad Id", name: "x", kind: "countdown", durationMs: 1000 },
      ],
    };
    expect(defsForSync(s)).toEqual([{ id: "timer-1", name: "BRB", kind: "countdown", durationMs: MIN_LENGTH_MS }]);
  });

  it("is empty with no timers", () => {
    expect(defsForSync(readTimers(null))).toEqual([]);
  });

  // The seam with the backend's `entry_from_wire`, which reads these keys:
  // timers.rs `sync_entry_keys_are_the_fixtures` reads the same list.
  // v1.60.0: `onStart` and `onFire` go, like `resume`, only when there is
  // something in them, and only for a countdown.
  it("sends exactly the fixture's defKeys: resume only with a run, lists only for a countdown with some", () => {
    const lists = {
      onStart: [{ kind: "setMuted", muted: true }],
      onFire: [{ kind: "preset", presetId: "x" }],
    };
    const [all, without, countup, empty] = defsForSync(
      stored([
        cd("timer-1", { run: { phase: "paused", ms: 5_000 }, colour: "red", ...lists }),
        cd("timer-2"),
        cd("timer-3", { kind: "countup", ...lists }),
        cd("timer-4", { onStart: [], onFire: [{ kind: "mute" }] }),
      ]),
    );
    expect(Object.keys(all!).sort()).toEqual(fixture.defKeys);
    expect(all).toMatchObject(lists);
    const optional = ["onFire", "onStart", "resume"];
    for (const def of [without, countup, empty]) {
      expect(Object.keys(def!).sort()).toEqual(fixture.defKeys.filter((k) => !optional.includes(k)));
    }
  });

  // The seam with the backend's `hooks_from_wire`: the same rows, read by
  // timers.rs (`hooks_from_wire_reads_eight`) and here.
  it("sends each fixture listSample's lists as the backend reads them", () => {
    for (const row of fixture.listSamples) {
      const [def] = defsForSync(stored([row.entry]));
      const sent: Record<string, unknown> = {};
      if (def?.onStart) sent.onStart = def.onStart;
      if (def?.onFire) sent.onFire = def.onFire;
      expect(sent, row.name).toEqual(row.sent);
    }
  });
});

describe("timerRoles", () => {
  it("runs the first 16 of a known kind; a newer kind and the 17th known are kept, not run", () => {
    const timers = [
      { id: "timer-1", name: "Laps", kind: "interval", durationMs: 1 },
      ...Array.from({ length: 17 }, (_, i) => cd(`timer-${i + 2}`, { kind: i % 2 ? "countup" : "countdown" })),
    ];
    const roles = timerRoles(stored(timers));
    expect(roles.get("timer-1")).toBe("newer");
    for (let n = 2; n <= 17; n += 1) expect(roles.get(`timer-${n}`)).toBe("run");
    expect(roles.get("timer-18")).toBe("over");
    expect(roles.size).toBe(18);
  });

  it("a known kind past the 64th entry is over: the backend never reads it", () => {
    const timers = [
      ...Array.from({ length: 64 }, (_, i) => ({ id: `timer-${i + 1}`, name: "x", kind: "interval", durationMs: 1 })),
      cd("timer-65"),
    ];
    expect(timerRoles(stored(timers)).get("timer-65")).toBe("over");
  });

  it("is empty with no timers", () => {
    expect(timerRoles(readTimers(null)).size).toBe(0);
  });
});

describe("nextTimerId and defaultName", () => {
  it("picks the lowest free timer-<n>, reusing gaps", () => {
    expect(nextTimerId(readTimers(null))).toBe("timer-1");
    expect(nextTimerId(stored([cd("timer-1"), cd("timer-2")]))).toBe("timer-3");
    expect(nextTimerId(stored([cd("timer-2"), cd("timer-3")]))).toBe("timer-1");
    expect(nextTimerId(stored([cd("timer-1"), cd("brb"), cd("timer-3")]))).toBe("timer-2");
  });

  it("names a new timer by its kind, numbering repeats", () => {
    expect(defaultName(readTimers(null), "countdown")).toBe("Countdown");
    expect(defaultName(readTimers(null), "countup")).toBe("Count-up");
    const s = stored([cd("timer-1", { name: "Countdown" }), cd("timer-2", { name: "Countdown 3" })]);
    expect(defaultName(s, "countdown")).toBe("Countdown 2");
    expect(defaultName(s, "countup")).toBe("Count-up");
  });

  it("every id it picks passes the grammar, and the parser reads it back", () => {
    let s = readTimers(null);
    for (let i = 0; i < MAX_TIMERS; i += 1) {
      const id = nextTimerId(s);
      expect(parseTimerHotkeyId(timerHotkeyId(id, "toggle"))).toEqual({ id, verb: "toggle" });
      s = stored([...s.timers, cd(id)]);
    }
  });
});

describe("runs", () => {
  it("runOf gives each sample's stored run, and readRun reads it back as it is", () => {
    for (const r of fixture.runSamples) {
      const run = runOf(r.state as TimerState);
      expect(run, r.name).toEqual(r.run);
      expect(readRun(run), r.name).toEqual(r.run);
      expect(Object.keys(run!).every((k) => fixture.runKeys.includes(k)), r.name).toBe(true);
    }
  });

  it("an idle timer has no run, and nor does a running one without an anchor", () => {
    for (const t of fixture.samples.idle.timers) expect(runOf(t as TimerState)).toBeUndefined();
    expect(runOf(state({ phase: "running", anchorMs: null }))).toBeUndefined();
  });

  it("the samples' snapshots store the runs their rows say", () => {
    expect(runOf(fixture.samples.running.timers[0] as TimerState)).toEqual(fixture.runSamples[0]!.run);
    expect(runOf(fixture.samples.done.timers[0] as TimerState)).toEqual({ phase: "done", totalMs: 300_000 });
  });

  it.each([
    ["running with an anchor", { phase: "running", anchorMs: 5 }, { phase: "running", anchorMs: 5 }],
    ["a negative anchor", { phase: "running", anchorMs: -5 }, { phase: "running", anchorMs: -5 }],
    ["done", { phase: "done", totalMs: 1000 }, { phase: "done", totalMs: 1000 }],
    ["unknown keys left out", { phase: "expired", totalMs: 0, x: 1 }, { phase: "expired", totalMs: 0 }],
    ["running without an anchor", { phase: "running", totalMs: 1000 }, undefined],
    ["idle (not a run)", { phase: "idle" }, undefined],
    ["an unknown phase", { phase: "overtime", ms: 1 }, undefined],
    ["no phase", { ms: 1 }, undefined],
    ["ms negative", { phase: "paused", ms: -1 }, undefined],
    ["ms fractional", { phase: "paused", ms: 1.5 }, undefined],
    ["ms a string", { phase: "paused", ms: "1" }, undefined],
    ["ms null", { phase: "paused", ms: null }, undefined],
    ["totalMs fractional", { phase: "done", totalMs: 0.5 }, undefined],
    ["anchorMs a string", { phase: "running", anchorMs: "x" }, undefined],
    ["anchorMs fractional", { phase: "running", anchorMs: 1.5 }, undefined],
    ["not an object", "running", undefined],
    ["a list", [{ phase: "done" }], undefined],
  ])("readRun: %s", (_name, raw, want) => {
    expect(readRun(raw)).toEqual(want);
  });

  it("sameRun compares every field", () => {
    const a: TimerRun = { phase: "running", totalMs: 300_000, anchorMs: 5 };
    expect(sameRun(undefined, undefined)).toBe(true);
    expect(sameRun(a, undefined)).toBe(false);
    expect(sameRun(undefined, a)).toBe(false);
    expect(sameRun(a, { ...a })).toBe(true);
    expect(sameRun(a, { ...a, anchorMs: 6 })).toBe(false);
    expect(sameRun(a, { ...a, totalMs: 330_000 })).toBe(false);
    expect(sameRun(a, { ...a, phase: "paused" })).toBe(false);
    expect(sameRun({ phase: "paused", ms: 1 }, { phase: "paused", ms: 2 })).toBe(false);
  });
});

describe("timer hotkey ids", () => {
  it("parses every fixture row as the backend does for a press", () => {
    for (const row of fixture.hotkeyIds) {
      const want = row.timer === null ? null : { id: row.timer, verb: row.verb };
      expect(parseTimerHotkeyId(row.id), row.id).toEqual(want);
      if (row.timer !== null) expect(timerHotkeyId(row.timer, row.verb as TimerVerb)).toBe(row.id);
    }
  });

  it("registers three verbs since v1.60.0: start or pause, reset and End now", () => {
    expect(TIMER_VERBS).toEqual(["toggle", "reset", "finish"]);
    expect(parseTimerHotkeyId("tm:timer-1:finish")).toEqual({ id: "timer-1", verb: "finish" });
    for (const id of ["tm:timer-1:Finish", "tm:timer-1:finish ", "tm:timer-1:end"]) {
      expect(parseTimerHotkeyId(id), id).toBeNull();
    }
  });
});

describe("parseLength and lengthText", () => {
  // §7.9.
  it.each([
    ["5", 300_000],
    ["5:00", 300_000],
    ["0:30", 30_000],
    ["1:30:00", 5_400_000],
    ["90", 5_400_000],
    ["24:00:00", 86_400_000],
    [" 7 ", 420_000],
    ["0:01", 1_000],
    ["59:59", 3_599_000],
    ["1440", 86_400_000],
  ])("%j → %d", (text, ms) => {
    expect(parseLength(text)).toBe(ms);
  });

  it.each(["0", "0:00", "0:60", "24:00:01", "1:2", "5m", "-5", "1.5", "", "   ", "1441", "1:60:00", "1::00", "a"])(
    "%j → null",
    (text) => {
      expect(parseLength(text)).toBeNull();
    },
  );

  it.each([
    [300_000, "5:00"],
    [30_000, "0:30"],
    [5_400_000, "1:30:00"],
    [86_400_000, "24:00:00"],
    [3_599_000, "59:59"],
    [3_600_000, "1:00:00"],
    [1_000, "0:01"],
    [1_500, "0:02"],
  ])("lengthText(%d) = %j", (ms, text) => {
    expect(lengthText(ms)).toBe(text);
  });

  it("every whole second from 1 s to 24 h survives the round trip", () => {
    for (let s = 1; s <= 86_400; s += 1) {
      const ms = s * 1000;
      if (parseLength(lengthText(ms)) !== ms) throw new Error(`${ms} → ${lengthText(ms)}`);
    }
  });
});

describe("sameKey", () => {
  it.each([
    ["ctrl+shift+m", "Shift+Ctrl+M"],
    ["Super+P", "Win+P"],
    ["Ctrl+Super+P", "Windows+Control+P"],
    ["Alt+KeyA", "Option+A"],
    ["Ctrl+Digit1", "Ctrl+1"],
    ["Ctrl+Up", "Ctrl+ArrowUp"],
    ["CmdOrCtrl+F13", "Ctrl+F13"],
    ["F13", "f13"],
    [" Ctrl + F9 ", "Ctrl+F9"],
  ])("%j is %j", (a, b) => {
    expect(sameKey(a, b)).toBe(true);
    expect(sameKey(b, a)).toBe(true);
  });

  it.each([
    ["Ctrl+M", "Ctrl+Shift+M"],
    ["Ctrl+M", "Alt+M"],
    ["F9", "F10"],
    ["Shift+F9", "F9"],
    ["", ""],
    ["F9", ""],
  ])("%j is not %j", (a, b) => {
    expect(sameKey(a, b)).toBe(false);
  });
});

describe("timerAction and phaseWord", () => {
  // §7.11.
  it.each([
    ["idle", "Start", "arm"],
    ["running", "Pause", "pause"],
    ["paused", "Resume", "resume"],
    ["done", "Start again", "arm"],
    ["expired", "Start again", "arm"],
  ] as const)("%s → %s, %s", (phase, label, cmd) => {
    expect(timerAction(state({ phase }))).toEqual({ label, cmd });
  });

  it("says each phase in a word, and Starting… before any state", () => {
    expect(fixture.phases.map((p) => phaseWord(p as TimerPhase))).toEqual([
      "Ready",
      "Running",
      "Paused",
      "Done",
      "Ran out while away",
    ]);
    expect(phaseWord(null)).toBe("Starting…");
  });
});

describe("timerAnnouncement", () => {
  const snap = (...timers: TimerState[]): TimerSnapshot => ({ seq: 1, timers });
  const brb = (fires: number, phase: TimerPhase): TimerState => state({ fires, phase });
  const intro = (fires: number, phase: TimerPhase): TimerState => state({ id: "timer-2", name: "Intro", fires, phase });
  const after = (held: TimerEdges | null, ...steps: TimerSnapshot[]): string[] => {
    const said: string[] = [];
    let h = held;
    for (const s of steps) {
      const a = timerAnnouncement(h, s);
      said.push(a.text);
      h = a.held;
    }
    return said;
  };

  // §7.12.
  it("says nothing for the first snapshot after the section mounts, and remembers it", () => {
    const a = timerAnnouncement(null, snap(brb(1, "done")));
    expect(a.text).toBe("");
    expect(a.held.get("timer-1")).toEqual({ fires: 1, phase: "done" });
  });

  it("says done when fires rises, even with no done in between", () => {
    expect(after(null, snap(brb(0, "running")), snap(brb(1, "done")))).toEqual(["", "BRB is done."]);
    expect(after(null, snap(brb(1, "done")), snap(brb(2, "running")))).toEqual(["", "BRB is done."]);
  });

  it("says ran out on the edge into expired, once", () => {
    expect(after(null, snap(brb(0, "running")), snap(brb(0, "expired")), snap(brb(0, "expired")))).toEqual([
      "",
      "BRB ran out while away.",
      "",
    ]);
  });

  it("says nothing for an id seen for the first time, or for a fires that dropped", () => {
    expect(after(null, snap(brb(0, "running")), snap(brb(0, "running"), intro(1, "done")))).toEqual(["", ""]);
    const said = after(null, snap(brb(3, "done")), snap(brb(0, "idle")), snap(brb(1, "done")));
    expect(said).toEqual(["", "", "BRB is done."]);
  });

  it("joins edges in one snapshot, then clears, so a repeat is spoken again", () => {
    const said = after(
      null,
      snap(brb(0, "running"), intro(0, "running")),
      snap(brb(1, "done"), intro(0, "expired")),
      snap(brb(1, "done"), intro(0, "expired")),
      snap(brb(2, "done"), intro(0, "expired")),
    );
    expect(said).toEqual(["", "BRB is done. Intro ran out while away.", "", "BRB is done."]);
  });

  it("forgets an id that left the snapshot", () => {
    const held: TimerEdges = new Map([["timer-9", { fires: 1, phase: "done" }]]);
    const a = timerAnnouncement(held, snap(brb(0, "idle")));
    expect([...a.held.keys()]).toEqual(["timer-1"]);
  });

  it("never says a time", () => {
    const said = after(null, snap(brb(0, "running")), snap(brb(1, "done")), snap(brb(1, "expired")));
    for (const text of said) expect(text).not.toMatch(/\d+:\d\d/);
  });
});
