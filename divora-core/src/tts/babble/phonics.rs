//! Spelling → syllable units.
//!
//! Critter speech voices one unit per written character, and what makes it
//! read as "almost words" rather than noise is that each unit follows the
//! spelling: `sh` hisses, `ee` is bright, a final `e` goes quiet. These rules
//! are ordinary English phonics, written for this project — deliberately
//! loose, because the result is babble, not pronunciation.
//!
//! Every unit carries a vowel. A consonant with no vowel after it (the `s`
//! and `t` in "strength") becomes a short *bare* syllable on a neutral vowel,
//! which is what gives the per-letter patter its rhythm.
//!
//! The invariant the tests lean on: the letters consumed by a word's
//! syllables always sum to the word's length, so timing never drifts from
//! the text.

use super::text::Token;

/// Consonant onsets. Phonetic values in the doc comments are approximate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Onset {
    B,
    /// `ch` as in *chip*.
    Ch,
    D,
    F,
    /// Hard `g` as in *go*.
    G,
    H,
    /// `j` as in *jam* (also soft `g`).
    J,
    K,
    L,
    M,
    N,
    /// `ng` as in *sing*.
    Ng,
    P,
    R,
    S,
    /// `sh` as in *ship*.
    Sh,
    T,
    /// `th` as in *thin*.
    Th,
    V,
    W,
    /// Consonant `y` as in *yes*.
    Y,
    Z,
}

impl Onset {
    /// Every onset, in index order.
    pub const ALL: [Self; 22] = [
        Self::B,
        Self::Ch,
        Self::D,
        Self::F,
        Self::G,
        Self::H,
        Self::J,
        Self::K,
        Self::L,
        Self::M,
        Self::N,
        Self::Ng,
        Self::P,
        Self::R,
        Self::S,
        Self::Sh,
        Self::T,
        Self::Th,
        Self::V,
        Self::W,
        Self::Y,
        Self::Z,
    ];
}

/// Vowel nuclei.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Nucleus {
    /// *cat* /æ/.
    A,
    /// *bed* /ɛ/.
    E,
    /// *sit* /ɪ/.
    I,
    /// *hot* /ɑ/.
    O,
    /// *cup* /ʌ/.
    U,
    /// *day* /eɪ/.
    LongA,
    /// *see* /i/.
    LongE,
    /// *my* /aɪ/.
    LongI,
    /// *go* /oʊ/.
    LongO,
    /// *too* /u/.
    LongU,
    /// The neutral vowel /ə/ — bare consonants and unstressed `er`.
    Schwa,
}

impl Nucleus {
    /// Every nucleus, in index order. [`Nucleus::Schwa`] is last.
    pub const ALL: [Self; 11] = [
        Self::A,
        Self::E,
        Self::I,
        Self::O,
        Self::U,
        Self::LongA,
        Self::LongE,
        Self::LongI,
        Self::LongO,
        Self::LongU,
        Self::Schwa,
    ];
}

/// One voiced unit: an optional consonant onset plus a vowel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Unit {
    pub onset: Option<Onset>,
    pub nucleus: Nucleus,
}

impl Unit {
    /// Size of the unit inventory: (no onset + 22 onsets) × 11 nuclei.
    pub const COUNT: usize = (Onset::ALL.len() + 1) * Nucleus::ALL.len();

    #[must_use]
    pub const fn new(onset: Option<Onset>, nucleus: Nucleus) -> Self {
        Self { onset, nucleus }
    }

    /// Dense index in `0..Unit::COUNT`, stable for a given inventory — unit
    /// sources store their audio in this order.
    #[must_use]
    pub fn index(self) -> usize {
        let o = self.onset.map_or(0, |on| {
            1 + Onset::ALL.iter().position(|&x| x == on).unwrap_or(0)
        });
        let n = Nucleus::ALL
            .iter()
            .position(|&x| x == self.nucleus)
            .unwrap_or(0);
        o * Nucleus::ALL.len() + n
    }

