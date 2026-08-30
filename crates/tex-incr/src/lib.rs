//! Named-boundary incremental editor sessions.
//!
//! Accepted editor revisions own opaque coarse engine generations. Runtime ids
//! remain branded inside generic admission episodes, while public output and
//! boundary observations stay handle-free.

#![forbid(unsafe_code)]

use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::fmt;
use std::mem::size_of;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
#[cfg(not(target_arch = "wasm32"))]
use std::time::Instant;

use tex_command::{
    CommandProfile, RegisteredSourceKind, SourceFramingPolicy, SourceRegistration,
    SourceRegistrationError,
};
use tex_exec::{
    Cancellation, CanonicalStepFailure, CanonicalStepResult, CanonicalStepRunner, CheckpointSink,
    DetachedEngineCompletion, DetachedFormatDump, DetachedPreparedPage, EngineBoundary,
    EngineCheckpoint, EngineCompletionDemand, MainControl, MainControlStep, OutputLedger,
    ResourceFulfillment, ResourceHost, ResourceNeed, ResourceOutcome, ResourceWorld,
    canonical_font_resource_path,
};
use tex_out::dvi::{DviError, DviStreamWriter};
pub use tex_out::html::RenderedOutputId;
use tex_state::interner::InternerBudget;
use tex_state::{
    ArtifactOrigin, AssignmentScope, CodeTableKind, CommittedArtifact, ContentHash,
    DetachedFormatImage, EditorLayout, EditorLayoutError, EffectRecord, FragmentStore,
    GenerationBrand, JobClock, LayoutGeneration, LayoutResolvedOrigin, Piece,
    ResolvedSourceLocation, Universe, World, WorldError,
};

mod candidate_lease;
mod history;
mod trace;

use candidate_lease::{CandidateLease, CandidateLeaseState};
pub use history::{BoundaryKey, BoundaryRecord};
use history::{HistoryComparison, RevisionEditMap, compare_histories};
pub use trace::{TraceCompositionError, TraceOperation, TraceSummary, TraceValidationError};

const SESSION_INTERNER_NAMES: u32 = 65_536;
const SESSION_INTERNER_SLOTS: u32 = 131_072;
const SESSION_INTERNER_BYTES: u32 = 16 * 1024 * 1024;

/// Creates the caller-owned reachability domain for one incremental session.
#[must_use]
pub fn new_reachability_store() -> tex_state::ReachabilityStore {
    let budget = InternerBudget::new(
        SESSION_INTERNER_NAMES,
        SESSION_INTERNER_SLOTS,
        SESSION_INTERNER_BYTES,
    )
    .expect("the incremental session interner budget is valid");
    tex_state::ReachabilityStore::new(budget)
}

/// Monotonic identity of an immutable editor buffer.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct RevisionId(u64);

impl RevisionId {
    #[must_use]
    pub const fn new(raw: u64) -> Self {
        Self(raw)
    }

    #[must_use]
    pub const fn raw(self) -> u64 {
        self.0
    }
}

/// One replacement against the currently accepted revision.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Edit {
    pub base_revision: RevisionId,
    pub expected_hash: ContentHash,
    pub range: std::ops::Range<usize>,
    pub replacement: String,
}

/// Honest split between restart observations and detached accepted output.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct RetentionMetrics {
    /// Total checkpoint-related retention, including detached boundary evidence.
    pub checkpoint_root_bytes: usize,
    /// Coarse generation owners charged once regardless of restart-root count.
    pub checkpoint_shared_owner_bytes: usize,
    /// Fixed metadata charged once for every restart-capable checkpoint root.
    pub checkpoint_metadata_bytes: usize,
    /// Boundary evidence retained for comparison but incapable of restart.
    pub detached_boundary_bytes: usize,
    pub memo_result_bytes: usize,
    pub diagnostic_bytes: usize,
    pub output_bytes: usize,
    pub protected_overage_bytes: usize,
    /// Immutable bytes owned by the session's frozen pre-job anchor.
    pub job_start_anchor_bytes: usize,
}

/// Capture and materialization costs for the session's frozen JobStart base.
/// Capture happens once for a fresh session; loaded formats reuse their
/// already-validated bytes. Restore is a cold fallback operation and never
/// runs at an ordinary paragraph boundary.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct JobStartAnchorMetrics {
    /// Byte-exact schema-12 image retained by the anchor.
    pub image_bytes: usize,
    /// Fixed session semantics required to interpret the image as JobStart.
    pub session_metadata_bytes: usize,
    pub bytes: usize,
    pub capture_latency: Duration,
    pub restore_count: u64,
    pub restore_latency: Duration,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct JobStartSessionMetadata {
    profile: CommandProfile,
    compatibility: CommandCompatibility,
    job_clock: JobClock,
}

#[derive(Clone, Debug)]
struct FrozenJobStartAnchor {
    image: Arc<[u8]>,
    session: Option<JobStartSessionMetadata>,
    metrics: JobStartAnchorMetrics,
}

impl FrozenJobStartAnchor {
    fn loaded(image: DetachedFormatImage) -> Self {
        let image: Arc<[u8]> = image.into_bytes().into();
        Self {
            metrics: JobStartAnchorMetrics {
                image_bytes: image.len(),
                bytes: image.len(),
                ..JobStartAnchorMetrics::default()
            },
            image,
            session: None,
        }
    }

    fn captured(
        image: DetachedFormatImage,
        session: JobStartSessionMetadata,
        capture_latency: Duration,
    ) -> Self {
        let mut anchor = Self::loaded(image);
        anchor.bind_session(session);
        anchor.metrics.capture_latency = capture_latency;
        anchor
    }

    fn materialize_image(
        &mut self,
        session: JobStartSessionMetadata,
    ) -> Result<DetachedFormatImage, SessionError> {
        match self.session {
            Some(bound) if bound != session => {
                return Err(SessionError::JobStartSessionMismatch);
            }
            Some(_) => {}
            None => self.bind_session(session),
        }
        let started = Timer::start();
        let image = DetachedFormatImage::try_from_bytes(self.image.as_ref().to_vec())
            .map_err(SessionError::Format)?;
        self.metrics.restore_count = self.metrics.restore_count.saturating_add(1);
        self.metrics.restore_latency = self
            .metrics
            .restore_latency
            .saturating_add(started.elapsed());
        Ok(image)
    }

    fn bind_session(&mut self, session: JobStartSessionMetadata) {
        debug_assert!(self.session.is_none());
        self.session = Some(session);
        self.metrics.session_metadata_bytes = size_of::<JobStartSessionMetadata>();
        self.metrics.bytes = self
            .metrics
            .image_bytes
            .saturating_add(self.metrics.session_metadata_bytes);
    }
}

/// Work and reuse observed while accepting a revision.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ReuseMetrics {
    pub execution_path: RevisionExecutionPath,
    pub restart_boundary: Option<BoundaryKey>,
    pub convergence_boundary: Option<BoundaryKey>,
    pub pages_retained_prefix: usize,
    pub pages_reused: usize,
    pub pages_retyped: usize,
    pub reexecuted_bytes: usize,
    pub reexecuted_tokens: usize,
    pub reexecuted_commands: usize,
    pub reexecuted_macro_text_span_tokens: usize,
    pub reexecuted_source_text_span_tokens: usize,
    pub reexecuted_paragraphs: usize,
    pub same_history_attempts: usize,
    pub same_history_hash_mismatches: usize,
    pub trace_nodes_walked: usize,
    pub trace_leaf_hits: usize,
    pub trace_subtree_hits: usize,
    pub trace_retained_bytes: usize,
    pub suffixes_adopted: usize,
    pub same_history_stop: SameHistoryStop,
    pub restart_fork_latency: Duration,
    pub revision_setup_latency: Duration,
    pub executor_latency: Duration,
    pub reexecution_latency: Duration,
    pub output_snapshot_latency: Duration,
    pub trace_validation_latency: Duration,
    pub trace_replay_latency: Duration,
    pub splice_latency: Duration,
    pub substrate_transition_latency: Duration,
    pub acceptance_latency: Duration,
}

/// Accepted token-delivery telemetry for one editor revision.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ExpansionStats {
    pub token_frame_steps: u64,
    pub provenance_resolutions: u64,
    pub character_tokens: u64,
    pub meaning_lookups: u64,
    pub literal_spans: u64,
    pub literal_tokens: u64,
    pub segmentation_cache_hits: u64,
    pub segmentation_cache_misses: u64,
    pub builder_appends: u64,
    pub source_text_span_attempts: u64,
    pub source_text_spans: u64,
    pub source_text_tokens: u64,
    pub meaning_cache_hits: u64,
    pub meaning_cache_misses: u64,
    pub frame_step_nanos: u64,
    pub provenance_nanos: u64,
    pub classification_meaning_nanos: u64,
    pub builder_append_nanos: u64,
    pub frame_step_timer_samples: u64,
    pub provenance_timer_samples: u64,
    pub classification_meaning_timer_samples: u64,
    pub builder_append_timer_samples: u64,
}

impl ExpansionStats {
    #[must_use]
    pub fn character_fraction(self) -> f64 {
        if self.token_frame_steps == 0 {
            0.0
        } else {
            self.character_tokens as f64 / self.token_frame_steps as f64
        }
    }

    #[must_use]
    pub fn mean_literal_run(self) -> f64 {
        if self.literal_spans == 0 {
            0.0
        } else {
            self.literal_tokens as f64 / self.literal_spans as f64
        }
    }

    #[must_use]
    pub fn mean_source_text_run(self) -> f64 {
        if self.source_text_spans == 0 {
            0.0
        } else {
            self.source_text_tokens as f64 / self.source_text_spans as f64
        }
    }

    #[must_use]
    pub const fn attributed_nanos(self) -> u64 {
        self.frame_step_nanos
            .saturating_add(self.provenance_nanos)
            .saturating_add(self.classification_meaning_nanos)
            .saturating_add(self.builder_append_nanos)
    }
}

/// High-level execution path used to produce one accepted revision.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum RevisionExecutionPath {
    #[default]
    Cold,
    FastEdit,
    SlowEdit,
    ExternalInputDelta,
    ForcedJobStartFallback,
}

/// Why identical-history comparison did or did not find convergence.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum SameHistoryStop {
    Matched,
    ScheduleDiverged,
    HashesDiverged,
    NoComparableBoundary,
    #[default]
    NotAttempted,
}

/// Detached result of one accepted editor revision.
///
/// The completion shares one immutable detached owner with the session so a
/// converged revision can splice output without duplicating page payloads.
#[derive(Debug)]
pub struct AcceptedOutput {
    output_id: RenderedOutputId,
    pub revision: RevisionId,
    pub content_hash: ContentHash,
    completion: Arc<DetachedEngineCompletion>,
    format_dump: Option<DetachedFormatDump>,
    pub reuse: ReuseMetrics,
    pub retention: RetentionMetrics,
}

impl AcceptedOutput {
    #[must_use]
    pub const fn output_id(&self) -> RenderedOutputId {
        self.output_id
    }

    #[must_use]
    pub fn completion(&self) -> &DetachedEngineCompletion {
        self.completion.as_ref()
    }

    #[must_use]
    pub fn pages(&self) -> &[DetachedPreparedPage] {
        self.completion.pages()
    }

    #[must_use]
    pub fn pdf(&self) -> Option<&tex_state::DetachedPdfCompletion> {
        self.completion.pdf()
    }

    pub fn into_completion(self) -> DetachedEngineCompletion {
        Arc::try_unwrap(self.completion).unwrap_or_else(|shared| (*shared).clone())
    }

    #[must_use]
    pub const fn format_dump(&self) -> Option<&DetachedFormatDump> {
        self.format_dump.as_ref()
    }

    pub fn into_terminal(self) -> (DetachedEngineCompletion, Option<DetachedFormatDump>) {
        let Self {
            completion,
            format_dump,
            ..
        } = self;
        (
            Arc::try_unwrap(completion).unwrap_or_else(|shared| (*shared).clone()),
            format_dump,
        )
    }

    pub fn dvi_bytes(&self) -> Result<Vec<u8>, DviError> {
        dvi_bytes(&self.completion)
    }
}

/// Borrowed terminal resource-discovery view.
///
/// Construction is possible only after a candidate reaches terminal
/// completion. Every exposed value is owned by the detached completion; the
/// current runtime generation remains private until acceptance or rejection.
#[derive(Clone, Copy, Debug)]
pub struct CompletionResourceDiscovery<'a> {
    output_id: RenderedOutputId,
    revision: RevisionId,
    content_hash: ContentHash,
    completion: &'a DetachedEngineCompletion,
}

impl<'a> CompletionResourceDiscovery<'a> {
    #[must_use]
    pub const fn output_id(self) -> RenderedOutputId {
        self.output_id
    }

    #[must_use]
    pub const fn revision(self) -> RevisionId {
        self.revision
    }

    #[must_use]
    pub const fn content_hash(self) -> ContentHash {
        self.content_hash
    }

    #[must_use]
    pub const fn completion(self) -> &'a DetachedEngineCompletion {
        self.completion
    }

    #[must_use]
    pub const fn pdf(self) -> Option<&'a tex_state::DetachedPdfCompletion> {
        self.completion.pdf()
    }

    pub fn pdf_fonts(self) -> impl Iterator<Item = &'a tex_state::DetachedPdfFontResource> {
        self.pdf().into_iter().flat_map(|pdf| pdf.fonts())
    }

    pub fn pdf_font_operations(
        self,
    ) -> impl Iterator<Item = &'a tex_state::DetachedPdfFontOperation> {
        self.pdf().into_iter().flat_map(|pdf| pdf.font_operations())
    }

    pub fn pdf_raw_object_file_needs(
        self,
    ) -> impl Iterator<Item = &'a tex_state::DetachedPdfRawObjectFileNeed> {
        self.pdf()
            .into_iter()
            .flat_map(|pdf| pdf.raw_object_file_needs())
    }
}

/// One fully executed revision awaiting an atomic session publication.
pub struct RevisionTransaction<'store> {
    session_output_id: RenderedOutputId,
    base_revision: RevisionId,
    base_content_hash: ContentHash,
    revision: RevisionId,
    source: String,
    fragments: FragmentStore,
    layout: EditorLayout,
    content_hash: ContentHash,
    restart_boundary: Option<BoundaryKey>,
    edit_map: Option<RevisionEditMap>,
    convergence_source_boundary: Option<BoundaryKey>,
    completion: DetachedEngineCompletion,
    history: Vec<BoundaryRecord>,
    dependencies: Vec<tex_state::InputDependency>,
    reuse: ReuseMetrics,
    format_dump: Option<DetachedFormatDump>,
    expansion_stats: ExpansionStats,
    generation: tex_exec::RetainedEngineGeneration<'store>,
    runtime_key: Option<tex_exec::RetainedEngineAttachmentKey>,
    checkpoint_retained_bytes: usize,
    checkpoint_shared_owner_bytes: usize,
    checkpoint_metadata_bytes: usize,
    detached_boundary_bytes: usize,
    checkpoint_protected_overage_bytes: usize,
    job_start_anchor: FrozenJobStartAnchor,
    _candidate_lease: CandidateLease,
}

impl RevisionTransaction<'_> {
    #[must_use]
    pub const fn revision(&self) -> RevisionId {
        self.revision
    }

    #[must_use]
    pub const fn content_hash(&self) -> ContentHash {
        self.content_hash
    }

    #[must_use]
    pub fn source(&self) -> &str {
        &self.source
    }

    /// Rejects this completed candidate, dropping its current generation and
    /// returning the session's candidate slot for immediate reuse.
    pub fn reject(mut self) {
        let prepared = prepare_candidate_runtime(self.generation, self.runtime_key.take())
            .expect("a completed candidate retains its terminal command/mode owner");
        prepared.reject();
    }

    #[must_use]
    pub const fn completion(&self) -> &DetachedEngineCompletion {
        &self.completion
    }

    #[must_use]
    pub fn pages(&self) -> &[DetachedPreparedPage] {
        self.completion.pages()
    }

    #[must_use]
    pub const fn reuse(&self) -> ReuseMetrics {
        self.reuse
    }

    pub fn dvi_bytes(&self) -> Result<Vec<u8>, DviError> {
        dvi_bytes(&self.completion)
    }

    /// Demand-free page ownership counters on the exact current generation.
    /// Profiling gates sample this before production settlement; it does not
    /// traverse page payload or create a detached projection.
    #[doc(hidden)]
    pub fn page_material_counters(
        &mut self,
    ) -> Result<tex_state::fork_arena::ForkArenaCounters, SessionError> {
        self.generation
            .with_admitted(ReadPageMaterialCounters)
            .map_err(SessionError::RetainedEngine)
    }

    /// Demand-free PageBuilder publication and candidate settlement work.
    #[doc(hidden)]
    pub fn page_candidate_settlement_counters(
        &mut self,
    ) -> Result<tex_state::PageCandidateSettlementCounters, SessionError> {
        self.generation
            .with_admitted(ReadPageCandidateSettlementCounters)
            .map_err(SessionError::RetainedEngine)
    }

    /// Demand-free page-region lifecycle counters on the candidate.
    #[doc(hidden)]
    pub fn page_region_counters(&mut self) -> Result<tex_state::PageRegionCounters, SessionError> {
        self.generation
            .with_admitted(ReadPageRegionCounters)
            .map_err(SessionError::RetainedEngine)
    }

    /// Demand-free command ownership counters on the exact current owner.
    #[doc(hidden)]
    pub fn command_timeline_counters(
        &mut self,
    ) -> Result<tex_command::CommandTimelineCounters, SessionError> {
        self.generation
            .with_admitted(ReadCommandTimelineCounters)
            .map_err(SessionError::RetainedEngine)?
            .map_err(SessionError::RetainedEngine)
    }
}

