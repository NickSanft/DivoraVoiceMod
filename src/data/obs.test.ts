import { describe, expect, it } from "vitest";
import fixture from "./obs.fixture.json";
import {
  adoptObsStatus,
  cutObsName,
  FOLLOW_OBS,
  hostPrecheck,
  isObsStopState,
  mergeObsSettings,
  OBS_COMMANDS,
  OBS_DEFAULT_HOST,
  OBS_DEFAULT_PORT,
  OBS_HOST_ERRORS,
  OBS_MAX_NAME_CHARS,
  OBS_MAX_PASSWORD_CHARS,
  OBS_MAX_SCENES,
  OBS_MAX_TEXT_SOURCES,
  OBS_MAX_UUID_CHARS,
  OBS_NOT_CONNECTED,
  OBS_PORT_ERROR,
  OBS_REASONS,
  OBS_RECORD_STATES,
  OBS_SKIP_OPS,
  OBS_SKIP_WHYS,
  OBS_SKIP_WHYS_BY_OP,
  OBS_STATES,
  OBS_STATUS_EVENT,
  OBS_STOP_STATES,
  OBS_STREAM_STATES,
  OBS_TEXT_SKIP_WHYS,
  OBS_TEXT_SOURCE_KINDS,
  obsAddressText,
  obsAnnouncement,
  obsClockTime,
  obsConfig,
  obsHeldBackLine,
  obsPasswordSendable,
  obsPortOk,
  obsRef,
  obsRetryLabel,
  obsSceneName,
  obsSkipLine,
  obsStatusLine,
  obsTextSkipLine,
  obsWarnings,
  pickerShown,
  readFollowObs,
  readObsInventory,
  readObsName,
  readObsRef,
  readObsSettings,
  readObsStatus,
  readObsUuid,
  sceneOptions,
  textSourceOptions,
  type ObsInventory,
  type ObsLastSkip,
  type ObsRecordState,
  type ObsStatus,
  type ObsStreamState,
  type SkipWhy,
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
      maxScenes: OBS_MAX_SCENES,
      maxTextSources: OBS_MAX_TEXT_SOURCES,
    }).toEqual({
      maxUuidChars: fixture.limits.maxUuidChars,
      maxSceneNameChars: fixture.limits.maxSceneNameChars,
      maxPasswordChars: fixture.limits.maxPasswordChars,
      maxScenes: fixture.limits.maxScenes,
      maxTextSources: fixture.limits.maxTextSources,
    });
    expect(OBS_HOST_ERRORS).toEqual(fixture.hostErrors);
    // The page says Rust's port words itself, for a port it never sends.
    expect(OBS_PORT_ERROR).toBe(fixture.commandErrors.configure.badPort);
    expect(OBS_NOT_CONNECTED).toBe(fixture.commandErrors.inventory.notConnected);
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

// ---- the words (design §9) ----

/** A sample by its name in the fixture. */
const sample = (name: string): ObsStatus => {
  const row = fixture.obsStatusSamples.find((r) => r.name === name);
  if (!row) throw new Error(`no sample ${name}`);
  return row.status as unknown as ObsStatus;
};
/** The fixture's retry times are this plus 0.8-12 s. */
const NOW = 1_793_217_900_000;
const CAN_T = "Can't reach OBS at 127.0.0.1:4455.";
const HEAD = "Connected to OBS 30.2.2 (WebSocket 5.5.2)";

