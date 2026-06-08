// v1.10.0: guided mic calibration.
//
// Pure math for turning a short "stay quiet" capture of input levels into
// a sensible noise gate (and denoiser) suggestion. Kept free of any store
// / Tauri coupling so it unit-tests in isolation; the store samples the
// existing ~30 Hz `audio-levels` stream and feeds the RMS series in.

/** Gate threshold range — mirrors `EFFECTS.gate` in `src/data/effects.ts`
 *  (the param the suggestion writes to). */
export const GATE_MIN_DB = -80;
export const GATE_MAX_DB = -20;

/** Headroom above the measured floor so the gate opens for the voice but
 *  not the room. */
export const GATE_HEADROOM_DB = 8;

export interface CalibrationResult {
  /** Measured noise floor in dBFS (a high percentile of the RMS series). */
  noiseFloorDb: number;
  /** Suggested gate threshold in dB, clamped to the gate's range. */
  gateThreshDb: number;
  /** Suggested denoiser mix (0..100); 0 for a quiet room, scaling up as
   *  the floor rises. */
  denoiserMix: number;
  /** How many level samples the measurement used. */
  sampleCount: number;
}

/** Convert a linear amplitude (0..1) to dBFS, flooring tiny / zero values
 *  to a finite −120 dB so `log10(0)` never leaks `-Infinity`. */
export function ampToDb(amp: number): number {
  if (!(amp > 1e-6)) return -120; // also catches NaN / negatives
  return 20 * Math.log10(amp);
}

/** The p-th percentile (p in 0..1) of a numeric series, nearest-rank. */
export function percentile(values: number[], p: number): number {
  if (values.length === 0) return 0;
  const sorted = [...values].sort((a, b) => a - b);
  const clampedP = Math.min(1, Math.max(0, p));
  const idx = Math.round(clampedP * (sorted.length - 1));
  return sorted[idx]!;
}

function clamp(v: number, lo: number, hi: number): number {
  return Math.min(hi, Math.max(lo, v));
}

/**
 * Compute a gate + denoiser suggestion from a series of input RMS samples
 * (each 0..1) captured during a "stay quiet" window.
 *
 * - Noise floor = the 90th percentile of the series (the loud end of
 *   "quiet" — robust to the odd spike).
 * - Gate threshold = floor + headroom, clamped to the gate's range.
 * - Denoiser mix scales from 0 (floor ≤ −60 dB, a clean room) up to 90%
 *   (floor ≥ −35 dB, a noisy one).
 */
export function computeCalibration(rmsSamples: number[]): CalibrationResult {
  const floorAmp = percentile(rmsSamples, 0.9);
  const noiseFloorDb = ampToDb(floorAmp);
  const gateThreshDb = Math.round(
    clamp(noiseFloorDb + GATE_HEADROOM_DB, GATE_MIN_DB, GATE_MAX_DB),
  );
  const denoiserMix = Math.round(
    clamp(((noiseFloorDb + 60) / 25) * 90, 0, 90),
  );
  return { noiseFloorDb, gateThreshDb, denoiserMix, sampleCount: rmsSamples.length };
}
