//! Text → IPA phonemes via the bundled **espeak-ng** CLI, invoked as an
//! arm's-length subprocess.
//!
//! espeak-ng is **GPL-3.0**. We never link it: we spawn its command-line
//! binary and read its stdout. Under the FSF's "mere aggregation" guidance
//! this keeps `DivoraVoice`'s own code MIT — espeak-ng stays a separate GPL
//! component (its binary + `espeak-ng-data` ship alongside the app and are
//! disclosed in the README). All linking-free.
//!
//! The argument construction and IPA cleanup are pure and unit-tested; the
//! actual subprocess call is exercised on the desktop once the binary is
//! staged (see `MANUAL_TESTS.md`).

use std::path::{Path, PathBuf};
use std::process::Command;

/// Locates the bundled espeak-ng binary (and its data dir) and turns text
/// into the continuous IPA string Kokoro's tokenizer indexes.
#[derive(Debug, Clone)]
pub struct Phonemizer {
    /// Path to the `espeak-ng` executable.
    bin: PathBuf,
    /// Path to the `espeak-ng-data` directory, if bundled alongside (passed
    /// via `--path`). espeak-ng can also find its data relative to the binary.
    data: Option<PathBuf>,
}

/// Failures from the phonemizer. [`PhonemizeError::NotInstalled`] is the
/// graceful "espeak-ng isn't staged yet" path the synthesis gate relies on.
#[derive(Debug, thiserror::Error)]
pub enum PhonemizeError {
    #[error("espeak-ng binary not found at {0}")]
    NotInstalled(PathBuf),
    #[error("could not run espeak-ng: {0}")]
    Spawn(#[from] std::io::Error),
    #[error("espeak-ng exited with status {code}: {stderr}")]
    Exited { code: i32, stderr: String },
}

impl Phonemizer {
    /// Build a phonemizer for a bundled espeak-ng layout under `dir`:
    /// `dir/espeak-ng(.exe)` plus an optional `dir/espeak-ng-data`.
    #[must_use]
    pub fn from_dir(dir: &Path) -> Self {
        let bin = dir.join(espeak_bin_name());
        let data_dir = dir.join("espeak-ng-data");
        let data = data_dir.is_dir().then_some(data_dir);
        Self { bin, data }
    }

    /// Whether the espeak-ng binary is physically present, so callers can gate
    /// synthesis behind a clear "voice not installed" rather than spawning a
    /// missing process.
    #[must_use]
    pub fn is_available(&self) -> bool {
        self.bin.exists()
    }

    /// Phonemize `text` in `lang` (e.g. `"en-us"`) to a continuous IPA string
    /// with stress marks — the form Kokoro's vocab maps to token ids.
    ///
    /// Returns [`PhonemizeError::NotInstalled`] without spawning anything when
    /// the binary is absent (so it can never hang waiting on a missing tool).
    pub fn phonemize(&self, text: &str, lang: &str) -> Result<String, PhonemizeError> {
        if !self.is_available() {
            return Err(PhonemizeError::NotInstalled(self.bin.clone()));
        }
        let mut cmd = Command::new(&self.bin);
        cmd.args(espeak_args(lang, text));
        if let Some(data) = &self.data {
            cmd.arg("--path").arg(data);
        }
        let output = cmd.output()?;
        if !output.status.success() {
            return Err(PhonemizeError::Exited {
                code: output.status.code().unwrap_or(-1),
                stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
            });
        }
        Ok(clean_ipa(&String::from_utf8_lossy(&output.stdout)))
    }
}

/// The espeak-ng arguments for quiet IPA phonemization, with the input text
/// passed positionally last. `-q` suppresses audio; `--ipa` emits IPA with
/// stress marks (matching the `phonemizer` espeak backend Kokoro trained on).
fn espeak_args(lang: &str, text: &str) -> Vec<String> {
    vec![
        "-q".to_string(),
        "--ipa".to_string(),
        "-v".to_string(),
        lang.to_string(),
        // Separator so espeak never parses the text as a flag; it ignores it.
        "--".to_string(),
        text.to_string(),
    ]
}

fn espeak_bin_name() -> &'static str {
    if cfg!(target_os = "windows") {
        "espeak-ng.exe"
    } else {
        "espeak-ng"
    }
}

/// Normalize espeak-ng's IPA stdout into a single clean phoneme line: drop
/// language-switch markers like `(en)`, strip the tie bar espeak inserts in
/// affricates (Kokoro stores affricate components separately), and collapse
/// whitespace/newlines to single spaces.
fn clean_ipa(raw: &str) -> String {
    let mut stripped = String::with_capacity(raw.len());
    let mut in_paren = false;
    for ch in raw.chars() {
        match ch {
            '(' => in_paren = true,
            ')' => in_paren = false,
            _ if in_paren => {}
            '\u{0361}' | '\u{200d}' => {} // combining tie bar / zero-width joiner
            '\n' | '\r' | '\t' => stripped.push(' '),
            _ => stripped.push(ch),
        }
    }
    // Collapse runs of spaces and trim.
    let mut out = String::with_capacity(stripped.len());
    let mut prev_space = false;
    for ch in stripped.trim().chars() {
        if ch == ' ' {
            if !prev_space {
                out.push(' ');
            }
            prev_space = true;
        } else {
            out.push(ch);
            prev_space = false;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn args_are_quiet_ipa_with_lang_and_text_last() {
        let args = espeak_args("en-us", "hello");
        assert_eq!(args, vec!["-q", "--ipa", "-v", "en-us", "--", "hello"]);
    }

    #[test]
    fn missing_binary_reports_not_installed_without_spawning() {
        let p = Phonemizer::from_dir(Path::new("Z:/divora/does-not-exist"));
        assert!(!p.is_available());
        match p.phonemize("hello", "en-us") {
            Err(PhonemizeError::NotInstalled(_)) => {}
            other => panic!("expected NotInstalled, got {other:?}"),
        }
    }

    #[test]
    fn clean_ipa_strips_markers_ties_and_collapses_space() {
        let raw = "  h\u{0361}ə(en)l\u{200d}ˈəʊ \n world  ";
        // tie bar + ZWJ removed, "(en)" removed, whitespace collapsed/trimmed.
        assert_eq!(clean_ipa(raw), "həlˈəʊ world");
    }

    #[test]
    fn clean_ipa_empty_is_empty() {
        assert_eq!(clean_ipa("   \n  "), "");
    }
}
