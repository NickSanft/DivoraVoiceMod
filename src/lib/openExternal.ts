// Open a URL in the user's default browser. Lazy-loads the Tauri shell
// plugin so callers still render in a browser preview that doesn't have
// the Tauri bridge.
//
// Extracted from SettingsScreen in v1.49.0 when the What's new panel
// needed the same behaviour for its "full changelog" link.
//
// v1.57.0: the backend's `open_external` first, which has the Windows
// desktop open it. The shell plugin's `open` runs ShellExecute in Divora's
// own process, so a browser that wasn't running started as Divora's child,
// and "Everything except Divora" (which leaves out Divora's whole process
// tree) left its sound out of recordings until it was restarted. Only when
// that fails (no desktop shell, a browser preview) does it fall back to the
// plugin, and then to a plain anchor; both are for links (the plugin refuses
// a folder path), and the backend already falls back for folders.
import { openExternalTarget } from "../audio/api";

export async function openExternal(url: string): Promise<void> {
  try {
    await openExternalTarget(url);
    return;
  } catch (err) {
    console.warn("[shell] opening it from the desktop failed", err);
  }
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