struct CandidatePlan {
    base_revision: RevisionId,
    base_content_hash: ContentHash,
    revision: RevisionId,
    source: String,
    fragments: FragmentStore,
    layout: EditorLayout,
    execution_path: RevisionExecutionPath,
    restart_limit: Option<usize>,
    edit_map: Option<RevisionEditMap>,
    restart_boundary: Option<BoundaryKey>,
    restart_fork_latency: Duration,
    revision_setup_latency: Duration,
}

struct InheritedBoundary {
    key: tex_exec::RetainedCheckpointKey,
    evidence: tex_exec::RetainedBoundaryEvidence,
    retention: tex_exec::CheckpointRetention,
}

struct CandidateCompletion {
    completion: DetachedEngineCompletion,
    history: Vec<BoundaryRecord>,
    dependencies: Vec<tex_state::InputDependency>,
    delivered_commands: usize,
    format_dump: Option<DetachedFormatDump>,
    checkpoint_retained_bytes: usize,
    checkpoint_shared_owner_bytes: usize,
    checkpoint_metadata_bytes: usize,
    detached_boundary_bytes: usize,
    checkpoint_protected_overage_bytes: usize,
    job_start_anchor: FrozenJobStartAnchor,
}

/// Command spellings and fresh-state conventions layered over a canonical
/// engine profile.
///
/// e-TeX and LaTeX share a dialect, but LaTeX exposes an additional primitive
/// namespace. Keeping that choice beside the profile lets a generation-free
/// session reproduce it for both fresh and format-loaded candidates.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum CommandCompatibility {
    #[default]
    Profile,
    Latex,
}

/// Outer plan and private retained generation for one revision execution.
///
/// No runtime id or owner is exposed through the candidate API.
pub struct RevisionCandidate<'store> {
    session_output_id: RenderedOutputId,
    reachability_store: tex_state::ReachabilityStore,
    reachability_owner: core::marker::PhantomData<&'store tex_state::ReachabilityStore>,
    job_name: String,
    source_path: String,
    plan: CandidatePlan,
    registered_inputs: BTreeMap<PathBuf, Arc<[u8]>>,
    profile: CommandProfile,
    compatibility: CommandCompatibility,
    required_font_layout_policy: Option<tex_fonts::FontLayoutPolicy>,
    initex: bool,
    dvi_output: bool,
    root_framing: SourceFramingPolicy,
    root_framing_name: Option<String>,
    root_source_is_byte_projection: bool,
    job_clock: JobClock,
    completed: Option<CandidateCompletion>,
    cumulative_fuel_limit: u64,
    execution_budgets: tex_exec::ExecutionBudgets,
    checkpoint_budget: usize,
    provenance_demand: tex_state::ProvenanceDemand,
    provenance_budgets: tex_state::ProvenanceBudgets,
    suspension_serial: u64,
    advance_calls: u64,
    cumulative_fuel: u64,
    generation: Option<tex_exec::RetainedEngineGeneration<'store>>,
    inherited_boundary: Option<InheritedBoundary>,
    checkpoint_control_key: Option<tex_exec::RetainedEngineAttachmentKey>,
    runtime_key: Option<tex_exec::RetainedEngineAttachmentKey>,
    candidate_lease: Option<CandidateLease>,
    job_start_anchor: Option<FrozenJobStartAnchor>,
    materialized_job_start: bool,
    comparison_history: Arc<[BoundaryRecord]>,
    comparison_start: Option<usize>,
    accepted_dependencies: Arc<[tex_state::InputDependency]>,
}

/// Result of driving a revision until it suspends or completes.
#[derive(Clone, Debug)]
pub enum RevisionCandidateResult {
    AwaitingResources(ResourceNeed),
    Complete,
}

impl<'store> RevisionCandidate<'store> {
    fn job_start_session_metadata(&self) -> JobStartSessionMetadata {
        JobStartSessionMetadata {
            profile: self.profile,
            compatibility: self.compatibility,
            job_clock: self.job_clock,
        }
    }

    fn new_retained_generation(
        &mut self,
    ) -> Result<tex_exec::RetainedEngineGeneration<'store>, SessionError> {
        let metadata = self.job_start_session_metadata();
        if let Some(anchor) = self.job_start_anchor.as_mut() {
            let image = anchor.materialize_image(metadata)?;
            self.materialized_job_start = true;
            return tex_exec::RetainedEngineGeneration::from_format_owned_with_page_node_identity_demand(
                self.reachability_store.clone(),
                World::memory_with_clock(self.job_clock),
                image,
                true,
            )
            .map_err(SessionError::Format);
        }
        tex_exec::RetainedEngineGeneration::new_owned(
            self.reachability_store.clone(),
            World::memory_with_clock(self.job_clock),
        )
        .map_err(SessionError::Epoch)
    }

    pub fn drive_with_resource_resolvers(
        &mut self,
        host: &mut dyn ResourceHost,
        cancellation: &Cancellation,
    ) -> Result<RevisionCandidateResult, SessionError> {
        if self.completed.is_some() {
            return Ok(RevisionCandidateResult::Complete);
        }
        let mut failed_attempt_fuel = 0;
        let generation = match self.generation.take() {
            Some(generation) => generation,
            None => self.new_retained_generation()?,
        };
        let checkpoint_control_key = self.checkpoint_control_key.take();
        let runtime_key = self.runtime_key.take();
        let mut generation = OwnedCandidateGeneration::new(generation);
        let result = generation
            .generation_mut()
            .with_admitted(CandidateRun {
                candidate: self,
                host,
                cancellation,
                failed_attempt_fuel: &mut failed_attempt_fuel,
                checkpoint_control_key,
                runtime_key,
            })
            .map_err(SessionError::RetainedEngine)?;
        self.runtime_key = result.runtime_key;
        let result = result.execution;
        self.generation = Some(generation.into_generation());
        let result = match result {
            Ok(result) => result,
            Err(error) => {
                self.cumulative_fuel = self.cumulative_fuel.saturating_add(failed_attempt_fuel);
                return Err(error);
            }
        };
        match result {
            PlanExecution::Suspended(need) => {
                self.suspension_serial = self.suspension_serial.saturating_add(1);
                Ok(RevisionCandidateResult::AwaitingResources(need))
            }
            PlanExecution::Complete(completion, fuel) => {
                self.advance_calls = self
                    .advance_calls
                    .saturating_add(completion.delivered_commands as u64);
                self.cumulative_fuel = self.cumulative_fuel.saturating_add(fuel);
                self.completed = Some(*completion);
                Ok(RevisionCandidateResult::Complete)
            }
        }
    }

    #[must_use]
    pub const fn suspension_serial(&self) -> u64 {
        self.suspension_serial
    }

    pub fn set_cumulative_fuel_limit(&mut self, limit: u64) {
        self.cumulative_fuel_limit = limit.max(1);
    }

    pub fn set_execution_budgets(&mut self, budgets: tex_exec::ExecutionBudgets) {
        self.execution_budgets = budgets;
    }

    /// Selects the provenance consumers and their independent retention
    /// budgets for this candidate's fresh or loaded engine job.
    pub fn set_provenance_config(
        &mut self,
        demand: tex_state::ProvenanceDemand,
        budgets: tex_state::ProvenanceBudgets,
    ) {
        self.provenance_demand = demand;
        self.provenance_budgets = budgets;
    }

    #[must_use]
    pub const fn execution_telemetry(&self) -> tex_exec::ExecutionTelemetry {
        tex_exec::ExecutionTelemetry {
            cold_starts: 1,
            advance_calls: self.advance_calls,
            suspensions: self.suspension_serial,
            local_step_retries: self.suspension_serial,
            replayed_delivered_tokens: 0,
            replayed_dispatches: 0,
            cumulative_fuel: self.cumulative_fuel,
            engine_time: Duration::ZERO,
            savepoint_capture_time: Duration::ZERO,
            savepoint_restore_time: Duration::ZERO,
        }
    }

    #[must_use]
    pub fn retention_metrics(&self) -> RetentionMetrics {
        let diagnostic_bytes = self
            .plan
            .fragments
            .retained_bytes()
            .saturating_add(self.plan.layout.retained_bytes());
        let output_bytes = self.completed.as_ref().map_or(0, |completion| {
            detached_output_bytes(&completion.completion)
        });
        RetentionMetrics {
            checkpoint_root_bytes: self
                .completed
                .as_ref()
                .map_or(0, |completion| completion.checkpoint_retained_bytes),
            checkpoint_shared_owner_bytes: self
                .completed
                .as_ref()
                .map_or(0, |completion| completion.checkpoint_shared_owner_bytes),
            checkpoint_metadata_bytes: self
                .completed
                .as_ref()
                .map_or(0, |completion| completion.checkpoint_metadata_bytes),
            detached_boundary_bytes: self
                .completed
                .as_ref()
                .map_or(0, |completion| completion.detached_boundary_bytes),
            memo_result_bytes: 0,
            diagnostic_bytes,
            output_bytes,
            protected_overage_bytes: self.completed.as_ref().map_or(0, |completion| {
                completion.checkpoint_protected_overage_bytes
            }),
            job_start_anchor_bytes: self
                .job_start_anchor
                .as_ref()
                .map_or(0, |anchor| anchor.metrics.bytes),
        }
    }

    #[must_use]
    pub fn resolve_frozen_diagnostic_primary(
        &self,
        frozen: &tex_exec::FrozenDiagnosticOrigin,
    ) -> Option<ResolvedSourceLocation> {
        resolve_frozen(frozen, &self.plan.source, &self.plan.layout)
    }

    /// Returns the terminal, handle-free resource projection after completion.
    /// Suspended and not-yet-driven candidates expose no projection.
    #[must_use]
    pub fn completion_resource_discovery(&self) -> Option<CompletionResourceDiscovery<'_>> {
        self.completed
            .as_ref()
            .map(|completed| CompletionResourceDiscovery {
                output_id: self.session_output_id,
                revision: self.plan.revision,
                content_hash: ContentHash::from_bytes(self.plan.source.as_bytes()),
                completion: &completed.completion,
            })
    }

    /// Rejects this candidate, dropping any current generation and returning
    /// the session's candidate slot for immediate reuse.
    pub fn reject(mut self) {
        if let Some(generation) = self.generation.take() {
            let prepared = prepare_candidate_runtime(generation, self.runtime_key.take())
                .expect("a live candidate retains its command/mode owner");
            prepared.reject();
        }
    }
}

enum PlanExecution {
    Suspended(ResourceNeed),
    Complete(Box<CandidateCompletion>, u64),
}

struct CandidateRun<'a, 'store> {
    candidate: &'a mut RevisionCandidate<'store>,
    host: &'a mut dyn ResourceHost,
    cancellation: &'a Cancellation,
    failed_attempt_fuel: &'a mut u64,
    checkpoint_control_key: Option<tex_exec::RetainedEngineAttachmentKey>,
    runtime_key: Option<tex_exec::RetainedEngineAttachmentKey>,
}

#[cfg(test)]
thread_local! {
    static PANIC_AFTER_CANDIDATE_OWNERS_DETACH: std::cell::Cell<bool> = const {
        std::cell::Cell::new(false)
    };
}

#[cfg(test)]
fn arm_candidate_owner_unwind_for_test() {
    PANIC_AFTER_CANDIDATE_OWNERS_DETACH.with(|armed| armed.set(true));
}

impl tex_exec::RetainedEngineOperation for CandidateRun<'_, '_> {
    type Output = CandidateRunResult;

    fn run<G: 'static>(
        self,
        mut admitted: tex_exec::AdmittedEngineGeneration<'_, G>,
    ) -> Self::Output {
        let mut attached = match self.runtime_key {
            Some(key) => {
                match admitted.prepare_attached_checkpoint_control::<CandidateRuntime>(key) {
                    Ok(attached) => attached,
                    Err(error) => {
                        return CandidateRunResult {
                            execution: Err(SessionError::RetainedEngine(error)),
                            runtime_key: None,
                        };
                    }
                }
            }
            None => match self.checkpoint_control_key {
                Some(key) => {
                    let mut attached = match admitted
                        .prepare_attached_checkpoint_control::<tex_exec::RestoredCheckpointRuntime>(
                            key,
                        ) {
                        Ok(attached) => attached,
                        Err(error) => {
                            return CandidateRunResult {
                                execution: Err(SessionError::RetainedEngine(error)),
                                runtime_key: None,
                            };
                        }
                    };
                    let initialized = {
                        let (universe, ledger, checkpoints, control) =
                            attached.initialization_parts();
                        initialize_candidate_runtime(
                            universe,
                            ledger,
                            checkpoints,
                            self.candidate,
                            control,
                        )
                    };
                    let runtime = match initialized {
                        Ok(runtime) => runtime,
                        Err(error) => {
                            return CandidateRunResult {
                                execution: Err(error),
                                runtime_key: None,
                            };
                        }
                    };
                    attached.replace_attachment(runtime);
                    attached
                }
                None => {
                    let mut control = None;
                    let initialized = {
                        let (universe, ledger, checkpoints) = admitted.parts();
                        initialize_candidate_runtime(
                            universe,
                            ledger,
                            checkpoints,
                            self.candidate,
                            &mut control,
                        )
                    };
                    let runtime = match initialized {
                        Ok(runtime) => runtime,
                        Err(error) => {
                            return CandidateRunResult {
                                execution: Err(error),
                                runtime_key: None,
                            };
                        }
                    };
                    let control = control.expect("candidate initialization constructs control");
                    let key = match admitted.attach_with_checkpoint_control(runtime, control) {
                        Ok(key) => key,
                        Err(error) => {
                            return CandidateRunResult {
                                execution: Err(SessionError::RetainedEngine(error)),
                                runtime_key: None,
                            };
                        }
                    };
                    match admitted.prepare_attached_checkpoint_control::<CandidateRuntime>(key) {
                        Ok(attached) => attached,
                        Err(error) => {
                            return CandidateRunResult {
                                execution: Err(SessionError::RetainedEngine(error)),
                                runtime_key: None,
                            };
                        }
                    }
                }
            },
        };
        #[cfg(test)]
        PANIC_AFTER_CANDIDATE_OWNERS_DETACH.with(|armed| {
            if armed.replace(false) {
                panic!("candidate owner unwind test hook");
            }
        });
        let execution = {
            let (universe, ledger, checkpoints, control, runtime) =
                attached.parts::<CandidateRuntime>();
            execute_plan(
                universe,
                ledger,
                checkpoints,
                control,
                runtime,
                self.candidate,
                self.host,
                self.cancellation,
                self.failed_attempt_fuel,
            )
        };
        // Terminal command and mode owners remain attached until the
        // aggregate accept/reject barrier explicitly settles them. Drop is
        // reserved for unwinding before an attachment can be published.
        let runtime_key = Some(attached.park());
        CandidateRunResult {
            execution,
            runtime_key,
        }
    }
}

struct CandidateRunResult {
    execution: Result<PlanExecution, SessionError>,
    runtime_key: Option<tex_exec::RetainedEngineAttachmentKey>,
}

struct SettleCandidateRuntime {
    key: tex_exec::RetainedEngineAttachmentKey,
    independent_job_start: bool,
}

struct ReadPageMaterialCounters;

impl tex_exec::RetainedEngineOperation for ReadPageMaterialCounters {
    type Output = tex_state::fork_arena::ForkArenaCounters;

    fn run<G: 'static>(
        self,
        mut admitted: tex_exec::AdmittedEngineGeneration<'_, G>,
    ) -> Self::Output {
        admitted.universe().page_material_counters()
    }
}

