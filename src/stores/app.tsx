// Top-level app state for DivoraVoice.
//
// Mirrors the prototype's `app.jsx` state shape (presetId, chains, ui,
// tweaks, glyphs) using SolidJS signals + stores. Phase 2 adds the
// audio engine signals (device lists, selected devices, engine status,
// live levels, last error) and wraps the Tauri command surface.

import {
  createContext,
  createMemo,
  createSignal,
  type JSX,
  type Setter,
  useContext,
} from "solid-js";
import { createStore, type SetStoreFunction } from "solid-js/store";
import {
  ZERO_LEVELS,
  listInputDevices,
  listOutputDevices,
  setAudioMonitor as setAudioMonitorCmd,
  startAudioEngine,
  stopAudioEngine,
  type DeviceInfo,
  type Levels,
  type StreamInfo,
} from "../audio/api";
import { PRESETS } from "../data/presets";
import type {
  AbSlot,
  ChainEntry,
  GlyphId,
  NavId,
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

const initialChains = (): Record<string, ChainEntry[]> =>
  Object.fromEntries(
    PRESETS.map((p) => [
      p.id,
      p.chain.map((c) => ({ ...c, vals: { ...c.vals } })),
    ]),
  );

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

export interface AppState {
  // Nav
  nav: () => NavId;
  setNav: Setter<NavId>;

  // Active preset + chain
  presetId: () => string;
  setPresetId: Setter<string>;
  preset: () => Preset;
  chains: Record<string, ChainEntry[]>;
  setChains: SetStoreFunction<Record<string, ChainEntry[]>>;
  chain: () => ChainEntry[];

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

  // Derived
  hasEnabled: () => boolean;
  effectiveModulated: () => boolean;
  status: () => VoiceStatus;
}

export function createAppState(): AppState {
  const [nav, setNav] = createSignal<NavId>("mixer");
  const firstPreset = PRESETS[0];
  if (!firstPreset) throw new Error("PRESETS must contain at least one preset");
  const [presetId, setPresetId] = createSignal<string>(firstPreset.id);
  const [chains, setChains] = createStore<Record<string, ChainEntry[]>>(initialChains());
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
    () => PRESETS.find((p) => p.id === presetId()) ?? firstPreset,
  );
  const chain = createMemo<ChainEntry[]>(() => chains[presetId()] ?? []);

  const hasEnabled = createMemo(() => chain().some((c) => c.enabled));
  const effectiveModulated = createMemo(() =>
    ui.ptmMode === "apply" ? ui.pressed : !ui.pressed,
  );
  const status = createMemo<VoiceStatus>(() =>
    ui.muted ? "muted" : hasEnabled() && effectiveModulated() ? "modulated" : "clean",
  );

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

  return {
    nav,
    setNav,
    presetId,
    setPresetId,
    preset,
    chains,
    setChains,
    chain,
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
