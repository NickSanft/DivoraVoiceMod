// v1.59.0: Hourglass's timers as the backend reports them.
//
// Rust keeps the time (src-tauri/src/timers.rs) and sends the whole set on
// `timer-state` after every change; every timer command answers the same
// shape. Main's store and the overlay page both read it from here, and each
// keeps the snapshot with the largest `seq`: an answer can land after the
// events its command caused. The overlay page never imports
// src/audio/api.ts, so the event and the one command it calls are named
// here too. Pure: no DOM, no Tauri. The names, keys, phases, kinds and the
// id grammar are frozen in timer.fixture.json, which the backend's tests
// read as well.

export type TimerKind = "countdown" | "countup";

/** `done`: a countdown reached zero while the PC was awake and Divora ran,
 *  and fired (late, if Divora was held up). `expired`: it reached zero while
 *  the PC slept or Divora was closed, and didn't fire. A count-up is only
 *  ever idle, running or paused. */
export type TimerPhase = "idle" | "running" | "paused" | "done" | "expired";

/** One timer the backend runs. Its keys are frozen in timer.fixture.json. */
export interface TimerState {
  id: string;
  name: string;
  kind: TimerKind;
  phase: TimerPhase;
  /** A countdown's time left (rounded up to whole ms), or a count-up's time
   *  counted (rounded down), at the snapshot. Always agrees with `text`. */
  ms: number;
  /** A countdown's current total (its length ± adjusts since it started; it
   *  can pass 24 hours); 0 for a count-up. */
  totalMs: number;
  /** What the timer shows everywhere: `05:00`, `1:02:03`. */
  text: string;
  /** A running countdown: the wall-clock ms it ends. A running count-up: the
   *  wall-clock ms it effectively started. Otherwise null. */
  anchorMs: number | null;
  /** How often this timer has fired since its id was added. */
  fires: number;
}

/** The whole set. Of two snapshots, the larger `seq` is newer. Only timers
 *  the backend runs are in it. */
export interface TimerSnapshot {
  seq: number;
  timers: TimerState[];
}

export const TIMER_STATE_EVENT = "timer-state";
export const TIMER_STATUS_CMD = "timer_status";

export const TIMER_KINDS: readonly TimerKind[] = ["countdown", "countup"];
export const TIMER_PHASES: readonly TimerPhase[] = ["idle", "running", "paused", "done", "expired"];

/** The longest timer id. */
export const MAX_TIMER_ID_CHARS = 32;

/** 1-32 of `a-z`, `0-9` and `-`, starting and ending with a letter or digit.
 *  No `:` (the id sits between two in `tm:<id>:<verb>`), no `.`, `/` or `\`
 *  (it names `timers/<id>.txt`), no upper case (NTFS ignores case). */
const TIMER_ID_RE = /^[a-z0-9](?:[a-z0-9-]{0,30}[a-z0-9])?$/;
/** Windows' reserved device names: `con.txt` can't be a file. */
const DEVICE_NAME_RE = /^(?:con|prn|aux|nul|com[0-9]|lpt[0-9])$/;

/** A timer id: the backend's `is_timer_id`, from the same table. */
export function isTimerId(id: unknown): id is string {
  return typeof id === "string" && TIMER_ID_RE.test(id) && !DEVICE_NAME_RE.test(id);
}

export function isTimerKind(kind: unknown): kind is TimerKind {
  return typeof kind === "string" && (TIMER_KINDS as readonly string[]).includes(kind);
}

function isTimerPhase(phase: unknown): phase is TimerPhase {
  return typeof phase === "string" && (TIMER_PHASES as readonly string[]).includes(phase);
}

/** A count: a whole number, 0 or more. */
function isCount(v: unknown): v is number {
  return typeof v === "number" && Number.isSafeInteger(v) && v >= 0;
}

function readState(raw: unknown): TimerState | null {
  if (typeof raw !== "object" || raw === null || Array.isArray(raw)) return null;
  const o = raw as Record<string, unknown>;
  if (!isTimerId(o.id) || typeof o.name !== "string" || typeof o.text !== "string") return null;
  if (!isTimerKind(o.kind) || !isTimerPhase(o.phase)) return null;
  if (!isCount(o.ms) || !isCount(o.totalMs) || !isCount(o.fires)) return null;
  const anchor = o.anchorMs;
  if (anchor !== null && !(typeof anchor === "number" && Number.isSafeInteger(anchor))) return null;
  return {
    id: o.id,
    name: o.name,
    kind: o.kind,
    phase: o.phase,
    ms: o.ms,
    totalMs: o.totalMs,
    text: o.text,
    anchorMs: anchor,
    fires: o.fires,
  };
}

/** A snapshot off the wire, or null when it isn't one (the e2e mock's
 *  `null`, a missing or negative `seq`, `timers` not a list). A timer entry
 *  with a key missing or of the wrong type, a phase or kind this build
 *  doesn't know, or an id already seen is dropped; the rest are kept. Keys
 *  a later backend adds are left out. */
export function readTimerSnapshot(raw: unknown): TimerSnapshot | null {
  if (typeof raw !== "object" || raw === null || Array.isArray(raw)) return null;
  const { seq, timers } = raw as Record<string, unknown>;
  if (!isCount(seq) || !Array.isArray(timers)) return null;
  const seen = new Set<string>();
  const out: TimerState[] = [];
  for (const t of timers as unknown[]) {
    const state = readState(t);
    if (state === null || seen.has(state.id)) continue;
    seen.add(state.id);
    out.push(state);
  }
  return { seq, timers: out };
}

/** What the holder keeps next: the incoming snapshot only if it is newer. */
export function adoptTimers(held: TimerSnapshot | null, incoming: TimerSnapshot | null): TimerSnapshot | null {
  if (incoming === null) return held;
  if (held === null || incoming.seq > held.seq) return incoming;
  return held;
}

/** The width of one character of Space Mono, in em, with a little room. */
const MONO_ADVANCE_EM = 0.62;
/** The outline and some air, both sides together, in em. */
const SIDE_EM = 0.4;
/** The font size's share of the box's height. */
const HEIGHT_SHARE = 0.78;

/** The face's font size in CSS px: as large as the box allows for `chars`
 *  characters of Space Mono on one line, `min(0.78·h, w / (0.62·chars +
 *  0.4))`. Sized in JS, so `59:59` turning into `1:00:00` shrinks the text
 *  instead of reflowing a locked box. 0 for an empty box. */
export function fitTimerFont(w: number, h: number, chars: number): number {
  if (!(Number.isFinite(w) && w > 0 && Number.isFinite(h) && h > 0)) return 0;
  const n = Number.isFinite(chars) && chars > 0 ? Math.floor(chars) : 0;
  return Math.min(HEIGHT_SHARE * h, w / (MONO_ADVANCE_EM * n + SIDE_EM));
}
