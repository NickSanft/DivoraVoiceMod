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

/** Rust's words for an address it refuses (`obs_configure`), by
 *  `HostError`. The page shows `notAnIp` for what `hostPrecheck` refuses, so
 *  both sides say the same. */
export const OBS_HOST_ERRORS = {
  notAnIp: "Type an IP address, like 127.0.0.1, or localhost.",
  linkLocalV6:
    "Divora can't use a link-local IPv6 address (fe80::…). Type the PC's IPv4 address, like 192.168.1.20.",
} as const;

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
// v1.61.0 S1: declared only. The store's OBS track writes these bodies and
// their tests; nothing calls them yet, and each throws if something does.

function notYet(name: string): never {
  throw new Error(`data/obs.ts: ${name} isn't written yet (v1.61.0 S1)`);
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

/** Settings → OBS's status line for `s` (§9.1); `nowMs` for "Trying again in
 *  N s". S1: declared only. */
export function obsStatusLine(s: ObsStatus, nowMs: number): string {
  return notYet("obsStatusLine");
}

/** The warnings under the status line, only for a status that isn't `off`
 *  and has a host: a network address, and OBS asking for no password. S1:
 *  declared only. */
export function obsWarnings(s: ObsStatus): string[] {
  return notYet("obsWarnings");
}

/** The last scene skip's line, `fmtTime` giving its hour and minute; null
 *  without one. S1: declared only. */
export function obsSkipLine(s: ObsStatus, fmtTime: (ms: number) => string): string | null {
  return notYet("obsSkipLine");
}

/** The last text skip's line; null without one. S1: declared only. */
export function obsTextSkipLine(s: ObsStatus, fmtTime: (ms: number) => string): string | null {
  return notYet("obsTextSkipLine");
}

/** What the OBS section's live region says for a change from `prev` to
 *  `next`: edges only, never a countdown; "" for nothing to say. S1:
 *  declared only. */
export function obsAnnouncement(prev: ObsStatus | null, next: ObsStatus): string {
  return notYet("obsAnnouncement");
}

/** §9.3's table: what an OBS picker shows for `stored`, given whether OBS
 *  control is on, the status, the inventory, whether one is loading and
 *  why the last one failed. `what` picks the words (sources or scenes). S1:
 *  declared only. */
export function pickerShown(p: {
  what: "textSource" | "scene";
  stored: ObsRef | null;
  enabled: boolean;
  status: ObsStatus | null;
  inventory: ObsInventory | null;
  busy: boolean;
  error: string | null;
}): ObsPickerShown {
  return notYet("pickerShown");
}

/** Show in OBS's choices: "None", then each text source, one that reads a
 *  file or that another timer shows (`mappedByOthers`: uuid → that timer's
 *  name) disabled with its reason. S1: declared only. */
export function textSourceOptions(inv: ObsInventory, mappedByOthers: ReadonlyMap<string, string>): ObsOption[] {
  return notYet("textSourceOptions");
}

/** A scene picker's choices, in OBS's order. S1: declared only. */
export function sceneOptions(inv: ObsInventory): ObsOption[] {
  return notYet("sceneOptions");
}
