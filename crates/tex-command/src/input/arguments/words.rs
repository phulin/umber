//! Actual spelling retained by one final macro argument.
use tex_state::token::{OriginId, TokenWord, TracedTokenWord};

/// A final pooled range, or a range in its invocation's retained-source lane.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct ArgumentValue {
    pub(super) start: u32,
    pub(super) len: u32,
}
impl ArgumentValue {
    #[inline(always)]
    pub(crate) const fn len(&self) -> u32 {
        self.len
    }
}

#[derive(Clone, Copy)]
pub(crate) enum ArgumentWords<'a> {
    Traced(&'a [TracedTokenWord]),
    Definition(&'a [TokenWord]),
}
impl<'a> ArgumentWords<'a> {
    #[inline(always)]
    pub(crate) fn get(self, index: usize) -> Option<TracedTokenWord> {
        match self {
            Self::Traced(words) => words.get(index).copied(),
            Self::Definition(words) => words
                .get(index)
                .map(|&word| TracedTokenWord::from_parts(word, OriginId::UNKNOWN)),
        }
    }
    pub(crate) fn slice(self, range: core::ops::Range<usize>) -> Option<Self> {
        match self {
            Self::Traced(words) => words.get(range).map(Self::Traced),
            Self::Definition(words) => words.get(range).map(Self::Definition),
        }
    }
    pub(crate) fn iter(self) -> MacroWords<'a> {
        match self {
            Self::Traced(words) => MacroWords::Traced(words.iter().copied()),
            Self::Definition(words) => MacroWords::Definition(words.iter().copied()),
        }
    }
}

pub(crate) enum MacroWords<'a> {
    Traced(core::iter::Copied<core::slice::Iter<'a, TracedTokenWord>>),
    Definition(core::iter::Copied<core::slice::Iter<'a, TokenWord>>),
}
impl Iterator for MacroWords<'_> {
    type Item = TracedTokenWord;
    #[inline(always)]
    fn next(&mut self) -> Option<Self::Item> {
        match self {
            Self::Traced(words) => words.next(),
            Self::Definition(words) => words
                .next()
                .map(|word| TracedTokenWord::from_parts(word, OriginId::UNKNOWN)),
        }
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        let len = self.len();
        (len, Some(len))
    }
}
impl ExactSizeIterator for MacroWords<'_> {
    fn len(&self) -> usize {
        match self {
            Self::Traced(words) => words.len(),
            Self::Definition(words) => words.len(),
        }
    }
}
impl DoubleEndedIterator for MacroWords<'_> {
    fn next_back(&mut self) -> Option<Self::Item> {
        match self {
            Self::Traced(words) => words.next_back(),
            Self::Definition(words) => words
                .next_back()
                .map(|word| TracedTokenWord::from_parts(word, OriginId::UNKNOWN)),
        }
    }
}
