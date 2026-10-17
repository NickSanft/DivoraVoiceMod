//! The directly driven H.264 encoder (v1.54.0), on any Windows machine — CI
//! included.
//!
//! CI has no GPU encoder, only Windows' own synchronous one, so that is what
//! drives [`EncoderSession`] here: 96 synthetic NV12 frames at 30 fps, a
//! keyframe every 30, one more forced at frame 45. The hardware encoder —
//! asynchronous, fed from the GPU — is the ignored test at the bottom, run by
//! hand on a desktop, and the gate (`examples/capture_spike.rs direct`).
//!
//! A machine without Media Foundation, or without an H.264 encoder, returns
//! early, saying so: the app reports "unavailable" there, not a failure.

#![cfg(windows)]

use divora_capture::annexb::has_idr;
use divora_capture::nv12::nv12_len;
use divora_capture::smoke::{
    api, software_encoders, ComScope, EncoderConfig, EncoderEngine, EncoderEvent, EncoderInput,
    EncoderSession, Submit,
};

/// Which way the test went, in the log even when it passes (see
/// `encoder_smoke.rs`).
fn say(line: &str) {
    use std::io::Write;
    let _ = writeln!(std::io::stderr(), "encoder session smoke: {line}");
}

const SIZE: (u32, u32) = (320, 240);
const FPS: u32 = 30;
const FRAMES: u64 = 96;
const FORCED: u64 = 45;

/// Frame `i`: a gradient that moves, so frames differ.
fn frame(i: u64) -> Vec<u8> {
    let (w, h) = (SIZE.0 as usize, SIZE.1 as usize);
    let mut v = vec![128u8; nv12_len(SIZE)];
    for y in 0..h {
        for x in 0..w {
            #[allow(clippy::cast_possible_truncation)] // wraps on purpose
            let luma = (x + y * 3 + i as usize * 7) as u8;
            v[y * w + x] = luma.clamp(16, 235);
        }
    }
    v
}

fn pts(i: u64) -> i64 {
    i64::try_from(i * 10_000_000 / u64::from(FPS)).unwrap()
}

#[test]
fn the_software_encoder_is_driven_directly() {
    let _com = ComScope::enter();
    let Ok(api) = api() else {
        say("skipped: Media Foundation does not load here");
        return;
    };
    let Ok(session) = api.session() else {
        say("skipped: MFStartup failed");
        return;
    };
    let (h264, _) = software_encoders(&session);
    if !h264 {
        say("skipped: no H.264 encoder is registered here");
        return;
    }
    let cfg = EncoderConfig {
        size: SIZE,
        fps: FPS,
        bitrate: 1_000_000,
        gop_frames: 30,
        low_latency: true,
        engine: EncoderEngine::Software,
    };
    let mut enc = EncoderSession::new(&session, None, &cfg).expect("build the software encoder");
    let info = enc.info().clone();
    say(&format!("{info:?}"));
    assert!(!info.is_async, "the inbox encoder is synchronous");
    assert!(info.force_key_supported);
    assert_eq!(
        info.gop_readback,
        Some(30),
        "the keyframe interval reads back"
    );
    let header = enc.sequence_header().cloned().expect("a sequence header");

    let mut events = Vec::new();
    for i in 0..FRAMES {
        enc.poll(&mut events).expect("poll");
        let bytes = frame(i);
        let submitted = enc
            .encode(
                &EncoderInput::Memory(&bytes),
                pts(i),
                pts(i + 1) - pts(i),
                i == FORCED,
            )
            .expect("encode");
        assert_eq!(submitted, Submit::Accepted, "frame {i}");
    }
    enc.drain(std::time::Duration::from_millis(500), &mut events)
        .expect("drain");

    let units: Vec<_> = events
        .iter()
        .filter_map(|e| match e {
            EncoderEvent::Unit(u) => Some(u),
            _ => None,
        })
        .collect();
    assert!(
        matches!(events.last(), Some(EncoderEvent::Drained)),
        "the drain finished"
    );
    assert_eq!(units.len() as u64, FRAMES, "one access unit per frame");
    for (k, u) in units.iter().enumerate() {
        assert_eq!(u.pts, pts(k as u64), "output {k} keeps its frame's time");
        assert!(u.dts.is_none_or(|d| d == u.pts), "no B-frames");
    }
    let idrs: Vec<u64> = units
        .iter()
        .enumerate()
        .filter(|(_, u)| has_idr(&u.data))
        .map(|(k, _)| k as u64)
        .collect();
    assert_eq!(idrs, vec![0, 30, 45, 75], "periodic and forced keyframes");
    // The output type's header is the first IDR's own.
    let first = &units[0].data;
    assert!(
        divora_capture::annexb::parameter_sets(first)
            .is_some_and(|p| divora_capture::annexb::same_parameter_sets(&p, &header)),
        "the sequence header matches the first IDR's SPS/PPS"
    );
    let c = enc.counters();
    assert_eq!(
        (c.not_accepting, c.force_key_failed, c.ignored_events),
        (0, 0, 0)
    );
    say(&format!(
        "encoded {} units, IDRs at {idrs:?}, {} bytes",
        units.len(),
        units.iter().map(|u| u.data.len()).sum::<usize>()
    ));
}

