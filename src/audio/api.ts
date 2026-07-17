// Tauri command + event wrappers for the audio engine.
//
// Mirrors the surface defined in `src-tauri/src/lib.rs`. All commands
// are typed; the level subscription returns an `Unlisten` you call when
// the listener should go away.

import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

/** A single audio device, as enumerated by cpal on the backend. */
export interface DeviceInfo {
  name: string;
  isDefault: boolean;
  defaultSampleRate: number;
  channels: number;
}

/** RMS + peak in [0..1] (assuming f32 input clamped to [-1, 1]). */
export interface Levels {
  rms: number;
  peak: number;
}

/** Info returned by `startAudioEngine` describing the live session. */
export interface StreamInfo {
  inputName: string;
  outputName: string;
  /** Phase 13: separate monitor ("hear yourself") device, if active. */
  monitorName: string | null;
  sampleRate: number;
  inputChannels: number;
  outputChannels: number;
}

/** One-shot engine status (running flag + last known levels). */
export interface EngineStatus {
  running: boolean;
  monitoring: boolean;
  input: Levels;
  output: Levels;
  /** Phase 14: latency added by the active DSP chain, in ms. */
  dspLatencyMs: number;
  /** Phase 16: true while the modulated output is being recorded. */
  recording: boolean;
  /** v1.7.0: makeup gain the loudness normalizer is applying, in dB
   *  (0 while disabled). */
  loudnessGainDb: number;
}

/** Periodic update emitted by the backend at ~30 Hz. */
export interface LevelUpdate {
  input: Levels;
  output: Levels;
  running: boolean;
  monitoring: boolean;
  /** Phase 14: latency added by the active DSP chain, in ms. */
  dspLatencyMs: number;
  /** Phase 16: true while the modulated output is being recorded. */
  recording: boolean;
  /** v1.7.0: makeup gain the loudness normalizer is applying, in dB. */
  loudnessGainDb: number;
  /** v1.39.0: true when device-loss auto-recovery gave up (engine stopped, no
   *  audio) — drives the "device lost — restart" banner. */
  deviceLost: boolean;
}

export async function listInputDevices(): Promise<DeviceInfo[]> {
  return invoke<DeviceInfo[]>("list_audio_input_devices");
}

export async function listOutputDevices(): Promise<DeviceInfo[]> {
  return invoke<DeviceInfo[]>("list_audio_output_devices");
}

export async function startAudioEngine(
  inputName: string | null = null,
  outputName: string | null = null,
  monitorName: string | null = null,
): Promise<StreamInfo> {
  return invoke<StreamInfo>("start_audio_engine", {
    inputName,
    outputName,
    monitorName,
  });
}

export async function stopAudioEngine(): Promise<void> {
  await invoke("stop_audio_engine");
}

export async function setAudioMonitor(enabled: boolean): Promise<void> {
  await invoke("set_audio_monitor", { enabled });
}

/** v1.29.0: toggle hearing Speak/soundboard previews in the monitor, separate
 *  from the Mixer's mic monitor ({@link setAudioMonitor}). */
export async function setSpeakMonitor(enabled: boolean): Promise<void> {
  await invoke("set_speak_monitor", { enabled });
}

/** v1.6.0: set the monitor ("hear yourself") stream gain (linear, 1.0 = unity). */
export async function setMonitorGain(gain: number): Promise<void> {
  await invoke("set_monitor_gain", { gain });
}

/** v1.7.0: enable/disable output loudness normalization (auto-gain + limiter). */
export async function setLoudnessEnabled(enabled: boolean): Promise<void> {
  await invoke("set_loudness_enabled", { enabled });
}

/** v1.7.0: set the loudness target level in dBFS (engine clamps to its window). */
export async function setLoudnessTarget(dbfs: number): Promise<void> {
  await invoke("set_loudness_target", { dbfs });
}

export async function getEngineStatus(): Promise<EngineStatus> {
  return invoke<EngineStatus>("audio_engine_status");
}

/**
 * Subscribe to `audio-levels` events from the backend. Returns the
 * unlisten function — call it on cleanup.
 */
export async function subscribeLevels(
  handler: (update: LevelUpdate) => void,
): Promise<UnlistenFn> {
  return listen<LevelUpdate>("audio-levels", (event) => handler(event.payload));
}

/** Empty level snapshot. Useful as a default when the engine isn't running. */
export const ZERO_LEVELS: Levels = { rms: 0, peak: 0 };

// ---- DSP ----

/** Mirrors `EffectKind` in `divora-core::dsp`. Snake-case from v0.12.0
 *  on so multi-word variants (`voice_convert`) round-trip cleanly. */
