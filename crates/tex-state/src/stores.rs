//! Coarse revision-generation owner and admitted state views.

use crate::definition_arena::{DefinitionAllocationError, DefinitionId, DefinitionView};
use crate::durable_arena::{
    DurableAllocationError, GlueId, ProvenanceId, TokenListBuilder, TokenListId, TokenListView,
};
use crate::env::{
    AcceptedDenseStateTail, DenseState, DenseStateCursor, DynamicMemoryRoot, StateError,
};
use crate::generation::{
    AcceptedGenerationTail, Generation, GenerationCursor, GenerationOwner, GenerationRetirement,
};
use crate::glue::GlueSpec;
use crate::provenance::OriginRecord;
use crate::token::TokenWord;
use std::sync::{RwLockReadGuard, RwLockWriteGuard};

#[cfg(test)]
#[path = "stores/tests.rs"]
mod tests;

/// Every mutable and immutable state owner for one revision generation.
pub(crate) struct StateCore<G> {
    generation: GenerationOwner<G>,
    state: DenseState<G>,
}

pub(crate) struct AcceptedStateCoreTail<G> {
    dense: AcceptedDenseStateTail<G>,
    generation: AcceptedGenerationTail,
}

impl<G> StateCore<G> {
    /// Selects semantic identities for immutable format payloads before the
    /// destination publishes them. Dense cells are deliberately enabled only
    /// after materialization, because format installation writes those banks
    /// directly and their exact root must be seeded from the completed image.
    pub(crate) fn prepare_format_reachable_state_identity(&mut self) -> bool {
        self.generation.generation_mut().enable_semantic_identity()
    }

    pub(crate) fn enable_reachable_state_identity(&mut self) -> bool {
        let generation = self.generation.generation_mut().enable_semantic_identity();
        if !generation || !self.state.enable_reachable_state_identity() {
            return false;
        }
        true
    }

    pub(crate) fn reachable_state_identity_root(&self) -> Option<u64> {
        self.state.reachable_state_identity_root()
    }

    pub(crate) fn checkpoint_retained_bytes(&self) -> usize {
        let (variable, dynamic) = self.memory_accounting().words(false);
        std::mem::size_of::<Self>()
            .saturating_add(
                variable
                    .saturating_add(dynamic)
                    .saturating_mul(std::mem::size_of::<usize>()),
            )
            .saturating_add(self.state.journal_retained_bytes())
    }

    #[must_use]
    pub(crate) fn generation_cursor(&self) -> GenerationCursor {
        self.generation.generation().cursor()
    }

    pub(crate) fn restore_generation_cursor(&mut self, cursor: GenerationCursor) {
        self.generation.generation_mut().restore_cursor(cursor);
    }

    pub(crate) fn validates_generation_cursor(&self, cursor: GenerationCursor) -> bool {
        self.generation.generation().validates_cursor(cursor)
    }

    pub(crate) fn capture_format_values(
        &self,
        extra_roots: impl IntoIterator<Item = DynamicMemoryRoot<G>>,
    ) -> (
        Vec<crate::format::schema::FormatDefinition>,
        Vec<Vec<u32>>,
        Vec<crate::format::schema::FormatGlue>,
    ) {
        let generation = self.generation.generation();
        let mut definitions = vec![None; generation.definitions().len()];
        let mut token_lists = vec![None; generation.token_lists().len()];
        let mut capture_root = |root| match root {
            DynamicMemoryRoot::Definition(definition) => {
                definitions[definition.format_index() as usize] = Some(definition.capture_format());
            }
            DynamicMemoryRoot::TokenList(tokens) => {
                token_lists[tokens.format_index() as usize] = Some(tokens.capture_format());
            }
        };
        self.state.visit_dynamic_memory_roots(&mut capture_root);
        for root in extra_roots {
            capture_root(root);
        }
        (
            definitions
                .into_iter()
                .map(|row| {
                    row.unwrap_or(crate::format::schema::FormatDefinition {
                        parameter_text: Vec::new(),
                        replacement_text: Vec::new(),
                    })
                })
                .collect(),
            token_lists
                .into_iter()
                .map(|row| row.unwrap_or_default())
                .collect(),
            generation.glue().capture_format_rows(),
        )
    }

