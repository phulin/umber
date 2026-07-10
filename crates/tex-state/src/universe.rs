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
    ConditionKind, ConditionLimb, InputFrameSummary, InputSummary, LexerState, SourceId,
    TokenListReplayKind,
};
use crate::interner::{ControlSequenceKind, Symbol};
use crate::macro_store::{MacroDefinitionProvenance, MacroMeaning};
use crate::math::MathFontSize;
use crate::meaning::Meaning;
use crate::node::Node;
use crate::node_arena::{NodeList, NodeListBuilder};
use crate::page::{
    PageBreak, PageBuilderState, PageContents, PageDimension, PageFireUp, PageInsertion,
    PageInteger, PageMark,
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
use crate::state_hash::{INITIAL_STATE_HASH, StateHasher, combine};
use crate::stores::StoreStateHashCursor;
use crate::stores::{
    FontParameterError, GroupKind, GroupMismatch, PrepareMagDiagnostic, ShipoutNodeMark,
    StoreSnapshot, Stores,
};
use crate::token::{Catcode, OriginId, Token, TracedTokenWord};
use crate::token_store::TokenListBuilder;
use crate::world::{
    ContentHash, EffectPos, EffectRecord, JobClock, PrintSink, ShellEscapePolicy,
    ShellEscapeRecord, StreamBufState, StreamSlot, World, WorldError, WorldSnapshot,
    WorldStateHashCursor, install_job_clock_params,
};
use std::hash::BuildHasher;
#[cfg(any(test, feature = "testing", feature = "shadow"))]
use std::hash::{Hash, Hasher};

/// State operations available to TeX's lexer and expansion engine.
///
/// This is intentionally narrower than `Universe`: it permits immutable state
/// reads plus the content/interner mutations that the mouth and gullet are
/// semantically allowed to perform, but it does not expose Env, register, box,
/// code-table, font-parameter, grouping, snapshot, input-file reads, or World
/// mutation APIs.
pub trait ExpansionState {
    /// Current execution-group depth used by TeX82 alignment `get_next`.
    fn execution_group_depth(&self) -> u32 {
        0
    }
    fn catcode(&self, ch: char) -> Catcode;
    fn lccode(&self, ch: char) -> LcCode;
    fn uccode(&self, ch: char) -> UcCode;
    fn sfcode(&self, ch: char) -> SfCode;
    fn mathcode(&self, ch: char) -> MathCode;
    fn delcode(&self, ch: char) -> DelCode;
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
    fn tokens(&self, id: TokenListId) -> &[Token];
    fn intern_glue(&mut self, spec: GlueSpec) -> GlueId;
    fn glue(&self, id: GlueId) -> GlueSpec;
    fn font_name(&self, id: FontId) -> String;
    fn font_identifier_symbol(&self, id: FontId) -> Option<Symbol>;
    fn font_parameter(&self, font: FontId, number: u16) -> Scaled;
    fn font_dimen(&self, font: FontId, number: u16) -> Scaled;
    fn font_parameter_count(&self, font: FontId) -> u16;
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
    fn int_param(&self, param: IntParam) -> i32;
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

/// Input file reads available to driver-supplied `\input` hooks.
///
/// This is intentionally separate from [`ExpansionState`] so ordinary gullet
/// code cannot open files and input hooks cannot see expansion, Env/register,
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
    store: StoreSnapshot,
    epoch: Epoch,
    world: WorldSnapshot,
    input_summary: InputSummary,
    interaction_mode: InteractionMode,
    page: PageBuilderState,
    state_hash: u64,
    state_hash_base: StateHashBase,
    checkpoint_id: CheckpointId,
    resume_kind: CheckpointResumeKind,
    resume_fallback: Option<ResumeFallback>,
    last_resume_boundary: Option<ResumeBoundarySnapshot>,
}

/// Timeline-local identifier for one semantic checkpoint.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct CheckpointId(u64);

/// Whether a checkpoint can be used as a direct execution resume point.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum CheckpointResumeKind {
    /// The executor was at a quiescent boundary and can restart directly here.
    ResumeValid,
    /// The checkpoint is valid for convergence hashing, but execution must
    /// resume from the recorded previous resume-valid boundary.
    HashOnly,
}

/// The checkpoint a hash-only checkpoint should resume from.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct ResumeBoundary {
    checkpoint_id: CheckpointId,
    state_hash: u64,
}

impl ResumeBoundary {
    /// Returns the timeline-local checkpoint id for this resume boundary.
    #[must_use]
    pub const fn checkpoint_id(self) -> CheckpointId {
        self.checkpoint_id
    }

    /// Returns the checkpoint-schedule-relative semantic state hash captured
    /// at this resume boundary (see [`Snapshot::state_hash`]).
    #[must_use]
    pub const fn state_hash(self) -> u64 {
        self.state_hash
    }
}

/// Whether execution can roll back directly to a checkpoint's resume boundary.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum ResumeFallback {
    /// The fallback boundary's effect history is still retained, so direct
    /// rollback to that boundary is available.
    DirectRollback(ResumeBoundary),
    /// The fallback boundary is known for replay/convergence, but bounded
    /// effect history has dropped the prefix needed for direct rollback.
    Unavailable(ResumeBoundary),
}