export type EffectKindWire =
  | "gate"
  | "denoiser"
  | "pitch"
  | "formant"
  | "eq"
  | "robot"
  | "distortion"
  | "echo"
  | "reverb"
  | "chorus"
  | "harmonizer"
  | "compressor"
  | "deesser"
  | "radio_bandpass"
  | "vintage_noise"
  | "tremolo"
  | "breath"
  | "warble"
  | "voice_convert";

/** Wire format for one effect in the chain. */
export interface EffectSpec {
  kind: EffectKindWire;
  enabled: boolean;
  params: Record<string, number>;
}

export async function setEffectChain(specs: EffectSpec[]): Promise<void> {
  await invoke("set_effect_chain", { specs });
}

export async function setEffectParam(
  index: number,
  key: string,
  value: number,
): Promise<void> {
  await invoke("set_effect_param", { index, key, value });
}

export async function setEffectEnabled(
  index: number,
  enabled: boolean,
): Promise<void> {
  await invoke("set_effect_enabled", { index, enabled });
}

export async function clearEffectChain(): Promise<void> {
  await invoke("clear_effect_chain");
}

// ---- Presets ----

/** Wire format for one chain entry inside a preset. Matches divora-core::presets. */
export interface WireChainEntry {
  id: string;
  enabled: boolean;
  vals: Record<string, number>;
}

/** Wire format for one preset, both bundled and user. */
export interface WirePreset {
  id: string;
  version: number;
  name: string;
  color: string;
  glyph: string;
  tag: "Bundled" | "User";
  desc: string;
  chain: WireChainEntry[];
}

export async function listPresets(): Promise<WirePreset[]> {
  return invoke<WirePreset[]>("list_presets");
}

export async function saveUserPreset(preset: WirePreset): Promise<void> {
  await invoke("save_user_preset", { preset });
}

export async function deleteUserPreset(id: string): Promise<void> {
  await invoke("delete_user_preset", { id });
}

export async function exportPresetJson(preset: WirePreset): Promise<string> {
  return invoke<string>("export_preset_json", { preset });
}

/**
 * v1.14.0: import a preset from a `.json` file on disk (the counterpart to
 * export). The backend reads + validates it, forces it to a **User**
 * preset with a unique, filesystem-safe id, saves it, and returns the
 * saved preset so the UI can select it.
 */
export async function importPreset(path: string): Promise<WirePreset> {
  return invoke<WirePreset>("import_preset", { path });
}

export async function presetStorePath(): Promise<string> {
  return invoke<string>("preset_store_path");
}

// ---- Voice library (Phase 12) ----

/** One installed voice-conversion model (`*.onnx` in the voices dir). */
export interface VoiceInfo {
  /** File stem — also the id the backend derives when loading. */
  id: string;
  name: string;
  path: string;
  sizeBytes: number;
}

/** Whether voice conversion can run + where models live. */
export interface OnnxRuntimeStatus {
  /** True when an onnxruntime shared library is locatable. */
  runtimeAvailable: boolean;
  voicesDir: string;
}

export async function voicesDir(): Promise<string> {
  return invoke<string>("voices_dir");
}

export async function listVoices(): Promise<VoiceInfo[]> {
  return invoke<VoiceInfo[]>("list_voices");
}

export async function onnxRuntimeStatus(): Promise<OnnxRuntimeStatus> {
  return invoke<OnnxRuntimeStatus>("onnx_runtime_status");
}

/**
 * Point the `VoiceConvert` effect at `index` in the live chain at a
 * model file path, or `null` to clear (passthrough). The backend loads
 * the model on a background thread, so this resolves immediately.
 */
export async function setVoiceModel(
  index: number,
  path: string | null,
): Promise<void> {
  await invoke("set_voice_model", { index, path });
}

// ---- Text-to-speech ("Speak") — v1.17.0 ----

/** One preset "Speak" voice. `installed` is false until the Kokoro model +
 *  voice pack + espeak-ng are staged in the bundle, so the UI can show a
 *  clear "voice not installed" state instead of failing on Speak. */
export interface TtsVoiceInfo {
  /** Kokoro voice id (e.g. "af_heart") — also the style-pack key. */
  id: string;
  name: string;
  /** espeak language used to phonemize this voice (e.g. "en-us"). */
  lang: string;
  /** True once every asset needed to synthesize is present on disk. */
  installed: boolean;
}

/** List the preset Speak voices, each flagged with whether it's installed. */
export async function listTtsVoices(): Promise<TtsVoiceInfo[]> {
  return invoke<TtsVoiceInfo[]>("list_tts_voices");
}

