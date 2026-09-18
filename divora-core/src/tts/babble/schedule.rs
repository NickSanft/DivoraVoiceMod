//! Syllables → timed, pitched events.
//!
//! Timing follows the written text: every character gets one equal slot, and
//! each unit rings on past its own slots so neighbours overlap. That overlap
//! is how the patter gets fast *without* the pitch rising with it — speeding
//! up recordings is what turns critter speech into a chipmunk.
//!
//! Everything here is deterministic. Per-unit pitch wander comes from a PRNG
//! seeded by a hash of the text, so the same line always babbles the same
//! way (and tests can assert exact events).

use super::phonics::{syllabify, Item, Unit};
use super::text::{tokenize, Pause, Token};
use crate::tts::TTS_SAMPLE_RATE;

/// Hard cap on voiced units per utterance. At typical rates this is well over
/// a minute of babble, and it bounds memory for pasted walls of text.
pub const MAX_UNITS: usize = 1_500;

/// How one babble voice sounds. Every field is a plain number so a variant is
/// just a named constant.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VoiceParams {
    /// Characters voiced per second. Each written character gets one slot.
    pub rate: f32,
    /// Pitch ratio applied to every unit (1.0 = the unit source's own pitch).
    pub pitch: f32,
    /// Random per-unit pitch wander, ± this many cents.
    pub jitter_cents: f32,
    /// Extra slots each unit rings past its own characters. 1.0 gives the
    /// ~50 % overlap between neighbours.
    pub tail_slots: f32,
}

/// One unit placed in time.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Event {
    pub unit: Unit,
    /// First sample, at [`TTS_SAMPLE_RATE`].
    pub start: usize,
    /// Length in samples, including the overlapping tail.
    pub len: usize,
    /// Playback ratio for this unit (base pitch × wander × intonation).
    pub pitch: f32,
    /// Linear gain for this unit.
    pub gain: f32,
}

/// A whole utterance.
#[derive(Debug, Clone, PartialEq)]
pub struct Schedule {
    pub events: Vec<Event>,
    /// Total length in samples (end of the last event's tail).
    pub len: usize,
    /// True when the text was longer than [`MAX_UNITS`] and got cut short.
    pub truncated: bool,
}

/// Slots of silence for each gap.
const fn gap_slots(t: Token) -> f64 {
    match t {
        Token::Pause(Pause::Comma) => 3.0,
        Token::Pause(Pause::Sentence) => 5.0,
        Token::Pause(Pause::Ellipsis) => 7.0,
        // Word gaps, and marks that carry no time of their own.
        _ => 1.0,
    }
}

/// `?` lifts the last sound (and a little of the one before it).
const QUESTION_LIFT: [f32; 2] = [1.2, 1.08];
/// `!` pushes the last sound louder (and nudges it up).
const EXCLAIM_GAIN: [f32; 2] = [1.25, 1.1];
const EXCLAIM_LIFT: f32 = 1.05;
/// Consonant clusters voiced on the neutral vowel sit back in the mix.
const BARE_GAIN: f32 = 0.7;
/// Pitch falls gently across a sentence, as speech does, down to this floor.
const DECLINATION_PER_UNIT: f32 = 0.004;
const DECLINATION_FLOOR: f32 = 0.92;

