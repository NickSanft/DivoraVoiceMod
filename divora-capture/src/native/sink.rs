//! The MP4 sink writer both muxers share (v1.54.0): the file, the AAC track
//! encoded from the timeline's PCM, and finishing.
//!
//! Moved out of `encoder.rs` unchanged, so the take writer of v1.52.0 (rung
//! 1, which still encodes through the sink writer) makes the same calls in
//! the same order as before, and the passthrough writer of v1.54.0 reuses
//! them.

use std::path::Path;

use windows::core::HSTRING;
use windows::Win32::Media::MediaFoundation::{
    IMFAttributes, IMFMediaBuffer, IMFMediaType, IMFSinkWriter, MFAudioFormat_AAC,
    MFAudioFormat_PCM, MFMediaType_Audio, MFTranscodeContainerType_MPEG4,
    MF_MT_AUDIO_AVG_BYTES_PER_SECOND, MF_MT_AUDIO_BITS_PER_SAMPLE, MF_MT_AUDIO_BLOCK_ALIGNMENT,
    MF_MT_AUDIO_NUM_CHANNELS, MF_MT_AUDIO_SAMPLES_PER_SECOND, MF_MT_MAJOR_TYPE, MF_MT_SUBTYPE,
    MF_SINK_WRITER_DISABLE_THROTTLING, MF_SINK_WRITER_STATISTICS, MF_TRANSCODE_CONTAINERTYPE,
};

use super::mf::{Mf, MfSession};
use crate::error::{CaptureError, Context};
use crate::timeline::{AudioBlock, OUT_RATE};

/// AAC-LC at 192 kbps: one of the four rates Windows' AAC encoder takes,
/// in bytes per second.
const AAC_BYTES_PER_SECOND: u32 = 24_000;
const AUDIO_CHANNELS: u32 = 2;

/// An open MP4 sink writer.
pub(crate) struct SinkCore {
    pub writer: IMFSinkWriter,
    audio: Option<u32>,
    api: &'static Mf,
    finished: bool,
}

impl SinkCore {
    /// Open `path` as an MP4 — chosen by attribute, not by the extension, so
    /// `path` can be a `.part` file renamed afterwards. `throttle` keeps the
    /// writer's back-pressure (a take encoding through it relies on it);
    /// `extra` adds attributes before the container type, as rung 1 always
    /// has (its device manager).
    pub fn create(
        mf: &MfSession,
        path: &Path,
        throttle: bool,
        extra: &dyn Fn(&IMFAttributes) -> Result<(), CaptureError>,
    ) -> Result<Self, CaptureError> {
        let api = mf.mf();
        let attrs = api.create_attributes(4)?;
        extra(&attrs)?;
        // SAFETY: setters on a live attribute store with valid keys/values.
        unsafe {
            attrs
                .SetGUID(&MF_TRANSCODE_CONTAINERTYPE, &MFTranscodeContainerType_MPEG4)
                .context("MF_TRANSCODE_CONTAINERTYPE")?;
            attrs
                .SetUINT32(&MF_SINK_WRITER_DISABLE_THROTTLING, u32::from(!throttle))
                .context("MF_SINK_WRITER_DISABLE_THROTTLING")?;
        }
        let path_w = HSTRING::from(path.as_os_str());
        let writer = api.create_sink_writer(&path_w, &attrs)?;
        Ok(Self {
            writer,
            audio: None,
            api,
            finished: false,
        })
    }

    /// Add the AAC-LC track, fed 48 kHz stereo 16-bit PCM.
    pub fn add_aac_from_pcm(&mut self) -> Result<(), CaptureError> {
        let out = audio_type(self.api, MFAudioFormat_AAC, AAC_BYTES_PER_SECOND)?;
        // SAFETY: live writer and type.
        let audio = unsafe { self.writer.AddStream(&out) }.context("AddStream (AAC)")?;
        let pcm = audio_type(self.api, MFAudioFormat_PCM, OUT_RATE * AUDIO_CHANNELS * 2)?;
        // SAFETY: setter on a live media type.
        unsafe { pcm.SetUINT32(&MF_MT_AUDIO_BLOCK_ALIGNMENT, AUDIO_CHANNELS * 2) }
            .context("MF_MT_AUDIO_BLOCK_ALIGNMENT")?;
        // SAFETY: live writer and type; no encoding parameters.
        unsafe {
            self.writer
                .SetInputMediaType(audio, &pcm, None::<&IMFAttributes>)
        }
        .context("SetInputMediaType (PCM)")?;
        self.audio = Some(audio);
        Ok(())
    }

    /// The AAC track's stream index, if the file has one.
    pub fn audio_stream(&self) -> Option<u32> {
        self.audio
    }

    /// Start writing: every stream is added.
    pub fn begin(&self) -> Result<(), CaptureError> {
        // SAFETY: configured above.
        unsafe { self.writer.BeginWriting() }.context("BeginWriting")
    }

