import { describe, expect, it } from "vitest";
import {
  acceleratorToKeys,
  HOTKEY_ACTIONS,
  keysToAccelerator,
  normalizeAccelerator,
  readHotkeyBindings,
} from "./hotkeys";

describe("keysToAccelerator", () => {
  it("joins chips, turning the Windows key into the name the parser accepts", () => {
    expect(keysToAccelerator(["Ctrl", "Win", "P"])).toBe("Ctrl+Super+P");
    expect(keysToAccelerator(["Win", "F9"])).toBe("Super+F9");
    expect(keysToAccelerator(["meta", "1"])).toBe("Super+1");
    expect(keysToAccelerator(["Ctrl", "Shift", "P"])).toBe("Ctrl+Shift+P");
  });

  it("empty or blank chips mean unbound", () => {
    expect(keysToAccelerator([])).toBe("");
    expect(keysToAccelerator([" ", ""])).toBe("");
  });

  it("does not touch a key merely containing 'win'", () => {
    // Only whole tokens are aliases; nothing here is the Windows key.
    expect(keysToAccelerator(["Ctrl", "W"])).toBe("Ctrl+W");
  });
});

describe("acceleratorToKeys", () => {
  it("shows Super (and its aliases) as Win", () => {
    expect(acceleratorToKeys("Ctrl+Super+P")).toEqual(["Ctrl", "Win", "P"]);
    expect(acceleratorToKeys("Cmd+K")).toEqual(["Win", "K"]);
    expect(acceleratorToKeys("Command+K")).toEqual(["Win", "K"]);
    expect(acceleratorToKeys("")).toEqual([]);
    expect(acceleratorToKeys("Space")).toEqual(["Space"]);
  });

  it("round-trips with keysToAccelerator", () => {
    for (const acc of ["Ctrl+Super+P", "Space", "Alt+Shift+F12", "Super+1"]) {
      expect(keysToAccelerator(acceleratorToKeys(acc))).toBe(acc);
    }
  });
});

describe("normalizeAccelerator", () => {
  it("fixes an accelerator saved with Win, and is idempotent", () => {
    expect(normalizeAccelerator("Ctrl+Win+M")).toBe("Ctrl+Super+M");
    expect(normalizeAccelerator(normalizeAccelerator("Ctrl+Win+M"))).toBe("Ctrl+Super+M");
    expect(normalizeAccelerator("Ctrl+Shift+P")).toBe("Ctrl+Shift+P");
    expect(normalizeAccelerator("")).toBe("");
  });

  it("never collapses a '+' key into a bare modifier", () => {
    // Shift+= types "+": the chips ["Shift", "+"] join to "Shift++". Dropping
    // the empty tokens would store "Shift", and the in-app push-to-modulate
    // listener matches the last token, so plain Shift would trigger it.
    expect(normalizeAccelerator("Shift++")).toBe("Shift++");
    expect(normalizeAccelerator("+")).toBe("+");
    expect(normalizeAccelerator("Win++")).toBe("Super++");
  });
});

describe("readHotkeyBindings", () => {
  it("nothing stored: every action unbound", () => {
    const none = {
      ptm: "",
      panic: "",
      monitor: "",
      "rec:screen": "",
      "rec:wav": "",
      "replay:save": "",
      "replay:toggle": "",
      "overlay:arrange": "",
      mute: "",
    };
    expect(readHotkeyBindings(null)).toEqual(none);
    expect(readHotkeyBindings("junk")).toEqual(none);
    expect(readHotkeyBindings([1, 2])).toEqual(none);
  });

  it("reads the known actions, ignores the rest, normalizes Win", () => {
    expect(
      readHotkeyBindings({ ptm: "Ctrl+Win+Space", panic: 7, monitor: "F8", "sb:x": "F1", future: "F2" }),
    ).toEqual({
      ptm: "Ctrl+Super+Space",
      panic: "",
      monitor: "F8",
      "rec:screen": "",
      "rec:wav": "",
      "replay:save": "",
      "replay:toggle": "",
      "overlay:arrange": "",
      mute: "",
    });
  });

  // v1.52.0: a v1.51.4 save has no recorder keys; they read as unbound, and
  // the recorders' own bindings round-trip.
  it("reads the recorder hotkeys, and older saves without them", () => {
    expect(readHotkeyBindings({ ptm: "F9" })).toMatchObject({
      ptm: "F9",
      "rec:screen": "",
      "rec:wav": "",
    });
    expect(
      readHotkeyBindings({ "rec:screen": "Ctrl+Alt+Win+F9", "rec:wav": "Ctrl+Alt+F10" }),
    ).toMatchObject({ "rec:screen": "Ctrl+Alt+Super+F9", "rec:wav": "Ctrl+Alt+F10" });
  });

  // v1.54.0: Replay's ids live under their own `replay:` prefix — `rec:`
  // means "start / stop a recorder", and a save is neither.
  it("reads Replay's hotkeys, turning Win into Super", () => {
    expect(
      readHotkeyBindings({ "replay:save": "Ctrl+Win+F11", "replay:toggle": "Ctrl+Alt+F12" }),
    ).toMatchObject({ "replay:save": "Ctrl+Super+F11", "replay:toggle": "Ctrl+Alt+F12" });
    // A v1.53.0 save has neither key: both read as unbound.
    expect(readHotkeyBindings({ "rec:screen": "F9" })).toMatchObject({
      "replay:save": "",
      "replay:toggle": "",
    });
  });

  // v1.55.0: a v1.54.0 save has no overlay key; it reads as unbound, and a
  // binding round-trips like any other.
  it("older save reads overlay:arrange as unbound", () => {
    const read = readHotkeyBindings({ ptm: "F9", "replay:save": "F11" });
    expect(read["overlay:arrange"]).toBe("");
    expect(read).toMatchObject({ ptm: "F9", "replay:save": "F11" });
    expect(readHotkeyBindings({ "overlay:arrange": "Ctrl+Win+O" })["overlay:arrange"]).toBe("Ctrl+Super+O");
  });

  // v1.56.0: a v1.55.0 save has no mute key; it reads as unbound, and a
  // binding round-trips like any other.
  it("older save reads mute as unbound, and a stored mute is read", () => {
    expect(readHotkeyBindings({ ptm: "F9" }).mute).toBe("");
    expect(readHotkeyBindings({ mute: "Ctrl+Win+F20" }).mute).toBe("Ctrl+Super+F20");
  });

  it("ignores a stored rec:replay key like any unknown key (it never shipped)", () => {
    const read = readHotkeyBindings({ "rec:replay": "F7", "rec:replay-arm": "F8" });
    expect(read).not.toHaveProperty("rec:replay");
    expect(read).not.toHaveProperty("rec:replay-arm");
    expect(Object.values(read).every((v) => v === "")).toBe(true);
  });
});

describe("HOTKEY_ACTIONS", () => {
  it("lists Replay's two ids after the recorders, then the overlay's, then mute last (the order they register)", () => {
    expect(HOTKEY_ACTIONS).toEqual([
      "ptm",
      "panic",
      "monitor",
      "rec:screen",
      "rec:wav",
      "replay:save",
      "replay:toggle",
      "overlay:arrange",
      "mute",
    ]);
  });
});