/// Lay out `text` as babble events for `params`. `salt` varies the pitch
/// wander between voices speaking the same line.
#[must_use]
pub fn schedule(text: &str, params: &VoiceParams, salt: u64) -> Schedule {
    let items = syllabify(&tokenize(text));
    let mut rng = XorShift::new(fnv1a(text.as_bytes()) ^ salt);

    let sr = f64::from(TTS_SAMPLE_RATE);
    let slot = sr / f64::from(params.rate.max(1.0));
    let tail = f64::from(params.tail_slots.max(0.0));

    let mut events: Vec<Event> = Vec::new();
    let mut cursor = 0.0_f64;
    let mut pending_gap = 0.0_f64;
    let mut sentence_units = 0_u32;
    let mut truncated = false;

    for item in items {
        match item {
            Item::Gap(Token::Question) => lift_last(&mut events, &QUESTION_LIFT),
            Item::Gap(Token::Exclaim) => {
                emphasise_last(&mut events);
            }
            Item::Gap(t) => {
                pending_gap = pending_gap.max(gap_slots(t));
                if matches!(t, Token::Pause(Pause::Sentence | Pause::Ellipsis)) {
                    sentence_units = 0;
                }
            }
            Item::Syllable(s) => {
                if events.len() >= MAX_UNITS {
                    truncated = true;
                    break;
                }
                // No silence before the first sound.
                if !events.is_empty() {
                    cursor += pending_gap * slot;
                }
                pending_gap = 0.0;

                let letters = f64::from(s.letters);
                let wander = cents_to_ratio(params.jitter_cents * rng.next_signed());
                #[allow(clippy::cast_precision_loss)]
                let fall =
                    (1.0 - DECLINATION_PER_UNIT * sentence_units as f32).max(DECLINATION_FLOOR);
                events.push(Event {
                    unit: s.unit,
                    start: to_samples(cursor),
                    len: to_samples((letters + tail) * slot).max(1),
                    pitch: params.pitch * wander * fall,
                    gain: if s.bare { BARE_GAIN } else { 1.0 },
                });
                cursor += letters * slot;
                sentence_units = sentence_units.saturating_add(1);
            }
        }
    }

    let len = events.iter().map(|e| e.start + e.len).max().unwrap_or(0);
    Schedule {
        events,
        len,
        truncated,
    }
}

fn lift_last(events: &mut [Event], lifts: &[f32]) {
    for (e, &k) in events.iter_mut().rev().zip(lifts) {
        e.pitch *= k;
    }
}

fn emphasise_last(events: &mut [Event]) {
    for (e, &g) in events.iter_mut().rev().zip(&EXCLAIM_GAIN) {
        e.gain *= g;
    }
    if let Some(last) = events.last_mut() {
        last.pitch *= EXCLAIM_LIFT;
    }
}

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn to_samples(x: f64) -> usize {
    x.max(0.0).round() as usize
}

fn cents_to_ratio(cents: f32) -> f32 {
    (cents / 1200.0).exp2()
}

/// FNV-1a, 64-bit — a stable seed from the text.
#[must_use]
pub fn fnv1a(bytes: &[u8]) -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325_u64;
    for &b in bytes {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    h
}

/// xorshift64* — tiny, deterministic, and good enough for pitch wander.
struct XorShift(u64);

impl XorShift {
    const fn new(seed: u64) -> Self {
        // A zero state would stay zero forever.
        Self(if seed == 0 {
            0x9E37_79B9_7F4A_7C15
        } else {
            seed
        })
    }

