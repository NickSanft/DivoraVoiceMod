import { describe, expect, it } from "vitest";
import {
  addRefusal,
  applyOp,
  asRec,
  asText,
  asTimer,
  aspectOf,
  assetRefCount,
  assetRefs,
  ASSET_ID_RE,
  charmLabels,
  charmUrl,
  CHARM_ORIGIN,
  CIRCLE_ID,
  CIRCLE_PX,
  defaultImageRect,
  defaultLightRect,
  defaultScene,
  defaultTextRect,
  defaultTimerRect,
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
  setRefusal,
  type KnownElement,
  type OverlayScene,
  type PxRect,
  type SceneElement,
  type SceneOp,
  type Size,
  type TimerElement,
} from "./scene";
import { SPELL_CIRCLE_PX } from "../components/SpellCircle";
import type { TextPatch } from "./text";
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
    { id: "future", type: "future", x: 0.1, y: 0.1, w: 0.2, h: 0.1, z: 2, future: { a: [1, 2], b: "x" } },
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
    // v1.59.0: a timer face whose ref fails the id grammar.
    { id: "badref", type: "timer", x: 0.3, y: 0.6, w: 0.3, h: 0.1, z: 3, timer: "Timer-1" },
  ];
}

const INERT_IDS = ["future", "avif", "w0", "wneg", "nox", "circle2", "nosrc", "srcnum", "badref"];

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

  it("v1.62.0: a stored text's lone surrogate half becomes U+FFFD, so the scene can still be pushed", () => {
    // JSON.stringify writes a lone half as an escape that Rust's serde_json
    // refuses, which would fail every push of the scene.
    const lone = /\\ud[89a-f][0-9a-f]{2}/i;
    const half = String.fromCharCode(0xd83c);
    const raw = JSON.parse(JSON.stringify({ elements: [] })) as { elements: SceneElement[] };
    const geo = { x: 0.1, y: 0.1, w: 0.2, h: 0.1, nw: 300, nh: 139 };
    const bad = { id: "t", type: "text", ...geo, z: 1, text: ` BRB ${half}\r\n`, future: { k: 1 } };
    const good = { id: "u", type: "text", ...geo, y: 0.5, z: 2, text: " 🎮 \r\n" };
    const pic = picture("p", { x: 0.5, y: 0.5, w: 0.1, h: 0.1 }, { name: "a\tb" });
    raw.elements.push(bad, good, pic);
    expect(JSON.stringify(raw)).toMatch(lone);
    const s = readScene(raw, fb);
    expect(JSON.stringify(s)).not.toMatch(lone);
    // Only that half changes; the rest of the text and every other key are
    // kept as stored.
    expect(s.elements[0]).toEqual({ ...bad, text: ` BRB �\r\n` });
    expect(s.elements[1]).toBe(good);
    expect(s.elements[2]).toBe(pic);
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
    const sixtyFour = Array.from({ length: MAX_ELEMENTS }, (_, i) => ({ id: `t${i}`, type: "future" }));
    const base = mixedScene();
    const cases: Array<[string, OverlayScene, KnownElement, string]> = [
      ["the id of a live element", base, newPic("A"), "exists"],
      ["the id of an inert element", base, newPic("future"), "exists"],
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
      ["an unknown type", base, { ...newPic("N"), type: "future" } as unknown as KnownElement, "invalid"],
      ["over the byte budget", sceneOf([]), newPic("N", { bytes: 200_000_001 }), "budget"],
    ];
    for (const [name, s, e, why] of cases) {
      expect(addRefusal(s, e), name).toBe(why);
      expect(applyOp(s, { kind: "add", element: e }), name).toBe(s);
    }
  });

  it("70 stored elements are all kept, and add refuses", () => {
    const raw = { elements: Array.from({ length: 70 }, (_, i) => ({ id: `t${i}`, type: "future", x: 0, y: 0, w: 0.1, h: 0.1 })) };
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
    const t0 = s.elements.find((e) => e.id === "future")!;
    const t1 = next.elements.find((e) => e.id === "future")!;
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
  // [circle 0, A 1, future 2 (an inert type), B 3]
  const base = (): OverlayScene =>
    sceneOf([
      circle(),
      picture("A", { x: 0.1, y: 0.1, w: 0.1, h: 0.1 }, { z: 1 }),
      { id: "future", type: "future", x: 0.1, y: 0.1, w: 0.1, h: 0.1, z: 2, future: { k: [1] } },
      picture("B", { x: 0.1, y: 0.1, w: 0.1, h: 0.1 }, { z: 3 }),
    ]);
  const zs = (s: OverlayScene) => Object.fromEntries(s.elements.map((e) => [e.id, e.z]));
  const drawn = (s: OverlayScene) => layoutScene(s, D480).map((r) => r.el.id);

  it("forward steps over an inert neighbour, past the next live one", () => {
    const next = applyOp(base(), { kind: "order", id: "A", to: "forward" });
    expect(drawn(next)).toEqual(["circle", "B", "A"]);
    expect(zs(next)).toEqual({ circle: 0, future: 1, B: 2, A: 3 });
    // The inert one is still below B, and keeps everything else.
    expect(next.elements[2]).toEqual({ ...base().elements[2], z: 1 });
  });

  it("backward from there", () => {
    const fwd = applyOp(base(), { kind: "order", id: "A", to: "forward" });
    const back = applyOp(fwd, { kind: "order", id: "A", to: "backward" });
    expect(drawn(back)).toEqual(["circle", "A", "B"]);
    expect(zs(back)).toEqual({ circle: 0, future: 1, A: 2, B: 3 });
  });

  it("backward steps over an inert neighbour, past the next live one", () => {
    const next = applyOp(base(), { kind: "order", id: "B", to: "backward" });
    expect(drawn(next)).toEqual(["circle", "B", "A"]);
    expect(zs(next)).toEqual({ circle: 0, B: 1, A: 2, future: 3 });
    // Only inert elements below: nothing to step behind.
    const s = sceneOf([
      { id: "future", type: "future", z: 0 },
      circle(CIRCLE_ID, undefined, 1),
      picture("A", { x: 0, y: 0, w: 0.1, h: 0.1 }, { z: 2 }),
    ]);
    expect(applyOp(s, { kind: "order", id: CIRCLE_ID, to: "backward" })).toBe(s);
  });

  it("forward on the top drawn element with only an inert one above is the identical scene", () => {
    const s = sceneOf([circle(), picture("A", { x: 0, y: 0, w: 0.1, h: 0.1 }, { z: 1 }), { id: "future", type: "future", z: 2 }]);
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
      { id: "t1", type: "future", z: 2 },
      picture("B", { x: 0, y: 0, w: 0.1, h: 0.1 }, { z: 3 }),
      picture("bad", { x: 0, y: 0, w: 0.1, h: 0.1 }, { z: 4, src: "x.avif" }),
      picture("C", { x: 0, y: 0, w: 0.1, h: 0.1 }, { z: 5 }),
      { id: "t2", type: "future", z: 6 },
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
    expect(applyOp(s, { kind: "order", id: "future", to: "front" })).toBe(s);
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
    ["an add of an unknown type", { kind: "add", element: { id: "N", type: "future" } }],
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

// v1.59.0: Hourglass's timer faces. The scene's `"future"` fixtures above
// stand in for a type this version doesn't know, as `"timer"` did before.
describe("timer elements (v1.59.0)", () => {
  const face = (more: Record<string, unknown> = {}): SceneElement => ({
    id: "c-0a1b2c3d",
    type: "timer",
    x: 0.35,
    y: 0.08,
    w: 0.3,
    h: 0.1,
    z: 1,
    timer: "timer-1",
    ...more,
  });

  it("asTimer takes usable geometry and a timer id, whether or not that timer exists", () => {
    const e = face();
    expect(asTimer(e)).toBe(e);
    expect(asTimer(face({ timer: "brb", future: { k: 1 } }))).not.toBeNull();
  });

  it.each([
    ["a timer ref that fails the grammar", { timer: "Timer-1" }],
    ["a device name for a ref", { timer: "con" }],
    ["no ref", { timer: undefined }],
    ["a number for a ref", { timer: 1 }],
    ["a zero width", { w: 0 }],
    ["no x", { x: undefined }],
    ["an id over 64", { id: "x".repeat(65) }],
    ["another type", { type: "image" }],
  ])("asTimer refuses %s", (_name, more) => {
    expect(asTimer(face(more))).toBeNull();
  });

  it("aspectOf keeps a timer's stored shape", () => {
    expect(aspectOf(face() as unknown as TimerElement, D480)).toBeCloseTo(3, 9);
    expect(aspectOf(face({ w: 0.2, h: 0.2 }) as unknown as TimerElement, { w: 960, h: 480 })).toBeCloseTo(2, 9);
  });

  it("defaultTimerRect is 3:1, centred across, its height 12 % of the window's short side", () => {
    const r = defaultTimerRect(sceneOf([circle()]), { w: 480, h: 480 });
    const pw = r.w * 480;
    const ph = r.h * 480;
    expect(ph).toBeCloseTo(57.6, 3);
    expect(pw / ph).toBeCloseTo(3, 3);
    expect(r.x + r.w / 2).toBeCloseTo(0.5, 5);
    expect(r.y).toBeCloseTo(0.08, 6);
  });

  it("its height stays within 32 to 160 px", () => {
    const small = defaultTimerRect(sceneOf([]), { w: 120, h: 120 });
    expect(small.h * 480).toBeCloseTo(32, 3);
    const big = defaultTimerRect(sceneOf([], { w: 3840, h: 2160 }), { w: 3840, h: 2160 });
    expect(big.h * 2160).toBeCloseTo(160, 3);
  });

  it("each timer face already there puts the next 6 % lower, five places round", () => {
    const tops: number[] = [];
    const elements: SceneElement[] = [];
    for (let i = 0; i < 6; i += 1) {
      const r = defaultTimerRect(sceneOf(elements), D480);
      tops.push(r.y);
      elements.push(face({ id: `c-${i}`, ...r }));
    }
    expect(tops.map((y) => Math.round(y * 100))).toEqual([8, 14, 20, 26, 32, 8]);
    // A face that isn't usable doesn't count.
    expect(defaultTimerRect(sceneOf([face({ w: 0 })]), D480).y).toBeCloseTo(0.08, 6);
  });

  it("stays inside the scene, even in a viewport larger than its design", () => {
    for (const view of [
      { w: 4000, h: 4000 },
      { w: 30, h: 30 },
      { w: 2000, h: 200 },
    ]) {
      const r = defaultTimerRect(sceneOf([]), view);
      expect(r.x).toBeGreaterThanOrEqual(0);
      expect(r.y).toBeGreaterThanOrEqual(0);
      expect(r.x + r.w).toBeLessThanOrEqual(1 + 1e-6);
      expect(r.y + r.h).toBeLessThanOrEqual(1 + 1e-6);
    }
  });

  it("charmLabels takes a timer's name and labels the rest as before", () => {
    const s = sceneOf([circle(), picture("A", { x: 0, y: 0, w: 0.1, h: 0.1 }, { name: "wave.gif" })]);
    expect(charmLabels(s, () => "BRB")).toEqual(charmLabels(s));
  });

  it("readOp still refuses a timer add: main adds one locally, the overlay never can", () => {
    for (const from of ["main", "overlay"] as const) {
      expect(readOp({ kind: "add", element: face() }, from)).toBeNull();
    }
  });

  it("a face is live whether or not its timer exists; a bad ref keeps it inert, as stored", () => {
    const s = sceneOf([
      circle(),
      face({ id: "brb" }),
      face({ id: "gone", timer: "timer-9" }),
      face({ id: "bad", timer: "timer:1" }),
      face({ id: "flat", h: 0 }),
    ]);
    expect(liveElements(s).map((e) => e.id)).toEqual(["circle", "brb", "gone"]);
    expect(layoutScene(s, D480).map((r) => r.el.id)).toEqual(["circle", "brb", "gone"]);
    expect(isLive(s, "bad")).toBe(false);
    expect(isLive(s, "flat")).toBe(false);
    // Faces aren't pictures.
    expect(pictureCount(s)).toBe(0);
    // Kept as stored through an edit of another element.
    const next = applyOp(s, { kind: "place", id: "brb", rect: { x: 10, y: 10, w: 144, h: 48 }, viewport: D480 });
    expect(next).not.toBe(s);
    expect(next.elements.find((e) => e.id === "bad")).toBe(s.elements.find((e) => e.id === "bad"));
    expect(applyOp(s, { kind: "place", id: "bad", rect: { x: 10, y: 10, w: 144, h: 48 }, viewport: D480 })).toBe(s);
  });

  it("add puts a face in front; only a taken id, 64 stored elements or a face that wouldn't be live refuse it", () => {
    const sixteen = Array.from({ length: 16 }, (_, i) =>
      picture(`p${i}`, { x: 0.1, y: 0.1, w: 0.1, h: 0.1 }, { src: `${i.toString(16).padStart(16, "0")}.png`, nw: 10, nh: 10 }),
    );
    const sixtyFour = Array.from({ length: MAX_ELEMENTS }, (_, i) => ({ id: `t${i}`, type: "future" }));
    const overBudget = [picture("huge", { x: 0.1, y: 0.1, w: 0.1, h: 0.1 }, { bytes: 300_000_000 })];
    const full = sceneOf([circle(), ...sixteen]);
    const add = (more: Record<string, unknown> = {}) => face({ id: "N", ...more }) as unknown as KnownElement;
    const cases: Array<[string, OverlayScene, KnownElement, string | null]> = [
      ["beside a circle and 16 pictures", full, add(), null],
      ["a scene already over the picture budget", sceneOf(overBudget), add(), null],
      ["a second face for the same timer (the controller keeps one)", sceneOf([face()]), add(), null],
      ["a face for a timer that doesn't exist", sceneOf([]), add({ timer: "timer-9" }), null],
      ["a taken id", sceneOf([face({ id: "N" })]), add(), "exists"],
      ["64 stored elements", sceneOf(sixtyFour), add(), "elements"],
      ["a ref that fails the grammar", sceneOf([]), add({ timer: "Timer-1" }), "invalid"],
      ["a device name for a ref", sceneOf([]), add({ timer: "nul" }), "invalid"],
      ["no ref", sceneOf([]), add({ timer: undefined }), "invalid"],
      ["a zero height", sceneOf([]), add({ h: 0 }), "invalid"],
    ];
    for (const [name, s, e, why] of cases) {
      expect(addRefusal(s, e), name).toBe(why);
      const next = applyOp(s, { kind: "add", element: e });
      if (why === null) expect(next.elements.at(-1), name).toMatchObject({ id: "N", type: "timer", timer: e.timer });
      else expect(next, name).toBe(s);
    }
    const next = applyOp(full, { kind: "add", element: add() });
    expect(layoutScene(next, D480).at(-1)!.el.id).toBe("N");
    expect(isLive(next, "N")).toBe(true);
  });

  it("place, order, remove and reframe treat a face as they treat a picture", () => {
    const s = sceneOf([circle(), picture("A", { x: 0.1, y: 0.6, w: 0.2, h: 0.1 }, { z: 1 }), face({ z: 2 })]);
    const id = "c-0a1b2c3d";
    // Moved: the same size, its centre where it was dropped.
    const moved = applyOp(s, { kind: "place", id, rect: { x: 10, y: 300, w: 144, h: 48 }, viewport: D480 });
    expectRectNear(rectOf(moved, id, D480), { x: 10, y: 300, w: 144, h: 48 }, 1e-3);
    // Resized from a square rect: its stored 3:1 shape wins.
    const sized = applyOp(s, { kind: "place", id, rect: { x: 100, y: 100, w: 240, h: 240 }, viewport: D480 });
    const r = rectOf(sized, id, D480);
    expect(r.w).toBeCloseTo(240, 3);
    expect(r.w / r.h).toBeCloseTo(3, 4);
    // Restacked.
    expect(layoutScene(applyOp(s, { kind: "order", id, to: "back" }), D480).map((x) => x.el.id)).toEqual([id, CIRCLE_ID, "A"]);
    // Removed.
    expect(applyOp(s, { kind: "remove", id }).elements.map((e) => e.id)).toEqual([CIRCLE_ID, "A"]);
    // Fit on: nothing moves on screen.
    const win = { x: 100, y: 50, w: 480, h: 480 };
    const mon = { x: 0, y: 0, w: 1920, h: 1080 };
    const fitted = applyOp(s, { kind: "reframe", from: win, to: mon, frame: "monitor" });
    const was = rectOf(s, id, D480);
    expectRectNear(rectOf(fitted, id, { w: 1920, h: 1080 }), { ...was, x: was.x + win.x, y: was.y + win.y }, 2e-3);
  });

  it("charmLabels names faces by their timers, numbering repeats with the rest", () => {
    const names: Record<string, string> = { "timer-1": "BRB", "timer-2": "BRB", "timer-3": "Uptime" };
    const s = sceneOf([
      circle(),
      picture("A", { x: 0, y: 0, w: 0.1, h: 0.1 }, { name: "BRB" }),
      face({ id: "f1" }),
      face({ id: "f2", timer: "timer-2" }),
      face({ id: "f3", timer: "timer-3" }),
      face({ id: "f4", timer: "timer-9" }),
      face({ id: "f5", timer: "timer-8" }),
    ]);
    expect([...charmLabels(s, (t) => names[t])]).toEqual([
      ["circle", "Spell circle"],
      ["A", "BRB"],
      ["f1", "BRB (2)"],
      ["f2", "BRB (3)"],
      ["f3", "Uptime"],
      ["f4", "Timer"],
      ["f5", "Timer (2)"],
    ]);
    // Without names (as before v1.59.0's callers pass them), every face is "Timer".
    expect(charmLabels(s).get("f3")).toBe("Timer (3)");
  });
});

// v1.62.0: text charms and the recording light. Main adds both itself and
// edits a text with `set`; older builds keep both inert (the `"future"`
// fixtures above stand in for them there).
describe("text charms and the recording light (v1.62.0)", () => {
  const T = "c-7e570001";
  const L = "c-11647001";
  // 240 × 48 px in a 480 design: 5 : 1, its own shape (`nw : nh`).
  const text = (more: Record<string, unknown> = {}): SceneElement => ({
    id: T,
    type: "text",
    x: 0.25,
    y: 0.4,
    w: 0.5,
    h: 0.1,
    z: 1,
    text: "BRB",
    font: "sans",
    bold: true,
    color: "#ffffff",
    outline: true,
    align: "center",
    nw: 500,
    nh: 100,
    ...more,
  });
  // 168 × 33.6 px at (19.2, 427.2): defaultLightRect's in a 480 view.
  const light = (more: Record<string, unknown> = {}): SceneElement => ({
    id: L,
    type: "rec",
    x: 0.04,
    y: 0.89,
    w: 0.35,
    h: 0.07,
    z: 2,
    ...more,
  });
  const known = (e: SceneElement): KnownElement => e as unknown as KnownElement;
  const edit = (s: OverlayScene, fields: Record<string, unknown>, id = T): OverlayScene =>
    applyOp(s, { kind: "set", id, fields: fields as unknown as TextPatch });
  const at = (s: OverlayScene, id = T): SceneElement => s.elements.find((e) => e.id === id)!;
  const box = (e: SceneElement) => ({ x: e.x, y: e.y, w: e.w, h: e.h });
  const ch = (...codes: number[]): string => String.fromCharCode(...codes);
  const GAME = "🎮";
  const sixteen = (): SceneElement[] =>
    Array.from({ length: 16 }, (_, i) =>
      picture(`p${i}`, { x: 0.1, y: 0.1, w: 0.1, h: 0.1 }, { src: `${i.toString(16).padStart(16, "0")}.png`, nw: 10, nh: 10 }),
    );
  const sixtyFour = (): SceneElement[] => Array.from({ length: MAX_ELEMENTS }, (_, i) => ({ id: `t${i}`, type: "future" }));

  // ---- readers ----

  it("text and rec readers: usable geometry and a valid id; a text also needs words", () => {
    const t = text();
    expect(asText(t)).toBe(t);
    // Every other field has a read default, so these are live too.
    expect(asText(text({ font: "serif", color: "red", align: 3, nw: undefined, future: { k: 1 } }))).not.toBeNull();
    const l = light();
    expect(asRec(l)).toBe(l);
    expect(asRec(light({ future: "x" }))).not.toBeNull();
  });

  it.each([
    ["an empty text", { text: "" }],
    ["only spaces and newlines", { text: " \n \r\n\t" }],
    ["only control characters", { text: ch(0x07, 0x85, 0x7f, 0x90) }],
    ["a number for a text", { text: 5 }],
    ["no text", { text: undefined }],
    ["a zero width", { w: 0 }],
    ["a negative height", { h: -0.1 }],
    ["no x", { x: undefined }],
    ["NaN y", { y: Number.NaN }],
    ["an empty id", { id: "" }],
    ["an id over 64", { id: "x".repeat(65) }],
    ["another type", { type: "image" }],
  ])("asText refuses %s, and the element stays inert", (_name, more) => {
    const e = text(more);
    expect(asText(e)).toBeNull();
    expect(liveElements(sceneOf([e]))).toEqual([]);
  });

  it.each([
    ["a zero height", { h: 0 }],
    ["a negative width", { w: -0.35 }],
    ["no y", { y: undefined }],
    ["an id over 64", { id: "x".repeat(65) }],
    ["another type", { type: "text" }],
  ])("asRec refuses %s", (_name, more) => {
    expect(asRec(light(more))).toBeNull();
  });

  it("a 201-unit or 6-line text is live and drawn; add and set refuse it", () => {
    // Limits apply when adding or setting, never when reading: a later
    // build's longer text isn't hidden and stranded here.
    const s = sceneOf([circle(), text({ id: "long", text: "x".repeat(201) }), text({ id: "six", text: "1\n2\n3\n4\n5\n6" })]);
    expect(liveElements(s).map((e) => e.id)).toEqual([CIRCLE_ID, "long", "six"]);
    expect(layoutScene(s, D480).map((r) => r.el.id)).toEqual([CIRCLE_ID, "long", "six"]);
    for (const raw of ["x".repeat(201), "1\n2\n3\n4\n5\n6", `${GAME.repeat(100)}x`]) {
      const n = known(text({ id: "N", text: raw }));
      expect(addRefusal(s, n), raw).toBe("invalid");
      expect(applyOp(s, { kind: "add", element: n }), raw).toBe(s);
      expect(setRefusal(s, "long", { nw: 500, nh: 100, text: raw }), raw).toBe("invalid");
      expect(edit(s, { nw: 500, nh: 100, text: raw }, "long"), raw).toBe(s);
    }
    // Within the limits, set takes a new text for it.
    expect(at(edit(s, { nw: 500, nh: 100, text: "short" }, "long"), "long").text).toBe("short");
  });

  it("one live light: the scene's first usable one; another is kept, inert", () => {
    const s = sceneOf([light({ id: "flat", h: 0 }), light({ id: "L1" }), light({ id: "L2" }), circle()]);
    expect(liveElements(s).map((e) => e.id)).toEqual(["L1", CIRCLE_ID]);
    expect(isLive(s, "L2")).toBe(false);
    expect(isLive(s, "flat")).toBe(false);
    expect(pictureCount(s)).toBe(0);
    // Inert: edits of it change nothing.
    expect(applyOp(s, { kind: "place", id: "L2", rect: { x: 10, y: 10, w: 100, h: 20 }, viewport: D480 })).toBe(s);
    expect(applyOp(s, { kind: "order", id: "L2", to: "front" })).toBe(s);
    // Removing the live one brings the next one in.
    expect(liveElements(applyOp(s, { kind: "remove", id: "L1" })).map((e) => e.id)).toEqual(["L2", CIRCLE_ID]);
  });

  it("aspectOf uses a text's nw/nh, and its stored shape when they're missing", () => {
    expect(aspectOf(known(text({ nw: 337, nh: 139 })), D480)).toBeCloseTo(337 / 139, 12);
    for (const more of [{ nw: undefined }, { nh: 0 }, { nw: 1.5 }, { nw: "337" }, { nh: Number.NaN }]) {
      // Stored 240 × 48.
      expect(aspectOf(known(text({ nw: 337, nh: 139, ...more })), D480), JSON.stringify(more)).toBeCloseTo(5, 9);
    }
    // The light keeps its stored shape, as a timer face does.
    expect(aspectOf(known(light()), D480)).toBeCloseTo(5, 9);
    expect(aspectOf(known(light({ w: 0.2, h: 0.2 })), { w: 960, h: 480 })).toBeCloseTo(2, 9);
  });

  it("a text stored off its own shape (a hand edit) still resizes, to its own shape", () => {
    // Stored 240 × 48 (5 : 1) with a natural box of 337 : 139. Its corner
    // drags carry the 5 : 1 it is drawn in; they land as resizes to
    // 337 : 139, as a picture's do, not as stale moves.
    const s = sceneOf([text({ nw: 337, nh: 139 })]);
    const next = applyOp(s, { kind: "place", id: T, rect: { x: 120, y: 192, w: 300, h: 60 }, viewport: D480 });
    const r = rectOf(next, T, D480);
    expect(r.w).toBeCloseTo(300, 3);
    expect(r.w / r.h).toBeCloseTo(337 / 139, 4);
  });

  // ---- set ----

  it.each([
    // [name, stored align, stored rect, new nw, new nh, result]
    ["centre keeps the centre", "center", { x: 0.25, y: 0.4, w: 0.5, h: 0.1 }, 337, 139, { x: 0.378777, y: 0.4, w: 0.242446, h: 0.1 }],
    ["left keeps the left edge", "left", { x: 0.25, y: 0.4, w: 0.5, h: 0.1 }, 337, 139, { x: 0.25, y: 0.4, w: 0.242446, h: 0.1 }],
    ["right keeps the right edge", "right", { x: 0.25, y: 0.4, w: 0.5, h: 0.1 }, 337, 139, { x: 0.507554, y: 0.4, w: 0.242446, h: 0.1 }],
    // 48 · 4000/139 = 1381.295 px: scaled down together to 480 × 16.68.
    ["too wide: fitted to the design, the vertical centre kept", "center", { x: 0.25, y: 0.4, w: 0.5, h: 0.1 }, 4000, 139, { x: 0, y: 0.432625, w: 1, h: 0.03475 }],
    ["right, past the left edge: clamped inside", "right", { x: 0.02, y: 0.4, w: 0.1, h: 0.1 }, 300, 100, { x: 0, y: 0.4, w: 0.3, h: 0.1 }],
    ["left, past the right edge: clamped inside", "left", { x: 0.9, y: 0.4, w: 0.1, h: 0.1 }, 300, 100, { x: 0.7, y: 0.4, w: 0.3, h: 0.1 }],
  ])("set table: a new shape keeps the height and the side its alignment names: %s", (_n, align, r, nw, nh, want) => {
    const e = text({ ...r, align, nw: Math.round(r.w * 480), nh: Math.round(r.h * 480) });
    const s = sceneOf([circle(), e, light()]);
    const next = edit(s, { text: "new words", nw, nh });
    expect(box(at(next))).toEqual(want);
    // The id, z, type and every untouched field kept.
    expect(at(next)).toEqual({ ...e, ...want, text: "new words", nw, nh });
    // Nothing else changes.
    expect(next.elements[0]).toBe(s.elements[0]);
    expect(next.elements[2]).toBe(s.elements[2]);
    expect(next.designSize).toEqual(s.designSize);
  });

  it("set keeps the aligned side where it is drawn in a window narrower than the design", () => {
    // designSize grew to 1920 × 1080 in an earlier, bigger window; the
    // window is now 960 × 540. Layout keeps a centre fraction, so the side
    // is kept on screen, not as a fraction of the design.
    const d = { w: 1920, h: 1080 };
    const v = { w: 960, h: 540 };
    const designed = { x: 0.3, y: 0.3, w: 600 / 1920, h: 100 / 1080 };
    for (const [align, rect] of [
      // Snapped to the window's left edge, and to its right edge.
      ["left", { x: 0, y: 100, w: 600, h: 100 }],
      ["right", { x: 360, y: 300, w: 600, h: 100 }],
      ["center", { x: 100, y: 200, w: 600, h: 100 }],
    ] as const) {
      const s0 = sceneOf([text({ ...designed, align, nw: 600, nh: 100 })], d);
      const s = applyOp(s0, { kind: "place", id: T, rect, viewport: v });
      expectRectNear(rectOf(s, T, v), rect, 2e-3);
      // Shortened to 200 × 100.
      const next = applyOp(s, { kind: "set", id: T, fields: { text: "hi", nw: 200, nh: 100 }, viewport: v });
      const after = rectOf(next, T, v);
      expect(after.w, align).toBeCloseTo(200, 2);
      expect(after.h, align).toBeCloseTo(100, 2);
      if (align === "left") expect(after.x, align).toBeCloseTo(0, 2);
      if (align === "right") expect(after.x + after.w, align).toBeCloseTo(960, 2);
      if (align === "center") expect(after.x + after.w / 2, align).toBeCloseTo(400, 2);
      expect(after.y + after.h / 2, align).toBeCloseTo(rect.y + 50, 2);
      // Without a viewport, the design size's own: the side kept as a
      // fraction of it, as in a window of that size.
      const bare = applyOp(s, { kind: "set", id: T, fields: { text: "hi", nw: 200, nh: 100 } });
      const there = rectOf(bare, T, d);
      const was = rectOf(s, T, d);
      if (align === "left") expect(there.x, align).toBeCloseTo(was.x, 2);
      if (align === "right") expect(there.x + there.w, align).toBeCloseTo(was.x + was.w, 2);
    }
  });

  it("set keeps the aligned side, and the box inside, in a window wider than the design", () => {
    // The overlay was enlarged with no drag since, so the design is still
    // 480 × 480 and the window draws a centre fraction of its own width: a
    // stored x of 0 draws 50 px in. Keeping the side inside the design's
    // fractions would move it.
    const d = { w: 480, h: 480 };
    const v = { w: 960, h: 540 };
    for (const [align, x, side] of [
      ["left", 0, 50],
      ["right", 1 - 100 / 480, 910],
    ] as const) {
      const s = sceneOf([text({ x, y: 0.3, w: 100 / 480, h: 100 / 480, align, nw: 100, nh: 100 })], d);
      const was = rectOf(s, T, v);
      expect(align === "left" ? was.x : was.x + was.w, align).toBeCloseTo(side, 3);
      // 1 : 1 to 3 : 1, 300 × 100: room in the window to keep its side.
      const next = applyOp(s, { kind: "set", id: T, fields: { text: "wider", nw: 300, nh: 100 }, viewport: v });
      const after = rectOf(next, T, v);
      expect(after.w, align).toBeCloseTo(300, 2);
      expect(align === "left" ? after.x : after.x + after.w, align).toBeCloseTo(side, 2);
      expect(after.y + after.h / 2, align).toBeCloseTo(was.y + was.h / 2, 2);
      expect(next.designSize).toEqual(d);
    }
  });

  it("set keeps the aligned side, and the vertical centre, in a window that draws the text shrunk", () => {
    const d = { w: 1920, h: 1080 };
    // A 900 × 300 text in a short window, drawn 720 × 240, dragged to its
    // left or right edge (place stores x past the design's own edge), then
    // edited to 4 : 1: 1200 × 300, drawn 960 × 240. Its side stays on the
    // window's edge.
    const short = { w: 1920, h: 240 };
    for (const align of ["left", "right"] as const) {
      const s0 = sceneOf([text({ x: 0.3, y: 0.3, w: 900 / 1920, h: 300 / 1080, align, nw: 900, nh: 300 })], d);
      const drawn = rectOf(s0, T, short);
      expect(drawn.w).toBeCloseTo(720, 6);
      expect(drawn.h).toBeCloseTo(240, 6);
      const rect = { ...drawn, x: align === "left" ? 0 : 1920 - 720 };
      const s = applyOp(s0, { kind: "place", id: T, rect, viewport: short });
      if (align === "left") expect(at(s).x).toBeCloseTo(-0.046875, 6);
      const after = rectOf(applyOp(s, { kind: "set", id: T, fields: { text: "long", nw: 1200, nh: 300 }, viewport: short }), T, short);
      expect(after.w, align).toBeCloseTo(960, 2);
      expect(after.h, align).toBeCloseTo(240, 2);
      if (align === "left") expect(after.x, align).toBeCloseTo(0, 2);
      else expect(after.x + after.w, align).toBeCloseTo(1920, 2);
    }
    // In a narrow window the same text is drawn 400 × 133.3 and dragged to
    // the top or bottom edge (place stores y past the design's edge); at
    // 4 : 1 it is drawn 400 × 100 around the same vertical centre.
    const narrow = { w: 400, h: 1080 };
    for (const edge of ["top", "bottom"] as const) {
      const s0 = sceneOf([text({ x: 0.3, y: 0.3, w: 900 / 1920, h: 300 / 1080, align: "center", nw: 900, nh: 300 })], d);
      const drawn = rectOf(s0, T, narrow);
      const rect = { ...drawn, y: edge === "top" ? 0 : 1080 - drawn.h };
      const s = applyOp(s0, { kind: "place", id: T, rect, viewport: narrow });
      if (edge === "top") expect(at(s).y).toBeCloseTo(-0.07716, 5);
      const after = rectOf(applyOp(s, { kind: "set", id: T, fields: { text: "long", nw: 1200, nh: 300 }, viewport: narrow }), T, narrow);
      expect(after.w, edge).toBeCloseTo(400, 2);
      expect(after.h, edge).toBeCloseTo(100, 2);
      expect(after.y + after.h / 2, edge).toBeCloseTo(rect.y + rect.h / 2, 2);
    }
  });

  it("set keeps the edited box inside the window it is seen in, vertically too", () => {
    // A centred 1200 × 300 text in a 400 × 1080 window is drawn 400 × 100;
    // dragged to the top edge, its centre is 50 px down. At 3 : 1 (900 ×
    // 300) it is drawn 400 × 133.3, so keeping that centre would put its top
    // 16.7 px above the window: it is kept inside, its centre at 66.7 px.
    // This window's layout would clamp either way; what is stored shows in
    // another window, e.g. 200 px wide.
    const d = { w: 1920, h: 1080 };
    const v = { w: 400, h: 1080 };
    const s0 = sceneOf([text({ x: 0.3, y: 0.3, w: 1200 / 1920, h: 300 / 1080, align: "center", nw: 1200, nh: 300 })], d);
    const drawn = rectOf(s0, T, v);
    expect(drawn.h).toBeCloseTo(100, 6);
    const s = applyOp(s0, { kind: "place", id: T, rect: { ...drawn, y: 0 }, viewport: v });
    const next = applyOp(s, { kind: "set", id: T, fields: { text: "shorter", nw: 900, nh: 300 }, viewport: v });
    const after = rectOf(next, T, v);
    expect(after.h).toBeCloseTo(400 / 3, 2);
    expect(after.y).toBeCloseTo(0, 2);
    // Stored with the centre at 66.7 px of 1080, not 50: (200/3)/1080 − (300/1080)/2.
    expect(at(next).y).toBeCloseTo(200 / 3 / 1080 - 150 / 1080, 5);
    expect(rectOf(next, T, { w: 200, h: 1080 }).y).toBeCloseTo(100 / 3, 2);
  });

  it("set keeps the aligned side of a text the window draws scaled down to its height", () => {
    // A 5-line text designed 600 × 700 in 1920 × 1080, in a 960 × 540
    // window: drawn 462.9 × 540, at the window's left or right edge (the
    // drawn size, so a move: 600 × 700 kept). Shortened to 400 × 700, it is
    // drawn 308.6 × 540, its side on the same edge.
    const d = { w: 1920, h: 1080 };
    const v = { w: 960, h: 540 };
    for (const align of ["left", "right"] as const) {
      const s0 = sceneOf([text({ x: 0.3, y: 0.2, w: 600 / 1920, h: 700 / 1080, align, nw: 600, nh: 700 })], d);
      const drawn = rectOf(s0, T, v);
      expect(drawn.h).toBeCloseTo(540, 6);
      const s = applyOp(s0, { kind: "place", id: T, rect: { ...drawn, x: align === "left" ? 0 : 960 - drawn.w }, viewport: v });
      const designed = rectOf(s, T, d);
      expect(designed.w).toBeCloseTo(600, 3);
      expect(designed.h).toBeCloseTo(700, 3);
      const after = rectOf(applyOp(s, { kind: "set", id: T, fields: { text: "hi", nw: 400, nh: 700 }, viewport: v }), T, v);
      expect(after.w, align).toBeCloseTo((400 * 540) / 700, 2);
      if (align === "left") expect(after.x, align).toBeCloseTo(0, 2);
      else expect(after.x + after.w, align).toBeCloseTo(960, 2);
    }
  });

  it("set table: the alignment after the patch names the side", () => {
    const s = sceneOf([text({ align: "center" })]);
    expect(box(at(edit(s, { align: "left", nw: 337, nh: 139 })))).toEqual({ x: 0.25, y: 0.4, w: 0.242446, h: 0.1 });
    expect(box(at(edit(s, { align: "right", nw: 337, nh: 139 })))).toEqual({ x: 0.507554, y: 0.4, w: 0.242446, h: 0.1 });
    // A stored alignment this build can't use reads as the centre.
    const odd = sceneOf([text({ align: "justify" })]);
    expect(box(at(edit(odd, { text: "x", nw: 337, nh: 139 })))).toEqual({ x: 0.378777, y: 0.4, w: 0.242446, h: 0.1 });
    expect(at(edit(odd, { text: "x", nw: 337, nh: 139 })).align).toBe("justify");
  });

  it("set table: the same shape changes only the fields", () => {
    const s = sceneOf([circle(), text()]);
    const next = edit(s, { text: "BRB!", nw: 1000, nh: 200 });
    expect(at(next)).toEqual({ ...text(), text: "BRB!", nw: 1000, nh: 200 });
    // Missing nw/nh: the shape counts as changed, from the stored box.
    const bare = text();
    delete bare.nw;
    delete bare.nh;
    const fresh = edit(sceneOf([bare]), { text: "new words", nw: 337, nh: 139 });
    expect(box(at(fresh))).toEqual({ x: 0.378777, y: 0.4, w: 0.242446, h: 0.1 });
  });

  it("set table: fields not in the patch, and unknown keys, are kept as stored", () => {
    // A later build's font and a hand-set colour survive an edit of the
    // words.
    const e = text({ font: "serif", color: "#abcdef", bold: "heavy", future: { k: [1, 2] } });
    const next = edit(sceneOf([e]), { text: "new words", nw: 500, nh: 100 });
    expect(at(next)).toEqual({ ...e, text: "new words" });
    // A patch that sets them replaces them, a colour lower-cased.
    const set2 = edit(sceneOf([e]), { font: "mono", color: "#ABCDEF", bold: false, nw: 500, nh: 100 });
    expect(at(set2)).toMatchObject({ font: "mono", color: "#abcdef", bold: false, future: { k: [1, 2] } });
    // A non-swatch hex is a colour like any other.
    expect(at(edit(sceneOf([text()]), { color: "#123456", nw: 500, nh: 100 })).color).toBe("#123456");
    // The text is stored normalized.
    expect(at(edit(sceneOf([text()]), { text: "  new\r\nwords\t! ", nw: 500, nh: 100 })).text).toBe("new\nwords !");
    // Keys a patch can't carry are ignored: never the id, type, z or box.
    const sneaky = edit(sceneOf([text()]), { text: "x", nw: 500, nh: 100, id: "other", type: "image", z: 99, x: 0.9 });
    expect(at(sneaky)).toEqual({ ...text(), text: "x" });
  });

  it.each([
    ["only nw/nh, as stored", { nw: 500, nh: 100 }],
    ["the stored text", { nw: 500, nh: 100, text: "BRB" }],
    ["the stored text, before normalizing", { nw: 500, nh: 100, text: " BRB\r\n" }],
    ["the stored colour in capitals", { nw: 500, nh: 100, color: "#FFFFFF" }],
    ["every field as stored", { nw: 500, nh: 100, text: "BRB", font: "sans", bold: true, color: "#ffffff", outline: true, align: "center" }],
  ])("set table: %s changes nothing, so the scene itself comes back", (_n, fields) => {
    const s = sceneOf([circle(), text()]);
    expect(setRefusal(s, T, fields as unknown as TextPatch)).toBeNull();
    expect(edit(s, fields)).toBe(s);
  });

  it("setRefusal: gone, invalid or null, as set applies", () => {
    const s = sceneOf([
      circle(),
      text(),
      text({ id: "inert", text: "" }),
      picture("P", { x: 0.1, y: 0.1, w: 0.1, h: 0.1 }),
      light(),
      { id: "future", type: "future", text: "hi", nw: 1, nh: 1 },
    ]);
    const ok = { nw: 337, nh: 139 };
    // No live text with that id; checked before the patch.
    for (const id of ["nope", "inert", "P", L, CIRCLE_ID, "future"]) {
      expect(setRefusal(s, id, ok), id).toBe("gone");
      expect(setRefusal(s, id, { nw: 0, nh: 0 }), id).toBe("gone");
      expect(edit(s, ok, id), id).toBe(s);
    }
    // A field the reader wouldn't take as itself.
    const invalid: Array<Record<string, unknown>> = [
      { nw: 0, nh: 139 },
      { nw: 1.5, nh: 139 },
      { nw: Number.NaN, nh: 139 },
      { nw: "337", nh: 139 },
      { nw: 337, nh: -1 },
      { nw: 337 },
      { ...ok, text: "" },
      { ...ok, text: " \n " },
      { ...ok, text: 5 },
      { ...ok, text: null },
      { ...ok, text: "x".repeat(201) },
      { ...ok, text: "1\n2\n3\n4\n5\n6" },
      { ...ok, font: "serif" },
      { ...ok, font: "Sans" },
      { ...ok, font: "constructor" },
      { ...ok, align: "centre" },
      { ...ok, align: "middle" },
      { ...ok, bold: "true" },
      { ...ok, bold: 1 },
      { ...ok, outline: null },
      { ...ok, color: "#fff" },
      { ...ok, color: "red" },
      { ...ok, color: "#abcdeg" },
      { ...ok, color: 0xffffff },
    ];
    for (const fields of invalid) {
      expect(setRefusal(s, T, fields as unknown as TextPatch), JSON.stringify(fields)).toBe("invalid");
      expect(edit(s, fields), JSON.stringify(fields)).toBe(s);
    }
    expect(setRefusal(s, T, null as unknown as TextPatch)).toBe("invalid");
    expect(applyOp(s, { kind: "set", id: T, fields: null } as unknown as SceneOp)).toBe(s);
    expect(applyOp(s, { kind: "set" } as unknown as SceneOp)).toBe(s);
    // What it accepts, set applies.
    const valid: Array<Record<string, unknown>> = [
      ok,
      { ...ok, text: "x".repeat(200) },
      { ...ok, text: "1\n2\n3\n4\n5" },
      { ...ok, color: "#ABCDEF" },
      { ...ok, font: "display", bold: false, outline: false, align: "right" },
    ];
    for (const fields of valid) {
      expect(setRefusal(s, T, fields as unknown as TextPatch), JSON.stringify(fields)).toBeNull();
      expect(edit(s, fields), JSON.stringify(fields)).not.toBe(s);
    }
  });

  // ---- a stale place ----

  it("stale place on a text: a rect of its old shape after set lands as a move", () => {
    // §7.4's worked row: a 144 × 48 "BRB" (3 : 1) at (168, 192).
    const s = sceneOf([circle(), text({ x: 0.35, y: 0.4, w: 0.3, h: 0.1, nw: 300, nh: 100 })]);
    const old = rectOf(s, T, D480);
    expectRectNear(old, { x: 168, y: 192, w: 144, h: 48 }, 1e-6);
    // Edited to an 8 : 1 text: 384 × 48, its centre kept.
    const edited = edit(s, { text: "BRB, back in five", nw: 800, nh: 100 });
    expectRectNear(rectOf(edited, T, D480), { x: 48, y: 192, w: 384, h: 48 }, 1e-3);
    // A drag of the old rect, 20 px right and 30 down, released after it.
    const dragged = moveRect(old, 20, 30, D480);
    const next = applyOp(edited, { kind: "place", id: T, rect: dragged, viewport: D480 });
    // A move: still 384 × 48 (the designed size kept), centred where the
    // dragged rect was.
    expectRectNear(rectOf(next, T, D480), { x: 68, y: 222, w: 384, h: 48 }, 1e-3);
    expect([at(next).w, at(next).h]).toEqual([at(edited).w, at(edited).h]);
    // Without the rule, the old width would have been kept: 144 × 18, then
    // the 24 px minimum, half the height.
    expectRectNear(normalizeRect(dragged, 8, D480), { x: 164, y: 234, w: 192, h: 24 }, 1e-9);
    // Near an edge it is kept inside.
    const far = applyOp(edited, { kind: "place", id: T, rect: { x: 330, y: 10, w: 144, h: 48 }, viewport: D480 });
    expectRectNear(rectOf(far, T, D480), { x: 96, y: 10, w: 384, h: 48 }, 1e-3);
    // In a smaller window: the size it is laid out at there, so still a move.
    const small = { w: 300, h: 300 };
    const there = applyOp(edited, { kind: "place", id: T, rect: { x: 100, y: 100, w: 90, h: 30 }, viewport: small });
    expect([at(there).w, at(there).h]).toEqual([at(edited).w, at(edited).h]);
    expect(there.designSize).toEqual(D480);
    // A rect of its own shape still resizes; so does one within a
    // thousandth of it.
    const sized = applyOp(edited, { kind: "place", id: T, rect: { x: 0, y: 0, w: 192, h: 24 }, viewport: D480 });
    expectRectNear(rectOf(sized, T, D480), { x: 0, y: 0, w: 192, h: 24 }, 1e-3);
    const near = applyOp(edited, { kind: "place", id: T, rect: { x: 0, y: 0, w: 192, h: 24 * 1.0009 }, viewport: D480 });
    expect(rectOf(near, T, D480).w).toBeCloseTo(192, 3);
    expect(rectOf(near, T, D480).h).toBeCloseTo(24, 3);
  });

  it("the stale-shape rule is a text's only: other kinds given a rect off their shape resize as before", () => {
    // A 2 : 1 picture: the width kept.
    const pic = sceneOf([picture("P", { x: 0.1, y: 0.1, w: 0.2, h: 0.1 }, { nw: 2, nh: 1 })]);
    const p = rectOf(applyOp(pic, { kind: "place", id: "P", rect: { x: 100, y: 100, w: 150, h: 50 }, viewport: D480 }), "P", D480);
    expect([p.w, p.h]).toEqual([150, 75]);
    // The light keeps its stored 5 : 1.
    const lit = sceneOf([light()]);
    const l = rectOf(applyOp(lit, { kind: "place", id: L, rect: { x: 100, y: 100, w: 200, h: 200 }, viewport: D480 }), L, D480);
    expect(l.w).toBeCloseTo(200, 3);
    expect(l.w / l.h).toBeCloseTo(5, 4);
  });

  // ---- add ----

  it("addRefusal order (§7.7): the id, 64 stored, then by kind; text and the light skip the picture caps", () => {
    const n = (more: Record<string, unknown> = {}) => known(text({ id: "N", ...more }));
    const nl = (more: Record<string, unknown> = {}) => known(light({ id: "N", ...more }));
    const full = sceneOf([circle(), ...sixteen()]);
    const overBudget = sceneOf([picture("huge", { x: 0.1, y: 0.1, w: 0.1, h: 0.1 }, { bytes: 300_000_000 })]);
    const lit = sceneOf([light()]);
    const cases: Array<[string, OverlayScene, KnownElement, string | null]> = [
      ["a text beside a circle and 16 pictures", full, n(), null],
      ["a text in a scene over the picture budget", overBudget, n(), null],
      ["a text at the limits: 200 units on 5 lines", sceneOf([]), n({ text: `${"x".repeat(192)}\n1\n2\n3\n4` }), null],
      ["a text with a font and colour this build only reads", sceneOf([]), n({ font: "serif", color: "#abcdef" }), null],
      ["the light beside a circle and 16 pictures", full, nl(), null],
      ["the light in a scene over the picture budget", overBudget, nl(), null],
      ["the light beside an inert one", sceneOf([light({ id: "flat", h: 0 })]), nl(), null],
      ["the light beside a text", sceneOf([text()]), nl(), null],
      ["a text's id taken", sceneOf([text({ id: "N" })]), n(), "exists"],
      ["a taken id before an empty text", sceneOf([text({ id: "N" })]), n({ text: "" }), "exists"],
      ["the light's id taken by an inert element", sceneOf([{ id: "N", type: "future" }]), nl(), "exists"],
      ["a text, 64 stored", sceneOf(sixtyFour()), n(), "elements"],
      ["the light, 64 stored", sceneOf(sixtyFour()), nl(), "elements"],
      ["the light, 64 stored with a live light: elements first", sceneOf([light(), ...sixtyFour().slice(1)]), nl(), "elements"],
      ["a second light", lit, nl(), "light"],
      ["a second light that wouldn't be live either", lit, nl({ w: 0 }), "light"],
      ["a light with a zero width", sceneOf([]), nl({ w: 0 }), "invalid"],
      ["a light with an id over 64", sceneOf([]), nl({ id: "x".repeat(65) }), "invalid"],
      ["an empty text", sceneOf([]), n({ text: "" }), "invalid"],
      ["a text of control characters", sceneOf([]), n({ text: ch(0x07, 0x85) }), "invalid"],
      ["201 units", sceneOf([]), n({ text: "x".repeat(201) }), "invalid"],
      ["6 lines", sceneOf([]), n({ text: "1\n2\n3\n4\n5\n6" }), "invalid"],
      ["no nw", sceneOf([]), n({ nw: undefined }), "invalid"],
      ["nw 0", sceneOf([]), n({ nw: 0 }), "invalid"],
      ["nh 1.5", sceneOf([]), n({ nh: 1.5 }), "invalid"],
      ["nh NaN", sceneOf([]), n({ nh: Number.NaN }), "invalid"],
      ["nw as a string", sceneOf([]), n({ nw: "500" }), "invalid"],
      ["a text with a zero height", sceneOf([]), n({ h: 0 }), "invalid"],
      ["a text with an id over 64", sceneOf([]), n({ id: "x".repeat(65) }), "invalid"],
    ];
    for (const [name, s, e, why] of cases) {
      expect(addRefusal(s, e), name).toBe(why);
      const next = applyOp(s, { kind: "add", element: e });
      if (why === null) {
        expect(next.elements.at(-1), name).toMatchObject({ id: "N", type: e.type });
        expect(isLive(next, "N"), name).toBe(true);
      } else expect(next, name).toBe(s);
    }
  });

  it("add puts a text and the light in front, every field kept", () => {
    const s = sceneOf([circle(), picture("A", { x: 0.1, y: 0.1, w: 0.1, h: 0.1 }, { z: 1 })]);
    const t = text({ id: "N", z: 0, x: 0.1234567, future: { k: 1 }, font: "serif" });
    const withText = applyOp(s, { kind: "add", element: known(t) });
    expect(withText.elements.at(-1)).toEqual({ ...t, x: 0.123457, z: 2 });
    const withLight = applyOp(withText, { kind: "add", element: known(light({ z: -5 })) });
    expect(withLight.elements.at(-1)).toEqual({ ...light(), z: 3 });
    expect(layoutScene(withLight, D480).map((r) => r.el.id)).toEqual([CIRCLE_ID, "A", "N", L]);
  });

  // ---- labels ----

  it("labels: a text's first line, cut at 40 with …; the light is Recording light; repeats numbered", () => {
    const s = sceneOf([
      circle(),
      text({ id: "t1", text: "BRB" }),
      text({ id: "t2", text: " BRB \r\n" }),
      text({ id: "t3", text: "BRB\nback soon" }),
      text({ id: "t4", text: "x".repeat(50) }),
      text({ id: "t5", text: GAME.repeat(30) }),
      text({ id: "empty", text: "" }),
      light({ id: "L1" }),
      light({ id: "L2" }),
      picture("P", { x: 0, y: 0, w: 0.1, h: 0.1 }, { name: "Recording light" }),
    ]);
    expect([...charmLabels(s)]).toEqual([
      [CIRCLE_ID, "Spell circle"],
      ["t1", "BRB"],
      ["t2", "BRB (2)"],
      ["t3", "BRB (3)"],
      ["t4", `${"x".repeat(40)}…`],
      ["t5", `${GAME.repeat(20)}…`],
      ["L1", "Recording light"],
      ["P", "Recording light (2)"],
    ]);
  });

  // ---- defaults ----

  it("defaultLightRect is 5:1 at the bottom left, its height 7 % of the window's short side", () => {
    // §7.8's row: 168 × 33.6 px at (19.2, 427.2) in a 480 × 480 view.
    const r = defaultLightRect(sceneOf([circle()]), D480);
    expectRectNear({ x: r.x * 480, y: r.y * 480, w: r.w * 480, h: r.h * 480 }, { x: 19.2, y: 427.2, w: 168, h: 33.6 }, 1e-3);
    // Clamped to 24–96 px tall.
    expect(defaultLightRect(sceneOf([]), { w: 200, h: 200 }).h * 480).toBeCloseTo(24, 3);
    const big = defaultLightRect(sceneOf([], { w: 3840, h: 2160 }), { w: 3840, h: 2160 });
    // Stored to 6 decimals: within a thousandth of a fraction's px.
    expect(big.h * 2160).toBeCloseTo(96, 2);
    // Never below 24 px tall where the window allows, even past 90 % of
    // its width: in a 125 px wide window 90 % would be 112.5 × 22.5, which
    // its first drag would grow, so it starts at 120 × 24…
    const slim = defaultLightRect(sceneOf([], { w: 125, h: 480 }), { w: 125, h: 480 });
    expect(slim.w * 125).toBeCloseTo(120, 3);
    expect(slim.h * 480).toBeCloseTo(24, 3);
    // …and in a 100 px wide one, 90 × 18 becomes the whole width, 100 × 20.
    const narrow = defaultLightRect(sceneOf([], { w: 100, h: 480 }), { w: 100, h: 480 });
    expect(narrow.w * 100).toBeCloseTo(100, 3);
    expect(narrow.h * 480).toBeCloseTo(20, 3);
    expect(narrow.x).toBe(0);
  });

  it("a new text or light is born the size place stores in a window the size of its design, so its first drag, nudge or key resize keeps it", () => {
    // A 50-character one-line text (2900 × 139, about 21 : 1) in the default
    // 480 × 480 window: 90 % of the width would be 432 × 20.7, under 24 px,
    // and its first place would grow it to 480 × 23 at x 0.
    const s0 = sceneOf([circle()]);
    const r = defaultTextRect(s0, 2900, 139, D480);
    expect(r.h * 480).toBeGreaterThanOrEqual(23);
    const s = applyOp(s0, { kind: "add", element: known(text({ id: "N", ...r, nw: 2900, nh: 139 })) });
    const shown = rectOf(s, "N", D480);
    expect(shown.w).toBeCloseTo(480, 3);
    expect(shown.h).toBeCloseTo((480 * 139) / 2900, 3);
    for (const [name, rect] of [
      ["dragged 10 px down", moveRect(shown, 0, 10, D480)],
      ["nudged 1 px up", moveRect(shown, 0, -1, D480)],
    ] as const) {
      const next = applyOp(s, { kind: "place", id: "N", rect, viewport: D480 });
      const after = rectOf(next, "N", D480);
      // A move: the size kept, where it was dropped.
      expect([at(next, "N").w, at(next, "N").h], name).toEqual([at(s, "N").w, at(s, "N").h]);
      expectRectNear(after, rect, 1e-3);
    }
    // The light in a 100 px wide window: its first nudge is a move too.
    const view = { w: 100, h: 480 };
    const where = defaultLightRect(sceneOf([], view), view);
    const lit = applyOp(sceneOf([], view), { kind: "add", element: known(light({ ...where })) });
    const was = rectOf(lit, L, view);
    const nudged = applyOp(lit, { kind: "place", id: L, rect: moveRect(was, 0, -1, view), viewport: view });
    expect([at(nudged, L).w, at(nudged, L).h]).toEqual([at(lit, L).w, at(lit, L).h]);
    expectRectNear(rectOf(nudged, L, view), { ...was, y: was.y - 1 }, 1e-3);
  });

  it("defaultLightRect sits below Arrange's grip and Lock pill (top 6 px, 28 px tall)", () => {
    for (const view of [D480, { w: 1920, h: 1080 }]) {
      const r = defaultLightRect(sceneOf([], view), view);
      expect(r.y * view.h, `${view.w}×${view.h}`).toBeGreaterThan(6 + 28);
      expect(r.y + r.h).toBeLessThanOrEqual(1 + 1e-6);
      expect(r.x + r.w).toBeLessThanOrEqual(1 + 1e-6);
    }
  });

  it("the light's default box and its resize handles miss the Move window grip and the Lock pill", () => {
    // Both pills are 6 px from the top and 28 px tall, at the left and the
    // right (Stage.tsx's PILL).
    for (const view of [D480, { w: 1920, h: 1080 }]) {
      const where = defaultLightRect(sceneOf([], view), view);
      const s = applyOp(sceneOf([], view), { kind: "add", element: known(light({ ...where })) });
      const r = rectOf(s, L, view);
      const hit = hitRect(r, view);
      for (const b of [r, hit, ...Object.values(handleRects(hit))]) {
        expect(b.y, `${view.w}×${view.h}`).toBeGreaterThan(6 + 28);
      }
      // 4 % in from the left edge and up from the bottom.
      expect(r.x).toBeCloseTo(0.04 * view.w, 2);
      expect(r.y + r.h).toBeCloseTo(0.96 * view.h, 2);
      expect(r.x + r.w).toBeLessThanOrEqual(view.w);
    }
  });

  it("defaultTextRect has its text's shape, 10 % of the short side tall, centred across, its top at 20 %", () => {
    const r = defaultTextRect(sceneOf([circle()]), 337, 139, D480);
    expect(r.h * 480).toBeCloseTo(48, 3);
    expect((r.w * 480) / (r.h * 480)).toBeCloseTo(337 / 139, 4);
    expect(r.x + r.w / 2).toBeCloseTo(0.5, 5);
    expect(r.y).toBeCloseTo(0.2, 6);
  });

  it("defaultTextRect scales a wide text down to 90 % of the window's width, its shape kept", () => {
    // 1000 : 139 at 48 px tall is 345 px: under 432, kept.
    const fits = defaultTextRect(sceneOf([]), 1000, 139, D480);
    expect(fits.w * 480).toBeCloseTo((48 * 1000) / 139, 3);
    // 2000 : 139 is 691 px: scaled down to 432 × 30, still 24 px or more.
    const r = defaultTextRect(sceneOf([]), 2000, 139, D480);
    expect(r.w * 480).toBeCloseTo(432, 3);
    expect((r.w * 480) / (r.h * 480)).toBeCloseTo(2000 / 139, 3);
    expect(r.x).toBeCloseTo(0.05, 5);
  });

  it("defaultTextRect keeps a very wide text 24 px tall where it can, up to the window's whole width", () => {
    // 3000 : 139 at 90 % would be 432 × 20: 24 px tall is 518, wider than
    // the window, so it takes the whole width, 480 × 22.24, at x 0.
    const r = defaultTextRect(sceneOf([]), 3000, 139, D480);
    expect(r.w * 480).toBeCloseTo(480, 3);
    expect(r.h * 480).toBeCloseTo((480 * 139) / 3000, 3);
    expect(r.x).toBe(0);
    // Where 24 px tall fits the window's width, it is 24 px tall, past 90 %:
    // 3336 : 139 (24 : 1) in a 600 px wide window is 576 × 24, centred.
    const view = { w: 600, h: 300 };
    const past = defaultTextRect(sceneOf([], view), 3336, 139, view);
    expect(past.h * 300).toBeCloseTo(24, 3);
    expect(past.w * 600).toBeCloseTo(576, 3);
    expect(past.x + past.w / 2).toBeCloseTo(0.5, 5);
  });

  it("defaultTextRect: each live text already there puts the next 6 % lower, five places round", () => {
    const tops: number[] = [];
    const elements: SceneElement[] = [];
    for (let i = 0; i < 6; i += 1) {
      const r = defaultTextRect(sceneOf(elements), 337, 139, D480);
      tops.push(r.y);
      elements.push(text({ id: `t${i}`, ...r, nw: 337, nh: 139 }));
    }
    expect(tops.map((y) => Math.round(y * 100))).toEqual([20, 26, 32, 38, 44, 20]);
    // A text that isn't live doesn't count.
    const inert = [text({ id: "a", text: "" }), text({ id: "b", w: 0 })];
    expect(defaultTextRect(sceneOf(inert), 337, 139, D480).y).toBeCloseTo(0.2, 6);
  });

  it("defaultTextRect: 32 to 160 px tall, square without a shape, always inside the scene", () => {
    expect(defaultTextRect(sceneOf([]), 337, 139, { w: 200, h: 200 }).h * 480).toBeCloseTo(32, 3);
    const big = defaultTextRect(sceneOf([], { w: 3840, h: 2160 }), 337, 139, { w: 3840, h: 2160 });
    expect(big.h * 2160).toBeCloseTo(160, 2);
    for (const [nw, nh] of [
      [0, 0],
      [Number.NaN, 139],
      [337.5, 139],
    ]) {
      const r = defaultTextRect(sceneOf([]), nw!, nh!, D480);
      expect(r.w, `${nw}×${nh}`).toBeCloseTo(r.h, 6);
    }
    for (const view of [
      { w: 4000, h: 4000 },
      { w: 30, h: 30 },
      { w: 2000, h: 200 },
    ]) {
      const r = defaultTextRect(sceneOf([]), 4000, 139, view);
      expect(r.x).toBeGreaterThanOrEqual(0);
      expect(r.y).toBeGreaterThanOrEqual(0);
      expect(r.x + r.w).toBeLessThanOrEqual(1 + 1e-6);
      expect(r.y + r.h).toBeLessThanOrEqual(1 + 1e-6);
    }
  });

  // ---- through every op ----

  const ops: Array<[string, SceneOp]> = [
    ["add", { kind: "add", element: picture("N", { x: 0.1, y: 0.1, w: 0.2, h: 0.1 }, { src: SRC_B }) as unknown as KnownElement }],
    ["remove", { kind: "remove", id: "A" }],
    ["place", { kind: "place", id: "A", rect: { x: 1, y: 2, w: 100, h: 50 }, viewport: D480 }],
    ["place, growing", { kind: "place", id: "A", rect: { x: 1, y: 2, w: 100, h: 50 }, viewport: { w: 900, h: 700 } }],
    ["order", { kind: "order", id: "A", to: "back" }],
    ["reframe", { kind: "reframe", from: { x: 10, y: 10, w: 480, h: 480 }, to: { x: 0, y: 0, w: 1280, h: 720 }, frame: "monitor" }],
  ];
  const bystanders = (): OverlayScene =>
    sceneOf([
      circle(),
      picture("A", { x: 0.7, y: 0.4, w: 0.3, h: 0.15 }, { z: 1, name: "wave.gif" }),
      text({ z: 2, font: "serif", color: "#ABCDEF", future: { k: [1] } }),
      light({ z: 3, future: "x" }),
    ]);

  it.each(ops)("text and rec survive every op: %s keeps every key but the box and z, and both stay live", (_n, op) => {
    const s = bystanders();
    const next = applyOp(s, op);
    expect(next).not.toBe(s);
    const strip = (e: SceneElement) => json({ ...e, x: 0, y: 0, w: 0, h: 0, z: 0 });
    for (const id of [T, L]) {
      expect(strip(at(next, id)), id).toBe(strip(at(s, id)));
      expect(isLive(next, id), id).toBe(true);
    }
    // Where they draw at the design size: unchanged unless reframed.
    if (op.kind !== "reframe") {
      for (const id of [T, L]) expectRectNear(rectOf(next, id, D480), rectOf(s, id, D480), 2e-3);
    }
  });

  it("set on the text keeps the light and every other element as they were", () => {
    const s = bystanders();
    const next = edit(s, { text: "new words", nw: 337, nh: 139 });
    s.elements.forEach((e, i) => {
      if (e.id !== T) expect(next.elements[i]).toBe(e);
    });
    expect(at(next)).toMatchObject({ font: "serif", color: "#ABCDEF", future: { k: [1] } });
  });

  it("place, order, remove and reframe treat a text and the light as they treat a picture", () => {
    for (const id of [T, L]) {
      const s = sceneOf([circle(), picture("A", { x: 0.1, y: 0.6, w: 0.2, h: 0.1 }, { z: 1 }), text({ z: 2 }), light({ z: 3 })]);
      const was = rectOf(s, id, D480);
      // Moved: the same size, where it was dropped.
      const moved = applyOp(s, { kind: "place", id, rect: moveRect(was, 30, -40, D480), viewport: D480 });
      expectRectNear(rectOf(moved, id, D480), { ...was, x: was.x + 30, y: was.y - 40 }, 1e-3);
      // Resized at its own shape.
      const to = { x: 10, y: 10, w: was.w * 1.5, h: was.h * 1.5 };
      expectRectNear(rectOf(applyOp(s, { kind: "place", id, rect: to, viewport: D480 }), id, D480), to, 1e-3);
      // Restacked.
      expect(layoutScene(applyOp(s, { kind: "order", id, to: "back" }), D480)[0]!.el.id, id).toBe(id);
      // Removed.
      expect(applyOp(s, { kind: "remove", id }).elements.map((e) => e.id), id).not.toContain(id);
      // Fit on: nothing moves on screen.
      const win = { x: 100, y: 50, w: 480, h: 480 };
      const mon = { x: 0, y: 0, w: 1920, h: 1080 };
      const fitted = applyOp(s, { kind: "reframe", from: win, to: mon, frame: "monitor" });
      expectRectNear(rectOf(fitted, id, { w: 1920, h: 1080 }), { ...was, x: was.x + win.x, y: was.y + win.y }, 2e-3);
    }
  });

  it("readOp never returns set, nor an add of a text or the light, from either side", () => {
    const set = { kind: "set", id: T, fields: { nw: 337, nh: 139, text: "hi" } };
    for (const from of ["main", "overlay"] as const) {
      expect(readOp(set, from)).toBeNull();
      expect(readOp({ kind: "add", element: text() }, from)).toBeNull();
      expect(readOp({ kind: "add", element: light() }, from)).toBeNull();
    }
  });

  it("an asset id typed as a text counts as a reference (K10)", () => {
    // assetRefs reads every string in the scene: harmless, the file is
    // kept while the text names it.
    const s = sceneOf([text({ text: "ba7816bf8f01cfea.png" })]);
    expect(assetRefCount(s, "ba7816bf8f01cfea.png")).toBe(1);
    expect(assetRefCount(applyOp(s, { kind: "remove", id: T }), "ba7816bf8f01cfea.png")).toBe(0);
  });
});
