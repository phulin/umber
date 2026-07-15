//! Top-level TeX state timeline.
//!
//! `Universe` is the only public checkpoint/rollback boundary. The older
//! `Stores` aggregate remains as private composition because its facade already
//! enforces handle liveness and couples Env/content/code-table rollback. The
//! public timeline tuple lives here so future World/effect/input state cannot
//! grow a partial rollback API beside the store tuple.

use crate::code_tables::{CodeTableGenerations, DelCode, LcCode, MathCode, SfCode, UcCode};
#[cfg(test)]
use crate::env::Env;
use crate::env::banks::{DimenParam, GlueParam, IntParam, TokParam};
use crate::epoch::Epoch;
use crate::font::{
    CharMetrics, ExtensibleRecipe, FontMetrics, LigKernChar, LigKernCommand, LigKernIter,
    LoadedFont, MissingCharacter,
};
use crate::glue::GlueSpec;
use crate::hyphenation::{ExceptionSpec, PatternSpec};
use crate::ids::{FontId, GlueId, MacroDefinitionId, NodeListId, OriginListId, TokenListId};
use crate::input::{
    ConditionKind, ConditionLimb, InputFrameSummary, InputSemanticRoot, InputSummary, LexerState,
    SourceId, TokenListReplayKind, TracedTokenList,
};
use crate::interner::{ControlSequenceKind, Symbol, SymbolId};
use crate::macro_store::{MacroDefinitionProvenance, MacroMeaning};
use crate::math::MathFontSize;
use crate::meaning::Meaning;
use crate::node::Node;
use crate::node_arena::{NodeList, NodeListBuilder};
use crate::page::{
    PageBreak, PageBuilderState, PageContents, PageDimension, PageFireUp, PageHashCache,
    PageInsertion, PageInteger, PageMark, PageStateHashCursor,
};
use crate::provenance::ProvenanceStats;
use crate::provenance::{
    InsertedOriginKind, OriginListBuilder, OriginRecord, SynthesizedOriginKind, SyntheticOriginKind,
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
use crate::stores::StoreStateHashCursor;
use crate::stores::{
    FontParameterError, GroupKind, GroupMismatch, PrepareMagDiagnostic, ShipoutNodeMark,
    StoreFormatError, StoreSnapshot, Stores,
};
use crate::token::{Catcode, OriginId, Token, TracedTokenWord};
use crate::token_store::TokenListBuilder;
use crate::world::{
    CommittedArtifact, ContentHash, EffectPos, EffectRecord, JobClock, PrintSink,
    ShellEscapePolicy, ShellEscapeRecord, StreamBufState, StreamSlot, World, WorldCommitMode,
    WorldError, WorldSnapshot, WorldStateHashCursor, install_job_clock_params,
};
use std::collections::HashMap;
#[cfg(any(test, feature = "testing", feature = "shadow"))]
use std::hash::{Hash, Hasher};
use std::sync::Arc;

/// State operations available to TeX's lexer and expansion engine.
///
/// This is intentionally narrower than `Universe`: it permits immutable state
/// reads plus the content/interner mutations that the mouth and gullet are
/// semantically allowed to perform, but it does not expose Env, register, box,
/// code-table, font-parameter, grouping, snapshot, input-file reads, or World
/// mutation APIs.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MeaningCacheGuard {
    owner_address: usize,
    owner_nonce: u64,
    generation: u64,
}

impl MeaningCacheGuard {
    pub(crate) const fn new(owner_address: usize, owner_nonce: u64, generation: u64) -> Self {
        Self {
            owner_address,
            owner_nonce,
            generation,
        }
    }
}

pub trait ExpansionState {
    #[doc(hidden)]
    fn frozen_end_template_token(&self) -> Token {
        Token::frozen_end_template()
    }
    #[doc(hidden)]
    fn frozen_endv_token(&self) -> Token {
        Token::frozen_endv()
    }
    /// Current execution-group depth used by TeX82 alignment `get_next`.
    fn execution_group_depth(&self) -> u32 {
        0
    }
    fn current_group_kind(&self) -> Option<GroupKind> {
        None
    }
    fn interaction_mode_value(&self) -> i32 {
        3
    }
    fn catcode(&self, ch: char) -> Catcode;
    fn lccode(&self, ch: char) -> LcCode;
    fn uccode(&self, ch: char) -> UcCode;
    fn sfcode(&self, ch: char) -> SfCode;
    fn mathcode(&self, ch: char) -> MathCode;
    fn delcode(&self, ch: char) -> DelCode;
    /// Monotonic guard for derived caches of control-sequence meanings.
    /// Implementations without a mutation-aware guard disable such caches.
    fn meaning_cache_guard(&self) -> Option<MeaningCacheGuard> {
        None
    }
    fn meaning(&self, symbol: Symbol) -> Meaning;
    fn macro_definition(&self, id: MacroDefinitionId) -> MacroMeaning;
    fn macro_definition_provenance(&self, id: MacroDefinitionId) -> MacroDefinitionProvenance;
    fn macro_meaning(&self, symbol: Symbol) -> Option<MacroMeaning>;
    fn intern_relaxed_control_sequence(&mut self, name: &str) -> Symbol;
    fn intern(&mut self, name: &str) -> Symbol;
    fn intern_active_character(&mut self, ch: char) -> Symbol;
    fn symbol(&self, name: &str) -> Option<Symbol>;
    fn active_character_symbol(&self, ch: char) -> Option<Symbol>;
    fn resolve(&self, symbol: Symbol) -> &str;
    fn control_sequence_kind(&self, symbol: Symbol) -> ControlSequenceKind;
    fn token_list_builder(&self) -> TokenListBuilder;
    fn intern_token_list(&mut self, tokens: &[Token]) -> TokenListId;
    fn finish_token_list(&mut self, builder: &mut TokenListBuilder) -> TokenListId;
    fn finish_traced_token_list(&mut self, tokens: &[TracedTokenWord]) -> TracedTokenList;
    fn tokens(&self, id: TokenListId) -> &[Token];
    fn intern_glue(&mut self, spec: GlueSpec) -> GlueId;
    fn glue(&self, id: GlueId) -> GlueSpec;
    fn font_name(&self, id: FontId) -> String;
    fn font_identifier_symbol(&self, id: FontId) -> Option<Symbol>;
    fn font_parameter(&self, font: FontId, number: u32) -> Scaled;
    fn font_dimen(&self, font: FontId, number: u32) -> Scaled;
    fn font_parameter_count(&self, font: FontId) -> u32;
    fn font_char_metrics(&self, font: FontId, code: u8) -> Option<crate::font::CharMetrics>;
    fn font_hyphen_char(&self, font: FontId) -> i32;
    fn font_skew_char(&self, font: FontId) -> i32;
    fn current_font(&self) -> FontId;
    fn current_font_symbol(&self) -> Option<Symbol>;
    fn math_family_font(&self, size: MathFontSize, family: u8) -> FontId;
    fn nodes(&self, id: NodeListId) -> NodeList<'_>;
    fn box_dimension(&self, index: u16, dimension: BoxDimension) -> Option<Scaled>;
    fn count(&self, index: u16) -> i32;
    fn dimen(&self, index: u16) -> Scaled;
    fn skip(&self, index: u16) -> GlueId;
    fn muskip(&self, index: u16) -> GlueId;
    fn toks(&self, index: u16) -> TokenListId;
    fn box_reg(&self, index: u16) -> Option<NodeListId>;
    fn page_dimension(&self, dimension: PageDimension) -> Scaled;
    fn page_integer(&self, integer: PageInteger) -> i32;
    fn page_mark(&self, mark: PageMark) -> TokenListId;
    fn page_mark_class(&self, mark: PageMark, class: u16) -> TokenListId;
    fn penalty_array_value(&self, kind: PenaltyArrayKind, index: i32) -> i32;
    fn paragraph_shape_dimension(&self, line: i32, width: bool) -> Scaled;
    fn report_bad_register_code(&mut self, _value: i32, _maximum: u16) {}
    fn report_missing_font_identifier(&mut self) {}
    fn int_param(&self, param: IntParam) -> i32;
    /// Emits the e-TeX `\scantokens` pseudo-file boundary when tracing is enabled.
    fn trace_scantokens_boundary(&mut self, _opening: bool) {}
    fn last_badness(&self) -> i32;
    fn mag(&self) -> i32;
    fn prepared_mag(&self) -> Option<i32>;
    fn prepare_mag(&mut self) -> (i32, Option<PrepareMagDiagnostic>);
    fn endlinechar(&self) -> i32;
    fn dimen_param(&self, param: DimenParam) -> Scaled;
    fn glue_param(&self, param: GlueParam) -> GlueId;
    fn tok_param(&self, param: TokParam) -> TokenListId;
    fn input_stream_eof(&self, stream: StreamSlot) -> bool;
    fn bootstrap_origin(&self) -> OriginId;
    fn synthetic_origin(&mut self, kind: SyntheticOriginKind) -> OriginId;
    fn synthesized_origin(&mut self, kind: SynthesizedOriginKind, parent: OriginId) -> OriginId;
    fn source_origin(
        &mut self,
        source: SourceId,
        byte_offset: u64,
        line: u32,
        column: u32,
    ) -> OriginId;
    fn source_origin_with_input_record(
        &mut self,
        source: SourceId,
        input_record: Option<crate::InputRecordId>,
        byte_offset: u64,
        line: u32,
        column: u32,
    ) -> OriginId;
    fn source_token_origin(
        &mut self,
        source: SourceId,
        byte_offset: u64,
        byte_end: u64,
    ) -> OriginId;
    fn source_range_origin(
        &mut self,
        source: SourceId,
        byte_offset: u64,
        byte_end: u64,
    ) -> OriginId;
    fn source_span_origin(&mut self, span: SourceSpan) -> OriginId;
    fn register_source(
        &mut self,
        source: SourceId,
        descriptor: SourceDescriptor,
    ) -> Result<SourcePos, SourceMapError>;
    fn register_input_source(
        &mut self,
        source: SourceId,
        descriptor: SourceDescriptor,
    ) -> Result<RegisteredSource, SourceMapError>;
    fn macro_invocation_origin(
        &mut self,
        definition: MacroDefinitionId,
        invocation: OriginId,
        definition_origin: OriginId,
        parent_invocation: OriginId,
    ) -> OriginId;
    fn inserted_origin(
        &mut self,
        kind: InsertedOriginKind,
        token: Token,
        parent: OriginId,
    ) -> OriginId;
    fn allocate_repeated_origin_list(&mut self, origin: OriginId, len: usize) -> OriginListId;
    fn origin_list_builder(&self) -> OriginListBuilder;
    fn finish_origin_list(&mut self, builder: &mut OriginListBuilder) -> OriginListId;
    fn origin_list(&self, id: OriginListId) -> &[OriginId];
    fn origin_list_if_live(&self, id: OriginListId) -> Option<&[OriginId]>;
}

/// Input file reads available to a driver-supplied `\input` resolver.
///
/// This is intentionally separate from [`ExpansionState`] so ordinary gullet
/// code cannot open files and input resolvers cannot see expansion, Env/register,
/// code-table, snapshot, font-assignment, or general [`World`] mutation APIs.
pub trait InputReadState {
    fn read_input_file(
        &mut self,
        path: &std::path::Path,
    ) -> Result<crate::FileContent, crate::WorldError>;
}

/// State operations available only to the top-level `\input` dispatch path.
///
/// This is separate from [`ExpansionState`] so helper code that is generic over
/// ordinary expansion authority cannot derive input-file read access.
pub trait InputOpenState {
    type Input<'a>: InputReadState
    where
        Self: 'a;

    fn input_open_context(&mut self) -> Self::Input<'_>;
}

/// Production expansion capability over a [`Universe`].
///
/// Pass this wrapper to lexer/expansion code instead of `&mut Universe` when
/// the caller does not need the full top-level driver surface.
pub struct ExpansionContext<'a> {
    universe: &'a mut Universe,
}

impl<'a> ExpansionContext<'a> {
    #[must_use]
    pub fn new(universe: &'a mut Universe) -> Self {
        Self { universe }
    }

    /// Reads expansion-control provenance attached to a delivered token.
    #[must_use]
    pub fn origin(&self, id: OriginId) -> OriginRecord {
        self.universe.origin(id)
    }
}

/// Production input-open capability over a [`Universe`].
pub struct InputOpenContext<'a> {
    universe: &'a mut Universe,
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
    owner: SnapshotOwner,
    serial: u64,
    store: StoreSnapshot,
    epoch: Epoch,
    world: WorldSnapshot,
    input_summary: InputSummary,
    interaction_mode: InteractionMode,
    page: PageBuilderState,
    state_hash: u64,
    state_hash_base: StateHashBase,
}

