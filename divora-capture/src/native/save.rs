//! Writing a replay clip from a snapshot (v1.54.0).
//!
//! The snapshot is the ring's window at the moment Save was pressed: the
//! encoded video as it is, and the mixed audio from the same first frame.
//! Both start at exactly zero in the file — video times from the window's
//! first keyframe, audio from its matching sample — and they are written in
//! time order, so neither runs ahead in the writer's queue.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use super::mf::{self, MfSession};
use super::mux::{Mp4Writer, VideoFormat};
use super::record::ComScope;
use super::system::free_bytes;
use crate::annexb::parameter_sets;
use crate::error::CaptureError;
use crate::pipeline::{SaveRequest, SavedClip};
use crate::replay::{Snapshot, Write};
use crate::take::part_path;

/// The most the writer may hold for the file before the save waits for it:
/// with its throttling off, nothing else bounds it. Small, because it is
/// what finishing the file must flush, which a Quit can't cut short.
pub const QUEUE_CAP: u64 = 4 * 1024 * 1024;

/// What a cancelled save says (Divora is quitting).
pub const CANCELLED: &str = "Divora quit before the replay was saved.";

/// How many writes between checks of `cancel` and of the queue.
const CHECK_EVERY: usize = 16;

/// Write `snap` as an MP4 at `path`. Returns whether the video went in as it
/// was (no encoder loaded). On an error the caller removes the file.
pub fn write_clip(
    mf: &MfSession,
    path: &Path,
    snap: &Snapshot,
    cancel: &AtomicBool,
) -> Result<bool, CaptureError> {
    let Some(first) = snap.video.first() else {
        return Err(CaptureError::Unavailable(
            "The replay had no picture to save.".to_string(),
        ));
    };
    // The file's avcC is the opening keyframe's own parameter sets when it
    // carries them, so it always matches the first frame.
    let seq_header = parameter_sets(&first.data).unwrap_or_else(|| snap.seq.clone());
    let format = VideoFormat {
        size: snap.size,
        fps: snap.fps,
        seq_header,
    };
    let mut w = Mp4Writer::create(mf, path, &format, true)?;
    let passthrough = w.passthrough();
    for (n, step) in snap.writes().into_iter().enumerate() {
        if n % CHECK_EVERY == 0 {
            let mut cancelled = cancel.load(Ordering::Acquire);
            while !cancelled && w.queued_bytes()? > QUEUE_CAP {
                std::thread::sleep(Duration::from_millis(1));
                cancelled = cancel.load(Ordering::Acquire);
            }
            if cancelled {
                // Divora is quitting: the file is let go of unfinished — no
                // flush of what is queued, which on a slow disk would outlast
                // Quit's wait — and removed by the caller.
                w.abandon();
                return Err(CaptureError::Unavailable(CANCELLED.to_string()));
            }
        }
        match step {
            Write::Video(i) => {
                let au = &snap.video[i];
                w.write_video(au.index, au.key, &au.data)?;
            }
            Write::Pcm { from, to } => {
                let a = usize::try_from(from * 2).unwrap_or(usize::MAX);
                let b = usize::try_from(to * 2).unwrap_or(usize::MAX);
                if let Some(frames) = snap.pcm.get(a..b.min(snap.pcm.len())) {
                    w.write_pcm(frames)?;
                }
            }
        }
    }
    // The last chance: finishing flushes the whole queue and can't be
    // stopped once begun.
    if cancel.load(Ordering::Acquire) {
        w.abandon();
        return Err(CaptureError::Unavailable(CANCELLED.to_string()));
    }
    w.finish()?;
    Ok(passthrough)
}

/// Save `snap` as `req` asks, on a thread of its own (`divora-replay-save`),
/// and answer its reply. If the thread can't be started the request is
/// dropped, and its reply answers that the replay wasn't saved.
pub fn spawn(snap: Snapshot, req: SaveRequest) {
    let spawned = std::thread::Builder::new()
        .name("divora-replay-save".into())
        .spawn(move || {
            let SaveRequest {
                dir,
                name,
                reply,
                cancel,
                ..
            } = req;
            let r = save(&snap, &dir, &name, &cancel);
            match &r {
                Ok(c) => {
                    tracing::info!(path = %c.path.display(), ms = c.duration_ms, "replay saved");
                }
                Err(e) => tracing::warn!(error = %e, "replay not saved"),
            }
            reply.send(r);
        });
    if let Err(e) = spawned {
        tracing::warn!(error = %e, "couldn't start saving a replay");
    }
}

