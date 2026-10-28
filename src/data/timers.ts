// v1.59.0: Hourglass's pure rules, for the store and Settings.
//
// Rust keeps the time (src-tauri/src/timers.rs). The page keeps what the
// user made in `divora.timers`: each timer's definition, its hotkeys and its
// last run, with a wall-clock anchor while it runs. Every `sync_timers`
// sends the definitions and the runs; Rust uses a run only for an id it
// hasn't accepted since it started ("Rust wins while a timer runs"), so a
// restart picks up where it was and a page reload changes nothing.
//
// Reading is lenient and goes field by field, by the rules Rust's
// `entry_from_wire` applies to what the page sends: a field that can't be
// read falls back, so one hand edit never stops every sync, and unknown
// keys, kinds and hotkey verbs are kept and written back. Pure, so it
// unit-tests.

import {
  isTimerId,
  isTimerKind,
  type TimerKind,
  type TimerPhase,
  type TimerSnapshot,
  type TimerState,
} from "../overlay/timerState";
import { normalizeAccelerator } from "./hotkeys";

/** `divora.timers`' version now. A larger one is read with these rules and
 *  never lowered. */
export const TIMERS_VERSION = 1;

/** The backend's limits, pinned by timer.fixture.json. Hourglass holds up to
 *  16 timers in all; a countdown runs from 1 second to 24 hours, 5:00 when
 *  its length can't be read; a name is 1-40 characters. */
export const MAX_TIMERS = 16;
export const MIN_LENGTH_MS = 1_000;
export const MAX_LENGTH_MS = 86_400_000;
export const DEFAULT_LENGTH_MS = 300_000;
export const MAX_NAME = 40;

/** The most entries `sync_timers` reads; Rust skips the rest. */
export const MAX_SYNC_ENTRIES = 64;

/** The hotkey verbs v1.59.0 registers. A stored timer may hold others (a
 *  later build's `finish`): they are kept and never registered. */
export type TimerVerb = "toggle" | "reset";
export const TIMER_VERBS: readonly TimerVerb[] = ["toggle", "reset"];

/** A timer's last run as the page saw it, stored and sent as `resume`.
 *  Integers. A running countdown: `totalMs` and `anchorMs` (when it ends). A
 *  running count-up: `anchorMs` (when it effectively started). Paused: `ms`
 *  (and a countdown's `totalMs`). Done or ran out: `totalMs`. Idle has none. */
export interface TimerRun {
  phase: "running" | "paused" | "done" | "expired";
  ms?: number;
  totalMs?: number;
  anchorMs?: number;
}

/** One entry of `divora.timers`. `kind` is any string: a kind this build
 *  doesn't know is kept and not run. `hotkeys` holds accelerators by verb.
 *  Every other key is kept. */
export interface StoredTimer {
  id: string;
  name: string;
  kind: string;
  durationMs: number;
  hotkeys?: Record<string, string>;
  run?: TimerRun;
  [k: string]: unknown;
}

/** `divora.timers`. Unknown keys are kept. */
export interface StoredTimers {
  version: number;
  timers: StoredTimer[];
  [k: string]: unknown;
}

/** What `sync_timers` takes for each timer: validated fields only. */
export interface TimerDefWire {
  id: string;
  name: string;
  kind: string;
  durationMs: number;
  resume?: TimerRun;
}

/** "run": the backend runs it. "newer": a kind this build doesn't know.
 *  "over": a known kind past the 16th. The last two are kept, shown with
 *  Delete only, and their keys aren't registered. */
export type TimerRole = "run" | "newer" | "over";

// ---- small helpers ----

function isObject(v: unknown): v is Record<string, unknown> {
  return typeof v === "object" && v !== null && !Array.isArray(v);
}

function isCount(v: unknown): v is number {
  return typeof v === "number" && Number.isSafeInteger(v) && v >= 0;
}

const RUN_PHASES: readonly string[] = ["running", "paused", "done", "expired"];

/** The default name's first word, by kind. */
function nameBase(kind: string): string {
  return kind === "countdown" ? "Countdown" : kind === "countup" ? "Count-up" : "Timer";
}

/** `base`, or `base 2`, `base 3`… the first one not in `taken`. */
function freshName(base: string, taken: ReadonlySet<string>): string {
  if (!taken.has(base)) return base;
  let n = 2;
  while (taken.has(`${base} ${n}`)) n += 1;
  return `${base} ${n}`;
}

