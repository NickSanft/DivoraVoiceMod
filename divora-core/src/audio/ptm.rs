//! v1.58.0: push-to-modulate — the state's encoding and the switch.
//!
//! The state is one engine-wide word, `seq << 16 | mode << 8 | held`, kept in
//! [`EngineState`](super::state::EngineState). `held` is a bit per source (bit
//! 0 the global hotkey, 1 the in-app key, 2 the Mixer's button, 3 MIDI; 4-7
//! reserved), so one source letting go never lets go of another. Every change
//! bumps `seq`, so the frontend can order command answers against level ticks,
//! as it does the mute's.
//!
//! The switch is [`PtmStage`], inside the voice stage after the chain and
//! before loudness. The chain always runs; the stage crossfades its output
//! with the dry voice, delayed by the chain's reported latency so the two
//! carry the same moment of speech ([`DryLine`]). [`PtmGate`] is the
//! crossfade: an edge toward the effects starts at once, an edge away from
//! them first waits out that latency, so the effects cover everything said
//! while they were meant to be on.

use serde::{Deserialize, Serialize};

use super::engine::MAX_FRAMES_PER_CALLBACK;
use super::mute::mute_ramp_len;

/// What the push-to-modulate key does. `Always`: the effects always apply
/// (push-to-modulate is off). `Apply`: holding it applies them. `Bypass`:
/// holding it takes them off.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PtmMode {
    #[default]
    Always,
    Apply,
    Bypass,
}

/// Who holds it. The discriminant is the bit in the word. `Hotkey` is the
/// backend's alone: only the global shortcut handler sets it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum PtmSource {
    Hotkey = 0,
    Key = 1,
    Button = 2,
    Midi = 3,
}

impl PtmSource {
    /// Its bit in the word's `held` byte.
    const fn bit(self) -> u8 {
        1 << self as u8
    }
}

/// The sources the page may set (`set_ptm_held`). No `Hotkey`, so the wire
/// refuses it: only the backend holds that one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PtmPageSource {
    Key,
    Button,
    Midi,
}

impl From<PtmPageSource> for PtmSource {
    fn from(source: PtmPageSource) -> Self {
        match source {
            PtmPageSource::Key => Self::Key,
            PtmPageSource::Button => Self::Button,
            PtmPageSource::Midi => Self::Midi,
        }
    }
}

/// Push-to-modulate, as the frontend sees it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PtmState {
    pub mode: PtmMode,
    /// Some source holds it.
    pub held: bool,
    /// What the callback is switching to from this state on: the sound
    /// crosses at once toward the effects, and after the dry's delay away
    /// from them.
    pub bypassed: bool,
    /// Changes since launch. Of two states, the larger `seq` is newer.
    pub seq: u64,
}

/// One setter call: the state before and after it, from one atomic op.
/// `before == after` means the call changed nothing. Not on the wire: the
/// commands answer `after`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PtmChange {
    pub before: PtmState,
    pub after: PtmState,
}

/// What a setter asks for. `SetMode`: the Mixer card's set, which lets go of
/// every hold. `Restore`: the page's boot restore, which lets go of the
/// page's holds and keeps the hotkey's (D14). `SetHeld`: one source, by level.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PtmOp {
    SetMode(PtmMode),
    Restore(PtmMode),
    SetHeld(PtmSource, bool),
}

/// The bits the page owns: key, button, MIDI.
pub(crate) const PAGE_BITS: u8 = 0b1110;

/// Kill switch (K1): `false` delays the dry by 0, and so waits 0 (D8b). The
/// dry is then early by the chain's latency, so a switch skips or repeats
/// that much, and a take is off by it while the effects are off.
pub(crate) const ALIGN_DRY: bool = true;

/// The largest `seq` the word holds, 2^48 − 1. Never reached.
const SEQ_MAX: u64 = u64::MAX >> 16;

/// The most samples a [`DryLine`] ring holds: at least a second at every
/// rate up to 1 048 575 Hz, so a rate no device offers can't ask for gigabytes.
const MAX_RING: usize = 1 << 20;

