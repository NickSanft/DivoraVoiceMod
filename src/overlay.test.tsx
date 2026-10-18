import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { render } from "@solidjs/testing-library";

// The Tauri event bus and commands, recorded in one log so their order shows.
type Handler = (e: { event: string; id: number; payload: unknown }) => void;
const handlers = new Map<string, Set<Handler>>();
const log: string[] = [];
const emitMock = vi.fn(async (event: string, _payload?: unknown): Promise<void> => {
  log.push(`emit ${event}`);
});
const listenMock = vi.fn(async (event: string, h: Handler): Promise<() => void> => {
  log.push(`listen ${event}`);
  const set = handlers.get(event) ?? new Set<Handler>();
  handlers.set(event, set);
  set.add(h);
  return () => set.delete(h);
});
const invokeMock = vi.fn(async (cmd: string, args?: unknown): Promise<unknown> => {
  log.push(`invoke ${cmd} ${JSON.stringify(args)}`);
  return null;
});
vi.mock("@tauri-apps/api/event", () => ({
  emit: (event: string, payload?: unknown) => emitMock(event, payload),
  listen: (event: string, h: Handler) => listenMock(event, h),
}));
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (cmd: string, args?: unknown) => invokeMock(cmd, args),
}));

import { OverlayApp, pageInstance } from "./overlay";
import { HOLD_MS } from "./overlay/mirror";
import { applyOp, type OverlayScene, type SceneOp } from "./overlay/scene";
import type { OverlayState, SceneEdit, ScenePush } from "./overlay/state";
import { NO_READY_GLOBAL, type OverlayStatus } from "./overlay/status";
import fixture from "./overlay/wire.fixture.json";

const PIC = "a1b2c3d4e5f60718.gif";
const READY = 'invoke overlay_handshake {"instance":3,"stage":"ready"}';
const PAINTED = 'invoke overlay_handshake {"instance":3,"stage":"painted"}';

const STATE: OverlayState = {
  chain: [],
  status: "clean",
  motion: 0,
  mystical: 0.7,
  mood: "violet",
  accent: "brand",
  theme: "dark",
  bg: "green",
};

function scene(picX = 0.5): OverlayScene {
  return {
    version: 1,
    designSize: { w: 1024, h: 768 },
    frame: "window",
    elements: [
      { id: "circle", type: "spellcircle", x: 0.1, y: 0.1, w: 0.2, h: 0.2, z: 0 },
      { id: "pic", type: "image", x: picX, y: 0.5, w: 0.125, h: 0.078125, z: 1, src: PIC, nw: 2, nh: 1 },
    ],
  };
}

function push(s: OverlayScene, revision: number, more: Partial<ScenePush> = {}): ScenePush {
  return { session: "main-1", revision, scene: s, ack: null, expectViewport: null, ...more };
}

function status(more: Partial<OverlayStatus> = {}): OverlayStatus {
  return {
    ...(fixture.statusSamples.closed as OverlayStatus),
    open: true,
    phase: "painted",
    instance: 3,
    canArrange: true,
    ...more,
  };
}

function fire(event: string, payload: unknown): void {
  for (const h of handlers.get(event) ?? []) h({ event, id: 0, payload });
}

function edits(): SceneEdit[] {
  return emitMock.mock.calls.filter((c) => c[0] === "overlay:edit").map((c) => c[1] as SceneEdit);
}

async function settle(): Promise<void> {
  for (let i = 0; i < 10; i += 1) await Promise.resolve();
}

/** Mount the page and wait until it is listening and has said so. */
async function open() {
  const r = render(() => <OverlayApp />);
  await vi.waitFor(() => expect(log).toContain("emit overlay:ready"));
  const q = (sel: string): HTMLElement | null => r.container.querySelector<HTMLElement>(sel);
  return { ...r, q };
}

function drag(el: Element, from: [number, number], to: [number, number]): void {
  el.dispatchEvent(
    new MouseEvent("pointerdown", { clientX: from[0], clientY: from[1], button: 0, buttons: 1, bubbles: true, cancelable: true }),
  );
  window.dispatchEvent(new MouseEvent("pointermove", { clientX: to[0], clientY: to[1], buttons: 1 }));
  window.dispatchEvent(new MouseEvent("pointerup", { clientX: to[0], clientY: to[1] }));
}

beforeEach(() => {
  handlers.clear();
  log.length = 0;
  emitMock.mockClear();
  listenMock.mockClear();
  invokeMock.mockClear();
  window.history.replaceState(null, "", "/overlay.html?instance=3");
});

afterEach(() => {
  vi.useRealTimers();
  delete (window as unknown as Record<string, unknown>)[NO_READY_GLOBAL];
});

