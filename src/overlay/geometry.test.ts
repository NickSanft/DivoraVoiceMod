import { describe, expect, it } from "vitest";
import {
  OVERLAY_MIN_SIZE,
  isSavablePosition,
  mergeOverlayPrefs,
  placeOverlay,
  readGeometry,
  readOverlayBg,
  type MonitorInfo,
} from "./geometry";

// A common desk: a 1920×1080 laptop panel at 100 % as the primary, and a
// 4K screen at 150 % to its right. All values physical, as Tauri reports them.
const LAPTOP: MonitorInfo = { name: "\\\\.\\DISPLAY1", x: 0, y: 0, width: 1920, height: 1080, scaleFactor: 1 };
const FOUR_K: MonitorInfo = { name: "\\\\.\\DISPLAY2", x: 1920, y: 0, width: 3840, height: 2160, scaleFactor: 1.5 };
// A screen to the LEFT of the primary has negative coordinates.
const LEFT: MonitorInfo = { name: "\\\\.\\DISPLAY3", x: -2560, y: -200, width: 2560, height: 1440, scaleFactor: 1.25 };

describe("readOverlayBg", () => {
  it("keeps a known mode and defaults anything else to transparent", () => {
    expect(readOverlayBg("green")).toBe("green");
    expect(readOverlayBg("magenta")).toBe("magenta");
    expect(readOverlayBg("transparent")).toBe("transparent");
    expect(readOverlayBg(undefined)).toBe("transparent");
    expect(readOverlayBg("purple")).toBe("transparent");
    expect(readOverlayBg(3)).toBe("transparent");
    // An inherited key is not a mode.
    expect(readOverlayBg("toString")).toBe("transparent");
  });
});

describe("readGeometry", () => {
  it("a v1.16.0 { bg } blob has no geometry", () => {
    expect(readGeometry({ bg: "green" })).toBeNull();
    expect(readGeometry(null)).toBeNull();
    expect(readGeometry("x")).toBeNull();
  });

  it("reads a saved rectangle with its monitor and scale", () => {
    expect(
      readGeometry({ bg: "green", x: 2100, y: 300, w: 720, h: 720, monitor: "\\\\.\\DISPLAY2", scale: 1.5 }),
    ).toEqual({ x: 2100, y: 300, w: 720, h: 720, monitor: "\\\\.\\DISPLAY2", scale: 1.5 });
  });

  it("monitor and scale are optional", () => {
    expect(readGeometry({ x: 10, y: 20, w: 480, h: 480 })).toEqual({ x: 10, y: 20, w: 480, h: 480 });
    expect(readGeometry({ x: 10, y: 20, w: 480, h: 480, monitor: "", scale: 0 })).toEqual({
      x: 10,
      y: 20,
      w: 480,
      h: 480,
    });
  });

  it("rejects anything we would never have written", () => {
    expect(readGeometry({ x: "10", y: 20, w: 480, h: 480 })).toBeNull();
    expect(readGeometry({ x: 10, y: 20, w: 0, h: 480 })).toBeNull();
    expect(readGeometry({ x: 10, y: 20, w: 480, h: -1 })).toBeNull();
    expect(readGeometry({ x: Number.NaN, y: 20, w: 480, h: 480 })).toBeNull();
    // Windows parks minimised windows at -32000.
    expect(readGeometry({ x: -32000, y: -32000, w: 160, h: 28 })).toBeNull();
  });

  it("keeps real negative positions (a screen left of the primary)", () => {
    expect(readGeometry({ x: -2400, y: -100, w: 600, h: 600 })).toEqual({ x: -2400, y: -100, w: 600, h: 600 });
  });
});

describe("isSavablePosition", () => {
  it("refuses the minimised parking spot, accepts real coordinates", () => {
    expect(isSavablePosition(-32000, -32000)).toBe(false);
    expect(isSavablePosition(Number.NaN, 0)).toBe(false);
    expect(isSavablePosition(-2400, -100)).toBe(true);
    expect(isSavablePosition(0, 0)).toBe(true);
  });
});

