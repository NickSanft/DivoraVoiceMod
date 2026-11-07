// v1.62.0 (Text charms): a text charm's look, its limits and its measure.
//
// Main and the overlay page both read this file: main to check what
// Settings saves and to measure the text's natural box (`nw`/`nh`), the
// overlay to draw the face and fit it inside its box. Both measure with the
// same font stack, so the canvas and the drawn face fall back the same way
// for letters the bundled fonts don't have. Pure, and imports nothing, so
// the overlay page and scene.ts can import it without a cycle. The stored
// names of the fonts, colours and alignments, and the creation limits, are
// frozen in wire.fixture.json → `textCharm`, which text.test.ts reads.
//
// A text element is live whenever its text, normalized, isn't empty. The
// 200-unit and 5-line limits are creation limits, applied when adding or
// setting, never when reading; drawing cuts a longer stored text at a
// ceiling far above them. Every other field has a read default, and the
// stored value is kept as stored.

/** The kill switch: false hides Add text… and Edit in Settings. Saved text
 *  still draws and can be removed. */
export const TEXT_CHARMS = true;

/** A text charm's stored font name. */
export type TextFont = "display" | "sans" | "mono";

/** A text charm's stored alignment. */
export type TextAlign = "left" | "center" | "right";

/** One of the bundled fonts: its Settings label, its family, and the face's
 *  whole stack (the CSS token's value, `src/styles.css`). */
export interface TextFontInfo {
  label: string;
  family: string;
  stack: string;
}

/** The three bundled fonts, keyed by the stored name, in Settings' order.
 *  Each stack equals its CSS token: `--font-display`, `--font-ui` and
 *  `--font-mono`. The face's `font-family` comes from here, never from a
 *  stored string. */
export const FONTS: Readonly<Record<TextFont, TextFontInfo>> = Object.freeze({
  display: {
    label: "Display",
    family: "Bricolage Grotesque Variable",
    stack: '"Bricolage Grotesque Variable", system-ui, sans-serif',
  },
  sans: {
    label: "Sans",
    family: "Space Grotesk Variable",
    stack: '"Space Grotesk Variable", system-ui, sans-serif',
  },
  mono: {
    label: "Mono",
    family: "Space Mono",
    stack: '"Space Mono", ui-monospace, monospace',
  },
});

/** One swatch: its Settings name and the `#rrggbb` it stores. */
export interface TextColour {
  name: string;
  hex: string;
}

/** The eight swatches, in Settings' order. Each is at least 0.50 from OBS
 *  Chroma Key's Green and Magenta presets in OBS's own key distance, so
 *  neither preset removes it at its defaults. */
export const COLOURS: readonly TextColour[] = Object.freeze([
  { name: "White", hex: "#ffffff" },
  { name: "Yellow", hex: "#ffd60a" },
  { name: "Orange", hex: "#ff9f0a" },
  { name: "Red", hex: "#ff4500" },
  { name: "Blue", hex: "#0a84ff" },
  { name: "Sky", hex: "#64d2ff" },
  { name: "Grey", hex: "#8e8e93" },
  { name: "Black", hex: "#111111" },
]);

/** The stored alignments, in Settings' order. */
export const ALIGNS: readonly TextAlign[] = Object.freeze(["left", "center", "right"]);

/** The creation limits: UTF-16 code units after normalization (as HTML
 *  `maxlength` counts them, so an emoji counts 2 or more), and lines. */
export const MAX_TEXT = 200;
export const MAX_LINES = 5;

/** The drawing ceiling, far above the creation limits: a longer stored text
 *  (a later build's, or a hand edit) still draws, cut here. */
export const DRAW_MAX_CHARS = 4096;
export const DRAW_MAX_LINES = 50;

/** The font size `nw`/`nh` are measured at, in CSS px. */
export const REF_PX = 100;

/** The face's line height, in em. */
export const LINE_H = 1.15;

/** The face's padding on each side, in em: it covers the outline. */
export const PAD_EM = 0.12;

/** What Settings' editor holds and saves: the six fields a streamer
 *  chooses. */
export interface TextInput {
  text: string;
  font: TextFont;
  bold: boolean;
  /** `#rrggbb`. */
  color: string;
  outline: boolean;
  align: TextAlign;
}

/** A text element's fields: the six, and the text's natural box in whole
 *  CSS px at REF_PX. */
export interface TextFields extends TextInput {
  nw: number;
  nh: number;
}

/** What a `set` carries: the box the new fields measure to, and only the
 *  fields the streamer changed. */
export type TextPatch = { nw: number; nh: number } & Partial<TextInput>;

/** How a window measures text: injected, so tests use fakes
 *  (textMetrics.ts holds the canvas one). `load` waits for the fonts a
 *  `spec` names to cover `sample`, and never rejects. `width` is one line's
 *  width at `spec`, in CSS px. */
