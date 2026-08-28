//! Session epoch plus one admitted revision-generation state core.

use crate::checkpoint::{BoundedStateMark, GenerationCheckpoint, RestoreTarget, prepare_restore};
use crate::command_context::{CommandContext, CommandContextParts};
use crate::definition_arena::{DefinitionAllocationError, DefinitionId};
use crate::dependency::{
    AcceptedDependencyTail, DependencyRegionError, DependencyRegionToken, DependencyRuntime,
    ObservedDependency, TrackedRegionBarrier,
};
use crate::durable_arena::{DurableAllocationError, GlueId, ProvenanceId, TokenListId};
use crate::env::group::{GroupFrame, GroupKind};
use crate::env::{AssignmentScope, CodeTableKind, DenseStateCursor, StateError};
use crate::font::{AcceptedFontStoreTail, FontStore, FontStoreMark};
use crate::fork_arena::{CheckpointMark, OperationMark, PageMaterialLane};
use crate::generation::{GenerationBrand, GenerationCursor, GenerationOwner, with_generation};
use crate::glue::GlueSpec;
use crate::hyphenation::HyphenationCheckpoint;
use crate::hyphenation::HyphenationTable;
use crate::interner::{
    ControlSequenceKind, Interner, InternerAccessError, InternerBudget, InternerError,
    InternerRetirement, InternerUsage, Symbol, SymbolId,
};
use crate::journal::{JournalCursor, StateOperation};
use crate::meaning::{Meaning, MeaningWord};
use crate::node::Node;
use crate::node_arena::{DurableListId, NodeArenaError, PageListId, PageNodeArena};
use crate::page::{AcceptedPageTail, PageBuilderState, PageCheckpointMark};
use crate::pdf::PdfStateSlot;
use crate::print::ErrorContextWidths;
use crate::provenance::OriginRecord;
use crate::scaled::Scaled;
use crate::session_epoch::{InternerLease, SessionEpochError, SessionInternerEpoch};
use crate::shipout_scratch::{
    ShipoutScratchArena, ShipoutScratchListId, ShipoutScratchMark, ShipoutScratchNode,
};
use crate::source_map::{AcceptedSourceMapTail, SourceMap, SourceMapMark};
use crate::stores::{AcceptedStateCoreTail, StateCore, StateCoreRetirement};
use crate::token::TokenWord;
use crate::world::{AcceptedWorldTail, World, WorldSnapshot};
use std::rc::Rc;

fn tex_memory_words(nodes: &[Node], etex_node_sizes: bool) -> (usize, usize) {
    nodes.iter().fold((0_usize, 0_usize), |words, node| {
        let node_words = node.tex_memory_words(etex_node_sizes);
        (
            words.0.saturating_add(node_words.0),
            words.1.saturating_add(node_words.1),
        )
    })
}

/// Canonical projection builder for executor-owned roots at memo/checkpoint
/// boundaries. Runtime coordinates are resolved and masked before hashing.
pub struct EngineBoundaryHasher<'a, G> {
    universe: &'a Universe<G>,
    hasher: crate::state_hash::StateHasher,
}

impl<G> EngineBoundaryHasher<'_, G> {
    pub fn tag(&mut self, value: u8) {
        self.hasher.tag(value);
    }
    pub fn bool(&mut self, value: bool) {
        self.hasher.bool(value);
    }
    pub fn u8(&mut self, value: u8) {
        self.hasher.u8(value);
    }
    pub fn u16(&mut self, value: u16) {
        self.hasher.u16(value);
    }
    pub fn u32(&mut self, value: u32) {
        self.hasher.u32(value);
    }
    pub fn u64(&mut self, value: u64) {
        self.hasher.u64(value);
    }
    pub fn i32(&mut self, value: i32) {
        self.hasher.i32(value);
    }
    pub fn usize(&mut self, value: usize) {
        self.hasher.usize(value);
    }
    pub fn str(&mut self, value: &str) {
        self.hasher.str(value);
    }

    pub fn token_list(&mut self, id: TokenListId<G>) {
        let admitted = self.universe.admitted().expect("live boundary generation");
        let words = admitted.token_list(id);
        self.hasher.usize(words.len());
        for word in words {
            self.hasher.u32(word.raw());
        }
    }

    pub fn glue(&mut self, id: GlueId<G>) {
        self.glue_value(self.universe.glue_value(id));
    }

    fn glue_value(&mut self, value: GlueSpec) {
        self.hasher.i32(value.width.raw());
        self.hasher.i32(value.stretch.raw());
        self.hasher.u8(value.stretch_order as u8);
        self.hasher.i32(value.shrink.raw());
        self.hasher.u8(value.shrink_order as u8);
    }

    pub fn font(&mut self, id: crate::ids::FontId) {
        let recipe = self.universe.fonts.artifact_recipe(id);
        // The artifact recipe is owned and handle-free. Its Debug vocabulary
        // is fixed by the executable build, while the surrounding state hash
        // is explicitly an in-process convergence aid rather than a durable
        // content identity.
        self.hasher.str(&format!("{recipe:?}"));
        self.universe
            .core
            .as_ref()
            .expect("live boundary generation")
            .state()
            .hash_font_runtime(id, self.universe.fonts.get(id), &mut self.hasher)
            .expect("live font has runtime state");
    }

    pub fn nodes(&mut self, nodes: &[Node]) {
        self.nodes_iter(nodes.iter());
    }

    pub fn nodes_iter<'a>(&mut self, nodes: impl ExactSizeIterator<Item = &'a Node>) {
        self.hasher.usize(nodes.len());
        for node in nodes {
            self.node(node);
        }
    }

    pub fn page_node_list(&mut self, _universe: &Universe<G>, list: PageListId) {
        let nodes = self
            .universe
            .page_node_list(list)
            .expect("boundary page root belongs to the live arena");
        self.nodes_iter(nodes.iter());
    }

    fn node(&mut self, node: &Node) {
        node.visit_semantic_node_lists(|child| {
            self.hasher.tag(0xf0);
            let child = self
                .universe
                .page_node_list(*child)
                .expect("semantic child belongs to the live page arena");
            self.nodes_iter(child.iter());
        });
        let mut value = node.clone();
        value.visit_node_lists_mut(|child| *child = PageListId::empty());
        match &mut value {
            Node::Char { font, .. } => {
                self.font(*font);
                *font = crate::font::NULL_FONT;
            }
            Node::Lig { font, .. } => {
                self.font(*font);
                *font = crate::font::NULL_FONT;
            }
            Node::MarginKern { font, .. } => {
                self.font(*font);
                *font = crate::font::NULL_FONT;
            }
            _ => {}
        }
        value.erase_diagnostic_sidecars();
        self.hasher.str(&format!("{value:?}"));
    }
}

/// Aggregate rollback roots retained while one shipout is speculative.
struct ShipoutRollback<G> {
    state: StateOperation<G>,
    page_nodes: OperationMark<PageMaterialLane>,
    page: PageCheckpointMark,
    pdf: crate::pdf::PdfStateSnapshot<G>,
    world: crate::world::WorldSnapshot,
    prepared_mag: Option<i32>,
    engine_usage: crate::command_context::EngineUsageRuntime,
}

struct PrimitiveRegistry<G> {
    names: Vec<String>,
    meanings: Vec<MeaningWord<G>>,
}

impl<G> Default for PrimitiveRegistry<G> {
    fn default() -> Self {
        Self {
            names: Vec::new(),
            meanings: Vec::new(),
        }
    }
}

struct CheckpointStateCandidate<G> {
    mark: StateCheckpointMark<G>,
    generation: GenerationCursor,
    core: AcceptedStateCoreTail<G>,
    page: AcceptedPageTail,
    source_mark: SourceMapMark,
    sources: AcceptedSourceMapTail,
    font_mark: FontStoreMark,
    fonts: AcceptedFontStoreTail,
    world_mark: WorldSnapshot,
    world: AcceptedWorldTail,
    dependencies: AcceptedDependencyTail,
}

/// Coarse generation owner plus every runtime root needed by an aggregate
/// executor checkpoint.
///
/// The value is opaque outside `tex-state`: consumers can retain and clone it
/// but cannot extract arena marks or individual store owners.
pub struct RuntimeCheckpoint<G> {
    state: StateCheckpoint<G>,
    generation: GenerationCursor,
    page: PageCheckpointMark,
    pdf: crate::pdf::PdfStateSnapshot<G>,
    world: crate::world::WorldSnapshot,
    fonts: FontStoreMark,
    sources: SourceMapMark,
    hyphenation: HyphenationCheckpoint,
    dependencies: crate::dependency::DependencyTrackerSnapshot,
    interaction_mode: InteractionMode,
    prepared_mag: Option<i32>,
    engine_usage: crate::command_context::EngineUsageRuntime,
    identity_roots: RuntimeCheckpointIdentityRoots,
    retention: RuntimeCheckpointRetention,
}

/// Owner-published semantic roots for the state-layer checkpoint families.
///
/// Every field is optional independently so aggregate composition can fail
/// closed while an ownership branch is being integrated. The aggregate layer
/// cannot construct this value from cursors, owner ids, or payload scans.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct RuntimeCheckpointIdentityRoots {
    page: Option<u64>,
    world: Option<u64>,
    hyphenation: Option<u64>,
    pdf: Option<u64>,
    dependency: Option<u64>,
    source: Option<u64>,
    font: Option<u64>,
    core: Option<u64>,
}

impl RuntimeCheckpointIdentityRoots {
    #[must_use]
    pub const fn page(self) -> Option<u64> {
        self.page
    }
    #[must_use]
    pub const fn world(self) -> Option<u64> {
        self.world
    }
    #[must_use]
    pub const fn hyphenation(self) -> Option<u64> {
        self.hyphenation
    }
    #[must_use]
    pub const fn pdf(self) -> Option<u64> {
        self.pdf
    }
    #[must_use]
    pub const fn dependency(self) -> Option<u64> {
        self.dependency
    }
    #[must_use]
    pub const fn source(self) -> Option<u64> {
        self.source
    }
    #[must_use]
    pub const fn font(self) -> Option<u64> {
        self.font
    }
    #[must_use]
    pub const fn core(self) -> Option<u64> {
        self.core
    }
}

/// Allocation-independent logical retained-byte charge for the runtime-owned
/// component families captured by one aggregate checkpoint.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RuntimeCheckpointRetention {
    core_owner: crate::CheckpointOwnerId,
    page_owner: crate::CheckpointOwnerId,
    world_owner: crate::CheckpointOwnerId,
    hyphenation_owner: crate::CheckpointOwnerId,
    pdf_owner: crate::CheckpointOwnerId,
    dependency_owner: crate::CheckpointOwnerId,
    source_font_owner: crate::CheckpointOwnerId,
    core: usize,
    page: usize,
    world: usize,
    hyphenation: usize,
    pdf: usize,
    dependency: usize,
    source_font: usize,
}

/// Whole-root font work performed by ordinary runtime checkpoint operations.
///
/// These counters are intentionally zero-only. Font-bearing meanings, nodes,
/// and PDF records validate their coordinates at publication, before the
/// monotonic font watermark can be captured. Capture and restore therefore
/// validate fixed owner/cursor marks and never revisit those payloads.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct RuntimeCheckpointFontScanCounters {
    pub capture_root_visits: u64,
    pub restore_root_visits: u64,
}

