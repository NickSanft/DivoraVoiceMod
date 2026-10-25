// v1.9.0: MIDI control-surface router.
//
// Pure mapping layer: given an incoming MIDI message and the user's
// mappings, dispatch to the existing store actions. Deliberately free of
// any Tauri / SolidJS coupling so it unit-tests in isolation — the store
// binds the handlers to its live actions (usePreset / playTileById / PTM /
// monitor / setChainParam / the mute).

import type { MidiMapping, MidiMessage, MidiTrigger } from "../types";

/** The existing store actions a mapping can drive. The store supplies the
 *  live implementations; tests supply spies. */
export interface MidiHandlers {
  usePreset: (presetId: string) => void;
  playTile: (tileId: string) => void;
  /** v1.58.0: a note on a `ptm` mapping, with the mapping's id: note-on
   *  (`down`) holds push-to-modulate for that mapping, note-off lets go. The
   *  store holds it while any mapping is down. */
  ptmNote: (mappingId: string, down: boolean) => void;
  /** v1.58.0: every CC on a `ptm` mapping, with the mapping's id. The store
   *  keeps the side each mapping is on (64 or more holds, 59 or less lets
   *  go) and acts only when it crosses. */
  ptmCc: (mappingId: string, value: number) => void;
  toggleMonitor: () => void;
  setParam: (effectIndex: number, key: string, value: number) => void;
  /** v1.56.0: a note on a `mute` mapping flips the mute. */
  toggleMuted: () => void;
  /** v1.56.0: every CC on a `mute` mapping, with the mapping's id. The store
   *  keeps the side each mapping is on and acts only when it crosses. */
  muteCc: (mappingId: string, value: number) => void;
}

/** The trigger a `midi-message` would match (kind + data1), or null when
 *  the message isn't a note / CC we bind to. */
export function triggerFor(msg: MidiMessage): MidiTrigger | null {
  if (msg.kind === "note-on" || msg.kind === "note-off") {
    return { kind: "note", data1: msg.data1, channel: msg.channel };
  }
  if (msg.kind === "control-change") {
    return { kind: "cc", data1: msg.data1, channel: msg.channel };
  }
  return null;
}

/** Does `msg` match `trigger`? Channel-agnostic, keyed on note-vs-CC and
 *  the note / controller number. */
export function matchesTrigger(msg: MidiMessage, trigger: MidiTrigger): boolean {
  const t = triggerFor(msg);
  return t !== null && t.kind === trigger.kind && t.data1 === trigger.data1;
}

/** A momentary "press" edge: a note-on, or a CC crossing into its upper
 *  half (so a button that sends CC 127 / 0 behaves like a pad). */
export function isActivation(msg: MidiMessage): boolean {
  if (msg.kind === "note-on") return true;
  if (msg.kind === "control-change") return msg.data2 >= 64;
  return false;
}

/** Scale a 0..127 MIDI data byte onto [min, max], clamped. */
export function scaleCc(value: number, min: number, max: number): number {
  const t = Math.min(Math.max(value, 0), 127) / 127;
  return min + t * (max - min);
}

/**
 * Route one MIDI message through the mappings, invoking handlers for
 * every mapping whose trigger matches. Mappings without a learned
 * trigger (or missing their target fields) are skipped.
 */
export function routeMidi(
  msg: MidiMessage,
  mappings: MidiMapping[],
  h: MidiHandlers,
): void {
  for (const m of mappings) {
    if (!m.trigger || !matchesTrigger(msg, m.trigger)) continue;
    switch (m.action) {
      case "preset":
        if (m.presetId !== undefined && isActivation(msg)) h.usePreset(m.presetId);
        break;
      case "tile":
        if (m.tileId !== undefined && isActivation(msg)) h.playTile(m.tileId);
        break;
      case "monitor":
        if (isActivation(msg)) h.toggleMonitor();
        break;
      case "ptm":
        // Momentary: held while the note is down / the CC is up. v1.58.0:
        // each mapping is its own hold, by id, and a CC goes to the store
        // with every value (the dead band needs the side it was on, and the
        // router keeps no state), as the mute's does.
        if (msg.kind === "note-on" || msg.kind === "note-off") {
          h.ptmNote(m.id, msg.kind === "note-on");
        } else if (msg.kind === "control-change") {
          h.ptmCc(m.id, msg.data2);
        }
        break;
      case "param":
        if (
          m.effectIndex !== undefined &&
          m.paramKey !== undefined &&
          m.min !== undefined &&
          m.max !== undefined
        ) {
          h.setParam(m.effectIndex, m.paramKey, scaleCc(msg.data2, m.min, m.max));
        }
        break;
      case "mute":
        // v1.56.0: a pad toggles on note-on (note-off does nothing). A CC
        // goes to the store with every value: whether it crossed needs the
        // side it was on, and the router keeps no state.
        if (msg.kind === "note-on") h.toggleMuted();
        else if (msg.kind === "control-change") h.muteCc(m.id, msg.data2);
        break;
    }
  }
}
