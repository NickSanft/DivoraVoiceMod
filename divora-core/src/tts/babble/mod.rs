//! Babble — procedural "critter speech" for the Speak screen, shipped as the
//! **Critter Chatter** voices.
//!
//! Small animated characters in many games "talk" by voicing one short
//! syllable per written character, blurred together at speed. This engine
//! renders that style from any text, locally, with no model to download:
//!
//! 1. [`text`] — text → letters, digits, pauses, and `?`/`!` marks.
//! 2. [`phonics`] — spelling → syllable units, one per written character.
//! 3. [`schedule`] — units → timed, pitched events (deterministic).
//! 4. [`render`] — events + a [`render::UnitSource`] → 24 kHz audio.
//!
//! Syllables come from [`formant`], a source-filter synthesiser: no audio
//! assets exist, so these voices are available on every install.

pub mod formant;
pub mod phonics;
pub mod render;
pub mod schedule;
pub mod text;

use std::sync::OnceLock;

use super::{TtsAudio, TtsError, TTS_SAMPLE_RATE};
use formant::{FormantBank, Timbre};
use schedule::{schedule, VoiceParams};

/// Every babble voice id starts with this. The colon matters: it fails the
/// cloned-voice id check, so a babble id can never resolve to (or collide
/// with) a cloned voice's folder, and previews are never disk-cached.
pub const ID_PREFIX: &str = "babble:";

/// A selectable babble voice.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BabbleVoice {
    /// Stable id, persisted in settings and saved clips — never rename.
    pub id: &'static str,
    /// Display name, shown under the "Critter Chatter" heading.
    pub name: &'static str,
    /// Timing and intonation for the scheduler.
    pub params: VoiceParams,
    /// Separates voices reading the same line.
    pub salt: u64,
    /// Pitch, vocal-tract size and pace of the voice's syllables.
    pub timbre: Timbre,
}

/// The shipped voices, in display order.
///
/// Each voice's pitch and tract scale are the ratios it was first voiced
/// with, when voices were one bank resampled (Bright ×1.30, Gruff ×0.72), so
/// they keep the timbre they were chosen by. The scheduler no longer shifts
/// pitch; tempo is set so each voice's syllables fit its beat.
pub const VOICES: &[BabbleVoice] = &[
    BabbleVoice {
        id: "babble:bright",
        name: "Bright",
        params: VoiceParams {
            rate: 20.0,
            pitch: 1.0,
            jitter_cents: 45.0,
            tail_slots: 1.0,
        },
        salt: 1,
        timbre: Timbre {
            f0: 224.0 * 1.30,
            tract: 1.30,
            tempo: 0.76,
        },
    },
    BabbleVoice {
        id: "babble:mellow",
        name: "Mellow",
        params: VoiceParams {
            rate: 18.0,
            pitch: 1.0,
            jitter_cents: 35.0,
            tail_slots: 1.0,
        },
        salt: 2,
        timbre: Timbre {
            f0: 224.0,
            tract: 1.0,
            tempo: 0.88,
        },
    },
    BabbleVoice {
        id: "babble:gruff",
        name: "Gruff",
        params: VoiceParams {
            rate: 15.0,
            pitch: 1.0,
            jitter_cents: 25.0,
            tail_slots: 1.0,
        },
        salt: 3,
        timbre: Timbre {
            f0: 224.0 * 0.72,
            tract: 0.72,
            tempo: 1.10,
        },
    },
];

/// One syllable bank per shipped voice, built the first time it speaks.
static BANKS: [OnceLock<FormantBank>; VOICES.len()] = [const { OnceLock::new() }; VOICES.len()];

/// Whether `id` names a babble voice (known or not).
#[must_use]
pub fn is_babble_id(id: &str) -> bool {
    id.starts_with(ID_PREFIX)
}

/// The voice for `id`, if it is one we ship.
#[must_use]
pub fn voice(id: &str) -> Option<&'static BabbleVoice> {
    VOICES.iter().find(|v| v.id == id)
}

/// The shared bank for `VOICES[index]`.
fn bank(index: usize) -> &'static FormantBank {
    BANKS[index].get_or_init(|| FormantBank::with_timbre(VOICES[index].timbre))
}

/// Render `text` in babble voice `voice_id` as 24 kHz mono audio.
///
/// The voice's syllable bank is built on its first use (tens of
/// milliseconds) and shared by every later call.
///
/// # Errors
/// [`TtsError::UnknownVoice`] for an id we don't ship;
/// [`TtsError::EmptyText`] when the text has nothing to voice.
pub fn synthesize(text: &str, voice_id: &str) -> Result<TtsAudio, TtsError> {
    let index = VOICES
        .iter()
        .position(|v| v.id == voice_id)
        .ok_or_else(|| TtsError::UnknownVoice(voice_id.to_string()))?;
    let voice = &VOICES[index];
    let plan = schedule(text, &voice.params, voice.salt);
    if plan.events.is_empty() {
        return Err(TtsError::EmptyText);
    }
    let samples = render::render(&plan, bank(index));
    Ok(TtsAudio {
        samples,
        sample_rate: TTS_SAMPLE_RATE,
    })
}

