//! The v1.52.0 spike: record a monitor to an MP4 and report what happened.
//!
//!     cargo run -p divora-capture --release --example capture_spike -- [seconds] [fps] [out.mp4] [monitor-number-or-id] [tone]
//!
//! Defaults: 30 s, 30 fps, `capture-spike.mp4`, the primary monitor, no audio.
//! `tone` adds an audio track fed by a pretend engine: 10 ms buffers of a
//! quiet 440 Hz tone with a click on every whole second, stamped as the real
//! engine stamps its own — so the AAC path runs end to end without it.

use std::f64::consts::TAU;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant};

use divora_capture::timeline::AudioChunk;
use divora_capture::{seconds_since, AudioFeed};

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
        ..Default::default()
    };
    let stop = AtomicBool::new(false);
    let t0 = Instant::now();
    let mut feed = ToneFeed {
        origin: Instant::now(),
        index: 0,
    };
    let audio: Option<&mut dyn AudioFeed> = if tone { Some(&mut feed) } else { None };
    match divora_capture::record_monitor(
        &id,
        &out,
        &opts,
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