/// A mode's byte in the word.
const fn mode_byte(mode: PtmMode) -> u8 {
    match mode {
        PtmMode::Always => 0,
        PtmMode::Apply => 1,
        PtmMode::Bypass => 2,
    }
}

/// The mode a byte names; any value but 1 and 2 reads as `Always`.
const fn mode_of(byte: u8) -> PtmMode {
    match byte {
        1 => PtmMode::Apply,
        2 => PtmMode::Bypass,
        _ => PtmMode::Always,
    }
}

/// `seq << 16 | mode << 8 | held` → its parts. `held` is any bit set.
pub(crate) fn unpack(word: u64) -> PtmState {
    let [held, mode, ..] = word.to_le_bytes();
    PtmState {
        mode: mode_of(mode),
        held: held != 0,
        bypassed: is_bypassed(word),
        seq: word >> 16,
    }
}

/// The parts → `seq << 16 | mode << 8 | held`. `seq` saturates at 2^48 − 1.
pub(crate) fn pack(mode: PtmMode, held: u8, seq: u64) -> u64 {
    (seq.min(SEQ_MAX) << 16) | (u64::from(mode_byte(mode)) << 8) | u64::from(held)
}

/// The callback's rule: `Always` → applied; `Apply` → bypassed while no bit
/// is held; `Bypass` → bypassed while a bit is held. Any other mode byte
/// reads as `Always`.
pub(crate) fn is_bypassed(word: u64) -> bool {
    let [held, mode, ..] = word.to_le_bytes();
    match mode_of(mode) {
        PtmMode::Always => false,
        PtmMode::Apply => held == 0,
        PtmMode::Bypass => held != 0,
    }
}

/// The word after `op`, or `None` when nothing changes. `SetMode`: the mode
/// set, every bit cleared; `None` only if the mode is the same and nothing
/// was held. `Restore`: the mode set, [`PAGE_BITS`] cleared, the hotkey's bit
/// kept; `None` if the mode is the same and no page bit was held. `SetHeld`:
/// one bit; `None` if it already has that value. A change bumps `seq` by one,
/// saturating at 2^48 − 1 (never reached).
pub(crate) fn ptm_transition(word: u64, op: PtmOp) -> Option<u64> {
    let [held, mode, ..] = word.to_le_bytes();
    let (next_mode, next_held) = match op {
        PtmOp::SetMode(next) => (next, 0),
        PtmOp::Restore(next) => (next, held & !PAGE_BITS),
        PtmOp::SetHeld(source, on) => {
            let next = if on {
                held | source.bit()
            } else {
                held & !source.bit()
            };
            (mode_of(mode), next)
        }
    };
    (mode_byte(next_mode) != mode || next_held != held)
        .then(|| pack(next_mode, next_held, (word >> 16).saturating_add(1)))
}

/// The crossfade length at `rate`: twice the mute's, so a power of two in
/// 128..=4096. 512 at 44.1 and 48 kHz (10.7-11.6 ms); at least 10 ms and
/// under 20 ms for every rate in `12_800..=409_600`. Twice the mute's 5 ms
/// because this splices two timbres rather than fading to silence.
#[must_use]
pub(crate) fn ptm_fade_len(rate: u32) -> u16 {
    mute_ramp_len(rate).saturating_mul(2)
}

/// The switch, per session, on the audio thread: `pos` in `0..=len`, 0 the
/// wet (the chain's output, untouched), `len` the aligned dry. It moves one
/// step per sample toward its target and the sample is scaled after the step.
/// A toggle mid-fade turns round: at once toward the wet; toward the dry
/// after the wait (D8b).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PtmGate {
    pos: u16,
    len: u16,
    /// The last request seen.
    want_dry: bool,
    /// The target `pos` moves to: `len` when true, 0 when false. Only ever
    /// true while `want_dry` is.
    to_dry: bool,
    /// While `want_dry && !to_dry`: the samples an edge toward the dry still
    /// waits before it turns (D8b).
    wait_left: usize,
}

