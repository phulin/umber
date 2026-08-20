//! Session epoch plus one admitted revision-generation state core.

use crate::checkpoint::{BoundedStateMark, GenerationCheckpoint, RestoreTarget, prepare_restore};
use crate::command_context::CommandContext;
use crate::definition_arena::{DefinitionAllocationError, DefinitionId};
use crate::durable_arena::{DurableAllocationError, GlueId, ProvenanceId, TokenListId};
use crate::env::group::{GroupFrame, GroupKind};
use crate::env::{AssignmentScope, CodeTableKind, StateError};
use crate::generation::{GenerationBrand, GenerationOwner, with_generation};
use crate::glue::GlueSpec;
use crate::interner::{
    ControlSequenceKind, Interner, InternerAccessError, InternerBudget, InternerError,
    InternerRetirement, InternerUsage, Symbol, SymbolId,
};
use crate::journal::JournalCursor;
use crate::meaning::{Meaning, MeaningWord};
use crate::node::Node;
use crate::node_arena::{
    DurableListId, NodeArenaCursor, NodeArenaError, NodeList, PageLifetime, PageListId,
    PageNodeArena,
};
use crate::provenance::OriginRecord;
use crate::scaled::Scaled;
use crate::stores::{StateCore, StateCoreRetirement};
use crate::token::TokenWord;
use crate::world::World;

#[cfg(test)]
#[path = "universe/tests.rs"]
mod tests;

/// Failure to construct, use, or retire the aggregate lifetime owners.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UniverseError {
    Interner(InternerError),
    InternerAccess(InternerAccessError),
    DefinitionAllocation(DefinitionAllocationError),
    DurableAllocation(DurableAllocationError),
    NodeArena(NodeArenaError),
    State(StateError),
    Retired,
}

/// Current engine interaction mode.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub enum InteractionMode {
    Batch,
    Nonstop,
    Scroll,
    #[default]
    ErrorStop,
}

/// One explicit macro-definition escape root in a promotion batch.
#[derive(Clone, Copy, Debug)]
pub struct DefinitionPromotion<'a> {
    pub parameter_text: &'a [TokenWord],
    pub replacement_text: &'a [TokenWord],
}

/// One explicit durable token-list escape root in a promotion batch.
#[derive(Clone, Copy, Debug)]
pub struct TokenListPromotion<'a> {
    pub words: &'a [TokenWord],
}

/// Failure to reserve or validate an atomic promotion batch.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PromotionError {
    CapacityOverflow,
    AllocationFailed,
    Retired,
    GenerationInUse,
}

impl From<DefinitionAllocationError> for PromotionError {
    fn from(error: DefinitionAllocationError) -> Self {
        match error {
            DefinitionAllocationError::CapacityOverflow => Self::CapacityOverflow,
            DefinitionAllocationError::AllocationFailed => Self::AllocationFailed,
        }
    }
}

impl From<DurableAllocationError> for PromotionError {
    fn from(error: DurableAllocationError) -> Self {
        match error {
            DurableAllocationError::CapacityOverflow => Self::CapacityOverflow,
            DurableAllocationError::AllocationFailed => Self::AllocationFailed,
        }
    }
}

/// Destination coordinates published together after complete batch staging.
#[derive(Debug)]
pub struct PromotionReceipt<G> {
    pub definitions: Vec<DefinitionId<G>>,
    pub token_lists: Vec<TokenListId<G>>,
    pub glue: Vec<GlueId<G>>,
    pub provenance: Vec<ProvenanceId<G>>,
}

/// Failure to promote an exact page-node closure into durable generation
/// storage.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NodePromotionError {
    Values(PromotionError),
    Nodes(NodeArenaError),
}

/// Fixed-size tex-state portion of a retained aggregate checkpoint.
pub type StateCheckpointMark<G, Input = ()> =
    BoundedStateMark<JournalCursor<G>, NodeArenaCursor<G>, NodeArenaCursor<PageLifetime>, Input>;

/// Coarse generation owner plus bounded state cursors.
pub type StateCheckpoint<G, Input = ()> =
    GenerationCheckpoint<GenerationOwner<G>, StateCheckpointMark<G, Input>>;

