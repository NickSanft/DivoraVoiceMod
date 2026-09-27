//! The rate-mismatch path: correct audio, and no allocation in the callback.
//!
//! # The harness is the point of this file
//!
//! Every signal test here runs through [`TwoClock`], which drives a real
//! `HeapRb` from two independent clocks — an input device pushing at
//! `input_rate`, an output device pulling at `output_rate` — advancing
//! whichever is behind. That is not ceremony. A harness that hands the
//! callback a full slice every time reports the pre-fix code at
//! `out_frames = 512` as **perfectly clean** (peak 0.5000, zero spikes,
//! 440.00 Hz), because 512 is a multiple of the old 256-frame chunk so the
//! overshoot never fires. The two-clock answer for that same configuration is
//! thousands of spikes and a frequency estimate an octave and a half out,
//! because the defect that dominates is the *starvation* — the engine asking
//! for more input than the device produces — and a saturating harness feeds
//! that starvation away.
//!
//! So: if a future edit finds this harness inconvenient and swaps in a
//! simpler one, the suite will keep passing and stop meaning anything.
//!
//! # What is pinned
//!
//! * a resampled ramp stays a ramp — constant slope, no reorder, no drop;
//! * a resampled sine has no discontinuities and keeps its amplitude;
//! * the output is genuinely the input signal, by SNR against an analytic
//!   least-squares fit, not merely smooth (a spike test passes on silence);
//! * every callback writes exactly the frames the device asked for;
//! * zero allocations and zero frees per callback, through the real
//!   `MonoResampler` methods the engine calls;
//! * nothing accumulates: queue depth and latency are flat over 60 s of audio,
//!   where the old code's backlog grew linearly to 32 s;
//! * the monitor chain, which is fed BY the output callback, stays bounded too;
//! * `reset` leaves nothing of the previous session behind.
//!
//! # What is NOT pinned
//!
//! * that a real cpal callback calls these methods. There is no audio device in
//!   CI; [`the_engine_still_calls_the_resampler_the_tested_way`] checks that
//!   seam at the source level and that is all that stands behind it.
//! * resampler quality as DSP — passband ripple, stopband rejection, phase.
//!   That is rubato's contract, and pinning a filter response here would fail
//!   on a rubato upgrade for no defect of ours.
//! * any configuration whose input demand exceeds `MAX_FRAMES_PER_CALLBACK`.
//!   [`a_steep_downsample_lowers_the_output_count_rather_than_underfeeding`]
//!   asserts that as a known *limit*, not as working.
//! * anything in the output callback other than the resample step. The
//!   allocation zeros here say nothing about the voice converter or the
//!   soundboard drain; see `rt_chain_swap_allocations.rs` for what those two
//!   still do.
//!
//! # Mutation record
//!
//! Every test here was checked by putting an original defect back at its real
//! site. Caught: the over-ask on top of the real need (3 tests), the fixed
//! 256-frame chunk (8), an allocation in `run` (1), a cushion that never
//! primes (3), and a `reset` that forgets rubato's own history (1).
//!
//! One mutation survives, and it is worth knowing why rather than fixing:
//! restoring the zero-fill of a short pop changes nothing, because with the
//! cushion in place the pop is never short — measured 0 under-pops across 841
//! served callbacks over nine seconds of audio. It is unreachable code, not an
//! untested branch, and the invariant that makes it unreachable is itself
//! pinned: [`the_steady_state_primes_once_and_then_pads_nothing`] asserts
//! `underruns() == 0`, and that assertion fails the moment the cushion stops
//! priming. Restoring the zero-fill *and* removing the cushion — which
//! together are the code as it shipped before v1.51.2 — fails two tests.

// The counting allocator is why this is its own binary.
#![allow(unsafe_code)]
#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::f64::consts::TAU;

use divora_core::audio::{resample_pop, resample_render, MonoResampler, RingCushion};
use ringbuf::traits::{Consumer, Observer, Producer, Split};
use ringbuf::HeapRb;

