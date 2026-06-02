// Presets — Phase 4 lights up the full browser + editor: clickable
// preset list grouped by Bundled / User on the left, with an editor on
// the right that lets the user adjust each effect's parameters, save,
// duplicate, delete, and export. The clicked preset immediately becomes
// active (the design's "Use" button is now a Mixer-shortcut at the top
// of the editor).

import {
  createMemo,
  createSignal,
  For,
  Show,
  type JSX,
} from "solid-js";
import { Badge } from "../components/Badge";
import { Button } from "../components/Button";
import { EmptyState } from "../components/EmptyState";
import { ExportPresetModal } from "../components/ExportPresetModal";
import { IconButton } from "../components/IconButton";
import { Sigil, type SigilName } from "../components/Sigil";
import { Slider } from "../components/Slider";
import { Toggle } from "../components/Toggle";
import { EFFECTS } from "../data/effects";
import { useApp } from "../stores/app";
import type { ChainEntry, EffectId, Preset } from "../types";

export function PresetsScreen(): JSX.Element {
  const app = useApp();

  // Editor target = active preset. Clicking a row in the list calls
  // usePreset(id), which sets the active id and resets A/B for it.
  const bundled = createMemo(() => app.presets().filter((p) => p.tag === "Bundled"));
  const user = createMemo(() => app.presets().filter((p) => p.tag === "User"));

  const [exportOpen, setExportOpen] = createSignal(false);
  const [exportJson, setExportJson] = createSignal("");
  const [actionError, setActionError] = createSignal<string | null>(null);

  const openExport = async (): Promise<void> => {
    const snapshot = app.presetWithCurrentChain(app.presetId());
    if (!snapshot) return;
    try {
      const json = await app.exportPreset(snapshot);
      setExportJson(json);
      setExportOpen(true);
    } catch (err) {
      setActionError(String(err));
    }
  };

  const onDuplicate = async (): Promise<void> => {
    setActionError(null);
    try {
      const copy = await app.duplicatePreset(app.presetId());
      if (copy) {
        // Switch to the new copy so the user can edit it immediately.
        app.usePreset(copy.id);
      }
    } catch (err) {
      setActionError(String(err));
    }
  };

  const onSave = async (): Promise<void> => {
    setActionError(null);
    const snapshot = app.presetWithCurrentChain(app.presetId());
    if (!snapshot) return;
    if (snapshot.tag !== "User") {
      // Bundled presets can't be saved over — use Duplicate instead.
      setActionError(
        "Bundled presets are read-only. Use Duplicate to make a user copy.",
      );
      return;
    }
    try {
      await app.savePreset(snapshot);
    } catch (err) {
      setActionError(String(err));
    }
  };

  const onDelete = async (): Promise<void> => {
    setActionError(null);
    try {
      await app.deletePreset(app.presetId());
    } catch (err) {
      setActionError(String(err));
    }
  };

  return (
    <div style={{ height: "100%", display: "flex", "min-height": 0 }}>
      <PresetList
        bundled={bundled()}
        user={user()}
        activeId={app.presetId()}
        onPick={(id) => app.usePreset(id)}
      />
      <div style={{ flex: 1, "min-width": 0, display: "flex", "flex-direction": "column" }}>
        <PresetEditor
          preset={app.preset()}
          chain={app.chain()}
          isActiveRunning={app.engineRunning()}
          onUse={() => app.setNav("mixer")}
          onSave={() => void onSave()}
          onDuplicate={() => void onDuplicate()}
          onExport={() => void openExport()}
          onDelete={() => void onDelete()}
          onParam={(idx, key, value) => app.setChainParam(idx, key, value)}
          onToggleEnabled={(idx, enabled) => app.setChainEnabled(idx, enabled)}
          onReorder={(from, to) => app.reorderChainEntries(from, to)}
          actionError={actionError()}
          dismissError={() => setActionError(null)}
        />
      </div>
      <ExportPresetModal
        open={exportOpen()}
        presetName={app.preset().name}
        json={exportJson()}
        onClose={() => setExportOpen(false)}
      />
    </div>
  );
}