impl From<PromotionError> for NodePromotionError {
    fn from(error: PromotionError) -> Self {
        Self::Values(error)
    }
}

impl From<NodeArenaError> for NodePromotionError {
    fn from(error: NodeArenaError) -> Self {
        Self::Nodes(error)
    }
}

impl From<InternerError> for UniverseError {
    fn from(error: InternerError) -> Self {
        Self::Interner(error)
    }
}

impl From<InternerAccessError> for UniverseError {
    fn from(error: InternerAccessError) -> Self {
        Self::InternerAccess(error)
    }
}

impl From<StateError> for UniverseError {
    fn from(error: StateError) -> Self {
        Self::State(error)
    }
}

impl From<DefinitionAllocationError> for UniverseError {
    fn from(error: DefinitionAllocationError) -> Self {
        Self::DefinitionAllocation(error)
    }
}

impl From<DurableAllocationError> for UniverseError {
    fn from(error: DurableAllocationError) -> Self {
        Self::DurableAllocation(error)
    }
}

impl From<NodeArenaError> for UniverseError {
    fn from(error: NodeArenaError) -> Self {
        Self::NodeArena(error)
    }
}

/// Coarse owner of one session interning epoch and current generation.
pub struct Universe<G> {
    interner: Interner,
    core: Option<StateCore<G>>,
    page_nodes: PageNodeArena,
    world: World,
    interaction_mode: InteractionMode,
    primitive_names: Vec<String>,
    primitive_meanings: Vec<Meaning>,
    restore_owner: Option<GenerationOwner<G>>,
}

impl<G> Universe<G> {
    fn new(interner: Interner, core: StateCore<G>) -> Self {
        Self {
            interner,
            core: Some(core),
            page_nodes: PageNodeArena::new(),
            world: World::default(),
            interaction_mode: InteractionMode::default(),
            primitive_names: Vec::new(),
            primitive_meanings: Vec::new(),
            restore_owner: None,
        }
    }

    /// Interns outside every TeX group and rollback cursor, then admits the
    /// issued compact slot into the generation's dense meaning bank.
    pub fn intern(&mut self, name: &str) -> Result<SymbolId, UniverseError> {
        if self.core.is_none() {
            return Err(UniverseError::Retired);
        }
        let symbol = self.interner.intern(name)?;
        self.core
            .as_mut()
            .ok_or(UniverseError::Retired)?
            .admit_mut()?
            .state()
            .admit_symbol(symbol.symbol())?;
        Ok(symbol)
    }

    pub fn resolve_symbol(&self, symbol: SymbolId) -> Result<&str, UniverseError> {
        Ok(self.interner.resolve_id(symbol)?)
    }

    #[must_use]
    pub fn resolve(&self, symbol: Symbol) -> Option<&str> {
        self.interner.resolve_local(symbol)
    }

    #[must_use]
    pub fn qualify_symbol(&self, symbol: Symbol) -> Option<SymbolId> {
        self.interner.qualify_local(symbol)
    }

    #[must_use]
    pub fn control_sequence_kind(&self, symbol: Symbol) -> Option<ControlSequenceKind> {
        self.qualify_symbol(symbol)
            .and_then(|id| self.interner.kind_id(id).ok())
    }

    #[must_use]
    pub fn active_character_symbol(&self, ch: char) -> Option<SymbolId> {
        self.interner.active(ch)
    }

    pub fn intern_active_character(&mut self, ch: char) -> Result<SymbolId, UniverseError> {
        let symbol = self.interner.intern_active(ch)?;
        self.core
            .as_mut()
            .ok_or(UniverseError::Retired)?
            .admit_mut()?
            .state()
            .admit_symbol(symbol.symbol())?;
        Ok(symbol)
    }

    /// Records one immutable primitive-table row without changing eqtb.
    pub fn register_primitive_meaning(&mut self, name: &str, meaning: Meaning) {
        if let Some(index) = self
            .primitive_names
            .iter()
            .position(|candidate| candidate == name)
        {
            assert_eq!(self.primitive_meanings[index], meaning);
            return;
        }
        let index = self.primitive_names.len();
        assert!(
            index < 60_000 - 2,
            "primitive registry exceeds frozen-token capacity"
        );
        self.primitive_names.push(name.to_owned());
        self.primitive_meanings.push(meaning);
    }

