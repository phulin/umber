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

use std::collections::BTreeMap;

use tex_state::env::banks::IntParam;
use tex_state::hyphenation::PatternSpec;
use tex_state::meaning::{Meaning, ResolvedMeaning, UnexpandablePrimitive};
use tex_state::token::Catcode;

use crate::scanners::structured::{
    PendingStructuredScalarPhase, PendingStructuredScanner, PendingStructuredScannerPhase,
    StructuredScannerChildDestination,
};
use crate::{CommandError, CommandProcessor};

fn static_meaning<G>(meaning: ResolvedMeaning<G>) -> Meaning {
    match meaning {
        ResolvedMeaning::Static(meaning) => meaning,
        ResolvedMeaning::Macro { .. } => Meaning::Undefined,
    }
}

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
/// Each word is the sequence of characters §935 accepted between two word
/// boundaries, already `\lccode`-normalized: §935 tests `lc_code(cur_chr)=0`
/// and reports `Not a letter` inside the scanning loop, so the test has to run
/// where §82 still sees the offending character as current. §934 sets
/// `cur_lang` immediately after `scan_left_brace`, which is what makes the
/// language available this early. Hyphens are kept as `-`; §937's hyphen
/// positions and the exception table representation are still the executor's.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ScannedHyphenationData {
    pub words: Vec<Vec<char>>,
    /// TeX82 §962's already-normalized pattern representation. Pattern
    /// parsing belongs to the live scan because its errors call §82 before
    /// the next `get_x_token`; the executor only installs these values.
    pub patterns: Vec<PatternSpec>,
}

#[derive(Debug, Eq, PartialEq)]
pub(super) struct PendingHyphenationData {
    kind: HyphenationDataKind,
    words: Vec<Vec<char>>,
    current: Vec<char>,
    patterns: Vec<PatternSpec>,
    pattern_letters: Vec<char>,
    pattern_values: Vec<u8>,
    pattern_digit_sensed: bool,
    pattern_language: u8,
    pending_pattern_paths: BTreeMap<Vec<char>, bool>,
}

