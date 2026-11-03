//! v1.60.0: timers that change your voice — what a countdown's start and
//! end run, as pure rules.
//!
//! A countdown carries two lists of actions, `onStart` and `onFire`
//! ([`Hooks`]), read from its `sync_timers` entry by [`hooks_from_wire`]
//! (§7.2). The core records what happens to a timer as a [`Transition`],
//! every one but a sleep pause carrying the timer's lists. The timer thread
//! turns each drained one into a hook run with [`plan`] (§7.1) and routes it
//! with [`route`]: the backend half ([`ActionBatch`]) goes to the action
//! worker, the page half
//! ([`TimerActionsEvent`]) to the main window as `timer-actions`. Nothing
//! here reads a clock, waits, logs or calls Tauri, so every rule is tested
//! on any machine.
//!
//! The wire contract — the event and command names, the action kinds and
//! their fields (v1.61.0: and the `obs` kind's two ops, `actionOps`), the
//! limits, the reading table, the list samples and one event per `why` — is
//! in `src/overlay/timer.fixture.json` beside Hourglass's, which the
//! frontend's tests read too.

use std::time::{Duration, Instant};

use serde::Serialize;
use serde_json::{json, Map, Value};

use crate::obs::{MAX_SCENE_NAME_CHARS, MAX_UUID_CHARS};
use crate::timers::{CancelWhy, TimerPhase, Transition};

/// D34's kill switch: `false` makes [`plan`] return nothing, so no batch
/// reaches the worker and no event the page: timers behave as v1.59.0's,
/// plus End now.
pub const TIMER_ACTIONS: bool = true;

/// The event a hook run's page half goes out as, to every window. Payload:
/// [`TimerActionsEvent`]. Only the main window's store acts on it.
pub const TIMER_ACTIONS_EVENT: &str = "timer-actions";

/// The event the action worker sends after a timer's Stop all clips and
/// lines (D23), so the page clears the clips it shows playing. Payload:
/// `null`.
pub const SOUNDBOARD_STOPPED_EVENT: &str = "soundboard-stopped";

/// D2: the entries of one list that are read — the first this many stored,
/// readable or not. The page counts every stored entry against it too.
pub const MAX_ACTIONS: usize = 8;

/// D2: a line's longest text, in characters (Unicode scalar values) after
/// trimming. A long line takes Kokoro seconds to render.
pub const MAX_LINE_CHARS: usize = 200;

/// D2: a `tileId`'s or a `voiceId`'s longest, in characters. A `presetId`
/// has no cap: preset ids have no length bound, so a cap would refuse a real
/// preset.
pub const MAX_TARGET_CHARS: usize = 128;

/// D9: a fire found this late or less runs all of `onFire`; a later one runs
/// only its restoring actions, since a line, a clip or a replay that late
/// reads as a glitch.
pub const LATE_ONE_SHOTS_MS: u64 = 30_000;

/// One entry of a countdown's `onStart` or `onFire` list (D1), as read by
/// [`action_from_wire`]: its validated fields only.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    /// Switch to this preset (the page's).
    Preset { preset_id: String },
    /// Switch to a preset that changes your voice, other than the live one
    /// (the page's, D20).
    RandomPreset,
    /// Back to the preset the start switched from, if the live one is still
    /// the one it chose (the page's, D17). Never runs at a start.
    RevertPreset,
    /// Play this soundboard tile (the page's).
    Tile { tile_id: String },
    /// Say this line in a Kokoro or Critter Chatter voice (the line thread's,
    /// D21). At most one per list.
    Speak { text: String, voice_id: String },
    /// Save the replay up to its hook's moment: the start, or the moment the
    /// countdown reached zero or End now ended it (D22).
    SaveReplay,
    /// Mute or unmute your voice: a plain set, never a toggle (D24).
    SetMuted { muted: bool },
    /// Stop every clip and line (D23). Runs first in its batch (D15).
    Panic,
    /// v1.61.0: switch OBS's program scene, or back (the OBS link's, D21).
    Obs(ObsOp),
}

/// v1.61.0 (D21): what an `obs` action asks of OBS — its wire shape after
/// `kind`, flat like every other kind's fields: `{op: "scene", sceneUuid,
/// sceneName}` or `{op: "sceneBack"}`.
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
#[serde(tag = "op", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum ObsOp {
    /// Switch OBS's program scene to this one: by uuid, 1 to
    /// [`MAX_UUID_CHARS`] characters; the name (at most
    /// [`MAX_SCENE_NAME_CHARS`], empty allowed) is only shown. At start or at
    /// zero.
    Scene {
        scene_uuid: String,
        scene_name: String,
    },
    /// Back to the scene this timer's `scene` replaced, if OBS is still on
    /// the one it set (D22). Never runs at a start.
    SceneBack,
}

impl ObsOp {
    /// The op's wire name (`op`).
    #[must_use]
    pub fn op(&self) -> &'static str {
        match self {
            Self::Scene { .. } => "scene",
            Self::SceneBack => "sceneBack",
        }
    }
}

/// Which half of a hook run an action belongs to (D5).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RunsIn {
    /// The main window's store: preset switches and clips.
    Page,
    /// The action worker (and the line thread): the mute, Stop all, a
    /// replay save, a line. v1.61.0: and OBS's scene switches, through the
    /// OBS link.
    Backend,
}

impl Action {
    /// The kind's wire name (`kind`).
    #[must_use]
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Preset { .. } => "preset",
            Self::RandomPreset => "randomPreset",
            Self::RevertPreset => "revertPreset",
            Self::Tile { .. } => "tile",
            Self::Speak { .. } => "speak",
            Self::SaveReplay => "saveReplay",
            Self::SetMuted { .. } => "setMuted",
            Self::Panic => "panic",
            Self::Obs(_) => "obs",
        }
    }

    /// D5: where it runs.
    #[must_use]
    pub fn runs_in(&self) -> RunsIn {
        match self {
            Self::Preset { .. } | Self::RandomPreset | Self::RevertPreset | Self::Tile { .. } => {
                RunsIn::Page
            }
            Self::Speak { .. }
            | Self::SaveReplay
            | Self::SetMuted { .. }
            | Self::Panic
            | Self::Obs(_) => RunsIn::Backend,
        }
    }

    /// §7.1: whether a hook ever runs it. `RevertPreset` and (v1.61.0, D21)
    /// OBS's `sceneBack` never run at a start (there is nothing to go back
    /// to yet), so the card offers them at zero only; everything else runs
    /// at either.
    #[must_use]
    pub fn runs_at(&self, hook: Hook) -> bool {
        !(hook == Hook::Start && matches!(self, Self::RevertPreset | Self::Obs(ObsOp::SceneBack)))
    }

    /// Whether it puts back what a start changed: `RevertPreset`,
    /// `SetMuted {muted: false}` and (v1.61.0, D21) OBS's `sceneBack`. The
    /// rest are one-shots. A late fire and a run-out run only these (D8, D9).
    #[must_use]
    pub fn restores(&self) -> bool {
        matches!(
            self,
            Self::RevertPreset | Self::SetMuted { muted: false } | Self::Obs(ObsOp::SceneBack)
        )
    }

    /// D11: whether a restart mid-run runs it again from `onStart`: the
    /// mute, which every launch forgets.
    #[must_use]
    pub fn reapplies(&self) -> bool {
        matches!(self, Self::SetMuted { .. })
    }

    /// D10: whether it [`restores`](Self::restores) something `on_start`
    /// did, so a Reset or a delete runs it: `SetMuted {false}` only when
    /// `on_start` has `SetMuted {true}`; `RevertPreset` always, because the
    /// page acts on it only with a return the start noted (§7.3); (v1.61.0,
    /// D21) OBS's `sceneBack` only when `on_start` has a `scene`.
    #[must_use]
    pub fn undoes(&self, on_start: &[Action]) -> bool {
        match self {
            Self::SetMuted { muted: false } => on_start.contains(&Self::SetMuted { muted: true }),
            Self::RevertPreset => true,
            Self::Obs(ObsOp::SceneBack) => on_start
                .iter()
                .any(|a| matches!(a, Self::Obs(ObsOp::Scene { .. }))),
            _ => false,
        }
    }

    /// The action as the page reads it: its `kind` and validated fields,
    /// camelCase, nothing else. [`action_from_wire`] reads it back as it was.
    #[must_use]
    pub fn to_wire(&self) -> Value {
        let kind = self.kind();
        match self {
            Self::Preset { preset_id } => json!({"kind": kind, "presetId": preset_id}),
            Self::Tile { tile_id } => json!({"kind": kind, "tileId": tile_id}),
            Self::Speak { text, voice_id } => {
                json!({"kind": kind, "text": text, "voiceId": voice_id})
            }
            Self::SetMuted { muted } => json!({"kind": kind, "muted": muted}),
            Self::RandomPreset | Self::RevertPreset | Self::SaveReplay | Self::Panic => {
                json!({"kind": kind})
            }
            Self::Obs(ObsOp::Scene {
                scene_uuid,
                scene_name,
            }) => json!({
                "kind": kind,
                "op": "scene",
                "sceneUuid": scene_uuid,
                "sceneName": scene_name,
            }),
            Self::Obs(ObsOp::SceneBack) => json!({"kind": kind, "op": "sceneBack"}),
        }
    }
}