/// Matches `RING_BUFFER_FRAMES` in `audio::engine`.
const RING_FRAMES: usize = 8192;
/// Matches `MAX_FRAMES_PER_CALLBACK` in `audio::engine`.
const MAX_FRAMES: usize = 4096;

// ---------------------------------------------------------------------------
// Counting allocator. Per-thread, for the reasons spelled out in
// `rt_chain_swap_allocations.rs`: a process-global counter picks up other
// tests running in parallel and any background thread's work.
// ---------------------------------------------------------------------------

struct Counting;

thread_local! {
    static ARMED: Cell<bool> = const { Cell::new(false) };
    static ALLOCS: Cell<usize> = const { Cell::new(0) };
    static FREES: Cell<usize> = const { Cell::new(0) };
}

fn bump(counter: &'static std::thread::LocalKey<Cell<usize>>) {
    if ARMED.try_with(Cell::get).unwrap_or(false) {
        let _ = counter.try_with(|c| c.set(c.get() + 1));
    }
}

// SAFETY: every call forwards to the system allocator unchanged; the flag and
// counters are const-initialised `Cell`s with no destructors, so nothing here
// allocates or re-enters.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        bump(&ALLOCS);
        System.alloc(layout)
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        bump(&FREES);
        System.dealloc(ptr, layout);
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

fn arm() {
    ALLOCS.with(|c| c.set(0));
    FREES.with(|c| c.set(0));
    ARMED.with(|a| a.set(true));
}

fn disarm() -> (usize, usize) {
    ARMED.with(|a| a.set(false));
    (ALLOCS.with(Cell::get), FREES.with(Cell::get))
}

// ---------------------------------------------------------------------------
// Signals
// ---------------------------------------------------------------------------

/// A ramp: `n` as a float. Resampling a ramp gives another ramp, so any
/// reorder, drop or duplicate shows up as a break in the slope.
fn ramp(n: u64) -> f32 {
    n as f32
}

/// Half-scale sine at `hz`.
fn sine(n: u64, rate: u32, hz: f64) -> f32 {
    (n as f64 * hz * TAU / f64::from(rate)).sin() as f32 * 0.5
}

// ---------------------------------------------------------------------------
// The two-clock harness
// ---------------------------------------------------------------------------

type Prod = <HeapRb<f32> as Split>::Prod;
type Cons = <HeapRb<f32> as Split>::Cons;

/// Per-callback census, so a test can assert on behaviour and not just output.
#[derive(Default, Debug)]
struct Census {
    /// Frames the resampler asked for.
    asked: Vec<usize>,
    /// Frames the ring actually supplied.
    got: Vec<usize>,
    /// Frames written to the device.
    wrote: Vec<usize>,
    /// Ring occupancy just before the pop.
    ring: Vec<usize>,
    /// Samples the input device produced but the ring could not hold.
    dropped: usize,
    /// Callbacks that wrote silence while the cushion refilled.
    filling: usize,
}

struct TwoClock {
    prod: Prod,
    cons: Cons,
    r: Option<MonoResampler>,
    cushion: RingCushion,
    input_rate: u32,
    output_rate: u32,
    out_frames: usize,
    /// Simulated seconds on each clock.
    t_in: f64,
    t_out: f64,
    /// Input samples generated so far, i.e. the signal's own sample index.
    n_in: u64,
    census: Census,
    out: Vec<f32>,
}

impl TwoClock {
    fn new(input_rate: u32, output_rate: u32, out_frames: usize) -> Self {
        let (prod, cons) = HeapRb::<f32>::new(RING_FRAMES).split();
        let r = if input_rate == output_rate {
            None
        } else {
            Some(MonoResampler::new(input_rate, output_rate, MAX_FRAMES).expect("construct"))
        };
        Self {
            prod,
            cons,
            r,
            cushion: RingCushion::default(),
            input_rate,
            output_rate,
            out_frames,
            t_in: 0.0,
            t_out: 0.0,
            n_in: 0,
            census: Census::default(),
            out: Vec::new(),
        }
    }

