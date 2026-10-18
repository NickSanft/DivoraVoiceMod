// v1.55.0: Charms — what the stream overlay shows, and where.
//
// Main owns the scene (`divora.overlayScene`): it reads it, applies every
// edit and pushes it to the overlay on `overlay:scene`. The overlay draws it
// and sends its edits back on `overlay:edit`. Both sides apply an edit with
// the same pure reducer, so they arrive at the same scene.
//
// Elements store `x, y, w, h` as fractions of `designSize`, which is in CSS
// px. An element this version can't show (an unknown type, a picture it
// can't recognise, a second circle) is kept as stored and written back
// unchanged. Pure: no DOM, no Tauri, and no component imports.

export const SCENE_VERSION = 1;

/** The migrated spell circle's id. */
export const CIRCLE_ID = "circle";

/** The spell circle's own size in CSS px. Equal to SpellCircle.tsx's
 *  `SPELL_CIRCLE_PX` (a test pins them), kept here so this file imports no
 *  component. */
export const CIRCLE_PX = 440;

/** The shortest side an element is given, and its smallest hit box. */
export const MIN_ELEMENT_PX = 24;

/** A resize handle's side. */
export const HANDLE_PX = 12;

/** Creation limits, applied when adding — never when reading. */
export const MAX_PICTURES = 16;
export const MAX_ELEMENTS = 64;

/** The most the scene's distinct pictures may add up to: file bytes,
 *  animated pixels and all pixels. */
export const BUDGET = {
  bytes: 200_000_000,
  animatedPixels: 16_777_216,
  pixels: 67_108_864,
} as const;

/** The design size of a scene with nothing better to go on. */
export const DEFAULT_DESIGN: Readonly<Size> = Object.freeze({ w: 480, h: 480 });

/** A charm picture's id, the same rule as the backend's `is_asset_id`. */
export const ASSET_ID_RE = /^[0-9a-f]{16}\.(png|jpg|gif|webp)$/;

/** Where the overlay loads charm pictures from (the backend's `charm`
 *  scheme, as WebView2 names it). */
export const CHARM_ORIGIN = "http://charm.localhost/";

/** A size in CSS px. */
export interface Size {
  w: number;
  h: number;
}

/** A rectangle in CSS px. */
export interface PxRect {
  x: number;
  y: number;
  w: number;
  h: number;
}

/** A rectangle as fractions of a scene's `designSize`. */
export interface FracRect {
  x: number;
  y: number;
  w: number;
  h: number;
}

export type Corner = "nw" | "ne" | "sw" | "se";

/** What a scene's `designSize` was measured against. */
export type Frame = "window" | "monitor";

export interface CircleElement {
  id: string;
  type: "spellcircle";
  x: number;
  y: number;
  w: number;
  h: number;
  z: number;
  [k: string]: unknown;
}

export interface ImageElement {
  id: string;
  type: "image";
  x: number;
  y: number;
  w: number;
  h: number;
  z: number;
  /** An asset id (`ASSET_ID_RE`), never a path or URL. */
  src: string;
  /** The picture's size as displayed, in its own pixels. */
  nw: number;
  nh: number;
  anim?: boolean;
  bytes?: number;
  name?: string;
  [k: string]: unknown;
}

/** An element this version shows and edits. */
export type KnownElement = CircleElement | ImageElement;

/** An element as stored: shown or not, every key kept. */
export type SceneElement = { id: string; type?: unknown } & Record<string, unknown>;

/** `divora.overlayScene`. Unknown keys, at any level, are kept. */
export interface OverlayScene {
  version: number;
  designSize: Size;
  frame?: unknown;
  elements: SceneElement[];
  [k: string]: unknown;
}

/** One change to the scene. The overlay may send only `remove`, `place`
 *  and `order`; `add` and `reframe` are main's. */
export type SceneOp =
  | { kind: "add"; element: KnownElement }
  | { kind: "remove"; id: string }
  | { kind: "place"; id: string; rect: PxRect; viewport: Size }
  | { kind: "order"; id: string; to: "front" | "back" | "forward" | "backward" }
  | { kind: "reframe"; from: PxRect; to: PxRect; frame: Frame };

/** Why an `add` was refused. */
export type AddRefusal = "exists" | "elements" | "pictures" | "circle" | "budget" | "invalid";

