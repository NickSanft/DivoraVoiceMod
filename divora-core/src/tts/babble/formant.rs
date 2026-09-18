//! Syllables synthesised in code as a [`super::render::UnitSource`].
//!
//! Every unit comes from a small source-filter synthesiser in the style of
//! Klatt (1980): a glottal pulse train and noise sources drive second-order
//! digital resonators — a cascade for the vocal tract, parallel branches for
//! frication, bursts, voice bars and nasal murmur. There are no audio assets;
//! the whole inventory is rendered once when the bank is built.
//!
//! Each unit is first *planned* as piecewise-linear control curves (formant
//! frequencies, source amplitudes) laid out by consonant class, then rendered
//! sample by sample. Every source is calibrated against the unit's own vowel,
//! so "frication 9 dB under the vowel" means the same thing whatever the
//! filters happen to do to raw noise.
//!
//! A bank is built in one [`Timbre`]: glottal pitch, vocal-tract size and
//! pace, so each voice gets its own bank rather than a resampled one.
//!
//! Sources (equations and published measurements only):
//! - Klatt, D. H. (1980). Software for a cascade/parallel formant
//!   synthesizer. JASA 67(3). Resonator and anti-resonator equations.
//! - Klatt, D. H. & Klatt, L. C. (1990). Analysis, synthesis, and perception
//!   of voice quality variations among female and male talkers. JASA 87(2).
//!   The KLGLOTT88 glottal source, spectral tilt, breathiness.
//! - Hillenbrand, J., Getty, L. A., Clark, M. J. & Wheeler, K. (1995).
//!   Acoustic characteristics of American English vowels. JASA 97(5).
//! - Delattre, P. C., Liberman, A. M. & Cooper, F. S. (1955). Acoustic loci
//!   and transitional cues for consonants. JASA 27(4).
//! - Stevens, K. N. & Blumstein, S. E. (1978). Invariant cues for place of
//!   articulation in stop consonants. JASA 64(5).
//! - Lisker, L. & Abramson, A. S. (1964). A cross-language study of voicing
//!   in initial stops. Word 20(3).
//! - Fujimura, O. (1962). Analysis of nasal consonants. JASA 34(12).
//! - Jongman, A., Wayland, R. & Wong, S. (2000). Acoustic characteristics of
//!   English fricatives. JASA 108(3).
//! - Fant, G. (1960). Acoustic Theory of Speech Production.

use std::f64::consts::{LN_10, PI, TAU};
use std::ops::Range;

use super::phonics::{Nucleus, Onset, Unit};
use super::render::UnitSource;
use crate::tts::TTS_SAMPLE_RATE;

/// The voice a bank is built in: glottal pitch, vocal-tract size and pace.
///
/// Voices differ here rather than by resampling one bank. Resampling moves
/// pitch, formants and duration together, so a low voice also got long units
/// that its events then cut short; a timbre sets each on its own.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Timbre {
    /// Glottal pitch where each unit starts, in Hz. It falls to 7/8 of this
    /// by the unit's end.
    pub f0: f64,
    /// Scale on every frequency the filters use: formant targets and loci,
    /// the upper formants, all bandwidths, noise peaks and corners, nasal
    /// poles and zeros, the voice bar and the glottal tilt corner. 1.0 is an
    /// adult-female tract; a smaller tract resonates higher.
    pub tract: f64,
    /// Scale on every planned duration. Below 1.0 each unit is quicker.
    pub tempo: f64,
}

impl Timbre {
    /// The adult-female voice the unit plans are written for.
    pub const DEFAULT: Self = Self {
        f0: 224.0,
        tract: 1.0,
        tempo: 1.0,
    };
}

impl Default for Timbre {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// Every unit synthesised once, indexed by [`Unit::index`].
///
/// Building it takes a few tens of milliseconds on one core and holds about
/// 4 MB at the default timbre, so build it once per voice and share it.
pub struct FormantBank {
    units: Vec<Vec<f32>>,
    leads: Vec<usize>,
    timbre: Timbre,
}

impl FormantBank {
    /// Synthesise the whole inventory at [`Timbre::DEFAULT`].
    #[must_use]
    pub fn new() -> Self {
        Self::with_timbre(Timbre::DEFAULT)
    }

    /// Synthesise the whole inventory in `timbre`. Deterministic: the same
    /// samples on every run and every machine.
    #[must_use]
    pub fn with_timbre(timbre: Timbre) -> Self {
        Self::build(timbre, Some(NOISE_CUTOFF_HZ))
    }

    /// `noise_cutoff: None` skips the noise low-pass, reproducing the bank as
    /// it was before sources had to be band-limited (tests pin that).
    fn build(timbre: Timbre, noise_cutoff: Option<f64>) -> Self {
        let (units, leads) = Unit::all()
            .map(|u| synthesize(&plan(u, timbre.tempo), seed(u), timbre, noise_cutoff))
            .unzip();
        Self {
            units,
            leads,
            timbre,
        }
    }

    /// The timbre this bank was built in.
    #[must_use]
    pub const fn timbre(&self) -> Timbre {
        self.timbre
    }
}

impl Default for FormantBank {
    fn default() -> Self {
        Self::new()
    }
}

impl UnitSource for FormantBank {
    fn unit(&self, unit: Unit) -> &[f32] {
        self.units.get(unit.index()).map_or(&[], Vec::as_slice)
    }

    fn lead(&self, unit: Unit) -> usize {
        self.leads.get(unit.index()).copied().unwrap_or(0)
    }
}

#[allow(clippy::cast_lossless)] // `f64::from` isn't const
const SR: f64 = TTS_SAMPLE_RATE as f64;
/// Formants and envelopes are re-evaluated once a millisecond, well inside
/// the 5 ms frame Klatt (1980) updated at.
const BLOCK: usize = 24;

/// Each unit's vowel is levelled to this RMS.
const UNIT_RMS: f64 = 0.1;
/// Headroom kept under full scale when a unit's peaks would exceed it.
const PEAK_MAX: f64 = 0.95;

/// F0 falls gently across a unit, to this share of where it started (224 to
/// 196 Hz at the default timbre).
const F0_FALL: f64 = 0.875;
/// KLGLOTT88 open quotient; female voices run open and a little breathy
/// (Klatt & Klatt 1990).
const OPEN_QUOTIENT: f64 = 0.6;
/// Corner of the one-pole spectral-tilt low-pass on the glottal flow.
const TILT_HZ: f64 = 3_000.0;
/// Cycle-to-cycle period and amplitude perturbation, peak fractions.
const JITTER: f64 = 0.006;
const SHIMMER: f64 = 0.04;
/// Aspiration mixed into voicing, re the vowel.
const BREATH_DB: f64 = -26.0;

/// F4–F7 hold still: they shape the top of the spectrum, not the vowel.
/// Klatt's 10 kHz cascade stopped at F5; at 24 kHz every pole pair would
/// otherwise roll off together above it and leave the top octave dead, so
/// the higher resonances of a uniform female-length tract (~1.2 kHz apart)
/// are kept.
const UPPER: [(f64, f64); 4] = [
    (4_200.0, 300.0),
    (5_400.0, 400.0),
    (6_600.0, 500.0),
    (7_800.0, 600.0),
];
/// B1–B3 for vowels.
const VOWEL_BW: [f64; 3] = [90.0, 110.0, 170.0];
/// B1 while the glottis is open for aspiration: the trachea couples in and
/// damps F1 (Klatt 1980's "F1 cutback").
const OPEN_B1: f64 = 300.0;

/// Stops and affricates start with this much silence so the renderer's
/// fade-in never swallows the burst.
const LEAD: f64 = 0.004;
/// Bare consonants (Schwa units) run their consonant this much shorter.
const BARE_SCALE: f64 = 0.8;
/// Voice bar through a closure or under a voiced fricative, re the vowel.
const VOICE_BAR_DB: f64 = -20.0;
const VOICE_BAR_HZ: f64 = 220.0;
/// Where voicing settles before the final decay.
const DECLINE: f64 = 0.85;
/// Clears the DC and sub-audio rumble noise leaves after the cascade.
const DC_BLOCK_HZ: f64 = 60.0;
/// Noise sources are low-passed here. The renderer lifts pitch by up to
/// about ×1.35 and does not anti-alias, so anything above 12 kHz / 1.35 ≈
/// 8.9 kHz in a unit would fold back down into the audible band.
const NOISE_CUTOFF_HZ: f64 = 8_500.0;

// ---------------------------------------------------------------------------
// Plans

/// F1–F3 and their bandwidths, in Hz.
#[derive(Debug, Clone, Copy)]
struct Formants {
    f: [f64; 3],
    b: [f64; 3],
}

impl Formants {
    const fn vowel(f: [f64; 3]) -> Self {
        Self { f, b: VOWEL_BW }
    }

    fn mid(a: [f64; 3], b: [f64; 3]) -> Self {
        Self::vowel([
            f64::midpoint(a[0], b[0]),
            f64::midpoint(a[1], b[1]),
            f64::midpoint(a[2], b[2]),
        ])
    }
}

/// A control value over time: knots of (seconds, value), linear between
/// them and held flat past either end.
#[derive(Debug, Clone, Default)]
struct Curve(Vec<(f64, f64)>);

impl Curve {
    /// Append a knot. A knot earlier than the last becomes a step at the
    /// last knot's time, so plans can't fold time back on itself.
    fn knot(&mut self, t: f64, v: f64) -> &mut Self {
        let t = self.0.last().map_or(t, |&(last, _)| t.max(last));
        self.0.push((t, v));
        self
    }

    fn at(&self, t: f64) -> f64 {
        let i = self.0.partition_point(|&(kt, _)| kt <= t);
        match (i.checked_sub(1).and_then(|j| self.0.get(j)), self.0.get(i)) {
            (Some(&(t0, v0)), Some(&(t1, v1))) => v0 + (v1 - v0) * (t - t0) / (t1 - t0),
            (Some(&(_, v)), None) | (None, Some(&(_, v))) => v,
            (None, None) => 0.0,
        }
    }

    /// One value per sample.
    fn sample(&self, n: usize) -> Vec<f64> {
        (0..n).map(|i| self.at(to_f64(i) / SR)).collect()
    }

