import { readFileSync, readdirSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import type { ChainEntry } from "../types";
import { OVERLAY_BG_COLOR, OVERLAY_EVENT, overlayPayload } from "./state";

describe("overlay state", () => {
  it("overlayPayload bundles the live values verbatim", () => {
    const chain: ChainEntry[] = [{ id: "gate", enabled: true, vals: { thresh: -52 } }];
    const p = overlayPayload({
      chain,
      status: "modulated",
      motion: 0.6,
      mystical: 0.7,
      mood: "ink",
      accent: "ember",
      theme: "light",
      bg: "green",
    });
    expect(p).toEqual({
      chain,
      status: "modulated",
      motion: 0.6,
      mystical: 0.7,
      mood: "ink",
      accent: "ember",
      theme: "light",
      bg: "green",
    });
  });

  it("the event name + chroma colours are stable", () => {
    expect(OVERLAY_EVENT).toBe("overlay:state");
    expect(OVERLAY_BG_COLOR.transparent).toBe("transparent");
    expect(OVERLAY_BG_COLOR.green).toMatch(/^#[0-9a-f]{6}$/i);
    expect(OVERLAY_BG_COLOR.magenta).toMatch(/^#[0-9a-f]{6}$/i);
  });
});

describe("the overlay carries no voice reading", () => {
  // Deliberate, and the one promise here that a user cannot check for
  // themselves: an overlay audience has no way to verify a reading, and a
  // guest speaking into the streamer's mic would be read too.
  //
  // This replaces an e2e test that could never fail. It filtered recorded
  // invokes for `plugin:event`, but the mock returns every `plugin:` call
  // before recording it, so the filter always matched nothing and the
  // assertion ran against an empty array. A green check on an unverified
  // claim is worse than no check.
  it("builds a payload whose keys are fixed and reading-free", () => {
    const payload = overlayPayload({
      chain: [],
      status: "clean",
      motion: 1,
      mystical: 0.5,
      mood: "violet",
      accent: "brand",
      theme: "dark",
      bg: "transparent",
    });
    expect(Object.keys(payload).sort()).toEqual([
      "accent",
      "bg",
      "chain",
      "mood",
      "motion",
      "mystical",
      "status",
      "theme",
    ]);
  });

  it("has no reading code in the overlay at all", () => {
    // Keys alone would not catch a future overlay component reading the
    // store directly, so check the source of the whole overlay surface.
    const dir = join(process.cwd(), "src", "overlay");
    const banned = /voiceReading|voice_reading|VoiceReading|wetBypassed|brightnessHz|paceOps/;
    for (const name of readdirSync(dir)) {
      if (!name.endsWith(".ts") && !name.endsWith(".tsx")) continue;
      if (name.endsWith(".test.ts")) continue;
      const source = readFileSync(join(dir, name), "utf8");
      expect(source, `${name} references the voice reading`).not.toMatch(banned);
    }
  });
});
