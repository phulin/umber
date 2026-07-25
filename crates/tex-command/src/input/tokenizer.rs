//! Canonical token-at-a-time source tokenization.
//!
//! The shared scalar state machine follows TeX.web section 24 (`get_next` and
//! `Scan a control sequence`). Exact-byte profiles retain canonical TeX
//! superscript behavior. `UnicodeExtended` is a separately identified Umber
//! contract over validated Unicode scalars, never a pdfTeX compatibility mode.

use std::sync::Arc;

use tex_state::token::Catcode;

use crate::profile::{CharacterCode, CharacterMode};

use super::lines::{SourceCharacter, SourceProvenance, SourceRange, SourceScalarRange};
use super::source::SourceCursor;

/// TeX's source-line lexical state (`mid_line`, `skip_blanks`, `new_line`).
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub enum LexerState {
    /// Ordinary material has been seen on the current physical line.
    MidLine,
    /// A control word or emitted space suppresses following spacer tokens.
    SkipBlanks,
    /// No material has been emitted from the current physical line.
    #[default]
    NewLine,
}

/// The namespace and construction rule for a source control sequence.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum SourceControlSequenceKind {
    /// An escape followed by one or more current-catcode letters.
    Word,
    /// An escape followed by one nonletter character.
    Symbol,
    /// An active character, whose namespace is distinct from escaped names.
    Active,
    /// TeX's frozen blank-line `\par` spelling in the active character domain.
    Paragraph,
    /// The null control sequence produced by an escape at normalized line end.
    Null,
}

/// One source token spelling before control-sequence interning.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub enum SourceToken {
    /// A character token with the catcode observed at delivery.
    Character {
        code: CharacterCode,
        catcode: Catcode,
        range: SourceRange,
        scalar_range: SourceScalarRange,
    },
    /// A control sequence whose name remains a semantic character sequence.
    ControlSequence {
        name: Vec<CharacterCode>,
        kind: SourceControlSequenceKind,
        range: SourceRange,
        scalar_range: SourceScalarRange,
    },
}

impl SourceToken {
    /// Exact half-open physical spelling range.
    #[must_use]
    pub fn range(&self) -> SourceRange {
        match self {
            Self::Character { range, .. } | Self::ControlSequence { range, .. } => *range,
        }
    }

    /// Exact half-open decoded-scalar spelling range within the physical line.
    #[must_use]
    pub fn scalar_range(&self) -> SourceScalarRange {
        match self {
            Self::Character { scalar_range, .. } | Self::ControlSequence { scalar_range, .. } => {
                *scalar_range
            }
        }
    }

    /// Raw span and canonical TeX82 location for this decoded spelling.
    #[must_use]
    pub fn provenance(&self) -> SourceProvenance {
        SourceProvenance::from_range(self.range())
    }
}

/// One recoverable invalid-character observation.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct InvalidSourceCharacter {
    code: CharacterCode,
    range: SourceRange,
    scalar_range: SourceScalarRange,
}

impl InvalidSourceCharacter {
    /// Exact semantic character that carried catcode 15.
    #[must_use]
    pub const fn code(self) -> CharacterCode {
        self.code
    }

    /// Complete physical spelling, including superscript notation.
    #[must_use]
    pub const fn range(self) -> SourceRange {
        self.range
    }

    /// Complete decoded-scalar spelling range within the physical line.
    #[must_use]
    pub const fn scalar_range(self) -> SourceScalarRange {
        self.scalar_range
    }
}

/// One externally observable step of source tokenization.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub enum SourceTokenizationStep {
    /// One complete semantic source token.
    Token(SourceToken),
    /// A consumed catcode-15 character requiring canonical recovery.
    InvalidCharacter(InvalidSourceCharacter),
    /// The registered source has no remaining physical input.
    End,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SuperscriptPolicy {
    ExactByte,
    UnicodeExtended,
}

impl SourceCursor {
    /// Delivers one exact-byte tokenization step.
    pub(crate) fn next_exact_byte_step(
        &mut self,
        endlinechar: i32,
        catcode: impl FnMut(CharacterCode) -> Catcode,
    ) -> SourceTokenizationStep {
        self.next_source_step(
            endlinechar,
            CharacterMode::EightBitExact,
            SuperscriptPolicy::ExactByte,
            catcode,
        )
    }

