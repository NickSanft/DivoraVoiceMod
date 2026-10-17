import { describe, expect, it } from "vitest";
import type { ReplayArmed, ReplayStatus } from "../audio/api";
import {
  capMb,
  ceilingMb,
  chipLabel,
  chipTitle,
  clipSeconds,
  clipWasModulated,
  desktopSettingWords,
  explainerNeeded,
  heldCopy,
  keptSources,
  lengthTitle,
  lockedLine,
  memoryLabel,
  OFF_REPLAY_STATUS,
  readReplaySettings,
  REPLAY_LENGTHS,
  saveLabel,
  savedToast,
  sanitizeReplaySeconds,
} from "./replay";

const ARMED: ReplayArmed = {
  fps: 30,
  quality: "balanced",
  cursor: true,
  desktopAudio: null,
  desktopGainDb: -6,
  avOffsetMs: 0,
};
const armed = (patch: Partial<ReplayStatus> = {}): ReplayStatus => ({
  ...OFF_REPLAY_STATUS,
  state: "armed",
  seconds: 30,
  bufferedMs: 30_000,
  monitorId: "\\\\.\\DISPLAY1",
  armed: ARMED,
  ...patch,
});

describe("readReplaySettings", () => {
  it("nothing stored, or junk: 30 s, explainer unread", () => {
    const none = { seconds: 30, explained: false, explainedDesktop: false };
    expect(readReplaySettings(null)).toEqual(none);
    expect(readReplaySettings("junk")).toEqual(none);
    expect(readReplaySettings([60, true])).toEqual(none);
  });

  it("each field falls back alone", () => {
    expect(readReplaySettings({ seconds: 45, explained: true, explainedDesktop: true })).toEqual({
      seconds: 30,
      explained: true,
      explainedDesktop: true,
    });
    expect(readReplaySettings({ seconds: 120, explained: "yes", explainedDesktop: true })).toEqual({
      seconds: 120,
      explained: false,
      explainedDesktop: true,
    });
    expect(readReplaySettings({ seconds: 15, explained: true, explainedDesktop: 1 })).toEqual({
      seconds: 15,
      explained: true,
      explainedDesktop: false,
    });
  });

  it("drops an armed key — Replay is never on at launch — and anything else", () => {
    const read = readReplaySettings({ seconds: 60, explained: true, armed: true, monitorName: "x" });
    expect(read).toEqual({ seconds: 60, explained: true, explainedDesktop: false });
    expect(read).not.toHaveProperty("armed");
  });

  it("only the four lengths survive", () => {
    expect(REPLAY_LENGTHS).toEqual([15, 30, 60, 120]);
    for (const s of REPLAY_LENGTHS) expect(sanitizeReplaySeconds(s)).toBe(s);
    for (const s of [0, 31, "60", null, Number.NaN]) expect(sanitizeReplaySeconds(s)).toBe(30);
  });
});

describe("explainerNeeded", () => {
  const read = (explained: boolean, explainedDesktop: boolean) => ({
    seconds: 30 as const,
    explained,
    explainedDesktop,
  });

  it("is needed until read, and again for desktop audio if read without it", () => {
    expect(explainerNeeded(read(false, false), false)).toBe(true);
    expect(explainerNeeded(read(true, false), false)).toBe(false);
    expect(explainerNeeded(read(true, false), true)).toBe(true);
    expect(explainerNeeded(read(true, true), true)).toBe(false);
    // explainedDesktop alone is not "read".
    expect(explainerNeeded(read(false, true), true)).toBe(true);
  });
});

