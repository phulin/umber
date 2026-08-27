//! Coarse revision-generation owner and admitted state views.

use crate::definition_arena::{DefinitionAllocationError, DefinitionId, DefinitionView};
use crate::durable_arena::{
    DurableAllocationError, GlueId, ProvenanceId, TokenListBuilder, TokenListId, TokenListView,
};
use crate::env::{DenseState, DynamicMemoryRoot, StateError};
use crate::generation::{Generation, GenerationCursor, GenerationOwner, GenerationRetirement};
use crate::glue::GlueSpec;
use crate::node_arena::{
    DurableListId, NodeArenaCursor, NodeArenaError, NodeList, NodeMemoryScratch,
    NodeRelocationScratch, PageLifetime, PageListId, PageNodeArena,
};
use crate::provenance::OriginRecord;
use crate::token::TokenWord;
use std::sync::{RwLockReadGuard, RwLockWriteGuard};

#[cfg(test)]
#[path = "stores/tests.rs"]
mod tests;

/// Every mutable and immutable state owner for one revision generation.
pub(crate) struct StateCore<G> {
    generation: GenerationOwner<G>,
    nodes: PageNodeArena,
    state: DenseState<G>,
}

impl<G> StateCore<G> {
    pub(crate) fn enable_reachable_state_identity(&mut self) -> bool {
        let generation = self.generation.generation_mut().enable_semantic_identity();
        if !generation || !self.state.enable_reachable_state_identity() {
            return false;
        }
        self.nodes.enable_semantic_identity();
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

    /// Materializes one exact named-boundary bank. Candidate construction
    /// later moves this bank out of its checkpoint slot instead of copying it.
    pub(crate) fn checkpoint_copy(&self) -> Self {
        let generation = self.generation.generation().fork();
        Self {
            generation: GenerationOwner::new(generation),
            nodes: self.nodes.fork(),
            state: self.state.clone(),
        }
    }

    #[must_use]
    pub(crate) fn generation_cursor(&self) -> GenerationCursor {
        self.generation.generation().cursor()
    }

    pub(crate) fn restore_generation_cursor(&mut self, cursor: GenerationCursor) {
        self.generation.generation_mut().restore_cursor(cursor);
    }

    pub(crate) fn checkpoint_is_exact_head(
        &self,
        journal: crate::journal::JournalCursor<G>,
        durable: NodeArenaCursor<PageLifetime>,
        generation: GenerationCursor,
    ) -> bool {
        self.state.checkpoint_is_head(journal)
            && self.nodes.cursor_is_head(durable)
            && self.generation.generation().cursor() == generation
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
            DynamicMemoryRoot::Nodes(root) => {
                let _ = root;
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
        let accounting = generation.memory_accounting();
        Ok(Self {
            generation: GenerationOwner::new(generation),
            nodes: PageNodeArena::with_memory_accounting(accounting),
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
            nodes: &self.nodes,
            state: &self.state,
        }
    }

    /// Mutable episode admission. All state writes and durable publication
    /// remain behind this unique aggregate borrow.
    pub(crate) fn admit_mut(&mut self) -> Result<AdmittedStateMut<'_, G>, StateError> {
        Ok(AdmittedStateMut {
            generation: self.generation.generation_mut(),
            nodes: &mut self.nodes,
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

    #[must_use]
    pub(crate) fn durable_node_cursor(&self) -> NodeArenaCursor<PageLifetime> {
        self.nodes.cursor()
    }

    pub(crate) fn validate_durable_node_cursor(
        &self,
        cursor: NodeArenaCursor<PageLifetime>,
    ) -> Result<(), NodeArenaError> {
        self.nodes.validate_cursor(cursor)
    }

    pub(crate) fn durable_font_roots_are_live(
        &self,
        cursor: NodeArenaCursor<PageLifetime>,
        is_live: impl FnMut(crate::ids::FontId) -> bool,
    ) -> Result<bool, NodeArenaError> {
        self.nodes.font_roots_are_live(cursor, is_live)
    }

    pub(crate) fn truncate_durable_nodes(
        &mut self,
        cursor: NodeArenaCursor<PageLifetime>,
    ) -> Result<(), NodeArenaError> {
        self.nodes.truncate(cursor)
    }

    pub(crate) fn restore_durable_node_cursor(
        &mut self,
        cursor: NodeArenaCursor<PageLifetime>,
    ) -> Result<(), NodeArenaError> {
        self.nodes.restore_checkpoint_cursor(cursor)
    }

    /// Retires the complete generation after all admitted borrows end.
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
    pub(crate) fn can_retire_after_dropping(&self, owner: &GenerationOwner<G>) -> bool {
        self.generation.is_unique() || self.generation.is_owned_only_by(owner)
    }
}

pub(crate) struct DynamicMemoryScratch<G> {
    nodes: NodeMemoryScratch<PageLifetime>,
    durable_to_page: NodeRelocationScratch<PageLifetime, PageLifetime>,
    _brand: core::marker::PhantomData<fn(&G) -> &G>,
}

impl<G> Default for DynamicMemoryScratch<G> {
    fn default() -> Self {
        Self {
            nodes: NodeMemoryScratch::default(),
            durable_to_page: NodeRelocationScratch::default(),
            _brand: core::marker::PhantomData,
        }
    }
}

/// Immutable, already-admitted hot view.
pub(crate) struct AdmittedState<'a, G> {
    generation: RwLockReadGuard<'a, Generation<G>>,
    nodes: &'a PageNodeArena,
    state: &'a DenseState<G>,
}

impl<'a, G> AdmittedState<'a, G> {
    pub(crate) fn current_dynamic_memory_words(
        &self,
        etex_node_sizes: bool,
    ) -> Result<usize, NodeArenaError> {
        Ok(14_usize.saturating_add(self.generation.memory_accounting().words(etex_node_sizes).1))
    }
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

    #[inline(always)]
    pub(crate) fn node_list(
        &self,
        id: DurableListId<G>,
    ) -> Result<NodeList<'a, PageLifetime>, NodeArenaError> {
        self.nodes.get(id.rebrand())
    }

    pub(crate) fn materialize_loaded_node_into_page(
        &self,
        root: DurableListId<G>,
        destination: &mut PageNodeArena,
        scratch: &mut DynamicMemoryScratch<G>,
    ) -> Result<PageListId, NodeArenaError> {
        Ok(self.nodes.promote_into_with_scratch(
            &[root.rebrand()],
            destination,
            &mut scratch.durable_to_page,
            core::convert::identity,
            core::convert::identity,
        )?[0])
    }

    pub(crate) fn copied_node_closure_tex_memory_words(
        &self,
        root: DurableListId<G>,
        etex_node_sizes: bool,
        scratch: &mut DynamicMemoryScratch<G>,
    ) -> Result<(usize, usize), NodeArenaError> {
        let (variable, dynamic, _) = self.nodes.semantic_memory_usage(
            root.rebrand(),
            etex_node_sizes,
            &mut scratch.nodes,
            |_| {},
        )?;
        Ok((variable.saturating_mul(2), dynamic))
    }

    #[cfg(test)]
    pub(crate) fn provenance(&self, id: ProvenanceId<G>) -> OriginRecord {
        self.generation.provenance().get(id)
    }
}

/// Unique admitted view used for assignment and commit publication.
pub(crate) struct AdmittedStateMut<'a, G> {
    generation: RwLockWriteGuard<'a, Generation<G>>,
    nodes: &'a mut PageNodeArena,
    state: &'a mut DenseState<G>,
}

impl<'a, G> AdmittedStateMut<'a, G> {
    pub(crate) fn current_dynamic_memory_words(
        &self,
        etex_node_sizes: bool,
    ) -> Result<usize, NodeArenaError> {
        Ok(14_usize.saturating_add(self.generation.memory_accounting().words(etex_node_sizes).1))
    }
    pub(crate) const fn state_ref(&self) -> &DenseState<G> {
        self.state
    }

    pub(crate) fn state(&mut self) -> &mut DenseState<G> {
        self.state
    }

    pub(crate) fn nodes_mut(&mut self) -> &mut PageNodeArena {
        self.nodes
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
        Parameters: Clone + ExactSizeIterator<Item = TokenWord>,
        Replacement: Clone + ExactSizeIterator<Item = TokenWord>,
    {
        self.generation
            .definitions_mut()
            .allocate_from_iter(parameter_text, replacement_text)
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

    pub(crate) fn node_list(
        &self,
        id: DurableListId<G>,
    ) -> Result<NodeList<'_, PageLifetime>, NodeArenaError> {
        self.nodes.get(id.rebrand())
    }

    pub(crate) fn copied_node_closure_tex_memory_words(
        &self,
        root: DurableListId<G>,
        etex_node_sizes: bool,
        scratch: &mut DynamicMemoryScratch<G>,
    ) -> Result<(usize, usize), NodeArenaError> {
        let (variable, dynamic, _) = self.nodes.semantic_memory_usage(
            root.rebrand(),
            etex_node_sizes,
            &mut scratch.nodes,
            |_| {},
        )?;
        Ok((variable.saturating_mul(2), dynamic))
    }

    pub(crate) fn allocate_provenance(
        &mut self,
        value: OriginRecord,
    ) -> Result<ProvenanceId<G>, DurableAllocationError> {
        self.generation.provenance_mut().allocate(value)
    }

    pub(crate) fn materialize_loaded_node_into_page(
        &self,
        root: DurableListId<G>,
        destination: &mut PageNodeArena,
        scratch: &mut DynamicMemoryScratch<G>,
    ) -> Result<PageListId, NodeArenaError> {
        Ok(self.nodes.promote_into_with_scratch(
            &[root.rebrand()],
            destination,
            &mut scratch.durable_to_page,
            core::convert::identity,
            core::convert::identity,
        )?[0])
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

        for (row, definition) in definitions.into_iter().enumerate() {
            if !live_definitions[row] {
                continue;
            }
            let id = self
                .generation
                .definitions_mut()
                .allocate_from_iter(
                    definition
                        .parameter_text
                        .into_iter()
                        .map(TokenWord::from_raw),
                    definition
                        .replacement_text
                        .into_iter()
                        .map(TokenWord::from_raw),
                )
                .expect("the complete format definition batch was reserved");
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
