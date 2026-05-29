//! Decode a single audio file into a mono f32 buffer.
//!
//! Uses symphonia to handle WAV / MP3 / OGG-Vorbis / FLAC / Opus. The
//! returned `DecodedClip` keeps the file's native sample rate; the
//! mixer interpolates against the engine's rate at play time.

// Sample format conversion inherently loses precision; that's the
// price of decoding integer PCM into f32. Mute the lints for the
// whole module rather than per-arm.
#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::similar_names,
    clippy::semicolon_if_nothing_returned
)]

use std::fs::File;
use std::path::Path;
use std::sync::Arc;

use symphonia::core::audio::{AudioBufferRef, Signal};
use symphonia::core::codecs::DecoderOptions;
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::{MediaSourceStream, MediaSourceStreamOptions};
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

use super::SoundboardError;

/// One decoded clip — mono f32 samples at the file's native rate.
#[derive(Debug, Clone)]
pub struct DecodedClip {
    /// Mono samples. Cloned into voices via the inner `Arc` without copy.
    pub samples: Arc<Vec<f32>>,
    /// File's native sample rate.
    pub sample_rate: u32,
    /// Duration in seconds (`samples` / `sample_rate`).
    pub duration_secs: f32,
}

/// Decode `path` into a `DecodedClip`. Stereo files are mixed to mono;
/// integer formats are normalised to [-1, 1].
pub fn decode_clip(path: &Path) -> Result<DecodedClip, SoundboardError> {
    let display = path.display().to_string();
    let extension = path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_lowercase)
        .unwrap_or_default();

    let file = File::open(path).map_err(|e| SoundboardError::Decode {
        path: display.clone(),
        message: format!("open: {e}"),
    })?;
    let mss = MediaSourceStream::new(Box::new(file), MediaSourceStreamOptions::default());

    let mut hint = Hint::new();
    if !extension.is_empty() {
        hint.with_extension(&extension);
    }

    let probed = symphonia::default::get_probe()
        .format(
            &hint,
            mss,
            &FormatOptions::default(),
            &MetadataOptions::default(),
        )
        .map_err(|e| SoundboardError::Decode {
            path: display.clone(),
            message: format!("probe: {e}"),
        })?;

    let mut format = probed.format;
    let track = format
        .default_track()
        .ok_or_else(|| SoundboardError::Decode {
            path: display.clone(),
            message: "no default audio track".into(),
        })?;
    let codec_params = track.codec_params.clone();
    let sample_rate = codec_params.sample_rate.unwrap_or(48_000);
    let track_id = track.id;

    let mut decoder = symphonia::default::get_codecs()
        .make(&codec_params, &DecoderOptions::default())
        .map_err(|e| SoundboardError::Decode {
            path: display.clone(),
            message: format!("make decoder: {e}"),
        })?;

    let mut samples: Vec<f32> = Vec::new();

    loop {
        let packet = match format.next_packet() {
            Ok(p) => p,
            Err(SymphoniaError::IoError(ref e))
                if e.kind() == std::io::ErrorKind::UnexpectedEof =>
            {
                break;
            }
            Err(SymphoniaError::ResetRequired) => break,
            Err(e) => {
                return Err(SoundboardError::Decode {
                    path: display.clone(),
                    message: format!("read packet: {e}"),
                });
            }
        };
        if packet.track_id() != track_id {
            continue;
        }
        let decoded = match decoder.decode(&packet) {
            Ok(d) => d,
            Err(SymphoniaError::DecodeError(_)) => continue,
            Err(e) => {
                return Err(SoundboardError::Decode {
                    path: display.clone(),
                    message: format!("decode: {e}"),
                });
            }
        };
        append_mono(&mut samples, &decoded);
    }

    if samples.is_empty() {
        return Err(SoundboardError::Decode {
            path: display.clone(),
            message: "decoded zero samples".into(),
        });
    }

    #[allow(clippy::cast_precision_loss)]
    let duration_secs = samples.len() as f32 / sample_rate as f32;

    Ok(DecodedClip {
        samples: Arc::new(samples),
        sample_rate,
        duration_secs,
    })
}

/// Mix `decoded` down to mono and append to `out`.
fn append_mono(out: &mut Vec<f32>, decoded: &AudioBufferRef<'_>) {
    match decoded {
        AudioBufferRef::F32(buf) => mix_mono_into::<f32>(out, buf, |s| s),
        AudioBufferRef::F64(buf) => mix_mono_into::<f64>(out, buf, |s| s as f32),
        AudioBufferRef::S8(buf) => mix_mono_into::<i8>(out, buf, |s| f32::from(s) / 128.0),
        AudioBufferRef::S16(buf) => mix_mono_into::<i16>(out, buf, |s| f32::from(s) / 32_768.0),
        AudioBufferRef::S24(buf) => mix_mono_into::<symphonia::core::sample::i24>(out, buf, |s| {
            s.inner() as f32 / 8_388_608.0
        }),
        AudioBufferRef::S32(buf) => mix_mono_into::<i32>(out, buf, |s| s as f32 / 2_147_483_648.0),
        AudioBufferRef::U8(buf) => {
            mix_mono_into::<u8>(out, buf, |s| (f32::from(s) - 128.0) / 128.0)
        }
        AudioBufferRef::U16(buf) => {
            mix_mono_into::<u16>(out, buf, |s| (f32::from(s) - 32_768.0) / 32_768.0)
        }
        AudioBufferRef::U24(buf) => mix_mono_into::<symphonia::core::sample::u24>(out, buf, |s| {
            (s.inner() as f32 - 8_388_608.0) / 8_388_608.0
        }),
        AudioBufferRef::U32(buf) => {
            mix_mono_into::<u32>(out, buf, |s| (s as f32 - 2_147_483_648.0) / 2_147_483_648.0)
        }
    }
}

fn mix_mono_into<S>(
    out: &mut Vec<f32>,
    buf: &symphonia::core::audio::AudioBuffer<S>,
    to_f32: impl Fn(S) -> f32,
) where
    S: symphonia::core::sample::Sample + Copy,
{
    let frames = buf.frames();
    let channels = buf.spec().channels.count();
    if frames == 0 || channels == 0 {
        return;
    }
    out.reserve(frames);
    for frame in 0..frames {
        let mut sum = 0.0_f32;
        for ch in 0..channels {
            sum += to_f32(buf.chan(ch)[frame]);
        }
        #[allow(clippy::cast_precision_loss)]
        let avg = sum / channels as f32;
        out.push(avg);
    }
}
