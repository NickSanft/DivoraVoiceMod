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
  | "gate"
  | "denoiser"
  /** Phase 12: ONNX-backed voice conversion (LLVC-style). The active
   *  voice is identified by the `voice` param (matched to a file in the
   *  Tauri-managed voices directory). Falls back to passthrough when no
   *  voice is selected or the model file is missing. */
  | "voice_convert";

export type NavId = "mixer" | "soundboard" | "presets" | "settings";

export type VoiceStatus = "clean" | "modulated" | "muted";

export type PtmMode = "apply" | "bypass";

export type AbSlot = "A" | "B";

export type GlyphId = "triangle" | "invtriangle" | "square" | "circle";

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
