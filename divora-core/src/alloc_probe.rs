//! A counting allocator for the crate's own unit tests.
//!
//! The RT rule this crate lives under is that the audio callback allocates
//! nothing, and the only way to hold a claim like that is to count. Two
//! integration binaries already do — `tests/rt_chain_swap_allocations.rs` for
//! the command drains and `tests/rt_resampler.rs` for the device-rate bridge —
//! because a `#[global_allocator]` is per test binary and those need one.
//!
//! This one covers the cases that are only reachable from *inside* the crate:
//! `VoiceConverter`'s resampling phases, which are `pub(crate)` on purpose, and
//! whose queues are private. The alternative was a row of `#[doc(hidden)] pub`
//! test accessors on a production type, which is permanent public surface in
//! exchange for one test's convenience.
//!
//! Counting per **thread** is what makes this safe to put in the shared test
//! binary. Cargo runs tests in parallel, so a process-global counter would
//! attribute every other test's allocations — and any background thread's — to
//! whoever happened to be armed. That is not hypothetical: it is how the
//! voice-model case in `rt_chain_swap_allocations.rs` first appeared to fail,
//! picking up a `SetParam` free from two threads away.

#![cfg(test)]
// The counting allocator is the whole point of the file, and a `GlobalAlloc`
// impl cannot be written any other way.
#![allow(unsafe_code)]

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

pub(crate) struct Counting;

thread_local! {
    /// `const` init so the slots need no lazy allocation and no destructor —
    /// either would re-enter the allocator from inside the allocator.
    static ARMED: Cell<bool> = const { Cell::new(false) };
    static ALLOCS: Cell<usize> = const { Cell::new(0) };
    static FREES: Cell<usize> = const { Cell::new(0) };
}

/// `try_with` because TLS is gone during thread teardown, and an unwind out of
/// the allocator would abort the process.
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

    // `alloc_zeroed` and `realloc` are left at their defaults, which go through
    // the two above — so nothing escapes the count.
}

/// Start counting this thread's allocations.
pub(crate) fn arm() {
    ALLOCS.with(|c| c.set(0));
    FREES.with(|c| c.set(0));
    ARMED.with(|a| a.set(true));
}

/// Stop counting and report `(allocations, deallocations)`.
pub(crate) fn disarm() -> (usize, usize) {
    ARMED.with(|a| a.set(false));
    (ALLOCS.with(Cell::get), FREES.with(Cell::get))
}