    fn is_silent(&self) -> bool {
        self.0.iter().all(|&(_, v)| v <= 0.0)
    }
}

/// A noise spectrum for frication and bursts: peak-normalised resonators in
/// parallel plus an unfiltered bypass, then a one-pole high-pass.
#[derive(Debug, Clone, Copy)]
struct NoiseShape {
    hp_hz: f64,
    bypass: f64,
    /// (centre Hz, bandwidth Hz, gain); gain 0 is unused.
    peaks: [(f64, f64, f64); 3],
}

/// The nasal murmur's upper pole and its place-specific anti-resonance.
#[derive(Debug, Clone, Copy)]
struct Nasal {
    pole: (f64, f64),
    zero: (f64, f64),
}

/// Everything needed to render one unit.
#[derive(Debug, Default)]
struct Plan {
    len: f64,
    /// F1, F2, F3 then B1, B2, B3.
    formants: [Curve; 6],
    /// Source amplitudes, each relative to the vowel's voicing.
    voicing: Curve,
    aspiration: Curve,
    frication: Curve,
    voice_bar: Curve,
    murmur: Curve,
    noise: Option<NoiseShape>,
    /// Frication rides on the glottal pulses (V, Z, J).
    pulsed_frication: bool,
    nasal: Option<Nasal>,
    /// Where the vowel is steady: calibration and levelling are measured here.
    reference: (f64, f64),
}

impl Plan {
    fn set_formants(&mut self, t: f64, fm: Formants) {
        for (k, curve) in self.formants.iter_mut().enumerate() {
            let v = if k < 3 { fm.f[k] } else { fm.b[k - 3] };
            curve.knot(t, v);
        }
    }

