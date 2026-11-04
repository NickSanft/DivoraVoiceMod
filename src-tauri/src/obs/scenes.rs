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
use serde_json::Value;

use super::MAX_UUID_CHARS;

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
    /// At most [`MAX_SLOTS`], oldest first, one per timer.
    list: Vec<Slot>,
    /// `(timer, offer's stamp)`, one per timer.
    due: Vec<(String, Instant)>,
    /// Marks [`take_due`](Self::take_due) handed out whose plan isn't
    /// committed yet, with their first stamp: a switch back that can't
    /// finish (§7.13: a Get that times out or gets 207, a session that ends
    /// first) is marked due again and keeps that stamp, so its grace still
    /// counts from `offer` (D23).
    taken: Vec<(String, Instant)>,
    /// `(timer, when its latest run started)`: a start that switches a
    /// scene, as [`started`](Self::started) recorded it, one per timer. A
    /// switch back stamped before it is an earlier run's and never runs.
    starts: Vec<(String, Instant)>,
}

impl Slots {
    /// §7.6's first table: `timer`'s `scene {to}` found `cur` on air;
    /// records the slot (before the Set, so a switch back works even if a
    /// timed-out Set lands later) and answers whether to send the Set.
    ///
    /// A slot still on the scene it set (a start again) keeps its `from`; a
    /// stale one (its run ended without a switch back) starts from `cur`.
    /// Scenes compare by uuid. The slot moves to the newest place; past
    /// [`MAX_SLOTS`] the oldest goes.
    pub fn after_scene(&mut self, timer: &str, cur: SceneRef, to: SceneRef, wall_ms: i64) -> bool {
        let switch = cur.uuid != to.uuid;
        let from = match self.list.iter().position(|s| s.timer == timer) {
            Some(at) => {
                let old = self.list.remove(at);
                if old.to.uuid == cur.uuid {
                    old.from
                } else {
                    cur
                }
            }
            None => cur,
        };
        self.list.push(Slot {
            timer: timer.to_string(),
            from,
            to,
            set_at_ms: wall_ms,
        });
        let excess = self.list.len().saturating_sub(MAX_SLOTS);
        self.list.drain(..excess);
        // A new run: an earlier switch back's stamp no longer applies.
        self.taken.retain(|(t, _)| t != timer);
        switch
    }

    /// §7.6's second table: what `timer`'s `sceneBack` does with `cur_uuid`
    /// on air. Changes nothing.
    ///
    /// Still on its `to`: back to its `from` (nothing when they're the same
    /// scene). Moved on: no switch, but a break stacked on this one (a slot
    /// whose `from` is this one's `to`; the newest, if several) inherits
    /// this one's `from`, so its own switch back goes all the way.
    pub fn plan_back(&self, timer: &str, cur_uuid: &str) -> BackPlan {
        let Some(slot) = self.list.iter().find(|s| s.timer == timer) else {
            return BackPlan::default();
        };
        if slot.to.uuid == cur_uuid {
            return BackPlan {
                to: (slot.from.uuid != slot.to.uuid).then(|| slot.from.clone()),
                inherit: None,
            };
        }
        let inherit = self
            .list
            .iter()
            .rev()
            .find(|s| s.timer != timer && s.from.uuid == slot.to.uuid)
            .map(|s| (s.timer.clone(), slot.from.clone()));
        BackPlan { to: None, inherit }
    }

    /// The plan done: `timer`'s slot dropped, an inheritance applied.
    pub fn commit_back(&mut self, timer: &str, plan: BackPlan) {
        self.list.retain(|s| s.timer != timer);
        self.taken.retain(|(t, _)| t != timer);
        if let Some((heir, from)) = plan.inherit {
            if let Some(s) = self.list.iter_mut().find(|s| s.timer == heir) {
                s.from = from;
            }
        }
    }

    /// D23: `timer`'s switch back is due, stamped `at`; a mark already there
    /// keeps its earlier stamp, and so does one taken and not committed
    /// (due again with its first stamp). A mark from before the timer's
    /// latest start is an earlier run's: it is ignored, so a break started
    /// again is never switched back by the run before it.
    pub fn mark_due(&mut self, timer: &str, at: Instant) {
        let first = self
            .taken
            .iter()
            .find(|(t, stamp)| t == timer && !self.superseded(timer, *stamp))
            .map_or(at, |&(_, first)| first.min(at));
        if self.superseded(timer, first) {
            return;
        }
        match self.due.iter_mut().find(|(t, _)| t == timer) {
            Some((_, stamp)) => *stamp = (*stamp).min(first),
            None => self.due.push((timer.to_string(), first)),
        }
    }