impl PtmGate {
    /// Starts settled where `bypassed` says, with no wait. `len` is a
    /// [`ptm_fade_len`] value.
    #[must_use]
    pub(crate) fn new(len: u16, bypassed: bool) -> Self {
        debug_assert!(len.is_power_of_two(), "not a ptm_fade_len value: {len}");
        // 0 would make every gain infinite; release keeps it in range too.
        let len = len.max(1);
        Self {
            pos: if bypassed { len } else { 0 },
            len,
            want_dry: bypassed,
            to_dry: bypassed,
            wait_left: 0,
        }
    }

    /// One buffer. `bypassed` is this buffer's request; `wait` is what an
    /// edge toward the dry waits, in samples, taken at the edge (a later
    /// change doesn't move it). Per sample:
    ///
    /// 1. on an edge, record it: toward the dry, `wait_left = wait`; toward
    ///    the wet, target the wet at once (a wait still running is cancelled);
    /// 2. while an edge toward the dry waits: turn when `wait_left` is 0,
    ///    else count down;
    /// 3. step `pos` one toward its target, then scale: 0 leaves the sample,
    ///    `len` writes the dry, anything between is `w + (d − w)·pos/len`.
    ///
    /// The request is the same for the whole buffer, so step 1 runs once, on
    /// the first sample. While `pos` stays 0 (settled applied, or waiting)
    /// `wet` is not touched and `dry` is not read. `len` is a power of two,
    /// so every gain is exact, and a `dry` equal to `wet` crosses with no
    /// change at all. Returns whether `pos` moved in this buffer (the voice
    /// crossed). An empty buffer moves nothing.
    pub(crate) fn blend(
        &mut self,
        bypassed: bool,
        wait: usize,
        wet: &mut [f32],
        dry: &[f32],
    ) -> bool {
        debug_assert!(dry.len() >= wet.len(), "the dry is shorter than the wet");
        if wet.is_empty() {
            return false;
        }
        if bypassed != self.want_dry {
            self.want_dry = bypassed;
            if bypassed {
                self.wait_left = wait;
            } else {
                self.to_dry = false;
                self.wait_left = 0;
            }
        }
        // Settled applied: the wet, untouched.
        if self.pos == 0 && !self.want_dry {
            return false;
        }
        // Waiting through the whole buffer: one count a sample, nothing heard.
        if self.pos == 0 && self.want_dry && !self.to_dry && self.wait_left >= wet.len() {
            self.wait_left -= wet.len();
            return false;
        }
        // Settled bypassed: the dry, copied.
        if self.pos == self.len && self.to_dry {
            for (w, &d) in wet.iter_mut().zip(dry) {
                *w = d;
            }
            return false;
        }
        let step = 1.0 / f32::from(self.len);
        let mut moved = false;
        for (w, &d) in wet.iter_mut().zip(dry) {
            if self.want_dry && !self.to_dry {
                if self.wait_left == 0 {
                    self.to_dry = true;
                } else {
                    self.wait_left -= 1;
                }
            }
            if self.to_dry {
                if self.pos < self.len {
                    self.pos += 1;
                    moved = true;
                }
            } else if self.pos > 0 {
                self.pos -= 1;
                moved = true;
            }
            if self.pos == self.len {
                *w = d;
            } else if self.pos > 0 {
                *w += (d - *w) * (f32::from(self.pos) * step);
            }
        }
        moved
    }

    /// Tests only: `pos` at its target's end and no edge waiting.
    #[cfg(test)]
    #[must_use]
    pub(crate) fn settled(&self) -> bool {
        !self.waiting() && self.pos == if self.to_dry { self.len } else { 0 }
    }

    /// Tests only: an edge toward the dry is waiting (D8b).
    #[cfg(test)]
    #[must_use]
    pub(crate) fn waiting(&self) -> bool {
        self.want_dry && !self.to_dry
    }

    /// Tests only: where the fade is, 0 (wet) to `len` (dry).
    #[cfg(test)]
    #[must_use]
    pub(crate) fn pos(&self) -> u16 {
        self.pos
    }
}

/// How the line is moving its read point (D4). The `u16`s count the fade's
/// samples done; `Gap` counts the silent samples left.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Move {
    Settled,
    /// A shrink: the old tap crossfades to the new one.
    Cross(u16),
    /// A growth, first: the old tap fades out.
    Out(u16),
    /// A growth, then: silence for the difference, so nothing is heard twice.
    Gap(usize),
    /// A growth, last: the new tap fades in.
    In(u16),
}