// ---- reading ----

/** A name as stored and sent: trimmed, 1-40 characters (counted as Rust
 *  counts them, by code point); null otherwise. */
export function readTimerName(raw: unknown): string | null {
  if (typeof raw !== "string") return null;
  const name = raw.trim();
  const chars = Array.from(name).length;
  return chars >= 1 && chars <= MAX_NAME ? name : null;
}

/** A length as stored and sent: a finite number, rounded to whole ms and
 *  held to 1 s..24 h; 5:00 when it isn't a finite number. */
export function readLength(raw: unknown): number {
  if (typeof raw !== "number" || !Number.isFinite(raw)) return DEFAULT_LENGTH_MS;
  return Math.min(MAX_LENGTH_MS, Math.max(MIN_LENGTH_MS, Math.round(raw)));
}

/** A stored or sent run, or undefined when it can't be read (the timer is
 *  then idle, and kept): a phase other than running, paused, done or ran
 *  out; `ms` or `totalMs` not a whole number of 0 or more; `anchorMs` not a
 *  whole number; a running run without `anchorMs`. Keys it doesn't know are
 *  left out. */
export function readRun(raw: unknown): TimerRun | undefined {
  if (!isObject(raw) || typeof raw.phase !== "string" || !RUN_PHASES.includes(raw.phase)) return undefined;
  const { ms, totalMs, anchorMs } = raw;
  if (ms !== undefined && !isCount(ms)) return undefined;
  if (totalMs !== undefined && !isCount(totalMs)) return undefined;
  if (anchorMs !== undefined && !(typeof anchorMs === "number" && Number.isSafeInteger(anchorMs))) return undefined;
  if (raw.phase === "running" && anchorMs === undefined) return undefined;
  const run: TimerRun = { phase: raw.phase as TimerRun["phase"] };
  if (ms !== undefined) run.ms = ms;
  if (totalMs !== undefined) run.totalMs = totalMs;
  if (anchorMs !== undefined) run.anchorMs = anchorMs as number;
  return run;
}

/** Every string value, normalized as `divora.hotkeyBindings`' are (a
 *  stored "Win" registers as "Super"); `{}` when it isn't an object. Every
 *  verb is kept. */
function readHotkeys(raw: unknown): Record<string, string> {
  const out: Record<string, string> = {};
  if (!isObject(raw)) return out;
  for (const [verb, v] of Object.entries(raw)) if (typeof v === "string") out[verb] = normalizeAccelerator(v);
  return out;
}

/** `divora.timers`, read field by field: no timers when it is missing or
 *  unreadable. An entry is dropped when it isn't an object, has no id that
 *  passes the grammar, repeats an id, or has no string `kind`. A name that
 *  isn't 1-40 characters after trimming gets a default name; a known kind's
 *  length that isn't a finite number is 5:00, any other is rounded and held
 *  to 1 s..24 h (a kind this build doesn't know keeps any finite length as
 *  it is: what it means is that kind's); a run that can't be read is
 *  dropped, and the timer is idle. Unknown keys, kinds and hotkey verbs are
 *  kept; `version` is never lowered. */
export function readTimers(raw: unknown): StoredTimers {
  if (!isObject(raw)) return { version: TIMERS_VERSION, timers: [] };
  const v = raw.version;
  const version = typeof v === "number" && Number.isSafeInteger(v) && v >= 1 ? v : TIMERS_VERSION;
  const read: Array<{ t: StoredTimer; named: boolean }> = [];
  if (Array.isArray(raw.timers)) {
    const seen = new Set<string>();
    for (const e of raw.timers as unknown[]) {
      if (!isObject(e) || !isTimerId(e.id) || seen.has(e.id) || typeof e.kind !== "string") continue;
      seen.add(e.id);
      const name = readTimerName(e.name);
      const length =
        isTimerKind(e.kind) || typeof e.durationMs !== "number" || !Number.isFinite(e.durationMs)
          ? readLength(e.durationMs)
          : e.durationMs;
      const t: StoredTimer = {
        ...e,
        id: e.id,
        name: name ?? "",
        kind: e.kind,
        durationMs: length,
        hotkeys: readHotkeys(e.hotkeys),
      };
      const run = readRun(e.run);
      if (run) t.run = run;
      else delete t.run;
      read.push({ t, named: name !== null });
    }
  }
  // Default names last, so one never takes a name a later timer has.
  const taken = new Set(read.filter((r) => r.named).map((r) => r.t.name));
  for (const r of read) {
    if (r.named) continue;
    r.t.name = freshName(nameBase(r.t.kind), taken);
    taken.add(r.t.name);
  }
  return { ...raw, version, timers: read.map((r) => r.t) };
}

