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
}

/** Periodic update emitted by the backend at ~30 Hz. */
export interface LevelUpdate {
  input: Levels;
  output: Levels;
  running: boolean;
  monitoring: boolean;
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
): Promise<StreamInfo> {
  return invoke<StreamInfo>("start_audio_engine", {
    inputName,
    outputName,
  });
}

export async function stopAudioEngine(): Promise<void> {
  await invoke("stop_audio_engine");
}

export async function setAudioMonitor(enabled: boolean): Promise<void> {
  await invoke("set_audio_monitor", { enabled });
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
