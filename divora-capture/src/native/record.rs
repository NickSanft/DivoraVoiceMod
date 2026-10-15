//! One take: capture a monitor, pace it to a constant frame rate, encode —
//! and, given an [`AudioFeed`], lay the audio track beside it.

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use windows::Win32::Graphics::Direct3D11::ID3D11Texture2D;
use windows::Win32::Graphics::Gdi::HMONITOR;
use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_MULTITHREADED};

use super::d3d::{latest_frame_texture, Gpu, Nv12Converter, Readback};
use super::encoder::TakeWriter;
use super::find_monitor;
use super::mf;
use super::wgc::MonitorCapture;
use crate::error::CaptureError;
use crate::pacer::{FramePacer, TICKS_PER_SECOND};
use crate::sizing::{default_bitrate, output_size};
use crate::take::{part_path, AudioFeed, RecordOptions, RecordStats, TakeObserver};
use crate::track::AudioTrack;

/// How long to wait for the very first frame before giving up: a monitor
/// that is asleep produces none.
const FIRST_FRAME_TIMEOUT: Duration = Duration::from_secs(5);

/// The longest the loop goes without taking in audio, frames due or not. A
/// frame period at 30 fps is longer; the queues would cope, but the track
/// should not trail the picture by more than it must, and an encoder that
/// can't keep up must not starve the audio of its turns.
const AUDIO_POLL: Duration = Duration::from_millis(20);

/// COM for this thread, for the life of the take.
pub struct ComScope(bool);

impl ComScope {
    #[must_use]
    pub fn enter() -> Self {
        // SAFETY: initializes COM on the calling thread; balanced in Drop
        // only when this call succeeded (another mode already set is fine).
        let hr = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
        Self(hr.is_ok())
    }
}

impl Drop for ComScope {
    fn drop(&mut self) {
        if self.0 {
            // SAFETY: balances the successful CoInitializeEx above.
            unsafe { CoUninitialize() };
        }
    }
}

/// Set to anything to convert on the CPU even where the GPU could: how the
/// fallback is checked on a machine that would never take it.
pub const FORCE_CPU_ENV: &str = "DIVORA_CAPTURE_CPU_CONVERT";

/// How frames get from the captured BGRA texture to the encoder as NV12.
enum Converter {
    /// The GPU's video processor: the texture never leaves the GPU.
    Gpu(Nv12Converter),
    /// The fallback: read back and converted on the CPU, one reused buffer.
    Cpu(Readback, Vec<u8>),
}

impl Converter {
    fn new(
        dev: &Gpu,
        latest: &ID3D11Texture2D,
        in_size: (u32, u32),
        out_size: (u32, u32),
        fps: u32,
    ) -> Result<Self, CaptureError> {
        let gpu = if std::env::var_os(FORCE_CPU_ENV).is_some() {
            Err(CaptureError::Unavailable(format!("{FORCE_CPU_ENV} is set")))
        } else {
            Nv12Converter::new(dev, latest, in_size, out_size, fps)
        };
        match gpu {
            Ok(c) => Ok(Self::Gpu(c)),
            Err(e) => {
                tracing::warn!(error = %e, "no GPU colour converter; converting on the CPU");
                let buf = vec![0; crate::nv12::nv12_len(out_size)];
                Ok(Self::Cpu(Readback::new(dev, in_size, out_size)?, buf))
            }
        }
    }

    /// The same kind of converter for a new input size: the writer was
    /// opened for textures or for bytes, and that cannot change mid-take.
    /// The CPU fallback stretches a changed shape; the GPU letterboxes it.
    fn rebuilt(
        &self,
        dev: &Gpu,
        latest: &ID3D11Texture2D,
        in_size: (u32, u32),
        out_size: (u32, u32),
        fps: u32,
    ) -> Result<Self, CaptureError> {
        Ok(match self {
            Self::Gpu(_) => Self::Gpu(Nv12Converter::new(dev, latest, in_size, out_size, fps)?),
            Self::Cpu(..) => Self::Cpu(
                Readback::new(dev, in_size, out_size)?,
                vec![0; crate::nv12::nv12_len(out_size)],
            ),
        })
    }

