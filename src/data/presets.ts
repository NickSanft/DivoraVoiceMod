// Bundled and user-default presets.
// Ported from docs/mockups/prototype/divora/data.jsx.

import type { Preset } from "../types";
import { fx } from "./effects";

export const PRESETS: Preset[] = [
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
    id: "deep-warden",
    name: "Deep Warden",
    color: "#34D9A0",
    glyph: "eq",
    tag: "User",
    desc: "Authoritative narration voice. Warm and grounded.",
    chain: [
      fx("gate", true, { thresh: -54 }),
      fx("pitch", true, { shift: -2 }),
      fx("eq", true, { low: 4, mid: 1, high: -1 }),
      fx("reverb", true, { size: 22, mix: 12 }),
    ],
  },
  {
    id: "glass-oracle",
    name: "Glass Oracle",
    color: "#A99FC4",
    glyph: "echo",
    tag: "User",
    desc: "Crystalline, prophetic shimmer with long tails.",
    chain: [
      fx("pitch", true, { shift: 7 }),
      fx("formant", true, { shift: 2 }),
      fx("echo", true, { time: 320, fb: 52 }),
      fx("reverb", true, { size: 88, mix: 56 }),
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
];
