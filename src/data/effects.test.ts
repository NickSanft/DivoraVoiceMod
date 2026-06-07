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

  it("voice_convert exposes a mix param in 0..100 (Phase 12)", () => {
    expect(EFFECTS.voice_convert).toBeDefined();
    const m = EFFECTS.voice_convert.params.find((p) => p.key === "mix");
    expect(m).toBeDefined();
    expect(m!.min).toBe(0);
    expect(m!.max).toBe(100);
    expect(m!.unit).toBe("%");
  });

  it("compressor exposes the standard dynamics params (v1.8.0)", () => {
    expect(EFFECTS.compressor).toBeDefined();
    const keys = EFFECTS.compressor.params.map((p) => p.key);
    expect(keys).toEqual(["thresh", "ratio", "attack", "release", "makeup"]);
    const ratio = EFFECTS.compressor.params.find((p) => p.key === "ratio");
    expect(ratio!.min).toBe(1);
    expect(ratio!.max).toBe(20);
  });

  it("de-esser exposes freq/thresh/range params (v1.8.0)", () => {
    expect(EFFECTS.deesser).toBeDefined();
    const keys = EFFECTS.deesser.params.map((p) => p.key);
    expect(keys).toEqual(["freq", "thresh", "range"]);
    const freq = EFFECTS.deesser.params.find((p) => p.key === "freq");
    expect(freq!.unit).toBe("Hz");
    expect(freq!.default).toBe(6000);
  });

  it("compressor and de-esser sit in the corrective group after EQ", () => {
    // Dynamics belong with the corrective effects (after tone-shaping
    // EQ), before the creative tail (robot/distortion/echo/reverb).
    const eqIdx = EFFECT_ORDER.indexOf("eq");
    const compIdx = EFFECT_ORDER.indexOf("compressor");
    const deessIdx = EFFECT_ORDER.indexOf("deesser");
    const robotIdx = EFFECT_ORDER.indexOf("robot");
    expect(compIdx).toBeGreaterThan(eqIdx);
    expect(deessIdx).toBeGreaterThan(compIdx);
    expect(robotIdx).toBeGreaterThan(deessIdx);
  });

  it("voice_convert runs after the cleanup effects but before pitch", () => {
    // Voice conversion wants a clean, gated, denoised signal as input;
    // it sits after gate + denoiser. Downstream tone-shaping (pitch,
    // eq, reverb) then colours the converted voice.
    const denoIdx = EFFECT_ORDER.indexOf("denoiser");
    const vcIdx = EFFECT_ORDER.indexOf("voice_convert");
    const pitchIdx = EFFECT_ORDER.indexOf("pitch");
    expect(vcIdx).toBeGreaterThan(denoIdx);
    expect(pitchIdx).toBeGreaterThan(vcIdx);
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
