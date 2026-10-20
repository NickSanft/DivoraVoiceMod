//! v1.56.0: the voice mute — the state's encoding and the gain stage.
//!
//! The state is one engine-wide word, `seq << 1 | muted`, kept in
//! [`EngineState`](super::state::EngineState). Every change bumps `seq`, so
//! the frontend can order command answers against level ticks.
//!
//! The gain stage is [`MuteGate`]: one position `pos` in `0..=2·len` on a
//! two-segment linear path. The chain's input gets `min(pos, len)/len` and the
//! voice after loudness gets `max(pos − len, 0)/len`. Muting closes the send
//! first, then starves the chain; unmuting opens the chain first, then the
//! send. A toggle mid-fade just turns round.

use serde::Serialize;

/// Divora's own mute of the voice, as the frontend sees it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InputMute {
    pub muted: bool,
    /// Changes since launch. Of two states, the larger `seq` is newer.
    pub seq: u64,
}

/// One setter call: the state before and after it, from one atomic op.
/// `before == after` means the call changed nothing (a Set to the value it
/// already had). Not on the wire: the commands answer `after`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MuteChange {
    pub before: InputMute,
    pub after: InputMute,
}

/// What a setter asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MuteOp {
    Set(bool),
    Toggle,
}

/// Kill switch (risk K1): `false` makes [`MuteRun::feed`] a no-op, so only the
/// voice after the chain is zeroed and the chain keeps hearing the mic.
pub(crate) const STARVE_CHAIN: bool = true;

/// The largest `seq` the word can hold. Never reached.
const SEQ_MAX: u64 = u64::MAX >> 1;

/// The longest fade [`MuteGate`] takes; `2 * MAX_LEN` still fits a `u16`.
const MAX_LEN: u16 = 2048;

/// `seq << 1 | muted` → its parts.
pub(crate) fn unpack(word: u64) -> InputMute {
    InputMute {
        muted: word & 1 == 1,
        seq: word >> 1,
    }
}

/// The parts → `seq << 1 | muted`. `seq` saturates at 2^63 − 1.
pub(crate) fn pack(m: InputMute) -> u64 {
    (m.seq.min(SEQ_MAX) << 1) | u64::from(m.muted)
}

/// The word after `op`, or `None` when nothing changes (a Set to the value it
/// already has). A change always bumps `seq` by one. `seq` saturates at
/// 2^63 − 1 (never reached).
pub(crate) fn mute_transition(word: u64, op: MuteOp) -> Option<u64> {
    let now = unpack(word);
    let muted = match op {
        MuteOp::Set(muted) => muted,
        MuteOp::Toggle => !now.muted,
    };
    (muted != now.muted).then(|| {
        pack(InputMute {
            muted,
            seq: now.seq.saturating_add(1),
        })
    })
}

/// The fade length at `rate`: the power of two at or above 5 ms, in 64..=2048.
/// So 5 ms ≤ L/rate < 10 ms for every rate in `12_800..=409_600`; below that
/// L is 64 (longer than 5 ms), above it 2048 (shorter).
#[must_use]
pub(crate) fn mute_ramp_len(rate: u32) -> u16 {
    // `div_ceil`, not `/`: a floor puts rates just above 200·2^k under 5 ms.
    // At most 21.5 M, so `next_power_of_two` can't overflow.
    let five_ms = rate.div_ceil(200).max(1);
    u16::try_from(five_ms.next_power_of_two().clamp(64, u32::from(MAX_LEN))).unwrap_or(MAX_LEN)
}

/// The gain stage, per session, on the audio thread. `pos` is in `0..=2·len`:
/// 0 is silent, `2·len` is open.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct MuteGate {
    pos: u16,
    len: u16,
}

impl MuteGate {
    /// Starts settled where `muted` says: silent (0) or open (2·len).
    /// `len` is a [`mute_ramp_len`] value.
    #[must_use]
    pub(crate) fn new(len: u16, muted: bool) -> Self {
        debug_assert!(
            len.is_power_of_two() && (64..=MAX_LEN).contains(&len),
            "not a mute_ramp_len value: {len}"
        );
        // Kept in range in release too: 0 would divide by zero and more than
        // MAX_LEN would overflow `2 * len`.
        let len = len.clamp(1, MAX_LEN);
        Self {
            pos: if muted { 0 } else { 2 * len },
            len,
        }
    }

