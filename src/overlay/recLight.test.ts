import { readFileSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import type {
  ReplayState as ApiReplayState,
  ReplayStatus,
  ScreenRecordingState,
  ScreenRecordingUpdate,
} from "../audio/api";
import { formatClock } from "../data/screenRecording";
import {
  fmtElapsed,
  LIGHT_RED,
  type LightCtx,
  readReplayView,
  readTakeView,
  REC_COMMANDS,
  REC_EVENTS,
  recFace,
  type RecFace,
  REPLAY_FACE,
  REPLAY_STATES,
  type ReplayState,
  type ReplayView,
  TAKE_STATES,
  type TakeState,
  type TakeView,
} from "./recLight";
import fixture from "./wire.fixture.json";

// wire.fixture.json → `recLight` names the recorder's events and commands
// the overlay page uses (v1.62.0). The backend's tests read the same rows:
// overlay_machine.rs `fixture_names_the_recorder_events_the_overlay_hears`
// and lib.rs `the_overlay_recorder_commands_are_registered`.

describe("the recording light's wire (v1.62.0)", () => {
  it("names and samples are the fixture's", () => {
    expect(REC_EVENTS).toEqual(fixture.recLight.events);
    expect(REC_COMMANDS).toEqual(fixture.recLight.commands);
    // Each sample is Rust's payload cut to the fields the light keeps, and
    // overlay_machine.rs checks it against Rust's own serialization. Its
    // keys are the views' fields, no more and no fewer (the typed objects
    // fail typecheck if a view gains, loses or renames one).
    const take: Record<keyof TakeView, true> = { state: true, elapsedMs: true };
    const replay: Record<keyof ReplayView, true> = { state: true };
    expect(Object.keys(fixture.recLight.samples.take).sort()).toEqual(Object.keys(take).sort());
    expect(Object.keys(fixture.recLight.samples.replay).sort()).toEqual(Object.keys(replay).sort());
  });

  it("command and event names are api.ts's", () => {
    // The page never imports api.ts, so the names are kept twice; read its
    // source rather than import it.
    const api = readFileSync(join(process.cwd(), "src", "audio", "api.ts"), "utf8");
    for (const name of [...Object.values(REC_EVENTS), ...Object.values(REC_COMMANDS)]) {
      expect(api, name).toContain(`("${name}"`);
    }
  });

  it("state names are api.ts's", () => {
    // A state Rust adds and api.ts learns but this file doesn't would read
    // as junk, and the light would stop following the recorder. The types
    // are compared at compile time (each line fails typecheck unless the two
    // unions are equal), the lists against the types.
    type Same<A, B> = [A] extends [B] ? ([B] extends [A] ? true : false) : false;
    const takeSame: Same<TakeState, ScreenRecordingState> = true;
    const replaySame: Same<ReplayState, ApiReplayState> = true;
    expect(takeSame && replaySame).toBe(true);
    const takes: Record<TakeState, true> = { idle: true, starting: true, recording: true, stopping: true };
    const replays: Record<ReplayState, true> = { off: true, arming: true, armed: true, held: true };
    expect([...TAKE_STATES].sort()).toEqual(Object.keys(takes).sort());
    expect([...REPLAY_STATES].sort()).toEqual(Object.keys(replays).sort());
  });

  it("the dot is the Red swatch", () => {
    expect(fixture.textCharm.colors.find((c) => c.name === "Red")?.hex).toBe(LIGHT_RED);
  });

  it("recLight.ts imports nothing", () => {
    // The overlay page imports it.
    const source = readFileSync(join(process.cwd(), "src", "overlay", "recLight.ts"), "utf8");
    expect(source).not.toMatch(/^\s*import\b/m);
  });
});

describe("readers", () => {
  it("read the fixture's samples as they are", () => {
    expect(readTakeView(fixture.recLight.samples.take)).toEqual(fixture.recLight.samples.take);
    expect(readReplayView(fixture.recLight.samples.replay)).toEqual(fixture.recLight.samples.replay);
  });

  it("keep only the fields the light needs", () => {
    // The rest of a real payload is never read. (`satisfies` keeps each key
    // one api.ts has, with its type.)
    const take = {
      state: "recording",
      elapsedMs: 65_000,
      path: "C:\\Users\\me\\Videos\\Divora\\divora-screen.mp4",
      droppedFrames: 3,
      driftPpm: 12.5,
      error: null,
      desktopDoubled: false,
    } satisfies Partial<ScreenRecordingUpdate>;
    expect(readTakeView(take)).toEqual({ state: "recording", elapsedMs: 65_000 });
    const replay = {
      state: "armed",
      supported: true,
      seconds: 30,
      bufferedMs: 12_000,
      heldReason: null,
      monitorId: "m1",
    } satisfies Partial<ReplayStatus>;
    expect(readReplayView(replay)).toEqual({ state: "armed" });
    expect(readReplayView({ state: "held", heldReason: "engine-stopped" })).toEqual({ state: "held" });
  });

  it("read every state as itself", () => {
    for (const state of TAKE_STATES) expect(readTakeView({ state, elapsedMs: 1_000 })).toEqual({ state, elapsedMs: 1_000 });
    for (const state of REPLAY_STATES) expect(readReplayView({ state })).toEqual({ state });
  });

  it.each([
    ["null (the e2e mock's answer)", null],
    ["undefined", undefined],
    ["a number", 3],
    ["a string", "recording"],
    ["a list", [fixture.recLight.samples.take]],
    ["no state", { elapsedMs: 65_000 }],
    ["a number for a state", { state: 5 }],
    ["a state this build doesn't know", { state: "paused" }],
    ["a state in the wrong case", { state: "Recording" }],
    ["a null state", { state: null }],
  ])("a take report that is %s → null", (_name, raw) => {
    expect(readTakeView(raw)).toBeNull();
  });

  it.each([
    ["null (the e2e mock's answer)", null],
    ["a list", [fixture.recLight.samples.replay]],
    ["no state", { heldReason: null }],
    ["a number for a state", { state: 5 }],
    ["a state this build doesn't know", { state: "paused" }],
    ["a take's state", { state: "recording" }],
  ])("a Replay report that is %s → null", (_name, raw) => {
    expect(readReplayView(raw)).toBeNull();
  });

  it.each([
    ["NaN", Number.NaN],
    ["Infinity", Number.POSITIVE_INFINITY],
    ["-Infinity", Number.NEGATIVE_INFINITY],
    ["a string", "65000"],
    ["null", null],
    ["missing", undefined],
  ])("a take's elapsedMs that is %s reads 0", (_name, elapsedMs) => {
    const raw: Record<string, unknown> = { state: "recording" };
    if (elapsedMs !== undefined) raw.elapsedMs = elapsedMs;
    expect(readTakeView(raw)).toEqual({ state: "recording", elapsedMs: 0 });
  });
});

const T = (state: TakeState, elapsedMs = 0): TakeView => ({ state, elapsedMs });
const R = (state: ReplayState): ReplayView => ({ state });
const NONE: RecFace = { kind: "none", text: "" };
const IDLE: RecFace = { kind: "idle", text: "REC" };
const REPLAY: RecFace = { kind: "replay", text: "REPLAY" };
const rec = (text: string): RecFace => ({ kind: "rec", text });

const ctx = (over: Partial<LightCtx>): LightCtx => ({
  arrange: false,
  statusSeen: true,
  hideGate: false,
  capturing: false,
  ...over,
});
/** Locked, with the page's own status, Hide off: the plain face. */
const PLAIN = ctx({});
/** Locked, with Hide on, while Windows is asked to leave the overlay out. */
const ASKED = ctx({ hideGate: true, capturing: true });

// Every pair of views the light can hold: none yet, or each state.
const TAKES: Array<TakeView | null> = [null, ...TAKE_STATES.map((s) => T(s, 65_000))];
const REPLAYS: Array<ReplayView | null> = [null, ...REPLAY_STATES.map(R)];
const PAIRS = TAKES.flatMap((t) => REPLAYS.map((r) => [t, r] as const));
// Every context: each of the four flags on and off.
const CTXS: LightCtx[] = [false, true].flatMap((arrange) =>
  [false, true].flatMap((statusSeen) =>
    [false, true].flatMap((hideGate) =>
      [false, true].map((capturing) => ({ arrange, statusSeen, hideGate, capturing })),
    ),
  ),
);

describe("recFace", () => {
  // The design's first table (§7.6): the face from the two views, as drawn
  // Locked with the page's own status and Hide off.
  it.each<[string, TakeView | null, ReplayView | null, RecFace]>([
    ["no reports yet", null, null, NONE],
    ["idle, no Replay report", T("idle"), null, NONE],
    ["idle, Replay off", T("idle"), R("off"), NONE],
    ["idle, Replay held (it takes in nothing new)", T("idle"), R("held"), NONE],
    ["no take report, Replay held", null, R("held"), NONE],
    ["idle, Replay arming", T("idle"), R("arming"), REPLAY],
    ["idle, Replay armed", T("idle"), R("armed"), REPLAY],
    ["no take report, Replay armed", null, R("armed"), REPLAY],
    ["starting", T("starting"), null, rec("REC")],
    ["starting, Replay armed", T("starting"), R("armed"), rec("REC")],
    ["recording 0", T("recording", 0), R("off"), rec("REC 0:00")],
    ["recording 65 000", T("recording", 65_000), R("off"), rec("REC 1:05")],
    ["recording 65 000, Replay armed (a take outranks Replay)", T("recording", 65_000), R("armed"), rec("REC 1:05")],
    ["recording 65 000, Replay arming", T("recording", 65_000), R("arming"), rec("REC 1:05")],
    ["recording 65 000, Replay held", T("recording", 65_000), R("held"), rec("REC 1:05")],
    ["recording an hour", T("recording", 3_600_000), null, rec("REC 1:00:00")],
    ["stopping 65 000 (the last time)", T("stopping", 65_000), R("off"), rec("REC 1:05")],
    ["stopping 65 000, Replay armed", T("stopping", 65_000), R("armed"), rec("REC 1:05")],
  ])("face table: %s", (_name, take, replay, face) => {
    expect(REPLAY_FACE).toBe(true);
    expect(recFace(take, replay, PLAIN)).toEqual(face);
    // Hide off: whether something records doesn't matter.
    expect(recFace(take, replay, ctx({ capturing: true }))).toEqual(face);
    // Arrange draws the same face, and the dimmed placeholder for none.
    expect(recFace(take, replay, ctx({ arrange: true }))).toEqual(face.kind === "none" ? IDLE : face);
  });

  // The design's worked rows for the gate (§7.6), Locked.
  it.each<[string, TakeView | null, ReplayView | null, LightCtx, RecFace]>([
    ["starting, before the page's own status", T("starting"), R("off"), ctx({ statusSeen: false }), NONE],
    ["starting, Hide on, not capturing yet", T("starting"), R("off"), ctx({ hideGate: true }), NONE],
    ["starting, Hide on, asked now", T("starting"), R("off"), ASKED, rec("REC")],
    ["recording 65 000, Hide on, asked", T("recording", 65_000), R("off"), ASKED, rec("REC 1:05")],
    ["stopping, Hide on, still asked (Stop: erase)", T("stopping", 65_000), R("off"), ASKED, NONE],
    ["stopping, Replay armed, Hide on, asked", T("stopping", 65_000), R("armed"), ASKED, REPLAY],
    ["idle, Replay armed, Hide on, asked", T("idle"), R("armed"), ASKED, REPLAY],
    ["idle, Replay held, Hide on, asked", T("idle"), R("held"), ASKED, NONE],
    ["recording, Hide on, a status that says not capturing", T("recording", 65_000), R("off"), ctx({ hideGate: true }), NONE],
    ["recording 65 000, Hide off", T("recording", 65_000), R("off"), PLAIN, rec("REC 1:05")],
  ])("gate table: %s", (_name, take, replay, c, face) => {
    expect(recFace(take, replay, c)).toEqual(face);
  });

  it("gate table: Arrange draws the face whatever the gate", () => {
    // Arrange is never left out of captures, so the gate doesn't apply: a
    // starting take shows while Hide waits for `capturing`, and a stopping
    // take keeps its time.
    const arrange = ctx({ arrange: true, hideGate: true, capturing: false });
    expect(recFace(T("starting"), R("off"), arrange)).toEqual(rec("REC"));
    expect(recFace(T("stopping", 65_000), R("off"), ctx({ arrange: true, hideGate: true, capturing: true }))).toEqual(
      rec("REC 1:05"),
    );
    expect(recFace(null, null, arrange)).toEqual(IDLE);
    for (const c of CTXS.filter((x) => x.arrange)) {
      for (const [t, r] of PAIRS) {
        const plain = recFace(t, r, PLAIN);
        expect(recFace(t, r, c)).toEqual(plain.kind === "none" ? IDLE : plain);
      }
    }
  });

  it("gate table: Locked draws nothing before the page's own status", () => {
    for (const c of CTXS.filter((x) => !x.arrange && !x.statusSeen)) {
      for (const [t, r] of PAIRS) expect(recFace(t, r, c)).toEqual(NONE);
    }
  });

  it("gate table: Locked with Hide off draws the face", () => {
    for (const capturing of [false, true]) {
      for (const [t, r] of PAIRS) expect(recFace(t, r, ctx({ capturing }))).toEqual(recFace(t, r, PLAIN));
    }
  });

  it("gate table: Locked with Hide on draws nothing until Windows is asked", () => {
    for (const [t, r] of PAIRS) expect(recFace(t, r, ctx({ hideGate: true, capturing: false }))).toEqual(NONE);
  });

  it("gate table: Locked with Hide on and asked draws the face, a stopping take read as idle", () => {
    for (const [t, r] of PAIRS) {
      const asIdle = t !== null && t.state === "stopping" ? T("idle") : t;
      expect(recFace(t, r, ASKED)).toEqual(recFace(asIdle, r, PLAIN));
    }
  });

  it("every face's words are §9.4's", () => {
    // "REC", "REC m:ss" or "REC h:mm:ss", "REPLAY"; the placeholder "REC";
    // nothing for none. The dot is drawn beside the text, never in it.
    for (const c of CTXS) {
      for (const [t, r] of PAIRS) {
        const f = recFace(t, r, c);
        if (f.kind === "none") expect(f.text).toBe("");
        else if (f.kind === "idle") expect(f.text).toBe("REC");
        else if (f.kind === "replay") expect(f.text).toBe("REPLAY");
        else expect(f.text).toMatch(/^REC(?: \d+:\d\d(?::\d\d)?)?$/);
        expect(f.kind === "idle").toBe(c.arrange && recFace(t, r, PLAIN).kind === "none");
      }
    }
  });
});

describe("fmtElapsed", () => {
  // The design's table (§7.6): whole seconds, rounded down.
  it.each<[number, string]>([
    [0, "0:00"],
    [999, "0:00"],
    [-1, "0:00"],
    [-65_000, "0:00"],
    [Number.NaN, "0:00"],
    [Number.POSITIVE_INFINITY, "0:00"],
    [Number.NEGATIVE_INFINITY, "0:00"],
    [1_000, "0:01"],
    [1_999, "0:01"],
    [59_999, "0:59"],
    [60_000, "1:00"],
    [65_000, "1:05"],
    [3_599_999, "59:59"],
    [3_600_000, "1:00:00"],
    [3_723_000, "1:02:03"],
    [36_000_000, "10:00:00"],
  ])("fmtElapsed table: %d → %s", (ms, text) => {
    expect(fmtElapsed(ms)).toBe(text);
  });

  it("shows the second the Mixer's clock shows", () => {
    // Main's "Recording · m:ss" (MixerScreen.tsx) reads the same reports.
    for (let ms = -2_000; ms <= 4_000_000; ms += 997) expect(fmtElapsed(ms), String(ms)).toBe(formatClock(ms));
    for (const ms of [0, 999, 1_000, 59_999, 60_000, 3_599_999, 3_600_000, 86_400_000]) {
      expect(fmtElapsed(ms), String(ms)).toBe(formatClock(ms));
    }
  });
});