/** The largest `designSize` side, and the largest viewport an edit may
 *  name. */
export const MAX_DESIGN_PX = 16384;

/** The longest id an element or an edit may carry. */
const MAX_ID = 64;

/** The longest picture name shown. */
const MAX_NAME = 120;

type Order = "front" | "back" | "forward" | "backward";
const ORDERS: readonly string[] = ["front", "back", "forward", "backward"];

// ---- small helpers ----

function finite(n: unknown): n is number {
  return typeof n === "number" && Number.isFinite(n);
}

function positive(n: unknown): n is number {
  return finite(n) && n > 0;
}

function isObject(v: unknown): v is Record<string, unknown> {
  return typeof v === "object" && v !== null && !Array.isArray(v);
}

function clamp(n: number, lo: number, hi: number): number {
  return Math.min(Math.max(n, lo), Math.max(lo, hi));
}

/** Every value the reducer writes is rounded to 6 decimals, so main and the
 *  overlay write byte-identical scenes. */
export function round6(n: number): number {
  return Math.round(n * 1e6) / 1e6;
}

function validId(id: unknown): id is string {
  return typeof id === "string" && id.length >= 1 && id.length <= MAX_ID;
}

/** Finite `x, y` and positive `w, h`: something the layout can draw. */
function usable(e: Record<string, unknown>): boolean {
  return finite(e.x) && finite(e.y) && positive(e.w) && positive(e.h);
}

function validSize(v: unknown): v is Size {
  return (
    isObject(v) && finite(v.w) && finite(v.h) && v.w >= 1 && v.h >= 1 && v.w <= MAX_DESIGN_PX && v.h <= MAX_DESIGN_PX
  );
}

function validRect(r: unknown): r is PxRect {
  return isObject(r) && finite(r.x) && finite(r.y) && positive(r.w) && positive(r.h);
}

/** A picture's own pixel count: an integer of at least 1. */
function pixelCount(n: unknown): n is number {
  return typeof n === "number" && Number.isInteger(n) && n >= 1;
}

/** An element's place in the stacking order: its `z`, or its index. */
function zOf(e: Record<string, unknown>, index: number): number {
  return finite(e.z) ? e.z : index;
}

/** `e` with this rect; `e` itself when nothing differs. */
function withRect(e: SceneElement, r: FracRect): SceneElement {
  if (e.x === r.x && e.y === r.y && e.w === r.w && e.h === r.h) return e;
  return { ...e, x: r.x, y: r.y, w: r.w, h: r.h };
}

// ---- reading ----

function defaultElements(design: Size): SceneElement[] {
  const s = Math.min(CIRCLE_PX, design.w, design.h);
  return [
    {
      id: CIRCLE_ID,
      type: "spellcircle",
      x: round6((design.w - s) / 2 / design.w),
      y: round6((design.h - s) / 2 / design.h),
      w: round6(s / design.w),
      h: round6(s / design.h),
      z: 0,
    },
  ];
}

/** The spell circle, centred and at most 440 px, in a scene of this size. */
export function defaultScene(design: Size = DEFAULT_DESIGN): OverlayScene {
  const d = validSize(design) ? { w: design.w, h: design.h } : { ...DEFAULT_DESIGN };
  return { version: SCENE_VERSION, designSize: d, frame: "window", elements: defaultElements(d) };
}

/** The scene a v1.51.3 user had: the circle where the overlay drew it, in
 *  the window `divora.overlay` remembers (physical px, so divided by the
 *  scale it was saved at). Used only while `divora.overlayScene` is absent
 *  or unreadable. */
export function migrateScene(overlayPrefs: unknown): OverlayScene {
  if (!isObject(overlayPrefs)) return defaultScene();
  const { w, h, scale } = overlayPrefs;
  if (!positive(w) || !positive(h)) return defaultScene();
  const k = positive(scale) ? scale : 1;
  return defaultScene({ w: Math.round(w / k), h: Math.round(h / k) });
}

/** A stored scene, or `fallback` when there is nothing to read. Reading
 *  changes no element: it drops only entries nothing can address (no
 *  string id) and later duplicate ids, and settles `version` and
 *  `designSize`. Every other key, at any level, is kept. */
