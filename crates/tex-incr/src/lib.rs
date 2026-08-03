//! Named-boundary incremental editor sessions.

#![forbid(unsafe_code)]

use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
use std::fmt;
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
    Cancellation, CanonicalMainControl, CanonicalResourceFulfillment, CanonicalResourceHost,
    CanonicalResourceNeed, CanonicalResourceOutcome, CanonicalResourceWorld, CanonicalStepResult,
    CheckpointSink, EditorRestoreError, EngineBoundary, EngineCheckpoint, ExecutionBudgetCounters,
    ExecutionContext, ExecutionStats, Executor, MainControlStep, canonical_font_resource_path,
};
use tex_expand::{ResourceLookup, ResourceResult};
use tex_lex::{InputStack, MemoryInput};
use tex_out::dvi::{DviError, DviPagePlan, DviStreamWriter};
pub use tex_out::html::RenderedOutputId;
use tex_state::token::OriginId;
use tex_state::{
    ArtifactOrigin, CommittedArtifact, ContentHash, EditorLayout, EditorLayoutError, EffectRecord,
    FragmentStore, GenerationForkError, GenerationSubstrate, InputReadState, InputResolver,
    LayoutGeneration, LayoutResolvedOrigin, Piece, ProvenanceResolver, ResolvedSourceLocation,
    Universe, WorldError,
};

mod trace;

pub use trace::{TraceCompositionError, TraceOperation, TraceSummary, TraceValidationError};

fn canonical_candidate_control(
    universe: &mut Universe,
    job_name: &str,
    source_path: &str,
    bytes: Vec<u8>,
    profile: CommandProfile,
    initex: bool,
    emit_dvi: bool,
    root_framing: SourceFramingPolicy,
    root_framing_name: Option<&str>,
) -> Result<CanonicalMainControl, SessionError> {
    let mut control = if initex {
        CanonicalMainControl::prepared_initex(profile)
    } else {
        CanonicalMainControl::with_profile(profile)
    };
    control.set_dvi_output(emit_dvi);
    let mut registration = SourceRegistration::new(RegisteredSourceKind::Generated, bytes)
        .with_name(source_path)
        .with_framing(root_framing);
    if let Some(name) = root_framing_name {
        registration = registration.with_framing_name(name);
    }
    control.register_root_source(registration)?;
    control.flush_pending_file_framing(universe);
    control.capabilities_mut().set_startup_job_name(job_name);
    Ok(control)
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

/// One directly restartable accepted-revision record.
#[derive(Clone, Debug)]
pub struct BoundaryRecord {
    revision: RevisionId,
    key: BoundaryKey,
    effect_prefix: usize,
    artifact_prefix: usize,
    checkpoint: EngineCheckpoint,
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
        self.checkpoint.state_hash()
    }

    #[must_use]
    pub const fn checkpoint(&self) -> &EngineCheckpoint {
        &self.checkpoint
    }
}

/// Honest split between restart roots and detached accepted output.
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
    /// Execution path that produced this accepted revision.
    pub execution_path: RevisionExecutionPath,
    pub restart_boundary: Option<BoundaryKey>,
    pub convergence_boundary: Option<BoundaryKey>,
    /// Accepted pages before the restart checkpoint, retained without replay.
    pub pages_retained_prefix: usize,
    pub pages_reused: usize,
    pub pages_retyped: usize,
    pub reexecuted_bytes: usize,
    /// Tokens accounted during reexecution, including text spans and memo-hit traces.
    pub reexecuted_tokens: usize,
    /// Tokens that required scalar main-control dispatch.
    pub reexecuted_commands: usize,
    /// Ordinary macro-body character tokens handled by the batched text path.
    pub reexecuted_macro_text_span_tokens: usize,
    /// Ordinary physical-source character tokens handled by the batched text path.
    pub reexecuted_source_text_span_tokens: usize,
    pub reexecuted_paragraphs: usize,
    /// Paragraph-history probes performed by this revision only.
    pub paragraph_replay_lookups: u64,
    /// Paragraph-history records mounted by this revision only.
    pub paragraph_replay_hits: u64,
    /// Typed paragraph dependency validation failures in this revision only.
    pub paragraph_replay_validation_misses: u64,
    pub same_history_attempts: usize,
    pub same_history_hash_mismatches: usize,
    pub trace_nodes_walked: usize,
    /// Adopted page leaves below a verified suffix summary.
    pub trace_leaf_hits: usize,
    /// Verified parent summaries replayed as a unit.
    pub trace_subtree_hits: usize,
    /// Shallow bytes retained by the accepted ordered boundary trace.
    pub trace_retained_bytes: usize,
    pub suffixes_adopted: usize,
    pub same_history_stop: SameHistoryStop,
    pub restart_fork_latency: Duration,
    /// Edit validation, accepted-output snapshots, and revision-layout setup.
    pub revision_setup_latency: Duration,
    /// Time inside the executor resume call, excluding session-owned setup.
    pub executor_latency: Duration,
    pub reexecution_latency: Duration,
    /// Copying detached diagnostics, effects, artifacts, and DVI page plans
    /// out of the completed scratch execution.
    pub output_snapshot_latency: Duration,
    /// Publishing or discarding speculative accepted paragraph history.
    pub paragraph_history_transition_latency: Duration,
    pub trace_validation_latency: Duration,
    pub trace_replay_latency: Duration,
    pub splice_latency: Duration,
    /// Accepted-substrate replacement or retained-origin publication,
    /// including release of the superseded generation.
    pub substrate_transition_latency: Duration,
    /// Pending-revision pruning and accepted-output view construction,
    /// excluding `substrate_transition_latency`.
    pub acceptance_latency: Duration,
}

/// Accepted token-delivery telemetry for one editor revision.
///
/// These counters belong to the incremental session's accepted-output model:
/// they describe work attributed to a revision, not live input-stack state.
/// The canonical command path currently reports the default value until its
/// finer-grained delivery counters are wired into candidate completion.
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

impl From<tex_lex::ExpansionStats> for ExpansionStats {
    fn from(stats: tex_lex::ExpansionStats) -> Self {
        Self {
            token_frame_steps: stats.token_frame_steps,
            provenance_resolutions: stats.provenance_resolutions,
            character_tokens: stats.character_tokens,
            meaning_lookups: stats.meaning_lookups,
            literal_spans: stats.literal_spans,
            literal_tokens: stats.literal_tokens,
            segmentation_cache_hits: stats.segmentation_cache_hits,
            segmentation_cache_misses: stats.segmentation_cache_misses,
            builder_appends: stats.builder_appends,
            source_text_span_attempts: stats.source_text_span_attempts,
            source_text_spans: stats.source_text_spans,
            source_text_tokens: stats.source_text_tokens,
            meaning_cache_hits: stats.meaning_cache_hits,
            meaning_cache_misses: stats.meaning_cache_misses,
            frame_step_nanos: stats.frame_step_nanos,
            provenance_nanos: stats.provenance_nanos,
            classification_meaning_nanos: stats.classification_meaning_nanos,
            builder_append_nanos: stats.builder_append_nanos,
            frame_step_timer_samples: stats.frame_step_timer_samples,
            provenance_timer_samples: stats.provenance_timer_samples,
            classification_meaning_timer_samples: stats.classification_meaning_timer_samples,
            builder_append_timer_samples: stats.builder_append_timer_samples,
        }
    }
}

/// High-level execution path used to produce one accepted revision.
///
/// Paragraph telemetry remains generic. This attribution distinguishes an
/// unchanged-root stabilization rerun from ordinary edits and safe cold
/// fallback without encoding generated-file or label semantics.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum RevisionExecutionPath {
    #[default]
    Cold,
    FastEdit,
    SlowEdit,
    ExternalInputDelta,
    ForcedJobStartFallback,
}

/// Why identical-history suffix adoption did or did not stop re-execution.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum SameHistoryStop {
    /// A mapped schedule entry matched the authoritative probabilistic
    /// future-state projection.
    Matched,
    /// The mapped named-boundary schedule differed from the accepted revision.
    ScheduleDiverged,
    /// Every comparable boundary missed the probabilistic future-state
    /// projection.
    HashesDiverged,
    /// No old boundary after the restart anchor could be mapped and compared.
    NoComparableBoundary,
    /// This was a cold execution, so identical-history adoption was not attempted.
    #[default]
    NotAttempted,
}

/// Detached result of one accepted editor revision.
#[derive(Clone, Debug)]
pub struct AcceptedOutput {
    pub revision: RevisionId,
    pub content_hash: ContentHash,
    pub effects: Vec<EffectRecord>,
    pub artifacts: Vec<CommittedArtifact>,
    pub dvi_pages: Vec<DviPagePlan>,
    pub history: Vec<BoundaryRecord>,
    pub reuse: ReuseMetrics,
    pub retention: RetentionMetrics,
}

/// Accepted engine state whose fallible page suffix remains unpublished.
///
/// This is the consuming native-finalization handoff. The incremental session
/// retains ordinary committed pages unchanged, but transfers the suffix
/// beginning with the first deferred `OpenOut` as a typed prepared value.
pub struct AcceptedUniverseFinalization {
    pub universe: Universe,
    pub prepared_pages: Option<tex_state::PreparedPageSuffix>,
}

/// One fully executed editor revision that has not replaced accepted session
/// state yet.
///
/// Hosts may materialize and validate its detached output before calling
/// [`Session::accept_pending`]. Dropping this value rolls the candidate back
/// without changing the accepted revision.
pub struct PendingRevision {
    session_output_id: RenderedOutputId,
    base_revision: RevisionId,
    base_content_hash: ContentHash,
    revision: RevisionId,
    source: String,
    fragments: FragmentStore,
    layout: EditorLayout,
    content_hash: ContentHash,
    effects: Vec<EffectRecord>,
    artifacts: Vec<CommittedArtifact>,
    dvi_pages: Vec<DviPagePlan>,
    history: Vec<BoundaryRecord>,
    substrate: PendingSubstrate,
    reuse: ReuseMetrics,
    dumped_format: bool,
    format_dump_receipt: Option<tex_exec::FormatDumpReceipt>,
    expansion_stats: ExpansionStats,
    candidate_memo: Option<tex_state::PureMemoRuntime>,
}

/// One private revision execution retained across resource suspensions.
///
/// The candidate owns every mutable engine root and speculative checkpoint
/// sink. Callers supply a fresh resolver view to each [`Self::drive`] call;
/// no host capability is retained between calls.
pub struct RevisionCandidate {
    universe: Universe,
    control: CanonicalMainControl,
    sink: CandidateSink,
    memo: tex_state::PureMemoRuntime,
    completed: Option<CanonicalCandidateCompletion>,
    suspension_serial: u64,
    delivered_commands: usize,
    effect_start: usize,
    job_start_captured: bool,
    execution_budgets: tex_exec::ExecutionBudgets,
    kind: RevisionCandidateKind,
}

struct CanonicalCandidateCompletion {
    dvi_pages: Vec<DviPagePlan>,
    dumped_format: bool,
    format_dump_receipt: Option<tex_exec::FormatDumpReceipt>,
    delivered_tokens: usize,
    main_control_dispatches: usize,
}

enum CandidateSink {
    Cold(HistorySink),
    Advance(ResumeSink),
}

enum RevisionCandidateKind {
    Initial {
        source_len: usize,
    },
    Replacement {
        setup: Box<AdvanceSetup>,
    },
    Incremental {
        setup: Box<AdvanceSetup>,
        restart: usize,
        restart_fork_latency: Duration,
    },
}

/// Result of driving a retained private revision until it either suspends or
/// reaches a terminal executor state.
#[derive(Clone, Debug)]
pub enum RevisionCandidateResult {
    AwaitingResources(CanonicalResourceNeed),
    Complete,
}

struct AdvanceSetup {
    execution_path: RevisionExecutionPath,
    next_revision: RevisionId,
    old_source: String,
    old_history: Vec<BoundaryRecord>,
    old_effects: Vec<EffectRecord>,
    old_artifacts: Vec<CommittedArtifact>,
    old_pages: Vec<DviPagePlan>,
    next: String,
    fragments: FragmentStore,
    next_layout: EditorLayout,
    map: EditMap,
    revision_setup_latency: Duration,
}

enum PendingSubstrate {
    Retained {
        scratch: Universe,
        adopted_origins: Vec<OriginId>,
    },
    Replaced(GenerationSubstrate),
}

impl PendingRevision {
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
    pub fn artifacts(&self) -> &[CommittedArtifact] {
        &self.artifacts
    }

    #[must_use]
    pub const fn reuse(&self) -> ReuseMetrics {
        self.reuse
    }

    pub fn dvi_bytes(&self) -> Result<Vec<u8>, DviError> {
        dvi_bytes(&self.dvi_pages)
    }
}