    /// Delivers one separately identified Unicode-extension tokenization step.
    pub(crate) fn next_unicode_step(
        &mut self,
        endlinechar: i32,
        catcode: impl FnMut(CharacterCode) -> Catcode,
    ) -> SourceTokenizationStep {
        self.next_source_step(
            endlinechar,
            CharacterMode::UnicodeExtended,
            SuperscriptPolicy::UnicodeExtended,
            catcode,
        )
    }

    fn next_source_step(
        &mut self,
        endlinechar: i32,
        mode: CharacterMode,
        superscript: SuperscriptPolicy,
        mut catcode: impl FnMut(CharacterCode) -> Catcode,
    ) -> SourceTokenizationStep {
        debug_assert_eq!(self.backing.mode, mode);
        let bytes = Arc::clone(&self.backing.bytes);

        loop {
            if self.line.is_none() && self.load_next_line(endlinechar).is_none() {
                return SourceTokenizationStep::End;
            }

            let Some(character) =
                self.next_reduced_character(&bytes, mode, superscript, &mut catcode)
            else {
                self.finish_line();
                if self.end_after_line {
                    return SourceTokenizationStep::End;
                }
                continue;
            };
            let scalar_range = self.spelling_scalar_range(character);
            let observed = catcode(character.code());

            match observed {
                Catcode::Ignored => continue,
                Catcode::Invalid => {
                    return SourceTokenizationStep::InvalidCharacter(InvalidSourceCharacter {
                        code: character.code(),
                        range: character.range(),
                        scalar_range,
                    });
                }
                Catcode::Comment => {
                    self.discard_line();
                    continue;
                }
                Catcode::Escape => {
                    return SourceTokenizationStep::Token(self.scan_control_sequence(
                        character,
                        &bytes,
                        mode,
                        superscript,
                        &mut catcode,
                    ));
                }
                Catcode::Active => {
                    self.lexer_state = LexerState::MidLine;
                    return SourceTokenizationStep::Token(SourceToken::ControlSequence {
                        name: vec![character.code()],
                        kind: SourceControlSequenceKind::Active,
                        range: character.range(),
                        scalar_range,
                    });
                }
                Catcode::Space => match self.lexer_state {
                    LexerState::MidLine => {
                        self.lexer_state = LexerState::SkipBlanks;
                        return SourceTokenizationStep::Token(SourceToken::Character {
                            code: semantic_ascii(mode, b' '),
                            catcode: Catcode::Space,
                            range: character.range(),
                            scalar_range,
                        });
                    }
                    LexerState::SkipBlanks | LexerState::NewLine => continue,
                },
                Catcode::EndLine => match self.lexer_state {
                    LexerState::MidLine => {
                        self.lexer_state = LexerState::NewLine;
                        return SourceTokenizationStep::Token(SourceToken::Character {
                            code: semantic_ascii(mode, b' '),
                            catcode: Catcode::Space,
                            range: character.range(),
                            scalar_range,
                        });
                    }
                    LexerState::SkipBlanks => {
                        self.lexer_state = LexerState::NewLine;
                        continue;
                    }
                    LexerState::NewLine => {
                        // The generated paragraph is a control sequence, so
                        // its canonical source position is the physical line
                        // terminator that caused it. Retaining this typed
                        // distinction avoids treating the zero-width
                        // synthetic endline anchor like an explicit `\par`
                        // spelling (whose position is its final source
                        // character). Unterminated EOF input intentionally
                        // remains a zero-width anchor.
                        let range = self.current_terminator_range();
                        return SourceTokenizationStep::Token(SourceToken::ControlSequence {
                            name: b"par"
                                .iter()
                                .copied()
                                .map(|byte| semantic_ascii(mode, byte))
                                .collect(),
                            kind: SourceControlSequenceKind::Paragraph,
                            range,
                            scalar_range,
                        });
                    }
                },
                _ => {
                    self.lexer_state = LexerState::MidLine;
                    return SourceTokenizationStep::Token(SourceToken::Character {
                        code: character.code(),
                        catcode: observed,
                        range: character.range(),
                        scalar_range,
                    });
                }
            }
        }
    }

