// v1.54.0: the titlebar's Replay chip — one of Replay's indicators, with the
// tray icon and Windows' border. Driven through the store's real
// `replay-buffer` subscription, as the backend drives it.

import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { render } from "@solidjs/testing-library";
import type { JSX } from "solid-js";

type Handler = (e: { payload: unknown }) => void;
const handlers = new Map<string, Handler[]>();

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(async (cmd: string) => {
    if (cmd === "screen_capture_status") {
      return { supported: true, reason: null, h264Encoder: "X", hardware: true };
    }
    if (cmd === "list_capture_monitors") {
      return [
        { id: "\\\\.\\DISPLAY1", number: 1, x: 0, y: 0, width: 2560, height: 1440, primary: true },
        { id: "\\\\.\\DISPLAY2", number: 2, x: 2560, y: 0, width: 1920, height: 1080, primary: false },
      ];
    }
    return null;
  }),
}));
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(async (event: string, handler: Handler) => {
    handlers.set(event, [...(handlers.get(event) ?? []), handler]);
    return () => {};
  }),
}));

import { AppProvider, useApp, type AppState } from "../stores/app";
import { Titlebar } from "./Titlebar";

const ARMED = {
  supported: true,
  reason: null,
  state: "armed",
  saving: false,
  seconds: 30,
  bufferedMs: 30_000,
  bytes: 1,
  capBytes: 80_044_800,
  monitorId: "\\\\.\\DISPLAY2",
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

async function setup() {
  let app: AppState | null = null;
  function Grab(): JSX.Element {
    app = useApp();
    return <Titlebar />;
  }
  const utils = render(() => (
    <AppProvider>
      <Grab />
    </AppProvider>
  ));
  // Let the store's subscriptions register.
  for (let i = 0; i < 10; i++) await Promise.resolve();
  await app!.refreshCapture();
  const emit = (patch: Record<string, unknown>) => {
    for (const h of handlers.get("replay-buffer") ?? []) h({ payload: { ...ARMED, ...patch } });
  };
  return { ...utils, emit };
}

describe("Titlebar — Replay chip (v1.54.0)", () => {
  beforeEach(() => handlers.clear());

  it("shows only while Replay is on", async () => {
    const { queryByTestId, emit } = await setup();
    expect(queryByTestId("titlebar-replay")).toBeNull();
    emit({});
    expect(queryByTestId("titlebar-replay")).not.toBeNull();
    emit({ state: "off", armed: null, monitorId: null });
    expect(queryByTestId("titlebar-replay")).toBeNull();
  });

  it("reads its length, at 60 % while arming, and HELD in the warning colour", async () => {
    const { getByTestId, emit } = await setup();
    emit({});
    const chip = () => getByTestId("titlebar-replay");
    expect(chip().textContent?.trim()).toBe("REPLAY · 30 s");
    expect(chip().className).not.toMatch(/is-arming|is-held/);

    emit({ state: "arming", bufferedMs: 0 });
    expect(chip().textContent?.trim()).toBe("REPLAY · 30 s");
    expect(chip()).toHaveClass("is-arming");

    emit({ state: "held", heldReason: "engine-stopped", seconds: 60 });
    expect(chip().textContent?.trim()).toBe("REPLAY · HELD");
    expect(chip()).toHaveClass("is-held");
    expect(chip().title).toMatch(/^The audio engine stopped/);
  });

  it("its title says what is kept, naming calls once desktop audio is", async () => {
    const { getByTestId, emit } = await setup();
    emit({});
    expect(getByTestId("titlebar-replay").title).toBe(
      "Replay is keeping the last 30 s of Screen 2, your voice and soundboard in memory. Windows draws a yellow border around that screen.",
    );
    emit({ armed: { ...ARMED.armed, desktopAudio: "everything" } });
    expect(getByTestId("titlebar-replay").title).toContain(
      "your voice and soundboard, and sound from other apps (calls included) in memory",
    );
  });

  it("is never animated, in any state", async () => {
    const { getByTestId, emit } = await setup();
    for (const patch of [{}, { state: "arming" }, { state: "held", heldReason: "screen-lost" }]) {
      emit(patch);
      const chip = getByTestId("titlebar-replay");
      for (const el of [chip, ...chip.querySelectorAll<HTMLElement | SVGElement>("*")]) {
        expect(el.getAttribute("style") ?? "").not.toMatch(/animation/);
        expect(el.style.animation ?? "").toBe("");
      }
    }
    // Nor in the stylesheet: the reduced-motion rule is dead after mount, so
    // "static" has to be literal.
    const css = readFileSync(join(dirname(fileURLToPath(import.meta.url)), "../styles.css"), "utf8");
    const rules = css.match(/\.titlebar-replay[^{]*\{[^}]*\}/g) ?? [];
    expect(rules.length).toBeGreaterThanOrEqual(3);
    for (const rule of rules) expect(rule).not.toMatch(/animation|transition/);
  });
});