impl RuntimeCheckpointRetention {
    #[must_use]
    pub const fn core_owner(self) -> crate::CheckpointOwnerId {
        self.core_owner
    }
    #[must_use]
    pub const fn page_owner(self) -> crate::CheckpointOwnerId {
        self.page_owner
    }
    #[must_use]
    pub const fn world_owner(self) -> crate::CheckpointOwnerId {
        self.world_owner
    }
    #[must_use]
    pub const fn hyphenation_owner(self) -> crate::CheckpointOwnerId {
        self.hyphenation_owner
    }
    #[must_use]
    pub const fn pdf_owner(self) -> crate::CheckpointOwnerId {
        self.pdf_owner
    }
    #[must_use]
    pub const fn dependency_owner(self) -> crate::CheckpointOwnerId {
        self.dependency_owner
    }
    #[must_use]
    pub const fn source_font_owner(self) -> crate::CheckpointOwnerId {
        self.source_font_owner
    }

    #[must_use]
    pub const fn core_bytes(self) -> usize {
        self.core
    }

    #[must_use]
    pub const fn page_bytes(self) -> usize {
        self.page
    }

    #[must_use]
    pub const fn world_bytes(self) -> usize {
        self.world
    }

    #[must_use]
    pub const fn hyphenation_bytes(self) -> usize {
        self.hyphenation
    }

    #[must_use]
    pub const fn pdf_bytes(self) -> usize {
        self.pdf
    }

    #[must_use]
    pub const fn dependency_bytes(self) -> usize {
        self.dependency
    }

    #[must_use]
    pub const fn source_font_bytes(self) -> usize {
        self.source_font
    }
}

impl<G> Clone for RuntimeCheckpoint<G> {
    fn clone(&self) -> Self {
        Self {
            state: self.state.clone(),
            generation: self.generation,
            page: self.page,
            pdf: self.pdf.clone(),
            world: self.world.clone(),
            fonts: self.fonts,
            sources: self.sources,
            hyphenation: self.hyphenation.clone(),
            dependencies: self.dependencies,
            interaction_mode: self.interaction_mode,
            prepared_mag: self.prepared_mag,
            engine_usage: self.engine_usage.clone(),
            identity_roots: self.identity_roots,
            retention: self.retention,
        }
    }
}

impl<G> RuntimeCheckpoint<G> {
    #[doc(hidden)]
    pub fn pdf_history_position(&self) -> (u64, u64) {
        self.pdf.history_position()
    }

    #[must_use]
    pub const fn retention(&self) -> RuntimeCheckpointRetention {
        self.retention
    }

    #[must_use]
    pub const fn reachable_state_identity_roots(&self) -> RuntimeCheckpointIdentityRoots {
        self.identity_roots
    }
}

impl<G> std::fmt::Debug for RuntimeCheckpoint<G> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("RuntimeCheckpoint(..)")
    }
}

/// Standalone source/font ownership fixture for the deterministic checkpoint
/// allocation gate. It deliberately excludes dense engine state so the gate
/// attributes only these two coarse stores.
#[cfg(feature = "profiling")]
pub struct SourceFontCheckpointHarness {
    sources: SourceMap,
    fonts: FontStore,
}

#[cfg(feature = "profiling")]
#[derive(Clone, Copy)]
pub struct SourceFontCheckpointMark {
    sources: SourceMapMark,
    fonts: FontStoreMark,
}

#[cfg(feature = "profiling")]
impl SourceFontCheckpointHarness {
    #[must_use]
    pub fn with_units(units: usize) -> Self {
        let mut harness = Self {
            sources: SourceMap::default(),
            fonts: FontStore::new(),
        };
        for index in 0..units {
            harness.append_unit(index);
        }
        harness
    }

    pub fn append_unit(&mut self, index: usize) {
        let source = crate::input::SourceId::new(index as u32);
        let bytes: std::sync::Arc<[u8]> = vec![index as u8; 96].into();
        self.sources
            .register_without_line_starts(
                source,
                crate::source_map::SourceDescriptor::named_generated(
                    format!("checkpoint-{index:04}.tex"),
                    bytes,
                ),
            )
            .expect("profiling source fits the logical position space");
        let font = tex_fonts::LoadedFont::new(
            format!("checkpoint-font-{index:04}"),
            std::path::PathBuf::from(format!("checkpoint-font-{index:04}.tfm")),
            tex_fonts::font_content_hash(&[index as u8; 32]),
            index as u32,
            Scaled::from_raw(10 * Scaled::UNITY),
            Scaled::from_raw(10 * Scaled::UNITY),
            vec![Scaled::from_raw(0); 7],
            tex_fonts::FontMetrics::default(),
        );
        self.fonts
            .intern(font)
            .expect("profiling font fits the store");
    }

    #[must_use]
    pub fn checkpoint(&self) -> SourceFontCheckpointMark {
        SourceFontCheckpointMark {
            sources: self.sources.watermark(),
            fonts: self.fonts.watermark(),
        }
    }

    #[must_use]
    pub fn fork(&self, mark: SourceFontCheckpointMark) -> Self {
        Self {
            sources: self.sources.fork_at(mark.sources),
            fonts: self.fonts.fork_at(mark.fonts),
        }
    }

    #[must_use]
    pub fn retained_payload_bytes(&self) -> usize {
        self.sources
            .retained_payload_bytes()
            .saturating_add(self.fonts.retained_payload_bytes())
    }
}

/// Exclusive aggregate transaction for one staged shipout.
pub struct ShipoutTransaction<'a, G> {
    universe: &'a mut Universe<G>,
    rollback: Option<ShipoutRollback<G>>,
    scratch: Option<ShipoutScratchMark>,
    empty_tokens: TokenListId<G>,
}

impl<G> std::ops::Deref for ShipoutTransaction<'_, G> {
    type Target = Universe<G>;

    fn deref(&self) -> &Self::Target {
        self.universe
    }
}

impl<G> std::ops::DerefMut for ShipoutTransaction<'_, G> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.universe
    }
}

impl<G> Drop for ShipoutTransaction<'_, G> {
    fn drop(&mut self) {
        if let Some(rollback) = self.rollback.take() {
            self.universe.page.rollback_transaction(rollback.page);
            self.universe.pdf.rollback(rollback.pdf);
            self.universe.world.rollback(&rollback.world);
            self.universe.prepared_mag = rollback.prepared_mag;
            self.universe.engine_usage = rollback.engine_usage;
            self.universe
                .restore_state(rollback.state)
                .expect("validated shipout rollback remains restorable");
            self.universe
                .page_nodes
                .restore_operation(rollback.page_nodes)
                .expect("shipout page cursor remains valid");
        }
        self.universe.shipout_scratch.reset(
            self.scratch
                .take()
                .expect("shipout transaction owns one scratch suffix"),
        );
    }
}

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

/// Destination-local coordinates produced by cold format admission.
///
/// Definition positions retain the wire row numbering so sparse environment
/// roots can relocate directly without materializing unreachable history.
pub(crate) struct FormatPromotionReceipt<G> {
    pub definitions: Vec<Option<DefinitionId<G>>>,
    pub token_lists: Vec<TokenListId<G>>,
    pub glue: Vec<GlueId<G>>,
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
pub type StateCheckpointMark<G, Input = DenseStateCursor> =
    BoundedStateMark<JournalCursor<G>, (), CheckpointMark<PageMaterialLane>, Input>;

/// Coarse generation owner plus bounded state cursors.
pub type StateCheckpoint<G, Input = DenseStateCursor> =
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
    pub(crate) interner: Option<InternerLease>,
    pub(crate) core: Option<StateCore<G>>,
    pub(crate) page_nodes: PageNodeArena,
    checkpoint_candidate: Option<CheckpointStateCandidate<G>>,
    shipout_scratch: ShipoutScratchArena<G>,
    pub(crate) fonts: FontStore,
    pub(crate) page: PageBuilderState,
    page_lent_to_candidate: bool,
    pub(crate) pdf: PdfStateSlot<G>,
    sources: SourceMap,
    pub(crate) hyphenation: HyphenationTable,
    pub(crate) world: World,
    dependencies: DependencyRuntime,
    pub(crate) interaction_mode: InteractionMode,
    /// TeX82 §288's job-level `mag_set`; deliberately absent from formats.
    prepared_mag: Option<i32>,
    error_context_widths: ErrorContextWidths,
    pub(crate) engine_usage: crate::command_context::EngineUsageRuntime,
    dynamic_memory_scratch: crate::stores::DynamicMemoryScratch<G>,
    pub(crate) provenance_demand: crate::ProvenanceDemand,
    pub(crate) provenance_budgets: crate::ProvenanceBudgets,
    primitive_registry: Rc<PrimitiveRegistry<G>>,
    command_generation_owner: Option<GenerationOwner<G>>,
    /// Driver-requested cache policy consumed exactly once by MainControl.
    pure_memo_config: Option<crate::PureMemoConfig>,
    /// Borrow-only capability for the execution-owned cache service.
    pure_memo_capability: std::sync::Weak<std::sync::Mutex<crate::PureMemoRuntime>>,
    restore_owner: Option<GenerationOwner<G>>,
    checkpoint_font_scan_counters: RuntimeCheckpointFontScanCounters,
}

impl<G> Universe<G> {
    fn checkpoint_state_is_ready(&self, checkpoint: &RuntimeCheckpoint<G>) -> bool {
        let mark = checkpoint.state.mark();
        let Some(core) = self.core.as_ref() else {
            return false;
        };
        core.owns_generation(checkpoint.state.owner())
            && core.state().validate_restore(*mark.journal()).is_ok()
            && core.state().validate_checkpoint_cursor(*mark.input())
            && core.validate_durable_node_cursor(*mark.durable()).is_ok()
            && core.validates_generation_cursor(checkpoint.generation)
            && self.page_nodes.can_restore_checkpoint(*mark.page())
    }

    fn activate_checkpoint_state(
        &mut self,
        checkpoint: &RuntimeCheckpoint<G>,
    ) -> Result<(), UniverseError> {
        let mark = *checkpoint.state.mark();
        let core = self.core.as_mut().ok_or(UniverseError::Retired)?;
        core.state_mut().restore(*mark.journal())?;
        core.state_mut().restore_checkpoint_cursor(*mark.input());
        core.restore_durable_node_cursor(*mark.durable())?;
        core.restore_generation_cursor(checkpoint.generation);
        Ok(())
    }

    /// Profiles the isolated page-owner fork/mutate/reject seam without
    /// constructing the independently owned aggregate fork families.
    #[doc(hidden)]
    #[cfg(feature = "profiling")]
    pub fn profile_page_owner_cycle(
        &mut self,
        checkpoint: &RuntimeCheckpoint<G>,
    ) -> Result<[u64; 6], UniverseError> {
        if !self.page.validates_checkpoint_mark(checkpoint.page) {
            return Err(UniverseError::State(StateError::InvalidCursor));
        }
        let before = self.page.checkpoint_replay_work();
        let mut page = std::mem::take(&mut self.page);
        page.begin_checkpoint_fork(checkpoint.page);
        page.push_contribution(&mut self.page_nodes, crate::node::Node::Penalty(19));
        let contribution = if let Some(carrier) = page.pop_contribution_front(&mut self.page_nodes)
        {
            page.discard_carrier(carrier);
            1
        } else {
            0
        };
        let current = u64::from(page.pop_current_page(&mut self.page_nodes).is_some());
        page.clear_page_discards(&self.page_nodes);
        page.clear_split_discards(&self.page_nodes);
        page.upsert_page_insertion(crate::page::PageInsertion::new(
            7,
            crate::scaled::Scaled::from_raw(29),
        ));
        page.set_mark_class(
            crate::page::PageMark::Bot,
            7,
            crate::node::NodeTokenList::new([crate::token::TokenWord::pack(
                crate::token::Token::Char {
                    ch: 'c',
                    cat: crate::token::Catcode::Other,
                },
            )]),
        );
        page.reject_checkpoint_fork();
        let replay = page.checkpoint_replay_work().saturating_sub(before);
        self.page = page;
        Ok([replay, contribution, current, 1, 1, 2])
    }
    #[doc(hidden)]
    pub fn prune_pdf_history(&mut self, low_water: (u64, u64)) {
        self.pdf.prune_history(low_water);
    }

