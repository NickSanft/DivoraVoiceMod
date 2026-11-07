// E2E for the stream overlay's own page (v1.55.0 Charms), Tauri mocked (see
// e2e/tauri-mock.ts): the ready and painted handshakes, Arrange's chrome and
// its beats, one edit per drag, real hit-testing, Lock inside the viewport,
// and Lock ending the beats (design §10.3). Main's side is in smoke.spec.ts;
// the last test joins the two windows.
//
// The mock never sends a status on its own: each step sends the one it
// needs with emitOverlayStatus, so each assertion is the page's answer to it.
// v1.59.0: the same for Hourglass's timers, with emitTimers.
//
// v1.62.0 (Text charms, design §10.4): text faces drawn in the real fonts
// and fitted by the page's own measure (E1); the recording light from the
// recorder's own events, through the page's status and the Hide gate (E2);
// snapping, and Ctrl placing freely, with real pointer and key events (E3);
// and a text added and edited in Settings, drawn on the overlay (E4). The
// recorder's events are sent the same way: a take's with emitTake, Replay's
// with emitReplay.

import type { Page } from "@playwright/test";
import {
  test,
  expect,
  emitOverlayStatus,
  emitReplay,
  emitsOf,
  emitTimers,
  IMPORTED_ASSET_SAMPLE,
  installTauriMock,
  invokesOf,
  linkWindows,
  listenCount,
  OVERLAY_STATUS_SAMPLES,
  pluginInvokesOf,
  TIMER_SAMPLES,
} from "./tauri-mock";
import wire from "../src/overlay/wire.fixture.json" with { type: "json" };
import type { OverlayStatus } from "../src/overlay/status";
import { drawText, FONTS, fontSpec, type TextFont } from "../src/overlay/text";
import type { TimerSnapshot } from "../src/overlay/timerState";

const INSTANCE = 3;

// A 1×1 PNG: every charm picture, whatever its id.
const PNG = Buffer.from(
  "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==",
  "base64",
);

/** What main sends on `overlay:state` (the circle's look). Motion off, so
 *  nothing moves under the pointer. */
const STATE = {
  chain: [],
  status: "clean",
  motion: 0,
  mystical: 0.7,
  mood: "violet",
  accent: "brand",
  theme: "dark",
  bg: "transparent",
};

const DESIGN = { w: 480, h: 480 };
/** The default scene's circle at 480×480: 440 px, centred → (20, 20). */
const CIRCLE = { id: "circle", type: "spellcircle", x: 20 / 480, y: 20 / 480, w: 440 / 480, h: 440 / 480, z: 0 };
/** A 2:1 picture over the circle's lower right, in front of it: at 480×480
 *  it draws at (264, 336) 144×72. */
const PICTURE = {
  id: "pic-1",
  type: "image",
  x: 0.55,
  y: 0.7,
  w: 0.3,
  h: 0.15,
  z: 1,
  src: "ba7816bf8f01cfea.png",
  nw: 2,
  nh: 1,
  name: "wave.png",
};

/** v1.59.0: BRB's face (`timer-1`), top left, in front of the circle: at
 *  480×480 it draws at (48, 24) 192×72. */
const FACE = { id: "face-1", type: "timer", x: 0.1, y: 0.05, w: 0.4, h: 0.15, z: 1, timer: "timer-1" };

// The fixture's sample is a PC that can't hide the overlay; this one can, so
// the recordings warning is due.
const ARRANGING = { ...OVERLAY_STATUS_SAMPLES.arranging, instance: INSTANCE, hideAvailable: true };

/** v1.62.0: text charm `i` of E1's six, one under another, each 432×72 at
 *  480×480 (whole pixels, so a fit has no rounding to hide in): (24,
 *  4 + 80·i). The stored `nw`/`nh` give the box its 6 : 1 shape; the page
 *  sizes the face by its own measure. */
function textRow(i: number, fields: Record<string, unknown>): Record<string, unknown> {
  return {
    id: `text-${i}`,
    type: "text",
    x: 0.05,
    y: (4 + 80 * i) / 480,
    w: 0.9,
    h: 0.15,
    z: i,
    nw: 600,
    nh: 100,
    ...fields,
  };
}

/** v1.62.0: words the bundled fonts don't have (Cyrillic, Greek, CJK and an
 *  emoji), which fall back to Windows' fonts through the face's stack. */
const NON_LATIN = "Привет Γειά 你好 🎮";

/** v1.62.0: the recording light where main's default puts it at 480×480:
 *  bottom left, 168×33.6. */
const LIGHT = { id: "rec-1", type: "rec", x: 0.04, y: 0.89, w: 0.35, h: 0.07, z: 0 };

/** v1.62.0: what the recorder sends, as the fixture freezes it (the backend's
 *  tests read the same rows). */
const REC = wire.recLight;

/** v1.62.0: fire `screen-recording` as the backend does: the whole
 *  `ScreenRecordingUpdate`, idle but for `update`. The light reads only
 *  `state` and `elapsedMs`. */
async function emitTake(page: Page, update: Record<string, unknown>): Promise<void> {
  await emit(page, REC.events.take, {
    state: "idle",
    path: null,
    elapsedMs: 0,
    droppedFrames: 0,
    driftPpm: 0,
    audioLostAt: null,
    error: null,
    desktopSourceId: null,
    desktopSource: null,
    desktopQuietSeconds: null,
    desktopDoubled: false,
    desktopLostAt: null,
    desktopError: null,
    stemsError: null,
    ...update,
  });
}

test.use({ viewport: { width: 480, height: 480 } });

/** Every console error and warning, and every uncaught error, from now on. */
function watchConsole(page: Page): string[] {
  const noise: string[] = [];
  page.on("console", (m) => {
    if (m.type() === "error" || m.type() === "warning") noise.push(`${m.type()}: ${m.text()}`);
  });
  page.on("pageerror", (e) => noise.push(`pageerror: ${String(e)}`));
  return noise;
}

async function emit(page: Page, event: string, payload: unknown): Promise<void> {
  await page.evaluate(
    ([e, p]) =>
      (window as unknown as { __E2E_EMIT__: (e: string, p: unknown) => void }).__E2E_EMIT__(
        e as string,
        p,
      ),
    [event, payload] as const,
  );
}

function scenePush(elements: unknown[], revision = 1, ack: { instance: number; seq: number } | null = null) {
  return {
    session: "main-1",
    revision,
    scene: { version: 1, designSize: DESIGN, frame: "window", elements },
    ack,
    expectViewport: null,
  };
}

const handshakes = (page: Page) => invokesOf(page, "overlay_handshake");
const beats = (page: Page) => invokesOf(page, "overlay_alive");
const lockRequests = (page: Page) => invokesOf(page, "set_overlay_mode");
const edits = (page: Page) =>
  emitsOf(page, "overlay:edit") as Promise<
    Array<{ instance: number; seq: number; op: { kind: string; id: string; rect: Record<string, number>; viewport: unknown } }>
  >;