    fn scan_control_sequence(
        &mut self,
        escape: SourceCharacter,
        bytes: &[u8],
        mode: CharacterMode,
        superscript: SuperscriptPolicy,
        catcode: &mut impl FnMut(CharacterCode) -> Catcode,
    ) -> SourceToken {
        let Some(first) = self.next_reduced_character(bytes, mode, superscript, catcode) else {
            return SourceToken::ControlSequence {
                name: Vec::new(),
                kind: SourceControlSequenceKind::Null,
                range: escape.range(),
                scalar_range: self.spelling_scalar_range(escape),
            };
        };
        let first_catcode = catcode(first.code());
        self.lexer_state = if matches!(first_catcode, Catcode::Letter | Catcode::Space) {
            LexerState::SkipBlanks
        } else {
            LexerState::MidLine
        };

        let mut name = vec![first.code()];
        let mut end = first.range().end();
        let kind = if first_catcode == Catcode::Letter {
            loop {
                let saved = self.line.clone().expect("control sequence has a line");
                let Some(next) = self.next_reduced_character(bytes, mode, superscript, catcode)
                else {
                    break;
                };
                if catcode(next.code()) != Catcode::Letter {
                    self.line = Some(saved);
                    break;
                }
                name.push(next.code());
                end = next.range().end();
            }
            SourceControlSequenceKind::Word
        } else {
            SourceControlSequenceKind::Symbol
        };

        SourceToken::ControlSequence {
            name,
            kind,
            range: SourceRange::new(escape.range().source(), escape.range().start(), end),
            scalar_range: SourceScalarRange::new(
                escape.scalar_offset(),
                self.line
                    .as_ref()
                    .map_or(first.scalar_offset() + 1, |line| line.scalar_cursor),
            ),
        }
    }

    fn next_reduced_character(
        &mut self,
        bytes: &[u8],
        mode: CharacterMode,
        superscript: SuperscriptPolicy,
        catcode: &mut impl FnMut(CharacterCode) -> Catcode,
    ) -> Option<SourceCharacter> {
        let line = self.line.as_mut()?;
        let first = line.next_character(mode, bytes)?;
        reduce_superscript_notation(line, bytes, first, mode, superscript, catcode)
    }

    fn spelling_scalar_range(&self, character: SourceCharacter) -> SourceScalarRange {
        SourceScalarRange::new(
            character.scalar_offset(),
            self.line
                .as_ref()
                .map_or(character.scalar_offset() + 1, |line| line.scalar_cursor),
        )
    }

    fn discard_line(&mut self) {
        if let Some(line) = &mut self.line {
            line.byte_cursor = line.retained_end;
            line.endline_delivered = true;
        }
        self.finish_line();
    }
}

fn reduce_superscript_notation(
    line: &mut super::lines::SourceLineState,
    bytes: &[u8],
    first: SourceCharacter,
    mode: CharacterMode,
    policy: SuperscriptPolicy,
    catcode: &mut impl FnMut(CharacterCode) -> Catcode,
) -> Option<SourceCharacter> {
    let start = first.range().start();
    let mut current = first;

    loop {
        if catcode(current.code()) != Catcode::Superscript {
            return Some(SourceCharacter {
                code: current.code(),
                range: SourceRange::new(current.range().source(), start, current.range().end()),
                scalar_offset: first.scalar_offset(),
                synthetic: current.is_synthetic(),
            });
        }
        let mut trial = line.clone();
        let Some(second) = trial.next_character(mode, bytes) else {
            return Some(current);
        };
        if second.code() != current.code() {
            return Some(current);
        }
        let Some(third) = trial.next_character(mode, bytes) else {
            return Some(current);
        };

        let reduced = match policy {
            SuperscriptPolicy::ExactByte => reduce_exact_superscript(trial, bytes, third),
            SuperscriptPolicy::UnicodeExtended => {
                reduce_unicode_superscript(trial, bytes, current.code(), third)
            }
        };
        let Some((code, consumed)) = reduced else {
            return Some(current);
        };
        *line = consumed;
        current = SourceCharacter {
            code,
            range: SourceRange::new(first.range().source(), start, line.byte_cursor),
            scalar_offset: first.scalar_offset(),
            synthetic: false,
        };
        // TeX's `reswitch` behavior applies again if the replacement currently
        // has superscript catcode.
    }
}

