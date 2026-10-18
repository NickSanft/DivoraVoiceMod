import { describe, expect, it } from "vitest";
import { planFrame, type MonitorsRead } from "./frame";
import type { MonitorInfo, OverlayGeometry } from "./geometry";
import { applyOp, CIRCLE_ID, layoutScene, migrateScene, type OverlayScene, type PxRect } from "./scene";

// Physical px, as Tauri reports them: a 100 % primary and a 150 % 4K screen
// to its right.
const LAPTOP: MonitorInfo = { name: "\\\\.\\DISPLAY1", x: 0, y: 0, width: 1920, height: 1080, scaleFactor: 1 };
const FOUR_K: MonitorInfo = { name: "\\\\.\\DISPLAY2", x: 1920, y: 0, width: 3840, height: 2160, scaleFactor: 1.5 };
const BOTH: MonitorsRead = { monitors: [LAPTOP, FOUR_K], primary: LAPTOP.name };

// 480 CSS px at (120, 200) CSS inside the 4K screen.
const ON_4K: OverlayGeometry = { x: 2100, y: 300, w: 720, h: 720, monitor: FOUR_K.name!, scale: 1.5 };
const ON_LAPTOP: OverlayGeometry = { x: 100, y: 50, w: 480, h: 480, monitor: LAPTOP.name!, scale: 1 };

const windowScene = (): OverlayScene => migrateScene({ w: 480, h: 480, scale: 1 });
const monitorScene = (): OverlayScene => ({ ...windowScene(), designSize: { w: 2560, h: 1440 }, frame: "monitor" });

function near(a: PxRect, b: PxRect): void {
  for (const k of ["x", "y", "w", "h"] as const) expect(a[k], k).toBeCloseTo(b[k], 6);
}

describe("planFrame", () => {
  it("Fit on: the monitor's rect, and a reframe from the window to it", () => {
    const plan = planFrame({ scene: windowScene(), wantFit: true, monitors: BOTH, saved: ON_4K, live: null });
    expect(plan.rect).toEqual({ x: 1920, y: 0, width: 3840, height: 2160 });
    expect(plan.fit).toBe(true);
    expect(plan.viewport).toEqual({ w: 2560, h: 1440 });
    expect(plan.reframe).toMatchObject({ kind: "reframe", frame: "monitor" });
    if (plan.reframe?.kind !== "reframe") throw new Error("no reframe");
    // Closed: the window side is where the window would have been.
    near(plan.reframe.from, { x: 120, y: 200, w: 480, h: 480 });
    near(plan.reframe.to, { x: 0, y: 0, w: 2560, h: 1440 });
  });

  it("Fit on while open: the window side is the live window", () => {
    const live = { x: 2220, y: 450, width: 720, height: 720 };
    const plan = planFrame({ scene: windowScene(), wantFit: true, monitors: BOTH, saved: ON_4K, live });
    if (plan.reframe?.kind !== "reframe") throw new Error("no reframe");
    near(plan.reframe.from, { x: 200, y: 300, w: 480, h: 480 });
  });

  it("Fit wanted but no monitors: unfitted, no reframe, the frame stays the window's", () => {
    for (const monitors of [null, { monitors: [], primary: null }]) {
      const plan = planFrame({ scene: windowScene(), wantFit: true, monitors, saved: ON_4K, live: null });
      expect(plan).toEqual({ rect: null, fit: false, reframe: null, viewport: null });
    }
    // A monitor-frame scene isn't touched either until the monitors are known.
    const plan = planFrame({ scene: monitorScene(), wantFit: false, monitors: null, saved: ON_4K, live: null });
    expect(plan.reframe).toBeNull();
  });

  it("a scene saved fitted, with Fit now off: reframed from the monitor to placeOverlay's rect", () => {
    // `live` is the still-fitted window when Fit is turned off while open;
    // the target is where the window goes, not where it is.
    const fittedLive = { x: 1920, y: 0, width: 3840, height: 2160 };
    for (const live of [null, fittedLive]) {
      const plan = planFrame({ scene: monitorScene(), wantFit: false, monitors: BOTH, saved: ON_4K, live });
      expect(plan.rect).toEqual({ x: 2100, y: 300, width: 720, height: 720 });
      expect(plan.fit).toBe(false);
      expect(plan.viewport).toEqual({ w: 480, h: 480 });
      if (plan.reframe?.kind !== "reframe") throw new Error("no reframe");
      expect(plan.reframe.frame).toBe("window");
      near(plan.reframe.from, { x: 0, y: 0, w: 2560, h: 1440 });
      near(plan.reframe.to, { x: 120, y: 200, w: 480, h: 480 });
    }
  });

  it("no saved place: a 480-logical square at the monitor's top-left stands in for the window", () => {
    const fourKFirst = { monitors: [LAPTOP, FOUR_K], primary: FOUR_K.name };
    const on = planFrame({ scene: windowScene(), wantFit: true, monitors: fourKFirst, saved: null, live: null });
    expect(on.rect).toEqual({ x: 1920, y: 0, width: 3840, height: 2160 });
    if (on.reframe?.kind !== "reframe") throw new Error("no reframe");
    near(on.reframe.from, { x: 0, y: 0, w: 480, h: 480 });
    const off = planFrame({ scene: monitorScene(), wantFit: false, monitors: fourKFirst, saved: null, live: null });
    // Windows picks the spot, so there is no rect and no viewport to expect.
    expect(off.rect).toBeNull();
    expect(off.viewport).toBeNull();
    if (off.reframe?.kind !== "reframe") throw new Error("no reframe");
    near(off.reframe.to, { x: 0, y: 0, w: 480, h: 480 });
  });

  it("no reframe when the scene is already in the target's frame", () => {
    const off = planFrame({ scene: windowScene(), wantFit: false, monitors: BOTH, saved: ON_4K, live: null });
    expect(off).toEqual({
      rect: { x: 2100, y: 300, width: 720, height: 720 },
      fit: false,
      reframe: null,
      viewport: { w: 480, h: 480 },
    });
    const on = planFrame({ scene: monitorScene(), wantFit: true, monitors: BOTH, saved: ON_4K, live: null });
    expect(on.fit).toBe(true);
    expect(on.reframe).toBeNull();
  });

  it("applying the planned reframe leaves the circle where it was on screen", () => {
    const scene = windowScene();
    const plan = planFrame({ scene, wantFit: true, monitors: BOTH, saved: ON_LAPTOP, live: null });
    expect(plan.rect).toEqual({ x: 0, y: 0, width: 1920, height: 1080 });
    const next = applyOp(scene, plan.reframe!);
    const circle = layoutScene(next, plan.viewport!).find((r) => r.el.id === CIRCLE_ID)!.rect;
    // The window was at (100, 50); the circle at (20, 20) inside it.
    for (const [k, v] of [
      ["x", 120],
      ["y", 70],
      ["w", 440],
      ["h", 440],
    ] as const) {
      expect(circle[k]).toBeCloseTo(v, 2);
    }
  });
});
