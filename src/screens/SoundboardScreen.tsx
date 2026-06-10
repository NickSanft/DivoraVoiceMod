// Soundboard — Phase 8 brings:
//   • Tile drag-reorder (HTML5 native, persisted per folder).
//   • Per-tile colors via a right-click palette popover.
//   • Recent-folders dropdown next to "Change folder" so users with
//     multiple soundboards can hop between them in one click.
//   • Global tile hotkeys — `bindTileHotkey` now registers with the
//     `tauri-plugin-global-shortcut` so clips fire even when
//     DivoraVoice isn't the focused window. The in-app keydown
//     listener that was here in Phase 5 has been removed; the global
//     path handles both focused and unfocused use, which avoided the
//     double-fire that would otherwise happen when both layers ran.

import {
  createMemo,
  createSignal,
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

/** Palette offered in the per-tile right-click color picker. */
const TILE_PALETTE: { color: string; label: string }[] = [
  { color: "#7C5CF6", label: "Indigo" },
  { color: "#EC4899", label: "Pink" },
  { color: "#58C6F2", label: "Cyan" },
  { color: "#34D9A0", label: "Emerald" },
  { color: "#E9B14C", label: "Gold" },
  { color: "#F2567A", label: "Crimson" },
  { color: "#A99FC4", label: "Lilac" },
  { color: "#6E6590", label: "Slate" },
];

function defaultTileColor(id: string): string {
  let h = 0;
  for (let i = 0; i < id.length; i++) {
    h = (h * 31 + id.charCodeAt(i)) >>> 0;
  }
  return COLORS[h % COLORS.length]!;
}

function tileEmoji(label: string): string {
  const ch = label.trim().charAt(0).toUpperCase();
  const mapping: Record<string, string> = {
    A: "🜂", B: "🔔", C: "🐦", D: "😈", E: "👁", F: "🔥", G: "👻", H: "🫀",
    I: "💡", J: "🎷", K: "🔑", L: "🍃", M: "🌙", N: "🜄", O: "🦉", P: "🌀",
    Q: "⚙️", R: "🔮", S: "✨", T: "⚡", U: "🜁", V: "🌬", W: "🌊", X: "❌",
    Y: "🟡", Z: "💤",
  };
  return mapping[ch] ?? "🎵";
}

function fmtBytes(bytes: number): string {
  if (bytes <= 0) return "—";
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

interface ContextMenuState {
  clipId: string;
  x: number;
  y: number;
}

export function SoundboardScreen(): JSX.Element {
  const app = useApp();
  const [menu, setMenu] = createSignal<ContextMenuState | null>(null);

  // Filter (search) the already-sorted tile list so per-folder order
  // survives across searches.
  const filtered = createMemo<SoundboardTile[]>(() => {
    const q = app.soundboardSearch().trim().toLowerCase();
    const tiles = app.sortedTiles();
    if (!q) return tiles;
    return tiles.filter((t) => t.label.toLowerCase().includes(q));
  });

  const playingCount = () => Object.keys(app.playingClips).length;

  const dismissMenu = (): void => {
    setMenu(null);
  };

  // Click anywhere outside the menu closes it.
  onMount(() => {
    const handler = (e: MouseEvent) => {
      const target = e.target as HTMLElement | null;
      if (target?.closest("[data-tile-context-menu]")) return;
      setMenu(null);
    };
    window.addEventListener("pointerdown", handler);
    onCleanup(() => window.removeEventListener("pointerdown", handler));
  });

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

        {/* Phase 11: tell the user where soundboard clips actually go.
            The output callback runs the DSP chain on the mic first,
            then `soundboard.mix_into` on the SAME mono buffer, then
            fans out to the output device. Routing the engine output
            into CABLE Input therefore carries the clips to call
            participants alongside the modulated mic. */}
        <Show when={app.soundboardFolder()}>
          <div
            style={{
              display: "flex",
              "align-items": "center",
              gap: "var(--s2)",
              padding: "6px var(--s3)",
              "border-radius": "var(--r-md)",
              background: "var(--info-bg)",
              border: "1px solid rgba(88, 198, 242, 0.25)",
              "font-size": "var(--t-xs)",
              color: "var(--text-mid)",
            }}
          >
            <Sigil name="info" size={13} style={{ color: "var(--info)" }} />
            <span>
              Clips play through your selected output device — including
              your modulated mic, so Discord / Zoom / OBS callers hear them.
            </span>
          </div>
        </Show>
      </div>

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
                <TileGrid
                  tiles={filtered()}
                  onReorder={(from, to) => {
                    const folder = app.soundboardFolder();
                    if (folder) app.reorderTiles(folder, from, to);
                  }}
                  onContext={(clipId, x, y) => setMenu({ clipId, x, y })}
                />
              </Show>
            }
          >
            <EmptyState icon="refresh" title="Scanning folder…" />
          </Show>
        </Show>
      </div>

      <Show when={menu()} keyed>
        {(m) => (
          <ColorContextMenu
            clipId={m.clipId}
            x={m.x}
            y={m.y}
            onDismiss={dismissMenu}
          />
        )}
      </Show>
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
  const [recentOpen, setRecentOpen] = createSignal(false);
  let recentRef: HTMLDivElement | undefined;

  const folder = () => app.soundboardFolder() ?? "No folder";

  // Click outside the recent menu closes it.
  onMount(() => {
    const handler = (e: PointerEvent) => {
      if (recentRef && !recentRef.contains(e.target as Node)) {
        setRecentOpen(false);
      }
    };
    document.addEventListener("pointerdown", handler);
    onCleanup(() => document.removeEventListener("pointerdown", handler));
  });

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
          <Show when={app.recentFolders().length > 0}>
            <div ref={recentRef} style={{ position: "relative" }}>
              <Button
                variant="ghost"
                size="sm"
                iconR="chevronD"
                onClick={() => setRecentOpen(!recentOpen())}
                aria-haspopup="menu"
                aria-expanded={recentOpen()}
              >
                Recent
              </Button>
              <Show when={recentOpen()}>
                <div
                  class="dropdown"
                  role="menu"
                  style={{
                    position: "absolute",
                    top: "calc(100% + 6px)",
                    right: 0,
                    "min-width": "320px",
                    "z-index": 30,
                  }}
                >
                  <For each={app.recentFolders()}>
                    {(path) => (
                      <div
                        class={`dropdown-opt ${path === app.soundboardFolder() ? "is-selected" : ""}`}
                        role="menuitem"
                        onClick={() => {
                          setRecentOpen(false);
                          void app.useRecentFolder(path);
                        }}
                      >
                        <Sigil
                          name="folder"
                          size={14}
                          style={{ color: "var(--text-lo)" }}
                        />
                        <div style={{ flex: 1, "min-width": 0 }}>
                          <div
                            class="mono"
                            style={{
                              "font-size": "var(--t-xs)",
                              color: "var(--text-hi)",
                              overflow: "hidden",
                              "text-overflow": "ellipsis",
                              "white-space": "nowrap",
                            }}
                            title={path}
                          >
                            {path}
                          </div>
                        </div>
                        <button
                          type="button"
                          class="icon-btn"
                          aria-label="Remove from recent"
                          onClick={(e) => {
                            e.stopPropagation();
                            app.removeRecentFolder(path);
                          }}
                          style={{
                            width: "24px",
                            height: "24px",
                          }}
                        >
                          <Sigil name="x" size={12} />
                        </button>
                      </div>
                    )}
                  </For>
                </div>
              </Show>
            </div>
          </Show>
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
      <div
        style={{ display: "flex", "align-items": "center", gap: "var(--s2)" }}
        title="Master soundboard volume"
      >
        <Sigil name="soundboard" size={16} style={{ color: "var(--text-lo)" }} />
        <input
          type="range"
          min="0"
          max="200"
          step="5"
          aria-label="Master soundboard volume"
          value={Math.round(app.soundboardMasterGain() * 100)}
          onInput={(e) =>
            app.setSoundboardMasterGain(Number(e.currentTarget.value) / 100)
          }
          style={{ width: "110px" }}
        />
        <span
          class="mono tnum"
          style={{
            "font-size": "var(--t-xs)",
            color: "var(--text-mid)",
            width: "40px",
          }}
        >
          {Math.round(app.soundboardMasterGain() * 100)}%
        </span>
      </div>
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
  onReorder: (from: number, to: number) => void;
  onContext: (clipId: string, x: number, y: number) => void;
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
      <For each={props.tiles}>
        {(tile, idx) => (
          <Tile
            tile={tile}
            index={idx()}
            onReorder={props.onReorder}
            onContext={props.onContext}
          />
        )}
      </For>
    </div>
  );
}

