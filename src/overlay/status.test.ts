import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const invokeMock = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
}));

import {
  ALIVE_CMD,
  closedStatus,
  HANDSHAKE_CMD,
  NO_READY_GLOBAL,
  readOverlayStatus,
  requestLock,
  sendAlive,
  sendHandshake,
  SET_OVERLAY_MODE_CMD,
  type OverlayStatus,
} from "./status";
import fixture from "./wire.fixture.json";

// wire.fixture.json is the overlay's wire contract, read by the backend's
// tests too (overlay_machine.rs `wire_fixture_matches_rust`): a change on
// either side fails the other side's suite.
describe("overlay status (v1.55.0)", () => {
  it("closedStatus is the backend's closed status", () => {
    expect(closedStatus()).toEqual(fixture.statusSamples.closed);
    expect(closedStatus().hideAvailable).toBe(false);
  });

  it("has exactly the frozen keys", () => {
    expect(Object.keys(closedStatus()).sort()).toEqual(fixture.statusKeys);
    for (const sample of Object.values(fixture.statusSamples)) {
      expect(Object.keys(sample).sort()).toEqual(fixture.statusKeys);
    }
  });

  it("names the commands the overlay page calls", () => {
    expect({
      setMode: SET_OVERLAY_MODE_CMD,
      handshake: HANDSHAKE_CMD,
      alive: ALIVE_CMD,
    }).toEqual(fixture.commands);
  });

  it("names the debug no-ready switch as the backend sets it", () => {
    expect(NO_READY_GLOBAL).toBe(fixture.debug.noReadyGlobal);
  });
});

describe("readOverlayStatus", () => {
  it("reads every sample the backend's tests produce", () => {
    for (const [name, sample] of Object.entries(fixture.statusSamples)) {
      expect(readOverlayStatus(sample), name).toEqual(sample);
    }
    expect(readOverlayStatus(closedStatus())).toEqual(closedStatus());
  });

  it("each key missing or of the wrong type → null", () => {
    const wrong: Record<keyof OverlayStatus, unknown[]> = {
      open: [1, "true", null],
      phase: ["arranging", "", 3, null],
      instance: [-1, 1.5, "1", Number.NaN, null],
      mode: ["toggle", "Locked", null],
      arrangeGen: [-1, 0.5, "1", null],
      canArrange: [0, null],
      fit: ["false", null],
      hideWanted: [1, null],
      hideAvailable: [undefined, null],
      capturing: ["yes", null],
      error: [undefined, 0, false],
    };
    const base = fixture.statusSamples.arranging;
    for (const key of fixture.statusKeys as Array<keyof OverlayStatus>) {
      const missing: Record<string, unknown> = { ...base };
      delete missing[key];
      expect(readOverlayStatus(missing), `no ${key}`).toBeNull();
      for (const bad of wrong[key]) {
        expect(readOverlayStatus({ ...base, [key]: bad }), `${key} = ${String(bad)}`).toBeNull();
      }
    }
  });

  it("a key a later backend adds is ignored, not required", () => {
    const later = { ...fixture.statusSamples.arranging, frame: { x: 0, y: 0 } };
    expect(readOverlayStatus(later)).toEqual(fixture.statusSamples.arranging);
  });

  it("not a status at all → null (the e2e mock answers null)", () => {
    for (const raw of [null, undefined, "closed", 3, []]) expect(readOverlayStatus(raw)).toBeNull();
  });
});

describe("the overlay page's commands", () => {
  let warn: ReturnType<typeof vi.spyOn>;

  beforeEach(() => {
    invokeMock.mockReset();
    warn = vi.spyOn(console, "warn").mockImplementation(() => {});
  });

  afterEach(() => {
    warn.mockRestore();
    delete (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__;
  });

  it("requestLock asks for locked", async () => {
    invokeMock.mockResolvedValue(closedStatus());
    await requestLock();
    expect(invokeMock).toHaveBeenCalledWith("set_overlay_mode", { mode: "locked" });
    expect(warn).not.toHaveBeenCalled();
  });

  it("requestLock logs a refusal and never throws", async () => {
    invokeMock.mockRejectedValue(new Error("not allowed by ACL"));
    await expect(requestLock()).resolves.toBeUndefined();
    expect(warn).toHaveBeenCalledWith("overlay: set_overlay_mode(locked) failed", expect.any(Error));
  });

  it("sendHandshake sends the instance and stage once when it works", async () => {
    invokeMock.mockResolvedValue(closedStatus());
    await sendHandshake(3, "ready");
    expect(invokeMock.mock.calls).toEqual([["overlay_handshake", { instance: 3, stage: "ready" }]]);
  });

  it("sendHandshake retries once, logging each failure, and never throws", async () => {
    invokeMock.mockRejectedValueOnce(new Error("busy")).mockResolvedValueOnce(closedStatus());
    await sendHandshake(3, "painted");
    expect(invokeMock).toHaveBeenCalledTimes(2);
    expect(warn).toHaveBeenCalledTimes(1);

    invokeMock.mockReset();
    warn.mockClear();
    invokeMock.mockRejectedValue(new Error("gone"));
    await expect(sendHandshake(3, "painted")).resolves.toBeUndefined();
    expect(invokeMock).toHaveBeenCalledTimes(2);
    expect(warn).toHaveBeenCalledTimes(2);
  });

  it("sendAlive reports whether the beat went", async () => {
    invokeMock.mockResolvedValue(null);
    await expect(sendAlive(3, 1)).resolves.toBe(true);
    expect(invokeMock).toHaveBeenCalledWith("overlay_alive", { instance: 3, gen: 1 });
    invokeMock.mockRejectedValue(new Error("no"));
    await expect(sendAlive(3, 1)).resolves.toBe(false);
  });

  it("a failing beat is logged once per arrange generation, and only under Tauri", async () => {
    invokeMock.mockRejectedValue(new Error("no"));
    // The browser preview: quiet.
    await sendAlive(7, 1);
    expect(warn).not.toHaveBeenCalled();
    (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {};
    await sendAlive(7, 2);
    await sendAlive(7, 2);
    await sendAlive(7, 2);
    expect(warn).toHaveBeenCalledTimes(1);
    await sendAlive(7, 3);
    expect(warn).toHaveBeenCalledTimes(2);
  });
});
