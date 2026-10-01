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

/// Mutable-build index of validated parameter markers.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct MacroParameterIndex {
    offsets: [u32; MACRO_PARAMETER_SLOTS],
    widths: [u8; MACRO_PARAMETER_SLOTS],
    count: u8,
}

/// Immutable definition-owned parameter program and delimiter failure links.
///
/// Links are indexed relative to the first delimiter so the marker index remains
/// the sole authority for delimiter bounds. Definitions with only empty or
/// single-word delimiters allocate no links.
#[derive(Debug, Eq, Hash, PartialEq)]
pub struct MacroParameterPattern {
    index: MacroParameterIndex,
    failure: Box<[u32]>,
}

/// A borrowed literal delimiter and its sealed overlap metadata.
#[derive(Clone, Copy, Debug)]
pub struct MacroDelimiter<'a> {
    words: &'a [TokenWord],
    failure: &'a [u32],
}

impl<'a> MacroDelimiter<'a> {
    #[must_use]
    pub const fn words(self) -> &'a [TokenWord] {
        self.words
    }

    #[must_use]
    pub fn len(self) -> usize {
        self.words.len()
    }

    #[must_use]
    pub fn is_empty(self) -> bool {
        self.words.is_empty()
    }

    /// Next reusable prefix after a mismatch at the held-prefix boundary.
    /// The caller owns brace, paragraph, recovery, and source-origin semantics.
    #[must_use]
    pub fn overlap(self, prefix_len: usize, current: TokenWord) -> usize {
        let mut matched = prefix_len;
        while matched != 0 && current != self.words[matched] {
            matched = self.failure[matched - 1] as usize;
        }
        if current == self.words[matched] {
            matched += 1;
        }
        matched.min(self.words.len() - 1)
    }
}

impl MacroParameterPattern {
    #[cfg(test)]
    pub(crate) fn from_words(words: &[TokenWord]) -> Result<Self, MacroParameterProgramError> {
        let mut builder = MacroParameterPatternBuilder::new();
        for word in words.iter().copied() {
            builder.push_parameter(word)?;
        }
        Ok(builder.finish(words))
    }

    #[must_use]
    pub const fn parameter_count(&self) -> usize {
        self.index.count as usize
    }

    #[must_use]
    pub fn leading_end(&self, token_count: usize) -> usize {
        if self.index.count == 0 {
            token_count
        } else {
            self.index.offsets[0] as usize
        }
    }

    #[must_use]
    pub fn delimiter_bounds(&self, parameter: usize, token_count: usize) -> (usize, usize) {
        assert!(parameter < self.parameter_count());
        let start =
            self.index.offsets[parameter] as usize + usize::from(self.index.widths[parameter]);
        let end = if parameter + 1 < self.parameter_count() {
            self.index.offsets[parameter + 1] as usize
        } else {
            token_count
        };
        (start, end)
    }

    #[must_use]
    pub const fn marker_index(&self, parameter: usize) -> Option<usize> {
        assert!(parameter < self.parameter_count());
        if self.index.widths[parameter] == 2 {
            Some(self.index.offsets[parameter] as usize)
        } else {
            None
        }
    }
    /// Borrows one delimiter directly from the retained definition words.
    #[must_use]
    pub fn delimiter<'a>(&'a self, parameter: usize, words: &'a [TokenWord]) -> MacroDelimiter<'a> {
        let (start, end) = self.delimiter_bounds(parameter, words.len());
        let base = self.delimiter_bounds(0, words.len()).0;
        MacroDelimiter {
            words: &words[start..end],
            failure: if end - start > 1 {
                &self.failure[start - base..end - base]
            } else {
                &[]
            },
        }
    }

    pub(crate) fn failure_len(&self) -> usize {
        self.failure.len()
    }
}

/// Checked, allocation-free incremental construction of one parameter plan.
///
/// The scanner, ordinary allocation, memo import, and format staging all use
/// this accumulator. A successful push changes it once; validation can be
/// performed on a copy before the caller commits a corresponding token word.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct MacroParameterPatternBuilder {
    pattern: MacroParameterIndex,
    previous: Option<Token>,
    word_len: u32,
}

impl MacroParameterPatternBuilder {
    pub(crate) const fn new() -> Self {
        Self {
            pattern: MacroParameterIndex {
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

    pub(crate) fn finish(self, words: &[TokenWord]) -> MacroParameterPattern {
        debug_assert_eq!(words.len(), self.word_len as usize);
        let mut pattern = MacroParameterPattern {
            index: self.pattern,
            failure: Box::default(),
        };
        if (0..pattern.parameter_count()).any(|parameter| {
            let (start, end) = pattern.delimiter_bounds(parameter, words.len());
            end - start > 1
        }) {
            let base = pattern.delimiter_bounds(0, words.len()).0;
            let mut failure = vec![0; words.len() - base];
            for parameter in 0..pattern.parameter_count() {
                let (start, end) = pattern.delimiter_bounds(parameter, words.len());
                for index in start + 1..end {
                    let mut matched = failure[index - base - 1] as usize;
                    while matched != 0 && words[index] != words[start + matched] {
                        matched = failure[start - base + matched - 1] as usize;
                    }
                    if words[index] == words[start + matched] {
                        matched += 1;
                    }
                    failure[index - base] = matched as u32;
                }
            }
            pattern.failure = failure.into_boxed_slice();
        }
        pattern
    }
}
