//! Named-boundary incremental editor sessions.
//!
//! Phase 6 deliberately keeps accepted editor state generation-free. A drive
//! runs inside one freshly branded runtime generation, and publishes only
//! detached output plus allocation-independent boundary observations. Coarse
//! revision-generation retention and checkpoint relocation belong to phase 7.

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
    Cancellation, CanonicalStepFailure, CanonicalStepResult, CanonicalStepRunner,
    CheckpointIdentity, CheckpointSink, DetachedEngineCompletion, DetachedFormatDump,
    DetachedPreparedPage, EngineBoundary, EngineCheckpoint, EngineCompletionDemand, MainControl,
    MainControlStep, OutputLedger, ResourceFulfillment, ResourceHost, ResourceNeed,
    ResourceOutcome, ResourceWorld, canonical_font_resource_path,
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

mod trace;

pub use trace::{TraceCompositionError, TraceOperation, TraceSummary, TraceValidationError};

const SESSION_INTERNER_NAMES: u32 = 65_536;
const SESSION_INTERNER_SLOTS: u32 = 131_072;
const SESSION_INTERNER_BYTES: u32 = 16 * 1024 * 1024;

fn session_interner_budget() -> InternerBudget {
    InternerBudget::new(
        SESSION_INTERNER_NAMES,
        SESSION_INTERNER_SLOTS,
        SESSION_INTERNER_BYTES,
    )
    .expect("the incremental session interner budget is valid")
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

/// Executor-owned occurrence key for one named boundary.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct BoundaryKey {
    pub position: usize,
    pub boundary: EngineBoundary,
    pub ordinal: u32,
}

/// Handle-free accepted observation of one named runtime boundary.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BoundaryRecord {
    revision: RevisionId,
    key: BoundaryKey,
    effect_prefix: usize,
    artifact_prefix: usize,
    state_hash: u64,
}

impl BoundaryRecord {
    #[must_use]
    pub const fn revision(&self) -> RevisionId {
        self.revision
    }

    #[must_use]
    pub const fn key(&self) -> BoundaryKey {
        self.key
    }

    #[must_use]
    pub const fn artifact_prefix(&self) -> usize {
        self.artifact_prefix
    }

    #[must_use]
    pub const fn effect_prefix(&self) -> usize {
        self.effect_prefix
    }

    #[must_use]
    pub const fn state_hash(&self) -> u64 {
        self.state_hash
    }
}

/// Honest split between restart observations and detached accepted output.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct RetentionMetrics {
    pub checkpoint_root_bytes: usize,
    pub memo_result_bytes: usize,
    pub diagnostic_bytes: usize,
    pub output_bytes: usize,
    pub protected_overage_bytes: usize,
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
/// The completion is the sole owner of accepted effects, artifacts, DVI page
/// plans, and PDF state.  Session metadata remains generation-free and never
/// duplicates those output families.
#[derive(Debug)]
pub struct AcceptedOutput {
    output_id: RenderedOutputId,
    pub revision: RevisionId,
    pub content_hash: ContentHash,
    completion: DetachedEngineCompletion,
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
    pub const fn completion(&self) -> &DetachedEngineCompletion {
        &self.completion
    }

    #[must_use]
    pub fn pages(&self) -> &[DetachedPreparedPage] {
        self.completion.pages()
    }

    #[must_use]
    pub const fn pdf(&self) -> Option<&tex_state::DetachedPdfCompletion> {
        self.completion.pdf()
    }

    pub fn into_completion(self) -> DetachedEngineCompletion {
        self.completion
    }

    #[must_use]
    pub const fn format_dump(&self) -> Option<&DetachedFormatDump> {
        self.format_dump.as_ref()
    }

    pub fn into_terminal(self) -> (DetachedEngineCompletion, Option<DetachedFormatDump>) {
        (self.completion, self.format_dump)
    }

    pub fn dvi_bytes(&self) -> Result<Vec<u8>, DviError> {
        dvi_bytes(&self.completion)
    }
}

