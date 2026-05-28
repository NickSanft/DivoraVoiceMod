// Slider — gradient fill with a 14px white thumb. `bipolar` fills from
// the center outward and adds a tick at the midpoint (used by Pitch,
// Formant, EQ — all the signed parameters).

import type { JSX } from "solid-js";

export interface SliderProps {
  value: number;
  min?: number;
  max?: number;
  step?: number;
  onChange?: (next: number) => void;
  disabled?: boolean;
  bipolar?: boolean;
  ariaLabel?: string;
}

export function Slider(props: SliderProps): JSX.Element {
  const min = () => props.min ?? 0;
  const max = () => props.max ?? 100;
  const step = () => props.step ?? 1;
  const pct = () => ((props.value - min()) / (max() - min())) * 100;
  let trackRef: HTMLDivElement | undefined;

  const setFromEvent = (clientX: number) => {
    if (!trackRef || props.disabled) return;
    const r = trackRef.getBoundingClientRect();
    let p = (clientX - r.left) / r.width;
    p = Math.max(0, Math.min(1, p));
    const raw = min() + p * (max() - min());
    const stepped = Math.round(raw / step()) * step();
    const clamped = Math.max(min(), Math.min(max(), stepped));
    props.onChange?.(clamped);
  };

  const onPointerDown = (e: PointerEvent) => {
    if (props.disabled) return;
    setFromEvent(e.clientX);
    const move = (ev: PointerEvent) => setFromEvent(ev.clientX);
    const up = () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up);
  };

  const fillStyle = (): JSX.CSSProperties => {
    const p = pct();
    if (props.bipolar) {
      return {
        left: `${Math.min(50, p)}%`,
        width: `${Math.abs(p - 50)}%`,
      };
    }
    return { width: `${p}%` };
  };

  return (
    <div
      ref={trackRef}
      class={`slider ${props.disabled ? "is-disabled" : ""}`}
      onPointerDown={onPointerDown}
      role="slider"
      aria-valuenow={props.value}
      aria-valuemin={min()}
      aria-valuemax={max()}
      aria-label={props.ariaLabel}
      aria-disabled={props.disabled || undefined}
    >
      <div class="slider-track" />
      <div class="slider-fill" style={fillStyle()} />
      {props.bipolar && <div class="slider-center-tick" style={{ left: "50%" }} />}
      <div class="slider-thumb" style={{ left: `${pct()}%` }} />
    </div>
  );
}
