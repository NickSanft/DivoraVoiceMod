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
      fx("pitch", true, { shift: -2 }),
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
      fx("reverb", true, { size: 48, mix: 34 }),
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
      fx("pitch", true, { shift: 2 }),
      fx("formant", true, { shift: 3 }),
      fx("harmonizer", true, { mix: 85, v1: 3, v2: 6, v3: 9 }),
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
    id: "seraph",
    name: "Seraph",
    color: "#F4D35E",
    glyph: "bolt",
    tag: "Bundled",
    desc: "Radiant and many-winged — a major-chord choir that lifts instead of dreads.",
    chain: [
      fx("gate", true, { thresh: -52 }),
      fx("formant", true, { shift: 2 }),
      fx("harmonizer", true, { mix: 80, v1: 4, v2: 7, v3: 12 }),
      fx("eq", true, { low: -2, mid: 0, high: 4 }),
      fx("reverb", true, { size: 58, mix: 36 }),
    ],
  },
  {
    id: "dirge",
    name: "Dirge",
    color: "#5B6B9E",
    glyph: "echo",
    tag: "Bundled",
    desc: "A funeral hymn for one — minor thirds over a low toll, draped in stone.",
    chain: [
      fx("gate", true, { thresh: -52 }),
      fx("pitch", true, { shift: -2 }),
      fx("harmonizer", true, { mix: 80, v1: 3, v2: 7, v3: -12 }),
      fx("eq", true, { low: 2, mid: -1, high: -2 }),
      fx("reverb", true, { size: 70, mix: 44 }),
    ],
  },
  {
    id: "the-swarm",
    name: "The Swarm",
    color: "#8FB339",
    glyph: "ab",
    tag: "Bundled",
    desc: "Not one voice but a thousand, beating against each other. We are legion.",
    chain: [
      fx("gate", true, { thresh: -48 }),
      fx("harmonizer", true, { mix: 75, v1: 1, v2: 2, v3: -1 }),
      fx("robot", true, { freq: 110, mix: 25 }),
      fx("distortion", true, { drive: 30 }),
      fx("eq", true, { low: -2, mid: 3, high: 3 }),
    ],
  },
  {
    id: "the-possessed",
    name: "The Possessed",
    color: "#9B2D5E",
    glyph: "panic",
    tag: "Bundled",
    desc: "Your voice, and the thing wearing it — an octave lower and a fifth beneath.",
    chain: [
      fx("gate", true, { thresh: -50 }),
      fx("harmonizer", true, { mix: 80, v1: -12, v2: -7, v3: -12 }),
      fx("robot", true, { freq: 70, mix: 22 }),
      fx("distortion", true, { drive: 28 }),
      fx("reverb", true, { size: 40, mix: 22 }),
    ],
  },
  {
    id: "leviathan",
    name: "Leviathan",
    color: "#2E5E8C",
    glyph: "modulated",
    tag: "Bundled",
    desc: "From the trench where light dies — the weight of the whole ocean in a voice.",
    chain: [
      fx("gate", true, { thresh: -50 }),
      fx("pitch", true, { shift: -10 }),
      fx("formant", true, { shift: -7 }),
      fx("eq", true, { low: 6, mid: -2, high: -3 }),
      fx("distortion", true, { drive: 18 }),
      fx("reverb", true, { size: 82, mix: 42 }),
    ],
  },
  {
    id: "the-imp",
    name: "The Imp",
    color: "#F2A65A",
    glyph: "play",
    tag: "Bundled",
    desc: "Small, quick, and delighted by your misfortune. Pitched up and won't sit still.",
    chain: [
      fx("gate", true, { thresh: -48 }),
      fx("pitch", true, { shift: 7 }),
      fx("formant", true, { shift: 6 }),
      fx("eq", true, { low: -3, mid: 2, high: 4 }),
      fx("echo", true, { time: 90, fb: 25 }),
    ],
  },
  {
    id: "dispatch",
    name: "Dispatch",
    color: "#5FA8A0",
    glyph: "mic",
    tag: "Bundled",
    desc: "Clipped, bandlimited, all business — the voice on the other end of the radio.",
    chain: [
      fx("gate", true, { thresh: -44 }),
      fx("eq", true, { low: -7, mid: 5, high: -5 }),
      fx("distortion", true, { drive: 25 }),
    ],
  },
  {
    id: "corrupted",
    name: "Corrupted",
    color: "#C13584",
    glyph: "warning",
    tag: "Bundled",
    desc: "A transmission eaten by the noise — bit-crushed, ring-modulated, breaking up.",
    chain: [
      fx("gate", true, { thresh: -46 }),
      fx("distortion", true, { drive: 65 }),
      fx("robot", true, { freq: 180, mix: 45 }),
      fx("echo", true, { time: 60, fb: 55 }),
      fx("eq", true, { low: -2, mid: 4, high: 5 }),
    ],
  },
  {
    id: "whisper-wraith",
    name: "Whisper Wraith",
    color: "#9AA7C7",
    glyph: "muted",
    tag: "Bundled",
    desc: "Close enough to feel the breath — airy, intimate, and not quite alive.",
    chain: [
      fx("gate", true, { thresh: -55 }),
      fx("formant", true, { shift: 1 }),
      fx("eq", true, { low: -4, mid: -1, high: 6 }),
      fx("reverb", true, { size: 48, mix: 30 }),
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
      fx("pitch", true, { shift: -7 }),
      fx("formant", true, { shift: -5 }),
      fx("eq", true, { low: 5, mid: 1, high: -2 }),
      fx("reverb", true, { size: 38, mix: 14 }),
    ],
  },
  {
    id: "spirit-radio",
    name: "Spirit Radio",
    color: "#C8862E",
    glyph: "wave",
    tag: "Bundled",
    desc: "The warm valve-glow of a 1940s broadcast — band-limited through the old transmitter, leaned flat by the announcer's compressor, warmed with tube saturation, and riding a faint carrier hiss that swells when the voice falls silent. The set that never signs off.",
    chain: [
      fx("gate", true, { thresh: -50 }),
      fx("compressor", true, { thresh: -30, ratio: 8, attack: 7, release: 160, makeup: 5 }),
      fx("distortion", true, { drive: 18, warmth: 60 }),
      fx("radio_bandpass", true, { hp: 180, lp: 3500, peak: 1500, gain: 7, q: 3 }),
      fx("vintage_noise", true, { hiss: 22, hum: 10, humFreq: 60, crackle: 10, tone: 45, duck: 75 }),
    ],
  },
  {
    id: "parlor-augur",
    name: "Parlor Augur",
    color: "#B5996A",
    glyph: "echo",
    tag: "Bundled",
    desc: "Heard across a darkened room through a tin-throated parlor set — a narrow, cone-honking band, squashed by a cheap valve AGC, with the hiss and crackle of the little box filling the quiet. It keeps only the middle of the word and throws the rest to the dust.",
    chain: [
      fx("gate", true, { thresh: -50 }),
      fx("compressor", true, { thresh: -28, ratio: 10, attack: 6, release: 110, makeup: 5 }),
      fx("distortion", true, { drive: 22, warmth: 50 }),
      fx("radio_bandpass", true, { hp: 350, lp: 2600, peak: 2400, gain: 9, q: 4 }),
      fx("reverb", true, { size: 8, mix: 6 }),
      fx("vintage_noise", true, { hiss: 40, hum: 14, humFreq: 60, crackle: 20, tone: 35, duck: 65 }),
    ],
  },
  {
    id: "villager",
    name: "Villager",
    color: "#8A9A5B",
    glyph: "pitch",
    tag: "Bundled",
    desc: "The blocked-nose hum of the village idle — pitched up small, honking through a pinched nasal band with the murmur boosted and the 'm' notch carved out, mumbling its endless 'hrm-hrm' to nobody. Will trade emeralds for your dignity.",
    chain: [
      fx("gate", true, { thresh: -48 }),
      fx("pitch", true, { shift: 4 }),
      fx("formant", true, { shift: 2 }),
      // Nasal POLE: boost the low ~300 Hz murmur + band-limit to the dull top.
      fx("radio_bandpass", true, { hp: 200, lp: 3400, peak: 300, gain: 8, q: 2.5 }),
      // Nasal NOTCH: cut the ~1.05 kHz "m" antiformant (edges parked wide so
      // this stage is a pure notch, not a second band-limit).
      fx("radio_bandpass", true, { hp: 100, lp: 6000, peak: 1050, gain: -8, q: 6 }),
      fx("eq", true, { low: -2, mid: 3, high: -4 }),
      fx("distortion", true, { drive: 13, warmth: 35 }),
      fx("compressor", true, { thresh: -28, ratio: 6, attack: 5, release: 90, makeup: 4 }),
      // The "hrm-hrm" wobble — after the compressor so leveling can't fight it.
      fx("tremolo", true, { rate: 5, depth: 40 }),
    ],
  },
  {
    id: "creeper",
    name: "Creeper",
    color: "#5CA83E",
    glyph: "wave",
    tag: "Bundled",
    desc: "The thing that crept up behind you — every word dissolving into a rising, sibilant hiss, the fuse-breath that swells the longer you speak. You have about one and a half seconds. Sssss.",
    chain: [
      fx("gate", true, { thresh: -46 }),
      fx("pitch", true, { shift: -4 }),
      // Thin the chest + a static air/brightness lift (the shelf hinges ~1.25 kHz).
      fx("eq", true, { low: -8, mid: -4, high: 12 }),
      // Band-limit to the sizzle window + a resonant sss cone at the 4 kHz cap.
      fx("radio_bandpass", true, { hp: 300, lp: 6000, peak: 4000, gain: 8, q: 2 }),
      fx("distortion", true, { drive: 18, warmth: 0 }),
      // The linchpin: hiss that SWELLS with each word — the building fuse.
      // swell 100 = no static floor, so it's silent when idle (only rides speech).
      fx("breath", true, { amount: 75, color: 65, swell: 100, sens: 60 }),
      // A slow simmer layered under the swell (breath carries most of it).
      fx("tremolo", true, { rate: 1, depth: 30 }),
      fx("reverb", true, { size: 65, mix: 30 }),
    ],
  },
  {
    id: "zombie",
    name: "Zombie",
    color: "#5E7250",
    glyph: "distortion",
    tag: "Bundled",
    desc: "A slow, phlegmy moan dragged up from a caved-in chest — pitched down just enough to be a person who died a while ago, gargling the words through the flu it never got over. Unnnh.",
    chain: [
      fx("gate", true, { thresh: -46 }),
      // Shallow drop — an undead HUMAN, not a demon octave. Keep it above Leviathan.
      fx("pitch", true, { shift: -5 }),
      fx("formant", true, { shift: -4 }),
      // Octave-down sub (all three voices on one octave), ground into the
      // distortion below so it fuses into one gravelly throat, not a chord.
      fx("harmonizer", true, { mix: 35, v1: -12, v2: -12, v3: -12 }),
      fx("distortion", true, { drive: 35, warmth: 45 }),
      // Dark + congested, but keep the CHEST (low HP, unlike the radio voices).
      fx("radio_bandpass", true, { hp: 120, lp: 3200, peak: 1000, gain: 4, q: 1.5 }),
      fx("eq", true, { low: 3, mid: 0, high: -5 }),
      // Watery detune = the wet-mouth gurgle (doesn't fight the slow tremolo).
      fx("chorus", true, { mix: 40, depth: 80 }),
      // Slow moan.
      fx("tremolo", true, { rate: 1.5, depth: 40 }),
      fx("reverb", true, { size: 65, mix: 30 }),
    ],
  },
  {
    id: "enderman",
    name: "Enderman",
    color: "#8B5CF6",
    glyph: "echo",
    tag: "Bundled",
    desc: "A tall shadow that speaks a language like your own words played backward — warbling up out of the void, pitched down and elsewhere. Don't look at it. Don't look— oh.",
    chain: [
      fx("gate", true, { thresh: -44 }),
      fx("pitch", true, { shift: -6 }),
      fx("formant", true, { shift: -5 }),
      // Low ring-mod carrier = the inhuman metallic garble.
      fx("robot", true, { freq: 45, mix: 40 }),
      fx("chorus", true, { mix: 70, depth: 90 }),
      // The linchpin: a real pitch-vibrato that bends the fundamental (the
      // otherworldly wobble the reversed enderman speech is famous for).
      fx("warble", true, { rate: 6, depth: 55, mix: 100 }),
      fx("distortion", true, { drive: 25, warmth: 40 }),
      fx("radio_bandpass", true, { hp: 250, lp: 3500, peak: 1200, gain: 4, q: 1.5 }),
      fx("eq", true, { low: 0, mid: 0, high: -6 }),
      fx("echo", true, { time: 200, fb: 25 }),
      // The void.
      fx("reverb", true, { size: 80, mix: 45 }),
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
