import { describe, expect, it } from "vitest";
import {
  ampToDb,
  computeCalibration,
  GATE_MAX_DB,
  GATE_MIN_DB,
  percentile,
} from "./calibration";

describe("calibration math", () => {
  it("ampToDb converts amplitude to dBFS and floors silence", () => {
    expect(ampToDb(1)).toBeCloseTo(0, 5);
    expect(ampToDb(0.1)).toBeCloseTo(-20, 5);
    expect(ampToDb(0.01)).toBeCloseTo(-40, 5);
    expect(ampToDb(0)).toBe(-120);
    expect(ampToDb(-5)).toBe(-120); // defensive
  });

  it("percentile takes the nearest-rank value", () => {
    expect(percentile([], 0.9)).toBe(0);
    expect(percentile([0.1], 0.9)).toBe(0.1);
    expect(percentile([1, 2, 3, 4, 5], 0)).toBe(1);
    expect(percentile([1, 2, 3, 4, 5], 1)).toBe(5);
    expect(percentile([5, 1, 3, 2, 4], 0.5)).toBe(3); // sorts first
  });

  it("suggests gate ≈ floor + 8 dB from a steady level series", () => {
    // A −40 dB room (amp 0.01): floor −40 → gate −32.
    const samples = Array.from({ length: 40 }, () => 0.01);
    const r = computeCalibration(samples);
    expect(r.noiseFloorDb).toBeCloseTo(-40, 0);
    expect(r.gateThreshDb).toBe(-32);
    expect(r.sampleCount).toBe(40);
  });

  it("clamps the gate suggestion to the gate's range", () => {
    // Silence → floor −120 → would be −112 → clamp to GATE_MIN.
    expect(computeCalibration([0, 0, 0]).gateThreshDb).toBe(GATE_MIN_DB);
    // Very loud "quiet" (amp 0.9, ≈ −0.9 dB) → +7 → clamp to GATE_MAX.
    expect(computeCalibration([0.9, 0.9, 0.9]).gateThreshDb).toBe(GATE_MAX_DB);
  });

  it("only suggests denoiser when the floor is high", () => {
    // Clean room (−60 dB, amp 0.001) → 0% denoiser.
    expect(computeCalibration(Array(20).fill(0.001)).denoiserMix).toBe(0);
    // Noisy room (−35 dB, amp ≈ 0.0178) → maxed at 90%.
    expect(computeCalibration(Array(20).fill(0.0178)).denoiserMix).toBe(90);
    // Mid (−47.5 dB) sits between.
    const mid = computeCalibration(Array(20).fill(10 ** (-47.5 / 20))).denoiserMix;
    expect(mid).toBeGreaterThan(0);
    expect(mid).toBeLessThan(90);
  });

  it("the 90th percentile ignores a few loud spikes", () => {
    // 38 quiet samples + 2 loud spikes: the floor stays near the quiet level.
    const samples = [...Array(38).fill(0.01), 0.8, 0.9];
    const r = computeCalibration(samples);
    expect(r.noiseFloorDb).toBeCloseTo(-40, 0);
  });
});
