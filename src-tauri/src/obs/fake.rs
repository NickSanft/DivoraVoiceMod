//! v1.61.0: a fake obs-websocket v5 server, for the link's tests only
//! (§10.2, track B2). Nothing here ships: the module is `#[cfg(test)]`.
//!
//! **Written by a different hand from the link (K1).** Everything below
//! comes from the protocol doc at 5.7.6 (P576 = the design scratchpad's
//! `obsproto/protocol-5.7.6.md`: the connection steps :48-80, the auth
//! string :84-104, the op codes :126-434, close codes :634-766, request
//! statuses :815-1165, event subscriptions :1167-1345, output states
//! :1417-1505, and the event and request sections cited at each handler)
//! and obs-websocket 5.7.6's own server sources under `obsws_src/`: WSS
//! (`WebSocketServer.cpp`), WSP (`WebSocketServer_Protocol.cpp`), CRY
//! (`Crypto.cpp`), RHS and RHI (`RequestHandler_Scenes.cpp`,
//! `RequestHandler_Inputs.cpp`), EH and EHO (`EventHandler.cpp`,
//! `EventHandler_Outputs.cpp`), ARR (`Obs_ArrayHelper.cpp`) and NUM
//! (`Obs_NumberHelper.cpp`). It reads none of `proto.rs`, `machine.rs`,
//! `scenes.rs`, `link.rs` or the obs fixture and imports nothing from them:
//! its request names, `availableRequests`, hashing and JSON are its own, so
//! one misreading of the protocol can't sit in both the link and its judge.
//! Where the sources in hand stop (Request.cpp, RequestHandler_General.cpp,
//! the stream and record handlers aren't in hand), a handler says so and
//! what it does instead. (In these docs a WSS cite sits in a code span:
//! clippy reads a bare `WSS:85` as a URL.)
//!
//! # Using it
//!
//! Every test builds its own current-thread runtime with every driver on
//! and a 10 s guard, which [`guarded`] does; the fake's tasks run on that
//! runtime, beside the link's.
//!
//! ```text
//! guarded(async {
//!     let fake = FakeObs::start(Config::default().with_password("pw")).await;
//!     // point the link at fake.addr() (127.0.0.1, a port the OS picked)
//!     assert!(fake.wait_identified(1, Duration::from_secs(2)).await);
//!     fake.script(Rule::on("SetInputSettings").times(1).delay(Duration::from_millis(600)));
//!     fake.stream_state(OutputState::Started);
//!     assert_eq!(fake.sessions()[0].subscriptions, Some(69));
//!     fake.assert_clean();
//! });
//! ```
//!
//! # The control API
//!
//! All of it is on [`FakeObs`], takes `&self`, never blocks, and applies at
//! once; a change to how connections start applies to the next one.
//!
//! | To | Call |
//! |---|---|
//! | start on 127.0.0.1:0 and learn the port | [`FakeObs::start`], [`FakeObs::addr`], [`FakeObs::port`] |
//! | Hello's versions, its `rpcVersion`, the accepted RPC versions, auth on or off | [`Config`] fields and builders; live: [`FakeObs::config`], [`FakeObs::set_password`] (a new salt, as a server restart makes), [`FakeObs::restart`] |
//! | "OBS down", a stalled upgrade, an endless header line | [`FakeObs::set_accept`] with [`Accept`] |
//! | delay or withhold Hello or Identified, or close instead | [`FakeObs::set_hello`], [`FakeObs::set_identified`] with [`Stage`] |
//! | delay, drop, fail (any code, 207 N times or forever), apply-but-never-answer, or close on a request | [`FakeObs::script`] with a [`Rule`]; [`FakeObs::clear_scripts`] |
//! | 207 to everything and no events (OBS not ready) | [`FakeObs::set_ready`] |
//! | close every connection, or one, with any code (4009, 4010, 4011, 1001, …) | [`FakeObs::close_all`], [`FakeObs::close_session`] |
//! | stop answering pings; neither echo a close nor close | [`FakeObs::set_ignore_pings`], [`FakeObs::set_close_echo`] |
//! | inject events, gated as OBS gates them | [`FakeObs::event`], [`FakeObs::exit_started`], [`FakeObs::switch_program`], [`FakeObs::stream_state`], [`FakeObs::record_state`], [`FakeObs::create_scene`], [`FakeObs::remove_scene`], [`FakeObs::rename_scene`], [`FakeObs::flood_scene_renames`] |
//! | `CurrentProgramSceneChanged` before (the default) or after a `SetCurrentProgramScene`'s answer | [`Config::program_event`] |
//! | an oversized frame, garbage, any text frame | [`FakeObs::send_big_frame`], [`FakeObs::send_raw`], [`FakeObs::send_text`] |
//! | read or change OBS's scenes, inputs and outputs mid-session | [`FakeObs::model`] ([`Model::standard`] is §10.2's collection) |
//! | read back every frame in and out, in order, with its time | [`FakeObs::log`] ([`Record`], [`Frame`]) |
//! | the requests, Identifies, connections, accepts, pongs | [`FakeObs::requests`], [`FakeObs::requests_of`], [`FakeObs::identifies`], [`FakeObs::sessions`], [`FakeObs::accepts`], [`FakeObs::pongs`] |
//! | wait for any of it | [`FakeObs::wait`] and the `wait_*` helpers |
//!
//! # What it counts as the link's fault
//!
//! Things a correct Divora link never does are recorded as violations
//! ([`FakeObs::violations`]; [`FakeObs::assert_clean`] fails the test on
//! any): a second request sent while one is unanswered (D9; also
//! [`FakeObs::max_outstanding`]), a request type outside [`REQUESTS`] or
//! left out of `availableRequests` (answered 204, P576:857-863), an
//! Identify whose `eventSubscriptions` isn't [`Config::expect_mask`] (69 by
//! default, D10), an Identify carrying `authentication` when Hello asked
//! for none or carrying fields Identify doesn't have (P576:188-193), a
//! request answered 3xx or 4xx (a malformed request), and every close OBS
//! itself would make for a broken client: 4002-4008, and 4010 for a
//! pre-5.0 message (`WSS:365-436`, WSP:72-353). The closes a correct link
//! can meet (4009 for a wrong password, 4010 for an RPC version the server
//! won't serve) aren't violations.
//!
//! # Where it differs from OBS on purpose
//!
//! It serves JSON only (no `obswebsocket.msgpack`, `WSS:208-245`). A
//! `RequestBatch` (op 8) is a violation closed with 4006, since the link
//! never batches. Events wait for nothing: the scripted
//! `CurrentProgramSceneChanged` order replaces OBS's thread-pool race
//! (WSP:355-411). Its own server's sockets set no-delay so a test's
//! timings aren't Nagle's.
//!
//! Like OBS it sends no event while not ready (WSP:359-360), and handles
//! what a client sends before a delayed Hello only after Hello has gone
//! out (OBS sends Hello before it handles any message, `WSS:219-294`).

// D39's denies are for the shipped link; this whole file is test code
// (mod.rs: "Test modules allow these"). A panic here fails a test, never
// the app.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic
)]

use std::future::{poll_fn, Future};
use std::io;
use std::net::{Ipv4Addr, SocketAddr};
use std::pin::{pin, Pin};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::task::{ready, Context, Poll};
use std::time::{Duration, Instant};

use base64::engine::general_purpose::STANDARD;
use base64::Engine as _;
use futures_core::Stream;
use futures_sink::Sink;
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, ReadBuf};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, Notify};
use tokio::task::JoinHandle;
use tokio_websockets::{CloseCode, Config as WsConfig, Message, ServerBuilder, WebSocketStream};

/// The requests this fake serves, typed from RHS, RHI and P576's request
/// sections. `GetVersion`'s `availableRequests` is built from this list
/// (less [`Config::unavailable`]); any other type is answered 204
/// `UnknownRequestType` (P576:857-863) and recorded as a violation.
pub const REQUESTS: [&str; 9] = [
    "GetVersion",             // P576:2830-2848
    "GetSceneList",           // RHS:22-78, P576:3419-3441
    "GetCurrentProgramScene", // RHS:108-135, P576:3463-3482
    "SetCurrentProgramScene", // RHS:137-167, P576:3486-3499
    "GetInputList",           // RHI:22-52, P576:3652-3670
    "GetInputSettings",       // RHI:296-328, P576:3805-3827
    "SetInputSettings",       // RHI:330-379, P576:3831-3846
    "GetStreamStatus",        // P576:5366-5385
    "GetRecordStatus",        // P576:5441-5457
];

/// `EventSubscription::General` (P576:1179-1185).
pub const GENERAL: u64 = 1;
/// `EventSubscription::Scenes` (P576:1199-1205).
pub const SCENES: u64 = 1 << 2;
/// `EventSubscription::Inputs` (P576:1209-1215).
pub const INPUTS: u64 = 1 << 3;
/// `EventSubscription::Outputs` (P576:1239-1245).
pub const OUTPUTS: u64 = 1 << 6;
/// `EventSubscription::All` at 5.7.6: General through Canvases, bits 0-11,
/// the high-volume ones left out (P576:1299-1305). An Identify without
/// `eventSubscriptions` gets this (P576:192, :197).
pub const ALL: u64 = (1 << 12) - 1;

/// The scripted collection's scenes, top of OBS's list first (§10.2).
pub const GAME: &str = "5c0f3a1e-2b4d-4e6f-8a9b-0c1d2e3f4a01";
pub const BRB: &str = "5c0f3a1e-2b4d-4e6f-8a9b-0c1d2e3f4a02";
pub const JUST_CHATTING: &str = "5c0f3a1e-2b4d-4e6f-8a9b-0c1d2e3f4a03";
/// Its inputs: a bare `text_gdiplus` (the version-0 id), a
/// `text_gdiplus_v3` holding [`SOURCE_TEXT`] and [`SOURCE_FILE`], a
/// `text_gdiplus_v3` that reads from a file, a `text_ft2_source_v2` and a
/// `browser_source`.
pub const TEXT_V0: &str = "7d1e4b2f-3c5e-4f70-9bac-1d2e3f4a5b01";
pub const TEXT_V3: &str = "7d1e4b2f-3c5e-4f70-9bac-1d2e3f4a5b02";
pub const TEXT_FROM_FILE: &str = "7d1e4b2f-3c5e-4f70-9bac-1d2e3f4a5b03";
pub const TEXT_FT2: &str = "7d1e4b2f-3c5e-4f70-9bac-1d2e3f4a5b04";
pub const BROWSER: &str = "7d1e4b2f-3c5e-4f70-9bac-1d2e3f4a5b05";
/// What OBS holds in the v3 source's settings: neither may reach an
/// `ObsInventory`, a status or a log line (§10.3).
pub const SOURCE_TEXT: &str = "zz-source-text-zz";
pub const SOURCE_FILE: &str = r"C:\Users\zz-user\a.txt";
/// `outputPath` on a recording's STARTED and STOPPED (EHO:82-86).
pub const RECORD_PATH: &str = r"C:\Users\zz-user\Videos\2026-11-03 20-00-00.mkv";

/// WSP:219.
const NOT_READY: &str = "OBS is not ready to perform the request.";
/// The fake's own words for a 204 (the handler that writes OBS's isn't in
/// hand).
const UNKNOWN: &str = "Your request type is not valid.";
/// One frame up to this size goes out whole (tokio-websockets would cut a
/// message into 4 MiB frames by default, TWS/src/proto/types.rs:600-608).
const ONE_FRAME: usize = 64 << 20;
/// How long the fake waits for the client's echo of its own close.
const CLOSE_WAIT: Duration = Duration::from_secs(1);
/// [`Accept::EndlessHeader`] stops writing here.
const ENDLESS_CAP: usize = 16 << 20;
/// §10.2's guard on every test.
const GUARD: Duration = Duration::from_secs(10);
/// The fake's own sockets skip Nagle, so an answer written right after an
/// event isn't held for the client's delayed ACK. (The link sets its own;
/// this is the server's side of the loopback socket.)
const NO_NAGLE: bool = true;

/// §10.2: runs `body` on a fresh current-thread runtime with every driver
/// on, under a 10 s guard. Never `#[tokio::test]`, which needs tokio's
/// `macros` (MF §5).
pub fn guarded<F: Future>(body: F) -> F::Output {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("a test runtime");
    rt.block_on(async move {
        tokio::time::timeout(GUARD, body)
            .await
            .expect("the 10 s guard tripped (§10.2)")
    })
}

// ---------------------------------------------------------------------------
// What the fake is told
// ---------------------------------------------------------------------------

/// How the fake starts and answers. `Default` is an OBS 32.2.2 with
/// obs-websocket 5.7.6, RPC 1, no auth, every request available, and
/// [`Model::standard`].
#[derive(Clone, Debug)]
pub struct Config {
    /// Hello's `obsStudioVersion` (`WSS:249`); `None` leaves it out, as a
    /// server before 5.6 does.
    pub obs_studio_version: Option<Value>,
    /// Hello's `obsWebSocketVersion` (`WSS:250`), and `GetVersion`'s; any
    /// JSON, or `None` to leave it out.
    pub ws_version: Option<Value>,
    /// Hello's `rpcVersion` (`WSS:251`), and `GetVersion`'s; any JSON.
    pub rpc_version: Option<Value>,
    /// The `rpcVersion`s an Identify may ask for. Today's server accepts
    /// only its own (WSP:32-35); anything else closes 4010 (WSP:139-144).
    pub accept_rpc: Vec<u64>,
    /// `Some`: Hello asks for auth with this password (`WSS:252-259`).
    pub password: Option<String>,
    /// `Some(m)`: an Identify subscribing to anything but `m` is a
    /// violation (D10's 69 by default).
    pub expect_mask: Option<u64>,
    /// `GetVersion`'s `obsVersion` (P576:2842).
    pub obs_version: String,
    /// Request types left out of `availableRequests`, and answered 204.
    pub unavailable: Vec<String>,
    /// Where `CurrentProgramSceneChanged` goes relative to a
    /// `SetCurrentProgramScene`'s own answer.
    pub program_event: ProgramEvent,
    /// OBS's scenes, inputs and outputs.
    pub model: Model,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            obs_studio_version: Some(json!("32.2.2")),
            ws_version: Some(json!("5.7.6")),
            rpc_version: Some(json!(1)),
            accept_rpc: vec![1],
            password: None,
            expect_mask: Some(GENERAL | SCENES | OUTPUTS),
            obs_version: "32.2.2".into(),
            unavailable: Vec::new(),
            program_event: ProgramEvent::BeforeAnswer,
            model: Model::standard(),
        }
    }
}

impl Config {
    /// Hello asks for auth with `password`.
    #[must_use]
    pub fn with_password(mut self, password: &str) -> Self {
        self.password = Some(password.into());
        self
    }

    /// Hello's (and `GetVersion`'s) `obsWebSocketVersion`.
    #[must_use]
    pub fn ws_version(mut self, version: &str) -> Self {
        self.ws_version = Some(json!(version));
        self
    }

    /// Hello's (and `GetVersion`'s) `rpcVersion`.
    #[must_use]
    pub fn hello_rpc(mut self, rpc: u64) -> Self {
        self.rpc_version = Some(json!(rpc));
        self
    }

    /// The `rpcVersion`s an Identify may ask for.
    #[must_use]
    pub fn accept_rpc(mut self, accepted: &[u64]) -> Self {
        self.accept_rpc = accepted.to_vec();
        self
    }

    /// Leaves `request_type` out of `availableRequests` (and answers it
    /// 204).
    #[must_use]
    pub fn unavailable(mut self, request_type: &str) -> Self {
        self.unavailable.push(request_type.into());
        self
    }

    #[must_use]
    pub fn model(mut self, model: Model) -> Self {
        self.model = model;
        self
    }
}

/// When `CurrentProgramSceneChanged` goes out for a switch a
/// `SetCurrentProgramScene` made. In OBS it is broadcast from the thread
/// pool while the request is still running (WSP:355-411; EH:313-314), so
/// it can reach the client either side of the answer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProgramEvent {
    /// Before the answer: the order a client is likelier to mishandle.
    BeforeAnswer,
    AfterAnswer,
    /// Not at all.
    Never,
}

/// What a new TCP connection meets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Accept {
    /// The upgrade, then Hello (`WSS:219-294`).
    Normal,
    /// Accepted and closed at once, before any byte: §10.2's "OBS down".
    CloseAtOnce,
    /// The upgrade request is read and never answered.
    Stall,
    /// A 101 status line, then one header line that never ends (up to 16
    /// MiB), for the link's 64 KiB handshake cap (D7).
    EndlessHeader,
}

/// When Hello, or Identified, goes out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    /// At once, as OBS does (`WSS:282-292`; WSP:167-170).
    Now,
    After(Duration),
    Never,
    /// A close with this code instead.
    Close(u16),
}

/// What a scripted request gets. A rule's `delay` comes first, then this.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// The model's real answer.
    Answer,
    /// `requestStatus {result: false, code}` and nothing applied.
    Code(u16),
    /// 207 `NotReady` (P576:889-897).
    NotReady,
    /// Never answered and never applied: it stays outstanding until the
    /// connection ends ("stall forever").
    Drop,
    /// Applied (a scene switches, a text changes) but never answered, as a
    /// Set whose answer is lost behind OBS's UI thread can.
    ApplyNoAnswer,
    /// The connection closed with this code instead of an answer.
    Close(u16),
}

/// A scripted answer: the first live rule that matches a request is used
/// (and, with [`Rule::times`], counted down); with none, the request gets
/// the model's answer at once. A rule given no outcome answers normally, so
/// `Rule::on(t).times(n)` lets the next `n` of type `t` through to the
/// rules after it (to drop only the third push, say).
#[derive(Clone, Debug)]
pub struct Rule {
    request_type: Option<String>,
    left: Option<u32>,
    delay: Duration,
    outcome: Outcome,
}

impl Rule {
    /// Matches requests of this type.
    pub fn on(request_type: &str) -> Self {
        Self {
            request_type: Some(request_type.into()),
            left: None,
            delay: Duration::ZERO,
            outcome: Outcome::Answer,
        }
    }

    /// Matches every request.
    pub fn any() -> Self {
        Self {
            request_type: None,
            left: None,
            delay: Duration::ZERO,
            outcome: Outcome::Answer,
        }
    }

    /// Only the next `n` matches; without it, every one.
    #[must_use]
    pub fn times(mut self, n: u32) -> Self {
        self.left = Some(n);
        self
    }

    /// The outcome comes this long after the request arrived.
    #[must_use]
    pub fn delay(mut self, delay: Duration) -> Self {
        self.delay = delay;
        self
    }

    #[must_use]
    pub fn code(mut self, code: u16) -> Self {
        self.outcome = Outcome::Code(code);
        self
    }

    #[must_use]
    pub fn not_ready(mut self) -> Self {
        self.outcome = Outcome::NotReady;
        self
    }

    #[must_use]
    pub fn drop_answer(mut self) -> Self {
        self.outcome = Outcome::Drop;
        self
    }

