//! Top-level TeX state timeline.
//!
//! `Universe` is the only public checkpoint/rollback boundary. The older
//! `Stores` aggregate remains as private composition because its facade already
//! enforces handle liveness and couples Env/content/code-table rollback. The
//! public timeline tuple lives here so future World/effect/input state cannot
//! grow a partial rollback API beside the store tuple.

use crate::cell::{BankTag, CellId};
use crate::code_tables::{CodeTableGenerations, DelCode, LcCode, MathCode, SfCode, UcCode};
use crate::dependency::{
    ChangedAt, DependencyCodeTable, DependencyEngineField, DependencyFontField, DependencyKey,
    DependencyPageField, DependencyRegionError, DependencyRegionToken, DependencyRuntime,
    DependencyTrackerSnapshot, DependencyValue, DependencyWorldField, ObservedDependency,
    TrackedRegionBarrier,
};
#[cfg(test)]
use crate::env::Env;
use crate::env::banks::{DimenParam, GlueParam, IntParam, TokParam};
use crate::epoch::Epoch;
use crate::font::{
    CharMetrics, ExtensibleRecipe, FontMetrics, LigKernChar, LigKernCommand, LigKernIter,
    LoadedFont, MissingCharacter,
};
use crate::glue::{GlueSpec, GlueSpecRef, Order};
use crate::hyphenation::{ExceptionSpec, PatternSpec};
use crate::ids::{FontId, GlueId, MacroDefinitionId, TokenListId};
use crate::input::{
    ConditionKind, ConditionLimb, InputFrameSummary, InputSemanticRoot, InputSummary, LexerState,
    SourceId, TokenListReplayKind, TracedTokenList,
};
use crate::interner::{ControlSequenceKind, Symbol, SymbolId};
use crate::macro_store::{MacroDefinitionProvenance, MacroDefinitionRef, MacroMeaning};
use crate::math::MathFontSize;
use crate::meaning::Meaning;
use crate::node::{GlueKind, KernKind, MarginKernSide, Node, Whatsit};
use crate::node_arena::{NodeList, NodeListBuilder, NodeListRef};
use crate::page::{
    PageBreak, PageBuilderState, PageContents, PageDimension, PageFireUp, PageHashCache,
    PageInsertion, PageInteger, PageMark, PageMemoState, PageStateHashCursor,
};
use crate::patch_domain::{PatchAllocationDomain, PatchOperationMark};
use crate::pdf::{
    PdfDocumentFragmentKind, PdfDocumentObjectIds, PdfExternalImageId, PdfExternalImageMetadata,
    PdfExternalImageRegistrationError, PdfFontResourceRecord, PdfFormatState,
    PdfObjectCapacityError, PdfOutputParameters, PdfPageParameters, PdfRawObjectData,
    PdfRawObjectId, PdfRawObjectInitializeError, PdfRawObjectRecord, PdfState, PdfStateCursor,
    PdfStateSnapshot, PdfTokenParameter,
};
use crate::provenance::{
    ExpansionFrameRef, InsertedOriginKind, OriginListRef, OriginRecord, OriginRef,
    SynthesizedOriginKind, SyntheticOriginKind,
};
use crate::provenance::{
    MacroInvocationProvenanceStats, ProvenanceBudgets, ProvenanceDemand, ProvenanceStats,
};
use crate::scaled::Scaled;
use crate::source_map::{
    GeneratedSource, RegisteredSource, SourceBacking, SourceDescriptor, SourceMapError, SourcePos,
    SourceRegion, SourceSpan,
};
use crate::state_hash::{
    CachedProjection, INITIAL_STATE_HASH, StateHashComponent, StateHashFragment, StateHasher,
    combine,
};
#[cfg(any(test, feature = "testing"))]
use crate::stores::TestingOwnershipCensus;
use crate::stores::{DirectStoreOperationMark, StorePatchOperationMark, StoreStateHashCursor};
use crate::stores::{
    FontParameterError, GroupKind, GroupMismatch, PrepareMagDiagnostic, StoreFormatError,
    StoreSnapshot, Stores,
};
use crate::token::{Catcode, OriginId, Token, TracedTokenWord};
use crate::token_store::{TokenListBuilder, TokenListRef};
use crate::world::{
    CommittedArtifact, ContentHash, EffectPos, EffectRecord, JobClock, PrintSink,
    ShellEscapePolicy, ShellEscapeRecord, StreamBufState, StreamSlot, World, WorldCommitMode,
    WorldError, WorldSnapshot, WorldStateHashCursor, install_job_clock_params,
};
use std::collections::HashMap;
use std::ops::{Deref, DerefMut};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};

/// Input file reads available to a driver-supplied `\input` resolver.
///
/// This is intentionally separate from [`Universe`] so input resolvers cannot
/// see state reads and mutations or general [`World`] mutation APIs.
pub trait InputReadState {
    fn read_input_file(
        &mut self,
        path: &std::path::Path,
    ) -> Result<crate::FileContent, crate::WorldError>;

    /// Reads an uncommitted TeX output only when this run has generated the
    /// requested path. Driver-owned resource policy remains responsible for
    /// every other input source.
    fn read_pending_output_file(
        &mut self,
        _path: &std::path::Path,
    ) -> Result<Option<crate::FileContent>, crate::WorldError> {
        Ok(None)
    }

    /// Records immutable bytes selected by a driver-owned resolver as a
    /// `World` input while preserving pending-output precedence.
    fn read_supplied_input_file(
        &mut self,
        path: &std::path::Path,
        bytes: std::sync::Arc<[u8]>,
    ) -> Result<crate::FileContent, crate::WorldError>;

    /// Records a completed semantic lookup against a canonical host path.
    fn record_input_dependency(
        &mut self,
        _path: &std::path::Path,
        _outcome: crate::InputDependencyOutcome,
        _access: crate::InputDependencyAccess,
    ) -> Result<(), crate::WorldError> {
        Ok(())
    }
}

/// State operations available only to the top-level `\input` dispatch path.
///
/// Helper code that is generic over ordinary state access cannot derive
/// input-file read access through this separate capability.
pub trait InputOpenState {
    type Input<'a>: InputReadState
    where
        Self: 'a;

    fn input_open_context(&mut self) -> Self::Input<'_>;
}

/// Production input-open capability over a [`Universe`].
pub struct InputOpenContext<'a> {
    universe: &'a mut Universe,
}

/// Opaque aggregate authority for one active tracked region.
///
/// The mark owns only runtime and environment-journal positions. It exposes no
/// substore, checkpoint, or replay capability.
#[derive(Debug)]
pub struct TrackedRegionMark {
    owner: SnapshotOwner,
    dependency: DependencyRegionToken,
    environment: crate::env::JournalRegionMark,
}

/// Driver-only mutable access to [`World`] with precise dependency stamping.
///
/// The guard compares only already-tracked World facts and advances the stamp
/// for each fact whose canonical projection changed during the borrow.
pub struct WorldMut<'a> {
    world: &'a mut World,
    dependencies: &'a mut DependencyRuntime,
    before: Vec<(DependencyKey, DependencyValue)>,
}

impl Deref for WorldMut<'_> {
    type Target = World;

    fn deref(&self) -> &Self::Target {
        self.world
    }
}

impl DerefMut for WorldMut<'_> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.world
    }
}

impl Drop for WorldMut<'_> {
    fn drop(&mut self) {
        for &(key, ref before) in &self.before {
            let Some(after) = world_backed_dependency_value(self.world, key) else {
                continue;
            };
            if &after != before {
                self.dependencies.mark_changed(key);
            }
        }
    }
}

/// One canonical environment cell written by a tracked region.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TrackedEnvironmentWrite {
    cell: CellId,
    value: DependencyValue,
}

impl TrackedEnvironmentWrite {
    #[must_use]
    pub const fn cell(&self) -> CellId {
        self.cell
    }

    #[must_use]
    pub const fn value(&self) -> &DependencyValue {
        &self.value
    }
}

/// Detached evidence recorded by one successfully finished region.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TrackedRegionRecord {
    observations: Vec<ObservedDependency>,
    environment_writes: Vec<TrackedEnvironmentWrite>,
}

impl TrackedRegionRecord {
    #[must_use]
    pub fn observations(&self) -> &[ObservedDependency] {
        &self.observations
    }

    #[must_use]
    pub fn environment_writes(&self) -> &[TrackedEnvironmentWrite] {
        &self.environment_writes
    }
}

/// Typed rejection from the aggregate tracked-region lifecycle.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TrackedRegionError {
    AlreadyActive,
    NoActiveRegion,
    ForeignMark,
    StaleMark,
    UnsupportedTimelineChange,
    UnsupportedEnvironmentCell(CellId),
    UnsupportedRegion(TrackedRegionBarrier),
}

impl std::fmt::Display for TrackedRegionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::AlreadyActive => f.write_str("a tracked region is already active"),
            Self::NoActiveRegion => f.write_str("no tracked region is active"),
            Self::ForeignMark => f.write_str("the tracked region mark belongs to another Universe"),
            Self::StaleMark => f.write_str("the tracked region mark is stale"),
            Self::UnsupportedTimelineChange => {
                f.write_str("the environment journal lineage changed inside the tracked region")
            }
            Self::UnsupportedEnvironmentCell(cell) => {
                write!(
                    f,
                    "environment cell {cell:?} has no detached semantic projection"
                )
            }
            Self::UnsupportedRegion(barrier) => {
                write!(f, "the tracked region is unsupported: {barrier:?}")
            }
        }
    }
}

impl std::error::Error for TrackedRegionError {}

impl From<DependencyRegionError> for TrackedRegionError {
    fn from(value: DependencyRegionError) -> Self {
        match value {
            DependencyRegionError::AlreadyActive => Self::AlreadyActive,
            DependencyRegionError::NoActiveRegion => Self::NoActiveRegion,
            DependencyRegionError::StaleToken => Self::StaleMark,
            DependencyRegionError::Unsupported(barrier) => Self::UnsupportedRegion(barrier),
        }
    }
}

impl<'a> InputOpenContext<'a> {
    #[must_use]
    pub fn new(universe: &'a mut Universe) -> Self {
        Self { universe }
    }
}

/// A whole-Universe rollback snapshot.
///
/// Snapshot capture is O(1): the private store snapshot is a tuple of marks,
/// roots, and positions; the remaining fields are small scalar placeholders
/// for M3 World/input state.
#[derive(Clone, Debug)]
pub struct Snapshot {
    geometry_observations_len: usize,
    owner: SnapshotOwner,
    serial: u64,
    store: StoreSnapshot,
    epoch: Epoch,
    world: WorldSnapshot,
    input_summary: InputSummary,
    interaction_mode: InteractionMode,
    page: PageBuilderState,
    pdf: PdfStateSnapshot,
    exact_state_identity: Option<u64>,
    /// Fixed-size derived component roots matching this snapshot.
    state_hash_projection_cache: StateHashProjectionCache,
    dependency_tracker: DependencyTrackerSnapshot,
    state_hash: u64,
    state_hash_base: StateHashBase,
}

/// Fixed-size allocation-domain mark for one preflighted executor operation.
///
/// Semantic owners commit directly or through their own journals. This mark
/// owns only a non-restoring environment-journal cursor and the disposable
/// private-revision allocation suffix, and therefore retains no aggregate
/// state roots.
#[doc(hidden)]
#[derive(Debug)]
pub struct DirectOperationMark {
    store: DirectStoreOperationMark,
    patch_operation: Option<PatchOperationMark>,
    patch_store: Option<StorePatchOperationMark>,
}

/// One immutable accepted-generation state substrate shared by O(1) snapshots.
#[derive(Debug)]
pub struct GenerationSubstrate {
    universe: Universe,
    charged_bytes: usize,
}

/// Rejection from the narrow validated generation-fork/retarget operations.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GenerationForkError {
    ForeignSnapshot,
    InvalidatedSnapshot,
    PrefixBeyondForkAnchor,
    UnrelatedFork,
    InvalidMappedAnchor,
    RootRevisionMismatch,
    ChangedRootInterval,
}

/// A private revision cannot become accepted until every domain allocation is
/// named by an explicit typed semantic or detached-output root.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PrivateRevisionAcceptanceError {
    ActiveOperation,
    UnrootedAllocations,
}

impl std::fmt::Display for PrivateRevisionAcceptanceError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::ActiveOperation => "private revision still owns an active allocation operation",
            Self::UnrootedAllocations => {
                "private revision has allocations without explicit accepted roots"
            }
        })
    }
}

impl std::error::Error for PrivateRevisionAcceptanceError {}

impl std::fmt::Display for GenerationForkError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::ForeignSnapshot => "checkpoint belongs to another generation substrate",
            Self::InvalidatedSnapshot => "checkpoint roots are no longer retained",
            Self::PrefixBeyondForkAnchor => "checkpoint is after the fork anchor",
            Self::UnrelatedFork => "target substrate was not forked from the source generation",
            Self::InvalidMappedAnchor => "mapped editor anchor is outside a UTF-8 boundary",
            Self::RootRevisionMismatch => "checkpoint root revision does not match the source",
            Self::ChangedRootInterval => "mapped root interval is not byte-identical",
        })
    }
}

impl std::error::Error for GenerationForkError {}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ForkOrigin {
    source_owner: SnapshotOwner,
    anchor_serial: u64,
}

#[derive(Debug)]
pub struct ShipoutTransaction<'a> {
    universe: &'a mut Universe,
    rollback: Option<ScopedRollback>,
    finished: bool,
}

/// Ordered page suffix retained outside artifact and PDF publication.
///
/// Native finalization owns this value across retry. Dropping it is rollback:
/// neither its artifacts nor its PDF page records can become observable.
#[derive(Clone)]
pub struct PreparedPageSuffix {
    artifacts: Vec<CommittedArtifact>,
    artifact_publications: Vec<crate::ArtifactPublicationRecord>,
    pdf_pages: Vec<crate::PdfPageRecord>,
    effects: Vec<(EffectPos, EffectRecord)>,
}

impl PreparedPageSuffix {
    #[must_use]
    pub fn artifacts(&self) -> &[CommittedArtifact] {
        &self.artifacts
    }

    pub fn artifacts_mut(&mut self) -> &mut [CommittedArtifact] {
        &mut self.artifacts
    }

    /// Returns the ordered PDF page records detached with the artifacts.
    ///
    /// Native drivers use this read-only view while constructing fallible
    /// output. Publication remains deferred until every effect commits.
    #[must_use]
    pub fn pdf_pages(&self) -> &[crate::PdfPageRecord] {
        &self.pdf_pages
    }

    #[must_use]
    pub fn effects(&self) -> &[(EffectPos, EffectRecord)] {
        &self.effects
    }
}

/// Full-state rollback guard for a speculative replay transition.
///
/// This is deliberately an opaque, lifetime-bound capability. Dropping it
/// restores the aggregate state captured at construction; [`Self::commit`]
/// keeps the transition. It avoids the semantic hashing performed by durable
/// editor snapshots.
#[doc(hidden)]
#[derive(Debug)]
pub struct ReplayProbeTransaction<'a> {
    universe: &'a mut Universe,
    rollback: Option<ScopedRollback>,
}

#[derive(Debug)]
struct ScopedRollback {
    owner: SnapshotOwner,
    store: StoreSnapshot,
    world: WorldSnapshot,
    input_summary: InputSummary,
    interaction_mode: InteractionMode,
    page: PageBuilderState,
    pdf: PdfStateSnapshot,
    state_hash_base: StateHashBase,
    state_hash_projection_cache: StateHashProjectionCache,
    dependency_tracker: DependencyTrackerSnapshot,
    geometry_observations_len: usize,
}

#[derive(serde::Deserialize, serde::Serialize)]
struct PageMemoWire {
    state: PageMemoState,
    detached_nodes: Vec<u8>,
}

/// Opaque allocation mark for one in-progress box-register construction.
///
/// Finishing the assignment promotes its live result into rollback-safe
/// storage, then releases every epoch node allocated during construction.
#[derive(Debug)]
pub struct BoxBuildTransaction<'a> {
    universe: &'a mut Universe,
    finished: bool,
}

impl std::ops::Deref for ShipoutTransaction<'_> {
    type Target = Universe;
    fn deref(&self) -> &Self::Target {
        self.universe
    }
}

impl std::ops::Deref for ReplayProbeTransaction<'_> {
    type Target = Universe;

    fn deref(&self) -> &Self::Target {
        self.universe
    }
}

impl std::ops::DerefMut for ReplayProbeTransaction<'_> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.universe
    }
}

impl std::ops::DerefMut for ShipoutTransaction<'_> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.universe
    }
}

impl Drop for ShipoutTransaction<'_> {
    fn drop(&mut self) {
        if !self.finished {
            let rollback = self
                .rollback
                .take()
                .expect("unfinished shipout transaction retains rollback roots");
            self.universe.rollback_scoped(rollback);
        }
    }
}

impl Drop for ReplayProbeTransaction<'_> {
    fn drop(&mut self) {
        if let Some(rollback) = self.rollback.take() {
            self.universe.rollback_scoped(rollback);
        }
    }
}

impl ReplayProbeTransaction<'_> {
    /// Keeps the state transition performed through this guard.
    pub fn commit(mut self) {
        self.rollback = None;
    }
}

impl std::ops::Deref for BoxBuildTransaction<'_> {
    type Target = Universe;
    fn deref(&self) -> &Self::Target {
        self.universe
    }
}

impl std::ops::DerefMut for BoxBuildTransaction<'_> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.universe
    }
}

impl Drop for BoxBuildTransaction<'_> {
    fn drop(&mut self) {
        if !self.finished {
            self.universe.stores.finish_node_operation();
        }
    }
}

impl ShipoutTransaction<'_> {
    /// Atomically finishes this transaction's artifact/effect publication.
    pub fn commit(
        mut self,
        artifact: crate::world::VerifiedArtifact,
        effect_pos: EffectPos,
        reservation: crate::ArtifactPublicationReservation,
    ) -> Result<(ContentHash, crate::ArtifactPublicationRecord), WorldError> {
        if self.world.commit_mode() != WorldCommitMode::Retained {
            self.poison_tracked_region(TrackedRegionBarrier::IrreversibleEffect);
        }
        let output_parameters = self.current_pdf_output_parameters();
        let page_parameters = self.current_pdf_page_parameters();
        let pk_mode = self.current_pdf_token_parameter(TokParam::PDF_PK_MODE);
        self.observe_pdf_dependency(DependencyEngineField::PdfPages);
        self.pdf
            .ensure_page_capacity(output_parameters)
            .map_err(|()| WorldError::pdf_object_ids_exhausted())?;
        self.mark_pdf_dependency_changed(DependencyEngineField::PdfPages);
        let hash_base = self.state_hash_base.clone();
        let hash = self.world.store_verified_artifact(&artifact)?;
        if self.world.commit_mode() == WorldCommitMode::Retained {
            self.stores.finish_node_operation();
            self.state_hash_base = self.retarget_hash_base_after_committed_boundary(hash_base);
            self.set_page_integer(PageInteger::DeadCycles, 0);
            self.pdf
                .commit_page(hash, output_parameters, page_parameters, pk_mode);
            self.mark_pdf_dependency_changed(DependencyEngineField::PdfPages);
            let (bytes, render_provenance, open_out_occurrences) = artifact.into_parts();
            let record = reservation.record();
            self.world.record_artifact_commit(
                hash,
                bytes,
                render_provenance,
                open_out_occurrences,
                reservation,
            );
            self.rollback = None;
            self.finished = true;
            return Ok((hash, record));
        }
        if let Err(err) = self.world.commit_effects(effect_pos) {
            self.state_hash_base = self.retarget_hash_base_after_committed_boundary(hash_base);
            self.stores.finish_node_operation();
            self.rollback = None;
            self.finished = true;
            return Err(err);
        }
        self.stores.finish_node_operation();
        self.state_hash_base = self.retarget_hash_base_after_committed_boundary(hash_base);
        self.set_page_integer(PageInteger::DeadCycles, 0);
        self.pdf
            .commit_page(hash, output_parameters, page_parameters, pk_mode);
        self.mark_pdf_dependency_changed(DependencyEngineField::PdfPages);
        let (bytes, render_provenance, open_out_occurrences) = artifact.into_parts();
        let record = reservation.record();
        self.world.record_artifact_commit(
            hash,
            bytes,
            render_provenance,
            open_out_occurrences,
            reservation,
        );
        self.rollback = None;
        self.finished = true;
        Ok((hash, record))
    }
}

impl BoxBuildTransaction<'_> {
    /// Moves the result into the register store and commits the owned suffix.
    pub fn finish(mut self, index: u16, value: Option<NodeListRef>, global: bool) {
        let receipt = match (global, value) {
            (false, Some(value)) => self.stores.write_box_reg_ref(index, Some(value), false),
            (true, Some(value)) => self.stores.write_box_reg_ref(index, Some(value), true),
            (false, None) => self.stores.clear_box_reg(index),
            (true, None) => self.stores.clear_box_reg_global(index),
        };
        self.consume_env_mutation(receipt);
        self.stores.finish_node_operation();
        self.finished = true;
    }
}

impl Snapshot {
    /// Returns the epoch captured by this snapshot.
    ///
    /// Rollback does not restore this value; the live Universe always bumps
    /// forward from its current maximum epoch after restoring state.
    #[must_use]
    pub const fn epoch(&self) -> Epoch {
        self.epoch
    }

    /// Returns the schedule-relative convergence lineage captured by this snapshot.
    ///
    /// The hash is a fold of semantic slice hashes over the checkpoint
    /// timeline (`combine(previous_checkpoint_hash, slice_hash)`), so it is
    /// checkpoint-schedule-relative: it witnesses "same lineage observed at
    /// the same checkpoint boundaries", not a canonical fingerprint of the
    /// reached state. Compare hashes only between runs
    /// that take checkpoints at the same positions under the same policy;
    /// see `docs/core_state.md` §9 (convergence detection).
    #[must_use]
    pub const fn state_hash(&self) -> u64 {
        self.state_hash
    }

    /// Compares the probabilistic fixed-seed 64-bit aHash projection of every
    /// future-relevant root retained by two named checkpoints.
    ///
    /// The canonical store identity is deliberately unavailable inside open
    /// groups, which are not named convergence boundaries; that case is a safe
    /// miss. Detached effect and artifact history is excluded; callers splice
    /// those ordered prefixes separately. Equality is authoritative for this
    /// session-local optimization, so a rare collision may cause incorrect
    /// suffix reuse; durable identities retain their cryptographic contracts.
    #[must_use]
    pub fn exact_future_state_matches(&self, other: &Self) -> bool {
        self.exact_state_identity.is_some()
            && self.exact_state_identity == other.exact_state_identity
    }

    /// Compares detached output/effect slices captured by two snapshots.
    #[doc(hidden)]
    #[must_use]
    pub fn output_segment_matches(
        &self,
        effect_range: std::ops::Range<usize>,
        artifact_range: std::ops::Range<usize>,
        other: &Self,
        other_effect_range: std::ops::Range<usize>,
        other_artifact_range: std::ops::Range<usize>,
    ) -> bool {
        self.world.output_segment_matches(
            effect_range,
            artifact_range,
            &other.world,
            other_effect_range,
            other_artifact_range,
        )
    }

    /// Returns whether the optional composed canonical projection was captured.
    #[doc(hidden)]
    #[must_use]
    pub fn has_exact_state_identity(&self) -> bool {
        self.exact_state_identity.is_some()
    }
}

impl GenerationSubstrate {
    /// Exact test-only owner census for the frozen accepted generation.
    #[cfg(any(test, feature = "testing"))]
    #[must_use]
    pub fn testing_ownership_census(&self) -> TestingOwnershipCensus {
        self.universe.testing_ownership_census()
    }

    /// Freezes one completed mutable timeline as an accepted generation.
    #[must_use]
    pub fn new(universe: Universe) -> Self {
        let charged_bytes = generation_charged_bytes(&universe);
        Self {
            universe,
            charged_bytes,
        }
    }

    /// Opaque charged bytes shared by every checkpoint on this substrate.
    #[must_use]
    pub const fn charged_bytes(&self) -> usize {
        self.charged_bytes
    }

    /// Completes the private-revision ownership transition before this
    /// substrate becomes accepted session state.
    #[doc(hidden)]
    pub fn accept_private_revision(&mut self) -> Result<(), PrivateRevisionAcceptanceError> {
        self.universe.accept_private_revision()?;
        self.charged_bytes = generation_charged_bytes(&self.universe);
        Ok(())
    }

    /// Exact private-domain ownership for cross-crate lifecycle tests.
    #[cfg(any(test, feature = "testing"))]
    #[must_use]
    pub fn testing_private_revision_domain_stats(&self) -> Option<(usize, usize, usize, bool)> {
        self.universe.testing_private_revision_domain_stats()
    }

    #[must_use]
    pub const fn world(&self) -> &World {
        self.universe.world()
    }

    /// Resolves one diagnostic origin retained by this accepted generation.
    #[must_use]
    pub fn resolve_origin(
        &self,
        origin: crate::token::OriginId,
    ) -> Option<crate::ResolvedSourceLocation> {
        crate::ProvenanceResolver::new(&self.universe).resolve_origin(origin)
    }

    /// Resolves an artifact-owned structural root without importing it into
    /// this generation's provenance index.
    #[must_use]
    pub fn resolve_rooted_origin(
        &self,
        origin: &crate::provenance::OriginRef,
    ) -> Option<crate::ResolvedSourceLocation> {
        crate::ProvenanceResolver::new(&self.universe).resolve_origin_ref(origin)
    }

    /// Resolves one retained origin against the session's current editor layout.
    #[must_use]
    pub fn resolve_layout_origin(
        &self,
        origin: crate::token::OriginId,
        fragments: &crate::FragmentStore,
        layout: &crate::EditorLayout,
    ) -> crate::LayoutResolvedOrigin {
        crate::ProvenanceResolver::new(&self.universe)
            .resolve_layout_origin(origin, fragments, layout)
    }

    /// Resolves an artifact-owned structural root against the live editor
    /// layout without convergence-time origin import.
    #[must_use]
    pub fn resolve_layout_rooted_origin(
        &self,
        origin: &crate::provenance::OriginRef,
        fragments: &crate::FragmentStore,
        layout: &crate::EditorLayout,
    ) -> crate::LayoutResolvedOrigin {
        crate::ProvenanceResolver::new(&self.universe)
            .resolve_layout_origin_ref(origin, fragments, layout)
    }

    /// Resolves a stable paragraph recipe span directly at the diagnostic
    /// boundary, without first allocating a live `OriginId`.
    #[must_use]
    pub fn resolve_stable_layout_origin(
        &self,
        span: crate::RootSpanId,
        fragments: &crate::FragmentStore,
        layout: &crate::EditorLayout,
    ) -> crate::LayoutResolvedOrigin {
        crate::source_fragments::resolve_root_span(span, fragments, layout)
    }

    #[doc(hidden)]
    pub fn validate_checkpoint_snapshot(
        &self,
        checkpoint: &Snapshot,
    ) -> Result<(), GenerationForkError> {
        self.universe.validate_fork_snapshot(checkpoint)
    }

    #[must_use]
    pub fn root_content_hash(&self, summary: &InputSummary) -> Option<ContentHash> {
        self.universe.root_editor_content_hash(summary)
    }

    /// Clones this frozen generation once and atomically rolls the clone back
    /// to an exact owner-validated checkpoint.
    pub fn fork_at(&self, checkpoint: &Snapshot) -> Result<Universe, GenerationForkError> {
        self.fork_at_prepared(checkpoint, |_| ())
            .map(|(fork, ())| fork)
    }

    /// Clones a retained generation, prepares handle-free continuation data
    /// from the still-complete clone, then rolls that clone to `checkpoint`.
    ///
    /// Preparation cannot mutate the source generation. This ordering is the
    /// atomic boundary required by clients whose retained continuation can
    /// reach immutable arenas allocated after the selected checkpoint.
    pub fn fork_at_prepared<T>(
        &self,
        checkpoint: &Snapshot,
        prepare: impl FnOnce(&Universe) -> T,
    ) -> Result<(Universe, T), GenerationForkError> {
        self.universe.validate_fork_snapshot(checkpoint)?;
        let mut fork = self.universe.clone();
        let prepared = prepare(&fork);
        let checkpoint = fork.retarget_inherited_snapshot(checkpoint);
        fork.rollback_generation_fork(&checkpoint);
        fork.fork_origin = Some(ForkOrigin {
            source_owner: self.universe.owner.snapshot_owner(),
            anchor_serial: checkpoint.serial,
        });
        Ok((fork, prepared))
    }

    /// Retargets a source-generation prefix snapshot onto a promoted fork.
    /// This is deliberately limited to records at or before the exact fork anchor.
    pub fn retarget_prefix_from(
        &self,
        source: &GenerationSubstrate,
        checkpoint: &Snapshot,
    ) -> Result<Snapshot, GenerationForkError> {
        source.universe.validate_fork_snapshot(checkpoint)?;
        let origin = self
            .universe
            .fork_origin
            .ok_or(GenerationForkError::UnrelatedFork)?;
        if origin.source_owner != source.universe.owner.snapshot_owner() {
            return Err(GenerationForkError::UnrelatedFork);
        }
        if checkpoint.serial > origin.anchor_serial {
            return Err(GenerationForkError::PrefixBeyondForkAnchor);
        }
        Ok(self.universe.retarget_inherited_snapshot(checkpoint))
    }

    /// Consumes the accepted generation, installs the session-owned ordered
    /// effect history, materializes it exactly once, and returns the sealed World.
    pub fn export_detached_outputs(
        self,
        effects: Vec<EffectRecord>,
        artifacts: Vec<CommittedArtifact>,
        artifact_publications: Vec<crate::ArtifactPublicationRecord>,
    ) -> Result<World, WorldError> {
        let mut universe =
            self.into_detached_universe(effects, artifacts, artifact_publications)?;
        universe.export_retained_effects()?;
        Ok(universe.world)
    }

    /// Consumes the accepted generation and installs its detached outputs
    /// without committing effects. Client-owned finalizers can inspect the
    /// reached engine state and then choose whether to publish those effects.
    pub fn into_detached_universe(
        self,
        effects: Vec<EffectRecord>,
        artifacts: Vec<CommittedArtifact>,
        artifact_publications: Vec<crate::ArtifactPublicationRecord>,
    ) -> Result<Universe, WorldError> {
        let mut universe = self.universe;
        universe
            .world
            .replace_retained_outputs(effects, artifacts, artifact_publications)?;
        Ok(universe)
    }

    /// Materializes detached session output without consuming the retained
    /// generation used by later incremental revisions.
    pub fn materialize_detached_outputs(
        &self,
        effects: Vec<EffectRecord>,
        artifacts: Vec<CommittedArtifact>,
        artifact_publications: Vec<crate::ArtifactPublicationRecord>,
    ) -> Result<World, WorldError> {
        let mut world = self.universe.world.clone();
        world.replace_retained_outputs(effects, artifacts, artifact_publications)?;
        world.export_retained_effects()?;
        Ok(world)
    }
}

fn generation_charged_bytes(universe: &Universe) -> usize {
    universe
        .stores
        .generation_retained_bytes()
        .saturating_add(
            universe
                .private_revision_domain
                .as_ref()
                .map_or(0, PatchAllocationDomain::retained_bytes),
        )
        .saturating_add(std::mem::size_of::<Universe>())
        .saturating_add(universe.input_summary.retained_bytes())
        .saturating_add(universe.page.retained_bytes())
        .saturating_add(universe.world.generation_retained_bytes())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct SnapshotOwner {
    address: usize,
    nonce: u64,
}

#[derive(Debug)]
struct UniverseOwner(Box<UniverseOwnerToken>);

#[derive(Debug)]
struct UniverseOwnerToken {
    nonce: u64,
}

impl UniverseOwner {
    fn new() -> Self {
        Self(Box::new(UniverseOwnerToken {
            nonce: random_owner_nonce(),
        }))
    }

    fn snapshot_owner(&self) -> SnapshotOwner {
        SnapshotOwner {
            address: self.0.as_ref() as *const UniverseOwnerToken as usize,
            nonce: self.0.nonce,
        }
    }
}

fn random_owner_nonce() -> u64 {
    let state = ahash::RandomState::new();
    state.hash_one(0x756e_6976_6572_7365_u64)
}

/// Current engine interaction mode.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub enum InteractionMode {
    /// Stop at recoverable errors.
    Batch,
    /// Stop and report recoverable errors without terminal prompting.
    Nonstop,
    /// Scroll through recoverable errors.
    Scroll,
    /// TeX's ordinary interactive mode.
    #[default]
    ErrorStop,
}

/// Validation or encoding failure for an Umber semantic format image.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FormatError {
    OpenGroups(u32),
    NonEmptyInput,
    NonEmptyPage,
    NonEmptyPdfDocument,
    BadMagic,
    UnsupportedVersion(u32),
    Truncated,
    TrailingBytes,
    Checksum,
    IncompatibleAbi(u64),
    IncompatibleLookupConfiguration(u64),
    InvalidInteractionMode(u8),
    InvalidState(String),
}

#[derive(Deserialize, Serialize)]
struct UniverseFormatPayload {
    interaction_mode: u8,
    pdf: PdfFormatState,
    string_pool: crate::stores::StringPoolAccounting,
}

impl std::fmt::Display for FormatError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::OpenGroups(depth) => write!(f, "cannot dump a format with {depth} open groups"),
            Self::NonEmptyInput => f.write_str("cannot dump a format with live input state"),
            Self::NonEmptyPage => f.write_str("cannot dump a format with page-builder material"),
            Self::NonEmptyPdfDocument => {
                f.write_str("cannot dump a format with non-format PDF document state")
            }
            Self::BadMagic => f.write_str("not an Umber format file"),
            Self::UnsupportedVersion(version) => {
                write!(f, "unsupported Umber format version {version}")
            }
            Self::Truncated => f.write_str("truncated Umber format file"),
            Self::TrailingBytes => f.write_str("trailing bytes in Umber format file"),
            Self::Checksum => f.write_str("Umber format checksum mismatch"),
            Self::IncompatibleAbi(found) => {
                write!(f, "incompatible Umber format ABI fingerprint {found:#018x}")
            }
            Self::IncompatibleLookupConfiguration(found) => write!(
                f,
                "incompatible Umber format lookup configuration {found:#018x}"
            ),
            Self::InvalidInteractionMode(mode) => {
                write!(f, "invalid format interaction mode {mode}")
            }
            Self::InvalidState(message) => write!(f, "invalid Umber format state: {message}"),
        }
    }
}

impl std::error::Error for FormatError {}

#[derive(Clone, Debug)]
struct StateHashBase {
    store: StoreStateHashCursor,
    world: WorldStateHashCursor,
    input_summary: InputSemanticRoot,
    input_fragment: StateHashFragment,
    interaction_mode: InteractionMode,
    page: PageStateHashCursor,
    pdf: PdfStateCursor,
    checkpoint_hash: u64,
}

const UNIVERSE_SLICE_DOMAIN: u64 = 0x756e_6976_6572_7365;
const WORLD_SLICE_DOMAIN: u64 = 0x776f_726c_645f_736c;
const WORLD_EFFECTS_DOMAIN: u64 = 0x776f_726c_645f_6566;
const WORLD_SHELL_ESCAPES_DOMAIN: u64 = 0x776f_726c_645f_7368;
const WORLD_SCALARS_DOMAIN: u64 = 0x776f_726c_645f_7363;
const WORLD_STREAMS_DOMAIN: u64 = 0x776f_726c_645f_6275;
/// plain.tex lines 11-17: the printable characters INITEX (tex.web §232)
/// leaves as `other_char` and that plain.tex gives conventional meanings.
const PLAIN_PRINTABLE_CATCODES: [(char, Catcode); 7] = [
    ('{', Catcode::BeginGroup),
    ('}', Catcode::EndGroup),
    ('$', Catcode::MathShift),
    ('&', Catcode::AlignmentTab),
    ('#', Catcode::Parameter),
    ('^', Catcode::Superscript),
    ('_', Catcode::Subscript),
];
const INPUT_PROJECTION_DOMAIN: u64 = 0x696e_7075_745f_7072;
const INTERACTION_PROJECTION_DOMAIN: u64 = 0x696e_7465_7261_6374;

#[derive(Clone, Debug, Default)]
struct StateHashProjectionCache {
    world_streams: Option<CachedProjection<Arc<StreamBufState>>>,
    input: Option<CachedProjection<InputSemanticRoot>>,
    page: PageHashCache,
    #[cfg(test)]
    input_hash_calls: usize,
}

/// One owned TeX state timeline.
#[derive(Clone, Debug)]
struct PrimitiveMeaningOwner {
    meaning: Meaning,
    /// Frozen macro sentinels are driver metadata rather than Env bindings,
    /// but they still own the definition occurrence named by their word.
    _macro_root: Option<MacroDefinitionRef>,
}

#[derive(Debug)]
pub struct Universe {
    owner: UniverseOwner,
    /// Disposable allocations owned by one private incremental revision.
    /// Absent from templates and accepted generations at rest.
    private_revision_domain: Option<PatchAllocationDomain>,
    stores: Stores,
    /// Immutable job-level selection and admission limits for optional
    /// provenance consumers. Excluded from semantic state and formats.
    provenance_demand: ProvenanceDemand,
    provenance_budgets: ProvenanceBudgets,
    world: World,
    interaction_mode: InteractionMode,
    /// Process-selected §79 context layout; excluded from formats, snapshots,
    /// and semantic hashes.
    error_context_widths: crate::print::ErrorContextWidths,
    input_summary: InputSummary,
    /// One-shot runtime marker set only when a format image starts a fresh job.
    pending_every_job: bool,
    /// Operational editor revision identity; excluded from snapshots and semantic hashes.
    editor_content_hash: Option<ContentHash>,
    page: PageBuilderState,
    pdf: PdfState,
    /// Driver-selected immutable primitive table. This is engine-mode
    /// metadata, not groupable or format semantic state; format drivers
    /// reconstruct it after loading.
    primitive_meanings: HashMap<String, PrimitiveMeaningOwner>,
    primitive_meanings_by_index: Vec<PrimitiveMeaningOwner>,
    primitive_names_by_index: Vec<String>,
    primitive_indices: HashMap<String, u16>,
    state_hash_base: StateHashBase,
    state_hash_projection_cache: StateHashProjectionCache,
    next_snapshot_serial: u64,
    fork_origin: Option<ForkOrigin>,
    /// Operational memo metadata; excluded from snapshots and semantic hashes.
    dependencies: Mutex<DependencyRuntime>,
    /// Allocation-free inactive fast path for dependency-aware getters.
    dependency_region_active: AtomicBool,
    /// Prevents dependency projection from recursively observing the getters
    /// used to build that same canonical projection.
    dependency_projection_active: AtomicBool,
    /// Driver-requested memo configuration. Execution consumes this once;
    /// retained values and acceptance policy never live in aggregate state.
    pure_memo_config: Option<crate::PureMemoConfig>,
    pure_memo_capability: std::sync::Weak<std::sync::Mutex<crate::PureMemoRuntime>>,
    geometry_observations: Vec<GeometryObservation>,
    geometry_observation_enabled: bool,
    /// tex.web's `line` and `pack_begin_line`, the two globals §660/§675's
    /// box diagnostics report positions from. Both are diagnostic-only --
    /// tex.web dumps neither into a format and neither is readable as an
    /// internal quantity -- so, like [`Self::error_context_widths`], they
    /// stay out of formats, snapshots, and semantic hashes.
    diagnostic_position: DiagnosticPosition,
}

/// TeX82 §177's `print_spec(p,"pt")`, used by §252 `show_eqtb` when §283
/// traces restoration of a named glue parameter.
fn format_restore_glue(spec: GlueSpec, normal_unit: &'static str) -> String {
    fn component_unit(order: Order, normal_unit: &'static str) -> &'static str {
        match order {
            Order::Normal => normal_unit,
            Order::Fil => "fil",
            Order::Fill => "fill",
            Order::Filll => "filll",
        }
    }

    let mut text = crate::scaled::print_scaled(spec.width);
    text.push_str(normal_unit);
    if spec.stretch.raw() != 0 {
        text.push_str(" plus ");
        text.push_str(&crate::scaled::print_scaled(spec.stretch));
        text.push_str(component_unit(spec.stretch_order, normal_unit));
    }
    if spec.shrink.raw() != 0 {
        text.push_str(" minus ");
        text.push_str(&crate::scaled::print_scaled(spec.shrink));
        text.push_str(component_unit(spec.shrink_order, normal_unit));
    }
    text
}

/// TeX82 §252's bounded token-list display used by §283 restore tracing.
fn format_restore_tokens(
    universe: &Universe,
    token_list: Option<TokenListId>,
    escape_char: i32,
) -> String {
    let mut value = String::new();
    if let Some(tokens) = token_list.map(|id| universe.tokens(id)) {
        let mut shown = 0;
        while shown < tokens.len() && value.chars().count() < 32 {
            crate::token_show::append_token_show_text(universe, tokens[shown], &mut value);
            shown += 1;
        }
        if shown < tokens.len() {
            value.push_str(&escaped_restore_name(escape_char, "ETC."));
        }
    }
    value
}

/// Decodes an environment token-parameter word and rebinds its stored handle
/// to this Universe's live token store.
fn restored_tok_param_tokens(universe: &Universe, stored: u64) -> Option<TokenListRef> {
    use crate::env::banks::{BankCodec, OptionalTokenListIdCodec};

    OptionalTokenListIdCodec::decode(stored).map(|id| universe.tokens(id))
}

/// Merged etex.web [17.233]'s `show_eqtb` representation for one of the four
/// penalty-array locations, decoded from Umber's internal token-list payload.
fn format_restore_penalty_array(universe: &Universe, stored: u64, escape_char: i32) -> String {
    let tokens = restored_tok_param_tokens(universe, stored);
    let tokens = tokens.as_deref().unwrap_or_default();
    assert_eq!(tokens.len() % 4, 0, "restored penalty array is truncated");
    let count = tokens.len() / 4;
    let mut value = count.to_string();
    if let Some(chunk) = tokens.get(..4) {
        let mut raw = [0_u8; 4];
        for (byte, token) in raw.iter_mut().zip(chunk) {
            let Token::Param(encoded) = token else {
                panic!("restored penalty array has a non-byte token");
            };
            *byte = *encoded;
        }
        value.push(' ');
        value.push_str(&i32::from_le_bytes(raw).to_string());
        if count > 1 {
            value.push_str(&escaped_restore_name(escape_char, "ETC."));
        }
    }
    value
}

/// tex.web's `line`, `pack_begin_line`, and the `mode_line` stack §804 reads
/// `pack_begin_line` from.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
struct DiagnosticPosition {
    /// The line number of the innermost open file, or 0 when input is not
    /// coming from a file.
    line: i32,
    source: Option<SourceId>,
    /// §661's `pack_begin_line`: 0 outside line breaking and alignment,
    /// §804's positive `mode_line` while `post_line_break` packs a
    /// paragraph's lines, §768's negative `mode_line` while `fin_align`
    /// packs an alignment's rows.
    pack_begin_line: i32,
    /// §1025's `output_active`, which §663 and §675 report in place of a
    /// line number: a box packed by the output routine has no meaningful
    /// source position.
    output_active: bool,
    /// §1091's `mode_line` for each open horizontal-mode nest level, which is
    /// the line the paragraph started on. §804 reads the innermost when it
    /// breaks that paragraph. tex.web keeps one per nest level; this is that
    /// per-level value for the levels that have one, which is what makes a
    /// paragraph inside a box inside a paragraph report its own start line
    /// rather than the outer paragraph's.
    paragraph_start_lines: Vec<i32>,
}

struct DependencyProjectionGuard<'a>(&'a AtomicBool);

impl<'a> DependencyProjectionGuard<'a> {
    fn enter(active: &'a AtomicBool) -> Self {
        let was_active = active.swap(true, Ordering::Relaxed);
        debug_assert!(!was_active);
        Self(active)
    }
}

impl Drop for DependencyProjectionGuard<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Relaxed);
    }
}

/// Canonical semantic hasher for executor-owned state at a named boundary.
///
/// Construction stays under [`Universe`] so handle-bearing mode state is
/// resolved through the owning stores rather than hashing runtime ids.
/// Finalized packing/shipout geometry retained only while observation is enabled.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GeometryObservation {
    Hpack {
        width_sp: i64,
        height_sp: i64,
        depth_sp: i64,
        line: u32,
        source: Option<SourceId>,
    },
    Vpack {
        width_sp: i64,
        height_sp: i64,
        depth_sp: i64,
        line: u32,
        source: Option<SourceId>,
    },
    Shipout {
        page_width_sp: i64,
        page_height_sp: i64,
        counts: [i32; 10],
        line: u32,
        source: Option<SourceId>,
    },
}

pub struct EngineBoundaryHasher<'a> {
    stores: &'a Stores,
    hasher: StateHasher,
    visits: usize,
}

impl EngineBoundaryHasher<'_> {
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

    pub fn nodes(&mut self, nodes: &[Node]) {
        self.visits += self
            .stores
            .hash_node_slice_semantic(nodes, &mut self.hasher);
    }

    pub fn node_list_ref(&mut self, owner: &NodeListRef) {
        self.hasher.tag(0x70);
        owner.semantic_id().apply(&mut self.hasher);
        self.visits += 1;
    }

    pub fn token_list(&mut self, id: TokenListId) {
        self.stores.hash_token_list_semantic(id, &mut self.hasher);
    }

    pub fn glue(&mut self, id: GlueId) {
        self.stores.hash_glue_semantic(id, &mut self.hasher);
    }

    pub fn font(&mut self, id: FontId) {
        self.stores.hash_font_semantic(id, &mut self.hasher);
    }

    pub fn code_table(&mut self, table: DependencyCodeTable) {
        self.stores
            .hash_dependency_code_table(table, &mut self.hasher);
    }

    pub fn meaning(&mut self, meaning: Meaning) {
        self.stores.hash_meaning_semantic(meaning, &mut self.hasher);
    }

    pub fn str(&mut self, value: &str) {
        self.hasher.str(value);
    }
}

/// One indent/width pair in TeX's current `\parshape` value.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ParagraphShapeLine {
    pub indent: Scaled,
    pub width: Scaled,
}

/// One of e-TeX's four group-scoped line-breaking penalty arrays.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum PenaltyArrayKind {
    InterLine,
    Club,
    Widow,
    DisplayWidow,
}

impl PenaltyArrayKind {
    const fn storage(self) -> TokParam {
        match self {
            Self::InterLine => TokParam::INTER_LINE_PENALTIES_INTERNAL,
            Self::Club => TokParam::CLUB_PENALTIES_INTERNAL,
            Self::Widow => TokParam::WIDOW_PENALTIES_INTERNAL,
            Self::DisplayWidow => TokParam::DISPLAY_WIDOW_PENALTIES_INTERNAL,
        }
    }
}

impl Clone for Universe {
    fn clone(&self) -> Self {
        assert!(
            self.private_revision_domain.is_none(),
            "a private revision allocation domain cannot be cloned"
        );
        let stores = self.stores.clone();
        let state_hash_base = StateHashBase {
            store: stores.retarget_state_hash_cursor(&self.state_hash_base.store),
            world: self.state_hash_base.world.clone(),
            input_summary: self.state_hash_base.input_summary.clone(),
            input_fragment: self.state_hash_base.input_fragment,
            interaction_mode: self.state_hash_base.interaction_mode,
            page: self.state_hash_base.page.clone(),
            pdf: self.state_hash_base.pdf.clone(),
            checkpoint_hash: self.state_hash_base.checkpoint_hash,
        };
        Self {
            owner: UniverseOwner::new(),
            private_revision_domain: None,
            stores,
            provenance_demand: self.provenance_demand,
            provenance_budgets: self.provenance_budgets,
            world: self.world.clone(),
            interaction_mode: self.interaction_mode,
            error_context_widths: self.error_context_widths,
            input_summary: self.input_summary.clone(),
            pending_every_job: self.pending_every_job,
            editor_content_hash: self.editor_content_hash,
            page: self.page.clone(),
            pdf: self.pdf.clone(),
            primitive_meanings: self.primitive_meanings.clone(),
            primitive_meanings_by_index: self.primitive_meanings_by_index.clone(),
            primitive_names_by_index: self.primitive_names_by_index.clone(),
            primitive_indices: self.primitive_indices.clone(),
            state_hash_base,
            state_hash_projection_cache: self.state_hash_projection_cache.clone(),
            next_snapshot_serial: self.next_snapshot_serial,
            fork_origin: self.fork_origin,
            dependencies: Mutex::new(
                self.dependencies
                    .lock()
                    .expect("dependency runtime mutex is not poisoned")
                    .clone(),
            ),
            dependency_region_active: AtomicBool::new(false),
            dependency_projection_active: AtomicBool::new(false),
            pure_memo_config: self.pure_memo_config,
            pure_memo_capability: self.pure_memo_capability.clone(),
            geometry_observations: self.geometry_observations.clone(),
            geometry_observation_enabled: self.geometry_observation_enabled,
            diagnostic_position: DiagnosticPosition::default(),
        }
    }
}

impl Default for Universe {
    fn default() -> Self {
        Self::new()
    }
}

impl Universe {
    /// Selects optional provenance consumers and their independent limits for
    /// subsequent job execution.
    ///
    /// This is operational configuration: it is cloned into revision forks,
    /// but excluded from formats, checkpoints, and semantic hashes.
    #[must_use]
    pub fn with_provenance_config(
        mut self,
        demand: ProvenanceDemand,
        budgets: ProvenanceBudgets,
    ) -> Self {
        self.provenance_demand = demand;
        self.provenance_budgets = budgets;
        self.stores.configure_provenance_budgets(budgets);
        self
    }

    /// Selects optional provenance consumers while retaining current limits.
    #[must_use]
    pub fn with_provenance_demand(self, demand: ProvenanceDemand) -> Self {
        let budgets = self.provenance_budgets;
        self.with_provenance_config(demand, budgets)
    }

    #[must_use]
    pub const fn provenance_demand(&self) -> ProvenanceDemand {
        self.provenance_demand
    }

    #[must_use]
    pub const fn provenance_budgets(&self) -> ProvenanceBudgets {
        self.provenance_budgets
    }

    /// Creates an isolated staging generation for atomic detached import.
    ///
    /// Private revision allocations cannot cross a generation fork, so an
    /// active private domain rejects staging instead of panicking in `Clone`.
    #[doc(hidden)]
    #[must_use]
    pub fn stage_detached_import(&self) -> Option<Self> {
        self.private_revision_domain.is_none().then(|| self.clone())
    }

    /// Applies the process-selected Web2C font-memory bound.
    ///
    /// This operational limit is intentionally excluded from formats,
    /// snapshots, and semantic hashes.
    pub fn configure_font_info_capacity(&mut self, capacity: usize) {
        self.stores.configure_font_info_capacity(capacity);
    }

    /// TeX82-shaped live allocator use projected from the typed stores.
    #[must_use]
    pub fn engine_usage_statistics(&mut self) -> crate::stores::EngineUsageStatistics {
        self.stores.engine_usage_statistics()
    }

    /// Merges runtime-only TeX82 stack maxima from their command/execution
    /// owners into §1334's aggregate diagnostic projection.
    pub fn record_engine_stack_usage(&mut self, usage: crate::stores::EngineStackUsage) {
        self.stores.record_engine_stack_usage(usage);
    }

    /// Live typed projection of TeX82's `save_ptr` for §1334 accounting.
    #[must_use]
    pub fn checked_save_stack_words(&self, save_group_source_lines: bool) -> usize {
        self.stores
            .checked_save_stack_words(save_group_source_lines)
    }

    /// Records completed TeX82 `make_string` allocations owned outside the
    /// control-sequence namespace.
    pub fn record_string_pool_allocations(&mut self, strings: usize, characters: usize) {
        self.stores.record_pool_strings(strings, characters);
    }

    /// Records one retained `make_string` result.
    pub fn make_string_pool_string(&mut self, value: &str) {
        self.stores.make_pool_string(value);
    }

    /// Interns one semantic name while retaining every canonical
    /// `make_string` result, including duplicate spellings.
    pub fn intern_retained_pool_string(&mut self, value: &str) -> SymbolId {
        self.stores.intern_retained_pool_string(value)
    }

    /// Records Web2C tex.ch [29.517]'s recycling `slow_make_string` result.
    pub fn slow_make_string_pool_string(&mut self, value: &str) {
        self.stores.slow_make_pool_string(value);
    }

    /// Registers a typed string already included in aggregate pool counters.
    pub fn remember_string_pool_string(&mut self, value: &str) {
        self.stores.remember_pool_string(value);
    }

    pub fn flush_string_pool_allocations(&mut self, strings: usize, characters: usize) {
        self.stores.flush_pool_strings(strings, characters);
    }

    /// Selects the engine's pre-input WEB string-pool vocabulary.
    pub fn select_string_pool_profile(&mut self, profile: crate::stores::StringPoolProfile) {
        self.stores.select_string_pool_profile(profile);
    }

    #[must_use]
    pub fn font_would_allocate(&self, font: &crate::font::LoadedFont) -> bool {
        self.stores.font_would_allocate(font)
    }

    #[must_use]
    pub fn string_pool_accounting(&self) -> crate::stores::StringPoolAccounting {
        self.stores.string_pool_accounting()
    }

    /// Returns TeX82 §638's live `(var_used, dyn_used)` projection.
    #[must_use]
    pub fn shipout_memory_usage(&mut self, shipped_node: Option<&Node>) -> (usize, usize) {
        self.stores.shipout_memory_usage(shipped_node)
    }

    /// Records scanner-owned one-word nodes before their typed token list is
    /// interned or installed in semantic state.
    pub fn observe_transient_token_words(&mut self, words: usize) {
        self.stores.observe_main_memory_dynamic_words(words);
    }

    /// Replays the pure line breaker's ordered variable-size scratch owners.
    pub fn observe_line_break_memory_search(&mut self, memory: &crate::PureBreakMemoryPlan) {
        self.stores.observe_line_break_memory_search(memory);
    }

    /// Releases line-break scratch after post-line-break packing has finished.
    pub fn observe_line_break_memory_cleanup(&mut self, memory: &crate::PureBreakMemoryPlan) {
        self.stores.observe_line_break_memory_cleanup(memory);
    }

    #[cfg(test)]
    fn testing_transient_memory_base_projections(&self) -> usize {
        self.stores.testing_transient_memory_base_projections()
    }

    #[cfg(test)]
    fn testing_main_memory_root_traversals(&self) -> usize {
        self.stores.testing_main_memory_root_traversals()
    }
    /// Removes an ordered suffix from committed artifact/PDF publication.
    pub fn prepare_page_suffix(&mut self, start: usize) -> PreparedPageSuffix {
        let effect_base = self.world.effect_pos().raw()
            - u64::try_from(self.world.effect_records().len()).unwrap_or(u64::MAX);
        PreparedPageSuffix {
            artifact_publications: self.world.take_artifact_publication_suffix(start),
            artifacts: self.world.take_artifact_suffix(start),
            pdf_pages: self.pdf.take_page_suffix(start),
            effects: self
                .world
                .effect_records()
                .iter()
                .cloned()
                .enumerate()
                .map(|(index, effect)| {
                    (
                        EffectPos::from_raw(
                            effect_base + u64::try_from(index).unwrap_or(u64::MAX) + 1,
                        ),
                        effect,
                    )
                })
                .collect(),
        }
    }

    /// Atomically publishes a prepared suffix after its fallible effects succeeded.
    pub fn publish_page_suffix(
        &mut self,
        mut suffix: PreparedPageSuffix,
    ) -> Result<(), WorldError> {
        if suffix.artifacts.len() != suffix.pdf_pages.len() && !suffix.pdf_pages.is_empty() {
            return Err(WorldError::new(
                "publish prepared page suffix",
                None,
                "artifact and PDF page suffixes are not aligned",
            ));
        }
        let mut hashes = Vec::with_capacity(suffix.artifacts.len());
        for artifact in &suffix.artifacts {
            hashes.push(self.world.store_prepared_artifact(artifact)?);
        }
        for (artifact, publication) in suffix
            .artifacts
            .into_iter()
            .zip(suffix.artifact_publications)
        {
            self.world.record_prepared_artifact(artifact, publication);
        }
        for (page, hash) in suffix.pdf_pages.iter_mut().zip(hashes) {
            page.retarget_artifact(hash);
        }
        self.pdf.restore_page_suffix(suffix.pdf_pages);
        Ok(())
    }
    pub const FORMAT_SCHEMA_VERSION: u32 = crate::format_container::SCHEMA_VERSION;
    /// Fingerprint of the current portable format container ABI.
    pub const FORMAT_ABI_FINGERPRINT: u64 = crate::format_container::ABI_FINGERPRINT;
    /// Fingerprint of the immutable lookup tables embedded in format images.
    pub const FORMAT_LOOKUP_CONFIGURATION_FINGERPRINT: u64 =
        crate::format_container::LOOKUP_CONFIGURATION_FINGERPRINT;

    /// Iterates immutable font records loaded in this timeline, including `nullfont`.
    pub fn loaded_fonts(&self) -> impl Iterator<Item = &LoadedFont> {
        self.stores.loaded_fonts()
    }

    /// Creates an isolated TeX state timeline.
    ///
    /// Code tables start at INITEX's values (tex.web §232 and §240), so
    /// `{`, `}`, `$`, `&`, `#`, `^`, and `_` are `other_char` until a format
    /// assigns them. Engines that need those conventions execute a format
    /// source; callers that only want the conventions without a format use
    /// [`Self::new_with_plain_catcodes`].
    #[must_use]
    pub fn new() -> Self {
        Self::with_world(World::default())
    }

    /// Creates an isolated TeX state timeline carrying plain.tex's printable
    /// `\catcode` assignments on top of INITEX's table.
    ///
    /// tex.web §232 leaves `{ } $ & # ^ _` as `other_char`; plain.tex lines
    /// 11-17 assign them their conventional meanings. This constructor stands
    /// in for that format prelude when a caller exercises grouping, macro
    /// parameters, math shifts, alignment, or scripts without loading one.
    #[must_use]
    pub fn new_with_plain_catcodes() -> Self {
        Self::new().with_plain_catcodes()
    }

    /// Applies plain.tex's printable `\catcode` assignments to an existing
    /// timeline. See [`Self::new_with_plain_catcodes`].
    #[must_use]
    pub fn with_plain_catcodes(mut self) -> Self {
        self.install_plain_catcodes();
        self
    }

    /// Assigns plain.tex's printable `\catcode` values in place. See
    /// [`Self::new_with_plain_catcodes`].
    pub fn install_plain_catcodes(&mut self) {
        for (character, catcode) in PLAIN_PRINTABLE_CATCODES {
            self.set_catcode_global(character, catcode);
        }
    }

    /// Selects the process-level TeX82 §79 pseudoprint widths.
    ///
    /// This is driver configuration, like Web2C's `error_line` and
    /// `half_error_line` bound variables; it is deliberately not dumped in a
    /// format or rolled back with engine state.
    pub fn set_error_context_widths(&mut self, widths: crate::print::ErrorContextWidths) {
        self.error_context_widths = widths;
    }

    #[must_use]
    pub(crate) const fn error_context_widths(&self) -> crate::print::ErrorContextWidths {
        self.error_context_widths
    }

    /// Creates an isolated TeX timeline backed by an explicit effect world.
    #[must_use]
    pub fn with_world(world: World) -> Self {
        let mut stores = Stores::new();
        let clock = world.job_clock();
        install_job_clock_params(
            &mut |param, value| {
                let _ = stores.set_int_param(param, value);
            },
            clock,
        );
        stores.initialize_exact_env_identity();
        stores.discard_exact_env_undo_history();
        let input_summary = InputSummary::default();
        let page = PageBuilderState::default();
        let pdf = PdfState::default();
        let input_fragment = hash_input_summary_fragment(&stores, &world, &input_summary);
        let state_hash_base = StateHashBase {
            store: stores.state_hash_cursor(),
            world: world.state_hash_cursor(),
            input_summary: input_summary.semantic_root(),
            input_fragment,
            interaction_mode: InteractionMode::default(),
            page: page.state_hash_cursor(),
            pdf: pdf.cursor(),
            checkpoint_hash: INITIAL_STATE_HASH,
        };
        Self {
            owner: UniverseOwner::new(),
            private_revision_domain: None,
            stores,
            provenance_demand: ProvenanceDemand::default(),
            provenance_budgets: ProvenanceBudgets::default(),
            world,
            interaction_mode: InteractionMode::default(),
            error_context_widths: crate::print::ErrorContextWidths::default(),
            input_summary,
            pending_every_job: false,
            editor_content_hash: None,
            page,
            pdf,
            primitive_meanings: HashMap::new(),
            primitive_meanings_by_index: Vec::new(),
            primitive_names_by_index: Vec::new(),
            primitive_indices: HashMap::new(),
            state_hash_base,
            state_hash_projection_cache: StateHashProjectionCache::default(),
            next_snapshot_serial: 0,
            fork_origin: None,
            dependencies: Mutex::new(DependencyRuntime::default()),
            dependency_region_active: AtomicBool::new(false),
            dependency_projection_active: AtomicBool::new(false),
            pure_memo_config: None,
            pure_memo_capability: std::sync::Weak::new(),
            geometry_observations: Vec::new(),
            geometry_observation_enabled: false,
            diagnostic_position: DiagnosticPosition::default(),
        }
    }

    /// Registers an original primitive meaning without changing the live
    /// control sequence. Used to reconstruct engine identity after format
    /// loading, where the current meaning may intentionally be shadowed.
    pub fn register_primitive_meaning(&mut self, name: &str, meaning: Meaning) {
        if let Some(previous) = self.primitive_meanings.get(name) {
            assert_eq!(
                previous.meaning, meaning,
                "primitive {name} was registered with conflicting meanings"
            );
            return;
        }
        let _macro_root = match meaning {
            Meaning::Macro { definition, .. } => Some(self.macro_definition_ref(definition)),
            _ => None,
        };
        let owned = PrimitiveMeaningOwner {
            meaning,
            _macro_root,
        };
        let index = u16::try_from(self.primitive_meanings_by_index.len())
            .expect("primitive registry exceeds frozen-token capacity");
        self.primitive_meanings
            .insert(name.to_owned(), owned.clone());
        self.primitive_meanings_by_index.push(owned);
        self.primitive_names_by_index.push(name.to_owned());
        self.primitive_indices.insert(name.to_owned(), index);
    }

    /// Registers and installs one primitive meaning.
    pub fn install_primitive_meaning(&mut self, name: &str, meaning: Meaning) {
        self.register_primitive_meaning(name, meaning);
        let symbol = self.intern(name);
        self.set_meaning(symbol, meaning);
    }

    #[must_use]
    pub fn primitive_meaning(&self, name: &str) -> Option<Meaning> {
        self.primitive_meanings.get(name).map(|owner| owner.meaning)
    }

    /// Returns the exact immutable primitive-registry cardinality for
    /// profile-installation conformance tests.
    #[cfg(any(test, feature = "testing"))]
    #[must_use]
    pub fn testing_primitive_count(&self) -> usize {
        self.primitive_meanings_by_index.len()
    }

    /// Returns TeX's assignment level for a live meaning cell.
    ///
    /// This deliberately narrow white-box projection derives ownership from
    /// the actual environment journal rather than duplicating binding
    /// metadata. Production consumers should resolve the live meaning.
    #[cfg(any(test, feature = "testing"))]
    #[must_use]
    pub fn testing_meaning_level(&self, symbol: impl crate::interner::SymbolReference) -> u32 {
        self.stores.testing_meaning_level(symbol)
    }

    /// Returns the first registered spelling for a primitive meaning.
    #[must_use]
    pub fn primitive_name(&self, meaning: Meaning) -> Option<&str> {
        self.primitive_meanings_by_index
            .iter()
            .position(|candidate| candidate.meaning == meaning)
            .and_then(|index| self.primitive_names_by_index.get(index))
            .map(String::as_str)
    }

    #[must_use]
    pub fn primitive_token(&self, name: &str) -> Option<Token> {
        self.primitive_indices
            .get(name)
            .copied()
            .map(Token::frozen_primitive)
    }

    /// The eqtb text of a frozen control sequence, for §262's `print_cs`.
    ///
    /// tex.web gives every frozen equivalent a real `text()`: `frozen_fi` is
    /// spelled `fi`, `frozen_par` is `par`, and so on, which is why a token
    /// list holding one displays as `\fi` rather than as its meaning.
    #[must_use]
    pub fn frozen_primitive_name(&self, token: Token) -> Option<&str> {
        let Token::Frozen(frozen) = token else {
            return None;
        };
        if frozen == crate::token::FrozenToken::END_TEMPLATE
            || frozen == crate::token::FrozenToken::END_V
        {
            return Some("endtemplate");
        }
        if frozen == crate::token::FrozenToken::RELAX {
            return Some("relax");
        }
        self.primitive_names_by_index
            .get(usize::from(frozen.primitive_index()?))
            .map(String::as_str)
    }

    #[must_use]
    pub fn frozen_primitive_meaning(&self, token: Token) -> Option<Meaning> {
        let Token::Frozen(frozen) = token else {
            return None;
        };
        self.primitive_meanings_by_index
            .get(usize::from(frozen.primitive_index()?))
            .map(|owner| owner.meaning)
    }

    /// Begins one generic tracked region.
    ///
    /// A fresh environment epoch ensures the journal records the first write
    /// to every cell after this boundary even when the preceding operation
    /// wrote the same cell in its epoch.
    pub fn begin_tracked_region(&mut self) -> Result<TrackedRegionMark, TrackedRegionError> {
        let dependency = self
            .dependencies
            .get_mut()
            .expect("dependency runtime mutex is not poisoned")
            .begin_region()?;
        self.dependency_region_active.store(true, Ordering::Release);
        let environment = self.stores.begin_dependency_journal_region();
        Ok(TrackedRegionMark {
            owner: self.owner.snapshot_owner(),
            dependency,
            environment,
        })
    }

    #[must_use]
    pub fn dependency_region_is_active(&self) -> bool {
        self.dependency_region_active.load(Ordering::Acquire)
    }

    /// Records a detached semantic read when a region is active.
    #[inline(always)]
    pub fn record_dependency(&self, key: DependencyKey, value: DependencyValue) {
        self.dependencies
            .lock()
            .expect("dependency runtime mutex is not poisoned")
            .record(key, value);
    }

    /// Marks an active tracked region unsupported without affecting TeX state.
    /// The first reason wins; with no recorder this is an allocation-free no-op.
    #[inline(always)]
    pub fn poison_tracked_region(&self, barrier: TrackedRegionBarrier) {
        if !self.dependency_region_is_active() {
            return;
        }
        self.dependencies
            .lock()
            .expect("dependency runtime mutex is not poisoned")
            .poison(barrier);
    }

    /// Finishes a tracked region into deterministic detached evidence.
    ///
    /// Timeline changes that can destructively compact the marked journal
    /// slice fail closed. The failed attempt also clears the recorder.
    pub fn finish_tracked_region(
        &mut self,
        mark: TrackedRegionMark,
    ) -> Result<TrackedRegionRecord, TrackedRegionError> {
        if mark.owner != self.owner.snapshot_owner() {
            self.dependencies
                .get_mut()
                .expect("dependency runtime mutex is not poisoned")
                .abandon_active_region();
            self.dependency_region_active
                .store(false, Ordering::Release);
            return Err(TrackedRegionError::ForeignMark);
        }
        if let Err(error) = self
            .dependencies
            .get_mut()
            .expect("dependency runtime mutex is not poisoned")
            .ensure_region(&mark.dependency)
        {
            self.dependencies
                .get_mut()
                .expect("dependency runtime mutex is not poisoned")
                .abandon_active_region();
            self.dependency_region_active
                .store(false, Ordering::Release);
            return Err(error.into());
        }
        let cells = match self
            .stores
            .dependency_journal_region_cells(mark.environment)
        {
            Ok(cells) => cells,
            Err(_) => {
                self.dependencies
                    .get_mut()
                    .expect("dependency runtime mutex is not poisoned")
                    .abandon_active_region();
                self.dependency_region_active
                    .store(false, Ordering::Release);
                return Err(TrackedRegionError::UnsupportedTimelineChange);
            }
        };
        let mut environment_writes = Vec::with_capacity(cells.len());
        let projection_guard = DependencyProjectionGuard::enter(&self.dependency_projection_active);
        for cell in cells {
            let Some(value) = self.tracked_environment_cell_value(cell) else {
                self.dependencies
                    .get_mut()
                    .expect("dependency runtime mutex is not poisoned")
                    .abandon_active_region();
                self.dependency_region_active
                    .store(false, Ordering::Release);
                return Err(TrackedRegionError::UnsupportedEnvironmentCell(cell));
            };
            environment_writes.push(TrackedEnvironmentWrite { cell, value });
        }
        drop(projection_guard);
        let observations = self
            .dependencies
            .get_mut()
            .expect("dependency runtime mutex is not poisoned")
            .finish_region(mark.dependency);
        self.dependency_region_active
            .store(false, Ordering::Release);
        let observations = observations?;
        Ok(TrackedRegionRecord {
            observations,
            environment_writes,
        })
    }

    /// Atomically abandons a tracked region without publishing evidence.
    pub fn abandon_tracked_region(
        &mut self,
        mark: TrackedRegionMark,
    ) -> Result<(), TrackedRegionError> {
        if mark.owner != self.owner.snapshot_owner() {
            self.dependencies
                .get_mut()
                .expect("dependency runtime mutex is not poisoned")
                .abandon_active_region();
            self.dependency_region_active
                .store(false, Ordering::Release);
            return Err(TrackedRegionError::ForeignMark);
        }
        let abandoned = self
            .dependencies
            .get_mut()
            .expect("dependency runtime mutex is not poisoned")
            .abandon_region(mark.dependency);
        self.dependency_region_active
            .store(false, Ordering::Release);
        abandoned?;
        Ok(())
    }

    /// Marks one observable fact after its aggregate mutation barrier.
    pub fn mark_dependency_changed(&mut self, key: DependencyKey) -> ChangedAt {
        self.dependencies
            .get_mut()
            .expect("dependency runtime mutex is not poisoned")
            .mark_changed(key)
    }

    /// Registers one memo read for later changed-at invalidation.
    pub fn track_dependency(&self, key: DependencyKey) -> ChangedAt {
        self.dependencies
            .lock()
            .expect("dependency runtime mutex is not poisoned")
            .track(key)
    }

    /// Records one typed state read using its allocation-independent semantic
    /// value. Reads outside an active region are intentionally ignored.
    pub fn observe_semantic_dependency(&self, key: DependencyKey) {
        if self.dependency_projection_active.load(Ordering::Relaxed)
            || !self.dependency_region_is_active()
        {
            return;
        }
        let projection_guard = DependencyProjectionGuard::enter(&self.dependency_projection_active);
        let value = self.semantic_dependency_value(key);
        drop(projection_guard);
        self.track_dependency(key);
        if let Some(value) = value {
            self.record_dependency(key, value);
        } else {
            self.poison_tracked_region(TrackedRegionBarrier::UnsupportedExecutionState);
        }
    }

    #[inline(always)]
    fn observe_cell_dependency(&self, bank: BankTag, index: u32) {
        self.observe_semantic_dependency(DependencyKey::Cell(CellId::new(bank, index)));
    }

    #[inline(always)]
    fn observe_font_dependency(&self, font: FontId, field: DependencyFontField, index: u32) {
        self.observe_semantic_dependency(DependencyKey::Font {
            field,
            font: font.raw(),
            index,
        });
    }

    #[inline(always)]
    fn observe_pdf_dependency(&self, field: DependencyEngineField) {
        self.observe_semantic_dependency(DependencyKey::Engine(field));
    }

    #[inline(always)]
    fn mark_pdf_dependency_changed(&mut self, _field: DependencyEngineField) {
        // PDF ledger projections deliberately use one canonical full-state
        // identity and one shared mutation stamp for now, so a mutation cannot
        // leave a differently keyed full projection falsely green.
        self.dependencies
            .get_mut()
            .expect("dependency runtime mutex is not poisoned")
            .mark_changed(DependencyKey::Engine(DependencyEngineField::PdfObjects));
    }

    /// Returns the current changed-at stamp for validation.
    #[must_use]
    pub fn dependency_changed_at(&self, key: DependencyKey) -> ChangedAt {
        self.dependencies
            .lock()
            .expect("dependency runtime mutex is not poisoned")
            .tracker()
            .changed_at(key)
    }

    /// Validates a recorded region through the aggregate state boundary.
    /// Current semantic values are requested only for keys whose stamps moved.
    pub fn validate_dependencies(
        &self,
        observations: &mut [ObservedDependency],
        read_current: impl FnMut(DependencyKey) -> DependencyValue,
    ) -> bool {
        let tracker = self
            .dependencies
            .lock()
            .expect("dependency runtime mutex is not poisoned")
            .tracker()
            .clone();
        tracker.validate_region(observations, read_current)
    }

    /// Validates a recorded region and identifies its first changed dependency.
    pub fn validate_dependencies_with_failure(
        &self,
        observations: &mut [ObservedDependency],
        read_current: impl FnMut(DependencyKey) -> DependencyValue,
    ) -> Option<DependencyKey> {
        let tracker = self
            .dependencies
            .lock()
            .expect("dependency runtime mutex is not poisoned")
            .tracker()
            .clone();
        tracker.validate_region_failure(observations, read_current)
    }

    /// Validates immutable shared observations without backdating stamps.
    pub fn validate_dependencies_with_failure_readonly(
        &self,
        observations: &[ObservedDependency],
        read_current: impl FnMut(DependencyKey) -> DependencyValue,
    ) -> Option<DependencyKey> {
        let tracker = self
            .dependencies
            .lock()
            .expect("dependency runtime mutex is not poisoned")
            .tracker()
            .clone();
        tracker.validate_region_failure_readonly(observations, read_current)
    }

    /// Reads one state-owned dependency as an allocation-independent value.
    ///
    /// Executor/input-stack facts return `None`; their owning layer must add
    /// them to its explicit key or reject the containing memo episode.
    #[must_use]
    pub fn semantic_dependency_value(&self, key: DependencyKey) -> Option<DependencyValue> {
        const DOMAIN: u64 = 0x6465_7065_6e64_0001;
        let projection =
            |build: &mut dyn FnMut(&mut EngineBoundaryHasher<'_>)| DependencyValue::Projection {
                schema: 1,
                fingerprint: self.engine_boundary_hash(DOMAIN, |hash| build(hash)),
            };
        let token_list = |id| {
            let mut build = |hash: &mut EngineBoundaryHasher<'_>| hash.token_list(id);
            projection(&mut build)
        };
        let glue = |id| {
            let mut build = |hash: &mut EngineBoundaryHasher<'_>| hash.glue(id);
            projection(&mut build)
        };
        let font = |id| self.semantic_font_dependency_value(id);
        match key {
            DependencyKey::Cell(cell) => {
                let index = cell.index();
                if cell.bank() == BankTag::Meaning {
                    let symbol = self.stores.try_resolve_stored_symbol(Symbol::new(index))?;
                    let meaning = self.meaning(symbol);
                    let mut build = |hash: &mut EngineBoundaryHasher<'_>| hash.meaning(meaning);
                    return Some(projection(&mut build));
                }
                let index16 = u16::try_from(index).ok()?;
                Some(match cell.bank() {
                    BankTag::Count => DependencyValue::Integer(i64::from(self.count(index16))),
                    BankTag::Dimen => {
                        DependencyValue::Integer(i64::from(self.dimen(index16).raw()))
                    }
                    BankTag::Skip => glue(self.skip(index16)),
                    BankTag::Muskip => glue(self.muskip(index16)),
                    BankTag::Toks => token_list(self.toks(index16)),
                    BankTag::Box => {
                        self.box_reg_ref(index16)
                            .map_or(DependencyValue::Absent, |owner| {
                                let mut build = |hash: &mut EngineBoundaryHasher<'_>| {
                                    hash.node_list_ref(&owner);
                                };
                                projection(&mut build)
                            })
                    }
                    BankTag::IntParam => {
                        DependencyValue::Integer(i64::from(self.int_param(IntParam::new(index16))))
                    }
                    BankTag::DimenParam => DependencyValue::Integer(i64::from(
                        self.dimen_param(DimenParam::new(index16)).raw(),
                    )),
                    BankTag::GlueParam => glue(self.glue_param(GlueParam::new(index16))),
                    BankTag::TokParam => self
                        .tok_param_option(TokParam::new(index16))
                        .map_or(DependencyValue::Absent, token_list),
                    BankTag::CurrentFont => font(self.current_font()),
                    BankTag::MathFamilyFont => {
                        let family = u8::try_from(index % 16).ok()?;
                        let size = match index / 16 {
                            0 => MathFontSize::Text,
                            1 => MathFontSize::Script,
                            2 => MathFontSize::ScriptScript,
                            _ => return None,
                        };
                        font(self.math_family_font(size, family))
                    }
                    BankTag::Meaning
                    | BankTag::FontDimen
                    | BankTag::FontParamLen
                    | BankTag::FontHyphenChar
                    | BankTag::FontSkewChar
                    | BankTag::PdfLpCode
                    | BankTag::PdfRpCode
                    | BankTag::PdfEfCode
                    | BankTag::PdfTagCode
                    | BankTag::PdfKnbsCode
                    | BankTag::PdfStbsCode
                    | BankTag::PdfShbsCode
                    | BankTag::PdfKnbcCode
                    | BankTag::PdfKnacCode
                    | BankTag::PdfNoLigatures => return None,
                })
            }
            DependencyKey::Code { table, scalar } => {
                let ch = char::from_u32(scalar)?;
                Some(DependencyValue::Integer(match table {
                    DependencyCodeTable::Catcode => i64::from(self.catcode(ch) as u8),
                    DependencyCodeTable::Lccode => i64::from(self.lccode(ch)),
                    DependencyCodeTable::Uccode => i64::from(self.uccode(ch)),
                    DependencyCodeTable::Sfcode => i64::from(self.sfcode(ch)),
                    DependencyCodeTable::Mathcode => i64::from(self.mathcode(ch)),
                    DependencyCodeTable::Delcode => i64::from(self.delcode(ch)),
                }))
            }
            DependencyKey::CodeGeneration(table) => {
                let mut build = |hash: &mut EngineBoundaryHasher<'_>| hash.code_table(table);
                Some(projection(&mut build))
            }
            DependencyKey::Font {
                field,
                font: raw,
                index,
            } => {
                let id = self.stores.resolve_stored_font(FontId::new(raw));
                Some(match field {
                    DependencyFontField::Identifier => {
                        self.font_identifier_symbol(id)
                            .map_or(DependencyValue::Absent, |symbol| {
                                let name = self.resolve(symbol);
                                let mut build =
                                    |hash: &mut EngineBoundaryHasher<'_>| hash.str(name);
                                projection(&mut build)
                            })
                    }
                    DependencyFontField::Name => {
                        let name = self.font_name(id);
                        let mut build = |hash: &mut EngineBoundaryHasher<'_>| hash.str(&name);
                        projection(&mut build)
                    }
                    DependencyFontField::Parameter => {
                        DependencyValue::Integer(i64::from(self.font_parameter(id, index).raw()))
                    }
                    DependencyFontField::ParameterCount => {
                        DependencyValue::Unsigned(u64::from(self.font_parameter_count(id)))
                    }
                    DependencyFontField::Parameters => {
                        let mut build = |hash: &mut EngineBoundaryHasher<'_>| {
                            let count = self.font_parameter_count(id);
                            hash.u32(count);
                            for index in 1..=count {
                                hash.i32(self.font_parameter(id, index).raw());
                            }
                        };
                        projection(&mut build)
                    }
                    DependencyFontField::HyphenChar => {
                        DependencyValue::Integer(i64::from(self.font_hyphen_char(id)))
                    }
                    DependencyFontField::SkewChar => {
                        DependencyValue::Integer(i64::from(self.font_skew_char(id)))
                    }
                    DependencyFontField::Metrics => font(id),
                    DependencyFontField::PdfCode => {
                        let table = match index / 256 {
                            0 => crate::font::PdfFontCode::Lp,
                            1 => crate::font::PdfFontCode::Rp,
                            2 => crate::font::PdfFontCode::Ef,
                            3 => crate::font::PdfFontCode::Tag,
                            4 => crate::font::PdfFontCode::Knbs,
                            5 => crate::font::PdfFontCode::Stbs,
                            6 => crate::font::PdfFontCode::Shbs,
                            7 => crate::font::PdfFontCode::Knbc,
                            8 => crate::font::PdfFontCode::Knac,
                            _ => return None,
                        };
                        DependencyValue::Integer(i64::from(self.pdf_font_code(
                            table,
                            id,
                            (index % 256) as u8,
                        )))
                    }
                    DependencyFontField::PdfShaping => {
                        let mut build = |hash: &mut EngineBoundaryHasher<'_>| {
                            hash.bool(self.pdf_font_ligatures_disabled(id));
                            for code in 0..=u8::MAX {
                                hash.i32(self.pdf_font_code(
                                    crate::font::PdfFontCode::Tag,
                                    id,
                                    code,
                                ));
                            }
                        };
                        projection(&mut build)
                    }
                })
            }
            DependencyKey::InputStream(_) => world_backed_dependency_value(&self.world, key),
            DependencyKey::PageDimension(raw) => {
                let dimension = PageDimension::from_index(raw)?;
                Some(DependencyValue::Integer(i64::from(
                    self.page_dimension(dimension).raw(),
                )))
            }
            DependencyKey::PageInteger(raw) => {
                let integer = PageInteger::from_index(raw)?;
                Some(DependencyValue::Integer(i64::from(
                    self.page_integer(integer),
                )))
            }
            DependencyKey::PageMark(raw) => {
                let mark = match raw {
                    0 => PageMark::Top,
                    1 => PageMark::First,
                    2 => PageMark::Bot,
                    3 => PageMark::SplitFirst,
                    4 => PageMark::SplitBot,
                    _ => return None,
                };
                Some(
                    self.page_mark_value(mark)
                        .map_or(DependencyValue::Absent, token_list),
                )
            }
            DependencyKey::PageMarkClass { mark, class } => {
                let mark = match mark {
                    0 => PageMark::Top,
                    1 => PageMark::First,
                    2 => PageMark::Bot,
                    3 => PageMark::SplitFirst,
                    4 => PageMark::SplitBot,
                    _ => return None,
                };
                Some(
                    self.page_mark_class_value(mark, class)
                        .map_or(DependencyValue::Absent, token_list),
                )
            }
            DependencyKey::Engine(DependencyEngineField::GroupLevel) => Some(
                DependencyValue::Unsigned(u64::from(self.execution_group_depth())),
            ),
            DependencyKey::Engine(DependencyEngineField::GroupType) => {
                Some(DependencyValue::Integer(i64::from(
                    self.innermost_group_kind().map_or(0, GroupKind::etex_code),
                )))
            }
            DependencyKey::Engine(DependencyEngineField::ParShape) => {
                let shape = self.paragraph_shape();
                let mut build = |hash: &mut EngineBoundaryHasher<'_>| {
                    hash.usize(shape.len());
                    for line in &shape {
                        hash.i32(line.indent.raw());
                        hash.i32(line.width.raw());
                    }
                };
                Some(projection(&mut build))
            }
            DependencyKey::Engine(DependencyEngineField::PenaltyArrays) => {
                let mut build = |hash: &mut EngineBoundaryHasher<'_>| {
                    for kind in [
                        PenaltyArrayKind::InterLine,
                        PenaltyArrayKind::Club,
                        PenaltyArrayKind::Widow,
                        PenaltyArrayKind::DisplayWidow,
                    ] {
                        let values = self.penalty_array(kind);
                        hash.usize(values.len());
                        for value in values {
                            hash.i32(value);
                        }
                    }
                };
                Some(projection(&mut build))
            }
            DependencyKey::Engine(DependencyEngineField::InteractionMode) => Some(
                DependencyValue::Integer(i64::from(encode_interaction_mode(self.interaction_mode))),
            ),
            DependencyKey::Engine(DependencyEngineField::LastNodeType) => Some(
                DependencyValue::Integer(i64::from(self.page.last_node_type())),
            ),
            DependencyKey::Engine(DependencyEngineField::PdfTimer) => Some(
                DependencyValue::Integer(i64::from(self.world.pdf_elapsed_time())),
            ),
            DependencyKey::Engine(DependencyEngineField::PdfRandom) => {
                world_backed_dependency_value(&self.world, key)
            }
            DependencyKey::Engine(DependencyEngineField::PdfShellEscape) => Some(
                DependencyValue::Integer(i64::from(self.pdf_shell_escape_status())),
            ),
            DependencyKey::Engine(DependencyEngineField::PageInsertions) => {
                Some(DependencyValue::Projection {
                    schema: 1,
                    fingerprint: self.page_memo_fingerprint(),
                })
            }
            DependencyKey::Engine(
                DependencyEngineField::PdfExternalImages
                | DependencyEngineField::PdfObjects
                | DependencyEngineField::PdfPositions
                | DependencyEngineField::PdfForms
                | DependencyEngineField::PdfPages,
            ) => Some(DependencyValue::Projection {
                schema: 1,
                fingerprint: self.pdf.hash_fragment().fingerprint(),
            }),
            DependencyKey::HyphenationPatterns(language) => Some(DependencyValue::Projection {
                schema: 1,
                fingerprint: self.stores.hyphenation_dependency_fingerprint(language, 0),
            }),
            DependencyKey::HyphenationExceptions(language) => Some(DependencyValue::Projection {
                schema: 1,
                fingerprint: self.stores.hyphenation_dependency_fingerprint(language, 1),
            }),
            DependencyKey::HyphenationCodes(language) => Some(DependencyValue::Projection {
                schema: 1,
                fingerprint: self.stores.hyphenation_dependency_fingerprint(language, 2),
            }),
            DependencyKey::Page(_) => Some(DependencyValue::Projection {
                schema: 1,
                fingerprint: self.page_memo_fingerprint(),
            }),
            DependencyKey::World { .. } => world_backed_dependency_value(&self.world, key),
            DependencyKey::InputRecord(_)
            | DependencyKey::PhysicalLine { .. }
            | DependencyKey::InputLine
            | DependencyKey::InputStack
            | DependencyKey::Engine(_)
            | DependencyKey::Query { .. } => None,
        }
    }

    fn tracked_environment_cell_value(&self, cell: CellId) -> Option<DependencyValue> {
        if let Some(value) = self.semantic_dependency_value(DependencyKey::Cell(cell)) {
            return Some(value);
        }
        let word = self.stores.semantic_env_word(cell);
        Some(match cell.bank() {
            BankTag::FontParamLen => DependencyValue::Unsigned(word),
            BankTag::PdfNoLigatures => DependencyValue::Bool(word != 0),
            BankTag::FontDimen
            | BankTag::FontHyphenChar
            | BankTag::FontSkewChar
            | BankTag::PdfLpCode
            | BankTag::PdfRpCode
            | BankTag::PdfEfCode
            | BankTag::PdfTagCode
            | BankTag::PdfKnbsCode
            | BankTag::PdfStbsCode
            | BankTag::PdfShbsCode
            | BankTag::PdfKnbcCode
            | BankTag::PdfKnacCode => DependencyValue::Integer(i64::from(word as u32 as i32)),
            BankTag::Meaning
            | BankTag::Count
            | BankTag::Dimen
            | BankTag::Skip
            | BankTag::Toks
            | BankTag::Box
            | BankTag::IntParam
            | BankTag::DimenParam
            | BankTag::GlueParam
            | BankTag::TokParam
            | BankTag::Muskip
            | BankTag::CurrentFont
            | BankTag::MathFamilyFont => return None,
        })
    }

    /// Projects a selected font through the same semantic dependency domain
    /// used by [`Self::semantic_dependency_value`].
    #[doc(hidden)]
    #[must_use]
    pub fn semantic_font_dependency_value(&self, font: FontId) -> DependencyValue {
        const DOMAIN: u64 = 0x6465_7065_6e64_0001;
        DependencyValue::Projection {
            schema: 1,
            fingerprint: self.engine_boundary_hash(DOMAIN, |hash| hash.font(font)),
        }
    }

    /// Requests a bounded session-local pure-query cache from the next executor.
    pub fn enable_pure_memo(&mut self, config: crate::PureMemoConfig) {
        self.pure_memo_config = Some(config);
    }

    /// Enables detached page-builder episode reuse in the session cache.
    pub fn enable_page_memo(&mut self) {
        self.pure_memo_config
            .get_or_insert_with(crate::PureMemoConfig::default)
            .recording
            .pages = true;
    }

    /// Enables finalized effect-free shipout artifact reuse.
    pub fn enable_shipout_memo(&mut self) {
        self.pure_memo_config
            .get_or_insert_with(crate::PureMemoConfig::default)
            .recording
            .shipouts = true;
    }

    /// Clears a memo request that has not yet been consumed by an executor.
    pub fn disable_pure_memo(&mut self) {
        self.pure_memo_config = None;
    }

    /// Consumes the driver-requested memo configuration. Aggregate state
    /// never constructs or owns the corresponding runtime.
    #[doc(hidden)]
    pub fn take_pure_memo_config(&mut self) -> Option<crate::PureMemoConfig> {
        self.pure_memo_config.take()
    }

    /// Installs a borrow-only execution capability. The aggregate retains no
    /// cache values and cannot keep the session service alive.
    #[doc(hidden)]
    pub fn attach_pure_memo_capability(
        &mut self,
        runtime: &std::sync::Arc<std::sync::Mutex<crate::PureMemoRuntime>>,
    ) {
        self.pure_memo_capability = std::sync::Arc::downgrade(runtime);
    }

    /// Returns the currently attached execution capability, when any.
    #[doc(hidden)]
    #[must_use]
    pub fn pure_memo_capability(
        &self,
    ) -> Option<std::sync::Arc<std::sync::Mutex<crate::PureMemoRuntime>>> {
        self.pure_memo_capability.upgrade()
    }

    /// Borrows the execution-owned memo service without exposing ownership.
    #[doc(hidden)]
    pub fn with_pure_memo<R>(
        &self,
        operation: impl FnOnce(&mut crate::PureMemoRuntime) -> R,
    ) -> Option<R> {
        let capability = self.pure_memo_capability()?;
        let mut runtime = capability
            .lock()
            .expect("memo runtime mutex is not poisoned");
        Some(operation(&mut runtime))
    }

    fn mark_code_changed(&mut self, table: DependencyCodeTable, _ch: char) {
        self.dependencies
            .get_mut()
            .expect("dependency runtime mutex is not poisoned")
            .mark_changed(DependencyKey::CodeGeneration(table));
    }

    fn consume_env_mutation(&mut self, receipt: crate::env::CellMutationReceipt) {
        if !receipt.changed() {
            return;
        }
        self.stores.synchronize_exact_env_identity();
        let cell = receipt.cell();
        self.stores.update_main_memory_roots(receipt);
        let index = cell.index();
        match cell.bank() {
            BankTag::Meaning
            | BankTag::Count
            | BankTag::Dimen
            | BankTag::Skip
            | BankTag::Toks
            | BankTag::Box
            | BankTag::Muskip
            | BankTag::IntParam
            | BankTag::DimenParam
            | BankTag::GlueParam
            | BankTag::TokParam
            | BankTag::CurrentFont
            | BankTag::MathFamilyFont => {
                self.dependencies
                    .get_mut()
                    .expect("dependency runtime mutex is not poisoned")
                    .mark_changed(DependencyKey::Cell(cell));
            }
            BankTag::FontDimen => {
                let font = index >> 17;
                self.dependencies
                    .get_mut()
                    .expect("dependency runtime mutex is not poisoned")
                    .mark_changed(DependencyKey::Font {
                        field: DependencyFontField::Parameters,
                        font,
                        index: 0,
                    });
                self.dependencies
                    .get_mut()
                    .expect("dependency runtime mutex is not poisoned")
                    .mark_changed(DependencyKey::Font {
                        field: DependencyFontField::Parameter,
                        font,
                        index: (index & ((1 << 17) - 1)) + 1,
                    });
            }
            BankTag::FontParamLen => {
                self.dependencies
                    .get_mut()
                    .expect("dependency runtime mutex is not poisoned")
                    .mark_changed(DependencyKey::Font {
                        field: DependencyFontField::Parameters,
                        font: index,
                        index: 0,
                    });
                self.dependencies
                    .get_mut()
                    .expect("dependency runtime mutex is not poisoned")
                    .mark_changed(DependencyKey::Font {
                        field: DependencyFontField::ParameterCount,
                        font: index,
                        index: 0,
                    });
            }
            BankTag::FontHyphenChar | BankTag::FontSkewChar => {
                self.dependencies
                    .get_mut()
                    .expect("dependency runtime mutex is not poisoned")
                    .mark_changed(DependencyKey::Font {
                        field: if cell.bank() == BankTag::FontHyphenChar {
                            DependencyFontField::HyphenChar
                        } else {
                            DependencyFontField::SkewChar
                        },
                        font: index,
                        index: 0,
                    });
            }
            BankTag::PdfTagCode | BankTag::PdfNoLigatures => {
                self.dependencies
                    .get_mut()
                    .expect("dependency runtime mutex is not poisoned")
                    .mark_changed(DependencyKey::Font {
                        field: DependencyFontField::PdfShaping,
                        font: if cell.bank() == BankTag::PdfTagCode {
                            index >> 8
                        } else {
                            index
                        },
                        index: 0,
                    });
                if cell.bank() == BankTag::PdfTagCode {
                    self.dependencies
                        .get_mut()
                        .expect("dependency runtime mutex is not poisoned")
                        .mark_changed(DependencyKey::Font {
                            field: DependencyFontField::PdfCode,
                            font: index >> 8,
                            index: 3 * 256 + (index & 0xff),
                        });
                }
            }
            BankTag::PdfLpCode
            | BankTag::PdfRpCode
            | BankTag::PdfEfCode
            | BankTag::PdfKnbsCode
            | BankTag::PdfStbsCode
            | BankTag::PdfShbsCode
            | BankTag::PdfKnbcCode
            | BankTag::PdfKnacCode => {
                let table = match cell.bank() {
                    BankTag::PdfLpCode => 0,
                    BankTag::PdfRpCode => 1,
                    BankTag::PdfEfCode => 2,
                    BankTag::PdfKnbsCode => 4,
                    BankTag::PdfStbsCode => 5,
                    BankTag::PdfShbsCode => 6,
                    BankTag::PdfKnbcCode => 7,
                    BankTag::PdfKnacCode => 8,
                    _ => unreachable!("matched one exact PDF font-code bank"),
                };
                self.dependencies
                    .get_mut()
                    .expect("dependency runtime mutex is not poisoned")
                    .mark_changed(DependencyKey::Font {
                        field: DependencyFontField::PdfCode,
                        font: index >> 8,
                        index: table * 256 + (index & 0xff),
                    });
            }
        }
    }

    fn consume_env_mutations(
        &mut self,
        receipts: impl IntoIterator<Item = crate::env::CellMutationReceipt>,
    ) {
        for receipt in receipts {
            self.consume_env_mutation(receipt);
        }
    }

    /// Projects executor-owned roots into the same allocation-independent
    /// semantic hash vocabulary used by Universe checkpoints.
    #[must_use]
    pub fn engine_boundary_hash(
        &self,
        domain: u64,
        build: impl FnOnce(&mut EngineBoundaryHasher<'_>),
    ) -> u64 {
        let mut projection = EngineBoundaryHasher {
            stores: &self.stores,
            hasher: StateHasher::new_exact(domain),
            visits: 0,
        };
        #[cfg(feature = "profiling")]
        let started = World::start_profiling_timer();
        build(&mut projection);
        let fingerprint = projection.hasher.finish();
        #[cfg(feature = "profiling")]
        crate::measurement::record_state_hash_component(
            StateHashComponent::Mode,
            projection.visits,
            started.elapsed(),
        );
        fingerprint
    }

    /// Projects executor-owned roots into four domain-separated fingerprints
    /// while traversing the semantic input only once.
    #[must_use]
    pub fn engine_boundary_hashes(
        &self,
        domains: [u64; 4],
        build: impl FnOnce(&mut EngineBoundaryHasher<'_>),
    ) -> [u64; 4] {
        let mut projection = EngineBoundaryHasher {
            stores: &self.stores,
            hasher: StateHasher::new_quad(domains),
            visits: 0,
        };
        #[cfg(feature = "profiling")]
        let started = World::start_profiling_timer();
        build(&mut projection);
        let fingerprints = projection.hasher.finish_quad();
        #[cfg(feature = "profiling")]
        crate::measurement::record_state_hash_component(
            StateHashComponent::Mode,
            projection.visits,
            started.elapsed(),
        );
        fingerprints
    }

    /// Serializes the allocation-independent semantic engine state.
    ///
    /// Host effects, provenance, checkpoints, journals, caches, and input
    /// cursors are intentionally absent. The image is deterministic for one
    /// semantic state across the portable schema-11 frozen stores and its
    /// fixed node arena and portable frozen environment base.
    pub fn dump_format(&self) -> Result<Vec<u8>, FormatError> {
        if !self.input_summary.is_empty() {
            return Err(FormatError::NonEmptyInput);
        }
        // e-TeX deliberately does not dump its saved vertical-discard lists.
        if !self.page.is_format_empty() {
            return Err(FormatError::NonEmptyPage);
        }
        let pdf = self
            .pdf
            .capture_format(
                |tokens| {
                    self.detach_token_list(tokens)
                        .and_then(|value| value.to_bytes())
                        .map_err(|error| format!("{error:?}"))
                },
                |nodes| {
                    self.detach_node_list(nodes)
                        .and_then(|value| value.to_bytes())
                        .map_err(|error| format!("{error:?}"))
                },
            )
            .map_err(FormatError::InvalidState)?;
        let Some(pdf) = pdf else {
            return Err(FormatError::NonEmptyPdfDocument);
        };
        let mut stores = self.stores.clone();
        stores
            .mark_string_pool_format_baseline()
            .map_err(map_store_format_error)?;
        let string_pool = stores.string_pool_accounting();
        let stores = stores
            .encode_frozen_format()
            .map_err(map_store_format_error)?;
        let payload = bincode::serialize(&UniverseFormatPayload {
            interaction_mode: encode_interaction_mode(self.interaction_mode),
            pdf,
            string_pool,
        })
        .map_err(|error| FormatError::InvalidState(error.to_string()))?;
        crate::format_container::encode(&[
            crate::format_container::SectionInput {
                kind: crate::format_container::TRANSITIONAL_SEMANTIC_SECTION,
                alignment: 8,
                bytes: &payload,
            },
            crate::format_container::SectionInput {
                kind: crate::stores::NAMES_SECTION,
                alignment: 8,
                bytes: &stores.names,
            },
            crate::format_container::SectionInput {
                kind: crate::stores::NAMES_LOOKUP_SECTION,
                alignment: 8,
                bytes: &stores.names_lookup,
            },
            crate::format_container::SectionInput {
                kind: crate::stores::TOKEN_LISTS_SECTION,
                alignment: 8,
                bytes: &stores.token_lists,
            },
            crate::format_container::SectionInput {
                kind: crate::stores::MACROS_SECTION,
                alignment: 8,
                bytes: &stores.macros,
            },
            crate::format_container::SectionInput {
                kind: crate::stores::GLUE_SECTION,
                alignment: 8,
                bytes: &stores.glue,
            },
            crate::format_container::SectionInput {
                kind: crate::stores::FONTS_SECTION,
                alignment: 8,
                bytes: &stores.fonts,
            },
            crate::format_container::SectionInput {
                kind: crate::stores::CODE_TABLES_SECTION,
                alignment: 8,
                bytes: &stores.code_tables,
            },
            crate::format_container::SectionInput {
                kind: crate::stores::HYPHENATION_SECTION,
                alignment: 8,
                bytes: &stores.hyphenation,
            },
            crate::format_container::SectionInput {
                kind: crate::stores::FROZEN_NODES_SECTION,
                alignment: 8,
                bytes: &stores.nodes,
            },
            crate::format_container::SectionInput {
                kind: crate::stores::FROZEN_ENV_SECTION,
                alignment: 8,
                bytes: &stores.env,
            },
        ])
        .map_err(map_container_error)
    }

    /// Constructs a fresh timeline from a validated semantic format image.
    pub fn from_format(world: World, bytes: &[u8]) -> Result<Self, FormatError> {
        let container = crate::format_container::decode(bytes).map_err(map_container_error)?;
        if container.sections.len() != 11 {
            return Err(FormatError::InvalidState(
                "schema-11 core format requires exactly eleven sections".to_owned(),
            ));
        }
        let payload = container
            .section(crate::format_container::TRANSITIONAL_SEMANTIC_SECTION)
            .ok_or_else(|| {
                FormatError::InvalidState(
                    "schema-11 transition is missing its semantic section".to_owned(),
                )
            })?;
        let format: UniverseFormatPayload = bincode::deserialize(payload.bytes.as_ref())
            .map_err(|error| FormatError::InvalidState(error.to_string()))?;
        if !format.string_pool.has_current_profile() {
            return Err(FormatError::InvalidState(
                "unsupported string-pool accounting profile".to_owned(),
            ));
        }
        let mode = decode_interaction_mode(format.interaction_mode)?;
        let frozen = crate::stores::FrozenCoreSections {
            names: required_format_section(&container, crate::stores::NAMES_SECTION)?,
            names_lookup: required_format_section(&container, crate::stores::NAMES_LOOKUP_SECTION)?,
            token_lists: required_format_section(&container, crate::stores::TOKEN_LISTS_SECTION)?,
            macros: required_format_section(&container, crate::stores::MACROS_SECTION)?,
            glue: required_format_section(&container, crate::stores::GLUE_SECTION)?,
            checksum: container.checksum,
        };
        let non_node = crate::stores::FrozenNonNodeSections {
            fonts: required_format_section(&container, crate::stores::FONTS_SECTION)?,
            code_tables: required_format_section(&container, crate::stores::CODE_TABLES_SECTION)?,
            hyphenation: required_format_section(&container, crate::stores::HYPHENATION_SECTION)?,
        };
        let nodes = crate::stores::FrozenNodeSection {
            bytes: required_format_section(&container, crate::stores::FROZEN_NODES_SECTION)?,
        };
        let environment = required_format_section(&container, crate::stores::FROZEN_ENV_SECTION)?;
        let mut stores = Stores::decode_frozen_format(environment, frozen, non_node, nodes)
            .map_err(map_store_format_error)?;
        stores.restore_string_pool_accounting(format.string_pool);
        let clock = world.job_clock();
        install_job_clock_params(
            &mut |param, value| {
                let _ = stores.set_int_param(param, value);
            },
            clock,
        );
        let input_summary = InputSummary::default();
        let page = PageBuilderState::default();
        let pdf_format = format.pdf;
        let pdf = PdfState::default();
        let input_fragment = hash_input_summary_fragment(&stores, &world, &input_summary);
        let state_hash_base = StateHashBase {
            store: stores.state_hash_cursor(),
            world: world.state_hash_cursor(),
            input_summary: input_summary.semantic_root(),
            input_fragment,
            interaction_mode: mode,
            page: page.state_hash_cursor(),
            pdf: pdf.cursor(),
            checkpoint_hash: container.checksum,
        };
        let mut universe = Self {
            owner: UniverseOwner::new(),
            private_revision_domain: None,
            stores,
            provenance_demand: ProvenanceDemand::default(),
            provenance_budgets: ProvenanceBudgets::default(),
            world,
            interaction_mode: mode,
            error_context_widths: crate::print::ErrorContextWidths::default(),
            input_summary,
            pending_every_job: true,
            editor_content_hash: None,
            page,
            pdf,
            primitive_meanings: HashMap::new(),
            primitive_meanings_by_index: Vec::new(),
            primitive_names_by_index: Vec::new(),
            primitive_indices: HashMap::new(),
            state_hash_base,
            state_hash_projection_cache: StateHashProjectionCache::default(),
            next_snapshot_serial: 0,
            fork_origin: None,
            dependencies: Mutex::new(DependencyRuntime::default()),
            dependency_region_active: AtomicBool::new(false),
            dependency_projection_active: AtomicBool::new(false),
            pure_memo_config: None,
            pure_memo_capability: std::sync::Weak::new(),
            geometry_observations: Vec::new(),
            geometry_observation_enabled: false,
            diagnostic_position: DiagnosticPosition::default(),
        };
        let pdf = {
            let cell = std::cell::RefCell::new(&mut universe);
            PdfState::restore_format(
                pdf_format,
                |bytes| {
                    let value = crate::DetachedMemoValue::from_bytes(
                        bytes,
                        crate::MemoValueLimits::default(),
                    )
                    .map_err(|error| format!("{error:?}"))?;
                    let mut universe = cell.borrow_mut();
                    let tokens = universe
                        .import_memo_token_list(&value, crate::MemoValueLimits::default())
                        .map_err(|error| format!("{error:?}"))?;
                    let semantic_id = universe.stores.token_list_semantic_fragment(tokens.id());
                    Ok(PdfTokenParameter {
                        tokens,
                        semantic_id,
                    })
                },
                |bytes| {
                    let value = crate::DetachedMemoValue::from_bytes(
                        bytes,
                        crate::MemoValueLimits::default(),
                    )
                    .map_err(|error| format!("{error:?}"))?;
                    let mut universe = cell.borrow_mut();
                    let nodes = universe
                        .import_memo_node_list(&value, crate::MemoValueLimits::default())
                        .map_err(|error| format!("{error:?}"))?;
                    let semantic = nodes.semantic_id().fragment();
                    Ok((nodes, semantic))
                },
            )
            .map_err(FormatError::InvalidState)?
        };
        universe.pdf = pdf;
        universe.state_hash_base.pdf = universe.pdf.cursor();
        // Format-carried PDF token parameters may upgrade an existing named
        // control sequence to its inaccessible internal namespace while they
        // are imported. Build exact Env identity after that canonicalization
        // so the initial loaded timeline matches a freshly constructed one.
        universe.stores.initialize_exact_env_identity();
        universe.stores.discard_exact_env_undo_history();
        Ok(universe)
    }

    /// Takes an O(1) snapshot of the whole timeline tuple.
    #[must_use]
    pub fn snapshot(&mut self) -> Snapshot {
        self.checkpoint_from_hash_base(self.state_hash_base.clone(), false)
    }

    /// Captures the strong optional identities used only by incremental suffix
    /// adoption. Ordinary rollback snapshots must remain O(1).
    #[doc(hidden)]
    #[must_use]
    pub fn snapshot_with_exact_identity(&mut self) -> Snapshot {
        self.checkpoint_from_hash_base(self.state_hash_base.clone(), true)
    }

    /// Returns whether `snapshot` still names a rollback point on this
    /// Universe's live timeline.
    ///
    /// Leaving the group that enclosed a checkpoint irreversibly consumes
    /// that save-stack level (tex.web §283), so aggregate operation drivers
    /// must commit rather than attempt to roll back after such an exit.
    #[must_use]
    pub fn can_rollback_to(&self, snapshot: &Snapshot) -> bool {
        snapshot.owner == self.owner.snapshot_owner()
            && self.stores.can_restore_snapshot(&snapshot.store)
            && self.world.snapshot_is_retained(&snapshot.world)
    }

    /// Installs one fresh allocation owner for a private revision.
    ///
    /// This is an engine/session lifecycle hook, not a store or host
    /// capability. Templates and accepted generations must not carry it.
    #[doc(hidden)]
    pub fn begin_private_revision(&mut self) {
        assert!(
            self.private_revision_domain.is_none(),
            "Universe already owns a private revision allocation domain"
        );
        self.private_revision_domain = Some(PatchAllocationDomain::new());
    }

    /// Closes an accepted private revision after its typed owners have
    /// transferred every explicit root.
    fn accept_private_revision(&mut self) -> Result<(), PrivateRevisionAcceptanceError> {
        let Some(domain) = self.private_revision_domain.as_ref() else {
            return Ok(());
        };
        let stats = domain.stats();
        if stats.operation_active {
            return Err(PrivateRevisionAcceptanceError::ActiveOperation);
        }
        if stats.allocations != self.stores.patch_allocation_count() {
            return Err(PrivateRevisionAcceptanceError::UnrootedAllocations);
        }
        let roots = self.stores.selected_patch_roots(domain);
        let domain = self
            .private_revision_domain
            .take()
            .expect("private domain was validated above");
        let accepted = domain
            .accept(roots)
            .expect("typed token roots were validated against the private domain");
        debug_assert!(accepted.len() <= stats.allocations);
        self.stores.clear_patch_allocations();
        drop(accepted);
        Ok(())
    }

    /// Commits an ordinary successful executor operation without creating an
    /// aggregate rollback snapshot.
    #[must_use]
    #[doc(hidden)]
    pub const fn direct_operation_supported(&self) -> bool {
        true
    }

    /// Opens an ordinary operation after capability preflight has established
    /// that it needs no rollback mark.
    #[doc(hidden)]
    #[must_use]
    pub fn begin_direct_operation(&mut self) -> DirectOperationMark {
        let (patch_operation, patch_store) =
            self.private_revision_domain
                .as_mut()
                .map_or((None, None), |domain| {
                    let operation = domain
                        .begin_operation()
                        .expect("private revision owns one direct operation mark");
                    (Some(operation), Some(self.stores.begin_patch_operation()))
                });
        let store = self.stores.begin_direct_operation();
        DirectOperationMark {
            store,
            patch_operation,
            patch_store,
        }
    }

    /// Commits an ordinary successful executor operation without creating an
    /// aggregate rollback snapshot.
    #[doc(hidden)]
    pub fn commit_direct_operation(&mut self, mark: DirectOperationMark) {
        let DirectOperationMark {
            store,
            patch_operation,
            patch_store,
        } = mark;
        match (
            &mut self.private_revision_domain,
            patch_operation,
            patch_store,
        ) {
            (Some(domain), Some(mark), Some(_)) => domain
                .commit_operation(mark)
                .expect("direct operation owns the active patch allocation mark"),
            (None, None, None) => {}
            _ => panic!("direct operation mark does not match its Universe"),
        }
        self.finish_direct_operation(store);
    }

    /// Discards private-revision allocations from a failed direct operation.
    /// Canonical partial semantic state is retained.
    #[doc(hidden)]
    pub fn discard_direct_operation_allocations(&mut self, mark: DirectOperationMark) {
        let DirectOperationMark {
            store,
            patch_operation,
            patch_store,
        } = mark;
        match (
            &mut self.private_revision_domain,
            patch_operation,
            patch_store,
        ) {
            (Some(domain), Some(mark), Some(store_mark)) => {
                self.stores.discard_patch_operation_allocations(store_mark);
                domain
                    .rollback_operation(mark)
                    .expect("direct operation owns the active patch allocation mark");
            }
            (None, None, None) => {}
            _ => panic!("direct operation mark does not match its Universe"),
        }
        self.finish_direct_operation(store);
    }

    fn finish_direct_operation(&mut self, store: DirectStoreOperationMark) {
        // A generation fork may later retarget every retained prefix record at
        // or before its anchor onto this timeline. `fork_origin` is that live
        // restoration authority even before those checkpoint values are
        // rehomed, so its shared prefix cannot become a fresh baseline here.
        if self.fork_origin.is_none() && !self.dependency_region_is_active() {
            if self
                .stores
                .commit_direct_operation(store, &self.state_hash_base.store)
            {
                self.retarget_hash_base_after_group_compaction();
            }
        } else {
            self.stores.finish_node_operation();
        }
    }

    #[must_use]
    pub fn freeze_generation(self) -> GenerationSubstrate {
        GenerationSubstrate::new(self)
    }

    fn validate_fork_snapshot(&self, snapshot: &Snapshot) -> Result<(), GenerationForkError> {
        if snapshot.owner != self.owner.snapshot_owner() {
            return Err(GenerationForkError::ForeignSnapshot);
        }
        if !self.stores.can_restore_snapshot(&snapshot.store)
            || !self.world.snapshot_is_forkable(&snapshot.world)
        {
            return Err(GenerationForkError::InvalidatedSnapshot);
        }
        Ok(())
    }

    fn retarget_inherited_snapshot(&self, snapshot: &Snapshot) -> Snapshot {
        let mut retargeted = snapshot.clone();
        retargeted.owner = self.owner.snapshot_owner();
        retargeted.store = self.stores.retarget_inherited_snapshot(&snapshot.store);
        retargeted.state_hash_base.store = self
            .stores
            .retarget_state_hash_cursor(&snapshot.state_hash_base.store);
        retargeted
    }

    fn capture_scoped_rollback(&mut self) -> ScopedRollback {
        ScopedRollback {
            owner: self.owner.snapshot_owner(),
            store: self.stores.checkpoint(),
            world: self.world.snapshot(),
            input_summary: self.input_summary.clone(),
            interaction_mode: self.interaction_mode,
            page: self.page.clone(),
            pdf: self.pdf.snapshot(),
            state_hash_base: self.state_hash_base.clone(),
            state_hash_projection_cache: self.state_hash_projection_cache.clone(),
            dependency_tracker: self
                .dependencies
                .lock()
                .expect("dependency runtime mutex is not poisoned")
                .snapshot_tracker(),
            geometry_observations_len: self.geometry_observations.len(),
        }
    }

    fn rollback_scoped(&mut self, rollback: ScopedRollback) {
        assert_eq!(
            rollback.owner,
            self.owner.snapshot_owner(),
            "scoped rollback belongs to a different Universe instance"
        );
        self.world.assert_snapshot_retained(&rollback.world);
        let receipts = self.stores.rollback(&rollback.store);
        self.consume_env_mutations(receipts);
        self.world.rollback(&rollback.world);
        self.input_summary = rollback.input_summary;
        self.interaction_mode = rollback.interaction_mode;
        self.page = rollback.page;
        self.pdf.rollback(rollback.pdf);
        self.state_hash_base = rollback.state_hash_base;
        self.state_hash_projection_cache = rollback.state_hash_projection_cache;
        self.dependencies
            .get_mut()
            .expect("dependency runtime mutex is not poisoned")
            .restore_tracker(&rollback.dependency_tracker);
        self.geometry_observations
            .truncate(rollback.geometry_observations_len);
    }

    fn checkpoint_from_hash_base(&mut self, hash_base: StateHashBase, exact: bool) -> Snapshot {
        let world = self.world.snapshot();
        let mut store = self.stores.checkpoint();
        let store_cursor = self.stores.state_hash_cursor_from_snapshot(&store);
        let world_cursor = World::state_hash_cursor_from_snapshot(&world);
        let input_cursor = self.input_summary.semantic_root();
        let input_fragment = if hash_base.input_summary == input_cursor {
            hash_base.input_fragment
        } else {
            let mut cache = std::mem::take(&mut self.state_hash_projection_cache);
            let fragment = self.hash_input_summary(&mut cache);
            self.state_hash_projection_cache = cache;
            fragment
        };
        let page_cursor = self.page.state_hash_cursor();
        let pdf_cursor = self.pdf.cursor();
        let state_hash = if hash_base.store == store_cursor
            && hash_base.world == world_cursor
            && hash_base.input_fragment == input_fragment
            && hash_base.interaction_mode == self.interaction_mode
            && hash_base.page == page_cursor
            && hash_base.pdf == pdf_cursor
        {
            hash_base.checkpoint_hash
        } else {
            let slice_hash = self.state_hash_slice(&hash_base, &mut store, input_fragment);
            combine(hash_base.checkpoint_hash, slice_hash)
        };
        let next_hash_base = StateHashBase {
            store: store_cursor,
            world: world_cursor,
            input_summary: input_cursor,
            input_fragment,
            interaction_mode: self.interaction_mode,
            page: page_cursor,
            pdf: pdf_cursor,
            checkpoint_hash: state_hash,
        };
        self.state_hash_base = next_hash_base.clone();
        let serial = self.next_snapshot_serial;
        self.next_snapshot_serial = self
            .next_snapshot_serial
            .checked_add(1)
            .expect("Universe snapshot serial exhausted");
        let exact_state_identity = exact
            .then(|| self.exact_checkpoint_identity().ok())
            .flatten();
        Snapshot {
            owner: self.owner.snapshot_owner(),
            serial,
            epoch: store.epoch(),
            store,
            world,
            input_summary: self.input_summary.clone(),
            interaction_mode: self.interaction_mode,
            page: self.page.clone(),
            pdf: self.pdf.snapshot(),
            exact_state_identity,
            state_hash_projection_cache: self.state_hash_projection_cache.clone(),
            dependency_tracker: self
                .dependencies
                .lock()
                .expect("dependency runtime mutex is not poisoned")
                .snapshot_tracker(),
            state_hash,
            state_hash_base: next_hash_base,
            geometry_observations_len: self.geometry_observations.len(),
        }
    }

    fn retarget_hash_base_after_committed_boundary(
        &self,
        hash_base: StateHashBase,
    ) -> StateHashBase {
        StateHashBase {
            store: self
                .stores
                .retarget_state_hash_cursor_after_node_release(&hash_base.store),
            world: self
                .world
                .retarget_state_hash_cursor_after_commit(&hash_base.world),
            input_summary: hash_base.input_summary,
            input_fragment: hash_base.input_fragment,
            interaction_mode: hash_base.interaction_mode,
            page: hash_base.page,
            pdf: hash_base.pdf,
            checkpoint_hash: hash_base.checkpoint_hash,
        }
    }

    fn retarget_hash_base_after_group_compaction(&mut self) {
        self.state_hash_base.store = self
            .stores
            .retarget_state_hash_cursor_after_journal_compaction(&self.state_hash_base.store);
    }

    /// Rolls the whole timeline back to `snapshot` atomically.
    pub fn rollback(&mut self, snapshot: &Snapshot) {
        self.assert_valid_snapshot(snapshot);
        self.world.assert_snapshot_retained(&snapshot.world);
        let receipts = self.stores.rollback(&snapshot.store);
        self.consume_env_mutations(receipts);
        self.world.rollback(&snapshot.world);
        self.input_summary = snapshot.input_summary.clone();
        self.interaction_mode = snapshot.interaction_mode;
        self.page = snapshot.page.clone();
        self.pdf.rollback(snapshot.pdf.clone());
        self.state_hash_base = snapshot.state_hash_base.clone();
        self.state_hash_projection_cache = snapshot.state_hash_projection_cache.clone();
        self.dependencies
            .get_mut()
            .expect("dependency runtime mutex is not poisoned")
            .restore_tracker(&snapshot.dependency_tracker);
        self.geometry_observations
            .truncate(snapshot.geometry_observations_len);
    }

    fn rollback_generation_fork(&mut self, snapshot: &Snapshot) {
        self.assert_valid_snapshot(snapshot);
        assert!(
            self.world.snapshot_is_forkable(&snapshot.world),
            "World snapshot effect root is not a valid generation fork"
        );
        let receipts = self.stores.rollback(&snapshot.store);
        self.consume_env_mutations(receipts);
        self.world.rollback_generation_fork(&snapshot.world);
        self.input_summary = snapshot.input_summary.clone();
        self.interaction_mode = snapshot.interaction_mode;
        self.page = snapshot.page.clone();
        self.pdf.rollback(snapshot.pdf.clone());
        self.state_hash_base = snapshot.state_hash_base.clone();
        self.state_hash_projection_cache = snapshot.state_hash_projection_cache.clone();
        self.dependencies
            .get_mut()
            .expect("dependency runtime mutex is not poisoned")
            .restore_tracker(&snapshot.dependency_tracker);
        self.geometry_observations
            .truncate(snapshot.geometry_observations_len);
    }

    pub fn enable_geometry_observation(&mut self) {
        self.geometry_observation_enabled = true;
    }

    pub fn geometry_observations_since(&self, start: usize) -> &[GeometryObservation] {
        &self.geometry_observations[start..]
    }

    #[must_use]
    pub const fn geometry_observation_len(&self) -> usize {
        self.geometry_observations.len()
    }

    pub fn record_geometry_observation(&mut self, observation: GeometryObservation) {
        if self.geometry_observation_enabled {
            self.geometry_observations.push(observation);
        }
    }

    fn state_hash_slice(
        &mut self,
        hash_base: &StateHashBase,
        store: &mut StoreSnapshot,
        input: StateHashFragment,
    ) -> u64 {
        let store = self.stores.state_hash_slice(&hash_base.store, store);
        let mut cache = std::mem::take(&mut self.state_hash_projection_cache);
        let world = self.hash_world_state_slice(&hash_base.world, &mut cache);
        let interaction = StateHashFragment::from_measured_builder(
            INTERACTION_PROJECTION_DOMAIN,
            StateHashComponent::Interaction,
            1,
            |projection| {
                hash_interaction_mode(self.interaction_mode, projection);
            },
        );
        let page = self.hash_page_state(&mut cache.page);
        let pdf = self.pdf.hash_fragment();
        self.state_hash_projection_cache = cache;

        let mut hasher = StateHasher::new_exact(UNIVERSE_SLICE_DOMAIN);
        hasher.u32(crate::CHECKPOINT_STATE_HASH_SCHEMA_VERSION);
        hasher.u64(store);
        world.apply(&mut hasher);
        input.apply(&mut hasher);
        interaction.apply(&mut hasher);
        page.apply(&mut hasher);
        pdf.apply(&mut hasher);
        hasher.finish()
    }

    fn hash_world_state_slice(
        &self,
        cursor: &WorldStateHashCursor,
        cache: &mut StateHashProjectionCache,
    ) -> StateHashFragment {
        let stream_root = self.world.stream_bufs_root();
        let streams = cache
            .world_streams
            .as_ref()
            .and_then(|cached| cached.fragment_if(|root| Arc::ptr_eq(root, &stream_root)))
            .unwrap_or_else(|| {
                let fragment = StateHashFragment::from_measured_builder(
                    WORLD_STREAMS_DOMAIN,
                    StateHashComponent::WorldStreams,
                    crate::world::STREAM_SLOT_COUNT,
                    |projection| {
                        hash_stream_bufs(&stream_root, projection);
                    },
                );
                cache.world_streams = Some(CachedProjection::new(stream_root, fragment));
                fragment
            });
        let effects = self.world.effect_records_since(cursor);
        let effects = StateHashFragment::from_measured_builder(
            WORLD_EFFECTS_DOMAIN,
            StateHashComponent::WorldEffects,
            effects.len(),
            |projection| {
                projection.tag(0x80);
                projection.usize(effects.len());
                for effect in effects {
                    self.hash_effect_record(effect, projection);
                }
            },
        );
        let shell_escapes = self.world.shell_escape_records_since(cursor);
        let shell_escapes = StateHashFragment::from_measured_builder(
            WORLD_SHELL_ESCAPES_DOMAIN,
            StateHashComponent::WorldShellEscapes,
            shell_escapes.len(),
            |projection| {
                projection.tag(0x82);
                projection.usize(shell_escapes.len());
                for record in shell_escapes {
                    hash_shell_escape_record(record, projection);
                }
            },
        );
        let scalars = StateHashFragment::from_measured_builder(
            WORLD_SCALARS_DOMAIN,
            StateHashComponent::WorldScalars,
            5,
            |projection| {
                hash_rng_state(self.world.rng_state(), projection);
                hash_pdf_random_state(&self.world, projection);
                hash_pdf_timer_state(&self.world, projection);
                hash_job_clock(self.world.job_clock(), projection);
                hash_shell_escape_policy(self.world.shell_escape_policy(), projection);
            },
        );
        StateHashFragment::from_exact_builder(WORLD_SLICE_DOMAIN, |projection| {
            effects.apply(projection);
            projection.tag(0x81);
            // Input records are content-addressed provenance allocations. Live
            // input frames hash the stable record content below; unreferenced
            // reads must not make semantic convergence allocation-sensitive.
            projection.usize(0);

            shell_escapes.apply(projection);
            streams.apply(projection);
            scalars.apply(projection);
        })
    }

    fn hash_exact_world_state(&self, cache: &mut StateHashProjectionCache) -> StateHashFragment {
        let stream_root = self.world.stream_bufs_root();
        let streams = cache
            .world_streams
            .as_ref()
            .and_then(|cached| cached.fragment_if(|root| Arc::ptr_eq(root, &stream_root)))
            .unwrap_or_else(|| {
                let fragment = StateHashFragment::from_measured_builder(
                    WORLD_STREAMS_DOMAIN,
                    StateHashComponent::WorldStreams,
                    crate::world::STREAM_SLOT_COUNT,
                    |projection| hash_stream_bufs(&stream_root, projection),
                );
                cache.world_streams = Some(CachedProjection::new(stream_root, fragment));
                fragment
            });
        let scalars = StateHashFragment::from_exact_builder(WORLD_SCALARS_DOMAIN, |projection| {
            hash_rng_state(self.world.rng_state(), projection);
            hash_pdf_random_state(&self.world, projection);
            hash_pdf_timer_state(&self.world, projection);
            hash_job_clock(self.world.job_clock(), projection);
            hash_shell_escape_policy(self.world.shell_escape_policy(), projection);
            projection.u8(match self.world.commit_mode() {
                WorldCommitMode::Eager => 0,
                WorldCommitMode::Retained => 1,
                WorldCommitMode::Exported => 2,
            });
        });
        StateHashFragment::from_exact_builder(WORLD_SLICE_DOMAIN ^ 0x6578_6163_7400, |projection| {
            streams.apply(projection);
            scalars.apply(projection);
        })
    }

    fn hash_effect_record(&self, record: &EffectRecord, hasher: &mut StateHasher) {
        match record {
            EffectRecord::StreamOpen { slot, target } => {
                hasher.tag(0);
                hash_stream_slot(*slot, hasher);
                hash_path(target.path(), hasher);
            }
            EffectRecord::StreamClose { slot } => {
                hasher.tag(1);
                hash_stream_slot(*slot, hasher);
            }
            EffectRecord::StreamWrite { sink, text } => {
                hasher.tag(2);
                hash_print_sink(*sink, hasher);
                hasher.str(text);
            }
            EffectRecord::StreamWriteBytes { sink, bytes } => {
                hasher.tag(7);
                hash_print_sink(*sink, hasher);
                hasher.bytes(bytes);
            }
            EffectRecord::DeferredWrite { stream, tokens } => {
                hasher.tag(3);
                hash_stream_slot(*stream, hasher);
                self.stores.hash_token_list_semantic(tokens.id(), hasher);
            }
            EffectRecord::Special { class, payload } => {
                hasher.tag(4);
                hasher.str(class);
                hasher.bytes(payload);
            }
            EffectRecord::PdfObjectPlaceholder { label } => {
                hasher.tag(5);
                hasher.str(label);
            }
            EffectRecord::ShellEscape(record) => {
                hasher.tag(6);
                hash_shell_escape_record(record, hasher);
            }
        }
    }

    fn hash_input_summary(&self, cache: &mut StateHashProjectionCache) -> StateHashFragment {
        let cursor = self.input_summary.semantic_root();
        if let Some(fragment) = cache
            .input
            .as_ref()
            .and_then(|cached| cached.fragment_if(|root| root == &cursor))
        {
            return fragment;
        }
        let fragment = hash_input_summary_fragment(&self.stores, &self.world, &self.input_summary);
        #[cfg(test)]
        {
            cache.input_hash_calls += 1;
        }
        cache.input = Some(CachedProjection::new(cursor, fragment));
        fragment
    }

    fn hash_page_state(&self, cache: &mut PageHashCache) -> StateHashFragment {
        StateHashFragment::from_exact_builder(0x7061_6765_5f62_6e64, |projection| {
            self.page.hash_semantic(
                projection,
                cache,
                |nodes, hasher| self.stores.hash_node_deque_semantic(nodes, hasher),
                |nodes, hasher| self.stores.hash_node_slice_semantic(nodes, hasher),
                |id, hasher| self.stores.hash_glue_semantic(id, hasher),
                |id, hasher| self.stores.hash_token_list_semantic(id, hasher),
            );
        })
    }

    fn exact_checkpoint_identity(&mut self) -> Result<u64, StoreFormatError> {
        #[cfg(feature = "profiling")]
        let started = World::start_profiling_timer();
        #[cfg(feature = "profiling")]
        let projections_before = crate::measurement::state_hash_measurement();
        let store = self.stores.semantic_identity()?;
        let mut cache = std::mem::take(&mut self.state_hash_projection_cache);
        let input = self.hash_input_summary(&mut cache);
        let world = self.hash_exact_world_state(&mut cache);
        let interaction =
            StateHashFragment::from_exact_builder(INTERACTION_PROJECTION_DOMAIN, |projection| {
                hash_interaction_mode(self.interaction_mode, projection)
            });
        let page = self.hash_page_state(&mut cache.page);
        let pdf = self.pdf.hash_fragment();
        self.state_hash_projection_cache = cache;

        let mut framed = Vec::with_capacity(192);
        framed.extend_from_slice(b"umber-exact-checkpoint-v4");
        framed.extend_from_slice(&store.to_le_bytes());
        for component in [input, world, interaction, page, pdf] {
            framed.extend_from_slice(&component.exact_identity().to_le_bytes());
        }
        let identity =
            crate::state_hash::exact_identity_bytes(b"umber-exact-checkpoint-v5", &framed);
        #[cfg(feature = "profiling")]
        {
            let projections_after = crate::measurement::state_hash_measurement();
            let mut calls = 0;
            let mut visits = 0;
            let mut nanos = 0;
            for (before, after) in projections_before
                .components
                .iter()
                .zip(projections_after.components)
            {
                calls += after.calls.saturating_sub(before.calls);
                visits += after.visits.saturating_sub(before.visits);
                nanos += after.nanos.saturating_sub(before.nanos);
            }
            crate::measurement::record_exact_identity(started.elapsed(), calls, visits, nanos);
        }
        Ok(identity)
    }

    /// Canonical allocation-independent identity of the complete live page root.
    #[doc(hidden)]
    #[must_use]
    pub fn page_memo_fingerprint(&self) -> u64 {
        self.hash_page_state(&mut PageHashCache::default())
            .fingerprint()
    }

    fn assert_valid_snapshot(&self, snapshot: &Snapshot) {
        assert_eq!(
            snapshot.owner,
            self.owner.snapshot_owner(),
            "Universe snapshot belongs to a different Universe instance"
        );
    }

    /// Reads the owned environment for crate-local replay oracles.
    #[must_use]
    #[allow(dead_code)]
    #[cfg(test)]
    pub(crate) fn env(&self) -> &Env {
        self.stores.env()
    }

    /// Reads the external-effect capability object.
    #[must_use]
    pub const fn world(&self) -> &World {
        &self.world
    }

    /// Takes one ErrorStop input mutation without opening the broad World
    /// mutation guard.
    ///
    /// The error channel is diagnostic control state, not a memo-observable
    /// [`DependencyWorldField`]. Canonical raw delivery polls this on every
    /// token, so routing the empty fast path through [`Self::world_mut`]
    /// would repeatedly collect and compare every tracked World-backed key.
    pub(crate) fn take_error_recovery_request(
        &mut self,
    ) -> Option<crate::print::ErrorRecoveryRequest> {
        self.world.error_channel_mut().take_recovery_request()
    }

    #[cfg(test)]
    pub(crate) fn testing_tracked_world_scan_calls(&self) -> usize {
        self.dependencies
            .lock()
            .expect("dependency runtime mutex is not poisoned")
            .tracked_world_scan_calls()
    }

    /// Reads one virtual output-stream state through the dependency boundary.
    #[must_use]
    pub fn output_stream_is_open(&self, stream: StreamSlot) -> bool {
        self.observe_semantic_dependency(DependencyKey::World {
            field: DependencyWorldField::OutputStream,
            index: u64::from(stream.raw()),
        });
        self.world.write_stream_is_open(stream)
    }

    /// Applies one virtual output-stream open through the dependency boundary.
    pub fn open_output_stream(&mut self, stream: StreamSlot, target: String) {
        self.world.open_out(stream, target);
        self.dependencies
            .get_mut()
            .expect("dependency runtime mutex is not poisoned")
            .mark_changed(DependencyKey::World {
                field: DependencyWorldField::OutputStream,
                index: u64::from(stream.raw()),
            });
    }

    /// Applies one virtual output-stream close through the dependency boundary.
    pub fn close_output_stream(&mut self, stream: StreamSlot) {
        self.world.close_out(stream);
        self.dependencies
            .get_mut()
            .expect("dependency runtime mutex is not poisoned")
            .mark_changed(DependencyKey::World {
                field: DependencyWorldField::OutputStream,
                index: u64::from(stream.raw()),
            });
    }

    /// Mutates the external-effect capability object through the Universe boundary.
    pub fn world_mut(&mut self) -> WorldMut<'_> {
        // This intentionally broad escape hatch is retained for top-level
        // drivers. Only facts already capable of validating a memo need to be
        // compared; capability-specific paths below remain the fast path.
        let dependencies = self
            .dependencies
            .get_mut()
            .expect("dependency runtime mutex is not poisoned");
        let before = dependencies
            .tracked_world_backed_keys()
            .into_iter()
            .filter_map(|key| {
                world_backed_dependency_value(&self.world, key).map(|value| (key, value))
            })
            .collect();
        WorldMut {
            world: &mut self.world,
            dependencies,
            before,
        }
    }

    /// Records an unexpanded deferred-write payload after validating that its
    /// token list belongs to this live timeline.
    pub fn record_deferred_write(&mut self, stream: StreamSlot, tokens: TokenListId) {
        self.stores.assert_live_token_list(tokens);
        let tokens = self.stores.token_list_ref(tokens);
        self.world.record_deferred_write(stream, tokens);
        self.dependencies
            .get_mut()
            .expect("dependency runtime mutex is not poisoned")
            .mark_changed(DependencyKey::World {
                field: DependencyWorldField::OutputStream,
                index: u64::from(stream.raw()),
            });
    }

    /// Marks the start of node allocations owned by one in-progress shipout.
    #[must_use]
    pub fn begin_shipout(&mut self) -> ShipoutTransaction<'_> {
        self.observe_semantic_dependency(DependencyKey::World {
            field: DependencyWorldField::EffectPolicy,
            index: 0,
        });
        let rollback = self.capture_scoped_rollback();
        ShipoutTransaction {
            universe: self,
            rollback: Some(rollback),
            finished: false,
        }
    }

    /// Begins a full-state speculative transition without computing a durable
    /// checkpoint identity.
    #[doc(hidden)]
    #[must_use]
    pub fn begin_replay_probe(&mut self) -> ReplayProbeTransaction<'_> {
        let rollback = self.capture_scoped_rollback();
        ReplayProbeTransaction {
            universe: self,
            rollback: Some(rollback),
        }
    }

    /// Commits an effect prefix and retargets semantic hash cursors after it is dropped.
    pub fn commit_effects(&mut self, effect_pos: EffectPos) -> Result<(), WorldError> {
        if self.world.commit_mode() == WorldCommitMode::Retained {
            return Ok(());
        }
        self.poison_tracked_region(TrackedRegionBarrier::IrreversibleEffect);
        let hash_base = self.state_hash_base.clone();
        if let Err(err) = self.world.commit_effects(effect_pos) {
            self.state_hash_base = self.retarget_hash_base_after_committed_boundary(hash_base);
            return Err(err);
        }
        self.dependencies
            .get_mut()
            .expect("dependency runtime mutex is not poisoned")
            .mark_changed(DependencyKey::World {
                field: DependencyWorldField::MaterializationBarrier,
                index: 0,
            });
        self.state_hash_base = self.retarget_hash_base_after_committed_boundary(hash_base);
        Ok(())
    }

    /// Opens a rollback-capable editor session with deferred host materialization.
    pub fn begin_retained_session(&mut self) -> Result<(), WorldError> {
        self.world.begin_retained_session()?;
        self.dependencies
            .get_mut()
            .expect("dependency runtime mutex is not poisoned")
            .mark_changed(DependencyKey::World {
                field: DependencyWorldField::EffectPolicy,
                index: 0,
            });
        Ok(())
    }

    /// Captures an authored fragment's borrowed terminal-input cursor.
    #[doc(hidden)]
    pub fn terminal_input_position(&self) -> crate::world::TerminalInputPosition {
        self.world.terminal_input_position()
    }

    /// Restores a position captured by [`Self::terminal_input_position`].
    #[doc(hidden)]
    pub fn restore_terminal_input_position(
        &mut self,
        position: crate::world::TerminalInputPosition,
    ) {
        self.world.restore_terminal_input_position(position);
        self.dependencies
            .get_mut()
            .expect("dependency runtime mutex is not poisoned")
            .mark_changed(DependencyKey::World {
                field: DependencyWorldField::TerminalInputCursor,
                index: 0,
            });
    }

    /// Consumes the retained effect branch by exposing it exactly once in order.
    pub fn export_retained_effects(&mut self) -> Result<(), WorldError> {
        let hash_base = self.state_hash_base.clone();
        self.world.export_retained_effects()?;
        let dependencies = self
            .dependencies
            .get_mut()
            .expect("dependency runtime mutex is not poisoned");
        dependencies.mark_changed(DependencyKey::World {
            field: DependencyWorldField::EffectPolicy,
            index: 0,
        });
        dependencies.mark_changed(DependencyKey::World {
            field: DependencyWorldField::MaterializationBarrier,
            index: 0,
        });
        self.state_hash_base = self.retarget_hash_base_after_committed_boundary(hash_base);
        Ok(())
    }

    /// Exposes one retained prefix while preserving its ordered suffix.
    #[doc(hidden)]
    pub fn export_retained_effects_through(
        &mut self,
        effect_pos: EffectPos,
    ) -> Result<(), WorldError> {
        let hash_base = self.state_hash_base.clone();
        self.world.commit_effects(effect_pos)?;
        self.state_hash_base = self.retarget_hash_base_after_committed_boundary(hash_base);
        Ok(())
    }

    /// Bytes required by detached artifacts and the virtual effect suffix.
    #[must_use]
    pub fn retained_output_bytes(&self) -> usize {
        self.world.retained_output_bytes()
    }

    /// Approximate ownership charge for a live private execution generation.
    /// This uses the same aggregate roots as accepted-generation accounting,
    /// before any additional accepted diagnostic-origin map is installed.
    #[must_use]
    pub fn live_generation_charged_bytes(&self) -> usize {
        generation_charged_bytes(self)
    }

    /// Rehomes the stable root editor frame without registering a document-sized backing.
    pub fn rebind_root_editor_layout(
        &self,
        summary: &InputSummary,
        bytes: &[u8],
        mapped_position: usize,
    ) -> Result<(InputSummary, SourceId), SourceMapError> {
        if mapped_position > bytes.len()
            || std::str::from_utf8(bytes)
                .ok()
                .is_none_or(|source| !source.is_char_boundary(mapped_position))
        {
            return Err(SourceMapError::OffsetOutsideSource);
        }
        summary
            .rebind_root_layout(bytes, mapped_position)
            .ok_or(SourceMapError::UnknownSource)
    }

    /// Installs the immutable session fragment snapshot for this compile after
    /// validating that the accepted layout belongs to the same lineage.
    pub fn install_editor_fragments(
        &mut self,
        fragments: &crate::FragmentStore,
        layout: &crate::EditorLayout,
    ) -> Result<(), crate::EditorLayoutError> {
        layout.validate_store(fragments)?;
        self.stores.install_source_fragments(
            fragments
                .metadata_snapshot_for_layout(layout, self.provenance_demand.rendered_source()),
        );
        Ok(())
    }

    /// Binds a command continuation's rebound root coordinate capability to
    /// the currently installed editor piece table.
    #[doc(hidden)]
    pub fn bind_rebound_editor_root_registration(&mut self, source: SourceId) {
        self.stores.bind_rebound_root_registration(source);
    }

    /// Resolves the root editor piece consumed by a token, following bounded
    /// macro/inserted provenance back to its invocation site.
    #[must_use]
    pub fn root_span_for_origin(
        &self,
        origin: crate::token::OriginId,
    ) -> Option<crate::RootSpanId> {
        let mut current = origin;
        for _ in 0..68 {
            if let Some(span) = self.stores.direct_root_span_id(current) {
                return Some(span);
            }
            current = match self.origin_if_live(current)? {
                crate::provenance::OriginRecord::MacroInvocation(invocation) => {
                    invocation.invocation()
                }
                crate::provenance::OriginRecord::Inserted(inserted) => inserted.parent(),
                crate::provenance::OriginRecord::Synthesized(synthesized) => synthesized.parent(),
                crate::provenance::OriginRecord::Source(source) => {
                    return self.stores.source_origin_root_span_id(source);
                }
                crate::provenance::OriginRecord::SourceSpan(span) => {
                    return self.stores.source_span_root_span_id(span);
                }
                crate::provenance::OriginRecord::UnknownBootstrap
                | crate::provenance::OriginRecord::Synthetic(_) => return None,
            };
        }
        None
    }

    /// Resolves arena-independent editor backing captured before provenance
    /// rollback against a live revision layout.
    #[must_use]
    pub fn resolve_root_span(
        &self,
        span: crate::RootSpanId,
        fragments: &crate::FragmentStore,
        layout: &crate::EditorLayout,
    ) -> crate::LayoutResolvedOrigin {
        crate::source_fragments::resolve_root_span(span, fragments, layout)
    }

    /// Recreates a diagnostic source origin from validated stable root identity.
    #[doc(hidden)]
    pub fn origin_for_root_span(&mut self, span: crate::RootSpanId) -> Option<OriginId> {
        let source_span = self.stores.source_span_for_root(span)?;
        Some(self.stores.source_span_origin(source_span))
    }

    /// Sets operational editor revision identity outside semantic state.
    pub fn set_root_editor_content_hash(&mut self, hash: ContentHash) {
        self.editor_content_hash = Some(hash);
    }

    /// Returns the explicit editor revision identity installed by the host.
    #[must_use]
    pub const fn explicit_root_editor_content_hash(&self) -> Option<ContentHash> {
        self.editor_content_hash
    }

    #[must_use]
    pub fn root_editor_content_hash(&self, summary: &InputSummary) -> Option<ContentHash> {
        self.editor_content_hash
            .or_else(|| self.stores.root_generated_content_hash(summary))
    }

    /// Records the current lexer-owned input stack state for the next snapshot.
    pub fn set_input_summary(&mut self, summary: InputSummary) {
        self.stores.assert_live_input_summary(&self.world, &summary);
        self.input_summary = summary;
        self.dependencies
            .get_mut()
            .expect("dependency runtime mutex is not poisoned")
            .mark_changed(DependencyKey::InputStack);
    }

    /// tex.web's `line`: the innermost open file's current line number, or 0
    /// when input is not coming from a file.
    ///
    /// The driver publishes this as input advances, the way tex.web's
    /// `get_next` maintains the global. §660/§675's box diagnostics are the
    /// consumers; `\inputlineno` reads the live input stack instead, because
    /// it is scanned where that stack is in hand.
    #[must_use]
    pub const fn current_input_line(&self) -> i32 {
        self.diagnostic_position.line
    }

    pub const fn set_current_input_line(&mut self, line: i32) {
        self.diagnostic_position.line = line;
        self.diagnostic_position.source = None;
    }

    pub const fn set_current_input_position(&mut self, line: i32, source: Option<SourceId>) {
        self.diagnostic_position.line = line;
        self.diagnostic_position.source = source;
    }

    #[must_use]
    pub const fn current_input_source(&self) -> Option<SourceId> {
        self.diagnostic_position.source
    }

    /// tex.web §661's `pack_begin_line`.
    #[must_use]
    pub const fn pack_begin_line(&self) -> i32 {
        self.diagnostic_position.pack_begin_line
    }

    /// §804's and §768's assignments, both of which restore 0 when the
    /// packing they scope is finished.
    pub const fn set_pack_begin_line(&mut self, line: i32) {
        self.diagnostic_position.pack_begin_line = line;
    }

    /// §1091's `new_graf`: records the line the paragraph now starting began
    /// on.
    pub fn push_paragraph_start_line(&mut self, line: i32) {
        self.diagnostic_position.paragraph_start_lines.push(line);
    }

    /// §804's `pack_begin_line:=mode_line`: removes and returns the innermost
    /// open paragraph's start line as that paragraph is broken.
    pub fn pop_paragraph_start_line(&mut self) -> Option<i32> {
        self.diagnostic_position.paragraph_start_lines.pop()
    }

    /// tex.web §1025's `output_active`.
    #[must_use]
    pub const fn output_routine_is_active(&self) -> bool {
        self.diagnostic_position.output_active
    }

    /// §1025's `output_active:=true` and §1026's restore.
    pub const fn set_output_routine_active(&mut self, active: bool) {
        self.diagnostic_position.output_active = active;
    }

    /// Returns the lexer-owned input stack state restored by the last rollback.
    #[must_use]
    pub const fn input_summary(&self) -> &InputSummary {
        &self.input_summary
    }

    /// Returns the format's `\everyjob` list exactly once for a fresh job.
    ///
    /// The marker is operational rather than format state: dumping a format
    /// does not schedule `\everyjob` in the INITEX job that creates it, while
    /// every timeline constructed from that image starts with it pending.
    pub fn take_pending_every_job(&mut self) -> TokenListId {
        if !std::mem::take(&mut self.pending_every_job) {
            return TokenListId::EMPTY;
        }
        self.tok_param(TokParam::EVERY_JOB)
    }

    /// Returns the current interaction mode.
    #[must_use]
    pub fn interaction_mode(&self) -> InteractionMode {
        self.observe_semantic_dependency(DependencyKey::Engine(
            DependencyEngineField::InteractionMode,
        ));
        self.interaction_mode
    }

    /// Sets the current interaction mode.
    pub fn set_interaction_mode(&mut self, mode: InteractionMode) {
        self.interaction_mode = mode;
        self.dependencies
            .get_mut()
            .expect("dependency runtime mutex is not poisoned")
            .mark_changed(DependencyKey::Engine(
                DependencyEngineField::InteractionMode,
            ));
    }

    pub fn set_pdf_match_state(
        &mut self,
        haystack: Vec<u8>,
        captures: Vec<Option<(u32, u32)>>,
        slot_count: u32,
        matched: bool,
    ) {
        self.pdf.set_match(haystack, captures, slot_count, matched);
        self.mark_pdf_dependency_changed(DependencyEngineField::PdfObjects);
    }

    #[must_use]
    pub fn pdf_match_capture(&self, index: u32) -> Option<(u32, &[u8])> {
        self.observe_pdf_dependency(DependencyEngineField::PdfObjects);
        self.pdf.match_capture(index)
    }

    #[must_use]
    pub fn pdf_elapsed_time(&self) -> i32 {
        self.observe_semantic_dependency(DependencyKey::Engine(DependencyEngineField::PdfTimer));
        self.world.pdf_elapsed_time()
    }

    #[must_use]
    pub fn pdf_random_seed(&self) -> i32 {
        self.observe_semantic_dependency(DependencyKey::Engine(DependencyEngineField::PdfRandom));
        self.world.pdf_random_seed()
    }

    #[must_use]
    pub fn pdf_shell_escape_status(&self) -> i32 {
        self.observe_semantic_dependency(DependencyKey::Engine(
            DependencyEngineField::PdfShellEscape,
        ));
        match self.world.shell_escape_policy() {
            ShellEscapePolicy::Disabled => 0,
            ShellEscapePolicy::Enabled => 1,
            ShellEscapePolicy::Restricted => 2,
        }
    }

    pub fn pdf_uniform_deviate(&mut self, bound: i32) -> i32 {
        self.observe_semantic_dependency(DependencyKey::Engine(DependencyEngineField::PdfRandom));
        let value = self.world.pdf_uniform_deviate(bound);
        self.dependencies
            .get_mut()
            .expect("dependency runtime mutex is not poisoned")
            .mark_changed(DependencyKey::Engine(DependencyEngineField::PdfRandom));
        value
    }

    pub fn pdf_normal_deviate(&mut self) -> i32 {
        self.observe_semantic_dependency(DependencyKey::Engine(DependencyEngineField::PdfRandom));
        let value = self.world.pdf_normal_deviate();
        self.dependencies
            .get_mut()
            .expect("dependency runtime mutex is not poisoned")
            .mark_changed(DependencyKey::Engine(DependencyEngineField::PdfRandom));
        value
    }

    /// Enables checkpointed PDF object allocation for this timeline.
    pub fn enable_pdf_output(&mut self) {
        self.pdf.enable();
        self.mark_pdf_dependency_changed(DependencyEngineField::PdfObjects);
    }

    #[must_use]
    pub fn pdf_output_enabled(&self) -> bool {
        self.observe_pdf_dependency(DependencyEngineField::PdfObjects);
        self.pdf.enabled()
    }

    #[must_use]
    pub fn pdf_pages(&self) -> &[crate::PdfPageRecord] {
        self.observe_pdf_dependency(DependencyEngineField::PdfPages);
        self.pdf.pages()
    }

    pub fn set_pdf_space_font_name(&mut self, name: Vec<u8>) {
        self.pdf.set_space_font_name(name);
        self.mark_pdf_dependency_changed(DependencyEngineField::PdfObjects);
    }

    #[must_use]
    pub fn pdf_space_font_name(&self, id: u32) -> Option<&[u8]> {
        self.observe_pdf_dependency(DependencyEngineField::PdfObjects);
        self.pdf.space_font_name(id)
    }

    #[must_use]
    pub fn pdf_next_object_id(&self) -> u32 {
        self.observe_pdf_dependency(DependencyEngineField::PdfObjects);
        self.pdf.next_object()
    }

    /// Returns the output controls fixed by the first committed shipout.
    #[must_use]
    pub fn fixed_pdf_output_parameters(&self) -> Option<PdfOutputParameters> {
        self.observe_pdf_dependency(DependencyEngineField::PdfPages);
        self.pdf.output_parameters()
    }

    /// Reads the live pdfTeX microtype/font-output parameters from their
    /// canonical grouped integer cells.
    #[must_use]
    pub fn pdf_font_configuration(&self) -> crate::PdfFontConfiguration {
        crate::PdfFontConfiguration {
            adjust_spacing: self.int_param(IntParam::PDF_ADJUST_SPACING),
            protrude_chars: self.int_param(IntParam::PDF_PROTRUDE_CHARS),
            tracing_fonts: self.int_param(IntParam::PDF_TRACING_FONTS),
            adjust_interword_glue: self.int_param(IntParam::PDF_ADJUST_INTERWORD_GLUE),
            prepend_kern: self.int_param(IntParam::PDF_PREPEND_KERN),
            append_kern: self.int_param(IntParam::PDF_APPEND_KERN),
            generate_to_unicode: self.int_param(IntParam::PDF_GEN_TO_UNICODE),
            pk_resolution: self.int_param(IntParam::PDF_PK_RESOLUTION),
            omit_charset: self.int_param(IntParam::PDF_OMIT_CHARSET),
        }
    }

    /// Returns the PK mode consumed when PDF output was first initialized.
    #[must_use]
    pub fn fixed_pdf_pk_mode(&self) -> Option<TokenListId> {
        self.observe_pdf_dependency(DependencyEngineField::PdfPages);
        self.pdf.pk_mode()
    }

    /// Registers validated, detached metadata for an external-image object.
    pub fn register_pdf_external_image(
        &mut self,
        id: PdfExternalImageId,
        metadata: PdfExternalImageMetadata,
    ) -> Result<(), PdfExternalImageRegistrationError> {
        let result = self.pdf.register_external_image(id, metadata);
        if result.is_ok() {
            self.mark_pdf_dependency_changed(DependencyEngineField::PdfExternalImages);
        }
        result
    }

    #[must_use]
    pub fn pdf_external_image(&self, id: PdfExternalImageId) -> Option<PdfExternalImageMetadata> {
        self.observe_pdf_dependency(DependencyEngineField::PdfExternalImages);
        self.pdf.external_image(id)
    }

    /// Starts pdfTeX's page-group selection policy for one output page.
    #[must_use]
    pub fn pdf_page_group_selector(&self) -> crate::PdfPageGroupSelector {
        crate::PdfPageGroupSelector::new(self.int_param(IntParam::PDF_SUPPRESS_WARNING_PAGE_GROUP))
    }

    pub fn allocate_pdf_color_stack(
        &mut self,
        mode: crate::PdfColorStackMode,
        restore_at_page_start: bool,
        initial: Vec<u8>,
    ) -> Result<u32, crate::PdfColorStackCapacityError> {
        let result = self
            .pdf
            .allocate_color_stack(mode, restore_at_page_start, initial);
        if result.is_ok() {
            self.mark_pdf_dependency_changed(DependencyEngineField::PdfObjects);
        }
        result
    }

    pub fn has_pdf_color_stack(&mut self, id: u32) -> bool {
        self.observe_pdf_dependency(DependencyEngineField::PdfObjects);
        self.pdf.has_color_stack(id)
    }

    pub fn apply_pdf_color_stack(
        &mut self,
        id: u32,
        target: crate::PdfColorStackTarget,
        action: &crate::PdfColorStackAction,
    ) -> Result<crate::PdfColorStackEmission, crate::PdfColorStackApplyError> {
        let result = self.pdf.apply_color_stack(id, target, action);
        if result.is_ok() {
            self.mark_pdf_dependency_changed(DependencyEngineField::PdfObjects);
        }
        result
    }

    pub fn pdf_page_color_stack_restorations(&mut self) -> Vec<crate::PdfColorStackEmission> {
        self.observe_pdf_dependency(DependencyEngineField::PdfObjects);
        self.pdf.page_color_stack_restorations()
    }

    pub fn pdf_last_position(&self) -> (Scaled, Scaled) {
        self.observe_pdf_dependency(DependencyEngineField::PdfPositions);
        self.pdf.last_position()
    }

    pub fn pdf_snap_reference(&self) -> (Scaled, Scaled) {
        self.observe_pdf_dependency(DependencyEngineField::PdfPositions);
        self.pdf.snap_reference()
    }

    pub fn publish_pdf_traversal_positions(
        &mut self,
        last_position: Option<(Scaled, Scaled)>,
        snap_reference: (Scaled, Scaled),
    ) {
        self.pdf
            .publish_traversal_positions(last_position, snap_reference);
        self.mark_pdf_dependency_changed(DependencyEngineField::PdfPositions);
    }

    /// Records a parsed, host-neutral font-map mutation.
    pub fn push_pdf_font_map(&mut self, operation: crate::PdfFontMapOperation) {
        self.pdf.push_font_map(operation);
        self.mark_pdf_dependency_changed(DependencyEngineField::PdfObjects);
    }

    pub fn pdf_font_maps(&self) -> impl Iterator<Item = &crate::PdfFontMapOperation> {
        self.observe_pdf_dependency(DependencyEngineField::PdfObjects);
        self.pdf.font_maps()
    }

    #[must_use]
    pub fn pdf_font_map_file_requests(&self) -> Vec<Vec<u8>> {
        self.observe_pdf_dependency(DependencyEngineField::PdfObjects);
        self.pdf.font_map_file_requests()
    }

    pub fn provide_pdf_font_map_file(
        &mut self,
        logical_name: Vec<u8>,
        bytes: &[u8],
    ) -> Result<(), tex_fonts::PdfFontMapError> {
        let map = tex_fonts::PdfFontMap::parse(bytes)?;
        self.pdf.provide_font_map_file(logical_name, map);
        self.mark_pdf_dependency_changed(DependencyEngineField::PdfObjects);
        Ok(())
    }

    #[must_use]
    pub fn has_pdf_font_map_file(&self, logical_name: &[u8]) -> bool {
        self.observe_pdf_dependency(DependencyEngineField::PdfObjects);
        self.pdf.has_font_map_file(logical_name)
    }

    #[must_use]
    pub fn authoritative_pdf_font_map_names(&self) -> Vec<Vec<u8>> {
        self.observe_pdf_dependency(DependencyEngineField::PdfObjects);
        self.pdf
            .authoritative_font_map_names()
            .into_keys()
            .collect()
    }

    #[must_use]
    pub fn resolved_pdf_font_map_lines(&self) -> Vec<tex_fonts::PdfFontMapEntry> {
        self.observe_pdf_dependency(DependencyEngineField::PdfObjects);
        self.pdf.resolved_font_map_lines()
    }

    #[must_use]
    pub fn pdf_font_map_duplicate_names(&self) -> Vec<Vec<u8>> {
        self.observe_pdf_dependency(DependencyEngineField::PdfObjects);
        self.pdf.font_map_duplicate_names()
    }

    pub fn set_pdf_font_attribute(&mut self, font: FontId, bytes: Vec<u8>) {
        self.pdf.set_font_attribute(font, bytes);
        self.mark_pdf_dependency_changed(DependencyEngineField::PdfObjects);
    }

    #[must_use]
    pub fn pdf_font_attribute(&self, font: FontId) -> &[u8] {
        self.observe_pdf_dependency(DependencyEngineField::PdfObjects);
        self.pdf.font_attribute(font)
    }

    pub fn include_pdf_font_chars(&mut self, font: FontId, chars: Vec<u8>) {
        self.pdf.include_font_chars(font, chars);
        self.mark_pdf_dependency_changed(DependencyEngineField::PdfObjects);
    }

    #[must_use]
    pub fn included_pdf_font_chars(&self, font: FontId) -> Vec<u8> {
        self.observe_pdf_dependency(DependencyEngineField::PdfObjects);
        self.pdf.included_font_chars(font)
    }

    pub fn set_pdf_glyph_to_unicode(&mut self, mapping: crate::PdfGlyphToUnicode) {
        self.pdf.set_glyph_to_unicode(mapping);
        self.mark_pdf_dependency_changed(DependencyEngineField::PdfObjects);
    }

    #[must_use]
    pub fn pdf_glyph_to_unicode(&self, tfm_name: &[u8], glyph_name: &[u8]) -> Option<&[u32]> {
        self.observe_pdf_dependency(DependencyEngineField::PdfObjects);
        self.pdf.glyph_to_unicode(tfm_name, glyph_name)
    }

    #[must_use]
    pub fn has_pdf_glyph_to_unicode_mappings(&self) -> bool {
        self.observe_pdf_dependency(DependencyEngineField::PdfObjects);
        self.pdf.has_glyph_to_unicode_mappings()
    }

    pub fn disable_pdf_builtin_to_unicode(&mut self, font: FontId) {
        self.pdf.disable_builtin_to_unicode(font);
        self.mark_pdf_dependency_changed(DependencyEngineField::PdfObjects);
    }

    #[must_use]
    pub fn pdf_builtin_to_unicode_disabled(&self, font: FontId) -> bool {
        self.observe_pdf_dependency(DependencyEngineField::PdfObjects);
        self.pdf.builtin_to_unicode_disabled(font)
    }

    /// Supplies already acquired Type-1 bytes through a typed, host-neutral
    /// boundary. Parsing strips PFB transport framing before publication.
    pub fn provide_pdf_type1_program(
        &mut self,
        logical_name: Vec<u8>,
        bytes: &[u8],
    ) -> Result<(), tex_fonts::PdfType1ProgramError> {
        let program = tex_fonts::PdfType1Program::from_pfb(bytes)?;
        self.pdf.provide_type1_program(logical_name, program);
        self.mark_pdf_dependency_changed(DependencyEngineField::PdfObjects);
        Ok(())
    }

    #[must_use]
    pub fn pdf_type1_program(&self, logical_name: &[u8]) -> Option<&tex_fonts::PdfType1Program> {
        self.observe_pdf_dependency(DependencyEngineField::PdfObjects);
        self.pdf.type1_program(logical_name)
    }

    pub fn provide_pdf_encoding(
        &mut self,
        logical_name: Vec<u8>,
        bytes: &[u8],
    ) -> Result<(), tex_fonts::PdfEncodingError> {
        let encoding = tex_fonts::PdfEncoding::parse(bytes)?;
        self.pdf.provide_encoding(logical_name, encoding);
        self.mark_pdf_dependency_changed(DependencyEngineField::PdfObjects);
        Ok(())
    }

    #[must_use]
    pub fn pdf_encoding(&self, logical_name: &[u8]) -> Option<&tex_fonts::PdfEncoding> {
        self.observe_pdf_dependency(DependencyEngineField::PdfObjects);
        self.pdf.encoding(logical_name)
    }

    pub fn provide_pdf_truetype_program(
        &mut self,
        logical_name: Vec<u8>,
        bytes: &[u8],
    ) -> Result<(), tex_fonts::PdfTrueTypeProgramError> {
        let is_woff2 = logical_name
            .rsplit(|byte| *byte == b'.')
            .next()
            .is_some_and(|extension| extension.eq_ignore_ascii_case(b"woff2"));
        let program = if is_woff2 {
            tex_fonts::PdfTrueTypeProgram::from_woff2(bytes)?
        } else {
            tex_fonts::PdfTrueTypeProgram::parse(bytes)?
        };
        self.pdf.provide_truetype_program(logical_name, program);
        self.mark_pdf_dependency_changed(DependencyEngineField::PdfObjects);
        Ok(())
    }

    #[must_use]
    pub fn pdf_truetype_program(
        &self,
        logical_name: &[u8],
    ) -> Option<&tex_fonts::PdfTrueTypeProgram> {
        self.observe_pdf_dependency(DependencyEngineField::PdfObjects);
        self.pdf.truetype_program(logical_name)
    }

    /// Supplies a PK bitmap program already acquired by a host for the exact
    /// typed name, resolution, and mode request.
    pub fn provide_pdf_pk_font(
        &mut self,
        request: tex_fonts::PdfPkFontRequest,
        bytes: &[u8],
    ) -> Result<(), tex_fonts::PdfPkFontError> {
        let font = tex_fonts::PdfPkFont::parse(bytes)?;
        self.pdf.provide_pk_font(request, font);
        self.mark_pdf_dependency_changed(DependencyEngineField::PdfObjects);
        Ok(())
    }

    #[must_use]
    pub fn pdf_pk_font(
        &self,
        request: &tex_fonts::PdfPkFontRequest,
    ) -> Option<&tex_fonts::PdfPkFont> {
        self.observe_pdf_dependency(DependencyEngineField::PdfObjects);
        self.pdf.pk_font(request)
    }

    pub fn allocate_pdf_external_image(
        &mut self,
        source: crate::PdfExternalImageSource,
        dimensions: crate::PdfExternalImageDimensions,
        color_space_object: i32,
    ) -> Result<crate::PdfExternalImageRecord, PdfObjectCapacityError> {
        let result = self
            .pdf
            .allocate_external_image(source, dimensions, color_space_object);
        if result.is_ok() {
            self.mark_pdf_dependency_changed(DependencyEngineField::PdfExternalImages);
            self.mark_pdf_dependency_changed(DependencyEngineField::PdfObjects);
        }
        result
    }

    #[must_use]
    pub fn pdf_last_external_image(&self) -> Option<crate::PdfExternalImageRecord> {
        self.observe_pdf_dependency(DependencyEngineField::PdfExternalImages);
        self.pdf.last_external_image()
    }

    #[must_use]
    pub fn pdf_external_image_record(
        &self,
        id: crate::PdfExternalImageId,
    ) -> Option<crate::PdfExternalImageRecord> {
        self.observe_pdf_dependency(DependencyEngineField::PdfExternalImages);
        self.pdf.external_image_record(id)
    }

    #[must_use]
    pub fn pdf_external_images(&self) -> &[crate::PdfExternalImageRecord] {
        self.observe_pdf_dependency(DependencyEngineField::PdfExternalImages);
        self.pdf.external_images()
    }

    /// Lazily reserves the page-resource name and font-dictionary object used
    /// by enquiries and by the first shipped page containing this font.
    pub fn ensure_pdf_font_resource(
        &mut self,
        font: FontId,
    ) -> Result<PdfFontResourceRecord, PdfObjectCapacityError> {
        let loaded = self.font(font);
        let source_identity = loaded.source_identity();
        let identity = loaded.pdf_resource_identity();
        let result = self
            .pdf
            .ensure_font_resource(font, source_identity, identity);
        if result.is_ok() {
            self.mark_pdf_dependency_changed(DependencyEngineField::PdfObjects);
        }
        result
    }

    #[must_use]
    pub fn pdf_font_resource(&self, font: FontId) -> Option<PdfFontResourceRecord> {
        self.observe_pdf_dependency(DependencyEngineField::PdfObjects);
        self.pdf.font_resource(font)
    }

    #[must_use]
    pub fn pdf_font_resource_by_identity(
        &self,
        identity: tex_fonts::FontSourceIdentity,
    ) -> Option<PdfFontResourceRecord> {
        self.observe_pdf_dependency(DependencyEngineField::PdfObjects);
        self.pdf.font_resource_by_identity(identity)
    }

    pub fn pdf_font_resources(&self) -> impl Iterator<Item = PdfFontResourceRecord> + '_ {
        self.observe_pdf_dependency(DependencyEngineField::PdfObjects);
        self.pdf.font_resources()
    }

    /// Reserves the next identity in the canonical PDF object ledger.
    pub fn reserve_pdf_raw_object(&mut self) -> Result<PdfRawObjectId, PdfObjectCapacityError> {
        let result = self.pdf.reserve_raw_object();
        if result.is_ok() {
            self.mark_pdf_dependency_changed(DependencyEngineField::PdfObjects);
        }
        result
    }

    /// Reserves pdfTeX's object and resource identities before scanning form options.
    pub fn reserve_pdf_form(&mut self) -> Result<(u32, u32), PdfObjectCapacityError> {
        let result = self.pdf.reserve_form();
        if result.is_ok() {
            self.mark_pdf_dependency_changed(DependencyEngineField::PdfForms);
            self.mark_pdf_dependency_changed(DependencyEngineField::PdfObjects);
        }
        result
    }

    /// Captures a consumed box into a previously reserved PDF form identity.
    pub fn initialize_pdf_form(
        &mut self,
        identity: (u32, u32),
        box_list: NodeListRef,
        dimensions: (Scaled, Scaled, Scaled),
        attr: Option<TokenListId>,
        resources: Option<TokenListId>,
        immediate: bool,
    ) -> Result<crate::PdfFormRecord, PdfObjectCapacityError> {
        let semantic_id = box_list.semantic_id().fragment();
        let attr = attr.map(|tokens| self.pdf_token_parameter(tokens));
        let resources = resources.map(|tokens| self.pdf_token_parameter(tokens));
        let form = self.pdf.initialize_form(
            identity,
            box_list,
            semantic_id,
            dimensions,
            (attr, resources),
            immediate,
        )?;
        self.mark_pdf_dependency_changed(DependencyEngineField::PdfForms);
        Ok(form)
    }

    #[must_use]
    pub fn pdf_form(&self, object: u32) -> Option<crate::PdfFormRecord> {
        self.observe_pdf_dependency(DependencyEngineField::PdfForms);
        self.pdf.form(object)
    }

    pub fn pdf_forms(&self) -> impl ExactSizeIterator<Item = crate::PdfFormRecord> + '_ {
        self.observe_pdf_dependency(DependencyEngineField::PdfForms);
        self.pdf.forms()
    }

    #[must_use]
    pub fn pdf_last_form(&self) -> u32 {
        self.observe_pdf_dependency(DependencyEngineField::PdfForms);
        self.pdf.last_form()
    }

    pub fn set_pdf_form_artifact(&mut self, object: u32, artifact: crate::PdfFormArtifact) {
        self.pdf.set_form_artifact(object, artifact);
        self.mark_pdf_dependency_changed(DependencyEngineField::PdfForms);
    }

    #[must_use]
    pub fn pdf_form_artifact(&self, object: u32) -> Option<&crate::PdfFormArtifact> {
        self.observe_pdf_dependency(DependencyEngineField::PdfForms);
        self.pdf.form_artifact(object)
    }

    pub fn pdf_form_color_rollback(&self) -> crate::PdfFormColorRollback {
        self.observe_pdf_dependency(DependencyEngineField::PdfForms);
        self.pdf.form_color_rollback()
    }

    pub fn rollback_pdf_form_colors(&mut self, rollback: crate::PdfFormColorRollback) {
        self.pdf.rollback_form_colors(rollback);
        self.mark_pdf_dependency_changed(DependencyEngineField::PdfForms);
    }

    /// Initializes a previously reserved raw object without changing its ID.
    pub fn initialize_pdf_raw_object(
        &mut self,
        id: PdfRawObjectId,
        stream: bool,
        stream_attr: Option<TokenListId>,
        file: bool,
        data: TokenListId,
        immediate: bool,
    ) -> Result<(), PdfRawObjectInitializeError> {
        let stream_attr = stream_attr.map(|tokens| self.pdf_token_parameter(tokens));
        let data = self.pdf_token_parameter(data);
        let result = self.pdf.initialize_raw_object(
            id,
            PdfRawObjectData::new(stream, stream_attr, file, data),
            immediate,
        );
        if result.is_ok() {
            self.mark_pdf_dependency_changed(DependencyEngineField::PdfObjects);
        }
        result
    }

    #[must_use]
    pub fn pdf_raw_object(&self, raw: u32) -> Option<PdfRawObjectRecord> {
        self.observe_pdf_dependency(DependencyEngineField::PdfObjects);
        self.pdf.raw_object(PdfRawObjectId::from_allocated(raw))
    }

    pub fn reference_pdf_raw_object(
        &mut self,
        raw: u32,
    ) -> Result<(), PdfRawObjectInitializeError> {
        let result = self
            .pdf
            .reference_raw_object(PdfRawObjectId::from_allocated(raw));
        if result.is_ok() {
            self.mark_pdf_dependency_changed(DependencyEngineField::PdfObjects);
        }
        result
    }

    #[must_use]
    pub fn pdf_raw_objects(&self) -> &[PdfRawObjectRecord] {
        self.observe_pdf_dependency(DependencyEngineField::PdfObjects);
        self.pdf.raw_objects()
    }

    #[must_use]
    pub fn pdf_last_object(&self) -> u32 {
        self.observe_pdf_dependency(DependencyEngineField::PdfObjects);
        self.pdf.last_raw_object()
    }

    #[must_use]
    pub fn pdf_last_annotation(&self) -> u32 {
        self.observe_pdf_dependency(DependencyEngineField::PdfObjects);
        self.pdf.last_annotation()
    }

    #[must_use]
    pub fn pdf_last_link(&self) -> u32 {
        self.observe_pdf_dependency(DependencyEngineField::PdfObjects);
        self.pdf.last_link()
    }

    pub fn reserve_pdf_annotation(
        &mut self,
    ) -> Result<crate::PdfAnnotationRecord, PdfObjectCapacityError> {
        let result = self.pdf.reserve_annotation();
        if result.is_ok() {
            self.mark_pdf_dependency_changed(DependencyEngineField::PdfObjects);
        }
        result
    }

    pub fn initialize_pdf_annotation(
        &mut self,
        object: u32,
        data: crate::PdfAnnotationData,
    ) -> Result<crate::PdfAnnotationRecord, crate::PdfAnnotationInitializeError> {
        let semantic_id = self.stores.token_list_semantic_fragment(data.entries.id());
        let result = self.pdf.initialize_annotation(object, data, semantic_id);
        if result.is_ok() {
            self.mark_pdf_dependency_changed(DependencyEngineField::PdfObjects);
        }
        result
    }

    pub fn create_pdf_annotation(
        &mut self,
        data: crate::PdfAnnotationData,
    ) -> Result<crate::PdfAnnotationRecord, PdfObjectCapacityError> {
        let record = self.reserve_pdf_annotation()?;
        let record = self
            .initialize_pdf_annotation(record.object(), data)
            .expect("fresh annotation reservation initializes");
        self.mark_pdf_dependency_changed(DependencyEngineField::PdfObjects);
        Ok(record)
    }

    #[must_use]
    pub fn pdf_annotations(&self) -> &[crate::PdfAnnotationRecord] {
        self.observe_pdf_dependency(DependencyEngineField::PdfObjects);
        self.pdf.annotations()
    }

    #[must_use]
    pub fn pdf_destination(
        &self,
        identity: &crate::PdfDestinationIdentity,
        structure: bool,
    ) -> Option<&crate::PdfDestinationRecord> {
        self.observe_pdf_dependency(DependencyEngineField::PdfObjects);
        self.pdf.destination(identity, structure)
    }

    pub fn reserve_pdf_destination(
        &mut self,
        identity: crate::PdfDestinationIdentity,
        structure: bool,
    ) -> Result<crate::PdfDestinationRecord, PdfObjectCapacityError> {
        let result = self.pdf.reserve_destination(identity, structure);
        if result.is_ok() {
            self.mark_pdf_dependency_changed(DependencyEngineField::PdfObjects);
        }
        result
    }

    pub fn define_pdf_destination(
        &mut self,
        identity: crate::PdfDestinationIdentity,
        structure_target: Option<u32>,
    ) -> Result<crate::PdfDestinationDefinition, PdfObjectCapacityError> {
        let result = self.pdf.define_destination(identity, structure_target);
        if result.is_ok() {
            self.mark_pdf_dependency_changed(DependencyEngineField::PdfObjects);
        }
        result
    }

    #[must_use]
    pub fn pdf_destinations(&self, structure: bool) -> &[crate::PdfDestinationRecord] {
        self.observe_pdf_dependency(DependencyEngineField::PdfObjects);
        self.pdf.destinations(structure)
    }

    pub fn append_pdf_thread_bead(
        &mut self,
        identity: crate::PdfDestinationIdentity,
    ) -> Result<(crate::PdfThreadRecord, crate::PdfThreadBeadRecord), PdfObjectCapacityError> {
        let result = self.pdf.append_thread_bead(identity);
        if result.is_ok() {
            self.mark_pdf_dependency_changed(DependencyEngineField::PdfObjects);
        }
        result
    }

    pub fn reserve_pdf_thread(
        &mut self,
        identity: crate::PdfDestinationIdentity,
    ) -> Result<crate::PdfThreadRecord, PdfObjectCapacityError> {
        let result = self.pdf.reserve_thread(identity);
        if result.is_ok() {
            self.mark_pdf_dependency_changed(DependencyEngineField::PdfObjects);
        }
        result
    }

    #[must_use]
    pub fn pdf_threads(&self) -> &[crate::PdfThreadRecord] {
        self.observe_pdf_dependency(DependencyEngineField::PdfObjects);
        self.pdf.threads()
    }

    pub fn create_pdf_outline(
        &mut self,
        attributes: TokenListId,
        action: crate::PdfActionSpec,
        count: i32,
        title: TokenListId,
    ) -> Result<crate::PdfOutlineRecord, PdfObjectCapacityError> {
        let attributes_semantic_id = self.stores.token_list_semantic_fragment(attributes);
        let action_semantic_id =
            action.fingerprint(|tokens| self.stores.token_list_semantic_fragment(tokens));
        let title_semantic_id = self.stores.token_list_semantic_fragment(title);
        let attributes = self.stores.token_list_ref(attributes);
        let title = self.stores.token_list_ref(title);
        let result = self.pdf.create_outline(
            attributes,
            action,
            count,
            title,
            [
                attributes_semantic_id,
                action_semantic_id,
                title_semantic_id,
            ],
        );
        if result.is_ok() {
            self.mark_pdf_dependency_changed(DependencyEngineField::PdfObjects);
        }
        result
    }

    #[must_use]
    pub fn pdf_outlines(&self) -> &[crate::PdfOutlineRecord] {
        self.observe_pdf_dependency(DependencyEngineField::PdfObjects);
        self.pdf.outlines()
    }

    pub fn create_pdf_link(
        &mut self,
        dimensions: crate::PdfAnnotationDimensions,
        attributes: TokenListId,
        action: crate::PdfActionSpec,
        nesting_depth: u32,
    ) -> Result<crate::PdfLinkRecord, PdfObjectCapacityError> {
        let attributes_semantic_id = self.stores.token_list_semantic_fragment(attributes);
        let action_semantic_id =
            action.fingerprint(|tokens| self.stores.token_list_semantic_fragment(tokens));
        let attributes = self.stores.token_list_ref(attributes);
        let result = self.pdf.create_link(
            dimensions,
            attributes,
            action,
            attributes_semantic_id,
            action_semantic_id,
            nesting_depth,
        );
        if result.is_ok() {
            self.mark_pdf_dependency_changed(DependencyEngineField::PdfObjects);
        }
        result
    }

    pub fn end_pdf_link(&mut self) -> Option<crate::PdfOpenLink> {
        let result = self.pdf.end_link();
        if result.is_some() {
            self.mark_pdf_dependency_changed(DependencyEngineField::PdfObjects);
        }
        result
    }

    #[must_use]
    pub fn open_pdf_links(&self) -> &[crate::PdfOpenLink] {
        self.observe_pdf_dependency(DependencyEngineField::PdfObjects);
        self.pdf.open_links()
    }

    #[must_use]
    pub fn pdf_links(&self) -> &[crate::PdfLinkRecord] {
        self.observe_pdf_dependency(DependencyEngineField::PdfObjects);
        self.pdf.links()
    }

    /// Reserves a distinct indirect annotation object for a shipped segment
    /// after the logical link's first segment.
    pub fn reserve_pdf_link_continuation(&mut self) -> Result<u32, PdfObjectCapacityError> {
        let result = self.pdf.reserve_link_continuation();
        if result.is_ok() {
            self.mark_pdf_dependency_changed(DependencyEngineField::PdfObjects);
        }
        result
    }

    #[must_use]
    pub fn pdf_last_ximage(&self) -> u32 {
        self.observe_pdf_dependency(DependencyEngineField::PdfExternalImages);
        self.pdf
            .last_external_image()
            .map_or(0, |record| record.id().raw())
    }

    /// Returns the page count reported by the most recently registered image.
    #[must_use]
    pub fn pdf_last_ximage_pages(&self) -> u32 {
        self.observe_pdf_dependency(DependencyEngineField::PdfExternalImages);
        self.pdf
            .last_external_image()
            .map_or(0, |record| record.metadata().page_count())
    }

    /// Returns the raster bits-per-component reported by the last image.
    #[must_use]
    pub fn pdf_last_ximage_color_depth(&self) -> u8 {
        self.observe_pdf_dependency(DependencyEngineField::PdfExternalImages);
        self.pdf
            .last_external_image()
            .map_or(0, |record| record.metadata().color_depth())
    }

    /// Returns pdfTeX's session-global result value.
    #[must_use]
    pub fn pdf_return_value(&self) -> i32 {
        self.observe_pdf_dependency(DependencyEngineField::PdfObjects);
        self.pdf.return_value()
    }

    /// Updates pdfTeX's session-global result value.
    pub fn set_pdf_return_value(&mut self, value: i32) {
        self.pdf.set_return_value(value);
        self.mark_pdf_dependency_changed(DependencyEngineField::PdfObjects);
    }

    /// Appends expanded tokens to one document-level PDF dictionary destination.
    pub fn append_pdf_document_fragment(
        &mut self,
        kind: PdfDocumentFragmentKind,
        tokens: TokenListId,
    ) {
        let value = self.pdf_token_parameter(tokens);
        self.pdf.append_document_fragment(kind, value);
        self.mark_pdf_dependency_changed(DependencyEngineField::PdfObjects);
    }

    /// Returns document-level fragments of `kind` in source order.
    pub fn pdf_document_fragments(
        &self,
        kind: PdfDocumentFragmentKind,
    ) -> impl Iterator<Item = TokenListId> + '_ {
        self.observe_pdf_dependency(DependencyEngineField::PdfObjects);
        self.pdf.document_fragments(kind)
    }

    #[must_use]
    pub fn pdf_catalog_open_action(&self) -> Option<crate::PdfActionRecord> {
        self.observe_pdf_dependency(DependencyEngineField::PdfObjects);
        self.pdf.catalog_open_action()
    }

    pub fn set_pdf_catalog_open_action(
        &mut self,
        spec: crate::PdfActionSpec,
    ) -> Result<crate::PdfActionRecord, PdfObjectCapacityError> {
        self.set_pdf_catalog_open_action_with_destinations(spec, None, None)
    }

    pub fn set_pdf_catalog_open_action_with_destinations(
        &mut self,
        spec: crate::PdfActionSpec,
        destination_identity: Option<crate::PdfDestinationIdentity>,
        structure_identity: Option<crate::PdfDestinationIdentity>,
    ) -> Result<crate::PdfActionRecord, PdfObjectCapacityError> {
        self.set_pdf_catalog_open_action_with_targets(
            spec,
            destination_identity,
            structure_identity,
            None,
        )
    }

    pub fn set_pdf_catalog_open_action_with_targets(
        &mut self,
        spec: crate::PdfActionSpec,
        destination_identity: Option<crate::PdfDestinationIdentity>,
        structure_identity: Option<crate::PdfDestinationIdentity>,
        thread_identity: Option<crate::PdfDestinationIdentity>,
    ) -> Result<crate::PdfActionRecord, PdfObjectCapacityError> {
        let fingerprint =
            spec.fingerprint(|tokens| self.stores.token_list_semantic_fragment(tokens));
        let result = self.pdf.set_catalog_open_action(
            spec,
            fingerprint,
            destination_identity,
            structure_identity,
            thread_identity,
        );
        if result.is_ok() {
            self.mark_pdf_dependency_changed(DependencyEngineField::PdfObjects);
        }
        result
    }

    /// Allocates final document dictionaries through the canonical PDF ledger.
    pub fn finalize_pdf_document_objects(
        &mut self,
        include_info: bool,
    ) -> Result<PdfDocumentObjectIds, PdfObjectCapacityError> {
        let result = self.pdf.finalize_document_objects(include_info);
        if result.is_ok() {
            self.mark_pdf_dependency_changed(DependencyEngineField::PdfObjects);
        }
        result
    }

    fn current_pdf_output_parameters(&self) -> PdfOutputParameters {
        PdfOutputParameters {
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

    fn current_pdf_token_parameter(&self, parameter: TokParam) -> PdfTokenParameter {
        let tokens = self.tok_param(parameter);
        self.pdf_token_parameter(tokens)
    }

    fn pdf_token_parameter(&self, tokens: TokenListId) -> PdfTokenParameter {
        PdfTokenParameter {
            tokens: self.stores.token_list_ref(tokens),
            semantic_id: self.stores.token_list_semantic_fragment(tokens),
        }
    }

    fn current_pdf_page_parameters(&self) -> PdfPageParameters {
        self.observe_pdf_dependency(DependencyEngineField::PdfObjects);
        PdfPageParameters {
            h_origin: self.dimen_param(DimenParam::PDF_H_ORIGIN),
            v_origin: self.dimen_param(DimenParam::PDF_V_ORIGIN),
            width: self.dimen_param(DimenParam::PDF_PAGE_WIDTH),
            height: self.dimen_param(DimenParam::PDF_PAGE_HEIGHT),
            link_margin: self.dimen_param(DimenParam::PDF_LINK_MARGIN),
            page_attr: self.current_pdf_token_parameter(TokParam::PDF_PAGE_ATTR),
            resources: self.current_pdf_token_parameter(TokParam::PDF_PAGE_RESOURCES),
            omit_procset: self.int_param(IntParam::PDF_OMIT_PROCSET),
            space_font_name: self.pdf.current_space_font_name_id(),
        }
    }

    /// Returns the current code-table generation vector.
    #[must_use]
    pub fn code_table_generations(&self) -> CodeTableGenerations {
        self.stores.code_table_generations()
    }

    #[must_use]
    pub fn catcode(&self, ch: char) -> Catcode {
        self.observe_semantic_dependency(DependencyKey::Code {
            table: DependencyCodeTable::Catcode,
            scalar: ch.into(),
        });
        self.stores.catcode(ch)
    }

    pub fn set_catcode(&mut self, ch: char, value: Catcode) {
        self.stores.set_catcode(ch, value);
        self.mark_code_changed(DependencyCodeTable::Catcode, ch);
    }

    pub fn set_catcode_global(&mut self, ch: char, value: Catcode) {
        self.stores.set_catcode_global(ch, value);
        self.mark_code_changed(DependencyCodeTable::Catcode, ch);
    }

    #[must_use]
    pub fn lccode(&self, ch: char) -> LcCode {
        self.observe_semantic_dependency(DependencyKey::Code {
            table: DependencyCodeTable::Lccode,
            scalar: ch.into(),
        });
        self.stores.lccode(ch)
    }

    pub fn set_lccode(&mut self, ch: char, value: LcCode) {
        self.stores.set_lccode(ch, value);
        self.mark_code_changed(DependencyCodeTable::Lccode, ch);
    }

    pub fn set_lccode_global(&mut self, ch: char, value: LcCode) {
        self.stores.set_lccode_global(ch, value);
        self.mark_code_changed(DependencyCodeTable::Lccode, ch);
    }

    #[must_use]
    pub fn uccode(&self, ch: char) -> UcCode {
        self.observe_semantic_dependency(DependencyKey::Code {
            table: DependencyCodeTable::Uccode,
            scalar: ch.into(),
        });
        self.stores.uccode(ch)
    }

    pub fn set_uccode(&mut self, ch: char, value: UcCode) {
        self.stores.set_uccode(ch, value);
        self.mark_code_changed(DependencyCodeTable::Uccode, ch);
    }

    pub fn set_uccode_global(&mut self, ch: char, value: UcCode) {
        self.stores.set_uccode_global(ch, value);
        self.mark_code_changed(DependencyCodeTable::Uccode, ch);
    }

    #[must_use]
    pub fn sfcode(&self, ch: char) -> SfCode {
        self.observe_semantic_dependency(DependencyKey::Code {
            table: DependencyCodeTable::Sfcode,
            scalar: ch.into(),
        });
        self.stores.sfcode(ch)
    }

    pub fn set_sfcode(&mut self, ch: char, value: SfCode) {
        self.stores.set_sfcode(ch, value);
        self.mark_code_changed(DependencyCodeTable::Sfcode, ch);
    }

    pub fn set_sfcode_global(&mut self, ch: char, value: SfCode) {
        self.stores.set_sfcode_global(ch, value);
        self.mark_code_changed(DependencyCodeTable::Sfcode, ch);
    }

    #[must_use]
    pub fn mathcode(&self, ch: char) -> MathCode {
        self.observe_semantic_dependency(DependencyKey::Code {
            table: DependencyCodeTable::Mathcode,
            scalar: ch.into(),
        });
        self.stores.mathcode(ch)
    }

    pub fn set_mathcode(&mut self, ch: char, value: MathCode) {
        self.stores.set_mathcode(ch, value);
        self.mark_code_changed(DependencyCodeTable::Mathcode, ch);
    }

    pub fn set_mathcode_global(&mut self, ch: char, value: MathCode) {
        self.stores.set_mathcode_global(ch, value);
        self.mark_code_changed(DependencyCodeTable::Mathcode, ch);
    }

    #[must_use]
    pub fn delcode(&self, ch: char) -> DelCode {
        self.observe_semantic_dependency(DependencyKey::Code {
            table: DependencyCodeTable::Delcode,
            scalar: ch.into(),
        });
        self.stores.delcode(ch)
    }

    pub fn set_delcode(&mut self, ch: char, value: DelCode) {
        self.stores.set_delcode(ch, value);
        self.mark_code_changed(DependencyCodeTable::Delcode, ch);
    }

    pub fn set_delcode_global(&mut self, ch: char, value: DelCode) {
        self.stores.set_delcode_global(ch, value);
        self.mark_code_changed(DependencyCodeTable::Delcode, ch);
    }

    pub fn add_hyphenation_pattern(
        &mut self,
        pattern: PatternSpec,
    ) -> Result<(), crate::hyphenation::HyphenationCapacityError> {
        self.stores.add_hyphenation_pattern(pattern)?;
        self.dependencies
            .get_mut()
            .expect("dependency runtime mutex is not poisoned")
            .mark_changed(DependencyKey::HyphenationPatterns(0));
        Ok(())
    }

    pub fn add_hyphenation_pattern_for_language(
        &mut self,
        language: u8,
        pattern: PatternSpec,
    ) -> Result<bool, crate::hyphenation::HyphenationCapacityError> {
        let duplicate = self
            .stores
            .add_hyphenation_pattern_for_language(language, pattern)?;
        self.dependencies
            .get_mut()
            .expect("dependency runtime mutex is not poisoned")
            .mark_changed(DependencyKey::HyphenationPatterns(language));
        Ok(duplicate)
    }

    /// Overrides pdfTeX's runtime `trie_size` before loading patterns.
    pub fn set_hyphenation_trie_capacity(&mut self, capacity: usize) {
        self.stores.set_hyphenation_trie_capacity(capacity);
    }

    /// Selects tex.web §934's profile-owned `hyph_size`.
    pub fn set_hyphenation_exception_capacity(&mut self, capacity: usize) {
        self.stores.set_hyphenation_exception_capacity(capacity);
    }

    #[must_use]
    pub fn contains_hyphenation_pattern_for_language(
        &self,
        language: u8,
        letters: &[char],
    ) -> bool {
        self.observe_semantic_dependency(DependencyKey::HyphenationPatterns(language));
        self.stores
            .contains_hyphenation_pattern_for_language(language, letters)
    }

    /// Reports TeX82 §960's live `trie_not_ready` state.
    #[must_use]
    pub fn hyphenation_patterns_open(&self) -> bool {
        self.stores.hyphenation_patterns_open()
    }

    /// Performs the one-way semantic part of TeX82 §919's `init_trie`.
    pub fn close_hyphenation_patterns(&mut self) {
        self.stores.close_hyphenation_patterns();
    }

    pub fn add_hyphenation_exception(&mut self, exception: ExceptionSpec) {
        self.stores.add_hyphenation_exception(exception);
        self.dependencies
            .get_mut()
            .expect("dependency runtime mutex is not poisoned")
            .mark_changed(DependencyKey::HyphenationExceptions(0));
    }

    pub fn add_hyphenation_exception_for_language(
        &mut self,
        language: u8,
        exception: ExceptionSpec,
    ) {
        self.stores
            .add_hyphenation_exception_for_language(language, exception);
        self.dependencies
            .get_mut()
            .expect("dependency runtime mutex is not poisoned")
            .mark_changed(DependencyKey::HyphenationExceptions(language));
    }

    pub fn save_hyphenation_codes(
        &mut self,
        language: u8,
        codes: impl IntoIterator<Item = (char, char)>,
    ) {
        self.stores.save_hyphenation_codes(language, codes);
        self.dependencies
            .get_mut()
            .expect("dependency runtime mutex is not poisoned")
            .mark_changed(DependencyKey::HyphenationCodes(language));
    }

    #[must_use]
    pub fn saved_hyphenation_code(&self, language: u8, ch: char) -> Option<Option<char>> {
        self.observe_semantic_dependency(DependencyKey::HyphenationCodes(language));
        self.stores.saved_hyphenation_code(language, ch)
    }

    #[must_use]
    pub fn hyphen_positions(&self, word: &str, left_min: usize, right_min: usize) -> Vec<usize> {
        self.observe_semantic_dependency(DependencyKey::HyphenationPatterns(0));
        self.observe_semantic_dependency(DependencyKey::HyphenationExceptions(0));
        self.stores.hyphen_positions(word, left_min, right_min)
    }

    #[must_use]
    pub fn hyphen_positions_for_language(
        &self,
        language: u8,
        word: &str,
        left_min: usize,
        right_min: usize,
    ) -> Vec<usize> {
        self.observe_semantic_dependency(DependencyKey::HyphenationPatterns(language));
        self.observe_semantic_dependency(DependencyKey::HyphenationExceptions(language));
        self.observe_semantic_dependency(DependencyKey::HyphenationCodes(language));
        self.stores
            .hyphen_positions_for_language(language, word, left_min, right_min)
    }

    #[must_use]
    pub fn hyphenation_exception(&self, word: &str) -> Option<&[usize]> {
        self.observe_semantic_dependency(DependencyKey::HyphenationExceptions(0));
        self.stores.hyphenation_exception(word)
    }

    #[must_use]
    pub fn meaning(&self, symbol: impl crate::interner::SymbolReference) -> Meaning {
        self.stores.meaning(symbol)
    }

    pub fn set_meaning(&mut self, symbol: impl crate::interner::SymbolReference, meaning: Meaning) {
        let receipt = self.stores.set_meaning(symbol, meaning);
        self.consume_env_mutation(receipt);
    }

    pub fn intern_relaxed_control_sequence(&mut self, name: &str) -> SymbolId {
        let (symbol, receipt) = self
            .stores
            .intern_relaxed_control_sequence_with_receipt(name);
        if let Some(receipt) = receipt {
            self.consume_env_mutation(receipt);
        }
        symbol
    }

    pub fn set_meaning_global(
        &mut self,
        symbol: impl crate::interner::SymbolReference,
        meaning: Meaning,
    ) {
        let receipt = self.stores.set_meaning_global(symbol, meaning);
        self.consume_env_mutation(receipt);
    }

    pub fn intern_macro(&mut self, macro_meaning: MacroMeaning) -> MacroDefinitionRef {
        self.stores.intern_macro_with_provenance_in_domain(
            macro_meaning,
            None,
            self.private_revision_domain.as_mut(),
        )
    }

    pub fn intern_macro_with_provenance(
        &mut self,
        macro_meaning: MacroMeaning,
        provenance: MacroDefinitionProvenance,
    ) -> MacroDefinitionRef {
        self.stores.intern_macro_with_provenance_in_domain(
            macro_meaning,
            Some(provenance),
            self.private_revision_domain.as_mut(),
        )
    }

    #[must_use]
    pub fn macro_definition_ref(&self, id: MacroDefinitionId) -> MacroDefinitionRef {
        self.stores.macro_definition_ref(id)
    }

    #[doc(hidden)]
    #[must_use]
    pub fn packed_macro_owner(
        &self,
        id: MacroDefinitionId,
    ) -> crate::macro_store::PackedMacroChunkOwner {
        self.stores.packed_macro_owner(id)
    }

    /// Reads the current packed meaning for a definition already rooted by
    /// live command state, without entering the weak value index.
    #[doc(hidden)]
    #[must_use]
    pub fn packed_macro_meaning(&self, id: MacroDefinitionId) -> Option<MacroMeaning> {
        self.stores.packed_macro_meaning(id)
    }

    #[must_use]
    pub fn macro_definition(&self, id: MacroDefinitionId) -> MacroMeaning {
        self.stores.macro_definition(id)
    }

    #[must_use]
    pub fn macro_definition_observation_operand(&self, id: MacroDefinitionId) -> i64 {
        self.stores.macro_definition_observation_operand(id)
    }

    #[doc(hidden)]
    #[must_use]
    pub fn packed_macro_observation_operand(&self, id: MacroDefinitionId) -> Option<i64> {
        self.stores.packed_macro_observation_operand(id)
    }

    #[must_use]
    pub fn macro_definition_parameter_pattern(
        &self,
        id: MacroDefinitionId,
    ) -> crate::macro_store::MacroParameterPattern {
        self.stores.macro_definition_parameter_pattern(id)
    }

    #[must_use]
    pub fn macro_definition_provenance(&self, id: MacroDefinitionId) -> MacroDefinitionProvenance {
        self.stores.macro_definition_provenance(id)
    }

    #[doc(hidden)]
    pub fn macro_definition_provenance_roots(
        &self,
        id: MacroDefinitionId,
    ) -> Option<(OriginRef, OriginListRef, OriginListRef)> {
        self.stores.macro_definition_provenance_roots(id)
    }

    /// Attaches provenance after a definition's semantic body has been interned.
    ///
    /// Detached continuation import uses this two-phase operation to break the
    /// legitimate definition -> provenance -> invocation -> definition cycle.
    pub fn set_macro_definition_provenance(
        &mut self,
        id: MacroDefinitionId,
        provenance: MacroDefinitionProvenance,
    ) {
        self.stores.set_macro_definition_provenance(id, provenance);
    }

    pub fn set_macro_meaning(
        &mut self,
        symbol: impl crate::interner::SymbolReference,
        macro_meaning: MacroMeaning,
    ) {
        let receipt = self.stores.set_macro_meaning(symbol, macro_meaning);
        self.consume_env_mutation(receipt);
    }

    pub fn set_macro_meaning_with_provenance(
        &mut self,
        symbol: impl crate::interner::SymbolReference,
        macro_meaning: MacroMeaning,
        provenance: MacroDefinitionProvenance,
    ) {
        let receipt =
            self.stores
                .set_macro_meaning_with_provenance(symbol, macro_meaning, provenance);
        self.consume_env_mutation(receipt);
    }

    pub fn set_macro_meaning_global(
        &mut self,
        symbol: impl crate::interner::SymbolReference,
        macro_meaning: MacroMeaning,
    ) {
        let receipt = self.stores.set_macro_meaning_global(symbol, macro_meaning);
        self.consume_env_mutation(receipt);
    }

    pub fn set_macro_meaning_global_with_provenance(
        &mut self,
        symbol: impl crate::interner::SymbolReference,
        macro_meaning: MacroMeaning,
        provenance: MacroDefinitionProvenance,
    ) {
        let receipt =
            self.stores
                .set_macro_meaning_global_with_provenance(symbol, macro_meaning, provenance);
        self.consume_env_mutation(receipt);
    }

    /// Installs an ordinary scanned macro occurrence from its existing token
    /// owners, bypassing weak resolution and cold exact-content indexes.
    pub fn set_macro_meaning_from_traced(
        &mut self,
        symbol: impl crate::interner::SymbolReference,
        flags: crate::meaning::MeaningFlags,
        parameter_text: &TracedTokenList,
        replacement_text: &TracedTokenList,
        provenance: MacroDefinitionProvenance,
        global: bool,
    ) {
        let receipt = self.stores.set_macro_meaning_from_traced(
            symbol,
            flags,
            parameter_text,
            replacement_text,
            provenance,
            global,
        );
        self.consume_env_mutation(receipt);
    }

    /// Installs a scanner-completed macro directly into the dense runtime
    /// arenas without first freezing exact token-list values.
    pub fn set_macro_meaning_from_buffers(
        &mut self,
        symbol: impl crate::interner::SymbolReference,
        flags: crate::meaning::MeaningFlags,
        parameter_text: &crate::token::RootedTracedTokenBuffer,
        replacement_text: &crate::token::RootedTracedTokenBuffer,
        definition_origin: crate::provenance::OriginRef,
        global: bool,
    ) {
        let receipt = self.stores.set_macro_meaning_from_buffers(
            symbol,
            flags,
            parameter_text,
            replacement_text,
            definition_origin,
            global,
        );
        self.consume_env_mutation(receipt);
    }

    #[must_use]
    pub fn macro_meaning(
        &self,
        symbol: impl crate::interner::SymbolReference,
    ) -> Option<MacroMeaning> {
        self.stores.macro_meaning(symbol)
    }

    pub fn intern(&mut self, name: &str) -> SymbolId {
        self.stores.intern(name)
    }

    /// Interns a spelling through TeX82 §259's hash-table path.
    pub fn intern_hash_control_sequence(&mut self, name: &str) -> SymbolId {
        self.stores
            .try_intern_hash(name)
            .expect("control-sequence symbol capacity exceeded")
    }

    /// Interns an active-character control sequence in its TeX82 namespace.
    pub fn intern_active_character(&mut self, ch: char) -> SymbolId {
        self.stores.intern_active_character(ch)
    }

    pub fn intern_internal_control_sequence(&mut self, name: &str) -> SymbolId {
        self.stores.intern_internal_control_sequence(name)
    }

    #[must_use]
    pub fn symbol(&self, name: &str) -> Option<SymbolId> {
        self.stores.symbol(name)
    }

    /// Returns the live symbol for an already-interned active character.
    #[must_use]
    pub fn active_character_symbol(&self, ch: char) -> Option<SymbolId> {
        self.stores.active_character_symbol(ch)
    }

    #[must_use]
    pub fn resolve(&self, symbol: impl crate::interner::SymbolReference) -> &str {
        self.stores.resolve(symbol)
    }

    /// Returns the TeX control-sequence namespace of a live symbol.
    #[must_use]
    pub fn control_sequence_kind(
        &self,
        symbol: impl crate::interner::SymbolReference,
    ) -> ControlSequenceKind {
        self.stores.control_sequence_kind(symbol)
    }

    #[must_use]
    pub fn token_list_builder(&self) -> TokenListBuilder {
        self.stores.token_list_builder()
    }

    #[cfg(any(test, feature = "testing"))]
    pub fn intern_token_list(&mut self, tokens: &[Token]) -> TokenListId {
        self.stores
            .intern_token_list_in_domain(tokens, self.private_revision_domain.as_mut())
    }

    /// Interns a token list and returns its strong exact-content owner.
    pub fn intern_token_list_ref(&mut self, tokens: &[Token]) -> TokenListRef {
        self.stores
            .intern_token_list_ref_in_domain(tokens, self.private_revision_domain.as_mut())
    }

    #[cfg(any(test, feature = "testing"))]
    pub fn finish_token_list(&mut self, builder: &mut TokenListBuilder) -> TokenListId {
        self.stores
            .finish_token_list_in_domain(builder, self.private_revision_domain.as_mut())
    }

    /// Freezes paired semantic tokens and diagnostic origins through the
    /// aggregate state boundary.
    pub fn finish_traced_token_list(&mut self, tokens: &[TracedTokenWord]) -> TracedTokenList {
        self.stores
            .finish_traced_token_list_in_domain(tokens, self.private_revision_domain.as_mut())
    }

    /// Freezes a structurally rooted transient buffer without consulting the
    /// provenance arena for ownership.
    pub fn finish_rooted_traced_token_list(
        &mut self,
        tokens: &crate::token::RootedTracedTokenBuffer,
    ) -> TracedTokenList {
        self.stores.finish_rooted_traced_token_list_in_domain(
            tokens,
            self.private_revision_domain.as_mut(),
        )
    }

    #[must_use]
    pub fn tokens(&self, id: TokenListId) -> TokenListRef {
        self.stores.tokens(id)
    }

    /// Clones the strong exact-content owner for a live token coordinate.
    #[must_use]
    pub fn token_list_ref(&self, id: TokenListId) -> TokenListRef {
        self.stores.token_list_ref(id)
    }

    /// Returns the reserved unknown/bootstrap provenance origin.
    #[must_use]
    pub fn bootstrap_origin(&self) -> OriginId {
        self.stores.bootstrap_origin()
    }

    /// Allocates a source-coordinate origin.
    pub fn source_origin(
        &mut self,
        source: SourceId,
        byte_offset: u64,
        line: u32,
        column: u32,
    ) -> OriginId {
        self.stores.source_origin(source, byte_offset, line, column)
    }

    /// Allocates a source-coordinate origin bound to its durable input record.
    pub fn source_origin_with_input_record(
        &mut self,
        source: SourceId,
        input_record: Option<crate::InputRecordId>,
        byte_offset: u64,
        line: u32,
        column: u32,
    ) -> OriginId {
        self.stores
            .source_origin_with_input_record(source, input_record, byte_offset, line, column)
    }

    /// Returns best-effort provenance for an ordinary backed source scalar.
    pub fn source_token_origin(
        &mut self,
        source: SourceId,
        byte_offset: u64,
        byte_end: u64,
    ) -> OriginId {
        self.stores
            .source_token_origin(source, byte_offset, byte_end)
    }

    /// Allocates an exact validated half-open source spelling range.
    pub fn source_range_origin(
        &mut self,
        source: SourceId,
        byte_offset: u64,
        byte_end: u64,
    ) -> OriginId {
        self.stores
            .source_range_origin(source, byte_offset, byte_end)
    }

    /// Allocates an origin for a range validated by `RegisteredSource`.
    pub fn source_span_origin(&mut self, span: SourceSpan) -> OriginId {
        self.stores.source_span_origin(span)
    }

    pub fn source_span_origin_ref(&mut self, span: SourceSpan) -> OriginRef {
        self.stores.source_span_origin_ref(span)
    }

    pub fn source_token_origin_ref(
        &mut self,
        source: SourceId,
        byte_offset: u64,
        byte_end: u64,
    ) -> OriginRef {
        self.stores
            .source_token_origin_ref(source, byte_offset, byte_end)
    }

    pub fn source_range_origin_ref(
        &mut self,
        source: SourceId,
        byte_offset: u64,
        byte_end: u64,
    ) -> OriginRef {
        self.stores
            .source_range_origin_ref(source, byte_offset, byte_end)
    }

    /// Allocates a macro-invocation origin.
    pub fn macro_invocation_origin(
        &mut self,
        definition: MacroDefinitionId,
        invocation: OriginId,
        definition_origin: OriginId,
        parent_invocation: OriginId,
    ) -> OriginId {
        self.stores.macro_invocation_origin(
            definition,
            invocation,
            definition_origin,
            parent_invocation,
        )
    }

    pub fn macro_invocation_frame(
        &mut self,
        definition: MacroDefinitionId,
        invocation: OriginRef,
        definition_origin: OriginRef,
        parent_invocation: OriginRef,
    ) -> ExpansionFrameRef {
        self.stores.macro_invocation_frame(
            definition,
            invocation,
            definition_origin,
            parent_invocation,
        )
    }

    #[doc(hidden)]
    pub fn macro_invocation_frame_from_nonowning_operand(
        &mut self,
        definition_operand: u64,
        invocation: OriginRef,
        definition_origin: OriginRef,
        parent_invocation: OriginRef,
    ) -> ExpansionFrameRef {
        self.stores.macro_invocation_frame_from_nonowning_operand(
            definition_operand,
            invocation,
            definition_origin,
            parent_invocation,
        )
    }

    #[doc(hidden)]
    pub fn macro_invocation_origin_from_nonowning_operand(
        &mut self,
        definition_operand: u64,
        invocation: OriginId,
        definition_origin: OriginId,
        parent_invocation: OriginId,
    ) -> OriginId {
        self.stores.macro_invocation_origin_from_nonowning_operand(
            definition_operand,
            invocation,
            definition_origin,
            parent_invocation,
        )
    }

    /// Allocates an inserted-token origin.
    pub fn inserted_origin(
        &mut self,
        kind: InsertedOriginKind,
        token: Token,
        parent: OriginId,
    ) -> OriginId {
        self.stores.inserted_origin(kind, token, parent)
    }

    pub fn inserted_origin_ref(
        &mut self,
        kind: InsertedOriginKind,
        token: Token,
        parent: OriginRef,
    ) -> OriginRef {
        self.stores.inserted_origin_ref(kind, token, parent)
    }

    /// Allocates a synthesized-token origin.
    pub fn synthesized_origin(
        &mut self,
        kind: SynthesizedOriginKind,
        parent: OriginId,
    ) -> OriginId {
        self.stores.synthesized_origin(kind, parent)
    }

    pub fn synthesized_origin_ref(
        &mut self,
        kind: SynthesizedOriginKind,
        parent: OriginRef,
    ) -> OriginRef {
        self.stores.synthesized_origin_ref(kind, parent)
    }

    /// Allocates a synthetic/bootstrap origin.
    pub fn synthetic_origin(&mut self, kind: SyntheticOriginKind) -> OriginId {
        self.stores.synthetic_origin(kind)
    }

    pub fn synthetic_origin_ref(&mut self, kind: SyntheticOriginKind) -> OriginRef {
        self.stores.synthetic_origin_ref(kind)
    }

    #[must_use]
    pub fn origin_ref(&self, id: OriginId) -> Option<OriginRef> {
        self.stores.origin_ref(id)
    }

    pub fn materialize_origin_ref(&mut self, id: OriginId) -> Option<OriginRef> {
        self.stores.materialize_origin_ref(id)
    }

    /// Reads a live origin record.
    #[must_use]
    pub fn origin(&self, id: OriginId) -> OriginRecord {
        self.origin_if_live(id)
            .expect("origin id is not live in this Universe timeline")
    }

    /// Reads an origin record if it is still live on this timeline.
    #[must_use]
    pub fn origin_if_live(&self, id: OriginId) -> Option<OriginRecord> {
        if let crate::token::OriginEncoding::DirectSource(position) = id.decode() {
            if let Some(span) = self.stores.direct_fragment_origin_span(id) {
                return Some(OriginRecord::SourceSpan(span));
            }
            let source = self.stores.source_origin_at_position(position)?;
            let region = self.stores.source_region(source.source())?;
            let bytes = self.source_backing_bytes(region)?;
            let offset = usize::try_from(source.byte_offset()).ok()?;
            let scalar_len = utf8_scalar_len_at(bytes, offset)?;
            let hi = self
                .stores
                .source_position(
                    source.source(),
                    source.byte_offset().checked_add(scalar_len as u64)?,
                )
                .ok()?;
            return self
                .stores
                .source_span(position, hi)
                .ok()
                .map(OriginRecord::SourceSpan);
        }
        self.stores.origin_if_live(id)
    }

    pub fn allocate_origin_list_ref(&mut self, origins: &[OriginRef]) -> OriginListRef {
        self.stores.allocate_origin_list_ref(origins)
    }

    /// Returns live provenance arena length counters.
    #[must_use]
    pub fn provenance_stats(&self) -> ProvenanceStats {
        self.stores.provenance_stats()
    }

    /// Computes on-demand retained-memory accounting for live macro
    /// invocation provenance without adding expansion-path counters.
    #[must_use]
    pub fn macro_invocation_provenance_stats(&self) -> MacroInvocationProvenanceStats {
        self.stores.macro_invocation_provenance_stats()
    }

    /// Returns live macro-invocation origins in allocation order for
    /// rollback and replay tests.
    ///
    /// This is a test-only timeline inspection escape hatch, not an engine
    /// capability. Production code must resolve provenance from tokens and
    /// diagnostics rather than enumerate the arena.
    #[cfg(any(test, feature = "testing"))]
    #[must_use]
    pub fn macro_invocation_origins_for_testing(&self) -> Vec<OriginId> {
        self.stores.macro_invocation_origins()
    }

    /// Registers a source backing after validating any World identity.
    pub fn register_source(
        &mut self,
        source: SourceId,
        descriptor: SourceDescriptor,
    ) -> Result<SourcePos, SourceMapError> {
        if let SourceDescriptor::World {
            input_record,
            byte_len,
        } = descriptor
        {
            let record = self
                .world
                .input_record(input_record)
                .ok_or(SourceMapError::MissingWorldInput)?;
            if u64::try_from(record.len()).ok() != Some(byte_len) {
                return Err(SourceMapError::WorldInputLengthMismatch);
            }
            if let Some(position) = self
                .stores
                .existing_source_registration(source, &descriptor)?
            {
                return Ok(position);
            }
            let bytes = self
                .world
                .input_content(record.hash())
                .ok_or(SourceMapError::MissingWorldInput)?;
            let line_starts = source_line_starts(bytes);
            return self.stores.register_source(
                source,
                SourceDescriptor::world(input_record, byte_len),
                line_starts,
            );
        }
        let SourceDescriptor::Generated(generated) = &descriptor else {
            unreachable!("world source handled above")
        };
        if let Some(position) = self
            .stores
            .existing_source_registration(source, &descriptor)?
        {
            return Ok(position);
        }
        let line_starts = source_line_starts(generated.bytes());
        self.stores.register_source(source, descriptor, line_starts)
    }

    /// Registers a source and returns an opaque capability used by its input
    /// frame to encode ordinary direct origins without repeated map lookup.
    pub fn register_input_source(
        &mut self,
        source: SourceId,
        descriptor: SourceDescriptor,
    ) -> Result<RegisteredSource, SourceMapError> {
        let byte_len = descriptor.byte_len();
        let start = self.register_source(source, descriptor)?;
        Ok(RegisteredSource::new(start, byte_len))
    }

    /// Resolves a source-local physical byte offset into logical space.
    pub fn source_position(
        &self,
        source: SourceId,
        byte_offset: u64,
    ) -> Result<SourcePos, SourceMapError> {
        self.stores.source_position(source, byte_offset)
    }

    /// Validates a half-open logical source span.
    pub fn source_span(&self, lo: SourcePos, hi: SourcePos) -> Result<SourceSpan, SourceMapError> {
        self.stores.source_span(lo, hi)
    }

    /// Copies the logical registration recipe for detached command state.
    ///
    /// The returned descriptor owns semantic backing data, never the live
    /// source-map registration root.
    #[doc(hidden)]
    #[must_use]
    pub fn detached_source_descriptor(&self, source: SourceId) -> Option<SourceDescriptor> {
        self.stores.source_descriptor(source)
    }

    /// Copies a World input record into allocation-independent detached data.
    #[doc(hidden)]
    #[must_use]
    pub fn detached_world_input(
        &self,
        input_record: crate::InputRecordId,
    ) -> Option<(
        std::path::PathBuf,
        Vec<u8>,
        Option<crate::FileModificationDate>,
        crate::InputOrigin,
    )> {
        let content = self.world.recorded_input_content(input_record)?;
        Some((
            content.path().to_owned(),
            content.bytes().to_vec(),
            content.modification_date(),
            content.origin(),
        ))
    }

    /// Installs one detached World backing with a destination-local input id.
    #[doc(hidden)]
    pub fn install_detached_world_source(
        &mut self,
        source: SourceId,
        path: std::path::PathBuf,
        bytes: std::sync::Arc<[u8]>,
        modification_date: Option<crate::FileModificationDate>,
        origin: crate::InputOrigin,
    ) -> Result<SourcePos, SourceMapError> {
        if let Some(descriptor) = self.stores.source_descriptor(source) {
            let SourceDescriptor::World { input_record, .. } = descriptor else {
                return Err(SourceMapError::ConflictingRegistration);
            };
            let Some(existing) = self.world.recorded_input_content(input_record) else {
                return Err(SourceMapError::MissingWorldInput);
            };
            if existing.path() != path
                || existing.bytes() != bytes.as_ref()
                || existing.modification_date() != modification_date
                || existing.origin() != origin
            {
                return Err(SourceMapError::ConflictingRegistration);
            }
            return self.register_source(source, descriptor);
        }
        let byte_len =
            u64::try_from(bytes.len()).map_err(|_| SourceMapError::WorldInputLengthMismatch)?;
        let input_record =
            self.world
                .register_detached_input_content(path, bytes, modification_date, origin);
        self.register_source(source, SourceDescriptor::world(input_record, byte_len))
    }

    /// Converts an allocation-local logical span to a source-relative recipe.
    #[doc(hidden)]
    #[must_use]
    pub fn detached_source_span(&self, span: SourceSpan) -> Option<(SourceId, u64, u64)> {
        let region = self.stores.source_region_at_position(span.lo())?;
        if span.hi().raw() > region.anchor().raw() {
            return None;
        }
        Some((
            region.source,
            span.lo().raw().checked_sub(region.start.raw())?,
            span.hi().raw().checked_sub(region.start.raw())?,
        ))
    }

    pub(crate) fn source_region(&self, source: SourceId) -> Option<SourceRegion> {
        self.stores.source_region(source)
    }

    pub(crate) fn source_region_at_position(&self, position: SourcePos) -> Option<SourceRegion> {
        self.stores.source_region_at_position(position)
    }

    pub(crate) fn source_line_starts(&self, region: SourceRegion) -> Option<&[usize]> {
        self.stores.source_line_starts(region)
    }

    pub(crate) fn source_backing_bytes(&self, region: SourceRegion) -> Option<&[u8]> {
        match region.backing {
            SourceBacking::World(record_id) => {
                let record = self.world.input_record(record_id)?;
                self.world.input_content(record.hash())
            }
            SourceBacking::Generated(_) => self
                .stores
                .generated_source(region.backing)
                .map(GeneratedSource::bytes),
        }
    }

    pub(crate) fn generated_source(&self, backing: SourceBacking) -> Option<&GeneratedSource> {
        self.stores.generated_source(backing)
    }

    pub(crate) fn direct_source_origin(
        &self,
        origin: OriginId,
    ) -> Option<crate::provenance::SourceOrigin> {
        self.stores.direct_source_origin(origin)
    }

    /// Tests an inserted-origin classification without resolving source origins.
    #[must_use]
    pub fn origin_is_inserted_kind(&self, id: OriginId, kind: InsertedOriginKind) -> bool {
        match id.decode() {
            crate::token::OriginEncoding::NoExpandFallback => kind == InsertedOriginKind::NoExpand,
            crate::token::OriginEncoding::DirectSource(_)
            | crate::token::OriginEncoding::Unknown => false,
            crate::token::OriginEncoding::Arena(_) => match self.stores.origin_if_live(id) {
                Some(OriginRecord::Inserted(inserted)) => inserted.kind() == kind,
                Some(_) => false,
                None => panic!("origin id is not live in this Universe timeline"),
            },
        }
    }

    pub(crate) fn source_origin_at_position(
        &self,
        position: SourcePos,
    ) -> Option<crate::provenance::SourceOrigin> {
        self.stores.source_origin_at_position(position)
    }

    pub fn intern_glue(&mut self, spec: GlueSpec) -> GlueSpecRef {
        self.stores
            .intern_glue_in_domain(spec, self.private_revision_domain.as_mut())
    }

    #[cfg(any(test, feature = "testing"))]
    #[allow(dead_code)]
    pub(crate) fn testing_intern_glue(&mut self, spec: GlueSpec) -> GlueId {
        self.stores.intern_glue(spec)
    }

    #[must_use]
    pub fn glue_ref(&self, id: GlueId) -> GlueSpecRef {
        self.stores.glue_ref(id)
    }

    #[must_use]
    pub fn glue(&self, id: impl crate::glue::GlueHandle) -> GlueSpec {
        self.stores.glue(id.glue_id())
    }

    pub fn intern_font(&mut self, font: LoadedFont) -> FontId {
        self.stores.intern_font(font)
    }

    pub fn try_intern_font(&mut self, font: LoadedFont) -> Result<FontId, FontParameterError> {
        self.stores.try_intern_font(font)
    }

    pub fn intern_font_with_identifier(
        &mut self,
        font: LoadedFont,
        symbol: impl crate::interner::SymbolReference,
    ) -> FontId {
        self.stores.intern_font_with_identifier(font, symbol)
    }

    pub fn try_intern_font_with_identifier(
        &mut self,
        font: LoadedFont,
        symbol: impl crate::interner::SymbolReference,
    ) -> Result<FontId, FontParameterError> {
        self.stores.try_intern_font_with_identifier(font, symbol)
    }

    pub fn try_copy_font_with_identifier(
        &mut self,
        source: FontId,
        symbol: impl crate::interner::SymbolReference,
    ) -> Result<FontId, FontParameterError> {
        self.stores.try_copy_font_with_identifier(source, symbol)
    }

    pub fn try_letterspace_font_with_identifier(
        &mut self,
        source: FontId,
        symbol: impl crate::interner::SymbolReference,
        amount: i16,
        no_ligatures: bool,
    ) -> Result<FontId, FontParameterError> {
        self.stores
            .try_letterspace_font_with_identifier(source, symbol, amount, no_ligatures)
    }

    pub fn configure_font_expansion(
        &mut self,
        font: FontId,
        expansion: crate::font::FontExpansion,
    ) -> Result<(), crate::font::FontExpansionConfigError> {
        self.stores.configure_font_expansion(font, expansion)
    }

    #[must_use]
    pub fn font_expansion(&self, font: FontId) -> Option<crate::font::FontExpansion> {
        self.observe_font_dependency(font, DependencyFontField::Metrics, 0);
        self.stores.font_expansion(font)
    }

    pub fn try_expanded_font(
        &mut self,
        source: FontId,
        ratio: i16,
    ) -> Result<FontId, FontParameterError> {
        self.stores.try_expanded_font(source, ratio)
    }

    #[must_use]
    pub fn font(&self, id: FontId) -> &LoadedFont {
        self.observe_font_dependency(id, DependencyFontField::Metrics, 0);
        self.stores.font(id)
    }

    /// Captures one font selection without its owner-bound runtime id.
    pub fn detach_font(
        &self,
        id: FontId,
    ) -> Result<crate::DetachedMemoValue, crate::MemoValueError> {
        let payload = self
            .stores
            .encode_memo_font(id)
            .map_err(|error| crate::MemoValueError::Codec(format!("{error:?}")))?;
        Ok(crate::DetachedMemoValue::from_payload(
            crate::MemoValueKind::Font,
            payload,
        ))
    }

    /// Imports a detached font through the aggregate owner boundary.
    pub fn import_memo_font(
        &mut self,
        value: &crate::DetachedMemoValue,
        limits: crate::MemoValueLimits,
    ) -> Result<FontId, crate::MemoValueError> {
        let payload = value.payload(crate::MemoValueKind::Font)?;
        if payload.len() > limits.max_payload_bytes {
            return Err(crate::MemoValueError::Oversized {
                actual: payload.len(),
                limit: limits.max_payload_bytes,
            });
        }
        let rollback = self.capture_scoped_rollback();
        match self.stores.import_memo_font(payload) {
            Ok(id) => Ok(id),
            Err(error) => {
                self.rollback_scoped(rollback);
                Err(crate::MemoValueError::Codec(format!("{error:?}")))
            }
        }
    }

    #[must_use]
    pub fn font_by_source_identity(
        &self,
        identity: tex_fonts::FontSourceIdentity,
    ) -> Option<FontId> {
        self.poison_tracked_region(TrackedRegionBarrier::UnsupportedExecutionState);
        self.stores.font_by_source_identity(identity)
    }

    #[must_use]
    pub fn font_name(&self, id: FontId) -> String {
        self.observe_font_dependency(id, DependencyFontField::Name, 0);
        self.stores.font_name(id)
    }

    #[must_use]
    pub fn font_identifier_symbol(&self, id: FontId) -> Option<SymbolId> {
        self.observe_font_dependency(id, DependencyFontField::Identifier, 0);
        self.stores.font_identifier_symbol(id)
    }

    /// Assigns the font's one-time control-sequence identifier.
    ///
    /// # Panics
    ///
    /// Panics when an unnamed font has already entered a frozen character or
    /// ligature node, because that node's published semantic identity includes
    /// the font's complete identity.
    pub fn set_font_identifier_symbol(
        &mut self,
        id: FontId,
        symbol: impl crate::interner::SymbolReference,
    ) {
        self.stores.set_font_identifier_symbol(id, symbol);
    }

    #[must_use]
    pub fn font_metrics(&self, font: FontId) -> &FontMetrics {
        self.observe_font_dependency(font, DependencyFontField::Metrics, 0);
        self.stores.font_metrics(font)
    }

    #[must_use]
    pub fn font_char_exists(&self, font: FontId, code: u8) -> bool {
        self.observe_font_dependency(font, DependencyFontField::Metrics, u32::from(code));
        self.stores.font_char_exists(font, code)
    }

    #[must_use]
    pub fn font_char_metrics(&self, font: FontId, code: u8) -> Option<CharMetrics> {
        self.observe_font_dependency(font, DependencyFontField::Metrics, u32::from(code));
        self.stores.font_char_metrics(font, code)
    }

    #[must_use]
    pub fn font_character_exists(&self, font: FontId, ch: char) -> bool {
        self.observe_font_dependency(font, DependencyFontField::Metrics, ch.into());
        self.stores.font_character_exists(font, ch)
    }

    #[must_use]
    pub fn font_character_metrics(&self, font: FontId, ch: char) -> Option<CharMetrics> {
        self.observe_font_dependency(font, DependencyFontField::Metrics, ch.into());
        self.stores.font_character_metrics(font, ch)
    }

    #[must_use]
    pub fn font_uses_tfm_metrics(&self, font: FontId) -> bool {
        self.observe_font_dependency(font, DependencyFontField::Metrics, 0);
        self.stores.font_uses_tfm_metrics(font)
    }

    /// Returns the immutable dense TFM-byte width projection for a live font.
    #[must_use]
    pub fn font_widths(&self, font: FontId) -> &[Scaled; 256] {
        self.observe_font_dependency(font, DependencyFontField::Metrics, 0);
        self.stores.font_widths(font)
    }

    #[must_use]
    pub fn font_characters(&self, font: FontId) -> &[Option<CharMetrics>] {
        self.observe_font_dependency(font, DependencyFontField::Metrics, 0);
        self.stores.font_characters(font)
    }

    #[must_use]
    pub fn font_next_larger(&self, font: FontId, code: u8) -> Option<u8> {
        self.observe_font_dependency(font, DependencyFontField::Metrics, u32::from(code));
        self.stores.font_next_larger(font, code)
    }

    #[must_use]
    pub fn missing_font_character(&self, font: FontId, code: u8) -> Option<MissingCharacter> {
        self.observe_font_dependency(font, DependencyFontField::Metrics, u32::from(code));
        self.stores.missing_font_character(font, code)
    }

    #[must_use]
    pub fn lig_kern_iter(
        &self,
        font: FontId,
        left: LigKernChar,
        right: LigKernChar,
    ) -> LigKernIter<'_> {
        self.observe_font_dependency(font, DependencyFontField::Metrics, 0);
        self.stores.lig_kern_iter(font, left, right)
    }

    #[must_use]
    pub fn lig_kern_command(
        &self,
        font: FontId,
        left: LigKernChar,
        right: LigKernChar,
    ) -> Option<LigKernCommand> {
        self.observe_font_dependency(font, DependencyFontField::Metrics, 0);
        self.stores.lig_kern_command(font, left, right)
    }

    #[must_use]
    pub fn tfm_lig_kern_command(
        &self,
        font: FontId,
        left: LigKernChar,
        right: LigKernChar,
    ) -> Option<LigKernCommand> {
        self.observe_font_dependency(font, DependencyFontField::Metrics, 0);
        self.stores.tfm_lig_kern_command(font, left, right)
    }

    #[must_use]
    pub fn font_false_boundary_char(&self, font: FontId) -> Option<u8> {
        self.observe_font_dependency(font, DependencyFontField::Metrics, 0);
        self.stores.font_false_boundary_char(font)
    }

    #[must_use]
    pub fn pdf_font_code(&self, table: crate::font::PdfFontCode, font: FontId, code: u8) -> i32 {
        let table_index = match table {
            crate::font::PdfFontCode::Lp => 0,
            crate::font::PdfFontCode::Rp => 1,
            crate::font::PdfFontCode::Ef => 2,
            crate::font::PdfFontCode::Tag => 3,
            crate::font::PdfFontCode::Knbs => 4,
            crate::font::PdfFontCode::Stbs => 5,
            crate::font::PdfFontCode::Shbs => 6,
            crate::font::PdfFontCode::Knbc => 7,
            crate::font::PdfFontCode::Knac => 8,
        };
        self.observe_font_dependency(
            font,
            DependencyFontField::PdfCode,
            table_index * 256 + u32::from(code),
        );
        self.stores.pdf_font_code(table, font, code)
    }

    pub fn set_pdf_font_code(
        &mut self,
        table: crate::font::PdfFontCode,
        font: FontId,
        code: u8,
        value: i32,
    ) {
        let receipt = self.stores.set_pdf_font_code(table, font, code, value);
        self.consume_env_mutation(receipt);
    }

    pub fn disable_pdf_font_ligatures(&mut self, font: FontId) {
        let receipt = self.stores.disable_pdf_font_ligatures(font);
        self.consume_env_mutation(receipt);
    }

    #[must_use]
    pub fn pdf_font_ligatures_disabled(&self, font: FontId) -> bool {
        self.observe_font_dependency(font, DependencyFontField::PdfShaping, 0);
        self.stores.pdf_font_ligatures_disabled(font)
    }

    #[must_use]
    pub fn extensible_recipe(&self, font: FontId, code: u8) -> Option<ExtensibleRecipe> {
        self.observe_font_dependency(font, DependencyFontField::Metrics, u32::from(code));
        self.stores.extensible_recipe(font, code)
    }

    #[must_use]
    pub fn font_parameter(&self, font: FontId, number: u32) -> Scaled {
        self.observe_font_dependency(font, DependencyFontField::Parameter, number);
        self.stores.font_parameter(font, number)
    }

    /// Reads the parameter authority used by classic Appendix G math.
    ///
    /// Mapped OpenType text fonts retain their immutable source TFM bank for
    /// math. Ordinary classic fonts continue to observe live `fontdimen`
    /// assignments in the environment.
    #[must_use]
    pub fn classic_math_parameter(&self, font: FontId, number: u16) -> Scaled {
        self.font(font)
            .classic_math_parameter_override(number)
            .unwrap_or_else(|| self.font_parameter(font, u32::from(number)))
    }

    /// Returns the parameter count visible to classic Appendix G math.
    #[must_use]
    pub fn classic_math_parameter_count(&self, font: FontId) -> u32 {
        self.font(font)
            .classic_math_parameter_count_override()
            .map_or_else(
                || self.font_parameter_count(font),
                |count| u32::try_from(count).expect("font parameter count exceeds u32"),
            )
    }

    #[must_use]
    pub fn current_font(&self) -> FontId {
        self.observe_cell_dependency(BankTag::CurrentFont, 0);
        self.stores.current_font()
    }

    #[must_use]
    pub fn current_font_symbol(&self) -> Option<SymbolId> {
        self.observe_cell_dependency(BankTag::CurrentFont, 0);
        self.stores.current_font_symbol()
    }

    pub fn set_current_font(&mut self, id: FontId) {
        let receipt = self.stores.set_current_font(id);
        self.consume_env_mutation(receipt);
    }

    pub fn set_current_font_global(&mut self, id: FontId) {
        let receipt = self.stores.set_current_font_global(id);
        self.consume_env_mutation(receipt);
    }

    pub fn set_current_font_selector(
        &mut self,
        symbol: impl crate::interner::SymbolReference,
        id: FontId,
    ) {
        let receipt = self.stores.set_current_font_selector(symbol, id);
        self.consume_env_mutation(receipt);
    }

    pub fn set_current_font_selector_global(
        &mut self,
        symbol: impl crate::interner::SymbolReference,
        id: FontId,
    ) {
        let receipt = self.stores.set_current_font_selector_global(symbol, id);
        self.consume_env_mutation(receipt);
    }

    #[must_use]
    pub fn math_family_font(&self, size: MathFontSize, family: u8) -> FontId {
        let size_index = match size {
            MathFontSize::Text => 0,
            MathFontSize::Script => 1,
            MathFontSize::ScriptScript => 2,
        };
        self.observe_cell_dependency(BankTag::MathFamilyFont, size_index * 16 + u32::from(family));
        self.stores.math_family_font(size, family)
    }

    pub fn set_math_family_font(
        &mut self,
        size: MathFontSize,
        family: u8,
        id: FontId,
        global: bool,
    ) {
        let receipt = self.stores.set_math_family_font(size, family, id, global);
        self.consume_env_mutation(receipt);
    }

    #[must_use]
    pub fn font_dimen(&self, font: FontId, number: u32) -> Scaled {
        self.observe_font_dependency(font, DependencyFontField::Parameter, number);
        self.stores.font_dimen(font, number)
    }

    #[must_use]
    pub fn font_parameter_count(&self, font: FontId) -> u32 {
        self.observe_font_dependency(font, DependencyFontField::ParameterCount, 0);
        self.stores.font_parameter_count(font)
    }

    /// TeX82 §578's `find_font_dimen` decision; see
    /// [`crate::stores::Stores::font_dimen_writable`].
    #[must_use]
    pub fn font_dimen_writable(&self, font: FontId, number: u32) -> bool {
        self.observe_font_dependency(font, DependencyFontField::ParameterCount, number);
        self.stores.font_dimen_writable(font, number)
    }

    pub fn set_font_dimen(
        &mut self,
        font: FontId,
        number: u32,
        value: Scaled,
    ) -> Result<(), FontParameterError> {
        let receipts = self.stores.set_font_dimen(font, number, value)?;
        self.consume_env_mutations(receipts);
        Ok(())
    }

    #[must_use]
    pub fn font_hyphen_char(&self, font: FontId) -> i32 {
        self.observe_font_dependency(font, DependencyFontField::HyphenChar, 0);
        self.stores.font_hyphen_char(font)
    }

    pub fn set_font_hyphen_char(&mut self, font: FontId, value: i32) {
        let receipt = self.stores.set_font_hyphen_char(font, value);
        self.consume_env_mutation(receipt);
    }

    #[must_use]
    pub fn font_skew_char(&self, font: FontId) -> i32 {
        self.observe_font_dependency(font, DependencyFontField::SkewChar, 0);
        self.stores.font_skew_char(font)
    }

    pub fn set_font_skew_char(&mut self, font: FontId, value: i32) {
        let receipt = self.stores.set_font_skew_char(font, value);
        self.consume_env_mutation(receipt);
    }

    #[must_use]
    pub fn node_list_builder(&self) -> NodeListBuilder {
        self.stores.node_list_builder()
    }

    pub fn freeze_node_list(&mut self, nodes: &[Node]) -> NodeListRef {
        self.stores.freeze_node_list(nodes)
    }

    pub fn freeze_node_list_owned(&mut self, nodes: &mut Vec<Node>) -> NodeListRef {
        self.stores.freeze_node_list_owned(nodes)
    }

    #[doc(hidden)]
    #[must_use]
    pub fn node_word_index(&self, list: &NodeListRef, index: usize) -> Option<u32> {
        list.id().start().checked_add(u32::try_from(index).ok()?)
    }

    pub fn finish_node_list(&mut self, builder: &mut NodeListBuilder) -> NodeListRef {
        self.stores.finish_node_list(builder)
    }

    /// Consumes an operation-local builder and returns direct immutable graph
    /// ownership without publishing it into an aggregate state destination.
    pub fn freeze_node_list_ref(&mut self, builder: NodeListBuilder) -> NodeListRef {
        self.stores.freeze_node_list_ref(builder)
    }

    /// Captures a handle-free, provenance-free node graph for memo retention.
    pub fn detach_node_list(
        &self,
        root: &NodeListRef,
    ) -> Result<crate::DetachedMemoValue, crate::MemoValueError> {
        let payload = self
            .stores
            .encode_memo_node_list_ref(root)
            .map_err(|error| crate::MemoValueError::Codec(format!("{error:?}")))?;
        Ok(crate::DetachedMemoValue::from_payload(
            crate::MemoValueKind::Nodes,
            payload,
        ))
    }

    /// Captures one box-root list as a detached memo value.
    pub fn detach_box(
        &self,
        root: &NodeListRef,
    ) -> Result<crate::DetachedMemoValue, crate::MemoValueError> {
        if root.nodes().len() != 1
            || !matches!(
                root.nodes().first(),
                Some(crate::node_arena::NodeRef::HList(_) | crate::node_arena::NodeRef::VList(_))
            )
        {
            return Err(crate::MemoValueError::Invalid(
                "memo box root is not one box",
            ));
        }
        self.detach_node_value(root, crate::MemoValueKind::Box)
    }

    fn detach_node_value(
        &self,
        root: &NodeListRef,
        kind: crate::MemoValueKind,
    ) -> Result<crate::DetachedMemoValue, crate::MemoValueError> {
        let payload = self
            .stores
            .encode_memo_node_list(root)
            .map_err(|error| crate::MemoValueError::Codec(format!("{error:?}")))?;
        Ok(crate::DetachedMemoValue::from_payload(kind, payload))
    }

    /// Atomically imports a detached node graph into this Universe owner.
    pub fn import_memo_node_list(
        &mut self,
        value: &crate::DetachedMemoValue,
        limits: crate::MemoValueLimits,
    ) -> Result<NodeListRef, crate::MemoValueError> {
        self.import_memo_node_value(value, crate::MemoValueKind::Nodes, limits, false)
    }

    /// Atomically imports and verifies a detached single-box root.
    pub fn import_memo_box(
        &mut self,
        value: &crate::DetachedMemoValue,
        limits: crate::MemoValueLimits,
    ) -> Result<NodeListRef, crate::MemoValueError> {
        self.import_memo_node_value(value, crate::MemoValueKind::Box, limits, true)
    }

    fn import_memo_node_value(
        &mut self,
        value: &crate::DetachedMemoValue,
        kind: crate::MemoValueKind,
        limits: crate::MemoValueLimits,
        require_box: bool,
    ) -> Result<NodeListRef, crate::MemoValueError> {
        let payload = value.payload(kind)?;
        if payload.len() > limits.max_payload_bytes {
            return Err(crate::MemoValueError::Oversized {
                actual: payload.len(),
                limit: limits.max_payload_bytes,
            });
        }
        let rollback = self.capture_scoped_rollback();
        match self.stores.import_memo_node_list(
            payload,
            limits.max_nodes,
            limits.max_tokens,
            limits.max_string_bytes,
        ) {
            Ok(root)
                if !require_box
                    || (root.nodes().len() == 1
                        && matches!(
                            root.nodes().first(),
                            Some(
                                crate::node_arena::NodeRef::HList(_)
                                    | crate::node_arena::NodeRef::VList(_)
                            )
                        )) =>
            {
                Ok(root)
            }
            Ok(_) => {
                self.rollback_scoped(rollback);
                Err(crate::MemoValueError::Invalid(
                    "memo box root is not one box",
                ))
            }
            Err(error) => {
                self.rollback_scoped(rollback);
                Err(crate::MemoValueError::Codec(format!("{error:?}")))
            }
        }
    }

    #[must_use]
    pub fn innermost_group_kind(&self) -> Option<GroupKind> {
        self.observe_semantic_dependency(DependencyKey::Engine(DependencyEngineField::GroupType));
        self.stores.innermost_group_kind()
    }

    /// Returns the current TeX execution-group depth.
    #[must_use]
    pub fn execution_group_depth(&self) -> u32 {
        self.observe_semantic_dependency(DependencyKey::Engine(DependencyEngineField::GroupLevel));
        self.stores.env_group_depth()
    }

    #[must_use]
    pub fn group_kinds(&self) -> impl DoubleEndedIterator<Item = GroupKind> + '_ {
        self.stores.group_kinds()
    }

    #[must_use]
    pub fn group_frames(&self) -> impl DoubleEndedIterator<Item = crate::GroupFrame> + '_ {
        self.stores.group_frames()
    }

    /// Number of open groups, TeX82's `cur_level-level_one` (§271).
    ///
    /// A nested construction that must run until *its own* group closes
    /// compares against a depth sampled before it opened, rather than
    /// watching [`Self::innermost_group_kind`]: groups nested inside the
    /// body make the innermost kind say nothing about whose brace arrived.
    #[must_use]
    pub fn group_depth(&self) -> u32 {
        self.stores.env_group_depth()
    }

    pub fn enter_group(&mut self) {
        self.stores.enter_group();
        self.mark_group_entry_dependencies();
        self.trace_group_enter(GroupKind::Simple, self.stores.env_group_depth(), 0);
    }

    pub fn enter_group_with_kind(&mut self, kind: GroupKind) {
        self.stores.enter_group_with_kind(kind);
        self.mark_group_entry_dependencies();
        self.trace_group_enter(kind, self.stores.env_group_depth(), 0);
    }

    pub fn enter_group_with_kind_at_line(&mut self, kind: GroupKind, entered_line: u32) {
        self.stores
            .enter_group_with_kind_at_line(kind, entered_line);
        self.mark_group_entry_dependencies();
        // e-TeX 2.6 [19.274]: `group_trace(false)` fires once the new frame
        // is live, so the displayed level already counts it.
        self.trace_group_enter(kind, self.stores.env_group_depth(), entered_line);
    }

    fn mark_group_entry_dependencies(&mut self) {
        self.dependencies
            .get_mut()
            .expect("dependency runtime mutex is not poisoned")
            .mark_changed(DependencyKey::Engine(DependencyEngineField::GroupLevel));
        self.dependencies
            .get_mut()
            .expect("dependency runtime mutex is not poisoned")
            .mark_changed(DependencyKey::Engine(DependencyEngineField::GroupType));
    }

    pub fn push_aftergroup(&mut self, payload: Token) {
        self.stores.push_aftergroup(payload);
    }

    pub fn push_aftergroup_traced(&mut self, payload: crate::token::RootedTracedTokenWord) {
        self.stores.push_aftergroup_traced(payload);
    }

    #[must_use]
    pub fn leave_group(&mut self) -> Vec<Token> {
        let trace_context = self.leaving_group_trace_context();
        let (tokens, receipts, code_before, code_after, restores, code_restores) =
            self.stores.leave_group_observing_dependencies();
        self.retarget_hash_base_after_group_compaction();
        self.mark_group_exit_dependencies(receipts, code_before, code_after);
        self.trace_interleaved_restores(&restores, &code_restores);
        if let Some((kind, level, entered_line)) = trace_context {
            self.trace_group_leave(kind, level, entered_line);
        }
        tokens
            .into_iter()
            .map(|word| word.word().semantic_token())
            .collect()
    }

    pub fn leave_group_with_kind(
        &mut self,
        expected: GroupKind,
    ) -> Result<Vec<crate::token::RootedTracedTokenWord>, GroupMismatch> {
        let trace_context = self.leaving_group_trace_context();
        let (tokens, receipts, code_before, code_after, restores, code_restores) = self
            .stores
            .leave_group_with_kind_observing_dependencies(expected)?;
        self.retarget_hash_base_after_group_compaction();
        self.mark_group_exit_dependencies(receipts, code_before, code_after);
        self.trace_interleaved_restores(&restores, &code_restores);
        if let Some((kind, level, entered_line)) = trace_context {
            self.trace_group_leave(kind, level, entered_line);
        }
        Ok(tokens)
    }

    fn trace_restores(&mut self, records: &[crate::env::group::RestoreRecord]) {
        use crate::cell::BankTag;
        use crate::env::banks::TokParam;

        for record in records {
            if record.tracing_restores() <= 0 || !record.trace_eligible() {
                continue;
            }
            let cell = record.cell();
            if cell.bank() == BankTag::Box {
                let label = if record.is_retaining() {
                    "retaining"
                } else {
                    "restoring"
                };
                let value = record
                    .box_trace_text()
                    .expect("box restore diagnostics are detached before group storage retires");
                let mut diagnostic = crate::diagnostic::Diagnostic::begin_with_tracing_online(
                    self,
                    record.tracing_online(),
                );
                diagnostic.print_char('{').print(label).print_char(' ');
                if let Ok(byte) = u8::try_from(record.escape_char()) {
                    diagnostic.print_ascii(char::from(byte));
                }
                diagnostic.print(&format!("box{}=", cell.index()));
                // TeX82 §252 prints a null box's `void` directly after the
                // equals sign. Only a non-null box enters `show_node_list`,
                // whose first node display begins with `print_ln` (§174).
                if value == "void" {
                    diagnostic.print(value);
                } else {
                    diagnostic.print_ln().print(value);
                }
                diagnostic.print_char('}');
                diagnostic.end(false);
                continue;
            }
            let (name, value, escape_name) = match cell.bank() {
                BankTag::Meaning => {
                    let Some(symbol) = self.stores.symbol_at_slot(cell.index()) else {
                        continue;
                    };
                    let symbol = self.stores.resolve_stored_symbol(symbol);
                    let meaning = self
                        .stores
                        .resolve_stored_meaning(Meaning::decode_stored(record.old()));
                    let value = match meaning {
                        // TeX82 §§252 and 283 render an undefined control
                        // sequence explicitly; it is not an absent restore
                        // record. This is especially visible after a local
                        // definition leaves its group.
                        Meaning::Undefined => "undefined".to_owned(),
                        Meaning::Relax
                        | Meaning::ExpandablePrimitive(_)
                        | Meaning::UnexpandablePrimitive(_) => {
                            let Some(canonical) = self.stores.first_symbol_with_meaning(meaning)
                            else {
                                continue;
                            };
                            escaped_restore_name(
                                record.escape_char(),
                                self.stores.resolve(canonical),
                            )
                        }
                        // TeX82 §§252/283 print the command class and its
                        // saved operand after `unsave` has restored a
                        // shorthand definition. Keep this keyed on the typed
                        // meaning so every `\chardef`/`\mathchardef` target
                        // (named, control-symbol, or active) follows the same
                        // path.
                        Meaning::CharGiven(character) => format!(
                            "{}\"{:X}",
                            escaped_restore_name(record.escape_char(), "char"),
                            u32::from(character)
                        ),
                        Meaning::MathCharGiven(code) => format!(
                            "{}\"{code:X}",
                            escaped_restore_name(record.escape_char(), "mathchar")
                        ),
                        Meaning::Macro { flags, definition } => {
                            let macro_meaning = self.stores.macro_definition(definition);
                            let mut value = String::new();
                            for (flag, name) in [
                                (crate::meaning::MeaningFlags::PROTECTED, "protected"),
                                (crate::meaning::MeaningFlags::LONG, "long"),
                                (crate::meaning::MeaningFlags::OUTER, "outer"),
                            ] {
                                if flags.contains(flag) {
                                    value.push_str(&escaped_restore_name(
                                        record.escape_char(),
                                        name,
                                    ));
                                }
                            }
                            if !value.is_empty() {
                                value.push(' ');
                            }
                            value.push_str("macro:");
                            append_bounded_macro_body(
                                self,
                                macro_meaning.parameter_text(),
                                macro_meaning.replacement_text(),
                                record.escape_char(),
                                &mut value,
                            );
                            value
                        }
                        _ => continue,
                    };
                    (
                        sprint_restore_name(
                            record.escape_char(),
                            self.stores.control_sequence_kind(symbol),
                            self.stores.resolve(symbol),
                        ),
                        value,
                        false,
                    )
                }
                BankTag::Count => (
                    format!("count{}", cell.index()),
                    (record.old() as u32 as i32).to_string(),
                    true,
                ),
                BankTag::Dimen => (
                    format!("dimen{}", cell.index()),
                    format!(
                        "{}pt",
                        format_restore_scaled(crate::scaled::Scaled::from_raw(
                            record.old() as u32 as i32,
                        ))
                    ),
                    true,
                ),
                BankTag::Skip => {
                    let id = GlueId::new(record.old() as u32);
                    (
                        format!("skip{}", cell.index()),
                        format_restore_glue(self.glue(id), "pt"),
                        true,
                    )
                }
                BankTag::Muskip => {
                    let id = GlueId::new(record.old() as u32);
                    (
                        format!("muskip{}", cell.index()),
                        format_restore_glue(self.glue(id), "mu"),
                        true,
                    )
                }
                BankTag::Toks => (
                    format!("toks{}", cell.index()),
                    // e-TeX [53a] stores a sparse token-register pointer
                    // directly. This is not the optional-plus-one encoding
                    // used by Umber's token-parameter cells.
                    format_restore_tokens(
                        self,
                        Some(TokenListId::new(
                            u32::try_from(record.old())
                                .expect("token-register restore word exceeds u32"),
                        )),
                        record.escape_char(),
                    ),
                    true,
                ),
                BankTag::IntParam if cell.index() < 128 => {
                    let Some(name) = self.primitive_name(Meaning::IntParam(cell.index() as u16))
                    else {
                        continue;
                    };
                    (
                        name.to_owned(),
                        (record.old() as u32 as i32).to_string(),
                        true,
                    )
                }
                BankTag::DimenParam if cell.index() < 128 => {
                    let Some(name) = self.primitive_name(Meaning::DimenParam(cell.index() as u16))
                    else {
                        continue;
                    };
                    (
                        name.to_owned(),
                        format!(
                            "{}pt",
                            format_restore_scaled(crate::scaled::Scaled::from_raw(record.old()
                                as u32
                                as i32,))
                        ),
                        true,
                    )
                }
                BankTag::GlueParam if cell.index() < 128 => {
                    let Some(name) = self.primitive_name(Meaning::GlueParam(cell.index() as u16))
                    else {
                        continue;
                    };
                    let id = GlueId::new(record.old() as u32);
                    (
                        name.to_owned(),
                        format_restore_glue(self.glue(id), "pt"),
                        true,
                    )
                }
                BankTag::TokParam
                    if cell.index() == u32::from(TokParam::PAR_SHAPE_INTERNAL.raw()) =>
                {
                    // TeX82 §252's region-four `show_eqtb` gives
                    // `par_shape_loc` its own representation: the number of
                    // indent/width pairs, rather than the backing token-list
                    // payload used by Umber. Section 283 calls that renderer
                    // immediately after restoring the saved eqtb entry.
                    let Some(tokens) = restored_tok_param_tokens(self, record.old()) else {
                        continue;
                    };
                    assert_eq!(
                        tokens.len() % 8,
                        0,
                        "restored internal parshape payload is truncated"
                    );
                    let line_count = tokens.len() / 8;
                    ("parshape".to_owned(), line_count.to_string(), true)
                }
                BankTag::TokParam
                    if cell.index() == u32::from(TokParam::INTER_LINE_PENALTIES_INTERNAL.raw()) =>
                {
                    (
                        "interlinepenalties".to_owned(),
                        format_restore_penalty_array(self, record.old(), record.escape_char()),
                        true,
                    )
                }
                BankTag::TokParam
                    if cell.index() == u32::from(TokParam::CLUB_PENALTIES_INTERNAL.raw()) =>
                {
                    (
                        "clubpenalties".to_owned(),
                        format_restore_penalty_array(self, record.old(), record.escape_char()),
                        true,
                    )
                }
                BankTag::TokParam
                    if cell.index() == u32::from(TokParam::WIDOW_PENALTIES_INTERNAL.raw()) =>
                {
                    (
                        "widowpenalties".to_owned(),
                        format_restore_penalty_array(self, record.old(), record.escape_char()),
                        true,
                    )
                }
                BankTag::TokParam
                    if cell.index()
                        == u32::from(TokParam::DISPLAY_WIDOW_PENALTIES_INTERNAL.raw()) =>
                {
                    (
                        "displaywidowpenalties".to_owned(),
                        format_restore_penalty_array(self, record.old(), record.escape_char()),
                        true,
                    )
                }
                BankTag::TokParam if cell.index() < 128 => {
                    let Some(name) = self.primitive_name(Meaning::TokParam(cell.index() as u16))
                    else {
                        continue;
                    };
                    use crate::env::banks::{BankCodec, OptionalTokenListIdCodec};
                    let value = format_restore_tokens(
                        self,
                        OptionalTokenListIdCodec::decode(record.old()),
                        record.escape_char(),
                    );
                    (name.to_owned(), value, true)
                }
                BankTag::CurrentFont => {
                    // TeX82 §252's `show_eqtb(cur_font_loc)` uses the literal
                    // label `current font` (without `print_esc`) and then
                    // `font_id_text(equiv(n))`: the restored font's frozen
                    // identifier, not the token that most recently selected
                    // it. Section 283 invokes that same renderer for restored
                    // and retained save-stack entries.
                    let font = self
                        .stores
                        .resolve_stored_font(FontId::new(record.old() as u32));
                    let Some(symbol) = self.stores.font_identifier_symbol(font) else {
                        continue;
                    };
                    let value =
                        escaped_restore_name(record.escape_char(), self.stores.resolve(symbol));
                    ("current font".to_owned(), value, false)
                }
                BankTag::MathFamilyFont if cell.index() < 48 => {
                    // TeX82 §§252/283 print the selector name and family
                    // number, followed by the restored font's identifier.
                    let size = cell.index() / 16;
                    let family = cell.index() % 16;
                    let name = match size {
                        0 => "textfont",
                        1 => "scriptfont",
                        2 => "scriptscriptfont",
                        _ => unreachable!("guard restricts math-family font size"),
                    };
                    let font = self
                        .stores
                        .resolve_stored_font(FontId::new(record.old() as u32));
                    let Some(symbol) = self.stores.font_identifier_symbol(font) else {
                        continue;
                    };
                    (
                        format!("{name}{family}"),
                        escaped_restore_name(record.escape_char(), self.stores.resolve(symbol)),
                        true,
                    )
                }
                _ => continue,
            };
            let label = if record.is_retaining() {
                "retaining"
            } else {
                "restoring"
            };
            let mut diagnostic = crate::diagnostic::Diagnostic::begin_with_tracing_online(
                self,
                record.tracing_online(),
            );
            diagnostic.print_char('{').print(label).print_char(' ');
            if escape_name && let Ok(byte) = u8::try_from(record.escape_char()) {
                diagnostic.print_ascii(char::from(byte));
            }
            diagnostic
                .print(&name)
                .print_char('=')
                .print(&value)
                .print_char('}');
            diagnostic.end(false);
        }
    }

    fn trace_interleaved_restores(
        &mut self,
        env: &[crate::env::group::RestoreRecord],
        code: &[crate::code_tables::CodeTableRestoreRecord],
    ) {
        // TeX82 §283 pops one save stack. Code tables use structural roots,
        // but their diagnostics must retain their position among eqtb saves.
        let (mut env_index, mut code_index) = (0, 0);
        while env_index < env.len() || code_index < code.len() {
            let take_env = code_index == code.len()
                || (env_index < env.len()
                    && env[env_index].save_position() >= code[code_index].save_position);
            if take_env {
                self.trace_restores(&env[env_index..=env_index]);
                env_index += 1;
            } else {
                self.trace_code_restores(&code[code_index..=code_index]);
                code_index += 1;
            }
        }
    }

    fn trace_code_restores(&mut self, records: &[crate::code_tables::CodeTableRestoreRecord]) {
        use crate::code_tables::CodeTableKind;

        let tracing_restores = self.int_param(crate::env::banks::IntParam::TRACING_RESTORES);
        if tracing_restores <= 0 {
            return;
        }
        let tracing_online = self.int_param(crate::env::banks::IntParam::TRACING_ONLINE);
        let escape_char = self.int_param(crate::env::banks::IntParam::ESCAPE_CHAR);
        for record in records {
            let name = match record.kind {
                CodeTableKind::Catcode => "catcode",
                CodeTableKind::Lccode => "lccode",
                CodeTableKind::Uccode => "uccode",
                CodeTableKind::Sfcode => "sfcode",
                CodeTableKind::Mathcode => "mathcode",
                CodeTableKind::Delcode => "delcode",
            };
            let mut diagnostic =
                crate::diagnostic::Diagnostic::begin_with_tracing_online(self, tracing_online);
            diagnostic.print_char('{').print(if record.retaining {
                "retaining "
            } else {
                "restoring "
            });
            if let Ok(byte) = u8::try_from(escape_char) {
                diagnostic.print_ascii(char::from(byte));
            }
            diagnostic
                .print(name)
                .print_int(record.ch as i32)
                .print_char('=')
                .print_int(record.value as i32)
                .print_char('}');
            diagnostic.end(false);
        }
    }

    /// Captures the frame fields e-TeX 2.6 [19.282]'s `group_trace(true)`
    /// needs after `unsave` has restored the enclosing parameter values.
    fn leaving_group_trace_context(&self) -> Option<(GroupKind, u32, u32)> {
        let frame = self.stores.group_frames().next_back()?;
        let (kind, entered_line) = (frame.kind(), frame.entered_line());
        Some((kind, self.stores.env_group_depth(), entered_line))
    }

    fn mark_group_exit_dependencies(
        &mut self,
        receipts: crate::env::group::MutationReceipts,
        code_before: CodeTableGenerations,
        code_after: CodeTableGenerations,
    ) {
        self.consume_env_mutations(receipts);
        for (table, changed) in [
            (
                DependencyCodeTable::Catcode,
                code_before.catcode != code_after.catcode,
            ),
            (
                DependencyCodeTable::Lccode,
                code_before.lccode != code_after.lccode,
            ),
            (
                DependencyCodeTable::Uccode,
                code_before.uccode != code_after.uccode,
            ),
            (
                DependencyCodeTable::Sfcode,
                code_before.sfcode != code_after.sfcode,
            ),
            (
                DependencyCodeTable::Mathcode,
                code_before.mathcode != code_after.mathcode,
            ),
            (
                DependencyCodeTable::Delcode,
                code_before.delcode != code_after.delcode,
            ),
        ] {
            if changed {
                self.dependencies
                    .get_mut()
                    .expect("dependency runtime mutex is not poisoned")
                    .mark_changed(DependencyKey::CodeGeneration(table));
            }
        }
        self.dependencies
            .get_mut()
            .expect("dependency runtime mutex is not poisoned")
            .mark_changed(DependencyKey::Engine(DependencyEngineField::GroupLevel));
        self.dependencies
            .get_mut()
            .expect("dependency runtime mutex is not poisoned")
            .mark_changed(DependencyKey::Engine(DependencyEngineField::GroupType));
    }

    pub fn set_afterassignment(&mut self, token: Token) {
        self.stores.set_afterassignment(token);
    }

    pub fn take_afterassignment(&mut self) -> Option<Token> {
        self.stores.take_afterassignment()
    }

    pub fn set_count(&mut self, index: u16, value: i32) {
        let receipt = self.stores.set_count(index, value);
        self.consume_env_mutation(receipt);
    }

    #[must_use]
    pub fn count(&self, index: u16) -> i32 {
        self.observe_cell_dependency(BankTag::Count, u32::from(index));
        self.stores.count(index)
    }

    pub fn set_count_global(&mut self, index: u16, value: i32) {
        let receipt = self.stores.set_count_global(index, value);
        self.consume_env_mutation(receipt);
    }

    pub fn set_dimen(&mut self, index: u16, value: Scaled) {
        let receipt = self.stores.set_dimen(index, value);
        self.consume_env_mutation(receipt);
    }

    #[must_use]
    pub fn dimen(&self, index: u16) -> Scaled {
        self.observe_cell_dependency(BankTag::Dimen, u32::from(index));
        self.stores.dimen(index)
    }

    pub fn set_dimen_global(&mut self, index: u16, value: Scaled) {
        let receipt = self.stores.set_dimen_global(index, value);
        self.consume_env_mutation(receipt);
    }

    pub fn set_skip(&mut self, index: u16, value: impl crate::glue::GlueHandle) {
        let receipt = self.stores.set_skip(index, value);
        self.consume_env_mutation(receipt);
    }

    #[must_use]
    pub fn skip(&self, index: u16) -> GlueId {
        self.observe_cell_dependency(BankTag::Skip, u32::from(index));
        self.stores.skip(index)
    }

    pub fn set_skip_global(&mut self, index: u16, value: impl crate::glue::GlueHandle) {
        let receipt = self.stores.set_skip_global(index, value);
        self.consume_env_mutation(receipt);
    }

    pub fn set_muskip(&mut self, index: u16, value: impl crate::glue::GlueHandle) {
        let receipt = self.stores.set_muskip(index, value);
        self.consume_env_mutation(receipt);
    }

    #[must_use]
    pub fn muskip(&self, index: u16) -> GlueId {
        self.observe_cell_dependency(BankTag::Muskip, u32::from(index));
        self.stores.muskip(index)
    }

    pub fn set_muskip_global(&mut self, index: u16, value: impl crate::glue::GlueHandle) {
        let receipt = self.stores.set_muskip_global(index, value);
        self.consume_env_mutation(receipt);
    }

    pub fn set_toks(&mut self, index: u16, value: TokenListId) {
        let receipt = self.stores.set_toks(index, value);
        self.consume_env_mutation(receipt);
    }

    #[must_use]
    pub fn toks(&self, index: u16) -> TokenListId {
        self.observe_cell_dependency(BankTag::Toks, u32::from(index));
        self.stores.toks(index)
    }

    pub fn set_toks_global(&mut self, index: u16, value: TokenListId) {
        let receipt = self.stores.set_toks_global(index, value);
        self.consume_env_mutation(receipt);
    }

    /// Installs an already-owned box without routing it through a raw
    /// coordinate lookup.
    pub fn set_box_reg_ref(&mut self, index: u16, value: NodeListRef) {
        let receipt = self.stores.write_box_reg_ref(index, Some(value), false);
        self.consume_env_mutation(receipt);
    }

    /// Globally installs an already-owned box.
    pub fn set_box_reg_ref_global(&mut self, index: u16, value: NodeListRef) {
        let receipt = self.stores.write_box_reg_ref(index, Some(value), true);
        self.consume_env_mutation(receipt);
    }

    /// Begins one box-register value scan.
    #[must_use]
    pub fn begin_box_build(&mut self) -> BoxBuildTransaction<'_> {
        BoxBuildTransaction {
            universe: self,
            finished: false,
        }
    }

    /// Clones the box register's structural owner.
    #[must_use]
    pub fn box_reg_ref(&self, index: u16) -> Option<NodeListRef> {
        self.observe_cell_dependency(BankTag::Box, u32::from(index));
        self.stores.box_reg_ref(index)
    }

    /// Observes TeX82 allocator pressure for a non-destructive box copy while
    /// borrowing the register's structural owner.
    pub fn observe_box_copy_ref(&mut self, root: &NodeListRef, live_dynamic_words: usize) {
        self.stores
            .observe_main_memory_box_copy(root, live_dynamic_words);
    }

    /// Formats a box register value through TeX82 §§174/252's compact
    /// `show_eqtb` representation used by assignment diagnostics.
    #[must_use]
    pub fn box_assignment_trace_text(&self, value: Option<&NodeListRef>) -> String {
        value.map_or_else(
            || "void".to_owned(),
            |root| self.stores.box_restore_trace_text_ref(root),
        )
    }

    #[must_use]
    pub fn page_dimension(&self, dimension: PageDimension) -> Scaled {
        self.observe_semantic_dependency(DependencyKey::PageDimension(dimension.index()));
        self.page
            .dimension(dimension, self.output_routine_is_active())
    }

    pub fn set_page_dimension(&mut self, dimension: PageDimension, value: Scaled) {
        self.page.set_dimension(dimension, value);
        self.dependencies
            .get_mut()
            .expect("dependency runtime mutex is not poisoned")
            .mark_changed(DependencyKey::PageDimension(dimension.index()));
    }

    #[must_use]
    pub fn page_integer(&self, integer: PageInteger) -> i32 {
        let index = match integer {
            PageInteger::DeadCycles => 0,
            PageInteger::InsertPenalties => 1,
        };
        self.observe_semantic_dependency(DependencyKey::PageInteger(index));
        self.page.integer(integer)
    }

    pub fn set_page_integer(&mut self, integer: PageInteger, value: i32) {
        self.page.set_integer(integer, value);
        let index = match integer {
            PageInteger::DeadCycles => 0,
            PageInteger::InsertPenalties => 1,
        };
        self.dependencies
            .get_mut()
            .expect("dependency runtime mutex is not poisoned")
            .mark_changed(DependencyKey::PageInteger(index));
    }

    #[must_use]
    pub fn page_mark(&self, mark: PageMark) -> TokenListId {
        self.observe_semantic_dependency(DependencyKey::PageMark(mark.index()));
        self.page.mark(mark)
    }

    #[must_use]
    pub fn page_mark_value(&self, mark: PageMark) -> Option<TokenListId> {
        self.observe_semantic_dependency(DependencyKey::PageMark(mark.index()));
        self.page.mark_value(mark)
    }

    pub fn set_page_mark(&mut self, mark: PageMark, value: TokenListId) {
        let value = self.stores.token_list_ref(value);
        self.page.set_mark(mark, value);
        self.dependencies
            .get_mut()
            .expect("dependency runtime mutex is not poisoned")
            .mark_changed(DependencyKey::PageMark(mark.index()));
        self.dependencies
            .get_mut()
            .expect("dependency runtime mutex is not poisoned")
            .mark_changed(DependencyKey::PageMarkClass {
                mark: mark.index(),
                class: 0,
            });
    }

    pub fn clear_page_mark(&mut self, mark: PageMark) {
        self.page.clear_mark(mark);
        self.dependencies
            .get_mut()
            .expect("dependency runtime mutex is not poisoned")
            .mark_changed(DependencyKey::PageMark(mark.index()));
        self.dependencies
            .get_mut()
            .expect("dependency runtime mutex is not poisoned")
            .mark_changed(DependencyKey::PageMarkClass {
                mark: mark.index(),
                class: 0,
            });
    }

    #[must_use]
    pub fn page_mark_class(&self, mark: PageMark, class: u16) -> TokenListId {
        self.observe_semantic_dependency(DependencyKey::PageMarkClass {
            mark: mark.index(),
            class,
        });
        self.page.mark_class(mark, class)
    }

    #[must_use]
    pub fn page_mark_class_value(&self, mark: PageMark, class: u16) -> Option<TokenListId> {
        self.observe_semantic_dependency(DependencyKey::PageMarkClass {
            mark: mark.index(),
            class,
        });
        self.page.mark_class_value(mark, class)
    }

    pub fn set_page_mark_class(&mut self, mark: PageMark, class: u16, value: TokenListId) {
        let value = self.stores.token_list_ref(value);
        self.page.set_mark_class(mark, class, value);
        self.dependencies
            .get_mut()
            .expect("dependency runtime mutex is not poisoned")
            .mark_changed(DependencyKey::PageMarkClass {
                mark: mark.index(),
                class,
            });
        if class == 0 {
            self.dependencies
                .get_mut()
                .expect("dependency runtime mutex is not poisoned")
                .mark_changed(DependencyKey::PageMark(mark.index()));
        }
    }

    pub fn clear_page_mark_class(&mut self, mark: PageMark, class: u16) {
        self.page.clear_mark_class(mark, class);
        self.dependencies
            .get_mut()
            .expect("dependency runtime mutex is not poisoned")
            .mark_changed(DependencyKey::PageMarkClass {
                mark: mark.index(),
                class,
            });
        if class == 0 {
            self.dependencies
                .get_mut()
                .expect("dependency runtime mutex is not poisoned")
                .mark_changed(DependencyKey::PageMark(mark.index()));
        }
    }

    pub fn page_mark_classes(&self) -> impl Iterator<Item = u16> + '_ {
        self.poison_tracked_region(TrackedRegionBarrier::UnsupportedExecutionState);
        self.page.mark_class_ids()
    }

    pub fn report_bad_register_code(&mut self, value: i32, maximum: u16) {
        self.world.write_text(
            PrintSink::TerminalAndLog,
            &format!(
                "\n! Bad register code ({value}).\nA register number must be between 0 and {maximum}.\nI changed this one to zero.\n"
            ),
        );
    }

    pub fn report_missing_font_identifier(&mut self) {
        self.world.write_text(
            PrintSink::TerminalAndLog,
            "\n! Missing font identifier.\nI was looking for a control sequence whose\ncurrent meaning has been defined by \\font.\n",
        );
    }

    pub fn freeze_page_specs(&mut self, contents: PageContents) {
        let vsize = self.dimen_param(DimenParam::V_SIZE);
        let max_depth = self.dimen_param(DimenParam::MAX_DEPTH);
        self.page.freeze_specs(contents, vsize, max_depth);
        self.dependencies
            .get_mut()
            .expect("dependency runtime mutex is not poisoned")
            .mark_changed(DependencyKey::Page(DependencyPageField::Contents));
    }

    pub fn start_new_page(&mut self) {
        self.page.start_new_page();
        self.dependencies
            .get_mut()
            .expect("dependency runtime mutex is not poisoned")
            .mark_changed(DependencyKey::Page(DependencyPageField::CurrentPage));
    }

    /// Applies TeX82 §1012's post-`fire_up` structural page reset while
    /// retaining `page_so_far` until the next §991 specification freeze.
    pub fn start_page_after_output(&mut self) {
        self.page.start_page_after_output();
        self.dependencies
            .get_mut()
            .expect("dependency runtime mutex is not poisoned")
            .mark_changed(DependencyKey::Page(DependencyPageField::CurrentPage));
    }

    #[must_use]
    pub fn page_discards(&self) -> &[Node] {
        self.observe_semantic_dependency(DependencyKey::Page(DependencyPageField::Discards));
        self.page.page_discards()
    }

    pub fn push_page_discard(&mut self, node: Node) {
        self.stores.assert_live_handles_in_node(&node);
        self.page.push_page_discard(node);
        self.dependencies
            .get_mut()
            .expect("dependency runtime mutex is not poisoned")
            .mark_changed(DependencyKey::Page(DependencyPageField::Discards));
    }

    pub fn take_page_discards(&mut self) -> Vec<Node> {
        let nodes = self.page.take_page_discards();
        self.dependencies
            .get_mut()
            .expect("dependency runtime mutex is not poisoned")
            .mark_changed(DependencyKey::Page(DependencyPageField::Discards));
        nodes
    }

    pub fn clear_page_discards(&mut self) {
        self.page.clear_page_discards();
        self.dependencies
            .get_mut()
            .expect("dependency runtime mutex is not poisoned")
            .mark_changed(DependencyKey::Page(DependencyPageField::Discards));
    }

    #[must_use]
    pub fn split_discards(&self) -> &[Node] {
        self.observe_semantic_dependency(DependencyKey::Page(DependencyPageField::SplitDiscards));
        self.page.split_discards()
    }

    pub fn set_split_discards(&mut self, nodes: Vec<Node>) {
        for node in &nodes {
            self.stores.assert_live_handles_in_node(node);
        }
        self.page.set_split_discards(nodes);
        self.dependencies
            .get_mut()
            .expect("dependency runtime mutex is not poisoned")
            .mark_changed(DependencyKey::Page(DependencyPageField::SplitDiscards));
    }

    pub fn take_split_discards(&mut self) -> Vec<Node> {
        let nodes = self.page.take_split_discards();
        self.dependencies
            .get_mut()
            .expect("dependency runtime mutex is not poisoned")
            .mark_changed(DependencyKey::Page(DependencyPageField::SplitDiscards));
        nodes
    }

    pub fn clear_split_discards(&mut self) {
        self.page.clear_split_discards();
        self.dependencies
            .get_mut()
            .expect("dependency runtime mutex is not poisoned")
            .mark_changed(DependencyKey::Page(DependencyPageField::SplitDiscards));
    }

    #[must_use]
    pub fn page_contents(&self) -> PageContents {
        self.observe_semantic_dependency(DependencyKey::Page(DependencyPageField::Contents));
        self.page.contents()
    }

    pub fn set_page_contents(&mut self, contents: PageContents) {
        self.page.set_contents(contents);
        self.dependencies
            .get_mut()
            .expect("dependency runtime mutex is not poisoned")
            .mark_changed(DependencyKey::Page(DependencyPageField::Contents));
    }

    #[must_use]
    pub fn page_max_depth(&self) -> Scaled {
        self.observe_semantic_dependency(DependencyKey::Page(DependencyPageField::Contents));
        self.page.page_max_depth()
    }

    #[must_use]
    pub fn insert_penalties(&self) -> i32 {
        self.observe_semantic_dependency(DependencyKey::PageInteger(1));
        self.page.insert_penalties()
    }

    #[must_use]
    pub fn least_page_cost(&self) -> i32 {
        self.observe_semantic_dependency(DependencyKey::Page(DependencyPageField::BreakState));
        self.page.least_page_cost()
    }

    #[must_use]
    pub fn best_page_break(&self) -> Option<PageBreak> {
        self.observe_semantic_dependency(DependencyKey::Page(DependencyPageField::BreakState));
        self.page.best_page_break()
    }

    #[must_use]
    pub fn best_size(&self) -> Scaled {
        self.observe_semantic_dependency(DependencyKey::Page(DependencyPageField::BreakState));
        self.page.best_size()
    }

    pub fn record_best_page_break(&mut self, break_index: usize, best_size: Scaled, cost: i32) {
        self.page.record_best_break(break_index, best_size, cost);
        self.dependencies
            .get_mut()
            .expect("dependency runtime mutex is not poisoned")
            .mark_changed(DependencyKey::Page(DependencyPageField::BreakState));
    }

    pub fn record_page_fire_up(&mut self, trigger_index: usize) {
        self.page.record_fire_up(trigger_index);
        self.dependencies
            .get_mut()
            .expect("dependency runtime mutex is not poisoned")
            .mark_changed(DependencyKey::Page(DependencyPageField::FireUp));
    }

    #[must_use]
    pub fn page_fire_up(&self) -> Option<PageFireUp> {
        self.observe_semantic_dependency(DependencyKey::Page(DependencyPageField::FireUp));
        self.page.fire_up()
    }

    #[doc(hidden)]
    pub fn defer_page_fire_up(&mut self) {
        self.page.defer_fire_up();
        self.dependencies
            .get_mut()
            .expect("dependency runtime mutex is not poisoned")
            .mark_changed(DependencyKey::Page(DependencyPageField::FireUp));
    }

    pub fn append_page_contribution(&mut self, node: Node) {
        self.stores.assert_live_handles_in_node(&node);
        self.page.push_contribution(node);
        self.dependencies
            .get_mut()
            .expect("dependency runtime mutex is not poisoned")
            .mark_changed(DependencyKey::Page(DependencyPageField::Contributions));
    }

    pub fn prepend_page_contribution(&mut self, node: Node) {
        self.stores.assert_live_handles_in_node(&node);
        self.page.prepend_contribution(node);
        self.dependencies
            .get_mut()
            .expect("dependency runtime mutex is not poisoned")
            .mark_changed(DependencyKey::Page(DependencyPageField::Contributions));
    }

    #[must_use]
    pub fn page_contributions(&self) -> &std::collections::VecDeque<Node> {
        self.observe_semantic_dependency(DependencyKey::Page(DependencyPageField::Contributions));
        self.page.contribution()
    }

    #[must_use]
    pub fn page_contribution_front(&self) -> Option<&Node> {
        self.observe_semantic_dependency(DependencyKey::Page(DependencyPageField::Contributions));
        self.page.contribution_front()
    }

    #[must_use]
    pub fn page_contribution_second(&self) -> Option<&Node> {
        self.observe_semantic_dependency(DependencyKey::Page(DependencyPageField::Contributions));
        self.page.contribution_second()
    }

    #[must_use]
    pub fn page_contribution_tail(&self) -> Option<&Node> {
        self.observe_semantic_dependency(DependencyKey::Page(DependencyPageField::Contributions));
        self.page.contribution_tail()
    }

    pub fn pop_page_contribution_front(&mut self) -> Option<Node> {
        let node = self.page.pop_contribution_front();
        self.dependencies
            .get_mut()
            .expect("dependency runtime mutex is not poisoned")
            .mark_changed(DependencyKey::Page(DependencyPageField::Contributions));
        node
    }

    pub fn pop_page_contribution_tail(&mut self) -> Option<Node> {
        let node = self.page.pop_contribution_tail();
        self.dependencies
            .get_mut()
            .expect("dependency runtime mutex is not poisoned")
            .mark_changed(DependencyKey::Page(DependencyPageField::Contributions));
        node
    }

    pub fn remove_page_contribution_range(
        &mut self,
        range: std::ops::RangeInclusive<usize>,
    ) -> Vec<Node> {
        let nodes = self.page.remove_contribution_range(range);
        self.dependencies
            .get_mut()
            .expect("dependency runtime mutex is not poisoned")
            .mark_changed(DependencyKey::Page(DependencyPageField::Contributions));
        nodes
    }

    /// Transfers the outer vertical contribution tail when it is a box.
    ///
    /// This is the page-owned counterpart of TeX's `\lastbox` tail operation:
    /// intervening material is never searched or removed, and a transferred
    /// box loses its previous raise/lower shift before entering a new context.
    pub fn take_page_contribution_last_box(&mut self) -> Option<Node> {
        match self.page.contribution_tail() {
            Some(Node::HList(_)) | Some(Node::VList(_)) => {}
            _ => return None,
        }
        let mut node = self
            .page
            .pop_contribution_tail()
            .expect("contribution tail was just inspected");
        match &mut node {
            Node::HList(box_node) | Node::VList(box_node) => {
                box_node.shift = Scaled::from_raw(0);
            }
            _ => unreachable!("contribution tail was checked to be a box"),
        }
        self.dependencies
            .get_mut()
            .expect("dependency runtime mutex is not poisoned")
            .mark_changed(DependencyKey::Page(DependencyPageField::Contributions));
        Some(node)
    }

    pub fn prepend_page_contributions(&mut self, nodes: Vec<Node>) {
        self.stores.assert_live_handles_in_nodes(&nodes);
        self.page.prepend_contributions(nodes);
        self.dependencies
            .get_mut()
            .expect("dependency runtime mutex is not poisoned")
            .mark_changed(DependencyKey::Page(DependencyPageField::Contributions));
    }

    /// Detaches the complete result root of one page-builder episode.
    #[doc(hidden)]
    pub fn detach_page_memo_transition(
        &mut self,
    ) -> Result<(crate::DetachedMemoValue, Vec<OriginRef>), crate::MemoValueError> {
        let (nodes, state) = self.page.memo_parts();
        let root = self.freeze_node_list(&nodes);
        let (payload, origins) = self
            .stores
            .encode_memo_node_list_with_origins(&root)
            .map_err(|error| crate::MemoValueError::Codec(format!("{error:?}")))?;
        let detached_nodes =
            crate::DetachedMemoValue::from_payload(crate::MemoValueKind::Nodes, payload)
                .to_bytes()?;
        let semantic_payload = bincode::serialize(&PageMemoWire {
            state,
            detached_nodes,
        })
        .map_err(|error| crate::MemoValueError::Codec(error.to_string()))?;
        let transition =
            crate::DetachedMemoValue::from_page_transition(&crate::DetachedPageTransition {
                transition_schema: 1,
                semantic_payload,
            })?;
        Ok((transition, origins))
    }

    /// Captures the current page graph's provenance sequence in detached-codec order.
    #[doc(hidden)]
    pub fn page_memo_origins(&mut self) -> Result<Vec<OriginRef>, crate::MemoValueError> {
        let (nodes, _) = self.page.memo_parts();
        let root = self.freeze_node_list(&nodes);
        self.stores
            .encode_memo_node_list_with_origins(&root)
            .map(|(_, origins)| origins)
            .map_err(|error| crate::MemoValueError::Codec(format!("{error:?}")))
    }

    /// Captures one shipout root's provenance sequence in detached-codec order.
    #[doc(hidden)]
    pub fn node_memo_origins(
        &mut self,
        node: &Node,
    ) -> Result<Vec<OriginRef>, crate::MemoValueError> {
        let root = self.freeze_node_list(std::slice::from_ref(node));
        self.stores
            .encode_memo_node_list_with_origins(&root)
            .map(|(_, origins)| origins)
            .map_err(|error| crate::MemoValueError::Codec(format!("{error:?}")))
    }

    /// Publishes already-verified artifact bytes through the ordinary shipout
    /// transaction, preserving effect-prefix and dead-cycle semantics.
    #[doc(hidden)]
    pub fn commit_replayed_artifact(
        &mut self,
        bytes: Vec<u8>,
        render_origin_ends: Vec<u32>,
        render_provenance: crate::OutputProvenanceRecipe,
        receipt: Option<crate::PageOutputPublicationReceiptId>,
    ) -> Result<
        (
            ContentHash,
            crate::PageOutputPublicationReceipt,
            crate::ArtifactPublicationRecord,
        ),
        WorldError,
    > {
        let effect_pos = self.world.effect_pos();
        let effect_index = self.world.effect_records().len();
        let reservation = self
            .world
            .reserve_active_artifact_publication_at(effect_index, receipt);
        let transaction = self.begin_shipout();
        let (hash, publication) = transaction.commit(
            crate::VerifiedArtifact::new(bytes)
                .with_deferred_render_origins(render_origin_ends, render_provenance),
            effect_pos,
            reservation,
        )?;
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

    /// Imports and atomically publishes a detached page-builder result root.
    #[doc(hidden)]
    pub fn import_page_memo_transition(
        &mut self,
        value: &crate::DetachedMemoValue,
        limits: crate::MemoValueLimits,
        origins: &[OriginRef],
    ) -> Result<(), crate::MemoValueError> {
        let transition = value.page_transition(limits)?;
        if transition.transition_schema != 1 {
            return Err(crate::MemoValueError::Invalid(
                "unsupported page transition schema",
            ));
        }
        let wire: PageMemoWire = bincode::deserialize(&transition.semantic_payload)
            .map_err(|error| crate::MemoValueError::Codec(error.to_string()))?;
        let detached = crate::DetachedMemoValue::from_bytes(&wire.detached_nodes, limits)?;
        let rollback = self.capture_scoped_rollback();
        let payload = detached.payload(crate::MemoValueKind::Nodes)?;
        let imported = match self.stores.import_memo_node_list_with_origins(
            payload,
            limits.max_nodes,
            limits.max_tokens,
            limits.max_string_bytes,
            origins,
        ) {
            Ok(imported) => imported,
            Err(error) => {
                self.rollback_scoped(rollback);
                return Err(crate::MemoValueError::Codec(format!("{error:?}")));
            }
        };
        let nodes = imported.to_vec();
        if let Err(error) = self.page.install_memo_parts(nodes, wire.state) {
            self.rollback_scoped(rollback);
            return Err(error);
        }
        self.dependencies
            .get_mut()
            .expect("dependency runtime mutex is not poisoned")
            .mark_changed(DependencyKey::Page(DependencyPageField::Contributions));
        self.dependencies
            .get_mut()
            .expect("dependency runtime mutex is not poisoned")
            .mark_changed(DependencyKey::Page(DependencyPageField::CurrentPage));
        for dimension in 0..8 {
            self.dependencies
                .get_mut()
                .expect("dependency runtime mutex is not poisoned")
                .mark_changed(DependencyKey::PageDimension(dimension));
        }
        self.dependencies
            .get_mut()
            .expect("dependency runtime mutex is not poisoned")
            .mark_changed(DependencyKey::Page(DependencyPageField::Insertions));
        self.dependencies
            .get_mut()
            .expect("dependency runtime mutex is not poisoned")
            .mark_changed(DependencyKey::Page(DependencyPageField::BreakState));
        self.dependencies
            .get_mut()
            .expect("dependency runtime mutex is not poisoned")
            .mark_changed(DependencyKey::Page(DependencyPageField::FireUp));
        Ok(())
    }

    #[must_use]
    pub fn current_page_nodes(&self) -> Vec<Node> {
        self.observe_semantic_dependency(DependencyKey::Page(DependencyPageField::CurrentPage));
        self.page.current_page().cloned().collect()
    }

    #[must_use]
    pub fn current_page_tail(&self) -> Option<&Node> {
        self.observe_semantic_dependency(DependencyKey::Page(DependencyPageField::CurrentPage));
        self.page.current_page_tail()
    }

    #[must_use]
    pub fn current_page_len(&self) -> usize {
        self.observe_semantic_dependency(DependencyKey::Page(DependencyPageField::CurrentPage));
        self.page.current_page_len()
    }

    pub fn push_current_page_node(&mut self, node: Node) {
        self.stores.assert_live_handles_in_node(&node);
        self.page.push_current_page(node);
        self.dependencies
            .get_mut()
            .expect("dependency runtime mutex is not poisoned")
            .mark_changed(DependencyKey::Page(DependencyPageField::CurrentPage));
    }

    #[must_use]
    pub fn page_insertions(&self) -> &[PageInsertion] {
        self.observe_semantic_dependency(DependencyKey::Page(DependencyPageField::Insertions));
        self.page.page_insertions()
    }

    #[must_use]
    pub fn page_insertion(&self, class: u16) -> Option<PageInsertion> {
        self.observe_semantic_dependency(DependencyKey::Page(DependencyPageField::Insertions));
        self.page.page_insertion(class)
    }

    #[must_use]
    pub fn page_insertion_height(&self, class: u16) -> Option<Scaled> {
        self.observe_semantic_dependency(DependencyKey::Page(DependencyPageField::Insertions));
        self.page
            .page_insertion(class)
            .map(|insertion| insertion.height())
    }

    pub fn upsert_page_insertion(&mut self, insertion: PageInsertion) {
        self.page.upsert_page_insertion(insertion);
        self.dependencies
            .get_mut()
            .expect("dependency runtime mutex is not poisoned")
            .mark_changed(DependencyKey::Page(DependencyPageField::Insertions));
    }

    pub fn take_current_page_prefix(&mut self, split_index: usize) -> (Vec<Node>, Vec<Node>) {
        let split = self.page.take_current_page_prefix(split_index);
        self.dependencies
            .get_mut()
            .expect("dependency runtime mutex is not poisoned")
            .mark_changed(DependencyKey::Page(DependencyPageField::CurrentPage));
        split
    }

    pub fn update_page_last_from_node(&mut self, node: &Node) {
        self.page.update_last_from_node(node);
        self.dependencies
            .get_mut()
            .expect("dependency runtime mutex is not poisoned")
            .mark_changed(DependencyKey::Page(DependencyPageField::CurrentPage));
    }

    #[must_use]
    pub fn page_last_skip(&self) -> GlueSpec {
        self.observe_semantic_dependency(DependencyKey::Page(DependencyPageField::CurrentPage));
        self.page.last_skip()
    }

    #[must_use]
    pub fn page_last_penalty(&self) -> i32 {
        self.observe_semantic_dependency(DependencyKey::Page(DependencyPageField::CurrentPage));
        self.page.last_penalty()
    }

    #[must_use]
    pub fn page_last_kern(&self) -> Scaled {
        self.observe_semantic_dependency(DependencyKey::Page(DependencyPageField::CurrentPage));
        self.page.last_kern()
    }

    #[must_use]
    pub fn page_last_node_type(&self) -> i32 {
        self.observe_semantic_dependency(DependencyKey::Page(DependencyPageField::CurrentPage));
        self.page.last_node_type()
    }

    /// See [`crate::page::PageBuilderState::has_last_glue`].
    #[must_use]
    pub fn page_has_last_glue(&self) -> bool {
        self.observe_semantic_dependency(DependencyKey::Page(DependencyPageField::CurrentPage));
        self.page.has_last_glue()
    }

    /// Destructively takes a box while moving its structural owner to the
    /// caller.
    pub fn take_box_reg_ref_same_level(&mut self, index: u16) -> Option<NodeListRef> {
        let (value, receipt) = self.stores.take_box_reg_ref_same_level_with_receipt(index);
        self.consume_env_mutation(receipt);
        value
    }

    /// Destructively takes a local box while moving structural ownership to
    /// the caller.
    pub fn take_box_reg_ref(&mut self, index: u16) -> Option<NodeListRef> {
        let (value, receipt) = self.stores.take_box_reg_ref_with_receipt(index);
        self.consume_env_mutation(receipt);
        value
    }

    /// Moves compatible box children out, then clears the register with
    /// same-level TeX assignment semantics.
    ///
    /// Compatibility is checked before mutation, and the child owner is cloned
    /// directly from the register payload. The outer
    /// one-node box wrapper is deliberately not retained by the consumer.
    pub fn take_unbox_children_same_level(
        &mut self,
        index: u16,
        expected: UnboxKind,
    ) -> TakeUnboxResult {
        let Some(value) = self.stores.box_reg_ref(index) else {
            return TakeUnboxResult::Void;
        };
        let nodes = value.nodes();
        if nodes.len() != 1 {
            return TakeUnboxResult::Incompatible;
        }
        let children = match (expected, nodes.first()) {
            (UnboxKind::Horizontal, Some(crate::node_arena::NodeRef::HList(box_node)))
            | (UnboxKind::Vertical, Some(crate::node_arena::NodeRef::VList(box_node))) => value
                .resolve(box_node.children)
                .expect("box child belongs to the register owner"),
            _ => return TakeUnboxResult::Incompatible,
        };
        let (taken, receipt) = self.stores.take_box_reg_ref_same_level_with_receipt(index);
        debug_assert_eq!(taken.as_ref().map(NodeListRef::id), Some(value.id()));
        self.consume_env_mutation(receipt);
        TakeUnboxResult::Children(children)
    }

    pub fn set_box_reg_same_level(&mut self, index: u16, value: NodeListRef) {
        let receipt = self.stores.write_box_reg_ref_same_level(index, Some(value));
        self.consume_env_mutation(receipt);
    }

    pub fn clear_box_reg(&mut self, index: u16) {
        let receipt = self.stores.clear_box_reg(index);
        self.consume_env_mutation(receipt);
    }

    pub fn clear_box_reg_global(&mut self, index: u16) {
        let receipt = self.stores.clear_box_reg_global(index);
        self.consume_env_mutation(receipt);
    }

    pub fn clear_box_reg_same_level(&mut self, index: u16) {
        let receipt = self.stores.clear_box_reg_same_level(index);
        self.consume_env_mutation(receipt);
    }

    #[must_use]
    pub fn box_dimension(&self, index: u16, dimension: BoxDimension) -> Option<Scaled> {
        let root = self.box_reg_ref(index)?;
        box_dimension_from_nodes(root.nodes(), dimension)
    }

    /// Reads pdfTeX's character-protrusion kern at one edge of an hbox.
    ///
    /// `None` distinguishes a void or non-horizontal box from a valid hbox
    /// whose queried edge has no margin kern (which returns zero).
    #[must_use]
    pub fn box_margin_kern(&self, index: u16, side: MarginKernSide) -> Option<Scaled> {
        let root = self.box_reg_ref(index)?;
        let nodes = root.nodes();
        let box_node = match (nodes.len(), nodes.first()) {
            (1, Some(crate::node_arena::NodeRef::HList(box_node))) => box_node,
            _ => return None,
        };
        let children_owner = root
            .resolve(box_node.children)
            .expect("box children belong to the register owner");
        let children = children_owner.nodes();
        let mut edge = children.iter();
        let mut next = || match side {
            MarginKernSide::Left => edge.next(),
            MarginKernSide::Right => edge.next_back(),
        };
        let candidate = loop {
            let candidate = next();
            match candidate {
                Some(node) if margin_kern_enquiry_skipable(&children_owner, &node, side) => {}
                _ => break candidate,
            }
        };
        Some(match candidate {
            Some(crate::node_arena::NodeRef::MarginKern {
                amount,
                side: candidate_side,
                ..
            }) if candidate_side == side => amount,
            _ => Scaled::from_raw(0),
        })
    }

    pub fn set_box_dimension(&mut self, index: u16, dimension: BoxDimension, value: Scaled) {
        self.set_box_dimension_impl(index, dimension, value);
    }

    pub fn set_box_dimension_global(&mut self, index: u16, dimension: BoxDimension, value: Scaled) {
        // TeX82's `alter_box_dimen` mutates the visible box node directly;
        // assignment prefixes do not change the binding level of the box.
        self.set_box_dimension_impl(index, dimension, value);
    }

    fn set_box_dimension_impl(&mut self, index: u16, dimension: BoxDimension, value: Scaled) {
        let Some(root) = self.box_reg_ref(index) else {
            return;
        };
        let Some(mut node) = root.to_vec().into_iter().next() else {
            return;
        };
        if !set_box_dimension_in_node(&mut node, dimension, value) {
            return;
        }
        let rewritten = self.freeze_node_list(&[node]);
        self.set_box_reg_same_level(index, rewritten);
    }

    pub fn set_int_param(&mut self, param: IntParam, value: i32) {
        let receipt = self.stores.set_int_param(param, value);
        self.consume_env_mutation(receipt);
    }

    pub fn set_int_param_global(&mut self, param: IntParam, value: i32) {
        let receipt = self.stores.set_int_param_global(param, value);
        self.consume_env_mutation(receipt);
    }

    #[must_use]
    pub fn int_param(&self, param: IntParam) -> i32 {
        self.observe_cell_dependency(BankTag::IntParam, u32::from(param.raw()));
        self.stores.int_param(param)
    }

    #[must_use]
    pub fn last_badness(&self) -> i32 {
        self.stores.last_badness()
    }

    pub fn set_last_badness(&mut self, value: i32) {
        let receipt = self.stores.set_last_badness(value);
        self.consume_env_mutation(receipt);
    }

    #[must_use]
    pub fn mag(&self) -> i32 {
        self.stores.mag()
    }

    pub fn set_mag(&mut self, value: i32) {
        let receipt = self.stores.set_mag(value);
        self.consume_env_mutation(receipt);
    }

    pub fn set_mag_global(&mut self, value: i32) {
        let receipt = self.stores.set_mag_global(value);
        self.consume_env_mutation(receipt);
    }

    #[must_use]
    pub fn prepared_mag(&self) -> Option<i32> {
        self.stores.prepared_mag()
    }

    pub fn prepare_mag(&mut self) -> (i32, Option<PrepareMagDiagnostic>) {
        let (result, receipts) = self.stores.prepare_mag_with_receipts();
        self.consume_env_mutations(receipts);
        result
    }

    #[must_use]
    pub fn endlinechar(&self) -> i32 {
        self.stores.endlinechar()
    }

    pub fn set_dimen_param(&mut self, param: DimenParam, value: Scaled) {
        let receipt = self.stores.set_dimen_param(param, value);
        self.consume_env_mutation(receipt);
    }

    pub fn set_dimen_param_global(&mut self, param: DimenParam, value: Scaled) {
        let receipt = self.stores.set_dimen_param_global(param, value);
        self.consume_env_mutation(receipt);
    }

    #[must_use]
    pub fn dimen_param(&self, param: DimenParam) -> Scaled {
        self.observe_cell_dependency(BankTag::DimenParam, u32::from(param.raw()));
        self.stores.dimen_param(param)
    }

    pub fn set_glue_param(&mut self, param: GlueParam, value: impl crate::glue::GlueHandle) {
        let receipt = self.stores.set_glue_param(param, value);
        self.consume_env_mutation(receipt);
    }

    #[must_use]
    pub fn glue_param(&self, param: GlueParam) -> GlueId {
        self.observe_cell_dependency(BankTag::GlueParam, u32::from(param.raw()));
        self.stores.glue_param(param)
    }

    pub fn set_glue_param_global(&mut self, param: GlueParam, value: impl crate::glue::GlueHandle) {
        let receipt = self.stores.set_glue_param_global(param, value);
        self.consume_env_mutation(receipt);
    }

    pub fn set_tok_param(&mut self, param: TokParam, value: TokenListId) {
        self.set_tok_param_option(param, Some(value));
    }

    /// Sets a token-list parameter without conflating TeX's null pointer with
    /// a present pointer to an empty list.
    pub fn set_tok_param_option(&mut self, param: TokParam, value: Option<TokenListId>) {
        let receipt = self.stores.set_tok_param_option(param, value);
        self.consume_env_mutation(receipt);
    }

    #[must_use]
    pub fn tok_param(&self, param: TokParam) -> TokenListId {
        self.observe_cell_dependency(BankTag::TokParam, u32::from(param.raw()));
        self.stores.tok_param(param)
    }

    /// Returns a token-list parameter while preserving an unassigned null cell.
    #[must_use]
    pub fn tok_param_option(&self, param: TokParam) -> Option<TokenListId> {
        self.observe_cell_dependency(BankTag::TokParam, u32::from(param.raw()));
        self.stores.tok_param_option(param)
    }

    pub fn set_tok_param_global(&mut self, param: TokParam, value: TokenListId) {
        self.set_tok_param_option_global(param, Some(value));
    }

    /// Globally sets a token-list parameter while preserving a null pointer.
    pub fn set_tok_param_option_global(&mut self, param: TokParam, value: Option<TokenListId>) {
        let receipt = self.stores.set_tok_param_option_global(param, value);
        self.consume_env_mutation(receipt);
    }

    /// Returns the current barriered, group-scoped `\parshape` value.
    #[must_use]
    pub fn paragraph_shape(&self) -> Vec<ParagraphShapeLine> {
        self.observe_semantic_dependency(DependencyKey::Engine(DependencyEngineField::ParShape));
        let id = self.tok_param(TokParam::PAR_SHAPE_INTERNAL);
        let tokens = self.tokens(id);
        assert_eq!(
            tokens.len() % 8,
            0,
            "internal parshape payload is truncated"
        );
        tokens
            .chunks_exact(8)
            .map(|chunk| {
                let mut raw = [0_u8; 8];
                for (byte, token) in raw.iter_mut().zip(chunk) {
                    let Token::Param(value) = token else {
                        panic!("internal parshape payload has a non-byte token");
                    };
                    *byte = *value;
                }
                ParagraphShapeLine {
                    indent: Scaled::from_raw(i32::from_le_bytes([raw[0], raw[1], raw[2], raw[3]])),
                    width: Scaled::from_raw(i32::from_le_bytes([raw[4], raw[5], raw[6], raw[7]])),
                }
            })
            .collect()
    }

    /// Returns the number of lines in the current barriered `\parshape`
    /// without materializing its decoded line pairs.
    #[must_use]
    pub fn paragraph_shape_len(&self) -> usize {
        self.observe_semantic_dependency(DependencyKey::Engine(DependencyEngineField::ParShape));
        let tokens = self.tokens(self.tok_param(TokParam::PAR_SHAPE_INTERNAL));
        assert_eq!(
            tokens.len() % 8,
            0,
            "internal parshape payload is truncated"
        );
        tokens.len() / 8
    }

    /// Returns one current `\parshape` component, repeating the final line
    /// for positive indexes beyond the explicitly assigned shape.
    #[must_use]
    pub fn paragraph_shape_dimension(&self, line: i32, width: bool) -> Scaled {
        self.observe_semantic_dependency(DependencyKey::Engine(DependencyEngineField::ParShape));
        if line <= 0 {
            return Scaled::from_raw(0);
        }
        let tokens = self.tokens(self.tok_param(TokParam::PAR_SHAPE_INTERNAL));
        let line_count = tokens.len() / 8;
        if line_count == 0 {
            return Scaled::from_raw(0);
        }
        let line = (line as usize).min(line_count) - 1;
        let start = line * 8 + usize::from(width) * 4;
        let mut raw = [0_u8; 4];
        for (byte, token) in raw.iter_mut().zip(&tokens[start..start + 4]) {
            let Token::Param(value) = token else {
                panic!("internal parshape payload has a non-byte token");
            };
            *byte = *value;
        }
        Scaled::from_raw(i32::from_le_bytes(raw))
    }

    /// Assigns TeX's `\parshape` through the ordinary group write barrier.
    pub fn set_paragraph_shape(&mut self, lines: &[ParagraphShapeLine], global: bool) {
        // TeX82 §1090 clears `par_shape_ptr` only when it is non-null. A
        // loaded format can represent the same null shape with a frozen
        // token-list handle, so test the typed effective shape rather than
        // rewriting that representation through `eq_define`. This preserves
        // §283's distinction between no save-stack entry and restoration of a
        // genuinely non-null shape at group exit.
        let representation_only_null = if lines.is_empty() {
            let cell = crate::cell::CellId::new(
                crate::cell::BankTag::TokParam,
                u32::from(TokParam::PAR_SHAPE_INTERNAL.raw()),
            );
            let effective = self.stores.effective_restored_env_word(cell);
            restored_tok_param_tokens(self, effective)
                .as_deref()
                .is_none_or(<[Token]>::is_empty)
        } else {
            false
        };
        let mut tokens = Vec::with_capacity(lines.len().saturating_mul(8));
        for line in lines {
            tokens.extend(
                line.indent
                    .raw()
                    .to_le_bytes()
                    .into_iter()
                    .chain(line.width.raw().to_le_bytes())
                    .map(Token::Param),
            );
        }
        let root = self.intern_token_list_ref(&tokens);
        let id = root.id();
        if representation_only_null {
            let receipt = self.stores.rewrite_null_parshape_representation(id);
            self.consume_env_mutation(receipt);
        } else if global {
            self.set_tok_param_global(TokParam::PAR_SHAPE_INTERNAL, id);
        } else {
            self.set_tok_param(TokParam::PAR_SHAPE_INTERNAL, id);
        }
    }

    /// Returns a current e-TeX penalty array through the state facade.
    #[must_use]
    pub fn penalty_array(&self, kind: PenaltyArrayKind) -> Vec<i32> {
        self.observe_semantic_dependency(DependencyKey::Engine(
            DependencyEngineField::PenaltyArrays,
        ));
        let tokens = self.tokens(self.tok_param(kind.storage()));
        assert_eq!(tokens.len() % 4, 0, "internal penalty array is truncated");
        tokens
            .chunks_exact(4)
            .map(|chunk| {
                let mut raw = [0_u8; 4];
                for (byte, token) in raw.iter_mut().zip(chunk) {
                    let Token::Param(value) = token else {
                        panic!("internal penalty array has a non-byte token");
                    };
                    *byte = *value;
                }
                i32::from_le_bytes(raw)
            })
            .collect()
    }

    /// Implements e-TeX's numeric penalty-array enquiry: zero returns the
    /// length and positive indexes repeat the last explicitly assigned value.
    #[must_use]
    pub fn penalty_array_value(&self, kind: PenaltyArrayKind, index: i32) -> i32 {
        self.observe_semantic_dependency(DependencyKey::Engine(
            DependencyEngineField::PenaltyArrays,
        ));
        let tokens = self.tokens(self.tok_param(kind.storage()));
        let len = tokens.len() / 4;
        if index <= 0 || len == 0 {
            return if index == 0 { len as i32 } else { 0 };
        }
        let index = (index as usize).min(len) - 1;
        let mut raw = [0_u8; 4];
        for (byte, token) in raw.iter_mut().zip(&tokens[index * 4..index * 4 + 4]) {
            let Token::Param(value) = token else {
                panic!("internal penalty array has a non-byte token");
            };
            *byte = *value;
        }
        i32::from_le_bytes(raw)
    }

    /// Assigns an e-TeX penalty array through the ordinary group barrier.
    pub fn set_penalty_array(&mut self, kind: PenaltyArrayKind, values: &[i32], global: bool) {
        // e-TeX [19.277]/[49.1248] represents an empty array by the null
        // shape pointer. An identical local null assignment returns before
        // `eq_save`, so it must not create a restore record in Umber either.
        if !global && values.is_empty() && self.penalty_array(kind).is_empty() {
            return;
        }
        let mut tokens = Vec::with_capacity(values.len().saturating_mul(4));
        for value in values {
            tokens.extend(value.to_le_bytes().into_iter().map(Token::Param));
        }
        let root = self.intern_token_list_ref(&tokens);
        let id = root.id();
        if global {
            self.set_tok_param_global(kind.storage(), id);
        } else {
            self.set_tok_param(kind.storage(), id);
        }
    }

    #[must_use]
    pub fn env_journal_bytes_since(&self, snapshot: &Snapshot) -> usize {
        self.assert_valid_snapshot(snapshot);
        self.stores.env_journal_bytes_since(&snapshot.store)
    }

    /// Current live bytes retained by the environment mutation journal.
    #[must_use]
    pub fn env_journal_bytes(&self) -> usize {
        self.stores.env_journal_bytes()
    }

    #[cfg(any(test, feature = "testing"))]
    pub fn env_journal_entry_count(&self) -> usize {
        self.stores.env_journal_entry_count()
    }

    #[cfg(feature = "shadow")]
    pub fn verify_shadow(&self) {
        self.stores.verify_shadow();
    }

    #[cfg(any(test, feature = "testing"))]
    pub fn testing_clear_state_hash_caches(&mut self) {
        self.stores.testing_clear_semantic_hash_cache();
        self.state_hash_projection_cache = StateHashProjectionCache::default();
    }

    #[cfg(test)]
    fn testing_input_projection_hash_calls(&self) -> usize {
        self.state_hash_projection_cache.input_hash_calls
    }

    /// Exact live-owner categories plus bounded weak/allocator metadata.
    ///
    /// This projection is test-only and has no role in semantic identity,
    /// reachability, acceptance, or cache authority.
    #[cfg(any(test, feature = "testing"))]
    #[must_use]
    pub fn testing_ownership_census(&self) -> TestingOwnershipCensus {
        self.stores.testing_ownership_census()
    }

    /// Exact private-domain ownership used by cross-crate operation controls.
    #[cfg(any(test, feature = "testing"))]
    #[must_use]
    pub fn testing_private_revision_domain_stats(&self) -> Option<(usize, usize, usize, bool)> {
        self.private_revision_domain.as_ref().map(|domain| {
            let stats = domain.stats();
            (
                stats.allocations,
                stats.logical_bytes,
                stats.slot_capacity_bytes,
                stats.operation_active,
            )
        })
    }

    /// Returns a non-owning liveness probe for candidate rejection tests.
    #[cfg(any(test, feature = "testing"))]
    #[must_use]
    pub fn testing_private_revision_domain_probe(
        &self,
    ) -> Option<crate::TestingPrivateRevisionDomainProbe> {
        self.private_revision_domain
            .as_ref()
            .map(PatchAllocationDomain::testing_probe)
    }

    /// Arms one exact allocation inside the next real aggregate operation.
    #[cfg(any(test, feature = "testing"))]
    pub fn testing_arm_next_private_revision_operation_allocation(&mut self, bytes: usize) {
        self.private_revision_domain
            .as_mut()
            .expect("testing allocation requires a private revision")
            .testing_arm_next_operation_allocation(bytes);
    }

    /// Allocates exact charged bytes in the active private operation. The
    /// payload has no semantic consumer and exists only to prove aggregate
    /// operation and candidate lifecycle behavior.
    #[cfg(any(test, feature = "testing"))]
    pub fn testing_allocate_private_revision_bytes(&mut self, bytes: usize) {
        self.private_revision_domain
            .as_mut()
            .expect("testing allocation requires a private revision")
            .allocate(vec![0_u8; bytes].into_boxed_slice(), bytes)
            .expect("testing allocation requires an active operation");
    }

    /// Commits one synthetic allocation as earlier successful private work.
    #[cfg(any(test, feature = "testing"))]
    pub fn testing_commit_private_revision_allocation(&mut self, bytes: usize) {
        let domain = self
            .private_revision_domain
            .as_mut()
            .expect("testing allocation requires a private revision");
        let mark = domain
            .begin_operation()
            .expect("testing allocation requires no active operation");
        domain
            .allocate(vec![0_u8; bytes].into_boxed_slice(), bytes)
            .expect("testing allocation belongs to the synthetic operation");
        domain
            .commit_operation(mark)
            .expect("synthetic allocation operation commits");
    }
}

fn format_restore_scaled(value: crate::scaled::Scaled) -> String {
    let raw = i64::from(value.raw());
    let sign = if raw < 0 { "-" } else { "" };
    let magnitude = raw.abs();
    let whole = magnitude / 65_536;
    let fraction = magnitude % 65_536;
    if fraction == 0 {
        return format!("{sign}{whole}.0");
    }
    let mut digits = format!("{:05}", (fraction * 100_000 + 32_768) / 65_536);
    while digits.ends_with('0') {
        digits.pop();
    }
    format!("{sign}{whole}.{digits}")
}

fn escaped_restore_name(escape_char: i32, name: &str) -> String {
    let mut escaped = String::new();
    if let Ok(byte) = u8::try_from(escape_char) {
        escaped.push(char::from(byte));
    }
    escaped.push_str(name);
    escaped
}

/// TeX82 §263's `sprint_cs` spelling under the saved `\escapechar` value.
fn sprint_restore_name(
    escape_char: i32,
    kind: crate::interner::ControlSequenceKind,
    name: &str,
) -> String {
    use crate::interner::ControlSequenceKind;

    match kind {
        ControlSequenceKind::ActiveCharacter => name.to_owned(),
        ControlSequenceKind::Null => format!(
            "{}{}",
            escaped_restore_name(escape_char, "csname"),
            escaped_restore_name(escape_char, "endcsname")
        ),
        ControlSequenceKind::SingleCharacter
        | ControlSequenceKind::Named
        | ControlSequenceKind::Internal => escaped_restore_name(escape_char, name),
    }
}

/// TeX82 §§252/262's bounded macro half of `show_eqtb`.
fn append_bounded_macro_body(
    universe: &Universe,
    parameter_text: TokenListId,
    replacement_text: TokenListId,
    escape_char: i32,
    text: &mut String,
) {
    let mut tally = 0;
    let parameter_tokens = universe.tokens(parameter_text);
    let mut parameter = parameter_tokens.iter();
    while tally < 32
        && let Some(&token) = parameter.next()
    {
        let before = text.chars().count();
        crate::token_show::append_token_show_text(universe, token, text);
        tally += text.chars().count() - before;
    }
    let mut remaining = parameter.next().is_some();
    if !remaining {
        if tally < 32 {
            text.push_str("->");
            tally += 2;
            let replacement_tokens = universe.tokens(replacement_text);
            let mut replacement = replacement_tokens.iter();
            while tally < 32
                && let Some(&token) = replacement.next()
            {
                let before = text.chars().count();
                crate::token_show::append_token_show_text(universe, token, text);
                tally += text.chars().count() - before;
            }
            remaining = replacement.next().is_some();
        } else {
            remaining = true;
        }
    }
    if remaining {
        text.push_str(&escaped_restore_name(escape_char, "ETC."));
    }
}

/// A mutable dimension field of a box register's top-level box.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum BoxDimension {
    Width,
    Height,
    Depth,
}

/// Box-list kind expected by a destructive unbox operation.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum UnboxKind {
    Horizontal,
    Vertical,
}

/// Outcome of a destructive unbox transfer.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TakeUnboxResult {
    Void,
    Incompatible,
    Children(NodeListRef),
}

fn box_dimension_from_nodes(nodes: NodeList<'_>, dimension: BoxDimension) -> Option<Scaled> {
    let box_node = match (nodes.len(), nodes.first()) {
        (
            1,
            Some(
                crate::node_arena::NodeRef::HList(box_node)
                | crate::node_arena::NodeRef::VList(box_node),
            ),
        ) => box_node,
        _ => return None,
    };
    Some(match dimension {
        BoxDimension::Width => box_node.width,
        BoxDimension::Height => box_node.height,
        BoxDimension::Depth => box_node.depth,
    })
}

/// pdftex.web §470's edge traversal, using its `cp_skipable` predicate.
fn margin_kern_enquiry_skipable(
    owner: &NodeListRef,
    node: &crate::node_arena::NodeRef<'_>,
    side: MarginKernSide,
) -> bool {
    use crate::node_arena::NodeRef;

    match node {
        NodeRef::Ins { .. } | NodeRef::Mark { .. } | NodeRef::Adjust(_) | NodeRef::Penalty(_) => {
            true
        }
        NodeRef::Whatsit(Whatsit::PdfRefXImage { .. } | Whatsit::PdfRefXForm { .. }) => false,
        NodeRef::Whatsit(_) => true,
        NodeRef::Disc {
            pre,
            post,
            replace,
            physical_replace_count,
            ..
        } => {
            *physical_replace_count == 0
                && owner.resolve(*pre).is_some_and(|list| list.is_empty())
                && owner.resolve(*post).is_some_and(|list| list.is_empty())
                && owner.resolve(*replace).is_some_and(|list| list.is_empty())
        }
        NodeRef::MathOn(width) | NodeRef::MathOff(width) => width.raw() == 0,
        NodeRef::Kern { amount, kind } => {
            amount.raw() == 0 || matches!(kind, KernKind::Font | KernKind::Auto)
        }
        NodeRef::Glue { spec, kind, .. } => {
            spec.spec() == GlueSpec::ZERO
                || matches!(
                    (side, kind),
                    (MarginKernSide::Left, GlueKind::LeftSkip)
                        | (MarginKernSide::Right, GlueKind::RightSkip)
                )
        }
        NodeRef::HList(box_node) => {
            box_node.width.raw() == 0
                && box_node.height.raw() == 0
                && box_node.depth.raw() == 0
                && owner
                    .resolve(box_node.children)
                    .is_some_and(|list| list.is_empty())
        }
        _ => false,
    }
}

fn set_box_dimension_in_node(node: &mut Node, dimension: BoxDimension, value: Scaled) -> bool {
    let box_node = match node {
        Node::HList(box_node) | Node::VList(box_node) => box_node,
        _ => return false,
    };
    match dimension {
        BoxDimension::Width => box_node.width = value,
        BoxDimension::Height => box_node.height = value,
        BoxDimension::Depth => box_node.depth = value,
    }
    true
}

impl InputReadState for InputOpenContext<'_> {
    fn read_input_file(
        &mut self,
        path: &std::path::Path,
    ) -> Result<crate::FileContent, crate::WorldError> {
        self.universe.world_mut().read_file(path)
    }

    fn read_pending_output_file(
        &mut self,
        path: &std::path::Path,
    ) -> Result<Option<crate::FileContent>, crate::WorldError> {
        self.universe.world_mut().read_pending_output_file(path)
    }

    fn read_supplied_input_file(
        &mut self,
        path: &std::path::Path,
        bytes: std::sync::Arc<[u8]>,
    ) -> Result<crate::FileContent, crate::WorldError> {
        self.universe.world_mut().read_supplied_file(path, bytes)
    }

    fn record_input_dependency(
        &mut self,
        path: &std::path::Path,
        outcome: crate::InputDependencyOutcome,
        access: crate::InputDependencyAccess,
    ) -> Result<(), crate::WorldError> {
        self.universe
            .world_mut()
            .record_input_dependency(path, outcome, access)
    }
}

impl InputOpenState for Universe {
    type Input<'a>
        = InputOpenContext<'a>
    where
        Self: 'a;

    fn input_open_context(&mut self) -> Self::Input<'_> {
        InputOpenContext::new(self)
    }
}

fn hash_stream_bufs(streams: &StreamBufState, hasher: &mut StateHasher) {
    hasher.tag(0x83);
    for raw in 0..crate::world::STREAM_SLOT_COUNT as u8 {
        let slot = StreamSlot::new(raw);
        match streams.read_stream_target(slot) {
            Some(target) => {
                hasher.bool(true);
                hash_path(target.path(), hasher);
                hasher.bytes(&target.hash().bytes());
                hasher.usize(target.next_byte());
            }
            None => hasher.bool(false),
        }
        match streams.write_stream_target(slot) {
            Some(target) => {
                hasher.bool(true);
                hash_path(target.path(), hasher);
            }
            None => hasher.bool(false),
        }
        hasher.str(streams.partial_line(slot));
    }
    hasher.str(streams.log_partial_line());
    hasher.str(streams.terminal_partial_line());
}

fn world_backed_dependency_value(world: &World, key: DependencyKey) -> Option<DependencyValue> {
    match key {
        DependencyKey::World { field, index } => world.dependency_value(field, index),
        DependencyKey::InputStream(raw) if usize::from(raw) < crate::world::STREAM_SLOT_COUNT => {
            let stream = StreamSlot::new(raw);
            Some(world.input_stream_dependency(stream).map_or(
                DependencyValue::Absent,
                |(content, cursor)| {
                    let fragment =
                        StateHashFragment::from_exact_builder(0x776f_726c_645f_6973, |hash| {
                            hash.bytes(&content.bytes());
                            hash.u64(cursor);
                        });
                    DependencyValue::Projection {
                        schema: 1,
                        fingerprint: fragment.fingerprint(),
                    }
                },
            ))
        }
        DependencyKey::Engine(DependencyEngineField::PdfTimer) => Some(DependencyValue::Integer(
            i64::from(world.pdf_elapsed_time()),
        )),
        DependencyKey::Engine(DependencyEngineField::PdfRandom) => {
            let fragment = StateHashFragment::from_exact_builder(0x776f_726c_645f_7072, |hash| {
                hash_pdf_random_state(world, hash)
            });
            Some(DependencyValue::Projection {
                schema: 1,
                fingerprint: fragment.fingerprint(),
            })
        }
        DependencyKey::Engine(DependencyEngineField::PdfShellEscape) => Some(
            DependencyValue::Integer(match world.shell_escape_policy() {
                ShellEscapePolicy::Disabled => 0,
                ShellEscapePolicy::Enabled => 1,
                ShellEscapePolicy::Restricted => 2,
            }),
        ),
        _ => None,
    }
}

fn hash_rng_state(rng: crate::world::RngState, hasher: &mut StateHasher) {
    hasher.tag(0x84);
    for word in rng.state_words() {
        hasher.u64(word);
    }
}

fn hash_pdf_random_state(world: &World, hasher: &mut StateHasher) {
    let (seed, next, values) = world.pdf_random_state();
    hasher.tag(0x92);
    hasher.i32(seed);
    hasher.usize(next);
    for value in values {
        hasher.i32(value);
    }
}

fn hash_pdf_timer_state(world: &World, hasher: &mut StateHasher) {
    let (current, origin) = world.pdf_timer_state();
    hasher.tag(0x93);
    hasher.u64(current);
    hasher.u64(origin);
}

fn hash_job_clock(clock: JobClock, hasher: &mut StateHasher) {
    hasher.tag(0x85);
    hasher.i32(clock.time);
    hasher.i32(clock.second);
    hasher.i32(clock.day);
    hasher.i32(clock.month);
    hasher.i32(clock.year);
}

fn hash_shell_escape_policy(policy: ShellEscapePolicy, hasher: &mut StateHasher) {
    hasher.tag(0x86);
    hasher.u8(match policy {
        ShellEscapePolicy::Disabled => 0,
        ShellEscapePolicy::Enabled => 1,
        ShellEscapePolicy::Restricted => 2,
    });
}

fn hash_interaction_mode(mode: InteractionMode, hasher: &mut StateHasher) {
    hasher.tag(0x91);
    hasher.u8(match mode {
        InteractionMode::Batch => 0,
        InteractionMode::Nonstop => 1,
        InteractionMode::Scroll => 2,
        InteractionMode::ErrorStop => 3,
    });
}

fn hash_print_sink(sink: PrintSink, hasher: &mut StateHasher) {
    match sink {
        PrintSink::Terminal => hasher.tag(0),
        PrintSink::Log => hasher.tag(1),
        PrintSink::TerminalAndLog => hasher.tag(2),
        PrintSink::Stream(slot) => {
            hasher.tag(3);
            hash_stream_slot(slot, hasher);
        }
    }
}

fn hash_stream_slot(slot: StreamSlot, hasher: &mut StateHasher) {
    hasher.u8(slot.raw());
}

fn hash_shell_escape_record(record: &ShellEscapeRecord, hasher: &mut StateHasher) {
    hasher.str(record.command());
    hasher.bool(record.allowed());
}

fn hash_path(path: &std::path::Path, hasher: &mut StateHasher) {
    hasher.bytes(path.as_os_str().as_encoded_bytes());
}

fn hash_input_summary_fields(
    stores: &Stores,
    world: &World,
    summary: &InputSummary,
    hasher: &mut StateHasher,
) {
    hasher.bool(summary.unicode_superscript_notation());
    hasher.bool(summary.utf8_input_as_bytes());
    hasher.usize(summary.frames().len());
    let mut root_source_seen = false;
    for frame in summary.frames() {
        match frame {
            InputFrameSummary::Source {
                source_id: _,
                input_record,
                source,
            } => {
                hasher.tag(0);
                let is_root = !root_source_seen;
                root_source_seen = true;
                if is_root {
                    // The editor root revision and its absolute physical
                    // coordinates are mapping metadata.  Hash only the live
                    // normalized-line state relative to that line's start so
                    // equal suffix state can converge after a byte-length edit.
                    hasher.bool(false);
                    let base = source.buffer_offset();
                    hasher.usize(source.next_source_offset().saturating_sub(base));
                    hasher.usize(source.line_number());
                    hasher.usize(source.column());
                    hash_lexer_state(source.lexer_state(), hasher);
                    hasher.str(source.normalized_line());
                    hasher.bool(source.bytes_as_chars());
                    hasher.bool(source.byte_projection());
                    hasher.usize(source.line_char_offset());
                    hasher.usize(source.line_byte_offset());
                    hasher.usize(source.physical_content_end().saturating_sub(base));
                    hasher.usize(source.terminator_start().saturating_sub(base));
                    hasher.usize(source.terminator_end().saturating_sub(base));
                    hasher.usize(source.normalized_end_anchor().saturating_sub(base));
                    match source.synthetic_endline_start() {
                        Some(offset) => {
                            hasher.bool(true);
                            hasher.usize(offset);
                        }
                        None => hasher.bool(false),
                    }
                    hasher.usize(source.pending().len());
                    for token in source.pending() {
                        hash_traced_token_semantic(stores, *token, hasher);
                    }
                    hasher.bool(source.end_after_current_line());
                    continue;
                }
                hash_input_record(world, *input_record, hasher);
                hasher.usize(source.buffer_offset());
                hasher.usize(source.next_source_offset());
                hasher.usize(source.line_number());
                hasher.usize(source.column());
                hash_lexer_state(source.lexer_state(), hasher);
                hasher.str(source.normalized_line());
                hasher.bool(source.bytes_as_chars());
                hasher.bool(source.byte_projection());
                hasher.usize(source.line_char_offset());
                hasher.usize(source.line_byte_offset());
                hasher.usize(source.physical_content_end());
                hasher.usize(source.terminator_start());
                hasher.usize(source.terminator_end());
                hasher.usize(source.normalized_end_anchor());
                match source.synthetic_endline_start() {
                    Some(offset) => {
                        hasher.bool(true);
                        hasher.usize(offset);
                    }
                    None => hasher.bool(false),
                }
                hasher.usize(source.pending().len());
                for token in source.pending() {
                    hash_traced_token_semantic(stores, *token, hasher);
                }
                hasher.bool(source.end_after_current_line());
            }
            InputFrameSummary::TokenList {
                token_list,
                origin_list: _,
                replay_kind,
                index,
                macro_arguments,
                macro_invocation: _,
                parent_macro_invocation: _,
                ..
            } => {
                hasher.tag(1);
                stores.hash_token_list_semantic(token_list.id(), hasher);
                hash_token_list_replay_kind(*replay_kind, hasher);
                hasher.usize(*index);
                for slot in 1..=crate::input::MACRO_ARGUMENT_SLOTS as u8 {
                    match macro_arguments.get(slot) {
                        Some(tokens) => {
                            hasher.bool(true);
                            hasher.usize(tokens.len());
                            for &word in tokens {
                                hash_traced_token_semantic(stores, word, hasher);
                            }
                        }
                        None => hasher.bool(false),
                    }
                }
            }
            InputFrameSummary::TransientTokenList {
                tokens,
                replay_kind,
                macro_invocation: _,
                parent_macro_invocation: _,
            } => {
                hasher.tag(2);
                hash_token_list_replay_kind(*replay_kind, hasher);
                hasher.usize(tokens.len());
                for &word in tokens.iter() {
                    hash_traced_token_semantic(stores, word, hasher);
                }
            }
            InputFrameSummary::Condition {
                token: _,
                condition,
            } => {
                hasher.tag(3);
                hash_condition_kind(condition.kind(), hasher);
                hash_condition_limb(condition.limb(), hasher);
                hasher.bool(condition.evaluating());
                hasher.bool(condition.current_limb_taken());
                hasher.bool(condition.any_limb_taken());
                hasher.u32(condition.ifcase_or_count());
                hasher.u32(condition.skip_nesting());
                hasher.bool(condition.inverted());
                hasher.u8(condition.if_type());
            }
        }
    }
    match summary.last_source_frame() {
        Some(source) => {
            hasher.bool(true);
            hash_input_record(world, summary.last_source_record(), hasher);
            hasher.usize(source.buffer_offset());
            hasher.usize(source.next_source_offset());
            hasher.usize(source.line_number());
            hasher.usize(source.column());
            hash_lexer_state(source.lexer_state(), hasher);
            hasher.str(source.normalized_line());
            hasher.usize(source.line_char_offset());
            hasher.usize(source.line_byte_offset());
            hasher.usize(source.physical_content_end());
            hasher.usize(source.terminator_start());
            hasher.usize(source.terminator_end());
            hasher.usize(source.normalized_end_anchor());
            match source.synthetic_endline_start() {
                Some(offset) => {
                    hasher.bool(true);
                    hasher.usize(offset);
                }
                None => hasher.bool(false),
            }
            hasher.usize(source.pending().len());
            for token in source.pending() {
                hash_traced_token_semantic(stores, *token, hasher);
            }
            hasher.bool(source.end_after_current_line());
        }
        None => hasher.bool(false),
    }
}

fn hash_input_summary_fragment(
    stores: &Stores,
    world: &World,
    summary: &InputSummary,
) -> StateHashFragment {
    let visits = summary.frames().len() + usize::from(summary.last_source_frame().is_some());
    StateHashFragment::from_measured_builder(
        INPUT_PROJECTION_DOMAIN,
        StateHashComponent::InputFrames,
        visits,
        |projection| hash_input_summary_fields(stores, world, summary, projection),
    )
}

fn hash_input_record(
    world: &World,
    record: Option<crate::InputRecordId>,
    hasher: &mut StateHasher,
) {
    match record {
        Some(record) => {
            hasher.bool(true);
            let record = world
                .input_record(record)
                .expect("published input summary record must remain live");
            hash_path(record.path(), hasher);
            hasher.bytes(&record.hash().bytes());
            hasher.usize(record.len());
        }
        None => hasher.bool(false),
    }
}

fn hash_traced_token_semantic(stores: &Stores, token: TracedTokenWord, hasher: &mut StateHasher) {
    let token = token
        .token()
        .expect("input-summary pending tokens must be valid traced tokens");
    hash_token(stores, token, hasher);
}

fn hash_token(stores: &Stores, token: Token, hasher: &mut StateHasher) {
    match token {
        Token::Char { ch, cat } => {
            hasher.tag(0);
            hasher.u32(ch as u32);
            hasher.u8(cat as u8);
        }
        Token::Cs(symbol) => {
            let symbol = stores.resolve_stored_symbol(symbol);
            hasher.tag(1);
            hasher.u8(match stores.control_sequence_kind(symbol) {
                ControlSequenceKind::Null
                | ControlSequenceKind::SingleCharacter
                | ControlSequenceKind::Named => 0,
                ControlSequenceKind::ActiveCharacter => 1,
                ControlSequenceKind::Internal => 2,
            });
            hasher.str(stores.resolve(symbol));
        }
        Token::Param(slot) => {
            hasher.tag(2);
            hasher.u8(slot);
        }
        Token::Frozen(crate::token::FrozenToken::END_TEMPLATE) => hasher.tag(3),
        Token::Frozen(crate::token::FrozenToken::END_V) => hasher.tag(4),
        Token::Frozen(crate::token::FrozenToken::EXPANDED_TEXT_BOUNDARY) => hasher.tag(6),
        Token::Frozen(crate::token::FrozenToken::RELAX) => hasher.tag(7),
        Token::Frozen(crate::token::FrozenToken::UNDEFINED_CONTROL_SEQUENCE) => hasher.tag(8),
        Token::Frozen(frozen) => {
            hasher.tag(5);
            hasher.u16(
                frozen
                    .primitive_index()
                    .expect("non-sentinel frozen token must identify a primitive"),
            );
        }
    }
}

fn hash_lexer_state(state: LexerState, hasher: &mut StateHasher) {
    hasher.u8(match state {
        LexerState::NewLine => 0,
        LexerState::MidLine => 1,
        LexerState::SkippingBlanks => 2,
    });
}

fn hash_token_list_replay_kind(kind: TokenListReplayKind, hasher: &mut StateHasher) {
    hasher.u8(match kind {
        TokenListReplayKind::MacroBody => 0,
        TokenListReplayKind::MacroArgument => 1,
        TokenListReplayKind::NoExpand => 2,
        TokenListReplayKind::Unexpanded => 10,
        TokenListReplayKind::EveryPar => 3,
        TokenListReplayKind::EveryHBox => 12,
        TokenListReplayKind::EveryVBox => 13,
        TokenListReplayKind::EveryJob => 11,
        TokenListReplayKind::EveryCr => 4,
        TokenListReplayKind::Mark => 5,
        TokenListReplayKind::OutputRoutine => 6,
        TokenListReplayKind::Inserted => 7,
        TokenListReplayKind::AlignmentUTemplate => 8,
        TokenListReplayKind::ScantokensEveryEof => 9,
        TokenListReplayKind::AlignmentVTemplate => 14,
        TokenListReplayKind::BackedUp => 15,
    });
}

fn hash_condition_kind(kind: ConditionKind, hasher: &mut StateHasher) {
    hasher.u8(match kind {
        ConditionKind::If => 0,
        ConditionKind::IfCase => 1,
    });
}

fn hash_condition_limb(limb: ConditionLimb, hasher: &mut StateHasher) {
    hasher.u8(match limb {
        ConditionLimb::If => 0,
        ConditionLimb::Or => 1,
        ConditionLimb::Else => 2,
    });
}

fn map_store_format_error(error: StoreFormatError) -> FormatError {
    match error {
        StoreFormatError::OpenGroups(depth) => FormatError::OpenGroups(depth),
        StoreFormatError::Codec(message) => FormatError::InvalidState(message),
        StoreFormatError::Invalid(message) => FormatError::InvalidState(message.to_owned()),
        StoreFormatError::InvalidFontMetrics { font, source } => {
            FormatError::InvalidState(format!("font {font} metrics: {source}"))
        }
    }
}

fn map_container_error(error: crate::format_container::ContainerError) -> FormatError {
    use crate::format_container::ContainerError;

    match error {
        ContainerError::BadMagic => FormatError::BadMagic,
        ContainerError::UnsupportedVersion(version) => FormatError::UnsupportedVersion(version),
        ContainerError::Truncated => FormatError::Truncated,
        ContainerError::TrailingBytes => FormatError::TrailingBytes,
        ContainerError::Checksum => FormatError::Checksum,
        ContainerError::IncompatibleAbi(found) => FormatError::IncompatibleAbi(found),
        ContainerError::IncompatibleLookupConfiguration(found) => {
            FormatError::IncompatibleLookupConfiguration(found)
        }
        ContainerError::Invalid(message) => {
            FormatError::InvalidState(format!("invalid portable container: {message}"))
        }
    }
}

fn required_format_section<'a>(
    container: &'a crate::format_container::DecodedContainer<'_>,
    kind: u32,
) -> Result<&'a [u8], FormatError> {
    container
        .section(kind)
        .map(|section| section.bytes.as_ref())
        .ok_or_else(|| FormatError::InvalidState(format!("missing format section {kind}")))
}

const fn encode_interaction_mode(mode: InteractionMode) -> u8 {
    match mode {
        InteractionMode::Batch => 0,
        InteractionMode::Nonstop => 1,
        InteractionMode::Scroll => 2,
        InteractionMode::ErrorStop => 3,
    }
}

fn decode_interaction_mode(mode: u8) -> Result<InteractionMode, FormatError> {
    match mode {
        0 => Ok(InteractionMode::Batch),
        1 => Ok(InteractionMode::Nonstop),
        2 => Ok(InteractionMode::Scroll),
        3 => Ok(InteractionMode::ErrorStop),
        _ => Err(FormatError::InvalidInteractionMode(mode)),
    }
}

fn source_line_starts(bytes: &[u8]) -> Arc<[usize]> {
    let mut starts = Vec::with_capacity(bytes.iter().filter(|&&byte| byte == b'\n').count() + 1);
    starts.push(0);
    starts.extend(
        bytes
            .iter()
            .enumerate()
            .filter(|(_, byte)| **byte == b'\n')
            .map(|(index, _)| index + 1),
    );
    starts.into()
}

fn utf8_scalar_len_at(bytes: &[u8], offset: usize) -> Option<usize> {
    let width = match *bytes.get(offset)? {
        0x00..=0x7f => 1,
        0xc2..=0xdf => 2,
        0xe0..=0xef => 3,
        0xf0..=0xf4 => 4,
        _ => return None,
    };
    let end = offset.checked_add(width)?;
    let scalar = std::str::from_utf8(bytes.get(offset..end)?).ok()?;
    (scalar.chars().count() == 1).then_some(width)
}

#[cfg(test)]
mod tests;
