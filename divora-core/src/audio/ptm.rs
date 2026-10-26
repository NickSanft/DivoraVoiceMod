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
//! carry the same moment of speech ([`DryLine`]) wherever each effect's real
//! delay is what it reports; where it isn't, a switch skips or repeats the
//! difference (448 samples, 9.3 ms, for the noise remover at 512-frame
//! buffers: `the_denoisers_real_delay_is_pinned` in engine.rs). [`PtmGate`]
//! is the crossfade: an edge toward the effects starts at once, an edge away
//! from them first waits out that latency, so the effects cover everything
//! said while they were meant to be on, except just after an edge toward
//! them when the chain reports less latency than the fade (`L`,
//! [`ptm_fade_len`]): the first `L − 1 − latency` samples said after the
//! edge go out partly dry while the effects fade in. That is 511 samples
//! (10.6 ms at 48 kHz) from a chain that reports 0, as Dispatch's gate, EQ
//! and distortion do.

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
    /// so every gain is exact, and a finite `dry` equal to `wet` crosses
    /// unchanged, bit for bit, except a −0.0, which comes out +0.0 mid-fade.
    /// Returns whether `pos` moved in this buffer (the voice crossed). An
    /// empty buffer moves nothing.
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
/// moment the chain's output does wherever each effect's real delay is what
/// it reports (the module doc has an example where it isn't). Per session;
/// it allocates in [`new`] (before the callback), never after.
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
        is_bypassed, pack, ptm_fade_len, ptm_transition, unpack, DryLine, Move, PtmGate, PtmMode,
        PtmOp, PtmSource, PtmStage, PtmState, ALIGN_DRY, SEQ_MAX,
    };

    /// The bundled presets' rate.
    const RATE: u32 = 48_000;

    /// Every fade length there is.
    const LENS: [u16; 6] = [128, 256, 512, 1024, 2048, 4096];

    /// Design §7.3's rates.
    const RATES: [u32; 13] = [
        8_000, 11_025, 16_000, 22_050, 24_000, 32_000, 44_100, 48_000, 88_200, 96_000, 176_400,
        192_000, 384_000,
    ];

    /// A signalling NaN. x86 `mulss` quiets it, so it is the one value that
    /// shows a multiply by 1.0: every other value survives `x · 1.0` intact.
    const SNAN: f32 = f32::from_bits(0x7f80_0001);

    /// Values a gain or a copy could change: NaNs, infinities, both zeros,
    /// subnormals, the extremes.
    const ODD: [f32; 15] = [
        f32::NAN,
        f32::from_bits(0x7fc0_1234),
        f32::from_bits(0xffc0_0001),
        SNAN,
        f32::INFINITY,
        f32::NEG_INFINITY,
        -0.0,
        0.0,
        f32::from_bits(1),
        f32::from_bits(0x8000_0001),
        f32::from_bits(0x007f_ffff),
        f32::MIN_POSITIVE,
        0.5,
        -1.0,
        f32::MAX,
    ];

    /// The fade length at [`RATE`], as a count.
    fn fade() -> usize {
        usize::from(ptm_fade_len(RATE))
    }

    /// `1, 2, 3, …`: every value distinct and exact, so a delayed copy can be
    /// told from any other, and `value − 1` is the index it came from.
    #[allow(clippy::cast_precision_loss)] // integers well under 2^24
    fn ramp(n: usize) -> Vec<f32> {
        (1..=n).map(|i| i as f32).collect()
    }

    /// The largest step in `y`, counting the one from `prev` into it.
    fn max_step(prev: f32, y: &[f32]) -> f32 {
        let mut last = prev;
        let mut max = 0.0_f32;
        for &s in y {
            max = max.max((s - last).abs());
            last = s;
        }
        max
    }

    /// xorshift64: a seeded walk, the same every run.
    struct XorShift(u64);

    impl XorShift {
        fn next(&mut self) -> u64 {
            let mut x = self.0;
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            self.0 = x;
            x
        }

        /// Uniform in `0..n`.
        fn below(&mut self, n: usize) -> usize {
            usize::try_from(self.next() % u64::try_from(n).unwrap()).unwrap()
        }

        /// Uniform in `-0.5..0.5`.
        fn unit(&mut self) -> f32 {
            f32::from(u16::try_from(self.next() >> 48).unwrap()) / 65_536.0 - 0.5
        }
    }

    /// The `i`th sample of a stream named by `salt`, in `-0.5..0.5`, from the
    /// index alone (splitmix64), so an oracle can ask for any past sample.
    fn noise_at(i: usize, salt: u64) -> f32 {
        let mut z = u64::try_from(i)
            .unwrap()
            .wrapping_add(salt)
            .wrapping_mul(0x9e37_79b9_7f4a_7c15);
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^= z >> 31;
        f32::from(u16::try_from(z >> 48).unwrap()) / 65_536.0 - 0.5
    }

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

    // ---- the gate ----

    /// Settled on the effects, and with an edge toward the dry still
    /// waiting, the gate leaves the wet as it is, bit for bit, and never
    /// reads the dry: a gain of 1 would quiet the signalling NaN, and a
    /// blend of 0 with a NaN dry would be NaN.
    #[test]
    fn an_applied_gate_is_bit_exact() {
        let nan = [f32::NAN; ODD.len()];
        for len in LENS {
            let mut gate = PtmGate::new(len, false);
            for _ in 0..3 {
                let mut wet = ODD;
                assert!(!gate.blend(false, 4096, &mut wet, &nan));
                assert_eq!(bits(&wet), bits(&ODD), "L {len}: settled applied");
            }
            assert!(gate.settled());

            // Waiting out a latency longer than three buffers.
            let wait = 3 * ODD.len() + 1;
            for _ in 0..3 {
                let mut wet = ODD;
                assert!(!gate.blend(true, wait, &mut wet, &nan));
                assert_eq!(bits(&wet), bits(&ODD), "L {len}: waiting");
                assert!(gate.waiting() && gate.pos() == 0);
            }

            // A wait that ends inside the buffer: nothing before the turn.
            for wait in [1, 5, ODD.len() - 1] {
                let mut gate = PtmGate::new(len, false);
                let mut wet = ODD;
                assert!(gate.blend(true, wait, &mut wet, &nan));
                assert_eq!(
                    bits(&wet[..wait]),
                    bits(&ODD[..wait]),
                    "L {len}: wait {wait}"
                );
            }
        }
    }

    /// Settled on the dry, the gate copies it, bits and all (a signalling
    /// NaN and −0.0 survive a copy, not a multiply), and nothing of the wet
    /// leaks through, NaN included; the same from the sample a crossing
    /// settles on.
    #[test]
    fn a_bypassed_gate_is_the_dry_exactly() {
        for len in LENS {
            let l = usize::from(len);
            let mut gate = PtmGate::new(len, true);
            for _ in 0..3 {
                let mut wet = [f32::NAN; ODD.len()];
                assert!(!gate.blend(true, 4096, &mut wet, &ODD));
                assert_eq!(bits(&wet), bits(&ODD), "L {len}: settled bypassed");
            }

            let dry: Vec<f32> = ODD.iter().copied().cycle().take(l + 45).collect();
            let mut wet = vec![f32::NAN; dry.len()];
            let mut gate = PtmGate::new(len, false);
            assert!(gate.blend(true, 0, &mut wet, &dry));
            assert_eq!(
                bits(&wet[l - 1..]),
                bits(&dry[l - 1..]),
                "L {len}: settling"
            );
            assert!(gate.settled());
        }
    }

    /// Design §3.2: a wet of 0 against a dry of 0.5 reads the gain straight
    /// off the output: `0.5·k/L` for `k = 1..L` exactly, then the dry; and
    /// back down the same steps, ending on the wet itself.
    #[test]
    fn the_fade_is_linear_and_exact() {
        for len in LENS {
            let l = usize::from(len);
            let at = |k: u16| 0.5 * (f32::from(k) / f32::from(len));
            let dry = vec![0.5_f32; 2 * l];
            let mut gate = PtmGate::new(len, false);
            let mut y = vec![0.0_f32; 2 * l];
            assert!(gate.blend(true, 0, &mut y, &dry));
            for k in 1..=len {
                assert_eq!(y[usize::from(k) - 1], at(k), "L {len}: k {k}");
            }
            assert!(y[l..].iter().all(|&s| s == 0.5), "L {len}: not settled");

            let mut y = vec![0.0_f32; 2 * l];
            assert!(gate.blend(false, 0, &mut y, &dry));
            for (i, k) in (0..len).rev().enumerate() {
                assert_eq!(y[i].to_bits(), at(k).to_bits(), "L {len}: back, k {k}");
            }
            assert!(
                y[l..].iter().all(|s| s.to_bits() == 0),
                "L {len}: not the wet"
            );
        }
    }

    /// When the dry is the wet (a chain that is a pure delay, aligned), every
    /// crossing leaves the samples as they were, bit for bit: `w + (d − w)·g`
    /// is `w` when `d == w`, for any finite value but −0.0 (which comes out
    /// +0.0 mid-fade; `unit` never draws it).
    #[test]
    fn a_pure_delay_crosses_with_no_change() {
        let mut rng = XorShift(0x853c_49e6_748f_ea9b);
        for len in LENS {
            let l = usize::from(len);
            let mut gate = PtmGate::new(len, false);
            let mut crossed = 0;
            for _ in 0..300 {
                let frames = 1 + rng.below(3 * l);
                let bypassed = rng.next().is_multiple_of(2);
                let wait = rng.below(2 * l);
                let wet: Vec<f32> = (0..frames).map(|_| rng.unit()).collect();
                let mut y = wet.clone();
                crossed += usize::from(gate.blend(bypassed, wait, &mut y, &wet));
                assert_eq!(bits(&y), bits(&wet), "L {len}");
            }
            assert!(crossed > 10, "L {len}: only {crossed} buffers crossed");
        }
    }

    /// A wet of +0.5 against a dry of −0.5, the largest jump there is: every
    /// step of every fade is exactly 1/L, at every rate.
    #[test]
    fn no_step_larger_than_the_slope() {
        for rate in RATES {
            let len = ptm_fade_len(rate);
            let l = usize::from(len);
            let slope = 1.0 / f32::from(len);
            let dry = vec![-0.5_f32; 2 * l];
            let mut gate = PtmGate::new(len, false);
            let mut last = 0.5_f32;
            for bypassed in [true, false, true, false] {
                let mut y = vec![0.5_f32; 2 * l];
                gate.blend(bypassed, 0, &mut y, &dry);
                let mut steps = 0;
                for &s in &y {
                    let step = (s - last).abs();
                    assert!(step == 0.0 || step == slope, "{rate} Hz: a step of {step}");
                    steps += usize::from(step > 0.0);
                    last = s;
                }
                assert_eq!(steps, l, "{rate} Hz: not one whole fade");
                assert_eq!(last, if bypassed { -0.5 } else { 0.5 });
            }
        }
    }

    /// D8b: an edge toward the dry holds `pos` at 0 for exactly the wait it
    /// was given, then moves one step a sample. The wait is taken at the
    /// edge: what later calls pass (here 0 and 99 999 in turn) doesn't
    /// change when it moves.
    #[test]
    fn an_edge_toward_the_dry_waits() {
        let len = ptm_fade_len(RATE);
        let l = usize::from(len);
        // Where `pos` is after sample `i` of an edge that waits `wait`.
        let pos_at = |wait: usize, i: usize| (i + 1).saturating_sub(wait).min(l);
        // The wait later calls pass.
        let later = |call: usize| if call.is_multiple_of(2) { 0 } else { 99_999 };
        for wait in [0, 1, 511, 512, 2048, 12_547] {
            // A sample a call, read through the accessor.
            let mut gate = PtmGate::new(len, false);
            for i in 0..wait + l + 10 {
                let mut y = [0.0_f32];
                gate.blend(true, if i == 0 { wait } else { later(i) }, &mut y, &[1.0]);
                assert_eq!(usize::from(gate.pos()), pos_at(wait, i), "wait {wait}: {i}");
                assert_eq!(gate.waiting(), i < wait, "wait {wait}: {i}");
            }
            // Bigger calls, read off the output: a wet of 0 and a dry of 1
            // put `pos/L` on every sample.
            for split in [7, 480, 4096] {
                let mut gate = PtmGate::new(len, false);
                let total = wait + l + 10;
                let mut y = vec![0.0_f32; total];
                for (call, chunk) in y.chunks_mut(split).enumerate() {
                    let w = if call == 0 { wait } else { later(call) };
                    gate.blend(true, w, chunk, &vec![1.0; chunk.len()]);
                }
                for (i, &s) in y.iter().enumerate() {
                    let want = f32::from(u16::try_from(pos_at(wait, i)).unwrap()) / f32::from(len);
                    assert_eq!(s, want, "wait {wait}, buffers of {split}: sample {i}");
                }
            }
        }
    }

    /// An edge back toward the effects before the wait ends cancels it: at
    /// every offset into the wait, `pos` never leaves 0, and a NaN dry never
    /// reaches the buffer however long it then runs.
    #[test]
    fn a_flip_back_during_the_wait_cancels_it() {
        let len = ptm_fade_len(RATE);
        for wait in [600, 2048] {
            let nan = vec![f32::NAN; 2 * wait + 4 * usize::from(len)];
            let wet = signal(nan.len());
            for offset in 1..wait {
                let mut gate = PtmGate::new(len, false);
                let mut y = wet.clone();
                let (first, rest) = y.split_at_mut(offset);
                assert!(!gate.blend(true, wait, first, &nan));
                assert!(gate.waiting());
                assert!(!gate.blend(false, wait, rest, &nan));
                assert!(gate.settled() && gate.pos() == 0, "wait {wait}: {offset}");
                assert!(
                    bits(&y) == bits(&wet),
                    "wait {wait}: touched, flipped at {offset}"
                );
            }
        }
    }

    /// An edge toward the effects doesn't wait, whatever wait it is passed:
    /// `pos` moves on its first sample.
    #[test]
    fn an_edge_toward_the_wet_starts_at_once() {
        for len in LENS {
            for frames in [1, 7, 480] {
                let mut gate = PtmGate::new(len, true);
                let mut y = vec![0.0_f32; frames];
                assert!(gate.blend(false, 2048, &mut y, &vec![1.0; frames]));
                assert_eq!(y[0], f32::from(len - 1) / f32::from(len), "L {len}");
                assert_eq!(
                    usize::from(gate.pos()),
                    usize::from(len).saturating_sub(frames),
                    "L {len}, {frames} frames"
                );
            }
        }
    }

    /// A flip mid-fade turns round rather than snapping to either end: toward
    /// the effects on the very next sample; toward the dry only after the
    /// wait, heading on toward the effects meanwhile. Either way the output
    /// stays continuous, no step over the slope, at every offset into the
    /// fade.
    #[test]
    fn a_toggle_mid_fade_turns_round() {
        let len = ptm_fade_len(RATE);
        let l = usize::from(len);
        let slope = 1.0 / f32::from(len);
        // A wet of +0.5 and a dry of −0.5: `0.5 − pos/L`, exactly.
        let out = |pos: usize| 0.5 - f32::from(u16::try_from(pos).unwrap()) * slope;
        for offset in 1..l {
            // Heading for the dry, then back toward the effects at once.
            let mut gate = PtmGate::new(len, false);
            let mut there = vec![0.5_f32; offset];
            gate.blend(true, 0, &mut there, &vec![-0.5; offset]);
            assert_eq!(usize::from(gate.pos()), offset);
            let mut back = vec![0.5_f32; 2 * l];
            gate.blend(false, 0, &mut back, &vec![-0.5; 2 * l]);
            for (j, &s) in back.iter().enumerate() {
                assert_eq!(
                    s,
                    out(offset.saturating_sub(j + 1)),
                    "back at {offset}: {j}"
                );
            }
            let step = max_step(0.5, &there).max(max_step(there[offset - 1], &back));
            assert!(step <= slope, "back at {offset}: a step of {step}");
            assert!(gate.settled());

            // Heading for the effects, then back toward the dry after `wait`.
            for wait in [0, 1, 100, l / 2, 2 * l] {
                let mut gate = PtmGate::new(len, true);
                let mut there = vec![0.5_f32; offset];
                gate.blend(false, 0, &mut there, &vec![-0.5; offset]);
                let from = l - offset;
                assert_eq!(usize::from(gate.pos()), from);
                let mut back = vec![0.5_f32; wait + 2 * l];
                gate.blend(true, wait, &mut back, &vec![-0.5; wait + 2 * l]);
                let turned = from.saturating_sub(wait);
                for (j, &s) in back.iter().enumerate() {
                    let pos = if j < wait {
                        from.saturating_sub(j + 1)
                    } else {
                        (turned + j + 1 - wait).min(l)
                    };
                    assert_eq!(s, out(pos), "forth at {offset}, wait {wait}: {j}");
                }
                let step = max_step(out(l), &there).max(max_step(there[offset - 1], &back));
                assert!(
                    step <= slope,
                    "forth at {offset}, wait {wait}: a step of {step}"
                );
                assert!(gate.settled());
            }
        }
    }

    // ---- buffers ----

    /// The gate, the line and the whole stage give the same output however a
    /// run is split into buffers, given the request holds within each phase
    /// (the callback reads it once a buffer): one call per phase against
    /// calls of 1 to 4096 frames, bit for bit. Phases shorter than a fade, a
    /// wait and a line move put flips and latency changes mid-fade, mid-wait
    /// and mid-move. A rule applied once a call instead of once a sample (the
    /// fade's position reset, a pending move picked up at the next call)
    /// fails it.
    #[test]
    fn buffers_of_any_size_give_one_result() {
        // (bypassed, latency, length); no phase over 4096, the most one
        // call may carry.
        let phases: [(bool, usize, usize); 18] = [
            (false, 0, 3000),
            (false, 1024, 700),
            (true, 1024, 300),
            (true, 2048, 4000),
            (false, 2048, 200),
            (true, 2048, 2500),
            (true, 512, 1000),
            (false, 512, 100),
            (true, 512, 4096),
            (true, 0, 4096),
            (false, 100, 300),
            (true, 3000, 4096),
            (false, 3000, 4096),
            (true, 3000, 4096),
            (true, 3000, 4096),
            (false, 0, 50),
            (true, 0, 600),
            (true, 70_000, 4096),
        ];
        let total = phases.iter().map(|&(_, _, n)| n).sum();
        let (input, wet) = (
            signal(total),
            (0..total).map(|i| noise_at(i, 1)).collect::<Vec<_>>(),
        );
        // One run of each part, every phase split into `split`-frame calls:
        // the gate (the latency as its wait), the line, the stage.
        let run = |split: usize| {
            let len = ptm_fade_len(RATE);
            let (mut gate, mut line) = (PtmGate::new(len, false), DryLine::new(RATE, len));
            let mut stage = PtmStage::new(RATE, false);
            let (mut gated, mut lined, mut staged) = (Vec::new(), Vec::new(), Vec::new());
            let mut at = 0;
            for &(bypassed, latency, n) in &phases {
                for start in (at..at + n).step_by(split) {
                    let end = (start + split).min(at + n);
                    let mut y = wet[start..end].to_vec();
                    gate.blend(bypassed, latency, &mut y, &input[start..end]);
                    gated.extend(y);
                    line.capture(&input[start..end], latency);
                    lined.extend_from_slice(line.aligned(end - start));
                    stage.capture(&input[start..end], latency);
                    let mut y = wet[start..end].to_vec();
                    stage.blend(&mut y, bypassed);
                    staged.extend(y);
                }
                at += n;
            }
            (bits(&gated), bits(&lined), bits(&staged))
        };
        let whole = run(4096);
        for split in [1, 7, 100, 255, 480, 1056] {
            let (gated, lined, staged) = run(split);
            assert!(gated == whole.0, "the gate: buffers of {split} differ");
            assert!(lined == whole.1, "the line: buffers of {split} differ");
            assert!(staged == whole.2, "the stage: buffers of {split} differ");
        }
    }

    // ---- the line ----

    /// Once the line's move from 0 has ended (`d + 2L` samples), its output
    /// is the input delayed by `d`, exactly, at every buffer size, out to the
    /// longest delay it holds.
    #[test]
    fn the_dry_is_the_input_delayed_by_the_latency() {
        let len = ptm_fade_len(RATE);
        let l = usize::from(len);
        let max = DryLine::new(RATE, len).max_delay();
        for d in [0, 1, 63, 480, 883, 1023, 1024, 2048, 4096, 12_547, max] {
            let input = ramp(d + 2 * l + 3000);
            for split in [1, 7, 100, 255, 480, 1056, 4096] {
                let mut line = DryLine::new(RATE, len);
                let mut out = Vec::with_capacity(input.len());
                for chunk in input.chunks(split) {
                    line.capture(chunk, d);
                    out.extend_from_slice(line.aligned(chunk.len()));
                }
                assert!(!line.moving() && line.delay() == d && line.want() == d);
                for i in d + 2 * l..input.len() {
                    assert_eq!(
                        out[i].to_bits(),
                        input[i - d].to_bits(),
                        "delay {d}, buffers of {split}: sample {i}"
                    );
                }
            }
        }
    }

    /// K11: a latency past the ring's reach is clamped to it, without a
    /// panic, and the line then delays by the most it can.
    #[test]
    fn a_latency_past_the_ring_is_clamped() {
        let len = ptm_fade_len(RATE);
        let l = usize::from(len);
        let max = DryLine::new(RATE, len).max_delay();
        let input = ramp(max + 2 * l + 3000);
        for asked in [max + 1, 10 * max, usize::MAX] {
            let mut line = DryLine::new(RATE, len);
            let mut out = Vec::with_capacity(input.len());
            for chunk in input.chunks(4096) {
                line.capture(chunk, asked);
                out.extend_from_slice(line.aligned(chunk.len()));
            }
            assert_eq!((line.want(), line.delay()), (max, max), "asked {asked}");
            for i in max + 2 * l..input.len() {
                assert_eq!(out[i], input[i - max], "asked {asked}: sample {i}");
            }
        }
    }

    /// One line run: `input` in `split`-frame calls, the latency `latency(i)`
    /// for the call starting at sample `i`. Its output.
    fn line_run(input: &[f32], split: usize, latency: impl Fn(usize) -> usize) -> Vec<f32> {
        let mut line = DryLine::new(RATE, ptm_fade_len(RATE));
        let mut out = Vec::with_capacity(input.len());
        for (call, chunk) in input.chunks(split).enumerate() {
            line.capture(chunk, latency(call * split));
            out.extend_from_slice(line.aligned(chunk.len()));
        }
        out
    }

    /// D4, a shrink: the old tap crossfades to the new one over L, so the
    /// difference is skipped, blended. A steady input doesn't change at all;
    /// a ramp's step is never more than its own plus `Δ/L` of it, and from L
    /// on the output is the input delayed by the new latency.
    #[test]
    fn a_shrink_crossfades_between_the_taps() {
        let l = fade();
        // Settled at 2048 well before it, and a multiple of every split.
        let edge = 4800;
        let latency = |i: usize| if i < edge { 2048 } else { 1024 };
        for split in [1, 100, 480, 2400] {
            let dc = line_run(&vec![1.0; edge + 3 * l], split, latency);
            assert!(
                dc[2048 + 2 * l..].iter().all(|&s| s == 1.0),
                "buffers of {split}: a steady input moved"
            );

            let input = ramp(edge + 3 * l);
            let out = line_run(&input, split, latency);
            // The ramp rises 1 a sample; the shrink adds 1024/L a sample.
            let bound = 1.0 + 1024.0 / f32::from(u16::try_from(l).unwrap());
            let step = max_step(out[edge - 1], &out[edge..]);
            assert!(step <= bound, "buffers of {split}: a step of {step}");
            assert!(step > 1.0, "buffers of {split}: no crossfade");
            for i in edge + l - 1..out.len() {
                assert_eq!(out[i], input[i - 1024], "buffers of {split}: sample {i}");
            }
        }
    }

    /// The gain the line puts on its taps through one growth from `from` to
    /// `to` that starts at sample `t0`, and the input index it is reading:
    /// `L` out from the old tap (its last sample at gain 0), `to − from`
    /// silent, `L` in on the new tap, then the new tap.
    fn growth(i: usize, t0: usize, from: usize, to: usize) -> (f32, usize) {
        let l = fade();
        let gain = |k: usize| f32::from(u16::try_from(k).unwrap()) / f32::from(ptm_fade_len(RATE));
        let gap = to - from;
        if i < t0 {
            (1.0, i - from)
        } else if i < t0 + l {
            (gain(l - (i - t0 + 1)), i - from)
        } else if i < t0 + l + gap {
            (0.0, 0)
        } else if i < t0 + 2 * l + gap {
            (gain(i - (t0 + l + gap) + 1), i - to)
        } else {
            (1.0, i - to)
        }
    }

    /// D4, a growth: the old tap fades out, the line is silent for the
    /// difference, the new tap fades in. With a steady input the output is
    /// that envelope exactly; with `x[i] = i + 1` every sample heard (gain
    /// over 0) names its source as `y / g`, and the sources only ever rise
    /// across the whole run: nothing is heard twice. Two runs: a new line
    /// given 2048, and one at 1024 given 2048.
    #[test]
    fn a_growth_pauses_and_repeats_nothing() {
        /// Where the second run grows, once settled at 1024: a multiple of
        /// every split below, so a call starts there.
        const EDGE: usize = 4800;
        // (the latency asked for from sample i, the envelope at i)
        type Run = (fn(usize) -> usize, fn(usize) -> (f32, usize));
        let runs: [Run; 2] = [
            (|_| 2048, |i| growth(i, 0, 0, 2048)),
            (
                |i| if i < EDGE { 1024 } else { 2048 },
                |i| {
                    if i < EDGE {
                        growth(i, 0, 0, 1024)
                    } else {
                        growth(i, EDGE, 1024, 2048)
                    }
                },
            ),
        ];
        let total = EDGE + 2048 + 2 * fade() + 1000;
        for (latency, envelope) in runs {
            for split in [1, 100, 480, 2400] {
                let dc = line_run(&vec![1.0; total], split, latency);
                for (i, &y) in dc.iter().enumerate() {
                    assert_eq!(y, envelope(i).0, "buffers of {split}: steady, sample {i}");
                }
                let input = ramp(total);
                let out = line_run(&input, split, latency);
                let mut last = None;
                for (i, &y) in out.iter().enumerate() {
                    let (g, source) = envelope(i);
                    if g == 0.0 {
                        assert_eq!(y.to_bits(), 0, "buffers of {split}: sample {i}");
                        continue;
                    }
                    let heard = y / g;
                    assert_eq!(heard, input[source], "buffers of {split}: sample {i}");
                    assert!(last < Some(source), "buffers of {split}: {source} twice");
                    last = Some(source);
                }
            }
        }
    }

    /// D4: a latency that changes while the line is moving is acted on at
    /// the first sample after that move ends, not by restarting it: a change
    /// at L/2 of a shrink, and one inside a growth's silence. Then the
    /// output is the input delayed by the last request.
    #[test]
    fn a_change_mid_move_waits_for_it() {
        let len = ptm_fade_len(RATE);
        let l = usize::from(len);
        let settle = 4096 + 2 * l + 1000;
        // (settled at, then asked for at `settle`, then at `settle + change`;
        // the sample the first move ends on)
        let cases = [
            (2048, 1024, l / 2, 3000, settle + l - 1),
            (1024, 2048, l + 500, 512, settle + 2 * l + 1024 - 1),
        ];
        for (from, first, change, second, ends) in cases {
            let latency = move |i: usize| match i {
                i if i < settle => from,
                i if i < settle + change => first,
                _ => second,
            };
            let total = ends + 2 * l + second + 3000;
            let input = ramp(total);
            // A sample a call, through the accessors.
            let mut line = DryLine::new(RATE, len);
            let mut one = Vec::with_capacity(total);
            for (i, x) in input.iter().enumerate() {
                line.capture(std::slice::from_ref(x), latency(i));
                one.extend_from_slice(line.aligned(1));
                if (settle..ends).contains(&i) {
                    assert!(line.moving() && line.delay() == first, "{from}: {i}");
                } else if i == ends {
                    assert!(!line.moving() && line.delay() == first, "{from}: the end");
                } else if i == ends + 1 {
                    assert!(line.moving() && line.delay() == second, "{from}: the next");
                }
            }
            assert!(!line.moving() && line.delay() == second);
            for i in total - 3000..total {
                assert_eq!(one[i], input[i - second], "{from}: sample {i}");
            }
            // Every split agrees, the calls starting where the latency changes.
            for split in [7, 100, 480] {
                let mut line = DryLine::new(RATE, len);
                let mut out = Vec::with_capacity(total);
                let bounds = [0, settle, settle + change, total];
                for pair in bounds.windows(2) {
                    for start in (pair[0]..pair[1]).step_by(split) {
                        let end = (start + split).min(pair[1]);
                        line.capture(&input[start..end], latency(start));
                        out.extend_from_slice(line.aligned(end - start));
                    }
                }
                assert!(
                    bits(&out) == bits(&one),
                    "{from}: buffers of {split} differ"
                );
            }
        }
    }

    /// D4: a new line starts at delay 0, whatever its first call asks for,
    /// and moves there by a growth: the session's chain is empty until the
    /// page sends one, so its first buffers report 0 anyway.
    #[test]
    fn a_new_line_starts_at_zero() {
        let len = ptm_fade_len(RATE);
        let mut line = DryLine::new(RATE, len);
        assert!(!line.moving() && line.delay() == 0 && line.want() == 0);
        line.capture(&[0.5], 1024);
        assert!(line.moving(), "the first call took its latency at once");
        assert_eq!((line.delay(), line.want()), (1024, 1024));
        assert_eq!(line.moving, Move::Out(1));
        assert_eq!(line.aligned(1), [0.5 * f32::from(len - 1) / f32::from(len)]);
    }

    // ---- the stage ----

    /// D8b: an edge toward the dry waits the latency the line was asked for
    /// at that edge, clamped as the line clamps it: 2048 waits 2048 samples,
    /// 0 none, a latency past the ring the ring's reach. A latency asked for
    /// after the edge doesn't move it. (With [`ALIGN_DRY`] off the line is
    /// asked for 0, so nothing waits.)
    #[test]
    fn the_wait_is_the_requested_latency() {
        let max = DryLine::new(RATE, ptm_fade_len(RATE)).max_delay();
        // (asked for until the edge, asked for after it, the wait)
        for (before, after, wait) in [
            (2048, 2048, 2048),
            (2048, 0, 2048),
            (0, 4096, 0),
            (10 * max, 10 * max, max),
        ] {
            let wait = if ALIGN_DRY { wait } else { 0 };
            let mut stage = PtmStage::new(RATE, false);
            stage.capture(&[0.25], before);
            let mut y = [0.5];
            stage.blend(&mut y, false);
            for i in 0..=wait {
                stage.capture(&[0.25], if i == 0 { before } else { after });
                let mut y = [0.5];
                stage.blend(&mut y, true);
                let moved = stage.gate.pos() > 0;
                assert_eq!(moved, i == wait, "asked {before} then {after}: sample {i}");
            }
        }
    }

    /// Thousands of buffers of 1 to 4096 frames with random flips and
    /// latencies (now and then past the ring), and quiet stretches where
    /// nothing changes. The oracle is the inputs alone:
    ///
    /// * a buffer that asks for the effects while `pos` is 0 is the wet, bit
    ///   for bit;
    /// * every sample's `pos`, read off the output, is in `0..=L` and moves
    ///   at most one step from the last (0 is the wet's bits, L the dry's);
    /// * after `Q = 3·dmax + 4L` quiet samples (dmax the longest clamped
    ///   latency since the last stretch, which bounds the line's moves and
    ///   the gate's wait) the output is the input delayed by the clamped
    ///   latency when bypassed, the wet when applied.
    ///
    /// And it reached every case it claims to check: each kind of line move,
    /// a cancelled wait, a fade turned round both ways, a clamp.
    #[test]
    #[allow(clippy::too_many_lines)] // one walk, its oracle inline
    fn seeded_walk() {
        // The voice is `noise_at(i, 0)`; the chain's output `2 + noise/2`,
        // which no dry sample comes near, so `pos` can be read off any
        // sample: `y = w + (d − w)·pos/L`.
        struct Walk {
            stage: PtmStage,
            at: usize,
            pos: usize,
        }
        let wet = |i: usize| 2.0 + 0.5 * noise_at(i, 7);
        let len = ptm_fade_len(RATE);
        let l = usize::from(len);
        let max = DryLine::new(RATE, len).max_delay();
        let clamp = |d: usize| if ALIGN_DRY { d.min(max) } else { 0 };
        let mut rng = XorShift(0x2545_f491_4f6c_dd1d);
        let latency_of = |rng: &mut XorShift| {
            if rng.below(20) == 0 {
                max - 100 + rng.below(201)
            } else {
                rng.below(4097)
            }
        };
        // One buffer; the output.
        let step = |walk: &mut Walk, frames: usize, bypass: bool, latency: usize| {
            let input: Vec<f32> = (walk.at..walk.at + frames)
                .map(|i| noise_at(i, 0))
                .collect();
            let w: Vec<f32> = (walk.at..walk.at + frames).map(wet).collect();
            let untouched = !bypass && walk.stage.gate.pos() == 0;
            walk.stage.capture(&input, latency);
            let dry = walk.stage.aligned(frames).to_vec();
            let mut y = w.clone();
            walk.stage.blend(&mut y, bypass);
            if untouched {
                assert!(
                    bits(&y) == bits(&w),
                    "applied at pos 0 touched, at {}",
                    walk.at
                );
            }
            for j in 0..frames {
                let ratio = f64::from(y[j] - w[j]) / f64::from(dry[j] - w[j]);
                #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                let pos = (ratio * f64::from(len)).round() as usize;
                assert!(pos <= l, "pos {pos} at {}", walk.at + j);
                assert!(
                    pos.abs_diff(walk.pos) <= 1,
                    "pos jumped to {pos} at {}",
                    walk.at + j
                );
                if pos == 0 {
                    assert_eq!(y[j].to_bits(), w[j].to_bits(), "at {}", walk.at + j);
                } else if pos == l {
                    assert_eq!(y[j].to_bits(), dry[j].to_bits(), "at {}", walk.at + j);
                }
                walk.pos = pos;
            }
            walk.at += frames;
            y
        };

        let mut walk = Walk {
            stage: PtmStage::new(RATE, false),
            at: 0,
            pos: 0,
        };
        let (mut bypass, mut latency, mut dmax) = (false, 0, 0);
        let mut seen = [false; 8]; // Cross, Out, Gap, In, cancel, turn ×2, clamp
        let mut quiet = 0;
        for b in 0..10_000 {
            if b % 97 == 96 {
                // A quiet stretch: one request, long enough for everything
                // in flight to finish, then checked.
                bypass = rng.next().is_multiple_of(2);
                latency = latency_of(&mut rng);
                dmax = dmax.max(clamp(latency));
                let q = 3 * dmax + 4 * l;
                let start = walk.at;
                while walk.at < start + q + 2000 {
                    let frames = (1 + rng.below(4096)).min(start + q + 2000 - walk.at);
                    let from = walk.at;
                    let y = step(&mut walk, frames, bypass, latency);
                    for (j, &s) in y.iter().enumerate() {
                        let i = from + j;
                        if i >= start + q {
                            let want = if bypass {
                                noise_at(i - clamp(latency), 0)
                            } else {
                                wet(i)
                            };
                            assert_eq!(s.to_bits(), want.to_bits(), "quiet {quiet}: {i}");
                        }
                    }
                }
                quiet += 1;
                dmax = clamp(latency);
                continue;
            }
            // Before the buffer: what it is about to do.
            let gate = walk.stage.gate;
            let next_bypass = if rng.below(10) < 3 { !bypass } else { bypass };
            if rng.below(10) < 2 {
                latency = latency_of(&mut rng);
                dmax = dmax.max(clamp(latency));
            }
            seen[4] |= gate.waiting() && !next_bypass;
            let mid = gate.pos() > 0 && gate.pos() < len;
            seen[5] |= mid && gate.to_dry && !next_bypass;
            seen[6] |= mid && !gate.to_dry && next_bypass && !gate.want_dry;
            seen[7] |= latency > max;
            bypass = next_bypass;
            step(&mut walk, 1 + rng.below(4096), bypass, latency);
            match walk.stage.line.moving {
                Move::Cross(_) => seen[0] = true,
                Move::Out(_) => seen[1] = true,
                Move::Gap(_) => seen[2] = true,
                Move::In(_) => seen[3] = true,
                Move::Settled => {}
            }
        }
        assert!(quiet > 100, "only {quiet} quiet stretches");
        assert!(seen.iter().all(|&s| s), "cases not reached: {seen:?}");
    }
}
