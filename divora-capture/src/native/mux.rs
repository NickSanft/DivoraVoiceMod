//! Already-encoded H.264 into an MP4, with the AAC track encoded from PCM
//! (v1.54.0): what a take on the directly driven encoder and a saved replay
//! are written with.
//!
//! The sink writer stores compressed video unchanged when the stream's
//! target type and its input type are one and the same object — Microsoft's
//! "compressed input with identical output". What was measured on this
//! machine decides every rule here:
//!
//! - the type needs the frame size and rate, and its sequence header (SPS +
//!   PPS): without one `Finalize` fails;
//! - the sink takes Annex B and writes length-prefixed NAL units and the
//!   `avcC` itself;
//! - only `MFSampleExtension_CleanPoint` builds the keyframe table, so it is
//!   set on IDRs and nowhere else;
//! - a leading non-IDR is written but undecodable, so writing starts at the
//!   first IDR;
//! - a negative time is dropped without a word, so every time is counted
//!   from the first frame written;
//! - with throttling on, one writing thread hangs once a stream leads the
//!   other by about two seconds, so throttling is off and the caller paces
//!   itself on [`Mp4Writer::queued_bytes`].

use std::path::Path;
use std::sync::Arc;

use windows::core::{Interface, GUID};
use windows::Win32::Media::MediaFoundation::{
    IMFAttributes, IMFTransform, MFSampleExtension_CleanPoint, MFVideoFormat_H264,
    MF_MT_MPEG_SEQUENCE_HEADER,
};

use super::encoder::video_type;
use super::mf::MfSession;
use super::sink::SinkCore;
use crate::error::{CaptureError, Context};
use crate::pacer::pts_at;
use crate::timeline::{AudioBlock, OUT_RATE};

/// The video a file is opened for.
#[derive(Debug, Clone)]
pub struct VideoFormat {
    pub size: (u32, u32),
    pub fps: u32,
    /// SPS + PPS, Annex B: the opening IDR's own when it carries them, so
    /// the file's `avcC` always matches its first frame.
    pub seq_header: Arc<[u8]>,
}

/// An MP4 being written from encoded access units.
pub struct Mp4Writer {
    core: SinkCore,
    video: u32,
    fps: u32,
    /// The index of the first access unit written (an IDR): times count
    /// from it.
    origin: Option<u64>,
    last: Option<u64>,
    /// 48 kHz frames written by [`Self::write_pcm`].
    pcm_frames: u64,
    passthrough: bool,
}

impl Mp4Writer {
    /// Open `path` as an MP4 for `v`'s video, with an AAC track if
    /// `with_audio`.
    pub fn create(
        mf: &MfSession,
        path: &Path,
        v: &VideoFormat,
        with_audio: bool,
    ) -> Result<Self, CaptureError> {
        let mut core = SinkCore::create(mf, path, false, &|_: &IMFAttributes| Ok(()))?;
        let t = video_type(core.api(), MFVideoFormat_H264, v.size, v.fps)?;
        // SAFETY: a setter on a live media type; the blob is copied.
        unsafe { t.SetBlob(&MF_MT_MPEG_SEQUENCE_HEADER, &v.seq_header) }
            .context("MF_MT_MPEG_SEQUENCE_HEADER")?;
        // One media type object as both the target and the input: any
        // difference between the two and the writer refuses it or loads an
        // encoder.
        // SAFETY: live writer and type.
        let video = unsafe { core.writer.AddStream(&t) }.context("AddStream (H.264)")?;
        // SAFETY: as above; no encoding parameters.
        unsafe {
            core.writer
                .SetInputMediaType(video, &t, None::<&IMFAttributes>)
        }
        .context("SetInputMediaType (H.264 passthrough)")?;
        if with_audio {
            core.add_aac_from_pcm()?;
        }
        core.begin()?;
        let passthrough = !has_encoder(&core, video);
        Ok(Self {
            core,
            video,
            fps: v.fps.max(1),
            origin: None,
            last: None,
            pcm_frames: 0,
            passthrough,
        })
    }

    /// Whether the video goes into the file as it is (no encoder loaded).
    pub fn passthrough(&self) -> bool {
        self.passthrough
    }

