//! A replay clip, end to end, on any Windows machine — CI included
//! (v1.54.0).
//!
//! Windows' own software encoder (the one CI has) is driven directly over 96
//! synthetic frames at 30 fps — a keyframe every 30, one more forced at 45 —
//! into a two-second replay ring, with a 440 Hz tone beside it. Save at frame
//! 96 (3.2 s), and the clip must start on the keyframe at 30: the latest one
//! that still leaves two seconds. The MP4 that comes out is read back box by
//! box: the video went in as it was, starting on a keyframe, every keyframe
//! in the sync table, no edit list, the audio from the same first frame.
//!
//! Two more cases: a muxer fed from frame 5 skips to the first keyframe, and
//! a take cut from the shared audio at frame 30 starts its audio at zero.
//!
//! A machine without Media Foundation or an H.264 encoder returns early,
//! saying so. Without an AAC encoder the video alone is checked.

#![cfg(windows)]

use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use divora_capture::annexb::{has_idr, parameter_sets};
use divora_capture::mp4_box::{avcc_nal_types, parse, SampleEntry};
use divora_capture::nv12::nv12_len;
use divora_capture::replay::{sample_of, AccessUnit, ReplayRing, RingConfig};
use divora_capture::smoke::{
    api, software_encoders, write_clip, ComScope, EncoderConfig, EncoderEngine, EncoderEvent,
    EncoderInput, EncoderSession, MfSession, Mp4Writer, VideoFormat,
};
use divora_capture::take_cut::{BlockTail, TakeAudioCut};
use divora_capture::timeline::{AudioBlock, BLOCK_FRAMES, OUT_RATE};

fn say(line: &str) {
    use std::io::Write;
    let _ = writeln!(std::io::stderr(), "replay smoke: {line}");
}

const SIZE: (u32, u32) = (320, 240);
const FPS: u32 = 30;
const FRAMES: u64 = 96;

fn temp_mp4(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("divora-{name}-{}.mp4", std::process::id()))
}

fn frame(i: u64) -> Vec<u8> {
    let (w, h) = (SIZE.0 as usize, SIZE.1 as usize);
    let mut v = vec![128u8; nv12_len(SIZE)];
    for y in 0..h {
        for x in 0..w {
            #[allow(clippy::cast_possible_truncation)] // wraps on purpose
            let luma = (x * 2 + y + i as usize * 5) as u8;
            v[y * w + x] = luma.clamp(16, 235);
        }
    }
    v
}

fn pts(i: u64) -> i64 {
    i64::try_from(i * 10_000_000 / u64::from(FPS)).unwrap()
}

/// The 96 access units, in order.
fn encode(session: &MfSession) -> (Vec<AccessUnit>, Arc<[u8]>) {
    let cfg = EncoderConfig {
        size: SIZE,
        fps: FPS,
        bitrate: 1_000_000,
        gop_frames: 30,
        low_latency: true,
        engine: EncoderEngine::Software,
    };
    let mut enc = EncoderSession::new(session, None, &cfg).expect("the software encoder");
    let header = enc.sequence_header().cloned().expect("a sequence header");
    let mut events = Vec::new();
    for i in 0..FRAMES {
        enc.poll(&mut events).expect("poll");
        let bytes = frame(i);
        enc.encode(
            &EncoderInput::Memory(&bytes),
            pts(i),
            pts(i + 1) - pts(i),
            i == 45,
        )
        .expect("encode");
    }
    enc.drain(std::time::Duration::from_millis(500), &mut events)
        .expect("drain");
    let units: Vec<AccessUnit> = events
        .into_iter()
        .filter_map(|e| match e {
            EncoderEvent::Unit(u) => Some(u),
            _ => None,
        })
        .map(|u| {
            // The frame this output was submitted as, by its exact time (30
            // fps is not a whole number of ticks, so no division undoes it).
            let index = (0..FRAMES)
                .find(|&i| pts(i) == u.pts)
                .expect("a submitted time");
            AccessUnit::new(index, FPS, u.data)
        })
        .collect();
    assert_eq!(units.len() as u64, FRAMES);
    (units, header)
}

/// A 440 Hz tone, interleaved stereo, from 48 kHz frame `first` for `n`.
fn tone(first: u64, n: u64) -> Vec<i16> {
    (first..first + n)
        .flat_map(|k| {
            #[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]
            let s = (8_000.0
                * (std::f64::consts::TAU * 440.0 * k as f64 / f64::from(OUT_RATE)).sin())
                as i16;
            [s, s]
        })
        .collect()
}

struct Checked {
    samples: usize,
    stss: Option<Vec<u32>>,
}

