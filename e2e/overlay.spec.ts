// E2E for the stream overlay's own page (v1.55.0 Charms), Tauri mocked (see
// e2e/tauri-mock.ts): the ready and painted handshakes, Arrange's chrome and
// its beats, one edit per drag, real hit-testing, Lock inside the viewport,
// and Lock ending the beats (design §10.3). Main's side is in smoke.spec.ts;
// the last test joins the two windows.
//
// The mock never sends a status on its own: each step sends the one it
// needs with emitOverlayStatus, so each assertion is the page's answer to it.
// v1.59.0: the same for Hourglass's timers, with emitTimers.

import type { Page } from "@playwright/test";
import {
  test,
  expect,
  emitOverlayStatus,
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
import type { OverlayStatus } from "../src/overlay/status";
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
    const row = page.locator('[data-charm][data-id="pic-1"]');
    const hit = row.locator("[data-hit]");
    const before = (await row.boundingBox())!;
    expect(before).toMatchObject({ x: 264, y: 336, width: 144, height: 72 });
    const from = center((await hit.boundingBox())!);
    await page.mouse.move(from.x, from.y);
    await page.mouse.down();
    for (let i = 1; i <= 10; i++) await page.mouse.move(from.x - 10 * i, from.y - 15 * i);
    // The preview followed the pointer, and nothing was sent while it did.
    await expect.poll(async () => (await row.boundingBox())?.x).toBeCloseTo(164, 0);
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
    expect(first!.op.rect.x).toBeCloseTo(164, 3);
    expect(first!.op.rect.y).toBeCloseTo(186, 3);
    expect(first!.op.rect.w).toBeCloseTo(144, 3);
    expect(first!.op.rect.h).toBeCloseTo(72, 3);
    // It stays where it was dropped, and stays one edit.
    await page.waitForTimeout(400);
    expect(await edits(page)).toHaveLength(1);
    expect((await row.boundingBox())!.x).toBeCloseTo(164, 0);

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
    // with a push that acks it.
    const from = center((await picture.locator("[data-hit]").boundingBox())!);
    await overlay.mouse.move(from.x, from.y);
    await overlay.mouse.down();
    for (let i = 1; i <= 10; i++) await overlay.mouse.move(from.x - 4 * i, from.y - 3 * i);
    await overlay.mouse.up();
    await expect.poll(async () => (await emitsOf(overlay, "overlay:edit")).length).toBe(1);
    await expect
      .poll(async () => ((await emitsOf(main, "overlay:scene")).at(-1) as { ack: unknown }).ack)
      .toEqual({ instance: 1, seq: 0 });
    const dropped = (await picture.boundingBox())!;
    expect(dropped.x).toBeCloseTo(box.x - 40, 0);
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
