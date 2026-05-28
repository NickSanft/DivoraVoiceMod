// DMark — the brand identity: a white "D" on the indigo→pink gradient
// rounded square. Used in the titlebar, the wizard, About, etc.
//
// Ported from docs/mockups/prototype/divora/sigils.jsx (`DMark`).

import { createUniqueId } from "solid-js";
import type { JSX } from "solid-js";

export interface DMarkProps {
  size?: number;
  radius?: number;
}

export function DMark(props: DMarkProps): JSX.Element {
  const size = () => props.size ?? 26;
  const radius = () => props.radius ?? 8;
  const gradId = `dmark-${createUniqueId()}`;
  return (
    <svg
      width={size()}
      height={size()}
      viewBox="0 0 40 40"
      style={{ display: "block" }}
      aria-label="DivoraVoice"
    >
      <defs>
        <linearGradient id={gradId} x1="0" y1="0" x2="1" y2="1">
          <stop offset="0" stop-color="#4F46E5" />
          <stop offset="0.5" stop-color="#7C5CF6" />
          <stop offset="1" stop-color="#DB2777" />
        </linearGradient>
      </defs>
      <rect width="40" height="40" rx={(radius() * 40) / size()} fill={`url(#${gradId})`} />
      <path
        d="M13 11 H20 a9 9 0 0 1 0 18 H13 Z"
        fill="none"
        stroke="#fff"
        stroke-width="3.4"
        stroke-linejoin="round"
      />
    </svg>
  );
}