/**
 * Synthesize `text` with the preset `voiceId` and play it through the output,
 * mixed with the live mic via the soundboard seam (so a Discord/stream
 * listener hears it too). Resolves to the clip's duration in seconds. Rejects
 * with a message (e.g. "voices are not installed") the UI can surface.
 *
 * `gain` (v1.18.0) is the linear playback volume (1.0 = unchanged).
 * `previewOnly` (v1.18.0) routes the speech to your local monitor only — you
 * hear it, the call doesn't — for previewing before sending.
 * `candidates` (v1.25.0) is the best-of-N count for `VoxCPM`-cloned voices: the
 * backend generates N takes and keeps the one closest to the reference speaker
 * (higher = more faithful but slower; ignored by preset/OpenVoice voices).
 */
export async function speak(
  text: string,
  voiceId: string,
  gain = 1.0,
  previewOnly = false,
  candidates?: number,
  useGpu?: boolean,
): Promise<number> {
  return invoke<number>("speak", { text, voiceId, gain, previewOnly, candidates, useGpu });
}

/** Stop any in-flight synthesized speech playing through the mixer. */
export async function stopSpeak(): Promise<void> {
  await invoke("stop_speak");
}

/** One saved Speak (TTS) clip, for the "Saved clips" list (v1.28.0). */
export interface SpeakClip {
  id: string;
  text: string;
  voice: string;
  /** Milliseconds since the Unix epoch. */
  createdAt: number;
  durationSecs: number;
  /** Absolute path to the WAV — replay via {@link playSoundboardClip}. */
  path: string;
}

/** Absolute path of the saved-clips folder (for an "Open folder" button). */
export async function speakClipsDir(): Promise<string> {
  return invoke<string>("speak_clips_dir");
}

/** Open the saved-clips folder in the OS file manager (Explorer on Windows). */
export async function openSpeakClipsFolder(): Promise<void> {
  await invoke("open_speak_clips_folder");
}

/** List saved Speak clips, newest first. */
export async function listSpeakClips(): Promise<SpeakClip[]> {
  return invoke<SpeakClip[]>("list_speak_clips");
}

/** Delete a saved Speak clip by id. */
export async function deleteSpeakClip(id: string): Promise<void> {
  await invoke("delete_speak_clip", { id });
}

// ---- Voice cloning ("Your voices") — v1.20.0 ----

/** One user-cloned voice. Its `id` is selectable in Speak like a preset. */
export interface ClonedVoiceInfo {
  id: string;
  name: string;
  /**
   * Short label of the preset this clone was auto-matched to (v1.22.0), e.g.
   * `"Puck"`. Empty string when the stored base isn't a known preset.
   */
  baseName: string;
  /**
   * Cloning engine: `"voxcpm"` (accent-preserving) or `"openvoice"`
   * (timbre-only). The Speak screen shows the best-of-N quality control only
   * for `voxcpm` voices (v1.25.0).
   */
  engine: string;
}

/** List the user's cloned voices. */
export async function listClonedVoices(): Promise<ClonedVoiceInfo[]> {
  return invoke<ClonedVoiceInfo[]>("list_cloned_voices");
}

/**
 * Create a cloned voice from a reference audio file: the backend decodes the
 * clip, extracts a speaker embedding (OpenVoice), and stores it. Resolves to
 * the new voice. Rejects with a message (e.g. "not installed") on failure.
 */
export async function cloneVoice(
  name: string,
  referencePath: string,
): Promise<ClonedVoiceInfo> {
  return invoke<ClonedVoiceInfo>("clone_voice", { name, referencePath });
}

/**
 * v1.23.0: start capturing the dry microphone for an in-app clone reference,
 * so a user can add their voice without an external WAV. The audio engine
 * must be running; rejects with a message (e.g. "start the engine first")
 * otherwise.
 */
export async function startVoiceRecording(): Promise<void> {
  return invoke("start_voice_recording");
}

/**
 * v1.23.0: stop the in-app capture and clone it into a voice named `name`
 * (the transient clip is deleted afterward). Resolves to the new voice;
 * rejects with a message on failure.
 */
export async function stopVoiceRecording(
  name: string,
): Promise<ClonedVoiceInfo> {
  return invoke<ClonedVoiceInfo>("stop_voice_recording", { name });
}

/** Delete a cloned voice by id. */
export async function deleteClonedVoice(id: string): Promise<void> {
  await invoke("delete_cloned_voice", { id });
}

/** Whether the voice-cloning models are downloaded (v1.21.0). */
export interface CloneModelsStatus {
  ready: boolean;
}

