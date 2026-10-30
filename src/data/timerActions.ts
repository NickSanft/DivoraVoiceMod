// v1.60.0: Timers that change your voice — the actions a countdown runs when
// it starts and when it reaches zero, for the store and Settings.
//
// Rust decides what runs (src-tauri/src/timer_actions.rs `plan`). The page
// keeps each countdown's two lists in `divora.timers` (`onStart` and
// `onFire`), sends the actions Rust would read with every `sync_timers`, and
// runs the page's half of a hook when a `timer-actions` event says so:
// preset switches and clips. The backend runs the rest: the mute, Stop all
// clips and lines, a replay, a line.
//
// Reading is lenient and goes action by action, by the rules Rust's
// `action_from_wire` and `hooks_from_wire` apply (the fixture's
// `actionSamples` and `listSamples` pin both sides): an action that can't
// be read is skipped and never sent, but it stays in storage where it is,
// so a newer Divora's action or a hand edit survives every edit made here.
// Pure, so it unit-tests.

import { isTimerId, type TimerPhase } from "../overlay/timerState";
import { ptmKeyWhere } from "./ptm";
// A cycle (timers.ts sends lists with `actionsForSync`): both sides only
// call each other's functions, never at load, so either may load first.
import { timerRoles, type StoredTimer, type StoredTimers } from "./timers";

// ---- the wire ----

/** The event that says a countdown started or ended, with the page's half
 *  of its actions. Pinned by timer.fixture.json (`actionWire`). */
export const TIMER_ACTIONS_EVENT = "timer-actions";
/** Sent (payload null) when a timer's action stops every clip and line. */
export const SOUNDBOARD_STOPPED_EVENT = "soundboard-stopped";
/** End now. */
export const FINISH_TIMER_CMD = "finish_timer";

/** The backend's limits, pinned by timer.fixture.json (`actionLimits`).
 *  Rust reads only a list's first 8 entries, so the page stores at most 8
 *  (every stored entry counts, readable or not). A line is 1-200 characters
 *  after trimming, counted by code point as Rust counts them; a clip's or a
 *  voice's id is 1-128. A preset's id has no cap: import and Duplicate make
 *  ids from names of any length. A countdown that ends more than 30 s late
 *  runs only the actions that put something back. */
export const MAX_ACTIONS = 8;
export const MAX_LINE_CHARS = 200;
export const MAX_TARGET_CHARS = 128;
export const LATE_ONE_SHOTS_MS = 30_000;

/** One action of a countdown's list (D1). `preset`, `tile` and `panic` keep
 *  the glyph actions' shape (types.ts `GlyphAction`); the glyph's toggles
 *  (`monitor`, `mute`) aren't timer actions: Mute my voice and Unmute my
 *  voice set the state they name. */
export type TimerAction =
  | { kind: "preset"; presetId: string }
  | { kind: "randomPreset" }
  | { kind: "revertPreset" }
  | { kind: "tile"; tileId: string }
  | { kind: "speak"; text: string; voiceId: string }
  | { kind: "saveReplay" }
  | { kind: "setMuted"; muted: boolean }
  | { kind: "panic" };
export type TimerActionKind = TimerAction["kind"];

/** A stored timer's two lists: At start and At zero. */
export type TimerHook = "onStart" | "onFire";

/** Why a `timer-actions` event was sent: `armed` is a start; the rest are
 *  ends — at 00:00 on time (`zero`), End now (`endNow`), more than 30 s late
 *  (`late`), ran out while the PC slept or Divora was closed (`ranOut`), a
 *  Reset or a kind change (`reset`), a delete (`deleted`). In the backend's
 *  enum order; a restart's re-mute is never sent. */
export type TimerWhy = "armed" | "zero" | "endNow" | "late" | "ranOut" | "reset" | "deleted";
export const TIMER_WHYS: readonly TimerWhy[] = ["armed", "zero", "endNow", "late", "ranOut", "reset", "deleted"];

/** Which end of a run an event is. */
export type TimerActionsHook = "start" | "end";
export const TIMER_ACTIONS_HOOKS: readonly TimerActionsHook[] = ["start", "end"];