/// The dry voice, delayed by the chain's reported latency so it carries the
/// moment the chain's output does. Per session; it allocates in [`new`]
/// (before the callback), never after.
///
/// [`new`]: DryLine::new
pub(crate) struct DryLine {
    /// The voice, the last `ring.len()` samples; a power of two long.
    ring: Box<[f32]>,
    mask: usize,
    /// Samples written, wrapping; the next write goes at `head & mask`.
    head: usize,
    /// This call's requested delay, clamped to [`max_delay`](Self::max_delay).
    want: usize,
    /// The tap being moved to, or settled on. Starts at 0.
    delay: usize,
    /// The tap being moved from.
    from: usize,
    moving: Move,
    /// The fade length.
    len: u16,
    /// This call's output, `voice` delayed.
    aligned: Box<[f32]>,
}

impl DryLine {
    /// Settled at delay 0, every sample zero. The ring is the power of two
    /// above `rate`, so it holds at least a second; `len` is a
    /// [`ptm_fade_len`] value.
    #[must_use]
    pub(crate) fn new(rate: u32, len: u16) -> Self {
        debug_assert!(len.is_power_of_two(), "not a ptm_fade_len value: {len}");
        let size = usize::try_from(rate)
            .unwrap_or(usize::MAX)
            .saturating_add(1)
            .min(MAX_RING)
            .next_power_of_two();
        Self {
            ring: vec![0.0; size].into_boxed_slice(),
            mask: size - 1,
            head: 0,
            want: 0,
            delay: 0,
            from: 0,
            moving: Move::Settled,
            len: len.max(1),
            aligned: vec![0.0; MAX_FRAMES_PER_CALLBACK].into_boxed_slice(),
        }
    }

    /// The longest delay it holds: the ring's length − 1 (at least a second).
    #[must_use]
    pub(crate) fn max_delay(&self) -> usize {
        self.ring.len().saturating_sub(1)
    }

    /// Latches `want = min(latency, max_delay)`, writes `voice` into the ring
    /// and fills [`aligned`](Self::aligned) with it delayed. Per sample: when
    /// settled and `want` differs from the delay, a move starts at that
    /// sample (`from = delay; delay = want`). A shrink crossfades the old tap
    /// to the new one over `len`, so the difference is skipped, blended. A
    /// growth fades the old tap out over `len`, is silent for the difference,
    /// then fades the new tap in over `len`, so no sample is heard twice. A
    /// request that changes during a move is acted on at the first sample
    /// after the move ends. So the output is the same for every split of a
    /// run into buffers. Takes at most `aligned`'s length (the callback's
    /// cap, never exceeded).
    pub(crate) fn capture(&mut self, voice: &[f32], latency: usize) {
        self.want = latency.min(self.max_delay());
        let Self {
            ring,
            mask,
            head,
            want,
            delay,
            from,
            moving,
            len,
            aligned,
        } = self;
        let (mask, want, len) = (*mask, *want, *len);
        let step = 1.0 / f32::from(len);
        let gain = |k: u16| f32::from(k) * step;
        for (&x, out) in voice.iter().zip(aligned.iter_mut()) {
            if let Some(slot) = ring.get_mut(*head & mask) {
                *slot = x;
            }
            let ring: &[f32] = ring;
            let at = *head;
            let tap = |d: usize| ring.get(at.wrapping_sub(d) & mask).copied().unwrap_or(0.0);
            if *moving == Move::Settled && want != *delay {
                *from = *delay;
                *delay = want;
                *moving = if *delay < *from {
                    Move::Cross(0)
                } else {
                    Move::Out(0)
                };
            }
            *out = match *moving {
                Move::Settled => tap(*delay),
                Move::Cross(k) => {
                    let k = k + 1;
                    if k >= len {
                        *moving = Move::Settled;
                        tap(*delay)
                    } else {
                        *moving = Move::Cross(k);
                        let old = tap(*from);
                        old + (tap(*delay) - old) * gain(k)
                    }
                }
                Move::Out(k) => {
                    let k = k + 1;
                    if k >= len {
                        *moving = Move::Gap(delay.saturating_sub(*from).max(1));
                        0.0
                    } else {
                        *moving = Move::Out(k);
                        tap(*from) * gain(len - k)
                    }
                }
                Move::Gap(left) => {
                    let left = left.saturating_sub(1);
                    *moving = if left == 0 {
                        Move::In(0)
                    } else {
                        Move::Gap(left)
                    };
                    0.0
                }
                Move::In(k) => {
                    let k = k + 1;
                    if k >= len {
                        *moving = Move::Settled;
                        tap(*delay)
                    } else {
                        *moving = Move::In(k);
                        tap(*delay) * gain(k)
                    }
                }
            };
            *head = head.wrapping_add(1);
        }
    }

