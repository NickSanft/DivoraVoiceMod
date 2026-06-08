// v1.9.0: MIDI control-surface router.
//
// Pure mapping layer: given an incoming MIDI message and the user's
// mappings, dispatch to the existing store actions. Deliberately free of
// any Tauri / SolidJS coupling so it unit-tests in isolation — App.tsx
// binds the handlers to the live store (usePreset / playTileById / PTM /
// monitor / setChainParam).

import type { MidiMapping, MidiMessage, MidiTrigger } from "../types";

/** The existing store actions a mapping can drive. App.tsx supplies the
 *  live implementations; tests supply spies. */
export interface MidiHandlers {
  usePreset: (presetId: string) => void;
  playTile: (tileId: string) => void;
  setPtm: (pressed: boolean) => void;
  toggleMonitor: () => void;
  setParam: (effectIndex: number, key: string, value: number) => void;
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
        // Momentary: held while the note is down / the CC is in its
        // upper half. Note-off (and CC < 64) releases.
        h.setPtm(isActivation(msg));
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
    }
  }
}
