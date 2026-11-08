import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { render } from "@solidjs/testing-library";
import { createSignal } from "solid-js";

const invokeMock = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
}));
const startDragging = vi.fn(() => Promise.resolve());
vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: () => ({ startDragging }),
}));

import { readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { SPELL_CIRCLE_PX } from "../components/SpellCircle";
import { NUDGE_FLUSH_MS } from "./editor";
import { LIGHT_RED, type ReplayView, type TakeView } from "./recLight";
import { applyOp, layoutScene, type OverlayScene, type PxRect, type SceneOp, type Size } from "./scene";
import { SNAP_HINT, BEAT_MS, circleFit, Stage } from "./Stage";
import type { OverlayBg, OverlayState } from "./state";
import type { OverlayStatus } from "./status";
import { drawText, fitTextFont, FONTS, fontSpec, textBox, textShadow, type TextMetrics } from "./text";
import { FONT_WAIT_MS } from "./textMetrics";
import timerFixture from "./timer.fixture.json";
import { fitTimerFont, type TimerSnapshot } from "./timerState";
import fixture from "./wire.fixture.json";

const PIC = "a1b2c3d4e5f60718.gif";
const V480: Size = { w: 480, h: 480 };

const STATE: OverlayState = {
  chain: [{ id: "pitch", enabled: true, vals: {} }],
  status: "clean",
  motion: 0,
  mystical: 0.7,
  mood: "violet",
  accent: "brand",
  theme: "dark",
  bg: "transparent",
};

function arranging(more: Partial<OverlayStatus> = {}): OverlayStatus {
  return { ...(fixture.statusSamples.arranging as OverlayStatus), instance: 3, capturing: false, ...more };
}

function locked(more: Partial<OverlayStatus> = {}): OverlayStatus {
  return arranging({ mode: "locked", ...more });
}

/** The migrated circle (20,20 440×440 at 480) and a 120×60 picture at
 *  (240,240) in front of it. */
function baseScene(): OverlayScene {
  return {
    version: 1,
    designSize: { w: 480, h: 480 },
    frame: "window",
    elements: [
      { id: "circle", type: "spellcircle", x: 0.041667, y: 0.041667, w: 0.916667, h: 0.916667, z: 0 },
      { id: "pic", type: "image", x: 0.5, y: 0.5, w: 0.25, h: 0.125, z: 1, src: PIC, nw: 2, nh: 1, name: "wave.gif" },
    ],
  };
}

interface Mount {
  scene?: OverlayScene | null;
  status?: OverlayStatus;
  viewport?: Size;
  state?: OverlayState | null;
  background?: OverlayBg;
  /** Apply each edit to the scene, as the overlay's mirror does. */
  apply?: boolean;
  timers?: TimerSnapshot | null;
  fontReady?: boolean;
  /** v1.62.0: the recording light's sources. */
  take?: TakeView | null;
  replay?: ReplayView | null;
  statusSeen?: boolean;
  /** v1.62.0: how text rows measure; the page's canvas when absent. */
  textMetrics?: TextMetrics;
}

function mount(o: Mount = {}) {
  const [scene, setScene] = createSignal<OverlayScene | null>(o.scene === undefined ? baseScene() : o.scene);
  const [status, setStatus] = createSignal<OverlayStatus>(o.status ?? arranging());
  const [viewport, setViewport] = createSignal<Size>(o.viewport ?? V480);
  const [state, setState] = createSignal<OverlayState | null>(o.state === undefined ? STATE : o.state);
  const [timers, setTimers] = createSignal<TimerSnapshot | null>(o.timers ?? null);
  const [fontReady, setFontReady] = createSignal(o.fontReady ?? true);
  const [take, setTake] = createSignal<TakeView | null>(o.take ?? null);
  const [replay, setReplay] = createSignal<ReplayView | null>(o.replay ?? null);
  const [statusSeen, setStatusSeen] = createSignal(o.statusSeen ?? false);
  const edits: SceneOp[] = [];
  const onEdit = (op: SceneOp): void => {
    edits.push(op);
    if (o.apply !== false) setScene((s) => (s ? applyOp(s, op) : s));
  };
  const r = render(() => (
    <Stage
      instance={3}
      state={state()}
      scene={scene()}
      status={status()}
      viewport={viewport()}
      background={o.background}
      timers={timers()}
      fontReady={fontReady()}
      take={take()}
      replay={replay()}
      statusSeen={statusSeen()}
      textMetrics={o.textMetrics}
      onEdit={onEdit}
    />
  ));
  const q = (sel: string): HTMLElement | null => r.container.querySelector<HTMLElement>(sel);
  const charm = (id: string): HTMLElement => q(`[data-charm][data-id="${id}"]`)!;
  const hitOf = (id: string): HTMLElement => charm(id).querySelector<HTMLElement>("[data-hit]")!;
  return {
    ...r,
    q,
    charm,
    hitOf,
    edits,
    scene,
    setScene,
    setStatus,
    setViewport,
    setState,
    setTimers,
    setFontReady,
    setTake,
    setReplay,
    setStatusSeen,
  };
}

function rectOf(el: HTMLElement): PxRect {
  return {
    x: parseFloat(el.style.left),
    y: parseFloat(el.style.top),
    w: parseFloat(el.style.width),
    h: parseFloat(el.style.height),
  };
}

function expectNear(a: PxRect, b: PxRect, tol = 1e-6): void {
  for (const k of ["x", "y", "w", "h"] as const) {
    expect(Math.abs(a[k] - b[k]), `${k}: ${a[k]} vs ${b[k]}`).toBeLessThanOrEqual(tol);
  }
}

// jsdom has no PointerEvent; a mouse event of that type carries the rest.
function down(el: Element, x: number, y: number, more: MouseEventInit = {}): MouseEvent {
  const e = new MouseEvent("pointerdown", {
    clientX: x,
    clientY: y,
    button: 0,
    buttons: 1,
    bubbles: true,
    cancelable: true,
    ...more,
  });
  el.dispatchEvent(e);
  return e;
}

function move(x: number, y: number, buttons = 1): void {
  window.dispatchEvent(new MouseEvent("pointermove", { clientX: x, clientY: y, buttons, bubbles: true }));
}

function up(x: number, y: number): void {
  window.dispatchEvent(new MouseEvent("pointerup", { clientX: x, clientY: y, bubbles: true }));
}

function key(k: string, more: KeyboardEventInit = {}, target: EventTarget = window): KeyboardEvent {
  const e = new KeyboardEvent("keydown", { key: k, bubbles: true, cancelable: true, ...more });
  target.dispatchEvent(e);
  return e;
}

/** Let the lazily imported Tauri calls run. */
async function settle(): Promise<void> {
  for (let i = 0; i < 5; i += 1) await Promise.resolve();
  await new Promise((r) => setTimeout(r, 0));
}

function aliveCalls(): unknown[] {
  return invokeMock.mock.calls.filter((c) => c[0] === "overlay_alive").map((c) => c[1]);
}

beforeEach(() => {
  invokeMock.mockReset();
  invokeMock.mockResolvedValue(null);
  startDragging.mockClear();
});

afterEach(() => {
  vi.useRealTimers();
});

describe("Stage: drawing the scene", () => {
  it("positions every charm where layoutScene puts it", () => {
    const view = { w: 300, h: 360 };
    const { charm } = mount({ viewport: view, status: locked() });
    for (const { el, rect } of layoutScene(baseScene(), view)) {
      expectNear(rectOf(charm(el.id)), rect);
    }
  });

  it("stacks by layout rank and keeps rows in scene order", () => {
    const s = baseScene();
    s.elements[0]!.z = 5; // the circle in front of the picture
    const { container, charm } = mount({ scene: s, status: locked() });
    const rows = [...container.querySelectorAll<HTMLElement>("[data-charm]")].map((r) => r.dataset.id);
    expect(rows).toEqual(["circle", "pic"]);
    expect(charm("circle").style.zIndex).toBe("1");
    expect(charm("pic").style.zIndex).toBe("0");
  });

  it("scales the circle from SPELL_CIRCLE_PX: a stored 300×200 circle draws 200×200", () => {
    const s = baseScene();
    s.elements = [{ id: "circle", type: "spellcircle", x: 0.1, y: 0.2, w: 300 / 480, h: 200 / 480, z: 0 }];
    const { charm } = mount({ scene: s, status: locked() });
    const row = charm("circle");
    expectNear(rectOf(row), { x: 48 + 50, y: 96, w: 200, h: 200 });
    const box = row.querySelector<HTMLElement>("[data-circle]")!;
    expect(box.style.width).toBe(`${SPELL_CIRCLE_PX}px`);
    expect(box.style.height).toBe(`${SPELL_CIRCLE_PX}px`);
    expect(box.style.transform).toBe(`translate(0px, 0px) scale(${200 / SPELL_CIRCLE_PX})`);
    expect(box.style.transformOrigin).toBe("0 0");
  });

  it("a rect a hair off square scales the circle by its short side, centred", () => {
    const wide = circleFit({ w: 300.001, h: 299.999 });
    expect(wide.k).toBeCloseTo(299.999 / SPELL_CIRCLE_PX, 12);
    expect(wide.x).toBeCloseTo(0.001, 9);
    expect(wide.y).toBeCloseTo(0, 9);
    const tall = circleFit({ w: 299.999, h: 300.001 });
    expect(tall.k).toBeCloseTo(299.999 / SPELL_CIRCLE_PX, 12);
    expect(tall.x).toBeCloseTo(0, 9);
    expect(tall.y).toBeCloseTo(0.001, 9);
    expect(circleFit({ w: SPELL_CIRCLE_PX, h: SPELL_CIRCLE_PX })).toEqual({ k: 1, x: 0, y: 0 });
  });

  it("the circle's box takes circleFit of its row", () => {
    const s = baseScene();
    s.elements = [{ id: "circle", type: "spellcircle", x: 0.1, y: 0.1, w: 300.001 / 480, h: 299.999 / 480, z: 0 }];
    const { charm } = mount({ scene: s, status: locked() });
    const row = charm("circle");
    const fit = circleFit(rectOf(row));
    expect(fit.k).toBeCloseTo(299.999 / SPELL_CIRCLE_PX, 9);
    const box = row.querySelector<HTMLElement>("[data-circle]")!;
    expect(box.style.transform).toBe(`translate(${fit.x}px, ${fit.y}px) scale(${fit.k})`);
  });

  it("draws no circle until main's state arrives, and no charm until a scene does", () => {
    const { charm, q, setState, setScene } = mount({ state: null, scene: null, status: locked() });
    expect(q("[data-charm]")).toBeNull();
    setScene(baseScene());
    expect(charm("circle").querySelector("[data-circle]")).toBeNull();
    expect(charm("pic").querySelector("img")).not.toBeNull();
    setState(STATE);
    expect(charm("circle").querySelector("[data-circle]")).not.toBeNull();
  });

  it("loads pictures from the charm scheme, never draggable", () => {
    const { charm } = mount({ status: locked() });
    const img = charm("pic").querySelector("img")!;
    expect(img.getAttribute("src")).toBe(`http://charm.localhost/${PIC}`);
    expect(img.getAttribute("draggable")).toBe("false");
    expect(img.getAttribute("alt")).toBe("");
    expect(charm("pic").getAttribute("aria-label")).toBe("Picture: wave.gif");
    expect(charm("circle").getAttribute("aria-label")).toBe("Spell circle");
  });

  it("paints the chosen background, and transparent without one", () => {
    const green = mount({ status: locked(), background: "green" });
    expect(green.q("[data-stage]")!.style.background).not.toBe("transparent");
    expect(green.q("[data-stage]")!.style.background).not.toBe("");
    green.unmount();
    const none = mount({ status: locked() });
    expect(none.q("[data-stage]")!.style.background).toBe("transparent");
  });

  it("has no title anywhere, even arranging with a selection and a missing picture", () => {
    const { container, charm, hitOf } = mount();
    down(hitOf("pic"), 250, 250);
    up(250, 250);
    charm("pic").querySelector("img")!.dispatchEvent(new Event("error"));
    expect(container.querySelector("[data-handle]")).not.toBeNull();
    expect(container.textContent).toContain("Picture missing");
    expect(container.querySelectorAll("[title]")).toHaveLength(0);
    expect(container.querySelector("select")).toBeNull();
  });

  it("says a picture is missing only while arranging", () => {
    const { container, charm, setStatus } = mount({ status: locked() });
    charm("pic").querySelector("img")!.dispatchEvent(new Event("error"));
    expect(container.textContent).not.toContain("Picture missing");
    setStatus(arranging());
    expect(container.textContent).toContain("Picture missing");
    charm("pic").querySelector("img")!.dispatchEvent(new Event("load"));
    expect(container.textContent).not.toContain("Picture missing");
  });
});

describe("Stage: the Arrange chrome", () => {
  it("Locked draws no chrome, no handles and no outlines", () => {
    const { q } = mount({ status: locked() });
    expect(q("[data-chrome]")).toBeNull();
    expect(q("[data-handle]")).toBeNull();
    expect(q("[data-outline]")).toBeNull();
    expect(q("[data-grip]")).toBeNull();
    expect(q("[data-charm]")!.tabIndex).toBe(-1);
  });

  it("Arrange shows the frame, Lock first in the toolbar, the grip and the hints", () => {
    const { q, container } = mount();
    const chrome = q("[data-chrome]")!;
    expect(chrome.getAttribute("role")).toBe("toolbar");
    expect(chrome.getAttribute("aria-label")).toBe("Arrange");
    const first = chrome.querySelector("button, [tabindex], a, input")!;
    expect(first).toBe(q("[data-lock]"));
    expect(first.getAttribute("aria-label")).toBe("Lock the overlay");
    expect(first.textContent).toBe("Lock");
    const grip = q("[data-grip]")!;
    expect(grip.getAttribute("aria-label")).toBe("Move window");
    expect(grip.textContent).toBe("Move window");
    expect(grip.hasAttribute("tabindex")).toBe(false);
    expect(q("[data-hints]")).not.toBeNull();
    expect(container.querySelectorAll("[data-outline]")).toHaveLength(2);
    expect(q("[data-charm]")!.tabIndex).toBe(0);
  });

  it("Lock asks the backend to lock", async () => {
    const { q } = mount();
    q("[data-lock]")!.click();
    await settle();
    expect(invokeMock).toHaveBeenCalledWith("set_overlay_mode", { mode: "locked" });
  });

  it("no grip while fitted", () => {
    const { q } = mount({ status: arranging({ fit: true }) });
    expect(q("[data-chrome]")).not.toBeNull();
    expect(q("[data-grip]")).toBeNull();
  });

  it("small windows keep Lock and the grip as icons, and drop the hints", () => {
    const { q, setViewport } = mount({ viewport: { w: 136, h: 39 } });
    expect(q("[data-lock]")!.textContent).toBe("");
    expect(q("[data-grip]")!.textContent).toBe("");
    expect(q("[data-hints]")).toBeNull();
    setViewport({ w: 200, h: 200 });
    expect(q("[data-lock]")!.textContent).toBe("Lock");
    expect(q("[data-grip]")!.textContent).toBe("");
    expect(q("[data-hints]")).toBeNull();
    setViewport({ w: 319, h: 480 });
    expect(q("[data-hints]")).toBeNull();
    setViewport({ w: 480, h: 119 });
    expect(q("[data-hints]")).toBeNull();
    setViewport({ w: 320, h: 120 });
    expect(q("[data-hints]")).not.toBeNull();
    expect(q("[data-grip]")!.textContent).toBe("Move window");
  });

  it("the hint follows whether the overlay has the keyboard", () => {
    const { q } = mount();
    window.dispatchEvent(new Event("focus"));
    // v1.62.0: the snapping line follows it, always in Arrange.
    expect(q("[data-hints]")!.textContent).toBe("Esc locks the overlay" + SNAP_HINT);
    window.dispatchEvent(new Event("blur"));
    expect(q("[data-hints]")!.textContent).toBe("Click Lock to lock the overlay" + SNAP_HINT);
  });

  it("says charms snap and that Ctrl places one freely (v1.62.0)", () => {
    expect(SNAP_HINT).toBe("Charms snap to edges and centres. Hold Ctrl to place one freely.");
    const { q } = mount();
    const lines = [...q("[data-hints]")!.children].map((c) => c.textContent);
    expect(lines).toContain(SNAP_HINT);
    // Its own line, beside the others.
    expect(lines.filter((l) => l === SNAP_HINT)).toHaveLength(1);
  });

  it("warns that recordings see the overlay only while capturing with Hide on", () => {
    const { q, setStatus } = mount({
      status: arranging({ capturing: true, hideWanted: false, hideAvailable: true }),
    });
    const warning = "Recordings can see the overlay while you arrange.";
    expect(q("[data-hints]")!.textContent).not.toContain(warning);
    setStatus(arranging({ capturing: true, hideWanted: true, hideAvailable: true }));
    expect(q("[data-hints]")!.textContent).toContain(warning);
    setStatus(arranging({ capturing: false, hideWanted: true, hideAvailable: true }));
    expect(q("[data-hints]")!.textContent).not.toContain(warning);
  });

  it("no recordings warning where Windows can't hide the overlay, even with Hide on", () => {
    // The stored flag reaches the backend even where the Settings row is
    // hidden, so hideWanted can be true here; Locked doesn't hide it either.
    const { q } = mount({ status: arranging({ capturing: true, hideWanted: true, hideAvailable: false }) });
    expect(q("[data-hints]")).not.toBeNull();
    expect(q("[data-hints]")!.textContent).not.toContain("Recordings can see the overlay");
  });

  it("entering Arrange gives the stage the keyboard", () => {
    const { q, setStatus } = mount({ status: locked() });
    expect(document.activeElement).not.toBe(q("[data-stage]"));
    setStatus(arranging());
    expect(document.activeElement).toBe(q("[data-stage]"));
  });

  it("another status while arranging (a take starting) leaves the keyboard where it is", () => {
    const { q, charm, setStatus } = mount({ status: locked() });
    setStatus(arranging());
    charm("circle").focus();
    expect(document.activeElement).toBe(charm("circle"));
    setStatus(arranging({ capturing: true }));
    expect(document.activeElement).toBe(charm("circle"));
    const lock = q("[data-lock]")!;
    lock.focus();
    setStatus(arranging({ capturing: true, fit: true }));
    expect(document.activeElement).toBe(lock);
  });
});

describe("Stage: the lease's beats", () => {
  beforeEach(() => {
    vi.useFakeTimers({
      toFake: ["setTimeout", "clearTimeout", "setInterval", "clearInterval", "requestAnimationFrame", "cancelAnimationFrame"],
    });
  });

  it("none while Locked; two frames after the chrome, then every 500 ms with the generation; none after Lock", async () => {
    const { setStatus, q } = mount({ status: locked() });
    await vi.advanceTimersByTimeAsync(2000);
    expect(aliveCalls()).toEqual([]);

    setStatus(arranging({ arrangeGen: 4 }));
    expect(q("[data-chrome]")).not.toBeNull();
    await vi.advanceTimersByTimeAsync(0);
    expect(aliveCalls()).toEqual([]);
    // Two animation frames.
    await vi.advanceTimersByTimeAsync(40);
    expect(aliveCalls()).toEqual([{ instance: 3, gen: 4 }]);
    await vi.advanceTimersByTimeAsync(BEAT_MS);
    expect(aliveCalls()).toHaveLength(2);
    await vi.advanceTimersByTimeAsync(BEAT_MS * 2);
    expect(aliveCalls()).toHaveLength(4);

    setStatus(locked());
    await vi.advanceTimersByTimeAsync(BEAT_MS * 4);
    expect(aliveCalls()).toHaveLength(4);
  });

  it("a new Arrange beats with its own generation", async () => {
    const { setStatus } = mount({ status: arranging({ arrangeGen: 1 }) });
    await vi.advanceTimersByTimeAsync(40);
    setStatus(locked({ arrangeGen: 1 }));
    setStatus(arranging({ arrangeGen: 2 }));
    await vi.advanceTimersByTimeAsync(40 + BEAT_MS);
    expect(aliveCalls()).toEqual([
      { instance: 3, gen: 1 },
      { instance: 3, gen: 2 },
      { instance: 3, gen: 2 },
    ]);
  });

  it("no beat when the stage is gone", async () => {
    const { unmount } = mount();
    unmount();
    await vi.advanceTimersByTimeAsync(2000);
    expect(aliveCalls()).toEqual([]);
  });
});

describe("Stage: dragging", () => {
  it("keeps the dragged node, and sends one edit, on release", () => {
    const { charm, hitOf, edits, setScene, scene } = mount();
    const node = charm("pic");
    down(hitOf("pic"), 300, 270);
    move(310, 270);
    move(320, 275);
    move(330, 280);
    expect(edits).toEqual([]);
    expect(charm("pic")).toBe(node);
    expect(node.isConnected).toBe(true);
    expectNear(rectOf(node), { x: 270, y: 250, w: 120, h: 60 });
    up(330, 280);
    expect(edits).toEqual([{ kind: "place", id: "pic", rect: { x: 270, y: 250, w: 120, h: 60 }, viewport: V480 }]);
    // Where the reducer stored it (6 decimals of the design size).
    expectNear(rectOf(node), { x: 270, y: 250, w: 120, h: 60 }, 1e-3);
    // The ack: main's copy of the same scene.
    setScene(JSON.parse(JSON.stringify(scene())) as OverlayScene);
    expect(charm("pic")).toBe(node);
    expect(node.isConnected).toBe(true);
    expectNear(rectOf(node), { x: 270, y: 250, w: 120, h: 60 }, 1e-3);
  });

  it("the circle's subtree survives a push and a Settings add", () => {
    const { charm, setScene, scene } = mount();
    const root = charm("circle").querySelector("[data-circle] > div")!;
    expect(root).not.toBeNull();
    setScene(JSON.parse(JSON.stringify(scene())) as OverlayScene);
    const added = applyOp(scene()!, {
      kind: "add",
      element: { id: "c-1", type: "image", x: 0.1, y: 0.1, w: 0.1, h: 0.1, z: 0, src: PIC, nw: 1, nh: 1 },
    });
    expect(added).not.toBe(scene());
    setScene(added);
    expect(charm("c-1")).not.toBeNull();
    expect(charm("circle").querySelector("[data-circle] > div")).toBe(root);
    expect(root.isConnected).toBe(true);
  });

  it("a release the window didn't see (no button on a move) ends the drag; hovering after sends nothing", () => {
    const { charm, hitOf, edits } = mount();
    down(hitOf("pic"), 300, 270);
    move(320, 270);
    move(325, 270, 0);
    expect(edits).toHaveLength(1);
    expect(edits[0]).toMatchObject({ kind: "place", rect: { x: 260, y: 240 } });
    move(400, 400, 0);
    move(410, 410, 0);
    up(410, 410);
    expect(edits).toHaveLength(1);
    expectNear(rectOf(charm("pic")), { x: 260, y: 240, w: 120, h: 60 }, 1e-3);
  });

  it("a press that doesn't move sends nothing", () => {
    const { hitOf, edits } = mount();
    down(hitOf("pic"), 300, 270);
    move(300.2, 270.1);
    up(300.2, 270.1);
    expect(edits).toEqual([]);
  });

  it("presses and moves from another pointer are ignored", () => {
    const { hitOf, edits } = mount();
    down(hitOf("pic"), 300, 270);
    const other = new MouseEvent("pointermove", { clientX: 400, clientY: 400, buttons: 1 });
    Object.defineProperty(other, "pointerId", { value: 7 });
    window.dispatchEvent(other);
    up(300, 270);
    expect(edits).toEqual([]);
  });

  it("a cancelled pointer drops the drag and snaps back", () => {
    const { charm, hitOf, edits } = mount();
    down(hitOf("pic"), 300, 270);
    move(340, 270);
    window.dispatchEvent(new MouseEvent("pointercancel", { bubbles: true }));
    expectNear(rectOf(charm("pic")), { x: 240, y: 240, w: 120, h: 60 });
    up(340, 270);
    expect(edits).toEqual([]);
  });

  it("a charm removed from Settings mid-drag drops the drag", () => {
    const { hitOf, edits, setScene, scene } = mount();
    down(hitOf("pic"), 300, 270);
    move(320, 270);
    setScene(applyOp(scene()!, { kind: "remove", id: "pic" }));
    move(330, 270);
    up(330, 270);
    expect(edits).toEqual([]);
  });

  it("a corner handle resizes, keeping the shape", () => {
    const { charm, hitOf, edits } = mount();
    down(hitOf("pic"), 300, 270);
    up(300, 270);
    const se = charm("pic").querySelector<HTMLElement>('[data-handle][data-corner="se"]')!;
    down(se, 360, 300);
    move(384, 312);
    up(384, 312);
    expect(edits).toHaveLength(1);
    const op = edits[0] as Extract<SceneOp, { kind: "place" }>;
    expect(op.kind).toBe("place");
    expect(op.rect.x).toBeCloseTo(240, 9);
    expect(op.rect.y).toBeCloseTo(240, 9);
    expect(op.rect.w).toBeCloseTo(144, 9);
    expect(op.rect.w / op.rect.h).toBeCloseTo(2, 9);
  });

  it("a press in the middle of a selected 24×24 charm moves it", () => {
    const s = baseScene();
    s.elements.push({ id: "dot", type: "image", x: 0.1, y: 0.1, w: 0.05, h: 0.05, z: 2, src: PIC, nw: 1, nh: 1 });
    const { charm, hitOf, edits } = mount({ scene: s });
    down(hitOf("dot"), 60, 60);
    up(60, 60);
    const row = charm("dot");
    const hit = rectOf(hitOf("dot"));
    expect(hit).toEqual({ x: 0, y: 0, w: 24, h: 24 });
    // No handle covers the centre, so a press there lands on the hit layer.
    const handles = [...row.querySelectorAll<HTMLElement>("[data-handle]")].map(rectOf);
    expect(handles).toHaveLength(4);
    for (const h of handles) {
      expect(12 >= h.x && 12 < h.x + h.w && 12 >= h.y && 12 < h.y + h.h).toBe(false);
    }
    down(hitOf("dot"), 60, 60);
    move(70, 65);
    up(70, 65);
    expect(edits).toEqual([{ kind: "place", id: "dot", rect: { x: 58, y: 53, w: 24, h: 24 }, viewport: V480 }]);
  });

  it("the circle takes presses on a round hit layer; its handles take them too", () => {
    const { charm, hitOf } = mount();
    const row = charm("circle");
    expect(row.style.pointerEvents).toBe("none");
    expect(hitOf("circle").style.borderRadius).toBe("50%");
    expect(hitOf("circle").style.pointerEvents).toBe("auto");
    expect(hitOf("pic").style.borderRadius).toBe("");
    down(hitOf("circle"), 240, 100);
    up(240, 100);
    const handles = row.querySelectorAll<HTMLElement>("[data-handle]");
    expect(handles).toHaveLength(4);
    for (const h of handles) expect(h.style.pointerEvents).toBe("auto");
    expect(row.querySelector<HTMLElement>("[data-circle]")!.style.pointerEvents).toBe("none");
  });

  it("only the selected charm has handles; a press on empty stage clears the selection", () => {
    const { container, q, hitOf } = mount();
    down(hitOf("pic"), 300, 270);
    up(300, 270);
    expect(container.querySelectorAll('[data-id="pic"] [data-handle]')).toHaveLength(4);
    expect(container.querySelectorAll('[data-id="circle"] [data-handle]')).toHaveLength(0);
    down(q("[data-stage]")!, 5, 5);
    expect(container.querySelector("[data-handle]")).toBeNull();
  });

  it("nothing is pressed while Locked", () => {
    const { hitOf, edits, container } = mount({ status: locked() });
    const e = down(hitOf("pic"), 300, 270);
    move(340, 270);
    up(340, 270);
    expect(e.defaultPrevented).toBe(false);
    expect(edits).toEqual([]);
    expect(container.querySelector("[data-handle]")).toBeNull();
  });

  it("a charm press is no window drag: it stops the mousedown that would start one", () => {
    const { hitOf } = mount();
    const e = down(hitOf("pic"), 300, 270);
    expect(e.defaultPrevented).toBe(true);
    up(300, 270);
  });
});

describe("Stage: moving the window", () => {
  function mousedown(el: Element, detail = 1): MouseEvent {
    const e = new MouseEvent("mousedown", { button: 0, detail, bubbles: true, cancelable: true });
    el.dispatchEvent(e);
    return e;
  }

  it("the grip drags the window, even with the circle covering it; a double press doesn't", async () => {
    const s = baseScene();
    s.elements = [{ id: "circle", type: "spellcircle", x: 0, y: 0, w: 1, h: 1, z: 0 }];
    const { q } = mount({ scene: s });
    const e = mousedown(q("[data-grip]")!);
    await settle();
    expect(startDragging).toHaveBeenCalledTimes(1);
    expect(e.defaultPrevented).toBe(true);
    mousedown(q("[data-grip]")!, 2);
    await settle();
    expect(startDragging).toHaveBeenCalledTimes(1);
  });

  it("empty stage drags the window only arranging and unfitted", async () => {
    const { q, setStatus } = mount({ status: locked() });
    mousedown(q("[data-stage]")!);
    await settle();
    expect(startDragging).not.toHaveBeenCalled();
    setStatus(arranging({ fit: true }));
    mousedown(q("[data-stage]")!);
    await settle();
    expect(startDragging).not.toHaveBeenCalled();
    setStatus(arranging());
    mousedown(q("[data-stage]")!);
    await settle();
    expect(startDragging).toHaveBeenCalledTimes(1);
  });

  it("the chrome and the charms never drag the window from the stage's handler", async () => {
    const { q, hitOf } = mount();
    mousedown(q("[data-lock]")!);
    mousedown(hitOf("pic"));
    await settle();
    expect(startDragging).not.toHaveBeenCalled();
  });
});

describe("Stage: keys", () => {
  const blocked: Array<[string, KeyboardEventInit]> = [
    ["F5", { key: "F5" }],
    ["Ctrl+P", { key: "p", ctrlKey: true }],
    ["Ctrl+F", { key: "f", ctrlKey: true }],
    ["Alt+Space", { key: " ", altKey: true }],
  ];
  const free: Array<[string, KeyboardEventInit]> = [
    ["Tab", { key: "Tab" }],
    ["Shift+Tab", { key: "Tab", shiftKey: true }],
    ["Alt+F4", { key: "F4", altKey: true }],
  ];

  for (const status of [locked(), arranging()]) {
    describe(status.mode, () => {
      it.each(blocked)("%s is blocked", (_n, init) => {
        mount({ status });
        expect(key(init.key!, init).defaultPrevented).toBe(true);
      });

      it.each(free)("%s is left alone", (_n, init) => {
        mount({ status });
        expect(key(init.key!, init).defaultPrevented).toBe(false);
      });

      it("no context menu", () => {
        const { q } = mount({ status });
        const e = new MouseEvent("contextmenu", { bubbles: true, cancelable: true });
        q("[data-stage]")!.dispatchEvent(e);
        expect(e.defaultPrevented).toBe(true);
      });
    });
  }

  it("Enter and Space press the Lock button; elsewhere they're blocked", () => {
    const { q } = mount();
    const lock = q("[data-lock]")!;
    expect(key("Enter", {}, lock).defaultPrevented).toBe(false);
    expect(key(" ", {}, lock).defaultPrevented).toBe(false);
    expect(key("Enter", { ctrlKey: true }, lock).defaultPrevented).toBe(true);
    expect(key("Enter").defaultPrevented).toBe(true);
  });

  it("Esc locks; Locked, it does nothing", async () => {
    const { setStatus } = mount({ status: locked() });
    key("Escape");
    await settle();
    expect(invokeMock).not.toHaveBeenCalled();
    setStatus(arranging());
    key("Escape");
    await settle();
    expect(invokeMock).toHaveBeenCalledWith("set_overlay_mode", { mode: "locked" });
  });

  it("Esc during a drag cancels it instead", async () => {
    const { charm, hitOf, edits } = mount();
    down(hitOf("pic"), 300, 270);
    move(340, 270);
    key("Escape");
    await settle();
    expect(invokeMock).not.toHaveBeenCalled();
    expectNear(rectOf(charm("pic")), { x: 240, y: 240, w: 120, h: 60 });
    move(350, 270);
    up(350, 270);
    expect(edits).toEqual([]);
  });

  it("other keys do nothing during a drag", () => {
    const { hitOf, edits } = mount();
    down(hitOf("pic"), 300, 270);
    key("Delete");
    key("ArrowLeft");
    key("]");
    up(300, 270);
    expect(edits).toEqual([]);
  });

  describe("with a charm selected", () => {
    beforeEach(() => {
      vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
    });

    function selectPic(m: ReturnType<typeof mount>): HTMLElement {
      const node = m.charm("pic");
      node.focus();
      return node;
    }

    it("Tab focus selects; arrows nudge as one edit, and focus stays", () => {
      const m = mount();
      const node = selectPic(m);
      expect(m.container.querySelectorAll('[data-id="pic"] [data-handle]')).toHaveLength(4);
      key("ArrowRight");
      key("ArrowRight");
      key("ArrowRight");
      key("ArrowDown", { shiftKey: true });
      expect(document.activeElement).toBe(node);
      expect(m.charm("pic")).toBe(node);
      expectNear(rectOf(node), { x: 243, y: 250, w: 120, h: 60 });
      vi.advanceTimersByTime(NUDGE_FLUSH_MS - 1);
      expect(m.edits).toEqual([]);
      vi.advanceTimersByTime(1);
      expect(m.edits).toEqual([{ kind: "place", id: "pic", rect: { x: 243, y: 250, w: 120, h: 60 }, viewport: V480 }]);
      vi.advanceTimersByTime(NUDGE_FLUSH_MS * 3);
      expect(m.edits).toHaveLength(1);
      expect(document.activeElement).toBe(node);
    });

    it("a nudge run keeps the viewport it began in", () => {
      const m = mount();
      selectPic(m);
      key("ArrowLeft");
      m.setViewport({ w: 600, h: 480 });
      key("ArrowLeft");
      vi.advanceTimersByTime(NUDGE_FLUSH_MS);
      expect(m.edits).toEqual([{ kind: "place", id: "pic", rect: { x: 238, y: 240, w: 120, h: 60 }, viewport: V480 }]);
    });

    it("+ and − resize about the centre, as one edit", () => {
      const m = mount();
      selectPic(m);
      key("+");
      vi.advanceTimersByTime(NUDGE_FLUSH_MS);
      const op = m.edits[0] as Extract<SceneOp, { kind: "place" }>;
      expect(op.kind).toBe("place");
      expect(op.rect.w).toBeCloseTo(132, 9);
      expect(op.rect.h).toBeCloseTo(66, 9);
      expect(op.rect.x + op.rect.w / 2).toBeCloseTo(300, 9);
      key("-");
      key("-");
      vi.advanceTimersByTime(NUDGE_FLUSH_MS);
      const op2 = m.edits[1] as Extract<SceneOp, { kind: "place" }>;
      expect(op2.rect.w).toBeCloseTo(128, 9);
      expect(op2.rect.w / op2.rect.h).toBeCloseTo(2, 9);
    });

    it("an arrow, then a drag: the nudge goes first, the drag starts from it, nothing after", () => {
      const m = mount();
      selectPic(m);
      key("ArrowRight");
      down(m.hitOf("pic"), 300, 270);
      expect(m.edits).toEqual([{ kind: "place", id: "pic", rect: { x: 241, y: 240, w: 120, h: 60 }, viewport: V480 }]);
      move(310, 270);
      up(310, 270);
      expect(m.edits).toHaveLength(2);
      const drag = m.edits[1] as Extract<SceneOp, { kind: "place" }>;
      expect(drag.kind).toBe("place");
      expectNear(drag.rect, { x: 251, y: 240, w: 120, h: 60 }, 1e-3);
      vi.advanceTimersByTime(NUDGE_FLUSH_MS * 3);
      expect(m.edits).toHaveLength(2);
    });

    it("Delete removes it (Backspace doesn't); [ ] { } restack it; a pending nudge goes first", () => {
      const m = mount();
      selectPic(m);
      key("Backspace");
      expect(m.edits).toEqual([]);
      key("ArrowUp");
      key("[");
      expect(m.edits).toEqual([
        { kind: "place", id: "pic", rect: { x: 240, y: 239, w: 120, h: 60 }, viewport: V480 },
        { kind: "order", id: "pic", to: "backward" },
      ]);
      key("]");
      key("{");
      key("}");
      expect(m.edits.slice(2)).toEqual([
        { kind: "order", id: "pic", to: "forward" },
        { kind: "order", id: "pic", to: "back" },
        { kind: "order", id: "pic", to: "front" },
      ]);
      key("Delete");
      expect(m.edits.at(-1)).toEqual({ kind: "remove", id: "pic" });
    });

    it("a restack moves no node, so focus stays", () => {
      const m = mount();
      const node = selectPic(m);
      const circle = m.charm("circle");
      key("[");
      expect(m.charm("pic").style.zIndex).toBe("0");
      expect(m.charm("circle").style.zIndex).toBe("1");
      expect(m.charm("pic")).toBe(node);
      expect(m.charm("circle")).toBe(circle);
      expect(document.activeElement).toBe(node);
    });

    it("Locking sends a pending nudge, ends the drag and clears the selection", () => {
      const m = mount();
      selectPic(m);
      key("ArrowRight");
      m.setStatus(locked());
      expect(m.edits).toEqual([{ kind: "place", id: "pic", rect: { x: 241, y: 240, w: 120, h: 60 }, viewport: V480 }]);
      m.setStatus(arranging());
      expect(m.container.querySelector("[data-handle]")).toBeNull();
      key("Delete");
      expect(m.edits).toHaveLength(1);
    });

    it("Esc sends a pending nudge before it locks", async () => {
      const m = mount();
      selectPic(m);
      key("ArrowRight");
      key("Escape");
      expect(m.edits).toHaveLength(1);
      vi.useRealTimers();
      await settle();
      expect(invokeMock).toHaveBeenCalledWith("set_overlay_mode", { mode: "locked" });
    });

    it("losing the keyboard sends a pending nudge", () => {
      const m = mount();
      selectPic(m);
      key("ArrowRight");
      window.dispatchEvent(new Event("blur"));
      expect(m.edits).toHaveLength(1);
    });

    it("selecting another charm sends a pending nudge first", () => {
      const m = mount();
      selectPic(m);
      key("ArrowRight");
      m.charm("circle").focus();
      expect(m.edits).toEqual([{ kind: "place", id: "pic", rect: { x: 241, y: 240, w: 120, h: 60 }, viewport: V480 }]);
    });

    it("with nothing selected, the editing keys do nothing", () => {
      const m = mount();
      key("ArrowRight");
      key("Delete");
      key("+");
      key("]");
      vi.advanceTimersByTime(NUDGE_FLUSH_MS);
      expect(m.edits).toEqual([]);
    });
  });
});

describe("Stage: a timer's face (v1.59.0)", () => {
  const SAMPLES = timerFixture.samples as Record<"idle" | "running" | "done", TimerSnapshot>;
  const [BRB, UPTIME] = SAMPLES.running.timers as [TimerSnapshot["timers"][number], TimerSnapshot["timers"][number]];

  /** The base scene and a 300×100 face for `timer-1` at (90, 30), in front. */
  function timerScene(more: Record<string, unknown> = {}): OverlayScene {
    const s = baseScene();
    s.elements.push({ id: "face", type: "timer", x: 90 / 480, y: 30 / 480, w: 300 / 480, h: 100 / 480, z: 2, timer: "timer-1", ...more });
    return s;
  }

  /** BRB showing `text`, and Uptime showing `other`. */
  function snap(seq: number, text: string, other = UPTIME.text): TimerSnapshot {
    return { seq, timers: [{ ...BRB, text }, { ...UPTIME, text: other }] };
  }

  const faceOf = (m: ReturnType<typeof mount>): HTMLElement | null =>
    m.charm("face").querySelector<HTMLElement>("[data-timer-face]");

  it("draws nothing before the backend's first snapshot, then the timer's text", () => {
    const m = mount({ scene: timerScene(), status: locked() });
    // Live, and placed, before any snapshot.
    expectNear(rectOf(m.charm("face")), { x: 90, y: 30, w: 300, h: 100 }, 1e-9);
    expect(faceOf(m)).toBeNull();
    expect(m.charm("face").textContent).toBe("");
    m.setTimers(SAMPLES.running);
    expect(faceOf(m)!.textContent).toBe("04:59");
    expect(faceOf(m)!.getAttribute("aria-hidden")).toBe("true");
  });

  it("waits for its font: no face until fontReady, and no badge meanwhile", () => {
    const m = mount({ scene: timerScene(), timers: SAMPLES.running, fontReady: false });
    expect(faceOf(m)).toBeNull();
    expect(m.charm("face").textContent).toBe("");
    m.setFontReady(true);
    expect(faceOf(m)!.textContent).toBe("04:59");
  });

  it("Locked, a timer the snapshot lacks draws nothing; Arrange says it's missing", () => {
    const m = mount({ scene: timerScene({ timer: "timer-9" }), status: locked(), timers: SAMPLES.running });
    expect(m.charm("face").textContent).toBe("");
    m.setStatus(arranging());
    expect(m.charm("face").querySelector("[data-missing]")!.textContent).toBe("Timer missing");
    // With its sigil, as Picture missing has its own.
    expect(m.charm("face").querySelector("[data-missing] svg.sigil")).not.toBeNull();
    // The timer arrives: the badge goes and the face draws.
    m.setTimers({ seq: 9, timers: [{ ...BRB, id: "timer-9" }] });
    expect(m.charm("face").querySelector("[data-missing]")).toBeNull();
    expect(faceOf(m)!.textContent).toBe("04:59");
    // And goes (the timer deleted): Locked, nothing at all.
    m.setStatus(locked());
    m.setTimers({ seq: 10, timers: [] });
    expect(faceOf(m)).toBeNull();
    expect(m.charm("face").textContent).toBe("");
  });

  it("Arrange before any snapshot shows no badge, and still arranges the face", () => {
    const m = mount({ scene: timerScene() });
    expect(m.container.textContent).not.toContain("Timer missing");
    expect(m.charm("face").querySelector("[data-outline]")).not.toBeNull();
    expect(m.charm("face").tabIndex).toBe(0);
    // The backend's first answer, without this timer.
    m.setTimers({ seq: 1, timers: [] });
    expect(m.container.textContent).toContain("Timer missing");
  });

  it("the text node is the same node across updates, and is written only when the text changes", () => {
    const m = mount({ scene: timerScene(), status: locked(), timers: snap(1, "05:00") });
    const face = faceOf(m)!;
    const node = face.firstChild!;
    expect(node.nodeType).toBe(Node.TEXT_NODE);
    const seen = new MutationObserver(() => undefined);
    seen.observe(m.charm("face"), { subtree: true, childList: true, characterData: true, attributes: true });
    for (const [seq, text] of [
      [2, "04:59"],
      [3, "04:58"],
      [4, "04:57"],
    ] as const) {
      m.setTimers(snap(seq, text));
      expect(faceOf(m)).toBe(face);
      expect(face.firstChild).toBe(node);
      expect(node.textContent).toBe(text);
    }
    expect(seen.takeRecords().map((r) => [r.type, r.target])).toEqual([
      ["characterData", node],
      ["characterData", node],
      ["characterData", node],
    ]);
    // Another timer's second: nothing here is written.
    m.setTimers(snap(5, "04:57", "1:02:04"));
    expect(seen.takeRecords()).toEqual([]);
    seen.disconnect();
  });

  it("sizes its text to the box: 59:59 turning into 1:00:00 shrinks the font, not the box", () => {
    const m = mount({ scene: timerScene(), status: locked(), timers: snap(1, "59:59") });
    const r = rectOf(m.charm("face"));
    expect(parseFloat(faceOf(m)!.style.fontSize)).toBeCloseTo(fitTimerFont(r.w, r.h, 5), 9);
    m.setTimers(snap(2, "1:00:00"));
    expect(parseFloat(faceOf(m)!.style.fontSize)).toBeCloseTo(fitTimerFont(r.w, r.h, 7), 9);
    expect(fitTimerFont(r.w, r.h, 7)).toBeLessThan(fitTimerFont(r.w, r.h, 5));
    expectNear(rectOf(m.charm("face")), r, 1e-9);
  });

  it("is white Space Mono Bold with an outline, centred, with no animation, transition or title", () => {
    const m = mount({ scene: timerScene(), timers: SAMPLES.running });
    const face = faceOf(m)!;
    expect(face.style.fontFamily).toBe("var(--font-mono)");
    expect(face.style.fontWeight).toBe("700");
    expect(face.style.color).toBe("rgb(255, 255, 255)");
    expect(face.style.textShadow).not.toBe("");
    expect(face.style.whiteSpace).toBe("nowrap");
    expect(face.style.justifyContent).toBe("center");
    expect(face.style.alignItems).toBe("center");
    expect(face.getAttribute("style")).not.toMatch(/animation|transition/);
    expect(face.hasAttribute("class")).toBe(false);
    expect(m.container.querySelectorAll("[title]")).toHaveLength(0);
  });

  it("its label names the timer and follows a rename", () => {
    const m = mount({ scene: timerScene(), timers: SAMPLES.running });
    expect(m.charm("face").getAttribute("role")).toBe("img");
    expect(m.charm("face").getAttribute("aria-label")).toBe("Timer: BRB");
    m.setTimers({ seq: 7, timers: [{ ...BRB, name: "Be right back" }] });
    expect(m.charm("face").getAttribute("aria-label")).toBe("Timer: Be right back");
    // The other charms keep theirs.
    expect(m.charm("pic").getAttribute("aria-label")).toBe("Picture: wave.gif");
  });

  it("arranges like a picture: a drag is one place, a resize keeps its 3:1 shape, Delete removes it", () => {
    const m = mount({ scene: timerScene(), timers: SAMPLES.running });
    down(m.hitOf("face"), 240, 80);
    move(250, 90);
    up(250, 90);
    expect(m.edits).toHaveLength(1);
    const drag = m.edits[0] as Extract<SceneOp, { kind: "place" }>;
    expect(drag).toMatchObject({ kind: "place", id: "face", viewport: V480 });
    expectNear(drag.rect, { x: 100, y: 40, w: 300, h: 100 }, 1e-9);
    const se =m.charm("face").querySelector<HTMLElement>('[data-handle][data-corner="se"]')!;
    down(se, 400, 140);
    move(430, 150);
    up(430, 150);
    const op = m.edits[1] as Extract<SceneOp, { kind: "place" }>;
    expect(op.kind).toBe("place");
    expect(op.rect.w).toBeGreaterThan(300);
    // Its stored shape, to the 6 decimals the scene keeps.
    expect(op.rect.w / op.rect.h).toBeCloseTo(3, 4);
    key("Delete");
    expect(m.edits.at(-1)).toEqual({ kind: "remove", id: "face" });
  });
});

// ---- v1.62.0 (Text charms) ----

/** `#rrggbb` as the DOM reads a colour back. */
function rgb(hex: string): string {
  const [r, g, b] = [1, 3, 5].map((i) => parseInt(hex.slice(i, i + 2), 16));
  return `rgb(${r}, ${g}, ${b})`;
}

/** A measure the test controls: `per.px` px a code unit at 100 px, in any
 *  font; `load` answers when `release` is called (held), or at once. */
function fakeMetrics(perChar = 50, held = false) {
  let release: () => void = () => undefined;
  const gate = held ? new Promise<void>((r) => (release = r)) : Promise.resolve();
  const per = { px: perChar };
  const width = vi.fn((_spec: string, line: string) => per.px * line.length);
  const load = vi.fn((_spec: string, _sample: string) => gate);
  return { metrics: { load, width } as TextMetrics, width, load, release: () => release(), per };
}

/** The base scene and a 337×139 text at (40, 300), in front: "BRB" in
 *  Sans Bold Yellow, outlined, centred, stored as measured at 337×139. */
function textScene(more: Record<string, unknown> = {}): OverlayScene {
  const s = baseScene();
  s.elements.push({
    id: "t",
    type: "text",
    x: 40 / 480,
    y: 300 / 480,
    w: 337 / 480,
    h: 139 / 480,
    z: 2,
    text: "BRB",
    font: "sans",
    bold: true,
    color: "#ffd60a",
    outline: true,
    align: "center",
    nw: 337,
    nh: 139,
    ...more,
  });
  return s;
}

const textFace = (m: ReturnType<typeof mount>): HTMLElement | null =>
  m.charm("t").querySelector<HTMLElement>("[data-text-face]");

/** The element holding the face's words. */
const textLines = (m: ReturnType<typeof mount>): HTMLElement => textFace(m)!.firstElementChild as HTMLElement;

/** Stand in for layout: the words' laid-out size (`offsetWidth` and
 *  `offsetHeight`), by their element; or, with `scroll`, their scroll size,
 *  which the backstop doesn't read. Returns a restore. */
function stubScroll(size: (el: HTMLElement) => { w: number; h: number }, scroll = false): () => void {
  const [wide, tall] = scroll ? ["scrollWidth", "scrollHeight"] : ["offsetWidth", "offsetHeight"];
  const words = (el: HTMLElement) => (el.parentElement?.hasAttribute("data-text-face") ? size(el) : null);
  // jsdom has its own `offsetWidth` here (always 0); put it back after.
  const own = [wide, tall].map((k) => [k, Object.getOwnPropertyDescriptor(HTMLElement.prototype, k)] as const);
  Object.defineProperty(HTMLElement.prototype, wide, {
    configurable: true,
    get(this: HTMLElement) {
      return words(this)?.w ?? 0;
    },
  });
  Object.defineProperty(HTMLElement.prototype, tall, {
    configurable: true,
    get(this: HTMLElement) {
      return words(this)?.h ?? 0;
    },
  });
  return () => {
    for (const [k, d] of own) {
      if (d) Object.defineProperty(HTMLElement.prototype, k, d);
      else delete (HTMLElement.prototype as unknown as Record<string, unknown>)[k];
    }
  };
}

describe("Stage: a text charm (v1.62.0)", () => {
  it("draws its words as one text node, never markup", async () => {
    const { metrics } = fakeMetrics();
    const words = "<b>hi</b> & <img src=x onerror=alert(1)>";
    const m = mount({ scene: textScene({ text: words }), status: locked(), textMetrics: metrics });
    await settle();
    const face = textFace(m)!;
    expect(face.children).toHaveLength(1);
    const lines = textLines(m);
    expect(lines.children).toHaveLength(0);
    expect(lines.childNodes).toHaveLength(1);
    expect(lines.firstChild!.nodeType).toBe(Node.TEXT_NODE);
    expect(lines.textContent).toBe(words);
    expect(m.charm("t").querySelector("b, img")).toBeNull();
    expect(face.getAttribute("aria-hidden")).toBe("true");
  });

  it("names the row by its words on one line, and draws them normalized", async () => {
    const { metrics } = fakeMetrics();
    const m = mount({ scene: textScene({ text: "  Be right\r\nback soon  " }), status: locked(), textMetrics: metrics });
    await settle();
    expect(m.charm("t").getAttribute("role")).toBe("img");
    expect(m.charm("t").getAttribute("aria-label")).toBe("Text: Be right back soon");
    expect(textLines(m).textContent).toBe("Be right\nback\nsoon");
  });

  it.each([
    ["display", true, "left", "#ffffff", true],
    ["sans", false, "center", "#ff4500", false],
    ["mono", true, "right", "#111111", true],
  ] as const)("its look comes from text.ts's tables: %s, bold %s, %s, %s, outline %s", async (font, bold, align, color, outline) => {
    const { metrics } = fakeMetrics();
    const m = mount({ scene: textScene({ font, bold, align, color, outline }), status: locked(), textMetrics: metrics });
    await settle();
    const face = textFace(m)!;
    expect(face.style.fontFamily).toBe(FONTS[font].stack);
    expect(face.style.fontWeight).toBe(bold ? "700" : "400");
    expect(face.style.getPropertyValue("font-kerning")).toBe("normal");
    expect(face.style.getPropertyValue("letter-spacing")).toMatch(/^0(px)?$/);
    expect(face.style.getPropertyValue("text-rendering")).toBe("optimizeLegibility");
    expect(face.style.color).toBe(rgb(color));
    expect(face.style.textShadow).toBe(outline ? textShadow(color) : "none");
    expect(face.style.textAlign).toBe(align);
    expect(face.style.justifyContent).toBe({ left: "flex-start", center: "center", right: "flex-end" }[align]);
    expect(face.style.alignItems).toBe("center");
    expect(face.style.whiteSpace).toBe("pre");
    expect(face.style.lineHeight).toBe("1.15");
    expect(textLines(m).style.padding).toBe("0.12em");
    expect(face.getAttribute("style")).not.toMatch(/animation|transition/);
  });

  it("pads the words for ink past the first line's box, and fits by the box that ink gives", async () => {
    // Vietnamese capitals' stacked marks reach 116 above the baseline at
    // 100 px; Display Bold's first font (93 and 27) puts the line's own box
    // 90.5 above it, so they need 0.255 em more room on top.
    const viet = "Ổ Ỗ Ộ";
    const { metrics, width } = fakeMetrics();
    const ink = vi.fn((_spec: string, line: string) => ({
      ascent: line === viet ? 116 : 70,
      descent: 20,
      fontAscent: 93,
      fontDescent: 27,
    }));
    const m = mount({
      scene: textScene({ text: viet, font: "display" }),
      status: locked(),
      textMetrics: { ...metrics, ink },
    });
    await settle();
    const spec = fontSpec("display", true);
    expect(ink).toHaveBeenCalledWith(spec, viet);
    const lines = textLines(m);
    expect(parseFloat(lines.style.paddingTop)).toBeCloseTo(0.375, 6);
    expect(parseFloat(lines.style.paddingBottom)).toBeCloseTo(0.12, 6);
    expect(parseFloat(lines.style.paddingLeft)).toBeCloseTo(0.12, 6);
    expect(parseFloat(lines.style.paddingRight)).toBeCloseTo(0.12, 6);
    // Fitted by the page's own measure with that room: 165 tall, not 139.
    const box = textBox(viet, (l) => width(spec, l), (l) => ink(spec, l));
    expect(box.nh).toBe(165);
    expect(parseFloat(textFace(m)!.style.fontSize)).toBeCloseTo(fitTextFont(337, 139, box.nw, box.nh), 9);
    // Latin ink stays inside its line: PAD_EM alone.
    const latin = mount({ scene: textScene(), status: locked(), textMetrics: { ...metrics, ink } });
    await settle();
    expect(textLines(latin).style.padding).toBe("0.12em");
  });

  it("a stored string never reaches the style: an unknown font, colour or alignment reads its default", async () => {
    const { metrics } = fakeMetrics();
    const m = mount({
      scene: textScene({ font: "x; color: red", color: "#ffd60a; background: red", align: "justify", bold: "yes", outline: 1 }),
      status: locked(),
      textMetrics: metrics,
    });
    await settle();
    const face = textFace(m)!;
    expect(face.style.fontFamily).toBe(FONTS.sans.stack);
    expect(face.style.color).toBe("rgb(255, 255, 255)");
    expect(face.style.background).toBe("");
    expect(face.style.textAlign).toBe("center");
    expect(face.style.fontWeight).toBe("700");
    expect(face.style.textShadow).toBe(textShadow("#ffffff"));
  });

  it("is fitted by the page's own measure, never the stored nw/nh, with the face's whole font stack", async () => {
    // 200 px a letter: "BRB" measures 624 × 139 here, not the stored 337 × 139.
    const { metrics, width, load } = fakeMetrics(200);
    const m = mount({ scene: textScene(), status: locked(), textMetrics: metrics });
    await settle();
    const spec = fontSpec("sans", true);
    expect(spec).toBe('700 100px "Space Grotesk Variable", system-ui, sans-serif');
    expect(load).toHaveBeenCalledWith(spec, "BRB");
    expect(width).toHaveBeenCalledWith(spec, "BRB");
    const own = textBox("BRB", (l) => 200 * l.length);
    expect(own).toEqual({ nw: 624, nh: 139 });
    const size = parseFloat(textFace(m)!.style.fontSize);
    expect(size).toBeCloseTo(fitTextFont(337, 139, 624, 139), 9);
    expect(size).not.toBeCloseTo(fitTextFont(337, 139, 337, 139), 3);
    // The words fit the box at that size.
    expect((own.nw * size) / 100).toBeLessThanOrEqual(337 + 1e-9);
    expect((own.nh * size) / 100).toBeLessThanOrEqual(139 + 1e-9);
  });

  it("measures each line, and fits the widest", async () => {
    const { metrics, width } = fakeMetrics(40);
    const m = mount({ scene: textScene({ text: "BRB\nback in five" }), status: locked(), textMetrics: metrics });
    await settle();
    expect(width.mock.calls.map((c) => c[1])).toEqual(["BRB", "back in five"]);
    const box = textBox("BRB\nback in five", (l) => 40 * l.length);
    expect(parseFloat(textFace(m)!.style.fontSize)).toBeCloseTo(fitTextFont(337, 139, box.nw, box.nh), 9);
  });

  it("a stored text over the drawing ceiling draws drawText's cut", async () => {
    const { metrics, width } = fakeMetrics(1);
    // The 4096th code unit would be the first half of a pair.
    const long = "x".repeat(4095) + "🎮" + "y".repeat(900);
    const m = mount({ scene: textScene({ text: long }), status: locked(), textMetrics: metrics });
    await settle();
    const drawn = textLines(m).textContent!;
    expect(drawn).toBe(drawText(long));
    expect(drawn).toHaveLength(4095);
    expect(width).toHaveBeenCalledWith(fontSpec("sans", true), drawn);

    const many = Array.from({ length: 60 }, (_, i) => `line ${i}`).join("\n");
    m.setScene(textScene({ text: many }));
    await settle();
    expect(textLines(m).textContent!.split("\n")).toHaveLength(50);
    expect(m.charm("t").getAttribute("aria-label")).toBe(`Text: ${drawText(many).split("\n").join(" ")}`);
  });

  describe("its font", () => {
    beforeEach(() => {
      vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
    });

    it("waits for its load, at most FONT_WAIT_MS, then draws in the fallback and measures again when the font comes", async () => {
      const fake = fakeMetrics(50, true);
      const m = mount({ scene: textScene(), status: locked(), textMetrics: fake.metrics });
      await vi.advanceTimersByTimeAsync(FONT_WAIT_MS - 1);
      expect(textFace(m)).toBeNull();
      expect(m.charm("t").textContent).toBe("");
      await vi.advanceTimersByTimeAsync(1);
      const face = textFace(m)!;
      const node = textLines(m).firstChild;
      const fallback = parseFloat(face.style.fontSize);
      expect(fallback).toBeCloseTo(fitTextFont(337, 139, textBox("BRB", (l) => 50 * l.length).nw, 139), 9);
      // The font arrives late and is wider: the same face, measured again.
      fake.per.px = 150;
      fake.release();
      await vi.advanceTimersByTimeAsync(0);
      expect(textFace(m)).toBe(face);
      expect(textLines(m).firstChild).toBe(node);
      const late = parseFloat(face.style.fontSize);
      expect(late).toBeCloseTo(fitTextFont(337, 139, textBox("BRB", (l) => 150 * l.length).nw, 139), 9);
      expect(late).toBeLessThan(fallback);
    });

    it("a load on time draws at once, measured once", async () => {
      const fake = fakeMetrics(50, true);
      const m = mount({ scene: textScene(), status: locked(), textMetrics: fake.metrics });
      await vi.advanceTimersByTimeAsync(10);
      expect(textFace(m)).toBeNull();
      fake.release();
      await vi.advanceTimersByTimeAsync(0);
      expect(textLines(m).textContent).toBe("BRB");
      await vi.advanceTimersByTimeAsync(FONT_WAIT_MS * 2);
      expect(fake.width).toHaveBeenCalledTimes(1);
    });

    it("a load that fails still draws at once", async () => {
      const fake = fakeMetrics(50);
      fake.load.mockImplementation(() => Promise.reject(new Error("no font")));
      const m = mount({ scene: textScene(), status: locked(), textMetrics: fake.metrics });
      await vi.advanceTimersByTimeAsync(0);
      expect(textLines(m).textContent).toBe("BRB");
    });

    it("an edit keeps the face before it until the new words' font is ready", async () => {
      const fake = fakeMetrics(50);
      const m = mount({ scene: textScene(), status: locked(), textMetrics: fake.metrics });
      await vi.advanceTimersByTimeAsync(0);
      expect(textLines(m).textContent).toBe("BRB");
      let release!: () => void;
      fake.load.mockImplementation(() => new Promise<void>((r) => (release = r)));
      m.setScene(textScene({ text: "Be right back", font: "mono" }));
      await vi.advanceTimersByTimeAsync(10);
      // Still the old words in the old font: never a blank charm on stream.
      expect(textLines(m).textContent).toBe("BRB");
      expect(textFace(m)!.style.fontFamily).toBe(FONTS.sans.stack);
      // But its name is the new words'.
      expect(m.charm("t").getAttribute("aria-label")).toBe("Text: Be right back");
      release();
      await vi.advanceTimersByTimeAsync(0);
      expect(textLines(m).textContent).toBe("Be right back");
      expect(textFace(m)!.style.fontFamily).toBe(FONTS.mono.stack);
      expect(fake.load).toHaveBeenLastCalledWith(fontSpec("mono", true), "Be right back");
    });

    it("a colour, outline or alignment change needs no font and shows at once", async () => {
      const fake = fakeMetrics(50);
      const m = mount({ scene: textScene(), status: locked(), textMetrics: fake.metrics });
      await vi.advanceTimersByTimeAsync(0);
      m.setScene(textScene({ color: "#0a84ff", outline: false, align: "left" }));
      expect(textFace(m)!.style.color).toBe(rgb("#0a84ff"));
      expect(textFace(m)!.style.textShadow).toBe("none");
      expect(textFace(m)!.style.justifyContent).toBe("flex-start");
      expect(fake.load).toHaveBeenCalledTimes(1);
      expect(fake.width).toHaveBeenCalledTimes(1);
    });
  });

  it("a drag and its ack measure nothing again", async () => {
    const { metrics, width, load } = fakeMetrics(50);
    const m = mount({ scene: textScene(), textMetrics: metrics });
    await settle();
    const face = textFace(m)!;
    const size = face.style.fontSize;
    down(m.hitOf("t"), 200, 370);
    move(205, 360);
    move(210, 350);
    up(210, 350);
    expect(m.edits).toHaveLength(1);
    m.setScene(JSON.parse(JSON.stringify(m.scene())) as OverlayScene);
    await settle();
    expect(load).toHaveBeenCalledTimes(1);
    expect(width).toHaveBeenCalledTimes(1);
    expect(textFace(m)).toBe(face);
    // The stored box is rounded to 6 places of the design: the same size, to
    // a ten-thousandth of a pixel.
    expect(parseFloat(face.style.fontSize)).toBeCloseTo(parseFloat(size), 3);
  });

  describe("the backstop", () => {
    let restore: (() => void) | undefined;
    afterEach(() => {
      restore?.();
      restore = undefined;
    });

    /** The words laid out `over` times the box's width at the fitted size,
     *  and in proportion to the font size, as a browser would. */
    function layoutOver(over: number, box: { w: number; h: number }, fitted: () => number): void {
      restore = stubScroll((el) => {
        const k = parseFloat(el.parentElement!.style.fontSize) / fitted();
        return { w: box.w * over * k, h: box.h * 0.5 * k };
      });
    }

    it("a face that still spills is cut once by the ratio, and counted", async () => {
      const { metrics } = fakeMetrics(50);
      const own = textBox("BRB", (l) => 50 * l.length);
      const fitted = fitTextFont(337, 139, own.nw, own.nh);
      layoutOver(1.1, { w: 337, h: 139 }, () => fitted);
      const m = mount({ scene: textScene(), status: locked(), textMetrics: metrics });
      await settle();
      const face = textFace(m)!;
      expect(face.getAttribute("data-fit-backstop")).toBe("1");
      expect(parseFloat(face.style.fontSize)).toBeCloseTo(fitted / 1.1, 9);
      // Once: nothing more happens at the cut size.
      await settle();
      expect(face.getAttribute("data-fit-backstop")).toBe("1");
      expect(parseFloat(face.style.fontSize)).toBeCloseTo(fitted / 1.1, 9);
    });

    it("cuts by the larger ratio when both sides spill", async () => {
      const { metrics } = fakeMetrics(50);
      const own = textBox("BRB", (l) => 50 * l.length);
      const fitted = fitTextFont(337, 139, own.nw, own.nh);
      restore = stubScroll(() => ({ w: 337 * 1.05, h: 139 * 1.25 }));
      const m = mount({ scene: textScene(), status: locked(), textMetrics: metrics });
      await settle();
      expect(textFace(m)!.getAttribute("data-fit-backstop")).toBe("1");
      expect(parseFloat(textFace(m)!.style.fontSize)).toBeCloseTo(fitted / 1.25, 9);
    });

    it("a face that fits is left alone", async () => {
      const { metrics } = fakeMetrics(50);
      const own = textBox("BRB", (l) => 50 * l.length);
      const fitted = fitTextFont(337, 139, own.nw, own.nh);
      // Under a pixel over: whole-pixel rounding, not a spill.
      restore = stubScroll(() => ({ w: 337.9, h: 139.9 }));
      const m = mount({ scene: textScene(), status: locked(), textMetrics: metrics });
      await settle();
      expect(textFace(m)!.getAttribute("data-fit-backstop")).toBe("0");
      expect(parseFloat(textFace(m)!.style.fontSize)).toBeCloseTo(fitted, 9);
    });

    it("reads the words' laid-out box, not their scroll size", async () => {
      // Chromium counts each font's ascent and descent in the scroll size:
      // Space Mono Bold, height-bound at 51.8 px in a 72 px box, lays out
      // exactly 72 px tall and scrolls 74 (E1).
      const { metrics } = fakeMetrics(50);
      const own = textBox("BRB", (l) => 50 * l.length);
      const fitted = fitTextFont(337, 139, own.nw, own.nh);
      restore = stubScroll(() => ({ w: 337, h: 139 }));
      const scrolled = stubScroll(() => ({ w: 337, h: 139 * 1.03 }), true);
      try {
        const m = mount({ scene: textScene(), status: locked(), textMetrics: metrics });
        await settle();
        expect(textFace(m)!.getAttribute("data-fit-backstop")).toBe("0");
        expect(parseFloat(textFace(m)!.style.fontSize)).toBeCloseTo(fitted, 9);
      } finally {
        scrolled();
      }
    });

    it("checks again at each new size", async () => {
      const { metrics } = fakeMetrics(50);
      const own = textBox("BRB", (l) => 50 * l.length);
      const fitted = fitTextFont(337, 139, own.nw, own.nh);
      layoutOver(1.2, { w: 337, h: 139 }, () => fitted);
      const m = mount({ scene: textScene(), textMetrics: metrics });
      await settle();
      expect(textFace(m)!.getAttribute("data-fit-backstop")).toBe("1");
      // Select it, then shrink it by a corner: at the new size the words
      // spill again by the same share, and are cut on their own.
      down(m.hitOf("t"), 200, 370);
      up(200, 370);
      const se = m.charm("t").querySelector<HTMLElement>('[data-handle][data-corner="se"]')!;
      down(se, 370, 430);
      move(370 - 33.7, 430 - 13.9);
      const r = rectOf(m.charm("t"));
      expect(r.w).toBeLessThan(337);
      expect(textFace(m)!.getAttribute("data-fit-backstop")).toBe("2");
      expect(parseFloat(textFace(m)!.style.fontSize)).toBeCloseTo(fitTextFont(r.w, r.h, own.nw, own.nh) / 1.2, 6);
      up(370 - 33.7, 430 - 13.9);
    });
  });
});

describe("Stage: the recording light (v1.62.0)", () => {
  /** The base scene and the default light, 168 × 33.6 at (19.2, 427.2). */
  function lightScene(): OverlayScene {
    const s = baseScene();
    s.elements.push({ id: "light", type: "rec", x: 19.2 / 480, y: 427.2 / 480, w: 168 / 480, h: 33.6 / 480, z: 3 });
    return s;
  }
  const faceOf = (m: ReturnType<typeof mount>): HTMLElement | null =>
    m.charm("light").querySelector<HTMLElement>("[data-rec-face]");
  const idleOf = (m: ReturnType<typeof mount>): HTMLElement | null =>
    m.charm("light").querySelector<HTMLElement>("[data-rec-idle]");

  const take = (state: TakeView["state"], elapsedMs = 0): TakeView => ({ state, elapsedMs });
  const replay = (state: ReplayView["state"]): ReplayView => ({ state });
  const HIDE = { hideWanted: true, hideAvailable: true };

  // §7.6's worked rows and the plain face's, Locked: [name, take, replay,
  // status, the page's own status seen, drawn].
  const LOCKED: Array<[string, TakeView | null, ReplayView | null, Partial<OverlayStatus>, boolean, string | null]> = [
    ["the page opened mid-start: no status yet", take("starting"), replay("off"), {}, false, null],
    ["no status yet, even with Hide off", take("recording", 65_000), null, {}, false, null],
    ["starting, before Windows is asked", take("starting"), replay("off"), { ...HIDE, capturing: false }, true, null],
    ["starting, asked now", take("starting"), replay("off"), { ...HIDE, capturing: true }, true, "REC"],
    ["recording, asked", take("recording", 65_000), replay("off"), { ...HIDE, capturing: true }, true, "REC 1:05"],
    ["Stop: erased while still asked", take("stopping", 65_000), replay("off"), { ...HIDE, capturing: true }, true, null],
    ["Stop inside Replay", take("stopping", 65_000), replay("armed"), { ...HIDE, capturing: true }, true, "REPLAY"],
    ["Replay armed", take("idle"), replay("armed"), { ...HIDE, capturing: true }, true, "REPLAY"],
    ["Replay held", take("idle"), replay("held"), { ...HIDE, capturing: true }, true, null],
    ["a status that says not capturing wins", take("recording", 65_000), replay("off"), { ...HIDE, capturing: false }, true, null],
    ["Hide off: meant to show", take("recording", 65_000), replay("off"), { capturing: true }, true, "REC 1:05"],
    ["Hide off: stopping keeps the last time", take("stopping", 65_000), replay("off"), { capturing: true }, true, "REC 1:05"],
    ["Hide wanted where Windows can't: no gate", take("recording", 65_000), null, { hideWanted: true, hideAvailable: false }, true, "REC 1:05"],
    ["a take outranks Replay", take("recording", 3_600_000), replay("armed"), { capturing: true }, true, "REC 1:00:00"],
    ["Replay arming", null, replay("arming"), {}, true, "REPLAY"],
    ["nothing records", take("idle"), replay("off"), {}, true, null],
    ["nothing heard yet", null, null, {}, true, null],
  ];

  it.each(LOCKED)("Locked: %s", (_name, t, r, st, seen, drawn) => {
    const m = mount({ scene: lightScene(), status: locked(st), take: t, replay: r, statusSeen: seen });
    expect(idleOf(m)).toBeNull();
    if (drawn === null) {
      expect(faceOf(m)).toBeNull();
      expect(m.charm("light").textContent).toBe("");
      expect(m.charm("light").querySelector("[data-rec-dot]")).toBeNull();
      return;
    }
    const face = faceOf(m)!;
    expect(face.textContent).toBe(drawn);
    expect(face.querySelector("[data-rec-dot]")).not.toBeNull();
    expect(face.getAttribute("aria-hidden")).toBe("true");
  });

  it("Arrange draws the face whatever the gate says, and a dimmed REC when nothing records", () => {
    const m = mount({ scene: lightScene(), status: arranging({ ...HIDE, capturing: false }), statusSeen: false });
    expect(faceOf(m)).toBeNull();
    const idle = idleOf(m)!;
    expect(idle.textContent).toBe("REC");
    expect(idle.style.opacity).toBe("0.45");
    expect(idle.querySelector("[data-rec-dot]")).not.toBeNull();
    m.setTake(take("starting"));
    expect(idleOf(m)).toBeNull();
    expect(faceOf(m)!.textContent).toBe("REC");
    expect(faceOf(m)!.style.opacity).toBe("");
    m.setTake(take("stopping", 5_000));
    expect(faceOf(m)!.textContent).toBe("REC 0:05");
    m.setTake(take("idle"));
    m.setReplay(replay("held"));
    expect(idleOf(m)!.textContent).toBe("REC");
    // Locked again: the gate applies, and nothing is drawn.
    m.setStatusSeen(true);
    m.setStatus(locked({ ...HIDE, capturing: false }));
    m.setTake(take("recording", 1_000));
    expect(faceOf(m)).toBeNull();
    expect(idleOf(m)).toBeNull();
  });

  it("the dot is a red circle beside the words, not a character", () => {
    const m = mount({ scene: lightScene(), status: locked(), take: take("recording", 65_000), statusSeen: true });
    const face = faceOf(m)!;
    const dot = face.querySelector<HTMLElement>("[data-rec-dot]")!;
    expect(LIGHT_RED).toBe("#ff4500");
    expect(dot.style.background).toBe(rgb(LIGHT_RED));
    expect(dot.style.borderRadius).toBe("50%");
    expect(dot.style.width).toBe("0.62em");
    expect(dot.style.height).toBe("0.62em");
    expect(dot.style.marginRight).toBe("0.3em");
    expect(dot.textContent).toBe("");
    expect(face.firstElementChild).toBe(dot);
  });

  it.each([
    ["REC", take("starting")],
    ["REC 1:05", take("recording", 65_000)],
    ["REC 1:00:00", take("recording", 3_600_000)],
  ] as const)("fits %s with two cells for the dot", (words, t) => {
    const m = mount({ scene: lightScene(), status: locked(), take: t, statusSeen: true });
    const r = rectOf(m.charm("light"));
    const size = parseFloat(faceOf(m)!.style.fontSize);
    expect(size).toBeCloseTo(fitTimerFont(r.w, r.h, words.length + 2), 9);
    // The words and the dot's two cells are never wider than the box.
    expect((words.length + 2) * 0.62 * size).toBeLessThanOrEqual(r.w + 1e-9);
  });

  it("the default box is bound by its width for REC 1:05, so the dot's room shows in the size", () => {
    // One cell short would draw it larger.
    expect(fitTimerFont(168, 33.6, 10)).toBeLessThan(fitTimerFont(168, 33.6, 9));
  });

  it("is in the timer face's style: Space Mono Bold, white, outlined, no animation", () => {
    const m = mount({ scene: lightScene(), status: locked(), replay: replay("armed"), statusSeen: true });
    const face = faceOf(m)!;
    expect(face.textContent).toBe("REPLAY");
    expect(face.style.fontFamily).toBe("var(--font-mono)");
    expect(face.style.fontWeight).toBe("700");
    expect(face.style.color).toBe("rgb(255, 255, 255)");
    expect(face.style.textShadow).not.toBe("");
    expect(face.getAttribute("style")).not.toMatch(/animation|transition/);
  });

  it("waits for its font as the timer face does: nothing until fontReady, the placeholder too", () => {
    const m = mount({
      scene: lightScene(),
      status: locked(),
      take: take("recording", 65_000),
      statusSeen: true,
      fontReady: false,
    });
    expect(faceOf(m)).toBeNull();
    expect(m.charm("light").textContent).toBe("");
    expect(m.charm("light").querySelector("[data-rec-dot]")).toBeNull();
    m.setFontReady(true);
    expect(faceOf(m)!.textContent).toBe("REC 1:05");

    // Arrange is visible to recordings: the dimmed REC waits as well.
    const a = mount({ scene: lightScene(), status: arranging(), fontReady: false });
    expect(idleOf(a)).toBeNull();
    expect(a.charm("light").textContent).toBe("");
    a.setFontReady(true);
    expect(idleOf(a)!.textContent).toBe("REC");
  });

  it("its words' text node is written only when the words change", () => {
    const m = mount({ scene: lightScene(), status: locked(), take: take("recording", 65_000), statusSeen: true });
    const face = faceOf(m)!;
    const node = [...face.childNodes].find((n) => n.nodeType === Node.TEXT_NODE)!;
    expect(node.textContent).toBe("REC 1:05");
    const seen = new MutationObserver(() => undefined);
    seen.observe(m.charm("light"), { subtree: true, childList: true, characterData: true, attributes: true });
    m.setTake(take("recording", 65_400));
    m.setTake(take("recording", 65_900));
    expect(seen.takeRecords()).toEqual([]);
    m.setTake(take("recording", 66_000));
    expect(seen.takeRecords().map((r) => [r.type, r.target])).toEqual([["characterData", node]]);
    expect(node.textContent).toBe("REC 1:06");
    seen.disconnect();
  });

  it("is named Recording light, and arranges like any charm", () => {
    const m = mount({ scene: lightScene() });
    expect(m.charm("light").getAttribute("role")).toBe("img");
    expect(m.charm("light").getAttribute("aria-label")).toBe("Recording light");
    down(m.hitOf("light"), 100, 440);
    move(130, 400);
    up(130, 400);
    expect(m.edits).toHaveLength(1);
    expect(m.edits[0]).toMatchObject({ kind: "place", id: "light", viewport: V480 });
  });
});

describe("Stage: snapping (v1.62.0)", () => {
  /** §7.5's table: a 480 × 480 viewport, another charm at (300, 100)
   *  100 × 50, and the dragged one at (180, 10) 100 × 40. */
  function snapScene(): OverlayScene {
    return {
      version: 1,
      designSize: { w: 480, h: 480 },
      frame: "window",
      elements: [
        { id: "other", type: "image", x: 300 / 480, y: 100 / 480, w: 100 / 480, h: 50 / 480, z: 0, src: PIC, nw: 2, nh: 1 },
        { id: "drag", type: "image", x: 180 / 480, y: 10 / 480, w: 100 / 480, h: 40 / 480, z: 1, src: PIC, nw: 5, nh: 2 },
      ],
    };
  }
  const guides = (m: ReturnType<typeof mount>) =>
    [...m.container.querySelectorAll<HTMLElement>("[data-guide]")].map((g) => [g.dataset.axis, Number(g.dataset.at)]);
  const placed = (m: ReturnType<typeof mount>) => (m.edits.at(-1) as Extract<SceneOp, { kind: "place" }>).rect;

  function keyup(k: string, more: KeyboardEventInit = {}): void {
    window.dispatchEvent(new KeyboardEvent("keyup", { key: k, bubbles: true, ...more }));
  }
  function moveWith(x: number, y: number, more: MouseEventInit): void {
    window.dispatchEvent(new MouseEvent("pointermove", { clientX: x, clientY: y, buttons: 1, bubbles: true, ...more }));
  }

  it("a drag snaps within 8 px, with a guide while the pointer is down", () => {
    const m = mount({ scene: snapScene() });
    down(m.hitOf("drag"), 230, 30);
    move(236, 30);
    // Its centre 236 → the viewport's centre line 240; its top 10 is 10 from 0.
    expectNear(rectOf(m.charm("drag")), { x: 190, y: 10, w: 100, h: 40 });
    expect(guides(m)).toEqual([["x", 240]]);
    const g = m.q("[data-guide]")!;
    expect(g.style.left).toBe("240px");
    expect(g.style.height).toBe("100%");
    expect(g.getAttribute("aria-hidden")).toBe("true");
    expect(g.textContent).toBe("");
    up(236, 30);
    expect(guides(m)).toEqual([]);
    expectNear(placed(m), { x: 190, y: 10, w: 100, h: 40 }, 1e-9);
  });

  it("snaps to another charm's lines: of three lines 8 px away, the lowest wins", () => {
    const m = mount({ scene: snapScene() });
    down(m.hitOf("drag"), 230, 30);
    move(342, 30);
    expectNear(rectOf(m.charm("drag")), { x: 300, y: 10, w: 100, h: 40 });
    expect(guides(m)).toEqual([["x", 300]]);
    move(341.99, 30);
    expectNear(rectOf(m.charm("drag")), { x: 291.99, y: 10, w: 100, h: 40 });
    expect(guides(m)).toEqual([]);
    up(341.99, 30);
  });

  it("with Ctrl held it follows the pointer exactly, with no guide", () => {
    const m = mount({ scene: snapScene() });
    down(m.hitOf("drag"), 230, 30);
    moveWith(236, 30, { ctrlKey: true });
    expectNear(rectOf(m.charm("drag")), { x: 186, y: 10, w: 100, h: 40 });
    expect(guides(m)).toEqual([]);
    up(236, 30);
    expectNear(placed(m), { x: 186, y: 10, w: 100, h: 40 }, 1e-9);
  });

  it("only Ctrl frees it: Alt or Shift still snap", () => {
    for (const more of [{ altKey: true }, { shiftKey: true }]) {
      const m = mount({ scene: snapScene() });
      down(m.hitOf("drag"), 230, 30);
      moveWith(236, 30, more);
      expectNear(rectOf(m.charm("drag")), { x: 190, y: 10, w: 100, h: 40 });
      expect(guides(m)).toEqual([["x", 240]]);
      up(236, 30);
      m.unmount();
    }
  });

  it("Ctrl let go or pressed mid-drag applies at once, at the last pointer point", () => {
    const m = mount({ scene: snapScene() });
    down(m.hitOf("drag"), 230, 30);
    moveWith(236, 30, { ctrlKey: true });
    expectNear(rectOf(m.charm("drag")), { x: 186, y: 10, w: 100, h: 40 });
    keyup("Control", { ctrlKey: false });
    expectNear(rectOf(m.charm("drag")), { x: 190, y: 10, w: 100, h: 40 });
    expect(guides(m)).toEqual([["x", 240]]);
    key("Control", { ctrlKey: true });
    expectNear(rectOf(m.charm("drag")), { x: 186, y: 10, w: 100, h: 40 });
    expect(guides(m)).toEqual([]);
    up(236, 30);
    expectNear(placed(m), { x: 186, y: 10, w: 100, h: 40 }, 1e-9);
  });

  it("a key before the pointer moves moves nothing", () => {
    const s = snapScene();
    // 4 px from the centre line already: a key alone mustn't jump it there.
    s.elements[1]!.x = 186 / 480;
    const m = mount({ scene: s });
    down(m.hitOf("drag"), 230, 30);
    keyup("Control");
    key("Control", { ctrlKey: true });
    keyup("Control");
    expectNear(rectOf(m.charm("drag")), { x: 186, y: 10, w: 100, h: 40 }, 1e-6);
    expect(guides(m)).toEqual([]);
    up(230, 30);
    expect(m.edits).toEqual([]);
  });

  it("a corner resize snaps its moving edge and keeps the shape", () => {
    const s = snapScene();
    s.elements = [{ id: "drag", type: "image", x: 100 / 480, y: 100 / 480, w: 100 / 480, h: 50 / 480, z: 0, src: PIC, nw: 2, nh: 1 }];
    const m = mount({ scene: s });
    down(m.hitOf("drag"), 150, 125);
    up(150, 125);
    const se = m.charm("drag").querySelector<HTMLElement>('[data-handle][data-corner="se"]')!;
    down(se, 200, 150);
    move(236, 168);
    expectNear(rectOf(m.charm("drag")), { x: 100, y: 100, w: 140, h: 70 }, 1e-9);
    expect(guides(m)).toEqual([["x", 240]]);
    moveWith(236, 168, { ctrlKey: true });
    expectNear(rectOf(m.charm("drag")), { x: 100, y: 100, w: 136, h: 68 }, 1e-9);
    expect(guides(m)).toEqual([]);
    move(236, 168);
    up(236, 168);
    expectNear(placed(m), { x: 100, y: 100, w: 140, h: 70 }, 1e-9);
  });

  it("guides are Arrange's alone: locking mid-drag drops them with the drag", () => {
    const m = mount({ scene: snapScene() });
    down(m.hitOf("drag"), 230, 30);
    move(236, 30);
    expect(guides(m)).toHaveLength(1);
    m.setStatus(locked());
    expect(guides(m)).toEqual([]);
    expect(m.edits).toEqual([]);
    // The drag itself is gone, not only hidden: the charm is drawn where it
    // is stored, and the pointer moving on and letting go places nothing.
    expectNear(rectOf(m.charm("drag")), { x: 180, y: 10, w: 100, h: 40 });
    move(250, 30);
    up(250, 30);
    expectNear(rectOf(m.charm("drag")), { x: 180, y: 10, w: 100, h: 40 });
    expect(m.edits).toEqual([]);
  });

  it("a guide on the viewport's edge is drawn on screen", () => {
    const m = mount({ scene: snapScene() });
    down(m.hitOf("drag"), 230, 30);
    // Its right edge 476 → 480.
    move(426, 30);
    expectNear(rectOf(m.charm("drag")), { x: 380, y: 10, w: 100, h: 40 });
    expect(guides(m)).toEqual([["x", 480]]);
    expect(m.q("[data-guide]")!.style.left).toBe("479px");
    up(426, 30);
  });

  describe("keys", () => {
    beforeEach(() => {
      vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
    });

    it("an arrow beside a line moves exactly 1 px, and draws no guide", () => {
      const s = snapScene();
      s.elements[1]!.x = 186 / 480;
      const m = mount({ scene: s });
      m.charm("drag").focus();
      key("ArrowRight");
      expect(guides(m)).toEqual([]);
      expectNear(rectOf(m.charm("drag")), { x: 187, y: 10, w: 100, h: 40 }, 1e-6);
      vi.advanceTimersByTime(NUDGE_FLUSH_MS);
      expectNear(placed(m), { x: 187, y: 10, w: 100, h: 40 }, 1e-6);
      key("ArrowRight", { shiftKey: true });
      vi.advanceTimersByTime(NUDGE_FLUSH_MS);
      expectNear(placed(m), { x: 197, y: 10, w: 100, h: 40 }, 1e-3);
    });
  });
});

describe("Stage: the overlay takes no text (v1.62.0)", () => {
  /** One of every kind: the circle, a picture, a timer face, a text and the
   *  light. */
  function everyKind(): OverlayScene {
    const s = textScene();
    s.elements.push({ id: "face", type: "timer", x: 0.1, y: 0.05, w: 0.3, h: 0.1, z: 3, timer: "timer-1" });
    s.elements.push({ id: "light", type: "rec", x: 0.04, y: 0.89, w: 0.35, h: 0.07, z: 4 });
    return s;
  }

  it("no field, picker, editable or tooltip anywhere, Locked or arranging", async () => {
    const { metrics } = fakeMetrics();
    const m = mount({
      scene: everyKind(),
      status: locked(),
      timers: (timerFixture.samples as Record<string, TimerSnapshot>).running!,
      take: { state: "recording", elapsedMs: 65_000 },
      statusSeen: true,
      textMetrics: metrics,
    });
    await settle();
    const none = (): void => {
      expect(m.container.querySelector("input, textarea, select, [contenteditable]")).toBeNull();
      expect(m.container.querySelectorAll("[title]")).toHaveLength(0);
    };
    expect(m.charm("t").querySelector("[data-text-face]")).not.toBeNull();
    expect(m.charm("light").querySelector("[data-rec-face]")).not.toBeNull();
    expect(m.charm("face").querySelector("[data-timer-face]")).not.toBeNull();
    none();
    m.setStatus(arranging());
    down(m.hitOf("t"), 200, 370);
    move(210, 360);
    expect(m.q("[data-handle]")).not.toBeNull();
    none();
    up(210, 360);
    m.setTake(null);
    expect(m.charm("light").querySelector("[data-rec-idle]")).not.toBeNull();
    none();
  });

  /** The overlay's own sources: the page's entry and every non-test file
   *  in src/overlay. */
  function overlaySources(): Array<[string, string]> {
    const dir = join(process.cwd(), "src", "overlay");
    const files = readdirSync(dir)
      .filter((n) => /\.tsx?$/.test(n) && !/\.test\.tsx?$/.test(n))
      .map((n) => join(dir, n));
    files.push(join(process.cwd(), "src", "overlay.tsx"));
    return files.map((f) => [f.replace(/\\/g, "/"), readFileSync(f, "utf8")]);
  }

  it("no HTML is parsed anywhere in the overlay (D4)", () => {
    const sources = overlaySources();
    const names = sources.map(([f]) => f);
    expect(names.some((f) => f.endsWith("src/overlay/Stage.tsx"))).toBe(true);
    expect(names.some((f) => f.endsWith("src/overlay/text.ts"))).toBe(true);
    expect(names.some((f) => f.endsWith("src/overlay.tsx"))).toBe(true);
    for (const [file, source] of sources) {
      expect(source, `${file} parses HTML`).not.toMatch(/innerHTML|insertAdjacentHTML|outerHTML|DOMParser/);
    }
  });

  it("no field, picker, editable or tooltip is written in the overlay's sources (D21)", () => {
    for (const [file, source] of overlaySources()) {
      expect(source, `${file} has a field or a tooltip`).not.toMatch(/<(input|textarea|select)\b|contenteditable|\btitle=/i);
    }
  });
});
