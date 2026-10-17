//! A take's own writing thread (v1.54.0): `divora-take-mux`.
//!
//! The capture pipeline never touches the disk — a slow disk must not make
//! it drop frames for the replay ring — so each take writes on a thread of
//! its own. It is told what to write, in order ([`MuxMsg`]): open the file
//! at the take's first keyframe, the encoded frames, the take's cut of the
//! mixed track and of the separate files, and the end. Then it finishes the
//! file and gives it its name, as a v1.52.0 take always has: `.part` while
//! writing, the separate files renamed after the video. It is the one that
//! tells the app the take has started (its file is open) and finished
//! (after the rename) — exactly once, however the take ended.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{sync_channel, Receiver, SyncSender, TrySendError};
use std::sync::Arc;
use std::time::Duration;

use super::mf;
use super::mux::{Mp4Writer, VideoFormat};
use super::record::ComScope;
use super::save::QUEUE_CAP;
use crate::error::CaptureError;
use crate::pipeline::{TakeEvents, NO_PICTURE};
use crate::pipeline_core::{MuxMsg, TakeEnd};
use crate::stems::{StemStats, StemWriter};
use crate::take::{part_path, RecordStats};

/// Messages the pipeline may get ahead of the disk by before the take is
/// given up as "the disk can't keep up".
const CHANNEL: usize = 4096;

/// Bytes of encoded video waiting for the disk past which the take is given
/// up the same way: half a minute even at the highest bitrate offered.
const QUEUED_CAP: u64 = 64 * 1024 * 1024;

/// What a writer that lost its pipeline says.
const LOST: &str = "The recording stopped unexpectedly. What was recorded is saved.";

/// What the writing came to.
#[derive(Debug, Default)]
struct MuxOutcome {
    /// Whether a file was opened (no first keyframe: no file).
    opened: bool,
    frames: u64,
    /// Blocks of the mixed track written.
    blocks: u64,
    stems: Option<StemStats>,
    stems_error: Option<String>,
}

/// Why a message couldn't be handed over.
pub enum SendFail {
    /// The writer is that far behind: the take must end.
    Full(MuxMsg),
    /// The writer has stopped (its file failed); it has said so itself.
    Gone,
}

/// The pipeline's end of a take's writing thread. Dropping it lets the
/// thread finish on its own; nobody waits for it.
pub struct TakeMux {
    tx: SyncSender<MuxMsg>,
    /// Encoded bytes sent and not yet written.
    queued: Arc<AtomicU64>,
}

/// What the file is.
pub struct TakeFile {
    pub path: PathBuf,
    pub size: (u32, u32),
    pub fps: u32,
    /// An AAC track (the take has an audio feed).
    pub with_audio: bool,
    /// Separate WAV files, and whether they include desktop audio.
    pub stems: Option<bool>,
}

impl TakeMux {
    pub fn spawn(file: TakeFile, events: Arc<dyn TakeEvents>) -> Result<Self, CaptureError> {
        let (tx, rx) = sync_channel(CHANNEL);
        let queued = Arc::new(AtomicU64::new(0));
        let written = Arc::clone(&queued);
        std::thread::Builder::new()
            .name("divora-take-mux".into())
            .spawn(move || {
                let mut end = None;
                let r = run(&file, &rx, events.as_ref(), &written, &mut end);
                report(&file, r, end, events.as_ref());
            })
            .map_err(CaptureError::Io)?;
        Ok(Self { tx, queued })
    }

    /// Hand `msg` over without waiting. `Full` once the writer is that far
    /// behind, by messages or by bytes.
    pub fn send(&self, msg: MuxMsg) -> Result<(), SendFail> {
        let bytes = match &msg {
            MuxMsg::Video(au) => au.data.len() as u64,
            _ => 0,
        };
        if bytes > 0 && self.queued.load(Ordering::Acquire) > QUEUED_CAP {
            return Err(SendFail::Full(msg));
        }
        match self.tx.try_send(msg) {
            Ok(()) => {
                self.queued.fetch_add(bytes, Ordering::AcqRel);
                Ok(())
            }
            Err(TrySendError::Full(m)) => Err(SendFail::Full(m)),
            Err(TrySendError::Disconnected(_)) => Err(SendFail::Gone),
        }
    }

    /// Hand over the rest of a take whose writer fell behind — waiting is
    /// fine, on a thread of its own, so the pipeline never does.
    pub fn finish_behind(self, msgs: Vec<MuxMsg>) {
        let tx = self.tx;
        let spawned = std::thread::Builder::new()
            .name("divora-take-mux-tail".into())
            .spawn(move || {
                for m in msgs {
                    if tx.send(m).is_err() {
                        break;
                    }
                }
            });
        if let Err(e) = spawned {
            // The writer then finishes the file as it stands.
            tracing::warn!(error = %e, "couldn't hand over the end of a take");
        }
    }
}

