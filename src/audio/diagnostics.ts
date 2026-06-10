// v1.13.0: "Test my setup" diagnostic.
//
// Pure evaluation of a routing-chain snapshot into a green/amber/red
// checklist. Kept free of store/Tauri coupling (like calibration.ts) so
// it unit-tests in isolation; the store gathers the snapshot (refreshing
// devices + VB-Cable, briefly starting the engine) and feeds it in.

export type DiagStatus = "pass" | "warn" | "fail";

export interface DiagCheck {
  id: string;
  label: string;
  status: DiagStatus;
  detail: string;
}

/** Everything the evaluator needs, captured by the store at run time. */
export interface DiagSnapshot {
  inputCount: number;
  outputCount: number;
  selectedInput: string | null;
  selectedOutput: string | null;
  /** True if the engine is running cleanly (started for the test or already). */
  engineStarted: boolean;
  engineError: string | null;
  /** True if the OUT meter moved during the short test window. */
  signalFlowing: boolean;
  cableDetected: boolean;
  /** The VB-Cable input device name (what other apps listen to), if known. */
  cableInputName: string | null;
}

/**
 * Turn a snapshot into an ordered checklist. Objective checks only —
 * "can you actually hear it" stays the user's ear (surfaced as guidance
 * on the signal row, not a hard pass/fail).
 */
export function evaluateDiagnostics(s: DiagSnapshot): DiagCheck[] {
  const checks: DiagCheck[] = [];

  // 1. Microphone
  if (s.inputCount === 0) {
    checks.push({ id: "input", label: "Microphone detected", status: "fail", detail: "No input devices found — plug in a microphone." });
  } else if (!s.selectedInput) {
    checks.push({ id: "input", label: "Microphone selected", status: "warn", detail: "Pick an input device in Audio devices." });
  } else {
    checks.push({ id: "input", label: "Microphone", status: "pass", detail: s.selectedInput });
  }

  // 2. Output
  if (s.outputCount === 0) {
    checks.push({ id: "output", label: "Output device detected", status: "fail", detail: "No output devices found." });
  } else if (!s.selectedOutput) {
    checks.push({ id: "output", label: "Output selected", status: "warn", detail: "Pick an output device in Audio devices." });
  } else {
    checks.push({ id: "output", label: "Output", status: "pass", detail: s.selectedOutput });
  }

  // 3. Engine
  if (s.engineError) {
    checks.push({ id: "engine", label: "Audio engine", status: "fail", detail: s.engineError });
  } else if (!s.engineStarted) {
    checks.push({ id: "engine", label: "Audio engine", status: "warn", detail: "Couldn't start — select an input and output first." });
  } else {
    checks.push({ id: "engine", label: "Audio engine starts", status: "pass", detail: "Started cleanly." });
  }

  // 4. Signal flow (only meaningful once the engine is up)
  if (s.engineStarted && !s.engineError) {
    checks.push({
      id: "signal",
      label: "Signal reaches the output",
      status: s.signalFlowing ? "pass" : "warn",
      detail: s.signalFlowing
        ? "Output level moved during the test."
        : "No output level detected — speak into the mic and run it again (and check the chain isn't muted).",
    });
  }

  // 5. VB-Cable present
  checks.push({
    id: "cable",
    label: "VB-Cable installed",
    status: s.cableDetected ? "pass" : "warn",
    detail: s.cableDetected
      ? "Other apps can use it as a microphone."
      : "Not detected — install VB-Cable to send your modulated voice to Discord / Zoom / OBS.",
  });

  // 6. Output routed to the cable (only when the cable exists)
  if (s.cableDetected) {
    const routed = !!s.cableInputName && s.selectedOutput === s.cableInputName;
    checks.push({
      id: "routing",
      label: "Output routed to VB-Cable",
      status: routed ? "pass" : "warn",
      detail: routed
        ? "Other apps will receive your modulated voice."
        : `Set Output to "${s.cableInputName ?? "CABLE Input"}" so other apps receive your voice (you'll still hear yourself via the Monitor device).`,
    });
  }

  return checks;
}

/** Worst status across the checks — drives the summary headline. */
export function overallStatus(checks: DiagCheck[]): DiagStatus {
  if (checks.some((c) => c.status === "fail")) return "fail";
  if (checks.some((c) => c.status === "warn")) return "warn";
  return "pass";
}
