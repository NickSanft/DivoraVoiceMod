// Vertical and horizontal level meters. Both fill emerald→gold→crimson
// bottom-to-top (or left-to-right) and carry a white peak-hold cap.
//
// Phase 1 wires the visual; Phase 2 replaces the level/peak inputs with
// real RMS/peak from the Rust audio engine.

import type { JSX } from "solid-js";
import { For, Show } from "solid-js";

const DEFAULT_TICKS = [0.25, 0.5, 0.7, 0.85];

export interface VMeterProps {
  level: number;
  peak?: number;
  height?: number;
  label?: string;
}

export function VMeter(props: VMeterProps): JSX.Element {
  const height = () => props.height ?? 200;
  const peak = () => props.peak ?? 0;
  return (
    <div class="vmeter">
      <div class="vmeter-col" style={{ height: `${height()}px` }}>
        <div class="vmeter-ticks">
          <For each={DEFAULT_TICKS}>
            {(t) => <div class="vmeter-tick" style={{ bottom: `${t * 100}%` }} />}
          </For>
        </div>
        <div class="vmeter-level" style={{ height: `${props.level * 100}%` }} />
        <Show when={peak() > 0.02}>
          <div class="vmeter-peak" style={{ bottom: `calc(${peak() * 100}% - 1px)` }} />
        </Show>
      </div>
      <Show when={props.label}>
        <div class="eyebrow" style={{ "letter-spacing": "0.16em" }}>
          {props.label}
        </div>
      </Show>
    </div>
  );
}

export interface HMeterProps {
  level: number;
  peak?: number;
}

export function HMeter(props: HMeterProps): JSX.Element {
  const peak = () => props.peak ?? 0;
  return (
    <div class="hmeter">
      <div class="hmeter-level" style={{ width: `${props.level * 100}%` }} />
      <Show when={peak() > 0.02}>
        <div class="hmeter-peak" style={{ left: `calc(${peak() * 100}% - 1px)` }} />
      </Show>
    </div>
  );
}
