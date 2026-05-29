// SpellCastReveal — the "◆ SPELL CAST ◆" ceremonial reveal that pops
// up after a successful glyph cast. Shows the bound preset's glyph
// (large, in the preset's brand colour) plus its name, plus the
// "◆ SPELL CAST ◆" eyebrow. Auto-dismisses after a fixed window so
// the user can return to the Mixer.
//
// Designed against the docs/mockups/screenshots/08-glyph-cast.png
// frame: centred panel, breath-pulse glow, preset name in colour.

import { onCleanup, onMount, type JSX } from "solid-js";
import { Sigil, type SigilName } from "./Sigil";

/** Default visible window — long enough to read the preset name, short
 * enough not to disrupt the user's flow. Exposed for tests / future tweaks. */
export const REVEAL_DURATION_MS = 1400;

export interface SpellCastRevealProps {
  glyph: SigilName;
  /** Preset display name. */
  name: string;
  /** Brand colour of the preset — used for the glow, glyph tint, and name colour. */
  color: string;
  /** Whether this is a "Bundled" or "User" preset; renders as a small tag. */
  tag: string;
  /** Fired once the reveal finishes its animation cycle. */
  onDone: () => void;
}

export function SpellCastReveal(props: SpellCastRevealProps): JSX.Element {
  onMount(() => {
    const timeout = window.setTimeout(props.onDone, REVEAL_DURATION_MS);
    onCleanup(() => window.clearTimeout(timeout));
  });

  return (
    <div
      role="status"
      aria-live="polite"
      aria-label={`Cast ${props.name}`}
      style={{
        position: "absolute",
        inset: 0,
        "z-index": 95,
        display: "grid",
        "place-items": "center",
        "pointer-events": "none",
        animation: "spell-cast-veil 1400ms ease-out forwards",
      }}
    >
      <div
        style={{
          display: "flex",
          "flex-direction": "column",
          "align-items": "center",
          gap: "var(--s5)",
          padding: "var(--s7) var(--s8)",
          "border-radius": "var(--r-xl)",
          background: "var(--surface-1)",
          border: `1.5px solid ${props.color}66`,
          "box-shadow": `0 0 40px ${props.color}55, var(--shadow-3)`,
          animation:
            "spell-cast-reveal 1400ms cubic-bezier(0.22, 0.61, 0.36, 1) forwards",
          "transform-origin": "center",
        }}
      >
        <div
          class="eyebrow"
          style={{
            color: props.color,
            "letter-spacing": "0.2em",
            "font-size": "var(--t-xs)",
          }}
        >
          ◆ SPELL CAST ◆
        </div>

        <div
          style={{
            width: "104px",
            height: "104px",
            "border-radius": "50%",
            display: "grid",
            "place-items": "center",
            background: `radial-gradient(circle, ${props.color}28, transparent 70%)`,
            color: props.color,
            animation: "spell-cast-breathe 1400ms ease-in-out forwards",
          }}
        >
          <Sigil name={props.glyph} size={64} />
        </div>

        <div
          class="display"
          style={{
            "font-size": "var(--t-display)",
            "font-weight": 700,
            color: props.color,
            "text-align": "center",
            "line-height": 1,
          }}
        >
          {props.name}
        </div>

        <div
          style={{
            "font-size": "var(--t-xs)",
            color: "var(--text-lo)",
            "font-family": "var(--font-mono)",
            "letter-spacing": "0.18em",
            "text-transform": "uppercase",
          }}
        >
          {props.tag}
        </div>
      </div>
    </div>
  );
}
