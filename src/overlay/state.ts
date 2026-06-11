// v1.16.0: shared contract between the main window (emitter) and the
// stream-overlay window (consumer). The overlay is a separate Tauri
// WebviewWindow with its own JS context, so it can't read the store — the
// main window pushes the slice it needs over the Tauri event bus.

import type { ChainEntry, TweaksState, VoiceStatus } from "../types";

/** The Tauri event the main window emits and the overlay listens for. */
export const OVERLAY_EVENT = "overlay:state";

/** Background mode for the overlay window's content. The window itself is
 *  always created transparent; this controls what sits behind the circle:
 *  nothing (alpha, for OBS Window Capture with transparency) or a solid
 *  chroma colour the streamer keys out. */
export type OverlayBg = "transparent" | "green" | "magenta";

/** CSS colour for each chroma mode (keyed-out in OBS). */
export const OVERLAY_BG_COLOR: Record<OverlayBg, string> = {
  transparent: "transparent",
  green: "#00b140", // standard chroma green
  magenta: "#ff00ff",
};

/** Everything the overlay needs to mirror the Mixer's spell circle. */
export interface OverlayState {
  chain: ChainEntry[];
  status: VoiceStatus;
  motion: number;
  mystical: number;
  mood: TweaksState["mood"];
  accent: TweaksState["accent"];
  theme: TweaksState["theme"];
  bg: OverlayBg;
}

/** Bundle the live store values into an overlay payload (pure, testable). */
export function overlayPayload(input: {
  chain: ChainEntry[];
  status: VoiceStatus;
  motion: number;
  mystical: number;
  mood: TweaksState["mood"];
  accent: TweaksState["accent"];
  theme: TweaksState["theme"];
  bg: OverlayBg;
}): OverlayState {
  return {
    chain: input.chain,
    status: input.status,
    motion: input.motion,
    mystical: input.mystical,
    mood: input.mood,
    accent: input.accent,
    theme: input.theme,
    bg: input.bg,
  };
}