    /// Records and installs one static primitive meaning.
    pub fn install_primitive_meaning(&mut self, name: &str, meaning: Meaning) {
        self.register_primitive_meaning(name, meaning);
        let symbol = self
            .intern(name)
            .expect("primitive name exceeds interner budget");
        self.assign_meaning(
            symbol,
            MeaningWord::from_static(meaning),
            AssignmentScope::Global,
        )
        .expect("primitive meaning installation must target admitted state");
    }

    #[must_use]
    pub fn primitive_meaning(&self, name: &str) -> Option<Meaning> {
        self.primitive_names
            .iter()
            .position(|candidate| candidate == name)
            .map(|index| self.primitive_meanings[index])
    }

    #[must_use]
    pub fn primitive_name(&self, meaning: Meaning) -> Option<&str> {
        self.primitive_meanings
            .iter()
            .position(|&candidate| candidate == meaning)
            .map(|index| self.primitive_names[index].as_str())
    }

    #[must_use]
    pub fn primitive_token(&self, name: &str) -> Option<crate::token::Token> {
        let index = self
            .primitive_names
            .iter()
            .position(|candidate| candidate == name)?;
        Some(crate::token::Token::frozen_primitive(
            u16::try_from(index).ok()?,
        ))
    }

    #[must_use]
    pub fn frozen_primitive_name(&self, token: crate::token::Token) -> Option<&str> {
        let crate::token::Token::Frozen(frozen) = token else {
            return None;
        };
        if token.is_frozen_end_template() || token.is_frozen_endv() {
            return Some("endtemplate");
        }
        if token.is_frozen_relax() {
            return Some("relax");
        }
        self.primitive_names
            .get(usize::from(frozen.primitive_index()?))
            .map(String::as_str)
    }

    #[must_use]
    pub fn frozen_primitive_meaning(&self, token: crate::token::Token) -> Option<Meaning> {
        let crate::token::Token::Frozen(frozen) = token else {
            return None;
        };
        self.primitive_meanings
            .get(usize::from(frozen.primitive_index()?))
            .copied()
    }

    #[must_use]
    pub fn catcode(&self, ch: char) -> crate::token::Catcode {
        let raw = self
            .command_context()
            .ok()
            .and_then(|context| context.code(CodeTableKind::Catcode, ch).ok())
            .unwrap_or(crate::token::Catcode::Other as i64);
        u8::try_from(raw)
            .ok()
            .and_then(crate::token::Catcode::from_raw)
            .unwrap_or(crate::token::Catcode::Other)
    }

    #[must_use]
    pub fn font_name(&self, id: crate::ids::FontId) -> String {
        format!("font{}", id.raw())
    }

    pub fn meaning(
        &self,
        symbol: Symbol,
    ) -> Result<crate::meaning::ResolvedMeaning<G>, UniverseError> {
        Ok(self.command_context()?.meaning(symbol)?)
    }

    #[must_use]
    pub const fn world(&self) -> &World {
        &self.world
    }

    pub const fn world_mut(&mut self) -> &mut World {
        &mut self.world
    }

    #[must_use]
    pub const fn interaction_mode(&self) -> InteractionMode {
        self.interaction_mode
    }

    pub const fn set_interaction_mode(&mut self, mode: InteractionMode) {
        self.interaction_mode = mode;
    }

    #[must_use]
    pub fn int_param(&self, parameter: crate::env::banks::IntParam) -> i32 {
        self.core
            .as_ref()
            .and_then(|core| core.admit().state().integer_parameter(parameter).ok())
            .unwrap_or(0)
    }

    pub fn allocate_definition(
        &mut self,
        parameter_text: &[TokenWord],
        replacement_text: &[TokenWord],
    ) -> Result<DefinitionId<G>, UniverseError> {
        Ok(self
            .core
            .as_mut()
            .ok_or(UniverseError::Retired)?
            .admit_mut()?
            .allocate_definition(parameter_text, replacement_text)?)
    }

    pub fn allocate_token_list(
        &mut self,
        words: &[TokenWord],
    ) -> Result<TokenListId<G>, UniverseError> {
        Ok(self
            .core
            .as_mut()
            .ok_or(UniverseError::Retired)?
            .admit_mut()?
            .allocate_token_list(words)?)
    }

