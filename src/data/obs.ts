// v1.61.0: Talk to OBS — the page's side of the connection to OBS, for the
// store and Settings.
//
// Rust holds the connection (src-tauri/src/obs/): one WebSocket to OBS's own
// server at the address in Settings → OBS, and only while it is switched on.
// The page keeps what the user chose in `divora.obs` (`{enabled, host, port,
// breakScene?}`), sends it with `obs_configure`, and keeps a read-only mirror
// of Rust's `ObsStatus`: every OBS command answers it and `obs-status`
// carries each change, and the larger `seq` wins, since an answer can land
// after the events its command caused. Rust is the only judge of an
// address; the page's `hostPrecheck` refuses only what can't be one.
//
// The command and event names, every key and word of `ObsConfig`,
// `ObsStatus` and `ObsInventory`, and the limits the page uses are frozen in
// obs.fixture.json, which the backend's tests read too. A timer's OBS fields
// (`obsText`, `followObs`) are data/timers.ts's, and its `obs` actions are
// data/timerActions.ts's. Pure, so it unit-tests.

// ---- the wire ----

/** Rust's status after every change, to every window. */
export const OBS_STATUS_EVENT = "obs-status";

/** The six commands, all async in the backend. The first four change
 *  something and go through the store's OBS queue; `status` and `inventory`
 *  only read. None of the four waits on OBS; `inventory` does (up to 15 s). */
export const OBS_COMMANDS = {
  configure: "obs_configure",
  reconnect: "obs_reconnect",
  setPassword: "obs_set_password",
  forgetPassword: "obs_forget_password",
  status: "obs_status",
  inventory: "obs_inventory",
} as const;

/** OBS's own default address: this PC, OBS's default port. */
export const OBS_DEFAULT_HOST = "127.0.0.1";
export const OBS_DEFAULT_PORT = 4455;

/** The backend's limits the page uses, pinned by obs.fixture.json
 *  (`limits`). A scene's or a source's uuid is 1-128 characters (OBS's are
 *  36); a stored name is at most 256 (OBS sets none, so the picker cuts a
 *  longer one: only the uuid switches anything); a password is 1-256.
 *  Characters are counted by code point, as Rust counts them. */
export const OBS_MAX_UUID_CHARS = 128;
export const OBS_MAX_NAME_CHARS = 256;
export const OBS_MAX_PASSWORD_CHARS = 256;
/** How many of each an inventory lists at most; past either, it says
 *  `truncated`. */
export const OBS_MAX_SCENES = 200;
export const OBS_MAX_TEXT_SOURCES = 100;

/** Rust's words for an address it refuses (`obs_configure`), by
 *  `HostError`. The page shows `notAnIp` for what `hostPrecheck` refuses, so
 *  both sides say the same. */
export const OBS_HOST_ERRORS = {
  notAnIp: "Type an IP address, like 127.0.0.1, or localhost.",
  linkLocalV6:
    "Divora can't use a link-local IPv6 address (fe80::…). Type the PC's IPv4 address, like 192.168.1.20.",
} as const;

/** Rust's words for a port it refuses (`BAD_PORT`). The page says them for
 *  any port that isn't a whole number from 1 to 65535 and sends nothing:
 *  Rust takes a `u16`, and Tauri would refuse anything else with words of
 *  its own. */
export const OBS_PORT_ERROR = "Type a port from 1 to 65535.";

/** v1.61.0: what Save password says, without sending it, for a password
 *  with a lone surrogate (a broken paste): the backend's JSON reader would
 *  refuse the whole invoke, and OBS can't hold one either. */
export const OBS_PASSWORD_UNSENDABLE =
  "This password has a character Divora can't send. Paste it again from OBS's Show Connect Info.";

/** What `obs_configure` takes. Rust keeps the last one in memory only;
 *  `divora.obs` is the record. */
export interface ObsConfig {
  enabled: boolean;
  host: string;
  port: number;
}

/** Where the link is. `connecting`, `connected` and `retrying` are a link
 *  at work; `off` is none; the rest are stop states, which stay until the
 *  address, the password or Try again changes something. */
export type ObsState =
  | "off"
  | "connecting"
  | "connected"
  | "retrying"
  | "needsPassword"
  | "authFailed"
  | "passwordUnreadable"
  | "tooOld"
  | "needsNewerDivora"
  | "kicked"
  | "unsupported";
export const OBS_STATES: readonly ObsState[] = [
  "off",
  "connecting",
  "connected",
  "retrying",
  "needsPassword",
  "authFailed",
  "passwordUnreadable",
  "tooOld",
  "needsNewerDivora",
  "kicked",
  "unsupported",
];

/** Why the link is retrying (or, for `badVersion` and `missingRequest`,
 *  why it is `unsupported`); `obsExiting` also while still `connected`
 *  after OBS said it is closing. Fixed codes, never OBS's own text. */
export type ObsReason =
  | "refused"
  | "unreachable"
  | "noAnswer"
  | "notObs"
  | "notReady"
  | "closed"
  | "obsExiting"
  | "silent"
  | "tooBig"
  | "protocol"
  | "badVersion"
  | "missingRequest";