impl RevisionCandidate {
    fn validate_execution_budgets(&self) -> Result<(), SessionError> {
        let attempted_steps = u64::try_from(self.delivered_commands).unwrap_or(u64::MAX);
        let input_frames = u64::try_from(self.control.input_level_count()).unwrap_or(u64::MAX);
        let journal_bytes = u64::try_from(self.universe.env_journal_bytes()).unwrap_or(u64::MAX);
        let pending_effects = u64::try_from(
            self.universe
                .world()
                .effect_records()
                .len()
                .saturating_sub(self.effect_start),
        )
        .unwrap_or(u64::MAX);
        for (resource, limit, attempted) in [
            ("steps", self.execution_budgets.steps, attempted_steps),
            (
                "live input frames",
                self.execution_budgets.input_frames,
                input_frames,
            ),
            (
                "environment journal bytes",
                self.execution_budgets.journal_bytes,
                journal_bytes,
            ),
            (
                "pending effects",
                self.execution_budgets.effects,
                pending_effects,
            ),
        ] {
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
        Ok(())
    }

    #[must_use]
    pub fn resolve_frozen_diagnostic_primary(
        &self,
        frozen: &tex_exec::FrozenDiagnosticOrigin,
    ) -> Option<ResolvedSourceLocation> {
        self.resolve_frozen_diagnostic_primary_with_layout(frozen, None)
    }

    fn resolve_frozen_diagnostic_primary_with_layout(
        &self,
        frozen: &tex_exec::FrozenDiagnosticOrigin,
        root_layout: Option<(&FragmentStore, &EditorLayout)>,
    ) -> Option<ResolvedSourceLocation> {
        match frozen {
            tex_exec::FrozenDiagnosticOrigin::Resolved(location) => Some(location.clone()),
            tex_exec::FrozenDiagnosticOrigin::Root(span) => {
                let layout = match &self.kind {
                    RevisionCandidateKind::Initial { .. } => root_layout,
                    RevisionCandidateKind::Replacement { setup }
                    | RevisionCandidateKind::Incremental { setup, .. } => {
                        Some((&setup.fragments, &setup.next_layout))
                    }
                }?;
                match self.universe.resolve_root_span(*span, layout.0, layout.1) {
                    LayoutResolvedOrigin::Current {
                        path,
                        doc_offset_lo,
                        doc_offset_hi,
                        line,
                        column,
                    } => Some(ResolvedSourceLocation {
                        path,
                        start: doc_offset_lo,
                        end: doc_offset_hi,
                        line,
                        column,
                    }),
                    _ => None,
                }
            }
            tex_exec::FrozenDiagnosticOrigin::Generated { span, fallback } => {
                let layout = match &self.kind {
                    RevisionCandidateKind::Initial { .. } => root_layout,
                    RevisionCandidateKind::Replacement { setup }
                    | RevisionCandidateKind::Incremental { setup, .. } => {
                        Some((&setup.fragments, &setup.next_layout))
                    }
                };
                let Some((fragments, layout)) = layout else {
                    return Some(fallback.clone());
                };
                let Some(root) =
                    fragments.root_span_for_generated_bytes(&span.bytes, span.start..span.end)
                else {
                    return Some(fallback.clone());
                };
                match self.universe.resolve_root_span(root, fragments, layout) {
                    LayoutResolvedOrigin::Current {
                        path,
                        doc_offset_lo,
                        doc_offset_hi,
                        line,
                        column,
                    } => Some(ResolvedSourceLocation {
                        path,
                        start: doc_offset_lo,
                        end: doc_offset_hi,
                        line,
                        column,
                    }),
                    _ => Some(fallback.clone()),
                }
            }
        }
    }

    /// Resolves a captured engine diagnostic while this candidate's private
    /// provenance universe and proposed editor layout are still live.
    #[must_use]
    pub fn resolve_diagnostic_site_primary(
        &self,
        site: &tex_state::provenance::DiagnosticSite,
    ) -> Option<ResolvedSourceLocation> {
        self.resolve_diagnostic_site_primary_with_layout(site, None)
    }

    fn resolve_diagnostic_site_primary_with_layout(
        &self,
        site: &tex_state::provenance::DiagnosticSite,
        root_layout: Option<(&FragmentStore, &EditorLayout)>,
    ) -> Option<ResolvedSourceLocation> {
        let origin = site.primary_origin()?;
        let resolver = ProvenanceResolver::new(&self.universe);
        let layout = match &self.kind {
            RevisionCandidateKind::Initial { .. } => root_layout,
            RevisionCandidateKind::Replacement { setup }
            | RevisionCandidateKind::Incremental { setup, .. } => {
                Some((&setup.fragments, &setup.next_layout))
            }
        };
        match layout {
            Some((fragments, layout)) => {
                match resolver.resolve_layout_origin(origin, fragments, layout) {
                    LayoutResolvedOrigin::Current {
                        path,
                        doc_offset_lo,
                        doc_offset_hi,
                        line,
                        column,
                    } => Some(ResolvedSourceLocation {
                        path,
                        start: doc_offset_lo,
                        end: doc_offset_hi,
                        line,
                        column,
                    }),
                    LayoutResolvedOrigin::Foreign => resolver.resolve_origin(origin),
                    LayoutResolvedOrigin::Deleted { .. } | LayoutResolvedOrigin::Unknown => None,
                }
            }
            None => resolver.resolve_origin(origin),
        }
    }

    /// Borrows the reached engine state after execution has completed but
    /// before the candidate is accepted. Downstream resource finalizers may
    /// use this boundary to install already validated immutable resources;
    /// incomplete candidates never expose speculative live state.
    pub fn completed_universe_mut(&mut self) -> Option<&mut Universe> {
        self.completed.as_ref().map(|_| &mut self.universe)
    }

    /// Drives committed executor steps until the candidate either needs a
    /// resource or completes. Resolver selection is call-local so a newly
    /// provisioned immutable generation is observed only by the replayed step.
    pub fn drive_with_resource_resolvers(
        &mut self,
        host: &mut dyn CanonicalResourceHost,
        cancellation: &Cancellation,
    ) -> Result<RevisionCandidateResult, SessionError> {
        if self.completed.is_some() {
            return Ok(RevisionCandidateResult::Complete);
        }
        if !self.job_start_captured {
            let sink: &mut dyn CheckpointSink = match &mut self.sink {
                CandidateSink::Cold(sink) => sink,
                CandidateSink::Advance(sink) => sink,
            };
            if sink.wants_checkpoint(EngineBoundary::JobStart) {
                let checkpoint = self.control.capture_checkpoint_with_exact_identity(
                    EngineBoundary::JobStart,
                    &mut self.universe,
                    ExecutionBudgetCounters::default(),
                )?;
                sink.checkpoint(checkpoint);
            }
            self.job_start_captured = true;
        }
        // A fulfilled or authoritatively unavailable capability must make the
        // replayed aggregate operation pass that same suspension boundary.
        // Keep this list only until a committed main-control step: encountering
        // an answered need again before then is a retained-host protocol bug,
        // not legitimate engine work to fund up to the command-fuel limit.
        let mut answered_needs = Vec::new();
        loop {
            if cancellation.is_cancelled() {
                return Err(SessionError::Execute(
                    tex_exec::ExecError::ExecutionCancelled,
                ));
            }
            self.validate_execution_budgets()?;
            match self.control.advance(&mut self.universe)? {
                CanonicalStepResult::Progress(step) => {
                    answered_needs.clear();
                    self.delivered_commands = self.delivered_commands.saturating_add(1);
                    self.validate_execution_budgets()?;
                    let boundaries = self.control.take_completed_boundaries();
                    for boundary in boundaries {
                        let sink: &mut dyn CheckpointSink = match &mut self.sink {
                            CandidateSink::Cold(sink) => sink,
                            CandidateSink::Advance(sink) => sink,
                        };
                        if sink.wants_checkpoint(boundary) {
                            let checkpoint = self.control.capture_checkpoint_with_exact_identity(
                                boundary,
                                &mut self.universe,
                                ExecutionBudgetCounters::default(),
                            )?;
                            sink.checkpoint(checkpoint);
                        }
                    }
                    let stop = match &self.sink {
                        CandidateSink::Cold(sink) => sink.stop_requested(),
                        CandidateSink::Advance(sink) => sink.stop_requested(),
                    };
                    if stop || matches!(step, MainControlStep::End) {
                        let dvi_pages = self
                            .control
                            .take_prepared_dvi_pages()
                            .into_iter()
                            .map(tex_exec::PreparedDviPage::into_plan)
                            .collect();
                        self.completed = Some(CanonicalCandidateCompletion {
                            dvi_pages,
                            dumped_format: self.control.dumped_format(),
                            format_dump_receipt: self.control.format_dump_receipt().cloned(),
                            delivered_tokens: self.delivered_commands,
                            main_control_dispatches: self.delivered_commands,
                        });
                        return Ok(RevisionCandidateResult::Complete);
                    }
                }
                CanonicalStepResult::Suspended(need) => {
                    self.validate_execution_budgets()?;
                    if answered_needs.contains(&need) {
                        return Err(SessionError::CanonicalResourceNoProgress {
                            need,
                            site: self.control.pending_resource_site(),
                        });
                    }
                    let outcome = {
                        let mut world = CanonicalResourceWorld::new(&mut self.universe);
                        host.fulfill(&mut world, &need)
                    };
                    match outcome {
                        CanonicalResourceOutcome::Fulfilled(fulfillment) => {
                            self.fulfill_resource(&need, fulfillment)?;
                            answered_needs.push(need);
                        }
                        CanonicalResourceOutcome::Unavailable => {
                            self.mark_unavailable(&need);
                            answered_needs.push(need);
                        }
                        CanonicalResourceOutcome::Declined => {
                            self.suspension_serial = self.suspension_serial.saturating_add(1);
                            return Ok(RevisionCandidateResult::AwaitingResources(need));
                        }
                    }
                }
            }
        }
    }

    fn fulfill_resource(
        &mut self,
        need: &CanonicalResourceNeed,
        fulfillment: CanonicalResourceFulfillment,
    ) -> Result<(), SessionError> {
        match (need, fulfillment) {
            (
                CanonicalResourceNeed::Input { name: expected, .. },
                CanonicalResourceFulfillment::Input { name, source },
            ) if expected == &name => self.control.capabilities_mut().register_input(name, source),
            (
                CanonicalResourceNeed::InputProbe { request: expected },
                CanonicalResourceFulfillment::InputProbe { request, resource },
            ) if expected == &request => self
                .control
                .capabilities_mut()
                .register_input_probe(request.name, resource),
            (
                CanonicalResourceNeed::Font { request: expected },
                CanonicalResourceFulfillment::Font { request, resource },
            ) if expected == &request => self
                .control
                .capabilities_mut()
                .register_font(canonical_font_resource_path(&request.name), *resource),
            (
                CanonicalResourceNeed::PdfImage { request: expected },
                CanonicalResourceFulfillment::PdfImage { request, resource },
            ) if expected == &request => self
                .control
                .capabilities_mut()
                .register_pdf_image(request, *resource),
            _ => return Err(SessionError::UnexpectedCanonicalResource),
        }
        Ok(())
    }

    fn mark_unavailable(&mut self, need: &CanonicalResourceNeed) {
        match need {
            CanonicalResourceNeed::Input { name, .. } => {
                self.control.capabilities_mut().mark_input_unavailable(name)
            }
            CanonicalResourceNeed::InputProbe { request } => self
                .control
                .capabilities_mut()
                .mark_input_probe_unavailable(&request.name),
            CanonicalResourceNeed::Font { request } => {
                self.control.capabilities_mut().register_font(
                    canonical_font_resource_path(&request.name),
                    tex_command::FontResource::Unavailable,
                )
            }
            CanonicalResourceNeed::PdfImage { request } => self
                .control
                .capabilities_mut()
                .register_pdf_image(request.clone(), tex_command::PdfImageResource::Unavailable),
        }
    }

    #[must_use]
    pub fn suspension_serial(&self) -> u64 {
        self.suspension_serial
    }

    pub fn set_cumulative_fuel_limit(&mut self, limit: u64) {
        self.control
            .set_fuel_limit(limit.max(1))
            .expect("positive session fuel limit");
    }

    pub fn set_execution_budgets(&mut self, budgets: tex_exec::ExecutionBudgets) {
        self.execution_budgets = budgets;
    }

    #[must_use]
    pub const fn execution_telemetry(&self) -> tex_exec::ExecutionTelemetry {
        tex_exec::ExecutionTelemetry {
            cold_starts: 0,
            advance_calls: self.delivered_commands as u64,
            suspensions: self.suspension_serial,
            local_step_retries: 0,
            replayed_delivered_tokens: 0,
            replayed_dispatches: 0,
            cumulative_fuel: self.control.fuel_burned(),
            engine_time: Duration::ZERO,
            savepoint_capture_time: Duration::ZERO,
            savepoint_restore_time: Duration::ZERO,
        }
    }

    /// Charges the private execution roots retained while this candidate is
    /// suspended. Accepted-session telemetry remains separate until commit.
    #[must_use]
    pub fn retention_metrics(&self) -> RetentionMetrics {
        let (diagnostic_bytes, output_bytes) = match &self.kind {
            RevisionCandidateKind::Initial { .. } => (0, self.universe.retained_output_bytes()),
            RevisionCandidateKind::Replacement { setup }
            | RevisionCandidateKind::Incremental { setup, .. } => (
                setup
                    .fragments
                    .retained_bytes()
                    .saturating_add(setup.next_layout.retained_bytes()),
                self.universe.retained_output_bytes(),
            ),
        };
        RetentionMetrics {
            checkpoint_root_bytes: self
                .universe
                .live_generation_charged_bytes()
                .saturating_add(std::mem::size_of::<CanonicalMainControl>()),
            memo_result_bytes: self.universe.pure_memo_stats().retained_bytes,
            diagnostic_bytes,
            output_bytes,
            protected_overage_bytes: 0,
        }
    }
}

/// Typed result of resolving an accepted rendered event against a DOM revision.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RenderedSourceResult {
    Current(tex_state::ResolvedSourceLocation),
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
            Some(unit) => *origins.get(usize::try_from(unit).ok()?)?,
            None => origins
                .iter()
                .copied()
                .find(|origin| *origin != ArtifactOrigin::Unknown)?,
        };
        (origin != ArtifactOrigin::Unknown).then_some(origin)
    }
}

#[derive(Debug, Default)]
struct RenderMapCache {
    pages: Vec<Option<PageRenderMap>>,
    #[cfg(test)]
    page_lowerings: Vec<usize>,
}

impl RenderMapCache {
    fn retained_bytes(&self) -> usize {
        self.pages
            .capacity()
            .saturating_mul(size_of::<Option<PageRenderMap>>())
            .saturating_add(
                self.pages
                    .iter()
                    .flatten()
                    .map(PageRenderMap::retained_bytes)
                    .sum::<usize>(),
            )
    }
}

impl AcceptedOutput {
    pub fn dvi_bytes(&self) -> Result<Vec<u8>, DviError> {
        dvi_bytes(&self.dvi_pages)
    }
}

fn dvi_bytes(pages: &[DviPagePlan]) -> Result<Vec<u8>, DviError> {
    let mut writer = DviStreamWriter::new(Vec::new());
    for plan in pages {
        writer.write_page_plan(plan)?;
    }
    writer.finish()
}

/// Long-lived incremental session. Live executor state is deliberately private.
pub struct Session {
    template: Universe,
    pure_memo: tex_state::PureMemoRuntime,
    job_name: String,
    source_path: String,
    revision: RevisionId,
    output_id: RenderedOutputId,
    source: String,
    fragments: FragmentStore,
    layout: EditorLayout,
    content_hash: ContentHash,
    effects: Vec<EffectRecord>,
    artifacts: Vec<CommittedArtifact>,
    dvi_pages: Vec<DviPagePlan>,
    history: Vec<BoundaryRecord>,
    substrate: Option<GenerationSubstrate>,
    checkpoint_budget: usize,
    registered_inputs: BTreeMap<PathBuf, Vec<u8>>,
    accepted_retention: Option<RetentionMetrics>,
    dumped_format: bool,
    format_dump_receipt: Option<tex_exec::FormatDumpReceipt>,
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
    /// Resolves one diagnostic captured by an unaccepted initial candidate
    /// against this session's editor layout.
    #[must_use]
    pub fn resolve_candidate_diagnostic_site_primary(
        &self,
        candidate: &RevisionCandidate,
        site: &tex_state::provenance::DiagnosticSite,
    ) -> Option<ResolvedSourceLocation> {
        candidate.resolve_diagnostic_site_primary_with_layout(
            site,
            Some((&self.fragments, &self.layout)),
        )
    }