/// Borrowed terminal resource-discovery view.
///
/// Construction is possible only after a candidate reaches terminal
/// completion.  Every exposed value is owned by the detached completion; the
/// runtime generation has already been dropped.
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
pub struct RevisionTransaction {
    session_output_id: RenderedOutputId,
    base_revision: RevisionId,
    base_content_hash: ContentHash,
    revision: RevisionId,
    source: String,
    fragments: FragmentStore,
    layout: EditorLayout,
    content_hash: ContentHash,
    completion: DetachedEngineCompletion,
    history: Vec<BoundaryRecord>,
    dependencies: Vec<tex_state::InputDependency>,
    reuse: ReuseMetrics,
    format_dump: Option<DetachedFormatDump>,
    expansion_stats: ExpansionStats,
}

impl RevisionTransaction {
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
}

struct CandidatePlan {
    base_revision: RevisionId,
    base_content_hash: ContentHash,
    revision: RevisionId,
    source: String,
    fragments: FragmentStore,
    layout: EditorLayout,
    old_history: Vec<BoundaryRecord>,
    execution_path: RevisionExecutionPath,
    revision_setup_latency: Duration,
}

struct CandidateCompletion {
    completion: DetachedEngineCompletion,
    history: Vec<BoundaryRecord>,
    dependencies: Vec<tex_state::InputDependency>,
    delivered_commands: usize,
    format_dump: Option<DetachedFormatDump>,
}

/// Handle-free plan for one revision execution.
///
/// A resource suspension retains only this plan and detached host state. The
/// next drive safely recomputes inside a fresh generation; no runtime id or
/// owner crosses the suspension boundary.
pub struct RevisionCandidate {
    session_output_id: RenderedOutputId,
    job_name: String,
    source_path: String,
    plan: CandidatePlan,
    registered_inputs: BTreeMap<PathBuf, Vec<u8>>,
    profile: CommandProfile,
    initex: bool,
    dvi_output: bool,
    root_framing: SourceFramingPolicy,
    root_framing_name: Option<String>,
    root_source_is_byte_projection: bool,
    format_image: Option<DetachedFormatImage>,
    job_clock: JobClock,
    completed: Option<CandidateCompletion>,
    cumulative_fuel_limit: u64,
    execution_budgets: tex_exec::ExecutionBudgets,
    suspension_serial: u64,
    advance_calls: u64,
    cumulative_fuel: u64,
}

/// Result of driving a revision until it suspends or completes.
#[derive(Clone, Debug)]
pub enum RevisionCandidateResult {
    AwaitingResources(ResourceNeed),
    Complete,
}