    #[must_use]
    pub fn apply_without_answer(mut self) -> Self {
        self.outcome = Outcome::ApplyNoAnswer;
        self
    }

    #[must_use]
    pub fn close(mut self, code: u16) -> Self {
        self.outcome = Outcome::Close(code);
        self
    }
}

// ---------------------------------------------------------------------------
// OBS's side: scenes, inputs, outputs
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Scene {
    pub name: String,
    pub uuid: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Input {
    pub name: String,
    pub uuid: String,
    /// The versioned id: `inputKind` (ARR:215, :223).
    pub kind: String,
    /// `unversionedInputKind` (ARR:224); `None` leaves the field out of
    /// `GetInputList`, as a server older than the doc in hand might (D19).
    pub unversioned_kind: Option<String>,
    /// The source's own settings, without defaults (RHI:298-299, :322-325).
    pub settings: Map<String, Value>,
    /// `inputKindCaps` (ARR:225); the fake's own number.
    pub caps: u64,
}

impl Input {
    /// `settings` must be a JSON object (anything else reads as `{}`).
    pub fn new(name: &str, uuid: &str, kind: &str, unversioned: &str, settings: &Value) -> Self {
        Self {
            name: name.into(),
            uuid: uuid.into(),
            kind: kind.into(),
            unversioned_kind: Some(unversioned.into()),
            settings: settings.as_object().cloned().unwrap_or_default(),
            caps: 1,
        }
    }
}

/// The stream or record output, as `GetStreamStatus`/`GetRecordStatus`
/// report it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Output {
    pub active: bool,
    pub reconnecting: bool,
    pub paused: bool,
    /// `outputDuration` while active; 0 while not (NUM:26-36).
    pub duration_ms: u64,
    pub bytes: u64,
}

/// What OBS holds. Tests read and change it through [`FakeObs::model`].
#[derive(Clone, Debug, PartialEq)]
pub struct Model {
    /// OBS's scene list, top first.
    pub scenes: Vec<Scene>,
    /// The program scene's uuid.
    pub program: String,
    pub inputs: Vec<Input>,
    pub stream: Output,
    pub record: Output,
}

impl Model {
    /// §10.2's collection: Game (the program), BRB, Just Chatting; the five
    /// inputs of [`TEXT_V0`] and the constants after it; both outputs off.
    pub fn standard() -> Self {
        let scene = |name: &str, uuid: &str| Scene {
            name: name.into(),
            uuid: uuid.into(),
        };
        Self {
            scenes: vec![
                scene("Game", GAME),
                scene("BRB", BRB),
                scene("Just Chatting", JUST_CHATTING),
            ],
            program: GAME.into(),
            inputs: vec![
                Input::new(
                    "Old timer",
                    TEXT_V0,
                    "text_gdiplus",
                    "text_gdiplus",
                    &json!({"text": "0:00"}),
                ),
                Input::new(
                    "Timer",
                    TEXT_V3,
                    "text_gdiplus_v3",
                    "text_gdiplus",
                    &json!({"text": SOURCE_TEXT, "file": SOURCE_FILE}),
                ),
                Input::new(
                    "Timer from a file",
                    TEXT_FROM_FILE,
                    "text_gdiplus_v3",
                    "text_gdiplus",
                    &json!({"read_from_file": true, "file": SOURCE_FILE}),
                ),
                Input::new(
                    "Timer FT2",
                    TEXT_FT2,
                    "text_ft2_source_v2",
                    "text_ft2_source",
                    &json!({"text": "0:00"}),
                ),
                Input::new(
                    "Alerts",
                    BROWSER,
                    "browser_source",
                    "browser_source",
                    &json!({"width": 800, "height": 600}),
                ),
            ],
            stream: Output::default(),
            record: Output::default(),
        }
    }

    /// The same, with `unversionedInputKind` left out of every input.
    #[must_use]
    pub fn without_unversioned_kinds(mut self) -> Self {
        for input in &mut self.inputs {
            input.unversioned_kind = None;
        }
        self
    }

    /// `n` more `text_gdiplus_v3` sources, "Text 1" to "Text n", uuids
    /// [`numbered_uuid`]`(1..=n)`.
    #[must_use]
    pub fn with_text_sources(mut self, n: usize) -> Self {
        for i in 1..=n {
            self.inputs.push(Input::new(
                &format!("Text {i}"),
                &numbered_uuid(i),
                "text_gdiplus_v3",
                "text_gdiplus",
                &json!({"text": ""}),
            ));
        }
        self
    }

    pub fn input(&self, uuid: &str) -> Option<&Input> {
        self.inputs.iter().find(|i| i.uuid == uuid)
    }

    pub fn scene(&self, uuid: &str) -> Option<&Scene> {
        self.scenes.iter().find(|s| s.uuid == uuid)
    }
}

/// The uuid of [`Model::with_text_sources`]' `i`th source.
pub fn numbered_uuid(i: usize) -> String {
    format!("b2000000-0000-4000-8000-{i:012}")
}

/// `ObsOutputState` (P576:1417-1505).
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
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Unknown => "OBS_WEBSOCKET_OUTPUT_UNKNOWN",
            Self::Starting => "OBS_WEBSOCKET_OUTPUT_STARTING",
            Self::Started => "OBS_WEBSOCKET_OUTPUT_STARTED",
            Self::Stopping => "OBS_WEBSOCKET_OUTPUT_STOPPING",
            Self::Stopped => "OBS_WEBSOCKET_OUTPUT_STOPPED",
            Self::Reconnecting => "OBS_WEBSOCKET_OUTPUT_RECONNECTING",
            Self::Reconnected => "OBS_WEBSOCKET_OUTPUT_RECONNECTED",
            Self::Paused => "OBS_WEBSOCKET_OUTPUT_PAUSED",
            Self::Resumed => "OBS_WEBSOCKET_OUTPUT_RESUMED",
        }
    }

    /// The event's `outputActive` (EHO:22-38).
    fn active(self) -> bool {
        matches!(self, Self::Started | Self::Resumed | Self::Reconnected)
    }

    /// What the state does to the output's status.
    fn apply(self, out: &mut Output) {
        match self {
            Self::Started => {
                *out = Output {
                    active: true,
                    ..Output::default()
                };
            }
            Self::Stopped => *out = Output::default(),
            Self::Reconnecting => out.reconnecting = true,
            Self::Reconnected => out.reconnecting = false,
            Self::Paused => out.paused = true,
            Self::Resumed => out.paused = false,
            Self::Unknown | Self::Starting | Self::Stopping => {}
        }
    }
}

/// An event as OBS sends it: `{op: 5, d: {eventType, eventIntent,
/// eventData?}}` (P576:261-292; WSP:362-369).
#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    /// General, no data (P576:1585-1591; EH:460-464).
    ExitStarted,
    /// Scenes (P576:1785-1799).
    SceneCreated { name: String, uuid: String },
    /// Scenes (P576:1803-1817).
    SceneRemoved { name: String, uuid: String },
    /// Scenes (P576:1821-1835).
    SceneNameChanged {
        uuid: String,
        old: String,
        name: String,
    },
    /// Scenes (P576:1839-1852).
    CurrentProgramSceneChanged { name: String, uuid: String },
    /// Outputs (EHO:40-60).
    StreamStateChanged(OutputState),
    /// Outputs, with `outputPath` (EHO:62-88).
    RecordStateChanged(OutputState),
    /// Inputs (P576:1948-1964).
    InputSettingsChanged {
        name: String,
        uuid: String,
        settings: Map<String, Value>,
    },
    /// Any other, exactly as given.
    Raw {
        event_type: String,
        intent: u64,
        data: Option<Value>,
    },
}

impl Event {
    /// `(eventIntent, the frame's text)`.
    fn wire(&self) -> (u64, String) {
        let (ty, intent, data) = match self {
            Self::ExitStarted => ("ExitStarted", GENERAL, None),
            Self::SceneCreated { name, uuid } => (
                "SceneCreated",
                SCENES,
                Some(json!({"sceneName": name, "sceneUuid": uuid, "isGroup": false})),
            ),
            Self::SceneRemoved { name, uuid } => (
                "SceneRemoved",
                SCENES,
                Some(json!({"sceneName": name, "sceneUuid": uuid, "isGroup": false})),
            ),
            Self::SceneNameChanged { uuid, old, name } => (
                "SceneNameChanged",
                SCENES,
                Some(json!({"sceneUuid": uuid, "oldSceneName": old, "sceneName": name})),
            ),
            Self::CurrentProgramSceneChanged { name, uuid } => (
                "CurrentProgramSceneChanged",
                SCENES,
                Some(json!({"sceneName": name, "sceneUuid": uuid})),
            ),
            Self::StreamStateChanged(s) => (
                "StreamStateChanged",
                OUTPUTS,
                Some(json!({"outputActive": s.active(), "outputState": s.as_str()})),
            ),
            Self::RecordStateChanged(s) => {
                let path = if matches!(s, OutputState::Started | OutputState::Stopped) {
                    json!(RECORD_PATH)
                } else {
                    Value::Null
                };
                (
                    "RecordStateChanged",
                    OUTPUTS,
                    Some(json!({
                        "outputActive": s.active(),
                        "outputState": s.as_str(),
                        "outputPath": path,
                    })),
                )
            }
            Self::InputSettingsChanged {
                name,
                uuid,
                settings,
            } => (
                "InputSettingsChanged",
                INPUTS,
                Some(json!({"inputName": name, "inputUuid": uuid, "inputSettings": settings})),
            ),
            Self::Raw {
                event_type,
                intent,
                data,
            } => (event_type.as_str(), *intent, data.clone()),
        };
        let mut d = json!({"eventType": ty, "eventIntent": intent});
        // WSP:368-369: `eventData` only when it is an object.
        if let Some(data) = data.filter(Value::is_object) {
            d["eventData"] = data;
        }
        (intent, json!({"op": 5, "d": d}).to_string())
    }
}

// ---------------------------------------------------------------------------
// What the fake saw
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dir {
    /// From the client.
    In,
    /// From the fake.
    Out,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Frame {
    /// The client's HTTP upgrade request: its path and headers (names
    /// lowercase).
    Upgrade {
        path: String,
        headers: Vec<(String, String)>,
    },
    Text(String),
    /// A binary frame's length.
    Binary(usize),
    Ping(Vec<u8>),
    Pong(Vec<u8>),
    /// A close frame's code (`None`: none in the frame) and reason.
    Close(Option<u16>, String),
    /// Bytes read or written outside WebSocket framing.
    Raw(usize),
    /// `n` `SceneNameChanged` events sent back to back.
    Flood(usize),
    /// One text frame of this many bytes.
    Big(usize),
    /// The peer's FIN.
    Eof,
    /// A read or write failed.
    Error(String),
}

/// One frame in or out, with when the fake read or flushed it.
#[derive(Clone, Debug)]
pub struct Record {
    pub session: u64,
    pub at: Instant,
    pub dir: Dir,
    pub frame: Frame,
}

impl Record {
    /// A text frame's JSON.
    pub fn json(&self) -> Option<Value> {
        match &self.frame {
            Frame::Text(t) => serde_json::from_str(t).ok(),
            _ => None,
        }
    }

    /// A text frame's `op`.
    pub fn op(&self) -> Option<u64> {
        self.json()
            .and_then(|v| v.get("op").and_then(Value::as_u64))
    }
}

/// A request the fake received, and when it answered it.
#[derive(Clone, Debug)]
pub struct Seen {
    pub session: u64,
    pub at: Instant,
    pub request_type: String,
    pub request_id: Value,
    /// `requestData` as sent; `None` when absent.
    pub data: Option<Value>,
    /// When the answer went out, and its code.
    pub answer: Option<(Instant, u16)>,
}

/// How a connection ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum End {
    /// The client sent a close (with this code) first.
    ClientClosed(Option<u16>),
    /// The fake closed with this code.
    ServerClosed(u16),
    /// FIN without a close frame.
    Eof,
    Error(String),
    /// [`Accept::CloseAtOnce`].
    Dropped,
    /// The fake was dropped.
    Shutdown,
}

/// One accepted connection.
#[derive(Clone, Debug)]
pub struct SessionInfo {
    /// 1 for the first accept, then counting.
    pub id: u64,
    pub accepted_at: Instant,
    pub upgraded: bool,
    pub hello_sent: bool,
    /// Identified has gone out.
    pub identified: bool,
    /// The Identify's mask (`ALL` when it had none); `None` before one
    /// passed.
    pub subscriptions: Option<u64>,
    /// The `rpcVersion` the Identify asked for.
    pub rpc_version: Option<u64>,
    /// This connection's challenge (`WSS:254-257`), when auth was asked.
    pub challenge: Option<String>,
    /// Requests received and not answered (a dropped one stays).
    pub outstanding: usize,
    /// [`Accept::EndlessHeader`]'s bytes written.
    pub header_bytes: usize,
    pub end: Option<End>,
}

// ---------------------------------------------------------------------------
// The fake
// ---------------------------------------------------------------------------

/// A running fake obs-websocket server on 127.0.0.1. Dropping it stops
/// every task (and so drops every socket).
pub struct FakeObs {
    inner: Arc<Inner>,
    addr: SocketAddr,
    tasks: Arc<Mutex<Vec<JoinHandle<()>>>>,
    acceptor: JoinHandle<()>,
}

impl Drop for FakeObs {
    fn drop(&mut self) {
        self.acceptor.abort();
        for task in lock(&self.tasks).drain(..) {
            task.abort();
        }
    }
}