impl ResumeFallback {
    /// Returns the resume-valid checkpoint identity for this fallback.
    #[must_use]
    pub const fn boundary(self) -> ResumeBoundary {
        match self {
            Self::DirectRollback(boundary) | Self::Unavailable(boundary) => boundary,
        }
    }

    /// Returns true when callers can roll back directly to the fallback.
    #[must_use]
    pub const fn direct_rollback_available(self) -> bool {
        matches!(self, Self::DirectRollback(_))
    }
}

/// Public metadata for the most recent semantic checkpoint.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct CheckpointMetadata {
    checkpoint_id: CheckpointId,
    state_hash: u64,
    resume_kind: CheckpointResumeKind,
    resume_fallback: Option<ResumeFallback>,
}

impl CheckpointMetadata {
    /// Returns the timeline-local checkpoint id.
    #[must_use]
    pub const fn checkpoint_id(self) -> CheckpointId {
        self.checkpoint_id
    }

    /// Returns the checkpoint-schedule-relative semantic state hash captured
    /// at this checkpoint (see [`Snapshot::state_hash`]).
    #[must_use]
    pub const fn state_hash(self) -> u64 {
        self.state_hash
    }

    /// Returns whether this checkpoint can be resumed directly.
    #[must_use]
    pub const fn resume_kind(self) -> CheckpointResumeKind {
        self.resume_kind
    }

    /// Returns the fallback boundary to use for execution resume.
    ///
    /// For a resume-valid checkpoint this is the checkpoint itself with
    /// direct rollback available. For a hash-only checkpoint this is the
    /// previous resume-valid boundary, when one has been established on the
    /// current timeline. Hash-only fallbacks can be unavailable for direct
    /// rollback when bounded effect history has already dropped their prefix.
    #[must_use]
    pub const fn resume_fallback(self) -> Option<ResumeFallback> {
        self.resume_fallback
    }
}

/// Opaque state mark for one in-progress shipout operation.
///
/// The mark can only be consumed by [`Universe::commit_shipout`]; it does not
/// expose raw node-arena release or rollback machinery.
#[derive(Debug)]
pub struct ShipoutBoundary {
    owner: SnapshotOwner,
    node_mark: ShipoutNodeMark,
}

/// Opaque allocation mark for one in-progress box-register construction.
///
/// Finishing the assignment promotes its live result into rollback-safe
/// storage, then releases every epoch node allocated during construction.
#[derive(Debug)]
pub struct BoxBuildBoundary {
    owner: SnapshotOwner,
    node_mark: ShipoutNodeMark,
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

    /// Returns this checkpoint's timeline-local id.
    #[must_use]
    pub const fn checkpoint_id(&self) -> CheckpointId {
        self.checkpoint_id
    }

    /// Returns whether this checkpoint can be resumed directly.
    #[must_use]
    pub const fn resume_kind(&self) -> CheckpointResumeKind {
        self.resume_kind
    }

    /// Returns true when the executor can restart directly from this snapshot.
    #[must_use]
    pub const fn is_resume_valid(&self) -> bool {
        matches!(self.resume_kind, CheckpointResumeKind::ResumeValid)
    }

    /// Returns the fallback boundary to use for execution resume.
    ///
    /// Hash-only checkpoints are still valid rollback/hash checkpoints, but
    /// callers that need to restart execution must use this fallback. If the
    /// fallback is unavailable, bounded effect history no longer retains
    /// enough state for direct rollback to that boundary.
    #[must_use]
    pub const fn resume_fallback(&self) -> Option<ResumeFallback> {
        self.resume_fallback
    }

