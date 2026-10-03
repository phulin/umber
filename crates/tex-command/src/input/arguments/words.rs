//! Actual spelling retained by one final macro argument.
use super::ArgumentRegionId;
use tex_state::ResidentMacroBody;
use tex_state::token::{OriginId, TokenWord, TracedTokenWord};

#[derive(Debug)]
pub(crate) enum ArgumentValue<G> {
    Inline(TracedTokenWord),
    Owned {
        start: u32,
        len: u32,
    },
    Definition {
        body: ResidentMacroBody<G>,
        start: u32,
        len: u32,
    },
    Region {
        region: ArgumentRegionId,
        slot: u8,
        start: u32,
        len: u32,
    },
}
impl<G> Default for ArgumentValue<G> {
    fn default() -> Self {
        Self::Owned { start: 0, len: 0 }
    }
}
impl<G> ArgumentValue<G> {
    #[inline(always)]
    pub(crate) fn len(&self) -> u32 {
        match self {
            Self::Inline(_) => 1,
            Self::Owned { len, .. } | Self::Definition { len, .. } | Self::Region { len, .. } => {
                *len
            }
        }
    }
    pub(super) fn trim(&mut self, start_delta: u32, len: u32) {
        match self {
            Self::Inline(_) => {
                debug_assert!(start_delta + len <= 1);
                if len == 0 {
                    *self = Self::default();
                }
            }
            Self::Owned { start, len: size }
            | Self::Definition {
                start, len: size, ..
            }
            | Self::Region {
                start, len: size, ..
            } => {
                *start += start_delta;
                *size = len;
            }
        }
    }
    pub(super) fn extend(&mut self, count: u32) {
        match self {
            Self::Inline(_) => unreachable!("extending an inline word materializes its owner"),
            Self::Owned { len, .. } | Self::Definition { len, .. } | Self::Region { len, .. } => {
                *len += count
            }
        }
    }
}

#[derive(Clone, Copy)]
pub(crate) enum ArgumentWords<'a> {
    Traced(&'a [TracedTokenWord]),
    Definition(&'a [TokenWord]),
}
impl<'a> ArgumentWords<'a> {
    #[inline(always)]
    pub(crate) fn len(self) -> usize {
        match self {
            Self::Traced(words) => words.len(),
            Self::Definition(words) => words.len(),
        }
    }
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