struct ReadPageCandidateSettlementCounters;

impl tex_exec::RetainedEngineOperation for ReadPageCandidateSettlementCounters {
    type Output = tex_state::PageCandidateSettlementCounters;

    fn run<G: 'static>(
        self,
        mut admitted: tex_exec::AdmittedEngineGeneration<'_, G>,
    ) -> Self::Output {
        admitted.universe().page_candidate_settlement_counters()
    }
}

struct ReadPageRegionCounters;

impl tex_exec::RetainedEngineOperation for ReadPageRegionCounters {
    type Output = tex_state::PageRegionCounters;

    fn run<G: 'static>(
        self,
        mut admitted: tex_exec::AdmittedEngineGeneration<'_, G>,
    ) -> Self::Output {
        admitted.universe().page_region_counters()
    }
}

struct ReadCommandTimelineCounters;

impl tex_exec::RetainedEngineOperation for ReadCommandTimelineCounters {
    type Output = Result<tex_command::CommandTimelineCounters, tex_exec::RetainedEngineAccessError>;

    fn run<G: 'static>(self, admitted: tex_exec::AdmittedEngineGeneration<'_, G>) -> Self::Output {
        admitted.command_timeline_counters()
    }
}

struct PreparedCandidateControl {
    control: Option<tex_exec::PreparedCheckpointControl>,
}

struct OwnedCandidateGeneration<'store> {
    generation: Option<tex_exec::RetainedEngineGeneration<'store>>,
}

impl<'store> OwnedCandidateGeneration<'store> {
    fn new(generation: tex_exec::RetainedEngineGeneration<'store>) -> Self {
        Self {
            generation: Some(generation),
        }
    }

    fn generation_mut(&mut self) -> &mut tex_exec::RetainedEngineGeneration<'store> {
        self.generation
            .as_mut()
            .expect("the aggregate candidate guard owns its generation")
    }

    fn into_generation(mut self) -> tex_exec::RetainedEngineGeneration<'store> {
        self.generation
            .take()
            .expect("the aggregate candidate guard owns its generation")
    }

    fn reject(&mut self) {
        let Some(mut generation) = self.generation.take() else {
            return;
        };
        generation.prepare_candidate_reject();
        generation.finish_candidate_reject();
    }
}

impl Drop for OwnedCandidateGeneration<'_> {
    fn drop(&mut self) {
        self.reject();
    }
}

struct PreparedCandidateRuntime<'store> {
    generation: OwnedCandidateGeneration<'store>,
    control: Option<tex_exec::PreparedCheckpointControl>,
}

impl<'store> PreparedCandidateRuntime<'store> {
    fn accept_control(&mut self) {
        if let Some(control) = self.control.take() {
            control.accept();
        }
    }

    fn generation_mut(&mut self) -> &mut tex_exec::RetainedEngineGeneration<'store> {
        self.generation.generation_mut()
    }

    fn into_generation(mut self) -> tex_exec::RetainedEngineGeneration<'store> {
        assert!(
            self.control.is_none(),
            "mode disposition precedes aggregate candidate publication"
        );
        self.generation
            .generation
            .take()
            .expect("the prepared aggregate owns its generation")
    }

    fn reject(self) {}
}

impl Drop for PreparedCandidateRuntime<'_> {
    fn drop(&mut self) {
        if let Some(control) = self.control.take() {
            control.reject();
        }
        // `OwnedCandidateGeneration` drops immediately after this method and
        // returns command, boundary, ledger, state, page, and PDF owners.
    }
}

impl tex_exec::RetainedEngineOperation for SettleCandidateRuntime {
    type Output = Result<PreparedCandidateControl, tex_exec::RetainedEngineAccessError>;

    fn run<G: 'static>(
        self,
        mut admitted: tex_exec::AdmittedEngineGeneration<'_, G>,
    ) -> Self::Output {
        let attached =
            admitted.prepare_attached_checkpoint_control::<CandidateRuntime>(self.key)?;
        if self.independent_job_start {
            attached.park_independent_settlement()?;
            return Ok(PreparedCandidateControl { control: None });
        }
        Ok(PreparedCandidateControl {
            control: Some(attached.prepare_live_settlement()?),
        })
    }
}

fn prepare_candidate_runtime<'store>(
    generation: tex_exec::RetainedEngineGeneration<'store>,
    key: Option<tex_exec::RetainedEngineAttachmentKey>,
) -> Result<PreparedCandidateRuntime<'store>, tex_exec::RetainedEngineAccessError> {
    let mut generation = OwnedCandidateGeneration::new(generation);
    let Some(key) = key else {
        return Ok(PreparedCandidateRuntime {
            generation,
            control: None,
        });
    };
    let independent_job_start = generation
        .generation_mut()
        .is_independent_job_start_candidate();
    let prepared = generation
        .generation_mut()
        .with_admitted(SettleCandidateRuntime {
            key,
            independent_job_start,
        })??;
    Ok(PreparedCandidateRuntime {
        generation,
        control: prepared.control,
    })
}

struct CandidateRuntime {
    history: LiveHistoryState,
    delivered_commands: usize,
    answered_needs: Vec<ResourceNeed>,
    job_start_anchor: Option<FrozenJobStartAnchor>,
}

struct LiveConvergence {
    accepted: Arc<[BoundaryRecord]>,
    next_old: usize,
    edit: Option<RevisionEditMap>,
    restart: BoundaryKey,
    matched: Option<(BoundaryKey, BoundaryKey)>,
    schedule_diverged: bool,
}

struct LiveHistoryState {
    revision: RevisionId,
    records: Vec<BoundaryRecord>,
    occurrences: HashMap<(usize, EngineBoundary), u32>,
    paragraphs: usize,
    checkpoint_keys: Vec<Option<tex_exec::RetainedCheckpointKey>>,
    checkpoint_retentions: Vec<Option<tex_exec::CheckpointRetention>>,
    checkpoint_budget: usize,
    shared_owner_charges: BTreeMap<
        (
            tex_exec::CheckpointOwnerKey,
            tex_exec::CheckpointOwnerFamily,
        ),
        SharedOwnerRetention,
    >,
    checkpoint_metadata_bytes: usize,
    protected_overage_bytes: usize,
    convergence: Option<LiveConvergence>,
}

#[derive(Clone, Copy)]
struct SharedOwnerRetention {
    bytes: usize,
    restart_roots: usize,
}

impl LiveHistoryState {
    fn new(
        revision: RevisionId,
        checkpoint_budget: usize,
        convergence: Option<LiveConvergence>,
    ) -> Self {
        Self {
            revision,
            records: Vec::new(),
            occurrences: HashMap::new(),
            paragraphs: 0,
            checkpoint_keys: Vec::new(),
            checkpoint_retentions: Vec::new(),
            checkpoint_budget,
            shared_owner_charges: BTreeMap::new(),
            checkpoint_metadata_bytes: 0,
            protected_overage_bytes: 0,
            convergence,
        }
    }

    fn observe_convergence(&mut self, record: &BoundaryRecord) {
        let Some(comparison) = self.convergence.as_mut() else {
            return;
        };
        if comparison.matched.is_some() || comparison.schedule_diverged {
            return;
        }
        // A restored rooted candidate carries the selected boundary as its
        // inherited first row. A materialized JobStart publishes the same
        // anchor before consuming input. Neither is newly executed work.
        if self.records.is_empty() && record.key == comparison.restart {
            return;
        }
        let Some(old) = comparison.accepted.get(comparison.next_old) else {
            comparison.schedule_diverged = true;
            return;
        };
        let mapped_position = comparison
            .edit
            .and_then(|edit| edit.map_position(old.key.position))
            .or_else(|| comparison.edit.is_none().then_some(old.key.position));
        let Some(mapped_position) = mapped_position else {
            comparison.schedule_diverged = true;
            return;
        };
        let mapped = BoundaryKey {
            position: mapped_position,
            boundary: old.key.boundary,
            ordinal: old.key.ordinal,
        };
        if mapped != record.key {
            comparison.schedule_diverged = true;
            return;
        }
        comparison.next_old = comparison.next_old.saturating_add(1);
        if old.reachable_state_identity.is_some()
            && old.reachable_state_identity == record.reachable_state_identity
        {
            comparison.matched = Some((old.key, record.key));
        }
    }

    fn convergence_match(&self) -> Option<(BoundaryKey, BoundaryKey)> {
        self.convergence
            .as_ref()
            .and_then(|comparison| comparison.matched)
    }

    fn inherit_boundary(&mut self, inherited: InheritedBoundary) {
        let evidence = inherited.evidence;
        let boundary = evidence.boundary();
        let position = evidence.position();
        let ordinal = evidence.ordinal();
        debug_assert!(self.records.is_empty());
        self.records.push(BoundaryRecord {
            revision: self.revision,
            key: BoundaryKey {
                position,
                boundary,
                ordinal,
            },
            effect_prefix: evidence.effect_prefix(),
            artifact_prefix: evidence.artifact_prefix(),
            reachable_state_identity: evidence.reachable_state_identity(),
        });
        self.occurrences
            .insert((position, boundary), ordinal.saturating_add(1));
        self.observe_shared_owners(inherited.retention);
        self.checkpoint_metadata_bytes = self
            .checkpoint_metadata_bytes
            .max(inherited.retention.checkpoint_metadata_bytes());
        self.checkpoint_keys.push(Some(inherited.key));
        self.checkpoint_retentions.push(Some(inherited.retention));
    }

    fn retained_restart_root_count(&self) -> usize {
        self.checkpoint_keys.iter().flatten().count()
    }

    fn restart_metadata_bytes(&self) -> usize {
        self.retained_restart_root_count()
            .saturating_mul(self.checkpoint_metadata_bytes)
    }

    fn detached_boundary_bytes(&self) -> usize {
        self.records
            .len()
            .saturating_mul(size_of::<BoundaryRecord>())
    }

    fn retained_bytes(&self) -> usize {
        self.shared_owner_bytes()
            .saturating_add(self.restart_metadata_bytes())
            .saturating_add(self.detached_boundary_bytes())
    }

    fn shared_owner_bytes(&self) -> usize {
        self.shared_owner_charges
            .values()
            .map(|charge| charge.bytes)
            .fold(0_usize, usize::saturating_add)
    }

    fn observe_shared_owners(&mut self, retention: tex_exec::CheckpointRetention) {
        for charge in retention.shared_owners() {
            self.shared_owner_charges
                .entry((charge.owner(), charge.family()))
                .and_modify(|retained| {
                    retained.bytes = retained.bytes.max(charge.bytes());
                    retained.restart_roots = retained.restart_roots.saturating_add(1);
                })
                .or_insert(SharedOwnerRetention {
                    bytes: charge.bytes(),
                    restart_roots: 1,
                });
        }
    }

    fn release_shared_owners(&mut self, retention: tex_exec::CheckpointRetention) {
        for charge in retention.shared_owners() {
            let key = (charge.owner(), charge.family());
            let remove = {
                let retained = self
                    .shared_owner_charges
                    .get_mut(&key)
                    .expect("released checkpoint owner was observed at publication");
                retained.restart_roots = retained
                    .restart_roots
                    .checked_sub(1)
                    .expect("checkpoint owner reference count underflow");
                retained.restart_roots == 0
            };
            if remove {
                self.shared_owner_charges.remove(&key);
            }
        }
    }

    fn restart_victim(&self) -> Option<usize> {
        self.records
            .iter()
            .enumerate()
            .find(|(index, record)| {
                *index != 0
                    && self.checkpoint_keys[*index].is_some()
                    && record.key.boundary == EngineBoundary::OuterParagraphEnd
            })
            .or_else(|| {
                self.checkpoint_keys
                    .iter()
                    .enumerate()
                    .find(|(index, key)| *index != 0 && key.is_some())
                    .map(|(index, _)| (index, &self.records[index]))
            })
            .map(|(index, _)| index)
    }

    fn evidence_victim(&self) -> Option<usize> {
        if self.records.len() <= 2 {
            return None;
        }
        let newest = self.records.len() - 1;
        self.records
            .iter()
            .enumerate()
            .find(|(index, record)| {
                *index != 0
                    && *index != newest
                    && record.key.boundary == EngineBoundary::OuterParagraphEnd
            })
            .or_else(|| {
                self.records
                    .iter()
                    .enumerate()
                    .find(|(index, _)| *index != 0 && *index != newest)
            })
            .map(|(index, _)| index)
    }

    fn take_budget_release<G>(
        &mut self,
        retained: &mut tex_exec::RetainedCheckpointStore<'_, G>,
    ) -> Option<tex_exec::EngineCheckpointRelease<G>> {
        while self.retained_bytes() > self.checkpoint_budget {
            if let Some(victim) = self.restart_victim() {
                let key = self.checkpoint_keys[victim]
                    .take()
                    .expect("restart victim owns a retained key");
                let release = retained
                    .release(key)
                    .expect("publication owns the retained checkpoint key");
                let retention = self.checkpoint_retentions[victim]
                    .take()
                    .expect("restart victim owns a retention descriptor");
                self.release_shared_owners(retention);
                self.protected_overage_bytes =
                    self.retained_bytes().saturating_sub(self.checkpoint_budget);
                return Some(release);
            }
            let Some(victim) = self.evidence_victim() else {
                break;
            };
            debug_assert!(self.checkpoint_keys[victim].is_none());
            self.records.remove(victim);
            self.checkpoint_keys.remove(victim);
            let retention = self.checkpoint_retentions.remove(victim);
            debug_assert!(retention.is_none());
        }
        self.protected_overage_bytes = self.retained_bytes().saturating_sub(self.checkpoint_budget);
        None
    }
}

struct LiveHistorySink<'state, 'generation, G> {
    state: &'state mut LiveHistoryState,
    retained: tex_exec::RetainedCheckpointStore<'generation, G>,
    pending_release: Option<tex_exec::EngineCheckpointRelease<G>>,
}

impl<G> CheckpointSink<G> for LiveHistorySink<'_, '_, G> {
    fn wants_reachable_state_identity(&self, _boundary: EngineBoundary) -> bool {
        true
    }

    fn checkpoint(&mut self, checkpoint: EngineCheckpoint<G>) {
        let position = checkpoint.root_anchor();
        let boundary = checkpoint.boundary();
        if boundary == EngineBoundary::OuterParagraphEnd {
            self.state.paragraphs = self.state.paragraphs.saturating_add(1);
        }
        let ordinal = *self
            .state
            .occurrences
            .entry((position, boundary))
            .or_default();
        self.state
            .occurrences
            .insert((position, boundary), ordinal.saturating_add(1));
        let record = BoundaryRecord {
            revision: self.state.revision,
            key: BoundaryKey {
                position,
                boundary,
                ordinal,
            },
            effect_prefix: checkpoint.effect_prefix_len(),
            artifact_prefix: checkpoint.artifact_prefix_len(),
            reachable_state_identity: checkpoint.reachable_state_identity(),
        };
        self.state.observe_convergence(&record);
        self.state.records.push(record);
        let retention = checkpoint.retention();
        let evidence = tex_exec::RetainedBoundaryEvidence::new(
            self.state.revision.raw(),
            position,
            boundary,
            ordinal,
            checkpoint.effect_prefix_len(),
            checkpoint.artifact_prefix_len(),
            checkpoint.reachable_state_identity(),
        );
        if boundary == EngineBoundary::JobStart {
            // The session's detached frozen anchor is the authoritative
            // restart owner. Keep schedule evidence, but never publish this
            // initial cursor as a live journal root.
            self.state.checkpoint_keys.push(None);
            self.state.checkpoint_retentions.push(None);
            self.retained.retain_evidence(evidence);
            self.pending_release = Some(checkpoint.release_unretained());
            return;
        }
        self.state.observe_shared_owners(retention);
        self.state.checkpoint_metadata_bytes = self
            .state
            .checkpoint_metadata_bytes
            .max(retention.checkpoint_metadata_bytes());
        self.state
            .checkpoint_keys
            .push(Some(self.retained.retain_boundary(checkpoint, evidence)));
        self.state.checkpoint_retentions.push(Some(retention));
    }

    fn take_checkpoint_release(&mut self) -> Option<tex_exec::EngineCheckpointRelease<G>> {
        self.pending_release
            .take()
            .or_else(|| self.state.take_budget_release(&mut self.retained))
    }

    fn stop_requested(&self) -> bool {
        self.state.convergence_match().is_some()
    }
}

