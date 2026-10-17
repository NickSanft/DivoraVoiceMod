// v1.54.0: Replay — its stored settings, and the words and numbers around a
// ring and a saved clip. Pure, so it unit-tests; the store does the invoking.

import type {
  DesktopAudioSource,
  ReplayArmed,
  ReplayHeldReason,
  ReplayLength,
  ReplaySaved,
  ReplaySeconds,
  ReplayStatus,
} from "../audio/api";
import { formatClock } from "./screenRecording";

/** The lengths offered, shortest first. */
export const REPLAY_LENGTHS: readonly ReplaySeconds[] = [15, 30, 60, 120];

/** Persisted as `divora.replayBuffer`. Never whether Replay is on: it is off
 *  at every launch, so nothing here can turn it on by itself. */
export interface ReplaySettings {
  seconds: ReplaySeconds;
  /** The card's explainer has been read and Replay turned on from it. */
  explained: boolean;
  /** …and read while desktop audio was on, so it named the calls Replay
   *  would keep. Seen with desktop audio off, it is shown again. */
  explainedDesktop: boolean;
}

export const DEFAULT_REPLAY_SETTINGS: ReplaySettings = {
  seconds: 30,
  explained: false,
  explainedDesktop: false,
};

/** 15, 30, 60 or 120; anything else is 30, as the backend reads it. */
export function sanitizeReplaySeconds(n: unknown): ReplaySeconds {
  return (REPLAY_LENGTHS as readonly unknown[]).includes(n) ? (n as ReplaySeconds) : 30;
}

/** Stored settings, field by field: each falls back alone, and anything
 *  else stored there (an `armed` flag above all) is dropped. */
export function readReplaySettings(raw: unknown): ReplaySettings {
  const out = { ...DEFAULT_REPLAY_SETTINGS };
  if (typeof raw !== "object" || raw === null || Array.isArray(raw)) return out;
  const o = raw as Record<string, unknown>;
  out.seconds = sanitizeReplaySeconds(o.seconds);
  if (typeof o.explained === "boolean") out.explained = o.explained;
  if (typeof o.explainedDesktop === "boolean") out.explainedDesktop = o.explainedDesktop;
  return out;
}

/** Whether Replay may be turned on only from the card's explainer: never
 *  read, or read before desktop audio — and the calls it carries — was on. */
export function explainerNeeded(settings: ReplaySettings, desktopOn: boolean): boolean {
  return !settings.explained || (desktopOn && !settings.explainedDesktop);
}

/** Said when anything but the card's explainer tries to turn Replay on first. */
export const REPLAY_GATE_TOAST =
  "Turn Replay on from the Record screen card first — it explains what Replay keeps.";

/** What the store shows before the backend has said anything. Supported
 *  until told otherwise: an arm the backend refuses says why. */
export const OFF_REPLAY_STATUS: ReplayStatus = {
  supported: true,
  reason: null,
  state: "off",
  saving: false,
  seconds: 30,
  bufferedMs: 0,
  bytes: 0,
  capBytes: 0,
  monitorId: null,
  heldReason: null,
  armed: null,
};

/** On in any form: arming, keeping, or held with what it kept. */
export function replayOn(status: ReplayStatus): boolean {
  return status.state !== "off";
}

const MB = 1_000_000;

/** Memory is in MB of 10⁶ bytes, the unit the backend's refusal uses. A cap
 *  rounds UP — "up to" must never be below what it can use — and a ceiling
 *  rounds DOWN, so an option refused against a ceiling always reads larger
 *  than it. */
export function capMb(bytes: number): number {
  return Math.ceil(bytes / MB);
}

export function ceilingMb(bytes: number): number {
  return Math.floor(bytes / MB);
}

/** `up to 81 MB`: the cap, worded as the maximum it is. */
export function memoryLabel(capBytes: number): string {
  return `up to ${capMb(capBytes)} MB`;
}

/** Why a length is greyed out (`null` while it is allowed). */
export function lengthTitle(len: ReplayLength, ceilingBytes: number): string | null {
  if (len.allowed) return null;
  return `Needs ${memoryLabel(len.capBytes)}; this PC allows ${ceilingMb(ceilingBytes)} MB.`;
}