    pub fn allocate_glue(&mut self, value: GlueSpec) -> Result<GlueId<G>, UniverseError> {
        Ok(self
            .core
            .as_mut()
            .ok_or(UniverseError::Retired)?
            .admit_mut()?
            .allocate_glue(value)?)
    }

    pub fn allocate_provenance(
        &mut self,
        value: OriginRecord,
    ) -> Result<ProvenanceId<G>, UniverseError> {
        Ok(self
            .core
            .as_mut()
            .ok_or(UniverseError::Retired)?
            .admit_mut()?
            .allocate_provenance(value)?)
    }

    /// Promotes only the caller's declared escaping roots.
    ///
    /// All source traversal happens before this boundary. The state core
    /// reserves every destination table and the receipt remains private until
    /// every row has been initialized, so callers cannot publish a partial
    /// relocation.
    pub fn promote_values(
        &mut self,
        definitions: &[DefinitionPromotion<'_>],
        token_lists: &[TokenListPromotion<'_>],
        glue_values: &[GlueSpec],
        provenance: &[OriginRecord],
    ) -> Result<PromotionReceipt<G>, PromotionError> {
        self.core
            .as_mut()
            .ok_or(PromotionError::Retired)?
            .admit_mut()
            .map_err(|error| match error {
                StateError::GenerationInUse => PromotionError::GenerationInUse,
                _ => PromotionError::AllocationFailed,
            })?
            .promote_values(definitions, token_lists, glue_values, provenance)
    }

    /// Promotes only the page-list closures reachable from `roots` into this
    /// revision generation.
    ///
    /// Page-owned glue and mark-token payloads are first collected through
    /// the same deterministic postorder used for node relocation. The values
    /// are published as one generation batch, then the node rows are densely
    /// relocated with those returned durable coordinates.
    pub fn promote_page_nodes(
        &mut self,
        source: &PageNodeArena,
        roots: &[PageListId],
    ) -> Result<Vec<DurableListId<G>>, NodePromotionError> {
        source.reserve_promotion(
            roots,
            self.core
                .as_mut()
                .ok_or(PromotionError::Retired)?
                .admit_mut()
                .map_err(|error| match error {
                    StateError::GenerationInUse => PromotionError::GenerationInUse,
                    _ => PromotionError::AllocationFailed,
                })?
                .nodes_mut(),
        )?;
        let (glue, tokens) = source.escaping_payloads(roots)?;
        let token_promotions = tokens
            .iter()
            .map(|tokens| TokenListPromotion {
                words: tokens.words(),
            })
            .collect::<Vec<_>>();
        let receipt = self.promote_values(&[], &token_promotions, &glue, &[])?;
        let mut glue_ids = receipt.glue.into_iter();
        let mut token_ids = receipt.token_lists.into_iter();
        let promoted = source.promote_into_with(
            roots,
            self.core
                .as_mut()
                .ok_or(PromotionError::Retired)?
                .admit_mut()
                .map_err(|error| match error {
                    StateError::GenerationInUse => PromotionError::GenerationInUse,
                    _ => PromotionError::AllocationFailed,
                })?
                .nodes_mut(),
            |_| glue_ids.next().expect("one durable id per page glue root"),
            |_| {
                token_ids
                    .next()
                    .expect("one durable id per page token root")
            },
        )?;
        debug_assert!(glue_ids.next().is_none());
        debug_assert!(token_ids.next().is_none());
        Ok(promoted)
    }

    /// Promotes closures from this live page arena into durable generation
    /// storage.
    pub fn promote_live_page_nodes(
        &mut self,
        roots: &[PageListId],
    ) -> Result<Vec<DurableListId<G>>, NodePromotionError> {
        {
            let (source, core) = (&self.page_nodes, &mut self.core);
            source.reserve_promotion(
                roots,
                core.as_mut()
                    .ok_or(PromotionError::Retired)?
                    .admit_mut()
                    .map_err(|error| match error {
                        StateError::GenerationInUse => PromotionError::GenerationInUse,
                        _ => PromotionError::AllocationFailed,
                    })?
                    .nodes_mut(),
            )?;
        }
        let (glue, tokens) = self.page_nodes.escaping_payloads(roots)?;
        let token_promotions = tokens
            .iter()
            .map(|tokens| TokenListPromotion {
                words: tokens.words(),
            })
            .collect::<Vec<_>>();
        let receipt = self.promote_values(&[], &token_promotions, &glue, &[])?;
        let mut glue_ids = receipt.glue.into_iter();
        let mut token_ids = receipt.token_lists.into_iter();
        let (source, core) = (&self.page_nodes, &mut self.core);
        let promoted = source.promote_into_with(
            roots,
            core.as_mut()
                .ok_or(PromotionError::Retired)?
                .admit_mut()
                .map_err(|error| match error {
                    StateError::GenerationInUse => PromotionError::GenerationInUse,
                    _ => PromotionError::AllocationFailed,
                })?
                .nodes_mut(),
            |_| glue_ids.next().expect("one durable id per page glue root"),
            |_| {
                token_ids
                    .next()
                    .expect("one durable id per page token root")
            },
        )?;
        debug_assert!(glue_ids.next().is_none());
        debug_assert!(token_ids.next().is_none());
        Ok(promoted)
    }

    /// Copies one durable box closure into page-lifetime storage.
    pub fn copy_durable_page_nodes(
        &mut self,
        root: DurableListId<G>,
    ) -> Result<PageListId, NodeArenaError> {
        let (core, page_nodes) = (&self.core, &mut self.page_nodes);
        Ok(core
            .as_ref()
            .ok_or(NodeArenaError::InvalidList)?
            .admit()
            .copy_nodes_into_page(&[root], page_nodes)?[0])
    }

    /// Publishes one complete page-lifetime node list.
    #[must_use]
    pub fn publish_page_nodes(&mut self, nodes: &[Node]) -> PageListId {
        self.page_nodes
            .publish(nodes.to_vec())
            .expect("page construction contains only live page-arena children")
    }

    /// Publishes a page-lifetime node list by moving the caller's buffer.
    pub fn publish_page_nodes_owned(&mut self, nodes: &mut Vec<Node>) -> PageListId {
        self.page_nodes
            .publish(core::mem::take(nodes))
            .expect("page construction contains only live page-arena children")
    }

    /// Resolves one list through this episode's page arena.
    pub fn page_node_list(
        &self,
        id: PageListId,
    ) -> Result<NodeList<'_, PageLifetime>, NodeArenaError> {
        self.page_nodes.get(id)
    }

    /// Captures the page-arena suffix for operation rollback.
    ///
    /// Aggregate operation marks store this cursor by value. Rollback must
    /// restore every canonical mode, alignment, insertion, and page-builder
    /// root before calling [`Self::truncate_page_nodes`].
    #[must_use]
    pub fn page_node_cursor(&self) -> NodeArenaCursor<PageLifetime> {
        self.page_nodes.cursor()
    }

    /// Truncates a rejected page-arena suffix after canonical roots restore.
    ///
    /// This ordering is part of the command-attempt integration contract:
    /// root restoration comes first, suffix truncation second.
    pub fn truncate_page_nodes(
        &mut self,
        cursor: NodeArenaCursor<PageLifetime>,
    ) -> Result<(), NodeArenaError> {
        self.page_nodes.truncate(cursor)
    }

    /// Releases storage reachable only from a completed page after its
    /// handle-free output has been validated and the canonical root removed.
    pub fn release_completed_page(&mut self, root: PageListId) -> Result<(), NodeArenaError> {
        self.page_nodes.release_closure(root)
    }

    pub fn assign_meaning(
        &mut self,
        symbol: SymbolId,
        value: MeaningWord<G>,
        scope: AssignmentScope,
    ) -> Result<(), UniverseError> {
        self.interner.resolve_id(symbol)?;
        self.live_state_mut()?
            .assign_meaning(symbol.symbol(), value, scope)?;
        Ok(())
    }

    pub fn assign_count(
        &mut self,
        index: u16,
        value: i32,
        scope: AssignmentScope,
    ) -> Result<(), UniverseError> {
        self.live_state_mut()?.assign_count(index, value, scope)?;
        Ok(())
    }

    pub fn assign_dimension(
        &mut self,
        index: u16,
        value: Scaled,
        scope: AssignmentScope,
    ) -> Result<(), UniverseError> {
        self.live_state_mut()?
            .assign_dimension(index, value, scope)?;
        Ok(())
    }

    pub fn assign_token_register(
        &mut self,
        index: u16,
        value: Option<TokenListId<G>>,
        scope: AssignmentScope,
    ) -> Result<(), UniverseError> {
        self.live_state_mut()?
            .assign_token_register(index, value, scope)?;
        Ok(())
    }

    pub fn assign_glue_register(
        &mut self,
        index: u16,
        value: Option<GlueId<G>>,
        scope: AssignmentScope,
    ) -> Result<(), UniverseError> {
        self.live_state_mut()?
            .assign_glue_register(index, value, scope)?;
        Ok(())
    }

    pub fn assign_box_register(
        &mut self,
        index: u16,
        value: Option<DurableListId<G>>,
        scope: AssignmentScope,
    ) -> Result<(), UniverseError> {
        self.live_state_mut()?
            .assign_box_register(index, value, scope)?;
        Ok(())
    }

    /// Promotes a page-lifetime box and assigns its durable root atomically at
    /// the TeX state boundary.
    pub fn assign_page_box(
        &mut self,
        index: u16,
        value: Option<PageListId>,
        scope: AssignmentScope,
    ) -> Result<(), NodePromotionError> {
        let durable = value
            .map(|root| self.promote_live_page_nodes(&[root]).map(|roots| roots[0]))
            .transpose()?;
        self.live_state_mut()
            .map_err(|error| {
                NodePromotionError::Values(match error {
                    UniverseError::State(StateError::GenerationInUse) => {
                        PromotionError::GenerationInUse
                    }
                    UniverseError::Retired => PromotionError::Retired,
                    _ => PromotionError::AllocationFailed,
                })
            })?
            .assign_box_register(index, durable, scope)
            .map_err(|_| NodePromotionError::Values(PromotionError::AllocationFailed))?;
        Ok(())
    }

    pub fn assign_page_box_local(&mut self, index: u16, value: PageListId) {
        self.assign_page_box(index, Some(value), AssignmentScope::Local)
            .expect("live page box promotion must succeed")
    }

    pub fn assign_page_box_global(&mut self, index: u16, value: PageListId) {
        self.assign_page_box(index, Some(value), AssignmentScope::Global)
            .expect("live page box promotion must succeed")
    }

    pub fn clear_box_local(&mut self, index: u16) {
        self.assign_page_box(index, None, AssignmentScope::Local)
            .expect("void box assignment cannot allocate")
    }

    pub fn clear_box_global(&mut self, index: u16) {
        self.assign_page_box(index, None, AssignmentScope::Global)
            .expect("void box assignment cannot allocate")
    }

    /// Promotes and replaces a box while retaining its current eq level.
    pub fn replace_page_box(&mut self, index: u16, value: PageListId) {
        let durable = self
            .promote_live_page_nodes(&[value])
            .expect("live page box promotion must succeed")[0];
        self.live_state_mut()
            .expect("live generation is admitted")
            .replace_box_register(index, Some(durable))
            .expect("box register index is admitted")
    }

    /// Copies a box-register closure into page-lifetime storage.
    pub fn copy_box_to_page(&mut self, index: u16) -> Option<PageListId> {
        self.box_register(index)
            .expect("box register index is admitted")
            .map(|root| {
                self.copy_durable_page_nodes(root)
                    .expect("durable box closure belongs to the live generation")
            })
    }

    /// Moves a box-register closure into page storage while preserving the
    /// register's TeX eq level.
    pub fn take_box_to_page(&mut self, index: u16) -> Option<PageListId> {
        let copied = self.copy_box_to_page(index);
        if copied.is_some() {
            self.live_state_mut()
                .expect("live generation is admitted")
                .replace_box_register(index, None)
                .expect("box register index is admitted");
        }
        copied
    }

    /// Voids one register without changing its TeX eq level.
    pub fn clear_box_preserving_level(&mut self, index: u16) {
        self.live_state_mut()
            .expect("live generation is admitted")
            .replace_box_register(index, None)
            .expect("box register index is admitted")
    }

    pub fn box_register(&self, index: u16) -> Result<Option<DurableListId<G>>, UniverseError> {
        Ok(self
            .core
            .as_ref()
            .ok_or(UniverseError::Retired)?
            .admit()
            .state()
            .box_register(index)?)
    }

    pub fn node_list(
        &self,
        id: DurableListId<G>,
    ) -> Result<NodeList<'_, G, crate::GlueId<G>, crate::TokenListId<G>>, UniverseError> {
        Ok(self
            .core
            .as_ref()
            .ok_or(UniverseError::Retired)?
            .admit()
            .node_list(id)?)
    }