/// Write the clip to `dir/name` (or `name-2`, … if taken), through a
/// `.part` that is renamed once the file is whole. Nothing is left behind on
/// a failure: no space means no file at all, and anything else removes the
/// `.part`.
fn save(snap: &Snapshot, dir: &Path, name: &str, cancel: &AtomicBool) -> Result<SavedClip, String> {
    let _com = ComScope::enter();
    let session = mf::api()
        .and_then(mf::Mf::session)
        .map_err(|e| format!("The replay couldn't be saved: {e}"))?;
    std::fs::create_dir_all(dir).map_err(|e| format!("The replay couldn't be saved: {e}"))?;
    let need = snap.bytes() * 11 / 10;
    if free_bytes(dir).is_some_and(|free| free < need) {
        return Err(no_space(need));
    }
    let path = unique(dir, name);
    let part = part_path(&path);
    if let Err(e) = write_clip(&session, &part, snap, cancel) {
        // The writer is gone by now (dropped inside), so the file can go.
        remove_part(&part);
        return Err(match e {
            CaptureError::Unavailable(text) if text == CANCELLED => text,
            e => format!("The replay couldn't be saved: {e}"),
        });
    }
    if let Err(e) = std::fs::rename(&part, &path) {
        remove_part(&part);
        return Err(format!("The replay couldn't be saved: {e}"));
    }
    let bytes = std::fs::metadata(&path).map_or(snap.bytes(), |m| m.len());
    Ok(SavedClip {
        path,
        duration_ms: snap.duration_ms(),
        bytes,
    })
}

/// What a drive without room for the clip says.
/// Remove a `.part` that won't be finished. A writer let go of with writes
/// still in flight can hold the file a moment longer, so a refusal is tried
/// again for up to a second before the file is left.
fn remove_part(part: &Path) {
    for _ in 0..20 {
        match std::fs::remove_file(part) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
                std::thread::sleep(Duration::from_millis(50));
            }
            _ => return,
        }
    }
    tracing::warn!(path = %part.display(), "couldn't remove an unfinished replay");
}

fn no_space(need: u64) -> String {
    format!(
        "There isn't enough free space on the drive to save the replay (it needs about {} MB).",
        need.div_ceil(1_000_000)
    )
}

/// `dir/name`, or with `-2`, `-3`, … before the extension if that is taken.
fn unique(dir: &Path, name: &str) -> PathBuf {
    let first = dir.join(name);
    if !first.exists() && !part_path(&first).exists() {
        return first;
    }
    let (stem, ext) = match name.rsplit_once('.') {
        Some((s, e)) => (s, format!(".{e}")),
        None => (name, String::new()),
    };
    (2u32..=9_999)
        .map(|n| dir.join(format!("{stem}-{n}{ext}")))
        .find(|p| !p.exists() && !part_path(p).exists())
        .unwrap_or(first)
}

#[cfg(test)]
mod tests {
    use super::{no_space, remove_part, unique};
    use std::time::Duration;

    #[test]
    fn a_taken_name_gets_a_number() {
        let dir = std::env::temp_dir().join(format!("divora-unique-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let a = unique(&dir, "divora-replay-x.mp4");
        assert_eq!(a, dir.join("divora-replay-x.mp4"));
        std::fs::write(&a, b"").unwrap();
        assert_eq!(
            unique(&dir, "divora-replay-x.mp4"),
            dir.join("divora-replay-x-2.mp4")
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_part_still_held_is_removed_once_let_go() {
        use std::os::windows::fs::OpenOptionsExt;
        let dir = std::env::temp_dir().join(format!("divora-part-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let part = dir.join("held.mp4.part");
        std::fs::write(&part, b"x").unwrap();
        // Held as a writer let go of with writes in flight can hold it.
        let held = std::fs::OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(&part)
            .unwrap();
        assert!(
            std::fs::remove_file(&part).is_err(),
            "held, it can't go yet"
        );
        let t = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(200));
            drop(held);
        });
        remove_part(&part);
        t.join().unwrap();
        assert!(!part.exists(), "removed once it was let go of");
        remove_part(&part); // gone already: nothing to wait for
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn no_space_says_how_much() {
        assert_eq!(
            no_space(70_400_001),
            "There isn't enough free space on the drive to save the replay (it needs about 71 MB)."
        );
    }
}
