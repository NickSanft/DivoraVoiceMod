//! The loopback thread's per-packet work allocates and frees nothing.
//!
//! Windows keeps only about 30 ms of process-loopback audio for its reader;
//! a reader held up longer loses audio without being told. So the work the
//! `divora-loopback` thread does per packet — [`DesktopTap::ingest`] — must
//! never wait on the allocator, which the recording thread on the other side
//! of the rings uses constantly. Measured here with a per-thread counting
//! allocator (a process-wide count would pick up other tests running at the
//! same time), on every path: the first packet, the steady state, a silent
//! packet, a new stream, and full rings dropping a packet.

// The counting allocator is why this is its own binary.
#![allow(unsafe_code)]

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::time::Instant;

use divora_capture::desktop::{desktop_tap, DesktopPacket};

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
        // SAFETY: forwarded unchanged.
        unsafe { System.alloc(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        bump(&FREES);
        // SAFETY: forwarded unchanged.
        unsafe { System.dealloc(ptr, layout) }
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

#[test]
fn ingest_allocates_nothing() {
    let (mut tap, mut drain) = desktop_tap();
    let samples = [0.25f32; 960];
    let at = Instant::now();
    let packet = |silent: bool| DesktopPacket {
        samples: &samples,
        frames: 480,
        at,
        arrived: at,
        timed: true,
        silent,
        discontinuity: false,
        resumed: false,
    };
    // A packet whose samples are shorter than it says (never seen from
    // Windows): recorded as silence, never as a partial copy that would put
    // every later packet out of step.
    let short = DesktopPacket {
        samples: &samples[..100],
        ..packet(false)
    };
    let empty = DesktopPacket {
        frames: 0,
        ..packet(false)
    };

    // The counter is live: the same window around a real allocation is not
    // zero, so the zeros below are a measurement, not a dead hook.
    arm();
    drop(std::hint::black_box(vec![0u8; 16]));
    assert_eq!(disarm(), (1, 1), "the allocation probe is not counting");

    arm();
    tap.begin_stream();
    tap.ingest(&packet(false));
    tap.ingest(&packet(false));
    tap.ingest(&packet(true));
    tap.ingest(&short);
    tap.ingest(&empty);
    tap.begin_stream();
    tap.ingest(&packet(false));
    assert_eq!(disarm(), (0, 0), "ingesting allocated or freed");

    // Fill the rings, then measure the dropping path too.
    let mut n = 0;
    while drain.dropped_frames() == 0 {
        tap.ingest(&packet(false));
        n += 1;
        assert!(n < 100_000, "the rings never filled");
    }
    arm();
    tap.ingest(&packet(false));
    tap.ingest(&packet(true));
    assert_eq!(disarm(), (0, 0), "dropping allocated or freed");
    assert_eq!(drain.dropped_frames(), 3 * 480);

    // And what went in comes out (the drain side may allocate: it runs on
    // the recording thread).
    let mut out = Vec::new();
    drain.drain(at, 0.0, &mut out);
    assert!(out.len() > 6);
    assert!(out[2].frames.iter().all(|&s| s == 0.0), "the silent packet");
    assert_eq!(
        out[3].frames.len(),
        960,
        "the short packet keeps its length"
    );
    assert!(out[3].frames.iter().all(|&s| s == 0.0), "the short packet");
    assert!(out[4].frames.is_empty(), "the empty packet");
    // Every whole packet after them is intact: nothing slid.
    assert!(out[5..]
        .iter()
        .all(|c| c.frames.len() == 960 && c.frames.iter().all(|&s| (s - 0.25).abs() < 1e-9)));
}
