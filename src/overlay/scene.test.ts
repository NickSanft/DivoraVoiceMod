import { describe, expect, it } from "vitest";
import {
  addRefusal,
  applyOp,
  assetRefCount,
  assetRefs,
  ASSET_ID_RE,
  charmLabels,
  charmUrl,
  CHARM_ORIGIN,
  CIRCLE_ID,
  CIRCLE_PX,
  defaultImageRect,
  defaultScene,
  frameOf,
  handleRects,
  hitPart,
  hitRect,
  isLive,
  layoutElement,
  layoutScene,
  liveElements,
  MAX_ELEMENTS,
  migrateScene,
  moveRect,
  normalizeRect,
  pictureCount,
  readOp,
  readScene,
  scaleRect,
  sceneForSave,
  type KnownElement,
  type OverlayScene,
  type PxRect,
  type SceneElement,
  type SceneOp,
  type Size,
} from "./scene";
import { SPELL_CIRCLE_PX } from "../components/SpellCircle";
import fixture from "./wire.fixture.json";

describe("charm picture ids (v1.55.0)", () => {
  it("ASSET_ID_RE agrees with the backend's is_asset_id", () => {
    // overlay_assets.rs `asset_id_table` checks the same cases.
    for (const { id, valid } of fixture.assetIds) {
      expect(ASSET_ID_RE.test(id), JSON.stringify(id)).toBe(valid);
    }
  });

  it("loads pictures from the backend's charm scheme", () => {
    expect(CHARM_ORIGIN).toBe(fixture.charmOrigin);
  });

  it("charmUrl builds a URL only for an asset id", () => {
    for (const { id, valid } of fixture.assetIds) {
      expect(charmUrl(id), JSON.stringify(id)).toBe(valid ? fixture.charmOrigin + id : "");
    }
  });
});

// ---- fixtures ----

const SRC_A = "a1b2c3d4e5f60718.gif";
const SRC_B = "0123456789abcdef.png";
const SRC_C = "ffffffffffffffff.webp";
const D480: Size = { w: 480, h: 480 };

function circle(id = CIRCLE_ID, r = { x: 0.041667, y: 0.041667, w: 0.916667, h: 0.916667 }, z = 0): SceneElement {
  return { id, type: "spellcircle", ...r, z };
}

function picture(id: string, r: { x: number; y: number; w: number; h: number }, more: Record<string, unknown> = {}): SceneElement {
  return { id, type: "image", ...r, z: 0, src: SRC_A, nw: 2, nh: 1, ...more };
}

function sceneOf(elements: SceneElement[], design: Size = D480, more: Record<string, unknown> = {}): OverlayScene {
  return { version: 1, designSize: { ...design }, frame: "window", elements, ...more };
}

const json = (v: unknown): string => JSON.stringify(v);

function rectOf(s: OverlayScene, id: string, v: Size): PxRect {
  const hit = layoutScene(s, v).find((r) => r.el.id === id);
  if (!hit) throw new Error(`${id} not laid out`);
  return hit.rect;
}

function expectRectNear(a: PxRect, b: PxRect, tol: number): void {
  for (const k of ["x", "y", "w", "h"] as const) {
    expect(Math.abs(a[k] - b[k]), `${k}: ${a[k]} vs ${b[k]}`).toBeLessThanOrEqual(tol);
  }
}

/** Every kind of element this version keeps but doesn't show (§3.2). */
function inertSet(): SceneElement[] {
  return [
    { id: "timer", type: "timer", x: 0.1, y: 0.1, w: 0.2, h: 0.1, z: 2, future: { a: [1, 2], b: "x" } },
    picture("avif", { x: 0.5, y: 0.5, w: 0.1, h: 0.1 }, { src: "0123456789abcdef.avif", future: { keep: true } }),
    picture("w0", { x: 0.2, y: 0.2, w: 0, h: 0.1 }),
    picture("wneg", { x: 0.2, y: 0.2, w: -0.2, h: 0.1 }),
    (() => {
      const e = picture("nox", { x: 0, y: 0.2, w: 0.1, h: 0.1 });
      delete e.x;
      return e;
    })(),
    circle("circle2", { x: 0.3, y: 0.3, w: 0.2, h: 0.2 }, 5),
    (() => {
      const e = picture("nosrc", { x: 0.6, y: 0.1, w: 0.1, h: 0.1 });
      delete e.src;
      return e;
    })(),
    picture("srcnum", { x: 0.6, y: 0.3, w: 0.1, h: 0.1 }, { src: 5 }),
  ];
}

const INERT_IDS = ["timer", "avif", "w0", "wneg", "nox", "circle2", "nosrc", "srcnum"];

function mixedScene(): OverlayScene {
  return sceneOf(
    [circle(), picture("A", { x: 0.7, y: 0.4, w: 0.3, h: 0.15 }, { z: 1, name: "wave.gif" }), ...inertSet()],
    D480,
    { theme: "x", future: { nested: [SRC_C] } },
  );
}

// ---- reading ----

describe("readScene", () => {
  const fb = defaultScene();

  it("nothing readable gives the fallback", () => {
    for (const raw of [null, undefined, "scene", 42, true, [], [{ id: "a" }]]) {
      expect(readScene(raw, fb)).toBe(fb);
    }
  });

  it("settles version and designSize, field by field", () => {
    const cases: Array<[unknown, number]> = [
      [1, 1],
      [2, 2],
      [0, 1],
      [-3, 1],
      [2.5, 1],
      ["2", 1],
      [Number.NaN, 1],
      [undefined, 1],
    ];
    for (const [v, want] of cases) {
      expect(readScene({ version: v, elements: [] }, fb).version, String(v)).toBe(want);
    }
    const sizes: Array<[unknown, Size]> = [
      [{ w: 600, h: 300 }, { w: 600, h: 300 }],
      [{ w: 1, h: 16384 }, { w: 1, h: 16384 }],
      [{ w: 0, h: 300 }, D480],
      [{ w: 0.5, h: 300 }, D480],
      [{ w: 16385, h: 300 }, D480],
      [{ w: Number.NaN, h: 300 }, D480],
      [{ w: Number.POSITIVE_INFINITY, h: 300 }, D480],
      [{ w: "600", h: 300 }, D480],
      [null, D480],
      [[600, 300], D480],
    ];
    for (const [ds, want] of sizes) {
      expect(readScene({ designSize: ds, elements: [] }, fb).designSize, json(ds)).toEqual(want);
    }
  });

  it("elements that aren't an array become the default circle for the design size", () => {
    const s = readScene({ designSize: { w: 600, h: 300 }, elements: "nope" }, fb);
    expect(s.elements).toEqual([{ id: "circle", type: "spellcircle", x: 0.25, y: 0, w: 0.5, h: 1, z: 0 }]);
  });

  it("keeps unknown keys at every level, and every element object as stored", () => {
    const raw = {
      version: 1,
      designSize: { w: 480, h: 480, unit: "css" },
      frame: "window",
      theme: "x",
      future: { a: 1 },
      elements: inertSet(),
    };
    const s = readScene(raw, fb);
    expect(s.theme).toBe("x");
    expect(s.future).toEqual({ a: 1 });
    expect(s.designSize).toEqual({ w: 480, h: 480, unit: "css" });
    // Reading changes no element: the same objects, deep-equal to the input.
    s.elements.forEach((e, i) => expect(e).toBe(raw.elements[i]));
    expect(json(s.elements)).toBe(json(raw.elements));
  });

  it("drops only entries nothing can address, and later duplicate ids", () => {
    const keep1 = picture("p", { x: 0.1, y: 0.1, w: 0.1, h: 0.1 });
    const dup = picture("p", { x: 0.9, y: 0.9, w: 0.1, h: 0.1 });
    const keep2 = { id: "u", type: "sticker" };
    const raw = {
      elements: [null, 3, "x", [], { type: "image" }, { id: "" }, { id: 5 }, keep1, dup, keep2],
    };
    expect(readScene(raw, fb).elements).toEqual([keep1, keep2]);
  });

  it("keeps a newer version on write, and never lowers it", () => {
    expect(sceneForSave(readScene({ version: 2, elements: [] }, fb)).version).toBe(2);
    expect(sceneForSave(readScene({ version: 0, elements: [] }, fb)).version).toBe(1);
    expect(sceneForSave({ ...defaultScene(), version: Number.NaN }).version).toBe(1);
    const s = mixedScene();
    expect(json({ ...sceneForSave(s), version: 1 })).toBe(json(s));
  });

  it("frame: missing or unknown reads as window, and the stored value is kept", () => {
    const cases: Array<[unknown, string]> = [
      [undefined, "window"],
      ["window", "window"],
      ["monitor", "monitor"],
      ["MONITOR", "window"],
      ["sideways", "window"],
      [5, "window"],
    ];
    for (const [frame, want] of cases) {
      const s = readScene({ frame, elements: [] }, fb);
      expect(frameOf(s), String(frame)).toBe(want);
      expect(s.frame).toBe(frame);
    }
  });

  it("NaN and Infinity geometry is kept, inert", () => {
    const e = picture("n", { x: Number.NaN, y: 0, w: Number.POSITIVE_INFINITY, h: 0.1 });
    const s = readScene({ elements: [e] }, fb);
    expect(s.elements[0]).toBe(e);
    expect(liveElements(s)).toEqual([]);
  });
});