describe("mergeOverlayPrefs", () => {
  it("adds geometry to a v1.16.0 { bg } blob", () => {
    expect(mergeOverlayPrefs({ bg: "green" }, "green", { x: 1, y: 2, w: 3, h: 4 })).toEqual({
      bg: "green",
      x: 1,
      y: 2,
      w: 3,
      h: 4,
    });
  });

  it("keeps keys this build doesn't know (additive-only surface)", () => {
    const merged = mergeOverlayPrefs({ bg: "green", future: { a: 1 } }, "magenta", null);
    expect(merged).toEqual({ bg: "magenta", future: { a: 1 } });
  });

  it("null geometry removes stale geometry keys rather than keeping them", () => {
    const merged = mergeOverlayPrefs(
      { bg: "green", x: 1, y: 2, w: 3, h: 4, monitor: "m", scale: 2 },
      "green",
      null,
    );
    expect(merged).toEqual({ bg: "green" });
  });

  it("replaces geometry wholesale: an old monitor name doesn't linger", () => {
    const merged = mergeOverlayPrefs(
      { bg: "green", x: 1, y: 2, w: 3, h: 4, monitor: "old", scale: 2 },
      "green",
      { x: 5, y: 6, w: 7, h: 8 },
    );
    expect(merged).toEqual({ bg: "green", x: 5, y: 6, w: 7, h: 8 });
  });

  it("starts fresh from garbage", () => {
    expect(mergeOverlayPrefs("junk", "transparent", null)).toEqual({ bg: "transparent" });
    expect(mergeOverlayPrefs([1, 2], "transparent", null)).toEqual({ bg: "transparent" });
  });

  it("round-trips through readGeometry", () => {
    const g = { x: 2100, y: 300, w: 720, h: 720, monitor: "\\\\.\\DISPLAY2", scale: 1.5 };
    expect(readGeometry(mergeOverlayPrefs({ bg: "green" }, "green", g))).toEqual(g);
  });
});

