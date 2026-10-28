import { createEffect, createRoot } from "solid-js";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { ImportedAsset } from "../audio/api";
import {
  createOverlayController,
  OVERLAY_PREFS_KEY as PREFS,
  OVERLAY_SCENE_KEY as SCENE,
  type OverlayControllerDeps,
} from "./controller";
import { beginMove, finishGesture, updateGesture } from "./editor";
import type { MonitorsRead } from "./frame";
import type { MonitorInfo, OverlayGeometry, Rect } from "./geometry";
import { emptyMirror, proposeEdit, receiveScene, renderedScene } from "./mirror";
import {
  CIRCLE_ID,
  defaultTimerRect,
  frameOf,
  layoutScene,
  liveElements,
  MAX_ELEMENTS,
  migrateScene,
  type OverlayScene,
  type PxRect,
  type SceneElement,
} from "./scene";
import { OVERLAY_READY_EVENT, OVERLAY_SCENE_EVENT, type ScenePush } from "./state";
import type { OverlayStatus } from "./status";
import fixture from "./wire.fixture.json";

// Drives the real controller with fake deps: an in-memory backend that
// answers like overlay_machine.rs, an in-memory localStorage, and a window
// whose rectangle the test sets. Physical px: one 100 % monitor.

const LAPTOP: MonitorInfo = { name: "\\\\.\\DISPLAY1", x: 0, y: 0, width: 1920, height: 1080, scaleFactor: 1 };
const MONITORS: MonitorsRead = { monitors: [LAPTOP], primary: LAPTOP.name };
const SAVED: OverlayGeometry = { x: 100, y: 50, w: 480, h: 480, monitor: LAPTOP.name!, scale: 1 };
const SAVED_RECT: Rect = { x: 100, y: 50, width: 480, height: 480 };
const FIT_RECT: Rect = { x: 0, y: 0, width: 1920, height: 1080 };
const V480 = { w: 480, h: 480 };

const CLOSED = fixture.statusSamples.closed as OverlayStatus;
const WATCHDOG = fixture.statusSamples.watchdog as OverlayStatus;
const SAMPLE = fixture.importedAssetSample as ImportedAsset;
const st = (more: Partial<OverlayStatus> = {}): OverlayStatus => ({ ...CLOSED, ...more });
const PAINTED = st({ open: true, phase: "painted", instance: 1, canArrange: true });

const LEASE_NEVER = "The overlay didn't respond, so it stayed locked.";
const STARTING = "The overlay is still starting. Try again in a moment.";

const flush = async (): Promise<void> => {
  for (let i = 0; i < 20; i++) await Promise.resolve();
  await new Promise<void>((r) => setTimeout(r, 0));
};

/** Within a thousandth of a pixel: stored fractions are rounded to 6
 *  decimals, so a 480 px window is exact only to about 0.0005 px. */
function near(a: PxRect, b: PxRect): void {
  for (const k of ["x", "y", "w", "h"] as const) expect(Math.abs(a[k] - b[k]), k).toBeLessThan(1e-3);
}