    pub fn assign_code(
        &mut self,
        kind: CodeTableKind,
        scalar: char,
        value: i64,
        scope: AssignmentScope,
    ) -> Result<(), UniverseError> {
        self.live_state_mut()?
            .assign_code(kind, scalar, value, scope)?;
        Ok(())
    }

    pub fn journal_cursor(&self) -> Result<JournalCursor<G>, UniverseError> {
        Ok(self
            .core
            .as_ref()
            .ok_or(UniverseError::Retired)?
            .admit()
            .state()
            .journal_cursor())
    }

    pub fn restore_state(&mut self, cursor: JournalCursor<G>) -> Result<(), UniverseError> {
        self.live_state_mut()?.restore(cursor)?;
        Ok(())
    }

    /// Retains one coarse generation beside bounded state and arena cursors.
    ///
    /// Command, mode, source, effect, and output owners compose their own
    /// bounded cursor fields around this state-layer foundation.
    pub fn state_checkpoint(&self) -> Result<StateCheckpoint<G>, UniverseError> {
        let core = self.core.as_ref().ok_or(UniverseError::Retired)?;
        Ok(GenerationCheckpoint::new(
            core.generation_owner(),
            BoundedStateMark::new(
                core.state().journal_cursor(),
                core.durable_node_cursor(),
                self.page_nodes.cursor(),
                (),
            ),
        ))
    }

