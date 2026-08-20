//! Coarse revision-generation owner and admitted state views.

use crate::definition_arena::{DefinitionAllocationError, DefinitionId, DefinitionView};
use crate::durable_arena::{DurableAllocationError, GlueId, ProvenanceId, TokenListId};
use crate::env::{DenseState, StateError};
use crate::generation::{Generation, GenerationOwner, GenerationRetirement};
use crate::glue::GlueSpec;
use crate::node_arena::{DurableListId, DurableNodeArena, NodeArenaError, NodeList};
use crate::provenance::OriginRecord;
use crate::token::TokenWord;

#[cfg(test)]
#[path = "stores/tests.rs"]
mod tests;

/// Every mutable and immutable state owner for one revision generation.
pub(crate) struct StateCore<G> {
    generation: GenerationOwner<G>,
    nodes: DurableNodeArena<G>,
    state: DenseState<G>,
}

impl<G> StateCore<G> {
    pub(crate) fn new(generation: Generation<G>) -> Result<Self, StateError> {
        Ok(Self {
            generation: GenerationOwner::new(generation),
            nodes: DurableNodeArena::new(),
            state: DenseState::new()?,
        })
    }

    /// Validates and borrows the one matching generation bundle once per
    /// command episode. Hot reads through the returned view do no owner work.
    #[must_use]
    pub(crate) fn admit(&self) -> AdmittedState<'_, G> {
        AdmittedState {
            generation: self.generation.generation(),
            nodes: &self.nodes,
            state: &self.state,
        }
    }

    /// Mutable episode admission. All state writes and durable publication
    /// remain behind this unique aggregate borrow.
    #[must_use]
    pub(crate) fn admit_mut(&mut self) -> Result<AdmittedStateMut<'_, G>, StateError> {
        Ok(AdmittedStateMut {
            generation: self
                .generation
                .generation_mut()
                .ok_or(StateError::GenerationInUse)?,
            nodes: &mut self.nodes,
            state: &mut self.state,
        })
    }

    pub(crate) const fn state_mut(&mut self) -> &mut DenseState<G> {
        &mut self.state
    }

    /// Retires the complete generation after all admitted borrows end.
    #[must_use]
    pub(crate) fn retire(self) -> Result<StateCoreRetirement, StateError> {
        let journal_entries = self.state.journal_len();
        let allocated_overflow_pages = self.state.allocated_overflow_pages();
        let durable_node_lists = self.nodes.len();
        let generation = self
            .generation
            .retire()
            .map_err(|_| StateError::GenerationInUse)?;
        Ok(StateCoreRetirement {
            generation,
            durable_node_lists,
            journal_entries,
            allocated_overflow_pages,
        })
    }

    #[must_use]
    pub(crate) fn generation_owner(&self) -> GenerationOwner<G> {
        self.generation.clone()
    }

    #[must_use]
    pub(crate) fn owns_generation(&self, owner: &GenerationOwner<G>) -> bool {
        self.generation.same_generation(owner)
    }

    #[must_use]
    pub(crate) fn can_retire(&self) -> bool {
        self.generation.is_unique()
    }
}

/// Immutable, already-admitted hot view.
pub(crate) struct AdmittedState<'a, G> {
    generation: &'a Generation<G>,
    nodes: &'a DurableNodeArena<G>,
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

    #[inline(always)]
    pub(crate) fn node_list(
        &self,
        id: DurableListId<G>,
    ) -> Result<NodeList<'a, G, GlueId<G>, TokenListId<G>>, NodeArenaError> {
        self.nodes.get(id)
    }

    pub(crate) fn provenance(&self, id: ProvenanceId<G>) -> OriginRecord {
        self.generation.provenance().get(id)
    }
}

/// Unique admitted view used for assignment and commit publication.
pub(crate) struct AdmittedStateMut<'a, G> {
    generation: &'a mut Generation<G>,
    nodes: &'a mut DurableNodeArena<G>,
    state: &'a mut DenseState<G>,
}

impl<'a, G> AdmittedStateMut<'a, G> {
    pub(crate) fn state(&mut self) -> &mut DenseState<G> {
        self.state
    }

