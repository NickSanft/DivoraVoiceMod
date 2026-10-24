// E2E for the model downloads (v1.57.0), Tauri mocked (see e2e/tauri-mock.ts).
// The backend runs a download on a worker now, so the app stays live while
// one runs, and a reload can happen mid-download. A mock can't show which
// thread runs a command; these pin the page's side: the button follows the
// backend's progress and done events, the rest of the app still answers
// while a download is open, and a reloaded page picks the download up from
// `model_download_status`.
//
// Each download stays open until the spec ends it (endDownload), in the
// backend's order: the slot frees, the done event goes out, then the command
// answers.

import type { Page } from "@playwright/test";
import {
  test,
  expect,
  emitDownloadProgress,
  endDownload,
  invokesOf,
  mockDownload,
} from "./tauri-mock";

// The cloning block, and so its downloads, shows only with a Kokoro voice.
test.use({ kokoroInstalled: true });

const nav = (page: Page, label: string) => page.locator(".sidebar-nav", { hasText: label });
const CLONE = "Download voice-cloning models (~157 MB)";
const VOXCPM = "Accent-preserving clones (~1.6 GB)";
const button = (page: Page, name: string | RegExp) => page.getByRole("button", { name });
const downloading = (page: Page) => button(page, /^Downloading… \d+%$/);

test.beforeEach(async ({ page }) => {
  await page.goto("/");
  await expect(page.locator(".titlebar-wordmark")).toBeVisible();
  await nav(page, "Speak").click();
  await expect(button(page, CLONE)).toBeEnabled();
});

test("the clone models download while the app stays live; the button goes once they're there", async ({
  page,
  consoleErrors,
}) => {
  await button(page, CLONE).click();
  await expect(downloading(page)).toHaveText("Downloading… 0%");
  await expect(downloading(page)).toBeDisabled();
  expect(await invokesOf(page, "download_clone_models")).toEqual([
    { cmd: "download_clone_models", args: {} },
  ]);

  await emitDownloadProgress(page, { received: 1_048_576, total: 2_621_440 });
  await expect(downloading(page)).toHaveText("Downloading… 40%");
  // A progress event with no length keeps the last %.
  await emitDownloadProgress(page, { received: 2_097_152, total: 0 });
  await page.waitForTimeout(100);
  await expect(downloading(page)).toHaveText("Downloading… 40%");
  await emitDownloadProgress(page, { file: 2, received: 1_000_000, total: 4_000_000 });
  await expect(downloading(page)).toHaveText("Downloading… 25%");

  // The rest of the app answers while the download is open: the Mixer's
  // Mute, sent and shown.
  await nav(page, "Mixer").click();
  await page.getByTestId("mute-button").click();
  await expect(page.getByTestId("mute-button")).toHaveText("Unmute");
  await expect(page.getByTestId("titlebar-status")).toHaveText("Muted");
  expect(await invokesOf(page, "set_input_muted")).toEqual([
    { cmd: "set_input_muted", args: { muted: true } },
  ]);

  // Back on Speak (a fresh mount), the download still shows.
  await nav(page, "Speak").click();
  await expect(downloading(page)).toHaveText("Downloading… 25%");
  await expect(downloading(page)).toBeDisabled();

  await endDownload(page);
  // The models are there: the button gives way to the VoxCPM upgrade.
  await expect(button(page, VOXCPM)).toBeEnabled();
  await expect(button(page, CLONE)).toHaveCount(0);
  await expect(downloading(page)).toHaveCount(0);
  await expect(page.getByRole("alert")).toHaveCount(0);
  expect(await invokesOf(page, "download_clone_models")).toHaveLength(1);

  // The next download, of another set, follows its own events.
  await button(page, VOXCPM).click();
  await expect(downloading(page)).toHaveText("Downloading… 0%");
  expect(await mockDownload(page)).toMatchObject({ kind: "voxcpm", fileCount: 6 });
  await emitDownloadProgress(page, { file: 3, received: 3, total: 4 });
  await expect(downloading(page)).toHaveText("Downloading… 75%");
  await endDownload(page);
  await expect(button(page, VOXCPM)).toHaveCount(0);
  await expect(downloading(page)).toHaveCount(0);
  expect(consoleErrors, consoleErrors.join("\n")).toEqual([]);
});

test("a failed download says why, and the button comes back", async ({ page, consoleErrors }) => {
  const WHY = "downloading openvoice-converter.onnx: ended early: got 1048576 of 3364792 bytes";
  await button(page, CLONE).click();
  await emitDownloadProgress(page, { received: 1_048_576, total: 3_364_792 });
  await expect(downloading(page)).toHaveText("Downloading… 31%");

  await endDownload(page, WHY);
  await expect(page.getByRole("alert")).toHaveText(WHY);
  await expect(button(page, CLONE)).toBeEnabled();

  // A retry clears the old error at once.
  await button(page, CLONE).click();
  await expect(downloading(page)).toHaveText("Downloading… 0%");
  await expect(page.getByRole("alert")).toHaveCount(0);
  expect(await invokesOf(page, "download_clone_models")).toHaveLength(2);
  expect(consoleErrors, consoleErrors.join("\n")).toEqual([]);
});

test("a reload mid-download picks the download up, and its done event ends it", async ({
  page,
  consoleErrors,
}) => {
  await button(page, CLONE).click();
  await emitDownloadProgress(page, { received: 2, total: 5 });
  await expect(downloading(page)).toHaveText("Downloading… 40%");

  await page.reload();
  await expect(page.locator(".titlebar-wordmark")).toBeVisible();
  await nav(page, "Speak").click();
  // From the read alone: this page never started a download.
  await expect(downloading(page)).toHaveText("Downloading… 40%");
  await expect(downloading(page)).toBeDisabled();
  expect(await invokesOf(page, "model_download_status")).toHaveLength(1);
  expect(await invokesOf(page, "download_clone_models")).toEqual([]);

  await emitDownloadProgress(page, { file: 2, received: 3, total: 5 });
  await expect(downloading(page)).toHaveText("Downloading… 60%");
  // The first page's answer died with it; the done event alone ends it here.
  await endDownload(page);
  await expect(button(page, VOXCPM)).toBeEnabled();
  await expect(downloading(page)).toHaveCount(0);
  expect(await invokesOf(page, "download_clone_models")).toEqual([]);
  expect(consoleErrors, consoleErrors.join("\n")).toEqual([]);
});
