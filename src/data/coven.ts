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
    presetId: "seraph",
    kind: "dsp",
    lore: "Radiant and many-winged — a major-chord choir that lifts where the Ash dreads.",
  },
  {
    presetId: "dirge",
    kind: "dsp",
    lore: "A funeral hymn for one: minor thirds over a low toll, draped in cold stone.",
  },
  {
    presetId: "the-swarm",
    kind: "dsp",
    lore: "Not one voice but a thousand, beating against each other. We are legion.",
  },
  {
    presetId: "the-possessed",
    kind: "dsp",
    lore: "Your voice, and the thing wearing it — an octave lower and a fifth beneath.",
  },
  {
    presetId: "leviathan",
    kind: "dsp",
    lore: "From the trench where light dies — the weight of the whole ocean in a voice.",
  },
  {
    presetId: "the-imp",
    kind: "dsp",
    lore: "Small, quick, and delighted by your misfortune — pitched up and never still.",
  },
  {
    presetId: "dispatch",
    kind: "dsp",
    lore: "Clipped, bandlimited, all business — the voice on the other end of the radio.",
  },
  {
    presetId: "spirit-radio",
    kind: "dsp",
    lore: "A warm wireless ghost from 1940 — band-limited and tube-saturated, still reading the late bulletin to a house that emptied years ago.",
  },
  {
    presetId: "parlor-augur",
    kind: "dsp",
    lore: "A thin voice calling its omens across a darkened parlor — all cone and rattle, the little tabletop set keeping only the middle of each word.",
  },
  {
    presetId: "villager",
    kind: "dsp",
    lore: "Hrm. A blocked-nose mumble pitched up small, honking and bobbing its endless 'hrm-hrm' — it has emeralds, dignity is extra.",
  },
  {
    presetId: "creeper",
    kind: "dsp",
    lore: "It doesn't speak so much as hiss — every word swelling into the sibilant fuse of something green and patient standing much too close. Sssss.",
  },
  {
    presetId: "corrupted",
    kind: "dsp",
    lore: "A transmission eaten by the noise — bit-crushed, ring-modulated, breaking up.",
  },
  {
    presetId: "whisper-wraith",
    kind: "dsp",
    lore: "Close enough to feel the breath — airy, intimate, and not quite alive.",
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
