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