/// One immutable accepted-generation state substrate shared by O(1) snapshots.
#[derive(Debug)]
pub struct GenerationSubstrate {
    universe: Universe,
    charged_bytes: usize,
    retained_origin_locations: HashMap<OriginId, crate::ResolvedSourceLocation>,
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
    node_mark: ShipoutNodeMark,
    rollback: Option<ScopedRollback>,
    finished: bool,
}

#[derive(Debug)]
struct ScopedRollback {
    owner: SnapshotOwner,
    store: StoreSnapshot,
    world: WorldSnapshot,
    input_summary: InputSummary,
    interaction_mode: InteractionMode,
    page: PageBuilderState,
    state_hash_base: StateHashBase,
}

/// Opaque allocation mark for one in-progress box-register construction.
///
/// Finishing the assignment promotes its live result into rollback-safe
/// storage, then releases every epoch node allocated during construction.
#[derive(Debug)]
pub struct BoxBuildTransaction<'a> {
    universe: &'a mut Universe,
    node_mark: ShipoutNodeMark,
    finished: bool,
}

impl std::ops::Deref for ShipoutTransaction<'_> {
    type Target = Universe;
    fn deref(&self) -> &Self::Target {
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
            self.universe.stores.release_shipout_nodes(self.node_mark);
        }
    }
}

impl ShipoutTransaction<'_> {
    /// Atomically finishes this transaction's artifact/effect publication.
    pub fn commit(
        mut self,
        artifact: crate::world::VerifiedArtifact,
        effect_pos: EffectPos,
    ) -> Result<ContentHash, WorldError> {
        let hash_base = self.state_hash_base.clone();
        let hash = self.world.store_verified_artifact(&artifact)?;
        if self.world.commit_mode() == WorldCommitMode::Retained {
            let node_mark = self.node_mark;
            self.stores.release_shipout_nodes(node_mark);
            self.state_hash_base = self.retarget_hash_base_after_committed_boundary(hash_base);
            self.page.set_integer(PageInteger::DeadCycles, 0);
            let (bytes, render_origins) = artifact.into_parts();
            self.world
                .record_artifact_commit(hash, bytes, render_origins);
            self.rollback = None;
            self.finished = true;
            return Ok(hash);
        }
        if let Err(err) = self.world.commit_effects(effect_pos) {
            self.state_hash_base = self.retarget_hash_base_after_committed_boundary(hash_base);
            let node_mark = self.node_mark;
            self.stores.release_shipout_nodes(node_mark);
            self.rollback = None;
            self.finished = true;
            return Err(err);
        }
        let node_mark = self.node_mark;
        self.stores.release_shipout_nodes(node_mark);
        self.state_hash_base = self.retarget_hash_base_after_committed_boundary(hash_base);
        self.page.set_integer(PageInteger::DeadCycles, 0);
        let (bytes, render_origins) = artifact.into_parts();
        self.world
            .record_artifact_commit(hash, bytes, render_origins);
        self.rollback = None;
        self.finished = true;
        Ok(hash)
    }
}

impl BoxBuildTransaction<'_> {
    /// Promotes the result into the register store and commits the owned suffix.
    pub fn finish(mut self, index: u16, value: Option<NodeListId>, global: bool) {
        match (global, value) {
            (false, Some(value)) => self.stores.set_box_reg(index, value),
            (true, Some(value)) => self.stores.set_box_reg_global(index, value),
            (false, None) => self.stores.clear_box_reg(index),
            (true, None) => self.stores.clear_box_reg_global(index),
        }
        let node_mark = self.node_mark;
        self.stores.release_shipout_nodes(node_mark);
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

    /// Returns the semantic convergence hash captured by this snapshot.
    ///
    /// The hash is a fold of semantic slice hashes over the checkpoint
    /// timeline (`combine(previous_checkpoint_hash, slice_hash)`), so it is
    /// checkpoint-schedule-relative: it witnesses "same semantic history
    /// observed at the same checkpoint boundaries", not a canonical
    /// fingerprint of the reached state. Compare hashes only between runs
    /// that take checkpoints at the same positions under the same policy;
    /// see `docs/core_state.md` §9 (convergence detection).
    #[must_use]
    pub const fn state_hash(&self) -> u64 {
        self.state_hash
    }
}

impl GenerationSubstrate {
    /// Freezes one completed mutable timeline as an accepted generation.
    #[must_use]
    pub fn new(universe: Universe) -> Self {
        let retained_origin_locations = HashMap::new();
        let charged_bytes = generation_charged_bytes(&universe, &retained_origin_locations);
        Self {
            universe,
            charged_bytes,
            retained_origin_locations,
        }
    }

    /// Opaque charged bytes shared by every checkpoint on this substrate.
    #[must_use]
    pub const fn charged_bytes(&self) -> usize {
        self.charged_bytes
    }

    #[must_use]
    pub const fn world(&self) -> &World {
        self.universe.world()
    }

    /// Resolves one diagnostic origin retained by this accepted generation.
    #[must_use]
    pub fn resolve_origin_with_generated_path(
        &self,
        origin: crate::token::OriginId,
        generated_path: &str,
    ) -> Option<crate::ResolvedSourceLocation> {
        crate::ProvenanceResolver::new(&self.universe)
            .resolve_origin_with_generated_path(origin, generated_path)
            .or_else(|| self.retained_origin_locations.get(&origin).cloned())
    }

    /// Resolves one retained origin against the session's current editor layout.
    #[must_use]
    pub fn resolve_layout_origin(
        &self,
        origin: crate::token::OriginId,
        fragments: &crate::FragmentStore,
        layout: &crate::EditorLayout,
    ) -> crate::LayoutResolvedOrigin {
        let resolved = crate::ProvenanceResolver::new(&self.universe)
            .resolve_layout_origin(origin, fragments, layout);
        if resolved == crate::LayoutResolvedOrigin::Unknown
            && self.retained_origin_locations.contains_key(&origin)
        {
            crate::LayoutResolvedOrigin::Foreign
        } else {
            resolved
        }
    }

    /// Retains only the diagnostic origin graph needed by artifacts adopted
    /// from a related scratch fork. Semantic state and source stores remain on
    /// the accepted generation.
    pub fn retain_artifact_origins_from_fork(
        &mut self,
        fork: &Universe,
        roots: &[OriginId],
        generated_path: &str,
    ) -> Result<(), GenerationForkError> {
        let origin = fork.fork_origin.ok_or(GenerationForkError::UnrelatedFork)?;
        if origin.source_owner != self.universe.owner.snapshot_owner() {
            return Err(GenerationForkError::UnrelatedFork);
        }
        let resolver = crate::ProvenanceResolver::new(fork);
        for &root in roots {
            if let Some(location) =
                resolver.resolve_origin_with_generated_path(root, generated_path)
            {
                self.retained_origin_locations
                    .entry(root)
                    .or_insert(location);
            }
        }
        self.universe
            .stores
            .retain_diagnostic_origins_from(&fork.stores, roots);
        self.charged_bytes =
            generation_charged_bytes(&self.universe, &self.retained_origin_locations);
        Ok(())
    }

    #[doc(hidden)]
    pub fn validate_checkpoint_snapshot(
        &self,
        checkpoint: &Snapshot,
    ) -> Result<(), GenerationForkError> {
        self.universe.validate_retained_snapshot(checkpoint)
    }

    #[must_use]
    pub fn root_content_hash(&self, summary: &InputSummary) -> Option<ContentHash> {
        self.universe.root_editor_content_hash(summary)
    }

    /// Clones this frozen generation once and atomically rolls the clone back
    /// to an exact owner-validated checkpoint.
    pub fn fork_at(&self, checkpoint: &Snapshot) -> Result<Universe, GenerationForkError> {
        self.universe.validate_retained_snapshot(checkpoint)?;
        let mut fork = self.universe.clone();
        let checkpoint = fork.retarget_inherited_snapshot(checkpoint);
        fork.rollback_generation_fork(&checkpoint);
        fork.fork_origin = Some(ForkOrigin {
            source_owner: self.universe.owner.snapshot_owner(),
            anchor_serial: checkpoint.serial,
        });
        Ok(fork)
    }

    /// Retargets a source-generation prefix snapshot onto a promoted fork.
    /// This is deliberately limited to records at or before the exact fork anchor.
    pub fn retarget_prefix_from(
        &self,
        source: &GenerationSubstrate,
        checkpoint: &Snapshot,
    ) -> Result<Snapshot, GenerationForkError> {
        source.universe.validate_retained_snapshot(checkpoint)?;
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
    ) -> Result<World, WorldError> {
        let mut universe = self.universe;
        universe
            .world
            .replace_retained_outputs(effects, artifacts)?;
        universe.export_retained_effects()?;
        Ok(universe.world)
    }

    /// Materializes detached session output without consuming the retained
    /// generation used by later incremental revisions.
    pub fn materialize_detached_outputs(
        &self,
        effects: Vec<EffectRecord>,
        artifacts: Vec<CommittedArtifact>,
    ) -> Result<World, WorldError> {
        let mut world = self.universe.world.clone();
        world.replace_retained_outputs(effects, artifacts)?;
        world.export_retained_effects()?;
        Ok(world)
    }
}

fn generation_charged_bytes(
    universe: &Universe,
    retained_origin_locations: &HashMap<OriginId, crate::ResolvedSourceLocation>,
) -> usize {
    universe
        .stores
        .generation_retained_bytes()
        .saturating_add(std::mem::size_of::<Universe>())
        .saturating_add(universe.input_summary.retained_bytes())
        .saturating_add(universe.page.retained_bytes())
        .saturating_add(universe.world.generation_retained_bytes())
        .saturating_add(
            retained_origin_locations
                .capacity()
                .saturating_mul(std::mem::size_of::<(OriginId, crate::ResolvedSourceLocation)>()),
        )
        .saturating_add(
            retained_origin_locations
                .values()
                .map(|location| location.path.capacity())
                .sum::<usize>(),
        )
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
    BadMagic,
    UnsupportedVersion(u32),
    Truncated,
    TrailingBytes,
    Checksum,
    InvalidInteractionMode(u8),
    InvalidState(String),
}

impl std::fmt::Display for FormatError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::OpenGroups(depth) => write!(f, "cannot dump a format with {depth} open groups"),
            Self::NonEmptyInput => f.write_str("cannot dump a format with live input state"),
            Self::NonEmptyPage => f.write_str("cannot dump a format with page-builder material"),
            Self::BadMagic => f.write_str("not an Umber format file"),
            Self::UnsupportedVersion(version) => {
                write!(f, "unsupported Umber format version {version}")
            }
            Self::Truncated => f.write_str("truncated Umber format file"),
            Self::TrailingBytes => f.write_str("trailing bytes in Umber format file"),
            Self::Checksum => f.write_str("Umber format checksum mismatch"),
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
    checkpoint_hash: u64,
}

const UNIVERSE_SLICE_DOMAIN: u64 = 0x756e_6976_6572_7365;
const WORLD_SLICE_DOMAIN: u64 = 0x776f_726c_645f_736c;
const WORLD_EFFECTS_DOMAIN: u64 = 0x776f_726c_645f_6566;
const WORLD_SHELL_ESCAPES_DOMAIN: u64 = 0x776f_726c_645f_7368;
const WORLD_SCALARS_DOMAIN: u64 = 0x776f_726c_645f_7363;
const WORLD_STREAMS_DOMAIN: u64 = 0x776f_726c_645f_6275;
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

impl StateHashProjectionCache {
    fn clear(&mut self) {
        self.world_streams = None;
        self.input = None;
        self.page.clear();
        #[cfg(test)]
        {
            self.input_hash_calls = 0;
        }
    }
}

/// One owned TeX state timeline.
#[derive(Debug)]
pub struct Universe {
    owner: UniverseOwner,
    stores: Stores,
    world: World,
    interaction_mode: InteractionMode,
    input_summary: InputSummary,
    /// Operational editor revision identity; excluded from snapshots and semantic hashes.
    editor_content_hash: Option<ContentHash>,
    page: PageBuilderState,
    state_hash_base: StateHashBase,
    state_hash_projection_cache: StateHashProjectionCache,
    next_snapshot_serial: u64,
    fork_origin: Option<ForkOrigin>,
}

