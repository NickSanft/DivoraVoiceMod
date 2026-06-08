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
    setPtm: vi.fn(),
    toggleMonitor: vi.fn(),
    setParam: vi.fn(),
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

  it("holds PTM on note-on and releases on note-off", () => {
    const h = mkHandlers();
    const mappings: MidiMapping[] = [
      { id: "p", action: "ptm", trigger: { kind: "note", data1: 36, channel: 0 } },
    ];
    routeMidi(noteOn(36), mappings, h);
    routeMidi(noteOff(36), mappings, h);
    expect(h.setPtm.mock.calls).toEqual([[true], [false]]);
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
