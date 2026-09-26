//! The audio callback must not allocate or free. This is the proof for the
//! chain-swap path — the one that used to do both, on every preset switch.
//!
//! It lives in `tests/` because a `#[global_allocator]` is per test binary.
//! The counters are per-thread, which is what makes the tests below safe to
//! run in parallel with each other and with the detached loader thread a
//! voice model starts — a process-global counter attributes every one of
//! those to whichever test happens to be armed.
//!
//! Every measurement runs a real drain — [`drain_dsp_edits`] or
//! [`drain_reactive_edits`], the functions the output callback calls — and
//! not a copy of one. That distinction is the whole value of the file: a copy
//! of the drain stays allocation-free for ever while the callback beside it
//! regresses, so a copy proves nothing about the callback.
//!
//! What is pinned:
//!
//! * a chain replacement and a `Clear` allocate and free **nothing**, and
//!   both displaced chains leave through the graveyard ring;
//! * installing a voice model allocates and frees nothing, and the model it
//!   displaces leaves the same way;
//! * a reactive config allocates and frees nothing either — including when
//!   the new route table is longer than the old one, which a copy would have
//!   grown into;
//! * `SetParam` frees exactly once — the owned key string it was sent. That
//!   is the one free left in the DRAIN, on the most frequent edit there is,
//!   and it is asserted rather than excused so it cannot grow quietly and so
//!   interning the keys will fail this test and take the comments claiming it
//!   with it;
//!
//! What is NOT pinned, and must not be read into the zeros above: the rest of
//! the output callback. `VoiceConverter::process` builds two sinc resamplers
//! in the callback after every preset switch and allocates per inference
//! chunk; `MonoResampler::process` allocates once per buffer when the device
//! rates differ; the soundboard drain frees a decoded clip on a Play or a
//! Stop. All three predate this file, all three are documented at their sites,
//! and none of them is in scope here — this binary measures the command and
//! config drains, with no resampler, no voice model session, no soundboard and
//! no inference in the context it builds.
//! * the counter is live — proved in the same run by measuring what the code
//!   used to do in that same place (build the replacement inline, drop the
//!   chain it displaced) and requiring that to be non-zero. Without this a
//!   broken allocator hook would let the test pass by measuring nothing.

// The counting allocator is the whole point of the file.
#![allow(unsafe_code)]

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::collections::HashMap;
use std::sync::mpsc::sync_channel;

use divora_core::audio::{drain_dsp_edits, drain_reactive_edits, ReadingTap};
use divora_core::dsp::{
    Displaced, DspCommand, DspEdit, EffectChain, EffectKind, EffectSpec, ModRoute,
    ReactiveModulator, ResolvedReactive, MODEL_RESOURCE_KEY,
};
use ringbuf::traits::{Consumer, Observer, Split};
use ringbuf::HeapRb;

/// Counts every allocation and deallocation made **by the arming thread**
/// while it is armed, and otherwise stays out of the way.
///
/// Both the flag and the counters are per-thread. Process-global ones are
/// wrong twice over: preparing a voice model spawns a detached loader thread
/// whose allocations would land in the measured window, and two of these
/// tests arming at once would each count the other's. Cargo runs tests in
/// parallel by default, so that second one is not hypothetical — it is how
/// the voice-model case below first "failed", picking up the one free from
/// `SetParam` two threads away.
struct Counting;

thread_local! {
    /// `const` init so the slots need no lazy allocation and no destructor —
    /// either would re-enter the allocator from inside the allocator.
    static ARMED: Cell<bool> = const { Cell::new(false) };
    static ALLOCS: Cell<usize> = const { Cell::new(0) };
    static FREES: Cell<usize> = const { Cell::new(0) };
}

/// Add one to a counter, but only while this thread is armed. `try_with`
/// because TLS is gone during thread teardown, and an unwind out of the
/// allocator would abort the process.
fn bump(counter: &'static std::thread::LocalKey<Cell<usize>>) {
    if ARMED.try_with(Cell::get).unwrap_or(false) {
        let _ = counter.try_with(|c| c.set(c.get() + 1));
    }
}

// SAFETY: every call is forwarded to the system allocator unchanged; the flag
// and counters are const-initialised `Cell`s with no destructors, so nothing
// here allocates or re-enters.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        bump(&ALLOCS);
        System.alloc(layout)
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        bump(&FREES);
        System.dealloc(ptr, layout);
    }

    // `alloc_zeroed` and `realloc` are left at their defaults, which go
    // through the two above — so nothing escapes the count.
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

fn spec(kind: EffectKind) -> EffectSpec {
    EffectSpec {
        kind,
        enabled: true,
        params: HashMap::new(),
    }
}

