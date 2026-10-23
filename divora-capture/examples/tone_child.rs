//! A tone for `tests/launch_outside_tree.rs` to start as another process:
//! `tone_child <device id> <hz> <ms>` plays a −90 dBFS sine on that playback
//! device (by its endpoint id) for `ms` milliseconds, then exits. Far below
//! hearing, like `tests/desktop_self_tone.rs`'s, and it refuses a device
//! whose name looks like a virtual cable: a cable can lead into a live call.

#![allow(unsafe_code)]
#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_ptr_alignment // WASAPI render buffers are f32-aligned
)]

#[cfg(windows)]
fn main() {
    use std::f64::consts::TAU;
    use std::time::{Duration, Instant};

    use windows::core::HSTRING;
    use windows::Win32::Media::Audio::{
        IAudioClient, IAudioRenderClient, IMMDeviceEnumerator, MMDeviceEnumerator,
        AUDCLNT_SHAREMODE_SHARED, AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM,
        AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY, WAVEFORMATEX,
    };
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CLSCTX_ALL, COINIT_MULTITHREADED,
    };

    const AMP: f64 = 3.162e-5; // −90 dBFS

    let args: Vec<String> = std::env::args().collect();
    let [_, id, hz, ms] = args.as_slice() else {
        eprintln!("usage: tone_child <device id> <hz> <ms>");
        std::process::exit(2);
    };
    let hz: f64 = hz.parse().expect("hz");
    let ms: u64 = ms.parse().expect("ms");
    let devices = divora_capture::list_playback_devices().expect("devices");
    let dev = devices
        .iter()
        .find(|d| &d.id == id)
        .expect("no such device");
    let lower = dev.name.to_lowercase();
    assert!(
        !lower.contains("cable") && !lower.contains("virtual"),
        "refusing to play on {:?}: a virtual cable can lead into a call",
        dev.name
    );
    // SAFETY: COM on this thread; the render buffer is written within the
    // frames WASAPI handed out and released before the next GetBuffer.
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        let en: IMMDeviceEnumerator =
            CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).unwrap();
        let client: IAudioClient = en
            .GetDevice(&HSTRING::from(id.as_str()))
            .unwrap()
            .Activate(CLSCTX_ALL, None)
            .unwrap();
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
        let end = Instant::now() + Duration::from_millis(ms);
        let mut n = 0u64;
        while Instant::now() < end {
            let free = size - client.GetCurrentPadding().unwrap();
            if free > 0 {
                let p = render.GetBuffer(free).unwrap().cast::<f32>();
                let out = std::slice::from_raw_parts_mut(p, free as usize * 2);
                for f in out.chunks_mut(2) {
                    let v = (AMP * (TAU * hz * n as f64 / 48_000.0).sin()) as f32;
                    f[0] = v;
                    f[1] = v;
                    n += 1;
                }
                render.ReleaseBuffer(free, 0).unwrap();
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        let _ = client.Stop();
    }
}

#[cfg(not(windows))]
fn main() {}