describe("obsStatusLine (§9.1)", () => {
  // Every sample's line at NOW, as §9.1 words it: each retry's "{Why}" a
  // whole sentence, its seconds rounded up.
  const LINES: Record<string, string> = {
    "off, never configured: ObsStatus::off()": "Off. Divora doesn't connect to OBS.",
    "off, configured with enabled:false, a password saved": "Off. Divora doesn't connect to OBS.",
    "connecting, the first attempt": "Connecting to 127.0.0.1:4455…",
    connected: HEAD,
    "connected, live and recording, a source missing, a scene skip and a text skip": `${HEAD} · live · recording`,
    "connected, OBS is closing (ExitStarted)": HEAD,
    "retrying, refused; a switch back ran out of time": `${CAN_T} Nothing is listening there. Is OBS open, with its WebSocket server on? Trying again in 1 s.`,
    "retrying, unreachable, OBS on another PC":
      "Can't reach OBS at 192.168.1.20:4455. No answer from that address. Trying again in 2 s.",
    "retrying, noAnswer": `${CAN_T} It didn't answer in time. Trying again in 4 s.`,
    "retrying, notObs": `${CAN_T} That isn't OBS's WebSocket server. Trying again in 7 s.`,
    "retrying, notReady": `${CAN_T} OBS stayed busy for 30 seconds (starting, closing or changing scene collection). Trying again in 12 s.`,
    "retrying, closed; a switch back held back": `${CAN_T} OBS closed the connection. Trying again in 1 s.`,
    "retrying, obsExiting": `${CAN_T} OBS is closing. Trying again in 1 s.`,
    "retrying, silent; a text push timed out": `${CAN_T} OBS stopped answering. Trying again in 1 s.`,
    "retrying, tooBig": `${CAN_T} OBS sent a message over 4 MiB, more than Divora takes. Trying again in 1 s.`,
    "retrying, protocol": `${CAN_T} OBS and Divora didn't understand each other. Trying again in 1 s.`,
    needsPassword:
      "OBS is asking for a password. Paste it from OBS's Show Connect Info, then Save password. Or turn off Enable Authentication in OBS and press Try again.",
    authFailed: "OBS rejected the password. Paste it again from OBS's Show Connect Info, then Save password.",
    passwordUnreadable:
      "Re-enter your OBS password. Windows couldn't read the saved one, which can happen after a change to your Windows account.",
    tooOld: "Update OBS, then press Try again. Its WebSocket server is 5.3.3, and Divora needs 5.4.0 or later.",
    needsNewerDivora: "This OBS needs a newer Divora. Check for updates in Settings → About.",
    kicked: "OBS disconnected Divora from its list of connections. Divora won't reconnect on its own.",
    "unsupported, badVersion": "Divora can't read this OBS's WebSocket version. Update OBS, then press Try again.",
    "unsupported, missingRequest":
      "This OBS's WebSocket server doesn't offer GetInputSettings, which Divora needs. Update OBS, then press Try again.",
  };
  const STREAM: Record<ObsStreamState, string> = {
    off: "",
    starting: " · stream starting",
    live: " · live",
    reconnecting: " · stream reconnecting",
    stopping: " · stream stopping",
  };
  const RECORD: Record<ObsRecordState, string> = {
    off: "",
    starting: " · recording starting",
    recording: " · recording",
    paused: " · recording paused",
    stopping: " · recording stopping",
  };

  it("says each sample's line, and has a line for every sample", () => {
    expect(Object.keys(LINES).sort()).toEqual(fixture.obsStatusSamples.map((r) => r.name).sort());
    for (const row of fixture.obsStatusSamples) {
      expect(obsStatusLine(row.status as unknown as ObsStatus, NOW), row.name).toBe(LINES[row.name]);
    }
  });

  it("every sample × every stream word × every record word: only connected says them, after its versions", () => {
    const streams: Array<ObsStreamState | null> = [null, ...OBS_STREAM_STATES];
    const records: Array<ObsRecordState | null> = [null, ...OBS_RECORD_STATES];
    for (const row of fixture.obsStatusSamples) {
      const base = row.status as unknown as ObsStatus;
      for (const stream of streams) {
        for (const record of records) {
          const line = obsStatusLine({ ...base, stream, record }, NOW);
          const want =
            base.state === "connected"
              ? `${HEAD}${stream === null ? "" : STREAM[stream]}${record === null ? "" : RECORD[record]}`
              : LINES[row.name];
          expect(line, `${row.name} ${stream} ${record}`).toBe(want);
        }
      }
    }
  });

  it("connected without one version or both", () => {
    const on = sample("connected");
    expect(obsStatusLine({ ...on, obsVersion: null }, NOW)).toBe("Connected to OBS (WebSocket 5.5.2)");
    expect(obsStatusLine({ ...on, obsVersion: null, websocketVersion: null }, NOW)).toBe("Connected to OBS");
    expect(obsStatusLine({ ...on, websocketVersion: null }, NOW)).toBe("Connected to OBS 30.2.2");
    expect(obsStatusLine({ ...on, obsVersion: null, websocketVersion: null, stream: "live" }, NOW)).toBe(
      "Connected to OBS · live",
    );
  });

  it("counts whole seconds up to the attempt, and says now at or past it", () => {
    const r = sample("retrying, closed; a switch back held back");
    const at = r.retryAtMs!;
    const tail = (now: number) => obsStatusLine(r, now).replace(`${CAN_T} OBS closed the connection. `, "");
    expect(tail(at - 30_000)).toBe("Trying again in 30 s.");
    expect(tail(at - 1001)).toBe("Trying again in 2 s.");
    expect(tail(at - 1000)).toBe("Trying again in 1 s.");
    expect(tail(at - 1)).toBe("Trying again in 1 s.");
    expect(tail(at)).toBe("Trying again now.");
    expect(tail(at + 5000)).toBe("Trying again now.");
    // No attempt time (not a status Rust writes): no promise of one.
    expect(obsStatusLine({ ...r, retryAtMs: null }, NOW)).toBe(`${CAN_T} OBS closed the connection.`);
  });

  it("brackets an IPv6 address, so its last group isn't read as the port", () => {
    expect(obsStatusLine({ ...sample("connecting, the first attempt"), host: "::1" }, NOW)).toBe(
      "Connecting to [::1]:4455…",
    );
    expect(obsAddressText("2001:db8::1", 4456)).toBe("[2001:db8::1]:4456");
    expect(obsAddressText("192.168.1.20", 4455)).toBe("192.168.1.20:4455");
  });

  it("tooOld without a version, and unsupported without a request's name, still say what to do", () => {
    expect(obsStatusLine({ ...sample("tooOld"), websocketVersion: null }, NOW)).toBe(
      "Update OBS, then press Try again. Divora needs its WebSocket server 5.4.0 or later.",
    );
    expect(obsStatusLine({ ...sample("unsupported, missingRequest"), detail: null }, NOW)).toBe(
      "This OBS's WebSocket server doesn't offer everything Divora needs. Update OBS, then press Try again.",
    );
  });

  it("offers Try again where pressing it can help, and Connect again after a Kick", () => {
    const want: Record<string, string | null> = {
      off: null,
      connecting: null,
      connected: null,
      retrying: "Try again",
      needsPassword: "Try again",
      authFailed: "Try again",
      passwordUnreadable: null,
      tooOld: "Try again",
      needsNewerDivora: null,
      kicked: "Connect again",
      unsupported: "Try again",
    };
    for (const state of OBS_STATES) expect(obsRetryLabel({ ...FULL, state }), state).toBe(want[state]);
  });

  it("the stop states are the ones that wait for a change", () => {
    expect(OBS_STOP_STATES).toEqual([
      "needsPassword",
      "authFailed",
      "passwordUnreadable",
      "tooOld",
      "needsNewerDivora",
      "kicked",
      "unsupported",
    ]);
    for (const state of ["off", "connecting", "connected", "retrying"] as const) {
      expect(isObsStopState(state)).toBe(false);
    }
  });
});

