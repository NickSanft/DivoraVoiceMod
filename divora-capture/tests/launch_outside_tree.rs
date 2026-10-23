//! v1.57.0: what "Everything except Divora" makes of a program Divora
//! starts, checked on a real machine.
//!
//! The capture excludes Divora's process TREE. A link opened the old way (the
//! shell plugin's `open`: `ShellExecuteExW` from Divora's own process) starts
//! a browser that wasn't running as Divora's child, so its sound is left out
//! too. `open_outside_tree` starts it from the desktop's Explorer instead.
//! This starts a tone player (`examples/tone_child.rs`) both ways and listens
//! for each in "everything except this process", while checking on the
//! device itself that each tone really played.
//!
//! Local only, and opt-in twice over: `#[ignore]`d, and it needs
//! `DIVORA_TEST_DEVICE` set to part of a playback device's name. It refuses a
//! device named like a virtual cable — a cable can lead into a live call —
//! and the tones are −90 dBFS, far below hearing. Build the example first
//! (any `cargo test` does), then run
//! `DIVORA_TEST_DEVICE=SMSL cargo test -p divora-capture --test launch_outside_tree -- --include-ignored --nocapture`.

#![cfg(windows)]
#![allow(unsafe_code)]
#![allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]

use std::f64::consts::TAU;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use divora_capture::desktop::{DesktopCapture, DesktopSource};
use windows::core::{HSTRING, PCWSTR};
use windows::Win32::UI::Shell::{ShellExecuteExW, SHELLEXECUTEINFOW};

const AMP: f64 = 3.162e-5; // −90 dBFS, what tone_child plays
/// How long each tone plays: a settle, then two 2 s captures, with room.
const PLAY_MS: u64 = 6_000;

/// The tone player, built beside this test (examples are built by `cargo test`).
fn tone_child() -> PathBuf {
    let exe = std::env::current_exe().expect("test exe");
    let dir = exe.parent().and_then(|d| d.parent()).expect("target dir");
    let path = dir.join("examples").join("tone_child.exe");
    assert!(
        path.is_file(),
        "build the example first: {}",
        path.display()
    );
    path
}

/// How the shell plugin's `open` starts a link today (`open` 5.3's
/// `that_detached`): `ShellExecuteExW` from this process, no verb, no mask.
fn start_from_here(exe: &str, args: &str) {
    let file = HSTRING::from(exe);
    let params = HSTRING::from(args);
    let mut info = SHELLEXECUTEINFOW {
        cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
        lpFile: PCWSTR(file.as_ptr()),
        lpParameters: PCWSTR(params.as_ptr()),
        nShow: 0, // SW_HIDE: a console the user needn't see
        ..Default::default()
    };
    // SAFETY: `info` and the strings it points to outlive the call.
    unsafe { ShellExecuteExW(&raw mut info) }.expect("ShellExecuteExW");
}

/// The amplitude at `hz` in `left` (48 kHz), by correlation.
fn level_at(left: &[f32], hz: f64) -> f64 {
    let (mut re, mut im) = (0.0, 0.0);
    for (k, &s) in left.iter().enumerate() {
        let ph = TAU * hz * k as f64 / 48_000.0;
        re += f64::from(s) * ph.cos();
        im += f64::from(s) * ph.sin();
    }
    2.0 * (re * re + im * im).sqrt() / left.len().max(1) as f64
}

/// The tone's level, and the loudest of its neighbours a few Hz either side:
/// other apps playing show up in both, the tone only in the first.
fn tone_level(left: &[f32], hz: f64) -> (f64, f64) {
    let neighbours = [-37.0, -23.0, -11.0, 13.0, 29.0, 41.0]
        .iter()
        .map(|d| level_at(left, hz + d))
        .fold(0.0, f64::max);
    (level_at(left, hz), neighbours)
}