describe("migrateScene", () => {
  it("a v1.51.3 480×480 window at 100 % is exactly the stored v1 default", () => {
    // §3.2's JSON, as written to divora.overlayScene.
    expect(json(migrateScene({ bg: "green", x: 10, y: 10, w: 480, h: 480, scale: 1 }))).toBe(
      json({
        version: 1,
        designSize: { w: 480, h: 480 },
        frame: "window",
        elements: [{ id: "circle", type: "spellcircle", x: 0.041667, y: 0.041667, w: 0.916667, h: 0.916667, z: 0 }],
      }),
    );
  });

  it("the circle is 440 px and centred, as v1.51.3 drew it", () => {
    const r = rectOf(migrateScene({ w: 480, h: 480, scale: 1 }), CIRCLE_ID, D480);
    expectRectNear(r, { x: 20, y: 20, w: 440, h: 440 }, 1e-3);
  });

  it("design size from the saved window, in CSS px", () => {
    const cases: Array<[unknown, Size, number]> = [
      [{ w: 300, h: 300, scale: 1 }, { w: 300, h: 300 }, 300],
      [{ w: 720, h: 720, scale: 1.5 }, D480, 440],
      [{ w: 600, h: 600 }, { w: 600, h: 600 }, 440],
      [{ w: 600, h: 600, scale: 0 }, { w: 600, h: 600 }, 440],
      [{ w: 1200, h: 675, scale: 1.25 }, { w: 960, h: 540 }, 440],
      [{ w: 0, h: 600 }, D480, 440],
      [{ bg: "green" }, D480, 440],
      [{ w: "480", h: 480 }, D480, 440],
      [null, D480, 440],
      ["junk", D480, 440],
    ];
    for (const [prefs, design, side] of cases) {
      const s = migrateScene(prefs);
      expect(s.designSize, json(prefs)).toEqual(design);
      expect(s.frame).toBe("window");
      expect(s.version).toBe(1);
      const r = rectOf(s, CIRCLE_ID, design);
      expect(r.w).toBeCloseTo(side, 3);
      expect(r.h).toBeCloseTo(side, 3);
      // Centred.
      expect(r.x + r.w / 2).toBeCloseTo(design.w / 2, 3);
      expect(r.y + r.h / 2).toBeCloseTo(design.h / 2, 3);
    }
  });
});

// ---- liveness and layout ----

describe("live and inert elements", () => {
  it("only the first usable circle and pictures with an asset id are live", () => {
    const s = mixedScene();
    expect(liveElements(s).map((e) => e.id)).toEqual(["circle", "A"]);
    expect(layoutScene(s, D480).map((r) => r.el.id)).toEqual(["circle", "A"]);
    expect(pictureCount(s)).toBe(1);
    for (const id of INERT_IDS) expect(isLive(s, id), id).toBe(false);
  });

  it("a circle with unusable geometry doesn't take the live slot", () => {
    const s = sceneOf([circle("bad", { x: 0, y: 0, w: 0, h: 0.5 }), circle("good", { x: 0.1, y: 0.1, w: 0.5, h: 0.5 })]);
    expect(liveElements(s).map((e) => e.id)).toEqual(["good"]);
  });

  it("an id over 64 characters isn't live", () => {
    const s = sceneOf([picture("x".repeat(65), { x: 0.1, y: 0.1, w: 0.1, h: 0.1 })]);
    expect(liveElements(s)).toEqual([]);
  });
});

describe("layout (§3.4)", () => {
  // A 120×48 picture whose centre is at (0.325, 0.35) of a 480 design.
  const pic = picture("p", { x: 0.2, y: 0.3, w: 0.25, h: 0.1 }) as unknown as KnownElement;

  const rows: Array<[string, Size, PxRect]> = [
    ["identity", D480, { x: 96, y: 144, w: 120, h: 48 }],
    ["double width", { w: 960, h: 480 }, { x: 252, y: 144, w: 120, h: 48 }],
    ["half size", { w: 240, h: 240 }, { x: 18, y: 60, w: 120, h: 48 }],
    ["tall and narrow", { w: 100, h: 480 }, { x: 0, y: 148, w: 100, h: 40 }],
    ["short and wide", { w: 960, h: 30 }, { x: 274.5, y: 0, w: 75, h: 30 }],
  ];

  it.each(rows)("%s", (_name, v, want) => {
    const r = layoutElement(pic, D480, v)!;
    expectRectNear(r, want, 1e-9);
    // Aspect exactly W:H; never outside, never larger than V or than designed.
    expect(r.w / r.h).toBeCloseTo(120 / 48, 12);
    expect(r.x).toBeGreaterThanOrEqual(0);
    expect(r.y).toBeGreaterThanOrEqual(0);
    expect(r.x + r.w).toBeLessThanOrEqual(v.w + 1e-9);
    expect(r.y + r.h).toBeLessThanOrEqual(v.h + 1e-9);
    expect(r.w).toBeLessThanOrEqual(120 + 1e-9);
  });

  it("stays inside every viewport, whatever it stores", () => {
    const odd = picture("o", { x: 1.4, y: -0.6, w: 1.5, h: 0.2 }) as unknown as KnownElement;
    for (const w of [1, 7, 24, 100, 479, 480, 1000]) {
      for (const h of [1, 13, 300, 480, 2000]) {
        for (const e of [pic, odd]) {
          const r = layoutElement(e, D480, { w, h })!;
          expect(r.x).toBeGreaterThanOrEqual(0);
          expect(r.y).toBeGreaterThanOrEqual(0);
          expect(r.x + r.w).toBeLessThanOrEqual(w + 1e-9);
          expect(r.y + r.h).toBeLessThanOrEqual(h + 1e-9);
        }
      }
    }
  });

  it("an empty viewport lays out nothing", () => {
    for (const v of [
      { w: 0, h: 480 },
      { w: 480, h: 0 },
      { w: -5, h: 480 },
      { w: Number.NaN, h: 480 },
    ]) {
      expect(layoutElement(pic, D480, v)).toBeNull();
      expect(layoutScene(mixedScene(), v)).toEqual([]);
    }
  });

  it("CIRCLE_PX is the spell circle's own size (SpellCircle.tsx)", () => {
    // scene.ts keeps its own copy so it imports no component; the overlay's
    // Stage scales a SPELL_CIRCLE_PX box, so the two must agree.
    expect(CIRCLE_PX).toBe(SPELL_CIRCLE_PX);
  });

  it("a stored 300×200 circle lays out as the 200×200 square centred in it", () => {
    const c = circle("c", { x: 0.1, y: 0.2, w: 300 / 480, h: 200 / 480 }) as unknown as KnownElement;
    expectRectNear(layoutElement(c, D480, D480)!, { x: 48 + 50, y: 96, w: 200, h: 200 }, 1e-9);
  });

  it("back to front by z, then by array index; a z that isn't a number counts as the index", () => {
    const s = sceneOf([
      picture("a", { x: 0, y: 0, w: 0.1, h: 0.1 }, { z: 3 }),
      picture("b", { x: 0, y: 0, w: 0.1, h: 0.1 }, { z: 1 }),
      picture("c", { x: 0, y: 0, w: 0.1, h: 0.1 }, { z: 1 }),
      picture("d", { x: 0, y: 0, w: 0.1, h: 0.1 }, { z: "top" }),
      picture("e", { x: 0, y: 0, w: 0.1, h: 0.1 }, { z: -1 }),
    ]);
    expect(layoutScene(s, D480).map((r) => r.el.id)).toEqual(["e", "b", "c", "a", "d"]);
  });
});

