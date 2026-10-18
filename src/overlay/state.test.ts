import { readFileSync, readdirSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import type { ChainEntry } from "../types";
import {
  OVERLAY_BG_COLOR,
  OVERLAY_EDIT_EVENT,
  OVERLAY_EVENT,
  OVERLAY_READY_EVENT,
  OVERLAY_SCENE_EVENT,
  OVERLAY_STATUS_EVENT,
  overlayPayload,
} from "./state";
import fixture from "./wire.fixture.json";

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
    // v1.51.3; frozen in STABLE-SURFACE §2 like `overlay:state`.
    expect(OVERLAY_READY_EVENT).toBe("overlay:ready");
    expect(OVERLAY_BG_COLOR.transparent).toBe("transparent");
    expect(OVERLAY_BG_COLOR.green).toMatch(/^#[0-9a-f]{6}$/i);
    expect(OVERLAY_BG_COLOR.magenta).toMatch(/^#[0-9a-f]{6}$/i);
  });

  it("every overlay event name is the shared fixture's (v1.55.0)", () => {
    // The backend's tests read the same file for `overlay:status`.
    expect({
      state: OVERLAY_EVENT,
      ready: OVERLAY_READY_EVENT,
      status: OVERLAY_STATUS_EVENT,
      scene: OVERLAY_SCENE_EVENT,
      edit: OVERLAY_EDIT_EVENT,
    }).toEqual(fixture.events);
  });

  it("src/overlay has no subfolders", () => {
    // The reading-free scan below lists this folder without recursing, so a
    // subfolder would be a place it can't see.
    const dir = join(process.cwd(), "src", "overlay");
    const folders = readdirSync(dir, { withFileTypes: true })
      .filter((d) => d.isDirectory())
      .map((d) => d.name);
    expect(folders).toEqual([]);
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
    const files = readdirSync(dir)
      .filter((n) => (n.endsWith(".ts") || n.endsWith(".tsx")) && !n.endsWith(".test.ts"))
      .map((n) => join(dir, n));
    // The overlay window's own entry lives one level up (v1.51.3).
    files.push(join(process.cwd(), "src", "overlay.tsx"));
    for (const file of files) {
      const source = readFileSync(file, "utf8");
      expect(source, `${file} references the voice reading`).not.toMatch(banned);
    }
  });
});
