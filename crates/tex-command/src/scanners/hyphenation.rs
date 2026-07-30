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

use tex_state::env::banks::IntParam;
use tex_state::hyphenation::PatternSpec;
use tex_state::meaning::{Meaning, UnexpandablePrimitive};
use tex_state::token::Catcode;

use crate::{CommandError, CommandProcessor};

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
    /// TeX82 §962's already-normalized pattern representation. Pattern
    /// parsing belongs to the live scan because its errors call §82 before
    /// the next `get_x_token`; the executor only installs these values.
    pub patterns: Vec<PatternSpec>,
}

impl CommandProcessor<'_> {
    /// Reports whether TeX82 §960 may still add patterns to the uninitialized
    /// hyphenation trie.
    #[must_use]
    pub fn hyphenation_patterns_open(&self) -> bool {
        self.state.hyphenation_patterns_open()
    }

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
        let mut patterns: Vec<PatternSpec> = Vec::new();
        let mut pattern_letters: Vec<char> = Vec::new();
        let mut pattern_values = vec![0];
        let mut pattern_digit_sensed = false;
        let pattern_language = u8::try_from(self.state.int_param(IntParam::LANGUAGE)).unwrap_or(0);
        loop {
            let command = self.get_x_token()?.ok_or(CommandError::input_invariant())?;
            let character = match command.meaning() {
                // §935/§961's `letter,other_char`.
                Meaning::CharToken {
                    ch,
                    cat: Catcode::Letter | Catcode::Other,
                } => {
                    if kind == HyphenationDataKind::Patterns {
                        let k = pattern_letters.len();
                        if pattern_digit_sensed || !ch.is_ascii_digit() {
                            let normalized = if ch == '.' {
                                '.'
                            } else {
                                let normalized = char::from_u32(self.state.lccode(ch));
                                if normalized.is_none_or(|normalized| normalized == '\0') {
                                    self.report_pattern_nonletter();
                                }
                                normalized.unwrap_or('\0')
                            };
                            // §962 changes `k` and `digit_sensed` only while
                            // `k<63`; characters beyond the bound are still
                            // classified (and can report Nonletter), but do
                            // not change the pattern state.
                            if k < 63 {
                                pattern_letters.push(normalized);
                                pattern_values.push(0);
                                pattern_digit_sensed = false;
                            }
                        } else if k < 63 {
                            *pattern_values.last_mut().expect("pattern has hyf[0]") =
                                ch.to_digit(10).expect("ASCII digit has a value") as u8;
                            pattern_digit_sensed = true;
                        }
                    }
                    Some(ch)
                }
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
                } => {
                    if !pattern_letters.is_empty() {
                        let pattern = PatternSpec {
                            letters: std::mem::take(&mut pattern_letters),
                            values: std::mem::replace(&mut pattern_values, vec![0]),
                        };
                        self.report_duplicate_pattern_if_needed(
                            pattern_language,
                            &patterns,
                            &pattern,
                        );
                        patterns.push(pattern);
                    }
                    pattern_digit_sensed = false;
                    None
                }
                Meaning::CharToken {
                    cat: Catcode::EndGroup,
                    ..
                } => {
                    if !current.is_empty() {
                        words.push(current);
                    }
                    if !pattern_letters.is_empty() {
                        let pattern = PatternSpec {
                            letters: pattern_letters,
                            values: pattern_values,
                        };
                        self.report_duplicate_pattern_if_needed(
                            pattern_language,
                            &patterns,
                            &pattern,
                        );
                        patterns.push(pattern);
                    }
                    return Ok(ScannedHyphenationData { words, patterns });
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

    fn report_pattern_nonletter(&mut self) {
        let context = self.command.output_open_context(&self.state);
        let mut report = self.state.print_err("Nonletter");
        report.help(&["(See Appendix H.)"]);
        report.context(context);
        report.error();
    }

    fn report_duplicate_pattern_if_needed(
        &mut self,
        language: u8,
        pending: &[PatternSpec],
        pattern: &PatternSpec,
    ) {
        let duplicate = pending
            .iter()
            .any(|prior| prior.letters == pattern.letters && prior.has_trie_operation())
            || self
                .state
                .contains_hyphenation_pattern_for_language(language, &pattern.letters);
        if duplicate {
            let context = self.command.output_open_context(&self.state);
            let mut report = self.state.print_err("Duplicate pattern");
            report.help(&["(See Appendix H.)"]);
            report.context(context);
            report.error();
        }
    }

    fn report_hyphenation_scan_error(&mut self, kind: HyphenationDataKind) {
        let context = (kind == HyphenationDataKind::Patterns)
            .then(|| self.command.output_open_context(&self.state));
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
        // §§936/961 print and call §82's `error`, but the schema-v1 TeX82
        // instrumentation records no diagnostic at either site. Publishing
        // one here inserts an event before the loop resumes `get_x_token`.
        let mut report = self.state.print_err(message);
        report.help(help);
        if let Some(context) = context {
            // §961 reaches §82 while the offending command is still current
            // and the source cursor is immediately after it. `CommandState`,
            // not `Universe`, owns that live input stack.
            report.context(context);
        }
        report.error();
    }
}
