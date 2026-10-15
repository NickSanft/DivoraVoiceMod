//! The MP4 writer, end to end, on any Windows machine — CI included.
//!
//! No screen and no GPU: synthetic NV12 frames (and, where an AAC encoder is
//! registered, a tone) go through the real [`TakeWriter`] in its memory-input
//! mode to whatever software encoder Windows has, and the file that comes out
//! is read back box by box. What CI cannot reach — Windows Graphics Capture,
//! the GPU colour converter, a hardware encoder — is covered by the ignored
//! test at the bottom, run by hand on a desktop.
//!
//! A machine without Media Foundation (Windows N, Server without the Media
//! Foundation feature) or without an H.264 encoder returns early, saying so:
//! that is a machine where the app reports "unavailable", not a failure.

#![cfg(windows)]

use std::path::{Path, PathBuf};

use divora_capture::nv12::nv12_len;
use divora_capture::smoke::{api, software_encoders, ComScope, TakeWriter};
use divora_capture::timeline::{AudioBlock, BLOCK_FRAMES, OUT_RATE};

/// A top-level (or child) MP4 box: its type and its payload.
fn boxes(mut bytes: &[u8]) -> Vec<([u8; 4], &[u8])> {
    let mut out = Vec::new();
    while bytes.len() >= 8 {
        let size32 = u32::from_be_bytes(bytes[0..4].try_into().unwrap());
        let kind: [u8; 4] = bytes[4..8].try_into().unwrap();
        let (header, size) = match size32 {
            0 => (8, bytes.len()),
            1 => (
                16,
                usize::try_from(u64::from_be_bytes(bytes[8..16].try_into().unwrap())).unwrap(),
            ),
            n => (8, n as usize),
        };
        assert!(size >= header && size <= bytes.len(), "bad box size");
        out.push((kind, &bytes[header..size]));
        bytes = &bytes[size..];
    }
    out
}

/// Which way the test went, in the test log even when it passes: libtest
/// captures `print!`/`eprint!` from passing tests, but not writes to the
/// stderr handle itself — and on CI "it encoded" and "it returned early" must
/// be told apart.
fn say(line: &str) {
    use std::io::Write;
    let _ = writeln!(std::io::stderr(), "encoder smoke: {line}");
}

fn temp_mp4(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("divora-{name}-{}.mp4", std::process::id()))
}

/// Write `frames` frames of a moving gradient, and a second-and-a-bit of
/// tone if `audio`.
fn write_take(path: &Path, size: (u32, u32), frames: u64, audio: bool) {
    let api = api().expect("checked by the caller");
    let session = api.session().expect("checked by the caller");
    let writer = TakeWriter::new(&session, None, path, size, 30, 1_000_000, audio)
        .expect("open a software sink writer");
    let (w, h) = (size.0 as usize, size.1 as usize);
    let mut frame = vec![128u8; nv12_len(size)];
    for i in 0..frames {
        for y in 0..h {
            for x in 0..w {
                #[allow(clippy::cast_possible_truncation)] // wraps on purpose
                let v = (x + y + i as usize * 4) as u8;
                frame[y * w + x] = v.clamp(16, 235);
            }
        }
        let pts = i * 10_000_000 / 30;
        let next = (i + 1) * 10_000_000 / 30;
        writer
            .write_memory(&frame, pts, next - pts)
            .expect("write a frame");
    }
    if audio {
        let blocks = frames * u64::from(OUT_RATE) / 30 / BLOCK_FRAMES as u64;
        for b in 0..blocks {
            let first = b * BLOCK_FRAMES as u64;
            let samples = (0..BLOCK_FRAMES as u64)
                .flat_map(|i| {
                    #[allow(clippy::cast_precision_loss)] // under a second of frames
                    let t = (first + i) as f64 / f64::from(OUT_RATE);
                    #[allow(clippy::cast_possible_truncation)] // within ±8000
                    let s = (8_000.0 * (std::f64::consts::TAU * 440.0 * t).sin()) as i16;
                    [s, s]
                })
                .collect();
            let pts = first * 10_000_000 / u64::from(OUT_RATE);
            let end = (first + BLOCK_FRAMES as u64) * 10_000_000 / u64::from(OUT_RATE);
            writer
                .write_audio(&AudioBlock {
                    pts,
                    duration: end - pts,
                    samples,
                })
                .expect("write an audio block");
        }
    }
    writer.finish().expect("finalize the MP4");
}