/// Write the take. `end` gets the pipeline's last word, if it came.
fn run(
    file: &TakeFile,
    rx: &Receiver<MuxMsg>,
    events: &dyn TakeEvents,
    queued: &AtomicU64,
    end: &mut Option<(TakeEnd, RecordStats)>,
) -> Result<MuxOutcome, CaptureError> {
    let _com = ComScope::enter();
    let session = mf::api()?.session()?;
    let part = part_path(&file.path);
    let mut out = MuxOutcome::default();
    let mut writer: Option<Mp4Writer> = None;
    let mut stems: Option<StemWriter> = None;
    for msg in rx {
        match msg {
            MuxMsg::Open { seq } => {
                if let Some(dir) = file.path.parent() {
                    std::fs::create_dir_all(dir)?;
                }
                let format = VideoFormat {
                    size: file.size,
                    fps: file.fps,
                    seq_header: seq,
                };
                writer = Some(Mp4Writer::create(
                    &session,
                    &part,
                    &format,
                    file.with_audio,
                )?);
                if let Some(desktop) = file.stems {
                    match StemWriter::create(&file.path, desktop) {
                        Ok(w) => stems = Some(w),
                        Err(e) => {
                            tracing::warn!(error = %e, "couldn't start the separate audio files");
                            out.stems_error = Some(e.to_string());
                        }
                    }
                }
                out.opened = true;
                events.started();
            }
            MuxMsg::Video(au) => {
                queued.fetch_sub(au.data.len() as u64, Ordering::AcqRel);
                if let Some(w) = writer.as_mut() {
                    // Throttling is off: the writer would queue without
                    // bound, so wait for it here — this thread may.
                    while w.queued_bytes()? > QUEUE_CAP {
                        std::thread::sleep(Duration::from_millis(1));
                    }
                    if w.write_video(au.index, au.key, &au.data)? {
                        out.frames += 1;
                    }
                }
            }
            MuxMsg::Audio(block) => {
                if let Some(w) = writer.as_ref() {
                    if file.with_audio {
                        w.write_audio(&block)?;
                        out.blocks += 1;
                    }
                }
            }
            MuxMsg::Stems(blocks) => write_stems(&mut stems, &blocks, &mut out),
            MuxMsg::End { how, stats } => {
                *end = Some((how, *stats));
                break;
            }
        }
    }
    let Some(w) = writer else {
        return Ok(out);
    };
    w.finish()?;
    std::fs::rename(&part, &file.path)?;
    finish_stems(stems, &mut out);
    Ok(out)
}

/// Tell the app how the take ended: its file, or why there is none (or why
/// it ended early, its file kept).
fn report(
    file: &TakeFile,
    r: Result<MuxOutcome, CaptureError>,
    end: Option<(TakeEnd, RecordStats)>,
    events: &dyn TakeEvents,
) {
    let (how, mut stats) = match end {
        Some((how, stats)) => (Some(how), stats),
        None => (None, RecordStats::default()),
    };
    let outcome = match r {
        Ok(out) => {
            stats.frames_written = out.frames;
            if let Some(a) = stats.audio.as_mut() {
                a.blocks = out.blocks;
                a.stems = out.stems;
                a.stems_error = out.stems_error;
            }
            match (out.opened, how) {
                (true, Some(TakeEnd::Clean)) => Ok(file.path.clone()),
                (_, Some(TakeEnd::Cut(why))) => Err(why),
                (true, None) => Err(LOST.to_string()),
                (false, Some(TakeEnd::Clean) | None) => Err(NO_PICTURE.to_string()),
            }
        }
        // As v1.53.0: a writer error leaves the finalized `.part`.
        Err(e) => {
            tracing::warn!(error = %e, "a take's writer failed");
            Err(e.to_string())
        }
    };
    events.finished(outcome, stats);
}

/// Stems never fail the take: an error stops them and says why.
fn write_stems(
    stems: &mut Option<StemWriter>,
    blocks: &[crate::timeline::StemBlock],
    out: &mut MuxOutcome,
) {
    let Some(w) = stems.as_mut() else {
        return;
    };
    for b in blocks {
        if let Err(e) = w.write(b) {
            tracing::warn!(error = %e, "the separate audio files stopped");
            out.stems_error = Some(e.to_string());
            *stems = None;
            return;
        }
    }
    if let (Some(frames), None) = (w.stopped_at(), &out.stems_error) {
        tracing::warn!(
            frames,
            "the separate audio files reached a WAV file's limit"
        );
        out.stems_error = Some(crate::track::full_at(frames));
    }
}

fn finish_stems(stems: Option<StemWriter>, out: &mut MuxOutcome) {
    if let Some(w) = stems {
        match w.finish() {
            Ok(s) => out.stems = Some(s),
            Err(e) => {
                tracing::warn!(error = %e, "couldn't finish the separate audio files");
                out.stems_error = Some(e.to_string());
            }
        }
    }
}