impl FakeObs {
    /// Listens on 127.0.0.1 at a port the OS picks (never OBS's default),
    /// with a new salt (`WSS:85-86`). Call it inside the test's runtime.
    pub async fn start(cfg: Config) -> Self {
        let listener = TcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, 0)))
            .await
            .expect("bind 127.0.0.1:0");
        let addr = listener.local_addr().expect("the fake's address");
        let inner = Arc::new(Inner {
            state: Mutex::new(State::new(cfg)),
            changed: Notify::new(),
            io: Arc::new(IoFlags::default()),
        });
        let tasks = Arc::new(Mutex::new(Vec::new()));
        let acceptor = tokio::spawn(accept_loop(listener, inner.clone(), tasks.clone()));
        Self {
            inner,
            addr,
            tasks,
            acceptor,
        }
    }

    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    pub fn port(&self) -> u16 {
        self.addr.port()
    }

    // --- how the fake starts and answers ---------------------------------

    /// Changes any [`Config`] field; Hello and Identify read it per
    /// connection, requests per request.
    pub fn config(&self, f: impl FnOnce(&mut Config)) {
        self.inner.with(|s| f(&mut s.cfg));
    }

    /// A new salt, as OBS makes at each start of its server (`WSS:85-86`): a
    /// secret derived from the old one stops working.
    pub fn restart(&self) {
        self.inner.with(|s| s.salt = salt());
    }

    /// Auth on (`Some`) or off for the next connections, with a new salt.
    pub fn set_password(&self, password: Option<&str>) {
        self.inner.with(|s| {
            s.cfg.password = password.map(str::to_owned);
            s.salt = salt();
        });
    }

    pub fn set_accept(&self, accept: Accept) {
        self.inner.with(|s| s.accept = accept);
    }

    pub fn set_hello(&self, stage: Stage) {
        self.inner.with(|s| s.hello = stage);
    }

    pub fn set_identified(&self, stage: Stage) {
        self.inner.with(|s| s.identified = stage);
    }

    /// While on, no pong goes out for a ping (tokio-websockets would
    /// answer every one itself, TWS/src/proto/stream.rs:353-358).
    pub fn set_ignore_pings(&self, ignore: bool) {
        self.inner.io.drop_pongs.store(ignore, Ordering::SeqCst);
    }

    /// Off: a client's close is neither echoed nor followed by the fake's
    /// own; the fake reads on until the client's FIN.
    pub fn set_close_echo(&self, echo: bool) {
        self.inner.with(|s| s.close_echo = echo);
    }

    /// Not ready: every request is answered 207 at once (WSP:212-220) and
    /// no event goes out (WSP:359-360), as during a scene-collection
    /// switch or exit (EH:270-298, :460-469).
    pub fn set_ready(&self, ready: bool) {
        self.inner.with(|s| s.ready = ready);
    }

    pub fn script(&self, rule: Rule) {
        self.inner.with(|s| s.rules.push(rule));
    }

    pub fn clear_scripts(&self) {
        self.inner.with(|s| s.rules.clear());
    }

    /// Reads or changes OBS's state, silently: no event goes out.
    pub fn model<R>(&self, f: impl FnOnce(&mut Model) -> R) -> R {
        self.inner.with(|s| f(&mut s.cfg.model))
    }

    // --- what the fake does to live connections ----------------------------

    /// Closes every upgraded connection with `code` (a Kick is 4011,
    /// `WSS:166-182`; a server stop 1001, `WSS:136-152`) and waits up to 1 s
    /// for each echo.
    pub fn close_all(&self, code: u16, reason: &str) {
        self.inner
            .with(|s| s.send_all(&Cmd::Close(code, reason.into()), None));
    }

    /// Closes connection `id` (see [`SessionInfo::id`]).
    pub fn close_session(&self, id: u64, code: u16, reason: &str) {
        self.inner
            .with(|s| s.send_all(&Cmd::Close(code, reason.into()), Some(id)));
    }

    /// Broadcasts `event` as OBS does: to identified connections whose
    /// mask has its intent (WSP:376-382), and only while ready
    /// (WSP:359-360). Returns how many it went to.
    pub fn event(&self, event: &Event) -> usize {
        let (intent, text) = event.wire();
        self.inner.with(|s| s.broadcast(intent, &text, None))
    }

    /// OBS starting to quit: `ExitStarted`, then not ready (EH:460-469).
    /// The server's stop (close 1001) is [`FakeObs::close_all`].
    pub fn exit_started(&self) {
        let (intent, text) = Event::ExitStarted.wire();
        self.inner.with(|s| {
            s.broadcast(intent, &text, None);
            s.ready = false;
        });
    }

    /// The streamer switches OBS's program scene: the model, then
    /// `CurrentProgramSceneChanged` (EH:313-314). `false`: no such scene.
    pub fn switch_program(&self, uuid: &str) -> bool {
        self.inner.with(|s| {
            let Some(scene) = s.cfg.model.scene(uuid).cloned() else {
                return false;
            };
            s.cfg.model.program.clone_from(&scene.uuid);
            let (intent, text) = Event::CurrentProgramSceneChanged {
                name: scene.name,
                uuid: scene.uuid,
            }
            .wire();
            s.broadcast(intent, &text, None);
            true
        })
    }

    /// The stream output changes state: the model, then the event
    /// (EHO:54-60).
    pub fn stream_state(&self, state: OutputState) {
        let (intent, text) = Event::StreamStateChanged(state).wire();
        self.inner.with(|s| {
            state.apply(&mut s.cfg.model.stream);
            s.broadcast(intent, &text, None);
        });
    }

    /// The record output changes state: the model, then the event
    /// (EHO:77-88).
    pub fn record_state(&self, state: OutputState) {
        let (intent, text) = Event::RecordStateChanged(state).wire();
        self.inner.with(|s| {
            state.apply(&mut s.cfg.model.record);
            s.broadcast(intent, &text, None);
        });
    }

    /// A scene added at the bottom of OBS's list, then `SceneCreated`.
    pub fn create_scene(&self, name: &str, uuid: &str) {
        let (intent, text) = Event::SceneCreated {
            name: name.into(),
            uuid: uuid.into(),
        }
        .wire();
        self.inner.with(|s| {
            s.cfg.model.scenes.push(Scene {
                name: name.into(),
                uuid: uuid.into(),
            });
            s.broadcast(intent, &text, None);
        });
    }

    /// A scene deleted, then `SceneRemoved`. `false`: no such scene.
    pub fn remove_scene(&self, uuid: &str) -> bool {
        self.inner.with(|s| {
            let Some(at) = s.cfg.model.scenes.iter().position(|x| x.uuid == uuid) else {
                return false;
            };
            let scene = s.cfg.model.scenes.remove(at);
            let (intent, text) = Event::SceneRemoved {
                name: scene.name,
                uuid: scene.uuid,
            }
            .wire();
            s.broadcast(intent, &text, None);
            true
        })
    }

    /// A scene renamed, then `SceneNameChanged`. `false`: no such scene.
    pub fn rename_scene(&self, uuid: &str, name: &str) -> bool {
        self.inner.with(|s| {
            let Some(scene) = s.cfg.model.scenes.iter_mut().find(|x| x.uuid == uuid) else {
                return false;
            };
            let old = std::mem::replace(&mut scene.name, name.into());
            let (intent, text) = Event::SceneNameChanged {
                uuid: uuid.into(),
                old,
                name: name.into(),
            }
            .wire();
            s.broadcast(intent, &text, None);
            true
        })
    }

    /// `n` `SceneNameChanged` events back to back, as fast as each socket
    /// takes them, to every connection [`FakeObs::event`] would reach. The
    /// model doesn't change.
    pub fn flood_scene_renames(&self, n: usize) {
        self.inner.with(|s| s.send_gated(SCENES, &Cmd::Flood(n)));
    }

    /// One text frame of `len` bytes (a valid event padded out) to every
    /// upgraded connection, sent whole, not cut into 4 MiB frames.
    pub fn send_big_frame(&self, len: usize) {
        self.inner.with(|s| s.send_all(&Cmd::Big(len), None));
    }

    /// Bytes straight onto every upgraded socket, outside any frame.
    pub fn send_raw(&self, bytes: &[u8]) {
        self.inner
            .with(|s| s.send_all(&Cmd::Raw(bytes.to_vec()), None));
    }

    /// A text frame (anything: not JSON, an unknown op) to every upgraded
    /// connection.
    pub fn send_text(&self, text: &str) {
        self.inner
            .with(|s| s.send_all(&Cmd::Text(text.into()), None));
    }

    // --- what the fake saw --------------------------------------------------

    /// TCP connections accepted, in every [`Accept`] mode.
    pub fn accepts(&self) -> usize {
        self.inner.lock().sessions.len()
    }

    /// Every frame in and out, in the order the fake read or sent them.
    pub fn log(&self) -> Vec<Record> {
        self.inner.lock().log.clone()
    }

    pub fn requests(&self) -> Vec<Seen> {
        self.inner.lock().requests.clone()
    }

    pub fn requests_of(&self, request_type: &str) -> Vec<Seen> {
        let s = self.inner.lock();
        s.requests
            .iter()
            .filter(|r| r.request_type == request_type)
            .cloned()
            .collect()
    }

    /// Every Identify's `d`, with its connection's id.
    pub fn identifies(&self) -> Vec<(u64, Value)> {
        self.inner.lock().identifies.clone()
    }

    pub fn sessions(&self) -> Vec<SessionInfo> {
        self.inner.lock().sessions.clone()
    }

    pub fn session(&self, id: u64) -> Option<SessionInfo> {
        let s = self.inner.lock();
        s.info(id).cloned()
    }

    /// The salt of this start (`WSS:85`).
    pub fn salt(&self) -> String {
        self.inner.lock().salt.clone()
    }

    /// The program scene's uuid.
    pub fn program(&self) -> String {
        self.inner.lock().cfg.model.program.clone()
    }

    /// Pongs sent and pongs withheld ([`FakeObs::set_ignore_pings`]).
    pub fn pongs(&self) -> (usize, usize) {
        (
            self.inner.io.pongs_sent.load(Ordering::SeqCst),
            self.inner.io.pongs_dropped.load(Ordering::SeqCst),
        )
    }

    /// The most requests one connection ever had unanswered at once.
    pub fn max_outstanding(&self) -> usize {
        self.inner.lock().max_outstanding
    }

    /// What a correct link never does, as the fake saw it.
    pub fn violations(&self) -> Vec<String> {
        self.inner.lock().violations.clone()
    }

    /// Fails the test, listing them, if [`FakeObs::violations`] isn't
    /// empty.
    #[track_caller]
    pub fn assert_clean(&self) {
        let v = self.violations();
        assert!(
            v.is_empty(),
            "the fake saw what a correct link never does:\n{}",
            v.join("\n")
        );
    }

    // --- waiting ------------------------------------------------------------

    /// Waits up to `timeout` for `done` to hold, re-checking at every
    /// change the fake records. Returns whether it held.
    pub async fn wait(&self, timeout: Duration, mut done: impl FnMut(&Self) -> bool) -> bool {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            let mut changed = pin!(self.inner.changed.notified());
            changed.as_mut().enable();
            if done(self) {
                return true;
            }
            if tokio::time::timeout_at(deadline, changed).await.is_err() {
                return done(self);
            }
        }
    }

    pub async fn wait_accepts(&self, n: usize, timeout: Duration) -> bool {
        self.wait(timeout, |f| f.accepts() >= n).await
    }

    /// `n` connections have been sent Identified.
    pub async fn wait_identified(&self, n: usize, timeout: Duration) -> bool {
        self.wait(timeout, |f| {
            f.sessions().iter().filter(|s| s.identified).count() >= n
        })
        .await
    }

    pub async fn wait_requests(&self, request_type: &str, n: usize, timeout: Duration) -> bool {
        self.wait(timeout, |f| f.requests_of(request_type).len() >= n)
            .await
    }

    /// `n` connections have ended.
    pub async fn wait_ended(&self, n: usize, timeout: Duration) -> bool {
        self.wait(timeout, |f| {
            f.sessions().iter().filter(|s| s.end.is_some()).count() >= n
        })
        .await
    }
}

// ---------------------------------------------------------------------------
// Shared state
// ---------------------------------------------------------------------------

struct Inner {
    state: Mutex<State>,
    /// Every recorded change wakes [`FakeObs::wait`].
    changed: Notify,
    io: Arc<IoFlags>,
}

impl Inner {
    fn lock(&self) -> MutexGuard<'_, State> {
        lock(&self.state)
    }

    /// Runs `f` under the lock, then wakes every waiter.
    fn with<R>(&self, f: impl FnOnce(&mut State) -> R) -> R {
        let r = f(&mut self.lock());
        self.changed.notify_waiters();
        r
    }

    fn record(&self, session: u64, dir: Dir, frame: Frame) {
        self.with(|s| {
            s.log.push(Record {
                session,
                at: Instant::now(),
                dir,
                frame,
            });
        });
    }

    fn violation(&self, session: u64, what: &str) {
        self.with(|s| s.violations.push(format!("connection {session}: {what}")));
    }

    fn update(&self, id: u64, f: impl FnOnce(&mut SessionInfo)) {
        self.with(|s| {
            if let Some(info) = s.info_mut(id) {
                f(info);
            }
        });
    }
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// What a test asks of a live connection.
#[derive(Clone, Debug)]
enum Cmd {
    Close(u16, String),
    Text(String),
    Raw(Vec<u8>),
    Flood(usize),
    Big(usize),
}

struct State {
    cfg: Config,
    /// Made once per start (`WSS:85-86`), and by [`FakeObs::restart`].
    salt: String,
    accept: Accept,
    hello: Stage,
    identified: Stage,
    close_echo: bool,
    ready: bool,
    rules: Vec<Rule>,
    sessions: Vec<SessionInfo>,
    /// The upgraded connections' command channels.
    live: Vec<(u64, mpsc::UnboundedSender<Cmd>)>,
    log: Vec<Record>,
    requests: Vec<Seen>,
    identifies: Vec<(u64, Value)>,
    violations: Vec<String>,
    max_outstanding: usize,
}

impl State {
    fn new(cfg: Config) -> Self {
        Self {
            cfg,
            salt: salt(),
            accept: Accept::Normal,
            hello: Stage::Now,
            identified: Stage::Now,
            close_echo: true,
            ready: true,
            rules: Vec::new(),
            sessions: Vec::new(),
            live: Vec::new(),
            log: Vec::new(),
            requests: Vec::new(),
            identifies: Vec::new(),
            violations: Vec::new(),
            max_outstanding: 0,
        }
    }

    fn info(&self, id: u64) -> Option<&SessionInfo> {
        let at = usize::try_from(id).ok()?.checked_sub(1)?;
        self.sessions.get(at)
    }

    fn info_mut(&mut self, id: u64) -> Option<&mut SessionInfo> {
        let at = usize::try_from(id).ok()?.checked_sub(1)?;
        self.sessions.get_mut(at)
    }

    /// `cmd` to every upgraded connection, or only to `only`.
    fn send_all(&self, cmd: &Cmd, only: Option<u64>) {
        for (id, tx) in &self.live {
            if only.is_none_or(|o| o == *id) {
                let _ = tx.send(cmd.clone());
            }
        }
    }

    /// Whether connection `id` gets an event of `intent` now: identified
    /// (WSP:378-379), subscribed (WSP:382), and OBS ready (WSP:359-360).
    fn wants(&self, id: u64, intent: u64) -> bool {
        self.ready
            && self
                .info(id)
                .is_some_and(|i| i.identified && i.subscriptions.unwrap_or(0) & intent != 0)
    }

    /// `cmd` to every connection [`State::wants`] an event of `intent`.
    fn send_gated(&self, intent: u64, cmd: &Cmd) {
        for (id, tx) in &self.live {
            if self.wants(*id, intent) {
                let _ = tx.send(cmd.clone());
            }
        }
    }

    /// An event to every connection that wants it but `except`; returns
    /// how many it went to.
    fn broadcast(&self, intent: u64, text: &str, except: Option<u64>) -> usize {
        let mut n = 0;
        for (id, tx) in &self.live {
            if Some(*id) != except
                && self.wants(*id, intent)
                && tx.send(Cmd::Text(text.into())).is_ok()
            {
                n += 1;
            }
        }
        n
    }

    /// The first live rule for `request_type`, counted down.
    fn take_rule(&mut self, request_type: &str) -> (Duration, Outcome) {
        for rule in &mut self.rules {
            if rule.left == Some(0)
                || rule
                    .request_type
                    .as_deref()
                    .is_some_and(|t| t != request_type)
            {
                continue;
            }
            if let Some(left) = &mut rule.left {
                *left -= 1;
            }
            return (rule.delay, rule.outcome);
        }
        (Duration::ZERO, Outcome::Answer)
    }

    /// In [`REQUESTS`] and not made unavailable.
    fn serves(&self, request_type: &str) -> bool {
        REQUESTS.contains(&request_type) && !self.cfg.unavailable.iter().any(|u| u == request_type)
    }
}

// ---------------------------------------------------------------------------
// Requests (RHS, RHI, P576's request sections)
// ---------------------------------------------------------------------------

/// A request's failure: `(code, comment)`.
type Fail = (u16, String);

/// A request's result, and the events it causes either side of its answer.
struct Done {
    result: Result<Option<Value>, Fail>,
    before: Vec<Event>,
    after: Vec<Event>,
}

/// A source named by name or by uuid.
enum Key {
    Name(String),
    Uuid(String),
}

impl Key {
    fn matches(&self, name: &str, uuid: &str) -> bool {
        match self {
            Self::Name(n) => n == name,
            Self::Uuid(u) => u == uuid,
        }
    }

    fn describe(&self) -> String {
        match self {
            Self::Name(n) => format!("name of `{n}`"),
            Self::Uuid(u) => format!("uuid of `{u}`"),
        }
    }
}

/// `requestData`'s fields. Request.cpp (which validates them) isn't in
/// hand, so the codes are this fake's reading of P576's status table: no
/// object 301 (:911-917), a missing field 300 (:901-907), a wrong type 401
/// (:933-939), an empty string 403 (:953-959), both of a name/uuid pair 404
/// (:963-969).
fn fields(data: Option<&Value>) -> Result<&Map<String, Value>, Fail> {
    data.and_then(Value::as_object).ok_or_else(|| {
        (
            301,
            "Your request data is missing or invalid (non-object).".into(),
        )
    })
}

fn present<'a>(obj: &'a Map<String, Value>, key: &str) -> Option<&'a Value> {
    obj.get(key).filter(|v| !v.is_null())
}

fn string(v: &Value, key: &str) -> Result<String, Fail> {
    match v.as_str() {
        None => Err((401, format!("The field value of `{key}` must be a string."))),
        Some("") => Err((403, format!("The field `{key}` must not be empty."))),
        Some(s) => Ok(s.to_owned()),
    }
}

/// One of `name_key` or `uuid_key`, never both.
fn pick(data: Option<&Value>, name_key: &str, uuid_key: &str) -> Result<Key, Fail> {
    let obj = fields(data)?;
    match (present(obj, name_key), present(obj, uuid_key)) {
        (Some(_), Some(_)) => Err((
            404,
            format!("Specify only one of `{name_key}` or `{uuid_key}`."),
        )),
        (None, None) => Err((
            300,
            format!("Your request must contain at least one of `{name_key}` or `{uuid_key}`."),
        )),
        (Some(v), None) => string(v, name_key).map(Key::Name),
        (None, Some(v)) => string(v, uuid_key).map(Key::Uuid),
    }
}

/// The fake's own timecode format (`StringHelper` isn't in hand).
fn timecode(ms: u64) -> String {
    let s = ms / 1000;
    format!(
        "{:02}:{:02}:{:02}.{:03}",
        s / 3600,
        s / 60 % 60,
        s % 60,
        ms % 1000
    )
}

impl Model {
    /// A scene by name or uuid: an input's 602, nothing 600 (P576:1043-1071).
    fn find_scene(&self, key: &Key) -> Result<&Scene, Fail> {
        if let Some(scene) = self.scenes.iter().find(|s| key.matches(&s.name, &s.uuid)) {
            return Ok(scene);
        }
        if self.inputs.iter().any(|i| key.matches(&i.name, &i.uuid)) {
            return Err((602, "The specified source is not a scene.".into()));
        }
        Err((
            600,
            format!("No scene was found by the {}.", key.describe()),
        ))
    }

    /// An input by name or uuid: a scene's 602, nothing 600.
    fn find_input(&self, key: &Key) -> Result<usize, Fail> {
        if let Some(at) = self
            .inputs
            .iter()
            .position(|i| key.matches(&i.name, &i.uuid))
        {
            return Ok(at);
        }
        if self.scenes.iter().any(|s| key.matches(&s.name, &s.uuid)) {
            return Err((602, "The specified source is not an input.".into()));
        }
        Err((
            600,
            format!("No input was found by the {}.", key.describe()),
        ))
    }

    /// `GetSceneList` (RHS:40-78): the program scene, no preview (Studio
    /// Mode off), and the scenes as ARR:87-111 lists them: `sceneIndex`
    /// counts up from the bottom of OBS's list, and the array starts at the
    /// bottom.
    fn scene_list(&self) -> Value {
        let program = self.scene(&self.program);
        let n = self.scenes.len();
        let scenes: Vec<Value> = self
            .scenes
            .iter()
            .enumerate()
            .rev()
            .map(
                |(i, s)| json!({"sceneName": s.name, "sceneUuid": s.uuid, "sceneIndex": n - i - 1}),
            )
            .collect();
        json!({
            "currentProgramSceneName": program.map(|s| s.name.clone()),
            "currentProgramSceneUuid": program.map(|s| s.uuid.clone()),
            "currentPreviewSceneName": Value::Null,
            "currentPreviewSceneUuid": Value::Null,
            "scenes": scenes,
        })
    }

    /// `GetCurrentProgramScene` (RHS:127-135): each field twice, the
    /// deprecated `currentProgram*` pair beside the new one.
    fn program_scene(&self) -> Value {
        let program = self.scene(&self.program);
        let name = program.map(|s| s.name.clone());
        let uuid = program.map(|s| s.uuid.clone());
        json!({
            "sceneName": name,
            "sceneUuid": uuid,
            "currentProgramSceneName": name,
            "currentProgramSceneUuid": uuid,
        })
    }

    /// `GetInputList` (RHI:36-52, ARR:203-235): every input, or those whose
    /// versioned `inputKind` equals the filter (ARR:217).
    fn input_list(&self, data: Option<&Value>) -> Result<Option<Value>, Fail> {
        let kind = match data.and_then(Value::as_object) {
            Some(obj) => match present(obj, "inputKind") {
                None => None,
                Some(Value::String(k)) => Some(k.clone()),
                Some(_) => {
                    return Err((
                        401,
                        "The field value of `inputKind` must be a string.".into(),
                    ))
                }
            },
            None => None,
        };
        let inputs: Vec<Value> = self
            .inputs
            .iter()
            .filter(|i| kind.as_deref().is_none_or(|k| k.is_empty() || k == i.kind))
            .map(|i| {
                let mut item = json!({
                    "inputName": i.name,
                    "inputUuid": i.uuid,
                    "inputKind": i.kind,
                    "inputKindCaps": i.caps,
                });
                if let Some(u) = &i.unversioned_kind {
                    item["unversionedInputKind"] = json!(u);
                }
                item
            })
            .collect();
        Ok(Some(json!({ "inputs": inputs })))
    }

    /// `GetInputSettings` (RHI:314-328): the whole settings object and the
    /// versioned kind.
    fn input_settings(&self, data: Option<&Value>) -> Result<Option<Value>, Fail> {
        let at = self.find_input(&pick(data, "inputName", "inputUuid")?)?;
        let input = &self.inputs[at];
        Ok(Some(
            json!({"inputSettings": input.settings, "inputKind": input.kind}),
        ))
    }
}

impl Output {
    /// `GetStreamStatus` (P576:5374-5385); the duration is 0 while not
    /// active (NUM:28-29).
    fn stream_status(&self) -> Value {
        let ms = if self.active { self.duration_ms } else { 0 };
        json!({
            "outputActive": self.active,
            "outputReconnecting": self.reconnecting,
            "outputTimecode": timecode(ms),
            "outputDuration": ms,
            "outputCongestion": 0.0,
            "outputBytes": self.bytes,
            "outputSkippedFrames": 0,
            "outputTotalFrames": ms * 60 / 1000,
        })
    }