export const OBS_REASONS: readonly ObsReason[] = [
  "refused",
  "unreachable",
  "noAnswer",
  "notObs",
  "notReady",
  "closed",
  "obsExiting",
  "silent",
  "tooBig",
  "protocol",
  "badVersion",
  "missingRequest",
];

/** Which scene op a skip was: a timer's Switch OBS scene or Switch OBS
 *  back. */
export type ObsSkipOp = "scene" | "sceneBack";
export const OBS_SKIP_OPS: readonly ObsSkipOp[] = ["scene", "sceneBack"];

/** Why a scene op didn't run, or may not have. */
export type SkipWhy = "notConnected" | "queueFull" | "notFound" | "notReady" | "timedOut" | "failed" | "late";
export const OBS_SKIP_WHYS: readonly SkipWhy[] = [
  "notConnected",
  "queueFull",
  "notFound",
  "notReady",
  "timedOut",
  "failed",
  "late",
];
/** The whys each op can have. A Switch OBS back is never `queueFull` or
 *  `late`: a full queue or a slow OBS makes it wait (up to 2 minutes)
 *  instead; its `notConnected` and `notReady` mean that wait ran out. */
export const OBS_SKIP_WHYS_BY_OP: Readonly<Record<ObsSkipOp, readonly SkipWhy[]>> = {
  scene: OBS_SKIP_WHYS,
  sceneBack: ["notConnected", "notFound", "notReady", "timedOut", "failed"],
};

/** Why a timer's text didn't reach OBS. */
export type ObsTextSkipWhy = "failed" | "timedOut";
export const OBS_TEXT_SKIP_WHYS: readonly ObsTextSkipWhy[] = ["failed", "timedOut"];

/** OBS's stream and recording, in words; null while not connected. */
export type ObsStreamState = "off" | "starting" | "live" | "reconnecting" | "stopping";
export const OBS_STREAM_STATES: readonly ObsStreamState[] = ["off", "starting", "live", "reconnecting", "stopping"];
export type ObsRecordState = "off" | "starting" | "recording" | "paused" | "stopping";
export const OBS_RECORD_STATES: readonly ObsRecordState[] = ["off", "starting", "recording", "paused", "stopping"];

/** The last scene op that didn't run (or may not have). Replaced by the
 *  next scene skip, cleared by the next switch that succeeds. */
export interface ObsLastSkip {
  op: ObsSkipOp;
  why: SkipWhy;
  /** The scene's name from the action; null for a Switch OBS back. */
  name: string | null;
  /** The timer whose action it was. */
  timer: string;
  /** When, in wall-clock ms. */
  atMs: number;
}

/** The last timer text OBS didn't take. Cleared by the next one it took. */
export interface ObsTextSkip {
  why: ObsTextSkipWhy;
  atMs: number;
}

/** Rust's whole status. No field can hold a secret or free text from OBS:
 *  versions are sanitized, reasons are fixed codes, and the only names are
 *  the user's own scene names from an action. */
export interface ObsStatus {
  /** +1 per change in the backend's process; the larger is newer. */
  seq: number;
  state: ObsState;
  /** The canonical IP literal in use; "" before any `obs_configure`. */
  host: string;
  port: number;
  /** The address is this PC. */
  loopback: boolean;
  /** A password is saved (`obs/password.bin`); never the password. */
  hasPassword: boolean;
  /** From the last Hello; null before one. */
  authRequired: boolean | null;
  /** OBS's version (GetVersion), sanitized. */
  obsVersion: string | null;
  /** OBS's WebSocket server's version (Hello), sanitized. */
  websocketVersion: string | null;
  stream: ObsStreamState | null;
  record: ObsRecordState | null;
  /** Why retrying or stopped. */
  reason: ObsReason | null;
  /** Only for `missingRequest`: the request OBS doesn't offer. */
  detail: string | null;
  /** While retrying: the wall-clock ms of the next attempt. */
  retryAtMs: number | null;
  /** Timer text sources OBS can't find on this connection, by uuid. */
  missing: string[];
  lastSkip: ObsLastSkip | null;
  textSkip: ObsTextSkip | null;
  /** Switch OBS backs waiting for a connection (each up to 2 minutes). */
  heldBack: number;
  /** +1 per connection and per burst of scene changes in OBS (at most one
   *  per 250 ms): the page reads the inventory again when it changes. */
  inventoryRev: number;
}

/** A scene, in OBS's top-to-bottom order. */
export interface ObsScene {
  uuid: string;
  name: string;
}

/** A Text (GDI+) or Text (FreeType 2) source. */
export type ObsTextSourceKind = "gdiplus" | "ft2";
export const OBS_TEXT_SOURCE_KINDS: readonly ObsTextSourceKind[] = ["gdiplus", "ft2"];

/** A text source, and whether it reads its text from a file (then a timer
 *  can't show there until that is unticked in OBS). Nothing else from its
 *  settings reaches the page. */
export interface ObsTextSource {
  uuid: string;
  name: string;
  kind: ObsTextSourceKind;
  readsFromFile: boolean;
}

/** `obs_inventory`'s answer: what the pickers offer. */
export interface ObsInventory {
  /** The `inventoryRev` it was read at. */
  rev: number;
  scenes: ObsScene[];
  textSources: ObsTextSource[];
  /** The scene that is live now. */
  programSceneUuid: string | null;
  /** OBS has more than 200 scenes or 100 text sources: the rest aren't
   *  listed. */
  truncated: boolean;
}