#[cfg(test)]
mod tests {
    use super::render::{place, UnitSource};
    use super::*;

    /// The lines the voices were auditioned on.
    const LINES: [&str; 3] = [
        "Hi there \u{2014} how does this sound to you? Give me a line and I'll read it back.",
        "Oh! Welcome to the island. Did you catch any fish today?",
        "Hmm... I'm not sure about that. Let's talk tomorrow, okay?",
    ];

    fn sr() -> f64 {
        f64::from(TTS_SAMPLE_RATE)
    }

    #[allow(clippy::cast_precision_loss)]
    const fn to_f64(n: usize) -> f64 {
        n as f64
    }

    fn median(mut x: Vec<f64>) -> f64 {
        x.sort_by(f64::total_cmp);
        x[x.len() / 2]
    }

    /// Per event of `voice` reading `text` with `source`: the share of the
    /// unit's energy (read at the event's rate) that survives the event's
    /// length and fades.
    fn surviving(voice: &BabbleVoice, text: &str, source: &dyn UnitSource) -> Vec<f64> {
        let (placements, _) = place(&schedule(text, &voice.params, voice.salt), source);
        placements
            .iter()
            .map(|p| {
                let src = source.unit(p.unit);
                let energy = |x: f32| f64::from(x) * f64::from(x);
                let kept: f64 = (0..p.len).map(|n| energy(p.sample(src, n))).sum();
                // Every output sample that reads inside the unit.
                #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                let natural = ((to_f64(src.len() - 1) - p.phase) / p.step) as usize + 1;
                let whole: f64 = (0..natural).map(|n| energy(p.read(src, n))).sum();
                kept / whole
            })
            .collect()
    }

    /// Where a lone event becomes loud and voiced, in output samples from its
    /// first sample: the first 1 ms step at which a centred 10 ms window
    /// reaches half the vowel level (the bank levels vowels to 0.1 RMS) and a
    /// centred 20 ms window is periodic at the voice's pitch. Independent of
    /// the bank's own `lead`, which is measured on the voiced branch alone.
    fn heard_onset(x: &[f32], f0: f64, step: f64) -> Option<usize> {
        let rms = |w: &[f32]| {
            (w.iter().map(|&s| f64::from(s) * f64::from(s)).sum::<f64>() / to_f64(w.len())).sqrt()
        };
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let lags = {
            let period = sr() / f0 / step;
            (0.9 * period) as usize..=(1.25 * period) as usize
        };
        let dot = |p: &[f32], q: &[f32]| -> f64 {
            p.iter()
                .zip(q)
                .map(|(&u, &v)| f64::from(u) * f64::from(v))
                .sum()
        };
        let periodic = |i: usize| {
            let a = &x[i - 240..i + 240];
            lags.clone()
                .map(|lag| {
                    let b = &x[i - 240 + lag..i + 240 + lag];
                    dot(a, b) / (dot(a, a) * dot(b, b)).sqrt().max(1e-30)
                })
                .fold(f64::MIN, f64::max)
        };
        (240..x.len().saturating_sub(240 + lags.end()))
            .step_by(24)
            .find(|&i| rms(&x[i - 120..i + 120]) >= 0.05 && periodic(i) >= 0.6)
    }

    /// Per event: how far (ms) its heard onset lands from its beat, as placed,
    /// and how far it would if the unit started on the beat instead.
    fn onset_errors(voice: &BabbleVoice, text: &str, bank: &FormantBank) -> Vec<(f64, f64)> {
        const PAD: usize = 480;
        let (placements, _) = place(&schedule(text, &voice.params, voice.salt), bank);
        placements
            .iter()
            .filter_map(|p| {
                let src = bank.unit(p.unit);
                // Silence in front, so an onset in the first 10 ms is found too.
                let mut alone = vec![0.0; PAD];
                alone.extend((0..p.len).map(|n| p.sample(src, n)));
                let heard = to_f64(heard_onset(&alone, voice.timbre.f0, p.step)?) - to_f64(PAD);
                let ms = |samples: f64| samples * 1e3 / sr();
                Some((ms(heard - to_f64(p.beat - p.start)), ms(heard)))
            })
            .collect()
    }

    #[test]
    fn every_voice_fits_its_syllables_to_its_beats() {
        for (i, v) in VOICES.iter().enumerate() {
            let kept: Vec<f64> = LINES
                .iter()
                .flat_map(|t| surviving(v, t, bank(i)))
                .collect();
            let least = kept.iter().copied().fold(f64::MAX, f64::min);
            let typical = median(kept);
            assert!(typical >= 0.95, "{}: median {typical:.3} survives", v.name);
            assert!(
                least >= 0.85,
                "{}: as little as {least:.3} survives",
                v.name
            );
        }
    }