// ---- the reducer ----

describe("applyOp add", () => {
  const newPic = (id: string, more: Record<string, unknown> = {}): KnownElement =>
    picture(id, { x: 0.6, y: 0.665663, w: 0.3, h: 0.168675 }, { src: SRC_B, nw: 498, nh: 280, ...more }) as unknown as KnownElement;

  it("adds a picture in front, renumbering z", () => {
    const s = sceneOf([circle(), picture("A", { x: 0.1, y: 0.1, w: 0.1, h: 0.1 }, { z: 7 })]);
    const next = applyOp(s, { kind: "add", element: newPic("N") });
    expect(next.elements.map((e) => [e.id, e.z])).toEqual([
      ["circle", 0],
      ["A", 1],
      ["N", 2],
    ]);
    expect(layoutScene(next, D480).map((r) => r.el.id)).toEqual(["circle", "A", "N"]);
  });

  it("adds the circle at the back", () => {
    const s = sceneOf([
      picture("A", { x: 0.1, y: 0.1, w: 0.1, h: 0.1 }, { z: 0 }),
      picture("B", { x: 0.1, y: 0.1, w: 0.1, h: 0.1 }, { z: 1 }),
    ]);
    const next = applyOp(s, { kind: "add", element: circle() as unknown as KnownElement });
    expect(next.elements.map((e) => [e.id, e.z])).toEqual([
      ["A", 1],
      ["B", 2],
      ["circle", 0],
    ]);
    expect(layoutScene(next, D480).map((r) => r.el.id)).toEqual(["circle", "A", "B"]);
  });

  it("each refusal returns the same scene", () => {
    const sixteen = Array.from({ length: 16 }, (_, i) =>
      picture(`p${i}`, { x: 0.1, y: 0.1, w: 0.1, h: 0.1 }, { src: `${i.toString(16).padStart(16, "0")}.png`, nw: 10, nh: 10 }),
    );
    const sixtyFour = Array.from({ length: MAX_ELEMENTS }, (_, i) => ({ id: `t${i}`, type: "timer" }));
    const base = mixedScene();
    const cases: Array<[string, OverlayScene, KnownElement, string]> = [
      ["the id of a live element", base, newPic("A"), "exists"],
      ["the id of an inert element", base, newPic("timer"), "exists"],
      ["64 stored elements", sceneOf(sixtyFour), newPic("N"), "elements"],
      ["a second circle", base, circle("c2", { x: 0.1, y: 0.1, w: 0.2, h: 0.2 }) as unknown as KnownElement, "circle"],
      ["a 17th picture", sceneOf(sixteen), newPic("N"), "pictures"],
      ["nw 0", base, newPic("N", { nw: 0 }), "invalid"],
      ["nw NaN", base, newPic("N", { nw: Number.NaN }), "invalid"],
      ["nh 1.5", base, newPic("N", { nh: 1.5 }), "invalid"],
      ["a .avif src", base, newPic("N", { src: "0123456789abcdef.avif" }), "invalid"],
      ["a path for src", base, newPic("N", { src: "C:/x.png" }), "invalid"],
      ["w 0", base, newPic("N", { w: 0 }), "invalid"],
      ["an id over 64", base, newPic("x".repeat(65)), "invalid"],
      ["an unknown type", base, { ...newPic("N"), type: "timer" } as unknown as KnownElement, "invalid"],
      ["over the byte budget", sceneOf([]), newPic("N", { bytes: 200_000_001 }), "budget"],
    ];
    for (const [name, s, e, why] of cases) {
      expect(addRefusal(s, e), name).toBe(why);
      expect(applyOp(s, { kind: "add", element: e }), name).toBe(s);
    }
  });

  it("70 stored elements are all kept, and add refuses", () => {
    const raw = { elements: Array.from({ length: 70 }, (_, i) => ({ id: `t${i}`, type: "timer", x: 0, y: 0, w: 0.1, h: 0.1 })) };
    const s = readScene(raw, defaultScene());
    expect(s.elements).toHaveLength(70);
    expect(applyOp(s, { kind: "add", element: newPic("N") })).toBe(s);
  });
});

describe("addRefusal: the scene budget (§8.4)", () => {
  const pic = (id: string, src: string, more: Record<string, unknown>): SceneElement =>
    picture(id, { x: 0.1, y: 0.1, w: 0.1, h: 0.1 }, { src, ...more });
  // A still picture of `bytes`, and an animated one of `nw × nh`.
  const file = (id: string, src: string, bytes: number) => pic(id, src, { bytes, anim: false, nw: 1, nh: 1 });
  const anim = (id: string, src: string, nw: number, nh: number) => pic(id, src, { anim: true, nw, nh });
  const GIF = "1111111111111111.gif";
  const PNG = "1111111111111111.png";
  const AVIF = "0123456789abcdef.avif";

  it.each([
    // [name, existing, new, refused]
    ["bytes at the limit", [file("a", SRC_A, 100_000_000)], file("n", SRC_B, 100_000_000), false],
    ["bytes over", [file("a", SRC_A, 100_000_000)], file("n", SRC_B, 100_000_001), true],
    ["a shared file counts once", [file("a", SRC_A, 150_000_000)], file("n", SRC_A, 150_000_000), false],
    ["animated pixels over", [anim("a", SRC_A, 4096, 2048)], anim("n", SRC_B, 4096, 2049), true],
    ["animated pixels at the limit", [anim("a", SRC_A, 4096, 2048)], anim("n", SRC_B, 4096, 2048), false],
    ["anim missing: a .gif counts as animated", [pic("a", SRC_A, { nw: 4096, nh: 2048 })], pic("n", GIF, { nw: 4096, nh: 2049 }), true],
    ["anim missing: a .png doesn't", [pic("a", SRC_B, { nw: 4096, nh: 2048 })], pic("n", PNG, { nw: 4096, nh: 2049 }), false],
    ["all pixels over", [pic("a", SRC_A, { anim: false, nw: 8192, nh: 4096 })], pic("n", SRC_B, { nw: 8192, nh: 4097 }), true],
    ["inert pictures don't count", [file("a", AVIF, 199_000_000)], file("n", SRC_B, 199_000_000), false],
    ["unknown types don't count", [{ id: "s", type: "sticker", src: SRC_A, bytes: 199_000_000 }], file("n", SRC_B, 199_000_000), false],
  ] as Array<[string, SceneElement[], SceneElement, boolean]>)("%s", (_name, existing, add, refused) => {
    expect(addRefusal(sceneOf(existing), add as unknown as KnownElement)).toBe(refused ? "budget" : null);
  });
});