fn initialize_candidate_runtime<G: 'static>(
    universe: &mut Universe<G>,
    ledger: &mut OutputLedger,
    mut checkpoints: tex_exec::RetainedCheckpointStore<'_, G>,
    candidate: &mut RevisionCandidate<'_>,
    control: &mut Option<MainControl<G>>,
) -> Result<CandidateRuntime, SessionError> {
    let rooted_restart = control.is_some()
        && candidate
            .plan
            .restart_boundary
            .is_some_and(|key| key.boundary != EngineBoundary::JobStart);
    let rooted = control.is_some();
    let materialized_job_start = candidate.materialized_job_start;
    universe.set_provenance_config(candidate.provenance_demand, candidate.provenance_budgets);
    if !rooted_restart && let Err(error) = universe.begin_retained_session() {
        return Err(error.into());
    }
    // Identity owners must see every job mutation, including fresh profile,
    // registered input, and JobStart setup. Batch execution never selects
    // this demand path and therefore performs none of the added hash work.
    universe.enable_reachable_state_identity();
    universe.set_interaction_mode(tex_state::InteractionMode::Nonstop);
    if !rooted
        && !materialized_job_start
        && let Err(error) = install_plain_catcodes(universe)
    {
        return Err(error);
    }
    if materialized_job_start {
        register_materialized_primitives(universe, candidate.profile, candidate.compatibility);
        validate_materialized_font_policy(universe, candidate.required_font_layout_policy)?;
    }
    for (path, bytes) in &candidate.registered_inputs {
        if let Err(error) = universe
            .world_mut()
            .set_shared_memory_file(path, Arc::clone(bytes))
        {
            return Err(error.into());
        }
    }
    let options = CandidateControlOptions {
        job_name: &candidate.job_name,
        source_path: &candidate.source_path,
        bytes: Arc::from(source_file_bytes(
            &candidate.plan.source,
            candidate.root_source_is_byte_projection,
        )),
        profile: candidate.profile,
        compatibility: candidate.compatibility,
        initex: candidate.initex,
        emit_dvi: candidate.dvi_output,
        root_framing: candidate.root_framing,
        root_framing_name: candidate.root_framing_name.as_deref(),
    };
    prepare_candidate_control(universe, &options, control, materialized_job_start)?;
    let control = control
        .as_mut()
        .expect("candidate control is installed before runtime initialization");
    if rooted_restart {
        let anchor = candidate
            .plan
            .restart_boundary
            .expect("a rooted restart has an exact selected boundary")
            .position;
        control.rebind_root_source_for_editor(Arc::clone(&options.bytes), anchor)?;
    }
    control
        .set_fuel_limit(candidate.cumulative_fuel_limit)
        .expect("candidate fuel limit is positive");
    control.attach_pure_memo_capability(universe);
    control.enable_reachable_state_identity(universe);
    let convergence = candidate
        .comparison_start
        .zip(candidate.plan.restart_boundary)
        .map(|(next_old, restart)| LiveConvergence {
            accepted: Arc::clone(&candidate.comparison_history),
            next_old,
            edit: candidate.plan.edit_map,
            restart,
            matched: None,
            schedule_diverged: false,
        });
    let mut history = LiveHistoryState::new(
        candidate.plan.revision,
        candidate.checkpoint_budget,
        convergence,
    );
    if let Some(inherited) = candidate.inherited_boundary.take() {
        debug_assert!(rooted);
        history.inherit_boundary(inherited);
        while let Some(release) = history.take_budget_release(&mut checkpoints) {
            release.apply(control, universe);
        }
    }
    if !rooted_restart {
        if !rooted {
            if let Err(error) = ledger.commit_job_start(
                control,
                universe,
                &mut LiveHistorySink {
                    state: &mut history,
                    retained: checkpoints,
                    pending_release: None,
                },
            ) {
                return Err(error.into());
            }
            if candidate.job_start_anchor.is_none() {
                let started = Timer::start();
                let image = match universe.capture_format_image() {
                    Ok(image) => image,
                    Err(error) => {
                        return Err(SessionError::Format(error));
                    }
                };
                candidate.job_start_anchor = Some(FrozenJobStartAnchor::captured(
                    image,
                    candidate.job_start_session_metadata(),
                    started.elapsed(),
                ));
            }
        }
        start_candidate_job(universe, control, options)?;
    }
    Ok(CandidateRuntime {
        history,
        delivered_commands: 0,
        answered_needs: Vec::new(),
        job_start_anchor: candidate.job_start_anchor.clone(),
    })
}

#[allow(clippy::too_many_arguments)] // Candidate execution keeps each mutable subsystem owner explicit.
fn execute_plan<G>(
    universe: &mut Universe<G>,
    ledger: &mut OutputLedger,
    checkpoints: tex_exec::RetainedCheckpointStore<'_, G>,
    control: &mut MainControl<G>,
    runtime: &mut CandidateRuntime,
    candidate: &RevisionCandidate<'_>,
    host: &mut dyn ResourceHost,
    cancellation: &Cancellation,
    failed_attempt_fuel: &mut u64,
) -> Result<PlanExecution, SessionError> {
    let CandidateRuntime {
        history,
        delivered_commands,
        answered_needs,
        job_start_anchor,
    } = runtime;
    let mut sink = LiveHistorySink {
        state: history,
        retained: checkpoints,
        pending_release: None,
    };
    loop {
        if cancellation.is_cancelled() {
            return Err(SessionError::Execute(
                tex_exec::ExecError::ExecutionCancelled,
            ));
        }
        let budget_usage = [
            (
                "steps",
                candidate.execution_budgets.steps,
                u64::try_from(*delivered_commands).unwrap_or(u64::MAX),
            ),
            (
                "live input frames",
                candidate.execution_budgets.input_frames,
                u64::try_from(control.input_level_count()).unwrap_or(u64::MAX),
            ),
            (
                "environment journal bytes",
                candidate.execution_budgets.journal_bytes,
                u64::try_from(
                    universe
                        .state_journal_bytes()
                        .map_err(SessionError::Universe)?,
                )
                .unwrap_or(u64::MAX),
            ),
            (
                "pending effects",
                candidate.execution_budgets.effects,
                u64::try_from(universe.world().effect_records().len()).unwrap_or(u64::MAX),
            ),
        ];
        for (resource, limit, attempted) in budget_usage {
            if attempted > limit {
                return Err(SessionError::Execute(
                    tex_exec::ExecError::ResourceBudgetExceeded {
                        resource,
                        limit,
                        attempted,
                    },
                ));
            }
        }
        // TeX82 §93's `succumb` reaches §81's `jump_out`, but a revision
        // candidate is still transactional: a fatal candidate must not be
        // accepted as a normal detached output. `step` retains the captured
        // source evidence and returns the fatal to the outer host, which may
        // publish its diagnostic effects without publishing the revision.
        let step =
            CanonicalStepRunner::new(control, universe, ledger).step(&mut sink, cancellation);
        *failed_attempt_fuel = control.fuel_burned();
        match step {
            CanonicalStepResult::Progress(step)
            | CanonicalStepResult::Committed(step)
            | CanonicalStepResult::Completed(step) => {
                answered_needs.clear();
                *delivered_commands = delivered_commands.saturating_add(1);
                if u64::try_from(*delivered_commands).unwrap_or(u64::MAX)
                    > candidate.execution_budgets.steps
                {
                    return Err(SessionError::Execute(
                        tex_exec::ExecError::ResourceBudgetExceeded {
                            resource: "steps",
                            limit: candidate.execution_budgets.steps,
                            attempted: u64::try_from(*delivered_commands).unwrap_or(u64::MAX),
                        },
                    ));
                }
                if sink.stop_requested()
                    && !matches!(step, MainControlStep::End | MainControlStep::EndOfInput)
                {
                    let dependencies = candidate
                        .accepted_dependencies
                        .iter()
                        .cloned()
                        .chain(universe.world().input_dependencies())
                        .collect::<std::collections::BTreeSet<_>>()
                        .into_iter()
                        .collect();
                    let completion = ledger.detach_checkpoint_prefix(control, universe)?;
                    let checkpoint_retained_bytes = sink.state.retained_bytes();
                    let checkpoint_shared_owner_bytes = sink.state.shared_owner_bytes();
                    let checkpoint_metadata_bytes = sink.state.restart_metadata_bytes();
                    let detached_boundary_bytes = sink.state.detached_boundary_bytes();
                    let checkpoint_protected_overage_bytes = sink.state.protected_overage_bytes;
                    return Ok(PlanExecution::Complete(
                        Box::new(CandidateCompletion {
                            completion,
                            history: std::mem::take(&mut sink.state.records),
                            dependencies,
                            delivered_commands: *delivered_commands,
                            format_dump: None,
                            checkpoint_retained_bytes,
                            checkpoint_shared_owner_bytes,
                            checkpoint_metadata_bytes,
                            detached_boundary_bytes,
                            checkpoint_protected_overage_bytes,
                            job_start_anchor: job_start_anchor
                                .take()
                                .expect("every executing candidate owns a frozen JobStart anchor"),
                        }),
                        control.fuel_burned(),
                    ));
                }
                if matches!(step, MainControlStep::End | MainControlStep::EndOfInput) {
                    let dependencies = universe.world().input_dependencies().collect();
                    let format_dump = control
                        .take_format_dump(universe)
                        .map_err(SessionError::FormatDump)?;
                    CanonicalStepRunner::new(control, universe, ledger)
                        .publish_terminal_boundary_suffix(&mut sink)
                        .map_err(map_step_failure)?;
                    let terminal = ledger.terminal_receipt(control, step).map_err(|error| {
                        map_terminal_completion_error(
                            error,
                            control.fuel_burned(),
                            control.fuel_limit(),
                        )
                    })?;
                    let completion = ledger.close_revision(
                        control,
                        universe,
                        &terminal,
                        EngineCompletionDemand::new(
                            candidate.profile.dialect() == tex_command::CommandDialect::Pdftex14029,
                        ),
                        0,
                    )?;
                    let checkpoint_retained_bytes = sink.state.retained_bytes();
                    let checkpoint_shared_owner_bytes = sink.state.shared_owner_bytes();
                    let checkpoint_metadata_bytes = sink.state.restart_metadata_bytes();
                    let detached_boundary_bytes = sink.state.detached_boundary_bytes();
                    let checkpoint_protected_overage_bytes = sink.state.protected_overage_bytes;
                    return Ok(PlanExecution::Complete(
                        Box::new(CandidateCompletion {
                            completion,
                            history: std::mem::take(&mut sink.state.records),
                            dependencies,
                            delivered_commands: *delivered_commands,
                            format_dump,
                            checkpoint_retained_bytes,
                            checkpoint_shared_owner_bytes,
                            checkpoint_metadata_bytes,
                            detached_boundary_bytes,
                            checkpoint_protected_overage_bytes,
                            job_start_anchor: job_start_anchor
                                .take()
                                .expect("every executing candidate owns a frozen JobStart anchor"),
                        }),
                        control.fuel_burned(),
                    ));
                }
            }
            CanonicalStepResult::ResourceNeed(need) => {
                if answered_needs.contains(&need) {
                    return Err(SessionError::ResourceNoProgress {
                        need: Box::new(need),
                    });
                }
                let outcome = {
                    let mut world = ResourceWorld::new(universe);
                    host.fulfill(&mut world, &need)
                };
                match outcome {
                    ResourceOutcome::Fulfilled(fulfillment) => {
                        ledger
                            .fulfill(control, &need, fulfillment)
                            .map_err(|_| SessionError::UnexpectedResource)?;
                        answered_needs.push(need);
                    }
                    ResourceOutcome::Unavailable => {
                        ledger.mark_unavailable(control, &need, false);
                        answered_needs.push(need);
                    }
                    ResourceOutcome::Declined => return Ok(PlanExecution::Suspended(need)),
                }
            }
            CanonicalStepResult::Failed(error) => return Err(map_step_failure(error)),
        }
    }
}

fn map_terminal_completion_error(
    error: tex_exec::EngineCompletionError,
    burned: u64,
    limit: u64,
) -> SessionError {
    if matches!(
        error,
        tex_exec::EngineCompletionError::TerminalRevisionUnavailable
    ) && burned >= limit
    {
        SessionError::Execute(tex_exec::ExecError::CumulativeFuelExceeded {
            limit,
            attempted: burned.saturating_add(1),
        })
    } else {
        SessionError::EngineCompletion(error)
    }
}

fn register_materialized_primitives<G>(
    universe: &mut Universe<G>,
    profile: CommandProfile,
    compatibility: CommandCompatibility,
) {
    tex_command::register_tex82_expandable_primitives(universe);
    tex_exec::register_unexpandable_primitives(universe);
    if profile.capabilities().supports_etex() {
        tex_command::register_etex_expandable_primitives(universe);
        tex_exec::register_etex_unexpandable_primitives(universe);
    }
    if profile.capabilities().supports_pdftex() {
        tex_command::register_pdftex_expandable_primitives(universe);
        tex_command::register_pdftex_unexpandable_primitives(universe);
    }
    if compatibility == CommandCompatibility::Latex {
        tex_command::register_latex_expandable_primitives(universe);
    }
}

fn validate_materialized_font_policy<G>(
    universe: &mut Universe<G>,
    required: Option<tex_fonts::FontLayoutPolicy>,
) -> Result<(), SessionError> {
    if required == Some(tex_fonts::FontLayoutPolicy::OpenTypePreferred)
        && let Some(font) = universe
            .command_context()
            .map_err(SessionError::Universe)?
            .font_artifact_recipes()
            .into_iter()
            .skip(1)
            .find(|font| font.layout_policy != tex_fonts::FontLayoutPolicy::OpenTypePreferred)
    {
        return Err(SessionError::FormatFontPolicy { name: font.name });
    }
    Ok(())
}

fn install_latex_compatibility<G>(universe: &mut Universe<G>) -> Result<(), SessionError> {
    tex_command::install_latex_expandable_primitives(universe);
    for character in ['{', '}', '$', '&', '#', '^', '_'] {
        universe
            .assign_code(
                CodeTableKind::Catcode,
                character,
                i64::from(tex_state::token::Catcode::Other as u8),
                AssignmentScope::Global,
            )
            .map_err(SessionError::Universe)?;
    }
    Ok(())
}

fn install_plain_catcodes<G>(universe: &mut Universe<G>) -> Result<(), SessionError> {
    use tex_state::token::Catcode;

    for (character, catcode) in [
        ('\\', Catcode::Escape),
        ('{', Catcode::BeginGroup),
        ('}', Catcode::EndGroup),
        ('$', Catcode::MathShift),
        ('&', Catcode::AlignmentTab),
        ('#', Catcode::Parameter),
        ('^', Catcode::Superscript),
        ('_', Catcode::Subscript),
        (' ', Catcode::Space),
        ('~', Catcode::Active),
        ('%', Catcode::Comment),
    ] {
        universe
            .assign_code(
                CodeTableKind::Catcode,
                character,
                i64::from(catcode as u8),
                AssignmentScope::Global,
            )
            .map_err(SessionError::Universe)?;
    }
    for character in ('A'..='Z').chain('a'..='z') {
        universe
            .assign_code(
                CodeTableKind::Catcode,
                character,
                i64::from(Catcode::Letter as u8),
                AssignmentScope::Global,
            )
            .map_err(SessionError::Universe)?;
    }
    Ok(())
}

struct CandidateControlOptions<'a> {
    job_name: &'a str,
    source_path: &'a str,
    bytes: Arc<[u8]>,
    profile: CommandProfile,
    compatibility: CommandCompatibility,
    initex: bool,
    emit_dvi: bool,
    root_framing: SourceFramingPolicy,
    root_framing_name: Option<&'a str>,
}

fn prepare_candidate_control<G>(
    universe: &mut Universe<G>,
    options: &CandidateControlOptions<'_>,
    control: &mut Option<MainControl<G>>,
    materialized_job_start: bool,
) -> Result<(), SessionError> {
    if control.is_none() && options.initex && !materialized_job_start {
        tex_command::install_tex82_expandable_primitives(universe);
        tex_exec::install_unexpandable_primitives(universe);
        if matches!(
            options.profile.dialect(),
            tex_command::CommandDialect::Etex26 | tex_command::CommandDialect::Pdftex14029
        ) {
            tex_command::install_etex_expandable_primitives(universe);
            tex_exec::install_etex_unexpandable_primitives(universe);
        }
        if options.profile.dialect() == tex_command::CommandDialect::Pdftex14029 {
            universe.enable_pdf_output();
            tex_command::install_pdftex_expandable_primitives(universe);
            tex_command::install_pdftex_unexpandable_primitives(universe);
        }
        if options.compatibility == CommandCompatibility::Latex {
            install_latex_compatibility(universe)?;
        }
    }
    control.get_or_insert_with(|| {
        if options.initex {
            MainControl::prepared_initex(options.profile)
        } else {
            MainControl::with_profile(options.profile)
        }
    });
    let control = control
        .as_mut()
        .expect("candidate control was inserted in the owner slot");
    debug_assert_eq!(control.command_profile(), options.profile);
    control.set_initex_mode(options.initex);
    if options.profile.dialect() == tex_command::CommandDialect::Pdftex14029 {
        control.set_engine_binary(tex_exec::EngineBinaryIdentity::Pdftex14029);
    }
    // The frozen JobStart anchor is captured before startup framing invokes
    // `begin_job_*`, so install the executable's immutable capacity contract
    // here as part of the pre-job aggregate. Startup repeats this selection
    // idempotently after materialization.
    control.prepare_job_start_stores(universe);
    control.set_dvi_output(options.emit_dvi);
    control
        .capabilities_mut()
        .set_startup_job_name(options.job_name);
    Ok(())
}

