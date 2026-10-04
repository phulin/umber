//! The actual current input level and its suspended owners.

use super::{ArgumentArena, InputLevel};
use core::hash::{Hash, Hasher};
use tex_state::token::TracedTokenWord;

/// One live cursor; the vector contains only suspended input levels.
pub(crate) struct InputStack<G> {
    pub(crate) current: Option<InputLevel<G>>,
    pub(super) suspended: Vec<InputLevel<G>>,
    pub(crate) words: Vec<TracedTokenWord>,
    pub(crate) arguments: ArgumentArena<G>,
    pub(crate) parameter_count: usize,
    pub(super) occupied_source_buffer_slots: usize,
    pub(super) context_epoch: u64,
}

impl<G> Default for InputStack<G> {
    fn default() -> Self {
        Self {
            current: None,
            suspended: Vec::new(),
            words: Vec::new(),
            arguments: ArgumentArena::default(),
            parameter_count: 0,
            occupied_source_buffer_slots: 0,
            context_epoch: 0,
        }
    }
}

impl<G> InputStack<G> {
    #[inline(always)]
    pub(crate) fn len(&self) -> usize {
        self.suspended.len() + usize::from(self.current.is_some())
    }

    /// Cold semantic coordinates retain oldest-to-current level order.
    pub(crate) fn level(&self, index: usize) -> Option<&InputLevel<G>> {
        if index == self.suspended.len() {
            self.current.as_ref()
        } else {
            self.suspended.get(index)
        }
    }

    pub(crate) fn level_mut(&mut self, index: usize) -> Option<&mut InputLevel<G>> {
        if index == self.suspended.len() {
            self.current.as_mut()
        } else {
            self.suspended.get_mut(index)
        }
    }

    pub(crate) fn iter(&self) -> InputLevels<'_, G> {
        InputLevels {
            suspended: self.suspended.iter(),
            current: self.current.as_ref(),
        }
    }

    pub(crate) fn iter_mut(&mut self) -> impl DoubleEndedIterator<Item = &mut InputLevel<G>> {
        self.suspended.iter_mut().chain(self.current.iter_mut())
    }
}

/// Borrowed traversal of actual owners, without a temporary row vector.
pub(crate) struct InputLevels<'a, G> {
    suspended: core::slice::Iter<'a, InputLevel<G>>,
    current: Option<&'a InputLevel<G>>,
}

impl<'a, G> Iterator for InputLevels<'a, G> {
    type Item = &'a InputLevel<G>;

    fn next(&mut self) -> Option<Self::Item> {
        self.suspended.next().or_else(|| self.current.take())
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let len = self.len();
        (len, Some(len))
    }
}

impl<G> DoubleEndedIterator for InputLevels<'_, G> {
    fn next_back(&mut self) -> Option<Self::Item> {
        self.current.take().or_else(|| self.suspended.next_back())
    }
}

impl<G> ExactSizeIterator for InputLevels<'_, G> {
    fn len(&self) -> usize {
        self.suspended.len() + usize::from(self.current.is_some())
    }
}

impl<G: core::fmt::Debug> core::fmt::Debug for InputStack<G> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("InputStack")
            .field("suspended", &self.suspended)
            .field("current", &self.current)
            .finish_non_exhaustive()
    }
}

impl<G: PartialEq> PartialEq for InputStack<G> {
    fn eq(&self, other: &Self) -> bool {
        self.iter().eq(other.iter())
            && self.words == other.words
            && self.parameter_count == other.parameter_count
            && self.arguments == other.arguments
    }
}

impl<G: Eq> Eq for InputStack<G> {}

impl<G: Hash> Hash for InputStack<G> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.len().hash(state);
        for level in self.iter() {
            level.hash(state);
        }
        self.words.hash(state);
        self.arguments.hash(state);
        self.parameter_count.hash(state);
    }
}
