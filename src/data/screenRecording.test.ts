import { describe, expect, it } from "vitest";
import {
  DEFAULT_SCREEN_SETTINGS,
  doubledHint,
  formatClock,
  IDLE_SCREEN_RECORDING,
  monitorLabel,
  pickDesktopSource,
  pickMonitor,
  quietWarning,
  readScreenSettings,
  screenFilename,
  screenRecordSubtitle,
} from "./screenRecording";
import type { CaptureMonitor, DesktopAudioSource } from "../audio/api";

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
    const desktopOff = { desktopAudio: "off", desktopGainDb: -6, stems: false };
    expect(
      readScreenSettings({
        monitorId: side.id,
        fps: 45,
        quality: "high",
        cursor: "yes",
        avOffsetMs: 9999,
      }),
    ).toEqual({
      monitorId: side.id,
      fps: 30,
      quality: "high",
      cursor: true,
      avOffsetMs: 500,
      ...desktopOff,
    });
    expect(readScreenSettings({ fps: 60, quality: "ultra", cursor: false, avOffsetMs: -40.4 }))
      .toEqual({
        monitorId: null,
        fps: 60,
        quality: "balanced",
        cursor: false,
        avOffsetMs: -40,
        ...desktopOff,
      });
  });

  it("v1.53.0: desktop audio is a first run's default, never an upgrade's", () => {
    expect(DEFAULT_SCREEN_SETTINGS.desktopAudio).toBe("everything");
    expect(readScreenSettings(null, true).desktopAudio).toBe("everything");
    expect(readScreenSettings(null, false).desktopAudio).toBe("off");
    // Settings saved before v1.53.0 have no desktopAudio at all.
    expect(readScreenSettings({ fps: 60 }, true).desktopAudio).toBe("off");
  });

  it("v1.53.0: desktop fields are checked like the rest", () => {
    expect(
      readScreenSettings({ desktopAudio: "device:{abc}", desktopGainDb: 4, stems: true }),
    ).toMatchObject({ desktopAudio: "device:{abc}", desktopGainDb: 0, stems: true });
    expect(
      readScreenSettings({ desktopAudio: "everything", desktopGainDb: -99.6, stems: "no" }),
    ).toMatchObject({ desktopAudio: "everything", desktopGainDb: -30, stems: false });
    for (const bad of ["device:", "app:12", "Everything", 3, null]) {
      expect(readScreenSettings({ desktopAudio: bad }).desktopAudio).toBe("off");
    }
    expect(readScreenSettings({ desktopGainDb: Number.NaN }).desktopGainDb).toBe(-6);
  });
});

const everything: DesktopAudioSource = {
  id: "everything",
  label: "Everything except Divora",
  kind: "everything",
  isDefault: false,
  divoraPlays: "nothing",
};
const phones: DesktopAudioSource = {
  id: "device:{phones}",
  label: "Headphones",
  kind: "device",
  isDefault: true,
  divoraPlays: "monitor",
};

describe("desktop audio", () => {
  it("picks the saved source and never a stand-in", () => {
    expect(pickDesktopSource([everything], "everything")).toBe(everything);
    expect(pickDesktopSource([everything], "off")).toBeNull();
    // Saved "everything" on a PC that can only offer devices: not swapped
    // for a device, which might be playing Divora's own sound.
    expect(pickDesktopSource([phones], "everything")).toBeNull();
    expect(pickDesktopSource([phones], "device:{gone}")).toBeNull();
    expect(pickDesktopSource([phones], "device:{phones}")).toBe(phones);
  });

  it("says what the take records", () => {
    expect(screenRecordSubtitle(null)).toBe("Your modulated voice and soundboard");
    expect(screenRecordSubtitle(everything)).toMatch(/plus sound from other apps/);
    expect(screenRecordSubtitle(phones)).toMatch(/what Headphones plays/);
    // A device Divora sends your voice to is never recorded.
    expect(screenRecordSubtitle({ ...phones, divoraPlays: "send" })).toBe(
      "Your modulated voice and soundboard",
    );
  });

  it("warns about devices that carry Divora's own sound, by what plays there", () => {
    expect(doubledHint(everything)).toBeNull();
    expect(doubledHint(null)).toBeNull();
    expect(doubledHint(phones)).toMatch(/hear yourself/);
    expect(doubledHint({ ...phones, divoraPlays: "send" })).toMatch(/sends your voice/);
    expect(doubledHint({ ...phones, divoraPlays: "onlyOutput" })).toMatch(/only output/);
    expect(doubledHint({ ...phones, divoraPlays: "unknown" })).toBeNull();
  });

  it("warns about silence only after ten seconds, in words that fit the source", () => {
    const base = { ...IDLE_SCREEN_RECORDING, state: "recording" as const };
    expect(quietWarning({ ...base, desktopSourceId: "everything", desktopQuietSeconds: 9.9 })).toBeNull();
    expect(
      quietWarning({ ...base, desktopSourceId: "everything", desktopQuietSeconds: 12.4 }),
    ).toMatch(/Nothing heard from other apps for 12 s/);
    expect(
      quietWarning({
        ...base,
        desktopSourceId: "device:{phones}",
        desktopSource: "Headphones",
        desktopQuietSeconds: 30,
      }),
    ).toMatch(/Nothing heard from Headphones for 30 s/);
    // Without desktop audio there is nothing to warn about.
    expect(quietWarning({ ...base, desktopQuietSeconds: 30 })).toBeNull();
    // Nor while the card already says why it is silent: left out (a device
    // that came to carry your voice), lost, or stopped for good.
    const quiet = { ...base, desktopSourceId: "everything", desktopQuietSeconds: 30 };
    expect(quietWarning({ ...quiet, desktopError: "Desktop audio stopped at 0:05" })).toBeNull();
    expect(quietWarning({ ...quiet, desktopLostAt: 5 })).toBeNull();
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
