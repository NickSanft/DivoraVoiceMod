// SpellCircle — the Mixer's signature visualization.
//
// Voice core in the center; effects orbit the core; threads of light
// connect the core to enabled effects. Outer decorative ring with
// runic ticks, rotating orbit dashes, an orbiting spark, pulse rings
// emanating from the core, and particles drifting up when modulated.
//
// Ported from docs/mockups/prototype/divora/spell_circle.jsx with the
// same SVG geometry; React hooks become Solid signals/memos.

import { createMemo, For, Show, type JSX } from "solid-js";
import { EFFECTS } from "../data/effects";
import type { ChainEntry, EffectId, VoiceStatus } from "../types";
import { Sigil, type SigilName } from "./Sigil";

const S = 440;
const C = S / 2;
const NODE_R = 158;
const CORE_R = 66;

export interface SpellCircleProps {
  chain: ChainEntry[];
  status: VoiceStatus;
  /** 0 = no animation, 0.6 = ambient, 1 = rich. */
  motion: number;
  /** 0 = subtle, 0.5 = balanced, 1 = rich. */
  mystical: number;
  selected: EffectId | null;
  onSelect: (id: EffectId) => void;
  onToggle: (id: EffectId) => void;
}

interface Node extends ChainEntry {
  x: number;
  y: number;
  angle: number;
}