/** An `obs` timer action's part after `kind` (data/timerActions.ts): flat on
 *  the wire, like every other action. `scene` switches OBS's program scene
 *  (by uuid; the name is for words); `sceneBack` switches it back to the
 *  scene the timer's start replaced, if OBS is still on the one it set. */
export type ObsOp = { op: "scene"; sceneUuid: string; sceneName: string } | { op: "sceneBack" };
export type ObsOpName = ObsOp["op"];

/** A count-up that follows OBS: it starts at 0 when the stream (or the
 *  recording) starts, and pauses when it stops. */
export type FollowObs = "stream" | "record";
export const FOLLOW_OBS: readonly FollowObs[] = ["stream", "record"];

// ---- what the page stores ----

/** A scene or a text source the user picked, as stored: OBS's uuid, which
 *  is what Divora acts on, and its name as last seen, for words. A timer's
 *  `obsText` and `divora.obs`' `breakScene`. */
export interface ObsRef {
  uuid: string;
  name: string;
}

/** `divora.obs`. Never a secret: the password lives only in Rust's
 *  `obs/password.bin`. Unknown keys are kept. */
export interface ObsSettings {
  enabled: boolean;
  host: string;
  port: number;
  /** The scene Add break (BRB) switches OBS to, while OBS control is on. */
  breakScene?: ObsRef;
  [k: string]: unknown;
}

/** A change to `divora.obs`; `breakScene: null` removes it. */
export interface ObsSettingsPatch {
  enabled?: boolean;
  host?: string;
  port?: number;
  breakScene?: ObsRef | null;
}

// ---- small helpers ----

function isObject(v: unknown): v is Record<string, unknown> {
  return typeof v === "object" && v !== null && !Array.isArray(v);
}

/** A count: a whole number, 0 or more. */
function isCount(v: unknown): v is number {
  return typeof v === "number" && Number.isSafeInteger(v) && v >= 0;
}

/** A wall-clock ms: a whole number. */
function isWallMs(v: unknown): v is number {
  return typeof v === "number" && Number.isSafeInteger(v);
}

function isOneOf<T extends string>(list: readonly T[], v: unknown): v is T {
  return typeof v === "string" && (list as readonly string[]).includes(v);
}

function isStringOrNull(v: unknown): v is string | null {
  return v === null || typeof v === "string";
}

/** A lone surrogate: `JSON.stringify` writes it as `\udXXX`, which the
 *  backend's JSON reader refuses for the whole invoke (data/timerActions.ts
 *  has the same rule for every text it sends). */
const LONE_SURROGATE = /\p{Cs}/u;

function codePoints(s: string): number {
  return Array.from(s).length;
}

/** Rust's `str::trim`: Unicode White_Space at either end. */
function trimLikeRust(s: string): string {
  return s.replace(/^\p{White_Space}+|\p{White_Space}+$/gu, "");
}

// ---- reading ----

/** A scene's or a source's uuid as stored and sent: 1-128 characters, no
 *  lone surrogate; null otherwise. */
export function readObsUuid(v: unknown): string | null {
  if (typeof v !== "string" || LONE_SURROGATE.test(v)) return null;
  const n = codePoints(v);
  return n >= 1 && n <= OBS_MAX_UUID_CHARS ? v : null;
}

/** A scene's or a source's stored name: a string of at most 256
 *  characters (empty allowed: the uuid is what switches), no lone
 *  surrogate; null otherwise. */
export function readObsName(v: unknown): string | null {
  if (typeof v !== "string" || LONE_SURROGATE.test(v)) return null;
  return codePoints(v) <= OBS_MAX_NAME_CHARS ? v : null;
}

/** A stored `{uuid, name}`, both readable, its other keys left out; null
 *  otherwise. */
export function readObsRef(v: unknown): ObsRef | null {
  if (!isObject(v)) return null;
  const uuid = readObsUuid(v.uuid);
  const name = readObsName(v.name);
  return uuid === null || name === null ? null : { uuid, name };
}

/** A count-up's `followObs`: "stream" or "record", spelled exactly; null
 *  otherwise. */
export function readFollowObs(v: unknown): FollowObs | null {
  return isOneOf(FOLLOW_OBS, v) ? v : null;
}

/** `divora.obs`, read field by field, so one bad field never loses the
 *  others: `enabled` only when it is `true`; `host` a non-empty string
 *  after trimming (stored trimmed), else 127.0.0.1; `port` a whole number
 *  from 1 to 65535, else 4455; `breakScene` only when `readObsRef` reads it
 *  (else absent). Unknown keys are kept. Missing or unreadable: off, at
 *  127.0.0.1:4455. */
export function readObsSettings(raw: unknown): ObsSettings {
  const o = isObject(raw) ? raw : {};
  const host = typeof o.host === "string" ? trimLikeRust(o.host) : "";
  const port = o.port;
  const out: ObsSettings = {
    ...o,
    enabled: o.enabled === true,
    host: host !== "" ? host : OBS_DEFAULT_HOST,
    port: typeof port === "number" && Number.isInteger(port) && port >= 1 && port <= 65535 ? port : OBS_DEFAULT_PORT,
  };
  const breakScene = readObsRef(o.breakScene);
  if (breakScene) out.breakScene = breakScene;
  else delete out.breakScene;
  return out;
}