export function readScene(raw: unknown, fallback: OverlayScene): OverlayScene {
  if (!isObject(raw)) return fallback;
  const v = raw.version;
  const version = typeof v === "number" && Number.isInteger(v) && v >= 1 ? v : SCENE_VERSION;
  const ds = raw.designSize;
  const designSize: Size = validSize(ds) ? { ...ds, w: ds.w, h: ds.h } : { ...DEFAULT_DESIGN };
  let elements: SceneElement[];
  if (Array.isArray(raw.elements)) {
    const seen = new Set<string>();
    elements = [];
    for (const e of raw.elements as unknown[]) {
      if (!isObject(e) || typeof e.id !== "string" || e.id.length === 0 || seen.has(e.id)) continue;
      seen.add(e.id);
      elements.push(e as SceneElement);
    }
  } else {
    elements = defaultElements(designSize);
  }
  return { ...raw, version, designSize, elements };
}

/** The scene to write: `version` is never lowered. */
export function sceneForSave(s: OverlayScene): OverlayScene {
  return { ...s, version: Math.max(finite(s.version) ? s.version : SCENE_VERSION, SCENE_VERSION) };
}

/** What the scene's `designSize` was measured against. */
export function frameOf(s: OverlayScene): Frame {
  return s.frame === "monitor" ? "monitor" : "window";
}

// ---- live and inert ----

/** A spell circle this version could draw. It is live only as the scene's
 *  first such circle (`liveElements`). */
export function asCircle(e: SceneElement): CircleElement | null {
  if (!isObject(e) || e.type !== "spellcircle" || !validId(e.id) || !usable(e)) return null;
  return e as unknown as CircleElement;
}

/** A picture this version can draw: usable geometry and an asset id. */
export function asImage(e: SceneElement): ImageElement | null {
  if (!isObject(e) || e.type !== "image" || !validId(e.id) || !usable(e)) return null;
  if (typeof e.src !== "string" || !ASSET_ID_RE.test(e.src)) return null;
  return e as unknown as ImageElement;
}

function liveEntries(s: OverlayScene): Array<{ el: KnownElement; index: number }> {
  const out: Array<{ el: KnownElement; index: number }> = [];
  let circle = false;
  s.elements.forEach((e, index) => {
    const c = asCircle(e);
    if (c) {
      // One live circle: SpellCircle's gradient ids are document-global.
      if (!circle) out.push({ el: c, index });
      circle = true;
      return;
    }
    const img = asImage(e);
    if (img) out.push({ el: img, index });
  });
  return out;
}

/** The elements this version shows and edits, in array order. */
export function liveElements(s: OverlayScene): KnownElement[] {
  return liveEntries(s).map((e) => e.el);
}

export function isLive(s: OverlayScene, id: string): boolean {
  return liveEntries(s).some((e) => e.el.id === id);
}

/** The live pictures (the 16-picture cap counts these). */
export function pictureCount(s: OverlayScene): number {
  return liveEntries(s).filter((e) => e.el.type === "image").length;
}

// ---- layout ----

/** An element's size as designed, in CSS px. The circle is the largest
 *  square in its stored rectangle. */
export function designedSize(e: KnownElement, design: Size): Size {
  const w = e.w * design.w;
  const h = e.h * design.h;
  if (e.type === "spellcircle") {
    const s = Math.min(w, h);
    return { w: s, h: s };
  }
  return { w, h };
}

/** Width over height, as the reducer keeps it: the picture's own pixels
 *  (its stored shape when those are missing), and 1 for the circle. */
export function aspectOf(e: KnownElement, design: Size): number {
  if (e.type === "spellcircle") return 1;
  if (pixelCount(e.nw) && pixelCount(e.nh)) return e.nw / e.nh;
  const d = designedSize(e, design);
  return d.w / d.h;
}

/** Lay out a stored rectangle in viewport `v`: keep its designed size,
 *  shrinking uniformly only when it doesn't fit, and keep its centre at the
 *  same fraction of `v`, clamped inside. `square` first takes the largest
 *  square centred in it (the circle). */
function layoutBox(e: FracRect, design: Size, v: Size, square: boolean): PxRect {
  let w = e.w * design.w;
  let h = e.h * design.h;
  const u = e.x + e.w / 2;
  const c = e.y + e.h / 2;
  if (square) w = h = Math.min(w, h);
  const k = Math.min(1, v.w / w, v.h / h);
  w *= k;
  h *= k;
  return { x: clamp(u * v.w - w / 2, 0, v.w - w), y: clamp(c * v.h - h / 2, 0, v.h - h), w, h };
}

