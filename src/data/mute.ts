// v1.56.0: the voice mute's pure rules, for the store and the screens.
//
// The engine owns the mute: one word, `seq << 1 | muted`, where every change
// bumps `seq`. The store keeps a read-only mirror of it. Command answers, the
// boot read and the ~30 Hz level ticks can land in any order, so the mirror
// keeps whichever state has the largest `seq`. Pure, so it unit-tests.

import type { InputMute, LevelUpdate } from "../audio/api";

/** A CC at or above this is "up" (mute). */
export const MUTE_CC_UP = 64;
/** A CC at or below this is "down" (unmute). 60..63 keeps the side it was on. */
export const MUTE_CC_DOWN = 59;

/** A change count: a whole number, 0 or more. */
function isSeq(v: unknown): v is number {
  return typeof v === "number" && Number.isSafeInteger(v) && v >= 0;
}

/** A command's answer → a state, or null when it isn't one (a mock's null,
 *  a missing field, a negative or fractional seq). */
export function readInputMute(raw: unknown): InputMute | null {
  if (typeof raw !== "object" || raw === null) return null;
  const { muted, seq } = raw as Record<string, unknown>;
  if (typeof muted !== "boolean" || !isSeq(seq)) return null;
  return { muted, seq };
}

/** A tick's pair → a state, or null when either field is absent or wrong.
 *  Absent is "no information", never "unmuted". */
export function levelMute(update: Partial<LevelUpdate>): InputMute | null {
  if (typeof update !== "object" || update === null) return null;
  const { inputMuted, inputMuteSeq } = update;
  if (typeof inputMuted !== "boolean" || !isSeq(inputMuteSeq)) return null;
  return { muted: inputMuted, seq: inputMuteSeq };
}

/** What the mirror holds next: the incoming state only if it is newer. */
export function adoptMute(
  held: InputMute | null,
  incoming: InputMute | null,
): InputMute | null {
  if (incoming === null) return held;
  if (held === null || incoming.seq > held.seq) return incoming;
  return held;
}

/** The titlebar's live region: "" until the first mute this page has seen. */
export function muteAnnouncement(muted: boolean, everMuted: boolean): string {
  if (muted) return "Your voice is muted.";
  return everMuted ? "Your voice is unmuted." : "";
}

/** A CC on a `mute` mapping. `up` is the side it is on: ≥ 64 up, ≤ 59 down,
 *  60..63 keeps `prevUp`. `act` is the mute to set, or null: only a change of
 *  side acts, and while the side is unknown (prevUp undefined: after load, or
 *  once the store forgets it on a MIDI input pick or a mapping change) a
 *  message acts only when it is up. So a control resting on one side never
 *  changes the mute, except that it mutes once if it is up while its side is
 *  unknown. */
export function muteCcStep(
  prevUp: boolean | undefined,
  value: number,
): { up: boolean | undefined; act: boolean | null } {
  const up = value >= MUTE_CC_UP ? true : value <= MUTE_CC_DOWN ? false : prevUp;
  if (up === undefined || up === prevUp) return { up, act: null };
  // First message after load: down does nothing (it may just be resting).
  if (prevUp === undefined && !up) return { up, act: null };
  return { up, act: up };
}
