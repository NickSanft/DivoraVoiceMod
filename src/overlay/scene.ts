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
//
// v1.62.0 (Text charms): two more kinds, a text charm (`text`) and the
// recording light (`rec`), which main adds itself, and a main-only `set`
// that edits a text charm's fields. Older builds keep both kinds inert.

import {
  firstLine,
  isTextAlign,
  isTextColour,
  isTextFont,
  normalizeCharmText,
  readCharmText,
  readTextFields,
  wellFormedText,
  type TextPatch,
} from "./text";
import { isTimerId } from "./timerState";

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

/** v1.59.0: an Hourglass timer's face. `timer` names the timer, which may
 *  not exist (yet, or any more): the element is still kept and placed.
 *  Kept inert since v1.55.0 under this type. */
export interface TimerElement {
  id: string;
  type: "timer";
  x: number;
  y: number;
  w: number;
  h: number;
  z: number;
  /** A timer id (`isTimerId`), never a name. */
  timer: string;
  [k: string]: unknown;
}

/** v1.62.0: a text charm, typed in Settings. Live when `text`, normalized,
 *  isn't empty; the creation limits apply only when adding or setting.
 *  The other fields are kept as stored and read through text.ts's
 *  `readTextFields`, which gives a value this build can't use its
 *  default. */
export interface TextElement {
  id: string;
  type: "text";
  x: number;
  y: number;
  w: number;
  h: number;
  z: number;
  text: string;
  /** A `TextFont`, as main writes it. */
  font?: unknown;
  bold?: unknown;
  /** `#rrggbb`, as main writes it. */
  color?: unknown;
  outline?: unknown;
  /** A `TextAlign`, as main writes it. */
  align?: unknown;
  /** The text's natural box in whole CSS px at text.ts's REF_PX: the
   *  element's shape, as a picture's own pixels are. */
  nw?: unknown;
  nh?: unknown;
  [k: string]: unknown;
}

/** v1.62.0: the recording light. Only the scene's first one is live: it
 *  reports one fact. */
export interface RecElement {
  id: string;
  type: "rec";
  x: number;
  y: number;
  w: number;
  h: number;
  z: number;
  [k: string]: unknown;
}

/** An element this version shows and edits. */
export type KnownElement = CircleElement | ImageElement | TimerElement | TextElement | RecElement;

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
 *  and `order`; `add` and `reframe` are main's. v1.62.0: `set` is main's
 *  own edit of a text charm, carrying `nw`/`nh` and only the fields that
 *  changed, and the overlay window's CSS size it is seen in (`viewport`,
 *  the design size when absent); it is never on the wire, and `readOp`
 *  never returns it. */
export type SceneOp =
  | { kind: "add"; element: KnownElement }
  | { kind: "remove"; id: string }
  | { kind: "place"; id: string; rect: PxRect; viewport: Size }
  | { kind: "order"; id: string; to: "front" | "back" | "forward" | "backward" }
  | { kind: "reframe"; from: PxRect; to: PxRect; frame: Frame }
  | { kind: "set"; id: string; fields: TextPatch; viewport?: Size };

/** Why an `add` was refused. v1.62.0: `light`, the scene already has a
 *  live recording light. */
export type AddRefusal = "exists" | "elements" | "pictures" | "circle" | "budget" | "invalid" | "light";

/** v1.62.0: why a `set` would change nothing. `gone`: no live text has that
 *  id. `invalid`: the patch holds a field the reader wouldn't accept. */
export type SetRefusal = "gone" | "invalid";

/** The largest `designSize` side, and the largest viewport an edit may
 *  name. */
export const MAX_DESIGN_PX = 16384;

/** The longest id an element or an edit may carry. */
const MAX_ID = 64;

/** The longest picture name shown. */
const MAX_NAME = 120;

/** v1.62.0: the longest first line a text charm's label shows. */
const TEXT_LABEL_CHARS = 40;

/** v1.62.0: how far a place's shape may be off a text's own (`nw`/`nh`)
 *  before it counts as made against an older shape. */
const STALE_SHAPE = 1e-3;

/** v1.62.0: round6's error on a stored centre fraction, a few writes over.
 *  Each write (an `add`, `place` or `set`, and each time a growing design
 *  re-expresses it) can move it by 7.5e-7 (`x` and half of `w` rounded);
 *  this is five writes' worth. `set` counts a box layout draws clamped at
 *  a window's edge as pushed in from past it only beyond this. */