/** `timer-actions`' payload. `actions` is the page's half only, in list
 *  order. `lateMs` is the fire's lateness for `zero` and `late`, else 0.
 *  `seq` rises by one per planned transition in the backend's process. */
export interface TimerActionsEvent {
  seq: number;
  id: string;
  hook: TimerActionsHook;
  why: TimerWhy;
  lateMs: number;
  actions: TimerAction[];
}

/** The page's record that a timer's start switched the preset, so Back to
 *  the preset before knows where to go (D17). Stored in `divora.timers`'
 *  `presetReturns`, at most one per timer. */
export interface PresetReturn {
  timer: string;
  from: string;
  to: string;
}

/** Each kind: where it runs (D5) and which list offers it (§7.2). Back to
 *  the preset before never runs at a start. In the fixture's order. */
export const ACTION_KINDS: Readonly<
  Record<TimerActionKind, { runsIn: "page" | "backend"; at: readonly ("start" | "zero")[] }>
> = {
  preset: { runsIn: "page", at: ["start", "zero"] },
  randomPreset: { runsIn: "page", at: ["start", "zero"] },
  revertPreset: { runsIn: "page", at: ["zero"] },
  tile: { runsIn: "page", at: ["start", "zero"] },
  speak: { runsIn: "backend", at: ["start", "zero"] },
  saveReplay: { runsIn: "backend", at: ["start", "zero"] },
  setMuted: { runsIn: "backend", at: ["start", "zero"] },
  panic: { runsIn: "backend", at: ["start", "zero"] },
};

/** It puts back what a start changed: Back to the preset before, Unmute my
 *  voice. Only these run when a countdown ends late or ran out, and (when
 *  they undo the start) on a Reset or a delete. */
export function restores(a: TimerAction): boolean {
  return a.kind === "revertPreset" || (a.kind === "setMuted" && !a.muted);
}

/** At start, it runs again when Divora restarts mid-run: the mute is never
 *  stored, and every launch starts unmuted. */
export function reapplies(a: TimerAction): boolean {
  return a.kind === "setMuted";
}

// ---- reading ----

function isObject(v: unknown): v is Record<string, unknown> {
  return typeof v === "object" && v !== null && !Array.isArray(v);
}

function isCount(v: unknown): v is number {
  return typeof v === "number" && Number.isSafeInteger(v) && v >= 0;
}

/** A lone surrogate: `JSON.stringify` writes it as `\udXXX`, which the
 *  backend's JSON reader refuses for the whole invoke, so every later
 *  `sync_timers` would fail. Rust never sees one, so the page refuses it. */
const LONE_SURROGATE = /\p{Cs}/u;

/** Rust's `str::trim`: Unicode White_Space at either end. JS's `trim` differs
 *  on U+FEFF (it trims it) and U+0085 (it doesn't), and a line both sides
 *  count must be trimmed the same way. */
function trimLikeRust(s: string): string {
  return s.replace(/^\p{White_Space}+|\p{White_Space}+$/gu, "");
}

function codePoints(s: string): number {
  return Array.from(s).length;
}

/** A preset's id: any non-empty string. */
function readPresetId(v: unknown): string | null {
  return typeof v === "string" && v !== "" && !LONE_SURROGATE.test(v) ? v : null;
}

/** A clip's or a voice's id: 1-128 characters. */
function readTarget(v: unknown): string | null {
  if (typeof v !== "string" || LONE_SURROGATE.test(v)) return null;
  const n = codePoints(v);
  return n >= 1 && n <= MAX_TARGET_CHARS ? v : null;
}

/** A line: trimmed, then 1-200 characters. */
function readLine(v: unknown): string | null {
  if (typeof v !== "string" || LONE_SURROGATE.test(v)) return null;
  const text = trimLikeRust(v);
  const n = codePoints(text);
  return n >= 1 && n <= MAX_LINE_CHARS ? text : null;
}

