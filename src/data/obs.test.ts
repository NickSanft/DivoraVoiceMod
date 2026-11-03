import { describe, expect, it } from "vitest";
import fixture from "./obs.fixture.json";
import {
  adoptObsStatus,
  FOLLOW_OBS,
  hostPrecheck,
  mergeObsSettings,
  OBS_COMMANDS,
  OBS_DEFAULT_HOST,
  OBS_DEFAULT_PORT,
  OBS_HOST_ERRORS,
  OBS_MAX_NAME_CHARS,
  OBS_MAX_PASSWORD_CHARS,
  OBS_MAX_UUID_CHARS,
  OBS_REASONS,
  OBS_RECORD_STATES,
  OBS_SKIP_OPS,
  OBS_SKIP_WHYS,
  OBS_SKIP_WHYS_BY_OP,
  OBS_STATES,
  OBS_STATUS_EVENT,
  OBS_STREAM_STATES,
  OBS_TEXT_SKIP_WHYS,
  OBS_TEXT_SOURCE_KINDS,
  obsConfig,
  readFollowObs,
  readObsInventory,
  readObsName,
  readObsRef,
  readObsSettings,
  readObsStatus,
  readObsUuid,
  type ObsStatus,
} from "./obs";

/** A scene's uuid (OBS's are 36 characters). */
const UUID = "5d3f1c9e-7a2b-4e6d-8f10-2c4b6a8e0d13";
const OTHER = "a7e2c94b-1f3d-4b8a-9c6e-0d5f7b2e4a91";

/** The sample with every nested shape: a missing source, both skips. */
const FULL = fixture.obsStatusSamples.find((r) => r.status.lastSkip !== null && r.status.textSkip !== null)!
  .status as unknown as ObsStatus;

describe("the wire is the fixture's (v1.61.0)", () => {
  it("names the six commands and the event as the fixture does", () => {
    expect({ commands: OBS_COMMANDS, events: { status: OBS_STATUS_EVENT } }).toEqual(fixture.obsWire);
  });

  it("has the fixture's words, in its order", () => {
    expect(OBS_STATES).toEqual(fixture.states);
    expect(OBS_REASONS).toEqual(fixture.reasons);
    expect(OBS_SKIP_OPS).toEqual(fixture.skipOps);
    expect(OBS_SKIP_WHYS).toEqual(fixture.skipWhys);
    expect(OBS_SKIP_WHYS_BY_OP).toEqual(fixture.skipWhysByOp);
    expect(OBS_TEXT_SKIP_WHYS).toEqual(fixture.textSkipWhys);
    expect(OBS_STREAM_STATES).toEqual(fixture.streamStates);
    expect(OBS_RECORD_STATES).toEqual(fixture.recordStates);
    expect(OBS_TEXT_SOURCE_KINDS).toEqual(fixture.textSourceKinds);
  });

  it("has the fixture's limits and Rust's words for an address it refuses", () => {
    expect({
      maxUuidChars: OBS_MAX_UUID_CHARS,
      maxSceneNameChars: OBS_MAX_NAME_CHARS,
      maxPasswordChars: OBS_MAX_PASSWORD_CHARS,
    }).toEqual({
      maxUuidChars: fixture.limits.maxUuidChars,
      maxSceneNameChars: fixture.limits.maxSceneNameChars,
      maxPasswordChars: fixture.limits.maxPasswordChars,
    });
    expect(OBS_HOST_ERRORS).toEqual(fixture.hostErrors);
  });

  it("sends exactly configKeys, and nothing else of divora.obs", () => {
    const s = readObsSettings({ enabled: true, host: "192.168.1.20", port: 4456, later: 1, breakScene: { uuid: UUID, name: "BRB" } });
    expect(obsConfig(s)).toEqual({ enabled: true, host: "192.168.1.20", port: 4456 });
    expect(Object.keys(obsConfig(s)).sort()).toEqual(fixture.configKeys);
  });
});

