// Two things must hold. The panel must render changelog markdown as real
// elements (never innerHTML), and the launch check must be conservative:
// silent on a fresh install, silent in dev, and never twice for the same
// version — a what's-new that re-nags is worse than one that misses once.

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render } from "@solidjs/testing-library";

const invokeMock = vi.fn();
const getVersionMock = vi.fn<() => Promise<string>>();

vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
}));
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(async () => () => {
    /* unlisten no-op */
  }),
}));
vi.mock("@tauri-apps/plugin-shell", () => ({
  open: vi.fn(async () => undefined),
}));
vi.mock("@tauri-apps/api/app", () => ({
  getVersion: () => getVersionMock(),
}));

import type { JSX } from "solid-js";
import { AppProvider, useApp } from "../stores/app";
import { WhatsNewBanner, WhatsNewModal } from "./WhatsNew";
import { RELEASE_NOTES } from "../data/changelog";

type App = ReturnType<typeof useApp>;

const CURRENT = RELEASE_NOTES[0]!.version; // the newest release we bundle
const SEEN_KEY = "divora.whatsNewSeenVersion";
const WIZARD_KEY = "divora.wizardSeen";

function setup() {
  let captured: App | null = null;
  function Inner(): JSX.Element {
    captured = useApp();
    return (
      <>
        <WhatsNewBanner />
        <WhatsNewModal />
      </>
    );
  }
  const utils = render(() => (
    <AppProvider>
      <Inner />
    </AppProvider>
  ));
  return { ...utils, app: () => captured! };
}

describe("WhatsNew launch check", () => {
  beforeEach(() => {
    invokeMock.mockReset();
    invokeMock.mockImplementation(async (cmd: string) =>
      cmd === "list_presets" ? [] : undefined,
    );
    getVersionMock.mockReset();
    getVersionMock.mockResolvedValue(CURRENT);
    window.localStorage.clear();
    // The wizard is closed in every case except the fresh-install test.
    window.localStorage.setItem(WIZARD_KEY, "true");
  });
  afterEach(() => window.localStorage.clear());

  it("stays silent on a brand-new install, and seeds the marker", async () => {
    // No wizard flag => first launch ever. Announcing "what's new" to
    // someone who has never run an older version is the headline bug.
    window.localStorage.removeItem(WIZARD_KEY);
    const { app, queryByRole } = setup();
    await app().checkWhatsNew();
    expect(queryByRole("status")).toBeNull();
    expect(window.localStorage.getItem(SEEN_KEY)).toBe(CURRENT);
  });

  it("announces the current release to someone who just updated into it", async () => {
    const { app, queryByRole } = setup();
    await app().checkWhatsNew();
    const banner = queryByRole("status");
    expect(banner).not.toBeNull();
    expect(banner!.textContent).toContain(`Updated to v${CURRENT}`);
  });

  it("shows the whole range after skipping several releases", async () => {
    window.localStorage.setItem(SEEN_KEY, "1.42.0");
    const { app, getByRole } = setup();
    await app().checkWhatsNew();
    app().openWhatsNew();
    const text = getByRole("dialog").textContent ?? "";
    expect(text).toContain("Bitcrusher");
    expect(text).toContain("Reactive effects");
    // v1.45.0 carries the whatsnew:skip marker.
    expect(text).not.toContain("Shared envelope follower");
  });

  it("says nothing when the version has already been seen", async () => {
    window.localStorage.setItem(SEEN_KEY, CURRENT);
    const { app, queryByRole } = setup();
    await app().checkWhatsNew();
    expect(queryByRole("status")).toBeNull();
  });

  it("neither shows nor seeds on a dev build", async () => {
    // Seeding at 0.0.0 would silently suppress the panel on this
    // machine's first real install.
    getVersionMock.mockResolvedValue("0.0.0");
    const { app, queryByRole } = setup();
    await app().checkWhatsNew();
    expect(queryByRole("status")).toBeNull();
    expect(window.localStorage.getItem(SEEN_KEY)).toBeNull();
  });

  it("records the version before the user dismisses anything", async () => {
    // If the marker were written on dismiss, closing the app with the
    // banner still up would re-announce on every launch.
    const { app, queryByRole } = setup();
    await app().checkWhatsNew();
    expect(queryByRole("status")).not.toBeNull();
    expect(window.localStorage.getItem(SEEN_KEY)).toBe(CURRENT);
  });

  it("runs at most once per process", async () => {
    const { app } = setup();
    await app().checkWhatsNew();
    app().dismissWhatsNewBanner();
    await app().checkWhatsNew();
    expect(app().whatsNewBanner()).toBeNull();
    expect(getVersionMock).toHaveBeenCalledTimes(1);
  });
});

