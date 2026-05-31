// Fallback / browser-preview preset definitions.
//
// In Tauri the real preset list arrives from the backend (`list_presets`
// command — bundled JSON + on-disk user presets). These constants are
// used when the backend isn't reachable (e.g. running `pnpm dev` in a
// browser tab without the Tauri shell).
//
// They mirror the bundled JSON files in
// `divora-core/src/presets/bundled/*.json` exactly.

import type { Preset } from "../types";
import { fx } from "./effects";

export const FALLBACK_PRESETS: Preset[] = [
  {
    id: "hollow-king",
    name: "Hollow King",
    color: "#7C5CF6",
    glyph: "reverb",
    tag: "Bundled",
    desc: "Cavernous, regal, distant. A voice from the throne of an empty hall.",
    chain: [
      fx("gate", true, { thresh: -48 }),
      fx("pitch", true, { shift: -5 }),
      fx("formant", true, { shift: -3 }),
      fx("eq", true, { low: 3, mid: -2, high: 1 }),
      fx("reverb", true, { size: 78, mix: 42 }),
    ],
  },
  {
    id: "static-wraith",
    name: "Static Wraith",
    color: "#58C6F2",
    glyph: "distortion",
    tag: "Bundled",
    desc: "Broken-radio specter. Bit-crushed whispers riding interference.",
    chain: [
      fx("gate", true, { thresh: -44 }),
      fx("pitch", true, { shift: 2 }),
      fx("distortion", true, { drive: 58 }),
      fx("eq", true, { low: -4, mid: 3, high: 5 }),
      fx("echo", true, { time: 180, fb: 48 }),
    ],
  },
  {
    id: "velvet-demon",
    name: "Velvet Demon",
    color: "#F2567A",
    glyph: "robot",
    tag: "Bundled",
    desc: "Smooth, low, and wrong in the best way. Sub-octave menace.",
    chain: [
      fx("gate", true, { thresh: -50 }),
      fx("pitch", true, { shift: -7 }),
      fx("formant", true, { shift: -5 }),
      fx("robot", true, { freq: 90, mix: 32 }),
      fx("reverb", true, { size: 30, mix: 18 }),
    ],
  },
  {
    id: "choir-of-ash",
    name: "Choir of Ash",
    color: "#E9B14C",
    glyph: "formant",
    tag: "Bundled",
    desc: "Layered, breathy, sacred. Many voices from one.",
    chain: [
      fx("pitch", true, { shift: 5 }),
      fx("formant", true, { shift: 4 }),
      fx("eq", true, { low: -2, mid: 0, high: 4 }),
      fx("reverb", true, { size: 64, mix: 50 }),
    ],
  },
  {
    id: "the-oracle",
    name: "The Oracle",
    color: "#7C83F2",
    glyph: "eye",
    tag: "Bundled",
    desc: "Calm, resonant, and certain — the voice that already knows.",
    chain: [
      fx("gate", true, { thresh: -52 }),
      fx("formant", true, { shift: 2 }),
      fx("eq", true, { low: -1, mid: 3, high: 2 }),
      fx("reverb", true, { size: 55, mix: 30 }),
    ],
  },
  {
    id: "clean",
    name: "Clean Passthrough",
    color: "#6E6590",
    glyph: "clean",
    tag: "Bundled",
    desc: "Your true voice. No effects, gate only.",
    chain: [fx("gate", true, { thresh: -56 })],
  },
  {
    id: "deep-narrator-ai",
    name: "Deep Narrator",
    color: "#34D9A0",
    glyph: "wave",
    tag: "Bundled",
    desc: "Deep, warm, close-mic narrator — pitch + formant lower the voice into the chest, an EQ shelf adds body and tames sibilance, and a touch of room gives gravitas. Drop an ONNX model in Settings → Voice library to layer AI voice conversion on top.",
    chain: [
      fx("gate", true, { thresh: -50 }),
      fx("denoiser", true, { mix: 60 }),
      fx("voice_convert", true, { mix: 90 }),
      fx("pitch", true, { shift: -4 }),
      fx("formant", true, { shift: -3 }),
      fx("eq", true, { low: 5, mid: 1, high: -2 }),
      fx("reverb", true, { size: 38, mix: 14 }),
    ],
  },
];

/**
 * Deprecated alias used by the original Phase 1–3 store. New code should
 * use `FALLBACK_PRESETS` or the store's reactive `presets()` signal.
 */
export const PRESETS = FALLBACK_PRESETS;

/** Coerce a backend wire preset into the frontend `Preset` shape. */
export function presetFromWire(wire: {
  id: string;
  name: string;
  color: string;
  glyph: string;
  tag: "Bundled" | "User";
  desc: string;
  chain: { id: string; enabled: boolean; vals: Record<string, number> }[];
}): Preset {
  return {
    id: wire.id,
    name: wire.name,
    color: wire.color,
    glyph: wire.glyph,
    tag: wire.tag,
    desc: wire.desc,
    chain: wire.chain.map((c) => ({
      id: c.id as import("../types").EffectId,
      enabled: c.enabled,
      vals: { ...c.vals },
    })),
  };
}