describe("obsWarnings (§9.1)", () => {
  const NETWORK =
    "192.168.1.20 is a network address, so Divora talks to OBS over the network, without encryption. Anyone who can see that traffic can read what Divora and OBS send each other, including your sources' names and settings and your recording's file path, and could change it or take over the connection to control OBS. Your password itself is never sent, but someone who records a connection's start can test guesses at it offline, so keep it long and random.";
  const NO_PASSWORD =
    "OBS isn't asking for a password, so anything that can reach it on your network can control it. Turn on Enable Authentication in OBS's WebSocket Server Settings.";
  const lan = sample("retrying, unreachable, OBS on another PC");

  it("warns of a network address and of OBS asking for no password, in that order", () => {
    expect(obsWarnings(sample("connected"))).toEqual([]);
    expect(obsWarnings(lan)).toEqual([NETWORK]);
    expect(obsWarnings({ ...sample("connected"), authRequired: false })).toEqual([NO_PASSWORD]);
    expect(obsWarnings({ ...lan, authRequired: false })).toEqual([NETWORK, NO_PASSWORD]);
    // Before any Hello, nobody knows.
    expect(obsWarnings({ ...lan, authRequired: null })).toEqual([NETWORK]);
  });

  it("says none while off, or before any address", () => {
    expect(obsWarnings({ ...lan, state: "off", authRequired: false })).toEqual([]);
    expect(obsWarnings({ ...lan, host: "", authRequired: false })).toEqual([]);
    expect(obsWarnings(sample("off, never configured: ObsStatus::off()"))).toEqual([]);
  });
});