    pub(crate) fn new(generation: Generation<G>) -> Result<Self, StateError> {
        Ok(Self {
            generation: GenerationOwner::new(generation),
            state: DenseState::new()?,
        })
    }

    #[must_use]
    pub(crate) fn memory_accounting(&self) -> crate::memory_accounting::MemoryAccounting {
        self.generation.generation().memory_accounting()
    }

    /// Validates and borrows the one matching generation bundle once per
    /// command episode. Hot reads through the returned view do no owner work.
    pub(crate) fn admit(&self) -> AdmittedState<'_, G> {
        AdmittedState {
            generation: self.generation.generation(),
            state: &self.state,
        }
    }

    /// Mutable episode admission. All state writes and durable publication
    /// remain behind this unique aggregate borrow.
    pub(crate) fn admit_mut(&mut self) -> Result<AdmittedStateMut<'_, G>, StateError> {
        Ok(AdmittedStateMut {
            generation: self.generation.generation_mut(),
            state: &mut self.state,
        })
    }

    pub(crate) const fn state_mut(&mut self) -> &mut DenseState<G> {
        &mut self.state
    }

    #[must_use]
    pub(crate) const fn state(&self) -> &DenseState<G> {
        &self.state
    }

    pub(crate) fn begin_checkpoint_candidate(
        &mut self,
        journal: crate::journal::JournalCursor<G>,
        dense: DenseStateCursor,
        generation: GenerationCursor,
    ) -> Result<AcceptedStateCoreTail<G>, StateError> {
        self.state.validate_restore(journal)?;
        if !self.generation.generation().validates_cursor(generation) {
            return Err(StateError::InvalidCursor);
        }
        let generation_tail = self
            .generation
            .generation_mut()
            .begin_checkpoint_candidate(generation);
        let dense = self.state.begin_checkpoint_candidate(journal, dense)?;
        Ok(AcceptedStateCoreTail {
            dense,
            generation: generation_tail,
        })
    }

    pub(crate) fn reject_checkpoint_candidate(
        &mut self,
        journal: crate::journal::JournalCursor<G>,
        dense: DenseStateCursor,
        generation: GenerationCursor,
        tail: AcceptedStateCoreTail<G>,
    ) -> Result<(), StateError> {
        self.state
            .reject_checkpoint_candidate(journal, dense, tail.dense)?;
        self.generation
            .generation_mut()
            .reject_checkpoint_candidate(generation, tail.generation);
        Ok(())
    }

    pub(crate) fn accept_checkpoint_candidate(&mut self, tail: AcceptedStateCoreTail<G>) {
        self.state.accept_checkpoint_candidate(tail.dense);
        drop(tail.generation);
    }

    /// Retires the complete generation after all admitted borrows end.
    pub(crate) fn retire(self) -> Result<StateCoreRetirement, StateError> {
        let journal_entries = self.state.journal_len();
        let allocated_overflow_pages = self.state.allocated_overflow_pages();
        let durable_node_lists = 0;
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
    pub(crate) fn can_retire_after_dropping(&self, owner: &GenerationOwner<G>) -> bool {
        self.generation.is_unique() || self.generation.is_owned_only_by(owner)
    }
}

/// Immutable, already-admitted hot view.
pub(crate) struct AdmittedState<'a, G> {
    generation: RwLockReadGuard<'a, Generation<G>>,
    state: &'a DenseState<G>,
}

impl<'a, G> AdmittedState<'a, G> {
    #[must_use]
    pub(crate) const fn state(&self) -> &'a DenseState<G> {
        self.state
    }

    #[inline(always)]
    pub(crate) fn definition(&self, id: DefinitionId<G>) -> DefinitionView<G> {
        self.generation.definitions().get(id)
    }

    #[inline(always)]
    pub(crate) fn token_list(&self, id: TokenListId<G>) -> TokenListView<G> {
        self.generation.token_lists().get(id)
    }

    #[inline(always)]
    pub(crate) fn glue(&self, id: GlueId<G>) -> GlueSpec {
        self.generation.glue().get(id)
    }

    pub(crate) fn definition_identity_policy(&self) -> crate::DefinitionIdentityPolicy {
        self.generation.definitions().identity_policy()
    }

    #[cfg(test)]
    pub(crate) fn provenance(&self, id: ProvenanceId<G>) -> OriginRecord {
        self.generation.provenance().get(id)
    }
}

