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
});