export interface TextMetrics {
  load(spec: string, sample: string): Promise<void>;
  width(spec: string, line: string): number;
}

/** A font name this build draws. A plain object lookup would also take
 *  "constructor" and the rest of Object's names. */
export function isTextFont(v: unknown): v is TextFont {
  return v === "display" || v === "sans" || v === "mono";
}

/** An alignment this build draws. */
export function isTextAlign(v: unknown): v is TextAlign {
  return v === "left" || v === "center" || v === "right";
}

/** A colour a text charm can store: `#rrggbb` in any case, any value, not
 *  only a swatch (a hand edit, or a later build's picker). Read and stored
 *  lower-cased. */
export function isTextColour(v: unknown): v is string {
  return typeof v === "string" && /^#[0-9a-fA-F]{6}$/.test(v);
}

/** The text as stored, read for drawing and liveness (§7.1), in order: CR
 *  LF, CR, U+2028, U+2029 and U+0085 become LF (left in, U+2028 and U+2029
 *  would break lines `split("\n")` doesn't count); a tab becomes a space;
 *  every other control character (U+0000–U+001F but LF, U+007F and
 *  U+0080–U+009F) is dropped; then the whole is trimmed. Null for a
 *  non-string or for nothing left. No length rule. */
export function normalizeCharmText(raw: unknown): string | null {
  if (typeof raw !== "string") return null;
  const text = raw
    .replace(/\r\n|[\r\u2028\u2029\u0085]/g, "\n")
    .replace(/\t/g, " ")
    .replace(/[\u0000-\u0009\u000b-\u001f\u007f-\u009f]/g, "")
    .trim();
  return text.length > 0 ? text : null;
}

/** `normalizeCharmText` within the creation limits, else null: for `add`,
 *  `set` and Save. MAX_TEXT counts UTF-16 code units, as HTML `maxlength`
 *  does, so "🎮" counts 2; lines are counted after the trim. */
export function readCharmText(raw: unknown): string | null {
  const text = normalizeCharmText(raw);
  if (text === null || text.length > MAX_TEXT || text.split("\n").length > MAX_LINES) return null;
  return text;
}

/** The normalized text's length in UTF-16 code units, as the limit counts
 *  it; 0 for nothing. Settings' "{n} of 200". */
export function textLength(raw: unknown): number {
  return normalizeCharmText(raw)?.length ?? 0;
}

/** A UTF-16 high surrogate: the first half of a pair. */
function highSurrogate(code: number): boolean {
  return code >= 0xd800 && code <= 0xdbff;
}

/** `text` cut to its first `n` code units, one fewer if the cut would end
 *  inside a surrogate pair. */
function cutUnits(text: string, n: number): string {
  if (text.length <= n) return text;
  const end = n > 0 && highSurrogate(text.charCodeAt(n - 1)) ? n - 1 : n;
  return text.slice(0, end);
}

/** The text as the face draws it: normalized, then its first
 *  DRAW_MAX_LINES lines, then its first DRAW_MAX_CHARS code units, never
 *  ending inside a surrogate pair. "" for nothing to draw. The ceiling is
 *  far above the creation limits, so it only cuts a text a later build or a
 *  hand edit stored. */
export function drawText(text: string): string {
  const lines = (normalizeCharmText(text) ?? "").split("\n");
  return cutUnits(lines.slice(0, DRAW_MAX_LINES).join("\n"), DRAW_MAX_CHARS);
}

/** The six fields as this build shows them, from a stored element (or one
 *  with a patch spread over it): the text normalized ("" for none); a font,
 *  colour or alignment it can't use reads as its default (sans, `#ffffff`,
 *  centre), a non-boolean bold or outline as true; a colour lower-cased.
 *  Reading never rewrites a stored value, and a stored string never
 *  reaches a style: the font's family comes from FONTS. */
export function readTextFields(e: Record<string, unknown>): TextInput {
  return {
    text: normalizeCharmText(e.text) ?? "",
    font: isTextFont(e.font) ? e.font : "sans",
    bold: typeof e.bold === "boolean" ? e.bold : true,
    color: isTextColour(e.color) ? e.color.toLowerCase() : "#ffffff",
    outline: typeof e.outline === "boolean" ? e.outline : true,
    align: isTextAlign(e.align) ? e.align : "center",
  };
}

/** The fields whose draft differs from what the editor opened with, in
 *  the editor's order: the text compared and given normalized (so a
 *  trailing space typed and deleted isn't a change), a colour compared in
 *  any case and given lower-cased. A field left as it opened, such as a
 *  stored colour that isn't a swatch, is absent, so Edit never rewrites
 *  it. */
