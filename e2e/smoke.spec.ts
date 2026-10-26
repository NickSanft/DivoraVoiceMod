// E2E smoke tests — the whole SolidJS app wired together, Tauri mocked.
// Covers the core UI flows that docs/MANUAL_TESTS.md otherwise carries
// entirely by hand. See e2e/tauri-mock.ts for the backend stub.

import {
  test,
  expect,
  delayPtmModeAnswers,
  emitLevels,
  emitOverlayStatus,
  emitReplay,
  emitReplaySaved,
  emitsOf,
  invokesOf,
  mockMute,
  mockPtm,
  pressMuteHotkey,
  pressPtmHotkey,
} from "./tauri-mock";

const nav = (page: import("@playwright/test").Page, label: string) =>
  page.locator(".sidebar-nav", { hasText: label });
const seg = (page: import("@playwright/test").Page, label: string) =>
  page.locator(".seg-btn", { hasText: label });

/** Trace a large triangle over the Mixer's empty lower area, as a user casts
 *  a glyph. The cast overlay captures pointer drags that start on empty
 *  space. */
async function traceTriangle(page: import("@playwright/test").Page): Promise<void> {
  const box = await page.locator("#root").boundingBox();
  if (!box) throw new Error("no root box");
  const cx = box.x + box.width / 2;
  const top = box.y + box.height * 0.3;
  const bottomY = box.y + box.height * 0.72;
  const left = cx - box.width * 0.22;
  const right = cx + box.width * 0.22;

  const path: Array<[number, number]> = [];
  const lerp = (a: [number, number], b: [number, number], steps: number): void => {
    for (let i = 1; i <= steps; i++) {
      path.push([a[0] + ((b[0] - a[0]) * i) / steps, a[1] + ((b[1] - a[1]) * i) / steps]);
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
}

test.beforeEach(async ({ page }) => {
  await page.goto("/");
  // The app shell is up once the titlebar wordmark renders.
  await expect(page.locator(".titlebar-wordmark")).toBeVisible();
});

test.describe("app shell", () => {
  test("boots cleanly with no console errors", async ({ page, consoleErrors }) => {
    await expect(page.locator(".titlebar-wordmark")).toHaveText(/DivoraVoice/i);
    await expect(page.locator(".sidebar-nav")).toHaveCount(6);
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

    await traceTriangle(page);

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

test.describe("what's new (v1.49)", () => {
  test("About opens release notes with no network at all", async ({ page }) => {
    // Fail any outbound request: the notes are compiled into the bundle,
    // so the panel must render fully offline. If this ever starts fetching,
    // this test is what catches it.
    await page.route("**/*", (route) =>
      route.request().url().startsWith("http://localhost")
        ? route.continue()
        : route.abort(),
    );

    await nav(page, "Settings").click();
    await page.getByRole("button", { name: /What.s new/i }).click();

    const dialog = page.getByRole("dialog");
    await expect(dialog).toBeVisible();
    // Newest release, its headline, and formatted (not raw) markdown.
    await expect(dialog.getByRole("heading", { level: 3 }).first()).toBeVisible();
    await expect(dialog).not.toContainText("**");
    await expect(
      dialog.getByRole("button", { name: /Full changelog/i }),
    ).toBeVisible();

    // No banner off Tauri: there is no app version to compare against. (The
    // shell's toast is a status region too, always mounted since v1.54.0, and
    // so is the titlebar's mute announcer since v1.56.0; both are empty.)
    await expect(
      page.locator(
        '[role="status"]:not([data-testid="toast"]):not([data-testid="mute-announcer"])',
      ),
    ).toHaveCount(0);
    await expect(page.getByTestId("toast")).toHaveText("");
    await expect(page.getByTestId("mute-announcer")).toHaveText("");

    await page.keyboard.press("Escape");
    await expect(dialog).toHaveCount(0);
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

test.describe("stream overlay (v1.16)", () => {
  test("Settings has the Stream overlay section + OBS guidance", async ({
    page,
  }) => {
    await nav(page, "Settings").click();
    await expect(page.getByText("Stream overlay")).toBeVisible();
    await expect(page.getByText(/Show overlay window/i)).toBeVisible();
    await expect(page.getByText(/capture it in OBS/i)).toBeVisible();
  });

  // v1.55.0: the main window's side of Charms, against the mock backend. The
  // mock can't read monitors (availableMonitors answers null), so the
  // overlay opens unplaced (rect null) and Fit can't turn on.
  test("Stream overlay controls render with monitors → null", async ({ page, consoleErrors }) => {
    await nav(page, "Settings").click();

    // Boot: main read the backend's status and pushed the stored Hide flag.
    await expect.poll(() => invokesOf(page, "overlay_status")).toHaveLength(1);
    expect((await invokesOf(page, "set_overlay_hide_from_recordings")).map((i) => i.args)).toEqual([
      { enabled: false },
    ]);

    // The rows. Closed, Arrange is disabled; the backend says Hide works here.
    const arrange = page.getByTestId("overlay-arrange");
    await expect(arrange).toBeVisible();
    await expect(arrange).toBeDisabled();
    await expect(arrange).toHaveAttribute("aria-label", "Arrange the overlay");
    const fit = page.getByRole("switch", { name: "Fit the overlay to its monitor" });
    await expect(fit).toBeVisible();
    await expect(fit).toHaveAttribute("aria-checked", "false");
    await expect(page.getByRole("switch", { name: "Hide the overlay from recordings" })).toBeVisible();
    const charms = page.getByTestId("overlay-charms");
    await expect(charms).toBeVisible();
    await expect(charms.locator('[data-charm-row="circle"]')).toContainText("Spell circle");
    await expect(page.getByTestId("overlay-add-picture")).toBeEnabled();

    // Fit with no readable monitors: it says so and stays off, and nothing
    // breaks on the way.
    await fit.click();
    await expect(page.getByTestId("toast")).toHaveText("Couldn't read your monitors.");
    await expect(fit).toHaveAttribute("aria-checked", "false");
    expect(await invokesOf(page, "place_overlay")).toEqual([]);
    expect(consoleErrors, consoleErrors.join("\n")).toEqual([]);

    // Show the overlay: no saved place and no monitors → Windows' default
    // spot, unfitted. The window starts hidden, so Arrange waits.
    const show = page.getByRole("switch", { name: "Show stream overlay window" });
    await show.click();
    await expect.poll(() => invokesOf(page, "open_overlay")).toEqual([
      { cmd: "open_overlay", args: { rect: null, fit: false } },
    ]);
    await expect(show).toHaveAttribute("aria-checked", "true");
    await expect(arrange).toHaveText("Starting…");
    await expect(arrange).toBeDisabled();
    // The migrated scene is saved the first time the overlay is seen open,
    // and pushed to it.
    expect(
      await page.evaluate(() => JSON.parse(localStorage.getItem("divora.overlayScene") ?? "null")),
    ).toMatchObject({ version: 1, frame: "window", elements: [{ id: "circle", type: "spellcircle" }] });
    await expect
      .poll(async () => (await emitsOf(page, "overlay:scene")).length)
      .toBeGreaterThanOrEqual(1);

    // Still starting while nothing else is heard; the shown status alone
    // doesn't allow Arrange either.
    await page.waitForTimeout(300);
    await expect(arrange).toHaveText("Starting…");
    await emitOverlayStatus(page, { phase: "shown" });
    await expect(arrange).toHaveText("Starting…");
    await expect(arrange).toBeDisabled();

    // Painted: Arrange is offered.
    await emitOverlayStatus(page, { phase: "painted", canArrange: true });
    await expect(arrange).toHaveText("Arrange");
    await expect(arrange).toBeEnabled();
    await expect(arrange).toHaveAttribute("aria-label", "Arrange the overlay");

    await arrange.click();
    await expect.poll(() => invokesOf(page, "set_overlay_mode")).toEqual([
      { cmd: "set_overlay_mode", args: { mode: "arrange" } },
    ]);
    // The backend's answer puts the button on Lock.
    await expect(arrange).toHaveText("Lock");
    await expect(arrange).toHaveAttribute("aria-label", "Lock the overlay");

    expect(consoleErrors, consoleErrors.join("\n")).toEqual([]);
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

test.describe("speak / text-to-speech (v1.17)", () => {
  test("Speak screen renders text box, voice picker, and the scaffolding notice", async ({
    page,
  }) => {
    await nav(page, "Speak").click();
    await expect(nav(page, "Speak")).toHaveAttribute("aria-current", "page");

    await expect(page.getByRole("heading", { name: "Speak" })).toBeVisible();
    await expect(page.getByPlaceholder(/echoes/i)).toBeVisible();

    // Preset voices come from the list_tts_voices mock.
    await expect(page.getByRole("radio", { name: /Aria/i })).toBeVisible();
    await expect(page.getByRole("radio", { name: /George/i })).toBeVisible();

    // Scaffolding notice: preset voices aren't installed in this build.
    await expect(page.getByText(/aren't installed/i)).toBeVisible();

    // The Speak action button (scope to the primary btn to avoid the
    // sidebar's "Speak" nav button).
    await expect(
      page.locator("button.btn-primary", { hasText: "Speak" }),
    ).toBeVisible();
  });

  test("typing then Speak surfaces the graceful 'not installed' message", async ({
    page,
  }) => {
    await nav(page, "Speak").click();
    await page.getByPlaceholder(/echoes/i).fill("Hello from the coven");
    await page.locator("button.btn-primary", { hasText: "Speak" }).click();
    await expect(page.getByText(/voices are not installed/i)).toBeVisible();
  });

  test("a Critter Chatter voice speaks with no error, even with presets missing (v1.50)", async ({
    page,
    consoleErrors,
  }) => {
    await nav(page, "Speak").click();

    const critters = page.getByRole("radiogroup", { name: "Critter Chatter" });
    await expect(critters.getByRole("radio")).toHaveCount(3);
    const gruff = critters.getByRole("radio", { name: /Gruff/ });
    await gruff.click();
    await expect(gruff).toHaveAttribute("aria-checked", "true");

    await page.getByPlaceholder(/echoes/i).fill("Hello from the burrow!");
    await page.locator("button.btn-primary", { hasText: "Speak" }).click();

    // The mock renders only babble ids, so the right id reaching speak — and
    // no error surfacing — is what proves the selection is wired through.
    await expect
      .poll(() =>
        page.evaluate(() =>
          (
            window as unknown as {
              __E2E_INVOKES__: Array<{ cmd: string; args: { voiceId?: string } }>;
            }
          ).__E2E_INVOKES__
            .filter((i) => i.cmd === "speak")
            .map((i) => i.args.voiceId),
        ),
      )
      .toEqual(["babble:gruff"]);
    await expect(
      page.locator("button.btn-primary", { hasText: "Speak" }),
    ).toBeEnabled();
    await expect(page.getByRole("alert")).toHaveCount(0);
    expect(consoleErrors, consoleErrors.join("\n")).toEqual([]);
  });
});

test.describe("reactive effects (v1.46)", () => {
  test("card renders, toggles, and warns when the preset has nothing to drive", async ({
    page,
  }) => {
    await nav(page, "Mixer").click();

    const toggle = page.getByRole("switch", { name: "Reactive effects" });
    await expect(toggle).toBeVisible();
    await expect(page.getByText(/Raise your voice, harden the character/i)).toBeVisible();

    // Off by default, so an untouched install behaves exactly as before.
    await expect(toggle).toHaveAttribute("aria-checked", "false");

    await toggle.click();
    await expect(toggle).toHaveAttribute("aria-checked", "true");

    // The seeded preset (Hollow King) has no Distortion, so enabling it would
    // do nothing audible — the card has to say so rather than look broken.
    await expect(page.getByText(/no Distortion to drive/i)).toBeVisible();

    // Making a preset that DOES use distortion active clears the warning.
    // Note this needs "Use", not just a preview click: the modulation targets
    // the LIVE chain, so the warning must track the active voice.
    await nav(page, "Presets").click();
    await page.getByText("Static Wraith", { exact: true }).first().click();
    await page.getByRole("button", { name: /^Use$/ }).click();
    await nav(page, "Mixer").click();
    await expect(page.getByText(/no Distortion to drive/i)).toHaveCount(0);
  });
});

test.describe("voice reading", () => {
  test("card renders, is off by default, and toggles on a live reading", async ({
    page,
    consoleErrors,
  }) => {
    await nav(page, "Mixer").click();

    const toggle = page.getByRole("switch", { name: "Voice reading" });
    await expect(toggle).toBeVisible();
    // Off by default: an untouched install analyzes nothing.
    await expect(toggle).toHaveAttribute("aria-checked", "false");
    await expect(page.getByTestId("reading-dry")).toHaveCount(0);

    await toggle.click();
    await expect(toggle).toHaveAttribute("aria-checked", "true");
    expect(
      await page.evaluate(() =>
        (
          window as unknown as {
            __E2E_INVOKES__: Array<{ cmd: string; args: unknown }>;
          }
        ).__E2E_INVOKES__.some(
          (i) =>
            i.cmd === "set_voice_reading_enabled" &&
            (i.args as { enabled: boolean }).enabled === true,
        ),
      ),
    ).toBe(true);

    // A speaking window arrives: both halves render, clearly separated, with
    // the after-effects half labelled as the chain's output.
    await page.evaluate(() => {
      const m = {
        energyDbfs: -17.4,
        energyRangeDb: 9.2,
        f0Hz: 118,
        f0RangeSt: 7.3,
        voicedRatio: 0.62,
        paceOps: 4.1,
        brightnessHz: 2280,
      };
      (
        window as unknown as {
          __E2E_EMIT__: (e: string, p: unknown) => void;
        }
      ).__E2E_EMIT__("voice-reading", {
        state: "speaking",
        dry: m,
        wet: { ...m, f0Hz: 84, brightnessHz: 1450 },
        words: ["bright", "wide range", "fast"],
        calibrated: true,
        stale: false,
        ageMs: 0,
      });
    });

    await expect(page.getByTestId("reading-phrase")).toHaveText(
      "bright, wide range, fast",
    );
    await expect(page.getByTestId("reading-dry")).toContainText("118 Hz");
    await expect(page.getByTestId("reading-wet")).toContainText("84 Hz");
    await expect(page.getByTestId("reading-wet")).toContainText(
      /chain's output/i,
    );
    await expect(page.getByTestId("reading-disclaimer")).toContainText(
      /not a reading of the person/i,
    );

    // A pause holds that window and says so rather than decaying.
    await page.evaluate(() => {
      const m = {
        energyDbfs: -17.4,
        energyRangeDb: 9.2,
        f0Hz: 118,
        f0RangeSt: 7.3,
        voicedRatio: 0.62,
        paceOps: 4.1,
        brightnessHz: 2280,
      };
      (
        window as unknown as {
          __E2E_EMIT__: (e: string, p: unknown) => void;
        }
      ).__E2E_EMIT__("voice-reading", {
        state: "quiet",
        dry: m,
        wet: { ...m, f0Hz: 84, brightnessHz: 1450 },
        words: ["bright", "wide range", "fast"],
        calibrated: true,
        stale: true,
        ageMs: 3400,
      });
    });
    await expect(page.getByTestId("reading-state")).toContainText(/paused/i);
    await expect(page.getByTestId("reading-state")).toContainText(/3\.4 s ago/i);
    await expect(page.getByTestId("reading-dry")).toContainText("118 Hz");

    await toggle.click();
    await expect(toggle).toHaveAttribute("aria-checked", "false");
    await expect(page.getByTestId("reading-dry")).toHaveCount(0);

    expect(consoleErrors, consoleErrors.join("\n")).toEqual([]);
  });

});

test.describe("screen recording (v1.52)", () => {
  /** Fire a `screen-recording` event as the backend would. */
  async function emitScreen(
    page: import("@playwright/test").Page,
    update: Record<string, unknown>,
  ): Promise<void> {
    await page.evaluate((u) => {
      (
        window as unknown as { __E2E_EMIT__: (e: string, p: unknown) => void }
      ).__E2E_EMIT__("screen-recording", {
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
        ...u,
      });
    }, update);
  }

  test("the Mixer card follows a take from the backend's events", async ({
    page,
    consoleErrors,
  }) => {
    await nav(page, "Mixer").click();
    const card = page.getByTestId("screen-record-card");
    await expect(card).toBeVisible();
    await expect(card).toContainText("Your modulated voice and soundboard");
    await expect(page.getByTestId("screen-record-title")).toHaveText("Record screen");
    // Two screens: the picker shows the main one.
    await expect(card).toContainText("Screen 1 · 2560×1440 · main");
    // The harness starts the engine, so Record is live and asks the backend
    // for a take of the chosen screen with the default settings.
    const toggle = page.getByTestId("screen-record-toggle");
    await expect(toggle).toBeEnabled();
    await toggle.click();
    await expect
      .poll(() =>
        page.evaluate(() =>
          (
            window as unknown as {
              __E2E_INVOKES__: Array<{ cmd: string; args: Record<string, unknown> }>;
            }
          ).__E2E_INVOKES__.find((i) => i.cmd === "start_screen_recording")?.args,
        ),
      )
      .toMatchObject({
        monitorId: "\\\\.\\DISPLAY1",
        fps: 30,
        quality: "balanced",
        cursor: true,
      });

    await emitScreen(page, {
      state: "recording",
      path: "C:/mock/recordings/divora-screen-x.mp4",
      elapsedMs: 42_000,
    });
    await expect(page.getByTestId("screen-record-title")).toHaveText("Recording · 0:42");
    await expect(toggle).toHaveText("Stop");
    await expect(toggle).toBeEnabled();

    await emitScreen(page, {
      state: "recording",
      path: "C:/mock/recordings/divora-screen-x.mp4",
      elapsedMs: 90_000,
      audioLostAt: 83.4,
    });
    await expect(page.getByTestId("screen-record-audio-lost")).toContainText(
      "Audio lost at 1:23",
    );

    await emitScreen(page, { state: "stopping", path: "C:/mock/recordings/divora-screen-x.mp4" });
    await expect(page.getByTestId("screen-record-title")).toHaveText("Finishing the file…");
    await expect(toggle).toBeDisabled();

    await emitScreen(page, { state: "idle", path: "C:/mock/recordings/divora-screen-x.mp4" });
    await expect(card).toContainText("Saved divora-screen-x.mp4");

    await emitScreen(page, { state: "idle", error: "That screen isn't sending any picture" });
    await expect(page.getByTestId("screen-record-error")).toHaveText(
      "That screen isn't sending any picture",
    );

    expect(consoleErrors, consoleErrors.join("\n")).toEqual([]);
  });

  test("v1.53: desktop audio is picked on the card, asked for, and reported on", async ({
    page,
    consoleErrors,
  }) => {
    await nav(page, "Mixer").click();
    const card = page.getByTestId("screen-record-card");
    // Someone who used the app before v1.53.0 starts with it off.
    const picker = page.getByTestId("screen-record-desktop");
    await expect(picker).toContainText("No desktop audio");
    await picker.getByRole("button").click();
    await picker.getByRole("option", { name: /Everything except Divora/ }).click();
    await expect(page.getByTestId("screen-record-subtitle")).toHaveText(
      "Your voice and soundboard, plus sound from other apps",
    );
    expect(
      await page.evaluate(() => JSON.parse(localStorage.getItem("divora.screenRecording") ?? "{}")),
    ).toMatchObject({ desktopAudio: "everything" });

    await page.getByTestId("screen-record-toggle").click();
    await expect
      .poll(() =>
        page.evaluate(() =>
          (
            window as unknown as {
              __E2E_INVOKES__: Array<{ cmd: string; args: Record<string, unknown> }>;
            }
          ).__E2E_INVOKES__.find((i) => i.cmd === "start_screen_recording")?.args,
        ),
      )
      .toMatchObject({ desktopAudio: "everything", desktopGainDb: -6, stems: false });

    const running = {
      state: "recording",
      path: "C:/mock/recordings/divora-screen-x.mp4",
      elapsedMs: 15_000,
      desktopSourceId: "everything",
      desktopSource: "Everything except Divora",
    };
    await emitScreen(page, { ...running, desktopQuietSeconds: 3 });
    await expect(page.getByTestId("screen-record-desktop-quiet")).toHaveCount(0);
    await emitScreen(page, { ...running, desktopQuietSeconds: 11.2 });
    await expect(page.getByTestId("screen-record-desktop-quiet")).toContainText(
      "Nothing heard from other apps for 11 s",
    );
    await emitScreen(page, { ...running, desktopLostAt: 14.2 });
    await expect(page.getByTestId("screen-record-desktop-lost")).toContainText(
      "Desktop audio lost at 0:14",
    );
    await emitScreen(page, {
      ...running,
      desktopSourceId: "device:{x}",
      desktopSource: "Headphones",
      desktopDoubled: true,
    });
    await expect(page.getByTestId("screen-record-doubled")).toContainText(
      "set to play its sound on Headphones, so your voice or soundboard is recorded twice",
    );
    await expect(page.getByTestId("screen-record-subtitle")).toHaveText(
      "Your voice and soundboard, plus what Headphones plays",
    );
    // The engine stops: with desktop audio still going, only the voice and
    // soundboard are lost.
    await emitScreen(page, { ...running, audioLostAt: 83.4 });
    await expect(page.getByTestId("screen-record-audio-lost")).toContainText(
      "Your voice and soundboard stopped at 1:23",
    );
    await emitScreen(page, {
      state: "recording",
      path: "C:/mock/recordings/divora-screen-x.mp4",
      desktopError: "Desktop audio couldn't start: no answer",
    });
    await expect(page.getByTestId("screen-record-desktop-error")).toContainText(
      "Recording without desktop audio: Desktop audio couldn't start",
    );
    await expect(page.getByTestId("screen-record-subtitle")).toHaveText(
      "Your modulated voice and soundboard",
    );
    await expect(card).toBeVisible();
    expect(consoleErrors, consoleErrors.join("\n")).toEqual([]);
  });

  test("the rec:screen and rec:wav hotkeys start their recorders", async ({ page }) => {
    await nav(page, "Mixer").click();
    await expect(page.getByTestId("screen-record-toggle")).toBeEnabled();
    const invoked = (cmd: string) =>
      page.evaluate(
        (c) =>
          (
            window as unknown as { __E2E_INVOKES__: Array<{ cmd: string }> }
          ).__E2E_INVOKES__.some((i) => i.cmd === c),
        cmd,
      );
    const press = (id: string) =>
      page.evaluate((i) => {
        (
          window as unknown as { __E2E_EMIT__: (e: string, p: unknown) => void }
        ).__E2E_EMIT__("global-shortcut", { id: i, accelerator: "F9", state: "pressed" });
      }, id);
    await press("rec:screen");
    await expect.poll(() => invoked("start_screen_recording")).toBe(true);
    await press("rec:wav");
    await expect.poll(() => invoked("start_recording")).toBe(true);
  });

  test("Settings has the recorder's rows and its hotkeys, and they persist", async ({ page }) => {
    await nav(page, "Settings").click();
    const rows = page.getByTestId("screen-recording-settings");
    await expect(rows).toBeVisible();
    await expect(rows).toContainText("Mock H.264 Encoder");
    await rows.getByRole("tab", { name: "60 fps" }).click();
    await rows.getByRole("tab", { name: "Sharpest" }).click();
    expect(
      await page.evaluate(() => JSON.parse(localStorage.getItem("divora.screenRecording") ?? "{}")),
    ).toMatchObject({ fps: 60, quality: "high" });
    // v1.53.0: the desktop audio volume and the separate files.
    await expect(rows).toContainText("Desktop audio volume · -6 dB");
    await rows.getByRole("switch", { name: /separate WAV files/ }).click();
    expect(
      await page.evaluate(() => JSON.parse(localStorage.getItem("divora.screenRecording") ?? "{}")),
    ).toMatchObject({ stems: true });
    // The recorders' hotkey rows (v1.52.0).
    await expect(
      page.getByText("Start or stop recording a screen with your voice and soundboard."),
    ).toBeVisible();
    await expect(page.getByText("Record voice (WAV)", { exact: true })).toBeVisible();
  });
});

// Everything here goes by test id, never /Replay/: that also matches
// Settings → About's "Replay setup", which re-opens the first-run wizard.
test.describe("replay (v1.54)", () => {
  type Page = import("@playwright/test").Page;

  const MAIN_SCREEN = "\\\\.\\DISPLAY1";
  const ARMED = {
    state: "armed",
    seconds: 30,
    bufferedMs: 12_000,
    capBytes: 80_044_800,
    monitorId: MAIN_SCREEN,
    heldReason: null,
    armed: {
      fps: 30,
      quality: "balanced",
      cursor: true,
      desktopAudio: null,
      desktopGainDb: -6,
      avOffsetMs: 0,
    },
  };
  const GATE = "Turn Replay on from the Record screen card first — it explains what Replay keeps.";

  const argsOf = (page: Page, cmd: string) =>
    page.evaluate(
      (c) =>
        (
          window as unknown as {
            __E2E_INVOKES__: Array<{ cmd: string; args: Record<string, unknown> }>;
          }
        ).__E2E_INVOKES__
          .filter((i) => i.cmd === c)
          .map((i) => i.args ?? {}),
      cmd,
    );
  const press = (page: Page, id: string) =>
    page.evaluate((i) => {
      (
        window as unknown as { __E2E_EMIT__: (e: string, p: unknown) => void }
      ).__E2E_EMIT__("global-shortcut", { id: i, accelerator: "F10", state: "pressed" });
    }, id);
  const stored = (page: Page) =>
    page.evaluate(() => JSON.parse(localStorage.getItem("divora.replayBuffer") ?? "null"));
  /** As if the card's explainer had been read in an earlier session. */
  async function explainedBefore(page: Page): Promise<void> {
    await page.evaluate(() =>
      localStorage.setItem(
        "divora.replayBuffer",
        JSON.stringify({ seconds: 30, explained: true, explainedDesktop: false }),
      ),
    );
    await page.reload();
    await expect(page.locator(".titlebar-wordmark")).toBeVisible();
  }
  /** The harness starts the engine; Record going live says it has. */
  async function engineUp(page: Page): Promise<void> {
    await nav(page, "Mixer").click();
    await expect(page.getByTestId("screen-record-toggle")).toBeEnabled();
  }

  test("1: the card explains Replay before it can be turned on, and focus follows", async ({
    page,
    consoleErrors,
  }) => {
    await engineUp(page);
    const setup = page.getByTestId("replay-setup");
    await expect(setup).toBeVisible();
    await expect(setup).toHaveAttribute("aria-expanded", "false");
    await expect(setup).toHaveAttribute("aria-controls", "replay-explainer");
    // A disclosure button, not a switch that opens text.
    await expect(page.getByTestId("replay-toggle")).toHaveCount(0);

    await setup.click();
    await expect(setup).toHaveAttribute("aria-expanded", "true");
    const explainer = page.getByTestId("replay-explainer");
    await expect(explainer).toBeVisible();
    await expect(explainer).toHaveAttribute("role", "region");
    await expect(explainer.getByRole("heading")).toBeFocused();
    await expect(explainer).toContainText(
      "Divora writes no picture or sound to disk until you press Save.",
    );
    await expect(explainer).toContainText("calls included");
    await expect(explainer).toContainText("Desktop audio is off here.");
    await expect(explainer).toContainText("Divora leaves it on, so you can always tell");

    await explainer.getByRole("button", { name: "Not now" }).click();
    await expect(explainer).toHaveCount(0);
    await expect(setup).toBeFocused();
    await expect(setup).toHaveAttribute("aria-expanded", "false");

    await setup.click();
    await page
      .getByTestId("replay-explainer")
      .getByRole("button", { name: "Turn Replay on" })
      .click();
    await expect.poll(() => argsOf(page, "arm_replay_buffer")).toHaveLength(1);
    const [args] = await argsOf(page, "arm_replay_buffer");
    // The card's screen and Settings' rows, plus the length. Someone who used
    // the app before v1.53.0 has desktop audio off, so none is asked for.
    expect(args).toEqual({
      monitorId: MAIN_SCREEN,
      seconds: 30,
      fps: 30,
      quality: "balanced",
      cursor: true,
      avOffsetMs: 0,
      desktopGainDb: -6,
    });
    await expect(page.getByTestId("replay-setup")).toHaveCount(0);
    await expect(page.getByTestId("replay-toggle").getByRole("switch")).toHaveAttribute(
      "aria-checked",
      "true",
    );
    expect(await stored(page)).toEqual({ seconds: 30, explained: true, explainedDesktop: false });
    expect(consoleErrors, consoleErrors.join("\n")).toEqual([]);
  });

  test("2: armed shows the chip, the buffered seconds and the locked line", async ({ page }) => {
    await engineUp(page);
    const card = page.getByTestId("screen-record-card");
    await expect(card).toContainText("Screen 1 · 2560×1440 · main");
    await expect(page.getByTestId("titlebar-replay")).toHaveCount(0);

    await emitReplay(page, ARMED);
    const chip = page.getByTestId("titlebar-replay");
    await expect(chip).toBeVisible();
    await expect(chip).toHaveText(/REPLAY · 30 s/);
    await expect(chip).toHaveAttribute(
      "title",
      "Replay is keeping the last 30 s of Screen 1, your voice and soundboard in memory. Windows draws a yellow border around that screen.",
    );
    await expect(page.getByTestId("replay-save")).toHaveText("Save (0:12 so far)");
    // The pickers give way to the line that says what Replay keeps.
    await expect(card).not.toContainText("2560×1440");
    await expect(page.getByTestId("screen-record-desktop")).toHaveCount(0);
    await expect(page.getByTestId("replay-locked")).toHaveText(
      "Replay is keeping Screen 1, your voice and soundboard. Turn it off to change the screen or desktop audio.",
    );

    await emitReplay(page, {
      bufferedMs: 30_000,
      armed: { ...ARMED.armed, desktopAudio: "everything" },
    });
    await expect(page.getByTestId("replay-save")).toHaveText("Save the last 30 s");
    await expect(page.getByTestId("replay-locked")).toContainText(
      "your voice and soundboard, and sound from other apps (calls included)",
    );
    await expect(chip).toHaveAttribute("title", /calls included/);

    await emitReplay(page, { saving: true });
    await expect(page.getByTestId("replay-save")).toHaveText("Saving…");
    await expect(page.getByTestId("replay-save")).toBeDisabled();
  });

  test("3: a save hotkey while off says so", async ({ page }) => {
    await engineUp(page);
    await press(page, "replay:save");
    await expect(page.getByTestId("toast")).toHaveText("Replay is off.");
    expect(await argsOf(page, "save_replay")).toHaveLength(1);
    // Esc dismisses it.
    await page.keyboard.press("Escape");
    await expect(page.getByTestId("toast")).toHaveText("");
  });

  test("4: a save hotkey while armed saves", async ({ page }) => {
    await engineUp(page);
    await emitReplay(page, ARMED);
    await press(page, "replay:save");
    await expect.poll(() => argsOf(page, "save_replay")).toHaveLength(1);
    // Hollow King's effects are on from the start (push-to-modulate is off by
    // default, v1.58.0), but only since this page loaded: not for the whole
    // 12 s clip plus the save's own 3 s, so the preset isn't named.
    await expect(page.getByTestId("toast")).toHaveText(
      "Saved a 12 s replay · your voice as your call heard it",
    );
  });

  test("5: the save toast names the preset only while the whole clip was disguised", async ({
    page,
  }) => {
    // A triangle mutes, so the voice can be muted from the Mixer.
    await page.evaluate(() =>
      localStorage.setItem("divora.glyphBindings", JSON.stringify({ triangle: { kind: "mute" } })),
    );
    await page.reload();
    await engineUp(page);
    const saved = { path: "C:/mock/recordings/divora-replay-x.mp4", durationMs: 400 };
    const pill = page.getByTestId("titlebar-status");
    const toast = page.getByTestId("toast");
    const DISGUISED = "Saved a 1 s replay · Hollow King";
    const HEARD = "Saved a 1 s replay · your voice as your call heard it";
    /** A save, its toast cleared first, so the same text twice is two toasts. */
    const save = async (): Promise<void> => {
      await page.keyboard.press("Escape");
      await expect(toast).toHaveText("");
      await emitReplaySaved(page, saved);
    };

    // v1.58.0: push-to-modulate is off by default, so Hollow King's effects
    // are on from the start. The clip (and the save's own 3 s) must lie after
    // the voice turned.
    await expect(pill).toHaveText("Modulated");
    await page.waitForTimeout(4_000);
    await save();
    await expect(toast).toHaveText(DISGUISED);

    // Muted, the call hears none of it.
    await traceTriangle(page);
    await expect(pill).toHaveText("Muted");
    // v1.56.0: the glyph really mutes, through the engine.
    expect(await invokesOf(page, "set_input_muted")).toEqual([
      { cmd: "set_input_muted", args: { muted: true } },
    ]);
    await save();
    await expect(toast).toHaveText(HEARD);
    await traceTriangle(page);
    await expect(pill).toHaveText("Modulated");

    // Push-to-modulate on (Hold to apply) with the key up: the call hears
    // your own voice.
    await page.getByRole("switch", { name: "Push to modulate", exact: true }).click();
    await expect(pill).toHaveText("Clean");
    await save();
    await expect(toast).toHaveText(HEARD);

    // Held, the preset from the press on: a clip that starts before it isn't
    // named, one that lies after it is.
    await page.keyboard.down("Space");
    await expect(pill).toHaveText("Modulated");
    await save();
    await expect(toast).toHaveText(HEARD);
    await page.waitForTimeout(4_000);
    await save();
    await expect(toast).toHaveText(DISGUISED);
    await page.keyboard.up("Space");
    await expect(pill).toHaveText("Clean");
    expect(
      (await invokesOf(page)).filter((i) => i.cmd === "set_ptm_mode" || i.cmd === "set_ptm_held"),
    ).toEqual([
      { cmd: "set_ptm_mode", args: { mode: "always", restore: true } },
      { cmd: "set_ptm_mode", args: { mode: "apply", restore: false } },
      { cmd: "set_ptm_held", args: { source: "key", held: true } },
      { cmd: "set_ptm_held", args: { source: "key", held: false } },
    ]);
  });

  test("6: a held ring says why, and can still be saved", async ({ page }) => {
    await engineUp(page);
    const cases: Array<[string, RegExp]> = [
      ["engine-stopped", /^The audio engine stopped, so Replay stopped keeping\./],
      ["screen-lost", /^That screen was disconnected, so Replay stopped keeping\./],
      ["encoder-failed", /^The video encoder stopped working, so Replay stopped keeping\./],
    ];
    for (const [heldReason, copy] of cases) {
      await emitReplay(page, { ...ARMED, state: "held", heldReason });
      await expect(page.getByTestId("replay-held")).toHaveText(copy);
      await expect(page.getByTestId("replay-held")).toContainText("Save still saves what it kept");
      await expect(page.getByTestId("titlebar-replay")).toHaveText(/REPLAY · HELD/);
      await expect(page.getByTestId("replay-save")).toBeEnabled();
    }
    await emitReplay(page, { state: "armed", heldReason: null });
    await expect(page.getByTestId("replay-held")).toHaveCount(0);
  });

  test("7: the on/off hotkey is refused until the explainer, then toggles", async ({ page }) => {
    await engineUp(page);
    await press(page, "replay:toggle");
    await expect(page.getByTestId("toast")).toHaveText(GATE);
    expect(await argsOf(page, "arm_replay_buffer")).toEqual([]);

    await explainedBefore(page);
    await engineUp(page);
    await press(page, "replay:toggle");
    await expect.poll(() => argsOf(page, "arm_replay_buffer")).toHaveLength(1);
    await expect(page.getByTestId("titlebar-replay")).toBeVisible();
    await press(page, "replay:toggle");
    await expect.poll(() => argsOf(page, "disarm_replay_buffer")).toHaveLength(1);
    await expect(page.getByTestId("titlebar-replay")).toHaveCount(0);
  });

  test("8: Settings has Replay's rows; the length persists; screen rows lock while on", async ({
    page,
  }) => {
    await engineUp(page);
    await nav(page, "Settings").click();
    const rows = page.getByTestId("replay-settings");
    await expect(rows).toBeVisible();
    await expect(rows).toContainText("Replays are saved in the Recordings folder above.");
    await expect(rows.getByRole("tab", { name: /^30 s · up to 81 MB$/ })).toBeVisible();
    await rows.getByRole("tab", { name: /^60 s/ }).click();
    await expect.poll(() => stored(page)).toEqual({
      seconds: 60,
      explained: false,
      explainedDesktop: false,
    });

    const screen = page.getByTestId("screen-recording-settings");
    await expect(screen.getByRole("tab", { name: "60 fps" })).toBeEnabled();
    await emitReplay(page, ARMED);
    await expect(screen).toContainText("Replay is on — turn it off to change these.");
    await expect(screen.getByRole("tab", { name: "60 fps" })).toBeDisabled();
    await expect(screen.getByRole("tab", { name: "Sharpest" })).toBeDisabled();
    await expect(screen.getByRole("switch", { name: /mouse pointer/ })).toBeDisabled();
    await expect(screen.getByTestId("desktop-audio-volume").getByRole("slider")).toHaveAttribute(
      "aria-disabled",
      "true",
    );
    // The separate files are the take's own.
    await expect(screen.getByRole("switch", { name: /separate WAV files/ })).toBeEnabled();

    await expect(page.getByText("Save replay", { exact: true })).toBeVisible();
    await expect(page.getByText("Turn Replay on or off", { exact: true })).toBeVisible();
    await expect(
      page.getByText("Works once you've turned Replay on from the Record screen card."),
    ).toBeVisible();
  });

  test("9: lengths over this PC's memory ceiling are greyed out and say why", async ({ page }) => {
    await engineUp(page);
    const lengths = page.getByTestId("replay-length");
    const over = lengths.getByRole("tab", { name: "120 s" });
    await expect(over).toBeDisabled();
    await expect(over).toHaveAttribute("title", /^Needs up to \d+ MB; this PC allows \d+ MB\.$/);
    // 1440p30 Balanced at 120 s caps at 303.8 MB; a 2 GiB PC allows 268.4.
    await expect(over).toHaveAttribute("title", "Needs up to 304 MB; this PC allows 268 MB.");
    await expect(lengths.getByRole("tab", { name: "60 s" })).toBeEnabled();
    await expect(lengths.getByRole("tab", { name: "60 s" })).not.toHaveAttribute("title", /.+/);
    await expect(page.getByTestId("replay-row")).toContainText(
      "Keeps the last 30 s · uses up to 81 MB of memory",
    );
  });
});

// v1.56.0: the mute (design §10.5). The mock keeps the engine's word in
// sessionStorage, so it outlives a reload as the engine's does, and answers
// the three commands by its rules. It never sends a tick: what the backend
// changes on its own (the hotkey) is played with pressMuteHotkey, and the
// tick that carries it with emitLevels. Every list of mute commands below is
// compared whole, so a command sent where none should be shows up in it.
test.describe("mute (v1.56)", () => {
  type Page = import("@playwright/test").Page;

  const pill = (page: Page) => page.getByTestId("titlebar-status");
  const announcer = (page: Page) => page.getByTestId("mute-announcer");
  const muteButton = (page: Page) => page.getByTestId("mute-button");
  const SET_TRUE = { cmd: "set_input_muted", args: { muted: true } };
  const SET_FALSE = { cmd: "set_input_muted", args: { muted: false } };
  const TOGGLE = { cmd: "toggle_input_muted", args: {} };
  const READ = { cmd: "input_mute", args: {} };

  /** The mute commands this page has sent, in order, the boot's read included. */
  const muteCalls = async (page: Page) =>
    (await invokesOf(page)).filter((i) =>
      ["input_mute", "set_input_muted", "toggle_input_muted"].includes(i.cmd),
    );
  /** Only the ones that change it. */
  const muteWrites = async (page: Page) =>
    (await muteCalls(page)).filter((i) => i.cmd !== "input_mute");
  /** A section of Settings, by its heading. */
  const settingsSection = (page: Page, heading: string) =>
    page.locator("section", {
      has: page.locator(".eyebrow", { hasText: new RegExp(`^${heading}$`) }),
    });
  /** Fire a `midi-message`, as the backend does for each note or CC. */
  const midi = (page: Page, kind: string, data1: number, data2: number) =>
    page.evaluate(
      (m) => {
        (
          window as unknown as { __E2E_EMIT__: (e: string, p: unknown) => void }
        ).__E2E_EMIT__("midi-message", m);
      },
      { channel: 0, kind, data1, data2 },
    );

  async function expectMuted(page: Page): Promise<void> {
    await expect(pill(page)).toHaveClass(/\bis-muted\b/);
    await expect(pill(page)).toHaveText("Muted");
    await expect(muteButton(page)).toHaveText("Unmute");
  }
  async function expectUnmuted(page: Page): Promise<void> {
    await expect(pill(page)).not.toHaveClass(/\bis-muted\b/);
    await expect(pill(page)).toHaveText(/^(Clean|Modulated)$/);
    await expect(muteButton(page)).toHaveText("Mute");
  }
  /** The Mixer, once the boot has read the mute (and sent nothing else for
   *  it) and started the engine, so the levels listener is up. */
  async function mixerReady(page: Page): Promise<void> {
    await nav(page, "Mixer").click();
    await expect(page.getByTestId("screen-record-toggle")).toBeEnabled();
    await expect.poll(() => muteCalls(page)).toEqual([READ]);
  }

  test("the Mixer button sends what its label says; the pill and the announcer follow", async ({
    page,
    consoleErrors,
  }) => {
    await mixerReady(page);
    await expectUnmuted(page);
    // Silent until this page has seen a mute: a launch never says "unmuted".
    await expect(announcer(page)).toHaveText("");
    await expect(pill(page)).not.toHaveAttribute("title");
    await expect(muteButton(page)).toHaveAttribute(
      "title",
      "Mute your voice for the call, your headphones, WAV and screen recordings and Replay. Clips and Speak still play.",
    );

    await muteButton(page).click();
    await expectMuted(page);
    expect(await muteWrites(page)).toEqual([SET_TRUE]);
    expect(await mockMute(page)).toEqual({ muted: true, seq: 1 });
    await expect(announcer(page)).toHaveText("Your voice is muted.");
    await expect(pill(page)).toHaveAttribute(
      "title",
      "Your voice is muted for the call, your headphones, WAV and screen recordings and Replay. Clips and Speak still play.",
    );
    await expect(page.getByText("Your voice is off · clips and Speak still play")).toBeVisible();
    await expect(muteButton(page)).toHaveAttribute("title", "Unmute your voice");

    await muteButton(page).click();
    await expectUnmuted(page);
    expect(await muteWrites(page)).toEqual([SET_TRUE, SET_FALSE]);
    expect(await mockMute(page)).toEqual({ muted: false, seq: 2 });
    await expect(announcer(page)).toHaveText("Your voice is unmuted.");
    await expect(pill(page)).not.toHaveAttribute("title");
    expect(consoleErrors, consoleErrors.join("\n")).toEqual([]);
  });

  test("the mute hotkey is the backend's: the page sends nothing, and the tick brings it", async ({
    page,
    consoleErrors,
  }) => {
    await mixerReady(page);

    // The backend flips its word on the press and still sends the event.
    expect(await pressMuteHotkey(page)).toEqual({ muted: true, seq: 1 });
    // The page must not act on it as well: that would flip it twice.
    await page.waitForTimeout(300);
    expect(await muteWrites(page)).toEqual([]);
    expect(await mockMute(page)).toEqual({ muted: true, seq: 1 });
    // The next tick brings it.
    await emitLevels(page, { inputMuted: true, inputMuteSeq: 1 });
    await expectMuted(page);
    await expect(announcer(page)).toHaveText("Your voice is muted.");

    // And back.
    expect(await pressMuteHotkey(page)).toEqual({ muted: false, seq: 2 });
    await page.waitForTimeout(300);
    expect(await muteWrites(page)).toEqual([]);
    await emitLevels(page, { inputMuted: false, inputMuteSeq: 2 });
    await expectUnmuted(page);
    await expect(announcer(page)).toHaveText("Your voice is unmuted.");
    expect(consoleErrors, consoleErrors.join("\n")).toEqual([]);
  });

  test("a stale Mute label sends Mute, never the opposite", async ({ page }) => {
    await mixerReady(page);
    // A hotkey muted the voice and its tick isn't in yet: the button still
    // says Mute.
    await pressMuteHotkey(page);
    await expect(muteButton(page)).toHaveText("Mute");

    await muteButton(page).click();
    // Mute again moves nothing in the engine, and the answer corrects the
    // label. A toggle here would have unmuted the call.
    await expectMuted(page);
    expect(await muteWrites(page)).toEqual([SET_TRUE]);
    expect(await mockMute(page)).toEqual({ muted: true, seq: 1 });
    // The late tick says the same.
    await emitLevels(page, { inputMuted: true, inputMuteSeq: 1 });
    await expectMuted(page);
  });

  test("a stale tick doesn't undo a mute; a tick without the mute says nothing; a newer one is taken", async ({
    page,
  }) => {
    await mixerReady(page);
    await muteButton(page).click();
    await expectMuted(page);
    expect(await mockMute(page)).toEqual({ muted: true, seq: 1 });

    // A tick built before the mute landed, arriving after its answer. Its
    // latency readout shows the page took the tick.
    await emitLevels(page, { inputMuted: false, inputMuteSeq: 0, dspLatencyMs: 12 });
    await expect(page.getByText(/\+12 ms latency/)).toBeVisible();
    await expectMuted(page);
    await expect(announcer(page)).toHaveText("Your voice is muted.");

    // No pair on the tick (an older backend): no information, never unmuted.
    await emitLevels(page, { dspLatencyMs: 13 });
    await expect(page.getByText(/\+13 ms latency/)).toBeVisible();
    await expectMuted(page);

    // Unmuted elsewhere (a newer seq): taken.
    await emitLevels(page, { inputMuted: false, inputMuteSeq: 2 });
    await expectUnmuted(page);
    await expect(announcer(page)).toHaveText("Your voice is unmuted.");
    expect(await muteWrites(page)).toEqual([SET_TRUE]);
  });

  test("a reload reads the mute and shows it, and never sends one", async ({ page }) => {
    await mixerReady(page);
    await muteButton(page).click();
    await expectMuted(page);

    await page.reload();
    await expect(page.locator(".titlebar-wordmark")).toBeVisible();
    // Muted from the read alone: the mock sends no tick.
    await expect(pill(page)).toHaveClass(/\bis-muted\b/);
    await expect(pill(page)).toHaveText("Muted");
    // Through the rest of the boot (enumeration, the auto-start, the hotkey
    // sync) it only ever read.
    await mixerReady(page);
    await expect(muteButton(page)).toHaveText("Unmute");
    await page.waitForTimeout(300);
    expect(await muteCalls(page)).toEqual([READ]);
    expect(await mockMute(page)).toEqual({ muted: true, seq: 1 });
    // Sent before the boot's device enumeration, which holds the main thread.
    const cmds = (await invokesOf(page)).map((i) => i.cmd);
    expect(cmds.indexOf("input_mute")).toBeLessThan(cmds.indexOf("list_audio_input_devices"));
  });

  test("a Mute / unmute glyph sends what its omen says", async ({ page, consoleErrors }) => {
    // Bound in Settings, as a user would.
    await nav(page, "Settings").click();
    const triangle = settingsSection(page, "Glyph casting")
      .locator(".panel > div")
      .filter({ has: page.getByText("Triangle", { exact: true }) });
    const kind = triangle.locator("button.select").first();
    await kind.click();
    await triangle.getByRole("option").filter({ hasText: "Mute / unmute" }).click();
    await expect(kind).toContainText("Mute / unmute");
    expect(
      await page.evaluate(
        () => JSON.parse(localStorage.getItem("divora.glyphBindings") ?? "{}").triangle,
      ),
    ).toEqual({ kind: "mute" });

    // The omen says Mute, and the cast sends Mute; then Unmute.
    await mixerReady(page);
    await traceTriangle(page);
    await expectMuted(page);
    expect(await muteWrites(page)).toEqual([SET_TRUE]);
    await traceTriangle(page);
    await expectUnmuted(page);
    expect(await muteWrites(page)).toEqual([SET_TRUE, SET_FALSE]);
    expect(await mockMute(page)).toEqual({ muted: false, seq: 2 });
    expect(consoleErrors, consoleErrors.join("\n")).toEqual([]);
  });

  test("Settings: Mute your voice binds a hotkey, which the boot registers again", async ({
    page,
  }) => {
    await nav(page, "Settings").click();
    const rows = settingsSection(page, "Hotkeys").locator(".panel > div");
    const row = rows.filter({ has: page.getByText("Mute your voice", { exact: true }) });
    await expect(row).toHaveCount(1);
    await expect(row).toContainText(
      "Mutes or unmutes your voice for the call, your headphones, WAV and screen recordings and Replay. Clips and Speak still play.",
    );
    // Right after Toggle monitor, whose row no longer says "mute".
    const labels = (await rows.allInnerTexts()).map((t) => t.split("\n")[0]);
    expect(labels).toContain("Toggle monitor");
    expect(labels.indexOf("Mute your voice")).toBe(labels.indexOf("Toggle monitor") + 1);
    const monitor = rows.filter({ has: page.getByText("Toggle monitor", { exact: true }) });
    await expect(monitor).toContainText("the call included");
    await expect(monitor).not.toContainText(/mute/i);

    // Unbound until set.
    await row.getByRole("button", { name: "Click to set" }).click();
    await page.keyboard.press("F8");
    const bound = { cmd: "register_global_shortcut", args: { id: "mute", accelerator: "F8" } };
    await expect.poll(() => invokesOf(page, "register_global_shortcut")).toEqual([bound]);
    expect(
      await page.evaluate(
        () => JSON.parse(localStorage.getItem("divora.hotkeyBindings") ?? "{}").mute,
      ),
    ).toBe("F8");
    await expect(row.getByRole("button", { name: "Clear Mute your voice hotkey" })).toBeVisible();

    // A restart registers it again from what was saved.
    await page.reload();
    await expect(page.locator(".titlebar-wordmark")).toBeVisible();
    await expect.poll(() => invokesOf(page, "register_global_shortcut")).toEqual([bound]);
    // Binding it sends no mute.
    expect(await muteWrites(page)).toEqual([]);
  });

  test("Settings: a MIDI mapping can be Mute your voice; a learned pad toggles it", async ({
    page,
    consoleErrors,
  }) => {
    await nav(page, "Settings").click();
    // The MIDI listener is up once the boot has listed the ports.
    await expect.poll(async () => (await invokesOf(page, "list_midi_inputs")).length).toBeGreaterThan(0);
    const surfaces = settingsSection(page, "Control surfaces");
    await expect(surfaces).toContainText(
      "Mute your voice toggles with each press of a pad that sends a note (its mapping shows Note), unless the pad is set to latch: then every second press just ends the note and does nothing. On a knob, fader or CC button it mutes when the control goes past halfway and unmutes when it comes back a little below halfway (59 or less); a control that stays on one side never changes it, except that its first move after launch (or after you pick the device or change its mapping) mutes if it is past halfway.",
    );

    await surfaces.getByRole("button", { name: "Add mapping" }).click();
    const action = surfaces.locator("button.select", { hasText: "Summon preset" });
    await action.click();
    await surfaces.getByRole("option").filter({ hasText: "Mute your voice" }).click();
    await expect(surfaces.locator("button.select", { hasText: "Mute your voice" })).toBeVisible();

    // Learn a pad: the message that teaches it isn't routed.
    await surfaces.getByRole("button", { name: "Learn" }).click();
    await midi(page, "note-on", 36, 100);
    await expect(surfaces.getByRole("button", { name: "Learn" })).toBeVisible();
    expect(
      await page.evaluate(() => JSON.parse(localStorage.getItem("divora.midiMappings") ?? "[]")),
    ).toMatchObject([{ action: "mute", trigger: { kind: "note", data1: 36, channel: 0 } }]);
    expect(await muteWrites(page)).toEqual([]);

    // The pad toggles in the backend, on note-on only.
    await midi(page, "note-on", 36, 100);
    await expect(pill(page)).toHaveText("Muted");
    await midi(page, "note-off", 36, 0);
    expect(await muteWrites(page)).toEqual([TOGGLE]);
    expect(await mockMute(page)).toEqual({ muted: true, seq: 1 });
    expect(consoleErrors, consoleErrors.join("\n")).toEqual([]);
  });

  test("MIDI: a pad toggles; a CC mutes and unmutes only when it crosses", async ({ page }) => {
    await page.evaluate(() =>
      localStorage.setItem(
        "divora.midiMappings",
        JSON.stringify([
          { id: "pad", action: "mute", trigger: { kind: "note", data1: 36, channel: 0 } },
          { id: "fader", action: "mute", trigger: { kind: "cc", data1: 7, channel: 0 } },
        ]),
      ),
    );
    await page.reload();
    await mixerReady(page);
    await expect.poll(async () => (await invokesOf(page, "list_midi_inputs")).length).toBeGreaterThan(0);

    await midi(page, "note-on", 36, 100);
    await expectMuted(page);
    await midi(page, "note-off", 36, 0);
    expect(await muteWrites(page)).toEqual([TOGGLE]);

    // The fader's first message after load, resting low: nothing.
    await midi(page, "control-change", 7, 0);
    expect(await muteWrites(page)).toEqual([TOGGLE]);
    // Pushed past halfway: Mute (already muted, so the engine keeps its seq).
    await midi(page, "control-change", 7, 127);
    await expect.poll(() => muteWrites(page)).toEqual([TOGGLE, SET_TRUE]);
    expect(await mockMute(page)).toEqual({ muted: true, seq: 1 });
    // Into the dead band (60-63): nothing; still muted. Routing is
    // synchronous, so the command list is final when the emit returns.
    await midi(page, "control-change", 7, 62);
    expect(await muteWrites(page)).toEqual([TOGGLE, SET_TRUE]);
    // Below it: Unmute.
    await midi(page, "control-change", 7, 0);
    await expect.poll(() => muteWrites(page)).toEqual([TOGGLE, SET_TRUE, SET_FALSE]);
    await expectUnmuted(page);

    // A hotkey mutes; the fader, resting low, is touched: it doesn't undo it.
    await pressMuteHotkey(page);
    await emitLevels(page, { inputMuted: true, inputMuteSeq: 3 });
    await expectMuted(page);
    await midi(page, "control-change", 7, 10);
    await midi(page, "control-change", 7, 11);
    expect(await muteWrites(page)).toEqual([TOGGLE, SET_TRUE, SET_FALSE]);
    await expectMuted(page);

    // The pad flips it back.
    await midi(page, "note-on", 36, 100);
    await expectUnmuted(page);
    expect(await muteWrites(page)).toEqual([TOGGLE, SET_TRUE, SET_FALSE, TOGGLE]);
    expect(await mockMute(page)).toEqual({ muted: false, seq: 4 });
  });
});

// v1.58.0: push-to-modulate (design §10.5). The mock keeps the engine's word
// (the mode, a held bit per source, `seq`) in sessionStorage, so it outlives
// a reload as the engine's does, and answers `set_ptm_mode` and
// `set_ptm_held` by the engine's rules. As for the mute, it never sends a
// tick: what the backend changes on its own (the global key) is played with
// pressPtmHotkey, and the tick that carries it with emitLevels. Every list of
// push-to-modulate commands below is compared whole, the boot's restore
// included, so a command sent where none should be shows up in it.
test.describe("push-to-modulate (v1.58)", () => {
  type Page = import("@playwright/test").Page;

  const pill = (page: Page) => page.getByTestId("titlebar-status");
  const ptmSwitch = (page: Page) =>
    page.getByRole("switch", { name: "Push to modulate", exact: true });
  const segment = (page: Page, label: string) =>
    page.getByRole("tab", { name: label, exact: true });
  const holdButton = (page: Page) => page.getByTestId("ptm-hold");
  /** The card's line under its switch (the switch's description). */
  const cardLine = (page: Page) => page.locator('[id^="ptm-line-"]');
  /** `divora.ptm`, as saved from the engine's answers. */
  const stored = (page: Page) =>
    page.evaluate(() => JSON.parse(localStorage.getItem("divora.ptm") ?? "null") as unknown);
  const focusIsOnThePage = (page: Page) =>
    page.evaluate(() => document.activeElement === document.body);

  const RESTORE = (mode: string) => ({ cmd: "set_ptm_mode", args: { mode, restore: true } });
  const SET = (mode: string) => ({ cmd: "set_ptm_mode", args: { mode, restore: false } });
  const HELD = (source: string, held: boolean) => ({ cmd: "set_ptm_held", args: { source, held } });
  const NONE = { hotkey: false, key: false, button: false, midi: false };
  const APPLY_LINE_IN_APP =
    "Your effects are on only while you hold Space in Divora's window. To use it in a call, set a key in Settings → Hotkeys.";

  /** The push-to-modulate commands this page has sent, in order, the boot's
   *  restore included. */
  const ptmCalls = async (page: Page) =>
    (await invokesOf(page)).filter((i) => i.cmd === "set_ptm_mode" || i.cmd === "set_ptm_held");
  /** Fire a `midi-message`, as the backend does for each note or CC. */
  const midi = (page: Page, kind: string, data1: number, data2: number) =>
    page.evaluate(
      (m) => {
        (
          window as unknown as { __E2E_EMIT__: (e: string, p: unknown) => void }
        ).__E2E_EMIT__("midi-message", m);
      },
      { channel: 0, kind, data1, data2 },
    );

  /** The Mixer with the engine up, once the boot has restored `mode` and
   *  sent nothing else for push-to-modulate. */
  async function mixerReady(page: Page, mode = "always"): Promise<void> {
    await nav(page, "Mixer").click();
    await expect(page.getByTestId("screen-record-toggle")).toBeEnabled();
    await expect.poll(() => ptmCalls(page)).toEqual([RESTORE(mode)]);
  }
  /** Turn it on from the card's switch, as a user does, and wait for the
   *  engine's answer to move the card. */
  async function turnOn(page: Page): Promise<void> {
    await ptmSwitch(page).click();
    await expect(ptmSwitch(page)).toHaveAttribute("aria-checked", "true");
  }

  test("off by default; the card turns it on and off and picks the hold, from the engine's answers", async ({
    page,
    consoleErrors,
  }) => {
    await mixerReady(page);
    // Off ("always"): the effects are on whatever is held, and the boot told
    // the engine so, which changed nothing there.
    await expect(pill(page)).toHaveText("Modulated");
    expect(await mockPtm(page)).toEqual({ mode: "always", held: NONE, seq: 0 });
    const line = cardLine(page);
    await expect(ptmSwitch(page)).toHaveAttribute("aria-checked", "false");
    await expect(ptmSwitch(page)).toHaveAttribute("aria-describedby", (await line.getAttribute("id"))!);
    await expect(line).toHaveText("Off · your effects are always on.");
    await expect(segment(page, "Hold to apply")).toHaveAttribute("aria-selected", "true");
    await expect(segment(page, "Hold to apply")).toBeDisabled();
    await expect(segment(page, "Hold to bypass")).toBeDisabled();
    await expect(holdButton(page)).toBeDisabled();
    await expect(holdButton(page)).toHaveAttribute("title", "Turn on Push to modulate to try it.");
    await expect(holdButton(page)).toHaveText("Hold to switch · or press Space");
    expect(await stored(page)).toBeNull();

    // On, Hold to apply: with the key up the call hears your own voice. The
    // setting is saved from the answer.
    await turnOn(page);
    expect(await ptmCalls(page)).toEqual([RESTORE("always"), SET("apply")]);
    await expect(pill(page)).toHaveText("Clean");
    expect(await stored(page)).toEqual({ on: true, hold: "apply" });
    expect(await mockPtm(page)).toEqual({ mode: "apply", held: NONE, seq: 1 });
    // No global key and no MIDI mapping: the line says where the key works,
    // and where to set one that works in a call.
    await expect(line).toHaveText(APPLY_LINE_IN_APP);
    await expect(segment(page, "Hold to bypass")).toBeEnabled();
    await expect(holdButton(page)).toBeEnabled();
    await expect(holdButton(page)).toHaveAttribute(
      "title",
      "Switches your effects for the call, your headphones, recordings and Replay while held.",
    );
    // The click handed the focus back to the page, so Space holds
    // push-to-modulate. On a focused switch it would have turned it off.
    expect(await focusIsOnThePage(page)).toBe(true);
    await page.keyboard.press("Space");
    await expect
      .poll(() => ptmCalls(page))
      .toEqual([RESTORE("always"), SET("apply"), HELD("key", true), HELD("key", false)]);
    await expect(ptmSwitch(page)).toHaveAttribute("aria-checked", "true");

    // Hold to bypass: the effects are on with the key up.
    await segment(page, "Hold to bypass").click();
    await expect(segment(page, "Hold to bypass")).toHaveAttribute("aria-selected", "true");
    await expect(pill(page)).toHaveText("Modulated");
    expect(await stored(page)).toEqual({ on: true, hold: "bypass" });
    await expect(line).toHaveText(
      "Your effects are off while you hold Space in Divora's window. To use it in a call, set a key in Settings → Hotkeys.",
    );
    expect(await focusIsOnThePage(page)).toBe(true);

    // Off keeps the hold for the next time it is turned on.
    await ptmSwitch(page).click();
    await expect(ptmSwitch(page)).toHaveAttribute("aria-checked", "false");
    await expect(line).toHaveText("Off · your effects are always on.");
    expect(await stored(page)).toEqual({ on: false, hold: "bypass" });
    await expect(segment(page, "Hold to bypass")).toHaveAttribute("aria-selected", "true");
    await expect(segment(page, "Hold to bypass")).toBeDisabled();
    await expect(holdButton(page)).toBeDisabled();
    await expect(pill(page)).toHaveText("Modulated");
    await turnOn(page);
    expect(await stored(page)).toEqual({ on: true, hold: "bypass" });
    // Picking the hold it already has sends it again: the way out of a
    // stuck hold.
    await segment(page, "Hold to bypass").click();
    await expect
      .poll(() => ptmCalls(page))
      .toEqual([
        RESTORE("always"),
        SET("apply"),
        HELD("key", true),
        HELD("key", false),
        SET("bypass"),
        SET("always"),
        SET("bypass"),
        SET("bypass"),
      ]);
    // Nothing was held, so the last moved nothing in the engine.
    expect(await mockPtm(page)).toEqual({ mode: "bypass", held: NONE, seq: 6 });
    await expect(pill(page)).toHaveText("Modulated");
    expect(consoleErrors, consoleErrors.join("\n")).toEqual([]);
  });

  test("Space holds while it's down: once, with exactly its modifiers; the pill and the ticks follow", async ({
    page,
    consoleErrors,
  }) => {
    await mixerReady(page);
    await turnOn(page);
    await expect(pill(page)).toHaveText("Clean");

    await page.keyboard.down("Space");
    await expect(pill(page)).toHaveText("Modulated");
    expect(await ptmCalls(page)).toEqual([RESTORE("always"), SET("apply"), HELD("key", true)]);
    expect(await mockPtm(page)).toEqual({ mode: "apply", held: { ...NONE, key: true }, seq: 2 });
    // Autorepeat (every down after the first is a repeat): nothing more.
    await page.keyboard.down("Space");
    await page.keyboard.down("Space");
    // The tick says the same. One built before the hold and arriving after
    // its answer is ignored; its latency readout shows the page took it.
    await emitLevels(page);
    await expect(pill(page)).toHaveText("Modulated");
    await emitLevels(page, {
      ptm: { mode: "apply", held: false, bypassed: true, seq: 1 },
      dspLatencyMs: 12,
    });
    await expect(page.getByText(/\+12 ms latency/)).toBeVisible();
    await expect(pill(page)).toHaveText("Modulated");

    await page.keyboard.up("Space");
    await expect(pill(page)).toHaveText("Clean");
    await emitLevels(page);
    await expect(pill(page)).toHaveText("Clean");
    expect(await mockPtm(page)).toEqual({ mode: "apply", held: NONE, seq: 3 });

    // The key is Space alone: Shift+Space holds nothing.
    await page.keyboard.down("Shift");
    await page.keyboard.down("Space");
    await page.keyboard.up("Space");
    await page.keyboard.up("Shift");
    // A press after it still holds, and the whole list shows nothing went
    // for the repeats or for Shift+Space.
    await page.keyboard.press("Space");
    await expect
      .poll(() => ptmCalls(page))
      .toEqual([
        RESTORE("always"),
        SET("apply"),
        HELD("key", true),
        HELD("key", false),
        HELD("key", true),
        HELD("key", false),
      ]);
    await expect(pill(page)).toHaveText("Clean");
    expect(consoleErrors, consoleErrors.join("\n")).toEqual([]);
  });

  test("Hold to switch holds while the pointer or Enter is down, and a pointer press hands the focus back", async ({
    page,
    consoleErrors,
  }) => {
    await mixerReady(page);
    await turnOn(page);
    const button = holdButton(page);
    await expect(button).toHaveText("Hold to switch · or press Space");
    await expect(button).toHaveAttribute("aria-pressed", "false");

    // The pointer: held from down to up.
    await button.hover();
    await page.mouse.down();
    await expect(button).toHaveAttribute("aria-pressed", "true");
    await expect(button).toHaveText("Effects on");
    await expect(pill(page)).toHaveText("Modulated");
    expect(await mockPtm(page)).toEqual({ mode: "apply", held: { ...NONE, button: true }, seq: 2 });
    await page.mouse.up();
    await expect(button).toHaveAttribute("aria-pressed", "false");
    await expect(button).toHaveText("Hold to switch · or press Space");
    await expect(pill(page)).toHaveText("Clean");
    expect(await ptmCalls(page)).toEqual([
      RESTORE("always"),
      SET("apply"),
      HELD("button", true),
      HELD("button", false),
    ]);
    // The focus is back on the page, so Space holds the key, not the button.
    expect(await focusIsOnThePage(page)).toBe(true);
    await page.keyboard.press("Space");
    await expect.poll(async () => (await ptmCalls(page)).slice(4)).toEqual([
      HELD("key", true),
      HELD("key", false),
    ]);

    // Leaving the button lets go, with the mouse still down (no capture, so
    // it can't stick); the release over the card sends nothing more.
    await button.hover();
    await page.mouse.down();
    await expect(pill(page)).toHaveText("Modulated");
    await page.getByText("Push to modulate", { exact: true }).hover();
    await expect(pill(page)).toHaveText("Clean");
    await page.mouse.up();
    await expect(button).toHaveAttribute("aria-pressed", "false");

    // From the keyboard: Enter holds while down, ignores its autorepeat, and
    // the focus stays on the button.
    await button.focus();
    await page.keyboard.down("Enter");
    await expect(button).toHaveText("Effects on");
    await expect(pill(page)).toHaveText("Modulated");
    await page.keyboard.down("Enter");
    await page.keyboard.up("Enter");
    await expect(button).toHaveText("Hold to switch · or press Space");
    await expect(pill(page)).toHaveText("Clean");
    await expect(button).toBeFocused();

    // Hold to bypass: held, the effects are off.
    await segment(page, "Hold to bypass").click();
    await expect(pill(page)).toHaveText("Modulated");
    await button.hover();
    await page.mouse.down();
    await expect(button).toHaveText("Effects off");
    await expect(pill(page)).toHaveText("Clean");
    await page.mouse.up();
    await expect(pill(page)).toHaveText("Modulated");
    await expect
      .poll(async () => (await ptmCalls(page)).slice(6))
      .toEqual([
        HELD("button", true),
        HELD("button", false),
        HELD("button", true),
        HELD("button", false),
        SET("bypass"),
        HELD("button", true),
        HELD("button", false),
      ]);
    expect(consoleErrors, consoleErrors.join("\n")).toEqual([]);
  });

  test("the global key is the backend's: the page sends nothing, the tick brings it, and a reload keeps it held", async ({
    page,
    consoleErrors,
  }) => {
    // Bound in Settings, as a user would.
    await nav(page, "Settings").click();
    const rows = page
      .locator("section", { has: page.locator(".eyebrow", { hasText: /^Hotkeys$/ }) })
      .locator(".panel > div");
    const row = rows.filter({ has: page.getByText("Push-to-modulate", { exact: true }) });
    await expect(row).toHaveCount(1);
    await expect(row).toContainText(
      "Hold to switch your effects on or off, as the Mixer's Push to modulate card is set. Does nothing while that card is off.",
    );
    await row.getByRole("button", { name: "Click to set" }).click();
    await page.keyboard.press("F8");
    const bound = { cmd: "register_global_shortcut", args: { id: "ptm", accelerator: "F8" } };
    await expect.poll(() => invokesOf(page, "register_global_shortcut")).toEqual([bound]);

    await mixerReady(page);
    await turnOn(page);
    await expect(pill(page)).toHaveText("Clean");
    // A key that works in a call: the line no longer sends you to Settings.
    await expect(cardLine(page)).toHaveText("Your effects are on only while you hold F8.");

    // The backend holds it on the press and still sends the event.
    expect(await pressPtmHotkey(page, "pressed")).toEqual({
      mode: "apply",
      held: true,
      bypassed: false,
      seq: 2,
    });
    // The page must not act on it as well.
    await page.waitForTimeout(300);
    expect(await ptmCalls(page)).toEqual([RESTORE("always"), SET("apply")]);
    // The next tick brings it.
    await emitLevels(page);
    await expect(pill(page)).toHaveText("Modulated");

    // A reload while it is held: the boot restores the mode, before it
    // enumerates devices, and keeps the global key's hold.
    await page.reload();
    await expect(page.locator(".titlebar-wordmark")).toBeVisible();
    await mixerReady(page, "apply");
    const cmds = (await invokesOf(page)).map((i) => i.cmd);
    expect(cmds.indexOf("set_ptm_mode")).toBeLessThan(cmds.indexOf("list_audio_input_devices"));
    expect(await mockPtm(page)).toEqual({ mode: "apply", held: { ...NONE, hotkey: true }, seq: 2 });
    await expect(pill(page)).toHaveText("Modulated");
    await expect.poll(() => invokesOf(page, "register_global_shortcut")).toEqual([bound]);

    // Let go: the backend's again, and the tick brings it.
    expect(await pressPtmHotkey(page, "released")).toEqual({
      mode: "apply",
      held: false,
      bypassed: true,
      seq: 3,
    });
    await page.waitForTimeout(300);
    expect(await ptmCalls(page)).toEqual([RESTORE("apply")]);
    await expect(pill(page)).toHaveText("Modulated");
    await emitLevels(page);
    await expect(pill(page)).toHaveText("Clean");
    expect(consoleErrors, consoleErrors.join("\n")).toEqual([]);
  });

  test("a reload restores the mode and lets go of the page's holds, sending no hold", async ({
    page,
    consoleErrors,
  }) => {
    await mixerReady(page);
    await turnOn(page);
    await page.keyboard.down("Space");
    await expect(pill(page)).toHaveText("Modulated");
    // As a page that went away without letting go would leave them: the
    // button and a pad held too.
    await page.evaluate(() => {
      const w = JSON.parse(sessionStorage.getItem("__E2E_PTM__") ?? "null") as {
        held: Record<string, boolean>;
        seq: number;
      };
      w.held = { ...w.held, button: true, midi: true };
      w.seq += 1;
      sessionStorage.setItem("__E2E_PTM__", JSON.stringify(w));
    });

    // Space is still down when the page comes back.
    await page.reload();
    await expect(page.locator(".titlebar-wordmark")).toBeVisible();
    // One restore, sent before the boot enumerates devices; the engine let go
    // of everything the page held.
    await expect.poll(() => ptmCalls(page)).toEqual([RESTORE("apply")]);
    const cmds = (await invokesOf(page)).map((i) => i.cmd);
    expect(cmds.indexOf("set_ptm_mode")).toBeLessThan(cmds.indexOf("list_audio_input_devices"));
    expect((await mockPtm(page)).held).toEqual(NONE);
    // Clean from the answer alone: the mock sends no tick.
    await expect(pill(page)).toHaveText("Clean");

    // This page never held the key, so letting go of it sends nothing.
    await page.keyboard.up("Space");
    await page.waitForTimeout(300);
    expect(await ptmCalls(page)).toEqual([RESTORE("apply")]);
    // A new press holds as before.
    await page.keyboard.press("Space");
    await expect
      .poll(() => ptmCalls(page))
      .toEqual([RESTORE("apply"), HELD("key", true), HELD("key", false)]);
    // The card says what was restored.
    await nav(page, "Mixer").click();
    await expect(ptmSwitch(page)).toHaveAttribute("aria-checked", "true");
    await expect(cardLine(page)).toHaveText(APPLY_LINE_IN_APP);
    expect(consoleErrors, consoleErrors.join("\n")).toEqual([]);
  });

  test("MIDI: a pad holds while its note is down; a CC past halfway, until 59 or less; two hold until both let go", async ({
    page,
    consoleErrors,
  }) => {
    await page.evaluate(() =>
      localStorage.setItem(
        "divora.midiMappings",
        JSON.stringify([
          { id: "pad", action: "ptm", trigger: { kind: "note", data1: 36, channel: 0 } },
          { id: "fader", action: "ptm", trigger: { kind: "cc", data1: 7, channel: 0 } },
        ]),
      ),
    );
    await page.reload();
    await mixerReady(page);
    await expect.poll(async () => (await invokesOf(page, "list_midi_inputs")).length).toBeGreaterThan(0);
    await turnOn(page);
    // A learned mapping works in a call, so the line doesn't send you to
    // Settings; with no global key, Space still works only in Divora's
    // window, so the line says so and names the MIDI control.
    await expect(cardLine(page)).toHaveText(
      "Your effects are on only while you hold Space in Divora's window, or with your MIDI control.",
    );
    await expect(pill(page)).toHaveText("Clean");
    const before = [RESTORE("always"), SET("apply")];

    // Routing is synchronous, so the command list is final when the emit
    // returns.
    await midi(page, "note-on", 36, 100);
    await expect(pill(page)).toHaveText("Modulated");
    expect((await mockPtm(page)).held).toEqual({ ...NONE, midi: true });
    await midi(page, "note-off", 36, 0);
    await expect(pill(page)).toHaveText("Clean");
    expect(await ptmCalls(page)).toEqual([...before, HELD("midi", true), HELD("midi", false)]);

    // The fader: past halfway holds, the dead band (60-63) keeps it, 59 or
    // less lets go.
    await midi(page, "control-change", 7, 70);
    await expect(pill(page)).toHaveText("Modulated");
    await midi(page, "control-change", 7, 62);
    expect(await ptmCalls(page)).toEqual([
      ...before,
      HELD("midi", true),
      HELD("midi", false),
      HELD("midi", true),
    ]);
    await midi(page, "control-change", 7, 50);
    await expect(pill(page)).toHaveText("Clean");

    // The pad and the fader together: held until both have let go.
    await midi(page, "note-on", 36, 100);
    await midi(page, "control-change", 7, 90);
    await midi(page, "note-off", 36, 0);
    await expect(pill(page)).toHaveText("Modulated");
    await midi(page, "control-change", 7, 20);
    await expect(pill(page)).toHaveText("Clean");
    expect(await ptmCalls(page)).toEqual([
      ...before,
      HELD("midi", true),
      HELD("midi", false),
      HELD("midi", true),
      HELD("midi", false),
      HELD("midi", true),
      HELD("midi", false),
    ]);
    expect(await mockPtm(page)).toEqual({ mode: "apply", held: NONE, seq: 7 });
    expect(consoleErrors, consoleErrors.join("\n")).toEqual([]);
  });

  test("the stream overlay's status follows it, and waits for the boot's restore after a reload", async ({
    page,
  }) => {
    type State = { status: string };
    const statuses = async (): Promise<string[]> =>
      ((await emitsOf(page, "overlay:state")) as State[]).map((s) => s.status);
    const lastStatus = async (): Promise<string | undefined> => (await statuses()).at(-1);

    // Open the overlay from Settings: Hollow King with push-to-modulate off.
    await nav(page, "Settings").click();
    await page.getByRole("switch", { name: "Show stream overlay window" }).click();
    await expect.poll(() => invokesOf(page, "open_overlay")).toHaveLength(1);
    await expect.poll(lastStatus).toBe("modulated");

    await mixerReady(page);
    await turnOn(page);
    await expect.poll(lastStatus).toBe("clean");
    await page.keyboard.down("Space");
    await expect.poll(lastStatus).toBe("modulated");
    await page.keyboard.up("Space");
    await expect.poll(lastStatus).toBe("clean");

    // A reload with the overlay already open and the engine slow to answer
    // the restore: nothing goes to the stream until the answer lands, so the
    // circle never says Modulated over a voice that is clean. (The overlay's
    // status lives in the page's mock; this opens it in the next one.)
    await page.addInitScript(() => {
      const o = (window as unknown as { __E2E_OVERLAY__: { status: Record<string, unknown> } })
        .__E2E_OVERLAY__;
      o.status = { ...o.status, open: true, phase: "painted", instance: 1, canArrange: true };
    });
    await delayPtmModeAnswers(page, 300);
    await page.reload();
    await expect(page.locator(".titlebar-wordmark")).toBeVisible();
    await expect.poll(() => ptmCalls(page)).toEqual([RESTORE("apply")]);
    await expect(pill(page)).toHaveText("Clean");
    await expect.poll(lastStatus).toBe("clean");
    await page.waitForTimeout(300);
    expect(await statuses()).not.toContain("modulated");
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
    await expect(page.locator(".sidebar-nav")).toHaveCount(6);
  });
});