fn start_candidate_job<G>(
    universe: &mut Universe<G>,
    control: &mut MainControl<G>,
    options: CandidateControlOptions<'_>,
) -> Result<(), SessionError> {
    // tex.web §241 refreshes the four volatile clock cells at the same
    // lifecycle boundary that opens §536's transcript and frames §534's
    // startup line. This must precede root registration so §537's opening
    // parenthesis cannot overtake the banner/log prefix.
    control.begin_job_for_input(
        universe,
        options.root_framing_name.unwrap_or(options.source_path),
        options.job_name,
    );
    let mut registration = SourceRegistration::new(RegisteredSourceKind::Generated, options.bytes)
        .with_name(options.source_path)
        .with_framing(options.root_framing);
    if let Some(name) = options.root_framing_name {
        registration = registration.with_framing_name(name);
    }
    control.register_root_source(registration)?;
    control.open_registered_root_framing(universe);
    Ok(())
}

/// Typed result of resolving an accepted rendered event against a DOM revision.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RenderedSourceResult {
    Current(ResolvedSourceLocation),
    Deleted { minted_revision: u64 },
    StaleRevision { accepted: RevisionId },
    OutputMismatch { accepted: RenderedOutputId },
}

#[derive(Debug)]
struct PageRenderMap {
    event_units: Vec<u32>,
    origins: Vec<ArtifactOrigin>,
}

impl PageRenderMap {
    fn retained_bytes(&self) -> usize {
        self.event_units
            .capacity()
            .saturating_mul(size_of::<u32>())
            .saturating_add(
                self.origins
                    .capacity()
                    .saturating_mul(size_of::<ArtifactOrigin>()),
            )
    }

    fn origin(&self, event: u32, unit: Option<u32>) -> Option<ArtifactOrigin> {
        let event = usize::try_from(event).ok()?;
        let start = *self.event_units.get(event)? as usize;
        let end = *self.event_units.get(event.checked_add(1)?)? as usize;
        let origins = self.origins.get(start..end)?;
        let origin = match unit {
            Some(unit) => origins.get(usize::try_from(unit).ok()?)?.clone(),
            None => origins
                .iter()
                .find(|origin| **origin != ArtifactOrigin::Unknown)?
                .clone(),
        };
        (origin != ArtifactOrigin::Unknown).then_some(origin)
    }
}

#[derive(Debug)]
struct RenderMapCache {
    pages: BTreeMap<usize, PageRenderMap>,
    clock: VecDeque<usize>,
    retained_bytes: usize,
    max_retained_bytes: usize,
}

struct RetainedRevisionGeneration<'store> {
    revision: RevisionId,
    generation: tex_exec::RetainedEngineGeneration<'store>,
    checkpoint_count: usize,
}

impl RenderMapCache {
    fn new(max_retained_bytes: usize) -> Self {
        Self {
            pages: BTreeMap::new(),
            clock: VecDeque::new(),
            retained_bytes: 0,
            max_retained_bytes,
        }
    }

    fn set_budget(&mut self, max_retained_bytes: usize) {
        self.max_retained_bytes = max_retained_bytes;
        self.evict_to_fit(0);
    }

    fn clear(&mut self) {
        self.pages.clear();
        self.clock.clear();
        self.retained_bytes = 0;
    }

    fn touch(&mut self, page: usize) {
        self.clock.retain(|candidate| *candidate != page);
        self.clock.push_back(page);
    }

    fn admit(&mut self, page: usize, map: PageRenderMap) {
        let charge = map
            .retained_bytes()
            .saturating_add(size_of::<usize>())
            .saturating_add(size_of::<PageRenderMap>());
        if charge > self.max_retained_bytes {
            return;
        }
        self.evict_to_fit(charge);
        self.pages.insert(page, map);
        self.touch(page);
        self.retained_bytes = self.retained_bytes.saturating_add(charge);
    }

    fn evict_to_fit(&mut self, incoming: usize) {
        while self.retained_bytes.saturating_add(incoming) > self.max_retained_bytes {
            let Some(page) = self.clock.pop_front() else {
                break;
            };
            let Some(map) = self.pages.remove(&page) else {
                continue;
            };
            let charge = map
                .retained_bytes()
                .saturating_add(size_of::<usize>())
                .saturating_add(size_of::<PageRenderMap>());
            self.retained_bytes = self.retained_bytes.saturating_sub(charge);
        }
    }
}

/// Long-lived incremental session borrowing one caller-owned reachability
/// store and retaining only an opaque coarse generation lease.
pub struct Session<'store> {
    job_name: String,
    reachability_store: tex_state::ReachabilityStore,
    reachability_owner: core::marker::PhantomData<&'store tex_state::ReachabilityStore>,
    revision: RevisionId,
    output_id: RenderedOutputId,
    source: String,
    fragments: FragmentStore,
    layout: EditorLayout,
    content_hash: ContentHash,
    history: Vec<BoundaryRecord>,
    dependencies: Vec<tex_state::InputDependency>,
    checkpoint_budget: usize,
    registered_inputs: BTreeMap<PathBuf, Arc<[u8]>>,
    accepted_retention: Option<RetentionMetrics>,
    required_font_layout_policy: Option<tex_fonts::FontLayoutPolicy>,
    job_clock: JobClock,
    utf8_input_as_bytes: bool,
    dvi_output: bool,
    root_framing: SourceFramingPolicy,
    root_framing_name: Option<String>,
    root_source_is_byte_projection: bool,
    command_profile: CommandProfile,
    command_compatibility: CommandCompatibility,
    initex: bool,
    expansion_stats: ExpansionStats,
    render_maps: RefCell<RenderMapCache>,
    /// The sole accepted runtime generation. A candidate owns the only other
    /// generation while it is being driven; detached history never enters
    /// this slot.
    prior_generation: Option<RetainedRevisionGeneration<'store>>,
    accepted_completion: Option<Arc<DetachedEngineCompletion>>,
    retired_generations: usize,
    converged_candidate_generations: usize,
    candidate_lease: Arc<CandidateLeaseState>,
    /// Complete immutable pre-job base. It is checkpoint-owned data, not a
    /// retained runtime generation or a mutable journal lineage.
    job_start_anchor: Option<FrozenJobStartAnchor>,
}

impl<'store> Session<'store> {
    fn job_start_session_metadata(&self) -> JobStartSessionMetadata {
        JobStartSessionMetadata {
            profile: self.effective_command_profile(),
            compatibility: self.command_compatibility,
            job_clock: self.job_clock,
        }
    }

    /// Starts an editor session with no admitted generation.
    ///
    /// `template` is accepted only as a migration-time configuration marker;
    /// live runtime state is never retained or cloned from it.
    pub fn start(
        reachability_store: &'store tex_state::ReachabilityStore,
        job_name: impl Into<String>,
        revision: RevisionId,
        source: impl Into<String>,
        checkpoint_budget: usize,
    ) -> Result<Self, SessionError> {
        Self::start_with_source_path(
            reachability_store,
            job_name,
            "<editor>",
            revision,
            source,
            checkpoint_budget,
        )
    }

    pub fn start_with_source_path(
        reachability_store: &'store tex_state::ReachabilityStore,
        job_name: impl Into<String>,
        source_path: impl Into<String>,
        revision: RevisionId,
        source: impl Into<String>,
        checkpoint_budget: usize,
    ) -> Result<Self, SessionError> {
        Self::start_with_prepared_source(
            reachability_store.clone(),
            job_name,
            source_path,
            revision,
            source.into(),
            false,
            checkpoint_budget,
        )
    }

    pub fn start_with_source_bytes(
        reachability_store: &'store tex_state::ReachabilityStore,
        job_name: impl Into<String>,
        source_path: impl Into<String>,
        revision: RevisionId,
        bytes: Vec<u8>,
        checkpoint_budget: usize,
    ) -> Result<Self, SessionError> {
        Self::start_with_source_bytes_owned(
            reachability_store.clone(),
            job_name,
            source_path,
            revision,
            bytes,
            checkpoint_budget,
        )
    }

    #[doc(hidden)]
    pub fn start_with_source_bytes_owned(
        reachability_store: tex_state::ReachabilityStore,
        job_name: impl Into<String>,
        source_path: impl Into<String>,
        revision: RevisionId,
        bytes: Vec<u8>,
        checkpoint_budget: usize,
    ) -> Result<Self, SessionError> {
        let (source, byte_projection) = match String::from_utf8(bytes) {
            Ok(source) => (source, false),
            Err(error) => (
                error.into_bytes().into_iter().map(char::from).collect(),
                true,
            ),
        };
        Self::start_with_prepared_source(
            reachability_store,
            job_name,
            source_path,
            revision,
            source,
            byte_projection,
            checkpoint_budget,
        )
    }

    fn start_with_prepared_source(
        reachability_store: tex_state::ReachabilityStore,
        job_name: impl Into<String>,
        source_path: impl Into<String>,
        revision: RevisionId,
        source: String,
        root_source_is_byte_projection: bool,
        checkpoint_budget: usize,
    ) -> Result<Self, SessionError> {
        let source_path = source_path.into();
        let mut fragments = FragmentStore::new();
        let (fragment, _) = fragments.append(Arc::from(source.as_bytes()), revision.raw())?;
        let fragment_len = u32::try_from(source.len())
            .map_err(|_| SessionError::Layout(EditorLayoutError::DocumentTooLarge))?;
        let layout = EditorLayout::new(
            source_path.clone(),
            LayoutGeneration::new(revision.raw()),
            vec![Piece::new(fragment, 0, fragment_len)],
            &fragments,
        )?;
        let mut output_id = [0; 16];
        getrandom::fill(&mut output_id).map_err(SessionError::OutputIdentity)?;
        Ok(Self {
            job_name: job_name.into(),
            reachability_store,
            reachability_owner: core::marker::PhantomData,
            revision,
            output_id: RenderedOutputId::from_bytes(output_id),
            content_hash: ContentHash::from_bytes(source.as_bytes()),
            source,
            fragments,
            layout,
            history: Vec::new(),
            dependencies: Vec::new(),
            checkpoint_budget,
            registered_inputs: BTreeMap::new(),
            accepted_retention: None,
            required_font_layout_policy: None,
            job_clock: JobClock::default(),
            utf8_input_as_bytes: false,
            dvi_output: true,
            root_framing: SourceFramingPolicy::Canonical,
            root_framing_name: None,
            root_source_is_byte_projection,
            command_profile: CommandProfile::TEX82,
            command_compatibility: CommandCompatibility::Profile,
            initex: true,
            expansion_stats: ExpansionStats::default(),
            render_maps: RefCell::new(RenderMapCache::new(usize::MAX)),
            prior_generation: None,
            accepted_completion: None,
            retired_generations: 0,
            converged_candidate_generations: 0,
            candidate_lease: CandidateLeaseState::new(),
            job_start_anchor: None,
        })
    }

    #[must_use]
    pub const fn revision(&self) -> RevisionId {
        self.revision
    }

    #[must_use]
    pub const fn output_id(&self) -> RenderedOutputId {
        self.output_id
    }

    #[must_use]
    pub const fn content_hash(&self) -> ContentHash {
        self.content_hash
    }

    #[must_use]
    pub fn source(&self) -> &str {
        &self.source
    }

    #[must_use]
    pub fn history(&self) -> &[BoundaryRecord] {
        &self.history
    }

    pub fn accepted_input_dependencies(&self) -> impl Iterator<Item = &tex_state::InputDependency> {
        self.dependencies.iter()
    }

    pub fn set_utf8_input_as_bytes(&mut self, enabled: bool) {
        assert!(
            self.history.is_empty(),
            "input mode is fixed after execution"
        );
        self.utf8_input_as_bytes = enabled;
    }

    pub fn set_command_profile(&mut self, profile: CommandProfile, initex: bool) {
        assert!(self.history.is_empty(), "profile is fixed after execution");
        self.command_profile = profile;
        self.initex = initex;
    }

    pub fn set_command_compatibility(&mut self, compatibility: CommandCompatibility) {
        assert!(
            self.history.is_empty(),
            "command compatibility is fixed after execution"
        );
        self.command_compatibility = compatibility;
    }

    /// Installs one validated transport image as the immutable pre-job base.
    /// Runtime materialization is deferred until a candidate actually starts;
    /// the anchor itself consumes neither retained-generation slot.
    pub fn set_format_image(&mut self, image: DetachedFormatImage) -> Result<(), SessionError> {
        assert!(self.history.is_empty(), "format is fixed after execution");
        assert!(
            self.prior_generation.is_none(),
            "one session admits at most one initial format checkpoint"
        );
        assert!(
            !self.candidate_lease.is_claimed(),
            "format admission precedes candidate construction"
        );
        self.job_start_anchor = Some(FrozenJobStartAnchor::loaded(image));
        self.initex = false;
        Ok(())
    }

    pub fn set_required_font_layout_policy(&mut self, policy: tex_fonts::FontLayoutPolicy) {
        assert!(
            self.history.is_empty(),
            "font policy is fixed after execution"
        );
        self.required_font_layout_policy = Some(policy);
    }

    pub fn set_job_clock(&mut self, clock: JobClock) {
        assert!(
            self.history.is_empty(),
            "job clock is fixed after execution"
        );
        self.job_clock = clock;
    }

    pub fn set_root_source_framing(&mut self, framing: SourceFramingPolicy) {
        assert!(self.history.is_empty(), "framing is fixed after execution");
        self.root_framing = framing;
    }

    pub fn set_root_source_framing_name(&mut self, name: impl Into<String>) {
        assert!(self.history.is_empty(), "framing is fixed after execution");
        self.root_framing_name = Some(name.into());
    }

    pub fn set_dvi_output(&mut self, enabled: bool) {
        assert!(
            self.history.is_empty(),
            "DVI policy is fixed after execution"
        );
        self.dvi_output = enabled;
    }

    #[must_use]
    pub fn source_file_bytes(&self, source: &str) -> Vec<u8> {
        source_file_bytes(source, self.root_source_is_byte_projection)
    }

    #[must_use]
    pub fn pure_memo_stats(&self) -> tex_state::PureMemoStats {
        tex_state::PureMemoStats::default()
    }

    pub fn set_render_cache_budget(&self, max_retained_bytes: usize) {
        self.render_maps.borrow_mut().set_budget(max_retained_bytes);
    }

    pub fn evict_rebuildable_caches(&mut self) {
        self.render_maps.borrow_mut().clear();
    }

    #[must_use]
    pub fn retention_metrics(&self) -> Option<RetentionMetrics> {
        self.accepted_retention.map(|mut retention| {
            retention.output_bytes = retention
                .output_bytes
                .saturating_add(self.render_maps.borrow().retained_bytes);
            retention
        })
    }

    #[must_use]
    pub fn job_start_anchor_metrics(&self) -> Option<JobStartAnchorMetrics> {
        self.job_start_anchor.as_ref().map(|anchor| anchor.metrics)
    }

    /// Number of previously accepted coarse generations retired as complete
    /// bundles by this session.
    #[must_use]
    pub const fn retired_generation_count(&self) -> usize {
        self.retired_generations
    }

    /// Number of current generations rejected after authoritative boundary
    /// convergence while the accepted generation remained resident.
    #[must_use]
    pub const fn converged_candidate_generation_count(&self) -> usize {
        self.converged_candidate_generations
    }

    #[must_use]
    pub fn retained_generation_count(&self) -> usize {
        usize::from(self.prior_generation.is_some())
    }

    /// Number of reserved current-candidate generation slots.
    ///
    /// A newly issued candidate reserves its slot before lazily constructing
    /// the generation, so this is an upper bound on current generations.
    #[must_use]
    pub fn current_candidate_generation_count(&self) -> usize {
        usize::from(self.candidate_lease.is_claimed())
    }