    /// The first `n` samples of the last [`capture`](Self::capture): the
    /// voice it was given, delayed.
    #[must_use]
    pub(crate) fn aligned(&self, n: usize) -> &[f32] {
        self.aligned.get(..n).unwrap_or(&self.aligned)
    }

    /// The delay the last call asked for, clamped: what an edge toward the
    /// dry waits (D8b).
    #[must_use]
    pub(crate) fn want(&self) -> usize {
        self.want
    }

    /// Tests only: a move is running.
    #[cfg(test)]
    #[must_use]
    pub(crate) fn moving(&self) -> bool {
        self.moving != Move::Settled
    }

    /// Tests only: the tap being moved to, or settled on.
    #[cfg(test)]
    #[must_use]
    pub(crate) fn delay(&self) -> usize {
        self.delay
    }
}

/// What the voice stage tells the callback.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct StageReport {
    /// The chain's reported latency this buffer, read once after the
    /// modulator: the delay the dry was given, and the screen recorder's.
    pub latency: usize,
    /// The switch moved in this buffer (the voice crossed), so the reading
    /// drops its after-effects window.
    pub crossed: bool,
}

/// What the output callback owns: the switch and the line.
pub(crate) struct PtmStage {
    gate: PtmGate,
    line: DryLine,
}

impl PtmStage {
    /// Settled at `bypassed`; the line at delay 0 (the chain is empty until
    /// the first `ReplaceChain`, D4). Allocates the line here.
    #[must_use]
    pub(crate) fn new(rate: u32, bypassed: bool) -> Self {
        let len = ptm_fade_len(rate);
        Self {
            gate: PtmGate::new(len, bypassed),
            line: DryLine::new(rate, len),
        }
    }

    /// The dry voice, before the chain rewrites it: into the line, delayed
    /// by `latency` (by 0 with [`ALIGN_DRY`] off).
    pub(crate) fn capture(&mut self, voice: &[f32], latency: usize) {
        self.line
            .capture(voice, if ALIGN_DRY { latency } else { 0 });
    }

    /// The chain's output in `voice`, switched toward the aligned dry by
    /// `bypass`; an edge toward the dry waits the line's latency. Whether
    /// the voice crossed in this buffer.
    pub(crate) fn blend(&mut self, voice: &mut [f32], bypass: bool) -> bool {
        let wait = self.line.want();
        self.gate
            .blend(bypass, wait, voice, self.line.aligned(voice.len()))
    }

    /// The aligned dry of the last capture, its first `n` samples.
    #[must_use]
    pub(crate) fn aligned(&self, n: usize) -> &[f32] {
        self.line.aligned(n)
    }

    /// Tests only: the gate's [`PtmGate::settled`].
    #[cfg(test)]
    #[must_use]
    pub(crate) fn settled(&self) -> bool {
        self.gate.settled()
    }

    /// Tests only: the gate's [`PtmGate::waiting`].
    #[cfg(test)]
    #[must_use]
    pub(crate) fn waiting(&self) -> bool {
        self.gate.waiting()
    }

    /// Tests only: the line's [`DryLine::moving`].
    #[cfg(test)]
    #[must_use]
    pub(crate) fn line_moving(&self) -> bool {
        self.line.moving()
    }

    /// Tests only: the line's [`DryLine::delay`].
    #[cfg(test)]
    #[must_use]
    pub(crate) fn delay(&self) -> usize {
        self.line.delay()
    }
}