/** What is written: `version` is never lowered. */
export function timersForSave(s: StoredTimers): StoredTimers {
  const v = typeof s.version === "number" && Number.isSafeInteger(s.version) ? s.version : TIMERS_VERSION;
  return { ...s, version: Math.max(v, TIMERS_VERSION) };
}

/** What `sync_timers` sends: every stored timer, in list order, kinds this
 *  build doesn't run included (so the backend keeps their files), built
 *  from validated fields only, with its run as `resume`. No other key is
 *  sent. */
export function defsForSync(s: StoredTimers): TimerDefWire[] {
  return readTimers(s).timers.map((t) => {
    const def: TimerDefWire = { id: t.id, name: t.name, kind: t.kind, durationMs: t.durationMs };
    if (t.run) def.resume = t.run;
    return def;
  });
}

/** Each stored timer's role: the first 16 of a known kind run, in list
 *  order, as the backend's sync picks them. A known kind past the 16th, or
 *  past the 64th entry (which the backend doesn't read), is "over"; a kind
 *  this build doesn't know is "newer". */
export function timerRoles(s: StoredTimers): Map<string, TimerRole> {
  const roles = new Map<string, TimerRole>();
  let running = 0;
  s.timers.forEach((t, i) => {
    if (roles.has(t.id)) return;
    if (!isTimerKind(t.kind)) {
      roles.set(t.id, "newer");
    } else if (running < MAX_TIMERS && i < MAX_SYNC_ENTRIES) {
      roles.set(t.id, "run");
      running += 1;
    } else {
      roles.set(t.id, "over");
    }
  });
  return roles;
}

/** A new timer's id: `timer-<n>`, the lowest n from 1 that no stored timer
 *  uses. Never changed afterwards: it names the timer's file and its
 *  hotkeys. */
export function nextTimerId(s: StoredTimers): string {
  const ids = new Set(s.timers.map((t) => t.id));
  let n = 1;
  while (ids.has(`timer-${n}`)) n += 1;
  return `timer-${n}`;
}

/** A new timer's name: "Countdown" (or "Count-up"), then "Countdown 2",
 *  "Countdown 3"…, the first no stored timer has. */
export function defaultName(s: StoredTimers, kind: TimerKind): string {
  return freshName(nameBase(kind), new Set(s.timers.map((t) => t.name)));
}

// ---- runs ----

/** The run the page stores for a timer in a snapshot; undefined while it is
 *  idle (a running one without an anchor reads as idle too, as Rust would
 *  seed it). */
export function runOf(t: TimerState): TimerRun | undefined {
  const countdown = t.kind === "countdown";
  if (t.phase === "running") {
    if (t.anchorMs === null) return undefined;
    return countdown
      ? { phase: "running", totalMs: t.totalMs, anchorMs: t.anchorMs }
      : { phase: "running", anchorMs: t.anchorMs };
  }
  if (t.phase === "paused") {
    return countdown ? { phase: "paused", ms: t.ms, totalMs: t.totalMs } : { phase: "paused", ms: t.ms };
  }
  if (t.phase === "done" || t.phase === "expired") return { phase: t.phase, totalMs: t.totalMs };
  return undefined;
}

/** Two runs are the same: storage is written only when one changes. */
export function sameRun(a?: TimerRun, b?: TimerRun): boolean {
  if (a === undefined || b === undefined) return a === b;
  return a.phase === b.phase && a.ms === b.ms && a.totalMs === b.totalMs && a.anchorMs === b.anchorMs;
}

// ---- hotkeys ----

const HOTKEY_PREFIX = "tm:";

/** The id a timer's key registers under: `tm:<id>:<verb>`. */
export function timerHotkeyId(id: string, verb: TimerVerb): string {
  return `${HOTKEY_PREFIX}${id}:${verb}`;
}

