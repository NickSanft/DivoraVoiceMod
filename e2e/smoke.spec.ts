// E2E smoke tests — the whole SolidJS app wired together, Tauri mocked.
// Covers the core UI flows that docs/MANUAL_TESTS.md otherwise carries
// entirely by hand. See e2e/tauri-mock.ts for the backend stub.

import { test, expect } from "./tauri-mock";

const nav = (page: import("@playwright/test").Page, label: string) =>
  page.locator(".sidebar-nav", { hasText: label });
const seg = (page: import("@playwright/test").Page, label: string) =>
  page.locator(".seg-btn", { hasText: label });

test.beforeEach(async ({ page }) => {
  await page.goto("/");
  // The app shell is up once the titlebar wordmark renders.
  await expect(page.locator(".titlebar-wordmark")).toBeVisible();
});

test.describe("app shell", () => {
  test("boots cleanly with no console errors", async ({ page, consoleErrors }) => {
    await expect(page.locator(".titlebar-wordmark")).toHaveText(/DivoraVoice/i);
    await expect(page.locator(".sidebar-nav")).toHaveCount(5);
    expect(consoleErrors, consoleErrors.join("\n")).toEqual([]);
  });

  test("nav rail switches between all screens", async ({ page }) => {
    await nav(page, "Settings").click();
    await expect(page.getByText(/Devices, virtual mic/i)).toBeVisible();
    await expect(nav(page, "Settings")).toHaveAttribute("aria-current", "page");

    await nav(page, "Board").click();
    await expect(page.getByText(/No folder picked yet/i)).toBeVisible();

    await nav(page, "Presets").click();
    await expect(nav(page, "Presets")).toHaveAttribute("aria-current", "page");

    await nav(page, "Coven").click();
    await expect(nav(page, "Coven")).toHaveAttribute("aria-current", "page");

    await nav(page, "Mixer").click();
    await expect(nav(page, "Mixer")).toHaveAttribute("aria-current", "page");
  });
});

test.describe("light theme (v1.11)", () => {
  test("toggles to light, persists across reload, composes with mood", async ({
    page,
  }) => {
    const html = page.locator("html");
    await nav(page, "Settings").click();
    // Dark is the default (no data-theme attribute).
    await expect(html).not.toHaveAttribute("data-theme", "light");

    await seg(page, "Light").click();
    await expect(html).toHaveAttribute("data-theme", "light");

    // Persists across a reload (written to divora.tweaks).
    await page.reload();
    await expect(html).toHaveAttribute("data-theme", "light");

    // The light axis composes with a Color mood (specificity layering).
    await nav(page, "Settings").click();
    await seg(page, "Ink + Candle").click();
    await expect(html).toHaveAttribute("data-mood", "ink");
    await expect(html).toHaveAttribute("data-theme", "light");

    // Back to dark removes the attribute.
    await seg(page, "Dark").click();
    await expect(html).not.toHaveAttribute("data-theme", "light");
  });
});

test.describe("presets — preview vs Use (v1.6)", () => {
  test("previewing doesn't change the live voice; Use does", async ({ page }) => {
    // Default active preset is the first bundled one (Hollow King).
    await expect(page.locator(".titlebar-status")).toContainText(/Clean|Modulated|Muted/i);

    await nav(page, "Presets").click();
    // Preview Static Wraith (not the active preset).
    await page.getByText("Static Wraith", { exact: true }).first().click();

    // Active voice is unchanged — the Mixer header still shows Hollow King.
    await nav(page, "Mixer").click();
    await expect(page.getByText("Hollow King").first()).toBeVisible();

    // Now apply it via Use.
    await nav(page, "Presets").click();
    await page.getByText("Static Wraith", { exact: true }).first().click();
    await page.getByRole("button", { name: /^Use$/ }).click();

    // The active voice is now Static Wraith.
    await nav(page, "Mixer").click();
    await expect(page.getByText("Static Wraith").first()).toBeVisible();
  });
});