    /// Inverse of [`Unit::index`]. `None` when out of range.
    #[must_use]
    pub fn from_index(i: usize) -> Option<Self> {
        if i >= Self::COUNT {
            return None;
        }
        let o = i / Nucleus::ALL.len();
        let n = i % Nucleus::ALL.len();
        let onset = if o == 0 {
            None
        } else {
            Some(Onset::ALL[o - 1])
        };
        Some(Self::new(onset, Nucleus::ALL[n]))
    }

    /// Every unit, in index order.
    pub fn all() -> impl Iterator<Item = Self> {
        (0..Self::COUNT).filter_map(Self::from_index)
    }
}

/// A unit as it sits in the text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Syllable {
    pub unit: Unit,
    /// Written characters this syllable accounts for (always ≥ 1). The
    /// scheduler gives each character one slot of time.
    pub letters: u8,
    /// A consonant voiced on the neutral vowel because no vowel followed it.
    pub bare: bool,
}

/// What the scheduler walks: syllables with the timing and intonation marks
/// that sit between them.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Item {
    Syllable(Syllable),
    Gap(Token),
}

/// Turn a token stream into syllables, keeping gaps and marks in place.
#[must_use]
pub fn syllabify(tokens: &[Token]) -> Vec<Item> {
    let mut out = Vec::with_capacity(tokens.len());
    let mut word: Vec<u8> = Vec::new();
    for &t in tokens {
        match t {
            Token::Letter(b) => word.push(b),
            Token::Digit(d) => {
                flush_word(&mut word, &mut out);
                out.push(Item::Syllable(digit_syllable(d)));
            }
            Token::Glyph(cp) => {
                flush_word(&mut word, &mut out);
                out.push(Item::Syllable(glyph_syllable(cp)));
            }
            other => {
                flush_word(&mut word, &mut out);
                out.push(Item::Gap(other));
            }
        }
    }
    flush_word(&mut word, &mut out);
    out
}

fn flush_word(word: &mut Vec<u8>, out: &mut Vec<Item>) {
    if word.is_empty() {
        return;
    }
    out.extend(word_syllables(word).into_iter().map(Item::Syllable));
    word.clear();
}

/// The first syllable of each digit's English name.
fn digit_syllable(d: u8) -> Syllable {
    use Nucleus::{LongA, LongE, LongI, LongO, LongU, E, I, U};
    let unit = match d {
        0 => Unit::new(Some(Onset::Z), LongE),
        1 => Unit::new(Some(Onset::W), U),
        2 => Unit::new(Some(Onset::T), LongU),
        3 => Unit::new(Some(Onset::Th), LongE),
        4 => Unit::new(Some(Onset::F), LongO),
        5 => Unit::new(Some(Onset::F), LongI),
        6 => Unit::new(Some(Onset::S), I),
        7 => Unit::new(Some(Onset::S), E),
        8 => Unit::new(None, LongA),
        _ => Unit::new(Some(Onset::N), LongI),
    };
    Syllable {
        unit,
        letters: 1,
        bare: false,
    }
}

/// A letter from a script without spelling rules: hash to a full syllable
/// (never schwa) so each character babbles distinctly and repeatably.
fn glyph_syllable(cp: u32) -> Syllable {
    let h = cp.wrapping_mul(0x9E37_79B1);
    let voiced = Nucleus::ALL.len() - 1; // every nucleus except Schwa
    let n = (h >> 8) as usize % voiced;
    let o = (h >> 20) as usize % (Onset::ALL.len() + 1);
    let onset = if o == 0 {
        None
    } else {
        Some(Onset::ALL[o - 1])
    };
    Syllable {
        unit: Unit::new(onset, Nucleus::ALL[n]),
        letters: 1,
        bare: false,
    }
}