describe("applyOp remove", () => {
  it("removes any element by id, the circle and inert ones included", () => {
    const s = mixedScene();
    for (const id of ["circle", "A", ...INERT_IDS]) {
      const next = applyOp(s, { kind: "remove", id });
      expect(next.elements.map((e) => e.id)).toEqual(s.elements.map((e) => e.id).filter((x) => x !== id));
    }
  });

  it("an unknown id is a no-op", () => {
    const s = mixedScene();
    expect(applyOp(s, { kind: "remove", id: "nope" })).toBe(s);
  });
});

describe("applyOp place", () => {
  it("keeps the picture's own aspect and the 24 px minimum", () => {
    const s = sceneOf([picture("p", { x: 0.1, y: 0.1, w: 0.2, h: 0.1 }, { nw: 2, nh: 1 })]);
    // A square rect for a 2:1 picture: the reducer doesn't stretch.
    const sq = applyOp(s, { kind: "place", id: "p", rect: { x: 100, y: 100, w: 200, h: 200 }, viewport: D480 });
    const r = rectOf(sq, "p", D480);
    expect(r.w / r.h).toBeCloseTo(2, 5);
    expect(r.w).toBeCloseTo(200, 3);
    // Centre kept.
    expect(r.x + r.w / 2).toBeCloseTo(200, 3);
    expect(r.y + r.h / 2).toBeCloseTo(200, 3);
    // Too small: the short side goes to 24.
    const tiny = applyOp(s, { kind: "place", id: "p", rect: { x: 100, y: 100, w: 10, h: 5 }, viewport: D480 });
    const t = rectOf(tiny, "p", D480);
    expect(t.h).toBeCloseTo(24, 3);
    expect(t.w).toBeCloseTo(48, 3);
  });

  it("with no growth, every other element is byte-identical (no re-rounding)", () => {
    const s = mixedScene();
    const next = applyOp(s, { kind: "place", id: "A", rect: { x: 50, y: 60, w: 144, h: 72 }, viewport: { w: 300, h: 300 } });
    expect(next.designSize).toEqual(D480);
    next.elements.forEach((e, i) => {
      if (e.id !== "A") expect(e).toBe(s.elements[i]);
    });
    expect(next.theme).toBe("x");
  });

  it("growing the design size keeps every other element where it draws", () => {
    const s = mixedScene();
    const v = { w: 600, h: 800 };
    const before = (vp: Size) => rectOf(s, CIRCLE_ID, vp);
    const next = applyOp(s, { kind: "place", id: "A", rect: { x: 10, y: 10, w: 144, h: 72 }, viewport: v });
    expect(next.designSize).toEqual({ w: 600, h: 800 });
    for (const vp of [D480, v, { w: 300, h: 300 }, { w: 1000, h: 500 }, { w: 2000, h: 2000 }]) {
      // round6 on the fractions: within a millionth of the viewport.
      expectRectNear(rectOf(next, CIRCLE_ID, vp), before(vp), 2e-6 * Math.max(vp.w, vp.h, 800));
    }
    // An inert element keeps its centre fraction and designed px size too.
    const t0 = s.elements.find((e) => e.id === "timer")!;
    const t1 = next.elements.find((e) => e.id === "timer")!;
    const n = (e: SceneElement, k: "x" | "y" | "w" | "h") => e[k] as number;
    expect(n(t1, "w") * 600).toBeCloseTo(n(t0, "w") * 480, 3);
    expect(n(t1, "h") * 800).toBeCloseTo(n(t0, "h") * 480, 3);
    expect(n(t1, "x") + n(t1, "w") / 2).toBeCloseTo(n(t0, "x") + n(t0, "w") / 2, 5);
    // Without usable geometry: left alone.
    for (const id of ["w0", "wneg", "nox"]) {
      expect(next.elements.find((e) => e.id === id)).toBe(s.elements.find((e) => e.id === id));
    }
  });

  it("round trip: place in a smaller window, and the rest draws as before at the old size", () => {
    // Migrated 480 with a 144 px picture centred at u = 0.85 (x = 336).
    const s = sceneOf([
      circle(),
      picture("P1", { x: 0.7, y: 0.4, w: 0.3, h: 0.15 }, { z: 1 }),
      picture("P2", { x: 0.1, y: 0.1, w: 0.2, h: 0.1 }, { z: 2 }),
    ]);
    const small = { w: 300, h: 300 };
    const p2 = rectOf(s, "P2", small);
    const next = applyOp(s, { kind: "place", id: "P2", rect: moveRect(p2, 10, 5, small), viewport: small });
    expect(next.designSize).toEqual(D480);
    expect(rectOf(next, CIRCLE_ID, D480)).toEqual(rectOf(s, CIRCLE_ID, D480));
    expect(rectOf(next, "P1", D480)).toEqual(rectOf(s, "P1", D480));
    expectRectNear(rectOf(next, CIRCLE_ID, D480), { x: 20, y: 20, w: 440, h: 440 }, 1e-3);
    expect(rectOf(next, "P1", D480).x).toBeCloseTo(336, 9);
  });

  it("mixed axes: each axis grows on its own", () => {
    const s = mixedScene();
    const next = applyOp(s, { kind: "place", id: "A", rect: { x: 10, y: 10, w: 144, h: 72 }, viewport: { w: 600, h: 300 } });
    expect(next.designSize).toEqual({ w: 600, h: 480 });
  });

  it("a nudge of a shrunk element keeps its designed size", () => {
    const s = migrateScene({ w: 480, h: 480, scale: 1 });
    const v = { w: 600, h: 300 };
    const drawn = rectOf(s, CIRCLE_ID, v);
    expect(drawn.w).toBeCloseTo(300, 6);
    const next = applyOp(s, { kind: "place", id: CIRCLE_ID, rect: moveRect(drawn, 10, 0, v), viewport: v });
    const c = next.elements[0]!;
    expect(next.designSize).toEqual({ w: 600, h: 480 });
    expect(c.x).toBeCloseTo(0.15, 5);
    expect(c.w).toBeCloseTo(0.733333, 5);
    expect(c.h).toBeCloseTo(0.916667, 5);
    // Still 440 when there is room again.
    expect(rectOf(next, CIRCLE_ID, D480).w).toBeCloseTo(440, 2);
  });

  it("a stored 300×200 circle that is moved commits a 200×200 square", () => {
    const d = { w: 600, h: 400 };
    const s = sceneOf([circle("c", { x: 0.1, y: 0.1, w: 0.5, h: 0.5 })], d);
    const drawn = rectOf(s, "c", d);
    expectRectNear(drawn, { x: 110, y: 40, w: 200, h: 200 }, 1e-9);
    const next = applyOp(s, { kind: "place", id: "c", rect: moveRect(drawn, 10, 0, d), viewport: d });
    const c = next.elements[0]!;
    expect((c.w as number) * 600).toBeCloseTo(200, 3);
    expect((c.h as number) * 400).toBeCloseTo(200, 3);
    expectRectNear(rectOf(next, "c", d), { x: 120, y: 40, w: 200, h: 200 }, 1e-3);
  });

  it("a resize stores the new size, drawn exactly there", () => {
    const s = mixedScene();
    const next = applyOp(s, { kind: "place", id: "A", rect: { x: 40, y: 30, w: 200, h: 100 }, viewport: D480 });
    expectRectNear(rectOf(next, "A", D480), { x: 40, y: 30, w: 200, h: 100 }, 1e-3);
  });

  it("returns the same scene for an inert or unknown target, a bad viewport or rect, or no change", () => {
    const s = mixedScene();
    const rect = { x: 10, y: 10, w: 50, h: 25 };
    for (const id of [...INERT_IDS, "nope"]) {
      expect(applyOp(s, { kind: "place", id, rect, viewport: D480 }), id).toBe(s);
    }
    for (const viewport of [
      { w: 0, h: 480 },
      { w: 480, h: 0.5 },
      { w: 16385, h: 480 },
      { w: Number.NaN, h: 480 },
    ]) {
      expect(applyOp(s, { kind: "place", id: "A", rect, viewport })).toBe(s);
    }
    for (const bad of [
      { ...rect, w: 0 },
      { ...rect, h: -1 },
      { ...rect, x: Number.NaN },
    ]) {
      expect(applyOp(s, { kind: "place", id: "A", rect: bad, viewport: D480 })).toBe(s);
    }
    expect(applyOp(s, { kind: "place", id: "A", rect: rectOf(s, "A", D480), viewport: D480 })).toBe(s);
  });
});