export function SpellCircle(props: SpellCircleProps): JSX.Element {
  const active = () => props.status === "modulated";
  const muted = () => props.status === "muted";
  const coreColor = () =>
    muted() ? "#F2567A" : active() ? "#7C5CF6" : "#6E6590";

  const nodes = createMemo<Node[]>(() => {
    const n = props.chain.length;
    if (n === 0) return [];
    return props.chain.map((c, i) => {
      const a = ((-90 + i * (360 / n)) * Math.PI) / 180;
      return {
        ...c,
        x: C + NODE_R * Math.cos(a),
        y: C + NODE_R * Math.sin(a),
        angle: a,
      };
    });
  });

  const showOuterRing = () => props.mystical >= 0.5;
  const tickMode = () =>
    props.mystical >= 0.9 ? "full" : props.mystical >= 0.5 ? "sparse" : "none";
  const showConstellation = () => props.mystical >= 0.9;

  const ticks = createMemo<JSX.Element[]>(() => {
    if (tickMode() === "none") return [];
    const items: JSX.Element[] = [];
    for (let i = 0; i < 48; i++) {
      const major = i % 4 === 0;
      if (tickMode() === "sparse" && !major) continue;
      const a = ((i * 360) / 48) * (Math.PI / 180);
      const r1 = 206;
      const r2 = major ? 195 : 201;
      items.push(
        <line
          x1={C + r1 * Math.cos(a)}
          y1={C + r1 * Math.sin(a)}
          x2={C + r2 * Math.cos(a)}
          y2={C + r2 * Math.sin(a)}
          stroke="currentColor"
          stroke-width={major ? 1.4 : 0.8}
        />,
      );
    }
    return items;
  });

  const dur = (base: number) =>
    `${(base / Math.max(props.motion, 0.001)).toFixed(1)}s`;

  return (
    <div style={{ position: "relative", width: `${S}px`, height: `${S}px`, flex: "none" }}>
      <svg
        width={S}
        height={S}
        viewBox={`0 0 ${S} ${S}`}
        style={{ position: "absolute", inset: 0, overflow: "visible" }}
      >
        <defs>
          <radialGradient id="sc-coreGlow" cx="50%" cy="50%" r="50%">
            <stop
              offset="0%"
              stop-color={active() ? "#9F7CFF" : coreColor()}
              stop-opacity={active() ? 0.9 : 0.5}
            />
            <stop
              offset="55%"
              stop-color={active() ? "#5B3FD6" : coreColor()}
              stop-opacity={active() ? 0.45 : 0.18}
            />
            <stop offset="100%" stop-color="#000" stop-opacity="0" />
          </radialGradient>
          <linearGradient id="sc-threadGrad" x1="0" y1="0" x2="1" y2="0">
            <stop offset="0%" stop-color="#DB2777" />
            <stop offset="100%" stop-color="#6D5BF0" />
          </linearGradient>
          <linearGradient id="sc-coreFill" x1="0" y1="0" x2="1" y2="1">
            <stop offset="0%" stop-color="#4F46E5" />
            <stop offset="55%" stop-color="#7C5CF6" />
            <stop offset="100%" stop-color="#DB2777" />
          </linearGradient>
        </defs>

        {/* ambient glow */}
        <circle
          cx={C}
          cy={C}
          r={170}
          fill="url(#sc-coreGlow)"
          style={{
            opacity: active() ? 0.9 : 0.4,
            transition: "opacity 0.6s",
            "transform-box": "fill-box",
            "transform-origin": "center",
            animation:
              props.motion > 0
                ? `breathe ${active() ? "3.2s" : "5s"} ease-in-out infinite`
                : "none",
          }}
        />

        {/* pulse rings */}
        <Show when={props.motion > 0}>
          <For each={[0, 1]}>
            {(i) => (
              <circle
                cx={C}
                cy={C}
                r={CORE_R + 6}
                fill="none"
                stroke={
                  active() ? "rgba(124,92,246,0.55)" : "rgba(168,150,220,0.32)"
                }
                stroke-width="1.4"
                style={{
                  "transform-box": "fill-box",
                  "transform-origin": "center",
                  animation: `pulse-ring ${dur(3.4)} ${(i * 1.7).toFixed(1)}s ease-out infinite`,
                }}
              />
            )}
          </For>
        </Show>

        {/* outer decorative ring */}
        <Show when={showOuterRing()}>
          <g
            style={{
              color: muted()
                ? "rgba(242,86,122,0.35)"
                : `rgba(168,150,220,${(0.18 + 0.14 * props.mystical).toFixed(3)})`,
              "transform-box": "fill-box",
              "transform-origin": "center",
              animation:
                props.motion > 0 ? `spin-rev ${dur(60)} linear infinite` : "none",
            }}
          >
            <circle
              cx={C}
              cy={C}
              r={201}
              fill="none"
              stroke="currentColor"
              stroke-width="1"
              opacity="0.5"
            />
            {ticks()}
          </g>
        </Show>

        {/* constellation dots */}
        <Show when={showConstellation()}>
          <g
            style={{
              "transform-box": "fill-box",
              "transform-origin": "center",
              animation:
                props.motion > 0 ? `spin-slow ${dur(120)} linear infinite` : "none",
            }}
          >
            <For each={Array.from({ length: 12 })}>
              {(_, i) => {
                const a = ((i() * 30 + 15) * Math.PI) / 180;
                const r = 178;
                return (
                  <circle
                    cx={C + r * Math.cos(a)}
                    cy={C + r * Math.sin(a)}
                    r={i() % 3 === 0 ? 1.6 : 0.9}
                    fill="rgba(201,184,255,0.7)"
                  />
                );
              }}
            </For>
          </g>
        </Show>

        {/* mid ring */}
        <circle
          cx={C}
          cy={C}
          r={NODE_R}
          fill="none"
          stroke="rgba(168,150,220,0.16)"
          stroke-width="1"
          stroke-dasharray="1 7"
          stroke-linecap="round"
          style={{
            "transform-box": "fill-box",
            "transform-origin": "center",
            animation:
              props.motion > 0 ? `spin-slow ${dur(40)} linear infinite` : "none",
          }}
        />

        {/* orbiting spark */}
        <Show when={props.motion > 0}>
          <g
            style={{
              "transform-box": "fill-box",
              "transform-origin": "center",
              animation: `spin-slow ${dur(7)} linear infinite`,
            }}
          >
            <circle
              cx={C}
              cy={C - NODE_R}
              r={3}
              fill={active() ? "#EC4899" : "#9F7CFF"}
              style={{
                filter: "drop-shadow(0 0 6px currentColor)",
                color: active() ? "#EC4899" : "#9F7CFF",
              }}
            />
          </g>
        </Show>

        {/* threads */}
        <For each={nodes()}>
          {(n) => {
            const ix = C + (CORE_R + 8) * Math.cos(n.angle);
            const iy = C + (CORE_R + 8) * Math.sin(n.angle);
            const ox = C + (NODE_R - 30) * Math.cos(n.angle);
            const oy = C + (NODE_R - 30) * Math.sin(n.angle);
            if (!n.enabled) {
              return (
                <line
                  x1={ix}
                  y1={iy}
                  x2={ox}
                  y2={oy}
                  stroke="rgba(168,150,220,0.10)"
                  stroke-width="1"
                  stroke-dasharray="2 4"
                />
              );
            }
            return (
              <line
                x1={ix}
                y1={iy}
                x2={ox}
                y2={oy}
                stroke="url(#sc-threadGrad)"
                stroke-width={active() ? 2 : 1.4}
                stroke-dasharray="5 6"
                stroke-linecap="round"
                opacity={active() ? 0.95 : 0.5}
                style={{
                  animation:
                    active() && props.motion > 0
                      ? "dash-flow 1.1s linear infinite"
                      : "none",
                }}
              />
            );
          }}
        </For>

        {/* core rings */}
        <circle
          cx={C}
          cy={C}
          r={CORE_R + 14}
          fill="none"
          stroke={
            active() ? "rgba(124,92,246,0.4)" : "rgba(168,150,220,0.2)"
          }
          stroke-width="1"
          stroke-dasharray={active() ? "2 5" : "none"}
          style={{
            "transform-box": "fill-box",
            "transform-origin": "center",
            animation:
              active() && props.motion > 0
                ? "spin-slow 22s linear infinite"
                : "none",
          }}
        />
        <circle
          cx={C}
          cy={C}
          r={CORE_R}
          fill="var(--core-ink)"
          stroke={coreColor()}
          stroke-width="1.5"
          style={{
            filter: active()
              ? "drop-shadow(0 0 24px rgba(124,92,246,0.6))"
              : "none",
            transition: "all 0.5s",
          }}
        />
      </svg>

      {/* particles */}
      <Show when={active() && props.motion > 0}>
        <div style={{ position: "absolute", inset: 0, "pointer-events": "none" }}>
          <For each={[0, 1, 2, 3, 4, 5]}>
            {(i) => (
              <span
                style={{
                  position: "absolute",
                  left: `${42 + ((i * 13) % 18)}%`,
                  top: `${48 + (i % 3) * 4}%`,
                  width: "4px",
                  height: "4px",
                  "border-radius": "50%",
                  background: i % 2 ? "#EC4899" : "#7C5CF6",
                  "box-shadow": "0 0 8px currentColor",
                  color: i % 2 ? "#EC4899" : "#7C5CF6",
                  animation: `float-up ${(2.4 + i * 0.4).toFixed(2)}s ${(i * 0.35).toFixed(2)}s ease-out infinite`,
                }}
              />
            )}
          </For>
        </div>
      </Show>

      {/* CORE label */}
      <div
        style={{
          position: "absolute",
          left: `${C}px`,
          top: `${C}px`,
          transform: "translate(-50%, -50%)",
          width: `${CORE_R * 2 - 16}px`,
          height: `${CORE_R * 2 - 16}px`,
        }}
      >
        <div
          style={{
            width: "100%",
            height: "100%",
            "border-radius": "50%",
            display: "flex",
            "flex-direction": "column",
            "align-items": "center",
            "justify-content": "center",
            gap: "4px",
            animation:
              active() && props.motion > 0
                ? "breathe 3.2s ease-in-out infinite"
                : "none",
          }}
        >
          <div
            style={{
              color: muted() ? "#F2567A" : active() ? "#fff" : "var(--text-mid)",
              filter: active()
                ? "drop-shadow(0 0 10px rgba(255,255,255,0.5))"
                : "none",
            }}
          >
            <Sigil
              name={muted() ? "muted" : active() ? "modulated" : "clean"}
              size={40}
            />
          </div>
          <div
            class="display"
            style={{
              "font-size": "15px",
              color: muted() ? "#F2567A" : active() ? "#fff" : "var(--text-mid)",
              "letter-spacing": "0.02em",
            }}
          >
            {muted() ? "Muted" : active() ? "Modulated" : "Clean"}
          </div>
        </div>
      </div>

      {/* NODES */}
      <For each={nodes()}>
        {(n) => {
          const eff = EFFECTS[n.id];
          const isSel = () => props.selected === n.id;
          return (
            <button
              type="button"
              onClick={() => props.onSelect(n.id)}
              onDblClick={() => props.onToggle(n.id)}
              title={`${eff.name} — click to inspect, double-click to ${
                n.enabled ? "disable" : "enable"
              }`}
              style={{
                position: "absolute",
                left: `${n.x}px`,
                top: `${n.y}px`,
                transform: "translate(-50%, -50%)",
                width: "62px",
                display: "flex",
                "flex-direction": "column",
                "align-items": "center",
                gap: "5px",
                background: "none",
                padding: 0,
                border: "none",
              }}
            >
              <span
                style={{
                  width: "50px",
                  height: "50px",
                  "border-radius": "50%",
                  display: "grid",
                  "place-items": "center",
                  background: n.enabled
                    ? "var(--surface-2)"
                    : "var(--surface-1)",
                  border: isSel()
                    ? "1.5px solid var(--indigo)"
                    : n.enabled
                      ? "1px solid var(--line-glow)"
                      : "1px dashed var(--line-strong)",
                  color: n.enabled
                    ? active()
                      ? "#C9B8FF"
                      : "var(--indigo)"
                    : "var(--text-dim)",
                  "box-shadow":
                    n.enabled && active()
                      ? "0 0 16px rgba(124,92,246,0.4)"
                      : isSel()
                        ? "0 0 0 4px rgba(124,92,246,0.2)"
                        : "none",
                  transition: "all 0.25s",
                }}
              >
                <Sigil name={eff.sigil as SigilName} size={24} />
              </span>
              <span
                style={{
                  "font-family": "var(--font-mono)",
                  "font-size": "9px",
                  "letter-spacing": "0.04em",
                  "text-transform": "uppercase",
                  color: n.enabled ? "var(--text-mid)" : "var(--text-dim)",
                  "white-space": "nowrap",
                }}
              >
                {eff.name}
              </span>
              <Show when={n.enabled}>
                <span
                  style={{
                    "font-family": "var(--font-mono)",
                    "font-size": "9.5px",
                    color: active() ? "var(--pink)" : "var(--text-lo)",
                    "white-space": "nowrap",
                  }}
                >
                  {eff.readout(n.vals)}
                </span>
              </Show>
            </button>
          );
        }}
      </For>
    </div>
  );
}

/** Compute node positions around the orbit. Exposed for testing. */
export function nodePositions(n: number): { x: number; y: number; angle: number }[] {
  if (n === 0) return [];
  const out: { x: number; y: number; angle: number }[] = [];
  for (let i = 0; i < n; i++) {
    const a = ((-90 + i * (360 / n)) * Math.PI) / 180;
    out.push({ x: C + NODE_R * Math.cos(a), y: C + NODE_R * Math.sin(a), angle: a });
  }
  return out;
}