    /// `GetRecordStatus` (P576:5449-5457).
    fn record_status(&self) -> Value {
        let ms = if self.active { self.duration_ms } else { 0 };
        json!({
            "outputActive": self.active,
            "outputPaused": self.paused,
            "outputTimecode": timecode(ms),
            "outputDuration": ms,
            "outputBytes": self.bytes,
        })
    }
}

impl State {
    /// Runs one served request on the model.
    fn process(&mut self, request_type: &str, data: Option<&Value>) -> Done {
        let mut done = Done {
            result: Ok(None),
            before: Vec::new(),
            after: Vec::new(),
        };
        let m = &self.cfg.model;
        let result = match request_type {
            "GetVersion" => Ok(Some(self.version())),
            "GetSceneList" => Ok(Some(m.scene_list())),
            "GetCurrentProgramScene" => Ok(Some(m.program_scene())),
            "SetCurrentProgramScene" => self.set_program(data, &mut done),
            "GetInputList" => m.input_list(data),
            "GetInputSettings" => m.input_settings(data),
            "SetInputSettings" => self.set_input_settings(data, &mut done),
            "GetStreamStatus" => Ok(Some(m.stream.stream_status())),
            "GetRecordStatus" => Ok(Some(m.record.record_status())),
            _ => Err((204, UNKNOWN.into())),
        };
        done.result = result;
        done
    }

    /// `GetVersion` (P576:2838-2848; `RequestHandler_General.cpp` isn't in
    /// hand). `availableRequests` is this fake's own list.
    fn version(&self) -> Value {
        let available: Vec<&str> = REQUESTS
            .iter()
            .copied()
            .filter(|r| self.serves(r))
            .collect();
        let mut d = Map::new();
        d.insert("obsVersion".into(), json!(self.cfg.obs_version));
        if let Some(v) = &self.cfg.ws_version {
            d.insert("obsWebSocketVersion".into(), v.clone());
        }
        if let Some(v) = &self.cfg.rpc_version {
            d.insert("rpcVersion".into(), v.clone());
        }
        d.insert("availableRequests".into(), json!(available));
        d.insert(
            "supportedImageFormats".into(),
            json!(["bmp", "jpeg", "jpg", "png"]),
        );
        d.insert("platform".into(), json!("windows"));
        d.insert("platformDescription".into(), json!("Windows 10 (10.0)"));
        Value::Object(d)
    }

    /// `SetCurrentProgramScene` (RHS:150-167): by name or uuid; the switch
    /// and its `CurrentProgramSceneChanged` (sent only when the program
    /// actually changed; `OBSStudioAPI.cpp` isn't among this fake's
    /// sources).
    fn set_program(
        &mut self,
        data: Option<&Value>,
        done: &mut Done,
    ) -> Result<Option<Value>, Fail> {
        let key = pick(data, "sceneName", "sceneUuid")?;
        let scene = self.cfg.model.find_scene(&key)?.clone();
        if self.cfg.model.program != scene.uuid {
            self.cfg.model.program.clone_from(&scene.uuid);
            let ev = Event::CurrentProgramSceneChanged {
                name: scene.name,
                uuid: scene.uuid,
            };
            match self.cfg.program_event {
                ProgramEvent::BeforeAnswer => done.before.push(ev),
                ProgramEvent::AfterAnswer => done.after.push(ev),
                ProgramEvent::Never => {}
            }
        }
        Ok(None)
    }

    /// `SetInputSettings` (RHI:345-379): the input first, then
    /// `inputSettings` (an object), then `overlay` (a boolean, default
    /// true): laid over the settings, or replacing them. Then
    /// `InputSettingsChanged` (Inputs; EH:586-597).
    fn set_input_settings(
        &mut self,
        data: Option<&Value>,
        done: &mut Done,
    ) -> Result<Option<Value>, Fail> {
        let at = self
            .cfg
            .model
            .find_input(&pick(data, "inputName", "inputUuid")?)?;
        let obj = fields(data)?;
        let settings = match present(obj, "inputSettings") {
            None => {
                return Err((
                    300,
                    "Your request is missing the `inputSettings` field.".into(),
                ))
            }
            Some(Value::Object(s)) => s.clone(),
            Some(_) => {
                return Err((
                    401,
                    "The field value of `inputSettings` must be an object.".into(),
                ))
            }
        };
        let overlay = match present(obj, "overlay") {
            None => true,
            Some(Value::Bool(b)) => *b,
            Some(_) => return Err((401, "The field value of `overlay` must be boolean.".into())),
        };
        let input = &mut self.cfg.model.inputs[at];
        if overlay {
            input.settings.extend(settings);
        } else {
            input.settings = settings;
        }
        done.after.push(Event::InputSettingsChanged {
            name: input.name.clone(),
            uuid: input.uuid.clone(),
            settings: input.settings.clone(),
        });
        Ok(None)
    }
}

// ---------------------------------------------------------------------------
// Auth (CRY)
// ---------------------------------------------------------------------------

/// CRY:30-43: 32 random bytes, each drawn below 255, in standard base64.
/// The salt (once per start) and each challenge (once per connection) are
/// made this way.
fn salt() -> String {
    let bytes: Vec<u8> = (0..32).map(|_| fastrand::u8(..255)).collect();
    STANDARD.encode(bytes)
}

/// CRY:45-56: base64(sha256(password ‖ salt)), standard padded base64 over
/// the strings' bytes.
pub fn secret(password: &str, salt: &str) -> String {
    let mut h = Sha256::new();
    h.update(password.as_bytes());
    h.update(salt.as_bytes());
    STANDARD.encode(h.finalize())
}

/// CRY:58-72: base64(sha256(secret ‖ challenge)), the string OBS compares
/// an Identify's `authentication` with, exactly.
pub fn auth(secret: &str, challenge: &str) -> String {
    let mut both = String::with_capacity(secret.len() + challenge.len());
    both.push_str(secret);
    both.push_str(challenge);
    STANDARD.encode(Sha256::digest(both.as_bytes()))
}

// ---------------------------------------------------------------------------
// Connections
// ---------------------------------------------------------------------------

async fn accept_loop(
    listener: TcpListener,
    inner: Arc<Inner>,
    tasks: Arc<Mutex<Vec<JoinHandle<()>>>>,
) {
    loop {
        let Ok((tcp, _peer)) = listener.accept().await else {
            // A failed accept (the OS out of something): try once more soon.
            tokio::time::sleep(Duration::from_millis(10)).await;
            continue;
        };
        let id = inner.with(|s| {
            let id = u64::try_from(s.sessions.len()).unwrap_or(u64::MAX - 1) + 1;
            s.sessions.push(SessionInfo {
                id,
                accepted_at: Instant::now(),
                upgraded: false,
                hello_sent: false,
                identified: false,
                subscriptions: None,
                rpc_version: None,
                challenge: None,
                outstanding: 0,
                header_bytes: 0,
                end: None,
            });
            id
        });
        let task = tokio::spawn(serve(inner.clone(), tcp, id));
        let mut tasks = lock(&tasks);
        tasks.retain(|t| !t.is_finished());
        tasks.push(task);
    }
}

async fn serve(inner: Arc<Inner>, tcp: TcpStream, id: u64) {
    let _ = tcp.set_nodelay(NO_NAGLE);
    let accept = inner.lock().accept;
    let end = match accept {
        Accept::CloseAtOnce => {
            drop(tcp);
            End::Dropped
        }
        Accept::Stall => stall(&inner, tcp, id).await,
        Accept::EndlessHeader => endless_header(&inner, tcp, id).await,
        Accept::Normal => websocket(&inner, tcp, id).await,
    };
    inner.with(|s| {
        s.live.retain(|(l, _)| *l != id);
        if let Some(info) = s.info_mut(id) {
            info.end = Some(end);
        }
    });
}

/// [`Accept::Stall`]: reads whatever comes and answers nothing.
async fn stall(inner: &Inner, mut tcp: TcpStream, id: u64) -> End {
    let mut buf = vec![0u8; 4096];
    loop {
        match tcp.read(&mut buf).await {
            Ok(0) => {
                inner.record(id, Dir::In, Frame::Eof);
                return End::Eof;
            }
            Ok(n) => inner.record(id, Dir::In, Frame::Raw(n)),
            Err(e) => {
                inner.record(id, Dir::In, Frame::Error(e.to_string()));
                return End::Error(e.to_string());
            }
        }
    }
}

/// [`Accept::EndlessHeader`]: once the request has arrived, a 101 status
/// line and a header line that doesn't end, until the client goes or 16
/// MiB have gone out.
async fn endless_header(inner: &Inner, mut tcp: TcpStream, id: u64) -> End {
    let mut head = Vec::new();
    let mut buf = [0u8; 1024];
    while !head.windows(4).any(|w| w == b"\r\n\r\n") && head.len() < 16 * 1024 {
        match tcp.read(&mut buf).await {
            Ok(0) | Err(_) => return End::Eof,
            Ok(n) => head.extend_from_slice(&buf[..n]),
        }
    }
    inner.record(id, Dir::In, Frame::Raw(head.len()));
    let start = b"HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nX-Endless: ";
    if tcp.write_all(start).await.is_err() {
        return End::Eof;
    }
    let chunk = vec![b'a'; 16 * 1024];
    let mut sent = start.len();
    while sent < ENDLESS_CAP {
        if tcp.write_all(&chunk).await.is_err() {
            break;
        }
        sent += chunk.len();
        inner.update(id, |i| i.header_bytes = sent);
    }
    inner.record(id, Dir::Out, Frame::Raw(sent));
    // Whatever happened, wait for the client to go.
    while matches!(tcp.read(&mut buf).await, Ok(n) if n > 0) {}
    End::Eof
}

/// [`Accept::Normal`]: the upgrade (tokio-websockets' server), then the
/// session.
async fn websocket(inner: &Arc<Inner>, tcp: TcpStream, id: u64) -> End {
    let io = FakeIo {
        tcp,
        framed: false,
        out: Vec::new(),
        sent: 0,
        cut: Cut::default(),
        flags: inner.io.clone(),
    };
    let builder = ServerBuilder::new().config(WsConfig::default().frame_size(ONE_FRAME));
    let (request, mut ws) = match builder.accept(io).await {
        Ok(x) => x,
        Err(e) => {
            inner.record(id, Dir::In, Frame::Error(e.to_string()));
            return End::Error(e.to_string());
        }
    };
    ws.get_mut().framed = true;
    let headers = request
        .headers()
        .iter()
        .map(|(k, v)| {
            (
                k.as_str().to_owned(),
                String::from_utf8_lossy(v.as_bytes()).into_owned(),
            )
        })
        .collect();
    inner.record(
        id,
        Dir::In,
        Frame::Upgrade {
            path: request.uri().to_string(),
            headers,
        },
    );
    let (tx, cmds) = mpsc::unbounded_channel();
    // WSS:252-259: the server's secret, and a challenge for this
    // connection, when auth is on.
    let (hello, auth) = inner.with(|s| {
        s.live.push((id, tx));
        let auth = s.cfg.password.as_deref().map(|pw| Auth {
            secret: secret(pw, &s.salt),
            salt: s.salt.clone(),
            challenge: salt(),
        });
        if let Some(info) = s.info_mut(id) {
            info.upgraded = true;
            info.challenge = auth.as_ref().map(|a| a.challenge.clone());
        }
        (s.hello, auth)
    });
    let session = Session {
        id,
        inner: inner.clone(),
        ws,
        cmds,
        hello_due: None,
        hello_sent: false,
        early: Vec::new(),
        auth,
        identify_passed: false,
        identified: false,
        pending: Vec::new(),
        outstanding: 0,
        closing: None,
    };
    session.run(hello).await
}

struct Auth {
    secret: String,
    salt: String,
    challenge: String,
}

/// Work a connection does later.
enum Pending {
    Identified {
        due: Instant,
        rpc: u64,
    },
    Answer {
        due: Instant,
        seen: usize,
        request_type: String,
        request_id: Value,
        data: Option<Value>,
        outcome: Outcome,
    },
}

impl Pending {
    fn due(&self) -> Instant {
        match self {
            Self::Identified { due, .. } | Self::Answer { due, .. } => *due,
        }
    }
}

enum Wake {
    Cmd(Option<Cmd>),
    Frame(Option<Result<Message, tokio_websockets::Error>>),
    Due,
}

enum Flow {
    Continue,
    End(End),
}

/// One upgraded connection: obs-websocket's per-session state machine
/// (`WSS:219-464`, WSP:57-353).
struct Session {
    id: u64,
    inner: Arc<Inner>,
    ws: WebSocketStream<FakeIo>,
    cmds: mpsc::UnboundedReceiver<Cmd>,
    /// When a delayed Hello goes out.
    hello_due: Option<Instant>,
    hello_sent: bool,
    /// Texts read before Hello went out, handled after it (OBS sends Hello
    /// before it handles any message, `WSS:219-294`).
    early: Vec<String>,
    auth: Option<Auth>,
    /// An Identify passed its checks; a second is 4008 (WSP:93-97).
    identify_passed: bool,
    /// Identified has gone out; until then only Identify is allowed
    /// (WSP:83-88).
    identified: bool,
    pending: Vec<Pending>,
    /// Requests received and not answered (D9: never more than one).
    outstanding: usize,
    /// The fake closed: until when it waits for the echo, and its code.
    closing: Option<(Instant, u16)>,
}

impl Session {
    async fn run(mut self, hello: Stage) -> End {
        let first = match hello {
            Stage::Now => self.hello().await,
            Stage::After(d) => {
                self.hello_due = Some(Instant::now() + d);
                Flow::Continue
            }
            Stage::Never => Flow::Continue,
            Stage::Close(code) => self.close(code, "The fake refused before Hello.").await,
        };
        if let Flow::End(end) = first {
            return end;
        }
        let far = Duration::from_secs(24 * 3600);
        let mut sleep = pin!(tokio::time::sleep(far));
        loop {
            let deadline = self.next_due().unwrap_or_else(|| Instant::now() + far);
            sleep
                .as_mut()
                .reset(tokio::time::Instant::from_std(deadline));
            let wake = poll_fn(|cx| {
                if let Poll::Ready(cmd) = self.cmds.poll_recv(cx) {
                    return Poll::Ready(Wake::Cmd(cmd));
                }
                if let Poll::Ready(frame) = Pin::new(&mut self.ws).poll_next(cx) {
                    return Poll::Ready(Wake::Frame(frame));
                }
                if sleep.as_mut().poll(cx).is_ready() {
                    return Poll::Ready(Wake::Due);
                }
                Poll::Pending
            })
            .await;
            let flow = match wake {
                Wake::Cmd(None) => Flow::End(End::Shutdown),
                Wake::Cmd(Some(cmd)) => self.on_cmd(cmd).await,
                Wake::Frame(frame) => self.on_frame(frame).await,
                Wake::Due => self.on_due().await,
            };
            if let Flow::End(end) = flow {
                return end;
            }
        }
    }

    fn next_due(&self) -> Option<Instant> {
        let pending = self.pending.iter().map(Pending::due);
        let others = [self.hello_due, self.closing.map(|(at, _)| at)];
        pending.chain(others.into_iter().flatten()).min()
    }

    // --- sending ------------------------------------------------------------

    async fn send(&mut self, msg: Message) -> bool {
        let ws = &mut self.ws;
        if poll_fn(|cx| Pin::new(&mut *ws).poll_ready(cx))
            .await
            .is_err()
        {
            return false;
        }
        if Pin::new(&mut *ws).start_send(msg).is_err() {
            return false;
        }
        poll_fn(|cx| Pin::new(&mut *ws).poll_flush(cx))
            .await
            .is_ok()
    }

    async fn send_text(&mut self, text: String) -> Flow {
        let ok = self.send(Message::text(text.clone())).await;
        self.inner.record(self.id, Dir::Out, Frame::Text(text));
        if ok {
            Flow::Continue
        } else {
            self.write_failed()
        }
    }

    fn write_failed(&self) -> Flow {
        let what = "a write failed".to_owned();
        self.inner
            .record(self.id, Dir::Out, Frame::Error(what.clone()));
        Flow::End(End::Error(what))
    }

    /// The fake's own close: the frame, then it waits up to 1 s for the
    /// echo, answering nothing more.
    async fn close(&mut self, code: u16, reason: &str) -> Flow {
        if self.closing.is_some() {
            return Flow::Continue;
        }
        let reason = clip(reason, 123);
        let msg = match CloseCode::try_from(code) {
            Ok(c) if !c.is_reserved() => Message::close(Some(c), reason),
            _ => Message::close(None, ""),
        };
        self.closing = Some((Instant::now() + CLOSE_WAIT, code));
        self.pending.clear();
        let ok = self.send(msg).await;
        self.inner.record(
            self.id,
            Dir::Out,
            Frame::Close(Some(code), reason.to_owned()),
        );
        if ok {
            Flow::Continue
        } else {
            Flow::End(End::ServerClosed(code))
        }
    }

    /// A close OBS makes only for a client that broke the protocol.
    async fn refuse(&mut self, code: u16, reason: &str) -> Flow {
        self.inner
            .violation(self.id, &format!("the fake closed {code}: {reason}"));
        self.close(code, reason).await
    }

    /// Hello (`WSS:247-262`, P576:126-176), then any text that came early.
    async fn hello(&mut self) -> Flow {
        let mut d = Map::new();
        let cfg = self.inner.lock().cfg.clone();
        if let Some(v) = cfg.obs_studio_version {
            d.insert("obsStudioVersion".into(), v);
        }
        if let Some(v) = cfg.ws_version {
            d.insert("obsWebSocketVersion".into(), v);
        }
        if let Some(v) = cfg.rpc_version {
            d.insert("rpcVersion".into(), v);
        }
        if let Some(a) = &self.auth {
            d.insert(
                "authentication".into(),
                json!({"challenge": a.challenge, "salt": a.salt}),
            );
        }
        self.hello_sent = true;
        self.inner.update(self.id, |i| i.hello_sent = true);
        if let Flow::End(end) = self.send_text(json!({"op": 0, "d": d}).to_string()).await {
            return Flow::End(end);
        }
        for text in std::mem::take(&mut self.early) {
            if self.closing.is_some() {
                break;
            }
            if let Flow::End(end) = self.on_text(&text).await {
                return Flow::End(end);
            }
        }
        Flow::Continue
    }

    /// Identified (WSP:155-168): the session counts as identified (for
    /// events too) from here.
    async fn send_identified(&mut self, rpc: u64) -> Flow {
        self.identified = true;
        self.inner.update(self.id, |i| i.identified = true);
        self.send_text(json!({"op": 2, "d": {"negotiatedRpcVersion": rpc}}).to_string())
            .await
    }

