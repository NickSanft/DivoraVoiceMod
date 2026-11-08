// v1.62.0 (Text charms): measuring text in a window, for text.ts's
// `textBox`.
//
// Main measures a text when Settings saves it, to store its natural box
// (`nw`/`nh`); the overlay page measures it again before drawing, and fits
// its face by its own measure. Both use one shared canvas whose font is
// text.ts's `fontSpec` (the face's whole stack), with the face's kerning,
// letter spacing and text rendering, and both wait for the fonts first, at
// most FONT_WAIT_MS. Uses the DOM, so it is not one of the pure modules;
// the controller and Stage take a `TextMetrics`, and tests inject fakes.
//
// Main's `load` gives up at FONT_WAIT_MS, so a save never waits longer. The
// overlay page's doesn't: Stage keeps its own FONT_WAIT_MS timer, draws in
// the fallback when it fires, and measures again when the font does arrive,
// which a capped `load` would never tell it.

import { REF_PX, type TextInk, type TextMetrics } from "./text";

/** The longest a face waits for its font before it draws in the fallback,
 *  and the longest main waits before measuring. The overlay page
 *  re-exports it (v1.59.0's timer face uses it too). */
export const FONT_WAIT_MS = 1000;

/** A line's width where the window has no 2D canvas (jsdom): 0.6 em a code
 *  unit at REF_PX. */
const NO_CANVAS_EM = 0.6;

/** The one canvas context every measure shares; null where there is none,
 *  undefined until the first measure asks (jsdom reports a canvas it can't
 *  draw on as an error, so only a measure creates it). */
let shared: CanvasRenderingContext2D | null | undefined;
/** The spec last given to `shared.font`: the canvas reads it back
 *  serialized, so it is kept as given. */
let sharedSpec = "";

function context(): CanvasRenderingContext2D | null {
  if (shared !== undefined) return shared;
  try {
    shared = document.createElement("canvas").getContext("2d") ?? null;
  } catch {
    shared = null;
  }
  return shared;
}

/** The face's text settings, set again with each new `font`: the face sets
 *  `font-kerning: normal; letter-spacing: 0; text-rendering:
 *  optimizeLegibility` (Stage.tsx), and `body` gives it the last one too
 *  (src/styles.css). */
function faceSettings(ctx: CanvasRenderingContext2D): void {
  ctx.fontKerning = "normal";
  ctx.letterSpacing = "0px";
  ctx.textRendering = "optimizeLegibility";
  // `ink` reads from the alphabetic baseline, as CSS lays a line out.
  ctx.textBaseline = "alphabetic";
}

/** One line measured on the shared canvas at `spec`; null where the window
 *  has no 2D canvas. */
function measured(spec: string, line: string): ReturnType<CanvasRenderingContext2D["measureText"]> | null {
  const ctx = context();
  if (!ctx) return null;
  if (sharedSpec !== spec) {
    ctx.font = spec;
    sharedSpec = spec;
    faceSettings(ctx);
  }
  return ctx.measureText(line);
}

/** `document.fonts.load(spec, sample)` settled either way; at once where the
 *  page can't wait for fonts (or the spec doesn't parse). */
function fontsLoaded(spec: string, sample: string): Promise<void> {
  const fonts = (document as { fonts?: Partial<FontFaceSet> }).fonts;
  if (typeof fonts?.load !== "function") return Promise.resolve();
  try {
    return fonts.load(spec, sample).then(
      () => undefined,
      () => undefined,
    );
  } catch {
    return Promise.resolve();
  }
}

/** The window's measure. `load` waits for `document.fonts.load(spec,
 *  sample + " ")` (the named families' faces for those letters, and for
 *  the space; generic families are skipped), at most `waitMs`, and never
 *  rejects; pass `Infinity` for no cap. The space is there because the
 *  face that has it is the line's first font, whose ascent and descent
 *  place each line (`ink`'s font ascent and descent), and the face draws
 *  in it. A text with none of the font's own letters (one Cyrillic word,
 *  say, or an emoji) would otherwise load nothing, and a window that
 *  hadn't loaded that face yet (the overlay page) would measure its lines
 *  by a fallback's ascent and descent, while main, whose own UI has loaded
 *  it, measures by the font's. `width` is one line's width at `spec` on
 *  the shared canvas, in CSS px, or NO_CANVAS_EM a code unit where the
 *  window has no 2D canvas. `ink` is the same measure's ink (its actual
 *  and font bounding boxes from the baseline), or null with no 2D canvas
 *  or a value that isn't a finite number. */
export function canvasMetrics(waitMs: number = FONT_WAIT_MS): TextMetrics {
  return {
    load(spec, sample) {
      const loaded = fontsLoaded(spec, `${sample} `);
      if (!Number.isFinite(waitMs)) return loaded;
      return new Promise<void>((resolve) => {
        const timer = setTimeout(resolve, Math.max(0, waitMs));
        void loaded.then(() => {
          clearTimeout(timer);
          resolve();
        });
      });
    },
    width(spec, line) {
      const m = measured(spec, line);
      return m ? m.width : NO_CANVAS_EM * REF_PX * line.length;
    },
    ink(spec, line): TextInk | null {
      const m = measured(spec, line);
      if (!m) return null;
      const ink = {
        ascent: m.actualBoundingBoxAscent,
        descent: m.actualBoundingBoxDescent,
        fontAscent: m.fontBoundingBoxAscent,
        fontDescent: m.fontBoundingBoxDescent,
      };
      return Object.values(ink).every((n) => typeof n === "number" && Number.isFinite(n)) ? ink : null;
    },
  };
}