// The frozen memory table (design §B.9), recomputed here from the backend's
// formula as test data: estimate = (s + 2)·bitrate/8 + (s + 4)·192 000,
// cap = 5/4 of it, ceiling = min(1 GiB, total RAM / 8).
const BITRATES = [
  8_087_040, // 1080p30 Balanced
  16_174_080, // 1080p60 Balanced
  14_376_960, // 1440p30 Balanced (the default)
  28_753_920, // 1440p60 Balanced
  10_682_880, // 3440×1440 → 2560×1070 at 30
  21_365_760, // the same at 60
  46_006_272, // 1440p60 High
  8_626_176, // 1440p30 Small
];
const capOf = (seconds: number, bitrate: number): number => {
  const estimate = Math.floor(((seconds + 2) * bitrate) / 8) + (seconds + 4) * 192_000;
  return Math.floor((estimate * 5) / 4);
};
const ceilingOf = (totalPhys: number): number => Math.min(2 ** 30, Math.floor(totalPhys / 8));
const GIB = 2 ** 30;

describe("memory labels", () => {
  it("reproduces the frozen table's default: 30 s at 1440p30 Balanced is 80.0 MB", () => {
    expect(capOf(30, 14_376_960)).toBe(80_044_800);
  });

  it("a cap reads as 'up to', in MB of 10⁶ bytes, never below what it can use", () => {
    expect(memoryLabel(80_044_800)).toBe("up to 81 MB");
    expect(memoryLabel(80_000_000)).toBe("up to 80 MB");
    expect(memoryLabel(577_900_001)).toBe("up to 578 MB");
    for (const b of [1, 999_999, 1_000_001, 303_820_800, 906_800_000]) {
      expect(capMb(b) * 1_000_000).toBeGreaterThanOrEqual(b);
    }
  });

  it("a ceiling never reads above what it allows", () => {
    expect(ceilingMb(ceilingOf(4 * GIB))).toBe(536);
    for (const b of [536_870_912, 510_027_366, 1_073_741_824]) {
      expect(ceilingMb(b) * 1_000_000).toBeLessThanOrEqual(b);
    }
  });

  for (const [name, totalPhys] of [
    ["3.8 GiB", Math.floor(3.8 * GIB)],
    ["4 GiB", 4 * GIB],
  ] as const) {
    it(`every refused option's label is above the ceiling it is refused against (${name})`, () => {
      const ceiling = ceilingOf(totalPhys);
      const refused: string[] = [];
      for (const bitrate of BITRATES) {
        for (const seconds of REPLAY_LENGTHS) {
          const cap = capOf(seconds, bitrate);
          const allowed = cap <= ceiling;
          const title = lengthTitle({ seconds, bytes: 0, capBytes: cap, allowed }, ceiling);
          if (allowed) {
            expect(title).toBeNull();
            continue;
          }
          refused.push(`${seconds}@${bitrate}`);
          expect(capMb(cap)).toBeGreaterThan(ceilingMb(ceiling));
          const [, needs, allows] = /^Needs up to (\d+) MB; this PC allows (\d+) MB\.$/.exec(
            title ?? "",
          )!;
          expect(Number(needs)).toBeGreaterThan(Number(allows));
          // The option's own label quotes the same figure as its title.
          expect(memoryLabel(cap)).toBe(`up to ${needs} MB`);
        }
      }
      // §B.9: 120 s at 1440p60 Balanced and at 1440p60 High; 60 s High fits.
      expect(refused).toEqual(["120@28753920", "120@46006272"]);
    });
  }

  it("titles the design's example", () => {
    expect(
      lengthTitle({ seconds: 120, bytes: 0, capBytes: 577_900_000, allowed: false }, 4 * GIB / 8),
    ).toBe("Needs up to 578 MB; this PC allows 536 MB.");
  });
});

