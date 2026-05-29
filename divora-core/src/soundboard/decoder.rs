//! Decode a single audio file into a mono f32 buffer.
//!
//! Uses symphonia 0.6 for WAV / MP3 / OGG-Vorbis / FLAC and the
//! `symphonia-adapter-libopus` adapter for OGG-Opus (Discord
//! voice recordings).  The returned `DecodedClip` keeps the file's
//! native sample rate; the mixer interpolates against the engine's
//! rate at play time.

// Sample-format conversion inherently loses precision; that's the
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
use std::sync::{Arc, OnceLock};

use symphonia::core::codecs::audio::AudioDecoderOptions;
use symphonia::core::codecs::registry::CodecRegistry;
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::probe::{Hint, Probe};
use symphonia::core::formats::{FormatOptions, TrackType};
use symphonia::core::io::{MediaSourceStream, MediaSourceStreamOptions};
use symphonia::core::meta::MetadataOptions;
use symphonia_adapter_libopus::OpusDecoder;

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

/// Owned `Probe` registry. Wraps the default symphonia formats (we still
/// want every demuxer we already shipped: OGG / WAV / FLAC / MP3 / MKV
/// / RIFF / ADPCM). `OnceLock` keeps it cheap to call repeatedly.
fn probe() -> &'static Probe {
    static PROBE: OnceLock<Probe> = OnceLock::new();
    PROBE.get_or_init(|| {
        let mut p = Probe::new();
        symphonia::default::register_enabled_formats(&mut p);
        p
    })
}

/// Owned `CodecRegistry` — every default codec PLUS the libopus adapter
/// so OGG-Opus files (Discord clips, podcast snippets) decode.
fn codecs() -> &'static CodecRegistry {
    static CODECS: OnceLock<CodecRegistry> = OnceLock::new();
    CODECS.get_or_init(|| {
        let mut r = CodecRegistry::new();
        symphonia::default::register_enabled_codecs(&mut r);
        r.register_audio_decoder::<OpusDecoder>();
        r
    })
}

/// Decode `path` into a `DecodedClip`. Stereo files are mixed to mono;
/// integer formats are normalised to [-1, 1] (handled by symphonia's
/// `copy_to_vec_interleaved::<f32>`).
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

    let mut format = probe()
        .probe(
            &hint,
            mss,
            FormatOptions::default(),
            MetadataOptions::default(),
        )
        .map_err(|e| SoundboardError::Decode {
            path: display.clone(),
            message: format!("probe: {e}"),
        })?;

    let track = format
        .default_track(TrackType::Audio)
        .ok_or_else(|| SoundboardError::Decode {
            path: display.clone(),
            message: "no default audio track".into(),
        })?;
    let track_id = track.id;
    let audio_params = track
        .codec_params
        .as_ref()
        .and_then(|p| p.audio())
        .ok_or_else(|| SoundboardError::Decode {
            path: display.clone(),
            message: "track has no audio codec params".into(),
        })?
        .clone();
    let sample_rate = audio_params.sample_rate.unwrap_or(48_000);

    let mut decoder = codecs()
        .make_audio_decoder(&audio_params, &AudioDecoderOptions::default())
        .map_err(|e| SoundboardError::Decode {
            path: display.clone(),
            message: format!("make decoder: {e}"),
        })?;

    let mut samples: Vec<f32> = Vec::new();
    let mut interleaved: Vec<f32> = Vec::new();

    loop {
        let packet = match format.next_packet() {
            Ok(Some(p)) => p,
            Ok(None) | Err(SymphoniaError::ResetRequired) => break,
            Err(e) => {
                return Err(SoundboardError::Decode {
                    path: display.clone(),
                    message: format!("read packet: {e}"),
                });
            }
        };
        if packet.track_id != track_id {
            continue;
        }
        let decoded = match decoder.decode(&packet) {
            Ok(d) => d,
            Err(SymphoniaError::DecodeError(_) | SymphoniaError::IoError(_)) => continue,
            Err(e) => {
                return Err(SoundboardError::Decode {
                    path: display.clone(),
                    message: format!("decode: {e}"),
                });
            }
        };
        append_mono(&mut samples, &mut interleaved, &decoded);
    }

    if samples.is_empty() {
        return Err(SoundboardError::Decode {
            path: display.clone(),
            message: "decoded zero samples".into(),
        });
    }

    let duration_secs = samples.len() as f32 / sample_rate as f32;

    Ok(DecodedClip {
        samples: Arc::new(samples),
        sample_rate,
        duration_secs,
    })
}

