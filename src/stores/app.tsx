// Top-level app state for DivoraVoice.
//
// Phases 1–3 used a hardcoded PRESETS array. Phase 4 makes the preset
// list reactive — initially seeded with the FALLBACK_PRESETS bundled in
// the frontend, then replaced by the live list from the backend
// (`list_presets` Tauri command, which merges compile-time bundled JSON
// with on-disk user JSON). Phase 4 also adds:
//
//   - A/B compare snapshots per preset (`abSlots`)
//   - `usePreset`, `savePreset`, `duplicatePreset`, `deletePreset`,
//     `exportPresetJson` actions
//   - `refreshPresets` to pull the backend list

import {
  createContext,
  createEffect,
  createMemo,
  createSignal,
  on,
  type JSX,
  type Setter,
  useContext,
} from "solid-js";
import { createStore, type SetStoreFunction } from "solid-js/store";
import {
  ZERO_LEVELS,
  deleteUserPreset as deleteUserPresetCmd,
  detectVirtualMic as detectVirtualMicCmd,
  exportPresetJson as exportPresetJsonCmd,
  listInputDevices,
  listOutputDevices,
  listPresets,
  pickSoundboardFolder as pickSoundboardFolderCmd,
  playSoundboardClip as playSoundboardClipCmd,
  registerGlobalShortcut as registerGlobalShortcutCmd,
  saveUserPreset as saveUserPresetCmd,
  scanSoundboardFolder as scanSoundboardFolderCmd,
  setAudioMonitor as setAudioMonitorCmd,
  setEffectChain as setEffectChainCmd,
  setEffectEnabled as setEffectEnabledCmd,
  setEffectParam as setEffectParamCmd,
  startAudioEngine,
  stopAllSoundboardClips as stopAllSoundboardClipsCmd,
  stopAudioEngine,
  stopSoundboardClip as stopSoundboardClipCmd,
  unregisterGlobalShortcut as unregisterGlobalShortcutCmd,
  type DeviceInfo,
  type EffectSpec,
  type Levels,
  type SoundboardTile,
  type StreamInfo,
  type VirtualMicStatus,
  type WirePreset,
} from "../audio/api";
import { FALLBACK_PRESETS, presetFromWire } from "../data/presets";
import type {
  AbSlot,
  ChainEntry,
  EffectId,
  GlyphId,
  NavId,
  PlayingClip,
  Preset,
  PtmMode,
  TweaksState,
  UiState,
  VoiceStatus,
} from "../types";

const prefersReducedMotion = (): boolean => {
  if (typeof window === "undefined") return false;
  return window.matchMedia?.("(prefers-reduced-motion: reduce)").matches ?? false;
};

// Per the prototype (docs/mockups/prototype/divora/tweaks.jsx):
//   subtle  → 0.3
//   balanced → 0.7  (default)
//   rich    → 1.0
// The earlier 0 / 0.5 / 1.0 mapping made balanced ride right on the
// SpellCircle's `mystical >= 0.5` threshold so it looked like rich,
// and `subtle` (0) read as "everything off" which was harsher than
// the design intended.
export const MYSTICAL_SUBTLE = 0.3;
export const MYSTICAL_BALANCED = 0.7;
export const MYSTICAL_RICH = 1.0;

const defaultTweaks = (): TweaksState => ({
  mystical: MYSTICAL_BALANCED,
  motion: prefersReducedMotion() ? 0 : 1,
  mood: "violet",
  accent: "brand",
  grain: false,
  vignette: false,
});

const defaultGlyphs: Record<GlyphId, string> = {
  triangle: "velvet-demon",
  invtriangle: "glass-oracle",
  square: "hollow-king",
  circle: "clean",
};

const defaultUi = (): UiState => ({
  muted: false,
  monitor: true,
  ab: "A",
  ptmMode: "apply",
  ptmKey: "Space",
  pressed: false,
});

/** Per-preset A/B comparison snapshots. */
export interface AbSnapshots {
  A: ChainEntry[];
  B: ChainEntry[];
}

/** Stable id used for the system-wide hotkeys we register. */
export type HotkeyAction = "ptm" | "panic" | "monitor";

/** localStorage keys for Phase 8 persistence (tile metadata + recent folders). */
const STORAGE_KEYS = {
  tileColors: "divora.tileColors",
  tileOrder: "divora.tileOrder",
  tileHotkeys: "divora.tileHotkeys",
  recentFolders: "divora.recentFolders",
  tweaks: "divora.tweaks",
} as const;

/** Read + parse a JSON blob from localStorage; return `fallback` on miss / parse failure. */
function loadJson<T>(key: string, fallback: T): T {
  if (typeof window === "undefined") return fallback;
  try {
    const raw = window.localStorage.getItem(key);
    if (raw === null) return fallback;
    const parsed = JSON.parse(raw) as unknown;
    return (parsed ?? fallback) as T;
  } catch {
    return fallback;
  }
}

function saveJson(key: string, value: unknown): void {
  if (typeof window === "undefined") return;
  try {
    window.localStorage.setItem(key, JSON.stringify(value));
  } catch {
    /* quota, private mode, etc — non-fatal */
  }
}

/** Most-recently-used cap for the recent-folders dropdown. */
const RECENT_FOLDERS_MAX = 5;

