// data.jsx — Divora shared content: effects, presets, devices, soundboard
// Effect catalog. Each param: {key,label,min,max,step,unit,default,bipolar}
const EFFECTS = {
  pitch: {
    id: "pitch", name: "Pitch", sigil: "pitch",
    desc: "Shift the fundamental up or down without changing tempo.",
    params: [{ key: "shift", label: "Shift", min: -12, max: 12, step: 1, unit: "st", default: 0, bipolar: true }],
    readout: (v) => `${v.shift > 0 ? "+" : ""}${v.shift} st`,
  },
  formant: {
    id: "formant", name: "Formant", sigil: "formant",
    desc: "Reshape the vocal tract — gender / size of the voice.",
    params: [{ key: "shift", label: "Formant", min: -10, max: 10, step: 1, unit: "", default: 0, bipolar: true }],
    readout: (v) => `${v.shift > 0 ? "+" : ""}${v.shift}`,
  },
  reverb: {
    id: "reverb", name: "Reverb", sigil: "reverb",
    desc: "Space and tail around the voice.",
    params: [
      { key: "size", label: "Size", min: 0, max: 100, step: 1, unit: "%", default: 40 },
      { key: "mix", label: "Mix", min: 0, max: 100, step: 1, unit: "%", default: 25 },
    ],
    readout: (v) => `${v.size}% · ${v.mix}%`,
  },
  eq: {
    id: "eq", name: "EQ", sigil: "eq",
    desc: "Tone-shape low, mid and high bands.",
    params: [
      { key: "low", label: "Low", min: -12, max: 12, step: 1, unit: "dB", default: 0, bipolar: true },
      { key: "mid", label: "Mid", min: -12, max: 12, step: 1, unit: "dB", default: 0, bipolar: true },
      { key: "high", label: "High", min: -12, max: 12, step: 1, unit: "dB", default: 0, bipolar: true },
    ],
    readout: (v) => `${v.low > 0 ? "+" : ""}${v.low} / ${v.mid > 0 ? "+" : ""}${v.mid} / ${v.high > 0 ? "+" : ""}${v.high}`,
  },
  robot: {
    id: "robot", name: "Robot", sigil: "robot",
    desc: "Vocoder-style metallic carrier.",
    params: [
      { key: "freq", label: "Carrier", min: 40, max: 400, step: 5, unit: "Hz", default: 120 },
      { key: "mix", label: "Mix", min: 0, max: 100, step: 1, unit: "%", default: 70 },
    ],
    readout: (v) => `${v.freq} Hz · ${v.mix}%`,
  },
  distortion: {
    id: "distortion", name: "Distortion", sigil: "distortion",
    desc: "Saturation and grit.",
    params: [{ key: "drive", label: "Drive", min: 0, max: 100, step: 1, unit: "%", default: 35 }],
    readout: (v) => `${v.drive}%`,
  },
  echo: {
    id: "echo", name: "Echo", sigil: "echo",
    desc: "Delayed repeats with feedback.",
    params: [
      { key: "time", label: "Time", min: 40, max: 800, step: 10, unit: "ms", default: 240 },
      { key: "fb", label: "Feedback", min: 0, max: 90, step: 1, unit: "%", default: 35 },
    ],
    readout: (v) => `${v.time}ms · ${v.fb}%`,
  },
  gate: {
    id: "gate", name: "Noise Gate", sigil: "gate",
    desc: "Silence the input below a threshold.",
    params: [{ key: "thresh", label: "Threshold", min: -80, max: -20, step: 1, unit: "dB", default: -52 }],
    readout: (v) => `${v.thresh} dB`,
  },
};
const EFFECT_ORDER = ["gate", "pitch", "formant", "eq", "robot", "distortion", "echo", "reverb"];

// helper to make a chain entry
const fx = (id, enabled, vals) => ({ id, enabled, vals: { ...Object.fromEntries(EFFECTS[id].params.map(p => [p.key, p.default])), ...vals } });