    /// D23: a mark [`take_due`](Self::take_due) handed out couldn't finish
    /// (a Get that timed out or got 207, a session that ended first): due
    /// again with its first stamp. Nothing when the take is gone — its
    /// grace ran out, OBS was switched off, the timer's next `scene` ran —
    /// or belongs to a run before the timer's latest start (then the take
    /// goes too).
    pub fn remark(&mut self, timer: &str) {
        let Some(first) = self
            .taken
            .iter()
            .find(|(t, _)| t == timer)
            .map(|&(_, first)| first)
        else {
            return;
        };
        if self.superseded(timer, first) {
            self.release(timer);
            return;
        }
        self.mark_due(timer, first);
    }

    /// D23: the timer's take is let go without a commit (a driver that went
    /// stale before it could commit): it no longer counts as held back.
    /// The slot stays.
    pub fn release(&mut self, timer: &str) {
        self.taken.retain(|(t, _)| t != timer);
    }

    /// D23: `timer` started a new run at `at` with a `scene` op: its switch
    /// backs from earlier runs — due, or in flight and failing later — no
    /// longer apply (a mark taken and in flight keeps its stamp, so its
    /// failure is known for an earlier run's). Answers whether a due mark
    /// was dropped.
    pub fn started(&mut self, timer: &str, at: Instant) -> bool {
        let before = self.due.len();
        self.due.retain(|(t, stamp)| t != timer || *stamp >= at);
        match self.starts.iter_mut().find(|(t, _)| t == timer) {
            Some((_, s)) => *s = (*s).max(at),
            None => self.starts.push((timer.to_string(), at)),
        }
        self.due.len() != before
    }

    /// When `timer`'s latest run started, if [`started`](Self::started)
    /// recorded it.
    fn start_of(&self, timer: &str) -> Option<Instant> {
        self.starts
            .iter()
            .find(|(t, _)| t == timer)
            .map(|&(_, at)| at)
    }

    /// Whether `stamp` is from before `timer`'s latest start.
    fn superseded(&self, timer: &str, stamp: Instant) -> bool {
        self.start_of(timer).is_some_and(|s| stamp < s)
    }

    /// D23: the due mark to run next (taken out; one whose timer has no slot
    /// does nothing), and the timers whose marks are older than `grace` at
    /// `now` (dropped).
    ///
    /// Next is the oldest stamp still within its grace (`now − stamp ≤
    /// grace`). An expired mark is listed only when its timer has a slot:
    /// one without had nothing to switch back. The slot of an expired mark
    /// stays (OBS stays where it is; the timer's next `scene` replaces it).
    pub fn take_due(&mut self, now: Instant, grace: Duration) -> (Option<String>, Vec<String>) {
        let expired = self.expire_due(now, grace);
        let next = loop {
            let Some(at) = self
                .due
                .iter()
                .enumerate()
                .min_by_key(|(_, (_, stamp))| *stamp)
                .map(|(at, _)| at)
            else {
                break None;
            };
            let (timer, stamp) = self.due.remove(at);
            if self.has_slot(&timer) {
                self.taken.retain(|(t, _)| *t != timer);
                self.taken.push((timer.clone(), stamp));
                break Some(timer);
            }
        };
        (next, expired)
    }

    /// D23: drops the marks older than `grace` at `now` and answers the
    /// timers among them that have a slot (each a `lastSkip {sceneBack,
    /// notConnected}` when the config is enabled), taking none to run: what
    /// a link that isn't connected does when [`next_expiry`](Self::next_expiry)
    /// comes.
    pub fn expire_due(&mut self, now: Instant, grace: Duration) -> Vec<String> {
        let within = |stamp: &Instant| now.saturating_duration_since(*stamp) <= grace;
        let mut expired = Vec::new();
        let slots = &self.list;
        self.due.retain(|(timer, stamp)| {
            if within(stamp) {
                return true;
            }
            if slots.iter().any(|s| s.timer == *timer) {
                expired.push(timer.clone());
            }
            false
        });
        self.taken.retain(|(_, stamp)| within(stamp));
        // A start older than the grace can't outrank a live mark: every
        // mark it would ignore is past the grace too.
        self.starts.retain(|(_, at)| within(at));
        expired
    }

    /// D23: when the earliest due mark's `grace` runs out: the first moment
    /// [`take_due`](Self::take_due) drops it (a mark is still live at
    /// exactly `grace`). A take not yet committed counts too, so one that
    /// is never finished still goes at its grace.
    pub fn next_expiry(&self, grace: Duration) -> Option<Instant> {
        let past = grace.saturating_add(Duration::from_nanos(1));
        let earliest = self
            .due
            .iter()
            .chain(&self.taken)
            .map(|(_, stamp)| *stamp)
            .min()?;
        earliest.checked_add(past)
    }