describe("words around a ring", () => {
  it("heldCopy says why for each reason, and that Save still works", () => {
    const copies = (["engine-stopped", "screen-lost", "encoder-failed"] as const).map(heldCopy);
    expect(copies[0]).toMatch(/^The audio engine stopped/);
    expect(copies[1]).toMatch(/^That screen was disconnected/);
    expect(copies[2]).toMatch(/^The video encoder stopped working/);
    for (const c of copies) expect(c).toContain("Save still saves what it kept");
    expect(new Set(copies).size).toBe(3);
    expect(heldCopy(null)).toContain("Save still saves what it kept");
  });

  it("keptSources names calls whenever desktop audio is kept", () => {
    expect(keptSources(ARMED)).toBe("your voice and soundboard");
    expect(keptSources(null)).toBe("your voice and soundboard");
    expect(keptSources({ ...ARMED, desktopAudio: "everything" })).toBe(
      "your voice and soundboard, and sound from other apps (calls included)",
    );
    expect(keptSources({ ...ARMED, desktopAudio: "device:{x}" })).toContain("(calls included)");
  });

  it("the locked line and the chip's title come from what the ring runs with", () => {
    expect(lockedLine(armed(), 1)).toBe(
      "Replay is keeping Screen 1, your voice and soundboard. Turn it off to change the screen or desktop audio.",
    );
    expect(
      chipTitle(armed({ armed: { ...ARMED, desktopAudio: "everything" } }), 2),
    ).toBe(
      "Replay is keeping the last 30 s of Screen 2, your voice and soundboard, and sound from other apps (calls included) in memory. Windows draws a yellow border around that screen.",
    );
    // Before the screens are known.
    expect(lockedLine(armed(), null)).toContain("Replay is keeping a screen,");
  });

  it("the chip reads its length, or HELD", () => {
    expect(chipLabel(armed({ seconds: 60 }))).toBe("REPLAY · 60 s");
    expect(chipLabel(armed({ state: "arming" }))).toBe("REPLAY · 30 s");
    expect(chipLabel(armed({ state: "held", heldReason: "screen-lost" }))).toBe("REPLAY · HELD");
    expect(chipTitle(armed({ state: "held", heldReason: "screen-lost" }), 1)).toBe(
      heldCopy("screen-lost"),
    );
  });

  it("the Save button says how much it holds", () => {
    expect(saveLabel(armed())).toBe("Save the last 30 s");
    expect(saveLabel(armed({ bufferedMs: 12_400 }))).toBe("Save (0:12 so far)");
    expect(saveLabel(armed({ saving: true }))).toBe("Saving…");
  });

  it("the explainer puts the card's desktop audio in words", () => {
    const everything = {
      id: "everything",
      label: "Everything except Divora",
      kind: "everything" as const,
      isDefault: false,
      divoraPlays: "nothing" as const,
    };
    expect(desktopSettingWords("off", null, [everything])).toBe("Desktop audio is off here.");
    expect(desktopSettingWords("everything", everything, [everything])).toBe(
      "Desktop audio is on here: Everything except Divora.",
    );
    expect(desktopSettingWords("everything", null, null)).toBe(
      "Desktop audio is on here: Everything except Divora.",
    );
    expect(desktopSettingWords("device:{gone}", null, [everything])).toContain(
      "isn't offered now",
    );
  });
});

describe("the save toast", () => {
  const ok = { path: "C:/rec/divora-replay-x.mp4", durationMs: 31_200, error: null };

  it("names the preset when told to, and the call's voice otherwise", () => {
    expect(savedToast(ok, "Villager")).toBe("Saved a 31 s replay · Villager");
    expect(savedToast(ok, null)).toBe("Saved a 31 s replay · your voice as your call heard it");
  });

  it("an error is said as it is", () => {
    expect(savedToast({ path: null, durationMs: 0, error: "Replay is off." }, "Villager")).toBe(
      "Replay is off.",
    );
  });

  it("never says a clip is 0 s long", () => {
    expect(clipSeconds(0)).toBe(1);
    expect(clipSeconds(400)).toBe(1);
    expect(clipSeconds(30_499)).toBe(30);
    expect(clipSeconds(30_500)).toBe(31);
  });

  it("the preset counts only if it was on from the clip's start, latency included", () => {
    // received at 100 s, a 30 s clip: the clip began no later than 67 s.
    expect(clipWasModulated(67_000, 100_000, 30_000)).toBe(true);
    expect(clipWasModulated(67_001, 100_000, 30_000)).toBe(false);
    expect(clipWasModulated(null, 100_000, 30_000)).toBe(false);
  });
});