/// Read `bytes` back and check what every clip and take must be.
fn check(bytes: &[u8], header: &[u8], with_audio: bool, audio_frames: u64) -> Checked {
    let mp4 = parse(bytes).expect("a parsable MP4");
    assert_eq!(&mp4.top[0].kind, b"ftyp", "ftyp first");
    let v = mp4.video().expect("a video track");
    let SampleEntry::Video {
        kind,
        width,
        height,
        avcc,
        colr,
    } = &v.entry
    else {
        panic!("not a video entry")
    };
    assert_eq!(kind, b"avc1");
    assert_eq!((u32::from(*width), u32::from(*height)), SIZE);
    let avcc = avcc.as_ref().expect("avcC");
    assert_eq!(avcc.length_size, 4);
    let ps = parameter_sets(header).expect("SPS and PPS in the header");
    let mut expect_sps_pps = Vec::new();
    for nal in divora_capture::annexb::nal_units(&ps) {
        expect_sps_pps.push(nal.bytes.to_vec());
    }
    let mut got = avcc.sps.clone();
    got.extend(avcc.pps.iter().cloned());
    assert_eq!(
        got, expect_sps_pps,
        "avcC carries the encoder's SPS and PPS"
    );
    assert_eq!(v.stts.len(), 1, "one frame duration throughout");
    assert!(v.ctts.is_none(), "no composition offsets");
    assert!(
        mp4.tracks.iter().all(|t| t.edits.is_none()),
        "no edit lists"
    );
    let first = v.sample(bytes, 0).expect("the first sample");
    let nals = avcc_nal_types(first, avcc.length_size).expect("AVCC framing");
    assert!(nals.contains(&5), "the first sample is an IDR: {nals:?}");
    assert!(
        !nals.contains(&7) && !nals.contains(&8),
        "the sink moved SPS/PPS into avcC: {nals:?}"
    );
    let c = colr.as_ref().expect("colr");
    assert_eq!(
        (c.primaries, c.transfer, c.matrix, c.full_range),
        (1, 1, 1, false),
        "BT.709, studio range"
    );
    if with_audio {
        let a = mp4.audio().expect("an audio track");
        let SampleEntry::Audio {
            kind, sample_rate, ..
        } = &a.entry
        else {
            panic!("not an audio entry")
        };
        assert_eq!(kind, b"mp4a");
        assert_eq!(*sample_rate, OUT_RATE);
        let d = i64::try_from(a.duration * u64::from(OUT_RATE) / u64::from(a.timescale)).unwrap();
        let want = i64::try_from(audio_frames).unwrap();
        assert!(
            (d - want).abs() <= i64::try_from(BLOCK_FRAMES).unwrap(),
            "audio lasts {d} frames, want {want}"
        );
        assert!(
            a.ctts
                .as_ref()
                .is_none_or(|c| c.iter().all(|&(_, o)| o == 0)),
            "audio starts at zero"
        );
    }
    Checked {
        samples: v.sizes.len(),
        stss: v.stss.clone(),
    }
}

/// Media Foundation, CI's software encoder, and whether AAC is there too;
/// `None` (said why) on a machine that can't.
fn setup() -> Option<(ComScope, MfSession, bool)> {
    let com = ComScope::enter();
    let Ok(api) = api() else {
        say("skipped: Media Foundation does not load here");
        return None;
    };
    let Ok(session) = api.session() else {
        say("skipped: MFStartup failed");
        return None;
    };
    let (h264, aac) = software_encoders(&session);
    if !h264 {
        say("skipped: no H.264 encoder is registered here");
        return None;
    }
    Some((com, session, aac))
}

#[test]
fn a_replay_clip_is_written_as_it_was_encoded() {
    let Some((_com, session, aac)) = setup() else {
        return;
    };
    let (units, header) = encode(&session);
    let idrs: Vec<u64> = units.iter().filter(|u| u.key).map(|u| u.index).collect();
    assert_eq!(idrs, vec![0, 30, 45, 75]);
    assert!(units.iter().all(|u| u.key == has_idr(&u.data)));

    // The ring: two seconds, the tone beside the video.
    let mut ring = ReplayRing::new(RingConfig {
        fps: FPS,
        seconds: 2,
        cap_bytes: u64::MAX / 4,
    });
    ring.start_segment(Some(header.clone()), 0);
    let audio_end = sample_of(FRAMES, FPS) + 4 * BLOCK_FRAMES as u64;
    let mut pushed_to = 0u64;
    for u in &units {
        // Audio arrives about as the frames do, in 1024-frame blocks.
        while pushed_to < sample_of(u.index + 1, FPS).min(audio_end) {
            ring.push_pcm(pushed_to, &tone(pushed_to, BLOCK_FRAMES as u64));
            pushed_to += BLOCK_FRAMES as u64;
        }
        ring.push_video(u.clone()).expect("push");
    }
    while pushed_to < audio_end {
        ring.push_pcm(pushed_to, &tone(pushed_to, BLOCK_FRAMES as u64));
        pushed_to += BLOCK_FRAMES as u64;
    }
    let w = ring.window(FRAMES).expect("a window");
    assert_eq!(w.first_index, 30, "the latest keyframe leaving two seconds");
    let mut snap = ring.snapshot(&w);
    snap.size = SIZE;
    say(&format!(
        "window {w:?}; snapshot frames {:?}..{:?} ({}); ring {:?}",
        snap.video.first().map(|u| u.index),
        snap.video.last().map(|u| u.index),
        snap.video.len(),
        ring.stats()
    ));
    assert_eq!(snap.video.len(), 66);
    assert_eq!(
        snap.pcm.len() / 2,
        66 * 1600,
        "the audio spans the video exactly"
    );

    let path = temp_mp4("replay");
    let passthrough =
        write_clip(&session, &path, &snap, &AtomicBool::new(false)).expect("write the clip");
    let bytes = std::fs::read(&path).expect("the clip exists");
    let _ = std::fs::remove_file(&path);
    assert!(passthrough, "the video went in as it was");
    let c = check(&bytes, &header, aac, 66 * 1600);
    assert_eq!(c.samples, 66);
    assert_eq!(c.stss, Some(vec![1, 16, 46]), "keyframes 30, 45, 75");
    say(&format!(
        "clip: {} bytes, 66 frames from keyframe 30{}",
        bytes.len(),
        if aac {
            ", with AAC"
        } else {
            "; no AAC encoder here"
        }
    ));
}

