import { createRoot, createSignal } from "solid-js";
import { describe, expect, it } from "vitest";
import { overlayPayload, OVERLAY_EVENT, OVERLAY_READY_EVENT, type OverlayState } from "./state";
import { syncOverlay } from "./sync";

// Drives the same `syncOverlay` App.tsx calls, with an in-memory event bus
// standing in for Tauri's.
function bus() {
  const sent: Array<{ event: string; payload: OverlayState }> = [];
  const handlers = new Map<string, Array<() => void>>();
  let unlistened = 0;
  return {
    sent,
    get unlistened() {
      return unlistened;
    },
    emit: async (event: string, payload: OverlayState): Promise<void> => {
      sent.push({ event, payload });
    },
    listen: async (event: string, handler: () => void): Promise<() => void> => {
      const list = handlers.get(event) ?? [];
      list.push(handler);
      handlers.set(event, list);
      return () => {
        unlistened += 1;
        handlers.set(
          event,
          (handlers.get(event) ?? []).filter((h) => h !== handler),
        );
      };
    },
    fire(event: string): void {
      for (const h of handlers.get(event) ?? []) h();
    },
  };
}

const flush = (): Promise<void> => new Promise((r) => setTimeout(r, 0));

function state(status: OverlayState["status"]): OverlayState {
  return overlayPayload({
    chain: [],
    status,
    motion: 1,
    mystical: 0.7,
    mood: "violet",
    accent: "brand",
    theme: "dark",
    bg: "transparent",
  });
}

describe("syncOverlay", () => {
  it("re-sends the current state when the overlay says it is ready", async () => {
    const b = bus();
    const [open] = createSignal(true);
    const [status, setStatus] = createSignal<OverlayState["status"]>("clean");
    const dispose = createRoot((d) => {
      syncOverlay({ isOpen: open, payload: () => state(status()), emit: b.emit, listen: b.listen });
      return d;
    });
    await flush();

    // The push made as the window opened — the one that could be lost.
    expect(b.sent.map((s) => s.payload.status)).toEqual(["clean"]);

    setStatus("modulated");
    expect(b.sent.map((s) => s.payload.status)).toEqual(["clean", "modulated"]);

    // The overlay's page finished loading: it gets the state as it is NOW.
    b.fire(OVERLAY_READY_EVENT);
    expect(b.sent.map((s) => s.payload.status)).toEqual(["clean", "modulated", "modulated"]);
    expect(b.sent.every((s) => s.event === OVERLAY_EVENT)).toBe(true);
    dispose();
  });

  it("sends nothing while the overlay is closed, ready or not", async () => {
    const b = bus();
    const [open, setOpen] = createSignal(false);
    const dispose = createRoot((d) => {
      syncOverlay({ isOpen: open, payload: () => state("clean"), emit: b.emit, listen: b.listen });
      return d;
    });
    await flush();
    b.fire(OVERLAY_READY_EVENT);
    expect(b.sent).toEqual([]);

    setOpen(true);
    expect(b.sent).toHaveLength(1);
    dispose();
  });

  it("stops listening when its owner goes away", async () => {
    const b = bus();
    const dispose = createRoot((d) => {
      syncOverlay({ isOpen: () => true, payload: () => state("clean"), emit: b.emit, listen: b.listen });
      return d;
    });
    await flush();
    dispose();
    expect(b.unlistened).toBe(1);
    const before = b.sent.length;
    b.fire(OVERLAY_READY_EVENT);
    expect(b.sent.length).toBe(before);
  });

  it("unlistens even if disposed before the listener resolved", async () => {
    const b = bus();
    const dispose = createRoot((d) => {
      syncOverlay({ isOpen: () => true, payload: () => state("clean"), emit: b.emit, listen: b.listen });
      return d;
    });
    dispose();
    await flush();
    expect(b.unlistened).toBe(1);
  });

  it("a failing bus (off Tauri) never throws", async () => {
    const dispose = createRoot((d) => {
      syncOverlay({
        isOpen: () => true,
        payload: () => state("clean"),
        emit: () => Promise.reject(new Error("no tauri")),
        listen: () => Promise.reject(new Error("no tauri")),
      });
      return d;
    });
    await flush();
    dispose();
  });
});
