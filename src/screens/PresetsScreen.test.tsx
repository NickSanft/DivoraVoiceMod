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
vi.mock("@tauri-apps/plugin-shell", () => ({
  open: vi.fn(async () => undefined),
}));

import { AppProvider, useApp } from "../stores/app";
import { PresetsScreen } from "./PresetsScreen";
import type { JSX } from "solid-js";

function Harness(props: { children: JSX.Element }): JSX.Element {
  return <AppProvider>{props.children}</AppProvider>;
}

function setupScreen() {
  let capturedApp: ReturnType<typeof useApp> | null = null;
  function Inner() {
    capturedApp = useApp();
    return <PresetsScreen />;
  }
  const utils = render(() => (
    <Harness>
      <Inner />
    </Harness>
  ));
  return { ...utils, app: () => capturedApp! };
}

/**
 * v0.8.2 regression: chain-card drag in the Presets editor wasn't
 * actually functional. The whole card had `draggable={true}` but the
 * card contained interactive children (Toggle button + parameter
 * Sliders) which suppressed drag initiation on most of the card's
 * surface; on top of that, `<div draggable>` is finicky in WebView2.
 *
 * The fix uses the explicit drag-handle pattern: only the
 * drag-handle <span> is `draggable={true}` + has `onDragStart`. The
 * card div drops the draggable attribute and only acts as a drop
 * target.
 */
describe("PresetsScreen — ChainCard drag-and-drop wiring (v0.8.2)", () => {
  beforeEach(() => {
    invokeMock.mockReset();
    invokeMock.mockResolvedValue(undefined);
  });

  it("the card itself is NOT directly draggable (drop target only)", () => {
    const { container } = setupScreen();
    const cards = container.querySelectorAll(".card");
    let chainCardCount = 0;
    for (const card of cards) {
      // ChainCards have a child <span draggable="true" role="button">
      // (the explicit handle). Other cards (e.g. action banner) won't.
      const handle = card.querySelector(
        'span[draggable="true"][role="button"]',
      );
      if (!handle) continue;
      chainCardCount += 1;
      // Regression: the card itself must NOT carry the draggable
      // attribute (the v0.8.0 / Phase 4 implementation did exactly
      // this and it failed in WebView2).
      expect(card.getAttribute("draggable")).not.toBe("true");
    }
    expect(chainCardCount).toBeGreaterThanOrEqual(2);
  });

  it("the drag-handle <span> carries the draggable attribute + aria-label", () => {
    const { container } = setupScreen();
    const handles = container.querySelectorAll<HTMLElement>(
      'span[draggable="true"][role="button"]',
    );
    expect(handles.length).toBeGreaterThanOrEqual(2);
    for (const handle of handles) {
      const label = handle.getAttribute("aria-label") ?? "";
      expect(label.toLowerCase()).toContain("drag");
    }
  });

  it("dispatching dragstart on a handle + drop on another card reorders the chain", () => {
    const { container, app } = setupScreen();
    const before = app().chain().map((c) => c.id);
    expect(before.length).toBeGreaterThanOrEqual(3);

    const handles = container.querySelectorAll<HTMLElement>(
      'span[draggable="true"][role="button"]',
    );
    const cards = Array.from(
      container.querySelectorAll<HTMLElement>(".card"),
    ).filter((c) =>
      c.querySelector('span[draggable="true"][role="button"]'),
    );
    expect(handles.length).toBe(cards.length);

    // Build a fake DataTransfer the way Chromium would.
    const data = new Map<string, string>();
    const dataTransfer = {
      effectAllowed: "",
      dropEffect: "",
      setData: (k: string, v: string) => {
        data.set(k, v);
      },
      getData: (k: string) => data.get(k) ?? "",
    } as unknown as DataTransfer;

    // Drag handle of card 0 → drop on card 2.
    const start = new Event("dragstart", { bubbles: true }) as DragEvent;
    Object.defineProperty(start, "dataTransfer", { value: dataTransfer });
    handles[0]!.dispatchEvent(start);

    const drop = new Event("drop", { bubbles: true, cancelable: true }) as DragEvent;
    Object.defineProperty(drop, "dataTransfer", { value: dataTransfer });
    cards[2]!.dispatchEvent(drop);

    const after = app().chain().map((c) => c.id);
    // First entry moved to position 2.
    expect(after[2]).toBe(before[0]);
    // The two middle entries shifted up by one.
    expect(after[0]).toBe(before[1]);
    expect(after[1]).toBe(before[2]);
  });
});
