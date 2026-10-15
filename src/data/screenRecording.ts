// v1.52.0: screen recording — the settings, names and labels around a take.
// Pure, so it unit-tests; the store does the invoking.

import type {
  CaptureMonitor,
  DesktopAudioSource,
  ScreenQuality,
  ScreenRecordingUpdate,
} from "../audio/api";

/** Settings → Recordings, persisted as `divora.screenRecording`. */
export interface ScreenSettings {
  /** The screen last picked on the card; `null` means the main screen. */
  monitorId: string | null;
  fps: 30 | 60;
  quality: ScreenQuality;
  cursor: boolean;
  /** Advanced: moves the sound against the picture, ms; positive is later. */
  avOffsetMs: number;
  /** v1.53.0: `everything`, `off`, or `device:<id>`. */
  desktopAudio: string;
  /** Desktop audio's volume in a take, dB (−30 to 0). */
  desktopGainDb: number;
  /** Also save the voice and the desktop audio as separate WAV files. */
  stems: boolean;
}

export const DEFAULT_SCREEN_SETTINGS: ScreenSettings = {
  monitorId: null,
  fps: 30,
  quality: "balanced",
  cursor: true,
  avOffsetMs: 0,
  desktopAudio: "everything",
  desktopGainDb: -6,
  stems: false,
};

/** The A/V offset's range, both ways (the backend clamps to the same). */
export const MAX_AV_OFFSET_MS = 500;

/** Desktop audio's volume range, dB (the backend clamps to the same). */
export const MIN_DESKTOP_GAIN_DB = -30;
export const MAX_DESKTOP_GAIN_DB = 0;

/** After this long without a sound from the desktop audio, the card says so. */
export const DESKTOP_QUIET_WARN_SECONDS = 10;

const DESKTOP_AUDIO = /^(everything|off|device:.+)$/;

const QUALITIES: readonly ScreenQuality[] = ["small", "balanced", "high"];

/** Stored settings, field by field: anything missing or malformed falls
 *  back to its default rather than spoiling the rest.
 *
 *  Desktop audio is the exception. Settings saved before it existed (or by
 *  someone who has used the app before — `firstRun` false) read it as off:
 *  a take started from a hotkey must not begin recording a call without
 *  the user having chosen that. A first run gets the default. */
export function readScreenSettings(raw: unknown, firstRun = true): ScreenSettings {
  const out = { ...DEFAULT_SCREEN_SETTINGS };
  if (typeof raw !== "object" || raw === null || Array.isArray(raw)) {
    if (!firstRun) out.desktopAudio = "off";
    return out;
  }
  const o = raw as Record<string, unknown>;
  if (typeof o.monitorId === "string" && o.monitorId) out.monitorId = o.monitorId;
  if (o.fps === 30 || o.fps === 60) out.fps = o.fps;
  if (typeof o.quality === "string" && (QUALITIES as readonly string[]).includes(o.quality)) {
    out.quality = o.quality as ScreenQuality;
  }
  if (typeof o.cursor === "boolean") out.cursor = o.cursor;
  if (typeof o.avOffsetMs === "number" && Number.isFinite(o.avOffsetMs)) {
    out.avOffsetMs = Math.round(
      Math.min(MAX_AV_OFFSET_MS, Math.max(-MAX_AV_OFFSET_MS, o.avOffsetMs)),
    );
  }
  out.desktopAudio =
    typeof o.desktopAudio === "string" && DESKTOP_AUDIO.test(o.desktopAudio)
      ? o.desktopAudio
      : "off";
  if (typeof o.desktopGainDb === "number" && Number.isFinite(o.desktopGainDb)) {
    out.desktopGainDb = Math.round(
      Math.min(MAX_DESKTOP_GAIN_DB, Math.max(MIN_DESKTOP_GAIN_DB, o.desktopGainDb)),
    );
  }
  if (typeof o.stems === "boolean") out.stems = o.stems;
  return out;
}

/** The desktop audio source a saved choice names, among those offered;
 *  `null` for off, or for a source that isn't offered (any more). Never a
 *  stand-in: another source could be playing Divora's own sound. */