/// Canonical semantic hasher for executor-owned state at a named boundary.
///
/// Construction stays under [`Universe`] so handle-bearing mode state is
/// resolved through the owning stores rather than hashing runtime ids.
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

    pub fn node_list(&mut self, id: NodeListId) {
        self.stores.hash_node_list_semantic(id, &mut self.hasher);
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
        let stores = self.stores.clone();
        let state_hash_base = StateHashBase {
            store: stores.retarget_state_hash_cursor(&self.state_hash_base.store),
            world: self.state_hash_base.world.clone(),
            input_summary: self.state_hash_base.input_summary.clone(),
            input_fragment: self.state_hash_base.input_fragment,
            interaction_mode: self.state_hash_base.interaction_mode,
            page: self.state_hash_base.page.clone(),
            checkpoint_hash: self.state_hash_base.checkpoint_hash,
        };
        Self {
            owner: UniverseOwner::new(),
            stores,
            world: self.world.clone(),
            interaction_mode: self.interaction_mode,
            input_summary: self.input_summary.clone(),
            editor_content_hash: self.editor_content_hash,
            page: self.page.clone(),
            state_hash_base,
            state_hash_projection_cache: self.state_hash_projection_cache.clone(),
            next_snapshot_serial: self.next_snapshot_serial,
            fork_origin: self.fork_origin,
        }
    }
}

impl Default for Universe {
    fn default() -> Self {
        Self::new()
    }
}

impl Universe {
    const FORMAT_MAGIC: [u8; 8] = *b"UMBRFMT\0";
    pub const FORMAT_SCHEMA_VERSION: u32 = 7;

    /// Creates an isolated TeX state timeline.
    #[must_use]
    pub fn new() -> Self {
        Self::with_world(World::default())
    }

    /// Creates an isolated TeX timeline backed by an explicit effect world.
    #[must_use]
    pub fn with_world(world: World) -> Self {
        let mut stores = Stores::new();
        let clock = world.job_clock();
        install_job_clock_params(
            &mut |param, value| stores.set_int_param(param, value),
            clock,
        );
        let input_summary = InputSummary::default();
        let page = PageBuilderState::default();
        let input_fragment = hash_input_summary_fragment(&stores, &world, &input_summary);
        let state_hash_base = StateHashBase {
            store: stores.state_hash_cursor(),
            world: world.state_hash_cursor(),
            input_summary: input_summary.semantic_root(),
            input_fragment,
            interaction_mode: InteractionMode::default(),
            page: page.state_hash_cursor(),
            checkpoint_hash: INITIAL_STATE_HASH,
        };
        Self {
            owner: UniverseOwner::new(),
            stores,
            world,
            interaction_mode: InteractionMode::default(),
            input_summary,
            editor_content_hash: None,
            page,
            state_hash_base,
            state_hash_projection_cache: StateHashProjectionCache::default(),
            next_snapshot_serial: 0,
            fork_origin: None,
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
            hasher: StateHasher::new(domain),
            visits: 0,
        };
        #[cfg(feature = "profiling-stats")]
        let started = std::time::Instant::now();
        build(&mut projection);
        let fingerprint = projection.hasher.finish();
        #[cfg(feature = "profiling-stats")]
        crate::measurement::record_state_hash_component(
            StateHashComponent::Mode,
            projection.visits,
            started.elapsed(),
        );
        fingerprint
    }

    /// Serializes the allocation-independent semantic engine state.
    ///
    /// Host effects, provenance, checkpoints, journals, caches, and input
    /// cursors are intentionally absent. The image is deterministic for one
    /// semantic state and carries an explicit schema version and checksum.
    pub fn dump_format(&self) -> Result<Vec<u8>, FormatError> {
        if !self.input_summary.is_empty() {
            return Err(FormatError::NonEmptyInput);
        }
        // e-TeX deliberately does not dump its saved vertical-discard lists.
        if !self.page.is_format_empty() {
            return Err(FormatError::NonEmptyPage);
        }
        let payload = self
            .stores
            .encode_format()
            .map_err(map_store_format_error)?;
        let mode = encode_interaction_mode(self.interaction_mode);
        let checksum = format_checksum(mode, &payload);
        let mut bytes = Vec::with_capacity(29 + payload.len());
        bytes.extend_from_slice(&Self::FORMAT_MAGIC);
        bytes.extend_from_slice(&Self::FORMAT_SCHEMA_VERSION.to_le_bytes());
        bytes.push(mode);
        bytes.extend_from_slice(&(payload.len() as u64).to_le_bytes());
        bytes.extend_from_slice(&checksum.to_le_bytes());
        bytes.extend_from_slice(&payload);
        Ok(bytes)
    }

    /// Constructs a fresh timeline from a validated semantic format image.
    pub fn from_format(world: World, bytes: &[u8]) -> Result<Self, FormatError> {
        const HEADER: usize = 29;
        if bytes.len() < HEADER {
            return Err(FormatError::Truncated);
        }
        if bytes[..8] != Self::FORMAT_MAGIC {
            return Err(FormatError::BadMagic);
        }
        let version = u32::from_le_bytes(bytes[8..12].try_into().expect("fixed header slice"));
        if version != Self::FORMAT_SCHEMA_VERSION {
            return Err(FormatError::UnsupportedVersion(version));
        }
        let mode_byte = bytes[12];
        let mode = decode_interaction_mode(mode_byte)?;
        let length = u64::from_le_bytes(bytes[13..21].try_into().expect("fixed header slice"));
        let length = usize::try_from(length).map_err(|_| FormatError::Truncated)?;
        let expected = HEADER.checked_add(length).ok_or(FormatError::Truncated)?;
        if bytes.len() < expected {
            return Err(FormatError::Truncated);
        }
        if bytes.len() > expected {
            return Err(FormatError::TrailingBytes);
        }
        let checksum = u64::from_le_bytes(bytes[21..29].try_into().expect("fixed header slice"));
        let payload = &bytes[HEADER..];
        if checksum != format_checksum(mode_byte, payload) {
            return Err(FormatError::Checksum);
        }
        let stores = Stores::decode_format(payload).map_err(map_store_format_error)?;
        let input_summary = InputSummary::default();
        let page = PageBuilderState::default();
        let input_fragment = hash_input_summary_fragment(&stores, &world, &input_summary);
        let state_hash_base = StateHashBase {
            store: stores.state_hash_cursor(),
            world: world.state_hash_cursor(),
            input_summary: input_summary.semantic_root(),
            input_fragment,
            interaction_mode: mode,
            page: page.state_hash_cursor(),
            checkpoint_hash: checksum,
        };
        Ok(Self {
            owner: UniverseOwner::new(),
            stores,
            world,
            interaction_mode: mode,
            input_summary,
            editor_content_hash: None,
            page,
            state_hash_base,
            state_hash_projection_cache: StateHashProjectionCache::default(),
            next_snapshot_serial: 0,
            fork_origin: None,
        })
    }

    /// Takes an O(1) snapshot of the whole timeline tuple.
    #[must_use]
    pub fn snapshot(&mut self) -> Snapshot {
        self.checkpoint_from_hash_base(self.state_hash_base.clone())
    }

    #[must_use]
    pub fn freeze_generation(self) -> GenerationSubstrate {
        GenerationSubstrate::new(self)
    }

