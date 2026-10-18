// v1.55.0: the main window's side of the stream overlay.
//
// The backend creates, places and shows the overlay window and owns its
// mode (overlay_machine.rs); every change comes back as an `OverlayStatus`.
// Main decides where the window goes (frame.ts) and owns the scene: it
// reads and migrates `divora.overlayScene`, applies the overlay's edits
// with the same reducer the overlay uses, saves, and pushes the result with
// a session, a revision and an ack (sync.ts). Pictures are imported and
// deleted one at a time, and a file is deleted only once nothing in the
// scene names it.
//
// Everything outside is injected, so the tests drive this controller rather
// than a copy of it (the sync.ts pattern). It replaces the overlay block
// app.tsx carried since v1.16.0. Call it inside a reactive owner.

import { createMemo, createSignal, onCleanup } from "solid-js";
import type { ImportedAsset } from "../audio/api";
import type { ToastTone } from "../components/Toast";
import { planFrame, type MonitorsRead } from "./frame";
import {
  effectiveFit,
  fitRect,
  isSavablePosition,
  mergeOverlayPrefs,
  pickMonitor,
  readGeometry,
  readOverlayBg,
  readOverlayFlags,
  type MonitorInfo,
  type OverlayGeometry,
  type Rect,
} from "./geometry";
import {
  addRefusal,
  applyOp,
  asCircle,
  assetRefCount,
  assetRefs,
  CIRCLE_ID,
  defaultImageRect,
  defaultScene,
  frameOf,
  MAX_PICTURES,
  migrateScene,
  pictureCount,
  readOp,
  readScene,
  sceneForSave,
  type AddRefusal,
  type ImageElement,
  type KnownElement,
  type OverlayScene,
  type PxRect,
  type SceneOp,
  type Size,
} from "./scene";
import type { OverlayBg, ScenePush } from "./state";
import { closedStatus, readOverlayStatus, type OverlayStatus } from "./status";
import { syncOverlayScene } from "./sync";

/** Where the overlay's settings and its scene are kept. */
export const OVERLAY_PREFS_KEY = "divora.overlay";
export const OVERLAY_SCENE_KEY = "divora.overlayScene";

/** How long a move or resize must be still before it counts as settled. */
const SETTLE_MS = 400;
/** How often the fit keeper checks the window still covers its monitor (a
 *  resolution change sends no event the overlay hears). */
const FIT_CHECK_MS = 5000;

// The shell's toast has two tones: "success" for news, "error" for
// anything that needs reading.
const INFO: ToastTone = "success";
const WARN: ToastTone = "error";

const ADDED = "Picture added. Use Arrange to place it.";
const AT_CAP = "The overlay holds up to 16 pictures. Remove one to add another.";
const NO_MONITORS = "Couldn't read your monitors.";
const HOTKEY_OFF = "The overlay is off. Turn it on in Settings first.";
const STARTING = "The overlay is still starting. Try again in a moment.";
const DAMAGED = "That picture looks damaged, so Divora can't add it.";
const REFUSED: Record<AddRefusal, string> = {
  pictures: AT_CAP,
  elements: "The overlay is full. Remove something to add another.",
  budget: "That's more than the overlay can hold. Remove a picture or pick a smaller one.",
  invalid: DAMAGED,
  // Not reachable from here: ids are picked unused, and the circle button
  // shows only without a circle.
  exists: DAMAGED,
  circle: DAMAGED,
};

/** A geometry tracker: saves after a move or resize settles. */
export interface GeometryTracker {
  stop(): void;
  /** Run a save that is still waiting for its move to settle, now. */
  flush(): Promise<void>;
}

/** The overlay window as main can see it. Every call is caught: off Tauri,
 *  or with the window gone, each answers null or does nothing. */