    /// A request's answer (WSP:222-232, P576:330-395).
    async fn answer(
        &mut self,
        seen: usize,
        request_type: &str,
        request_id: &Value,
        result: Result<Option<Value>, Fail>,
    ) -> Flow {
        let (code, comment, data) = match result {
            Ok(data) => (100, None, data),
            Err((code, comment)) => (code, Some(comment), None),
        };
        if (300..500).contains(&code) {
            self.inner.violation(
                self.id,
                &format!("{request_type} was malformed: answered {code}"),
            );
        }
        let mut status = json!({"result": code == 100, "code": code});
        if let Some(comment) = comment {
            status["comment"] = json!(comment);
        }
        let mut d = json!({
            "requestType": request_type,
            "requestId": request_id,
            "requestStatus": status,
        });
        if let Some(data) = data.filter(Value::is_object) {
            d["responseData"] = data;
        }
        let flow = self.send_text(json!({"op": 7, "d": d}).to_string()).await;
        self.answered(seen, Some(code));
        flow
    }

    /// `seen` is no longer outstanding; `code` is its answer, if one went
    /// out.
    fn answered(&mut self, seen: usize, code: Option<u16>) {
        self.outstanding = self.outstanding.saturating_sub(1);
        let (id, outstanding) = (self.id, self.outstanding);
        self.inner.with(|s| {
            if let (Some(code), Some(r)) = (code, s.requests.get_mut(seen)) {
                r.answer = Some((Instant::now(), code));
            }
            if let Some(info) = s.info_mut(id) {
                info.outstanding = outstanding;
            }
        });
    }

    /// An event this request caused: to the other connections as a
    /// broadcast, and to this one in line, so its order beside the answer
    /// is the script's.
    async fn emit(&mut self, event: &Event) -> Flow {
        let (intent, text) = event.wire();
        let id = self.id;
        let mine = self.inner.with(|s| {
            s.broadcast(intent, &text, Some(id));
            s.wants(id, intent)
        });
        if mine {
            self.send_text(text).await
        } else {
            Flow::Continue
        }
    }

    // --- reading ------------------------------------------------------------

    async fn on_frame(&mut self, frame: Option<Result<Message, tokio_websockets::Error>>) -> Flow {
        let msg = match frame {
            None => {
                self.inner.record(self.id, Dir::In, Frame::Eof);
                return Flow::End(match self.closing {
                    Some((_, code)) => End::ServerClosed(code),
                    None => End::Eof,
                });
            }
            Some(Err(e)) => {
                self.inner
                    .record(self.id, Dir::In, Frame::Error(e.to_string()));
                return Flow::End(End::Error(e.to_string()));
            }
            Some(Ok(msg)) => msg,
        };
        if msg.is_close() {
            return self.on_close(&msg).await;
        }
        let payload = msg.as_payload().to_vec();
        if msg.is_ping() {
            // The pong is tokio-websockets' own (stream.rs:353-358), cut
            // by `FakeIo` while pings are ignored.
            self.inner.record(self.id, Dir::In, Frame::Ping(payload));
            return Flow::Continue;
        }
        if msg.is_pong() {
            self.inner.record(self.id, Dir::In, Frame::Pong(payload));
            return Flow::Continue;
        }
        if msg.is_binary() {
            self.inner
                .record(self.id, Dir::In, Frame::Binary(payload.len()));
            if self.closing.is_some() {
                return Flow::Continue;
            }
            // WSS:370-376.
            return self
                .refuse(
                    4002,
                    "Your session encoding is set to Json, but a binary message was received.",
                )
                .await;
        }
        let text = String::from_utf8_lossy(&payload).into_owned();
        self.inner
            .record(self.id, Dir::In, Frame::Text(text.clone()));
        if self.closing.is_some() {
            return Flow::Continue;
        }
        if !self.hello_sent {
            self.early.push(text);
            return Flow::Continue;
        }
        self.on_text(&text).await
    }

    /// The client's close: the echo of the fake's own, or the client's
    /// first, which tokio-websockets has already queued an echo for
    /// (stream.rs:340-348).
    async fn on_close(&mut self, msg: &Message) -> Flow {
        let (code, reason) = msg.as_close().map_or((None, String::new()), |(c, r)| {
            let code = u16::from(c);
            // 1005 is what tokio-websockets reads a close without a code as.
            ((code != 1005).then_some(code), r.to_owned())
        });
        self.inner
            .record(self.id, Dir::In, Frame::Close(code, reason));
        if let Some((_, ours)) = self.closing {
            return Flow::End(End::ServerClosed(ours));
        }
        if self.inner.lock().close_echo {
            // The next poll flushes the queued echo and ends the stream.
            let ws = &mut self.ws;
            let _ = poll_fn(|cx| Pin::new(&mut *ws).poll_next(cx)).await;
            self.inner
                .record(self.id, Dir::Out, Frame::Close(code, String::new()));
        } else {
            // Never poll the stream any more (that would flush the echo):
            // read the socket itself until the client's FIN.
            let mut buf = [0u8; 1024];
            let io = self.ws.get_mut();
            while matches!(io.read(&mut buf).await, Ok(n) if n > 0) {}
            self.inner.record(self.id, Dir::In, Frame::Eof);
        }
        Flow::End(End::ClientClosed(code))
    }

    /// A text frame (`WSS:365-436`, then WSP:69-88's checks), in their order.
    async fn on_text(&mut self, text: &str) -> Flow {
        let v: Value = match serde_json::from_str(text) {
            Ok(v) => v,
            Err(e) => {
                return self
                    .refuse(4002, &format!("Unable to decode Json: {e}"))
                    .await
            }
        };
        let Some(obj) = v.as_object() else {
            return self.refuse(4002, "You sent a non-object payload.").await;
        };
        if !self.identified && obj.contains_key("request-type") {
            return self
                .refuse(
                    4010,
                    "You appear to be attempting to connect with the pre-5.0.0 plugin protocol.",
                )
                .await;
        }
        let Some(op) = obj.get("op") else {
            return self.refuse(4006, "Your request is missing an `op`.").await;
        };
        if !op.is_number() {
            return self.refuse(4006, "Your `op` is not a number.").await;
        }
        let d = match obj.get("d") {
            None | Some(Value::Null) => {
                return self
                    .refuse(4003, "Your payload is missing data (`d`).")
                    .await
            }
            Some(d) if !d.is_object() => {
                return self
                    .refuse(4004, "Your payload's data (`d`) is not an object.")
                    .await
            }
            Some(d) => d.clone(),
        };
        // A float or negative `op` is no op code.
        let op = op.as_u64();
        if !self.identified && op != Some(1) {
            return self
                .refuse(
                    4007,
                    "You attempted to send a non-Identify message while not identified.",
                )
                .await;
        }
        match op {
            Some(1) => self.identify(&d).await,
            Some(3) => self.reidentify(&d).await,
            Some(6) => self.request(&d).await,
            Some(8) => {
                self.refuse(
                    4006,
                    "The fake serves no RequestBatch (Divora never batches).",
                )
                .await
            }
            other => {
                let n = other.map_or_else(|| "?".to_owned(), |n| n.to_string());
                self.refuse(4006, &format!("Unknown OpCode: {n}")).await
            }
        }
    }

    /// Identify (WSP:91-170), in its order: already identified 4008; auth
    /// 4009 before `rpcVersion` is looked at; `rpcVersion` 4003/4004, then
    /// 4010; `eventSubscriptions` 4004.
    async fn identify(&mut self, d: &Value) -> Flow {
        if self.identify_passed {
            return self
                .refuse(
                    4008,
                    "You are already Identified with the obs-websocket server.",
                )
                .await;
        }
        let id = self.id;
        let (accept_rpc, expect_mask, stage) = self.inner.with(|s| {
            s.identifies.push((id, d.clone()));
            (s.cfg.accept_rpc.clone(), s.cfg.expect_mask, s.identified)
        });
        if let Some(extra) = d.as_object().and_then(|o| {
            o.keys().find(|k| {
                !["rpcVersion", "authentication", "eventSubscriptions"].contains(&k.as_str())
            })
        }) {
            self.inner.violation(
                id,
                &format!("Identify carried `{extra}`, a field it doesn't have"),
            );
        }
        if let Some(flow) = self.check_auth(d).await {
            return flow;
        }
        let Some(rpc) = d.get("rpcVersion") else {
            return self
                .refuse(4003, "Your payload's data is missing an `rpcVersion`.")
                .await;
        };
        let Some(rpc) = rpc.as_u64() else {
            return self
                .refuse(4004, "Your `rpcVersion` is not an unsigned number.")
                .await;
        };
        // WSP:139 keeps it in a uint8_t.
        let rpc = rpc & 0xff;
        if !accept_rpc.contains(&rpc) {
            return self
                .close(
                    4010,
                    "Your requested RPC version is not supported by this server.",
                )
                .await;
        }
        let subs = match d.get("eventSubscriptions") {
            None => ALL,
            Some(v) => match v.as_u64() {
                Some(m) => m,
                None => {
                    return self
                        .refuse(4004, "Your `eventSubscriptions` is not an unsigned number.")
                        .await
                }
            },
        };
        if let Some(want) = expect_mask.filter(|w| *w != subs) {
            self.inner
                .violation(id, &format!("Identify subscribed to {subs}, not {want}"));
        }
        self.identify_passed = true;
        self.inner.update(id, |i| {
            i.subscriptions = Some(subs);
            i.rpc_version = Some(rpc);
        });
        match stage {
            Stage::Now => self.send_identified(rpc).await,
            Stage::After(delay) => {
                self.pending.push(Pending::Identified {
                    due: Instant::now() + delay,
                    rpc,
                });
                Flow::Continue
            }
            Stage::Never => Flow::Continue,
            Stage::Close(code) => self.close(code, "The fake closed at Identify.").await,
        }
    }

    /// WSP:99-125, CRY:58-72. `Some` when the connection is closing.
    async fn check_auth(&mut self, d: &Value) -> Option<Flow> {
        let given = d.get("authentication");
        let Some(a) = &self.auth else {
            if given.is_some() {
                self.inner.violation(
                    self.id,
                    "Identify carried `authentication` though Hello asked for none",
                );
            }
            return None;
        };
        let expected = auth(&a.secret, &a.challenge);
        let reason = match given {
            None => "Your payload's data is missing an `authentication` string, however authentication is required.",
            Some(v) if !v.is_string() => "Your `authentication` field is not a string.",
            Some(v) if v.as_str() != Some(expected.as_str()) => "Authentication failed.",
            Some(_) => return None,
        };
        Some(self.close(4009, reason).await)
    }

    /// Reidentify (WSP:171-188): a new mask, answered with Identified.
    async fn reidentify(&mut self, d: &Value) -> Flow {
        if let Some(v) = d.get("eventSubscriptions") {
            let Some(m) = v.as_u64() else {
                return self
                    .refuse(4004, "Your `eventSubscriptions` is not an unsigned number.")
                    .await;
            };
            self.inner.update(self.id, |i| i.subscriptions = Some(m));
        }
        let rpc = self
            .inner
            .lock()
            .info(self.id)
            .and_then(|i| i.rpc_version)
            .unwrap_or(1);
        self.send_text(json!({"op": 2, "d": {"negotiatedRpcVersion": rpc}}).to_string())
            .await
    }

    /// Request (WSP:190-233): `requestId` and `requestType` checked, then
    /// 207 while not ready, then (the fake's own) 204 for a type it doesn't
    /// serve, then the script.
    async fn request(&mut self, d: &Value) -> Flow {
        let Some(request_id) = d.get("requestId").cloned() else {
            return self
                .refuse(4003, "Your payload data is missing a `requestId`.")
                .await;
        };
        let Some(request_type) = d.get("requestType") else {
            return self
                .refuse(4003, "Your payload's data is missing an `requestType`.")
                .await;
        };
        let Some(request_type) = request_type.as_str().map(str::to_owned) else {
            return self
                .refuse(4004, "Your `requestType` is not a string.")
                .await;
        };
        let data = d.get("requestData").cloned();
        if self.outstanding > 0 {
            self.inner.violation(
                self.id,
                &format!(
                    "{request_type} sent while {} request(s) were unanswered (D9)",
                    self.outstanding
                ),
            );
        }
        self.outstanding += 1;
        let (id, outstanding) = (self.id, self.outstanding);
        let (seen, ready, serves, (delay, outcome)) = self.inner.with(|s| {
            s.requests.push(Seen {
                session: id,
                at: Instant::now(),
                request_type: request_type.clone(),
                request_id: request_id.clone(),
                data: data.clone(),
                answer: None,
            });
            s.max_outstanding = s.max_outstanding.max(outstanding);
            if let Some(info) = s.info_mut(id) {
                info.outstanding = outstanding;
            }
            let serves = s.serves(&request_type);
            let rule = if s.ready && serves {
                s.take_rule(&request_type)
            } else {
                (Duration::ZERO, Outcome::Answer)
            };
            (s.requests.len() - 1, s.ready, serves, rule)
        });
        if !ready {
            return self
                .answer(
                    seen,
                    &request_type,
                    &request_id,
                    Err((207, NOT_READY.into())),
                )
                .await;
        }
        if !serves {
            self.inner
                .violation(id, &format!("{request_type} isn't a request it may send"));
            return self
                .answer(seen, &request_type, &request_id, Err((204, UNKNOWN.into())))
                .await;
        }
        match outcome {
            Outcome::Drop => Flow::Continue,
            _ if delay.is_zero() => {
                self.finish(seen, &request_type, &request_id, data, outcome)
                    .await
            }
            _ => {
                self.pending.push(Pending::Answer {
                    due: Instant::now() + delay,
                    seen,
                    request_type,
                    request_id,
                    data,
                    outcome,
                });
                Flow::Continue
            }
        }
    }

    /// A request's outcome, when its time comes.
    async fn finish(
        &mut self,
        seen: usize,
        request_type: &str,
        request_id: &Value,
        data: Option<Value>,
        outcome: Outcome,
    ) -> Flow {
        match outcome {
            Outcome::Answer | Outcome::ApplyNoAnswer => {
                let answer = outcome == Outcome::Answer;
                let done = self.inner.with(|s| s.process(request_type, data.as_ref()));
                for ev in &done.before {
                    if let Flow::End(end) = self.emit(ev).await {
                        return Flow::End(end);
                    }
                }
                if answer {
                    if let Flow::End(end) = self
                        .answer(seen, request_type, request_id, done.result)
                        .await
                    {
                        return Flow::End(end);
                    }
                }
                for ev in &done.after {
                    if let Flow::End(end) = self.emit(ev).await {
                        return Flow::End(end);
                    }
                }
                Flow::Continue
            }
            Outcome::Code(code) => {
                let comment = if code == 207 {
                    NOT_READY.to_owned()
                } else {
                    format!("The fake's script answered {code}.")
                };
                self.answer(seen, request_type, request_id, Err((code, comment)))
                    .await
            }
            Outcome::NotReady => {
                self.answer(seen, request_type, request_id, Err((207, NOT_READY.into())))
                    .await
            }
            Outcome::Close(code) => {
                self.answered(seen, None);
                self.close(code, "The fake's script closed here.").await
            }
            Outcome::Drop => Flow::Continue,
        }
    }

    async fn on_due(&mut self) -> Flow {
        let now = Instant::now();
        if let Some((at, code)) = self.closing {
            if now >= at {
                // No echo in time.
                return Flow::End(End::ServerClosed(code));
            }
        }
        if self.hello_due.is_some_and(|at| at <= now) {
            self.hello_due = None;
            if let Flow::End(end) = self.hello().await {
                return Flow::End(end);
            }
        }
        while let Some(at) = self
            .pending
            .iter()
            .enumerate()
            .filter(|(_, p)| p.due() <= now)
            .min_by_key(|(_, p)| p.due())
            .map(|(i, _)| i)
        {
            let flow = match self.pending.remove(at) {
                Pending::Identified { rpc, .. } => self.send_identified(rpc).await,
                Pending::Answer {
                    seen,
                    request_type,
                    request_id,
                    data,
                    outcome,
                    ..
                } => {
                    self.finish(seen, &request_type, &request_id, data, outcome)
                        .await
                }
            };
            if let Flow::End(end) = flow {
                return Flow::End(end);
            }
        }
        Flow::Continue
    }

    // --- what a test asks ---------------------------------------------------

    async fn on_cmd(&mut self, cmd: Cmd) -> Flow {
        if self.closing.is_some() {
            return Flow::Continue;
        }
        match cmd {
            Cmd::Close(code, reason) => self.close(code, &reason).await,
            Cmd::Text(text) => self.send_text(text).await,
            Cmd::Raw(bytes) => {
                let io = self.ws.get_mut();
                io.out.extend_from_slice(&bytes);
                let ok = poll_fn(|cx| Pin::new(&mut *io).poll_flush(cx))
                    .await
                    .is_ok();
                self.inner
                    .record(self.id, Dir::Out, Frame::Raw(bytes.len()));
                if ok {
                    Flow::Continue
                } else {
                    self.write_failed()
                }
            }
            Cmd::Flood(n) => self.flood(n).await,
            Cmd::Big(len) => {
                let ok = self.send(Message::text(padded_event(len))).await;
                self.inner.record(self.id, Dir::Out, Frame::Big(len));
                if ok {
                    Flow::Continue
                } else {
                    self.write_failed()
                }
            }
        }
    }

    /// `n` `SceneNameChanged` queued back to back (tokio-websockets flushes
    /// past 8 KiB, stream.rs:470-478), then one flush.
    async fn flood(&mut self, n: usize) -> Flow {
        let ws = &mut self.ws;
        let mut ok = true;
        for i in 0..n {
            let (_, text) = Event::SceneNameChanged {
                uuid: GAME.into(),
                old: format!("Game {i}"),
                name: format!("Game {}", i + 1),
            }
            .wire();
            ok = poll_fn(|cx| Pin::new(&mut *ws).poll_ready(cx))
                .await
                .is_ok()
                && Pin::new(&mut *ws).start_send(Message::text(text)).is_ok();
            if !ok {
                break;
            }
        }
        ok = ok
            && poll_fn(|cx| Pin::new(&mut *ws).poll_flush(cx))
                .await
                .is_ok();
        self.inner.record(self.id, Dir::Out, Frame::Flood(n));
        if ok {
            Flow::Continue
        } else {
            self.write_failed()
        }
    }
}

/// A valid `SceneNameChanged` padded out to `len` bytes (or as short as it
/// can be), so a link without a frame cap would read it as an event.
fn padded_event(len: usize) -> String {
    let head = r#"{"op":5,"d":{"eventType":"SceneNameChanged","eventIntent":4,"eventData":{"sceneUuid":"x","oldSceneName":"x","sceneName":""#;
    let tail = r#""}}}"#;
    let pad = len.saturating_sub(head.len() + tail.len());
    let mut s = String::with_capacity(head.len() + pad + tail.len());
    s.push_str(head);
    s.extend(std::iter::repeat_n('a', pad));
    s.push_str(tail);
    s
}