    /// D4: `obs_configure {enabled:false}` drops every due mark (the slots
    /// stay).
    pub fn drop_due(&mut self) {
        self.due.clear();
        self.taken.clear();
    }

    /// D23: the status's `heldBack`: the timers with a slot whose switch
    /// back is due, or taken and not yet committed (so a switch back OBS
    /// keeps answering 207 stays held while it is tried, rather than
    /// dropping to 0 for each try), each once. A take from before the
    /// timer's latest start doesn't count: it never switches back again.
    pub fn due_count(&self) -> u32 {
        let due = self.due.iter().map(|(t, _)| t);
        let taken = self
            .taken
            .iter()
            .filter(|(t, stamp)| {
                !self.superseded(t, *stamp) && !self.due.iter().any(|(d, _)| d == t)
            })
            .map(|(t, _)| t);
        let n = due.chain(taken).filter(|t| self.has_slot(t)).count();
        u32::try_from(n).unwrap_or(u32::MAX)
    }

    /// D24: `returns.json`: `{version: 1, returns: [{timer, from, to,
    /// setAtMs}]}`, oldest first.
    pub fn to_json(&self) -> String {
        #[derive(Serialize)]
        struct File<'a> {
            version: u32,
            returns: &'a [Slot],
        }
        // Strings and an i64: serializing can't fail.
        serde_json::to_string(&File {
            version: RETURNS_VERSION,
            returns: &self.list,
        })
        .unwrap_or_default()
    }

    /// D24: `returns.json` read at `wall_ms`: slots older than
    /// [`SLOT_MAX_AGE_MS`] dropped; a file that can't be read is empty (the
    /// caller logs it). No due marks.
    ///
    /// Another `version`, or no `returns` list, reads as empty. Inside the
    /// list a slot that can't be read (or has an empty timer id or an
    /// unusable uuid) is skipped, not the file; a timer's last slot wins;
    /// past [`MAX_SLOTS`] the oldest go.
    pub fn from_json(text: &str, wall_ms: i64) -> Self {
        #[derive(Deserialize)]
        struct File {
            version: u64,
            returns: Vec<Value>,
        }
        let Ok(file) = serde_json::from_str::<File>(text) else {
            return Self::default();
        };
        if file.version != u64::from(RETURNS_VERSION) {
            return Self::default();
        }
        let mut slots: Vec<Slot> = Vec::new();
        for v in file.returns {
            let Ok(slot) = serde_json::from_value::<Slot>(v) else {
                continue;
            };
            let usable = !slot.timer.is_empty()
                && usable_uuid(&slot.from.uuid)
                && usable_uuid(&slot.to.uuid);
            if !usable || wall_ms.saturating_sub(slot.set_at_ms) > SLOT_MAX_AGE_MS {
                continue;
            }
            slots.retain(|s| s.timer != slot.timer);
            slots.push(slot);
        }
        let excess = slots.len().saturating_sub(MAX_SLOTS);
        slots.drain(..excess);
        Self {
            list: slots,
            ..Self::default()
        }
    }

    /// Whether `timer` has a slot.
    fn has_slot(&self, timer: &str) -> bool {
        self.list.iter().any(|s| s.timer == timer)
    }
}

/// D24: the only `returns.json` version this build reads and writes.
const RETURNS_VERSION: u32 = 1;

/// A scene uuid a slot can switch to: 1 to [`MAX_UUID_CHARS`] characters,
/// as an action's `sceneUuid` is read (D21).
fn usable_uuid(uuid: &str) -> bool {
    !uuid.is_empty() && uuid.chars().count() <= MAX_UUID_CHARS
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic
)]
mod tests {
    use super::{BackPlan, SceneRef, Slot, Slots, MAX_SLOTS, SLOT_MAX_AGE_MS};
    use serde_json::{json, Value};
    use std::time::{Duration, Instant};

    /// A scene whose uuid is its name's.
    fn scene(name: &str) -> SceneRef {
        SceneRef {
            uuid: format!("uuid-{name}"),
            name: name.to_string(),
        }
    }

    /// `timer`'s slot as `(from, to)` names.
    fn slot_of(s: &Slots, timer: &str) -> Option<(String, String)> {
        s.list
            .iter()
            .find(|x| x.timer == timer)
            .map(|x| (x.from.name.clone(), x.to.name.clone()))
    }

    #[allow(clippy::unnecessary_wraps)] // compared with `slot_of`'s answer
    fn pair(from: &str, to: &str) -> Option<(String, String)> {
        Some((from.to_string(), to.to_string()))
    }

