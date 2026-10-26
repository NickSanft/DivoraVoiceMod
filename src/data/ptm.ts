// v1.58.0: push-to-modulate's pure rules, for the store, the shell and the
// Mixer card.
//
// The engine owns push-to-modulate: one word, `seq << 16 | mode << 8 | held`,
// with a held bit per source (the global key, the in-app key, the Mixer's
// Hold to switch button, MIDI), where every change bumps `seq`. The store
// keeps a read-only mirror of it. Command answers, the boot restore and the
// ~30 Hz level ticks can land in any order, so the mirror keeps whichever
// state has the largest `seq`, as the mute's does (`data/mute.ts`).
//
// What the user picked (on or off, and which way the hold goes) is saved in
// `divora.ptm`, only from an engine answer, so the page never shows a mode
// the engine doesn't have. Pure, so it unit-tests.

import type { LevelUpdate, PtmHold, PtmMode, PtmState } from "../audio/api";
import type { ChainEntry } from "../types";

/** `divora.ptm`: whether push-to-modulate is on, and which way the hold goes
 *  (kept while it is off, so turning it back on restores it). */
export type PtmSetting = { on: boolean; hold: PtmHold };

/** Off, Hold to apply: what an absent or malformed `divora.ptm` reads as. */
export function defaultPtmSetting(): PtmSetting {
  return { on: false, hold: "apply" };
}

const MODES: readonly PtmMode[] = ["always", "apply", "bypass"];

function isMode(v: unknown): v is PtmMode {
  return typeof v === "string" && (MODES as readonly string[]).includes(v);
}

function isHold(v: unknown): v is PtmHold {
  return v === "apply" || v === "bypass";
}

/** A change count: a whole number, 0 or more. */
function isSeq(v: unknown): v is number {
  return typeof v === "number" && Number.isSafeInteger(v) && v >= 0;
}

/** A command's answer → a state, or null when it isn't one (a mock's null,
 *  a missing field, a mode the wire doesn't have, a negative or fractional
 *  seq). */
export function readPtmState(raw: unknown): PtmState | null {
  if (typeof raw !== "object" || raw === null) return null;
  const { mode, held, bypassed, seq } = raw as Record<string, unknown>;
  if (!isMode(mode) || typeof held !== "boolean" || typeof bypassed !== "boolean") return null;
  if (!isSeq(seq)) return null;
  return { mode, held, bypassed, seq };
}

/** A tick's `ptm` → a state, or null when it is absent or wrong. Absent is
 *  "no information", never "always". */
export function levelPtm(update: Partial<LevelUpdate>): PtmState | null {
  if (typeof update !== "object" || update === null) return null;
  return readPtmState(update.ptm);
}

/** What the mirror holds next: the incoming state only if it is newer. */
export function adoptPtm(held: PtmState | null, incoming: PtmState | null): PtmState | null {
  if (incoming === null) return held;
  if (held === null || incoming.seq > held.seq) return incoming;
  return held;
}

/** `divora.ptm` → a setting. Anything malformed (not an object, `on` not a
 *  boolean, `hold` not "apply" or "bypass") reads as off, Hold to apply. */
export function readPtmSetting(raw: unknown): PtmSetting {
  if (typeof raw !== "object" || raw === null || Array.isArray(raw)) return defaultPtmSetting();
  const { on, hold } = raw as Record<string, unknown>;
  if (typeof on !== "boolean" || !isHold(hold)) return defaultPtmSetting();
  return { on, hold };
}

/** The mode a setting asks the engine for: its hold while on, "always" while
 *  off. */
export function ptmModeOf(s: PtmSetting): PtmMode {
  return s.on ? s.hold : "always";
}

/** An answer's mode → the setting to save. "always" is off and keeps
 *  `prev.hold`, so the card's choice survives turning it off. */
export function settingOf(mode: PtmMode, prev: PtmSetting): PtmSetting {
  return mode === "always" ? { on: false, hold: prev.hold } : { on: true, hold: mode };
}

/** Where the key works when nothing reaches it from outside Divora. The
 *  in-app key holds only while the page itself has the keyboard (`App.tsx`
 *  matches it on `document.body`): after a mouse click on a slider, Space
 *  goes to the slider. */
const IN_APP_ONLY =
  " in Divora's window, when no control in it has the keyboard. To use it in a call, set a key in Settings → Hotkeys.";

/** Where the key works when only a MIDI control reaches outside Divora. */
const IN_APP_OR_MIDI =
  " in Divora's window, when no control in it has the keyboard, or with your MIDI control.";

