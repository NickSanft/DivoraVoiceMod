//! v1.61.0: obs-websocket v5's wire, as pure functions — Hello and the
//! version gate, Identify and its auth string, the nine requests Divora ever
//! sends, what comes in (answers and the six events it reads), close codes,
//! and which inputs are text sources. No IO, no clock, no log (§5.2): the
//! link hands frames in and sends what comes out.

use base64::engine::general_purpose::STANDARD;
use base64::Engine as _;
use serde_json::Value;
use sha2::{Digest, Sha256};
use zeroize::{Zeroize, Zeroizing};

use super::machine::{ObsReason, OutputStatus, Stop, TextKind, Unsupported};
use super::scenes::SceneRef;
use super::MAX_UUID_CHARS;

/// D11: the RPC version Divora always Identifies with, whatever Hello
/// announced (a 4010 then means OBS no longer serves it).
pub const RPC_VERSION: u64 = 1;

/// D10: `eventSubscriptions`, always sent: General (1) | Scenes (4) |
/// Outputs (64).
pub const EVENT_MASK: u64 = 69;

/// An obs-websocket version: major, minor, patch.
pub type WsVersion = (u32, u32, u32);

/// D11: the oldest WebSocket server Divora talks to (scene and input uuids
/// first appear in 5.4.0).
pub const FLOOR: WsVersion = (5, 4, 0);

// ---- Hello, the gate, Identify (D11, D12) -----------------------------------

/// What Hello's `authentication` asks for (D12).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuthChallenge {
    pub challenge: String,
    pub salt: String,
}

/// op 0, as [`parse_hello`] reads it. Unknown fields are ignored.
#[derive(Clone, Debug, PartialEq)]
pub struct Hello {
    /// The RPC version the server would like; parsed, never compared (D11).
    pub rpc_version: u64,
    /// `obsWebSocketVersion` as sent (absent: `Null`); [`gate`] reads it.
    pub ws_version: Value,
    /// `obsStudioVersion` (absent before 5.6).
    pub obs_version: Option<String>,
    /// `None`: OBS asks for no password.
    pub auth: Option<AuthChallenge>,
}

/// A first frame that isn't a Hello (§7.1: `notObs`, retried).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProtoError;

/// Hello's `challenge` and `salt` are each at most this many characters
/// (OBS's are 44: base64 of 32 bytes).
const MAX_AUTH_FIELD_CHARS: usize = 128;

/// D11: op 0 with `d` an object, `rpcVersion` an unsigned integer ≥ 1, and
/// `authentication` absent or `{challenge, salt}`, strings of at most 128
/// characters; anything else isn't a Hello.
pub fn parse_hello(text: &str) -> Result<Hello, ProtoError> {
    let msg: Value = serde_json::from_str(text).map_err(|_| ProtoError)?;
    if msg.get("op").and_then(Value::as_u64) != Some(0) {
        return Err(ProtoError);
    }
    let d = msg.get("d").and_then(Value::as_object).ok_or(ProtoError)?;
    // `as_u64` is `None` for a negative number, a fraction and a string.
    let rpc_version = d
        .get("rpcVersion")
        .and_then(Value::as_u64)
        .filter(|&v| v >= 1)
        .ok_or(ProtoError)?;
    let auth = match d.get("authentication") {
        None => None,
        Some(a) => {
            let field = |key: &str| {
                a.get(key)
                    .and_then(Value::as_str)
                    .filter(|s| s.chars().count() <= MAX_AUTH_FIELD_CHARS)
                    .map(str::to_string)
                    .ok_or(ProtoError)
            };
            Some(AuthChallenge {
                challenge: field("challenge")?,
                salt: field("salt")?,
            })
        }
    };
    Ok(Hello {
        rpc_version,
        ws_version: d.get("obsWebSocketVersion").cloned().unwrap_or(Value::Null),
        obs_version: d
            .get("obsStudioVersion")
            .and_then(Value::as_str)
            .map(str::to_string),
        auth,
    })
}

/// D11, §7.2, before any Identify: a version that isn't a string with a
/// leading `x.y.z` is `unsupported` (`badVersion`); below [`FLOOR`],
/// `tooOld`; else `Ok`. `rpc_version` isn't compared.
pub fn gate(h: &Hello) -> Result<(), Stop> {
    match ws_version(&h.ws_version) {
        None => Err(Stop::Unsupported(Unsupported::BadVersion)),
        Some(v) if v < FLOOR => Err(Stop::TooOld),
        Some(_) => Ok(()),
    }
}

/// D11: a version's leading `x.y.z` (a `-…` suffix dropped); `None` for
/// anything else, a number included. "Leading": whatever follows the third
/// number is ignored, so `5.5.0-beta1` is 5.5.0. A part past `u32` is
/// unreadable.
pub fn ws_version(v: &Value) -> Option<WsVersion> {
    /// The run of ASCII digits `rest` starts with, as a number; `rest` moves
    /// past it.
    fn number(rest: &mut &str) -> Option<u32> {
        let end = rest
            .find(|c: char| !c.is_ascii_digit())
            .unwrap_or(rest.len());
        let (digits, tail) = rest.split_at(end);
        *rest = tail;
        if digits.is_empty() {
            return None;
        }
        digits.parse().ok()
    }
    let mut rest = v.as_str()?;
    let major = number(&mut rest)?;
    rest = rest.strip_prefix('.')?;
    let minor = number(&mut rest)?;
    rest = rest.strip_prefix('.')?;
    let patch = number(&mut rest)?;
    Some((major, minor, patch))
}

/// D12: Identify's `authentication`: `base64(sha256(base64(sha256(password
/// ‖ salt)) ‖ challenge))`, standard padded base64 over UTF-8 bytes (fixture
/// `authVectors`). Built without joining strings, so no copy of the password
/// or of the secret is left behind a reallocation: each digest is copied
/// into a wiped array and the hasher's returned one wiped, and each text is
/// encoded into a buffer sized for it. The secret answers any challenge
/// until OBS restarts its server, so it is wiped when this returns. (sha2's
/// own block buffer isn't: it has no zeroize feature.)
#[must_use]
pub fn auth_string(password: &[u8], salt: &str, challenge: &str) -> Zeroizing<String> {
    let secret = encoded_digest(password, salt.as_bytes());
    encoded_digest(secret.as_bytes(), challenge.as_bytes())
}

/// `base64(sha256(a ‖ b))`, every buffer Divora holds wiped on drop.
fn encoded_digest(a: &[u8], b: &[u8]) -> Zeroizing<String> {
    let mut out = Sha256::new().chain_update(a).chain_update(b).finalize();
    let mut digest = Zeroizing::new([0_u8; 32]);
    digest.copy_from_slice(&out);
    out.as_mut_slice().zeroize();
    // 32 bytes are 44 characters of padded base64: the buffer never grows.
    let mut text = Zeroizing::new(String::with_capacity(44));
    STANDARD.encode_string(digest.as_slice(), &mut text);
    text
}

/// D11: op 1, exactly `{"op":1,"d":{"rpcVersion":1,"authentication":…,
/// "eventSubscriptions":69}}`, `authentication` left out with `None`. The
/// keys are in P576's example's order; the mask is always sent, since an
/// absent one means every category, Inputs' echo of each push included.
pub fn identify_json(auth: Option<&str>) -> String {
    let auth = auth
        .map(|a| format!(",\"authentication\":{}", json_string(a)))
        .unwrap_or_default();
    format!(
        "{{\"op\":1,\"d\":{{\"rpcVersion\":{RPC_VERSION}{auth},\"eventSubscriptions\":{EVENT_MASK}}}}}"
    )
}

/// `s` as a JSON string, quoted and escaped.
fn json_string(s: &str) -> String {
    Value::from(s).to_string()
}

// ---- The nine requests (D9) ---------------------------------------------------

/// Which of the nine a request is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RequestKind {
    GetVersion,
    GetSceneList,
    GetCurrentProgramScene,
    SetCurrentProgramScene,
    GetInputList,
    GetInputSettings,
    SetInputSettings,
    GetStreamStatus,
    GetRecordStatus,
}

/// D9: every request type Divora ever sends, in [`RequestKind::ALL`]'s order
/// (fixture `requests`). `GetVersion`'s `availableRequests` must hold each
/// (D11).
pub const REQUEST_TYPES: [&str; 9] = [
    "GetVersion",
    "GetSceneList",
    "GetCurrentProgramScene",
    "SetCurrentProgramScene",
    "GetInputList",
    "GetInputSettings",
    "SetInputSettings",
    "GetStreamStatus",
    "GetRecordStatus",
];

