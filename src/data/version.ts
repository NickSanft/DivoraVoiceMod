// v1.12.0: tiny semver comparison for the in-app update check.
//
// We only ever ship plain `vX.Y.Z` release tags, so this ignores
// pre-release / build metadata and compares the numeric triple. Kept
// pure (no I/O) so it unit-tests in isolation.

export interface Semver {
  major: number;
  minor: number;
  patch: number;
}

/** Parse "v1.2.3" / "1.2.3" → {1,2,3}. Returns null on anything that
 *  isn't a leading numeric triple (so garbage never triggers a nag). */
export function parseSemver(input: string): Semver | null {
  if (typeof input !== "string") return null;
  const m = input.trim().replace(/^v/i, "").match(/^(\d+)\.(\d+)\.(\d+)/);
  if (!m) return null;
  return { major: Number(m[1]), minor: Number(m[2]), patch: Number(m[3]) };
}

/** True when `latest` is a strictly newer version than `current`.
 *  Either side malformed → false. */
export function isNewer(latest: string, current: string): boolean {
  const a = parseSemver(latest);
  const b = parseSemver(current);
  if (!a || !b) return false;
  if (a.major !== b.major) return a.major > b.major;
  if (a.minor !== b.minor) return a.minor > b.minor;
  return a.patch > b.patch;
}