describe("readObsStatus", () => {
  // The seam with the backend's `ObsStatus`: `obs_status_samples_are_rusts`
  // builds these rows in Rust and compares its own serialization to them.
  it("reads every obsStatusSample as it is, with exactly statusKeys", () => {
    for (const row of fixture.obsStatusSamples) {
      const s = readObsStatus(row.status);
      expect(s, row.name).toEqual(row.status);
      expect(Object.keys(s!).sort(), row.name).toEqual(fixture.statusKeys);
      if (s!.lastSkip) expect(Object.keys(s!.lastSkip).sort(), row.name).toEqual(fixture.lastSkipKeys);
      if (s!.textSkip) expect(Object.keys(s!.textSkip).sort(), row.name).toEqual(fixture.textSkipKeys);
    }
  });

  it("the samples hold every state, every reason and every nested shape", () => {
    const statuses = fixture.obsStatusSamples.map((r) => r.status);
    for (const state of OBS_STATES) expect(statuses.some((s) => s.state === state), state).toBe(true);
    for (const reason of OBS_REASONS) expect(statuses.some((s) => s.reason === reason), reason).toBe(true);
    for (const op of OBS_SKIP_OPS) expect(statuses.some((s) => s.lastSkip?.op === op), op).toBe(true);
    for (const why of OBS_TEXT_SKIP_WHYS) expect(statuses.some((s) => s.textSkip?.why === why), why).toBe(true);
    expect(statuses.some((s) => s.missing.length > 0)).toBe(true);
  });

  const lastSkip = FULL.lastSkip!;
  it.each([
    ["not an object", null],
    ["a list", [FULL]],
    ["seq missing", { ...FULL, seq: undefined }],
    ["seq negative", { ...FULL, seq: -1 }],
    ["seq fractional", { ...FULL, seq: 1.5 }],
    ["a state it doesn't know", { ...FULL, state: "paused" }],
    ["host not a string", { ...FULL, host: 5 }],
    ["port negative", { ...FULL, port: -1 }],
    ["port past 65535", { ...FULL, port: 65_536 }],
    ["loopback a string", { ...FULL, loopback: "true" }],
    ["hasPassword missing", { ...FULL, hasPassword: undefined }],
    ["authRequired a string", { ...FULL, authRequired: "yes" }],
    ["obsVersion a number", { ...FULL, obsVersion: 30 }],
    ["websocketVersion missing", { ...FULL, websocketVersion: undefined }],
    ["stream a recording word", { ...FULL, stream: "recording" }],
    ["record a stream word", { ...FULL, record: "live" }],
    ["a reason it doesn't know", { ...FULL, reason: "busy" }],
    ["detail a number", { ...FULL, detail: 5 }],
    ["retryAtMs fractional", { ...FULL, retryAtMs: 1.5 }],
    ["missing not a list", { ...FULL, missing: OTHER }],
    ["missing holding a number", { ...FULL, missing: [OTHER, 5] }],
    ["lastSkip an op it doesn't know", { ...FULL, lastSkip: { ...lastSkip, op: "startStream" } }],
    ["lastSkip a why it doesn't know", { ...FULL, lastSkip: { ...lastSkip, why: "busy" } }],
    ["a Switch OBS back that was late (its op can't be)", { ...FULL, lastSkip: { ...lastSkip, op: "sceneBack", why: "late" } }],
    ["a Switch OBS back with a full queue", { ...FULL, lastSkip: { ...lastSkip, op: "sceneBack", why: "queueFull" } }],
    ["lastSkip without its timer", { ...FULL, lastSkip: { ...lastSkip, timer: undefined } }],
    ["lastSkip a name that isn't a string", { ...FULL, lastSkip: { ...lastSkip, name: 5 } }],
    ["lastSkip atMs fractional", { ...FULL, lastSkip: { ...lastSkip, atMs: 1.5 } }],
    ["textSkip a scene's why", { ...FULL, textSkip: { why: "notFound", atMs: 1 } }],
    ["textSkip without atMs", { ...FULL, textSkip: { why: "failed" } }],
    ["heldBack negative", { ...FULL, heldBack: -1 }],
    ["inventoryRev missing", { ...FULL, inventoryRev: undefined }],
  ])("%s → null", (_name, raw) => {
    expect(readObsStatus(raw)).toBeNull();
  });

  it("leaves out keys a later backend adds, at the top and in a skip", () => {
    const s = readObsStatus({ ...FULL, later: 1, lastSkip: { ...lastSkip, later: 2 }, textSkip: { ...FULL.textSkip!, x: 3 } });
    expect(s).toEqual(FULL);
  });

  it("copies missing, so the mirror never shares the payload's list", () => {
    const raw = { ...FULL, missing: [OTHER] };
    const s = readObsStatus(raw)!;
    raw.missing.push(UUID);
    expect(s.missing).toEqual([OTHER]);
  });
});

describe("adoptObsStatus", () => {
  const at = (seq: number): ObsStatus => ({ ...FULL, seq });
  it("keeps the larger seq: an answer that lands after a newer event doesn't win", () => {
    const held = at(7);
    expect(adoptObsStatus(null, held)).toBe(held);
    expect(adoptObsStatus(held, at(8))).toEqual(at(8));
    expect(adoptObsStatus(held, at(6))).toBe(held);
    expect(adoptObsStatus(held, at(7))).toBe(held);
    expect(adoptObsStatus(held, null)).toBe(held);
    expect(adoptObsStatus(null, null)).toBeNull();
  });
});