    /// Returns the public metadata for this checkpoint.
    #[must_use]
    pub const fn checkpoint_metadata(&self) -> CheckpointMetadata {
        CheckpointMetadata {
            checkpoint_id: self.checkpoint_id,
            state_hash: self.state_hash,
            resume_kind: self.resume_kind,
            resume_fallback: self.resume_fallback,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ResumeBoundarySnapshot {
    boundary: ResumeBoundary,
    effect_pos: EffectPos,
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
    let state = std::collections::hash_map::RandomState::new();
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

#[derive(Clone, Debug)]
struct StateHashBase {
    store: StoreStateHashCursor,
    world: WorldStateHashCursor,
    input_summary: InputSummary,
    interaction_mode: InteractionMode,
    page: PageBuilderState,
    checkpoint_hash: u64,
}

/// One owned TeX state timeline.
#[derive(Debug)]
pub struct Universe {
    owner: UniverseOwner,
    stores: Stores,
    world: World,
    interaction_mode: InteractionMode,
    input_summary: InputSummary,
    page: PageBuilderState,
    state_hash_base: StateHashBase,
    next_checkpoint_id: u64,
    hash_only_checkpoint_depth: u32,
    last_resume_boundary: Option<ResumeBoundarySnapshot>,
    last_checkpoint: Option<CheckpointMetadata>,
}

impl Clone for Universe {
    fn clone(&self) -> Self {
        let stores = self.stores.clone();
        let state_hash_base = StateHashBase {
            store: stores.retarget_state_hash_cursor(&self.state_hash_base.store),
            world: self.state_hash_base.world.clone(),
            input_summary: self.state_hash_base.input_summary.clone(),
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
            page: self.page.clone(),
            state_hash_base,
            next_checkpoint_id: self.next_checkpoint_id,
            hash_only_checkpoint_depth: self.hash_only_checkpoint_depth,
            last_resume_boundary: None,
            last_checkpoint: None,
        }
    }
}

impl Default for Universe {
    fn default() -> Self {
        Self::new()
    }
}

impl Universe {
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
        let state_hash_base = StateHashBase {
            store: stores.state_hash_cursor(),
            world: world.state_hash_cursor(),
            input_summary: InputSummary::default(),
            interaction_mode: InteractionMode::default(),
            page: PageBuilderState::default(),
            checkpoint_hash: INITIAL_STATE_HASH,
        };
        Self {
            owner: UniverseOwner::new(),
            stores,
            world,
            interaction_mode: InteractionMode::default(),
            input_summary: InputSummary::default(),
            page: PageBuilderState::default(),
            state_hash_base,
            next_checkpoint_id: 1,
            hash_only_checkpoint_depth: 0,
            last_resume_boundary: None,
            last_checkpoint: None,
        }
    }

    /// Takes an O(1) snapshot of the whole timeline tuple.
    #[must_use]
    pub fn snapshot(&mut self) -> Snapshot {
        self.checkpoint_from_hash_base(self.state_hash_base.clone())
    }

    fn checkpoint_from_hash_base(&mut self, hash_base: StateHashBase) -> Snapshot {
        let world = self.world.snapshot();
        let store = self.stores.checkpoint();
        let store_cursor = Stores::state_hash_cursor_from_snapshot(&store);
        let world_cursor = World::state_hash_cursor_from_snapshot(&world);
        let state_hash = if hash_base.store == store_cursor
            && hash_base.world == world_cursor
            && hash_base.input_summary == self.input_summary
            && hash_base.interaction_mode == self.interaction_mode
            && hash_base.page == self.page
        {
            hash_base.checkpoint_hash
        } else {
            let slice_hash = self.state_hash_slice(&hash_base, &store);
            combine(hash_base.checkpoint_hash, slice_hash)
        };
        let next_hash_base = StateHashBase {
            store: store_cursor,
            world: world_cursor,
            input_summary: self.input_summary.clone(),
            interaction_mode: self.interaction_mode,
            page: self.page.clone(),
            checkpoint_hash: state_hash,
        };
        let checkpoint_id = self.allocate_checkpoint_id();
        let resume_kind = if self.hash_only_checkpoint_depth == 0 {
            CheckpointResumeKind::ResumeValid
        } else {
            CheckpointResumeKind::HashOnly
        };
        let own_boundary = ResumeBoundary {
            checkpoint_id,
            state_hash,
        };
        let own_boundary_snapshot = ResumeBoundarySnapshot {
            boundary: own_boundary,
            effect_pos: self.world.effect_pos(),
        };
        let resume_fallback = match resume_kind {
            CheckpointResumeKind::ResumeValid => Some(ResumeFallback::DirectRollback(own_boundary)),
            CheckpointResumeKind::HashOnly => self
                .last_resume_boundary
                .map(|boundary| self.resume_fallback_for(boundary)),
        };
        let last_resume_boundary = match resume_kind {
            CheckpointResumeKind::ResumeValid => Some(own_boundary_snapshot),
            CheckpointResumeKind::HashOnly => self.last_resume_boundary,
        };
        let checkpoint = CheckpointMetadata {
            checkpoint_id,
            state_hash,
            resume_kind,
            resume_fallback,
        };
        if resume_kind == CheckpointResumeKind::ResumeValid {
            self.last_resume_boundary = Some(own_boundary_snapshot);
        }
        self.last_checkpoint = Some(checkpoint);
        self.state_hash_base = next_hash_base.clone();
        Snapshot {
            owner: self.owner.snapshot_owner(),
            epoch: store.epoch(),
            store,
            world,
            input_summary: self.input_summary.clone(),
            interaction_mode: self.interaction_mode,
            page: self.page.clone(),
            state_hash,
            state_hash_base: next_hash_base,
            checkpoint_id,
            resume_kind,
            resume_fallback,
            last_resume_boundary,
        }
    }

    fn resume_fallback_for(&self, boundary: ResumeBoundarySnapshot) -> ResumeFallback {
        if self.world.effect_pos_is_retained(boundary.effect_pos) {
            ResumeFallback::DirectRollback(boundary.boundary)
        } else {
            ResumeFallback::Unavailable(boundary.boundary)
        }
    }

    fn allocate_checkpoint_id(&mut self) -> CheckpointId {
        let id = self.next_checkpoint_id;
        self.next_checkpoint_id = self
            .next_checkpoint_id
            .checked_add(1)
            .expect("checkpoint id overflow");
        CheckpointId(id)
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
            interaction_mode: hash_base.interaction_mode,
            page: hash_base.page,
            checkpoint_hash: hash_base.checkpoint_hash,
        }
    }

    fn checkpoint_after_committed_boundary(&mut self, hash_base: StateHashBase) -> Snapshot {
        let hash_base = self.retarget_hash_base_after_committed_boundary(hash_base);
        self.checkpoint_from_hash_base(hash_base)
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
        self.last_resume_boundary = snapshot.last_resume_boundary;
        self.last_checkpoint = Some(snapshot.checkpoint_metadata());
    }

    fn state_hash_slice(&mut self, hash_base: &StateHashBase, store: &StoreSnapshot) -> u64 {
        let mut hasher = StateHasher::new(0x756e_6976_6572_7365);
        hasher.u64(self.stores.state_hash_slice(&hash_base.store, store));
        self.hash_world_state_slice(&hash_base.world, &mut hasher);
        self.hash_input_summary(&mut hasher);
        hash_interaction_mode(self.interaction_mode, &mut hasher);
        self.hash_page_state(&mut hasher);
        hasher.finish()
    }

    fn hash_world_state_slice(&self, cursor: &WorldStateHashCursor, hasher: &mut StateHasher) {
        hasher.tag(0x80);
        let effects = self.world.effect_records_since(cursor);
        hasher.usize(effects.len());
        for effect in effects {
            self.hash_effect_record(effect, hasher);
        }

        hasher.tag(0x81);
        let inputs = self.world.input_records_since(cursor);
        hasher.usize(inputs.len());
        for input in inputs {
            hash_path(input.path(), hasher);
            hasher.bytes(&input.hash().bytes());
            hasher.usize(input.len());
        }

        hasher.tag(0x82);
        let shell_escapes = self.world.shell_escape_records_since(cursor);
        hasher.usize(shell_escapes.len());
        for record in shell_escapes {
            hash_shell_escape_record(record, hasher);
        }

        hash_stream_bufs(self.world.stream_bufs(), hasher);
        hash_rng_state(self.world.rng_state(), hasher);
        hash_job_clock(self.world.job_clock(), hasher);
        hash_shell_escape_policy(self.world.shell_escape_policy(), hasher);
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

    fn hash_input_summary(&self, hasher: &mut StateHasher) {
        hasher.tag(0x90);
        hash_input_summary_fields(&self.stores, &self.input_summary, hasher);
    }

    fn hash_page_state(&self, hasher: &mut StateHasher) {
        self.page.hash_semantic(
            hasher,
            |nodes, hasher| self.stores.hash_node_slice_semantic(nodes, hasher),
            |id, hasher| self.stores.hash_glue_semantic(id, hasher),
            |id, hasher| self.stores.hash_token_list_semantic(id, hasher),
        );
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

    /// Marks the start of node allocations owned by one in-progress shipout.
    #[must_use]
    pub fn begin_shipout(&self) -> ShipoutBoundary {
        ShipoutBoundary {
            owner: self.owner.snapshot_owner(),
            node_mark: self.stores.shipout_node_mark(),
        }
    }

    /// Stores a shipped page artifact, flushes its effects, releases its
    /// shipout-local epoch nodes, and advances the checkpoint as one boundary.
    pub fn commit_shipout(
        &mut self,
        boundary: ShipoutBoundary,
        artifact_bytes: &[u8],
        effect_pos: EffectPos,
    ) -> Result<ContentHash, WorldError> {
        assert_eq!(
            boundary.owner,
            self.owner.snapshot_owner(),
            "shipout boundary belongs to a different Universe instance"
        );

        let hash_base = self.state_hash_base.clone();
        let hash = self.world.store_artifact(artifact_bytes)?;
        if let Err(err) = self.world.commit_effects(effect_pos) {
            self.state_hash_base = self.retarget_hash_base_after_committed_boundary(hash_base);
            return Err(err);
        }
        self.stores.release_shipout_nodes(boundary.node_mark);
        let _checkpoint = self.checkpoint_after_committed_boundary(hash_base);
        Ok(hash)
    }

    /// Commits an effect prefix and advances the checkpoint after the prefix is dropped.
    pub fn commit_effects(&mut self, effect_pos: EffectPos) -> Result<(), WorldError> {
        let hash_base = self.state_hash_base.clone();
        if let Err(err) = self.world.commit_effects(effect_pos) {
            self.state_hash_base = self.retarget_hash_base_after_committed_boundary(hash_base);
            return Err(err);
        }
        let _checkpoint = self.checkpoint_after_committed_boundary(hash_base);
        Ok(())
    }

    /// Runs `f` while checkpoints are marked hash-only.
    ///
    /// State snapshots taken in this scope remain valid for rollback and
    /// convergence hashing, but their metadata points execution resume back to
    /// the latest resume-valid boundary established before the scope.
    pub fn with_hash_only_checkpoints<T>(&mut self, f: impl FnOnce(&mut Self) -> T) -> T {
        self.hash_only_checkpoint_depth = self
            .hash_only_checkpoint_depth
            .checked_add(1)
            .expect("hash-only checkpoint scope overflow");
        let result = f(self);
        self.hash_only_checkpoint_depth -= 1;
        result
    }

    /// Returns the metadata for the latest checkpoint created on this timeline.
    #[must_use]
    pub const fn last_checkpoint(&self) -> Option<CheckpointMetadata> {
        self.last_checkpoint
    }

    /// Records the current lexer-owned input stack state for the next snapshot.
    pub fn set_input_summary(&mut self, summary: InputSummary) {
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

    pub fn add_hyphenation_exception(&mut self, exception: ExceptionSpec) {
        self.stores.add_hyphenation_exception(exception);
    }

    #[must_use]
    pub fn hyphen_positions(&self, word: &str, left_min: usize, right_min: usize) -> Vec<usize> {
        self.stores.hyphen_positions(word, left_min, right_min)
    }

    #[must_use]
    pub fn hyphenation_exception(&self, word: &str) -> Option<&[usize]> {
        self.stores.hyphenation_exception(word)
    }

    #[must_use]
    pub fn meaning(&self, symbol: Symbol) -> Meaning {
        self.stores.meaning(symbol)
    }

    pub fn set_meaning(&mut self, symbol: Symbol, meaning: Meaning) {
        self.stores.set_meaning(symbol, meaning);
    }

    pub fn intern_relaxed_control_sequence(&mut self, name: &str) -> Symbol {
        self.stores.intern_relaxed_control_sequence(name)
    }

    pub fn set_meaning_global(&mut self, symbol: Symbol, meaning: Meaning) {
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

    pub fn set_macro_meaning(&mut self, symbol: Symbol, macro_meaning: MacroMeaning) {
        self.stores.set_macro_meaning(symbol, macro_meaning);
    }

    pub fn set_macro_meaning_with_provenance(
        &mut self,
        symbol: Symbol,
        macro_meaning: MacroMeaning,
        provenance: MacroDefinitionProvenance,
    ) {
        self.stores
            .set_macro_meaning_with_provenance(symbol, macro_meaning, provenance);
    }

    pub fn set_macro_meaning_global(&mut self, symbol: Symbol, macro_meaning: MacroMeaning) {
        self.stores.set_macro_meaning_global(symbol, macro_meaning);
    }

    pub fn set_macro_meaning_global_with_provenance(
        &mut self,
        symbol: Symbol,
        macro_meaning: MacroMeaning,
        provenance: MacroDefinitionProvenance,
    ) {
        self.stores
            .set_macro_meaning_global_with_provenance(symbol, macro_meaning, provenance);
    }

    #[must_use]
    pub fn macro_meaning(&self, symbol: Symbol) -> Option<MacroMeaning> {
        self.stores.macro_meaning(symbol)
    }

    pub fn intern(&mut self, name: &str) -> Symbol {
        self.stores.intern(name)
    }

    /// Interns an active-character control sequence in its TeX82 namespace.
    pub fn intern_active_character(&mut self, ch: char) -> Symbol {
        self.stores.intern_active_character(ch)
    }

    #[must_use]
    pub fn symbol(&self, name: &str) -> Option<Symbol> {
        self.stores.symbol(name)
    }

    /// Returns the live symbol for an already-interned active character.
    #[must_use]
    pub fn active_character_symbol(&self, ch: char) -> Option<Symbol> {
        self.stores.active_character_symbol(ch)
    }

    #[must_use]
    pub fn resolve(&self, symbol: Symbol) -> &str {
        self.stores.resolve(symbol)
    }

    /// Returns the TeX control-sequence namespace of a live symbol.
    #[must_use]
    pub fn control_sequence_kind(&self, symbol: Symbol) -> ControlSequenceKind {
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

    /// Allocates a macro-invocation origin.
    pub fn macro_invocation_origin(
        &mut self,
        definition: MacroDefinitionId,
        invocation: OriginId,
        definition_origin: OriginId,
    ) -> OriginId {
        self.stores
            .macro_invocation_origin(definition, invocation, definition_origin)
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
            let scalar_len = std::str::from_utf8(bytes.get(offset..)?)
                .ok()?
                .chars()
                .next()?
                .len_utf8();
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
                .input_records()
                .get(input_record.raw() as usize)
                .ok_or(SourceMapError::MissingWorldInput)?;
            if u64::try_from(record.len()).ok() != Some(byte_len) {
                return Err(SourceMapError::WorldInputLengthMismatch);
            }
            return self
                .stores
                .register_source(source, SourceDescriptor::world(input_record, byte_len));
        }
        self.stores.register_source(source, descriptor)
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

    pub(crate) fn source_backing_bytes(&self, region: SourceRegion) -> Option<&[u8]> {
        match region.backing {
            SourceBacking::World(record_id) => {
                let record = self.world.input_records().get(record_id.raw() as usize)?;
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

    pub fn intern_font_with_identifier(&mut self, font: LoadedFont, symbol: Symbol) -> FontId {
        self.stores.intern_font_with_identifier(font, symbol)
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
    pub fn font_identifier_symbol(&self, id: FontId) -> Option<Symbol> {
        self.stores.font_identifier_symbol(id)
    }

    pub fn set_font_identifier_symbol(&mut self, id: FontId, symbol: Symbol) {
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
    pub fn font_parameter(&self, font: FontId, number: u16) -> Scaled {
        self.stores.font_parameter(font, number)
    }

    #[must_use]
    pub fn current_font(&self) -> FontId {
        self.stores.current_font()
    }

    #[must_use]
    pub fn current_font_symbol(&self) -> Option<Symbol> {
        self.stores.current_font_symbol()
    }

    pub fn set_current_font(&mut self, id: FontId) {
        self.stores.set_current_font(id);
    }

    pub fn set_current_font_global(&mut self, id: FontId) {
        self.stores.set_current_font_global(id);
    }

    pub fn set_current_font_selector(&mut self, symbol: Symbol, id: FontId) {
        self.stores.set_current_font_selector(symbol, id);
    }

    pub fn set_current_font_selector_global(&mut self, symbol: Symbol, id: FontId) {
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
    pub fn font_dimen(&self, font: FontId, number: u16) -> Scaled {
        self.stores.font_dimen(font, number)
    }

    #[must_use]
    pub fn font_parameter_count(&self, font: FontId) -> u16 {
        self.stores.font_parameter_count(font)
    }

    pub fn set_font_dimen(
        &mut self,
        font: FontId,
        number: u16,
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

    pub fn finish_node_list(&mut self, builder: &mut NodeListBuilder) -> NodeListId {
        self.stores.finish_node_list(builder)
    }

    #[must_use]
    pub fn nodes(&self, id: NodeListId) -> NodeList<'_> {
        self.stores.nodes(id)
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
    pub fn begin_box_build(&self) -> BoxBuildBoundary {
        BoxBuildBoundary {
            owner: self.owner.snapshot_owner(),
            node_mark: self.stores.shipout_node_mark(),
        }
    }

    /// Completes a box-register assignment and releases its construction nodes.
    pub fn finish_box_assignment(
        &mut self,
        boundary: BoxBuildBoundary,
        index: u16,
        value: Option<NodeListId>,
        global: bool,
    ) {
        self.assert_box_build_owner(&boundary);
        match (global, value) {
            (false, Some(value)) => self.stores.set_box_reg(index, value),
            (true, Some(value)) => self.stores.set_box_reg_global(index, value),
            (false, None) => self.stores.clear_box_reg(index),
            (true, None) => self.stores.clear_box_reg_global(index),
        }
        self.stores.release_shipout_nodes(boundary.node_mark);
    }

    /// Abandons a failed box-register value scan and releases its node suffix.
    pub fn cancel_box_build(&mut self, boundary: BoxBuildBoundary) {
        self.assert_box_build_owner(&boundary);
        self.stores.release_shipout_nodes(boundary.node_mark);
    }

    fn assert_box_build_owner(&self, boundary: &BoxBuildBoundary) {
        assert_eq!(
            boundary.owner,
            self.owner.snapshot_owner(),
            "box-build boundary belongs to a different Universe instance"
        );
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

    pub fn freeze_page_specs(&mut self, contents: PageContents) {
        let vsize = self.dimen_param(DimenParam::V_SIZE);
        let max_depth = self.dimen_param(DimenParam::MAX_DEPTH);
        self.page.freeze_specs(contents, vsize, max_depth);
    }

    pub fn start_new_page(&mut self) {
        self.page.start_new_page();
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
        self.page.push_contribution(node);
    }

    pub fn prepend_page_contribution(&mut self, node: Node) {
        self.page.prepend_contribution(node);
    }

    #[must_use]
    pub fn page_contributions(&self) -> &[Node] {
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
        self.page.prepend_contributions(nodes);
    }

    #[must_use]
    pub fn current_page_nodes(&self) -> &[Node] {
        self.page.current_page()
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

    pub fn take_box_reg(&mut self, index: u16) -> Option<NodeListId> {
        let value = self
            .stores
            .box_reg(index)
            .map(|value| self.clone_node_list_to_epoch(value));
        let _ = self.stores.take_box_reg(index);
        value
    }

    pub fn take_box_reg_same_level(&mut self, index: u16) -> Option<NodeListId> {
        let value = self
            .stores
            .box_reg(index)
            .map(|value| self.clone_node_list_to_epoch(value));
        let _ = self.stores.take_box_reg_same_level(index);
        value
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
        let Some(id) = self.box_reg(index) else {
            return;
        };
        let epoch_id = self.clone_node_list_to_epoch(id);
        let mut nodes = self.nodes(epoch_id).to_vec();
        set_box_dimension_in_nodes(&mut nodes, dimension, value);
        let rewritten = self.freeze_node_list(&nodes);
        self.set_box_reg(index, rewritten);
    }

    pub fn clone_node_list_to_epoch(&mut self, id: NodeListId) -> NodeListId {
        let nodes = self.nodes(id).to_vec();
        let cloned: Vec<_> = nodes
            .into_iter()
            .map(|node| self.clone_node_to_epoch(node))
            .collect();
        self.freeze_node_list(&cloned)
    }

    pub fn clone_node_to_epoch(&mut self, node: Node) -> Node {
        match node {
            Node::HList(mut box_node) => {
                box_node.children = self.clone_node_list_to_epoch(box_node.children);
                Node::HList(box_node)
            }
            Node::VList(mut box_node) => {
                box_node.children = self.clone_node_list_to_epoch(box_node.children);
                Node::VList(box_node)
            }
            Node::Unset(mut unset) => {
                unset.children = self.clone_node_list_to_epoch(unset.children);
                Node::Unset(unset)
            }
            Node::Disc {
                kind,
                pre,
                post,
                replace,
            } => Node::Disc {
                kind,
                pre: self.clone_node_list_to_epoch(pre),
                post: self.clone_node_list_to_epoch(post),
                replace: self.clone_node_list_to_epoch(replace),
            },
            Node::Ins {
                class,
                size,
                split_top_skip,
                split_max_depth,
                floating_penalty,
                content,
            } => Node::Ins {
                class,
                size,
                split_top_skip,
                split_max_depth,
                floating_penalty,
                content: self.clone_node_list_to_epoch(content),
            },
            Node::Adjust(content) => Node::Adjust(self.clone_node_list_to_epoch(content)),
            Node::MathNoad(mut noad) => {
                noad.nucleus = self.clone_math_field_to_epoch(noad.nucleus);
                noad.subscript = self.clone_math_field_to_epoch(noad.subscript);
                noad.superscript = self.clone_math_field_to_epoch(noad.superscript);
                Node::MathNoad(noad)
            }
            Node::FractionNoad(mut fraction) => {
                fraction.numerator = self.clone_node_list_to_epoch(fraction.numerator);
                fraction.denominator = self.clone_node_list_to_epoch(fraction.denominator);
                Node::FractionNoad(fraction)
            }
            Node::MathChoice(mut choice) => {
                choice.display = self.clone_node_list_to_epoch(choice.display);
                choice.text = self.clone_node_list_to_epoch(choice.text);
                choice.script = self.clone_node_list_to_epoch(choice.script);
                choice.script_script = self.clone_node_list_to_epoch(choice.script_script);
                Node::MathChoice(choice)
            }
            Node::MathList(mut list) => {
                list.content = self.clone_node_list_to_epoch(list.content);
                Node::MathList(list)
            }
            Node::Glue {
                spec,
                kind,
                leader: Some(payload),
            } => Node::Glue {
                spec,
                kind,
                leader: Some(match payload {
                    crate::node::LeaderPayload::HList(mut box_node) => {
                        box_node.children = self.clone_node_list_to_epoch(box_node.children);
                        crate::node::LeaderPayload::HList(box_node)
                    }
                    crate::node::LeaderPayload::VList(mut box_node) => {
                        box_node.children = self.clone_node_list_to_epoch(box_node.children);
                        crate::node::LeaderPayload::VList(box_node)
                    }
                    payload => payload,
                }),
            },
            node => node,
        }
    }

    fn clone_math_field_to_epoch(
        &mut self,
        field: crate::math::MathField,
    ) -> crate::math::MathField {
        match field {
            crate::math::MathField::SubBox(list) => {
                crate::math::MathField::SubBox(self.clone_node_list_to_epoch(list))
            }
            crate::math::MathField::SubMlist(list) => {
                crate::math::MathField::SubMlist(self.clone_node_list_to_epoch(list))
            }
            field => field,
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
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        self.stores.testing_state_hash().hash(&mut hasher);
        self.world.testing_state_hash().hash(&mut hasher);
        self.input_summary.hash(&mut hasher);
        self.interaction_mode.hash(&mut hasher);
        let mut page_hasher = StateHasher::new(0x7061_6765_7465_7374);
        self.hash_page_state(&mut page_hasher);
        page_hasher.finish().hash(&mut hasher);
        hasher.finish()
    }

    #[cfg(any(test, feature = "testing", feature = "shadow"))]
    pub fn testing_hash_node_list_content(&self, id: NodeListId, hasher: &mut impl Hasher) {
        self.stores.testing_hash_node_list_content(id, hasher);
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
}

/// A mutable dimension field of a box register's top-level box.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum BoxDimension {
    Width,
    Height,
    Depth,
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

fn set_box_dimension_in_nodes(nodes: &mut [Node], dimension: BoxDimension, value: Scaled) {
    let box_node = match nodes {
        [Node::HList(box_node)] | [Node::VList(box_node)] => box_node,
        _ => return,
    };
    match dimension {
        BoxDimension::Width => box_node.width = value,
        BoxDimension::Height => box_node.height = value,
        BoxDimension::Depth => box_node.depth = value,
    }
}

impl ExpansionState for Universe {
    fn execution_group_depth(&self) -> u32 {
        self.stores.env_group_depth()
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

    fn meaning(&self, symbol: Symbol) -> Meaning {
        Self::meaning(self, symbol)
    }

    fn macro_definition(&self, id: MacroDefinitionId) -> MacroMeaning {
        Self::macro_definition(self, id)
    }

    fn macro_definition_provenance(&self, id: MacroDefinitionId) -> MacroDefinitionProvenance {
        Self::macro_definition_provenance(self, id)
    }

    fn macro_meaning(&self, symbol: Symbol) -> Option<MacroMeaning> {
        Self::macro_meaning(self, symbol)
    }

    fn intern_relaxed_control_sequence(&mut self, name: &str) -> Symbol {
        Self::intern_relaxed_control_sequence(self, name)
    }

    fn intern(&mut self, name: &str) -> Symbol {
        Self::intern(self, name)
    }

    fn intern_active_character(&mut self, ch: char) -> Symbol {
        Self::intern_active_character(self, ch)
    }

    fn symbol(&self, name: &str) -> Option<Symbol> {
        Self::symbol(self, name)
    }

    fn active_character_symbol(&self, ch: char) -> Option<Symbol> {
        Self::active_character_symbol(self, ch)
    }

    fn resolve(&self, symbol: Symbol) -> &str {
        Self::resolve(self, symbol)
    }

    fn control_sequence_kind(&self, symbol: Symbol) -> ControlSequenceKind {
        Self::control_sequence_kind(self, symbol)
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
        Self::font_identifier_symbol(self, id)
    }

    fn font_parameter(&self, font: FontId, number: u16) -> Scaled {
        Self::font_parameter(self, font, number)
    }

    fn font_dimen(&self, font: FontId, number: u16) -> Scaled {
        Self::font_dimen(self, font, number)
    }

    fn font_parameter_count(&self, font: FontId) -> u16 {
        Self::font_parameter_count(self, font)
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
        Self::current_font_symbol(self)
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

    fn box_dimension(&self, index: u16, dimension: BoxDimension) -> Option<Scaled> {
        Self::box_dimension(self, index, dimension)
    }

    fn int_param(&self, param: IntParam) -> i32 {
        Self::int_param(self, param)
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
    ) -> OriginId {
        Self::macro_invocation_origin(self, definition, invocation, definition_origin)
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

    fn meaning(&self, symbol: Symbol) -> Meaning {
        self.universe.meaning(symbol)
    }

    fn macro_definition(&self, id: MacroDefinitionId) -> MacroMeaning {
        self.universe.macro_definition(id)
    }

    fn macro_definition_provenance(&self, id: MacroDefinitionId) -> MacroDefinitionProvenance {
        self.universe.macro_definition_provenance(id)
    }

    fn macro_meaning(&self, symbol: Symbol) -> Option<MacroMeaning> {
        self.universe.macro_meaning(symbol)
    }

    fn intern_relaxed_control_sequence(&mut self, name: &str) -> Symbol {
        self.universe.intern_relaxed_control_sequence(name)
    }

    fn intern(&mut self, name: &str) -> Symbol {
        self.universe.intern(name)
    }

    fn intern_active_character(&mut self, ch: char) -> Symbol {
        self.universe.intern_active_character(ch)
    }

    fn symbol(&self, name: &str) -> Option<Symbol> {
        self.universe.symbol(name)
    }

    fn active_character_symbol(&self, ch: char) -> Option<Symbol> {
        self.universe.active_character_symbol(ch)
    }

    fn resolve(&self, symbol: Symbol) -> &str {
        self.universe.resolve(symbol)
    }

    fn control_sequence_kind(&self, symbol: Symbol) -> ControlSequenceKind {
        self.universe.control_sequence_kind(symbol)
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
        self.universe.font_identifier_symbol(id)
    }

    fn font_parameter(&self, font: FontId, number: u16) -> Scaled {
        self.universe.font_parameter(font, number)
    }

    fn font_dimen(&self, font: FontId, number: u16) -> Scaled {
        self.universe.font_dimen(font, number)
    }

    fn font_parameter_count(&self, font: FontId) -> u16 {
        self.universe.font_parameter_count(font)
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
        self.universe.current_font_symbol()
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

    fn box_dimension(&self, index: u16, dimension: BoxDimension) -> Option<Scaled> {
        self.universe.box_dimension(index, dimension)
    }

    fn int_param(&self, param: IntParam) -> i32 {
        self.universe.int_param(param)
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
    ) -> OriginId {
        self.universe
            .macro_invocation_origin(definition, invocation, definition_origin)
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
                hasher.usize(target.next_line());
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
    let text = format!("{rng:?}");
    hasher.str(&text);
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

fn hash_input_summary_fields(stores: &Stores, summary: &InputSummary, hasher: &mut StateHasher) {
    hasher.u32(summary.next_source_id());
    hasher.bool(summary.unicode_superscript_notation());
    hasher.usize(summary.frames().len());
    for frame in summary.frames() {
        match frame {
            InputFrameSummary::Source {
                source_id,
                input_record,
                source,
            } => {
                hasher.tag(0);
                hasher.u32(source_id.raw());
                hash_input_record_id(*input_record, hasher);
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
            } => {
                hasher.tag(1);
                stores.hash_token_list_semantic(*token_list, hasher);
                hash_token_list_replay_kind(*replay_kind, hasher);
                hasher.usize(*index);
                for slot in 1..=crate::input::MACRO_ARGUMENT_SLOTS as u8 {
                    match macro_arguments.get(slot) {
                        Some(token_list) => {
                            hasher.bool(true);
                            stores.hash_token_list_semantic(token_list, hasher);
                        }
                        None => hasher.bool(false),
                    }
                }
            }
            InputFrameSummary::Condition {
                token: _,
                condition,
            } => {
                hasher.tag(2);
                hash_condition_kind(condition.kind(), hasher);
                hash_condition_limb(condition.limb(), hasher);
                hasher.bool(condition.evaluating());
                hasher.bool(condition.current_limb_taken());
                hasher.bool(condition.any_limb_taken());
                hasher.u32(condition.ifcase_or_count());
                hasher.u32(condition.skip_nesting());
            }
        }
    }
    match summary.last_source_frame() {
        Some(source) => {
            hasher.bool(true);
            hasher.u32(
                summary
                    .last_source_id()
                    .expect("last source frame must retain its source id")
                    .raw(),
            );
            hash_input_record_id(summary.last_source_record(), hasher);
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

fn hash_input_record_id(record: Option<crate::InputRecordId>, hasher: &mut StateHasher) {
    match record {
        Some(record) => {
            hasher.bool(true);
            hasher.u32(record.raw());
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
        Token::Frozen(crate::token::FrozenToken::EndTemplate) => hasher.tag(3),
        Token::Frozen(crate::token::FrozenToken::EndV) => hasher.tag(4),
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

#[cfg(test)]
mod tests;