export function changedFields(initial: TextInput, draft: TextInput): Partial<TextInput> {
  const out: Partial<TextInput> = {};
  const text = normalizeCharmText(draft.text);
  if (text !== normalizeCharmText(initial.text)) out.text = text ?? "";
  if (draft.font !== initial.font) out.font = draft.font;
  if (draft.bold !== initial.bold) out.bold = draft.bold;
  if (draft.color.toLowerCase() !== initial.color.toLowerCase()) out.color = draft.color.toLowerCase();
  if (draft.outline !== initial.outline) out.outline = draft.outline;
  if (draft.align !== initial.align) out.align = draft.align;
  return out;
}

/** Up to the next whole px. A width or line height that is a whole number
 *  but for float error (1.15 · 100 is 114.99999999999999) isn't bumped. */
function ceilPx(n: number): number {
  return Math.ceil(n - 1e-9);
}

/** The text's natural box at REF_PX, in whole CSS px (§7.2): the widest
 *  line (an empty line is 0) plus PAD_EM a side, by its lines at LINE_H
 *  plus PAD_EM top and bottom. `widthOf` measures one line at REF_PX with
 *  `fontSpec`'s whole stack; a width that isn't a finite number of at
 *  least 0 counts as 0. `nw` is at least 1. */
export function textBox(text: string, widthOf: (line: string) => number): { nw: number; nh: number } {
  const lines = text.split("\n");
  let widest = 0;
  for (const line of lines) {
    const w = line.length > 0 ? widthOf(line) : 0;
    if (Number.isFinite(w) && w > widest) widest = w;
  }
  const pad = 2 * PAD_EM * REF_PX;
  return {
    nw: Math.max(1, ceilPx(widest + pad)),
    nh: ceilPx(lines.length * LINE_H * REF_PX + pad),
  };
}

/** The face's font size in CSS px for a box of `boxW × boxH` holding a text
 *  whose natural box at REF_PX is `nw × nh`: fitted inside ("contain"), so
 *  a box wider than the text's shape is bound by its height and the text is
 *  aligned across it. 0 when any input isn't a positive finite number. */
export function fitTextFont(boxW: number, boxH: number, nw: number, nh: number): number {
  for (const n of [boxW, boxH, nw, nh]) if (!(Number.isFinite(n) && n > 0)) return 0;
  return REF_PX * Math.min(boxW / nw, boxH / nh);
}

/** The face's whole font stack for a stored font name; Sans's for a name
 *  this build doesn't know. */
export function fontStack(font: unknown): string {
  return FONTS[isTextFont(font) ? font : "sans"].stack;
}

/** The canvas font that measures a face: its weight, REF_PX and the face's
 *  whole stack, so the canvas runs the same fallback chain as the drawn
 *  face. `700 100px "Space Grotesk Variable", system-ui, sans-serif`. */
export function fontSpec(font: unknown, bold: boolean): string {
  return `${bold ? 700 : 400} ${REF_PX}px ${fontStack(font)}`;
}

/** Text lighter than this (WCAG relative luminance) gets the dark outline,
 *  the rest the light one. Every swatch but Black is above it. */
const OUTLINE_LUMINANCE = 0.18;

/** WCAG relative luminance of a `#rrggbb` colour. */
function luminance(hex: string): number {
  const [r, g, b] = [1, 3, 5].map((i) => {
    const c = parseInt(hex.slice(i, i + 2), 16) / 255;
    return c <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4;
  }) as [number, number, number];
  return 0.2126 * r + 0.7152 * g + 0.0722 * b;
}

/** The outline's colour for text of colour `hex`: `#000000` for light text
 *  (luminance above 0.18), `#ffffff` for dark text (§7.3). A colour this
 *  build can't read draws white (`readTextFields`), so it gets the dark
 *  one. */
export function outlineColor(hex: string): string {
  const light = !isTextColour(hex) || luminance(hex) > OUTLINE_LUMINANCE;
  return light ? "#000000" : "#ffffff";
}

/** The outline as a `text-shadow` for text of colour `hex`: the timer
 *  face's three layers (Stage.tsx's TIMER_FACE), in black, or the same in
 *  white for dark text. Both are clear of OBS's key presets. */
export function textShadow(hex: string): string {
  const dark = outlineColor(hex) === "#000000";
  const rgb = dark ? "0, 0, 0" : "255, 255, 255";
  return `0 0 0.06em ${dark ? "#000" : "#fff"}, 0 0 0.12em rgba(${rgb}, 0.85), 0 0.04em 0.1em rgba(${rgb}, 0.6)`;
}

/** The text's first line, its trailing spaces dropped, cut at `max` code
 *  units with "…" after them, never inside a surrogate pair: a row's
 *  label. */
export function firstLine(text: string, max: number): string {
  const line = (text.split("\n")[0] ?? "").trimEnd();
  if (line.length <= max) return line;
  return `${cutUnits(line, max).trimEnd()}…`;
}