describe("applyOp order", () => {
  // [circle 0, A 1, timer 2 (inert), B 3]
  const base = (): OverlayScene =>
    sceneOf([
      circle(),
      picture("A", { x: 0.1, y: 0.1, w: 0.1, h: 0.1 }, { z: 1 }),
      { id: "timer", type: "timer", x: 0.1, y: 0.1, w: 0.1, h: 0.1, z: 2, future: { k: [1] } },
      picture("B", { x: 0.1, y: 0.1, w: 0.1, h: 0.1 }, { z: 3 }),
    ]);
  const zs = (s: OverlayScene) => Object.fromEntries(s.elements.map((e) => [e.id, e.z]));
  const drawn = (s: OverlayScene) => layoutScene(s, D480).map((r) => r.el.id);

  it("forward steps over an inert neighbour, past the next live one", () => {
    const next = applyOp(base(), { kind: "order", id: "A", to: "forward" });
    expect(drawn(next)).toEqual(["circle", "B", "A"]);
    expect(zs(next)).toEqual({ circle: 0, timer: 1, B: 2, A: 3 });
    // The timer is still below B, and keeps everything else.
    expect(next.elements[2]).toEqual({ ...base().elements[2], z: 1 });
  });

  it("backward from there", () => {
    const fwd = applyOp(base(), { kind: "order", id: "A", to: "forward" });
    const back = applyOp(fwd, { kind: "order", id: "A", to: "backward" });
    expect(drawn(back)).toEqual(["circle", "A", "B"]);
    expect(zs(back)).toEqual({ circle: 0, timer: 1, A: 2, B: 3 });
  });

  it("backward steps over an inert neighbour, past the next live one", () => {
    const next = applyOp(base(), { kind: "order", id: "B", to: "backward" });
    expect(drawn(next)).toEqual(["circle", "B", "A"]);
    expect(zs(next)).toEqual({ circle: 0, B: 1, A: 2, timer: 3 });
    // Only inert elements below: nothing to step behind.
    const s = sceneOf([
      { id: "timer", type: "timer", z: 0 },
      circle(CIRCLE_ID, undefined, 1),
      picture("A", { x: 0, y: 0, w: 0.1, h: 0.1 }, { z: 2 }),
    ]);
    expect(applyOp(s, { kind: "order", id: CIRCLE_ID, to: "backward" })).toBe(s);
  });

  it("forward on the top drawn element with only an inert one above is the identical scene", () => {
    const s = sceneOf([circle(), picture("A", { x: 0, y: 0, w: 0.1, h: 0.1 }, { z: 1 }), { id: "timer", type: "timer", z: 2 }]);
    const next = applyOp(s, { kind: "order", id: "A", to: "forward" });
    expect(next).toBe(s);
    expect(json(next)).toBe(json(s));
    expect(applyOp(s, { kind: "order", id: CIRCLE_ID, to: "backward" })).toBe(s);
  });

  it("front and back move to the end and renumber 0..n-1", () => {
    const s = sceneOf([
      picture("a", { x: 0, y: 0, w: 0.1, h: 0.1 }, { z: 10 }),
      picture("b", { x: 0, y: 0, w: 0.1, h: 0.1 }, { z: 20 }),
      picture("c", { x: 0, y: 0, w: 0.1, h: 0.1 }, { z: 30 }),
    ]);
    expect(zs(applyOp(s, { kind: "order", id: "a", to: "front" }))).toEqual({ a: 2, b: 0, c: 1 });
    expect(zs(applyOp(s, { kind: "order", id: "c", to: "back" }))).toEqual({ a: 1, b: 2, c: 0 });
    const tidy = applyOp(s, { kind: "order", id: "c", to: "front" });
    expect(zs(tidy)).toEqual({ a: 0, b: 1, c: 2 });
    // Already in front, already numbered: nothing to commit.
    expect(applyOp(tidy, { kind: "order", id: "c", to: "front" })).toBe(tidy);
    expect(applyOp(tidy, { kind: "order", id: "a", to: "back" })).toBe(tidy);
  });

  it("only the target moves: every other pair keeps its order", () => {
    const s = sceneOf([
      circle(),
      picture("A", { x: 0, y: 0, w: 0.1, h: 0.1 }, { z: 1 }),
      { id: "t1", type: "timer", z: 2 },
      picture("B", { x: 0, y: 0, w: 0.1, h: 0.1 }, { z: 3 }),
      picture("bad", { x: 0, y: 0, w: 0.1, h: 0.1 }, { z: 4, src: "x.avif" }),
      picture("C", { x: 0, y: 0, w: 0.1, h: 0.1 }, { z: 5 }),
      { id: "t2", type: "timer", z: 6 },
    ]);
    const rank = (sc: OverlayScene) =>
      [...sc.elements].sort((a, b) => (a.z as number) - (b.z as number)).map((e) => e.id);
    for (const id of ["circle", "A", "B", "C"]) {
      for (const to of ["front", "back", "forward", "backward"] as const) {
        const before = rank(s).filter((x) => x !== id);
        const after = rank(applyOp(s, { kind: "order", id, to })).filter((x) => x !== id);
        expect(after, `${id} ${to}`).toEqual(before);
      }
    }
  });

  it("an inert or unknown target, or an unknown direction, changes nothing", () => {
    const s = base();
    expect(applyOp(s, { kind: "order", id: "timer", to: "front" })).toBe(s);
    expect(applyOp(s, { kind: "order", id: "nope", to: "front" })).toBe(s);
    expect(applyOp(s, { kind: "order", id: "A", to: "up" as "front" })).toBe(s);
  });
});

