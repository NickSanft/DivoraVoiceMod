// E2E smoke tests — the whole SolidJS app wired together, Tauri mocked.
// Covers the core UI flows that docs/MANUAL_TESTS.md otherwise carries
// entirely by hand. See e2e/tauri-mock.ts for the backend stub.

import { test, expect, emitReplay, emitReplaySaved } from "./tauri-mock";

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
    // shell's toast is a status region too, always mounted since v1.54.0.)
    await expect(page.locator('[role="status"]:not([data-testid="toast"])')).toHaveCount(0);
    await expect(page.getByTestId("toast")).toHaveText("");

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
    // The clean voice (push-to-modulate up) is named as what the call heard.
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

    // Push-to-modulate off ("Hold to bypass"), Hollow King's effects on.
    await seg(page, "Hold to bypass").click();
    await expect(page.locator(".titlebar-status")).toContainText(/Modulated/i);
    // The clip (and the save's own 3 s) must lie after the voice turned.
    await page.waitForTimeout(4_000);
    await emitReplaySaved(page, saved);
    await expect(page.getByTestId("toast")).toHaveText("Saved a 1 s replay · Hollow King");

    await traceTriangle(page);
    await expect(page.locator(".titlebar-status")).toContainText(/Muted/i);
    await emitReplaySaved(page, saved);
    await expect(page.getByTestId("toast")).toHaveText(
      "Saved a 1 s replay · your voice as your call heard it",
    );
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