    #[doc(hidden)]
    pub fn pdf_history_head(&self) -> (u64, u64) {
        self.pdf.history_head()
    }

    /// Direct PDF mark loop used by the focused profiling gate. Other
    /// aggregate checkpoint owners are deliberately excluded.
    #[doc(hidden)]
    #[cfg(feature = "profiling")]
    pub fn profile_pdf_checkpoint_capture(&self, iterations: usize) -> u64 {
        let mut checksum = 0_u64;
        for _ in 0..iterations {
            let mark = std::hint::black_box(self.pdf.snapshot());
            let position = mark.history_position();
            checksum ^= position.0 ^ position.1.rotate_left(17);
        }
        checksum
    }

    #[doc(hidden)]
    #[cfg(feature = "profiling")]
    pub fn profile_pdf_checkpoint_restore(&mut self, iterations: usize) -> u64 {
        let mark = self.pdf.snapshot();
        let mut checksum = 0_u64;
        for _ in 0..iterations {
            self.pdf.rollback(mark.clone());
            checksum ^= self.pdf.history_head().0;
        }
        checksum
    }

    #[doc(hidden)]
    #[cfg(feature = "profiling")]
    pub fn profile_pdf_payload_bytes(&self) -> usize {
        self.pdf.payload_bytes()
    }

    /// Creates a destination-local runtime from one retained checkpoint.
    /// Validation is complete before the returned fork becomes visible.
    #[doc(hidden)]
    pub fn fork_runtime_checkpoint(
        &mut self,
        checkpoint: &RuntimeCheckpoint<G>,
    ) -> Result<Self, UniverseError> {
        let mark = checkpoint.state.mark();
        if !self.world.snapshot_is_forkable(&checkpoint.world)
            || !self.pdf.snapshot_is_retained(&checkpoint.pdf)
            || !self.fonts.validates(checkpoint.fonts)
            || !self.sources.validates(checkpoint.sources)
            || !self.page.validates_checkpoint_mark(checkpoint.page)
            || !self.checkpoint_state_is_ready(checkpoint)
        {
            return Err(UniverseError::State(StateError::InvalidCursor));
        }

        let core_tail = self
            .core
            .as_mut()
            .ok_or(UniverseError::Retired)?
            .begin_checkpoint_candidate(
                *mark.journal(),
                *mark.input(),
                *mark.durable(),
                checkpoint.generation,
            )?;
        // PageBuilder roots must select the checkpoint prefix while every
        // accepted page-material chunk is still attached. The arena selection
        // was prevalidated above, so detachment is infallible after this root
        // rewind.
        let page_tail = self.page.begin_checkpoint_candidate(checkpoint.page);
        self.page_nodes
            .begin_checkpoint_candidate(*mark.page())
            .expect("prevalidated page-material checkpoint can detach");
        let world_tail = self.world.begin_checkpoint_candidate(&checkpoint.world);
        let dependency_tail = self
            .dependencies
            .begin_checkpoint_candidate(&checkpoint.dependencies);
        let source_tail = self.sources.begin_checkpoint_candidate(checkpoint.sources);
        let font_tail = self.fonts.begin_checkpoint_candidate(checkpoint.fonts);
        let core = self
            .core
            .take()
            .expect("validated source owns its state core");
        let page_nodes = std::mem::take(&mut self.page_nodes);
        let sources = std::mem::take(&mut self.sources);
        let fonts = std::mem::take(&mut self.fonts);
        let world = std::mem::take(&mut self.world);
        let dependencies = std::mem::take(&mut self.dependencies);
        let destination_owner = core.generation_owner();
        let pdf = self.pdf.take_candidate(&checkpoint.pdf);
        let page = std::mem::take(&mut self.page);
        self.page_lent_to_candidate = true;
        let fork = Self {
            interner: None,
            core: Some(core),
            page_nodes,
            checkpoint_candidate: Some(CheckpointStateCandidate {
                mark: *mark,
                generation: checkpoint.generation,
                core: core_tail,
                page: page_tail,
                source_mark: checkpoint.sources,
                sources: source_tail,
                font_mark: checkpoint.fonts,
                fonts: font_tail,
                world_mark: checkpoint.world.clone(),
                world: world_tail,
                dependencies: dependency_tail,
            }),
            shipout_scratch: ShipoutScratchArena::default(),
            fonts,
            page,
            page_lent_to_candidate: false,
            pdf,
            sources,
            hyphenation: HyphenationTable::from_checkpoint(&checkpoint.hyphenation),
            world,
            dependencies,
            interaction_mode: checkpoint.interaction_mode,
            prepared_mag: checkpoint.prepared_mag,
            error_context_widths: self.error_context_widths,
            engine_usage: checkpoint.engine_usage.clone(),
            dynamic_memory_scratch: crate::stores::DynamicMemoryScratch::default(),
            provenance_demand: self.provenance_demand,
            provenance_budgets: self.provenance_budgets,
            primitive_registry: Rc::clone(&self.primitive_registry),
            command_generation_owner: Some(destination_owner),
            pure_memo_config: self.pure_memo_config,
            pure_memo_capability: self.pure_memo_capability.clone(),
            restore_owner: None,
            checkpoint_font_scan_counters: self.checkpoint_font_scan_counters,
        };
        Ok(fork)
    }

    #[doc(hidden)]
    pub fn reject_checkpoint_candidate(&mut self, candidate: &mut Self) {
        let Some(transaction) = candidate.checkpoint_candidate.take() else {
            // Emergency Drop can revisit a candidate after unwinding began.
            // Normal settlement always arrives here exactly once.
            return;
        };
        let mark = transaction.mark;
        let page_boundary = candidate
            .page_nodes
            .seal_boundary()
            .expect("checkpoint settlement seals page material before root mutation");
        candidate
            .page
            .prepare_checkpoint_candidate_rejection(&transaction.page);
        candidate
            .page_nodes
            .reject_checkpoint_candidate(page_boundary)
            .expect("prevalidated candidate page nodes can release current and reattach prior");
        candidate
            .page
            .finish_checkpoint_candidate_rejection(transaction.page);
        candidate
            .fonts
            .reject_checkpoint_candidate(transaction.font_mark, transaction.fonts);
        candidate
            .sources
            .reject_checkpoint_candidate(transaction.source_mark, transaction.sources);
        candidate
            .dependencies
            .reject_checkpoint_candidate(transaction.dependencies);
        candidate
            .world
            .reject_checkpoint_candidate(&transaction.world_mark, transaction.world);
        let mut core = candidate
            .core
            .take()
            .expect("the current lineage owns the direct state core");
        core.reject_checkpoint_candidate(
            *mark.journal(),
            *mark.input(),
            *mark.durable(),
            transaction.generation,
            transaction.core,
        )
        .expect("validated candidate state can undo and redo");
        self.core = Some(core);
        self.page_nodes = std::mem::take(&mut candidate.page_nodes);
        self.sources = std::mem::take(&mut candidate.sources);
        self.fonts = std::mem::take(&mut candidate.fonts);
        self.world = std::mem::take(&mut candidate.world);
        self.dependencies = std::mem::take(&mut candidate.dependencies);
        self.pdf.return_rejected(&mut candidate.pdf);
        self.page = std::mem::take(&mut candidate.page);
        self.page_lent_to_candidate = false;
    }

    pub(crate) fn accept_checkpoint_candidate(&mut self) {
        let transaction = self
            .checkpoint_candidate
            .take()
            .expect("the current lineage owns one rooted state transaction");
        let page_boundary = self
            .page_nodes
            .seal_boundary()
            .expect("checkpoint settlement seals page material before root mutation");
        self.page
            .prepare_checkpoint_candidate_acceptance(transaction.page);
        self.page_nodes
            .accept_checkpoint_candidate(page_boundary)
            .expect("prevalidated candidate page nodes can prune prior and promote current");
        self.core
            .as_mut()
            .expect("the current lineage owns the direct state core")
            .accept_checkpoint_candidate(transaction.core);
        self.world.accept_checkpoint_candidate(transaction.world);
        self.dependencies
            .accept_checkpoint_candidate(transaction.dependencies);
        self.sources
            .accept_checkpoint_candidate(transaction.sources);
        self.fonts.accept_checkpoint_candidate(transaction.fonts);
        self.pdf.commit_candidate();
    }

    #[doc(hidden)]
    #[cfg(feature = "profiling")]
    pub fn profile_commit_checkpoint_candidate(&mut self) {
        self.accept_checkpoint_candidate();
    }

    pub(crate) fn interner(&self) -> &Interner {
        self.interner
            .as_deref()
            .expect("live Universe has an admitted session epoch")
    }

    pub(crate) fn interner_mut(&mut self) -> &mut Interner {
        self.interner
            .as_deref_mut()
            .expect("live Universe has an admitted session epoch")
    }

    pub(crate) fn release_session_epoch(&mut self) -> InternerLease {
        self.interner
            .take()
            .expect("retained generation releases one admitted session epoch")
    }

    pub(crate) fn admit_session_epoch(&mut self, interner: InternerLease) {
        assert!(
            self.interner.replace(interner).is_none(),
            "retained generation admits its session epoch exactly once"
        );
    }

    /// Atomically installs one canonical fresh profile layer in dense state.
    /// Restored-format construction uses primitive registration instead and
    /// must not call this method.
    pub fn install_fresh_parameter_profile(
        &mut self,
        profile: crate::FreshParameterProfile,
        defaults: &[crate::FreshParameterDefault],
    ) -> Result<crate::FreshParameterInstallation, crate::FreshParameterInstallError> {
        let installation = self
            .core
            .as_mut()
            .ok_or(crate::FreshParameterInstallError::Retired)?
            .state_mut()
            .install_fresh_parameter_profile(profile, defaults)?;
        if profile == crate::FreshParameterProfile::Etex26 {
            self.engine_usage.select_etex26_profile();
        }
        Ok(installation)
    }

    /// Refreshes tex.web §241's four volatile clock parameters from the
    /// current host world without changing any restored format-owned cell.
    pub fn refresh_job_clock_parameters(&mut self) -> Result<(), UniverseError> {
        let clock = self.world.job_clock();
        self.core
            .as_mut()
            .ok_or(UniverseError::Retired)?
            .state_mut()
            .refresh_job_clock(clock);
        Ok(())
    }

    #[must_use]
    pub fn with_provenance_config(
        mut self,
        demand: crate::ProvenanceDemand,
        budgets: crate::ProvenanceBudgets,
    ) -> Self {
        self.provenance_demand = demand;
        self.provenance_budgets = budgets;
        self
    }

    /// Installs the explicit provenance consumer policy before an admitted
    /// job begins. Unlike the builder form, this works inside an HRTB fresh or
    /// format-materialization callback without moving the branded universe.
    pub fn set_provenance_config(
        &mut self,
        demand: crate::ProvenanceDemand,
        budgets: crate::ProvenanceBudgets,
    ) {
        self.provenance_demand = demand;
        self.provenance_budgets = budgets;
    }

    /// Changes only the provenance consumer demand, retaining its budgets.
    pub fn set_provenance_demand(&mut self, demand: crate::ProvenanceDemand) {
        self.provenance_demand = demand;
    }

    #[must_use]
    pub fn with_provenance_demand(self, demand: crate::ProvenanceDemand) -> Self {
        let budgets = self.provenance_budgets;
        self.with_provenance_config(demand, budgets)
    }

