//! A take's sources as separate WAV files (v1.53.0, opt-in): `<name>.voice.wav`
//! (your voice and the soundboard, mono) and `<name>.desktop.wav` (desktop
//! audio before its volume, stereo), beside `<name>.mp4`.
//!
//! They are the very frames the MP4's track is mixed from
//! ([`crate::timeline::StemBlock`]), 48 kHz 32-bit float, starting at the
//! take's first frame and ending on the track's last block — so they line up
//! with each other and with the picture in any editor. Like the MP4 they are
//! written as `.part` files and renamed once finished.

use std::fs::File;
use std::io::BufWriter;
use std::path::{Path, PathBuf};

use crate::error::CaptureError;
use crate::take::part_path;
use crate::timeline::{ticks, StemBlock, BLOCK_FRAMES, OUT_RATE};

/// hound counts a WAV's data bytes in a `u32` that release builds would let
/// wrap, writing a header that lies. Both files stop together a block short
/// of it: about 3.1 hours of stereo float.
const MAX_DATA_BYTES: u64 = u32::MAX as u64 - (1 << 16);

/// What the stems came to.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StemStats {
    /// Frames in each file.
    pub frames: u64,
    /// The take ran past what a WAV file can hold; the stems stop there.
    pub truncated: bool,
}

/// Where the stem `which` ("voice" or "desktop") of the take at `take` goes.
#[must_use]
pub fn stem_path(take: &Path, which: &str) -> PathBuf {
    take.with_extension(format!("{which}.wav"))
}

type Wav = hound::WavWriter<BufWriter<File>>;

struct Stem {
    wav: Wav,
    part: PathBuf,
    path: PathBuf,
}

impl Stem {
    fn create(take: &Path, which: &str, channels: u16) -> Result<Self, CaptureError> {
        let path = stem_path(take, which);
        let part = part_path(&path);
        let spec = hound::WavSpec {
            channels,
            sample_rate: OUT_RATE,
            bits_per_sample: 32,
            sample_format: hound::SampleFormat::Float,
        };
        let file = BufWriter::with_capacity(1 << 16, File::create(&part)?);
        let wav = hound::WavWriter::new(file, spec).map_err(wav_error)?;
        Ok(Self { wav, part, path })
    }

    fn write(&mut self, samples: &[f32]) -> Result<(), CaptureError> {
        for &s in samples {
            self.wav.write_sample(s).map_err(wav_error)?;
        }
        Ok(())
    }

    fn finish(self) -> Result<(), CaptureError> {
        self.wav.finalize().map_err(wav_error)?;
        std::fs::rename(&self.part, &self.path)?;
        Ok(())
    }
}

/// The stems of one take, being written.
pub struct StemWriter {
    voice: Stem,
    desktop: Option<Stem>,
    frames: u64,
    truncated: bool,
    /// [`MAX_DATA_BYTES`], or less in a test.
    cap: u64,
}

impl StemWriter {
    /// Open the stems beside `take`: the voice always, desktop audio when
    /// the track has it.
    pub fn create(take: &Path, desktop: bool) -> Result<Self, CaptureError> {
        Self::with_cap(take, desktop, MAX_DATA_BYTES)
    }

    pub(crate) fn with_cap(take: &Path, desktop: bool, cap: u64) -> Result<Self, CaptureError> {
        let voice = Stem::create(take, "voice", 1)?;
        let desktop = if desktop {
            Some(Stem::create(take, "desktop", 2)?)
        } else {
            None
        };
        Ok(Self {
            voice,
            desktop,
            frames: 0,
            truncated: false,
            cap,
        })
    }

    /// Write the next block. A block that does not follow on from the last
    /// is an error — a slid stem is worse than none.
    pub fn write(&mut self, b: &StemBlock) -> Result<(), CaptureError> {
        if self.truncated {
            return Ok(());
        }
        if b.pts != ticks(self.frames) {
            return Err(CaptureError::Unavailable(format!(
                "stem block at {} where {} was due",
                b.pts,
                ticks(self.frames)
            )));
        }
        let channels: u64 = if self.desktop.is_some() { 2 } else { 1 };
        let bytes_after = (self.frames + BLOCK_FRAMES as u64) * 4 * channels;
        if bytes_after > self.cap {
            self.truncated = true;
            return Ok(());
        }
        self.voice.write(&b.voice)?;
        if let Some(d) = self.desktop.as_mut() {
            if let Some(s) = &b.desktop {
                d.write(s)?;
            } else {
                d.write(&vec![0.0; 2 * b.voice.len()])?;
            }
        }
        self.frames += b.voice.len() as u64;
        Ok(())
    }