    /// Slots holding `timer`'s slot `from` → `to`.
    fn with(rows: &[(&str, &str, &str)]) -> Slots {
        let mut s = Slots::default();
        for (timer, from, to) in rows {
            s.list.push(Slot {
                timer: (*timer).to_string(),
                from: scene(from),
                to: scene(to),
                set_at_ms: 1_000,
            });
        }
        s
    }

    /// §7.6's first table: a slot is recorded before the Set (so a back
    /// works if a timed-out Set lands later); a start again keeps `from`; a
    /// stale slot (its run ended without a back) starts over from `cur`.
    #[test]
    fn after_scene_table() {
        // No slot, cur ≠ to: from cur, switch.
        let mut s = Slots::default();
        assert!(s.after_scene("A", scene("Game"), scene("BRB"), 10));
        assert_eq!(slot_of(&s, "A"), pair("Game", "BRB"));
        assert_eq!(s.list[0].set_at_ms, 10);
        // No slot, cur = to: recorded so its back does nothing; no switch.
        let mut s = Slots::default();
        assert!(!s.after_scene("A", scene("BRB"), scene("BRB"), 10));
        assert_eq!(slot_of(&s, "A"), pair("BRB", "BRB"));
        assert_eq!(s.plan_back("A", "uuid-BRB"), BackPlan::default());
        // Still on its own `to` (a start again): `from` kept.
        let mut s = with(&[("A", "Game", "BRB")]);
        assert!(!s.after_scene("A", scene("BRB"), scene("BRB"), 20));
        assert_eq!(slot_of(&s, "A"), pair("Game", "BRB"));
        assert_eq!(s.list[0].set_at_ms, 20);
        let mut s = with(&[("A", "Game", "BRB")]);
        assert!(s.after_scene("A", scene("BRB"), scene("Chat"), 20));
        assert_eq!(slot_of(&s, "A"), pair("Game", "Chat"));
        // Stale (OBS moved on): from `cur`, not the old `from`.
        let mut s = with(&[("A", "Game", "BRB")]);
        assert!(s.after_scene("A", scene("Chat"), scene("BRB"), 30));
        assert_eq!(slot_of(&s, "A"), pair("Chat", "BRB"));
        let mut s = with(&[("A", "Game", "BRB")]);
        assert!(!s.after_scene("A", scene("BRB2"), scene("BRB2"), 30));
        assert_eq!(slot_of(&s, "A"), pair("BRB2", "BRB2"));
        // Scenes compare by uuid: a renamed scene is the same scene.
        let mut s = with(&[("A", "Game", "BRB")]);
        let renamed = SceneRef {
            uuid: "uuid-BRB".into(),
            name: "Away".into(),
        };
        assert!(!s.after_scene("A", renamed.clone(), renamed, 40));
        assert_eq!(slot_of(&s, "A"), pair("Game", "Away"));
        // Other timers' slots are untouched.
        let mut s = with(&[("A", "Game", "BRB"), ("B", "BRB", "Chat")]);
        s.after_scene("A", scene("Chat"), scene("BRB"), 50);
        assert_eq!(slot_of(&s, "B"), pair("BRB", "Chat"));

        // At most 64: the oldest goes; a timer set again is the newest.
        let mut s = Slots::default();
        for i in 0..=MAX_SLOTS {
            s.after_scene(&format!("t{i}"), scene("Game"), scene("BRB"), 0);
        }
        assert_eq!(s.list.len(), MAX_SLOTS);
        assert_eq!(slot_of(&s, "t0"), None);
        s.after_scene("t1", scene("Game"), scene("BRB"), 0);
        s.after_scene("new", scene("Game"), scene("BRB"), 0);
        assert!(slot_of(&s, "t1").is_some());
        assert_eq!(slot_of(&s, "t2"), None);
    }

