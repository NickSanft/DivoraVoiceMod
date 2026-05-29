import { describe, expect, it } from "vitest";
import { EFFECTS, EFFECT_ORDER, fx } from "./effects";

describe("EFFECTS catalog", () => {
  it("lists every effect in EFFECT_ORDER", () => {
    for (const id of EFFECT_ORDER) {
      expect(EFFECTS[id]).toBeDefined();
    }
  });

  it("each effect has at least one parameter", () => {
    for (const id of EFFECT_ORDER) {
      expect(EFFECTS[id].params.length).toBeGreaterThan(0);
    }
  });

  it("bipolar params have a default of 0 (centered)", () => {
    for (const id of EFFECT_ORDER) {
      for (const p of EFFECTS[id].params) {
        if (p.bipolar) {
          expect(p.default).toBe(0);
        }
      }
    }
  });

  it("noise gate threshold range matches the design spec", () => {
    const t = EFFECTS.gate.params.find((p) => p.key === "thresh");
    expect(t).toBeDefined();
    expect(t!.min).toBe(-80);
    expect(t!.max).toBe(-20);
    expect(t!.default).toBe(-52);
  });

  it("pitch supports ±12 semitones (design spec)", () => {
    const p = EFFECTS.pitch.params[0];
    expect(p).toBeDefined();
    expect(p?.min).toBe(-12);
    expect(p?.max).toBe(12);
    expect(p?.bipolar).toBe(true);
  });

  it("denoiser exposes a mix param in 0..100 (Phase 10)", () => {
    expect(EFFECTS.denoiser).toBeDefined();
    const m = EFFECTS.denoiser.params.find((p) => p.key === "mix");
    expect(m).toBeDefined();
    expect(m!.min).toBe(0);
    expect(m!.max).toBe(100);
    expect(m!.unit).toBe("%");
  });

  it("denoiser sits between gate and pitch in EFFECT_ORDER", () => {
    // The placement matters: gate is a hard threshold; denoiser is a
    // learned model; both belong at the head of the chain, with the
    // denoiser running on the gated stream.
    const gateIdx = EFFECT_ORDER.indexOf("gate");
    const denoIdx = EFFECT_ORDER.indexOf("denoiser");
    const pitchIdx = EFFECT_ORDER.indexOf("pitch");
    expect(gateIdx).toBeGreaterThanOrEqual(0);
    expect(denoIdx).toBeGreaterThan(gateIdx);
    expect(pitchIdx).toBeGreaterThan(denoIdx);
  });
});

describe("fx helper", () => {
  it("seeds defaults for every param of the effect", () => {
    const e = fx("reverb", true);
    expect(e.vals).toEqual({ size: 40, mix: 25 });
  });

  it("respects overrides while preserving unspecified defaults", () => {
    const e = fx("eq", true, { low: 3 });
    expect(e.vals.low).toBe(3);
    expect(e.vals.mid).toBe(0);
    expect(e.vals.high).toBe(0);
  });
});
