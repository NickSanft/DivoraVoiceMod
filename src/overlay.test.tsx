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
/** What `timer_status` answers; the e2e mock's null by default. */
let timerStatus: () => Promise<unknown> = async () => null;
/** v1.62.0: what the recorder's two status reads answer; null by default
 *  (read as nothing heard). */
let recStatus: Record<string, () => Promise<unknown>> = {};
const invokeMock = vi.fn(async (cmd: string, args?: unknown): Promise<unknown> => {
  log.push(`invoke ${cmd} ${JSON.stringify(args)}`);
  if (cmd === "timer_status") return timerStatus();
  const rec = recStatus[cmd];
  return rec ? rec() : null;
});
vi.mock("@tauri-apps/api/event", () => ({
  emit: (event: string, payload?: unknown) => emitMock(event, payload),
  listen: (event: string, h: Handler) => listenMock(event, h),
}));
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (cmd: string, args?: unknown) => invokeMock(cmd, args),
}));

import { FONT_WAIT_MS, OverlayApp, pageInstance } from "./overlay";
import { HOLD_MS } from "./overlay/mirror";
import { REC_COMMANDS, REC_EVENTS } from "./overlay/recLight";
import { applyOp, type OverlayScene, type SceneOp } from "./overlay/scene";
import type { OverlayState, SceneEdit, ScenePush } from "./overlay/state";
import { NO_READY_GLOBAL, type OverlayStatus } from "./overlay/status";
import timerFixture from "./overlay/timer.fixture.json";
import { FONT_WAIT_MS as SHARED_FONT_WAIT_MS } from "./overlay/textMetrics";
import type { TimerSnapshot } from "./overlay/timerState";
import fixture from "./overlay/wire.fixture.json";

const PIC = "a1b2c3d4e5f60718.gif";
const READY = 'invoke overlay_handshake {"instance":3,"stage":"ready"}';
const PAINTED = 'invoke overlay_handshake {"instance":3,"stage":"painted"}';
const PULL = "invoke timer_status undefined";
const TAKE_READ = "invoke screen_recording_status undefined";
const REPLAY_READ = "invoke replay_buffer_status undefined";

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
  timerStatus = async () => null;
  recStatus = {};
  emitMock.mockClear();
  listenMock.mockClear();
  invokeMock.mockClear();
  window.history.replaceState(null, "", "/overlay.html?instance=3");
});