/// Two seconds of `source`'s left channel.
fn capture(source: &DesktopSource) -> Vec<f32> {
    let mut cap = DesktopCapture::start(source).expect("desktop capture");
    let mut drain = cap.take_drain().expect("drain");
    std::thread::sleep(Duration::from_millis(2_000));
    let mut chunks = Vec::new();
    drain.drain(Instant::now(), 0.0, &mut chunks);
    drop(cap);
    chunks
        .iter()
        .flat_map(|c| c.frames.chunks(2).map(|lr| lr[0]))
        .skip(4_800)
        .collect()
}

/// Start the player with `start`, let it settle, then measure its tone in
/// "everything except this process" and on the device. Waits for it to end.
fn measure(
    how: &str,
    hz: f64,
    device: &str,
    start: impl FnOnce(&str, &str),
) -> ((f64, f64), (f64, f64)) {
    let began = Instant::now();
    start(
        tone_child().to_str().expect("utf-8 path"),
        &format!("\"{device}\" {hz} {PLAY_MS}"),
    );
    std::thread::sleep(Duration::from_millis(800));
    let everything = tone_level(&capture(&DesktopSource::Everything), hz);
    let on_device = tone_level(&capture(&DesktopSource::Device(device.to_owned())), hz);
    eprintln!(
        "{how}: {hz} Hz (tone, loudest neighbour) everything-except-us ({:.3e}, {:.3e}), \
         on the device ({:.3e}, {:.3e})",
        everything.0, everything.1, on_device.0, on_device.1
    );
    let left = Duration::from_millis(PLAY_MS + 500).saturating_sub(began.elapsed());
    std::thread::sleep(left);
    (everything, on_device)
}

#[test]
#[ignore = "plays inaudible tones on a real device; set DIVORA_TEST_DEVICE and run locally"]
fn a_program_explorer_starts_is_heard_and_one_we_start_is_not() {
    if std::env::var_os("CI").is_some() {
        return;
    }
    let Ok(want) = std::env::var("DIVORA_TEST_DEVICE") else {
        eprintln!("DIVORA_TEST_DEVICE not set: nothing played, nothing checked");
        return;
    };
    assert!(
        divora_capture::process_loopback_available(),
        "this Windows has no process loopback: nothing to check"
    );
    let devices = divora_capture::list_playback_devices().expect("devices");
    let dev = devices
        .iter()
        .find(|d| d.name.contains(&want))
        .unwrap_or_else(|| panic!("no playback device named like {want:?}"));
    let lower = dev.name.to_lowercase();
    assert!(
        !lower.contains("cable") && !lower.contains("virtual"),
        "refusing to play on {:?}: a virtual cable can lead into a call",
        dev.name
    );
    eprintln!("playing −90 dBFS tones on {:?}", dev.name);

    let (ours, ours_device) = measure("started from here", 1_111.5, &dev.id, start_from_here);
    let (explorers, explorers_device) =
        measure("started by Explorer", 1_333.5, &dev.id, |exe, args| {
            divora_capture::open_outside_tree(exe, Some(args), divora_capture::Show::Hidden)
                .expect("open_outside_tree");
        });

    // Both tones really played: the device carries each, well above its
    // neighbours (after the device's volume).
    for (how, (tone, neighbours)) in [("ours", ours_device), ("Explorer's", explorers_device)] {
        assert!(
            tone > 1e-8 && tone > 10.0 * neighbours,
            "{how} tone never reached {:?}: {tone:.3e} vs {neighbours:.3e}",
            dev.name
        );
    }
    // Started from here, the player is in this process's tree: left out.
    let (tone, neighbours) = ours;
    assert!(
        tone < AMP / 20.0 && tone < 3.0 * neighbours.max(1e-9),
        "a program this process started was heard: {tone:.3e} vs {neighbours:.3e}"
    );
    // Started by Explorer, it isn't: heard, before the device's volume, at
    // about the level it plays.
    let (tone, neighbours) = explorers;
    assert!(
        tone > AMP / 3.0 && tone > 10.0 * neighbours,
        "a program Explorer started was left out: {tone:.3e} vs {neighbours:.3e}"
    );
}
