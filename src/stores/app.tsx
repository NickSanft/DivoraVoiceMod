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
import { createStore, reconcile, type SetStoreFunction } from "solid-js/store";
import {
  ZERO_LEVELS,
  deleteUserPreset as deleteUserPresetCmd,
  detectVirtualMic as detectVirtualMicCmd,
  exportPresetJson as exportPresetJsonCmd,
  importPreset as importPresetCmd,
  listInputDevices,
  listMidiInputs as listMidiInputsCmd,
  listOutputDevices,
  listPresets,
  listVoices as listVoicesCmd,
  listTtsVoices as listTtsVoicesCmd,
  openMidiInput as openMidiInputCmd,
  closeMidiInput as closeMidiInputCmd,
  onnxRuntimeStatus as onnxRuntimeStatusCmd,
  pickSoundboardFolder as pickSoundboardFolderCmd,
  playSoundboardClip as playSoundboardClipCmd,
  recordingsDir as recordingsDirCmd,
  logsDir as logsDirCmd,
  registerGlobalShortcut as registerGlobalShortcutCmd,
  saveUserPreset as saveUserPresetCmd,
  scanSoundboardFolder as scanSoundboardFolderCmd,
  setAudioMonitor as setAudioMonitorCmd,
  setMonitorGain as setMonitorGainCmd,
  setLoudnessEnabled as setLoudnessEnabledCmd,
  setLoudnessTarget as setLoudnessTargetCmd,
  setEffectChain as setEffectChainCmd,
  setEffectEnabled as setEffectEnabledCmd,
  setEffectParam as setEffectParamCmd,
  setSoundboardMasterGain as setSoundboardMasterGainCmd,
  setVoiceModel as setVoiceModelCmd,
  startAudioEngine,
  startRecording as startRecordingCmd,
  stopAllSoundboardClips as stopAllSoundboardClipsCmd,
  stopAudioEngine,
  stopRecording as stopRecordingCmd,
  stopSoundboardClip as stopSoundboardClipCmd,
  speak as speakCmd,
  stopSpeak as stopSpeakCmd,
  unregisterGlobalShortcut as unregisterGlobalShortcutCmd,
  type DeviceInfo,
  type EffectSpec,
  type Levels,
  type MidiInputInfo,
  type MidiMessage,
  type OnnxRuntimeStatus,
  type SoundboardTile,
  type StreamInfo,
  type TtsVoiceInfo,
  type VirtualMicStatus,
  type VoiceInfo,
  type WirePreset,
} from "../audio/api";
import {
  computeCalibration,
  type CalibrationResult,
} from "../audio/calibration";
import { checkForUpdate, type UpdateInfo } from "../data/updates";
import { evaluateDiagnostics, type DiagCheck } from "../audio/diagnostics";
import {
  makeTemplate,
  recognize as recognizeUnistroke,
  type UnistrokeTemplate,
} from "../data/unistroke";
import type { Point } from "../data/glyphs";
import type { OverlayBg } from "../overlay/state";
import { FALLBACK_PRESETS, presetFromWire } from "../data/presets";
import { routeMidi, triggerFor, type MidiHandlers } from "../midi/router";
import type {
  AbSlot,
  ChainEntry,
  CustomGlyph,
  EffectId,
  GlyphAction,
  GlyphId,
  MidiMapping,
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
  theme: "dark",
  mood: "violet",
  accent: "brand",
  grain: false,
  vignette: false,
});

/** v1.15.0: built-in glyphs default to their classic preset bindings, now
 *  expressed as `GlyphAction`s (any glyph can bind to any action). */
const defaultGlyphBindings = (): Record<string, GlyphAction> => ({
  triangle: { kind: "preset", presetId: "velvet-demon" },
  invtriangle: { kind: "preset", presetId: "glass-oracle" },
  square: { kind: "preset", presetId: "hollow-king" },
  circle: { kind: "preset", presetId: "clean" },
});