describe("obsSkipLine and obsTextSkipLine (§9.1)", () => {
  const fmt = (ms: number) => (ms === 1_793_217_840_000 ? "14:04" : `?${ms}`);
  const skip = (op: "scene" | "sceneBack", why: SkipWhy, name: string | null = op === "scene" ? "BRB" : null) => ({
    ...FULL,
    lastSkip: { op, why, name, timer: "timer-1", atMs: 1_793_217_840_000 } as ObsLastSkip,
  });
  // Every op × why skipWhysByOp allows.
  const LINES: Record<"scene" | "sceneBack", Partial<Record<SkipWhy, string>>> = {
    scene: {
      notConnected: "14:04: Didn't switch OBS to BRB: Divora wasn't connected to OBS.",
      queueFull: "14:04: Didn't switch OBS to BRB: too many OBS requests were waiting.",
      notFound: "14:04: Didn't switch OBS to BRB: OBS doesn't have that scene now.",
      notReady: "14:04: Didn't switch OBS to BRB: OBS was busy (starting, closing or changing scene collection).",
      timedOut: "14:04: OBS didn't answer the switch to BRB in time. It may switch late, or not at all.",
      failed: "14:04: OBS refused the switch to BRB.",
      late: "14:04: Didn't switch OBS to BRB: OBS was busy for more than 5 seconds, and a late switch is worse than none.",
    },
    sceneBack: {
      notConnected:
        "14:04: Didn't switch OBS back: Divora wasn't connected to OBS within 2 minutes of the break's end.",
      notFound: "14:04: Didn't switch OBS back: OBS doesn't have that scene now.",
      notReady: "14:04: Didn't switch OBS back: OBS stayed busy for 2 minutes after the break's end.",
      timedOut: "14:04: OBS didn't answer the switch back in time. It may switch late, or not at all.",
      failed: "14:04: OBS refused the switch back.",
    },
  };

  it("says every op × why the fixture allows, with its time and its scene", () => {
    for (const op of OBS_SKIP_OPS) {
      expect(Object.keys(LINES[op]).sort(), op).toEqual([...fixture.skipWhysByOp[op]].sort());
      for (const why of fixture.skipWhysByOp[op] as SkipWhy[]) {
        expect(obsSkipLine(skip(op, why), fmt), `${op} ${why}`).toBe(LINES[op][why]);
      }
    }
  });

  it("a scene with no name reads as one, never as nothing", () => {
    expect(obsSkipLine(skip("scene", "failed", ""), fmt)).toBe("14:04: OBS refused the switch to a scene with no name.");
    expect(obsSkipLine(skip("scene", "failed", null), fmt)).toBe(
      "14:04: OBS refused the switch to a scene with no name.",
    );
    expect(obsSceneName("Just Chatting")).toBe("Just Chatting");
  });

  it("none without a skip", () => {
    expect(obsSkipLine(sample("connected"), fmt)).toBeNull();
    expect(obsTextSkipLine(sample("connected"), fmt)).toBeNull();
  });

  it("a text skip says OBS refused it or didn't take it in time, and that Divora tries again", () => {
    const text = (why: "failed" | "timedOut") => ({ ...FULL, textSkip: { why, atMs: 1_793_217_840_000 } });
    expect(obsTextSkipLine(text("failed"), fmt)).toBe(
      "14:04: OBS refused a timer's text. Divora tries again in a moment.",
    );
    expect(obsTextSkipLine(text("timedOut"), fmt)).toBe(
      "14:04: OBS didn't take a timer's text in time. Divora tries again in a moment.",
    );
  });

  it("the held-back line, only while one waits", () => {
    expect(obsHeldBackLine(sample("retrying, closed; a switch back held back"))).toBe(
      "Waiting for OBS, to switch it back after a break (up to 2 minutes).",
    );
    expect(obsHeldBackLine({ ...FULL, heldBack: 0 })).toBeNull();
  });

  it("obsClockTime gives the local hour and minute", () => {
    const at = new Date(2026, 10, 3, 14, 5, 59).getTime();
    expect(obsClockTime(at)).toMatch(/^(14|2):05\b/);
  });
});

