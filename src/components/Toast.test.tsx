// The toast is the only way a save made from a hotkey (or a refused one)
// reports itself, so the three things that matter are tested here: the live
// region is always there and announces a repeat, an error stays long enough
// to read, and nothing is shown — or timed — while nobody can see it.

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render } from "@solidjs/testing-library";
import { createSignal } from "solid-js";

const isVisibleMock = vi.fn(async () => true);
vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: () => ({
    isVisible: () => isVisibleMock(),
    onFocusChanged: async () => () => {},
  }),
}));

import { Toast, toastDuration, type ToastMessage } from "./Toast";

let visibility: DocumentVisibilityState = "visible";

function setup() {
  const [message, setMessage] = createSignal<ToastMessage | null>(null);
  const utils = render(() => <Toast message={message()} />);
  const region = utils.getByRole("status");
  let next = 0;
  const say = (text: string, tone: ToastMessage["tone"] = "success") => {
    next += 1;
    setMessage({ id: next, text, tone });
  };
  return { ...utils, region, say };
}

/** Let the visibility check (a dynamic import, then a promise) settle. */
async function settle(): Promise<void> {
  await vi.dynamicImportSettled();
  for (let i = 0; i < 10; i++) await Promise.resolve();
}

/** Settle, then run the animation frame the text is set on — and no
 *  further, so a toast's clock starts at zero. */
async function frame(): Promise<void> {
  await settle();
  vi.advanceTimersToNextFrame();
}

describe("Toast", () => {
  beforeEach(() => {
    vi.useFakeTimers({
      toFake: ["setTimeout", "clearTimeout", "requestAnimationFrame", "cancelAnimationFrame"],
    });
    visibility = "visible";
    Object.defineProperty(document, "visibilityState", {
      configurable: true,
      get: () => visibility,
    });
    isVisibleMock.mockReset();
    isVisibleMock.mockResolvedValue(true);
  });
  afterEach(() => {
    vi.useRealTimers();
    delete (document as unknown as { visibilityState?: unknown }).visibilityState;
  });

  it("the live region is there before any toast, saying nothing", () => {
    const { region } = setup();
    expect(region).toHaveAttribute("aria-live", "polite");
    expect(region).toHaveAttribute("aria-atomic", "true");
    expect(region).toHaveTextContent("");
  });

  it("two identical toasts change the text twice, so both are announced", async () => {
    const { region, say } = setup();
    const seen: string[] = [];
    const observer = new MutationObserver(() => seen.push(region.textContent ?? ""));
    observer.observe(region, { childList: true, characterData: true, subtree: true });

    say("Still saving the last replay.", "error");
    await frame();
    expect(region).toHaveTextContent("Still saving the last replay.");
    say("Still saving the last replay.", "error");
    // Cleared at once, set again on the next frame.
    expect(region).toHaveTextContent("");
    await frame();
    expect(region).toHaveTextContent("Still saving the last replay.");

    await settle();
    observer.disconnect();
    const shown = seen.filter((t, i) => t !== "" && t !== seen[i - 1]);
    expect(shown).toEqual(["Still saving the last replay.", "Still saving the last replay."]);
  });

  it("a success shows 3 s; an error 3 s + 50 ms per character", async () => {
    expect(toastDuration({ text: "x".repeat(95), tone: "error" })).toBe(7_750);
    const { region, say } = setup();

    say("Saved a 31 s replay · Villager");
    await frame();
    vi.advanceTimersByTime(2_999);
    expect(region).toHaveTextContent("Saved a 31 s replay");
    vi.advanceTimersByTime(1);
    expect(region).toHaveTextContent("");

    const error = "There isn't enough free space on the drive to save the replay.";
    say(error, "error");
    await frame();
    vi.advanceTimersByTime(3_000 + 50 * error.length - 1);
    expect(region).toHaveTextContent(error);
    vi.advanceTimersByTime(1);
    expect(region).toHaveTextContent("");
  });

  it("Esc or a click dismisses it", async () => {
    const { region, say } = setup();
    say("Replay is off.", "error");
    await frame();
    expect(region).toHaveTextContent("Replay is off.");
    fireEvent.keyDown(window, { key: "Escape" });
    expect(region).toHaveTextContent("");

    say("Replay is off.", "error");
    await frame();
    fireEvent.click(region);
    expect(region).toHaveTextContent("");
  });

  it("while the document is hidden, a toast waits — then shows for its full time", async () => {
    const { region, say } = setup();
    visibility = "hidden";
    say("Saved a 31 s replay · Villager");
    await frame();
    vi.advanceTimersByTime(60_000);
    expect(region).toHaveTextContent("");

    visibility = "visible";
    document.dispatchEvent(new Event("visibilitychange"));
    await frame();
    expect(region).toHaveTextContent("Saved a 31 s replay · Villager");
    vi.advanceTimersByTime(2_999);
    expect(region).toHaveTextContent("Saved a 31 s replay · Villager");
    vi.advanceTimersByTime(1);
    expect(region).toHaveTextContent("");
  });

  it("a window hidden to the tray holds it too, until the window is focused", async () => {
    isVisibleMock.mockResolvedValue(false);
    const { region, say } = setup();
    say("Turn Replay on from the Record screen card first — it explains what Replay keeps.", "error");
    await frame();
    vi.advanceTimersByTime(30_000);
    expect(region).toHaveTextContent("");

    // Only the latest held toast is shown.
    say("Replay is off.", "error");
    await frame();
    expect(region).toHaveTextContent("");
    window.dispatchEvent(new Event("focus"));
    await frame();
    expect(region).toHaveTextContent("Replay is off.");
  });
});