    #[must_use]
    pub const fn provenance_demand(&self) -> crate::ProvenanceDemand {
        self.provenance_demand
    }

    #[must_use]
    pub const fn provenance_budgets(&self) -> crate::ProvenanceBudgets {
        self.provenance_budgets
    }

    /// Opens the outer transaction barrier for one dependency-observed
    /// command episode. Hot reads are recorded through `CommandContext`.
    pub fn begin_dependency_region(
        &mut self,
    ) -> Result<DependencyRegionToken, DependencyRegionError> {
        self.dependencies.begin_region()
    }

    /// Publishes detached dependency evidence after a command episode.
    pub fn finish_dependency_region(
        &mut self,
        token: DependencyRegionToken,
    ) -> Result<Vec<ObservedDependency>, DependencyRegionError> {
        self.dependencies.finish_region(token)
    }

    /// Discards an incomplete dependency episode without publishing it.
    pub fn abandon_dependency_region(
        &mut self,
        token: DependencyRegionToken,
    ) -> Result<(), DependencyRegionError> {
        self.dependencies.abandon_region(token)
    }

    /// Records why the active dependency episode cannot be memoized.
    pub fn poison_dependency_region(&mut self, barrier: TrackedRegionBarrier) {
        self.dependencies.poison(barrier);
    }

    pub(crate) fn new(interner: InternerLease, mut core: StateCore<G>) -> Self {
        let fonts = FontStore::new();
        let null_font = fonts.get(crate::font::NULL_FONT);
        let prepared = core
            .state_mut()
            .prepare_font_runtime(null_font.parameters(), i32::from(b'-'), -1)
            .expect("null-font runtime row fits state storage");
        core.admit_mut()
            .expect("fresh generation admits null-font runtime state")
            .state()
            .install_font_runtime(crate::font::NULL_FONT, prepared)
            .expect("null-font runtime row is first");
        let page_nodes = PageNodeArena::new();
        let command_generation_owner = core.generation_owner();
        Self {
            interner: Some(interner),
            core: Some(core),
            page_nodes,
            checkpoint_candidate: None,
            shipout_scratch: ShipoutScratchArena::default(),
            fonts,
            page: PageBuilderState::default(),
            page_lent_to_candidate: false,
            pdf: PdfStateSlot::default(),
            sources: SourceMap::default(),
            hyphenation: HyphenationTable::new(),
            world: World::default(),
            dependencies: DependencyRuntime::default(),
            interaction_mode: InteractionMode::default(),
            prepared_mag: None,
            error_context_widths: ErrorContextWidths::default(),
            engine_usage: crate::command_context::EngineUsageRuntime::default(),
            dynamic_memory_scratch: crate::stores::DynamicMemoryScratch::default(),
            provenance_demand: crate::ProvenanceDemand::default(),
            provenance_budgets: crate::ProvenanceBudgets::default(),
            primitive_registry: Rc::new(PrimitiveRegistry::default()),
            command_generation_owner: Some(command_generation_owner),
            pure_memo_config: None,
            pure_memo_capability: std::sync::Weak::new(),
            restore_owner: None,
            checkpoint_font_scan_counters: RuntimeCheckpointFontScanCounters::default(),
        }
    }

    /// Reports the structural zero-scan checkpoint contract for fonts.
    #[must_use]
    pub const fn runtime_checkpoint_font_scan_counters(&self) -> RuntimeCheckpointFontScanCounters {
        self.checkpoint_font_scan_counters
    }

    /// Observes one TeX82 §259 lookup outside every group and rollback cursor,
    /// then admits the issued compact slot into the dense meaning bank.
    pub fn intern(&mut self, name: &str) -> Result<SymbolId, UniverseError> {
        if self.core.is_none() {
            return Err(UniverseError::Retired);
        }
        let (symbol, created) = self.interner_mut().intern_hash_with_status(name)?;
        if created && name.chars().nth(1).is_some() {
            self.engine_usage.make_string(name);
        }
        self.core
            .as_mut()
            .ok_or(UniverseError::Retired)?
            .admit_mut()?
            .state()
            .admit_symbol(symbol.symbol())?;
        Ok(symbol)
    }

    pub fn resolve_symbol(&self, symbol: SymbolId) -> Result<&str, UniverseError> {
        Ok(self.interner().resolve_id(symbol)?)
    }

    #[must_use]
    pub fn resolve(&self, symbol: Symbol) -> Option<&str> {
        self.interner().resolve_local(symbol)
    }

    #[must_use]
    pub fn qualify_symbol(&self, symbol: Symbol) -> Option<SymbolId> {
        self.interner().qualify_local(symbol)
    }

    #[must_use]
    pub fn control_sequence_kind(&self, symbol: Symbol) -> Option<ControlSequenceKind> {
        self.qualify_symbol(symbol)
            .and_then(|id| self.interner().kind_id(id).ok())
    }

    #[must_use]
    pub fn active_character_symbol(&self, ch: char) -> Option<SymbolId> {
        self.interner().active(ch)
    }

    pub fn intern_active_character(&mut self, ch: char) -> Result<SymbolId, UniverseError> {
        let symbol = self.interner_mut().intern_active(ch)?;
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
        self.register_primitive_word(name, MeaningWord::from_static(meaning));
    }

    /// Records a static or generation-local frozen primitive meaning.
    pub fn register_primitive_word(&mut self, name: &str, meaning: MeaningWord<G>) {
        if let Some(font) = meaning.font() {
            assert!(
                self.fonts.contains(font),
                "frozen font meaning retains a live Universe font"
            );
        }
        if let Some(index) = self
            .primitive_registry
            .names
            .iter()
            .position(|candidate| candidate == name)
        {
            assert_eq!(self.primitive_registry.meanings[index], meaning);
            return;
        }
        let registry = Rc::get_mut(&mut self.primitive_registry)
            .expect("primitive registration completes before a checkpoint shares the registry");
        let index = registry.names.len();
        assert!(
            index < 60_000 - 2,
            "primitive registry exceeds frozen-token capacity"
        );
        registry.names.push(name.to_owned());
        registry.meanings.push(meaning);
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
        self.primitive_registry
            .names
            .iter()
            .position(|candidate| candidate == name)
            .and_then(
                |index| match self.primitive_registry.meanings[index].resolve() {
                    crate::ResolvedMeaning::Static(meaning) => Some(meaning),
                    crate::ResolvedMeaning::Macro { .. } => None,
                },
            )
    }

    /// Resolves one immutable primitive row once into a packed direct handle.
    ///
    /// The handle names the frozen registry, not the mutable control-sequence
    /// cell bearing the same spelling.  It therefore cannot bypass `\def`,
    /// `\let`, or another assignment to that control sequence.
    #[must_use]
    pub fn primitive_handle(&self, name: &str) -> Option<crate::PrimitiveHandle<G>> {
        let index = self
            .primitive_registry
            .names
            .iter()
            .position(|candidate| candidate == name)?;
        self.primitive_registry.meanings[index].static_meaning()?;
        Some(crate::PrimitiveHandle::new(
            self.interner().epoch_identity(),
            u16::try_from(index).ok()?,
            u16::try_from(self.primitive_registry.meanings.len()).ok()?,
        ))
    }

    /// Resolves a packed immutable primitive handle by direct indexing.
    #[must_use]
    pub fn resolve_primitive_handle(&self, handle: crate::PrimitiveHandle<G>) -> Option<Meaning> {
        if handle.session_epoch() != self.interner().epoch_identity()
            || handle.registry_len() != self.primitive_registry.meanings.len()
        {
            return None;
        }
        self.primitive_registry
            .meanings
            .get(handle.index())?
            .static_meaning()
    }

    /// Returns the current append-only primitive-registry extent.
    #[doc(hidden)]
    #[must_use]
    pub fn primitive_registry_len(&self) -> usize {
        self.primitive_registry.meanings.len()
    }

    #[must_use]
    pub fn primitive_name(&self, meaning: Meaning) -> Option<&str> {
        self.primitive_registry
            .meanings
            .iter()
            .position(|candidate| {
                matches!(candidate.resolve(), crate::ResolvedMeaning::Static(value) if value == meaning)
            })
            .map(|index| self.primitive_registry.names[index].as_str())
    }

    #[must_use]
    pub fn primitive_token(&self, name: &str) -> Option<crate::token::Token> {
        let index = self
            .primitive_registry
            .names
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
        self.primitive_registry
            .names
            .get(usize::from(frozen.primitive_index()?))
            .map(String::as_str)
    }

    #[must_use]
    pub fn frozen_primitive_meaning(&self, token: crate::token::Token) -> Option<Meaning> {
        match self.frozen_primitive_resolved(token)? {
            crate::ResolvedMeaning::Static(meaning) => Some(meaning),
            crate::ResolvedMeaning::Macro { .. } => None,
        }
    }

    #[must_use]
    pub fn frozen_primitive_resolved(
        &self,
        token: crate::token::Token,
    ) -> Option<crate::ResolvedMeaning<G>> {
        let crate::token::Token::Frozen(frozen) = token else {
            return None;
        };
        self.primitive_registry
            .meanings
            .get(usize::from(frozen.primitive_index()?))
            .cloned()
            .map(|meaning| meaning.resolve())
    }

    #[must_use]
    pub fn catcode(&self, ch: char) -> crate::token::Catcode {
        let raw = self
            .live_state()
            .ok()
            .and_then(|state| state.code(CodeTableKind::Catcode, ch).ok())
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
        self.interner()
            .resolve_local(symbol)
            .ok_or(UniverseError::State(StateError::ForeignSession))?;
        Ok(self
            .core
            .as_ref()
            .ok_or(UniverseError::Retired)?
            .state()
            .meaning(symbol)?)
    }

    #[must_use]
    pub const fn world(&self) -> &World {
        &self.world
    }

    pub const fn world_mut(&mut self) -> &mut World {
        &mut self.world
    }

    /// Captures an opaque cursor into this job's retained terminal input.
    #[must_use]
    pub fn capture_terminal_input_position(&self) -> crate::TerminalInputPosition {
        self.world.terminal_input_position()
    }

    /// Restores a cursor only when it belongs to this exact caller World.
    /// Foreign and stale positions are rejected before the live cursor moves.
    pub fn restore_terminal_input_position(
        &mut self,
        position: crate::TerminalInputPosition,
    ) -> Result<(), crate::WorldError> {
        self.world.restore_terminal_input_position(position)
    }

    #[must_use]
    pub const fn interaction_mode(&self) -> InteractionMode {
        self.interaction_mode
    }

    pub const fn set_interaction_mode(&mut self, mode: InteractionMode) {
        self.interaction_mode = mode;
    }

    /// Opens a rollback-capable execution timeline whose effects are detached
    /// at an outer completion barrier instead of touching the host eagerly.
    pub fn begin_retained_session(&mut self) -> Result<(), crate::WorldError> {
        self.world.begin_retained_session()
    }

    /// Materializes a retained destination after its detached completion has
    /// been accepted. The effect cursor remains solely World-owned.
    pub fn export_retained_effects(&mut self) -> Result<(), crate::WorldError> {
        self.world.export_retained_effects()
    }

    /// Enables the pdfTeX document ledger for a PDF-producing engine binary.
    /// Output mode remains controlled by the ordinary `\pdfoutput` parameter.
    pub fn enable_pdf_output(&mut self) {
        self.pdf.enable();
    }

    /// Requests a bounded execution-owned pure-query cache.
    pub const fn enable_pure_memo(&mut self, config: crate::PureMemoConfig) {
        self.pure_memo_config = Some(config);
    }