interface PresetListProps {
  bundled: Preset[];
  user: Preset[];
  activeId: string;
  onPick: (id: string) => void;
}

function PresetList(props: PresetListProps): JSX.Element {
  return (
    <div
      class="panel"
      style={{
        width: "248px",
        "flex-shrink": 0,
        "border-radius": 0,
        "border-top": "none",
        "border-bottom": "none",
        "border-left": "none",
        display: "flex",
        "flex-direction": "column",
        background: "var(--surface-1)",
      }}
    >
      <div
        style={{
          padding: "var(--s5) var(--s5) var(--s4)",
          display: "flex",
          "align-items": "center",
          gap: "var(--s2)",
        }}
      >
        <h2
          class="display"
          style={{ "font-size": "var(--t-h2)", "font-weight": 700, flex: 1 }}
        >
          Presets
        </h2>
        <span
          class="eyebrow"
          style={{ color: "var(--text-lo)" }}
          title="Add new preset (Phase 4.x)"
        >
          {props.bundled.length + props.user.length}
        </span>
      </div>

      <div style={{ flex: 1, overflow: "auto", padding: "0 var(--s3) var(--s5)" }}>
        <PresetGroup
          label="Bundled"
          presets={props.bundled}
          activeId={props.activeId}
          onPick={props.onPick}
        />
        <PresetGroup
          label="User"
          presets={props.user}
          activeId={props.activeId}
          onPick={props.onPick}
        />
      </div>
    </div>
  );
}

interface PresetGroupProps {
  label: string;
  presets: Preset[];
  activeId: string;
  onPick: (id: string) => void;
}

function PresetGroup(props: PresetGroupProps): JSX.Element {
  return (
    <div style={{ "margin-top": "var(--s4)" }}>
      <div
        class="eyebrow"
        style={{
          padding: "0 var(--s2)",
          "margin-bottom": "var(--s2)",
          display: "flex",
          gap: "var(--s2)",
          "align-items": "center",
        }}
      >
        <span>{props.label}</span>
        <span style={{ color: "var(--text-dim)" }}>· {props.presets.length}</span>
      </div>
      <Show
        when={props.presets.length > 0}
        fallback={
          <div
            style={{
              "font-size": "var(--t-xs)",
              color: "var(--text-dim)",
              padding: "0 var(--s2)",
            }}
          >
            None yet.
          </div>
        }
      >
        <ul style={{ "list-style": "none", padding: 0, margin: 0 }}>
          <For each={props.presets}>
            {(p) => (
              <li>
                <button
                  type="button"
                  onClick={() => props.onPick(p.id)}
                  aria-pressed={props.activeId === p.id}
                  style={{
                    width: "100%",
                    display: "flex",
                    "align-items": "center",
                    gap: "var(--s3)",
                    padding: "8px 10px",
                    "border-radius": "var(--r-md)",
                    background:
                      props.activeId === p.id ? "var(--accent-bg)" : "transparent",
                    border:
                      props.activeId === p.id
                        ? "1px solid var(--line-glow)"
                        : "1px solid transparent",
                    color: "var(--text-hi)",
                    "text-align": "left",
                    cursor: "pointer",
                    transition: "all 0.12s",
                  }}
                >
                  <span
                    style={{
                      width: "28px",
                      height: "28px",
                      "border-radius": "var(--r-sm)",
                      display: "grid",
                      "place-items": "center",
                      background: p.color + "22",
                      color: p.color,
                      "flex-shrink": 0,
                    }}
                  >
                    <Sigil name={p.glyph as SigilName} size={16} />
                  </span>
                  <span style={{ flex: 1, "min-width": 0 }}>
                    <div
                      style={{
                        "font-size": "var(--t-sm)",
                        "font-weight": 600,
                        overflow: "hidden",
                        "text-overflow": "ellipsis",
                        "white-space": "nowrap",
                      }}
                    >
                      {p.name}
                    </div>
                    <div
                      style={{
                        "font-size": "var(--t-xs)",
                        color: "var(--text-lo)",
                      }}
                    >
                      {p.chain.length} runes
                    </div>
                  </span>
                  <Show when={props.activeId === p.id}>
                    <span
                      title="In use"
                      style={{
                        width: "7px",
                        height: "7px",
                        "border-radius": "50%",
                        background: "var(--success)",
                        "box-shadow": "0 0 6px var(--success)",
                      }}
                    />
                  </Show>
                </button>
              </li>
            )}
          </For>
        </ul>
      </Show>
    </div>
  );
}