    /// Plan this buffer: remembers where it starts, then moves `pos` by up to
    /// `frames` steps toward the target. Both passes below use the plan, so
    /// their order can't change what they compute. Pass them a slice of
    /// `frames` samples; `frames` = 0 moves nothing.
    #[must_use]
    pub(crate) fn run(&mut self, muted: bool, frames: usize) -> MuteRun {
        let start = self.pos;
        let target = if muted { 0 } else { 2 * self.len };
        let steps = u16::try_from(frames).unwrap_or(u16::MAX);
        self.pos = if start < target {
            start.saturating_add(steps).min(target)
        } else {
            start.saturating_sub(steps).max(target)
        };
        MuteRun {
            start,
            target,
            len: self.len,
        }
    }

    /// Tests only: settled open or settled silent (`pos` is 0 or 2·len).
    #[cfg(test)]
    #[must_use]
    pub(crate) fn settled(self) -> bool {
        self.pos == 0 || self.pos == 2 * self.len
    }
}

/// One buffer's plan, from [`MuteGate::run`].
#[derive(Debug, Clone, Copy)]
pub(crate) struct MuteRun {
    start: u16,
    target: u16,
    len: u16,
}

impl MuteRun {
    /// The chain's input. Settled open → untouched. Settled silent → `+0.0`.
    /// Otherwise per sample: step, then gain `min(pos, len)/len`.
    /// A no-op while [`STARVE_CHAIN`] is off. A long mute leaves the chain's
    /// states decaying toward subnormals; the output callback flushes them
    /// to zero (`denormals`, K16), so the mute costs what a pause does.
    pub(crate) fn feed(self, voice: &mut [f32]) {
        if STARVE_CHAIN {
            self.scale(voice, u16::min);
        }
    }

    /// The voice after loudness. Settled open → untouched. Settled silent →
    /// `+0.0`. Otherwise per sample: step, then gain `max(pos − len, 0)/len`.
    pub(crate) fn mute(self, voice: &mut [f32]) {
        self.scale(voice, u16::saturating_sub);
    }

    /// Walks `pos` from `start` toward `target`, one step before each sample,
    /// and scales the sample by `gain(pos, len) / len`. A gain of 0 writes
    /// `+0.0` (never `x · 0`, which keeps a NaN); a gain of 1 leaves the
    /// sample as it is. `len` is a power of two, so every gain is exact.
    fn scale(self, voice: &mut [f32], gain: impl Fn(u16, u16) -> u16) {
        if self.start == self.target {
            if self.target == 0 {
                voice.fill(0.0);
            }
            return;
        }
        let step = 1.0 / f32::from(self.len);
        let mut pos = self.start;
        for sample in voice.iter_mut() {
            if pos < self.target {
                pos += 1;
            } else if pos > self.target {
                pos -= 1;
            }
            let k = gain(pos, self.len);
            if k == 0 {
                *sample = 0.0;
            } else if k < self.len {
                *sample *= f32::from(k) * step;
            }
        }
    }
}

#[cfg(test)]
#[allow(clippy::float_cmp)] // every gain is exact, so the outputs are compared exactly
mod tests {
    use super::{
        mute_ramp_len, mute_transition, pack, unpack, InputMute, MuteGate, MuteOp, MAX_LEN,
        SEQ_MAX, STARVE_CHAIN,
    };

    /// Design §7.1: a rate and the fade length it gets.
    const TABLE: [(u32, u16); 14] = [
        (8_000, 64),
        (11_025, 64),
        (16_000, 128),
        (22_050, 128),
        (24_000, 128),
        (32_000, 256),
        (44_100, 256),
        (48_000, 256),
        (88_200, 512),
        (96_000, 512),
        (176_400, 1024),
        (192_000, 1024),
        (384_000, 2048),
        (0, 64),
    ];

    /// Every fade length there is.
    const LENS: [u16; 6] = [64, 128, 256, 512, 1024, 2048];

    /// A signalling NaN. x86 `mulss` quiets it, so it is the one value that
    /// shows a multiply by 1.0: every other value survives `x · 1.0` intact.
    const SNAN: f32 = f32::from_bits(0x7f80_0001);

    /// One buffer through the stage as the engine runs it, with a passthrough
    /// chain between the two passes.
    fn stage(gate: &mut MuteGate, muted: bool, voice: &mut [f32]) {
        let run = gate.run(muted, voice.len());
        run.feed(voice);
        run.mute(voice);
    }