/** What a held ring says. It can still be saved; only the way back differs. */
export function heldCopy(reason: ReplayHeldReason | null): string {
  switch (reason) {
    case "engine-stopped":
      return "The audio engine stopped, so Replay stopped keeping. Save still saves what it kept; starting the engine clears it and starts again.";
    case "screen-lost":
      return "That screen was disconnected, so Replay stopped keeping. Save still saves what it kept; turn Replay off and on again to keep a screen.";
    case "encoder-failed":
      return "The video encoder stopped working, so Replay stopped keeping. Save still saves what it kept; turn Replay off and on again to try again.";
    default:
      return "Replay stopped keeping. Save still saves what it kept.";
  }
}

/** The sound a running ring keeps besides the picture. Any desktop audio
 *  can carry a call, so the long form says so. */
export function keptSources(armed: ReplayArmed | null): string {
  return armed?.desktopAudio
    ? "your voice and soundboard, and sound from other apps (calls included)"
    : "your voice and soundboard";
}

/** `Screen 1`, or a neutral stand-in before the screens are known. */
function screenName(screenNumber: number | null): string {
  return screenNumber === null ? "a screen" : `Screen ${screenNumber}`;
}

/** The card's line while Replay is on, in place of the pickers it locks. */
export function lockedLine(status: ReplayStatus, screenNumber: number | null): string {
  return `Replay is keeping ${screenName(screenNumber)}, ${keptSources(status.armed)}. Turn it off to change the screen or desktop audio.`;
}

/** The titlebar chip's text. Held says so in place of the length. */
export function chipLabel(status: ReplayStatus): string {
  return status.state === "held" ? "REPLAY · HELD" : `REPLAY · ${status.seconds} s`;
}

/** The chip's tooltip: what is being kept, and where to see it. */
export function chipTitle(status: ReplayStatus, screenNumber: number | null): string {
  if (status.state === "held") return heldCopy(status.heldReason);
  return `Replay is keeping the last ${status.seconds} s of ${screenName(screenNumber)}, ${keptSources(status.armed)} in memory. Windows draws a yellow border around that screen.`;
}

/** The card's Save button: the full length once the ring holds it, how
 *  much so far while it fills. */
export function saveLabel(status: ReplayStatus): string {
  if (status.saving) return "Saving…";
  if (status.bufferedMs >= status.seconds * 1000) return `Save the last ${status.seconds} s`;
  return `Save (${formatClock(status.bufferedMs)} so far)`;
}

/** The explainer's account of the card's desktop audio, in words. `sources`
 *  is null until they have been asked for. */
export function desktopSettingWords(
  saved: string,
  source: DesktopAudioSource | null,
  sources: readonly DesktopAudioSource[] | null,
): string {
  if (saved === "off") return "Desktop audio is off here.";
  if (source) return `Desktop audio is on here: ${source.label}.`;
  if (sources === null && saved === "everything") {
    return "Desktop audio is on here: Everything except Divora.";
  }
  return "Desktop audio is on here, but its source isn't offered now, so Replay leaves it out.";
}

/** How long the save toast says a clip is. Never "0 s": a clip has at
 *  least its first keyframe's worth. */
export function clipSeconds(durationMs: number): number {
  return Math.max(1, Math.round(durationMs / 1000));
}

/** The save's own latency: the cut, the snapshot and the write. */
export const SAVE_LATENCY_MS = 3000;

/** Whether the clip held the disguised voice from its first frame: the
 *  voice became modulated with the current preset (`modulatedSince`, on the
 *  clock `receivedMs` is read from) no later than the clip's start, allowing
 *  for the save's own latency. */
export function clipWasModulated(
  modulatedSince: number | null,
  receivedMs: number,
  durationMs: number,
): boolean {
  return modulatedSince !== null && modulatedSince <= receivedMs - durationMs - SAVE_LATENCY_MS;
}

/** The toast for a `replay-saved`. A preset is named only when the clip held
 *  it throughout; otherwise it held what the call heard, which may be the
 *  user's own voice — and saying it was disguised would mislead anyone the
 *  clip is shared with. */
export function savedToast(saved: ReplaySaved, presetName: string | null): string {
  if (saved.error) return saved.error;
  const voice = presetName ?? "your voice as your call heard it";
  return `Saved a ${clipSeconds(saved.durationMs)} s replay · ${voice}`;
}