    /// §7.6's second table: back only while OBS is still on the scene the
    /// timer set; a stacked break inherits a `from` when you moved on.
    /// Planning changes nothing; `commit_back` does.
    #[test]
    fn plan_back_table() {
        let back = |to: Option<&str>, inherit: Option<(&str, &str)>| BackPlan {
            to: to.map(scene),
            inherit: inherit.map(|(t, s)| (t.to_string(), scene(s))),
        };
        for (name, slots, timer, cur, want) in [
            ("no slot", with(&[]), "A", "BRB", back(None, None)),
            (
                "still ours",
                with(&[("A", "Game", "BRB")]),
                "A",
                "BRB",
                back(Some("Game"), None),
            ),
            (
                "still ours, nothing to go back to",
                with(&[("A", "BRB", "BRB")]),
                "A",
                "BRB",
                back(None, None),
            ),
            (
                "moved on, a break stacked on it",
                with(&[("A", "Game", "BRB"), ("B", "BRB", "Chat")]),
                "A",
                "Chat",
                back(None, Some(("B", "Game"))),
            ),
            (
                "moved on, none stacked",
                with(&[("A", "Game", "BRB"), ("B", "Game", "Chat")]),
                "A",
                "Chat",
                back(None, None),
            ),
            (
                "moved on: two stacked, the newest inherits",
                with(&[
                    ("A", "Game", "BRB"),
                    ("B", "BRB", "Chat"),
                    ("C", "BRB", "Ad"),
                ]),
                "A",
                "Ad",
                back(None, Some(("C", "Game"))),
            ),
            (
                "another timer's slot isn't this one's",
                with(&[("B", "Game", "BRB")]),
                "A",
                "BRB",
                back(None, None),
            ),
        ] {
            let before = slots.to_json();
            assert_eq!(
                slots.plan_back(timer, &format!("uuid-{cur}")),
                want,
                "{name}"
            );
            assert_eq!(slots.to_json(), before, "{name}: planning changed nothing");
        }

        // Stacked breaks, end to end: A (Game → BRB), then B (BRB → Chat).
        let mut s = Slots::default();
        assert!(s.after_scene("A", scene("Game"), scene("BRB"), 0));
        assert!(s.after_scene("B", scene("BRB"), scene("Chat"), 0));
        // A ends first, on Chat: no switch; B now goes back to Game.
        let plan = s.plan_back("A", "uuid-Chat");
        assert_eq!(plan, back(None, Some(("B", "Game"))));
        assert_eq!(slot_of(&s, "A"), pair("Game", "BRB"));
        s.commit_back("A", plan);
        assert_eq!(slot_of(&s, "A"), None);
        assert_eq!(slot_of(&s, "B"), pair("Game", "Chat"));
        let plan = s.plan_back("B", "uuid-Chat");
        assert_eq!(plan, back(Some("Game"), None));
        s.commit_back("B", plan);
        assert_eq!(s.list, Vec::<Slot>::new());

        // A plan that can't finish leaves the slot for the next try.
        let s = with(&[("A", "Game", "BRB")]);
        let first = s.plan_back("A", "uuid-BRB");
        assert_eq!(s.plan_back("A", "uuid-BRB"), first);
        // An heir gone by the commit: the slot still goes.
        let mut s = with(&[("A", "Game", "BRB")]);
        s.commit_back("A", back(None, Some(("gone", "Game"))));
        assert_eq!(s.list, Vec::<Slot>::new());
    }

