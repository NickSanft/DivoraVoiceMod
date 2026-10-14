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
import { isNewer } from "./version";

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

  it("returns the range, newest first", () => {
    // Taken from what is bundled now rather than pinned to version numbers:
    // only the newest BUNDLED_RELEASES ship, so pinned versions age out of
    // the window with every release (v1.51.4 pushed 1.42.0 out, and this
    // test failed on correct behaviour). `since` is a bundled release, so
    // nothing between it and `current` can be missing.
    const versions = RELEASE_NOTES.map((n) => n.version);
    const current = versions[1]!;
    const since = versions[6]!;
    const got = notesSince(since, current);
    expect(got.notes.map((n) => n.version)).toEqual(versions.slice(1, 6));
    expect(got.moreUnknown).toBe(false);
  });

  it("an internal release is never bundled", () => {
    // 1.45.0 carries the whatsnew:skip marker. While it still sits between
    // bundled releases (1.46.0 and 1.44.0 at v1.51.4) its absence shows the
    // marker is honoured on the real changelog; once it ages out of the
    // window this passes trivially, and the parser's fixture test
    // (changelog-parse.test.ts, SKIP_MARKER) is what still guards the rule.
    expect(RELEASE_NOTES.map((n) => n.version)).not.toContain("1.45.0");
  });

  it("a marker older than everything bundled reports the rest as unknown", () => {
    const oldest = RELEASE_NOTES[RELEASE_NOTES.length - 1]!.version;
    const got = notesSince("0.0.1", RELEASE_NOTES[0]!.version);
    expect(oldest).not.toBe("0.0.1");
    expect(got.moreUnknown).toBe(true);
  });

  it("never shows a release newer than the running build", () => {
    // Taken from the bundle, not named: a pinned version ages out of the
    // ~12-release window (1.43.0 did when 1.52.0 was stamped). And compared
    // as versions, not strings — as strings "1.5.0" sorts after "1.43.0".
    const running = RELEASE_NOTES[3]!.version;
    const newer = RELEASE_NOTES[2]!.version;
    const lastSeen = RELEASE_NOTES[7]!.version;
    const got = notesSince(lastSeen, running);
    expect(got.notes.every((n) => !isNewer(n.version, running))).toBe(true);
    expect(got.notes.map((n) => n.version)).toContain(running);
    expect(got.notes.map((n) => n.version)).not.toContain(newer);
  });

  it("flags — without a number — that older releases exist beyond the bundle", () => {
    // Someone updating from a year-old build. We hold ~12 releases, so the
    // true remainder is unknowable; printing a count would be a lie.
    // Read the running version off the bundle rather than naming one: with a
    // hardcoded ceiling this test drifted every release, because each new
    // entry pushes an old one out of the window and fewer notes fall below
    // the named version until the cap stops being reached at all.
    const oldest = RELEASE_NOTES[RELEASE_NOTES.length - 1]!.version;
    const current = RELEASE_NOTES[0]!.version;
    const got = notesSince("0.1.0", current);
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
