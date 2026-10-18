import { createRoot, createSignal } from "solid-js";
import { describe, expect, it } from "vitest";
import { defaultScene } from "./scene";
import {
  overlayPayload,
  OVERLAY_EVENT,
  OVERLAY_READY_EVENT,
  OVERLAY_SCENE_EVENT,
  type OverlayState,
  type ScenePush,
} from "./state";
import { syncOverlay, syncOverlayScene } from "./sync";

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

describe("syncOverlayScene (v1.55.0)", () => {
  // The scene's bus: emits can be held open to check their order.
  function sceneBus(opts: { deferred?: boolean } = {}) {
    const calls: ScenePush[] = [];
    const done: ScenePush[] = [];
    const pending: Array<() => void> = [];
    const handlers = new Map<string, Array<() => void>>();
    let unlistened = 0;
    return {
      calls,
      done,
      get unlistened() {
        return unlistened;
      },
      emit: (event: string, payload: ScenePush): Promise<void> => {
        expect(event).toBe(OVERLAY_SCENE_EVENT);
        calls.push(payload);
        if (!opts.deferred) {
          done.push(payload);
          return Promise.resolve();
        }
        return new Promise<void>((resolve) =>
          pending.push(() => {
            done.push(payload);
            resolve();
          }),
        );
      },
      release(): void {
        pending.shift()?.();
      },
      listen: async (event: string, handler: () => void): Promise<() => void> => {
        handlers.set(event, [...(handlers.get(event) ?? []), handler]);
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

  const scene = defaultScene();

  /** A controller in miniature: a revision and an ack, read by push(). */
  function source() {
    const [revision, setRevision] = createSignal(1);
    const [ack, setAck] = createSignal<ScenePush["ack"]>(null);
    const push = (): ScenePush => ({ session: "s", revision: revision(), scene, ack: ack(), expectViewport: null });
    return { setRevision, setAck, push };
  }

  it("re-pushes on ready, after onReady has reset the ack, once", async () => {
    const b = sceneBus();
    const src = source();
    src.setAck({ instance: 1, seq: 4 });
    const order: string[] = [];
    const sync = createRoot(() =>
      syncOverlayScene({
        isOpen: () => true,
        push: src.push,
        emit: b.emit,
        listen: b.listen,
        onReady: () => {
          order.push("onReady");
          src.setAck(null);
        },
      }),
    );
    await flush();
    await sync.flushed();
    expect(b.calls.map((p) => p.ack)).toEqual([{ instance: 1, seq: 4 }]);

    b.fire(OVERLAY_READY_EVENT);
    await sync.flushed();
    expect(order).toEqual(["onReady"]);
    // Exactly one more push, and it carries no ack: a reloaded page counts
    // its edits from 0 again.
    expect(b.calls.map((p) => p.ack)).toEqual([{ instance: 1, seq: 4 }, null]);
    sync.dispose();
  });

  it("re-pushes the scene on every ready, changed or not", async () => {
    const b = sceneBus();
    const src = source();
    const sync = createRoot(() =>
      syncOverlayScene({ isOpen: () => true, push: src.push, emit: b.emit, listen: b.listen, onReady: () => {} }),
    );
    await flush();
    b.fire(OVERLAY_READY_EVENT);
    b.fire(OVERLAY_READY_EVENT);
    await sync.flushed();
    expect(b.calls).toHaveLength(3);
    sync.dispose();
  });

  it("sends nothing while the overlay is closed, ready or not", async () => {
    const b = sceneBus();
    const src = source();
    const [open, setOpen] = createSignal(false);
    const sync = createRoot(() =>
      syncOverlayScene({ isOpen: open, push: src.push, emit: b.emit, listen: b.listen, onReady: () => {} }),
    );
    await flush();
    b.fire(OVERLAY_READY_EVENT);
    src.setRevision(2);
    await sync.flushed();
    expect(b.calls).toEqual([]);

    setOpen(true);
    await sync.flushed();
    expect(b.calls.map((p) => p.revision)).toEqual([2]);
    sync.dispose();
  });

  it("emits one at a time, in order: the next waits for the one before", async () => {
    const b = sceneBus({ deferred: true });
    const src = source();
    const sync = createRoot(() =>
      syncOverlayScene({ isOpen: () => true, push: src.push, emit: b.emit, listen: b.listen, onReady: () => {} }),
    );
    // Two more changes in the same tick.
    src.setRevision(2);
    src.setRevision(3);
    let flushed = false;
    void sync.flushed().then(() => {
      flushed = true;
    });
    await flush();
    expect(b.calls.map((p) => p.revision)).toEqual([1]);
    b.release();
    await flush();
    expect(b.calls.map((p) => p.revision)).toEqual([1, 2]);
    expect(flushed).toBe(false);
    b.release();
    await flush();
    expect(b.calls.map((p) => p.revision)).toEqual([1, 2, 3]);
    expect(flushed).toBe(false);
    b.release();
    await flush();
    expect(b.done.map((p) => p.revision)).toEqual([1, 2, 3]);
    expect(flushed).toBe(true);
    sync.dispose();
  });

  it("a failed emit doesn't stop the ones after it", async () => {
    const sent: number[] = [];
    let fail = true;
    const src = source();
    const sync = createRoot(() =>
      syncOverlayScene({
        isOpen: () => true,
        push: src.push,
        emit: async (_e, p) => {
          if (fail) {
            fail = false;
            throw new Error("no tauri");
          }
          sent.push(p.revision);
        },
        listen: () => Promise.reject(new Error("no tauri")),
        onReady: () => {},
      }),
    );
    src.setRevision(2);
    await sync.flushed();
    expect(sent).toEqual([2]);
    sync.dispose();
  });

  it("dispose, or its owner going, stops the listener and the pushes", async () => {
    const b = sceneBus();
    const src = source();
    let sync!: ReturnType<typeof syncOverlayScene>;
    const disposeRoot = createRoot((d) => {
      sync = syncOverlayScene({ isOpen: () => true, push: src.push, emit: b.emit, listen: b.listen, onReady: () => {} });
      return d;
    });
    await flush();
    await sync.flushed();
    sync.dispose();
    expect(b.unlistened).toBe(1);
    b.fire(OVERLAY_READY_EVENT);
    src.setRevision(5);
    await sync.flushed();
    expect(b.calls).toHaveLength(1);
    disposeRoot();
    expect(b.unlistened).toBe(1);

    // Its owner disposed before the listener resolved: removed once it does.
    const b2 = sceneBus();
    const d2 = createRoot((d) => {
      syncOverlayScene({ isOpen: () => true, push: src.push, emit: b2.emit, listen: b2.listen, onReady: () => {} });
      return d;
    });
    d2();
    await flush();
    expect(b2.unlistened).toBe(1);
  });
});