/// A countdown's two lists, as [`hooks_from_wire`] read them; a count-up's
/// are empty (D32). The core shares one per timer with every transition it
/// records ([`std::sync::Arc`]), so a deleted timer's lists still reach the
/// plan.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Hooks {
    /// At start: runs on `Started`; its mute again at a restart mid-run.
    pub on_start: Vec<Action>,
    /// At zero: runs at 00:00 and on End now; its restoring actions after a
    /// late fire or a run-out, and those that undo the start on a cancel.
    pub on_fire: Vec<Action>,
}

/// §7.2: one list entry, read on its own. `None` — skipped, and kept in the
/// page's storage — for anything but an object whose `kind` is one of the
/// nine with every field it needs: a non-empty `presetId`; a `tileId` and a
/// `voiceId` of 1 to [`MAX_TARGET_CHARS`] characters; a `text` of 1 to
/// [`MAX_LINE_CHARS`] characters once trimmed (and read trimmed); a boolean
/// `muted`; (v1.61.0, D21) an `obs` `op` of `scene`, with a `sceneUuid` of 1
/// to [`MAX_UUID_CHARS`] characters and a string `sceneName` of at most
/// [`MAX_SCENE_NAME_CHARS`], or `sceneBack`. Nothing is coerced; extra keys
/// and another kind's or op's fields are ignored. A kind this build doesn't
/// know (`monitor`, `mute`), or an `obs` op (`startStream`), is skipped.
#[must_use]
pub fn action_from_wire(v: &Value) -> Option<Action> {
    let a = v.as_object()?;
    let string = |key: &str| a.get(key).and_then(Value::as_str);
    let target = |key: &str| {
        string(key)
            .filter(|s| (1..=MAX_TARGET_CHARS).contains(&s.chars().count()))
            .map(str::to_string)
    };
    Some(match a.get("kind")?.as_str()? {
        "preset" => Action::Preset {
            preset_id: string("presetId").filter(|id| !id.is_empty())?.to_string(),
        },
        "randomPreset" => Action::RandomPreset,
        "revertPreset" => Action::RevertPreset,
        "tile" => Action::Tile {
            tile_id: target("tileId")?,
        },
        "speak" => Action::Speak {
            text: string("text")
                .map(str::trim)
                .filter(|text| (1..=MAX_LINE_CHARS).contains(&text.chars().count()))?
                .to_string(),
            voice_id: target("voiceId")?,
        },
        "saveReplay" => Action::SaveReplay,
        "setMuted" => Action::SetMuted {
            muted: a.get("muted")?.as_bool()?,
        },
        "panic" => Action::Panic,
        "obs" => Action::Obs(match string("op")? {
            "scene" => ObsOp::Scene {
                scene_uuid: string("sceneUuid")
                    .filter(|uuid| (1..=MAX_UUID_CHARS).contains(&uuid.chars().count()))?
                    .to_string(),
                scene_name: string("sceneName")
                    .filter(|name| name.chars().count() <= MAX_SCENE_NAME_CHARS)?
                    .to_string(),
            },
            "sceneBack" => ObsOp::SceneBack,
            _ => return None,
        }),
        _ => return None,
    })
}

/// D4, §7.2: a sync entry's `onStart` and `onFire`, each read on its own: a
/// list that is absent or not an array is empty; only its first
/// [`MAX_ACTIONS`] entries are read, each by [`action_from_wire`] (one that
/// can't be read is skipped, the rest kept); a `speak` after the list's first
/// readable one is skipped (D2: a second line could never be heard). Never
/// fails. `revertPreset` in `onStart` is read, and never run (§7.1). The
/// caller drops a count-up's (D32).
#[must_use]
pub fn hooks_from_wire(entry: &Map<String, Value>) -> Hooks {
    Hooks {
        on_start: list_from_wire(entry.get("onStart")),
        on_fire: list_from_wire(entry.get("onFire")),
    }
}

/// One list, as [`hooks_from_wire`] reads it.
fn list_from_wire(v: Option<&Value>) -> Vec<Action> {
    let Some(entries) = v.and_then(Value::as_array) else {
        return Vec::new();
    };
    let mut list = Vec::new();
    let mut has_line = false;
    for action in entries
        .iter()
        .take(MAX_ACTIONS)
        .filter_map(action_from_wire)
    {
        if matches!(action, Action::Speak { .. }) {
            if has_line {
                continue;
            }
            has_line = true;
        }
        list.push(action);
    }
    list
}

/// Which list a hook run runs from: `start` (`onStart`) or `end` (`onFire`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Hook {
    Start,
    End,
}

/// Why a hook run happened (§7.1), as the page's `why`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Why {
    /// `Started`: Start, Start again, or the start-or-pause key from idle,
    /// done or ran out.
    Armed,
    /// Reached 00:00 awake, at most [`LATE_ONE_SHOTS_MS`] late.
    Zero,
    /// End now: the button, `finish_timer` or the `finish` key.
    EndNow,
    /// Reached 00:00 more than [`LATE_ONE_SHOTS_MS`] late, the PC awake.
    Late,
    /// Ran out while the PC slept or Divora was closed.
    RanOut,
    /// Reset (or a kind change) while running or paused.
    Reset,
    /// Deleted while running or paused.
    Deleted,
    /// A restart mid-run re-applies the start's mute (D11). Backend only:
    /// never on the wire.
    Restart,
}

/// One transition's hook run (§7.1): which list, why, the fire's lateness,
/// and the actions to run, split into the backend half (`panic` first, then
/// list order, D15) and the page half (list order).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Planned {
    pub id: String,
    pub hook: Hook,
    pub why: Why,
    /// The fire's lateness for `zero` and `late`; 0 for every other `why`.
    pub late_ms: u64,
    pub backend: Vec<Action>,
    pub page: Vec<Action>,
}

impl Planned {
    /// Whether the page hears of it: every run but a restart's re-mute.
    #[must_use]
    pub fn emits(&self) -> bool {
        self.why != Why::Restart
    }
}

/// Which of a list's actions a hook run takes (§7.1).
#[derive(Clone, Copy)]
enum Take<'a> {
    /// Every one: a start, a fire on time, End now.
    All,
    /// onFire·R, those that put back what a start changed: a late fire, a
    /// run-out (D8, D9).
    Restoring,
    /// onFire·U, those that put back what this start did — the onStart its
    /// run started with: a Reset, a kind change, a delete (D10, Q2).
    Undoing(&'a [Action]),
    /// onStart·A, those a restart mid-run runs again: the mute (D11).
    Reapplied,
}