#[cfg(test)]
#[allow(clippy::float_cmp)] // every gain is exact, so the outputs are compared exactly
mod tests {
    use super::{
        is_bypassed, pack, ptm_fade_len, ptm_transition, unpack, DryLine, PtmMode, PtmOp,
        PtmSource, PtmStage, PtmState, SEQ_MAX,
    };

    /// The bundled presets' rate.
    const RATE: u32 = 48_000;

    /// A signal with no repeats in any window shorter than 997 and no zeros,
    /// so a bit compare means something.
    fn signal(n: usize) -> Vec<f32> {
        (0..n)
            .map(|i| {
                let k = u16::try_from(i % 997).unwrap();
                (f32::from(k) + 1.0) / 1000.0 - 0.5
            })
            .collect()
    }

    fn bits(y: &[f32]) -> Vec<u32> {
        y.iter().map(|s| s.to_bits()).collect()
    }

    /// Design §7.1: every mode against nothing held and each bit held, plus
    /// two mode bytes no setter writes.
    #[test]
    fn bypassed_table() {
        // (mode byte, bypassed with nothing held, bypassed with a bit held)
        for (mode, none, any) in [
            (0_u64, false, false),
            (1, true, false),
            (2, false, true),
            (3, false, false),
            (255, false, false),
        ] {
            for seq in [0_u64, 1, SEQ_MAX] {
                let word = seq << 16 | mode << 8;
                assert_eq!(is_bypassed(word), none, "mode {mode}, nothing held");
                for bit in 0..8 {
                    assert_eq!(
                        is_bypassed(word | 1 << bit),
                        any,
                        "mode {mode}, bit {bit} held"
                    );
                }
            }
        }
    }

    /// Design §7.2, every row, through the word.
    #[test]
    fn transition_table() {
        use PtmMode::{Always, Apply, Bypass};
        use PtmSource::{Button, Hotkey, Key, Midi};
        /// A word's parts: mode, held bits, seq.
        type Parts = (PtmMode, u8, u64);
        let rows: [(Parts, PtmOp, Option<Parts>); 13] = [
            ((Always, 0, 0), PtmOp::SetMode(Always), None),
            ((Always, 0, 0), PtmOp::SetMode(Apply), Some((Apply, 0, 1))),
            (
                (Apply, 0b0010, 5),
                PtmOp::SetMode(Apply),
                Some((Apply, 0, 6)),
            ),
            (
                (Apply, 0b0001, 6),
                PtmOp::SetMode(Apply),
                Some((Apply, 0, 7)),
            ),
            (
                (Apply, 0, 6),
                PtmOp::SetHeld(Key, true),
                Some((Apply, 0b0010, 7)),
            ),
            ((Apply, 0b0010, 7), PtmOp::SetHeld(Key, true), None),
            ((Apply, 0b0010, 7), PtmOp::SetHeld(Hotkey, false), None),
            (
                (Apply, 0b0011, 9),
                PtmOp::SetHeld(Key, false),
                Some((Apply, 0b0001, 10)),
            ),
            (
                (Apply, 0b0011, 9),
                PtmOp::Restore(Apply),
                Some((Apply, 0b0001, 10)),
            ),
            ((Apply, 0b0001, 10), PtmOp::Restore(Apply), None),
            (
                (Always, 0b0001, 3),
                PtmOp::Restore(Apply),
                Some((Apply, 0b0001, 4)),
            ),
            (
                (Apply, 0b1110, 4),
                PtmOp::Restore(Apply),
                Some((Apply, 0, 5)),
            ),
            (
                (Bypass, 0b1000, SEQ_MAX),
                PtmOp::SetHeld(Midi, false),
                Some((Bypass, 0, SEQ_MAX)),
            ),
        ];
        for ((mode, held, seq), op, want) in rows {
            let word = pack(mode, held, seq);
            assert_eq!(
                ptm_transition(word, op),
                want.map(|(m, h, s)| pack(m, h, s)),
                "{op:?} from ({mode:?}, {held:#06b}, {seq})"
            );
        }
        // Launch is "always", nothing held, seq 0; the parts round-trip.
        assert_eq!(unpack(0), PtmState::default());
        for mode in [Always, Apply, Bypass] {
            for held in [0_u8, 1, 0b1110, 0xff] {
                for seq in [0, 1, 1 << 40, SEQ_MAX] {
                    let word = pack(mode, held, seq);
                    assert_eq!(
                        unpack(word),
                        PtmState {
                            mode,
                            held: held != 0,
                            bypassed: is_bypassed(word),
                            seq
                        }
                    );
                }
            }
        }
        // A set to what the word holds moves nothing, `seq` included.
        for word in [0, pack(Apply, 0b0101, 7), pack(Bypass, 0b1000, SEQ_MAX)] {
            let state = unpack(word);
            let [held, ..] = word.to_le_bytes();
            for source in [Hotkey, Key, Button, Midi] {
                let on = held & source.bit() != 0;
                assert_eq!(ptm_transition(word, PtmOp::SetHeld(source, on)), None);
            }
            if !state.held {
                assert_eq!(ptm_transition(word, PtmOp::SetMode(state.mode)), None);
            }
        }
    }

