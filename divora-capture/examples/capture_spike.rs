//! The v1.52.0 spike: record a monitor to an MP4 and report what happened.
//!
//!     cargo run -p divora-capture --release --example capture_spike -- [seconds] [fps] [out.mp4] [monitor-number-or-id] [tone] [direct] [repeat=N] [max=WxH]
//!
//! Defaults: 30 s, 30 fps, `capture-spike.mp4`, the primary monitor, no audio.
//! v1.54.0: `direct` records through the directly driven encoder (the day-4
//! gate) instead of the sink writer; `repeat=N` records N takes in a row in
//! one process (`out-1.mp4`, …), with a pause between them to read the
//! process's memory; `max=WxH` caps the encoded size. With
//! `DIVORA_DIRECT_QUIESCE` set, direct takes end with frames in flight.
//! `replay=N` runs the replay pipeline instead: Replay keeping the last N
//! seconds, saved once `seconds` have passed (to `out.mp4`), with `take`
//! also recording a take from a third of the way in to two thirds.
//! `tone` adds an audio track fed by a pretend engine: 10 ms buffers of a
//! quiet 440 Hz tone with a click on every whole second, stamped as the real
//! engine stamps its own — so the AAC path runs end to end without it.

use std::f64::consts::TAU;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64};
use std::sync::mpsc::{channel, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use divora_capture::replay::budget;
use divora_capture::timeline::AudioChunk;
use divora_capture::{
    seconds_since, AudioFeed, HeldReason, PipelineCommand, PipelineEvents, PipelineSettings,
    RecordStats, RingStatus, SaveReply, SaveRequest, TakeEvents, TakeRequest,
};

/// A pretend engine at 48 kHz: it makes whatever buffers are due by now.
struct ToneFeed {
    origin: Instant,
    index: u64,
}

impl AudioFeed for ToneFeed {
    #[allow(
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss
    )]
    fn poll(&mut self, start: Instant, out: &mut Vec<AudioChunk>) {
        const SPAN: u64 = 480;
        let due = (self.origin.elapsed().as_secs_f64() * 48_000.0) as u64;
        while self.index + SPAN <= due {
            let at = self.origin + Duration::from_secs_f64(self.index as f64 / 48_000.0);
            let take = seconds_since(start, at);
            let sb = (0..SPAN)
                .map(|i| {
                    let t = take + i as f64 / 48_000.0;
                    let click = if t.fract().abs() < 0.002 && t > 0.0 {
                        0.7
                    } else {
                        0.0
                    };
                    (0.1 * (TAU * 440.0 * t).sin() + click) as f32
                })
                .collect();
            out.push(AudioChunk {
                epoch: 1,
                at: take,
                index: self.index,
                rate: 48_000,
                out_frames: 480,
                out_rate: 48_000,
                voice_wait: 0.0,
                chain_latency: 0,
                voice: vec![0.0; SPAN as usize],
                sb,
            });
            self.index += SPAN;
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let seconds: u64 = args.first().and_then(|s| s.parse().ok()).unwrap_or(30);
    let fps: u32 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(30);
    let out = PathBuf::from(args.get(2).map_or("capture-spike.mp4", String::as_str));
    let tone = args.get(4).is_some_and(|a| a == "tone");
    let direct = args.iter().any(|a| a == "direct");
    let repeat: usize = args
        .iter()
        .find_map(|a| a.strip_prefix("repeat=").and_then(|n| n.parse().ok()))
        .unwrap_or(1);
    let max = args.iter().find_map(|a| {
        let (w, h) = a.strip_prefix("max=")?.split_once('x')?;
        Some((w.parse().ok()?, h.parse().ok()?))
    });

    let status = divora_capture::capture_status();
    println!("status: {status:?}");
    if !status.supported {
        std::process::exit(2);
    }
    let monitors = divora_capture::list_monitors();
    for m in &monitors {
        println!("monitor: {m:?}");
    }
    // A monitor number (0 = primary) or an id; shells mangle the
    // backslashes in ids like `\\.\DISPLAY1`, so numbers are easier.
    let id = match args.get(3) {
        Some(a) => match a.parse::<usize>() {
            Ok(i) => monitors
                .get(i)
                .map(|m| m.id.clone())
                .expect("no such monitor number"),
            Err(_) => a.clone(),
        },
        None => monitors.first().map(|m| m.id.clone()).expect("no monitor"),
    };
    println!(
        "recording {id}{}",
        if tone { " with a tone track" } else { "" }
    );

    let opts = divora_capture::RecordOptions {
        fps,
        max_size: max.unwrap_or(divora_capture::sizing::DEFAULT_MAX),
        ..Default::default()
    };
    println!("pid {}", std::process::id());
    if let Some(keep) = args
        .iter()
        .find_map(|a| a.strip_prefix("replay=").and_then(|n| n.parse().ok()))
    {
        let with_take = args.iter().any(|a| a == "take");
        replay(&id, &out, &opts, seconds, keep, with_take);
        return;
    }
    for n in 1..=repeat {
        let out = if repeat > 1 {
            out.with_file_name(format!(
                "{}-{n}.mp4",
                out.file_stem().and_then(|s| s.to_str()).unwrap_or("take")
            ))
        } else {
            out.clone()
        };
        take(&id, &out, &opts, seconds, fps, tone, direct);
        if repeat > 1 {
            println!("take {n} of {repeat} done; pausing 3 s");
            std::thread::sleep(Duration::from_secs(3));
        }
    }
}

fn take(
    id: &str,
    out: &std::path::Path,
    opts: &divora_capture::RecordOptions,
    seconds: u64,
    fps: u32,
    tone: bool,
    direct: bool,
) {
    let stop = AtomicBool::new(false);
    let t0 = Instant::now();
    let mut feed = ToneFeed {
        origin: Instant::now(),
        index: 0,
    };
    let audio: Option<&mut dyn AudioFeed> = if tone { Some(&mut feed) } else { None };
    let record = if direct {
        divora_capture::record_monitor_direct
    } else {
        divora_capture::record_monitor
    };
    match record(
        id,
        out,
        opts,
        &stop,
        Some(Duration::from_secs(seconds)),
        audio,
        &mut (),
    ) {
        Ok(take) => {
            let expected = seconds * u64::from(fps);
            #[allow(clippy::cast_precision_loss)]
            let dropped_pct = 100.0 * take.frames_dropped as f64 / expected.max(1) as f64;
            println!("stats: {take:?}");
            println!(
                "wall {:.2}s, expected ~{expected} frames, written {}, dropped {} ({dropped_pct:.2}%), repeated {}",
                t0.elapsed().as_secs_f64(),
                take.frames_written,
                take.frames_dropped,
                take.frames_repeated
            );
            println!("wrote {}", out.display());
        }
        Err(e) => {
            eprintln!("record failed: {e}");
            std::process::exit(1);
        }
    }
}

/// What the pipeline says, printed as it comes.
struct Said(Mutex<Sender<String>>);

impl Said {
    fn say(&self, s: String) {
        let _ = self.0.lock().map(|tx| tx.send(s));
    }
}

impl PipelineEvents for Said {
    fn ring_ready(&self) {
        self.say("ring ready".into());
    }
    fn ring(&self, s: RingStatus) {
        self.say(format!(
            "ring: {} ms, {} of {} bytes",
            s.buffered_ms, s.bytes, s.cap_bytes
        ));
    }
    fn held(&self, reason: HeldReason, detail: Option<String>) {
        self.say(format!("held: {reason:?} {detail:?}"));
    }
    fn resumed(&self) {
        self.say("resumed".into());
    }
    fn rebuilding(&self) {
        self.say("rebuilding".into());
    }
    fn arm_refused(&self, why: String) {
        self.say(format!("arm refused: {why}"));
    }
    fn direct_failed(&self, why: String) {
        self.say(format!("direct failed: {why}"));
    }
    fn failed(&self, why: String) {
        self.say(format!("replay turned off: {why}"));
    }
    fn ended(&self, error: Option<String>) {
        self.say(format!("ended {error:?}"));
    }
}

impl TakeEvents for Said {
    fn started(&self) {
        self.say("take started".into());
    }
    fn progress(&self, _stats: &RecordStats) {}
    fn finished(&self, outcome: Result<PathBuf, String>, stats: RecordStats) {
        self.say(format!(
            "take finished: {outcome:?}, {} frames, {} dropped",
            stats.frames_written, stats.frames_dropped
        ));
    }
}

/// Replay keeping `keep` seconds, saved after `seconds`.
fn replay(
    id: &str,
    out: &Path,
    opts: &divora_capture::RecordOptions,
    seconds: u64,
    keep: u32,
    with_take: bool,
) {
    let monitor = divora_capture::list_monitors()
        .into_iter()
        .find(|m| m.id == id)
        .expect("the monitor");
    let size = divora_capture::sizing::output_size((monitor.width, monitor.height), opts.max_size);
    let bitrate = divora_capture::sizing::default_bitrate(size, opts.fps);
    let settings = PipelineSettings {
        monitor_id: id.to_string(),
        fps: opts.fps,
        max_size: opts.max_size,
        bitrate,
        cursor: true,
        track: opts.audio,
    };
    let cap_bytes = budget::cap_bytes(budget::estimate_bytes(keep, bitrate));
    let (tx, rx) = channel();
    let said = Arc::new(Said(Mutex::new(tx)));
    let feed = Box::new(ToneFeed {
        origin: Instant::now(),
        index: 0,
    });
    let t0 = Instant::now();
    let h = match divora_capture::spawn_pipeline(
        settings,
        feed,
        Arc::clone(&said) as Arc<dyn PipelineEvents>,
        PipelineCommand::Arm {
            seconds: keep,
            cap_bytes,
        },
    ) {
        Ok(h) => h,
        Err(e) => {
            eprintln!("the pipeline didn't start: {e}");
            std::process::exit(1);
        }
    };
    println!("armed {keep} s (cap {cap_bytes} B) in {:?}", t0.elapsed());
    let print = |until: Instant| {
        while Instant::now() < until {
            if let Ok(s) = rx.recv_timeout(Duration::from_millis(100)) {
                if !s.starts_with("ring:") || s.contains("000 ms") {
                    println!("{:7.2} s  {s}", t0.elapsed().as_secs_f64());
                }
            }
        }
    };
    let third = Duration::from_secs(seconds) / 3;
    if with_take {
        print(t0 + third);
        let take_path = out.with_file_name("replay-spike-take.mp4");
        let _ = h.send(PipelineCommand::StartTake(TakeRequest {
            take: 1,
            path: take_path,
            stems: false,
            events: Arc::clone(&said) as Arc<dyn TakeEvents>,
        }));
        print(t0 + 2 * third);
        let _ = h.send(PipelineCommand::StopTake { take: 1 });
    }
    print(t0 + Duration::from_secs(seconds));
    let (rtx, rrx) = channel();
    let pressed = Instant::now();
    let _ = h.send(PipelineCommand::Save(SaveRequest {
        at: pressed,
        dir: out
            .parent()
            .map_or_else(|| PathBuf::from("."), Path::to_path_buf),
        name: out
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("replay-spike.mp4")
            .to_string(),
        reply: SaveReply::new(move |r| {
            let _ = rtx.send(r);
        }),
        size: Arc::new(AtomicU64::new(0)),
        cancel: Arc::new(AtomicBool::new(false)),
    }));
    match rrx.recv_timeout(Duration::from_secs(30)) {
        Ok(r) => println!("saved in {:?}: {r:?}", pressed.elapsed()),
        Err(_) => println!("no answer to the save"),
    }
    let _ = h.send(PipelineCommand::Shutdown);
    let joined = h.join(Duration::from_secs(5));
    print(Instant::now() + Duration::from_millis(500));
    println!("ended: {joined}; wall {:.2} s", t0.elapsed().as_secs_f64());
}
