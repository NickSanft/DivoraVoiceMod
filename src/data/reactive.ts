// Reactive effects (v1.46.0) — the dry-voice envelope drives effect
// parameters. Shared between the store (which builds the wire config) and the
// Mixer panel (which renders the controls).
//
// One named behaviour ships: "Rage". The alternative that reads better on
// paper — louder means drier, quieter means wetter, i.e. loudness as
// proximity — is a LEVEL effect, and the whole delivery path is built to
// flatten exactly that: Discord's AGC, Twitch's loudness normalisation, and
// DivoraVoice's own post-chain loudness normalizer. You would hear it in your
// monitor and your audience would not. Drive survives all of it, because
// harmonic content is not level.

import type { EffectId } from "../types";
import { EFFECTS } from "./effects";

/** One leg of a named behaviour: which parameter moves, and how far. */
export interface ReactiveTarget {
  id: EffectId;
  key: string;
  /** Points added at full envelope, before Intensity scales it. Signed. */
  depth: number;
}

/**
 * "Rage" — raise your voice and the character hardens.
 *
 * Drive is the primary; the reverb is seasoning (the room reacts too), which
 * is why its depth is a quarter of the drive's. Both are dry/wet-style scalars
 * the backend whitelist permits; nothing here moves a delay time or a filter
 * corner, which would click when stepped per buffer.
 */
export const RAGE_TARGETS: readonly ReactiveTarget[] = [
  { id: "distortion", key: "drive", depth: 45 },
  { id: "reverb", key: "mix", depth: 12 },
];

/** The effect "Rage" needs in the chain to do anything audible. */
export const RAGE_PRIMARY: EffectId = "distortion";

/** Response window, in dBFS. Floor sits above room noise, ceiling at a shout. */
export const REACTIVE_FLOOR_DB = -42;
export const REACTIVE_CEIL_DB = -14;

/**
 * Follower timing. Release is ~25x attack deliberately: speech modulation
 * peaks around 4–5 Hz (syllable rate), so a release fast enough to resolve
 * syllables makes the parameter flutter at exactly that rate — which reads as
 * "something is wrong with my voice" rather than as responsiveness.
 */
export const REACTIVE_ATTACK_MS = 12;
export const REACTIVE_HOLD_MS = 60;
export const REACTIVE_RELEASE_MS = 300;

/** Default master depth scale, as a percentage. */
export const REACTIVE_DEFAULT_INTENSITY = 70;

/** The catalog default for a parameter, used when a preset doesn't set it. */
export function paramDefault(id: EffectId, key: string): number {
  return EFFECTS[id]?.params.find((p) => p.key === key)?.default ?? 0;
}
