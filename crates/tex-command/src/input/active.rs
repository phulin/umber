//! The authoritative stack of actual input owners.

use super::{ArgumentArena, InputLevel};
use core::hash::{Hash, Hasher};
use tex_state::token::TracedTokenWord;

/// Oldest-to-current input owners, borrowed directly at physical read boundaries.
pub(crate) struct InputStack<G> {
    pub(crate) rows: Vec<InputLevel<G>>,
    pub(crate) words: Vec<TracedTokenWord>,
    pub(crate) arguments: ArgumentArena<G>,
    pub(crate) parameter_count: usize,
    pub(super) occupied_source_buffer_slots: usize,
    pub(super) context_epoch: u64,
}

impl<G> Default for InputStack<G> {
    fn default() -> Self {
        Self {
            rows: Vec::new(),
            words: Vec::new(),
            arguments: ArgumentArena::default(),
            parameter_count: 0,
            occupied_source_buffer_slots: 0,
            context_epoch: 0,
        }
    }
}

impl<G: core::fmt::Debug> core::fmt::Debug for InputStack<G> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("InputStack")
            .field("rows", &self.rows)
            .finish_non_exhaustive()
    }
}

impl<G: PartialEq> PartialEq for InputStack<G> {
    fn eq(&self, other: &Self) -> bool {
        self.rows == other.rows
            && self.words == other.words
            && self.parameter_count == other.parameter_count
            && self.arguments == other.arguments
    }
}

impl<G: Eq> Eq for InputStack<G> {}

impl<G: Hash> Hash for InputStack<G> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.rows.hash(state);
        self.words.hash(state);
        self.arguments.hash(state);
        self.parameter_count.hash(state);
    }
}