impl RequestKind {
    /// The nine, in [`REQUEST_TYPES`]' order.
    pub const ALL: [Self; 9] = [
        Self::GetVersion,
        Self::GetSceneList,
        Self::GetCurrentProgramScene,
        Self::SetCurrentProgramScene,
        Self::GetInputList,
        Self::GetInputSettings,
        Self::SetInputSettings,
        Self::GetStreamStatus,
        Self::GetRecordStatus,
    ];

    /// Its `requestType`.
    #[must_use]
    pub fn request_type(self) -> &'static str {
        match self {
            Self::GetVersion => "GetVersion",
            Self::GetSceneList => "GetSceneList",
            Self::GetCurrentProgramScene => "GetCurrentProgramScene",
            Self::SetCurrentProgramScene => "SetCurrentProgramScene",
            Self::GetInputList => "GetInputList",
            Self::GetInputSettings => "GetInputSettings",
            Self::SetInputSettings => "SetInputSettings",
            Self::GetStreamStatus => "GetStreamStatus",
            Self::GetRecordStatus => "GetRecordStatus",
        }
    }
}

/// D9: the only requests built: scenes and inputs named by uuid only (a name
/// beside a uuid risks 404 `TooManyRequestFields`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Request {
    GetVersion,
    GetSceneList,
    GetCurrentProgramScene,
    SetCurrentProgramScene {
        uuid: String,
    },
    /// Unfiltered: the request's own kind filter compares versioned ids
    /// (D19).
    GetInputList,
    GetInputSettings {
        uuid: String,
    },
    /// `SetInputSettings {inputUuid, inputSettings: {text}, overlay: true}`:
    /// the source's other settings kept (D18).
    SetText {
        uuid: String,
        text: String,
    },
    GetStreamStatus,
    GetRecordStatus,
}

impl Request {
    /// Which of the nine it is.
    #[must_use]
    pub fn kind(&self) -> RequestKind {
        match self {
            Self::GetVersion => RequestKind::GetVersion,
            Self::GetSceneList => RequestKind::GetSceneList,
            Self::GetCurrentProgramScene => RequestKind::GetCurrentProgramScene,
            Self::SetCurrentProgramScene { .. } => RequestKind::SetCurrentProgramScene,
            Self::GetInputList => RequestKind::GetInputList,
            Self::GetInputSettings { .. } => RequestKind::GetInputSettings,
            Self::SetText { .. } => RequestKind::SetInputSettings,
            Self::GetStreamStatus => RequestKind::GetStreamStatus,
            Self::GetRecordStatus => RequestKind::GetRecordStatus,
        }
    }

    /// Its `requestType`.
    #[must_use]
    pub fn request_type(&self) -> &'static str {
        self.kind().request_type()
    }

    /// op 6: `{"op":6,"d":{"requestType":…,"requestId":id[,"requestData":…]}}`,
    /// `requestData` left out when there is none. Keys in P576's order (its
    /// example, then each request's field table); a scene or an input named
    /// by `sceneUuid` or `inputUuid` alone.
    pub fn request_json(&self, id: &str) -> String {
        let data = match self {
            Self::SetCurrentProgramScene { uuid } => {
                Some(format!("{{\"sceneUuid\":{}}}", json_string(uuid)))
            }
            Self::GetInputSettings { uuid } => {
                Some(format!("{{\"inputUuid\":{}}}", json_string(uuid)))
            }
            Self::SetText { uuid, text } => Some(format!(
                "{{\"inputUuid\":{},\"inputSettings\":{{\"text\":{}}},\"overlay\":true}}",
                json_string(uuid),
                json_string(text)
            )),
            Self::GetVersion
            | Self::GetSceneList
            | Self::GetCurrentProgramScene
            | Self::GetInputList
            | Self::GetStreamStatus
            | Self::GetRecordStatus => None,
        };
        let data = data
            .map(|d| format!(",\"requestData\":{d}"))
            .unwrap_or_default();
        format!(
            "{{\"op\":6,\"d\":{{\"requestType\":\"{}\",\"requestId\":{}{data}}}}}",
            self.request_type(),
            json_string(id)
        )
    }
}

// ---- What comes in (D10) -------------------------------------------------------

/// An output's state, from `outputState`: one of the nine
/// `OBS_WEBSOCKET_OUTPUT_*` identifiers; anything else is `Unknown`. PAUSED
/// and RESUMED come only for a recording, RECONNECTING and RECONNECTED only
/// for a stream.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutputState {
    Unknown,
    Starting,
    Started,
    Stopping,
    Stopped,
    Reconnecting,
    Reconnected,
    Paused,
    Resumed,
}

impl OutputState {
    /// The identifier as OBS sends it; any other text is `Unknown`.
    #[must_use]
    pub fn from_wire(s: &str) -> Self {
        match s {
            "OBS_WEBSOCKET_OUTPUT_STARTING" => Self::Starting,
            "OBS_WEBSOCKET_OUTPUT_STARTED" => Self::Started,
            "OBS_WEBSOCKET_OUTPUT_STOPPING" => Self::Stopping,
            "OBS_WEBSOCKET_OUTPUT_STOPPED" => Self::Stopped,
            "OBS_WEBSOCKET_OUTPUT_RECONNECTING" => Self::Reconnecting,
            "OBS_WEBSOCKET_OUTPUT_RECONNECTED" => Self::Reconnected,
            "OBS_WEBSOCKET_OUTPUT_PAUSED" => Self::Paused,
            "OBS_WEBSOCKET_OUTPUT_RESUMED" => Self::Resumed,
            // `OBS_WEBSOCKET_OUTPUT_UNKNOWN`, and anything else.
            _ => Self::Unknown,
        }
    }
}

/// The six events Divora reads (mask 69); no other field of one is read —
/// the recording's path among them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ObsEvent {
    ExitStarted,
    SceneCreated,
    SceneRemoved,
    SceneNameChanged,
    /// `StreamStateChanged`'s `outputState`.
    Stream(OutputState),
    /// `RecordStateChanged`'s `outputState`.
    Record(OutputState),
}

/// One frame from OBS, as [`parse_incoming`] reads it.
#[derive(Clone, Debug, PartialEq)]
pub enum Incoming {
    /// op 0: ignored on a live connection.
    Hello,
    /// op 2.
    Identified,
    /// op 5, one of the six.
    Event(ObsEvent),
    /// op 7, by `requestId`. `comment` is dropped: it is OBS's text.
    Response {
        id: String,
        ok: bool,
        code: u64,
        data: Option<Value>,
    },
    /// Every other op, event and shape.
    Ignored,
}

/// D10: op 0, 2, 5 and 7, each with `d` an object; everything else —
/// another op, text that isn't a JSON object, an event Divora doesn't read,
/// an answer with no string `requestId` — is [`Incoming::Ignored`]. Of an
/// event only `eventType` and `outputState` are read; of an answer,
/// `requestId`, `requestStatus.result` and `.code`, and `responseData`
/// (moved out, not copied). A `requestStatus` that can't be read is a
/// failure with code 0, so the answer still reaches its request.
pub fn parse_incoming(text: &str) -> Incoming {
    let Ok(Value::Object(mut msg)) = serde_json::from_str::<Value>(text) else {
        return Incoming::Ignored;
    };
    let op = msg.get("op").and_then(Value::as_u64);
    let Some(Value::Object(mut d)) = msg.remove("d") else {
        return Incoming::Ignored;
    };
    match op {
        Some(0) => Incoming::Hello,
        Some(2) => Incoming::Identified,
        Some(5) => {
            let state = || {
                OutputState::from_wire(
                    d.get("eventData")
                        .and_then(|e| e.get("outputState"))
                        .and_then(Value::as_str)
                        .unwrap_or_default(),
                )
            };
            let event = match d.get("eventType").and_then(Value::as_str) {
                Some("ExitStarted") => ObsEvent::ExitStarted,
                Some("SceneCreated") => ObsEvent::SceneCreated,
                Some("SceneRemoved") => ObsEvent::SceneRemoved,
                Some("SceneNameChanged") => ObsEvent::SceneNameChanged,
                Some("StreamStateChanged") => ObsEvent::Stream(state()),
                Some("RecordStateChanged") => ObsEvent::Record(state()),
                _ => return Incoming::Ignored,
            };
            Incoming::Event(event)
        }
        Some(7) => {
            let Some(Value::String(id)) = d.remove("requestId") else {
                return Incoming::Ignored;
            };
            let status = d.get("requestStatus");
            let ok = status
                .and_then(|s| s.get("result"))
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let code = status
                .and_then(|s| s.get("code"))
                .and_then(Value::as_u64)
                .unwrap_or(0);
            let data = d.remove("responseData").filter(|v| !v.is_null());
            Incoming::Response { id, ok, code, data }
        }
        _ => Incoming::Ignored,
    }
}