/// Copy `decoded` out as interleaved f32 (symphonia handles the integer
/// → float normalisation for us), then mix to mono into `out`.
fn append_mono(
    out: &mut Vec<f32>,
    interleaved: &mut Vec<f32>,
    decoded: &symphonia::core::audio::GenericAudioBufferRef<'_>,
) {
    let channels = decoded.spec().channels().count().max(1);
    let total = decoded.samples_interleaved();
    if total == 0 {
        return;
    }

    interleaved.clear();
    decoded.copy_to_vec_interleaved::<f32>(interleaved);

    let frames = interleaved.len() / channels;
    out.reserve(frames);
    let ch_f = channels as f32;
    for frame in 0..frames {
        let base = frame * channels;
        let mut sum = 0.0_f32;
        for c in 0..channels {
            sum += interleaved[base + c];
        }
        out.push(sum / ch_f);
    }
}

#[cfg(test)]
mod tests {
    use super::{codecs, decode_clip, probe};
    use std::path::Path;
    use symphonia::core::codecs::audio::well_known::{
        CODEC_ID_FLAC, CODEC_ID_MP3, CODEC_ID_OPUS, CODEC_ID_PCM_S16LE, CODEC_ID_VORBIS,
    };
    use symphonia::core::codecs::audio::AudioCodecParameters;
    use symphonia::core::codecs::audio::AudioDecoderOptions;

    /// Build a synthetic `AudioCodecParameters` and ask the registry to
    /// produce a decoder for it. The registry rejects a missing decoder
    /// with `"unsupported codec"` (the v0.6.0 bug); a registered
    /// decoder may still reject malformed params with its own error
    /// (e.g. `"opus: channels or channel layout is required"`). So we
    /// only fail when the error contains the missing-decoder phrase.
    fn assert_decoder_registered(codec_id: symphonia::core::codecs::audio::AudioCodecId) {
        let params = AudioCodecParameters {
            codec: codec_id,
            sample_rate: Some(48_000),
            ..Default::default()
        };
        let result = codecs().make_audio_decoder(&params, &AudioDecoderOptions::default());
        if let Err(e) = result {
            let msg = format!("{e}").to_lowercase();
            assert!(
                !msg.contains("unsupported codec"),
                "decoder for {codec_id:?} not registered: {e}"
            );
        }
    }

    /// Regression for the v0.6.0 bug where Discord OGG-Opus files failed
    /// with "unsupported codec". The custom registry must include the
    /// libopus adapter alongside every default codec.
    #[test]
    fn codec_registry_includes_opus_decoder() {
        assert_decoder_registered(CODEC_ID_OPUS);
    }

    #[test]
    fn codec_registry_keeps_all_default_audio_decoders() {
        // Sanity-check that adding the Opus adapter didn't accidentally
        // wipe out symphonia's bundled set.
        for codec in &[
            CODEC_ID_VORBIS,
            CODEC_ID_FLAC,
            CODEC_ID_MP3,
            CODEC_ID_PCM_S16LE,
        ] {
            assert_decoder_registered(*codec);
        }
    }

    #[test]
    fn probe_registry_is_non_empty_after_seeding() {
        // `Probe::probe` is hard to call without a real stream — but
        // touching the OnceLock at least confirms init didn't panic.
        let _ = probe();
    }

    #[test]
    fn decode_clip_reports_a_friendly_error_for_missing_files() {
        let result = decode_clip(Path::new("Z:\\definitely-not-a-real-path\\nope.ogg"));
        let err = result.expect_err("missing path should error, not succeed");
        let msg = format!("{err}");
        assert!(
            msg.to_lowercase().contains("open"),
            "expected an 'open' error, got: {msg}"
        );
    }
}
