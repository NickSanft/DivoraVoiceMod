//! Decode a file with Windows' own decoders — what Windows Media Player and
//! Films & TV play with — and report every frame and audio sample (v1.54.0
//! gate check).
//!
//!     cargo run -p divora-capture --release --example mf_decode -- file.mp4
//!
//! Reads the video to NV12 and the audio to PCM through a Media Foundation
//! source reader, counting samples, gaps in their times, and any error flag.
//! An example only: it links Media Foundation statically, which the app
//! itself never does.

// A measuring tool, not app code: one unsafe block of plain Media
// Foundation calls, held to the workspace's lints only loosely.
#![allow(unsafe_code, clippy::pedantic)]

#[cfg(windows)]
fn main() {
    use windows::core::HSTRING;
    use windows::Win32::Media::MediaFoundation::{
        IMFAttributes, IMFMediaType, IMFSourceReader, MFAudioFormat_PCM, MFCreateMediaType,
        MFCreateSourceReaderFromURL, MFMediaType_Audio, MFMediaType_Video, MFShutdown, MFStartup,
        MFVideoFormat_NV12, MFSTARTUP_FULL, MF_MT_MAJOR_TYPE, MF_MT_SUBTYPE,
        MF_SOURCE_READERF_ENDOFSTREAM, MF_SOURCE_READERF_ERROR,
        MF_SOURCE_READER_FIRST_AUDIO_STREAM, MF_SOURCE_READER_FIRST_VIDEO_STREAM, MF_VERSION,
    };
    use windows::Win32::System::Com::{CoInitializeEx, COINIT_MULTITHREADED};

    let path = std::env::args().nth(1).expect("a file");
    // SAFETY: an example: plain calls on live objects, results checked.
    unsafe {
        CoInitializeEx(None, COINIT_MULTITHREADED)
            .ok()
            .expect("COM");
        MFStartup(MF_VERSION, MFSTARTUP_FULL).expect("MFStartup");
        let reader: IMFSourceReader =
            MFCreateSourceReaderFromURL(&HSTRING::from(path.as_str()), None::<&IMFAttributes>)
                .expect("open the file");
        let set = |stream: u32, major, sub| {
            let t: IMFMediaType = MFCreateMediaType().expect("media type");
            t.SetGUID(&MF_MT_MAJOR_TYPE, &major).expect("major");
            t.SetGUID(&MF_MT_SUBTYPE, &sub).expect("sub");
            reader
                .SetCurrentMediaType(stream, None, &t)
                .expect("a decoder for this stream");
        };
        set(
            MF_SOURCE_READER_FIRST_VIDEO_STREAM.0 as u32,
            MFMediaType_Video,
            MFVideoFormat_NV12,
        );
        let audio = reader
            .GetCurrentMediaType(MF_SOURCE_READER_FIRST_AUDIO_STREAM.0 as u32)
            .is_ok();
        if audio {
            set(
                MF_SOURCE_READER_FIRST_AUDIO_STREAM.0 as u32,
                MFMediaType_Audio,
                MFAudioFormat_PCM,
            );
        }
        for (label, stream) in [
            ("video", MF_SOURCE_READER_FIRST_VIDEO_STREAM.0 as u32),
            ("audio", MF_SOURCE_READER_FIRST_AUDIO_STREAM.0 as u32),
        ] {
            if label == "audio" && !audio {
                println!("audio: none");
                continue;
            }
            let (mut n, mut errors, mut gaps, mut last_end) = (0u64, 0u64, 0u64, None::<i64>);
            let mut first = None;
            loop {
                let mut flags = 0u32;
                let mut ts = 0i64;
                let mut sample = None;
                if reader
                    .ReadSample(
                        stream,
                        0,
                        None,
                        Some(&raw mut flags),
                        Some(&raw mut ts),
                        Some(&raw mut sample),
                    )
                    .is_err()
                {
                    errors += 1;
                    break;
                }
                if flags & MF_SOURCE_READERF_ERROR.0 as u32 != 0 {
                    errors += 1;
                    break;
                }
                if let Some(s) = sample {
                    n += 1;
                    first.get_or_insert(ts);
                    let dur = s.GetSampleDuration().unwrap_or(0);
                    if let Some(end) = last_end {
                        if (ts - end).abs() > 10_000 {
                            gaps += 1;
                        }
                    }
                    last_end = Some(ts + dur);
                }
                if flags & MF_SOURCE_READERF_ENDOFSTREAM.0 as u32 != 0 {
                    break;
                }
            }
            #[allow(clippy::cast_precision_loss)]
            let secs = |t: Option<i64>| t.map_or(0.0, |t| t as f64 / 1e7);
            println!(
                "{label}: {n} samples decoded, first at {:.4} s, ends at {:.4} s, {gaps} gaps over 1 ms, {errors} errors",
                secs(first),
                secs(last_end)
            );
        }
        let _ = MFShutdown();
    }
}

#[cfg(not(windows))]
fn main() {}