    /// Write the access unit of frame `index` (Annex B; `key` when it holds
    /// an IDR). `Ok(false)` while waiting for the first IDR — nothing before
    /// it could be decoded. Indices must increase.
    pub fn write_video(
        &mut self,
        index: u64,
        key: bool,
        data: &[u8],
    ) -> Result<bool, CaptureError> {
        let origin = match self.origin {
            Some(o) => o,
            None if key => {
                self.origin = Some(index);
                index
            }
            None => return Ok(false),
        };
        if self.last.is_some_and(|l| index <= l) {
            return Err(CaptureError::Unavailable(format!(
                "frame {index} after frame {}",
                self.last.unwrap_or(0)
            )));
        }
        let rel = index - origin;
        let pts = pts_at(rel, self.fps);
        let dur = pts_at(rel + 1, self.fps) - pts;
        let buffer = self.core.memory_buffer(data)?;
        let sample = self.core.sample(&buffer, pts, dur)?;
        if key {
            // SAFETY: a setter on a live sample.
            unsafe { sample.SetUINT32(&MFSampleExtension_CleanPoint, 1) }
                .context("MFSampleExtension_CleanPoint")?;
        }
        // SAFETY: live writer and sample.
        unsafe { self.core.writer.WriteSample(self.video, &sample) }
            .context("WriteSample (H.264)")?;
        self.last = Some(index);
        Ok(true)
    }

    /// The index of the first access unit written, if any.
    pub fn origin(&self) -> Option<u64> {
        self.origin
    }

    /// One block of the take's audio, already timed from the file's start.
    pub fn write_audio(&self, block: &AudioBlock) -> Result<(), CaptureError> {
        self.core.write_audio(block)
    }

    /// The next interleaved stereo 16-bit frames of a saved clip's audio,
    /// timed by how many came before.
    pub fn write_pcm(&mut self, frames: &[i16]) -> Result<(), CaptureError> {
        let Some(stream) = self.core.audio_stream() else {
            return Ok(());
        };
        let n = (frames.len() / 2) as u64;
        if n == 0 {
            return Ok(());
        }
        let pts = sample_ticks(self.pcm_frames);
        let end = sample_ticks(self.pcm_frames + n);
        let bytes: Vec<u8> = frames[..usize::try_from(n * 2).unwrap_or(frames.len())]
            .iter()
            .flat_map(|s| s.to_le_bytes())
            .collect();
        let buffer = self.core.memory_buffer(&bytes)?;
        self.core
            .write_sample(stream, &buffer, pts, end - pts, "WriteSample (audio)")?;
        self.pcm_frames += n;
        Ok(())
    }

    /// Bytes the writer holds for both streams, not yet in the file.
    pub fn queued_bytes(&self) -> Result<u64, CaptureError> {
        match self.core.audio_stream() {
            Some(a) => self.core.queued_bytes(&[self.video, a]),
            None => self.core.queued_bytes(&[self.video]),
        }
    }

    pub fn finish(self) -> Result<(), CaptureError> {
        self.core.finish()
    }

    /// Let go of the file unfinished, for its removal (see
    /// [`SinkCore::abandon`]).
    pub fn abandon(self) {
        self.core.abandon();
    }
}

/// 48 kHz frame `n` in 100-ns ticks.
fn sample_ticks(n: u64) -> u64 {
    u64::try_from(u128::from(n) * 10_000_000 / u128::from(OUT_RATE)).unwrap_or(u64::MAX)
}

/// Whether the writer loaded an encoder for `stream` (it shouldn't have).
fn has_encoder(core: &SinkCore, stream: u32) -> bool {
    let mut p = std::ptr::null_mut();
    // SAFETY: a valid out-pointer; asks for the stream's encoder, if any.
    let got = unsafe {
        core.writer
            .GetServiceForStream(stream, &GUID::zeroed(), &IMFTransform::IID, &raw mut p)
    };
    if got.is_err() || p.is_null() {
        return false;
    }
    // SAFETY: an owned IMFTransform; released at once.
    drop(unsafe { IMFTransform::from_raw(p) });
    true
}