/// `s` cut to at most `max` bytes, on a character boundary
/// (`Message::close` asserts 123, TWS/src/proto/types.rs:337-350).
fn clip(s: &str, max: usize) -> &str {
    let mut end = s.len().min(max);
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

// ---------------------------------------------------------------------------
// The socket
// ---------------------------------------------------------------------------

/// Shared by every connection of one fake.
#[derive(Default)]
struct IoFlags {
    drop_pongs: AtomicBool,
    pongs_sent: AtomicUsize,
    pongs_dropped: AtomicUsize,
}

/// The server's end of one socket. Reads pass straight through. Until the
/// upgrade is done, writes do too; after it, they wait for a flush and go
/// out in one write, and while pings are ignored every pong frame
/// tokio-websockets writes is cut out. Server frames aren't masked (RFC
/// 6455 §5.1), so the cut is a parse of each frame's header.
struct FakeIo {
    tcp: TcpStream,
    /// The upgrade is done: writes are WebSocket frames.
    framed: bool,
    out: Vec<u8>,
    /// How much of `out` has been written.
    sent: usize,
    cut: Cut,
    flags: Arc<IoFlags>,
}

/// Where the outgoing byte stream is: in a frame's header (`have` of
/// `need` bytes) or its payload (`left` bytes, kept or cut).
#[derive(Default)]
struct Cut {
    head: [u8; 14],
    have: usize,
    need: usize,
    left: u64,
    keep: bool,
}

impl Cut {
    fn feed(&mut self, mut buf: &[u8], flags: &IoFlags, out: &mut Vec<u8>) {
        while !buf.is_empty() {
            if self.left > 0 {
                let n = usize::try_from(self.left).map_or(buf.len(), |l| l.min(buf.len()));
                let (now, rest) = buf.split_at(n);
                if self.keep {
                    out.extend_from_slice(now);
                }
                self.left -= u64::try_from(n).unwrap_or(self.left);
                buf = rest;
                continue;
            }
            self.head[self.have] = buf[0];
            self.have += 1;
            buf = &buf[1..];
            if self.have == 2 {
                let ext = match self.head[1] & 0x7f {
                    126 => 2,
                    127 => 8,
                    _ => 0,
                };
                let mask = if self.head[1] & 0x80 == 0 { 0 } else { 4 };
                self.need = 2 + ext + mask;
            }
            if self.have < 2 || self.have < self.need {
                continue;
            }
            let len = match self.head[1] & 0x7f {
                126 => u64::from(u16::from_be_bytes([self.head[2], self.head[3]])),
                127 => u64::from_be_bytes(self.head[2..10].try_into().unwrap()),
                n => u64::from(n),
            };
            let pong = self.head[0] & 0x0f == 0x0a;
            self.keep = !(pong && flags.drop_pongs.load(Ordering::SeqCst));
            if pong {
                let count = if self.keep {
                    &flags.pongs_sent
                } else {
                    &flags.pongs_dropped
                };
                count.fetch_add(1, Ordering::SeqCst);
            }
            if self.keep {
                out.extend_from_slice(&self.head[..self.have]);
            }
            self.left = len;
            self.have = 0;
            self.need = 0;
        }
    }
}

impl FakeIo {
    fn drain(&mut self, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        while self.sent < self.out.len() {
            let n = ready!(Pin::new(&mut self.tcp).poll_write(cx, &self.out[self.sent..]))?;
            if n == 0 {
                return Poll::Ready(Err(io::ErrorKind::WriteZero.into()));
            }
            self.sent += n;
        }
        self.out.clear();
        self.sent = 0;
        Poll::Ready(Ok(()))
    }
}

impl AsyncRead for FakeIo {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().tcp).poll_read(cx, buf)
    }
}

impl AsyncWrite for FakeIo {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        let this = self.get_mut();
        if !this.framed {
            return Pin::new(&mut this.tcp).poll_write(cx, buf);
        }
        this.cut.feed(buf, &this.flags, &mut this.out);
        Poll::Ready(Ok(buf.len()))
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        ready!(this.drain(cx))?;
        Pin::new(&mut this.tcp).poll_flush(cx)
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        ready!(this.drain(cx))?;
        Pin::new(&mut this.tcp).poll_shutdown(cx)
    }
}

/// The fake checked with P576 and the server sources, through a raw
/// tokio-websockets client and hand-written JSON (never the link): the
/// handshake, CRY's auth, WSP's checks in their order, 204 for an unknown
/// request, events gated by subscriptions, and every control knob.
mod tests {
    use super::*;
    use tokio_websockets::{ClientBuilder, Limits};

    type Ws = WebSocketStream<TcpStream>;

    /// How long a test waits for a frame that must come.
    const T: Duration = Duration::from_secs(2);

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    /// D10's mask: General, Scenes, Outputs.
    const MASK69: Option<u64> = Some(GENERAL | SCENES | OUTPUTS);

    fn upgrade_request(fake: &FakeObs) -> String {
        format!(
            "GET / HTTP/1.1\r\nHost: {}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n\
             Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Version: 13\r\n\r\n",
            fake.addr()
        )
    }

    /// The upgrade answer's head, read a byte at a time so nothing after
    /// it is taken.
    async fn read_head(tcp: &mut TcpStream) -> String {
        let mut head = Vec::new();
        let mut b = [0u8; 1];
        while !head.ends_with(b"\r\n\r\n") {
            let n = tcp.read(&mut b).await.unwrap();
            assert_eq!(n, 1, "the answer ended: {}", String::from_utf8_lossy(&head));
            head.push(b[0]);
        }
        String::from_utf8(head).unwrap()
    }

    /// The HTTP upgrade by hand (RFC 6455 §4.1), then tokio-websockets'
    /// client framing on the same socket.
    async fn upgrade(fake: &FakeObs, limits: Limits) -> Ws {
        let mut tcp = TcpStream::connect(fake.addr()).await.unwrap();
        tcp.write_all(upgrade_request(fake).as_bytes())
            .await
            .unwrap();
        let head = read_head(&mut tcp).await;
        assert!(head.starts_with("HTTP/1.1 101 "), "{head}");
        ClientBuilder::new().limits(limits).take_over(tcp)
    }

    async fn connect(fake: &FakeObs) -> Ws {
        upgrade(fake, Limits::unlimited()).await
    }

    async fn send(ws: &mut Ws, msg: Message) {
        poll_fn(|cx| Pin::new(&mut *ws).poll_ready(cx))
            .await
            .unwrap();
        Pin::new(&mut *ws).start_send(msg).unwrap();
        poll_fn(|cx| Pin::new(&mut *ws).poll_flush(cx))
            .await
            .unwrap();
    }

    async fn send_json(ws: &mut Ws, v: &Value) {
        send(ws, Message::text(v.to_string())).await;
    }

    /// The next frame, pings and pongs too; `None` at the stream's end.
    async fn raw_next(ws: &mut Ws) -> Option<Result<Message, tokio_websockets::Error>> {
        tokio::time::timeout(T, poll_fn(|cx| Pin::new(&mut *ws).poll_next(cx)))
            .await
            .expect("a frame within 2 s")
    }

    /// The next frame that isn't a ping or a pong.
    async fn next(ws: &mut Ws) -> Message {
        loop {
            let m = raw_next(ws).await.expect("the stream ended").unwrap();
            if !m.is_ping() && !m.is_pong() {
                return m;
            }
        }
    }

    async fn recv(ws: &mut Ws) -> Value {
        let m = next(ws).await;
        serde_json::from_str(m.as_text().expect("a text frame")).unwrap()
    }

    /// The close the fake sent; the client's echo flushed.
    async fn closed(ws: &mut Ws) -> u16 {
        let m = next(ws).await;
        let code = m.as_close().map(|(c, _)| u16::from(c)).expect("a close");
        let _ = poll_fn(|cx| Pin::new(&mut *ws).poll_next(cx)).await;
        code
    }

    /// Nothing but pings and pongs arrives for `d`.
    async fn quiet(ws: &mut Ws, d: Duration) -> bool {
        let wait = async {
            while let Some(Ok(m)) = poll_fn(|cx| Pin::new(&mut *ws).poll_next(cx)).await {
                if !m.is_ping() && !m.is_pong() {
                    return;
                }
            }
        };
        tokio::time::timeout(d, wait).await.is_err()
    }

    /// P576:199-210's Identify.
    fn identify(auth: Option<&str>, mask: Option<u64>) -> Value {
        let mut d = json!({"rpcVersion": 1});
        if let Some(a) = auth {
            d["authentication"] = json!(a);
        }
        if let Some(m) = mask {
            d["eventSubscriptions"] = json!(m);
        }
        json!({"op": 1, "d": d})
    }

    /// The client's own reading of P576:99-104 for this Hello, written out
    /// here rather than through the fake's [`secret`] and [`auth`], so the
    /// handshake tests check the server's side with a second copy.
    fn auth_for(hello: &Value, password: &str) -> String {
        let a = &hello["d"]["authentication"];
        let salt = a["salt"].as_str().unwrap();
        let challenge = a["challenge"].as_str().unwrap();
        let hashed = |bytes: Vec<u8>| STANDARD.encode(Sha256::digest(&bytes));
        let secret = hashed([password.as_bytes(), salt.as_bytes()].concat());
        hashed([secret.as_bytes(), challenge.as_bytes()].concat())
    }

    /// Upgrade, Hello, Identify, Identified (P576:48-71).
    async fn identified(fake: &FakeObs, password: Option<&str>, mask: Option<u64>) -> Ws {
        let mut ws = connect(fake).await;
        let hello = recv(&mut ws).await;
        assert_eq!(hello["op"], 0);
        let a = password.map(|pw| auth_for(&hello, pw));
        send_json(&mut ws, &identify(a.as_deref(), mask)).await;
        assert_eq!(
            recv(&mut ws).await,
            json!({"op": 2, "d": {"negotiatedRpcVersion": 1}})
        );
        ws
    }

    fn request_json(request_type: &str, data: Option<Value>) -> Value {
        let mut d = json!({"requestType": request_type, "requestId": request_type});
        if let Some(data) = data {
            d["requestData"] = data;
        }
        json!({"op": 6, "d": d})
    }

    /// P576:313-326's Request; its answer's `d`, any event before it
    /// skipped.
    async fn request(ws: &mut Ws, request_type: &str, data: Option<Value>) -> Value {
        send_json(ws, &request_json(request_type, data)).await;
        loop {
            let v = recv(ws).await;
            if v["op"] == 7 {
                assert_eq!(v["d"]["requestId"], request_type);
                assert_eq!(v["d"]["requestType"], request_type);
                return v["d"].clone();
            }
        }
    }

    fn code(answer: &Value) -> u64 {
        answer["requestStatus"]["code"].as_u64().unwrap()
    }

    fn keys(v: &Value) -> Vec<String> {
        let mut k: Vec<String> = v.as_object().unwrap().keys().cloned().collect();
        k.sort();
        k
    }

    fn event_type(v: &Value) -> &str {
        assert_eq!(v["op"], 5, "{v}");
        v["d"]["eventType"].as_str().unwrap()
    }

    /// CRY:45-72 checked with strings computed outside Rust, with Python
    /// `hashlib` and Node `crypto` agreeing (2026-11-03): P576:92-104's own
    /// inputs, a non-ASCII password and an empty one. P576:206's example
    /// string isn't any of them: the doc's example isn't computed from its
    /// inputs.
    #[test]
    fn secret_and_auth_match_python_and_node() {
        let rows = [
            (
                "supersecretpassword",
                "lM1GncleQOaCu9lT1yeUZhFYnqhsLLP1G5lAGo3ixaI=",
                "+IxH4CnCiqpX1rM9scsNynZzbOe4KhDeYcTNS3PDaeY=",
                "H1IfVz1pSREUQzbFTVnX/Tyb+gMhMik5x7yUBCY0PTs=",
                "1Ct943GAT+6YQUUX47Ia/ncufilbe6+oD6lY+5kaCu4=",
            ),
            (
                "b2 fake \u{2713} p\u{e4}ss",
                "QjJzYWx0LWZvci10aGUtZmFrZS1zZWxmLXRlc3Qh",
                "QjJjaGFsbGVuZ2UtMDEyMzQ1Njc4OWFiY2RlZg==",
                "rQ0JgbO2NVvy/zoeSZbridwUvOa/oTrmQ4UW0TfiEGI=",
                "otGGBdEd4PGZ3JalcngxQF9QXsouGDCNpYSPH9ur3sI=",
            ),
            (
                "",
                "c2FsdA==",
                "Y2hhbGxlbmdl",
                "8BNXdbixYUuAdvQEWcf8sKp2FrAgpj2di4DLL+/Lh7U=",
                "GPsEJ6KSp8eX06jpjh8EM4OXqycJkb4o94LdGSN/YYg=",
            ),
        ];
        for (password, salt, challenge, want_secret, want_auth) in rows {
            let s = secret(password, salt);
            assert_eq!(s, want_secret, "{password:?}");
            assert_eq!(auth(&s, challenge), want_auth, "{password:?}");
        }
        assert_ne!(rows[0].4, "Dj6cLS+jrNA0HpCArRg0Z/Fc+YHdt2FQfAvgD1mip6Y=");
    }

    /// P576:126-176 and `WSS:247-262`: Hello's keys; one salt per start and a
    /// challenge per connection (`WSS:85-86`, :254-257), each 32 bytes in
    /// base64 (CRY:30-43); a restart or a new password makes a new salt.
    #[test]
    fn hello_has_a_salt_per_start_and_a_challenge_per_connection() {
        guarded(async {
            let fake = FakeObs::start(Config::default().ws_version("5.3.3")).await;
            let mut open = connect(&fake).await;
            let hello = recv(&mut open).await;
            assert_eq!(hello["d"]["obsWebSocketVersion"], "5.3.3");
            assert_eq!(
                keys(&hello["d"]),
                ["obsStudioVersion", "obsWebSocketVersion", "rpcVersion"]
            );
            assert_eq!(hello["d"]["rpcVersion"], 1);
            assert_eq!(fake.session(1).unwrap().challenge, None);

            fake.set_password(Some("pw"));
            let (mut a, mut b) = (connect(&fake).await, connect(&fake).await);
            let (ha, hb) = (recv(&mut a).await, recv(&mut b).await);
            assert_eq!(keys(&ha["d"]["authentication"]), ["challenge", "salt"]);
            let (sa, sb) = (&ha["d"]["authentication"], &hb["d"]["authentication"]);
            assert_eq!(sa["salt"], sb["salt"]);
            assert_eq!(sa["salt"], fake.salt());
            assert_ne!(sa["challenge"], sb["challenge"]);
            for s in [&sa["salt"], &sa["challenge"], &sb["challenge"]] {
                assert_eq!(STANDARD.decode(s.as_str().unwrap()).unwrap().len(), 32);
            }
            assert_eq!(
                fake.session(2).unwrap().challenge.as_deref(),
                sa["challenge"].as_str()
            );
            let before = fake.salt();
            fake.restart();
            let mut c = connect(&fake).await;
            let hc = recv(&mut c).await;
            assert_ne!(hc["d"]["authentication"]["salt"], json!(before));
            assert_eq!(fake.port(), fake.addr().port());
        });
    }

    /// WSP:91-170: the right auth (CRY) is Identified, the mask and RPC
    /// version recorded; a missing, non-string or wrong one closes 4009,
    /// and auth is looked at before `rpcVersion` (WSP:99-144). None of it is
    /// a violation.
    #[test]
    fn identify_checks_auth_first() {
        guarded(async {
            let fake = FakeObs::start(Config::default().with_password("correct horse")).await;
            let _ok = identified(&fake, Some("correct horse"), MASK69).await;
            let info = fake.session(1).unwrap();
            assert_eq!((info.subscriptions, info.rpc_version), (Some(69), Some(1)));
            assert!(info.identified && info.hello_sent && info.upgraded);
            let cases = [
                json!({"rpcVersion": 1, "eventSubscriptions": 69}),
                json!({"rpcVersion": 1, "authentication": 7, "eventSubscriptions": 69}),
                json!({"rpcVersion": 1, "authentication": "nope", "eventSubscriptions": 69}),
                json!({"rpcVersion": 99, "authentication": "nope", "eventSubscriptions": 69}),
            ];
            for d in cases {
                let mut ws = connect(&fake).await;
                let _hello = recv(&mut ws).await;
                send_json(&mut ws, &json!({"op": 1, "d": d})).await;
                assert_eq!(closed(&mut ws).await, 4009, "{d}");
            }
            // CRY:60-63: secret first, then the challenge.
            let mut ws = connect(&fake).await;
            let hello = recv(&mut ws).await;
            let a = &hello["d"]["authentication"];
            let hashed = |bytes: Vec<u8>| STANDARD.encode(Sha256::digest(&bytes));
            let salt = a["salt"].as_str().unwrap().as_bytes();
            let pw_secret = hashed([b"correct horse".as_slice(), salt].concat());
            let challenge = a["challenge"].as_str().unwrap().as_bytes();
            let reversed = hashed([challenge, pw_secret.as_bytes()].concat());
            send_json(&mut ws, &identify(Some(&reversed), MASK69)).await;
            assert_eq!(closed(&mut ws).await, 4009);
            assert_eq!(fake.identifies().len(), 6);
            assert!(fake.wait_ended(5, T).await);
            fake.assert_clean();
        });
    }

    /// WSP:32-35, :127-144: Identify asks for an RPC version; one outside
    /// the server's set closes 4010 (not a violation: it's how a client
    /// learns it's too old); a missing or non-unsigned one is 4003/4004.
    #[test]
    fn rpc_versions_are_negotiated_as_wsp_does() {
        guarded(async {
            let only2 = FakeObs::start(Config::default().hello_rpc(2).accept_rpc(&[2])).await;
            let mut ws = connect(&only2).await;
            assert_eq!(recv(&mut ws).await["d"]["rpcVersion"], 2);
            send_json(&mut ws, &identify(None, MASK69)).await;
            assert_eq!(closed(&mut ws).await, 4010);
            only2.assert_clean();

            let serves1 = FakeObs::start(Config::default().hello_rpc(2)).await;
            let _ok = identified(&serves1, None, MASK69).await;
            for (d, want) in [
                (json!({"eventSubscriptions": 69}), 4003),
                (json!({"rpcVersion": "1", "eventSubscriptions": 69}), 4004),
                (json!({"rpcVersion": -1, "eventSubscriptions": 69}), 4004),
                (json!({"rpcVersion": 1, "eventSubscriptions": "69"}), 4004),
            ] {
                let mut ws = connect(&serves1).await;
                let _hello = recv(&mut ws).await;
                send_json(&mut ws, &json!({"op": 1, "d": d})).await;
                assert_eq!(closed(&mut ws).await, want, "{d}");
            }
            assert_eq!(serves1.violations().len(), 4);
        });
    }