    /// Where the files stopped, in frames, once the take outgrew what a WAV
    /// file can hold; `None` while they are still being written.
    #[must_use]
    pub fn stopped_at(&self) -> Option<u64> {
        self.truncated.then_some(self.frames)
    }

    /// Finish both files and give them their real names.
    pub fn finish(self) -> Result<StemStats, CaptureError> {
        self.voice.finish()?;
        if let Some(d) = self.desktop {
            d.finish()?;
        }
        Ok(StemStats {
            frames: self.frames,
            truncated: self.truncated,
        })
    }
}

fn wav_error(e: hound::Error) -> CaptureError {
    match e {
        hound::Error::IoError(io) => CaptureError::Io(io),
        other => CaptureError::Io(std::io::Error::other(other.to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::{stem_path, StemWriter};
    use crate::timeline::{ticks, StemBlock, BLOCK_FRAMES};
    use std::path::Path;

    fn block(k: u64, voice: f32, left: f32, right: f32) -> StemBlock {
        StemBlock {
            pts: ticks(k * BLOCK_FRAMES as u64),
            voice: vec![voice; BLOCK_FRAMES],
            desktop: Some([left, right].repeat(BLOCK_FRAMES)),
        }
    }

    #[test]
    fn stems_sit_beside_the_take() {
        assert_eq!(
            stem_path(Path::new(r"C:\rec\divora-screen-1.mp4"), "voice"),
            Path::new(r"C:\rec\divora-screen-1.voice.wav")
        );
    }

    /// Written, finished and read back: the format, the length, and every
    /// sample where it was put — left still left.
    #[test]
    fn stems_round_trip() {
        let dir = std::env::temp_dir().join(format!("divora-stems-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let take = dir.join("take.mp4");
        let mut w = StemWriter::create(&take, true).unwrap();
        for k in 0..3u8 {
            w.write(&block(u64::from(k), 0.25 * f32::from(k), 0.5, -0.5))
                .unwrap();
        }
        let stats = w.finish().unwrap();
        assert_eq!(stats.frames, 3 * BLOCK_FRAMES as u64);
        assert!(!stats.truncated);
        assert!(!dir.join("take.voice.wav.part").exists());

        let mut voice = hound::WavReader::open(dir.join("take.voice.wav")).unwrap();
        let spec = voice.spec();
        assert_eq!(
            (spec.channels, spec.sample_rate, spec.bits_per_sample),
            (1, 48_000, 32)
        );
        let v: Vec<f32> = voice.samples::<f32>().map(Result::unwrap).collect();
        assert_eq!(v.len(), 3 * BLOCK_FRAMES);
        assert_eq!(
            (v[0], v[BLOCK_FRAMES], v[3 * BLOCK_FRAMES - 1]),
            (0.0, 0.25, 0.5)
        );

        let mut desk = hound::WavReader::open(dir.join("take.desktop.wav")).unwrap();
        assert_eq!(desk.spec().channels, 2);
        let d: Vec<f32> = desk.samples::<f32>().map(Result::unwrap).collect();
        assert_eq!(d.len(), 6 * BLOCK_FRAMES);
        assert!(
            d.chunks(2).all(|lr| lr == [0.5, -0.5]),
            "left and right swapped"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// Past what a WAV can hold, both stems stop at the same block, and
    /// both files are still whole.
    #[test]
    fn stems_stop_together_at_the_size_cap() {
        let dir = std::env::temp_dir().join(format!("divora-stems-cap-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let take = dir.join("take.mp4");
        // Room for two stereo blocks, not three.
        let cap = 2 * BLOCK_FRAMES as u64 * 8 + 100;
        let mut w = StemWriter::with_cap(&take, true, cap).unwrap();
        for k in 0..5 {
            w.write(&block(k, 0.1, 0.2, 0.3)).unwrap();
        }
        let stats = w.finish().unwrap();
        assert!(stats.truncated);
        assert_eq!(stats.frames, 2 * BLOCK_FRAMES as u64);
        let v = hound::WavReader::open(dir.join("take.voice.wav")).unwrap();
        let d = hound::WavReader::open(dir.join("take.desktop.wav")).unwrap();
        let two = u32::try_from(2 * BLOCK_FRAMES).unwrap();
        assert_eq!((v.duration(), d.duration()), (two, two));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// A block out of step is refused, not written somewhere it is not.
    #[test]
    fn a_block_out_of_step_is_an_error() {
        let dir = std::env::temp_dir().join(format!("divora-stems-gap-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut w = StemWriter::create(&dir.join("take.mp4"), true).unwrap();
        w.write(&block(0, 0.0, 0.0, 0.0)).unwrap();
        assert!(w.write(&block(2, 0.0, 0.0, 0.0)).is_err());
        drop(w);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
