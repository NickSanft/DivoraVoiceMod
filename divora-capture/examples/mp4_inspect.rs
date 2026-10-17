//! Prints what `mp4_box` reads from an MP4: the boxes the v1.54.0 gate checks.
//!
//!     cargo run -p divora-capture --example mp4_inspect -- file.mp4
//!
//! The gate's MP4 check in one look: which samples `stss` marks as keyframes,
//! and whether those are exactly the samples holding an IDR (NAL type 5);
//! the `stts` runs; any `ctts` or edit list; `colr`; the `avcC` parameter
//! sets and NAL length size; and how long each track and the movie say they
//! are. Any file the reader refuses — cut short, never finalized — is said
//! so and exits 1.

use std::path::Path;
use std::process::ExitCode;

use divora_capture::mp4_box::{self, avcc_nal_types, Mp4, SampleEntry, Track};

/// How many entries of a long list to print before "… N in all".
const SHOWN: usize = 12;

fn main() -> ExitCode {
    let Some(path) = std::env::args_os().nth(1) else {
        eprintln!("usage: mp4_inspect <file.mp4>");
        return ExitCode::from(2);
    };
    let path = Path::new(&path);
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(e) => {
            eprintln!("{}: {e}", path.display());
            return ExitCode::FAILURE;
        }
    };
    match mp4_box::parse(&bytes) {
        Ok(mp4) => {
            print_file(path, &bytes, &mp4);
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("{}: not a readable MP4: {e}", path.display());
            ExitCode::FAILURE
        }
    }
}

fn print_file(path: &Path, bytes: &[u8], mp4: &Mp4) {
    println!("{} ({} bytes)", path.display(), bytes.len());
    let top: Vec<String> = mp4
        .top
        .iter()
        .map(|b| format!("{} @{} ({} B)", b.kind, b.offset, b.size))
        .collect();
    println!("boxes: {}", top.join(", "));
    if let Some(f) = &mp4.ftyp {
        let compatible: Vec<String> = f.compatible.iter().map(ToString::to_string).collect();
        println!(
            "ftyp: {} {}, compatible {}",
            f.major,
            f.minor,
            compatible.join(" ")
        );
    }
    println!(
        "movie: {} (mvhd, timescale {})",
        seconds(mp4.duration, mp4.timescale),
        mp4.timescale
    );
    for (n, track) in mp4.tracks.iter().enumerate() {
        print_track(n + 1, track, bytes);
    }
}

fn print_track(n: usize, t: &Track, bytes: &[u8]) {
    println!("track {n} (id {}, {}):", t.track_id, t.handler);
    match &t.entry {
        SampleEntry::Video {
            kind,
            width,
            height,
            avcc,
            colr,
        } => {
            println!("  entry: {kind} {width}x{height}");
            match avcc {
                Some(c) => println!(
                    "  avcC: profile {} level {}, NAL length size {}, SPS {}, PPS {}",
                    c.profile,
                    c.level,
                    c.length_size,
                    hex_list(&c.sps),
                    hex_list(&c.pps)
                ),
                None => println!("  avcC: none"),
            }
            match colr {
                Some(c) => println!(
                    "  colr: {} {}/{}/{} {}",
                    c.kind,
                    c.primaries,
                    c.transfer,
                    c.matrix,
                    if c.full_range { "full" } else { "limited" }
                ),
                None => println!("  colr: none"),
            }
        }
        SampleEntry::Audio {
            kind,
            channels,
            sample_rate,
        } => println!("  entry: {kind}, {channels} channel(s), {sample_rate} Hz"),
        SampleEntry::Other { kind } => println!("  entry: {kind}"),
    }
    println!(
        "  duration: {} (mdhd v{}: {} at {}/s)",
        seconds(t.duration, t.timescale),
        t.mdhd_version,
        t.duration,
        t.timescale
    );
    match &t.edits {
        None => println!("  edit list: none (no edts)"),
        Some(edits) => println!("  edit list: {edits:?}"),
    }
    println!(
        "  stts: {} run(s): {}",
        t.stts.len(),
        shorten(
            &t.stts
                .iter()
                .map(|(n, d)| format!("{n}×{d}"))
                .collect::<Vec<_>>()
        )
    );
    match &t.ctts {
        None => println!("  ctts: none"),
        Some(runs) => {
            let lo = runs.iter().map(|r| r.1).min().unwrap_or(0);
            let hi = runs.iter().map(|r| r.1).max().unwrap_or(0);
            println!("  ctts: {} run(s), offsets {lo}..={hi}", runs.len());
        }
    }
    println!(
        "  samples: {} in {} chunk(s), offsets from {}",
        t.sample_count(),
        t.chunk_offsets.len(),
        if t.co64 { "co64" } else { "stco" }
    );
    match &t.stss {
        None => println!("  stss: none (every sample is a sync sample)"),
        Some(sync) => println!("  stss: {}", numbers(sync)),
    }
    if let Some(c) = t.avcc() {
        print_idrs(t, bytes, c.length_size);
    }
}