const PRESETS = [
  {
    id: "hollow-king", name: "Hollow King", color: "#7C5CF6", glyph: "reverb",
    tag: "Bundled", desc: "Cavernous, regal, distant. A voice from the throne of an empty hall.",
    chain: [fx("gate", true, { thresh: -48 }), fx("pitch", true, { shift: -5 }), fx("formant", true, { shift: -3 }), fx("eq", true, { low: 3, mid: -2, high: 1 }), fx("reverb", true, { size: 78, mix: 42 })],
  },
  {
    id: "static-wraith", name: "Static Wraith", color: "#58C6F2", glyph: "distortion",
    tag: "Bundled", desc: "Broken-radio specter. Bit-crushed whispers riding interference.",
    chain: [fx("gate", true, { thresh: -44 }), fx("pitch", true, { shift: 2 }), fx("distortion", true, { drive: 58 }), fx("eq", true, { low: -4, mid: 3, high: 5 }), fx("echo", true, { time: 180, fb: 48 })],
  },
  {
    id: "velvet-demon", name: "Velvet Demon", color: "#F2567A", glyph: "robot",
    tag: "Bundled", desc: "Smooth, low, and wrong in the best way. Sub-octave menace.",
    chain: [fx("gate", true, { thresh: -50 }), fx("pitch", true, { shift: -7 }), fx("formant", true, { shift: -5 }), fx("robot", true, { freq: 90, mix: 32 }), fx("reverb", true, { size: 30, mix: 18 })],
  },
  {
    id: "choir-of-ash", name: "Choir of Ash", color: "#E9B14C", glyph: "formant",
    tag: "Bundled", desc: "Layered, breathy, sacred. Many voices from one.",
    chain: [fx("pitch", true, { shift: 5 }), fx("formant", true, { shift: 4 }), fx("eq", true, { low: -2, mid: 0, high: 4 }), fx("reverb", true, { size: 64, mix: 50 })],
  },
  {
    id: "deep-warden", name: "Deep Warden", color: "#34D9A0", glyph: "eq",
    tag: "User", desc: "Authoritative narration voice. Warm and grounded.",
    chain: [fx("gate", true, { thresh: -54 }), fx("pitch", true, { shift: -2 }), fx("eq", true, { low: 4, mid: 1, high: -1 }), fx("reverb", true, { size: 22, mix: 12 })],
  },
  {
    id: "glass-oracle", name: "Glass Oracle", color: "#A99FC4", glyph: "echo",
    tag: "User", desc: "Crystalline, prophetic shimmer with long tails.",
    chain: [fx("pitch", true, { shift: 7 }), fx("formant", true, { shift: 2 }), fx("echo", true, { time: 320, fb: 52 }), fx("reverb", true, { size: 88, mix: 56 })],
  },
  {
    id: "clean", name: "Clean Passthrough", color: "#6E6590", glyph: "clean",
    tag: "Bundled", desc: "Your true voice. No effects, gate only.",
    chain: [fx("gate", true, { thresh: -56 })],
  },
];

const DEVICES_IN = [
  { value: "blue-yeti", label: "Blue Yeti X", sub: "USB · 48 kHz · default" },
  { value: "scarlett", label: "Focusrite Scarlett 2i2", sub: "USB interface" },
  { value: "realtek", label: "Realtek HD Audio", sub: "Internal · line-in" },
  { value: "headset", label: "HyperX Cloud II", sub: "Headset mic" },
];
const DEVICES_OUT = [
  { value: "vb-cable", label: "CABLE Input (VB-Audio)", sub: "Virtual · route to apps" },
  { value: "headphones", label: "HyperX Cloud II", sub: "Headphones" },
  { value: "speakers", label: "Realtek Speakers", sub: "Stereo" },
];

const SOUNDBOARD = [
  { id: 1, label: "Thunder Crack", emoji: "⚡", color: "#E9B14C", key: ["F1"], dur: 3.2 },
  { id: 2, label: "Wraith Scream", emoji: "👻", color: "#58C6F2", key: ["F2"], dur: 2.1 },
  { id: 3, label: "Bell Toll", emoji: "🔔", color: "#A99FC4", key: ["F3"], dur: 5.6 },
  { id: 4, label: "Demon Laugh", emoji: "😈", color: "#F2567A", key: ["F4"], dur: 4.0 },
  { id: 5, label: "Rune Hum", emoji: "🜂", color: "#7C5CF6", key: null, dur: 8.0 },
  { id: 6, label: "Glass Shatter", emoji: "🔮", color: "#34D9A0", key: ["F6"], dur: 1.4 },
  { id: 7, label: "Owl Call", emoji: "🦉", color: "#E9B14C", key: null, dur: 2.8 },
  { id: 8, label: "Sub Drop", emoji: "🜄", color: "#6D5BF0", key: ["F8"], dur: 3.6 },
  { id: 9, label: "Whisper Gust", emoji: "🌬️", color: "#58C6F2", key: null, dur: 6.2 },
  { id: 10, label: "Coin Clink", emoji: "🪙", color: "#E9B14C", key: ["F10"], dur: 0.9 },
  { id: 11, label: "Portal Open", emoji: "🌀", color: "#7C5CF6", key: null, dur: 4.4 },
  { id: 12, label: "Heartbeat", emoji: "🫀", color: "#F2567A", key: ["F12"], dur: 7.0 },
];

Object.assign(window, { EFFECTS, EFFECT_ORDER, PRESETS, DEVICES_IN, DEVICES_OUT, SOUNDBOARD, fx });
