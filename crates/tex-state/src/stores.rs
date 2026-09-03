//! Coarse revision-generation owner and admitted state views.

use crate::definition_arena::{
    DefinitionAllocationError, DefinitionRef, DefinitionView, ResidentMacroBody,
};
use crate::durable_arena::{
    DurableAllocationError, GlueId, ProvenanceId, TokenListBuilder, TokenListId, TokenListView,
};
use crate::env::{
    AcceptedDenseStateTail, DenseState, DenseStateCursor, DynamicMemoryRoot, StateError,
};
use crate::generation::{
    AcceptedGenerationTail, CheckpointGenerationOwner, Generation, GenerationCursor,
    GenerationOwner, GenerationRetirement,
};
use crate::glue::GlueSpec;
use crate::provenance::OriginRecord;
use crate::token::TokenWord;
use smallvec::SmallVec;
use std::sync::{RwLockReadGuard, RwLockWriteGuard};

#[cfg(test)]
#[path = "stores/tests.rs"]
mod tests;

trait DefinitionBatchItem {
    fn builder(&self) -> &crate::DefinitionBuilder;
    fn builder_mut(&mut self) -> &mut crate::DefinitionBuilder;
}

impl DefinitionBatchItem for crate::DefinitionBuilder {
    fn builder(&self) -> &crate::DefinitionBuilder {
        self
    }

    fn builder_mut(&mut self) -> &mut crate::DefinitionBuilder {
        self
    }
}

impl DefinitionBatchItem for crate::universe::DefinitionPromotion {
    fn builder(&self) -> &crate::DefinitionBuilder {
        self.builder()
    }

    fn builder_mut(&mut self) -> &mut crate::DefinitionBuilder {
        self.builder_mut()
    }
}

/// Every mutable and immutable state owner for one revision generation.
pub(crate) struct StateCore<G> {
    generation: GenerationOwner<G>,
    state: DenseState<G>,
}

pub(crate) struct AcceptedStateCoreTail<G> {
    dense: AcceptedDenseStateTail<G>,
    generation: AcceptedGenerationTail<G>,
}

type CapturedFormatValues<G> = (
    Vec<crate::format::schema::FormatDefinition>,
    Vec<(DefinitionRef<G>, u32)>,
    Vec<Vec<u32>>,
    Vec<crate::format::schema::FormatGlue>,
);

impl<G> StateCore<G> {
    pub(crate) fn capture_node_token_list(
        &self,
        key: crate::node::NodeTokenKey,
    ) -> Option<Vec<u32>> {
        self.generation
            .generation()
            .token_lists()
            .node_words(key)
            .map(|words| words.iter().map(|word| word.raw()).collect())
    }

    pub(crate) fn definition_meanings_match_accepted_checkpoint(
        &self,
        restart: crate::journal::JournalCursor<G>,
        prior: crate::journal::JournalCursor<G>,
        accepted: &AcceptedStateCoreTail<G>,
    ) -> bool {
        self.state.definition_meanings_match_accepted_checkpoint(
            restart,
            prior,
            |current, prior| {
                self.generation
                    .generation()
                    .current_and_accepted_definition_contents_equal(
                        &accepted.generation,
                        current,
                        prior,
                    )
            },
        )
    }

    /// Selects semantic identities for immutable format payloads before the
    /// destination publishes them. Dense cells are deliberately enabled only
    /// after materialization, because format installation writes those banks
    /// directly and their exact root must be seeded from the completed image.
    pub(crate) fn prepare_format_reachable_state_identity(&mut self) -> bool {
        self.generation.generation_mut().enable_semantic_identity()
    }

    pub(crate) fn enable_reachable_state_identity(&mut self) -> bool {
        let mut generation = self.generation.generation_mut();
        if !generation.enable_semantic_identity() {
            return false;
        }
        self.state.enable_reachable_state_identity()
    }