    /// D23: due marks. The earlier stamp wins; a mark runs while `now −
    /// stamp ≤ grace` and is dropped past it, counted from `offer`'s stamp
    /// however often it is taken and made due again; only marks with a
    /// slot count, run or report; the oldest runs first; `drop_due` drops
    /// them all and keeps the slots.
    #[test]
    fn due_table() {
        let grace = Duration::from_secs(120);
        let s_ = Duration::from_secs;
        let t0 = Instant::now();
        let mut s = with(&[("A", "Game", "BRB"), ("B", "Game", "Chat")]);
        assert_eq!((s.due_count(), s.next_expiry(grace)), (0, None));
        assert_eq!(s.take_due(t0, grace), (None, Vec::new()));

        // The earlier stamp is kept, either way round.
        s.mark_due("A", t0 + s_(5));
        s.mark_due("A", t0);
        s.mark_due("A", t0 + s_(9));
        assert_eq!(s.due, vec![("A".to_string(), t0)]);
        assert_eq!(s.due_count(), 1);
        let expiry = s.next_expiry(grace).unwrap();
        assert!(expiry > t0 + grace && expiry - (t0 + grace) <= Duration::from_micros(1));

        // Live at exactly the grace; dropped just past it, its slot kept.
        assert_eq!(
            s.clone().take_due(t0 + grace, grace),
            (Some("A".to_string()), Vec::new())
        );
        let mut late = s.clone();
        assert_eq!(late.take_due(expiry, grace), (None, vec!["A".to_string()]));
        assert_eq!(late.due_count(), 0);
        assert_eq!(slot_of(&late, "A"), pair("Game", "BRB"));

        // Taken at 100 s, its Get timed out, made due again at 110 s: the
        // grace still counts from t0. While taken it is still held back
        // (a switch back OBS answers 207 to stays 1 while it is tried).
        let (next, gone) = s.take_due(t0 + s_(100), grace);
        assert_eq!((next.as_deref(), gone.len()), (Some("A"), 0));
        assert_eq!(s.due_count(), 1);
        assert_eq!(s.next_expiry(grace), Some(expiry), "a take expires too");
        s.mark_due("A", t0 + s_(110));
        assert_eq!(s.due, vec![("A".to_string(), t0)]);
        assert_eq!(s.due_count(), 1, "due and taken: one timer");
        assert_eq!(s.next_expiry(grace), Some(expiry));
        assert_eq!(
            s.take_due(t0 + s_(121), grace),
            (None, vec!["A".to_string()])
        );
        // Once expired, a later mark is a new switch back with its own stamp.
        s.mark_due("A", t0 + s_(130));
        assert_eq!(s.due, vec![("A".to_string(), t0 + s_(130))]);

        // A committed plan forgets the stamp: the next mark is new.
        let mut s = with(&[("A", "Game", "BRB")]);
        s.mark_due("A", t0);
        assert_eq!(s.take_due(t0 + s_(1), grace).0.as_deref(), Some("A"));
        let plan = s.plan_back("A", "uuid-BRB");
        s.commit_back("A", plan);
        s.after_scene("A", scene("Game"), scene("BRB"), 0);
        s.mark_due("A", t0 + s_(200));
        assert_eq!(s.due, vec![("A".to_string(), t0 + s_(200))]);
        // So does the timer's next `scene` (a new run).
        let mut s = with(&[("A", "Game", "BRB")]);
        s.mark_due("A", t0);
        s.take_due(t0, grace);
        s.after_scene("A", scene("Game"), scene("BRB"), 0);
        s.mark_due("A", t0 + s_(50));
        assert_eq!(s.due, vec![("A".to_string(), t0 + s_(50))]);

        // The oldest first; a mark without a slot does nothing (its `scene`
        // never ran), isn't held back and isn't reported when it expires.
        let mut s = with(&[("A", "Game", "BRB"), ("B", "Game", "Chat")]);
        s.mark_due("B", t0 + s_(2));
        s.mark_due("none", t0);
        s.mark_due("A", t0 + s_(1));
        assert_eq!(s.due_count(), 2);
        assert_eq!(s.take_due(t0 + s_(3), grace).0.as_deref(), Some("A"));
        assert_eq!(s.due_count(), 2, "A taken, B due");
        let plan = s.plan_back("A", "uuid-BRB");
        s.commit_back("A", plan);
        assert_eq!(s.due_count(), 1, "A committed");
        assert_eq!(s.take_due(t0 + s_(3), grace).0.as_deref(), Some("B"));
        assert_eq!(s.due_count(), 1, "B taken");
        assert_eq!(s.take_due(t0 + s_(3), grace), (None, Vec::new()));
        assert_eq!(s.due, Vec::<(String, Instant)>::new());
        // A take let go (a stale driver's) is no longer held back; the slot
        // stays.
        s.release("B");
        assert_eq!((s.due_count(), s.next_expiry(grace)), (0, None));
        assert_eq!(slot_of(&s, "B"), pair("Game", "Chat"));
        let mut s = with(&[("A", "Game", "BRB")]);
        s.mark_due("none", t0);
        s.mark_due("A", t0 + s_(10));
        assert_eq!(s.due_count(), 1);
        assert_eq!(
            s.take_due(t0 + s_(125), grace),
            (Some("A".to_string()), Vec::new())
        );

        // Expiring alone (a link that isn't connected) runs nothing.
        let mut s = with(&[("A", "Game", "BRB"), ("B", "Game", "Chat")]);
        s.mark_due("A", t0);
        s.mark_due("B", t0 + s_(60));
        assert_eq!(s.expire_due(t0 + s_(150), grace), vec!["A".to_string()]);
        assert_eq!(s.due, vec![("B".to_string(), t0 + s_(60))]);
        assert_eq!(s.expire_due(t0 + s_(150), grace), Vec::<String>::new());

        // A zero grace (the kill switch): only a mark taken at its own
        // moment runs.
        let zero = Duration::ZERO;
        let mut s = with(&[("A", "Game", "BRB")]);
        s.mark_due("A", t0);
        assert_eq!(s.clone().take_due(t0, zero).0.as_deref(), Some("A"));
        assert_eq!(
            s.take_due(t0 + Duration::from_millis(1), zero),
            (None, vec!["A".to_string()])
        );

        // `obs_configure {enabled:false}`: every mark goes, slots stay, and
        // a taken mark's stamp is forgotten.
        let mut s = with(&[("A", "Game", "BRB"), ("B", "Game", "Chat")]);
        s.mark_due("A", t0);
        s.mark_due("B", t0);
        s.take_due(t0, grace);
        s.drop_due();
        assert_eq!((s.due_count(), s.next_expiry(grace)), (0, None));
        assert_eq!(s.list.len(), 2);
        s.mark_due("A", t0 + s_(300));
        assert_eq!(s.due, vec![("A".to_string(), t0 + s_(300))]);
    }