impl<G> CommandProcessor<'_, '_, G> {
    /// Reports whether TeX82 §960 may still add patterns to the uninitialized
    /// hyphenation trie.
    #[must_use]
    pub fn hyphenation_patterns_open(&mut self) -> bool {
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
        let pending = self.take_pending_structured_scanner()?;
        let mut progress = match pending {
            Some(PendingStructuredScanner { phase, mut child }) => {
                let PendingStructuredScannerPhase::Scalar(
                    PendingStructuredScalarPhase::Hyphenation(progress),
                ) = phase
                else {
                    if let Some(child) = child.take() {
                        self.abort_continuation(child.restore().0)?;
                    }
                    return Err(CommandError::input_invariant());
                };
                if progress.kind != kind {
                    if let Some(child) = child.take() {
                        self.abort_continuation(child.restore().0)?;
                    }
                    return Err(CommandError::input_invariant());
                }
                self.restore_structured_scanner_child(
                    &mut child,
                    StructuredScannerChildDestination::Scalar,
                )?;
                let result = self.scan_character_number_retained();
                let (ch, phase) = self.retain_structured_scalar_progress(
                    result,
                    PendingStructuredScalarPhase::Hyphenation(progress),
                )?;
                let PendingStructuredScalarPhase::Hyphenation(mut progress) = phase else {
                    unreachable!("hyphenation progress was returned unchanged")
                };
                if let Some(normalized) =
                    self.exception_word_character(progress.pattern_language, ch)?
                {
                    progress.current.push(normalized);
                }
                progress
            }
            None => {
                // §403: a left brace must follow `\patterns`/`\hyphenation`.
                self.scan_left_brace(true)?;
                PendingHyphenationData {
                    kind,
                    words: Vec::new(),
                    current: Vec::new(),
                    patterns: Vec::new(),
                    pattern_letters: Vec::new(),
                    pattern_values: vec![0],
                    pattern_digit_sensed: false,
                    pattern_language: u8::try_from(self.state.int_param(IntParam::LANGUAGE))
                        .unwrap_or(0),
                    pending_pattern_paths: BTreeMap::new(),
                }
            }
        };
        loop {
            let command = self.get_x_token()?.ok_or(CommandError::input_invariant())?;
            let character = match static_meaning(command.meaning()) {
                // §935/§961's `letter,other_char`.
                Meaning::CharToken {
                    ch,
                    cat: Catcode::Letter | Catcode::Other,
                } => {
                    if kind == HyphenationDataKind::Patterns {
                        let k = progress.pattern_letters.len();
                        if progress.pattern_digit_sensed || !ch.is_ascii_digit() {
                            let normalized = if ch == '.' {
                                '.'
                            } else {
                                let normalized = char::from_u32(self.state.lccode(ch));
                                if normalized.is_none_or(|normalized| normalized == '\0') {
                                    self.report_pattern_nonletter()?;
                                }
                                // TeX82 §962 leaves `cur_chr=0` after the
                                // Nonletter diagnostic and inserts it into
                                // the trie. This is the same edge character
                                // that a literal period selects above.
                                normalized.filter(|&mapped| mapped != '\0').unwrap_or('.')
                            };
                            // §962 changes `k` and `digit_sensed` only while
                            // `k<63`; characters beyond the bound are still
                            // classified (and can report Nonletter), but do
                            // not change the pattern state.
                            if k < 63 {
                                progress.pattern_letters.push(normalized);
                                progress.pattern_values.push(0);
                                progress.pattern_digit_sensed = false;
                            }
                        } else if k < 63 {
                            *progress
                                .pattern_values
                                .last_mut()
                                .expect("pattern has hyf[0]") =
                                ch.to_digit(10).expect("ASCII digit has a value") as u8;
                            progress.pattern_digit_sensed = true;
                        }
                    }
                    if kind == HyphenationDataKind::Exceptions {
                        match self.exception_word_character(progress.pattern_language, ch)? {
                            Some(normalized) => Some(normalized),
                            // §935 ignores the character it just read.
                            None => continue,
                        }
                    } else {
                        Some(ch)
                    }
                }
                // §935's `char_given`, which §961 does not accept.
                Meaning::CharGiven(ch) if kind == HyphenationDataKind::Exceptions => {
                    match self.exception_word_character(progress.pattern_language, ch)? {
                        Some(normalized) => Some(normalized),
                        None => continue,
                    }
                }
                // §935's `char_num`: `scan_char_num` selects the character and
                // the scan rejoins the `char_given` case.
                Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Char)
                    if kind == HyphenationDataKind::Exceptions =>
                {
                    let result = self.scan_character_number_retained();
                    let (ch, phase) = self.retain_structured_scalar_progress(
                        result,
                        PendingStructuredScalarPhase::Hyphenation(progress),
                    )?;
                    let PendingStructuredScalarPhase::Hyphenation(returned) = phase else {
                        unreachable!("hyphenation progress was returned unchanged")
                    };
                    progress = returned;
                    match self.exception_word_character(progress.pattern_language, ch)? {
                        Some(normalized) => Some(normalized),
                        None => continue,
                    }
                }
                // §935/§961's `spacer,right_brace`: both end the current word,
                // and only the right brace ends the scan.
                Meaning::CharToken {
                    cat: Catcode::Space,
                    ..
                } => {
                    if !progress.pattern_letters.is_empty() {
                        let pattern = PatternSpec {
                            letters: std::mem::take(&mut progress.pattern_letters),
                            values: std::mem::replace(&mut progress.pattern_values, vec![0]),
                        };
                        self.report_duplicate_pattern_if_needed(
                            progress.pattern_language,
                            &mut progress.pending_pattern_paths,
                            &pattern,
                        )?;
                        progress.patterns.push(pattern);
                    }
                    progress.pattern_digit_sensed = false;
                    None
                }
                Meaning::CharToken {
                    cat: Catcode::EndGroup,
                    ..
                } => {
                    if !progress.current.is_empty() {
                        progress.words.push(progress.current);
                    }
                    if !progress.pattern_letters.is_empty() {
                        let pattern = PatternSpec {
                            letters: progress.pattern_letters,
                            values: progress.pattern_values,
                        };
                        self.report_duplicate_pattern_if_needed(
                            progress.pattern_language,
                            &mut progress.pending_pattern_paths,
                            &pattern,
                        )?;
                        progress.patterns.push(pattern);
                    }
                    return Ok(ScannedHyphenationData {
                        words: progress.words,
                        patterns: progress.patterns,
                    });
                }
                // §§936/961: diagnose and resume with the offending command
                // consumed. In particular, this does not end or reset the
                // partially collected word.
                _ => {
                    self.report_hyphenation_scan_error(kind)?;
                    continue;
                }
            };
            match character {
                Some(character) => progress.current.push(character),
                None => {
                    if !progress.current.is_empty() {
                        progress.words.push(std::mem::take(&mut progress.current));
                    }
                }
            }
        }
    }

    /// TeX82 §935's per-character test for `\hyphenation`.
    ///
    /// A hyphen is §937's position marker and bypasses the test; anything
    /// whose `lc_code` is zero is diagnosed and ignored; everything else
    /// enters the word lowercased. pdfTeX §934 forces `hyph_index=0` while
    /// `trie_not_ready`, so saved `\savinghyphcodes` values take precedence
    /// only after the pattern trie has been initialized.
    fn exception_word_character(
        &mut self,
        language: u8,
        ch: char,
    ) -> Result<Option<char>, CommandError> {
        if ch == '-' {
            return Ok(Some('-'));
        }
        let normalized = if self.state.hyphenation_patterns_open() {
            char::from_u32(self.state.lccode(ch)).filter(|&mapped| mapped != '\0')
        } else {
            match self.state.saved_hyphenation_code(language, ch) {
                Some(saved) => saved,
                None => char::from_u32(self.state.lccode(ch)).filter(|&mapped| mapped != '\0'),
            }
        };
        if normalized.is_none() {
            let context = self.command.output_open_context(&self.state);
            let mut report = self.state.print_err("Not a letter");
            report.help(&[
                "Letters in \\hyphenation words must have \\lccode>0.",
                "Proceed; I'll ignore the character I just read.",
            ]);
            report.context(context);
            report.error().jump_out()?;
        }
        Ok(normalized)
    }

    fn report_pattern_nonletter(&mut self) -> Result<(), CommandError> {
        let context = self.command.output_open_context(&self.state);
        let mut report = self.state.print_err("Nonletter");
        report.help(&["(See Appendix H.)"]);
        report.context(context);
        report.error().jump_out()?;
        Ok(())
    }

    fn report_duplicate_pattern_if_needed(
        &mut self,
        language: u8,
        pending: &mut BTreeMap<Vec<char>, bool>,
        pattern: &PatternSpec,
    ) -> Result<(), CommandError> {
        // §963 tests the current terminal `trie_o`, then replaces it with
        // §965's newly computed `v` even after diagnosing a duplicate. Keep
        // the pending view in that same order: its value is the current
        // replacement, not whether any historical occurrence had an op.
        let current = pending.entry(pattern.letters.clone()).or_insert_with(|| {
            self.state
                .contains_hyphenation_pattern_for_language(language, &pattern.letters)
        });
        let duplicate = *current;
        *current = pattern.has_trie_operation();
        if duplicate {
            let context = self.command.output_open_context(&self.state);
            let mut report = self.state.print_err("Duplicate pattern");
            report.help(&["(See Appendix H.)"]);
            report.context(context);
            report.error().jump_out()?;
        }
        Ok(())
    }

    fn report_hyphenation_scan_error(
        &mut self,
        kind: HyphenationDataKind,
    ) -> Result<(), CommandError> {
        // §936 and §961 both reach §82 while the offending command is still
        // current and the source cursor is immediately after it, so both get a
        // context. `CommandState`, not `Universe`, owns that live input stack.
        let context = self.command.output_open_context(&self.state);
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
        report.context(context);
        report.error().jump_out()?;
        Ok(())
    }
}