/// §7.1: how a close code ends the link: a stop or a retry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CloseOutcome {
    Stop(Stop),
    Retry(ObsReason),
}

/// §7.1's close rows: 4009, 4010 and 4011 stop; every other code — none
/// included — retries. obs-websocket's other codes (4000-4008, 4012, any
/// later 4xxx) and RFC 6455's complaints about what Divora sent (1002
/// protocol error, 1003 unacceptable data, 1007 inconsistent data, 1008
/// policy, 1009 too big, 1010 a missing extension) are `protocol`; the rest
/// (1000, 1001 OBS's server stopping, 1005 no code, 1006, 1011, …, none)
/// `closed`. The driver turns `closed` into `obsExiting` after `ExitStarted`.
pub fn close_outcome(code: Option<u16>) -> CloseOutcome {
    match code {
        Some(4009) => CloseOutcome::Stop(Stop::AuthFailed),
        Some(4010) => CloseOutcome::Stop(Stop::NeedsNewerDivora),
        Some(4011) => CloseOutcome::Stop(Stop::Kicked),
        Some(4000..=4999 | 1002 | 1003 | 1007..=1010) => CloseOutcome::Retry(ObsReason::Protocol),
        _ => CloseOutcome::Retry(ObsReason::Closed),
    }
}

// ---- Answers (§5.2): each skips an item it can't read ---------------------------

/// `GetVersion`: `obsVersion` (unsanitized) and `availableRequests` (its
/// strings).
pub fn version(data: &Value) -> (Option<String>, Vec<String>) {
    let obs = data
        .get("obsVersion")
        .and_then(Value::as_str)
        .map(str::to_string);
    let available = data
        .get("availableRequests")
        .and_then(Value::as_array)
        .map(|names| {
            names
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    (obs, available)
}

/// A uuid Divora can keep: 1 to [`MAX_UUID_CHARS`] characters (an action's
/// `sceneUuid` and a timer's `obsText` are read the same way, D18, D21).
fn usable_uuid(v: Option<&Value>) -> Option<String> {
    v.and_then(Value::as_str)
        .filter(|s| !s.is_empty() && s.chars().count() <= MAX_UUID_CHARS)
        .map(str::to_string)
}

/// A scene from `v`'s `uuid_key` and `name_key`; `None` without a usable
/// uuid and a string name.
fn scene_ref(v: &Value, uuid_key: &str, name_key: &str) -> Option<SceneRef> {
    Some(SceneRef {
        uuid: usable_uuid(v.get(uuid_key))?,
        name: v.get(name_key)?.as_str()?.to_string(),
    })
}

/// `GetSceneList`: the program scene and every scene, top first (`sceneIndex`
/// descending). OBS lists them bottom first (ARR:94-108), so a scene with no
/// `sceneIndex` keeps the reversed list's order, after the ones that have
/// one.
pub fn scene_list(data: &Value) -> (Option<SceneRef>, Vec<SceneRef>) {
    let program = scene_ref(data, "currentProgramSceneUuid", "currentProgramSceneName");
    let mut scenes: Vec<(Option<u64>, SceneRef)> = data
        .get("scenes")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .rev()
                .filter_map(|item| {
                    let index = item.get("sceneIndex").and_then(Value::as_u64);
                    Some((index, scene_ref(item, "sceneUuid", "sceneName")?))
                })
                .collect()
        })
        .unwrap_or_default();
    // Stable, descending; `None` sorts below every index.
    scenes.sort_by_key(|(index, _)| std::cmp::Reverse(*index));
    (program, scenes.into_iter().map(|(_, s)| s).collect())
}

/// `GetCurrentProgramScene`: `sceneUuid`/`sceneName`, else the deprecated
/// `currentProgramScene*` pair.
pub fn program_scene(data: &Value) -> Option<SceneRef> {
    scene_ref(data, "sceneUuid", "sceneName")
        .or_else(|| scene_ref(data, "currentProgramSceneUuid", "currentProgramSceneName"))
}

/// One input [`text_kind`] lists, from `GetInputList`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextSourceRef {
    pub uuid: String,
    pub name: String,
    pub kind: TextKind,
}

/// `GetInputList`: the text sources, in OBS's order. An input with no usable
/// `inputUuid` or no string `inputName` is skipped; `inputKind` is read only
/// when `unversionedInputKind` isn't there.
pub fn input_list(data: &Value) -> Vec<TextSourceRef> {
    data.get("inputs")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| {
                    let kind = text_kind(
                        item.get("inputKind")
                            .and_then(Value::as_str)
                            .unwrap_or_default(),
                        item.get("unversionedInputKind").and_then(Value::as_str),
                    )?;
                    Some(TextSourceRef {
                        uuid: usable_uuid(item.get("inputUuid"))?,
                        name: item.get("inputName")?.as_str()?.to_string(),
                        kind,
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// §7.11, D19: whether an input is a text source: `unversionedInputKind` —
/// or, absent, `inputKind` less a trailing `_v<digits>` — is
/// `text_gdiplus` or `text_ft2_source`. Exact names only: `text_gdiplus_old`
/// isn't one.
pub fn text_kind(input_kind: &str, unversioned: Option<&str>) -> Option<TextKind> {
    match unversioned.unwrap_or_else(|| unversioned_kind(input_kind)) {
        "text_gdiplus" => Some(TextKind::Gdiplus),
        "text_ft2_source" => Some(TextKind::Ft2),
        _ => None,
    }
}

/// `kind` less a trailing `_v<digits>` (libobs' versioned ids), else
/// `kind`.
fn unversioned_kind(kind: &str) -> &str {
    if let Some(at) = kind.rfind("_v") {
        let (head, tail) = kind.split_at(at);
        let digits = tail.strip_prefix("_v").unwrap_or_default();
        if !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()) {
            return head;
        }
    }
    kind
}

/// §7.11, D19: `GetInputSettings`' `inputSettings` reads a file: GDI+'s
/// `read_from_file` or `FreeType` 2's `from_file` is `true`. Nothing else of
/// `settings` leaves the link: the flag has no default in OBS, so absent
/// (or anything but JSON `true`) is false.
pub fn reads_from_file(kind: TextKind, settings: &Value) -> bool {
    let flag = match kind {
        TextKind::Gdiplus => "read_from_file",
        TextKind::Ft2 => "from_file",
    };
    settings.get(flag).and_then(Value::as_bool) == Some(true)
}

/// `GetStreamStatus`: `outputActive`, `outputReconnecting`, `outputDuration`.
pub fn stream_status(data: &Value) -> OutputStatus {
    OutputStatus {
        active: flag(data, "outputActive"),
        reconnecting: flag(data, "outputReconnecting"),
        paused: false,
        duration_ms: duration_ms(data),
    }
}

/// `GetRecordStatus`: `outputActive`, `outputPaused`, `outputDuration`.
pub fn record_status(data: &Value) -> OutputStatus {
    OutputStatus {
        active: flag(data, "outputActive"),
        reconnecting: false,
        paused: flag(data, "outputPaused"),
        duration_ms: duration_ms(data),
    }
}

/// `data[key]` is JSON `true`.
fn flag(data: &Value, key: &str) -> bool {
    data.get(key).and_then(Value::as_bool) == Some(true)
}

/// `outputDuration` in whole ms: OBS sends an unsigned integer (NUM:26-36);
/// a fraction is rounded, and anything unreadable is 0 (what OBS answers
/// for an inactive output).
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)] // finite, ≥ 0; `as` saturates
fn duration_ms(data: &Value) -> u64 {
    let Some(v) = data.get("outputDuration") else {
        return 0;
    };
    v.as_u64().unwrap_or_else(|| match v.as_f64() {
        Some(f) if f.is_finite() && f >= 0.0 => f.round() as u64,
        _ => 0,
    })
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic
)]
mod tests {
    use super::{
        auth_string, close_outcome, gate, identify_json, input_list, parse_hello, parse_incoming,
        program_scene, reads_from_file, record_status, scene_list, stream_status, text_kind,
        version, ws_version, AuthChallenge, CloseOutcome, Hello, Incoming, ObsEvent, OutputState,
        ProtoError, Request, RequestKind, TextSourceRef, EVENT_MASK, REQUEST_TYPES, RPC_VERSION,
    };
    use crate::obs::machine::{ObsReason, OutputStatus, Stop, TextKind, Unsupported};
    use crate::obs::scenes::SceneRef;
    use base64::engine::general_purpose::STANDARD;
    use base64::Engine as _;
    use serde_json::{json, Value};
    use sha2::{Digest, Sha256};
    use std::fmt::Write as _;

