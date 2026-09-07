// The bundled presets exist TWICE: as JSON compiled into divora-core, and as
// FALLBACK_PRESETS here (used when the backend list call hasn't landed or
// fails). Nothing kept those in sync — a preset edit had to be made by hand in
// both places, and a drift would surface only as the fallback quietly
// disagreeing with the real voice.
//
// This reads the Rust-side JSON off disk and asserts the mirror matches. Added
// in v1.48.0 after the Bitcrusher work required exactly that hand-duplication.

import { readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import { FALLBACK_PRESETS } from "./presets";

const BUNDLED_DIR = join(
  process.cwd(),
  "divora-core",
  "src",
  "presets",
  "bundled",
);

interface WireEntry {
  id: string;
  enabled: boolean;
  vals: Record<string, number>;
}
interface WireJson {
  id: string;
  name: string;
  desc: string;
  color: string;
  glyph: string;
  chain: WireEntry[];
}

function loadBundled(): WireJson[] {
  return readdirSync(BUNDLED_DIR)
    .filter((f) => f.endsWith(".json"))
    .map((f) => JSON.parse(readFileSync(join(BUNDLED_DIR, f), "utf8")) as WireJson);
}

describe("bundled presets mirror divora-core", () => {
  const bundled = loadBundled();

  it("finds the Rust-side bundled JSON", () => {
    // Guards the path itself: if this ever resolves to an empty directory the
    // rest of the suite would pass vacuously.
    expect(bundled.length).toBeGreaterThan(20);
  });

  it("every bundled preset exists in the frontend fallback", () => {
    const front = new Set(FALLBACK_PRESETS.map((p) => p.id));
    const missing = bundled.map((b) => b.id).filter((id) => !front.has(id));
    expect(missing).toEqual([]);
  });

  it("the fallback invents no preset the backend does not ship", () => {
    const back = new Set(bundled.map((b) => b.id));
    const extra = FALLBACK_PRESETS.map((p) => p.id).filter((id) => !back.has(id));
    expect(extra).toEqual([]);
  });

  it("names, descriptions, colours and glyphs match", () => {
    for (const b of bundled) {
      const f = FALLBACK_PRESETS.find((p) => p.id === b.id);
      expect(f, `no fallback for ${b.id}`).toBeDefined();
      expect(f!.name, `name drift on ${b.id}`).toBe(b.name);
      expect(f!.desc, `desc drift on ${b.id}`).toBe(b.desc);
      expect(f!.color, `color drift on ${b.id}`).toBe(b.color);
      expect(f!.glyph, `glyph drift on ${b.id}`).toBe(b.glyph);
    }
  });

  it("effect chains match in order and in authored values", () => {
    for (const b of bundled) {
      const f = FALLBACK_PRESETS.find((p) => p.id === b.id)!;
      expect(
        f.chain.map((c) => c.id),
        `chain order drift on ${b.id}`,
      ).toEqual(b.chain.map((c) => c.id));

      // A SUBSET check, not deep equality: fx() seeds every catalog default,
      // so the frontend entry legitimately carries keys the JSON omits. What
      // must agree is every value the preset actually authored.
      b.chain.forEach((entry, i) => {
        const mine = f.chain[i]!;
        // `enabled` is the drift that would matter most and show up least: two
        // copies with the same effects in the same order, one of them silently
        // bypassing a stage.
        expect(
          mine.enabled,
          `${b.id} → ${entry.id} enabled drift`,
        ).toBe(entry.enabled);
        for (const [key, value] of Object.entries(entry.vals)) {
          expect(
            mine.vals[key],
            `${b.id} → ${entry.id}.${key} drift`,
          ).toBe(value);
        }
      });
    }
  });
});
