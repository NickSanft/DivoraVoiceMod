import { defineConfig, devices } from "@playwright/test";

// E2E smoke tests run against the SolidJS frontend served by Vite, with
// the Tauri `invoke`/event layer mocked (see e2e/tauri-mock.ts). They
// exercise the real app wiring — router, stores, screens — end to end,
// which the Vitest unit tests (individual pieces) don't. The Rust /
// IPC / audio paths need real hardware and stay in docs/MANUAL_TESTS.md.
export default defineConfig({
  testDir: "./e2e",
  fullyParallel: true,
  forbidOnly: !!process.env.CI,
  retries: process.env.CI ? 1 : 0,
  workers: process.env.CI ? 1 : undefined,
  reporter: process.env.CI ? [["list"], ["html", { open: "never" }]] : "list",
  // Generous because the FIRST test pays Vite's one-time dep-optimize on
  // cold start (≈20s); warm navigations are sub-second. CI runs serially
  // (workers: 1) so only the first test eats it.
  timeout: 60_000,
  expect: { timeout: 10_000 },
  use: {
    baseURL: "http://localhost:1420",
    headless: true,
    trace: "on-first-retry",
  },
  projects: [{ name: "chromium", use: { ...devices["Desktop Chrome"] } }],
  // Reuse the running `pnpm dev` server locally; start a fresh one in CI.
  webServer: {
    command: "pnpm dev",
    url: "http://localhost:1420",
    reuseExistingServer: !process.env.CI,
    timeout: 120_000,
  },
});
