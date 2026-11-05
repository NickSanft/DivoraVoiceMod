// E2E for Talk to OBS (v1.61.0), Tauri mocked (see e2e/tauri-mock.ts).
// Divora's own client talks to OBS's WebSocket server from Rust, and none of
// it runs here. These pin the page's side against the backend's wire
// (src/data/obs.fixture.json): a profile that never turned OBS on sends no
// OBS command and doesn't listen for one; Settings → OBS sends what its
// controls say and shows each status the backend sends, its announcer
// saying only the edges; the password goes into its one command and
// nowhere else; the pickers fill from `obs_inventory` when opened; a
// timer's OBS fields and actions reach the backend in `sync_timers`; and a
// reload keeps all of it, the backend's link included.
//
// The mock's link only gets as far as `connecting`: every later state is
// one a spec sends with emitObsStatus, as the link's driver would.

import type { Page } from "@playwright/test";
import obsWire from "../src/data/obs.fixture.json" with { type: "json" };
import {
  test,
  expect,
  answerObsInventory,
  emitObsStatus,
  emitTimers,
  holdObsInventory,
  invokesOf,
  listenCount,
  MOCK_OBS_INVENTORY,
  mockObsStatus,
  obsStatusSample,
  setMockObsInventory,
  TIMER_SAMPLES,
} from "./tauri-mock";

// A skip line's time is the PC's own hour and minute: pinned here.
test.use({ locale: "en-GB", timezoneId: "UTC" });

const OBS_CMDS = [
  "obs_configure",
  "obs_reconnect",
  "obs_set_password",
  "obs_forget_password",
  "obs_status",
  "obs_inventory",
];
const OBS_EVENT = obsWire.obsWire.events.status;
/** The OBS commands this page has sent, in order. */
const obsCalls = async (page: Page) => (await invokesOf(page)).filter((i) => OBS_CMDS.includes(i.cmd));
/** The four that change something (the store's OBS queue), in order. */
const obsWrites = async (page: Page) =>
  (await obsCalls(page)).filter((i) => i.cmd !== "obs_status" && i.cmd !== "obs_inventory");
const CONFIGURE = (enabled: boolean, host = "127.0.0.1", port = 4455) => ({
  cmd: "obs_configure",
  args: { config: { enabled, host, port } },
});
const STATUS = { cmd: "obs_status", args: {} };
const INVENTORY = { cmd: "obs_inventory", args: {} };
const RECONNECT = { cmd: "obs_reconnect", args: {} };
const FORGET = { cmd: "obs_forget_password", args: {} };
const SET_PASSWORD = (password: string) => ({ cmd: "obs_set_password", args: { password } });
const SYNC = (timers: unknown[]) => ({ cmd: "sync_timers", args: { timers } });

/** What the page stores: `divora.obs` on at OBS's default address, and BRB
 *  (a countdown) and Uptime (a count-up), as the fixture's snapshots name
 *  them, as each is synced. */
const ON = { enabled: true, host: "127.0.0.1", port: 4455 };
const BRB = { id: "timer-1", name: "BRB", kind: "countdown", durationMs: 300_000 };
const UPTIME = { id: "timer-2", name: "Uptime", kind: "countup", durationMs: 300_000 };
const TIMERS = { version: 1, timers: [{ ...BRB, hotkeys: {} }, { ...UPTIME, hotkeys: {} }] };
const [GAME, BRB_SCENE, CHATTING] = MOCK_OBS_INVENTORY.scenes as [
  { uuid: string; name: string },
  { uuid: string; name: string },
  { uuid: string; name: string },
];
const TIMER_TEXT = MOCK_OBS_INVENTORY.textSources[0]!;
const CONNECTED = obsStatusSample("connected");

// Settings → OBS's words (design §9), as the page must say them.
const INTRO =
  "Divora can show a timer's time in an OBS text source, switch OBS to a scene for a break and back, and start a count-up when you go live or start recording. It talks to the WebSocket server built into OBS.";
const OBS_SIDE =
  "OBS ships with its WebSocket server off. In OBS: Tools → WebSocket Server Settings → Enable WebSocket server, then Show Connect Info for the password.";
const NONE_SAVED = "No password saved. If OBS's Enable Authentication is on, it needs one.";
const SAVED = "A password is saved, stored encrypted for your Windows account. OBS keeps its own copy in its settings.";
const LINES = {
  connected: "Connected to OBS 30.2.2 (WebSocket 5.5.2)",
  needsPassword:
    "OBS is asking for a password. Paste it from OBS's Show Connect Info, then Save password.",
  authFailed: "OBS rejected the password. Paste it again from OBS's Show Connect Info, then Save password.",
  tooOld: "Update OBS, then press Try again. Its WebSocket server is 5.3.3, and Divora needs 5.4.0 or later.",
  kicked: "OBS disconnected Divora from its list of connections. Divora won't reconnect on its own.",
};
const REFUSED = "Can't reach OBS at 127.0.0.1:4455. Nothing is listening there. Is OBS open, with its WebSocket server on?";

const nav = (page: Page, label: string) => page.locator(".sidebar-nav", { hasText: label });
const section = (page: Page, eyebrow: string) =>
  page.locator("section", { has: page.locator(".eyebrow", { hasText: new RegExp(`^${eyebrow}$`) }) });