    /// Upper bound on live prior-plus-current revision generations.
    #[must_use]
    pub fn occupied_generation_slot_count(&self) -> usize {
        self.retained_generation_count()
            .saturating_add(self.current_candidate_generation_count())
    }

    pub fn retained_revision_ids(&self) -> impl Iterator<Item = RevisionId> + '_ {
        self.prior_generation
            .iter()
            .map(|generation| generation.revision)
    }

    #[must_use]
    pub fn current_retained_checkpoint_count(&self) -> usize {
        self.prior_generation
            .as_ref()
            .map_or(0, |generation| generation.checkpoint_count)
    }

    /// Demand-free page ownership counters on the accepted production
    /// generation. This is intentionally a scalar lifecycle observation, not
    /// a page-list traversal or detached output request.
    #[doc(hidden)]
    pub fn page_material_counters(
        &mut self,
    ) -> Result<Option<tex_state::fork_arena::ForkArenaCounters>, SessionError> {
        self.prior_generation
            .as_mut()
            .map(|generation| {
                generation
                    .generation
                    .with_admitted(ReadPageMaterialCounters)
                    .map_err(SessionError::RetainedEngine)
            })
            .transpose()
    }

    /// Demand-free page-region lifecycle counters on the accepted generation.
    #[doc(hidden)]
    pub fn page_region_counters(
        &mut self,
    ) -> Result<Option<tex_state::PageRegionCounters>, SessionError> {
        self.prior_generation
            .as_mut()
            .map(|generation| {
                generation
                    .generation
                    .with_admitted(ReadPageRegionCounters)
                    .map_err(SessionError::RetainedEngine)
            })
            .transpose()
    }

    /// Demand-free PageBuilder candidate-settlement work on the accepted
    /// production generation.
    #[doc(hidden)]
    pub fn page_candidate_settlement_counters(
        &mut self,
    ) -> Result<Option<tex_state::PageCandidateSettlementCounters>, SessionError> {
        self.prior_generation
            .as_mut()
            .map(|generation| {
                generation
                    .generation
                    .with_admitted(ReadPageCandidateSettlementCounters)
                    .map_err(SessionError::RetainedEngine)
            })
            .transpose()
    }

    /// Demand-free command checkpoint work on the accepted production owner.
    #[doc(hidden)]
    pub fn command_timeline_counters(
        &mut self,
    ) -> Result<Option<tex_command::CommandTimelineCounters>, SessionError> {
        self.prior_generation
            .as_mut()
            .map(|generation| {
                generation
                    .generation
                    .with_admitted(ReadCommandTimelineCounters)
                    .map_err(SessionError::RetainedEngine)?
                    .map_err(SessionError::RetainedEngine)
            })
            .transpose()
    }

    pub fn register_input_file(
        &mut self,
        path: &Path,
        bytes: impl Into<Arc<[u8]>>,
    ) -> Result<(), SessionError> {
        self.registered_inputs.insert(path.to_owned(), bytes.into());
        Ok(())
    }

    pub fn cold(&mut self) -> Result<AcceptedOutput, SessionError> {
        self.cold_with_resolvers(&mut DirectResourceHost)
    }

    pub fn cold_with_resolvers(
        &mut self,
        host: &mut dyn ResourceHost,
    ) -> Result<AcceptedOutput, SessionError> {
        let mut candidate = self.start_cold_candidate()?;
        drive_synchronous_candidate(&mut candidate, host)?;
        self.accept_cold_candidate(candidate)
    }

    pub fn cold_with_resource_resolvers(
        &mut self,
        host: &mut dyn ResourceHost,
    ) -> Result<AcceptedOutput, SessionError> {
        self.cold_with_resolvers(host)
    }

    pub fn start_cold_candidate(&mut self) -> Result<RevisionCandidate<'store>, SessionError> {
        self.candidate(CandidatePlan {
            base_revision: self.revision,
            base_content_hash: self.content_hash,
            revision: self.revision,
            source: self.source.clone(),
            fragments: self.fragments.clone(),
            layout: clone_layout(&self.layout, &self.fragments)?,
            execution_path: RevisionExecutionPath::Cold,
            restart_limit: None,
            edit_map: None,
            restart_boundary: None,
            restart_fork_latency: Duration::ZERO,
            revision_setup_latency: Duration::ZERO,
        })
    }

    pub fn accept_cold_candidate(
        &mut self,
        candidate: RevisionCandidate<'store>,
    ) -> Result<AcceptedOutput, SessionError> {
        let transaction = self.prepare_revision_candidate(candidate)?;
        self.accept_revision(transaction)
    }

    pub fn start_advance_candidate(
        &mut self,
        next_revision: RevisionId,
        edit: Edit,
    ) -> Result<RevisionCandidate<'store>, SessionError> {
        self.start_advance_candidate_with_path(next_revision, edit, RevisionExecutionPath::SlowEdit)
    }

    pub fn start_advance_candidate_from_job_start(
        &mut self,
        next_revision: RevisionId,
        edit: Edit,
    ) -> Result<RevisionCandidate<'store>, SessionError> {
        self.start_advance_candidate_with_path(
            next_revision,
            edit,
            RevisionExecutionPath::ForcedJobStartFallback,
        )
    }

    pub fn start_external_input_delta_candidate(
        &mut self,
    ) -> Result<RevisionCandidate<'store>, SessionError> {
        self.candidate(CandidatePlan {
            base_revision: self.revision,
            base_content_hash: self.content_hash,
            revision: self.revision,
            source: self.source.clone(),
            fragments: self.fragments.clone(),
            layout: clone_layout(&self.layout, &self.fragments)?,
            execution_path: RevisionExecutionPath::ExternalInputDelta,
            restart_limit: None,
            edit_map: None,
            restart_boundary: None,
            restart_fork_latency: Duration::ZERO,
            revision_setup_latency: Duration::ZERO,
        })
    }

    fn start_advance_candidate_with_path(
        &mut self,
        next_revision: RevisionId,
        edit: Edit,
        execution_path: RevisionExecutionPath,
    ) -> Result<RevisionCandidate<'store>, SessionError> {
        let started = Timer::start();
        self.validate_edit(next_revision, &edit)?;
        let mut next = self.source.clone();
        next.replace_range(edit.range.clone(), &edit.replacement);
        let (expanded_range, expanded_replacement) = line_expanded_replacement(&self.source, &edit);
        let mut fragments = self.fragments.clone();
        let (fragment, _) = fragments.append(
            Arc::from(expanded_replacement.as_bytes()),
            next_revision.raw(),
        )?;
        let layout = replace_layout_range(
            &self.layout,
            &fragments,
            expanded_range,
            fragment,
            expanded_replacement.len(),
            LayoutGeneration::new(next_revision.raw()),
        )?;
        self.candidate(CandidatePlan {
            base_revision: self.revision,
            base_content_hash: self.content_hash,
            revision: next_revision,
            source: next,
            fragments,
            layout,
            execution_path,
            restart_limit: (execution_path == RevisionExecutionPath::SlowEdit)
                .then_some(edit.range.start),
            edit_map: Some(RevisionEditMap {
                old_start: edit.range.start,
                old_end: edit.range.end,
                new_end: edit.range.start.saturating_add(edit.replacement.len()),
            }),
            restart_boundary: None,
            restart_fork_latency: Duration::ZERO,
            revision_setup_latency: started.elapsed(),
        })
    }

    fn candidate(
        &mut self,
        mut plan: CandidatePlan,
    ) -> Result<RevisionCandidate<'store>, SessionError> {
        let candidate_lease = self.candidate_lease.claim()?;
        let fork_started = Timer::start();
        let mut materialized_job_start = false;
        let rooted = if let (Some(prior), Some(limit)) =
            (self.prior_generation.as_mut(), plan.restart_limit)
        {
            prior
                .generation
                .fork_latest_boundary_at_or_before(limit)
                .map_err(SessionError::RetainedEngineFork)?
        } else {
            None
        };
        let (generation, checkpoint_control_key, inherited_boundary) = if let Some((
            generation,
            runtime,
            _budget_counters,
            selected,
            selected_key,
            retention,
        )) = rooted
        {
            let selected_boundary = BoundaryKey {
                position: selected.position(),
                boundary: selected.boundary(),
                ordinal: selected.ordinal(),
            };
            let inherited_boundary = (!self
                .history
                .iter()
                .any(|record| record.key == selected_boundary))
            .then_some(InheritedBoundary {
                key: selected_key,
                evidence: selected,
                retention,
            });
            plan.restart_boundary = Some(selected_boundary);
            (Some(generation), Some(runtime), inherited_boundary)
        } else if self.prior_generation.is_some() {
            if plan.execution_path == RevisionExecutionPath::SlowEdit {
                plan.execution_path = RevisionExecutionPath::ForcedJobStartFallback;
            }
            let metadata = self.job_start_session_metadata();
            let anchor = self
                .job_start_anchor
                .as_mut()
                .ok_or(SessionError::MissingJobStartAnchor)?;
            let image = anchor.materialize_image(metadata)?;
            let generation = self
                .prior_generation
                .as_mut()
                .expect("JobStart fallback has an accepted generation")
                .generation
                .materialize_job_start_candidate(
                    World::memory_with_clock(self.job_clock),
                    image,
                    true,
                )
                .map_err(SessionError::Format)?;
            plan.restart_boundary = self
                .history
                .iter()
                .find(|record| record.key.boundary == EngineBoundary::JobStart)
                .map(|record| record.key);
            materialized_job_start = true;
            (Some(generation), None, None)
        } else {
            (None, None, None)
        };
        let comparison_history: Arc<[BoundaryRecord]> = Arc::from(self.history.clone());
        let comparison_start = plan.restart_boundary.and_then(|selected| {
            comparison_history
                .iter()
                .position(|record| record.key == selected)
                .map(|index| index.saturating_add(1))
        });
        plan.restart_fork_latency = fork_started.elapsed();
        Ok(RevisionCandidate {
            session_output_id: self.output_id,
            reachability_store: self.reachability_store.clone(),
            reachability_owner: core::marker::PhantomData,
            job_name: self.job_name.clone(),
            source_path: plan.layout.path().to_owned(),
            plan,
            registered_inputs: self.registered_inputs.clone(),
            profile: self.effective_command_profile(),
            compatibility: self.command_compatibility,
            required_font_layout_policy: self.required_font_layout_policy,
            initex: self.initex,
            dvi_output: self.dvi_output,
            root_framing: self.root_framing,
            root_framing_name: self.root_framing_name.clone(),
            root_source_is_byte_projection: self.root_source_is_byte_projection,
            job_clock: self.job_clock,
            completed: None,
            cumulative_fuel_limit: MainControl::<GenerationBrand<'static>>::DEFAULT_FUEL_LIMIT,
            execution_budgets: tex_exec::ExecutionBudgets::default(),
            checkpoint_budget: self.checkpoint_budget,
            provenance_demand: tex_state::ProvenanceDemand::default(),
            provenance_budgets: tex_state::ProvenanceBudgets::default(),
            suspension_serial: 0,
            advance_calls: 0,
            cumulative_fuel: 0,
            generation,
            inherited_boundary,
            checkpoint_control_key,
            runtime_key: None,
            candidate_lease: Some(candidate_lease),
            job_start_anchor: self.job_start_anchor.clone(),
            materialized_job_start,
            comparison_history,
            comparison_start,
            accepted_dependencies: Arc::from(self.dependencies.clone()),
        })
    }

    fn effective_command_profile(&self) -> CommandProfile {
        if self.utf8_input_as_bytes || self.root_source_is_byte_projection {
            self.command_profile
        } else {
            CommandProfile::unicode_extended(self.command_profile.dialect())
        }
    }

    pub fn prepare_revision_candidate(
        &mut self,
        mut candidate: RevisionCandidate<'store>,
    ) -> Result<RevisionTransaction<'store>, SessionError> {
        let mut completion = candidate
            .completed
            .take()
            .ok_or(SessionError::CandidateNotComplete)?;
        let old_history_start = candidate
            .plan
            .restart_boundary
            .and_then(|selected| {
                self.history
                    .iter()
                    .position(|record| record.key == selected)
            })
            .map_or(0, |selected| selected.saturating_add(1));
        let new_history_start = candidate
            .plan
            .restart_boundary
            .filter(|selected| {
                completion
                    .history
                    .first()
                    .is_some_and(|record| record.key == *selected)
            })
            .map_or(0, |_| 1);
        let mut reuse = compare_histories(HistoryComparison {
            execution_path: candidate.plan.execution_path,
            old: &self.history[old_history_start..],
            new: &completion.history[new_history_start..],
            edit: candidate.plan.edit_map,
            source_len: candidate.plan.source.len(),
            delivered_commands: completion.delivered_commands,
            revision_setup_latency: candidate.plan.revision_setup_latency,
            restart_fork_latency: candidate.plan.restart_fork_latency,
            pages_retyped: completion.completion.pages().len(),
        });
        if reuse.same_history_attempts == 0
            && candidate.plan.execution_path != RevisionExecutionPath::Cold
            && candidate.plan.base_content_hash
                == ContentHash::from_bytes(candidate.plan.source.as_bytes())
            && let Some(selected) = candidate.plan.restart_boundary
            && self
                .history
                .iter()
                .any(|record| record.key == selected && record.reachable_state_identity.is_some())
        {
            // The exact selected root was identity-validated before the fork.
            // With unchanged authored content it is itself the earliest
            // convergence point even when execution publishes no later
            // eligible boundary (for example a shipout-only job).
            reuse.same_history_attempts = 1;
            reuse.trace_nodes_walked = 1;
            reuse.convergence_boundary = Some(selected);
            reuse.same_history_stop = SameHistoryStop::Matched;
        }
        let mut convergence_source_boundary = None;
        if let Some(convergence) = reuse.convergence_boundary
            && let Some(new_index) = completion
                .history
                .iter()
                .position(|record| record.key == convergence)
            && let Some(old_index) = self.history.iter().position(|record| {
                let mapped = candidate
                    .plan
                    .edit_map
                    .and_then(|edit| edit.map_position(record.key.position))
                    .or_else(|| {
                        candidate
                            .plan
                            .edit_map
                            .is_none()
                            .then_some(record.key.position)
                    });
                mapped == Some(convergence.position)
                    && record.key.boundary == convergence.boundary
                    && record.key.ordinal == convergence.ordinal
                    && (candidate.plan.restart_boundary == Some(convergence)
                        || record.reachable_state_identity
                            == completion.history[new_index].reachable_state_identity)
            })
        {
            let new_record = &completion.history[new_index];
            let old_record = &self.history[old_index];
            convergence_source_boundary = Some(old_record.key);
            reuse.suffixes_adopted = self.history.len().saturating_sub(old_index);
            reuse.pages_retained_prefix = new_record.artifact_prefix;
            reuse.pages_reused = self.accepted_completion.as_ref().map_or(0, |accepted| {
                accepted
                    .pages()
                    .len()
                    .saturating_sub(old_record.artifact_prefix)
            });
            reuse.pages_retyped = new_record.artifact_prefix;
            if let Some(accepted) = self.accepted_completion.as_deref() {
                completion.completion.splice_retained_suffix(
                    accepted,
                    new_record.effect_prefix,
                    old_record.effect_prefix,
                    new_record.artifact_prefix,
                    old_record.artifact_prefix,
                )?;
            }
        }
        reuse.restart_boundary = candidate.plan.restart_boundary;
        let generation = candidate
            .generation
            .take()
            .ok_or(SessionError::CandidateNotComplete)?;
        let runtime_key = candidate.runtime_key.take();
        let candidate_lease = candidate
            .candidate_lease
            .take()
            .expect("a live candidate owns its session lease");
        Ok(RevisionTransaction {
            session_output_id: candidate.session_output_id,
            base_revision: candidate.plan.base_revision,
            base_content_hash: candidate.plan.base_content_hash,
            revision: candidate.plan.revision,
            content_hash: ContentHash::from_bytes(candidate.plan.source.as_bytes()),
            restart_boundary: candidate.plan.restart_boundary,
            edit_map: candidate.plan.edit_map,
            convergence_source_boundary,
            source: candidate.plan.source,
            fragments: candidate.plan.fragments,
            layout: candidate.plan.layout,
            completion: completion.completion,
            history: completion.history,
            dependencies: completion.dependencies,
            reuse,
            format_dump: completion.format_dump,
            expansion_stats: ExpansionStats::default(),
            generation,
            runtime_key,
            checkpoint_retained_bytes: completion.checkpoint_retained_bytes,
            checkpoint_shared_owner_bytes: completion.checkpoint_shared_owner_bytes,
            checkpoint_metadata_bytes: completion.checkpoint_metadata_bytes,
            detached_boundary_bytes: completion.detached_boundary_bytes,
            checkpoint_protected_overage_bytes: completion.checkpoint_protected_overage_bytes,
            job_start_anchor: completion.job_start_anchor,
            _candidate_lease: candidate_lease,
        })
    }

    pub fn accept_revision(
        &mut self,
        transaction: RevisionTransaction<'store>,
    ) -> Result<AcceptedOutput, SessionError> {
        if transaction.session_output_id != self.output_id {
            return Err(SessionError::CandidateKindMismatch);
        }
        if transaction.base_revision != self.revision {
            return Err(SessionError::StaleRevision {
                expected: self.revision,
                actual: transaction.base_revision,
            });
        }
        if transaction.base_content_hash != self.content_hash {
            return Err(SessionError::ContentHashMismatch);
        }
        let prior_retention = self.accepted_retention;
        let prior_checkpoint_count = self
            .prior_generation
            .as_ref()
            .map_or(0, |generation| generation.checkpoint_count);
        let mut prepared =
            prepare_candidate_runtime(transaction.generation, transaction.runtime_key)
                .map_err(SessionError::RetainedEngine)?;
        if let Err(error) = prepared.generation_mut().preflight_boundary_lane() {
            return Err(SessionError::RetainedEngine(error));
        }
        let checkpoint_count = prepared
            .generation_mut()
            .boundary_lane_checkpoint_count()
            .map_err(SessionError::RetainedEngine)?;
        // All fallible current-generation validation and root pruning happens
        // before either accepted metadata or the prior owner changes. The
        // current generation was constructed independently under its own
        // HRTB brand, so its checkpoint roots cannot contain a prior id.
        let converged =
            transaction.reuse.convergence_boundary.is_some() && self.prior_generation.is_some();
        let incoming = if converged {
            let edit_map = transaction
                .edit_map
                .expect("an editor convergence owns its revision map");
            let restart = transaction
                .restart_boundary
                .expect("an editor convergence owns its restart boundary");
            let old_convergence = transaction
                .convergence_source_boundary
                .expect("convergence names the accepted suffix boundary");
            let new_convergence = transaction
                .reuse
                .convergence_boundary
                .and_then(|key| transaction.history.iter().find(|record| record.key == key))
                .expect("convergence names the current boundary record");
            drop(prepared);
            let previous = self
                .prior_generation
                .as_mut()
                .expect("convergence retains the accepted generation");
            let accepted_root =
                source_file_bytes(&self.source, self.root_source_is_byte_projection);
            let current_root = Arc::from(source_file_bytes(
                &transaction.source,
                self.root_source_is_byte_projection,
            ));
            previous.checkpoint_count = previous
                .generation
                .rehome_editor_revision(
                    &accepted_root,
                    current_root,
                    transaction.revision.raw(),
                    edit_map.old_start,
                    edit_map.old_end,
                    edit_map.new_end,
                    (restart.position, restart.boundary, restart.ordinal),
                    (
                        old_convergence.position,
                        old_convergence.boundary,
                        old_convergence.ordinal,
                    ),
                    new_convergence.effect_prefix,
                    new_convergence.artifact_prefix,
                )
                .map_err(SessionError::RetainedEngine)?;
            previous.revision = transaction.revision;
            self.converged_candidate_generations =
                self.converged_candidate_generations.saturating_add(1);
            None
        } else {
            let previous = self.prior_generation.take();
            let generation = if let Some(mut previous) = previous {
                prepared.accept_control();
                previous
                    .generation
                    .prepare_candidate_accept(prepared.generation_mut());
                previous
                    .generation
                    .finish_candidate_accept(prepared.generation_mut());
                let generation = prepared.into_generation();
                previous
                    .generation
                    .retire()
                    .map_err(SessionError::Universe)?;
                self.retired_generations = self.retired_generations.saturating_add(1);
                generation
            } else {
                prepared.accept_control();
                prepared.into_generation()
            };
            Some(RetainedRevisionGeneration {
                revision: transaction.revision,
                generation,
                checkpoint_count,
            })
        };
        let acceptance = Timer::start();
        self.revision = transaction.revision;
        self.source = transaction.source;
        self.fragments = transaction.fragments;
        self.layout = transaction.layout;
        self.content_hash = transaction.content_hash;
        if converged {
            let edit_map = transaction
                .edit_map
                .expect("an editor convergence owns its revision map");
            let restart = transaction
                .restart_boundary
                .expect("an editor convergence owns its restart boundary");
            let old_convergence = transaction
                .convergence_source_boundary
                .expect("convergence names the accepted suffix boundary");
            let new_convergence = transaction
                .reuse
                .convergence_boundary
                .and_then(|key| transaction.history.iter().find(|record| record.key == key))
                .expect("convergence names the current boundary record");
            let restart_index = self
                .history
                .iter()
                .position(|record| record.key == restart)
                .expect("restart boundary belongs to accepted history");
            let convergence_index = self
                .history
                .iter()
                .position(|record| record.key == old_convergence)
                .expect("convergence boundary belongs to accepted history");
            let old_effect_prefix = self.history[convergence_index].effect_prefix;
            let old_artifact_prefix = self.history[convergence_index].artifact_prefix;
            let mut rehomed = self.history[..=restart_index].to_vec();
            let suffix_start = convergence_index.max(restart_index.saturating_add(1));
            rehomed.extend(
                self.history[suffix_start..]
                    .iter()
                    .cloned()
                    .map(|mut record| {
                        record.key.position = edit_map
                            .map_position(record.key.position)
                            .expect("adopted suffix anchors map outside the edit");
                        record.effect_prefix = new_convergence
                            .effect_prefix
                            .saturating_add(record.effect_prefix.saturating_sub(old_effect_prefix));
                        record.artifact_prefix = new_convergence.artifact_prefix.saturating_add(
                            record.artifact_prefix.saturating_sub(old_artifact_prefix),
                        );
                        record.revision = transaction.revision;
                        record
                    }),
            );
            for record in &mut rehomed[..=restart_index] {
                record.revision = transaction.revision;
            }
            self.history = rehomed;
        } else if let Some(selected) = transaction.restart_boundary
            && selected.boundary != EngineBoundary::JobStart
        {
            let prefix = self
                .history
                .iter()
                .position(|record| record.key == selected)
                .map_or(0, |index| index.saturating_add(1));
            self.history.truncate(prefix);
            self.history.extend(transaction.history);
        } else {
            let mut history = transaction.history;
            if let Some(selected) = transaction
                .restart_boundary
                .filter(|key| key.boundary == EngineBoundary::JobStart)
                && let Some(anchor_revision) = self
                    .history
                    .iter()
                    .find(|record| record.key == selected)
                    .map(|record| record.revision)
                && let Some(anchor) = history.iter_mut().find(|record| record.key == selected)
            {
                // The independently materialized boundary publishes fresh
                // identity/effect coordinates, but it still describes the
                // session's original frozen anchor.
                anchor.revision = anchor_revision;
            }
            self.history = history;
        }
        if let Some(incoming) = incoming {
            self.prior_generation = Some(incoming);
        }
        self.job_start_anchor = Some(transaction.job_start_anchor);
        self.dependencies = transaction.dependencies;
        self.expansion_stats = transaction.expansion_stats;
        self.render_maps.borrow_mut().clear();
        let output_bytes = detached_output_bytes(&transaction.completion);
        let retained_checkpoint = converged.then_some(prior_retention).flatten();
        let converged_checkpoint_count = self
            .prior_generation
            .as_ref()
            .map_or(0, |generation| generation.checkpoint_count);
        let converged_checkpoint_metadata = retained_checkpoint.map_or(0, |retention| {
            retention
                .checkpoint_metadata_bytes
                .checked_div(prior_checkpoint_count)
                .unwrap_or(0)
                .saturating_mul(converged_checkpoint_count)
        });
        let converged_detached_boundary_bytes = self
            .history
            .len()
            .saturating_mul(size_of::<BoundaryRecord>());
        let converged_checkpoint_root_bytes = retained_checkpoint.map_or(0, |retention| {
            retention
                .checkpoint_shared_owner_bytes
                .saturating_add(converged_checkpoint_metadata)
                .saturating_add(converged_detached_boundary_bytes)
        });
        let retention = RetentionMetrics {
            checkpoint_root_bytes: retained_checkpoint
                .map_or(transaction.checkpoint_retained_bytes, |_| {
                    converged_checkpoint_root_bytes
                }),
            checkpoint_shared_owner_bytes: retained_checkpoint
                .map_or(transaction.checkpoint_shared_owner_bytes, |retention| {
                    retention.checkpoint_shared_owner_bytes
                }),
            checkpoint_metadata_bytes: retained_checkpoint
                .map_or(transaction.checkpoint_metadata_bytes, |_| {
                    converged_checkpoint_metadata
                }),
            detached_boundary_bytes: retained_checkpoint
                .map_or(transaction.detached_boundary_bytes, |_| {
                    converged_detached_boundary_bytes
                }),
            memo_result_bytes: 0,
            diagnostic_bytes: self
                .fragments
                .retained_bytes()
                .saturating_add(self.layout.retained_bytes()),
            output_bytes,
            protected_overage_bytes: retained_checkpoint
                .map_or(transaction.checkpoint_protected_overage_bytes, |_| {
                    converged_checkpoint_root_bytes.saturating_sub(self.checkpoint_budget)
                }),
            job_start_anchor_bytes: self
                .job_start_anchor
                .as_ref()
                .map_or(0, |anchor| anchor.metrics.bytes),
        };
        self.accepted_retention = Some(retention);
        let mut reuse = transaction.reuse;
        reuse.acceptance_latency = acceptance.elapsed();
        let completion = Arc::new(transaction.completion);
        self.accepted_completion = Some(Arc::clone(&completion));
        Ok(AcceptedOutput {
            output_id: self.output_id,
            revision: self.revision,
            content_hash: self.content_hash,
            completion,
            format_dump: transaction.format_dump,
            reuse,
            retention,
        })
    }

    pub fn advance(
        &mut self,
        next_revision: RevisionId,
        edit: Edit,
    ) -> Result<AcceptedOutput, SessionError> {
        self.advance_with_resolvers(next_revision, edit, &mut DirectResourceHost)
    }

    pub fn advance_with_resolvers(
        &mut self,
        next_revision: RevisionId,
        edit: Edit,
        host: &mut dyn ResourceHost,
    ) -> Result<AcceptedOutput, SessionError> {
        let transaction = self.prepare_revision_with_resolvers(next_revision, edit, host)?;
        self.accept_revision(transaction)
    }

    pub fn advance_with_resource_resolvers(
        &mut self,
        next_revision: RevisionId,
        edit: Edit,
        host: &mut dyn ResourceHost,
    ) -> Result<AcceptedOutput, SessionError> {
        self.advance_with_resolvers(next_revision, edit, host)
    }

    pub fn prepare_revision_with_resolvers(
        &mut self,
        next_revision: RevisionId,
        edit: Edit,
        host: &mut dyn ResourceHost,
    ) -> Result<RevisionTransaction<'store>, SessionError> {
        let mut candidate = self.start_advance_candidate(next_revision, edit)?;
        drive_synchronous_candidate(&mut candidate, host)?;
        self.prepare_revision_candidate(candidate)
    }

    pub fn prepare_revision_with_resource_resolvers(
        &mut self,
        next_revision: RevisionId,
        edit: Edit,
        host: &mut dyn ResourceHost,
    ) -> Result<RevisionTransaction<'store>, SessionError> {
        self.prepare_revision_with_resolvers(next_revision, edit, host)
    }

    pub fn validate_edit(
        &self,
        next_revision: RevisionId,
        edit: &Edit,
    ) -> Result<(), SessionError> {
        if edit.base_revision != self.revision {
            return Err(SessionError::StaleRevision {
                expected: self.revision,
                actual: edit.base_revision,
            });
        }
        if edit.expected_hash != self.content_hash {
            return Err(SessionError::ContentHashMismatch);
        }
        if next_revision <= self.revision {
            return Err(SessionError::NonMonotonicRevision);
        }
        if edit.range.start > edit.range.end
            || edit.range.end > self.source.len()
            || !self.source.is_char_boundary(edit.range.start)
            || !self.source.is_char_boundary(edit.range.end)
        {
            return Err(SessionError::InvalidEditRange);
        }
        Ok(())
    }

    #[must_use]
    pub const fn accepted_expansion_stats(&self) -> ExpansionStats {
        self.expansion_stats
    }

    pub fn rendered_source_location(
        &self,
        output: &AcceptedOutput,
        page: u32,
        event: u32,
        unit: Option<u32>,
        output_id: RenderedOutputId,
        revision: RevisionId,
    ) -> Result<Option<RenderedSourceResult>, SessionError> {
        if output_id != self.output_id {
            return Ok(Some(RenderedSourceResult::OutputMismatch {
                accepted: self.output_id,
            }));
        }
        if revision != self.revision {
            return Ok(Some(RenderedSourceResult::StaleRevision {
                accepted: self.revision,
            }));
        }
        if output.output_id != self.output_id || output.revision != self.revision {
            return Ok(Some(RenderedSourceResult::StaleRevision {
                accepted: self.revision,
            }));
        }
        let Some(origin) = self.rendered_artifact_origin(output, page, event, unit)? else {
            return Ok(None);
        };
        let ArtifactOrigin::Detached(recipe) = origin else {
            return Ok(None);
        };
        let start = usize::try_from(recipe.start).ok();
        let end = usize::try_from(recipe.end).ok();
        let Some((start, end)) = start.zip(end) else {
            return Ok(None);
        };
        if recipe.logical_path != self.layout.path() || start > end || end > self.source.len() {
            return Ok(None);
        }
        let (line, column) = line_column(&self.source, start);
        Ok(Some(RenderedSourceResult::Current(
            ResolvedSourceLocation {
                path: recipe.logical_path,
                start: start as u64,
                end: end as u64,
                line,
                column,
                excerpt: self.source[start..end].to_owned(),
            },
        )))
    }

    pub fn rendered_source_origin(
        &self,
        output: &AcceptedOutput,
        page: u32,
        event: u32,
        unit: Option<u32>,
    ) -> Result<Option<LayoutResolvedOrigin>, SessionError> {
        if output.output_id != self.output_id || output.revision != self.revision {
            return Ok(None);
        }
        let Some(ArtifactOrigin::Detached(recipe)) =
            self.rendered_artifact_origin(output, page, event, unit)?
        else {
            return Ok(None);
        };
        if recipe.logical_path != self.layout.path() {
            return Ok(Some(LayoutResolvedOrigin::Foreign));
        }
        let (Ok(start), Ok(end)) = (usize::try_from(recipe.start), usize::try_from(recipe.end))
        else {
            return Ok(Some(LayoutResolvedOrigin::Unknown));
        };
        if start > end || end > self.source.len() {
            return Ok(Some(LayoutResolvedOrigin::Unknown));
        }
        let (line, column) = line_column(&self.source, start);
        Ok(Some(LayoutResolvedOrigin::Current {
            path: recipe.logical_path,
            doc_offset_lo: start as u64,
            doc_offset_hi: end as u64,
            line,
            column,
        }))
    }

    fn rendered_artifact_origin(
        &self,
        output: &AcceptedOutput,
        page: u32,
        event: u32,
        unit: Option<u32>,
    ) -> Result<Option<ArtifactOrigin>, SessionError> {
        let Some(page_index) = page.checked_sub(1).map(|page| page as usize) else {
            return Ok(None);
        };
        let Some(artifact) = output
            .completion
            .pages()
            .get(page_index)
            .map(DetachedPreparedPage::artifact)
        else {
            return Ok(None);
        };
        let mut maps = self.render_maps.borrow_mut();
        if let Some(origin) = maps
            .pages
            .get(&page_index)
            .map(|map| map.origin(event, unit))
        {
            maps.touch(page_index);
            return Ok(origin);
        }
        let map = build_page_render_map(artifact, page)?;
        let origin = map.origin(event, unit);
        maps.admit(page_index, map);
        Ok(origin)
    }
}