/** `cur` with `patch` applied: the one way the store changes
 *  `divora.obs`, always into the value current at save time, so a save
 *  that lands late never writes back an older `enabled`. */
export function mergeObsSettings(cur: ObsSettings, patch: ObsSettingsPatch): ObsSettings {
  const { breakScene, ...rest } = patch;
  const next: ObsSettings = { ...cur, ...rest };
  if (breakScene === null) delete next.breakScene;
  else if (breakScene !== undefined) next.breakScene = breakScene;
  return next;
}

/** What `obs_configure` sends for the stored settings: those three keys,
 *  nothing else. */
export function obsConfig(s: ObsSettings): ObsConfig {
  return { enabled: s.enabled, host: s.host, port: s.port };
}

/** The Address field's text, trimmed, or null when it can't be an address
 *  at all: empty, a space inside, or a URL (`://`). Everything else goes to
 *  Rust, which decides (obs.fixture.json `hostSamples`, `precheck`). */
export function hostPrecheck(typed: string): string | null {
  const host = trimLikeRust(typed);
  if (host === "" || /\p{White_Space}/u.test(host) || host.includes("://")) return null;
  return host;
}

/** A port Rust takes: a whole number from 1 to 65535. */
export function obsPortOk(port: unknown): port is number {
  return typeof port === "number" && Number.isInteger(port) && port >= 1 && port <= 65535;
}

/** A password the page can send: no lone surrogate. Rust judges the rest
 *  (empty, longer than 256 characters) and answers in its own words. */
export function obsPasswordSendable(password: string): boolean {
  return !LONE_SURROGATE.test(password);
}

/** A name from OBS as the page stores it: its first 256 characters, counted
 *  by code point as `readObsName` (and Rust) count them, so a picked scene
 *  or source with a longer name still makes an action and an `obsText`
 *  that read. OBS sets no limit; the uuid is what Divora acts on, and the
 *  name is only for words. A cut never splits a pair: Rust strings hold
 *  none to split, and a code point is never cut in half. */
export function cutObsName(name: string): string {
  return codePoints(name) <= OBS_MAX_NAME_CHARS ? name : Array.from(name).slice(0, OBS_MAX_NAME_CHARS).join("");
}

/** A picked scene or source as stored: its uuid and its cut name; null
 *  when the uuid can't be stored (`readObsRef`). */
export function obsRef(uuid: string, name: string): ObsRef | null {
  return readObsRef({ uuid, name: cutObsName(name) });
}

function readLastSkip(v: unknown): ObsLastSkip | null | undefined {
  if (v === null) return null;
  if (!isObject(v)) return undefined;
  const { op, why, name, timer, atMs } = v;
  if (!isOneOf(OBS_SKIP_OPS, op) || !isOneOf(OBS_SKIP_WHYS_BY_OP[op], why)) return undefined;
  if (!isStringOrNull(name) || typeof timer !== "string" || !isWallMs(atMs)) return undefined;
  return { op, why, name, timer, atMs };
}

function readTextSkip(v: unknown): ObsTextSkip | null | undefined {
  if (v === null) return null;
  if (!isObject(v) || !isOneOf(OBS_TEXT_SKIP_WHYS, v.why) || !isWallMs(v.atMs)) return undefined;
  return { why: v.why, atMs: v.atMs };
}

/** A status off the wire (an `obs-status` payload or a command's answer),
 *  or null when it isn't one: a key missing or of the wrong type, a state,
 *  reason, word or skip this build doesn't know, a skip's why its op can't
 *  have. Keys a later backend adds are left out. */
export function readObsStatus(raw: unknown): ObsStatus | null {
  if (!isObject(raw)) return null;
  const o = raw;
  if (!isCount(o.seq) || !isOneOf(OBS_STATES, o.state)) return null;
  if (typeof o.host !== "string" || !isCount(o.port) || o.port > 65535) return null;
  if (typeof o.loopback !== "boolean" || typeof o.hasPassword !== "boolean") return null;
  if (o.authRequired !== null && typeof o.authRequired !== "boolean") return null;
  if (!isStringOrNull(o.obsVersion) || !isStringOrNull(o.websocketVersion) || !isStringOrNull(o.detail)) return null;
  if (o.stream !== null && !isOneOf(OBS_STREAM_STATES, o.stream)) return null;
  if (o.record !== null && !isOneOf(OBS_RECORD_STATES, o.record)) return null;
  if (o.reason !== null && !isOneOf(OBS_REASONS, o.reason)) return null;
  if (o.retryAtMs !== null && !isWallMs(o.retryAtMs)) return null;
  if (!Array.isArray(o.missing) || !(o.missing as unknown[]).every((u) => typeof u === "string")) return null;
  if (!isCount(o.heldBack) || !isCount(o.inventoryRev)) return null;
  const lastSkip = readLastSkip(o.lastSkip);
  const textSkip = readTextSkip(o.textSkip);
  if (lastSkip === undefined || textSkip === undefined) return null;
  return {
    seq: o.seq,
    state: o.state,
    host: o.host,
    port: o.port,
    loopback: o.loopback,
    hasPassword: o.hasPassword,
    authRequired: o.authRequired,
    obsVersion: o.obsVersion,
    websocketVersion: o.websocketVersion,
    stream: o.stream,
    record: o.record,
    reason: o.reason,
    detail: o.detail,
    retryAtMs: o.retryAtMs,
    missing: [...(o.missing as string[])],
    lastSkip,
    textSkip,
    heldBack: o.heldBack,
    inventoryRev: o.inventoryRev,
  };
}

