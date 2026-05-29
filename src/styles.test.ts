// Source-level sanity checks for src/styles.css.
//
// jsdom doesn't compute layout, so we can't render the actual app and
// observe scroll containers working. Instead we assert the CSS rules
// that bug-fixes depend on are still in the source — a thin regression
// net for the kind of CSS that's easy to delete by accident.

import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";
import { describe, expect, it } from "vitest";

const here = dirname(fileURLToPath(import.meta.url));
const css = readFileSync(join(here, "styles.css"), "utf8");

describe("styles.css", () => {
  it("anchors #root to viewport height (v0.7.1 fix for scroll-broken screens)", () => {
    // Without this rule, App's `height: 100%` has no parent height to
    // resolve against; the entire flex chain collapses and screens
    // taller than the viewport get clipped silently by
    // `body { overflow: hidden }`. The bug looked like "scrolling is
    // broken in Settings / Presets / etc."
    expect(css).toMatch(/#root\s*\{[^}]*height:\s*100%/);
  });

  it("keeps html + body at full height + body overflow:hidden (frameless window contract)", () => {
    expect(css).toMatch(/html,\s*body\s*\{[^}]*height:\s*100%/);
    expect(css).toMatch(/body\s*\{[^}]*overflow:\s*hidden/);
  });

  // v0.11.2: SpellCircle references six animation names via `animation:
  // <name> <dur> ...`. They are defined as @keyframes at the top level
  // of styles.css. If any of these go missing the Motion / Mystical
  // tweaks knobs appear broken because the SpellCircle has nothing to
  // animate — the orbit, pulse rings, decorative ring, dash flow, and
  // particle drift all rely on these keyframes.
  describe("SpellCircle keyframes (v0.11.2)", () => {
    for (const name of [
      "breathe",
      "spin-slow",
      "spin-rev",
      "pulse-ring",
      "dash-flow",
      "float-up",
    ] as const) {
      it(`defines @keyframes ${name}`, () => {
        const pattern = new RegExp(`@keyframes\\s+${name}\\s*\\{`);
        expect(css).toMatch(pattern);
      });
    }

    it("breathe keyframe reads --motion so functional mode visibly damps", () => {
      // The breathe keyframe should reference --motion in opacity OR
      // transform so a low motion setting collapses the animation
      // amplitude even when the animation-duration isn't overridden.
      const breatheMatch = css.match(/@keyframes\s+breathe\s*\{([^}]+\}[^}]+)\}/);
      expect(breatheMatch).toBeTruthy();
      expect(breatheMatch![0]).toMatch(/var\(--motion\)/);
    });
  });

  // v0.11.1 introduced a top-level rule that makes
  // `:root[data-motion="functional"] *` collapse animation-duration and
  // transition-duration to ~0. This is the only thing that makes
  // "functional" feel meaningfully different from "ambient" outside the
  // Spell Circle, so guard it explicitly.
  it("functional motion collapses *all* animations + transitions", () => {
    expect(css).toMatch(
      /:root\[data-motion="functional"\]\s+\*[^}]*animation-duration:\s*0\.001ms\s*!important/,
    );
    expect(css).toMatch(
      /:root\[data-motion="functional"\]\s+\*[^}]*transition-duration:\s*0\.001ms\s*!important/,
    );
  });
});