describe("readObsInventory", () => {
  const inv = {
    rev: 3,
    scenes: [
      { uuid: UUID, name: "BRB" },
      { uuid: OTHER, name: "Game" },
    ],
    textSources: [
      { uuid: OTHER, name: "Timer text", kind: "gdiplus", readsFromFile: false },
      { uuid: "c3d4e5f6-0000-4000-8000-000000000001", name: "Uptime", kind: "ft2", readsFromFile: true },
    ],
    programSceneUuid: OTHER,
    truncated: false,
  };

  it("reads Rust's answer as it is, with exactly the fixture's keys", () => {
    const read = readObsInventory(inv)!;
    expect(read).toEqual(inv);
    expect(Object.keys(read).sort()).toEqual(fixture.inventoryKeys);
    for (const s of read.scenes) expect(Object.keys(s).sort()).toEqual(fixture.sceneKeys);
    for (const s of read.textSources) expect(Object.keys(s).sort()).toEqual(fixture.textSourceKeys);
    expect(readObsInventory({ ...inv, programSceneUuid: null, truncated: true })).toMatchObject({
      programSceneUuid: null,
      truncated: true,
    });
  });

  it("leaves out a scene or a source it can't read, a repeated uuid, and other keys", () => {
    const read = readObsInventory({
      ...inv,
      later: 1,
      scenes: [
        { uuid: UUID, name: "BRB", sceneIndex: 1 },
        { uuid: UUID, name: "BRB again" },
        { uuid: "", name: "No uuid" },
        { uuid: "f".repeat(129), name: "Long uuid" },
        { uuid: OTHER, name: 5 },
        "Game",
      ],
      textSources: [
        { uuid: OTHER, name: "Timer text", kind: "gdiplus", readsFromFile: false, text: "05:00" },
        { uuid: UUID, name: "Browser", kind: "browser", readsFromFile: false },
        { uuid: UUID, name: "No flag", kind: "ft2" },
        { uuid: OTHER, name: "Twice", kind: "ft2", readsFromFile: false },
      ],
    });
    expect(read).toEqual({
      ...inv,
      scenes: [{ uuid: UUID, name: "BRB" }],
      textSources: [{ uuid: OTHER, name: "Timer text", kind: "gdiplus", readsFromFile: false }],
    });
  });

  it.each([
    ["not an object", null],
    ["rev negative", { ...inv, rev: -1 }],
    ["scenes not a list", { ...inv, scenes: {} }],
    ["textSources missing", { ...inv, textSources: undefined }],
    ["programSceneUuid a number", { ...inv, programSceneUuid: 5 }],
    ["truncated missing", { ...inv, truncated: undefined }],
  ])("%s → null", (_name, raw) => {
    expect(readObsInventory(raw)).toBeNull();
  });
});

describe("readObsSettings (D32)", () => {
  const OFF = { enabled: false, host: OBS_DEFAULT_HOST, port: OBS_DEFAULT_PORT };

  it("is OBS's own address, off, when divora.obs is missing or isn't an object", () => {
    expect(OFF).toEqual({ enabled: false, host: "127.0.0.1", port: 4455 });
    for (const raw of [null, undefined, "x", 5, [OFF]]) expect(readObsSettings(raw), String(raw)).toEqual(OFF);
  });

  it("reads a good value as it is, its unknown keys kept", () => {
    const raw = { enabled: true, host: "192.168.1.20", port: 4456, breakScene: { uuid: UUID, name: "BRB" }, later: [1] };
    expect(readObsSettings(raw)).toEqual(raw);
  });

  it("reads each field on its own: one bad field never loses the others", () => {
    const good = { enabled: true, host: "192.168.1.20", port: 4456 };
    expect(readObsSettings({ ...good, enabled: "yes" })).toEqual({ ...good, enabled: false });
    expect(readObsSettings({ ...good, enabled: 1 })).toEqual({ ...good, enabled: false });
    for (const port of [0, 70_000, 65_536, -1, 4455.5, "4456", null]) {
      expect(readObsSettings({ ...good, port }), String(port)).toEqual({ ...good, port: 4455 });
    }
    expect(readObsSettings({ ...good, port: 1 }).port).toBe(1);
    expect(readObsSettings({ ...good, port: 65_535 }).port).toBe(65_535);
    for (const host of [5, "", "   ", null]) {
      expect(readObsSettings({ ...good, host }), String(host)).toEqual({ ...good, host: "127.0.0.1" });
    }
    expect(readObsSettings({ ...good, host: " 192.168.1.20\u0085" }).host).toBe("192.168.1.20");
  });

  it("keeps a break scene only when its uuid and name can be stored", () => {
    const read = (breakScene: unknown) => readObsSettings({ enabled: true, breakScene }).breakScene;
    expect(read({ uuid: UUID, name: "BRB", later: 1 })).toEqual({ uuid: UUID, name: "BRB" });
    expect(read({ uuid: UUID, name: "" })).toEqual({ uuid: UUID, name: "" });
    expect(read({ uuid: "f".repeat(128), name: "a".repeat(256) })).toEqual({ uuid: "f".repeat(128), name: "a".repeat(256) });
    for (const bad of [
      UUID,
      null,
      { uuid: UUID },
      { name: "BRB" },
      { uuid: "", name: "BRB" },
      { uuid: "f".repeat(129), name: "BRB" },
      { uuid: UUID, name: "a".repeat(257) },
      { uuid: UUID, name: 5 },
      { uuid: `${UUID}\uD800`, name: "BRB" },
      { uuid: UUID, name: "BRB\uDC00" },
    ]) {
      expect(read(bad), JSON.stringify(bad)).toBeUndefined();
    }
    expect("breakScene" in readObsSettings({ breakScene: { uuid: "" } })).toBe(false);
  });
});