export interface OverlayWindows {
  monitors(): Promise<MonitorsRead | null>;
  /** The overlay's `outerPosition` and `innerSize`, or null. */
  liveRect(): Promise<Rect | null>;
  /** Calls `onSettled` with where the window is once a move or resize has
   *  settled. */
  track(onSettled: (g: OverlayGeometry) => void): Promise<GeometryTracker>;
  /** Calls `onSettled` once a scale change, move or resize has settled, and
   *  every few seconds. */
  keepFit(onSettled: () => void): Promise<() => void>;
}

export interface OverlayControllerDeps {
  api: {
    open(rect: Rect | null, fit: boolean): Promise<unknown>;
    close(): Promise<unknown>;
    place(rect: Rect, fit: boolean): Promise<unknown>;
    setMode(m: "locked" | "arrange" | "toggle"): Promise<unknown>;
    setHide(on: boolean): Promise<unknown>;
    status(): Promise<unknown>;
    importAsset(path: string): Promise<ImportedAsset>;
    deleteAsset(id: string): Promise<void>;
    pickImage(): Promise<string | null>;
    subscribeStatus(h: (raw: unknown) => void): Promise<() => void>;
    subscribeEdit(h: (raw: unknown) => void): Promise<() => void>;
  };
  events: {
    emit(event: string, payload: unknown): Promise<void>;
    listen(event: string, h: (p: unknown) => void): Promise<() => void>;
  };
  windows: OverlayWindows;
  storage: { load(key: string): unknown; save(key: string, v: unknown): void };
  toast(text: string, tone?: ToastTone): void;
  /** A fresh element id: "c-" and 8 hex digits. */
  newId(): string;
  /** This page load; a new one tells the overlay main reloaded. */
  session: string;
}

export interface OverlayController {
  overlayOpen(): boolean;
  overlayStatus(): OverlayStatus;
  overlayMode(): "locked" | "arrange";
  overlayBg(): OverlayBg;
  setOverlayBg(bg: OverlayBg): Promise<void>;
  overlayFit(): boolean;
  setOverlayFit(on: boolean): Promise<void>;
  overlayHide(): boolean;
  setOverlayHide(on: boolean): Promise<void>;
  overlayScene(): OverlayScene;
  overlayScenePush(): ScenePush;
  openOverlay(): Promise<void>;
  closeOverlay(): Promise<void>;
  setOverlayMode(m: "locked" | "arrange"): Promise<void>;
  /** The `overlay:arrange` hotkey: the backend already acted on it, so this
   *  only says why nothing happened. */
  noteOverlayHotkey(): void;
  addOverlayPicture(): Promise<void>;
  removeOverlayElement(id: string): Promise<void>;
  addSpellCircle(): void;
}

/** Tasks run one at a time, in order. One failing never stops the rest. */
function makeQueue(): <T>(task: () => Promise<T>) => Promise<T> {
  let tail: Promise<void> = Promise.resolve();
  return <T>(task: () => Promise<T>): Promise<T> => {
    const run = tail.then(task);
    tail = run.then(
      () => undefined,
      () => undefined,
    );
    return run;
  };
}

/** The public actions never reject: their callers are `void` click
 *  handlers. Each task catches what it expects, so a failure here is a bug,
 *  and is logged. */
function logged(p: Promise<unknown>, what: string): Promise<void> {
  return p.then(
    () => undefined,
    (e) => console.warn(`overlay: ${what} failed`, e),
  );
}

function sameRect(a: Rect, b: Rect): boolean {
  return a.x === b.x && a.y === b.y && a.width === b.width && a.height === b.height;
}

/** A rect in a monitor's CSS px (as `canvasRects` gives) back in physical
 *  px. */
function physicalRect(r: PxRect, m: MonitorInfo): Rect {
  const s = m.scaleFactor > 0 ? m.scaleFactor : 1;
  return {
    x: Math.round(m.x + r.x * s),
    y: Math.round(m.y + r.y * s),
    width: Math.round(r.w * s),
    height: Math.round(r.h * s),
  };
}

/** The file's name from a picked path, for the Settings list. */
function fileName(path: string): string | undefined {
  const name = path.split(/[\\/]/).pop()?.trim();
  return name ? name.slice(0, 120) : undefined;
}

