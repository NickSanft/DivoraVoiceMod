import { describe, expect, it } from "vitest";
import type { CloneDownloadProgress } from "../audio/api";
import {
  downloadAfter,
  isModelDownloadKind,
  readDownloadProgress,
  type ModelDownload,
} from "./modelDownload";

const p = (more: Partial<CloneDownloadProgress> = {}): CloneDownloadProgress => ({
  file: 1,
  fileCount: 2,
  received: 0,
  total: 0,
  ...more,
});

describe("isModelDownloadKind", () => {
  it("is the backend's three names, camelCase", () => {
    expect(["clone", "voxcpm", "voxcpmGpu"].every(isModelDownloadKind)).toBe(true);
    expect(["gpu", "voxcpm_gpu", "Clone", "", null, undefined, 1].some(isModelDownloadKind)).toBe(
      false,
    );
  });
});

describe("readDownloadProgress", () => {
  it("reads the status command's answer, kind included", () => {
    const running = { file: 2, fileCount: 6, received: 3_145_728, total: 6_291_456, kind: "voxcpm" };
    expect(readDownloadProgress(running)).toEqual(running);
  });

  it("drops a kind it doesn't know, not the progress", () => {
    expect(readDownloadProgress({ ...p({ received: 5 }), kind: "turbo" })).toEqual(p({ received: 5 }));
    expect(readDownloadProgress(p())).toEqual(p());
  });

  it("is null for nothing running, a mock's answer, or a count that isn't one", () => {
    expect(readDownloadProgress(null)).toBeNull();
    expect(readDownloadProgress(undefined)).toBeNull();
    expect(readDownloadProgress("clone")).toBeNull();
    expect(readDownloadProgress({ ready: true })).toBeNull();
    for (const key of ["file", "fileCount", "received", "total"] as const) {
      expect(readDownloadProgress({ ...p(), [key]: undefined })).toBeNull();
      expect(readDownloadProgress({ ...p(), [key]: -1 })).toBeNull();
      expect(readDownloadProgress({ ...p(), [key]: 1.5 })).toBeNull();
      expect(readDownloadProgress({ ...p(), [key]: "1" })).toBeNull();
    }
  });
});

describe("downloadAfter", () => {
  it("the % is of the file, from received over total", () => {
    expect(downloadAfter(null, p({ received: 50, total: 200, kind: "clone" }))).toEqual({
      kind: "clone",
      pct: 25,
    });
    expect(downloadAfter(null, p({ received: 2, total: 3 }))).toEqual({ kind: null, pct: 67 });
    // A new file starts again from its own 0.
    const held: ModelDownload = { kind: "voxcpm", pct: 100 };
    expect(downloadAfter(held, p({ file: 2, received: 0, total: 10, kind: "voxcpm" })).pct).toBe(0);
  });

  it("a payload with no length keeps the last %", () => {
    expect(downloadAfter({ kind: "voxcpmGpu", pct: 42 }, p({ received: 99, total: 0 }))).toEqual({
      kind: "voxcpmGpu",
      pct: 42,
    });
    expect(downloadAfter(null, p({ received: 99, total: 0 }))).toEqual({ kind: null, pct: 0 });
  });

  it("takes the payload's kind, else keeps the held one", () => {
    expect(downloadAfter({ kind: "clone", pct: 0 }, p({ kind: "voxcpm" })).kind).toBe("voxcpm");
    expect(downloadAfter({ kind: "clone", pct: 0 }, p()).kind).toBe("clone");
  });

  it("is always a new object, even when nothing changed", () => {
    const held: ModelDownload = { kind: "clone", pct: 0 };
    const next = downloadAfter(held, p({ kind: "clone" }));
    expect(next).toEqual(held);
    expect(next).not.toBe(held);
  });
});
