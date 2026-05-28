// Sample soundboard tiles for Phase 1 UI. Real folder scanning + audio
// decoding arrives in Phase 5.

import type { SoundboardTile } from "../types";

export const SOUNDBOARD: SoundboardTile[] = [
  { id: 1, label: "Thunder Crack", emoji: "⚡", color: "#E9B14C", key: ["F1"], dur: 3.2 },
  { id: 2, label: "Wraith Scream", emoji: "👻", color: "#58C6F2", key: ["F2"], dur: 2.1 },
  { id: 3, label: "Bell Toll", emoji: "🔔", color: "#A99FC4", key: ["F3"], dur: 5.6 },
  { id: 4, label: "Demon Laugh", emoji: "😈", color: "#F2567A", key: ["F4"], dur: 4.0 },
  { id: 5, label: "Rune Hum", emoji: "🜂", color: "#7C5CF6", key: null, dur: 8.0 },
  { id: 6, label: "Glass Shatter", emoji: "🔮", color: "#34D9A0", key: ["F6"], dur: 1.4 },
  { id: 7, label: "Owl Call", emoji: "🦉", color: "#E9B14C", key: null, dur: 2.8 },
  { id: 8, label: "Sub Drop", emoji: "🜄", color: "#6D5BF0", key: ["F8"], dur: 3.6 },
  { id: 9, label: "Whisper Gust", emoji: "🌬️", color: "#58C6F2", key: null, dur: 6.2 },
  { id: 10, label: "Coin Clink", emoji: "🪙", color: "#E9B14C", key: ["F10"], dur: 0.9 },
  { id: 11, label: "Portal Open", emoji: "🌀", color: "#7C5CF6", key: null, dur: 4.4 },
  { id: 12, label: "Heartbeat", emoji: "🫀", color: "#F2567A", key: ["F12"], dur: 7.0 },
];