/** Load this window's page and wait for its ready handshake. */
async function load(page: Page): Promise<void> {
  await page.route("http://charm.localhost/**", (route) =>
    route.fulfill({ status: 200, contentType: "image/png", body: PNG }),
  );
  await page.goto(`/overlay.html?instance=${INSTANCE}`);
  await expect.poll(() => handshakes(page)).toHaveLength(1);
}

/** Draw `elements`, show the window and wait for its painted handshake. */
async function shown(page: Page, elements: unknown[]): Promise<void> {
  await emit(page, "overlay:state", STATE);
  await emit(page, "overlay:scene", scenePush(elements));
  await expect(page.locator("[data-charm]")).toHaveCount(elements.length);
  await emitOverlayStatus(page, { open: true, phase: "shown", instance: INSTANCE });
  await expect.poll(() => handshakes(page)).toHaveLength(2);
  await emitOverlayStatus(page, { phase: "painted", canArrange: true });
}

/** Enter Arrange (generation 1, recording with Hide on) and wait for the
 *  first beat. */
async function arranging(page: Page): Promise<void> {
  await emitOverlayStatus(page, ARRANGING);
  await expect(page.locator("[data-chrome]")).toBeVisible();
  await expect.poll(async () => (await beats(page)).length).toBeGreaterThanOrEqual(1);
}

function center(b: { x: number; y: number; width: number; height: number }): { x: number; y: number } {
  return { x: b.x + b.width / 2, y: b.y + b.height / 2 };
}

/** Where `(x, y)` hits: inside a charm (its id), the grip, the Lock button,
 *  or the stage itself. */
function hitAt(page: Page, x: number, y: number): Promise<string> {
  return page.evaluate(
    ([px, py]) => {
      const el = document.elementFromPoint(px as number, py as number);
      if (!el) return "nothing";
      const charm = el.closest("[data-charm]");
      if (charm) return `charm:${charm.getAttribute("data-id")}`;
      if (el.closest("[data-grip]")) return "grip";
      if (el.closest("[data-lock]")) return "lock";
      if (el.closest("[data-chrome]")) return "chrome";
      if (el.hasAttribute("data-stage")) return "stage";
      return el.tagName;
    },
    [x, y] as const,
  );
}