/** What the mirror keeps next: the incoming status only if it is newer (a
 *  larger `seq`); an equal one keeps what it holds. */
export function adoptObsStatus(held: ObsStatus | null, incoming: ObsStatus | null): ObsStatus | null {
  if (incoming === null) return held;
  if (held === null || incoming.seq > held.seq) return incoming;
  return held;
}

/** `obs_inventory`'s answer, or null when it isn't one (a key missing or of
 *  the wrong type). A scene or a source that can't be read (a uuid that
 *  isn't 1-128 characters, a name that isn't a string, a kind this build
 *  doesn't know) is left out, and so is a uuid seen before in its list.
 *  Keys a later backend adds are left out. Names are kept at any length:
 *  the picker cuts what it stores. */
export function readObsInventory(raw: unknown): ObsInventory | null {
  if (!isObject(raw)) return null;
  const { rev, scenes, textSources, programSceneUuid, truncated } = raw;
  if (!isCount(rev) || !Array.isArray(scenes) || !Array.isArray(textSources)) return null;
  if (!isStringOrNull(programSceneUuid) || typeof truncated !== "boolean") return null;
  const sceneOut: ObsScene[] = [];
  for (const s of scenes as unknown[]) {
    if (!isObject(s) || typeof s.name !== "string") continue;
    const uuid = readObsUuid(s.uuid);
    if (uuid === null || sceneOut.some((x) => x.uuid === uuid)) continue;
    sceneOut.push({ uuid, name: s.name });
  }
  const sourceOut: ObsTextSource[] = [];
  for (const s of textSources as unknown[]) {
    if (!isObject(s) || typeof s.name !== "string") continue;
    if (!isOneOf(OBS_TEXT_SOURCE_KINDS, s.kind) || typeof s.readsFromFile !== "boolean") continue;
    const uuid = readObsUuid(s.uuid);
    if (uuid === null || sourceOut.some((x) => x.uuid === uuid)) continue;
    sourceOut.push({ uuid, name: s.name, kind: s.kind, readsFromFile: s.readsFromFile });
  }
  return { rev, scenes: sceneOut, textSources: sourceOut, programSceneUuid, truncated };
}

// ---- words and pickers ----
// Every user-visible string the OBS section, a timer's card and the pickers
// build from a status or an inventory (design §9). Settings lays them out;
// these say them.

/** The states that stay until the address, the password or Try again
 *  changes something (`retrying` retries by itself; `off` is no link). */
export const OBS_STOP_STATES: readonly ObsState[] = [
  "needsPassword",
  "authFailed",
  "passwordUnreadable",
  "tooOld",
  "needsNewerDivora",
  "kicked",
  "unsupported",
];

export function isObsStopState(state: ObsState): boolean {
  return OBS_STOP_STATES.includes(state);
}

/** `{host}:{port}` as the status line says it; an IPv6 address in
 *  brackets, so its last group isn't read as the port. */
export function obsAddressText(host: string, port: number): string {
  return host.includes(":") ? `[${host}]:${port}` : `${host}:${port}`;
}

/** A scene's name in a sentence. OBS gives every scene one, so an empty
 *  name is a hand edit; it reads as a scene with no name, never as nothing
 *  ("Switch OBS to "). */
export function obsSceneName(name: string | null): string {
  return name === null || name === "" ? "a scene with no name" : name;
}

/** A name as a picker shows it; an empty one as "(no name)". */
function pickerName(name: string): string {
  return name === "" ? "(no name)" : name;
}

/** A skip's `{time}`: its wall-clock ms as the local hour and minute, in
 *  the PC's own way of writing them. */
export function obsClockTime(ms: number): string {
  const d = new Date(ms);
  try {
    return d.toLocaleTimeString(undefined, { hour: "numeric", minute: "2-digit" });
  } catch {
    return `${String(d.getHours()).padStart(2, "0")}:${String(d.getMinutes()).padStart(2, "0")}`;
  }
}

/** Why the link is retrying, as a sentence (§9.1). `badVersion` and
 *  `missingRequest` are `unsupported`'s, never a retry's. */
const RETRY_WHY: Partial<Record<ObsReason, string>> = {
  refused: "Nothing is listening there. Is OBS open, with its WebSocket server on?",
  unreachable: "No answer from that address.",
  noAnswer: "It didn't answer in time.",
  notObs: "That isn't OBS's WebSocket server.",
  notReady: "OBS stayed busy for 30 seconds (starting, closing or changing scene collection).",
  closed: "OBS closed the connection.",
  obsExiting: "OBS is closing.",
  silent: "OBS stopped answering.",
  tooBig: "OBS sent a message over 4 MiB, more than Divora takes.",
  protocol: "OBS and Divora didn't understand each other.",
};

