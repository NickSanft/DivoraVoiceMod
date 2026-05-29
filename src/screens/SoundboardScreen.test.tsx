import { beforeEach, describe, expect, it, vi } from "vitest";
import { render } from "@solidjs/testing-library";

const invokeMock = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
}));
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(async () => () => {
    /* unlisten no-op */
  }),
}));
vi.mock("@tauri-apps/plugin-dialog", () => ({
  open: vi.fn(async () => null),
}));
vi.mock("@tauri-apps/plugin-shell", () => ({
  open: vi.fn(async () => undefined),
}));

import { AppProvider, useApp } from "../stores/app";
import { SoundboardScreen } from "./SoundboardScreen";
import type { JSX } from "solid-js";

function Harness(props: { children: JSX.Element }): JSX.Element {
  return <AppProvider>{props.children}</AppProvider>;
}

function setupScreen() {
  let capturedApp: ReturnType<typeof useApp> | null = null;
  function Inner() {
    capturedApp = useApp();
    return <SoundboardScreen />;
  }
  const utils = render(() => (
    <Harness>
      <Inner />
    </Harness>
  ));
  return { ...utils, app: () => capturedApp! };
}

describe("SoundboardScreen — Tile drag-and-drop wiring (v0.8.1)", () => {
  beforeEach(() => {
    invokeMock.mockReset();
    invokeMock.mockResolvedValue(undefined);
    try {
      window.localStorage.clear();
    } catch {
      /* fine */
    }
  });

  it("renders tiles as draggable <div role='button'> (not <button draggable>) — WebView2 requires this", () => {
    const { container, app } = setupScreen();
    app().setSoundboardFolder("/sb");
    app().setSoundboardTiles([
      { id: "a", path: "/sb/a.wav", label: "a", extension: "wav", sizeBytes: 100 },
      { id: "b", path: "/sb/b.wav", label: "b", extension: "wav", sizeBytes: 100 },
    ]);
    // Find tile elements: divs with role="button" and draggable.
    const tiles = container.querySelectorAll<HTMLDivElement>(
      'div[role="button"][draggable="true"]',
    );
    expect(tiles.length).toBeGreaterThanOrEqual(2);
    // Confirm NO <button draggable> escaped (the v0.8.0 regression).
    expect(
      container.querySelectorAll('button[draggable="true"]').length,
    ).toBe(0);
  });

  it("each draggable tile carries an aria-label that tells the user about both play + reorder", () => {
    const { container, app } = setupScreen();
    app().setSoundboardFolder("/sb");
    app().setSoundboardTiles([
      {
        id: "bell",
        path: "/sb/bell.wav",
        label: "bell",
        extension: "wav",
        sizeBytes: 100,
      },
    ]);
    const tile = container.querySelector<HTMLDivElement>(
      'div[role="button"][draggable="true"]',
    );
    expect(tile).not.toBeNull();
    const label = tile!.getAttribute("aria-label") ?? "";
    expect(label.toLowerCase()).toContain("play");
    expect(label.toLowerCase()).toContain("drag");
  });

  it("dispatching dragstart → drop on two tiles reorders via app.reorderTiles", () => {
    const { container, app } = setupScreen();
    app().setSoundboardFolder("/sb");
    app().setSoundboardTiles([
      { id: "a", path: "/sb/a.wav", label: "a", extension: "wav", sizeBytes: 100 },
      { id: "b", path: "/sb/b.wav", label: "b", extension: "wav", sizeBytes: 100 },
      { id: "c", path: "/sb/c.wav", label: "c", extension: "wav", sizeBytes: 100 },
    ]);

    const tiles = container.querySelectorAll<HTMLDivElement>(
      'div[role="button"][draggable="true"]',
    );
    expect(tiles).toHaveLength(3);

    // jsdom doesn't fire real HTML5 drag, but we can simulate the event
    // sequence the browser fires by constructing DragEvents with a fake
    // DataTransfer.
    const data = new Map<string, string>();
    const dataTransfer = {
      effectAllowed: "",
      dropEffect: "",
      setData: (k: string, v: string) => {
        data.set(k, v);
      },
      getData: (k: string) => data.get(k) ?? "",
    } as unknown as DataTransfer;

    const start = new Event("dragstart", { bubbles: true }) as DragEvent;
    Object.defineProperty(start, "dataTransfer", { value: dataTransfer });
    tiles[0]!.dispatchEvent(start);

    const drop = new Event("drop", { bubbles: true, cancelable: true }) as DragEvent;
    Object.defineProperty(drop, "dataTransfer", { value: dataTransfer });
    tiles[2]!.dispatchEvent(drop);

    expect(app().sortedTiles().map((t) => t.id)).toEqual(["b", "c", "a"]);
  });

  it("click on a tile right after a drop is swallowed (no playClip burst)", () => {
    const { container, app } = setupScreen();
    app().setSoundboardFolder("/sb");
    app().setSoundboardTiles([
      { id: "a", path: "/sb/a.wav", label: "a", extension: "wav", sizeBytes: 100 },
      { id: "b", path: "/sb/b.wav", label: "b", extension: "wav", sizeBytes: 100 },
    ]);
    const tiles = container.querySelectorAll<HTMLDivElement>(
      'div[role="button"][draggable="true"]',
    );

    const data = new Map<string, string>();
    const dataTransfer = {
      effectAllowed: "",
      dropEffect: "",
      setData: (k: string, v: string) => {
        data.set(k, v);
      },
      getData: (k: string) => data.get(k) ?? "",
    } as unknown as DataTransfer;
    const start = new Event("dragstart", { bubbles: true }) as DragEvent;
    Object.defineProperty(start, "dataTransfer", { value: dataTransfer });
    tiles[0]!.dispatchEvent(start);
    const drop = new Event("drop", { bubbles: true, cancelable: true }) as DragEvent;
    Object.defineProperty(drop, "dataTransfer", { value: dataTransfer });
    tiles[1]!.dispatchEvent(drop);

    // Synthetic click that Chromium would fire on the drop target —
    // should NOT cause `play_soundboard_clip` to be invoked.
    invokeMock.mockClear();
    tiles[1]!.dispatchEvent(
      new MouseEvent("click", { bubbles: true, cancelable: true }),
    );
    expect(invokeMock).not.toHaveBeenCalledWith(
      "play_soundboard_clip",
      expect.anything(),
    );
  });
});