    /// Design §7.3, the L row; and at least 10 ms, under 20 ms, wherever a
    /// power of two in 128..=4096 can give it.
    #[test]
    fn fade_len_table() {
        for (rate, len) in [
            (8_000, 128),
            (11_025, 128),
            (16_000, 256),
            (22_050, 256),
            (24_000, 256),
            (32_000, 512),
            (44_100, 512),
            (48_000, 512),
            (88_200, 1024),
            (96_000, 1024),
            (176_400, 2048),
            (192_000, 2048),
            (384_000, 4096),
        ] {
            assert_eq!(ptm_fade_len(rate), len, "{rate} Hz");
        }
        for rate in 12_800..=409_600_u32 {
            let len = u32::from(ptm_fade_len(rate));
            assert!(len.is_power_of_two(), "{rate} Hz: L {len}");
            assert!(100 * len >= rate, "{rate} Hz: L {len} is under 10 ms");
            assert!(50 * len < rate, "{rate} Hz: L {len} is 20 ms or more");
        }
    }

    /// Design §7.3, the ring and max-delay rows: at least a second.
    #[test]
    fn ring_table() {
        for (rate, ring) in [
            (8_000, 8_192),
            (11_025, 16_384),
            (16_000, 16_384),
            (22_050, 32_768),
            (24_000, 32_768),
            (32_000, 32_768),
            (44_100, 65_536),
            (48_000, 65_536),
            (88_200, 131_072),
            (96_000, 131_072),
            (176_400, 262_144),
            (192_000, 262_144),
            (384_000, 524_288_usize),
        ] {
            let line = DryLine::new(rate, ptm_fade_len(rate));
            assert_eq!(line.max_delay(), ring - 1, "{rate} Hz");
            assert!(line.max_delay() >= usize::try_from(rate).unwrap());
        }
    }

    /// One run through a stage whose chain is a pure delay of `n`: per phase
    /// `(bypassed, length)`, split into calls of `split` frames. Returns the
    /// stage's output.
    fn pure_delay_run(n: usize, phases: &[(bool, usize)], split: usize) -> Vec<f32> {
        let total = phases.iter().map(|&(_, len)| len).sum();
        let input = signal(total);
        let wet = |i: usize| i.checked_sub(n).map_or(0.0, |j| input[j]);
        let mut stage = PtmStage::new(RATE, false);
        let mut out = Vec::with_capacity(total);
        let mut at = 0;
        for &(bypassed, len) in phases {
            let end = at + len;
            while at < end {
                let frames = split.min(end - at);
                let mut buf = input[at..at + frames].to_vec();
                stage.capture(&buf, n);
                for (k, s) in buf.iter_mut().enumerate() {
                    *s = wet(at + k);
                }
                stage.blend(&mut buf, bypassed);
                out.extend(buf);
                at += frames;
            }
        }
        out
    }