describe("obsAnnouncement (§9.1: edges only)", () => {
  const fmt = () => "14:04";
  const connected = sample("connected");
  const retrying = sample("retrying, refused; a switch back ran out of time");
  const connecting = sample("connecting, the first attempt");
  const at = (s: ObsStatus, seq: number, more: Partial<ObsStatus> = {}): ObsStatus => ({ ...s, seq, ...more });
  const plain = (s: ObsStatus): ObsStatus => ({ ...s, lastSkip: null, heldBack: 0 });

  it("two retrying statuses that differ only in their attempt time and seq: nothing (never a countdown)", () => {
    const a = plain(retrying);
    expect(obsAnnouncement(a, at(a, a.seq + 1, { retryAtMs: a.retryAtMs! + 2000 }), fmt)).toBe("");
  });

  it("connecting: nothing", () => {
    const off = plain(sample("off, configured with enabled:false, a password saved"));
    expect(obsAnnouncement(off, connecting, fmt)).toBe("");
    expect(obsAnnouncement(plain(retrying), at(connecting, 99), fmt)).toBe("");
  });

  it("entering connected: once", () => {
    expect(obsAnnouncement(connecting, connected, fmt)).toBe("Connected to OBS.");
    expect(obsAnnouncement(connected, at(connected, 9, { stream: "live" }), fmt)).toBe("");
  });

  it("a drop says it once; the attempts after it say nothing", () => {
    const lost = plain(at(retrying, 10));
    expect(obsAnnouncement(connected, lost, fmt)).toBe("Lost the connection to OBS. Divora keeps trying.");
    const attempt = at(connecting, 11);
    expect(obsAnnouncement(lost, attempt, fmt)).toBe("");
    expect(obsAnnouncement(attempt, plain(at(retrying, 12)), fmt)).toBe("");
  });

  it("entering each stop state says its status line", () => {
    for (const state of OBS_STOP_STATES) {
      const row = fixture.obsStatusSamples.find((r) => r.status.state === state)!.status as unknown as ObsStatus;
      expect(obsAnnouncement(connecting, row, fmt), state).toBe(obsStatusLine(row, 0));
    }
    expect(obsAnnouncement(connecting, sample("authFailed"), fmt)).toBe(
      "OBS rejected the password. Paste it again from OBS's Show Connect Info, then Save password.",
    );
  });

  it("a new scene skip says its line; the same skip again says nothing", () => {
    const skipped: ObsLastSkip = { op: "scene", why: "notConnected", name: "BRB", timer: "timer-1", atMs: 5 };
    const a = at(connected, 7, { lastSkip: skipped });
    const line = "14:04: Didn't switch OBS to BRB: Divora wasn't connected to OBS.";
    expect(obsAnnouncement(connected, a, fmt)).toBe(line);
    expect(obsAnnouncement(a, at(a, 8, { stream: "live" }), fmt)).toBe("");
    expect(obsAnnouncement(a, at(a, 9, { lastSkip: { ...skipped, atMs: 6 } }), fmt)).toBe(line);
    // Cleared by a switch that worked: nothing to say.
    expect(obsAnnouncement(a, at(a, 10, { lastSkip: null }), fmt)).toBe("");
  });

  it("held-back switches going from none to some say the line once", () => {
    const none = plain(retrying);
    const one = at(none, none.seq + 1, { heldBack: 1 });
    expect(obsAnnouncement(none, one, fmt)).toBe("Waiting for OBS, to switch it back after a break (up to 2 minutes).");
    expect(obsAnnouncement(one, at(one, one.seq + 1, { heldBack: 2 }), fmt)).toBe("");
  });

  it("the first status seen, and turning it off, are not news; several edges at once are said in order", () => {
    expect(obsAnnouncement(null, connected, fmt)).toBe("");
    expect(obsAnnouncement(connected, sample("off, configured with enabled:false, a password saved"), fmt)).toBe("");
    const both = at(retrying, 30, { heldBack: 1 });
    expect(obsAnnouncement(connected, both, fmt)).toBe(
      "Lost the connection to OBS. Divora keeps trying. 14:04: Didn't switch OBS back: Divora wasn't connected to OBS within 2 minutes of the break's end. Waiting for OBS, to switch it back after a break (up to 2 minutes).",
    );
  });

  it("says a skip's time by the local clock without a formatter", () => {
    const skipped: ObsLastSkip = { op: "scene", why: "failed", name: "BRB", timer: "timer-1", atMs: 1_793_217_840_000 };
    expect(obsAnnouncement(connected, at(connected, 7, { lastSkip: skipped }))).toBe(
      `${obsClockTime(skipped.atMs)}: OBS refused the switch to BRB.`,
    );
  });
});