/// D7, §7.1: what a transition runs, and why; `None` when it runs nothing
/// and tells the page nothing (a stored ended run seeded again, a count-up's
/// sleep pause, a restart with no mute to re-apply), and always with
/// [`TIMER_ACTIONS`] off.
///
/// - `Started` (P1): all of `onStart` but `revertPreset`, `start`/`armed`.
/// - `Fired` (P2-P4): End now runs all of `onFire` (`endNow`, `lateMs` 0); a
///   fire at most [`LATE_ONE_SHOTS_MS`] late runs all of it (`zero`, its
///   lateness); a later one only its restoring actions (`late`, its
///   lateness).
/// - `Expired` (P5) and a run that ran out while Divora was closed
///   (`Seeded {expired, from: running}`, P6): `onFire`'s restoring actions,
///   `ranOut`.
/// - A restart mid-run (`Seeded {running|paused}`, P7): `onStart`'s mute
///   again, backend only (`restart`, never sent).
/// - `Cancelled` (P9, P10): the restoring actions of `onFire` that undo
///   what this run's start did ([`Action::undoes`] the `onStart` it started
///   with, not one edited since), `reset` or `deleted`.
///
/// Lists are the ones the transition carries: what they were when it
/// happened (Q12), and a cancel's start what it was at its start. P1-P6, P9 and P10 plan even with nothing to run, so the
/// page can settle a return and an end retires its start line. The backend
/// half puts `panic` first, the rest in list order (D15); the page half
/// keeps list order.
#[must_use]
pub fn plan(t: &Transition) -> Option<Planned> {
    if !TIMER_ACTIONS {
        return None;
    }
    let (id, hooks, hook, why, late_ms, take) = match t {
        Transition::Started { id, hooks, .. } => (id, hooks, Hook::Start, Why::Armed, 0, Take::All),
        Transition::Fired {
            id,
            finished: true,
            hooks,
            ..
        } => (id, hooks, Hook::End, Why::EndNow, 0, Take::All),
        Transition::Fired {
            id, late_ms, hooks, ..
        } if *late_ms <= LATE_ONE_SHOTS_MS => {
            (id, hooks, Hook::End, Why::Zero, *late_ms, Take::All)
        }
        Transition::Fired {
            id, late_ms, hooks, ..
        } => (id, hooks, Hook::End, Why::Late, *late_ms, Take::Restoring),
        Transition::Expired { id, hooks }
        | Transition::Seeded {
            id,
            phase: TimerPhase::Expired,
            from: TimerPhase::Running,
            hooks,
        } => (id, hooks, Hook::End, Why::RanOut, 0, Take::Restoring),
        Transition::Seeded {
            id,
            phase: TimerPhase::Running | TimerPhase::Paused,
            hooks,
            ..
        } => (id, hooks, Hook::Start, Why::Restart, 0, Take::Reapplied),
        // P8 (a stored ended run, seeded at every launch) and P11.
        Transition::Seeded { .. } | Transition::SleepPaused { .. } => return None,
        Transition::Cancelled {
            id,
            why,
            hooks,
            started,
        } => {
            let why = match why {
                CancelWhy::Reset => Why::Reset,
                CancelWhy::Removed => Why::Deleted,
            };
            let undoing = Take::Undoing(&started.on_start);
            (id, hooks, Hook::End, why, 0, undoing)
        }
    };
    let list = match hook {
        Hook::Start => &hooks.on_start,
        Hook::End => &hooks.on_fire,
    };
    let (mut backend, page): (Vec<Action>, Vec<Action>) = list
        .iter()
        .filter(|a| {
            a.runs_at(hook)
                && match take {
                    Take::All => true,
                    Take::Restoring => a.restores(),
                    Take::Undoing(on_start) => a.undoes(on_start),
                    Take::Reapplied => a.reapplies(),
                }
        })
        .cloned()
        .partition(|a| a.runs_in() == RunsIn::Backend);
    // D15: Stop all first, so it stops what played before the hook and
    // this hook's own clips and line play after it. A stable sort: the rest
    // keep list order.
    backend.sort_by_key(|a| *a != Action::Panic);
    if why == Why::Restart && backend.is_empty() {
        return None;
    }
    Some(Planned {
        id: id.clone(),
        hook,
        why,
        late_ms,
        backend,
        page,
    })
}

/// What the action worker runs for one hook run, in order (D14).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ActionBatch {
    pub id: String,
    pub hook: Hook,
    pub why: Why,
    /// [`replay_at`] of the drain and the fire's lateness: the moment a
    /// replay save covers up to (D22).
    pub at: Instant,
    /// The backend half, `panic` first.
    pub actions: Vec<Action>,
}

/// D22: the moment a save covers up to — `drained` less the fire's
/// lateness: the moment the countdown reached zero, or, for a start (and
/// End now, never late), the drain itself; `drained` if that is earlier
/// than an `Instant` can be.
#[must_use]
pub fn replay_at(drained: Instant, late_ms: u64) -> Instant {
    drained
        .checked_sub(Duration::from_millis(late_ms))
        .unwrap_or(drained)
}

/// D13: where a hook run goes. A batch for the worker when its backend half
/// has actions or it is an end — an end retires its timer's start line even
/// with nothing else to run (D21) — with `at` = [`replay_at`]`(drained,
/// late_ms)`; an event for the page when it [`emits`](Planned::emits),
/// numbered `seq`.
#[must_use]
pub fn route(
    p: &Planned,
    seq: u64,
    drained: Instant,
) -> (Option<ActionBatch>, Option<TimerActionsEvent>) {
    let batch = (!p.backend.is_empty() || p.hook == Hook::End).then(|| ActionBatch {
        id: p.id.clone(),
        hook: p.hook,
        why: p.why,
        at: replay_at(drained, p.late_ms),
        actions: p.backend.clone(),
    });
    let event = p.emits().then(|| TimerActionsEvent::of(p, seq));
    (batch, event)
}

/// The `timer-actions` payload: one hook run's page half. `seq` rises by one
/// per planned transition in this process (a gap is a restart's re-mute,
/// which isn't sent).
#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TimerActionsEvent {
    pub seq: u64,
    pub id: String,
    pub hook: Hook,
    pub why: Why,
    pub late_ms: u64,
    /// The page half, in list order, each as [`Action::to_wire`] writes it.
    pub actions: Vec<Value>,
}

