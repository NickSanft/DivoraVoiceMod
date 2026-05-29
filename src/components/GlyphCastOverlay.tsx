// GlyphCastOverlay — full-screen pointer-capture surface for casting a
// glyph. The user presses, drags out one of the four target shapes
// (▲ ▽ □ ○), and releases — the classifier in `data/glyphs.ts` decides
// which glyph it was, and the overlay calls back with the result.
//
// Visual: a translucent dusk-violet veil over the Mixer with a glowing
// stroke that traces the user's path. While idle (cursor down, no
// drag yet), an instruction blurb floats centred. Esc cancels.

import {
  createMemo,
  createSignal,
  onCleanup,
  onMount,
  Show,
  type JSX,
} from "solid-js";
import { classifyGlyph, type Point } from "../data/glyphs";
import { Sigil } from "./Sigil";
import type { GlyphId } from "../types";

export interface GlyphCastOverlayProps {
  /** Called on release with the classified glyph (null if unrecognised). */
  onClassified: (glyph: GlyphId | null, raw: Point[]) => void;
  /** Called when the user dismisses the overlay (Esc or backdrop click). */
  onCancel: () => void;
}

/**
 * Translate a pointer event's viewport coordinates into the SVG's local
 * coordinate frame. The overlay is `position: absolute` inside the
 * content area (which itself sits below the titlebar and to the right
 * of the sidebar), so `clientX/clientY` are offset from the SVG's
 * origin by exactly the SVG's bounding-rect top-left. Subtracting that
 * keeps the painted trace under the cursor.
 *
 * Exported for unit testing — `localPoint(svg, e)` is pure given the
 * SVG's `getBoundingClientRect()` and the event.
 */
export function localPoint(
  svg: SVGSVGElement | null,
  e: { clientX: number; clientY: number },
): Point {
  if (!svg) return { x: e.clientX, y: e.clientY };
  const rect = svg.getBoundingClientRect();
  return { x: e.clientX - rect.left, y: e.clientY - rect.top };
}

export function GlyphCastOverlay(props: GlyphCastOverlayProps): JSX.Element {
  const [points, setPoints] = createSignal<Point[]>([]);
  const [drawing, setDrawing] = createSignal(false);
  // We track the SVG dimensions so the `viewBox` matches the painted
  // area exactly (not the whole window). That way `localPoint` lands
  // in the SVG's own coord frame.
  const [size, setSize] = createSignal({ w: 1, h: 1 });
  let rootRef: HTMLDivElement | undefined;
  let svgRef: SVGSVGElement | undefined;

  const pathD = createMemo<string>(() => {
    const pts = points();
    if (pts.length === 0) return "";
    const first = pts[0]!;
    let d = `M${first.x} ${first.y}`;
    for (let i = 1; i < pts.length; i++) {
      const p = pts[i]!;
      d += ` L${p.x} ${p.y}`;
    }
    return d;
  });

  const measure = (): void => {
    if (!svgRef) return;
    const rect = svgRef.getBoundingClientRect();
    setSize({ w: Math.max(1, rect.width), h: Math.max(1, rect.height) });
  };

  const onPointerDown = (e: PointerEvent): void => {
    if (e.button !== 0) return;
    e.preventDefault();
    (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
    measure();
    setPoints([localPoint(svgRef ?? null, e)]);
    setDrawing(true);
  };

  const onPointerMove = (e: PointerEvent): void => {
    if (!drawing()) return;
    e.preventDefault();
    setPoints([...points(), localPoint(svgRef ?? null, e)]);
  };

  const onPointerUp = (e: PointerEvent): void => {
    if (!drawing()) return;
    e.preventDefault();
    setDrawing(false);
    try {
      (e.currentTarget as HTMLElement).releasePointerCapture(e.pointerId);
    } catch {
      /* not captured */
    }
    const raw = points();
    setPoints([]);
    const result = classifyGlyph(raw);
    props.onClassified(result, raw);
  };

  onMount(() => {
    measure();
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.preventDefault();
        props.onCancel();
      }
    };
    const onResize = (): void => measure();
    window.addEventListener("keydown", onKey);
    window.addEventListener("resize", onResize);
    onCleanup(() => {
      window.removeEventListener("keydown", onKey);
      window.removeEventListener("resize", onResize);
    });
  });

  return (
    <div
      ref={rootRef}
      role="dialog"
      aria-modal="true"
      aria-label="Cast a glyph"
      style={{
        position: "absolute",
        inset: 0,
        "z-index": 90,
        background: "rgba(13, 10, 22, 0.72)",
        "backdrop-filter": "blur(2px)",
        cursor: "crosshair",
        "touch-action": "none",
      }}
      onPointerDown={onPointerDown}
      onPointerMove={onPointerMove}
      onPointerUp={onPointerUp}
      onPointerCancel={onPointerUp}
    >
      {/* Trace. The viewBox matches the SVG's CSS size so 1 SVG unit
          = 1 CSS pixel — `localPoint()` already maps the pointer event
          into this space. */}
      <svg
        ref={svgRef}
        width="100%"
        height="100%"
        viewBox={`0 0 ${size().w} ${size().h}`}
        style={{
          position: "absolute",
          inset: 0,
          "pointer-events": "none",
        }}
        aria-hidden="true"
      >
        <Show when={pathD()}>
          <path
            d={pathD()}
            fill="none"
            stroke="url(#glyph-cast-grad)"
            stroke-width="3"
            stroke-linecap="round"
            stroke-linejoin="round"
            style={{ filter: "drop-shadow(0 0 8px rgba(124, 92, 246, 0.8))" }}
          />
        </Show>
        <defs>
          <linearGradient id="glyph-cast-grad" x1="0" y1="0" x2="1" y2="1">
            <stop offset="0" stop-color="#7C5CF6" />
            <stop offset="1" stop-color="#EC4899" />
          </linearGradient>
        </defs>
      </svg>

      {/* Idle instructions (only when the user hasn't started drawing) */}
      <Show when={!drawing() && points().length === 0}>
        <div
          style={{
            position: "absolute",
            inset: 0,
            display: "grid",
            "place-items": "center",
            "pointer-events": "none",
          }}
        >
          <div
            style={{
              "max-width": "420px",
              padding: "var(--s5) var(--s6)",
              "border-radius": "var(--r-lg)",
              background: "var(--surface-2)",
              border: "1px solid var(--line-glow)",
              "text-align": "center",
              "box-shadow": "var(--shadow-3)",
            }}
          >
            <div
              style={{
                display: "flex",
                "justify-content": "center",
                gap: "var(--s4)",
                "margin-bottom": "var(--s3)",
                color: "var(--indigo)",
              }}
            >
              <Sigil name="bolt" size={24} />
            </div>
            <div
              class="display"
              style={{
                "font-size": "var(--t-h2)",
                "margin-bottom": "var(--s2)",
              }}
            >
              Cast a glyph
            </div>
            <div
              style={{
                "font-size": "var(--t-sm)",
                color: "var(--text-mid)",
                "line-height": 1.5,
              }}
            >
              Press and drag to trace one of: triangle, inverted triangle,
              square, or circle. Releases switches to the bound preset.
              Press <span class="kbd">Esc</span> to cancel.
            </div>
          </div>
        </div>
      </Show>
    </div>
  );
}