#[derive(Debug, Clone, Copy)]
enum Phone {
    C(Onset, u8),
    V(Nucleus, u8),
    Silent(u8),
}

const fn is_vowel(b: u8) -> bool {
    matches!(b, b'a' | b'e' | b'i' | b'o' | b'u')
}

/// Syllables for one lowercase ASCII word.
#[must_use]
pub fn word_syllables(w: &[u8]) -> Vec<Syllable> {
    let phones = phones(w);
    let mut out: Vec<Syllable> = Vec::with_capacity(phones.len());
    let mut pending: Option<(Onset, u8)> = None;
    let mut carry: u8 = 0; // silent letters seen before any syllable exists
    for p in phones {
        match p {
            Phone::C(onset, n) => {
                if let Some((prev, pn)) = pending.take() {
                    out.push(bare(prev, pn + std::mem::take(&mut carry)));
                }
                pending = Some((onset, n));
            }
            Phone::V(nucleus, n) => {
                let (onset, on) = pending.take().map_or((None, 0), |(o, k)| (Some(o), k));
                out.push(Syllable {
                    unit: Unit::new(onset, nucleus),
                    letters: on + n + std::mem::take(&mut carry),
                    bare: false,
                });
            }
            Phone::Silent(n) => {
                if let Some((_, pn)) = pending.as_mut() {
                    *pn += n;
                } else if let Some(last) = out.last_mut() {
                    last.letters += n;
                } else {
                    carry += n;
                }
            }
        }
    }
    if let Some((onset, n)) = pending {
        out.push(bare(onset, n + carry));
    } else if carry > 0 {
        // A word of only silent letters can't happen with these rules, but
        // never drop letters: voice them as one neutral syllable.
        out.push(Syllable {
            unit: Unit::new(None, Nucleus::Schwa),
            letters: carry,
            bare: true,
        });
    }
    out
}

const fn bare(onset: Onset, letters: u8) -> Syllable {
    Syllable {
        unit: Unit::new(Some(onset), Nucleus::Schwa),
        letters,
        bare: true,
    }
}

/// Multi-letter vowels, longest first.
const VOWEL_TEAMS: &[(&[u8], Nucleus)] = &[
    (b"eigh", Nucleus::LongA),
    (b"igh", Nucleus::LongI),
    (b"ee", Nucleus::LongE),
    (b"ea", Nucleus::LongE),
    (b"ei", Nucleus::LongE),
    (b"ie", Nucleus::LongE),
    (b"oo", Nucleus::LongU),
    (b"ou", Nucleus::LongU),
    (b"ue", Nucleus::LongU),
    (b"ew", Nucleus::LongU),
    (b"ai", Nucleus::LongA),
    (b"ay", Nucleus::LongA),
    (b"ey", Nucleus::LongE),
    (b"oa", Nucleus::LongO),
    (b"ow", Nucleus::LongO),
    (b"oi", Nucleus::LongO),
    (b"oy", Nucleus::LongO),
    (b"au", Nucleus::O),
    (b"aw", Nucleus::O),
];

/// Consonant spellings that only behave this way at the start of a word.
const WORD_START: &[(&[u8], Onset)] = &[
    (b"kn", Onset::N),
    (b"gn", Onset::N),
    (b"wr", Onset::R),
    (b"ps", Onset::S),
];

/// Multi-letter consonants, longest first.
const CONSONANT_TEAMS: &[(&[u8], Onset)] = &[
    (b"tch", Onset::Ch),
    (b"sh", Onset::Sh),
    (b"ch", Onset::Ch),
    (b"th", Onset::Th),
    (b"ph", Onset::F),
    (b"wh", Onset::W),
    (b"ck", Onset::K),
    (b"ng", Onset::Ng),
    (b"qu", Onset::K),
];

/// Letters in a spelling pattern. Patterns are at most four letters.
fn width(pat: &[u8]) -> u8 {
    u8::try_from(pat.len()).unwrap_or(u8::MAX)
}