fn source_file_bytes(source: &str, byte_projection: bool) -> Vec<u8> {
    if !byte_projection {
        return source.as_bytes().to_vec();
    }
    let mut bytes = Vec::with_capacity(source.len());
    for ch in source.chars() {
        if let Ok(byte) = u8::try_from(u32::from(ch)) {
            bytes.push(byte);
        } else {
            let mut encoded = [0; 4];
            bytes.extend_from_slice(ch.encode_utf8(&mut encoded).as_bytes());
        }
    }
    bytes
}

fn drive_synchronous_candidate(
    candidate: &mut RevisionCandidate<'_>,
    host: &mut dyn ResourceHost,
) -> Result<(), SessionError> {
    match candidate.drive_with_resource_resolvers(host, &Cancellation::new())? {
        RevisionCandidateResult::Complete => Ok(()),
        RevisionCandidateResult::AwaitingResources(need) => Err(SessionError::ResourceNoProgress {
            need: Box::new(need),
        }),
    }
}

fn map_step_failure(error: CanonicalStepFailure) -> SessionError {
    match error {
        CanonicalStepFailure::Execution(error) => SessionError::Execute(error),
        CanonicalStepFailure::Checkpoint(error) => SessionError::CommandSummary(error),
    }
}

fn dvi_bytes(completion: &DetachedEngineCompletion) -> Result<Vec<u8>, DviError> {
    let mut writer = DviStreamWriter::new(Vec::new());
    for page in completion.pages() {
        if let Some(plan) = page.dvi() {
            writer.write_page_plan(plan)?;
        }
    }
    writer.finish()
}