/**
 * Default hotkey bindings are intentionally EMPTY — registering any
 * accelerator (especially a plain key like "Space") via
 * `tauri-plugin-global-shortcut` captures it system-wide, meaning
 * Discord / games / browsers never see the keypress while DivoraVoice
 * runs. That's a bad default for a tool a user might leave open in the
 * background.
 *
 * The in-app focused-window PTM listener in `App.tsx` reads
 * `ui.ptmKey` (which still defaults to "Space" via `defaultUi()`) and
 * only fires while DivoraVoice is the active window — so Space-to-
 * modulate still works in-app, without stealing the key globally.
 *
 * Users who want a true global hotkey (e.g. to PTM while a game is
 * focused) can bind one explicitly in Settings → Hotkeys.
 */
const DEFAULT_HOTKEY_BINDINGS: Record<HotkeyAction, string> = {
  ptm: "",
  panic: "",
  monitor: "",
};

const cloneChain = (entries: ChainEntry[]): ChainEntry[] =>
  entries.map((c) => ({ ...c, vals: { ...c.vals } }));

const initialChainsFor = (presets: Preset[]): Record<string, ChainEntry[]> =>
  Object.fromEntries(presets.map((p) => [p.id, cloneChain(p.chain)]));

const initialAbSlotsFor = (presets: Preset[]): Record<string, AbSnapshots> =>
  Object.fromEntries(
    presets.map((p) => [p.id, { A: cloneChain(p.chain), B: cloneChain(p.chain) }]),
  );

export interface AppState {
  // Nav
  nav: () => NavId;
  setNav: Setter<NavId>;

  // Presets
  presets: () => Preset[];
  setPresets: Setter<Preset[]>;
  refreshPresets: () => Promise<void>;
  /** True once at least one successful backend list has populated `presets`. */
  presetsLoaded: () => boolean;

  // Active preset + chain
  presetId: () => string;
  setPresetId: Setter<string>;
  preset: () => Preset;
  chains: Record<string, ChainEntry[]>;
  setChains: SetStoreFunction<Record<string, ChainEntry[]>>;
  chain: () => ChainEntry[];

  // A/B compare snapshots
  abSlots: Record<string, AbSnapshots>;
  setAbSlots: SetStoreFunction<Record<string, AbSnapshots>>;
  setAbSlot: (slot: AbSlot) => void;
  resetAbSlots: () => void;

  // UI state
  ui: UiState;
  setUi: SetStoreFunction<UiState>;

  // Wizard
  wizardOpen: () => boolean;
  setWizardOpen: Setter<boolean>;

  // Tweaks
  tweaks: TweaksState;
  setTweaks: SetStoreFunction<TweaksState>;

  // Glyph bindings
  glyphs: Record<GlyphId, string>;
  setGlyphs: SetStoreFunction<Record<GlyphId, string>>;

  // Audio engine
  audioInputs: () => DeviceInfo[];
  setAudioInputs: Setter<DeviceInfo[]>;
  audioOutputs: () => DeviceInfo[];
  setAudioOutputs: Setter<DeviceInfo[]>;
  selectedInput: () => string | null;
  setSelectedInput: Setter<string | null>;
  selectedOutput: () => string | null;
  setSelectedOutput: Setter<string | null>;
  engineRunning: () => boolean;
  setEngineRunning: Setter<boolean>;
  engineMonitoring: () => boolean;
  setEngineMonitoring: Setter<boolean>;
  engineError: () => string | null;
  setEngineError: Setter<string | null>;
  streamInfo: () => StreamInfo | null;
  setStreamInfo: Setter<StreamInfo | null>;
  inputLevels: () => Levels;
  setInputLevels: Setter<Levels>;
  outputLevels: () => Levels;
  setOutputLevels: Setter<Levels>;

  // Audio actions
  refreshDevices: () => Promise<void>;
  startEngine: () => Promise<void>;
  stopEngine: () => Promise<void>;
  toggleMonitor: () => Promise<void>;
  setMonitor: (enabled: boolean) => Promise<void>;

  // DSP / chain editing — local store mutation + backend sync.
  setChainParam: (effectIndex: number, key: string, value: number) => void;
  setChainEnabled: (effectIndex: number, enabled: boolean) => void;
  toggleEffectById: (id: EffectId) => void;
  syncChain: () => void;
  reorderChainEntries: (from: number, to: number) => void;

  // Preset actions
  usePreset: (id: string) => void;
  savePreset: (preset: Preset) => Promise<void>;
  duplicatePreset: (sourceId: string) => Promise<Preset | null>;
  deletePreset: (id: string) => Promise<void>;
  exportPreset: (preset: Preset) => Promise<string>;
  /** Snapshot the current `chain()` into the active preset's record. Used by Save. */
  presetWithCurrentChain: (id: string) => Preset | null;

  // Soundboard
  soundboardFolder: () => string | null;
  setSoundboardFolder: Setter<string | null>;
  soundboardTiles: () => SoundboardTile[];
  setSoundboardTiles: Setter<SoundboardTile[]>;
  soundboardLoading: () => boolean;
  soundboardError: () => string | null;
  setSoundboardError: Setter<string | null>;
  playingClips: Record<string, PlayingClip>;
  setPlayingClips: SetStoreFunction<Record<string, PlayingClip>>;
  tileHotkeys: Record<string, string[]>;
  setTileHotkeys: SetStoreFunction<Record<string, string[]>>;
  soundboardSearch: () => string;
  setSoundboardSearch: Setter<string>;
  clockTick: () => number;

