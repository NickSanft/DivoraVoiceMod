// v1.57.0: the model downloads' pure rules, for the store.
//
// The backend runs a download on a worker and keeps its last progress, so the
// page stays live — and can be reloaded — while one runs. The page hears it
// through `clone-model-download` (progress) and `clone-model-download-done`
// (the end), whichever page started it, and reads it once at creation with
// `model_download_status`. Pure, so it unit-tests.

import type { CloneDownloadProgress, ModelDownloadKind } from "../audio/api";

/** What the page shows of the running download. */
export interface ModelDownload {
  /** Which download; null while nothing has said (a payload without `kind`). */
  kind: ModelDownloadKind | null;
  /** 0–100, of the file being downloaded. */
  pct: number;
}

const KINDS: readonly string[] = ["clone", "voxcpm", "voxcpmGpu"];

export function isModelDownloadKind(v: unknown): v is ModelDownloadKind {
  return typeof v === "string" && KINDS.includes(v);
}

/** A count: a whole number, 0 or more. */
function isCount(v: unknown): v is number {
  return typeof v === "number" && Number.isSafeInteger(v) && v >= 0;
}

/** `model_download_status`'s answer → a progress, or null when it isn't one
 *  (none running, a mock's undefined, a missing or negative count). An
 *  unknown `kind` is dropped, not the whole answer. */
export function readDownloadProgress(raw: unknown): CloneDownloadProgress | null {
  if (typeof raw !== "object" || raw === null) return null;
  const { file, fileCount, received, total, kind } = raw as Record<string, unknown>;
  if (!isCount(file) || !isCount(fileCount) || !isCount(received) || !isCount(total)) {
    return null;
  }
  return isModelDownloadKind(kind)
    ? { file, fileCount, received, total, kind }
    : { file, fileCount, received, total };
}

/** What the page shows after progress `p`, given what it showed (`held`). The
 *  % is of the current file, as before v1.57.0; a payload with no length
 *  (`total` 0) keeps the last %. Always a new object: the store tells its own
 *  start apart from a later download by identity. */
export function downloadAfter(
  held: ModelDownload | null,
  p: CloneDownloadProgress,
): ModelDownload {
  return {
    kind: isModelDownloadKind(p.kind) ? p.kind : (held?.kind ?? null),
    pct: p.total > 0 ? Math.round((p.received / p.total) * 100) : (held?.pct ?? 0),
  };
}