    /// The input device's callback: one 10 ms block at `input_rate`.
    fn input_tick(&mut self, sig: &impl Fn(u64, u32) -> f32) {
        let frames = self.input_rate as usize / 100;
        for _ in 0..frames {
            let v = sig(self.n_in, self.input_rate);
            self.n_in += 1;
            if self.prod.try_push(v).is_err() {
                self.census.dropped += 1;
            }
        }
        self.t_in += 0.01;
    }

    /// The output device's callback — the REAL `resample_pop` and
    /// `resample_render` the engine calls, not a copy of them. That is the
    /// whole reason those two are exported: the sizing arithmetic was the
    /// defect, so a test that reimplemented it would prove nothing.
    fn output_tick(&mut self) {
        let mut mono = [0f32; MAX_FRAMES];
        self.census.ring.push(self.cons.occupied_len());
        let (block, serving) = resample_pop(
            &mut self.cons,
            self.r.as_mut(),
            &mut self.cushion,
            self.out_frames,
            &mut mono,
        );
        if !serving {
            self.census.filling += 1;
        }
        let asked = self
            .r
            .as_ref()
            .map_or(self.out_frames, MonoResampler::prepared_need);
        let mut output_mono = [0f32; MAX_FRAMES];
        let wrote = resample_render(
            self.r.as_mut(),
            serving,
            &mono,
            block,
            &mut output_mono[..self.out_frames],
        );
        // Everything the DEVICE received, including the silence a short or
        // skipped buffer leaves behind. Collecting only `wrote` would hide a
        // stutter completely: the ramp would stay continuous and the SNR clean
        // while the user heard a gap.
        self.out.extend_from_slice(&output_mono[..wrote]);
        for _ in wrote..self.out_frames {
            self.out.push(0.0);
        }
        self.census.asked.push(asked);
        self.census.got.push(block);
        self.census.wrote.push(wrote);
        self.t_out += self.out_frames as f64 / f64::from(self.output_rate);
    }

    /// Output samples to skip before measuring: the priming silence plus the
    /// filter's lead-in.
    fn measure_from(&self) -> usize {
        let priming = self.census.filling * self.out_frames;
        (priming + self.output_rate as usize / 20).min(self.out.len() / 2)
    }

    /// Run `seconds` of audio, advancing whichever device clock is behind.
    fn run(&mut self, seconds: f64, sig: &impl Fn(u64, u32) -> f32) {
        while self.t_out < seconds {
            if self.t_in <= self.t_out {
                self.input_tick(sig);
            } else {
                self.output_tick();
            }
        }
    }

    fn queued(&self) -> usize {
        self.cons.occupied_len()
    }
}

// ---------------------------------------------------------------------------
// Measurements
// ---------------------------------------------------------------------------

/// The six rate pairs worth covering. Both directions of the pair that
/// actually happens on Windows, two integer ratios, and two non-integer ones.
const PAIRS: [(u32, u32); 6] = [
    (44_100, 48_000),
    (48_000, 44_100),
    (96_000, 48_000),
    (48_000, 16_000),
    (16_000, 48_000),
    (44_100, 96_000),
];

/// 480 is the real-world WASAPI shared-mode 10 ms buffer at 48 kHz; 441 the
/// same at 44.1 kHz. 512 is the control: a multiple of the old 256-frame
/// chunk, where the overshoot defect never fired, so it proves this suite
/// catches the starvation and not only the overshoot. 383 is prime.
const OUT_FRAMES: [usize; 4] = [480, 441, 512, 383];

/// Largest absolute step between adjacent samples, ignoring a lead-in.
fn worst_step(x: &[f32], skip: usize) -> f32 {
    x.windows(2)
        .skip(skip)
        .map(|w| (w[1] - w[0]).abs())
        .fold(0.0, f32::max)
}

fn peak(x: &[f32], skip: usize) -> f32 {
    x.iter().skip(skip).fold(0.0_f32, |a, b| a.max(b.abs()))
}