test.describe("stream overlay page (v1.55)", () => {
  test("1-3: says ready on load, and painted only once its window is shown", async ({ page }) => {
    const noise = watchConsole(page);
    await load(page);
    expect(await handshakes(page)).toEqual([
      { cmd: "overlay_handshake", args: { instance: INSTANCE, stage: "ready" } },
    ]);
    // The broadcast main re-pushes on, also sent.
    expect(await emitsOf(page, "overlay:ready")).toHaveLength(1);
    // Nothing drawn before main's state and scene: OBS would capture it.
    await expect(page.locator("[data-charm]")).toHaveCount(0);

    // Not shown yet, or another window shown: no painted.
    await page.waitForTimeout(300);
    await emitOverlayStatus(page, { open: true, phase: "shown", instance: INSTANCE - 1 });
    await page.waitForTimeout(300);
    expect(await handshakes(page)).toHaveLength(1);

    await emitOverlayStatus(page, { open: true, phase: "shown", instance: INSTANCE });
    await expect.poll(() => handshakes(page)).toEqual([
      { cmd: "overlay_handshake", args: { instance: INSTANCE, stage: "ready" } },
      { cmd: "overlay_handshake", args: { instance: INSTANCE, stage: "painted" } },
    ]);
    // Once per window.
    await emitOverlayStatus(page, { phase: "shown" });
    await page.waitForTimeout(300);
    expect(await handshakes(page)).toHaveLength(2);

    expect(noise, noise.join("\n")).toEqual([]);
  });

  test("4-5: Arrange draws the chrome and beats; each drag sends one edit, on release", async ({
    page,
  }) => {
    const noise = watchConsole(page);
    await load(page);
    await shown(page, [CIRCLE, PICTURE]);
    // Locked: no chrome, no outlines, no beats.
    await expect(page.locator("[data-chrome]")).toHaveCount(0);
    await expect(page.locator("[data-outline]")).toHaveCount(0);
    expect(await beats(page)).toEqual([]);

    await arranging(page);
    await expect(page.getByRole("toolbar", { name: "Arrange" })).toBeVisible();
    await expect(page.getByRole("button", { name: "Lock the overlay" })).toBeVisible();
    await expect(page.locator("[data-grip]")).toBeVisible();
    await expect(page.locator("[data-hints]")).toContainText(
      "Recordings can see the overlay while you arrange.",
    );
    await expect(page.locator("[data-outline]")).toHaveCount(2);
    // Every beat names this window and this Arrange; they keep coming.
    await expect.poll(async () => (await beats(page)).length).toBeGreaterThanOrEqual(3);
    for (const b of await beats(page)) expect(b.args).toEqual({ instance: INSTANCE, gen: 1 });

    // Drag the picture (in front of the circle there): 10 moves, then release.
    // v1.62.0: to (144, 186), where no edge or centre is within 8 px of the
    // window's or the circle's lines, so nothing snaps (E3 covers snapping).
    const row = page.locator('[data-charm][data-id="pic-1"]');
    const hit = row.locator("[data-hit]");
    const before = (await row.boundingBox())!;
    expect(before).toMatchObject({ x: 264, y: 336, width: 144, height: 72 });
    const from = center((await hit.boundingBox())!);
    await page.mouse.move(from.x, from.y);
    await page.mouse.down();
    for (let i = 1; i <= 10; i++) await page.mouse.move(from.x - 12 * i, from.y - 15 * i);
    // The preview followed the pointer, and nothing was sent while it did.
    await expect.poll(async () => (await row.boundingBox())?.x).toBeCloseTo(144, 0);
    expect((await row.boundingBox())!.y).toBeCloseTo(186, 0);
    expect(await edits(page)).toEqual([]);
    await page.mouse.up();
    await expect.poll(() => edits(page)).toHaveLength(1);
    const [first] = await edits(page);
    expect(first).toMatchObject({
      instance: INSTANCE,
      seq: 0,
      op: { kind: "place", id: "pic-1", viewport: { w: 480, h: 480 } },
    });
    expect(first!.op.rect.x).toBeCloseTo(144, 3);
    expect(first!.op.rect.y).toBeCloseTo(186, 3);
    expect(first!.op.rect.w).toBeCloseTo(144, 3);
    expect(first!.op.rect.h).toBeCloseTo(72, 3);
    // It stays where it was dropped, and stays one edit.
    await page.waitForTimeout(400);
    expect(await edits(page)).toHaveLength(1);
    expect((await row.boundingBox())!.x).toBeCloseTo(144, 0);

    // A drag that ends over the chrome's Lock button: still one edit, and no
    // lock.
    const again = center((await hit.boundingBox())!);
    const lock = center((await page.locator("[data-lock]").boundingBox())!);
    await page.mouse.move(again.x, again.y);
    await page.mouse.down();
    for (let i = 1; i <= 10; i++) {
      await page.mouse.move(again.x + ((lock.x - again.x) * i) / 10, again.y + ((lock.y - again.y) * i) / 10);
    }
    expect(await edits(page)).toHaveLength(1);
    expect(await hitAt(page, lock.x, lock.y)).toBe("lock");
    await page.mouse.up();
    await expect.poll(() => edits(page)).toHaveLength(2);
    const second = (await edits(page))[1]!;
    expect(second).toMatchObject({ instance: INSTANCE, seq: 1, op: { kind: "place", id: "pic-1" } });
    await page.waitForTimeout(400);
    expect(await edits(page)).toHaveLength(2);
    expect(await lockRequests(page)).toEqual([]);

    expect(noise, noise.join("\n")).toEqual([]);
  });

  test("6: hit-testing is real: the circle is round, the grip and an empty spot move the window", async ({
    page,
  }) => {
    const noise = watchConsole(page);
    await load(page);
    await shown(page, [CIRCLE]);
    const drags = () => pluginInvokesOf(page, "plugin:window|start_dragging");

    // Locked: the circle's square corner is empty stage; its middle is the
    // circle.
    expect(await hitAt(page, 28, 28)).toBe("stage");
    expect(await hitAt(page, 240, 240)).toBe("charm:circle");

    await arranging(page);
    // (28, 28) is now the Move window grip; still not a charm.
    expect(await hitAt(page, 28, 28)).not.toMatch(/^charm:/);
    expect(await hitAt(page, 28, 28)).toBe("grip");
    expect(await hitAt(page, 240, 240)).toBe("charm:circle");
    // Inside the circle's box, outside the circle.
    expect(await hitAt(page, 30, 450)).toBe("stage");

    // The grip drags this window.
    const grip = center((await page.locator("[data-grip]").boundingBox())!);
    await page.mouse.move(grip.x, grip.y);
    await page.mouse.down();
    await page.mouse.up();
    await expect.poll(drags).toEqual([{ cmd: "plugin:window|start_dragging", args: { label: "overlay" } }]);

    // So does an empty spot, even inside the circle's box.
    await page.mouse.move(30, 450);
    await page.mouse.down();
    await page.mouse.up();
    await expect.poll(async () => (await drags()).length).toBe(2);

    // The circle selects instead, and a press that doesn't move sends nothing.
    await page.mouse.move(240, 240);
    await page.mouse.down();
    await page.mouse.up();
    await expect(page.locator('[data-charm][data-id="circle"] [data-handle]')).toHaveCount(4);
    await page.waitForTimeout(300);
    expect(await drags()).toHaveLength(2);
    expect(await edits(page)).toEqual([]);

    // Fitted, the window is the monitor: no grip, and an empty spot stays put.
    await emitOverlayStatus(page, { fit: true });
    await expect(page.locator("[data-grip]")).toHaveCount(0);
    await page.mouse.move(30, 450);
    await page.mouse.down();
    await page.mouse.up();
    await page.waitForTimeout(300);
    expect(await drags()).toHaveLength(2);

    expect(noise, noise.join("\n")).toEqual([]);
  });

  test("7: Lock stays inside the window at 136×39, 200×200 and 480×480", async ({ page }) => {
    const noise = watchConsole(page);
    await load(page);
    await shown(page, [CIRCLE, PICTURE]);
    // Recording with Hide on: the longest hint is due.
    await arranging(page);
    expect(ARRANGING).toMatchObject({ capturing: true, hideWanted: true, hideAvailable: true });

    const sizes = [
      { width: 136, height: 39, text: false, hints: false },
      { width: 200, height: 200, text: true, hints: false },
      { width: 480, height: 480, text: true, hints: true },
    ];
    const lock = page.getByRole("button", { name: "Lock the overlay" });
    for (const [i, s] of sizes.entries()) {
      await page.setViewportSize({ width: s.width, height: s.height });
      // The stage follows the window a frame later.
      await expect(page.locator("[data-hints]")).toHaveCount(s.hints ? 1 : 0);
      await expect(lock.getByText("Lock", { exact: true })).toHaveCount(s.text ? 1 : 0);
      if (s.hints) {
        await expect(page.locator("[data-hints]")).toContainText(
          "Recordings can see the overlay while you arrange.",
        );
      }
      const at = await lock.evaluate((el) => {
        const r = el.getBoundingClientRect();
        const top = document.elementFromPoint(r.left + r.width / 2, r.top + r.height / 2);
        return {
          left: r.left,
          top: r.top,
          right: r.right,
          bottom: r.bottom,
          w: window.innerWidth,
          h: window.innerHeight,
          onTop: top !== null && el.contains(top),
        };
      });
      expect(at.w, `viewport ${s.width}×${s.height}`).toBe(s.width);
      expect(at.h).toBe(s.height);
      expect(at.left, `left at ${s.width}×${s.height}`).toBeGreaterThanOrEqual(0);
      expect(at.top, `top at ${s.width}×${s.height}`).toBeGreaterThanOrEqual(0);
      expect(at.right, `right at ${s.width}×${s.height}`).toBeLessThanOrEqual(s.width);
      expect(at.bottom, `bottom at ${s.width}×${s.height}`).toBeLessThanOrEqual(s.height);
      // Nothing covers it.
      expect(at.onTop, `Lock is on top at ${s.width}×${s.height}`).toBe(true);

      await lock.click();
      await expect.poll(() => lockRequests(page)).toHaveLength(i + 1);
    }
    for (const r of await lockRequests(page)) expect(r.args).toEqual({ mode: "locked" });

    expect(noise, noise.join("\n")).toEqual([]);
  });

  test("8-9: Esc asks to lock; the locked status takes the chrome down and ends the beats", async ({
    page,
  }) => {
    const noise = watchConsole(page);
    await load(page);
    await shown(page, [CIRCLE, PICTURE]);
    await arranging(page);
    // A charm selected, so Esc has something to drop too.
    await page.mouse.click(240, 240);
    await expect(page.locator("[data-handle]")).toHaveCount(4);

    await page.keyboard.press("Escape");
    await expect.poll(() => lockRequests(page)).toEqual([
      { cmd: "set_overlay_mode", args: { mode: "locked" } },
    ]);
    // The backend decides: until its status says so, the page still arranges.
    await expect(page.locator("[data-chrome]")).toBeVisible();

    await emitOverlayStatus(page, { mode: "locked" });
    await expect(page.locator("[data-chrome]")).toHaveCount(0);
    await expect(page.locator("[data-outline]")).toHaveCount(0);
    await expect(page.locator("[data-handle]")).toHaveCount(0);
    // A beat already on its way may still land; none after it.
    await page.waitForTimeout(150);
    const after = (await beats(page)).length;
    await page.waitForTimeout(1_500);
    expect(await beats(page)).toHaveLength(after);

    // Locked, Esc is nothing.
    await page.keyboard.press("Escape");
    await page.waitForTimeout(200);
    expect(await lockRequests(page)).toHaveLength(1);

    expect(noise, noise.join("\n")).toEqual([]);
  });
});