    #[test]
    fn every_voice_lands_voicing_on_the_beat() {
        for (i, v) in VOICES.iter().enumerate() {
            let errors: Vec<(f64, f64)> = LINES
                .iter()
                .flat_map(|t| onset_errors(v, t, bank(i)))
                .collect();
            let events: usize = LINES
                .iter()
                .map(|t| schedule(t, &v.params, v.salt).events.len())
                .sum();
            assert_eq!(errors.len(), events, "{}: an onset went unheard", v.name);
            let placed: Vec<f64> = errors.iter().map(|e| e.0.abs()).collect();
            let worst = placed.iter().copied().fold(0.0, f64::max);
            let typical = median(placed);
            // Heard onsets use 1 ms steps and a 10 ms window; the renderer's
            // leads are measured on the voiced branch alone. Unplaced, these
            // same units would land 5–80 ms after their beats.
            assert!(
                typical <= 2.0,
                "{}: median {typical:.1} ms off the beat",
                v.name
            );
            assert!(worst <= 8.0, "{}: up to {worst:.1} ms off the beat", v.name);
        }
    }

    #[test]
    fn every_voice_is_as_loud_as_the_kokoro_voices() {
        for v in VOICES {
            for text in LINES {
                let audio = synthesize(text, v.id).expect("renders");
                let loudness = render::loudness_dbfs(&audio.samples).expect("sounds");
                assert!(
                    (loudness - render::TARGET_DBFS).abs() <= 0.25,
                    "{} at {loudness:.2} dBFS on {text:?}",
                    v.name
                );
                assert!(audio
                    .samples
                    .iter()
                    .all(|x| x.abs() <= render::PEAK_CEILING));
            }
        }
    }

    #[test]
    fn each_voice_builds_its_bank_once_in_its_own_timbre() {
        for (i, v) in VOICES.iter().enumerate() {
            synthesize("once", v.id).expect("renders");
            let first: *const FormantBank = bank(i);
            synthesize("twice", v.id).expect("renders");
            assert!(std::ptr::eq(first, bank(i)), "{} rebuilt its bank", v.name);
            assert_eq!(bank(i).timbre(), v.timbre);
            // No voice is resampled into shape any more.
            assert!((v.params.pitch - 1.0).abs() < f32::EPSILON, "{}", v.name);
        }
    }

    #[test]
    #[ignore = "prints measurements: cargo test --release -p divora-core --lib babble::tests::report -- --ignored --nocapture"]
    fn report() {
        for (i, v) in VOICES.iter().enumerate() {
            let kept: Vec<f64> = LINES
                .iter()
                .flat_map(|t| surviving(v, t, bank(i)))
                .collect();
            let errors: Vec<(f64, f64)> = LINES
                .iter()
                .flat_map(|t| onset_errors(v, t, bank(i)))
                .collect();
            let placed: Vec<f64> = errors.iter().map(|e| e.0.abs()).collect();
            let unplaced: Vec<f64> = errors.iter().map(|e| e.1).collect();
            println!(
                "{}: {} events; energy surviving median {:.3} min {:.3}; \
                 onset off the beat median {:.2} max {:.2} ms (unplaced {:.1}..{:.1} ms)",
                v.name,
                kept.len(),
                median(kept.clone()),
                kept.iter().copied().fold(f64::MAX, f64::min),
                median(placed.clone()),
                placed.iter().copied().fold(0.0, f64::max),
                unplaced.iter().copied().fold(f64::MAX, f64::min),
                unplaced.iter().copied().fold(0.0, f64::max),
            );
        }
    }

    #[test]
    fn ids_are_namespaced_unique_and_resolvable() {
        let mut seen = std::collections::HashSet::new();
        for v in VOICES {
            assert!(is_babble_id(v.id), "{} lacks the prefix", v.id);
            assert!(seen.insert(v.id), "duplicate id {}", v.id);
            assert_eq!(voice(v.id), Some(v));
        }
        assert!(!is_babble_id("af_heart"));
        assert_eq!(voice("babble:nope"), None);
    }

    #[test]
    fn synthesizes_every_voice() {
        for v in VOICES {
            let audio = synthesize("Hi there!", v.id).expect("renders");
            assert_eq!(audio.sample_rate, TTS_SAMPLE_RATE);
            assert!(!audio.samples.is_empty());
            assert!(audio.samples.iter().all(|x| x.is_finite()));
        }
    }

    #[test]
    fn unknown_voice_and_empty_text_are_errors() {
        assert!(matches!(
            synthesize("hello", "babble:nope"),
            Err(TtsError::UnknownVoice(_))
        ));
        assert!(matches!(
            synthesize(" ?! \u{1F642} ", "babble:bright"),
            Err(TtsError::EmptyText)
        ));
    }
}