    /// Clears a cache request which MainControl has not consumed.
    pub const fn disable_pure_memo(&mut self) {
        self.pure_memo_config = None;
    }

    #[doc(hidden)]
    pub fn take_pure_memo_config(&mut self) -> Option<crate::PureMemoConfig> {
        self.pure_memo_config.take()
    }

    /// Installs a borrow-only capability to MainControl's cache runtime.
    #[doc(hidden)]
    pub fn attach_pure_memo_capability(
        &mut self,
        runtime: &std::sync::Arc<std::sync::Mutex<crate::PureMemoRuntime>>,
    ) {
        self.pure_memo_capability = std::sync::Arc::downgrade(runtime);
    }

    /// Borrows the execution-owned cache without transferring its ownership.
    #[doc(hidden)]
    pub fn with_pure_memo<R>(
        &self,
        operation: impl FnOnce(&mut crate::PureMemoRuntime) -> R,
    ) -> Option<R> {
        let runtime = self.pure_memo_capability.upgrade()?;
        let mut runtime = runtime.lock().expect("memo runtime mutex is not poisoned");
        Some(operation(&mut runtime))
    }

    /// Returns the PDF controls frozen by the first committed page.
    #[must_use]
    pub fn fixed_pdf_output_parameters(&self) -> Option<crate::PdfOutputParameters> {
        self.pdf.output_parameters()
    }

