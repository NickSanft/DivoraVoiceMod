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

use super::machine::{ObsReason, OutputStatus, Stop, TextKind};
use super::scenes::SceneRef;

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

/// D11: op 0 with `d` an object, `rpcVersion` an unsigned integer ≥ 1, and
/// `authentication` absent or `{challenge, salt}`, strings of at most 128
/// characters; anything else isn't a Hello.
#[allow(unused_variables)] // S1: track A writes this body
pub fn parse_hello(text: &str) -> Result<Hello, ProtoError> {
    todo!("S1: track A")
}

/// D11, §7.2, before any Identify: a version that isn't a string with a
/// leading `x.y.z` is `unsupported` (`badVersion`); below [`FLOOR`],
/// `tooOld`; else `Ok`. `rpc_version` isn't compared.
#[allow(unused_variables)] // S1: track A writes this body
pub fn gate(h: &Hello) -> Result<(), Stop> {
    todo!("S1: track A")
}

/// D11: a version's leading `x.y.z` (a `-…` suffix dropped); `None` for
/// anything else, a number included.
#[allow(unused_variables)] // S1: track A writes this body
pub fn ws_version(v: &Value) -> Option<WsVersion> {
    todo!("S1: track A")
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
/// "eventSubscriptions":69}}`, `authentication` left out with `None`.
#[allow(unused_variables)] // S1: track A writes this body
pub fn identify_json(auth: Option<&str>) -> String {
    todo!("S1: track A")
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
    /// `requestData` left out when there is none.
    #[allow(unused_variables, clippy::unused_self)] // S1: track A writes this body
    pub fn request_json(&self, id: &str) -> String {
        todo!("S1: track A")
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

/// D10: op 0, 2, 5 and 7; everything else [`Incoming::Ignored`].
#[allow(unused_variables)] // S1: track A writes this body
pub fn parse_incoming(text: &str) -> Incoming {
    todo!("S1: track A")
}

/// §7.1: how a close code ends the link: a stop or a retry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CloseOutcome {
    Stop(Stop),
    Retry(ObsReason),
}

/// §7.1's close rows: 4009, 4010 and 4011 stop; every other code — none
/// included — retries.
#[allow(unused_variables)] // S1: track A writes this body
pub fn close_outcome(code: Option<u16>) -> CloseOutcome {
    todo!("S1: track A")
}

// ---- Answers (§5.2): each skips an item it can't read ---------------------------

/// `GetVersion`: `obsVersion` (unsanitized) and `availableRequests`.
#[allow(unused_variables)] // S1: track A writes this body
pub fn version(data: &Value) -> (Option<String>, Vec<String>) {
    todo!("S1: track A")
}

/// `GetSceneList`: the program scene and every scene, top first (`sceneIndex`
/// descending).
#[allow(unused_variables)] // S1: track A writes this body
pub fn scene_list(data: &Value) -> (Option<SceneRef>, Vec<SceneRef>) {
    todo!("S1: track A")
}

/// `GetCurrentProgramScene`: `sceneUuid`/`sceneName`, else the deprecated
/// `currentProgramScene*` pair.
#[allow(unused_variables)] // S1: track A writes this body
pub fn program_scene(data: &Value) -> Option<SceneRef> {
    todo!("S1: track A")
}

/// One input [`text_kind`] lists, from `GetInputList`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextSourceRef {
    pub uuid: String,
    pub name: String,
    pub kind: TextKind,
}

/// `GetInputList`: the text sources, in OBS's order.
#[allow(unused_variables)] // S1: track A writes this body
pub fn input_list(data: &Value) -> Vec<TextSourceRef> {
    todo!("S1: track A")
}

/// §7.11, D19: whether an input is a text source: `unversionedInputKind` —
/// or, absent, `inputKind` less a trailing `_v<digits>` — is
/// `text_gdiplus` or `text_ft2_source`.
#[allow(unused_variables)] // S1: track A writes this body
pub fn text_kind(input_kind: &str, unversioned: Option<&str>) -> Option<TextKind> {
    todo!("S1: track A")
}

/// §7.11, D19: `GetInputSettings`' `inputSettings` reads a file: GDI+'s
/// `read_from_file` or `FreeType` 2's `from_file` is `true`. Nothing else of
/// `settings` leaves the link.
#[allow(unused_variables)] // S1: track A writes this body
pub fn reads_from_file(kind: TextKind, settings: &Value) -> bool {
    todo!("S1: track A")
}

/// `GetStreamStatus`: `outputActive`, `outputReconnecting`, `outputDuration`.
#[allow(unused_variables)] // S1: track A writes this body
pub fn stream_status(data: &Value) -> OutputStatus {
    todo!("S1: track A")
}

/// `GetRecordStatus`: `outputActive`, `outputPaused`, `outputDuration`.
#[allow(unused_variables)] // S1: track A writes this body
pub fn record_status(data: &Value) -> OutputStatus {
    todo!("S1: track A")
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
        auth_string, OutputState, Request, RequestKind, EVENT_MASK, REQUEST_TYPES, RPC_VERSION,
    };
    use base64::engine::general_purpose::STANDARD;
    use base64::Engine as _;
    use serde_json::Value;
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
}