    /// WSP:83-97: nothing but Identify before Identified (4007), and no
    /// second Identify (4008); both are violations.
    #[test]
    fn only_one_identify_and_nothing_before_it() {
        guarded(async {
            let fake = FakeObs::start(Config::default()).await;
            let mut ws = connect(&fake).await;
            let _hello = recv(&mut ws).await;
            send_json(&mut ws, &request_json("GetVersion", None)).await;
            assert_eq!(closed(&mut ws).await, 4007);
            assert!(fake.requests().is_empty());

            let mut ws = identified(&fake, None, MASK69).await;
            send_json(&mut ws, &identify(None, MASK69)).await;
            assert_eq!(closed(&mut ws).await, 4008);
            let v = fake.violations();
            assert_eq!(v.len(), 2, "{v:?}");
            assert!(v[0].contains("4007") && v[1].contains("4008"), "{v:?}");
        });
    }

    /// WSP:57-67, P576:188-197: what an Identify may carry. An unknown
    /// field, `authentication` to a server that asked for none, or a mask
    /// other than [`Config::expect_mask`] is let through, as OBS does, and
    /// recorded.
    #[test]
    fn identify_extras_are_recorded() {
        guarded(async {
            let fake = FakeObs::start(Config::default()).await;
            let mut ws = connect(&fake).await;
            let _hello = recv(&mut ws).await;
            let d =
                json!({"rpcVersion": 1, "authentication": "x", "eventSubscriptions": 69, "x": 1});
            send_json(&mut ws, &json!({"op": 1, "d": d})).await;
            assert_eq!(recv(&mut ws).await["op"], 2);
            let _all = identified(&fake, None, None).await;
            assert_eq!(fake.session(2).unwrap().subscriptions, Some(ALL));
            let v = fake.violations();
            assert_eq!(v.len(), 3, "{v:?}");
            assert!(v[0].contains("`x`") && v[1].contains("authentication"));
            assert!(v[2].contains("subscribed to 4095, not 69"), "{v:?}");
        });
    }