/// Unique admitted view used for assignment and commit publication.
pub(crate) struct AdmittedStateMut<'a, G> {
    generation: RwLockWriteGuard<'a, Generation<G>>,
    state: &'a mut DenseState<G>,
}

impl<'a, G> AdmittedStateMut<'a, G> {
    pub(crate) const fn state_ref(&self) -> &DenseState<G> {
        self.state
    }

    pub(crate) fn state(&mut self) -> &mut DenseState<G> {
        self.state
    }

    #[inline(always)]
    pub(crate) fn definition(&self, id: DefinitionId<G>) -> DefinitionView<G> {
        self.generation.definitions().get(id)
    }

    #[inline(always)]
    pub(crate) fn token_list(&self, id: TokenListId<G>) -> TokenListView<G> {
        self.generation.token_lists().get(id)
    }

    #[inline(always)]
    pub(crate) fn glue(&self, id: GlueId<G>) -> GlueSpec {
        self.generation.glue().get(id)
    }

    #[inline(always)]
    pub(crate) fn provenance(&self, id: ProvenanceId<G>) -> OriginRecord {
        self.generation.provenance().get(id)
    }

    pub(crate) fn provenance_coordinate_at(&self, index: u32) -> Option<ProvenanceId<G>> {
        self.generation.provenance().coordinate_at(index)
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

    pub(crate) fn allocate_definition_from_iter<Parameters, Replacement>(
        &mut self,
        parameter_text: Parameters,
        replacement_text: Replacement,
    ) -> Result<DefinitionId<G>, DefinitionAllocationError>
    where
        Parameters: ExactSizeIterator<Item = TokenWord>,
        Replacement: ExactSizeIterator<Item = TokenWord>,
    {
        self.generation
            .definitions_mut()
            .allocate_from_iter(parameter_text, replacement_text)
    }

    pub(crate) fn publish_definition_builder(
        &mut self,
        builder: &crate::DefinitionBuilder,
    ) -> Result<DefinitionId<G>, DefinitionAllocationError> {
        self.generation.definitions_mut().publish(builder)
    }

    pub(crate) fn definition_identity_policy(&self) -> crate::DefinitionIdentityPolicy {
        self.generation.definitions().identity_policy()
    }

    pub(crate) fn allocate_token_list(
        &mut self,
        words: &[TokenWord],
    ) -> Result<TokenListId<G>, DurableAllocationError> {
        self.generation.token_lists_mut().allocate(words)
    }

    pub(crate) fn begin_token_list_builder(
        &mut self,
    ) -> Result<TokenListBuilder<G>, DurableAllocationError> {
        self.generation.token_lists_mut().begin_builder()
    }

    pub(crate) fn append_token_list_word(
        &mut self,
        builder: &TokenListBuilder<G>,
        word: TokenWord,
    ) -> Result<(), DurableAllocationError> {
        self.generation
            .token_lists_mut()
            .push_builder_word(builder, word)
    }

    pub(crate) fn seal_token_list_builder(
        &mut self,
        builder: TokenListBuilder<G>,
    ) -> Result<TokenListId<G>, DurableAllocationError> {
        self.generation.token_lists_mut().seal_builder(builder)
    }

    pub(crate) fn discard_token_list_builder(
        &mut self,
        builder: TokenListBuilder<G>,
    ) -> Result<(), DurableAllocationError> {
        self.generation.token_lists_mut().discard_builder(builder)
    }

    pub(crate) fn allocate_glue(
        &mut self,
        value: GlueSpec,
    ) -> Result<GlueId<G>, DurableAllocationError> {
        self.generation.glue_mut().allocate(value)
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
        definitions: &[crate::universe::DefinitionPromotion],
        token_lists: &[crate::universe::TokenListPromotion<'_>],
        glue_values: &[GlueSpec],
        provenance: &[OriginRecord],
    ) -> Result<crate::universe::PromotionReceipt<G>, crate::universe::PromotionError> {
        self.promote_value_streams(
            definitions.iter().map(|definition| definition.builder()),
            token_lists.iter().map(|list| list.words.iter().copied()),
            glue_values.iter().copied(),
            provenance.iter().copied(),
        )
    }

    /// Publishes a prevalidated batch from repeatable borrowed source walks.
    ///
    /// The caller keeps each source in its original owner through reservation.
    /// Once all capacity is reserved, publication streams directly into the
    /// destination publishers and cannot fail. No batch-local payload owner is
    /// constructed merely to cross this boundary.
    pub(crate) fn promote_value_streams<'source, Definitions, TokenLists, Words, Glue, Provenance>(
        &mut self,
        definitions: Definitions,
        token_lists: TokenLists,
        glue_values: Glue,
        provenance: Provenance,
    ) -> Result<crate::universe::PromotionReceipt<G>, crate::universe::PromotionError>
    where
        Definitions: Clone + Iterator<Item = &'source crate::DefinitionBuilder>,
        TokenLists: Clone + Iterator<Item = Words>,
        Words: ExactSizeIterator<Item = TokenWord>,
        Glue: Clone + Iterator<Item = GlueSpec>,
        Provenance: Clone + Iterator<Item = OriginRecord>,
    {
        let definition_count = definitions.clone().count();
        let token_list_count = token_lists.clone().count();
        let glue_count = glue_values.clone().count();
        let provenance_count = provenance.clone().count();
        let definition_words = definitions
            .clone()
            .try_fold(0usize, |total, definition| {
                total.checked_add(definition.words().len())
            })
            .ok_or(crate::universe::PromotionError::CapacityOverflow)?;
        let token_words = token_lists
            .clone()
            .try_fold(0usize, |total, words| total.checked_add(words.len()))
            .ok_or(crate::universe::PromotionError::CapacityOverflow)?;

        let definitions_arena = self.generation.definitions();
        for definition in definitions.clone() {
            definitions_arena.validate_builder(definition)?;
        }

        // Reserve every destination before the first logical length changes.
        // Once these calls succeed, the individual arena allocators cannot
        // allocate or fail for this validated batch.
        self.generation
            .definitions_mut()
            .reserve_batch(definition_count, definition_words)?;
        self.generation
            .token_lists_mut()
            .reserve_batch(token_list_count, token_words)?;
        self.generation.glue_mut().reserve_batch(glue_count)?;
        self.generation
            .provenance_mut()
            .reserve_batch(provenance_count)?;

        let mut promoted_definitions = Vec::new();
        let mut promoted_token_lists = Vec::new();
        let mut promoted_glue = Vec::new();
        let mut promoted_provenance = Vec::new();
        promoted_definitions
            .try_reserve_exact(definition_count)
            .map_err(|_| crate::universe::PromotionError::AllocationFailed)?;
        promoted_token_lists
            .try_reserve_exact(token_list_count)
            .map_err(|_| crate::universe::PromotionError::AllocationFailed)?;
        promoted_glue
            .try_reserve_exact(glue_count)
            .map_err(|_| crate::universe::PromotionError::AllocationFailed)?;
        promoted_provenance
            .try_reserve_exact(provenance_count)
            .map_err(|_| crate::universe::PromotionError::AllocationFailed)?;

        for definition in definitions {
            promoted_definitions.push(
                self.generation
                    .definitions_mut()
                    .publish(definition)
                    .expect("validated destination-policy definition publication"),
            );
        }
        for words in token_lists {
            promoted_token_lists.push(
                self.generation
                    .token_lists_mut()
                    .allocate_from_iter(words)
                    .expect("the complete token-list promotion batch was reserved"),
            );
        }
        for glue in glue_values {
            promoted_glue.push(
                self.generation
                    .glue_mut()
                    .allocate(glue)
                    .expect("the complete glue promotion batch was reserved"),
            );
        }
        for record in provenance {
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

    /// Admits validated format rows directly into their final arenas.
    ///
    /// Dead compatibility rows in a detached image remain validated wire data
    /// but do not become live shared owners. All final capacities and
    /// relocation tables are reserved before the first value is published.
    pub(crate) fn promote_format_values(
        &mut self,
        definitions: Vec<crate::format::schema::FormatDefinition>,
        live_definitions: Vec<bool>,
        token_lists: Vec<Vec<u32>>,
        glue_values: Vec<GlueSpec>,
    ) -> Result<crate::universe::FormatPromotionReceipt<G>, crate::universe::PromotionError> {
        if definitions.len() != live_definitions.len() {
            return Err(crate::universe::PromotionError::CapacityOverflow);
        }
        let definition_rows = live_definitions.iter().filter(|&&live| live).count();
        let definition_words = definitions
            .iter()
            .zip(&live_definitions)
            .filter(|(_, live)| **live)
            .try_fold(0usize, |total, (definition, _)| {
                total
                    .checked_add(definition.parameter_text.len())
                    .and_then(|total| total.checked_add(definition.replacement_text.len()))
            })
            .ok_or(crate::universe::PromotionError::CapacityOverflow)?;
        let token_words = token_lists
            .iter()
            .try_fold(0usize, |total, words| total.checked_add(words.len()))
            .ok_or(crate::universe::PromotionError::CapacityOverflow)?;

        let policy = self.generation.definitions().identity_policy();
        let mut definition_builders = Vec::new();
        definition_builders
            .try_reserve_exact(definitions.len())
            .map_err(|_| crate::universe::PromotionError::AllocationFailed)?;
        for (definition, live) in definitions.iter().zip(&live_definitions) {
            definition_builders.push(if *live {
                let mut builder = crate::DefinitionBuilder::new(policy);
                for &word in &definition.parameter_text {
                    builder.push_parameter(TokenWord::from_raw(word))?;
                }
                builder.finish_parameters()?;
                for &word in &definition.replacement_text {
                    builder.push_replacement(TokenWord::from_raw(word))?;
                }
                builder.seal()?;
                Some(builder)
            } else {
                None
            });
        }

        self.generation
            .definitions_mut()
            .reserve_batch(definition_rows, definition_words)?;
        self.generation
            .token_lists_mut()
            .reserve_batch(token_lists.len(), token_words)?;
        self.generation
            .glue_mut()
            .reserve_batch(glue_values.len())?;

        let mut promoted_definitions = Vec::new();
        promoted_definitions
            .try_reserve_exact(definitions.len())
            .map_err(|_| crate::universe::PromotionError::AllocationFailed)?;
        promoted_definitions.resize(definitions.len(), None);
        let mut promoted_token_lists = Vec::new();
        promoted_token_lists
            .try_reserve_exact(token_lists.len())
            .map_err(|_| crate::universe::PromotionError::AllocationFailed)?;
        let mut promoted_glue = Vec::new();
        promoted_glue
            .try_reserve_exact(glue_values.len())
            .map_err(|_| crate::universe::PromotionError::AllocationFailed)?;

        for (row, definition) in definition_builders.iter().enumerate() {
            if !live_definitions[row] {
                continue;
            }
            let id = self
                .generation
                .definitions_mut()
                .publish(definition.as_ref().expect("live builder was staged"))
                .expect("validated destination-policy format publication");
            promoted_definitions[row] = Some(id);
        }
        for words in token_lists {
            promoted_token_lists.push(
                self.generation
                    .token_lists_mut()
                    .allocate_from_iter(words.into_iter().map(TokenWord::from_raw))
                    .expect("the complete format token-list batch was reserved"),
            );
        }
        for glue in glue_values {
            promoted_glue.push(
                self.generation
                    .glue_mut()
                    .allocate(glue)
                    .expect("the complete format glue batch was reserved"),
            );
        }

        Ok(crate::universe::FormatPromotionReceipt {
            definitions: promoted_definitions,
            token_lists: promoted_token_lists,
            glue: promoted_glue,
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

impl StateCoreRetirement {
    pub(crate) const fn transferred() -> Self {
        Self {
            generation: GenerationRetirement {
                definitions: 0,
                token_lists: 0,
                glue_values: 0,
                provenance_records: 0,
            },
            durable_node_lists: 0,
            journal_entries: 0,
            allocated_overflow_pages: 0,
        }
    }
}