impl TimerActionsEvent {
    /// `p`'s page half as the event numbered `seq`.
    #[must_use]
    pub fn of(p: &Planned, seq: u64) -> Self {
        Self {
            seq,
            id: p.id.clone(),
            hook: p.hook,
            why: p.why,
            late_ms: p.late_ms,
            actions: p.page.iter().map(Action::to_wire).collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        action_from_wire, hooks_from_wire, plan, replay_at, route, Action, Hook, Hooks, ObsOp,
        Planned, RunsIn, TimerActionsEvent, Why, LATE_ONE_SHOTS_MS, MAX_LINE_CHARS,
        MAX_TARGET_CHARS,
    };
    use crate::obs::{MAX_SCENE_NAME_CHARS, MAX_UUID_CHARS};
    use crate::timers::{entry_from_wire, CancelWhy, TimerPhase, Transition, WireEntry};
    use serde_json::{json, Value};
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    fn fixture() -> Value {
        serde_json::from_str(include_str!("../../src/overlay/timer.fixture.json")).unwrap()
    }

    /// A fixture row's name, for a failure message.
    fn row_name(row: &Value) -> &str {
        row["name"].as_str().unwrap_or("?")
    }

    fn rows<'a>(f: &'a Value, key: &str) -> &'a Vec<Value> {
        f[key].as_array().unwrap_or_else(|| panic!("{key}"))
    }

    /// v1.61.0: an OBS scene switch to `name`, by a uuid made from it.
    fn scene(name: &str) -> Action {
        Action::Obs(ObsOp::Scene {
            scene_uuid: format!("5d3f1c9e-{name}"),
            scene_name: name.into(),
        })
    }

    const SCENE_BACK: Action = Action::Obs(ObsOp::SceneBack);

    /// v1.61.0: one `obs` op of each, in the fixture's `actionOps.obs`
    /// order. The match makes a new op fail to compile here until it has
    /// its place (and its fixture row).
    fn every_obs_op() -> Vec<ObsOp> {
        let every = vec![
            ObsOp::Scene {
                scene_uuid: "5d3f1c9e-7a2b-4e6d-8f10-2c4b6a8e0d13".into(),
                scene_name: "BRB".into(),
            },
            ObsOp::SceneBack,
        ];
        for (i, op) in every.iter().enumerate() {
            let at = match op {
                ObsOp::Scene { .. } => 0,
                ObsOp::SceneBack => 1,
            };
            assert_eq!(at, i, "{op:?}");
        }
        every
    }

    /// One action of every kind, in §7.2's order. The match makes a new kind
    /// fail to compile here until it has its place (and its fixture row).
    /// v1.61.0: `obs` by its `scene` op, the one whose fields and hooks the
    /// kind's row carries.
    fn every_kind() -> Vec<Action> {
        let every = vec![
            Action::Preset {
                preset_id: "villager".into(),
            },
            Action::RandomPreset,
            Action::RevertPreset,
            Action::Tile {
                tile_id: "9f2c4e1a7b3d5f60".into(),
            },
            Action::Speak {
                text: "I'm back.".into(),
                voice_id: "af_heart".into(),
            },
            Action::SaveReplay,
            Action::SetMuted { muted: true },
            Action::Panic,
            Action::Obs(every_obs_op().swap_remove(0)),
        ];
        for (i, a) in every.iter().enumerate() {
            let at = match a {
                Action::Preset { .. } => 0,
                Action::RandomPreset => 1,
                Action::RevertPreset => 2,
                Action::Tile { .. } => 3,
                Action::Speak { .. } => 4,
                Action::SaveReplay => 5,
                Action::SetMuted { .. } => 6,
                Action::Panic => 7,
                Action::Obs(_) => 8,
            };
            assert_eq!(at, i, "{a:?}");
        }
        every
    }

    fn runs_in(r: RunsIn) -> &'static str {
        match r {
            RunsIn::Page => "page",
            RunsIn::Backend => "backend",
        }
    }

    /// Each list read, as the wire writes it.
    fn wire(list: &[Action]) -> Value {
        Value::from(list.iter().map(Action::to_wire).collect::<Vec<_>>())
    }

    /// The hooks `a` can run at, as the fixture names them (`zero` is the
    /// end's).
    fn hooks_of(a: &Action) -> Vec<&'static str> {
        [(Hook::Start, "start"), (Hook::End, "zero")]
            .into_iter()
            .filter(|(hook, _)| a.runs_at(*hook))
            .map(|(_, name)| name)
            .collect()
    }

    /// The fields beside `kind` in `wire`, sorted.
    fn fields_in(wire: &Value) -> Vec<String> {
        let mut fields: Vec<String> = wire.as_object().unwrap().keys().cloned().collect();
        fields.retain(|k| k != "kind");
        fields.sort();
        fields
    }

    /// §7.2's table, from the fixture's `actionKinds` (the page's
    /// `ACTION_KINDS` reads the same rows): every kind in order, where it
    /// runs, the hooks it can run at (`zero` is the end's), and the fields
    /// `to_wire` writes beside `kind`.
    #[test]
    fn action_kinds_match_the_fixture() {
        let f = fixture();
        let kinds = rows(&f, "actionKinds");
        let every = every_kind();
        assert_eq!(kinds.len(), every.len());
        for (row, a) in kinds.iter().zip(&every) {
            assert_eq!(row["kind"], a.kind(), "{a:?}");
            assert_eq!(row["runsIn"], runs_in(a.runs_in()), "{a:?}");
            assert_eq!(row["at"], json!(hooks_of(a)), "{a:?}");
            assert_eq!(row["fields"], json!(fields_in(&a.to_wire())), "{a:?}");
        }
        // Every kind has a valid sample.
        let sampled: Vec<&str> = rows(&f["actionSamples"], "valid")
            .iter()
            .map(|row| row["read"]["kind"].as_str().unwrap())
            .collect();
        for a in &every {
            assert!(sampled.contains(&a.kind()), "{a:?}");
        }
    }

    /// v1.61.0 (D21, §7.9): one kind, two ops. The fixture's `actionOps.obs`
    /// (the page's `ACTION_OPS` and `runsAt` read the same) holds each op's
    /// fields and hooks, as `to_wire` and `runs_at` give them; the kind's
    /// `actionKinds` row is their fields' union with `scene`'s hooks; every
    /// valid `obs` sample has its op's fields; and an op's serde shape is its
    /// wire shape less `kind`.
    #[test]
    fn action_ops_match_the_fixture() {
        let f = fixture();
        let ops = f["actionOps"].as_object().unwrap();
        assert_eq!(ops.keys().collect::<Vec<_>>(), ["obs"]);
        let obs = ops["obs"].as_object().unwrap();
        let every = every_obs_op();
        assert_eq!(obs.len(), every.len());
        let mut union = Vec::new();
        for op in &every {
            let a = Action::Obs(op.clone());
            let row = &obs[op.op()];
            assert_eq!(row["at"], json!(hooks_of(&a)), "{op:?}");
            let wire = a.to_wire();
            assert_eq!(row["fields"], json!(fields_in(&wire)), "{op:?}");
            union.extend(fields_in(&wire));
            let mut serde_wire = serde_json::to_value(op).unwrap();
            serde_wire["kind"] = json!("obs");
            assert_eq!(serde_wire, wire, "{op:?}");
        }
        union.sort();
        union.dedup();
        let row = rows(&f, "actionKinds")
            .iter()
            .find(|row| row["kind"] == "obs")
            .unwrap();
        assert_eq!(row["fields"], json!(union));
        assert_eq!(row["at"], obs["scene"]["at"]);
        let mut sampled = 0;
        for row in rows(&f["actionSamples"], "valid") {
            if row["read"]["kind"] != "obs" {
                continue;
            }
            sampled += 1;
            let op = row["read"]["op"].as_str().unwrap();
            assert_eq!(
                obs[op]["fields"],
                json!(fields_in(&row["read"])),
                "{}",
                row_name(row)
            );
        }
        assert!(sampled >= every.len());
    }

    /// §7.2, from the fixture's `actionSamples` (the page's `readAction`
    /// reads the same rows): each valid one reads as `read` says — and `read`
    /// reads back as itself — with its `runsIn`, `restores` and `reapplies`;
    /// each skipped one is `None`, but the lone surrogate, which JSON can't
    /// carry to Rust (`serde_json` refuses the whole invoke).
    #[test]
    fn action_from_wire_table() {
        let f = fixture();
        let samples = &f["actionSamples"];
        let valid = rows(samples, "valid");
        let skipped = rows(samples, "skipped");
        for row in valid {
            let name = row_name(row);
            let a = action_from_wire(&row["action"]).unwrap_or_else(|| panic!("{name}"));
            assert_eq!(a.to_wire(), row["read"], "{name}");
            assert_eq!(action_from_wire(&row["read"]), Some(a.clone()), "{name}");
            assert_eq!(row["runsIn"], runs_in(a.runs_in()), "{name}");
            assert_eq!(row["restores"], a.restores(), "{name}");
            assert_eq!(row["reapplies"], a.reapplies(), "{name}");
        }
        let mut page_only = 0;
        for row in skipped {
            if row["pageOnly"] == true {
                page_only += 1;
                continue;
            }
            assert_eq!(action_from_wire(&row["action"]), None, "{}", row_name(row));
        }
        // The line's, and v1.61.0's two in a scene switch's uuid and name.
        assert_eq!(page_only, 3);

        // The edges the rows above carry, stated here too, so neither the
        // fixture nor the reader can lose one.
        let speak = |text: String| json!({"kind": "speak", "text": text, "voiceId": "af_heart"});
        let line = "a".repeat(MAX_LINE_CHARS);
        assert!(action_from_wire(&speak(line.clone())).is_some());
        assert!(action_from_wire(&speak(format!("{line}a"))).is_none());
        assert!(action_from_wire(&speak(format!("  {line}  "))).is_some());
        // Trimmed by Unicode White_Space, as the page's `readAction` trims
        // (timerActions.test.ts pins the same three): U+0085 is trimmed,
        // U+FEFF isn't.
        let said = |text: &str| Action::Speak {
            text: text.into(),
            voice_id: "af_heart".into(),
        };
        let read = |text: &str| action_from_wire(&speak(text.into()));
        assert_eq!(read("\u{85}I'm back.\u{85}"), Some(said("I'm back.")));
        assert_eq!(read("\u{85}"), None);
        assert_eq!(read("\u{FEFF}I'm back."), Some(said("\u{FEFF}I'm back.")));
        // Characters, not UTF-8 bytes or UTF-16 units.
        assert!(action_from_wire(&speak("é".repeat(MAX_LINE_CHARS))).is_some());
        assert!(
            action_from_wire(&speak(format!("{}🙂", "a".repeat(MAX_LINE_CHARS - 1)))).is_some()
        );
        let tile = |id: String| json!({"kind": "tile", "tileId": id});
        assert!(action_from_wire(&tile("f".repeat(MAX_TARGET_CHARS))).is_some());
        assert!(action_from_wire(&tile("f".repeat(MAX_TARGET_CHARS + 1))).is_none());
        // No cap on a preset id.
        let preset = json!({"kind": "preset", "presetId": "a".repeat(10 * MAX_TARGET_CHARS)});
        assert!(action_from_wire(&preset).is_some());
        // v1.61.0 (D21): a scene's uuid is 1 to 128 characters, its name at
        // most 256 (empty allowed), counted in characters and kept as sent.
        let switch = |uuid: String, name: String| json!({"kind": "obs", "op": "scene", "sceneUuid": uuid, "sceneName": name});
        let uuid = || "5d3f1c9e-7a2b-4e6d-8f10-2c4b6a8e0d13".to_string();
        let ok = |uuid: String, name: String| action_from_wire(&switch(uuid, name)).is_some();
        assert!(ok("f".repeat(MAX_UUID_CHARS), "BRB".into()));
        assert!(!ok("f".repeat(MAX_UUID_CHARS + 1), "BRB".into()));
        assert!(ok("é".repeat(MAX_UUID_CHARS), "BRB".into()));
        assert!(!ok(String::new(), "BRB".into()));
        assert!(ok(uuid(), "a".repeat(MAX_SCENE_NAME_CHARS)));
        assert!(!ok(uuid(), "a".repeat(MAX_SCENE_NAME_CHARS + 1)));
        assert!(ok(uuid(), "🙂".repeat(MAX_SCENE_NAME_CHARS)));
        assert!(ok(uuid(), String::new()));
        assert_eq!(
            action_from_wire(&switch(uuid(), " BRB ".into())),
            Some(Action::Obs(ObsOp::Scene {
                scene_uuid: uuid(),
                scene_name: " BRB ".into(),
            }))
        );
    }

    /// TP1: every kind, and both mutes, read back from what `to_wire`
    /// writes, as they were — so the event's actions are what the page's
    /// reader takes.
    #[test]
    fn to_wire_round_trips() {
        let f = fixture();
        let mut all = every_kind();
        all.push(Action::SetMuted { muted: false });
        for row in rows(&f["actionSamples"], "valid") {
            all.push(action_from_wire(&row["action"]).unwrap());
        }
        for a in all {
            let wire = a.to_wire();
            assert_eq!(action_from_wire(&wire), Some(a.clone()), "{wire}");
            // camelCase on the wire, nothing else beside the kind's fields.
            for key in wire.as_object().unwrap().keys() {
                assert!(!key.contains('_'), "{wire}");
            }
        }
    }

    /// §7.2's list rules, from the fixture's `listSamples` (the page's
    /// `actionsForSync` reads the same rows): each list read on its own —
    /// the first 8 entries, the unreadable skipped, one line — and a sync
    /// entry's lists reach the core only for a countdown (D32).
    #[test]
    fn hooks_from_wire_reads_eight() {
        let f = fixture();
        for row in rows(&f, "listSamples") {
            let name = row_name(row);
            let entry = row["entry"].as_object().unwrap();
            let read = hooks_from_wire(entry);
            assert_eq!(wire(&read.on_start), row["read"]["onStart"], "{name}");
            assert_eq!(wire(&read.on_fire), row["read"]["onFire"], "{name}");
            let Ok(WireEntry::Def(def)) = entry_from_wire(&row["entry"]) else {
                panic!("{name}: a definition");
            };
            for (list, key) in [
                (&def.hooks.on_start, "onStart"),
                (&def.hooks.on_fire, "onFire"),
            ] {
                let sent = row["sent"].get(key).cloned().unwrap_or_else(|| json!([]));
                assert_eq!(wire(list), sent, "{name}: {key}");
            }
        }
        // Ten entries, the third an `obs` op this build doesn't know:
        // entries 1-8 are read, so 7 run, and the 10th's line never counts.
        let ten = json!({"onFire": [
            {"kind": "setMuted", "muted": true},
            {"kind": "preset", "presetId": "villager"},
            {"kind": "obs", "op": "startStream"},
            {"kind": "tile", "tileId": "9f2c4e1a7b3d5f60"},
            {"kind": "saveReplay"},
            {"kind": "randomPreset"},
            {"kind": "panic"},
            {"kind": "revertPreset"},
            {"kind": "setMuted", "muted": false},
            {"kind": "speak", "text": "I'm back.", "voiceId": "af_heart"},
        ]});
        let read = hooks_from_wire(ten.as_object().unwrap());
        assert_eq!(read.on_fire.len(), 7);
        assert_eq!(read.on_fire.last(), Some(&Action::RevertPreset));
        assert_eq!(read.on_start, Vec::<Action>::new());
        let odd = json!({"onStart": {}, "onFire": 5});
        assert_eq!(hooks_from_wire(odd.as_object().unwrap()), Hooks::default());
    }

    /// D2: a list's second line is skipped; one that can't be read doesn't
    /// use up the list's line.
    #[test]
    fn hooks_from_wire_keeps_one_line() {
        let speak = |text: &str| json!({"kind": "speak", "text": text, "voiceId": "af_heart"});
        let said = |text: &str| Action::Speak {
            text: text.into(),
            voice_id: "af_heart".into(),
        };
        let entry = json!({
            "onStart": [speak("A"), {"kind": "setMuted", "muted": true}, speak("B")],
            "onFire": [speak("   "), speak("C"), speak("D")],
        });
        let read = hooks_from_wire(entry.as_object().unwrap());
        assert_eq!(read.on_start, [said("A"), Action::SetMuted { muted: true }]);
        assert_eq!(read.on_fire, [said("C")]);
    }

    /// The `Transition` an `actionsEventSamples` row states, with its
    /// lists read as a sync would read them.
    fn transition_of(row: &Value) -> Transition {
        let name = row_name(row);
        let t = &row["transition"];
        let lists = row["lists"].as_object().unwrap();
        let hooks = hooks_from_wire(lists);
        // Every entry reads, so a row tests the plan, not the reader.
        for (list, key) in [(&hooks.on_start, "onStart"), (&hooks.on_fire, "onFire")] {
            let stored = lists.get(key).and_then(Value::as_array).map_or(0, Vec::len);
            assert_eq!(list.len(), stored, "{name}: {key}");
        }
        let hooks = Arc::new(hooks);
        let id = t["id"].as_str().unwrap().to_string();
        let fires = || u32::try_from(t["fires"].as_u64().unwrap()).unwrap();
        match t["variant"].as_str().unwrap() {
            "Started" => Transition::Started {
                id,
                fires: fires(),
                hooks,
            },
            "Fired" => Transition::Fired {
                id,
                fires: fires(),
                late_ms: t["lateMs"].as_u64().unwrap(),
                finished: t["finished"].as_bool().unwrap(),
                hooks,
            },
            "Expired" => Transition::Expired { id, hooks },
            "Cancelled" => Transition::Cancelled {
                id,
                why: match t["why"].as_str() {
                    Some("reset") => CancelWhy::Reset,
                    Some("removed") => CancelWhy::Removed,
                    other => panic!("{name}: why {other:?}"),
                },
                // A run that started with the lists the row states.
                started: Arc::clone(&hooks),
                hooks,
            },
            other => panic!("{name}: variant {other}"),
        }
    }

    fn named<T: serde::Serialize + Copy>(all: &[T], name: &Value) -> T {
        *all.iter()
            .find(|v| serde_json::to_value(v).unwrap() == *name)
            .unwrap_or_else(|| panic!("{name}"))
    }

    const WHYS: [Why; 8] = [
        Why::Armed,
        Why::Zero,
        Why::EndNow,
        Why::Late,
        Why::RanOut,
        Why::Reset,
        Why::Deleted,
        Why::Restart,
    ];

    /// §4's `timer-actions` wire, from the fixture's `actionsEventSamples`
    /// (the page's `readTimerActionsEvent` reads the same rows): one per
    /// `why`, in order; each event's keys are `actionsEventKeys`; a start
    /// only for `armed`; `lateMs` the fire's lateness for `zero` and `late`
    /// and 0 otherwise; only page kinds in `actions`; and the event Rust
    /// builds from that page half is the row's, byte for byte. Each row's
    /// transition builds. (That `plan` gives exactly these is
    /// `actions_event_samples_are_plans`.)
    #[test]
    fn actions_event_samples_are_rusts() {
        let f = fixture();
        let samples = rows(&f, "actionsEventSamples");
        let whys: Vec<&Value> = samples.iter().map(|row| &row["event"]["why"]).collect();
        assert_eq!(
            whys,
            f["whys"].as_array().unwrap().iter().collect::<Vec<_>>()
        );
        let keys: Vec<Value> = f["actionsEventKeys"].as_array().unwrap().clone();
        for row in samples {
            let name = row_name(row);
            let event = &row["event"];
            let mut got: Vec<&String> = event.as_object().unwrap().keys().collect();
            got.sort();
            assert_eq!(json!(got), json!(keys), "{name}");
            let t = transition_of(row);
            let why = named(&WHYS, &event["why"]);
            let hook = named(&[Hook::Start, Hook::End], &event["hook"]);
            assert_eq!(hook == Hook::Start, why == Why::Armed, "{name}");
            let late_ms = event["lateMs"].as_u64().unwrap();
            match (&t, why) {
                (Transition::Fired { late_ms: fired, .. }, Why::Zero | Why::Late) => {
                    assert_eq!(late_ms, *fired, "{name}");
                }
                _ => assert_eq!(late_ms, 0, "{name}"),
            }
            let page: Vec<Action> = event["actions"]
                .as_array()
                .unwrap()
                .iter()
                .map(|a| action_from_wire(a).unwrap_or_else(|| panic!("{name}: {a}")))
                .collect();
            assert!(page.iter().all(|a| a.runs_in() == RunsIn::Page), "{name}");
            let planned = Planned {
                id: event["id"].as_str().unwrap().to_string(),
                hook,
                why,
                late_ms,
                backend: Vec::new(),
                page,
            };
            assert!(planned.emits(), "{name}");
            let seq = row["seq"].as_u64().unwrap();
            assert_eq!(
                serde_json::to_value(TimerActionsEvent::of(&planned, seq)).unwrap(),
                *event,
                "{name}"
            );
        }
        // The `zero` row's page half holds every page kind.
        let zero = samples
            .iter()
            .find(|row| row["event"]["why"] == "zero")
            .unwrap();
        let kinds: Vec<&str> = zero["event"]["actions"]
            .as_array()
            .unwrap()
            .iter()
            .map(|a| a["kind"].as_str().unwrap())
            .collect();
        assert_eq!(kinds, ["preset", "randomPreset", "revertPreset", "tile"]);
    }

    /// Seam 4: each `actionsEventSamples` row — the event the page's
    /// `readTimerActionsEvent` reads — is what `plan` makes of its
    /// transition, one per `why`.
    #[test]
    fn actions_event_samples_are_plans() {
        let f = fixture();
        for row in rows(&f, "actionsEventSamples") {
            let name = row_name(row);
            let p = plan(&transition_of(row)).unwrap_or_else(|| panic!("{name}: planned"));
            let seq = row["seq"].as_u64().unwrap();
            assert_eq!(
                serde_json::to_value(TimerActionsEvent::of(&p, seq)).unwrap(),
                row["event"],
                "{name}"
            );
        }
    }

    // ---- §7.1: `plan`, and where `route` sends what it plans -------------

    fn preset(id: &str) -> Action {
        Action::Preset {
            preset_id: id.into(),
        }
    }

    fn tile() -> Action {
        Action::Tile {
            tile_id: "9f2c4e1a7b3d5f60".into(),
        }
    }

    fn line(text: &str) -> Action {
        Action::Speak {
            text: text.into(),
            voice_id: "af_heart".into(),
        }
    }

    const MUTE: Action = Action::SetMuted { muted: true };
    const UNMUTE: Action = Action::SetMuted { muted: false };

    /// Both lists holding every kind, none of them in the order `plan`
    /// puts them in: `panic` neither first nor last, page and backend kinds
    /// mixed, `revertPreset` in At start too.
    fn every_hook() -> Arc<Hooks> {
        Arc::new(Hooks {
            on_start: vec![
                preset("villager"),
                MUTE,
                tile(),
                Action::RevertPreset,
                line("Be right back."),
                Action::Panic,
                Action::RandomPreset,
                Action::SaveReplay,
            ],
            on_fire: vec![
                tile(),
                UNMUTE,
                preset("hollow-king"),
                line("I'm back."),
                Action::Panic,
                Action::RevertPreset,
                Action::SaveReplay,
                Action::RandomPreset,
            ],
        })
    }

    fn lists(on_start: Vec<Action>, on_fire: Vec<Action>) -> Arc<Hooks> {
        Arc::new(Hooks { on_start, on_fire })
    }

    fn started(hooks: &Arc<Hooks>) -> Transition {
        Transition::Started {
            id: "timer-1".into(),
            fires: 0,
            hooks: hooks.clone(),
        }
    }

    fn fired(hooks: &Arc<Hooks>, late_ms: u64, finished: bool) -> Transition {
        Transition::Fired {
            id: "timer-1".into(),
            fires: 1,
            late_ms,
            finished,
            hooks: hooks.clone(),
        }
    }

    fn expired(hooks: &Arc<Hooks>) -> Transition {
        Transition::Expired {
            id: "timer-1".into(),
            hooks: hooks.clone(),
        }
    }

    fn seeded(hooks: &Arc<Hooks>, phase: TimerPhase, from: TimerPhase) -> Transition {
        Transition::Seeded {
            id: "timer-1".into(),
            phase,
            from,
            hooks: hooks.clone(),
        }
    }

    /// A cancel of a run that started with the lists it has.
    fn cancelled(hooks: &Arc<Hooks>, why: CancelWhy) -> Transition {
        cancelled_since(hooks, hooks, why)
    }

    /// A cancel with `hooks` now, of a run that started with `started`:
    /// its lists were edited mid-run.
    fn cancelled_since(hooks: &Arc<Hooks>, started: &Arc<Hooks>, why: CancelWhy) -> Transition {
        Transition::Cancelled {
            id: "timer-1".into(),
            why,
            hooks: hooks.clone(),
            started: started.clone(),
        }
    }

    #[allow(clippy::unnecessary_wraps)] // the shape `plan` answers in
    fn planned(
        hook: Hook,
        why: Why,
        late_ms: u64,
        backend: Vec<Action>,
        page: Vec<Action>,
    ) -> Option<Planned> {
        Some(Planned {
            id: "timer-1".into(),
            hook,
            why,
            late_ms,
            backend,
            page,
        })
    }

    /// §7.1's rows, P1-P11, on [`every_hook`], then the rows its undo rule
    /// adds (AT1, UX1, AT5): the transition, and what `plan` makes of it.
    /// P12 records no transition, so there is nothing to plan: `op_table`
    /// and `a_sync_takes_new_hooks` pin that the core records none.
    #[allow(clippy::too_many_lines)] // §7.1 in one table, a row per line of it
    fn plan_rows() -> Vec<(&'static str, Transition, Option<Planned>)> {
        use CancelWhy::{Removed, Reset};
        use Hook::{End, Start};
        use TimerPhase::{Done, Expired, Paused, Running};
        let every = every_hook();
        // onFire whole: Stop all first, the rest in list order.
        let fire_backend = || vec![Action::Panic, UNMUTE, line("I'm back."), Action::SaveReplay];
        let fire_page = || {
            vec![
                tile(),
                preset("hollow-king"),
                Action::RevertPreset,
                Action::RandomPreset,
            ]
        };
        // onFire·R, and here onFire·U too: At start mutes.
        let restoring = |why| planned(End, why, 0, vec![UNMUTE], vec![Action::RevertPreset]);
        vec![
            (
                "P1 Started: all of At start, Stop all first, no revertPreset",
                started(&every),
                planned(
                    Start,
                    Why::Armed,
                    0,
                    vec![
                        Action::Panic,
                        MUTE,
                        line("Be right back."),
                        Action::SaveReplay,
                    ],
                    vec![preset("villager"), tile(), Action::RandomPreset],
                ),
            ),
            (
                "P2 Fired on time: all of At zero",
                fired(&every, 0, false),
                planned(End, Why::Zero, 0, fire_backend(), fire_page()),
            ),
            (
                "P2 Fired 30 000 ms late: still all of it",
                fired(&every, LATE_ONE_SHOTS_MS, false),
                planned(End, Why::Zero, 30_000, fire_backend(), fire_page()),
            ),
            (
                "P3 End now: all of At zero, not late",
                fired(&every, 0, true),
                planned(End, Why::EndNow, 0, fire_backend(), fire_page()),
            ),
            (
                "P4 Fired 30 001 ms late: restoring only",
                fired(&every, LATE_ONE_SHOTS_MS + 1, false),
                planned(
                    End,
                    Why::Late,
                    30_001,
                    vec![UNMUTE],
                    vec![Action::RevertPreset],
                ),
            ),
            (
                "P4 Fired an hour late: restoring only",
                fired(&every, 3_600_000, false),
                planned(
                    End,
                    Why::Late,
                    3_600_000,
                    vec![UNMUTE],
                    vec![Action::RevertPreset],
                ),
            ),
            (
                "P5 Expired: restoring only",
                expired(&every),
                restoring(Why::RanOut),
            ),
            (
                "P6 ran out while Divora was closed: restoring only",
                seeded(&every, Expired, Running),
                restoring(Why::RanOut),
            ),
            (
                "P7 restart while running: At start's mute again, backend only",
                seeded(&every, Running, Running),
                planned(Start, Why::Restart, 0, vec![MUTE], vec![]),
            ),
            (
                "P7 restart while paused: the same",
                seeded(&every, Paused, Paused),
                planned(Start, Why::Restart, 0, vec![MUTE], vec![]),
            ),
            (
                "P8 a stored done run: nothing",
                seeded(&every, Done, Done),
                None,
            ),
            (
                "P8 a stored ran-out run: nothing",
                seeded(&every, Expired, Expired),
                None,
            ),
            (
                "P9 Reset mid-run: the undo of its start",
                cancelled(&every, Reset),
                restoring(Why::Reset),
            ),
            (
                "P10 deleted mid-run: the undo of its start",
                cancelled(&every, Removed),
                restoring(Why::Deleted),
            ),
            (
                "P11 a count-up's sleep pause: nothing",
                Transition::SleepPaused {
                    id: "timer-1".into(),
                },
                None,
            ),
            // The undo rule's rows.
            (
                "unmute in 5 minutes, Reset: its start muted nothing",
                cancelled(&lists(vec![], vec![UNMUTE]), Reset),
                planned(End, Why::Reset, 0, vec![], vec![]),
            ),
            (
                "unmute in 5 minutes, deleted: the same",
                cancelled(&lists(vec![], vec![UNMUTE]), Removed),
                planned(End, Why::Deleted, 0, vec![], vec![]),
            ),
            (
                "unmute in 5 minutes, Expired: zero passed",
                expired(&lists(vec![], vec![UNMUTE])),
                planned(End, Why::RanOut, 0, vec![UNMUTE], vec![]),
            ),
            (
                "unmute in 5 minutes, found late: zero passed",
                fired(&lists(vec![], vec![UNMUTE]), 40_000, false),
                planned(End, Why::Late, 40_000, vec![UNMUTE], vec![]),
            ),
            (
                "unmute in 5 minutes, ran out while closed: zero passed",
                seeded(&lists(vec![], vec![UNMUTE]), Expired, Running),
                planned(End, Why::RanOut, 0, vec![UNMUTE], vec![]),
            ),
            (
                "a break, Reset: unmuted",
                cancelled(&lists(vec![MUTE], vec![UNMUTE]), Reset),
                planned(End, Why::Reset, 0, vec![UNMUTE], vec![]),
            ),
            (
                "a mute with nothing at zero, Reset: you stay muted",
                cancelled(&lists(vec![MUTE], vec![]), Reset),
                planned(End, Why::Reset, 0, vec![], vec![]),
            ),
            (
                "a mute with nothing at zero, at zero: you stay muted",
                fired(&lists(vec![MUTE], vec![]), 0, false),
                planned(End, Why::Zero, 0, vec![], vec![]),
            ),
            (
                "a switch, deleted: back to the preset before",
                cancelled(
                    &lists(vec![preset("x")], vec![Action::RevertPreset]),
                    Removed,
                ),
                planned(End, Why::Deleted, 0, vec![], vec![Action::RevertPreset]),
            ),
            (
                "a mute edited in after the start, restart: the current lists",
                seeded(&lists(vec![MUTE], vec![UNMUTE]), Running, Running),
                planned(Start, Why::Restart, 0, vec![MUTE], vec![]),
            ),
            (
                "a break, its start's mute edited out mid-run, Reset: its start muted",
                cancelled_since(
                    &lists(vec![], vec![UNMUTE]),
                    &lists(vec![MUTE], vec![UNMUTE]),
                    Reset,
                ),
                planned(End, Why::Reset, 0, vec![UNMUTE], vec![]),
            ),
            (
                "unmute in 5 minutes, a mute edited in mid-run, deleted: its start muted nothing",
                cancelled_since(
                    &lists(vec![MUTE], vec![UNMUTE]),
                    &lists(vec![], vec![UNMUTE]),
                    Removed,
                ),
                planned(End, Why::Deleted, 0, vec![], vec![]),
            ),
            (
                "a break, its unmute edited out mid-run, Reset: At zero as it is now",
                cancelled_since(
                    &lists(vec![MUTE], vec![]),
                    &lists(vec![MUTE], vec![UNMUTE]),
                    Reset,
                ),
                planned(End, Why::Reset, 0, vec![], vec![]),
            ),
            (
                "a line, a clip and Stop all at zero: Stop all first",
                fired(
                    &lists(vec![MUTE], vec![line("I'm back."), tile(), Action::Panic]),
                    0,
                    false,
                ),
                planned(
                    End,
                    Why::Zero,
                    0,
                    vec![Action::Panic, line("I'm back.")],
                    vec![tile()],
                ),
            ),
            // v1.61.0 (§7.10): OBS's scene switches go where every backend
            // kind goes, with no change to `plan`.
            (
                "obs: a break's start mutes, then switches",
                started(&obs_break()),
                planned(Start, Why::Armed, 0, vec![MUTE, scene("BRB")], vec![]),
            ),
            (
                "obs: at zero, every switch, in list order",
                fired(&obs_break(), 0, false),
                planned(End, Why::Zero, 0, obs_zero(), vec![]),
            ),
            (
                "obs: End now, the same",
                fired(&obs_break(), 0, true),
                planned(End, Why::EndNow, 0, obs_zero(), vec![]),
            ),
            (
                "obs: found late, only the switch back",
                fired(&obs_break(), 40_000, false),
                planned(End, Why::Late, 40_000, vec![UNMUTE, SCENE_BACK], vec![]),
            ),
            (
                "obs: slept, only the switch back",
                expired(&obs_break()),
                planned(End, Why::RanOut, 0, vec![UNMUTE, SCENE_BACK], vec![]),
            ),
            (
                "obs: ran out while Divora was closed, only the switch back",
                seeded(&obs_break(), Expired, Running),
                planned(End, Why::RanOut, 0, vec![UNMUTE, SCENE_BACK], vec![]),
            ),
            (
                "obs: Reset, the switch back: its start switched",
                cancelled(&obs_break(), Reset),
                planned(End, Why::Reset, 0, vec![UNMUTE, SCENE_BACK], vec![]),
            ),
            (
                "obs: deleted, a start that switched nothing: no switch back",
                cancelled(&lists(vec![MUTE], vec![UNMUTE, SCENE_BACK]), Removed),
                planned(End, Why::Deleted, 0, vec![UNMUTE], vec![]),
            ),
            (
                "obs: a restart mid-break re-applies only the mute",
                seeded(&obs_break(), Running, Running),
                planned(Start, Why::Restart, 0, vec![MUTE], vec![]),
            ),
        ]
    }

    /// v1.61.0: a break that switches OBS (`Add break` with a break scene),
    /// with a switch at zero too.
    fn obs_break() -> Arc<Hooks> {
        lists(
            vec![MUTE, scene("BRB")],
            vec![UNMUTE, scene("Just Chatting"), SCENE_BACK],
        )
    }

    /// [`obs_break`]'s At zero, whole.
    fn obs_zero() -> Vec<Action> {
        vec![UNMUTE, scene("Just Chatting"), SCENE_BACK]
    }

    /// D7, §7.1: every row — the backend and page halves, `hook`, `why`,
    /// `lateMs`, whether the page hears of it, list order kept, Stop all
    /// first — and the undo rule's rows: a Reset or a delete runs only what
    /// undoes the start, a late fire or a run-out every restoring action.
    #[test]
    fn plan_table() {
        for (name, t, want) in plan_rows() {
            let got = plan(&t);
            assert_eq!(got, want, "{name}");
            let Some(p) = got else { continue };
            let restart = matches!(
                t,
                Transition::Seeded {
                    phase: TimerPhase::Running | TimerPhase::Paused,
                    ..
                }
            );
            assert_eq!(p.emits(), !restart, "{name}");
            assert!(
                p.backend.iter().all(|a| a.runs_in() == RunsIn::Backend),
                "{name}"
            );
            assert!(p.page.iter().all(|a| a.runs_in() == RunsIn::Page), "{name}");
        }
    }

    /// D9: 30 000 ms late is on time — all of At zero, `zero` — and 30 001
    /// is late: only the restoring actions, `late`.
    #[test]
    fn late_boundary() {
        assert_eq!(LATE_ONE_SHOTS_MS, 30_000);
        let every = every_hook();
        let on_time = plan(&fired(&every, 30_000, false)).unwrap();
        assert_eq!((on_time.why, on_time.late_ms), (Why::Zero, 30_000));
        assert_eq!(
            on_time.backend.len() + on_time.page.len(),
            every.on_fire.len()
        );
        let late = plan(&fired(&every, 30_001, false)).unwrap();
        assert_eq!(
            (late.why, late.late_ms, late.backend, late.page),
            (Why::Late, 30_001, vec![UNMUTE], vec![Action::RevertPreset])
        );
    }

    /// D11: a restart mid-run (running or paused) runs At start's mute
    /// again and nothing else — no preset, line or replay — in the backend,
    /// and tells the page nothing; with no mute in At start it plans
    /// nothing at all.
    #[test]
    fn restart_reapplies_only_the_mute() {
        let start = lists(
            vec![
                preset("villager"),
                MUTE,
                line("Be right back."),
                Action::SaveReplay,
            ],
            vec![UNMUTE, Action::RevertPreset],
        );
        for phase in [TimerPhase::Running, TimerPhase::Paused] {
            let p = plan(&seeded(&start, phase, phase)).unwrap();
            assert_eq!(
                Some(p.clone()),
                planned(Hook::Start, Why::Restart, 0, vec![MUTE], vec![]),
                "{phase:?}"
            );
            assert!(!p.emits(), "{phase:?}");
            let (batch, event) = route(&p, 1, Instant::now());
            assert_eq!(batch.map(|b| b.actions), Some(vec![MUTE]), "{phase:?}");
            assert_eq!(event, None, "{phase:?}");
        }
        let no_mute = lists(vec![preset("villager"), tile()], vec![]);
        for phase in [TimerPhase::Running, TimerPhase::Paused] {
            assert_eq!(plan(&seeded(&no_mute, phase, phase)), None, "{phase:?}");
        }
    }

    /// P8: a stored done or ran-out run, seeded again at every launch, runs
    /// nothing — not even its restoring actions, which ran when it ended.
    #[test]
    fn a_stored_end_runs_nothing() {
        let every = every_hook();
        for phase in [TimerPhase::Done, TimerPhase::Expired] {
            assert_eq!(plan(&seeded(&every, phase, phase)), None, "{phase:?}");
        }
    }

    /// §7.1: `revertPreset` in At start is read and never run: there is
    /// nothing to go back to at a start. The start still plans, so the page
    /// hears of it.
    #[test]
    fn revert_never_runs_at_a_start() {
        let p = plan(&started(&lists(vec![Action::RevertPreset], vec![]))).unwrap();
        assert_eq!(
            (p.hook, p.why, p.backend, p.page),
            (Hook::Start, Why::Armed, vec![], vec![])
        );
        let p = plan(&started(&lists(
            vec![
                Action::RevertPreset,
                preset("villager"),
                Action::RevertPreset,
            ],
            vec![],
        )))
        .unwrap();
        assert_eq!(p.page, vec![preset("villager")]);
    }

    /// v1.61.0 (D21): OBS's `sceneBack` in At start is read and never run,
    /// as `revertPreset`; a `scene` there runs.
    #[test]
    fn scene_back_never_runs_at_a_start() {
        assert!(!SCENE_BACK.runs_at(Hook::Start));
        assert!(SCENE_BACK.runs_at(Hook::End));
        assert!(scene("BRB").runs_at(Hook::Start));
        let p = plan(&started(&lists(
            vec![SCENE_BACK, scene("BRB"), SCENE_BACK],
            vec![],
        )))
        .unwrap();
        assert_eq!((p.backend, p.page), (vec![scene("BRB")], vec![]));
    }

    /// The instant a test drains at: an hour ahead, so taking a lateness
    /// off it never meets the process's first `Instant`.
    fn drained() -> Instant {
        Instant::now() + Duration::from_secs(3_600)
    }

    /// `ms` before `at`.
    fn ms_before(at: Instant, ms: u64) -> Instant {
        at.checked_sub(Duration::from_millis(ms)).unwrap()
    }

    /// D13, D21, D22, for every `plan_table` row: a batch iff the backend
    /// half has actions or it is an end (an end retires its start line), its
    /// `at` its hook's moment — the drain less the fire's lateness, so the
    /// start, or the moment the countdown reached zero or End now ended it —
    /// and an event iff the page hears of it, numbered as asked.
    #[test]
    fn route_table() {
        let at = drained();
        let (mut empty_ends, mut late) = (0, 0);
        for (seq, (name, _, want)) in (1_u64..).zip(plan_rows()) {
            let Some(p) = want else { continue };
            let (batch, event) = route(&p, seq, at);
            let batched = !p.backend.is_empty() || p.hook == Hook::End;
            assert_eq!(batch.is_some(), batched, "{name}");
            if let Some(b) = batch {
                assert_eq!(
                    (b.id.as_str(), b.hook, b.why, &b.actions),
                    (p.id.as_str(), p.hook, p.why, &p.backend),
                    "{name}"
                );
                assert_eq!(b.at, ms_before(at, p.late_ms), "{name}");
                empty_ends += usize::from(b.actions.is_empty());
                late += usize::from(p.late_ms > 0);
            }
            assert_eq!(
                event,
                p.emits().then(|| TimerActionsEvent::of(&p, seq)),
                "{name}"
            );
        }
        // The rows above hold ends with nothing for the worker to run, and
        // late fires.
        assert!(empty_ends > 0 && late > 0, "{empty_ends} {late}");
    }

    /// D22: `replay_at` takes the lateness off the drain; a lateness longer
    /// than the process has existed gives the drain itself.
    #[test]
    fn replay_at_table() {
        let at = drained();
        assert_eq!(replay_at(at, 0), at);
        assert_eq!(replay_at(at, 1), ms_before(at, 1));
        assert_eq!(replay_at(at, 30_000), ms_before(at, 30_000));
        assert_eq!(replay_at(at, u64::MAX), at);
    }
}