const obs = (page: Page) => section(page, "OBS");
const hourglass = (page: Page) => section(page, "Hourglass");
const card = (page: Page, id: string) => page.locator(`[data-timer-card="${id}"]`);
const statusLine = (page: Page) => obs(page).getByTestId("obs-status-line");
const toggle = (page: Page) => obs(page).getByRole("switch", { name: "Connect to OBS" });
/** What the page has stored under `key`. */
const stored = (page: Page, key: string) =>
  page.evaluate((k) => JSON.parse(localStorage.getItem(k) ?? "null") as unknown, key);
/** The line a field's `aria-describedby` names: its refusal. */
async function reasonOf(page: Page, field: ReturnType<Page["locator"]>) {
  const id = await field.getAttribute("aria-describedby");
  if (!id) throw new Error("the field names no reason");
  return page.locator(`[id="${id}"]`);
}

/** The app booted with `seed` in localStorage (set only where absent, so
 *  what the page saves survives a reload). */
async function boot(page: Page, seed: Record<string, unknown> = {}): Promise<void> {
  await page.addInitScript((s) => {
    for (const [k, v] of Object.entries(s)) {
      if (localStorage.getItem(k) === null) localStorage.setItem(k, JSON.stringify(v));
    }
  }, seed);
  await page.goto("/");
  await expect(page.locator(".titlebar-wordmark")).toBeVisible();
}

/** Every text the OBS section's announcer put up from now on (it empties
 *  itself between), in order: what a screen reader was handed. */
async function recordAnnouncer(page: Page): Promise<void> {
  await page.evaluate(() => {
    const el = document.querySelector('[data-testid="obs-announcer"]');
    if (!el) throw new Error("no OBS announcer");
    const said: string[] = [];
    (window as unknown as { __E2E_OBS_SAID__: string[] }).__E2E_OBS_SAID__ = said;
    let last = el.textContent ?? "";
    new MutationObserver(() => {
      const now = el.textContent ?? "";
      if (now !== last && now !== "") said.push(now);
      last = now;
    }).observe(el, { childList: true, characterData: true, subtree: true });
  });
}
const spoken = (page: Page) =>
  page.evaluate(() => [...(window as unknown as { __E2E_OBS_SAID__: string[] }).__E2E_OBS_SAID__]);
/** Two frames: long enough for the announcer to put up anything it was
 *  given (it waits one). */
const frames = (page: Page) =>
  page.evaluate(() => new Promise<void>((r) => requestAnimationFrame(() => requestAnimationFrame(() => r()))));

