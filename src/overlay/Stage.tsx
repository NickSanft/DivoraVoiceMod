// v1.55.0: the stream overlay's stage — the charms, and arranging them.
//
// Draws the scene the overlay page hands it, laid out in the window's size,
// and in Arrange turns presses and keys into edits through editor.ts. Rows
// are keyed by element id, so a drag, an ack, a Settings change or a resize
// restyles a charm and never remounts it: the spell circle keeps animating,
// a picture never reloads and a focused charm keeps focus.
//
// Locked, the window is click-through, so no press arrives; the keys that
// still might are blocked (shouldPreventKey). In Arrange the chrome's beats
// keep the backend's lease (D21): no beat, and the backend locks again.
//
// v1.59.0: a third kind of charm, an Hourglass timer's face: the text the
// backend formats, in white Space Mono Bold with a dark outline, sized in
// JS to fit its box. Nothing about it is animated by CSS, so Functional
// motion can't freeze it; its text node changes only when the text does.

import {
  batch,
  createEffect,
  createMemo,
  createSignal,
  For,
  on,
  onCleanup,
  onMount,
  Show,
  type JSX,
} from "solid-js";
import { Sigil } from "../components/Sigil";
import { SpellCircle, SPELL_CIRCLE_PX } from "../components/SpellCircle";
import { isPlainLeftPress, startsWindowDrag } from "./drag";
import {
  beginMove,
  beginResize,
  finishGesture,
  keyAction,
  NUDGE_FLUSH_MS,
  shouldPreventKey,
  updateGesture,
  type Gesture,
} from "./editor";
import {
  aspectOf,
  charmLabels,
  charmUrl,
  handleRects,
  hitRect,
  layoutScene,
  liveElements,
  moveRect,
  scaleRect,
  type Corner,
  type OverlayScene,
  type PxRect,
  type SceneOp,
  type Size,
} from "./scene";
import { OVERLAY_BG_COLOR, type OverlayBg, type OverlayState } from "./state";
import { requestLock, sendAlive, type OverlayStatus } from "./status";
import { fitTimerFont, type TimerSnapshot, type TimerState } from "./timerState";

/** How often an arranging page tells the backend it is alive. */
export const BEAT_MS = 500;

/** Above every charm. */
const CHROME_Z = "100000";

const CORNERS: readonly Corner[] = ["nw", "ne", "sw", "se"];

export interface StageProps {
  /** This window's number (the page's `?instance=`), echoed in each beat. */
  instance: number;
  /** The circle's look. The circle isn't drawn until it arrives. */
  state: OverlayState | null;
  /** What to draw; null until main's first push. */
  scene: OverlayScene | null;
  status: OverlayStatus;
  /** The window's client size in CSS px. */
  viewport: Size;
  /** What sits behind the charms. */
  background: OverlayBg | undefined;
  /** v1.59.0: the backend's latest timers; null until its first snapshot,
   *  and no timer face draws before one. */
  timers: TimerSnapshot | null;
  /** v1.59.0: the face's font has loaded, or the page stopped waiting. */
  fontReady: boolean;
  /** One finished edit: a drag, a nudge run, Delete or a restack. */
  onEdit: (op: SceneOp) => void;
}

function px(n: number): string {
  return `${n}px`;
}

function sameStrings(a: string[], b: string[]): boolean {
  return a.length === b.length && a.every((x, i) => x === b[i]);
}

function sameNames(a: Map<string, string>, b: Map<string, string>): boolean {
  if (a.size !== b.size) return false;
  for (const [k, v] of a) if (b.get(k) !== v) return false;
  return true;
}

/** Run `cb` two animation frames from now; returns a cancel. */
export function twoFrames(cb: () => void): () => void {
  let id = requestAnimationFrame(() => {
    id = requestAnimationFrame(cb);
  });
  return () => cancelAnimationFrame(id);
}

/** How the spell circle's SPELL_CIRCLE_PX box fits a charm's rect: scaled
 *  to the largest square in it, centred. The layout already squares the
 *  circle; this keeps a rect a hair off square from stretching it. */
