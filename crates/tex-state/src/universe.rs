//! Session epoch plus one admitted revision-generation state core.

use crate::checkpoint::{BoundedStateMark, GenerationCheckpoint, RestoreTarget, prepare_restore};
use crate::command_context::{CommandContext, CommandContextParts};
use crate::definition_arena::{DefinitionAllocationError, DefinitionId};
use crate::dependency::{
    DependencyRegionError, DependencyRegionToken, DependencyRuntime, ObservedDependency,
    TrackedRegionBarrier,
};
use crate::durable_arena::{DurableAllocationError, GlueId, ProvenanceId, TokenListId};
use crate::env::group::{GroupFrame, GroupKind};
use crate::env::{AssignmentScope, CodeTableKind, StateError};
use crate::font::{FontStore, FontStoreMark};
use crate::generation::{GenerationBrand, GenerationOwner, with_generation};
use crate::glue::GlueSpec;
use crate::hyphenation::HyphenationTable;
use crate::interner::{
    ControlSequenceKind, Interner, InternerAccessError, InternerBudget, InternerError,
    InternerRetirement, InternerUsage, Symbol, SymbolId,
};
use crate::journal::JournalCursor;
use crate::meaning::{Meaning, MeaningWord};
use crate::node::Node;
use crate::node_arena::{
    DurableListId, NodeArenaCursor, NodeArenaError, NodeArenaRegion, NodeList, PageLifetime,
    PageListId, PageNodeArena,
};
use crate::page::PageBuilderState;
use crate::pdf::PdfState;
use crate::print::ErrorContextWidths;
use crate::provenance::OriginRecord;
use crate::scaled::Scaled;
use crate::session_epoch::{InternerLease, SessionEpochError, SessionInternerEpoch};
use crate::shipout_scratch::{
    ShipoutScratchArena, ShipoutScratchListId, ShipoutScratchMark, ShipoutScratchNode,
};
use crate::source_map::{SourceMap, SourceMapMark};
use crate::stores::{StateCore, StateCoreRetirement};
use crate::token::TokenWord;
use crate::world::World;

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
        self.nodes(nodes.nodes());
    }

    fn node(&mut self, node: &Node) {
        node.visit_semantic_node_lists(|child| {
            self.hasher.tag(0xf0);
            let child = self
                .universe
                .page_node_list(*child)
                .expect("semantic child belongs to the live page arena");
            self.nodes(child.nodes());
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
    state: StateCheckpoint<G>,
    page: PageBuilderState,
    pdf: crate::pdf::PdfStateSnapshot<G>,
    world: crate::world::WorldSnapshot,
    prepared_mag: Option<i32>,
    engine_usage: crate::command_context::EngineUsageRuntime,
}

/// Coarse generation owner plus every runtime root needed by an aggregate
/// executor checkpoint.
///
/// The value is opaque outside `tex-state`: consumers can retain and clone it
/// but cannot extract arena marks or individual store owners.
pub struct RuntimeCheckpoint<G> {
    state: StateCheckpoint<G>,
    page: PageBuilderState,
    pdf: crate::pdf::PdfStateSnapshot<G>,
    world: crate::world::WorldSnapshot,
    fonts: FontStoreMark,
    sources: SourceMapMark,
    hyphenation: HyphenationTable,
    dependencies: crate::dependency::DependencyTrackerSnapshot,
    interaction_mode: InteractionMode,
    prepared_mag: Option<i32>,
    engine_usage: crate::command_context::EngineUsageRuntime,
}

impl<G> Clone for RuntimeCheckpoint<G> {
    fn clone(&self) -> Self {
        Self {
            state: self.state.clone(),
            page: self.page.clone(),
            pdf: self.pdf.clone(),
            world: self.world.clone(),
            fonts: self.fonts,
            sources: self.sources,
            hyphenation: self.hyphenation.clone(),
            dependencies: self.dependencies.clone(),
            interaction_mode: self.interaction_mode,
            prepared_mag: self.prepared_mag,
            engine_usage: self.engine_usage.clone(),
        }
    }
}

impl<G> RuntimeCheckpoint<G> {
    #[doc(hidden)]
    pub fn pdf_history_position(&self) -> (u64, u64) {
        self.pdf.history_position()
    }
}

impl<G> std::fmt::Debug for RuntimeCheckpoint<G> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("RuntimeCheckpoint(..)")
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
            self.universe.page = rollback.page;
            self.universe.pdf.rollback(rollback.pdf);
            self.universe.world.rollback(&rollback.world);
            self.universe.prepared_mag = rollback.prepared_mag;
            self.universe.engine_usage = rollback.engine_usage;
            self.universe
                .restore_state_checkpoint(&rollback.state)
                .expect("validated shipout rollback remains restorable");
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
    pub(crate) interner: Option<InternerLease>,
    pub(crate) core: Option<StateCore<G>>,
    page_nodes: PageNodeArena,
    retained_page_bound: NodeArenaCursor<PageLifetime>,
    shipout_scratch: ShipoutScratchArena<G>,
    pub(crate) fonts: FontStore,
    pub(crate) page: PageBuilderState,
    pub(crate) pdf: PdfState<G>,
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
    pub(crate) primitive_names: Vec<String>,
    pub(crate) primitive_meanings: Vec<MeaningWord<G>>,
    /// Driver-requested cache policy consumed exactly once by MainControl.
    pure_memo_config: Option<crate::PureMemoConfig>,
    /// Borrow-only capability for the execution-owned cache service.
    pure_memo_capability: std::sync::Weak<std::sync::Mutex<crate::PureMemoRuntime>>,
    restore_owner: Option<GenerationOwner<G>>,
}

impl<G> Universe<G> {
    #[doc(hidden)]
    pub fn prune_pdf_history(&mut self, low_water: (u64, u64)) {
        self.pdf.prune_history(low_water);
    }

    #[doc(hidden)]
    pub fn pdf_history_head(&self) -> (u64, u64) {
        self.pdf.history_head()
    }

    /// Creates a destination-local runtime from one retained checkpoint.
    /// Validation is complete before the returned fork becomes visible.
    #[doc(hidden)]
    pub fn fork_runtime_checkpoint(
        &mut self,
        checkpoint: &RuntimeCheckpoint<G>,
    ) -> Result<Self, UniverseError> {
        let owner = checkpoint.state.owner();
        let mark = checkpoint.state.mark();
        <Self as RestoreTarget<GenerationOwner<G>, StateCheckpointMark<G>>>::validate_restore(
            self, owner, mark,
        )?;
        if !self.world.snapshot_is_forkable(&checkpoint.world)
            || !self.pdf.snapshot_is_retained(&checkpoint.pdf)
            || !self.fonts.validates(checkpoint.fonts)
            || !self.sources.validates(checkpoint.sources)
        {
            return Err(UniverseError::State(StateError::InvalidCursor));
        }

        let core = self.core.as_ref().ok_or(UniverseError::Retired)?.fork();
        let destination_owner = core.generation_owner();
        let mut fork = Self {
            interner: None,
            core: Some(core),
            page_nodes: self.page_nodes.fork(),
            retained_page_bound: self.retained_page_bound,
            shipout_scratch: ShipoutScratchArena::default(),
            fonts: self.fonts.clone(),
            page: self.page.clone(),
            pdf: self.pdf.fork_snapshot(&checkpoint.pdf),
            sources: self.sources.clone(),
            hyphenation: self.hyphenation.clone(),
            world: self.world.clone(),
            dependencies: self.dependencies.clone(),
            interaction_mode: self.interaction_mode,
            prepared_mag: self.prepared_mag,
            error_context_widths: self.error_context_widths,
            engine_usage: self.engine_usage.clone(),
            dynamic_memory_scratch: crate::stores::DynamicMemoryScratch::default(),
            provenance_demand: self.provenance_demand,
            provenance_budgets: self.provenance_budgets,
            primitive_names: self.primitive_names.clone(),
            primitive_meanings: self.primitive_meanings.clone(),
            pure_memo_config: self.pure_memo_config,
            pure_memo_capability: self.pure_memo_capability.clone(),
            restore_owner: None,
        };
        let retargeted = RuntimeCheckpoint {
            state: GenerationCheckpoint::new(destination_owner, *mark),
            page: checkpoint.page.clone(),
            pdf: checkpoint.pdf.clone(),
            world: checkpoint.world.clone(),
            fonts: checkpoint.fonts,
            sources: checkpoint.sources,
            hyphenation: checkpoint.hyphenation.clone(),
            dependencies: checkpoint.dependencies.clone(),
            interaction_mode: checkpoint.interaction_mode,
            prepared_mag: checkpoint.prepared_mag,
            engine_usage: checkpoint.engine_usage.clone(),
        };
        fork.restore_runtime_checkpoint_with_roots_mode(&retargeted, || {}, true)?;
        Ok(fork)
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
        let page_nodes = PageNodeArena::with_memory_accounting(core.memory_accounting());
        let retained_page_bound = page_nodes.cursor();
        Self {
            interner: Some(interner),
            core: Some(core),
            page_nodes,
            retained_page_bound,
            shipout_scratch: ShipoutScratchArena::default(),
            fonts,
            page: PageBuilderState::default(),
            pdf: PdfState::default(),
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
            primitive_names: Vec::new(),
            primitive_meanings: Vec::new(),
            pure_memo_config: None,
            pure_memo_capability: std::sync::Weak::new(),
            restore_owner: None,
        }
    }

    /// Observes one TeX82 §259 lookup outside every group and rollback cursor,
    /// then admits the issued compact slot into the dense meaning bank.
    pub fn intern(&mut self, name: &str) -> Result<SymbolId, UniverseError> {
        if self.core.is_none() {
            return Err(UniverseError::Retired);
        }
        let is_new = self.interner().known(name).is_none();
        let symbol = self.interner_mut().intern_hash(name)?;
        if is_new && name.chars().nth(1).is_some() {
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
            .and_then(|index| match self.primitive_meanings[index].resolve() {
                crate::ResolvedMeaning::Static(meaning) => Some(meaning),
                crate::ResolvedMeaning::Macro { .. } => None,
            })
    }

    #[must_use]
    pub fn primitive_name(&self, meaning: Meaning) -> Option<&str> {
        self.primitive_meanings
            .iter()
            .position(|candidate| {
                matches!(candidate.resolve(), crate::ResolvedMeaning::Static(value) if value == meaning)
            })
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
        self.primitive_meanings
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
    pub const fn fixed_pdf_output_parameters(&self) -> Option<crate::PdfOutputParameters> {
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
        Replacement: ExactSizeIterator<Item = TokenWord>,
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
        definitions: &[crate::format::schema::FormatDefinition],
        live_definitions: &[bool],
        token_lists: &[Vec<u32>],
        glue_values: &[GlueSpec],
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
            source.reserve_promotion_with_scratch(
                roots,
                core.as_mut()
                    .ok_or(PromotionError::Retired)?
                    .admit_mut()
                    .map_err(|error| match error {
                        StateError::GenerationInUse => PromotionError::GenerationInUse,
                        _ => PromotionError::AllocationFailed,
                    })?
                    .nodes_mut(),
                self.dynamic_memory_scratch.page_to_durable(),
            )?;
        }
        let (glue, tokens) = self
            .page_nodes
            .escaping_payloads_with_scratch(roots, self.dynamic_memory_scratch.page_to_durable())?;
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
        let promoted = source.promote_into_with_scratch(
            roots,
            core.as_mut()
                .ok_or(PromotionError::Retired)?
                .admit_mut()
                .map_err(|error| match error {
                    StateError::GenerationInUse => PromotionError::GenerationInUse,
                    _ => PromotionError::AllocationFailed,
                })?
                .nodes_mut(),
            self.dynamic_memory_scratch.page_to_durable(),
            |_| glue_ids.next().expect("one durable id per page glue root"),
            |_| {
                token_ids
                    .next()
                    .expect("one durable id per page token root")
            },
        )?;
        debug_assert!(glue_ids.next().is_none());
        debug_assert!(token_ids.next().is_none());
        Ok(promoted.into_vec())
    }

    /// Copies one durable box closure into page-lifetime storage.
    pub fn copy_durable_page_nodes(
        &mut self,
        root: DurableListId<G>,
    ) -> Result<PageListId, NodeArenaError> {
        let etex_node_sizes = self.engine_usage.uses_etex_node_sizes();
        let (core, page_nodes) = (&self.core, &mut self.page_nodes);
        let admitted = core.as_ref().ok_or(NodeArenaError::InvalidList)?.admit();
        let words = admitted.copied_node_closure_tex_memory_words(
            root,
            etex_node_sizes,
            &mut self.dynamic_memory_scratch,
        )?;
        let current_dynamic = admitted
            .current_dynamic_memory_words(etex_node_sizes)?
            .saturating_add(self.page.dynamic_memory_words(etex_node_sizes));
        let copied =
            admitted.copy_node_into_page(root, page_nodes, &mut self.dynamic_memory_scratch)?;
        self.engine_usage
            .observe_node_copy(words.0, current_dynamic, words.1);
        Ok(copied)
    }

    /// Publishes one complete page-lifetime node list.
    #[must_use]
    pub fn publish_page_nodes(&mut self, nodes: &[Node]) -> PageListId {
        let words = tex_memory_words(nodes, self.engine_usage.uses_etex_node_sizes());
        for node in nodes {
            node.visit_fonts(|font| {
                assert!(
                    self.fonts.contains(font),
                    "published page node retains a live Universe font"
                );
            });
        }
        let list = self
            .page_nodes
            .publish(nodes.to_vec())
            .expect("page construction contains only live page-arena children");
        self.engine_usage.observe_transient_memory(words.0, words.1);
        list
    }

    /// Publishes a page-lifetime node list by moving the caller's buffer.
    pub fn publish_page_nodes_owned(&mut self, nodes: &mut Vec<Node>) -> PageListId {
        let words = tex_memory_words(nodes, self.engine_usage.uses_etex_node_sizes());
        for node in nodes.iter() {
            node.visit_fonts(|font| {
                assert!(
                    self.fonts.contains(font),
                    "published page node retains a live Universe font"
                );
            });
        }
        let list = self
            .page_nodes
            .publish(core::mem::take(nodes))
            .expect("page construction contains only live page-arena children");
        self.engine_usage.observe_transient_memory(words.0, words.1);
        list
    }

    /// Resolves one list through this episode's page arena.
    pub fn page_node_list(
        &self,
        id: PageListId,
    ) -> Result<NodeList<'_, PageLifetime>, NodeArenaError> {
        self.page_nodes.get(id)
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
    pub fn page_node_cursor(&self) -> NodeArenaCursor<PageLifetime> {
        self.page_nodes.cursor()
    }

    /// Opens one nested page-storage suffix owned by a structural box or
    /// shipout operation.
    #[must_use]
    pub fn begin_page_node_region(&self) -> NodeArenaRegion<PageLifetime> {
        self.page_nodes.begin_region()
    }

    /// Consumes and releases a complete page-storage suffix after every
    /// survivor has crossed into durable storage or detached output.
    pub fn release_page_node_region(
        &mut self,
        region: NodeArenaRegion<PageLifetime>,
    ) -> Result<(), NodeArenaError> {
        self.page_nodes.release_region(region)
    }

    /// Transfers a failed nested suffix back to the enclosing page owner when
    /// an outer rollback can restore roots into it.
    pub fn retain_page_node_region(
        &self,
        region: NodeArenaRegion<PageLifetime>,
    ) -> Result<(), NodeArenaError> {
        self.page_nodes.retain_region(region)
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

    pub fn journal_cursor(&self) -> Result<JournalCursor<G>, UniverseError> {
        Ok(self
            .core
            .as_ref()
            .ok_or(UniverseError::Retired)?
            .admit()
            .state()
            .journal_cursor())
    }

    /// Current bytes retained by the ordered state journal.
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

    pub fn restore_state(&mut self, cursor: JournalCursor<G>) -> Result<(), UniverseError> {
        self.live_state_mut()?.restore(cursor)?;
        Ok(())
    }

    /// Retains one coarse generation beside bounded state and arena cursors.
    ///
    /// Command, mode, source, effect, and output owners compose their own
    /// bounded cursor fields around this state-layer foundation. A checkpoint
    /// with no page-handle carrier records only the generation's conservative
    /// retained page bound; incidental rootless allocation is not history.
    pub fn state_checkpoint(&self) -> Result<StateCheckpoint<G>, UniverseError> {
        self.state_checkpoint_at(self.retained_page_bound)
    }

    fn state_checkpoint_at(
        &self,
        page: NodeArenaCursor<PageLifetime>,
    ) -> Result<StateCheckpoint<G>, UniverseError> {
        let core = self.core.as_ref().ok_or(UniverseError::Retired)?;
        Ok(GenerationCheckpoint::new(
            core.generation_owner(),
            BoundedStateMark::new(
                core.state().journal_cursor(),
                core.durable_node_cursor(),
                page,
                (),
            ),
        ))
    }

    /// Exact page cursor for an operation-local rollback owner.
    ///
    /// This mark is intentionally separate from named checkpoints: a
    /// speculative shipout must be able to restore its opening suffix, while
    /// retained history must not pin rootless shipout construction.
    fn operation_state_checkpoint(&self) -> Result<StateCheckpoint<G>, UniverseError> {
        self.state_checkpoint_at(self.page_nodes.cursor())
    }

    /// Captures the complete state-facing portion of an executor checkpoint.
    /// Runtime ids remain reachable only through the single retained state
    /// generation and opaque subsystem roots.
    pub fn runtime_checkpoint(&mut self) -> Result<RuntimeCheckpoint<G>, UniverseError> {
        self.runtime_checkpoint_with_page_roots(false)
    }

    /// Captures runtime roots while incorporating executor-owned page
    /// carriers into the generation's monotonic retained prefix.
    #[doc(hidden)]
    pub fn runtime_checkpoint_with_page_roots(
        &mut self,
        external_page_roots: bool,
    ) -> Result<RuntimeCheckpoint<G>, UniverseError> {
        let carries_page_roots = external_page_roots || self.page.retains_page_node_handles();
        if carries_page_roots {
            self.retained_page_bound = self.page_nodes.cursor();
        }
        let checkpoint = RuntimeCheckpoint {
            state: self.state_checkpoint()?,
            page: self.page.clone(),
            pdf: self.pdf.snapshot(),
            world: self.world.snapshot(),
            fonts: self.fonts.watermark(),
            sources: self.sources.watermark(),
            hyphenation: self.hyphenation.clone(),
            dependencies: self.dependencies.snapshot_tracker(),
            interaction_mode: self.interaction_mode,
            prepared_mag: self.prepared_mag,
            engine_usage: self.engine_usage.clone(),
        };
        if !carries_page_roots {
            self.release_unretained_page_suffix()?;
        }
        Ok(checkpoint)
    }

    /// Releases only rootless page rows above the generation's monotonic
    /// retained checkpoint prefix.
    pub fn release_unretained_page_suffix(&mut self) -> Result<(), UniverseError> {
        self.page_nodes.truncate(self.retained_page_bound)?;
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

    /// Returns whether `font` is an exact immutable-row coordinate retained
    /// by this runtime checkpoint's font-store prefix.
    #[must_use]
    pub fn runtime_checkpoint_retains_font(
        &self,
        checkpoint: &RuntimeCheckpoint<G>,
        font: crate::ids::FontId,
    ) -> bool {
        self.fonts.validates(checkpoint.fonts) && self.fonts.contains_at(checkpoint.fonts, font)
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
        let owner = checkpoint.state.owner();
        let mark = checkpoint.state.mark();
        <Self as RestoreTarget<GenerationOwner<G>, StateCheckpointMark<G>>>::validate_restore(
            self, owner, mark,
        )?;
        if !(if generation_fork {
            self.world.snapshot_is_forkable(&checkpoint.world)
        } else {
            self.world.snapshot_is_retained(&checkpoint.world)
        }) || !self.pdf.snapshot_is_retained(&checkpoint.pdf)
            || !self.fonts.validates(checkpoint.fonts)
            || !self.sources.validates(checkpoint.sources)
        {
            return Err(UniverseError::State(StateError::InvalidCursor));
        }
        let font_survives = |font| self.fonts.contains_at(checkpoint.fonts, font);
        let core = self.core.as_ref().ok_or(UniverseError::Retired)?;
        if !self
            .primitive_meanings
            .iter()
            .all(|meaning| meaning.font().is_none_or(font_survives))
            || !core
                .state()
                .restored_font_roots_are_live(*mark.journal(), font_survives)?
            || !core.durable_font_roots_are_live(*mark.durable(), font_survives)?
            || !self
                .page_nodes
                .font_roots_are_live(*mark.page(), font_survives)?
            || !checkpoint.page.font_roots_are_live(font_survives)
            || !self
                .pdf
                .snapshot_font_roots_are_live(&checkpoint.pdf, font_survives)
        {
            return Err(UniverseError::State(StateError::InvalidCursor));
        }

        <Self as RestoreTarget<GenerationOwner<G>, StateCheckpointMark<G>>>::acquire_target_owner(
            self,
            owner.clone(),
        );
        <Self as RestoreTarget<GenerationOwner<G>, StateCheckpointMark<G>>>::restore_dense_state(
            self, mark,
        );
        self.page = checkpoint.page.clone();
        if !generation_fork {
            self.pdf.rollback(checkpoint.pdf.clone());
        }
        if generation_fork {
            self.world.install_checkpoint_fork(&checkpoint.world);
        } else {
            self.world.rollback(&checkpoint.world);
        }
        self.hyphenation = checkpoint.hyphenation.clone();
        self.dependencies.restore_tracker(&checkpoint.dependencies);
        self.interaction_mode = checkpoint.interaction_mode;
        self.prepared_mag = checkpoint.prepared_mag;
        self.engine_usage = checkpoint.engine_usage.clone();
        transfer_external_roots();
        <Self as RestoreTarget<GenerationOwner<G>, StateCheckpointMark<G>>>::transfer_roots(
            self, mark,
        );
        self.core
            .as_mut()
            .expect("restore plan validated a live state core")
            .state_mut()
            .truncate_font_runtime(checkpoint.fonts.len)
            .expect("font runtime prefix follows the validated font-store mark");
        self.fonts.truncate_to(checkpoint.fonts);
        self.sources.truncate_to(checkpoint.sources);
        <Self as RestoreTarget<GenerationOwner<G>, StateCheckpointMark<G>>>::truncate_suffixes(
            self, mark,
        );
        <Self as RestoreTarget<GenerationOwner<G>, StateCheckpointMark<G>>>::release_replaced_owners(
            self,
        );
        Ok(())
    }

    /// Begins one exclusive, rollback-capable artifact publication.
    #[must_use]
    pub fn begin_shipout(&mut self) -> ShipoutTransaction<'_, G> {
        let rollback = ShipoutRollback {
            state: self
                .operation_state_checkpoint()
                .expect("live shipout generation can be retained"),
            page: self.page.clone(),
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
            primitive_names: &self.primitive_names,
            primitive_meanings: &self.primitive_meanings,
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
            || !self
                .core
                .as_ref()
                .ok_or(UniverseError::Retired)?
                .can_retire()
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
        if !self
            .core
            .as_ref()
            .ok_or(UniverseError::Retired)?
            .can_retire()
        {
            return Err(UniverseError::State(StateError::GenerationInUse));
        }
        self.core
            .take()
            .ok_or(UniverseError::Retired)?
            .retire()
            .map_err(UniverseError::State)
    }

    #[must_use]
    pub const fn is_retired(&self) -> bool {
        self.core.is_none()
    }
}

impl<G> ShipoutTransaction<'_, G> {
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
            self.rollback = None;
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
        self.rollback = None;
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