/// A chain with real allocating effects in it: STFT rings (pitch, formant),
/// comb buffers (reverb), harmonizer state.
fn five_effects() -> Vec<EffectSpec> {
    vec![
        spec(EffectKind::Gate),
        spec(EffectKind::Pitch),
        spec(EffectKind::Formant),
        spec(EffectKind::Reverb),
        spec(EffectKind::Harmonizer),
    ]
}

fn arm() {
    ALLOCS.with(|c| c.set(0));
    FREES.with(|c| c.set(0));
    ARMED.with(|a| a.set(true));
}

/// Stop counting and report `(allocations, deallocations)`.
fn disarm() -> (usize, usize) {
    ARMED.with(|a| a.set(false));
    (ALLOCS.with(Cell::get), FREES.with(Cell::get))
}

/// The callback's side of the world: the receiving end of the engine's
/// bounded command channel, the live chain, the graveyard ring and the tap.
struct Callback {
    rx: std::sync::mpsc::Receiver<DspEdit>,
    chain: EffectChain,
    graveyard: <HeapRb<Displaced> as Split>::Prod,
    bin: <HeapRb<Displaced> as Split>::Cons,
    reading: ReadingTap,
}

impl Callback {
    fn new(specs: &[EffectSpec]) -> (std::sync::mpsc::SyncSender<DspEdit>, Self) {
        // The same bounded channel the engine uses. Bounded matters: an
        // unbounded `mpsc` frees its blocks on the RECEIVING side, which is
        // the audio thread.
        let (tx, rx) = sync_channel::<DspEdit>(8);
        let (graveyard, bin) = HeapRb::<Displaced>::new(4).split();
        (
            tx,
            Self {
                rx,
                chain: EffectChain::from_specs(specs),
                graveyard,
                bin,
                reading: ReadingTap::default(),
            },
        )
    }

    /// Run the real drain with the counters on, and report what it cost.
    fn drain_measured(&mut self) -> (usize, usize) {
        arm();
        drain_dsp_edits(
            &self.rx,
            &mut self.chain,
            &mut self.graveyard,
            &self.reading,
        );
        disarm()
    }

    /// Run the real drain without measuring — for setting up a "before" state.
    fn drain_quiet(&mut self) {
        drain_dsp_edits(
            &self.rx,
            &mut self.chain,
            &mut self.graveyard,
            &self.reading,
        );
    }

    /// Empty the graveyard ring, freeing what the callback handed over. This
    /// is the graveyard thread's job; here it just counts.
    fn retire(&mut self) -> usize {
        let mut n = 0;
        while self.bin.try_pop().is_some() {
            n += 1;
        }
        n
    }
}

#[test]
fn a_chain_swap_and_a_clear_allocate_nothing_in_the_callback() {
    let specs = five_effects();
    let (tx, mut cb) = Callback::new(&specs);

    // ---- the control thread's half: everything built up front ----------
    // Prepared exactly as `AudioEngine::send_dsp` prepares them.
    let replace = DspEdit::prepare(DspCommand::SetChain {
        specs: specs.clone(),
    })
    .expect("a SetChain always prepares");
    tx.send(replace).expect("the queue has room");
    tx.send(DspEdit::Clear).expect("the queue has room");

    // ---- the callback's half: measured ---------------------------------
    let (allocs, frees) = cb.drain_measured();

    assert_eq!(
        allocs, 0,
        "the callback allocated {allocs} times applying a chain swap + a Clear"
    );
    assert_eq!(
        frees, 0,
        "the callback freed {frees} times applying a chain swap + a Clear"
    );
    assert!(cb.chain.is_empty(), "Clear should have emptied the chain");
    assert_eq!(
        cb.retire(),
        2,
        "the replaced chain and the cleared one must both reach the graveyard"
    );

    // ---- and the counter is live ---------------------------------------
    // What the old code did in the callback: build the replacement there and
    // let the assignment drop the chain it displaced. If this reads zero, the
    // zeros above mean nothing.
    let mut victim = EffectChain::from_specs(&specs);
    arm();
    let displaced_inline = std::mem::replace(&mut victim, EffectChain::from_specs(&specs));
    drop(displaced_inline);
    let (old_allocs, old_frees) = disarm();
    drop(victim);

    assert!(
        old_allocs > 0 && old_frees > 0,
        "the allocator hook counts nothing ({old_allocs} allocations, \
         {old_frees} deallocations for a five-effect chain built and dropped) \
         — the assertions above are then vacuous"
    );
}

