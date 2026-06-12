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
 */
export async function speak(text: string, voiceId: string): Promise<number> {
  return invoke<number>("speak", { text, voiceId });
}

/** Stop any in-flight synthesized speech playing through the mixer. */
export async function stopSpeak(): Promise<void> {
  await invoke("stop_speak");
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