describe("pickerShown (§9.3's table)", () => {
  const SRC = "c3d4e5f6-0000-4000-8000-000000000001";
  const FILE_SRC = "c3d4e5f6-0000-4000-8000-000000000002";
  const GONE = "c3d4e5f6-0000-4000-8000-0000000000ff";
  const INV: ObsInventory = {
    rev: 1,
    scenes: [
      { uuid: UUID, name: "BRB" },
      { uuid: OTHER, name: "Game" },
    ],
    textSources: [
      { uuid: SRC, name: "Timer text", kind: "gdiplus", readsFromFile: false },
      { uuid: FILE_SRC, name: "Uptime", kind: "ft2", readsFromFile: true },
    ],
    programSceneUuid: OTHER,
    truncated: false,
  };
  const on = sample("connected");
  const p = (more: Partial<Parameters<typeof pickerShown>[0]> = {}) =>
    pickerShown({
      what: "textSource",
      stored: { uuid: SRC, name: "Old name" },
      enabled: true,
      status: on,
      inventory: INV,
      busy: false,
      error: null,
      ...more,
    });
  const shown = (text: string, sub: string | null = null, note: string | null = null) => ({ text, sub, note });

  it("OBS control off: the stored name, nothing more", () => {
    expect(p({ enabled: false, status: null, inventory: null })).toEqual(shown("Old name"));
    expect(p({ enabled: false, stored: null })).toEqual(shown("None"));
    expect(p({ enabled: false, stored: null, what: "scene" })).toEqual(shown("Don't switch"));
  });

  it("a text source OBS can't find on this connection", () => {
    const missing = { ...on, missing: [SRC] };
    expect(p({ status: missing })).toEqual(shown("Old name (not in OBS now)"));
    // `missing` is the text sources': a scene picker goes on to its list.
    expect(p({ what: "scene", stored: { uuid: UUID, name: "Old" }, status: { ...on, missing: [UUID] } })).toEqual(
      shown("BRB"),
    );
  });

  it("not connected", () => {
    expect(p({ status: sample("retrying, noAnswer") })).toEqual(shown("Old name (not connected)"));
    expect(p({ status: null })).toEqual(shown("Old name (not connected)"));
    expect(p({ status: sample("needsPassword"), stored: null })).toEqual(shown("None"));
  });

  it("connected, loading or with no inventory yet", () => {
    expect(p({ busy: true })).toEqual(shown("Old name", "Loading OBS sources…"));
    expect(p({ inventory: null })).toEqual(shown("Old name", "Loading OBS sources…"));
    expect(p({ what: "scene", stored: { uuid: UUID, name: "Old" }, busy: true, error: "x" })).toEqual(
      shown("Old", "Loading OBS scenes…"),
    );
  });

  it("the inventory failed: the stored name, and Rust's words under the picker", () => {
    expect(p({ inventory: null, error: "OBS is busy. Try again in a moment." })).toEqual(
      shown("Old name", null, "Couldn't read OBS's sources: OBS is busy. Try again in a moment."),
    );
    expect(p({ what: "scene", stored: null, inventory: null, error: "OBS isn't connected." })).toEqual(
      shown("Don't switch", null, "Couldn't read OBS's scenes: OBS isn't connected."),
    );
  });

  it("listed: the inventory's name; not listed: not in OBS now", () => {
    expect(p()).toEqual(shown("Timer text"));
    expect(p({ what: "scene", stored: { uuid: OTHER, name: "Old" } })).toEqual(shown("Game"));
    expect(p({ stored: { uuid: GONE, name: "Gone" } })).toEqual(shown("Gone (not in OBS now)"));
    expect(p({ what: "scene", stored: { uuid: GONE, name: "Gone" } })).toEqual(shown("Gone (not in OBS now)"));
    expect(p({ stored: null })).toEqual(shown("None"));
    expect(p({ what: "scene", stored: null })).toEqual(shown("Don't switch"));
  });

  it("no text sources: None, and how to add one", () => {
    const empty = { ...INV, textSources: [] };
    const note = "No text sources in OBS. Add a Text (GDI+) source there, then open this again.";
    expect(p({ stored: null, inventory: empty })).toEqual(shown("None", null, note));
    expect(p({ inventory: empty })).toEqual(shown("Old name (not in OBS now)", null, note));
    expect(p({ what: "scene", stored: { uuid: UUID, name: "Old" }, inventory: empty })).toEqual(shown("BRB"));
  });

  it("a list cut at its limit says so, and a uuid past the cut shows its stored name", () => {
    const id = (i: number) => `00000000-0000-4000-8000-${String(i).padStart(12, "0")}`;
    const scenes = Array.from({ length: OBS_MAX_SCENES }, (_, i) => ({ uuid: id(i), name: `Scene ${i}` }));
    const sources = Array.from({ length: OBS_MAX_TEXT_SOURCES }, (_, i) => ({
      uuid: id(i),
      name: `Text ${i}`,
      kind: "gdiplus" as const,
      readsFromFile: false,
    }));
    const scenesCut = { ...INV, scenes, truncated: true };
    expect(p({ what: "scene", stored: { uuid: GONE, name: "Gone" }, inventory: scenesCut })).toEqual(
      shown("Gone", null, "Divora lists OBS's first 200 scenes."),
    );
    // Only the scenes were cut: the text sources' list is whole.
    expect(p({ stored: { uuid: GONE, name: "Gone" }, inventory: scenesCut })).toEqual(shown("Gone (not in OBS now)"));
    const sourcesCut = { ...INV, textSources: sources, truncated: true };
    expect(p({ stored: { uuid: GONE, name: "Gone" }, inventory: sourcesCut })).toEqual(
      shown("Gone", null, "Divora lists OBS's first 100 text sources."),
    );
    expect(p({ stored: { uuid: id(3), name: "Old" }, inventory: sourcesCut })).toEqual(
      shown("Text 3", null, "Divora lists OBS's first 100 text sources."),
    );
  });

  it("a name with nothing in it shows as one", () => {
    expect(p({ stored: { uuid: GONE, name: "" }, status: null })).toEqual(shown("(no name) (not connected)"));
    expect(p({ inventory: { ...INV, textSources: [{ ...INV.textSources[0]!, name: "" }] } })).toEqual(
      shown("(no name)"),
    );
  });
});