describe("mergeObsSettings", () => {
  const cur = readObsSettings({ enabled: true, host: "192.168.1.20", port: 4456, breakScene: { uuid: UUID, name: "BRB" }, later: 1 });

  it("changes only what the patch names, into the value it is given", () => {
    expect(mergeObsSettings(cur, { enabled: false })).toEqual({ ...cur, enabled: false });
    expect(mergeObsSettings(cur, { host: "127.0.0.1", port: 4455 })).toEqual({ ...cur, host: "127.0.0.1", port: 4455 });
    expect(mergeObsSettings(cur, {})).toEqual(cur);
    expect(cur).toEqual({ enabled: true, host: "192.168.1.20", port: 4456, breakScene: { uuid: UUID, name: "BRB" }, later: 1 });
  });

  it("sets a break scene, or removes it with null", () => {
    expect(mergeObsSettings(cur, { breakScene: { uuid: OTHER, name: "Be right back" } }).breakScene).toEqual({
      uuid: OTHER,
      name: "Be right back",
    });
    const without = mergeObsSettings(cur, { breakScene: null });
    expect("breakScene" in without).toBe(false);
    expect(without).toEqual({ enabled: true, host: "192.168.1.20", port: 4456, later: 1 });
  });
});

describe("hostPrecheck", () => {
  // The seam with Rust's `parse_host`: the page refuses only what can't be
  // an address and leaves the rest to Rust (obs.fixture.json `hostSamples`).
  it("passes on what the fixture's hostSamples say, trimmed", () => {
    for (const row of fixture.hostSamples) {
      const host = hostPrecheck(row.typed);
      expect(host !== null, JSON.stringify(row.typed)).toBe(row.precheck);
      if (host !== null) expect(host).toBe(row.typed.trim());
    }
  });

  it("trims as Rust trims, and refuses any space inside", () => {
    expect(hostPrecheck("\u0085127.0.0.1 ")).toBe("127.0.0.1");
    expect(hostPrecheck("127.0.0.1\t4455")).toBeNull();
    expect(hostPrecheck("http://192.168.1.20")).toBeNull();
    expect(hostPrecheck(" \t ")).toBeNull();
  });
});

describe("the small readers", () => {
  it("readObsUuid: 1-128 characters, no lone surrogate", () => {
    expect(readObsUuid(UUID)).toBe(UUID);
    expect(readObsUuid("f".repeat(128))).toBe("f".repeat(128));
    for (const bad of ["", "f".repeat(129), `${UUID}\uD800`, 5, null, undefined]) expect(readObsUuid(bad)).toBeNull();
  });

  it("readObsName: a string of at most 256 characters, counted by code point; empty allowed", () => {
    expect(readObsName("")).toBe("");
    expect(readObsName("a".repeat(256))).toBe("a".repeat(256));
    expect(readObsName("\u{1F642}".repeat(256))).toBe("\u{1F642}".repeat(256));
    for (const bad of ["a".repeat(257), "BRB\uD800", 5, null]) expect(readObsName(bad)).toBeNull();
  });

  it("readObsRef keeps uuid and name only", () => {
    expect(readObsRef({ uuid: UUID, name: "BRB", later: 1 })).toEqual({ uuid: UUID, name: "BRB" });
    expect(readObsRef({ uuid: UUID })).toBeNull();
    expect(readObsRef([UUID, "BRB"])).toBeNull();
  });

  it("readFollowObs: stream or record, spelled exactly", () => {
    expect(FOLLOW_OBS).toEqual(["stream", "record"]);
    expect(readFollowObs("stream")).toBe("stream");
    expect(readFollowObs("record")).toBe("record");
    for (const bad of ["Stream", "recording", "", true, null, undefined]) expect(readFollowObs(bad)).toBeNull();
  });
});
