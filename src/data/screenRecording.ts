// v1.52.0: screen recording — the settings, names and labels around a take.
// Pure, so it unit-tests; the store does the invoking.

import type {
  CaptureMonitor,
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
}

export const DEFAULT_SCREEN_SETTINGS: ScreenSettings = {
  monitorId: null,
  fps: 30,
  quality: "balanced",
  cursor: true,
  avOffsetMs: 0,
};

/** The A/V offset's range, both ways (the backend clamps to the same). */
export const MAX_AV_OFFSET_MS = 500;

const QUALITIES: readonly ScreenQuality[] = ["small", "balanced", "high"];

/** Stored settings, field by field: anything missing or malformed falls
 *  back to its default rather than spoiling the rest. */
export function readScreenSettings(raw: unknown): ScreenSettings {
  const out = { ...DEFAULT_SCREEN_SETTINGS };
  if (typeof raw !== "object" || raw === null || Array.isArray(raw)) return out;
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
  return out;
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
};

/** Whether a take is under way in any phase. */
export function screenBusy(u: ScreenRecordingUpdate): boolean {
  return u.state !== "idle";
}