    /// Scale every time in the plan by `k`.
    fn stretch(&mut self, k: f64) {
        let curves = self.formants.iter_mut().chain([
            &mut self.voicing,
            &mut self.aspiration,
            &mut self.frication,
            &mut self.voice_bar,
            &mut self.murmur,
        ]);
        for curve in curves {
            for knot in &mut curve.0 {
                knot.0 *= k;
            }
        }
        self.len *= k;
        self.reference = (self.reference.0 * k, self.reference.1 * k);
    }
}

/// What a consonant hands on to its vowel.
struct Handoff {
    /// Voicing starts rising here (or, for sonorants, rising further).
    at: f64,
    /// ...and reaches full level this much later.
    rise: f64,
    /// Formants reach the vowel's start target by this time.
    settle: f64,
}

#[derive(Debug, Clone, Copy)]
enum Place {
    Labial,
    Labiodental,
    Dental,
    Alveolar,
    Postalveolar,
    Velar,
}

/// The plan for `unit`, laid out at the default pace and then scaled by
/// `tempo`: consonant and vowel keep their proportions at any pace.
fn plan(unit: Unit, tempo: f64) -> Plan {
    let (start, end) = vowel_targets(unit.nucleus);
    let k = if unit.nucleus == Nucleus::Schwa {
        BARE_SCALE
    } else {
        1.0
    };
    let mut p = Plan::default();
    let handoff = match unit.onset {
        None => {
            p.voicing.knot(0.0, 0.0);
            Handoff {
                at: 0.0,
                rise: 0.02,
                settle: 0.0,
            }
        }
        Some(o) => consonant(&mut p, o, start, k),
    };
    vowel(&mut p, &handoff, unit.nucleus, start, end);
    p.stretch(tempo);
    p
}

/// F1–F3 at the start and end of each nucleus. Monophthongs hold still;
/// diphthongs glide.
fn vowel_targets(n: Nucleus) -> (Formants, Formants) {
    // Hillenbrand et al. (1995), adult female means, by keyword.
    const HAD: [f64; 3] = [669.0, 2_349.0, 2_972.0];
    const HEAD: [f64; 3] = [731.0, 2_058.0, 2_979.0];
    const HID: [f64; 3] = [483.0, 2_365.0, 3_053.0];
    const HOD: [f64; 3] = [936.0, 1_551.0, 2_815.0];
    const HUD: [f64; 3] = [753.0, 1_426.0, 2_933.0];
    const HAYED: [f64; 3] = [536.0, 2_530.0, 3_047.0];
    const HEED: [f64; 3] = [437.0, 2_761.0, 3_372.0];
    const HOED: [f64; 3] = [555.0, 1_035.0, 2_828.0];
    const HAWED: [f64; 3] = [781.0, 1_136.0, 2_824.0];
    const HOOD: [f64; 3] = [519.0, 1_225.0, 2_827.0];
    const WHOD: [f64; 3] = [459.0, 1_105.0, 2_735.0];
    // A uniform tube the length of an adult female vocal tract resonates at
    // odd quarter wavelengths (Fant 1960): the neutral vowel.
    const NEUTRAL: [f64; 3] = [600.0, 1_800.0, 3_000.0];

    let still = |f| (Formants::vowel(f), Formants::vowel(f));
    match n {
        Nucleus::A => still(HAD),
        Nucleus::E => still(HEAD),
        Nucleus::I => still(HID),
        Nucleus::O => still(HOD),
        Nucleus::U => still(HUD),
        Nucleus::LongE => still(HEED),
        Nucleus::LongU => still(WHOD),
        Nucleus::Schwa => still(NEUTRAL),
        // Diphthongs start more open than their steady-state measurement and
        // glide toward the high front or high back corner.
        Nucleus::LongA => (Formants::mid(HEAD, HAYED), Formants::mid(HID, HEED)),
        Nucleus::LongI => (Formants::mid(HOD, HAD), Formants::vowel(HID)),
        Nucleus::LongO => (Formants::mid(HAWED, HOED), Formants::mid(HOOD, WHOD)),
    }
}

/// Lay out the vowel after its consonant: voicing up, a slight decline, a
/// natural decay, and any diphthong glide.
fn vowel(p: &mut Plan, h: &Handoff, n: Nucleus, start: Formants, end: Formants) {
    // (whole-unit length, least vowel kept after a long consonant, decay)
    let (total, min_body, decay) = match n {
        Nucleus::Schwa => (0.105, 0.05, 0.03),
        Nucleus::LongA | Nucleus::LongE | Nucleus::LongI | Nucleus::LongO | Nucleus::LongU => {
            (0.195, 0.115, 0.05)
        }
        _ => (0.165, 0.095, 0.045),
    };
    let body = (total - h.at).max(min_body);
    let len = h.at + body;
    let rise = h.rise.min(0.6 * body);
    let fade = (len - decay).max(h.at + rise);
    p.voicing
        .knot(h.at + rise, 1.0)
        .knot(fade, DECLINE)
        .knot(fade + 0.5 * (len - fade), 0.4 * DECLINE)
        .knot(len, 0.0);

    let settle = h.settle.min(fade);
    p.set_formants(settle, start);
    if matches!(n, Nucleus::LongA | Nucleus::LongI | Nucleus::LongO) {
        p.set_formants(h.at + 0.3 * body, start);
        p.set_formants(h.at + 0.85 * body, end);
    }

    p.len = len;
    let steady = (settle.max(h.at + rise), fade);
    p.reference = if steady.1 - steady.0 < 0.02 {
        (h.at, len)
    } else {
        steady
    };
}

/// Formants at a consonant release. Transitions point at a place-specific
/// locus but start only part of the way there (Delattre, Liberman & Cooper
/// 1955), so the onset blends locus and vowel. Loci are raised from their
/// synthetic-speech values toward an adult-female vocal tract.
fn release(place: Place, v: Formants) -> Formants {
    const F1_LOCUS: f64 = 250.0;
    const F1_REACH: f64 = 0.3;
    const REACH: f64 = 0.5;
    let [f1, f2, f3] = v.f;
    let (l2, l3) = match place {
        Place::Labial => (850.0, 2_350.0),
        Place::Labiodental => (1_050.0, 2_500.0),
        Place::Dental => (1_500.0, 2_750.0),
        Place::Alveolar => (1_900.0, 2_950.0),
        Place::Postalveolar => (2_300.0, 2_750.0),
        // Velars have no single locus: F2 starts high before front vowels and
        // low before back ones, with F3 close above it (the velar pinch).
        Place::Velar => {
            let l2 = (1_400.0 + 0.9 * (f2 - 1_000.0)).clamp(1_400.0, 3_000.0);
            (l2, (l2 + 350.0).max(2_500.0))
        }
    };
    Formants {
        f: [
            F1_LOCUS + F1_REACH * (f1 - F1_LOCUS),
            l2 + REACH * (f2 - l2),
            l3 + REACH * (f3 - l3),
        ],
        b: v.b,
    }
}

fn consonant(p: &mut Plan, o: Onset, v: Formants, k: f64) -> Handoff {
    match o {
        Onset::P | Onset::T | Onset::K => voiceless_stop(p, o, v, k),
        Onset::B | Onset::D | Onset::G => voiced_stop(p, o, v, k),
        Onset::F | Onset::Th | Onset::S | Onset::Sh | Onset::V | Onset::Z => fricative(p, o, v, k),
        Onset::H => aspirate(p, v, k),
        Onset::Ch | Onset::J => affricate(p, o == Onset::J, v, k),
        Onset::M | Onset::N | Onset::Ng => nasal(p, o, v, k),
        Onset::L | Onset::R | Onset::W | Onset::Y => approximant(p, o, k),
    }
}

/// P, T, K: a burst, then aspiration while the formants move, then voicing
/// after a long voice-onset time. Lisker & Abramson (1964) measured roughly
/// 60/70/80 ms in English; babble runs them shorter, in the same order.
fn voiceless_stop(p: &mut Plan, o: Onset, v: Formants, k: f64) -> Handoff {
    let (place, burst_ms, vot_ms, burst_db, asp_db) = match o {
        Onset::P => (Place::Labial, 6.0, 45.0, -14.0, -13.0),
        Onset::T => (Place::Alveolar, 9.0, 55.0, -5.0, -12.0),
        _ => (Place::Velar, 12.0, 60.0, -7.0, -11.0),
    };
    let mut rel = release(place, v);
    let burst = ms(burst_ms * k);
    let at = LEAD + ms(vot_ms * k);
    let (burst_level, asp_level) = (db(burst_db), db(asp_db));
    p.frication
        .knot(LEAD, 0.0)
        .knot(LEAD + ms(1.0), burst_level)
        .knot(LEAD + burst, 0.3 * burst_level)
        .knot(LEAD + burst + ms(3.0), 0.0);
    p.aspiration
        .knot(LEAD + ms(2.0), 0.0)
        .knot(LEAD + ms(6.0), asp_level)
        .knot(at - ms(5.0), 0.8 * asp_level)
        .knot(at + ms(10.0), 0.0);
    p.voicing.knot(at, 0.0);
    p.noise = Some(noise_shape(o, rel));
    rel.b[0] = OPEN_B1;
    p.set_formants(LEAD, rel);
    Handoff {
        at,
        rise: ms(10.0),
        settle: at + ms(20.0),
    }
}

/// B, D, G: a voice bar through the closure, a weak burst, and voicing
/// almost at once (short-lag VOT).
fn voiced_stop(p: &mut Plan, o: Onset, v: Formants, k: f64) -> Handoff {
    let (place, burst_ms, vot_ms, burst_db) = match o {
        Onset::B => (Place::Labial, 4.0, 6.0, -18.0),
        Onset::D => (Place::Alveolar, 6.0, 8.0, -10.0),
        _ => (Place::Velar, 8.0, 12.0, -10.0),
    };
    let rel = release(place, v);
    let release_at = ms(18.0 * k);
    let bar = db(VOICE_BAR_DB);
    p.voice_bar
        .knot(0.0, 0.0)
        .knot(ms(4.0), bar)
        .knot(release_at, bar)
        .knot(release_at + ms(4.0), 0.0);
    p.frication
        .knot(release_at, 0.0)
        .knot(release_at + ms(1.0), db(burst_db))
        .knot(release_at + ms(burst_ms * k), 0.0);
    let at = release_at + ms(vot_ms * k);
    p.voicing.knot(at, 0.0);
    p.noise = Some(noise_shape(o, rel));
    p.set_formants(release_at, rel);
    Handoff {
        at,
        rise: ms(8.0),
        settle: release_at + ms(45.0 * k),
    }
}

/// S, Sh, F, Th and voiced V, Z. Sibilants are strong and non-sibilants
/// faint (Jongman et al. 2000) — though F, Th and V sit a few dB above
/// natural speech so they survive the overlap of fast babble. Voiced ones
/// add a voice bar and ride their noise on the glottal pulses. Durations are
/// short for fricatives: every letter gets an equal slot, and the renderer
/// runs a hiss ahead of its beat, into the syllable before.
fn fricative(p: &mut Plan, o: Onset, v: Formants, k: f64) -> Handoff {
    let (place, dur_ms, fric_db, voiced) = match o {
        Onset::S => (Place::Alveolar, 70.0, -9.0, false),
        Onset::Sh => (Place::Postalveolar, 70.0, -6.0, false),
        Onset::F => (Place::Labiodental, 60.0, -13.0, false),
        Onset::Th => (Place::Dental, 60.0, -14.0, false),
        Onset::V => (Place::Labiodental, 50.0, -15.0, true),
        _ => (Place::Alveolar, 55.0, -13.0, true),
    };
    let dur = ms(dur_ms * k);
    let level = db(fric_db);
    p.frication
        .knot(0.0, 0.0)
        .knot(ms(15.0), level)
        .knot(dur - ms(10.0), level)
        .knot(dur + ms(4.0), 0.0);
    if voiced {
        let bar = db(VOICE_BAR_DB);
        p.voice_bar
            .knot(0.0, 0.0)
            .knot(ms(10.0), bar)
            .knot(dur - ms(5.0), bar)
            .knot(dur + ms(5.0), 0.0);
        p.pulsed_frication = true;
    }
    let at = dur - ms(10.0);
    p.voicing.knot(at, 0.0);
    let rel = release(place, v);
    p.noise = Some(noise_shape(o, rel));
    p.set_formants(at, rel);
    Handoff {
        at,
        rise: ms(15.0),
        settle: at + ms(45.0 * k),
    }
}

/// H: aspiration through the vowel's own formants, with F1 damped by the
/// open glottis, fading under the voicing onset.
fn aspirate(p: &mut Plan, v: Formants, k: f64) -> Handoff {
    let dur = ms(50.0 * k);
    let level = db(-12.0);
    p.aspiration
        .knot(0.0, 0.0)
        .knot(ms(12.0), level)
        .knot(dur - ms(8.0), level)
        .knot(dur + ms(15.0), 0.0);
    let at = dur - ms(10.0);
    p.voicing.knot(at, 0.0);
    let mut open = v;
    open.b[0] = OPEN_B1;
    p.set_formants(0.0, open);
    Handoff {
        at,
        rise: ms(20.0),
        settle: dur + ms(10.0),
    }
}

/// Ch = T + Sh; J = D + voiced Sh. The release burst and the frication share
/// the postalveolar noise shape.
fn affricate(p: &mut Plan, voiced: bool, v: Formants, k: f64) -> Handoff {
    let rel = release(Place::Postalveolar, v);
    p.noise = Some(noise_shape(Onset::Sh, rel));
    let at = if voiced {
        let release_at = ms(14.0 * k);
        let fric = ms(40.0 * k);
        let bar = db(VOICE_BAR_DB);
        p.voice_bar
            .knot(0.0, 0.0)
            .knot(ms(4.0), bar)
            .knot(release_at + fric, bar)
            .knot(release_at + fric + ms(5.0), 0.0);
        let level = db(-13.0);
        p.frication
            .knot(release_at, 0.0)
            .knot(release_at + ms(1.0), db(-8.0))
            .knot(release_at + ms(5.0), 0.8 * level)
            .knot(release_at + fric - ms(5.0), level)
            .knot(release_at + fric + ms(4.0), 0.0);
        p.pulsed_frication = true;
        release_at + fric - ms(8.0)
    } else {
        let fric = ms(50.0 * k);
        let level = db(-7.0);
        p.frication
            .knot(LEAD, 0.0)
            .knot(LEAD + ms(1.0), db(-3.0))
            .knot(LEAD + ms(6.0), 0.7 * level)
            .knot(LEAD + ms(20.0 * k), level)
            .knot(LEAD + fric - ms(8.0), level)
            .knot(LEAD + fric + ms(4.0), 0.0);
        LEAD + fric - ms(8.0)
    };
    p.voicing.knot(at, 0.0);
    p.set_formants(at, rel);
    Handoff {
        at,
        rise: ms(12.0),
        settle: at + ms(40.0 * k),
    }
}

/// M, N, Ng: a low murmur with a place-specific anti-resonance (Fujimura
/// 1962: roughly 750–1250 Hz for m, 1450–2200 Hz for n, above 3 kHz for ng),
/// then an abrupt oral release with a little nasality carried into the vowel.
fn nasal(p: &mut Plan, o: Onset, v: Formants, k: f64) -> Handoff {
    let (place, pole, zero, murmur_db) = match o {
        Onset::M => (Place::Labial, (1_350.0, 300.0), (1_000.0, 150.0), -9.0),
        Onset::N => (Place::Alveolar, (2_200.0, 300.0), (1_800.0, 200.0), -9.0),
        _ => (Place::Velar, (2_400.0, 300.0), (3_200.0, 300.0), -10.0),
    };
    let murmur = ms(45.0 * k);
    let level = db(murmur_db);
    p.murmur
        .knot(0.0, 0.0)
        .knot(ms(10.0), level)
        .knot(murmur, level)
        .knot(murmur + ms(10.0), 0.3 * level)
        .knot(murmur + ms(45.0 * k), 0.0);
    p.voicing.knot(murmur, 0.0);
    p.nasal = Some(Nasal { pole, zero });
    p.set_formants(murmur, release(place, v));
    Handoff {
        at: murmur,
        rise: ms(6.0),
        settle: murmur + ms(40.0 * k),
    }
}

/// L, R, W, Y: voiced throughout, holding their own formants briefly, then
/// gliding slowly into the vowel. Targets are typical adult-female values:
/// L with a mid F2, R with its low F3, W low F1/F2, Y high F2.
fn approximant(p: &mut Plan, o: Onset, k: f64) -> Handoff {
    const LATERAL: Formants = Formants {
        f: [380.0, 1_350.0, 2_850.0],
        b: [120.0, 200.0, 250.0],
    };
    const RHOTIC: Formants = Formants {
        f: [420.0, 1_250.0, 1_750.0],
        b: [100.0, 150.0, 150.0],
    };
    const LABIAL_GLIDE: Formants = Formants {
        f: [330.0, 760.0, 2_450.0],
        b: [100.0, 120.0, 200.0],
    };
    const PALATAL_GLIDE: Formants = Formants {
        f: [300.0, 2_650.0, 3_300.0],
        b: [90.0, 150.0, 200.0],
    };
    let (target, hold_ms, glide_ms, level) = match o {
        Onset::L => (LATERAL, 40.0, 55.0, 0.6),
        // R and W crowd their formants together, which lifts the cascade's
        // own gain; they need less source to sit under the vowel.
        Onset::R => (RHOTIC, 40.0, 70.0, 0.35),
        Onset::W => (LABIAL_GLIDE, 30.0, 80.0, 0.4),
        _ => (PALATAL_GLIDE, 30.0, 65.0, 0.55),
    };
    let hold = ms(hold_ms * k);
    let glide = ms(glide_ms * k);
    p.voicing
        .knot(0.0, 0.0)
        .knot(ms(15.0), level)
        .knot(hold, level);
    p.set_formants(0.0, target);
    p.set_formants(hold, target);
    Handoff {
        at: hold,
        rise: glide,
        settle: hold + glide,
    }
}

/// Noise spectra by place. Fricative peaks follow Jongman et al. (2000):
/// s high, sh lower and stronger, f/th flat and diffuse. Bursts follow
/// Stevens & Blumstein (1978): labial diffuse-falling, alveolar
/// diffuse-rising, velar compact near the F2/F3 onset. Peaks stay at or
/// under 8 kHz for the default tract; a larger tract scale can lift them past
/// [`NOISE_CUTOFF_HZ`], where the low-pass takes them off.
fn noise_shape(o: Onset, rel: Formants) -> NoiseShape {
    let (hp_hz, bypass, peaks) = match o {
        Onset::S | Onset::Z => (
            3_000.0,
            0.0,
            [
                (5_200.0, 900.0, 0.5),
                (6_600.0, 1_200.0, 1.0),
                (8_000.0, 1_500.0, 0.5),
            ],
        ),
        Onset::Sh | Onset::Ch | Onset::J => (
            1_400.0,
            0.0,
            [
                (2_700.0, 450.0, 0.6),
                (3_600.0, 800.0, 1.0),
                (5_400.0, 1_500.0, 0.3),
            ],
        ),
        Onset::F | Onset::V => (
            700.0,
            0.5,
            [(7_000.0, 3_000.0, 0.5), (0.0, 0.0, 0.0), (0.0, 0.0, 0.0)],
        ),
        Onset::Th => (
            700.0,
            0.5,
            [(5_500.0, 3_000.0, 0.35), (0.0, 0.0, 0.0), (0.0, 0.0, 0.0)],
        ),
        Onset::P | Onset::B => (
            150.0,
            0.25,
            [
                (900.0, 1_200.0, 1.0),
                (2_400.0, 2_000.0, 0.3),
                (0.0, 0.0, 0.0),
            ],
        ),
        Onset::T | Onset::D => (
            1_500.0,
            0.15,
            [
                (4_300.0, 1_500.0, 0.6),
                (6_300.0, 2_000.0, 1.0),
                (0.0, 0.0, 0.0),
            ],
        ),
        _ => (
            500.0,
            0.05,
            [
                (rel.f[1], 350.0, 1.0),
                (rel.f[2], 500.0, 0.35),
                (0.0, 0.0, 0.0),
            ],
        ),
    };
    NoiseShape {
        hp_hz,
        bypass,
        peaks,
    }
}

// ---------------------------------------------------------------------------
// Rendering

fn seed(u: Unit) -> u64 {
    0xD1B5_4A32_D192_ED03 ^ (u.index() as u64)
}

/// Render one unit, and find where its vowel gets loud.
fn synthesize(p: &Plan, seed: u64, timbre: Timbre, noise_cutoff: Option<f64>) -> (Vec<f32>, usize) {
    let n = samples(p.len);
    let reference = samples(p.reference.0).min(n)..samples(p.reference.1).min(n);
    let k = timbre.tract;
    let mut rng = Rng(seed);
    let glottis = Glottis::new(n, p.len, timbre, &mut rng);
    // Noise is calibrated at full band, then low-passed: what stays below
    // the cutoff sounds exactly as loud as it did before band-limiting.
    let band_limit = |mut x: Vec<f64>| {
        if let Some(hz) = noise_cutoff {
            LowPass::new(hz).run(&mut x);
        }
        x
    };

    let mut voice = cascade(&glottis.flow, p, k);
    radiate(&mut voice);
    let voicing = p.voicing.sample(n);
    let mut mix: Vec<f64> = voice.iter().zip(&voicing).map(|(x, a)| x * a).collect();
    let vowel = rms(&mix[reference.clone()]);
    let lead = onset(&mix, vowel);

    // Aspiration shares the tract's formants, so its level depends on where
    // they are: calibrate it where it sounds. Breathiness is aspiration too,
    // calibrated on the vowel it rides on, stronger while the folds are open.
    let aspiration = cascade(&rng.noise(n), p, k);
    let ah = p.aspiration.sample(n);
    let ah_gain = calibrate(vowel, &aspiration, &ah);
    let breath = db(BREATH_DB) * finite_or_zero(vowel / rms(&aspiration[reference.clone()]));
    let envelope: Vec<f64> = ah
        .iter()
        .zip(&voicing)
        .zip(&glottis.open)
        .map(|((ah, av), open)| ah * ah_gain + breath * av * (0.5 + 0.5 * open.min(1.0)))
        .collect();
    add(&mut mix, &band_limit(aspiration), &envelope, 1.0);

    if let Some(shape) = p.noise {
        let noise = frication(&rng.noise(n), &shape, k);
        let mut envelope = p.frication.sample(n);
        let gain = calibrate(vowel, &noise, &envelope);
        if p.pulsed_frication {
            for (e, open) in envelope.iter_mut().zip(&glottis.open) {
                *e *= 0.4 + 0.6 * open.min(1.0);
            }
        }
        add(&mut mix, &band_limit(noise), &envelope, gain);
    }
    if !p.voice_bar.is_silent() {
        let mut bar = Resonator::new(VOICE_BAR_HZ * k, 120.0 * k);
        let mut carrier: Vec<f64> = glottis.flow.iter().map(|&x| bar.tick(x)).collect();
        radiate(&mut carrier);
        let envelope = p.voice_bar.sample(n);
        add(
            &mut mix,
            &carrier,
            &envelope,
            calibrate(vowel, &carrier, &envelope),
        );
    }
    if let Some(shape) = p.nasal {
        let carrier = murmur(&glottis.flow, shape, k);
        let envelope = p.murmur.sample(n);
        add(
            &mut mix,
            &carrier,
            &envelope,
            calibrate(vowel, &carrier, &envelope),
        );
    }
    (finish(mix, reference), lead)
}

/// Where the voiced branch first reaches half the vowel's level (-6 dB),
/// measured over 10 ms centred on each sample. This is where a unit "gets
/// loud": the renderer puts it on the beat, so a long consonant runs ahead
/// of its slot instead of dragging the vowel late. Measured rather than read
/// from the voicing curve, because approximants start voiced and their
/// crowded formants lift the level well before the curve says.
fn onset(voiced: &[f64], vowel: f64) -> usize {
    const HALF: usize = 120;
    let threshold = 0.25 * vowel * vowel * to_f64(2 * HALF);
    let mut prefix = Vec::with_capacity(voiced.len() + 1);
    prefix.push(0.0);
    let mut sum = 0.0;
    for x in voiced {
        sum += x * x;
        prefix.push(sum);
    }
    let window = |i: usize| prefix[(i + HALF).min(voiced.len())] - prefix[i.saturating_sub(HALF)];
    (0..voiced.len())
        .find(|&i| window(i) >= threshold)
        .unwrap_or(0)
}

/// The gain that makes `carrier` as loud as the vowel wherever `envelope`
/// is 1: its RMS is weighted by the envelope, so only the stretch where the
/// source actually sounds counts.
fn calibrate(vowel: f64, carrier: &[f64], envelope: &[f64]) -> f64 {
    let (mut num, mut den) = (0.0, 0.0);
    for (c, e) in carrier.iter().zip(envelope) {
        num += (c * e) * (c * e);
        den += e * e;
    }
    finite_or_zero(vowel / (num / den).sqrt())
}

/// `mix += carrier · envelope · gain`.
fn add(mix: &mut [f64], carrier: &[f64], envelope: &[f64], gain: f64) {
    for ((m, c), e) in mix.iter_mut().zip(carrier).zip(envelope) {
        *m += c * e * gain;
    }
}

fn finite_or_zero(x: f64) -> f64 {
    if x.is_finite() {
        x
    } else {
        0.0
    }
}

/// DC-block, level the vowel to [`UNIT_RMS`] with peaks kept under
/// [`PEAK_MAX`], and hand back `f32` with nothing subnormal in it.
fn finish(mut mix: Vec<f64>, reference: Range<usize>) -> Vec<f32> {
    let mut dc = HighPass::new(DC_BLOCK_HZ);
    for x in &mut mix {
        *x = dc.tick(*x);
    }
    let level = rms(&mix[reference]);
    let peak = mix.iter().fold(0.0_f64, |m, x| m.max(x.abs()));
    let mut gain = if level > 1e-12 { UNIT_RMS / level } else { 0.0 };
    if peak * gain > PEAK_MAX {
        gain = PEAK_MAX / peak;
    }
    mix.iter()
        .map(|&x| {
            let y = x * gain;
            #[allow(clippy::cast_possible_truncation)]
            if y.is_finite() && y.abs() >= 1e-20 {
                y as f32
            } else {
                0.0
            }
        })
        .collect()
}

/// The voicing source for one unit.
struct Glottis {
    /// Volume velocity after spectral tilt: what drives the voiced branches.
    flow: Vec<f64>,
    /// How open the folds are, sample by sample (the pulse before tilt).
    open: Vec<f64>,
}

impl Glottis {
    /// KLGLOTT88 (Klatt & Klatt 1990): over the open phase the flow is
    /// `a·t² − b·t³`, zero while the folds are closed, then low-passed for
    /// spectral tilt. F0 glides from the timbre's pitch down by [`F0_FALL`],
    /// and each new cycle draws a little jitter and shimmer.
    fn new(n: usize, len: f64, timbre: Timbre, rng: &mut Rng) -> Self {
        let tilt = 1.0 - exp(-TAU * (TILT_HZ * timbre.tract) / SR);
        let (f0_start, f0_end) = (timbre.f0, timbre.f0 * F0_FALL);
        let mut flow = Vec::with_capacity(n);
        let mut open = Vec::with_capacity(n);
        let (mut phase, mut wobble, mut amp, mut lp) = (0.0, 1.0, 1.0, 0.0);
        for i in 0..n {
            let tau = phase / OPEN_QUOTIENT;
            // 27/4 · (τ² − τ³) peaks at exactly 1, at τ = 2/3.
            let u = if tau < 1.0 {
                6.75 * tau * tau * (1.0 - tau) * amp
            } else {
                0.0
            };
            lp += tilt * (u - lp);
            flow.push(lp);
            open.push(u);
            let progress = (to_f64(i) / SR / len).min(1.0);
            let f0 = f0_start + (f0_end - f0_start) * progress;
            phase += f0 * wobble / SR;
            if phase >= 1.0 {
                phase -= 1.0;
                wobble = 1.0 + JITTER * rng.signed();
                amp = 1.0 + SHIMMER * rng.signed();
            }
        }
        Self { flow, open }
    }
}

/// A source through the vocal-tract cascade (Klatt 1980's cascade branch):
/// F1–F3 follow the plan, the upper formants hold, and every frequency and
/// bandwidth is scaled by the tract factor `scale`.
fn cascade(src: &[f64], p: &Plan, scale: f64) -> Vec<f64> {
    let mut tract = [Resonator::default(); 3 + UPPER.len()];
    for (r, &(f, bw)) in tract[3..].iter_mut().zip(&UPPER) {
        r.tune(f * scale, bw * scale);
    }
    let mut out = Vec::with_capacity(src.len());
    for (k, block) in src.chunks(BLOCK).enumerate() {
        let t = to_f64(k * BLOCK) / SR;
        for (j, r) in tract[..3].iter_mut().enumerate() {
            r.tune(p.formants[j].at(t) * scale, p.formants[3 + j].at(t) * scale);
        }
        out.extend(
            block
                .iter()
                .map(|&x| tract.iter_mut().fold(x, |y, r| r.tick(y))),
        );
    }
    out
}

/// Frication or burst noise through its parallel peaks, tract-scaled by `k`.
fn frication(noise: &[f64], shape: &NoiseShape, k: f64) -> Vec<f64> {
    let mut peaks: Vec<(Resonator, f64)> = shape
        .peaks
        .iter()
        .filter(|&&(_, _, gain)| gain > 0.0)
        .map(|&(f, bw, gain)| (Resonator::peak_normalised(f * k, bw * k), gain))
        .collect();
    let mut hp = HighPass::new(shape.hp_hz * k);
    noise
        .iter()
        .map(|&x| {
            let y = peaks
                .iter_mut()
                .fold(shape.bypass * x, |acc, (r, gain)| acc + *gain * r.tick(x));
            hp.tick(y)
        })
        .collect()
}

/// Nasal murmur: the low nasal pole, an upper pole, and the oral cavity's
/// anti-resonance (Klatt 1980's nasal pole-zero pair, with a place zero),
/// tract-scaled by `k`.
fn murmur(flow: &[f64], shape: Nasal, k: f64) -> Vec<f64> {
    let mut low = Resonator::new(270.0 * k, 100.0 * k);
    let mut high = Resonator::new(shape.pole.0 * k, shape.pole.1 * k);
    let mut zero = AntiResonator::new(shape.zero.0 * k, shape.zero.1 * k);
    let mut out: Vec<f64> = flow
        .iter()
        .map(|&x| zero.tick(high.tick(low.tick(x))))
        .collect();
    radiate(&mut out);
    out
}

/// Lip radiation: a first difference, roughly +6 dB/octave (Klatt 1980).
/// Applied to volume-velocity branches only; the noise sources are already
/// shaped as radiated pressure.
fn radiate(x: &mut [f64]) {
    let mut prev = 0.0;
    for s in x {
        let v = *s;
        *s = v - prev;
        prev = v;
    }
}

// ---------------------------------------------------------------------------
// Filters

/// Klatt (1980) second-order resonator:
/// `y[n] = A·x[n] + B·y[n−1] + C·y[n−2]` with `C = −exp(−2π·BW·T)`,
/// `B = 2·exp(−π·BW·T)·cos(2π·F·T)` and `A = 1 − B − C` (unity gain at DC).
#[derive(Debug, Clone, Copy, Default)]
struct Resonator {
    a: f64,
    b: f64,
    c: f64,
    y1: f64,
    y2: f64,
}

impl Resonator {
    fn new(f: f64, bw: f64) -> Self {
        let mut r = Self::default();
        r.tune(f, bw);
        r
    }

