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
import type { GlyphId, Preset } from "../types";

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

interface Omen {
  shape: DetectedShape;
  /** Brand color from the cast preset. */
  color: string;
  /** Preset display name (drawn under the shape). */
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
  /** Look up the preset bound to a glyph; return null if unbound. */
  resolvePreset: (glyph: GlyphId) => Preset | null;
  /** Apply the cast — switch chain/preset. Called when a shape is
   *  recognised AND it's bound to a preset. */
  onCast: (preset: Preset) => void;
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
  const propsRef = { resolvePreset: props.resolvePreset, onCast: props.onCast, onMessage: props.onMessage };

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
      const half =
        Math.min(150, Math.max(95, o.shape.size * 0.5)) * pop;

      ctx.save();
      ctx.strokeStyle = o.color;
      ctx.shadowColor = o.color;
      ctx.shadowBlur = 22;
      ctx.lineWidth = 3.2;
      ctx.lineJoin = "round";

      // Expanding halo ring behind the shape (subtle, low alpha).
      ctx.globalAlpha = a * 0.32;
      ctx.beginPath();
      ctx.arc(
        o.shape.cx,
        o.shape.cy,
        half * (1.12 + (age / OMEN_LIFE_MS) * 0.55),
        0,
        Math.PI * 2,
      );
      ctx.stroke();

      // The shape outline itself.
      ctx.globalAlpha = a;
      drawShapeOutline(o.shape, half);

      // Labels — preset name + SPELL CAST eyebrow.
      ctx.shadowBlur = 14;
      ctx.fillStyle = o.color;
      ctx.textAlign = "center";
      ctx.textBaseline = "middle";
      ctx.font = '700 19px "Bricolage Grotesque", system-ui, sans-serif';
      ctx.fillText(o.name, o.shape.cx, o.shape.cy + half + 28);
      ctx.shadowBlur = 0;
      ctx.fillStyle = o.eyebrow;
      ctx.globalAlpha = a * 0.65;
      ctx.font = '700 10px "Space Mono", monospace';
      ctx.fillText("◆  S P E L L   C A S T  ◆", o.shape.cx, o.shape.cy + half + 47);
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

    const conjure = (shape: DetectedShape): void => {
      const preset = propsRef.resolvePreset(shape.type);
      if (!preset) {
        propsRef.onMessage(`No preset bound to ${glyphLabel(shape.type)}`);
        return;
      }
      const colors = [preset.color, "#FFFFFF", preset.color, "#C9B8FF"];
      // Trace sparks along the recognised outline so the user sees the
      // app "complete" their stroke into the canonical shape.
      if (shape.corners) {
        const c = shape.corners;
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
      } else if (shape.r) {
        for (let i = 0; i < 28; i++) {
          const ang = (i / 28) * Math.PI * 2;
          spawn(
            shape.cx + shape.r * Math.cos(ang),
            shape.cy + shape.r * Math.sin(ang),
            2,
            { colors, size: 2.2 },
          );
        }
      }
      // Central burst.
      spawn(shape.cx, shape.cy, 110, { colors, burst: 6.5, size: 2.6 });
      state.omen = {
        shape,
        color: preset.color,
        name: preset.name,
        eyebrow: readToken("--text-hi", "rgba(233,231,248,0.85)"),
        start: performance.now(),
      };
      ensure();
      propsRef.onCast(preset);
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
      const shape = detectShape(state.path);
      if (shape) {
        conjure(shape);
      } else if (state.path.length > 4) {
        // The user drew something but it didn't classify — surface a
        // brief hint so they understand the gesture happened.
        propsRef.onMessage("Glyph not recognised — try again");
      }
      state.path = [];
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