/** Progress for the on-demand cloning-model download. */
export interface CloneDownloadProgress {
  file: number;
  fileCount: number;
  received: number;
  total: number;
}

export async function cloneModelsStatus(): Promise<CloneModelsStatus> {
  return invoke<CloneModelsStatus>("clone_models_status");
}

/**
 * VoxCPM accent-preserving cloning status. When `available`, the in-app
 * recorder produces an accent-preserving clone and should show `readPrompt`
 * for the user to read aloud (so the reference transcript is known).
 */
export interface VoxCpmStatus {
  available: boolean;
  readPrompt: string;
}

export async function voxcpmStatus(): Promise<VoxCpmStatus> {
  return invoke<VoxCpmStatus>("voxcpm_status");
}

/**
 * Download the voice-cloning models (~157 MB, one-time) into the user dir.
 * Resolves when complete; progress arrives via {@link subscribeCloneDownload}.
 */
export async function downloadCloneModels(): Promise<void> {
  await invoke("download_clone_models");
}

/**
 * v2: download the VoxCPM accent-cloning models (~1.6 GB, one-time). Resolves
 * when complete; progress arrives via {@link subscribeCloneDownload}.
 */
export async function downloadVoxcpmModels(): Promise<void> {
  await invoke("download_voxcpm_models");
}

/**
 * Experimental GPU (DirectML) cloning status. `supported` is whether the platform
 * can run the DirectML build; `modelPresent` is whether the opt-in fp16 GPU decode
 * model (~1.2 GB) is downloaded. The toggle is offered only when both hold.
 */
export interface VoxCpmGpuStatus {
  supported: boolean;
  modelPresent: boolean;
}

export async function voxcpmGpuStatus(): Promise<VoxCpmGpuStatus> {
  return invoke<VoxCpmGpuStatus>("voxcpm_gpu_status");
}

/**
 * Download the optional fp16 DirectML decode model (~1.2 GB, one-time) for GPU
 * cloning. Resolves when complete; progress arrives via {@link subscribeCloneDownload}.
 */
export async function downloadVoxcpmGpuModel(): Promise<void> {
  await invoke("download_voxcpm_gpu_model");
}

/** Subscribe to cloning-model download progress. Returns the unlisten fn. */
export async function subscribeCloneDownload(
  handler: (p: CloneDownloadProgress) => void,
): Promise<UnlistenFn> {
  return listen<CloneDownloadProgress>("clone-model-download", (e) =>
    handler(e.payload),
  );
}

/** Best-of-N synthesis progress for a VoxCPM cloned voice (v1.27.0): the
 *  backend is generating take `done` of `total`. */
export interface TtsProgress {
  done: number;
  total: number;
}

/** Subscribe to per-take TTS synthesis progress. Returns the unlisten fn. */
export async function subscribeTtsProgress(
  handler: (p: TtsProgress) => void,
): Promise<UnlistenFn> {
  return listen<TtsProgress>("tts-progress", (e) => handler(e.payload));
}

/**
 * Open the native file-picker for a voice reference clip and return the chosen
 * absolute path (or `null` if cancelled). Lazy-imports the dialog plugin so
 * tests mocking `@tauri-apps/api/core` don't need to mock it too.
 */
export async function pickAudioFile(): Promise<string | null> {
  try {
    const { open } = await import("@tauri-apps/plugin-dialog");
    const result = await open({
      multiple: false,
      title: "Pick a voice reference clip (20–30s of clear speech)",
      filters: [{ name: "Audio", extensions: ["wav", "mp3", "ogg", "flac", "m4a"] }],
    });
    return typeof result === "string" ? result : null;
  } catch (err) {
    console.warn("[clone] file picker unavailable", err);
    return null;
  }
}

// ---- Recording (Phase 16) ----

/** Absolute path of the recordings directory. */
export async function recordingsDir(): Promise<string> {
  return invoke<string>("recordings_dir");
}

/** v1.14.0: absolute path of the logs directory (for "Open logs folder"). */
export async function logsDir(): Promise<string> {
  return invoke<string>("logs_dir");
}

/**
 * Begin recording the modulated output. `filename` is the desired file
 * name (built from the local time); the backend sanitizes it to a single
 * path component, forces a `.wav` extension, and returns the full
 * destination path. Only takes effect while the engine is running.
 */
export async function startRecording(filename: string): Promise<string> {
  return invoke<string>("start_recording", { filename });
}

/** Stop the current recording and finalize the WAV file. */
export async function stopRecording(): Promise<void> {
  await invoke("stop_recording");
}

