// v1.62.0 (Text charms): the recording light's face.
//
// The overlay page hears the recorder's own broadcast events itself
// (`screen-recording` for a take, `replay-buffer` for Replay), never
// through main, and reads both statuses once after its ready handshake;
// an answer counts only if no event of its kind came while it was on its
// way. The page never imports src/audio/api.ts (timerState.ts says why),
// so the event and command names are kept here too. The readers keep only
// the fields the face needs: a take's `state` and `elapsedMs`, Replay's
// `state`. No page timer: the time is the take's own report, about once a
// second. Pure, and imports nothing. The names and a sample of each
// payload are frozen in wire.fixture.json → `recLight`, which the
// backend's tests read too.
//
// What it draws: a take in any phase but idle shows "REC", with its time
// from `recording` on; else Replay arming or armed shows "REPLAY"; else
// nothing while Locked and a dimmed "REC" in Arrange. Locked, it draws
// nothing until the page holds its own `overlay:status`, and with Hide
// from recordings on it draws only while that status says Windows is asked
// to leave the overlay out (`capturing`), a stopping take counting as no
// take.
//
// Why the gate is `hideGate && capturing`: Divora asks Windows to leave the
// overlay out exactly while it is shown and Locked, Hide is wanted and
// available, and something records (overlay_machine.rs `protected()`); a
// window that isn't shown yet has nothing on screen to capture. "Asked",
// never "hidden": Divora can't read back whether a capture path honours
// the request. v1.64.0's window takes make `capturing` stop implying the
// request; the status's `hideApplies` then takes the place of
// `hideGate && capturing`, and the tables keep their shape.

/** The recorder's events the overlay page listens to. */
export const REC_EVENTS = Object.freeze({ take: "screen-recording", replay: "replay-buffer" } as const);

/** The recorder's status commands the overlay page calls once after its
 *  ready handshake. */
export const REC_COMMANDS = Object.freeze({ take: "screen_recording_status", replay: "replay_buffer_status" } as const);

/** The dot's colour: the Red swatch, clear of OBS's key presets. */
export const LIGHT_RED = "#ff4500";

/** The kill switch: false drops the "REPLAY" face, so the light shows takes
 *  only. */
export const REPLAY_FACE = true;

/** A take's phase, as the backend reports it. */
export type TakeState = "idle" | "starting" | "recording" | "stopping";

/** Replay's state, as the backend reports it. */
export type ReplayState = "off" | "arming" | "armed" | "held";

/** Every take phase, in the order a take goes through them. */
export const TAKE_STATES: readonly TakeState[] = ["idle", "starting", "recording", "stopping"];

/** Every Replay state. */
export const REPLAY_STATES: readonly ReplayState[] = ["off", "arming", "armed", "held"];

/** What the light keeps of a take's report. */
export interface TakeView {
  state: TakeState;
  /** The take's time, in ms; 0 for anything that isn't a finite number. */
  elapsedMs: number;
}

/** What the light keeps of Replay's report. */
export interface ReplayView {
  state: ReplayState;
}

/** What the face depends on besides the two views. `statusSeen`: the page
 *  holds an `overlay:status` for its own instance. `hideGate`: that
 *  status's `hideWanted && hideAvailable`. `capturing`: that status's
 *  `capturing`. */
export interface LightCtx {
  arrange: boolean;
  statusSeen: boolean;
  hideGate: boolean;
  capturing: boolean;
}

/** What the light draws. `none`: nothing. `idle`: the dimmed Arrange
 *  placeholder. `rec` and `replay`: the face, with its `text`. The dot is
 *  drawn beside `text`, never part of it. */
export interface RecFace {
  kind: "none" | "idle" | "rec" | "replay";
  text: string;
}

function isOneOf<T extends string>(list: readonly T[], v: unknown): v is T {
  return typeof v === "string" && (list as readonly string[]).includes(v);
}