function degenerate(v: Size): boolean {
  return !positive(v.w) || !positive(v.h);
}

/** Where a live element draws in `viewport` (CSS px); null in an empty
 *  viewport. */
export function layoutElement(e: KnownElement, design: Size, viewport: Size): PxRect | null {
  if (degenerate(viewport) || !usable(e)) return null;
  return layoutBox(e, design, viewport, e.type === "spellcircle");
}

/** The live elements laid out in `viewport`, back to front: by `z`, then by
 *  array index. Empty in an empty viewport. */
export function layoutScene(s: OverlayScene, viewport: Size): Array<{ el: KnownElement; rect: PxRect }> {
  if (degenerate(viewport)) return [];
  return liveEntries(s)
    .sort((a, b) => zOf(a.el, a.index) - zOf(b.el, b.index) || a.index - b.index)
    .map(({ el }) => ({ el, rect: layoutBox(el, s.designSize, viewport, el.type === "spellcircle") }));
}

// ---- the reducer ----

/** Apply one edit. Pure and deterministic; returns `s` itself when nothing
 *  changes, so main commits nothing. Never throws. */
export function applyOp(s: OverlayScene, op: SceneOp): OverlayScene {
  if (!isObject(op)) return s;
  switch (op.kind) {
    case "add":
      return add(s, op.element);
    case "remove":
      return remove(s, op.id);
    case "place":
      return place(s, op.id, op.rect, op.viewport);
    case "order":
      return order(s, op.id, op.to);
    case "reframe":
      return reframe(s, op.from, op.to, op.frame);
    default:
      return s;
  }
}

/** Give the elements at `indices` the `z` of their place in that list,
 *  copying only those whose `z` changes; null when none does. */
function renumber(elements: SceneElement[], indices: number[]): SceneElement[] | null {
  const out = elements.slice();
  let changed = false;
  indices.forEach((i, z) => {
    const e = out[i]!;
    if (e.z !== z) {
      out[i] = { ...e, z };
      changed = true;
    }
  });
  return changed ? out : null;
}

/** The indices of every element with a finite `z` (and `extra`), back to
 *  front by `(z, index)`. */
function stacking(elements: SceneElement[], extra = -1): number[] {
  const idx: number[] = [];
  elements.forEach((e, i) => {
    if (i === extra || finite(e.z)) idx.push(i);
  });
  return idx.sort((a, b) => zOf(elements[a]!, a) - zOf(elements[b]!, b) || a - b);
}

function add(s: OverlayScene, element: KnownElement): OverlayScene {
  if (!isObject(element) || addRefusal(s, element) !== null) return s;
  const zs = s.elements.map((e) => e.z).filter(finite);
  // The circle goes to the back, a picture to the front.
  const z = element.type === "spellcircle" ? Math.min(0, ...zs) - 1 : Math.max(-1, ...zs) + 1;
  const added: SceneElement = {
    ...element,
    x: round6(element.x),
    y: round6(element.y),
    w: round6(element.w),
    h: round6(element.h),
    z,
  };
  const elements = [...s.elements, added];
  return { ...s, elements: renumber(elements, stacking(elements)) ?? elements };
}

function remove(s: OverlayScene, id: string): OverlayScene {
  const i = s.elements.findIndex((e) => e.id === id);
  if (i < 0) return s;
  return { ...s, elements: s.elements.filter((_, j) => j !== i) };
}

/** Keep an element's centre fraction and designed px size while
 *  `designSize` grows from `from` to `to`, so its layout at every viewport
 *  is unchanged. Left alone without usable geometry (a negative `w` would
 *  flip sign). */
function reexpress(e: SceneElement, from: Size, to: Size): SceneElement {
  if (!usable(e)) return e;
  const r = e as unknown as FracRect;
  const w = r.w * from.w;
  const h = r.h * from.h;
  return withRect(e, {
    x: round6(r.x + r.w / 2 - w / (2 * to.w)),
    y: round6(r.y + r.h / 2 - h / (2 * to.h)),
    w: round6(w / to.w),
    h: round6(h / to.h),
  });
}