function deferred<T>(): { promise: Promise<T>; resolve: (v: T) => void; reject: (e: unknown) => void } {
  let resolve!: (v: T) => void;
  let reject!: (e: unknown) => void;
  const promise = new Promise<T>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

/** A picture element of the migrated 480 scene. */
function picture(id: string, src: string, more: Record<string, unknown> = {}): SceneElement {
  return { id, type: "image", x: 0.6, y: 0.6, w: 0.3, h: 0.168675, z: 1, src, nw: 498, nh: 280, ...more };
}

/** The migrated 480 scene plus these elements. */
function sceneWith(...extra: SceneElement[]): OverlayScene {
  const s = migrateScene({ ...SAVED });
  return { ...s, elements: [...s.elements, ...extra] };
}

interface Options {
  stored?: Record<string, unknown>;
  /** Share storage with an earlier harness: a restart. */
  store?: Map<string, string>;
  monitors?: MonitorsRead | null;
  /** The backend's status at launch: an overlay left open by a reload. */
  backend?: OverlayStatus;
  /** What `overlay_status` answers at launch, when not simply the backend's. */
  pull?: () => Promise<unknown>;
}

const disposers: Array<() => void> = [];
afterEach(() => {
  for (const d of disposers.splice(0)) d();
  vi.restoreAllMocks();
});

function harness(o: Options = {}) {
  const store = o.store ?? new Map<string, string>();
  for (const [k, v] of Object.entries(o.stored ?? {})) store.set(k, JSON.stringify(v));
  const log: string[] = [];
  const toasts: Array<{ text: string; tone?: string }> = [];
  const pushes: ScenePush[] = [];
  const listeners = new Map<string, Array<(p: unknown) => void>>();
  let onStatus: ((raw: unknown) => void) | null = null;
  let onEdit: ((raw: unknown) => void) | null = null;
  const env = {
    monitors: o.monitors === undefined ? MONITORS : o.monitors,
    live: null as Rect | null,
    placeError: null as string | null,
    tracks: 0,
    keepers: 0,
    trackSettled: null as ((g: OverlayGeometry) => void) | null,
    fitSettled: null as (() => void) | null,
    /** A move still waiting to settle; `flush` saves it. */
    pending: null as OverlayGeometry | null,
    /** The next monitors read throws outright (a bug, not a refusal). */
    monitorsThrow: false,
  };
  // The backend's status; each command answers with it, as lib.rs does.
  let backend = o.backend ?? st();
  const answer = (s: OverlayStatus): OverlayStatus => {
    backend = s;
    return s;
  };
  const api = {
    open: vi.fn(async (_rect: Rect | null, fit: boolean) => {
      log.push(`open:${fit}`);
      return answer(st({ open: true, phase: "starting", instance: backend.instance + 1, fit }));
    }),
    close: vi.fn(async () => {
      log.push("close");
      return answer(st({ instance: backend.instance }));
    }),
    place: vi.fn(async (rect: Rect, fit: boolean) => {
      log.push(`place:${fit}`);
      if (env.placeError) throw env.placeError;
      env.live = rect;
      return answer({ ...backend, fit });
    }),
    setMode: vi.fn(async (_m: "locked" | "arrange" | "toggle"): Promise<unknown> => backend),
    setHide: vi.fn(async (on: boolean) => answer({ ...backend, hideWanted: on })),
    status: vi.fn(async () => backend),
    importAsset: vi.fn(async (_path: string): Promise<ImportedAsset> => ({ ...SAMPLE })),
    deleteAsset: vi.fn(async (_id: string): Promise<void> => undefined),
    pickImage: vi.fn(async (): Promise<string | null> => "C:\\pics\\wave.gif"),
    subscribeStatus: async (h: (raw: unknown) => void) => {
      onStatus = h;
      return () => {
        onStatus = null;
      };
    },
    subscribeEdit: async (h: (raw: unknown) => void) => {
      onEdit = h;
      return () => {
        onEdit = null;
      };
    },
  };
  if (o.pull) api.status.mockImplementation(o.pull as () => Promise<OverlayStatus>);
  let ids = 0;
  const deps: OverlayControllerDeps = {
    api,
    events: {
      emit: async (event, payload) => {
        if (event !== OVERLAY_SCENE_EVENT) return;
        // Through JSON, as over the wire.
        const p = JSON.parse(JSON.stringify(payload)) as ScenePush;
        pushes.push(p);
        log.push(`push:${p.revision}${p.expectViewport ? ":expect" : ""}`);
      },
      listen: async (event, h) => {
        listeners.set(event, [...(listeners.get(event) ?? []), h]);
        return () => undefined;
      },
    },
    windows: {
      monitors: () => {
        if (env.monitorsThrow) {
          env.monitorsThrow = false;
          throw new Error("boom");
        }
        return Promise.resolve(env.monitors);
      },
      liveRect: async () => env.live,
      track: async (onSettled) => {
        log.push("track");
        env.tracks += 1;
        env.trackSettled = onSettled;
        return {
          stop: () => {
            env.trackSettled = null;
          },
          flush: async () => {
            const g = env.pending;
            env.pending = null;
            if (g) onSettled(g);
          },
        };
      },
      keepFit: async (onSettled) => {
        env.keepers += 1;
        env.fitSettled = onSettled;
        return () => {
          env.fitSettled = null;
        };
      },
    },
    storage: {
      load: (k) => {
        const raw = store.get(k);
        return raw === undefined ? null : JSON.parse(raw);
      },
      save: (k, v) => {
        log.push(`save:${k}`);
        store.set(k, JSON.stringify(v));
      },
    },
    toast: (text, tone) => toasts.push({ text, tone }),
    newId: () => `c-${(ids++).toString(16).padStart(8, "0")}`,
    session: "s1",
  };
  let dispose: () => void = () => undefined;
  const c = createRoot((d) => {
    dispose = d;
    return createOverlayController(deps);
  });
  disposers.push(dispose);
  return {
    c,
    api,
    env,
    log,
    toasts,
    pushes,
    store,
    dispose,
    emitStatus: (raw: unknown) => onStatus?.(raw),
    edit: (raw: unknown) => onEdit?.(raw),
    fire: (event: string) => {
      for (const h of listeners.get(event) ?? []) h(undefined);
    },
    loaded: (k: string) => JSON.parse(store.get(k)!) as Record<string, unknown> & OverlayScene,
    saves: (k: string) => log.filter((l) => l === `save:${k}`).length,
  };
}

const prefs = (more: Record<string, unknown> = {}) => ({ [PREFS]: { bg: "transparent", ...SAVED, ...more } });

describe("overlay controller — where the window goes (v1.55.0)", () => {
  it("open passes placeOverlay's rect, fitRect when fitted, and null when the monitors can't be read", async () => {
    const plain = harness({ stored: prefs() });
    await flush();
    await plain.c.openOverlay();
    expect(plain.api.open).toHaveBeenLastCalledWith(SAVED_RECT, false);

    const fitted = harness({ stored: prefs({ fit: true }) });
    await flush();
    await fitted.c.openOverlay();
    expect(fitted.api.open).toHaveBeenLastCalledWith(FIT_RECT, true);

    const blind = harness({ stored: prefs({ fit: true }), monitors: null });
    await flush();
    await blind.c.openOverlay();
    expect(blind.api.open).toHaveBeenLastCalledWith(null, false);
  });

  it("a second click while one opens finds it open", async () => {
    const h = harness({ stored: prefs() });
    await flush();
    await Promise.all([h.c.openOverlay(), h.c.openOverlay()]);
    expect(h.api.open).toHaveBeenCalledTimes(1);
  });

  it("a stored Fit with Green opens unfitted, reads as off, and writes nothing at boot", async () => {
    const h = harness({ stored: prefs({ bg: "green", fit: true }) });
    await flush();
    expect(h.c.overlayFit()).toBe(false);
    expect(h.c.overlayBg()).toBe("green");
    expect(h.log.filter((l) => l.startsWith("save:"))).toEqual([]);
    await h.c.openOverlay();
    expect(h.api.open).toHaveBeenLastCalledWith(SAVED_RECT, false);
  });

  it("Fit on without readable monitors: says so and stays off", async () => {
    for (const monitors of [null, { monitors: [], primary: null }]) {
      const h = harness({ stored: prefs(), monitors });
      await flush();
      await h.c.setOverlayFit(true);
      expect(h.toasts).toEqual([{ text: "Couldn't read your monitors.", tone: "error" }]);
      expect(h.c.overlayFit()).toBe(false);
      expect(h.saves(PREFS)).toBe(0);
      await h.c.openOverlay();
      expect(h.api.open).toHaveBeenLastCalledWith(null, false);
    }
  });

  it("Fit changed while closed is reconciled at the next open, before the window exists", async () => {
    const h = harness({ stored: prefs() });
    await flush();
    await h.c.setOverlayFit(true);
    // Closed: only the flag is saved.
    expect(h.loaded(PREFS).fit).toBe(true);
    expect(h.store.has(SCENE)).toBe(false);

    await h.c.openOverlay();
    const saved = h.log.indexOf(`save:${SCENE}`);
    expect(saved).toBeGreaterThanOrEqual(0);
    expect(saved).toBeLessThan(h.log.indexOf("open:true"));
    const fitted = h.loaded(SCENE);
    expect(fitted.frame).toBe("monitor");
    // The 480 window at (100, 50) drew the 440 circle 20 px in: on the
    // monitor it is still there.
    near(layoutScene(fitted, { w: 1920, h: 1080 })[0]!.rect, { x: 120, y: 70, w: 440, h: 440 });

    // Off while closed: back from the monitor to the saved place.
    await h.c.closeOverlay();
    await h.c.setOverlayFit(false);
    await h.c.openOverlay();
    expect(h.api.open).toHaveBeenLastCalledWith(SAVED_RECT, false);
    const back = h.loaded(SCENE);
    expect(back.frame).toBe("window");
    near(layoutScene(back, V480)[0]!.rect, { x: 20, y: 20, w: 440, h: 440 });
  });

  it("Fit on while open: the reframed scene goes first, then the place, then the saves", async () => {
    const h = harness({ stored: prefs() });
    await flush();
    await h.c.openOverlay();
    await flush();
    h.env.live = SAVED_RECT;
    h.log.length = 0;

    await h.c.setOverlayFit(true);
    expect(h.log.indexOf("push:1:expect")).toBeGreaterThanOrEqual(0);
    expect(h.log.indexOf("push:1:expect")).toBeLessThan(h.log.indexOf("place:true"));
    expect(h.log.indexOf("place:true")).toBeLessThan(h.log.indexOf(`save:${SCENE}`));
    expect(h.api.place).toHaveBeenLastCalledWith(FIT_RECT, true);
    expect(h.pushes.at(-1)!.expectViewport).toEqual({ w: 1920, h: 1080 });
    expect(h.loaded(SCENE).frame).toBe("monitor");
    expect(h.loaded(PREFS).fit).toBe(true);
    // The keeper replaces geometry tracking.
    expect(h.env.keepers).toBe(1);
    expect(h.env.trackSettled).toBeNull();
  });

  it("fitted since a fresh install, Fit off goes to the place the scene was reframed to", async () => {
    // No saved geometry: the window was never anywhere but fitted.
    const h = harness({ stored: { [PREFS]: { bg: "transparent", fit: true } } });
    await flush();
    await h.c.openOverlay();
    expect(h.api.open).toHaveBeenLastCalledWith(FIT_RECT, true);
    h.env.live = FIT_RECT;
    await h.c.setOverlayFit(false);
    // planFrame's stand-in: 480 logical px at the monitor's top-left.
    expect(h.api.place).toHaveBeenLastCalledWith({ x: 0, y: 0, width: 480, height: 480 }, false);
    expect(h.pushes.find((p) => p.expectViewport)?.expectViewport).toEqual(V480);
    expect(h.c.overlayFit()).toBe(false);
    expect(h.loaded(SCENE).frame).toBe("window");
    near(layoutScene(h.c.overlayScene(), V480)[0]!.rect, { x: 20, y: 20, w: 440, h: 440 });
  });

  it("a refused place leaves the saved scene, the flag and the frame, and re-pushes the old scene", async () => {
    const h = harness({ stored: prefs() });
    await flush();
    await h.c.openOverlay();
    await flush();
    const savedScene = h.store.get(SCENE);
    const savedPrefs = h.store.get(PREFS);
    const old = h.c.overlayScene();
    h.env.placeError = "The overlay isn't open.";

    await h.c.setOverlayFit(true);
    await flush();
    expect(h.store.get(SCENE)).toBe(savedScene);
    expect(h.store.get(PREFS)).toBe(savedPrefs);
    expect(h.c.overlayFit()).toBe(false);
    expect(frameOf(h.c.overlayScene())).toBe("window");
    expect(h.toasts.at(-1)).toEqual({ text: "The overlay isn't open.", tone: "error" });
    const [reframed, rolledBack] = h.pushes.slice(-2);
    expect(reframed!.expectViewport).not.toBeNull();
    expect(rolledBack!.revision).toBeGreaterThan(reframed!.revision);
    expect(rolledBack!.expectViewport).toBeNull();
    expect(rolledBack!.scene).toEqual(JSON.parse(JSON.stringify(old)));
  });

  it("two quick Fit toggles place in order, and the frame ends as the flag says", async () => {
    const h = harness({ stored: prefs() });
    await flush();
    await h.c.openOverlay();
    h.env.live = SAVED_RECT;
    await Promise.all([h.c.setOverlayFit(true), h.c.setOverlayFit(false)]);
    expect(h.api.place.mock.calls.map((c) => c[1])).toEqual([true, false]);
    expect(h.api.place).toHaveBeenLastCalledWith(SAVED_RECT, false);
    expect(h.c.overlayFit()).toBe(false);
    expect(h.loaded(SCENE).frame).toBe("window");
    near(layoutScene(h.c.overlayScene(), V480)[0]!.rect, { x: 20, y: 20, w: 440, h: 440 });
  });

  it("the fit keeper re-fits a window a scale change resized, and settles", async () => {
    const h = harness({ stored: prefs({ fit: true }) });
    await flush();
    await h.c.openOverlay();
    await flush();
    expect(h.env.keepers).toBe(1);
    expect(h.env.tracks).toBe(0);

    h.env.live = FIT_RECT;
    h.env.fitSettled!();
    await flush();
    expect(h.api.place).not.toHaveBeenCalled();

    // tao keeps the logical size across WM_DPICHANGED, so it spills over.
    h.env.live = { x: 0, y: 0, width: 2880, height: 1620 };
    h.env.fitSettled!();
    await flush();
    expect(h.api.place).toHaveBeenCalledTimes(1);
    expect(h.api.place).toHaveBeenLastCalledWith(FIT_RECT, true);
    // The place's own events come back with the rect it set.
    h.env.fitSettled!();
    h.env.fitSettled!();
    await flush();
    expect(h.api.place).toHaveBeenCalledTimes(1);

    h.env.monitors = null;
    h.env.live = { x: 0, y: 0, width: 2880, height: 1620 };
    h.env.fitSettled!();
    await flush();
    expect(h.api.place).toHaveBeenCalledTimes(1);
  });

  it("an unfitted overlay gets geometry tracking, not the keeper", async () => {
    const h = harness({ stored: prefs() });
    await flush();
    await h.c.openOverlay();
    await flush();
    expect(h.env.keepers).toBe(0);
    expect(h.env.tracks).toBe(1);
  });

  it("geometry saves are suspended while fitted, so Fit off goes back to the user's place", async () => {
    const h = harness({ stored: prefs() });
    await flush();
    await h.c.openOverlay();
    await flush();
    const late = h.env.trackSettled!;
    h.env.live = SAVED_RECT;
    await h.c.setOverlayFit(true);
    // A settle from before the swap lands while fitted: ignored.
    late({ x: 0, y: 0, w: 1920, h: 1080, monitor: LAPTOP.name!, scale: 1 });
    expect(h.loaded(PREFS)).toMatchObject({ x: 100, y: 50, w: 480, h: 480 });
    await h.c.setOverlayFit(false);
    expect(h.api.place).toHaveBeenLastCalledWith(SAVED_RECT, false);
  });

  it("closing saves a move that hadn't settled, before the window goes", async () => {
    const h = harness({ stored: prefs() });
    await flush();
    await h.c.openOverlay();
    await flush();
    h.env.pending = { x: 300, y: 200, w: 480, h: 480, monitor: LAPTOP.name!, scale: 1 };
    h.log.length = 0;
    await h.c.closeOverlay();
    expect(h.log).toEqual([`save:${PREFS}`, "close"]);
    expect(h.loaded(PREFS)).toMatchObject({ x: 300, y: 200 });
    expect(h.c.overlayOpen()).toBe(false);
  });

  it("a placement that throws is logged, and the next one still runs", async () => {
    const warn = vi.spyOn(console, "warn").mockImplementation(() => undefined);
    const h = harness({ stored: prefs() });
    await flush();
    h.env.monitorsThrow = true;
    await expect(h.c.setOverlayFit(true)).resolves.toBeUndefined();
    expect(warn).toHaveBeenCalled();
    await h.c.openOverlay();
    expect(h.api.open).toHaveBeenCalledTimes(1);
  });

  it("a colour background turns Fit off first; when that fails the colour isn't applied", async () => {
    const h = harness({ stored: prefs({ fit: true }) });
    await flush();
    await h.c.openOverlay();
    h.env.live = FIT_RECT;
    await h.c.setOverlayBg("green");
    expect(h.api.place).toHaveBeenLastCalledWith(SAVED_RECT, false);
    expect(h.c.overlayFit()).toBe(false);
    expect(h.c.overlayBg()).toBe("green");
    expect(h.loaded(PREFS)).toMatchObject({ bg: "green", fit: false });

    const f = harness({ stored: prefs({ fit: true }) });
    await flush();
    await f.c.openOverlay();
    f.env.placeError = "The overlay isn't open.";
    await f.c.setOverlayBg("magenta");
    expect(f.c.overlayBg()).toBe("transparent");
    expect(f.c.overlayFit()).toBe(true);
    expect(f.toasts.at(-1)?.text).toBe("The overlay isn't open.");

    // Closed, only the flag goes.
    const closed = harness({ stored: prefs({ fit: true }) });
    await flush();
    await closed.c.setOverlayBg("magenta");
    expect(closed.api.place).not.toHaveBeenCalled();
    expect(closed.loaded(PREFS)).toMatchObject({ bg: "magenta", fit: false });
  });
});

describe("overlay controller — status (v1.55.0)", () => {
  it("a null status (the e2e mock) leaves it closed, with no toast", async () => {
    const h = harness({ pull: async () => null });
    await flush();
    h.emitStatus(null);
    h.api.open.mockResolvedValueOnce(null as unknown as OverlayStatus);
    await h.c.openOverlay();
    expect(h.c.overlayOpen()).toBe(false);
    expect(h.toasts).toEqual([]);
  });

  it("status events drive overlayOpen and the mode; the watchdog's error toasts once", async () => {
    const h = harness();
    await flush();
    h.emitStatus(st({ ...PAINTED, mode: "arrange", arrangeGen: 1 }));
    expect(h.c.overlayOpen()).toBe(true);
    expect(h.c.overlayMode()).toBe("arrange");
    h.emitStatus(PAINTED);
    expect(h.c.overlayMode()).toBe("locked");

    h.emitStatus(WATCHDOG);
    h.emitStatus(WATCHDOG);
    h.emitStatus(CLOSED);
    expect(h.c.overlayOpen()).toBe(false);
    expect(h.toasts).toEqual([{ text: WATCHDOG.error, tone: "error" }]);
  });

  it("after a reload: a closed status doesn't toast; a closing one with an error toasts once", async () => {
    const quiet = harness({ backend: CLOSED });
    await flush();
    expect(quiet.toasts).toEqual([]);

    const racing = harness({ backend: WATCHDOG });
    await flush();
    expect(racing.toasts).toEqual([{ text: WATCHDOG.error, tone: "error" }]);
    racing.emitStatus(CLOSED);
    expect(racing.toasts).toHaveLength(1);

    // A lease error already shown before the reload isn't shown again.
    const seen = harness({ backend: st({ ...PAINTED, error: LEASE_NEVER }) });
    await flush();
    expect(seen.toasts).toEqual([]);
  });

  it("a lease error toasts, and again after a new Arrange", async () => {
    const h = harness();
    await flush();
    h.emitStatus(st({ ...PAINTED, error: LEASE_NEVER }));
    h.emitStatus(st({ ...PAINTED, mode: "arrange", arrangeGen: 2 }));
    h.emitStatus(st({ ...PAINTED, arrangeGen: 2, error: LEASE_NEVER }));
    expect(h.toasts).toEqual([
      { text: LEASE_NEVER, tone: "error" },
      { text: LEASE_NEVER, tone: "error" },
    ]);
  });

  it("adopts an open overlay at launch, re-attaching tracking and re-pushing the scene", async () => {
    const h = harness({ stored: prefs(), backend: PAINTED });
    await flush();
    expect(h.c.overlayOpen()).toBe(true);
    expect(h.env.tracks).toBe(1);
    expect(h.pushes).toHaveLength(1);
    expect(h.pushes[0]).toMatchObject({ session: "s1", revision: 0, ack: null, expectViewport: null });
  });

  it("a status event that lands before the launch read wins", async () => {
    const late = deferred<unknown>();
    const h = harness({ stored: prefs(), pull: () => late.promise });
    await flush();
    h.emitStatus(CLOSED);
    late.resolve(PAINTED);
    await flush();
    expect(h.c.overlayOpen()).toBe(false);
    expect(h.env.tracks).toBe(0);
  });

  // Found in the real app: the IPC answer of close_overlay landed 10 ms
  // after the "closed" event, and the toggle stayed on.
  it("a close whose answer lands after the closed event leaves it closed", async () => {
    const h = harness({ backend: PAINTED });
    await flush();
    expect(h.c.overlayOpen()).toBe(true);
    const answer = deferred<OverlayStatus>();
    h.api.close.mockImplementationOnce(() => answer.promise);
    const closing = h.c.closeOverlay();
    await flush();
    h.emitStatus(st({ ...PAINTED, phase: "closing", canArrange: false }));
    h.emitStatus(st({ instance: 1 }));
    answer.resolve(st({ ...PAINTED, phase: "closing", canArrange: false }));
    await closing;
    expect(h.c.overlayOpen()).toBe(false);
    expect(h.env.tracks).toBe(1);
    expect(h.env.trackSettled).toBeNull();
  });

  it("an open whose 'starting' answer lands after painted can still arrange", async () => {
    const h = harness();
    await flush();
    const answer = deferred<OverlayStatus>();
    h.api.open.mockImplementationOnce(() => answer.promise);
    const opening = h.c.openOverlay();
    await flush();
    const starting = st({ open: true, phase: "starting", instance: 1 });
    h.emitStatus(starting);
    h.emitStatus(st({ open: true, phase: "shown", instance: 1 }));
    h.emitStatus(PAINTED);
    answer.resolve(starting);
    await opening;
    expect(h.c.overlayStatus().canArrange).toBe(true);
  });

  it("an answer with no event on its way is applied (a refusal-free mode change)", async () => {
    const h = harness({ backend: PAINTED });
    await flush();
    h.api.setMode.mockResolvedValueOnce(st({ ...PAINTED, mode: "arrange", arrangeGen: 1 }));
    await h.c.setOverlayMode("arrange");
    expect(h.c.overlayMode()).toBe("arrange");
  });

  it("the stored hide flag goes to the backend at launch, and every change after", async () => {
    const h = harness({ stored: prefs({ hideFromRecordings: true }) });
    await flush();
    expect(h.api.setHide).toHaveBeenCalledWith(true);
    expect(h.c.overlayHide()).toBe(true);
    await h.c.setOverlayHide(false);
    expect(h.api.setHide).toHaveBeenLastCalledWith(false);
    expect(h.loaded(PREFS).hideFromRecordings).toBe(false);
    expect(h.c.overlayStatus().hideWanted).toBe(false);
  });

  it("the hotkey's notice: off and starting say so; painted says nothing", async () => {
    const h = harness();
    await flush();
    h.c.noteOverlayHotkey();
    h.emitStatus(st({ open: true, phase: "starting", instance: 1 }));
    h.c.noteOverlayHotkey();
    h.emitStatus(PAINTED);
    h.c.noteOverlayHotkey();
    expect(h.toasts).toEqual([
      { text: "The overlay is off. Turn it on in Settings first.", tone: "success" },
      { text: STARTING, tone: "success" },
    ]);
    expect(h.api.setMode).not.toHaveBeenCalled();
  });

  it("a refused mode change shows the backend's words", async () => {
    const h = harness();
    await flush();
    h.api.setMode.mockRejectedValueOnce(STARTING);
    await h.c.setOverlayMode("arrange");
    expect(h.api.setMode).toHaveBeenCalledWith("arrange");
    expect(h.toasts).toEqual([{ text: STARTING, tone: "error" }]);
  });
});

describe("overlay controller — the migrated scene (v1.55.0)", () => {
  it("is saved when the overlay is first seen open, so a resize can't re-migrate it", async () => {
    const store = new Map<string, string>();
    const first = harness({ store, stored: { [PREFS]: { w: 300, h: 300, scale: 1 } } });
    await flush();
    await first.c.openOverlay();
    await flush();
    expect(first.log.indexOf(`save:${SCENE}`)).toBeLessThan(first.log.indexOf("track"));
    first.env.trackSettled!({ x: 10, y: 10, w: 600, h: 600, monitor: LAPTOP.name!, scale: 1 });
    first.dispose();

    const restarted = harness({ store });
    await flush();
    // As before the restart: the circle the 300 window had, unstretched.
    near(layoutScene(restarted.c.overlayScene(), { w: 600, h: 600 })[0]!.rect, { x: 150, y: 150, w: 300, h: 300 });
  });

  it("and the other way: a 600 window resized to 300 keeps its 440 circle", async () => {
    const store = new Map<string, string>();
    const first = harness({ store, stored: { [PREFS]: { w: 600, h: 600, scale: 1 } } });
    await flush();
    await first.c.openOverlay();
    await flush();
    first.env.trackSettled!({ x: 10, y: 10, w: 300, h: 300, monitor: LAPTOP.name!, scale: 1 });
    first.dispose();

    const restarted = harness({ store });
    await flush();
    expect(restarted.c.overlayScene().designSize).toEqual({ w: 600, h: 600 });
    expect(layoutScene(restarted.c.overlayScene(), { w: 600, h: 600 })[0]!.rect.w).toBeCloseTo(440, 3);
  });

  it("a launch that never opens the overlay writes no scene", async () => {
    const h = harness({ stored: prefs() });
    await flush();
    await h.c.setOverlayFit(true);
    await h.c.setOverlayHide(true);
    expect(h.store.has(SCENE)).toBe(false);
  });

  it("adoption saves the scene before tracking starts", async () => {
    const h = harness({ stored: { [PREFS]: { w: 300, h: 300, scale: 1 } }, backend: PAINTED });
    await flush();
    expect(h.log.indexOf(`save:${SCENE}`)).toBeGreaterThanOrEqual(0);
    expect(h.log.indexOf(`save:${SCENE}`)).toBeLessThan(h.log.indexOf("track"));
  });
});

describe("overlay controller — pictures (v1.55.0)", () => {
  async function opened(scene?: OverlayScene) {
    const h = harness({ stored: { ...prefs(), ...(scene ? { [SCENE]: scene } : {}) } });
    await flush();
    await h.c.openOverlay();
    await flush();
    return h;
  }

  it("imports, adds where defaultImageRect says, saves, pushes and says so", async () => {
    const h = await opened();
    const before = h.pushes.length;
    await h.c.addOverlayPicture();
    await flush();
    expect(h.api.importAsset).toHaveBeenCalledWith("C:\\pics\\wave.gif");
    const img = liveElements(h.c.overlayScene()).find((e) => e.type === "image");
    // §3.2's example: a 498×280 GIF in a 480 window.
    expect(img).toMatchObject({
      x: 0.6,
      y: 0.665663,
      w: 0.3,
      h: 0.168675,
      src: SAMPLE.id,
      nw: 498,
      nh: 280,
      anim: true,
      bytes: 1834221,
      name: "wave.gif",
    });
    expect(h.loaded(SCENE).elements).toHaveLength(2);
    expect(h.pushes.length).toBeGreaterThan(before);
    expect(h.pushes.at(-1)!.scene.elements).toHaveLength(2);
    expect(h.toasts.at(-1)).toEqual({ text: "Picture added. Use Arrange to place it.", tone: "success" });
  });

  it("a picture is sized for the window, not for a scene grown bigger than it", async () => {
    // Arranged once in a 1200 window, now back to 480.
    const big = { ...migrateScene({ w: 1200, h: 1200, scale: 1 }), designSize: { w: 1200, h: 1200 } };
    const h = await opened(big);
    await h.c.addOverlayPicture();
    const img = liveElements(h.c.overlayScene()).find((e) => e.type === "image")!;
    // 30 % of the window's 480, not of the scene's 1200.
    expect(img.w * 1200).toBeCloseTo(144, 3);
  });

  it("a large picture is added with the backend's warning", async () => {
    const h = await opened();
    const warning = "Picture added. It's a large picture, so the overlay may use more memory and CPU.";
    h.api.importAsset.mockResolvedValueOnce({ ...SAMPLE, warning });
    await h.c.addOverlayPicture();
    expect(h.toasts.at(-1)).toEqual({ text: warning, tone: "error" });
  });

  it("at 16 pictures, says so before the picker opens", async () => {
    const pics = Array.from({ length: 16 }, (_, i) => picture(`p${i}`, SAMPLE.id));
    const h = await opened(sceneWith(...pics));
    await h.c.addOverlayPicture();
    expect(h.api.pickImage).not.toHaveBeenCalled();
    expect(h.toasts.at(-1)?.text).toBe("The overlay holds up to 16 pictures. Remove one to add another.");
  });

  it("a cancelled picker does nothing", async () => {
    const h = await opened();
    h.api.pickImage.mockResolvedValueOnce(null);
    await h.c.addOverlayPicture();
    expect(h.api.importAsset).not.toHaveBeenCalled();
    expect(h.toasts).toEqual([]);
  });

  it("an import of width 0 is refused, never 'Picture added', and its file is deleted", async () => {
    const h = await opened();
    h.api.importAsset.mockResolvedValueOnce({ ...SAMPLE, width: 0 });
    await h.c.addOverlayPicture();
    await flush();
    expect(h.toasts.map((t) => t.text)).toEqual(["That picture looks damaged, so Divora can't add it."]);
    expect(liveElements(h.c.overlayScene())).toHaveLength(1);
    expect(h.api.deleteAsset).toHaveBeenCalledWith(SAMPLE.id);
  });

  it("a rejected import shows the backend's words", async () => {
    const h = await opened();
    h.api.importAsset.mockRejectedValueOnce("That picture is over 50 MB. Pick a smaller one.");
    await h.c.addOverlayPicture();
    expect(h.toasts).toEqual([{ text: "That picture is over 50 MB. Pick a smaller one.", tone: "error" }]);
  });

  it("(a) an import of the same file queued before a remove keeps the file", async () => {
    const h = await opened(sceneWith(picture("A", SAMPLE.id)));
    const importing = deferred<ImportedAsset>();
    h.api.importAsset.mockImplementationOnce(() => importing.promise);
    const adding = h.c.addOverlayPicture();
    await flush();
    expect(h.api.importAsset).toHaveBeenCalledTimes(1);
    await h.c.removeOverlayElement("A");
    importing.resolve({ ...SAMPLE });
    await adding;
    await flush();
    const left = liveElements(h.c.overlayScene()).filter((e) => e.type === "image");
    expect(left.map((e) => (e as { src: string }).src)).toEqual([SAMPLE.id]);
    expect(h.api.deleteAsset).not.toHaveBeenCalled();
  });

  it("(b) a refused add whose file is already in use deletes nothing", async () => {
    // At 15, two adds both get past the cap check; the second is refused.
    const pics = Array.from({ length: 15 }, (_, i) => picture(`p${i}`, SAMPLE.id));
    const h = await opened(sceneWith(...pics));
    await Promise.all([h.c.addOverlayPicture(), h.c.addOverlayPicture()]);
    await flush();
    expect(h.toasts.map((t) => t.text)).toEqual([
      "Picture added. Use Arrange to place it.",
      "The overlay holds up to 16 pictures. Remove one to add another.",
    ]);
    expect(h.api.deleteAsset).not.toHaveBeenCalled();
  });

  it("(c) removing the last user of a file deletes it, once", async () => {
    const h = await opened(sceneWith(picture("A", SAMPLE.id)));
    await h.c.removeOverlayElement("A");
    await flush();
    expect(h.api.deleteAsset.mock.calls).toEqual([[SAMPLE.id]]);
  });

  it("a file any other string in the scene names is kept, an unknown element's included", async () => {
    const h = await opened(sceneWith(picture("A", SAMPLE.id), { id: "s", type: "sticker", icon: SAMPLE.id }));
    await h.c.removeOverlayElement("A");
    await flush();
    expect(h.api.deleteAsset).not.toHaveBeenCalled();
  });

  it("one failed delete doesn't stop the queue", async () => {
    const warn = vi.spyOn(console, "warn").mockImplementation(() => undefined);
    const h = await opened(sceneWith(picture("A", SAMPLE.id)));
    h.api.deleteAsset.mockRejectedValueOnce("Couldn't remove the picture: it's open elsewhere");
    await expect(h.c.removeOverlayElement("A")).resolves.toBeUndefined();
    await flush();
    expect(h.api.deleteAsset).toHaveBeenCalledTimes(1);
    expect(warn).toHaveBeenCalled();

    await h.c.addOverlayPicture();
    expect(liveElements(h.c.overlayScene()).filter((e) => e.type === "image")).toHaveLength(1);
    expect(h.toasts.at(-1)?.text).toBe("Picture added. Use Arrange to place it.");

    // A rejected import, and a delete queued behind it that still runs.
    const id = liveElements(h.c.overlayScene()).find((e) => e.type === "image")!.id;
    const importing = deferred<ImportedAsset>();
    h.api.importAsset.mockImplementationOnce(() => importing.promise);
    const adding = h.c.addOverlayPicture();
    await flush();
    await h.c.removeOverlayElement(id);
    importing.reject("Couldn't read that picture: gone");
    await adding;
    await flush();
    expect(h.toasts.at(-1)?.text).toBe("Couldn't read that picture: gone");
    expect(h.api.deleteAsset).toHaveBeenCalledTimes(2);
    expect(h.api.deleteAsset).toHaveBeenLastCalledWith(SAMPLE.id);
  });

  it("a picture task that throws is logged, and the next one still runs", async () => {
    const warn = vi.spyOn(console, "warn").mockImplementation(() => undefined);
    const h = await opened(sceneWith(picture("A", SAMPLE.id)));
    const broken = {
      get id(): string {
        throw new Error("boom");
      },
    };
    h.api.importAsset.mockResolvedValueOnce(broken as unknown as ImportedAsset);
    await expect(h.c.addOverlayPicture()).resolves.toBeUndefined();
    expect(warn).toHaveBeenCalled();
    await h.c.removeOverlayElement("A");
    await flush();
    expect(h.api.deleteAsset).toHaveBeenCalledWith(SAMPLE.id);
  });

  it("the spell circle can be removed and added back, at the back", async () => {
    const h = await opened(sceneWith(picture("A", SAMPLE.id)));
    await h.c.removeOverlayElement(CIRCLE_ID);
    expect(liveElements(h.c.overlayScene()).map((e) => e.id)).toEqual(["A"]);
    h.c.addSpellCircle();
    const drawn = layoutScene(h.c.overlayScene(), V480);
    expect(drawn.map((r) => r.el.id)).toEqual([CIRCLE_ID, "A"]);
    near(drawn[0]!.rect, { x: 20, y: 20, w: 440, h: 440 });
    expect(h.loaded(SCENE).elements).toHaveLength(2);
    // Only one.
    h.c.addSpellCircle();
    expect(liveElements(h.c.overlayScene())).toHaveLength(2);
  });
});

describe("overlay controller — edits from the overlay (v1.55.0)", () => {
  it("seam: a drag's edit is applied by main, saved, acked, and the overlay drops it", async () => {
    const h = harness({ stored: prefs() });
    await flush();
    await h.c.openOverlay();
    h.emitStatus(PAINTED);
    await flush();
    const first = h.pushes.at(-1)!;

    // The overlay's side: the mirror, a drag, its edit.
    let m = receiveScene(emptyMirror(), first, 1, 0, V480);
    const circle = layoutScene(renderedScene(m, V480, 0)!, V480).find((r) => r.el.id === CIRCLE_ID)!;
    let g = beginMove(CIRCLE_ID, 1, { x: 240, y: 240 }, circle.rect, V480);
    g = updateGesture(g, { x: 230, y: 250 });
    const proposed = proposeEdit(m, finishGesture(g)!, 1);
    m = proposed.mirror;
    expect(m.pending).toHaveLength(1);

    h.edit(JSON.parse(JSON.stringify(proposed.edit)));
    await flush();

    // Main's side: the same reducer, saved.
    near(layoutScene(h.loaded(SCENE), V480)[0]!.rect, { x: 10, y: 30, w: 440, h: 440 });
    const acked = h.pushes.at(-1)!;
    expect(acked.ack).toEqual({ instance: 1, seq: 0 });
    expect(acked.revision).toBeGreaterThan(first.revision);

    m = receiveScene(m, acked, 1, 1, V480);
    expect(m.pending).toEqual([]);
    expect(renderedScene(m, V480, 1)).toEqual(acked.scene);
  });

  it("an edit main can't read is ignored, and still acked", async () => {
    const warn = vi.spyOn(console, "warn").mockImplementation(() => undefined);
    const h = harness({ stored: prefs() });
    await flush();
    await h.c.openOverlay();
    await flush();
    const before = h.pushes.at(-1)!;
    const scene = h.c.overlayScene();
    // `add` is main's alone.
    h.edit({ instance: 1, seq: 4, op: { kind: "add", element: picture("x", SAMPLE.id) } });
    await flush();
    expect(warn).toHaveBeenCalled();
    expect(h.c.overlayScene()).toBe(scene);
    const acked = h.pushes.at(-1)!;
    expect(acked.ack).toEqual({ instance: 1, seq: 4 });
    expect(acked.revision).toBe(before.revision);
  });

  it("the ready re-push carries no ack", async () => {
    const h = harness({ stored: prefs() });
    await flush();
    await h.c.openOverlay();
    await flush();
    h.edit({ instance: 1, seq: 0, op: { kind: "order", id: CIRCLE_ID, to: "front" } });
    await flush();
    expect(h.pushes.at(-1)!.ack).toEqual({ instance: 1, seq: 0 });
    h.fire(OVERLAY_READY_EVENT);
    await flush();
    expect(h.pushes.at(-1)).toMatchObject({ ack: null, expectViewport: null });
    expect(h.c.overlayScenePush().ack).toBeNull();
  });

  it("nothing is pushed while the overlay is closed", async () => {
    const h = harness({ stored: prefs() });
    await flush();
    h.edit({ instance: 1, seq: 0, op: { kind: "remove", id: CIRCLE_ID } });
    await flush();
    expect(h.pushes).toEqual([]);
    expect(liveElements(h.c.overlayScene())).toEqual([]);
  });
});

describe("overlay controller — timer faces (v1.59.0)", () => {
  async function opened(scene?: OverlayScene) {
    const h = harness({ stored: { ...prefs(), ...(scene ? { [SCENE]: scene } : {}) } });
    await flush();
    await h.c.openOverlay();
    await flush();
    return h;
  }

  /** A face for `timer`, of the migrated 480 scene. */
  function face(id: string, timer: string, more: Record<string, unknown> = {}): SceneElement {
    return { id, type: "timer", x: 0.35, y: 0.08, w: 0.3, h: 0.1, z: 1, timer, ...more };
  }

  const faces = (s: OverlayScene) => s.elements.filter((e) => e.type === "timer");

  it("addOverlayTimer adds one face per timer where defaultTimerRect says, saves and pushes it", async () => {
    const h = await opened();
    const before = h.pushes.length;
    const at = defaultTimerRect(h.c.overlayScene(), V480);
    h.c.addOverlayTimer("timer-1");
    await flush();
    expect(faces(h.c.overlayScene())).toEqual([{ id: "c-00000000", type: "timer", ...at, z: 1, timer: "timer-1" }]);
    expect(h.c.overlayHasTimer("timer-1")).toBe(true);
    expect(faces(h.loaded(SCENE))).toHaveLength(1);
    expect(h.pushes.length).toBe(before + 1);
    expect(faces(h.pushes.at(-1)!.scene)).toHaveLength(1);
    expect(h.toasts).toEqual([]);

    // Once per timer.
    h.c.addOverlayTimer("timer-1");
    await flush();
    expect(faces(h.c.overlayScene())).toHaveLength(1);
    expect(h.pushes.length).toBe(before + 1);

    // Another timer's face goes 6 % lower.
    h.c.addOverlayTimer("timer-2");
    const [one, two] = faces(h.c.overlayScene()) as [SceneElement, SceneElement];
    expect(two.timer).toBe("timer-2");
    expect((two.y as number) - (one.y as number)).toBeCloseTo(0.06, 6);
    expect(h.c.overlayHasTimer("timer-2")).toBe(true);
    expect(h.c.overlayHasTimer("timer-3")).toBe(false);
  });

  it("a face is sized for the window, not for a scene grown bigger than it", async () => {
    const big = { ...migrateScene({ w: 1200, h: 1200, scale: 1 }), designSize: { w: 1200, h: 1200 } };
    const h = await opened(big);
    h.c.addOverlayTimer("timer-1");
    const f = faces(h.c.overlayScene())[0]!;
    // 12 % of the window's 480, not of the scene's 1200.
    expect((f.h as number) * 1200).toBeCloseTo(57.6, 3);
  });

  it("an id that isn't a timer's adds nothing", async () => {
    const h = await opened();
    const scene = h.c.overlayScene();
    for (const id of ["Timer-1", "con", "", "tm:timer-1:toggle"]) h.c.addOverlayTimer(id);
    expect(h.c.overlayScene()).toBe(scene);
    expect(h.toasts).toEqual([]);
  });

  it("at 64 stored elements, says the overlay is full", async () => {
    const stored = Array.from({ length: MAX_ELEMENTS - 1 }, (_, i) => ({ id: `t${i}`, type: "future" }));
    const h = await opened(sceneWith(...stored));
    h.c.addOverlayTimer("timer-1");
    expect(h.c.overlayHasTimer("timer-1")).toBe(false);
    expect(h.toasts).toEqual([{ text: "The overlay is full. Remove something to add another.", tone: "error" }]);
  });

  it("a stored face with unusable geometry doesn't count: the toggle reads off, and on adds a live one", async () => {
    const h = await opened(sceneWith(face("flat", "timer-1", { h: 0 })));
    expect(h.c.overlayHasTimer("timer-1")).toBe(false);
    h.c.addOverlayTimer("timer-1");
    expect(h.c.overlayHasTimer("timer-1")).toBe(true);
    expect(faces(h.c.overlayScene()).map((e) => e.id)).toEqual(["flat", "c-00000000"]);
  });

  it("removeOverlayTimer removes every face for that timer, inert ones included, in one push", async () => {
    const h = await opened(
      sceneWith(
        face("a", "timer-1"),
        picture("P", "0123456789abcdef.png"),
        face("b", "timer-2"),
        face("c", "timer-1", { y: 0.2 }),
        face("flat", "timer-1", { w: 0 }),
        { id: "s", type: "sticker", timer: "timer-1" },
      ),
    );
    const before = h.pushes.length;
    h.c.removeOverlayTimer("timer-1");
    await flush();
    expect(h.c.overlayScene().elements.map((e) => e.id)).toEqual([CIRCLE_ID, "P", "b", "s"]);
    expect(h.loaded(SCENE).elements.map((e) => e.id)).toEqual([CIRCLE_ID, "P", "b", "s"]);
    expect(h.pushes.length).toBe(before + 1);
    expect(h.c.overlayHasTimer("timer-1")).toBe(false);
    expect(h.c.overlayHasTimer("timer-2")).toBe(true);

    // Nothing left of it: nothing saved, nothing pushed.
    const saves = h.saves(SCENE);
    h.c.removeOverlayTimer("timer-1");
    await flush();
    expect(h.saves(SCENE)).toBe(saves);
    expect(h.pushes.length).toBe(before + 1);
  });

  it("overlayHasTimer follows an Arrange delete, reactively", async () => {
    const h = await opened();
    h.c.addOverlayTimer("timer-1");
    const seen: boolean[] = [];
    createRoot((dispose) => {
      disposers.push(dispose);
      createEffect(() => seen.push(h.c.overlayHasTimer("timer-1")));
    });
    expect(seen).toEqual([true]);
    const id = faces(h.c.overlayScene())[0]!.id;
    // The overlay's Delete key, as its edit reaches main.
    h.edit({ instance: 1, seq: 0, op: { kind: "remove", id } });
    await flush();
    expect(seen).toEqual([true, false]);
    // Moving another charm changes the scene, not the answer: no re-run.
    const scene = h.c.overlayScene();
    h.edit({ instance: 1, seq: 1, op: { kind: "place", id: CIRCLE_ID, rect: { x: 10, y: 10, w: 440, h: 440 }, viewport: V480 } });
    await flush();
    expect(h.c.overlayScene()).not.toBe(scene);
    expect(seen).toEqual([true, false]);
    // Show on overlay again.
    h.c.addOverlayTimer("timer-1");
    expect(seen).toEqual([true, false, true]);
  });
});
