//! Renders of every shipped babble voice, with the numbers that matter.
//!
//! ```text
//! cargo run --release -p divora-core --example babble_voices -- <out-dir>
//! ```
//!
//! Writes `{voice}-{line}.wav` for every voice × line through
//! [`babble::synthesize`], exactly as the Speak screen hears it, plus
//! `units-{voice}.wav`: a fixed list of units from that voice's bank played
//! raw (no scheduler, no pitch change) with 150 ms of silence between them.
//! All files are 24 kHz mono 16-bit PCM.
//!
//! Prints, per render: duration, peak, clipped samples, gated loudness and
//! the share of energy above 9 kHz; per voice: bank build time and size, and
//! `synthesize()` time for the preview line, cold and warm. Energy surviving
//! the renderer and onset placement come from the crate's tests:
//! `cargo test --release -p divora-core --lib babble::tests::report -- --ignored --nocapture`.

use std::error::Error;
use std::path::{Path, PathBuf};
use std::time::Instant;

use divora_core::tts::babble::formant::FormantBank;
use divora_core::tts::babble::phonics::{Nucleus, Onset, Unit};
use divora_core::tts::babble::render::{loudness_dbfs, UnitSource};
use divora_core::tts::babble::{self, VOICES};
use divora_core::tts::TTS_SAMPLE_RATE;
use realfft::RealFftPlanner;

const LINES: [(&str, &str); 3] = [
    (
        "preview",
        "Hi there \u{2014} how does this sound to you? Give me a line and I'll read it back.",
    ),
    (
        "island",
        "Oh! Welcome to the island. Did you catch any fish today?",
    ),
    (
        "unsure",
        "Hmm... I'm not sure about that. Let's talk tomorrow, okay?",
    ),
];

/// ba bee bye bow boo sha see tih kah muh lay reh woe you chih theh nuh puh a ee
const SHEET: [(Option<Onset>, Nucleus); 20] = [
    (Some(Onset::B), Nucleus::A),
    (Some(Onset::B), Nucleus::LongE),
    (Some(Onset::B), Nucleus::LongI),
    (Some(Onset::B), Nucleus::LongO),
    (Some(Onset::B), Nucleus::LongU),
    (Some(Onset::Sh), Nucleus::A),
    (Some(Onset::S), Nucleus::LongE),
    (Some(Onset::T), Nucleus::I),
    (Some(Onset::K), Nucleus::O),
    (Some(Onset::M), Nucleus::U),
    (Some(Onset::L), Nucleus::LongA),
    (Some(Onset::R), Nucleus::E),
    (Some(Onset::W), Nucleus::LongO),
    (Some(Onset::Y), Nucleus::LongU),
    (Some(Onset::Ch), Nucleus::I),
    (Some(Onset::Th), Nucleus::E),
    (Some(Onset::N), Nucleus::Schwa),
    (Some(Onset::P), Nucleus::Schwa),
    (None, Nucleus::A),
    (None, Nucleus::LongE),
];

const SHEET_GAP_SECS: f64 = 0.150;

fn main() -> Result<(), Box<dyn Error>> {
    let out = std::env::args_os()
        .nth(1)
        .map_or_else(|| PathBuf::from("babble-voices"), PathBuf::from);
    std::fs::create_dir_all(&out)?;

    for v in VOICES {
        let slug = v.name.to_lowercase();
        println!("== {} ({})  {:?}", v.name, v.id, v.timbre);

        // Cold: the first call builds this voice's bank.
        let t = Instant::now();
        babble::synthesize(LINES[0].1, v.id)?;
        let cold = millis(t);
        let mut warm = f64::INFINITY;
        for _ in 0..5 {
            let t = Instant::now();
            babble::synthesize(LINES[0].1, v.id)?;
            warm = warm.min(millis(t));
        }
        println!("synthesize(preview): cold {cold:.1} ms, warm {warm:.1} ms (best of 5)");

        let mut build = f64::INFINITY;
        let mut bank = None;
        for _ in 0..5 {
            let t = Instant::now();
            let b = FormantBank::with_timbre(v.timbre);
            build = build.min(millis(t));
            bank = Some(b);
        }
        let bank = bank.ok_or("no bank")?;
        let samples: usize = Unit::all().map(|u| bank.unit(u).len()).sum();
        #[allow(clippy::cast_precision_loss)]
        let megabytes = (samples * std::mem::size_of::<f32>()) as f64 / 1e6;
        println!(
            "bank: {build:.1} ms to build (best of 5), {samples} samples = {megabytes:.2} MB of audio"
        );

        for (line, text) in &LINES {
            let audio = babble::synthesize(text, v.id)?;
            let x = &audio.samples;
            let peak = x.iter().fold(0.0_f32, |m, s| m.max(s.abs()));
            let clipped = x.iter().filter(|s| s.abs() >= 1.0).count();
            let loudness = loudness_dbfs(x).ok_or("silent render")?;
            #[allow(clippy::cast_precision_loss)]
            let secs = x.len() as f64 / f64::from(TTS_SAMPLE_RATE);
            println!(
                "{line:>8}: {secs:.2} s, peak {peak:.3}, clipped {clipped}, gated {loudness:.2} dBFS, \
                 {:.4} % above 9 kHz",
                100.0 * share_above(x, 9_000.0)
            );
            write_wav(&out.join(format!("{slug}-{line}.wav")), x)?;
        }

        let gap = vec![0.0_f32; seconds_to_samples(SHEET_GAP_SECS)];
        let mut sheet = Vec::new();
        for (i, &(onset, nucleus)) in SHEET.iter().enumerate() {
            if i > 0 {
                sheet.extend_from_slice(&gap);
            }
            sheet.extend_from_slice(bank.unit(Unit::new(onset, nucleus)));
        }
        write_wav(&out.join(format!("units-{slug}.wav")), &sheet)?;
    }
    println!("wrote {}", out.display());
    Ok(())
}

fn millis(t: Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1e3
}

/// Share of `x`'s energy above `hz`, from its whole-signal power spectrum.
fn share_above(x: &[f32], hz: f64) -> f64 {
    let n = x.len().next_power_of_two();
    let fft = RealFftPlanner::<f64>::new().plan_fft_forward(n);
    let mut buf: Vec<f64> = x.iter().map(|&s| f64::from(s)).collect();
    buf.resize(n, 0.0);
    let mut spectrum = fft.make_output_vec();
    if fft.process(&mut buf, &mut spectrum).is_err() {
        return f64::NAN;
    }
    #[allow(clippy::cast_precision_loss)]
    let bin_hz = f64::from(TTS_SAMPLE_RATE) / n as f64;
    let (high, total) = spectrum
        .iter()
        .enumerate()
        .fold((0.0, 0.0), |(high, total), (k, c)| {
            let p = c.norm_sqr();
            #[allow(clippy::cast_precision_loss)]
            let above = k as f64 * bin_hz >= hz;
            (if above { high + p } else { high }, total + p)
        });
    high / total
}

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn seconds_to_samples(secs: f64) -> usize {
    (secs * f64::from(TTS_SAMPLE_RATE)).round() as usize
}

fn write_wav(path: &Path, samples: &[f32]) -> Result<(), Box<dyn Error>> {
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: TTS_SAMPLE_RATE,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut w = hound::WavWriter::create(path, spec)?;
    for &x in samples {
        #[allow(clippy::cast_possible_truncation)]
        let s = (x.clamp(-1.0, 1.0) * f32::from(i16::MAX)).round() as i16;
        w.write_sample(s)?;
    }
    w.finalize()?;
    Ok(())
}