    fn fixture() -> Value {
        serde_json::from_str(include_str!("../../../src/data/obs.fixture.json")).unwrap()
    }

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().fold(String::new(), |mut s, b| {
            let _ = write!(s, "{b:02x}");
            s
        })
    }

    /// D12, §7.12: the fixture's `authVectors` (Python's `hashlib` and Node's
    /// `crypto` agree on each): the password's UTF-8 bytes are the row's
    /// (so a re-saved file can't change the input), its secret is
    /// `base64(sha256(password ‖ salt))`, and `auth_string` gives its
    /// authentication.
    #[test]
    fn auth_string_matches_python_and_node() {
        let f = fixture();
        let rows = f["authVectors"].as_array().unwrap();
        assert_eq!(rows.len(), 2);
        for row in rows {
            let password = row["password"].as_str().unwrap();
            let salt = row["salt"].as_str().unwrap();
            let challenge = row["challenge"].as_str().unwrap();
            assert_eq!(
                hex(password.as_bytes()),
                row["passwordUtf8Hex"],
                "{password}"
            );
            let secret = STANDARD.encode(Sha256::digest([password, salt].concat()));
            assert_eq!(secret, row["secret"], "{password}");
            let auth = auth_string(password.as_bytes(), salt, challenge);
            assert_eq!(auth.as_str(), row["authentication"], "{password}");
            assert_eq!(auth.len(), 44);
        }
    }

    /// D9, D10, D11: the nine request types, in order (fixture `requests`;
    /// `GetVersion`'s check and the fake's list read the same), each request's
    /// kind, and the RPC version and mask Identify sends.
    #[test]
    fn requests_mask_and_rpc_are_the_fixtures() {
        let f = fixture();
        assert_eq!(f["requests"], serde_json::json!(REQUEST_TYPES));
        let named: Vec<&str> = RequestKind::ALL.iter().map(|k| k.request_type()).collect();
        assert_eq!(named, REQUEST_TYPES);
        let uuid = || "5d3f1c9e-7a2b-4e6d-8f10-2c4b6a8e0d13".to_string();
        let every = [
            Request::GetVersion,
            Request::GetSceneList,
            Request::GetCurrentProgramScene,
            Request::SetCurrentProgramScene { uuid: uuid() },
            Request::GetInputList,
            Request::GetInputSettings { uuid: uuid() },
            Request::SetText {
                uuid: uuid(),
                text: "04:59".into(),
            },
            Request::GetStreamStatus,
            Request::GetRecordStatus,
        ];
        let kinds: Vec<RequestKind> = every.iter().map(Request::kind).collect();
        assert_eq!(kinds, RequestKind::ALL);
        let types: Vec<&str> = every.iter().map(Request::request_type).collect();
        assert_eq!(types, REQUEST_TYPES);
        assert_eq!(f["mask"], EVENT_MASK);
        assert_eq!(f["rpcVersion"], RPC_VERSION);
        assert_eq!(EVENT_MASK, 1 | 4 | 64);
    }

    /// D26: the nine `outputState` identifiers, and nothing else — a bare
    /// `STARTED`, another case, an empty string — is `Unknown` (the
    /// fixture's `followTable` has those rows).
    #[test]
    fn output_states_read_only_the_nine() {
        use OutputState::{
            Paused, Reconnected, Reconnecting, Resumed, Started, Starting, Stopped, Stopping,
            Unknown,
        };
        for (word, state) in [
            ("OBS_WEBSOCKET_OUTPUT_UNKNOWN", Unknown),
            ("OBS_WEBSOCKET_OUTPUT_STARTING", Starting),
            ("OBS_WEBSOCKET_OUTPUT_STARTED", Started),
            ("OBS_WEBSOCKET_OUTPUT_STOPPING", Stopping),
            ("OBS_WEBSOCKET_OUTPUT_STOPPED", Stopped),
            ("OBS_WEBSOCKET_OUTPUT_RECONNECTING", Reconnecting),
            ("OBS_WEBSOCKET_OUTPUT_RECONNECTED", Reconnected),
            ("OBS_WEBSOCKET_OUTPUT_PAUSED", Paused),
            ("OBS_WEBSOCKET_OUTPUT_RESUMED", Resumed),
            ("STARTED", Unknown),
            ("obs_websocket_output_started", Unknown),
            ("", Unknown),
        ] {
            assert_eq!(OutputState::from_wire(word), state, "{word}");
        }
    }

    /// P576:200-210, Identify's example, minified: its `authentication`
    /// matches no password (§7.12) and its mask is 33, so the tests swap in
    /// their own value and 69 — the frame's bytes are otherwise P576's.
    const P576_IDENTIFY: &str = r#"{"op":1,"d":{"rpcVersion":1,"authentication":"Dj6cLS+jrNA0HpCArRg0Z/Fc+YHdt2FQfAvgD1mip6Y=","eventSubscriptions":33}}"#;

    /// D11, §5.2: Identify byte for byte, built from P576's example (K1):
    /// `rpcVersion` 1, then `authentication` only when Hello asked, then the
    /// mask, always 69. A Hello that announces RPC 2 changes none of it.
    #[test]
    fn identify_is_exact() {
        let f = fixture();
        let auth = f["authVectors"][0]["authentication"].as_str().unwrap();
        let ours = P576_IDENTIFY
            .replace("Dj6cLS+jrNA0HpCArRg0Z/Fc+YHdt2FQfAvgD1mip6Y=", auth)
            .replace("\"eventSubscriptions\":33", "\"eventSubscriptions\":69");
        assert_eq!(identify_json(Some(auth)), ours);
        // No auth asked: the example less its `authentication` key.
        let bare = ours.replace(&format!(",\"authentication\":\"{auth}\""), "");
        assert_eq!(
            bare,
            r#"{"op":1,"d":{"rpcVersion":1,"eventSubscriptions":69}}"#
        );
        assert_eq!(identify_json(None), bare);

        // Keys, read back: exactly these, no empty `authentication`.
        let keys = |frame: &str| {
            let v: Value = serde_json::from_str(frame).unwrap();
            assert_eq!(v["op"], 1);
            let mut k: Vec<String> = v["d"].as_object().unwrap().keys().cloned().collect();
            k.sort();
            k
        };
        assert_eq!(
            keys(&identify_json(Some(auth))),
            ["authentication", "eventSubscriptions", "rpcVersion"]
        );
        assert_eq!(
            keys(&identify_json(None)),
            ["eventSubscriptions", "rpcVersion"]
        );

        // A Hello announcing RPC 2 passes the gate, and Divora still
        // Identifies with 1 (a 4010 would then mean `needsNewerDivora`).
        let hello =
            parse_hello(r#"{"op":0,"d":{"obsWebSocketVersion":"6.0.0","rpcVersion":2}}"#).unwrap();
        assert_eq!(hello.rpc_version, 2);
        assert_eq!(gate(&hello), Ok(()));
        let sent: Value = serde_json::from_str(&identify_json(None)).unwrap();
        assert_eq!(sent["d"]["rpcVersion"], RPC_VERSION);
        assert_eq!(sent["d"]["rpcVersion"], 1);
        assert_eq!(sent["d"]["eventSubscriptions"], 69);

        // Escaped as JSON, whatever it holds (an auth string never needs it).
        let odd = "a\"b\\c";
        let sent: Value = serde_json::from_str(&identify_json(Some(odd))).unwrap();
        assert_eq!(sent["d"]["authentication"], odd);
    }

    /// The test's request id: P576:316's example.
    const P576_REQUEST_ID: &str = "f819dcf0-89cc-11eb-8f0e-382c4ac93b9c";

    /// A scene's and an input's uuid, as OBS writes them (36 characters).
    const SCENE_UUID: &str = "5d3f1c9e-7a2b-4e6d-8f10-2c4b6a8e0d13";
    const INPUT_UUID: &str = "a7e2c94b-1f3d-4b8a-9c6e-0d5f7b2e4a91";

    /// D9, §5.2: each of the nine requests, byte for byte. The expected
    /// frames are written out from P576's Request example (P576:310-320:
    /// `requestType`, `requestId`, `requestData`) and each request's own
    /// field table (P576:3496-3499 `SetCurrentProgramScene`, :3817-3820
    /// `GetInputSettings`, :3841-3846 `SetInputSettings`), with the uuid in place
    /// of a name — never both — and `overlay: true`.
    #[test]
    fn the_nine_requests_are_exact() {
        let f = fixture();
        assert_eq!(f["requests"], json!(REQUEST_TYPES));
        let id = P576_REQUEST_ID;
        let rows: [(Request, String); 9] = [
            (
                Request::GetVersion,
                format!(r#"{{"op":6,"d":{{"requestType":"GetVersion","requestId":"{id}"}}}}"#),
            ),
            (
                Request::GetSceneList,
                format!(r#"{{"op":6,"d":{{"requestType":"GetSceneList","requestId":"{id}"}}}}"#),
            ),
            (
                Request::GetCurrentProgramScene,
                format!(
                    r#"{{"op":6,"d":{{"requestType":"GetCurrentProgramScene","requestId":"{id}"}}}}"#
                ),
            ),
            (
                Request::SetCurrentProgramScene {
                    uuid: SCENE_UUID.into(),
                },
                format!(
                    r#"{{"op":6,"d":{{"requestType":"SetCurrentProgramScene","requestId":"{id}","requestData":{{"sceneUuid":"{SCENE_UUID}"}}}}}}"#
                ),
            ),
            (
                Request::GetInputList,
                format!(r#"{{"op":6,"d":{{"requestType":"GetInputList","requestId":"{id}"}}}}"#),
            ),
            (
                Request::GetInputSettings {
                    uuid: INPUT_UUID.into(),
                },
                format!(
                    r#"{{"op":6,"d":{{"requestType":"GetInputSettings","requestId":"{id}","requestData":{{"inputUuid":"{INPUT_UUID}"}}}}}}"#
                ),
            ),
            (
                Request::SetText {
                    uuid: INPUT_UUID.into(),
                    text: "04:59".into(),
                },
                format!(
                    r#"{{"op":6,"d":{{"requestType":"SetInputSettings","requestId":"{id}","requestData":{{"inputUuid":"{INPUT_UUID}","inputSettings":{{"text":"04:59"}},"overlay":true}}}}}}"#
                ),
            ),
            (
                Request::GetStreamStatus,
                format!(r#"{{"op":6,"d":{{"requestType":"GetStreamStatus","requestId":"{id}"}}}}"#),
            ),
            (
                Request::GetRecordStatus,
                format!(r#"{{"op":6,"d":{{"requestType":"GetRecordStatus","requestId":"{id}"}}}}"#),
            ),
        ];
        for (request, want) in &rows {
            // A variant added to `Request` fails to compile here until it
            // has a row (and the fixture a tenth name).
            let _: () = match request {
                Request::GetVersion
                | Request::GetSceneList
                | Request::GetCurrentProgramScene
                | Request::SetCurrentProgramScene { .. }
                | Request::GetInputList
                | Request::GetInputSettings { .. }
                | Request::SetText { .. }
                | Request::GetStreamStatus
                | Request::GetRecordStatus => (),
            };
            let frame = request.request_json(id);
            assert_eq!(&frame, want, "{request:?}");
            // Never a name beside (or instead of) a uuid.
            let v: Value = serde_json::from_str(&frame).unwrap();
            for name in ["sceneName", "inputName"] {
                assert!(v["d"]["requestData"].get(name).is_none(), "{request:?}");
            }
        }
        let kinds: Vec<RequestKind> = rows.iter().map(|(r, _)| r.kind()).collect();
        assert_eq!(kinds, RequestKind::ALL);

        // The text and the id are JSON-escaped: quotes, backslashes, control
        // characters and non-ASCII text arrive as sent.
        let text = "a \"quoted\" \\ back\nslash ✓ \u{1}";
        let frame = Request::SetText {
            uuid: INPUT_UUID.into(),
            text: text.into(),
        }
        .request_json("id \"1\"");
        let v: Value = serde_json::from_str(&frame).unwrap();
        assert_eq!(v["d"]["requestData"]["inputSettings"]["text"], text);
        assert_eq!(v["d"]["requestId"], "id \"1\"");
        assert_eq!(v["d"]["requestData"]["overlay"], true);
    }

    /// D9 (mutation round): nothing is built beside the nine: the variants
    /// of `RequestKind` and `Request`, read from this file, are the nine
    /// (`SetText` is `SetInputSettings`). A tenth given an arm in
    /// `the_nine_requests_are_exact`'s match and never sent by a link test
    /// (whose fake answers 204 and fails it) passes every row there.
    #[test]
    fn only_the_nine_requests_are_built() {
        let src = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/obs/proto.rs"),
        )
        .unwrap()
        .replace('\r', "");
        let variants = |head: &str| -> Vec<String> {
            let body = &src[src.find(head).unwrap() + head.len()..];
            body[..body.find("\n}\n").unwrap()]
                .lines()
                .map(str::trim)
                .filter(|l| !l.starts_with("//"))
                .filter_map(|l| l.split([',', ' ', '{']).next())
                .filter(|w| w.starts_with(|c: char| c.is_ascii_uppercase()))
                .map(String::from)
                .collect()
        };
        assert_eq!(variants("pub enum RequestKind {"), REQUEST_TYPES);
        let mut built = REQUEST_TYPES.map(String::from);
        built[6] = "SetText".into();
        assert_eq!(variants("pub enum Request {"), built);
    }

    /// The fixture's `gateTable` row's Hello, as a frame.
    fn hello_frame(d: &Value) -> String {
        json!({"op": 0, "d": d}).to_string()
    }

    /// D11, §5.2: Hellos as each 5.x doc shows them (P540:147-171 has no
    /// `obsStudioVersion`; P576:150-180 has it), with and without
    /// `authentication`; an RPC 2 Hello; a numeric or absent version kept as
    /// sent for the gate to refuse; and the shapes that aren't a Hello.
    #[test]
    #[allow(clippy::too_many_lines)] // one Hello per row
    fn hello_reads_every_5x_shape() {
        let challenge = "+IxH4CnCiqpX1rM9scsNynZzbOe4KhDeYcTNS3PDaeY=";
        let salt = "lM1GncleQOaCu9lT1yeUZhFYnqhsLLP1G5lAGo3ixaI=";
        let asked = Some(AuthChallenge {
            challenge: challenge.into(),
            salt: salt.into(),
        });
        // P576:150-163, as printed.
        let p576_auth = r#"{
  "op": 0,
  "d": {
    "obsStudioVersion": "30.2.2",
    "obsWebSocketVersion": "5.5.2",
    "rpcVersion": 1,
    "authentication": {
      "challenge": "+IxH4CnCiqpX1rM9scsNynZzbOe4KhDeYcTNS3PDaeY=",
      "salt": "lM1GncleQOaCu9lT1yeUZhFYnqhsLLP1G5lAGo3ixaI="
    }
  }
}"#;
        assert_eq!(
            parse_hello(p576_auth),
            Ok(Hello {
                rpc_version: 1,
                ws_version: json!("5.5.2"),
                obs_version: Some("30.2.2".into()),
                auth: asked.clone(),
            })
        );
        // P576:167-176.
        let p576_open = r#"{
  "op": 0,
  "d": {
    "obsStudioVersion": "30.2.2",
    "obsWebSocketVersion": "5.5.2",
    "rpcVersion": 1
  }
}"#;
        let open = parse_hello(p576_open).unwrap();
        assert_eq!(open.auth, None);
        assert_eq!(open.obs_version.as_deref(), Some("30.2.2"));
        // P540:148-158: no `obsStudioVersion` before 5.6.
        let p540_auth = r#"{
  "op": 0,
  "d": {
    "obsWebSocketVersion": "5.1.0",
    "rpcVersion": 1,
    "authentication": {
      "challenge": "+IxH4CnCiqpX1rM9scsNynZzbOe4KhDeYcTNS3PDaeY=",
      "salt": "lM1GncleQOaCu9lT1yeUZhFYnqhsLLP1G5lAGo3ixaI="
    }
  }
}"#;
        assert_eq!(
            parse_hello(p540_auth),
            Ok(Hello {
                rpc_version: 1,
                ws_version: json!("5.1.0"),
                obs_version: None,
                auth: asked,
            })
        );
        // P540:164-170.
        let p540_open = r#"{"op":0,"d":{"obsWebSocketVersion":"5.1.0","rpcVersion":1}}"#;
        let open = parse_hello(p540_open).unwrap();
        assert_eq!((open.auth, open.obs_version), (None, None));
        // 5.1.0 is below the floor: refused before any Identify.
        assert_eq!(gate(&parse_hello(p540_open).unwrap()), Err(Stop::TooOld));

        // RPC 2, unknown fields, a numeric and an absent version: read, the
        // version kept as sent.
        let two = parse_hello(
            r#"{"op":0,"d":{"rpcVersion":2,"obsWebSocketVersion":"6.0.0","later":{"x":1}},"extra":true}"#,
        )
        .unwrap();
        assert_eq!(two.rpc_version, 2);
        let numeric =
            parse_hello(r#"{"op":0,"d":{"rpcVersion":1,"obsWebSocketVersion":5.4}}"#).unwrap();
        assert_eq!(numeric.ws_version, json!(5.4));
        assert_eq!(
            gate(&numeric),
            Err(Stop::Unsupported(Unsupported::BadVersion))
        );
        let absent = parse_hello(r#"{"op":0,"d":{"rpcVersion":1}}"#).unwrap();
        assert_eq!(absent.ws_version, Value::Null);
        // A challenge and a salt of 128 characters each are read.
        let long = "a".repeat(128);
        let at_limit = json!({"op": 0, "d": {"rpcVersion": 1, "obsWebSocketVersion": "5.5.2",
            "authentication": {"challenge": long, "salt": long}}});
        assert!(parse_hello(&at_limit.to_string()).unwrap().auth.is_some());

        // Not a Hello: `notObs`, retried.
        let too_long = "a".repeat(129);
        for text in [
            r#"{"op":0,"d":{"obsWebSocketVersion":"5.7.6"}}"#.to_string(),
            r#"{"op":0,"d":{"rpcVersion":0,"obsWebSocketVersion":"5.7.6"}}"#.into(),
            r#"{"op":0,"d":{"rpcVersion":-1,"obsWebSocketVersion":"5.7.6"}}"#.into(),
            r#"{"op":0,"d":{"rpcVersion":"1","obsWebSocketVersion":"5.7.6"}}"#.into(),
            r#"{"op":0,"d":{"rpcVersion":1.5,"obsWebSocketVersion":"5.7.6"}}"#.into(),
            r#"{"op":0,"d":{"rpcVersion":null}}"#.into(),
            r#"{"op":2,"d":{"negotiatedRpcVersion":1}}"#.into(),
            r#"{"d":{"rpcVersion":1}}"#.into(),
            r#"{"op":"0","d":{"rpcVersion":1}}"#.into(),
            r#"{"op":0}"#.into(),
            r#"{"op":0,"d":[1]}"#.into(),
            r#"{"op":0,"d":{"rpcVersion":1,"authentication":{"challenge":"c"}}}"#.into(),
            r#"{"op":0,"d":{"rpcVersion":1,"authentication":{"challenge":"c","salt":5}}}"#.into(),
            r#"{"op":0,"d":{"rpcVersion":1,"authentication":"c"}}"#.into(),
            r#"{"op":0,"d":{"rpcVersion":1,"authentication":null}}"#.into(),
            json!({"op": 0, "d": {"rpcVersion": 1,
                "authentication": {"challenge": too_long, "salt": "s"}}})
            .to_string(),
            json!({"op": 0, "d": {"rpcVersion": 1,
                "authentication": {"challenge": "c", "salt": too_long}}})
            .to_string(),
            "[0]".into(),
            "Hello".into(),
            String::new(),
        ] {
            assert_eq!(parse_hello(&text), Err(ProtoError), "{text}");
        }
    }

    /// §7.2 (fixture `gateTable`): `parse_hello` then `gate`. A Hello
    /// announcing RPC 2 passes; a numeric version is `badVersion`, not "not
    /// a Hello"; 5.4.0 itself passes.
    #[test]
    fn gate_table() {
        let f = fixture();
        let rows = f["gateTable"].as_array().unwrap();
        assert_eq!(rows.len(), 22);
        for row in rows {
            let got = match parse_hello(&hello_frame(&row["hello"])) {
                Err(ProtoError) => "notObs",
                Ok(h) => match gate(&h) {
                    Ok(()) => "pass",
                    Err(Stop::TooOld) => "tooOld",
                    Err(Stop::Unsupported(Unsupported::BadVersion)) => "badVersion",
                    Err(other) => panic!("{other:?} from the gate"),
                },
            };
            assert_eq!(got, row["result"], "{}", row["hello"]);
        }
        // The leading `x.y.z`, a suffix dropped; nothing else is a version.
        for (v, want) in [
            (json!("5.4.0"), Some((5, 4, 0))),
            (json!("5.5.0-beta1"), Some((5, 5, 0))),
            (json!("5.10.0"), Some((5, 10, 0))),
            (json!("5.4.0.1"), Some((5, 4, 0))),
            (json!("0.0.0"), Some((0, 0, 0))),
            (json!("4294967295.0.0"), Some((u32::MAX, 0, 0))),
            (json!("4294967296.0.0"), None),
            (json!("v5.4.0"), None),
            (json!(" 5.4.0"), None),
            (json!("5.4"), None),
            (json!("5.4."), None),
            (json!("5..4.0"), None),
            (json!("-5.4.0"), None),
            (json!(""), None),
            (json!(5.4), None),
            (json!(["5.4.0"]), None),
            (Value::Null, None),
        ] {
            assert_eq!(ws_version(&v), want, "{v}");
        }
    }

    /// §7.1's close codes: 4009, 4010 and 4011 stop and are never retried;
    /// obs-websocket's other codes and RFC 6455's complaints are
    /// `protocol`; the rest, and no code at all, `closed`.
    #[test]
    fn close_outcome_table() {
        use CloseOutcome::{Retry, Stop as Stops};
        for (code, want) in [
            (Some(4009), Stops(Stop::AuthFailed)),
            (Some(4010), Stops(Stop::NeedsNewerDivora)),
            (Some(4011), Stops(Stop::Kicked)),
            (Some(4000), Retry(ObsReason::Protocol)),
            (Some(4002), Retry(ObsReason::Protocol)),
            (Some(4003), Retry(ObsReason::Protocol)),
            (Some(4004), Retry(ObsReason::Protocol)),
            (Some(4005), Retry(ObsReason::Protocol)),
            (Some(4006), Retry(ObsReason::Protocol)),
            (Some(4007), Retry(ObsReason::Protocol)),
            (Some(4008), Retry(ObsReason::Protocol)),
            (Some(4012), Retry(ObsReason::Protocol)),
            (Some(4999), Retry(ObsReason::Protocol)),
            (Some(1002), Retry(ObsReason::Protocol)),
            (Some(1003), Retry(ObsReason::Protocol)),
            (Some(1007), Retry(ObsReason::Protocol)),
            (Some(1008), Retry(ObsReason::Protocol)),
            (Some(1009), Retry(ObsReason::Protocol)),
            (Some(1010), Retry(ObsReason::Protocol)),
            (Some(1000), Retry(ObsReason::Closed)),
            (Some(1001), Retry(ObsReason::Closed)),
            (Some(1005), Retry(ObsReason::Closed)),
            (Some(1006), Retry(ObsReason::Closed)),
            (Some(1011), Retry(ObsReason::Closed)),
            (Some(1012), Retry(ObsReason::Closed)),
            (Some(3000), Retry(ObsReason::Closed)),
            (Some(5000), Retry(ObsReason::Closed)),
            (None, Retry(ObsReason::Closed)),
        ] {
            assert_eq!(close_outcome(code), want, "{code:?}");
        }
    }

    /// D10: what comes in. op 0, 2, 5 and 7 are read, each with `d` an
    /// object; every other op, shape and event is ignored. Of an event only
    /// its type and `outputState` are kept — the recording's path is never
    /// read — and of an answer its id, result, code and data, never OBS's
    /// `comment`.
    #[test]
    #[allow(clippy::too_many_lines)] // a row per frame
    fn incoming_table() {
        let event = |kind: &str, data: Value| {
            json!({"op": 5, "d": {"eventType": kind, "eventIntent": 4, "eventData": data}})
                .to_string()
        };
        let stream = |state: &str| {
            event(
                "StreamStateChanged",
                json!({"outputActive": false, "outputState": state}),
            )
        };
        let rows: Vec<(String, Incoming)> = vec![
            (
                // P576:222-239 has Identified; Hello's own parse is above.
                r#"{"op":0,"d":{"obsWebSocketVersion":"5.5.2","rpcVersion":1}}"#.into(),
                Incoming::Hello,
            ),
            (
                r#"{"op":2,"d":{"negotiatedRpcVersion":1}}"#.into(),
                Incoming::Identified,
            ),
            // ExitStarted has no `eventData` (WSP:368-369).
            (
                r#"{"op":5,"d":{"eventType":"ExitStarted","eventIntent":1}}"#.into(),
                Incoming::Event(ObsEvent::ExitStarted),
            ),
            (
                event(
                    "SceneCreated",
                    json!({"sceneName": "BRB", "sceneUuid": SCENE_UUID, "isGroup": false}),
                ),
                Incoming::Event(ObsEvent::SceneCreated),
            ),
            (
                event(
                    "SceneRemoved",
                    json!({"sceneName": "BRB", "sceneUuid": SCENE_UUID, "isGroup": false}),
                ),
                Incoming::Event(ObsEvent::SceneRemoved),
            ),
            (
                event(
                    "SceneNameChanged",
                    json!({"sceneUuid": SCENE_UUID, "oldSceneName": "BRB", "sceneName": "Away"}),
                ),
                Incoming::Event(ObsEvent::SceneNameChanged),
            ),
            (
                stream("OBS_WEBSOCKET_OUTPUT_STARTED"),
                Incoming::Event(ObsEvent::Stream(OutputState::Started)),
            ),
            (
                stream("OBS_WEBSOCKET_OUTPUT_RECONNECTING"),
                Incoming::Event(ObsEvent::Stream(OutputState::Reconnecting)),
            ),
            (
                stream("STARTED"),
                Incoming::Event(ObsEvent::Stream(OutputState::Unknown)),
            ),
            (
                event("StreamStateChanged", json!({"outputActive": true})),
                Incoming::Event(ObsEvent::Stream(OutputState::Unknown)),
            ),
            (
                event(
                    "RecordStateChanged",
                    json!({"outputActive": false, "outputState": "OBS_WEBSOCKET_OUTPUT_STOPPED",
                        "outputPath": "C:\\Users\\zz-user\\Videos\\take.mkv"}),
                ),
                Incoming::Event(ObsEvent::Record(OutputState::Stopped)),
            ),
            (
                event(
                    "RecordStateChanged",
                    json!({"outputActive": false, "outputState": "OBS_WEBSOCKET_OUTPUT_PAUSED",
                        "outputPath": null}),
                ),
                Incoming::Event(ObsEvent::Record(OutputState::Paused)),
            ),
            // Events Divora doesn't read, under mask 69 or not.
            (
                event(
                    "CurrentProgramSceneChanged",
                    json!({"sceneName": "BRB", "sceneUuid": SCENE_UUID}),
                ),
                Incoming::Ignored,
            ),
            (
                event(
                    "InputSettingsChanged",
                    json!({"inputSettings": {"text": "zz-source-text-zz"}}),
                ),
                Incoming::Ignored,
            ),
            (
                event("StudioModeStateChanged", json!({"studioModeEnabled": true})),
                Incoming::Ignored,
            ),
            (
                r#"{"op":5,"d":{"eventIntent":1}}"#.into(),
                Incoming::Ignored,
            ),
            // P576:362-376, a success with no `responseData`.
            (
                format!(
                    r#"{{"op":7,"d":{{"requestType":"SetCurrentProgramScene","requestId":"{P576_REQUEST_ID}","requestStatus":{{"result":true,"code":100}}}}}}"#
                ),
                Incoming::Response {
                    id: P576_REQUEST_ID.into(),
                    ok: true,
                    code: 100,
                    data: None,
                },
            ),
            // P576:380-393, a failure with OBS's comment: dropped.
            (
                format!(
                    r#"{{"op":7,"d":{{"requestType":"SetCurrentProgramScene","requestId":"{P576_REQUEST_ID}","requestStatus":{{"result":false,"code":608,"comment":"Parameter: sceneName"}}}}}}"#
                ),
                Incoming::Response {
                    id: P576_REQUEST_ID.into(),
                    ok: false,
                    code: 608,
                    data: None,
                },
            ),
            (
                json!({"op": 7, "d": {"requestType": "GetCurrentProgramScene", "requestId": "r1",
                    "requestStatus": {"result": true, "code": 100},
                    "responseData": {"sceneName": "Game", "sceneUuid": SCENE_UUID}}})
                .to_string(),
                Incoming::Response {
                    id: "r1".into(),
                    ok: true,
                    code: 100,
                    data: Some(json!({"sceneName": "Game", "sceneUuid": SCENE_UUID})),
                },
            ),
            (
                json!({"op": 7, "d": {"requestId": "r2",
                    "requestStatus": {"result": true, "code": 100}, "responseData": null}})
                .to_string(),
                Incoming::Response {
                    id: "r2".into(),
                    ok: true,
                    code: 100,
                    data: None,
                },
            ),
            // An unreadable status still reaches its request, as a failure.
            (
                r#"{"op":7,"d":{"requestId":"r3"}}"#.into(),
                Incoming::Response {
                    id: "r3".into(),
                    ok: false,
                    code: 0,
                    data: None,
                },
            ),
            (
                r#"{"op":7,"d":{"requestId":"r4","requestStatus":{"result":"yes","code":"100"}}}"#
                    .into(),
                Incoming::Response {
                    id: "r4".into(),
                    ok: false,
                    code: 0,
                    data: None,
                },
            ),
            // No string id: nothing to route it to.
            (
                r#"{"op":7,"d":{"requestStatus":{"result":true,"code":100}}}"#.into(),
                Incoming::Ignored,
            ),
            (
                r#"{"op":7,"d":{"requestId":7,"requestStatus":{"result":true,"code":100}}}"#.into(),
                Incoming::Ignored,
            ),
            // Other ops and shapes.
            (r#"{"op":1,"d":{"rpcVersion":1}}"#.into(), Incoming::Ignored),
            (
                r#"{"op":3,"d":{"eventSubscriptions":69}}"#.into(),
                Incoming::Ignored,
            ),
            (
                r#"{"op":6,"d":{"requestType":"GetVersion","requestId":"x"}}"#.into(),
                Incoming::Ignored,
            ),
            (r#"{"op":8,"d":{}}"#.into(), Incoming::Ignored),
            (r#"{"op":9,"d":{}}"#.into(), Incoming::Ignored),
            (r#"{"op":42,"d":{}}"#.into(), Incoming::Ignored),
            (
                r#"{"op":"7","d":{"requestId":"x"}}"#.into(),
                Incoming::Ignored,
            ),
            (r#"{"op":2}"#.into(), Incoming::Ignored),
            (r#"{"op":5,"d":"ExitStarted"}"#.into(), Incoming::Ignored),
            ("[5]".into(), Incoming::Ignored),
            ("not json".into(), Incoming::Ignored),
            (String::new(), Incoming::Ignored),
        ];
        for (text, want) in rows {
            let got = parse_incoming(&text);
            // What OBS wrote in a comment or a recording's path is never
            // kept.
            let kept = format!("{got:?}");
            assert!(!kept.contains("zz-user"), "{text}");
            assert!(!kept.contains("Parameter"), "{text}");
            assert!(!kept.contains("zz-source-text-zz"), "{text}");
            assert_eq!(got, want, "{text}");
        }
    }

    /// §5.2's readers, on answers shaped as P576's response tables
    /// (P576:2840-2850, :3435-3441, :3477-3482, :3666-3670, :3823-3826,
    /// :5376-5385, :5451-5457) and ARR's items; an item a reader can't use
    /// is skipped, not the list.
    #[test]
    #[allow(clippy::too_many_lines)] // one answer per request
    fn responses_table() {
        let scene = |name: &str, uuid: &str| SceneRef {
            uuid: uuid.into(),
            name: name.into(),
        };
        // GetVersion.
        let answer = json!({"obsVersion": "30.2.2", "obsWebSocketVersion": "5.5.2",
            "rpcVersion": 1, "availableRequests": ["GetVersion", "GetSceneList", 7, null],
            "supportedImageFormats": ["png"], "platform": "windows",
            "platformDescription": "Windows 11 (10.0)"});
        assert_eq!(
            version(&answer),
            (
                Some("30.2.2".to_string()),
                vec!["GetVersion".to_string(), "GetSceneList".to_string()]
            )
        );
        assert_eq!(version(&json!({})), (None, Vec::new()));

        // GetSceneList: ARR sends the bottom scene first with sceneIndex 0;
        // Divora lists the top first.
        let long = "u".repeat(129);
        let answer = json!({
            "currentProgramSceneName": "Game", "currentProgramSceneUuid": "u-game",
            "currentPreviewSceneName": null, "currentPreviewSceneUuid": null,
            "scenes": [
                {"sceneName": "Just Chatting", "sceneUuid": "u-chat", "sceneIndex": 0},
                {"sceneName": "BRB", "sceneUuid": "u-brb", "sceneIndex": 1},
                {"sceneName": "no uuid", "sceneIndex": 2},
                {"sceneName": 5, "sceneUuid": "u-five", "sceneIndex": 3},
                {"sceneName": "empty uuid", "sceneUuid": "", "sceneIndex": 4},
                {"sceneName": "long uuid", "sceneUuid": long, "sceneIndex": 5},
                "not a scene",
                {"sceneName": "Game", "sceneUuid": "u-game", "sceneIndex": 6},
            ],
        });
        assert_eq!(
            scene_list(&answer),
            (
                Some(scene("Game", "u-game")),
                vec![
                    scene("Game", "u-game"),
                    scene("BRB", "u-brb"),
                    scene("Just Chatting", "u-chat"),
                ]
            )
        );
        // The program can be null (a desync, a non-main canvas); a scene
        // without an index keeps the reversed list's place, after the rest.
        let answer = json!({
            "currentProgramSceneName": null, "currentProgramSceneUuid": null,
            "scenes": [
                {"sceneName": "B", "sceneUuid": "u-b"},
                {"sceneName": "A", "sceneUuid": "u-a"},
                {"sceneName": "C", "sceneUuid": "u-c", "sceneIndex": 0},
            ],
        });
        assert_eq!(
            scene_list(&answer),
            (
                None,
                vec![scene("C", "u-c"), scene("A", "u-a"), scene("B", "u-b")]
            )
        );
        assert_eq!(scene_list(&json!({})), (None, Vec::new()));

        // GetCurrentProgramScene: the 5.4+ pair, else the deprecated one.
        let both = json!({"sceneName": "BRB", "sceneUuid": "u-brb",
            "currentProgramSceneName": "Old", "currentProgramSceneUuid": "u-old"});
        assert_eq!(program_scene(&both), Some(scene("BRB", "u-brb")));
        let deprecated =
            json!({"currentProgramSceneName": "BRB", "currentProgramSceneUuid": "u-brb"});
        assert_eq!(program_scene(&deprecated), Some(scene("BRB", "u-brb")));
        // P533's shape: a name, no uuid — nothing Divora can switch by.
        assert_eq!(
            program_scene(&json!({"currentProgramSceneName": "BRB"})),
            None
        );
        assert_eq!(program_scene(&json!({"sceneUuid": "u-brb"})), None);

        // GetInputList: ARR's items (ARR:220-225); only text sources, in
        // OBS's order, the kind from `unversionedInputKind` or the suffix.
        let answer = json!({"inputs": [
            {"inputName": "Timer", "inputUuid": "u-1", "inputKind": "text_gdiplus_v3",
                "unversionedInputKind": "text_gdiplus", "inputKindCaps": 0},
            {"inputName": "Cam", "inputUuid": "u-2", "inputKind": "dshow_input",
                "unversionedInputKind": "dshow_input", "inputKindCaps": 0},
            {"inputName": "Free", "inputUuid": "u-3", "inputKind": "text_ft2_source_v2",
                "unversionedInputKind": "text_ft2_source", "inputKindCaps": 0},
            {"inputName": "Old", "inputUuid": "u-4", "inputKind": "text_gdiplus_v3"},
            {"inputName": "Web", "inputUuid": "u-5", "inputKind": "browser_source"},
            {"inputName": "No uuid", "inputKind": "text_gdiplus_v3"},
            {"inputName": 9, "inputUuid": "u-6", "inputKind": "text_gdiplus_v3"},
            {"inputName": "Bare", "inputUuid": "u-7", "inputKind": "text_gdiplus",
                "unversionedInputKind": null},
        ]});
        let source = |name: &str, uuid: &str, kind| TextSourceRef {
            uuid: uuid.into(),
            name: name.into(),
            kind,
        };
        assert_eq!(
            input_list(&answer),
            vec![
                source("Timer", "u-1", TextKind::Gdiplus),
                source("Free", "u-3", TextKind::Ft2),
                source("Old", "u-4", TextKind::Gdiplus),
                source("Bare", "u-7", TextKind::Gdiplus),
            ]
        );
        assert_eq!(input_list(&json!({"inputs": "none"})), Vec::new());

        // GetInputSettings: only the file flag is read.
        let answer = json!({"inputKind": "text_gdiplus_v3", "inputSettings": {
            "text": "zz-source-text-zz", "file": "C:\\Users\\zz-user\\a.txt",
            "read_from_file": true}});
        assert!(reads_from_file(TextKind::Gdiplus, &answer["inputSettings"]));
        assert!(!reads_from_file(TextKind::Ft2, &answer["inputSettings"]));

        // GetStreamStatus and GetRecordStatus.
        let answer = json!({"outputActive": true, "outputReconnecting": false,
            "outputTimecode": "00:02:05.000", "outputDuration": 125_000,
            "outputCongestion": 0.0, "outputBytes": 1000, "outputSkippedFrames": 0,
            "outputTotalFrames": 7500});
        assert_eq!(
            stream_status(&answer),
            OutputStatus {
                active: true,
                reconnecting: false,
                paused: false,
                duration_ms: 125_000,
            }
        );
        let answer = json!({"outputActive": true, "outputReconnecting": true,
            "outputDuration": 0, "outputPaused": true});
        assert_eq!(
            stream_status(&answer),
            OutputStatus {
                active: true,
                reconnecting: true,
                paused: false,
                duration_ms: 0,
            }
        );
        let answer = json!({"outputActive": true, "outputPaused": true,
            "outputTimecode": "00:00:01.500", "outputDuration": 1_500.6, "outputBytes": 10,
            "outputReconnecting": true});
        assert_eq!(
            record_status(&answer),
            OutputStatus {
                active: true,
                reconnecting: false,
                paused: true,
                duration_ms: 1_501,
            }
        );
        // Unreadable fields: off, 0 ms.
        for answer in [
            json!({}),
            json!({"outputActive": "yes", "outputPaused": 1, "outputDuration": -5}),
            json!({"outputActive": null, "outputDuration": "125000"}),
        ] {
            assert_eq!(record_status(&answer), OutputStatus::default(), "{answer}");
            assert_eq!(stream_status(&answer), OutputStatus::default(), "{answer}");
        }
    }

    /// §7.11 (fixture `textKindTable`): which inputs are text sources.
    #[test]
    fn text_kind_table() {
        let f = fixture();
        let rows = f["textKindTable"].as_array().unwrap();
        assert_eq!(rows.len(), 11);
        for row in rows {
            let got = text_kind(
                row["inputKind"].as_str().unwrap(),
                row["unversionedInputKind"].as_str(),
            );
            let want = kind_of(&row["kind"]);
            assert_eq!(got, want, "{row}");
        }
        // The suffix is `_v` and digits only, and only at the end.
        for (kind, want) in [
            ("text_gdiplus_v12", Some(TextKind::Gdiplus)),
            ("text_gdiplus_v", None),
            ("text_gdiplus_vx", None),
            ("text_gdiplus_v3_v2", None),
            ("text_gdiplus", Some(TextKind::Gdiplus)),
            ("TEXT_GDIPLUS", None),
            ("", None),
        ] {
            assert_eq!(text_kind(kind, None), want, "{kind}");
        }
        // `unversionedInputKind`, when there, decides.
        assert_eq!(text_kind("text_gdiplus_v3", Some("browser_source")), None);
        assert_eq!(text_kind("", Some("text_ft2_source")), Some(TextKind::Ft2));
    }

    /// §7.11 (fixture `readsFromFileTable`): each kind reads only its own
    /// flag, and only JSON `true` counts.
    #[test]
    fn reads_from_file_table() {
        let f = fixture();
        let rows = f["readsFromFileTable"].as_array().unwrap();
        assert_eq!(rows.len(), 10);
        for row in rows {
            let kind = kind_of(&row["kind"]).unwrap();
            assert_eq!(
                reads_from_file(kind, &row["settings"]),
                row["readsFromFile"].as_bool().unwrap(),
                "{row}"
            );
        }
        assert!(!reads_from_file(
            TextKind::Gdiplus,
            &json!("read_from_file")
        ));
        assert!(!reads_from_file(TextKind::Ft2, &Value::Null));
    }

    /// The fixture's `kind` word (`textSourceKinds`), `null` for none.
    fn kind_of(v: &Value) -> Option<TextKind> {
        match v.as_str() {
            Some("gdiplus") => Some(TextKind::Gdiplus),
            Some("ft2") => Some(TextKind::Ft2),
            None => None,
            Some(other) => panic!("kind {other}"),
        }
    }
}