    pub(crate) fn nodes_mut(&mut self) -> &mut DurableNodeArena<G> {
        self.nodes
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

    pub(crate) fn node_list(
        &self,
        id: DurableListId<G>,
    ) -> Result<NodeList<'_, G, GlueId<G>, TokenListId<G>>, NodeArenaError> {
        self.nodes.get(id)
    }

    pub(crate) fn allocate_provenance(
        &mut self,
        value: OriginRecord,
    ) -> Result<ProvenanceId<G>, DurableAllocationError> {
        self.generation.provenance_mut().allocate(value)
    }

    /// Promotes one already-validated batch while this unique admitted borrow
    /// prevents any consumer from observing partial destination state.
    pub(crate) fn promote_values(
        &mut self,
        definitions: &[crate::universe::DefinitionPromotion<'_>],
        token_lists: &[crate::universe::TokenListPromotion<'_>],
        glue_values: &[GlueSpec],
        provenance: &[OriginRecord],
    ) -> Result<crate::universe::PromotionReceipt<G>, crate::universe::PromotionError> {
        let definition_words = definitions.iter().try_fold(0usize, |total, definition| {
            total
                .checked_add(definition.parameter_text.len())
                .and_then(|total| total.checked_add(definition.replacement_text.len()))
        });
        let token_words = token_lists
            .iter()
            .try_fold(0usize, |total, list| total.checked_add(list.words.len()));
        let Some(definition_words) = definition_words else {
            return Err(crate::universe::PromotionError::CapacityOverflow);
        };
        let Some(token_words) = token_words else {
            return Err(crate::universe::PromotionError::CapacityOverflow);
        };

        // Reserve every destination before the first logical length changes.
        // Once these calls succeed, the individual arena allocators cannot
        // allocate or fail for this validated batch.
        self.generation
            .definitions_mut()
            .reserve_batch(definitions.len(), definition_words)?;
        self.generation
            .token_lists_mut()
            .reserve_batch(token_lists.len(), token_words)?;
        self.generation
            .glue_mut()
            .reserve_batch(glue_values.len())?;
        self.generation
            .provenance_mut()
            .reserve_batch(provenance.len())?;

        let mut promoted_definitions = Vec::new();
        let mut promoted_token_lists = Vec::new();
        let mut promoted_glue = Vec::new();
        let mut promoted_provenance = Vec::new();
        promoted_definitions
            .try_reserve_exact(definitions.len())
            .map_err(|_| crate::universe::PromotionError::AllocationFailed)?;
        promoted_token_lists
            .try_reserve_exact(token_lists.len())
            .map_err(|_| crate::universe::PromotionError::AllocationFailed)?;
        promoted_glue
            .try_reserve_exact(glue_values.len())
            .map_err(|_| crate::universe::PromotionError::AllocationFailed)?;
        promoted_provenance
            .try_reserve_exact(provenance.len())
            .map_err(|_| crate::universe::PromotionError::AllocationFailed)?;

        for definition in definitions {
            promoted_definitions.push(
                self.generation
                    .definitions_mut()
                    .allocate(definition.parameter_text, definition.replacement_text)
                    .expect("the complete definition promotion batch was reserved"),
            );
        }
        for list in token_lists {
            promoted_token_lists.push(
                self.generation
                    .token_lists_mut()
                    .allocate(list.words)
                    .expect("the complete token-list promotion batch was reserved"),
            );
        }
        for &glue in glue_values {
            promoted_glue.push(
                self.generation
                    .glue_mut()
                    .allocate(glue)
                    .expect("the complete glue promotion batch was reserved"),
            );
        }
        for &record in provenance {
            promoted_provenance.push(
                self.generation
                    .provenance_mut()
                    .allocate(record)
                    .expect("the complete provenance promotion batch was reserved"),
            );
        }

        Ok(crate::universe::PromotionReceipt {
            definitions: promoted_definitions,
            token_lists: promoted_token_lists,
            glue: promoted_glue,
            provenance: promoted_provenance,
        })
    }
}

/// Evidence returned when a whole generation bundle is released.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct StateCoreRetirement {
    pub(crate) generation: GenerationRetirement,
    pub(crate) durable_node_lists: usize,
    pub(crate) journal_entries: usize,
    pub(crate) allocated_overflow_pages: usize,
}
