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
  onCleanup,
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
  listClonedVoices as listClonedVoicesCmd,
  cloneVoice as cloneVoiceCmd,
  startVoiceRecording as startVoiceRecordingCmd,
  stopVoiceRecording as stopVoiceRecordingCmd,
  deleteClonedVoice as deleteClonedVoiceCmd,
  renameClonedVoice as renameClonedVoiceCmd,
  pickAudioFile as pickAudioFileCmd,
  cloneModelsStatus as cloneModelsStatusCmd,
  voxcpmStatus as voxcpmStatusCmd,
  voxcpmGpuStatus as voxcpmGpuStatusCmd,
  downloadCloneModels as downloadCloneModelsCmd,
  downloadVoxcpmModels as downloadVoxcpmModelsCmd,
  downloadVoxcpmGpuModel as downloadVoxcpmGpuModelCmd,
  modelDownloadStatus as modelDownloadStatusCmd,
  subscribeCloneDownload,
  subscribeCloneDownloadDone,
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
  setReactiveConfig as setReactiveConfigCmd,
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
  previewVoice as previewVoiceCmd,
  stopPreviewVoice as stopPreviewVoiceCmd,
  listSpeakClips as listSpeakClipsCmd,
  deleteSpeakClip as deleteSpeakClipCmd,
  setSpeakMonitor as setSpeakMonitorCmd,
  setVoiceReadingEnabled as setVoiceReadingEnabledCmd,
  getVoiceReading as getVoiceReadingCmd,
  subscribeVoiceReading,
  setInputMuted as setInputMutedCmd,
  toggleInputMuted as toggleInputMutedCmd,
  inputMute as inputMuteCmd,
  setPtmMode as setPtmModeCmd,
  setPtmHeld as setPtmHeldCmd,
  screenCaptureStatus as screenCaptureStatusCmd,
  listCaptureMonitors as listCaptureMonitorsCmd,
  listDesktopAudioSources as listDesktopAudioSourcesCmd,
  startScreenRecording as startScreenRecordingCmd,
  stopScreenRecording as stopScreenRecordingCmd,
  screenRecordingStatus as screenRecordingStatusCmd,
  subscribeScreenRecording,
  replayBufferStatus as replayBufferStatusCmd,
  replayMemory as replayMemoryCmd,
  armReplayBuffer as armReplayBufferCmd,
  disarmReplayBuffer as disarmReplayBufferCmd,
  saveReplay as saveReplayCmd,
  subscribeReplayBuffer,
  subscribeReplaySaved,
  openOverlayWindow,
  closeOverlayWindow,
  placeOverlayWindow,
  setOverlayMode as setOverlayModeCmd,
  setOverlayHideFromRecordings,
  overlayStatus as overlayStatusCmd,
  importOverlayAsset,
  deleteOverlayAsset,
  pickImageFile,
  subscribeOverlayStatus,
  subscribeOverlayEdit,
  syncTimersCmd,
  armTimerCmd,
  pauseTimerCmd,
  resumeTimerCmd,
  resetTimerCmd,
  adjustTimerCmd,
  timerStatusCmd,
  timersDirCmd,
  subscribeTimerState,
  finishTimerCmd,
  subscribeTimerActions,
  subscribeSoundboardStopped,
  type CaptureMonitor,
  type CaptureStatus,
  type DesktopAudioSource,
  type ReplayArmOptions,
  type ReplayMemory,
  type ReplayMemoryOptions,
  type ReplaySaved,
  type ReplaySeconds,
  type ReplayStatus,
  type ScreenRecordingUpdate,
  unregisterGlobalShortcut as unregisterGlobalShortcutCmd,
  type DeviceInfo,
  type EffectSpec,
  type InputMute,
  type Levels,
  type MidiInputInfo,
  type MidiMessage,
  type OnnxRuntimeStatus,
  type PtmPageSource,
  type PtmState,
  type ClonedVoiceInfo,
  type CloneDownloadProgress,
  type ModelDownloadDone,
  type ModelDownloadKind,
  type SpeakClip,
  type SoundboardTile,
  type StreamInfo,
  type TtsVoiceInfo,
  type VirtualMicStatus,
  type VoiceInfo,
  type VoiceReadingUpdate,
  type WirePreset,
} from "../audio/api";
import {
  paramDefault,
  RAGE_PRIMARY,
  RAGE_TARGETS,
  REACTIVE_ATTACK_MS,
  REACTIVE_CEIL_DB,
  REACTIVE_DEFAULT_INTENSITY,
  REACTIVE_FLOOR_DB,
  REACTIVE_HOLD_MS,
  REACTIVE_RELEASE_MS,
} from "../data/reactive";
import {
  computeCalibration,
  type CalibrationResult,
} from "../audio/calibration";
import { checkForUpdate, type UpdateInfo } from "../data/updates";
import {
  IDLE_SCREEN_RECORDING,
  pickDesktopSource,
  pickMonitor,
  readScreenSettings,
  screenFilename,
  type ScreenSettings,
} from "../data/screenRecording";
import {
  clipWasModulated,
  explainerNeeded,
  OFF_REPLAY_STATUS,
  readReplaySettings,
  REPLAY_GATE_TOAST,
  replayOn,
  savedToast,
  type ReplaySettings,
} from "../data/replay";
import type { ToastMessage, ToastTone } from "../components/Toast";
import {
  decideWhatsNew,
  notesSince,
  RELEASE_NOTES,
  RENDER_CAP,
  type ReleaseNote,
  type WhatsNewContent,
} from "../data/changelog";
import { evaluateDiagnostics, type DiagCheck } from "../audio/diagnostics";
import {
  makeTemplate,
  recognize as recognizeUnistroke,
  type UnistrokeTemplate,
} from "../data/unistroke";
import type { ReactiveConfig, ReactiveRouteSpec } from "../audio/api";
import type { Point } from "../data/glyphs";
import type { OverlayBg, ScenePush } from "../overlay/state";
import type { OverlayScene } from "../overlay/scene";
import type { OverlayStatus } from "../overlay/status";
import {
  adoptTimers,
  readTimerSnapshot,
  type TimerKind,
  type TimerPhase,
  type TimerSnapshot,
} from "../overlay/timerState";
import {
  createOverlayController,
  OVERLAY_PREFS_KEY,
  OVERLAY_SCENE_KEY,
  tauriOverlayWindows,
} from "../overlay/controller";
import {
  keysToAccelerator,
  normalizeAccelerator,
  readHotkeyBindings,
  unboundHotkeys,
} from "../data/hotkeys";
import { FALLBACK_PRESETS, presetFromWire } from "../data/presets";
import { adoptMute, muteCcStep, readInputMute } from "../data/mute";
import { inOrder } from "../data/inOrder";
import {
  DEFAULT_LENGTH_MS,
  defaultName,
  defsForSync,
  freshNameFrom,
  MAX_LENGTH_MS,
  MAX_TIMERS,
  MIN_LENGTH_MS,
  nextTimerId,
  readTimerName,
  readTimers,
  runOf,
  sameRun,
  timerAction,
  timerHotkeyId,
  timerRoles as rolesOf,
  timersForSave,
  TIMER_VERBS,
  type StoredTimer,
  type StoredTimers,
  type TimerRole,
  type TimerVerb,
} from "../data/timers";
import {
  ACTION_KINDS,
  breakTemplate,
  canAddAction,
  pickRandomPreset,
  readAction,
  readReturns,
  readStoredActions,
  readTimerActionsEvent,
  reconcileReturns,
  restartNotice,
  returnsAfterStart,
  revertPlan,
  revertsAtZero,
  withAdded,
  withRemoved,
  withReplaced,
  type PresetReturn,
  type TimerAction,
  type TimerHook,
} from "../data/timerActions";
import {
  adoptPtm,
  ptmModeOf,
  readPtmSetting,
  readPtmState,
  settingOf,
  transforms,
  type PtmSetting,
} from "../data/ptm";
import {
  downloadAfter,
  readDownloadProgress,
  type ModelDownload,
} from "../data/modelDownload";
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
  PtmHold,
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
  monitor: true,
  ab: "A",
  ptmKey: "Space",
});

/** Per-preset A/B comparison snapshots. */
export interface AbSnapshots {
  A: ChainEntry[];
  B: ChainEntry[];
}

/** Stable id used for the system-wide hotkeys we register. v1.52.0 adds the
 *  recorders, under the reserved `rec:` prefix; v1.54.0 adds Replay's save
 *  and on/off, under `replay:`; v1.55.0 the stream overlay's Arrange / Lock
 *  switch, under `overlay:`. v1.56.0 adds `mute`, which the backend acts on
 *  itself (a toggle on press); the `mute:` prefix is reserved. v1.58.0: the
 *  backend acts on `ptm` itself too, press and release (it holds
 *  push-to-modulate while the key is down); the event still comes and the
 *  page does nothing with it. The `ptm:` prefix is reserved. */
export type HotkeyAction =
  | "ptm"
  | "panic"
  | "monitor"
  | "rec:screen"
  | "rec:wav"
  | "replay:save"
  | "replay:toggle"
  | "overlay:arrange"
  | "mute";

/** localStorage keys for Phase 8 persistence (tile metadata + recent folders). */
/** Reactive intensity is a 0..100 percentage everywhere in the UI. */
function clampIntensity(pct: number): number {
  return Math.min(100, Math.max(0, Math.round(pct)));
}

const STORAGE_KEYS = {
  tileColors: "divora.tileColors",
  tileOrder: "divora.tileOrder",
  tileHotkeys: "divora.tileHotkeys",
  recentFolders: "divora.recentFolders",
  tweaks: "divora.tweaks",
  activePreset: "divora.activePreset",
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
  overlay: OVERLAY_PREFS_KEY,
  // v1.55.0: the overlay's charms and where they sit. Written only by main
  // (overlay/controller.ts).
  overlayScene: OVERLAY_SCENE_KEY,
  reactive: "divora.reactive",
  ttsVoice: "divora.ttsVoice",
  ttsVolume: "divora.ttsVolume",
  ttsPreviewOnly: "divora.ttsPreviewOnly",
  cloneQuality: "divora.cloneQuality",
  speakMonitor: "divora.speakMonitor",
  cloneGpu: "divora.cloneGpu",
  whatsNewSeenVersion: "divora.whatsNewSeenVersion",
  // Only the on/off switch is remembered. Nothing derived from the user's
  // voice is ever written to disk — the rolling baseline the descriptors are
  // relative to lives in the engine for the life of a session and dies there.
  voiceReading: "divora.voiceReading",
  // v1.51.4: push-to-modulate / panic / monitor global hotkeys. Never
  // persisted before, although the docs said they were.
  hotkeyBindings: "divora.hotkeyBindings",
  // v1.52.0: Settings → Recordings for the screen recorder, and the screen
  // last picked on its card.
  screenRecording: "divora.screenRecording",
  // v1.54.0: Replay's length and whether its explainer has been read —
  // `{seconds, explained, explainedDesktop}` and nothing else. Never whether
  // it is on: Replay is off at every launch.
  replayBuffer: "divora.replayBuffer",
  // v1.58.0: push-to-modulate's `{on, hold}`, written only from the engine's
  // answer to `set_ptm_mode`. Never a hold: what holds it lives in the engine
  // and in memory.
  ptm: "divora.ptm",
  // v1.59.0: Hourglass's timers — `{version, timers: [{id, name, kind,
  // durationMs, hotkeys?, run?}]}` (data/timers.ts). Each `run` is the last
  // running, paused, done or ran-out state the page saw, with wall-clock
  // anchors, so a restart can pick it up. Absent: boot sends no timer
  // command.
  timers: "divora.timers",
} as const;

/** The first-run wizard's own key (it manages its own state in
 *  components/Wizard.tsx). Read here only to tell a brand-new install
 *  apart from an existing user who just updated. */
const WIZARD_SEEN_KEY = "divora.wizardSeen";

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

/** `bytes` random bytes as lowercase hex (overlay element ids, and the page
 *  load's session). */
function randomHex(bytes: number): string {
  const b = new Uint8Array(bytes);
  try {
    crypto.getRandomValues(b);
  } catch {
    for (let i = 0; i < bytes; i += 1) b[i] = Math.floor(Math.random() * 256);
  }
  return Array.from(b, (x) => x.toString(16).padStart(2, "0")).join("");
}

function saveJson(key: string, value: unknown): void {
  if (typeof window === "undefined") return;
  try {
    window.localStorage.setItem(key, JSON.stringify(value));
  } catch {
    /* quota, private mode, etc — non-fatal */
  }
}

/** Read a bare string (not JSON) — for keys shared with code that stores
 *  plain values, like the wizard's flag. */
function loadRaw(key: string): string | null {
  if (typeof window === "undefined") return null;
  try {
    return window.localStorage.getItem(key);
  } catch {
    return null;
  }
}

/** Write a bare string. Returns false when storage refused it, which the
 *  what's-new trigger uses to avoid re-announcing on every launch. */
function saveRaw(key: string, value: string): boolean {
  if (typeof window === "undefined") return false;
  try {
    window.localStorage.setItem(key, value);
    return true;
  } catch {
    return false;
  }
}

/** Most-recently-used cap for the recent-folders dropdown. */
const RECENT_FOLDERS_MAX = 5;

/** v1.60.0: how long after the reconcile pass a re-mute at launch is named
 *  by the restart notice (D36). */