describe("applyOp reframe", () => {
  const MON = { x: 0, y: 0, w: 1920, h: 1080 };
  const WIN = { x: 100, y: 50, w: 480, h: 480 };

  it("Fit on: nothing moves on screen", () => {
    const s = mixedScene();
    const next = applyOp(s, { kind: "reframe", from: WIN, to: MON, frame: "monitor" });
    expect(next.designSize).toEqual({ w: 1920, h: 1080 });
    expect(next.frame).toBe("monitor");
    for (const id of [CIRCLE_ID, "A"]) {
      const was = rectOf(s, id, D480);
      const screen = { ...was, x: was.x + WIN.x, y: was.y + WIN.y };
      expectRectNear(rectOf(next, id, { w: 1920, h: 1080 }), screen, 2e-3);
    }
    expect(next.theme).toBe("x");
  });

  it("Fit off: what lay outside the window is pulled in, and shrinks if larger", () => {
    const s = sceneOf(
      [
        picture("far", { x: 0.9, y: 0.9, w: 0.05, h: 0.05 }, { nw: 96, nh: 54 }),
        picture("big", { x: 0.1, y: 0.1, w: 0.5, h: 0.6 }, { nw: 960, nh: 648 }),
        picture("inside", { x: 0.1, y: 0.1, w: 0.05, h: 0.05 }, { nw: 96, nh: 54 }),
      ],
      { w: 1920, h: 1080 },
      { frame: "monitor" },
    );
    const next = applyOp(s, { kind: "reframe", from: MON, to: WIN, frame: "window" });
    expect(next.frame).toBe("window");
    expect(next.designSize).toEqual({ w: 480, h: 480 });
    expectRectNear(rectOf(next, "far", D480), { x: 384, y: 426, w: 96, h: 54 }, 1e-3);
    const big = rectOf(next, "big", D480);
    expect(big.w).toBeCloseTo(480, 3);
    expect(big.h).toBeCloseTo(324, 3);
    // Already inside the window's area: it stays where it was on screen.
    expectRectNear(rectOf(next, "inside", D480), { x: 192 - 100, y: 108 - 50, w: 96, h: 54 }, 1e-3);
  });

  it("leaves elements without usable geometry untouched", () => {
    const s = mixedScene();
    const next = applyOp(s, { kind: "reframe", from: WIN, to: MON, frame: "monitor" });
    for (const id of ["w0", "wneg", "nox"]) {
      expect(next.elements.find((e) => e.id === id)).toBe(s.elements.find((e) => e.id === id));
    }
  });

  it("bad rects or frames change nothing", () => {
    const s = mixedScene();
    expect(applyOp(s, { kind: "reframe", from: { ...WIN, w: 0 }, to: MON, frame: "monitor" })).toBe(s);
    expect(applyOp(s, { kind: "reframe", from: WIN, to: { ...MON, h: Number.NaN }, frame: "monitor" })).toBe(s);
    expect(applyOp(s, { kind: "reframe", from: WIN, to: { ...MON, w: 20000 }, frame: "monitor" })).toBe(s);
    expect(applyOp(s, { kind: "reframe", from: WIN, to: MON, frame: "desk" as "monitor" })).toBe(s);
  });
});

describe("unknown fields and inert elements through every op", () => {
  const ops: Array<[string, SceneOp]> = [
    ["add", { kind: "add", element: picture("N", { x: 0.1, y: 0.1, w: 0.2, h: 0.1 }, { src: SRC_B }) as unknown as KnownElement }],
    ["remove", { kind: "remove", id: "A" }],
    ["place", { kind: "place", id: "A", rect: { x: 1, y: 2, w: 100, h: 50 }, viewport: D480 }],
    ["place, growing", { kind: "place", id: "A", rect: { x: 1, y: 2, w: 100, h: 50 }, viewport: { w: 900, h: 700 } }],
    ["order", { kind: "order", id: "A", to: "back" }],
    ["reframe", { kind: "reframe", from: { x: 10, y: 10, w: 480, h: 480 }, to: { x: 0, y: 0, w: 1280, h: 720 }, frame: "monitor" }],
  ];

  it.each(ops)("%s", (_name, op) => {
    const s = mixedScene();
    const next = applyOp(s, op);
    expect(next).not.toBe(s);
    expect(next.theme).toBe("x");
    expect(next.future).toEqual({ nested: [SRC_C] });
    for (const id of INERT_IDS) {
      const was = s.elements.find((e) => e.id === id)!;
      const now = next.elements.find((e) => e.id === id);
      expect(now, id).toBeDefined();
      // Everything but the geometry place/reframe re-express and the z
      // that add/order renumber is byte-identical.
      const strip = (e: SceneElement) => json({ ...e, x: 0, y: 0, w: 0, h: 0, z: 0 });
      expect(strip(now!), id).toBe(strip(was));
      if (!(Number.isFinite(was.x) && Number.isFinite(was.y) && (was.w as number) > 0 && (was.h as number) > 0)) {
        expect([now!.x, now!.y, now!.w, now!.h], id).toEqual([was.x, was.y, was.w, was.h]);
      }
      expect(isLive(next, id), id).toBe(false);
    }
    expect(pictureCount(next)).toBe(op.kind === "remove" ? 0 : op.kind === "add" ? 2 : 1);
  });

  it("the same op twice gives the identical JSON", () => {
    for (const [name, op] of ops) {
      expect(json(applyOp(mixedScene(), op)), name).toBe(json(applyOp(mixedScene(), op)));
    }
  });

  it("never throws on a malformed op", () => {
    const s = mixedScene();
    for (const op of [null, undefined, 3, "place", {}, { kind: "fly" }, { kind: "place" }, { kind: "order", id: "A" }]) {
      expect(applyOp(s, op as unknown as SceneOp)).toBe(s);
    }
  });
});

describe("readOp", () => {
  const place = { kind: "place", id: "A", rect: { x: 1, y: 2, w: 3, h: 4 }, viewport: { w: 480, h: 480 } };

  it("the overlay may send remove, place and order; add and reframe are main's", () => {
    expect(readOp({ kind: "remove", id: "A" }, "overlay")).toEqual({ kind: "remove", id: "A" });
    expect(readOp(place, "overlay")).toEqual(place);
    expect(readOp({ kind: "order", id: "A", to: "forward" }, "overlay")).toEqual({ kind: "order", id: "A", to: "forward" });
    const add = { kind: "add", element: picture("N", { x: 0, y: 0, w: 0.1, h: 0.1 }) };
    const reframe = { kind: "reframe", from: { x: 0, y: 0, w: 1, h: 1 }, to: { x: 0, y: 0, w: 2, h: 2 }, frame: "monitor" };
    expect(readOp(add, "overlay")).toBeNull();
    expect(readOp(reframe, "overlay")).toBeNull();
    expect(readOp(add, "main")).toEqual(add);
    expect(readOp(reframe, "main")).toEqual(reframe);
  });

  it("keeps only the keys it knows", () => {
    expect(readOp({ ...place, extra: 1, rect: { ...place.rect, z: 9 } }, "overlay")).toEqual(place);
  });

  it.each([
    ["not an object", "place"],
    ["no kind", { id: "A" }],
    ["an unknown kind", { kind: "fly", id: "A" }],
    ["an empty id", { kind: "remove", id: "" }],
    ["an id over 64", { kind: "remove", id: "x".repeat(65) }],
    ["a numeric id", { kind: "remove", id: 5 }],
    ["NaN in the rect", { ...place, rect: { ...place.rect, x: Number.NaN } }],
    ["Infinity in the rect", { ...place, rect: { ...place.rect, w: Number.POSITIVE_INFINITY } }],
    ["a zero width", { ...place, rect: { ...place.rect, w: 0 } }],
    ["a negative height", { ...place, rect: { ...place.rect, h: -4 } }],
    ["a string number", { ...place, rect: { ...place.rect, x: "1" } }],
    ["no viewport", { kind: "place", id: "A", rect: place.rect }],
    ["a zero viewport", { ...place, viewport: { w: 0, h: 480 } }],
    ["an unknown order", { kind: "order", id: "A", to: "up" }],
    ["an inherited order name", { kind: "order", id: "A", to: "toString" }],
    ["a bad frame", { kind: "reframe", from: place.rect, to: place.rect, frame: "desk" }],
    ["an add of an unknown type", { kind: "add", element: { id: "N", type: "timer" } }],
  ])("refuses %s", (_name, raw) => {
    expect(readOp(raw, "main")).toBeNull();
    expect(readOp(raw, "overlay")).toBeNull();
  });
});