#[test]
fn the_software_writer_makes_a_readable_mp4() {
    let _com = ComScope::enter();
    let api = match api() {
        Ok(api) => api,
        Err(e) => {
            say(&format!(
                "skipped: Media Foundation does not load here ({e})"
            ));
            return;
        }
    };
    let session = match api.session() {
        Ok(s) => s,
        Err(e) => {
            say(&format!("skipped: MFStartup failed ({e})"));
            return;
        }
    };
    let (h264, aac) = software_encoders(&session);
    if !h264 {
        say("skipped: no H.264 encoder is registered here");
        return;
    }
    drop(session);

    // An awkward size on purpose: not a multiple of 16 high, as a 3440×1440
    // monitor scaled to 2560 wide is not.
    let path = temp_mp4("smoke");
    write_take(&path, (320, 178), 45, aac);
    let bytes = std::fs::read(&path).expect("the MP4 exists");
    let _ = std::fs::remove_file(&path);

    let top = boxes(&bytes);
    assert_eq!(&top[0].0, b"ftyp", "an MP4 starts with ftyp");
    assert!(top.iter().any(|(k, _)| k == b"mdat"), "no media data");
    let (_, moov) = top
        .iter()
        .find(|(k, _)| k == b"moov")
        .expect("no moov: the file was not finalized");
    let children = boxes(moov);
    let tracks = children.iter().filter(|(k, _)| k == b"trak").count();
    assert_eq!(
        tracks,
        1 + usize::from(aac),
        "one video track, plus audio when AAC exists"
    );
    let (_, mvhd) = children
        .iter()
        .find(|(k, _)| k == b"mvhd")
        .expect("no mvhd");
    // Version 0: version/flags, created, modified, then timescale, duration.
    assert_eq!(mvhd[0], 0, "a 32-bit mvhd");
    let timescale = u32::from_be_bytes(mvhd[12..16].try_into().unwrap());
    let duration = u32::from_be_bytes(mvhd[16..20].try_into().unwrap());
    let seconds = f64::from(duration) / f64::from(timescale);
    assert!(
        (seconds - 1.5).abs() < 0.1,
        "45 frames at 30 fps lasted {seconds:.3} s"
    );
    // v1.53.0: whether each track starts with an edit list — what decides
    // if players skip the AAC encoder's priming, and so whether the MP4's
    // audio lines up with the separate WAV files. Reported, not asserted:
    // the alignment itself is measured on the decoded file (MANUAL_TESTS).
    for (k, (_, trak)) in children.iter().filter(|(k, _)| k == b"trak").enumerate() {
        let edits = boxes(trak)
            .into_iter()
            .find(|(k, _)| k == b"edts")
            .and_then(|(_, edts)| boxes(edts).into_iter().find(|(k, _)| k == b"elst"))
            .map_or_else(
                || "no edit list".to_string(),
                |(_, elst)| format!("edit list {:02x?}", &elst[..elst.len().min(24)]),
            );
        say(&format!("track {k}: {edits}"));
    }
    say(&format!(
        "encoded {} bytes: {} track(s), {seconds:.3} s{}",
        bytes.len(),
        tracks,
        if aac {
            ""
        } else {
            "; no AAC encoder here, so no audio track"
        }
    ));
}

/// The real thing: Windows Graphics Capture of the main screen, the GPU
/// converter and whatever encoder the machine has, for two seconds. Needs a
/// desktop session, so CI skips it.
#[test]
#[ignore = "needs a desktop: run with --ignored"]
fn it_records_the_main_screen() {
    if std::env::var_os("CI").is_some() {
        return;
    }
    let capture = divora_capture::capture_status();
    assert!(capture.supported, "{:?}", capture.reason);
    let monitor = divora_capture::list_monitors().remove(0);
    let path = temp_mp4("capture");
    let stop = std::sync::atomic::AtomicBool::new(false);
    let stats = divora_capture::record_monitor(
        &monitor.id,
        &path,
        &divora_capture::RecordOptions::default(),
        &stop,
        Some(std::time::Duration::from_secs(2)),
        None,
        &mut (),
    )
    .expect("record");
    let bytes = std::fs::read(&path).expect("the MP4 exists");
    let _ = std::fs::remove_file(&path);
    assert!(boxes(&bytes).iter().any(|(k, _)| k == b"moov"));
    assert_eq!(stats.frames_written, 60);
    assert!(stats.encoder.is_some() || !stats.hardware);
}
