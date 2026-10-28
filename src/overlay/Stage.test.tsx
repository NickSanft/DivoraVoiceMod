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

import { SPELL_CIRCLE_PX } from "../components/SpellCircle";
import { NUDGE_FLUSH_MS } from "./editor";
import { applyOp, layoutScene, type OverlayScene, type PxRect, type SceneOp, type Size } from "./scene";
import { BEAT_MS, circleFit, Stage } from "./Stage";
import type { OverlayBg, OverlayState } from "./state";
import type { OverlayStatus } from "./status";
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
}

function mount(o: Mount = {}) {
  const [scene, setScene] = createSignal<OverlayScene | null>(o.scene === undefined ? baseScene() : o.scene);
  const [status, setStatus] = createSignal<OverlayStatus>(o.status ?? arranging());
  const [viewport, setViewport] = createSignal<Size>(o.viewport ?? V480);
  const [state, setState] = createSignal<OverlayState | null>(o.state === undefined ? STATE : o.state);
  const [timers, setTimers] = createSignal<TimerSnapshot | null>(o.timers ?? null);
  const [fontReady, setFontReady] = createSignal(o.fontReady ?? true);
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
      onEdit={onEdit}
    />
  ));
  const q = (sel: string): HTMLElement | null => r.container.querySelector<HTMLElement>(sel);
  const charm = (id: string): HTMLElement => q(`[data-charm][data-id="${id}"]`)!;
  const hitOf = (id: string): HTMLElement => charm(id).querySelector<HTMLElement>("[data-hit]")!;
  return { ...r, q, charm, hitOf, edits, scene, setScene, setStatus, setViewport, setState, setTimers, setFontReady };
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
    expect(q("[data-hints]")!.textContent).toBe("Esc locks the overlay");
    window.dispatchEvent(new Event("blur"));
    expect(q("[data-hints]")!.textContent).toBe("Click Lock to lock the overlay");
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