describe("pageInstance", () => {
  it.each([
    ["?instance=3", 3],
    ["?instance=4294967295", 4294967295],
    ["", 0],
    ["?instance=", 0],
    ["?instance=abc", 0],
    ["?instance=-2", 0],
    ["?instance=1.5", 0],
  ])("%s → %d", (search, n) => {
    expect(pageInstance(search)).toBe(n);
  });
});

describe("the overlay page's handshake (v1.55.0)", () => {
  it("listens first, then broadcasts ready, then tells the backend with its instance", async () => {
    await open();
    await vi.waitFor(() => expect(log).toContain(READY));
    expect(log).toEqual([
      "listen overlay:state",
      "listen overlay:scene",
      "listen overlay:status",
      "emit overlay:ready",
      READY,
    ]);
  });

  it("the debug switch skips the ready handshake, never the broadcast", async () => {
    (window as unknown as Record<string, unknown>)[NO_READY_GLOBAL] = true;
    await open();
    await settle();
    expect(log).toContain("emit overlay:ready");
    expect(log.filter((l) => l.includes("overlay_handshake"))).toEqual([]);
  });

  it("says painted two frames after the first shown status for its own window, once", async () => {
    vi.useFakeTimers({ toFake: ["requestAnimationFrame", "cancelAnimationFrame"] });
    await open();
    await vi.waitFor(() => expect(log).toContain(READY));
    const painted = () => log.filter((l) => l === PAINTED).length;

    fire("overlay:status", status({ instance: 4, phase: "shown", canArrange: false }));
    fire("overlay:status", status({ phase: "starting", canArrange: false }));
    await vi.advanceTimersByTimeAsync(100);
    expect(painted()).toBe(0);

    fire("overlay:status", status({ phase: "shown", canArrange: false }));
    await vi.advanceTimersByTimeAsync(16);
    expect(painted()).toBe(0);
    await vi.advanceTimersByTimeAsync(16);
    await settle();
    expect(painted()).toBe(1);

    fire("overlay:status", status({ phase: "shown", canArrange: false, hideWanted: true }));
    fire("overlay:status", status({ phase: "painted" }));
    await vi.advanceTimersByTimeAsync(100);
    await settle();
    expect(painted()).toBe(1);
  });

  it("a status for another window changes nothing here", async () => {
    const { q } = await open();
    fire("overlay:status", status({ instance: 4, mode: "arrange", arrangeGen: 1 }));
    expect(q("[data-chrome]")).toBeNull();
    fire("overlay:status", status({ mode: "arrange", arrangeGen: 1 }));
    expect(q("[data-chrome]")).not.toBeNull();
  });

  it("stops listening when it goes away", async () => {
    const { unmount } = await open();
    expect([...handlers.values()].reduce((n, s) => n + s.size, 0)).toBe(3);
    unmount();
    expect([...handlers.values()].reduce((n, s) => n + s.size, 0)).toBe(0);
  });

  it("the browser preview (no Tauri) draws the default circle and sends nothing", async () => {
    listenMock.mockRejectedValueOnce(new Error("not under Tauri"));
    const r = render(() => <OverlayApp />);
    await vi.waitFor(() => expect(r.container.querySelector("[data-circle]")).not.toBeNull());
    expect(r.container.querySelectorAll("[data-charm]")).toHaveLength(1);
    expect(emitMock).not.toHaveBeenCalled();
    expect(invokeMock).not.toHaveBeenCalled();
  });
});

