// v1.12.0: lightweight in-app update check.
//
// Asks the public GitHub Releases API for the latest release and reports
// it only when it's newer than the running build. One-way version read —
// no account, no telemetry, no payload sent. Best-effort: any network or
// parse failure resolves to null (never throws, never blocks the app).
// The caller gates this behind the opt-out flag and a real (non-dev,
// under-Tauri) build, so the browser / E2E never hit the network.

import { isNewer } from "./version";

export interface UpdateInfo {
  /** Latest release version with no leading "v" (e.g. "1.12.0"). */
  latest: string;
  /** Release page to open for the download. */
  url: string;
}

const LATEST_RELEASE_API =
  "https://api.github.com/repos/NickSanft/DivoraVoiceMod/releases/latest";
const RELEASES_PAGE =
  "https://github.com/NickSanft/DivoraVoiceMod/releases/latest";

export async function checkForUpdate(
  currentVersion: string,
): Promise<UpdateInfo | null> {
  try {
    const res = await fetch(LATEST_RELEASE_API, {
      headers: { Accept: "application/vnd.github+json" },
    });
    if (!res.ok) return null;
    const data = (await res.json()) as {
      tag_name?: string;
      html_url?: string;
    };
    const tag = data.tag_name;
    if (!tag || !isNewer(tag, currentVersion)) return null;
    return { latest: tag.replace(/^v/i, ""), url: data.html_url ?? RELEASES_PAGE };
  } catch {
    return null;
  }
}

export { RELEASES_PAGE };