export function pickDesktopSource(
  sources: readonly DesktopAudioSource[],
  saved: string,
): DesktopAudioSource | null {
  if (saved === "off") return null;
  return sources.find((s) => s.id === saved) ?? null;
}

/** What the card says the take records. */
export function screenRecordSubtitle(source: DesktopAudioSource | null): string {
  // A device Divora sends your voice to is never recorded.
  if (!source || source.divoraPlays === "send") return "Your modulated voice and soundboard";
  if (source.kind === "everything") {
    return "Your voice and soundboard, plus sound from other apps";
  }
  return `Your voice and soundboard, plus what ${source.label} plays`;
}

/** Said beside a playback device that already carries Divora's sound
 *  (`null` when it doesn't), before and during a take. */
export function doubledHint(source: DesktopAudioSource | null): string | null {
  switch (source?.divoraPlays) {
    case "send":
      return "Divora sends your voice to this device, so recording it would record your voice twice.";
    case "monitor":
      return "You hear yourself on this device: while Monitor is on, your voice is recorded twice, and while the Speak screen's Monitor is on (it is by default), so is the soundboard.";
    case "onlyOutput":
      return "This is Divora's only output: while Monitor is on, your voice and soundboard are recorded twice.";
    default:
      return null;
  }
}

/** The quiet warning, once desktop audio has been silent long enough —
 *  and not while the card already says why it is silent (left out, lost,
 *  or stopped for good). */
export function quietWarning(update: ScreenRecordingUpdate): string | null {
  const s = update.desktopQuietSeconds;
  if (
    s === null ||
    s < DESKTOP_QUIET_WARN_SECONDS ||
    !update.desktopSourceId ||
    update.desktopError ||
    update.desktopLostAt !== null
  ) {
    return null;
  }
  const secs = Math.floor(s);
  if (update.desktopSourceId === "everything") {
    return `Nothing heard from other apps for ${secs} s. If something is playing, its device's own sound software may be hiding it from recording.`;
  }
  return `Nothing heard from ${update.desktopSource ?? "that device"} for ${secs} s. If something is playing there, the device's own sound software may be hiding it from recording.`;
}

/** `divora-screen-2026-10-14_09-30-05.mp4`, from local time — the same
 *  shape as the WAV recorder's names, so the folder sorts by time. */
export function screenFilename(d: Date): string {
  const pad = (n: number): string => String(n).padStart(2, "0");
  return (
    `divora-screen-${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}` +
    `_${pad(d.getHours())}-${pad(d.getMinutes())}-${pad(d.getSeconds())}.mp4`
  );
}

/** `0:07`, `12:34`, `1:02:03`. */
export function formatClock(ms: number): string {
  const total = Math.max(0, Math.floor(ms / 1000));
  const h = Math.floor(total / 3600);
  const m = Math.floor((total % 3600) / 60);
  const s = total % 60;
  const ss = String(s).padStart(2, "0");
  return h > 0 ? `${h}:${String(m).padStart(2, "0")}:${ss}` : `${m}:${ss}`;
}

/** The screen to record: the saved one if it is still connected, else the
 *  main screen, else whatever there is. */
export function pickMonitor(
  monitors: readonly CaptureMonitor[],
  savedId: string | null,
): CaptureMonitor | null {
  return (
    monitors.find((m) => m.id === savedId) ??
    monitors.find((m) => m.primary) ??
    monitors[0] ??
    null
  );
}

/** `Screen 1 · 3440×1440 · main`. */
export function monitorLabel(m: CaptureMonitor): string {
  return `Screen ${m.number} · ${m.width}×${m.height}${m.primary ? " · main" : ""}`;
}

/** What the card shows before any take. */
export const IDLE_SCREEN_RECORDING: ScreenRecordingUpdate = {
  state: "idle",
  path: null,
  elapsedMs: 0,
  droppedFrames: 0,
  driftPpm: 0,
  audioLostAt: null,
  error: null,
  desktopSourceId: null,
  desktopSource: null,
  desktopQuietSeconds: null,
  desktopDoubled: false,
  desktopLostAt: null,
  desktopError: null,
  stemsError: null,
};

/** Whether a take is under way in any phase. */
export function screenBusy(u: ScreenRecordingUpdate): boolean {
  return u.state !== "idle";
}
