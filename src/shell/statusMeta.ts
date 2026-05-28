// statusMeta — UI presentation for the three voice states.
// Pure data; safe to import anywhere.

import type { SigilName } from "../components/Sigil";
import type { VoiceStatus } from "../types";

export interface StatusMeta {
  label: string;
  sub: string;
  sigil: SigilName;
  /** CSS color reference for the status dot, ring, and core. */
  color: string;
  /** Border / hairline color around status surfaces. */
  line: string;
  /** Background tint for the Voice status card. */
  bg: string;
}

const META: Record<VoiceStatus, StatusMeta> = {
  muted: {
    label: "Muted",
    sub: "Nothing is being sent",
    sigil: "muted",
    color: "var(--danger)",
    line: "rgba(242, 86, 122, 0.3)",
    bg: "var(--danger-bg)",
  },
  modulated: {
    label: "Modulated",
    sub: "Spell active · voice transformed",
    sigil: "modulated",
    color: "var(--indigo)",
    line: "var(--line-glow)",
    bg: "var(--accent-bg)",
  },
  clean: {
    label: "Clean",
    sub: "Your true voice, passing through",
    sigil: "clean",
    color: "var(--text-mid)",
    line: "var(--line)",
    bg: "transparent",
  },
};

export function statusMeta(status: VoiceStatus): StatusMeta {
  return META[status];
}
