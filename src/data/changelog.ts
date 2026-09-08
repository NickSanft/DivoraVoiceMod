// v1.49.0: the app-facing half of "What's new".
//
// Content comes from `virtual:changelog` — CHANGELOG.md, parsed at build
// time (vite/changelog-plugin.ts). Deliberately NOT the GitHub release
// body: that body is a SHA-256 checksum block plus a compare link, and it
// always describes the LATEST release rather than the one the user is
// running. Bundled notes can't desynchronise from the binary, cost no
// network request, and keep the local-first promise intact.

import notes from "virtual:changelog";
import type { ReleaseNote } from "./changelog-parse";
import { isNewer } from "./version";

export type { ReleaseNote };

/** Every release we ship notes for, newest first. */
export const RELEASE_NOTES: ReleaseNote[] = notes;

/** Most releases rendered at once. Someone updating after a long gap gets a
 *  readable page plus a link, not a wall. Must stay <= BUNDLED_RELEASES in
 *  vite/changelog-plugin.ts or `moreCount` below would undercount. */
export const RENDER_CAP = 10;

export const CHANGELOG_URL =
  "https://github.com/NickSanft/DivoraVoiceMod/blob/main/CHANGELOG.md";

export interface WhatsNewContent {
  /** Releases to render, newest first, already capped. */
  notes: ReleaseNote[];
  /** Further matching releases we hold but didn't render (exact). */
  moreCount: number;
  /** True when releases exist that predate what we bundle, so the real
   *  remainder is unknown — the UI must not print a number in that case. */
  moreUnknown: boolean;
}

/** Notes strictly newer than `since`, capped for display.
 *  `since === null` means "just this version" (we don't know where the
 *  user came from, so we don't guess). */
export function notesSince(
  since: string | null,
  current: string,
): WhatsNewContent {
  const matched =
    since === null
      ? RELEASE_NOTES.filter((n) => n.version === current)
      : RELEASE_NOTES.filter(
          (n) => isNewer(n.version, since) && !isNewer(n.version, current),
        );

  const oldest = RELEASE_NOTES[RELEASE_NOTES.length - 1];
  const moreUnknown =
    since !== null && oldest !== undefined && isNewer(oldest.version, since);

  return {
    notes: matched.slice(0, RENDER_CAP),
    moreCount: Math.max(0, matched.length - RENDER_CAP),
    moreUnknown,
  };
}

export type WhatsNewAction =
  /** Do nothing and leave the marker alone. */
  | { action: "none" }
  /** Record the version silently — no panel. */
  | { action: "seed" }
  /** Open the panel, then record. */
  | { action: "show"; since: string | null };

/** The whole trigger policy, in one pure function.
 *
 *  The case that matters most is a brand-new install: a first-time user
 *  must never be shown "here's what changed since a version you never ran".
 *  `wizardSeen` is what separates that from someone who upgraded into this
 *  feature and therefore has no marker yet. */
export function decideWhatsNew(opts: {
  /** Running app version, or null when unavailable (browser / tests). */
  version: string | null;
  /** Persisted marker, or null when absent. */
  lastSeen: string | null;
  /** Whether the first-run wizard has already been completed. */
  wizardSeen: boolean;
}): WhatsNewAction {
  const { version, lastSeen, wizardSeen } = opts;

  // Dev builds report "0.0.0". Showing the panel there is noise; WRITING the
  // marker there is worse — it would suppress the real thing on this
  // machine's first proper install.
  if (!version || version === "0.0.0") return { action: "none" };

  if (lastSeen === version) return { action: "none" };

  if (lastSeen === null) {
    // No marker and no wizard => first launch ever. The wizard owns this
    // moment; seed so the next update is the first thing we announce.
    if (!wizardSeen) return { action: "seed" };
    // No marker but the wizard is done => an existing user who just updated
    // into this feature. We can't know their previous version, so show only
    // what's running rather than inventing a range.
    return { action: "show", since: null };
  }

  // A marker that isn't older than the running build (a downgrade, or
  // garbage in localStorage) gets quietly re-seeded rather than shown.
  if (!isNewer(version, lastSeen)) return { action: "seed" };

  return { action: "show", since: lastSeen };
}
