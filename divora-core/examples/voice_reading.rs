//! What the voice-reading analyzer says about a signal, and what it costs.
//!
//! ```text
//! cargo run --release -p divora-core --example voice_reading
//! ```
//!
//! No mic and no assets: every signal here is a formula, so the numbers below
//! are reproducible on any machine. Prints, for each scene, the state, both
//! taps' metrics and the phrase; then the CPU per second of audio for the two
//! taps at 44.1 / 48 / 96 kHz, and the analyzer's memory.
//!
//! Remember what the phrase is and is not. Every word describes the *signal* —
//! how loud, how high, how much it moves, how fast, how bright. None of them
//! is a claim about the person making it.

#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]

use std::f32::consts::{PI, TAU};
use std::time::Instant;

use divora_core::dsp::{Analyzer, InputFacts, Metrics, VoiceReading};

/// A synthetic talker. Harmonic stack, gated into bursts, optional vibrato.
#[derive(Clone)]
struct Voice {
    rate: f32,
    f0: f32,
    harmonics: Vec<f32>,
    amp: f32,
    vibrato_st: f32,
    vibrato_hz: f32,
    burst_s: f32,
    gap_s: f32,
}

impl Voice {
    fn talking(rate: f32) -> Self {
        Self {
            rate,
            f0: 140.0,
            harmonics: vec![1.0, 0.6, 0.4, 0.25, 0.15, 0.1],
            amp: 0.1,
            vibrato_st: 0.0,
            vibrato_hz: 0.0,
            burst_s: 0.28,
            gap_s: 0.12,
        }
    }

    fn render(&self, secs: f32) -> Vec<f32> {
        let n = (secs * self.rate) as usize;
        let norm = self
            .harmonics
            .iter()
            .map(|a| a * a)
            .sum::<f32>()
            .sqrt()
            .max(f32::MIN_POSITIVE);
        let period = self.burst_s + self.gap_s;
        let edge = 0.02f32.min(self.burst_s * 0.4);
        let mut phase = 0.0f32;
        let mut out = Vec::with_capacity(n);
        for i in 0..n {
            let t = i as f32 / self.rate;
            let hz =
                self.f0 * ((self.vibrato_st * (TAU * self.vibrato_hz * t).sin()) / 12.0).exp2();
            phase += TAU * hz / self.rate;
            if phase > TAU {
                phase -= TAU;
            }
            let pos = t % period;
            let env = if pos >= self.burst_s {
                0.0
            } else if pos < edge {
                0.5 - 0.5 * (PI * pos / edge).cos()
            } else if pos > self.burst_s - edge {
                0.5 - 0.5 * (PI * (self.burst_s - pos) / edge).cos()
            } else {
                1.0
            };
            let mut s = 0.0f32;
            for (k, a) in self.harmonics.iter().enumerate() {
                s = a.mul_add((((k + 1) as f32) * phase).sin(), s);
            }
            out.push(self.amp * env * s * std::f32::consts::SQRT_2 / norm);
        }
        out
    }
}

fn noise(rate: f32, secs: f32, amp: f32) -> Vec<f32> {
    let n = (secs * rate) as usize;
    let mut state = 0x853c_49e6_748f_ea9bu64;
    (0..n)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            amp * (((state >> 40) as f32) / 8_388_608.0 - 1.0)
        })
        .collect()
}

fn scaled(signal: &[f32], db: f32) -> Vec<f32> {
    let g = 10f32.powf(db / 20.0);
    signal.iter().map(|s| s * g).collect()
}

/// Samples per block, the size a device callback delivers.
const BLOCK: usize = 480;

/// Feed both taps the way the worker would.
fn feed(a: &mut Analyzer, dry: &[f32], wet: &[f32], facts: InputFacts) -> Vec<VoiceReading> {
    let n = dry.len().min(wet.len());
    let mut out = Vec::new();
    let mut start = 0;
    while start < n {
        let end = (start + BLOCK).min(n);
        if let Some(r) = a.observe(&dry[start..end], &wet[start..end], facts) {
            out.push(r);
        }
        start = end;
    }
    out
}

fn show_metrics(label: &str, m: &Metrics) {
    let level = if m.energy_dbfs.is_finite() {
        format!("{:7.1}", m.energy_dbfs)
    } else {
        "      -".into()
    };
    println!(
        "    {label:<6} level {level} dBFS  span {:5.1} dB   f0 {:6.1} Hz  spread {:5.2} st   \
         voiced {:4.2}   pace {:4.2} /s   centroid {:7.0} Hz",
        m.energy_range_db, m.f0_hz, m.f0_range_st, m.voiced_ratio, m.pace_ops, m.brightness_hz
    );
}