describe("the overlay page's scene", () => {
  it("draws nothing until main's push, then the scene", async () => {
    const { q } = await open();
    expect(q("[data-charm]")).toBeNull();
    fire("overlay:state", STATE);
    fire("overlay:scene", push(scene(), 1));
    expect(q('[data-id="pic"]')!.style.left).toBe("512px");
    expect(q('[data-id="circle"] [data-circle]')).not.toBeNull();
  });

  it("an edit goes to main with this window's instance and number; the ack keeps the node", async () => {
    const { q } = await open();
    fire("overlay:state", STATE);
    fire("overlay:scene", push(scene(), 1));
    fire("overlay:status", status({ mode: "arrange", arrangeGen: 1 }));
    const node = q('[data-id="pic"]')!;
    drag(node.querySelector("[data-hit]")!, [540, 400], [560, 400]);
    await settle();
    const sent = edits();
    expect(sent).toEqual([
      {
        instance: 3,
        seq: 0,
        op: { kind: "place", id: "pic", rect: { x: 532, y: 384, w: 128, h: 60 }, viewport: { w: 1024, h: 768 } },
      },
    ]);
    // Drawn where it was dropped while the edit travels.
    expect(parseFloat(node.style.left)).toBeCloseTo(532, 3);

    // Main applies it and acks it.
    const acked = applyOp(scene(), JSON.parse(JSON.stringify(sent[0]!.op)) as SceneOp);
    fire("overlay:scene", push(acked, 2, { ack: { instance: 3, seq: 0 } }));
    expect(q('[data-id="pic"]')).toBe(node);
    expect(parseFloat(node.style.left)).toBeCloseTo(532, 3);

    // The acked edit is gone: a later push is drawn as main sent it.
    fire("overlay:scene", push(scene(), 3));
    expect(node.style.left).toBe("512px");
  });

  it("an unacknowledged edit stays drawn over a push that crosses it", async () => {
    const { q } = await open();
    fire("overlay:state", STATE);
    fire("overlay:scene", push(scene(), 1));
    fire("overlay:status", status({ mode: "arrange", arrangeGen: 1 }));
    const node = q('[data-id="pic"]')!;
    drag(node.querySelector("[data-hit]")!, [540, 400], [560, 400]);
    fire("overlay:scene", push(scene(), 2));
    expect(parseFloat(node.style.left)).toBeCloseTo(532, 3);
  });

  it("edits go to main one at a time, in the order they were made", async () => {
    const { q } = await open();
    fire("overlay:state", STATE);
    fire("overlay:scene", push(scene(), 1));
    fire("overlay:status", status({ mode: "arrange", arrangeGen: 1 }));
    let release!: () => void;
    const gate = new Promise<void>((r) => (release = r));
    emitMock.mockImplementationOnce(async (event: string) => {
      log.push(`emit ${event}`);
      await gate;
    });
    const hit = q('[data-id="pic"] [data-hit]')!;
    drag(hit, [540, 400], [560, 400]);
    drag(hit, [560, 400], [580, 400]);
    await settle();
    expect(edits().map((e) => e.seq)).toEqual([0]);
    release();
    await settle();
    expect(edits().map((e) => e.seq)).toEqual([0, 1]);
  });

  it("holds the old scene while a Fit change resizes the window, for at most HOLD_MS", async () => {
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
    const { q } = await open();
    fire("overlay:state", STATE);
    fire("overlay:scene", push(scene(), 1));
    fire("overlay:scene", push(scene(0.25), 2, { expectViewport: { w: 1920, h: 1080 } }));
    expect(q('[data-id="pic"]')!.style.left).toBe("512px");
    await vi.advanceTimersByTimeAsync(HOLD_MS - 1);
    expect(q('[data-id="pic"]')!.style.left).toBe("512px");
    await vi.advanceTimersByTimeAsync(1);
    expect(q('[data-id="pic"]')!.style.left).toBe("256px");
  });

  it("the held scene goes once the window has its new size", async () => {
    const { q } = await open();
    fire("overlay:state", STATE);
    fire("overlay:scene", push(scene(), 1));
    fire("overlay:scene", push(scene(0.25), 2, { expectViewport: { w: 800, h: 600 } }));
    expect(q('[data-id="pic"]')!.style.left).toBe("512px");
    Object.defineProperty(window, "innerWidth", { value: 800, configurable: true });
    Object.defineProperty(window, "innerHeight", { value: 600, configurable: true });
    try {
      window.dispatchEvent(new Event("resize"));
      // The new scene at the new size: its 128 px picture centred at 0.3125.
      await vi.waitFor(() => expect(q('[data-id="pic"]')!.style.left).toBe("186px"));
    } finally {
      Object.defineProperty(window, "innerWidth", { value: 1024, configurable: true });
      Object.defineProperty(window, "innerHeight", { value: 768, configurable: true });
    }
  });
});

describe("the overlay page's look", () => {
  it("a fitted window is transparent whatever the colour; unfitted, the colour shows", async () => {
    const { q } = await open();
    fire("overlay:state", STATE);
    fire("overlay:status", status({ fit: true }));
    expect(q("[data-stage]")!.style.background).toBe("transparent");
    fire("overlay:status", status({ fit: false }));
    expect(q("[data-stage]")!.style.background).not.toBe("transparent");
    expect(q("[data-stage]")!.style.background).not.toBe("");
    // The page itself stays clear: only the stage is coloured.
    expect(document.body.style.background).toBe("transparent");
  });

  it("applies main's theme to its own root", async () => {
    await open();
    fire("overlay:state", { ...STATE, mood: "ink", theme: "light", motion: 0, mystical: 1 });
    const r = document.documentElement;
    expect(r.getAttribute("data-mood")).toBe("ink");
    expect(r.getAttribute("data-theme")).toBe("light");
    expect(r.getAttribute("data-motion")).toBe("functional");
    expect(r.getAttribute("data-mystical")).toBe("rich");
  });
});
