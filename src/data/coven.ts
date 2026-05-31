// The Coven — Divora's curated cast of character voices (v1.1.0).
//
// A "cast member" is a thin curation layer over an existing bundled
// preset: the preset is the source of truth for name / color / glyph /
// chain (loaded via `list_presets`), and this file adds only the
// cast-specific flavor — the lore blurb, the ordering, and the
// DSP-vs-model distinction.
//
// `kind` is the seam for the hybrid roadmap: today every member is
// "dsp" (a tuned effect chain) except the narrator, which is "model"
// (the LLVC voice-conversion model). When zero-shot conversion lands
// (v1.3), realistic members slot in here as additional "model" entries
// with their own `modelId` — no structural change required.

/** Whether a cast member is a DSP effect chain or a conversion model. */
export type CastKind = "dsp" | "model";

/** One member of the Coven. References a bundled preset by id. */
export interface CastMember {
  /** The bundled preset applied when this voice is summoned. Also the
   *  source of the member's name / color / glyph / chain. */
  presetId: string;
  /** DSP character vs. AI conversion model. */
  kind: CastKind;
  /** For `kind: "model"` — the voice-model id to load into Voice Convert
   *  (matches a `VoiceInfo.id` from `list_voices`). */
  modelId?: string;
  /** Evocative one-liner shown on the cast card (richer than the
   *  preset's own `desc`). */
  lore: string;
}

/** The cast, in display order. */
export const COVEN: CastMember[] = [
  {
    presetId: "velvet-demon",
    kind: "dsp",
    lore: "Sub-octave menace, smooth as smoke. Speak and the room leans away.",
  },
  {
    presetId: "hollow-king",
    kind: "dsp",
    lore: "A throne-room baritone from the bottom of a well. Old, vast, patient.",
  },
  {
    presetId: "choir-of-ash",
    kind: "dsp",
    lore: "Many voices from one — breathy, sacred, and just slightly burnt.",
  },
  {
    presetId: "static-wraith",
    kind: "dsp",
    lore: "A signal that shouldn't be talking back. Glitched, ring-modulated, elsewhere.",
  },
  {
    presetId: "the-oracle",
    kind: "dsp",
    lore: "Calm, resonant, certain — the voice that already knows how this ends.",
  },
  {
    presetId: "deep-narrator-ai",
    kind: "model",
    modelId: "llvc-narrator",
    lore: "A real conversion to a deep audiobook narrator — the AI voice, made flesh.",
  },
];

/** Look up a cast member by the preset it applies. */
export function castMemberFor(presetId: string): CastMember | undefined {
  return COVEN.find((m) => m.presetId === presetId);
}