  // Phase 8 tile metadata (persisted to localStorage).
  /** clipId → hex color override. Missing entry = default palette colour. */
  tileColors: Record<string, string>;
  setTileColor: (clipId: string, color: string | null) => void;
  /** folderPath → ordered list of clipIds. Tiles not in the list (e.g.
   * new files since last scan) fall to the end in alphabetical order. */
  tileOrder: Record<string, string[]>;
  reorderTiles: (folder: string, from: number, to: number) => void;
  /** `soundboardTiles()` with the per-folder order applied. */
  sortedTiles: () => SoundboardTile[];

  // Phase 8 recent folders (most-recently-used, capped at 5).
  recentFolders: () => string[];
  pushRecentFolder: (folder: string) => void;
  removeRecentFolder: (folder: string) => void;
  useRecentFolder: (folder: string) => Promise<void>;

  // Soundboard actions
  pickSoundboardFolder: () => Promise<void>;
  scanCurrentSoundboardFolder: () => Promise<void>;
  playClip: (tile: SoundboardTile) => Promise<void>;
  stopClip: (clipId: string) => Promise<void>;
  panicSoundboard: () => Promise<void>;
  bindTileHotkey: (clipId: string, keys: string[]) => Promise<void>;
  clearTileHotkey: (clipId: string) => Promise<void>;
  /** Triggered by SoundboardScreen when a tile finishes naturally. */
  markClipFinished: (clipId: string) => void;
  /** Map a global-shortcut event (id = clipId) to a playClip call. */
  playTileById: (clipId: string) => Promise<void>;

  // Virtual mic / VB-Cable
  virtualMicStatus: () => VirtualMicStatus | null;
  setVirtualMicStatus: Setter<VirtualMicStatus | null>;
  refreshVirtualMicStatus: () => Promise<void>;

  // Global hotkeys (system-level, registered via tauri-plugin-global-shortcut)
  hotkeyBindings: Record<HotkeyAction, string>;
  setHotkeyBindings: SetStoreFunction<Record<HotkeyAction, string>>;
  setHotkeyBinding: (action: HotkeyAction, accelerator: string) => Promise<void>;
  clearHotkeyBinding: (action: HotkeyAction) => Promise<void>;
  /** Push all currently-set bindings into the backend. Idempotent. */
  syncHotkeyBindings: () => Promise<void>;

  // Currently selected rune (effect) for the inspector.
  selectedEffect: () => EffectId | null;
  setSelectedEffect: Setter<EffectId | null>;

  // Derived
  hasEnabled: () => boolean;
  effectiveModulated: () => boolean;
  status: () => VoiceStatus;
}

