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
  exportPresetJson as exportPresetJsonCmd,
  listInputDevices,
  listOutputDevices,
  listPresets,
  pickSoundboardFolder as pickSoundboardFolderCmd,
  playSoundboardClip as playSoundboardClipCmd,
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
  type DeviceInfo,
  type EffectSpec,
  type Levels,
  type SoundboardTile,
  type StreamInfo,
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

const defaultTweaks = (): TweaksState => ({
  mystical: 1,
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

  // Soundboard actions
  pickSoundboardFolder: () => Promise<void>;
  scanCurrentSoundboardFolder: () => Promise<void>;
  playClip: (tile: SoundboardTile) => Promise<void>;
  stopClip: (clipId: string) => Promise<void>;
  panicSoundboard: () => Promise<void>;
  bindTileHotkey: (clipId: string, keys: string[]) => void;
  clearTileHotkey: (clipId: string) => void;
  /** Triggered by SoundboardScreen when a tile finishes naturally. */
  markClipFinished: (clipId: string) => void;

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
  const [tweaks, setTweaks] = createStore<TweaksState>(defaultTweaks());
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
  const [tileHotkeys, setTileHotkeys] = createStore<Record<string, string[]>>({});
  const [soundboardSearch, setSoundboardSearch] = createSignal("");
  const [clockTick, setClockTick] = createSignal(0);

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

  const pickSoundboardFolder = async (): Promise<void> => {
    const path = await pickSoundboardFolderCmd();
    if (!path) return;
    setSoundboardFolder(path);
    await scanCurrentSoundboardFolder();
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

  const bindTileHotkey = (clipId: string, keys: string[]): void => {
    setTileHotkeys(clipId, keys);
  };

  const clearTileHotkey = (clipId: string): void => {
    setTileHotkeys(clipId, undefined as unknown as string[]);
  };

  const markClipFinished = (clipId: string): void => {
    setPlayingClips(clipId, undefined as unknown as PlayingClip);
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