/** A plain object off the wire, or null. */
function fields(raw: unknown): Record<string, unknown> | null {
  return typeof raw === "object" && raw !== null && !Array.isArray(raw) ? (raw as Record<string, unknown>) : null;
}

/** A take's report off the wire (an event or the command's answer), or null
 *  when it isn't one: not an object, or a `state` this build doesn't know.
 *  Only `state` and `elapsedMs` are read; `path` and the rest never are. */
export function readTakeView(raw: unknown): TakeView | null {
  const o = fields(raw);
  if (o === null || !isOneOf(TAKE_STATES, o.state)) return null;
  const ms = o.elapsedMs;
  return { state: o.state, elapsedMs: typeof ms === "number" && Number.isFinite(ms) ? ms : 0 };
}

/** Replay's report off the wire, or null when it isn't one; only `state` is
 *  read (a held ring's reason goes out as `heldReason`, which the light
 *  doesn't need). */
export function readReplayView(raw: unknown): ReplayView | null {
  const o = fields(raw);
  if (o === null || !isOneOf(REPLAY_STATES, o.state)) return null;
  return { state: o.state };
}

const NOTHING: RecFace = Object.freeze({ kind: "none", text: "" });

/** The face from the two views alone. A take outranks
 *  Replay, because it is what is being written; `stopping` keeps the take's
 *  last time. Held means the ring takes in nothing new, so it shows
 *  nothing. */
function plainFace(take: TakeView | null, replay: ReplayView | null): RecFace {
  if (take !== null && take.state === "starting") return { kind: "rec", text: "REC" };
  if (take !== null && (take.state === "recording" || take.state === "stopping")) {
    return { kind: "rec", text: `REC ${fmtElapsed(take.elapsedMs)}` };
  }
  if (REPLAY_FACE && replay !== null && (replay.state === "arming" || replay.state === "armed")) {
    return { kind: "replay", text: "REPLAY" };
  }
  return NOTHING;
}

/** What the light draws now, from the two views (null before the first of
 *  each) and `ctx`.
 *
 *  - Arrange: the face, or the dimmed "REC" placeholder for none. The gate
 *    doesn't apply: Arrange is never left out of captures, and the page's
 *    hint says recordings can see the overlay.
 *  - Locked, before the page's own status: nothing. The page starts from a
 *    closed status (Locked, Hide off) that no event has confirmed, and the
 *    status the handshake causes can land after a take's answer.
 *  - Locked, Hide off or unavailable: the face.
 *  - Locked, Hide on: nothing unless the status says `capturing` (a take
 *    reports `starting` before Windows is asked); then the face with a
 *    stopping take read as idle, so the take's part goes at Stop while the
 *    overlay is still asked to be left out. */
export function recFace(take: TakeView | null, replay: ReplayView | null, ctx: LightCtx): RecFace {
  if (ctx.arrange) {
    const face = plainFace(take, replay);
    return face.kind === "none" ? { kind: "idle", text: "REC" } : face;
  }
  if (!ctx.statusSeen) return NOTHING;
  if (!ctx.hideGate) return plainFace(take, replay);
  if (!ctx.capturing) return NOTHING;
  return plainFace(take !== null && take.state === "stopping" ? null : take, replay);
}

/** A take's time as the face shows it: `m:ss`, and `h:mm:ss` from an hour,
 *  in whole seconds rounded down; `0:00` for anything below a second or not
 *  a finite number. For every finite number it is the Mixer's clock
 *  (`formatClock`, src/data/screenRecording.ts), which this file can't
 *  import, so both windows show the same second. */
export function fmtElapsed(ms: number): string {
  const total = Number.isFinite(ms) && ms > 0 ? Math.floor(ms / 1000) : 0;
  const h = Math.floor(total / 3600);
  const m = Math.floor((total % 3600) / 60);
  const ss = String(total % 60).padStart(2, "0");
  return h > 0 ? `${h}:${String(m).padStart(2, "0")}:${ss}` : `${m}:${ss}`;
}
