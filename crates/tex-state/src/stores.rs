//! Coarse revision-generation owner and admitted state views.

use crate::definition_arena::{DefinitionAllocationError, DefinitionId, DefinitionView};
use crate::durable_arena::{DurableAllocationError, GlueId, TokenListId};
use crate::env::{DenseState, StateError};
use crate::generation::{Generation, GenerationRetirement};
use crate::glue::GlueSpec;
use crate::token::TokenWord;

#[cfg(test)]
#[path = "stores/tests.rs"]
mod tests;

/// Every mutable and immutable state owner for one revision generation.
pub(crate) struct StateCore<G> {
    generation: Generation<G>,
    state: DenseState<G>,
}

impl<G> StateCore<G> {
    pub(crate) fn new(generation: Generation<G>) -> Result<Self, StateError> {
        Ok(Self {
            generation,
            state: DenseState::new()?,
        })
    }

    /// Validates and borrows the one matching generation bundle once per
    /// command episode. Hot reads through the returned view do no owner work.
    #[must_use]
    pub(crate) const fn admit(&self) -> AdmittedState<'_, G> {
        AdmittedState {
            generation: &self.generation,
            state: &self.state,
        }
    }

    /// Mutable episode admission. All state writes and durable publication
    /// remain behind this unique aggregate borrow.
    #[must_use]
    pub(crate) const fn admit_mut(&mut self) -> AdmittedStateMut<'_, G> {
        AdmittedStateMut {
            generation: &mut self.generation,
            state: &mut self.state,
        }
    }

    pub(crate) const fn state_mut(&mut self) -> &mut DenseState<G> {
        &mut self.state
    }

    /// Retires the complete generation after all admitted borrows end.
    #[must_use]
    pub(crate) fn retire(self) -> StateCoreRetirement {
        let journal_entries = self.state.journal_len();
        let allocated_overflow_pages = self.state.allocated_overflow_pages();
        let generation = self.generation.retire();
        StateCoreRetirement {
            generation,
            journal_entries,
            allocated_overflow_pages,
        }
    }
}

/// Immutable, already-admitted hot view.
pub(crate) struct AdmittedState<'a, G> {
    generation: &'a Generation<G>,
    state: &'a DenseState<G>,
}

impl<'a, G> AdmittedState<'a, G> {
    #[must_use]
    pub(crate) const fn state(&self) -> &'a DenseState<G> {
        self.state
    }

    #[inline(always)]
    pub(crate) fn definition(&self, id: DefinitionId<G>) -> DefinitionView<'a, G> {
        self.generation.definitions().get(id)
    }

    #[inline(always)]
    pub(crate) fn token_list(&self, id: TokenListId<G>) -> &'a [TokenWord] {
        self.generation.token_lists().get(id)
    }

    #[inline(always)]
    pub(crate) fn glue(&self, id: GlueId<G>) -> GlueSpec {
        self.generation.glue().get(id)
    }
}

/// Unique admitted view used for assignment and commit publication.
pub(crate) struct AdmittedStateMut<'a, G> {
    generation: &'a mut Generation<G>,
    state: &'a mut DenseState<G>,
}

impl<'a, G> AdmittedStateMut<'a, G> {
    pub(crate) fn state(&mut self) -> &mut DenseState<G> {
        self.state
    }

    pub(crate) fn allocate_definition(
        &mut self,
        parameter_text: &[TokenWord],
        replacement_text: &[TokenWord],
    ) -> Result<DefinitionId<G>, DefinitionAllocationError> {
        self.generation
            .definitions_mut()
            .allocate(parameter_text, replacement_text)
    }

    pub(crate) fn allocate_token_list(
        &mut self,
        words: &[TokenWord],
    ) -> Result<TokenListId<G>, DurableAllocationError> {
        self.generation.token_lists_mut().allocate(words)
    }

    pub(crate) fn allocate_glue(
        &mut self,
        value: GlueSpec,
    ) -> Result<GlueId<G>, DurableAllocationError> {
        self.generation.glue_mut().allocate(value)
    }
}

/// Evidence returned when a whole generation bundle is released.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct StateCoreRetirement {
    pub(crate) generation: GenerationRetirement,
    pub(crate) journal_entries: usize,
    pub(crate) allocated_overflow_pages: usize,
}
