// Effect catalog. Each effect has a sigil, a description, and a
// parameter list. The readout function takes the current vals and
// produces the short string shown beneath each node on the Mixer's
// spell circle (Phase 3) and on chain cards in the Presets editor
// (Phase 4).
//
// Ported from docs/mockups/prototype/divora/data.jsx.

import type { EffectDef, EffectId } from "../types";

function signed(n: number): string {
  return n > 0 ? `+${n}` : `${n}`;
}

export const EFFECTS: Record<EffectId, EffectDef> = {
  pitch: {
    id: "pitch",
    name: "Pitch",
    sigil: "pitch",
    desc: "Shift the fundamental up or down without changing tempo.",
    params: [
      { key: "shift", label: "Shift", min: -12, max: 12, step: 1, unit: "st", default: 0, bipolar: true },
    ],
    readout: (v) => `${signed(v.shift ?? 0)} st`,
  },
  formant: {
    id: "formant",
    name: "Formant",
    sigil: "formant",
    desc: "Reshape the vocal tract — gender / size of the voice.",
    params: [
      { key: "shift", label: "Formant", min: -10, max: 10, step: 1, unit: "", default: 0, bipolar: true },
    ],
    readout: (v) => signed(v.shift ?? 0),
  },
  reverb: {
    id: "reverb",
    name: "Reverb",
    sigil: "reverb",
    desc: "Space and tail around the voice.",
    params: [
      { key: "size", label: "Size", min: 0, max: 100, step: 1, unit: "%", default: 40 },
      { key: "mix", label: "Mix", min: 0, max: 100, step: 1, unit: "%", default: 25 },
    ],
    readout: (v) => `${v.size ?? 0}% · ${v.mix ?? 0}%`,
  },
  eq: {
    id: "eq",
    name: "EQ",
    sigil: "eq",
    desc: "Tone-shape low, mid and high bands.",
    params: [
      { key: "low", label: "Low", min: -12, max: 12, step: 1, unit: "dB", default: 0, bipolar: true },
      { key: "mid", label: "Mid", min: -12, max: 12, step: 1, unit: "dB", default: 0, bipolar: true },
      { key: "high", label: "High", min: -12, max: 12, step: 1, unit: "dB", default: 0, bipolar: true },
    ],
    readout: (v) =>
      `${signed(v.low ?? 0)} / ${signed(v.mid ?? 0)} / ${signed(v.high ?? 0)}`,
  },
  robot: {
    id: "robot",
    name: "Robot",
    sigil: "robot",
    desc: "Vocoder-style metallic carrier.",
    params: [
      { key: "freq", label: "Carrier", min: 40, max: 400, step: 5, unit: "Hz", default: 120 },
      { key: "mix", label: "Mix", min: 0, max: 100, step: 1, unit: "%", default: 70 },
    ],
    readout: (v) => `${v.freq ?? 0} Hz · ${v.mix ?? 0}%`,
  },
  distortion: {
    id: "distortion",
    name: "Distortion",
    sigil: "distortion",
    desc: "Saturation and grit.",
    params: [
      { key: "drive", label: "Drive", min: 0, max: 100, step: 1, unit: "%", default: 35 },
    ],
    readout: (v) => `${v.drive ?? 0}%`,
  },
  echo: {
    id: "echo",
    name: "Echo",
    sigil: "echo",
    desc: "Delayed repeats with feedback.",
    params: [
      { key: "time", label: "Time", min: 40, max: 800, step: 10, unit: "ms", default: 240 },
      { key: "fb", label: "Feedback", min: 0, max: 90, step: 1, unit: "%", default: 35 },
    ],
    readout: (v) => `${v.time ?? 0}ms · ${v.fb ?? 0}%`,
  },
  gate: {
    id: "gate",
    name: "Noise Gate",
    sigil: "gate",
    desc: "Silence the input below a threshold.",
    params: [
      { key: "thresh", label: "Threshold", min: -80, max: -20, step: 1, unit: "dB", default: -52 },
    ],
    readout: (v) => `${v.thresh ?? 0} dB`,
  },
  denoiser: {
    id: "denoiser",
    name: "Denoiser",
    sigil: "shield",
    desc: "RNNoise-based suppression. Only active at 48 kHz; 10 ms latency.",
    params: [
      { key: "mix", label: "Mix", min: 0, max: 100, step: 1, unit: "%", default: 80 },
    ],
    readout: (v) => `${v.mix ?? 0}%`,
  },
  voice_convert: {
    id: "voice_convert",
    name: "Voice Convert",
    sigil: "wave",
    desc: "ONNX-backed AI voice conversion (LLVC). Pick a voice in Settings → Voice library, then dial in wet/dry mix. Falls back to passthrough when the ONNX Runtime DLL or chosen voice model isn't installed.",
    params: [
      { key: "mix", label: "Mix", min: 0, max: 100, step: 1, unit: "%", default: 90 },
    ],
    readout: (v) => `${v.mix ?? 0}%`,
  },
};

/** Default order effects appear in pickers and the spell circle. */
export const EFFECT_ORDER: EffectId[] = [
  "gate",
  "denoiser",
  "voice_convert",
  "pitch",
  "formant",
  "eq",
  "robot",
  "distortion",
  "echo",
  "reverb",
];

/** Build a chain entry with sensible defaults, optionally overriding values. */
export function fx(
  id: EffectId,
  enabled: boolean,
  vals: Record<string, number> = {},
): import("../types").ChainEntry {
  const defaults: Record<string, number> = {};
  for (const p of EFFECTS[id].params) defaults[p.key] = p.default;
  return { id, enabled, vals: { ...defaults, ...vals } };
}