    pub(crate) fn reachable_state_identity_root(&self) -> Option<u64> {
        self.state.reachable_state_identity_root(|definition| {
            self.generation
                .generation()
                .definitions()
                .get(definition)
                .semantic_identity()
        })
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
    ) -> CapturedFormatValues<G> {
        let generation = self.generation.generation();
        let mut definitions = Vec::new();
        let mut definition_rows = Vec::new();
        let mut token_lists = vec![None; generation.token_lists().len()];
        let mut capture_root = |root| match root {
            DynamicMemoryRoot::Definition(definition) => {
                if definition_rows
                    .iter()
                    .any(|(candidate, _)| *candidate == definition)
                {
                    return;
                }
                let row = u32::try_from(definitions.len()).expect("format definitions fit u32");
                definitions.push(generation.definitions().get(definition).capture_format());
                definition_rows.push((definition, row));
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
            definitions,
            definition_rows,
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

    pub(crate) fn new_format(
        generation: Generation<G>,
        meaning_slots: usize,
    ) -> Result<Self, StateError> {
        Ok(Self {
            generation: GenerationOwner::new(generation),
            state: DenseState::new_format(meaning_slots)?,
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
        let dense = self.state.begin_checkpoint_candidate(journal, dense)?;
        let generation_tail = self
            .generation
            .generation_mut()
            .begin_checkpoint_candidate(generation);
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
        self.state
            .accept_checkpoint_candidate(tail.dense)
            .expect("validated state candidate is at a quiescent checkpoint boundary");
        self.generation
            .generation_mut()
            .accept_checkpoint_candidate(tail.generation);
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
    pub(crate) fn checkpoint_generation_owner(&self) -> CheckpointGenerationOwner<G> {
        self.generation.checkpoint_owner()
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
    pub(crate) fn definition(&self, id: DefinitionRef<G>) -> DefinitionView<'_, G> {
        self.generation.definitions().get(id)
    }

    #[inline(always)]
    pub(crate) fn token_list(&self, id: TokenListId<G>) -> TokenListView<G> {
        self.generation.token_lists().get(id)
    }

    #[inline(always)]
    pub(crate) fn node_token_words(&self, key: crate::node::NodeTokenKey) -> Option<&[TokenWord]> {
        self.generation.token_lists().node_words(key)
    }

    pub(crate) fn node_token_key(&self, id: &TokenListId<G>) -> Option<crate::node::NodeTokenKey> {
        self.generation.token_lists().node_key(id)
    }

    #[inline(always)]
    pub(crate) fn glue(&self, id: GlueId<G>) -> GlueSpec {
        self.generation.glue().get(id)
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

    pub(crate) fn assign_meaning(
        &mut self,
        symbol: crate::interner::Symbol,
        value: crate::MeaningWord<G>,
        scope: crate::AssignmentScope,
    ) -> Result<(), StateError> {
        self.state.assign_meaning(symbol, value, scope)
    }

    #[inline(always)]
    pub(crate) fn definition(&self, id: DefinitionRef<G>) -> DefinitionView<'_, G> {
        self.generation.definitions().get(id)
    }

    pub(crate) fn definition_contents_equal(
        &self,
        left: DefinitionRef<G>,
        right: DefinitionRef<G>,
    ) -> bool {
        self.generation.definitions().contents_equal(left, right)
    }

    pub(crate) fn admit_macro_body(
        &self,
        id: DefinitionRef<G>,
    ) -> Option<(
        crate::macro_definition::MacroParameterPattern,
        usize,
        ResidentMacroBody<G>,
    )> {
        self.generation.definitions().admit_macro_body(id)
    }

    pub(crate) fn admit_macro_definition(
        &self,
        id: DefinitionRef<G>,
    ) -> Option<crate::AdmittedMacroDefinition<G>> {
        self.generation.definitions().admit_macro_definition(id)
    }

    #[inline(always)]
    pub(crate) fn token_list(&self, id: TokenListId<G>) -> TokenListView<G> {
        self.generation.token_lists().get(id)
    }

    #[inline(always)]
    pub(crate) fn node_token_words(&self, key: crate::node::NodeTokenKey) -> Option<&[TokenWord]> {
        self.generation.token_lists().node_words(key)
    }

    pub(crate) fn node_token_key(&self, id: &TokenListId<G>) -> Option<crate::node::NodeTokenKey> {
        self.generation.token_lists().node_key(id)
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
    ) -> Result<DefinitionRef<G>, DefinitionAllocationError> {
        self.generation
            .definitions_mut()
            .allocate(parameter_text, replacement_text)
    }

    pub(crate) fn allocate_definition_from_iter<Parameters, Replacement>(
        &mut self,
        parameter_text: Parameters,
        replacement_text: Replacement,
    ) -> Result<DefinitionRef<G>, DefinitionAllocationError>
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
        builder: &mut crate::DefinitionBuilder,
    ) -> Result<DefinitionRef<G>, DefinitionAllocationError> {
        self.generation.definitions_mut().publish(builder)
    }

    pub(crate) fn begin_definition_build(
        &mut self,
        destination: crate::DefinitionDestination,
        origin: crate::token::OriginId,
    ) -> Result<crate::DefinitionBuildKey<G>, DefinitionAllocationError> {
        self.generation
            .definitions_mut()
            .begin_build(destination, origin)
    }

    pub(crate) fn push_definition_parameter(
        &mut self,
        build: crate::DefinitionBuildKey<G>,
        word: TokenWord,
    ) -> Result<(), crate::DefinitionBuildError> {
        self.generation
            .definitions_mut()
            .push_parameter(build, word)
    }

    pub(crate) fn finish_definition_parameters(
        &mut self,
        build: crate::DefinitionBuildKey<G>,
    ) -> Result<(), crate::DefinitionBuildError> {
        self.generation.definitions_mut().finish_parameters(build)
    }

    pub(crate) fn set_definition_build_origin(
        &mut self,
        build: crate::DefinitionBuildKey<G>,
        origin: crate::token::OriginId,
    ) -> Result<(), crate::DefinitionBuildError> {
        self.generation
            .definitions_mut()
            .set_build_origin(build, origin)
    }

    pub(crate) fn push_definition_replacement(
        &mut self,
        build: crate::DefinitionBuildKey<G>,
        word: TokenWord,
    ) -> Result<(), crate::DefinitionBuildError> {
        self.generation
            .definitions_mut()
            .push_replacement(build, word)
    }

    pub(crate) fn seal_definition_build(
        &mut self,
        build: crate::DefinitionBuildKey<G>,
    ) -> Result<DefinitionRef<G>, crate::DefinitionBuildError> {
        self.generation.definitions_mut().seal_build(build)
    }

    pub(crate) fn abort_definition_build(&mut self, build: crate::DefinitionBuildKey<G>) {
        self.generation.definitions_mut().abort_build(build);
    }

    pub(crate) fn promote_definition_global(
        &mut self,
        definition: DefinitionRef<G>,
    ) -> Result<DefinitionRef<G>, DefinitionAllocationError> {
        self.generation.definitions_mut().promote_global(definition)
    }

    #[cfg(any(test, feature = "profiling"))]
    pub(crate) fn reserve_definition_arena(
        &mut self,
        rows: usize,
        words: usize,
    ) -> Result<(), DefinitionAllocationError> {
        self.generation.definitions_mut().reserve_batch(rows, words)
    }

    pub(crate) fn begin_definition_group(&mut self) -> Result<(), DefinitionAllocationError> {
        self.generation.definitions_mut().begin_group()
    }

    pub(crate) fn end_definition_group(&mut self) {
        self.generation.definitions_mut().end_group();
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

    pub(crate) fn append_node_tokens_to_builder(
        &mut self,
        builder: &TokenListBuilder<G>,
        key: crate::node::NodeTokenKey,
    ) -> Result<(), DurableAllocationError> {
        self.generation
            .token_lists_mut()
            .append_node_words_to_builder(builder, key)
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

    #[cfg(any(test, feature = "profiling"))]
    pub(crate) fn reserve_provenance_arena(
        &mut self,
        rows: usize,
    ) -> Result<(), DurableAllocationError> {
        self.generation.provenance_mut().reserve_batch(rows)
    }

    /// Promotes one already-validated batch while this unique admitted borrow
    /// prevents any consumer from observing partial destination state.
    pub(crate) fn promote_values(
        &mut self,
        definitions: &mut [crate::universe::DefinitionPromotion],
        token_lists: &[crate::universe::TokenListPromotion<'_>],
        glue_values: &[GlueSpec],
        provenance: &[OriginRecord],
    ) -> Result<crate::universe::PromotionReceipt<G>, crate::universe::PromotionError> {
        self.promote_value_streams_from(
            definitions,
            token_lists.iter().map(|list| list.words.iter().copied()),
            glue_values.iter().copied(),
            provenance.iter().copied(),
        )
    }

    /// Publishes a completely preflighted batch directly into its resident
    /// destination fields.
    pub(crate) fn promote_resident_batch<B>(
        &mut self,
        batch: &mut B,
    ) -> Result<(), crate::universe::PromotionError>
    where
        B: crate::universe::ResidentPromotionBatch<G>,
    {
        let definition_count = batch.definition_count();
        let definition_source_count = batch.definition_source_count();
        let token_list_count = batch.token_list_count();
        let token_list_source_count = batch.token_list_source_count();
        let glue_count = batch.glue_count();
        let glue_source_count = batch.glue_source_count();
        let provenance_count = batch.provenance_count();
        let provenance_source_count = batch.provenance_source_count();

        let definitions_arena = self.generation.definitions();
        for index in 0..definition_source_count {
            definitions_arena.validate_builder(batch.definition(index))?;
        }
        let definition_words = (0..definition_source_count)
            .try_fold(0usize, |total, index| {
                total.checked_add(batch.definition(index).words().len())
            })
            .ok_or(crate::universe::PromotionError::CapacityOverflow)?;
        let token_words = (0..token_list_source_count)
            .try_fold(0usize, |total, index| {
                total.checked_add(batch.token_list_len(index))
            })
            .ok_or(crate::universe::PromotionError::CapacityOverflow)?;

        // Reserve every destination before the first resident field changes.
        // Publication and field settlement below are then infallible.
        self.generation
            .definitions_mut()
            .reserve_batch(definition_count, definition_words)?;
        self.generation
            .token_lists_mut()
            .reserve_batch(token_list_count, token_words)?;
        for index in 0..glue_source_count {
            let _ = batch.glue(index);
        }
        for index in 0..provenance_source_count {
            let _ = batch.provenance(index);
        }
        self.generation.glue_mut().reserve_batch(glue_count)?;
        self.generation
            .provenance_mut()
            .reserve_batch(provenance_count)?;

        for _ in 0..definition_count {
            let definition = self
                .generation
                .definitions_mut()
                .publish_prevalidated(batch.next_definition_mut());
            batch.settle_next_definition(definition);
        }
        for _ in 0..token_list_count {
            let word_count = batch.next_token_list_len();
            let words = (0..word_count).map(|index| batch.next_token_list_word(index));
            let tokens = self
                .generation
                .token_lists_mut()
                .allocate_from_iter(words)
                .expect("the complete resident token-list batch was reserved");
            batch.settle_next_token_list(tokens);
        }
        for _ in 0..glue_count {
            let glue = self
                .generation
                .glue_mut()
                .allocate(batch.next_glue())
                .expect("the complete resident glue batch was reserved");
            batch.settle_next_glue(glue);
        }
        for _ in 0..provenance_count {
            let provenance = self
                .generation
                .provenance_mut()
                .allocate(batch.next_provenance())
                .expect("the complete resident provenance batch was reserved");
            batch.settle_next_provenance(provenance);
        }
        Ok(())
    }

    fn promote_value_streams_from<Definition, TokenLists, Words, Glue, Provenance>(
        &mut self,
        definitions: &mut [Definition],
        token_lists: TokenLists,
        glue_values: Glue,
        provenance: Provenance,
    ) -> Result<crate::universe::PromotionReceipt<G>, crate::universe::PromotionError>
    where
        Definition: DefinitionBatchItem,
        TokenLists: Clone + Iterator<Item = Words>,
        Words: ExactSizeIterator<Item = TokenWord>,
        Glue: Clone + Iterator<Item = GlueSpec>,
        Provenance: Clone + Iterator<Item = OriginRecord>,
    {
        let definitions_arena = self.generation.definitions();
        for definition in definitions.iter() {
            definitions_arena.validate_builder(definition.builder())?;
        }

        let definition_count = definitions.len();
        let token_list_count = token_lists.clone().count();
        let glue_count = glue_values.clone().count();
        let provenance_count = provenance.clone().count();
        let definition_words = definitions
            .iter()
            .try_fold(0usize, |total, definition| {
                total.checked_add(definition.builder().words().len())
            })
            .ok_or(crate::universe::PromotionError::CapacityOverflow)?;
        let token_words = token_lists
            .clone()
            .try_fold(0usize, |total, words| total.checked_add(words.len()))
            .ok_or(crate::universe::PromotionError::CapacityOverflow)?;

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

        let mut promoted_definitions = SmallVec::<[DefinitionRef<G>; 4]>::new();
        let mut promoted_token_lists = SmallVec::<[TokenListId<G>; 4]>::new();
        let mut promoted_glue = SmallVec::<[GlueId<G>; 4]>::new();
        let mut promoted_provenance = SmallVec::<[ProvenanceId<G>; 4]>::new();
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

        for definition in definitions.iter_mut() {
            promoted_definitions.push(
                self.generation
                    .definitions_mut()
                    .publish_prevalidated(definition.builder_mut()),
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
    /// but do not become live format-region rows. All final capacities and
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
        self.generation
            .definitions_mut()
            .reserve_format_batch(definition_rows, definition_words)?;
        self.generation
            .glue_mut()
            .reserve_batch(glue_values.len())?;

        let mut promoted_definitions = Vec::new();
        promoted_definitions
            .try_reserve_exact(definitions.len())
            .map_err(|_| crate::universe::PromotionError::AllocationFailed)?;
        promoted_definitions.resize(definitions.len(), None);
        let mut promoted_glue = Vec::new();
        promoted_glue
            .try_reserve_exact(glue_values.len())
            .map_err(|_| crate::universe::PromotionError::AllocationFailed)?;

        for (row, definition) in definitions.iter().enumerate() {
            if !live_definitions[row] {
                continue;
            }
            let id = self
                .generation
                .definitions_mut()
                .publish_format_row(definition)?;
            promoted_definitions[row] = Some(id);
        }
        let promoted_token_lists = self
            .generation
            .token_lists_mut()
            .install_format_rows(token_lists)?;
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