/** The Mixer card's line under its switch. `key` is the in-app key as the
 *  card shows it, the global key when one is bound. `inCall`: what still
 *  works while a call has the focus. "key": a global push-to-modulate key is
 *  bound, so `key` works anywhere. "midi": no global key, but a learned
 *  `ptm` MIDI mapping and a MIDI input picked in Settings, so `key` works
 *  only in Divora's window and the MIDI control works anywhere. null:
 *  neither, so the line says the key works only in Divora's window and
 *  where to set one. */
export function ptmCardLine(
  s: PtmSetting,
  key: string,
  inCall: "key" | "midi" | null,
): string {
  if (!s.on) return "Off · your effects are always on.";
  const where = inCall === "key" ? "." : inCall === "midi" ? IN_APP_OR_MIDI : IN_APP_ONLY;
  return s.hold === "apply"
    ? `Your effects are on only while you hold ${key}${where}`
    : `Your effects are off while you hold ${key}${where}`;
}

/** The Hold to switch button's label. `down`: the button holds it now.
 *  `bypassed`: the mirror's, so the label says what the engine does. */
export function ptmButtonLabel(down: boolean, bypassed: boolean, key: string): string {
  if (!down) return `Hold to switch · or press ${key}`;
  return bypassed ? "Effects off" : "Effects on";
}

/** The Hold to switch button's title: every place it switches, while on. */
export function ptmButtonTitle(on: boolean): string {
  return on
    ? "Switches your effects for the call, your headphones, recordings and Replay while held."
    : "Turn on Push to modulate to try it.";
}

/** Accelerator modifier names (as the global-hotkey parser takes them, any
 *  case, plus Meta, Win and Windows, which the parser refuses and the store
 *  saves as Super) → the KeyboardEvent flag each one needs. On Windows
 *  "Command or Control" is Ctrl; "Super" is the Windows key (`metaKey`). */
const MODIFIER_FLAGS: Readonly<Record<string, "ctrlKey" | "altKey" | "shiftKey" | "metaKey">> = {
  CTRL: "ctrlKey",
  CONTROL: "ctrlKey",
  COMMANDORCONTROL: "ctrlKey",
  COMMANDORCTRL: "ctrlKey",
  CMDORCTRL: "ctrlKey",
  CMDORCONTROL: "ctrlKey",
  ALT: "altKey",
  OPTION: "altKey",
  SHIFT: "shiftKey",
  SUPER: "metaKey",
  META: "metaKey",
  CMD: "metaKey",
  COMMAND: "metaKey",
  WIN: "metaKey",
  WINDOWS: "metaKey",
};

const FLAGS = ["ctrlKey", "altKey", "shiftKey", "metaKey"] as const;

/** The in-app push-to-modulate matcher. `"down"` needs the accelerator's
 *  modifiers exactly (no more, no fewer), so a bare M can't switch the call
 *  when the key is Ctrl+Shift+M. `"up"` matches the trailing key alone, so
 *  letting go of a modifier first still lets go when the trailing key's
 *  character doesn't change with it. One that does (Shift+1 binds as "!",
 *  and its keyup after Shift says "1") is the shell's to match by the
 *  physical key that held (`App.tsx`). The trailing key matches as
 *  it always has: "Space" by `code`, one character by `key` in any case,
 *  anything else (F13, Enter…) by `key` exactly. A modifier name the parser
 *  doesn't know never matches a press, except Meta, Win and Windows, which
 *  match the Windows key as Super does (the store saves them as Super). */
export function ptmKeyMatches(
  accelerator: string,
  e: Pick<KeyboardEvent, "key" | "code" | "ctrlKey" | "altKey" | "shiftKey" | "metaKey">,
  phase: "down" | "up",
): boolean {
  const parts = accelerator.split("+");
  const last = parts[parts.length - 1] ?? "";
  const trailing =
    last === "Space"
      ? e.code === "Space"
      : last.length === 1
        ? e.key.toUpperCase() === last.toUpperCase()
        : last !== "" && e.key === last;
  if (!trailing) return false;
  if (phase === "up") return true;
  const need = { ctrlKey: false, altKey: false, shiftKey: false, metaKey: false };
  for (const token of parts.slice(0, -1)) {
    const flag = MODIFIER_FLAGS[token.trim().toUpperCase()];
    if (flag === undefined) return false;
    need[flag] = true;
  }
  return FLAGS.every((f) => e[f] === need[f]);
}

/** Whether the chain changes the voice: an enabled effect other than the
 *  noise gate. The gate passes speech at unity (its gain is exactly 1 above
 *  its threshold), so a gate-only chain such as Clean Passthrough reads
 *  Clean. */
export function transforms(chain: readonly ChainEntry[]): boolean {
  return chain.some((c) => c.enabled && c.id !== "gate");
}