    fn validate_retained_snapshot(&self, snapshot: &Snapshot) -> Result<(), GenerationForkError> {
        if snapshot.owner != self.owner.snapshot_owner() {
            return Err(GenerationForkError::ForeignSnapshot);
        }
        if !self.stores.can_restore_snapshot(&snapshot.store)
            || !self.world.snapshot_is_retained(&snapshot.world)
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
            state_hash_base: self.state_hash_base.clone(),
        }
    }

    fn rollback_scoped(&mut self, rollback: ScopedRollback) {
        assert_eq!(
            rollback.owner,
            self.owner.snapshot_owner(),
            "scoped rollback belongs to a different Universe instance"
        );
        self.world.assert_snapshot_retained(&rollback.world);
        self.stores.rollback(&rollback.store);
        self.world.rollback(&rollback.world);
        self.input_summary = rollback.input_summary;
        self.interaction_mode = rollback.interaction_mode;
        self.page = rollback.page;
        self.state_hash_base = rollback.state_hash_base;
        self.state_hash_projection_cache.clear();
    }

    fn checkpoint_from_hash_base(&mut self, hash_base: StateHashBase) -> Snapshot {
        let world = self.world.snapshot();
        let store = self.stores.checkpoint();
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
        let state_hash = if hash_base.store == store_cursor
            && hash_base.world == world_cursor
            && hash_base.input_fragment == input_fragment
            && hash_base.interaction_mode == self.interaction_mode
            && hash_base.page == page_cursor
        {
            hash_base.checkpoint_hash
        } else {
            let slice_hash = self.state_hash_slice(&hash_base, &store, input_fragment);
            combine(hash_base.checkpoint_hash, slice_hash)
        };
        let next_hash_base = StateHashBase {
            store: store_cursor,
            world: world_cursor,
            input_summary: input_cursor,
            input_fragment,
            interaction_mode: self.interaction_mode,
            page: page_cursor,
            checkpoint_hash: state_hash,
        };
        self.state_hash_base = next_hash_base.clone();
        let serial = self.next_snapshot_serial;
        self.next_snapshot_serial = self
            .next_snapshot_serial
            .checked_add(1)
            .expect("Universe snapshot serial exhausted");
        Snapshot {
            owner: self.owner.snapshot_owner(),
            serial,
            epoch: store.epoch(),
            store,
            world,
            input_summary: self.input_summary.clone(),
            interaction_mode: self.interaction_mode,
            page: self.page.clone(),
            state_hash,
            state_hash_base: next_hash_base,
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
        self.stores.rollback(&snapshot.store);
        self.world.rollback(&snapshot.world);
        self.input_summary = snapshot.input_summary.clone();
        self.interaction_mode = snapshot.interaction_mode;
        self.page = snapshot.page.clone();
        self.state_hash_base = snapshot.state_hash_base.clone();
        self.state_hash_projection_cache.clear();
    }

    fn rollback_generation_fork(&mut self, snapshot: &Snapshot) {
        self.assert_valid_snapshot(snapshot);
        self.world.assert_snapshot_retained(&snapshot.world);
        self.stores.rollback(&snapshot.store);
        self.world.rollback_generation_fork(&snapshot.world);
        self.input_summary = snapshot.input_summary.clone();
        self.interaction_mode = snapshot.interaction_mode;
        self.page = snapshot.page.clone();
        self.state_hash_base = snapshot.state_hash_base.clone();
        self.state_hash_projection_cache.clear();
    }

    fn state_hash_slice(
        &mut self,
        hash_base: &StateHashBase,
        store: &StoreSnapshot,
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
        self.state_hash_projection_cache = cache;

        let mut hasher = StateHasher::new(UNIVERSE_SLICE_DOMAIN);
        hasher.u32(crate::CHECKPOINT_STATE_HASH_SCHEMA_VERSION);
        hasher.u64(store);
        world.apply(&mut hasher);
        input.apply(&mut hasher);
        interaction.apply(&mut hasher);
        page.apply(&mut hasher);
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
            3,
            |projection| {
                hash_rng_state(self.world.rng_state(), projection);
                hash_job_clock(self.world.job_clock(), projection);
                hash_shell_escape_policy(self.world.shell_escape_policy(), projection);
            },
        );
        StateHashFragment::from_builder(WORLD_SLICE_DOMAIN, |projection| {
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
            EffectRecord::DeferredWrite { stream, tokens } => {
                hasher.tag(3);
                hash_stream_slot(*stream, hasher);
                self.stores.hash_token_list_semantic(*tokens, hasher);
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
        StateHashFragment::from_builder(0x7061_6765_5f62_6e64, |projection| {
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

    /// Mutates the external-effect capability object through the Universe boundary.
    pub fn world_mut(&mut self) -> &mut World {
        &mut self.world
    }

    /// Records an unexpanded deferred-write payload after validating that its
    /// token list belongs to this live timeline.
    pub fn record_deferred_write(&mut self, stream: StreamSlot, tokens: TokenListId) {
        self.stores.assert_live_token_list(tokens);
        self.world.record_deferred_write(stream, tokens);
    }

    /// Marks the start of node allocations owned by one in-progress shipout.
    #[must_use]
    pub fn begin_shipout(&mut self) -> ShipoutTransaction<'_> {
        let rollback = self.capture_scoped_rollback();
        let node_mark = self.stores.shipout_node_mark();
        ShipoutTransaction {
            universe: self,
            node_mark,
            rollback: Some(rollback),
            finished: false,
        }
    }

    /// Commits an effect prefix and retargets semantic hash cursors after it is dropped.
    pub fn commit_effects(&mut self, effect_pos: EffectPos) -> Result<(), WorldError> {
        if self.world.commit_mode() == WorldCommitMode::Retained {
            return Ok(());
        }
        let hash_base = self.state_hash_base.clone();
        if let Err(err) = self.world.commit_effects(effect_pos) {
            self.state_hash_base = self.retarget_hash_base_after_committed_boundary(hash_base);
            return Err(err);
        }
        self.state_hash_base = self.retarget_hash_base_after_committed_boundary(hash_base);
        Ok(())
    }

    /// Opens a rollback-capable editor session with deferred host materialization.
    pub fn begin_retained_session(&mut self) -> Result<(), WorldError> {
        self.world.begin_retained_session()
    }

    /// Consumes the retained effect branch by exposing it exactly once in order.
    pub fn export_retained_effects(&mut self) -> Result<(), WorldError> {
        let hash_base = self.state_hash_base.clone();
        self.world.export_retained_effects()?;
        self.state_hash_base = self.retarget_hash_base_after_committed_boundary(hash_base);
        Ok(())
    }

    /// Bytes required by detached artifacts and the virtual effect suffix.
    #[must_use]
    pub fn retained_output_bytes(&self) -> usize {
        self.world.retained_output_bytes()
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
        self.stores
            .install_source_fragments(fragments.metadata_snapshot());
        Ok(())
    }

    /// Sets operational editor revision identity outside semantic state.
    pub fn set_root_editor_content_hash(&mut self, hash: ContentHash) {
        self.editor_content_hash = Some(hash);
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
    }

    /// Returns the lexer-owned input stack state restored by the last rollback.
    #[must_use]
    pub const fn input_summary(&self) -> &InputSummary {
        &self.input_summary
    }

    /// Returns the current interaction mode.
    #[must_use]
    pub const fn interaction_mode(&self) -> InteractionMode {
        self.interaction_mode
    }

    /// Sets the current interaction mode.
    pub fn set_interaction_mode(&mut self, mode: InteractionMode) {
        self.interaction_mode = mode;
    }

    /// Returns the current code-table generation vector.
    #[must_use]
    pub fn code_table_generations(&self) -> CodeTableGenerations {
        self.stores.code_table_generations()
    }

    #[must_use]
    pub fn catcode(&self, ch: char) -> Catcode {
        self.stores.catcode(ch)
    }

    pub fn set_catcode(&mut self, ch: char, value: Catcode) {
        self.stores.set_catcode(ch, value);
    }

    pub fn set_catcode_global(&mut self, ch: char, value: Catcode) {
        self.stores.set_catcode_global(ch, value);
    }

    #[must_use]
    pub fn lccode(&self, ch: char) -> LcCode {
        self.stores.lccode(ch)
    }

    pub fn set_lccode(&mut self, ch: char, value: LcCode) {
        self.stores.set_lccode(ch, value);
    }

    pub fn set_lccode_global(&mut self, ch: char, value: LcCode) {
        self.stores.set_lccode_global(ch, value);
    }

    #[must_use]
    pub fn uccode(&self, ch: char) -> UcCode {
        self.stores.uccode(ch)
    }

    pub fn set_uccode(&mut self, ch: char, value: UcCode) {
        self.stores.set_uccode(ch, value);
    }

    pub fn set_uccode_global(&mut self, ch: char, value: UcCode) {
        self.stores.set_uccode_global(ch, value);
    }

    #[must_use]
    pub fn sfcode(&self, ch: char) -> SfCode {
        self.stores.sfcode(ch)
    }

    pub fn set_sfcode(&mut self, ch: char, value: SfCode) {
        self.stores.set_sfcode(ch, value);
    }

    pub fn set_sfcode_global(&mut self, ch: char, value: SfCode) {
        self.stores.set_sfcode_global(ch, value);
    }

    #[must_use]
    pub fn mathcode(&self, ch: char) -> MathCode {
        self.stores.mathcode(ch)
    }

    pub fn set_mathcode(&mut self, ch: char, value: MathCode) {
        self.stores.set_mathcode(ch, value);
    }

    pub fn set_mathcode_global(&mut self, ch: char, value: MathCode) {
        self.stores.set_mathcode_global(ch, value);
    }

    #[must_use]
    pub fn delcode(&self, ch: char) -> DelCode {
        self.stores.delcode(ch)
    }

    pub fn set_delcode(&mut self, ch: char, value: DelCode) {
        self.stores.set_delcode(ch, value);
    }

    pub fn set_delcode_global(&mut self, ch: char, value: DelCode) {
        self.stores.set_delcode_global(ch, value);
    }

    pub fn add_hyphenation_pattern(&mut self, pattern: PatternSpec) {
        self.stores.add_hyphenation_pattern(pattern);
    }

    pub fn add_hyphenation_pattern_for_language(&mut self, language: u8, pattern: PatternSpec) {
        self.stores
            .add_hyphenation_pattern_for_language(language, pattern);
    }

    pub fn add_hyphenation_exception(&mut self, exception: ExceptionSpec) {
        self.stores.add_hyphenation_exception(exception);
    }

    pub fn add_hyphenation_exception_for_language(
        &mut self,
        language: u8,
        exception: ExceptionSpec,
    ) {
        self.stores
            .add_hyphenation_exception_for_language(language, exception);
    }

    pub fn save_hyphenation_codes(
        &mut self,
        language: u8,
        codes: impl IntoIterator<Item = (char, char)>,
    ) {
        self.stores.save_hyphenation_codes(language, codes);
    }

    #[must_use]
    pub fn saved_hyphenation_code(&self, language: u8, ch: char) -> Option<Option<char>> {
        self.stores.saved_hyphenation_code(language, ch)
    }

    #[must_use]
    pub fn hyphen_positions(&self, word: &str, left_min: usize, right_min: usize) -> Vec<usize> {
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
        self.stores
            .hyphen_positions_for_language(language, word, left_min, right_min)
    }

    #[must_use]
    pub fn hyphenation_exception(&self, word: &str) -> Option<&[usize]> {
        self.stores.hyphenation_exception(word)
    }

    #[must_use]
    pub fn meaning(&self, symbol: impl crate::interner::SymbolReference) -> Meaning {
        self.stores.meaning(symbol)
    }

    pub fn set_meaning(&mut self, symbol: impl crate::interner::SymbolReference, meaning: Meaning) {
        self.stores.set_meaning(symbol, meaning);
    }

    pub fn intern_relaxed_control_sequence(&mut self, name: &str) -> SymbolId {
        self.stores.intern_relaxed_control_sequence(name)
    }

    pub fn set_meaning_global(
        &mut self,
        symbol: impl crate::interner::SymbolReference,
        meaning: Meaning,
    ) {
        self.stores.set_meaning_global(symbol, meaning);
    }

    pub fn intern_macro(&mut self, macro_meaning: MacroMeaning) -> MacroDefinitionId {
        self.stores.intern_macro(macro_meaning)
    }

    pub fn intern_macro_with_provenance(
        &mut self,
        macro_meaning: MacroMeaning,
        provenance: MacroDefinitionProvenance,
    ) -> MacroDefinitionId {
        self.stores
            .intern_macro_with_provenance(macro_meaning, Some(provenance))
    }

    #[must_use]
    pub fn macro_definition(&self, id: MacroDefinitionId) -> MacroMeaning {
        self.stores.macro_definition(id)
    }

    #[must_use]
    pub fn macro_definition_provenance(&self, id: MacroDefinitionId) -> MacroDefinitionProvenance {
        self.stores.macro_definition_provenance(id)
    }

    pub fn set_macro_meaning(
        &mut self,
        symbol: impl crate::interner::SymbolReference,
        macro_meaning: MacroMeaning,
    ) {
        self.stores.set_macro_meaning(symbol, macro_meaning);
    }

    pub fn set_macro_meaning_with_provenance(
        &mut self,
        symbol: impl crate::interner::SymbolReference,
        macro_meaning: MacroMeaning,
        provenance: MacroDefinitionProvenance,
    ) {
        self.stores
            .set_macro_meaning_with_provenance(symbol, macro_meaning, provenance);
    }

    pub fn set_macro_meaning_global(
        &mut self,
        symbol: impl crate::interner::SymbolReference,
        macro_meaning: MacroMeaning,
    ) {
        self.stores.set_macro_meaning_global(symbol, macro_meaning);
    }

    pub fn set_macro_meaning_global_with_provenance(
        &mut self,
        symbol: impl crate::interner::SymbolReference,
        macro_meaning: MacroMeaning,
        provenance: MacroDefinitionProvenance,
    ) {
        self.stores
            .set_macro_meaning_global_with_provenance(symbol, macro_meaning, provenance);
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

    /// Interns an active-character control sequence in its TeX82 namespace.
    pub fn intern_active_character(&mut self, ch: char) -> SymbolId {
        self.stores.intern_active_character(ch)
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

    pub fn intern_token_list(&mut self, tokens: &[Token]) -> TokenListId {
        self.stores.intern_token_list(tokens)
    }

    pub fn finish_token_list(&mut self, builder: &mut TokenListBuilder) -> TokenListId {
        self.stores.finish_token_list(builder)
    }

    /// Freezes paired semantic tokens and diagnostic origins through the
    /// aggregate state boundary.
    pub fn finish_traced_token_list(&mut self, tokens: &[TracedTokenWord]) -> TracedTokenList {
        self.stores.finish_traced_token_list(tokens)
    }

    #[must_use]
    pub fn tokens(&self, id: TokenListId) -> &[Token] {
        self.stores.tokens(id)
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

    /// Allocates an inserted-token origin.
    pub fn inserted_origin(
        &mut self,
        kind: InsertedOriginKind,
        token: Token,
        parent: OriginId,
    ) -> OriginId {
        self.stores.inserted_origin(kind, token, parent)
    }

    /// Allocates a synthesized-token origin.
    pub fn synthesized_origin(
        &mut self,
        kind: SynthesizedOriginKind,
        parent: OriginId,
    ) -> OriginId {
        self.stores.synthesized_origin(kind, parent)
    }

    /// Allocates a synthetic/bootstrap origin.
    pub fn synthetic_origin(&mut self, kind: SyntheticOriginKind) -> OriginId {
        self.stores.synthetic_origin(kind)
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

    /// Allocates an origin-list span.
    pub fn allocate_origin_list(&mut self, origins: &[OriginId]) -> OriginListId {
        self.stores.allocate_origin_list(origins)
    }

    /// Allocates an origin-list span by repeating one live origin.
    pub fn allocate_repeated_origin_list(&mut self, origin: OriginId, len: usize) -> OriginListId {
        self.stores.allocate_repeated_origin_list(origin, len)
    }

    /// Creates a fresh owned scratch origin-list builder.
    #[must_use]
    pub fn origin_list_builder(&self) -> OriginListBuilder {
        self.stores.origin_list_builder()
    }

    /// Allocates the current origin-list builder value and clears it for reuse.
    pub fn finish_origin_list(&mut self, builder: &mut OriginListBuilder) -> OriginListId {
        self.stores.finish_origin_list(builder)
    }

    /// Reads a live origin-list span.
    #[must_use]
    pub fn origin_list(&self, id: OriginListId) -> &[OriginId] {
        self.stores.origin_list(id)
    }

    /// Reads an origin-list span if it is still live on this timeline.
    #[must_use]
    pub fn origin_list_if_live(&self, id: OriginListId) -> Option<&[OriginId]> {
        self.stores.origin_list_if_live(id)
    }

    /// Returns live provenance arena length counters.
    #[must_use]
    pub fn provenance_stats(&self) -> ProvenanceStats {
        self.stores.provenance_stats()
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

    pub(crate) fn direct_source_origin(
        &self,
        origin: OriginId,
    ) -> Option<crate::provenance::SourceOrigin> {
        self.stores.direct_source_origin(origin)
    }

    /// Tests an inserted-origin classification without resolving source origins.
    #[must_use]
    pub fn origin_is_inserted_kind(&self, id: OriginId, kind: InsertedOriginKind) -> bool {
        matches!(id.decode(), crate::token::OriginEncoding::Arena(_))
            && matches!(
                self.stores.origin_if_live(id),
                Some(OriginRecord::Inserted(inserted)) if inserted.kind() == kind
            )
    }

    pub(crate) fn source_origin_at_position(
        &self,
        position: SourcePos,
    ) -> Option<crate::provenance::SourceOrigin> {
        self.stores.source_origin_at_position(position)
    }

    pub fn intern_glue(&mut self, spec: GlueSpec) -> GlueId {
        self.stores.intern_glue(spec)
    }

    #[must_use]
    pub fn glue(&self, id: GlueId) -> GlueSpec {
        self.stores.glue(id)
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

    #[must_use]
    pub fn font(&self, id: FontId) -> &LoadedFont {
        self.stores.font(id)
    }

    #[must_use]
    pub fn font_name(&self, id: FontId) -> String {
        self.stores.font_name(id)
    }

    #[must_use]
    pub fn font_identifier_symbol(&self, id: FontId) -> Option<SymbolId> {
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
        self.stores.font_metrics(font)
    }

    #[must_use]
    pub fn font_char_exists(&self, font: FontId, code: u8) -> bool {
        self.stores.font_char_exists(font, code)
    }

    #[must_use]
    pub fn font_char_metrics(&self, font: FontId, code: u8) -> Option<CharMetrics> {
        self.stores.font_char_metrics(font, code)
    }

    /// Returns the immutable dense TFM-byte width projection for a live font.
    #[must_use]
    pub fn font_widths(&self, font: FontId) -> &[Scaled; 256] {
        self.stores.font_widths(font)
    }

    #[must_use]
    pub fn font_characters(&self, font: FontId) -> &[Option<CharMetrics>] {
        self.stores.font_characters(font)
    }

    #[must_use]
    pub fn font_next_larger(&self, font: FontId, code: u8) -> Option<u8> {
        self.stores.font_next_larger(font, code)
    }

    #[must_use]
    pub fn missing_font_character(&self, font: FontId, code: u8) -> Option<MissingCharacter> {
        self.stores.missing_font_character(font, code)
    }

    #[must_use]
    pub fn lig_kern_iter(
        &self,
        font: FontId,
        left: LigKernChar,
        right: LigKernChar,
    ) -> LigKernIter<'_> {
        self.stores.lig_kern_iter(font, left, right)
    }

    #[must_use]
    pub fn lig_kern_command(
        &self,
        font: FontId,
        left: LigKernChar,
        right: LigKernChar,
    ) -> Option<LigKernCommand> {
        self.stores.lig_kern_command(font, left, right)
    }

    #[must_use]
    pub fn extensible_recipe(&self, font: FontId, code: u8) -> Option<ExtensibleRecipe> {
        self.stores.extensible_recipe(font, code)
    }

    #[must_use]
    pub fn font_parameter(&self, font: FontId, number: u32) -> Scaled {
        self.stores.font_parameter(font, number)
    }

    #[must_use]
    pub fn current_font(&self) -> FontId {
        self.stores.current_font()
    }

    #[must_use]
    pub fn current_font_symbol(&self) -> Option<SymbolId> {
        self.stores.current_font_symbol()
    }

    pub fn set_current_font(&mut self, id: FontId) {
        self.stores.set_current_font(id);
    }

    pub fn set_current_font_global(&mut self, id: FontId) {
        self.stores.set_current_font_global(id);
    }

    pub fn set_current_font_selector(
        &mut self,
        symbol: impl crate::interner::SymbolReference,
        id: FontId,
    ) {
        self.stores.set_current_font_selector(symbol, id);
    }

    pub fn set_current_font_selector_global(
        &mut self,
        symbol: impl crate::interner::SymbolReference,
        id: FontId,
    ) {
        self.stores.set_current_font_selector_global(symbol, id);
    }

    #[must_use]
    pub fn math_family_font(&self, size: MathFontSize, family: u8) -> FontId {
        self.stores.math_family_font(size, family)
    }

    pub fn set_math_family_font(
        &mut self,
        size: MathFontSize,
        family: u8,
        id: FontId,
        global: bool,
    ) {
        self.stores.set_math_family_font(size, family, id, global);
    }

    #[must_use]
    pub fn font_dimen(&self, font: FontId, number: u32) -> Scaled {
        self.stores.font_dimen(font, number)
    }

    #[must_use]
    pub fn font_parameter_count(&self, font: FontId) -> u32 {
        self.stores.font_parameter_count(font)
    }

    pub fn set_font_dimen(
        &mut self,
        font: FontId,
        number: u32,
        value: Scaled,
        global: bool,
    ) -> Result<(), FontParameterError> {
        self.stores.set_font_dimen(font, number, value, global)
    }

    #[must_use]
    pub fn font_hyphen_char(&self, font: FontId) -> i32 {
        self.stores.font_hyphen_char(font)
    }

    pub fn set_font_hyphen_char(&mut self, font: FontId, value: i32, global: bool) {
        self.stores.set_font_hyphen_char(font, value, global);
    }

    #[must_use]
    pub fn font_skew_char(&self, font: FontId) -> i32 {
        self.stores.font_skew_char(font)
    }

    pub fn set_font_skew_char(&mut self, font: FontId, value: i32, global: bool) {
        self.stores.set_font_skew_char(font, value, global);
    }

    #[must_use]
    pub fn node_list_builder(&self) -> NodeListBuilder {
        self.stores.node_list_builder()
    }

    pub fn freeze_node_list(&mut self, nodes: &[Node]) -> NodeListId {
        self.stores.freeze_node_list(nodes)
    }

    pub fn freeze_node_list_owned(&mut self, nodes: &mut Vec<Node>) -> NodeListId {
        self.stores.freeze_node_list_owned(nodes)
    }

    pub fn finish_node_list(&mut self, builder: &mut NodeListBuilder) -> NodeListId {
        self.stores.finish_node_list(builder)
    }

    #[must_use]
    pub fn nodes(&self, id: NodeListId) -> NodeList<'_> {
        self.stores.nodes(id)
    }

    #[must_use]
    pub fn innermost_group_kind(&self) -> Option<GroupKind> {
        self.stores.innermost_group_kind()
    }

    #[must_use]
    pub fn group_kinds(&self) -> impl DoubleEndedIterator<Item = GroupKind> + '_ {
        self.stores.group_kinds()
    }

    pub fn enter_group(&mut self) {
        self.stores.enter_group();
    }

    pub fn enter_group_with_kind(&mut self, kind: GroupKind) {
        self.stores.enter_group_with_kind(kind);
    }

    pub fn push_aftergroup(&mut self, payload: Token) {
        self.stores.push_aftergroup(payload);
    }

    #[must_use]
    pub fn leave_group(&mut self) -> Vec<Token> {
        let tokens = self.stores.leave_group();
        self.retarget_hash_base_after_group_compaction();
        tokens
    }

    pub fn leave_group_with_kind(
        &mut self,
        expected: GroupKind,
    ) -> Result<Vec<Token>, GroupMismatch> {
        let tokens = self.stores.leave_group_with_kind(expected)?;
        self.retarget_hash_base_after_group_compaction();
        Ok(tokens)
    }

    pub fn set_afterassignment(&mut self, token: Token) {
        self.stores.set_afterassignment(token);
    }

    pub fn take_afterassignment(&mut self) -> Option<Token> {
        self.stores.take_afterassignment()
    }

    pub fn set_count(&mut self, index: u16, value: i32) {
        self.stores.set_count(index, value);
    }

    #[must_use]
    pub fn count(&self, index: u16) -> i32 {
        self.stores.count(index)
    }

    pub fn set_count_global(&mut self, index: u16, value: i32) {
        self.stores.set_count_global(index, value);
    }

    pub fn set_dimen(&mut self, index: u16, value: Scaled) {
        self.stores.set_dimen(index, value);
    }

    #[must_use]
    pub fn dimen(&self, index: u16) -> Scaled {
        self.stores.dimen(index)
    }

    pub fn set_dimen_global(&mut self, index: u16, value: Scaled) {
        self.stores.set_dimen_global(index, value);
    }

    pub fn set_skip(&mut self, index: u16, value: GlueId) {
        self.stores.set_skip(index, value);
    }

    #[must_use]
    pub fn skip(&self, index: u16) -> GlueId {
        self.stores.skip(index)
    }

    pub fn set_skip_global(&mut self, index: u16, value: GlueId) {
        self.stores.set_skip_global(index, value);
    }

    pub fn set_muskip(&mut self, index: u16, value: GlueId) {
        self.stores.set_muskip(index, value);
    }

    #[must_use]
    pub fn muskip(&self, index: u16) -> GlueId {
        self.stores.muskip(index)
    }

    pub fn set_muskip_global(&mut self, index: u16, value: GlueId) {
        self.stores.set_muskip_global(index, value);
    }

    pub fn set_toks(&mut self, index: u16, value: TokenListId) {
        self.stores.set_toks(index, value);
    }

    #[must_use]
    pub fn toks(&self, index: u16) -> TokenListId {
        self.stores.toks(index)
    }

    pub fn set_toks_global(&mut self, index: u16, value: TokenListId) {
        self.stores.set_toks_global(index, value);
    }

    pub fn set_box_reg(&mut self, index: u16, value: NodeListId) {
        self.stores.set_box_reg(index, value);
    }

    pub fn set_box_reg_global(&mut self, index: u16, value: NodeListId) {
        self.stores.set_box_reg_global(index, value);
    }

    /// Marks the epoch-node suffix owned by one box-register value scan.
    #[must_use]
    pub fn begin_box_build(&mut self) -> BoxBuildTransaction<'_> {
        let node_mark = self.stores.shipout_node_mark();
        BoxBuildTransaction {
            universe: self,
            node_mark,
            finished: false,
        }
    }

    #[must_use]
    pub fn box_reg(&self, index: u16) -> Option<NodeListId> {
        self.stores.box_reg(index)
    }

    #[must_use]
    pub fn page_dimension(&self, dimension: PageDimension) -> Scaled {
        self.page.dimension(dimension)
    }

    pub fn set_page_dimension(&mut self, dimension: PageDimension, value: Scaled) {
        self.page.set_dimension(dimension, value);
    }

    #[must_use]
    pub fn page_integer(&self, integer: PageInteger) -> i32 {
        self.page.integer(integer)
    }

    pub fn set_page_integer(&mut self, integer: PageInteger, value: i32) {
        self.page.set_integer(integer, value);
    }

    #[must_use]
    pub fn page_mark(&self, mark: PageMark) -> TokenListId {
        self.page.mark(mark)
    }

    pub fn set_page_mark(&mut self, mark: PageMark, value: TokenListId) {
        let _ = self.stores.tokens(value);
        self.page.set_mark(mark, value);
    }

    #[must_use]
    pub fn page_mark_class(&self, mark: PageMark, class: u16) -> TokenListId {
        self.page.mark_class(mark, class)
    }

    pub fn set_page_mark_class(&mut self, mark: PageMark, class: u16, value: TokenListId) {
        let _ = self.stores.tokens(value);
        self.page.set_mark_class(mark, class, value);
    }

    pub fn page_mark_classes(&self) -> impl Iterator<Item = u16> + '_ {
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
    }

    pub fn start_new_page(&mut self) {
        self.page.start_new_page();
    }

    #[must_use]
    pub fn page_discards(&self) -> &[Node] {
        self.page.page_discards()
    }

    pub fn push_page_discard(&mut self, node: Node) {
        self.stores.assert_live_handles_in_node(&node);
        self.page.push_page_discard(node);
    }

    pub fn take_page_discards(&mut self) -> Vec<Node> {
        self.page.take_page_discards()
    }

    pub fn clear_page_discards(&mut self) {
        self.page.clear_page_discards();
    }

    #[must_use]
    pub fn split_discards(&self) -> &[Node] {
        self.page.split_discards()
    }

    pub fn set_split_discards(&mut self, nodes: Vec<Node>) {
        for node in &nodes {
            self.stores.assert_live_handles_in_node(node);
        }
        self.page.set_split_discards(nodes);
    }

    pub fn take_split_discards(&mut self) -> Vec<Node> {
        self.page.take_split_discards()
    }

    pub fn clear_split_discards(&mut self) {
        self.page.clear_split_discards();
    }

    #[must_use]
    pub fn page_contents(&self) -> PageContents {
        self.page.contents()
    }

    pub fn set_page_contents(&mut self, contents: PageContents) {
        self.page.set_contents(contents);
    }

    #[must_use]
    pub fn page_max_depth(&self) -> Scaled {
        self.page.page_max_depth()
    }

    #[must_use]
    pub fn insert_penalties(&self) -> i32 {
        self.page.insert_penalties()
    }

    #[must_use]
    pub fn least_page_cost(&self) -> i32 {
        self.page.least_page_cost()
    }

    #[must_use]
    pub fn best_page_break(&self) -> Option<PageBreak> {
        self.page.best_page_break()
    }

    #[must_use]
    pub fn best_size(&self) -> Scaled {
        self.page.best_size()
    }

    pub fn record_best_page_break(&mut self, break_index: usize, best_size: Scaled, cost: i32) {
        self.page.record_best_break(break_index, best_size, cost);
    }

    pub fn record_page_fire_up(&mut self, trigger_index: usize) {
        self.page.record_fire_up(trigger_index);
    }

    #[must_use]
    pub fn page_fire_up(&self) -> Option<PageFireUp> {
        self.page.fire_up()
    }

    pub fn append_page_contribution(&mut self, node: Node) {
        self.stores.assert_live_handles_in_node(&node);
        self.page.push_contribution(node);
    }

    pub fn prepend_page_contribution(&mut self, node: Node) {
        self.stores.assert_live_handles_in_node(&node);
        self.page.prepend_contribution(node);
    }

    #[must_use]
    pub fn page_contributions(&self) -> &std::collections::VecDeque<Node> {
        self.page.contribution()
    }

    #[must_use]
    pub fn page_contribution_front(&self) -> Option<&Node> {
        self.page.contribution_front()
    }

    #[must_use]
    pub fn page_contribution_second(&self) -> Option<&Node> {
        self.page.contribution_second()
    }

    #[must_use]
    pub fn page_contribution_tail(&self) -> Option<&Node> {
        self.page.contribution_tail()
    }

    pub fn pop_page_contribution_front(&mut self) -> Option<Node> {
        self.page.pop_contribution_front()
    }

    pub fn pop_page_contribution_tail(&mut self) -> Option<Node> {
        self.page.pop_contribution_tail()
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
        Some(node)
    }

    pub fn prepend_page_contributions(&mut self, nodes: Vec<Node>) {
        self.stores.assert_live_handles_in_nodes(&nodes);
        self.page.prepend_contributions(nodes);
    }

    #[must_use]
    pub fn current_page_nodes(&self) -> Vec<Node> {
        self.page.current_page().cloned().collect()
    }

    #[must_use]
    pub fn current_page_tail(&self) -> Option<&Node> {
        self.page.current_page_tail()
    }

    #[must_use]
    pub fn current_page_len(&self) -> usize {
        self.page.current_page_len()
    }

    pub fn push_current_page_node(&mut self, node: Node) {
        self.stores.assert_live_handles_in_node(&node);
        self.page.push_current_page(node);
    }

    #[must_use]
    pub fn page_insertions(&self) -> &[PageInsertion] {
        self.page.page_insertions()
    }

    #[must_use]
    pub fn page_insertion(&self, class: u16) -> Option<PageInsertion> {
        self.page.page_insertion(class)
    }

    pub fn upsert_page_insertion(&mut self, insertion: PageInsertion) {
        self.page.upsert_page_insertion(insertion);
    }

    pub fn take_current_page_prefix(&mut self, split_index: usize) -> (Vec<Node>, Vec<Node>) {
        self.page.take_current_page_prefix(split_index)
    }

    pub fn update_page_last_from_node(&mut self, node: &Node) {
        self.page.update_last_from_node(node);
    }

    #[must_use]
    pub fn page_last_skip(&self) -> GlueSpec {
        self.page.last_skip(|id| self.glue(id))
    }

    #[must_use]
    pub fn page_last_penalty(&self) -> i32 {
        self.page.last_penalty()
    }

    #[must_use]
    pub fn page_last_kern(&self) -> Scaled {
        self.page.last_kern()
    }

    #[must_use]
    pub fn page_last_node_type(&self) -> i32 {
        self.page.last_node_type()
    }

    pub fn take_box_reg(&mut self, index: u16) -> Option<NodeListId> {
        let value = self.stores.box_reg(index);
        if let Some(value) = value {
            self.stores.pin_survivor(value);
        }
        let _ = self.stores.take_box_reg(index);
        value
    }

    pub fn take_box_reg_same_level(&mut self, index: u16) -> Option<NodeListId> {
        let value = self.stores.box_reg(index);
        if let Some(value) = value {
            self.stores.pin_survivor(value);
        }
        let _ = self.stores.take_box_reg_same_level(index);
        value
    }

    /// Keeps a survivor root alive after a non-destructive register read.
    pub fn pin_survivor(&mut self, id: NodeListId) {
        self.stores.pin_survivor(id);
    }

    /// Pins compatible box children, then clears the register with same-level
    /// TeX assignment semantics.
    ///
    /// Compatibility is checked before mutation, and the children are cloned
    /// while the survivor-backed register owner is still live. The outer
    /// one-node box wrapper is deliberately not retained by the consumer.
    pub fn take_unbox_children_same_level(
        &mut self,
        index: u16,
        expected: UnboxKind,
    ) -> TakeUnboxResult {
        let Some(value) = self.stores.box_reg(index) else {
            return TakeUnboxResult::Void;
        };
        let nodes = self.nodes(value);
        if nodes.len() != 1 {
            return TakeUnboxResult::Incompatible;
        }
        let children = match (expected, nodes.first()) {
            (UnboxKind::Horizontal, Some(crate::node_arena::NodeRef::HList(box_node)))
            | (UnboxKind::Vertical, Some(crate::node_arena::NodeRef::VList(box_node))) => {
                box_node.children
            }
            _ => return TakeUnboxResult::Incompatible,
        };
        self.stores.pin_survivor(value);
        let taken = self.stores.take_box_reg_same_level(index);
        debug_assert_eq!(taken, Some(value));
        TakeUnboxResult::Children(children)
    }

    pub fn set_box_reg_same_level(&mut self, index: u16, value: NodeListId) {
        self.stores.set_box_reg_same_level(index, value);
    }

    pub fn clear_box_reg(&mut self, index: u16) {
        self.stores.clear_box_reg(index);
    }

    pub fn clear_box_reg_global(&mut self, index: u16) {
        self.stores.clear_box_reg_global(index);
    }

    pub fn clear_box_reg_same_level(&mut self, index: u16) {
        self.stores.clear_box_reg_same_level(index);
    }

    #[must_use]
    pub fn box_dimension(&self, index: u16, dimension: BoxDimension) -> Option<Scaled> {
        let id = self.box_reg(index)?;
        box_dimension_from_nodes(self.nodes(id), dimension)
    }

    pub fn set_box_dimension(&mut self, index: u16, dimension: BoxDimension, value: Scaled) {
        self.set_box_dimension_impl(index, dimension, value, false);
    }

    pub fn set_box_dimension_global(&mut self, index: u16, dimension: BoxDimension, value: Scaled) {
        self.set_box_dimension_impl(index, dimension, value, true);
    }

    fn set_box_dimension_impl(
        &mut self,
        index: u16,
        dimension: BoxDimension,
        value: Scaled,
        global: bool,
    ) {
        let Some(id) = self.box_reg(index) else {
            return;
        };
        let Some(mut node) = self.nodes(id).first().map(|node| node.to_owned()) else {
            return;
        };
        if !set_box_dimension_in_node(&mut node, dimension, value) {
            return;
        }
        let rewritten = self.freeze_node_list(&[node]);
        if global {
            self.set_box_reg_global(index, rewritten);
        } else {
            self.set_box_reg(index, rewritten);
        }
    }

    pub fn set_int_param(&mut self, param: IntParam, value: i32) {
        self.stores.set_int_param(param, value);
    }

    pub fn set_int_param_global(&mut self, param: IntParam, value: i32) {
        self.stores.set_int_param_global(param, value);
    }

    #[must_use]
    pub fn int_param(&self, param: IntParam) -> i32 {
        self.stores.int_param(param)
    }

    #[must_use]
    pub fn last_badness(&self) -> i32 {
        self.stores.last_badness()
    }

    pub fn set_last_badness(&mut self, value: i32) {
        self.stores.set_last_badness(value);
    }

    #[must_use]
    pub fn mag(&self) -> i32 {
        self.stores.mag()
    }

    pub fn set_mag(&mut self, value: i32) {
        self.stores.set_mag(value);
    }

    pub fn set_mag_global(&mut self, value: i32) {
        self.stores.set_mag_global(value);
    }

    #[must_use]
    pub fn prepared_mag(&self) -> Option<i32> {
        self.stores.prepared_mag()
    }

    pub fn prepare_mag(&mut self) -> (i32, Option<PrepareMagDiagnostic>) {
        self.stores.prepare_mag()
    }

    #[must_use]
    pub fn endlinechar(&self) -> i32 {
        self.stores.endlinechar()
    }

    pub fn set_dimen_param(&mut self, param: DimenParam, value: Scaled) {
        self.stores.set_dimen_param(param, value);
    }

    pub fn set_dimen_param_global(&mut self, param: DimenParam, value: Scaled) {
        self.stores.set_dimen_param_global(param, value);
    }

    #[must_use]
    pub fn dimen_param(&self, param: DimenParam) -> Scaled {
        self.stores.dimen_param(param)
    }

    pub fn set_glue_param(&mut self, param: GlueParam, value: GlueId) {
        self.stores.set_glue_param(param, value);
    }

    #[must_use]
    pub fn glue_param(&self, param: GlueParam) -> GlueId {
        self.stores.glue_param(param)
    }

    pub fn set_glue_param_global(&mut self, param: GlueParam, value: GlueId) {
        self.stores.set_glue_param_global(param, value);
    }

    pub fn set_tok_param(&mut self, param: TokParam, value: TokenListId) {
        self.stores.set_tok_param(param, value);
    }

    #[must_use]
    pub fn tok_param(&self, param: TokParam) -> TokenListId {
        self.stores.tok_param(param)
    }

    pub fn set_tok_param_global(&mut self, param: TokParam, value: TokenListId) {
        self.stores.set_tok_param_global(param, value);
    }

    /// Returns the current barriered, group-scoped `\parshape` value.
    #[must_use]
    pub fn paragraph_shape(&self) -> Vec<ParagraphShapeLine> {
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
        let id = self.intern_token_list(&tokens);
        if global {
            self.set_tok_param_global(TokParam::PAR_SHAPE_INTERNAL, id);
        } else {
            self.set_tok_param(TokParam::PAR_SHAPE_INTERNAL, id);
        }
    }

    /// Returns a current e-TeX penalty array through the state facade.
    #[must_use]
    pub fn penalty_array(&self, kind: PenaltyArrayKind) -> Vec<i32> {
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
        let mut tokens = Vec::with_capacity(values.len().saturating_mul(4));
        for value in values {
            tokens.extend(value.to_le_bytes().into_iter().map(Token::Param));
        }
        let id = self.intern_token_list(&tokens);
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

    #[cfg(feature = "shadow")]
    pub fn verify_shadow(&self) {
        self.stores.verify_shadow();
    }

    #[cfg(any(test, feature = "testing", feature = "shadow"))]
    #[must_use]
    pub fn testing_state_hash(&self) -> u64 {
        let mut hasher = ahash::AHasher::default();
        self.stores.testing_state_hash().hash(&mut hasher);
        self.world.testing_state_hash().hash(&mut hasher);
        self.input_summary.hash(&mut hasher);
        self.interaction_mode.hash(&mut hasher);
        self.hash_page_state(&mut PageHashCache::default())
            .fingerprint()
            .hash(&mut hasher);
        hasher.finish()
    }

    #[cfg(any(test, feature = "testing", feature = "shadow"))]
    pub fn testing_hash_node_list_content(&self, id: NodeListId, hasher: &mut impl Hasher) {
        self.stores.testing_hash_node_list_content(id, hasher);
    }

    #[cfg(any(test, feature = "testing"))]
    pub fn testing_clear_state_hash_caches(&mut self) {
        self.stores.testing_clear_semantic_hash_cache();
        self.state_hash_projection_cache.clear();
    }

    #[cfg(test)]
    fn testing_input_projection_hash_calls(&self) -> usize {
        self.state_hash_projection_cache.input_hash_calls
    }

    #[cfg(any(test, feature = "testing"))]
    #[must_use]
    pub fn testing_live_survivor_slot_count(&self) -> usize {
        self.stores.testing_live_survivor_slot_count()
    }

    #[cfg(any(test, feature = "testing"))]
    #[must_use]
    pub fn testing_epoch_node_count(&self) -> usize {
        self.stores.testing_epoch_node_count()
    }

    #[cfg(any(test, feature = "testing"))]
    #[must_use]
    pub fn testing_survivor_refcount(&self, id: NodeListId) -> u32 {
        self.stores.testing_survivor_refcount(id)
    }

    #[cfg(any(test, feature = "testing"))]
    #[must_use]
    pub fn testing_survivor_recycled_buffer_uses(&self) -> usize {
        self.stores.testing_survivor_recycled_buffer_uses()
    }

    #[cfg(any(test, feature = "testing"))]
    #[must_use]
    pub fn testing_survivor_root_slot_count(&self) -> usize {
        self.stores.testing_survivor_root_slot_count()
    }

    /// Returns `(clone operations, epoch-owned source lists visited)` for
    /// focused ownership-transfer tests.
    #[cfg(any(test, feature = "testing"))]
    #[must_use]
    pub fn testing_epoch_clone_counts(&self) -> (u64, u64) {
        self.stores.testing_epoch_clone_counts()
    }

    #[cfg(any(test, feature = "testing"))]
    #[must_use]
    pub fn testing_survivor_pin_count(&self) -> usize {
        self.stores.testing_survivor_pin_count()
    }

    #[cfg(any(test, feature = "testing"))]
    #[must_use]
    pub fn testing_survivor_pin_retained_bytes(&self) -> usize {
        self.stores.testing_survivor_pin_retained_bytes()
    }

    /// Computes allocator-payload accounting for all compact node storage.
    /// The returned diagnostic value is not semantic engine state.
    #[cfg(feature = "profiling-stats")]
    #[must_use]
    pub fn node_memory_columns(&self) -> Vec<crate::node_arena::NodeMemoryColumn> {
        self.stores.node_memory_columns()
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
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TakeUnboxResult {
    Void,
    Incompatible,
    Children(NodeListId),
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

impl ExpansionState for Universe {
    fn execution_group_depth(&self) -> u32 {
        self.stores.env_group_depth()
    }
    fn current_group_kind(&self) -> Option<GroupKind> {
        self.innermost_group_kind()
    }
    fn interaction_mode_value(&self) -> i32 {
        encode_interaction_mode(self.interaction_mode()).into()
    }
    fn catcode(&self, ch: char) -> Catcode {
        Self::catcode(self, ch)
    }

    fn lccode(&self, ch: char) -> LcCode {
        Self::lccode(self, ch)
    }

    fn uccode(&self, ch: char) -> UcCode {
        Self::uccode(self, ch)
    }

    fn sfcode(&self, ch: char) -> SfCode {
        Self::sfcode(self, ch)
    }

    fn mathcode(&self, ch: char) -> MathCode {
        Self::mathcode(self, ch)
    }

    fn delcode(&self, ch: char) -> DelCode {
        Self::delcode(self, ch)
    }

    fn meaning_cache_guard(&self) -> Option<MeaningCacheGuard> {
        Some(self.stores.meaning_cache_guard())
    }

    fn meaning(&self, symbol: Symbol) -> Meaning {
        Self::meaning(self, self.stores.resolve_stored_symbol(symbol))
    }

    fn macro_definition(&self, id: MacroDefinitionId) -> MacroMeaning {
        Self::macro_definition(self, id)
    }

    fn macro_definition_provenance(&self, id: MacroDefinitionId) -> MacroDefinitionProvenance {
        Self::macro_definition_provenance(self, id)
    }

    fn macro_meaning(&self, symbol: Symbol) -> Option<MacroMeaning> {
        Self::macro_meaning(self, self.stores.resolve_stored_symbol(symbol))
    }

    fn intern_relaxed_control_sequence(&mut self, name: &str) -> Symbol {
        Self::intern_relaxed_control_sequence(self, name).symbol()
    }

    fn intern(&mut self, name: &str) -> Symbol {
        Self::intern(self, name).symbol()
    }

    fn intern_active_character(&mut self, ch: char) -> Symbol {
        Self::intern_active_character(self, ch).symbol()
    }

    fn symbol(&self, name: &str) -> Option<Symbol> {
        Self::symbol(self, name).map(SymbolId::symbol)
    }

    fn active_character_symbol(&self, ch: char) -> Option<Symbol> {
        Self::active_character_symbol(self, ch).map(SymbolId::symbol)
    }

    fn resolve(&self, symbol: Symbol) -> &str {
        Self::resolve(self, self.stores.resolve_stored_symbol(symbol))
    }

    fn control_sequence_kind(&self, symbol: Symbol) -> ControlSequenceKind {
        Self::control_sequence_kind(self, self.stores.resolve_stored_symbol(symbol))
    }

    fn token_list_builder(&self) -> TokenListBuilder {
        Self::token_list_builder(self)
    }

    fn intern_token_list(&mut self, tokens: &[Token]) -> TokenListId {
        Self::intern_token_list(self, tokens)
    }

    fn finish_token_list(&mut self, builder: &mut TokenListBuilder) -> TokenListId {
        Self::finish_token_list(self, builder)
    }

    fn finish_traced_token_list(&mut self, tokens: &[TracedTokenWord]) -> TracedTokenList {
        Self::finish_traced_token_list(self, tokens)
    }

    fn tokens(&self, id: TokenListId) -> &[Token] {
        Self::tokens(self, id)
    }

    fn intern_glue(&mut self, spec: GlueSpec) -> GlueId {
        Self::intern_glue(self, spec)
    }

    fn glue(&self, id: GlueId) -> GlueSpec {
        Self::glue(self, id)
    }

    fn font_name(&self, id: FontId) -> String {
        Self::font_name(self, id)
    }

    fn font_identifier_symbol(&self, id: FontId) -> Option<Symbol> {
        Self::font_identifier_symbol(self, id).map(SymbolId::symbol)
    }

    fn font_parameter(&self, font: FontId, number: u32) -> Scaled {
        Self::font_parameter(self, font, number)
    }

    fn font_dimen(&self, font: FontId, number: u32) -> Scaled {
        Self::font_dimen(self, font, number)
    }

    fn font_parameter_count(&self, font: FontId) -> u32 {
        Self::font_parameter_count(self, font)
    }
    fn font_char_metrics(&self, font: FontId, code: u8) -> Option<crate::font::CharMetrics> {
        Self::font_char_metrics(self, font, code)
    }

    fn font_hyphen_char(&self, font: FontId) -> i32 {
        Self::font_hyphen_char(self, font)
    }

    fn font_skew_char(&self, font: FontId) -> i32 {
        Self::font_skew_char(self, font)
    }

    fn current_font(&self) -> FontId {
        Self::current_font(self)
    }

    fn current_font_symbol(&self) -> Option<Symbol> {
        Self::current_font_symbol(self).map(SymbolId::symbol)
    }

    fn math_family_font(&self, size: MathFontSize, family: u8) -> FontId {
        Self::math_family_font(self, size, family)
    }

    fn nodes(&self, id: NodeListId) -> NodeList<'_> {
        Self::nodes(self, id)
    }

    fn count(&self, index: u16) -> i32 {
        Self::count(self, index)
    }

    fn dimen(&self, index: u16) -> Scaled {
        Self::dimen(self, index)
    }

    fn skip(&self, index: u16) -> GlueId {
        Self::skip(self, index)
    }

    fn muskip(&self, index: u16) -> GlueId {
        Self::muskip(self, index)
    }

    fn toks(&self, index: u16) -> TokenListId {
        Self::toks(self, index)
    }

    fn box_reg(&self, index: u16) -> Option<NodeListId> {
        Self::box_reg(self, index)
    }

    fn page_dimension(&self, dimension: PageDimension) -> Scaled {
        Self::page_dimension(self, dimension)
    }

    fn page_integer(&self, integer: PageInteger) -> i32 {
        Self::page_integer(self, integer)
    }

    fn page_mark(&self, mark: PageMark) -> TokenListId {
        Self::page_mark(self, mark)
    }

    fn page_mark_class(&self, mark: PageMark, class: u16) -> TokenListId {
        Self::page_mark_class(self, mark, class)
    }

    fn penalty_array_value(&self, kind: PenaltyArrayKind, index: i32) -> i32 {
        Self::penalty_array_value(self, kind, index)
    }

    fn paragraph_shape_dimension(&self, line: i32, width: bool) -> Scaled {
        Self::paragraph_shape_dimension(self, line, width)
    }

    fn report_bad_register_code(&mut self, value: i32, maximum: u16) {
        Self::report_bad_register_code(self, value, maximum);
    }

    fn report_missing_font_identifier(&mut self) {
        Self::report_missing_font_identifier(self);
    }

    fn box_dimension(&self, index: u16, dimension: BoxDimension) -> Option<Scaled> {
        Self::box_dimension(self, index, dimension)
    }

    fn int_param(&self, param: IntParam) -> i32 {
        Self::int_param(self, param)
    }

    fn trace_scantokens_boundary(&mut self, opening: bool) {
        if Self::int_param(self, IntParam::TRACING_SCAN_TOKENS) > 0 {
            self.world_mut()
                .write_text(PrintSink::TerminalAndLog, if opening { "( " } else { ")" });
        }
    }

    fn last_badness(&self) -> i32 {
        Self::last_badness(self)
    }

    fn mag(&self) -> i32 {
        Self::mag(self)
    }

    fn prepared_mag(&self) -> Option<i32> {
        Self::prepared_mag(self)
    }

    fn prepare_mag(&mut self) -> (i32, Option<PrepareMagDiagnostic>) {
        Self::prepare_mag(self)
    }

    fn endlinechar(&self) -> i32 {
        Self::endlinechar(self)
    }

    fn dimen_param(&self, param: DimenParam) -> Scaled {
        Self::dimen_param(self, param)
    }

    fn glue_param(&self, param: GlueParam) -> GlueId {
        Self::glue_param(self, param)
    }

    fn tok_param(&self, param: TokParam) -> TokenListId {
        Self::tok_param(self, param)
    }

    fn bootstrap_origin(&self) -> OriginId {
        Self::bootstrap_origin(self)
    }

    fn synthetic_origin(&mut self, kind: SyntheticOriginKind) -> OriginId {
        Self::synthetic_origin(self, kind)
    }

    fn synthesized_origin(&mut self, kind: SynthesizedOriginKind, parent: OriginId) -> OriginId {
        Self::synthesized_origin(self, kind, parent)
    }

    fn source_origin(
        &mut self,
        source: SourceId,
        byte_offset: u64,
        line: u32,
        column: u32,
    ) -> OriginId {
        Self::source_origin(self, source, byte_offset, line, column)
    }

    fn source_origin_with_input_record(
        &mut self,
        source: SourceId,
        input_record: Option<crate::InputRecordId>,
        byte_offset: u64,
        line: u32,
        column: u32,
    ) -> OriginId {
        Self::source_origin_with_input_record(self, source, input_record, byte_offset, line, column)
    }

    fn source_token_origin(
        &mut self,
        source: SourceId,
        byte_offset: u64,
        byte_end: u64,
    ) -> OriginId {
        Self::source_token_origin(self, source, byte_offset, byte_end)
    }

    fn source_range_origin(
        &mut self,
        source: SourceId,
        byte_offset: u64,
        byte_end: u64,
    ) -> OriginId {
        Self::source_range_origin(self, source, byte_offset, byte_end)
    }

    fn source_span_origin(&mut self, span: SourceSpan) -> OriginId {
        Self::source_span_origin(self, span)
    }

    fn register_source(
        &mut self,
        source: SourceId,
        descriptor: SourceDescriptor,
    ) -> Result<SourcePos, SourceMapError> {
        Self::register_source(self, source, descriptor)
    }

    fn register_input_source(
        &mut self,
        source: SourceId,
        descriptor: SourceDescriptor,
    ) -> Result<RegisteredSource, SourceMapError> {
        Self::register_input_source(self, source, descriptor)
    }

    fn macro_invocation_origin(
        &mut self,
        definition: MacroDefinitionId,
        invocation: OriginId,
        definition_origin: OriginId,
        parent_invocation: OriginId,
    ) -> OriginId {
        Self::macro_invocation_origin(
            self,
            definition,
            invocation,
            definition_origin,
            parent_invocation,
        )
    }

    fn inserted_origin(
        &mut self,
        kind: InsertedOriginKind,
        token: Token,
        parent: OriginId,
    ) -> OriginId {
        Self::inserted_origin(self, kind, token, parent)
    }

    fn allocate_repeated_origin_list(&mut self, origin: OriginId, len: usize) -> OriginListId {
        Self::allocate_repeated_origin_list(self, origin, len)
    }

    fn origin_list_builder(&self) -> OriginListBuilder {
        Self::origin_list_builder(self)
    }

    fn finish_origin_list(&mut self, builder: &mut OriginListBuilder) -> OriginListId {
        Self::finish_origin_list(self, builder)
    }

    fn origin_list(&self, id: OriginListId) -> &[OriginId] {
        Self::origin_list(self, id)
    }

    fn origin_list_if_live(&self, id: OriginListId) -> Option<&[OriginId]> {
        Self::origin_list_if_live(self, id)
    }

    fn input_stream_eof(&self, stream: StreamSlot) -> bool {
        self.world.input_stream_eof(stream)
    }
}

impl ExpansionState for ExpansionContext<'_> {
    fn execution_group_depth(&self) -> u32 {
        self.universe.execution_group_depth()
    }
    fn current_group_kind(&self) -> Option<GroupKind> {
        self.universe.innermost_group_kind()
    }
    fn interaction_mode_value(&self) -> i32 {
        self.universe.interaction_mode_value()
    }
    fn catcode(&self, ch: char) -> Catcode {
        self.universe.catcode(ch)
    }

    fn lccode(&self, ch: char) -> LcCode {
        self.universe.lccode(ch)
    }

    fn uccode(&self, ch: char) -> UcCode {
        self.universe.uccode(ch)
    }

    fn sfcode(&self, ch: char) -> SfCode {
        self.universe.sfcode(ch)
    }

    fn mathcode(&self, ch: char) -> MathCode {
        self.universe.mathcode(ch)
    }

    fn delcode(&self, ch: char) -> DelCode {
        self.universe.delcode(ch)
    }

    fn meaning_cache_guard(&self) -> Option<MeaningCacheGuard> {
        Some(self.universe.stores.meaning_cache_guard())
    }

    fn meaning(&self, symbol: Symbol) -> Meaning {
        self.universe
            .meaning(self.universe.stores.resolve_stored_symbol(symbol))
    }

    fn macro_definition(&self, id: MacroDefinitionId) -> MacroMeaning {
        self.universe.macro_definition(id)
    }

    fn macro_definition_provenance(&self, id: MacroDefinitionId) -> MacroDefinitionProvenance {
        self.universe.macro_definition_provenance(id)
    }

    fn macro_meaning(&self, symbol: Symbol) -> Option<MacroMeaning> {
        self.universe
            .macro_meaning(self.universe.stores.resolve_stored_symbol(symbol))
    }

    fn intern_relaxed_control_sequence(&mut self, name: &str) -> Symbol {
        self.universe.intern_relaxed_control_sequence(name).symbol()
    }

    fn intern(&mut self, name: &str) -> Symbol {
        self.universe.intern(name).symbol()
    }

    fn intern_active_character(&mut self, ch: char) -> Symbol {
        self.universe.intern_active_character(ch).symbol()
    }

    fn symbol(&self, name: &str) -> Option<Symbol> {
        self.universe.symbol(name).map(SymbolId::symbol)
    }

    fn active_character_symbol(&self, ch: char) -> Option<Symbol> {
        self.universe
            .active_character_symbol(ch)
            .map(SymbolId::symbol)
    }

    fn resolve(&self, symbol: Symbol) -> &str {
        self.universe
            .resolve(self.universe.stores.resolve_stored_symbol(symbol))
    }

    fn control_sequence_kind(&self, symbol: Symbol) -> ControlSequenceKind {
        self.universe
            .control_sequence_kind(self.universe.stores.resolve_stored_symbol(symbol))
    }

    fn token_list_builder(&self) -> TokenListBuilder {
        self.universe.token_list_builder()
    }

    fn intern_token_list(&mut self, tokens: &[Token]) -> TokenListId {
        self.universe.intern_token_list(tokens)
    }

    fn finish_token_list(&mut self, builder: &mut TokenListBuilder) -> TokenListId {
        self.universe.finish_token_list(builder)
    }

    fn finish_traced_token_list(&mut self, tokens: &[TracedTokenWord]) -> TracedTokenList {
        self.universe.finish_traced_token_list(tokens)
    }

    fn tokens(&self, id: TokenListId) -> &[Token] {
        self.universe.tokens(id)
    }

    fn intern_glue(&mut self, spec: GlueSpec) -> GlueId {
        self.universe.intern_glue(spec)
    }

    fn glue(&self, id: GlueId) -> GlueSpec {
        self.universe.glue(id)
    }

    fn font_name(&self, id: FontId) -> String {
        self.universe.font_name(id)
    }

    fn font_identifier_symbol(&self, id: FontId) -> Option<Symbol> {
        self.universe
            .font_identifier_symbol(id)
            .map(SymbolId::symbol)
    }

    fn font_parameter(&self, font: FontId, number: u32) -> Scaled {
        self.universe.font_parameter(font, number)
    }

    fn font_dimen(&self, font: FontId, number: u32) -> Scaled {
        self.universe.font_dimen(font, number)
    }

    fn font_parameter_count(&self, font: FontId) -> u32 {
        self.universe.font_parameter_count(font)
    }
    fn font_char_metrics(&self, font: FontId, code: u8) -> Option<crate::font::CharMetrics> {
        self.universe.font_char_metrics(font, code)
    }

    fn font_hyphen_char(&self, font: FontId) -> i32 {
        self.universe.font_hyphen_char(font)
    }

    fn font_skew_char(&self, font: FontId) -> i32 {
        self.universe.font_skew_char(font)
    }

    fn current_font(&self) -> FontId {
        self.universe.current_font()
    }

    fn current_font_symbol(&self) -> Option<Symbol> {
        self.universe.current_font_symbol().map(SymbolId::symbol)
    }

    fn math_family_font(&self, size: MathFontSize, family: u8) -> FontId {
        self.universe.math_family_font(size, family)
    }

    fn nodes(&self, id: NodeListId) -> NodeList<'_> {
        self.universe.nodes(id)
    }

    fn count(&self, index: u16) -> i32 {
        self.universe.count(index)
    }

    fn dimen(&self, index: u16) -> Scaled {
        self.universe.dimen(index)
    }

    fn skip(&self, index: u16) -> GlueId {
        self.universe.skip(index)
    }

    fn muskip(&self, index: u16) -> GlueId {
        self.universe.muskip(index)
    }

    fn toks(&self, index: u16) -> TokenListId {
        self.universe.toks(index)
    }

    fn box_reg(&self, index: u16) -> Option<NodeListId> {
        self.universe.box_reg(index)
    }

    fn page_dimension(&self, dimension: PageDimension) -> Scaled {
        self.universe.page_dimension(dimension)
    }

    fn page_integer(&self, integer: PageInteger) -> i32 {
        self.universe.page_integer(integer)
    }

    fn page_mark(&self, mark: PageMark) -> TokenListId {
        self.universe.page_mark(mark)
    }

    fn page_mark_class(&self, mark: PageMark, class: u16) -> TokenListId {
        self.universe.page_mark_class(mark, class)
    }

    fn penalty_array_value(&self, kind: PenaltyArrayKind, index: i32) -> i32 {
        self.universe.penalty_array_value(kind, index)
    }

    fn paragraph_shape_dimension(&self, line: i32, width: bool) -> Scaled {
        self.universe.paragraph_shape_dimension(line, width)
    }

    fn report_bad_register_code(&mut self, value: i32, maximum: u16) {
        self.universe.report_bad_register_code(value, maximum);
    }

    fn report_missing_font_identifier(&mut self) {
        self.universe.report_missing_font_identifier();
    }

    fn box_dimension(&self, index: u16, dimension: BoxDimension) -> Option<Scaled> {
        self.universe.box_dimension(index, dimension)
    }

    fn int_param(&self, param: IntParam) -> i32 {
        self.universe.int_param(param)
    }

    fn trace_scantokens_boundary(&mut self, opening: bool) {
        if self.universe.int_param(IntParam::TRACING_SCAN_TOKENS) > 0 {
            self.universe
                .world_mut()
                .write_text(PrintSink::TerminalAndLog, if opening { "( " } else { ")" });
        }
    }

    fn last_badness(&self) -> i32 {
        self.universe.last_badness()
    }

    fn mag(&self) -> i32 {
        self.universe.mag()
    }

    fn prepared_mag(&self) -> Option<i32> {
        self.universe.prepared_mag()
    }

    fn prepare_mag(&mut self) -> (i32, Option<PrepareMagDiagnostic>) {
        self.universe.prepare_mag()
    }

    fn endlinechar(&self) -> i32 {
        self.universe.endlinechar()
    }

    fn dimen_param(&self, param: DimenParam) -> Scaled {
        self.universe.dimen_param(param)
    }

    fn glue_param(&self, param: GlueParam) -> GlueId {
        self.universe.glue_param(param)
    }

    fn tok_param(&self, param: TokParam) -> TokenListId {
        self.universe.tok_param(param)
    }

    fn bootstrap_origin(&self) -> OriginId {
        self.universe.bootstrap_origin()
    }

    fn synthetic_origin(&mut self, kind: SyntheticOriginKind) -> OriginId {
        self.universe.synthetic_origin(kind)
    }

    fn synthesized_origin(&mut self, kind: SynthesizedOriginKind, parent: OriginId) -> OriginId {
        self.universe.synthesized_origin(kind, parent)
    }

    fn source_origin(
        &mut self,
        source: SourceId,
        byte_offset: u64,
        line: u32,
        column: u32,
    ) -> OriginId {
        self.universe
            .source_origin(source, byte_offset, line, column)
    }

    fn source_origin_with_input_record(
        &mut self,
        source: SourceId,
        input_record: Option<crate::InputRecordId>,
        byte_offset: u64,
        line: u32,
        column: u32,
    ) -> OriginId {
        self.universe.source_origin_with_input_record(
            source,
            input_record,
            byte_offset,
            line,
            column,
        )
    }

    fn source_token_origin(
        &mut self,
        source: SourceId,
        byte_offset: u64,
        byte_end: u64,
    ) -> OriginId {
        self.universe
            .source_token_origin(source, byte_offset, byte_end)
    }

    fn source_range_origin(
        &mut self,
        source: SourceId,
        byte_offset: u64,
        byte_end: u64,
    ) -> OriginId {
        self.universe
            .source_range_origin(source, byte_offset, byte_end)
    }

    fn source_span_origin(&mut self, span: SourceSpan) -> OriginId {
        self.universe.source_span_origin(span)
    }

    fn register_source(
        &mut self,
        source: SourceId,
        descriptor: SourceDescriptor,
    ) -> Result<SourcePos, SourceMapError> {
        self.universe.register_source(source, descriptor)
    }

    fn register_input_source(
        &mut self,
        source: SourceId,
        descriptor: SourceDescriptor,
    ) -> Result<RegisteredSource, SourceMapError> {
        self.universe.register_input_source(source, descriptor)
    }

    fn macro_invocation_origin(
        &mut self,
        definition: MacroDefinitionId,
        invocation: OriginId,
        definition_origin: OriginId,
        parent_invocation: OriginId,
    ) -> OriginId {
        self.universe.macro_invocation_origin(
            definition,
            invocation,
            definition_origin,
            parent_invocation,
        )
    }

    fn inserted_origin(
        &mut self,
        kind: InsertedOriginKind,
        token: Token,
        parent: OriginId,
    ) -> OriginId {
        self.universe.inserted_origin(kind, token, parent)
    }

    fn allocate_repeated_origin_list(&mut self, origin: OriginId, len: usize) -> OriginListId {
        self.universe.allocate_repeated_origin_list(origin, len)
    }

    fn origin_list_builder(&self) -> OriginListBuilder {
        self.universe.origin_list_builder()
    }

    fn finish_origin_list(&mut self, builder: &mut OriginListBuilder) -> OriginListId {
        self.universe.finish_origin_list(builder)
    }

    fn origin_list(&self, id: OriginListId) -> &[OriginId] {
        self.universe.origin_list(id)
    }

    fn origin_list_if_live(&self, id: OriginListId) -> Option<&[OriginId]> {
        self.universe.origin_list_if_live(id)
    }

    fn input_stream_eof(&self, stream: StreamSlot) -> bool {
        self.universe.world.input_stream_eof(stream)
    }
}

impl InputReadState for InputOpenContext<'_> {
    fn read_input_file(
        &mut self,
        path: &std::path::Path,
    ) -> Result<crate::FileContent, crate::WorldError> {
        self.universe.world.read_file(path)
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

impl InputOpenState for ExpansionContext<'_> {
    type Input<'a>
        = InputOpenContext<'a>
    where
        Self: 'a;

    fn input_open_context(&mut self) -> Self::Input<'_> {
        InputOpenContext::new(self.universe)
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

fn hash_rng_state(rng: crate::world::RngState, hasher: &mut StateHasher) {
    hasher.tag(0x84);
    for word in rng.state_words() {
        hasher.u64(word);
    }
}

fn hash_job_clock(clock: JobClock, hasher: &mut StateHasher) {
    hasher.tag(0x85);
    hasher.i32(clock.time);
    hasher.i32(clock.day);
    hasher.i32(clock.month);
    hasher.i32(clock.year);
}

fn hash_shell_escape_policy(policy: ShellEscapePolicy, hasher: &mut StateHasher) {
    hasher.tag(0x86);
    hasher.u8(match policy {
        ShellEscapePolicy::Disabled => 0,
        ShellEscapePolicy::Enabled => 1,
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
            } => {
                hasher.tag(1);
                stores.hash_token_list_semantic(*token_list, hasher);
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
                ControlSequenceKind::Named => 0,
                ControlSequenceKind::ActiveCharacter => 1,
            });
            hasher.str(stores.resolve(symbol));
        }
        Token::Param(slot) => {
            hasher.tag(2);
            hasher.u8(slot);
        }
        Token::Frozen(crate::token::FrozenToken::END_TEMPLATE) => hasher.tag(3),
        Token::Frozen(crate::token::FrozenToken::END_V) => hasher.tag(4),
        Token::Frozen(_) => unreachable!("invalid frozen token payload"),
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
        TokenListReplayKind::EveryPar => 3,
        TokenListReplayKind::EveryCr => 4,
        TokenListReplayKind::Mark => 5,
        TokenListReplayKind::OutputRoutine => 6,
        TokenListReplayKind::Inserted => 7,
        TokenListReplayKind::AlignmentUTemplate => 8,
        TokenListReplayKind::ScantokensEveryEof => 9,
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

fn format_checksum(mode: u8, payload: &[u8]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64 ^ u64::from(mode);
    for byte in payload {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
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