/** One stored or sent action, or null when it can't run here (§7.2): not an
 *  object; a `kind` that is missing, not a string or one this build doesn't
 *  know (the glyph's `monitor` and `mute`, v1.61.0's `obs`); a field it
 *  needs that is missing, of the wrong type or out of its range. Only the
 *  kind's own fields are kept, a line trimmed: exactly what Rust's
 *  `action_from_wire` reads. A lone surrogate in any text is refused here
 *  only (Rust never receives one). */
export function readAction(raw: unknown): TimerAction | null {
  if (!isObject(raw) || typeof raw.kind !== "string") return null;
  switch (raw.kind) {
    case "preset": {
      const presetId = readPresetId(raw.presetId);
      return presetId === null ? null : { kind: "preset", presetId };
    }
    case "tile": {
      const tileId = readTarget(raw.tileId);
      return tileId === null ? null : { kind: "tile", tileId };
    }
    case "speak": {
      const text = readLine(raw.text);
      const voiceId = readTarget(raw.voiceId);
      return text === null || voiceId === null ? null : { kind: "speak", text, voiceId };
    }
    case "setMuted":
      return typeof raw.muted === "boolean" ? { kind: "setMuted", muted: raw.muted } : null;
    case "randomPreset":
      return { kind: "randomPreset" };
    case "revertPreset":
      return { kind: "revertPreset" };
    case "saveReplay":
      return { kind: "saveReplay" };
    case "panic":
      return { kind: "panic" };
    default:
      return null;
  }
}

/** One stored entry as the card shows it. `runs`: sent and run. Otherwise
 *  why not: it can't be read here, it is a list's second line, or it is
 *  past the 8th entry (which Rust never reads, whatever it holds). */
export type ActionRow =
  | { index: number; runs: true; action: TimerAction }
  | { index: number; runs: false; action: TimerAction | null; why: "unreadable" | "secondLine" | "pastEight" };

/** Every stored entry of a list, in stored order, `index` its place in the
 *  stored array; none when the list isn't an array. Read as Rust reads a
 *  list: the first 8 entries, each on its own, and only the first readable
 *  line (an unreadable one doesn't use it up). */
export function readStoredActions(raw: unknown): ActionRow[] {
  if (!Array.isArray(raw)) return [];
  let line = false;
  return raw.map((entry: unknown, index): ActionRow => {
    const action = readAction(entry);
    if (index >= MAX_ACTIONS) return { index, runs: false, action, why: "pastEight" };
    if (action === null) return { index, runs: false, action: null, why: "unreadable" };
    if (action.kind === "speak") {
      if (line) return { index, runs: false, action, why: "secondLine" };
      line = true;
    }
    return { index, runs: true, action };
  });
}

/** What `sync_timers` sends for a list: the actions that run, in order. */
export function actionsForSync(raw: unknown): TimerAction[] {
  const out: TimerAction[] = [];
  for (const row of readStoredActions(raw)) if (row.runs) out.push(row.action);
  return out;
}

/** A `timer-actions` payload, or null when it can't be read. Actions the
 *  page can't read are left out; the store skips the backend's kinds. */
export function readTimerActionsEvent(raw: unknown): TimerActionsEvent | null {
  if (!isObject(raw)) return null;
  const { seq, id, hook, why, lateMs, actions } = raw;
  if (!isCount(seq) || !isTimerId(id) || !isCount(lateMs) || !Array.isArray(actions)) return null;
  if (!(TIMER_ACTIONS_HOOKS as readonly unknown[]).includes(hook)) return null;
  if (!(TIMER_WHYS as readonly unknown[]).includes(why)) return null;
  const read: TimerAction[] = [];
  for (const a of actions as unknown[]) {
    const action = readAction(a);
    if (action) read.push(action);
  }
  return { seq, id, hook: hook as TimerActionsHook, why: why as TimerWhy, lateMs, actions: read };
}

/** `divora.timers`' `presetReturns`: each `{timer, from, to}` whose timer
 *  is an id and whose presets are non-empty strings, the first one per
 *  timer; none when it isn't an array. What it can't read is left out
 *  here; storage keeps the key as written until the store rewrites it. */
