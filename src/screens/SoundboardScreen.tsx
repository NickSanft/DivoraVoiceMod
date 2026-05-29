// Soundboard — Phase 5 lights up the tile grid backed by a real folder
// on disk. Click a tile to play; click the Stop-all button to panic;
// search filters by label; per-tile in-app hotkeys play whatever clip
// they're bound to (global hotkeys are Phase 6).

import {
  createMemo,
  For,
  onCleanup,
  onMount,
  Show,
  type JSX,
} from "solid-js";
import { Button } from "../components/Button";
import { EmptyState } from "../components/EmptyState";
import { IconButton } from "../components/IconButton";
import { Kbd } from "../components/Kbd";
import { Sigil } from "../components/Sigil";
import { useApp } from "../stores/app";
import type { SoundboardTile } from "../types";

// Small palette so tiles look distinct without users having to pick colours.
const COLORS = [
  "#7C5CF6",
  "#58C6F2",
  "#34D9A0",
  "#E9B14C",
  "#F2567A",
  "#A99FC4",
  "#6D5BF0",
  "#EC4899",
];

function tileColor(id: string): string {
  let h = 0;
  for (let i = 0; i < id.length; i++) {
    h = (h * 31 + id.charCodeAt(i)) >>> 0;
  }
  return COLORS[h % COLORS.length]!;
}

function tileEmoji(label: string): string {
  const ch = label.trim().charAt(0).toUpperCase();
  // Map first letter to a vaguely sound-themed emoji set; otherwise fall
  // back to a music note. Purely decorative — users can rebind in a
  // future phase.
  const mapping: Record<string, string> = {
    A: "🜂",
    B: "🔔",
    C: "🐦",
    D: "😈",
    E: "👁",
    F: "🔥",
    G: "👻",
    H: "🫀",
    I: "💡",
    J: "🎷",
    K: "🔑",
    L: "🍃",
    M: "🌙",
    N: "🜄",
    O: "🦉",
    P: "🌀",
    Q: "⚙️",
    R: "🔮",
    S: "✨",
    T: "⚡",
    U: "🜁",
    V: "🌬",
    W: "🌊",
    X: "❌",
    Y: "🟡",
    Z: "💤",
  };
  return mapping[ch] ?? "🎵";
}

