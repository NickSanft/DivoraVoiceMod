//! The core promise of "everything except Divora", checked on a real
//! machine: sound this process plays is not in its own desktop audio, while
//! recording the device it plays on does hear it.
//!
//! Local only, and opt-in twice over: `#[ignore]`d, and it needs
//! `DIVORA_TEST_DEVICE` set to part of a playback device's name (as Windows
//! shows it) to play on. It refuses any device named like a virtual cable —
//! a cable can lead into a live call — and plays a 1234.5 Hz tone at
//! −90 dBFS, far below hearing. Run it with
//! `DIVORA_TEST_DEVICE=SMSL cargo test -p divora-capture --test desktop_self_tone -- --include-ignored --nocapture`.

#![cfg(windows)]
// Playing the tone is COM, through the same bindings the crate uses.
#![allow(unsafe_code)]
#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_ptr_alignment // WASAPI render buffers are f32-aligned
)]

use std::f64::consts::TAU;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use divora_capture::desktop::{DesktopCapture, DesktopSource};
use windows::core::HSTRING;
use windows::Win32::Media::Audio::{
    IAudioClient, IAudioRenderClient, IMMDeviceEnumerator, MMDeviceEnumerator,
    AUDCLNT_SHAREMODE_SHARED, AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM,
    AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY, WAVEFORMATEX,
};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CLSCTX_ALL, COINIT_MULTITHREADED,
};

const TONE_HZ: f64 = 1_234.5;
const TONE_AMP: f64 = 3.162e-5; // −90 dBFS

/// Play the tone on the device `id` from a thread of this process until
/// `stop`.
fn play_tone(id: String, stop: Arc<AtomicBool>) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        let en: IMMDeviceEnumerator =
            CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).unwrap();
        let dev = en.GetDevice(&HSTRING::from(id.as_str())).unwrap();
        let client: IAudioClient = dev.Activate(CLSCTX_ALL, None).unwrap();
        let format = WAVEFORMATEX {
            wFormatTag: 3,
            nChannels: 2,
            nSamplesPerSec: 48_000,
            nAvgBytesPerSec: 48_000 * 8,
            nBlockAlign: 8,
            wBitsPerSample: 32,
            cbSize: 0,
        };
        client
            .Initialize(
                AUDCLNT_SHAREMODE_SHARED,
                AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM | AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY,
                1_000_000,
                0,
                &raw const format,
                None,
            )
            .unwrap();
        let render: IAudioRenderClient = client.GetService().unwrap();
        let size = client.GetBufferSize().unwrap();
        client.Start().unwrap();
        let mut n = 0u64;
        while !stop.load(Ordering::Acquire) {
            let free = size - client.GetCurrentPadding().unwrap();
            if free > 0 {
                let p = render.GetBuffer(free).unwrap().cast::<f32>();
                let out = std::slice::from_raw_parts_mut(p, free as usize * 2);
                for f in out.chunks_mut(2) {
                    let v = (TONE_AMP * (TAU * TONE_HZ * n as f64 / 48_000.0).sin()) as f32;
                    f[0] = v;
                    f[1] = v;
                    n += 1;
                }
                render.ReleaseBuffer(free, 0).unwrap();
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        let _ = client.Stop();
    })
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

/// The tone's level, and the loudest of its neighbours a few Hz either
/// side: other apps playing as well show up in both, the tone only in the
/// first.
fn tone_level(left: &[f32]) -> (f64, f64) {
    let neighbours = [-37.0, -23.0, -11.0, 13.0, 29.0, 41.0]
        .iter()
        .map(|d| level_at(left, TONE_HZ + d))
        .fold(0.0, f64::max);
    (level_at(left, TONE_HZ), neighbours)
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
        .skip(4_800) // the tone's own start
        .collect()
}

#[test]
#[ignore = "plays an inaudible tone on a real device; set DIVORA_TEST_DEVICE and run locally"]
fn divora_is_not_in_its_own_desktop_audio() {
    if std::env::var_os("CI").is_some() {
        return;
    }
    let Ok(want) = std::env::var("DIVORA_TEST_DEVICE") else {
        eprintln!("DIVORA_TEST_DEVICE not set: nothing played, nothing checked");
        return;
    };
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
    eprintln!("playing a −90 dBFS {TONE_HZ} Hz tone on {:?}", dev.name);
    let stop = Arc::new(AtomicBool::new(false));
    let player = play_tone(dev.id.clone(), Arc::clone(&stop));
    std::thread::sleep(Duration::from_millis(300));

    let on_device = tone_level(&capture(&DesktopSource::Device(dev.id.clone())));
    let everything = if divora_capture::process_loopback_available() {
        Some(tone_level(&capture(&DesktopSource::Everything)))
    } else {
        None
    };
    stop.store(true, Ordering::Release);
    player.join().unwrap();
    eprintln!("tone level (tone, loudest neighbour): on the device {on_device:?}");
    eprintln!("tone level (tone, loudest neighbour): everything-except-us {everything:?}");

    // The device itself carries it, well above its neighbours (after the
    // device's volume, so its level is whatever that makes it).
    assert!(
        on_device.0 > 1e-8 && on_device.0 > 10.0 * on_device.1,
        "the tone never reached {:?}",
        dev.name
    );
    // Everything-but-this-process doesn't. It is taken before the device's
    // volume, so the tone would be there at full level; whatever else the
    // PC is playing shows equally at the neighbouring frequencies.
    if let Some((tone, neighbours)) = everything {
        assert!(tone < TONE_AMP / 20.0, "heard ourselves: {tone:.3e}");
        assert!(
            tone < 3.0 * neighbours.max(1e-9),
            "heard ourselves: {tone:.3e} vs {neighbours:.3e}"
        );
    }
}