    fn tune(&mut self, f: f64, bw: f64) {
        let r = exp(-PI * bw / SR);
        self.c = -r * r;
        self.b = 2.0 * r * cos(TAU * f / SR);
        self.a = 1.0 - self.b - self.c;
    }

    /// The same poles with `A` chosen for unity gain at the centre frequency,
    /// so parallel branch gains read as peak levels.
    fn peak_normalised(f: f64, bw: f64) -> Self {
        let mut r = Self::new(f, bw);
        // |1 − B·e^(−jθ) − C·e^(−2jθ)| at θ = 2πF/fs.
        let cos1 = cos(TAU * f / SR);
        let sin1 = (1.0 - cos1 * cos1).max(0.0).sqrt();
        let (cos2, sin2) = (2.0 * cos1 * cos1 - 1.0, 2.0 * sin1 * cos1);
        let re = 1.0 - r.b * cos1 - r.c * cos2;
        let im = r.b * sin1 + r.c * sin2;
        r.a = (re * re + im * im).sqrt();
        r
    }

    fn tick(&mut self, x: f64) -> f64 {
        let y = self.a * x + self.b * self.y1 + self.c * self.y2;
        self.y2 = self.y1;
        self.y1 = y;
        y
    }
}

/// Klatt (1980) anti-resonator: `y[n] = A'·x[n] + B'·x[n−1] + C'·x[n−2]`
/// with `A' = 1/A`, `B' = −B/A`, `C' = −C/A` from the resonator at the same
/// frequency and bandwidth.
#[derive(Debug, Clone, Copy)]
struct AntiResonator {
    a: f64,
    b: f64,
    c: f64,
    x1: f64,
    x2: f64,
}

impl AntiResonator {
    fn new(f: f64, bw: f64) -> Self {
        let r = Resonator::new(f, bw);
        Self {
            a: 1.0 / r.a,
            b: -r.b / r.a,
            c: -r.c / r.a,
            x1: 0.0,
            x2: 0.0,
        }
    }