    fn write(
        &mut self,
        dev: &Gpu,
        latest: &ID3D11Texture2D,
        writer: &TakeWriter,
        pts: u64,
        duration: u64,
    ) -> Result<(), CaptureError> {
        match self {
            Self::Gpu(c) => writer.write(&c.convert(dev)?, pts, duration),
            Self::Cpu(r, buf) => {
                r.convert_into(dev, latest, buf)?;
                writer.write_memory(buf, pts, duration)
            }
        }
    }
}

/// The monitor being captured and what its frames go through: the newest
/// frame's texture and the converter reading it. A monitor that changes size
/// mid-take (a game switching resolution, a DPI change) rebuilds all three at
/// the new size; the take's encoded size stays put.
struct Screen {
    capture: MonitorCapture,
    latest: ID3D11Texture2D,
    converter: Converter,
    out_size: (u32, u32),
    fps: u32,
}

impl Screen {
    /// Start capturing `monitor`, at an encoded size fitting `opts.max_size`.
    fn open(dev: &Gpu, monitor: HMONITOR, opts: &RecordOptions) -> Result<Self, CaptureError> {
        let capture = MonitorCapture::start(dev, monitor, opts.cursor)?;
        let in_size = capture.size();
        let out_size = output_size(in_size, opts.max_size);
        let fps = opts.fps.clamp(1, 120);
        let latest = latest_frame_texture(dev, in_size)?;
        let converter = Converter::new(dev, &latest, in_size, out_size, fps)?;
        Ok(Self {
            capture,
            latest,
            converter,
            out_size,
            fps,
        })
    }

    /// Take every frame waiting into `latest`; how many arrived.
    fn drain(&mut self, dev: &Gpu) -> Result<u64, CaptureError> {
        let d = self.capture.drain_into(dev, &self.latest)?;
        let Some(size) = d.resized else {
            return Ok(d.frames);
        };
        tracing::info!(
            width = size.0,
            height = size.1,
            "the screen changed size mid-take"
        );
        self.capture.recreate(dev, size)?;
        self.latest = latest_frame_texture(dev, size)?;
        self.converter =
            self.converter
                .rebuilt(dev, &self.latest, size, self.out_size, self.fps)?;
        // Nothing usable arrived: the next frame comes at the new size.
        Ok(0)
    }

    fn write(
        &mut self,
        dev: &Gpu,
        writer: &TakeWriter,
        pts: u64,
        duration: u64,
    ) -> Result<(), CaptureError> {
        self.converter
            .write(dev, &self.latest, writer, pts, duration)
    }
}

/// Wait for the monitor's first frame: the number of frames that arrived,
/// or `None` if `stop` was set first.
fn first_frame(
    screen: &mut Screen,
    dev: &Gpu,
    stop: &AtomicBool,
) -> Result<Option<u64>, CaptureError> {
    let wait_start = Instant::now();
    loop {
        if stop.load(Ordering::Acquire) {
            return Ok(None);
        }
        if screen.capture.wait(Duration::from_millis(50)) {
            let frames = screen.drain(dev)?;
            if frames > 0 {
                return Ok(Some(frames));
            }
        }
        if wait_start.elapsed() > FIRST_FRAME_TIMEOUT {
            return Err(CaptureError::Unavailable(
                "That screen isn't sending any picture (is it asleep?).".to_string(),
            ));
        }
    }
}

/// Where the track ends: at the end of the last frame written, in seconds.
fn end_seconds(pacer: &FramePacer) -> f64 {
    #[allow(clippy::cast_precision_loss)] // ticks in a take: exact far past any length
    let end = pacer.end_pts() as f64 / TICKS_PER_SECOND as f64;
    end
}

/// How often [`TakeObserver::progress`] is called.
const PROGRESS_EVERY: Duration = Duration::from_secs(1);