    #[must_use]
    pub fn resolve_candidate_frozen_diagnostic_primary(
        &self,
        candidate: &RevisionCandidate,
        frozen: &tex_exec::FrozenDiagnosticOrigin,
    ) -> Option<ResolvedSourceLocation> {
        candidate.resolve_frozen_diagnostic_primary_with_layout(
            frozen,
            Some((&self.fragments, &self.layout)),
        )
    }

    pub fn start(
        template: Universe,
        job_name: impl Into<String>,
        revision: RevisionId,
        source: impl Into<String>,
        checkpoint_budget: usize,
    ) -> Result<Self, SessionError> {
        Self::start_with_source_path(
            template,
            job_name,
            "<editor>",
            revision,
            source,
            checkpoint_budget,
        )
    }

    pub fn start_with_source_path(
        template: Universe,
        job_name: impl Into<String>,
        source_path: impl Into<String>,
        revision: RevisionId,
        source: impl Into<String>,
        checkpoint_budget: usize,
    ) -> Result<Self, SessionError> {
        Self::start_with_prepared_source(
            template,
            job_name,
            source_path,
            revision,
            source.into(),
            false,
            checkpoint_budget,
        )
    }

    /// Starts a session from arbitrary physical file bytes.
    ///
    /// Valid UTF-8 remains ordinary editor text. Invalid UTF-8 is projected
    /// losslessly so every original byte becomes the same-valued Unicode
    /// scalar; the lexer recognizes that representation and does not split
    /// its UTF-8 backing encoding again in classic byte-input mode.
    pub fn start_with_source_bytes(
        template: Universe,
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
            template,
            job_name,
            source_path,
            revision,
            source,
            byte_projection,
            checkpoint_budget,
        )
    }

    fn start_with_prepared_source(
        template: Universe,
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
        let mut template = template;
        let pure_memo = template.take_pure_memo_runtime();
        Ok(Self {
            template,
            pure_memo,
            job_name: job_name.into(),
            source_path,
            revision,
            output_id: RenderedOutputId::from_bytes(output_id),
            content_hash: ContentHash::from_bytes(source.as_bytes()),
            source,
            fragments,
            layout,
            effects: Vec::new(),
            artifacts: Vec::new(),
            dvi_pages: Vec::new(),
            history: Vec::new(),
            substrate: None,
            checkpoint_budget,
            registered_inputs: BTreeMap::new(),
            accepted_retention: None,
            dumped_format: false,
            format_dump_receipt: None,
            utf8_input_as_bytes: false,
            dvi_output: true,
            root_framing: SourceFramingPolicy::Canonical,
            root_framing_name: None,
            root_source_is_byte_projection,
            command_profile: CommandProfile::TEX82,
            initex: true,
            expansion_stats: ExpansionStats::default(),
            render_maps: RefCell::default(),
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

    /// Selects classic TeX byte-oriented physical input for this session.
    ///
    /// This must be configured before the initial revision is executed. The
    /// resulting input-stack summaries retain the mode across later edits.
    pub fn set_utf8_input_as_bytes(&mut self, enabled: bool) {
        assert!(
            self.history.is_empty(),
            "input decoding mode cannot change after execution starts"
        );
        self.utf8_input_as_bytes = enabled;
    }

    /// Selects the canonical command profile and whether the session starts
    /// in INITEX mode. This must be configured before candidate execution.
    pub fn set_command_profile(&mut self, profile: CommandProfile, initex: bool) {
        assert!(
            self.history.is_empty(),
            "command profile cannot change after execution starts"
        );
        self.command_profile = profile;
        self.initex = initex;
    }

    /// Selects transcript-framing ownership for the editor root.
    ///
    /// The source path remains attached to the editor layout and registered
    /// source for provenance. Only the root's command-owned open/close events
    /// are affected; resolved included files retain canonical framing.
    pub fn set_root_source_framing(&mut self, framing: SourceFramingPolicy) {
        assert!(
            self.history.is_empty(),
            "root source framing cannot change after execution starts"
        );
        self.root_framing = framing;
    }

    /// Selects the selector-visible filename for canonical root framing.
    ///
    /// The editor layout keeps `source_path` as provenance. A driver whose
    /// internal VFS path differs from TeX's startup filename uses this value
    /// only for tex.web §537's `(name` display.
    pub fn set_root_source_framing_name(&mut self, name: impl Into<String>) {
        assert!(
            self.history.is_empty(),
            "root source framing name cannot change after execution starts"
        );
        self.root_framing_name = Some(name.into());
    }

    /// Selects the character domain already promised by this editor session.
    ///
    /// Valid UTF-8 authored roots use Umber's separately identified Unicode
    /// extension. LaTeX's compatibility input layer and a root projected from
    /// arbitrary legacy bytes remain exact eight-bit jobs.
    fn effective_command_profile(&self) -> CommandProfile {
        if self.utf8_input_as_bytes || self.root_source_is_byte_projection {
            self.command_profile
        } else {
            CommandProfile::unicode_extended(self.command_profile.dialect())
        }
    }

    /// Selects whether candidates prepare classic TeX82 DVI page plans.
    ///
    /// Artifacts are always committed for downstream outputs. This capability
    /// must be fixed before execution so every revision has one output policy.
    pub fn set_dvi_output(&mut self, enabled: bool) {
        assert!(
            self.history.is_empty(),
            "DVI output selection cannot change after execution starts"
        );
        self.dvi_output = enabled;
    }

    #[must_use]
    pub fn source(&self) -> &str {
        &self.source
    }

    /// Encodes an editor representation back to physical main-file bytes.
    /// Legacy byte projections map U+0000..U+00FF back to their original byte;
    /// newly inserted larger scalars retain their ordinary UTF-8 encoding.
    #[must_use]
    pub fn source_file_bytes(&self, source: &str) -> Vec<u8> {
        if !self.root_source_is_byte_projection {
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

    #[must_use]
    pub fn history(&self) -> &[BoundaryRecord] {
        &self.history
    }

    /// Enumerates semantic external-input observations retained by the
    /// accepted engine generation in canonical path order.
    pub fn accepted_input_dependencies(&self) -> impl Iterator<Item = &tex_state::InputDependency> {
        self.substrate
            .iter()
            .flat_map(|substrate| substrate.world().input_dependencies())
    }

    /// Returns telemetry for the session-owned pure-query cache.
    #[must_use]
    pub fn pure_memo_stats(&self) -> tex_state::PureMemoStats {
        self.pure_memo.stats()
    }

    /// Returns live retention telemetry for the accepted session state.
    ///
    /// The accepted output keeps its point-in-time metrics, while this view
    /// also charges caches constructed by later rendered-source queries.
    #[must_use]
    pub fn retention_metrics(&self) -> Option<RetentionMetrics> {
        self.accepted_retention.map(|mut retention| {
            retention.memo_result_bytes = self.pure_memo.stats().retained_bytes;
            retention.diagnostic_bytes = self.diagnostic_retained_bytes();
            retention.output_bytes = retention
                .output_bytes
                .saturating_add(self.render_maps.borrow().retained_bytes());
            retention.protected_overage_bytes = retention
                .checkpoint_root_bytes
                .saturating_add(retention.diagnostic_bytes)
                .saturating_sub(self.checkpoint_budget);
            retention
        })
    }

    pub fn cold(&mut self) -> Result<AcceptedOutput, SessionError> {
        let mut input_resolver = DirectInputResolver;
        let mut font_resolver = DirectFontResolver;
        self.cold_with_resolvers(&mut input_resolver, &mut font_resolver)
    }

    pub fn cold_with_resolvers(
        &mut self,
        input_resolver: &mut dyn InputResolver,
        font_resolver: &mut dyn tex_exec::FontResolver,
    ) -> Result<AcceptedOutput, SessionError> {
        self.cold_with_optional_image_resolver(input_resolver, font_resolver, None)
    }

    pub fn cold_with_resource_resolvers(
        &mut self,
        input_resolver: &mut dyn InputResolver,
        font_resolver: &mut dyn tex_exec::FontResolver,
        image_resolver: &mut dyn tex_exec::PdfImageResolver,
    ) -> Result<AcceptedOutput, SessionError> {
        self.cold_with_optional_image_resolver(input_resolver, font_resolver, Some(image_resolver))
    }

    /// Creates a private cold candidate without changing accepted session
    /// state. The returned owner may be retained across resource batches.
    pub fn start_cold_candidate(&self) -> Result<RevisionCandidate, SessionError> {
        let mut universe = self.template.clone();
        universe.begin_retained_session()?;
        universe.install_editor_fragments(&self.fragments, &self.layout)?;
        universe.set_root_editor_content_hash(ContentHash::from_bytes(self.source.as_bytes()));
        let control = canonical_candidate_control(
            &mut universe,
            &self.job_name,
            &self.source_path,
            self.source_file_bytes(&self.source),
            self.effective_command_profile(),
            self.initex,
            self.dvi_output,
            self.root_framing,
            self.root_framing_name.as_deref(),
        )?;
        let mut memo = self.pure_memo.clone();
        memo.begin_paragraph_history(false);
        universe.install_pure_memo_runtime(std::mem::take(&mut memo));
        let effect_start = universe.world().effect_records().len();
        Ok(RevisionCandidate {
            universe,
            control,
            sink: CandidateSink::Cold(HistorySink::default()),
            memo,
            completed: None,
            suspension_serial: 0,
            delivered_commands: 0,
            effect_start,
            job_start_captured: false,
            execution_budgets: tex_exec::ExecutionBudgets::default(),
            kind: RevisionCandidateKind::Initial {
                source_len: self.source.len(),
            },
        })
    }

    /// Accepts a completed private cold candidate into this (typically still
    /// private) session.
    pub fn accept_cold_candidate(
        &mut self,
        candidate: RevisionCandidate,
    ) -> Result<AcceptedOutput, SessionError> {
        let run = finish_cold_candidate(candidate)?;
        self.pure_memo = run.memo;
        self.accept_cold(run.run)
    }

    /// Creates a private edited-revision candidate while leaving accepted
    /// history, output, and substrate untouched.
    pub fn start_advance_candidate(
        &self,
        next_revision: RevisionId,
        edit: Edit,
    ) -> Result<RevisionCandidate, SessionError> {
        self.start_advance_candidate_with_policy(next_revision, edit, false)
    }

    /// Creates a private edited-revision candidate that executes from
    /// [`EngineBoundary::JobStart`] while preserving accepted history until
    /// the candidate is committed.
    ///
    /// This entry point is for hosts that have already found an accepted
    /// external-input dependency mismatch against the exact immutable input
    /// snapshot supplied to the candidate. It deliberately bypasses retained
    /// checkpoint restore and paragraph replay for this pass.
    pub fn start_advance_candidate_from_job_start(
        &self,
        next_revision: RevisionId,
        edit: Edit,
    ) -> Result<RevisionCandidate, SessionError> {
        self.start_advance_candidate_with_policy(next_revision, edit, true)
    }

    /// Creates a private unchanged-root candidate for a changed external-input
    /// snapshot.
    ///
    /// The candidate restores only the accepted [`EngineBoundary::JobStart`]
    /// root, retains the accepted paragraph history for typed dependency
    /// validation, and preserves the accepted editor revision and source
    /// layout. It deliberately disables suffix adoption: an external input
    /// that has not yet been consumed can leave an earlier checkpoint equal
    /// even though its future execution differs against the new snapshot.
    /// Dropping the candidate leaves all accepted state untouched.
    pub fn start_external_input_delta_candidate(&self) -> Result<RevisionCandidate, SessionError> {
        let revision_setup_started = Timer::start();
        let fragments = self.fragments.clone();
        let next_layout = EditorLayout::new(
            self.layout.path(),
            self.layout.generation(),
            self.layout.pieces().to_vec(),
            &fragments,
        )?;
        let setup = Box::new(AdvanceSetup {
            execution_path: RevisionExecutionPath::ExternalInputDelta,
            next_revision: self.revision,
            old_source: self.source.clone(),
            old_history: self.history.clone(),
            old_effects: self.effects.clone(),
            old_artifacts: self.artifacts.clone(),
            old_pages: self.dvi_pages.clone(),
            next: self.source.clone(),
            fragments,
            next_layout,
            map: EditMap::new(0..0, 0),
            revision_setup_latency: revision_setup_started.elapsed(),
        });
        let restart = setup
            .old_history
            .iter()
            .position(|record| record.key.boundary == EngineBoundary::JobStart);
        let can_restore = restart.is_some_and(|restart| {
            self.substrate.as_ref().is_some_and(|substrate| {
                setup.old_history[restart]
                    .checkpoint()
                    .can_fork_canonical_editor(
                        substrate,
                        setup.old_source.as_bytes(),
                        &self.source_file_bytes(&setup.next),
                    )
            })
        });
        if let Some(restart) = restart.filter(|_| can_restore) {
            self.start_restored_candidate(setup, restart, false)
        } else {
            self.start_replacement_candidate(setup)
        }
    }

    fn start_advance_candidate_with_policy(
        &self,
        next_revision: RevisionId,
        edit: Edit,
        force_job_start: bool,
    ) -> Result<RevisionCandidate, SessionError> {
        let revision_setup_started = Timer::start();
        self.validate_edit(next_revision, &edit)?;
        let old_source = self.source.clone();
        let old_history = self.history.clone();
        let mut next = old_source.clone();
        next.replace_range(edit.range.clone(), &edit.replacement);
        let (expanded_range, expanded_replacement) = line_expanded_replacement(&old_source, &edit);
        let mut fragments = self.fragments.clone();
        let (fragment, _) = fragments.append(
            Arc::from(expanded_replacement.as_bytes()),
            next_revision.raw(),
        )?;
        let next_layout = replace_layout_range(
            &self.layout,
            &fragments,
            expanded_range,
            fragment,
            expanded_replacement.len(),
            LayoutGeneration::new(next_revision.raw()),
        )?;
        let restart = if force_job_start {
            None
        } else {
            select_restart(&old_history, &old_source, &next, &edit)
        };
        let map = EditMap::new(edit.range.clone(), edit.replacement.len());
        if !force_job_start {
            self.substrate
                .as_ref()
                .ok_or(SessionError::MissingAcceptedSubstrate)?
                .world()
                .validate_recorded_inputs()?;
        }
        let setup = Box::new(AdvanceSetup {
            execution_path: if force_job_start {
                RevisionExecutionPath::ForcedJobStartFallback
            } else {
                RevisionExecutionPath::SlowEdit
            },
            next_revision,
            old_source,
            old_history,
            old_effects: self.effects.clone(),
            old_artifacts: self.artifacts.clone(),
            old_pages: self.dvi_pages.clone(),
            next,
            fragments,
            next_layout,
            map,
            revision_setup_latency: revision_setup_started.elapsed(),
        });

        match restart {
            Some(restart) => {
                let can_restore = self.substrate.as_ref().is_some_and(|substrate| {
                    setup.old_history[restart]
                        .checkpoint()
                        .can_fork_canonical_editor(
                            substrate,
                            setup.old_source.as_bytes(),
                            &self.source_file_bytes(&setup.next),
                        )
                });
                if can_restore {
                    self.start_restored_candidate(setup, restart, true)
                } else {
                    let mut fallback = setup;
                    fallback.execution_path = RevisionExecutionPath::ForcedJobStartFallback;
                    self.start_replacement_candidate(fallback)
                }
            }
            None => self.start_replacement_candidate(setup),
        }
    }

    fn start_restored_candidate(
        &self,
        setup: Box<AdvanceSetup>,
        restart: usize,
        allow_convergence: bool,
    ) -> Result<RevisionCandidate, SessionError> {
        let substrate = self
            .substrate
            .as_ref()
            .ok_or(SessionError::MissingAcceptedSubstrate)?;
        let anchor = &setup.old_history[restart];
        let mut control = if self.initex {
            CanonicalMainControl::prepared_initex(self.effective_command_profile())
        } else {
            CanonicalMainControl::with_profile(self.effective_command_profile())
        };
        control.set_dvi_output(self.dvi_output);
        control
            .capabilities_mut()
            .set_startup_job_name(&self.job_name);
        let (mut universe, restart_fork_latency) = anchor.checkpoint().fork_canonical_editor(
            &mut control,
            substrate,
            setup.old_source.as_bytes(),
            Arc::from(self.source_file_bytes(&setup.next)),
            &setup.fragments,
            &setup.next_layout,
        )?;
        for (path, bytes) in &self.registered_inputs {
            universe.world_mut().set_memory_file(path, bytes.clone())?;
        }
        let mut memo = self.pure_memo.clone();
        memo.begin_paragraph_history(true);
        universe.install_pure_memo_runtime(std::mem::take(&mut memo));
        let sink = ResumeSink::new(&setup.old_history, restart, &setup.map, allow_convergence);
        let effect_start = universe.world().effect_records().len();
        Ok(RevisionCandidate {
            universe,
            control,
            sink: CandidateSink::Advance(sink),
            memo,
            completed: None,
            suspension_serial: 0,
            delivered_commands: 0,
            effect_start,
            job_start_captured: true,
            execution_budgets: tex_exec::ExecutionBudgets::default(),
            kind: RevisionCandidateKind::Incremental {
                setup,
                restart,
                restart_fork_latency,
            },
        })
    }

    fn start_replacement_candidate(
        &self,
        setup: Box<AdvanceSetup>,
    ) -> Result<RevisionCandidate, SessionError> {
        let mut memo = self.pure_memo.clone();
        let mut universe = self.template.clone();
        universe.begin_retained_session()?;
        universe.install_editor_fragments(&setup.fragments, &setup.next_layout)?;
        universe.set_root_editor_content_hash(ContentHash::from_bytes(setup.next.as_bytes()));
        let control = canonical_candidate_control(
            &mut universe,
            &self.job_name,
            setup.next_layout.path(),
            self.source_file_bytes(&setup.next),
            self.effective_command_profile(),
            self.initex,
            self.dvi_output,
            self.root_framing,
            self.root_framing_name.as_deref(),
        )?;
        memo.begin_paragraph_history(false);
        universe.install_pure_memo_runtime(std::mem::take(&mut memo));
        let effect_start = universe.world().effect_records().len();
        Ok(RevisionCandidate {
            universe,
            control,
            sink: CandidateSink::Cold(HistorySink::default()),
            memo,
            completed: None,
            suspension_serial: 0,
            delivered_commands: 0,
            effect_start,
            job_start_captured: false,
            execution_budgets: tex_exec::ExecutionBudgets::default(),
            kind: RevisionCandidateKind::Replacement { setup },
        })
    }

    /// Converts a completed edited candidate into a private pending revision.
    pub fn finish_advance_candidate(
        &mut self,
        candidate: RevisionCandidate,
    ) -> Result<PendingRevision, SessionError> {
        match &candidate.kind {
            RevisionCandidateKind::Replacement { .. } => {
                self.finish_replacement_candidate(candidate)
            }
            RevisionCandidateKind::Incremental { .. } => {
                self.finish_incremental_candidate(candidate)
            }
            RevisionCandidateKind::Initial { .. } => Err(SessionError::CandidateKindMismatch),
        }
    }

    fn finish_replacement_candidate(
        &self,
        mut candidate: RevisionCandidate,
    ) -> Result<PendingRevision, SessionError> {
        let RevisionCandidateKind::Replacement { setup } = candidate.kind else {
            return Err(SessionError::CandidateKindMismatch);
        };
        let stats = candidate
            .completed
            .take()
            .ok_or(SessionError::CandidateNotComplete)?;
        let CandidateSink::Cold(mut sink) = candidate.sink else {
            return Err(SessionError::CandidateKindMismatch);
        };
        let before_memo = self.pure_memo.stats();
        let mut memo = candidate.universe.take_pure_memo_runtime();
        memo.accept_paragraph_history(candidate.universe.paragraph_origin_resolver());
        let paragraph_replay = paragraph_replay_delta(before_memo, memo.stats());
        for record in &mut sink.records {
            record.revision = setup.next_revision;
        }
        let effects = candidate.universe.world().effect_records().to_vec();
        let artifacts = candidate.universe.world().committed_artifacts().to_vec();
        let expansion_stats = ExpansionStats::default();
        let CanonicalCandidateCompletion {
            dvi_pages,
            dumped_format,
            format_dump_receipt,
            delivered_tokens,
            main_control_dispatches,
        } = stats;
        let substrate = candidate.universe.freeze_generation();
        let history = retain_restorable_history(sink.records, &substrate)?;
        let reuse = ReuseMetrics {
            execution_path: setup.execution_path,
            pages_retyped: artifacts.len(),
            reexecuted_bytes: setup.next.len(),
            reexecuted_tokens: delivered_tokens,
            reexecuted_commands: main_control_dispatches,
            reexecuted_macro_text_span_tokens: 0,
            reexecuted_source_text_span_tokens: 0,
            reexecuted_paragraphs: history
                .iter()
                .filter(|record| record.key.boundary == EngineBoundary::OuterParagraphEnd)
                .count(),
            paragraph_replay_lookups: paragraph_replay.lookups,
            paragraph_replay_hits: paragraph_replay.hits,
            paragraph_replay_validation_misses: paragraph_replay.validation_misses,
            revision_setup_latency: setup.revision_setup_latency,
            ..ReuseMetrics::default()
        };
        let content_hash = ContentHash::from_bytes(setup.next.as_bytes());
        Ok(PendingRevision {
            session_output_id: self.output_id,
            base_revision: self.revision,
            base_content_hash: self.content_hash,
            revision: setup.next_revision,
            source: setup.next,
            fragments: setup.fragments,
            layout: setup.next_layout,
            content_hash,
            effects,
            artifacts,
            dvi_pages,
            history,
            substrate: PendingSubstrate::Replaced(substrate),
            reuse,
            dumped_format,
            format_dump_receipt,
            expansion_stats,
            candidate_memo: Some(memo),
        })
    }

    fn finish_incremental_candidate(
        &self,
        mut candidate: RevisionCandidate,
    ) -> Result<PendingRevision, SessionError> {
        let RevisionCandidateKind::Incremental {
            setup,
            restart,
            restart_fork_latency,
        } = candidate.kind
        else {
            return Err(SessionError::CandidateKindMismatch);
        };
        let stats = candidate
            .completed
            .take()
            .ok_or(SessionError::CandidateNotComplete)?;
        let CandidateSink::Advance(sink) = candidate.sink else {
            return Err(SessionError::CandidateKindMismatch);
        };
        let before_memo = self.pure_memo.stats();
        let mut memo = candidate.universe.take_pure_memo_runtime();
        let paragraph_replay = paragraph_replay_delta(before_memo, memo.stats());
        let CanonicalCandidateCompletion {
            dvi_pages,
            dumped_format,
            format_dump_receipt: _,
            delivered_tokens,
            main_control_dispatches,
        } = stats;
        let reexecuted_paragraphs = sink
            .records
            .iter()
            .filter(|record| record.key.boundary == EngineBoundary::OuterParagraphEnd)
            .count();
        let reexecuted_through = sink
            .records
            .last()
            .map_or(setup.next.len(), |record| record.key.position);
        let same_history_stop = if !sink.allow_convergence {
            SameHistoryStop::NotAttempted
        } else if sink.convergence_old_index.is_some() {
            SameHistoryStop::Matched
        } else if sink.schedule_diverged {
            SameHistoryStop::ScheduleDiverged
        } else if sink.same_history_attempts > 0 {
            SameHistoryStop::HashesDiverged
        } else {
            SameHistoryStop::NoComparableBoundary
        };
        let expansion_stats = ExpansionStats::default();
        let effects = candidate.universe.world().effect_records().to_vec();
        let artifacts = candidate.universe.world().committed_artifacts().to_vec();
        let mut pages_through_stop =
            setup.old_pages[..setup.old_history[restart].artifact_prefix].to_vec();
        pages_through_stop.extend(dvi_pages);

        let roots = tex_exec::RootRehomeContext::new(&setup.old_source, &setup.next);
        let paragraph_history_transition_started = Timer::start();
        if sink.convergence_old_index.is_some() {
            memo.discard_paragraph_history();
        } else {
            memo.accept_paragraph_history(candidate.universe.paragraph_origin_resolver());
        }
        let paragraph_history_transition_latency = paragraph_history_transition_started.elapsed();
        let splice_started = Timer::start();
        let substrate = self
            .substrate
            .as_ref()
            .ok_or(SessionError::MissingAcceptedSubstrate)?;
        let anchor = &setup.old_history[restart];
        let (effects, artifacts, pages, mut history, pending_substrate, mut reuse) =
            if let Some(old_index) = sink.convergence_old_index {
                let old_effect_prefix = setup.old_history[old_index].effect_prefix;
                let new_effect_prefix = sink
                    .records
                    .last()
                    .expect("convergence requires a new matching record")
                    .effect_prefix;
                let scratch_effect_count = new_effect_prefix.saturating_sub(anchor.effect_prefix);
                let mut joined_effects = setup.old_effects[..anchor.effect_prefix].to_vec();
                joined_effects.extend_from_slice(&effects[..scratch_effect_count]);
                joined_effects.extend_from_slice(&setup.old_effects[old_effect_prefix..]);

                let old_prefix = setup.old_history[old_index].artifact_prefix;
                let new_prefix = sink
                    .records
                    .last()
                    .expect("convergence requires a new matching record")
                    .artifact_prefix;
                let scratch_artifact_count = new_prefix.saturating_sub(anchor.artifact_prefix);
                let mut joined_artifacts = setup.old_artifacts[..anchor.artifact_prefix].to_vec();
                joined_artifacts.extend_from_slice(&artifacts[..scratch_artifact_count]);
                joined_artifacts.extend_from_slice(&setup.old_artifacts[old_prefix..]);
                rebase_and_validate_adopted_artifacts(
                    &mut joined_artifacts[anchor.artifact_prefix + scratch_artifact_count..],
                    old_effect_prefix,
                    new_effect_prefix,
                    &joined_effects,
                )?;
                let mut joined_pages = pages_through_stop;
                joined_pages.extend_from_slice(&setup.old_pages[old_prefix..]);
                let mut history = Vec::with_capacity(
                    restart + 1 + setup.old_history.len().saturating_sub(old_index),
                );
                for mut record in setup.old_history[..=restart].iter().cloned() {
                    record.checkpoint = record
                        .checkpoint
                        .rehome_unchanged_prefix(substrate, &roots)?;
                    history.push(record);
                }
                for mut record in setup.old_history[old_index..].iter().cloned() {
                    let mapped_position = setup
                        .map
                        .map(record.key.position)
                        .expect("adopted suffix anchors were validated as mappable");
                    record.key.position = mapped_position;
                    record.checkpoint = record.checkpoint.rehome_converged_root(
                        substrate,
                        &roots,
                        mapped_position,
                    )?;
                    record.revision = setup.next_revision;
                    history.push(record);
                }
                let adopted_origins = artifacts[..scratch_artifact_count]
                    .iter()
                    .flat_map(|artifact| artifact.live_render_origins().iter())
                    .copied()
                    .collect::<Vec<_>>();
                let convergence_boundary = history.get(restart + 1).map(BoundaryRecord::key);
                (
                    joined_effects,
                    joined_artifacts,
                    joined_pages,
                    history,
                    PendingSubstrate::Retained {
                        scratch: candidate.universe,
                        adopted_origins,
                    },
                    ReuseMetrics {
                        execution_path: match setup.execution_path {
                            RevisionExecutionPath::SlowEdit => RevisionExecutionPath::FastEdit,
                            path => path,
                        },
                        restart_boundary: Some(anchor.key),
                        convergence_boundary,
                        pages_retained_prefix: anchor.artifact_prefix,
                        pages_reused: setup.old_artifacts.len().saturating_sub(old_prefix),
                        pages_retyped: scratch_artifact_count,
                        reexecuted_bytes: reexecuted_through.saturating_sub(anchor.key.position),
                        reexecuted_tokens: delivered_tokens,
                        reexecuted_commands: main_control_dispatches,
                        reexecuted_macro_text_span_tokens: 0,
                        reexecuted_source_text_span_tokens: 0,
                        reexecuted_paragraphs,
                        paragraph_replay_lookups: paragraph_replay.lookups,
                        paragraph_replay_hits: paragraph_replay.hits,
                        paragraph_replay_validation_misses: paragraph_replay.validation_misses,
                        same_history_attempts: sink.same_history_attempts,
                        same_history_hash_mismatches: sink.same_history_hash_mismatches,
                        trace_nodes_walked: sink.same_history_attempts,
                        trace_leaf_hits: setup.old_artifacts.len().saturating_sub(old_prefix),
                        trace_subtree_hits: 1,
                        suffixes_adopted: 1,
                        same_history_stop,
                        restart_fork_latency,
                        revision_setup_latency: setup.revision_setup_latency,
                        paragraph_history_transition_latency,
                        trace_validation_latency: sink.trace_validation_latency,
                        ..ReuseMetrics::default()
                    },
                )
            } else {
                let target = candidate.universe.freeze_generation();
                let mut history = Vec::with_capacity(restart + 1 + sink.records.len());
                for record in &setup.old_history[..=restart] {
                    let mut record = record.clone();
                    record.checkpoint = record
                        .checkpoint
                        .retarget_prefix(&target, substrate, &roots)?;
                    record.revision = setup.next_revision;
                    history.push(record);
                }
                history.extend(sink.records);
                let pages_retyped = artifacts.len();
                let mut joined_artifacts = setup.old_artifacts[..anchor.artifact_prefix].to_vec();
                joined_artifacts.extend(artifacts);
                let mut joined_effects = setup.old_effects[..anchor.effect_prefix].to_vec();
                joined_effects.extend(effects);
                (
                    joined_effects,
                    joined_artifacts,
                    pages_through_stop,
                    history,
                    PendingSubstrate::Replaced(target),
                    ReuseMetrics {
                        execution_path: setup.execution_path,
                        restart_boundary: Some(anchor.key),
                        pages_retained_prefix: anchor.artifact_prefix,
                        pages_retyped,
                        reexecuted_bytes: reexecuted_through.saturating_sub(anchor.key.position),
                        reexecuted_tokens: delivered_tokens,
                        reexecuted_commands: main_control_dispatches,
                        reexecuted_macro_text_span_tokens: 0,
                        reexecuted_source_text_span_tokens: 0,
                        reexecuted_paragraphs,
                        paragraph_replay_lookups: paragraph_replay.lookups,
                        paragraph_replay_hits: paragraph_replay.hits,
                        paragraph_replay_validation_misses: paragraph_replay.validation_misses,
                        same_history_attempts: sink.same_history_attempts,
                        same_history_hash_mismatches: sink.same_history_hash_mismatches,
                        trace_nodes_walked: sink.same_history_attempts,
                        same_history_stop,
                        restart_fork_latency,
                        revision_setup_latency: setup.revision_setup_latency,
                        paragraph_history_transition_latency,
                        trace_validation_latency: sink.trace_validation_latency,
                        ..ReuseMetrics::default()
                    },
                )
            };
        for record in &mut history {
            record.revision = setup.next_revision;
        }
        let retained_substrate = match &pending_substrate {
            PendingSubstrate::Retained { .. } => substrate,
            PendingSubstrate::Replaced(substrate) => substrate,
        };
        let history = retain_restorable_history(history, retained_substrate)?;
        reuse.trace_retained_bytes = std::mem::size_of_val(history.as_slice());
        reuse.splice_latency = splice_started.elapsed();
        reuse.trace_replay_latency = reuse.splice_latency;
        Ok(PendingRevision {
            session_output_id: self.output_id,
            base_revision: self.revision,
            base_content_hash: self.content_hash,
            revision: setup.next_revision,
            content_hash: roots.new_content_hash(),
            source: setup.next,
            fragments: setup.fragments,
            layout: setup.next_layout,
            effects,
            artifacts,
            dvi_pages: pages,
            history,
            substrate: pending_substrate,
            reuse,
            dumped_format,
            expansion_stats,
            format_dump_receipt: None,
            candidate_memo: Some(memo),
        })
    }

    fn cold_with_optional_image_resolver(
        &mut self,
        input_resolver: &mut dyn InputResolver,
        font_resolver: &mut dyn tex_exec::FontResolver,
        image_resolver: Option<&mut dyn tex_exec::PdfImageResolver>,
    ) -> Result<AcceptedOutput, SessionError> {
        let run = execute_revision(
            &self.template,
            &mut self.pure_memo,
            &self.job_name,
            &self.source,
            &self.fragments,
            &self.layout,
            self.utf8_input_as_bytes,
            self.root_source_is_byte_projection,
            input_resolver,
            font_resolver,
            image_resolver,
        )?;
        self.accept_cold(run)
    }

    /// Adds immutable host input to the template used by a not-yet-accepted
    /// initial revision or a retry that discovered a new resource.
    pub fn register_input_file(&mut self, path: &Path, bytes: Vec<u8>) -> Result<(), SessionError> {
        self.template
            .world_mut()
            .set_memory_file(path, bytes.clone())?;
        self.registered_inputs.insert(path.to_owned(), bytes);
        Ok(())
    }

    /// Materializes the currently accepted detached effects without consuming
    /// the checkpoints required by later edits.
    pub fn materialize_accepted_world(&self) -> Result<tex_state::World, SessionError> {
        let substrate = self
            .substrate
            .as_ref()
            .ok_or(SessionError::MissingAcceptedSubstrate)?;
        Ok(substrate.materialize_detached_outputs(self.effects.clone(), self.artifacts.clone())?)
    }

    /// Consumes the accepted session into the reached engine state with its
    /// detached effects still uncommitted. This is the client finalization
    /// boundary for one-shot drivers.
    pub fn into_accepted_universe(mut self) -> Result<AcceptedUniverseFinalization, SessionError> {
        let substrate = self
            .substrate
            .take()
            .ok_or(SessionError::MissingAcceptedSubstrate)?;
        let mut universe = substrate.into_detached_universe(self.effects, self.artifacts)?;
        let first_fallible_page =
            universe
                .world()
                .committed_artifacts()
                .iter()
                .position(|artifact| {
                    tex_out::PageArtifact::from_bytes(artifact.bytes()).is_ok_and(|page| {
                        page.effects
                            .iter()
                            .any(|effect| matches!(effect, tex_out::PageEffect::OpenOut { .. }))
                    })
                });
        let prepared_pages = first_fallible_page.map(|start| universe.prepare_page_suffix(start));
        Ok(AcceptedUniverseFinalization {
            universe,
            prepared_pages,
        })
    }

    #[must_use]
    pub const fn accepted_dumped_format(&self) -> bool {
        self.dumped_format
    }

    #[must_use]
    pub fn accepted_format_dump_receipt(&self) -> Option<&tex_exec::FormatDumpReceipt> {
        self.format_dump_receipt.as_ref()
    }

    #[must_use]
    pub const fn accepted_expansion_stats(&self) -> ExpansionStats {
        self.expansion_stats
    }

    /// Resolves one rendered HTML event/unit against the accepted revision.
    pub fn rendered_source_location(
        &self,
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
        match self.rendered_source_origin(page, event, unit)? {
            Some(LayoutResolvedOrigin::Current {
                path,
                doc_offset_lo,
                doc_offset_hi,
                line,
                column,
            }) => Ok(Some(RenderedSourceResult::Current(
                tex_state::ResolvedSourceLocation {
                    path,
                    start: doc_offset_lo,
                    end: doc_offset_hi,
                    line,
                    column,
                },
            ))),
            Some(LayoutResolvedOrigin::Foreign) => {
                let Some(origin) = self.rendered_origin(page, event, unit)? else {
                    return Ok(None);
                };
                let substrate = self
                    .substrate
                    .as_ref()
                    .ok_or(SessionError::MissingAcceptedSubstrate)?;
                Ok(substrate
                    .resolve_origin(origin)
                    .map(RenderedSourceResult::Current))
            }
            Some(LayoutResolvedOrigin::Deleted { minted_revision }) => {
                Ok(Some(RenderedSourceResult::Deleted { minted_revision }))
            }
            Some(LayoutResolvedOrigin::Unknown) | None => Ok(None),
        }
    }

    /// Resolves one rendered unit with typed current/deleted editor semantics.
    pub fn rendered_source_origin(
        &self,
        page: u32,
        event: u32,
        unit: Option<u32>,
    ) -> Result<Option<LayoutResolvedOrigin>, SessionError> {
        let Some(origin) = self.rendered_artifact_origin(page, event, unit)? else {
            return Ok(None);
        };
        let substrate = self
            .substrate
            .as_ref()
            .ok_or(SessionError::MissingAcceptedSubstrate)?;
        Ok(Some(match origin {
            ArtifactOrigin::Live(origin) => {
                substrate.resolve_layout_origin(origin, &self.fragments, &self.layout)
            }
            ArtifactOrigin::Stable(span) => {
                substrate.resolve_stable_layout_origin(span, &self.fragments, &self.layout)
            }
            ArtifactOrigin::Unknown => return Ok(None),
        }))
    }

    fn rendered_origin(
        &self,
        page: u32,
        event: u32,
        unit: Option<u32>,
    ) -> Result<Option<OriginId>, SessionError> {
        Ok(match self.rendered_artifact_origin(page, event, unit)? {
            Some(ArtifactOrigin::Live(origin)) => Some(origin),
            Some(ArtifactOrigin::Stable(_) | ArtifactOrigin::Unknown) | None => None,
        })
    }

    fn rendered_artifact_origin(
        &self,
        page: u32,
        event: u32,
        unit: Option<u32>,
    ) -> Result<Option<ArtifactOrigin>, SessionError> {
        let Some(page_index) = page.checked_sub(1).map(|page| page as usize) else {
            return Ok(None);
        };
        let Some(artifact) = self.artifacts.get(page_index) else {
            return Ok(None);
        };
        let mut maps = self.render_maps.borrow_mut();
        if maps.pages.len() <= page_index {
            maps.pages.resize_with(page_index + 1, || None);
            #[cfg(test)]
            maps.page_lowerings.resize(page_index + 1, 0);
        }
        if maps.pages[page_index].is_none() {
            maps.pages[page_index] = Some(build_page_render_map(artifact, page)?);
            #[cfg(test)]
            {
                maps.page_lowerings[page_index] += 1;
            }
        }
        Ok(maps.pages[page_index]
            .as_ref()
            .and_then(|map| map.origin(event, unit)))
    }

    fn clear_render_maps(&self) {
        *self.render_maps.borrow_mut() = RenderMapCache::default();
    }

    #[cfg(test)]
    fn page_lowerings(&self, page: u32) -> usize {
        let Some(index) = page.checked_sub(1).map(|page| page as usize) else {
            return 0;
        };
        self.render_maps
            .borrow()
            .page_lowerings
            .get(index)
            .copied()
            .unwrap_or(0)
    }

    /// Consumes the rollback-capable session and materializes its accepted
    /// effect history once. Further edits require constructing a new Session.
    pub fn finalize(mut self) -> Result<tex_state::World, SessionError> {
        let substrate = self
            .substrate
            .take()
            .ok_or(SessionError::MissingAcceptedSubstrate)?;
        Ok(substrate.export_detached_outputs(self.effects, self.artifacts)?)
    }

    #[allow(clippy::disallowed_methods)] // Session telemetry; no TeX state observes it.
    pub fn advance(
        &mut self,
        next_revision: RevisionId,
        edit: Edit,
    ) -> Result<AcceptedOutput, SessionError> {
        let mut input_resolver = DirectInputResolver;
        let mut font_resolver = DirectFontResolver;
        self.advance_with_resolvers(next_revision, edit, &mut input_resolver, &mut font_resolver)
    }

    pub fn advance_with_resolvers(
        &mut self,
        next_revision: RevisionId,
        edit: Edit,
        input_resolver: &mut dyn InputResolver,
        font_resolver: &mut dyn tex_exec::FontResolver,
    ) -> Result<AcceptedOutput, SessionError> {
        let pending = self.prepare_advance_with_resolvers(
            next_revision,
            edit,
            input_resolver,
            font_resolver,
        )?;
        self.accept_pending(pending)
    }

    pub fn advance_with_resource_resolvers(
        &mut self,
        next_revision: RevisionId,
        edit: Edit,
        input_resolver: &mut dyn InputResolver,
        font_resolver: &mut dyn tex_exec::FontResolver,
        image_resolver: &mut dyn tex_exec::PdfImageResolver,
    ) -> Result<AcceptedOutput, SessionError> {
        let pending = self.prepare_advance_with_resource_resolvers(
            next_revision,
            edit,
            input_resolver,
            font_resolver,
            image_resolver,
        )?;
        self.accept_pending(pending)
    }

    /// Executes an edit into private candidate state without changing the
    /// accepted revision. The caller may validate all downstream output and
    /// either atomically accept the candidate or drop it.
    pub fn prepare_advance_with_resolvers(
        &mut self,
        next_revision: RevisionId,
        edit: Edit,
        input_resolver: &mut dyn InputResolver,
        font_resolver: &mut dyn tex_exec::FontResolver,
    ) -> Result<PendingRevision, SessionError> {
        self.prepare_advance_with_optional_image_resolver(
            next_revision,
            edit,
            input_resolver,
            font_resolver,
            None,
        )
    }

    pub fn prepare_advance_with_resource_resolvers(
        &mut self,
        next_revision: RevisionId,
        edit: Edit,
        input_resolver: &mut dyn InputResolver,
        font_resolver: &mut dyn tex_exec::FontResolver,
        image_resolver: &mut dyn tex_exec::PdfImageResolver,
    ) -> Result<PendingRevision, SessionError> {
        self.prepare_advance_with_optional_image_resolver(
            next_revision,
            edit,
            input_resolver,
            font_resolver,
            Some(image_resolver),
        )
    }

    fn prepare_advance_with_optional_image_resolver(
        &mut self,
        next_revision: RevisionId,
        edit: Edit,
        input_resolver: &mut dyn InputResolver,
        font_resolver: &mut dyn tex_exec::FontResolver,
        image_resolver: Option<&mut dyn tex_exec::PdfImageResolver>,
    ) -> Result<PendingRevision, SessionError> {
        let revision_setup_started = Timer::start();
        self.validate_edit(next_revision, &edit)?;
        let before_memo = self.pure_memo.stats();
        self.clear_render_maps();
        let old_source = self.source.clone();
        let old_history = self.history.clone();
        let old_effects = self.effects.clone();
        let old_artifacts = self.artifacts.clone();
        let old_pages = self.dvi_pages.clone();
        let mut next = old_source.clone();
        next.replace_range(edit.range.clone(), &edit.replacement);
        let (expanded_range, expanded_replacement) = line_expanded_replacement(&old_source, &edit);
        let mut fragments = self.fragments.clone();
        let (fragment, _) = fragments.append(
            Arc::from(expanded_replacement.as_bytes()),
            next_revision.raw(),
        )?;
        let next_layout = replace_layout_range(
            &self.layout,
            &fragments,
            expanded_range,
            fragment,
            expanded_replacement.len(),
            LayoutGeneration::new(next_revision.raw()),
        )?;
        let restart_index = select_restart(&old_history, &old_source, &next, &edit);
        let map = EditMap::new(edit.range.clone(), edit.replacement.len());
        self.substrate
            .as_ref()
            .ok_or(SessionError::MissingAcceptedSubstrate)?
            .world()
            .validate_recorded_inputs()?;
        let revision_setup_latency = revision_setup_started.elapsed();
        let Some(restart_index) = restart_index else {
            let mut run = execute_revision(
                &self.template,
                &mut self.pure_memo,
                &self.job_name,
                &next,
                &fragments,
                &next_layout,
                self.utf8_input_as_bytes,
                self.root_source_is_byte_projection,
                input_resolver,
                font_resolver,
                image_resolver,
            )?;
            for record in &mut run.history {
                record.revision = next_revision;
            }
            let history = retain_restorable_history(run.history, &run.substrate)?;
            let paragraph_replay = paragraph_replay_delta(before_memo, self.pure_memo.stats());
            let reuse = ReuseMetrics {
                execution_path: RevisionExecutionPath::SlowEdit,
                pages_retyped: run.artifacts.len(),
                reexecuted_bytes: run.executed_bytes,
                reexecuted_tokens: run.executed_tokens,
                reexecuted_commands: run.executed_commands,
                reexecuted_macro_text_span_tokens: run.executed_macro_text_span_tokens,
                reexecuted_source_text_span_tokens: run.executed_source_text_span_tokens,
                reexecuted_paragraphs: run.executed_paragraphs,
                paragraph_replay_lookups: paragraph_replay.lookups,
                paragraph_replay_hits: paragraph_replay.hits,
                paragraph_replay_validation_misses: paragraph_replay.validation_misses,
                ..ReuseMetrics::default()
            };
            return Ok(PendingRevision {
                session_output_id: self.output_id,
                base_revision: self.revision,
                base_content_hash: self.content_hash,
                revision: next_revision,
                content_hash: ContentHash::from_bytes(next.as_bytes()),
                source: next,
                fragments,
                layout: next_layout,
                effects: run.effects,
                artifacts: run.artifacts,
                dvi_pages: run.dvi_pages,
                history,
                substrate: PendingSubstrate::Replaced(run.substrate),
                reuse,
                dumped_format: run.dumped_format,
                format_dump_receipt: run.format_dump_receipt,
                expansion_stats: run.expansion_stats,
                candidate_memo: None,
            });
        };
        let substrate = self
            .substrate
            .as_ref()
            .ok_or(SessionError::MissingAcceptedSubstrate)?;
        let advance = execute_advance(
            &self.template,
            &mut self.pure_memo,
            substrate,
            &self.job_name,
            &old_source,
            &next,
            &old_history,
            &old_pages,
            &fragments,
            &next_layout,
            restart_index,
            &map,
            input_resolver,
            font_resolver,
            image_resolver,
            &self.registered_inputs,
        )?;

        let restart_fork_latency = advance.restart_fork_latency;
        let executor_latency = advance.executor_latency;
        let reexecution_latency = advance.reexecution_latency;
        let output_snapshot_latency = advance.output_snapshot_latency;
        let reexecuted_bytes = advance.reexecuted_bytes;
        let reexecuted_tokens = advance.reexecuted_tokens;
        let reexecuted_commands = advance.reexecuted_commands;
        let reexecuted_macro_text_span_tokens = advance.reexecuted_macro_text_span_tokens;
        let reexecuted_source_text_span_tokens = advance.reexecuted_source_text_span_tokens;
        let reexecuted_paragraphs = advance.reexecuted_paragraphs;
        let same_history_attempts = advance.same_history_attempts;
        let same_history_hash_mismatches = advance.same_history_hash_mismatches;
        let trace_validation_latency = advance.trace_validation_latency;
        let same_history_stop = advance.same_history_stop;
        let roots = tex_exec::RootRehomeContext::new(&old_source, &next);
        let paragraph_history_transition_started = Timer::start();
        if advance.convergence_old_index.is_some() {
            self.pure_memo.discard_paragraph_history();
        } else {
            self.pure_memo
                .accept_paragraph_history(advance.scratch.paragraph_origin_resolver());
        }
        let paragraph_history_transition_latency = paragraph_history_transition_started.elapsed();
        let paragraph_replay = paragraph_replay_delta(before_memo, self.pure_memo.stats());
        let splice_started = Timer::start();
        let (effects, artifacts, pages, mut history, pending_substrate, mut reuse) =
            if let Some(old_index) = advance.convergence_old_index {
                let old_effect_prefix = old_history[old_index].effect_prefix;
                let new_effect_prefix = advance
                    .new_records
                    .last()
                    .expect("convergence requires a new matching record")
                    .effect_prefix;
                let restart_effect_prefix = old_history[restart_index].effect_prefix;
                let scratch_effect_count = new_effect_prefix.saturating_sub(restart_effect_prefix);
                let mut effects = old_effects[..restart_effect_prefix].to_vec();
                effects.extend_from_slice(&advance.effects[..scratch_effect_count]);
                effects.extend_from_slice(&old_effects[old_effect_prefix..]);
                let old_prefix = old_history[old_index].artifact_prefix;
                let new_prefix = advance
                    .new_records
                    .last()
                    .expect("convergence requires a new matching record")
                    .artifact_prefix;
                let restart_artifact_prefix = old_history[restart_index].artifact_prefix;
                let scratch_artifact_count = new_prefix.saturating_sub(restart_artifact_prefix);
                let mut artifacts = old_artifacts[..restart_artifact_prefix].to_vec();
                artifacts.extend_from_slice(&advance.artifacts[..scratch_artifact_count]);
                artifacts.extend_from_slice(&old_artifacts[old_prefix..]);
                rebase_and_validate_adopted_artifacts(
                    &mut artifacts[restart_artifact_prefix + scratch_artifact_count..],
                    old_effect_prefix,
                    new_effect_prefix,
                    &effects,
                )?;
                let mut pages = advance.pages_through_stop;
                pages.extend_from_slice(&old_pages[old_prefix..]);
                let mut history = Vec::with_capacity(
                    restart_index + 1 + old_history.len().saturating_sub(old_index),
                );
                for mut record in old_history[..=restart_index].iter().cloned() {
                    record.checkpoint = record
                        .checkpoint
                        .rehome_unchanged_prefix(substrate, &roots)?;
                    history.push(record);
                }
                for mut record in old_history[old_index..].iter().cloned() {
                    let mapped_position = map
                        .map(record.key.position)
                        .expect("adopted suffix anchors were validated as mappable");
                    record.key.position = mapped_position;
                    record.checkpoint = record.checkpoint.rehome_converged_root(
                        substrate,
                        &roots,
                        mapped_position,
                    )?;
                    record.revision = next_revision;
                    history.push(record);
                }
                let adopted_origins = advance.artifacts[..scratch_artifact_count]
                    .iter()
                    .flat_map(|artifact| artifact.live_render_origins().iter())
                    .copied()
                    .collect::<Vec<_>>();
                let convergence_boundary = history.get(restart_index + 1).map(BoundaryRecord::key);
                (
                    effects,
                    artifacts,
                    pages,
                    history,
                    PendingSubstrate::Retained {
                        scratch: advance.scratch,
                        adopted_origins,
                    },
                    ReuseMetrics {
                        execution_path: RevisionExecutionPath::FastEdit,
                        restart_boundary: old_history.get(restart_index).map(BoundaryRecord::key),
                        convergence_boundary,
                        pages_retained_prefix: restart_artifact_prefix,
                        pages_reused: old_artifacts.len().saturating_sub(old_prefix),
                        pages_retyped: scratch_artifact_count,
                        reexecuted_bytes,
                        reexecuted_tokens,
                        reexecuted_commands,
                        reexecuted_macro_text_span_tokens,
                        reexecuted_source_text_span_tokens,
                        reexecuted_paragraphs,
                        paragraph_replay_lookups: paragraph_replay.lookups,
                        paragraph_replay_hits: paragraph_replay.hits,
                        paragraph_replay_validation_misses: paragraph_replay.validation_misses,
                        same_history_attempts,
                        same_history_hash_mismatches,
                        trace_nodes_walked: same_history_attempts,
                        trace_leaf_hits: old_artifacts.len().saturating_sub(old_prefix),
                        trace_subtree_hits: 1,
                        suffixes_adopted: 1,
                        same_history_stop,
                        restart_fork_latency,
                        revision_setup_latency,
                        executor_latency,
                        reexecution_latency,
                        output_snapshot_latency,
                        paragraph_history_transition_latency,
                        trace_validation_latency,
                        ..ReuseMetrics::default()
                    },
                )
            } else {
                let target = advance.scratch.freeze_generation();
                let mut history = Vec::with_capacity(restart_index + 1 + advance.new_records.len());
                for record in &old_history[..=restart_index] {
                    let mut record = record.clone();
                    record.checkpoint = record
                        .checkpoint
                        .retarget_prefix(&target, substrate, &roots)?;
                    record.revision = next_revision;
                    history.push(record);
                }
                history.extend(advance.new_records);
                let pages_retyped = advance.artifacts.len();
                let mut artifacts =
                    old_artifacts[..old_history[restart_index].artifact_prefix].to_vec();
                artifacts.extend(advance.artifacts);
                (
                    {
                        let mut effects =
                            old_effects[..old_history[restart_index].effect_prefix].to_vec();
                        effects.extend(advance.effects);
                        effects
                    },
                    artifacts,
                    advance.pages_through_stop,
                    history,
                    PendingSubstrate::Replaced(target),
                    ReuseMetrics {
                        execution_path: RevisionExecutionPath::SlowEdit,
                        restart_boundary: old_history.get(restart_index).map(BoundaryRecord::key),
                        convergence_boundary: None,
                        pages_retained_prefix: old_history[restart_index].artifact_prefix,
                        pages_reused: 0,
                        pages_retyped,
                        reexecuted_bytes,
                        reexecuted_tokens,
                        reexecuted_commands,
                        reexecuted_macro_text_span_tokens,
                        reexecuted_source_text_span_tokens,
                        reexecuted_paragraphs,
                        paragraph_replay_lookups: paragraph_replay.lookups,
                        paragraph_replay_hits: paragraph_replay.hits,
                        paragraph_replay_validation_misses: paragraph_replay.validation_misses,
                        same_history_attempts,
                        same_history_hash_mismatches,
                        trace_nodes_walked: same_history_attempts,
                        trace_leaf_hits: 0,
                        trace_subtree_hits: 0,
                        suffixes_adopted: 0,
                        same_history_stop,
                        restart_fork_latency,
                        revision_setup_latency,
                        executor_latency,
                        reexecution_latency,
                        output_snapshot_latency,
                        paragraph_history_transition_latency,
                        trace_validation_latency,
                        ..ReuseMetrics::default()
                    },
                )
            };
        for record in &mut history {
            record.revision = next_revision;
        }
        let retained_substrate = match &pending_substrate {
            PendingSubstrate::Retained { .. } => self
                .substrate
                .as_ref()
                .ok_or(SessionError::MissingAcceptedSubstrate)?,
            PendingSubstrate::Replaced(substrate) => substrate,
        };
        let history = retain_restorable_history(history, retained_substrate)?;
        reuse.trace_retained_bytes = std::mem::size_of_val(history.as_slice());
        let content_hash = roots.new_content_hash();
        reuse.splice_latency = splice_started.elapsed();
        reuse.trace_replay_latency = reuse.splice_latency;
        Ok(PendingRevision {
            session_output_id: self.output_id,
            base_revision: self.revision,
            base_content_hash: self.content_hash,
            revision: next_revision,
            source: next,
            fragments,
            layout: next_layout,
            content_hash,
            effects,
            artifacts,
            dvi_pages: pages,
            history,
            substrate: pending_substrate,
            reuse,
            dumped_format: advance.dumped_format,
            format_dump_receipt: advance.format_dump_receipt,
            expansion_stats: advance.expansion_stats,
            candidate_memo: None,
        })
    }

    /// Materializes detached effects for a prepared revision without
    /// publishing that revision into the session.
    pub fn materialize_pending_world(
        &self,
        pending: &PendingRevision,
    ) -> Result<tex_state::World, SessionError> {
        self.validate_pending(pending)?;
        let substrate = match &pending.substrate {
            PendingSubstrate::Retained { .. } => self
                .substrate
                .as_ref()
                .ok_or(SessionError::MissingAcceptedSubstrate)?,
            PendingSubstrate::Replaced(substrate) => substrate,
        };
        Ok(substrate
            .materialize_detached_outputs(pending.effects.clone(), pending.artifacts.clone())?)
    }

    /// Atomically replaces accepted editor state with one prepared revision.
    pub fn accept_pending(
        &mut self,
        pending: PendingRevision,
    ) -> Result<AcceptedOutput, SessionError> {
        let acceptance_started = Timer::start();
        self.validate_pending(&pending)?;
        let PendingRevision {
            revision,
            source,
            mut fragments,
            layout,
            content_hash,
            effects,
            artifacts,
            dvi_pages,
            history,
            substrate,
            reuse,
            dumped_format,
            format_dump_receipt,
            expansion_stats,
            candidate_memo,
            ..
        } = pending;

        let substrate_transition_started = Timer::start();
        match substrate {
            PendingSubstrate::Retained {
                scratch,
                adopted_origins,
            } => self
                .substrate
                .as_mut()
                .ok_or(SessionError::MissingAcceptedSubstrate)?
                .retain_artifact_origins_from_fork(&scratch, &adopted_origins)?,
            PendingSubstrate::Replaced(substrate) => self.substrate = Some(substrate),
        }
        let substrate_transition_latency = substrate_transition_started.elapsed();

        let substrate_bytes = self
            .substrate
            .as_ref()
            .expect("prepared revisions retain an accepted substrate")
            .charged_bytes();
        let output_bytes = output_bytes(&effects, &artifacts);
        let oldest_revision = oldest_retained_revision(&history, revision);
        fragments.prune_for_layout(&layout, revision.raw(), oldest_revision.raw());
        let diagnostic_bytes = fragments
            .retained_bytes()
            .saturating_add(layout.retained_bytes());
        let (history, mut retention) = prune_history(
            history,
            self.checkpoint_budget,
            substrate_bytes,
            diagnostic_bytes,
            output_bytes,
        );
        retention.memo_result_bytes = self.pure_memo.stats().retained_bytes;
        let pruned_oldest_revision = oldest_retained_revision(&history, revision);
        if pruned_oldest_revision > oldest_revision
            && fragments.prune_for_layout(&layout, revision.raw(), pruned_oldest_revision.raw()) > 0
        {
            retention.diagnostic_bytes = fragments
                .retained_bytes()
                .saturating_add(layout.retained_bytes());
            retention.protected_overage_bytes = retention
                .checkpoint_root_bytes
                .saturating_add(retention.diagnostic_bytes)
                .saturating_sub(self.checkpoint_budget);
        }

        self.clear_render_maps();
        self.revision = revision;
        self.source = source;
        self.fragments = fragments;
        self.layout = layout;
        self.content_hash = content_hash;
        self.effects = effects;
        self.artifacts = artifacts;
        self.dvi_pages = dvi_pages;
        self.history = history;
        self.dumped_format = dumped_format;
        self.format_dump_receipt = format_dump_receipt;
        self.expansion_stats = expansion_stats;
        if let Some(candidate_memo) = candidate_memo {
            self.pure_memo = candidate_memo;
        }
        self.accepted_retention = Some(retention);
        let mut output = self.output(reuse, retention);
        output.reuse.substrate_transition_latency = substrate_transition_latency;
        output.reuse.acceptance_latency = acceptance_started
            .elapsed()
            .saturating_sub(substrate_transition_latency);
        Ok(output)
    }

    fn validate_pending(&self, pending: &PendingRevision) -> Result<(), SessionError> {
        if pending.session_output_id != self.output_id
            || pending.base_revision != self.revision
            || pending.base_content_hash != self.content_hash
        {
            return Err(SessionError::StaleRevision {
                expected: self.revision,
                actual: pending.base_revision,
            });
        }
        Ok(())
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

    fn accept_cold(&mut self, mut run: RevisionRun) -> Result<AcceptedOutput, SessionError> {
        self.clear_render_maps();
        for record in &mut run.history {
            record.revision = self.revision;
        }
        run.history = retain_restorable_history(run.history, &run.substrate)?;
        let substrate_bytes = run.substrate.charged_bytes();
        let diagnostic_bytes = self.diagnostic_retained_bytes();
        let (history, mut retention) = prune_history(
            run.history,
            self.checkpoint_budget,
            substrate_bytes,
            diagnostic_bytes,
            run.output_bytes,
        );
        retention.memo_result_bytes = self.pure_memo.stats().retained_bytes;
        self.history = history;
        self.effects = run.effects;
        self.artifacts = run.artifacts;
        self.dvi_pages = run.dvi_pages;
        self.dumped_format = run.dumped_format;
        self.format_dump_receipt = run.format_dump_receipt;
        self.expansion_stats = run.expansion_stats;
        self.substrate = Some(run.substrate);
        self.accepted_retention = Some(retention);
        Ok(self.output(
            ReuseMetrics {
                pages_retyped: self.artifacts.len(),
                reexecuted_bytes: run.executed_bytes,
                reexecuted_tokens: run.executed_tokens,
                reexecuted_commands: run.executed_commands,
                reexecuted_macro_text_span_tokens: run.executed_macro_text_span_tokens,
                reexecuted_source_text_span_tokens: run.executed_source_text_span_tokens,
                reexecuted_paragraphs: run.executed_paragraphs,
                ..ReuseMetrics::default()
            },
            retention,
        ))
    }

    fn output(&self, reuse: ReuseMetrics, retention: RetentionMetrics) -> AcceptedOutput {
        AcceptedOutput {
            revision: self.revision,
            content_hash: self.content_hash,
            effects: self.effects.clone(),
            artifacts: self.artifacts.clone(),
            dvi_pages: self.dvi_pages.clone(),
            history: self.history.clone(),
            reuse,
            retention,
        }
    }

    fn diagnostic_retained_bytes(&self) -> usize {
        self.fragments
            .retained_bytes()
            .saturating_add(self.layout.retained_bytes())
    }
}

fn retain_restorable_history(
    history: Vec<BoundaryRecord>,
    substrate: &GenerationSubstrate,
) -> Result<Vec<BoundaryRecord>, SessionError> {
    let mut retained = Vec::with_capacity(history.len());
    for record in history {
        match record.checkpoint.validate_retained_by(substrate) {
            Ok(()) => retained.push(record),
            Err(GenerationForkError::InvalidatedSnapshot) => {}
            Err(error) => return Err(SessionError::Fork(error)),
        }
    }
    if retained.is_empty() {
        return Err(SessionError::MissingAcceptedSubstrate);
    }
    Ok(retained)
}

fn build_page_render_map(
    artifact: &CommittedArtifact,
    page: u32,
) -> Result<PageRenderMap, SessionError> {
    let page_artifact = tex_out::PageArtifact::from_bytes(artifact.bytes())
        .map_err(|error| SessionError::RenderSource(error.to_string()))?;
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

struct RevisionRun {
    history: Vec<BoundaryRecord>,
    effects: Vec<EffectRecord>,
    artifacts: Vec<CommittedArtifact>,
    dvi_pages: Vec<DviPagePlan>,
    output_bytes: usize,
    substrate: GenerationSubstrate,
    dumped_format: bool,
    format_dump_receipt: Option<tex_exec::FormatDumpReceipt>,
    expansion_stats: ExpansionStats,
    executed_bytes: usize,
    executed_tokens: usize,
    executed_commands: usize,
    executed_macro_text_span_tokens: usize,
    executed_source_text_span_tokens: usize,
    executed_paragraphs: usize,
}

struct FinishedColdCandidate {
    run: RevisionRun,
    memo: tex_state::PureMemoRuntime,
}

fn finish_cold_candidate(
    mut candidate: RevisionCandidate,
) -> Result<FinishedColdCandidate, SessionError> {
    let RevisionCandidateKind::Initial { source_len } = candidate.kind else {
        return Err(SessionError::CandidateKindMismatch);
    };
    let stats = candidate
        .completed
        .take()
        .ok_or(SessionError::CandidateNotComplete)?;
    let CandidateSink::Cold(sink) = candidate.sink else {
        return Err(SessionError::CandidateKindMismatch);
    };
    candidate.memo = candidate.universe.take_pure_memo_runtime();
    candidate
        .memo
        .accept_paragraph_history(candidate.universe.paragraph_origin_resolver());
    let effects = candidate.universe.world().effect_records().to_vec();
    let artifacts = candidate.universe.world().committed_artifacts().to_vec();
    let output_bytes = candidate.universe.retained_output_bytes();
    let expansion_stats = ExpansionStats::default();
    let executed_paragraphs = sink
        .records
        .iter()
        .filter(|record| record.key.boundary == EngineBoundary::OuterParagraphEnd)
        .count();
    let CanonicalCandidateCompletion {
        dvi_pages,
        dumped_format,
        format_dump_receipt,
        delivered_tokens,
        main_control_dispatches,
    } = stats;
    Ok(FinishedColdCandidate {
        run: RevisionRun {
            history: sink.records,
            effects,
            artifacts,
            dvi_pages,
            output_bytes,
            substrate: candidate.universe.freeze_generation(),
            dumped_format,
            expansion_stats,
            format_dump_receipt,
            executed_bytes: source_len,
            executed_tokens: delivered_tokens,
            executed_commands: main_control_dispatches,
            executed_macro_text_span_tokens: 0,
            executed_source_text_span_tokens: 0,
            executed_paragraphs,
        },
        memo: candidate.memo,
    })
}

#[derive(Default)]
struct HistorySink {
    records: Vec<BoundaryRecord>,
    occurrences: HashMap<(usize, EngineBoundary), u32>,
}

impl CheckpointSink for HistorySink {
    fn wants_exact_state_identity(&self, _boundary: EngineBoundary, _root_anchor: usize) -> bool {
        true
    }

    fn checkpoint(&mut self, checkpoint: EngineCheckpoint) {
        push_checkpoint(&mut self.records, &mut self.occurrences, checkpoint);
    }
}

#[allow(clippy::too_many_arguments)]
fn execute_revision(
    template: &Universe,
    pure_memo: &mut tex_state::PureMemoRuntime,
    job_name: &str,
    source: &str,
    fragments: &FragmentStore,
    layout: &EditorLayout,
    utf8_input_as_bytes: bool,
    root_source_is_byte_projection: bool,
    input_resolver: &mut dyn InputResolver,
    font_resolver: &mut dyn tex_exec::FontResolver,
    image_resolver: Option<&mut dyn tex_exec::PdfImageResolver>,
) -> Result<RevisionRun, SessionError> {
    let mut universe = template.clone();
    universe.begin_retained_session()?;
    let root = if root_source_is_byte_projection {
        MemoryInput::byte_projection(source)
    } else {
        MemoryInput::new(source)
    }
    .with_logical_path(layout.path());
    let mut input = InputStack::new(root);
    input.set_utf8_input_as_bytes(utf8_input_as_bytes);
    universe.install_editor_fragments(fragments, layout)?;
    universe.set_root_editor_content_hash(ContentHash::from_bytes(source.as_bytes()));
    let mut executor = Executor::new();
    executor.install_editor_root_layout(&mut input, layout, fragments)?;
    let mut sink = HistorySink::default();
    let mut context = match image_resolver {
        Some(image_resolver) => ExecutionContext::with_resource_resolvers(
            job_name,
            input_resolver,
            font_resolver,
            image_resolver,
        ),
        None => ExecutionContext::with_resolvers(job_name, input_resolver, font_resolver),
    };
    pure_memo.begin_paragraph_history(false);
    universe.install_pure_memo_runtime(std::mem::take(pure_memo));
    let execution_result = executor.run_with_context_and_checkpoints(
        &mut input,
        &mut universe,
        &mut context,
        &mut sink,
    );
    *pure_memo = universe.take_pure_memo_runtime();
    if execution_result.is_err() {
        pure_memo.discard_paragraph_history();
    }
    let ExecutionStats {
        dvi_pages,
        dumped_format,
        format_dump_receipt,
        delivered_tokens,
        main_control_dispatches,
        macro_text_span_tokens,
        source_text_span_tokens,
        ..
    } = execution_result?;
    let expansion_stats = input.expansion_stats().into();
    pure_memo.accept_paragraph_history(universe.paragraph_origin_resolver());
    let effects = universe.world().effect_records().to_vec();
    let artifacts = universe.world().committed_artifacts().to_vec();
    let output_bytes = universe.retained_output_bytes();
    let substrate = universe.freeze_generation();
    let executed_paragraphs = sink
        .records
        .iter()
        .filter(|record| record.key.boundary == EngineBoundary::OuterParagraphEnd)
        .count();
    Ok(RevisionRun {
        history: sink.records,
        effects,
        artifacts,
        dvi_pages,
        output_bytes,
        substrate,
        dumped_format,
        expansion_stats,
        format_dump_receipt,
        executed_bytes: source.len(),
        executed_tokens: delivered_tokens,
        executed_commands: main_control_dispatches,
        executed_macro_text_span_tokens: macro_text_span_tokens,
        executed_source_text_span_tokens: source_text_span_tokens,
        executed_paragraphs,
    })
}

struct AdvanceRun {
    scratch: Universe,
    new_records: Vec<BoundaryRecord>,
    effects: Vec<EffectRecord>,
    artifacts: Vec<CommittedArtifact>,
    pages_through_stop: Vec<DviPagePlan>,
    convergence_old_index: Option<usize>,
    reexecuted_bytes: usize,
    reexecuted_tokens: usize,
    reexecuted_commands: usize,
    reexecuted_macro_text_span_tokens: usize,
    reexecuted_source_text_span_tokens: usize,
    reexecuted_paragraphs: usize,
    same_history_attempts: usize,
    same_history_hash_mismatches: usize,
    trace_validation_latency: Duration,
    same_history_stop: SameHistoryStop,
    restart_fork_latency: Duration,
    executor_latency: Duration,
    reexecution_latency: Duration,
    dumped_format: bool,
    format_dump_receipt: Option<tex_exec::FormatDumpReceipt>,
    expansion_stats: ExpansionStats,
    output_snapshot_latency: Duration,
}

struct ResumeSink {
    records: Vec<BoundaryRecord>,
    occurrences: HashMap<(usize, EngineBoundary), u32>,
    expected: Vec<(usize, BoundaryKey, BoundaryRecord)>,
    next_expected: usize,
    convergence_old_index: Option<usize>,
    schedule_diverged: bool,
    changed_new_range: std::ops::Range<usize>,
    same_history_attempts: usize,
    same_history_hash_mismatches: usize,
    trace_validation_latency: Duration,
    allow_convergence: bool,
}

impl ResumeSink {
    fn new(old: &[BoundaryRecord], restart: usize, map: &EditMap, allow_convergence: bool) -> Self {
        let mut occurrences = HashMap::new();
        for record in &old[..=restart] {
            occurrences
                .entry((record.key.position, record.key.boundary))
                .and_modify(|next: &mut u32| *next = (*next).max(record.key.ordinal + 1))
                .or_insert(record.key.ordinal + 1);
        }
        let expected = old[restart + 1..]
            .iter()
            .enumerate()
            .filter_map(|(offset, record)| {
                map.map(record.key.position).map(|position| {
                    (
                        restart + 1 + offset,
                        BoundaryKey {
                            position,
                            ..record.key
                        },
                        record.clone(),
                    )
                })
            })
            .collect();
        Self {
            records: Vec::new(),
            occurrences,
            expected,
            next_expected: 0,
            convergence_old_index: None,
            schedule_diverged: false,
            changed_new_range: map.old.start..map.old.start + map.replacement_len,
            same_history_attempts: 0,
            same_history_hash_mismatches: 0,
            trace_validation_latency: Duration::ZERO,
            allow_convergence,
        }
    }
}

impl CheckpointSink for ResumeSink {
    fn wants_exact_state_identity(&self, _boundary: EngineBoundary, _root_anchor: usize) -> bool {
        // Every checkpoint may become accepted history if this revision does not
        // converge. Capture its canonical identity while its Universe state is
        // live so a later revision never has to reconstruct it by rollback.
        true
    }

    fn stop_requested(&self) -> bool {
        self.convergence_old_index.is_some()
    }

    fn checkpoint(&mut self, checkpoint: EngineCheckpoint) {
        push_checkpoint(&mut self.records, &mut self.occurrences, checkpoint);
        if !self.allow_convergence || self.schedule_diverged {
            return;
        }
        let Some((old_index, expected_key, expected_record)) =
            self.expected.get(self.next_expected)
        else {
            self.schedule_diverged = true;
            return;
        };
        let actual = self.records.last().expect("checkpoint was just recorded");
        if self.changed_new_range.contains(&actual.key.position) {
            return;
        }
        if actual.key != *expected_key {
            self.schedule_diverged = true;
            return;
        }
        self.next_expected += 1;
        self.same_history_attempts += 1;
        let validation_started = Timer::start();
        let exact_match = actual
            .checkpoint()
            .exact_future_state_matches(expected_record.checkpoint());
        self.trace_validation_latency = self
            .trace_validation_latency
            .saturating_add(validation_started.elapsed());
        if exact_match {
            self.convergence_old_index = Some(*old_index);
        } else {
            self.same_history_hash_mismatches += 1;
        }
    }
}

fn push_checkpoint(
    records: &mut Vec<BoundaryRecord>,
    occurrences: &mut HashMap<(usize, EngineBoundary), u32>,
    checkpoint: EngineCheckpoint,
) {
    let position = checkpoint.root_anchor();
    let boundary = checkpoint.boundary();
    let ordinal = occurrences.entry((position, boundary)).or_default();
    let key = BoundaryKey {
        position,
        boundary,
        ordinal: *ordinal,
    };
    *ordinal = ordinal.saturating_add(1);
    records.push(BoundaryRecord {
        revision: RevisionId::new(0),
        key,
        effect_prefix: checkpoint.effect_prefix_len(),
        artifact_prefix: checkpoint.artifact_prefix_len(),
        checkpoint,
    });
}

#[allow(clippy::too_many_arguments)]
#[allow(clippy::disallowed_methods)] // Session telemetry; no TeX state observes it.
fn execute_advance(
    template: &Universe,
    pure_memo: &mut tex_state::PureMemoRuntime,
    substrate: &GenerationSubstrate,
    job_name: &str,
    old_source: &str,
    source: &str,
    old_history: &[BoundaryRecord],
    old_pages: &[DviPagePlan],
    fragments: &FragmentStore,
    layout: &EditorLayout,
    restart: usize,
    map: &EditMap,
    input_resolver: &mut dyn InputResolver,
    font_resolver: &mut dyn tex_exec::FontResolver,
    image_resolver: Option<&mut dyn tex_exec::PdfImageResolver>,
    registered_inputs: &BTreeMap<PathBuf, Vec<u8>>,
) -> Result<AdvanceRun, SessionError> {
    let anchor = &old_history[restart];
    let mut scratch = template.clone();
    let mut input = InputStack::new(MemoryInput::new(String::new()));
    let mut executor = Executor::new();
    let restart_fork_latency = executor.restore_editor_checkpoint(
        &mut input,
        &mut scratch,
        substrate,
        anchor.checkpoint(),
        old_source,
        source,
        fragments,
        layout,
    )?;
    for (path, bytes) in registered_inputs {
        scratch.world_mut().set_memory_file(path, bytes.clone())?;
    }
    let mut sink = ResumeSink::new(old_history, restart, map, true);
    let mut context = match image_resolver {
        Some(image_resolver) => ExecutionContext::with_resource_resolvers(
            job_name,
            input_resolver,
            font_resolver,
            image_resolver,
        ),
        None => ExecutionContext::with_resolvers(job_name, input_resolver, font_resolver),
    };
    let reexecution_started = Timer::start();
    pure_memo.begin_paragraph_history(true);
    scratch.install_pure_memo_runtime(std::mem::take(pure_memo));
    let executor_started = Timer::start();
    let execution_result = executor.resume_with_context_and_checkpoints(
        &mut input,
        &mut scratch,
        &mut context,
        &mut sink,
    );
    let executor_latency = executor_started.elapsed();
    *pure_memo = scratch.take_pure_memo_runtime();
    if execution_result.is_err() {
        pure_memo.discard_paragraph_history();
    }
    let ExecutionStats {
        dvi_pages,
        dumped_format,
        format_dump_receipt,
        delivered_tokens,
        main_control_dispatches,
        macro_text_span_tokens,
        source_text_span_tokens,
        ..
    } = execution_result?;
    let reexecution_latency = reexecution_started.elapsed();
    let reexecuted_paragraphs = sink
        .records
        .iter()
        .filter(|record| record.key.boundary == EngineBoundary::OuterParagraphEnd)
        .count();
    let reexecuted_through = sink
        .records
        .last()
        .map_or(source.len(), |record| record.key.position);
    let reexecuted_bytes = reexecuted_through.saturating_sub(anchor.key.position);
    let same_history_stop = if !sink.allow_convergence {
        SameHistoryStop::NotAttempted
    } else if sink.convergence_old_index.is_some() {
        SameHistoryStop::Matched
    } else if sink.schedule_diverged {
        SameHistoryStop::ScheduleDiverged
    } else if sink.same_history_attempts > 0 {
        SameHistoryStop::HashesDiverged
    } else {
        SameHistoryStop::NoComparableBoundary
    };
    let expansion_stats = input.expansion_stats().into();
    let output_snapshot_started = Timer::start();
    let effects = scratch.world().effect_records().to_vec();
    let artifacts = scratch.world().committed_artifacts().to_vec();
    let mut pages_through_stop = old_pages[..anchor.artifact_prefix].to_vec();
    pages_through_stop.extend(dvi_pages);
    let output_snapshot_latency = output_snapshot_started.elapsed();
    Ok(AdvanceRun {
        scratch,
        new_records: sink.records,
        effects,
        artifacts,
        pages_through_stop,
        convergence_old_index: sink.convergence_old_index,
        reexecuted_bytes,
        reexecuted_tokens: delivered_tokens,
        reexecuted_commands: main_control_dispatches,
        reexecuted_macro_text_span_tokens: macro_text_span_tokens,
        reexecuted_source_text_span_tokens: source_text_span_tokens,
        reexecuted_paragraphs,
        same_history_attempts: sink.same_history_attempts,
        same_history_hash_mismatches: sink.same_history_hash_mismatches,
        trace_validation_latency: sink.trace_validation_latency,
        same_history_stop,
        restart_fork_latency,
        executor_latency,
        reexecution_latency,
        dumped_format,
        expansion_stats,
        format_dump_receipt,
        output_snapshot_latency,
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

fn select_restart(history: &[BoundaryRecord], old: &str, new: &str, edit: &Edit) -> Option<usize> {
    history
        .iter()
        .enumerate()
        .rev()
        .find(|(_, record)| {
            record.key.position <= edit.range.start
                && old.as_bytes().get(..record.key.position)
                    == new.as_bytes().get(..record.key.position)
        })
        .map(|(index, _)| index)
}

struct DirectInputResolver;

impl InputResolver for DirectInputResolver {
    fn open_input(
        &mut self,
        input: &mut dyn InputReadState,
        name: &str,
        _request_index: u64,
    ) -> ResourceResult<tex_state::FileContent> {
        Ok(match input.read_input_file(Path::new(name)) {
            Ok(content) => ResourceLookup::Available(content),
            Err(_) => ResourceLookup::Unavailable,
        })
    }
}

struct DirectFontResolver;

struct Timer {
    #[cfg(not(target_arch = "wasm32"))]
    started: Instant,
}

impl Timer {
    #[allow(clippy::disallowed_methods)] // Session telemetry; no TeX state observes it.
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

impl tex_exec::FontResolver for DirectFontResolver {
    fn open_font(
        &mut self,
        input: &mut dyn InputReadState,
        path: &Path,
        _request_index: u64,
    ) -> tex_exec::ResourceResult<tex_exec::FontSource> {
        Ok(match input.read_input_file(path) {
            Ok(metrics) => tex_exec::ResourceLookup::Available(tex_exec::FontSource::Tfm {
                metrics,
                opentype: None,
            }),
            Err(_) => tex_exec::ResourceLookup::Unavailable,
        })
    }
}

#[derive(Clone, Debug)]
struct EditMap {
    old: std::ops::Range<usize>,
    replacement_len: usize,
}

impl EditMap {
    fn new(old: std::ops::Range<usize>, replacement_len: usize) -> Self {
        Self {
            old,
            replacement_len,
        }
    }

    fn map(&self, position: usize) -> Option<usize> {
        if position < self.old.start {
            Some(position)
        } else if position >= self.old.end {
            position
                .checked_sub(self.old.end - self.old.start)
                .and_then(|position| position.checked_add(self.replacement_len))
        } else {
            None
        }
    }
}

fn prune_history(
    mut history: Vec<BoundaryRecord>,
    budget: usize,
    substrate_bytes: usize,
    diagnostic_bytes: usize,
    output_bytes: usize,
) -> (Vec<BoundaryRecord>, RetentionMetrics) {
    loop {
        let checkpoint_root_bytes = charged_bytes(&history, substrate_bytes);
        let charged = checkpoint_root_bytes.saturating_add(diagnostic_bytes);
        if charged <= budget || history.len() <= 2 {
            let overage = charged.saturating_sub(budget);
            return (
                history,
                RetentionMetrics {
                    checkpoint_root_bytes,
                    memo_result_bytes: 0,
                    diagnostic_bytes,
                    output_bytes,
                    protected_overage_bytes: overage,
                },
            );
        }
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
                history.iter().enumerate().find(|(index, record)| {
                    *index != 0
                        && *index != newest
                        && record.key.boundary == EngineBoundary::ShipoutComplete
                })
            })
            .map(|(index, _)| index);
        let Some(victim) = victim else {
            let checkpoint_root_bytes = charged_bytes(&history, substrate_bytes);
            let charged = checkpoint_root_bytes.saturating_add(diagnostic_bytes);
            return (
                history,
                RetentionMetrics {
                    checkpoint_root_bytes,
                    memo_result_bytes: 0,
                    diagnostic_bytes,
                    output_bytes,
                    protected_overage_bytes: charged.saturating_sub(budget),
                },
            );
        };
        history.remove(victim);
    }
}

fn oldest_retained_revision(history: &[BoundaryRecord], fallback: RevisionId) -> RevisionId {
    history
        .iter()
        .map(BoundaryRecord::revision)
        .min()
        .unwrap_or(fallback)
}

fn charged_bytes(history: &[BoundaryRecord], substrate_bytes: usize) -> usize {
    substrate_bytes.saturating_add(std::mem::size_of_val(history))
}

fn output_bytes(effects: &[EffectRecord], artifacts: &[CommittedArtifact]) -> usize {
    effects
        .iter()
        .map(EffectRecord::retained_bytes)
        .sum::<usize>()
        .saturating_add(
            artifacts
                .iter()
                .map(|artifact| {
                    artifact
                        .bytes()
                        .len()
                        .saturating_add(artifact.render_provenance_bytes())
                })
                .sum::<usize>(),
        )
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct ParagraphReplayDelta {
    lookups: u64,
    hits: u64,
    validation_misses: u64,
}

fn paragraph_replay_delta(
    before: tex_state::PureMemoStats,
    after: tex_state::PureMemoStats,
) -> ParagraphReplayDelta {
    ParagraphReplayDelta {
        lookups: after
            .paragraph_lookups
            .saturating_sub(before.paragraph_lookups),
        hits: after.paragraph_hits.saturating_sub(before.paragraph_hits),
        validation_misses: after
            .paragraph_validation_misses
            .saturating_sub(before.paragraph_validation_misses),
    }
}

fn rebase_and_validate_adopted_artifacts(
    artifacts: &mut [CommittedArtifact],
    old_effect_prefix: usize,
    new_effect_prefix: usize,
    effects: &[EffectRecord],
) -> Result<(), SessionError> {
    for artifact in artifacts {
        artifact.rebase_open_out_suffix(old_effect_prefix, new_effect_prefix)?;
        let page = tex_out::PageArtifact::from_bytes(artifact.bytes())
            .map_err(|error| SessionError::InvalidArtifactEffectSidecar(error.to_string()))?;
        for &(page_index, position) in artifact.open_out_occurrences() {
            let Some(tex_out::PageEffect::OpenOut { stream, path }) = page.effects.get(page_index)
            else {
                return Err(SessionError::InvalidArtifactEffectSidecar(
                    "OpenOut sidecar does not address an OpenOut page effect".to_owned(),
                ));
            };
            let Some(effect_index) = position
                .raw()
                .checked_sub(1)
                .and_then(|index| usize::try_from(index).ok())
            else {
                return Err(SessionError::InvalidArtifactEffectSidecar(
                    "OpenOut sidecar has an invalid absolute effect position".to_owned(),
                ));
            };
            if !matches!(
                effects.get(effect_index),
                Some(EffectRecord::StreamOpen { slot, target })
                    if slot.raw() == *stream
                        && target.path().to_string_lossy().as_ref() == path
            ) {
                return Err(SessionError::InvalidArtifactEffectSidecar(
                    "OpenOut sidecar diverges from the spliced effect history".to_owned(),
                ));
            }
        }
    }
    Ok(())
}

#[derive(Debug)]
pub enum SessionError {
    InvalidArtifactEffectSidecar(String),
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
    UnexpectedCanonicalResource,
    CanonicalResourceNoProgress {
        need: CanonicalResourceNeed,
        site: Option<tex_state::provenance::DiagnosticSite>,
    },
    SourceRegistration(SourceRegistrationError),
    CommandSummary(tex_command::CommandSummaryError),
    MissingAcceptedSubstrate,
    Execute(tex_exec::ExecError),
    World(WorldError),
    Restore(EditorRestoreError),
    Fork(GenerationForkError),
    Fragment(tex_state::source_map::SourceMapError),
    Layout(EditorLayoutError),
    RenderSource(String),
}

impl fmt::Display for SessionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidArtifactEffectSidecar(message) => {
                write!(
                    f,
                    "incremental artifact effect sidecar is invalid: {message}"
                )
            }
            Self::OutputIdentity(error) => {
                write!(f, "could not create rendered-output identity: {error}")
            }
            Self::StaleRevision { expected, actual } => write!(
                f,
                "edit targets stale revision {} (accepted revision is {})",
                actual.raw(),
                expected.raw()
            ),
            Self::ContentHashMismatch => f.write_str("edit base content hash does not match"),
            Self::NonMonotonicRevision => f.write_str("new revision id must increase"),
            Self::InvalidEditRange => f.write_str("edit range is outside UTF-8 boundaries"),
            Self::CandidateKindMismatch => {
                f.write_str("revision candidate does not belong to this completion path")
            }
            Self::CandidateNotComplete => {
                f.write_str("revision candidate is still executing or suspended")
            }
            Self::UnexpectedCanonicalResource => {
                f.write_str("canonical resource fulfillment does not match the pending need")
            }
            Self::CanonicalResourceNoProgress { need, .. } => write!(
                f,
                "canonical retained host answered {need:?}, but replay suspended on the identical need again before committing progress"
            ),
            Self::SourceRegistration(error) => {
                write!(f, "canonical source registration failed: {error}")
            }
            Self::CommandSummary(error) => write!(f, "canonical checkpoint failed: {error}"),
            Self::MissingAcceptedSubstrate => {
                f.write_str("session has no accepted cold generation")
            }
            Self::Execute(error) => write!(f, "incremental execution failed: {error}"),
            Self::World(error) => write!(f, "incremental world failed: {error}"),
            Self::Restore(error) => write!(f, "incremental restart failed: {error}"),
            Self::Fork(error) => write!(f, "incremental generation retarget failed: {error}"),
            Self::Fragment(error) => write!(f, "editor fragment allocation failed: {error}"),
            Self::Layout(error) => write!(f, "editor layout update failed: {error}"),
            Self::RenderSource(error) => write!(f, "rendered source query failed: {error}"),
        }
    }
}

impl std::error::Error for SessionError {}

impl SessionError {
    /// Returns engine-captured diagnostic provenance, when this failure came
    /// from execution rather than session orchestration.
    #[must_use]
    pub fn diagnostic_site(&self) -> Option<tex_state::provenance::DiagnosticSite> {
        match self {
            Self::Execute(error) => Some(error.diagnostic_site()),
            Self::CanonicalResourceNoProgress { site, .. } => site.clone(),
            _ => None,
        }
    }

    #[must_use]
    pub fn frozen_diagnostic_origin(&self) -> Option<&tex_exec::FrozenDiagnosticOrigin> {
        match self {
            Self::Execute(error) => error.frozen_diagnostic_origin(),
            _ => None,
        }
    }
}

impl From<tex_exec::ExecError> for SessionError {
    fn from(value: tex_exec::ExecError) -> Self {
        Self::Execute(value)
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

impl From<EditorRestoreError> for SessionError {
    fn from(value: EditorRestoreError) -> Self {
        Self::Restore(value)
    }
}

impl From<GenerationForkError> for SessionError {
    fn from(value: GenerationForkError) -> Self {
        Self::Fork(value)
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
mod tests;
