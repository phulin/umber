//! Storage-independent macro parameter-program semantics.

use crate::token::{Token, TokenWord};

#[cfg(test)]
#[path = "macro_definition/tests.rs"]
mod tests;

const MACRO_PARAMETER_SLOTS: usize = 9;

/// A malformed immutable macro parameter program.
///
/// These errors are detected while a definition is still mutable attempt or
/// staging data. They must never escape as a panic from immutable
/// publication, including when a checksummed format contains a semantically
/// invalid row.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MacroParameterProgramError {
    InvalidToken,
    TooManyParameters,
    NonSequentialParameter { expected: u8, found: u8 },
    InvalidReplacementParameter { highest: u8, found: u8 },
    CapacityOverflow,
}

/// Allocation-free index of parameter markers in macro parameter text.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct MacroParameterPattern {
    offsets: [u32; MACRO_PARAMETER_SLOTS],
    widths: [u8; MACRO_PARAMETER_SLOTS],
    count: u8,
}

impl MacroParameterPattern {
    pub fn from_tokens(tokens: &[Token]) -> Result<Self, MacroParameterProgramError> {
        let mut builder = MacroParameterPatternBuilder::new();
        for token in tokens.iter().copied() {
            builder.push_parameter_token(token)?;
        }
        Ok(builder.finish())
    }

    #[cfg(test)]
    pub(crate) fn from_words(words: &[TokenWord]) -> Result<Self, MacroParameterProgramError> {
        let mut builder = MacroParameterPatternBuilder::new();
        for word in words.iter().copied() {
            builder.push_parameter(word)?;
        }
        Ok(builder.finish())
    }

    #[must_use]
    pub const fn parameter_count(&self) -> usize {
        self.count as usize
    }

    #[must_use]
    pub fn leading_end(&self, token_count: usize) -> usize {
        if self.count == 0 {
            token_count
        } else {
            self.offsets[0] as usize
        }
    }

    #[must_use]
    pub fn delimiter_bounds(&self, parameter: usize, token_count: usize) -> (usize, usize) {
        assert!(parameter < self.parameter_count());
        let start = self.offsets[parameter] as usize + usize::from(self.widths[parameter]);
        let end = if parameter + 1 < self.parameter_count() {
            self.offsets[parameter + 1] as usize
        } else {
            token_count
        };
        (start, end)
    }

    #[must_use]
    pub const fn marker_index(&self, parameter: usize) -> Option<usize> {
        assert!(parameter < self.parameter_count());
        if self.widths[parameter] == 2 {
            Some(self.offsets[parameter] as usize)
        } else {
            None
        }
    }
}

/// Checked, allocation-free incremental construction of one parameter plan.
///
/// The scanner, ordinary allocation, memo import, and format staging all use
/// this accumulator. A successful push changes it once; validation can be
/// performed on a copy before the caller commits a corresponding token word.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct MacroParameterPatternBuilder {
    pattern: MacroParameterPattern,
    previous: Option<Token>,
    word_len: u32,
}

impl MacroParameterPatternBuilder {
    pub(crate) const fn new() -> Self {
        Self {
            pattern: MacroParameterPattern {
                offsets: [0; MACRO_PARAMETER_SLOTS],
                widths: [0; MACRO_PARAMETER_SLOTS],
                count: 0,
            },
            previous: None,
            word_len: 0,
        }
    }

    pub(crate) fn push_parameter(
        &mut self,
        word: TokenWord,
    ) -> Result<(), MacroParameterProgramError> {
        let token = word
            .token()
            .ok_or(MacroParameterProgramError::InvalidToken)?;
        self.push_parameter_token(token)
    }

    fn push_parameter_token(&mut self, token: Token) -> Result<(), MacroParameterProgramError> {
        let next_len = self
            .word_len
            .checked_add(1)
            .ok_or(MacroParameterProgramError::CapacityOverflow)?;
        if let Token::Param(slot) = token {
            let count = self.pattern.count as usize;
            if count == MACRO_PARAMETER_SLOTS {
                return Err(MacroParameterProgramError::TooManyParameters);
            }
            let expected = self.pattern.count + 1;
            if slot != expected {
                return Err(MacroParameterProgramError::NonSequentialParameter {
                    expected,
                    found: slot,
                });
            }
            let has_spelled_marker = matches!(
                self.previous,
                Some(Token::Char {
                    cat: crate::token::Catcode::Parameter,
                    ..
                })
            );
            self.pattern.offsets[count] = self.word_len - u32::from(has_spelled_marker);
            self.pattern.widths[count] = if has_spelled_marker { 2 } else { 1 };
            self.pattern.count = expected;
        }
        self.previous = Some(token);
        self.word_len = next_len;
        Ok(())
    }

    pub(crate) fn validate_replacement(
        &self,
        word: TokenWord,
    ) -> Result<(), MacroParameterProgramError> {
        let token = word
            .token()
            .ok_or(MacroParameterProgramError::InvalidToken)?;
        if let Token::Param(slot) = token
            && slot > self.pattern.count
        {
            return Err(MacroParameterProgramError::InvalidReplacementParameter {
                highest: self.pattern.count,
                found: slot,
            });
        }
        Ok(())
    }

    pub(crate) const fn finish(self) -> MacroParameterPattern {
        self.pattern
    }
}
