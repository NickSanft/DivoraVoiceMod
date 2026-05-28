// sigils.jsx — Divora's custom arcane icon language.
// Geometric occult marks drawn as stroked SVG. Functional sigils for
// effects, nav, statuses + a utility icon set. All use currentColor.
const { createElement: h } = React;

// Each entry returns the inner SVG markup for a 24x24 viewBox.
const SIGIL_PATHS = {
  // ---------- EFFECTS ----------
  pitch: (
    <g fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round">
      <path d="M5 16.5 L9 11 L12 13.5 L15.5 7.5 L19 5" />
      <path d="M19 5 L19 8.5 M19 5 L15.7 5.3" />
      <circle cx="5" cy="16.5" r="1.3" fill="currentColor" stroke="none" />
    </g>
  ),
  formant: (
    <g fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round">
      <path d="M6 12 a6 6 0 0 1 12 0" />
      <path d="M8.5 12 a3.5 3.5 0 0 1 7 0" />
      <circle cx="12" cy="12" r="1.2" fill="currentColor" stroke="none" />
      <path d="M5 15.5 h14" strokeDasharray="1.5 2.5" opacity="0.7" />
    </g>
  ),
  reverb: (
    <g fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round">
      <circle cx="12" cy="12" r="1.4" fill="currentColor" stroke="none" />
      <path d="M12 7.5 a4.5 4.5 0 0 1 0 9" opacity="0.95" />
      <path d="M12 4.5 a7.5 7.5 0 0 1 0 15" opacity="0.55" />
      <path d="M12 16.5 a4.5 4.5 0 0 1 0 -9" opacity="0.95" />
      <path d="M12 19.5 a7.5 7.5 0 0 1 0 -15" opacity="0.55" />
    </g>
  ),
  eq: (
    <g fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round">
      <path d="M7 5 V19 M12 5 V19 M17 5 V19" />
      <circle cx="7" cy="14" r="1.7" fill="var(--surface-2,#1a1528)" />
      <circle cx="12" cy="9" r="1.7" fill="var(--surface-2,#1a1528)" />
      <circle cx="17" cy="15" r="1.7" fill="var(--surface-2,#1a1528)" />
    </g>
  ),
  robot: (
    <g fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round">
      <rect x="5.5" y="8" width="13" height="11" rx="2.5" />
      <path d="M12 8 V4.5 M12 4.5 h2" />
      <circle cx="12" cy="4.2" r="0.9" fill="currentColor" stroke="none" />
      <path d="M9 13 h0.01 M15 13 h0.01" strokeWidth="2.4" />
      <path d="M9.5 16 h5" />
    </g>
  ),
  distortion: (
    <g fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round">
      <path d="M4 12 L8 12 L10 5 L13.5 19 L15.5 12 L20 12" />
    </g>
  ),
  echo: (
    <g fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round">
      <path d="M7.5 7 L12 12 L7.5 17" />
      <path d="M12.5 7 L17 12 L12.5 17" opacity="0.55" />
      <path d="M3 9.5 L3 14.5" opacity="0.9" />
    </g>
  ),
  gate: (
    <g fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round">
      <path d="M6 20 V7 a6 6 0 0 1 12 0 V20" />
      <path d="M12 4.5 V20" strokeDasharray="2 2.4" opacity="0.7" />
      <path d="M4.5 20 h15" />
    </g>
  ),
  // ---------- NAV ----------
  mixer: (
    <g fill="none" stroke="currentColor" strokeWidth="1.5">
      <circle cx="12" cy="12" r="7.5" />
      <circle cx="12" cy="12" r="2" fill="currentColor" stroke="none" />
      <circle cx="12" cy="4.5" r="1.4" fill="currentColor" stroke="none" />
      <circle cx="18.5" cy="15.5" r="1.1" fill="currentColor" stroke="none" opacity="0.7" />
      <circle cx="5.5" cy="15.5" r="1.1" fill="currentColor" stroke="none" opacity="0.7" />
    </g>
  ),
  soundboard: (
    <g fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinejoin="round">
      <rect x="4.5" y="4.5" width="6.2" height="6.2" rx="1.6" />
      <rect x="13.3" y="4.5" width="6.2" height="6.2" rx="1.6" fill="currentColor" opacity="0.85" />
      <rect x="4.5" y="13.3" width="6.2" height="6.2" rx="1.6" fill="currentColor" opacity="0.85" />
      <rect x="13.3" y="13.3" width="6.2" height="6.2" rx="1.6" />
    </g>
  ),
  presets: (
    <g fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinejoin="round">
      <path d="M12 3.5 L20 8 L12 12.5 L4 8 Z" />
      <path d="M4 12 L12 16.5 L20 12" opacity="0.6" />
      <path d="M4 16 L12 20.5 L20 16" opacity="0.35" />
    </g>
  ),
  settings: (
    <g fill="none" stroke="currentColor" strokeWidth="1.5">
      <circle cx="12" cy="12" r="3" />
      <circle cx="12" cy="12" r="7.5" strokeDasharray="0.5 3.2" strokeLinecap="round" />
      <path d="M12 2.5 V5 M12 19 V21.5 M2.5 12 H5 M19 12 H21.5" strokeLinecap="round" />
    </g>
  ),
  // ---------- STATUS ----------
  clean: (
    <g fill="none" stroke="currentColor" strokeWidth="1.5">
      <circle cx="12" cy="12" r="7" />
      <circle cx="12" cy="12" r="1.6" fill="currentColor" stroke="none" />
    </g>
  ),
  modulated: (
    <g fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round">
      <circle cx="12" cy="12" r="2.4" fill="currentColor" stroke="none" />
      <path d="M12 2 V5.5 M12 18.5 V22 M2 12 H5.5 M18.5 12 H22 M5 5 L7.5 7.5 M16.5 16.5 L19 19 M19 5 L16.5 7.5 M7.5 16.5 L5 19" />
    </g>
  ),
  muted: (
    <g fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round">
      <circle cx="12" cy="12" r="7" />
      <path d="M7 7 L17 17" />
    </g>
  ),
  monitor: (
    <g fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round">
      <path d="M5 13 v-1 a7 7 0 0 1 14 0 v1" />
      <path d="M5 13 a2 2 0 0 1 2 2 v2 a2 2 0 0 1 -2 2 a2 2 0 0 1 -2 -2 v-2 a2 2 0 0 1 2 -2" />
      <path d="M19 13 a2 2 0 0 0 -2 2 v2 a2 2 0 0 0 2 2 a2 2 0 0 0 2 -2 v-2 a2 2 0 0 0 -2 -2" />
    </g>
  ),
  mic: (
    <g fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round">
      <rect x="9" y="3" width="6" height="11" rx="3" />
      <path d="M5.5 11 a6.5 6.5 0 0 0 13 0" />
      <path d="M12 17.5 V21 M9 21 h6" />
    </g>
  ),
  output: (
    <g fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round">
      <path d="M4 9.5 H7.5 L12 5.5 V18.5 L7.5 14.5 H4 Z" />
      <path d="M15 9 a4 4 0 0 1 0 6" />
      <path d="M17.5 6.5 a8 8 0 0 1 0 11" opacity="0.55" />
    </g>
  ),
  lock: (
    <g fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinejoin="round">
      <rect x="5" y="10.5" width="14" height="9.5" rx="2.5" />
      <path d="M8 10.5 V8 a4 4 0 0 1 8 0 v2.5" />
      <circle cx="12" cy="15" r="1.3" fill="currentColor" stroke="none" />
    </g>
  ),
  panic: (
    <g fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinejoin="round">
      <rect x="6" y="6" width="12" height="12" rx="2.5" />
    </g>
  ),
  // ---------- UTILITY ----------
  play: (<path d="M8 5.5 L18 12 L8 18.5 Z" fill="currentColor" />),
  stop: (<rect x="7" y="7" width="10" height="10" rx="2" fill="currentColor" />),
  chevronR: (<path d="M9.5 6 L15.5 12 L9.5 18" fill="none" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" strokeLinejoin="round" />),
  chevronD: (<path d="M6 9.5 L12 15.5 L18 9.5" fill="none" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" strokeLinejoin="round" />),
  chevronL: (<path d="M14.5 6 L8.5 12 L14.5 18" fill="none" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" strokeLinejoin="round" />),
  check: (<path d="M5 12.5 L10 17.5 L19 6.5" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round" />),
  x: (<path d="M6 6 L18 18 M18 6 L6 18" fill="none" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" />),
  plus: (<path d="M12 5 V19 M5 12 H19" fill="none" stroke="currentColor" strokeWidth="1.7" strokeLinecap="round" />),
  search: (
    <g fill="none" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round">
      <circle cx="11" cy="11" r="6" /><path d="M15.5 15.5 L20 20" />
    </g>
  ),
  drag: (
    <g fill="currentColor"><circle cx="9" cy="6" r="1.4" /><circle cx="15" cy="6" r="1.4" /><circle cx="9" cy="12" r="1.4" /><circle cx="15" cy="12" r="1.4" /><circle cx="9" cy="18" r="1.4" /><circle cx="15" cy="18" r="1.4" /></g>
  ),
  folder: (
    <path d="M4 7.5 a2 2 0 0 1 2 -2 H9.5 L12 8 H18 a2 2 0 0 1 2 2 V17 a2 2 0 0 1 -2 2 H6 a2 2 0 0 1 -2 -2 Z" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinejoin="round" />
  ),
  download: (
    <g fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round">
      <path d="M12 4 V15 M8 11 L12 15 L16 11" /><path d="M5 18.5 H19" />
    </g>
  ),
  external: (
    <g fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round">
      <path d="M14 5 H19 V10" /><path d="M19 5 L11 13" /><path d="M17 13.5 V18 a1.5 1.5 0 0 1 -1.5 1.5 H6.5 A1.5 1.5 0 0 1 5 18 V8.5 A1.5 1.5 0 0 1 6.5 7 H11" />
    </g>
  ),
  trash: (
    <g fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round">
      <path d="M5 7 H19 M9 7 V5.5 A1.5 1.5 0 0 1 10.5 4 H13.5 A1.5 1.5 0 0 1 15 5.5 V7 M7 7 L8 19 A1.5 1.5 0 0 0 9.5 20.3 H14.5 A1.5 1.5 0 0 0 16 19 L17 7" />
    </g>
  ),
  copy: (
    <g fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinejoin="round">
      <rect x="8" y="8" width="11" height="11" rx="2" /><path d="M5 16 H4.5 A1.5 1.5 0 0 1 5 5 H14.5 A1.5 1.5 0 0 1 16 5" transform="translate(0 0)" /><path d="M5 16 a1.5 1.5 0 0 1 -1.5 -1.5 V6.5 A1.5 1.5 0 0 1 5 5 H13.5 A1.5 1.5 0 0 1 15 6.5 V8" />
    </g>
  ),
  info: (
    <g fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round">
      <circle cx="12" cy="12" r="8" /><path d="M12 11 V16" /><circle cx="12" cy="8" r="0.3" strokeWidth="2" />
    </g>
  ),
  warning: (
    <g fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round">
      <path d="M12 4 L21 19 H3 Z" /><path d="M12 10 V14" /><circle cx="12" cy="16.5" r="0.3" strokeWidth="2" />
    </g>
  ),
  github: (
    <path fill="currentColor" d="M12 2.5a9.5 9.5 0 0 0-3 18.5c.5.1.66-.2.66-.46v-1.6c-2.7.6-3.27-1.16-3.27-1.16-.44-1.12-1.08-1.42-1.08-1.42-.88-.6.07-.59.07-.59.97.07 1.49 1 1.49 1 .87 1.5 2.27 1.06 2.82.8.09-.63.34-1.06.62-1.3-2.16-.25-4.43-1.08-4.43-4.8 0-1.06.38-1.93 1-2.6-.1-.25-.44-1.24.1-2.58 0 0 .82-.26 2.68 1a9.3 9.3 0 0 1 4.88 0c1.86-1.26 2.67-1 2.67-1 .55 1.34.2 2.33.1 2.58.63.67 1 1.54 1 2.6 0 3.73-2.27 4.55-4.44 4.79.35.3.66.9.66 1.81v2.68c0 .26.16.57.67.46A9.5 9.5 0 0 0 12 2.5Z" />
  ),
  shield: (
    <g fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinejoin="round">
      <path d="M12 3.5 L19 6 V11 c0 4.5 -3 7.5 -7 9.5 c-4 -2 -7 -5 -7 -9.5 V6 Z" />
      <path d="M9 12 L11 14 L15.5 9.5" strokeLinecap="round" />
    </g>
  ),
  ab: (
    <g fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round">
      <path d="M4 16 L7 7 L10 16 M5 13 H9" /><path d="M14 7 H17 a2 2 0 0 1 0 4 H14 Z M14 11 H17.5 a2 2 0 0 1 0 4.5 H14 V7" />
    </g>
  ),
  bolt: (<path d="M13 3 L5 13 H11 L10 21 L19 10 H12 Z" fill="currentColor" />),
  wave: (
    <g fill="none" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round">
      <path d="M3 12 q3 -7 6 0 t6 0 t6 0" />
    </g>
  ),
  eye: (
    <g fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round">
      <path d="M3 12 s3.5 -6 9 -6 9 6 9 6 -3.5 6 -9 6 -9 -6 -9 -6Z" /><circle cx="12" cy="12" r="2.4" />
    </g>
  ),
  keyboard: (
    <g fill="none" stroke="currentColor" strokeWidth="1.4" strokeLinecap="round" strokeLinejoin="round">
      <rect x="3" y="6.5" width="18" height="11" rx="2.2" />
      <path d="M7 10 h0.01 M11 10 h0.01 M15 10 h0.01 M7 13.5 h0.01 M8.5 13.5 h7" strokeWidth="1.8" />
    </g>
  ),
  refresh: (
    <g fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round">
      <path d="M19 5.5 V9.5 H15" /><path d="M18.5 9.5 A7 7 0 1 0 19 13" />
    </g>
  ),
};

function Sigil({ name, size = 20, className = "", style = {}, strokeColor }) {
  const inner = SIGIL_PATHS[name];
  if (!inner) return null;
  return (
    <svg width={size} height={size} viewBox="0 0 24 24" className={"sigil " + className}
      style={{ display: "block", color: strokeColor, ...style }} aria-hidden="true">
      {inner}
    </svg>
  );
}

// Brand "D" mark — white D on indigo→pink gradient.
function DMark({ size = 26, radius = 8 }) {
  const id = "dgrad" + size;
  return (
    <svg width={size} height={size} viewBox="0 0 40 40" style={{ display: "block" }}>
      <defs>
        <linearGradient id={id} x1="0" y1="0" x2="1" y2="1">
          <stop offset="0" stopColor="#4F46E5" /><stop offset="0.5" stopColor="#7C5CF6" /><stop offset="1" stopColor="#DB2777" />
        </linearGradient>
      </defs>
      <rect width="40" height="40" rx={radius * 40 / size} fill={`url(#${id})`} />
      <path d="M13 11 H20 a9 9 0 0 1 0 18 H13 Z" fill="none" stroke="#fff" strokeWidth="3.4" strokeLinejoin="round" />
    </svg>
  );
}

Object.assign(window, { Sigil, DMark, SIGIL_PATHS });