// ---- rect helpers ----

describe("normalizeRect", () => {
  it("keeps the centre, sets the aspect, fits the viewport, and floors the short side at 24", () => {
    const rows: Array<[string, PxRect, number, Size, PxRect]> = [
      ["as is", { x: 10, y: 20, w: 100, h: 50 }, 2, D480, { x: 10, y: 20, w: 100, h: 50 }],
      ["aspect from the width", { x: 0, y: 0, w: 100, h: 100 }, 2, D480, { x: 0, y: 25, w: 100, h: 50 }],
      ["too big", { x: 0, y: 0, w: 960, h: 480 }, 2, D480, { x: 0, y: 120, w: 480, h: 240 }],
      ["too small", { x: 100, y: 100, w: 10, h: 10 }, 1, D480, { x: 93, y: 93, w: 24, h: 24 }],
      ["thin, held to the viewport", { x: 0, y: 0, w: 100, h: 1 }, 100, D480, { x: 0, y: 0, w: 480, h: 4.8 }],
      ["clamped inside", { x: 470, y: -20, w: 40, h: 40 }, 1, D480, { x: 440, y: 0, w: 40, h: 40 }],
      ["a viewport under 24", { x: 0, y: 0, w: 5, h: 5 }, 1, { w: 20, h: 10 }, { x: 0, y: 0, w: 10, h: 10 }],
    ];
    for (const [name, r, a, v, want] of rows) expectRectNear(normalizeRect(r, a, v), want, 1e-9);
  });
});

describe("scaleRect", () => {
  const v = D480;
  const r = { x: 100, y: 100, w: 100, h: 50 };

  it.each([
    ["small grow", 1, false, 102],
    ["small shrink", -1, false, 98],
    ["big grow", 1, true, 110],
    ["big shrink", -1, true, 90],
  ] as Array<[string, 1 | -1, boolean, number]>)("%s keeps the centre and the aspect", (_n, dir, big, w) => {
    const s = scaleRect(r, dir, big, 2, v);
    expect(s.w).toBeCloseTo(w, 9);
    expect(s.w / s.h).toBeCloseTo(2, 9);
    expect(s.x + s.w / 2).toBeCloseTo(150, 9);
    expect(s.y + s.h / 2).toBeCloseTo(125, 9);
  });

  it("holds the 24 px minimum and the viewport", () => {
    const small = scaleRect({ x: 10, y: 10, w: 48, h: 24 }, -1, true, 2, v);
    expect(small.h).toBeCloseTo(24, 9);
    const edge = scaleRect({ x: 0, y: 0, w: 480, h: 240 }, 1, true, 2, v);
    expectRectNear(edge, { x: 0, y: 0, w: 480, h: 240 }, 1e-9);
    const corner = scaleRect({ x: 400, y: 400, w: 80, h: 80 }, 1, true, 1, v);
    expect(corner.x + corner.w).toBeLessThanOrEqual(480 + 1e-9);
    expect(corner.y + corner.h).toBeLessThanOrEqual(480 + 1e-9);
  });
});

describe("defaultImageRect", () => {
  it("the §3.2 example: a 498×280 GIF at the first spot in a 480 window", () => {
    expect(defaultImageRect(sceneOf([circle()]), 498, 280)).toEqual({ x: 0.6, y: 0.665663, w: 0.3, h: 0.168675 });
  });

  it("cascades through five spots, then wraps", () => {
    let s = sceneOf([circle()]);
    const centres: number[] = [];
    for (let i = 0; i < 6; i += 1) {
      const r = defaultImageRect(s, 100, 100);
      centres.push(Math.round((r.x + r.w / 2) * 1e4) / 1e4);
      s = applyOp(s, {
        kind: "add",
        element: picture(`p${i}`, r, { src: `${i.toString(16).padStart(16, "0")}.png`, nw: 100, nh: 100 }) as unknown as KnownElement,
      });
    }
    expect(centres).toEqual([0.75, 0.71, 0.67, 0.63, 0.59, 0.75]);
  });

  it("sizes: no upscale, the 24 px floor, the viewport limit", () => {
    const px = (r: { w: number; h: number }, d: Size) => [r.w * d.w, r.h * d.h];
    const s480 = sceneOf([]);
    const rows: Array<[number, number, Size | undefined, OverlayScene, number, number]> = [
      [100, 50, undefined, s480, 100, 50],
      [1920, 192, undefined, s480, 240, 24],
      [8192, 100, undefined, s480, 480, 5.859375],
      [500, 500, { w: 480, h: 480 }, sceneOf([], { w: 1200, h: 1200 }), 144, 144],
      [10, 10, undefined, s480, 24, 24],
    ];
    for (const [nw, nh, view, s, w, h] of rows) {
      const [gw, gh] = px(defaultImageRect(s, nw, nh, view), s.designSize);
      expect(gw, `${nw}×${nh}`).toBeCloseTo(w, 3);
      expect(gh, `${nw}×${nh}`).toBeCloseTo(h, 3);
    }
  });

  it("missing pixel sizes fall back to square, never NaN", () => {
    for (const [nw, nh] of [
      [0, 0],
      [0, 5],
      [5, 0],
      [Number.NaN, 5],
    ]) {
      const r = defaultImageRect(sceneOf([]), nw!, nh!);
      for (const v of [r.x, r.y, r.w, r.h]) expect(Number.isFinite(v)).toBe(true);
      expect(r.w).toBeCloseTo(r.h, 6);
    }
  });

  it("fits inside a design size smaller than the window", () => {
    const s = sceneOf([], { w: 100, h: 100 });
    const r = defaultImageRect(s, 1000, 1000, { w: 1000, h: 1000 });
    expect(r.w).toBeLessThanOrEqual(1);
    expect(r.x).toBeGreaterThanOrEqual(0);
    expect(r.x + r.w).toBeLessThanOrEqual(1 + 1e-6);
  });
});

