// Open a URL in the user's default browser. Lazy-loads the Tauri shell
// plugin so callers still render in a browser preview that doesn't have
// the Tauri bridge.
//
// Extracted from SettingsScreen in v1.49.0 when the What's new panel
// needed the same behaviour for its "full changelog" link.
export async function openExternal(url: string): Promise<void> {
  try {
    const { open } = await import("@tauri-apps/plugin-shell");
    await open(url);
  } catch (err) {
    console.warn("[shell] external open failed", err);
    // Fall back to a normal anchor in case we're running in a browser
    // preview without the Tauri shell plugin.
    if (typeof window !== "undefined") window.open(url, "_blank", "noopener");
  }
}
