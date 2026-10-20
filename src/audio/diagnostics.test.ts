import { describe, expect, it } from "vitest";
import {
  evaluateDiagnostics,
  overallStatus,
  type DiagCheck,
  type DiagSnapshot,
} from "./diagnostics";

const CABLE = "CABLE Input (VB-Audio Virtual Cable)";

const GOOD: DiagSnapshot = {
  inputCount: 1,
  outputCount: 1,
  selectedInput: "Mock Mic",
  selectedOutput: CABLE,
  engineStarted: true,
  engineError: null,
  signalFlowing: true,
  cableDetected: true,
  cableInputName: CABLE,
};

const by = (checks: DiagCheck[], id: string): DiagCheck | undefined =>
  checks.find((c) => c.id === id);

describe("evaluateDiagnostics", () => {
  it("is all-green when everything is set up + routed", () => {
    const checks = evaluateDiagnostics(GOOD);
    expect(checks.every((c) => c.status === "pass")).toBe(true);
    expect(overallStatus(checks)).toBe("pass");
    expect(by(checks, "routing")?.status).toBe("pass");
  });

  it("fails when there are no input devices", () => {
    const checks = evaluateDiagnostics({
      ...GOOD,
      inputCount: 0,
      selectedInput: null,
    });
    expect(by(checks, "input")?.status).toBe("fail");
    expect(overallStatus(checks)).toBe("fail");
  });

  it("warns when devices exist but none are selected", () => {
    const checks = evaluateDiagnostics({
      ...GOOD,
      selectedInput: null,
      selectedOutput: null,
      cableDetected: false,
      cableInputName: null,
    });
    expect(by(checks, "input")?.status).toBe("warn");
    expect(by(checks, "output")?.status).toBe("warn");
  });

  it("fails the engine check on an engine error and drops the signal row", () => {
    const checks = evaluateDiagnostics({
      ...GOOD,
      engineError: "sample-rate mismatch",
    });
    expect(by(checks, "engine")?.status).toBe("fail");
    expect(by(checks, "engine")?.detail).toContain("sample-rate");
    expect(by(checks, "signal")).toBeUndefined();
  });

  it("warns the engine check when it didn't start (no error)", () => {
    const checks = evaluateDiagnostics({ ...GOOD, engineStarted: false });
    expect(by(checks, "engine")?.status).toBe("warn");
    expect(by(checks, "signal")).toBeUndefined();
  });

  it("warns the signal row when no output level moved", () => {
    const checks = evaluateDiagnostics({ ...GOOD, signalFlowing: false });
    expect(by(checks, "signal")?.status).toBe("warn");
    expect(overallStatus(checks)).toBe("warn");
  });

  // v1.56.0: when Divora's own mute is why nothing reached the output, the
  // signal row says so instead of asking the user to speak up.
  it("names Divora's mute when no signal moved while muted", () => {
    const muted = by(evaluateDiagnostics({ ...GOOD, signalFlowing: false, muted: true }), "signal");
    expect(muted?.status).toBe("warn");
    expect(muted?.detail).toBe("Your voice is muted in Divora. Unmute it, then run the test again.");

    const today = by(evaluateDiagnostics({ ...GOOD, signalFlowing: false, muted: false }), "signal");
    expect(today?.status).toBe("warn");
    expect(today?.detail).toBe(
      "No output level detected — speak into the mic and run it again (with no separate monitor device, also check the Monitor toggle is on).",
    );
    // An older snapshot without the field reads as not muted.
    expect(by(evaluateDiagnostics({ ...GOOD, signalFlowing: false }), "signal")?.detail).toBe(today?.detail);

    // A clip can still move the meter while muted: then it passes, as today.
    const flowing = by(evaluateDiagnostics({ ...GOOD, signalFlowing: true, muted: true }), "signal");
    expect(flowing?.status).toBe("pass");
    expect(flowing?.detail).toBe("Output level moved during the test.");
  });

  it("warns + omits routing when VB-Cable isn't detected", () => {
    const checks = evaluateDiagnostics({
      ...GOOD,
      cableDetected: false,
      cableInputName: null,
    });
    expect(by(checks, "cable")?.status).toBe("warn");
    expect(by(checks, "routing")).toBeUndefined();
  });

  it("warns routing when the output isn't the cable input", () => {
    const checks = evaluateDiagnostics({ ...GOOD, selectedOutput: "Speakers" });
    expect(by(checks, "routing")?.status).toBe("warn");
    expect(by(checks, "routing")?.detail).toContain("CABLE Input");
    expect(overallStatus(checks)).toBe("warn");
  });
});
