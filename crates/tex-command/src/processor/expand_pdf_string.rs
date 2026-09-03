//! pdfTeX string, hexadecimal, and regular-expression expansion primitives.

use std::fmt::Write as _;

use posix_regex::{PosixRegexBuilder, compile::Error as PosixRegexError};
use tex_state::token::{Catcode, OriginId, Token, TracedTokenWord};

use crate::observation::{CommandObservation, TokenListRecord};
use crate::{CommandError, CurrentCommand};

use super::CommandProcessor;

#[derive(Debug, Eq, PartialEq)]
pub(crate) struct PendingPdfStringCompare<G> {
    phase: PdfStringComparePhase,
    child: Option<crate::execution_scratch::ChildContinuation<G, PdfStringCompareDestination>>,
}

impl<G> PendingPdfStringCompare<G> {
    pub(crate) fn take_child(&mut self) -> Option<crate::execution_scratch::ScannerFrameKey<G>> {
        self.child.take().map(|child| child.restore().0)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PdfStringComparePhase {
    Left,
    Right { left: crate::AttemptTokenListId },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PdfStringCompareDestination {
    Scan,
}

fn escape_pdf_literal_string(text: &str) -> String {
    fn append_byte(output: &mut String, byte: u8) {
        match byte {
            b'!'..=b'~' => {
                if matches!(byte, b'(' | b')' | b'\\') {
                    output.push('\\');
                }
                output.push(char::from(byte));
            }
            _ => write!(output, "\\{byte:03o}").expect("writing to String cannot fail"),
        }
    }

    let mut escaped = String::with_capacity(text.len());
    for character in text.chars() {
        if let Ok(byte) = u8::try_from(u32::from(character)) {
            append_byte(&mut escaped, byte);
        } else {
            let mut encoded = [0; 4];
            for byte in character.encode_utf8(&mut encoded).bytes() {
                append_byte(&mut escaped, byte);
            }
        }
    }
    escaped
}

/// pdfTeX's `utils.c` `escapehex` projection for a PDF hexadecimal string.
fn escape_pdf_hex(bytes: &[u8]) -> String {
    let mut escaped = String::with_capacity(bytes.len().saturating_mul(2));
    for byte in bytes {
        write!(escaped, "{byte:02X}").expect("writing to String cannot fail");
    }
    escaped
}

/// pdfTeX's `utils.c` `unescapehex` projection for a PDF hexadecimal string.
fn unescape_pdf_hex(bytes: &[u8]) -> String {
    let mut unescaped = String::with_capacity(bytes.len().div_ceil(2));
    let mut high_nibble = None;
    for byte in bytes {
        let nibble = match byte {
            b'0'..=b'9' => byte - b'0',
            b'A'..=b'F' => byte - b'A' + 10,
            b'a'..=b'f' => byte - b'a' + 10,
            _ => continue,
        };
        if let Some(high) = high_nibble.take() {
            unescaped.push(char::from(high | nibble));
        } else {
            high_nibble = Some(nibble << 4);
        }
    }
    if let Some(high) = high_nibble {
        unescaped.push(char::from(high));
    }
    unescaped
}

impl<G> CommandProcessor<'_, '_, G> {
    /// Starts a balanced expanded-text collector for one of pdfTeX's string
    /// projections. The body is consumed by the canonical expanded-delivery
    /// loop; only the finished attempt buffer crosses into the byte renderer.
    pub(super) fn begin_pdf_string_continuation(
        &mut self,
        opener: OriginId,
        kind: crate::expansion_work::control::SynchronousExpandedKind,
    ) -> Result<(), CommandError> {
        let attempt_opening = self.command.attempt.arena().mark();
        let writer = self
            .command
            .attempt
            .arena_mut()
            .allocate_token_buffer()
            .map_err(crate::scan_toks::attempt_command_error)?;
        if let Err(error) = self.command.scratch.push_pdf_string_control(
            opener,
            kind,
            attempt_opening,
            writer,
            None,
        ) {
            self.command
                .attempt
                .arena_mut()
                .truncate(attempt_opening)
                .map_err(crate::scan_toks::attempt_command_error)?;
            return Err(crate::scan_toks::scratch_command_error(error));
        }
        Ok(())
    }

    /// Projects a completed hot pdf-string collector and inserts the result
    /// as category-12/space character tokens. This is the same semantic
    /// boundary as the legacy `expand_pdf_escape_*` methods, but no scanner
    /// call or second delivery loop is entered while the body is active.
    pub(super) fn finish_pdf_string_continuation(
        &mut self,
        kind: crate::expansion_work::control::SynchronousExpandedKind,
        list: crate::AttemptTokenListId,
        opener: OriginId,
    ) -> Result<(), CommandError> {
        let text = self.attempt_token_list_string_text(list)?;
        let rendered = match kind {
            crate::expansion_work::control::SynchronousExpandedKind::PdfEscapeString => {
                escape_pdf_literal_string(&text)
            }
            crate::expansion_work::control::SynchronousExpandedKind::PdfEscapeHex => {
                escape_pdf_hex(&self.attempt_token_list_bytes(list)?)
            }
            crate::expansion_work::control::SynchronousExpandedKind::PdfUnescapeHex => {
                unescape_pdf_hex(&self.attempt_token_list_bytes(list)?)
            }
            _ => return Err(CommandError::input_invariant()),
        };
        self.push_rendered_text(&rendered, opener);
        Ok(())
    }

    pub(super) fn finish_pdf_string_compare_continuation(
        &mut self,
        left: crate::AttemptTokenListId,
        right: crate::AttemptTokenListId,
        opener: OriginId,
    ) -> Result<(), CommandError> {
        let left = self.attempt_token_list_string_text(left)?;
        let right = self.attempt_token_list_string_text(right)?;
        let value = match left.as_bytes().cmp(right.as_bytes()) {
            std::cmp::Ordering::Less => -1,
            std::cmp::Ordering::Equal => 0,
            std::cmp::Ordering::Greater => 1,
        };
        self.push_rendered_text(&value.to_string(), opener);
        Ok(())
    }

    /// pdftex.web §§495 and 1535's `compare_strings` conversion.
    ///
    /// Both operands are independently collected by `scan_pdf_ext_toks`,
    /// rendered through `tokens_to_string`, and compared lexicographically as
    /// pdfTeX string-pool bytes. Canonical pdfTeX input is byte-valued; UTF-8
    /// preserves that ordering for Umber's extended scalar domain as well.
    pub(super) fn expand_string_compare(
        &mut self,
        opener: CurrentCommand<G>,
    ) -> Result<(), CommandError> {
        let pending = if self
            .scanner_resume
            .as_ref()
            .is_some_and(crate::ScannerFrameKey::is_pdf_string_compare)
        {
            let key = self
                .scanner_resume
                .take()
                .expect("matched pdf string-compare frame");
            Some(
                self.command
                    .scratch
                    .take_pdf_string_compare_frame(key)
                    .map_err(crate::scan_toks::scratch_command_error)?,
            )
        } else {
            None
        };
        let phase = pending
            .as_ref()
            .map_or(PdfStringComparePhase::Left, |pending| pending.phase);
        if let Some(mut pending) = pending
            && let Some(child) = pending.child.take()
        {
            let (key, destination) = child.restore();
            if destination != PdfStringCompareDestination::Scan {
                return Err(CommandError::input_invariant());
            }
            self.scanner_resume = Some(key);
        }
        let left = match phase {
            PdfStringComparePhase::Left => {
                match self.scan_toks(crate::scan_toks::ScanToksMode::General { expanded: true }) {
                    Ok(scanned) => scanned.replacement_text,
                    Err(error) => {
                        if error.is_resource_suspension() {
                            let key = self
                                .command
                                .scratch
                                .store_pdf_string_compare_frame(PendingPdfStringCompare {
                                    phase: PdfStringComparePhase::Left,
                                    child: crate::execution_scratch::ChildContinuation::capture(
                                        &mut self.scanner_resume,
                                        PdfStringCompareDestination::Scan,
                                    ),
                                })
                                .map_err(crate::scan_toks::scratch_command_error)?;
                            self.scanner_resume = Some(key);
                        }
                        return Err(error);
                    }
                }
            }
            PdfStringComparePhase::Right { left } => left,
        };
        let right = match self.scan_toks(crate::scan_toks::ScanToksMode::General { expanded: true })
        {
            Ok(scanned) => scanned,
            Err(error) => {
                if error.is_resource_suspension() {
                    let key = self
                        .command
                        .scratch
                        .store_pdf_string_compare_frame(PendingPdfStringCompare {
                            phase: PdfStringComparePhase::Right { left },
                            child: crate::execution_scratch::ChildContinuation::capture(
                                &mut self.scanner_resume,
                                PdfStringCompareDestination::Scan,
                            ),
                        })
                        .map_err(crate::scan_toks::scratch_command_error)?;
                    self.scanner_resume = Some(key);
                }
                return Err(error);
            }
        };
        let left = self.attempt_token_list_string_text(left)?;
        let right = self.attempt_token_list_string_text(right.replacement_text)?;
        let value = match left.as_bytes().cmp(right.as_bytes()) {
            std::cmp::Ordering::Less => -1,
            std::cmp::Ordering::Equal => 0,
            std::cmp::Ordering::Greater => 1,
        };
        self.push_rendered_text(&value.to_string(), opener.origin());
        Ok(())
    }

    /// pdftex.web §495's `pdf_escape_string_code` conversion.
    ///
    /// The operand is one expanded general-text token list. `tokens_to_string`
    /// first projects that list to pdfTeX's byte string, then `escapestring`
    /// writes a PDF literal-string body: parentheses and backslashes gain an
    /// escape prefix, while bytes outside `!` through `~` use three octal
    /// digits. The result reenters expansion as category-12 characters.
    pub(super) fn expand_pdf_escape_string(
        &mut self,
        opener: CurrentCommand<G>,
    ) -> Result<(), CommandError> {
        let scanned = self.scan_toks(crate::scan_toks::ScanToksMode::General { expanded: true })?;
        let text = self.attempt_token_list_string_text(scanned.replacement_text)?;
        let escaped = escape_pdf_literal_string(&text);
        observe!(
            self,
            CommandObservation::TokenList(TokenListRecord {
                transition: "complete",
                purpose: "pdf_escape_string",
                tokens: escaped
                    .chars()
                    .map(|ch| {
                        self.observed_token(TracedTokenWord::pack(
                            Token::Char {
                                ch,
                                cat: Catcode::Other,
                            },
                            OriginId::UNKNOWN,
                        ))
                    })
                    .collect(),
            }),
        );
        self.push_rendered_text(&escaped, opener.origin());
        Ok(())
    }

    /// pdftex.web §§494 and 496--497's `pdf_escape_hex_code` conversion.
    ///
    /// The operand is one expanded general-text token list. `tokens_to_string`
    /// projects that list to pdfTeX bytes, then `escapehex` writes exactly two
    /// uppercase hexadecimal digits for every byte, without angle brackets.
    /// TeX82 §464's `str_toks` returns those digits as category-12 characters.
    pub(super) fn expand_pdf_escape_hex(
        &mut self,
        opener: CurrentCommand<G>,
    ) -> Result<(), CommandError> {
        let scanned = self.scan_toks(crate::scan_toks::ScanToksMode::General { expanded: true })?;
        let bytes = self.attempt_token_list_bytes(scanned.replacement_text)?;
        let escaped = escape_pdf_hex(&bytes);
        observe!(
            self,
            CommandObservation::TokenList(TokenListRecord {
                transition: "complete",
                purpose: "pdf_escape_hex",
                tokens: escaped
                    .chars()
                    .map(|ch| {
                        self.observed_token(TracedTokenWord::pack(
                            Token::Char {
                                ch,
                                cat: Catcode::Other,
                            },
                            OriginId::UNKNOWN,
                        ))
                    })
                    .collect(),
            }),
        );
        self.push_rendered_text(&escaped, opener.origin());
        Ok(())
    }

    /// pdftex.web §§494 and 496--497's `pdf_unescape_hex_code` conversion.
    ///
    /// After one expanded general-text operand is projected to pdfTeX bytes,
    /// `unescapehex` ignores non-hexadecimal bytes, combines each pair of
    /// remaining digits case-insensitively, and pads a final high nibble with
    /// zero. TeX82 §464's `str_toks` makes a decoded space category 10 and
    /// every other decoded byte category 12.
    pub(super) fn expand_pdf_unescape_hex(
        &mut self,
        opener: CurrentCommand<G>,
    ) -> Result<(), CommandError> {
        let scanned = self.scan_toks(crate::scan_toks::ScanToksMode::General { expanded: true })?;
        let bytes = self.attempt_token_list_bytes(scanned.replacement_text)?;
        let unescaped = unescape_pdf_hex(&bytes);
        observe!(
            self,
            CommandObservation::TokenList(TokenListRecord {
                transition: "complete",
                purpose: "pdf_unescape_hex",
                tokens: unescaped
                    .chars()
                    .map(|ch| {
                        self.observed_token(TracedTokenWord::pack(
                            Token::Char {
                                ch,
                                cat: if ch == ' ' {
                                    Catcode::Space
                                } else {
                                    Catcode::Other
                                },
                            },
                            OriginId::UNKNOWN,
                        ))
                    })
                    .collect(),
            }),
        );
        self.push_rendered_text(&unescaped, opener.origin());
        Ok(())
    }

    /// Installs TeX82 §386's `mark_text` level for `\\topmark` and its kin.
    ///
    /// §386 is `begin_token_list(cur_mark[cur_chr], mark_text)`, a distinct
    /// §307 token type from §467's `inserted`: a mark's text is the stored list
    /// itself, never a copy handed back through `ins_list`.
    pub(super) fn expand_pdf_match(
        &mut self,
        opener: CurrentCommand<G>,
        resume: &mut crate::state::PendingExpansionResume,
        suspended: &mut Option<crate::state::PendingExpansionResume>,
    ) -> Result<(), CommandError> {
        let pending = std::mem::replace(resume, crate::state::PendingExpansionResume::Dispatch);
        let (mut case_insensitive, mut subcount, mut option_phase, retained_pattern) = match pending
        {
            crate::state::PendingExpansionResume::Dispatch => (false, 10_u32, Some(0_u8), None),
            crate::state::PendingExpansionResume::PdfMatchOptions {
                case_insensitive,
                subcount,
                phase,
            } => (case_insensitive, subcount, Some(phase), None),
            crate::state::PendingExpansionResume::PdfMatchPattern {
                case_insensitive,
                subcount,
            } => (case_insensitive, subcount, None, None),
            crate::state::PendingExpansionResume::PdfMatchHaystack {
                case_insensitive,
                subcount,
                pattern,
            } => (case_insensitive, subcount, None, Some(pattern)),
            _ => return Err(CommandError::input_invariant()),
        };
        while let Some(phase) = option_phase {
            match phase {
                0 => {
                    let scan = self.scan_keyword_retained("icase");
                    if self
                        .retain_expansion_scalar(
                            scan,
                            crate::state::PendingExpansionResume::PdfMatchOptions {
                                case_insensitive,
                                subcount,
                                phase,
                            },
                            suspended,
                        )?
                        .value
                    {
                        case_insensitive = true;
                    } else {
                        option_phase = Some(1);
                        continue;
                    }
                }
                1 => {
                    let scan = self.scan_keyword_retained("subcount");
                    if self
                        .retain_expansion_scalar(
                            scan,
                            crate::state::PendingExpansionResume::PdfMatchOptions {
                                case_insensitive,
                                subcount,
                                phase,
                            },
                            suspended,
                        )?
                        .value
                    {
                        option_phase = Some(2);
                        continue;
                    }
                    option_phase = None;
                    continue;
                }
                2 => {
                    let scan = self.scan_integer_retained();
                    subcount = self
                        .retain_expansion_scalar(
                            scan,
                            crate::state::PendingExpansionResume::PdfMatchOptions {
                                case_insensitive,
                                subcount,
                                phase,
                            },
                            suspended,
                        )?
                        .value
                        .max(0) as u32;
                }
                _ => return Err(CommandError::input_invariant()),
            }
            option_phase = Some(0);
        }
        let pattern = if let Some(pattern) = retained_pattern {
            pattern
        } else {
            match self.scan_balanced_text(true) {
                Ok(pattern) => pattern.tokens,
                Err(error) => {
                    if error.is_resource_suspension() {
                        *suspended = Some(crate::state::PendingExpansionResume::PdfMatchPattern {
                            case_insensitive,
                            subcount,
                        });
                    }
                    return Err(error);
                }
            }
        };
        let haystack = match self.scan_balanced_text(true) {
            Ok(haystack) => haystack.tokens,
            Err(error) => {
                if error.is_resource_suspension() {
                    *suspended = Some(crate::state::PendingExpansionResume::PdfMatchHaystack {
                        case_insensitive,
                        subcount,
                        pattern,
                    });
                }
                return Err(error);
            }
        };
        let pattern = pdftex_c_string(self.attempt_token_list_bytes(pattern)?);
        let haystack = pdftex_c_string(self.attempt_token_list_bytes(haystack)?);
        let regex = match PosixRegexBuilder::new(&pattern)
            .with_default_classes()
            .extended(true)
            .compile()
        {
            Ok(regex) => regex.case_insensitive(case_insensitive),
            Err(error) => {
                self.pdftex_regex_warning(posix_regex_diagnostic(&error, &pattern));
                self.push_rendered_text("-1", opener.origin());
                return Ok(());
            }
        };
        let captures = regex.matches(&haystack, Some(1)).into_iter().next();
        let matched = captures.is_some();
        let captures = captures
            .unwrap_or_default()
            .iter()
            .take(subcount as usize)
            .map(|capture| {
                capture.map(|(start, end)| {
                    (
                        u32::try_from(start).expect("bounded TeX string offset fits u32"),
                        u32::try_from(end).expect("bounded TeX string offset fits u32"),
                    )
                })
            })
            .collect();
        self.state
            .set_pdf_match_state(haystack, captures, subcount, matched);
        self.push_rendered_text(if matched { "1" } else { "0" }, opener.origin());
        Ok(())
    }

    fn pdftex_regex_warning(&mut self, message: &str) {
        self.command.semantic_diagnostics.push(
            crate::CommandSemanticDiagnostic::PdfExpansionMessage {
                text: format!("pdfTeX warning: pdftex: \\pdfmatch: {message}"),
            },
        );
    }
}

fn pdftex_c_string(mut bytes: Vec<u8>) -> Vec<u8> {
    if let Some(nul) = bytes.iter().position(|byte| *byte == 0) {
        bytes.truncate(nul);
    }
    bytes
}

fn posix_regex_diagnostic(error: &PosixRegexError, pattern: &[u8]) -> &'static str {
    if matches!(error, PosixRegexError::EOF) && has_unclosed_bracket(pattern) {
        return "brackets ([ ]) not balanced";
    }
    match error {
        PosixRegexError::Expected(b']', _) => "brackets ([ ]) not balanced",
        PosixRegexError::Expected(b')', _) => "parentheses not balanced",
        PosixRegexError::IllegalRange => "invalid character range",
        PosixRegexError::IntegerOverflow | PosixRegexError::EmptyRepetition => {
            "invalid repetition count(s)"
        }
        PosixRegexError::InvalidBackRef(_) => "invalid back reference",
        PosixRegexError::LeadingRepetition => "repetition-operator operand invalid",
        PosixRegexError::UnclosedRepetition => "braces not balanced",
        PosixRegexError::UnknownClass(_) => "invalid character class",
        PosixRegexError::UnknownCollation => "invalid collating element",
        PosixRegexError::EOF | PosixRegexError::Expected(_, _) => {
            "premature end of regular expression"
        }
        PosixRegexError::UnexpectedToken(_) => "invalid regular expression",
    }
}

fn has_unclosed_bracket(pattern: &[u8]) -> bool {
    let mut escaped = false;
    let mut bracket = false;
    for &byte in pattern {
        if escaped {
            escaped = false;
        } else if byte == b'\\' {
            escaped = true;
        } else if byte == b'[' {
            bracket = true;
        } else if byte == b']' {
            bracket = false;
        }
    }
    bracket
}