export function readReturns(raw: unknown): PresetReturn[] {
  if (!Array.isArray(raw)) return [];
  const out: PresetReturn[] = [];
  for (const e of raw as unknown[]) {
    if (!isObject(e) || !isTimerId(e.timer) || out.some((r) => r.timer === e.timer)) continue;
    const from = readPresetId(e.from);
    const to = readPresetId(e.to);
    if (from !== null && to !== null) out.push({ timer: e.timer, from, to });
  }
  return out;
}

// ---- editing a list ----
// The card edits the stored array by index, so an entry it can't read is
// never dropped by an edit (D27).

function listOf(raw: unknown): unknown[] {
  return Array.isArray(raw) ? [...(raw as unknown[])] : [];
}

/** Whether Add action may add `kind` to a list: "full" at 8 stored entries,
 *  readable or not; "hasLine" for a second line. */
export function canAddAction(raw: unknown, kind: TimerActionKind): "ok" | "full" | "hasLine" {
  if (listOf(raw).length >= MAX_ACTIONS) return "full";
  if (kind === "speak" && readStoredActions(raw).some((r) => r.runs && r.action.kind === "speak")) return "hasLine";
  return "ok";
}

/** The list with `a` added: Stop all clips and lines at the top, since it
 *  runs first (D15); anything else at the end. A list that isn't an array
 *  starts empty. */
export function withAdded(raw: unknown, a: TimerAction): unknown[] {
  const list = listOf(raw);
  return a.kind === "panic" ? [a, ...list] : [...list, a];
}

/** The list with entry `index` replaced; unchanged when there is none. */
export function withReplaced(raw: unknown, index: number, a: TimerAction): unknown[] {
  const list = listOf(raw);
  if (Number.isInteger(index) && index >= 0 && index < list.length) list[index] = a;
  return list;
}

/** The list without entry `index`; every other entry kept in place. */
export function withRemoved(raw: unknown, index: number): unknown[] {
  return listOf(raw).filter((_, i) => i !== index);
}

/** A start whose change nothing at zero puts back, and a switch back with
 *  nothing to go back to (§7.9). Runnable rows only. */
export type PairingWarning = "muteNoUnmute" | "switchNoRevert" | "revertNoSwitch";

export function pairingWarnings(onStart: unknown, onFire: unknown): PairingWarning[] {
  const start = actionsForSync(onStart);
  const fire = actionsForSync(onFire);
  const switches = start.some((a) => a.kind === "preset" || a.kind === "randomPreset");
  const reverts = fire.some((a) => a.kind === "revertPreset");
  const out: PairingWarning[] = [];
  if (start.some((a) => a.kind === "setMuted" && a.muted) && !fire.some((a) => a.kind === "setMuted" && !a.muted)) {
    out.push("muteNoUnmute");
  }
  if (switches && !reverts) out.push("switchNoRevert");
  if (reverts && !switches) out.push("revertNoSwitch");
  return out;
}

// ---- returns (§7.3) ----

/** A start switched `timer` from `from` to `to`: a timer that already has a
 *  return keeps its `from` and takes the new `to`; else one is added. */
export function returnsAfterStart(rs: PresetReturn[], timer: string, from: string, to: string): PresetReturn[] {
  if (!rs.some((r) => r.timer === timer)) return [...rs, { timer, from, to }];
  return rs.map((r) => (r.timer === timer ? { ...r, to } : r));
}

/** An end settles `timer`'s return (§7.3). `revert`: the end runs Back to
 *  the preset before. It switches back only while the live preset is still
 *  the one the timer chose and the one before still exists; when another
 *  timer's return was stacked on top, that one inherits where to go back
 *  to; otherwise you moved on, and nothing switches. The return is then
 *  dropped. `exists` must be judged against the loaded preset list (D16). */