const STREAM_WORDS: Record<ObsStreamState, string> = {
  off: "",
  starting: " · stream starting",
  live: " · live",
  reconnecting: " · stream reconnecting",
  stopping: " · stream stopping",
};

const RECORD_WORDS: Record<ObsRecordState, string> = {
  off: "",
  starting: " · recording starting",
  recording: " · recording",
  paused: " · recording paused",
  stopping: " · recording stopping",
};

/** What the status line says with the toggle on and the saved address
 *  refused by Rust (at boot, or on Connect to OBS): the store's
 *  `obsConfigError` holds Rust's words for the field. */
export const OBS_SAVED_ADDRESS_LINE = "Not connected: the saved address isn't valid. Fix it below.";

/** Settings → OBS's status line for `s` (§9.1); `nowMs` for "Trying again in
 *  N s", whole seconds rounded up, "now" at or past the attempt. Plain
 *  text: the section never puts it in a live region (it would speak the
 *  countdown), and `obsAnnouncement` says only the edges. */
export function obsStatusLine(s: ObsStatus, nowMs: number): string {
  switch (s.state) {
    case "off":
      return "Off. Divora doesn't connect to OBS.";
    case "connecting":
      return `Connecting to ${obsAddressText(s.host, s.port)}…`;
    case "connected": {
      const head =
        s.obsVersion !== null && s.websocketVersion !== null
          ? `Connected to OBS ${s.obsVersion} (WebSocket ${s.websocketVersion})`
          : s.obsVersion !== null
            ? `Connected to OBS ${s.obsVersion}`
            : s.websocketVersion !== null
              ? `Connected to OBS (WebSocket ${s.websocketVersion})`
              : "Connected to OBS";
      return `${head}${s.stream === null ? "" : STREAM_WORDS[s.stream]}${s.record === null ? "" : RECORD_WORDS[s.record]}`;
    }
    case "retrying": {
      const why = s.reason === null ? undefined : RETRY_WHY[s.reason];
      const parts = [`Can't reach OBS at ${obsAddressText(s.host, s.port)}.`];
      if (why !== undefined) parts.push(why);
      if (s.retryAtMs !== null) {
        const n = Math.ceil((s.retryAtMs - nowMs) / 1000);
        parts.push(n <= 0 ? "Trying again now." : `Trying again in ${n} s.`);
      }
      return parts.join(" ");
    }
    case "needsPassword":
      return "OBS is asking for a password. Paste it from OBS's Show Connect Info, then Save password. Or turn off Enable Authentication in OBS and press Try again.";
    case "authFailed":
      return "OBS rejected the password. Paste it again from OBS's Show Connect Info, then Save password.";
    case "passwordUnreadable":
      return "Re-enter your OBS password. Windows couldn't read the saved one, which can happen after a change to your Windows account.";
    case "tooOld":
      return s.websocketVersion !== null
        ? `Update OBS, then press Try again. Its WebSocket server is ${s.websocketVersion}, and Divora needs 5.4.0 or later.`
        : "Update OBS, then press Try again. Divora needs its WebSocket server 5.4.0 or later.";
    case "needsNewerDivora":
      return "This OBS needs a newer Divora. Check for updates in Settings → About.";
    case "kicked":
      return "OBS disconnected Divora from its list of connections. Divora won't reconnect on its own.";
    case "unsupported":
      if (s.reason === "badVersion") {
        return "Divora can't read this OBS's WebSocket version. Update OBS, then press Try again.";
      }
      // `missingRequest` names the request; without a name (not a status
      // Rust writes), the sentence still says what to do.
      return s.detail !== null
        ? `This OBS's WebSocket server doesn't offer ${s.detail}, which Divora needs. Update OBS, then press Try again.`
        : "This OBS's WebSocket server doesn't offer everything Divora needs. Update OBS, then press Try again.";
  }
}

/** The button beside the status line: "Try again" where pressing it can
 *  help (a retry now, or after fixing OBS or the password), "Connect again"
 *  after OBS's Kick; none otherwise (§9.1). */
export function obsRetryLabel(s: ObsStatus): "Try again" | "Connect again" | null {
  switch (s.state) {
    case "needsPassword":
    case "authFailed":
    case "tooOld":
    case "unsupported":
    case "retrying":
      return "Try again";
    case "kicked":
      return "Connect again";
    default:
      return null;
  }
}

/** The warnings under the status line, only for a status that isn't `off`
 *  and has a host: a network address, and OBS asking for no password. */
export function obsWarnings(s: ObsStatus): string[] {
  if (s.state === "off" || s.host === "") return [];
  const out: string[] = [];
  if (!s.loopback) {
    out.push(
      `${s.host} is a network address, so Divora talks to OBS over the network, without encryption. Anyone who can see that traffic can read what Divora and OBS send each other, including your sources' names and settings and your recording's file path, and could change it or take over the connection to control OBS. Your password itself is never sent.`,
    );
  }
  if (s.authRequired === false) {
    out.push(
      "OBS isn't asking for a password, so anything that can reach it on your network can control it. Turn on Enable Authentication in OBS's WebSocket Server Settings.",
    );
  }
  return out;
}

