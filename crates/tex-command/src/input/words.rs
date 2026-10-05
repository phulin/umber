//! Borrowed physical input words, with their actual spelling representation.
use tex_state::token::{OriginId, TokenWord, TracedTokenWord};

#[derive(Clone, Copy)]
pub(crate) enum InputWords<'a> {
    Traced(&'a [TracedTokenWord]),
    Packed(&'a [TokenWord]),
}
impl<'a> InputWords<'a> {
    #[inline(always)]
    pub(crate) fn get(self, index: usize) -> Option<TracedTokenWord> {
        match self {
            Self::Traced(words) => words.get(index).copied(),
            Self::Packed(words) => words
                .get(index)
                .map(|&word| TracedTokenWord::from_parts(word, OriginId::UNKNOWN)),
        }
    }
    #[inline(always)]
    pub(crate) fn slice(self, range: core::ops::Range<usize>) -> Option<Self> {
        match self {
            Self::Traced(words) => words.get(range).map(Self::Traced),
            Self::Packed(words) => words.get(range).map(Self::Packed),
        }
    }
    pub(crate) fn iter(self) -> InputWordIter<'a> {
        match self {
            Self::Traced(words) => InputWordIter::Traced(words.iter().copied()),
            Self::Packed(words) => InputWordIter::Packed(words.iter().copied()),
        }
    }
}

pub(crate) enum InputWordIter<'a> {
    Traced(core::iter::Copied<core::slice::Iter<'a, TracedTokenWord>>),
    Packed(core::iter::Copied<core::slice::Iter<'a, TokenWord>>),
}
impl Iterator for InputWordIter<'_> {
    type Item = TracedTokenWord;
    #[inline(always)]
    fn next(&mut self) -> Option<Self::Item> {
        match self {
            Self::Traced(words) => words.next(),
            Self::Packed(words) => words
                .next()
                .map(|word| TracedTokenWord::from_parts(word, OriginId::UNKNOWN)),
        }
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        let len = self.len();
        (len, Some(len))
    }
}
impl ExactSizeIterator for InputWordIter<'_> {
    fn len(&self) -> usize {
        match self {
            Self::Traced(words) => words.len(),
            Self::Packed(words) => words.len(),
        }
    }
}
impl DoubleEndedIterator for InputWordIter<'_> {
    fn next_back(&mut self) -> Option<Self::Item> {
        match self {
            Self::Traced(words) => words.next_back(),
            Self::Packed(words) => words
                .next_back()
                .map(|word| TracedTokenWord::from_parts(word, OriginId::UNKNOWN)),
        }
    }
}
