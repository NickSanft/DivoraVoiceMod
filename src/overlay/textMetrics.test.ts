import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { TextMetrics } from "./text";

// textMetrics.ts keeps one canvas context for the window, so each test
// loads a fresh copy of the module.
async function fresh(): Promise<typeof import("./textMetrics")> {
  vi.resetModules();
  return import("./textMetrics");
}

function stubFonts(load: (spec: string, sample?: string) => Promise<unknown>): ReturnType<typeof vi.fn> {
  const spy = vi.fn(load);
  Object.defineProperty(document, "fonts", { value: { load: spy }, configurable: true });
  return spy;
}

/** Whether `p` has settled, without waiting for it. */
async function settled(p: Promise<unknown>): Promise<boolean> {
  let done = false;
  void p.then(
    () => (done = true),
    () => (done = true),
  );
  for (let i = 0; i < 5; i += 1) await Promise.resolve();
  return done;
}

/** A 2D context that records what it is given; `perChar` px a code unit. */
function fakeContext(perChar = 7) {
  const sets: string[] = [];
  const ctx = {
    _font: "",
    get font() {
      return this._font;
    },
    set font(v: string) {
      sets.push(v);
      this._font = v;
    },
    fontKerning: "auto",
    letterSpacing: "10px",
    textRendering: "auto",
    measureText: vi.fn((line: string) => ({ width: perChar * line.length })),
  };
  return { ctx, sets };
}

beforeEach(() => {
  vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
});

afterEach(() => {
  vi.useRealTimers();
  vi.restoreAllMocks();
  delete (document as unknown as Record<string, unknown>).fonts;
});

describe("canvasMetrics: load", () => {
  it("asks document.fonts for the spec's faces covering the sample", async () => {
    const { canvasMetrics } = await fresh();
    const load = stubFonts(async () => []);
    await canvasMetrics().load('700 100px "Space Mono", ui-monospace, monospace', "REC 1:05");
    expect(load).toHaveBeenCalledWith('700 100px "Space Mono", ui-monospace, monospace', "REC 1:05");
  });

  it("main's: gives up at FONT_WAIT_MS", async () => {
    const { canvasMetrics, FONT_WAIT_MS } = await fresh();
    stubFonts(() => new Promise(() => undefined));
    const p = canvasMetrics().load("700 100px x", "hi");
    await vi.advanceTimersByTimeAsync(FONT_WAIT_MS - 1);
    expect(await settled(p)).toBe(false);
    await vi.advanceTimersByTimeAsync(1);
    expect(await settled(p)).toBe(true);
  });

  it("main's: settles as soon as the font does", async () => {
    const { canvasMetrics } = await fresh();
    let arrive!: () => void;
    stubFonts(() => new Promise<void>((r) => (arrive = r)));
    const p = canvasMetrics().load("700 100px x", "hi");
    await vi.advanceTimersByTimeAsync(10);
    expect(await settled(p)).toBe(false);
    arrive();
    expect(await settled(p)).toBe(true);
  });

  it("the overlay page's (no cap): waits for the font however long it takes", async () => {
    const { canvasMetrics, FONT_WAIT_MS } = await fresh();
    let arrive!: () => void;
    stubFonts(() => new Promise<void>((r) => (arrive = r)));
    const p = canvasMetrics(Infinity).load("700 100px x", "hi");
    await vi.advanceTimersByTimeAsync(FONT_WAIT_MS * 5);
    expect(await settled(p)).toBe(false);
    arrive();
    expect(await settled(p)).toBe(true);
  });

  it.each([
    ["rejects", () => Promise.reject(new Error("no such font"))],
    [
      "throws",
      () => {
        throw new SyntaxError("bad spec");
      },
    ],
  ])("never rejects: a load that %s settles", async (_name, load) => {
    const { canvasMetrics } = await fresh();
    stubFonts(load as () => Promise<unknown>);
    for (const m of [canvasMetrics(), canvasMetrics(Infinity)]) {
      await expect(m.load("700 100px x", "hi")).resolves.toBeUndefined();
    }
  });

  it("where the page can't wait for fonts, at once", async () => {
    const { canvasMetrics } = await fresh();
    expect((document as { fonts?: unknown }).fonts).toBeUndefined();
    expect(await settled(canvasMetrics().load("700 100px x", "hi"))).toBe(true);
    expect(await settled(canvasMetrics(Infinity).load("700 100px x", "hi"))).toBe(true);
  });
});

describe("canvasMetrics: width", () => {
  it("measures on the canvas at the spec, with the face's kerning, spacing and rendering", async () => {
    const { canvasMetrics } = await fresh();
    const { ctx, sets } = fakeContext(7);
    vi.spyOn(HTMLCanvasElement.prototype, "getContext").mockReturnValue(ctx as unknown as CanvasRenderingContext2D);
    const m: TextMetrics = canvasMetrics();
    const spec = '700 100px "Space Grotesk Variable", system-ui, sans-serif';
    expect(m.width(spec, "BRB")).toBe(21);
    expect(ctx.measureText).toHaveBeenCalledWith("BRB");
    expect(ctx.font).toBe(spec);
    expect(ctx.fontKerning).toBe("normal");
    expect(ctx.letterSpacing).toBe("0px");
    expect(ctx.textRendering).toBe("optimizeLegibility");
    // The same spec again isn't set again; a new one is.
    m.width(spec, "back");
    expect(sets).toEqual([spec]);
    m.width("400 100px x", "back");
    expect(sets).toEqual([spec, "400 100px x"]);
    expect(ctx.letterSpacing).toBe("0px");
  });

  it("one canvas for the window, made at the first measure, not before", async () => {
    const { canvasMetrics } = await fresh();
    const { ctx } = fakeContext();
    const getContext = vi
      .spyOn(HTMLCanvasElement.prototype, "getContext")
      .mockReturnValue(ctx as unknown as CanvasRenderingContext2D);
    const main = canvasMetrics();
    const page = canvasMetrics(Infinity);
    expect(getContext).not.toHaveBeenCalled();
    main.width("700 100px x", "a");
    page.width("700 100px x", "b");
    expect(getContext).toHaveBeenCalledTimes(1);
    expect(getContext).toHaveBeenCalledWith("2d");
  });

  it("with no 2D canvas (jsdom), 0.6 em a code unit at 100 px", async () => {
    const { canvasMetrics } = await fresh();
    vi.spyOn(HTMLCanvasElement.prototype, "getContext").mockReturnValue(null);
    const m = canvasMetrics();
    expect(m.width("700 100px x", "BRB")).toBeCloseTo(180, 9);
    expect(m.width("700 100px x", "🎮")).toBeCloseTo(120, 9);
  });
});
