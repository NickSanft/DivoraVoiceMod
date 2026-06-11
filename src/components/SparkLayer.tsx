// SparkLayer — canvas-based glyph cast surface.
//
// Replaces the SVG GlyphCastOverlay + the HTML SpellCastReveal with a
// single transparent canvas that lives over the Mixer at all times.
// Drag anywhere not on a UI control to draw a glyph; release to either:
//   - cast the bound preset (with an "omen" — outlined shape + preset
//     name + "◆ SPELL CAST ◆" eyebrow, blooming over 2.5 s), or
//   - call `onMessage` with a short string ("Glyph not recognised…",
//     "No preset bound to △") which the host displays as a flash.
//
// Ported from docs/mockups/prototype/divora/spark_canvas.jsx with one
// key difference: instead of a window-relative virtual 1100×720 coord
// space, we use device-pixel-aware canvas sizing so 1 canvas unit ≈
// 1 CSS pixel. This keeps lines crisp on hi-DPI displays.
//
// All particle + omen state lives in a single mutable `state` object
// so the rAF loop doesn't have to chase Solid signals 60 fps.

import { onCleanup, onMount, type JSX } from "solid-js";
import { detectShape, type DetectedShape, type Point } from "../data/glyphs";
import type { GlyphId } from "../types";

const SPARK_CAP = 1600;
const OMEN_LIFE_MS = 2500;
const OMEN_POP_MS = 280;
const OMEN_FADE_TAIL_MS = 650;
/** Default trail palette when no preset is in play. */
const TRAIL = ["#7C5CF6", "#EC4899", "#9F7CFF", "#C9B8FF"] as const;

interface Spark {
  x: number;
  y: number;
  vx: number;
  vy: number;
  /** 0..1 alpha. Decremented by `decay` per frame; spark dies at 0. */
  life: number;
  decay: number;
  size: number;
  color: string;
}

/** A recognised cast: a glyph id + where it was drawn + the outline to
 *  render (built-in shape geometry OR a custom normalized template). */
interface GlyphHit {
  glyphId: string;
  shape: DetectedShape | null;
  outline: Point[] | null;
  cx: number;
  cy: number;
  size: number;
}

interface Omen {
  cx: number;
  cy: number;
  size: number;
  /** Built-in shape geometry for the outline (null for custom glyphs). */
  shape: DetectedShape | null;
  /** Custom glyph normalized template polyline (null for built-ins). */
  outline: Point[] | null;
  /** Brand color (preset colour, or the accent for non-preset actions). */
  color: string;
  /** Action label drawn under the shape (preset name, "Monitor", …). */
  name: string;
  /** Theme-aware "SPELL CAST" eyebrow color, read off :root at cast time
   *  so it stays legible on both the dark and light themes. */
  eyebrow: string;
  /** performance.now() at omen start. */
  start: number;
}

/** Read a CSS custom property off :root, with a fallback for tests/SSR.
 *  Canvas can't reference CSS vars, so theme-dependent canvas colors are
 *  snapshotted from the live tokens here. */
function readToken(name: string, fallback: string): string {
  if (typeof window === "undefined") return fallback;
  try {
    const v = getComputedStyle(document.documentElement)
      .getPropertyValue(name)
      .trim();
    return v || fallback;
  } catch {
    return fallback;
  }
}

interface SparkState {
  parts: Spark[];
  drawing: boolean;
  path: Point[];
  last: Point | null;
  raf: number;
  running: boolean;
  omen: Omen | null;
}

export interface SparkLayerProps {
  /** Match the drawn path against the user's custom glyphs (the built-in
   *  four shapes are recognised internally via `detectShape`). Returns the
   *  matched glyph id + its normalized template, or null. */
  recognizeCustom: (path: Point[]) => { id: string; template: Point[] } | null;
  /** Resolve a recognised glyph id (built-in `GlyphId` or a custom id) to a
   *  cast outcome — canvas colour, label, and the action to run — or null
   *  when the glyph has no binding. */
  resolveGlyph: (
    glyphId: string,
  ) => { color: string; label: string; run: () => void } | null;
  /** Short messages for the host's flash banner (unrecognised, unbound). */
  onMessage: (text: string) => void;
}

