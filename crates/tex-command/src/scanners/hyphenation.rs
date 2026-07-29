//! TeX82's two `hyph_data` scanners: `\patterns` and `\hyphenation`.
//!
//! Neither primitive absorbs a balanced text. TeX82 §473's `scan_toks` is the
//! only routine that sets `scanner_status:=absorbing`, and neither §934's
//! `new_hyph_exceptions` nor §960's `new_patterns` calls it: each reads its
//! compulsory opening brace through §403's `scan_left_brace` and then runs a
//! plain `get_x_token` loop (§935, §961) that classifies every delivered
//! command as a word character, a word boundary, or the group's closing brace.
//!
//! Routing these two primitives through `scan_toks` instead conflated three
//! separate things: it published an `absorbing` scanner-status episode TeX
//! never enters here, it tracked a brace depth TeX does not maintain (a `{`
//! inside the group is §936/§961's `othercases`, not a nested level), and it
//! gave `\the` the direct-splice treatment §473 reserves for token-list
//! collection instead of the ordinary expansion `get_x_token` performs.

use tex_state::meaning::{Meaning, UnexpandablePrimitive};
use tex_state::token::Catcode;

use crate::{CommandError, CommandProcessor};
#[cfg(any(test, feature = "observe"))]
use crate::{CommandObservation, DiagnosticRecord};

#[cfg(test)]
mod tests;

/// Which of TeX82's two `hyph_data` scans is running.
///
/// The two loops are deliberately distinct values rather than one boolean
/// flag over a shared scan: §935 accepts `char_given` and `char_num` as word
/// characters, and §961 does not, so a `\chardef` token or `\char` inside
/// `\patterns` is an error case there.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HyphenationDataKind {
    /// `\hyphenation` -- TeX82 §934's `new_hyph_exceptions`.
    Exceptions,
    /// `\patterns` -- TeX82 §960's `new_patterns`.
    Patterns,
}

/// The raw words one `\patterns`/`\hyphenation` group listed.
///
/// Each word is the exact sequence of characters §935/§961 accepted between
/// two word boundaries. `\lccode` normalization, §937's hyphen positions, and
/// §962's hyphen levels are applied by the executor when the words are
/// installed, so this scan stays free of the current `\language` and of the
/// pattern/exception table representations.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ScannedHyphenationData {
    pub words: Vec<Vec<char>>,
}

impl CommandProcessor<'_> {
    /// Runs TeX82 §934/§960's hyphenation-data scan to its closing brace.
    ///
    /// §960 runs `set_cur_lang` before `scan_left_brace` and §934 runs it
    /// after; both only read `\language`, and no command this loop delivers
    /// can assign it -- `get_x_token` expands, it never executes an
    /// assignment -- so the language is read once when the scanned words are
    /// installed rather than being captured here.
    pub fn scan_hyphenation_data(
        &mut self,
        kind: HyphenationDataKind,
    ) -> Result<ScannedHyphenationData, CommandError> {
        // §403: a left brace must follow `\patterns`/`\hyphenation`.
        self.scan_left_brace(true)?;
        let mut words: Vec<Vec<char>> = Vec::new();
        let mut current: Vec<char> = Vec::new();
        loop {
            let command = self.get_x_token()?.ok_or(CommandError::input_invariant())?;
            let character = match command.meaning() {
                // §935/§961's `letter,other_char`.
                Meaning::CharToken {
                    ch,
                    cat: Catcode::Letter | Catcode::Other,
                } => Some(ch),
                // §935's `char_given`, which §961 does not accept.
                Meaning::CharGiven(ch) if kind == HyphenationDataKind::Exceptions => Some(ch),
                // §935's `char_num`: `scan_char_num` selects the character and
                // the scan rejoins the `char_given` case.
                Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Char)
                    if kind == HyphenationDataKind::Exceptions =>
                {
                    Some(self.scan_character_number()?)
                }
                // §935/§961's `spacer,right_brace`: both end the current word,
                // and only the right brace ends the scan.
                Meaning::CharToken {
                    cat: Catcode::Space,
                    ..
                } => None,
                Meaning::CharToken {
                    cat: Catcode::EndGroup,
                    ..
                } => {
                    if !current.is_empty() {
                        words.push(current);
                    }
                    return Ok(ScannedHyphenationData { words });
                }
                // §§936/961: diagnose and resume with the offending command
                // consumed. In particular, this does not end or reset the
                // partially collected word.
                _ => {
                    self.report_hyphenation_scan_error(kind);
                    continue;
                }
            };
            match character {
                Some(character) => current.push(character),
                None => {
                    if !current.is_empty() {
                        words.push(std::mem::take(&mut current));
                    }
                }
            }
        }
    }

    fn report_hyphenation_scan_error(&mut self, kind: HyphenationDataKind) {
        let (message, help): (&str, &[&str]) = match kind {
            HyphenationDataKind::Exceptions => (
                "Improper \\hyphenation will be flushed",
                &[
                    "Hyphenation exceptions must contain only letters",
                    "and hyphens. But continue; I'll forgive and forget.",
                ],
            ),
            HyphenationDataKind::Patterns => ("Bad \\patterns", &["(See Appendix H.)"]),
        };
        #[cfg(any(test, feature = "observe"))]
        self.observe(CommandObservation::Diagnostic(DiagnosticRecord {
            severity: "error",
            diagnostic: match kind {
                HyphenationDataKind::Exceptions => "improper_hyphenation",
                HyphenationDataKind::Patterns => "bad_patterns",
            },
            arguments: Vec::new(),
        }));
        let mut report = self.state.print_err(message);
        report.help(help);
        report.error();
    }
}