    /// Uniform in `[-1, 1)`.
    fn next_signed(&mut self) -> f32 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        let x = self.0.wrapping_mul(0x2545_F491_4F6C_DD1D);
        // Top 24 bits → [0, 1).
        #[allow(clippy::cast_precision_loss)]
        let unit = (x >> 40) as f32 / (1u64 << 24) as f32;
        unit.mul_add(2.0, -1.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FLAT: VoiceParams = VoiceParams {
        rate: 20.0, // 1200-sample slots at 24 kHz: easy arithmetic
        pitch: 1.0,
        jitter_cents: 0.0,
        tail_slots: 1.0,
    };

    #[test]
    fn one_slot_per_letter_with_an_overlapping_tail() {
        // "ba" is one syllable over two letters: 2 slots, plus a 1-slot tail.
        let s = schedule("ba", &FLAT, 0);
        assert_eq!(s.events.len(), 1);
        assert_eq!(s.events[0].start, 0);
        assert_eq!(s.events[0].len, 3 * 1200);
        assert_eq!(s.len, 3 * 1200);
    }

    #[test]
    fn neighbours_overlap_by_their_tail() {
        // "bab" → [ba (2 letters)] [b bare (1 letter)]. The second starts after
        // two slots, while the first is still ringing for one more.
        let s = schedule("bab", &FLAT, 0);
        assert_eq!(s.events[1].start, 2 * 1200);
        assert!(s.events[0].start + s.events[0].len > s.events[1].start);
    }

    #[test]
    fn word_gaps_and_pauses_add_silence_between_sounds() {
        let gap = schedule("ba ba", &FLAT, 0);
        assert_eq!(gap.events[1].start, 3 * 1200); // 2 letters + 1 gap slot

        let comma = schedule("ba, ba", &FLAT, 0);
        assert_eq!(comma.events[1].start, 5 * 1200); // 2 + 3

        let stop = schedule("ba. ba", &FLAT, 0);
        assert_eq!(stop.events[1].start, 7 * 1200); // 2 + 5
    }

    #[test]
    fn adjacent_pauses_merge_longest_wins() {
        // ",  —  ." between words is one sentence pause, not the sum.
        let s = schedule("ba ,\u{2014}. ba", &FLAT, 0);
        assert_eq!(s.events[1].start, 7 * 1200);
    }

    #[test]
    fn no_leading_silence_and_no_trailing_padding() {
        let s = schedule("  ...  ba!!  ", &FLAT, 0);
        assert_eq!(s.events[0].start, 0);
        assert_eq!(s.len, 3 * 1200);
    }

    #[test]
    fn question_lifts_the_end() {
        let flat = schedule("ba ba", &FLAT, 0);
        let q = schedule("ba ba?", &FLAT, 0);
        let last = q.events.len() - 1;
        assert!(q.events[last].pitch > flat.events[last].pitch * 1.15);
        assert!(q.events[0].pitch <= flat.events[0].pitch * 1.09);
    }

    #[test]
    fn exclamation_pushes_the_end_louder() {
        let flat = schedule("ba ba", &FLAT, 0);
        let bang = schedule("ba ba!", &FLAT, 0);
        let last = bang.events.len() - 1;
        assert!(bang.events[last].gain > flat.events[last].gain * 1.2);
    }

    #[test]
    fn pitch_falls_gently_through_a_sentence_and_resets() {
        let s = schedule("ba ba ba ba ba ba. ba", &FLAT, 0);
        assert!(s.events[5].pitch < s.events[0].pitch);
        assert!(s.events[5].pitch >= DECLINATION_FLOOR);
        // The new sentence starts high again.
        assert!((s.events[6].pitch - s.events[0].pitch).abs() < 1e-6);
    }

    #[test]
    fn deterministic_for_the_same_text_and_varied_across_texts() {
        let p = VoiceParams {
            jitter_cents: 60.0,
            ..FLAT
        };
        assert_eq!(
            schedule("hello there", &p, 7),
            schedule("hello there", &p, 7)
        );
        let a: Vec<f32> = schedule("hello there", &p, 7)
            .events
            .iter()
            .map(|e| e.pitch)
            .collect();
        let b: Vec<f32> = schedule("hello thera", &p, 7)
            .events
            .iter()
            .map(|e| e.pitch)
            .collect();
        assert_ne!(a, b);
        // The salt separates voices reading the same line.
        let c: Vec<f32> = schedule("hello there", &p, 8)
            .events
            .iter()
            .map(|e| e.pitch)
            .collect();
        assert_ne!(a, c);
    }

    #[test]
    fn wander_stays_inside_its_cents_bound() {
        let p = VoiceParams {
            jitter_cents: 50.0,
            ..FLAT
        };
        let s = schedule("the quick brown fox jumps over the lazy dog", &p, 1);
        let bound = cents_to_ratio(50.0) + 1e-4;
        for e in &s.events {
            // Declination can only pull pitch down, never past the floor.
            assert!(e.pitch <= bound, "{} above wander bound", e.pitch);
            assert!(
                e.pitch >= DECLINATION_FLOOR / bound,
                "{} below bound",
                e.pitch
            );
        }
    }

    #[test]
    fn caps_runaway_text_and_says_so() {
        let wall = "ba ".repeat(MAX_UNITS + 50);
        let s = schedule(&wall, &FLAT, 0);
        assert_eq!(s.events.len(), MAX_UNITS);
        assert!(s.truncated);
        assert!(!schedule("ba ba", &FLAT, 0).truncated);
    }

    #[test]
    fn punctuation_only_text_has_no_events() {
        let s = schedule(" ?! ... \u{1F642} ", &FLAT, 0);
        assert!(s.events.is_empty());
        assert_eq!(s.len, 0);
    }

    #[test]
    fn rate_is_honoured() {
        let fast = VoiceParams { rate: 30.0, ..FLAT };
        let s = schedule("ba ba", &fast, 0);
        assert_eq!(s.events[1].start, 3 * 800); // 24_000 / 30 = 800
    }
}