    pub fn api(&self) -> &'static Mf {
        self.api
    }

    /// A media buffer holding a copy of `bytes`.
    pub fn memory_buffer(&self, bytes: &[u8]) -> Result<IMFMediaBuffer, CaptureError> {
        let len = u32::try_from(bytes.len()).unwrap_or(u32::MAX);
        let buffer = self.api.create_memory_buffer(len)?;
        // SAFETY: Lock hands out `max` writable bytes, at least `len` as
        // asked; exactly `bytes.len()` are copied in before Unlock.
        unsafe {
            let mut data: *mut u8 = std::ptr::null_mut();
            let mut max = 0u32;
            buffer
                .Lock(&raw mut data, Some(&raw mut max), None)
                .context("IMFMediaBuffer::Lock")?;
            if data.is_null() || (max as usize) < bytes.len() {
                let _ = buffer.Unlock();
                return Err(CaptureError::Unavailable(
                    "Media Foundation gave a short buffer.".to_string(),
                ));
            }
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), data, bytes.len());
            buffer.Unlock().context("IMFMediaBuffer::Unlock")?;
            buffer.SetCurrentLength(len).context("SetCurrentLength")?;
        }
        Ok(buffer)
    }

    /// A sample of `buffer` at `pts` for `duration` (100-ns ticks).
    pub fn sample(
        &self,
        buffer: &IMFMediaBuffer,
        pts: u64,
        duration: u64,
    ) -> Result<windows::Win32::Media::MediaFoundation::IMFSample, CaptureError> {
        let sample = self.api.create_sample()?;
        // SAFETY: live sample and buffer; times are non-negative.
        unsafe {
            sample.AddBuffer(buffer).context("AddBuffer")?;
            sample
                .SetSampleTime(i64::try_from(pts).unwrap_or(i64::MAX))
                .context("SetSampleTime")?;
            sample
                .SetSampleDuration(i64::try_from(duration).unwrap_or(1).max(1))
                .context("SetSampleDuration")?;
        }
        Ok(sample)
    }

    /// Wrap `buffer` in a sample at `pts` for `duration` and write it.
    pub fn write_sample(
        &self,
        stream: u32,
        buffer: &IMFMediaBuffer,
        pts: u64,
        duration: u64,
        what: &'static str,
    ) -> Result<(), CaptureError> {
        let sample = self.sample(buffer, pts, duration)?;
        // SAFETY: live writer and sample.
        unsafe { self.writer.WriteSample(stream, &sample) }.context(what)
    }

    /// Write one block of the audio track. Ignored when the file has none.
    pub fn write_audio(&self, block: &AudioBlock) -> Result<(), CaptureError> {
        let Some(stream) = self.audio else {
            return Ok(());
        };
        let bytes: Vec<u8> = block.samples.iter().flat_map(|s| s.to_le_bytes()).collect();
        let buffer = self.memory_buffer(&bytes)?;
        self.write_sample(
            stream,
            &buffer,
            block.pts,
            block.duration,
            "WriteSample (audio)",
        )
    }

    /// Bytes the writer holds, not yet in the file, summed over `streams`:
    /// the caller's back-pressure when throttling is off. An error is the
    /// writer's error — never "nothing queued", which would turn the
    /// back-pressure off without a word.
    pub fn queued_bytes(&self, streams: &[u32]) -> Result<u64, CaptureError> {
        let mut total = 0u64;
        for &stream in streams {
            let mut stats = MF_SINK_WRITER_STATISTICS {
                cb: u32::try_from(std::mem::size_of::<MF_SINK_WRITER_STATISTICS>())
                    .unwrap_or(u32::MAX),
                ..Default::default()
            };
            // SAFETY: a live writer and a statistics struct whose `cb` is set,
            // as the method requires.
            unsafe { self.writer.GetStatistics(stream, &raw mut stats) }
                .context("IMFSinkWriter::GetStatistics")?;
            total += u64::from(stats.dwByteCountQueued);
        }
        Ok(total)
    }

    /// Let go of the file unfinished: nothing queued is written, no index —
    /// for a file that is about to be removed (a cancelled save), so it goes
    /// at once rather than after the disk has caught up.
    pub fn abandon(mut self) {
        self.finished = true;
    }

    /// Finish the file (writes the MP4 index). Must be called for the file
    /// to be playable.
    pub fn finish(mut self) -> Result<(), CaptureError> {
        self.finished = true;
        // SAFETY: live writer; called once.
        unsafe { self.writer.Finalize() }.context("Finalize")
    }
}

impl Drop for SinkCore {
    fn drop(&mut self) {
        if !self.finished {
            // Dropped unfinished on an error: finalize what there is rather
            // than leave an index-less file, and ignore failure (the file is
            // a `.part`). A file let go of on purpose is `abandon`ed instead.
            // SAFETY: live writer.
            let _ = unsafe { self.writer.Finalize() };
        }
    }
}

/// 48 kHz stereo 16-bit audio of `subtype`, at `bytes_per_second`.
fn audio_type(
    api: &Mf,
    subtype: windows::core::GUID,
    bytes_per_second: u32,
) -> Result<IMFMediaType, CaptureError> {
    let t = api.create_media_type()?;
    // SAFETY: setters on a live media type with valid keys/values.
    unsafe {
        t.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Audio)
            .context("MF_MT_MAJOR_TYPE")?;
        t.SetGUID(&MF_MT_SUBTYPE, &raw const subtype)
            .context("MF_MT_SUBTYPE")?;
        t.SetUINT32(&MF_MT_AUDIO_BITS_PER_SAMPLE, 16)
            .context("MF_MT_AUDIO_BITS_PER_SAMPLE")?;
        t.SetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND, OUT_RATE)
            .context("MF_MT_AUDIO_SAMPLES_PER_SECOND")?;
        t.SetUINT32(&MF_MT_AUDIO_NUM_CHANNELS, AUDIO_CHANNELS)
            .context("MF_MT_AUDIO_NUM_CHANNELS")?;
        t.SetUINT32(&MF_MT_AUDIO_AVG_BYTES_PER_SECOND, bytes_per_second)
            .context("MF_MT_AUDIO_AVG_BYTES_PER_SECOND")?;
    }
    Ok(t)
}