/// The real thing: the GPU's own encoder, asynchronous, fed from the input
/// pool, for 96 frames. Needs a desktop with a hardware H.264 encoder, so
/// CI skips it.
#[test]
#[ignore = "needs a hardware encoder: run with --ignored"]
fn the_hardware_encoder_is_driven_directly() {
    use divora_capture::smoke::{Gpu, InputPool, MfDevice};
    use std::time::{Duration, Instant};
    if std::env::var_os("CI").is_some() {
        return;
    }
    let _com = ComScope::enter();
    let api = api().expect("Media Foundation");
    let session = api.session().expect("MFStartup");
    let gpu = Gpu::new().expect("a Direct3D 11 device");
    let dev = MfDevice::new(api, &gpu).expect("a DXGI device manager");
    let size = (1920, 1080);
    let cfg = EncoderConfig {
        size,
        fps: FPS,
        bitrate: 8_000_000,
        gop_frames: 60,
        low_latency: true,
        engine: EncoderEngine::Hardware {
            vendor_id: gpu.vendor_id().ok(),
        },
    };
    let mut enc = EncoderSession::new(&session, Some(&dev), &cfg).expect("the hardware encoder");
    let info = enc.info().clone();
    say(&format!("{info:?}"));
    assert!(info.hardware && info.is_async && info.d3d11_aware);
    let pool = InputPool::new(api, &dev, size, FPS).expect("the input pool");
    say(&format!("pool bind flags {:#x}", pool.bind_flags));

    let mut events = Vec::new();
    let mut next = 0u64;
    let mut min_free = u32::MAX;
    let deadline = Instant::now() + Duration::from_secs(20);
    while next < FRAMES && Instant::now() < deadline {
        enc.poll(&mut events).expect("poll");
        if !enc.wants_input() {
            std::thread::sleep(Duration::from_millis(1));
            continue;
        }
        min_free = min_free.min(pool.free().expect("free count"));
        let Some(f) = pool.try_acquire().expect("acquire") else {
            std::thread::sleep(Duration::from_millis(1));
            continue;
        };
        let got = enc
            .encode(
                &EncoderInput::Gpu(&f.sample),
                pts(next),
                pts(next + 1) - pts(next),
                next == FORCED,
            )
            .expect("encode");
        if got == Submit::Accepted {
            next += 1;
        }
    }
    enc.drain(Duration::from_millis(500), &mut events)
        .expect("drain");
    let units: Vec<_> = events
        .iter()
        .filter_map(|e| match e {
            EncoderEvent::Unit(u) => Some(u),
            _ => None,
        })
        .collect();
    let idrs: Vec<u64> = units
        .iter()
        .filter(|u| has_idr(&u.data))
        .map(|u| u64::try_from(u.pts).unwrap() * u64::from(FPS) / 10_000_000)
        .collect();
    say(&format!(
        "{} units, IDRs at {idrs:?}, pool min free {min_free}, counters {:?}",
        units.len(),
        enc.counters()
    ));
    assert_eq!(units.len() as u64, FRAMES);
    // The count restarts at a forced keyframe (measured on NVIDIA): the
    // next periodic one would be at 105, past the end.
    assert_eq!(idrs, vec![0, FORCED], "the first frame and the forced one");
    assert!(units.iter().all(|u| u.dts.is_none_or(|d| d == u.pts)));
}