    /// Projects executor-owned semantic roots into one deterministic,
    /// allocation-independent in-process convergence fingerprint.
    #[must_use]
    pub fn engine_boundary_hash(
        &self,
        domain: u64,
        build: impl FnOnce(&mut EngineBoundaryHasher<'_, G>),
    ) -> u64 {
        let mut projection = EngineBoundaryHasher {
            universe: self,
            hasher: crate::state_hash::StateHasher::new_exact(domain),
        };
        build(&mut projection);
        projection.hasher.finish()
    }

    /// Computes four domain-separated projections with one semantic traversal.
    #[must_use]
    pub fn engine_boundary_hashes(
        &self,
        domains: [u64; 4],
        build: impl FnOnce(&mut EngineBoundaryHasher<'_, G>),
    ) -> [u64; 4] {
        let mut projection = EngineBoundaryHasher {
            universe: self,
            hasher: crate::state_hash::StateHasher::new_quad(domains),
        };
        build(&mut projection);
        projection.hasher.finish_quad()
    }

    fn current_pdf_output_parameters(&self) -> crate::PdfOutputParameters {
        use crate::env::banks::IntParam;
        crate::PdfOutputParameters {
            output: self.int_param(IntParam::PDF_OUTPUT),
            major_version: self.int_param(IntParam::PDF_MAJOR_VERSION),
            minor_version: self.int_param(IntParam::PDF_MINOR_VERSION),
            compress_level: self.int_param(IntParam::PDF_COMPRESS_LEVEL),
            object_compress_level: self.int_param(IntParam::PDF_OBJ_COMPRESS_LEVEL),
            decimal_digits: self.int_param(IntParam::PDF_DECIMAL_DIGITS),
            gamma: self.int_param(IntParam::PDF_GAMMA),
            image_gamma: self.int_param(IntParam::PDF_IMAGE_GAMMA),
            image_hicolor: self.int_param(IntParam::PDF_IMAGE_HICOLOR),
            image_apply_gamma: self.int_param(IntParam::PDF_IMAGE_APPLY_GAMMA),
            draft_mode: self.int_param(IntParam::PDF_DRAFT_MODE),
            inclusion_copy_fonts: self.int_param(IntParam::PDF_INCLUSION_COPY_FONTS),
            pk_resolution: self.int_param(IntParam::PDF_PK_RESOLUTION),
            unique_resource_names: self.int_param(IntParam::PDF_UNIQUE_RESNAME),
        }
        .normalized()
    }

    fn pdf_token_parameter(&self, tokens: TokenListId<G>) -> crate::pdf::PdfTokenParameter<G> {
        let admitted = self.admitted().expect("live shipout generation");
        let words = admitted.token_list(tokens.clone());
        let semantic_id = crate::state_hash::StateHashFragment::from_exact_builder(
            0x7064_665f_746f_6b70,
            |hasher| {
                hasher.usize(words.len());
                for word in words {
                    hasher.u32(word.raw());
                }
            },
        );
        crate::pdf::PdfTokenParameter {
            tokens,
            semantic_id,
        }
    }

    fn current_pdf_page_parameters(
        &self,
        empty_tokens: TokenListId<G>,
    ) -> crate::pdf::PdfPageParameters<G> {
        use crate::env::banks::{DimenParam, IntParam, TokParam};
        let token_parameter = |parameter| {
            self.pdf_token_parameter(
                self.token_parameter(parameter)
                    .expect("PDF token parameter is admitted")
                    .unwrap_or_else(|| empty_tokens.clone()),
            )
        };
        crate::pdf::PdfPageParameters {
            h_origin: self
                .dimen_param(DimenParam::PDF_H_ORIGIN)
                .expect("PDF dimension parameter is admitted"),
            v_origin: self
                .dimen_param(DimenParam::PDF_V_ORIGIN)
                .expect("PDF dimension parameter is admitted"),
            width: self
                .dimen_param(DimenParam::PDF_PAGE_WIDTH)
                .expect("PDF dimension parameter is admitted"),
            height: self
                .dimen_param(DimenParam::PDF_PAGE_HEIGHT)
                .expect("PDF dimension parameter is admitted"),
            link_margin: self
                .dimen_param(DimenParam::PDF_LINK_MARGIN)
                .expect("PDF dimension parameter is admitted"),
            page_attr: token_parameter(TokParam::PDF_PAGE_ATTR),
            resources: token_parameter(TokParam::PDF_PAGE_RESOURCES),
            omit_procset: self.int_param(IntParam::PDF_OMIT_PROCSET),
            space_font_name: self.pdf.current_space_font_name_id(),
        }
    }

    /// Returns the process-selected tex.web §3 display widths.
    #[must_use]
    pub const fn error_context_widths(&self) -> ErrorContextWidths {
        self.error_context_widths
    }

    /// Replaces operational error-display widths outside semantic state.
    pub const fn set_error_context_widths(&mut self, widths: ErrorContextWidths) {
        self.error_context_widths = widths;
    }

    #[must_use]
    pub fn int_param(&self, parameter: crate::env::banks::IntParam) -> i32 {
        self.core
            .as_ref()
            .and_then(|core| core.admit().state().integer_parameter(parameter).ok())
            .unwrap_or(0)
    }

    /// Reads one admitted count-register value.
    pub fn count(&self, index: u16) -> Result<i32, UniverseError> {
        Ok(self.live_state()?.count(index)?)
    }

    /// Reads one admitted dimension-register value.
    pub fn dimension(&self, index: u16) -> Result<Scaled, UniverseError> {
        Ok(self.live_state()?.dimension(index)?)
    }

    /// Reads one admitted dimension parameter.
    pub fn dimen_param(
        &self,
        parameter: crate::env::banks::DimenParam,
    ) -> Result<Scaled, UniverseError> {
        Ok(self.live_state()?.dimension_parameter(parameter)?)
    }

    /// Reads one admitted token-register root.
    pub fn token_register(&self, index: u16) -> Result<Option<TokenListId<G>>, UniverseError> {
        Ok(self.live_state()?.token_register(index)?)
    }

    /// Reads one admitted token-parameter root.
    pub fn token_parameter(
        &self,
        parameter: crate::env::banks::TokParam,
    ) -> Result<Option<TokenListId<G>>, UniverseError> {
        Ok(self.live_state()?.token_parameter(parameter)?)
    }

    /// Reads one admitted ordinary glue-register root.
    pub fn glue_register(&self, index: u16) -> Result<Option<GlueId<G>>, UniverseError> {
        Ok(self.live_state()?.glue_register(index)?)
    }

    /// Reads one admitted math-glue-register root.
    pub fn mu_glue_register(&self, index: u16) -> Result<Option<GlueId<G>>, UniverseError> {
        Ok(self.live_state()?.mu_glue_register(index)?)
    }

    /// Reads one admitted glue-parameter root.
    pub fn glue_parameter(
        &self,
        parameter: crate::env::banks::GlueParam,
    ) -> Result<Option<GlueId<G>>, UniverseError> {
        Ok(self.live_state()?.glue_parameter(parameter)?)
    }

    /// Resolves a durable glue coordinate under this generation.
    pub fn glue(&self, id: GlueId<G>) -> Result<GlueSpec, UniverseError> {
        Ok(self
            .core
            .as_ref()
            .ok_or(UniverseError::Retired)?
            .admit()
            .glue(id))
    }

    /// Returns the current text font from admitted dense state.
    pub fn current_font(&self) -> Result<crate::ids::FontId, UniverseError> {
        Ok(self.live_state()?.current_font())
    }

    /// Returns one current math-family font from admitted dense state.
    pub fn math_family_font(
        &self,
        size: crate::math::MathFontSize,
        family: u8,
    ) -> Result<crate::ids::FontId, UniverseError> {
        let index = u8::try_from(size.index())
            .expect("math font size is bounded")
            .saturating_mul(16)
            .saturating_add(family);
        Ok(self.live_state()?.math_family_font(index)?)
    }

    /// Assigns one dimension parameter through the exact save journal.
    pub fn assign_dimen_param(
        &mut self,
        parameter: crate::env::banks::DimenParam,
        value: Scaled,
        scope: AssignmentScope,
    ) -> Result<(), UniverseError> {
        self.live_state_mut()?
            .assign_dimension_parameter(parameter, value, scope)?;
        Ok(())
    }

    /// Assigns one token parameter through the exact save journal.
    pub fn assign_token_parameter(
        &mut self,
        parameter: crate::env::banks::TokParam,
        value: Option<TokenListId<G>>,
        scope: AssignmentScope,
    ) -> Result<(), UniverseError> {
        self.live_state_mut()?
            .assign_token_parameter(parameter, value, scope)?;
        Ok(())
    }

    /// Assigns one glue parameter through the exact save journal.
    pub fn assign_glue_parameter(
        &mut self,
        parameter: crate::env::banks::GlueParam,
        value: Option<GlueId<G>>,
        scope: AssignmentScope,
    ) -> Result<(), UniverseError> {
        self.live_state_mut()?
            .assign_glue_parameter(parameter, value, scope)?;
        Ok(())
    }

    /// Assigns one math-glue register through the exact save journal.
    pub fn assign_mu_glue_register(
        &mut self,
        index: u16,
        value: Option<GlueId<G>>,
        scope: AssignmentScope,
    ) -> Result<(), UniverseError> {
        self.live_state_mut()?
            .assign_mu_glue_register(index, value, scope)?;
        Ok(())
    }

    /// Assigns the current text font through the exact save journal.
    pub fn assign_current_font(
        &mut self,
        value: crate::ids::FontId,
        scope: AssignmentScope,
    ) -> Result<(), UniverseError> {
        if !self.fonts.contains(value) {
            return Err(UniverseError::State(StateError::ForeignSession));
        }
        self.live_state_mut()?.assign_current_font(value, scope)?;
        Ok(())
    }

    /// Assigns one math-family font through the exact save journal.
    pub fn assign_math_family_font(
        &mut self,
        size: crate::math::MathFontSize,
        family: u8,
        value: crate::ids::FontId,
        scope: AssignmentScope,
    ) -> Result<(), UniverseError> {
        if !self.fonts.contains(value) {
            return Err(UniverseError::State(StateError::ForeignSession));
        }
        let index = u8::try_from(size.index())
            .expect("math font size is bounded")
            .saturating_mul(16)
            .saturating_add(family);
        self.live_state_mut()?
            .assign_math_family_font(index, value, scope)?;
        Ok(())
    }

    pub fn allocate_definition(
        &mut self,
        parameter_text: &[TokenWord],
        replacement_text: &[TokenWord],
    ) -> Result<DefinitionId<G>, UniverseError> {
        let id = self
            .core
            .as_mut()
            .ok_or(UniverseError::Retired)?
            .admit_mut()?
            .allocate_definition(parameter_text, replacement_text)?;
        self.engine_usage.observe_transient_memory(
            0,
            parameter_text
                .len()
                .saturating_add(replacement_text.len())
                .saturating_add(2),
        );
        Ok(id)
    }

    /// Promotes exact-size semantic token streams directly into one durable
    /// definition after reserving every destination word and row.
    pub fn promote_definition_from_words<Parameters, Replacement>(
        &mut self,
        parameter_text: Parameters,
        replacement_text: Replacement,
    ) -> Result<DefinitionId<G>, PromotionError>
    where
        Parameters: Clone + ExactSizeIterator<Item = TokenWord>,
        Replacement: Clone + ExactSizeIterator<Item = TokenWord>,
    {
        let parameter_words = parameter_text.len();
        let replacement_words = replacement_text.len();
        let id = self
            .core
            .as_mut()
            .ok_or(PromotionError::Retired)?
            .admit_mut()
            .map_err(|error| match error {
                StateError::GenerationInUse => PromotionError::GenerationInUse,
                _ => PromotionError::AllocationFailed,
            })?
            .allocate_definition_from_iter(parameter_text, replacement_text)
            .map_err(PromotionError::from)?;
        self.engine_usage.observe_transient_memory(
            0,
            parameter_words
                .saturating_add(replacement_words)
                .saturating_add(2),
        );
        Ok(id)
    }

    pub(crate) fn glue_value(&self, id: GlueId<G>) -> GlueSpec {
        self.core.as_ref().expect("live universe").admit().glue(id)
    }

    #[cfg(test)]
    pub(crate) fn provenance_record(&self, id: ProvenanceId<G>) -> OriginRecord {
        self.core
            .as_ref()
            .expect("live universe")
            .admit()
            .provenance(id)
    }

    pub(crate) fn admitted(&self) -> Result<crate::stores::AdmittedState<'_, G>, UniverseError> {
        Ok(self.core.as_ref().ok_or(UniverseError::Retired)?.admit())
    }

    pub fn allocate_token_list(
        &mut self,
        words: &[TokenWord],
    ) -> Result<TokenListId<G>, UniverseError> {
        let id = self
            .core
            .as_mut()
            .ok_or(UniverseError::Retired)?
            .admit_mut()?
            .allocate_token_list(words)?;
        self.engine_usage
            .observe_transient_memory(0, words.len().saturating_add(1));
        Ok(id)
    }

    pub fn allocate_glue(&mut self, value: GlueSpec) -> Result<GlueId<G>, UniverseError> {
        let id = self
            .core
            .as_mut()
            .ok_or(UniverseError::Retired)?
            .admit_mut()?
            .allocate_glue(value)?;
        self.engine_usage.observe_transient_memory(4, 0);
        Ok(id)
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

    pub(crate) fn promote_format_values(
        &mut self,
        definitions: Vec<crate::format::schema::FormatDefinition>,
        live_definitions: Vec<bool>,
        token_lists: Vec<Vec<u32>>,
        glue_values: Vec<GlueSpec>,
    ) -> Result<FormatPromotionReceipt<G>, PromotionError> {
        self.core
            .as_mut()
            .ok_or(PromotionError::Retired)?
            .admit_mut()
            .map_err(|error| match error {
                StateError::GenerationInUse => PromotionError::GenerationInUse,
                _ => PromotionError::AllocationFailed,
            })?
            .promote_format_values(definitions, live_definitions, token_lists, glue_values)
    }

    /// Resolves a runtime alias or materializes one cold loaded-format closure.
    fn resolve_box_root_to_page(
        &mut self,
        root: DurableListId<G>,
    ) -> Result<PageListId, NodeArenaError> {
        let page_root = root.rebrand();
        self.page_nodes
            .contains(page_root)
            .then_some(page_root)
            .ok_or(NodeArenaError::InvalidList)
    }

    /// Publishes a page-lifetime node list by moving the caller's buffer.
    pub fn publish_page_nodes_owned(&mut self, nodes: Vec<Node>) -> PageListId {
        let words = tex_memory_words(&nodes, self.engine_usage.uses_etex_node_sizes());
        for node in &nodes {
            node.visit_fonts(|font| {
                assert!(
                    self.fonts.contains(font),
                    "published page node retains a live Universe font"
                );
            });
        }
        let list = self
            .page_nodes
            .publish_owned(nodes)
            .expect("page construction contains only live page-arena children");
        self.engine_usage.observe_transient_memory(words.0, words.1);
        list
    }

    #[cfg(test)]
    pub(crate) fn publish_page_nodes(&mut self, nodes: &[Node]) -> PageListId {
        self.publish_page_nodes_owned(nodes.to_vec())
    }

    /// Resolves one list through this episode's page arena.
    pub fn page_node_list(
        &self,
        id: PageListId,
    ) -> Result<crate::node_arena::NodeCursor<'_>, NodeArenaError> {
        self.page_nodes
            .node_cursor(id)
            .map_err(|_| NodeArenaError::InvalidList)
    }

    /// Opens one final shipout-scratch row for direct construction.
    pub fn begin_shipout_scratch_list(&mut self) -> ShipoutScratchListId {
        self.shipout_scratch.begin_list()
    }

    /// Appends directly into a final shipout-scratch row.
    pub fn push_shipout_scratch_node(
        &mut self,
        list: ShipoutScratchListId,
        node: ShipoutScratchNode<G>,
    ) {
        self.shipout_scratch.push(list, node);
    }

    /// Resolves one live shipout-only scratch row.
    pub fn shipout_scratch_nodes(
        &self,
        id: ShipoutScratchListId,
    ) -> Option<&[ShipoutScratchNode<G>]> {
        self.shipout_scratch.get(id)
    }

    #[cfg(test)]
    pub(crate) fn shipout_scratch_high_water(&self) -> (usize, Vec<usize>) {
        self.shipout_scratch.high_water()
    }

    #[cfg(test)]
    pub(crate) fn page_node_rows(&self) -> usize {
        self.page_nodes.len()
    }

    /// Captures the page-arena suffix for operation rollback.
    ///
    /// Aggregate operation marks store this cursor by value. Rollback must
    /// restore every canonical mode, alignment, insertion, and page-builder
    /// root before calling [`Self::truncate_page_nodes`].
    #[must_use]
    pub fn page_node_cursor(&self) -> OperationMark<PageMaterialLane> {
        self.page_nodes.operation_mark()
    }

    /// Opens one nested page-storage suffix owned by a structural box or
    /// shipout operation.
    #[must_use]
    pub fn begin_page_node_region(&self) -> OperationMark<PageMaterialLane> {
        self.page_nodes.operation_mark()
    }

    /// Consumes and releases a complete page-storage suffix after every
    /// survivor has crossed into durable storage or detached output.
    pub fn release_page_node_region(
        &mut self,
        region: OperationMark<PageMaterialLane>,
    ) -> Result<(), NodeArenaError> {
        self.page_nodes
            .restore_operation(region)
            .map_err(|_| NodeArenaError::ForeignCursor)
    }

    /// Transfers a failed nested suffix back to the enclosing page owner when
    /// an outer rollback can restore roots into it.
    pub fn retain_page_node_region(
        &self,
        _region: OperationMark<PageMaterialLane>,
    ) -> Result<(), NodeArenaError> {
        Ok(())
    }

    /// Truncates a rejected page-arena suffix after canonical roots restore.
    ///
    /// This ordering is part of the command-attempt integration contract:
    /// root restoration comes first, suffix truncation second.
    pub fn truncate_page_nodes(
        &mut self,
        cursor: OperationMark<PageMaterialLane>,
    ) -> Result<(), NodeArenaError> {
        self.page_nodes
            .restore_operation(cursor)
            .map_err(|_| NodeArenaError::ForeignCursor)
    }

    /// Releases storage reachable only from a completed page after its
    /// handle-free output has been validated and the canonical root removed.
    pub fn release_completed_page(&mut self, _root: PageListId) -> Result<(), NodeArenaError> {
        Ok(())
    }

    pub fn assign_meaning(
        &mut self,
        symbol: SymbolId,
        value: MeaningWord<G>,
        scope: AssignmentScope,
    ) -> Result<(), UniverseError> {
        self.interner().resolve_id(symbol)?;
        if value.font().is_some_and(|font| !self.fonts.contains(font)) {
            return Err(UniverseError::State(StateError::ForeignSession));
        }
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

    pub fn assign_int_param(
        &mut self,
        parameter: crate::env::banks::IntParam,
        value: i32,
        scope: AssignmentScope,
    ) -> Result<(), UniverseError> {
        self.live_state_mut()?
            .assign_integer_parameter(parameter, value, scope)?;
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

    /// Rebrands a page-lifetime box coordinate and assigns it atomically at
    /// the TeX state boundary without moving its graph.
    pub fn assign_page_box(
        &mut self,
        index: u16,
        value: Option<PageListId>,
        scope: AssignmentScope,
    ) -> Result<(), NodePromotionError> {
        let durable = value.map(|root| root.rebrand());
        if durable.is_some() {
            #[cfg(feature = "profiling")]
            crate::measurement::record_node_coordinate_transfer();
        }
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
        let durable = value.rebrand();
        #[cfg(feature = "profiling")]
        crate::measurement::record_node_coordinate_transfer();
        self.live_state_mut()
            .expect("live generation is admitted")
            .replace_box_register(index, Some(durable))
            .expect("box register index is admitted")
    }

    /// Obtains TeX's logical box copy as a page coordinate.
    ///
    /// Ordinary runtime boxes already live in the page arena and only rebrand
    /// their coordinate. A box loaded from a detached format is materialized
    /// once into the destination arena on first use.
    pub fn copy_box_to_page(&mut self, index: u16) -> Option<PageListId> {
        self.box_register(index)
            .expect("box register index is admitted")
            .map(|root| {
                self.resolve_box_root_to_page(root)
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
    ) -> Result<crate::node_arena::NodeCursor<'_>, UniverseError> {
        self.page_nodes
            .node_cursor(id.rebrand())
            .map_err(|_| UniverseError::NodeArena(NodeArenaError::InvalidList))
    }

    /// Resolves a child coordinate borrowed from a durable root.
    pub fn durable_child_node_list(
        &self,
        id: PageListId,
    ) -> Result<crate::node_arena::NodeCursor<'_>, UniverseError> {
        self.page_nodes
            .node_cursor(id)
            .map_err(|_| UniverseError::NodeArena(NodeArenaError::InvalidList))
    }

    /// Resolves a generation-owned token payload for borrow-only shipout
    /// lowering.
    pub fn with_durable_token_list<R>(
        &self,
        id: TokenListId<G>,
        read: impl FnOnce(crate::TokenListView<G>) -> R,
    ) -> Result<R, UniverseError> {
        let admitted = self.core.as_ref().ok_or(UniverseError::Retired)?.admit();
        Ok(read(admitted.token_list(id)))
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

    pub fn journal_cursor(&mut self) -> Result<JournalCursor<G>, UniverseError> {
        Ok(self.live_state_mut()?.journal_cursor())
    }

    pub fn begin_state_operation(&mut self) -> Result<StateOperation<G>, UniverseError> {
        Ok(self.live_state_mut()?.begin_state_operation())
    }

    pub fn commit_state_operation(
        &mut self,
        operation: StateOperation<G>,
    ) -> Result<(), UniverseError> {
        self.live_state_mut()?.commit_state_operation(operation);
        Ok(())
    }

    /// Current bytes retained by split group, checkpoint, and operation undo.
    ///
    /// This detached scalar exposes no journal entries or restoration cursor;
    /// outer execution hosts use it solely to enforce configured admission
    /// budgets.
    pub fn state_journal_bytes(&self) -> Result<usize, UniverseError> {
        Ok(self
            .core
            .as_ref()
            .ok_or(UniverseError::Retired)?
            .admit()
            .state()
            .journal_retained_bytes())
    }

    pub fn restore_state(&mut self, operation: StateOperation<G>) -> Result<(), UniverseError> {
        self.live_state_mut()?.rollback_state_operation(operation)?;
        Ok(())
    }

    /// Retains one coarse generation beside bounded state and arena cursors.
    ///
    /// Command, mode, source, effect, and output owners compose their own
    /// bounded cursor fields around this state-layer foundation. A checkpoint
    /// with no page-handle carrier records only the generation's conservative
    /// retained page bound; incidental rootless allocation is not history.
    pub fn state_checkpoint(&mut self) -> Result<StateCheckpoint<G>, UniverseError> {
        let boundary = self
            .page_nodes
            .seal_boundary()
            .map_err(|_| UniverseError::State(StateError::InvalidCursor))?;
        let page = self
            .page_nodes
            .checkpoint_mark(boundary)
            .map_err(|_| UniverseError::State(StateError::InvalidCursor))?;
        self.state_checkpoint_at(page)
    }

    fn state_checkpoint_at(
        &mut self,
        page: CheckpointMark<PageMaterialLane>,
    ) -> Result<StateCheckpoint<G>, UniverseError> {
        let core = self.core.as_mut().ok_or(UniverseError::Retired)?;
        let owner = core.generation_owner();
        let journal = core.state_mut().journal_cursor();
        let dense = core.state().checkpoint_cursor();
        Ok(GenerationCheckpoint::new(
            owner,
            BoundedStateMark::new(journal, core.durable_node_cursor(), page, dense),
        ))
    }

    /// Captures the complete state-facing portion of an executor checkpoint.
    /// Runtime ids remain reachable only through the single retained state
    /// generation and opaque subsystem roots.
    pub fn runtime_checkpoint(&mut self) -> Result<RuntimeCheckpoint<G>, UniverseError> {
        self.runtime_checkpoint_with_page_roots(false)
    }

    /// Selects maintained mode/page convergence identity before page material
    /// is published for an incremental session.
    #[doc(hidden)]
    pub fn enable_reachable_state_identity(&mut self) {
        if let Some(core) = &mut self.core {
            let _ = core.enable_reachable_state_identity();
        }
        let _ = self.world.enable_reachable_state_identity();
        let _ = self.hyphenation.enable_reachable_state_identity();
        let _ = self.dependencies.enable_reachable_state_identity();
        let _ = self.sources.enable_reachable_state_identity();
        let _ = self.fonts.enable_reachable_state_identity();
        self.page.enable_reachable_state_identity();
        self.page_nodes.enable_semantic_identity();
    }

    /// Captures runtime roots while incorporating executor-owned page
    /// carriers into the generation's monotonic retained prefix.
    #[doc(hidden)]
    pub fn runtime_checkpoint_with_page_roots(
        &mut self,
        external_page_roots: bool,
    ) -> Result<RuntimeCheckpoint<G>, UniverseError> {
        self.runtime_checkpoint_with_page_roots_and_identity(external_page_roots, false)
    }

    /// Captures runtime roots and, only when explicitly requested, asks each
    /// authoritative component owner for its maintained semantic root.
    #[doc(hidden)]
    pub fn runtime_checkpoint_with_page_roots_and_identity(
        &mut self,
        external_page_roots: bool,
        wants_reachable_state_identity: bool,
    ) -> Result<RuntimeCheckpoint<G>, UniverseError> {
        #[cfg(feature = "profiling")]
        self.live_state_mut()?.record_journal_checkpoint();
        if !(external_page_roots || self.page.retains_page_node_handles()) {
            self.release_unretained_page_suffix()?;
        }
        let live_state = self.state_checkpoint()?;
        let live_mark = *live_state.mark();
        let font_mark = self.fonts.watermark();
        let live_core = self.core.as_ref().ok_or(UniverseError::Retired)?;
        let core_retained_bytes = live_core
            .checkpoint_retained_bytes()
            .saturating_add(
                self.primitive_registry
                    .names
                    .len()
                    .saturating_mul(std::mem::size_of::<String>()),
            )
            .saturating_add(
                self.primitive_registry
                    .meanings
                    .len()
                    .saturating_mul(std::mem::size_of::<MeaningWord<G>>()),
            );
        let mark = BoundedStateMark::new(
            *live_mark.journal(),
            *live_mark.durable(),
            *live_mark.page(),
            *live_mark.input(),
        );
        let owner = live_core.generation_owner();
        let core_owner = owner.checkpoint_owner_id();
        let generation = live_core.generation_cursor();
        let source_mark = self.sources.watermark();
        let retention = RuntimeCheckpointRetention {
            core_owner,
            page_owner: crate::CheckpointOwnerId::from_owner(&self.page),
            world_owner: crate::CheckpointOwnerId::from_owner(&self.world),
            hyphenation_owner: crate::CheckpointOwnerId::from_owner(&self.hyphenation),
            pdf_owner: crate::CheckpointOwnerId::from_owner(&self.pdf),
            dependency_owner: crate::CheckpointOwnerId::from_owner(&self.dependencies),
            source_font_owner: crate::CheckpointOwnerId::from_owner(&self.sources),
            core: core_retained_bytes,
            page: self.page.retained_bytes(),
            world: self.world.checkpoint_retained_bytes(),
            hyphenation: self.hyphenation.checkpoint_retained_bytes(),
            pdf: self.pdf.checkpoint_retained_bytes(),
            dependency: self.dependencies.checkpoint_retained_bytes(),
            source_font: source_mark
                .checkpoint_retained_bytes()
                .saturating_add(self.fonts.checkpoint_retained_bytes(font_mark)),
        };
        let pdf = self.pdf.snapshot();
        let page = self.page.checkpoint_mark();
        let core_identity = live_core.reachable_state_identity_root().map(|core| {
            crate::state_hash::semantic_scalar_root(0x636f_7265_5f61_6767, |hasher| {
                hasher.u64(core);
                hasher.u8(self.interaction_mode as u8);
                match self.prepared_mag {
                    Some(mag) => {
                        hasher.bool(true);
                        hasher.i32(mag);
                    }
                    None => hasher.bool(false),
                }
            })
        });
        let identity_roots = RuntimeCheckpointIdentityRoots {
            page: wants_reachable_state_identity
                .then(|| page.reachable_state_identity_root())
                .flatten(),
            pdf: wants_reachable_state_identity.then(|| pdf.reachable_state_identity_root()),
            world: wants_reachable_state_identity
                .then(|| self.world.reachable_state_identity_root())
                .flatten(),
            hyphenation: wants_reachable_state_identity
                .then(|| self.hyphenation.reachable_state_identity_root())
                .flatten(),
            dependency: wants_reachable_state_identity
                .then(|| self.dependencies.reachable_state_identity_root())
                .flatten(),
            source: wants_reachable_state_identity
                .then(|| self.sources.reachable_state_identity_root())
                .flatten(),
            font: wants_reachable_state_identity
                .then(|| self.fonts.reachable_state_identity_root())
                .flatten(),
            core: wants_reachable_state_identity
                .then_some(core_identity)
                .flatten(),
        };
        let checkpoint = RuntimeCheckpoint {
            state: GenerationCheckpoint::new(owner, mark),
            generation,
            page,
            pdf,
            world: self.world.snapshot(),
            fonts: font_mark,
            sources: source_mark,
            hyphenation: self.hyphenation.checkpoint(),
            dependencies: self.dependencies.snapshot_tracker(),
            interaction_mode: self.interaction_mode,
            prepared_mag: self.prepared_mag,
            engine_usage: self.engine_usage.clone(),
            // Component owners publish roots here. No aggregate fallback is
            // allowed: missing hooks remain explicit until their mutation
            // journals maintain a complete canonical semantic root.
            identity_roots,
            retention,
        };
        Ok(checkpoint)
    }

    /// Releases only rootless page rows above the generation's monotonic
    /// retained checkpoint prefix.
    pub fn release_unretained_page_suffix(&mut self) -> Result<(), UniverseError> {
        // Root liveness is not yet tracked at chunk-envelope granularity.
        // Retain conservatively until generation retirement.
        Ok(())
    }

    /// Applies the retained-prefix release only when every checkpointable
    /// state carrier is rootless at the current outer boundary.
    #[doc(hidden)]
    pub fn release_page_suffix_if_rootless(
        &mut self,
        external_page_roots: bool,
    ) -> Result<bool, UniverseError> {
        if external_page_roots || self.page.retains_page_node_handles() {
            return Ok(false);
        }
        self.release_unretained_page_suffix()?;
        Ok(true)
    }

    /// Validates and restores a complete runtime checkpoint while allowing
    /// the executor to transfer command and mode roots before any state,
    /// source, or font arena suffix is truncated.
    pub fn restore_runtime_checkpoint_with_roots(
        &mut self,
        checkpoint: &RuntimeCheckpoint<G>,
        transfer_external_roots: impl FnOnce(),
    ) -> Result<(), UniverseError> {
        self.restore_runtime_checkpoint_with_roots_mode(checkpoint, transfer_external_roots, false)
    }

    fn restore_runtime_checkpoint_with_roots_mode(
        &mut self,
        checkpoint: &RuntimeCheckpoint<G>,
        transfer_external_roots: impl FnOnce(),
        generation_fork: bool,
    ) -> Result<(), UniverseError> {
        if !(if generation_fork {
            self.world.snapshot_is_forkable(&checkpoint.world)
        } else {
            self.world.snapshot_is_retained(&checkpoint.world)
        }) || !self.pdf.snapshot_is_retained(&checkpoint.pdf)
            || !self.fonts.validates(checkpoint.fonts)
            || !self.sources.validates(checkpoint.sources)
            || !self.checkpoint_state_is_ready(checkpoint)
            || !self.page.validates_checkpoint_mark(checkpoint.page)
        {
            return Err(UniverseError::State(StateError::InvalidCursor));
        }
        self.activate_checkpoint_state(checkpoint)?;
        self.page.restore_checkpoint_mark(checkpoint.page);
        if !generation_fork {
            self.pdf.rollback(checkpoint.pdf.clone());
        }
        if !generation_fork {
            self.world.rollback(&checkpoint.world);
        }
        if !generation_fork {
            self.hyphenation.restore_checkpoint(&checkpoint.hyphenation);
        }
        self.dependencies.restore_tracker(&checkpoint.dependencies);
        self.interaction_mode = checkpoint.interaction_mode;
        self.prepared_mag = checkpoint.prepared_mag;
        self.engine_usage = checkpoint.engine_usage.clone();
        transfer_external_roots();
        self.page_nodes
            .restore_checkpoint(*checkpoint.state.mark().page())
            .expect("runtime restore prevalidated the page-material checkpoint");
        self.fonts.truncate_to(checkpoint.fonts);
        self.sources.truncate_to(checkpoint.sources);
        Ok(())
    }

    /// Begins one exclusive, rollback-capable artifact publication.
    #[must_use]
    pub fn begin_shipout(&mut self) -> ShipoutTransaction<'_, G> {
        let rollback = ShipoutRollback {
            state: self
                .begin_state_operation()
                .expect("live shipout generation has an operation journal"),
            page_nodes: self.page_nodes.operation_mark(),
            page: self.page.checkpoint_mark(),
            pdf: self.pdf.snapshot(),
            world: self.world.snapshot(),
            prepared_mag: self.prepared_mag,
            engine_usage: self.engine_usage.clone(),
        };
        let empty_tokens = self
            .allocate_token_list(&[])
            .expect("shipout can allocate its canonical empty token root");
        ShipoutTransaction {
            scratch: Some(self.shipout_scratch.mark()),
            universe: self,
            rollback: Some(rollback),
            empty_tokens,
        }
    }

    /// Publishes the already-committed effect prefix after command admission
    /// has been released. This remains an aggregate outer barrier because a
    /// partial host publication cannot be rolled back as a state mutation.
    pub fn publish_effect_prefix(
        &mut self,
        effect_pos: crate::EffectPos,
    ) -> Result<(), crate::WorldError> {
        if self.world.commit_mode() == crate::WorldCommitMode::Retained {
            return Ok(());
        }
        self.world.commit_effects(effect_pos)
    }

    /// Publishes already-verified replay bytes through the ordinary shipout
    /// barrier. Memo replay is disabled whenever rendered-source provenance is
    /// demanded, so the legacy compact provenance input is intentionally not
    /// attached to the detached artifact.
    #[doc(hidden)]
    pub fn commit_replayed_artifact(
        &mut self,
        bytes: Vec<u8>,
        _render_origin_ends: Vec<u32>,
        _render_provenance: crate::OutputProvenanceRecipe,
        receipt: Option<crate::PageOutputPublicationReceiptId>,
    ) -> Result<
        (
            crate::ContentHash,
            crate::PageOutputPublicationReceipt,
            crate::ArtifactPublicationRecord,
        ),
        crate::WorldError,
    > {
        let effect_pos = self.world.effect_pos();
        let effect_index = self.world.effect_records().len();
        let reservation = self
            .world
            .reserve_active_artifact_publication_at(effect_index, receipt);
        let transaction = self.begin_shipout();
        let (hash, publication) =
            transaction.commit(crate::VerifiedArtifact::new(bytes), effect_pos, reservation)?;
        let effect_publication = self.world.reserve_effect_publication();
        self.world
            .link_artifact_effect_publication(publication.publication(), effect_publication);
        let publication = publication.with_effect_publication(effect_publication);
        Ok((
            hash,
            crate::PageOutputPublicationReceipt::committed(effect_publication, publication),
            publication,
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

    pub fn end_group(
        &mut self,
        kind: GroupKind,
    ) -> Result<crate::GroupRestorationReceipt<G>, UniverseError> {
        Ok(self.live_state_mut()?.end_group(kind)?)
    }

    /// Admits matching coarse owners once for a command episode.
    pub fn command_context(&mut self) -> Result<CommandContext<'_, G>, UniverseError> {
        let core = self.core.as_mut().ok_or(UniverseError::Retired)?;
        Ok(CommandContext::new(CommandContextParts {
            interner: self
                .interner
                .as_mut()
                .expect("live Universe has an admitted session epoch"),
            admitted: core.admit_mut()?,
            primitive_names: &self.primitive_registry.names,
            primitive_meanings: &self.primitive_registry.meanings,
            world: &mut self.world,
            dependencies: &mut self.dependencies,
            fonts: &mut self.fonts,
            page_nodes: &mut self.page_nodes,
            shipout_scratch: &mut self.shipout_scratch,
            page: &mut self.page,
            pdf: &mut self.pdf,
            sources: &mut self.sources,
            hyphenation: &mut self.hyphenation,
            interaction_mode: &mut self.interaction_mode,
            prepared_mag: &mut self.prepared_mag,
            error_context_widths: self.error_context_widths,
            engine_usage: &mut self.engine_usage,
            dynamic_memory_scratch: &mut self.dynamic_memory_scratch,
        }))
    }

    /// Selects every capacity owned by the executable process profile.
    ///
    /// The format boundary identifies its producer profile through the
    /// retained string-pool coordinates. Executable framing may retain that
    /// profile or expand it to the pdfTeX process before the first command.
    pub fn set_engine_capacity_profile(&mut self, profile: crate::EngineCapacityProfile) {
        self.engine_usage.select_capacity_profile(profile);
        self.hyphenation
            .set_trie_capacity(profile.configuration().trie_nodes);
    }

    /// Selects the executable-process `hyph_size` bound.
    ///
    /// TeX82 §934 uses this value for the exception table, §1308 retains it
    /// as a format compatibility constant, and §1334 reports the selected
    /// value. Web2C `tex.ch` [51.1332] makes the bound process-configurable.
    pub fn set_hyphenation_exception_capacity(&mut self, capacity: usize) {
        self.hyphenation.set_exception_capacity(capacity);
    }

    /// Retains the complete immutable generation across an in-process
    /// resource suspension. No individual runtime value acquires an owner.
    pub fn generation_owner(&self) -> Result<GenerationOwner<G>, UniverseError> {
        self.command_generation_owner
            .clone()
            .ok_or(UniverseError::Retired)
    }

    /// Validates a returned continuation owner before its attempt is resumed.
    #[must_use]
    pub fn owns_generation(&self, owner: &GenerationOwner<G>) -> bool {
        self.core.is_some()
            && self
                .command_generation_owner
                .as_ref()
                .is_some_and(|current| current.same_generation(owner))
    }

    fn live_state_mut(&mut self) -> Result<&mut crate::env::DenseState<G>, UniverseError> {
        Ok(self
            .core
            .as_mut()
            .ok_or(UniverseError::Retired)?
            .state_mut())
    }

    pub(crate) fn live_state(&self) -> Result<&crate::env::DenseState<G>, StateError> {
        self.core
            .as_ref()
            .ok_or(StateError::InvalidCursor)
            .map(StateCore::state)
    }

    /// Retires the session epoch and revision generation together. The
    /// aggregate remains only as a typed retired shell which rejects reuse.
    pub fn retire(&mut self) -> Result<UniverseRetirement, UniverseError> {
        if !self
            .interner
            .as_ref()
            .expect("live Universe has an admitted session epoch")
            .is_last_owner()
            || self.core.as_ref().is_some_and(|core| {
                !core.can_retire_after_dropping(
                    self.command_generation_owner
                        .as_ref()
                        .expect("live generation has command authority"),
                )
            })
        {
            return Err(UniverseError::State(StateError::GenerationInUse));
        }
        let interner = self.interner_mut().retire()?;
        Ok(UniverseRetirement {
            interner,
            state: self.retire_generation()?,
        })
    }

    pub(crate) fn retire_generation(&mut self) -> Result<StateCoreRetirement, UniverseError> {
        let command_owner = self
            .command_generation_owner
            .as_ref()
            .ok_or(UniverseError::Retired)?;
        if self
            .core
            .as_ref()
            .is_some_and(|core| !core.can_retire_after_dropping(command_owner))
        {
            return Err(UniverseError::State(StateError::GenerationInUse));
        }
        drop(
            self.command_generation_owner
                .take()
                .expect("live generation has one command authority"),
        );
        self.core.take().map_or_else(
            || Ok(StateCoreRetirement::transferred()),
            |core| core.retire().map_err(UniverseError::State),
        )
    }

    #[must_use]
    pub const fn is_retired(&self) -> bool {
        self.core.is_none()
    }
}

impl<G> Drop for Universe<G> {
    fn drop(&mut self) {
        debug_assert!(
            self.checkpoint_candidate.is_none(),
            "reachability-store cleanup settles a candidate before dropping its Universe"
        );
    }
}

impl<G> ShipoutTransaction<'_, G> {
    fn commit_state_operation(&mut self) {
        let rollback = self
            .rollback
            .take()
            .expect("shipout owns its rollback mark");
        self.universe
            .commit_state_operation(rollback.state)
            .expect("shipout owns the active state operation");
        self.universe.page.commit_transaction(rollback.page);
    }

    /// Atomically commits the staged artifact, effect prefix, and fixed PDF
    /// page record. Dropping before this point restores aggregate roots before
    /// it truncates the state/page suffixes they address.
    pub fn commit(
        mut self,
        artifact: crate::VerifiedArtifact,
        effect_pos: crate::EffectPos,
        reservation: crate::ArtifactPublicationReservation,
    ) -> Result<(crate::ContentHash, crate::ArtifactPublicationRecord), crate::WorldError> {
        let output_parameters = self.current_pdf_output_parameters();
        let page_parameters = self.current_pdf_page_parameters(self.empty_tokens.clone());
        let pk_mode = self.pdf_token_parameter(
            self.token_parameter(crate::env::banks::TokParam::PDF_PK_MODE)
                .expect("PDF token parameter is admitted")
                .unwrap_or_else(|| self.empty_tokens.clone()),
        );
        self.pdf
            .ensure_page_capacity(output_parameters)
            .map_err(|()| crate::WorldError::pdf_object_ids_exhausted())?;
        let hash = self.world.store_verified_artifact(&artifact)?;
        if self.world.commit_mode() != crate::WorldCommitMode::Retained
            && let Err(error) = self.world.commit_effects(effect_pos)
        {
            // A partially materialized effect prefix is irreversible. Preserve
            // that canonical state and prevent Drop from pretending rollback
            // remained possible.
            self.commit_state_operation();
            return Err(error);
        }
        self.page
            .set_integer(crate::page::PageInteger::DeadCycles, 0);
        self.pdf
            .commit_page(hash, output_parameters, page_parameters, pk_mode);
        let record = reservation.record();
        let (bytes, render_provenance, open_out_occurrences) = artifact.into_parts();
        self.world.record_artifact_commit(
            hash,
            bytes,
            render_provenance,
            open_out_occurrences,
            reservation,
        );
        self.world.finish_page_effect_interval();
        self.commit_state_operation();
        Ok((hash, record))
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
        if !core.state().validate_checkpoint_cursor(*mark.input()) {
            return Err(UniverseError::State(StateError::InvalidCursor));
        }
        core.validate_durable_node_cursor(*mark.durable())?;
        if !self.page_nodes.can_restore_checkpoint(*mark.page()) {
            return Err(UniverseError::State(StateError::InvalidCursor));
        }
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
        let core = self
            .core
            .as_mut()
            .expect("restore plan validated a live state core");
        core.state_mut().restore_checkpoint_cursor(*mark.input());
        core.truncate_durable_nodes(*mark.durable())
            .expect("restore plan prevalidated durable-node suffix");
        self.page_nodes
            .restore_checkpoint(*mark.page())
            .expect("restore plan prevalidated page-material settlement");
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
    let epoch = SessionInternerEpoch::new(budget);
    let interner = epoch.lease().map_err(|_| StateError::ForeignSession)?;
    drop(epoch);
    with_generation(|generation| {
        let core = StateCore::new(generation)?;
        let mut universe = Universe::new(interner, core);
        Ok(use_universe(&mut universe))
    })
}

/// Introduces one fresh revision generation under an existing session epoch.
pub fn with_universe_in_epoch<R>(
    epoch: &SessionInternerEpoch,
    use_universe: impl for<'id> FnOnce(&mut Universe<GenerationBrand<'id>>) -> R,
) -> Result<R, SessionEpochError> {
    let interner = epoch.lease()?;
    with_generation(|generation| {
        let core = StateCore::new(generation).map_err(|_| SessionEpochError::Retired)?;
        let mut universe = Universe::new(interner, core);
        Ok(use_universe(&mut universe))
    })
}