/** A `tm:` hotkey id → its timer and verb, or null: exactly `tm:`, an id
 *  that passes the grammar, `:` and `toggle` or `reset`, spelled exactly.
 *  The backend's `hotkey_timer_request` reads the same table, and also
 *  ignores a release; this one doesn't look at presses. */
export function parseTimerHotkeyId(s: string): { id: string; verb: TimerVerb } | null {
  if (typeof s !== "string" || !s.startsWith(HOTKEY_PREFIX)) return null;
  const parts = s.slice(HOTKEY_PREFIX.length).split(":");
  if (parts.length !== 2) return null;
  const [id, verb] = parts as [string, string];
  if (!isTimerId(id) || !(TIMER_VERBS as readonly string[]).includes(verb)) return null;
  return { id, verb: verb as TimerVerb };
}

/** Modifier names the backend's parser takes (global-hotkey 0.8.0,
 *  `hotkey.rs`), each to one spelling. "Win" and its aliases are the Super
 *  key, as `normalizeAccelerator` has it; Cmd-or-Ctrl is Ctrl on Windows. */
const MODIFIERS: Readonly<Record<string, string>> = {
  CTRL: "CTRL",
  CONTROL: "CTRL",
  COMMANDORCONTROL: "CTRL",
  COMMANDORCTRL: "CTRL",
  CMDORCTRL: "CTRL",
  CMDORCONTROL: "CTRL",
  ALT: "ALT",
  OPTION: "ALT",
  SHIFT: "SHIFT",
  SUPER: "SUPER",
  CMD: "SUPER",
  COMMAND: "SUPER",
  WIN: "SUPER",
  WINDOWS: "SUPER",
  META: "SUPER",
};

/** Key names the parser takes two ways, each to one spelling. Letters and
 *  digits (`KeyM`, `Digit1`) are handled below. */
const KEY_ALIASES: Readonly<Record<string, string>> = {
  "`": "BACKQUOTE",
  "\\": "BACKSLASH",
  "[": "BRACKETLEFT",
  "]": "BRACKETRIGHT",
  PAUSEBREAK: "PAUSE",
  ",": "COMMA",
  "=": "EQUAL",
  "-": "MINUS",
  ".": "PERIOD",
  "'": "QUOTE",
  ";": "SEMICOLON",
  "/": "SLASH",
  DOWN: "ARROWDOWN",
  LEFT: "ARROWLEFT",
  RIGHT: "ARROWRIGHT",
  UP: "ARROWUP",
  ESC: "ESCAPE",
  NUMADD: "NUMPADADD",
  NUMPADPLUS: "NUMPADADD",
  NUMPLUS: "NUMPADADD",
  NUMDECIMAL: "NUMPADDECIMAL",
  NUMDIVIDE: "NUMPADDIVIDE",
  NUMENTER: "NUMPADENTER",
  NUMEQUAL: "NUMPADEQUAL",
  NUMMULTIPLY: "NUMPADMULTIPLY",
  NUMSUBTRACT: "NUMPADSUBTRACT",
  VOLUMEDOWN: "AUDIOVOLUMEDOWN",
  VOLUMEUP: "AUDIOVOLUMEUP",
  VOLUMEMUTE: "AUDIOVOLUMEMUTE",
  MEDIATRACKPREVIOUS: "MEDIATRACKPREV",
};

function keyToken(raw: string): string {
  const u = raw.trim().toUpperCase();
  const mod = MODIFIERS[u];
  if (mod) return mod;
  const short = /^(?:KEY([A-Z])|DIGIT([0-9]))$/.exec(u);
  if (short) return (short[1] ?? short[2])!;
  const num = /^NUM([0-9])$/.exec(u);
  if (num) return `NUMPAD${num[1]}`;
  return KEY_ALIASES[u] ?? u;
}

/** Two accelerators are the same key: the same modifiers, in any order and
 *  any spelling, and the same main key ("ctrl+shift+m" = "Shift+Ctrl+M",
 *  "Super+P" = "Win+P"). An unbound key ("") is never the same as
 *  anything. */
export function sameKey(a: string, b: string): boolean {
  if (a.trim() === "" || b.trim() === "") return false;
  const set = (s: string): string => [...new Set(s.split("+").map(keyToken))].sort().join("+");
  return set(a) === set(b);
}

// ---- length ----