interface PresetEditorProps {
  preset: Preset;
  chain: ChainEntry[];
  isActiveRunning: boolean;
  onUse: () => void;
  onSave: () => void;
  onDuplicate: () => void;
  onExport: () => void;
  onDelete: () => void;
  onParam: (idx: number, key: string, value: number) => void;
  onToggleEnabled: (idx: number, enabled: boolean) => void;
  onReorder: (from: number, to: number) => void;
  actionError: string | null;
  dismissError: () => void;
}

function PresetEditor(props: PresetEditorProps): JSX.Element {
  const isBundled = () => props.preset.tag === "Bundled";
  const activeCount = () => props.chain.filter((c) => c.enabled).length;

  return (
    <div
      style={{
        flex: 1,
        display: "flex",
        "flex-direction": "column",
        "min-height": 0,
        padding: "var(--s7) var(--s7)",
        gap: "var(--s5)",
        overflow: "auto",
      }}
    >
      {/* Header */}
      <div style={{ display: "flex", "align-items": "flex-start", gap: "var(--s4)" }}>
        <div
          style={{
            width: "44px",
            height: "44px",
            "border-radius": "var(--r-md)",
            display: "grid",
            "place-items": "center",
            background: props.preset.color + "26",
            border: `1px solid ${props.preset.color}55`,
            color: props.preset.color,
            "flex-shrink": 0,
          }}
        >
          <Sigil name={props.preset.glyph as SigilName} size={24} />
        </div>
        <div style={{ flex: 1, "min-width": 0 }}>
          <div style={{ display: "flex", "align-items": "center", gap: "var(--s2)" }}>
            <h2
              class="display"
              style={{ "font-size": "var(--t-h1)", "font-weight": 700 }}
            >
              {props.preset.name}
            </h2>
            <Badge tone={isBundled() ? "accent" : "info"}>{props.preset.tag}</Badge>
            <Show when={props.isActiveRunning}>
              <Badge tone="success" icon="check">
                In use
              </Badge>
            </Show>
          </div>
          <div
            style={{
              "font-size": "var(--t-sm)",
              color: "var(--text-mid)",
              "margin-top": "4px",
              "max-width": "640px",
            }}
          >
            {props.preset.desc}
          </div>
        </div>
        <Button variant="primary" icon="play" onClick={props.onUse}>
          Use
        </Button>
      </div>

      {/* Action row */}
      <div
        style={{
          display: "flex",
          "align-items": "center",
          "justify-content": "space-between",
          gap: "var(--s3)",
        }}
      >
        <span class="eyebrow">
          Effect chain · {activeCount()} / {props.chain.length} active
        </span>
        <div style={{ display: "flex", gap: "var(--s2)" }}>
          <Button variant="ghost" icon="copy" onClick={props.onDuplicate}>
            Duplicate
          </Button>
          <Button variant="ghost" icon="external" onClick={props.onExport}>
            Export JSON
          </Button>
          <Button
            variant="ghost"
            icon="download"
            onClick={props.onSave}
            disabled={isBundled()}
          >
            Save
          </Button>
          <Button
            variant="danger"
            icon="trash"
            onClick={props.onDelete}
            disabled={isBundled()}
          >
            Delete
          </Button>
        </div>
      </div>

      <Show when={props.actionError}>
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
          <span style={{ flex: 1, "font-size": "var(--t-xs)" }}>
            {props.actionError}
          </span>
          <IconButton icon="x" onClick={props.dismissError} tip="Dismiss" />
        </div>
      </Show>

      {/* Chain editor */}
      <Show
        when={props.chain.length > 0}
        fallback={
          <EmptyState icon="presets" title="No effects in this chain">
            Add a rune to the chain (coming in a later phase).
          </EmptyState>
        }
      >
        <div
          style={{
            display: "flex",
            "flex-direction": "column",
            gap: "var(--s3)",
            "max-width": "760px",
          }}
        >
          <For each={props.chain}>
            {(entry, idx) => (
              <ChainCard
                entry={entry}
                index={idx()}
                total={props.chain.length}
                onParam={(key, value) => props.onParam(idx(), key, value)}
                onToggle={(enabled) => props.onToggleEnabled(idx(), enabled)}
                onReorder={props.onReorder}
              />
            )}
          </For>
        </div>
      </Show>
    </div>
  );
}

