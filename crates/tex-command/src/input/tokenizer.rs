//! Canonical token-at-a-time source tokenization.
//!
//! The scalar state machine follows TeX.web section 24 (`get_next` and
//! `Scan a control sequence`). It deliberately stops at one token or one
//! recoverable invalid-character diagnostic so callers can observe aggregate
//! state changes before the next character is classified.

use std::sync::Arc;

use tex_state::token::Catcode;

use crate::profile::{CharacterCode, CharacterMode};

use super::lines::{SourceCharacter, SourceRange};
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
    /// A backslash followed by one or more current-catcode letters.
    Word,
    /// A backslash followed by one nonletter character.
    Symbol,
    /// An active character, whose namespace is distinct from escaped names.
    Active,
    /// TeX's frozen blank-line `\par` spelling.
    Paragraph,
    /// The null control sequence produced by an escape at normalized line end.
    Null,
}

/// An exact-byte token spelling before control-sequence interning.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub enum SourceToken {
    /// A character token with the catcode observed at delivery.
    Character {
        code: CharacterCode,
        catcode: Catcode,
        range: SourceRange,
    },
    /// A control sequence whose name remains a semantic character sequence.
    ControlSequence {
        name: Vec<CharacterCode>,
        kind: SourceControlSequenceKind,
        range: SourceRange,
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
}

/// One recoverable invalid-character observation.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct InvalidSourceCharacter {
    code: CharacterCode,
    range: SourceRange,
}

impl InvalidSourceCharacter {
    /// Exact byte-domain character that carried catcode 15.
    #[must_use]
    pub const fn code(self) -> CharacterCode {
        self.code
    }

    /// Complete source spelling, including any canonical `^^` notation.
    #[must_use]
    pub const fn range(self) -> SourceRange {
        self.range
    }
}

/// One externally observable step of exact-byte source tokenization.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub enum SourceTokenizationStep {
    /// One complete semantic source token.
    Token(SourceToken),
    /// A consumed catcode-15 character requiring canonical recovery.
    InvalidCharacter(InvalidSourceCharacter),
    /// The registered source has no remaining physical input.
    End,
}

impl SourceCursor {
    /// Delivers one exact-byte tokenization step.
    ///
    /// `catcode` is called for every classified character, including
    /// control-word lookahead and each result of `^^` reduction. The callback
    /// is intentionally supplied per call rather than retained in input state.
    pub(crate) fn next_exact_byte_step(
        &mut self,
        endlinechar: i32,
        mut catcode: impl FnMut(CharacterCode) -> Catcode,
    ) -> SourceTokenizationStep {
        debug_assert_eq!(self.backing.mode, CharacterMode::EightBitExact);
        let bytes = Arc::clone(&self.backing.bytes);

        loop {
            if self.line.is_none() && self.load_next_line(endlinechar).is_none() {
                return SourceTokenizationStep::End;
            }

            let Some(character) = self.next_reduced_character(&bytes, &mut catcode) else {
                self.finish_line();
                continue;
            };
            let observed = catcode(character.code());

            match observed {
                Catcode::Ignored => continue,
                Catcode::Invalid => {
                    return SourceTokenizationStep::InvalidCharacter(InvalidSourceCharacter {
                        code: character.code(),
                        range: character.range(),
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
                        &mut catcode,
                    ));
                }
                Catcode::Active => {
                    self.lexer_state = LexerState::MidLine;
                    return SourceTokenizationStep::Token(SourceToken::ControlSequence {
                        name: vec![character.code()],
                        kind: SourceControlSequenceKind::Active,
                        range: character.range(),
                    });
                }
                Catcode::Space => match self.lexer_state {
                    LexerState::MidLine => {
                        self.lexer_state = LexerState::SkipBlanks;
                        return SourceTokenizationStep::Token(SourceToken::Character {
                            code: CharacterCode::from_byte(b' '),
                            catcode: Catcode::Space,
                            range: character.range(),
                        });
                    }
                    LexerState::SkipBlanks | LexerState::NewLine => continue,
                },
                Catcode::EndLine => match self.lexer_state {
                    LexerState::MidLine => {
                        self.lexer_state = LexerState::NewLine;
                        return SourceTokenizationStep::Token(SourceToken::Character {
                            code: CharacterCode::from_byte(b' '),
                            catcode: Catcode::Space,
                            range: character.range(),
                        });
                    }
                    LexerState::SkipBlanks => {
                        self.lexer_state = LexerState::NewLine;
                        continue;
                    }
                    LexerState::NewLine => {
                        return SourceTokenizationStep::Token(SourceToken::ControlSequence {
                            name: b"par"
                                .iter()
                                .copied()
                                .map(CharacterCode::from_byte)
                                .collect(),
                            kind: SourceControlSequenceKind::Paragraph,
                            range: character.range(),
                        });
                    }
                },
                _ => {
                    self.lexer_state = LexerState::MidLine;
                    return SourceTokenizationStep::Token(SourceToken::Character {
                        code: character.code(),
                        catcode: observed,
                        range: character.range(),
                    });
                }
            }
        }
    }

    fn scan_control_sequence(
        &mut self,
        escape: SourceCharacter,
        bytes: &[u8],
        catcode: &mut impl FnMut(CharacterCode) -> Catcode,
    ) -> SourceToken {
        let Some(first) = self.next_reduced_character(bytes, catcode) else {
            return SourceToken::ControlSequence {
                name: Vec::new(),
                kind: SourceControlSequenceKind::Null,
                range: escape.range(),
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
                let Some(next) = self.next_reduced_character(bytes, catcode) else {
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
        }
    }

    fn next_reduced_character(
        &mut self,
        bytes: &[u8],
        catcode: &mut impl FnMut(CharacterCode) -> Catcode,
    ) -> Option<SourceCharacter> {
        let line = self.line.as_mut()?;
        let first = line.next_character(CharacterMode::EightBitExact, bytes)?;
        reduce_superscript_notation(line, bytes, first, catcode)
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
        let Some(second) = trial.next_character(CharacterMode::EightBitExact, bytes) else {
            return Some(current);
        };
        if second.code() != current.code() {
            return Some(current);
        }
        let Some(third) = trial.next_character(CharacterMode::EightBitExact, bytes) else {
            return Some(current);
        };
        let third_byte = third.code().to_byte().ok()?;
        if third_byte >= 128 {
            return Some(current);
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
        *line = consumed;
        current = SourceCharacter {
            code: CharacterCode::from_byte(result),
            range: SourceRange::new(first.range().source(), start, line.byte_cursor),
            scalar_offset: first.scalar_offset(),
            synthetic: false,
        };
        // TeX's `goto reswitch` can reduce again when the replacement itself
        // has the current superscript catcode.
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