fn reduce_exact_superscript(
    trial: super::lines::SourceLineState,
    bytes: &[u8],
    third: SourceCharacter,
) -> Option<(CharacterCode, super::lines::SourceLineState)> {
    let third_byte = third.code().to_byte().ok()?;
    if third_byte >= 128 {
        return None;
    }
    let (result, consumed) = if let Some(high) = lower_hex_value(third_byte) {
        let mut hexadecimal = trial.clone();
        match hexadecimal.next_character(CharacterMode::EightBitExact, bytes) {
            Some(fourth) => match fourth.code().to_byte().ok().and_then(lower_hex_value) {
                Some(low) => (16 * high + low, hexadecimal),
                None => (toggle_ascii(third_byte), trial),
            },
            None => (toggle_ascii(third_byte), trial),
        }
    } else {
        (toggle_ascii(third_byte), trial)
    };
    Some((CharacterCode::from_byte(result), consumed))
}

fn reduce_unicode_superscript(
    trial: super::lines::SourceLineState,
    bytes: &[u8],
    superscript: CharacterCode,
    third: SourceCharacter,
) -> Option<(CharacterCode, super::lines::SourceLineState)> {
    let mut after_fourth = trial.clone();
    if third.code() == superscript
        && after_fourth
            .next_character(CharacterMode::UnicodeExtended, bytes)
            .is_some_and(|fourth| fourth.code() == superscript)
        && let Some((value, consumed)) = take_unicode_hex(after_fourth, bytes, 4)
        && let Ok(code) = CharacterCode::from_unicode_scalar(value)
    {
        return Some((code, consumed));
    }

    if let Some(high) = ascii_hex_value(third.code()) {
        let mut consumed = trial.clone();
        if let Some(fourth) = consumed.next_character(CharacterMode::UnicodeExtended, bytes)
            && let Some(low) = ascii_hex_value(fourth.code())
        {
            return Some((
                CharacterCode::from_unicode_scalar(16 * high + low)
                    .expect("two hexadecimal digits form a scalar"),
                consumed,
            ));
        }
    }

    let scalar = third.code().to_unicode_scalar().ok()?;
    let toggled = if scalar < 64 {
        scalar.checked_add(64)?
    } else {
        scalar.checked_sub(64)?
    };
    CharacterCode::from_unicode_scalar(toggled)
        .ok()
        .map(|code| (code, trial))
}

fn take_unicode_hex(
    mut line: super::lines::SourceLineState,
    bytes: &[u8],
    count: usize,
) -> Option<(u32, super::lines::SourceLineState)> {
    let mut value = 0_u32;
    for _ in 0..count {
        let character = line.next_character(CharacterMode::UnicodeExtended, bytes)?;
        value = value
            .checked_mul(16)?
            .checked_add(ascii_hex_value(character.code())?)?;
    }
    Some((value, line))
}

fn semantic_ascii(mode: CharacterMode, byte: u8) -> CharacterCode {
    match mode {
        CharacterMode::EightBitExact => CharacterCode::from_byte(byte),
        CharacterMode::UnicodeExtended => CharacterCode::from_unicode_scalar(u32::from(byte))
            .expect("ASCII is a valid Unicode scalar"),
    }
}

fn ascii_hex_value(code: CharacterCode) -> Option<u32> {
    let scalar = code.to_unicode_scalar().ok()?;
    match scalar {
        value @ 0x30..=0x39 => Some(value - 0x30),
        value @ 0x41..=0x46 => Some(value - 0x41 + 10),
        value @ 0x61..=0x66 => Some(value - 0x61 + 10),
        _ => None,
    }
}

const fn lower_hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        _ => None,
    }
}

const fn toggle_ascii(byte: u8) -> u8 {
    if byte < 64 { byte + 64 } else { byte - 64 }
}

#[cfg(test)]
mod tests;