/** A Switch OBS scene's skip, by why, `t` its time and `n` its scene. */
const SCENE_SKIP: Record<SkipWhy, (t: string, n: string) => string> = {
  notConnected: (t, n) => `${t}: Didn't switch OBS to ${n}: Divora wasn't connected to OBS.`,
  queueFull: (t, n) => `${t}: Didn't switch OBS to ${n}: too many OBS requests were waiting.`,
  notFound: (t, n) => `${t}: Didn't switch OBS to ${n}: OBS doesn't have that scene now.`,
  notReady: (t, n) => `${t}: Didn't switch OBS to ${n}: OBS was busy (starting, closing or changing scene collection).`,
  late: (t, n) =>
    `${t}: Didn't switch OBS to ${n}: OBS was busy for more than 5 seconds, and a late switch is worse than none.`,
  timedOut: (t, n) => `${t}: OBS didn't answer the switch to ${n} in time. It may switch late, or not at all.`,
  failed: (t, n) => `${t}: OBS refused the switch to ${n}.`,
};

/** A Switch OBS back's skip. Never `queueFull` or `late` (`readObsStatus`
 *  refuses a status with either): a full queue or a slow OBS makes it wait
 *  instead. */
const BACK_SKIP: Partial<Record<SkipWhy, (t: string) => string>> = {
  notConnected: (t) => `${t}: Didn't switch OBS back: Divora wasn't connected to OBS within 2 minutes of the break's end.`,
  notReady: (t) => `${t}: Didn't switch OBS back: OBS stayed busy for 2 minutes after the break's end.`,
  notFound: (t) => `${t}: Didn't switch OBS back: OBS doesn't have that scene now.`,
  timedOut: (t) => `${t}: OBS didn't answer the switch back in time. It may switch late, or not at all.`,
  failed: (t) => `${t}: OBS refused the switch back.`,
};

/** The last scene skip's line, `fmtTime` giving its hour and minute; null
 *  without one. */
export function obsSkipLine(s: ObsStatus, fmtTime: (ms: number) => string): string | null {
  const k = s.lastSkip;
  if (k === null) return null;
  const t = fmtTime(k.atMs);
  if (k.op === "scene") return SCENE_SKIP[k.why](t, obsSceneName(k.name));
  return BACK_SKIP[k.why]?.(t) ?? null;
}

/** The last text skip's line; null without one. */
export function obsTextSkipLine(s: ObsStatus, fmtTime: (ms: number) => string): string | null {
  const k = s.textSkip;
  if (k === null) return null;
  const t = fmtTime(k.atMs);
  return k.why === "failed"
    ? `${t}: OBS refused a timer's text. Divora tries again in a moment.`
    : `${t}: OBS didn't take a timer's text in time. Divora tries again in a moment.`;
}

/** The held-back line: Switch OBS backs waiting for a connection; null
 *  with none. */
export function obsHeldBackLine(s: ObsStatus): string | null {
  return s.heldBack > 0 ? "Waiting for OBS, to switch it back after a break (up to 2 minutes)." : null;
}

function sameSkip(a: ObsLastSkip | null, b: ObsLastSkip): boolean {
  return a !== null && a.atMs === b.atMs && a.op === b.op && a.why === b.why && a.timer === b.timer && a.name === b.name;
}

/** What the OBS section's live region says for a change from `prev` to
 *  `next`: edges only, never a countdown; "" for nothing to say (§9.1).
 *  The edges: entering `connected` ("Connected to OBS."); entering a stop
 *  state (its status line); `connected` → `retrying` ("Lost the connection
 *  to OBS. Divora keeps trying."); a new scene skip (its line); held-back
 *  switches going from none to some (the held-back line). Never
 *  `connecting`, a countdown, or a `retrying` after a `retrying` or an
 *  attempt (`connecting`): backoff goes retrying → connecting → retrying
 *  every attempt, and only the first loss is news. The first status the
 *  section sees (`prev` null) is not a change. Several edges at once are
 *  said in that order. `fmtTime` as `obsSkipLine`'s. */
export function obsAnnouncement(
  prev: ObsStatus | null,
  next: ObsStatus,
  fmtTime: (ms: number) => string = obsClockTime,
): string {
  if (prev === null) return "";
  const said: string[] = [];
  if (prev.state !== next.state) {
    if (next.state === "connected") said.push("Connected to OBS.");
    else if (isObsStopState(next.state)) said.push(obsStatusLine(next, 0));
    else if (prev.state === "connected" && next.state === "retrying") {
      said.push("Lost the connection to OBS. Divora keeps trying.");
    }
  }
  if (next.lastSkip !== null && !sameSkip(prev.lastSkip, next.lastSkip)) {
    const line = obsSkipLine(next, fmtTime);
    if (line !== null) said.push(line);
  }
  if (prev.heldBack === 0 && next.heldBack > 0) said.push(obsHeldBackLine(next)!);
  return said.join(" ");
}

/** One choice in an OBS picker, as `Select` takes it. */
export interface ObsOption {
  value: string;
  label: string;
  sub?: string;
  disabled?: boolean;
}