const ONE_SEC = 1_000;
const ONE_MIN = 60_000;
const ONE_HOUR = 3_600_000;

/** The Length field's text → ms, or null: a whole number of minutes
 *  (`5`, `90`), `m:ss` (`5:00`, `0:30`) or `h:mm:ss` (`1:30:00`), with
 *  two-digit seconds and minutes under 60, from 1 second to 24 hours.
 *  Spaces around it are ignored. */
export function parseLength(text: string): number | null {
  const t = text.trim();
  let ms: number;
  let m: RegExpExecArray | null;
  if (/^\d+$/.test(t)) {
    ms = Number(t) * ONE_MIN;
  } else if ((m = /^(\d+):([0-5]\d)$/.exec(t))) {
    ms = Number(m[1]) * ONE_MIN + Number(m[2]) * ONE_SEC;
  } else if ((m = /^(\d+):([0-5]\d):([0-5]\d)$/.exec(t))) {
    ms = Number(m[1]) * ONE_HOUR + Number(m[2]) * ONE_MIN + Number(m[3]) * ONE_SEC;
  } else {
    return null;
  }
  return Number.isFinite(ms) && ms >= MIN_LENGTH_MS && ms <= MAX_LENGTH_MS ? ms : null;
}

/** A length as the field shows it after a commit, and on the card's kind
 *  line: `m:ss` under an hour (`5:00`, `0:30`), `h:mm:ss` from an hour
 *  (`1:30:00`, `24:00:00`). Whole seconds, rounded up as an idle
 *  countdown's text is. */
export function lengthText(ms: number): string {
  const total = Number.isFinite(ms) && ms > 0 ? Math.ceil(ms / ONE_SEC) : 0;
  const h = Math.floor(total / 3600);
  const m = Math.floor((total % 3600) / 60);
  const s = total % 60;
  const ss = String(s).padStart(2, "0");
  return h > 0 ? `${h}:${String(m).padStart(2, "0")}:${ss}` : `${m}:${ss}`;
}

// ---- words ----

/** Settings' main button for a timer, by its phase: what it says and the
 *  command it sends. A labelled button sends what its label says. */
export function timerAction(t: TimerState): {
  label: "Start" | "Start again" | "Pause" | "Resume";
  cmd: "arm" | "pause" | "resume";
} {
  switch (t.phase) {
    case "running":
      return { label: "Pause", cmd: "pause" };
    case "paused":
      return { label: "Resume", cmd: "resume" };
    case "done":
    case "expired":
      return { label: "Start again", cmd: "arm" };
    default:
      return { label: "Start", cmd: "arm" };
  }
}

/** The card's status word; null (no state from the backend yet) is
 *  "Starting…". */
export function phaseWord(p: TimerPhase | null): string {
  switch (p) {
    case "idle":
      return "Ready";
    case "running":
      return "Running";
    case "paused":
      return "Paused";
    case "done":
      return "Done";
    case "expired":
      return "Ran out while away";
    default:
      return "Starting…";
  }
}

/** What the live region remembers of each timer between snapshots. */
export type TimerEdges = Map<string, { fires: number; phase: TimerPhase }>;

/** The live region's text for the next adopted snapshot, and what to
 *  remember. It speaks only edges: "{name} is done." when a timer's `fires`
 *  rose (so a fire and a re-arm in one press is heard, though no snapshot
 *  shows `done`), "{name} ran out while away." when it turned `expired`.
 *  Nothing for the first snapshot after the section mounts (`held` null) or
 *  for an id seen for the first time; a `fires` that dropped (the id was
 *  deleted and added again) is remembered, not spoken. Edges in one
 *  snapshot are joined. No edge gives "", which clears the region, so the
 *  next repeat is spoken. Never a time. */
export function timerAnnouncement(
  held: TimerEdges | null,
  next: TimerSnapshot,
): { text: string; held: TimerEdges } {
  const now: TimerEdges = new Map();
  const said: string[] = [];
  for (const t of next.timers) {
    now.set(t.id, { fires: t.fires, phase: t.phase });
    const before = held?.get(t.id);
    if (!before) continue;
    if (t.fires > before.fires) said.push(`${t.name} is done.`);
    if (t.phase === "expired" && before.phase !== "expired") said.push(`${t.name} ran out while away.`);
  }
  return { text: said.join(" "), held: now };
}