export function circleFit(r: { w: number; h: number }): { k: number; x: number; y: number } {
  const k = Math.min(r.w, r.h) / SPELL_CIRCLE_PX;
  return { k, x: (r.w - SPELL_CIRCLE_PX * k) / 2, y: (r.h - SPELL_CIRCLE_PX * k) / 2 };
}

/** A pointer's id; jsdom's stand-in events have none. */
function pointerOf(e: PointerEvent): number {
  return typeof e.pointerId === "number" ? e.pointerId : 0;
}

async function dragWindow(): Promise<void> {
  try {
    const { getCurrentWindow } = await import("@tauri-apps/api/window");
    await getCurrentWindow().startDragging();
  } catch {
    /* not under Tauri */
  }
}

const PILL: JSX.CSSProperties = {
  position: "fixed",
  top: "6px",
  display: "inline-flex",
  "align-items": "center",
  gap: "6px",
  height: "28px",
  "max-height": "28px",
  padding: "0 8px",
  "border-radius": "var(--r-pill)",
  border: "1px solid var(--indigo)",
  background: "var(--surface-2)",
  color: "var(--text-hi)",
  "font-size": "var(--t-sm)",
  "line-height": "1",
  "white-space": "nowrap",
  "box-shadow": "0 1px 6px rgba(0, 0, 0, 0.5)",
  "pointer-events": "auto",
};

/** A charm's Arrange-only "missing" badge, over the charm. */
const MISSING: JSX.CSSProperties = {
  position: "absolute",
  inset: "0",
  display: "flex",
  "align-items": "center",
  "justify-content": "center",
  gap: "4px",
  overflow: "hidden",
  color: "var(--warning)",
  background: "var(--scrim)",
  "font-size": "var(--t-xs)",
  "white-space": "nowrap",
  "pointer-events": "none",
};

/** v1.59.0: a timer's face, less its font size. White with a dark outline
 *  reads over any game and isn't keyed out by the green or magenta
 *  backgrounds. No animation and no transition: only JS changes it. */
const TIMER_FACE: JSX.CSSProperties = {
  position: "absolute",
  inset: "0",
  display: "flex",
  "align-items": "center",
  "justify-content": "center",
  "font-family": "var(--font-mono)",
  "font-weight": "700",
  "line-height": "1",
  color: "#fff",
  "text-shadow": "0 0 0.06em #000, 0 0 0.12em rgba(0, 0, 0, 0.85), 0 0.04em 0.1em rgba(0, 0, 0, 0.6)",
  "white-space": "nowrap",
  "pointer-events": "none",
};

/** The Arrange lease's beats: two frames after the chrome mounts, then
 *  every BEAT_MS until it unmounts (a lock, or the page going away). */
function Beats(props: { instance: number; gen: () => number }): JSX.Element {
  let stopFrames: (() => void) | undefined;
  let timer: ReturnType<typeof setInterval> | undefined;
  onMount(() => {
    const beat = (): void => void sendAlive(props.instance, props.gen());
    stopFrames = twoFrames(() => {
      beat();
      timer = setInterval(beat, BEAT_MS);
    });
  });
  onCleanup(() => {
    stopFrames?.();
    clearInterval(timer);
  });
  return null;
}