describe("hit boxes", () => {
  const v = D480;

  it("hitRect grows a small element to 24×24, centred, inside the viewport", () => {
    const rows: Array<[string, PxRect, Size, PxRect]> = [
      ["thin", { x: 100, y: 100, w: 144, h: 1.76 }, v, { x: 100, y: 100.88 - 12, w: 144, h: 24 }],
      ["already 24", { x: 10, y: 10, w: 24, h: 24 }, v, { x: 10, y: 10, w: 24, h: 24 }],
      ["left edge", { x: 0, y: 100, w: 4, h: 4 }, v, { x: 0, y: 90, w: 24, h: 24 }],
      ["top edge", { x: 100, y: 0, w: 4, h: 4 }, v, { x: 90, y: 0, w: 24, h: 24 }],
      ["right edge", { x: 476, y: 100, w: 4, h: 4 }, v, { x: 456, y: 90, w: 24, h: 24 }],
      ["bottom edge", { x: 100, y: 476, w: 4, h: 4 }, v, { x: 90, y: 456, w: 24, h: 24 }],
      ["a viewport under 24", { x: 2, y: 2, w: 5, h: 5 }, { w: 20, h: 10 }, { x: 0, y: 0, w: 20, h: 10 }],
      ["big stays", { x: 50, y: 50, w: 200, h: 100 }, v, { x: 50, y: 50, w: 200, h: 100 }],
    ];
    for (const [name, r, vp, want] of rows) expectRectNear(hitRect(r, vp), want, 1e-9);
  });

  it("handleRects centres a 12 px square on each corner", () => {
    expect(handleRects({ x: 10, y: 20, w: 100, h: 50 })).toEqual({
      nw: { x: 4, y: 14, w: 12, h: 12 },
      ne: { x: 104, y: 14, w: 12, h: 12 },
      sw: { x: 4, y: 64, w: 12, h: 12 },
      se: { x: 104, y: 64, w: 12, h: 12 },
    });
  });

  it("hitPart: a corner only inside its handle, a move elsewhere in the box", () => {
    for (const side of [24, 28, 48]) {
      const hit = { x: 100, y: 100, w: side, h: side };
      expect(hitPart(hit, { x: 101, y: 101 })).toBe("nw");
      expect(hitPart(hit, { x: 95, y: 95 })).toBe("nw"); // the outer half of the handle
      expect(hitPart(hit, { x: 100 + side - 1, y: 101 })).toBe("ne");
      expect(hitPart(hit, { x: 101, y: 100 + side - 1 })).toBe("sw");
      expect(hitPart(hit, { x: 100 + side + 5, y: 100 + side + 5 })).toBe("se");
      expect(hitPart(hit, { x: 100 + side / 2, y: 100 + side / 2 })).toBe("move");
      expect(hitPart(hit, { x: 100 + side / 2, y: 101 })).toBe("move"); // the cross between handles
      expect(hitPart(hit, { x: 50, y: 50 })).toBeNull();
    }
    // At 24×24 a quarter is handles, the 12 px cross moves.
    const hit = { x: 0, y: 0, w: 24, h: 24 };
    let move = 0;
    for (let x = 0; x < 24; x += 1) for (let y = 0; y < 24; y += 1) if (hitPart(hit, { x, y }) === "move") move += 1;
    expect(move / 576).toBe(0.75);
  });

  it("a thin hit box still has a move strip between its handles", () => {
    const hit = { x: 0, y: 0, w: 200, h: 24 };
    expect(hitPart(hit, { x: 100, y: 12 })).toBe("move");
    expect(hitPart(hit, { x: 199, y: 23 })).toBe("se");
  });
});

// ---- pictures ----

describe("assetRefs", () => {
  it("counts every asset id anywhere in the scene", () => {
    const s = sceneOf(
      [
        picture("a", { x: 0, y: 0, w: 0.1, h: 0.1 }, { src: SRC_A }),
        picture("b", { x: 0, y: 0, w: 0.1, h: 0.1 }, { src: SRC_A }),
        { id: "st", type: "sticker", icon: SRC_B },
        picture("inert", { x: 0, y: 0, w: 0, h: 0.1 }, { src: SRC_C }),
        picture("avif", { x: 0, y: 0, w: 0.1, h: 0.1 }, { src: "0123456789abcdef.avif" }),
        picture("deep", { x: 0, y: 0, w: 0.1, h: 0.1 }, { src: SRC_B, layers: [{ src: "1111111111111111.jpg" }] }),
      ],
      D480,
      { extras: { bg: "2222222222222222.png" } },
    );
    expect(assetRefCount(s, "9999999999999999.png")).toBe(0);
    expect(assetRefCount(s, SRC_A)).toBe(2);
    expect(assetRefCount(s, SRC_B)).toBe(2);
    expect(assetRefCount(s, SRC_C)).toBe(1);
    expect(assetRefCount(s, "1111111111111111.jpg")).toBe(1);
    expect(assetRefCount(s, "2222222222222222.png")).toBe(1);
    expect(assetRefCount(s, "0123456789abcdef.avif")).toBe(0);
    expect([...assetRefs(s).keys()].sort()).toEqual(
      [SRC_A, SRC_B, SRC_C, "1111111111111111.jpg", "2222222222222222.png"].sort(),
    );
  });

  it("a nested value two elements share counts for each; a cycle doesn't hang", () => {
    const shared = { src: SRC_A };
    const s = sceneOf([
      { id: "a", type: "sticker", layer: shared },
      { id: "b", type: "sticker", layer: shared },
    ]);
    expect(assetRefCount(s, SRC_A)).toBe(2);
    const loop: Record<string, unknown> = { icon: SRC_B };
    loop.self = loop;
    expect(assetRefCount(sceneOf([{ id: "c", type: "sticker", loop }]), SRC_B)).toBe(1);
  });

  it("removing the last user brings the count to 0, and only then", () => {
    let s = sceneOf([picture("a", { x: 0, y: 0, w: 0.1, h: 0.1 }), picture("b", { x: 0, y: 0, w: 0.1, h: 0.1 })]);
    s = applyOp(s, { kind: "remove", id: "a" });
    expect(assetRefCount(s, SRC_A)).toBe(1);
    s = applyOp(s, { kind: "remove", id: "b" });
    expect(assetRefCount(s, SRC_A)).toBe(0);
  });
});

describe("charmLabels", () => {
  const named = (id: string, name?: unknown) =>
    picture(id, { x: 0, y: 0, w: 0.1, h: 0.1 }, name === undefined ? {} : { name });

  it.each([
    ["unique names unchanged", [named("a", "wave.gif"), named("b", "cat.png")], ["wave.gif", "cat.png"]],
    [
      "repeats numbered",
      [named("a", "wave.gif"), named("b", "wave.gif"), named("c", "wave.gif")],
      ["wave.gif", "wave.gif (2)", "wave.gif (3)"],
    ],
    ["unnamed", [named("a"), named("b"), named("c", "  ")], ["Picture", "Picture (2)", "Picture (3)"]],
    [
      "a real name that looks like a suffix",
      [named("a", "wave.gif"), named("b", "wave.gif"), named("c", "wave.gif (2)")],
      ["wave.gif", "wave.gif (3)", "wave.gif (2)"],
    ],
    [
      "the suffix comes first",
      [named("a", "wave.gif (2)"), named("b", "wave.gif"), named("c", "wave.gif")],
      ["wave.gif (2)", "wave.gif", "wave.gif (3)"],
    ],
  ] as Array<[string, SceneElement[], string[]]>)("%s", (_n, pics, want) => {
    const labels = charmLabels(sceneOf(pics));
    expect(pics.map((p) => labels.get(p.id))).toEqual(want);
    expect(new Set(labels.values()).size).toBe(pics.length);
  });

  it("the circle is the Spell circle; inert elements have no label", () => {
    const labels = charmLabels(mixedScene());
    expect([...labels]).toEqual([
      ["circle", "Spell circle"],
      ["A", "wave.gif"],
    ]);
  });

  it("adding a copy leaves the existing labels unchanged", () => {
    const s = sceneOf([named("a", "wave.gif"), named("b", "wave.gif"), named("c", "cat.png")]);
    const before = charmLabels(s);
    const after = charmLabels(applyOp(s, { kind: "add", element: named("d", "wave.gif") as unknown as KnownElement }));
    for (const [id, label] of before) expect(after.get(id)).toBe(label);
    expect(after.get("d")).toBe("wave.gif (3)");
  });

  it("long names are cut to 120 characters", () => {
    const labels = charmLabels(sceneOf([named("a", "x".repeat(300))]));
    expect(labels.get("a")).toHaveLength(120);
  });
});