const ROUNDED_CENTRE = 4e-6;

/** v1.62.0: how near, in CSS px, a box `set` draws must be to a window's
 *  edge to be on it: round6's error on a stored length (5e-7 of the design,
 *  up to 0.008 px at MAX_DESIGN_PX), twice over for a design that grew
 *  since. Far too little to see. */
const ON_EDGE_PX = 0.02;

/** v1.62.0: the text fields a `set` may carry besides `nw`/`nh`. */
const TEXT_KEYS = ["text", "font", "bold", "color", "outline", "align"] as const;

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

/** v1.62.0: a stored text element whose `text` holds half a surrogate pair
 *  without its other half, with that half made U+FFFD (text.ts's
 *  `wellFormedText`): left in, every push of the scene would fail at the
 *  IPC. Every other element, and every other text, is `e` itself. */
function wellFormedElement(e: SceneElement): SceneElement {
  if (e.type !== "text" || typeof e.text !== "string") return e;
  const text = wellFormedText(e.text);
  return text === e.text ? e : { ...e, text };
}

/** A stored scene, or `fallback` when there is nothing to read. Reading
 *  changes no element: it drops only entries nothing can address (no
 *  string id) and later duplicate ids, and settles `version` and
 *  `designSize`. Every other key, at any level, is kept. v1.62.0: but for
 *  a text's lone surrogate half, which becomes U+FFFD
 *  (`wellFormedElement`). */
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
      elements.push(wellFormedElement(e as SceneElement));
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

/** v1.59.0: a timer face this version could draw: usable geometry and a
 *  `timer` that passes the id grammar, whether or not that timer exists. */
export function asTimer(e: SceneElement): TimerElement | null {
  if (!isObject(e) || e.type !== "timer" || !validId(e.id) || !usable(e)) return null;
  if (!isTimerId(e.timer)) return null;
  return e as unknown as TimerElement;
}

/** v1.62.0: a text charm this version could draw: usable geometry, a valid
 *  id and a `text` that, normalized, isn't empty. No length rule: a text
 *  over the creation limits is live and drawn, cut at the drawing ceiling,
 *  so a later build's longer text isn't hidden and stranded here. Every
 *  other field has a read default (text.ts's `readTextFields`). */
export function asText(e: SceneElement): TextElement | null {
  if (!isObject(e) || e.type !== "text" || !validId(e.id) || !usable(e)) return null;
  if (normalizeCharmText(e.text) === null) return null;
  return e as unknown as TextElement;
}

/** v1.62.0: a recording light this version could draw: usable geometry and
 *  a valid id. It is live only as the scene's first such light
 *  (`liveElements`). */
export function asRec(e: SceneElement): RecElement | null {
  if (!isObject(e) || e.type !== "rec" || !validId(e.id) || !usable(e)) return null;
  return e as unknown as RecElement;
}