/** The four built-in shapes, in display order (for the Settings list). */
export const BUILTIN_GLYPHS: GlyphId[] = [
  "triangle",
  "invtriangle",
  "square",
  "circle",
];

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
  activeVoice: "divora.activeVoice",
  inputDevice: "divora.inputDevice",
  outputDevice: "divora.outputDevice",
  monitorDevice: "divora.monitorDevice",
  monitorGain: "divora.monitorGain",
  loudnessEnabled: "divora.loudnessEnabled",
  loudnessTarget: "divora.loudnessTarget",
  soundboardFolder: "divora.soundboardFolder",
  tileGains: "divora.tileGains",
  soundboardMasterGain: "divora.soundboardMasterGain",
  midiInput: "divora.midiInput",
  midiMappings: "divora.midiMappings",
  calibrated: "divora.calibrated",
  updateCheckEnabled: "divora.updateCheckEnabled",
  glyphBindings: "divora.glyphBindings",
  customGlyphs: "divora.customGlyphs",
  overlay: "divora.overlay",
  ttsVoice: "divora.ttsVoice",
  ttsVolume: "divora.ttsVolume",
  ttsPreviewOnly: "divora.ttsPreviewOnly",
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

  // Active preset + chain (the live voice — Mixer + engine)
  presetId: () => string;
  setPresetId: Setter<string>;
  preset: () => Preset;
  chains: Record<string, ChainEntry[]>;
  setChains: SetStoreFunction<Record<string, ChainEntry[]>>;
  chain: () => ChainEntry[];
  // Viewed preset + chain (the Presets editor's selection — may differ
  // from the active one while browsing; applied via `usePreset`).
  viewedId: () => string;
  viewedPreset: () => Preset;
  viewedChain: () => ChainEntry[];

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
  // Glyph casting (v1.15.0 — any glyph → any action + custom glyphs)
  /** Glyph id (built-in `GlyphId` or a custom id) → the action it casts. */
  glyphBindings: Record<string, GlyphAction>;
  /** Bind a glyph to an action (persisted to `divora.glyphBindings`). */
  setGlyphBinding: (glyphId: string, action: GlyphAction) => void;
  /** User-recorded custom glyphs (persisted to `divora.customGlyphs`). */
  customGlyphs: () => CustomGlyph[];
  addCustomGlyph: (glyph: CustomGlyph) => void;
  removeCustomGlyph: (id: string) => void;
  setCustomGlyphAction: (id: string, action: GlyphAction) => void;
  /** Resolve a recognised glyph id → cast outcome (canvas colour + label +
   *  the bound action), or null when unbound / its target is gone. */
  glyphOutcome: (
    glyphId: string,
  ) => { color: string; label: string; action: GlyphAction } | null;
  /** Run a glyph's action against the live store. */
  dispatchGlyphAction: (action: GlyphAction) => void;
  /** Match a drawn path against the custom-glyph templates (built-ins use
   *  the geometric `detectShape`). */
  recognizeCustomGlyph: (
    path: Point[],
  ) => { id: string; template: Point[] } | null;

  // Stream overlay (v1.16.0)
  /** Whether the transparent spell-circle overlay window is open. */
  overlayOpen: () => boolean;
  /** Overlay background mode (persisted): alpha, or a chroma colour. */
  overlayBg: () => OverlayBg;
  setOverlayBg: (bg: OverlayBg) => void;
  /** Open / close the overlay window (a second transparent Tauri window). */
  openOverlay: () => Promise<void>;
  closeOverlay: () => Promise<void>;

  // Audio engine
  audioInputs: () => DeviceInfo[];
  setAudioInputs: Setter<DeviceInfo[]>;
  audioOutputs: () => DeviceInfo[];
  setAudioOutputs: Setter<DeviceInfo[]>;
  selectedInput: () => string | null;
  setSelectedInput: (name: string | null) => void;
  selectedOutput: () => string | null;
  setSelectedOutput: (name: string | null) => void;
  /** Phase 13: separate monitor output device (null = none). */
  selectedMonitor: () => string | null;
  setSelectedMonitor: (name: string | null) => void;
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
  /** Phase 14: latency added by the active DSP chain, in ms. */
  dspLatencyMs: () => number;
  setDspLatencyMs: Setter<number>;
  /** Phase 16: true while the modulated output is being recorded. */
  isRecording: () => boolean;
  setIsRecording: Setter<boolean>;
  /** Phase 16: full path of the most recent recording (for the hint). */
  recordingPath: () => string | null;

  // Audio actions
  refreshDevices: () => Promise<void>;
  startEngine: () => Promise<void>;
  stopEngine: () => Promise<void>;
  toggleMonitor: () => Promise<void>;
  setMonitor: (enabled: boolean) => Promise<void>;
  /** v1.6.0: monitor ("hear yourself") volume — linear gain, 1.0 = unity. */
  monitorGain: () => number;
  setMonitorGain: (gain: number) => void;
  /** v1.7.0: output loudness normalization (auto-gain + limiter). */
  loudnessEnabled: () => boolean;
  setLoudnessEnabled: (enabled: boolean) => void;
  /** v1.7.0: loudness target level, dBFS (clamped to [-30, -6]). */
  loudnessTarget: () => number;
  setLoudnessTarget: (dbfs: number) => void;
  /** v1.7.0: live makeup gain the normalizer is applying, in dB. */
  loudnessGainDb: () => number;
  setLoudnessGainDb: (db: number) => void;
  /** Phase 16: toggle recording the modulated output to a WAV file. */
  toggleRecording: () => Promise<void>;
  /** Phase 16: absolute path of the recordings directory. */
  getRecordingsDir: () => Promise<string>;
  /** v1.14.0: absolute path of the logs directory (for "Open logs folder"). */
  getLogsDir: () => Promise<string>;

  // DSP / chain editing — local store mutation + backend sync.
  setChainParam: (effectIndex: number, key: string, value: number) => void;
  setChainEnabled: (effectIndex: number, enabled: boolean) => void;
  toggleEffectById: (id: EffectId) => void;
  syncChain: () => void;
  reorderChainEntries: (from: number, to: number) => void;

  // Voice library (Phase 12)
  voices: () => VoiceInfo[];
  onnxStatus: () => OnnxRuntimeStatus | null;
  activeVoiceId: () => string | null;
  setActiveVoice: (id: string | null) => void;
  refreshVoiceLibrary: () => Promise<void>;

  // Text-to-speech ("Speak") — v1.17.0
  /** Preset Speak voices (each flagged `installed`). */
  ttsVoices: () => TtsVoiceInfo[];
  /** Selected preset voice id (persisted `divora.ttsVoice`); null until first load. */
  selectedTtsVoice: () => string | null;
  setSelectedTtsVoice: (id: string) => void;
  /** Speak playback volume, linear 0..2 (persisted `divora.ttsVolume`). */
  ttsVolume: () => number;
  setTtsVolume: (gain: number) => void;
  /** v1.18.0: when true, Speak plays to the local monitor only (you hear it,
   *  the call doesn't). Persisted `divora.ttsPreviewOnly`. */
  ttsPreviewOnly: () => boolean;
  setTtsPreviewOnly: (on: boolean) => void;
  /** Draft text in the Speak box (ephemeral — not persisted). */
  ttsText: () => string;
  setTtsText: (text: string) => void;
  /** True while a synthesize request is in flight. */
  synthesizing: () => boolean;
  /** Last synthesis error (e.g. "voices not installed"), or null. */
  ttsError: () => string | null;
  /** Pull the preset voice list from the backend + default the selection. */
  refreshTtsVoices: () => Promise<void>;
  /** Synthesize the current text with the selected voice and play it. */
  speakText: () => Promise<void>;
  /** Stop any in-flight synthesized speech. */
  stopSpeaking: () => Promise<void>;

  // Preset actions
  usePreset: (id: string) => void;
  /** Preview a preset in the Presets editor without making it live. */
  viewPreset: (id: string) => void;
  /** The Coven: apply a cast member's preset + (un)set its conversion model. */
  summon: (presetId: string, modelId?: string | null) => void;
  savePreset: (preset: Preset) => Promise<void>;
  duplicatePreset: (sourceId: string) => Promise<Preset | null>;
  deletePreset: (id: string) => Promise<void>;
  exportPreset: (preset: Preset) => Promise<string>;
  /** v1.14.0: import a preset `.json` from disk (path), refresh the list,
   *  and return the saved (User) preset. Throws on a bad file. */
  importPreset: (path: string) => Promise<Preset>;
  /** Snapshot the current `chain()` into the active preset's record. Used by Save. */
  presetWithCurrentChain: (id: string) => Preset | null;

  // Soundboard
  soundboardFolder: () => string | null;
  setSoundboardFolder: (folder: string | null) => void;
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
  /** Phase 15: per-tile + master soundboard volume (linear gain). */
  tileGain: (clipId: string) => number;
  setTileGain: (clipId: string, gain: number) => void;
  soundboardMasterGain: () => number;
  setSoundboardMasterGain: (gain: number) => void;
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

  // MIDI control surfaces (v1.9.0)
  /** Available MIDI input ports (display names). */
  midiInputs: () => MidiInputInfo[];
  refreshMidiInputs: () => Promise<void>;
  /** The currently-open MIDI input name, or null when disconnected. */
  selectedMidiInput: () => string | null;
  /** Open (name) or close (null) a MIDI input port; persists the choice. */
  selectMidiInput: (name: string | null) => Promise<void>;
  /** User's note/CC → action mappings (persisted). */
  midiMappings: () => MidiMapping[];
  addMidiMapping: (mapping: Omit<MidiMapping, "id">) => void;
  updateMidiMapping: (id: string, patch: Partial<MidiMapping>) => void;
  removeMidiMapping: (id: string) => void;
  /** The mapping id currently capturing its trigger via MIDI-learn, or null. */
  midiLearnId: () => string | null;
  setMidiLearnId: (id: string | null) => void;
  /** Feed a `midi-message` in: learns a trigger if armed, else routes it. */
  handleMidiMessage: (msg: MidiMessage) => void;

  // Guided mic calibration (v1.10.0)
  /** True once the user has run mic calibration (persisted; suppresses the
   *  wizard's calibration step). */
  calibrated: () => boolean;
  setCalibrated: (value: boolean) => void;
  /** True while a "stay quiet" measurement is in progress. */
  calibrating: () => boolean;
  /** The last calibration result, or null if never run this session. */
  calibrationResult: () => CalibrationResult | null;
  /** Measure the noise floor over `durationMs` of input levels, then set
   *  the active chain's gate (and denoiser when the floor is high). */
  runCalibration: (durationMs?: number) => Promise<CalibrationResult>;
  /** Apply a calibration result to the active chain's gate / denoiser. */
  applyCalibration: (result: CalibrationResult) => void;

  // In-app update check (v1.12.0)
  /** Opt-out for the lightweight GitHub-release update check (persisted,
   *  default on). No telemetry — a one-way version read only. */
  updateCheckEnabled: () => boolean;
  setUpdateCheckEnabled: (value: boolean) => void;
  /** The newer release found by the last check, or null. */
  updateAvailable: () => UpdateInfo | null;
  /** Clear the "update available" notice (user dismissed it). */
  dismissUpdate: () => void;
  /** True while a check is in flight. */
  updateChecking: () => boolean;
  /** Run the update check. No-op when disabled, in a dev build, or off
   *  Tauri (browser / E2E never hit the network). */
  checkUpdates: () => Promise<void>;

  // Setup diagnostic (v1.13.0)
  /** Last "Test my setup" result, or null if never run. */
  diagnostics: () => DiagCheck[] | null;
  /** True while a diagnostic is running. */
  diagnosing: () => boolean;
  /** Run the routing-chain diagnostic. Refreshes devices + VB-Cable,
   *  briefly starts the engine if stopped (and restores that state),
   *  then evaluates a green/amber/red checklist. */
  runDiagnostics: () => Promise<DiagCheck[]>;

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
  // `presetId` is the ACTIVE / live preset — what the engine runs and the
  // Mixer shows. `viewedId` is what the Presets editor shows + edits;
  // clicking a row in the Presets list only changes `viewedId` (a
  // preview), and the "Use" button applies it (active = viewed). The two
  // are kept in sync everywhere except the Presets screen (see the nav
  // resync effect below), so the Mixer always edits the active preset.
  const [presetId, setPresetId] = createSignal<string>(firstPreset.id);
  const [viewedId, setViewedId] = createSignal<string>(firstPreset.id);
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
  // Glyph casting (v1.15.0). Bindings (built-in + custom) and the custom
  // templates persist; SparkLayer/MixerScreen recognise + dispatch through
  // `recognizeCustomGlyph` / `glyphOutcome` / `dispatchGlyphAction`.
  const [glyphBindings, setGlyphBindingsRaw] = createStore<
    Record<string, GlyphAction>
  >({
    ...defaultGlyphBindings(),
    ...loadJson<Record<string, GlyphAction>>(STORAGE_KEYS.glyphBindings, {}),
  });
  const setGlyphBinding = (glyphId: string, action: GlyphAction): void => {
    // `reconcile` REPLACES the value — a plain-object set (even via a
    // function) *merges* into the previous action, leaving stale fields
    // like `presetId` when switching a glyph to a non-preset action.
    setGlyphBindingsRaw(glyphId, reconcile(action));
    saveJson(STORAGE_KEYS.glyphBindings, { ...glyphBindings });
  };
  const [customGlyphs, setCustomGlyphs] = createSignal<CustomGlyph[]>(
    loadJson<CustomGlyph[]>(STORAGE_KEYS.customGlyphs, []),
  );
  const persistCustomGlyphs = (next: CustomGlyph[]): void => {
    setCustomGlyphs(next);
    saveJson(STORAGE_KEYS.customGlyphs, next);
  };
  const addCustomGlyph = (glyph: CustomGlyph): void => {
    persistCustomGlyphs([...customGlyphs(), glyph]);
  };
  const removeCustomGlyph = (id: string): void => {
    persistCustomGlyphs(customGlyphs().filter((g) => g.id !== id));
  };
  const setCustomGlyphAction = (id: string, action: GlyphAction): void => {
    persistCustomGlyphs(
      customGlyphs().map((g) => (g.id === id ? { ...g, action } : g)),
    );
  };

  const dispatchGlyphAction = (action: GlyphAction): void => {
    switch (action.kind) {
      case "preset":
        usePreset(action.presetId);
        break;
      case "tile":
        void playTileById(action.tileId);
        break;
      case "monitor":
        void toggleMonitor();
        break;
      case "mute":
        setUi("muted", !ui.muted);
        break;
      case "panic":
        void panicSoundboard();
        break;
    }
  };

  // Canvas-drawn omen colour for non-preset actions (a hex, since the
  // cast canvas can't resolve CSS vars).
  const GLYPH_ACCENT = "#7C5CF6";
  const glyphOutcome = (
    glyphId: string,
  ): { color: string; label: string; action: GlyphAction } | null => {
    // Custom glyphs own their action; built-ins use `glyphBindings`.
    const custom = customGlyphs().find((g) => g.id === glyphId);
    const action = custom ? custom.action : glyphBindings[glyphId];
    if (!action) return null;
    switch (action.kind) {
      case "preset": {
        const p = presets().find((x) => x.id === action.presetId);
        return p ? { color: p.color, label: p.name, action } : null;
      }
      case "tile": {
        const t = soundboardTiles().find((x) => x.id === action.tileId);
        return { color: GLYPH_ACCENT, label: t ? `▶ ${t.label}` : "▶ Clip", action };
      }
      case "monitor":
        return { color: GLYPH_ACCENT, label: "Monitor", action };
      case "mute":
        return { color: GLYPH_ACCENT, label: "Mute", action };
      case "panic":
        return { color: GLYPH_ACCENT, label: "Panic", action };
    }
  };

  const recognizeCustomGlyph = (
    path: Point[],
  ): { id: string; template: Point[] } | null => {
    const glyphs = customGlyphs();
    if (glyphs.length === 0) return null;
    const templates: UnistrokeTemplate[] = glyphs.map((g) => ({
      id: g.id,
      points: g.template,
    }));
    const m = recognizeUnistroke(path, templates);
    if (!m) return null;
    const g = glyphs.find((x) => x.id === m.id);
    return g ? { id: g.id, template: g.template } : null;
  };

  // Stream overlay (v1.16.0). A second transparent Tauri window renders
  // the spell circle for OBS; the main window pushes state to it (App.tsx).
  const [overlayOpen, setOverlayOpen] = createSignal(false);
  const [overlayBg, setOverlayBgRaw] = createSignal<OverlayBg>(
    loadJson<{ bg: OverlayBg }>(STORAGE_KEYS.overlay, { bg: "transparent" }).bg,
  );
  const setOverlayBg = (bg: OverlayBg): void => {
    setOverlayBgRaw(bg);
    saveJson(STORAGE_KEYS.overlay, { bg });
  };
  const openOverlay = async (): Promise<void> => {
    try {
      const { WebviewWindow } = await import("@tauri-apps/api/webviewWindow");
      const existing = await WebviewWindow.getByLabel("overlay");
      if (existing) {
        setOverlayOpen(true);
        return;
      }
      const w = new WebviewWindow("overlay", {
        url: "overlay.html",
        title: "DivoraVoice Overlay",
        width: 480,
        height: 480,
        transparent: true,
        decorations: false,
        alwaysOnTop: true,
        resizable: true,
      });
      void w.once("tauri://created", () => setOverlayOpen(true));
      void w.once("tauri://error", () => setOverlayOpen(false));
      void w.once("tauri://destroyed", () => setOverlayOpen(false));
    } catch {
      /* not under Tauri (browser / E2E) — overlay is desktop-only */
    }
  };
  const closeOverlay = async (): Promise<void> => {
    try {
      const { WebviewWindow } = await import("@tauri-apps/api/webviewWindow");
      const w = await WebviewWindow.getByLabel("overlay");
      if (w) await w.close();
    } catch {
      /* ignore */
    }
    setOverlayOpen(false);
  };

  // Audio engine signals.
  const [audioInputs, setAudioInputs] = createSignal<DeviceInfo[]>([]);
  const [audioOutputs, setAudioOutputs] = createSignal<DeviceInfo[]>([]);
  // Persisted so device choices survive a restart. Restored here;
  // refreshDevices() validates the saved name still exists and falls
  // back to the host default if it was unplugged.
  const [selectedInput, setSelectedInputRaw] = createSignal<string | null>(
    loadJson<string | null>(STORAGE_KEYS.inputDevice, null),
  );
  const setSelectedInput = (name: string | null): void => {
    setSelectedInputRaw(name);
    saveJson(STORAGE_KEYS.inputDevice, name);
  };
  const [selectedOutput, setSelectedOutputRaw] = createSignal<string | null>(
    loadJson<string | null>(STORAGE_KEYS.outputDevice, null),
  );
  const setSelectedOutput = (name: string | null): void => {
    setSelectedOutputRaw(name);
    saveJson(STORAGE_KEYS.outputDevice, name);
  };
  // Phase 13: optional separate monitor ("hear yourself") output device.
  // null = no separate monitor (monitoring rides the main output, as
  // before). Persisted because it's an explicit opt-in the user
  // shouldn't have to re-pick each launch.
  const [selectedMonitorRaw, setSelectedMonitorRaw] = createSignal<string | null>(
    loadJson<string | null>(STORAGE_KEYS.monitorDevice, null),
  );
  const selectedMonitor = selectedMonitorRaw;
  const setSelectedMonitor = (name: string | null): void => {
    setSelectedMonitorRaw(name);
    saveJson(STORAGE_KEYS.monitorDevice, name);
  };
  // v1.6.0: monitor ("hear yourself") volume, linear gain (1.0 = unity).
  // Persisted + re-applied on engine start (a fresh session resets the
  // engine's gain to unity). Sent live so the slider moves it instantly.
  const [monitorGain, setMonitorGainRaw] = createSignal<number>(
    loadJson<number>(STORAGE_KEYS.monitorGain, 1.0),
  );
  const setMonitorGain = (gain: number): void => {
    const g = Math.min(Math.max(gain, 0), 4);
    setMonitorGainRaw(g);
    saveJson(STORAGE_KEYS.monitorGain, g);
    void setMonitorGainCmd(g);
  };
  // v1.7.0: output loudness normalization (auto-gain + limiter). Opt-in;
  // persisted + re-applied on engine start (a fresh session resets the
  // engine to disabled/default-target). Sent live so the controls move it
  // instantly. The target is dBFS, clamped to the engine's [-30, -6] window.
  const LOUDNESS_TARGET_MIN = -30;
  const LOUDNESS_TARGET_MAX = -6;
  const LOUDNESS_TARGET_DEFAULT = -18;
  const [loudnessEnabled, setLoudnessEnabledRaw] = createSignal<boolean>(
    loadJson<boolean>(STORAGE_KEYS.loudnessEnabled, false),
  );
  const setLoudnessEnabled = (enabled: boolean): void => {
    setLoudnessEnabledRaw(enabled);
    saveJson(STORAGE_KEYS.loudnessEnabled, enabled);
    void setLoudnessEnabledCmd(enabled);
  };
  const [loudnessTarget, setLoudnessTargetRaw] = createSignal<number>(
    loadJson<number>(STORAGE_KEYS.loudnessTarget, LOUDNESS_TARGET_DEFAULT),
  );
  const setLoudnessTarget = (dbfs: number): void => {
    const t = Math.min(Math.max(dbfs, LOUDNESS_TARGET_MIN), LOUDNESS_TARGET_MAX);
    setLoudnessTargetRaw(t);
    saveJson(STORAGE_KEYS.loudnessTarget, t);
    void setLoudnessTargetCmd(t);
  };
  // Live makeup-gain readout (dB), driven by the ~30 Hz level event.
  const [loudnessGainDb, setLoudnessGainDb] = createSignal(0);
  const [engineRunning, setEngineRunning] = createSignal(false);
  const [engineMonitoring, setEngineMonitoring] = createSignal(true);
  const [engineError, setEngineError] = createSignal<string | null>(null);
  const [streamInfo, setStreamInfo] = createSignal<StreamInfo | null>(null);
  const [inputLevels, setInputLevels] = createSignal<Levels>(ZERO_LEVELS);
  const [outputLevels, setOutputLevels] = createSignal<Levels>(ZERO_LEVELS);
  // Phase 14: latency added by the active DSP chain (ms), driven by the
  // ~30 Hz level event so it moves live as effects toggle.
  const [dspLatencyMs, setDspLatencyMs] = createSignal(0);
  // Phase 16: recording state. `isRecording` is confirmed by the ~30 Hz
  // level event (backend is source of truth), but we also set it
  // optimistically on toggle for instant button feedback.
  const [isRecording, setIsRecording] = createSignal(false);
  const [recordingPath, setRecordingPath] = createSignal<string | null>(null);

  const preset = createMemo<Preset>(
    () => presets().find((p) => p.id === presetId()) ?? presets()[0] ?? firstPreset,
  );
  const chain = createMemo<ChainEntry[]>(() => chains[presetId()] ?? []);

  // The preset the Presets editor shows + edits (the "viewed" one, which
  // may differ from the active one while browsing the list).
  const viewedPreset = createMemo<Preset>(
    () => presets().find((p) => p.id === viewedId()) ?? preset(),
  );
  const viewedChain = createMemo<ChainEntry[]>(() => chains[viewedId()] ?? []);

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
    // Pick a default when nothing is selected OR the restored selection
    // is no longer present (device unplugged since last run). Never
    // clobber a valid saved choice, and don't reset on a transiently
    // empty enumeration.
    const curIn = selectedInput();
    if ((!curIn || !ins.some((d) => d.name === curIn)) && ins.length > 0) {
      const def = ins.find((d) => d.isDefault) ?? ins[0];
      if (def) setSelectedInput(def.name);
    }
    const curOut = selectedOutput();
    if ((!curOut || !outs.some((d) => d.name === curOut)) && outs.length > 0) {
      const def = outs.find((d) => d.isDefault) ?? outs[0];
      if (def) setSelectedOutput(def.name);
    }
  };

  const startEngine = async (): Promise<void> => {
    try {
      const info = await startAudioEngine(
        selectedInput(),
        selectedOutput(),
        selectedMonitor(),
      );
      setStreamInfo(info);
      setEngineRunning(true);
      setEngineError(null);
      // Phase 15: a fresh engine session builds a new SoundboardMixer
      // (master gain = unity), so re-apply the persisted master gain.
      if (soundboardMasterGain() !== 1.0) {
        void setSoundboardMasterGainCmd(soundboardMasterGain());
      }
      // v1.6.0: push the persisted monitor volume to the engine (a fresh
      // app launch starts at unity until told otherwise).
      if (monitorGain() !== 1.0) {
        void setMonitorGainCmd(monitorGain());
      }
      // v1.7.0: a fresh engine starts with loudness normalization off at the
      // default target, so re-apply the persisted settings. Always push the
      // target (cheap) and only enable when the user had it on.
      void setLoudnessTargetCmd(loudnessTarget());
      if (loudnessEnabled()) {
        void setLoudnessEnabledCmd(true);
      }
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
    // Stopping the engine tears down the recording writer, so the
    // backend already finalized any open file — reflect that at once.
    setIsRecording(false);
  };

  const setMonitor = async (enabled: boolean): Promise<void> => {
    await setAudioMonitorCmd(enabled);
    setEngineMonitoring(enabled);
  };

  const toggleMonitor = (): Promise<void> => setMonitor(!engineMonitoring());

  // Phase 16: toggle recording the modulated output to a WAV file.
  // Starting builds a timestamped filename; the backend sanitizes it,
  // forces a `.wav` extension, and returns the final destination path.
  // The writer thread only exists while the engine is running, so we
  // refuse to start when stopped (surfaced via engineError).
  const toggleRecording = async (): Promise<void> => {
    if (isRecording()) {
      setIsRecording(false); // optimistic; confirmed by the level event
      try {
        await stopRecordingCmd();
      } catch (err) {
        console.warn("[recording] stop failed", err);
      }
      return;
    }
    if (!engineRunning()) {
      setEngineError("Start the engine before recording.");
      return;
    }
    const pad = (n: number): string => String(n).padStart(2, "0");
    const d = new Date();
    const filename =
      `divora-${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}` +
      `_${pad(d.getHours())}-${pad(d.getMinutes())}-${pad(d.getSeconds())}.wav`;
    try {
      const dest = await startRecordingCmd(filename);
      setRecordingPath(dest);
      setIsRecording(true); // optimistic; confirmed by the level event
    } catch (err) {
      setIsRecording(false);
      setEngineError(String(err));
    }
  };

  const getRecordingsDir = (): Promise<string> => recordingsDirCmd();
  const getLogsDir = (): Promise<string> => logsDirCmd();

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
      [selectedInput, selectedOutput, selectedMonitor],
      ([newIn, newOut, newMon], prev) => {
        // `defer: true` already skips the on-mount run, so any call here is
        // a real change. Solid leaves `prev` undefined on the FIRST
        // post-mount change (it doesn't record a prevInput during the
        // deferred run) — treat that as a change too, otherwise the first
        // device switch of the session is silently swallowed and only the
        // second takes effect. Only dedupe a genuine no-op (prev present
        // and all three equal).
        if (prev) {
          const [prevIn, prevOut, prevMon] = prev;
          if (newIn === prevIn && newOut === prevOut && newMon === prevMon) {
            return;
          }
        }
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
    applyActiveVoice();
  };

  // Send a full SetChain when either the active preset changes or the
  // engine starts up.
  createEffect(
    on([presetId, engineRunning], ([, running]) => {
      if (running) {
        void setEffectChainCmd(chainToSpecs(chain()));
        // A SetChain rebuilds every effect from scratch — including the
        // VoiceConvert, which comes back with no model. Re-point it at
        // the active voice so the selection survives preset switches.
        applyActiveVoice();
      }
    }, { defer: true }),
  );

  // ---- Phase 12: voice library ----

  const [voices, setVoices] = createSignal<VoiceInfo[]>([]);
  const [onnxStatus, setOnnxStatus] = createSignal<OnnxRuntimeStatus | null>(
    null,
  );
  const [activeVoiceId, setActiveVoiceIdRaw] = createSignal<string | null>(
    loadJson<string | null>(STORAGE_KEYS.activeVoice, null),
  );

  /** Path of the active voice, looked up in the current `voices` list. */
  const activeVoicePath = (): string | null => {
    const id = activeVoiceId();
    if (!id) return null;
    return voices().find((v) => v.id === id)?.path ?? null;
  };

  /** Index of the (first) VoiceConvert effect in the live chain, or -1. */
  const voiceConvertIndex = (): number =>
    chain().findIndex((c) => c.id === "voice_convert");

  /**
   * Push the active voice's model path to the backend's VoiceConvert
   * effect. No-op when the engine is stopped or the chain has no
   * VoiceConvert entry. Sending `null` clears it to passthrough.
   */
  const applyActiveVoice = (): void => {
    if (!engineRunning()) return;
    const idx = voiceConvertIndex();
    if (idx < 0) return;
    void setVoiceModelCmd(idx, activeVoicePath());
  };

  const refreshVoiceLibrary = async (): Promise<void> => {
    try {
      const [list, status] = await Promise.all([
        listVoicesCmd(),
        onnxRuntimeStatusCmd(),
      ]);
      // Defensive: a stubbed/!Tauri bridge can hand back null; never
      // let `voices()` become non-array (the UI calls `.length`/`.find`).
      setVoices(Array.isArray(list) ? list : []);
      setOnnxStatus(status ?? null);
      // If the persisted active voice no longer exists on disk, clear
      // it so the UI doesn't claim a missing model is selected.
      const id = activeVoiceId();
      if (id && !voices().some((v) => v.id === id)) {
        setActiveVoiceIdRaw(null);
        saveJson(STORAGE_KEYS.activeVoice, null);
        applyActiveVoice();
      }
    } catch (err) {
      console.warn("[voices] refresh failed", err);
    }
  };

  const setActiveVoice = (id: string | null): void => {
    setActiveVoiceIdRaw(id);
    saveJson(STORAGE_KEYS.activeVoice, id);
    applyActiveVoice();
  };

  // Editing targets the VIEWED preset (on the Mixer that's always the
  // active one; on Presets it may be a non-active preset being tweaked).
  // Changes only reach the live engine when the viewed preset IS the
  // active one — otherwise they're stored and applied when it's Used.
  const editIsLive = (): boolean => engineRunning() && viewedId() === presetId();

  const setChainParam = (effectIndex: number, key: string, value: number): void => {
    setChains(viewedId(), effectIndex, "vals", key, value);
    if (editIsLive()) {
      void setEffectParamCmd(effectIndex, key, value);
    }
  };

  const setChainEnabled = (effectIndex: number, enabled: boolean): void => {
    setChains(viewedId(), effectIndex, "enabled", enabled);
    if (editIsLive()) {
      void setEffectEnabledCmd(effectIndex, enabled);
    }
  };

  const toggleEffectById = (id: EffectId): void => {
    const idx = viewedChain().findIndex((c) => c.id === id);
    if (idx < 0) return;
    const entry = viewedChain()[idx];
    if (!entry) return;
    setChainEnabled(idx, !entry.enabled);
  };

  const reorderChainEntries = (from: number, to: number): void => {
    const current = viewedChain();
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
    setChains(viewedId(), next);
    if (editIsLive()) {
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

  // Preview a preset in the Presets editor without making it live. The
  // "Use" button (usePreset) is what actually applies it.
  const viewPreset = (id: string): void => {
    if (!presets().some((p) => p.id === id)) return;
    setViewedId(id);
  };

  const usePreset = (id: string): void => {
    if (!presets().some((p) => p.id === id)) return;
    setPresetId(id);
    setViewedId(id); // applying makes the viewed preset the active one

    // Reset A/B for the new preset (both slots = current chain).
    const c = chains[id] ?? [];
    setAbSlots(id, { A: cloneChain(c), B: cloneChain(c) });
    setUi("ab", "A");
    if (engineRunning()) {
      void setEffectChainCmd(chainToSpecs(c));
    }
  };

  // The Coven (v1.1.0): summon a cast member — apply its preset chain and
  // set the conversion model (model-backed members like the narrator) or
  // clear it (DSP characters). Ordering matters: `usePreset` switches the
  // chain first so `setActiveVoice` finds the live `voice_convert` slot.
  const summon = (presetId: string, modelId: string | null = null): void => {
    usePreset(presetId);
    setActiveVoice(modelId);
  };

  // Keep the viewed preset synced to the active one everywhere except the
  // Presets screen, so the Mixer (and every other screen) always edits
  // the live preset, while the Presets list is free to preview a
  // non-active preset without applying it.
  createEffect(
    on(
      nav,
      (n) => {
        if (n !== "presets") setViewedId(presetId());
      },
      { defer: true },
    ),
  );

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

  // v1.14.0: import a preset file. The backend validates + saves it as a
  // unique User preset; we refresh the list so it appears, and hand back
  // the saved preset so the caller can preview it.
  const importPreset = async (path: string): Promise<Preset> => {
    const wire = await importPresetCmd(path);
    await refreshPresets();
    return presetFromWire(wire);
  };

  // Soundboard state.

  // Phase 15: the active soundboard folder persists across restarts so
  // the user doesn't have to re-pick it every launch.
  const [soundboardFolderRaw, setSoundboardFolderRaw] = createSignal<string | null>(
    loadJson<string | null>(STORAGE_KEYS.soundboardFolder, null),
  );
  const soundboardFolder = soundboardFolderRaw;
  const setSoundboardFolder = (folder: string | null): void => {
    setSoundboardFolderRaw(folder);
    saveJson(STORAGE_KEYS.soundboardFolder, folder);
  };
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
  // Phase 15: per-tile gain (linear, 1.0 = unchanged) + master gain.
  const [tileGains, setTileGains] = createStore<Record<string, number>>(
    loadJson<Record<string, number>>(STORAGE_KEYS.tileGains, {}),
  );
  const [soundboardMasterGain, setSoundboardMasterGainRaw] = createSignal<number>(
    loadJson<number>(STORAGE_KEYS.soundboardMasterGain, 1.0),
  );
  const tileGain = (clipId: string): number => tileGains[clipId] ?? 1.0;
  const setTileGain = (clipId: string, gain: number): void => {
    setTileGains(clipId, gain);
    saveJson(STORAGE_KEYS.tileGains, { ...tileGains });
  };
  const setSoundboardMasterGain = (gain: number): void => {
    setSoundboardMasterGainRaw(gain);
    saveJson(STORAGE_KEYS.soundboardMasterGain, gain);
    void setSoundboardMasterGainCmd(gain);
  };
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
      const durationSecs = await playSoundboardClipCmd(
        tile.id,
        tile.path,
        tileGain(tile.id),
      );
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

  // MIDI control surfaces (v1.9.0). The backend forwards each note / CC
  // as a `midi-message` event; App.tsx feeds them to `handleMidiMessage`,
  // which either captures a trigger (MIDI-learn) or routes the message
  // through the user's mappings to the existing store actions.

  const [midiInputs, setMidiInputs] = createSignal<MidiInputInfo[]>([]);
  const [selectedMidiInput, setSelectedMidiInput] = createSignal<string | null>(
    loadJson<string | null>(STORAGE_KEYS.midiInput, null),
  );
  const [midiMappings, setMidiMappings] = createSignal<MidiMapping[]>(
    loadJson<MidiMapping[]>(STORAGE_KEYS.midiMappings, []),
  );
  const [midiLearnId, setMidiLearnIdRaw] = createSignal<string | null>(null);

  const persistMidiMappings = (next: MidiMapping[]): void => {
    setMidiMappings(next);
    saveJson(STORAGE_KEYS.midiMappings, next);
  };

  const refreshMidiInputs = async (): Promise<void> => {
    try {
      setMidiInputs(await listMidiInputsCmd());
    } catch (err) {
      console.warn("[midi] list inputs failed", err);
    }
  };

  const selectMidiInput = async (name: string | null): Promise<void> => {
    setSelectedMidiInput(name);
    saveJson(STORAGE_KEYS.midiInput, name);
    try {
      if (name) await openMidiInputCmd(name);
      else await closeMidiInputCmd();
    } catch (err) {
      console.warn("[midi] open/close failed", err);
      // Leave the selection set so the UI shows the attempted device;
      // the user can re-pick once the port is available.
    }
  };

  const newMidiId = (): string =>
    typeof crypto !== "undefined" && typeof crypto.randomUUID === "function"
      ? crypto.randomUUID()
      : `midi-${Date.now().toString(36)}-${Math.floor(Math.random() * 1e6)}`;

  const addMidiMapping = (mapping: Omit<MidiMapping, "id">): void => {
    persistMidiMappings([...midiMappings(), { ...mapping, id: newMidiId() }]);
  };

  const updateMidiMapping = (id: string, patch: Partial<MidiMapping>): void => {
    persistMidiMappings(
      midiMappings().map((m) => (m.id === id ? { ...m, ...patch } : m)),
    );
  };

  const removeMidiMapping = (id: string): void => {
    if (midiLearnId() === id) setMidiLearnIdRaw(null);
    persistMidiMappings(midiMappings().filter((m) => m.id !== id));
  };

  const setMidiLearnId = (id: string | null): void => {
    setMidiLearnIdRaw(id);
  };

  // Bind the pure router's handler interface to the live store actions.
  const midiHandlers: MidiHandlers = {
    usePreset: (id) => usePreset(id),
    playTile: (id) => void playTileById(id),
    setPtm: (pressed) => setUi("pressed", pressed),
    toggleMonitor: () => void toggleMonitor(),
    setParam: (idx, key, value) => setChainParam(idx, key, value),
  };

  const handleMidiMessage = (msg: MidiMessage): void => {
    const learnId = midiLearnId();
    if (learnId !== null) {
      const trig = triggerFor(msg);
      if (trig) {
        updateMidiMapping(learnId, { trigger: trig });
        setMidiLearnIdRaw(null);
      }
      // Swallow the message used for learning — don't also route it.
      return;
    }
    routeMidi(msg, midiMappings(), midiHandlers);
  };

  // Guided mic calibration (v1.10.0). Sample the existing input-level
  // stream over a short "stay quiet" window, derive a noise-gate (and
  // denoiser) suggestion, and write it into the active chain.

  const [calibrated, setCalibratedRaw] = createSignal<boolean>(
    loadJson<boolean>(STORAGE_KEYS.calibrated, false),
  );
  const setCalibrated = (value: boolean): void => {
    setCalibratedRaw(value);
    saveJson(STORAGE_KEYS.calibrated, value);
  };
  const [calibrating, setCalibrating] = createSignal(false);
  const [calibrationResult, setCalibrationResult] =
    createSignal<CalibrationResult | null>(null);

  const applyCalibration = (result: CalibrationResult): void => {
    const chain = viewedChain();
    const gateIdx = chain.findIndex((e) => e.id === "gate");
    if (gateIdx >= 0) setChainParam(gateIdx, "thresh", result.gateThreshDb);
    if (result.denoiserMix > 0) {
      const denIdx = chain.findIndex((e) => e.id === "denoiser");
      if (denIdx >= 0) setChainParam(denIdx, "mix", result.denoiserMix);
    }
  };

  const runCalibration = async (durationMs = 2000): Promise<CalibrationResult> => {
    setCalibrating(true);
    const samples: number[] = [];
    const startedAt = performance.now();
    await new Promise<void>((resolve) => {
      const tick = (): void => {
        samples.push(inputLevels().rms);
        if (performance.now() - startedAt >= durationMs) {
          resolve();
        } else {
          setTimeout(tick, 50);
        }
      };
      tick();
    });
    const result = computeCalibration(samples);
    setCalibrationResult(result);
    applyCalibration(result);
    setCalibrated(true);
    setCalibrating(false);
    return result;
  };

  // In-app update check (v1.12.0). Lightweight, opt-out, no telemetry —
  // a one-way read of the latest GitHub release. Gated so the browser /
  // E2E / dev builds never hit the network.
  const [updateCheckEnabled, setUpdateCheckEnabledRaw] = createSignal<boolean>(
    loadJson<boolean>(STORAGE_KEYS.updateCheckEnabled, true),
  );
  const setUpdateCheckEnabled = (value: boolean): void => {
    setUpdateCheckEnabledRaw(value);
    saveJson(STORAGE_KEYS.updateCheckEnabled, value);
  };
  const [updateAvailable, setUpdateAvailable] = createSignal<UpdateInfo | null>(
    null,
  );
  const [updateChecking, setUpdateChecking] = createSignal(false);
  const dismissUpdate = (): void => {
    setUpdateAvailable(null);
  };

  const checkUpdates = async (): Promise<void> => {
    if (!updateCheckEnabled() || updateChecking()) return;
    setUpdateChecking(true);
    try {
      const { getVersion } = await import("@tauri-apps/api/app");
      const version = await getVersion().catch(() => null);
      // Skip in the browser / E2E (no version) and dev builds ("0.0.0").
      if (!version || version === "0.0.0") return;
      setUpdateAvailable(await checkForUpdate(version));
    } catch {
      /* best-effort — never disrupt the app */
    } finally {
      setUpdateChecking(false);
    }
  };

  // Setup diagnostic (v1.13.0). Frontend-orchestrated over existing
  // commands; careful with the engine — restores its prior run-state.
  const [diagnostics, setDiagnostics] = createSignal<DiagCheck[] | null>(null);
  const [diagnosing, setDiagnosing] = createSignal(false);

  const runDiagnostics = async (): Promise<DiagCheck[]> => {
    if (diagnosing()) return diagnostics() ?? [];
    setDiagnosing(true);
    const wasRunning = engineRunning();
    try {
      await refreshDevices();
      await refreshVirtualMicStatus();

      // Start the engine if stopped — to exercise startup + signal flow —
      // but only when there's an input and output to talk to.
      let engineStarted = engineRunning();
      if (!engineStarted && selectedInput() && selectedOutput()) {
        await startEngine();
        engineStarted = engineRunning();
      }

      // Sample the OUT meter briefly to see whether signal is flowing.
      let signalFlowing = false;
      if (engineStarted && !engineError()) {
        const startedAt = performance.now();
        while (performance.now() - startedAt < 700) {
          if (outputLevels().rms > 0.0005 || outputLevels().peak > 0.001) {
            signalFlowing = true;
            break;
          }
          await new Promise<void>((r) => setTimeout(r, 50));
        }
      }

      const cable = virtualMicStatus();
      const result = evaluateDiagnostics({
        inputCount: audioInputs().length,
        outputCount: audioOutputs().length,
        selectedInput: selectedInput(),
        selectedOutput: selectedOutput(),
        engineStarted,
        engineError: engineError(),
        signalFlowing,
        cableDetected: cable?.detected ?? false,
        cableInputName: cable?.cableInputDevice?.name ?? null,
      });
      setDiagnostics(result);
      return result;
    } finally {
      // Leave the engine exactly as we found it.
      if (!wasRunning && engineRunning()) {
        await stopEngine();
      }
      setDiagnosing(false);
    }
  };

  // ---- v1.17.0: text-to-speech ("Speak") ----

  const [ttsVoices, setTtsVoices] = createSignal<TtsVoiceInfo[]>([]);
  const [selectedTtsVoice, setSelectedTtsVoiceRaw] = createSignal<string | null>(
    loadJson<string | null>(STORAGE_KEYS.ttsVoice, null),
  );
  const [ttsText, setTtsText] = createSignal("");
  const [synthesizing, setSynthesizing] = createSignal(false);
  const [ttsError, setTtsError] = createSignal<string | null>(null);
  const [ttsVolume, setTtsVolumeRaw] = createSignal<number>(
    loadJson<number>(STORAGE_KEYS.ttsVolume, 1.0),
  );
  const [ttsPreviewOnly, setTtsPreviewOnlyRaw] = createSignal<boolean>(
    loadJson<boolean>(STORAGE_KEYS.ttsPreviewOnly, false),
  );

  const setSelectedTtsVoice = (id: string): void => {
    setSelectedTtsVoiceRaw(id);
    saveJson(STORAGE_KEYS.ttsVoice, id);
  };

  const setTtsVolume = (gain: number): void => {
    const clamped = Math.max(0, Math.min(2, gain));
    setTtsVolumeRaw(clamped);
    saveJson(STORAGE_KEYS.ttsVolume, clamped);
  };

  const setTtsPreviewOnly = (on: boolean): void => {
    setTtsPreviewOnlyRaw(on);
    saveJson(STORAGE_KEYS.ttsPreviewOnly, on);
  };

  const refreshTtsVoices = async (): Promise<void> => {
    try {
      const list = await listTtsVoicesCmd();
      const next = Array.isArray(list) ? list : [];
      setTtsVoices(next);
      // Default the selection to the first voice when none is chosen (or
      // the persisted one no longer exists in the list).
      const current = selectedTtsVoice();
      const first = next[0];
      if ((!current || !next.some((v) => v.id === current)) && first) {
        setSelectedTtsVoice(first.id);
      }
    } catch (err) {
      console.warn("[tts] voice refresh failed", err);
    }
  };

  const speakText = async (): Promise<void> => {
    const text = ttsText().trim();
    const voiceId = selectedTtsVoice();
    if (!text || !voiceId || synthesizing()) return;
    setTtsError(null);
    setSynthesizing(true);
    try {
      const durationSecs = await speakCmd(
        text,
        voiceId,
        ttsVolume(),
        ttsPreviewOnly(),
      );
      // Reuse the soundboard playback seam: register a "tts" clip so the
      // shared ~30 Hz clock drives the progress ring and auto-expires it.
      setPlayingClips("tts", {
        clipId: "tts",
        startedAt: performance.now(),
        durationSecs,
      });
    } catch (err) {
      setTtsError(
        typeof err === "string"
          ? err
          : err instanceof Error
            ? err.message
            : String(err),
      );
    } finally {
      setSynthesizing(false);
    }
  };

  const stopSpeaking = async (): Promise<void> => {
    setPlayingClips("tts", undefined as unknown as PlayingClip);
    try {
      await stopSpeakCmd();
    } catch (err) {
      console.warn("[tts] stop failed", err);
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
    viewedId,
    viewedPreset,
    viewedChain,
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
    glyphBindings,
    setGlyphBinding,
    customGlyphs,
    addCustomGlyph,
    removeCustomGlyph,
    setCustomGlyphAction,
    glyphOutcome,
    dispatchGlyphAction,
    recognizeCustomGlyph,

    overlayOpen,
    overlayBg,
    setOverlayBg,
    openOverlay,
    closeOverlay,

    audioInputs,
    setAudioInputs,
    audioOutputs,
    setAudioOutputs,
    selectedInput,
    setSelectedInput,
    selectedOutput,
    setSelectedOutput,
    selectedMonitor,
    setSelectedMonitor,
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
    dspLatencyMs,
    setDspLatencyMs,
    isRecording,
    setIsRecording,
    recordingPath,

    refreshDevices,
    startEngine,
    stopEngine,
    toggleMonitor,
    setMonitor,
    monitorGain,
    setMonitorGain,
    loudnessEnabled,
    setLoudnessEnabled,
    loudnessTarget,
    setLoudnessTarget,
    loudnessGainDb,
    setLoudnessGainDb,
    toggleRecording,
    getRecordingsDir,
    getLogsDir,

    setChainParam,
    setChainEnabled,
    toggleEffectById,
    syncChain,
    reorderChainEntries,

    voices,
    onnxStatus,
    activeVoiceId,
    setActiveVoice,
    refreshVoiceLibrary,

    ttsVoices,
    selectedTtsVoice,
    setSelectedTtsVoice,
    ttsVolume,
    setTtsVolume,
    ttsPreviewOnly,
    setTtsPreviewOnly,
    ttsText,
    setTtsText,
    synthesizing,
    ttsError,
    refreshTtsVoices,
    speakText,
    stopSpeaking,

    usePreset,
    viewPreset,
    summon,
    savePreset,
    duplicatePreset,
    deletePreset,
    exportPreset,
    importPreset,
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
    tileGain,
    setTileGain,
    soundboardMasterGain,
    setSoundboardMasterGain,
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

    midiInputs,
    refreshMidiInputs,
    selectedMidiInput,
    selectMidiInput,
    midiMappings,
    addMidiMapping,
    updateMidiMapping,
    removeMidiMapping,
    midiLearnId,
    setMidiLearnId,
    handleMidiMessage,

    calibrated,
    setCalibrated,
    calibrating,
    calibrationResult,
    runCalibration,
    applyCalibration,

    updateCheckEnabled,
    setUpdateCheckEnabled,
    updateAvailable,
    dismissUpdate,
    updateChecking,
    checkUpdates,

    diagnostics,
    diagnosing,
    runDiagnostics,

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