fn build_page_render_map(
    artifact: &CommittedArtifact,
    page: u32,
) -> Result<PageRenderMap, SessionError> {
    let page_artifact = tex_out::PageArtifact::from_bytes(artifact.bytes()).map_err(|error| {
        SessionError::RenderSource(format!("page {page} artifact decode failed: {error}"))
    })?;
    let positioned = tex_out::positioned::lower_page(&page_artifact, page)
        .map_err(|error| SessionError::RenderSource(error.to_string()))?;
    let mut event_units = Vec::with_capacity(positioned.events.len().saturating_add(1));
    let mut origins = Vec::new();
    event_units.push(0);
    for event in positioned.events {
        if let tex_out::positioned::PositionedEvent::TextRun(run) = event {
            for source in run.sources {
                origins.push(
                    source
                        .map(|source| {
                            artifact.render_origin(
                                source.node_ordinal as usize,
                                source.source_index as usize,
                            )
                        })
                        .unwrap_or(ArtifactOrigin::Unknown),
                );
            }
        }
        event_units.push(u32::try_from(origins.len()).map_err(|_| {
            SessionError::RenderSource("rendered source map exceeds u32 capacity".to_owned())
        })?);
    }
    Ok(PageRenderMap {
        event_units,
        origins,
    })
}

fn line_expanded_replacement(old: &str, edit: &Edit) -> (std::ops::Range<usize>, String) {
    let start = old.as_bytes()[..edit.range.start]
        .iter()
        .rposition(|byte| *byte == b'\n')
        .map_or(0, |position| position + 1);
    let end = if edit.range.start != edit.range.end
        && old.as_bytes().get(edit.range.end.wrapping_sub(1)) == Some(&b'\n')
    {
        edit.range.end
    } else {
        old.as_bytes()[edit.range.end..]
            .iter()
            .position(|byte| *byte == b'\n')
            .map_or(old.len(), |position| edit.range.end + position + 1)
    };
    let mut replacement = String::with_capacity(
        edit.range.start - start + edit.replacement.len() + end - edit.range.end,
    );
    replacement.push_str(&old[start..edit.range.start]);
    replacement.push_str(&edit.replacement);
    replacement.push_str(&old[edit.range.end..end]);
    (start..end, replacement)
}

fn clone_layout(
    layout: &EditorLayout,
    fragments: &FragmentStore,
) -> Result<EditorLayout, SessionError> {
    Ok(EditorLayout::new(
        layout.path(),
        layout.generation(),
        layout.pieces().to_vec(),
        fragments,
    )?)
}

fn line_column(source: &str, offset: usize) -> (u32, u32) {
    let prefix = &source.as_bytes()[..offset.min(source.len())];
    let line_start = prefix
        .iter()
        .rposition(|byte| *byte == b'\n')
        .map_or(0, |position| position + 1);
    let line = u32::try_from(prefix.iter().filter(|byte| **byte == b'\n').count())
        .unwrap_or(u32::MAX)
        .saturating_add(1);
    let column = u32::try_from(prefix.len().saturating_sub(line_start))
        .unwrap_or(u32::MAX)
        .saturating_add(1);
    (line, column)
}

fn replace_layout_range(
    old: &EditorLayout,
    fragments: &FragmentStore,
    replaced: std::ops::Range<usize>,
    replacement: tex_state::FragmentId,
    replacement_len: usize,
    generation: LayoutGeneration,
) -> Result<EditorLayout, SessionError> {
    let replaced_start = u64::try_from(replaced.start)
        .map_err(|_| SessionError::Layout(EditorLayoutError::DocumentTooLarge))?;
    let replaced_end = u64::try_from(replaced.end)
        .map_err(|_| SessionError::Layout(EditorLayoutError::DocumentTooLarge))?;
    let replacement_len = u32::try_from(replacement_len)
        .map_err(|_| SessionError::Layout(EditorLayoutError::DocumentTooLarge))?;
    let mut pieces = Vec::with_capacity(old.pieces().len().saturating_add(2));
    let mut inserted = false;
    for (index, piece) in old.pieces().iter().enumerate() {
        if piece.start() == piece.end() {
            continue;
        }
        let doc_start = old.doc_starts()[index];
        let doc_end = doc_start + u64::from(piece.end() - piece.start());
        if doc_end <= replaced_start {
            pieces.push(piece.clone());
            continue;
        }
        if doc_start >= replaced_end {
            if !inserted {
                pieces.push(Piece::new(replacement, 0, replacement_len));
                inserted = true;
            }
            pieces.push(piece.clone());
            continue;
        }
        if doc_start < replaced_start {
            let left_end = piece.start()
                + u32::try_from(replaced_start - doc_start)
                    .map_err(|_| SessionError::Layout(EditorLayoutError::DocumentTooLarge))?;
            pieces.push(Piece::new(piece.fragment(), piece.start(), left_end));
        }
        if !inserted {
            pieces.push(Piece::new(replacement, 0, replacement_len));
            inserted = true;
        }
        if doc_end > replaced_end {
            let right_start = piece.start()
                + u32::try_from(replaced_end - doc_start)
                    .map_err(|_| SessionError::Layout(EditorLayoutError::DocumentTooLarge))?;
            pieces.push(Piece::new(piece.fragment(), right_start, piece.end()));
        }
    }
    if !inserted {
        pieces.push(Piece::new(replacement, 0, replacement_len));
    }
    Ok(EditorLayout::new(
        old.path(),
        generation,
        pieces,
        fragments,
    )?)
}

fn detached_output_bytes(completion: &DetachedEngineCompletion) -> usize {
    completion
        .effects()
        .iter()
        .map(EffectRecord::retained_bytes)
        .sum::<usize>()
        .saturating_add(
            completion
                .pages()
                .iter()
                .map(|page| {
                    let artifact = page.artifact();
                    artifact
                        .bytes()
                        .len()
                        .saturating_add(artifact.render_provenance_bytes())
                })
                .sum::<usize>(),
        )
}

fn resolve_frozen(
    frozen: &tex_exec::FrozenDiagnosticOrigin,
    source: &str,
    layout: &EditorLayout,
) -> Option<ResolvedSourceLocation> {
    match frozen {
        tex_exec::FrozenDiagnosticOrigin::Resolved(location) => Some(location.clone()),
        tex_exec::FrozenDiagnosticOrigin::Generated { fallback, .. } => Some(fallback.clone()),
        tex_exec::FrozenDiagnosticOrigin::Root(_) => Some(ResolvedSourceLocation {
            path: layout.path().to_owned(),
            start: 0,
            end: 0,
            line: 1,
            column: 1,
            excerpt: source.lines().next().unwrap_or_default().to_owned(),
        }),
    }
}

struct DirectResourceHost;

impl ResourceHost for DirectResourceHost {
    fn fulfill(&mut self, world: &mut ResourceWorld<'_>, need: &ResourceNeed) -> ResourceOutcome {
        match need {
            ResourceNeed::Input { name, .. } => world.read_file(Path::new(name)).ok().map_or(
                ResourceOutcome::Unavailable,
                |content| {
                    ResourceOutcome::Fulfilled(ResourceFulfillment::world_input(name, content))
                },
            ),
            ResourceNeed::InputProbe { request } => world
                .read_file(Path::new(&request.name))
                .ok()
                .map_or(ResourceOutcome::Unavailable, |content| {
                    ResourceOutcome::Fulfilled(ResourceFulfillment::world_input_probe(
                        request.clone(),
                        content,
                    ))
                }),
            ResourceNeed::Font { request } => world
                .read_file(canonical_font_resource_path(&request.name))
                .ok()
                .map_or(ResourceOutcome::Unavailable, |metrics| {
                    ResourceOutcome::Fulfilled(ResourceFulfillment::Font {
                        request: request.clone(),
                        resource: Box::new(tex_command::FontResource::Tfm {
                            metrics,
                            opentype: None,
                        }),
                    })
                }),
            ResourceNeed::PdfImage { .. } => ResourceOutcome::Unavailable,
        }
    }
}

struct Timer {
    #[cfg(not(target_arch = "wasm32"))]
    started: Instant,
}

impl Timer {
    #[allow(clippy::disallowed_methods)]
    fn start() -> Self {
        Self {
            #[cfg(not(target_arch = "wasm32"))]
            started: Instant::now(),
        }
    }

    fn elapsed(&self) -> Duration {
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.started.elapsed()
        }
        #[cfg(target_arch = "wasm32")]
        {
            Duration::ZERO
        }
    }
}

/// Errors at the generation-free editor/session boundary.
#[derive(Debug)]
pub enum SessionError {
    EngineCompletion(tex_exec::EngineCompletionError),
    OutputIdentity(getrandom::Error),
    StaleRevision {
        expected: RevisionId,
        actual: RevisionId,
    },
    ContentHashMismatch,
    NonMonotonicRevision,
    InvalidEditRange,
    CandidateKindMismatch,
    CandidateAlreadyLive,
    CandidateNotComplete,
    MissingJobStartAnchor,
    JobStartSessionMismatch,
    UnexpectedResource,
    ResourceNoProgress {
        need: Box<ResourceNeed>,
    },
    SourceRegistration(SourceRegistrationError),
    CommandSummary(tex_command::CommandSummaryError),
    Execute(tex_exec::ExecError),
    Format(tex_state::FormatError),
    FormatDump(tex_exec::FormatDumpError),
    FormatFontPolicy {
        name: String,
    },
    World(WorldError),
    State(tex_state::StateError),
    Epoch(tex_state::SessionEpochError),
    RetainedEngine(tex_exec::RetainedEngineAccessError),
    RetainedEngineFork(tex_exec::RetainedEngineForkError),
    Universe(tex_state::UniverseError),
    Fragment(tex_state::source_map::SourceMapError),
    Layout(EditorLayoutError),
    RenderSource(String),
}

impl fmt::Display for SessionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EngineCompletion(error) => {
                write!(f, "incremental terminal completion failed: {error}")
            }
            Self::OutputIdentity(error) => write!(f, "could not create output identity: {error}"),
            Self::StaleRevision { expected, actual } => write!(
                f,
                "edit targets stale revision {} (accepted revision is {})",
                actual.raw(),
                expected.raw()
            ),
            Self::ContentHashMismatch => f.write_str("edit base content hash does not match"),
            Self::NonMonotonicRevision => f.write_str("new revision id must increase"),
            Self::InvalidEditRange => f.write_str("edit range is outside UTF-8 boundaries"),
            Self::CandidateKindMismatch => f.write_str("candidate belongs to another session"),
            Self::CandidateAlreadyLive => {
                f.write_str("session already has a live revision candidate")
            }
            Self::CandidateNotComplete => f.write_str("revision candidate is not complete"),
            Self::MissingJobStartAnchor => {
                f.write_str("accepted session has no frozen JobStart anchor")
            }
            Self::JobStartSessionMismatch => {
                f.write_str("frozen JobStart anchor belongs to different session semantics")
            }
            Self::UnexpectedResource => f.write_str("resource fulfillment does not match"),
            Self::ResourceNoProgress { need, .. } => {
                write!(f, "resource replay made no progress for {need:?}")
            }
            Self::SourceRegistration(error) => write!(f, "source registration failed: {error}"),
            Self::CommandSummary(error) => write!(f, "checkpoint failed: {error}"),
            Self::Execute(error) => write!(f, "incremental execution failed: {error}"),
            Self::Format(error) => write!(f, "incremental format materialization failed: {error}"),
            Self::FormatDump(error) => write!(f, "incremental format capture failed: {error}"),
            Self::FormatFontPolicy { name } => write!(
                f,
                "format preloads classic TFM font {name}; OpenTypePreferred requires fonts to be selected through typed resources before layout"
            ),
            Self::World(error) => write!(f, "incremental world failed: {error}"),
            Self::State(error) => write!(f, "incremental generation failed: {error:?}"),
            Self::Epoch(error) => write!(f, "incremental session epoch failed: {error:?}"),
            Self::RetainedEngine(error) => {
                write!(f, "incremental retained generation failed: {error:?}")
            }
            Self::RetainedEngineFork(error) => {
                write!(f, "incremental checkpoint fork failed: {error}")
            }
            Self::Universe(error) => write!(f, "incremental runtime setup failed: {error:?}"),
            Self::Fragment(error) => write!(f, "editor fragment allocation failed: {error}"),
            Self::Layout(error) => write!(f, "editor layout update failed: {error}"),
            Self::RenderSource(error) => write!(f, "rendered source query failed: {error}"),
        }
    }
}

impl std::error::Error for SessionError {}

impl SessionError {
    #[must_use]
    pub fn frozen_diagnostic_origin(&self) -> Option<&tex_exec::FrozenDiagnosticOrigin> {
        match self {
            Self::Execute(error) => error.frozen_diagnostic_origin(),
            _ => None,
        }
    }

    #[must_use]
    pub fn frozen_diagnostic_context(&self) -> Option<&tex_exec::FrozenDiagnosticContext> {
        match self {
            Self::Execute(error) => error.frozen_diagnostic_context(),
            _ => None,
        }
    }
}

impl From<tex_exec::ExecError> for SessionError {
    fn from(value: tex_exec::ExecError) -> Self {
        Self::Execute(value)
    }
}

impl From<tex_exec::EngineCompletionError> for SessionError {
    fn from(value: tex_exec::EngineCompletionError) -> Self {
        Self::EngineCompletion(value)
    }
}

impl From<tex_command::CommandSummaryError> for SessionError {
    fn from(value: tex_command::CommandSummaryError) -> Self {
        Self::CommandSummary(value)
    }
}

impl From<SourceRegistrationError> for SessionError {
    fn from(value: SourceRegistrationError) -> Self {
        Self::SourceRegistration(value)
    }
}

impl From<WorldError> for SessionError {
    fn from(value: WorldError) -> Self {
        Self::World(value)
    }
}

impl From<tex_state::source_map::SourceMapError> for SessionError {
    fn from(value: tex_state::source_map::SourceMapError) -> Self {
        Self::Fragment(value)
    }
}

impl From<EditorLayoutError> for SessionError {
    fn from(value: EditorLayoutError) -> Self {
        Self::Layout(value)
    }
}

#[cfg(test)]
mod retained_generation_tests {
    use super::*;

    #[test]
    fn rejected_transaction_leaves_detached_session_unchanged() {
        let session_store = new_reachability_store();
        let foreign_store = new_reachability_store();
        let mut session =
            Session::start(&session_store, "reject", RevisionId::new(1), "\\end", 1024)
                .expect("session");
        let before = session.content_hash();
        let mut foreign =
            Session::start(&foreign_store, "foreign", RevisionId::new(1), "\\end", 1024)
                .expect("foreign session");
        let mut candidate = foreign.start_cold_candidate().expect("candidate");
        drive_synchronous_candidate(&mut candidate, &mut DirectResourceHost).expect("drive");
        let transaction = session
            .prepare_revision_candidate(candidate)
            .expect("transaction");
        assert!(matches!(
            session.accept_revision(transaction),
            Err(SessionError::CandidateKindMismatch)
        ));
        assert_eq!(session.content_hash(), before);
    }

    #[test]
    fn accepted_history_contains_no_runtime_checkpoint() {
        let store = new_reachability_store();
        let mut session =
            Session::start(&store, "history", RevisionId::new(1), "\\end", 1024).expect("session");
        session.cold().expect("cold run");
        assert!(!session.history().is_empty());
        assert!(
            session
                .history()
                .iter()
                .all(|record| record.revision() == RevisionId::new(1))
        );
    }
}

#[cfg(test)]
mod tests;