export function createAppState(): AppState {
  const [nav, setNav] = createSignal<NavId>("mixer");

  // Presets are reactive: initially the fallback bundled list, replaced
  // by the backend list after `refreshPresets`.
  const [presets, setPresets] = createSignal<Preset[]>(FALLBACK_PRESETS);
  const [presetsLoaded, setPresetsLoaded] = createSignal(false);

  const firstPreset = FALLBACK_PRESETS[0];
  if (!firstPreset) {
    throw new Error("FALLBACK_PRESETS must contain at least one preset");
  }
  const [presetId, setPresetId] = createSignal<string>(firstPreset.id);
  const [chains, setChains] = createStore<Record<string, ChainEntry[]>>(
    initialChainsFor(FALLBACK_PRESETS),
  );
  const [abSlots, setAbSlots] = createStore<Record<string, AbSnapshots>>(
    initialAbSlotsFor(FALLBACK_PRESETS),
  );
  const [ui, setUi] = createStore<UiState>(defaultUi());
  const [wizardOpen, setWizardOpen] = createSignal(true);
  // Persist Tweaks across sessions — without this every restart resets
  // the Mystical/Motion/etc. choices, which made the controls feel
  // broken. Load is partial-merged onto defaults so adding new tweak
  // fields in a future phase doesn't blow up on old payloads.
  const persistedTweaks = loadJson<Partial<TweaksState>>(
    STORAGE_KEYS.tweaks,
    {},
  );
  const [tweaks, setTweaksRaw] = createStore<TweaksState>({
    ...defaultTweaks(),
    ...persistedTweaks,
  });
  const setTweaks: SetStoreFunction<TweaksState> = ((...args: unknown[]) => {
    // Re-dispatch into the underlying setter, then snapshot for save.
    (setTweaksRaw as (...a: unknown[]) => void)(...args);
    saveJson(STORAGE_KEYS.tweaks, { ...tweaks });
  }) as SetStoreFunction<TweaksState>;
  const [glyphs, setGlyphs] = createStore<Record<GlyphId, string>>({ ...defaultGlyphs });

  // Audio engine signals.
  const [audioInputs, setAudioInputs] = createSignal<DeviceInfo[]>([]);
  const [audioOutputs, setAudioOutputs] = createSignal<DeviceInfo[]>([]);
  const [selectedInput, setSelectedInput] = createSignal<string | null>(null);
  const [selectedOutput, setSelectedOutput] = createSignal<string | null>(null);
  const [engineRunning, setEngineRunning] = createSignal(false);
  const [engineMonitoring, setEngineMonitoring] = createSignal(true);
  const [engineError, setEngineError] = createSignal<string | null>(null);
  const [streamInfo, setStreamInfo] = createSignal<StreamInfo | null>(null);
  const [inputLevels, setInputLevels] = createSignal<Levels>(ZERO_LEVELS);
  const [outputLevels, setOutputLevels] = createSignal<Levels>(ZERO_LEVELS);

  const preset = createMemo<Preset>(
    () => presets().find((p) => p.id === presetId()) ?? presets()[0] ?? firstPreset,
  );
  const chain = createMemo<ChainEntry[]>(() => chains[presetId()] ?? []);

  const hasEnabled = createMemo(() => chain().some((c) => c.enabled));
  const effectiveModulated = createMemo(() =>
    ui.ptmMode === "apply" ? ui.pressed : !ui.pressed,
  );
  const status = createMemo<VoiceStatus>(() =>
    ui.muted ? "muted" : hasEnabled() && effectiveModulated() ? "modulated" : "clean",
  );

  // When the preset list changes (e.g. backend load adds user presets),
  // seed `chains` and `abSlots` for any preset not yet tracked.
  createEffect(() => {
    for (const p of presets()) {
      if (!chains[p.id]) {
        setChains(p.id, cloneChain(p.chain));
      }
      if (!abSlots[p.id]) {
        setAbSlots(p.id, { A: cloneChain(p.chain), B: cloneChain(p.chain) });
      }
    }
  });

  const refreshPresets = async (): Promise<void> => {
    try {
      const wire = await listPresets();
      if (!Array.isArray(wire)) {
        // Defensive: a mock or partial response shouldn't crash.
        return;
      }
      const next: Preset[] = wire.map(presetFromWire);
      if (next.length > 0) {
        setPresets(next);
        setPresetsLoaded(true);
      }
    } catch (err) {
      // Browser preview / Tauri unreachable — keep the fallback.
      console.warn(
        "[presets] listPresets failed; keeping fallback bundled set",
        err,
      );
    }
  };

  const refreshDevices = async (): Promise<void> => {
    const [ins, outs] = await Promise.all([
      listInputDevices(),
      listOutputDevices(),
    ]);
    setAudioInputs(ins);
    setAudioOutputs(outs);
    if (!selectedInput()) {
      const def = ins.find((d) => d.isDefault) ?? ins[0];
      if (def) setSelectedInput(def.name);
    }
    if (!selectedOutput()) {
      const def = outs.find((d) => d.isDefault) ?? outs[0];
      if (def) setSelectedOutput(def.name);
    }
  };

  const startEngine = async (): Promise<void> => {
    try {
      const info = await startAudioEngine(selectedInput(), selectedOutput());
      setStreamInfo(info);
      setEngineRunning(true);
      setEngineError(null);
    } catch (err) {
      setEngineRunning(false);
      setStreamInfo(null);
      setEngineError(String(err));
    }
  };

  const stopEngine = async (): Promise<void> => {
    await stopAudioEngine();
    setEngineRunning(false);
    setStreamInfo(null);
    setInputLevels(ZERO_LEVELS);
    setOutputLevels(ZERO_LEVELS);
  };

  const setMonitor = async (enabled: boolean): Promise<void> => {
    await setAudioMonitorCmd(enabled);
    setEngineMonitoring(enabled);
  };

  const toggleMonitor = (): Promise<void> => setMonitor(!engineMonitoring());

  // Phase 11: live device switching. When the user picks a different
  // input or output device in Settings while the engine is running,
  // restart the engine so the new device takes effect immediately.
  // Without this, only the displayed selection updates — the running
  // streams still point at the OLD devices, and the user sees no
  // change in behavior until they manually Stop / Start.
  //
  // `defer: true` prevents the effect from firing on creation; the
  // first call happens only when one of the signals actually changes.
  createEffect(
    on(
      [selectedInput, selectedOutput],
      ([newIn, newOut], prev) => {
        if (prev === undefined) return; // initial run guard (belt + suspenders with defer)
        const [prevIn, prevOut] = prev;
        if (newIn === prevIn && newOut === prevOut) return;
        if (!engineRunning()) return;
        void (async () => {
          await stopEngine();
          await startEngine();
        })();
      },
      { defer: true },
    ),
  );

  // Inspector selection — defaults to the first effect of the active chain.
  const [selectedEffect, setSelectedEffect] = createSignal<EffectId | null>(
    (firstPreset.chain[0]?.id as EffectId | undefined) ?? null,
  );

  // Whenever the active preset changes, reset selection to its first effect.
  createEffect(
    on(presetId, () => {
      const first = chain()[0]?.id ?? null;
      setSelectedEffect(first);
    }, { defer: true }),
  );

  const chainToSpecs = (entries: ChainEntry[]): EffectSpec[] =>
    entries.map((e) => ({
      kind: e.id,
      enabled: e.enabled,
      params: { ...e.vals },
    }));

  const syncChain = (): void => {
    if (!engineRunning()) return;
    void setEffectChainCmd(chainToSpecs(chain()));
  };

  // Send a full SetChain when either the active preset changes or the
  // engine starts up.
  createEffect(
    on([presetId, engineRunning], ([, running]) => {
      if (running) {
        void setEffectChainCmd(chainToSpecs(chain()));
      }
    }, { defer: true }),
  );

  const setChainParam = (effectIndex: number, key: string, value: number): void => {
    setChains(presetId(), effectIndex, "vals", key, value);
    if (engineRunning()) {
      void setEffectParamCmd(effectIndex, key, value);
    }
  };

  const setChainEnabled = (effectIndex: number, enabled: boolean): void => {
    setChains(presetId(), effectIndex, "enabled", enabled);
    if (engineRunning()) {
      void setEffectEnabledCmd(effectIndex, enabled);
    }
  };

  const toggleEffectById = (id: EffectId): void => {
    const idx = chain().findIndex((c) => c.id === id);
    if (idx < 0) return;
    const entry = chain()[idx];
    if (!entry) return;
    setChainEnabled(idx, !entry.enabled);
  };

  const reorderChainEntries = (from: number, to: number): void => {
    const current = chain();
    if (
      from === to ||
      from < 0 ||
      to < 0 ||
      from >= current.length ||
      to >= current.length
    ) {
      return;
    }
    const next = current.slice();
    const [moved] = next.splice(from, 1);
    if (moved !== undefined) next.splice(to, 0, moved);
    setChains(presetId(), next);
    if (engineRunning()) {
      void setEffectChainCmd(chainToSpecs(next));
    }
  };

  // A/B compare actions.

  const setAbSlot = (slot: AbSlot): void => {
    const current = ui.ab;
    if (current === slot) return;
    const id = presetId();
    // Snapshot the current chain into the slot we're leaving.
    setAbSlots(id, current, cloneChain(chain()));
    // Load the destination slot's chain into the active chain.
    const next = abSlots[id]?.[slot] ?? chain();
    setChains(id, cloneChain(next));
    setUi("ab", slot);
    if (engineRunning()) {
      void setEffectChainCmd(chainToSpecs(next));
    }
  };

  const resetAbSlots = (): void => {
    const id = presetId();
    const c = chain();
    setAbSlots(id, { A: cloneChain(c), B: cloneChain(c) });
    setUi("ab", "A");
  };

  // Preset actions.

  const presetWithCurrentChain = (id: string): Preset | null => {
    const p = presets().find((q) => q.id === id);
    if (!p) return null;
    const c = chains[id];
    if (!c) return p;
    return { ...p, chain: cloneChain(c) };
  };

  const usePreset = (id: string): void => {
    if (!presets().some((p) => p.id === id)) return;
    setPresetId(id);
    // Reset A/B for the new preset (both slots = current chain).
    const c = chains[id] ?? [];
    setAbSlots(id, { A: cloneChain(c), B: cloneChain(c) });
    setUi("ab", "A");
    if (engineRunning()) {
      void setEffectChainCmd(chainToSpecs(c));
    }
  };

  const presetToWire = (p: Preset): WirePreset => ({
    id: p.id,
    version: 1,
    name: p.name,
    color: p.color,
    glyph: p.glyph,
    tag: p.tag,
    desc: p.desc,
    chain: p.chain.map((c) => ({
      id: c.id,
      enabled: c.enabled,
      vals: { ...c.vals },
    })),
  });

  const savePreset = async (preset: Preset): Promise<void> => {
    if (preset.tag !== "User") {
      throw new Error(
        `cannot save bundled preset "${preset.id}" directly — use Save as`,
      );
    }
    await saveUserPresetCmd(presetToWire(preset));
    // Reflect into local state immediately.
    const idx = presets().findIndex((p) => p.id === preset.id);
    if (idx >= 0) {
      const next = presets().slice();
      next[idx] = preset;
      setPresets(next);
    } else {
      setPresets([...presets(), preset]);
    }
    setChains(preset.id, cloneChain(preset.chain));
    setAbSlots(preset.id, {
      A: cloneChain(preset.chain),
      B: cloneChain(preset.chain),
    });
  };

  const slugifyName = (name: string, existingIds: Set<string>): string => {
    const base = name
      .toLowerCase()
      .replace(/[^a-z0-9_-]+/g, "-")
      .replace(/^-+|-+$/g, "")
      .replace(/-+/g, "-");
    const safe = base.length > 0 ? base : "preset";
    if (!existingIds.has(safe)) return safe;
    let i = 2;
    while (existingIds.has(`${safe}-${i}`)) i += 1;
    return `${safe}-${i}`;
  };

  const duplicatePreset = async (sourceId: string): Promise<Preset | null> => {
    const source = presets().find((p) => p.id === sourceId);
    if (!source) return null;
    const liveChain = chains[sourceId] ?? source.chain;
    const existing = new Set(presets().map((p) => p.id));
    const newName = `${source.name} Copy`;
    const newId = slugifyName(newName, existing);
    const copy: Preset = {
      id: newId,
      name: newName,
      color: source.color,
      glyph: source.glyph,
      tag: "User",
      desc: source.desc,
      chain: cloneChain(liveChain),
    };
    await savePreset(copy);
    return copy;
  };

  const deletePreset = async (id: string): Promise<void> => {
    const target = presets().find((p) => p.id === id);
    if (!target) return;
    if (target.tag !== "User") {
      throw new Error(`cannot delete bundled preset "${id}"`);
    }
    await deleteUserPresetCmd(id);
    setPresets(presets().filter((p) => p.id !== id));
    // If we just deleted the active preset, fall back to the first one.
    if (presetId() === id) {
      const next = presets()[0] ?? firstPreset;
      setPresetId(next.id);
    }
  };

  const exportPreset = async (preset: Preset): Promise<string> => {
    try {
      return await exportPresetJsonCmd(presetToWire(preset));
    } catch {
      // Fall back to JSON.stringify when the backend isn't reachable.
      return `${JSON.stringify(presetToWire(preset), null, 2)}\n`;
    }
  };

  // Soundboard state.

  const [soundboardFolder, setSoundboardFolder] = createSignal<string | null>(null);
  const [soundboardTiles, setSoundboardTiles] = createSignal<SoundboardTile[]>([]);
  const [soundboardLoading, setSoundboardLoading] = createSignal(false);
  const [soundboardError, setSoundboardError] = createSignal<string | null>(null);
  const [playingClips, setPlayingClips] = createStore<Record<string, PlayingClip>>({});
  const [tileHotkeys, setTileHotkeys] = createStore<Record<string, string[]>>(
    loadJson<Record<string, string[]>>(STORAGE_KEYS.tileHotkeys, {}),
  );
  const [soundboardSearch, setSoundboardSearch] = createSignal("");
  const [clockTick, setClockTick] = createSignal(0);

  // Phase 8 tile metadata.
  const [tileColors, setTileColors] = createStore<Record<string, string>>(
    loadJson<Record<string, string>>(STORAGE_KEYS.tileColors, {}),
  );
  const [tileOrder, setTileOrder] = createStore<Record<string, string[]>>(
    loadJson<Record<string, string[]>>(STORAGE_KEYS.tileOrder, {}),
  );
  const [recentFolders, setRecentFolders] = createSignal<string[]>(
    loadJson<string[]>(STORAGE_KEYS.recentFolders, []),
  );

  // Tick the clock at ~30 Hz whenever any clip is playing. Components
  // read `clockTick()` to re-render progress rings without each tile
  // owning its own animation frame.
  createEffect(() => {
    const anyPlaying = Object.keys(playingClips).length > 0;
    if (!anyPlaying || typeof window === "undefined") return;
    let cancelled = false;
    let rafId = 0;
    const tick = () => {
      if (cancelled) return;
      setClockTick((n) => n + 1);
      const now = performance.now();
      for (const id of Object.keys(playingClips)) {
        const clip = playingClips[id];
        if (clip && (now - clip.startedAt) / 1000 >= clip.durationSecs) {
          setPlayingClips(id, undefined as unknown as PlayingClip);
        }
      }
      rafId = requestAnimationFrame(tick);
    };
    rafId = requestAnimationFrame(tick);
    return () => {
      cancelled = true;
      cancelAnimationFrame(rafId);
    };
  });

  /**
   * Apply the persisted per-folder order to the freshly scanned tile
   * list. Tiles whose id isn't in the saved order (new files since the
   * last scan) fall to the end in their scanner-default alphabetical
   * order.
   */
  const sortedTiles = createMemo<SoundboardTile[]>(() => {
    const folder = soundboardFolder();
    const tiles = soundboardTiles();
    if (!folder) return tiles;
    const order = tileOrder[folder];
    if (!order || order.length === 0) return tiles;
    const orderIndex = new Map<string, number>();
    order.forEach((id, i) => orderIndex.set(id, i));
    const withOrder: SoundboardTile[] = [];
    const without: SoundboardTile[] = [];
    for (const tile of tiles) {
      if (orderIndex.has(tile.id)) withOrder.push(tile);
      else without.push(tile);
    }
    withOrder.sort((a, b) => orderIndex.get(a.id)! - orderIndex.get(b.id)!);
    return [...withOrder, ...without];
  });

  const scanCurrentSoundboardFolder = async (): Promise<void> => {
    const folder = soundboardFolder();
    if (!folder) return;
    setSoundboardLoading(true);
    setSoundboardError(null);
    try {
      const tiles = await scanSoundboardFolderCmd(folder);
      setSoundboardTiles(tiles);
    } catch (err) {
      setSoundboardError(`scan failed: ${String(err)}`);
      setSoundboardTiles([]);
    } finally {
      setSoundboardLoading(false);
    }
  };

  // Push a folder to the recents list and persist. Move-to-front
  // semantics; cap at RECENT_FOLDERS_MAX.
  const pushRecentFolder = (folder: string): void => {
    if (!folder) return;
    const next = [folder, ...recentFolders().filter((p) => p !== folder)].slice(
      0,
      RECENT_FOLDERS_MAX,
    );
    setRecentFolders(next);
    saveJson(STORAGE_KEYS.recentFolders, next);
  };

  const removeRecentFolder = (folder: string): void => {
    const next = recentFolders().filter((p) => p !== folder);
    if (next.length === recentFolders().length) return;
    setRecentFolders(next);
    saveJson(STORAGE_KEYS.recentFolders, next);
  };

  const useRecentFolder = async (folder: string): Promise<void> => {
    setSoundboardFolder(folder);
    pushRecentFolder(folder);
    await scanCurrentSoundboardFolder();
  };

  const pickSoundboardFolder = async (): Promise<void> => {
    const path = await pickSoundboardFolderCmd();
    if (!path) return;
    setSoundboardFolder(path);
    pushRecentFolder(path);
    await scanCurrentSoundboardFolder();
  };

  const setTileColor = (clipId: string, color: string | null): void => {
    if (color === null || color === "") {
      setTileColors(clipId, undefined as unknown as string);
    } else {
      setTileColors(clipId, color);
    }
    // Persist the post-mutation snapshot (un-proxied via spread).
    const snapshot: Record<string, string> = {};
    for (const k of Object.keys(tileColors)) {
      const v = tileColors[k];
      if (v !== undefined) snapshot[k] = v;
    }
    saveJson(STORAGE_KEYS.tileColors, snapshot);
  };

  const reorderTiles = (folder: string, from: number, to: number): void => {
    if (!folder) return;
    // Source list = currently displayed order (sortedTiles when this
    // folder is active, falls back to the saved order or the raw scan
    // for other folders).
    const isCurrent = folder === soundboardFolder();
    const current = isCurrent
      ? sortedTiles().map((t) => t.id)
      : tileOrder[folder] ?? [];
    if (
      from === to ||
      from < 0 ||
      to < 0 ||
      from >= current.length ||
      to >= current.length
    ) {
      return;
    }
    const next = current.slice();
    const [moved] = next.splice(from, 1);
    if (moved !== undefined) next.splice(to, 0, moved);
    setTileOrder(folder, next);
    saveJson(STORAGE_KEYS.tileOrder, { ...tileOrder, [folder]: next });
  };

  const playClip = async (tile: SoundboardTile): Promise<void> => {
    try {
      const durationSecs = await playSoundboardClipCmd(tile.id, tile.path);
      setPlayingClips(tile.id, {
        clipId: tile.id,
        startedAt: typeof performance !== "undefined" ? performance.now() : Date.now(),
        durationSecs,
      });
    } catch (err) {
      setSoundboardError(`play failed: ${String(err)}`);
    }
  };

  const stopClip = async (clipId: string): Promise<void> => {
    try {
      await stopSoundboardClipCmd(clipId);
    } catch (err) {
      // The local "not playing" state is what users see; record the
      // backend failure but don't re-throw — stop should always succeed
      // from the UI's perspective.
      setSoundboardError(`stop failed: ${String(err)}`);
    }
    setPlayingClips(clipId, undefined as unknown as PlayingClip);
  };

  const panicSoundboard = async (): Promise<void> => {
    try {
      await stopAllSoundboardClipsCmd();
    } finally {
      // Replace the whole playing-clips store with an empty record.
      const ids = Object.keys(playingClips);
      for (const id of ids) {
        setPlayingClips(id, undefined as unknown as PlayingClip);
      }
    }
  };

  /**
   * Per-tile hotkey id used when talking to `tauri-plugin-global-
   * shortcut`. The plugin's id namespace is shared between PTM /
   * panic / monitor and every tile, so we prefix tiles to keep them
   * out of the action namespace.
   */
  const tileHotkeyId = (clipId: string): string => `sb:${clipId}`;

  const bindTileHotkey = async (
    clipId: string,
    keys: string[],
  ): Promise<void> => {
    setTileHotkeys(clipId, keys);
    saveJson(STORAGE_KEYS.tileHotkeys, { ...tileHotkeys });
    const accelerator = keys.join("+");
    if (!accelerator) {
      try {
        await unregisterGlobalShortcutCmd(tileHotkeyId(clipId));
      } catch (err) {
        console.warn("[soundboard] unregister hotkey failed", err);
      }
      return;
    }
    try {
      await registerGlobalShortcutCmd(tileHotkeyId(clipId), accelerator);
    } catch (err) {
      console.warn("[soundboard] register hotkey failed", err);
    }
  };

  const clearTileHotkey = async (clipId: string): Promise<void> => {
    setTileHotkeys(clipId, undefined as unknown as string[]);
    saveJson(STORAGE_KEYS.tileHotkeys, { ...tileHotkeys });
    try {
      await unregisterGlobalShortcutCmd(tileHotkeyId(clipId));
    } catch (err) {
      console.warn("[soundboard] unregister hotkey failed", err);
    }
  };

  /** Look up a tile by id from the active scan and fire `playClip`. */
  const playTileById = async (clipId: string): Promise<void> => {
    const tile = soundboardTiles().find((t) => t.id === clipId);
    if (!tile) return;
    await playClip(tile);
  };

  const markClipFinished = (clipId: string): void => {
    setPlayingClips(clipId, undefined as unknown as PlayingClip);
  };

  // Virtual mic (VB-Cable detection).

  const [virtualMicStatus, setVirtualMicStatus] = createSignal<VirtualMicStatus | null>(null);

  const refreshVirtualMicStatus = async (): Promise<void> => {
    try {
      const status = await detectVirtualMicCmd();
      setVirtualMicStatus(status);
    } catch (err) {
      console.warn("[virtual-mic] detect failed", err);
      setVirtualMicStatus(null);
    }
  };

  // Global hotkeys.

  const [hotkeyBindings, setHotkeyBindings] = createStore<Record<HotkeyAction, string>>({
    ...DEFAULT_HOTKEY_BINDINGS,
  });

  const setHotkeyBinding = async (
    action: HotkeyAction,
    accelerator: string,
  ): Promise<void> => {
    setHotkeyBindings(action, accelerator);
    // PTM also drives the in-app fallback listener via ui.ptmKey.
    if (action === "ptm") {
      setUi("ptmKey", accelerator || "Space");
    }
    if (!accelerator) {
      try {
        await unregisterGlobalShortcutCmd(action);
      } catch (err) {
        console.warn("[hotkey] unregister failed", err);
      }
      return;
    }
    try {
      await registerGlobalShortcutCmd(action, accelerator);
    } catch (err) {
      console.warn("[hotkey] register failed", err);
    }
  };

  const clearHotkeyBinding = (action: HotkeyAction): Promise<void> =>
    setHotkeyBinding(action, "");

  const syncHotkeyBindings = async (): Promise<void> => {
    for (const action of Object.keys(hotkeyBindings) as HotkeyAction[]) {
      const accelerator = hotkeyBindings[action];
      if (!accelerator) continue;
      try {
        await registerGlobalShortcutCmd(action, accelerator);
      } catch (err) {
        console.warn(`[hotkey] sync failed for ${action}`, err);
      }
    }
    // Re-register tile hotkeys too — they live in `tileHotkeys` and
    // their `sb:` namespaced ids round-trip back through the
    // global-shortcut listener in App.tsx.
    for (const [clipId, keys] of Object.entries(tileHotkeys)) {
      if (!keys || keys.length === 0) continue;
      const accelerator = keys.join("+");
      try {
        await registerGlobalShortcutCmd(tileHotkeyId(clipId), accelerator);
      } catch (err) {
        console.warn(`[soundboard] sync tile hotkey failed for ${clipId}`, err);
      }
    }
  };

  return {
    nav,
    setNav,
    presets,
    setPresets,
    refreshPresets,
    presetsLoaded,
    presetId,
    setPresetId,
    preset,
    chains,
    setChains,
    chain,
    abSlots,
    setAbSlots,
    setAbSlot,
    resetAbSlots,
    ui,
    setUi,
    wizardOpen,
    setWizardOpen,
    tweaks,
    setTweaks,
    glyphs,
    setGlyphs,

    audioInputs,
    setAudioInputs,
    audioOutputs,
    setAudioOutputs,
    selectedInput,
    setSelectedInput,
    selectedOutput,
    setSelectedOutput,
    engineRunning,
    setEngineRunning,
    engineMonitoring,
    setEngineMonitoring,
    engineError,
    setEngineError,
    streamInfo,
    setStreamInfo,
    inputLevels,
    setInputLevels,
    outputLevels,
    setOutputLevels,

    refreshDevices,
    startEngine,
    stopEngine,
    toggleMonitor,
    setMonitor,

    setChainParam,
    setChainEnabled,
    toggleEffectById,
    syncChain,
    reorderChainEntries,

    usePreset,
    savePreset,
    duplicatePreset,
    deletePreset,
    exportPreset,
    presetWithCurrentChain,

    soundboardFolder,
    setSoundboardFolder,
    soundboardTiles,
    setSoundboardTiles,
    soundboardLoading,
    soundboardError,
    setSoundboardError,
    playingClips,
    setPlayingClips,
    tileHotkeys,
    setTileHotkeys,
    soundboardSearch,
    setSoundboardSearch,
    clockTick,

    pickSoundboardFolder,
    scanCurrentSoundboardFolder,
    playClip,
    stopClip,
    panicSoundboard,
    bindTileHotkey,
    clearTileHotkey,
    markClipFinished,
    playTileById,

    // Phase 8 tile metadata
    tileColors,
    setTileColor,
    tileOrder,
    reorderTiles,
    sortedTiles,

    // Phase 8 recent folders
    recentFolders,
    pushRecentFolder,
    removeRecentFolder,
    useRecentFolder,

    virtualMicStatus,
    setVirtualMicStatus,
    refreshVirtualMicStatus,

    hotkeyBindings,
    setHotkeyBindings,
    setHotkeyBinding,
    clearHotkeyBinding,
    syncHotkeyBindings,

    selectedEffect,
    setSelectedEffect,

    hasEnabled,
    effectiveModulated,
    status,
  };
}

const AppContext = createContext<AppState>();

export function AppProvider(props: { children: JSX.Element }): JSX.Element {
  const state = createAppState();
  return (
    <AppContext.Provider value={state}>{props.children}</AppContext.Provider>
  );
}

export function useApp(): AppState {
  const ctx = useContext(AppContext);
  if (!ctx) throw new Error("useApp must be used within <AppProvider>");
  return ctx;
}

// Re-exports for convenience.
export type {
  AbSlot,
  ChainEntry,
  GlyphId,
  NavId,
  Preset,
  PtmMode,
  TweaksState,
  UiState,
  VoiceStatus,
};
