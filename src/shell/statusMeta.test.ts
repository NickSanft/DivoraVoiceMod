import { describe, expect, it } from "vitest";
import { statusMeta } from "./statusMeta";

describe("statusMeta", () => {
  it("returns Clean copy for clean status", () => {
    const m = statusMeta("clean");
    expect(m.label).toBe("Clean");
    expect(m.sigil).toBe("clean");
    expect(m.bg).toBe("transparent");
  });

  it("returns Modulated copy for modulated status", () => {
    const m = statusMeta("modulated");
    expect(m.label).toBe("Modulated");
    expect(m.sigil).toBe("modulated");
    expect(m.color).toContain("indigo");
  });

  it("returns Muted copy for muted status", () => {
    const m = statusMeta("muted");
    expect(m.label).toBe("Muted");
    expect(m.sigil).toBe("muted");
    expect(m.color).toContain("danger");
  });

  // v1.56.0: Mute silences the voice only; "Nothing is being sent" was false
  // while a clip played.
  it("says the voice is off and clips still play while muted", () => {
    expect(statusMeta("muted").sub).toBe("Your voice is off · clips and Speak still play");
  });
});