export function Stage(props: StageProps): JSX.Element {
  let stage: HTMLDivElement | undefined;
  // A memo: every status push is a new object, and only a change of mode
  // may move the keyboard (a capture or Fit change mid-arrange must not).
  const arrange = createMemo(() => props.status.mode === "arrange");

  const layout = createMemo(() => (props.scene ? layoutScene(props.scene, props.viewport) : []));
  const byId = createMemo(() => new Map(layout().map((r) => [r.el.id, r])));
  // Rows in scene-array order, which a restack never changes: `[` and `]`
  // only change z-index, so no row's node moves.
  const ids = createMemo(
    () => (props.scene ? liveElements(props.scene).map((e) => e.id).filter((id) => byId().has(id)) : []),
    [],
    { equals: sameStrings },
  );
  const rank = createMemo(() => new Map(layout().map((r, i) => [r.el.id, i])));
  // Each timer's name by id. Changes only when a name does, so a running
  // timer's snapshot every second relabels nothing.
  const timerNames = createMemo(
    () => new Map((props.timers?.timers ?? []).map((t) => [t.id, t.name])),
    new Map<string, string>(),
    { equals: sameNames },
  );
  const labels = createMemo(() =>
    props.scene ? charmLabels(props.scene, (timer) => timerNames().get(timer)) : new Map<string, string>(),
  );

  const [selected, setSelected] = createSignal<string | null>(null);
  const [gesture, setGesture] = createSignal<Gesture | null>(null);
  // An arrow or +/− run, kept as a move gesture so it finishes the same way:
  // one `place`, NUDGE_FLUSH_MS after its last key.
  const [nudge, setNudge] = createSignal<Gesture | null>(null);
  const [hasFocus, setHasFocus] = createSignal(document.hasFocus());
  let nudgeTimer: ReturnType<typeof setTimeout> | undefined;

  /** The selection, while that charm is still on the stage. */
  const current = (): string | null => {
    const id = selected();
    return id !== null && byId().has(id) ? id : null;
  };

  const flushNudge = (): void => {
    clearTimeout(nudgeTimer);
    nudgeTimer = undefined;
    const n = nudge();
    if (!n) return;
    const op = finishGesture(n);
    batch(() => {
      setNudge(null);
      if (op) props.onEdit(op);
    });
  };

  const select = (id: string | null): void => {
    if (selected() === id) return;
    flushNudge();
    setSelected(id);
  };

  // ---- pointer gestures (window listeners, as Slider does) ----

  const unlisten = (): void => {
    window.removeEventListener("pointermove", onMove);
    window.removeEventListener("pointerup", onUp);
    window.removeEventListener("pointercancel", onLost);
    window.removeEventListener("lostpointercapture", onLost);
  };

  const drop = (): void => {
    unlisten();
    setGesture(null);
  };

  const finish = (): void => {
    const g = gesture();
    unlisten();
    if (!g) return;
    const op = finishGesture(g);
    batch(() => {
      setGesture(null);
      if (op) props.onEdit(op);
    });
  };

  function onMove(e: PointerEvent): void {
    const g = gesture();
    if (!g || pointerOf(e) !== g.pointerId) return;
    // The release happened where we couldn't see it.
    if ((e.buttons & 1) === 0) return finish();
    // Removed from Settings mid-drag.
    if (!byId().has(g.id)) return drop();
    setGesture(updateGesture(g, { x: e.clientX, y: e.clientY }));
  }

  function onUp(e: PointerEvent): void {
    const g = gesture();
    if (g && pointerOf(e) === g.pointerId) finish();
  }

  function onLost(e: PointerEvent): void {
    const g = gesture();
    if (g && pointerOf(e) === g.pointerId) drop();
  }

  const press = (id: string, e: PointerEvent, row: HTMLElement): void => {
    if (!arrange() || e.button !== 0) return;
    // First, so the gesture starts from the nudged rect and no timer fires
    // after its release.
    flushNudge();
    const r = byId().get(id);
    if (!r) return;
    // Also stops the compatibility mousedown, so the window never drags.
    e.preventDefault();
    e.stopPropagation();
    select(id);
    row.focus({ preventScroll: true });
    try {
      row.setPointerCapture?.(pointerOf(e));
    } catch {
      /* the pointer is already gone */
    }
    drop();
    const handle = e.target instanceof Element ? e.target.closest("[data-handle]") : null;
    const corner = handle?.getAttribute("data-corner") as Corner | null | undefined;
    const at = { x: e.clientX, y: e.clientY };
    const v = { w: props.viewport.w, h: props.viewport.h };
    setGesture(
      corner && CORNERS.includes(corner)
        ? beginResize(id, corner, pointerOf(e), at, r.rect, v)
        : beginMove(id, pointerOf(e), at, r.rect, v),
    );
    window.addEventListener("pointermove", onMove);
    window.addEventListener("pointerup", onUp);
    window.addEventListener("pointercancel", onLost);
    window.addEventListener("lostpointercapture", onLost);
  };

  // A press that reached the stage itself: nothing there to select.
  const pressStage = (e: PointerEvent): void => {
    if (!arrange()) return;
    if (e.target instanceof Element && e.target.closest("[data-chrome]")) return;
    select(null);
  };

  // Empty stage moves the window, only in Arrange and only when not fitted.
  const pressStageMouse = (e: MouseEvent): void => {
    if (!arrange() || props.status.fit || !startsWindowDrag(e)) return;
    e.preventDefault();
    void dragWindow();
  };

  const pressGrip = (e: MouseEvent): void => {
    if (!isPlainLeftPress(e)) return;
    e.preventDefault();
    e.stopPropagation();
    void dragWindow();
  };

  // ---- keys ----

  /** Start or extend the nudge run on `id`, in the viewport it began in. */
  const stepNudge = (id: string, next: (r: PxRect, v: Size) => PxRect): void => {
    let n = nudge();
    if (n && n.id !== id) {
      flushNudge();
      n = null;
    }
    if (!n) {
      const r = byId().get(id);
      if (!r) return;
      // No pointer: -1 matches none.
      n = beginMove(id, -1, { x: 0, y: 0 }, r.rect, { w: props.viewport.w, h: props.viewport.h });
    }
    setNudge({ ...n, preview: next(n.preview, n.v0) });
    clearTimeout(nudgeTimer);
    nudgeTimer = setTimeout(flushNudge, NUDGE_FLUSH_MS);
  };

  const onKey = (e: KeyboardEvent): void => {
    const onButton = e.target instanceof Element && e.target.closest("button") !== null;
    if (shouldPreventKey(e, onButton)) e.preventDefault();
    const id = current();
    const act = keyAction(e, { arrange: arrange(), gesture: gesture() !== null, selected: id !== null });
    if (act.kind === "none") return;
    // A pending nudge goes first, so the edits keep the order of the keys.
    if (act.kind !== "nudge" && act.kind !== "scale") flushNudge();
    if (act.kind === "lock") return void requestLock();
    if (act.kind === "cancel") return drop();
    if (id === null) return;
    switch (act.kind) {
      case "nudge":
        stepNudge(id, (r, v) => moveRect(r, act.dx, act.dy, v));
        break;
      case "scale": {
        const el = byId().get(id)?.el;
        const design = props.scene?.designSize;
        if (!el || !design) return;
        const aspect = aspectOf(el, design);
        stepNudge(id, (r, v) => scaleRect(r, act.dir, act.big, aspect, v));
        break;
      }
      case "remove":
        props.onEdit({ kind: "remove", id });
        break;
      case "order":
        props.onEdit({ kind: "order", id, to: act.to });
        break;
    }
  };

  const onContextMenu = (e: Event): void => e.preventDefault();
  const onFocus = (): void => void setHasFocus(true);
  const onBlur = (): void => {
    setHasFocus(false);
    flushNudge();
  };

  onMount(() => {
    window.addEventListener("keydown", onKey);
    window.addEventListener("contextmenu", onContextMenu);
    window.addEventListener("focus", onFocus);
    window.addEventListener("blur", onBlur);
  });
  onCleanup(() => {
    window.removeEventListener("keydown", onKey);
    window.removeEventListener("contextmenu", onContextMenu);
    window.removeEventListener("focus", onFocus);
    window.removeEventListener("blur", onBlur);
    unlisten();
    clearTimeout(nudgeTimer);
  });

  // Entering Arrange takes the keyboard to the stage; locking ends whatever
  // was in hand.
  createEffect(
    on(arrange, (arranging) => {
      if (arranging) {
        setHasFocus(document.hasFocus());
        stage?.focus({ preventScroll: true });
        return;
      }
      flushNudge();
      drop();
      setSelected(null);
    }),
  );

  // ---- one charm ----

  function Row(p: { id: string }): JSX.Element {
    const id = p.id;
    let row: HTMLDivElement | undefined;
    const entry = () => byId().get(id);
    const kind = () => entry()?.el.type;
    const circle = () => kind() === "spellcircle";
    const rect = (): PxRect | undefined => {
      const g = gesture();
      if (g && g.id === id) return g.preview;
      const n = nudge();
      if (n && n.id === id) return n.preview;
      return entry()?.rect;
    };
    // The hit box, relative to the row.
    const hit = (): PxRect | undefined => {
      const r = rect();
      if (!r) return undefined;
      const h = hitRect(r, props.viewport);
      return { x: h.x - r.x, y: h.y - r.y, w: h.w, h: h.h };
    };
    const isSelected = () => arrange() && current() === id;
    const label = () => {
      if (kind() === "timer") return `Timer: ${labels().get(id) ?? "Timer"}`;
      const name = labels().get(id) ?? "Picture";
      return circle() ? name : `Picture: ${name}`;
    };
    const src = () => {
      const el = entry()?.el;
      return el && el.type === "image" ? charmUrl(el.src) : "";
    };
    const [missing, setMissing] = createSignal(false);
    createEffect(on(src, () => setMissing(false), { defer: true }));

    // v1.59.0: a timer face's timer, in the latest snapshot.
    const timerOf = (): TimerState | undefined => {
      const el = entry()?.el;
      return el && el.type === "timer" ? props.timers?.timers.find((t) => t.id === el.timer) : undefined;
    };
    // A memo, so the text node is written only when the text changes.
    const text = createMemo(() => timerOf()?.text ?? "");
    const showFace = () => props.fontReady && timerOf() !== undefined;
    // Only once the backend has answered: before that, every timer is
    // "missing", and Arrange is visible to recordings.
    const timerMissing = () => arrange() && props.timers !== null && timerOf() === undefined;
    const faceStyle = (): JSX.CSSProperties => {
      const r = rect();
      return { ...TIMER_FACE, "font-size": px(r ? fitTimerFont(r.w, r.h, text().length) : 0) };
    };

    const box = (): JSX.CSSProperties => {
      const r = rect();
      if (!r) return { display: "none" };
      return {
        position: "absolute",
        left: px(r.x),
        top: px(r.y),
        width: px(r.w),
        height: px(r.h),
        "z-index": String(rank().get(id) ?? 0),
        "pointer-events": "none",
        "touch-action": "none",
        outline: "none",
      };
    };
    const at = (h: PxRect | undefined): JSX.CSSProperties =>
      h ? { position: "absolute", left: px(h.x), top: px(h.y), width: px(h.w), height: px(h.h) } : { display: "none" };
    // The circle draws at its own size, scaled by transform.
    const circleBox = (): JSX.CSSProperties => {
      const r = rect();
      const fit = r ? circleFit(r) : { k: 1, x: 0, y: 0 };
      return {
        position: "absolute",
        left: "0",
        top: "0",
        width: px(SPELL_CIRCLE_PX),
        height: px(SPELL_CIRCLE_PX),
        transform: `translate(${fit.x}px, ${fit.y}px) scale(${fit.k})`,
        "transform-origin": "0 0",
        "pointer-events": "none",
      };
    };
    const handleStyle = (c: Corner): JSX.CSSProperties => {
      const h = hit();
      if (!h) return { display: "none" };
      const r = handleRects(h)[c];
      return {
        ...at(r),
        "pointer-events": "auto",
        cursor: c === "nw" || c === "se" ? "nwse-resize" : "nesw-resize",
        background: "var(--indigo)",
        border: "1px solid var(--text-hi)",
        "border-radius": "2px",
      };
    };

    return (
      <div
        ref={row}
        data-charm
        data-id={id}
        role="img"
        aria-label={label()}
        tabIndex={arrange() ? 0 : -1}
        style={box()}
        onPointerDown={(e) => press(id, e, row!)}
        onFocus={() => {
          if (arrange()) select(id);
        }}
      >
        <Show when={circle() && props.state}>
          <div data-circle style={circleBox()}>
            <SpellCircle
              decorative
              chain={props.state!.chain}
              status={props.state!.status}
              motion={props.state!.motion}
              mystical={props.state!.mystical}
              selected={null}
              onSelect={() => {}}
              onToggle={() => {}}
            />
          </div>
        </Show>
        <Show when={kind() === "image"}>
          <img
            src={src()}
            alt=""
            draggable={false}
            style={{
              display: "block",
              width: "100%",
              height: "100%",
              "object-fit": "contain",
              "pointer-events": "none",
              "-webkit-user-drag": "none",
            }}
            onError={() => setMissing(true)}
            onLoad={() => setMissing(false)}
          />
          <Show when={missing() && arrange()}>
            <div data-missing style={MISSING}>
              <Sigil name="image" size={14} />
              <span>Picture missing</span>
            </div>
          </Show>
        </Show>
        <Show when={kind() === "timer"}>
          <Show
            when={showFace()}
            fallback={
              <Show when={timerMissing()}>
                <div data-missing style={MISSING}>
                  <Sigil name="hourglass" size={14} />
                  <span>Timer missing</span>
                </div>
              </Show>
            }
          >
            <div data-timer-face aria-hidden="true" style={faceStyle()}>
              {text()}
            </div>
          </Show>
        </Show>
        <div
          data-hit
          style={{
            ...at(hit()),
            "pointer-events": "auto",
            "border-radius": circle() ? "50%" : undefined,
            cursor: arrange() ? "move" : undefined,
          }}
        />
        <Show when={arrange()}>
          <div
            data-outline
            aria-hidden="true"
            style={{
              ...at(hit()),
              "pointer-events": "none",
              border: isSelected() ? "2px dashed var(--indigo)" : "1px dashed var(--line-glow)",
              "border-radius": circle() ? "50%" : undefined,
            }}
          />
        </Show>
        <Show when={isSelected()}>
          <For each={CORNERS}>
            {(c) => <div data-handle data-corner={c} aria-hidden="true" style={handleStyle(c)} />}
          </For>
        </Show>
      </div>
    );
  }

  const background = (): string => (props.background && OVERLAY_BG_COLOR[props.background]) || "transparent";

  return (
    <div
      ref={stage}
      data-stage
      tabIndex={-1}
      style={{
        position: "fixed",
        inset: "0",
        overflow: "hidden",
        outline: "none",
        background: background(),
        "user-select": "none",
        "-webkit-user-select": "none",
      }}
      onPointerDown={pressStage}
      onMouseDown={pressStageMouse}
    >
      <For each={ids()}>{(id) => <Row id={id} />}</For>
      <Show when={arrange()}>
        <div
          aria-hidden="true"
          style={{
            position: "fixed",
            inset: "0",
            border: "2px dashed var(--indigo)",
            "box-shadow": "inset 0 0 0 1px rgba(0, 0, 0, 0.67)",
            "pointer-events": "none",
            "z-index": CHROME_Z,
          }}
        />
        <div
          data-chrome
          role="toolbar"
          aria-label="Arrange"
          style={{ position: "fixed", inset: "0", "pointer-events": "none", "z-index": CHROME_Z }}
        >
          <button
            type="button"
            data-lock
            aria-label="Lock the overlay"
            onClick={() => void requestLock()}
            style={{ ...PILL, right: "6px" }}
          >
            <Sigil name="lock" size={16} />
            <Show when={props.viewport.w >= 160}>
              <span>Lock</span>
            </Show>
          </button>
          <Show when={!props.status.fit}>
            <div
              data-grip
              aria-label="Move window"
              onMouseDown={pressGrip}
              style={{ ...PILL, left: "6px", cursor: "move" }}
            >
              <Sigil name="drag" size={16} />
              <Show when={props.viewport.w >= 320}>
                <span>Move window</span>
              </Show>
            </div>
          </Show>
          <Show when={props.viewport.w >= 320 && props.viewport.h >= 120}>
            <div
              data-hints
              style={{
                position: "fixed",
                top: "40px",
                left: "50%",
                transform: "translateX(-50%)",
                "max-width": "calc(100vw - 24px)",
                display: "flex",
                "flex-direction": "column",
                gap: "2px",
                padding: "4px 8px",
                "border-radius": "var(--r-sm)",
                background: "var(--scrim)",
                color: "var(--text-hi)",
                "font-size": "var(--t-xs)",
                "text-align": "center",
                "pointer-events": "none",
              }}
            >
              <span>{hasFocus() ? "Esc locks the overlay" : "Click Lock to lock the overlay"}</span>
              {/* Only where Locked would hide it: Hide on, and Windows able to. */}
              <Show when={props.status.capturing && props.status.hideWanted && props.status.hideAvailable}>
                <span>Recordings can see the overlay while you arrange.</span>
              </Show>
            </div>
          </Show>
          <Beats instance={props.instance} gen={() => props.status.arrangeGen} />
        </div>
      </Show>
    </div>
  );
}