function fmtBytes(bytes: number): string {
  if (bytes <= 0) return "—";
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

export function SoundboardScreen(): JSX.Element {
  const app = useApp();

  // In-app hotkey listener — Phase 6 will move this to a global Tauri
  // hotkey registration so soundboard hotkeys work while the window is
  // unfocused. For Phase 5 the document listener is enough.
  onMount(() => {
    const handler = (e: KeyboardEvent) => {
      // Skip when typing into a field.
      const target = e.target as HTMLElement | null;
      if (
        target &&
        (target.tagName === "INPUT" ||
          target.tagName === "TEXTAREA" ||
          target.isContentEditable)
      ) {
        return;
      }
      const pressed = describeKey(e);
      if (!pressed) return;
      for (const tile of app.soundboardTiles()) {
        const binding = app.tileHotkeys[tile.id];
        if (!binding || binding.length === 0) continue;
        if (binding.join("+") === pressed) {
          e.preventDefault();
          void app.playClip(tile);
          return;
        }
      }
    };
    window.addEventListener("keydown", handler);
    onCleanup(() => window.removeEventListener("keydown", handler));
  });

  const filtered = createMemo<SoundboardTile[]>(() => {
    const q = app.soundboardSearch().trim().toLowerCase();
    if (!q) return app.soundboardTiles();
    return app
      .soundboardTiles()
      .filter((t) => t.label.toLowerCase().includes(q));
  });

  const playingCount = () => Object.keys(app.playingClips).length;

  return (
    <div
      style={{
        height: "100%",
        display: "flex",
        "flex-direction": "column",
        "min-height": 0,
      }}
    >
      <div
        style={{
          padding: "20px 24px 0",
          display: "flex",
          "flex-direction": "column",
          gap: "var(--s5)",
          "flex-shrink": 0,
        }}
      >
        <Header
          playingCount={playingCount()}
          onPickFolder={() => void app.pickSoundboardFolder()}
          onPanic={() => void app.panicSoundboard()}
        />

        <Show when={app.soundboardError()} keyed>
          {(err) => (
            <div
              class="card"
              style={{
                padding: "var(--s3) var(--s4)",
                background: "var(--danger-bg)",
                "border-color": "rgba(242, 86, 122, 0.4)",
                color: "var(--danger)",
                display: "flex",
                "align-items": "center",
                gap: "var(--s3)",
              }}
            >
              <Sigil name="warning" size={16} />
              <span style={{ flex: 1, "font-size": "var(--t-xs)" }}>{err}</span>
              <IconButton
                icon="x"
                onClick={() => app.setSoundboardError(null)}
                tip="Dismiss"
              />
            </div>
          )}
        </Show>
      </div>

      {/* Dedicated scroll container — sits above the Show chain so its
          flex sizing doesn't depend on which branch renders. */}
      <div
        style={{
          flex: 1,
          "min-height": 0,
          overflow: "auto",
          padding: "var(--s5) 24px 24px",
        }}
      >
        <Show
          when={app.soundboardFolder()}
          fallback={
            <EmptyState icon="folder" title="No folder picked yet">
              <p>
                Pick a folder of audio files (WAV, MP3, OGG, FLAC, Opus) and
                they'll appear here as clickable tiles. Click one to play it
                into the modulated mic feed.
              </p>
              <Button
                variant="primary"
                icon="folder"
                onClick={() => void app.pickSoundboardFolder()}
              >
                Pick a folder
              </Button>
            </EmptyState>
          }
        >
          <Show
            when={app.soundboardLoading()}
            fallback={
              <Show
                when={filtered().length > 0}
                fallback={
                  <EmptyState
                    icon="search"
                    title={
                      app.soundboardSearch().length > 0
                        ? "No tiles match your search"
                        : "No audio files in this folder"
                    }
                  >
                    {app.soundboardSearch().length > 0
                      ? "Try a different query, or clear the search."
                      : "WAV, MP3, OGG, FLAC, and Opus files are picked up."}
                  </EmptyState>
                }
              >
                <TileGrid tiles={filtered()} />
              </Show>
            }
          >
            <EmptyState icon="refresh" title="Scanning folder…" />
          </Show>
        </Show>
      </div>
    </div>
  );
}

interface HeaderProps {
  playingCount: number;
  onPickFolder: () => void;
  onPanic: () => void;
}

function Header(props: HeaderProps): JSX.Element {
  const app = useApp();
  const folder = () => app.soundboardFolder() ?? "No folder";
  return (
    <div
      style={{
        display: "flex",
        "align-items": "center",
        gap: "var(--s4)",
        flex: "none",
      }}
    >
      <div style={{ display: "flex", "flex-direction": "column", gap: "2px" }}>
        <span class="eyebrow">Soundboard</span>
        <div style={{ display: "flex", "align-items": "center", gap: "var(--s2)" }}>
          <Sigil name="folder" size={16} style={{ color: "var(--text-lo)" }} />
          <span
            style={{
              "font-family": "var(--font-mono)",
              "font-size": "var(--t-xs)",
              color: "var(--text-mid)",
              "max-width": "520px",
              overflow: "hidden",
              "text-overflow": "ellipsis",
              "white-space": "nowrap",
            }}
            title={folder()}
          >
            {folder()}
          </span>
          <Button variant="ghost" size="sm" onClick={props.onPickFolder}>
            Change folder
          </Button>
        </div>
      </div>
      <div style={{ flex: 1 }} />
      <input
        type="text"
        placeholder="Search clips…"
        value={app.soundboardSearch()}
        onInput={(e) => app.setSoundboardSearch(e.currentTarget.value)}
        style={{
          width: "260px",
          height: "36px",
          padding: "0 12px",
          "border-radius": "var(--r-md)",
          background: "var(--surface-2)",
          border: "1px solid var(--line-strong)",
          color: "var(--text-hi)",
          "font-size": "var(--t-sm)",
        }}
      />
      <Button
        variant="danger"
        solid={props.playingCount > 0}
        icon="stop"
        disabled={props.playingCount === 0}
        onClick={props.onPanic}
      >
        Stop all{props.playingCount > 0 ? ` (${props.playingCount})` : ""}
      </Button>
    </div>
  );
}

interface TileGridProps {
  tiles: SoundboardTile[];
}

function TileGrid(props: TileGridProps): JSX.Element {
  return (
    <div
      style={{
        display: "grid",
        "grid-template-columns": "repeat(auto-fill, minmax(180px, 1fr))",
        gap: "var(--s4)",
        "align-content": "start",
      }}
    >
      <For each={props.tiles}>{(tile) => <Tile tile={tile} />}</For>
    </div>
  );
}

interface TileProps {
  tile: SoundboardTile;
}

function Tile(props: TileProps): JSX.Element {
  const app = useApp();
  const color = createMemo(() => tileColor(props.tile.id));
  const emoji = createMemo(() => tileEmoji(props.tile.label));
  const hotkey = () => app.tileHotkeys[props.tile.id];

  const playing = createMemo(() => {
    // Read clockTick so this memo refreshes during playback.
    void app.clockTick();
    return app.playingClips[props.tile.id] ?? null;
  });

  const progress = createMemo<number>(() => {
    const p = playing();
    if (!p) return 0;
    void app.clockTick();
    const now = typeof performance !== "undefined" ? performance.now() : Date.now();
    const ratio = (now - p.startedAt) / 1000 / p.durationSecs;
    return Math.max(0, Math.min(1, ratio));
  });

  const remaining = createMemo<string>(() => {
    const p = playing();
    if (!p) return "";
    void app.clockTick();
    const now = typeof performance !== "undefined" ? performance.now() : Date.now();
    const elapsed = (now - p.startedAt) / 1000;
    const left = Math.max(0, p.durationSecs - elapsed);
    return `${left.toFixed(1)}s`;
  });

  return (
    <button
      type="button"
      onClick={() => void app.playClip(props.tile)}
      title={props.tile.path}
      style={{
        position: "relative",
        height: "120px",
        padding: "var(--s3) var(--s3) var(--s3)",
        "border-radius": "14px",
        background: "var(--surface-2)",
        border: playing()
          ? `1.5px solid ${color()}`
          : "1px solid var(--line)",
        "box-shadow": playing() ? `0 0 18px ${color()}66` : undefined,
        color: "var(--text-hi)",
        display: "flex",
        "flex-direction": "column",
        "justify-content": "space-between",
        "align-items": "stretch",
        "text-align": "left",
        cursor: "pointer",
        transition: "border-color 0.15s, box-shadow 0.15s",
      }}
    >
      <div
        style={{
          display: "flex",
          "align-items": "flex-start",
          "justify-content": "space-between",
          gap: "var(--s2)",
        }}
      >
        <span
          style={{
            "font-size": playing() ? "30px" : "26px",
            "line-height": 1,
            transition: "font-size 0.15s",
          }}
        >
          {emoji()}
        </span>
        <span style={{ display: "flex", "flex-direction": "column", "align-items": "flex-end", gap: "4px" }}>
          <Show when={hotkey() && hotkey()!.length > 0}>
            <Kbd>{hotkey()!.join("+")}</Kbd>
          </Show>
          <Show when={playing()} keyed>
            {(p) => (
              <ProgressRing
                color={color()}
                progress={progress()}
                durationSecs={p.durationSecs}
              />
            )}
          </Show>
        </span>
      </div>
      <div style={{ display: "flex", "align-items": "center", gap: "var(--s2)" }}>
        <span
          style={{
            width: "7px",
            height: "7px",
            "border-radius": "50%",
            background: color(),
            "flex-shrink": 0,
          }}
        />
        <span
          style={{
            flex: 1,
            "font-size": "var(--t-sm)",
            "font-weight": 600,
            overflow: "hidden",
            "text-overflow": "ellipsis",
            "white-space": "nowrap",
          }}
        >
          {props.tile.label}
        </span>
        <Show
          when={playing()}
          fallback={
            <span
              class="mono"
              style={{ "font-size": "var(--t-xs)", color: "var(--text-lo)" }}
            >
              {fmtBytes(props.tile.sizeBytes)}
            </span>
          }
        >
          <span
            class="mono"
            style={{ "font-size": "var(--t-xs)", color: color() }}
          >
            {remaining()}
          </span>
        </Show>
      </div>
    </button>
  );
}

interface ProgressRingProps {
  color: string;
  progress: number; // 0..1
  durationSecs: number;
}

function ProgressRing(props: ProgressRingProps): JSX.Element {
  const RADIUS = 11;
  const CIRC = 2 * Math.PI * RADIUS;
  const dashoffset = () => CIRC * (1 - props.progress);
  return (
    <svg width="28" height="28" viewBox="0 0 28 28" aria-hidden="true">
      <circle
        cx="14"
        cy="14"
        r={RADIUS}
        fill="none"
        stroke="rgba(168,150,220,0.18)"
        stroke-width="2"
      />
      <circle
        cx="14"
        cy="14"
        r={RADIUS}
        fill="none"
        stroke={props.color}
        stroke-width="2.5"
        stroke-linecap="round"
        stroke-dasharray={String(CIRC)}
        stroke-dashoffset={String(dashoffset())}
        transform="rotate(-90 14 14)"
        style={{ transition: "stroke-dashoffset 0.05s linear" }}
      />
    </svg>
  );
}

/** Translate a keydown event into the same dotted form `bindTileHotkey` uses. */
function describeKey(e: KeyboardEvent): string | null {
  const parts: string[] = [];
  if (e.ctrlKey) parts.push("Ctrl");
  if (e.altKey) parts.push("Alt");
  if (e.shiftKey) parts.push("Shift");
  if (e.metaKey) parts.push("Win");
  let k = e.key;
  if (k === " ") k = "Space";
  else if (k.length === 1) k = k.toUpperCase();
  if (["Control", "Alt", "Shift", "Meta"].includes(e.key)) return null;
  parts.push(k);
  return parts.join("+");
}
