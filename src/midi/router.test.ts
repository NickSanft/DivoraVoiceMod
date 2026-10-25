import { describe, expect, it, vi } from "vitest";
import { matchesTrigger, routeMidi, scaleCc } from "./router";
import type { MidiMapping, MidiMessage } from "../types";

const noteOn = (n: number, vel = 100, ch = 0): MidiMessage => ({
  channel: ch,
  kind: "note-on",
  data1: n,
  data2: vel,
});
const noteOff = (n: number, ch = 0): MidiMessage => ({
  channel: ch,
  kind: "note-off",
  data1: n,
  data2: 0,
});
const cc = (controller: number, value: number, ch = 0): MidiMessage => ({
  channel: ch,
  kind: "control-change",
  data1: controller,
  data2: value,
});

function mkHandlers() {
  return {
    usePreset: vi.fn(),
    playTile: vi.fn(),
    ptmNote: vi.fn(),
    ptmCc: vi.fn(),
    toggleMonitor: vi.fn(),
    setParam: vi.fn(),
    toggleMuted: vi.fn(),
    muteCc: vi.fn(),
  };
}

describe("midi router", () => {
  it("scaleCc maps 0..127 onto [min, max] and clamps", () => {
    expect(scaleCc(0, -12, 12)).toBe(-12);
    expect(scaleCc(127, -12, 12)).toBe(12);
    expect(scaleCc(64, 0, 100)).toBeCloseTo(50.4, 1);
    expect(scaleCc(200, 0, 1)).toBe(1);
    expect(scaleCc(-5, 0, 1)).toBe(0);
  });

  it("matchesTrigger separates note from CC and is channel-agnostic", () => {
    expect(matchesTrigger(noteOn(60, 100, 3), { kind: "note", data1: 60, channel: 0 })).toBe(true);
    expect(matchesTrigger(cc(60, 100), { kind: "note", data1: 60, channel: 0 })).toBe(false);
    expect(matchesTrigger(cc(7, 100), { kind: "cc", data1: 7, channel: 0 })).toBe(true);
    expect(matchesTrigger(noteOn(61), { kind: "note", data1: 60, channel: 0 })).toBe(false);
  });

  it("summons a preset on a matching note-on, ignoring note-off", () => {
    const h = mkHandlers();
    const mappings: MidiMapping[] = [
      { id: "1", action: "preset", presetId: "velvet-demon", trigger: { kind: "note", data1: 60, channel: 0 } },
    ];
    routeMidi(noteOn(60), mappings, h);
    routeMidi(noteOff(60), mappings, h);
    expect(h.usePreset).toHaveBeenCalledTimes(1);
    expect(h.usePreset).toHaveBeenCalledWith("velvet-demon");
  });

  // v1.58.0: each `ptm` mapping is its own hold, by id, so the store can hold
  // push-to-modulate until every pad is up.
  it("holds PTM on note-on and releases on note-off, with the mapping's id", () => {
    const h = mkHandlers();
    const mappings: MidiMapping[] = [
      { id: "p", action: "ptm", trigger: { kind: "note", data1: 36, channel: 0 } },
      { id: "q", action: "ptm", trigger: { kind: "note", data1: 37, channel: 0 } },
    ];
    routeMidi(noteOn(36), mappings, h);
    routeMidi(noteOn(37), mappings, h);
    routeMidi(noteOff(36), mappings, h);
    routeMidi(noteOff(37), mappings, h);
    expect(h.ptmNote.mock.calls).toEqual([
      ["p", true],
      ["q", true],
      ["p", false],
      ["q", false],
    ]);
    expect(h.ptmCc).not.toHaveBeenCalled();
  });

  it("PTM: a CC passes the mapping's id and every value (the store keeps the dead band)", () => {
    const h = mkHandlers();
    const mappings: MidiMapping[] = [
      { id: "p-cc", action: "ptm", trigger: { kind: "cc", data1: 22, channel: 0 } },
    ];
    for (const v of [0, 63, 64, 62, 59, 127]) routeMidi(cc(22, v), mappings, h);
    expect(h.ptmCc.mock.calls).toEqual([
      ["p-cc", 0],
      ["p-cc", 63],
      ["p-cc", 64],
      ["p-cc", 62],
      ["p-cc", 59],
      ["p-cc", 127],
    ]);
    expect(h.ptmNote).not.toHaveBeenCalled();
    // Nothing else is driven by a ptm mapping.
    expect(h.toggleMuted).not.toHaveBeenCalled();
    expect(h.muteCc).not.toHaveBeenCalled();
  });

  it("PTM: no trigger, another control, or a message it doesn't bind drives nothing", () => {
    const h = mkHandlers();
    const mappings: MidiMapping[] = [
      { id: "none", action: "ptm", trigger: null },
      { id: "pad", action: "ptm", trigger: { kind: "note", data1: 36, channel: 0 } },
    ];
    routeMidi(noteOn(37), mappings, h);
    routeMidi(cc(36, 127), mappings, h); // CC 36 is not note 36
    routeMidi({ channel: 0, kind: "pitch-bend", data1: 36, data2: 64 }, mappings, h);
    expect(h.ptmNote).not.toHaveBeenCalled();
    expect(h.ptmCc).not.toHaveBeenCalled();
  });

  it("sweeps a bound param from a CC across its full range", () => {
    const h = mkHandlers();
    const mappings: MidiMapping[] = [
      {
        id: "x",
        action: "param",
        effectIndex: 2,
        paramKey: "shift",
        min: -12,
        max: 12,
        trigger: { kind: "cc", data1: 7, channel: 0 },
      },
    ];
    routeMidi(cc(7, 127), mappings, h);
    expect(h.setParam).toHaveBeenCalledWith(2, "shift", 12);
    routeMidi(cc(7, 0), mappings, h);
    expect(h.setParam).toHaveBeenCalledWith(2, "shift", -12);
  });

  it("fires a soundboard tile on note-on", () => {
    const h = mkHandlers();
    const mappings: MidiMapping[] = [
      { id: "t", action: "tile", tileId: "clip-1", trigger: { kind: "note", data1: 40, channel: 0 } },
    ];
    routeMidi(noteOn(40), mappings, h);
    expect(h.playTile).toHaveBeenCalledWith("clip-1");
  });

  it("toggles monitor only on an activating CC (>= 64)", () => {
    const h = mkHandlers();
    const mappings: MidiMapping[] = [
      { id: "m", action: "monitor", trigger: { kind: "cc", data1: 20, channel: 0 } },
    ];
    routeMidi(cc(20, 127), mappings, h);
    routeMidi(cc(20, 0), mappings, h);
    expect(h.toggleMonitor).toHaveBeenCalledTimes(1);
  });

  // v1.56.0: the mute. A pad toggles; a CC hands every value to the store,
  // which acts only on crossings (see muteCcStep).
  it("mute: a note-on toggles, a note-off does nothing", () => {
    const h = mkHandlers();
    const mappings: MidiMapping[] = [
      { id: "mu", action: "mute", trigger: { kind: "note", data1: 48, channel: 0 } },
    ];
    routeMidi(noteOn(48), mappings, h);
    routeMidi(noteOff(48), mappings, h);
    routeMidi(noteOn(48, 1, 5), mappings, h);
    expect(h.toggleMuted).toHaveBeenCalledTimes(2);
    expect(h.muteCc).not.toHaveBeenCalled();
  });

  it("mute: a CC passes the mapping's id and every value, never toggles", () => {
    const h = mkHandlers();
    const mappings: MidiMapping[] = [
      { id: "mu-cc", action: "mute", trigger: { kind: "cc", data1: 21, channel: 0 } },
    ];
    for (const v of [0, 59, 62, 64, 127, 10]) routeMidi(cc(21, v), mappings, h);
    expect(h.muteCc.mock.calls).toEqual([
      ["mu-cc", 0],
      ["mu-cc", 59],
      ["mu-cc", 62],
      ["mu-cc", 64],
      ["mu-cc", 127],
      ["mu-cc", 10],
    ]);
    expect(h.toggleMuted).not.toHaveBeenCalled();
    // Nothing else is driven by a mute mapping.
    expect(h.ptmNote).not.toHaveBeenCalled();
    expect(h.ptmCc).not.toHaveBeenCalled();
    expect(h.toggleMonitor).not.toHaveBeenCalled();
  });

  it("mute: no trigger, or a trigger that doesn't match, does nothing", () => {
    const h = mkHandlers();
    const mappings: MidiMapping[] = [
      { id: "a", action: "mute", trigger: null },
      { id: "b", action: "mute", trigger: { kind: "note", data1: 48, channel: 0 } },
      { id: "c", action: "mute", trigger: { kind: "cc", data1: 21, channel: 0 } },
    ];
    routeMidi(noteOn(49), mappings, h);
    routeMidi(cc(48, 127), mappings, h); // CC 48 is not note 48
    routeMidi(noteOn(21), mappings, h); // note 21 is not CC 21
    expect(h.toggleMuted).not.toHaveBeenCalled();
    expect(h.muteCc).not.toHaveBeenCalled();
  });

  it("ignores unmapped messages and mappings without a learned trigger", () => {
    const h = mkHandlers();
    const mappings: MidiMapping[] = [
      { id: "1", action: "preset", presetId: "x", trigger: null },
      { id: "2", action: "tile", tileId: "y", trigger: { kind: "note", data1: 99, channel: 0 } },
    ];
    routeMidi(noteOn(1), mappings, h);
    expect(h.usePreset).not.toHaveBeenCalled();
    expect(h.playTile).not.toHaveBeenCalled();
  });
});