/// Quit cancelling a save: the writer lets go of the file at once, and the
/// file can be removed — no `.part` left behind.
#[test]
fn a_cancelled_clip_leaves_no_file() {
    let Some((_com, session, _)) = setup() else {
        return;
    };
    let (units, header) = encode(&session);
    let mut ring = ReplayRing::new(RingConfig {
        fps: FPS,
        seconds: 2,
        cap_bytes: u64::MAX / 4,
    });
    ring.start_segment(Some(header), 0);
    for u in &units {
        ring.push_video(u.clone()).expect("push");
    }
    let w = ring.window(FRAMES).expect("a window");
    let mut snap = ring.snapshot(&w);
    snap.size = SIZE;
    let path = temp_mp4("cancelled");
    let r = write_clip(&session, &path, &snap, &AtomicBool::new(true));
    assert!(r.is_err(), "a cancelled save is an error");
    assert!(path.exists(), "the writer had opened the file");
    std::fs::remove_file(&path).expect("the abandoned file can be removed at once");
}

#[test]
fn a_muxer_fed_mid_gop_starts_at_the_first_keyframe() {
    let Some((_com, session, _)) = setup() else {
        return;
    };
    let (units, header) = encode(&session);
    // A muxer fed from frame 5 writes nothing before the keyframe at 30.
    let path = temp_mp4("midgop");
    let mut m = Mp4Writer::create(
        &session,
        &path,
        &VideoFormat {
            size: SIZE,
            fps: FPS,
            seq_header: header.clone(),
        },
        false,
    )
    .expect("open");
    for u in &units[5..] {
        let wrote = m.write_video(u.index, u.key, &u.data).expect("write");
        assert_eq!(wrote, u.index >= 30, "frame {}", u.index);
    }
    m.finish().expect("finish");
    let bytes = std::fs::read(&path).expect("the file exists");
    let _ = std::fs::remove_file(&path);
    let c = check(&bytes, &header, false, 0);
    assert_eq!(c.stss.as_ref().map(|s| s[0]), Some(1));
    assert_eq!(c.samples, 66);
}

#[test]
fn a_take_cut_at_a_keyframe_starts_its_audio_at_zero() {
    let Some((_com, session, aac)) = setup() else {
        return;
    };
    if !aac {
        say("skipped the take case: no AAC encoder here");
        return;
    }
    let (units, header) = encode(&session);
    // A take cut from the shared audio at frame 30 starts at zero.
    let path = temp_mp4("take");
    let mut m = Mp4Writer::create(
        &session,
        &path,
        &VideoFormat {
            size: SIZE,
            fps: FPS,
            seq_header: header.clone(),
        },
        true,
    )
    .expect("open");
    let mut cut = TakeAudioCut::new(sample_of(30, FPS), &BlockTail::default());
    let mut next = 0u64;
    for u in &units[30..] {
        while next < sample_of(u.index + 1, FPS) {
            let block = AudioBlock {
                pts: next * 10_000_000 / u64::from(OUT_RATE),
                duration: BLOCK_FRAMES as u64 * 10_000_000 / u64::from(OUT_RATE),
                samples: tone(next, BLOCK_FRAMES as u64),
            };
            for b in cut.audio(&block) {
                m.write_audio(&b).expect("audio");
            }
            next += BLOCK_FRAMES as u64;
        }
        m.write_video(u.index, u.key, &u.data).expect("video");
    }
    let (rest, _) = cut.finish(66 * 1600);
    for b in rest {
        m.write_audio(&b).expect("audio");
    }
    m.finish().expect("finish");
    let bytes = std::fs::read(&path).expect("the take exists");
    let _ = std::fs::remove_file(&path);
    let c = check(&bytes, &header, true, 66 * 1600);
    assert_eq!(c.stss, Some(vec![1, 16, 46]));
}