    /// D23: a timer started again (its start's `scene`) drops the switch
    /// backs of its earlier runs: one due, one taken and failing after the
    /// start, one still queued and failing after it; its new run's own
    /// mark, made after the start, still counts. A mark taken that can't
    /// finish is due again with its first stamp, or nothing once the take
    /// is gone.
    #[test]
    fn a_new_run_drops_earlier_switch_backs() {
        let grace = Duration::from_secs(120);
        let s_ = Duration::from_secs;
        let t0 = Instant::now();

        // Due while OBS was down, then Start again: dropped at once.
        let mut s = with(&[("A", "Game", "BRB"), ("B", "Game", "Chat")]);
        s.mark_due("A", t0);
        s.mark_due("B", t0);
        assert!(s.started("A", t0 + s_(5)));
        assert_eq!(s.due, vec![("B".to_string(), t0)], "B's run goes on");
        assert_eq!(s.due_count(), 1);
        assert!(!s.started("A", t0 + s_(6)), "nothing left to drop");
        // The run before it, failing later (a queued op's stamp): ignored.
        s.mark_due("A", t0 + s_(1));
        assert_eq!(s.take_due(t0 + s_(7), grace).0.as_deref(), Some("B"));
        assert_eq!(s.take_due(t0 + s_(7), grace), (None, Vec::new()));
        // The new run's own end: due.
        s.mark_due("A", t0 + s_(8));
        assert_eq!(s.due, vec![("A".to_string(), t0 + s_(8))]);

        // Taken and in flight when Start comes: its failure isn't due
        // again, and the take goes, so it isn't held back.
        let mut s = with(&[("A", "Game", "BRB")]);
        s.mark_due("A", t0);
        assert_eq!(s.take_due(t0 + s_(1), grace).0.as_deref(), Some("A"));
        s.started("A", t0 + s_(2));
        assert_eq!(s.due_count(), 0, "an earlier run's take isn't held back");
        s.remark("A");
        assert_eq!((s.due_count(), s.next_expiry(grace)), (0, None));
        // The new run's end, after its scene was skipped (the slot is the
        // old one): due with its own stamp.
        s.mark_due("A", t0 + s_(3));
        assert_eq!(s.due, vec![("A".to_string(), t0 + s_(3))]);

        // The new run ends while the earlier run's take is still in flight:
        // its mark is due with its own stamp (the earlier take's isn't its
        // first), and the earlier take failing after leaves it as it is.
        let mut s = with(&[("A", "Game", "BRB")]);
        s.mark_due("A", t0);
        assert_eq!(s.take_due(t0 + s_(1), grace).0.as_deref(), Some("A"));
        s.started("A", t0 + s_(2));
        s.mark_due("A", t0 + s_(3));
        assert_eq!(s.due, vec![("A".to_string(), t0 + s_(3))]);
        assert_eq!(s.due_count(), 1, "the new run's");
        s.remark("A");
        assert_eq!(s.due, vec![("A".to_string(), t0 + s_(3))]);
        assert_eq!(
            s.next_expiry(grace),
            (t0 + s_(3)).checked_add(grace + Duration::from_nanos(1))
        );

        // A take that can't finish: due again with its first stamp; once
        // the take is gone (expired, off, committed, the next scene), the
        // failure makes nothing.
        let mut s = with(&[("A", "Game", "BRB")]);
        s.mark_due("A", t0);
        s.take_due(t0 + s_(1), grace);
        s.remark("A");
        assert_eq!(s.due, vec![("A".to_string(), t0)]);
        s.take_due(t0 + s_(2), grace);
        s.drop_due();
        s.remark("A");
        assert_eq!(s.due, Vec::<(String, Instant)>::new());
        s.remark("never-taken");
        assert_eq!(s.due, Vec::<(String, Instant)>::new());

        // A start older than the grace is forgotten with the marks it
        // could outrank.
        let mut s = with(&[("A", "Game", "BRB")]);
        s.started("A", t0);
        s.expire_due(t0 + grace + s_(1), grace);
        assert_eq!(s.starts, Vec::<(String, Instant)>::new());
    }