test.describe("Talk to OBS (v1.61)", () => {
  test("the mock refuses a status, a skip or an inventory the wire doesn't have", async ({ page }) => {
    // Each would pass for one the page ignored: it drops what it can't read.
    await expect(emitObsStatus(page, { stat: "connected" } as never)).rejects.toThrow("the status has stat");
    await expect(emitObsStatus(page, { state: "online" } as never)).rejects.toThrow('state "online"');
    await expect(
      emitObsStatus(page, { lastSkip: { op: "sceneBack", why: "queueFull", name: null, timer: "timer-1", atMs: 0 } }),
    ).rejects.toThrow("lastSkip's sceneBack has why queueFull");
    await expect(
      emitObsStatus(page, { textSkip: { why: "failed", atMs: 0, timer: "timer-1" } as never }),
    ).rejects.toThrow("textSkip has timer");
    await expect(
      setMockObsInventory(page, {
        ...MOCK_OBS_INVENTORY,
        textSources: [{ uuid: "u", name: "Image", kind: "image_source" as never, readsFromFile: false }],
      }),
    ).rejects.toThrow("textSources[0] has kind image_source");
  });

  const quiet: Array<[string, Record<string, unknown>]> = [
    ["without divora.obs", {}],
    ["with divora.obs off", { "divora.obs": { ...ON, enabled: false } }],
  ];
  for (const [what, seed] of quiet) {
    test(`a profile ${what} sends no OBS command and doesn't listen for OBS`, async ({ page, consoleErrors }) => {
      // The list here is the wire's.
      expect([...OBS_CMDS].sort()).toEqual(Object.values(obsWire.obsWire.commands).sort());
      await boot(page, seed);
      // Through the rest of the boot: the enumeration and the hotkey sync.
      // Settings isn't opened: its OBS section reads the status once while
      // it's on screen.
      await expect.poll(async () => (await invokesOf(page, "list_audio_input_devices")).length).toBeGreaterThan(0);
      await page.waitForTimeout(300);
      expect(await obsCalls(page)).toEqual([]);
      expect(await listenCount(page, OBS_EVENT)).toBe(0);
      expect(await stored(page, "divora.obs")).toEqual(seed["divora.obs"] ?? null);
      expect(consoleErrors, consoleErrors.join("\n")).toEqual([]);
    });
  }

  test("the card shows its fields while off; Connect to OBS connects to 127.0.0.1:4455, Off stops it, and an address is only checked while off", async ({
    page,
    consoleErrors,
  }) => {
    await boot(page);
    await nav(page, "Settings").click();
    await expect(obs(page)).toContainText(INTRO);
    await expect(obs(page).getByTestId("obs-explainer")).toContainText(OBS_SIDE);
    const address = obs(page).getByRole("textbox", { name: "OBS address" });
    const port = obs(page).getByRole("textbox", { name: "OBS port" });
    const password = obs(page).getByLabel("OBS password");
    await expect(address).toHaveValue("127.0.0.1");
    await expect(port).toHaveValue("4455");
    await expect(password).toHaveValue("");
    await expect(password).toHaveAttribute("type", "password");
    await expect(toggle(page)).toHaveAttribute("aria-checked", "false");
    await expect(obs(page).getByTestId("obs-status")).toHaveCount(0);
    // On screen while off: one read, for whether a password is saved. No
    // listener, no connection.
    await expect.poll(() => obsCalls(page)).toEqual([STATUS]);
    await expect(obs(page)).toContainText(NONE_SAVED);
    expect(await listenCount(page, OBS_EVENT)).toBe(0);

    // On: saved, listened for, then the configure, which the backend
    // answers with the link starting.
    await toggle(page).click();
    await expect.poll(() => obsCalls(page)).toEqual([STATUS, CONFIGURE(true)]);
    expect(await listenCount(page, OBS_EVENT)).toBe(1);
    await expect(toggle(page)).toHaveAttribute("aria-checked", "true");
    await expect(statusLine(page)).toHaveText("Connecting to 127.0.0.1:4455…");
    expect(await stored(page, "divora.obs")).toEqual(ON);
    // The link connects, and the section on screen reads OBS's list once
    // for it (again only at the next `inventoryRev`).
    await emitObsStatus(page, CONNECTED);
    await expect(statusLine(page)).toHaveText(LINES.connected);
    await expect.poll(() => obsCalls(page)).toEqual([STATUS, CONFIGURE(true), INVENTORY]);

    // Off: the link stops, and the line goes with it.
    await toggle(page).click();
    await expect.poll(() => obsCalls(page)).toEqual([STATUS, CONFIGURE(true), INVENTORY, CONFIGURE(false)]);
    await expect(obs(page).getByTestId("obs-status")).toHaveCount(0);
    expect(await stored(page, "divora.obs")).toEqual({ ...ON, enabled: false });
    expect((await mockObsStatus(page)).state).toBe("off");

    // An address while off: the backend checks it and connects nothing,
    // and only one it took is saved.
    await address.fill("192.168.1.20");
    await address.press("Enter");
    await expect
      .poll(() => obsWrites(page))
      .toEqual([CONFIGURE(true), CONFIGURE(false), CONFIGURE(false, "192.168.1.20")]);
    await expect.poll(() => stored(page, "divora.obs")).toEqual({ ...ON, enabled: false, host: "192.168.1.20" });
    await address.fill("obs.local");
    await address.press("Enter");
    await expect(address).toHaveAttribute("aria-invalid", "true");
    await expect(await reasonOf(page, address)).toHaveText("Type an IP address, like 127.0.0.1, or localhost.");
    expect((await obsCalls(page)).at(-1)).toEqual(CONFIGURE(false, "obs.local"));
    expect(await stored(page, "divora.obs")).toEqual({ ...ON, enabled: false, host: "192.168.1.20" });
    // Esc puts the stored one back.
    await address.press("Escape");
    await expect(address).toHaveValue("192.168.1.20");
    await expect(address).not.toHaveAttribute("aria-invalid", "true");
    // Never on again: the only configure that connects is the switch's.
    expect((await obsCalls(page)).filter((i) => i.cmd === "obs_configure" && JSON.stringify(i.args).includes('"enabled":true'))).toEqual([
      CONFIGURE(true),
    ]);
    expect(consoleErrors, consoleErrors.join("\n")).toEqual([]);
  });

  test("Save password works while off: refusals show under the field, the field empties, and the password is nowhere but its command; Forget asks twice", async ({
    page,
    consoleErrors,
  }) => {
    const logged: string[] = [];
    page.on("console", (m) => logged.push(m.text()));
    await boot(page);
    await nav(page, "Settings").click();
    const password = obs(page).getByLabel("OBS password");
    const save = obs(page).getByRole("button", { name: "Save password" });
    await expect(obs(page)).toContainText(NONE_SAVED);

    // Nothing pasted: the backend's words under the field.
    await save.click();
    await expect(password).toHaveAttribute("aria-invalid", "true");
    await expect(await reasonOf(page, password)).toHaveText("Paste a password first.");
    // Too long: refused, and the field empties all the same.
    const long = "x".repeat(obsWire.limits.maxPasswordChars + 1);
    await password.fill(long);
    await expect(password).not.toHaveAttribute("aria-invalid", "true");
    await save.click();
    await expect(await reasonOf(page, password)).toHaveText("The password can be up to 256 characters.");
    await expect(password).toHaveValue("");
    // A space at either end: said, since OBS's has to have it too.
    await password.fill(" spaced");
    await expect(obs(page)).toContainText(
      "This starts or ends with a space. OBS's password has to as well, or OBS will reject it.",
    );

    // The one OBS shows, saved by Enter.
    const SECRET = "e2e-Obs-Secret-7Qx";
    await password.fill(SECRET);
    await password.press("Enter");
    await expect(password).toHaveValue("");
    await expect(obs(page)).toContainText(SAVED);
    await expect(password).not.toHaveAttribute("aria-invalid", "true");
    expect(await obsWrites(page)).toEqual([SET_PASSWORD(""), SET_PASSWORD(long), SET_PASSWORD(SECRET)]);
    expect((await mockObsStatus(page)).hasPassword).toBe(true);
    // Nowhere else: not on the page, not in either storage (the mock
    // backend's status included), not in a log line.
    expect(await page.content()).not.toContain(SECRET);
    const fields = await page.evaluate(() => [...document.querySelectorAll("input")].map((i) => i.value));
    expect(fields.join("\n")).not.toContain(SECRET);
    const storage = await page.evaluate(() =>
      [localStorage, sessionStorage].flatMap((s) => Object.keys(s).map((k) => `${k}=${s.getItem(k)}`)).join("\n"),
    );
    expect(storage).not.toContain(SECRET);
    expect(JSON.stringify(await mockObsStatus(page))).not.toContain(SECRET);
    expect(logged.join("\n")).not.toContain(SECRET);

    // Forget: asks first, then goes, still off.
    await obs(page).getByRole("button", { name: "Forget password" }).click();
    const confirm = obs(page).getByRole("button", { name: "Click again to forget the OBS password" });
    await expect(confirm).toHaveText("Forget?");
    expect((await obsWrites(page)).at(-1)).toEqual(SET_PASSWORD(SECRET));
    await confirm.click();
    await expect(obs(page)).toContainText(NONE_SAVED);
    await expect(obs(page).getByRole("button", { name: /Forget/ })).toHaveCount(0);
    expect(await obsWrites(page)).toEqual([SET_PASSWORD(""), SET_PASSWORD(long), SET_PASSWORD(SECRET), FORGET]);
    // Off throughout: nothing connected, and nothing listens.
    expect(await listenCount(page, OBS_EVENT)).toBe(0);
    expect(await stored(page, "divora.obs")).toBeNull();
    expect(consoleErrors, consoleErrors.join("\n")).toEqual([]);
  });

  test("each status the backend sends: its line and button, both warnings, a skip on the section and its timer's card; the announcer says only the edges", async ({
    page,
    consoleErrors,
  }) => {
    await boot(page, { "divora.timers": TIMERS });
    await nav(page, "Settings").click();
    await expect.poll(() => obsCalls(page)).toEqual([STATUS]);
    await recordAnnouncer(page);
    const tryAgain = obs(page).getByRole("button", { name: "Try again" });
    const connectAgain = obs(page).getByRole("button", { name: "Connect again" });
    await toggle(page).click();
    await expect(statusLine(page)).toHaveText("Connecting to 127.0.0.1:4455…");
    // The line is plain text: its countdown would talk every second.
    await expect(page.locator('[aria-live] [data-testid="obs-status-line"], [role="status"] [data-testid="obs-status-line"]')).toHaveCount(0);
    await expect(statusLine(page)).not.toHaveAttribute("aria-live");

    // Connected, then live and recording paused.
    const live = await emitObsStatus(page, CONNECTED);
    await expect(statusLine(page)).toHaveText(LINES.connected);
    await expect.poll(() => spoken(page)).toEqual(["Connected to OBS."]);
    await expect(tryAgain).toHaveCount(0);
    await expect(obs(page).getByTestId("obs-warning")).toHaveCount(0);
    await emitObsStatus(page, { stream: "live", record: "paused" });
    await expect(statusLine(page)).toHaveText(`${LINES.connected} · live · recording paused`);
    // An older status, arriving late, changes nothing.
    await emitObsStatus(page, { ...obsStatusSample("connecting"), seq: live.seq });
    await frames(page);
    await expect(statusLine(page)).toHaveText(`${LINES.connected} · live · recording paused`);

    // Lost: said once, and a retry after a retry says nothing more.
    await emitObsStatus(page, { ...obsStatusSample("retrying", "refused"), lastSkip: null, retryAtMs: Date.now() - 1_000 });
    await expect(statusLine(page)).toHaveText(`${REFUSED} Trying again now.`);
    await expect.poll(() => spoken(page)).toEqual(["Connected to OBS.", "Lost the connection to OBS. Divora keeps trying."]);
    await emitObsStatus(page, { retryAtMs: Date.now() + 60_000 });
    await expect(statusLine(page)).toHaveText(new RegExp(`^${REFUSED.replace(/[.?]/g, "\\$&")} Trying again in \\d+ s\\.$`));
    await frames(page);
    expect(await spoken(page)).toHaveLength(2);
    // Try again while retrying: an attempt now, nothing to show yet.
    await tryAgain.click();
    await expect.poll(() => obsWrites(page)).toEqual([CONFIGURE(true), RECONNECT]);

    // The stop states, each with its line, its button and its words said
    // (each waited for: words given within one frame are said as one).
    const stops = async () => (await spoken(page)).slice(2);
    await emitObsStatus(page, obsStatusSample("needsPassword"));
    await expect(statusLine(page)).toHaveText(LINES.needsPassword);
    await expect.poll(stops).toEqual([LINES.needsPassword]);
    await expect(tryAgain).toBeVisible();
    await expect(obs(page)).toContainText(NONE_SAVED);
    await emitObsStatus(page, obsStatusSample("authFailed"));
    await expect(statusLine(page)).toHaveText(LINES.authFailed);
    await expect.poll(stops).toEqual([LINES.needsPassword, LINES.authFailed]);
    // Try again after a stop: the link starts over.
    await tryAgain.click();
    await expect(statusLine(page)).toHaveText("Connecting to 127.0.0.1:4455…");
    await emitObsStatus(page, obsStatusSample("tooOld"));
    await expect(statusLine(page)).toHaveText(LINES.tooOld);
    await expect.poll(stops).toEqual([LINES.needsPassword, LINES.authFailed, LINES.tooOld]);
    await expect(tryAgain).toBeVisible();
    await emitObsStatus(page, obsStatusSample("kicked"));
    await expect(statusLine(page)).toHaveText(LINES.kicked);
    await expect.poll(stops).toEqual([LINES.needsPassword, LINES.authFailed, LINES.tooOld, LINES.kicked]);
    await expect(tryAgain).toHaveCount(0);
    // From the keyboard: Connect again goes once it's connecting, and hands
    // its focus to the status line beside it, not to the page.
    await connectAgain.focus();
    await page.keyboard.press("Enter");
    await expect(statusLine(page)).toHaveText("Connecting to 127.0.0.1:4455…");
    await expect(connectAgain).toHaveCount(0);
    await expect(statusLine(page)).toBeFocused();
    await frames(page);
    expect(await spoken(page)).toHaveLength(6);
    expect(await obsWrites(page)).toEqual([CONFIGURE(true), RECONNECT, RECONNECT, RECONNECT]);

    // OBS on another PC, asking for no password: both warnings.
    await emitObsStatus(page, { ...CONNECTED, host: "192.168.1.20", loopback: false, authRequired: false });
    await expect(statusLine(page)).toHaveText(LINES.connected);
    await expect.poll(async () => (await spoken(page)).slice(6)).toEqual(["Connected to OBS."]);
    await expect(obs(page).getByTestId("obs-warning")).toHaveText([
      "192.168.1.20 is a network address, so Divora talks to OBS over the network, without encryption. Anyone who can see that traffic can read what Divora and OBS send each other, including your sources' names and settings and your recording's file path, and could change it or take over the connection to control OBS. Your password itself is never sent, but someone who records a connection's start can test guesses at it offline, so keep it long and random.",
      "OBS isn't asking for a password, so anything that can reach it on your network can control it. Turn on Enable Authentication in OBS's WebSocket Server Settings.",
    ]);

    // BRB runs; its start's switch didn't happen, a text push timed out,
    // and a switch back waits for OBS.
    await expect.poll(() => listenCount(page, "timer-state")).toBe(1);
    await emitTimers(page, TIMER_SAMPLES.running);
    const SKIP = "18:04: Didn't switch OBS to BRB scene: OBS doesn't have that scene now.";
    const HELD = "Waiting for OBS, to switch it back after a break (up to 2 minutes).";
    await emitObsStatus(page, {
      lastSkip: { op: "scene", why: "notFound", name: "BRB scene", timer: "timer-1", atMs: Date.UTC(2026, 10, 3, 18, 4) },
      textSkip: { why: "timedOut", atMs: Date.UTC(2026, 10, 3, 18, 5) },
      heldBack: 1,
    });
    await expect(obs(page).getByTestId("obs-skip-line")).toHaveText(SKIP);
    await expect(obs(page).getByTestId("obs-text-skip-line")).toHaveText(
      "18:05: OBS didn't take a timer's text in time. Divora tries again in a moment.",
    );
    await expect(obs(page).getByTestId("obs-held-back")).toHaveText(HELD);
    await expect(card(page, "timer-1").getByTestId("timer-obs-skip")).toHaveText(SKIP);
    await expect(card(page, "timer-2").getByTestId("timer-obs-skip")).toHaveCount(0);
    await expect
      .poll(async () => (await spoken(page)).slice(6))
      .toEqual(["Connected to OBS.", `${SKIP} ${HELD}`]);
    expect(consoleErrors, consoleErrors.join("\n")).toEqual([]);
  });

  test("Show in OBS and Follow OBS on the cards: the picker loads OBS's sources when opened, one that reads a file or another timer shows can't be picked, and each pick syncs once", async ({
    page,
    consoleErrors,
  }) => {
    await boot(page, { "divora.timers": TIMERS, "divora.obs": ON });
    // The boot: the timers' sync, then OBS's configure, which starts the link.
    await expect.poll(() => invokesOf(page, "sync_timers")).toEqual([SYNC([BRB, UPTIME])]);
    await expect.poll(() => obsCalls(page)).toEqual([CONFIGURE(true)]);
    expect(await listenCount(page, OBS_EVENT)).toBe(1);
    await holdObsInventory(page);
    await nav(page, "Settings").click();
    await expect.poll(() => listenCount(page, "timer-state")).toBe(1);
    await emitTimers(page, TIMER_SAMPLES.idle);
    const uptime = card(page, "timer-2");
    const picker = uptime.getByRole("button", { name: /^Show in OBS for Uptime: / });
    await expect(picker).toHaveAccessibleName("Show in OBS for Uptime: None");
    // Only a count-up follows OBS.
    await expect(card(page, "timer-1").getByRole("radiogroup")).toHaveCount(0);

    // Connected: the page asks for OBS's sources, and says so while OBS
    // reads them.
    const reads = async () => (await obsCalls(page)).filter((i) => i.cmd === "obs_inventory").length;
    await emitObsStatus(page, CONNECTED);
    await expect(picker).toContainText("Loading OBS sources…");
    await expect.poll(reads).toBe(1);
    await picker.click();
    const list = page.getByRole("listbox", { name: "Show in OBS for Uptime" });
    await expect(list.getByRole("option")).toHaveCount(1);
    // The open joined the read on its way; its answer fills the list.
    expect(await answerObsInventory(page)).toBe(1);
    expect(await reads()).toBe(1);
    await expect(list.getByRole("option")).toHaveCount(4);
    await expect(picker).not.toContainText("Loading OBS sources…");
    await expect(list.getByRole("option", { name: /^Timer text/ })).toContainText("Text (GDI+)");
    await expect(list.getByRole("option", { name: /^Uptime text/ })).toContainText("Text (FreeType 2)");
    // A source that reads a file is offered, and says why it can't be
    // picked; a click on it picks nothing.
    const file = list.getByRole("option", { name: /^Clock file/ });
    await expect(file).toHaveAttribute("aria-disabled", "true");
    await expect(file).toContainText(
      "reads a file — untick Read from file in its Properties in OBS to show a timer here",
    );
    await file.click({ force: true });
    await expect(list).toBeVisible();
    await page.waitForTimeout(200);
    expect(await invokesOf(page, "sync_timers")).toEqual([SYNC([BRB, UPTIME])]);

    // Timer text: its uuid goes to the backend, its name stays here.
    await list.getByRole("option", { name: /^Timer text/ }).click();
    await expect(list).toHaveCount(0);
    const shown = { ...UPTIME, obsText: { uuid: TIMER_TEXT.uuid } };
    await expect.poll(() => invokesOf(page, "sync_timers")).toEqual([SYNC([BRB, UPTIME]), SYNC([BRB, shown])]);
    await expect(picker).toHaveAccessibleName("Show in OBS for Uptime: Timer text");
    expect(await stored(page, "divora.timers")).toMatchObject({
      timers: [{ id: "timer-1" }, { id: "timer-2", obsText: { uuid: TIMER_TEXT.uuid, name: "Timer text" } }],
    });

    // BRB's picker reads OBS's list again as it opens, and offers Timer
    // text as shown by Uptime.
    const brbPicker = card(page, "timer-1").getByRole("button", { name: /^Show in OBS for BRB: / });
    await brbPicker.click();
    await expect.poll(reads).toBe(2);
    const brbList = page.getByRole("listbox", { name: "Show in OBS for BRB" });
    const taken = brbList.getByRole("option", { name: /^Timer text/ });
    await expect(taken).toHaveAttribute("aria-disabled", "true");
    await expect(taken).toContainText("shown by Uptime");
    await brbPicker.click();
    await expect(brbList).toHaveCount(0);

    // Follow OBS: a named group of three.
    const follow = uptime.getByRole("radiogroup", { name: "Follow OBS for Uptime" });
    await expect(follow.getByRole("radio")).toHaveText(["Off", "Streaming", "Recording"]);
    await expect(follow.getByRole("radio", { name: "Off" })).toHaveAttribute("aria-checked", "true");
    await follow.getByRole("radio", { name: "Recording" }).click();
    await expect(follow.getByRole("radio", { name: "Recording" })).toHaveAttribute("aria-checked", "true");
    await expect
      .poll(() => invokesOf(page, "sync_timers"))
      .toEqual([SYNC([BRB, UPTIME]), SYNC([BRB, shown]), SYNC([BRB, { ...shown, followObs: "record" }])]);
    expect(await stored(page, "divora.timers")).toMatchObject({ timers: [{}, { followObs: "record" }] });
    expect(consoleErrors, consoleErrors.join("\n")).toEqual([]);
  });

  test("Switch OBS scene and Switch OBS back from the action editor: each waits for OBS control, the scene comes from OBS when it's picked, and each change syncs once", async ({
    page,
    consoleErrors,
  }) => {
    await boot(page, { "divora.timers": TIMERS });
    await nav(page, "Settings").click();
    await expect.poll(() => listenCount(page, "timer-state")).toBe(1);
    await emitTimers(page, TIMER_SAMPLES.idle);
    const brb = card(page, "timer-1");
    const addAtStart = brb.getByRole("button", { name: "Add action at the start of BRB" });
    const addAtZero = brb.getByRole("button", { name: "Add action at zero of BRB" });
    const startChoices = page.getByRole("listbox", { name: "Actions to add at the start of BRB" });
    const zeroChoices = page.getByRole("listbox", { name: "Actions to add at zero of BRB" });

    // Off: offered, each saying what it waits for; Switch OBS back only at zero.
    await addAtStart.click();
    await expect(
      startChoices.getByRole("option", { name: "Switch OBS scene — turn on Settings → OBS first", exact: true }),
    ).toHaveAttribute("aria-disabled", "true");
    await expect(startChoices.getByRole("option", { name: /^Switch OBS back/ })).toHaveCount(0);
    await addAtStart.click();
    await addAtZero.click();
    await expect(
      zeroChoices.getByRole("option", { name: "Switch OBS back — turn on Settings → OBS first", exact: true }),
    ).toHaveAttribute("aria-disabled", "true");
    await addAtZero.click();
    // On, not connected yet: Switch OBS scene needs OBS's scenes now.
    await toggle(page).click();
    await expect(statusLine(page)).toHaveText("Connecting to 127.0.0.1:4455…");
    await addAtStart.click();
    await expect(
      startChoices.getByRole("option", { name: "Switch OBS scene — Divora isn't connected to OBS", exact: true }),
    ).toHaveAttribute("aria-disabled", "true");
    await addAtStart.click();

    // Connected, OBS still reading its scenes: picking Switch OBS scene
    // waits for them, then adds one other than the live scene (Game).
    await holdObsInventory(page);
    await emitObsStatus(page, CONNECTED);
    await addAtStart.click();
    await startChoices.getByRole("option", { name: "Switch OBS scene", exact: true }).click();
    await page.waitForTimeout(200);
    expect(await invokesOf(page, "sync_timers")).toEqual([SYNC([BRB, UPTIME])]);
    expect(await answerObsInventory(page)).toBe(1);
    const toBrbScene = { kind: "obs", op: "scene", sceneUuid: BRB_SCENE.uuid, sceneName: BRB_SCENE.name };
    await expect
      .poll(() => invokesOf(page, "sync_timers"))
      .toEqual([SYNC([BRB, UPTIME]), SYNC([{ ...BRB, onStart: [toBrbScene] }, UPTIME])]);
    const scenePicker = brb.getByRole("button", { name: /^BRB OBS scene at the start: / });
    await expect(scenePicker).toHaveAccessibleName("BRB OBS scene at the start: BRB scene");
    await expect(brb).toContainText(
      "Switches OBS's Program scene, in Studio Mode too. If Divora isn't connected to OBS then, or OBS is busy for more than 5 seconds before Divora can ask it, it doesn't switch. Once asked, a slow OBS can still switch late.",
    );
    const NO_BACK =
      "This switches OBS's scene when it starts, and At zero has no Switch OBS back, so a reset or a delete leaves OBS on that scene, as does its end unless At zero switches scene. Add Switch OBS back at zero.";
    await expect(brb).toContainText(NO_BACK);

    // Its scene, picked from OBS's list.
    await scenePicker.click();
    const scenes = page.getByRole("listbox", { name: "BRB OBS scene at the start" });
    await expect(scenes.getByRole("option")).toHaveText([GAME.name, BRB_SCENE.name, CHATTING.name]);
    await scenes.getByRole("option", { name: CHATTING.name }).click();
    const toChatting = { kind: "obs", op: "scene", sceneUuid: CHATTING.uuid, sceneName: CHATTING.name };
    await expect.poll(async () => (await invokesOf(page, "sync_timers")).slice(2)).toEqual([
      SYNC([{ ...BRB, onStart: [toChatting] }, UPTIME]),
    ]);
    await expect(scenePicker).toHaveAccessibleName("BRB OBS scene at the start: Just Chatting");

    // At zero: Switch OBS back, and the warning goes.
    await addAtZero.click();
    await zeroChoices.getByRole("option", { name: "Switch OBS back", exact: true }).click();
    const back = { kind: "obs", op: "sceneBack" };
    await expect.poll(async () => (await invokesOf(page, "sync_timers")).slice(2)).toEqual([
      SYNC([{ ...BRB, onStart: [toChatting] }, UPTIME]),
      SYNC([{ ...BRB, onStart: [toChatting], onFire: [back] }, UPTIME]),
    ]);
    await expect(brb.locator('[data-timer-list="onFire"]')).toContainText("Switch OBS back");
    await expect(brb).not.toContainText(NO_BACK);
    await expect(brb).toContainText(
      "Switches back to the scene OBS was on before this timer's start, but only if OBS is still on the scene the start switched to. If this timer started on the scene another timer switched OBS to, and that timer's Switch OBS back came first and didn't switch, this one goes back to the scene before that timer's start instead. If Divora isn't connected to OBS then, it switches back if it reconnects within 2 minutes; after that, OBS stays where it is.",
    );
    expect(await stored(page, "divora.timers")).toMatchObject({
      timers: [{ id: "timer-1", onStart: [toChatting], onFire: [back] }, { id: "timer-2" }],
    });
    expect(consoleErrors, consoleErrors.join("\n")).toEqual([]);
  });

  test("Add break (BRB) with a break scene saves its four actions and syncs once", async ({ page, consoleErrors }) => {
    await boot(page, { "divora.obs": ON });
    await nav(page, "Settings").click();
    const row = hourglass(page).getByTestId("obs-break-row");
    const picker = row.getByRole("button", { name: /^OBS scene for breaks: / });
    await expect(picker).toHaveAccessibleName("OBS scene for breaks: Don't switch");
    await emitObsStatus(page, CONNECTED);
    await picker.click();
    const scenes = page.getByRole("listbox", { name: "OBS scene for breaks" });
    await expect(scenes.getByRole("option")).toHaveText(["Don't switch", GAME.name, BRB_SCENE.name, CHATTING.name]);
    await scenes.getByRole("option", { name: BRB_SCENE.name }).click();
    await expect(picker).toHaveAccessibleName("OBS scene for breaks: BRB scene");
    // Stored only: it reaches the backend in the next break's actions.
    expect(await stored(page, "divora.obs")).toEqual({ ...ON, breakScene: BRB_SCENE });
    expect(await invokesOf(page, "sync_timers")).toEqual([]);

    await hourglass(page).getByRole("button", { name: "Add break (BRB)" }).click();
    const BREAK = {
      ...BRB,
      onStart: [
        { kind: "setMuted", muted: true },
        { kind: "obs", op: "scene", sceneUuid: BRB_SCENE.uuid, sceneName: BRB_SCENE.name },
      ],
      onFire: [
        { kind: "setMuted", muted: false },
        { kind: "obs", op: "sceneBack" },
      ],
    };
    await expect.poll(() => invokesOf(page, "sync_timers")).toEqual([SYNC([BREAK])]);
    expect(await stored(page, "divora.timers")).toEqual({ version: 1, timers: [{ ...BREAK, hotkeys: {} }] });
    await page.waitForTimeout(300);
    expect(await invokesOf(page, "sync_timers")).toHaveLength(1);
    expect(consoleErrors, consoleErrors.join("\n")).toEqual([]);
  });

  test("a reload keeps OBS on, the saved password, each timer's OBS fields and the backend's link", async ({
    page,
    consoleErrors,
  }) => {
    await boot(page, { "divora.timers": TIMERS, "divora.obs": ON });
    await nav(page, "Settings").click();
    await emitObsStatus(page, CONNECTED);
    await expect(statusLine(page)).toHaveText(LINES.connected);
    await expect.poll(() => listenCount(page, "timer-state")).toBe(1);
    await emitTimers(page, TIMER_SAMPLES.idle);
    // BRB shows in Timer text; Uptime follows the stream; a password saved
    // (the link restarts with it, and connects).
    await card(page, "timer-1").getByRole("button", { name: /^Show in OBS for BRB: / }).click();
    await page.getByRole("listbox", { name: "Show in OBS for BRB" }).getByRole("option", { name: /^Timer text/ }).click();
    await card(page, "timer-2")
      .getByRole("radiogroup", { name: "Follow OBS for Uptime" })
      .getByRole("radio", { name: "Streaming" })
      .click();
    const synced = [{ ...BRB, obsText: { uuid: TIMER_TEXT.uuid } }, { ...UPTIME, followObs: "stream" }];
    await expect.poll(async () => (await invokesOf(page, "sync_timers")).at(-1)).toEqual(SYNC(synced));
    await obs(page).getByLabel("OBS password").fill("e2e-reload-pw");
    await obs(page).getByRole("button", { name: "Save password" }).click();
    await expect(statusLine(page)).toHaveText("Connecting to 127.0.0.1:4455…");
    await expect(obs(page)).toContainText(SAVED);
    await emitObsStatus(page, CONNECTED);
    await expect(statusLine(page)).toHaveText(LINES.connected);
    const kept = await stored(page, "divora.timers");

    await page.reload();
    await expect(page.locator(".titlebar-wordmark")).toBeVisible();
    // The boot: the timers' sync with their OBS fields, then OBS's
    // configure, after its listener. The backend keeps the link for the
    // same address, and its answer says so.
    await expect.poll(() => invokesOf(page, "sync_timers")).toEqual([SYNC(synced)]);
    await expect.poll(() => obsCalls(page)).toEqual([CONFIGURE(true)]);
    expect(await listenCount(page, OBS_EVENT)).toBe(1);
    expect(await stored(page, "divora.obs")).toEqual(ON);
    expect(await stored(page, "divora.timers")).toEqual(kept);
    await nav(page, "Settings").click();
    await expect(toggle(page)).toHaveAttribute("aria-checked", "true");
    await expect(statusLine(page)).toHaveText(LINES.connected);
    await expect(obs(page)).toContainText(SAVED);
    await emitTimers(page, TIMER_SAMPLES.idle);
    await expect(card(page, "timer-1").getByRole("button", { name: /^Show in OBS for BRB: / })).toHaveAccessibleName(
      "Show in OBS for BRB: Timer text",
    );
    await expect(
      card(page, "timer-2").getByRole("radiogroup", { name: "Follow OBS for Uptime" }).getByRole("radio", { name: "Streaming" }),
    ).toHaveAttribute("aria-checked", "true");

    // Off: each still says what it does once OBS is on again.
    await toggle(page).click();
    await expect.poll(() => obsWrites(page)).toEqual([CONFIGURE(true), CONFIGURE(false)]);
    await expect(card(page, "timer-1")).toContainText("Shows in OBS's “Timer text” when Settings → OBS is on.");
    await expect(card(page, "timer-2")).toContainText("Follows OBS's streaming when Settings → OBS is on.");
    expect(consoleErrors, consoleErrors.join("\n")).toEqual([]);
  });
});