describe("textSourceOptions and sceneOptions", () => {
  const src = (uuid: string, name: string, kind: "gdiplus" | "ft2", readsFromFile = false) => ({
    uuid,
    name,
    kind,
    readsFromFile,
  });
  const READS = "reads a file — untick Read from file in its Properties in OBS to show a timer here";
  const INV: ObsInventory = {
    rev: 2,
    scenes: [
      { uuid: UUID, name: "BRB" },
      { uuid: OTHER, name: "" },
    ],
    textSources: [
      src("s-1", "Countdown text", "gdiplus"),
      src("s-2", "Uptime text", "ft2"),
      src("s-3", "File text", "gdiplus", true),
      src("s-4", "Shared", "ft2"),
      src("s-5", "Both", "gdiplus", true),
    ],
    programSceneUuid: UUID,
    truncated: false,
  };

  it("None first, then each source in OBS's order with its kind; one that reads a file or that another timer shows can't be picked, and says why", () => {
    const others = new Map([
      ["s-4", "Uptime"],
      ["s-5", "Intro"],
    ]);
    expect(textSourceOptions(INV, others)).toEqual([
      { value: "", label: "None" },
      { value: "s-1", label: "Countdown text", sub: "Text (GDI+)" },
      { value: "s-2", label: "Uptime text", sub: "Text (FreeType 2)" },
      { value: "s-3", label: "File text", sub: READS, disabled: true },
      { value: "s-4", label: "Shared", sub: "shown by Uptime", disabled: true },
      // Reading a file is what to fix first.
      { value: "s-5", label: "Both", sub: READS, disabled: true },
    ]);
    expect(textSourceOptions({ ...INV, textSources: [] }, new Map())).toEqual([{ value: "", label: "None" }]);
  });

  it("scenes in OBS's order, by uuid", () => {
    expect(sceneOptions(INV)).toEqual([
      { value: UUID, label: "BRB" },
      { value: OTHER, label: "(no name)" },
    ]);
  });
});

