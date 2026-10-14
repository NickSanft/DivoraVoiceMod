import { describe, expect, it } from "vitest";
import {
  DEFAULT_SCREEN_SETTINGS,
  formatClock,
  monitorLabel,
  pickMonitor,
  readScreenSettings,
  screenFilename,
} from "./screenRecording";
import type { CaptureMonitor } from "../audio/api";

const main: CaptureMonitor = {
  id: "\\\\.\\DISPLAY11",
  number: 1,
  x: 0,
  y: 0,
  width: 3440,
  height: 1440,
  primary: true,
};
const side: CaptureMonitor = {
  id: "\\\\.\\DISPLAY12",
  number: 2,
  x: -1920,
  y: 217,
  width: 1920,
  height: 1080,
  primary: false,
};

describe("readScreenSettings", () => {
  it("nothing stored: the defaults", () => {
    expect(readScreenSettings(null)).toEqual(DEFAULT_SCREEN_SETTINGS);
    expect(readScreenSettings("junk")).toEqual(DEFAULT_SCREEN_SETTINGS);
    expect(readScreenSettings([1])).toEqual(DEFAULT_SCREEN_SETTINGS);
  });

  it("keeps what is valid and defaults the rest, field by field", () => {
    expect(
      readScreenSettings({
        monitorId: side.id,
        fps: 45,
        quality: "high",
        cursor: "yes",
        avOffsetMs: 9999,
      }),
    ).toEqual({ monitorId: side.id, fps: 30, quality: "high", cursor: true, avOffsetMs: 500 });
    expect(readScreenSettings({ fps: 60, quality: "ultra", cursor: false, avOffsetMs: -40.4 }))
      .toEqual({ monitorId: null, fps: 60, quality: "balanced", cursor: false, avOffsetMs: -40 });
  });
});

describe("screenFilename", () => {
  it("is timestamped like the WAV recorder's, and an mp4", () => {
    expect(screenFilename(new Date(2026, 9, 14, 9, 5, 3))).toBe(
      "divora-screen-2026-10-14_09-05-03.mp4",
    );
  });
});

describe("formatClock", () => {
  it("reads like a stopwatch", () => {
    expect(formatClock(0)).toBe("0:00");
    expect(formatClock(7_900)).toBe("0:07");
    expect(formatClock(754_000)).toBe("12:34");
    expect(formatClock(3_723_000)).toBe("1:02:03");
    expect(formatClock(-5)).toBe("0:00");
  });
});

describe("pickMonitor", () => {
  it("the saved screen if still there, else the main one, else any", () => {
    expect(pickMonitor([main, side], side.id)).toBe(side);
    expect(pickMonitor([main, side], "\\\\.\\DISPLAY99")).toBe(main);
    expect(pickMonitor([main, side], null)).toBe(main);
    expect(pickMonitor([side], null)).toBe(side);
    expect(pickMonitor([], null)).toBeNull();
  });

  it("labels screens by number and size", () => {
    expect(monitorLabel(main)).toBe("Screen 1 · 3440×1440 · main");
    expect(monitorLabel(side)).toBe("Screen 2 · 1920×1080");
  });
});