test.describe("Hourglass timer face (v1.59)", () => {
  /** BRB (`timer-1`) alone, from one of the fixture's snapshots. */
  const brb = (s: TimerSnapshot): TimerSnapshot => ({
    seq: s.seq,
    timers: s.timers.filter((t) => t.id === "timer-1"),
  });

  test("draws only the backend's text, keeps the newest, updates with motion off, and says when its timer is missing", async ({
    page,
  }) => {
    const noise = watchConsole(page);
    await load(page);
    // The page hears the backend itself, not through main.
    expect(await listenCount(page, "timer-state")).toBe(1);
    await shown(page, [CIRCLE, FACE]);
    const row = page.locator('[data-charm][data-id="face-1"]');
    const face = row.locator("[data-timer-face]");
    const badge = row.locator("[data-missing]");

    // It asks once, after the ready handshake; the mock answers no snapshot.
    await expect.poll(() => invokesOf(page, "timer_status")).toEqual([
      { cmd: "timer_status", args: {} },
    ]);
    // Before the backend's first snapshot: no face, which OBS would capture
    // as a made-up time…
    await page.waitForTimeout(300);
    await expect(face).toHaveCount(0);
    // …and in Arrange no badge, though every timer is "missing" until then.
    await arranging(page);
    await page.waitForTimeout(300);
    await expect(badge).toHaveCount(0);
    await expect(face).toHaveCount(0);
    await emitOverlayStatus(page, { mode: "locked" });
    await expect(page.locator("[data-chrome]")).toHaveCount(0);

    await emitTimers(page, brb(TIMER_SAMPLES.idle));
    await expect(face).toHaveText("05:00");
    await expect(row).toHaveAttribute("aria-label", "Timer: BRB");
    await emitTimers(page, brb(TIMER_SAMPLES.running));
    await expect(face).toHaveText("04:59");
    // An older snapshot, arriving late, changes nothing.
    await emitTimers(page, brb(TIMER_SAMPLES.idle));
    await page.waitForTimeout(300);
    await expect(face).toHaveText("04:59");

    // Motion is off (main's state says 0): Functional cuts every CSS
    // animation short, and nothing on the face is one, so it still follows
    // the backend.
    expect(await page.evaluate(() => document.documentElement.dataset.motion)).toBe("functional");
    expect(await face.evaluate((el) => getComputedStyle(el).animationName)).toBe("none");
    expect(await face.getAttribute("style")).not.toMatch(/animation|transition/);
    await emitTimers(page, brb(TIMER_SAMPLES.done));
    await expect(face).toHaveText("00:00");

    // Its timer gone from the backend's set: locked, nothing at all; in
    // Arrange, the badge.
    await emitTimers(page, { seq: 4, timers: [] });
    await expect(face).toHaveCount(0);
    await page.waitForTimeout(300);
    await expect(badge).toHaveCount(0);
    await emitOverlayStatus(page, { mode: "arrange", arrangeGen: 2 });
    await expect(page.locator("[data-chrome]")).toBeVisible();
    await expect(badge).toHaveText("Timer missing");
    await expect(face).toHaveCount(0);

    expect(noise, noise.join("\n")).toEqual([]);
  });
});