/// Selecting a voice is the other edit that moves something heavy: an ONNX
/// session, plus the load that may still be in flight behind it.
#[test]
fn installing_a_voice_model_allocates_nothing_in_the_callback() {
    let specs = vec![spec(EffectKind::VoiceConvert)];
    let (tx, mut cb) = Callback::new(&specs);
    let index = cb
        .chain
        .index_of_kind(EffectKind::VoiceConvert, 0)
        .expect("the chain was built with one");

    let resource = |path: &str| {
        DspEdit::prepare(DspCommand::SetResource {
            index,
            key: MODEL_RESOURCE_KEY.to_string(),
            value: Some(path.to_string()),
        })
        .expect("the model key always prepares")
    };

    // A voice is already selected, installed through the real drain so the
    // effect holds exactly what it would in a session. The paths need not
    // exist: `start` spawns the load on the CONTROL thread either way, and
    // whether it finds a file changes nothing about who frees what.
    tx.send(resource("voice-a.onnx")).expect("room");
    cb.drain_quiet();
    assert_eq!(cb.retire(), 1, "the effect's initial model was displaced");

    // Now switch to another voice — the measured edit.
    tx.send(resource("voice-b.onnx")).expect("room");
    let (allocs, frees) = cb.drain_measured();

    assert_eq!(
        allocs, 0,
        "the callback allocated {allocs} times installing a voice model"
    );
    assert_eq!(
        frees, 0,
        "the callback freed {frees} times installing a voice model"
    );
    assert_eq!(
        cb.retire(),
        1,
        "the model it displaced must leave through the graveyard, not be \
         dropped in the callback"
    );
}

/// The known remaining defect, pinned at its exact size.
///
/// `DspCommand::SetParam` carries an owned `String` key, so applying one frees
/// it on the audio thread — once per slider tick, the most frequent edit there
/// is. The per-buffer modulation path avoids this entirely
/// (`EffectChain::set_param_at` takes `&str`); the command path does not.
///
/// Asserted rather than excused, for two reasons: it cannot grow without this
/// failing, and interning the keys will also fail it — which is the prompt to
/// go and delete the comments in `dsp::mod` and `audio::engine` that admit it.
#[test]
fn set_param_frees_its_key_and_that_is_the_only_free_left() {
    let specs = five_effects();
    let (tx, mut cb) = Callback::new(&specs);

    let edit = DspEdit::prepare(DspCommand::SetParam {
        index: 0,
        key: "threshold".to_string(),
        value: -40.0,
    })
    .expect("a SetParam always prepares");
    tx.send(edit).expect("the queue has room");

    let (allocs, frees) = cb.drain_measured();

    assert_eq!(allocs, 0, "applying a SetParam should allocate nothing");
    assert_eq!(
        frees, 1,
        "a SetParam should free exactly its key string ({frees} frees). More \
         means something new leaks into the callback; fewer means the keys are \
         interned now — good, so update `EffectChain::apply`'s doc, the drain \
         comment in `audio::engine` and `docs/ARCHITECTURE.md`, which all still \
         admit this one."
    );
    assert_eq!(
        cb.bin.occupied_len(),
        0,
        "a SetParam displaces nothing and should not touch the graveyard"
    );
}

/// The reactive drain sits one line above the DSP drain in the same callback,
/// and a preset switch sends one config through each. It used to free the
/// route table it was handed — and allocate as well, whenever the new table
/// was longer than the modulator's own.
#[test]
fn a_reactive_config_allocates_nothing_in_the_callback() {
    let specs = vec![spec(EffectKind::Pitch)];
    let mut chain = EffectChain::from_specs(&specs);
    let mut modulator = ReactiveModulator::new();
    let (mut graveyard, mut bin) = HeapRb::<Displaced>::new(8).split();

    let (tx, rx) = sync_channel::<ResolvedReactive>(8);
    let config = |routes: usize| ResolvedReactive {
        enabled: true,
        intensity: 1.0,
        floor_db: -50.0,
        ceil_db: -10.0,
        attack_ms: 10.0,
        hold_ms: 50.0,
        release_ms: 100.0,
        routes: (0..routes)
            .map(|_| ModRoute {
                kind: EffectKind::Pitch,
                nth: 0,
                key: "semitones",
                base: 0.0,
                depth: 2.0,
                min: -24.0,
                max: 24.0,
            })
            .collect(),
    };

    // A config is already in force, installed through the real drain.
    tx.send(config(4)).expect("room");
    drain_reactive_edits(&rx, &mut modulator, &mut chain, &mut graveyard);
    while bin.try_pop().is_some() {}

    // Now a longer table — the case a copy would have grown the `Vec` for.
    tx.send(config(12)).expect("room");
    arm();
    drain_reactive_edits(&rx, &mut modulator, &mut chain, &mut graveyard);
    let (allocs, frees) = disarm();

    assert_eq!(
        allocs, 0,
        "the callback allocated {allocs} times applying a reactive config"
    );
    assert_eq!(
        frees, 0,
        "the callback freed {frees} times applying a reactive config"
    );
    assert!(
        bin.try_pop().is_some(),
        "the route table it displaced must leave through the graveyard"
    );
}