test.describe("glyph casting", () => {
  test("drawing a triangle on the Mixer casts the bound preset", async ({
    page,
  }) => {
    // Default binding: triangle → Velvet Demon (not the active Hollow King).
    await expect(page.getByText("Hollow King").first()).toBeVisible();

    // Trace a large triangle over the Mixer's empty lower area. The cast
    // overlay captures pointer drags that start on empty space.
    const box = await page.locator("#root").boundingBox();
    if (!box) throw new Error("no root box");
    const cx = box.x + box.width / 2;
    const top = box.y + box.height * 0.3;
    const bottomY = box.y + box.height * 0.72;
    const left = cx - box.width * 0.22;
    const right = cx + box.width * 0.22;

    const path: Array<[number, number]> = [];
    const lerp = (
      a: [number, number],
      b: [number, number],
      steps: number,
    ): void => {
      for (let i = 1; i <= steps; i++) {
        path.push([
          a[0] + ((b[0] - a[0]) * i) / steps,
          a[1] + ((b[1] - a[1]) * i) / steps,
        ]);
      }
    };
    const apex: [number, number] = [cx, top];
    const bl: [number, number] = [left, bottomY];
    const br: [number, number] = [right, bottomY];
    path.push(apex);
    lerp(apex, bl, 8);
    lerp(bl, br, 8);
    lerp(br, apex, 8);

    await page.mouse.move(path[0]![0], path[0]![1]);
    await page.mouse.down();
    for (const [x, y] of path) await page.mouse.move(x, y, { steps: 2 });
    await page.mouse.up();

    // A recognised triangle switches the active preset to Velvet Demon.
    await expect(page.getByText("Velvet Demon").first()).toBeVisible({
      timeout: 8000,
    });
  });
});

test.describe("in-app update check (v1.12)", () => {
  test("Settings → About shows the Updates control; no network in-browser", async ({
    page,
  }) => {
    await nav(page, "Settings").click();
    await expect(page.getByText("Updates", { exact: true })).toBeVisible();
    await expect(page.getByText(/no account, no telemetry/i)).toBeVisible();
    await expect(
      page.getByRole("button", { name: /Check now/i }),
    ).toBeVisible();
    // Off Tauri there's no version → no fetch → no "update available" card.
    await expect(page.getByText(/is available/i)).toHaveCount(0);
  });
});

test.describe("setup diagnostic (v1.13)", () => {
  test("'Test my setup' runs and renders a results checklist", async ({
    page,
  }) => {
    await nav(page, "Settings").click();
    await expect(
      page.getByText(/Check your mic, output, engine/i),
    ).toBeVisible();
    await page.getByRole("button", { name: /Test my setup/i }).click();
    // The engine briefly starts; results then render. VB-Cable + engine
    // rows are always present.
    await expect(page.getByText("VB-Cable installed")).toBeVisible({
      timeout: 12_000,
    });
    await expect(page.getByText(/Audio engine/)).toBeVisible();
  });
});

test.describe("quick wins (v1.14)", () => {
  test("Presets has an Import button; About has a Report-a-problem button", async ({
    page,
  }) => {
    await nav(page, "Presets").click();
    await expect(
      page.getByRole("button", { name: /Import preset from JSON/i }),
    ).toBeVisible();

    await nav(page, "Settings").click();
    await expect(
      page.getByRole("button", { name: /Report a problem/i }),
    ).toBeVisible();
  });
});

test.describe("custom glyph casting (v1.15)", () => {
  test("Settings glyph section has action pickers + a Record button", async ({
    page,
  }) => {
    await nav(page, "Settings").click();
    await expect(page.getByText("Custom glyphs")).toBeVisible();
    await expect(
      page.getByRole("button", { name: /Record a glyph/i }),
    ).toBeVisible();
    // Opening the recorder shows the draw pad.
    await page.getByRole("button", { name: /Record a glyph/i }).click();
    await expect(page.getByText(/Draw a single stroke/i)).toBeVisible();
  });
});

test.describe("first-run wizard (v0.7)", () => {
  test.use({ skipWizard: false });

  test("appears on first run and completes", async ({ page }) => {
    await expect(page.getByText(/transmuted in real time/i)).toBeVisible();

    // Click Continue through the steps, then finish.
    for (let i = 0; i < 5; i++) {
      const cont = page.getByRole("button", { name: "Continue" });
      if (await cont.isVisible().catch(() => false)) {
        await cont.click();
      } else {
        break;
      }
    }
    await page.getByRole("button", { name: /Enter Divora/i }).click();

    // Wizard dismissed — the Mixer shell is interactive.
    await expect(page.getByText(/transmuted in real time/i)).toBeHidden();
    await expect(page.locator(".sidebar-nav")).toHaveCount(5);
  });
});