    fn tick(&mut self, x: f64) -> f64 {
        let y = self.a * x + self.b * self.x1 + self.c * self.x2;
        self.x2 = self.x1;
        self.x1 = x;
        y
    }
}

/// One-pole high-pass: `y[n] = a·(y[n−1] + x[n] − x[n−1])`.
struct HighPass {
    a: f64,
    x1: f64,
    y1: f64,
}

impl HighPass {
    fn new(hz: f64) -> Self {
        Self {
            a: exp(-TAU * hz / SR),
            x1: 0.0,
            y1: 0.0,
        }
    }

    fn tick(&mut self, x: f64) -> f64 {
        self.y1 = self.a * (self.y1 + x - self.x1);
        self.x1 = x;
        self.y1
    }
}

/// Butterworth low-pass, 8th order, as four biquads (bilinear transform,
/// Bristow-Johnson's cookbook coefficients).
struct LowPass([Biquad; 4]);

impl LowPass {
    fn new(hz: f64) -> Self {
        let w = TAU * hz / SR;
        let (c, s) = (cos(w), sin(w));
        // Section Qs for 8th order are 1 / (2·cos((2k − 1)·π/16)), so each
        // section's alpha = sin(w) / 2Q is sin(w)·cos((2k − 1)·π/16).
        Self([1.0, 3.0, 5.0, 7.0].map(|odd| {
            let alpha = s * cos(odd * PI / 16.0);
            let a0 = 1.0 + alpha;
            Biquad {
                b0: 0.5 * (1.0 - c) / a0,
                b1: (1.0 - c) / a0,
                a1: -2.0 * c / a0,
                a2: (1.0 - alpha) / a0,
                ..Biquad::default()
            }
        }))
    }

    fn run(&mut self, x: &mut [f64]) {
        for s in x {
            *s = self.0.iter_mut().fold(*s, |y, q| q.tick(y));
        }
    }
}

/// One low-pass biquad section (`b2 = b0`), direct form I.
#[derive(Debug, Clone, Copy, Default)]
struct Biquad {
    b0: f64,
    b1: f64,
    a1: f64,
    a2: f64,
    x1: f64,
    x2: f64,
    y1: f64,
    y2: f64,
}

impl Biquad {
    fn tick(&mut self, x: f64) -> f64 {
        let y = self.b0 * (x + self.x2) + self.b1 * self.x1 - self.a1 * self.y1 - self.a2 * self.y2;
        (self.x2, self.x1) = (self.x1, x);
        (self.y2, self.y1) = (self.y1, y);
        y
    }
}

// ---------------------------------------------------------------------------
// Numbers

/// `SplitMix64` (Steele, Lea & Flood 2014): tiny and identical everywhere.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniform in `[-1, 1)`.
    fn signed(&mut self) -> f64 {
        // Top 53 bits → [0, 1).
        #[allow(clippy::cast_precision_loss)]
        let unit = (self.next() >> 11) as f64 / (1_u64 << 53) as f64;
        2.0 * unit - 1.0
    }

    /// Zero-mean noise with a triangular distribution (two uniforms summed).
    fn noise(&mut self, n: usize) -> Vec<f64> {
        (0..n)
            .map(|_| f64::midpoint(self.signed(), self.signed()))
            .collect()
    }
}

/// `e^x` from plain arithmetic. `f64::exp` defers to the platform's libm,
/// which may differ in the last bit between operating systems; building the
/// bank from `+ − × ÷` alone makes it bit-identical everywhere.
fn exp(x: f64) -> f64 {
    if !x.is_finite() {
        return if x > 0.0 { f64::INFINITY } else { 0.0 };
    }
    // Halve into Taylor range, sum, square back up.
    let mut r = x;
    let mut halvings = 0;
    while r.abs() > 0.01 && halvings < 64 {
        r *= 0.5;
        halvings += 1;
    }
    let mut s = 1.0
        + r * (1.0
            + r * (0.5 + r * (1.0 / 6.0 + r * (1.0 / 24.0 + r * (1.0 / 120.0 + r / 720.0)))));
    for _ in 0..halvings {
        s *= s;
    }
    s
}

/// `cos x` for `0 ≤ x ≤ π` (any frequency up to Nyquist), by Horner over its
/// Taylor series — plain arithmetic, for the same reason as [`exp`].
fn cos(x: f64) -> f64 {
    let x2 = x * x;
    (1..=16).rev().fold(1.0, |acc, k: u32| {
        let k = f64::from(k);
        1.0 - x2 / ((2.0 * k - 1.0) * (2.0 * k)) * acc
    })
}

/// `sin x` for `0 ≤ x ≤ π`, from [`cos`].
fn sin(x: f64) -> f64 {
    let c = cos(x);
    (1.0 - c * c).max(0.0).sqrt()
}

/// Decibels to a linear amplitude ratio.
fn db(x: f64) -> f64 {
    exp(x * LN_10 / 20.0)
}

fn ms(x: f64) -> f64 {
    x / 1_000.0
}

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn samples(secs: f64) -> usize {
    (secs * SR).round().max(0.0) as usize
}

#[allow(clippy::cast_precision_loss)]
fn to_f64(n: usize) -> f64 {
    n as f64
}

fn rms(x: &[f64]) -> f64 {
    if x.is_empty() {
        return 0.0;
    }
    (x.iter().map(|v| v * v).sum::<f64>() / to_f64(x.len())).sqrt()
}

#[cfg(test)]
mod tests {
    use super::super::{bank as voice_bank, VOICES};
    use super::*;
    use realfft::RealFftPlanner;
    use std::sync::OnceLock;