    /// D24: `returns.json` round-trips; on reading, slots older than 24 h,
    /// unreadable entries and unusable ones are dropped, a timer's last
    /// slot wins, at most 64 are kept, no due mark comes back, and a file
    /// that can't be read is empty.
    #[test]
    fn returns_json_round_trip_and_prune() {
        let now = 1_793_217_840_000_i64;
        let mut s = Slots::default();
        s.after_scene("timer-1", scene("Game"), scene("BRB"), now - 5_000);
        s.after_scene("timer-2", scene("BRB"), scene("Chat"), now - 1_000);
        s.mark_due("timer-1", Instant::now());
        let text = s.to_json();
        let v: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(
            v,
            json!({"version": 1, "returns": [
                {"timer": "timer-1", "from": {"uuid": "uuid-Game", "name": "Game"},
                    "to": {"uuid": "uuid-BRB", "name": "BRB"}, "setAtMs": now - 5_000},
                {"timer": "timer-2", "from": {"uuid": "uuid-BRB", "name": "BRB"},
                    "to": {"uuid": "uuid-Chat", "name": "Chat"}, "setAtMs": now - 1_000},
            ]})
        );
        let back = Slots::from_json(&text, now);
        assert_eq!(back.to_json(), text);
        assert_eq!(back.due, Vec::<(String, Instant)>::new());
        assert_eq!(
            back.plan_back("timer-1", "uuid-BRB").to,
            Some(scene("Game"))
        );
        assert_eq!(Slots::default().to_json(), r#"{"version":1,"returns":[]}"#);

        // The 24 h prune: exactly 24 h is kept, a ms older isn't; a slot
        // from a clock that went back is kept.
        let entry = |timer: &str, set_at: i64| {
            json!({"timer": timer, "from": {"uuid": "u-game", "name": "Game"},
                "to": {"uuid": "u-brb", "name": "BRB"}, "setAtMs": set_at})
        };
        let file = json!({"version": 1, "returns": [
            entry("day", now - SLOT_MAX_AGE_MS),
            entry("older", now - SLOT_MAX_AGE_MS - 1),
            entry("ahead", now + 60_000),
        ]});
        let read = Slots::from_json(&file.to_string(), now);
        let timers: Vec<&str> = read.list.iter().map(|s| s.timer.as_str()).collect();
        assert_eq!(timers, ["day", "ahead"]);

        // One bad entry costs only itself; the last slot per timer wins;
        // unknown keys are ignored.
        let long = "u".repeat(129);
        let file = json!({"version": 1, "later": true, "returns": [
            entry("A", now),
            {"timer": 5, "from": {"uuid": "u", "name": "n"}, "to": {"uuid": "u", "name": "n"},
                "setAtMs": now},
            {"timer": "no-to", "from": {"uuid": "u", "name": "n"}, "setAtMs": now},
            {"timer": "", "from": {"uuid": "u", "name": "n"}, "to": {"uuid": "u", "name": "n"},
                "setAtMs": now},
            {"timer": "empty-uuid", "from": {"uuid": "", "name": "n"},
                "to": {"uuid": "u", "name": "n"}, "setAtMs": now},
            {"timer": "long-uuid", "from": {"uuid": "u", "name": "n"},
                "to": {"uuid": long, "name": "n"}, "setAtMs": now},
            {"timer": "B", "from": {"uuid": "u-game", "name": "Game"},
                "to": {"uuid": "u-brb", "name": "BRB"}, "setAtMs": now, "extra": 1},
            entry("A", now - 1),
            "not a slot",
        ]});
        let read = Slots::from_json(&file.to_string(), now);
        let timers: Vec<(&str, i64)> = read
            .list
            .iter()
            .map(|s| (s.timer.as_str(), s.set_at_ms))
            .collect();
        assert_eq!(timers, [("B", now), ("A", now - 1)]);

        // Past 64, the newest 64.
        let many: Vec<Value> = (0..70).map(|i| entry(&format!("t{i}"), now)).collect();
        let read = Slots::from_json(&json!({"version": 1, "returns": many}).to_string(), now);
        assert_eq!(read.list.len(), MAX_SLOTS);
        assert_eq!(read.list[0].timer, "t6");

        // Files that can't be read are empty.
        for text in [
            "",
            "{",
            "[]",
            "null",
            r#"{"version":1}"#,
            r#"{"returns":[]}"#,
            r#"{"version":2,"returns":[]}"#,
            r#"{"version":"1","returns":[]}"#,
            r#"{"version":1,"returns":{}}"#,
        ] {
            let read = Slots::from_json(text, now);
            assert!(read.list.is_empty() && read.due.is_empty(), "{text}");
        }
        let v2 = json!({"version": 2, "returns": [entry("A", now)]});
        assert_eq!(
            Slots::from_json(&v2.to_string(), now).list,
            Vec::<Slot>::new()
        );
    }
}