interface ChainCardProps {
  entry: ChainEntry;
  index: number;
  total: number;
  onParam: (key: string, value: number) => void;
  onToggle: (enabled: boolean) => void;
  onReorder: (from: number, to: number) => void;
}

function ChainCard(props: ChainCardProps): JSX.Element {
  const def = () => EFFECTS[props.entry.id as EffectId];
  const [dragOver, setDragOver] = createSignal(false);
  const [dragging, setDragging] = createSignal(false);

  // EXPLICIT DRAG HANDLE PATTERN (v0.8.2). The card contains interactive
  // children (Toggle button + parameter Sliders), and HTML5 drag will
  // refuse to initiate when the pointer-down lands on any of those.
  // Plus <div draggable> in WebView2 is finicky enough that we want one
  // unambiguous drag source. The drag-handle <span> below carries
  // `draggable={true}` and `onDragStart`; the card itself only acts as
  // a drop target (no `draggable`, no dragstart). End result: drag only
  // starts from the visible drag-handle sigil and lands anywhere on
  // any sibling card.

  const onDragStart = (e: DragEvent): void => {
    if (!e.dataTransfer) return;
    e.dataTransfer.effectAllowed = "move";
    e.dataTransfer.setData("text/plain", String(props.index));
    // Custom MIME survives Chromium's occasional getData quirks.
    e.dataTransfer.setData(
      "application/x-divora-chain-index",
      String(props.index),
    );
    setDragging(true);
  };
  const onDragEnd = (): void => {
    setDragging(false);
    setDragOver(false);
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
    // Ignore leave events bubbling from descendants inside the card.
    const related = e.relatedTarget as Node | null;
    const currentEl = e.currentTarget as Node | null;
    if (related && currentEl && currentEl.contains(related)) return;
    setDragOver(false);
  };
  const onDrop = (e: DragEvent): void => {
    e.preventDefault();
    setDragOver(false);
    const dt = e.dataTransfer;
    const raw =
      dt?.getData("application/x-divora-chain-index") ??
      dt?.getData("text/plain") ??
      "";
    const from = Number(raw);
    if (Number.isFinite(from) && from !== props.index) {
      props.onReorder(from, props.index);
    }
  };

  const fmtVal = (v: number, unit: string): string => {
    if (Number.isInteger(v) || Math.abs(v) >= 100) {
      return `${v}${unit}`;
    }
    return `${v.toFixed(1)}${unit}`;
  };

  return (
    <div
      class="card"
      onDragEnter={onDragEnter}
      onDragOver={onDragOver}
      onDragLeave={onDragLeave}
      onDrop={onDrop}
      style={{
        padding: "var(--s4)",
        display: "flex",
        "flex-direction": "column",
        gap: "var(--s3)",
        "border-color": dragOver() ? "var(--indigo)" : undefined,
        "box-shadow": dragOver()
          ? "0 0 0 3px rgba(124, 92, 246, 0.25)"
          : undefined,
        opacity: dragging() ? 0.6 : 1,
        transition: "border-color 0.12s, box-shadow 0.12s, opacity 0.12s",
      }}
    >
      <div
        style={{
          display: "flex",
          "align-items": "center",
          gap: "var(--s3)",
        }}
      >
        <span
          draggable={true}
          onDragStart={onDragStart}
          onDragEnd={onDragEnd}
          role="button"
          tabindex={0}
          aria-label={`Drag to reorder ${def().name}. Currently at position ${props.index + 1} of ${props.total}.`}
          style={{
            color: "var(--text-lo)",
            cursor: dragging() ? "grabbing" : "grab",
            display: "grid",
            "place-items": "center",
            padding: "4px",
            margin: "-4px",
            "border-radius": "var(--r-sm)",
            "user-select": "none",
            "touch-action": "none",
          }}
          title={`Position ${props.index + 1} of ${props.total} — drag to reorder`}
        >
          <Sigil name="drag" size={18} />
        </span>
        <span
          style={{
            width: "32px",
            height: "32px",
            "border-radius": "var(--r-sm)",
            display: "grid",
            "place-items": "center",
            background: props.entry.enabled
              ? "var(--accent-bg)"
              : "var(--surface-3)",
            color: props.entry.enabled ? "var(--indigo)" : "var(--text-lo)",
            "flex-shrink": 0,
          }}
        >
          <Sigil name={def().sigil as SigilName} size={18} />
        </span>
        <div style={{ flex: 1, "min-width": 0 }}>
          <div
            style={{
              "font-size": "var(--t-sm)",
              "font-weight": 600,
              color: "var(--text-hi)",
            }}
          >
            {def().name}
          </div>
          <div
            class="mono tnum"
            style={{
              "font-size": "var(--t-xs)",
              color: props.entry.enabled ? "var(--text-mid)" : "var(--text-lo)",
            }}
          >
            {def().readout(props.entry.vals)}
          </div>
        </div>
        <Toggle
          on={props.entry.enabled}
          onChange={(next) => props.onToggle(next)}
          ariaLabel={`Enable ${def().name}`}
        />
      </div>
      <Show when={props.entry.enabled}>
        <div
          style={{
            display: "grid",
            "grid-template-columns":
              def().params.length > 1 ? "1fr 1fr" : "1fr",
            gap: "var(--s4) var(--s5)",
            "padding-top": "var(--s2)",
            "border-top": "1px dashed var(--line)",
          }}
        >
          <For each={def().params}>
            {(p) => {
              const value = () => props.entry.vals[p.key] ?? p.default;
              return (
                <div>
                  <div
                    style={{
                      display: "flex",
                      "justify-content": "space-between",
                      "margin-bottom": "4px",
                    }}
                  >
                    <span
                      style={{
                        "font-size": "var(--t-xs)",
                        color: "var(--text-mid)",
                      }}
                    >
                      {p.label}
                    </span>
                    <span
                      class="mono tnum"
                      style={{
                        "font-size": "var(--t-xs)",
                        color: "var(--text-hi)",
                      }}
                    >
                      {fmtVal(value(), p.unit)}
                    </span>
                  </div>
                  <Slider
                    value={value()}
                    min={p.min}
                    max={p.max}
                    step={p.step}
                    bipolar={p.bipolar}
                    onChange={(v) => props.onParam(p.key, v)}
                    ariaLabel={`${def().name} ${p.label}`}
                  />
                </div>
              );
            }}
          </For>
        </div>
      </Show>
    </div>
  );
}