    /// Restores the state-owned portion of a retained checkpoint atomically.
    ///
    /// Validation is complete before the first mutation. The retained owner is
    /// installed before dense words can expose restored coordinates, and both
    /// durable and page suffixes truncate only after root-bearing dense state
    /// has been restored.
    pub fn restore_state_checkpoint(
        &mut self,
        checkpoint: &StateCheckpoint<G>,
    ) -> Result<(), UniverseError> {
        let plan = prepare_restore(self, checkpoint.clone())?;
        plan.apply(self);
        Ok(())
    }

    pub fn begin_group(
        &mut self,
        kind: GroupKind,
        entered_line: u32,
    ) -> Result<GroupFrame, UniverseError> {
        Ok(self.live_state_mut()?.begin_group(kind, entered_line)?)
    }

    pub fn end_group(&mut self, kind: GroupKind) -> Result<GroupFrame, UniverseError> {
        Ok(self.live_state_mut()?.end_group(kind)?)
    }

    /// Admits matching coarse owners once for a command episode.
    pub fn command_context(&self) -> Result<CommandContext<'_, G>, UniverseError> {
        let core = self.core.as_ref().ok_or(UniverseError::Retired)?;
        Ok(CommandContext::new(
            &self.interner,
            core.admit(),
            &self.primitive_names,
            &self.primitive_meanings,
        ))
    }

    /// Retains the complete immutable generation across an in-process
    /// resource suspension. No individual runtime value acquires an owner.
    pub fn generation_owner(&self) -> Result<GenerationOwner<G>, UniverseError> {
        Ok(self
            .core
            .as_ref()
            .ok_or(UniverseError::Retired)?
            .generation_owner())
    }

    /// Validates a returned continuation owner before its attempt is resumed.
    #[must_use]
    pub fn owns_generation(&self, owner: &GenerationOwner<G>) -> bool {
        self.core
            .as_ref()
            .is_some_and(|core| core.owns_generation(owner))
    }

    fn live_state_mut(&mut self) -> Result<&mut crate::env::DenseState<G>, UniverseError> {
        Ok(self
            .core
            .as_mut()
            .ok_or(UniverseError::Retired)?
            .state_mut())
    }

    /// Retires the session epoch and revision generation together. The
    /// aggregate remains only as a typed retired shell which rejects reuse.
    pub fn retire(&mut self) -> Result<UniverseRetirement, UniverseError> {
        if !self
            .core
            .as_ref()
            .ok_or(UniverseError::Retired)?
            .can_retire()
        {
            return Err(UniverseError::State(StateError::GenerationInUse));
        }
        let core = self.core.take().ok_or(UniverseError::Retired)?;
        let interner = self.interner.retire()?;
        Ok(UniverseRetirement {
            interner,
            state: core.retire()?,
        })
    }

    #[must_use]
    pub const fn is_retired(&self) -> bool {
        self.core.is_none()
    }
}

