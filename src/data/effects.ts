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
    desc: "Saturation and grit. Drive is the symmetric (odd-harmonic) edge; Warmth biases it asymmetric for even-harmonic 'valve' warmth.",
    params: [
      { key: "drive", label: "Drive", min: 0, max: 100, step: 1, unit: "%", default: 35 },
      { key: "warmth", label: "Warmth", min: 0, max: 100, step: 1, unit: "%", default: 0 },
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
  chorus: {
    id: "chorus",
    name: "Chorus",
    sigil: "wave",
    desc: "Detuned doubler — stacks modulated copies into an ensemble ('many voices from one').",
    params: [
      { key: "mix", label: "Mix", min: 0, max: 100, step: 1, unit: "%", default: 70 },
      { key: "depth", label: "Depth", min: 0, max: 100, step: 1, unit: "%", default: 60 },
    ],
    readout: (v) => `${v.mix ?? 0}% · ${v.depth ?? 0}%`,
  },
  harmonizer: {
    id: "harmonizer",
    name: "Harmonizer",
    sigil: "ab",
    desc: "Stacks pitched copies into a chord (diminished by default). Mix + three interval voices in semitones.",
    params: [
      { key: "mix", label: "Mix", min: 0, max: 100, step: 1, unit: "%", default: 70 },
      { key: "v1", label: "Voice 1", min: -24, max: 24, step: 1, unit: "st", default: 3 },
      { key: "v2", label: "Voice 2", min: -24, max: 24, step: 1, unit: "st", default: 6 },
      { key: "v3", label: "Voice 3", min: -24, max: 24, step: 1, unit: "st", default: 9 },
    ],
    readout: (v) =>
      `${signed(v.v1 ?? 0)}/${signed(v.v2 ?? 0)}/${signed(v.v3 ?? 0)} st`,
  },
  compressor: {
    id: "compressor",
    name: "Compressor",
    sigil: "compressor",
    desc: "Evens out level — tames peaks for a steadier, broadcast-sounding voice. Zero added latency.",
    params: [
      { key: "thresh", label: "Threshold", min: -60, max: 0, step: 1, unit: "dB", default: -24 },
      { key: "ratio", label: "Ratio", min: 1, max: 20, step: 0.5, unit: ":1", default: 3 },
      { key: "attack", label: "Attack", min: 1, max: 100, step: 1, unit: "ms", default: 10 },
      { key: "release", label: "Release", min: 10, max: 500, step: 5, unit: "ms", default: 120 },
      { key: "makeup", label: "Makeup", min: 0, max: 24, step: 1, unit: "dB", default: 0 },
    ],
    readout: (v) => `${v.thresh ?? 0} dB · ${v.ratio ?? 1}:1`,
  },
  deesser: {
    id: "deesser",
    name: "De-esser",
    sigil: "deesser",
    desc: "Tames sibilance — ducks the harsh 'sss' band that pitch/formant shifting exaggerates. Zero added latency.",
    params: [
      { key: "freq", label: "Frequency", min: 3000, max: 10000, step: 100, unit: "Hz", default: 6000 },
      { key: "thresh", label: "Threshold", min: -60, max: 0, step: 1, unit: "dB", default: -30 },
      { key: "range", label: "Range", min: 0, max: 24, step: 1, unit: "dB", default: 12 },
    ],
    readout: (v) => `${((v.freq ?? 0) / 1000).toFixed(1)}k · ${v.thresh ?? 0} dB`,
  },
  radio_bandpass: {
    id: "radio_bandpass",
    name: "Radio Band-pass",
    sigil: "wave",
    desc: "A real band-limiter — steep high-pass + low-pass with a movable resonant 'cone' peak. The through-an-old-radio sound the EQ shelves can't make.",
    params: [
      { key: "hp", label: "Low cut", min: 100, max: 1000, step: 10, unit: "Hz", default: 300 },
      { key: "lp", label: "High cut", min: 1500, max: 6000, step: 50, unit: "Hz", default: 3000 },
      { key: "peak", label: "Resonance", min: 300, max: 4000, step: 50, unit: "Hz", default: 1500 },
      // v1.34.0: gain goes negative so the peak can CUT (a notch), e.g. the
      // Villager voice's ~1 kHz nasal antiformant — not just boost. Not marked
      // bipolar: it defaults to a +6 dB boost, not a centered 0.
      { key: "gain", label: "Res gain", min: -18, max: 18, step: 1, unit: "dB", default: 6 },
      { key: "q", label: "Res Q", min: 0.5, max: 8, step: 0.5, unit: "", default: 3 },
    ],
    readout: (v) => `${v.hp ?? 0}–${v.lp ?? 0} Hz`,
  },
  vintage_noise: {
    id: "vintage_noise",
    name: "Vintage Noise",
    sigil: "distortion",
    desc: "An old-radio noise bed — hiss, mains hum and sparse crackle that swells in the silences and ducks under your voice. Run it last in the chain.",
    params: [
      { key: "hiss", label: "Hiss", min: 0, max: 100, step: 1, unit: "%", default: 25 },
      { key: "hum", label: "Hum", min: 0, max: 100, step: 1, unit: "%", default: 12 },
      { key: "humFreq", label: "Mains", min: 50, max: 60, step: 10, unit: "Hz", default: 60 },
      { key: "crackle", label: "Crackle", min: 0, max: 100, step: 1, unit: "%", default: 15 },
      { key: "tone", label: "Tone", min: 0, max: 100, step: 1, unit: "%", default: 50 },
      { key: "duck", label: "Duck", min: 0, max: 100, step: 1, unit: "%", default: 70 },
    ],
    readout: (v) => `hiss ${v.hiss ?? 0}% · duck ${v.duck ?? 0}%`,
  },
  tremolo: {
    id: "tremolo",
    name: "Tremolo",
    sigil: "wave",
    desc: "A slow amplitude wobble (sub-audio LFO). The pulsing 'hrm-hrm' cadence behind the Villager voice — also helicopter, sci-fi pulse, and classic guitar tremolo.",
    params: [
      { key: "rate", label: "Rate", min: 0.5, max: 12, step: 0.5, unit: "Hz", default: 5 },
      { key: "depth", label: "Depth", min: 0, max: 100, step: 1, unit: "%", default: 40 },
    ],
    readout: (v) => `${v.rate ?? 0} Hz · ${v.depth ?? 0}%`,
  },
  breath: {
    id: "breath",
    name: "Breath",
    sigil: "wave",
    desc: "A whisperizer — additive band-passed hiss that SWELLS with your voice (the opposite of Vintage Noise's duck). The building 'sssSSS' fuse behind the Creeper voice; also whisper / ASMR / wind. Run it late in the chain.",
    params: [
      { key: "amount", label: "Amount", min: 0, max: 100, step: 1, unit: "%", default: 70 },
      { key: "color", label: "Color", min: 0, max: 100, step: 1, unit: "%", default: 65 },
      { key: "swell", label: "Swell", min: 0, max: 100, step: 1, unit: "%", default: 100 },
      { key: "sens", label: "Sensitivity", min: 0, max: 100, step: 1, unit: "%", default: 60 },
    ],
    readout: (v) => `${v.amount ?? 0}% · swell ${v.swell ?? 0}%`,
  },
  warble: {
    id: "warble",
    name: "Warble",
    sigil: "wave",
    desc: "A pitch vibrato — an LFO bends the fundamental up and down (unlike Chorus, which only sums a diluted copy). The otherworldly wobble behind the Enderman; also sci-fi / seasick / theremin. Adds a small latency.",
    params: [
      { key: "rate", label: "Rate", min: 0.5, max: 12, step: 0.5, unit: "Hz", default: 6 },
      { key: "depth", label: "Depth", min: 0, max: 100, step: 1, unit: "%", default: 50 },
      { key: "mix", label: "Mix", min: 0, max: 100, step: 1, unit: "%", default: 100 },
    ],
    readout: (v) => `${v.rate ?? 0} Hz · ${v.depth ?? 0}%`,
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
  "radio_bandpass",
  "compressor",
  "deesser",
  "robot",
  "distortion",
  "echo",
  "reverb",
  "chorus",
  "harmonizer",
  "tremolo",
  "breath",
  "warble",
  "vintage_noise",
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