export const RESTART_NOTICE_MS = 3000;

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
const DEFAULT_HOTKEY_BINDINGS: Record<HotkeyAction, string> = unboundHotkeys();

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
  /** v1.60.0: the one dispatcher, for a glyph's action and a timer's page
   *  half (`timer-actions`). A glyph's runs as `dispatchGlyphAction` always
   *  has. A timer's runs only its page kinds: a clip at once; a preset
   *  switch, a random preset and (at an end) Back to the preset before once
   *  the real preset list has loaded, in the order they came (D16). The
   *  backend runs the rest, so the page never sends a mute, a line, a save
   *  or a Stop all for a timer, and the glyph's toggles never run from
   *  one. */
  dispatchAction: (action: GlyphAction | TimerAction, from: ActionSource) => void;
  /** Match a drawn path against the custom-glyph templates (built-ins use
   *  the geometric `detectShape`). */
  recognizeCustomGlyph: (
    path: Point[],
  ) => { id: string; template: Point[] } | null;

  // Stream overlay (v1.16.0; charms v1.55.0 — overlay/controller.ts)
  /** Whether the overlay window exists, as the backend reports it. */
  overlayOpen: () => boolean;
  /** The backend's whole overlay status (phase, mode, fit, hide…). */
  overlayStatus: () => OverlayStatus;
  overlayMode: () => "locked" | "arrange";
  /** Overlay background mode (persisted): alpha, or a chroma colour. A
   *  colour turns Fit to monitor off first, and isn't applied if that
   *  fails. */
  overlayBg: () => OverlayBg;
  setOverlayBg: (bg: OverlayBg) => Promise<void>;
  /** Fit to monitor (persisted); it needs the Transparent background. */
  overlayFit: () => boolean;
  setOverlayFit: (on: boolean) => Promise<void>;
  /** Hide from recordings (persisted), pushed to the backend. */
  overlayHide: () => boolean;
  setOverlayHide: (on: boolean) => Promise<void>;
  /** The charms (`divora.overlayScene`), and what was last pushed. */
  overlayScene: () => OverlayScene;
  overlayScenePush: () => ScenePush;
  /** Open / close the overlay window; the backend creates and shows it. */
  openOverlay: () => Promise<void>;
  closeOverlay: () => Promise<void>;
  setOverlayMode: (mode: "locked" | "arrange") => Promise<void>;
  /** The `overlay:arrange` hotkey's notice; the backend acts on the key. */
  noteOverlayHotkey: () => void;
  addOverlayPicture: () => Promise<void>;
  removeOverlayElement: (id: string) => Promise<void>;
  addSpellCircle: () => void;

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
  /** v1.39.0: true when device-loss auto-recovery gave up (engine stopped,
   *  no audio) — drives the "device lost — restart" banner. */
  deviceLost: () => boolean;
  setDeviceLost: Setter<boolean>;
  /** v1.46.0: live reactive-modulation depth, 0..1, from the audio thread. */
  modEnv: () => number;
  setModEnv: Setter<number>;
  /** v1.46.0: whether reactive effects are driving the chain. */
  reactiveEnabled: () => boolean;
  setReactiveEnabled: (on: boolean) => void;
  /** v1.46.0: master depth scale, 0..100. */
  reactiveIntensity: () => number;
  setReactiveIntensity: (pct: number) => void;
  /** Whether the Voice reading panel is analyzing. Off by default; while it
   *  is off the backend copies no audio at all. */
  voiceReadingOn: () => boolean;
  setVoiceReadingOn: (on: boolean) => void;
  /** The latest reading, or null when nothing is being measured. */
  voiceReading: () => VoiceReadingUpdate | null;
  setVoiceReading: Setter<VoiceReadingUpdate | null>;
  /**
   * v1.47.0: the voice currently being auditioned, or null. Drives each card's
   * play/stop affordance and its "rendering..." state.
   */
  previewingVoice: () => string | null;
  /**
   * v1.47.0: true while a preview is still being SYNTHESIZED rather than
   * played. A cached voice resolves in milliseconds; an uncached clone takes
   * seconds, and a button that looks identical either way invites a second tap.
   */
  previewRendering: () => boolean;
  /** v1.47.0: play a short fixed sample in `voiceId` (monitor-only). */
  previewVoice: (voiceId: string) => Promise<void>;
  /** v1.47.0: stop the running audition. */
  stopPreview: () => void;
  /**
   * v1.46.0: whether the active chain contains the effect "Rage" drives.
   * False means enabling it would do nothing audible, so the UI offers to add
   * one rather than silently having no effect.
   */
  reactiveHasTarget: () => boolean;
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

  // ---- v1.52.0: screen recording ----
  /** Whether this PC can record the screen (`null` until asked). */
  captureStatus: () => CaptureStatus | null;
  captureMonitors: () => CaptureMonitor[];
  /** Ask the backend again: the status (cached there), the screens, and
   *  the desktop audio sources. */
  refreshCapture: () => Promise<void>;
  /** v1.53.0: where desktop audio can come from (`null` until asked). */
  desktopSources: () => DesktopAudioSource[] | null;
  /** The latest `screen-recording` report. */
  screenRecording: () => ScreenRecordingUpdate;
  screenSettings: () => ScreenSettings;
  setScreenSettings: (patch: Partial<ScreenSettings>) => void;
  /** Start a take of the chosen screen, or stop the running one. While
   *  Replay is on, the take uses Replay's settings (v1.54.0). */
  toggleScreenRecording: () => Promise<void>;
  /** The last finished (or current) take's MP4. */
  screenRecordingPath: () => string | null;

  // ---- v1.54.0: Replay ----
  /** The latest `replay-buffer` report. */
  replayStatus: () => ReplayStatus;
  /** The length, and whether the card's explainer has been read. */
  replaySettings: () => ReplaySettings;
  /** Each length's memory cap at the current settings (`null` until asked). */
  replayMemory: () => ReplayMemory | null;
  /** Ask again — the card and Settings do on mount; the store does when the
   *  screen, frame rate or quality changes. */
  refreshReplayMemory: () => Promise<void>;
  /** Turn Replay on (or resize it while on). The ONLY caller of
   *  `arm_replay_buffer`: it refuses, with a toast, until the card's
   *  explainer has been read — so every arm path, now and later, is gated. */
  armReplay: () => Promise<void>;
  disarmReplay: () => Promise<void>;
  /** Off if on; otherwise `armReplay()`, gate and all. */
  toggleReplay: () => Promise<void>;
  /** Ask for a save. The `replay-saved` event reports every outcome. */
  saveReplay: () => Promise<void>;
  /** Persist a length, and resize a running ring to it. */
  setReplaySeconds: (seconds: ReplaySeconds) => Promise<void>;
  /** The explainer was read — with desktop audio on, if `desktop`. */
  markReplayExplained: (desktop: boolean) => void;
  /** When the voice last became modulated with the current preset, on the
   *  `performance.now()` clock; `null` while clean or muted. A save names the
   *  preset only when this precedes the whole clip. */
  modulatedSince: () => number | null;
  /** The shell's toast (`components/Toast`). */
  toast: () => ToastMessage | null;
  showToast: (text: string, tone?: ToastTone) => void;
  /** Move the running engine to the selected devices without a Stop: the
   *  start tears the old streams down itself, so Replay sees a rebuild it
   *  carries on through rather than a stop it must hold for. */
  restartEngine: () => Promise<void>;
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
  /** Built-in Speak voices — Kokoro presets, then Critter Chatter (each flagged `installed`). */
  ttsVoices: () => TtsVoiceInfo[];
  /** Selected voice id — built-in or cloned (persisted `divora.ttsVoice`); null until first load. */
  selectedTtsVoice: () => string | null;
  /** v1.60.0: fills `ttsVoices` once, for a timer's line, when nothing has
   *  yet, without touching Speak's selection (`refreshTtsVoices` can
   *  reset it). Settles when the list has answered or failed. */
  ensureTtsVoices: () => Promise<void>;
  /** v1.60.0: true once a list of Speak voices has answered (or failed):
   *  until then a timer's line says "Loading voices…", not that its voice
   *  is missing. */
  ttsVoicesLoaded: () => boolean;
  setSelectedTtsVoice: (id: string) => void;
  /** Speak playback volume, linear 0..2 (persisted `divora.ttsVolume`). */
  ttsVolume: () => number;
  setTtsVolume: (gain: number) => void;
  /** v1.18.0: when true, Speak plays to the local monitor only (you hear it,
   *  the call doesn't). Persisted `divora.ttsPreviewOnly`. */
  ttsPreviewOnly: () => boolean;
  setTtsPreviewOnly: (on: boolean) => void;
  /** v1.25.0: best-of-N count for VoxCPM cloned voices — the backend generates
   *  N takes and keeps the one closest to your voice (Fast 1 / Balanced 3 /
   *  Best 6; higher = more faithful but slower). Persisted `divora.cloneQuality`. */
  cloneQuality: () => number;
  setCloneQuality: (n: number) => void;
  /** v1.29.0: hear Speak previews in the monitor, independent of the Mixer's
   *  mic monitor. Default on; persisted `divora.speakMonitor`. */
  speakMonitor: () => boolean;
  setSpeakMonitor: (on: boolean) => void;
  /** v1.30.0: experimental DirectML GPU for VoxCPM cloning (~1.3× on the decode
   *  step). Opt-in — needs the ~1.2 GB fp16 GPU model. Persisted `divora.cloneGpu`. */
  cloneGpu: () => boolean;
  setCloneGpu: (on: boolean) => void;
  /** Draft text in the Speak box (ephemeral — not persisted). */
  ttsText: () => string;
  setTtsText: (text: string) => void;
  /** True while a synthesize request is in flight. */
  synthesizing: () => boolean;
  /** Best-of-N take progress (v1.27.0): `{done,total}` while a VoxCPM clone
   *  generates its takes, null when idle or for single-pass voices. */
  ttsProgress: () => { done: number; total: number } | null;
  setTtsProgress: (p: { done: number; total: number } | null) => void;
  /** Last synthesis error (e.g. "voices not installed"), or null. */
  ttsError: () => string | null;
  /** Pull the preset voice list from the backend + default the selection. */
  refreshTtsVoices: () => Promise<void>;
  /** Synthesize the current text with the selected voice and play it. */
  speakText: () => Promise<void>;
  /** Stop any in-flight synthesized speech. */
  stopSpeaking: () => Promise<void>;
  /** v1.28.0: saved Speak clips (TTS outputs), newest first. */
  speakClips: () => SpeakClip[];
  /** Reload the saved-clips list from the backend. */
  refreshSpeakClips: () => Promise<void>;
  /** Replay a saved clip through the soundboard. */
  playSpeakClip: (clip: SpeakClip) => Promise<void>;
  /** Delete a saved clip (WAV + sidecar) by id. */
  removeSpeakClip: (id: string) => Promise<void>;

  // Voice cloning ("Your voices") — v1.20.0
  /** The user's cloned voices (selectable in the picker alongside presets). */
  clonedVoices: () => ClonedVoiceInfo[];
  /** True while a clone (reference → speaker embedding) is being created. */
  cloning: () => boolean;
  /** Last clone error (e.g. "not installed"), or null. */
  cloneError: () => string | null;
  /** Pull the cloned-voice list from the backend. */
  refreshClonedVoices: () => Promise<void>;
  /** Pick a reference clip and create a cloned voice named `name`, then select it. */
  addClonedVoice: (name: string) => Promise<void>;
  /** Delete a cloned voice; if it was selected, fall back to the first preset. */
  removeClonedVoice: (id: string) => Promise<void>;
  /**
   * v1.43.0: rename a cloned voice's display name (the id stays stable, so the
   * selection and saved clips are unaffected). Resolves true on success; on
   * failure the reason lands in `cloneError`.
   */
  renameClonedVoice: (id: string, name: string) => Promise<boolean>;
  /** v1.23.0: true while the dry mic is being captured for an in-app clone. */
  recordingVoice: () => boolean;
  /** v1.57.0: the sentence the take in progress is to be read from (a VoxCPM
   *  clone), or "" to speak freely. Fixed when the take starts, as the
   *  backend's choice of engine is, so it can't change mid-take. */
  recordingReadPrompt: () => string;
  /** v1.23.0: start capturing the dry mic (engine must be running). */
  startRecordingVoice: () => Promise<void>;
  /** v1.23.0: stop the capture and clone it into a voice named `name`, then select it. */
  stopRecordingVoice: (name: string) => Promise<void>;
  /** v1.21.0: whether the (on-demand) voice-cloning models are downloaded. */
  cloneModelsReady: () => boolean;
  /** True while any of the three model downloads runs (v1.57.0: whichever
   *  page started it). */
  cloneDownloading: () => boolean;
  /** Download progress 0–100, of the file being downloaded. */
  cloneDownloadPct: () => number;
  /** v1.57.0: the running model download (which one, and its %), or null.
   *  `cloneDownloading` and `cloneDownloadPct` read it. */
  modelDownload: () => ModelDownload | null;
  /** Refresh whether the cloning models are present. */
  refreshCloneModelsStatus: () => Promise<void>;
  /** Download the cloning models (~157 MB, one-time). */
  downloadCloneModels: () => Promise<void>;
  /** v2: whether VoxCPM accent-preserving cloning is available (models present). */
  voxcpmAvailable: () => boolean;
  /** v2: the fixed sentence the user reads when recording a VoxCPM clone. */
  voxcpmReadPrompt: () => string;
  /** Refresh VoxCPM availability + read prompt. */
  refreshVoxcpmStatus: () => Promise<void>;
  /** Download the VoxCPM accent-cloning models (~1.6 GB, one-time). */
  downloadVoxcpmModels: () => Promise<void>;
  /** v1.30.0: whether DirectML GPU cloning is supported on this platform. */
  voxcpmGpuSupported: () => boolean;
  /** v1.30.0: whether the opt-in fp16 GPU decode model is downloaded. */
  voxcpmGpuModelPresent: () => boolean;
  /** Refresh GPU-cloning support + model presence. */
  refreshVoxcpmGpuStatus: () => Promise<void>;
  /** Download the optional fp16 DirectML decode model (~1.2 GB, one-time). */
  downloadVoxcpmGpuModel: () => Promise<void>;

  // Preset actions
  /** Makes `id` the live preset (nothing for an id not in `presets()`),
   *  with Compare back on A and both slots its chain. v1.60.0: the chain
   *  goes out once, from the effect on the active preset, then the voice
   *  model (D19). `fromTimer`: a timer's switch, which keeps the Presets
   *  screen on the preset you are editing there (D18); anywhere else the
   *  view follows, as for every switch. */
  usePreset: (id: string, opts?: { fromTimer?: boolean }) => void;
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
  /** v1.60.0: the current folder's clips are known: its scan has finished
   *  (or failed), or there is no folder. Until then a clip missing from
   *  `soundboardTiles` may only be not scanned yet (a launch's or a
   *  reload's scan comes after the devices and the presets). */
  soundboardScanned: () => boolean;
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
  /** Open (name) or close (null) a MIDI input port; persists the choice.
   *  v1.58.0: lets go of push-to-modulate's MIDI hold, every mapping's. */
  selectMidiInput: (name: string | null) => Promise<void>;
  /** User's note/CC → action mappings (persisted). */
  midiMappings: () => MidiMapping[];
  addMidiMapping: (mapping: Omit<MidiMapping, "id">) => void;
  /** v1.58.0: lets go of push-to-modulate's MIDI hold for that mapping. */
  updateMidiMapping: (id: string, patch: Partial<MidiMapping>) => void;
  /** v1.58.0: lets go of push-to-modulate's MIDI hold for that mapping. */
  removeMidiMapping: (id: string) => void;
  /** The mapping id currently capturing its trigger via MIDI-learn, or null. */
  midiLearnId: () => string | null;
  /** v1.58.0: entering learn (any id) lets go of push-to-modulate's MIDI
   *  hold, every mapping's: learn swallows the note-offs. */
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

  // What's new (v1.49.0)
  /** Release notes to show, or null when there's nothing to announce.
   *  Content is bundled at build time — no network, no fetch to fail. */
  whatsNewContent: () => WhatsNewContent | null;
  /** The release the post-update banner is advertising, or null. */
  whatsNewBanner: () => ReleaseNote | null;
  /** True while the What's new panel is open. */
  whatsNewOpen: () => boolean;
  /** Open the panel. With no argument (Settings -> About) it shows the
   *  most recent releases rather than only what changed for this user. */
  openWhatsNew: () => void;
  closeWhatsNew: () => void;
  /** Hide the banner without opening the panel. */
  dismissWhatsNewBanner: () => void;
  /** Decide whether this launch should announce anything. Runs once per
   *  process; silent on dev builds and brand-new installs. */
  checkWhatsNew: () => Promise<void>;

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

  // Divora's mute of the voice (v1.56.0). The engine owns it; the store
  // keeps a read-only mirror, ordered by `seq`.
  /** The mirror: true while muted. False until the state is known. */
  muted: () => boolean;
  /** The newest state seen (largest `seq`), or null before any. */
  muteState: () => InputMute | null;
  /** True once the boot read settled, any state was adopted, or 500 ms
   *  passed, so nothing that waits on it can wait for ever. */
  muteKnown: () => boolean;
  /** The mirror's only writer: takes `incoming` only if it is newer. */
  applyMuteState: (incoming: InputMute | null) => void;
  /** Sends `set_input_muted` and adopts the answer. For controls with a
   *  label: they send what the label says. v1.59.0: sent only once the mute
   *  command before it has answered, as `toggleMuted`'s is. */
  setMuted: (on: boolean) => Promise<void>;
  /** Sends `toggle_input_muted` and adopts the answer. For triggers with no
   *  label (a MIDI pad). */
  toggleMuted: () => Promise<void>;

  // Push-to-modulate (v1.58.0). The engine owns it; the store keeps a
  // read-only mirror, ordered by `seq`, as the mute's. v1.59.0: the page's
  // commands go one at a time, each once the one before it has answered.
  /** The mirror: the newest state seen (largest `seq`), or null before any. */
  ptmState: () => PtmState | null;
  /** True once the boot restore settled (answer or failure) or 500 ms
   *  passed. Never set by an adopted tick: a tick that lands before the
   *  boot's answer carries the previous page's state. */
  ptmKnown: () => boolean;
  /** The mirror's `bypassed`: the engine is switching the effects off. Null
   *  reads as applied, the engine's launch state. */
  ptmBypassed: () => boolean;
  /** What the user picked, saved in `divora.ptm`; changes only when an
   *  answer to `set_ptm_mode` lands. */
  ptmSetting: () => PtmSetting;
  /** Turns push-to-modulate on (with the saved hold) or off ("always"). Lets
   *  go of every hold, the global key's included. */
  setPtmOn: (on: boolean) => Promise<void>;
  /** Picks which way the hold goes; while off it is only remembered (the
   *  engine stays at "always"). Lets go of every hold, even when it is the
   *  hold already picked. */
  setPtmHold: (hold: PtmHold) => Promise<void>;
  /** The page holds it for `source`. Sends `set_ptm_held` only if the page
   *  didn't hold it already, so a second down sends nothing. */
  ptmDown: (source: "key" | "button") => void;
  /** The page lets go for `source`. Sends only if the page held it, so a
   *  stray up sends nothing. The shell calls it for both sources when the
   *  window loses the focus or the page is hidden, where no keyup or
   *  pointerup would come. */
  ptmUp: (source: "key" | "button") => void;
  /** The mirror's only writer: takes `incoming` only if it is newer. Never
   *  touches `ptmKnown`. */
  applyPtmState: (incoming: PtmState | null) => void;

  // Hourglass (v1.59.0). Rust keeps the time; the store keeps the stored
  // timers (`divora.timers`) and a read-only mirror of Rust's snapshot,
  // ordered by `seq`. Settings' cards are the stored timers, joined by id
  // with the mirror.
  /** The stored timers, read by `readTimers`: every one, in list order. */
  timerStore: () => StoredTimers;
  /** Each stored timer's role: the first 16 of a known kind run; the rest
   *  are kept, shown with Delete only, and never registered. */
  timerRoles: () => Map<string, TimerRole>;
  /** The mirror: the newest snapshot seen (largest `seq`), or null before
   *  the first answer or event. */
  timerSnapshot: () => TimerSnapshot | null;
  /** The mirror's only writer: an answer or a `timer-state` payload, taken
   *  only if it is newer. Saves a stored timer's run when it changed. */
  adoptTimerSnapshot: (raw: unknown) => void;
  /** Adds a countdown or a count-up and syncs; its id, or null at 16 stored
   *  timers. v1.60.0: `template` "break" (with "countdown") adds Add break
   *  (BRB)'s countdown, "BRB" numbered if taken, 5:00, muting at its start
   *  and unmuting at zero, its lists in the one save and the one sync
   *  (D26); null for a count-up. */
  addTimer: (kind: TimerKind, template?: "break") => Promise<string | null>;
  /** Called on commit only (Enter, or leaving the field), with a name that
   *  passes `readTimerName`. Sends nothing when the name is the one stored,
   *  or the timer isn't one the backend runs. */
  renameTimer: (id: string, name: string) => Promise<void>;
  /** Called on commit only, with a length `parseLength` gave. Sends nothing
   *  for the length stored, a count-up, or a timer the backend doesn't run. */
  setTimerLength: (id: string, ms: number) => Promise<void>;
  /** Unregisters its keys (every verb in `TIMER_VERBS`), takes it off the
   *  overlay, then syncs without it. */
  deleteTimer: (id: string) => Promise<void>;
  /** Settings' main button: arm, pause or resume, by `timerAction` on the
   *  mirror's state; nothing without one. */
  startTimer: (id: string) => Promise<void>;
  pauseTimer: (id: string) => Promise<void>;
  resetTimer: (id: string) => Promise<void>;
  adjustTimer: (id: string, deltaMs: number) => Promise<void>;
  /** v1.60.0: End now, through the timers' queue: a running or paused
   *  countdown fires at once and runs its At zero list. */
  finishTimer: (id: string) => Promise<void>;
  /** v1.60.0: a countdown's At start (`onStart`) and At zero (`onFire`)
   *  lists, edited by index into the stored array so an entry this build
   *  can't read stays where it is (D27); each saves, then syncs. Only for a
   *  countdown the backend runs, and only an action `readAction` reads
   *  (stored as read: a line trimmed). Add is refused unless `canAddAction`
   *  says "ok" and the list offers the kind (Back to the preset before only
   *  at zero); Stop all clips and lines goes to the top. Set changes a
   *  row that runs, to an action of its kind (a new target or line), and
   *  keeps the entry's other keys. Remove takes any row, one that doesn't
   *  run included. */
  addTimerAction: (id: string, hook: TimerHook, action: TimerAction) => Promise<void>;
  setTimerAction: (id: string, hook: TimerHook, index: number, action: TimerAction) => Promise<void>;
  removeTimerAction: (id: string, hook: TimerHook, index: number) => Promise<void>;
  /** v1.60.0: where `id`'s start switched the preset from and to, while it
   *  is stored (`presetReturns`); null without one. */
  timerReturn: (id: string) => PresetReturn | null;
  /** Saves (`""` clears that verb) and registers or unregisters
   *  `tm:<id>:<verb>`. Does nothing for an id that isn't a stored timer the
   *  backend runs. */
  bindTimerHotkey: (id: string, verb: TimerVerb, accelerator: string) => Promise<void>;
  /** The timers' folder for OBS (asked once, then remembered); null when it
   *  can't be had. */
  timersDir: () => Promise<string | null>;
  /** Whether the scene has a `timer` element for this timer. */
  timerOnOverlay: (id: string) => boolean;
  /** Show on overlay: adds the timer's element, or removes every element
   *  for it. */
  setTimerOnOverlay: (id: string, on: boolean) => void;

  // Derived
  hasEnabled: () => boolean;
  /** v1.58.0: "muted" while muted; else "modulated" while an effect other
   *  than the noise gate is on and push-to-modulate isn't switching them
   *  off; else "clean". */
  status: () => VoiceStatus;
}