describe("what the page stores of a pick, and sends", () => {
  it("cutObsName keeps a name's first 256 characters, counted by code point, never splitting one", () => {
    expect(cutObsName("BRB")).toBe("BRB");
    expect(cutObsName("a".repeat(256))).toBe("a".repeat(256));
    expect(cutObsName("a".repeat(300))).toBe("a".repeat(256));
    const emoji = "\u{1F642}".repeat(300);
    expect(cutObsName(emoji)).toBe("\u{1F642}".repeat(256));
    expect(readObsName(cutObsName(emoji))).not.toBeNull();
    expect(readObsName(cutObsName(`x${emoji}`))).not.toBeNull();
  });

  it("obsRef: the uuid and the cut name, or null for a uuid it can't store", () => {
    expect(obsRef(UUID, "b".repeat(400))).toEqual({ uuid: UUID, name: "b".repeat(256) });
    expect(obsRef("", "BRB")).toBeNull();
    expect(obsRef("f".repeat(129), "BRB")).toBeNull();
    expect(obsRef(`${UUID}\uD800`, "BRB")).toBeNull();
  });

  it("obsPortOk: a whole number from 1 to 65535", () => {
    for (const ok of [1, 4455, 65_535]) expect(obsPortOk(ok)).toBe(true);
    for (const bad of [0, 65_536, 4455.5, Number.NaN, "4455", null]) expect(obsPortOk(bad), String(bad)).toBe(false);
  });

  it("obsPasswordSendable refuses only a lone surrogate", () => {
    expect(obsPasswordSendable("pässwörd ✓ \u{1F642}")).toBe(true);
    expect(obsPasswordSendable("")).toBe(true);
    expect(obsPasswordSendable("abc\uD800")).toBe(false);
    expect(obsPasswordSendable("\uDC00abc")).toBe(false);
  });
});
