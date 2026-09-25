//! The audio callback must not allocate or free. This is the proof for the
//! chain-swap path — the one that used to do both, on every preset switch.
//!
//! It lives in `tests/` because a `#[global_allocator]` is per test binary,
//! and it is the ONLY test in this binary because the counters are
//! process-global: any other test running in parallel would have its
//! allocations counted here.
//!
//! Two things are pinned:
//!
//! * a chain replacement and a `Clear`, drained and applied the way the output
//!   callback does it, allocate and free **nothing**;
//! * the counter is live — proved in the same run by measuring what the code
//!   used to do in that same place (build the replacement inline, drop the
//!   chain it displaced) and requiring that to be non-zero. Without this a
//!   broken allocator hook would let the test pass by measuring nothing.

// The counting allocator is the whole point of the file.
#![allow(unsafe_code)]

use std::alloc::{GlobalAlloc, Layout, System};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::sync_channel;

use divora_core::dsp::{Displaced, DspCommand, DspEdit, EffectChain, EffectKind, EffectSpec};
use ringbuf::traits::{Consumer, Producer, Split};
use ringbuf::HeapRb;

/// Counts every allocation and deallocation while `ARMED`, and otherwise
/// stays out of the way.
struct Counting;

static ARMED: AtomicBool = AtomicBool::new(false);
static ALLOCS: AtomicUsize = AtomicUsize::new(0);
static FREES: AtomicUsize = AtomicUsize::new(0);

// SAFETY: every call is forwarded to the system allocator unchanged; the
// counters are atomics and touch no allocation state of their own.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if ARMED.load(Ordering::Relaxed) {
            ALLOCS.fetch_add(1, Ordering::Relaxed);
        }
        System.alloc(layout)
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        if ARMED.load(Ordering::Relaxed) {
            FREES.fetch_add(1, Ordering::Relaxed);
        }
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
    ALLOCS.store(0, Ordering::SeqCst);
    FREES.store(0, Ordering::SeqCst);
    ARMED.store(true, Ordering::SeqCst);
}

/// Stop counting and report `(allocations, deallocations)`.
fn disarm() -> (usize, usize) {
    ARMED.store(false, Ordering::SeqCst);
    (ALLOCS.load(Ordering::SeqCst), FREES.load(Ordering::SeqCst))
}

#[test]
fn a_chain_swap_and_a_clear_allocate_nothing_in_the_callback() {
    let specs = five_effects();

    // ---- the control thread's half: everything built up front ----------
    // The same bounded channel the engine uses. Bounded matters: an unbounded
    // `mpsc` frees its blocks on the RECEIVING side, which is the audio
    // thread.
    let (tx, rx) = sync_channel::<DspEdit>(8);
    let (mut graveyard, mut bin) = HeapRb::<Displaced>::new(4).split();
    let mut chain = EffectChain::from_specs(&specs);

    // Prepared exactly as `AudioEngine::send_dsp` prepares them.
    let replace = DspEdit::prepare(DspCommand::SetChain {
        specs: specs.clone(),
    })
    .expect("a SetChain always prepares");
    tx.send(replace).expect("the queue has room");
    tx.send(DspEdit::Clear).expect("the queue has room");

    // ---- the callback's half: measured ---------------------------------
    // Mirrors `drain_dsp_edits` in `audio::engine`.
    let mut drained = 0_usize;
    arm();
    while let Ok(edit) = rx.try_recv() {
        if let Some(displaced) = chain.apply(edit) {
            let _ = graveyard.try_push(displaced);
        }
        drained += 1;
    }
    let (allocs, frees) = disarm();

    assert_eq!(drained, 2, "both edits should have been drained");
    assert_eq!(
        allocs, 0,
        "the callback allocated {allocs} times applying a chain swap + a Clear"
    );
    assert_eq!(
        frees, 0,
        "the callback freed {frees} times applying a chain swap + a Clear"
    );
    assert!(chain.is_empty(), "Clear should have emptied the chain");

    // Both displaced chains went to the graveyard rather than being freed.
    let mut retired = 0_usize;
    while bin.try_pop().is_some() {
        retired += 1;
    }
    assert_eq!(
        retired, 2,
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