/** v1.60.0: who an action came from: a glyph cast, or a timer's start or
 *  end (`timer-actions`). */
export type ActionSource = { kind: "glyph" } | { kind: "timer"; id: string; hook: "start" | "end" };

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
  // Restore the last active preset across restarts (persisted below). Falls
  // back to the first preset; if the id no longer exists (a deleted user
  // preset), `refreshPresets` resets it once the real list loads.
  const persistedPresetId = loadJson<string>(STORAGE_KEYS.activePreset, firstPreset.id);
  const [presetId, setPresetId] = createSignal<string>(persistedPresetId);
  const [viewedId, setViewedId] = createSignal<string>(persistedPresetId);
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

  // v1.60.0: the one dispatcher (D1). A glyph's five actions run as they
  // always have. A timer's page half (preset, random preset, back to the
  // preset before, a clip) runs in Hourglass's block (`runTimerAction`);
  // its backend kinds and the glyph's toggles never run from a timer.
  const dispatchAction = (action: GlyphAction | TimerAction, from: ActionSource): void => {
    if (from.kind === "timer") {
      runTimerAction(action, from.id, from.hook);
      return;
    }
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
        // v1.56.0: really mutes. Sends the opposite of the mirror, so the
        // cast does what its omen said.
        void setMuted(!muted());
        break;
      case "panic":
        void panicSoundboard();
        break;
    }
  };
  const dispatchGlyphAction = (action: GlyphAction): void => dispatchAction(action, { kind: "glyph" });

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
        // v1.56.0: the omen names what the cast will send, read from the
        // same mirror `dispatchGlyphAction` reads (SparkLayer resolves and
        // runs in one go), so the two can't disagree.
        return { color: GLYPH_ACCENT, label: muted() ? "Unmute" : "Mute", action };
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
  // the spell circle for OBS; the main window pushes state to it (App.tsx,
  // via overlay/sync.ts).
  //
  // v1.55.0: the backend creates, places and shows the window and owns its
  // mode; main says where it goes and owns the charms. All of that lives in
  // overlay/controller.ts, which also pushes the scene. Its toasts call
  // `showToast` (below) only after an await, once this store is built.
  const overlay = createOverlayController({
    api: {
      open: (rect, fit) => openOverlayWindow({ rect, fit }),
      close: closeOverlayWindow,
      place: placeOverlayWindow,
      setMode: setOverlayModeCmd,
      setHide: setOverlayHideFromRecordings,
      status: overlayStatusCmd,
      importAsset: importOverlayAsset,
      deleteAsset: deleteOverlayAsset,
      pickImage: pickImageFile,
      subscribeStatus: subscribeOverlayStatus,
      subscribeEdit: subscribeOverlayEdit,
    },
    events: {
      emit: async (event, payload) => {
        const { emit } = await import("@tauri-apps/api/event");
        await emit(event, payload);
      },
      listen: async (event, handler) => {
        const { listen } = await import("@tauri-apps/api/event");
        return listen(event, (e) => handler(e.payload));
      },
    },
    windows: tauriOverlayWindows(),
    storage: {
      load: (key) => loadJson<unknown>(key, null),
      save: saveJson,
    },
    toast: (text, tone) => showToast(text, tone),
    newId: () => `c-${randomHex(4)}`,
    session: randomHex(8),
  });

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
  // v1.39.0: true when the backend's device-loss auto-recovery gave up — the
  // engine is stopped with no audio. Confirmed by the ~30 Hz level event.
  const [deviceLost, setDeviceLost] = createSignal(false);

  // ---- v1.46.0: reactive effects ----
  const [modEnv, setModEnv] = createSignal(0);
  const persistedReactive = loadJson<{ enabled: boolean; intensity: number }>(
    STORAGE_KEYS.reactive,
    { enabled: false, intensity: REACTIVE_DEFAULT_INTENSITY },
  );
  const [reactiveEnabled, setReactiveEnabledRaw] = createSignal(
    !!persistedReactive.enabled,
  );
  const [reactiveIntensity, setReactiveIntensityRaw] = createSignal(
    // Clamp on load as well as on write — a hand-edited or corrupt
    // localStorage value would otherwise render as "900%" beside a slider
    // pegged at its maximum.
    clampIntensity(
      typeof persistedReactive.intensity === "number"
        ? persistedReactive.intensity
        : REACTIVE_DEFAULT_INTENSITY,
    ),
  );

  const reactiveHasTarget = (): boolean =>
    chain().some((e) => e.id === RAGE_PRIMARY && e.enabled);

  /**
   * Build the wire routes from the CURRENT chain, reading each target's
   * authored value as the base the modulation offsets from.
   *
   * Deriving them here — rather than storing bases — is what keeps them
   * correct across a preset switch: the backend addresses targets by kind and
   * offsets from whatever base it was handed, so a stale base would make the
   * modulation push the new preset away from the OLD preset's authored value.
   */
  const reactiveRoutes = (): ReactiveRouteSpec[] => {
    const entries = chain();
    return RAGE_TARGETS.flatMap((t) => {
      // Find the first ENABLED entry of this kind, but report its occurrence
      // index among ALL entries of the kind — the backend's `index_of_kind`
      // counts every occurrence, enabled or not. Reporting 0 unconditionally
      // would target a disabled duplicate sitting earlier in the chain.
      let seen = 0;
      for (const entry of entries) {
        if (entry.id !== t.id) continue;
        if (entry.enabled) {
          return [
            {
              kind: t.id as ReactiveRouteSpec["kind"],
              nth: seen,
              key: t.key,
              base: entry.vals[t.key] ?? paramDefault(t.id, t.key),
              depth: t.depth,
            },
          ];
        }
        seen += 1;
      }
      return [];
    });
  };

  const reactiveConfig = (): ReactiveConfig => ({
    enabled: reactiveEnabled(),
    intensity: reactiveIntensity() / 100,
    floorDb: REACTIVE_FLOOR_DB,
    ceilDb: REACTIVE_CEIL_DB,
    attackMs: REACTIVE_ATTACK_MS,
    holdMs: REACTIVE_HOLD_MS,
    releaseMs: REACTIVE_RELEASE_MS,
    // While off, send an empty table: the backend restores each outgoing
    // route to its authored value, and the config then stops changing with
    // the chain, so the dedupe below keeps a disabled feature silent on the
    // wire.
    routes: reactiveEnabled() ? reactiveRoutes() : [],
  });

  // Last config actually sent, so redundant re-sends are dropped. Without
  // this, dragging the Intensity slider would fire a message per input event.
  let lastReactiveSent = "";

  const syncReactive = (): void => {
    if (!engineRunning()) {
      // A fresh engine session starts with no reactive config, so forget what
      // we sent — the next start must re-send rather than dedupe itself away.
      lastReactiveSent = "";
      return;
    }
    const cfg = reactiveConfig();
    const json = JSON.stringify(cfg);
    if (json === lastReactiveSent) return;
    lastReactiveSent = json;
    void setReactiveConfigCmd(cfg);
  };

  const persistReactive = (): void => {
    saveJson(STORAGE_KEYS.reactive, {
      enabled: reactiveEnabled(),
      intensity: reactiveIntensity(),
    });
  };

  // Neither setter syncs explicitly: the effect below tracks every input the
  // config derives from, so a change here re-sends by construction.
  const setReactiveEnabled = (on: boolean): void => {
    setReactiveEnabledRaw(on);
    if (!on) setModEnv(0);
    persistReactive();
  };

  const setReactiveIntensity = (pct: number): void => {
    setReactiveIntensityRaw(clampIntensity(pct));
    persistReactive();
  };

  // ---- Voice reading ----
  //
  // Only the switch is persisted. The rolling baseline the descriptors are
  // relative to is derived from the user's voice and lives in the engine for
  // the life of a session; nothing derived from the voice reaches disk.
  const persistedReading = loadJson<{ enabled: boolean }>(
    STORAGE_KEYS.voiceReading,
    { enabled: false },
  );
  const [voiceReadingOn, setVoiceReadingOnRaw] = createSignal(
    !!persistedReading.enabled,
  );
  const [voiceReading, setVoiceReading] =
    createSignal<VoiceReadingUpdate | null>(null);

  const setVoiceReadingOn = (on: boolean): void => {
    setVoiceReadingOnRaw(on);
    // Drop the reading the moment the switch goes off. Leaving the last
    // window on screen would keep showing a measurement of something that is
    // no longer being measured — the same lie as a decaying readout, just
    // frozen instead of drifting.
    if (!on) setVoiceReading(null);
    saveJson(STORAGE_KEYS.voiceReading, { enabled: on });
    void setVoiceReadingEnabledCmd(on).catch(() => {
      /* browser preview without the Tauri bridge */
    });
  };

  // One subscription for the life of the provider. The backend stays silent
  // while the feature is off, so an inactive panel costs one idle listener
  // rather than a subscribe/unsubscribe race on every toggle.
  let readingUnlisten: (() => void) | null = null;
  void (async () => {
    try {
      readingUnlisten = await subscribeVoiceReading((update) => {
        // Late events can arrive just after the switch goes off; the switch
        // is the authority on whether anything should be shown.
        if (voiceReadingOn()) setVoiceReading(update);
      });
    } catch {
      /* browser preview without the Tauri bridge */
    }
  })();
  onCleanup(() => {
    if (readingUnlisten) readingUnlisten();
    readingUnlisten = null;
  });

  // Push the persisted switch to the backend once, and paint immediately from
  // a one-shot read rather than waiting up to a fifth of a second for the
  // first event.
  createEffect(
    on(voiceReadingOn, (on) => {
      if (!on) return;
      void (async () => {
        try {
          await setVoiceReadingEnabledCmd(true);
          const first = await getVoiceReadingCmd();
          if (voiceReadingOn()) setVoiceReading(first);
        } catch {
          /* browser preview without the Tauri bridge */
        }
      })();
    }),
  );
  const [recordingPath, setRecordingPath] = createSignal<string | null>(null);

  // ---- v1.52.0: screen recording ----
  // The backend's `screen-recording` events are the truth about a take; the
  // store only sets "starting" optimistically so the button answers at once.
  const [captureStatus, setCaptureStatus] = createSignal<CaptureStatus | null>(null);
  const [captureMonitors, setCaptureMonitors] = createSignal<CaptureMonitor[]>([]);
  const [screenRecording, setScreenRecording] =
    createSignal<ScreenRecordingUpdate>(IDLE_SCREEN_RECORDING);
  const [screenRecordingPath, setScreenRecordingPath] = createSignal<string | null>(null);
  const storedScreenSettings = loadJson<unknown>(STORAGE_KEYS.screenRecording, null);
  // A first run: no sign of an earlier launch. The wizard's flag alone isn't
  // proof — "Replay setup" clears it — but every real launch since v1.49.0
  // has recorded its version for What's new.
  const firstRun =
    loadRaw(WIZARD_SEEN_KEY) === null && loadRaw(STORAGE_KEYS.whatsNewSeenVersion) === null;
  const [screenSettings, setScreenSettingsRaw] = createSignal<ScreenSettings>(
    readScreenSettings(storedScreenSettings, firstRun),
  );
  // v1.53.0: what desktop audio defaults to depends on whether this is a
  // first run; settle it now, so finishing the wizard doesn't change it.
  if (storedScreenSettings === null) {
    saveJson(STORAGE_KEYS.screenRecording, screenSettings());
  }
  const [desktopSources, setDesktopSources] = createSignal<DesktopAudioSource[] | null>(null);
  const setScreenSettings = (patch: Partial<ScreenSettings>): void => {
    const next = readScreenSettings({ ...screenSettings(), ...patch });
    setScreenSettingsRaw(next);
    saveJson(STORAGE_KEYS.screenRecording, next);
  };

  // Asked for by the card when it mounts, not here: the first answer starts
  // Media Foundation, which nothing should pay for until a recorder is shown.
  const refreshCapture = async (): Promise<void> => {
    try {
      const status = await screenCaptureStatusCmd();
      if (!status) return; // a bridge that answered nothing (tests, previews)
      setCaptureStatus(status);
      setCaptureMonitors(status.supported ? await listCaptureMonitorsCmd() : []);
      if (status.supported) await refreshDesktopSources();
    } catch (err) {
      console.warn("[screen] capture status unavailable", err);
    }
  };

  /** Ask where desktop audio can come from — and what Divora plays on each
   *  playback device, which follows the engine's routing. */
  const refreshDesktopSources = async (): Promise<DesktopAudioSource[] | null> => {
    try {
      const sources = await listDesktopAudioSourcesCmd();
      if (Array.isArray(sources)) {
        setDesktopSources(sources);
        return sources;
      }
    } catch (err) {
      console.warn("[screen] desktop audio sources unavailable", err);
    }
    return null;
  };
  // What Divora plays where changes whenever the engine starts, stops or
  // moves to other devices: ask again then, not only when the card mounts.
  createEffect(
    on(
      [engineRunning, streamInfo, selectedOutput, selectedMonitorRaw],
      () => {
        if (captureStatus()?.supported) void refreshDesktopSources();
      },
      { defer: true },
    ),
  );

  let screenUnlisten: (() => void) | null = null;
  void (async () => {
    try {
      screenUnlisten = await subscribeScreenRecording((update) => {
        setScreenRecording(update);
        if (update.path) setScreenRecordingPath(update.path);
      });
      // A window opened mid-take paints the take, not "idle".
      setScreenRecording(await screenRecordingStatusCmd());
    } catch {
      /* browser preview without the Tauri bridge */
    }
  })();
  onCleanup(() => {
    if (screenUnlisten) screenUnlisten();
    screenUnlisten = null;
  });

  /** What a pipeline is asked to capture. While Replay is on, exactly what
   *  its ring runs with: a take shares Replay's capture and encoder, and the
   *  backend refuses any other settings as a mismatch. Otherwise the card's
   *  screen and desktop audio, and Settings' rows. A string is why there is
   *  nothing to ask for. */
  const pipelineOptions = async (): Promise<Omit<ReplayArmOptions, "seconds"> | string> => {
    const ring = replayStatus();
    if (replayOn(ring) && ring.armed && ring.monitorId) {
      const a = ring.armed;
      return {
        monitorId: ring.monitorId,
        fps: a.fps === 60 ? 60 : 30,
        quality: a.quality,
        cursor: a.cursor,
        avOffsetMs: a.avOffsetMs,
        ...(a.desktopAudio ? { desktopAudio: a.desktopAudio } : {}),
        desktopGainDb: a.desktopGainDb,
      };
    }
    if (!captureStatus()) await refreshCapture();
    const status = captureStatus();
    if (!status?.supported) return status?.reason ?? "Screen recording isn't available.";
    let monitors = captureMonitors();
    if (monitors.length === 0) {
      await refreshCapture();
      monitors = captureMonitors();
    }
    const settings = screenSettings();
    const monitor = pickMonitor(monitors, settings.monitorId);
    if (!monitor) return "No screen to record.";
    // The pipeline records what the card shows: the saved desktop audio only
    // if it is offered right now — never some other source in its place.
    let desktopAudio: string | undefined;
    if (settings.desktopAudio !== "off") {
      const sources = (await refreshDesktopSources()) ?? desktopSources() ?? [];
      desktopAudio = pickDesktopSource(sources, settings.desktopAudio)?.id;
    }
    return {
      monitorId: monitor.id,
      fps: settings.fps,
      quality: settings.quality,
      cursor: settings.cursor,
      avOffsetMs: settings.avOffsetMs,
      ...(desktopAudio ? { desktopAudio } : {}),
      desktopGainDb: settings.desktopGainDb,
    };
  };

  const toggleScreenRecording = async (): Promise<void> => {
    const phase = screenRecording().state;
    if (phase === "stopping") return;
    if (phase === "recording" || phase === "starting") {
      try {
        setScreenRecordingPath(await stopScreenRecordingCmd());
      } catch (err) {
        console.warn("[screen] stop failed", err);
      }
      return;
    }
    // Refused with the WAV recorder's own words.
    if (!engineRunning()) {
      setEngineError("Start the engine before recording.");
      return;
    }
    const opts = await pipelineOptions();
    if (typeof opts === "string") {
      setScreenRecording({ ...IDLE_SCREEN_RECORDING, error: opts });
      return;
    }
    setScreenRecording({ ...IDLE_SCREEN_RECORDING, state: "starting" });
    try {
      const path = await startScreenRecordingCmd({
        ...opts,
        filename: screenFilename(new Date()),
        // The separate WAV files are the take's own, not the pipeline's.
        stems: screenSettings().stems,
      });
      setScreenRecordingPath(path);
    } catch (err) {
      setScreenRecording({ ...IDLE_SCREEN_RECORDING, error: String(err) });
    }
  };

  // ---- v1.54.0: Replay ----
  // The backend's `replay-buffer` events are the truth about the ring; the
  // store keeps only the length and whether the card's explainer has been
  // read. Replay is off at every launch: nothing persisted can turn it on.
  const [toast, setToast] = createSignal<ToastMessage | null>(null);
  let toastSeq = 0;
  const showToast = (text: string, tone: ToastTone = "success"): void => {
    toastSeq += 1;
    setToast({ id: toastSeq, text, tone });
  };

  const [replayStatus, setReplayStatus] = createSignal<ReplayStatus>(OFF_REPLAY_STATUS);
  const [replaySettings, setReplaySettingsRaw] = createSignal<ReplaySettings>(
    readReplaySettings(loadJson<unknown>(STORAGE_KEYS.replayBuffer, null)),
  );
  const [replayMemory, setReplayMemory] = createSignal<ReplayMemory | null>(null);
  // `readReplaySettings` drops every other key, so an `armed` flag can never
  // be written here, whoever calls this.
  const saveReplaySettings = (patch: Partial<ReplaySettings>): void => {
    const next = readReplaySettings({ ...replaySettings(), ...patch });
    setReplaySettingsRaw(next);
    saveJson(STORAGE_KEYS.replayBuffer, next);
  };

  /** The screen and settings the memory figures are for: Replay's own
   *  while it is on, the card's and Settings' otherwise. */
  const replayMemoryAsk = (): ReplayMemoryOptions | null => {
    const ring = replayStatus();
    if (replayOn(ring) && ring.armed && ring.monitorId) {
      return {
        monitorId: ring.monitorId,
        fps: ring.armed.fps === 60 ? 60 : 30,
        quality: ring.armed.quality,
      };
    }
    if (!captureStatus()?.supported) return null;
    const s = screenSettings();
    const monitor = pickMonitor(captureMonitors(), s.monitorId);
    return monitor ? { monitorId: monitor.id, fps: s.fps, quality: s.quality } : null;
  };
  let memoryAsks = 0;
  const refreshReplayMemory = async (): Promise<void> => {
    const opts = replayMemoryAsk();
    if (!opts) return;
    const ask = ++memoryAsks;
    try {
      const memory = await replayMemoryCmd(opts);
      // Only the newest answer: an older one is for settings no longer shown.
      if (ask === memoryAsks && memory && Array.isArray(memory.lengths)) {
        setReplayMemory(memory);
      }
    } catch (err) {
      console.warn("[replay] memory figures unavailable", err);
    }
  };
  const replayMemoryKey = createMemo(() => {
    const o = replayMemoryAsk();
    return o ? `${o.monitorId}|${o.fps}|${o.quality}` : null;
  });
  createEffect(
    on(replayMemoryKey, (key) => {
      if (key) void refreshReplayMemory();
    }),
  );

  // When the ring was last held (on this clock), while it still is: a held
  // ring's clip ends there, not at the save, so whether the voice was
  // disguised throughout is judged back from then.
  let heldAt: number | null = null;
  createEffect(
    on(replayStatus, (s) => {
      if (s.state !== "held") heldAt = null;
      else heldAt ??= performance.now();
    }),
  );

  /** Every save attempt ends in one `replay-saved`, refusals included, so
   *  this is the only place a save's toast comes from. */
  const onReplaySaved = (saved: ReplaySaved): void => {
    if (saved.error) {
      showToast(saved.error, "error");
      return;
    }
    const end = heldAt ?? performance.now();
    const named = clipWasModulated(modulatedSince(), end, saved.durationMs);
    showToast(savedToast(saved, named ? preset().name : null), "success");
  };

  const replayUnlisten: Array<() => void> = [];
  // An event, or an arm's or disarm's answer, is newer than the launch pull
  // below whenever it lands first.
  let replayHeard = false;
  let replayDisposed = false;
  const takeReplayStatus = (s: ReplayStatus): void => {
    replayHeard = true;
    setReplayStatus(s);
  };
  void (async () => {
    // All three are asked for at once; the pull — so a window opened while
    // Replay runs shows it on, and locks what it locks — counts only if
    // nothing newer has come in meanwhile. Each failure (a browser preview
    // without the Tauri bridge) is caught where it starts, so none goes
    // unhandled while another is awaited.
    const buffer = subscribeReplayBuffer(takeReplayStatus).catch(() => null);
    const saved = subscribeReplaySaved(onReplaySaved).catch(() => null);
    const pull = replayBufferStatusCmd().catch(() => null);
    for (const un of [await buffer, await saved]) {
      if (!un) continue;
      if (replayDisposed) un();
      else replayUnlisten.push(un);
    }
    const now = await pull;
    if (now && !replayHeard) setReplayStatus(now);
  })();
  onCleanup(() => {
    replayDisposed = true;
    for (const un of replayUnlisten.splice(0)) un();
  });

  const armReplay = async (): Promise<void> => {
    // The explainer gate. It lives here, not in any caller, so a hotkey —
    // or any later way of turning Replay on — can't skip what the card
    // explains: what is kept, calls included, and that nothing is written
    // until Save.
    const ring = replayStatus();
    const desktopOn =
      replayOn(ring) && ring.armed
        ? ring.armed.desktopAudio !== null
        : screenSettings().desktopAudio !== "off";
    if (explainerNeeded(replaySettings(), desktopOn)) {
      showToast(REPLAY_GATE_TOAST, "error");
      return;
    }
    // Refused with the recorders' own words.
    if (!engineRunning()) {
      setEngineError("Start the engine before recording.");
      return;
    }
    const opts = await pipelineOptions();
    if (typeof opts === "string") {
      showToast(opts, "error");
      return;
    }
    try {
      const next = await armReplayBufferCmd({ ...opts, seconds: replaySettings().seconds });
      if (next) takeReplayStatus(next);
    } catch (err) {
      showToast(String(err), "error");
    }
  };

  const disarmReplay = async (): Promise<void> => {
    try {
      const next = await disarmReplayBufferCmd();
      if (next) takeReplayStatus(next);
    } catch (err) {
      showToast(String(err), "error");
    }
  };

  const toggleReplay = (): Promise<void> =>
    replayOn(replayStatus()) ? disarmReplay() : armReplay();

  const saveReplay = async (): Promise<void> => {
    try {
      await saveReplayCmd();
    } catch {
      /* the replay-saved event says why */
    }
  };

  const setReplaySeconds = async (seconds: ReplaySeconds): Promise<void> => {
    saveReplaySettings({ seconds });
    // A running ring is resized — through armReplay, like every arm. A held
    // one keeps what it has; the new length applies when it next runs.
    const ring = replayStatus();
    if ((ring.state === "armed" || ring.state === "arming") && ring.seconds !== seconds) {
      await armReplay();
    }
  };

  // …and when it next runs, it runs at the length it kept: the backend
  // resumes a held ring as it was. So once it runs again, and the engine with
  // it (which comes first is a race), a length picked meanwhile is applied —
  // once, through armReplay like every arm.
  let resizeOnResume = false;
  createEffect(
    on([replayStatus, engineRunning], ([ring, running]) => {
      if (ring.state === "held") {
        resizeOnResume = true;
        return;
      }
      if (ring.state === "off") {
        resizeOnResume = false;
        return;
      }
      if (!resizeOnResume || !running) return;
      resizeOnResume = false;
      if (ring.seconds !== replaySettings().seconds) void armReplay();
    }),
  );

  const markReplayExplained = (desktop: boolean): void => {
    saveReplaySettings({ explained: true, ...(desktop ? { explainedDesktop: true } : {}) });
  };

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

  // v1.56.0: Divora's mute of the voice. The engine's word is the only
  // truth; this is a read-only mirror of it. Command answers, the boot read
  // and level ticks can land in any order, so the mirror keeps the state with
  // the largest `seq` and ignores older ones.
  const [muteState, setMuteState] = createSignal<InputMute | null>(null);
  const [muteKnown, setMuteKnown] = createSignal(false);
  const muted = createMemo(() => muteState()?.muted === true);
  const applyMuteState = (incoming: InputMute | null): void => {
    const next = adoptMute(muteState(), incoming);
    if (next !== muteState()) setMuteState(next);
    if (next !== null) setMuteKnown(true);
  };
  // Neither command can fail in the app; outside Tauri (or a mock without
  // them) the click changes nothing. v1.59.0: both go through one queue
  // (data/inOrder.ts), each sent only once the one before it answered:
  // invokes in flight together can reach Rust out of send order (measured
  // in the real app with 40 sent at once), and two labelled sets landing
  // reversed would leave the opposite of the last click.
  const muteQueue = inOrder();
  const setMuted = async (on: boolean): Promise<void> => {
    try {
      applyMuteState(readInputMute(await muteQueue(() => setInputMutedCmd(on))));
    } catch (e) {
      console.warn("[mute] set_input_muted failed:", e);
    }
  };
  const toggleMuted = async (): Promise<void> => {
    try {
      applyMuteState(readInputMute(await muteQueue(() => toggleInputMutedCmd())));
    } catch (e) {
      console.warn("[mute] toggle_input_muted failed:", e);
    }
  };
  // The boot read. Sent here, at creation, so it is sent, though not
  // necessarily run, before App's onMount enumerates devices (sync commands
  // that hold the main thread), and a reload during a mute shows it in one
  // round trip. It only reads: a reload must never unmute the call.
  // `muteKnown` comes true when it settles, or after 500 ms if it never
  // does. Not queued: it changes nothing, and its answer is adopted by `seq`
  // wherever it lands.
  const muteKnownTimer = setTimeout(() => setMuteKnown(true), 500);
  void (async () => {
    try {
      applyMuteState(readInputMute(await inputMuteCmd()));
    } catch {
      /* browser preview without the Tauri bridge */
    } finally {
      setMuteKnown(true);
    }
  })();
  onCleanup(() => clearTimeout(muteKnownTimer));

  // v1.58.0: push-to-modulate. The engine's word (mode + a held bit per
  // source + `seq`) is the only truth; this is a read-only mirror of it,
  // ordered by `seq` as the mute's is. The page never writes a mode or a
  // hold it guessed: `divora.ptm` is saved from the engine's answer.
  const [ptmState, setPtmState] = createSignal<PtmState | null>(null);
  const [ptmKnown, setPtmKnown] = createSignal(false);
  const [ptmSetting, setPtmSettingRaw] = createSignal<PtmSetting>(
    readPtmSetting(loadJson<unknown>(STORAGE_KEYS.ptm, null)),
  );
  const ptmBypassed = createMemo(() => ptmState()?.bypassed === true);
  const applyPtmState = (incoming: PtmState | null): void => {
    const next = adoptPtm(ptmState(), incoming);
    if (next !== ptmState()) setPtmState(next);
  };
  // What the page itself holds, so it sends only edges: a second down, an
  // autorepeat or a stray up sends nothing. The `midi` source is held while
  // any `ptm` mapping is down; each CC mapping's side (up or down) is kept so
  // a CC acts only when it crosses (the mute's `muteCcStep`).
  const ptmPageHeld = { key: false, button: false };
  const ptmMidiDown = new Set<string>();
  const ptmCcSides = new Map<string, boolean>();
  // v1.59.0: every `set_ptm_held` and `set_ptm_mode` the page sends, the
  // boot restore's included, goes through this one queue: each is sent only
  // once the one before it answered. Invokes in flight together can reach
  // Rust out of send order (measured in the real app with 40 sent at once),
  // and a release applied before its press would leave the hold on until
  // the next edge.
  const ptmQueue = inOrder();
  const sendPtmHeld = async (source: PtmPageSource, held: boolean): Promise<void> => {
    try {
      applyPtmState(readPtmState(await ptmQueue(() => setPtmHeldCmd(source, held))));
    } catch (e) {
      console.warn("[ptm] set_ptm_held failed:", e);
    }
  };
  // The flag is set before the command is queued, and the queue sends a
  // release only once its press has answered, so a press can't overtake its
  // release.
  const ptmDown = (source: "key" | "button"): void => {
    if (ptmPageHeld[source]) return;
    ptmPageHeld[source] = true;
    void sendPtmHeld(source, true);
  };
  const ptmUp = (source: "key" | "button"): void => {
    if (!ptmPageHeld[source]) return;
    ptmPageHeld[source] = false;
    void sendPtmHeld(source, false);
  };
  /** Runs `change` on the set of `ptm` mappings that are down, and sends the
   *  `midi` source only when the set turns empty or non-empty. */
  const changePtmMidi = (change: () => void): void => {
    const before = ptmMidiDown.size > 0;
    change();
    const after = ptmMidiDown.size > 0;
    if (after !== before) void sendPtmHeld("midi", after);
  };
  // The card's set. This very call lets go of every hold in the engine, and
  // the queue sends a press made while it is in flight only after its
  // answer, so the page forgets its own holds now: that press lands after
  // it and holds again. The CC
  // sides are kept, so a CC left up must come down and back up to hold.
  // Saved only from the answer: on failure the setting and the card stay.
  // What is saved builds on the setting as it stands when the answer lands,
  // not when the request went out: an off sent while a hold pick is still in
  // flight was built from the old hold, and must keep the pick that landed
  // before it. `picked` is the hold a hold pick asked for, so one made while
  // off is still remembered.
  const setPtmTo = async (want: PtmSetting, picked?: PtmHold): Promise<void> => {
    ptmPageHeld.key = false;
    ptmPageHeld.button = false;
    ptmMidiDown.clear();
    try {
      const mode = ptmModeOf(want);
      const answer = readPtmState(await ptmQueue(() => setPtmModeCmd(mode)));
      if (answer === null) return;
      applyPtmState(answer);
      const now = ptmSetting();
      const next = settingOf(answer.mode, picked === undefined ? now : { ...now, hold: picked });
      setPtmSettingRaw(next);
      saveJson(STORAGE_KEYS.ptm, next);
    } catch (e) {
      console.warn("[ptm] set_ptm_mode failed:", e);
    }
  };
  const setPtmOn = (on: boolean): Promise<void> => setPtmTo({ ...ptmSetting(), on });
  const setPtmHold = (hold: PtmHold): Promise<void> =>
    setPtmTo({ ...ptmSetting(), hold }, hold);
  // The boot restore. Sent here, at creation, beside the mute's read, so it
  // is sent, though not necessarily run, before App's onMount enumerates
  // devices. It restores the saved mode and lets go of the page's holds (a
  // new page holds nothing) but keeps a global key that is still held: a
  // reload must not drop it. Its answer is the boot read. `ptmKnown` comes
  // true when it settles, or after 500 ms if it never does. It is the
  // queue's first, so it goes out at once, and a press made before its
  // answer goes after it: the restore can't let go of that press.
  const ptmKnownTimer = setTimeout(() => setPtmKnown(true), 500);
  void (async () => {
    const mode = ptmModeOf(ptmSetting());
    try {
      applyPtmState(readPtmState(await ptmQueue(() => setPtmModeCmd(mode, true))));
    } catch (err) {
      console.warn("[ptm] boot restore failed", err);
    } finally {
      setPtmKnown(true);
    }
  })();
  onCleanup(() => clearTimeout(ptmKnownTimer));

  // v1.59.0: Hourglass. Rust keeps the time (src-tauri/src/timers.rs); the
  // page keeps the stored timers (`divora.timers`) and a read-only mirror of
  // Rust's snapshot, ordered by `seq` as the mute's is. The page never
  // decides a timer's state: every answer and every `timer-state` event is
  // the whole set, and the newest one wins.
  const [timerStore, setTimerStore] = createSignal<StoredTimers>(
    readTimers(loadJson<unknown>(STORAGE_KEYS.timers, null)),
  );
  // A user who never added a timer has no key, and boot sends nothing new.
  const hourglassUsed = loadRaw(STORAGE_KEYS.timers) !== null;
  const timerRoles = createMemo(() => rolesOf(timerStore()));
  const [timerSnapshot, setTimerSnapshot] = createSignal<TimerSnapshot | null>(null);
  // Set when the provider goes: a late answer or event then changes nothing,
  // and never writes storage.
  let timersDisposed = false;
  // v1.60.0: three listeners (`timer-state`, `timer-actions`,
  // `soundboard-stopped`), let go together.
  let timerUnlistens: Array<() => void> = [];
  onCleanup(() => {
    timersDisposed = true;
    for (const un of timerUnlistens) un();
    timerUnlistens = [];
  });

  const saveTimers = (next: StoredTimers): void => {
    setTimerStore(next);
    saveJson(STORAGE_KEYS.timers, timersForSave(next));
  };
  /** The stored timers with `id`'s entry replaced; every other entry keeps
   *  its object, so a card for it isn't rebuilt. */
  const withTimer = (s: StoredTimers, id: string, next: StoredTimer): StoredTimers => ({
    ...s,
    timers: s.timers.map((t) => (t.id === id ? next : t)),
  });
  /** A stored timer the backend runs, or undefined. */
  const runTimer = (id: string): StoredTimer | undefined =>
    timerRoles().get(id) === "run" ? timerStore().timers.find((t) => t.id === id) : undefined;

  // Each run is saved only when it changes in an adopted snapshot: a start,
  // a pause, a resume, an adjust, a fire, a reset. A running timer's text
  // changes every second, and that alone writes nothing (its anchor holds
  // still). Only timers still stored are written: a late snapshot never
  // brings back a deleted one.
  const persistRuns = (snap: TimerSnapshot): void => {
    const s = timerStore();
    const byId = new Map(snap.timers.map((t) => [t.id, t]));
    let changed = false;
    const timers = s.timers.map((t) => {
      const state = byId.get(t.id);
      if (!state) return t;
      const run = runOf(state);
      if (sameRun(t.run, run)) return t;
      changed = true;
      const next: StoredTimer = { ...t };
      if (run) next.run = run;
      else delete next.run;
      return next;
    });
    if (changed) saveTimers({ ...s, timers });
  };
  const adoptTimerSnapshot = (raw: unknown): void => {
    if (timersDisposed) return;
    const prev = timerSnapshot();
    const next = adoptTimers(prev, readTimerSnapshot(raw));
    if (next === prev || next === null) return;
    setTimerSnapshot(next);
    persistRuns(next);
    // v1.60.0: a mirror that arrives only after everything else did (the
    // boot sync and the pull both failed) can still let the one reconcile
    // run. Never settles a return by itself: only events do (AT8).
    tryReconcile();
  };

  // Every command that changes Hourglass in the backend goes through this
  // one queue, each sent only once the one before it answered: the syncs,
  // the labelled commands, and the `tm:<id>:<verb>` keys' registers and
  // unregisters. Invokes in flight together can reach Rust out of send order
  // (measured in the real app with 40 sent at once), so a Start and a Reset
  // clicked quickly could land reversed and leave the timer as the first
  // click left it, and a delete's unregister could land after a key bound
  // for the timer that took its id. `timer_status` isn't queued: it changes
  // nothing, and its answer is adopted by `seq` wherever it lands, so it
  // needn't wait behind a slow command.
  const timerQueue = inOrder();

  // Every stored timer, in list order, with its run as `resume`: the backend
  // seeds a timer from it only for an id it hasn't accepted since it started
  // (a restart); after a page reload it knows them all and keeps its own.
  // The set is read when the sync is queued, not when it goes, so each sync
  // carries the timers as the change that sent it left them.
  const sendTimerSync = async (): Promise<void> => {
    const defs = defsForSync(timerStore());
    try {
      adoptTimerSnapshot(await timerQueue(() => syncTimersCmd(defs)));
    } catch (e) {
      console.warn("[hourglass] sync_timers failed:", e);
    }
  };

  // The page hears `timer-state` from here on. `listen` is itself a command,
  // and an event reaches only a listener already registered, so a change
  // made before it was (a restored countdown running out) would reach
  // nobody, and a paused or done timer sends nothing more: once listening,
  // one `timer_status` closes that gap. Once per page. v1.60.0: with
  // `timer-actions` (a countdown's start or end, with the page's half of
  // its actions) and `soundboard-stopped` (a timer's Stop all clips and
  // lines), all three registered before the pull. No pull brings back a
  // `timer-actions` the page missed: a preset return is caught up by the
  // one reconcile below (§7.6), and a missed clip is lost (D16).
  let timersSubscribed = false;
  let timersListening = false;
  const listenFor = async (what: string, listen: () => Promise<() => void>): Promise<void> => {
    try {
      const un = await listen();
      if (timersDisposed) un();
      else timerUnlistens.push(un);
    } catch (e) {
      console.warn(`[hourglass] listening to ${what} failed:`, e);
    }
  };
  const subscribeTimers = async (): Promise<void> => {
    if (timersSubscribed) return;
    timersSubscribed = true;
    // Wrapped: the handlers are defined further down, and run only on an
    // event, once this store is built.
    await Promise.all([
      listenFor("timer-state", () => subscribeTimerState(adoptTimerSnapshot)),
      listenFor("timer-actions", () => subscribeTimerActions((raw) => runTimerActions(raw))),
      listenFor("soundboard-stopped", () => subscribeSoundboardStopped(() => clearPlayingClips())),
    ]);
    timersListening = true;
    try {
      adoptTimerSnapshot(await timerStatusCmd());
    } catch (e) {
      console.warn("[hourglass] timer_status failed:", e);
    }
    tryReconcile();
  };

  // The boot sync. Sent here, at creation, beside the mute's read and the
  // push-to-modulate restore, so it is sent, though not necessarily run,
  // before App's onMount enumerates devices (sync commands that hold the
  // main thread). It is the queue's first, and the timers' keys App
  // registers go through the queue after it: the backend knows every timer
  // before any of their keys works. v1.60.0: once it has answered (or
  // failed), the stored preset returns may be reconciled: a `timer_status`
  // answer that landed before it may not hold the stored timers yet.
  let bootSynced = false;
  if (hourglassUsed) {
    void sendTimerSync().then(() => {
      bootSynced = true;
      tryReconcile();
    });
    void subscribeTimers();
  }

  const addTimer: AppState["addTimer"] = async (kind, template) => {
    // v1.60.0: Add break (BRB) is a countdown (D26); count-ups have no
    // lists (D32).
    if (template !== undefined && (template !== "break" || kind !== "countdown")) return null;
    const s = timerStore();
    // Every stored timer counts, a newer kind's too, so each one counted has
    // a card with Delete.
    if (s.timers.length >= MAX_TIMERS) return null;
    const id = nextTimerId(s);
    let timer: StoredTimer = {
      id,
      name: defaultName(s, kind),
      kind,
      durationMs: DEFAULT_LENGTH_MS,
      hotkeys: {},
    };
    // The template's name, length and lists go in this one save, so one
    // sync carries them all (not an add, a rename and an edit).
    if (template === "break") {
      const b = breakTemplate();
      timer = {
        ...timer,
        name: freshNameFrom(s, b.name),
        durationMs: b.durationMs,
        onStart: b.onStart,
        onFire: b.onFire,
      };
    }
    saveTimers({ ...s, timers: [...s.timers, timer] });
    const synced = sendTimerSync();
    void subscribeTimers();
    await synced;
    return id;
  };

  const renameTimer: AppState["renameTimer"] = async (id, raw) => {
    const name = readTimerName(raw);
    const t = runTimer(id);
    if (name === null || !t || t.name === name) return;
    saveTimers(withTimer(timerStore(), id, { ...t, name }));
    await sendTimerSync();
  };

  const setTimerLength: AppState["setTimerLength"] = async (id, ms) => {
    if (!Number.isSafeInteger(ms) || ms < MIN_LENGTH_MS || ms > MAX_LENGTH_MS) return;
    const t = runTimer(id);
    if (!t || t.kind !== "countdown" || t.durationMs === ms) return;
    saveTimers(withTimer(timerStore(), id, { ...t, durationMs: ms }));
    await sendTimerSync();
  };

  const deleteTimer: AppState["deleteTimer"] = async (id) => {
    const s = timerStore();
    const doomed = s.timers.find((t) => t.id === id);
    if (!doomed) return;
    let after: StoredTimers = { ...s, timers: s.timers.filter((t) => t.id !== id) };
    // v1.60.0: its preset return (D17), decided before the one save. A
    // countdown running or paused gets a cancel from the backend, whose
    // event switches back when its At zero has Back to the preset before:
    // the return is kept for it. Without one, the cancel would only drop
    // it (R2), so it goes now. One that isn't running is settled now (R2 or
    // §7.3), its switch once the preset list has loaded. So a return left
    // for a timer no longer stored always means "switch back", which is
    // what reconcile does with one (§7.6).
    const returns = readReturns(s.presetReturns);
    const ret = returns.find((r) => r.timer === id);
    let switchTo: string | null = null;
    let settleLater = false;
    if (ret) {
      const phase = timerSnapshot()?.timers.find((t) => t.id === id)?.phase;
      const active = phase === "running" || phase === "paused";
      if (!revertsAtZero(doomed)) after = withReturns(after, returns.filter((r) => r !== ret));
      else if (!active && presetsLoaded()) {
        const plan = revertPlan(returns, id, presetId(), presetExists, true);
        after = withReturns(after, plan.returns);
        switchTo = plan.switchTo;
      } else if (!active) settleLater = true;
    }
    // A delete can move a timer that was over the limit up to "run" (D36):
    // the backend runs it from this sync on, so its stored keys go out too.
    const before = rolesOf(s);
    const promoted = [...rolesOf(after)]
      .filter(([tid, role]) => role === "run" && before.get(tid) !== "run")
      .map(([tid]) => tid);
    // The card goes at once, so a capture still open can't bind its keys
    // again (`bindTimerHotkey` needs a stored timer). Then its keys, before
    // the sync: one left registered would start whatever timer takes this id
    // next. All of it is queued in this one call, the sync carrying the
    // timers without the id. An Add made before these have gone takes the
    // id, but its sync is queued after this one, so the backend sees the id
    // go before it comes back and never hands the new timer the deleted
    // one's run; a key bound for the new timer is registered after these
    // unregisters.
    saveTimers(after);
    if (switchTo !== null) switchForReturn(switchTo);
    if (settleLater) whenPresetsLoaded(() => settleReturn(id, true));
    const keys = TIMER_VERBS.map((verb) =>
      timerQueue(() => unregisterGlobalShortcutCmd(timerHotkeyId(id, verb))).catch((err: unknown) => {
        console.warn("[hourglass] unregister hotkey failed", err);
      }),
    );
    overlay.removeOverlayTimer(id);
    const synced = sendTimerSync();
    // After the sync, so the backend runs the timer before its keys work.
    const registered = promoted.flatMap((tid) =>
      TIMER_VERBS.map((verb) => {
        const accelerator = runTimer(tid)?.hotkeys?.[verb];
        if (!accelerator) return Promise.resolve();
        return timerQueue(() => registerGlobalShortcutCmd(timerHotkeyId(tid, verb), accelerator)).catch(
          (err: unknown) => {
            console.warn(`[hourglass] register hotkey failed for ${tid} ${verb}`, err);
          },
        );
      }),
    );
    await Promise.all([...keys, synced, ...registered]);
  };

  /** A timer command, through the queue; its answer is a whole snapshot
   *  like any event. */
  const sendTimerOp = async (what: string, send: () => Promise<unknown>): Promise<void> => {
    try {
      adoptTimerSnapshot(await timerQueue(send));
    } catch (e) {
      console.warn(`[hourglass] ${what} failed:`, e);
    }
  };
  // Settings' main button sends what its label says, from the mirror the
  // label was drawn from; without state from the backend there is no
  // button, and nothing is sent.
  const startTimer: AppState["startTimer"] = async (id) => {
    const state = timerSnapshot()?.timers.find((t) => t.id === id);
    if (!state) return;
    const { cmd } = timerAction(state);
    if (cmd === "arm") await sendTimerOp("arm_timer", () => armTimerCmd(id));
    else if (cmd === "pause") await sendTimerOp("pause_timer", () => pauseTimerCmd(id));
    else await sendTimerOp("resume_timer", () => resumeTimerCmd(id));
  };
  const pauseTimer: AppState["pauseTimer"] = (id) =>
    sendTimerOp("pause_timer", () => pauseTimerCmd(id));
  const resetTimer: AppState["resetTimer"] = (id) =>
    sendTimerOp("reset_timer", () => resetTimerCmd(id));
  const adjustTimer: AppState["adjustTimer"] = (id, deltaMs) =>
    sendTimerOp("adjust_timer", () => adjustTimerCmd(id, deltaMs));
  // v1.60.0: End now goes through the same queue, so an End now clicked
  // right after a Pause lands after it. Its answer is adopted like any
  // other; what its At zero list does comes as a `timer-actions` event.
  const finishTimer: AppState["finishTimer"] = (id) =>
    sendTimerOp("finish_timer", () => finishTimerCmd(id));

  // v1.60.0: a countdown's two lists, edited by index into the stored array
  // (D27): an entry this build can't read (a newer Divora's, a hand edit)
  // stays where it is through every edit, and counts toward the 8. Each
  // change is saved, then synced: the sync carries the actions Rust will
  // read (`defsForSync`), and an At zero edit counts for a run that is
  // going on (Q12).
  const runCountdown = (id: string): StoredTimer | undefined => {
    const t = runTimer(id);
    return t?.kind === "countdown" ? t : undefined;
  };
  const saveList = async (t: StoredTimer, hook: TimerHook, list: unknown[]): Promise<void> => {
    saveTimers(withTimer(timerStore(), t.id, { ...t, [hook]: list }));
    await sendTimerSync();
  };
  const addTimerAction: AppState["addTimerAction"] = async (id, hook, action) => {
    const t = runCountdown(id);
    const a = readAction(action);
    if (!t || a === null) return;
    if (!ACTION_KINDS[a.kind].at.includes(hook === "onStart" ? "start" : "zero")) return;
    if (canAddAction(t[hook], a.kind) !== "ok") return;
    await saveList(t, hook, withAdded(t[hook], a));
  };
  const setTimerAction: AppState["setTimerAction"] = async (id, hook, index, action) => {
    const t = runCountdown(id);
    const a = readAction(action);
    if (!t || a === null) return;
    const row = readStoredActions(t[hook])[index];
    // A row that runs, to an action of its kind: the card changes a target
    // or a line, never a kind, so this can't make a second line.
    if (!row?.runs || row.action.kind !== a.kind) return;
    if (JSON.stringify(row.action) === JSON.stringify(a)) return;
    const entry = (t[hook] as unknown[])[index];
    // The entry's other keys (a later build's) are kept; its own fields are
    // the new ones.
    const next = typeof entry === "object" && entry !== null ? { ...(entry as Record<string, unknown>), ...a } : a;
    await saveList(t, hook, withReplaced(t[hook], index, next as TimerAction));
  };
  const removeTimerAction: AppState["removeTimerAction"] = async (id, hook, index) => {
    const t = runCountdown(id);
    if (!t || !Array.isArray(t[hook]) || !Number.isInteger(index) || index < 0) return;
    if (index >= (t[hook] as unknown[]).length) return;
    await saveList(t, hook, withRemoved(t[hook], index));
  };
  const timerReturn: AppState["timerReturn"] = (id) =>
    readReturns(timerStore().presetReturns).find((r) => r.timer === id) ?? null;

  // ---- v1.60.0: what a countdown's start and end do on the page ----
  // Rust decides what runs (timer_actions.rs `plan`) and runs the backend's
  // half itself: the mute, Stop all clips and lines, a replay, a line. The
  // page hears `timer-actions` for its half (D16): clips, which play at
  // once, and preset work (a switch, a random switch, Back to the preset
  // before), which waits until the real preset list has loaded: until then
  // `presets()` is the bundled list, a user preset would read as gone, and
  // `usePreset` would ignore it. Waiting work runs in the order it came,
  // the first time the list loads. The page never sends a command for a
  // backend kind.

  /** `presetReturns` as `returns`; the key goes when none is left. */
  const withReturns = (s: StoredTimers, returns: PresetReturn[]): StoredTimers => {
    const next: StoredTimers = { ...s };
    if (returns.length > 0) next.presetReturns = returns;
    else delete next.presetReturns;
    return next;
  };
  const saveReturns = (returns: PresetReturn[]): void => {
    if (timersDisposed) return;
    saveTimers(withReturns(timerStore(), returns));
  };
  /** A preset in the loaded list. */
  const presetExists = (id: string): boolean => presets().some((p) => p.id === id);
  /** A switch back. Nothing when it is the live preset already (a timer
   *  that "switched" to the preset you were on): a switch would only put
   *  Compare back on A. */
  const switchForReturn = (id: string): void => {
    if (id !== presetId()) usePreset(id, { fromTimer: true });
  };

  const presetWork: Array<() => void> = [];
  const whenPresetsLoaded = (work: () => void): void => {
    if (presetsLoaded() && presetWork.length === 0) work();
    else presetWork.push(work);
  };
  /** Runs the preset work that waited for the list, in arrival order. */
  const drainPresetWork = (): void => {
    while (presetsLoaded() && presetWork.length > 0 && !timersDisposed) presetWork.shift()!();
  };

  /** A timer's switch (§7.5). At a start it notes where the switch came
   *  from first (D17): a timer that already has a return keeps its `from`
   *  and takes the new `to`. An end's switch notes nothing. A preset not in
   *  the loaded list switches nothing. */
  const switchForTimer = (id: string, hook: "start" | "end", to: string): void => {
    if (timersDisposed || !presetExists(to)) return;
    if (hook === "start") saveReturns(returnsAfterStart(readReturns(timerStore().presetReturns), id, presetId(), to));
    usePreset(to, { fromTimer: true });
  };
  /** Settles `id`'s return (§7.3): with `revert`, back to where its start
   *  switched from while the live preset is still the timer's, or hands
   *  that to a return stacked on top of it; then the return goes. */
  const settleReturn = (id: string, revert: boolean): void => {
    if (timersDisposed) return;
    const returns = readReturns(timerStore().presetReturns);
    const plan = revertPlan(returns, id, presetId(), presetExists, revert);
    if (plan.returns !== returns) saveReturns(plan.returns);
    if (plan.switchTo !== null) switchForReturn(plan.switchTo);
  };

  /** One action of a timer's page half (`dispatchAction` from a timer). */
  const runTimerAction = (action: GlyphAction | TimerAction, id: string, hook: "start" | "end"): void => {
    switch (action.kind) {
      case "preset": {
        const to = action.presetId;
        whenPresetsLoaded(() => switchForTimer(id, hook, to));
        break;
      }
      case "randomPreset":
        // Any preset that changes the voice but the live one (§7.4, Q11).
        whenPresetsLoaded(() => {
          const to = pickRandomPreset(
            presets().map((p) => p.id),
            (pid) => transforms(chains[pid] ?? []),
            presetId(),
            Math.random,
          );
          if (to !== null) switchForTimer(id, hook, to);
        });
        break;
      case "revertPreset":
        // Never at a start (Rust never sends one there).
        if (hook === "end") whenPresetsLoaded(() => settleReturn(id, true));
        break;
      case "tile":
        void playTileById(action.tileId);
        break;
      default:
        // The backend's kinds (setMuted, speak, saveReplay, panic) and the
        // glyph's toggles: nothing on the page.
        break;
    }
  };

  // Events are adopted by `seq`: the backend numbers them in the order its
  // timer thread planned them, so one at or below the last seen is old.
  let lastActionsSeq = -1;
  // Timers this page has heard an event for: their events settle their
  // returns, so reconcile leaves them be. The mirror can lag an event (the
  // thread's `timer-state` follows its `timer-actions`), and a start just
  // heard would otherwise read as idle and be undone.
  const timersHeard = new Set<string>();
  /** The `timer-actions` listener (§7.5). An end's own list runs first;
   *  then any return still left for the timer is dropped (R2): a start's
   *  switch that At zero doesn't switch back from stays, and its record
   *  goes. */
  const runTimerActions = (raw: unknown): void => {
    if (timersDisposed) return;
    const ev = readTimerActionsEvent(raw);
    if (ev === null || ev.seq <= lastActionsSeq) return;
    lastActionsSeq = ev.seq;
    timersHeard.add(ev.id);
    for (const a of ev.actions) dispatchAction(a, { kind: "timer", id: ev.id, hook: ev.hook });
    if (ev.hook === "end") whenPresetsLoaded(() => settleReturn(ev.id, false));
  };

  // Once per page (§7.6): after the boot sync has answered, the listeners
  // are up, the mirror holds a snapshot and the real preset list has
  // loaded, the stored returns whose end no page heard are settled: a
  // revert whose event fell inside a reload, or a countdown that ran out
  // while Divora was closed. Then the restart notice is armed (D36).
  let reconciled = false;
  const tryReconcile = (): void => {
    if (reconciled || timersDisposed || !bootSynced || !timersListening) return;
    const snap = timerSnapshot();
    if (snap === null || !presetsLoaded()) return;
    reconciled = true;
    const returns = readReturns(timerStore().presetReturns);
    if (returns.length > 0) {
      // A timer heard since the page loaded is kept, as a running one is.
      const phaseOf = (id: string): TimerPhase | null =>
        timersHeard.has(id) ? "running" : (snap.timers.find((t) => t.id === id)?.phase ?? null);
      const plan = reconcileReturns(returns, timerStore(), phaseOf, presetId(), presetExists);
      // Kept, every one: nothing to write.
      if (plan.returns !== returns) saveReturns(plan.returns);
      if (plan.switchTo !== null) switchForReturn(plan.switchTo);
    }
    armRestartNotice();
  };

  // D36: a restart mid-break mutes you again in the backend (P7), which
  // tells nobody why. For 3 s after the reconcile pass, the first time the
  // mute mirror reads muted while a stored countdown that mutes at its
  // start is running or paused, a toast names it. The window covers a boot
  // read that lands before the re-mute (the mirror adopts by `seq`). Once
  // per page; a reload says it again.
  const [restartNoticeArmed, setRestartNoticeArmed] = createSignal(false);
  let restartNoticeTimer: ReturnType<typeof setTimeout> | undefined;
  onCleanup(() => clearTimeout(restartNoticeTimer));
  const armRestartNotice = (): void => {
    setRestartNoticeArmed(true);
    restartNoticeTimer = setTimeout(() => setRestartNoticeArmed(false), RESTART_NOTICE_MS);
  };
  createEffect(() => {
    if (!restartNoticeArmed()) return;
    const snap = timerSnapshot();
    const phaseOf = (id: string): TimerPhase | null => snap?.timers.find((t) => t.id === id)?.phase ?? null;
    const text = restartNotice(timerStore(), phaseOf, muted());
    if (text === null) return;
    setRestartNoticeArmed(false);
    clearTimeout(restartNoticeTimer);
    // An error's tone: it says why you can't be heard, and it stays long
    // enough to read.
    showToast(text, "error");
  });

  // Each timer's keys live in its own entry, by verb. This build registers
  // `toggle`, `reset` and (v1.60.0) `finish`; any other verb (a later
  // build's) is kept and written back as it is. Saved first, then
  // registered, as `setHotkeyBinding` does; a bind changes only its own
  // verb. Through the timers' queue, so binds land in the order they were
  // made, and after a delete's unregisters of an id an Add took again.
  const bindTimerHotkey: AppState["bindTimerHotkey"] = async (id, verb, raw) => {
    // A capture can outlive its card: a key for a timer no longer stored
    // (or one the backend doesn't run) would be one nothing shows, and the
    // next timer to take that id would answer to it.
    const t = runTimer(id);
    if (!t || !TIMER_VERBS.includes(verb)) return;
    const accelerator = normalizeAccelerator(raw);
    const hotkeys = { ...t.hotkeys };
    if (accelerator) hotkeys[verb] = accelerator;
    else delete hotkeys[verb];
    saveTimers(withTimer(timerStore(), id, { ...t, hotkeys }));
    const hotkeyId = timerHotkeyId(id, verb);
    if (!accelerator) {
      try {
        await timerQueue(() => unregisterGlobalShortcutCmd(hotkeyId));
      } catch (err) {
        console.warn("[hourglass] unregister hotkey failed", err);
      }
      return;
    }
    try {
      await timerQueue(() => registerGlobalShortcutCmd(hotkeyId, accelerator));
    } catch (err) {
      console.warn("[hourglass] register hotkey failed", err);
    }
  };

  // Asked once, then remembered: the command creates the folder, and the
  // file writer makes it again at its next write (within about 5 s while a
  // timer has a file; with no timer, nothing does).
  let timersDirPath: string | null = null;
  const timersDir: AppState["timersDir"] = async () => {
    if (timersDirPath !== null) return timersDirPath;
    try {
      const dir = await timersDirCmd();
      if (typeof dir === "string" && dir !== "") timersDirPath = dir;
    } catch (e) {
      console.warn("[hourglass] timers_dir failed:", e);
    }
    return timersDirPath;
  };
  const timerOnOverlay = (id: string): boolean => overlay.overlayHasTimer(id);
  const setTimerOnOverlay = (id: string, on: boolean): void => {
    if (on) overlay.addOverlayTimer(id);
    else overlay.removeOverlayTimer(id);
  };

  const hasEnabled = createMemo(() => chain().some((c) => c.enabled));
  // v1.58.0: the status says what happens to what you say from now on. Push
  // to modulate is off ("always") by default, so a preset with effects on
  // reads Modulated; a gate-only chain (Clean Passthrough) reads Clean.
  const status = createMemo<VoiceStatus>(() =>
    muted() ? "muted" : transforms(chain()) && !ptmBypassed() ? "modulated" : "clean",
  );

  // v1.54.0: when the voice last became modulated with the current preset,
  // for Replay's save toast. The AV tap carries what the call hears, so a
  // clip holds the disguised voice only while the status says "modulated":
  // push-to-modulate up in Hold to apply (or down in Hold to bypass), no
  // effect on but the noise gate, or muted is the user's own voice (or
  // nothing). A preset change restarts the clock. v1.58.0: true of
  // push-to-modulate now, not only of the pill: the engine switches the
  // effects inside the voice stage, before the AV tap, and the status reads
  // its mirror. The mirror lands a tick or a round trip after the engine
  // turns the effects on, so this clock starts after they did: it errs
  // toward not naming the preset.
  const [modulatedSince, setModulatedSince] = createSignal<number | null>(null);
  createEffect(
    on([status, presetId], ([s]) => {
      setModulatedSince(s === "modulated" ? performance.now() : null);
    }),
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
        // The restored active preset may have been a user preset that's
        // since been deleted — if it's gone from the real list, fall back
        // so the engine doesn't run an empty chain.
        if (!next.some((p) => p.id === presetId())) {
          const fallback = next.find((p) => p.id === firstPreset.id) ?? next[0];
          if (fallback) {
            setPresetId(fallback.id);
            setViewedId(fallback.id);
          }
        }
        // v1.60.0: timers' preset work that waited for the real list runs
        // now, in the order it came, then the one reconcile (D16, §7.6).
        drainPresetWork();
        tryReconcile();
      }
    } catch (err) {
      // Browser preview / Tauri unreachable — keep the fallback bundled
      // set. A restored active id we can't validate yet (e.g. a user
      // preset, offline) stands until the next successful refresh; the
      // engine isn't auto-started, so this resolves before the user can
      // run an unvalidated chain.
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

  // v1.54.0: a device switch must not be a Stop. `start_audio_engine` tears
  // the running streams down itself, so starting again is enough — and a
  // Stop is what Replay reads as the user stopping the engine, which holds
  // the ring and then clears it on every device switch. A switch that
  // starts cleanly is a rebuild Replay carries on through (silence in the
  // gap); one that fails counts as a stop.
  const restartEngine = async (): Promise<void> => {
    // The teardown finalizes a WAV recording, as a stop does.
    setIsRecording(false);
    await startEngine();
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
        // v1.54.0: a restart, never stop + start (see restartEngine).
        void restartEngine();
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

  // v1.60.0: the effect chain's commands, one at a time (D19), as the
  // mute's and the timers' are: invokes in flight together can reach Rust
  // out of send order (measured in the real app with 40 sent at once), so
  // a timer's preset switch and a slider moved at the same time could
  // otherwise land reversed, and a param sent before a switch would edit
  // the new chain's effect at that index. Every `set_effect_chain`,
  // `set_effect_param`, `set_effect_enabled` and `set_voice_model` the
  // store sends goes through it; a voice model sent after a chain lands
  // after it. A failure is logged and holds nothing back.
  const chainQueue = inOrder();
  const sendChainCmd = (what: string, send: () => Promise<unknown>): void => {
    chainQueue(send).catch((e: unknown) => {
      console.warn(`[chain] ${what} failed:`, e);
    });
  };

  const chainToSpecs = (entries: ChainEntry[]): EffectSpec[] =>
    entries.map((e) => ({
      kind: e.id,
      enabled: e.enabled,
      params: { ...e.vals },
    }));

  const syncChain = (): void => {
    if (!engineRunning()) return;
    const specs = chainToSpecs(chain());
    sendChainCmd("set_effect_chain", () => setEffectChainCmd(specs));
    applyActiveVoice();
  };

  // v1.46.0: keep the backend's reactive config in step with everything it
  // derives from.
  //
  // This is an effect rather than calls bolted onto each mutator because the
  // config depends on the LIVE CHAIN, and the chain changes through many
  // paths — preset switch, an Inspector slider, toggling an effect, a
  // reorder, an A/B swap — plus the engine starting. An earlier version hung
  // the re-send off `syncChain()`, which turned out to have no production
  // callers at all, so none of it ran: the backend kept offsetting from the
  // previous preset's authored values, and an Inspector edit was overwritten
  // on the next buffer. Tracking the inputs makes every path correct by
  // construction instead of by remembering to call something.
  createEffect(() => {
    // Tracked reads. `chain()` covers every chain mutation; `engineRunning()`
    // covers a fresh session (including one rebuilt after device loss).
    reactiveEnabled();
    reactiveIntensity();
    chain();
    engineRunning();
    syncReactive();
  });

  // Send a full SetChain when either the active preset changes or the
  // engine starts up. v1.60.0: the one chain a preset switch sends
  // (`usePreset` sent a second, which, landing after the voice model,
  // cleared it, D19).
  createEffect(
    on([presetId, engineRunning], ([, running]) => {
      if (running) {
        const specs = chainToSpecs(chain());
        sendChainCmd("set_effect_chain", () => setEffectChainCmd(specs));
        // A SetChain rebuilds every effect from scratch — including the
        // VoiceConvert, which comes back with no model. Re-point it at
        // the active voice so the selection survives preset switches.
        applyActiveVoice();
      }
    }, { defer: true }),
  );

  // Persist the active preset so it's restored on the next launch. `defer`
  // skips the redundant write of the just-loaded value on mount; every real
  // switch (Use / summon / glyph / the validity fallback) is saved.
  createEffect(
    on(presetId, (id) => saveJson(STORAGE_KEYS.activePreset, id), { defer: true }),
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
    const path = activeVoicePath();
    sendChainCmd("set_voice_model", () => setVoiceModelCmd(idx, path));
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
      sendChainCmd("set_effect_param", () => setEffectParamCmd(effectIndex, key, value));
    }
  };

  const setChainEnabled = (effectIndex: number, enabled: boolean): void => {
    setChains(viewedId(), effectIndex, "enabled", enabled);
    if (editIsLive()) {
      sendChainCmd("set_effect_enabled", () => setEffectEnabledCmd(effectIndex, enabled));
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
      const specs = chainToSpecs(next);
      sendChainCmd("set_effect_chain", () => setEffectChainCmd(specs));
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
      const specs = chainToSpecs(next);
      sendChainCmd("set_effect_chain", () => setEffectChainCmd(specs));
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

  const usePreset: AppState["usePreset"] = (id, opts) => {
    if (!presets().some((p) => p.id === id)) return;
    setPresetId(id);
    // Applying makes the viewed preset the active one. v1.60.0: except a
    // timer's switch while you are on the Presets screen, which leaves the
    // preset you are editing there in front of you (D18); leaving the
    // screen brings the view to the live preset, as always.
    if (!(opts?.fromTimer && nav() === "presets")) setViewedId(id);

    // Reset A/B for the new preset (both slots = current chain).
    const c = chains[id] ?? [];
    setAbSlots(id, { A: cloneChain(c), B: cloneChain(c) });
    setUi("ab", "A");
    // v1.60.0: no chain sent here. The effect on the active preset sends
    // the one chain, then the voice model; a second one queued after the
    // model would clear it (D19). A Use of the live preset changes no chain.
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
  // v1.60.0: the folder whose last scan has finished (or failed).
  const [soundboardScannedFolder, setSoundboardScannedFolder] = createSignal<string | null>(null);
  const soundboardScanned = (): boolean => {
    const folder = soundboardFolder();
    return !folder || soundboardScannedFolder() === folder;
  };
  const [soundboardError, setSoundboardError] = createSignal<string | null>(null);
  const [playingClips, setPlayingClips] = createStore<Record<string, PlayingClip>>({});
  // v1.60.0: the soundboard's commands, one at a time (D19): invokes in
  // flight together can reach Rust out of send order, so a Panic pressed
  // right after a clip (a timer's or a click's) could land first and leave
  // the clip playing. Every `play_soundboard_clip`, `stop_soundboard_clip`
  // and `stop_all_soundboard_clips` the store sends goes through it, Speak's
  // saved-clip play included. A timer's own Stop all clips and lines runs in
  // the backend and meets this queue only at the engine (K2).
  const soundboardQueue = inOrder();
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
      setSoundboardScannedFolder(folder);
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
    const gain = tileGain(tile.id);
    try {
      const durationSecs = await soundboardQueue(() => playSoundboardClipCmd(tile.id, tile.path, gain));
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
      await soundboardQueue(() => stopSoundboardClipCmd(clipId));
    } catch (err) {
      // The local "not playing" state is what users see; record the
      // backend failure but don't re-throw — stop should always succeed
      // from the UI's perspective.
      setSoundboardError(`stop failed: ${String(err)}`);
    }
    setPlayingClips(clipId, undefined as unknown as PlayingClip);
  };

  /** Nothing shows as playing: the whole playing-clips store emptied. */
  const clearPlayingClips = (): void => {
    for (const id of Object.keys(playingClips)) {
      setPlayingClips(id, undefined as unknown as PlayingClip);
    }
  };

  const panicSoundboard = async (): Promise<void> => {
    try {
      await soundboardQueue(() => stopAllSoundboardClipsCmd());
    } finally {
      clearPlayingClips();
    }
  };

  /**
   * Per-tile hotkey id used when talking to `tauri-plugin-global-
   * shortcut`. The plugin's id namespace is shared between PTM /
   * panic / monitor and every tile, so we prefix tiles to keep them
   * out of the action namespace. v1.59.0: Hourglass's timers use
   * `tm:<id>:<verb>` the same way (`timerHotkeyId`, data/timers.ts).
   */
  const tileHotkeyId = (clipId: string): string => `sb:${clipId}`;

  const bindTileHotkey = async (
    clipId: string,
    keys: string[],
  ): Promise<void> => {
    setTileHotkeys(clipId, keys);
    saveJson(STORAGE_KEYS.tileHotkeys, { ...tileHotkeys });
    const accelerator = keysToAccelerator(keys);
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

  // v1.51.4: loaded from `divora.hotkeyBindings` (defaults stay empty, so
  // nothing is registered system-wide until the user binds a key), and
  // normalized so a binding saved with "Win" registers as "Super".
  const [hotkeyBindings, setHotkeyBindings] = createStore<Record<HotkeyAction, string>>({
    ...DEFAULT_HOTKEY_BINDINGS,
    ...readHotkeyBindings(loadJson<unknown>(STORAGE_KEYS.hotkeyBindings, null)),
  });
  // The in-app push-to-modulate listener follows the bound key, as it does
  // when the key is set (below).
  if (hotkeyBindings.ptm) setUi("ptmKey", hotkeyBindings.ptm);

  const setHotkeyBinding = async (
    action: HotkeyAction,
    rawAccelerator: string,
  ): Promise<void> => {
    const accelerator = normalizeAccelerator(rawAccelerator);
    setHotkeyBindings(action, accelerator);
    saveJson(STORAGE_KEYS.hotkeyBindings, { ...hotkeyBindings });
    // PTM also drives the in-app fallback listener via ui.ptmKey. v1.58.0:
    // the page lets go of the in-app key first, so a key held under the old
    // binding can't outlive it. (Its keyup would let go too: the shell also
    // matches the physical key that made the press.)
    if (action === "ptm") {
      ptmUp("key");
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
      const accelerator = keysToAccelerator(keys);
      try {
        await registerGlobalShortcutCmd(tileHotkeyId(clipId), accelerator);
      } catch (err) {
        console.warn(`[soundboard] sync tile hotkey failed for ${clipId}`, err);
      }
    }
    // v1.59.0: then Hourglass's timers, from `divora.timers`. Only timers the
    // backend runs, and only the verbs this build acts on (`TIMER_VERBS`;
    // v1.60.0 adds `finish`, End now's key, registered only when bound): a
    // newer kind's keys, a timer past the 16th and a later build's verb are
    // kept in storage and never registered. Each key is read just before it
    // is queued, so a delete or a bind made before is the one that counts.
    // The timers' queue sends it after the boot sync, and a delete or a bind
    // made after it is queued after it.
    for (const { id } of timerStore().timers) {
      for (const verb of TIMER_VERBS) {
        const accelerator = runTimer(id)?.hotkeys?.[verb];
        if (!accelerator) continue;
        try {
          await timerQueue(() => registerGlobalShortcutCmd(timerHotkeyId(id, verb), accelerator));
        } catch (err) {
          console.warn(`[hourglass] sync hotkey failed for ${id} ${verb}`, err);
        }
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

  // v1.56.0: the side (up or down) each `mute` CC mapping was last on, by
  // mapping id, so a CC acts only when it crosses. Not persisted: the first
  // message after load acts only when it is up. A mapping that changes or
  // goes forgets its side, and every mapping forgets it when the input is
  // picked: the same CC number on another device (or on this one, moved
  // while closed) is a control we haven't heard yet.
  const muteCcSides = new Map<string, boolean>();

  // v1.58.0: push-to-modulate's MIDI hold, forgotten where a note-off or a
  // CC's fall could no longer reach it. Every mapping on an input pick (the
  // old port's pads go quiet, and the same numbers on the new one are
  // controls we haven't heard) and on entering learn (learn swallows every
  // message, the note-off of a pad held on another mapping included); one
  // mapping when it changes or goes. Lets go in the engine if that empties
  // the set.
  const forgetPtmMidi = (id?: string): void =>
    changePtmMidi(() => {
      if (id === undefined) {
        ptmMidiDown.clear();
        ptmCcSides.clear();
      } else {
        ptmMidiDown.delete(id);
        ptmCcSides.delete(id);
      }
    });

  const selectMidiInput = async (name: string | null): Promise<void> => {
    muteCcSides.clear();
    forgetPtmMidi();
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
    muteCcSides.delete(id);
    forgetPtmMidi(id);
    persistMidiMappings(
      midiMappings().map((m) => (m.id === id ? { ...m, ...patch } : m)),
    );
  };

  const removeMidiMapping = (id: string): void => {
    if (midiLearnId() === id) setMidiLearnIdRaw(null);
    muteCcSides.delete(id);
    forgetPtmMidi(id);
    persistMidiMappings(midiMappings().filter((m) => m.id !== id));
  };

  const setMidiLearnId = (id: string | null): void => {
    if (id !== null) forgetPtmMidi();
    setMidiLearnIdRaw(id);
  };

  // Bind the pure router's handler interface to the live store actions.
  const midiHandlers: MidiHandlers = {
    usePreset: (id) => usePreset(id),
    playTile: (id) => void playTileById(id),
    // v1.58.0: each `ptm` mapping holds while it is down; the `midi` source
    // is held while any is, so pad B can't be let go by pad A's note-off. A
    // CC holds once it rises to 64 or more and lets go at 59 or less; 60-63
    // keep the side it was on, so a knob near halfway can't flap.
    ptmNote: (id, down) =>
      changePtmMidi(() => {
        if (down) ptmMidiDown.add(id);
        else ptmMidiDown.delete(id);
      }),
    ptmCc: (id, value) =>
      changePtmMidi(() => {
        const step = muteCcStep(ptmCcSides.get(id), value);
        if (step.up !== undefined) ptmCcSides.set(id, step.up);
        if (step.act === true) ptmMidiDown.add(id);
        else if (step.act === false) ptmMidiDown.delete(id);
      }),
    toggleMonitor: () => void toggleMonitor(),
    setParam: (idx, key, value) => setChainParam(idx, key, value),
    // v1.56.0: a pad has no label, so it toggles in the backend. A CC sets
    // the side it crossed to; no dedupe against the mirror (it can lag a
    // tick, and the backend's set is idempotent).
    toggleMuted: () => void toggleMuted(),
    muteCc: (id, value) => {
      const step = muteCcStep(muteCcSides.get(id), value);
      if (step.up !== undefined) muteCcSides.set(id, step.up);
      if (step.act !== null) void setMuted(step.act);
    },
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

  // What's new (v1.49.0). Content is compiled in from CHANGELOG.md, so
  // this never touches the network and can't disagree with the running
  // build. See src/data/changelog.ts for the trigger rules.
  const [whatsNewContent, setWhatsNewContent] =
    createSignal<WhatsNewContent | null>(null);
  const [whatsNewBanner, setWhatsNewBanner] = createSignal<ReleaseNote | null>(
    null,
  );
  const [whatsNewOpen, setWhatsNewOpen] = createSignal(false);

  const openWhatsNew = (): void => {
    // Opened deliberately from Settings: show recent releases even when
    // there's nothing new for this user, so the entry is never a dead end.
    if (whatsNewContent() === null) {
      setWhatsNewContent({
        notes: RELEASE_NOTES.slice(0, RENDER_CAP),
        moreCount: 0,
        moreUnknown: RELEASE_NOTES.length > RENDER_CAP,
      });
    }
    setWhatsNewBanner(null);
    setWhatsNewOpen(true);
  };
  const closeWhatsNew = (): void => {
    setWhatsNewOpen(false);
  };
  const dismissWhatsNewBanner = (): void => {
    setWhatsNewBanner(null);
  };

  let whatsNewChecked = false;
  const checkWhatsNew = async (): Promise<void> => {
    // Once per process. A second pass could only re-announce something
    // already dismissed.
    if (whatsNewChecked) return;
    whatsNewChecked = true;
    let version: string | null = null;
    try {
      const { getVersion } = await import("@tauri-apps/api/app");
      version = await getVersion().catch(() => null);
    } catch {
      return; // not under Tauri (browser preview / E2E)
    }
    const decision = decideWhatsNew({
      version,
      lastSeen: loadRaw(STORAGE_KEYS.whatsNewSeenVersion),
      wizardSeen: loadRaw(WIZARD_SEEN_KEY) !== null,
    });
    if (decision.action === "none" || !version) return;

    // Record BEFORE announcing, not on dismiss. If the user closes the app
    // with the banner still up — or it never gets dismissed at all — the
    // next launch must stay quiet. Firefox's "just updated on every start"
    // bug is this write going missing; fail closed.
    saveRaw(STORAGE_KEYS.whatsNewSeenVersion, version);
    if (decision.action === "seed") return;

    const content = notesSince(decision.since, version);
    if (content.notes.length === 0) return;
    setWhatsNewContent(content);
    setWhatsNewBanner(content.notes[0] ?? null);
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
        muted: muted(),
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
  // Best-of-N take progress for VoxCPM clones (v1.27.0), driven by the backend's
  // "tts-progress" events; null when idle or for single-pass voices.
  const [ttsProgress, setTtsProgress] = createSignal<{
    done: number;
    total: number;
  } | null>(null);
  const [ttsError, setTtsError] = createSignal<string | null>(null);
  // v1.28.0: saved Speak clips (TTS outputs persisted to the speak-clips folder).
  const [speakClips, setSpeakClips] = createSignal<SpeakClip[]>([]);
  const [ttsVolume, setTtsVolumeRaw] = createSignal<number>(
    loadJson<number>(STORAGE_KEYS.ttsVolume, 1.0),
  );
  const [ttsPreviewOnly, setTtsPreviewOnlyRaw] = createSignal<boolean>(
    loadJson<boolean>(STORAGE_KEYS.ttsPreviewOnly, false),
  );
  // Best-of-N count for VoxCPM cloned voices (Fast 1 / Balanced 3 / Best 6).
  // Mirrors the backend's DEFAULT_CANDIDATES; higher = more faithful, slower.
  const [cloneQuality, setCloneQualityRaw] = createSignal<number>(
    loadJson<number>(STORAGE_KEYS.cloneQuality, 3),
  );
  // v1.29.0: hear Speak previews in the monitor, independent of the Mixer's mic
  // monitor. Default on; persisted + pushed to the engine (which also defaults
  // it on) so it survives restarts.
  const [speakMonitor, setSpeakMonitorRaw] = createSignal<boolean>(
    loadJson<boolean>(STORAGE_KEYS.speakMonitor, true),
  );
  // Experimental DirectML GPU for VoxCPM cloning. Default off (opt-in: needs the
  // ~1.2 GB fp16 GPU model). Persisted; passed to `speak` per-utterance.
  const [cloneGpu, setCloneGpuRaw] = createSignal<boolean>(
    loadJson<boolean>(STORAGE_KEYS.cloneGpu, false),
  );
  // Sync the persisted value to the engine at startup (covers a persisted
  // "off"); the command just stores an atomic, so it's safe before streams run.
  void setSpeakMonitorCmd(speakMonitor()).catch(() => {});

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

  const setCloneQuality = (n: number): void => {
    const clamped = Math.max(1, Math.min(8, Math.round(n)));
    setCloneQualityRaw(clamped);
    saveJson(STORAGE_KEYS.cloneQuality, clamped);
  };

  const setSpeakMonitor = (on: boolean): void => {
    setSpeakMonitorRaw(on);
    saveJson(STORAGE_KEYS.speakMonitor, on);
    void setSpeakMonitorCmd(on).catch(() => {});
  };
  const setCloneGpu = (on: boolean): void => {
    setCloneGpuRaw(on);
    saveJson(STORAGE_KEYS.cloneGpu, on);
  };

  // v1.60.0: whether a list of voices has answered (or failed), for a
  // timer's line: "Loading voices…" until then, never "not available".
  const [ttsVoicesLoaded, setTtsVoicesLoaded] = createSignal(false);
  let ttsVoicesAsked: Promise<void> | null = null;

  const refreshTtsVoices = async (): Promise<void> => {
    try {
      const list = await listTtsVoicesCmd();
      const next = Array.isArray(list) ? list : [];
      setTtsVoices(next);
      setTtsVoicesLoaded(true);
      // Default to the first listed voice (a Kokoro preset — the backend lists
      // those first) when none is chosen, or the persisted one exists as
      // neither a built-in nor a cloned voice.
      const current = selectedTtsVoice();
      const first = next[0];
      const known =
        !!current &&
        (next.some((v) => v.id === current) ||
          clonedVoices().some((v) => v.id === current));
      if (!known && first) {
        setSelectedTtsVoice(first.id);
      }
    } catch (err) {
      console.warn("[tts] voice refresh failed", err);
    }
  };

  // v1.60.0: a timer's line needs the voices (Settings → Hourglass, which
  // may be opened before Speak ever was). Asked once while the list is
  // empty, and only the list is set: Speak's selection is Speak's own, and
  // `refreshTtsVoices` would move it off a cloned voice it doesn't list.
  // A failure lets the next call ask again.
  const ensureTtsVoices = (): Promise<void> => {
    if (ttsVoices().length > 0) {
      setTtsVoicesLoaded(true);
      return Promise.resolve();
    }
    ttsVoicesAsked ??= (async () => {
      try {
        const list = await listTtsVoicesCmd();
        if (ttsVoices().length === 0 && Array.isArray(list)) setTtsVoices(list);
      } catch (err) {
        console.warn("[tts] voice list for a timer failed", err);
        ttsVoicesAsked = null;
      } finally {
        setTtsVoicesLoaded(true);
      }
    })();
    return ttsVoicesAsked;
  };

  const refreshSpeakClips = async (): Promise<void> => {
    try {
      const clips = await listSpeakClipsCmd();
      setSpeakClips(Array.isArray(clips) ? clips : []);
    } catch (err) {
      console.warn("[speak] list clips failed", err);
    }
  };

  const playSpeakClip = async (clip: SpeakClip): Promise<void> => {
    const gain = ttsVolume();
    try {
      await soundboardQueue(() => playSoundboardClipCmd(`speak-clip-${clip.id}`, clip.path, gain));
    } catch (err) {
      console.warn("[speak] play clip failed", err);
    }
  };

  const removeSpeakClip = async (id: string): Promise<void> => {
    try {
      await deleteSpeakClipCmd(id);
      setSpeakClips((cs) => cs.filter((c) => c.id !== id));
    } catch (err) {
      console.warn("[speak] delete clip failed", err);
    }
  };

  const speakText = async (): Promise<void> => {
    const text = ttsText().trim();
    const voiceId = selectedTtsVoice();
    if (!text || !voiceId || synthesizing()) return;
    setTtsError(null);
    setTtsProgress(null);
    setSynthesizing(true);
    try {
      const durationSecs = await speakCmd(
        text,
        voiceId,
        ttsVolume(),
        ttsPreviewOnly(),
        cloneQuality(),
        cloneGpu(),
      );
      // Reuse the soundboard playback seam: register a "tts" clip so the
      // shared ~30 Hz clock drives the progress ring and auto-expires it.
      setPlayingClips("tts", {
        clipId: "tts",
        startedAt: performance.now(),
        durationSecs,
      });
      // The backend just saved this clip; refresh the "Saved clips" list.
      void refreshSpeakClips();
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
      setTtsProgress(null);
    }
  };

  // ---- v1.47.0: tap-to-audition previews ----
  const [previewingVoice, setPreviewingVoice] = createSignal<string | null>(null);
  const [previewRendering, setPreviewRendering] = createSignal(false);
  // Timer that returns a card from Stop to Play when the clip ends.
  let previewTimer: number | null = null;
  // Monotonic id for in-flight auditions. A render can take seconds on a
  // cloned voice, so a result that arrives after the user has moved on must be
  // dropped rather than played over the voice they actually asked for.
  let previewGeneration = 0;

  const previewVoice = async (voiceId: string): Promise<void> => {
    if (!voiceId) return;
    // Tapping the voice that is already auditioning stops it.
    if (previewingVoice() === voiceId) {
      stopPreview();
      return;
    }
    previewGeneration += 1;
    const generation = previewGeneration;
    if (previewTimer !== null) {
      window.clearTimeout(previewTimer);
      previewTimer = null;
    }
    // Stop whatever is playing before starting the next one — the mixer takes
    // an idle slot rather than replacing a same-id clip, so without this two
    // voices would play at once.
    void stopPreviewVoiceCmd();
    setPreviewingVoice(voiceId);
    setPreviewRendering(true);
    setTtsError(null);
    try {
      // The GPU flag rides along: the backend shares ONE VoxCPM engine with
      // Speak, and asking for a different mode rebuilds ~1.6 GB of ONNX.
      const durationSecs = await previewVoiceCmd(voiceId, cloneGpu());
      if (generation !== previewGeneration) return; // superseded
      setPreviewRendering(false);
      // Clear when the clip finishes so the card returns to its play state.
      previewTimer = window.setTimeout(() => {
        previewTimer = null;
        if (generation === previewGeneration) setPreviewingVoice(null);
      }, Math.max(250, durationSecs * 1000));
    } catch (err) {
      if (generation !== previewGeneration) return;
      setPreviewingVoice(null);
      setPreviewRendering(false);
      setTtsError(
        typeof err === "string"
          ? err
          : err instanceof Error
            ? err.message
            : String(err),
      );
    }
  };

  const stopPreview = (): void => {
    // Bumping the generation orphans any in-flight render, so a slow result
    // cannot arrive later and play unprompted.
    previewGeneration += 1;
    if (previewTimer !== null) {
      window.clearTimeout(previewTimer);
      previewTimer = null;
    }
    setPreviewingVoice(null);
    setPreviewRendering(false);
    // The backend bumps its OWN epoch here — that is what stops an in-flight
    // render from playing once it finishes, since it cannot be aborted.
    void stopPreviewVoiceCmd();
  };

  const stopSpeaking = async (): Promise<void> => {
    setPlayingClips("tts", undefined as unknown as PlayingClip);
    try {
      await stopSpeakCmd();
    } catch (err) {
      console.warn("[tts] stop failed", err);
    }
  };

  // ---- v1.20.0: voice cloning ("Your voices") ----

  const [clonedVoices, setClonedVoices] = createSignal<ClonedVoiceInfo[]>([]);
  const [cloning, setCloning] = createSignal(false);
  const [cloneError, setCloneError] = createSignal<string | null>(null);

  const refreshClonedVoices = async (): Promise<void> => {
    try {
      const list = await listClonedVoicesCmd();
      setClonedVoices(Array.isArray(list) ? list : []);
    } catch (err) {
      console.warn("[clone] refresh failed", err);
    }
  };

  const addClonedVoice = async (name: string): Promise<void> => {
    const trimmed = name.trim();
    if (!trimmed || cloning()) return;
    const path = await pickAudioFileCmd();
    if (!path) return; // user cancelled the file picker
    setCloneError(null);
    setCloning(true);
    try {
      const voice = await cloneVoiceCmd(trimmed, path);
      await refreshClonedVoices();
      setSelectedTtsVoice(voice.id); // auto-select the new voice
    } catch (err) {
      setCloneError(
        typeof err === "string"
          ? err
          : err instanceof Error
            ? err.message
            : String(err),
      );
    } finally {
      setCloning(false);
    }
  };

  // v1.43.0: rename a cloned voice. Only the display name changes — the id is
  // stable, so the current selection and saved clips keep working. Resolves to
  // true on success; on failure the message lands in `cloneError`.
  const renameClonedVoice = async (
    id: string,
    name: string,
  ): Promise<boolean> => {
    // Cleared up front: a stale failure from an earlier attempt must not sit
    // on screen next to a rename that then succeeds (including the no-op).
    setCloneError(null);
    const trimmed = name.trim();
    if (!trimmed) return false;
    // No-op if the name didn't actually change.
    const current = clonedVoices().find((v) => v.id === id);
    if (current && current.name === trimmed) return true;
    try {
      await renameClonedVoiceCmd(id, trimmed);
    } catch (err) {
      setCloneError(
        typeof err === "string"
          ? err
          : err instanceof Error
            ? err.message
            : String(err),
      );
      return false;
    }
    await refreshClonedVoices();
    return true;
  };

  const removeClonedVoice = async (id: string): Promise<void> => {
    try {
      await deleteClonedVoiceCmd(id);
    } catch (err) {
      console.warn("[clone] delete failed", err);
    }
    await refreshClonedVoices();
    if (selectedTtsVoice() === id) {
      const first = ttsVoices()[0];
      if (first) setSelectedTtsVoice(first.id);
    }
  };

  // v1.23.0: in-app dry-mic recording of a clone reference.
  const [recordingVoice, setRecordingVoice] = createSignal(false);
  // v1.57.0: the backend picks the engine when the take starts and answers
  // the sentence to read (or null). A model download no longer holds the
  // main thread, so VoxCPM's last file can land mid-take: the hint is the
  // start's answer, kept for the take, not VoxCPM's status now, which could
  // turn it into "read this aloud" partway through a take saved as a
  // free-speech clone (or, a round trip after the download ends, say
  // "speak naturally" for a take the backend makes a VoxCPM voice).
  const [recordingReadPrompt, setRecordingReadPrompt] = createSignal("");

  const startRecordingVoice = async (): Promise<void> => {
    if (recordingVoice() || cloning()) return;
    setCloneError(null);
    try {
      setRecordingReadPrompt((await startVoiceRecordingCmd()) ?? "");
      setRecordingVoice(true);
    } catch (err) {
      setCloneError(
        typeof err === "string"
          ? err
          : err instanceof Error
            ? err.message
            : String(err),
      );
    }
  };

  const stopRecordingVoice = async (name: string): Promise<void> => {
    if (!recordingVoice()) return;
    setRecordingVoice(false);
    setRecordingReadPrompt("");
    setCloneError(null);
    setCloning(true);
    try {
      const voice = await stopVoiceRecordingCmd(name.trim());
      await refreshClonedVoices();
      setSelectedTtsVoice(voice.id); // auto-select the new voice
    } catch (err) {
      setCloneError(
        typeof err === "string"
          ? err
          : err instanceof Error
            ? err.message
            : String(err),
      );
    } finally {
      setCloning(false);
    }
  };

  // v1.21.0: on-demand cloning-model download.
  const [cloneModelsReady, setCloneModelsReady] = createSignal(false);
  // v1.57.0: the running model download, any of the three, or null. The
  // backend runs it on a worker and keeps its progress, so this is set by
  // this page's start, by the progress and done events (whichever page
  // started it) and by the read at creation. One download at a time, so the
  // three buttons share it, as they shared the flag before.
  const [modelDownload, setModelDownload] = createSignal<ModelDownload | null>(null);
  const cloneDownloading = createMemo(() => modelDownload() !== null);
  const cloneDownloadPct = createMemo(() => modelDownload()?.pct ?? 0);

  const refreshCloneModelsStatus = async (): Promise<void> => {
    try {
      const status = await cloneModelsStatusCmd();
      setCloneModelsReady(!!status?.ready);
    } catch (err) {
      console.warn("[clone] models status failed", err);
    }
  };

  // VoxCPM accent-preserving cloning (v2): whether available + the read prompt.
  const [voxcpmAvailable, setVoxcpmAvailable] = createSignal(false);
  const [voxcpmReadPrompt, setVoxcpmReadPrompt] = createSignal("");

  const refreshVoxcpmStatus = async (): Promise<void> => {
    try {
      const status = await voxcpmStatusCmd();
      setVoxcpmAvailable(!!status?.available);
      setVoxcpmReadPrompt(status?.readPrompt ?? "");
    } catch (err) {
      console.warn("[voxcpm] status failed", err);
    }
  };

  // v1.30.0: experimental DirectML GPU cloning — whether the platform supports it
  // and whether the opt-in fp16 GPU decode model is downloaded.
  const [voxcpmGpuSupported, setVoxcpmGpuSupported] = createSignal(false);
  const [voxcpmGpuModelPresent, setVoxcpmGpuModelPresent] = createSignal(false);

  const refreshVoxcpmGpuStatus = async (): Promise<void> => {
    try {
      const status = await voxcpmGpuStatusCmd();
      setVoxcpmGpuSupported(!!status?.supported);
      setVoxcpmGpuModelPresent(!!status?.modelPresent);
    } catch (err) {
      console.warn("[voxcpm] gpu status failed", err);
    }
  };

  /** The status a download of `kind` changes. */
  const refreshDownloaded = async (kind: ModelDownloadKind): Promise<void> => {
    if (kind === "clone") await refreshCloneModelsStatus();
    else if (kind === "voxcpm") await refreshVoxcpmStatus();
    else await refreshVoxcpmGpuStatus();
  };

  /** A download has ended: read its status, then clear `held` — in that
   *  order, so the button goes from "Downloading… 100%" straight to what
   *  the models unlock, never showing its idle label for a round trip (as
   *  before v1.57.0). Only `held` is cleared: a download that started since,
   *  this page's or another's, keeps what it shows. */
  const endModelDownload = async (
    held: ModelDownload | null,
    kind: ModelDownloadKind,
  ): Promise<void> => {
    await refreshDownloaded(kind);
    if (modelDownload() === held) setModelDownload(null);
  };

  // One subscription for the life of the provider, so a download started by
  // a page since reloaded still shows here and ends here. A command's answer
  // can land after the events that followed it (v1.55.0), so the read at
  // creation counts only if no event came while it was on its way. A read of
  // "none running" is never applied: with no event heard, the only thing it
  // could clear is a start from this page that the read didn't see.
  let downloadHeard = 0;
  const onDownloadProgress = (p: CloneDownloadProgress): void => {
    downloadHeard += 1;
    setModelDownload(downloadAfter(modelDownload(), p));
  };
  const onDownloadDone = (d: ModelDownloadDone): void => {
    downloadHeard += 1;
    if (d.error) setCloneError(d.error);
    void endModelDownload(modelDownload(), d.kind);
  };
  const downloadUnlisten: Array<() => void> = [];
  let downloadDisposed = false;
  void (async () => {
    // v1.57.0: one after another — the done listener, then progress, then
    // the read. An event emitted before its listener is registered reaches
    // no one, and a listen answers only once Tauri has registered it. A
    // progress event or the read is all that can show a download here, so
    // with the done listener in first, the done event that ends it always
    // arrives. (Sent together, the read could see a download whose done
    // event went out before the listener was in, and show it until a
    // reload.) Each failure (a browser preview without the Tauri bridge) is
    // caught where it starts.
    const keep = (un: (() => void) | null): void => {
      if (!un) return;
      if (downloadDisposed) un();
      else downloadUnlisten.push(un);
    };
    keep(await subscribeCloneDownloadDone(onDownloadDone).catch(() => null));
    keep(await subscribeCloneDownload(onDownloadProgress).catch(() => null));
    const before = downloadHeard;
    const now = readDownloadProgress(await modelDownloadStatusCmd().catch(() => null));
    if (now && downloadHeard === before) setModelDownload(downloadAfter(null, now));
  })();
  onCleanup(() => {
    downloadDisposed = true;
    for (const un of downloadUnlisten.splice(0)) un();
  });

  /** Start one of the three downloads. The page's guard keeps a running
   *  download on screen: the backend refuses another (in words shown as-is),
   *  or answers at once for a set already on disk, and either answer would
   *  clear what this start put up in its place. The command answers after
   *  the done event, so its answer clears only this start's own state, which
   *  the first progress event replaces. */
  const startModelDownload = async (
    kind: ModelDownloadKind,
    run: () => Promise<void>,
  ): Promise<void> => {
    if (cloneDownloading()) return;
    setCloneError(null);
    const mine: ModelDownload = { kind, pct: 0 };
    setModelDownload(mine);
    try {
      await run();
    } catch (err) {
      setCloneError(
        typeof err === "string"
          ? err
          : err instanceof Error
            ? err.message
            : String(err),
      );
    } finally {
      await endModelDownload(mine, kind);
    }
  };

  const downloadCloneModels = (): Promise<void> =>
    startModelDownload("clone", downloadCloneModelsCmd);

  // v2: download the VoxCPM accent-cloning models (~1.6 GB). Refreshes VoxCPM
  // availability when done.
  const downloadVoxcpmModels = (): Promise<void> =>
    startModelDownload("voxcpm", downloadVoxcpmModelsCmd);

  // v1.30.0: download the optional fp16 DirectML decode model (~1.2 GB).
  // Refreshes GPU status when done.
  const downloadVoxcpmGpuModel = (): Promise<void> =>
    startModelDownload("voxcpmGpu", downloadVoxcpmGpuModelCmd);

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
    dispatchAction,
    recognizeCustomGlyph,

    overlayOpen: overlay.overlayOpen,
    overlayStatus: overlay.overlayStatus,
    overlayMode: overlay.overlayMode,
    overlayBg: overlay.overlayBg,
    setOverlayBg: overlay.setOverlayBg,
    overlayFit: overlay.overlayFit,
    setOverlayFit: overlay.setOverlayFit,
    overlayHide: overlay.overlayHide,
    setOverlayHide: overlay.setOverlayHide,
    overlayScene: overlay.overlayScene,
    overlayScenePush: overlay.overlayScenePush,
    openOverlay: overlay.openOverlay,
    closeOverlay: overlay.closeOverlay,
    setOverlayMode: overlay.setOverlayMode,
    noteOverlayHotkey: overlay.noteOverlayHotkey,
    addOverlayPicture: overlay.addOverlayPicture,
    removeOverlayElement: overlay.removeOverlayElement,
    addSpellCircle: overlay.addSpellCircle,

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
    deviceLost,
    setDeviceLost,
    modEnv,
    setModEnv,
    reactiveEnabled,
    setReactiveEnabled,
    reactiveIntensity,
    setReactiveIntensity,
    reactiveHasTarget,
    voiceReadingOn,
    setVoiceReadingOn,
    voiceReading,
    setVoiceReading,
    previewingVoice,
    previewRendering,
    previewVoice,
    stopPreview,
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

    captureStatus,
    captureMonitors,
    refreshCapture,
    desktopSources,
    screenRecording,
    screenSettings,
    setScreenSettings,
    toggleScreenRecording,
    screenRecordingPath,

    replayStatus,
    replaySettings,
    replayMemory,
    refreshReplayMemory,
    armReplay,
    disarmReplay,
    toggleReplay,
    saveReplay,
    setReplaySeconds,
    markReplayExplained,
    modulatedSince,
    toast,
    showToast,
    restartEngine,

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
    ensureTtsVoices,
    ttsVoicesLoaded,
    setSelectedTtsVoice,
    ttsVolume,
    setTtsVolume,
    ttsPreviewOnly,
    setTtsPreviewOnly,
    cloneQuality,
    setCloneQuality,
    speakMonitor,
    setSpeakMonitor,
    cloneGpu,
    setCloneGpu,
    ttsText,
    setTtsText,
    synthesizing,
    ttsProgress,
    setTtsProgress,
    ttsError,
    refreshTtsVoices,
    speakText,
    stopSpeaking,
    speakClips,
    refreshSpeakClips,
    playSpeakClip,
    removeSpeakClip,

    clonedVoices,
    cloning,
    cloneError,
    refreshClonedVoices,
    addClonedVoice,
    removeClonedVoice,
    renameClonedVoice,
    recordingVoice,
    recordingReadPrompt,
    startRecordingVoice,
    stopRecordingVoice,
    cloneModelsReady,
    cloneDownloading,
    cloneDownloadPct,
    modelDownload,
    refreshCloneModelsStatus,
    downloadCloneModels,
    voxcpmAvailable,
    voxcpmReadPrompt,
    refreshVoxcpmStatus,
    downloadVoxcpmModels,
    voxcpmGpuSupported,
    voxcpmGpuModelPresent,
    refreshVoxcpmGpuStatus,
    downloadVoxcpmGpuModel,

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
    soundboardScanned,
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
    whatsNewContent,
    whatsNewBanner,
    whatsNewOpen,
    openWhatsNew,
    closeWhatsNew,
    dismissWhatsNewBanner,
    checkWhatsNew,

    diagnostics,
    diagnosing,
    runDiagnostics,

    selectedEffect,
    setSelectedEffect,

    muted,
    muteState,
    muteKnown,
    applyMuteState,
    setMuted,
    toggleMuted,

    ptmState,
    ptmKnown,
    ptmBypassed,
    ptmSetting,
    setPtmOn,
    setPtmHold,
    ptmDown,
    ptmUp,
    applyPtmState,

    timerStore,
    timerRoles,
    timerSnapshot,
    adoptTimerSnapshot,
    addTimer,
    renameTimer,
    setTimerLength,
    deleteTimer,
    startTimer,
    pauseTimer,
    resetTimer,
    adjustTimer,
    finishTimer,
    addTimerAction,
    setTimerAction,
    removeTimerAction,
    timerReturn,
    bindTimerHotkey,
    timersDir,
    timerOnOverlay,
    setTimerOnOverlay,

    hasEnabled,
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
  PtmHold,
  PtmMode,
  TweaksState,
  UiState,
  VoiceStatus,
};