test.describe("Text charms (v1.62)", () => {
  test("E1: each text draws in its font, as text, fitted inside its box by the page's own measure", async ({
    page,
  }) => {
    const noise = watchConsole(page);
    await load(page);
    const texts = [
      textRow(0, { text: "BRB", font: "display", bold: true, color: "#ffd60a", outline: true, align: "center" }),
      textRow(1, {
        text: "Back in 5\nGrab a snack",
        font: "sans",
        bold: false,
        color: "#ffffff",
        outline: false,
        align: "left",
      }),
      textRow(2, { text: "<b>hi</b>", font: "mono", bold: true, color: "#111111", outline: true, align: "right" }),
      textRow(3, { text: NON_LATIN, font: "display", bold: true, color: "#ffffff", outline: true, align: "center" }),
      textRow(4, { text: NON_LATIN, font: "sans", bold: false, color: "#64d2ff", outline: true, align: "center" }),
      textRow(5, { text: NON_LATIN, font: "mono", bold: true, color: "#ff9f0a", outline: true, align: "center" }),
    ];
    await shown(page, texts);
    // Each face draws once its font has loaded for its words (or a second
    // on); a font that comes later measures again. Settled, nothing more
    // changes.
    await expect(page.locator("[data-text-face]")).toHaveCount(texts.length);
    await page.evaluate(async () => {
      await document.fonts.ready;
      await new Promise<void>((done) => requestAnimationFrame(() => requestAnimationFrame(() => done())));
    });

    for (const t of texts) {
      const id = t.id as string;
      const words = t.text as string;
      const font = t.font as TextFont;
      const bold = t.bold as boolean;
      const row = page.locator(`[data-charm][data-id="${id}"]`);
      await expect(row, id).toHaveAttribute("aria-label", `Text: ${drawText(words).split("\n").join(" ")}`);
      const face = row.locator("[data-text-face]");
      // The words as text: one text node, never markup.
      await expect(face, id).toHaveText(words);
      const seen = await face.evaluate(
        (el, a) => {
          const lines = el.firstElementChild as HTMLElement;
          const node = lines.firstChild as Text;
          const box = (el.closest("[data-charm]") as HTMLElement).getBoundingClientRect();
          const css = getComputedStyle(el);
          // Where each line's words sit, and their glyphs' ink. A Range's
          // box runs across the words' advances, but up and down it is the
          // font's content area (its ascent and descent), not ink: Space
          // Mono Bold's is about 1.47 em, taller than its 1.15 em line, so
          // it reads past a row its glyphs are well inside. So the ink up
          // and down is the canvas's, at the face's font, from each line's
          // baseline (the Range's top plus the face's own font's ascent).
          const ctx = document.createElement("canvas").getContext("2d")!;
          ctx.font = `${css.fontWeight} ${css.fontSize} ${css.fontFamily}`;
          ctx.fontKerning = "normal";
          ctx.letterSpacing = "0px";
          ctx.textRendering = "optimizeLegibility";
          const ascent = ctx.measureText("x").fontBoundingBoxAscent;
          const words = { left: Infinity, right: -Infinity };
          const ink = { left: Infinity, top: Infinity, right: -Infinity, bottom: -Infinity };
          let at = 0;
          for (const line of node.data.split("\n")) {
            if (line.length > 0) {
              const range = document.createRange();
              range.setStart(node, at);
              range.setEnd(node, at + line.length);
              const r = range.getBoundingClientRect();
              const m = ctx.measureText(line);
              const baseline = r.top + ascent;
              words.left = Math.min(words.left, r.left);
              words.right = Math.max(words.right, r.right);
              ink.left = Math.min(ink.left, r.left - m.actualBoundingBoxLeft);
              ink.right = Math.max(ink.right, r.left + m.actualBoundingBoxRight);
              ink.top = Math.min(ink.top, baseline - m.actualBoundingBoxAscent);
              ink.bottom = Math.max(ink.bottom, baseline + m.actualBoundingBoxDescent);
            }
            at += line.length + 1;
          }
          return {
            family: css.fontFamily,
            weight: css.fontWeight,
            color: css.color,
            fontSize: parseFloat(css.fontSize),
            elements: el.querySelectorAll("*").length,
            textNodes: Array.from(lines.childNodes).filter((n) => n.nodeType === Node.TEXT_NODE).length,
            backstop: el.getAttribute("data-fit-backstop"),
            // "Loaded" says nothing about fallback: it is also true when no
            // face in the stack has those letters.
            loaded: document.fonts.check(a.spec, a.words),
            words,
            ink,
            box: { left: box.left, top: box.top, right: box.right, bottom: box.bottom },
            face: { w: el.clientWidth, h: el.clientHeight },
            laid: { w: lines.offsetWidth, h: lines.offsetHeight },
          };
        },
        { spec: fontSpec(font, bold), words },
      );
      // The family from text.ts's table, first in the face's stack.
      expect(seen.family.replace(/["']/g, ""), id).toMatch(new RegExp(`^${FONTS[font].family},`));
      expect(seen.weight, id).toBe(bold ? "700" : "400");
      // The face itself and the one element holding its words.
      expect(seen.elements, id).toBe(1);
      expect(seen.textNodes, id).toBe(1);
      expect(seen.fontSize, id).toBeGreaterThan(0);
      if (words !== NON_LATIN) expect(seen.loaded, `${id}: its font loaded`).toBe(true);
      // Inside its row, give or take a pixel's rounding: the words across
      // (where a font measured differently would spill), and their ink.
      expect(seen.words.left, `${id} words left`).toBeGreaterThanOrEqual(seen.box.left - 1);
      expect(seen.words.right, `${id} words right`).toBeLessThanOrEqual(seen.box.right + 1);
      expect(seen.ink.left, `${id} ink left`).toBeGreaterThanOrEqual(seen.box.left - 1);
      expect(seen.ink.top, `${id} ink top`).toBeGreaterThanOrEqual(seen.box.top - 1);
      expect(seen.ink.right, `${id} ink right`).toBeLessThanOrEqual(seen.box.right + 1);
      expect(seen.ink.bottom, `${id} ink bottom`).toBeLessThanOrEqual(seen.box.bottom + 1);
      // Laid out, it fits (whole pixels). Its laid-out size, not its scroll
      // size, which counts each font's content area too (see above).
      expect(seen.laid.w, `${id} laid-out width`).toBeLessThanOrEqual(seen.face.w + 1);
      expect(seen.laid.h, `${id} laid-out height`).toBeLessThanOrEqual(seen.face.h + 1);
      // The canvas measured what the face draws, so the backstop never cut it.
      expect(seen.backstop, `${id} backstop`).toBe("0");
    }
    // Markup is shown as typed.
    const markup = page.locator('[data-charm][data-id="text-2"]');
    await expect(markup.locator("[data-text-face]")).toHaveText("<b>hi</b>");
    await expect(markup.locator("b")).toHaveCount(0);

    expect(noise, noise.join("\n")).toEqual([]);
  });

  test("E2: the light follows the recorder's events, waits for the page's own status, and keeps the Hide gate", async ({
    page,
  }) => {
    const noise = watchConsole(page);
    await load(page);
    // The page hears the recorder itself, and reads both statuses once,
    // after its ready handshake (the mock answers idle and off).
    expect(await listenCount(page, REC.events.take)).toBe(1);
    expect(await listenCount(page, REC.events.replay)).toBe(1);
    await expect.poll(() => invokesOf(page, REC.commands.take)).toEqual([{ cmd: REC.commands.take, args: {} }]);
    await expect.poll(() => invokesOf(page, REC.commands.replay)).toEqual([
      { cmd: REC.commands.replay, args: {} },
    ]);

    // Main's state and scene, but no status for this window yet.
    await emit(page, "overlay:state", STATE);
    await emit(page, "overlay:scene", scenePush([LIGHT]));
    const row = page.locator('[data-charm][data-id="rec-1"]');
    await expect(row).toHaveAttribute("aria-label", "Recording light");
    const face = row.locator("[data-rec-face]");
    const idle = row.locator("[data-rec-idle]");

    // A take starting before the page has its own status: nothing. The
    // status the handshake causes may come after a take's answer.
    await emitTake(page, { state: "starting" });
    await page.waitForTimeout(300);
    await expect(face).toHaveCount(0);
    await expect(idle).toHaveCount(0);

    // Its status (Locked, Hide off): the face, with its dot.
    await emitOverlayStatus(page, { open: true, phase: "shown", instance: INSTANCE });
    await expect(face).toHaveText("REC");
    await expect(face.locator("[data-rec-dot]")).toBeVisible();
    await expect(face.locator("[data-rec-dot]")).toHaveCSS("background-color", "rgb(255, 69, 0)");
    await expect(idle).toHaveCount(0);

    await emitTake(page, REC.samples.take);
    expect(REC.samples.take).toEqual({ state: "recording", elapsedMs: 65_000 });
    await expect(face).toHaveText("REC 1:05");
    await emitTake(page, { state: "idle", elapsedMs: 65_000 });
    await expect(face).toHaveCount(0);
    await expect(idle).toHaveCount(0);

    await emitReplay(page, REC.samples.replay);
    expect(REC.samples.replay).toEqual({ state: "armed" });
    await expect(face).toHaveText("REPLAY");

    // Hide from recordings on: nothing until Windows is asked to leave the
    // overlay out (`capturing`), then the face.
    await emitOverlayStatus(page, { hideWanted: true, hideAvailable: true, capturing: false });
    await expect(face).toHaveCount(0);
    await page.waitForTimeout(300);
    await expect(face).toHaveCount(0);
    await emitOverlayStatus(page, { capturing: true });
    await expect(face).toHaveText("REPLAY");
    // A take inside Replay, then its Stop: the take's part goes at Stop,
    // while the overlay is still asked to be left out.
    await emitTake(page, { state: "recording", elapsedMs: 5_000 });
    await expect(face).toHaveText("REC 0:05");
    await emitTake(page, { state: "stopping", elapsedMs: 6_000 });
    await expect(face).toHaveText("REPLAY");
    await emitTake(page, { state: "idle", elapsedMs: 6_000 });
    await emitReplay(page, { state: "off" });
    await expect(face).toHaveCount(0);

    // Arrange: a dimmed placeholder to place, and no gate (Arrange is never
    // left out, and the page's hint says so).
    await emitOverlayStatus(page, { mode: "arrange", arrangeGen: 1, canArrange: true, phase: "painted" });
    await expect(page.locator("[data-chrome]")).toBeVisible();
    await expect(idle).toHaveText("REC");
    await expect(idle).toHaveCSS("opacity", "0.45");
    await expect(face).toHaveCount(0);
    await emitOverlayStatus(page, { capturing: false });
    await emitTake(page, REC.samples.take);
    await expect(face).toHaveText("REC 1:05");
    await expect(idle).toHaveCount(0);
    // Locked again with Hide on and nothing asked: gone.
    await emitOverlayStatus(page, { mode: "locked", arrangeGen: 0 });
    await expect(page.locator("[data-chrome]")).toHaveCount(0);
    await expect(face).toHaveCount(0);
    await expect(idle).toHaveCount(0);

    expect(noise, noise.join("\n")).toEqual([]);
  });

  test("E3: a drag snaps to the overlay's centre line with a guide; Ctrl places it freely; arrows never snap", async ({
    page,
  }) => {
    const noise = watchConsole(page);
    await load(page);
    // One charm, 144×48 at (48, 100): the only lines are the window's.
    const brb = {
      id: "text-1",
      type: "text",
      x: 0.1,
      y: 100 / 480,
      w: 0.3,
      h: 0.1,
      z: 0,
      text: "BRB",
      font: "sans",
      bold: true,
      color: "#ffffff",
      outline: true,
      align: "center",
      nw: 300,
      nh: 100,
    };
    await shown(page, [brb]);
    await arranging(page);
    const row = page.locator('[data-charm][data-id="text-1"]');
    const guides = page.locator("[data-guide]");
    expect((await row.boundingBox())!).toMatchObject({ x: 48, y: 100, width: 144, height: 48 });

    // Its centre from 120 to 245, 5 px past the window's centre line: the
    // preview jumps onto it, with a guide down the window. Nothing else is
    // within 8 px of a line, so nothing else snaps.
    await page.mouse.move(120, 124);
    await page.mouse.down();
    for (let i = 1; i <= 5; i++) await page.mouse.move(120 + 25 * i, 124);
    await expect(guides).toHaveCount(1);
    await expect(page.locator('[data-guide][data-axis="x"]')).toHaveAttribute("data-at", "240");
    await expect.poll(async () => (await row.boundingBox())?.x).toBeCloseTo(168, 1);
    expect(await edits(page)).toEqual([]);
    await page.mouse.up();
    await expect.poll(() => edits(page)).toHaveLength(1);
    const snapped = (await edits(page))[0]!.op;
    expect(snapped).toMatchObject({ kind: "place", id: "text-1", viewport: { w: 480, h: 480 } });
    expect(Math.abs(snapped.rect.x! + snapped.rect.w! / 2 - 240)).toBeLessThanOrEqual(0.01);
    expect(snapped.rect.y).toBeCloseTo(100, 2);
    expect(snapped.rect.w).toBeCloseTo(144, 2);
    expect(snapped.rect.h).toBeCloseTo(48, 2);
    // Guides only while the pointer is down.
    await expect(guides).toHaveCount(0);

    // With Ctrl held, the same 5 px stays where the pointer put it: no guide.
    await page.keyboard.down("Control");
    await page.mouse.move(240, 124);
    await page.mouse.down();
    for (let i = 1; i <= 4; i++) await page.mouse.move(240 - 15 * i, 124);
    await page.mouse.move(235, 124);
    await expect.poll(async () => (await row.boundingBox())?.x).toBeCloseTo(163, 1);
    await expect(guides).toHaveCount(0);
    // Ctrl let go mid-drag snaps at once, where the pointer is; pressed
    // again, free again.
    await page.keyboard.up("Control");
    await expect(page.locator('[data-guide][data-axis="x"]')).toHaveAttribute("data-at", "240");
    await expect.poll(async () => (await row.boundingBox())?.x).toBeCloseTo(168, 1);
    await page.keyboard.down("Control");
    await expect(guides).toHaveCount(0);
    await expect.poll(async () => (await row.boundingBox())?.x).toBeCloseTo(163, 1);
    await page.mouse.up();
    await page.keyboard.up("Control");
    await expect.poll(() => edits(page)).toHaveLength(2);
    const free = (await edits(page))[1]!.op;
    expect(free).toMatchObject({ kind: "place", id: "text-1" });
    expect(Math.abs(free.rect.x! + free.rect.w! / 2 - 235)).toBeLessThanOrEqual(0.01);
    expect(free.rect.y).toBeCloseTo(100, 2);

    // An arrow nudges 1 px, though the centre is then 4 px from the line.
    await page.keyboard.press("ArrowRight");
    await expect.poll(() => edits(page)).toHaveLength(3);
    const nudged = (await edits(page))[2]!.op;
    expect(nudged).toMatchObject({ kind: "place", id: "text-1" });
    expect(Math.abs(nudged.rect.x! + nudged.rect.w! / 2 - 236)).toBeLessThanOrEqual(0.01);
    await expect(guides).toHaveCount(0);

    expect(noise, noise.join("\n")).toEqual([]);
  });
});

// The two halves joined: main's controller and Settings (smoke.spec.ts) and
// this page, with each window's broadcasts carried to the other as the event
// bus would. Each half passing alone doesn't show the edit main receives is
// the one the overlay sent, or that main's answer lands where it was dropped.
test.describe("main window and overlay together (v1.55)", () => {
  test("a picture added in Settings shows on the overlay; a drag there is saved by main", async ({
    page: main,
    context,
    consoleErrors,
  }) => {
    await main.setViewportSize({ width: 1280, height: 720 });
    const overlay = await context.newPage();
    await installTauriMock(overlay);
    const noise = watchConsole(overlay);
    await linkWindows(main, overlay);
    await overlay.route("http://charm.localhost/**", (route) =>
      route.fulfill({ status: 200, contentType: "image/png", body: PNG }),
    );
    /** The backend's status, to both windows. */
    const broadcast = async (partial: Partial<OverlayStatus>): Promise<void> => {
      const s = await emitOverlayStatus(main, partial);
      await emitOverlayStatus(overlay, s);
    };

    await main.goto("/");
    await expect(main.locator(".titlebar-wordmark")).toBeVisible();
    await main.locator(".sidebar-nav", { hasText: "Settings" }).click();
    await main.getByRole("switch", { name: "Show stream overlay window" }).click();
    await expect.poll(() => invokesOf(main, "open_overlay")).toHaveLength(1);

    // The backend opens window 1. Its ready broadcast makes main send the
    // state and the scene, and the circle draws from both.
    await overlay.goto("/overlay.html?instance=1");
    await expect(overlay.locator('[data-charm][data-id="circle"] [data-circle]')).toBeVisible();
    await broadcast({ phase: "shown" });
    await expect.poll(() => invokesOf(overlay, "overlay_handshake")).toHaveLength(2);
    await broadcast({ phase: "painted", canArrange: true });

    // Add a picture from Settings: the import's answer becomes a charm on
    // the overlay, loaded from the charm scheme.
    const path = "C:\\Users\\me\\Pictures\\wave.gif";
    await main.evaluate((p) => {
      (window as unknown as { __E2E_DIALOG_PICK__: string }).__E2E_DIALOG_PICK__ = p;
    }, path);
    await main.getByTestId("overlay-add-picture").click();
    await expect.poll(() => invokesOf(main, "import_overlay_asset")).toEqual([
      { cmd: "import_overlay_asset", args: { path } },
    ]);
    await expect(main.getByTestId("toast")).toHaveText("Picture added. Use Arrange to place it.");
    await expect(main.getByTestId("overlay-charms")).toContainText("wave.gif · 498×280 · animated");
    const picture = overlay.locator('[data-charm]:not([data-id="circle"])');
    await expect(picture).toHaveCount(1);
    await expect(picture.locator("img")).toHaveAttribute(
      "src",
      `http://charm.localhost/${IMPORTED_ASSET_SAMPLE.id}`,
    );
    await expect(picture).toHaveAttribute("aria-label", "Picture: wave.gif");
    const box = (await picture.boundingBox())!;
    // The picture's own shape.
    expect(box.width / box.height).toBeCloseTo(498 / 280, 2);

    // Arrange from Settings; the backend tells the overlay.
    await main.getByTestId("overlay-arrange").click();
    await expect(main.getByTestId("overlay-arrange")).toHaveText("Lock");
    await broadcast({});
    await expect(overlay.locator("[data-chrome]")).toBeVisible();
    await expect.poll(async () => (await invokesOf(overlay, "overlay_alive")).length).toBeGreaterThanOrEqual(1);
    expect((await invokesOf(overlay, "overlay_alive"))[0]!.args).toEqual({ instance: 1, gen: 1 });

    // Drag it on the overlay. Main applies the edit, saves it and answers
    // with a push that acks it. v1.62.0: 30 px up and left, where no edge or
    // centre is within 8 px of the window's or the circle's lines, so
    // nothing snaps.
    const from = center((await picture.locator("[data-hit]").boundingBox())!);
    await overlay.mouse.move(from.x, from.y);
    await overlay.mouse.down();
    for (let i = 1; i <= 10; i++) await overlay.mouse.move(from.x - 3 * i, from.y - 3 * i);
    await overlay.mouse.up();
    await expect.poll(async () => (await emitsOf(overlay, "overlay:edit")).length).toBe(1);
    await expect
      .poll(async () => ((await emitsOf(main, "overlay:scene")).at(-1) as { ack: unknown }).ack)
      .toEqual({ instance: 1, seq: 0 });
    const dropped = (await picture.boundingBox())!;
    expect(dropped.x).toBeCloseTo(box.x - 30, 0);
    expect(dropped.y).toBeCloseTo(box.y - 30, 0);
    // Acked, main's scene is what's drawn, and it stays put.
    await overlay.waitForTimeout(300);
    const acked = (await picture.boundingBox())!;
    expect(acked.x).toBeCloseTo(dropped.x, 1);
    expect(acked.y).toBeCloseTo(dropped.y, 1);
    expect(acked.width).toBeCloseTo(dropped.width, 1);

    // Esc on the overlay asks to lock; the backend's answer reaches both.
    await overlay.keyboard.press("Escape");
    await expect.poll(() => invokesOf(overlay, "set_overlay_mode")).toEqual([
      { cmd: "set_overlay_mode", args: { mode: "locked" } },
    ]);
    await broadcast({ mode: "locked" });
    await expect(overlay.locator("[data-chrome]")).toHaveCount(0);
    await expect(main.getByTestId("overlay-arrange")).toHaveText("Arrange");

    // A fresh page for the window draws main's saved scene: the picture is
    // where it was dropped.
    await overlay.reload();
    await expect(picture).toHaveCount(1);
    await expect.poll(async () => (await picture.boundingBox())?.x).toBeCloseTo(dropped.x, 0);
    expect((await picture.boundingBox())!.y).toBeCloseTo(dropped.y, 0);
    const saved = await main.evaluate(() =>
      JSON.parse(localStorage.getItem("divora.overlayScene") ?? "null"),
    );
    expect(saved.elements).toHaveLength(2);
    expect(saved.elements[1]).toMatchObject({ type: "image", src: IMPORTED_ASSET_SAMPLE.id, name: "wave.gif" });

    expect(consoleErrors, consoleErrors.join("\n")).toEqual([]);
    expect(noise, noise.join("\n")).toEqual([]);
  });
});

// v1.62.0: the seam for text, both windows joined. Settings measures the
// words and main stores them; the overlay draws what main pushes and
// measures again for itself. A drag there while Settings' editor is open
// lands in main's scene and leaves the draft alone, and Edit keeps the
// dragged height and centre.
test.describe("main window and overlay together (v1.62)", () => {
  test("E4: a text added in Settings shows on the overlay; Edit changes it, and a drag meanwhile keeps the draft", async ({
    page: main,
    context,
    consoleErrors,
  }) => {
    await main.setViewportSize({ width: 1280, height: 720 });
    const overlay = await context.newPage();
    await installTauriMock(overlay);
    const noise = watchConsole(overlay);
    await linkWindows(main, overlay);
    /** The backend's status, to both windows. */
    const broadcast = async (partial: Partial<OverlayStatus>): Promise<void> => {
      const s = await emitOverlayStatus(main, partial);
      await emitOverlayStatus(overlay, s);
    };
    /** Main's saved text charms. */
    const savedTexts = (): Promise<Array<Record<string, unknown>>> =>
      main.evaluate(() =>
        (
          (JSON.parse(localStorage.getItem("divora.overlayScene") ?? "null")?.elements ?? []) as Array<
            Record<string, unknown>
          >
        ).filter((e) => e.type === "text"),
      );

    await main.goto("/");
    await expect(main.locator(".titlebar-wordmark")).toBeVisible();
    await main.locator(".sidebar-nav", { hasText: "Settings" }).click();
    await main.getByRole("switch", { name: "Show stream overlay window" }).click();
    await expect.poll(() => invokesOf(main, "open_overlay")).toHaveLength(1);
    await overlay.goto("/overlay.html?instance=1");
    await expect(overlay.locator('[data-charm][data-id="circle"] [data-circle]')).toBeVisible();
    await broadcast({ phase: "shown" });
    await expect.poll(() => invokesOf(overlay, "overlay_handshake")).toHaveLength(2);
    await broadcast({ phase: "painted", canArrange: true });

    // Add text…: typed in Settings, never on the overlay.
    await main.getByTestId("overlay-add-text").click();
    const editor = main.getByTestId("overlay-text-editor");
    await expect(editor).toHaveAttribute("aria-label", "New text");
    const field = editor.getByRole("textbox", { name: "Overlay text" });
    await expect(field).toBeFocused();
    await field.fill("!discord in chat");
    await expect(editor.getByTestId("overlay-text-count")).toHaveText("16 of 200");
    await editor.getByRole("radio", { name: "Yellow" }).click();
    await editor.getByTestId("overlay-text-save").click();
    await expect(main.getByTestId("toast")).toHaveText("Text added. Use Arrange to place it.");
    await expect(editor).toHaveCount(0);
    await expect(main.getByTestId("overlay-add-text")).toBeFocused();
    await expect(main.getByTestId("overlay-charms")).toContainText("Text · !discord in chat");
    const [added] = await savedTexts();
    expect(added).toMatchObject({
      type: "text",
      text: "!discord in chat",
      font: "sans",
      bold: true,
      color: "#ffd60a",
      outline: true,
      align: "center",
    });
    // Measured in main, in whole pixels.
    expect(Number.isInteger(added!.nw) && (added!.nw as number) > 1).toBe(true);
    expect(Number.isInteger(added!.nh) && (added!.nh as number) > 1).toBe(true);
    const id = added!.id as string;

    // The overlay draws it, in its box of the text's shape.
    const text = overlay.locator(`[data-charm][data-id="${id}"]`);
    await expect(text).toHaveAttribute("aria-label", "Text: !discord in chat");
    const face = text.locator("[data-text-face]");
    await expect(face).toHaveText("!discord in chat");
    await expect(face).toHaveCSS("color", "rgb(255, 214, 10)");
    await expect(face).toHaveAttribute("data-fit-backstop", "0");
    const box = (await text.boundingBox())!;
    expect(box.width / box.height).toBeCloseTo((added!.nw as number) / (added!.nh as number), 2);

    // Arrange from Settings; the backend tells the overlay.
    await main.getByTestId("overlay-arrange").click();
    await expect(main.getByTestId("overlay-arrange")).toHaveText("Lock");
    await broadcast({});
    await expect(overlay.locator("[data-chrome]")).toBeVisible();

    // Edit opens the same editor with the text, and marks its row.
    await main.getByRole("button", { name: "Edit !discord in chat" }).click();
    await expect(editor).toHaveAttribute("aria-label", "Edit text");
    await expect(field).toHaveValue("!discord in chat");
    await expect(field).toBeFocused();
    await expect(main.locator(`[data-charm-row="${id}"]`)).toContainText("Editing");
    await field.fill("!discord · !socials");
    await expect(editor.getByTestId("overlay-text-count")).toHaveText("19 of 200");
    // The caret mid-text, and the field marked, so a field made again (its
    // focus and caret set afresh) can't pass for this one.
    await field.evaluate((el: HTMLTextAreaElement) => {
      el.setSelectionRange(9, 9);
      (el as unknown as { e2eMark?: boolean }).e2eMark = true;
    });
    const caret = () =>
      main.evaluate(() => {
        const el = document.activeElement as HTMLTextAreaElement | null;
        return {
          name: el?.getAttribute("aria-label") ?? null,
          at: el?.selectionStart ?? null,
          same: (el as unknown as { e2eMark?: boolean } | null)?.e2eMark === true,
        };
      });
    const before = await caret();
    expect(before).toEqual({ name: "Overlay text", at: 9, same: true });

    // A drag of that text on the overlay meanwhile: main saves it and acks.
    const from = center((await text.locator("[data-hit]").boundingBox())!);
    await overlay.mouse.move(from.x, from.y);
    await overlay.mouse.down();
    for (let i = 1; i <= 10; i++) await overlay.mouse.move(from.x - 4 * i, from.y + 3 * i);
    await overlay.mouse.up();
    await expect.poll(async () => (await emitsOf(overlay, "overlay:edit")).length).toBe(1);
    await expect
      .poll(async () => ((await emitsOf(main, "overlay:scene")).at(-1) as { ack: unknown }).ack)
      .toEqual({ instance: 1, seq: 0 });
    const [dragged] = await savedTexts();
    expect(dragged).toMatchObject({ id, text: "!discord in chat" });
    expect(dragged!.y).not.toBe(added!.y);
    // The draft, its caret and the focus are as they were.
    await expect(editor).toHaveAttribute("aria-label", "Edit text");
    await expect(field).toHaveValue("!discord · !socials");
    expect(await caret()).toEqual(before);

    // Save: the overlay shows the new words, keeping the dragged height and
    // (centred text) its centre; same id, same place in the stack.
    await editor.getByTestId("overlay-text-save").click();
    await expect(face).toHaveText("!discord · !socials");
    await expect(text).toHaveAttribute("aria-label", "Text: !discord · !socials");
    await expect(editor).toHaveCount(0);
    await expect(main.locator(`[data-charm-edit="${id}"]`)).toBeFocused();
    await expect(main.locator(`[data-charm-row="${id}"]`)).toContainText("Text · !discord · !socials");
    await expect(main.locator(`[data-charm-row="${id}"]`)).not.toContainText("Editing");
    const [edited] = await savedTexts();
    expect(edited).toMatchObject({ id, z: dragged!.z, text: "!discord · !socials", color: "#ffd60a" });
    const n = (e: Record<string, unknown>, k: string) => e[k] as number;
    expect(n(edited!, "nw")).toBeGreaterThan(n(dragged!, "nw"));
    expect(Math.abs(n(edited!, "h") - n(dragged!, "h"))).toBeLessThanOrEqual(1e-6);
    expect(Math.abs(n(edited!, "y") - n(dragged!, "y"))).toBeLessThanOrEqual(1e-6);
    expect(
      Math.abs(n(edited!, "x") + n(edited!, "w") / 2 - (n(dragged!, "x") + n(dragged!, "w") / 2)),
    ).toBeLessThanOrEqual(1e-6);
    await expect(face).toHaveAttribute("data-fit-backstop", "0");
    const shown = (await text.boundingBox())!;

    // Lock, then a fresh page for the window: main's saved scene, the new
    // words where they were.
    await overlay.keyboard.press("Escape");
    await expect.poll(() => invokesOf(overlay, "set_overlay_mode")).toHaveLength(1);
    await broadcast({ mode: "locked" });
    await expect(overlay.locator("[data-chrome]")).toHaveCount(0);
    await overlay.reload();
    await expect(face).toHaveText("!discord · !socials");
    await expect.poll(async () => (await text.boundingBox())?.x).toBeCloseTo(shown.x, 0);
    const again = (await text.boundingBox())!;
    expect(again.y).toBeCloseTo(shown.y, 0);
    expect(again.width).toBeCloseTo(shown.width, 0);
    expect(again.height).toBeCloseTo(shown.height, 0);

    expect(consoleErrors, consoleErrors.join("\n")).toEqual([]);
    expect(noise, noise.join("\n")).toEqual([]);
  });
});