afterEach(() => {
  vi.useRealTimers();
  delete (window as unknown as Record<string, unknown>)[NO_READY_GLOBAL];
  // A test's stand-in for the font loader.
  delete (document as unknown as Record<string, unknown>).fonts;
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
    await vi.waitFor(() => expect(log).toContain(REPLAY_READ));
    expect(log).toEqual([
      "listen overlay:state",
      "listen overlay:scene",
      "listen overlay:status",
      // v1.59.0: before ready, so no change after it is missed…
      "listen timer-state",
      // v1.62.0: the recording light's two sources, likewise.
      "listen screen-recording",
      "listen replay-buffer",
      "emit overlay:ready",
      READY,
      // …and the timers, the take and Replay as they are, once each, after
      // the handshake.
      PULL,
      TAKE_READ,
      REPLAY_READ,
    ]);
  });

  it("sends the three reads together: none waits for another's answer (v1.62.0)", async () => {
    // Neither the timers nor the take ever answer.
    timerStatus = () => new Promise(() => undefined);
    recStatus[REC_COMMANDS.take] = () => new Promise(() => undefined);
    await open();
    await vi.waitFor(() => expect(log).toContain(REPLAY_READ));
    const reads = log.slice(log.indexOf(READY) + 1);
    expect(reads).toEqual([PULL, TAKE_READ, REPLAY_READ]);
  });

  it("the debug switch skips the ready handshake, never the broadcast", async () => {
    (window as unknown as Record<string, unknown>)[NO_READY_GLOBAL] = true;
    await open();
    await settle();
    expect(log).toContain("emit overlay:ready");
    expect(log.filter((l) => l.includes("overlay_handshake"))).toEqual([]);
    // The timers are still read; v1.62.0: and the take and Replay, once each.
    expect(log.filter((l) => l === PULL)).toHaveLength(1);
    expect(log.filter((l) => l === TAKE_READ)).toHaveLength(1);
    expect(log.filter((l) => l === REPLAY_READ)).toHaveLength(1);
    expect(log.slice(log.indexOf("emit overlay:ready") + 1)).toEqual([PULL, TAKE_READ, REPLAY_READ]);
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
    // v1.62.0: six, with the recorder's two events.
    expect([...handlers.values()].reduce((n, s) => n + s.size, 0)).toBe(6);
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

describe("the overlay page's timers (v1.59.0)", () => {
  const SAMPLES = timerFixture.samples as Record<"idle" | "running" | "done", TimerSnapshot>;

  /** The scene and a 384×96 face for `timer-1`, in front. */
  function timerScene(): OverlayScene {
    const s = scene();
    s.elements.push({ id: "face", type: "timer", x: 0.25, y: 0.1, w: 0.375, h: 0.125, z: 2, timer: "timer-1" });
    return s;
  }

  // vi.waitFor moves fake timers on while it polls, which would run out the
  // font wait before a test could look; these polls use the real clock.
  const realTimeout = globalThis.setTimeout.bind(globalThis);
  async function until(check: () => boolean): Promise<void> {
    for (let i = 0; i < 400 && !check(); i += 1) await new Promise((r) => realTimeout(r, 5));
    expect(check()).toBe(true);
  }

  /** Open, with main's state and the scene drawn. */
  async function shown() {
    const r = render(() => <OverlayApp />);
    await until(() => log.includes("emit overlay:ready"));
    const q = (sel: string): HTMLElement | null => r.container.querySelector<HTMLElement>(sel);
    fire("overlay:state", STATE);
    fire("overlay:scene", push(timerScene(), 1));
    expect(q('[data-id="face"]')).not.toBeNull();
    return { ...r, q, face: () => q('[data-id="face"] [data-timer-face]') };
  }

  function stubFonts(load: (font: string) => Promise<unknown>): ReturnType<typeof vi.fn> {
    const spy = vi.fn(load);
    Object.defineProperty(document, "fonts", { value: { load: spy }, configurable: true });
    return spy;
  }

  it("hears timer-state itself and keeps the snapshot with the largest seq", async () => {
    const { face } = await shown();
    await vi.waitFor(() => expect(log).toContain(PULL));
    // The pull answered null (no timers yet): nothing drawn.
    expect(face()).toBeNull();
    fire("timer-state", SAMPLES.running);
    expect(face()!.textContent).toBe("04:59");
    fire("timer-state", SAMPLES.idle);
    expect(face()!.textContent).toBe("04:59");
    fire("timer-state", SAMPLES.done);
    expect(face()!.textContent).toBe("00:00");
    fire("timer-state", { seq: 9, timers: "none" });
    fire("timer-state", null);
    expect(face()!.textContent).toBe("00:00");
    // Never through main.
    expect(emitMock.mock.calls.map((c) => c[0])).not.toContain("timer-state");
  });

  it("a paused timer's face comes from timer_status alone", async () => {
    timerStatus = async () => SAMPLES.idle;
    const { face } = await shown();
    await vi.waitFor(() => expect(face()?.textContent).toBe("05:00"));
  });

  it("a late timer_status answer with a lower seq doesn't replace an event", async () => {
    let answer!: (v: unknown) => void;
    timerStatus = () => new Promise((r) => (answer = r));
    const { face } = await shown();
    await vi.waitFor(() => expect(log).toContain(PULL));
    fire("timer-state", SAMPLES.running);
    answer(SAMPLES.idle);
    await settle();
    expect(face()!.textContent).toBe("04:59");
    // A newer one still wins.
    fire("timer-state", SAMPLES.done);
    expect(face()!.textContent).toBe("00:00");
  });

  it("a failed timer_status leaves the events to tell", async () => {
    timerStatus = async () => {
      throw new Error("not under Tauri");
    };
    const { face } = await shown();
    await vi.waitFor(() => expect(log).toContain(PULL));
    await settle();
    fire("timer-state", SAMPLES.running);
    expect(face()!.textContent).toBe("04:59");
  });

  it("waits for Space Mono Bold before the first face, at most FONT_WAIT_MS", async () => {
    const load = stubFonts(() => new Promise(() => undefined));
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
    const { face } = await shown();
    fire("timer-state", SAMPLES.running);
    expect(load).toHaveBeenCalledWith('700 1em "Space Mono"');
    expect(FONT_WAIT_MS).toBe(1000);
    await vi.advanceTimersByTimeAsync(FONT_WAIT_MS - 1);
    expect(face()).toBeNull();
    await vi.advanceTimersByTimeAsync(1);
    expect(face()!.textContent).toBe("04:59");
  });

  it.each([
    ["loads", true],
    ["fails to load", false],
  ])("a font that %s lets the face draw at once", async (_name, ok) => {
    let done!: () => void;
    stubFonts(() => new Promise<void>((res, rej) => (done = ok ? res : () => rej(new Error("no font")))));
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
    const { face } = await shown();
    fire("timer-state", SAMPLES.running);
    expect(face()).toBeNull();
    done();
    await settle();
    expect(face()!.textContent).toBe("04:59");
  });

  it("where the page can't wait for fonts, the face draws at once", async () => {
    expect((document as { fonts?: unknown }).fonts).toBeUndefined();
    const { face } = await shown();
    fire("timer-state", SAMPLES.running);
    expect(face()!.textContent).toBe("04:59");
  });
});

describe("the overlay page's recording light (v1.62.0)", () => {
  /** The scene and the light, in front. */
  function lightScene(): OverlayScene {
    const s = scene();
    s.elements.push({ id: "light", type: "rec", x: 0.04, y: 0.9, w: 0.2, h: 0.04, z: 2 });
    return s;
  }

  const RECORDING = { state: "recording", elapsedMs: 65_000, path: "C:/x.mp4", droppedFrames: 0 };
  const HIDE = { hideWanted: true, hideAvailable: true };

  /** Open, with main's state and the scene drawn. */
  async function shown() {
    const r = await open();
    fire("overlay:state", STATE);
    fire("overlay:scene", push(lightScene(), 1));
    expect(r.q('[data-id="light"]')).not.toBeNull();
    return { ...r, face: () => r.q('[data-id="light"] [data-rec-face]')?.textContent ?? null };
  }

  /** A read whose answer the test gives, or fails. */
  function held(cmd: string) {
    let answer!: (v: unknown) => void;
    let fail!: (e: unknown) => void;
    recStatus[cmd] = () =>
      new Promise((res, rej) => {
        answer = res;
        fail = rej;
      });
    return { answer: (v: unknown) => answer(v), fail: () => fail(new Error("not under Tauri")) };
  }

  it("hears the recorder's events itself, never through main", async () => {
    const { face } = await shown();
    fire("overlay:status", status());
    expect(face()).toBeNull();
    fire(REC_EVENTS.take, { state: "starting", elapsedMs: 0 });
    expect(face()).toBe("REC");
    fire(REC_EVENTS.take, RECORDING);
    expect(face()).toBe("REC 1:05");
    // An unreadable report changes nothing.
    fire(REC_EVENTS.take, { state: "paused", elapsedMs: 70_000 });
    fire(REC_EVENTS.take, null);
    expect(face()).toBe("REC 1:05");
    fire(REC_EVENTS.take, { state: "idle", elapsedMs: 0 });
    expect(face()).toBeNull();
    fire(REC_EVENTS.replay, { state: "armed", seconds: 30 });
    expect(face()).toBe("REPLAY");
    fire(REC_EVENTS.replay, { state: "held", heldReason: "engineStopped" });
    expect(face()).toBeNull();
    expect(emitMock.mock.calls.map((c) => c[0])).not.toContain(REC_EVENTS.take);
    expect(emitMock.mock.calls.map((c) => c[0])).not.toContain(REC_EVENTS.replay);
  });

  it("a page opened mid-take shows the take from its read", async () => {
    recStatus[REC_COMMANDS.take] = async () => RECORDING;
    recStatus[REC_COMMANDS.replay] = async () => ({ state: "off" });
    const { face } = await shown();
    fire("overlay:status", status());
    await vi.waitFor(() => expect(face()).toBe("REC 1:05"));
  });

  it("an answer with no event meanwhile is kept", async () => {
    const take = held(REC_COMMANDS.take);
    const { face } = await shown();
    fire("overlay:status", status());
    await vi.waitFor(() => expect(log).toContain(TAKE_READ));
    take.answer(RECORDING);
    await settle();
    expect(face()).toBe("REC 1:05");
  });

  it("an event that comes while the read is on its way wins over its answer", async () => {
    const take = held(REC_COMMANDS.take);
    const { face } = await shown();
    fire("overlay:status", status());
    await vi.waitFor(() => expect(log).toContain(TAKE_READ));
    fire(REC_EVENTS.take, { state: "idle", elapsedMs: 0 });
    take.answer(RECORDING);
    await settle();
    expect(face()).toBeNull();
    // Later events still apply.
    fire(REC_EVENTS.take, { state: "starting", elapsedMs: 0 });
    expect(face()).toBe("REC");
  });

  it("a Replay event in flight doesn't drop the take's answer: one count per kind", async () => {
    const take = held(REC_COMMANDS.take);
    const { face } = await shown();
    fire("overlay:status", status());
    await vi.waitFor(() => expect(log).toContain(TAKE_READ));
    fire(REC_EVENTS.replay, { state: "armed" });
    expect(face()).toBe("REPLAY");
    take.answer(RECORDING);
    await settle();
    // The take outranks Replay.
    expect(face()).toBe("REC 1:05");
  });

  it("a take event in flight doesn't drop Replay's answer", async () => {
    const replay = held(REC_COMMANDS.replay);
    const { face } = await shown();
    fire("overlay:status", status());
    await vi.waitFor(() => expect(log).toContain(REPLAY_READ));
    fire(REC_EVENTS.take, { state: "idle", elapsedMs: 0 });
    replay.answer({ state: "armed" });
    await settle();
    expect(face()).toBe("REPLAY");
  });

  it("a Replay event in flight wins over Replay's answer", async () => {
    const replay = held(REC_COMMANDS.replay);
    const { face } = await shown();
    fire("overlay:status", status());
    await vi.waitFor(() => expect(log).toContain(REPLAY_READ));
    fire(REC_EVENTS.replay, { state: "off" });
    replay.answer({ state: "armed" });
    await settle();
    expect(face()).toBeNull();
  });

  it("a failed read changes nothing, and events still apply", async () => {
    const take = held(REC_COMMANDS.take);
    const { face } = await shown();
    fire("overlay:status", status());
    await vi.waitFor(() => expect(log).toContain(TAKE_READ));
    take.fail();
    await settle();
    expect(face()).toBeNull();
    fire(REC_EVENTS.take, RECORDING);
    expect(face()).toBe("REC 1:05");
  });

  it("Locked, nothing is drawn until this window's own status, and the Hide gate then applies", async () => {
    recStatus[REC_COMMANDS.take] = async () => RECORDING;
    const { face } = await shown();
    await vi.waitFor(() => expect(log).toContain(TAKE_READ));
    await settle();
    // The take is known, but no status for this window has come.
    expect(face()).toBeNull();
    fire("overlay:status", status({ instance: 4, ...HIDE, capturing: true }));
    expect(face()).toBeNull();
    // Its own: Hide on, and Windows not asked yet.
    fire("overlay:status", status({ ...HIDE, capturing: false }));
    expect(face()).toBeNull();
    fire("overlay:status", status({ ...HIDE, capturing: true }));
    expect(face()).toBe("REC 1:05");
    // Stop: the take's part goes while the overlay is still left out.
    fire(REC_EVENTS.take, { ...RECORDING, state: "stopping" });
    expect(face()).toBeNull();
  });

  it("Arrange draws it whatever the gate says, and the placeholder when idle", async () => {
    const r = await shown();
    fire("overlay:status", status({ mode: "arrange", arrangeGen: 1, ...HIDE, capturing: false }));
    expect(r.q('[data-id="light"] [data-rec-idle]')!.textContent).toBe("REC");
    fire(REC_EVENTS.take, RECORDING);
    expect(r.face()).toBe("REC 1:05");
  });

  it("waits for Space Mono Bold as the timer face does, at most FONT_WAIT_MS", async () => {
    // vi.waitFor would move the fake clock on and run out the wait: poll on
    // the real one.
    const realTimeout = globalThis.setTimeout.bind(globalThis);
    const until = async (check: () => boolean): Promise<void> => {
      for (let i = 0; i < 400 && !check(); i += 1) await new Promise((res) => realTimeout(res, 5));
      expect(check()).toBe(true);
    };
    const load = vi.fn((_font: string) => new Promise(() => undefined));
    Object.defineProperty(document, "fonts", { value: { load }, configurable: true });
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
    const r = render(() => <OverlayApp />);
    await until(() => log.includes("emit overlay:ready"));
    const face = () => r.container.querySelector('[data-id="light"] [data-rec-face]')?.textContent ?? null;
    fire("overlay:state", STATE);
    fire("overlay:scene", push(lightScene(), 1));
    fire("overlay:status", status());
    fire(REC_EVENTS.replay, { state: "armed" });
    expect(load).toHaveBeenCalledWith('700 1em "Space Mono"');
    await vi.advanceTimersByTimeAsync(FONT_WAIT_MS - 1);
    expect(face()).toBeNull();
    await vi.advanceTimersByTimeAsync(1);
    expect(face()).toBe("REPLAY");
  });
});

describe("the overlay page's text charms (v1.62.0)", () => {
  // vi.waitFor moves fake timers on while it polls; these polls use the
  // real clock.
  const realTimeout = globalThis.setTimeout.bind(globalThis);
  async function until(check: () => boolean): Promise<void> {
    for (let i = 0; i < 400 && !check(); i += 1) await new Promise((r) => realTimeout(r, 5));
    expect(check()).toBe(true);
  }

  afterEach(() => {
    vi.restoreAllMocks();
  });

  it("FONT_WAIT_MS is still exported here, and is the one text measures share", () => {
    expect(FONT_WAIT_MS).toBe(1000);
    expect(FONT_WAIT_MS).toBe(SHARED_FONT_WAIT_MS);
  });

  it("measures on the page's canvas with no cap of its own: a font later than FONT_WAIT_MS is measured again when it comes", async () => {
    // A canvas whose letters widen once the font has come.
    const font = { loaded: false };
    const ctx = {
      font: "",
      fontKerning: "",
      letterSpacing: "",
      textRendering: "",
      measureText: (line: string) => ({ width: (font.loaded ? 150 : 50) * line.length }),
    };
    vi.spyOn(HTMLCanvasElement.prototype, "getContext").mockReturnValue(ctx as unknown as CanvasRenderingContext2D);
    let arrive!: () => void;
    const textLoad = new Promise<void>((r) => (arrive = r));
    const load = vi.fn((_spec: string, sample?: string) => (sample === undefined ? Promise.resolve([]) : textLoad));
    Object.defineProperty(document, "fonts", { value: { load }, configurable: true });
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });

    const r = render(() => <OverlayApp />);
    await until(() => log.includes("emit overlay:ready"));
    const q = (sel: string): HTMLElement | null => r.container.querySelector<HTMLElement>(sel);
    fire("overlay:state", STATE);
    const s = scene();
    s.elements.push({
      id: "t",
      type: "text",
      x: 0.1,
      y: 0.1,
      w: 337 / 1024,
      h: 139 / 768,
      z: 2,
      text: "BRB",
      font: "display",
      bold: false,
      color: "#ffffff",
      outline: true,
      align: "left",
      nw: 337,
      nh: 139,
    });
    fire("overlay:scene", push(s, 1));
    fire("overlay:status", status());
    const face = () => q('[data-id="t"] [data-text-face]');
    // Its words, and a space for the font's own first face (textMetrics.ts).
    expect(load).toHaveBeenCalledWith('400 100px "Bricolage Grotesque Variable", system-ui, sans-serif', "BRB ");
    await vi.advanceTimersByTimeAsync(FONT_WAIT_MS - 1);
    expect(face()).toBeNull();
    await vi.advanceTimersByTimeAsync(1);
    const before = parseFloat(face()!.style.fontSize);
    // The canvas was given the face's settings with the spec.
    expect(ctx.font).toBe('400 100px "Bricolage Grotesque Variable", system-ui, sans-serif');
    expect(ctx.fontKerning).toBe("normal");
    expect(ctx.letterSpacing).toBe("0px");
    expect(ctx.textRendering).toBe("optimizeLegibility");
    // The font comes half a second late.
    await vi.advanceTimersByTimeAsync(500);
    font.loaded = true;
    arrive();
    await vi.advanceTimersByTimeAsync(0);
    const after = parseFloat(face()!.style.fontSize);
    expect(after).toBeLessThan(before);
    expect(face()!.firstElementChild!.textContent).toBe("BRB");
  });
});