/**
 * Predicate matching the prototype's `isInteractive`. Walks via
 * `closest()` so a click that started on any descendant of a control
 * still counts as "control" and bypasses the cast. Exposed for tests.
 */
export function isCastBlocker(el: EventTarget | null): boolean {
  if (!el || !(el instanceof Element)) return false;
  return !!el.closest(
    [
      "button",
      "a",
      "input",
      "select",
      "textarea",
      '[role="button"]',
      '[role="slider"]',
      '[role="switch"]',
      '[role="tab"]',
      '[role="radio"]',
      '[role="radiogroup"]',
      ".card",
      ".seg",
      ".kbd",
      // v0.11.5: the custom titlebar uses `data-tauri-drag-region` to
      // mark surfaces the OS should treat as a draggable window chrome.
      // Without this exclusion, the cast surface intercepts the
      // pointerdown in capture-phase before Tauri's native drag handler
      // can fire — and the window stops being draggable. Adding the
      // attribute selector lets the drag continue to the OS.
      "[data-tauri-drag-region]",
      "[data-cast-block]",
    ].join(","),
  );
}

export function SparkLayer(props: SparkLayerProps): JSX.Element {
  let canvasRef: HTMLCanvasElement | undefined;
  const state: SparkState = {
    parts: [],
    drawing: false,
    path: [],
    last: null,
    raf: 0,
    running: false,
    omen: null,
  };

  /** Look-up done at draw time, so prop changes flow without re-mount. */
  const propsRef = {
    recognizeCustom: props.recognizeCustom,
    resolveGlyph: props.resolveGlyph,
    onMessage: props.onMessage,
  };

  onMount(() => {
    if (!canvasRef) return;
    const canvas = canvasRef;
    const ctx = canvas.getContext("2d");
    if (!ctx) return;

    // Track devicePixelRatio so strokes stay crisp on hi-DPI. We resize
    // the canvas's intrinsic buffer to match CSS size × DPR and scale
    // the context once; all draw calls use CSS pixels.
    let dpr = window.devicePixelRatio || 1;
    const resize = (): void => {
      const rect = canvas.getBoundingClientRect();
      const w = Math.max(1, rect.width);
      const h = Math.max(1, rect.height);
      dpr = window.devicePixelRatio || 1;
      canvas.width = Math.round(w * dpr);
      canvas.height = Math.round(h * dpr);
      ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    };
    resize();

    const toLocal = (clientX: number, clientY: number): Point => {
      const r = canvas.getBoundingClientRect();
      return { x: clientX - r.left, y: clientY - r.top };
    };

    const spawn = (
      x: number,
      y: number,
      n: number,
      opts: { dir?: number; burst?: number; size?: number; colors?: readonly string[] } = {},
    ): void => {
      const pal = opts.colors ?? TRAIL;
      const burst = opts.burst;
      for (let i = 0; i < n; i++) {
        const ang =
          burst != null
            ? Math.random() * Math.PI * 2
            : (opts.dir ?? 0) + (Math.random() - 0.5) * 1.4;
        const spd =
          burst != null ? 1.5 + Math.random() * burst : 0.3 + Math.random() * 0.9;
        state.parts.push({
          x,
          y,
          vx: Math.cos(ang) * spd,
          vy: Math.sin(ang) * spd - (burst != null ? 0 : 0.3),
          life: 1,
          decay: 0.012 + Math.random() * 0.02,
          size: (opts.size ?? 2) + Math.random() * 2.4,
          color: pal[Math.floor(Math.random() * pal.length)]!,
        });
      }
      // Cap the particle pool so a marathon stroke can't OOM us. We
      // splice rather than rotate to keep the most recent emissions.
      if (state.parts.length > SPARK_CAP) {
        state.parts.splice(0, state.parts.length - SPARK_CAP);
      }
      ensure();
    };

    const drawShapeOutline = (o: DetectedShape, half: number): void => {
      const x = o.cx;
      const y = o.cy;
      ctx.beginPath();
      if (o.type === "triangle") {
        ctx.moveTo(x, y - half);
        ctx.lineTo(x + half * 0.92, y + half * 0.7);
        ctx.lineTo(x - half * 0.92, y + half * 0.7);
        ctx.closePath();
      } else if (o.type === "invtriangle") {
        ctx.moveTo(x - half * 0.92, y - half * 0.7);
        ctx.lineTo(x + half * 0.92, y - half * 0.7);
        ctx.lineTo(x, y + half);
        ctx.closePath();
      } else if (o.type === "square") {
        const s = half * 0.82;
        const rad = 10;
        // Rounded-corner square via arcTo.
        ctx.moveTo(x - s + rad, y - s);
        ctx.arcTo(x + s, y - s, x + s, y + s, rad);
        ctx.arcTo(x + s, y + s, x - s, y + s, rad);
        ctx.arcTo(x - s, y + s, x - s, y - s, rad);
        ctx.arcTo(x - s, y - s, x + s, y - s, rad);
        ctx.closePath();
      } else {
        // circle
        ctx.arc(x, y, half * 0.86, 0, Math.PI * 2);
      }
      ctx.stroke();
    };

    const drawOmen = (now: number): void => {
      const o = state.omen;
      if (!o) return;
      const age = now - o.start;
      if (age > OMEN_LIFE_MS) {
        state.omen = null;
        return;
      }
      // Three-phase alpha envelope: pop-in, hold, fade-out.
      let a = 1;
      if (age < OMEN_POP_MS) a = age / OMEN_POP_MS;
      else if (age > OMEN_LIFE_MS - OMEN_FADE_TAIL_MS) {
        a = (OMEN_LIFE_MS - age) / OMEN_FADE_TAIL_MS;
      }
      a = Math.max(0, Math.min(1, a));
      const pop =
        age < OMEN_POP_MS ? 0.6 + 0.4 * (age / OMEN_POP_MS) : 1;
      const half = Math.min(150, Math.max(95, o.size * 0.5)) * pop;

      ctx.save();
      ctx.strokeStyle = o.color;
      ctx.shadowColor = o.color;
      ctx.shadowBlur = 22;
      ctx.lineWidth = 3.2;
      ctx.lineJoin = "round";

      // Expanding halo ring behind the shape (subtle, low alpha).
      ctx.globalAlpha = a * 0.32;
      ctx.beginPath();
      ctx.arc(o.cx, o.cy, half * (1.12 + (age / OMEN_LIFE_MS) * 0.55), 0, Math.PI * 2);
      ctx.stroke();

      // The outline itself — a built-in shape, or the custom template stroke.
      ctx.globalAlpha = a;
      if (o.shape) {
        drawShapeOutline(o.shape, half);
      } else if (o.outline && o.outline.length > 1) {
        ctx.beginPath();
        o.outline.forEach((pt, i) => {
          const px = o.cx + pt.x * half;
          const py = o.cy + pt.y * half;
          if (i === 0) ctx.moveTo(px, py);
          else ctx.lineTo(px, py);
        });
        ctx.stroke();
      }

      // Labels — action name + SPELL CAST eyebrow.
      ctx.shadowBlur = 14;
      ctx.fillStyle = o.color;
      ctx.textAlign = "center";
      ctx.textBaseline = "middle";
      ctx.font = '700 19px "Bricolage Grotesque", system-ui, sans-serif';
      ctx.fillText(o.name, o.cx, o.cy + half + 28);
      ctx.shadowBlur = 0;
      ctx.fillStyle = o.eyebrow;
      ctx.globalAlpha = a * 0.65;
      ctx.font = '700 10px "Space Mono", monospace';
      ctx.fillText("◆  S P E L L   C A S T  ◆", o.cx, o.cy + half + 47);
      ctx.restore();
    };

    const tick = (now: number): void => {
      const w = canvas.width / dpr;
      const h = canvas.height / dpr;
      ctx.clearRect(0, 0, w, h);
      const ps = state.parts;
      for (let i = ps.length - 1; i >= 0; i--) {
        const p = ps[i]!;
        p.x += p.vx;
        p.y += p.vy;
        p.vy += 0.012; // gravity
        p.vx *= 0.985;
        p.vy *= 0.985;
        p.life -= p.decay;
        if (p.life <= 0) {
          ps.splice(i, 1);
          continue;
        }
        ctx.globalAlpha = Math.max(0, p.life);
        ctx.fillStyle = p.color;
        ctx.shadowColor = p.color;
        ctx.shadowBlur = 10;
        ctx.beginPath();
        ctx.arc(p.x, p.y, p.size * p.life, 0, Math.PI * 2);
        ctx.fill();
      }
      ctx.globalAlpha = 1;
      ctx.shadowBlur = 0;
      drawOmen(now);

      const omenAlive =
        state.omen !== null && now - state.omen.start <= OMEN_LIFE_MS;
      if (ps.length > 0 || state.drawing || omenAlive) {
        state.raf = requestAnimationFrame(tick);
      } else {
        state.running = false;
      }
    };
    const ensure = (): void => {
      if (!state.running) {
        state.running = true;
        state.raf = requestAnimationFrame(tick);
      }
    };

    const BUILTIN_IDS: GlyphId[] = ["triangle", "invtriangle", "square", "circle"];

    /** Bounding-box centre + diagonal of a drawn path (positions a custom
     *  glyph's omen where the user actually drew). */
    const pathBox = (pts: Point[]): { cx: number; cy: number; size: number } => {
      let minX = Infinity;
      let minY = Infinity;
      let maxX = -Infinity;
      let maxY = -Infinity;
      for (const p of pts) {
        if (p.x < minX) minX = p.x;
        if (p.y < minY) minY = p.y;
        if (p.x > maxX) maxX = p.x;
        if (p.y > maxY) maxY = p.y;
      }
      return {
        cx: (minX + maxX) / 2,
        cy: (minY + maxY) / 2,
        size: Math.hypot(maxX - minX, maxY - minY),
      };
    };

    const castHit = (hit: GlyphHit): void => {
      const outcome = propsRef.resolveGlyph(hit.glyphId);
      if (!outcome) {
        const label = (BUILTIN_IDS as string[]).includes(hit.glyphId)
          ? glyphLabel(hit.glyphId as GlyphId)
          : "that glyph";
        propsRef.onMessage(`No action bound to ${label}`);
        return;
      }
      const colors = [outcome.color, "#FFFFFF", outcome.color, "#C9B8FF"];
      const half = Math.min(150, Math.max(95, hit.size * 0.5));
      // Trace sparks along the recognised outline so the user sees the
      // app "complete" their stroke.
      if (hit.shape?.corners) {
        const c = hit.shape.corners;
        for (let e = 0; e < c.length; e++) {
          const a = c[e]!;
          const b = c[(e + 1) % c.length]!;
          const steps = Math.max(
            6,
            Math.floor(Math.hypot(b.x - a.x, b.y - a.y) / 14),
          );
          for (let s = 0; s <= steps; s++) {
            spawn(
              a.x + ((b.x - a.x) * s) / steps,
              a.y + ((b.y - a.y) * s) / steps,
              2,
              { colors, size: 2.2 },
            );
          }
        }
      } else if (hit.shape?.r) {
        for (let i = 0; i < 28; i++) {
          const ang = (i / 28) * Math.PI * 2;
          spawn(
            hit.cx + hit.shape.r * Math.cos(ang),
            hit.cy + hit.shape.r * Math.sin(ang),
            2,
            { colors, size: 2.2 },
          );
        }
      } else if (hit.outline) {
        for (const pt of hit.outline) {
          spawn(hit.cx + pt.x * half, hit.cy + pt.y * half, 1, { colors, size: 2.2 });
        }
      }
      // Central burst.
      spawn(hit.cx, hit.cy, 110, { colors, burst: 6.5, size: 2.6 });
      state.omen = {
        cx: hit.cx,
        cy: hit.cy,
        size: hit.size,
        shape: hit.shape,
        outline: hit.outline,
        color: outcome.color,
        name: outcome.label,
        eyebrow: readToken("--text-hi", "rgba(233,231,248,0.85)"),
        start: performance.now(),
      };
      ensure();
      outcome.run();
    };

    const onDown = (ev: PointerEvent): void => {
      if (ev.button !== 0) return;
      if (isCastBlocker(ev.target)) {
        state.drawing = false;
        return;
      }
      ev.preventDefault();
      state.drawing = true;
      state.path = [];
      const p = toLocal(ev.clientX, ev.clientY);
      state.last = p;
      state.path.push(p);
      spawn(p.x, p.y, 4);
    };
    const onMove = (ev: PointerEvent): void => {
      if (!state.drawing) return;
      const p = toLocal(ev.clientX, ev.clientY);
      const last = state.last;
      const d = last ? Math.hypot(p.x - last.x, p.y - last.y) : 0;
      const dir = last ? Math.atan2(p.y - last.y, p.x - last.x) : 0;
      spawn(p.x, p.y, Math.min(6, 1 + Math.floor(d / 6)), { dir });
      state.last = p;
      state.path.push(p);
    };
    const onUp = (): void => {
      if (!state.drawing) return;
      state.drawing = false;
      const path = state.path;
      state.path = [];
      if (path.length <= 4) return;
      // Built-in four shapes first (their tuned geometric detector + omen).
      const shape = detectShape(path);
      if (shape) {
        castHit({
          glyphId: shape.type,
          shape,
          outline: null,
          cx: shape.cx,
          cy: shape.cy,
          size: shape.size,
        });
        return;
      }
      // Then the user's custom glyphs (unistroke template match).
      const custom = propsRef.recognizeCustom(path);
      if (custom) {
        const box = pathBox(path);
        castHit({
          glyphId: custom.id,
          shape: null,
          outline: custom.template,
          cx: box.cx,
          cy: box.cy,
          size: box.size,
        });
        return;
      }
      // Drew something, but it didn't classify — a brief hint.
      propsRef.onMessage("Glyph not recognised — try again");
    };

    // Capture-phase window listeners so we see the pointer BEFORE any
    // UI controls. The `isCastBlocker` check above lets normal clicks
    // through.
    window.addEventListener("pointerdown", onDown, true);
    window.addEventListener("pointermove", onMove, true);
    window.addEventListener("pointerup", onUp, true);
    window.addEventListener("pointercancel", onUp, true);
    window.addEventListener("resize", resize);

    onCleanup(() => {
      window.removeEventListener("pointerdown", onDown, true);
      window.removeEventListener("pointermove", onMove, true);
      window.removeEventListener("pointerup", onUp, true);
      window.removeEventListener("pointercancel", onUp, true);
      window.removeEventListener("resize", resize);
      cancelAnimationFrame(state.raf);
      state.running = false;
    });
  });

  // Keep propsRef pointing at the latest closures so the canvas loop
  // (which captured the initial values in `onMount`) always calls
  // through to fresh handlers.
  return (
    <canvas
      ref={canvasRef}
      aria-hidden="true"
      style={{
        position: "absolute",
        inset: 0,
        width: "100%",
        height: "100%",
        "pointer-events": "none",
        "z-index": 58,
      }}
    />
  );
}

/** Human-readable label for an unbound-glyph message. */
function glyphLabel(g: GlyphId): string {
  switch (g) {
    case "triangle":
      return "△";
    case "invtriangle":
      return "▽";
    case "square":
      return "□";
    case "circle":
      return "○";
  }
}
