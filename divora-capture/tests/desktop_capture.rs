//! Desktop audio from the real device, on a real machine.
//!
//! Ignored on CI, which has no audio devices: run it with
//! `cargo test -p divora-capture --test desktop_capture -- --include-ignored`.
//! It keeps no audio — it counts packets and frames and prints them.

#![cfg(windows)]
#![allow(clippy::cast_precision_loss)] // frame counts printed as seconds

use std::time::{Duration, Instant};

use divora_capture::desktop::{DesktopCapture, DesktopSource, OpenedSource};

fn on_ci() -> bool {
    std::env::var_os("CI").is_some()
}

/// Process loopback opens, delivers a packet stream in real time (Windows
/// sends one every 10 ms even when nothing plays), and stops cleanly.
#[test]
#[ignore = "needs an audio device; run locally with --include-ignored"]
fn everything_except_divora_delivers_in_real_time() {
    if on_ci() {
        return;
    }
    let available = divora_capture::process_loopback_available();
    eprintln!("process loopback available: {available}");
    let mut cap = DesktopCapture::start(&DesktopSource::Everything).expect("desktop capture");
    let opened = cap.opened().clone();
    eprintln!("opened: {opened:?}");
    let mut drain = cap.take_drain().expect("the drain");
    let start = Instant::now();
    std::thread::sleep(Duration::from_secs(2));
    let mut chunks = Vec::new();
    drain.drain(start, 0.0, &mut chunks);
    drop(cap);
    let frames: usize = chunks.iter().map(|c| c.frames.len() / 2).sum();
    let streams: std::collections::BTreeSet<u64> = chunks.iter().map(|c| c.stream).collect();
    let timed = chunks.iter().filter(|c| c.timed).count();
    eprintln!(
        "{} packets, {frames} frames ({:.2} s), {timed} timed, streams {streams:?}, dropped {}",
        chunks.len(),
        frames as f64 / 48_000.0,
        drain.dropped_frames()
    );
    if available {
        assert_eq!(opened, OpenedSource::Everything);
        // Two seconds at 48 kHz, give or take the start and the 30 ms queue.
        assert!(frames > 85_000, "only {frames} frames in two seconds");
        assert_eq!(timed, chunks.len(), "untimed packets");
        // Packets follow each other: each starts where the last ended.
        for w in chunks.windows(2) {
            let step = w[1].at - w[0].at;
            let len = (w[0].frames.len() / 2) as f64 / 48_000.0;
            assert!((step - len).abs() < 0.002, "a jump of {:.4} s", step - len);
        }
    }
    assert_eq!(drain.dropped_frames(), 0);
}

/// The default playback device opens, by its id, and closes; it sends
/// nothing while idle, so no packet count is expected.
#[test]
#[ignore = "needs an audio device; run locally with --include-ignored"]
fn a_playback_device_opens_by_id() {
    if on_ci() {
        return;
    }
    let devices = divora_capture::list_playback_devices().expect("devices");
    for d in &devices {
        eprintln!("{} {:?}", if d.is_default { "*" } else { " " }, d.name);
    }
    let default = devices
        .iter()
        .find(|d| d.is_default)
        .expect("a default device");
    let cap =
        DesktopCapture::start(&DesktopSource::Device(default.id.clone())).expect("desktop capture");
    assert_eq!(
        cap.opened(),
        &OpenedSource::Device {
            id: default.id.clone(),
            name: default.name.clone()
        }
    );
    std::thread::sleep(Duration::from_millis(300));
    assert!(!cap.is_lost());
    // A device that isn't there is an error, not a silent take.
    assert!(DesktopCapture::start(&DesktopSource::Device("{not-a-device}".into())).is_err());
}