    /// The aligned dry of a pure delay is its wet, sample for sample, so once
    /// the line has moved to the delay every crossing changes nothing; and
    /// the result is the same for every split into buffers. An aligned dry
    /// off by one sample fails the first; a rule applied per call rather than
    /// per sample fails the second.
    #[test]
    fn a_pure_delay_crosses_with_no_change_at_any_split() {
        let l = usize::from(ptm_fade_len(RATE));
        for n in [0, 1, 480, 1024, 2048] {
            // Applied until the line has settled (n + 2L), then flips that
            // complete, and some that turn a fade round or cancel a wait.
            let phases = [
                (false, n + 2 * l + 1),
                (true, n + l + 700),
                (false, l / 2),
                (true, n / 2 + 1),
                (false, l + 300),
                (true, n + 3 * l),
                (false, 2 * l),
            ];
            let total: usize = phases.iter().map(|&(_, len)| len).sum();
            let input = signal(total);
            let reference = pure_delay_run(n, &phases, 1);
            for (i, &y) in reference.iter().enumerate() {
                let wet = i.checked_sub(n).map_or(0.0, |j| input[j]);
                assert_eq!(y.to_bits(), wet.to_bits(), "delay {n}: sample {i}");
            }
            for split in [7, 100, 255, 480, 1056, 4096] {
                assert!(
                    bits(&pure_delay_run(n, &phases, split)) == bits(&reference),
                    "delay {n}: buffers of {split} differ"
                );
            }
        }
    }

    /// Through the stage's accessors: a new stage is settled at delay 0; a
    /// latency starts a growth move (out, gap, in) with the gate applied and
    /// the buffer untouched; an edge toward the dry waits the latency, then
    /// crosses over L to the aligned dry exactly; an edge back moves at once.
    #[test]
    fn the_stage_waits_out_the_latency_then_crosses() {
        const N: usize = 1024;
        let l = usize::from(ptm_fade_len(RATE));
        let input = signal(20 * N);
        let mut at = 0;
        // One buffer of the input through the stage, with `-input` as the
        // chain's output: whether it crossed, and what it sent.
        let mut buffer = |stage: &mut PtmStage, frames: usize, bypass: bool| {
            let mut buf = input[at..at + frames].to_vec();
            stage.capture(&buf, N);
            for s in &mut buf {
                *s = -*s;
            }
            let crossed = stage.blend(&mut buf, bypass);
            at += frames;
            (crossed, buf, at)
        };
        let mut stage = PtmStage::new(RATE, false);
        assert!(stage.settled() && !stage.waiting() && !stage.line_moving());
        assert_eq!((stage.delay(), stage.gate.pos()), (0, 0));

        // The growth from 0 (out over L, N silent, in over L), applied: the
        // wet untouched, nothing crossed.
        let mut moved = 0;
        loop {
            let (crossed, out, now) = buffer(&mut stage, 1, false);
            assert!(!crossed);
            assert_eq!(out, [-input[now - 1]]);
            assert_eq!(stage.delay(), N);
            moved += 1;
            if !stage.line_moving() {
                assert_eq!(stage.aligned(1), [input[now - 1 - N]]);
                break;
            }
        }
        assert_eq!(moved, N + 2 * l, "the growth took {moved} samples");

        // An edge toward the dry waits N samples, the buffer untouched.
        let (crossed, out, now) = buffer(&mut stage, N, true);
        assert!(!crossed && stage.waiting() && !stage.settled());
        assert_eq!(stage.gate.pos(), 0);
        assert!(out.iter().zip(&input[now - N..now]).all(|(&y, &x)| y == -x));
        // Then it crosses, a step a sample, and settles on the aligned dry.
        let (crossed, out, now) = buffer(&mut stage, l, true);
        assert!(crossed && !stage.waiting() && stage.settled());
        assert_eq!(usize::from(stage.gate.pos()), l);
        assert_eq!(out[l - 1], input[now - 1 - N]);
        assert_ne!(out[l - 2], input[now - 2 - N]);
        let (crossed, out, now) = buffer(&mut stage, 480, true);
        assert!(!crossed);
        assert_eq!(bits(&out), bits(&input[now - 480 - N..now - N]));

        // An edge back toward the effects moves on its first sample.
        let (crossed, _, _) = buffer(&mut stage, 1, false);
        assert!(crossed && !stage.settled());
        assert_eq!(usize::from(stage.gate.pos()), l - 1);
    }
}