function place(s: OverlayScene, id: string, rect: PxRect, viewport: Size): OverlayScene {
  if (!validSize(viewport) || !validRect(rect)) return s;
  const entry = liveEntries(s).find((e) => e.el.id === id);
  if (!entry) return s;
  const d = s.designSize;
  const v = viewport;
  // designSize only grows, so shrinking the window and growing it back
  // changes nothing.
  const grown = { w: round6(Math.max(d.w, v.w)), h: round6(Math.max(d.h, v.h)) };
  const grew = grown.w !== d.w || grown.h !== d.h;
  const target = entry.el;
  const n = normalizeRect(rect, aspectOf(target, d), v);
  const shown = layoutBox(target, d, v, target.type === "spellcircle");
  // A drag or a nudge keeps the designed size: a 1 px nudge of a shrunk
  // element would otherwise bake in its shrunk size. A resize takes the
  // new one.
  const moved = Math.abs(n.w - shown.w) <= 0.5 && Math.abs(n.h - shown.h) <= 0.5;
  const size = moved ? designedSize(target, d) : { w: n.w, h: n.h };
  const u = (n.x + n.w / 2) / v.w;
  const c = (n.y + n.h / 2) / v.h;
  const before = s.elements[entry.index]!;
  const after = withRect(before, {
    x: round6(u - size.w / (2 * grown.w)),
    y: round6(c - size.h / (2 * grown.h)),
    w: round6(size.w / grown.w),
    h: round6(size.h / grown.h),
  });
  if (!grew && after === before) return s;
  // Unchanged elements stay the same objects: no re-rounding, no drift.
  const elements = s.elements.map((e, i) => (i === entry.index ? after : grew ? reexpress(e, d, grown) : e));
  return { ...s, designSize: grew ? { ...d, w: grown.w, h: grown.h } : d, elements };
}

function order(s: OverlayScene, id: string, to: Order): OverlayScene {
  if (!ORDERS.includes(to)) return s;
  const live = liveEntries(s);
  const entry = live.find((e) => e.el.id === id);
  if (!entry) return s;
  const liveAt = new Set(live.map((e) => e.index));
  const stack = stacking(s.elements, entry.index);
  const from = stack.indexOf(entry.index);
  stack.splice(from, 1);
  if (to === "front") stack.push(entry.index);
  else if (to === "back") stack.unshift(entry.index);
  else if (to === "forward") {
    // Past the nearest live element above. Stepping over an inert one
    // would change nothing on screen.
    let at = from;
    while (at < stack.length && !liveAt.has(stack[at]!)) at += 1;
    if (at === stack.length) return s;
    stack.splice(at + 1, 0, entry.index);
  } else {
    let at = from - 1;
    while (at >= 0 && !liveAt.has(stack[at]!)) at -= 1;
    if (at < 0) return s;
    stack.splice(at, 0, entry.index);
  }
  const elements = renumber(s.elements, stack);
  return elements ? { ...s, elements } : s;
}

function reframe(s: OverlayScene, from: PxRect, to: PxRect, frame: Frame): OverlayScene {
  if (!validRect(from) || !validRect(to) || (frame !== "window" && frame !== "monitor")) return s;
  const size = { w: round6(to.w), h: round6(to.h) };
  if (!validSize(size)) return s;
  const d = s.designSize;
  const circle = liveEntries(s).find((e) => e.el.type === "spellcircle")?.index ?? -1;
  const elements = s.elements.map((e, i) => {
    if (!usable(e)) return e;
    // Where it draws in `from`, in `to`'s coordinates…
    const r = layoutBox(e as unknown as FracRect, d, from, i === circle);
    const rx = r.x + from.x - to.x;
    const ry = r.y + from.y - to.y;
    // …shrunk if it is larger than `to`, then pulled inside it.
    const k = Math.min(1, to.w / r.w, to.h / r.h);
    const w = r.w * k;
    const h = r.h * k;
    const x = clamp(rx + r.w / 2 - w / 2, 0, to.w - w);
    const y = clamp(ry + r.h / 2 - h / 2, 0, to.h - h);
    return withRect(e, {
      x: round6(x / size.w),
      y: round6(y / size.h),
      w: round6(w / size.w),
      h: round6(h / size.h),
    });
  });
  const same =
    elements.every((e, i) => e === s.elements[i]) && size.w === d.w && size.h === d.h && s.frame === frame;
  if (same) return s;
  return { ...s, designSize: { ...d, w: size.w, h: size.h }, frame, elements };
}

