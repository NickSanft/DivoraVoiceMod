// v1.15.0: record-a-glyph modal + a tiny preview for custom glyphs.
//
// The user draws a single stroke on the pad; on save we normalize it into
// a `makeTemplate` template (the same normalization the recognizer uses)
// and hand it back to bind to an action.

import { createSignal, For, Show, type JSX } from "solid-js";
import { Button } from "./Button";
import { makeTemplate } from "../data/unistroke";
import type { Point } from "../data/glyphs";

const PAD = 220;

/** Render a normalized template (points ~[-0.5, 0.5]) as an SVG polyline. */
export function GlyphPreview(props: {
  template: { x: number; y: number }[];
  size?: number;
}): JSX.Element {
  const s = (): number => props.size ?? 34;
  const inset = 0.82; // keep the stroke off the edges
  const pts = (): string =>
    props.template
      .map((p) => {
        const x = (p.x * inset + 0.5) * s();
        const y = (p.y * inset + 0.5) * s();
        return `${x.toFixed(1)},${y.toFixed(1)}`;
      })
      .join(" ");
  return (
    <svg width={s()} height={s()} viewBox={`0 0 ${s()} ${s()}`} style={{ display: "block" }}>
      <polyline
        points={pts()}
        fill="none"
        stroke="currentColor"
        stroke-width="1.6"
        stroke-linecap="round"
        stroke-linejoin="round"
      />
    </svg>
  );
}

export function GlyphRecorder(props: {
  open: boolean;
  onClose: () => void;
  onSave: (name: string, template: Point[]) => void;
}): JSX.Element {
  const [raw, setRaw] = createSignal<Point[]>([]);
  const [name, setName] = createSignal("");
  let drawing = false;
  let padRef: HTMLDivElement | undefined;

  const close = (): void => {
    setRaw([]);
    setName("");
    props.onClose();
  };
  const toLocal = (e: PointerEvent): Point => {
    const r = padRef!.getBoundingClientRect();
    return { x: e.clientX - r.left, y: e.clientY - r.top };
  };
  const down = (e: PointerEvent): void => {
    e.preventDefault();
    drawing = true;
    setRaw([toLocal(e)]);
  };
  const move = (e: PointerEvent): void => {
    if (!drawing) return;
    setRaw([...raw(), toLocal(e)]);
  };
  const up = (): void => {
    drawing = false;
  };

  const canSave = (): boolean => raw().length > 8 && name().trim().length > 0;

  return (
    <Show when={props.open}>
      <div
        role="dialog"
        aria-modal="true"
        aria-label="Record a glyph"
        style={{
          position: "fixed",
          inset: 0,
          background: "var(--scrim)",
          "backdrop-filter": "blur(4px)",
          display: "grid",
          "place-items": "center",
          "z-index": 200,
        }}
      >
        <div
          class="panel"
          style={{
            padding: "var(--s5)",
            display: "flex",
            "flex-direction": "column",
            gap: "var(--s4)",
            width: "300px",
          }}
        >
          <div class="display" style={{ "font-size": "var(--t-h3)" }}>
            Record a glyph
          </div>
          <div style={{ "font-size": "var(--t-xs)", color: "var(--text-lo)", "line-height": 1.5 }}>
            Draw a single stroke. Cast it later by drawing it the same way.
          </div>
          <div
            ref={padRef}
            onPointerDown={down}
            onPointerMove={move}
            onPointerUp={up}
            style={{
              width: `${PAD}px`,
              height: `${PAD}px`,
              "align-self": "center",
              "border-radius": "var(--r-md)",
              border: "1px dashed var(--line-strong)",
              background: "var(--surface-2)",
              "touch-action": "none",
              position: "relative",
              display: "grid",
              "place-items": "center",
            }}
          >
            <Show
              when={raw().length > 1}
              fallback={
                <span style={{ "font-size": "var(--t-xs)", color: "var(--text-lo)" }}>
                  draw here
                </span>
              }
            >
              <svg
                width={PAD}
                height={PAD}
                style={{ position: "absolute", inset: 0, "pointer-events": "none" }}
              >
                <For each={[raw()]}>
                  {(pts) => (
                    <polyline
                      points={pts.map((p) => `${p.x},${p.y}`).join(" ")}
                      fill="none"
                      stroke="var(--indigo)"
                      stroke-width="2.5"
                      stroke-linecap="round"
                      stroke-linejoin="round"
                    />
                  )}
                </For>
              </svg>
            </Show>
          </div>
          <input
            value={name()}
            onInput={(e) => setName(e.currentTarget.value)}
            placeholder="Name this glyph"
            style={{
              height: "38px",
              "border-radius": "var(--r-md)",
              border: "1px solid var(--line-strong)",
              background: "var(--surface-2)",
              color: "var(--text-hi)",
              padding: "0 12px",
              "font-family": "var(--font-ui)",
              "font-size": "var(--t-body)",
            }}
          />
          <div style={{ display: "flex", gap: "var(--s2)", "justify-content": "flex-end" }}>
            <Button variant="ghost" size="sm" onClick={() => setRaw([])} disabled={raw().length === 0}>
              Redraw
            </Button>
            <Button variant="ghost" size="sm" onClick={close}>
              Cancel
            </Button>
            <Button
              variant="primary"
              size="sm"
              disabled={!canSave()}
              onClick={() => {
                props.onSave(name().trim(), makeTemplate(raw()));
                close();
              }}
            >
              Save
            </Button>
          </div>
        </div>
      </div>
    </Show>
  );
}