impl<G> RestoreTarget<GenerationOwner<G>, StateCheckpointMark<G>> for Universe<G> {
    type Error = UniverseError;
    type Output = ();

    fn validate_restore(
        &self,
        owner: &GenerationOwner<G>,
        mark: &StateCheckpointMark<G>,
    ) -> Result<(), Self::Error> {
        let core = self.core.as_ref().ok_or(UniverseError::Retired)?;
        if !core.owns_generation(owner) || self.restore_owner.is_some() {
            return Err(UniverseError::State(StateError::InvalidCursor));
        }
        core.state().validate_restore(*mark.journal())?;
        core.validate_durable_node_cursor(*mark.durable())?;
        self.page_nodes.validate_cursor(*mark.page())?;
        Ok(())
    }

    fn acquire_target_owner(&mut self, owner: GenerationOwner<G>) {
        debug_assert!(self.restore_owner.is_none());
        self.restore_owner = Some(owner);
    }

    fn restore_dense_state(&mut self, mark: &StateCheckpointMark<G>) {
        self.core
            .as_mut()
            .expect("restore plan validated a live state core")
            .state_mut()
            .restore(*mark.journal())
            .expect("restore plan prevalidated dense state");
    }

    fn transfer_roots(&mut self, _mark: &StateCheckpointMark<G>) {
        // Dense state owns every state-layer root. Command, mode, source,
        // effect, and output roots are transferred by the aggregate plan
        // composed in later migration stages.
    }