/** Check an edit off the wire. The overlay may send only `remove`, `place`
 *  and `order`. Returns a fresh op holding only the keys it knows, or
 *  null. */
export function readOp(raw: unknown, from: "overlay" | "main"): SceneOp | null {
  if (!isObject(raw)) return null;
  const rect = (r: unknown): PxRect | null => (validRect(r) ? { x: r.x, y: r.y, w: r.w, h: r.h } : null);
  switch (raw.kind) {
    case "remove":
      return validId(raw.id) ? { kind: "remove", id: raw.id } : null;
    case "place": {
      const r = rect(raw.rect);
      const v = raw.viewport;
      if (!validId(raw.id) || !r || !isObject(v) || !positive(v.w) || !positive(v.h)) return null;
      return { kind: "place", id: raw.id, rect: r, viewport: { w: v.w, h: v.h } };
    }
    case "order":
      if (!validId(raw.id) || typeof raw.to !== "string" || !ORDERS.includes(raw.to)) return null;
      return { kind: "order", id: raw.id, to: raw.to as Order };
    case "add": {
      const e = raw.element;
      if (from !== "main" || !isObject(e) || !validId(e.id)) return null;
      if (e.type !== "spellcircle" && e.type !== "image") return null;
      return { kind: "add", element: e as KnownElement };
    }
    case "reframe": {
      const f = rect(raw.from);
      const t = rect(raw.to);
      if (from !== "main" || !f || !t || (raw.frame !== "window" && raw.frame !== "monitor")) return null;
      return { kind: "reframe", from: f, to: t, frame: raw.frame };
    }
    default:
      return null;
  }
}

/** Why `add` would refuse `e`, or null. In order: the id is taken; 64
 *  elements are stored; a live circle exists; 16 live pictures; it wouldn't
 *  be live, or a picture lacks its pixel size; the scene budget. */
export function addRefusal(s: OverlayScene, e: KnownElement): AddRefusal | null {
  if (s.elements.some((x) => x.id === e.id)) return "exists";
  if (s.elements.length >= MAX_ELEMENTS) return "elements";
  const live = liveElements(s);
  if (e.type === "spellcircle" && live.some((x) => x.type === "spellcircle")) return "circle";
  if (e.type === "image" && live.filter((x) => x.type === "image").length >= MAX_PICTURES) return "pictures";
  if (e.type === "spellcircle") return asCircle(e) ? null : "invalid";
  const img = asImage(e);
  if (!img || !pixelCount(img.nw) || !pixelCount(img.nh)) return "invalid";
  // Summed over distinct pictures, so a shared file counts once.
  const bySrc = new Map<string, ImageElement>();
  for (const x of [...live, img]) if (x.type === "image" && !bySrc.has(x.src)) bySrc.set(x.src, x);
  let bytes = 0;
  let animated = 0;
  let pixels = 0;
  for (const p of bySrc.values()) {
    const area = pixelCount(p.nw) && pixelCount(p.nh) ? p.nw * p.nh : 0;
    bytes += positive(p.bytes) ? p.bytes : 0;
    pixels += area;
    if (typeof p.anim === "boolean" ? p.anim : p.src.endsWith(".gif")) animated += area;
  }
  if (bytes > BUDGET.bytes || animated > BUDGET.animatedPixels || pixels > BUDGET.pixels) return "budget";
  return null;
}

// ---- rect helpers (CSS px) ----

/** Move by `(dx, dy)`, kept inside the viewport. */
export function moveRect(r: PxRect, dx: number, dy: number, v: Size): PxRect {
  return { x: clamp(r.x + dx, 0, v.w - r.w), y: clamp(r.y + dy, 0, v.h - r.h), w: r.w, h: r.h };
}

/** Resize from a corner, keeping the shape and holding the opposite corner
 *  still. The pointer's move is projected onto the corner's diagonal, so an
 *  inward drag along one axis shrinks it and the size never jumps. Never
 *  below `min` on the short side (unless it already is and can't reach it),
 *  never past the viewport. */