    /// The steepest a fade of DC 0.5 may be: 0.5 / L.
    fn slope(len: u16) -> f32 {
        0.5 / f32::from(len)
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

    fn bits(y: &[f32]) -> Vec<u32> {
        y.iter().map(|s| s.to_bits()).collect()
    }

    fn all_plus_zero(y: &[f32]) -> bool {
        y.iter().all(|s| s.to_bits() == 0)
    }

    /// A signal with no repeats and no zeros, so a bit compare means
    /// something.
    fn signal(n: usize) -> Vec<f32> {
        (0..n)
            .map(|i| {
                let k = u16::try_from(i % 997).unwrap();
                (f32::from(k) + 1.0) / 1000.0 - 0.5
            })
            .collect()
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
    }

    #[test]
    fn ramp_len_table() {
        for (rate, len) in TABLE {
            assert_eq!(mute_ramp_len(rate), len, "{rate} Hz");
        }
        for (rate, len) in [
            (12_800, 64),
            (12_801, 128),
            (25_600, 128),
            (25_601, 256),
            (409_600, 2048),
        ] {
            assert_eq!(mute_ramp_len(rate), len, "{rate} Hz");
        }
        // 5 ms <= L / rate < 10 ms, in integers, wherever a power of two in
        // 64..=2048 can give it.
        for rate in 12_800..=409_600_u32 {
            let len = u32::from(mute_ramp_len(rate));
            assert!(len.is_power_of_two(), "{rate} Hz: L {len}");
            assert!(200 * len >= rate, "{rate} Hz: L {len} is under 5 ms");
            assert!(100 * len < rate, "{rate} Hz: L {len} is 10 ms or more");
        }
        // Below that range the floor, above it the cap.
        for rate in 0..12_800 {
            assert_eq!(mute_ramp_len(rate), 64, "{rate} Hz");
        }
        for rate in [409_601, 768_000, u32::MAX] {
            assert_eq!(mute_ramp_len(rate), MAX_LEN, "{rate} Hz");
        }
    }

    #[test]
    fn transition_table() {
        let state = |seq, muted| InputMute { muted, seq };
        let ops = [MuteOp::Set(true), MuteOp::Set(false), MuteOp::Toggle];
        // Design §7.2: for each start, the state after Set(true), Set(false)
        // and Toggle; `None` is "nothing changes".
        let rows = [
            (
                state(0, false),
                [Some(state(1, true)), None, Some(state(1, true))],
            ),
            (
                state(1, true),
                [None, Some(state(2, false)), Some(state(2, false))],
            ),
            (
                state(7, false),
                [Some(state(8, true)), None, Some(state(8, true))],
            ),
            (
                state(SEQ_MAX, true),
                [
                    None,
                    Some(state(SEQ_MAX, false)),
                    Some(state(SEQ_MAX, false)),
                ],
            ),
        ];
        for (before, after) in rows {
            for (op, want) in ops.into_iter().zip(after) {
                assert_eq!(
                    mute_transition(pack(before), op).map(unpack),
                    want,
                    "{op:?} from {before:?}"
                );
            }
        }
        // Launch is unmuted at seq 0, and the word round-trips.
        assert_eq!(unpack(0), InputMute::default());
        for word in [0, 1, 2, 3, 14, 15, u64::MAX - 1, u64::MAX] {
            assert_eq!(pack(unpack(word)), word, "{word:#x}");
        }
        // A Set to the value it has moves nothing, `seq` included.
        for word in [0, 3, 14, u64::MAX] {
            let now = unpack(word);
            assert_eq!(mute_transition(word, MuteOp::Set(now.muted)), None);
        }
    }

    #[test]
    fn an_open_gate_is_bit_exact() {
        let odd = [
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
        for len in LENS {
            let mut gate = MuteGate::new(len, false);
            for _ in 0..3 {
                let (mut fed, mut muted) = (odd, odd);
                let run = gate.run(false, odd.len());
                run.feed(&mut fed);
                run.mute(&mut muted);
                assert_eq!(
                    bits(&fed),
                    bits(&odd),
                    "feed touched an open buffer (L {len})"
                );
                assert_eq!(
                    bits(&muted),
                    bits(&odd),
                    "mute touched an open buffer (L {len})"
                );
            }
            assert!(gate.settled());
        }
    }

    #[test]
    fn a_gate_built_muted_is_silent_from_its_first_sample() {
        for len in LENS {
            let mut gate = MuteGate::new(len, true);
            assert!(gate.settled());
            let mut input = signal(3 * usize::from(len));
            input[..6].copy_from_slice(&[
                -0.0,
                f32::NAN,
                f32::INFINITY,
                SNAN,
                f32::from_bits(1),
                0.5,
            ]);
            let (mut fed, mut muted) = (input.clone(), input);
            let run = gate.run(true, fed.len());
            run.feed(&mut fed);
            run.mute(&mut muted);
            // The chain's input too, unless the K1 kill switch is pulled.
            if STARVE_CHAIN {
                assert!(all_plus_zero(&fed), "feed let something through (L {len})");
            }
            assert!(
                all_plus_zero(&muted),
                "mute let something through (L {len})"
            );
        }
    }

    #[test]
    fn the_send_is_exactly_zero_l_samples_after_mute() {
        for (rate, _) in TABLE {
            let len = mute_ramp_len(rate);
            let l = usize::from(len);
            let mut gate = MuteGate::new(len, false);
            let mut y = vec![0.5_f32; 3 * l];
            stage(&mut gate, true, &mut y);
            // From open, the L-th sample after the edge is the first zero.
            assert!(y[l - 2] > 0.0, "{rate} Hz: silent before L");
            assert!(all_plus_zero(&y[l - 1..]), "{rate} Hz: not +0.0 from L on");
        }
    }

    #[test]
    fn no_step_larger_than_the_slope() {
        for (rate, _) in TABLE {
            let len = mute_ramp_len(rate);
            let mut gate = MuteGate::new(len, false);
            let mut prev = 0.5;
            for muted in [false, true, false, true, false] {
                let mut y = vec![0.5_f32; 3 * usize::from(len)];
                stage(&mut gate, muted, &mut y);
                let step = max_step(prev, &y);
                assert!(step <= slope(len), "{rate} Hz: a step of {step}");
                prev = y[y.len() - 1];
                // Each fade got where it was going, so the bound saw a whole one.
                assert_eq!(prev, if muted { 0.0 } else { 0.5 });
            }
        }
    }

    #[test]
    fn the_send_is_a_pure_linear_ramp() {
        for len in LENS {
            let l = usize::from(len);
            let at = |k: u16| 0.5 * f32::from(k) / f32::from(len);

            // Unmute: L zeros while the chain's input opens, then 0.5·k/L.
            let mut gate = MuteGate::new(len, true);
            let mut y = vec![0.5_f32; 3 * l];
            stage(&mut gate, false, &mut y);
            assert!(all_plus_zero(&y[..l]), "L {len}: the send opened early");
            for k in 1..=len {
                assert_eq!(y[l - 1 + usize::from(k)], at(k), "L {len}, k {k}");
            }
            assert!(y[2 * l..].iter().all(|&s| s == 0.5));

            // Mute: 0.5·k/L down to the first zero at L.
            let mut y = vec![0.5_f32; 3 * l];
            stage(&mut gate, true, &mut y);
            for (i, k) in (0..len).rev().enumerate() {
                assert_eq!(y[i], at(k), "L {len}, sample {i}");
            }
        }
    }

    #[test]
    fn a_toggle_mid_ramp_turns_round() {
        for len in [64, 256, 2048] {
            let l = usize::from(len);
            for from_muted in [false, true] {
                let rest = if from_muted { 0.0 } else { 0.5 };
                for offset in 1..2 * l {
                    let mut gate = MuteGate::new(len, from_muted);
                    let mut there = vec![0.5_f32; offset];
                    stage(&mut gate, !from_muted, &mut there);
                    let mut back = vec![0.5_f32; 2 * l];
                    stage(&mut gate, from_muted, &mut back);
                    let step = max_step(rest, &there).max(max_step(there[offset - 1], &back));
                    assert!(
                        step <= slope(len),
                        "L {len}, from muted {from_muted}, turned at {offset}: a step of {step}"
                    );
                    // It went back the way it came, no further.
                    assert!(gate.settled());
                    assert_eq!(back[2 * l - 1], rest);
                }
            }
        }
    }

    #[test]
    fn buffers_of_any_size_give_one_result() {
        let len = mute_ramp_len(48_000);
        // The flag holds for a whole buffer, so each phase is split on its
        // own; a 300-sample phase turns a fade round mid-way.
        let phases = [
            (true, 3000),
            (false, 5000),
            (true, 300),
            (false, 2000),
            (true, 9000),
        ];
        let run = |split: Option<usize>| {
            let mut gate = MuteGate::new(len, false);
            let mut out = Vec::new();
            for (muted, n) in phases {
                let mut y = signal(n);
                for chunk in y.chunks_mut(split.unwrap_or(n)) {
                    stage(&mut gate, muted, chunk);
                }
                out.extend(y);
            }
            bits(&out)
        };
        let whole = run(None);
        for split in [1, 7, 100, 255, 480, 4096] {
            assert!(run(Some(split)) == whole, "buffers of {split} differ");
        }
    }

    #[test]
    fn open_again_two_ramps_after_unmute() {
        for len in LENS {
            let l = usize::from(len);
            let mut gate = MuteGate::new(len, true);
            let input = signal(3 * l);
            let mut y = input.clone();
            stage(&mut gate, false, &mut y);
            assert!(gate.settled());
            assert_ne!(y[2 * l - 2], input[2 * l - 2], "L {len}: open before 2L");
            assert_eq!(bits(&y[2 * l - 1..]), bits(&input[2 * l - 1..]), "L {len}");
            // And the next buffer is left alone, odd values included.
            let mut next = [SNAN, f32::NAN, -0.0, f32::from_bits(1)];
            stage(&mut gate, false, &mut next);
            assert_eq!(
                bits(&next),
                bits(&[SNAN, f32::NAN, -0.0, f32::from_bits(1)])
            );
        }
    }

    #[test]
    fn nan_is_zero_once_silent() {
        let bad = [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, SNAN, -f32::NAN];
        for len in LENS {
            let l = usize::from(len);
            let input: Vec<f32> = bad.iter().copied().cycle().take(3 * l).collect();

            // Settled silent: both passes write +0.0, not `NaN · 0`.
            let mut gate = MuteGate::new(len, true);
            let (mut fed, mut muted) = (input.clone(), input.clone());
            let run = gate.run(true, fed.len());
            run.feed(&mut fed);
            run.mute(&mut muted);
            assert!(all_plus_zero(&muted), "L {len}: settled");
            if STARVE_CHAIN {
                assert!(all_plus_zero(&fed), "L {len}: settled, the chain's input");
            }

            // Fading: from the L-th sample after the edge.
            let mut gate = MuteGate::new(len, false);
            let mut y = input;
            stage(&mut gate, true, &mut y);
            assert!(all_plus_zero(&y[l - 1..]), "L {len}: the edge");
        }
    }

    #[test]
    fn feed_and_mute_share_one_plan() {
        let len = mute_ramp_len(48_000);
        let n = 2 * usize::from(len);
        // A plan that crosses both segments, so each pass has work to do.
        let mut gate = MuteGate::new(len, false);
        let run = gate.run(true, n);
        let input = signal(n);
        let (mut fed, mut muted) = (input.clone(), input.clone());
        run.feed(&mut fed);
        run.mute(&mut muted);
        let (mut fed_after, mut muted_first) = (input.clone(), input.clone());
        run.mute(&mut muted_first);
        run.feed(&mut fed_after);
        assert_eq!(bits(&fed), bits(&fed_after), "feed depends on the order");
        assert_eq!(
            bits(&muted),
            bits(&muted_first),
            "mute depends on the order"
        );
        assert_eq!(STARVE_CHAIN, bits(&fed) != bits(&input));
        assert_ne!(bits(&muted), bits(&input));
        // The passes read the plan; neither moves the gate.
        assert!(gate.settled());
    }

    #[test]
    fn seeded_toggle_walk() {
        for len in [64, 256, 2048] {
            let l = usize::from(len);
            let mut rng = XorShift(0x9e37_79b9_7f4a_7c15 ^ u64::from(len));
            let mut gate = MuteGate::new(len, false);
            let (mut muted, mut run_len, mut prev) = (false, 2 * l, 0.5_f32);
            let (mut zeros, mut opens, mut turns) = (0_usize, 0_usize, 0_usize);
            for _ in 0..10_000 {
                let frames = 1 + usize::try_from(rng.next() % 4096).unwrap();
                if rng.next().is_multiple_of(3) {
                    turns += usize::from(!gate.settled());
                    muted = !muted;
                    run_len = 0;
                }
                let mut y = vec![0.5_f32; frames];
                stage(&mut gate, muted, &mut y);
                assert!(
                    max_step(prev, &y) <= slope(len),
                    "L {len}: a step over the slope"
                );
                for &s in &y {
                    run_len += 1;
                    if muted && run_len >= l {
                        assert_eq!(s.to_bits(), 0, "L {len}: L muted samples, not +0.0");
                        zeros += 1;
                    } else if !muted && run_len >= 2 * l {
                        assert_eq!(
                            s.to_bits(),
                            0.5_f32.to_bits(),
                            "L {len}: 2L open, not exact"
                        );
                        opens += 1;
                    }
                }
                prev = y[frames - 1];
            }
            // The walk reached every case it claims to check.
            assert!(
                zeros > 0 && opens > 0 && turns > 0,
                "L {len}: {zeros} {opens} {turns}"
            );
        }
    }
}
