//! Storage-independent macro parameter-program semantics.

use crate::token::{Token, TokenWord};

#[cfg(test)]
#[path = "macro_definition/tests.rs"]
mod tests;

const MACRO_PARAMETER_SLOTS: usize = 9;

/// Allocation-free index of parameter markers in macro parameter text.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct MacroParameterPattern {
    offsets: [u32; MACRO_PARAMETER_SLOTS],
    widths: [u8; MACRO_PARAMETER_SLOTS],
    count: u8,
}

impl MacroParameterPattern {
    pub fn from_tokens(tokens: &[Token]) -> Self {
        Self::from_token_iter(tokens.iter().copied())
    }

    pub(crate) fn from_words(words: &[TokenWord]) -> Self {
        Self::from_token_iter(words.iter().map(|word| word.semantic_token()))
    }

    fn from_token_iter(tokens: impl Iterator<Item = Token>) -> Self {
        let mut offsets = [0; MACRO_PARAMETER_SLOTS];
        let mut widths = [0; MACRO_PARAMETER_SLOTS];
        let mut count = 0_usize;
        let mut previous = None;
        for (index, token) in tokens.enumerate() {
            if matches!(token, Token::Param(_)) {
                assert!(
                    count < MACRO_PARAMETER_SLOTS,
                    "macro has more than nine parameters"
                );
                let has_spelled_marker = matches!(
                    previous,
                    Some(Token::Char {
                        cat: crate::token::Catcode::Parameter,
                        ..
                    })
                );
                offsets[count] = u32::try_from(index - usize::from(has_spelled_marker))
                    .expect("token list length exceeds u32");
                widths[count] = if has_spelled_marker { 2 } else { 1 };
                count += 1;
            }
            previous = Some(token);
        }
        Self {
            offsets,
            widths,
            count: count as u8,
        }
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