export function resizeRect(r: PxRect, c: Corner, dx: number, dy: number, v: Size, min = MIN_ELEMENT_PX): PxRect {
  if (!positive(r.w) || !positive(r.h)) return r;
  const sx = c === "ne" || c === "se" ? 1 : -1;
  const sy = c === "sw" || c === "se" ? 1 : -1;
  const ax = sx > 0 ? r.x : r.x + r.w;
  const ay = sy > 0 ? r.y : r.y + r.h;
  const pull = 1 + (sx * dx * r.w + sy * dy * r.h) / (r.w * r.w + r.h * r.h);
  const sMax = Math.min((sx > 0 ? v.w - ax : ax) / r.w, (sy > 0 ? v.h - ay : ay) / r.h);
  if (!(sMax > 0)) return r;
  const sFloor = min / Math.min(r.w, r.h);
  // Already below the minimum and unable to reach it: no further shrink.
  const sMin = sFloor > sMax ? Math.min(1, sMax) : sFloor;
  const s = clamp(finite(pull) ? pull : 1, sMin, sMax);
  const w = r.w * s;
  const h = r.h * s;
  return { x: sx > 0 ? ax : ax - w, y: sy > 0 ? ay : ay - h, w, h };
}

/** This shape, fitted inside the viewport, with its short side at least
 *  `min` where the viewport allows. */
function fitSize(w: number, h: number, v: Size, min: number): Size {
  const k = Math.min(1, v.w / w, v.h / h);
  w *= k;
  h *= k;
  const short = Math.min(w, h);
  if (short < min) {
    const up = Math.min(min / short, v.w / w, v.h / h);
    w *= up;
    h *= up;
  }
  return { w, h };
}

/** The keyboard's resize, about the centre: a small step is 2 px on the
 *  long side, a big one 10 %. Then `normalizeRect`. */
export function scaleRect(r: PxRect, dir: 1 | -1, big: boolean, aspect: number, v: Size): PxRect {
  const long = Math.max(r.w, r.h);
  if (!positive(long)) return normalizeRect(r, aspect, v);
  const next = big ? long * (1 + 0.1 * dir) : long + 2 * dir;
  const k = Math.max(next, 1) / long;
  const w = r.w * k;
  const h = r.h * k;
  return normalizeRect({ x: r.x + (r.w - w) / 2, y: r.y + (r.h - h) / 2, w, h }, aspect, v);
}

/** The rect `place` stores: the same centre, height = width / aspect,
 *  fitted inside the viewport, its short side at least `min` (still
 *  inside), then clamped inside. */
export function normalizeRect(r: PxRect, aspect: number, v: Size, min = MIN_ELEMENT_PX): PxRect {
  const w0 = positive(r.w) ? r.w : min;
  const a = positive(aspect) ? aspect : positive(r.h) ? w0 / r.h : 1;
  const cx = r.x + r.w / 2;
  const cy = r.y + r.h / 2;
  const { w, h } = fitSize(w0, w0 / a, v, min);
  return { x: clamp(cx - w / 2, 0, v.w - w), y: clamp(cy - h / 2, 0, v.h - h), w, h };
}

/** Where a new picture goes, as fractions of the scene's `designSize`: its
 *  long side 30 % of the window's short side (never upscaled past its own
 *  pixels), its short side at least 24 px, in one of five cascading spots
 *  towards the bottom right. `view` is the window's CSS size. */
export function defaultImageRect(s: OverlayScene, nw: number, nh: number, view?: Size): FracRect {
  const d = s.designSize;
  const v = view && positive(view.w) && positive(view.h) ? view : d;
  const ok = finite(nw) && finite(nh) && nw >= 1 && nh >= 1;
  const pw = ok ? nw : 1;
  const ph = ok ? nh : 1;
  const long = Math.max(MIN_ELEMENT_PX, Math.min(0.3 * Math.min(v.w, v.h), Math.max(pw, ph)));
  const fitted = fitSize(pw >= ph ? long : (long * pw) / ph, pw >= ph ? (long * ph) / pw : long, v, MIN_ELEMENT_PX);
  const k = Math.min(1, d.w / fitted.w, d.h / fitted.h);
  const w = fitted.w * k;
  const h = fitted.h * k;
  const at = 0.75 - 0.04 * (pictureCount(s) % 5);
  const cx = clamp(at * d.w, w / 2, d.w - w / 2);
  const cy = clamp(at * d.h, h / 2, d.h - h / 2);
  return {
    x: round6((cx - w / 2) / d.w),
    y: round6((cy - h / 2) / d.h),
    w: round6(w / d.w),
    h: round6(h / d.h),
  };
}

