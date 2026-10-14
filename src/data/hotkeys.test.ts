import { describe, expect, it } from "vitest";
import {
  acceleratorToKeys,
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
    const none = { ptm: "", panic: "", monitor: "", "rec:screen": "", "rec:wav": "" };
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
});