// ---- Soundboard ----

/** Wire format for one soundboard tile (output of `scan_soundboard_folder`). */
export interface SoundboardTile {
  id: string;
  path: string;
  label: string;
  extension: string;
  sizeBytes: number;
  modifiedSecs?: number;
}

/**
 * Open the native folder-picker dialog and return the selected absolute
 * path (or `null` if the user cancelled). Loads the dialog plugin lazily
 * so tests that mock `@tauri-apps/api/core` don't need to mock it too.
 */
export async function pickSoundboardFolder(): Promise<string | null> {
  try {
    const { open } = await import("@tauri-apps/plugin-dialog");
    const result = await open({
      directory: true,
      multiple: false,
      title: "Pick a soundboard folder",
    });
    if (typeof result === "string") return result;
    return null;
  } catch (err) {
    console.warn("[soundboard] folder picker unavailable", err);
    return null;
  }
}

export async function scanSoundboardFolder(
  folder: string,
): Promise<SoundboardTile[]> {
  return invoke<SoundboardTile[]>("scan_soundboard_folder", { folder });
}

/** Play a clip; returns the clip's duration in seconds. */
export async function playSoundboardClip(
  clipId: string,
  path: string,
  gain = 1.0,
): Promise<number> {
  return invoke<number>("play_soundboard_clip", { clipId, path, gain });
}

export async function stopSoundboardClip(clipId: string): Promise<void> {
  await invoke("stop_soundboard_clip", { clipId });
}

export async function stopAllSoundboardClips(): Promise<void> {
  await invoke("stop_all_soundboard_clips");
}

/** Phase 15: master soundboard gain (linear, 1.0 = unity). */
export async function setSoundboardMasterGain(gain: number): Promise<void> {
  await invoke("set_soundboard_master_gain", { gain });
}

// ---- Virtual mic (VB-Cable) ----

export interface VirtualMicStatus {
  detected: boolean;
  cableInputDevice: DeviceInfo | null;
  cableOutputDevice: DeviceInfo | null;
  downloadUrl: string;
}

export async function detectVirtualMic(): Promise<VirtualMicStatus> {
  return invoke<VirtualMicStatus>("detect_virtual_mic");
}

// ---- Global hotkeys ----

/** Periodic event payload emitted by the backend on every press / release. */
export interface GlobalShortcutEvent {
  id: string;
  accelerator: string;
  state: "pressed" | "released";
}

export async function registerGlobalShortcut(
  id: string,
  accelerator: string,
): Promise<void> {
  await invoke("register_global_shortcut", { id, accelerator });
}

export async function unregisterGlobalShortcut(id: string): Promise<void> {
  await invoke("unregister_global_shortcut", { id });
}

export async function unregisterAllGlobalShortcuts(): Promise<void> {
  await invoke("unregister_all_global_shortcuts");
}

/** Subscribe to global-shortcut press / release events. Returns the
 *  Tauri unlisten function. */
export async function subscribeGlobalShortcut(
  handler: (event: GlobalShortcutEvent) => void,
): Promise<UnlistenFn> {
  return listen<GlobalShortcutEvent>("global-shortcut", (e) => handler(e.payload));
}

// ---- MIDI control surfaces (v1.9.0) ----

/** One available MIDI input port. `id` is the stable handle (the port's
 *  display name) the backend re-resolves on open — midir port indices
 *  aren't stable across hot-plug. */
export interface MidiInputInfo {
  id: string;
  name: string;
}

/** Payload of the backend `midi-message` event. `kind` is a stable
 *  string ("note-on" | "note-off" | "control-change" | …); `data1` /
 *  `data2` are the two MIDI data bytes (note + velocity, or controller
 *  + value). */
export interface MidiMessage {
  channel: number;
  kind: string;
  data1: number;
  data2: number;
}

export async function listMidiInputs(): Promise<MidiInputInfo[]> {
  return invoke<MidiInputInfo[]>("list_midi_inputs");
}

/** Open the named MIDI input port; the backend then emits `midi-message`
 *  events for each note / CC. Opening replaces any existing connection. */
export async function openMidiInput(name: string): Promise<void> {
  await invoke("open_midi_input", { name });
}

/** Close the live MIDI input port (if any). */
export async function closeMidiInput(): Promise<void> {
  await invoke("close_midi_input");
}

/** Subscribe to `midi-message` events. Returns the Tauri unlisten function. */
export async function subscribeMidi(
  handler: (msg: MidiMessage) => void,
): Promise<UnlistenFn> {
  return listen<MidiMessage>("midi-message", (e) => handler(e.payload));
}