// ---- hit testing ----

/** An element's hit box: at least `min` a side (or the viewport's side),
 *  centred on it, inside the viewport. The picture still draws at `r`. */
export function hitRect(r: PxRect, v: Size, min = MIN_ELEMENT_PX): PxRect {
  const w = Math.max(r.w, Math.min(min, v.w));
  const h = Math.max(r.h, Math.min(min, v.h));
  return {
    x: clamp(r.x + r.w / 2 - w / 2, 0, v.w - w),
    y: clamp(r.y + r.h / 2 - h / 2, 0, v.h - h),
    w,
    h,
  };
}

/** The four resize handles: squares centred on the hit box's corners, half
 *  of each outside it. */
export function handleRects(hit: PxRect): Record<Corner, PxRect> {
  const half = HANDLE_PX / 2;
  const at = (x: number, y: number): PxRect => ({ x: x - half, y: y - half, w: HANDLE_PX, h: HANDLE_PX });
  return {
    nw: at(hit.x, hit.y),
    ne: at(hit.x + hit.w, hit.y),
    sw: at(hit.x, hit.y + hit.h),
    se: at(hit.x + hit.w, hit.y + hit.h),
  };
}

function contains(r: PxRect, p: { x: number; y: number }): boolean {
  return p.x >= r.x && p.x < r.x + r.w && p.y >= r.y && p.y < r.y + r.h;
}

const CORNERS: readonly Corner[] = ["nw", "ne", "sw", "se"];

/** What a press at `p` grabs: a corner inside its handle, a move anywhere
 *  else in the hit box, or nothing. */
export function hitPart(hit: PxRect, p: { x: number; y: number }): Corner | "move" | null {
  const handles = handleRects(hit);
  for (const c of CORNERS) if (contains(handles[c], p)) return c;
  return contains(hit, p) ? "move" : null;
}

// ---- pictures ----

/** Every string anywhere in the scene that is an asset id, counted: live
 *  and inert elements, nested values and unknown top-level keys. A file is
 *  kept while anything names it. */
export function assetRefs(s: OverlayScene): Map<string, number> {
  const refs = new Map<string, number>();
  // The objects on the current path: a cycle stops, a shared object counts
  // each time it appears.
  const path = new Set<object>();
  const walk = (v: unknown): void => {
    if (typeof v === "string") {
      if (ASSET_ID_RE.test(v)) refs.set(v, (refs.get(v) ?? 0) + 1);
      return;
    }
    if (typeof v !== "object" || v === null || path.has(v)) return;
    path.add(v);
    for (const x of Array.isArray(v) ? v : Object.values(v)) walk(x);
    path.delete(v);
  };
  walk(s);
  return refs;
}

/** How many strings in the scene name this asset: the one definition of
 *  "unreferenced" (0). */
export function assetRefCount(s: OverlayScene, id: string): number {
  return assetRefs(s).get(id) ?? 0;
}

/** Each live element's name in Settings and the overlay's labels: "Spell
 *  circle", or the picture's name ("Picture" without one). Repeats are
 *  numbered " (2)", " (3)"… in array order, so adding a copy never renames
 *  the ones before it, and a suffix another picture's own name already
 *  uses is skipped. */
export function charmLabels(s: OverlayScene): Map<string, string> {
  const live = liveElements(s);
  const base = (e: KnownElement): string => {
    if (e.type === "spellcircle") return "Spell circle";
    return typeof e.name === "string" && e.name.trim().length > 0 ? e.name.slice(0, MAX_NAME) : "Picture";
  };
  const names = new Set(live.map(base));
  const used = new Set<string>();
  const labels = new Map<string, string>();
  for (const e of live) {
    const b = base(e);
    let label = b;
    if (used.has(label)) {
      let n = 2;
      while (used.has(`${b} (${n})`) || names.has(`${b} (${n})`)) n += 1;
      label = `${b} (${n})`;
    }
    used.add(label);
    labels.set(e.id, label);
  }
  return labels;
}

/** Where the overlay loads a picture from; "" for anything that isn't an
 *  asset id, so no other URL is ever built. */
export function charmUrl(id: string): string {
  return ASSET_ID_RE.test(id) ? CHARM_ORIGIN + id : "";
}
