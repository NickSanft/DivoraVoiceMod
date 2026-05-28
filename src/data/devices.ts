// Mock device lists for Phase 1 UI. Real device enumeration via cpal
// arrives in Phase 2 (`list_devices` Tauri command).

import type { DeviceOption } from "../types";

export const DEVICES_IN: DeviceOption[] = [
  { value: "blue-yeti", label: "Blue Yeti X", sub: "USB · 48 kHz · default" },
  { value: "scarlett", label: "Focusrite Scarlett 2i2", sub: "USB interface" },
  { value: "realtek", label: "Realtek HD Audio", sub: "Internal · line-in" },
  { value: "headset", label: "HyperX Cloud II", sub: "Headset mic" },
];

export const DEVICES_OUT: DeviceOption[] = [
  { value: "vb-cable", label: "CABLE Input (VB-Audio)", sub: "Virtual · route to apps" },
  { value: "headphones", label: "HyperX Cloud II", sub: "Headphones" },
  { value: "speakers", label: "Realtek Speakers", sub: "Stereo" },
];
