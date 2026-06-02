import { describe, expect, it } from "vitest";
import { COVEN, castMemberFor } from "./coven";
import { FALLBACK_PRESETS } from "./presets";
import { EFFECTS } from "./effects";

describe("The Coven cast", () => {
  it("every member maps to a known bundled preset", () => {
    const ids = new Set(FALLBACK_PRESETS.map((p) => p.id));
    for (const m of COVEN) {
      expect(ids.has(m.presetId)).toBe(true);
    }
  });

  it("model members carry a modelId; dsp members don't", () => {
    for (const m of COVEN) {
      if (m.kind === "model") {
        expect(m.modelId).toBeTruthy();
      } else {
        expect(m.modelId).toBeUndefined();
      }
    }
  });

  it("has exactly one model-backed member — the LLVC narrator", () => {
    const models = COVEN.filter((m) => m.kind === "model");
    expect(models).toHaveLength(1);
    expect(models[0]!.modelId).toBe("llvc-narrator");
  });

  it("every member has non-empty lore", () => {
    for (const m of COVEN) {
      expect(m.lore.length).toBeGreaterThan(0);
    }
  });

  it("castMemberFor resolves by preset id", () => {
    expect(castMemberFor("velvet-demon")?.kind).toBe("dsp");
    expect(castMemberFor("deep-narrator-ai")?.kind).toBe("model");
    expect(castMemberFor("not-a-member")).toBeUndefined();
  });

  // v1.2.0 / v1.2.1: the chorus + harmonizer effects power the Coven.
  it("the chorus + harmonizer effects are in the catalog", () => {
    expect(EFFECTS.chorus).toBeDefined();
    expect(EFFECTS.harmonizer).toBeDefined();
    expect(EFFECTS.harmonizer.params.some((p) => p.key === "v1")).toBe(true);
  });

  it("Choir of Ash layers a harmonizer for its chord", () => {
    const choir = FALLBACK_PRESETS.find((p) => p.id === "choir-of-ash");
    expect(choir).toBeDefined();
    expect(choir!.chain.some((e) => e.id === "harmonizer" && e.enabled)).toBe(
      true,
    );
  });
});