export function createOverlayController(d: OverlayControllerDeps): OverlayController {
  const placement = makeQueue();
  const assets = makeQueue();

  // ---- settings (`divora.overlay`) ----
  const prefs = d.storage.load(OVERLAY_PREFS_KEY);
  const [bg, setBg] = createSignal<OverlayBg>(
    readOverlayBg(typeof prefs === "object" && prefs !== null ? (prefs as { bg?: unknown }).bg : undefined),
  );
  const flags = readOverlayFlags(prefs);
  // A stored Fit with a colour background (mixed builds) reads as off. Nothing
  // is written at boot; the next save stores the corrected flag.
  const [fit, setFit] = createSignal(effectiveFit(flags, bg()));
  const [hide, setHide] = createSignal(flags.hideFromRecordings);
  let geometry: OverlayGeometry | null = readGeometry(prefs);
  const savePrefs = (): void => {
    d.storage.save(
      OVERLAY_PREFS_KEY,
      mergeOverlayPrefs(d.storage.load(OVERLAY_PREFS_KEY), bg(), geometry, {
        fit: fit(),
        hideFromRecordings: hide(),
      }),
    );
  };

  // ---- the scene (`divora.overlayScene`) ----
  const migrated = migrateScene(prefs);
  const initial = readScene(d.storage.load(OVERLAY_SCENE_KEY), migrated);
  // A migrated scene is saved the first time the overlay is seen open, before
  // any geometry save can change what it was migrated from.
  let sceneStored = initial !== migrated;
  const [scene, setScene] = createSignal<OverlayScene>(initial);
  const saveScene = (): void => {
    d.storage.save(OVERLAY_SCENE_KEY, sceneForSave(scene()));
    sceneStored = true;
  };

  let revision = 0;
  let ack: ScenePush["ack"] = null;
  const pushOf = (expectViewport: Size | null): ScenePush => ({
    session: d.session,
    revision,
    scene: scene(),
    ack,
    expectViewport,
  });
  const [push, setPush] = createSignal<ScenePush>(pushOf(null));
  /** Send the scene as it is now. `expectViewport` only for a Fit change. */
  const publish = (expectViewport: Size | null = null): void => {
    setPush(pushOf(expectViewport));
  };

  // ---- status ----
  const [status, setStatus] = createSignal<OverlayStatus>(closedStatus());
  const open = createMemo(() => status().open);
  let lastError: string | null = null;

  const sync = syncOverlayScene({
    isOpen: open,
    push,
    emit: d.events.emit,
    listen: d.events.listen,
    onReady: () => {
      // A reloaded page numbers its edits from 0 again.
      ack = null;
      setPush((p) => ({ ...p, ack: null, expectViewport: null }));
    },
  });

  // ---- geometry tracking, or the fit keeper while fitted ----
  let attached: "track" | "fit" | null = null;
  let attachGen = 0;
  let tracker: GeometryTracker | null = null;
  let stopKeeper: (() => void) | null = null;

  const onGeometry = (g: OverlayGeometry): void => {
    // Fitted, `divora.overlay` keeps the user's own place.
    if (status().fit) return;
    geometry = g;
    savePrefs();
  };

  const detach = (): void => {
    attachGen += 1;
    attached = null;
    tracker?.stop();
    tracker = null;
    stopKeeper?.();
    stopKeeper = null;
  };

  const attach = (kind: "track" | "fit"): void => {
    if (attached === kind) return;
    detach();
    attached = kind;
    const gen = attachGen;
    if (kind === "track") {
      void d.windows.track(onGeometry).then(
        (t) => {
          if (gen === attachGen) tracker = t;
          else t.stop();
        },
        () => undefined,
      );
    } else {
      void d.windows.keepFit(onFitSettled).then(
        (stop) => {
          if (gen === attachGen) stopKeeper = stop;
          else stop();
        },
        () => undefined,
      );
    }
  };

  const flushGeometry = async (): Promise<void> => {
    try {
      await tracker?.flush();
    } catch {
      /* keep the last good geometry */
    }
  };

  const applyStatus = (raw: unknown): void => {
    const s = readOverlayStatus(raw);
    if (!s) return;
    const was = status().open;
    setStatus(s);
    if (s.open && !was) {
      if (!sceneStored) saveScene();
      if (attached === null) attach(s.fit ? "fit" : "track");
    } else if (!s.open && was) {
      detach();
    }
    if (s.error !== null && s.error !== lastError) d.toast(s.error, WARN);
    lastError = s.error;
  };

  // Every change the backend makes goes out as an `overlay:status` event,
  // and a command's answer can land after the events that followed it (a
  // close answers "closing" after "closed" has arrived). So an answer only
  // counts when no event came while it was on its way.
  let heard = 0;
  const answered = async (call: () => Promise<unknown>): Promise<void> => {
    const before = heard;
    const raw = await call();
    if (heard === before) applyStatus(raw);
  };

  // ---- the fit keeper ----
  let refitQueued = false;
  function onFitSettled(): void {
    if (refitQueued) return;
    refitQueued = true;
    const refit = placement(async () => {
      refitQueued = false;
      const s = status();
      if (!s.open || !s.fit) return;
      const [read, live] = await Promise.all([
        d.windows.monitors().catch(() => null),
        d.windows.liveRect().catch(() => null),
      ]);
      if (!read || !live) return;
      // `geometry` holds still while fitted, so this is the same monitor.
      const want = fitRect(geometry, read.monitors, read.primary);
      if (!want || sameRect(want, live)) return;
      try {
        await answered(() => d.api.place(want, true));
      } catch (e) {
        console.warn("overlay: couldn't re-fit the window", e);
      }
    });
    void logged(refit, "re-fitting the window");
  }

  // ---- edits ----
  const deleteTask = async (id: string): Promise<void> => {
    // An import of the same picture may have been queued meanwhile.
    if (assetRefCount(scene(), id) !== 0) return;
    try {
      await d.api.deleteAsset(id);
    } catch (e) {
      console.warn("overlay: couldn't delete charm asset", id, e);
    }
  };

  /** Apply one op, save, and queue the deletion of every picture it left
   *  unnamed. False when nothing changed. The caller publishes. */
  const commit = (op: SceneOp): boolean => {
    const prev = scene();
    const next = applyOp(prev, op);
    if (next === prev) return false;
    setScene(next);
    revision += 1;
    saveScene();
    for (const [id, n] of assetRefs(prev)) {
      if (n >= 1 && assetRefCount(next, id) === 0) void assets(() => deleteTask(id));
    }
    return true;
  };

  const onEdit = (raw: unknown): void => {
    const e = (typeof raw === "object" && raw !== null ? raw : {}) as Record<string, unknown>;
    const op = readOp(e.op, "overlay");
    let changed = false;
    if (op) changed = commit(op);
    else console.warn("overlay: ignored an edit main can't read", raw);
    // Acked even when nothing changed, so the overlay drops it.
    if (Number.isInteger(e.instance) && Number.isInteger(e.seq)) {
      ack = { instance: e.instance as number, seq: e.seq as number };
      publish();
    } else if (changed) {
      publish();
    }
  };

  const freshId = (): string => {
    const taken = new Set(scene().elements.map((e) => e.id));
    let id = d.newId();
    for (let i = 0; i < 16 && taken.has(id); i += 1) id = d.newId();
    return id;
  };

  // ---- start ----
  let disposed = false;
  const unlisten: Array<() => void> = [];
  const keep = (un: (() => void) | null): void => {
    if (!un) return;
    if (disposed) un();
    else unlisten.push(un);
  };
  // An event is newer than the launch reads below whenever it lands first.
  // All four are asked for at once, as Replay's are (app.tsx); each failure
  // (off Tauri) is caught where it starts.
  const statusSub = d.api
    .subscribeStatus((raw) => {
      heard += 1;
      applyStatus(raw);
    })
    .catch(() => null);
  const editSub = d.api.subscribeEdit(onEdit).catch(() => null);
  const pull = d.api.status().catch(() => null);
  // The stored Hide flag, which the backend doesn't keep.
  const pushedHide = d.api.setHide(hide()).catch(() => null);
  void (async () => {
    keep(await statusSub);
    keep(await editSub);
    // A main-window reload finds the overlay still open: adopt it. Its error
    // was already shown, unless it is closing (a reload racing the
    // watchdog).
    const now = readOverlayStatus(await pull);
    if (disposed) return;
    if (now && heard === 0) {
      if (now.phase !== "closing") lastError = now.error;
      applyStatus(now);
    }
    const after = await pushedHide;
    if (!disposed && heard === 0) applyStatus(after);
  })();
  onCleanup(() => {
    disposed = true;
    for (const un of unlisten.splice(0)) un();
    detach();
  });

  // ---- Fit to monitor ----
  /** Turn Fit on or off; true when it took. Runs on the placement queue. */
  const fitTask = async (on: boolean): Promise<boolean> => {
    if (on && bg() !== "transparent") return false;
    const read = await d.windows.monitors().catch(() => null);
    if (on && !(read && fitRect(geometry, read.monitors, read.primary))) {
      d.toast(NO_MONITORS, WARN);
      return false;
    }
    const s = status();
    // Closed, the next open reconciles the scene's frame. Open with the
    // window already as wanted, there is nothing to place.
    if (!s.open || s.fit === on) {
      setFit(on);
      savePrefs();
      return true;
    }
    await flushGeometry();
    const live = await d.windows.liveRect().catch(() => null);
    const plan = planFrame({ scene: scene(), wantFit: on, monitors: read, saved: geometry, live });
    let { rect, viewport } = plan;
    // Fitted since a fresh install, the window has no place of its own to
    // go back to: use the one the scene is reframed to.
    if (!rect && read && plan.reframe?.kind === "reframe") {
      const m = pickMonitor(geometry, read.monitors, read.primary);
      if (m) {
        rect = physicalRect(plan.reframe.to, m);
        viewport = { w: plan.reframe.to.w, h: plan.reframe.to.h };
      }
    }
    if (!rect) {
      d.toast(NO_MONITORS, WARN);
      return false;
    }
    // The reframed scene goes first, unsaved, and the overlay keeps drawing
    // the old one until its window has the new size.
    const before = scene();
    if (plan.reframe) {
      const next = applyOp(before, plan.reframe);
      if (next !== before) {
        setScene(next);
        revision += 1;
        publish(viewport);
        await sync.flushed();
      }
    }
    let placed: unknown;
    const sent = heard;
    try {
      placed = await d.api.place(rect, plan.fit);
    } catch (e) {
      if (scene() !== before) {
        setScene(before);
        revision += 1;
        publish();
      }
      d.toast(String(e), WARN);
      return false;
    }
    saveScene();
    setFit(on);
    savePrefs();
    attach(plan.fit ? "fit" : "track");
    if (heard === sent) applyStatus(placed);
    return true;
  };

  const pictureView = (): Size | undefined => {
    // Fitted, the scene's own size is the monitor's.
    if (frameOf(scene()) === "monitor" || !geometry) return undefined;
    const k = geometry.scale ?? 1;
    return { w: geometry.w / k, h: geometry.h / k };
  };

  return {
    overlayOpen: open,
    overlayStatus: status,
    overlayMode: () => status().mode,
    overlayBg: bg,
    overlayFit: fit,
    overlayHide: hide,
    overlayScene: scene,
    overlayScenePush: push,

    // In order with Fit changes: a colour background needs Fit off first, and
    // isn't applied when that fails.
    setOverlayBg: (next) =>
      logged(
        placement(async () => {
          if (next !== "transparent" && (fit() || status().fit) && !(await fitTask(false))) return;
          setBg(next);
          savePrefs();
        }),
        "setting the background",
      ),

    setOverlayFit: (on) => logged(placement(() => fitTask(on)), "changing Fit to monitor"),

    setOverlayHide: async (on) => {
      setHide(on);
      savePrefs();
      try {
        await answered(() => d.api.setHide(on));
      } catch (e) {
        console.warn("overlay: set_overlay_hide_from_recordings failed", e);
      }
    },

    openOverlay: () =>
      logged(
        placement(async () => {
          // A second click while one opens runs after it, and finds it open.
          if (status().open) return;
          const plan = planFrame({
            scene: scene(),
            wantFit: effectiveFit({ fit: fit(), hideFromRecordings: hide() }, bg()),
            monitors: await d.windows.monitors().catch(() => null),
            saved: geometry,
            live: null,
          });
          // Saved now, before the window exists; the overlay gets it on ready.
          if (plan.reframe && commit(plan.reframe)) publish();
          try {
            await answered(() => d.api.open(plan.rect, plan.fit));
          } catch (e) {
            d.toast(String(e), WARN);
          }
        }),
        "opening the overlay",
      ),

    closeOverlay: () =>
      logged(
        placement(async () => {
          // A move that hasn't settled yet would otherwise be lost.
          await flushGeometry();
          try {
            await answered(() => d.api.close());
          } catch {
            /* not under Tauri */
          }
        }),
        "closing the overlay",
      ),

    setOverlayMode: async (m) => {
      try {
        await answered(() => d.api.setMode(m));
      } catch (e) {
        d.toast(String(e), WARN);
      }
    },

    noteOverlayHotkey: () => {
      const s = status();
      if (!s.open || s.phase === "closing") d.toast(HOTKEY_OFF, INFO);
      else if (!s.canArrange) d.toast(STARTING, INFO);
    },

    addOverlayPicture: async () => {
      if (pictureCount(scene()) >= MAX_PICTURES) {
        d.toast(AT_CAP, WARN);
        return;
      }
      const path = await d.api.pickImage().catch(() => null);
      if (!path) return;
      const adding = assets(async () => {
        let asset: ImportedAsset;
        try {
          asset = await d.api.importAsset(path);
        } catch (e) {
          d.toast(String(e), WARN);
          return;
        }
        if (typeof asset !== "object" || asset === null || typeof asset.id !== "string") {
          console.warn("overlay: the import answered something it shouldn't", asset);
          d.toast(DAMAGED, WARN);
          return;
        }
        const width = Number(asset.width);
        const height = Number(asset.height);
        const s = scene();
        const name = fileName(path);
        const element: ImageElement = {
          id: freshId(),
          type: "image",
          ...defaultImageRect(s, width, height, pictureView()),
          z: 0,
          src: asset.id,
          nw: width,
          nh: height,
          anim: asset.animated === true,
          bytes: Number(asset.bytes) || 0,
          ...(name ? { name } : {}),
        };
        const refusal = addRefusal(s, element);
        if (refusal) {
          d.toast(REFUSED[refusal], WARN);
          // The import may have stored a file nothing uses; the delete checks
          // again, as the same picture may be in the scene already.
          void assets(() => deleteTask(asset.id));
          return;
        }
        if (commit({ kind: "add", element })) publish();
        if (asset.warning) d.toast(asset.warning, WARN);
        else d.toast(ADDED, INFO);
      });
      await logged(adding, "adding a picture");
    },

    removeOverlayElement: async (id) => {
      if (commit({ kind: "remove", id })) publish();
    },

    addSpellCircle: () => {
      const s = scene();
      const at = asCircle(defaultScene(s.designSize).elements[0]!);
      if (!at) return;
      const taken = s.elements.some((e) => e.id === CIRCLE_ID);
      const element: KnownElement = { ...at, id: taken ? freshId() : CIRCLE_ID };
      const refusal = addRefusal(s, element);
      if (refusal) {
        if (refusal !== "circle") d.toast(REFUSED[refusal], WARN);
        return;
      }
      if (commit({ kind: "add", element })) publish();
    },
  };
}