export function revertPlan(
  rs: PresetReturn[],
  timer: string,
  live: string,
  exists: (id: string) => boolean,
  revert: boolean,
): { switchTo: string | null; returns: PresetReturn[] } {
  const r = rs.find((x) => x.timer === timer);
  if (!r) return { switchTo: null, returns: rs };
  const rest = rs.filter((x) => x !== r);
  if (!revert) return { switchTo: null, returns: rest };
  if (live === r.to) return { switchTo: exists(r.from) ? r.from : null, returns: rest };
  const above = rest.find((o) => o.from === r.to);
  if (above) return { switchTo: null, returns: rest.map((o) => (o === above ? { ...o, from: r.from } : o)) };
  return { switchTo: null, returns: rest };
}

/** Whether a stored timer's At zero switches back: a countdown whose
 *  `onFire` has a Back to the preset before that runs. What its end, a
 *  delete and reconcile decide a leftover return by. */
export function revertsAtZero(t: Pick<StoredTimer, "kind" | "onFire">): boolean {
  return t.kind === "countdown" && actionsForSync(t.onFire).some((a) => a.kind === "revertPreset");
}

/** Settles the returns whose end no page heard (§7.6): a revert whose
 *  `timer-actions` event fell inside a reload, or a countdown that ran out
 *  while Divora was closed. The store runs it once per page, after the boot
 *  sync has answered, the listeners are up and the real preset list has
 *  loaded, so `exists` is never judged against the bundled list (D16).
 *  Each return, oldest first, by the first rule that holds:
 *  1. its timer isn't stored (deleted, its event lost) → settle with
 *     `revert` true: `deleteTimer` drops at once a return whose At zero has
 *     no Back to the preset before, so one left means "switch back";
 *  2. its timer is stored as `newer` or `over` (`timerRoles`) → drop it;
 *  3. the mirror says running or paused → keep it;
 *  4. otherwise (done, ran out, idle, or not in the mirror) → settle with
 *     `revert` = its At zero has Back to the preset before.
 *  `resetReverts` false is Q2's alternative (a Reset or a delete changes
 *  nothing else): rule 1 and an idle timer in rule 4 settle without
 *  switching. Each settle is §7.3's `revertPlan` with the live preset as
 *  the switches before it left it; only the last switch is returned. */
export function reconcileReturns(
  rs: PresetReturn[],
  stored: StoredTimers,
  phaseOf: (id: string) => TimerPhase | null,
  live: string,
  exists: (id: string) => boolean,
  resetReverts = true,
): { switchTo: string | null; returns: PresetReturn[] } {
  const roles = timerRoles(stored);
  let returns = rs;
  let now = live;
  let switchTo: string | null = null;
  for (const { timer } of rs) {
    const t = stored.timers.find((x) => x.id === timer);
    const phase = phaseOf(timer);
    let revert: boolean;
    if (!t) revert = resetReverts;
    else if (roles.get(timer) !== "run") revert = false;
    else if (phase === "running" || phase === "paused") continue;
    else revert = (resetReverts || phase !== "idle") && revertsAtZero(t);
    const settled = revertPlan(returns, timer, now, exists, revert);
    returns = settled.returns;
    if (settled.switchTo !== null) {
      now = settled.switchTo;
      switchTo = settled.switchTo;
    }
  }
  return { switchTo, returns };
}

// ---- words ----

/** The restart notice (D36): with the mute on, the first stored countdown
 *  that is running or paused and mutes at its start, named; else null.
 *  `phaseOf` is the mirror's (null for a timer the backend doesn't run). */
export function restartNotice(
  stored: StoredTimers,
  phaseOf: (id: string) => TimerPhase | null,
  muted: boolean,
): string | null {
  if (!muted) return null;
  for (const t of stored.timers) {
    const phase = phaseOf(t.id);
    if (t.kind !== "countdown" || (phase !== "running" && phase !== "paused")) continue;
    if (!actionsForSync(t.onStart).some((a) => a.kind === "setMuted" && a.muted)) continue;
    if (!actionsForSync(t.onFire).some((a) => a.kind === "setMuted" && !a.muted)) {
      return `${t.name} muted you when it started, and nothing at zero unmutes you. Unmute from the Mixer.`;
    }
    return phase === "running"
      ? `${t.name} is running, so you're muted until it ends. Press End now or Reset to unmute.`
      : `${t.name} is paused, so you stay muted. Press End now or Reset to unmute.`;
  }
  return null;
}