fn phones(w: &[u8]) -> Vec<Phone> {
    let mut out = Vec::with_capacity(w.len());
    let mut i = 0;
    while i < w.len() {
        let b = w[i];
        let y_is_vowel = b == b'y' && !w.get(i + 1).copied().is_some_and(is_vowel);
        let phone = if is_vowel(b) || y_is_vowel {
            vowel_phone(w, i)
        } else {
            consonant_phone(w, i)
        };
        let consumed = match phone {
            Phone::C(_, n) | Phone::V(_, n) | Phone::Silent(n) => usize::from(n),
        };
        out.push(phone);
        i += consumed.max(1);
    }
    out
}

/// The vowel sound starting at `w[i]` (a vowel letter, or a vowel `y`).
fn vowel_phone(w: &[u8], i: usize) -> Phone {
    let len = w.len();
    let b = w[i];
    let next = w.get(i + 1).copied();
    let vowels_before = w[..i].iter().any(|&c| is_vowel(c) || c == b'y');

    if let Some(&(pat, n)) = VOWEL_TEAMS.iter().find(|(pat, _)| w[i..].starts_with(pat)) {
        return Phone::V(n, width(pat));
    }
    // r-controlled vowels, when the `r` closes the syllable.
    if next == Some(b'r') && !w.get(i + 2).copied().is_some_and(is_vowel) {
        match b {
            b'a' => return Phone::V(Nucleus::O, 2),
            b'o' => return Phone::V(Nucleus::LongO, 2),
            b'e' | b'i' | b'u' => return Phone::V(Nucleus::Schwa, 2),
            _ => {}
        }
    }
    if b == b'y' {
        // Final `y` after a consonant: "my" / "fly" vs "happy".
        let n = match (i + 1 == len && i > 0, len <= 3) {
            (true, true) => Nucleus::LongI,
            (true, false) => Nucleus::LongE,
            (false, _) => Nucleus::I,
        };
        return Phone::V(n, 1);
    }
    // Final silent `e` when the word already has a vowel ("make").
    if b == b'e' && i + 1 == len && vowels_before {
        return Phone::Silent(1);
    }
    let magic_e = i + 3 == len
        && w[len - 1] == b'e'
        && next.is_some_and(|c| !is_vowel(c) && c != b'y')
        && b != b'e';
    let open_single = i + 1 == len && !vowels_before;
    Phone::V(vowel(b, magic_e || open_single, len), 1)
}

/// The consonant sound starting at `w[i]`.
fn consonant_phone(w: &[u8], i: usize) -> Phone {
    if i == 0 {
        if let Some(&(pat, o)) = WORD_START.iter().find(|(pat, _)| w.starts_with(pat)) {
            return Phone::C(o, width(pat));
        }
    }
    if let Some(&(pat, o)) = CONSONANT_TEAMS
        .iter()
        .find(|(pat, _)| w[i..].starts_with(pat))
    {
        return Phone::C(o, width(pat));
    }
    if w[i..].starts_with(b"gh") {
        return Phone::C(if i == 0 { Onset::G } else { Onset::F }, 2);
    }
    let b = w[i];
    let next = w.get(i + 1).copied();
    let onset = consonant(b, next, i, w.len());
    // Doubled consonants are one sound ("ll", "ss", "tt").
    Phone::C(onset, if next == Some(b) { 2 } else { 1 })
}

const fn vowel(b: u8, long: bool, word_len: usize) -> Nucleus {
    match (b, long) {
        // A lone "a" is the article: unstressed.
        (b'a', true) if word_len == 1 => Nucleus::Schwa,
        (b'a', false) => Nucleus::A,
        (b'a', true) => Nucleus::LongA,
        (b'e', false) => Nucleus::E,
        (b'e', true) => Nucleus::LongE,
        (b'i', false) => Nucleus::I,
        (b'i', true) => Nucleus::LongI,
        (b'o', false) => Nucleus::O,
        (b'o', true) => Nucleus::LongO,
        (b'u', false) => Nucleus::U,
        _ => Nucleus::LongU,
    }
}