describe("WhatsNew UI", () => {
  beforeEach(() => {
    invokeMock.mockReset();
    invokeMock.mockImplementation(async (cmd: string) =>
      cmd === "list_presets" ? [] : undefined,
    );
    getVersionMock.mockReset();
    getVersionMock.mockResolvedValue(CURRENT);
    window.localStorage.clear();
    window.localStorage.setItem(WIZARD_KEY, "true");
  });
  afterEach(() => window.localStorage.clear());

  it("renders changelog markdown as elements, not as raw text", () => {
    // "**Bitcrusher effect.** Retro digital destruction…" must not reach
    // the user with its asterisks intact, nor via innerHTML.
    const { app, getByRole, container } = setup();
    app().openWhatsNew();
    expect(getByRole("dialog").textContent).not.toMatch(/\*\*/);
    expect(container.querySelectorAll("strong").length).toBeGreaterThan(0);
    // v1.44.0's note has `windows-2025` in backticks.
    expect(container.querySelectorAll("code").length).toBeGreaterThan(0);
  });

  it("opens from Settings even when nothing is new for this user", () => {
    // The About entry must never be a dead end.
    const { app, getByRole } = setup();
    expect(app().whatsNewContent()).toBeNull();
    app().openWhatsNew();
    expect(getByRole("dialog").textContent).toContain(RELEASE_NOTES[0]!.title!);
  });

  it("closes on Escape and on a backdrop click", () => {
    const { app, getByRole, queryByRole } = setup();
    app().openWhatsNew();
    fireEvent.keyDown(window, { key: "Escape" });
    expect(queryByRole("dialog")).toBeNull();

    app().openWhatsNew();
    fireEvent.click(getByRole("dialog"));
    expect(queryByRole("dialog")).toBeNull();
  });

  it("banner opens the panel and stands down once dismissed", async () => {
    const { app, getByText, queryByRole } = setup();
    await app().checkWhatsNew();
    fireEvent.click(getByText(/See what.s new/));
    expect(queryByRole("dialog")).not.toBeNull();
    // Opening the panel consumes the banner — it must not reappear behind it.
    expect(queryByRole("status")).toBeNull();
  });

  it("dismissing the banner never opens the panel", async () => {
    const { app, container, queryByRole } = setup();
    await app().checkWhatsNew();
    const buttons = container.querySelectorAll<HTMLElement>(
      '[role="status"] button',
    );
    const dismiss = buttons[buttons.length - 1];
    expect(dismiss).toBeDefined();
    fireEvent.click(dismiss!);
    expect(queryByRole("status")).toBeNull();
    expect(queryByRole("dialog")).toBeNull();
  });

  it("prints no bogus count for the earlier-releases line", () => {
    // Note: a blanket /NaN/ check is useless here — v1.39.0's real note
    // says "NaN-guarded". Assert on the line itself instead.
    const { app, getByRole } = setup();
    app().openWhatsNew();
    const text = getByRole("dialog").textContent ?? "";
    expect(text).toContain("…and earlier releases.");
    expect(text).not.toMatch(/and (NaN|undefined|-?\d*\.\d+) earlier/);
  });
});
