//! Text → a flat token stream for the babble engine.
//!
//! Critter speech is voiced one written character at a time, so this stage
//! keeps every letter and throws away almost everything else. Punctuation
//! survives only as timing (pauses) and intonation (`?` / `!`). Letters from
//! scripts we have no spelling rules for are kept as [`Token::Glyph`] so they
//! still babble — deterministically — instead of going silent.

/// One element of the babble token stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Token {
    /// A Latin letter, lowercased and stripped of accents (`b'a'..=b'z'`).
    Letter(u8),
    /// A digit value, `0..=9`.
    Digit(u8),
    /// A letter from a script without spelling rules here (CJK, Cyrillic,
    /// kana, …), carried as its code point.
    Glyph(u32),
    /// The gap between words.
    WordGap,
    /// A pause from punctuation. Adjacent pauses are merged later by the
    /// scheduler, longest wins.
    Pause(Pause),
    /// `?` — the preceding sound rises.
    Question,
    /// `!` — the preceding sound is emphasised.
    Exclaim,
}

/// Pause lengths, shortest to longest.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Pause {
    /// `,` `;` `:` and dashes.
    Comma,
    /// `.` `?` `!`.
    Sentence,
    /// `…` or `...`.
    Ellipsis,
}

/// Split `text` into babble tokens.
#[must_use]
pub fn tokenize(text: &str) -> Vec<Token> {
    let chars: Vec<char> = text.chars().collect();
    let mut out = Vec::with_capacity(chars.len());
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '.' && chars.get(i + 1) == Some(&'.') && chars.get(i + 2) == Some(&'.') {
            out.push(Token::Pause(Pause::Ellipsis));
            while chars.get(i) == Some(&'.') {
                i += 1;
            }
            continue;
        }
        i += 1;
        match c {
            'a'..='z' | 'A'..='Z' => out.push(Token::Letter(c.to_ascii_lowercase() as u8)),
            '0'..='9' => out.push(Token::Digit(c as u8 - b'0')),
            // Contractions read as one word: "I'll" babbles like "ill".
            '\'' | '\u{2019}' | '\u{02BC}' => {}
            '?' => {
                out.push(Token::Question);
                out.push(Token::Pause(Pause::Sentence));
            }
            '!' => {
                out.push(Token::Exclaim);
                out.push(Token::Pause(Pause::Sentence));
            }
            '.' => out.push(Token::Pause(Pause::Sentence)),
            '\u{2026}' => out.push(Token::Pause(Pause::Ellipsis)),
            ',' | ';' | ':' | '\u{2014}' | '\u{2013}' => out.push(Token::Pause(Pause::Comma)),
            c if c.is_whitespace() || c == '-' => out.push(Token::WordGap),
            c => push_other(&mut out, c),
        }
    }
    out
}

/// Accented Latin folds to its base letters; other alphabetic characters
/// become glyphs; everything else (emoji, symbols) separates words silently.
fn push_other(out: &mut Vec<Token>, c: char) {
    let mut folded = false;
    for lower in c.to_lowercase() {
        if let Some(base) = fold_latin(lower) {
            out.extend(base.bytes().map(Token::Letter));
            folded = true;
        }
    }
    if folded {
        return;
    }
    if c.is_alphabetic() {
        out.push(Token::Glyph(c as u32));
    } else {
        out.push(Token::WordGap);
    }
}

/// Base letters for a lowercase accented Latin character. Hand-written so
/// the core stays free of a Unicode-normalisation dependency; it covers
/// Latin-1 Supplement and Latin Extended-A, which is what real text uses.
fn fold_latin(c: char) -> Option<&'static str> {
    Some(match c {
        'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' | 'ā' | 'ă' | 'ą' => "a",
        'ç' | 'ć' | 'ĉ' | 'ċ' | 'č' => "c",
        'ď' | 'đ' => "d",
        'è' | 'é' | 'ê' | 'ë' | 'ē' | 'ĕ' | 'ė' | 'ę' | 'ě' => "e",
        'ĝ' | 'ğ' | 'ġ' | 'ģ' => "g",
        'ĥ' | 'ħ' => "h",
        'ì' | 'í' | 'î' | 'ï' | 'ĩ' | 'ī' | 'ĭ' | 'į' | 'ı' => "i",
        'ĵ' => "j",
        'ķ' => "k",
        'ĺ' | 'ļ' | 'ľ' | 'ŀ' | 'ł' => "l",
        'ñ' | 'ń' | 'ņ' | 'ň' => "n",
        'ò' | 'ó' | 'ô' | 'õ' | 'ö' | 'ø' | 'ō' | 'ŏ' | 'ő' => "o",
        'ŕ' | 'ŗ' | 'ř' => "r",
        'ś' | 'ŝ' | 'ş' | 'š' => "s",
        'ţ' | 'ť' | 'ŧ' => "t",
        'ù' | 'ú' | 'û' | 'ü' | 'ũ' | 'ū' | 'ŭ' | 'ů' | 'ű' | 'ų' => "u",
        'ŵ' => "w",
        'ý' | 'ÿ' | 'ŷ' => "y",
        'ź' | 'ż' | 'ž' => "z",
        'ß' => "ss",
        'æ' => "ae",
        'œ' => "oe",
        'þ' | 'ð' => "th",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use Token::{Digit, Exclaim, Glyph, Letter, Question, WordGap};

    fn letters(s: &str) -> Vec<Token> {
        s.bytes().map(Letter).collect()
    }

    #[test]
    fn lowercases_letters_and_keeps_digits() {
        let mut want = letters("hi");
        want.push(WordGap);
        want.push(Digit(4));
        want.push(Digit(2));
        assert_eq!(tokenize("Hi 42"), want);
    }

    #[test]
    fn contractions_stay_one_word() {
        assert_eq!(tokenize("I'll"), letters("ill"));
        assert_eq!(tokenize("I\u{2019}ll"), letters("ill"));
    }

    #[test]
    fn folds_accents_to_base_letters() {
        assert_eq!(tokenize("Café"), letters("cafe"));
        assert_eq!(tokenize("straße"), letters("strasse"));
        assert_eq!(tokenize("ÆON"), letters("aeon"));
    }

    #[test]
    fn punctuation_becomes_timing_and_intonation() {
        let t = tokenize("ok? yes! so, no. hm\u{2026}");
        assert!(t.contains(&Question));
        assert!(t.contains(&Exclaim));
        assert!(t.contains(&Token::Pause(Pause::Comma)));
        assert!(t.contains(&Token::Pause(Pause::Sentence)));
        assert!(t.contains(&Token::Pause(Pause::Ellipsis)));
    }

    #[test]
    fn three_dots_are_one_ellipsis_not_three_sentences() {
        let t = tokenize("wait...");
        assert_eq!(t.iter().filter(|x| matches!(x, Token::Pause(_))).count(), 1);
        assert_eq!(t.last(), Some(&Token::Pause(Pause::Ellipsis)));
    }

    #[test]
    fn other_scripts_babble_as_glyphs() {
        assert_eq!(
            tokenize("日本"),
            vec![Glyph('日' as u32), Glyph('本' as u32)]
        );
    }

    #[test]
    fn emoji_and_symbols_are_silent_separators() {
        let t = tokenize("hi\u{1F642}there");
        assert!(!t.iter().any(|x| matches!(x, Glyph(_))));
        assert!(t.contains(&WordGap));
    }
}