    fn truncate_suffixes(&mut self, mark: &StateCheckpointMark<G>) {
        self.core
            .as_mut()
            .expect("restore plan validated a live state core")
            .truncate_durable_nodes(*mark.durable())
            .expect("restore plan prevalidated durable-node suffix");
        self.page_nodes
            .truncate(*mark.page())
            .expect("restore plan prevalidated page-node suffix");
    }

    fn release_replaced_owners(&mut self) {
        drop(
            self.restore_owner
                .take()
                .expect("restore acquired its target generation owner"),
        );
    }
}

/// Evidence from whole-session/coarse-generation retirement.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UniverseRetirement {
    interner: InternerRetirement,
    state: StateCoreRetirement,
}

impl UniverseRetirement {
    #[must_use]
    pub const fn interner_usage(self) -> InternerUsage {
        self.interner.usage()
    }

    #[must_use]
    pub const fn definition_rows(self) -> usize {
        self.state.generation.definitions
    }

    #[must_use]
    pub const fn token_list_rows(self) -> usize {
        self.state.generation.token_lists
    }

    #[must_use]
    pub const fn glue_rows(self) -> usize {
        self.state.generation.glue_values
    }

    #[must_use]
    pub const fn provenance_rows(self) -> usize {
        self.state.generation.provenance_records
    }

    #[must_use]
    pub const fn durable_node_lists(self) -> usize {
        self.state.durable_node_lists
    }

    #[must_use]
    pub const fn journal_entries(self) -> usize {
        self.state.journal_entries
    }
}

/// Introduces one fresh generation brand and keeps it inside `use_universe`.
pub fn with_universe<R>(
    budget: InternerBudget,
    use_universe: impl for<'id> FnOnce(&mut Universe<GenerationBrand<'id>>) -> R,
) -> Result<R, StateError> {
    with_generation(|generation| {
        let core = StateCore::new(generation)?;
        let mut universe = Universe::new(Interner::new(budget), core);
        Ok(use_universe(&mut universe))
    })
}