function liveEntries(s: OverlayScene): Array<{ el: KnownElement; index: number }> {
  const out: Array<{ el: KnownElement; index: number }> = [];
  let circle = false;
  let light = false;
  s.elements.forEach((e, index) => {
    const c = asCircle(e);
    if (c) {
      // One live circle: SpellCircle's gradient ids are document-global.
      if (!circle) out.push({ el: c, index });
      circle = true;
      return;
    }
    const img = asImage(e);
    if (img) {
      out.push({ el: img, index });
      return;
    }
    // v1.59.0: live whether or not its timer exists; the overlay draws a
    // face only for a timer the backend reports.
    const timer = asTimer(e);
    if (timer) {
      out.push({ el: timer, index });
      return;
    }
    const text = asText(e);
    if (text) {
      out.push({ el: text, index });
      return;
    }
    // v1.62.0: one live light, as for the circle: it reports one fact.
    const rec = asRec(e);
    if (rec) {
      if (!light) out.push({ el: rec, index });
      light = true;
    }
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
 *  (its stored shape when those are missing), 1 for the circle, and a
 *  timer's stored shape. v1.62.0: a text's natural box (`nw`/`nh`, its
 *  stored shape when those aren't whole numbers of at least 1), and the
 *  light's stored shape. */
export function aspectOf(e: KnownElement, design: Size): number {
  if (e.type === "spellcircle") return 1;
  if (e.type === "image" && pixelCount(e.nw) && pixelCount(e.nh)) return e.nw / e.nh;
  if (e.type === "text" && pixelCount(e.nw) && pixelCount(e.nh)) return e.nw / e.nh;
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
    case "set":
      return set(s, op.id, op.fields, op.viewport);
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
  // The circle goes to the back, anything else (a picture, a timer face, a
  // text, the light) to the front.
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
  const shown = layoutBox(target, d, v, target.type === "spellcircle");
  // v1.62.0: `set` can change a text's shape, so a place made against the
  // old one can arrive after it (in flight from the overlay, or the
  // mirror's pending copy re-applied). It lands as a move: the laid-out
  // size around the rect's centre. Without this, `normalizeRect` would keep
  // the old width and halve a widened text's height.
  const n = staleShape(target, rect, shown) ? centredOn(rect, shown, v) : normalizeRect(rect, aspectOf(target, d), v);
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

/** v1.62.0: a place on a text made against an older shape: its rect is off
 *  the text's own shape (`nw`/`nh`) by more than STALE_SHAPE, and off the
 *  shape the text is laid out in (`shown`) too. Gestures, nudges, snaps
 *  and keyboard resizes all carry the laid-out shape, which this build
 *  keeps equal to `nw`/`nh`, so only a place made before a `set` meets
 *  this. The second test keeps a stored shape that isn't the text's own (a
 *  hand edit) resizable: its gestures carry that shape, and land as
 *  resizes to `nw`/`nh`, as a picture's do. */
function staleShape(target: KnownElement, rect: PxRect, shown: PxRect): boolean {
  if (target.type !== "text" || !pixelCount(target.nw) || !pixelCount(target.nh)) return false;
  const a = rect.w / rect.h;
  const off = (b: number): boolean => Math.abs(a / b - 1) > STALE_SHAPE;
  return off(target.nw / target.nh) && off(shown.w / shown.h);
}

/** A box of `size`'s width and height centred where `r` is, kept inside
 *  `v`. */
function centredOn(r: PxRect, size: PxRect, v: Size): PxRect {
  const cx = r.x + r.w / 2;
  const cy = r.y + r.h / 2;
  return {
    x: clamp(cx - size.w / 2, 0, v.w - size.w),
    y: clamp(cy - size.h / 2, 0, v.h - size.h),
    w: size.w,
    h: size.h,
  };
}

/** v1.62.0: main's edit of a live text (§7.4): the patch's fields over the
 *  stored ones, the text normalized and a colour lower-cased; fields not
 *  in it, and unknown keys, are kept as stored, so a later build's font or
 *  a hand-set colour survives an edit of the words. The id and `z` never
 *  change. If the shape (`nw : nh`) changed, it keeps its designed height,
 *  takes the new shape's width, and is scaled down together by `fitSize`'s
 *  rule if that is wider than the design. Where it is drawn in `viewport`
 *  (the overlay window's CSS size; the design size when absent), it keeps
 *  the side its alignment names (left its left edge, right its right edge,
 *  centre its centre) and is kept inside that window, stored as a centre
 *  fraction that draws it there (`drawnFraction`). Down, it keeps its
 *  stored vertical centre (taken inside [0, 1]), as the design has it,
 *  which layout draws kept inside every window. `s` itself when
 *  `setRefusal` refuses or no stored value would change. */
function set(s: OverlayScene, id: string, fields: TextPatch, viewport?: Size): OverlayScene {
  if (setRefusal(s, id, fields) !== null) return s;
  // setRefusal found a live text with this id.
  const entry = liveEntries(s).find((x) => x.el.id === id)!;
  const e = s.elements[entry.index]!;
  const patch: Record<string, unknown> = { nw: fields.nw, nh: fields.nh };
  for (const k of TEXT_KEYS) {
    const v = fields[k];
    if (v === undefined) continue;
    patch[k] = k === "text" ? readCharmText(v) : k === "color" ? String(v).toLowerCase() : v;
  }
  if (Object.keys(patch).every((k) => e[k] === patch[k])) return s;
  let next: SceneElement = { ...e, ...patch };
  const sameShape = pixelCount(e.nw) && pixelCount(e.nh) && fields.nw * e.nh === fields.nh * e.nw;
  if (!sameShape) {
    const d = s.designSize;
    const r = e as unknown as FracRect;
    const size = fitSize((r.h * d.h * fields.nw) / fields.nh, r.h * d.h, d, MIN_ELEMENT_PX);
    const w = round6(size.w / d.w);
    const h = round6(size.h / d.h);
    const align = readTextFields(next).align;
    // The side is kept on screen, in the window it is seen in; then the box
    // is kept inside that window. Layout keeps a centre fraction and the
    // designed px size, shrunk to fit the window, so in a window that isn't
    // the design's size, keeping the side's design fraction (or clamping
    // inside the design's fractions) would move an edge-snapped label off
    // its edge, or a label that fits where it is. In a window of the design
    // size, for a box stored inside the design, they are the same.
    const v = validSize(viewport) ? viewport : d;
    const shown = layoutBox(r, d, v, false);
    // The new box's width as this window draws it.
    const lw = size.w * Math.min(1, v.w / size.w, v.h / size.h);
    const lx =
      align === "left" ? shown.x : align === "right" ? shown.x + shown.w - lw : shown.x + shown.w / 2 - lw / 2;
    // Across, stored as a centre fraction that draws it there
    // (`drawnFraction`); `ux` is what the design-fraction rule keeps. Down,
    // the stored vertical centre, taken inside [0, 1] (as `drawnFraction`
    // takes `kept`). Its designed height is the same unless `fitSize`
    // changed it, so it is where the design has it (on the design's top or
    // bottom, say), and layout draws it kept inside every window: in this
    // one around the vertical centre it was drawn on, or, drawn pushed in
    // from past the top or bottom, on that edge or nearer it. The centre
    // that draws it kept inside this window would draw the same here, but
    // would move a label lined up on a top or bottom edge off it in another
    // window, or here once it is edited back.
    const ux = align === "left" ? r.x + w / 2 : align === "right" ? r.x + r.w - w / 2 : r.x + r.w / 2;
    const pastX = r.x + r.w / 2 - (shown.x + shown.w / 2) / v.w;
    const left = drawnFraction(lx, lw, v.w, pastX, ux, shown.w) - w / 2;
    const top = clamp(r.y + r.h / 2, 0, 1) - h / 2;
    next = { ...next, x: round6(left), y: round6(top), w, h };
  }
  return { ...s, elements: s.elements.map((x, i) => (i === entry.index ? next : x)) };
}

/** `set`'s stored centre fraction across a window `span` px wide, for an
 *  edited box `len` px wide that would be drawn from `at`: kept inside the
 *  window. Usually the centre fraction that draws it there, so it is
 *  stored inside the window. But when layout was drawing the stored box
 *  pushed in from past this window's left or right edge (`past`: its
 *  stored centre fraction less the drawn one; `pushedIn`), or shrunk to
 *  fill the window's width (`was`: its drawn width), the stored fraction
 *  holds where it is in a window of another size (the design's, say),
 *  which this window can't show. Against an edge (to within ON_EDGE_PX, so
 *  moved at most that far onto it), the drawn fraction and every one past
 *  it draw the same here; of those, the one nearest `kept`, the
 *  design-fraction rule's. A centre fraction outside [0, 1] draws the box
 *  on that edge in every window, as 0 or 1 does, so `kept` is taken inside
 *  it (and round6 stays finite for a hand edit's huge one). So a label that
 *  rule keeps on a side of the design, drawn here pushed in or filling the
 *  window's width, and its edited box drawn on the same side here, is
 *  still on the design's side when the window grows back. */
function drawnFraction(at: number, len: number, span: number, past: number, kept: number, was: number): number {
  const p = clamp(at, 0, span - len);
  const c = (p + len / 2) / span;
  if (!pushedIn(past) && was < span - ON_EDGE_PX) return c;
  const lo = p <= ON_EDGE_PX ? -Infinity : c;
  const hi = p >= span - len - ON_EDGE_PX ? Infinity : c;
  return Math.min(Math.max(clamp(kept, 0, 1), lo), hi);
}

/** `set`: layout draws the stored box clamped at a window's edge, pushed
 *  in from past it, by more than round6's error (`past`: its stored centre
 *  fraction less the drawn one). */
function pushedIn(past: number): boolean {
  return Math.abs(past) > ROUNDED_CENTRE;
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
      // A timer face, a text or the light is never read off the wire: main
      // adds them itself.
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
    case "set":
      // v1.62.0: main's own edit, which never comes off the wire from
      // either side.
      return null;
    default:
      return null;
  }
}

/** Why `add` would refuse `e`, or null. In order: the id is taken; 64
 *  elements are stored; a timer face that wouldn't be live (v1.59.0: no
 *  other limit applies to one); v1.62.0: a text that wouldn't be live, is
 *  over the creation limits or lacks its natural box, and a light when a
 *  live one exists or it wouldn't be live (no other limit applies to
 *  either: both are outside the picture budget); a live circle exists; 16
 *  live pictures; it wouldn't be live, or a picture lacks its pixel size;
 *  the scene budget. */
export function addRefusal(s: OverlayScene, e: KnownElement): AddRefusal | null {
  if (s.elements.some((x) => x.id === e.id)) return "exists";
  if (s.elements.length >= MAX_ELEMENTS) return "elements";
  if (e.type === "timer") return asTimer(e) ? null : "invalid";
  if (e.type === "text") {
    const ok = asText(e) !== null && readCharmText(e.text) !== null && pixelCount(e.nw) && pixelCount(e.nh);
    return ok ? null : "invalid";
  }
  const live = liveElements(s);
  if (e.type === "rec") {
    if (live.some((x) => x.type === "rec")) return "light";
    return asRec(e) ? null : "invalid";
  }
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

/** v1.62.0: why `set` would refuse this patch, or null (it applies, or
 *  changes nothing). `gone`: no live text has `id`. `invalid`: `nw`/`nh`
 *  aren't whole numbers of at least 1, or a field it holds isn't one the
 *  reader accepts as itself: a text within the creation limits, a font or
 *  alignment among its three, a boolean bold or outline, a `#rrggbb`
 *  colour in any case. Lets main tell a refusal from no change. */
export function setRefusal(s: OverlayScene, id: string, fields: TextPatch): SetRefusal | null {
  if (!liveEntries(s).some((e) => e.el.id === id && e.el.type === "text")) return "gone";
  if (!isObject(fields) || !pixelCount(fields.nw) || !pixelCount(fields.nh)) return "invalid";
  // The reader's own tests (text.ts), so a value it reads as itself is
  // never refused.
  const { text, font, bold, color, outline, align } = fields as Record<string, unknown>;
  if (text !== undefined && readCharmText(text) === null) return "invalid";
  if (font !== undefined && !isTextFont(font)) return "invalid";
  if (align !== undefined && !isTextAlign(align)) return "invalid";
  if (bold !== undefined && typeof bold !== "boolean") return "invalid";
  if (outline !== undefined && typeof outline !== "boolean") return "invalid";
  if (color !== undefined && !isTextColour(color)) return "invalid";
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

/** v1.59.0: where a new timer face goes, as fractions of the scene's
 *  `designSize`: 3:1, its height 12 % of the window's short side (32 to
 *  160 px), centred across, its top at 8 % of the height and 6 % lower for
 *  each of the next four, then round again. `view` is the window's CSS
 *  size. */
export function defaultTimerRect(s: OverlayScene, view?: Size): FracRect {
  const d = s.designSize;
  const v = view && positive(view.w) && positive(view.h) ? view : d;
  const tall = clamp(0.12 * Math.min(v.w, v.h), 32, 160);
  const fitted = fitSize(3 * tall, tall, v, MIN_ELEMENT_PX);
  const k = Math.min(1, d.w / fitted.w, d.h / fitted.h);
  const w = fitted.w * k;
  const h = fitted.h * k;
  const timers = s.elements.filter((e) => asTimer(e) !== null).length;
  const top = clamp((0.08 + 0.06 * (timers % 5)) * d.h, 0, d.h - h);
  return {
    x: round6((d.w - w) / 2 / d.w),
    y: round6(top / d.h),
    w: round6(w / d.w),
    h: round6(h / d.h),
  };
}

/** `w × h` scaled down together until its width is at most `share` of
 *  `v`'s; then grown, as `fitSize` does, until its short side is at least
 *  MIN_ELEMENT_PX where `v` allows (which can take it past `share`); then
 *  scaled down to fit inside `d`. When the window is no bigger than `d`,
 *  this is the box `place` would store, so a new element's first drag or
 *  nudge is a move, not a resize. In a bigger window (enlarged with no drag
 *  since), a box wider than `d` is scaled down to fit it and can be under
 *  MIN_ELEMENT_PX, as a picture can; its first place then grows the design
 *  and resizes it. */
function fitShare(w: number, h: number, share: number, v: Size, d: Size): Size {
  const k = Math.min(1, (share * v.w) / w);
  const f = fitSize(w * k, h * k, v, MIN_ELEMENT_PX);
  const kd = Math.min(1, d.w / f.w, d.h / f.h);
  return { w: f.w * kd, h: f.h * kd };
}

/** v1.62.0: where a new text charm goes, as fractions of the scene's
 *  `designSize`: the shape of its natural box (`nw × nh`), its height 10 %
 *  of the window's short side (32 to 160 px), scaled down together to fit
 *  90 % of the window's width, but its short side at least 24 px where the
 *  window and the design allow (so a long one-line text can be wider than
 *  90 %, up to the whole width, and a one-letter one taller than 10 %; see
 *  `fitShare`), centred across, its top at 20 % of the height and 6 % lower
 *  for each of the next four, then round again. `view` is the window's CSS
 *  size. */
export function defaultTextRect(s: OverlayScene, nw: number, nh: number, view?: Size): FracRect {
  const d = s.designSize;
  const v = view && positive(view.w) && positive(view.h) ? view : d;
  const a = pixelCount(nw) && pixelCount(nh) ? nw / nh : 1;
  const tall = clamp(0.1 * Math.min(v.w, v.h), 32, 160);
  const { w, h } = fitShare(tall * a, tall, 0.9, v, d);
  const texts = s.elements.filter((e) => asText(e) !== null).length;
  const top = clamp((0.2 + 0.06 * (texts % 5)) * d.h, 0, d.h - h);
  return {
    x: round6((d.w - w) / 2 / d.w),
    y: round6(top / d.h),
    w: round6(w / d.w),
    h: round6(h / d.h),
  };
}

/** v1.62.0: where the recording light goes, as fractions of the scene's
 *  `designSize`: 5:1, its height 7 % of the window's short side (24 to
 *  96 px), scaled down together to fit 90 % of the window's width, but its
 *  short side at least 24 px where the window and the design allow (in a
 *  window under about 133 px wide it is then wider than 90 %; see
 *  `fitShare`), at the bottom left: its left edge 4 % of the width in,
 *  its bottom 4 % of the height up. Clear of Arrange's Move window grip and
 *  Lock pill, which sit at the top. `view` is the window's CSS size. */
export function defaultLightRect(s: OverlayScene, view?: Size): FracRect {
  const d = s.designSize;
  const v = view && positive(view.w) && positive(view.h) ? view : d;
  const tall = clamp(0.07 * Math.min(v.w, v.h), 24, 96);
  const { w, h } = fitShare(5 * tall, tall, 0.9, v, d);
  const left = clamp(0.04 * d.w, 0, d.w - w);
  const top = clamp(0.96 * d.h - h, 0, d.h - h);
  return {
    x: round6(left / d.w),
    y: round6(top / d.h),
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
 *  circle", the picture's name ("Picture" without one), or the timer's name
 *  as `timerName` gives it ("Timer" when it gives none: no such timer, or
 *  no `timerName`). v1.62.0: a text's first line, cut at 40 with "…", and
 *  "Recording light". Repeats are numbered " (2)", " (3)"… in array order,
 *  so adding a copy never renames the ones before it, and a suffix another
 *  element's own name already uses is skipped. */
export function charmLabels(
  s: OverlayScene,
  timerName?: (timerId: string) => string | undefined,
): Map<string, string> {
  const live = liveElements(s);
  const base = (e: KnownElement): string => {
    if (e.type === "spellcircle") return "Spell circle";
    if (e.type === "timer") {
      const name = timerName?.(e.timer);
      return typeof name === "string" && name.trim().length > 0 ? name.slice(0, MAX_NAME) : "Timer";
    }
    // Never empty: a live text's normalized text isn't, and it is trimmed.
    if (e.type === "text") return firstLine(normalizeCharmText(e.text) ?? "", TEXT_LABEL_CHARS);
    if (e.type === "rec") return "Recording light";
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
