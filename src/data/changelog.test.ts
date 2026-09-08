// The trigger rules decide whether a user is interrupted on launch, so the
// interesting cases are all failure-shaped: a brand-new install must stay
// quiet, a dev build must not burn the marker, and nothing may repeat.

import { describe, expect, it } from "vitest";
import {
  decideWhatsNew,
  notesSince,
  RELEASE_NOTES,
  RENDER_CAP,
} from "./changelog";
import { BUNDLED_RELEASES } from "../../vite/changelog-plugin";

describe("decideWhatsNew", () => {
  it("stays silent on a brand-new install", () => {
    // No marker AND no wizard => first launch ever. Showing "what's new
    // since a version you never ran" is the headline bug to avoid; the
    // first-run wizard owns this moment.
    expect(
      decideWhatsNew({ version: "1.49.0", lastSeen: null, wizardSeen: false }),
    ).toEqual({ action: "seed" });
  });

  it("shows only the current version to someone who upgraded into the feature", () => {
    // Wizard done but no marker: a real user on an older build who just
    // updated. We can't know where they came from, so we don't invent a range.
    expect(
      decideWhatsNew({ version: "1.49.0", lastSeen: null, wizardSeen: true }),
    ).toEqual({ action: "show", since: null });
  });

  it("shows the full range across a multi-version gap", () => {
    expect(
      decideWhatsNew({ version: "1.49.0", lastSeen: "1.44.0", wizardSeen: true }),
    ).toEqual({ action: "show", since: "1.44.0" });
  });

  it("does nothing when already seen", () => {
    expect(
      decideWhatsNew({ version: "1.49.0", lastSeen: "1.49.0", wizardSeen: true }),
    ).toEqual({ action: "none" });
  });

  it("never shows AND never seeds on a dev build", () => {
    // Seeding at 0.0.0 would silently suppress the panel on this machine's
    // first real install.
    for (const wizardSeen of [true, false]) {
      expect(
        decideWhatsNew({ version: "0.0.0", lastSeen: null, wizardSeen }),
      ).toEqual({ action: "none" });
    }
    expect(
      decideWhatsNew({ version: null, lastSeen: "1.40.0", wizardSeen: true }),
    ).toEqual({ action: "none" });
  });

  it("re-seeds quietly on a downgrade or a corrupt marker", () => {
    expect(
      decideWhatsNew({ version: "1.40.0", lastSeen: "1.49.0", wizardSeen: true }),
    ).toEqual({ action: "seed" });
    expect(
      decideWhatsNew({ version: "1.49.0", lastSeen: "garbage", wizardSeen: true }),
    ).toEqual({ action: "seed" });
  });
});

describe("notesSince", () => {
  it("returns just the running version when there's no marker", () => {
    const got = notesSince(null, "1.48.0");
    expect(got.notes.map((n) => n.version)).toEqual(["1.48.0"]);
    expect(got.moreCount).toBe(0);
  });

  it("returns the range, newest first, skipping internal releases", () => {
    const got = notesSince("1.42.0", "1.48.0");
    // 1.45.0 carries the whatsnew:skip marker.
    expect(got.notes.map((n) => n.version)).toEqual([
      "1.48.0",
      "1.47.0",
      "1.46.0",
      "1.44.0",
      "1.43.0",
    ]);
    expect(got.moreUnknown).toBe(false);
  });

  it("never shows a release newer than the running build", () => {
    const got = notesSince("1.40.0", "1.43.0");
    expect(got.notes.every((n) => n.version <= "1.43.0")).toBe(true);
    expect(got.notes.map((n) => n.version)).toContain("1.43.0");
    expect(got.notes.map((n) => n.version)).not.toContain("1.44.0");
  });

  it("flags — without a number — that older releases exist beyond the bundle", () => {
    // Someone updating from a year-old build. We hold ~12 releases, so the
    // true remainder is unknowable; printing a count would be a lie.
    const oldest = RELEASE_NOTES[RELEASE_NOTES.length - 1]!.version;
    const got = notesSince("0.1.0", "1.48.0");
    expect(got.moreUnknown).toBe(true);
    expect(got.notes).toHaveLength(RENDER_CAP);
    expect(oldest).not.toBe("0.1.0");
  });

  it("holds at least as many releases as it will ever render", () => {
    // If the render cap ever exceeds the bundled depth, moreCount silently
    // undercounts and the "and N earlier" line starts lying.
    expect(RENDER_CAP).toBeLessThanOrEqual(BUNDLED_RELEASES);
    expect(RELEASE_NOTES.length).toBeLessThanOrEqual(BUNDLED_RELEASES);
  });
});