fn consonant(b: u8, next: Option<u8>, i: usize, len: usize) -> Onset {
    let soft = next.is_some_and(|n| matches!(n, b'e' | b'i' | b'y'));
    match b {
        b'b' => Onset::B,
        b'c' if soft => Onset::S,
        // Hard c, k, q, and a mid-word x all land on k.
        b'c' | b'k' | b'q' | b'x' if !(b == b'x' && i == 0) => Onset::K,
        b'd' => Onset::D,
        b'f' => Onset::F,
        // Soft `g` only in the reliable spot: a final "ge" ("page").
        b'g' if next == Some(b'e') && i + 2 == len => Onset::J,
        b'g' => Onset::G,
        b'h' => Onset::H,
        b'j' => Onset::J,
        b'l' => Onset::L,
        b'm' => Onset::M,
        b'n' => Onset::N,
        b'p' => Onset::P,
        b'r' => Onset::R,
        b's' => Onset::S,
        b't' => Onset::T,
        b'v' => Onset::V,
        b'w' => Onset::W,
        b'y' => Onset::Y,
        // `z`, and a word-initial `x` ("xylophone").
        _ => Onset::Z,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use Nucleus::{LongA, LongE, LongI, LongO, LongU, Schwa, A, E, I, O, U};
    use Onset::{Ch, Ng, Sh, Th, K, L, M, N, P, R, S, T, Y};

    fn units(word: &str) -> Vec<(Option<Onset>, Nucleus, u8)> {
        word_syllables(word.as_bytes())
            .iter()
            .map(|s| (s.unit.onset, s.unit.nucleus, s.letters))
            .collect()
    }

    #[test]
    fn unit_index_round_trips_across_the_whole_inventory() {
        assert_eq!(Unit::COUNT, 253);
        for i in 0..Unit::COUNT {
            let u = Unit::from_index(i).expect("in range");
            assert_eq!(u.index(), i);
        }
        assert_eq!(Unit::from_index(Unit::COUNT), None);
        assert_eq!(Unit::all().count(), Unit::COUNT);
    }

    #[test]
    fn simple_consonant_vowel_words() {
        assert_eq!(units("cat"), vec![(Some(K), A, 2), (Some(T), Schwa, 1)]);
        assert_eq!(units("pup"), vec![(Some(P), U, 2), (Some(P), Schwa, 1)]);
    }

    #[test]
    fn digraph_consonants_are_one_onset() {
        assert_eq!(units("ship"), vec![(Some(Sh), I, 3), (Some(P), Schwa, 1)]);
        assert_eq!(units("chin")[0], (Some(Ch), I, 3));
        assert_eq!(units("thin")[0], (Some(Th), I, 3));
        assert_eq!(units("sing"), vec![(Some(S), I, 2), (Some(Ng), Schwa, 2)]);
    }

    #[test]
    fn vowel_teams_are_long() {
        assert_eq!(units("see"), vec![(Some(S), LongE, 3)]);
        assert_eq!(units("moon")[0], (Some(M), LongU, 3));
        assert_eq!(units("rain")[0], (Some(R), LongA, 3));
        assert_eq!(units("light")[0], (Some(L), LongI, 4));
        assert_eq!(units("you"), vec![(Some(Y), LongU, 3)]);
    }

    #[test]
    fn magic_e_lengthens_and_goes_silent() {
        // "make": the a turns long, the final e rides on the k.
        assert_eq!(
            units("make"),
            vec![(Some(M), LongA, 2), (Some(K), Schwa, 2)]
        );
        assert_eq!(units("note")[0], (Some(N), LongO, 2));
    }

    #[test]
    fn open_one_vowel_words_are_long() {
        assert_eq!(units("go"), vec![(Some(Onset::G), LongO, 2)]);
        assert_eq!(units("me"), vec![(Some(M), LongE, 2)]);
        assert_eq!(units("i"), vec![(None, LongI, 1)]);
        assert_eq!(units("a"), vec![(None, Schwa, 1)]);
    }

    #[test]
    fn y_as_consonant_and_vowel() {
        assert_eq!(units("yes")[0], (Some(Y), E, 2));
        assert_eq!(units("my"), vec![(Some(M), LongI, 2)]);
        assert_eq!(units("happy").last().copied(), Some((Some(P), LongE, 3)));
    }

    #[test]
    fn soft_c_and_r_controlled_vowels() {
        assert_eq!(units("city")[0], (Some(S), I, 2));
        assert_eq!(units("car"), vec![(Some(K), O, 3)]);
        assert_eq!(units("her"), vec![(Some(Onset::H), Schwa, 3)]);
    }

    #[test]
    fn consonant_clusters_become_bare_syllables() {
        let s = word_syllables(b"strength");
        assert_eq!(s[0].unit, Unit::new(Some(S), Schwa));
        assert!(s[0].bare);
        assert_eq!(s[2].unit, Unit::new(Some(R), E));
        assert!(!s[2].bare);
    }

    #[test]
    fn every_letter_is_accounted_for() {
        // The timing invariant: syllable letters sum to the word length, so
        // the babble can never drift out of step with the text.
        let words = [
            "strength",
            "rhythm",
            "queue",
            "eighty",
            "knight",
            "psychology",
            "throughout",
            "a",
            "i",
            "bookkeeper",
            "xylophone",
            "aaaaaa",
            "zzz",
            "yyy",
            "eee",
            "sooooo",
            "e",
            "the",
            "judge",
            "whatchamacallit",
        ];
        for w in words {
            let total: u32 = word_syllables(w.as_bytes())
                .iter()
                .map(|s| u32::from(s.letters))
                .sum();
            assert_eq!(total as usize, w.len(), "letters lost in {w:?}");
        }
    }

    #[test]
    fn every_letter_is_accounted_for_on_arbitrary_strings() {
        // Deterministic sweep over many letter strings, not just real words.
        let mut state = 0x2545_F491_4F6C_DD1Du64;
        for _ in 0..5_000 {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            let len = 1 + (state % 12) as usize;
            let w: Vec<u8> = (0..len)
                .map(|k| b'a' + ((state >> (k * 5)) % 26) as u8)
                .collect();
            let total: usize = word_syllables(&w).iter().map(|s| s.letters as usize).sum();
            assert_eq!(
                total,
                w.len(),
                "letters lost in {:?}",
                String::from_utf8_lossy(&w)
            );
            assert!(word_syllables(&w).iter().all(|s| s.letters >= 1));
        }
    }

    #[test]
    fn digits_and_glyphs_voice_one_full_syllable_each() {
        use super::super::text::tokenize;
        let items = syllabify(&tokenize("42日"));
        let syl: Vec<_> = items
            .iter()
            .filter_map(|i| match i {
                Item::Syllable(s) => Some(*s),
                Item::Gap(_) => None,
            })
            .collect();
        assert_eq!(syl.len(), 3);
        assert_eq!(syl[0].unit, Unit::new(Some(Onset::F), LongO));
        assert!(syl
            .iter()
            .all(|s| s.letters == 1 && s.unit.nucleus != Schwa));
        // Deterministic: the same glyph always babbles the same way.
        assert_eq!(glyph_syllable('日' as u32), glyph_syllable('日' as u32));
    }

    #[test]
    fn short_vowels_by_default() {
        assert_eq!(units("bed")[0].1, E);
        assert_eq!(units("sit")[0].1, I);
        assert_eq!(units("hot")[0].1, O);
        assert_eq!(units("lop")[0].1, O);
        assert_eq!(units("sun")[0].1, U);
    }
}
