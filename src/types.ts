// Shared types for DivoraVoice. Audio/DSP types live alongside the
// engine in Phase 2+; this file is UI-side only.

export type EffectId =
  | "pitch"
  | "formant"
  | "reverb"
  | "eq"
  | "robot"
  | "distortion"
  | "echo"
  | "chorus"
  | "harmonizer"
  /** v1.8.0: dynamics compressor (feed-forward, zero look-ahead). */
  | "compressor"
  /** v1.8.0: split-band de-esser for sibilance. */
  | "deesser"
  /** v1.32.0: vintage-radio band-pass — steep HP+LP + a movable high-Q
   *  "cone" resonance. The real band-limiter behind the old-radio voices. */
  | "radio_bandpass"
  /** v1.32.0: vintage-noise bed — additive hiss + mains hum + crackle that
   *  swells in the gaps and ducks under speech. The first additive effect. */
  | "vintage_noise"
  | "gate"
  | "denoiser"
  /** Phase 12: ONNX-backed voice conversion (LLVC-style). The active
   *  voice is identified by the `voice` param (matched to a file in the
   *  Tauri-managed voices directory). Falls back to passthrough when no
   *  voice is selected or the model file is missing. */
  | "voice_convert";

export type NavId =
  | "mixer"
  | "coven"
  | "soundboard"
  | "speak"
  | "presets"
  | "settings";

export type VoiceStatus = "clean" | "modulated" | "muted";

export type PtmMode = "apply" | "bypass";

export type AbSlot = "A" | "B";

export type GlyphId = "triangle" | "invtriangle" | "square" | "circle";

/** What a cast glyph does (v1.15.0). Discrete one-shot actions only — a
 *  tagged union with its target baked in. Built-in glyphs and custom
 *  glyphs both bind to one of these. */
export type GlyphAction =
  | { kind: "preset"; presetId: string }
  | { kind: "tile"; tileId: string }
  | { kind: "monitor" }
  | { kind: "mute" }
  | { kind: "panic" };

/** A user-recorded custom glyph (v1.15.0): a normalized single-stroke
 *  template (from `makeTemplate` in `data/unistroke`) + the action it
 *  casts when drawn on the Mixer. */
export interface CustomGlyph {
  /** Stable local id — list key and binding key. */
  id: string;
  name: string;
  template: { x: number; y: number }[];
  action: GlyphAction;
}

export type Tone =
  | ""
  | "accent"
  | "success"
  | "warning"
  | "danger"
  | "info";

/** Single parameter knob on an effect. */
export interface EffectParam {
  key: string;
  label: string;
  min: number;
  max: number;
  step: number;
  unit: string;
  default: number;
  bipolar?: boolean;
}

/** Effect catalog entry. */
export interface EffectDef {
  id: EffectId;
  name: string;
  sigil: string;
  desc: string;
  params: EffectParam[];
  readout: (vals: Record<string, number>) => string;
}

/** One effect's state inside a preset chain. */
export interface ChainEntry {
  id: EffectId;
  enabled: boolean;
  vals: Record<string, number>;
}

/** A named, color-tagged preset with an ordered effect chain. */
export interface Preset {
  id: string;
  name: string;
  color: string;
  glyph: string;
  tag: "Bundled" | "User";
  desc: string;
  chain: ChainEntry[];
}

/** Audio device option for input/output pickers. */
export interface DeviceOption {
  value: string;
  label: string;
  sub: string;
}

/**
 * Soundboard tile — comes from the backend (`scan_soundboard_folder`) at
 * runtime. See `src/audio/api.ts` for the canonical wire definition; this
 * alias keeps imports tidy from non-API call sites.
 */
export type { SoundboardTile } from "./audio/api";

/** UI overlay describing a currently-playing clip. */
export interface PlayingClip {
  /** Tile id (matches `SoundboardTile.id`). */
  clipId: string;
  /** `performance.now()` milliseconds when play started. */
  startedAt: number;
  /** Duration in seconds, as reported by the backend on play. */
  durationSecs: number;
}

/** Tweaks visual variations. */
export interface TweaksState {
  /** 0 = subtle, 0.5 = balanced, 1 = rich */
  mystical: number;
  /** 0 = functional, 0.6 = ambient, 1 = rich */
  motion: number;
  /** Light/dark polarity. Orthogonal to `mood` — composes with each hue. */
  theme: "dark" | "light";
  mood: "violet" | "ink" | "midnight";
  accent: "brand" | "abyssal" | "ember";
  grain: boolean;
  vignette: boolean;
}

/** UI-side ephemeral state. */
export interface UiState {
  muted: boolean;
  monitor: boolean;
  ab: AbSlot;
  ptmMode: PtmMode;
  ptmKey: string;
  pressed: boolean;
}

// ---- MIDI control surfaces (v1.9.0) ----

/** The wire message type lives with the IPC layer; re-exported here so
 *  non-API call sites (the router, the store) import it tidily. */
export type { MidiMessage } from "./audio/api";

/** Which existing action a MIDI mapping drives. */
export type MidiActionKind = "preset" | "tile" | "ptm" | "monitor" | "param";

/** The MIDI signal a mapping listens for, captured via MIDI-learn.
 *  Matching is channel-agnostic (hardware varies); `channel` is kept
 *  for display. */
export interface MidiTrigger {
  /** "note" matches note-on / note-off; "cc" matches control-change. */
  kind: "note" | "cc";
  /** Note number or controller number (0..127). */
  data1: number;
  channel: number;
}

/** One control-surface mapping: a learned trigger → an action. */
export interface MidiMapping {
  /** Stable local id (also the list key). */
  id: string;
  action: MidiActionKind;
  /** Null until the user runs MIDI-learn. */
  trigger: MidiTrigger | null;
  /** action === "preset": the preset to summon. */
  presetId?: string;
  /** action === "tile": the soundboard tile to fire. */
  tileId?: string;
  /** action === "param": index into the active (viewed) chain. */
  effectIndex?: number;
  /** action === "param": the effect param key to sweep. */
  paramKey?: string;
  /** action === "param": a CC value of 0..127 maps onto [min, max]. */
  min?: number;
  max?: number;
}