fn show(scene: &str, r: &VoiceReading) {
    let words: Vec<&str> = if r.descriptors.is_empty() {
        r.held_descriptors.iter().map(|d| d.word()).collect()
    } else {
        r.descriptors.iter().map(|d| d.word()).collect()
    };
    let phrase = if words.is_empty() {
        "(still listening)".to_string()
    } else {
        words.join(", ")
    };
    let flags = [
        (r.held, "held"),
        (!r.calibrated, "uncalibrated"),
        (r.wet_bypassed, "chain bypassed"),
        (!r.wet_settled, "wet still filling"),
    ]
    .iter()
    .filter(|(on, _)| *on)
    .map(|(_, n)| *n)
    .collect::<Vec<_>>()
    .join(", ");
    println!("\n  {scene}");
    println!(
        "    state {:?}{}",
        r.state,
        if flags.is_empty() {
            String::new()
        } else {
            format!("   [{flags}]")
        }
    );
    show_metrics("dry", &r.dry);
    show_metrics("wet", &r.wet);
    println!("    signal reads: {phrase}");
}

/// Long enough to calibrate the baseline (12 observations, one per second of
/// speech) with a window's margin.
const CALIBRATE_S: f32 = 16.0;

fn scenes() {
    let rate = 48_000.0;
    let voice = Voice::talking(rate);
    println!("\n=== What it says ===");

    // A character preset: +7 semitones and brighter, which is what the wet
    // tap is for.
    let mut character = voice.clone();
    character.f0 = voice.f0 * 2f32.powf(7.0 / 12.0);
    character.harmonics = vec![0.3, 0.6, 1.0, 0.9, 0.8, 0.6, 0.5];

    let mut a = Analyzer::new(48_000);
    let dry = voice.render(CALIBRATE_S);
    let wet = character.render(CALIBRATE_S);
    let readings = feed(&mut a, &dry, &wet, InputFacts::default());
    show(
        "Talking, after the baseline has settled",
        readings.last().expect("no reading"),
    );

    let loud = scaled(&voice.render(4.0), 9.0);
    let loud_wet = scaled(&character.render(4.0), 9.0);
    let readings = feed(&mut a, &loud, &loud_wet, InputFacts::default());
    show(
        "The same talker, 9 dB up",
        readings.last().expect("no reading"),
    );

    // Room tone: nothing decays, because most of a session is not speech.
    let room = noise(rate, 6.0, 0.0003);
    let readings = feed(&mut a, &room, &room, InputFacts::default());
    show(
        "A pause (the numbers are the last speaking window, frozen)",
        readings.last().expect("no reading"),
    );

    // Push-to-modulate, key up: the chain is out of the path.
    let plain = voice.render(4.0);
    let facts = InputFacts {
        chain_bypassed: true,
        ..InputFacts::default()
    };
    let readings = feed(&mut a, &plain, &plain, facts);
    show(
        "Push-to-modulate released (wet is the dry signal, not a broken preset)",
        readings.last().expect("no reading"),
    );

    // A preset switch moves the wet tap instantly and nothing else.
    a.note_chain_changed();
    let readings = feed(
        &mut a,
        &voice.render(0.75),
        &character.render(0.75),
        InputFacts::default(),
    );
    show(
        "0.75 s after switching to a +7 st preset",
        readings.last().expect("no reading"),
    );

    // Digital silence is a mute, not a quiet room, and neither is a stopped
    // engine.
    let zeros = vec![0.0f32; 48_000 * 3];
    let readings = feed(&mut a, &zeros, &zeros, InputFacts::default());
    show("Muted at the device", readings.last().expect("no reading"));
    let stopped = a
        .observe(
            &[],
            &[],
            InputFacts {
                engine_running: false,
                ..InputFacts::default()
            },
        )
        .expect("no reading");
    show(
        "Engine stopped (a fact, never inferred from silence)",
        &stopped,
    );
}

fn cost() {
    println!("\n=== What it costs ===");
    println!("  Both taps, dry and wet, full pipeline. Measure with --release.");
    for &rate in &[44_100u32, 48_000, 96_000] {
        let voice = Voice::talking(rate as f32);
        let secs = 20.0f32;
        let dry = voice.render(secs);
        let wet = voice.render(secs);
        let mut a = Analyzer::new(rate);
        // One untimed pass, so the first-touch page faults are not in the
        // number.
        feed(
            &mut a,
            &dry[..rate as usize],
            &wet[..rate as usize],
            InputFacts::default(),
        );
        let mut a = Analyzer::new(rate);
        let start = Instant::now();
        let readings = feed(&mut a, &dry, &wet, InputFacts::default());
        let elapsed = start.elapsed();
        let per_second = elapsed.as_secs_f64() / f64::from(secs);
        println!(
            "  {rate:>6} Hz   {:>7.2} ms per second of audio   ({:.3} % of one core)   \
             {} readings   {} kB",
            per_second * 1000.0,
            per_second * 100.0,
            readings.len(),
            a.heap_bytes() / 1024
        );
    }
    println!(
        "\n  Audio-thread cost is not in the above: the callback only pushes two\n  \
         slices into preallocated rings. Ring memory is one second per tap\n  \
         ({} kB for the pair at 48 kHz).",
        48_000 * 4 * 2 / 1024
    );
}

fn main() {
    println!("Voice reading — measured acoustic properties of a signal.");
    println!("Not emotion recognition: every word below describes a sound.");
    scenes();
    cost();
}
