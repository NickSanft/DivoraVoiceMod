//! v1.61.0: which scene each break replaced, and the switch-backs waiting
//! for OBS, as pure rules (§7.6, D22-D24).
//!
//! A *slot* is what a timer's `scene` op recorded: the program scene it
//! found (`from`) and the one it switched to (`to`). Its `sceneBack` switches
//! back only while OBS is still on `to`. A *due mark* is a `sceneBack` that
//! couldn't run yet, stamped with the moment `offer` received it and held up
//! to the grace from that moment (D23). `ObsShared` holds one [`Slots`] for
//! the life of the process, loaded from `returns.json` at launch, so a
//! reconnect, Try again, a saved password or a Divora restart doesn't lose a
//! break's switch back. Nothing here reads a clock, waits or touches a file:
//! callers hand in the time and save the JSON.

use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

/// Slots kept at most; the oldest is dropped past it.
pub const MAX_SLOTS: usize = 64;

/// D24: a stored slot older than this (24 h) is dropped when read.
pub const SLOT_MAX_AGE_MS: i64 = 86_400_000;

/// A scene, by uuid (what switches) and name (what is shown).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Hash)]
pub struct SceneRef {
    pub uuid: String,
    pub name: String,
}

/// §7.6: what `timer`'s `scene` op recorded.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Slot {
    pub timer: String,
    /// The program scene it found.
    pub from: SceneRef,
    /// The scene it switched to.
    pub to: SceneRef,
    /// Wall ms it was set at (for the 24 h prune).
    pub set_at_ms: i64,
}

/// §7.6: what a `sceneBack` does, planned against the program scene now;
/// [`Slots::commit_back`] applies it once its Set is answered (or none was
/// needed), so a switch back that can't finish stays due with its slot.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct BackPlan {
    /// The scene to switch to; `None`: no switch (no slot, nothing to go back
    /// to, or you moved on).
    pub to: Option<SceneRef>,
    /// A stacked break's slot (its timer) whose `from` becomes this slot's.
    pub inherit: Option<(String, SceneRef)>,
}

/// D24: every slot and due mark, in `ObsShared` for the life of the
/// process. Due marks are in memory only.
#[derive(Clone, Debug, Default)]
pub struct Slots {
    /// At most [`MAX_SLOTS`].
    #[allow(dead_code)] // S1: track A's bodies read it
    slots: Vec<Slot>,
    /// `(timer, offer's stamp)`.
    due: Vec<(String, Instant)>,
}

impl Slots {
    /// §7.6's first table: `timer`'s `scene {to}` found `cur` on air;
    /// records the slot (before the Set, so a switch back works even if a
    /// timed-out Set lands later) and answers whether to send the Set.
    #[allow(unused_variables, clippy::unused_self)] // S1: track A writes this body
    pub fn after_scene(&mut self, timer: &str, cur: SceneRef, to: SceneRef, wall_ms: i64) -> bool {
        todo!("S1: track A")
    }

    /// §7.6's second table: what `timer`'s `sceneBack` does with `cur_uuid`
    /// on air. Changes nothing.
    #[allow(unused_variables, clippy::unused_self)] // S1: track A writes this body
    pub fn plan_back(&self, timer: &str, cur_uuid: &str) -> BackPlan {
        todo!("S1: track A")
    }

    /// The plan done: `timer`'s slot dropped, an inheritance applied.
    #[allow(unused_variables, clippy::unused_self)] // S1: track A writes this body
    pub fn commit_back(&mut self, timer: &str, plan: BackPlan) {
        todo!("S1: track A")
    }

    /// D23: `timer`'s switch back is due, stamped `at`; a mark already there
    /// keeps its earlier stamp.
    #[allow(unused_variables, clippy::unused_self)] // S1: track A writes this body
    pub fn mark_due(&mut self, timer: &str, at: Instant) {
        todo!("S1: track A")
    }

    /// D23: the due mark to run next (taken out; one whose timer has no slot
    /// does nothing), and the timers whose marks are older than `grace` at
    /// `now` (dropped).
    #[allow(unused_variables, clippy::unused_self)] // S1: track A writes this body
    pub fn take_due(&mut self, now: Instant, grace: Duration) -> (Option<String>, Vec<String>) {
        todo!("S1: track A")
    }

    /// D23: when the earliest due mark's `grace` runs out.
    #[allow(unused_variables, clippy::unused_self)] // S1: track A writes this body
    pub fn next_expiry(&self, grace: Duration) -> Option<Instant> {
        todo!("S1: track A")
    }

    /// D4: `obs_configure {enabled:false}` drops every due mark (the slots
    /// stay).
    pub fn drop_due(&mut self) {
        self.due.clear();
    }

    /// D23: the status's `heldBack`: due marks whose timer has a slot.
    #[allow(clippy::unused_self)] // S1: track A writes this body
    pub fn due_count(&self) -> u32 {
        todo!("S1: track A")
    }

    /// D24: `returns.json`: `{version: 1, returns: [{timer, from, to,
    /// setAtMs}]}`.
    #[allow(clippy::unused_self)] // S1: track A writes this body
    pub fn to_json(&self) -> String {
        todo!("S1: track A")
    }

    /// D24: `returns.json` read at `wall_ms`: slots older than
    /// [`SLOT_MAX_AGE_MS`] dropped; a file that can't be read is empty (the
    /// caller logs it). No due marks.
    #[allow(unused_variables)] // S1: track A writes this body
    pub fn from_json(text: &str, wall_ms: i64) -> Self {
        todo!("S1: track A")
    }
}