interface TileProps {
  tile: SoundboardTile;
  index: number;
  onReorder: (from: number, to: number) => void;
  onContext: (clipId: string, x: number, y: number) => void;
}

function Tile(props: TileProps): JSX.Element {
  const app = useApp();
  const color = createMemo(
    () => app.tileColors[props.tile.id] ?? defaultTileColor(props.tile.id),
  );
  const emoji = createMemo(() => tileEmoji(props.tile.label));
  const hotkey = () => app.tileHotkeys[props.tile.id];
  const [dragOver, setDragOver] = createSignal(false);
  const [dragging, setDragging] = createSignal(false);
  // True for a short window after a successful drag — used to swallow
  // the synthetic `click` event that fires on pointerup. Otherwise the
  // tile the user just dropped onto would immediately play.
  let suppressClickUntil = 0;

  const playing = createMemo(() => {
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

  const onDragStart = (e: DragEvent): void => {
    if (!e.dataTransfer) return;
    e.dataTransfer.effectAllowed = "move";
    e.dataTransfer.setData("text/plain", String(props.index));
    // Some Chromium builds (notably WebView2 on Windows) won't initiate
    // a drag without a non-empty payload; the line above already covers
    // that. Setting a custom MIME helps too:
    e.dataTransfer.setData("application/x-divora-tile-index", String(props.index));
    setDragging(true);
  };
  const onDragEnd = (): void => {
    setDragging(false);
    setDragOver(false);
    // Block click for ~300 ms after drag ends so the synthetic click
    // (which fires on the source tile after a drop) doesn't trigger play.
    suppressClickUntil = Date.now() + 300;
  };
  const onDragEnter = (e: DragEvent): void => {
    e.preventDefault();
    setDragOver(true);
  };
  const onDragOver = (e: DragEvent): void => {
    e.preventDefault();
    if (e.dataTransfer) e.dataTransfer.dropEffect = "move";
    setDragOver(true);
  };
  const onDragLeave = (e: DragEvent): void => {
    // Ignore leave events that bubble up from child nodes inside the tile.
    const related = e.relatedTarget as Node | null;
    const currentEl = e.currentTarget as Node | null;
    if (related && currentEl && currentEl.contains(related)) return;
    setDragOver(false);
  };
  const onDrop = (e: DragEvent): void => {
    e.preventDefault();
    setDragOver(false);
    // Prefer our custom MIME (survives `dataTransfer.getData` quirks on
    // some Windows builds); fall back to text/plain.
    const dt = e.dataTransfer;
    const raw =
      dt?.getData("application/x-divora-tile-index") ??
      dt?.getData("text/plain") ??
      "";
    const from = Number(raw);
    if (Number.isFinite(from) && from !== props.index) {
      props.onReorder(from, props.index);
    }
    // Swallow the post-drop click that Chromium dispatches on the
    // drop target.
    suppressClickUntil = Date.now() + 300;
  };

  const onContextMenu = (e: MouseEvent): void => {
    e.preventDefault();
    props.onContext(props.tile.id, e.clientX, e.clientY);
  };

  const onClick = (e: MouseEvent): void => {
    if (Date.now() < suppressClickUntil) {
      e.preventDefault();
      return;
    }
    void app.playClip(props.tile);
  };
  const onKeyDown = (e: KeyboardEvent): void => {
    if (e.key === "Enter" || e.key === " ") {
      e.preventDefault();
      void app.playClip(props.tile);
    }
  };

  return (
    // A <button> with draggable=true is unreliable in Chromium / WebView2
    // (the browser frequently refuses to initiate the drag). A
    // div + role="button" + tabindex makes drag work AND keeps the tile
    // keyboard-operable via Enter / Space.
    <div
      role="button"
      tabindex={0}
      aria-label={`Play ${props.tile.label}. Right-click for color, drag to reorder.`}
      draggable={true}
      onClick={onClick}
      onKeyDown={onKeyDown}
      onContextMenu={onContextMenu}
      onDragStart={onDragStart}
      onDragEnd={onDragEnd}
      onDragEnter={onDragEnter}
      onDragOver={onDragOver}
      onDragLeave={onDragLeave}
      onDrop={onDrop}
      title={`${props.tile.path}\nRight-click for color · drag to reorder`}
      style={{
        position: "relative",
        height: "120px",
        padding: "var(--s3) var(--s3) var(--s3)",
        "border-radius": "14px",
        background: "var(--surface-2)",
        border: dragOver()
          ? `1.5px dashed ${color()}`
          : playing()
            ? `1.5px solid ${color()}`
            : "1px solid var(--line)",
        "box-shadow": dragOver()
          ? `0 0 18px ${color()}55`
          : playing()
            ? `0 0 18px ${color()}66`
            : undefined,
        color: "var(--text-hi)",
        display: "flex",
        "flex-direction": "column",
        "justify-content": "space-between",
        "align-items": "stretch",
        "text-align": "left",
        cursor: dragging() ? "grabbing" : "grab",
        opacity: dragging() ? 0.6 : 1,
        transition: "border-color 0.15s, box-shadow 0.15s, opacity 0.15s",
        "user-select": "none",
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
    </div>
  );
}

interface ProgressRingProps {
  color: string;
  progress: number;
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

interface ColorContextMenuProps {
  clipId: string;
  x: number;
  y: number;
  onDismiss: () => void;
}

function ColorContextMenu(props: ColorContextMenuProps): JSX.Element {
  const app = useApp();
  const current = () => app.tileColors[props.clipId];

  return (
    <div
      data-tile-context-menu
      role="menu"
      style={{
        position: "fixed",
        top: `${Math.min(props.y, window.innerHeight - 320)}px`,
        left: `${Math.min(props.x, window.innerWidth - 240)}px`,
        "z-index": 100,
        padding: "var(--s3)",
        "border-radius": "var(--r-md)",
        background: "var(--surface-2)",
        border: "1px solid var(--line-glow)",
        "box-shadow": "var(--shadow-3)",
        "min-width": "220px",
      }}
    >
      {/* Phase 15: per-tile volume */}
      <div
        class="eyebrow"
        style={{ "margin-bottom": "var(--s2)", color: "var(--text-lo)" }}
      >
        Volume
      </div>
      <div style={{ display: "flex", "align-items": "center", gap: "var(--s2)" }}>
        <input
          type="range"
          min="0"
          max="200"
          step="5"
          aria-label="Tile volume"
          value={Math.round(app.tileGain(props.clipId) * 100)}
          onInput={(e) =>
            app.setTileGain(props.clipId, Number(e.currentTarget.value) / 100)
          }
          style={{ flex: 1 }}
        />
        <span
          class="mono tnum"
          style={{
            "font-size": "var(--t-xs)",
            color: "var(--text-mid)",
            width: "40px",
          }}
        >
          {Math.round(app.tileGain(props.clipId) * 100)}%
        </span>
      </div>
      <div
        class="eyebrow"
        style={{
          "margin-bottom": "var(--s2)",
          "margin-top": "var(--s3)",
          color: "var(--text-lo)",
        }}
      >
        Tile color
      </div>
      <div
        style={{
          display: "grid",
          "grid-template-columns": "repeat(4, 1fr)",
          gap: "var(--s2)",
        }}
      >
        <For each={TILE_PALETTE}>
          {(swatch) => (
            <button
              type="button"
              title={swatch.label}
              aria-label={swatch.label}
              onClick={() => {
                app.setTileColor(props.clipId, swatch.color);
                props.onDismiss();
              }}
              style={{
                width: "40px",
                height: "32px",
                "border-radius": "var(--r-sm)",
                background: swatch.color,
                border:
                  current() === swatch.color
                    ? "2px solid var(--text-hi)"
                    : "1px solid var(--line)",
                cursor: "pointer",
                "box-shadow":
                  current() === swatch.color
                    ? `0 0 12px ${swatch.color}88`
                    : "none",
                transition: "box-shadow 0.15s, border-color 0.15s",
              }}
            />
          )}
        </For>
      </div>
      <Show when={current()}>
        <button
          type="button"
          class="btn btn-ghost btn-sm"
          style={{ width: "100%", "margin-top": "var(--s3)" }}
          onClick={() => {
            app.setTileColor(props.clipId, null);
            props.onDismiss();
          }}
        >
          Reset to default
        </button>
      </Show>
    </div>
  );
}