/** What a picker shows for its stored choice (§9.3's table): the text, a
 *  sub under it, and a line under the picker. */
export interface ObsPickerShown {
  text: string;
  sub: string | null;
  note: string | null;
}

/** §9.3's table: what an OBS picker (Show in OBS, a Switch OBS scene row,
 *  the Break row) shows for `stored`, given whether OBS control is on, the
 *  status, the inventory, whether one is loading and why the last one
 *  failed (Rust's words). `what` picks the words: sources or scenes; with
 *  nothing stored a source picker reads "None" and a scene picker "Don't
 *  switch". By the first row that holds:
 *  1. OBS control off: the stored name, nothing more (the card says it
 *     works once Settings → OBS is on);
 *  2. a text source in `status.missing`: "{name} (not in OBS now)";
 *  3. not connected: "{name} (not connected)";
 *  4. loading, or connected with no inventory yet: "{name}", sub "Loading
 *     OBS sources…" / "Loading OBS scenes…";
 *  5. the inventory failed: the stored name, and "Couldn't read OBS's
 *     sources: {error}" / "…OBS's scenes: …" under the picker;
 *  6. the inventory lists the uuid: the inventory's name (the store
 *     refreshes the stored one to it); it doesn't: "{name} (not in OBS
 *     now)", unless that list was cut at its limit (`truncated`), when the
 *     uuid may be past the cut and the stored name shows. Under the picker,
 *     no text sources: "No text sources in OBS…"; a list cut: "Divora lists
 *     OBS's first 100 text sources." / "…first 200 scenes.".
 *  §9.3 names row 6's "not in OBS now" for scenes; a text source an
 *  uncut inventory doesn't list is just as gone (deleted, or in another
 *  scene collection), so it says so too. */
export function pickerShown(p: {
  what: "textSource" | "scene";
  stored: ObsRef | null;
  enabled: boolean;
  status: ObsStatus | null;
  inventory: ObsInventory | null;
  busy: boolean;
  error: string | null;
}): ObsPickerShown {
  const scenes = p.what === "scene";
  const name = p.stored === null ? (scenes ? "Don't switch" : "None") : pickerName(p.stored.name);
  const plain = (text: string, sub: string | null = null, note: string | null = null): ObsPickerShown => ({
    text,
    sub,
    note,
  });
  const gone = p.stored === null ? name : `${name} (not in OBS now)`;
  if (!p.enabled) return plain(name);
  if (!scenes && p.stored !== null && p.status?.missing.includes(p.stored.uuid)) return plain(gone);
  if (p.status?.state !== "connected") return plain(p.stored === null ? name : `${name} (not connected)`);
  if (p.busy || (p.inventory === null && p.error === null)) {
    return plain(name, scenes ? "Loading OBS scenes…" : "Loading OBS sources…");
  }
  if (p.error !== null || p.inventory === null) {
    return plain(name, null, `Couldn't read OBS's ${scenes ? "scenes" : "sources"}: ${p.error ?? ""}`);
  }
  const inv = p.inventory;
  const list: ReadonlyArray<{ uuid: string; name: string }> = scenes ? inv.scenes : inv.textSources;
  const cut = inv.truncated && list.length >= (scenes ? OBS_MAX_SCENES : OBS_MAX_TEXT_SOURCES);
  const note =
    !scenes && inv.textSources.length === 0
      ? "No text sources in OBS. Add a Text (GDI+) source there, then open this again."
      : cut
        ? scenes
          ? "Divora lists OBS's first 200 scenes."
          : "Divora lists OBS's first 100 text sources."
        : null;
  if (p.stored === null) return plain(name, null, note);
  const listed = list.find((x) => x.uuid === p.stored!.uuid);
  if (listed) return plain(pickerName(listed.name), null, note);
  return plain(cut ? name : gone, null, note);
}

/** The sub of a text source that reads a file: it can't be picked. */
const READS_A_FILE = "reads a file — untick Read from file in its Properties in OBS to show a timer here";

/** Show in OBS's choices: "None" (value ""), then each text source in OBS's
 *  order (value its uuid), its kind as its sub; one that reads a file, or
 *  that another timer shows (`mappedByOthers`: uuid → that timer's name),
 *  disabled with its reason as its sub. */
export function textSourceOptions(inv: ObsInventory, mappedByOthers: ReadonlyMap<string, string>): ObsOption[] {
  const out: ObsOption[] = [{ value: "", label: "None" }];
  for (const t of inv.textSources) {
    const label = pickerName(t.name);
    const other = mappedByOthers.get(t.uuid);
    if (t.readsFromFile) out.push({ value: t.uuid, label, sub: READS_A_FILE, disabled: true });
    else if (other !== undefined) out.push({ value: t.uuid, label, sub: `shown by ${other}`, disabled: true });
    else out.push({ value: t.uuid, label, sub: t.kind === "gdiplus" ? "Text (GDI+)" : "Text (FreeType 2)" });
  }
  return out;
}

/** A scene picker's choices, in OBS's order (value its uuid). The Break
 *  row puts its "Don't switch" first. */
export function sceneOptions(inv: ObsInventory): ObsOption[] {
  return inv.scenes.map((s) => ({ value: s.uuid, label: pickerName(s.name) }));
}