describe("placeOverlay", () => {
  it("puts it back exactly where it was when nothing changed", () => {
    const saved = { x: 2100, y: 300, w: 720, h: 720, monitor: FOUR_K.name!, scale: 1.5 };
    expect(placeOverlay(saved, [LAPTOP, FOUR_K], LAPTOP.name)).toEqual({ x: 2100, y: 300, width: 720, height: 720 });
  });

  it("no monitors: nowhere to place it", () => {
    expect(placeOverlay({ x: 0, y: 0, w: 480, h: 480 }, [])).toBeNull();
  });

  it("second screen unplugged: lands wholly on the primary", () => {
    const saved = { x: 4800, y: 1500, w: 720, h: 720, monitor: FOUR_K.name!, scale: 1.5 };
    const r = placeOverlay(saved, [LAPTOP], LAPTOP.name)!;
    // Back at 100 %: 720 physical at 150 % was 480 logical, so 480 here.
    expect(r.width).toBe(480);
    expect(r.height).toBe(480);
    expect(r.x).toBeGreaterThanOrEqual(LAPTOP.x);
    expect(r.y).toBeGreaterThanOrEqual(LAPTOP.y);
    expect(r.x + r.width).toBeLessThanOrEqual(LAPTOP.x + LAPTOP.width);
    expect(r.y + r.height).toBeLessThanOrEqual(LAPTOP.y + LAPTOP.height);
  });

  it("primary not listed first: a screen that overlaps nothing falls back to the primary", () => {
    // availableMonitors() follows EnumDisplayMonitors order and does not
    // promise the primary comes first, so "first monitor" is not "primary".
    const saved = { x: 4800, y: 1500, w: 720, h: 720, monitor: FOUR_K.name!, scale: 1.5 };
    expect(placeOverlay(saved, [LEFT, LAPTOP], LAPTOP.name)).toEqual({ x: 1440, y: 600, width: 480, height: 480 });
  });

  it("screen unplugged and another moved into its place: lands on that one", () => {
    // Three side by side; the middle one (where it was) is unplugged and
    // Windows closes the gap. The old rectangle now lies on the right one.
    const C_SHIFTED: MonitorInfo = { name: "\\\\.\\DISPLAY3", x: 1920, y: 0, width: 1920, height: 1080, scaleFactor: 1 };
    const saved = { x: 2500, y: 300, w: 480, h: 480, monitor: "\\\\.\\DISPLAY2", scale: 1 };
    expect(placeOverlay(saved, [LAPTOP, C_SHIFTED], LAPTOP.name)).toEqual({ x: 2500, y: 300, width: 480, height: 480 });
  });

  it("same monitor, display scaling changed from 100 % to 150 %: keeps its apparent size", () => {
    const saved = { x: 100, y: 100, w: 480, h: 480, monitor: LAPTOP.name!, scale: 1 };
    const laptopAt150 = { ...LAPTOP, scaleFactor: 1.5 };
    expect(placeOverlay(saved, [laptopAt150], LAPTOP.name)).toEqual({ x: 100, y: 100, width: 720, height: 720 });
  });

  it("mixed 100 %/150 %: a window on the 150 % screen stays physical-exact there", () => {
    // The bug this guards against: treating saved physical pixels as
    // logical would put a 720 px window at 1080 px, or at x = 2100 × 1.5.
    const saved = { x: 3000, y: 900, w: 720, h: 720, monitor: FOUR_K.name!, scale: 1.5 };
    expect(placeOverlay(saved, [LAPTOP, FOUR_K], LAPTOP.name)).toEqual({ x: 3000, y: 900, width: 720, height: 720 });
  });

  it("monitor renamed but still there: found by overlap instead", () => {
    const saved = { x: 3000, y: 900, w: 720, h: 720, monitor: "gone", scale: 1.5 };
    const renamed = { ...FOUR_K, name: "\\\\.\\DISPLAY9" };
    expect(placeOverlay(saved, [LAPTOP, renamed], LAPTOP.name)).toEqual({ x: 3000, y: 900, width: 720, height: 720 });
  });

  it("monitor names unavailable (null): overlap decides", () => {
    const saved = { x: 3000, y: 900, w: 720, h: 720 };
    const anon = [
      { ...LAPTOP, name: null },
      { ...FOUR_K, name: null },
    ];
    expect(placeOverlay(saved, anon, null)).toEqual({ x: 3000, y: 900, width: 720, height: 720 });
  });

  it("a screen left of the primary (negative coordinates) works", () => {
    const saved = { x: -2400, y: -100, w: 600, h: 600, monitor: LEFT.name!, scale: 1.25 };
    expect(placeOverlay(saved, [LAPTOP, LEFT], LAPTOP.name)).toEqual({ x: -2400, y: -100, width: 600, height: 600 });
  });

  it("hanging off the edge: pulled fully onto its monitor", () => {
    const saved = { x: 1700, y: 900, w: 480, h: 480, monitor: LAPTOP.name!, scale: 1 };
    expect(placeOverlay(saved, [LAPTOP, FOUR_K], LAPTOP.name)).toEqual({ x: 1440, y: 600, width: 480, height: 480 });
  });

  it("bigger than its monitor: shrunk to fit", () => {
    const saved = { x: 0, y: 0, w: 5000, h: 5000, monitor: LAPTOP.name!, scale: 1 };
    expect(placeOverlay(saved, [LAPTOP], LAPTOP.name)).toEqual({ x: 0, y: 0, width: 1920, height: 1080 });
  });

  it("never smaller than the minimum", () => {
    const saved = { x: 10, y: 10, w: 10, h: 10, monitor: LAPTOP.name!, scale: 1 };
    const r = placeOverlay(saved, [LAPTOP], LAPTOP.name)!;
    expect(r.width).toBe(OVERLAY_MIN_SIZE);
    expect(r.height).toBe(OVERLAY_MIN_SIZE);
  });

  it("no overlap, no name, no primary: first monitor", () => {
    const saved = { x: 99999, y: 99999, w: 480, h: 480 };
    const r = placeOverlay(saved, [FOUR_K, LAPTOP], null)!;
    expect(r.x + r.width).toBeLessThanOrEqual(FOUR_K.x + FOUR_K.width);
    expect(r.x).toBeGreaterThanOrEqual(FOUR_K.x);
  });

  it("results are whole pixels", () => {
    const saved = { x: 100.4, y: 100.6, w: 333, h: 333, monitor: LAPTOP.name!, scale: 1.25 };
    const r = placeOverlay(saved, [LAPTOP], LAPTOP.name)!;
    for (const v of [r.x, r.y, r.width, r.height]) expect(Number.isInteger(v)).toBe(true);
  });
});