/// SNR in dB of `x` against a least-squares fit of `hz`, i.e. "how much of
/// this is the tone we put in". Amplitude and phase are solved for, so a
/// group delay costs nothing; anything else — silence spliced mid-stream,
/// output folded back into input, a wrong rate — lands in the residual.
///
/// Returns `f64::NEG_INFINITY` for an all-zero signal rather than dividing by
/// its power, so a silent output cannot read as a clean pass.
fn tone_snr_db(samples: &[f32], rate: u32, hz: f64, skip: usize) -> f64 {
    let y = &samples[skip.min(samples.len())..];
    if y.is_empty() {
        return f64::NEG_INFINITY;
    }
    let omega = hz * TAU / f64::from(rate);
    // Normal equations for `amp_sin * sin + amp_cos * cos`.
    let (mut ss, mut cc, mut sc, mut ys, mut yc) = (0.0, 0.0, 0.0, 0.0, 0.0);
    for (i, &v) in y.iter().enumerate() {
        let (sin, cos) = ((i as f64 * omega).sin(), (i as f64 * omega).cos());
        ss += sin * sin;
        cc += cos * cos;
        sc += sin * cos;
        ys += f64::from(v) * sin;
        yc += f64::from(v) * cos;
    }
    let det = ss * cc - sc * sc;
    if det.abs() < 1e-9 {
        return f64::NEG_INFINITY;
    }
    let amp_sin = (ys * cc - yc * sc) / det;
    let amp_cos = (yc * ss - ys * sc) / det;
    let mut sig = 0.0;
    let mut res = 0.0;
    for (i, &v) in y.iter().enumerate() {
        let fit = amp_sin * (i as f64 * omega).sin() + amp_cos * (i as f64 * omega).cos();
        sig += fit * fit;
        res += (f64::from(v) - fit) * (f64::from(v) - fit);
    }
    if sig <= 0.0 {
        return f64::NEG_INFINITY;
    }
    if res <= 0.0 {
        return f64::INFINITY;
    }
    10.0 * (sig / res).log10()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// A resampled ramp is still a ramp. Its slope is `input_rate / output_rate`
/// and it never varies — so a reordered, dropped or duplicated sample is a
/// visible break, which is what makes this the reorder detector.
///
/// On the pre-fix code the first break landed at output sample 480, the very
/// first callback boundary.
#[test]
fn a_resampled_ramp_keeps_a_constant_slope() {
    for (inr, outr) in PAIRS {
        for ofr in OUT_FRAMES {
            let mut h = TwoClock::new(inr, outr, ofr);
            h.run(0.5, &|n, _| ramp(n));
            let slope = f64::from(inr) / f64::from(outr);
            // Skip the priming silence and the filter's lead-in, where the
            // ramp is still building out of zeros.
            let skip = h.measure_from().max(4096).min(h.out.len() / 2);
            let steps: Vec<f64> = h.out[skip..]
                .windows(2)
                .map(|w| f64::from(w[1] - w[0]))
                .collect();
            assert!(!steps.is_empty(), "{inr}->{outr} ofr={ofr}: no output");
            let worst = steps
                .iter()
                .map(|d| (d - slope).abs())
                .fold(0.0_f64, f64::max);
            assert!(
                worst < 0.05,
                "{inr}->{outr} ofr={ofr}: slope should be {slope:.4} everywhere, \
                 worst deviation {worst:.4} — a sample was reordered, dropped or \
                 duplicated"
            );
        }
    }
}

/// Every callback writes exactly what the device asked for. The old code wrote
/// short whenever its fixed chunk did not divide the device's frame count.
#[test]
fn every_callback_fills_the_device_buffer() {
    for (inr, outr) in PAIRS {
        for ofr in OUT_FRAMES {
            let mut h = TwoClock::new(inr, outr, ofr);
            h.run(0.5, &|n, r| sine(n, r, 440.0));
            // Callbacks that wrote nothing are the cushion filling, which is
            // allowed — but only at the start, and only a handful. Every
            // callback that writes at all must fill the buffer exactly.
            let wrong: Vec<(usize, usize)> = h
                .census
                .wrote
                .iter()
                .copied()
                .enumerate()
                .filter(|&(_, w)| w != 0 && w != ofr)
                .collect();
            assert!(
                wrong.is_empty(),
                "{inr}->{outr} ofr={ofr}: {} callbacks wrote a partial buffer, \
                 first {:?}",
                wrong.len(),
                wrong.first()
            );
            let filling = h.census.filling;
            assert!(
                filling <= 4,
                "{inr}->{outr} ofr={ofr}: {filling} callbacks wrote silence while \
                 the cushion refilled — it should prime once and hold"
            );
            let last_silent = h.census.wrote.iter().rposition(|&w| w == 0).unwrap_or(0);
            assert!(
                last_silent < 8,
                "{inr}->{outr} ofr={ofr}: still starving at callback {last_silent} \
                 of {}",
                h.census.wrote.len()
            );
        }
    }
}

/// The signal that comes out is the signal that went in. This is the assertion
/// that actually matters: a spike or peak check passes on silence, and the
/// pre-fix output measured **0.00 dB** here — the input tone explained none of
/// the output energy.
#[test]
fn the_output_is_genuinely_the_input_tone() {
    for (inr, outr) in PAIRS {
        for ofr in OUT_FRAMES {
            let mut h = TwoClock::new(inr, outr, ofr);
            h.run(0.5, &|n, r| sine(n, r, 440.0));
            let skip = h.measure_from();
            let snr = tone_snr_db(&h.out, outr, 440.0, skip);
            assert!(
                snr > 60.0,
                "{inr}->{outr} ofr={ofr}: only {snr:.2} dB of the output is the \
                 440 Hz tone we fed in"
            );
            let p = peak(&h.out, skip);
            assert!(
                (p - 0.5).abs() < 0.02,
                "{inr}->{outr} ofr={ofr}: peak {p:.4}, expected 0.5"
            );
            // A 440 Hz half-scale sine steps by at most 0.5*2*pi*440/rate
            // between samples; anything near 0.1 is a splice, not a slope.
            let step = worst_step(&h.out, skip);
            assert!(
                step < 0.1,
                "{inr}->{outr} ofr={ofr}: worst adjacent step {step:.4} — the \
                 stream is discontinuous"
            );
        }
    }
}

/// Nothing accumulates. The old code's input backlog grew linearly — 5.4 s of
/// audio after 10 s, 32 s after 60 s — and realloc-copied it inside the
/// callback as it went.
#[test]
fn nothing_accumulates_over_a_minute() {
    for (inr, outr) in [(44_100, 48_000), (48_000, 44_100), (96_000, 48_000)] {
        let mut h = TwoClock::new(inr, outr, 480);
        h.run(10.0, &|n, r| sine(n, r, 440.0));
        let at_10 = h.queued();
        h.run(60.0, &|n, r| sine(n, r, 440.0));
        let at_60 = h.queued();
        let ring_max = h.census.ring.iter().copied().max().unwrap_or(0);
        // A stationary queue, not a growing one: one device block of slack is
        // fine, ten seconds of audio is not.
        let slack = inr as usize / 50; // 20 ms
        assert!(
            at_60 <= slack,
            "{inr}->{outr}: {at_60} frames queued at 60 s ({:.2} s of audio); \
             was {at_10} at 10 s",
            at_60 as f64 / f64::from(inr)
        );
        assert!(
            at_60 <= at_10.max(slack),
            "{inr}->{outr}: queue grew from {at_10} to {at_60}"
        );
        assert!(
            ring_max < RING_FRAMES,
            "{inr}->{outr}: ring hit its {RING_FRAMES}-frame limit (peak {ring_max}), \
             so the input device's samples were being dropped"
        );
        assert_eq!(
            h.census.dropped, 0,
            "{inr}->{outr}: {} input samples never fit in the ring",
            h.census.dropped
        );
    }
}

/// The resample step allocates and frees nothing, measured around the real
/// methods the output callback calls.
#[test]
fn the_resample_step_allocates_nothing() {
    for (inr, outr) in PAIRS {
        for ofr in OUT_FRAMES {
            // A real ring, kept full by hand, and the real two functions the
            // callback runs — including the pop and the cushion, not just the
            // resampler.
            let (mut prod, mut cons) = HeapRb::<f32>::new(RING_FRAMES).split();
            let mut r = MonoResampler::new(inr, outr, MAX_FRAMES).expect("construct");
            let mut cushion = RingCushion::default();
            let mut mono = [0f32; MAX_FRAMES];
            let mut out = [0f32; MAX_FRAMES];
            let top_up = |prod: &mut Prod| {
                while prod.vacant_len() > 0 {
                    if prod.try_push(0.25).is_err() {
                        break;
                    }
                }
            };
            // Warm a round outside the window, so first-call effects are not
            // attributed to the steady state.
            top_up(&mut prod);
            let (b, sv) = resample_pop(&mut cons, Some(&mut r), &mut cushion, ofr, &mut mono);
            resample_render(Some(&mut r), sv, &mono, b, &mut out[..ofr]);

            let mut total = (0_usize, 0_usize);
            for _ in 0..500 {
                // Refilling the ring is the input device's job, not the
                // callback's, so it stays outside the armed window.
                top_up(&mut prod);
                arm();
                let (b, sv) = resample_pop(&mut cons, Some(&mut r), &mut cushion, ofr, &mut mono);
                resample_render(Some(&mut r), sv, &mono, b, &mut out[..ofr]);
                let (a, f) = disarm();
                total.0 += a;
                total.1 += f;
            }
            assert_eq!(
                total,
                (0, 0),
                "{inr}->{outr} ofr={ofr}: {} allocations and {} frees over 500 \
                 callbacks",
                total.0,
                total.1
            );
        }
    }
}

/// An underrun costs one buffer, not a growing backlog — and it is recorded
/// rather than hidden, because a session that climbs here is starved.
#[test]
fn an_underrun_is_bounded_and_counted() {
    let mut r = MonoResampler::new(44_100, 48_000, MAX_FRAMES).expect("construct");
    let native = vec![0.5_f32; MAX_FRAMES];
    let mut out = vec![0.0_f32; MAX_FRAMES];

    let need = r.prepare_within(480, MAX_FRAMES);
    assert_eq!(r.run(&native, need / 2, &mut out), 480);
    assert_eq!(r.underruns(), 1);

    // The next full round is clean again: nothing was queued, so nothing
    // carried the shortfall forward.
    let need = r.prepare_within(480, MAX_FRAMES);
    assert_eq!(r.run(&native, need, &mut out), 480);
    assert_eq!(
        r.underruns(),
        1,
        "a full round must not count as an underrun"
    );
}

/// A steep downsample needs more input than output, so a native budget it
/// cannot meet lowers the output count instead of quietly under-feeding. This
/// pins a known **limit**, not a working configuration: the engine's DSP block
/// is a fixed-size stack buffer, and 48 kHz → 16 kHz at a 4096-frame device
/// buffer wants 12 288 input frames.
#[test]
fn a_steep_downsample_lowers_the_output_count_rather_than_underfeeding() {
    let mut r = MonoResampler::new(48_000, 16_000, MAX_FRAMES).expect("construct");
    let need = r.prepare_within(MAX_FRAMES, MAX_FRAMES);
    assert!(
        need <= MAX_FRAMES,
        "asked for {need} native frames against a {MAX_FRAMES} budget"
    );
    assert!(
        r.prepared_out_frames() < MAX_FRAMES,
        "should have lowered the output count; kept {}",
        r.prepared_out_frames()
    );
    // And at a realistic device buffer the budget never binds.
    let need = r.prepare_within(480, MAX_FRAMES);
    assert_eq!(r.prepared_out_frames(), 480, "480 out needs {need} in");
}

/// `reset` must leave nothing of the previous session: rubato holds a sinc
/// history and a fractional read position, and before v1.51.2 `reset` cleared
/// neither, so the first audio of a new session was blended with the old one's
/// tail.
#[test]
fn reset_leaves_nothing_behind() {
    let mut r = MonoResampler::new(44_100, 48_000, MAX_FRAMES).expect("construct");
    let loud = vec![0.9_f32; MAX_FRAMES];
    let mut out = vec![0.0_f32; MAX_FRAMES];
    for _ in 0..8 {
        let need = r.prepare_within(480, MAX_FRAMES);
        r.run(&loud, need, &mut out);
    }
    r.reset();

    let silence = vec![0.0_f32; MAX_FRAMES];
    let need = r.prepare_within(480, MAX_FRAMES);
    let n = r.run(&silence, need, &mut out);
    let p = peak(&out[..n], 0);
    assert!(p < 1e-6, "{p:.6} of the previous session survived reset");
    assert_eq!(r.underruns(), 0, "reset should clear the underrun count");
}

/// The monitor stream is fed by the output callback, not by the input device,
/// so it is a second place the same arithmetic has to be right. With the two
/// ends disagreeing, the monitor ring either pins full — 32.6% of samples
/// dropped and the monitor permanently 186 ms behind — or starves.
#[test]
fn the_monitor_chain_stays_bounded_and_clean() {
    // Mic at 44.1 k, main output at 48 k, headphones at 96 k: three clocks,
    // and both streams resampling.
    let (input_rate, output_rate, monitor_rate) = (44_100_u32, 48_000_u32, 96_000_u32);
    let mut main = TwoClock::new(input_rate, output_rate, 480);
    let (mut mon_prod, mut mon_cons) = HeapRb::<f32>::new(RING_FRAMES).split();
    let mut mon_r =
        MonoResampler::new(input_rate, monitor_rate, MAX_FRAMES).expect("construct monitor");

    let mut mon_out: Vec<f32> = Vec::new();
    let mut mon_dropped = 0_usize;
    let mut mon_ring_max = 0_usize;
    let mut t_mon = 0.0_f64;
    let mon_frames = 480_usize;
    let sig = |n: u64, r: u32| sine(n, r, 440.0);

    while main.t_out < 2.0 {
        if main.t_in <= main.t_out {
            main.input_tick(&sig);
            continue;
        }
        // The main output callback: pop, and tap what it popped into the
        // monitor ring — exactly what `build_output_stream` does.
        let mut mono = [0f32; MAX_FRAMES];
        let asked = main
            .r
            .as_mut()
            .map_or(480, |r| r.prepare_within(480, MAX_FRAMES));
        let block = main.cons.pop_slice(&mut mono[..asked]);
        let mut scratch = [0f32; MAX_FRAMES];
        if let Some(r) = main.r.as_mut() {
            r.run(&mono, block, &mut scratch);
        }
        for &v in &mono[..block] {
            if mon_prod.try_push(v).is_err() {
                mon_dropped += 1;
            }
        }
        main.t_out += 480.0 / f64::from(output_rate);

        // The monitor callback, on its own clock.
        while t_mon < main.t_out {
            mon_ring_max = mon_ring_max.max(mon_cons.occupied_len());
            let mut m = [0f32; MAX_FRAMES];
            let want = mon_r.prepare_within(mon_frames, MAX_FRAMES);
            let got = mon_cons.pop_slice(&mut m[..want]);
            let mut o = [0f32; MAX_FRAMES];
            let wrote = mon_r.run(&m, got, &mut o);
            mon_out.extend_from_slice(&o[..wrote]);
            t_mon += mon_frames as f64 / f64::from(monitor_rate);
        }
    }

    assert_eq!(
        mon_dropped, 0,
        "{mon_dropped} samples could not fit in the monitor ring — the output \
         callback is producing faster than the monitor consumes"
    );
    assert!(
        mon_ring_max < RING_FRAMES,
        "monitor ring peaked at {mon_ring_max} of {RING_FRAMES}"
    );
    let skip = (monitor_rate as usize / 10).min(mon_out.len() / 2);
    let snr = tone_snr_db(&mon_out, monitor_rate, 440.0, skip);
    assert!(
        snr > 60.0,
        "the monitor mix is only {snr:.2} dB the tone we fed in"
    );
    let p = peak(&mon_out, skip);
    assert!((p - 0.5).abs() < 0.02, "monitor peak {p:.4}, expected 0.5");
}

/// Priming happens once, and after it nothing is padded.
///
/// This is the claim the fix rests on, so it is asserted rather than inferred
/// from the SNR: the cushion fills for two or three callbacks at session start
/// and then every buffer is served in full, with the ring's occupancy
/// stationary well below its limit. The old code's equivalent numbers were a
/// zero-fill on 499 of every 500 callbacks and a backlog that grew for ever.
#[test]
fn the_steady_state_primes_once_and_then_pads_nothing() {
    for (inr, outr) in PAIRS {
        for ofr in OUT_FRAMES {
            let mut h = TwoClock::new(inr, outr, ofr);
            h.run(3.0, &|n, r| sine(n, r, 440.0));

            assert!(
                h.census.filling <= 4,
                "{inr}->{outr} ofr={ofr}: {} callbacks wrote silence; priming \
                 should take two or three and then hold",
                h.census.filling
            );
            // All the silence is at the start, not sprinkled through.
            let last_silent = h.census.wrote.iter().rposition(|&w| w == 0).unwrap_or(0);
            assert!(
                last_silent <= h.census.filling + 1,
                "{inr}->{outr} ofr={ofr}: a buffer went silent at callback \
                 {last_silent} of {}, long after priming",
                h.census.wrote.len()
            );
            // And once serving, every round got everything it asked for, so the
            // resampler never had to pad a tail.
            let r = h.r.as_ref().expect("these pairs all resample");
            assert_eq!(
                r.underruns(),
                0,
                "{inr}->{outr} ofr={ofr}: {} rounds ran with a zero-filled tail",
                r.underruns()
            );
            // Stationary, which is the actual property — not "small", since a
            // 3:1 downsample legitimately needs three input frames per output
            // one and so carries a proportionally bigger cushion. The old code
            // failed this by growing without bound.
            let ring = &h.census.ring;
            let half = ring.len() / 2;
            let early = ring[..half].iter().copied().max().unwrap_or(0);
            let late = ring[half..].iter().copied().max().unwrap_or(0);
            let asked = h.census.asked.iter().copied().max().unwrap_or(1);
            assert!(
                late <= early + asked,
                "{inr}->{outr} ofr={ofr}: ring peak grew from {early} to {late} \
                 between the first and second half of the run"
            );
            assert!(
                late < RING_FRAMES,
                "{inr}->{outr} ofr={ofr}: ring reached {late} of {RING_FRAMES}"
            );
            assert_eq!(
                h.census.dropped, 0,
                "{inr}->{outr} ofr={ofr}: samples dropped"
            );
        }
    }
}

/// The seam. There is no audio device in CI, so nothing here proves a real
/// cpal callback runs the code above — this checks at the source level that
/// both callbacks still drive the resampler the way these tests do, and that
/// the two constructs this fix removed have not come back.
///
/// It is a grep, and it is the honest limit of what this file can claim.
#[test]
fn the_engine_still_calls_the_resampler_the_tested_way() {
    let src = include_str!("../src/audio/engine.rs");
    let pops = src.matches("let (block, serving) = resample_pop(").count();
    assert_eq!(
        pops, 2,
        "expected the output and monitor callbacks to each size their pop with \
         `resample_pop`, found {pops}"
    );
    let renders = src.matches("let written_out = resample_render(").count();
    assert_eq!(
        renders, 2,
        "expected both callbacks to render through `resample_render`, found {renders}"
    );
    assert!(
        !src.contains("push_input"),
        "`push_input` is gone: the resampler no longer queues between rounds"
    );
    assert!(
        !src.contains("for slot in &mut mono[popped..") && !src.contains("mono[zero_from.."),
        "the zero-fill of the shortfall is back — that is the defect that put \
         silence into a third of every DSP block"
    );
}