    /// `WSS:365-436` and WSP:72-81, :190-208, :348-351: what isn't a JSON
    /// object with a numeric `op` and an object `d` closes, each with OBS's
    /// code, and each is a violation; so are an op it has no use for, a
    /// batch and a request without its id.
    #[test]
    fn malformed_messages_close_as_wss_does() {
        guarded(async {
            let fake = FakeObs::start(Config::default()).await;
            let cases: [(Message, u16, bool); 11] = [
                (Message::binary(&b"{}"[..]), 4002, false),
                (Message::text("not json"), 4002, false),
                (Message::text("[1]"), 4002, false),
                (
                    Message::text(r#"{"request-type":"GetVersion","message-id":"1"}"#),
                    4010,
                    false,
                ),
                (Message::text(r#"{"d":{}}"#), 4006, false),
                (Message::text(r#"{"op":"1","d":{}}"#), 4006, false),
                (Message::text(r#"{"op":1}"#), 4003, false),
                (Message::text(r#"{"op":1,"d":[]}"#), 4004, false),
                (Message::text(r#"{"op":4,"d":{}}"#), 4006, true),
                (
                    Message::text(r#"{"op":8,"d":{"requestId":"b","requests":[]}}"#),
                    4006,
                    true,
                ),
                (
                    Message::text(r#"{"op":6,"d":{"requestType":"GetVersion"}}"#),
                    4003,
                    true,
                ),
            ];
            for (msg, want, after_identify) in cases {
                let shown = format!("{msg:?}");
                let mut ws = if after_identify {
                    identified(&fake, None, MASK69).await
                } else {
                    let mut ws = connect(&fake).await;
                    let _hello = recv(&mut ws).await;
                    ws
                };
                send(&mut ws, msg).await;
                assert_eq!(closed(&mut ws).await, want, "{shown}");
            }
            assert_eq!(fake.violations().len(), 11);
        });
    }

    /// P576:2830-2848: `availableRequests` is the fake's own list; a type
    /// outside it (here `GetSceneItemId`, which the link must never send)
    /// or made unavailable is answered 204 `UnknownRequestType`
    /// (P576:857-863) and recorded.
    #[test]
    fn an_unknown_request_is_204_and_recorded() {
        guarded(async {
            let fake = FakeObs::start(Config::default()).await;
            let mut ws = identified(&fake, None, MASK69).await;
            let v = request(&mut ws, "GetVersion", None).await;
            assert_eq!(v["requestStatus"], json!({"result": true, "code": 100}));
            assert_eq!(v["responseData"]["availableRequests"], json!(REQUESTS));
            assert_eq!(v["responseData"]["obsVersion"], "32.2.2");
            assert_eq!(v["responseData"]["obsWebSocketVersion"], "5.7.6");
            assert_eq!(v["responseData"]["rpcVersion"], 1);
            fake.assert_clean();
            let v = request(
                &mut ws,
                "GetSceneItemId",
                Some(json!({"sceneName": "Game"})),
            )
            .await;
            assert_eq!(v["requestStatus"]["result"], false);
            assert_eq!(code(&v), 204);
            assert!(fake.violations()[0].contains("GetSceneItemId"));

            let fake = FakeObs::start(Config::default().unavailable("SetInputSettings")).await;
            let mut ws = identified(&fake, None, MASK69).await;
            let v = request(&mut ws, "GetVersion", None).await;
            let listed = v["responseData"]["availableRequests"]
                .as_array()
                .unwrap()
                .clone();
            assert_eq!(listed.len(), 8);
            assert!(!listed.contains(&json!("SetInputSettings")));
            let set = json!({"inputUuid": TEXT_V3, "inputSettings": {"text": "1:00"}});
            assert_eq!(
                code(&request(&mut ws, "SetInputSettings", Some(set)).await),
                204
            );
            assert_eq!(fake.violations().len(), 1);
        });
    }

    /// RHS:40-167, ARR:87-111: the scene list bottom first with
    /// `sceneIndex` counting up from the bottom; the program scene with its
    /// deprecated twin fields; a switch by uuid whose
    /// `CurrentProgramSceneChanged` comes before its answer by default,
    /// after it, or not at all; and P576's codes for a bad target.
    #[test]
    fn scenes_answer_as_rhs_does() {
        guarded(async {
            let fake = FakeObs::start(Config::default()).await;
            let mut ws = identified(&fake, None, MASK69).await;
            let list = request(&mut ws, "GetSceneList", None).await["responseData"].clone();
            assert_eq!(list["currentProgramSceneUuid"], GAME);
            assert_eq!(list["currentProgramSceneName"], "Game");
            assert_eq!(list["currentPreviewSceneUuid"], Value::Null);
            let scenes: Vec<(String, u64)> = list["scenes"]
                .as_array()
                .unwrap()
                .iter()
                .map(|s| {
                    (
                        s["sceneName"].as_str().unwrap().to_owned(),
                        s["sceneIndex"].as_u64().unwrap(),
                    )
                })
                .collect();
            let want =
                [("Just Chatting", 0), ("BRB", 1), ("Game", 2)].map(|(n, i)| (n.to_owned(), i));
            assert_eq!(scenes, want);
            let cur =
                request(&mut ws, "GetCurrentProgramScene", None).await["responseData"].clone();
            assert_eq!(
                cur,
                json!({"sceneName": "Game", "sceneUuid": GAME,
                       "currentProgramSceneName": "Game", "currentProgramSceneUuid": GAME})
            );

            for (order, to) in [
                (ProgramEvent::BeforeAnswer, BRB),
                (ProgramEvent::AfterAnswer, JUST_CHATTING),
                (ProgramEvent::Never, GAME),
            ] {
                fake.config(|c| c.program_event = order);
                let set = request_json("SetCurrentProgramScene", Some(json!({"sceneUuid": to})));
                send_json(&mut ws, &set).await;
                let first = recv(&mut ws).await;
                match order {
                    ProgramEvent::BeforeAnswer => {
                        assert_eq!(event_type(&first), "CurrentProgramSceneChanged");
                        assert_eq!(
                            first["d"]["eventData"],
                            json!({"sceneName": "BRB", "sceneUuid": BRB})
                        );
                        assert_eq!(first["d"]["eventIntent"], SCENES);
                        assert_eq!(recv(&mut ws).await["op"], 7);
                    }
                    ProgramEvent::AfterAnswer => {
                        assert_eq!(first["op"], 7);
                        assert_eq!(
                            event_type(&recv(&mut ws).await),
                            "CurrentProgramSceneChanged"
                        );
                    }
                    ProgramEvent::Never => assert_eq!(first["op"], 7),
                }
                assert_eq!(fake.program(), to);
            }
            assert!(quiet(&mut ws, ms(100)).await);
            fake.assert_clean();

            for (data, want) in [
                (Some(json!({"sceneUuid": "no-such-scene"})), 600),
                (Some(json!({"sceneUuid": TEXT_V3})), 602),
                (Some(json!({"sceneName": "BRB"})), 100),
                (Some(json!({"sceneName": "BRB", "sceneUuid": BRB})), 404),
                (Some(json!({"sceneUuid": 7})), 401),
                (None, 301),
            ] {
                let v = request(&mut ws, "SetCurrentProgramScene", data.clone()).await;
                assert_eq!(code(&v), want, "{data:?}");
            }
            assert_eq!(fake.violations().len(), 3);
        });
    }

    /// RHI:36-52, :314-328 and ARR:203-235: every input with its versioned
    /// and unversioned kind (the latter left out on request), a filter on
    /// the versioned id only, and a source's whole settings.
    #[test]
    fn inputs_are_listed_as_rhi_and_arr_do() {
        guarded(async {
            let fake = FakeObs::start(Config::default()).await;
            let mut ws = identified(&fake, None, MASK69).await;
            let list =
                request(&mut ws, "GetInputList", None).await["responseData"]["inputs"].clone();
            let kinds: Vec<(&str, &str, &str)> = list
                .as_array()
                .unwrap()
                .iter()
                .map(|i| {
                    (
                        i["inputUuid"].as_str().unwrap(),
                        i["inputKind"].as_str().unwrap(),
                        i["unversionedInputKind"].as_str().unwrap(),
                    )
                })
                .collect();
            assert_eq!(
                kinds,
                [
                    (TEXT_V0, "text_gdiplus", "text_gdiplus"),
                    (TEXT_V3, "text_gdiplus_v3", "text_gdiplus"),
                    (TEXT_FROM_FILE, "text_gdiplus_v3", "text_gdiplus"),
                    (TEXT_FT2, "text_ft2_source_v2", "text_ft2_source"),
                    (BROWSER, "browser_source", "browser_source"),
                ]
            );
            assert_eq!(
                keys(&list[0]),
                [
                    "inputKind",
                    "inputKindCaps",
                    "inputName",
                    "inputUuid",
                    "unversionedInputKind"
                ]
            );
            for (kind, n) in [("text_gdiplus_v3", 2), ("text_gdiplus", 1), ("", 5)] {
                let v = request(&mut ws, "GetInputList", Some(json!({"inputKind": kind}))).await;
                assert_eq!(
                    v["responseData"]["inputs"].as_array().unwrap().len(),
                    n,
                    "{kind}"
                );
            }
            let got = request(
                &mut ws,
                "GetInputSettings",
                Some(json!({"inputUuid": TEXT_V3})),
            )
            .await;
            assert_eq!(
                got["responseData"],
                json!({"inputKind": "text_gdiplus_v3", "inputSettings": {"text": SOURCE_TEXT, "file": SOURCE_FILE}})
            );
            let older = FakeObs::start(
                Config::default().model(
                    Model::standard()
                        .with_text_sources(3)
                        .without_unversioned_kinds(),
                ),
            )
            .await;
            let mut ws = identified(&older, None, MASK69).await;
            let list =
                request(&mut ws, "GetInputList", None).await["responseData"]["inputs"].clone();
            assert_eq!(list.as_array().unwrap().len(), 8);
            assert!(list
                .as_array()
                .unwrap()
                .iter()
                .all(|i| i.get("unversionedInputKind").is_none()));
            assert_eq!(list[7]["inputUuid"], numbered_uuid(3));
            older.assert_clean();
        });
    }

    /// RHI:345-379: `SetInputSettings` names its input, then
    /// `inputSettings` (an object) is laid over the settings or, with
    /// `overlay` false, replaces them; its `InputSettingsChanged` (Inputs,
    /// P576:1948-1964) reaches only a connection subscribed to Inputs.
    #[test]
    fn set_input_settings_lays_over_or_replaces() {
        guarded(async {
            let fake = FakeObs::start(Config::default()).await;
            let mut ws = identified(&fake, None, MASK69).await;
            let mut all = identified(&fake, None, None).await;
            let set =
                json!({"inputUuid": TEXT_V3, "inputSettings": {"text": "4:59"}, "overlay": true});
            assert_eq!(
                request(&mut ws, "SetInputSettings", Some(set)).await["requestStatus"]["result"],
                true
            );
            let now = fake.model(|m| m.input(TEXT_V3).unwrap().settings.clone());
            assert_eq!(
                Value::Object(now),
                json!({"text": "4:59", "file": SOURCE_FILE})
            );
            let changed = recv(&mut all).await;
            assert_eq!(event_type(&changed), "InputSettingsChanged");
            assert_eq!(changed["d"]["eventIntent"], INPUTS);
            assert_eq!(changed["d"]["eventData"]["inputSettings"]["text"], "4:59");
            let set =
                json!({"inputName": "Timer", "inputSettings": {"text": "4:58"}, "overlay": false});
            assert_eq!(
                code(&request(&mut ws, "SetInputSettings", Some(set)).await),
                100
            );
            let now = fake.model(|m| m.input(TEXT_V3).unwrap().settings.clone());
            assert_eq!(Value::Object(now), json!({"text": "4:58"}));
            assert!(quiet(&mut ws, ms(100)).await, "mask 69 has no Inputs");

            for (data, want) in [
                (json!({"inputUuid": TEXT_V3}), 300),
                (json!({"inputUuid": TEXT_V3, "inputSettings": "text"}), 401),
                (
                    json!({"inputUuid": TEXT_V3, "inputSettings": {}, "overlay": 1}),
                    401,
                ),
                (json!({"inputUuid": GAME, "inputSettings": {}}), 602),
                (
                    json!({"inputUuid": numbered_uuid(1), "inputSettings": {}}),
                    600,
                ),
            ] {
                assert_eq!(
                    code(&request(&mut ws, "SetInputSettings", Some(data.clone())).await),
                    want,
                    "{data}"
                );
            }
        });
    }

    /// P576:1417-1505's nine states; EHO:22-88's events (`outputActive`
    /// from the state, a recording's `outputPath` on STARTED and STOPPED
    /// only); P576:5366-5457's statuses, with no duration while the output
    /// isn't active (NUM:28-29).
    #[test]
    fn outputs_answer_as_eho_and_num_do() {
        let states = [
            (OutputState::Unknown, "UNKNOWN"),
            (OutputState::Starting, "STARTING"),
            (OutputState::Started, "STARTED"),
            (OutputState::Stopping, "STOPPING"),
            (OutputState::Stopped, "STOPPED"),
            (OutputState::Reconnecting, "RECONNECTING"),
            (OutputState::Reconnected, "RECONNECTED"),
            (OutputState::Paused, "PAUSED"),
            (OutputState::Resumed, "RESUMED"),
        ];
        for (s, word) in states {
            assert_eq!(s.as_str(), format!("OBS_WEBSOCKET_OUTPUT_{word}"));
        }
        guarded(async {
            let fake = FakeObs::start(Config::default()).await;
            let mut ws = identified(&fake, None, MASK69).await;
            fake.model(|m| m.stream.duration_ms = 9_000);
            let st = request(&mut ws, "GetStreamStatus", None).await["responseData"].clone();
            assert_eq!(
                (st["outputActive"].clone(), st["outputDuration"].clone()),
                (json!(false), json!(0))
            );

            fake.record_state(OutputState::Started);
            let ev = recv(&mut ws).await;
            assert_eq!(event_type(&ev), "RecordStateChanged");
            assert_eq!(ev["d"]["eventIntent"], OUTPUTS);
            assert_eq!(
                ev["d"]["eventData"],
                json!({"outputActive": true, "outputState": "OBS_WEBSOCKET_OUTPUT_STARTED", "outputPath": RECORD_PATH})
            );
            fake.model(|m| m.record.duration_ms = 125_000);
            let rec = request(&mut ws, "GetRecordStatus", None).await["responseData"].clone();
            assert_eq!(
                keys(&rec),
                [
                    "outputActive",
                    "outputBytes",
                    "outputDuration",
                    "outputPaused",
                    "outputTimecode"
                ]
            );
            assert_eq!(rec["outputDuration"], 125_000);
            assert_eq!(rec["outputTimecode"], "00:02:05.000");
            fake.record_state(OutputState::Paused);
            let ev = recv(&mut ws).await;
            assert_eq!(ev["d"]["eventData"]["outputActive"], false);
            assert_eq!(ev["d"]["eventData"]["outputPath"], Value::Null);
            assert_eq!(
                request(&mut ws, "GetRecordStatus", None).await["responseData"]["outputPaused"],
                true
            );

            fake.stream_state(OutputState::Started);
            fake.stream_state(OutputState::Reconnecting);
            for want in [
                "OBS_WEBSOCKET_OUTPUT_STARTED",
                "OBS_WEBSOCKET_OUTPUT_RECONNECTING",
            ] {
                let ev = recv(&mut ws).await;
                assert_eq!(event_type(&ev), "StreamStateChanged");
                assert_eq!(ev["d"]["eventData"]["outputState"], want);
                assert!(ev["d"]["eventData"].get("outputPath").is_none());
            }
            let st = request(&mut ws, "GetStreamStatus", None).await["responseData"].clone();
            assert_eq!(keys(&st).len(), 8);
            assert_eq!(
                (st["outputActive"].clone(), st["outputReconnecting"].clone()),
                (json!(true), json!(true))
            );
            fake.stream_state(OutputState::Stopped);
            assert_eq!(fake.model(|m| m.stream), Output::default());
            fake.assert_clean();
        });
    }

    /// WSP:355-411: an event reaches identified connections whose mask has
    /// its intent, and none while OBS isn't ready; `ExitStarted` (General)
    /// goes out, then every request is 207 (EH:460-469, WSP:212-220).
    #[test]
    fn subscriptions_gate_events() {
        guarded(async {
            let fake = FakeObs::start(Config::default()).await;
            let mut m69 = identified(&fake, None, MASK69).await;
            let mut none = identified(&fake, None, Some(0)).await;
            let mut all = identified(&fake, None, None).await;
            let mut early = connect(&fake).await;
            let _hello = recv(&mut early).await;
            assert_eq!(
                fake.event(&Event::StreamStateChanged(OutputState::Started)),
                2
            );
            for ws in [&mut m69, &mut all] {
                assert_eq!(event_type(&recv(ws).await), "StreamStateChanged");
            }
            let input = Event::Raw {
                event_type: "InputMuteStateChanged".into(),
                intent: INPUTS,
                data: Some(json!({"inputMuted": true})),
            };
            assert_eq!(fake.event(&input), 1);
            assert_eq!(
                recv(&mut all).await["d"]["eventData"],
                json!({"inputMuted": true})
            );
            let none_data = Event::Raw {
                event_type: "X".into(),
                intent: GENERAL,
                data: Some(json!(1)),
            };
            assert_eq!(fake.event(&none_data), 2);
            assert!(recv(&mut m69).await["d"].get("eventData").is_none());
            let _ = recv(&mut all).await;

            fake.exit_started();
            assert_eq!(fake.event(&Event::ExitStarted), 0, "not ready: no events");
            for ws in [&mut m69, &mut all] {
                let ev = recv(ws).await;
                assert_eq!(
                    ev,
                    json!({"op": 5, "d": {"eventType": "ExitStarted", "eventIntent": 1}})
                );
            }
            let v = request(&mut m69, "GetVersion", None).await;
            assert_eq!(
                v["requestStatus"],
                json!({"result": false, "code": 207, "comment": NOT_READY})
            );
            fake.set_ready(true);
            assert_eq!(code(&request(&mut m69, "GetVersion", None).await), 100);
            for ws in [&mut none, &mut early] {
                assert!(quiet(ws, ms(100)).await);
            }
            assert_eq!(fake.violations().len(), 2, "masks 0 and All aren't 69");
        });
    }

    /// D9: a request sent while another is unanswered is the link's
    /// fault, however the first was delayed; answers go out when due, so the
    /// second can overtake the first.
    #[test]
    fn two_requests_outstanding_are_a_violation() {
        guarded(async {
            let fake = FakeObs::start(Config::default()).await;
            fake.script(Rule::on("GetVersion").times(1).delay(ms(300)));
            let mut ws = identified(&fake, None, MASK69).await;
            let t0 = Instant::now();
            send_json(&mut ws, &request_json("GetVersion", None)).await;
            assert_eq!(code(&request(&mut ws, "GetStreamStatus", None).await), 100);
            let late = recv(&mut ws).await;
            assert_eq!(late["d"]["requestType"], "GetVersion");
            assert!(t0.elapsed() >= ms(300));
            assert_eq!(fake.max_outstanding(), 2);
            let v = fake.violations();
            assert_eq!(v.len(), 1);
            assert!(
                v[0].contains("GetStreamStatus sent while 1 request(s) were unanswered"),
                "{v:?}"
            );
            let seen = fake.requests();
            let (first, second) = (seen[0].answer.unwrap(), seen[1].answer.unwrap());
            assert!(second.0 < first.0 && first.0 - seen[0].at >= ms(300));
            assert_eq!((first.1, second.1), (100, 100));
            assert_eq!(fake.session(1).unwrap().outstanding, 0);
        });
    }

    /// The scripts: rules counted down in order (207 twice, then the
    /// answer; one 702); a dropped answer stays outstanding; a Set applied
    /// with no answer still switches and announces; a close instead of an
    /// answer; [`FakeObs::clear_scripts`].
    #[test]
    fn scripts_play_in_order() {
        guarded(async {
            let fake = FakeObs::start(Config::default()).await;
            fake.script(Rule::any().times(2).not_ready());
            fake.script(Rule::on("SetInputSettings").times(1).code(702));
            let mut ws = identified(&fake, None, MASK69).await;
            let codes = [
                code(&request(&mut ws, "GetVersion", None).await),
                code(&request(&mut ws, "GetStreamStatus", None).await),
                code(&request(&mut ws, "GetVersion", None).await),
            ];
            assert_eq!(codes, [207, 207, 100]);
            let set = json!({"inputUuid": TEXT_V0, "inputSettings": {"text": "x"}});
            assert_eq!(
                code(&request(&mut ws, "SetInputSettings", Some(set.clone())).await),
                702
            );
            assert_eq!(
                fake.model(|m| m.input(TEXT_V0).unwrap().settings["text"].clone()),
                "0:00"
            );
            assert_eq!(
                code(&request(&mut ws, "SetInputSettings", Some(set)).await),
                100
            );

            fake.script(Rule::on("GetRecordStatus").times(1).drop_answer());
            send_json(&mut ws, &request_json("GetRecordStatus", None)).await;
            assert!(quiet(&mut ws, ms(300)).await);
            assert_eq!(fake.session(1).unwrap().outstanding, 1);
            assert!(fake.requests_of("GetRecordStatus")[0].answer.is_none());

            fake.script(
                Rule::on("SetCurrentProgramScene")
                    .times(1)
                    .apply_without_answer(),
            );
            let mut ws = identified(&fake, None, MASK69).await;
            send_json(
                &mut ws,
                &request_json("SetCurrentProgramScene", Some(json!({"sceneUuid": BRB}))),
            )
            .await;
            assert_eq!(
                event_type(&recv(&mut ws).await),
                "CurrentProgramSceneChanged"
            );
            assert!(quiet(&mut ws, ms(200)).await);
            assert_eq!(fake.program(), BRB);

            fake.script(Rule::on("GetSceneList").close(4011));
            let mut ws = identified(&fake, None, MASK69).await;
            send_json(&mut ws, &request_json("GetSceneList", None)).await;
            assert_eq!(closed(&mut ws).await, 4011);
            fake.clear_scripts();
            let mut ws = identified(&fake, None, MASK69).await;
            assert_eq!(code(&request(&mut ws, "GetSceneList", None).await), 100);
            fake.assert_clean();
        });
    }

    /// The fake's closes: any code to every connection or one (a Kick is
    /// 4011, `WSS:166-182`), each waiting for the echo; a client's close is
    /// echoed, or with the echo off neither echoed nor answered with a
    /// close, the fake reading on until the client's FIN.
    #[test]
    fn closes_both_ways() {
        guarded(async {
            let fake = FakeObs::start(Config::default()).await;
            let (mut a, mut b) = (
                identified(&fake, None, MASK69).await,
                identified(&fake, None, MASK69).await,
            );
            fake.close_session(2, 1001, "Server stopping.");
            assert_eq!(closed(&mut b).await, 1001);
            fake.close_all(4011, "Your session has been invalidated.");
            assert_eq!(closed(&mut a).await, 4011);
            assert!(fake.wait_ended(2, T).await);
            assert_eq!(fake.session(1).unwrap().end, Some(End::ServerClosed(4011)));
            assert_eq!(fake.session(2).unwrap().end, Some(End::ServerClosed(1001)));

            let mut c = identified(&fake, None, MASK69).await;
            send(&mut c, Message::close(Some(CloseCode::GOING_AWAY), "")).await;
            let echo = next(&mut c).await;
            assert_eq!(echo.as_close().map(|(code, _)| u16::from(code)), Some(1001));
            assert!(fake.wait_ended(3, T).await);
            assert_eq!(
                fake.session(3).unwrap().end,
                Some(End::ClientClosed(Some(1001)))
            );

            fake.set_close_echo(false);
            let mut d = identified(&fake, None, MASK69).await;
            send(&mut d, Message::close(Some(CloseCode::GOING_AWAY), "")).await;
            assert!(quiet(&mut d, ms(300)).await, "no echo");
            assert_eq!(fake.session(4).unwrap().end, None);
            drop(d);
            assert!(fake.wait_ended(4, T).await);
            let tail: Vec<Frame> = fake
                .log()
                .into_iter()
                .filter(|r| r.session == 4)
                .map(|r| r.frame)
                .rev()
                .take(2)
                .collect();
            assert_eq!(tail, [Frame::Eof, Frame::Close(Some(1001), String::new())]);
            fake.assert_clean();
        });
    }

    /// Pings are answered (by tokio-websockets) until the fake is told to
    /// ignore them; then the pong is cut and counted.
    #[test]
    fn pings_are_answered_unless_ignored() {
        guarded(async {
            let fake = FakeObs::start(Config::default()).await;
            let mut ws = identified(&fake, None, MASK69).await;
            send(&mut ws, Message::ping(&b"p1"[..])).await;
            let pong = raw_next(&mut ws).await.unwrap().unwrap();
            assert!(pong.is_pong());
            assert_eq!(&pong.as_payload()[..], b"p1");
            fake.set_ignore_pings(true);
            send(&mut ws, Message::ping(&b"p2"[..])).await;
            let none =
                tokio::time::timeout(ms(300), poll_fn(|cx| Pin::new(&mut ws).poll_next(cx))).await;
            assert!(none.is_err(), "{none:?}");
            assert_eq!(fake.pongs(), (1, 1));
            // Framing still lines up after a cut pong.
            assert_eq!(code(&request(&mut ws, "GetVersion", None).await), 100);
            let pings: Vec<Frame> = fake
                .log()
                .into_iter()
                .filter(|r| matches!(r.frame, Frame::Ping(_)))
                .map(|r| r.frame)
                .collect();
            assert_eq!(
                pings,
                [Frame::Ping(b"p1".to_vec()), Frame::Ping(b"p2".to_vec())]
            );
        });
    }

    /// Hello late (what comes before it is handled after it, as OBS sends
    /// Hello before it handles anything), never, or a close instead;
    /// Identified late (a request meanwhile is 4007) or never.
    #[test]
    fn hello_and_identified_can_wait() {
        guarded(async {
            let fake = FakeObs::start(Config::default()).await;
            fake.set_hello(Stage::After(ms(300)));
            let t0 = Instant::now();
            let mut ws = connect(&fake).await;
            send_json(&mut ws, &identify(None, MASK69)).await;
            assert_eq!(recv(&mut ws).await["op"], 0);
            assert!(t0.elapsed() >= ms(300));
            assert_eq!(recv(&mut ws).await["op"], 2);

            fake.set_hello(Stage::Never);
            let mut ws = connect(&fake).await;
            assert!(quiet(&mut ws, ms(300)).await);
            assert!(!fake.session(2).unwrap().hello_sent);
            fake.set_hello(Stage::Close(4011));
            let mut ws = connect(&fake).await;
            assert_eq!(closed(&mut ws).await, 4011);

            fake.set_hello(Stage::Now);
            fake.set_identified(Stage::Never);
            let mut ws = connect(&fake).await;
            let _hello = recv(&mut ws).await;
            send_json(&mut ws, &identify(None, MASK69)).await;
            assert!(quiet(&mut ws, ms(300)).await);
            assert_eq!(fake.session(4).unwrap().subscriptions, Some(69));
            assert!(!fake.session(4).unwrap().identified);

            fake.set_identified(Stage::After(ms(300)));
            let mut ws = connect(&fake).await;
            let _hello = recv(&mut ws).await;
            let t0 = Instant::now();
            send_json(&mut ws, &identify(None, MASK69)).await;
            assert_eq!(recv(&mut ws).await["op"], 2);
            assert!(t0.elapsed() >= ms(300));
            let mut ws = connect(&fake).await;
            let _hello = recv(&mut ws).await;
            send_json(&mut ws, &identify(None, MASK69)).await;
            send_json(&mut ws, &request_json("GetVersion", None)).await;
            assert_eq!(closed(&mut ws).await, 4007);
            fake.set_identified(Stage::Close(4009));
            let mut ws = connect(&fake).await;
            let _hello = recv(&mut ws).await;
            send_json(&mut ws, &identify(None, MASK69)).await;
            assert_eq!(closed(&mut ws).await, 4009);
            assert_eq!(fake.violations().len(), 1);
        });
    }

    /// What a TCP connection meets: "OBS down" (accepted, closed at once),
    /// an upgrade never answered, a header line that never ends; each
    /// counted as an accept.
    #[test]
    fn connections_can_be_refused_stalled_or_flooded() {
        guarded(async {
            let fake = FakeObs::start(Config::default()).await;
            let mut buf = vec![0u8; 64 * 1024];
            fake.set_accept(Accept::CloseAtOnce);
            let mut tcp = TcpStream::connect(fake.addr()).await.unwrap();
            let got = tokio::time::timeout(T, tcp.read(&mut buf)).await.unwrap();
            assert!(matches!(got, Ok(0) | Err(_)), "{got:?}");
            assert!(fake.wait_ended(1, T).await);
            assert_eq!(fake.session(1).unwrap().end, Some(End::Dropped));

            fake.set_accept(Accept::Stall);
            let mut tcp = TcpStream::connect(fake.addr()).await.unwrap();
            tcp.write_all(upgrade_request(&fake).as_bytes())
                .await
                .unwrap();
            assert!(tokio::time::timeout(ms(300), tcp.read(&mut buf))
                .await
                .is_err());
            drop(tcp);

            fake.set_accept(Accept::EndlessHeader);
            let mut tcp = TcpStream::connect(fake.addr()).await.unwrap();
            tcp.write_all(upgrade_request(&fake).as_bytes())
                .await
                .unwrap();
            let mut got = Vec::new();
            while got.len() < 128 * 1024 {
                let n = tcp.read(&mut buf).await.unwrap();
                assert!(n > 0);
                got.extend_from_slice(&buf[..n]);
            }
            assert!(got.starts_with(b"HTTP/1.1 101 Switching Protocols\r\n"));
            assert!(!got.windows(4).any(|w| w == b"\r\n\r\n"));
            drop(tcp);
            assert!(
                fake.wait(T, |f| f.session(3).unwrap().header_bytes >= 128 * 1024)
                    .await
            );

            fake.set_accept(Accept::Normal);
            let _ok = identified(&fake, None, MASK69).await;
            assert!(fake.wait_accepts(4, T).await);
            assert!(fake.wait_ended(3, T).await);
            fake.assert_clean();
        });
    }

    /// A 5 MiB text frame sent whole (a client capped at 4 MiB refuses it),
    /// any text, raw bytes outside framing, and a flood of
    /// `SceneNameChanged` to a Scenes subscriber.
    #[test]
    fn bulk_and_broken_frames_go_out_as_asked() {
        guarded(async {
            let fake = FakeObs::start(Config::default()).await;
            let mut ws = identified(&fake, None, MASK69).await;
            fake.send_big_frame(5 << 20);
            let big = next(&mut ws).await;
            assert_eq!(big.as_payload().len(), 5 << 20);
            assert_eq!(
                event_type(&serde_json::from_str(big.as_text().unwrap()).unwrap()),
                "SceneNameChanged"
            );
            fake.send_text("not json");
            assert_eq!(next(&mut ws).await.as_text(), Some("not json"));
            fake.flood_scene_renames(1000);
            for i in 0..1000 {
                let ev = recv(&mut ws).await;
                assert_eq!(ev["d"]["eventData"]["oldSceneName"], format!("Game {i}"));
            }
            fake.send_raw(&[0xff, 0x00]);
            assert!(raw_next(&mut ws).await.is_some_and(|r| r.is_err()));
            drop(ws);
            assert!(fake.wait_ended(1, T).await);
            let kinds: Vec<Frame> = fake
                .log()
                .into_iter()
                .filter(|r| r.dir == Dir::Out && !matches!(r.frame, Frame::Text(_)))
                .map(|r| r.frame)
                .collect();
            assert_eq!(
                kinds,
                [Frame::Big(5 << 20), Frame::Flood(1000), Frame::Raw(2)]
            );

            let mut capped = upgrade(&fake, Limits::default().max_payload_len(Some(4 << 20))).await;
            let _hello = recv(&mut capped).await;
            fake.send_big_frame(5 << 20);
            assert!(raw_next(&mut capped).await.is_some_and(|r| r.is_err()));
        });
    }

    /// The log: every frame in and out of a handshake and a request, in
    /// order, with times that never go back; the upgrade's path and
    /// headers, no subprotocol among them (P576:52-55); each request with
    /// its answer.
    #[test]
    fn the_log_keeps_every_frame_in_order() {
        guarded(async {
            let fake = FakeObs::start(Config::default()).await;
            let mut ws = identified(&fake, None, MASK69).await;
            assert_eq!(
                code(&request(&mut ws, "GetVersion", Some(json!({}))).await),
                100
            );
            assert!(fake.wait_requests("GetVersion", 1, T).await);
            assert!(fake.wait_identified(1, T).await);
            let log = fake.log();
            let Frame::Upgrade { path, headers } = &log[0].frame else {
                panic!("{:?}", log[0]);
            };
            assert_eq!(path, "/");
            assert!(headers
                .iter()
                .any(|(k, v)| k == "host" && *v == fake.addr().to_string()));
            assert!(!headers.iter().any(|(k, _)| k == "sec-websocket-protocol"));
            let ops: Vec<(Dir, Option<u64>)> = log[1..].iter().map(|r| (r.dir, r.op())).collect();
            assert_eq!(
                ops,
                [
                    (Dir::Out, Some(0)),
                    (Dir::In, Some(1)),
                    (Dir::Out, Some(2)),
                    (Dir::In, Some(6)),
                    (Dir::Out, Some(7))
                ]
            );
            assert!(log.windows(2).all(|w| w[0].at <= w[1].at));
            let info = &fake.sessions()[0];
            assert!(info.id == 1 && info.accepted_at <= log[0].at);
            assert_eq!(log[2].json().unwrap()["d"]["eventSubscriptions"], 69);
            let seen = &fake.requests()[0];
            assert_eq!(
                (seen.session, seen.request_id.clone(), seen.data.clone()),
                (1, json!("GetVersion"), Some(json!({})))
            );
            assert_eq!(seen.answer.map(|a| a.1), Some(100));
            assert_eq!(
                fake.identifies(),
                [(1, json!({"rpcVersion": 1, "eventSubscriptions": 69}))]
            );
            fake.assert_clean();
        });
    }

    /// The streamer's own changes in OBS: the model, then the event OBS
    /// sends (EH:313-314, P576:1785-1852).
    #[test]
    fn scene_changes_in_obs_update_the_model_and_announce() {
        guarded(async {
            let fake = FakeObs::start(Config::default()).await;
            let mut ws = identified(&fake, None, MASK69).await;
            fake.create_scene("Intermission", "c0ffee00-0000-4000-8000-000000000001");
            assert_eq!(
                recv(&mut ws).await["d"]["eventData"],
                json!({"sceneName": "Intermission", "sceneUuid": "c0ffee00-0000-4000-8000-000000000001", "isGroup": false})
            );
            assert!(fake.rename_scene(BRB, "Be right back"));
            assert_eq!(
                recv(&mut ws).await["d"]["eventData"],
                json!({"sceneUuid": BRB, "oldSceneName": "BRB", "sceneName": "Be right back"})
            );
            assert!(fake.switch_program(BRB));
            let ev = recv(&mut ws).await;
            assert_eq!(event_type(&ev), "CurrentProgramSceneChanged");
            assert_eq!(fake.program(), BRB);
            assert!(fake.remove_scene(JUST_CHATTING));
            assert_eq!(event_type(&recv(&mut ws).await), "SceneRemoved");
            assert_eq!(fake.model(|m| m.scenes.len()), 3);
            assert!(fake.model(|m| m.scene(JUST_CHATTING).is_none()));
            assert!(
                !fake.switch_program(JUST_CHATTING)
                    && !fake.remove_scene("x")
                    && !fake.rename_scene("x", "y")
            );
            assert!(quiet(&mut ws, ms(100)).await);
            let program = request(&mut ws, "GetCurrentProgramScene", None).await;
            assert_eq!(program["responseData"]["sceneName"], "Be right back");
            fake.assert_clean();
        });
    }
}