/// Which samples hold an IDR, by their NAL units, against what `stss` says;
/// and the first sample's NAL types (AVCC framing: no 7 or 8 expected).
fn print_idrs(t: &Track, bytes: &[u8], length_size: u8) {
    let mut idrs = Vec::new();
    let mut unreadable = 0usize;
    for i in 0..t.sample_count() {
        match t.sample(bytes, i).map(|s| avcc_nal_types(s, length_size)) {
            Some(Ok(types)) if types.contains(&5) => {
                idrs.push(u32::try_from(i + 1).unwrap_or(u32::MAX));
            }
            Some(Ok(_)) => {}
            _ => unreadable += 1,
        }
    }
    println!("  IDR samples (NAL 5): {}", numbers(&idrs));
    let verdict = match &t.stss {
        Some(sync) if *sync == idrs => "the same as stss",
        None if idrs.len() == t.sample_count() => "every sample, as no stss says",
        _ => "NOT the same as stss",
    };
    println!("  IDR samples vs stss: {verdict}");
    if unreadable > 0 {
        println!("  samples not AVCC-framed: {unreadable}");
    }
    if let Some(first) = t.sample(bytes, 0) {
        match avcc_nal_types(first, length_size) {
            Ok(types) => println!("  first sample NAL types: {types:?}"),
            Err(e) => println!("  first sample: not AVCC-framed ({e})"),
        }
    }
}

/// `duration / timescale` as seconds with milliseconds.
fn seconds(duration: u64, timescale: u32) -> String {
    if timescale == 0 {
        return "? s (timescale 0)".to_string();
    }
    let ms = u128::from(duration) * 1000 / u128::from(timescale);
    format!("{}.{:03} s", ms / 1000, ms % 1000)
}

fn numbers(list: &[u32]) -> String {
    shorten(&list.iter().map(ToString::to_string).collect::<Vec<_>>())
}

fn shorten(items: &[String]) -> String {
    if items.is_empty() {
        return "(empty)".to_string();
    }
    let shown = items
        .iter()
        .take(SHOWN)
        .cloned()
        .collect::<Vec<_>>()
        .join(", ");
    if items.len() > SHOWN {
        format!("{shown}, … ({} in all)", items.len())
    } else {
        shown
    }
}

fn hex_list(sets: &[Vec<u8>]) -> String {
    let each: Vec<String> = sets
        .iter()
        .map(|set| {
            let hex: Vec<String> = set.iter().take(16).map(|b| format!("{b:02x}")).collect();
            let more = if set.len() > 16 { " …" } else { "" };
            format!("[{} B: {}{more}]", set.len(), hex.join(" "))
        })
        .collect();
    if each.is_empty() {
        "none".to_string()
    } else {
        each.join(" ")
    }
}