/// Record the monitor `monitor_id` to `path` until `stop` is set or
/// `max_duration` has been recorded, with an audio track from `audio` if
/// given, telling `observer` how it goes. Blocks the calling thread; run it
/// on a thread of its own.
///
/// A take stopped before its first frame arrives writes no file and
/// returns empty stats.
pub fn record_monitor(
    monitor_id: &str,
    path: &Path,
    opts: &RecordOptions,
    stop: &AtomicBool,
    max_duration: Option<Duration>,
    audio: Option<&mut dyn AudioFeed>,
    observer: &mut dyn TakeObserver,
) -> Result<RecordStats, CaptureError> {
    let _com = ComScope::enter();
    let api = mf::api()?;
    let session = api.session()?;
    let monitor = find_monitor(monitor_id).ok_or(CaptureError::NoSuchMonitor)?;

    let dev = Gpu::new()?;
    let mut screen = Screen::open(&dev, monitor, opts)?;
    let (out_size, fps) = (screen.out_size, screen.fps);
    let bitrate = opts
        .bitrate
        .unwrap_or_else(|| default_bitrate(out_size, fps));

    let part = part_path(path);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }

    let mut stats = RecordStats {
        width: out_size.0,
        height: out_size.1,
        ..RecordStats::default()
    };

    // Start on real content, not a black first frame.
    match first_frame(&mut screen, &dev, stop)? {
        Some(frames) => stats.frames_arrived += frames,
        None => return Ok(stats),
    }

    let gpu_frames = matches!(screen.converter, Converter::Gpu(_)).then_some(&dev);
    let writer = TakeWriter::new(
        &session,
        gpu_frames,
        &part,
        out_size,
        fps,
        bitrate,
        audio.is_some(),
    )?;
    let mut track = audio.map(|feed| AudioTrack::new(feed, opts.audio, path));
    let mut write_audio = |block: &crate::timeline::AudioBlock| writer.write_audio(block);
    let mut pacer = FramePacer::new(fps);
    let start = Instant::now();
    let mut changed_since_write = true;
    observer.started();
    let mut last_progress = start;
    let mut last_audio = start;

    loop {
        let elapsed = start.elapsed();
        if stop.load(Ordering::Acquire) || max_duration.is_some_and(|m| elapsed >= m) {
            break;
        }
        if let Some(emit) = pacer.due(elapsed) {
            screen.write(&dev, &writer, emit.pts, emit.duration)?;
            stats.frames_written += 1;
            if !changed_since_write {
                stats.frames_repeated += 1;
            }
            changed_since_write = false;
            // Frames due back to back (an encoder that can't keep up) still
            // leave the audio its turns.
            if last_audio.elapsed() < AUDIO_POLL {
                continue;
            }
        }
        if let Some(t) = track.as_mut() {
            t.pump(start, &mut write_audio)?;
            last_audio = Instant::now();
        }
        if last_progress.elapsed() >= PROGRESS_EVERY {
            last_progress = Instant::now();
            stats.frames_dropped = pacer.dropped();
            stats.audio = track.as_ref().map(AudioTrack::stats);
            observer.progress(&stats);
        }
        // Sleep until the next tick — or the next audio poll, whichever is
        // sooner — waking early to take a new frame.
        let mut until_next = pacer.next_deadline().saturating_sub(start.elapsed());
        if track.is_some() {
            until_next = until_next.min(AUDIO_POLL);
        }
        if screen.capture.wait(until_next) {
            let frames = screen.drain(&dev)?;
            if frames > 0 {
                stats.frames_arrived += frames;
                changed_since_write = true;
            }
        }
    }

    stats.frames_dropped = pacer.dropped();
    let (name, hardware) = writer.encoder_info();
    stats.encoder = name;
    stats.hardware = hardware;
    // Windows' yellow border goes as soon as the picture is done.
    let Screen {
        capture, converter, ..
    } = screen;
    drop(capture);
    if let Some(t) = track.as_mut() {
        t.finish(start, end_seconds(&pacer), &mut write_audio)?;
    }
    writer.finish()?;
    drop(converter);
    drop(session);
    std::fs::rename(&part, path)?;
    if let Some(mut t) = track {
        t.finish_stems();
        stats.audio = Some(t.stats());
    }
    Ok(stats)
}