/** Random preset (§7.4): any preset that changes the voice, the live one
 *  excluded, in list order; null when there is none. `rand` is in [0, 1). */
export function pickRandomPreset(
  ids: readonly string[],
  changes: (id: string) => boolean,
  live: string,
  rand: () => number,
): string | null {
  const pool = ids.filter((id) => id !== live && changes(id));
  if (pool.length === 0) return null;
  const r = rand();
  const i = Number.isFinite(r) ? Math.min(pool.length - 1, Math.max(0, Math.floor(r * pool.length))) : 0;
  return pool[i] ?? null;
}

/** A target's name for a row's words, or null when it matches nothing. */
export interface ActionNames {
  preset: (id: string) => string | null;
  tile: (id: string) => string | null;
  voice: (id: string) => string | null;
}

/** A row's words (§9). A target that matches nothing reads as missing. */
export function actionWords(a: TimerAction, names: ActionNames): string {
  switch (a.kind) {
    case "preset":
      return `Switch to ${names.preset(a.presetId) ?? "Missing preset"}`;
    case "randomPreset":
      return "Switch to a random preset";
    case "revertPreset":
      return "Back to the preset before";
    case "tile":
      return `Play ${names.tile(a.tileId) ?? "Missing clip"}`;
    case "speak":
      return `Say “${a.text}” in ${names.voice(a.voiceId) ?? "Voice not available"}`;
    case "saveReplay":
      return "Save a replay";
    case "setMuted":
      return a.muted ? "Mute my voice" : "Unmute my voice";
    case "panic":
      return "Stop all clips and lines";
  }
}

/** What the call hears, for the card's line (§7.7). `key` is the string the
 *  push-to-modulate card shows; `inCall` is that card's own. */
export interface CallVoice {
  muted: boolean;
  transforms: boolean;
  bypassed: boolean;
  mode: "apply" | "bypass" | null;
  key: string;
  inCall: "key" | "midi" | null;
}

/** The card's line while a timer's switch is the live preset (§7.7, D29):
 *  which preset it is, whether it stays after the end, and, when the call
 *  doesn't hear it, why. Each fact comes on its own (UX4): the status
 *  reads Clean both for a preset that doesn't change the voice and for
 *  push-to-modulate holding the effects off, and the line tells them
 *  apart. `where` is the push-to-modulate card's own ending (data/ptm.ts
 *  `ptmKeyWhere`). No line without a return, when you moved on from the
 *  timer's preset, or for a preset with no name. */
export function timerVoiceLine(
  ret: PresetReturn | null,
  live: string,
  presetName: (id: string) => string | null,
  revertsAtEnd: boolean,
  v: CallVoice,
): string | null {
  if (ret === null || live !== ret.to) return null;
  const name = presetName(ret.to);
  if (name === null) return null;
  const head = revertsAtEnd
    ? `Your preset is ${name} until this ends.`
    : `Your preset is ${name}, and it stays after this ends.`;
  if (v.muted) return `${head} You're muted, so your call hears none of your voice.`;
  if (!v.transforms) return `${head} It doesn't change your voice.`;
  if (v.bypassed && v.mode === "apply") {
    return `${head} Push-to-modulate keeps it off your call until you hold ${v.key}${ptmKeyWhere(v.inCall)}`;
  }
  if (v.bypassed && v.mode === "bypass") {
    return `${head} Push-to-modulate is holding it off, so your call hears your own voice.`;
  }
  return head;
}

/** Add break (BRB)'s countdown (D26): "BRB" (numbered by the store when
 *  taken), 5:00, Mute my voice at the start, Unmute my voice at zero. No
 *  line and no keys (Q5). New lists each call. */
export function breakTemplate(): { name: string; durationMs: number; onStart: TimerAction[]; onFire: TimerAction[] } {
  return {
    name: "BRB",
    durationMs: 300_000,
    onStart: [{ kind: "setMuted", muted: true }],
    onFire: [{ kind: "setMuted", muted: false }],
  };
}