    /// One default bank shared by the tests: building it is the expensive part.
    fn bank() -> &'static FormantBank {
        static BANK: OnceLock<FormantBank> = OnceLock::new();
        BANK.get_or_init(FormantBank::new)
    }

    /// Every shipped voice with its (shared, cached) bank.
    fn voices() -> impl Iterator<Item = (&'static super::super::BabbleVoice, &'static FormantBank)>
    {
        VOICES.iter().enumerate().map(|(i, v)| (v, voice_bank(i)))
    }

    fn unit(onset: Option<Onset>, nucleus: Nucleus) -> &'static [f32] {
        bank().unit(Unit::new(onset, nucleus))
    }

    fn slice_ms(x: &[f32], from: f64, to: f64) -> &[f32] {
        &x[samples(ms(from))..samples(ms(to)).min(x.len())]
    }

    fn energy(x: &[f32]) -> f64 {
        x.iter().map(|&s| f64::from(s) * f64::from(s)).sum()
    }

    fn rms32(x: &[f32]) -> f64 {
        (energy(x) / to_f64(x.len())).sqrt()
    }

    fn dot(p: &[f32], q: &[f32]) -> f64 {
        p.iter()
            .zip(q)
            .map(|(&i, &j)| f64::from(i) * f64::from(j))
            .sum()
    }

    /// FNV-1a over every unit's length and sample bits.
    fn fingerprint(b: &FormantBank) -> u64 {
        let mut bytes = Vec::new();
        for u in Unit::all() {
            let x = b.unit(u);
            bytes.extend_from_slice(&(x.len() as u64).to_le_bytes());
            for s in x {
                bytes.extend_from_slice(&s.to_bits().to_le_bytes());
            }
        }
        super::super::schedule::fnv1a(&bytes)
    }

    /// Hann-windowed power spectrum as (Hz, power), by direct DFT.
    fn spectrum(x: &[f32]) -> Vec<(f64, f64)> {
        let len = to_f64(x.len());
        let windowed: Vec<f64> = x
            .iter()
            .enumerate()
            .map(|(i, &s)| f64::from(s) * (0.5 - 0.5 * (TAU * to_f64(i) / len).cos()))
            .collect();
        (0..x.len() / 2)
            .map(|bin| {
                let step = TAU * to_f64(bin) / len;
                let (step_cos, step_sin) = (step.cos(), step.sin());
                let (mut re, mut im, mut ph_cos, mut ph_sin) = (0.0, 0.0, 1.0, 0.0);
                for &s in &windowed {
                    re += s * ph_cos;
                    im += s * ph_sin;
                    (ph_cos, ph_sin) = (
                        ph_cos * step_cos - ph_sin * step_sin,
                        ph_sin * step_cos + ph_cos * step_sin,
                    );
                }
                (to_f64(bin) * SR / len, re * re + im * im)
            })
            .collect()
    }

    /// Power in `band` (Hz) and in total, from a Hann-windowed periodogram.
    fn band_power(x: &[f32], band: &Range<f64>) -> (f64, f64) {
        let n = x.len().next_power_of_two();
        let fft = RealFftPlanner::<f64>::new().plan_fft_forward(n);
        let len = to_f64(x.len());
        let mut buf: Vec<f64> = x
            .iter()
            .enumerate()
            .map(|(i, &s)| f64::from(s) * (0.5 - 0.5 * (TAU * to_f64(i) / len).cos()))
            .collect();
        buf.resize(n, 0.0);
        let mut spec = fft.make_output_vec();
        fft.process(&mut buf, &mut spec)
            .expect("buffers sized by the plan");
        spec.iter()
            .enumerate()
            .fold((0.0, 0.0), |(inside, total), (k, c)| {
                let p = c.norm_sqr();
                let hz = to_f64(k) * SR / to_f64(n);
                (inside + if band.contains(&hz) { p } else { 0.0 }, total + p)
            })
    }

    /// Peak normalised autocorrelation over voice-like lags (150–400 Hz):
    /// near 1 for voicing, low for noise.
    fn periodicity(x: &[f32]) -> f64 {
        (60..=160)
            .map(|lag| {
                let (early, late) = (&x[..x.len() - lag], &x[lag..]);
                dot(early, late) / (dot(early, early) * dot(late, late)).sqrt().max(1e-30)
            })
            .fold(-1.0, f64::max)
    }

    /// F0 of `x[from..from + len]` near `expect` Hz: the best normalised
    /// autocorrelation lag within ±30 %, refined by a parabola.
    fn pitch(x: &[f32], from: usize, len: usize, expect: f64) -> f64 {
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let lags = (SR / (1.3 * expect)) as usize..=(SR / (0.7 * expect)) as usize;
        let score = |lag: usize| {
            let (a, b) = (&x[from..from + len], &x[from + lag..from + lag + len]);
            dot(a, b) / (dot(a, a) * dot(b, b)).sqrt()
        };
        let best = lags
            .clone()
            .max_by(|&a, &b| score(a).total_cmp(&score(b)))
            .expect("lags");
        let (l, c, r) = (score(best - 1), score(best), score(best + 1));
        let offset = 0.5 * (l - r) / (l - 2.0 * c + r);
        SR / (to_f64(best) + offset)
    }

    /// Formants of a steady stretch, lowest first: the peaks of its LPC
    /// envelope (autocorrelation method, order 24, pre-emphasis 0.9) on a
    /// 10 Hz grid from 150 Hz to 6 kHz.
    fn formants_of(signal: &[f32]) -> Vec<f64> {
        const ORDER: usize = 24;
        let len = to_f64(signal.len());
        let emphasised: Vec<f64> = signal
            .iter()
            .enumerate()
            .map(|(i, &s)| {
                let prev = if i > 0 { f64::from(signal[i - 1]) } else { 0.0 };
                (f64::from(s) - 0.9 * prev) * (0.5 - 0.5 * (TAU * to_f64(i) / len).cos())
            })
            .collect();
        let autocorr: Vec<f64> = (0..=ORDER)
            .map(|lag| {
                emphasised
                    .iter()
                    .zip(&emphasised[lag..])
                    .map(|(p, q)| p * q)
                    .sum()
            })
            .collect();
        // Levinson-Durbin: A(z) = 1 + a1·z⁻¹ + … + ap·z⁻ᵖ.
        let mut lpc = vec![0.0; ORDER + 1];
        lpc[0] = 1.0;
        let mut err = autocorr[0];
        for i in 1..=ORDER {
            let acc: f64 = (0..i).map(|j| lpc[j] * autocorr[i - j]).sum();
            let reflection = -acc / err;
            let prev = lpc.clone();
            for j in 1..i {
                lpc[j] = prev[j] + reflection * prev[i - j];
            }
            lpc[i] = reflection;
            err *= 1.0 - reflection * reflection;
        }
        let envelope = |hz: f64| {
            let omega = TAU * hz / SR;
            let (re, im) = lpc
                .iter()
                .enumerate()
                .fold((0.0, 0.0), |(re, im), (tap, &coef)| {
                    let phase = omega * to_f64(tap);
                    (re + coef * phase.cos(), im - coef * phase.sin())
                });
            1.0 / (re * re + im * im)
        };
        let grid: Vec<(f64, f64)> = (15..=600)
            .map(|i| {
                let hz = to_f64(i) * 10.0;
                (hz, envelope(hz))
            })
            .collect();
        grid.windows(3)
            .filter(|w| w[1].1 > w[0].1 && w[1].1 >= w[2].1)
            .map(|w| w[1].0)
            .collect()
    }

    /// F1 and F2 of `x`.
    fn f1_f2(x: &[f32]) -> (f64, f64) {
        let f = formants_of(x);
        assert!(f.len() >= 2, "no formants found: {f:?}");
        (f[0], f[1])
    }

    const FRAME: usize = 240; // 10 ms

    /// Start of the first 10 ms frame that is loud and clearly periodic.
    fn voicing_onset(x: &[f32]) -> usize {
        (0..x.len() - 2 * FRAME)
            .step_by(FRAME)
            .find(|&i| {
                rms32(&x[i..i + FRAME]) >= 0.5 * UNIT_RMS
                    && periodicity(&x[i..i + 2 * FRAME]) >= 0.7
            })
            .expect("the unit voices")
    }

    /// Energy of the aperiodic frames before voicing: burst and aspiration.
    fn noise_before_voicing(x: &[f32]) -> f64 {
        (0..voicing_onset(x))
            .step_by(FRAME)
            .filter(|&i| periodicity(&x[i..i + 2 * FRAME]) < 0.5)
            .map(|i| energy(&x[i..i + FRAME]))
            .sum()
    }

    /// Hillenbrand et al. (1995) adult female F1, F2 for the steady vowels.
    const STEADY: [(Nucleus, f64, f64); 8] = [
        (Nucleus::A, 669.0, 2_349.0),
        (Nucleus::E, 731.0, 2_058.0),
        (Nucleus::I, 483.0, 2_365.0),
        (Nucleus::O, 936.0, 1_551.0),
        (Nucleus::U, 753.0, 1_426.0),
        (Nucleus::LongE, 437.0, 2_761.0),
        (Nucleus::LongU, 459.0, 1_105.0),
        (Nucleus::Schwa, 600.0, 1_800.0),
    ];

    /// The steady vowel of `u` in a bank built at `tempo`.
    fn steady(x: &[f32], u: Unit, tempo: f64) -> &[f32] {
        let (from, to) = plan(u, tempo).reference;
        &x[samples(from)..samples(to).min(x.len())]
    }

    #[test]
    fn the_default_timbre_is_the_bank_chosen_by_ear() {
        // Fingerprint of the bank as it was auditioned, before timbres and
        // the noise low-pass existed. Built from `+ − × ÷ √` alone, so it
        // holds on every machine.
        const AUDITIONED: u64 = 0xc88a_46fc_2c0a_0f1e;
        let raw = FormantBank::build(Timbre::DEFAULT, None);
        assert_eq!(fingerprint(&raw), AUDITIONED);
        assert_eq!(
            fingerprint(bank()),
            fingerprint(&FormantBank::with_timbre(Timbre::DEFAULT))
        );

        // Noise is calibrated before it is low-passed, so below 8 kHz every
        // band carrying a unit's energy stays where it was. What moves at all
        // (0.63 dB at worst) is filtered noise summing with the voicing at a
        // different phase.
        for u in Unit::all() {
            let (a, b) = (raw.unit(u), bank().unit(u));
            assert_eq!(a.len(), b.len());
            for from in [0.0, 2_000.0, 4_000.0, 6_000.0] {
                let band = from..from + 2_000.0;
                let ((pa, total), (pb, _)) = (band_power(a, &band), band_power(b, &band));
                if pa >= 1e-3 * total {
                    let change = 10.0 * (pb / pa).log10();
                    assert!(change.abs() <= 1.0, "{u:?} {band:?} moved {change:.2} dB");
                }
            }
        }
    }

    #[test]
    fn every_unit_is_finite_bounded_and_clean() {
        for (v, bank) in voices() {
            for u in Unit::all() {
                let x = bank.unit(u);
                assert!(!x.is_empty(), "{} {u:?} is empty", v.name);
                assert!(
                    x.iter().all(|s| s.is_finite() && s.abs() <= 1.0),
                    "{} {u:?} out of range",
                    v.name
                );
                // Subnormals stall float arithmetic downstream on some CPUs.
                assert!(
                    x.iter().all(|&s| s == 0.0 || s.abs() >= f32::MIN_POSITIVE),
                    "{} {u:?} has subnormal samples",
                    v.name
                );
                let mean = x.iter().map(|&s| f64::from(s)).sum::<f64>() / to_f64(x.len());
                assert!(
                    mean.abs() < 0.01 * rms32(x),
                    "{} {u:?} has DC {mean}",
                    v.name
                );
            }
        }
    }

    #[test]
    fn every_unit_ends_in_silence() {
        // The renderer doesn't taper a unit it reads to the end, so a unit
        // that stops loud would click at every syllable.
        // The loudest ending measured is 0.0015 (Bright, bare K).
        for (v, bank) in voices() {
            for u in Unit::all() {
                let x = bank.unit(u);
                let last = x[x.len() - 6..].iter().fold(0.0_f32, |m, s| m.max(s.abs()));
                assert!(
                    f64::from(last) <= 0.05 * UNIT_RMS,
                    "{} {u:?} ends at {last:.4}",
                    v.name
                );
            }
        }
    }

    #[test]
    fn vowel_only_units_rise_from_silence() {
        for (v, bank) in voices() {
            let tempo = v.timbre.tempo;
            for n in Nucleus::ALL {
                let x = bank.unit(Unit::new(None, n));
                let first = rms32(&x[..samples(ms(2.0))]);
                let settled = rms32(&x[samples(ms(30.0 * tempo))..samples(ms(50.0 * tempo))]);
                assert!(
                    first <= 0.1 * settled,
                    "{} {n:?} starts at {:.1} dB re its vowel",
                    v.name,
                    20.0 * (first / settled).log10()
                );
            }
        }
    }

    #[test]
    fn each_units_steady_vowel_is_what_gets_levelled() {
        for (v, bank) in voices() {
            let mut limited = 0;
            for u in Unit::all() {
                let x = bank.unit(u);
                let level = rms32(steady(x, u, v.timbre.tempo));
                let peak = x.iter().fold(0.0_f64, |m, &s| m.max(f64::from(s).abs()));
                if peak >= PEAK_MAX - 1e-6 {
                    // Held down by the peak ceiling instead.
                    limited += 1;
                    assert!(level < UNIT_RMS, "{} {u:?} at {level}", v.name);
                } else {
                    assert!(
                        (20.0 * (level / UNIT_RMS).log10()).abs() < 0.05,
                        "{} {u:?} steady vowel at {level:.4} RMS",
                        v.name
                    );
                }
            }
            assert!(
                limited * 20 <= Unit::COUNT,
                "{} {limited} units peak-limited",
                v.name
            );
        }
    }

    #[test]
    fn units_share_a_common_level() {
        let levels: Vec<(Unit, f64)> = Unit::all()
            .map(|u| (u, 20.0 * rms32(bank().unit(u)).log10()))
            .collect();
        let mut sorted: Vec<f64> = levels.iter().map(|&(_, l)| l).collect();
        sorted.sort_by(f64::total_cmp);
        let common = sorted[sorted.len() / 2];
        assert!(
            (common - 20.0 * UNIT_RMS.log10()).abs() < 3.0,
            "common level {common:.1} dBFS"
        );
        for (u, level) in levels {
            assert!(
                (level - common).abs() <= 3.0,
                "{u:?} at {level:.1} dBFS, common level {common:.1}"
            );
        }
    }

    #[test]
    fn deterministic() {
        let again = FormantBank::new();
        for u in Unit::all() {
            let (a, b) = (again.unit(u), bank().unit(u));
            assert_eq!(a.len(), b.len(), "{u:?}");
            assert!(
                a.iter().zip(b).all(|(x, y)| x.to_bits() == y.to_bits()),
                "{u:?} differs between builds"
            );
            assert_eq!(again.lead(u), bank().lead(u));
        }
    }

    #[test]
    fn every_voice_is_band_limited() {
        // Content above 12 kHz / 1.35 ≈ 8.9 kHz folds back when the renderer
        // lifts pitch. Unfiltered, sibilants put up to 3.0 % (Bright), 0.77 %
        // (Mellow) and 0.24 % (Gruff) of their energy above 9 kHz; with the
        // noise low-pass no noisy unit tops 0.03 %. What remains is Bright's
        // voiced top, worst in a bare R at 0.21 %: its tract-scaled F7 at
        // 10.1 kHz, which folds no lower than 10.3 kHz even at ×1.35.
        for (v, bank) in voices() {
            for u in Unit::all() {
                let (high, total) = band_power(bank.unit(u), &(9_000.0..SR));
                assert!(
                    high <= 0.003 * total,
                    "{} {u:?} puts {:.3} % above 9 kHz",
                    v.name,
                    100.0 * high / total
                );
            }
        }
    }

    #[test]
    fn schwa_units_are_shorter_than_full_vowels() {
        let millis = |x: &[f32]| to_f64(x.len()) * 1_000.0 / SR;
        for onset in std::iter::once(None).chain(Onset::ALL.map(Some)) {
            let bare = millis(unit(onset, Nucleus::Schwa));
            assert!((90.0..=120.0).contains(&bare), "{onset:?} Schwa {bare} ms");
            for &n in &Nucleus::ALL[..Nucleus::ALL.len() - 1] {
                let full = millis(unit(onset, n));
                assert!((150.0..=200.0).contains(&full), "{onset:?} {n:?} {full} ms");
            }
        }
    }

    #[test]
    fn tempo_scales_units_and_their_leads() {
        let quick = Timbre {
            tempo: 0.8,
            ..Timbre::DEFAULT
        };
        for u in Unit::all().step_by(7) {
            let (a, lead_a) = synthesize(&plan(u, 1.0), seed(u), Timbre::DEFAULT, None);
            let (b, lead_b) = synthesize(&plan(u, 0.8), seed(u), quick, None);
            let want = 0.8 * to_f64(a.len());
            assert!(
                (to_f64(b.len()) - want).abs() <= 1.0,
                "{u:?} {} vs {want}",
                b.len()
            );
            // A lead is found on glottal pulses, which keep their own pitch
            // at any tempo: it moves to the nearest one.
            let want = 0.8 * to_f64(lead_a);
            assert!(
                (to_f64(lead_b) - want).abs() <= SR / Timbre::DEFAULT.f0,
                "{u:?} lead {lead_b} vs {want}"
            );
        }
    }

    #[test]
    fn leads_mark_where_the_vowel_gets_loud() {
        let lead = |o: Option<Onset>, n| to_f64(bank().lead(Unit::new(o, n))) * 1_000.0 / SR;
        for n in [Nucleus::A, Nucleus::LongE, Nucleus::O] {
            // Voicing rises over 20 ms: it is half up at about 10 ms.
            assert!(
                (6.0..=14.0).contains(&lead(None, n)),
                "{n:?} {}",
                lead(None, n)
            );
            for o in [Onset::K, Onset::S, Onset::Sh] {
                assert!((55.0..=80.0).contains(&lead(Some(o), n)), "{o:?}{n:?}");
            }
            for (voiceless, voiced) in [(Onset::P, Onset::B), (Onset::T, Onset::D)] {
                assert!(
                    lead(Some(voiceless), n) >= lead(Some(voiced), n) + 15.0,
                    "{voiceless:?}{n:?} vs {voiced:?}{n:?}"
                );
            }
        }
        // And the whole unit really is loud there, not just its voiced branch.
        for u in Unit::all() {
            let x = bank().unit(u);
            let at = bank().lead(u);
            let after = rms32(&x[at..(at + samples(ms(20.0))).min(x.len())]);
            assert!(after >= 0.4 * UNIT_RMS, "{u:?} is quiet after its lead");
        }
    }

    #[test]
    fn vowels_sit_at_each_voices_pitch() {
        for (v, bank) in voices() {
            // Early in the vowel, before the unit's gentle F0 fall has gone far.
            let x = bank.unit(Unit::new(None, Nucleus::A));
            let from = samples(ms(20.0 * v.timbre.tempo));
            let f0 = pitch(x, from, samples(ms(25.0)), v.timbre.f0);
            assert!(
                (f0 / v.timbre.f0 - 1.0).abs() <= 0.05,
                "{} F0 {f0:.0} Hz, voice at {:.0} Hz",
                v.name,
                v.timbre.f0
            );
        }
    }

    #[test]
    fn steady_vowels_have_their_published_formants_in_every_voice() {
        for (v, bank) in voices() {
            let k = v.timbre.tract;
            for (n, f1, f2) in STEADY {
                let u = Unit::new(None, n);
                let (m1, m2) = f1_f2(steady(bank.unit(u), u, v.timbre.tempo));
                assert!(
                    (m2 / (f2 * k) - 1.0).abs() <= 0.07,
                    "{} {n:?} F2 {m2:.0} Hz, want {:.0}",
                    v.name,
                    f2 * k
                );
                assert!(
                    (m1 / (f1 * k) - 1.0).abs() <= 0.2,
                    "{} {n:?} F1 {m1:.0} Hz, want {:.0}",
                    v.name,
                    f1 * k
                );
            }
        }
    }

    #[test]
    fn tract_scale_moves_f2_proportionally() {
        let (mellow, reference) = voices()
            .find(|(v, _)| v.id == "babble:mellow")
            .expect("Mellow has the default tract");
        for (v, bank) in voices() {
            for n in [Nucleus::A, Nucleus::O, Nucleus::LongE, Nucleus::LongU] {
                let u = Unit::new(None, n);
                let base = formants_of(steady(reference.unit(u), u, mellow.timbre.tempo));
                let moved = formants_of(steady(bank.unit(u), u, v.timbre.tempo));
                // F2 follows the vowel; F4 is a fixed upper resonance, so it
                // shows the whole tract scaled, not just the vowel targets.
                for (name, k) in [("F2", 1), ("F4", 3)] {
                    let ratio = moved[k] / base[k];
                    assert!(
                        (ratio / v.timbre.tract - 1.0).abs() <= 0.05,
                        "{} {n:?} {name} ×{ratio:.3} for tract ×{}",
                        v.name,
                        v.timbre.tract
                    );
                }
            }
        }
    }

    #[test]
    fn diphthongs_glide_and_steady_vowels_hold() {
        let ends = |n| {
            let x = unit(None, n);
            let at = |share: usize| x.len() * share / 100;
            (f1_f2(&x[at(12)..at(30)]), f1_f2(&x[at(68)..at(88)]))
        };
        // Each glide closes toward a high vowel, so F1 falls (measured ×0.61
        // to ×0.72). Steady vowels' F1 wanders ×0.93 to ×1.09 as voicing
        // decays under the breath noise.
        for n in [Nucleus::LongA, Nucleus::LongI, Nucleus::LongO] {
            let ((f1a, _), (f1b, _)) = ends(n);
            assert!(f1b <= 0.85 * f1a, "{n:?} F1 {f1a:.0} → {f1b:.0} Hz");
        }
        for n in [Nucleus::A, Nucleus::O, Nucleus::LongE] {
            let ((f1a, f2a), (f1b, f2b)) = ends(n);
            assert!(
                (f1b / f1a - 1.0).abs() <= 0.15,
                "{n:?} F1 {f1a:.0} → {f1b:.0} Hz"
            );
            assert!(
                (f2b / f2a - 1.0).abs() <= 0.05,
                "{n:?} F2 {f2a:.0} → {f2b:.0} Hz"
            );
        }
    }

    #[test]
    fn long_e_has_a_higher_f2_region_than_o() {
        // Energy centroid over 1–3.5 kHz of the steady vowel.
        let centroid = |x: &[f32]| {
            let (moment, total) = spectrum(slice_ms(x, 50.0, 130.0))
                .into_iter()
                .filter(|&(hz, _)| (1_000.0..3_500.0).contains(&hz))
                .fold((0.0, 0.0), |(m, t), (hz, p)| (m + hz * p, t + p));
            moment / total
        };
        let ee = centroid(unit(None, Nucleus::LongE));
        let o = centroid(unit(None, Nucleus::O));
        assert!(ee > o + 1_000.0, "LongE {ee:.0} Hz vs O {o:.0} Hz");
    }

    #[test]
    fn s_hisses_above_4khz_and_m_does_not() {
        let high_share = |x: &[f32]| {
            let spec = spectrum(slice_ms(x, 0.0, 80.0));
            let total: f64 = spec.iter().map(|&(_, p)| p).sum();
            let high: f64 = spec
                .iter()
                .filter(|&&(hz, _)| hz >= 4_000.0)
                .map(|&(_, p)| p)
                .sum();
            high / total
        };
        let s = high_share(unit(Some(Onset::S), Nucleus::A));
        let m = high_share(unit(Some(Onset::M), Nucleus::A));
        assert!(s > 0.5, "S puts {s:.3} of its energy above 4 kHz");
        assert!(m < 0.01, "M puts {m:.4} of its energy above 4 kHz");
    }

    /// Spectral centroid (1–8.5 kHz) of a voiceless fricative's hiss, pooled
    /// over every full vowel so one noise draw can't decide it.
    fn hiss_centroid(onset: Onset) -> f64 {
        let (moment, total) = Nucleus::ALL[..Nucleus::ALL.len() - 1]
            .iter()
            .flat_map(|&n| spectrum(slice_ms(unit(Some(onset), n), 15.0, 45.0)))
            .filter(|&(hz, _)| (1_000.0..8_500.0).contains(&hz))
            .fold((0.0, 0.0), |(m, t), (hz, p)| (m + hz * p, t + p));
        moment / total
    }

    #[test]
    fn fricatives_keep_their_own_noise_spectra() {
        // Jongman et al. (2000): s peaks high, sh an octave lower; f is
        // flat and bright, th flat and a little duller.
        let (s, sh) = (hiss_centroid(Onset::S), hiss_centroid(Onset::Sh));
        assert!(s >= sh + 1_500.0, "S {s:.0} Hz vs Sh {sh:.0} Hz");
        assert!((2_500.0..4_500.0).contains(&sh), "Sh {sh:.0} Hz");
        let (f, th) = (hiss_centroid(Onset::F), hiss_centroid(Onset::Th));
        assert!(f >= th + 150.0, "F {f:.0} Hz vs Th {th:.0} Hz");
    }

    #[test]
    fn nasals_hum_before_their_release() {
        for o in [Onset::M, Onset::N, Onset::Ng] {
            for (n, k) in [
                (Nucleus::A, 1.0),
                (Nucleus::LongE, 1.0),
                (Nucleus::Schwa, BARE_SCALE),
            ] {
                let hum = slice_ms(unit(Some(o), n), 12.0, 45.0 * k - 3.0);
                let level = rms32(hum);
                assert!(level >= 0.2 * UNIT_RMS, "{o:?}{n:?} murmur at {level:.4}");
                assert!(periodicity(hum) >= 0.8, "{o:?}{n:?} murmur isn't voiced");
                let (low, total) = band_power(hum, &(0.0..1_000.0));
                assert!(low >= 0.8 * total, "{o:?}{n:?} murmur isn't low");
            }
        }
    }

    #[test]
    fn voiced_stops_have_a_voice_bar_and_a_burst() {
        for n in [Nucleus::A, Nucleus::LongE, Nucleus::O] {
            for o in [Onset::B, Onset::D, Onset::G] {
                // Closure: 0–18 ms, bar up by 4 ms.
                let bar = slice_ms(unit(Some(o), n), 5.0, 17.0);
                let level = rms32(bar);
                assert!(level >= 0.05 * UNIT_RMS, "{o:?}{n:?} closure at {level:.4}");
                assert!(periodicity(bar) >= 0.7, "{o:?}{n:?} closure isn't voiced");
            }
            for o in [Onset::D, Onset::G] {
                // At the release, before voicing resumes: a brief noise burst,
                // well above the closure's low hum.
                let burst = slice_ms(unit(Some(o), n), 18.0, 24.0);
                let level = rms32(burst);
                let (high, total) = band_power(burst, &(1_000.0..SR));
                assert!(
                    level >= 0.1 * UNIT_RMS && high >= 0.5 * total,
                    "{o:?}{n:?} release at {level:.4}, {:.2} of it above 1 kHz",
                    high / total
                );
            }
        }
    }

    #[test]
    fn voiceless_stops_are_aspirated_and_voice_later() {
        let pairs = [
            (Onset::P, Onset::B),
            (Onset::T, Onset::D),
            (Onset::K, Onset::G),
        ];
        for (voiceless, voiced) in pairs {
            for n in [Nucleus::A, Nucleus::LongE, Nucleus::O, Nucleus::Schwa] {
                let (p, b) = (unit(Some(voiceless), n), unit(Some(voiced), n));
                let (vot_p, vot_b) = (voicing_onset(p), voicing_onset(b));
                assert!(
                    vot_p >= vot_b + 2 * FRAME,
                    "{voiceless:?}{n:?} voices at {vot_p}, {voiced:?}{n:?} at {vot_b}"
                );
                let (noise_p, noise_b) = (noise_before_voicing(p), noise_before_voicing(b));
                assert!(
                    noise_p > 4.0 * noise_b && noise_p > 0.003 * energy(p),
                    "{voiceless:?}{n:?} noise {noise_p:.2e} vs {voiced:?}{n:?} {noise_b:.2e}"
                );
            }
        }
    }

    #[test]
    fn finishing_removes_dc_and_rumble() {
        // Half a second of DC, a 20 Hz rumble and a 1 kHz tone, all at 0.3.
        let n = samples(0.5);
        let tone = |hz: f64, i: usize| (TAU * hz * to_f64(i) / SR).sin();
        let mix: Vec<f64> = (0..n)
            .map(|i| 0.3 + 0.3 * tone(20.0, i) + 0.3 * tone(1_000.0, i))
            .collect();
        let out = finish(mix, 0..n);
        // Over the last 200 ms (4 rumble cycles, 200 tone cycles), after the
        // blocker has settled.
        let tail = &out[n - samples(0.2)..];
        let amplitude = |hz: f64| {
            let (re, im) = tail
                .iter()
                .enumerate()
                .fold((0.0, 0.0), |(re, im), (i, &x)| {
                    let w = TAU * hz * to_f64(i) / SR;
                    (re + f64::from(x) * w.cos(), im + f64::from(x) * w.sin())
                });
            2.0 * (re * re + im * im).sqrt() / to_f64(tail.len())
        };
        let mean = tail.iter().map(|&x| f64::from(x)).sum::<f64>() / to_f64(tail.len());
        let kept = amplitude(1_000.0);
        assert!(mean.abs() <= 0.01 * kept, "DC {mean:.4} vs tone {kept:.4}");
        assert!(amplitude(20.0) <= 0.5 * kept, "rumble not cut");
    }

    #[test]
    fn plain_arithmetic_exp_cos_and_sin_track_std() {
        for i in 0..=2_000 {
            let x = -15.0 + 0.008 * f64::from(i);
            assert!((exp(x) - x.exp()).abs() <= 1e-12 * x.exp(), "exp({x})");
            let theta = PI * f64::from(i) / 2_000.0;
            assert!((cos(theta) - theta.cos()).abs() <= 1e-12, "cos({theta})");
            assert!((sin(theta) - theta.sin()).abs() <= 1e-7, "sin({theta})");
        }
    }

    #[test]
    fn the_noise_low_pass_is_flat_below_and_steep_above() {
        // Measured on a long noise, relative to the same noise unfiltered.
        let noise = Rng(7).noise(samples(2.0));
        let mut limited = noise.clone();
        LowPass::new(NOISE_CUTOFF_HZ).run(&mut limited);
        let to32 = |x: &[f64]| -> Vec<f32> {
            #[allow(clippy::cast_possible_truncation)]
            x.iter().map(|&s| s as f32).collect()
        };
        let (a, b) = (to32(&noise), to32(&limited));
        let gain_db = |band: Range<f64>| {
            let (p, _) = band_power(&a, &band);
            let (q, _) = band_power(&b, &band);
            10.0 * (q / p).log10()
        };
        assert!(
            gain_db(100.0..7_000.0).abs() < 0.3,
            "{:.2} dB",
            gain_db(100.0..7_000.0)
        );
        assert!(gain_db(9_000.0..9_500.0) < -12.0);
        assert!(gain_db(10_000.0..12_000.0) < -40.0);
    }

    #[test]
    fn resonators_have_the_gains_klatt_describes() {
        // |A / (1 − B·z⁻¹ − C·z⁻²)| on the unit circle.
        let gain = |r: &Resonator, hz: f64| {
            let w = TAU * hz / SR;
            let re = 1.0 - r.b * w.cos() - r.c * (2.0 * w).cos();
            let im = r.b * w.sin() + r.c * (2.0 * w).sin();
            r.a / (re * re + im * im).sqrt()
        };
        let klatt = Resonator::new(700.0, 90.0);
        assert!((gain(&klatt, 0.0) - 1.0).abs() < 1e-9);
        assert!(gain(&klatt, 700.0) > 5.0 * gain(&klatt, 2_000.0));

        let peak = Resonator::peak_normalised(3_600.0, 800.0);
        assert!((gain(&peak, 3_600.0) - 1.0).abs() < 1e-9);
        assert!(gain(&peak, 3_600.0) > gain(&peak, 3_000.0));
        assert!(gain(&peak, 3_600.0) > gain(&peak, 4_200.0));

        // The anti-resonator is the resonator's inverse: a notch, unity at DC.
        let mut notch = AntiResonator::new(1_000.0, 150.0);
        let mut ring = Resonator::new(1_000.0, 150.0);
        let pulse = (0..64).map(|i| if i == 0 { 1.0 } else { 0.0 });
        for (i, x) in pulse.enumerate() {
            let y = notch.tick(ring.tick(x));
            let want = if i == 0 { 1.0 } else { 0.0 };
            assert!((y - want).abs() < 1e-9, "sample {i}: {y}");
        }
    }

    #[test]
    fn curves_interpolate_hold_and_never_run_backwards() {
        let mut c = Curve::default();
        c.knot(0.1, 1.0).knot(0.2, 3.0).knot(0.15, 5.0);
        assert!((c.at(0.0) - 1.0).abs() < 1e-12);
        assert!((c.at(0.15) - 2.0).abs() < 1e-12);
        // The out-of-order knot became a step at 0.2.
        assert!((c.at(0.2) - 5.0).abs() < 1e-12);
        assert!((c.at(9.0) - 5.0).abs() < 1e-12);
        assert!(Curve::default().at(0.5).abs() < 1e-12);
    }
}