/** The real `OverlayWindows`, through @tauri-apps/api (loaded lazily). Main
 *  only reads the overlay window and listens to it: the backend places it. */
export function tauriOverlayWindows(): OverlayWindows {
  const overlayWindow = async () => {
    const { Window } = await import("@tauri-apps/api/window");
    return Window.getByLabel("overlay");
  };
  type Un = () => void;
  const stopAll = (un: Un[]): void => {
    for (const f of un.splice(0)) f();
  };

  return {
    async monitors() {
      try {
        const win = await import("@tauri-apps/api/window");
        const [list, primary] = await Promise.all([win.availableMonitors(), win.primaryMonitor()]);
        return {
          monitors: list.map((m) => ({
            name: m.name,
            x: m.position.x,
            y: m.position.y,
            width: m.size.width,
            height: m.size.height,
            scaleFactor: m.scaleFactor,
          })),
          primary: primary?.name ?? null,
        };
      } catch {
        return null;
      }
    },

    async liveRect() {
      try {
        const w = await overlayWindow();
        if (!w) return null;
        const [p, s] = await Promise.all([w.outerPosition(), w.innerSize()]);
        return { x: p.x, y: p.y, width: s.width, height: s.height };
      } catch {
        return null;
      }
    },

    async track(onSettled) {
      let timer: ReturnType<typeof setTimeout> | null = null;
      const un: Un[] = [];
      const idle: GeometryTracker = { stop: () => undefined, flush: async () => undefined };
      let w: Awaited<ReturnType<typeof overlayWindow>>;
      try {
        w = await overlayWindow();
      } catch {
        return idle;
      }
      if (!w) return idle;
      const win = w;
      /** Read where the overlay is now. */
      const read = async (): Promise<void> => {
        try {
          if (await win.isMinimized()) return;
          const [pos, size] = await Promise.all([win.outerPosition(), win.innerSize()]);
          if (!isSavablePosition(pos.x, pos.y) || size.width <= 0 || size.height <= 0) return;
          // The monitor under the overlay's centre. (`currentMonitor()` would
          // be the monitor of the window calling it, which is main.)
          const { monitorFromPoint } = await import("@tauri-apps/api/window");
          const monitor = await monitorFromPoint(pos.x + size.width / 2, pos.y + size.height / 2);
          onSettled({
            x: pos.x,
            y: pos.y,
            w: size.width,
            h: size.height,
            ...(monitor?.name ? { monitor: monitor.name } : {}),
            ...(monitor ? { scale: monitor.scaleFactor } : {}),
          });
        } catch {
          /* the window went away mid-read: keep the last good geometry */
        }
      };
      const schedule = (): void => {
        if (timer !== null) clearTimeout(timer);
        timer = setTimeout(() => {
          timer = null;
          void read();
        }, SETTLE_MS);
      };
      const stop = (): void => {
        if (timer !== null) clearTimeout(timer);
        timer = null;
        stopAll(un);
      };
      try {
        un.push(await win.onMoved(schedule), await win.onResized(schedule));
      } catch {
        /* tracking is a convenience; the window still works without it */
      }
      return {
        stop,
        flush: async () => {
          if (timer === null) return;
          clearTimeout(timer);
          timer = null;
          await read();
        },
      };
    },

    async keepFit(onSettled) {
      let timer: ReturnType<typeof setTimeout> | null = null;
      const un: Un[] = [];
      const schedule = (): void => {
        if (timer !== null) clearTimeout(timer);
        timer = setTimeout(() => {
          timer = null;
          onSettled();
        }, SETTLE_MS);
      };
      try {
        const w = await overlayWindow();
        if (w) un.push(await w.onScaleChanged(schedule), await w.onResized(schedule), await w.onMoved(schedule));
      } catch {
        /* the 5 s check still runs */
      }
      const check = setInterval(onSettled, FIT_CHECK_MS);
      return () => {
        if (timer !== null) clearTimeout(timer);
        timer = null;
        clearInterval(check);
        stopAll(un);
      };
    },
  };
}