impl RevisionCandidate {
    pub fn drive_with_resource_resolvers(
        &mut self,
        host: &mut dyn ResourceHost,
        cancellation: &Cancellation,
    ) -> Result<RevisionCandidateResult, SessionError> {
        if self.completed.is_some() {
            return Ok(RevisionCandidateResult::Complete);
        }
        let result = if let Some(image) = &self.format_image {
            tex_state::with_materialized_format(
                session_interner_budget(),
                World::memory_with_clock(self.job_clock),
                image,
                |universe| execute_plan(universe, self, host, cancellation),
            )
            .map_err(SessionError::Format)??
        } else {
            tex_state::with_universe(session_interner_budget(), |universe| {
                *universe.world_mut() = World::memory_with_clock(self.job_clock);
                execute_plan(universe, self, host, cancellation)
            })
            .map_err(SessionError::State)??
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
                self.completed = Some(completion);
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
            checkpoint_root_bytes: 0,
            memo_result_bytes: 0,
            diagnostic_bytes,
            output_bytes,
            protected_overage_bytes: 0,
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
}

enum PlanExecution {
    Suspended(ResourceNeed),
    Complete(CandidateCompletion, u64),
}

struct LiveHistorySink<G> {
    revision: RevisionId,
    records: Vec<BoundaryRecord>,
    occurrences: HashMap<(usize, EngineBoundary), u32>,
    paragraphs: usize,
    marker: std::marker::PhantomData<fn(G) -> G>,
}

impl<G> LiveHistorySink<G> {
    fn new(revision: RevisionId) -> Self {
        Self {
            revision,
            records: Vec::new(),
            occurrences: HashMap::new(),
            paragraphs: 0,
            marker: std::marker::PhantomData,
        }
    }
}

impl<G> CheckpointSink<G> for LiveHistorySink<G> {
    fn wants_exact_state_identity(&self, _boundary: EngineBoundary, _root_anchor: usize) -> bool {
        true
    }

    fn checkpoint(&mut self, checkpoint: EngineCheckpoint<G>) {
        let position = checkpoint.root_anchor();
        let boundary = checkpoint.boundary();
        if boundary == EngineBoundary::OuterParagraphEnd {
            self.paragraphs = self.paragraphs.saturating_add(1);
        }
        let ordinal = self.occurrences.entry((position, boundary)).or_default();
        self.records.push(BoundaryRecord {
            revision: self.revision,
            key: BoundaryKey {
                position,
                boundary,
                ordinal: *ordinal,
            },
            effect_prefix: checkpoint.effect_prefix_len(),
            artifact_prefix: checkpoint.artifact_prefix_len(),
            state_hash: checkpoint.mode_hash(),
        });
        *ordinal = ordinal.saturating_add(1);
    }
}

fn execute_plan<G>(
    universe: &mut Universe<G>,
    candidate: &RevisionCandidate,
    host: &mut dyn ResourceHost,
    cancellation: &Cancellation,
) -> Result<PlanExecution, SessionError> {
    universe.begin_retained_session()?;
    universe.set_interaction_mode(tex_state::InteractionMode::Nonstop);
    if candidate.format_image.is_none() {
        install_plain_catcodes(universe)?;
    } else {
        register_materialized_primitives(universe, candidate.profile);
    }
    for (path, bytes) in &candidate.registered_inputs {
        universe.world_mut().set_memory_file(path, bytes.clone())?;
    }
    let mut control = candidate_control(
        universe,
        CandidateControlOptions {
            job_name: &candidate.job_name,
            source_path: &candidate.source_path,
            bytes: source_file_bytes(
                &candidate.plan.source,
                candidate.root_source_is_byte_projection,
            ),
            profile: candidate.profile,
            initex: candidate.initex,
            emit_dvi: candidate.dvi_output,
            root_framing: candidate.root_framing,
            root_framing_name: candidate.root_framing_name.as_deref(),
        },
    )?;
    control
        .set_fuel_limit(candidate.cumulative_fuel_limit)
        .expect("candidate fuel limit is positive");
    control.attach_pure_memo_capability(universe);
    let mut sink = LiveHistorySink::new(candidate.plan.revision);
    let mut ledger = OutputLedger::new(CheckpointIdentity::Exact);
    ledger.commit_job_start(&mut control, universe, &mut sink)?;
    let mut delivered_commands = 0usize;
    let mut answered_needs = Vec::new();
    loop {
        if cancellation.is_cancelled() {
            return Err(SessionError::Execute(
                tex_exec::ExecError::ExecutionCancelled,
            ));
        }
        let attempted = u64::try_from(delivered_commands).unwrap_or(u64::MAX);
        if attempted > candidate.execution_budgets.steps {
            return Err(SessionError::Execute(
                tex_exec::ExecError::ResourceBudgetExceeded {
                    resource: "steps",
                    limit: candidate.execution_budgets.steps,
                    attempted,
                },
            ));
        }
        match CanonicalStepRunner::new(&mut control, universe, &mut ledger)
            .step(&mut sink, cancellation)
        {
            CanonicalStepResult::Progress(step)
            | CanonicalStepResult::Committed(step)
            | CanonicalStepResult::Completed(step) => {
                answered_needs.clear();
                delivered_commands = delivered_commands.saturating_add(1);
                if matches!(step, MainControlStep::End | MainControlStep::EndOfInput) {
                    let dependencies = universe.world().input_dependencies().cloned().collect();
                    let format_dump = control
                        .take_format_dump(universe)
                        .map_err(SessionError::FormatDump)?;
                    let completion = ledger.close_revision(
                        &mut control,
                        universe,
                        EngineCompletionDemand::new(
                            candidate.profile.dialect() == tex_command::CommandDialect::Pdftex14029,
                        ),
                    )?;
                    return Ok(PlanExecution::Complete(
                        CandidateCompletion {
                            completion,
                            history: sink.records,
                            dependencies,
                            delivered_commands,
                            format_dump,
                        },
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
                            .fulfill(&mut control, &need, fulfillment)
                            .map_err(|_| SessionError::UnexpectedResource)?;
                        answered_needs.push(need);
                    }
                    ResourceOutcome::Unavailable => {
                        ledger.mark_unavailable(&mut control, &need, false);
                        answered_needs.push(need);
                    }
                    ResourceOutcome::Declined => return Ok(PlanExecution::Suspended(need)),
                }
            }
            CanonicalStepResult::Failed(error) => return Err(map_step_failure(error)),
        }
    }
}

fn register_materialized_primitives<G>(universe: &mut Universe<G>, profile: CommandProfile) {
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
    bytes: Vec<u8>,
    profile: CommandProfile,
    initex: bool,
    emit_dvi: bool,
    root_framing: SourceFramingPolicy,
    root_framing_name: Option<&'a str>,
}

fn candidate_control<G>(
    universe: &mut Universe<G>,
    options: CandidateControlOptions<'_>,
) -> Result<MainControl<G>, SessionError> {
    if options.initex {
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
    }
    let mut control = if options.initex {
        MainControl::prepared_initex(options.profile)
    } else {
        MainControl::with_profile(options.profile)
    };
    if options.profile.dialect() == tex_command::CommandDialect::Pdftex14029 {
        control.set_engine_binary(tex_exec::EngineBinaryIdentity::Pdftex14029);
    }
    control.set_dvi_output(options.emit_dvi);
    let mut registration = SourceRegistration::new(RegisteredSourceKind::Generated, options.bytes)
        .with_name(options.source_path)
        .with_framing(options.root_framing);
    if let Some(name) = options.root_framing_name {
        registration = registration.with_framing_name(name);
    }
    control.register_root_source(registration)?;
    control.flush_pending_file_framing(universe);
    control
        .capabilities_mut()
        .set_startup_job_name(options.job_name);
    Ok(control)
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

/// Long-lived incremental session containing no runtime generation handle.
pub struct Session {
    job_name: String,
    revision: RevisionId,
    output_id: RenderedOutputId,
    source: String,
    fragments: FragmentStore,
    layout: EditorLayout,
    content_hash: ContentHash,
    history: Vec<BoundaryRecord>,
    dependencies: Vec<tex_state::InputDependency>,
    checkpoint_budget: usize,
    registered_inputs: BTreeMap<PathBuf, Vec<u8>>,
    accepted_retention: Option<RetentionMetrics>,
    format_image: Option<DetachedFormatImage>,
    job_clock: JobClock,
    utf8_input_as_bytes: bool,
    dvi_output: bool,
    root_framing: SourceFramingPolicy,
    root_framing_name: Option<String>,
    root_source_is_byte_projection: bool,
    command_profile: CommandProfile,
    initex: bool,
    expansion_stats: ExpansionStats,
    render_maps: RefCell<RenderMapCache>,
}

impl Session {
    /// Starts a generation-free editor session.
    ///
    /// `template` is accepted only as a migration-time configuration marker;
    /// live runtime state is never retained or cloned from it.
    pub fn start<T>(
        _template: T,
        job_name: impl Into<String>,
        revision: RevisionId,
        source: impl Into<String>,
        checkpoint_budget: usize,
    ) -> Result<Self, SessionError> {
        Self::start_with_source_path(
            (),
            job_name,
            "<editor>",
            revision,
            source,
            checkpoint_budget,
        )
    }

    pub fn start_with_source_path<T>(
        _template: T,
        job_name: impl Into<String>,
        source_path: impl Into<String>,
        revision: RevisionId,
        source: impl Into<String>,
        checkpoint_budget: usize,
    ) -> Result<Self, SessionError> {
        Self::start_with_prepared_source(
            job_name,
            source_path,
            revision,
            source.into(),
            false,
            checkpoint_budget,
        )
    }

    pub fn start_with_source_bytes<T>(
        _template: T,
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
            job_name,
            source_path,
            revision,
            source,
            byte_projection,
            checkpoint_budget,
        )
    }

    fn start_with_prepared_source(
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
            format_image: None,
            job_clock: JobClock::default(),
            utf8_input_as_bytes: false,
            dvi_output: true,
            root_framing: SourceFramingPolicy::Canonical,
            root_framing_name: None,
            root_source_is_byte_projection,
            command_profile: CommandProfile::TEX82,
            initex: true,
            expansion_stats: ExpansionStats::default(),
            render_maps: RefCell::new(RenderMapCache::new(usize::MAX)),
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

    /// Selects one validated, handle-free format image for every candidate.
    /// Each drive materializes it inside a fresh generation and drops that
    /// generation before retaining any candidate or accepted state.
    pub fn set_format_image(&mut self, image: DetachedFormatImage) {
        assert!(self.history.is_empty(), "format is fixed after execution");
        self.format_image = Some(image);
        self.initex = false;
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

    pub fn register_input_file(&mut self, path: &Path, bytes: Vec<u8>) -> Result<(), SessionError> {
        self.registered_inputs.insert(path.to_owned(), bytes);
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

    pub fn start_cold_candidate(&self) -> Result<RevisionCandidate, SessionError> {
        self.candidate(CandidatePlan {
            base_revision: self.revision,
            base_content_hash: self.content_hash,
            revision: self.revision,
            source: self.source.clone(),
            fragments: self.fragments.clone(),
            layout: clone_layout(&self.layout, &self.fragments)?,
            old_history: Vec::new(),
            execution_path: RevisionExecutionPath::Cold,
            revision_setup_latency: Duration::ZERO,
        })
    }

    pub fn accept_cold_candidate(
        &mut self,
        candidate: RevisionCandidate,
    ) -> Result<AcceptedOutput, SessionError> {
        let transaction = self.prepare_revision_candidate(candidate)?;
        self.accept_revision(transaction)
    }

    pub fn start_advance_candidate(
        &self,
        next_revision: RevisionId,
        edit: Edit,
    ) -> Result<RevisionCandidate, SessionError> {
        self.start_advance_candidate_with_path(next_revision, edit, RevisionExecutionPath::SlowEdit)
    }

    pub fn start_advance_candidate_from_job_start(
        &self,
        next_revision: RevisionId,
        edit: Edit,
    ) -> Result<RevisionCandidate, SessionError> {
        self.start_advance_candidate_with_path(
            next_revision,
            edit,
            RevisionExecutionPath::ForcedJobStartFallback,
        )
    }

    pub fn start_external_input_delta_candidate(&self) -> Result<RevisionCandidate, SessionError> {
        self.candidate(CandidatePlan {
            base_revision: self.revision,
            base_content_hash: self.content_hash,
            revision: self.revision,
            source: self.source.clone(),
            fragments: self.fragments.clone(),
            layout: clone_layout(&self.layout, &self.fragments)?,
            old_history: self.history.clone(),
            execution_path: RevisionExecutionPath::ExternalInputDelta,
            revision_setup_latency: Duration::ZERO,
        })
    }

    fn start_advance_candidate_with_path(
        &self,
        next_revision: RevisionId,
        edit: Edit,
        execution_path: RevisionExecutionPath,
    ) -> Result<RevisionCandidate, SessionError> {
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
            old_history: self.history.clone(),
            execution_path,
            revision_setup_latency: started.elapsed(),
        })
    }

    fn candidate(&self, plan: CandidatePlan) -> Result<RevisionCandidate, SessionError> {
        let format_image = self
            .format_image
            .as_ref()
            .map(|image| DetachedFormatImage::try_from_bytes(image.as_bytes().to_vec()))
            .transpose()
            .map_err(SessionError::Format)?;
        Ok(RevisionCandidate {
            session_output_id: self.output_id,
            job_name: self.job_name.clone(),
            source_path: plan.layout.path().to_owned(),
            plan,
            registered_inputs: self.registered_inputs.clone(),
            profile: if self.utf8_input_as_bytes || self.root_source_is_byte_projection {
                self.command_profile
            } else {
                CommandProfile::unicode_extended(self.command_profile.dialect())
            },
            initex: self.initex,
            dvi_output: self.dvi_output,
            root_framing: self.root_framing,
            root_framing_name: self.root_framing_name.clone(),
            root_source_is_byte_projection: self.root_source_is_byte_projection,
            format_image,
            job_clock: self.job_clock,
            completed: None,
            cumulative_fuel_limit: MainControl::<GenerationBrand<'static>>::DEFAULT_FUEL_LIMIT,
            execution_budgets: tex_exec::ExecutionBudgets::default(),
            suspension_serial: 0,
            advance_calls: 0,
            cumulative_fuel: 0,
        })
    }

    pub fn prepare_revision_candidate(
        &mut self,
        mut candidate: RevisionCandidate,
    ) -> Result<RevisionTransaction, SessionError> {
        let completion = candidate
            .completed
            .take()
            .ok_or(SessionError::CandidateNotComplete)?;
        let reuse = compare_histories(
            candidate.plan.execution_path,
            &candidate.plan.old_history,
            &completion.history,
            candidate.plan.base_content_hash
                == ContentHash::from_bytes(candidate.plan.source.as_bytes()),
            candidate.plan.source.len(),
            completion.delivered_commands,
            candidate.plan.revision_setup_latency,
            completion.completion.pages().len(),
        );
        Ok(RevisionTransaction {
            session_output_id: candidate.session_output_id,
            base_revision: candidate.plan.base_revision,
            base_content_hash: candidate.plan.base_content_hash,
            revision: candidate.plan.revision,
            content_hash: ContentHash::from_bytes(candidate.plan.source.as_bytes()),
            source: candidate.plan.source,
            fragments: candidate.plan.fragments,
            layout: candidate.plan.layout,
            completion: completion.completion,
            history: completion.history,
            dependencies: completion.dependencies,
            reuse,
            format_dump: completion.format_dump,
            expansion_stats: ExpansionStats::default(),
        })
    }

    pub fn accept_revision(
        &mut self,
        transaction: RevisionTransaction,
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
        let acceptance = Timer::start();
        self.revision = transaction.revision;
        self.source = transaction.source;
        self.fragments = transaction.fragments;
        self.layout = transaction.layout;
        self.content_hash = transaction.content_hash;
        self.history = prune_history(transaction.history, self.checkpoint_budget);
        self.dependencies = transaction.dependencies;
        self.expansion_stats = transaction.expansion_stats;
        self.render_maps.borrow_mut().clear();
        let output_bytes = detached_output_bytes(&transaction.completion);
        let retention = RetentionMetrics {
            checkpoint_root_bytes: std::mem::size_of_val(self.history.as_slice()),
            memo_result_bytes: 0,
            diagnostic_bytes: self
                .fragments
                .retained_bytes()
                .saturating_add(self.layout.retained_bytes()),
            output_bytes,
            protected_overage_bytes: 0,
        };
        self.accepted_retention = Some(retention);
        let mut reuse = transaction.reuse;
        reuse.acceptance_latency = acceptance.elapsed();
        Ok(AcceptedOutput {
            output_id: self.output_id,
            revision: self.revision,
            content_hash: self.content_hash,
            completion: transaction.completion,
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
    ) -> Result<RevisionTransaction, SessionError> {
        let mut candidate = self.start_advance_candidate(next_revision, edit)?;
        drive_synchronous_candidate(&mut candidate, host)?;
        self.prepare_revision_candidate(candidate)
    }

    pub fn prepare_revision_with_resource_resolvers(
        &mut self,
        next_revision: RevisionId,
        edit: Edit,
        host: &mut dyn ResourceHost,
    ) -> Result<RevisionTransaction, SessionError> {
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

fn compare_histories(
    execution_path: RevisionExecutionPath,
    old: &[BoundaryRecord],
    new: &[BoundaryRecord],
    unchanged_content: bool,
    source_len: usize,
    delivered_commands: usize,
    revision_setup_latency: Duration,
    pages_retyped: usize,
) -> ReuseMetrics {
    if execution_path == RevisionExecutionPath::Cold || old.is_empty() {
        return ReuseMetrics {
            execution_path,
            pages_retyped,
            reexecuted_bytes: source_len,
            reexecuted_tokens: delivered_commands,
            reexecuted_commands: delivered_commands,
            reexecuted_paragraphs: new
                .iter()
                .filter(|record| record.key.boundary == EngineBoundary::OuterParagraphEnd)
                .count(),
            revision_setup_latency,
            trace_retained_bytes: std::mem::size_of_val(new),
            ..ReuseMetrics::default()
        };
    }
    if !unchanged_content {
        return ReuseMetrics {
            execution_path,
            pages_retyped,
            reexecuted_bytes: source_len,
            reexecuted_tokens: delivered_commands,
            reexecuted_commands: delivered_commands,
            reexecuted_paragraphs: new
                .iter()
                .filter(|record| record.key.boundary == EngineBoundary::OuterParagraphEnd)
                .count(),
            same_history_stop: SameHistoryStop::HashesDiverged,
            revision_setup_latency,
            trace_retained_bytes: std::mem::size_of_val(new),
            ..ReuseMetrics::default()
        };
    }
    let started = Timer::start();
    let mut attempts = 0usize;
    let mut mismatches = 0usize;
    let mut convergence = None;
    for (old_record, new_record) in old.iter().zip(new) {
        if old_record.key.boundary != new_record.key.boundary {
            continue;
        }
        attempts = attempts.saturating_add(1);
        if old_record.key == new_record.key && old_record.state_hash == new_record.state_hash {
            convergence.get_or_insert(new_record.key);
        } else {
            mismatches = mismatches.saturating_add(1);
        }
    }
    ReuseMetrics {
        execution_path,
        convergence_boundary: convergence,
        pages_retyped,
        reexecuted_bytes: source_len,
        reexecuted_tokens: delivered_commands,
        reexecuted_commands: delivered_commands,
        reexecuted_paragraphs: new
            .iter()
            .filter(|record| record.key.boundary == EngineBoundary::OuterParagraphEnd)
            .count(),
        same_history_attempts: attempts,
        same_history_hash_mismatches: mismatches,
        trace_nodes_walked: attempts,
        trace_retained_bytes: std::mem::size_of_val(new),
        same_history_stop: if convergence.is_some() {
            SameHistoryStop::Matched
        } else if attempts == 0 {
            SameHistoryStop::NoComparableBoundary
        } else {
            SameHistoryStop::HashesDiverged
        },
        revision_setup_latency,
        trace_validation_latency: started.elapsed(),
        ..ReuseMetrics::default()
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
    candidate: &mut RevisionCandidate,
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

fn prune_history(mut history: Vec<BoundaryRecord>, budget: usize) -> Vec<BoundaryRecord> {
    while std::mem::size_of_val(history.as_slice()) > budget && history.len() > 2 {
        let newest = history.len() - 1;
        let victim = history
            .iter()
            .enumerate()
            .find(|(index, record)| {
                *index != 0
                    && *index != newest
                    && record.key.boundary == EngineBoundary::OuterParagraphEnd
            })
            .or_else(|| {
                history
                    .iter()
                    .enumerate()
                    .find(|(index, _)| *index != 0 && *index != newest)
            })
            .map(|(index, _)| index);
        let Some(victim) = victim else {
            break;
        };
        history.remove(victim);
    }
    history
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
    CandidateNotComplete,
    UnexpectedResource,
    ResourceNoProgress {
        need: Box<ResourceNeed>,
    },
    SourceRegistration(SourceRegistrationError),
    CommandSummary(tex_command::CommandSummaryError),
    Execute(tex_exec::ExecError),
    Format(tex_state::FormatError),
    FormatDump(tex_exec::FormatDumpError),
    World(WorldError),
    State(tex_state::StateError),
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
            Self::CandidateNotComplete => f.write_str("revision candidate is not complete"),
            Self::UnexpectedResource => f.write_str("resource fulfillment does not match"),
            Self::ResourceNoProgress { need, .. } => {
                write!(f, "resource replay made no progress for {need:?}")
            }
            Self::SourceRegistration(error) => write!(f, "source registration failed: {error}"),
            Self::CommandSummary(error) => write!(f, "checkpoint failed: {error}"),
            Self::Execute(error) => write!(f, "incremental execution failed: {error}"),
            Self::Format(error) => write!(f, "incremental format materialization failed: {error}"),
            Self::FormatDump(error) => write!(f, "incremental format capture failed: {error}"),
            Self::World(error) => write!(f, "incremental world failed: {error}"),
            Self::State(error) => write!(f, "incremental generation failed: {error:?}"),
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
mod phase6_tests {
    use super::*;

    #[test]
    fn rejected_transaction_leaves_detached_session_unchanged() {
        let mut session =
            Session::start((), "reject", RevisionId::new(1), "\\end", 1024).expect("session");
        let before = session.content_hash();
        let foreign = Session::start((), "foreign", RevisionId::new(1), "\\end", 1024)
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
        let mut session =
            Session::start((), "history", RevisionId::new(1), "\\end", 1024).expect("session");
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
