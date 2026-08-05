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
    Cancellation, CanonicalStepFailure, CanonicalStepResult, CanonicalStepRunner,
    CheckpointIdentity, CheckpointSink, EditorRestoreError, EngineBoundary, EngineCheckpoint,
    MainControl, MainControlStep, OutputLedger, ParagraphRegion, ResourceFulfillment, ResourceHost,
    ResourceNeed, ResourceOutcome, ResourceWorld, canonical_font_resource_path,
};
use tex_out::dvi::{DviError, DviPagePlan, DviStreamWriter};
pub use tex_out::html::RenderedOutputId;
use tex_state::token::OriginId;
use tex_state::{
    ArtifactOrigin, CommittedArtifact, ContentHash, EditorLayout, EditorLayoutError, EffectRecord,
    FragmentStore, GenerationForkError, GenerationSubstrate, LayoutGeneration,
    LayoutResolvedOrigin, Piece, ProvenanceResolver, ResolvedSourceLocation, Universe, WorldError,
};

mod trace;

pub use trace::{TraceCompositionError, TraceOperation, TraceSummary, TraceValidationError};

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

fn candidate_control(
    universe: &mut Universe,
    options: CandidateControlOptions<'_>,
) -> Result<MainControl, SessionError> {
    if options.initex {
        // `prepared_initex` deliberately owns only command-local state. The
        // composed Session is therefore the authoritative owner of the shared
        // primitive meanings, just like `MainControl::tex82_initex`.
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
            tex_command::install_pdftex_expandable_primitives(universe);
        }
    }
    let mut control = if options.initex {
        MainControl::prepared_initex(options.profile)
    } else {
        MainControl::with_profile(options.profile)
    };
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
/// The command path currently reports the default value until its
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
    effect_sequences: Vec<tex_state::EffectSequence>,
    effect_publications: Vec<Option<tex_state::EffectPublicationId>>,
    effect_publication_record_ordinals: Vec<Option<tex_state::EffectPublicationRecordOrdinal>>,
    effect_episode_owners: Vec<Option<tex_state::PageOutputEpisodeId>>,
    effect_domains: Vec<tex_state::EffectDomain>,
    effect_semantic_record_ordinals: Vec<tex_state::EffectSemanticRecordOrdinal>,
    effect_placement_intra_orders: Vec<tex_state::EffectPlacementIntraOrder>,
    artifact_publications: Vec<tex_state::ArtifactPublicationRecord>,
    artifacts: Vec<CommittedArtifact>,
    dvi_pages: Vec<DviPagePlan>,
    history: Vec<BoundaryRecord>,
    accepted_paragraphs: Vec<ParagraphRegion>,
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
    control: MainControl,
    sink: CandidateSink,
    memo: tex_state::PureMemoRuntime,
    completed: Option<CandidateCompletion>,
    output_ledger: OutputLedger,
    delivered_commands: usize,
    effect_start: usize,
    execution_budgets: tex_exec::ExecutionBudgets,
    kind: RevisionCandidateKind,
    carried_paragraphs: Vec<ParagraphRegion>,
    replay_suffix: Vec<ParagraphRegion>,
}

struct CandidateCompletion {
    prepared_dvi_pages: Vec<tex_exec::PreparedDviPage>,
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
    AwaitingResources(ResourceNeed),
    Complete,
}

struct AdvanceSetup {
    execution_path: RevisionExecutionPath,
    replacement_restart_boundary: Option<BoundaryKey>,
    allow_paragraph_replay: bool,
    next_revision: RevisionId,
    old_source: String,
    old_history: Vec<BoundaryRecord>,
    old_effects: Vec<EffectRecord>,
    old_effect_sequences: Vec<tex_state::EffectSequence>,
    old_effect_publications: Vec<Option<tex_state::EffectPublicationId>>,
    old_effect_publication_record_ordinals: Vec<Option<tex_state::EffectPublicationRecordOrdinal>>,
    old_effect_episode_owners: Vec<Option<tex_state::PageOutputEpisodeId>>,
    old_effect_domains: Vec<tex_state::EffectDomain>,
    old_effect_semantic_record_ordinals: Vec<tex_state::EffectSemanticRecordOrdinal>,
    old_effect_placement_intra_orders: Vec<tex_state::EffectPlacementIntraOrder>,
    old_artifact_publications: Vec<tex_state::ArtifactPublicationRecord>,
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
    Replaced {
        substrate: GenerationSubstrate,
        current_artifact_origins: Vec<OriginId>,
    },
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
    fn take_accepted_paragraphs(
        &mut self,
        convergence_position: Option<usize>,
    ) -> Vec<ParagraphRegion> {
        let resolver = self.universe.paragraph_origin_resolver();
        let mut accepted = std::mem::take(&mut self.carried_paragraphs);
        accepted.extend(self.control.take_finished_paragraph_regions());
        for region in &self.replay_suffix {
            let may_carry = convergence_position.is_none_or(|position| {
                region
                    .input()
                    .coverage()
                    .root_start()
                    .is_some_and(|start| start >= position)
            });
            if may_carry
                && !accepted
                    .iter()
                    .any(|accepted| accepted.identity() == region.identity())
            {
                region.publish_carried_history(&mut self.universe);
                accepted.push(region.clone());
            }
        }
        for region in &mut accepted {
            region.accept_line_provenance(Arc::clone(&resolver));
        }
        accepted
    }

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
        host: &mut dyn ResourceHost,
        cancellation: &Cancellation,
    ) -> Result<RevisionCandidateResult, SessionError> {
        if self.completed.is_some() {
            return Ok(RevisionCandidateResult::Complete);
        }
        {
            let sink: &mut dyn CheckpointSink = match &mut self.sink {
                CandidateSink::Cold(sink) => sink,
                CandidateSink::Advance(sink) => sink,
            };
            self.output_ledger
                .commit_job_start(&self.control, &mut self.universe, sink)?;
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
            let step_result = {
                let sink: &mut dyn CheckpointSink = match &mut self.sink {
                    CandidateSink::Cold(sink) => sink,
                    CandidateSink::Advance(sink) => sink,
                };
                CanonicalStepRunner::new(
                    &mut self.control,
                    &mut self.universe,
                    &mut self.output_ledger,
                )
                .step(sink, cancellation)
            };
            match step_result {
                CanonicalStepResult::Progress(step) => {
                    answered_needs.clear();
                    self.delivered_commands = self.delivered_commands.saturating_add(1);
                    self.validate_execution_budgets()?;
                    if self.finish_committed_step(step)? {
                        return Ok(RevisionCandidateResult::Complete);
                    }
                }
                CanonicalStepResult::Committed(step) => {
                    answered_needs.clear();
                    self.delivered_commands = self.delivered_commands.saturating_add(1);
                    self.validate_execution_budgets()?;
                    if self.finish_committed_step(step)? {
                        return Ok(RevisionCandidateResult::Complete);
                    }
                }
                CanonicalStepResult::Completed(step) => {
                    answered_needs.clear();
                    self.delivered_commands = self.delivered_commands.saturating_add(1);
                    self.validate_execution_budgets()?;
                    if self.finish_committed_step(step)? {
                        return Ok(RevisionCandidateResult::Complete);
                    }
                    unreachable!("a completed canonical step is terminal");
                }
                CanonicalStepResult::ResourceNeed(need) => {
                    self.validate_execution_budgets()?;
                    if answered_needs.contains(&need) {
                        return Err(SessionError::ResourceNoProgress {
                            need: Box::new(need),
                            site: self.control.pending_resource_site(),
                        });
                    }
                    let outcome = {
                        let mut world = ResourceWorld::new(&mut self.universe);
                        host.fulfill(&mut world, &need)
                    };
                    match outcome {
                        ResourceOutcome::Fulfilled(fulfillment) => {
                            self.output_ledger
                                .fulfill(&mut self.control, &need, fulfillment)
                                .map_err(|_| SessionError::UnexpectedResource)?;
                            answered_needs.push(need);
                        }
                        ResourceOutcome::Unavailable => {
                            self.output_ledger
                                .mark_unavailable(&mut self.control, &need, false);
                            answered_needs.push(need);
                        }
                        ResourceOutcome::Declined => {
                            self.output_ledger.record_suspension();
                            return Ok(RevisionCandidateResult::AwaitingResources(need));
                        }
                    }
                }
                CanonicalStepResult::Failed(error) => return Err(map_step_failure(error)),
            }
        }
    }

    fn finish_committed_step(&mut self, step: MainControlStep) -> Result<bool, SessionError> {
        if self.control.has_pending_paragraph_replay()
            && let CandidateSink::Advance(sink) = &mut self.sink
        {
            sink.defer_convergence_for_paragraph_replay();
        }
        let stop = match &self.sink {
            CandidateSink::Cold(sink) => sink.stop_requested(),
            CandidateSink::Advance(sink) => sink.stop_requested(),
        };
        // Full jobs convert root EOF into §93's fatal `End` inside
        // main control. Explicit fragment sessions retain
        // `EndOfInput` as their successful host boundary. Either
        // result is terminal here; replaying an exhausted source
        // can only duplicate diagnostics and grow state.
        if stop || matches!(step, MainControlStep::End | MainControlStep::EndOfInput) {
            self.control
                .finalize_page_output_receipts(&mut self.universe);
            let prepared_dvi_pages = self.control.take_prepared_dvi_pages();
            self.completed = Some(CandidateCompletion {
                prepared_dvi_pages,
                dumped_format: self.control.dumped_format(),
                format_dump_receipt: self.control.format_dump_receipt().cloned(),
                delivered_tokens: self.delivered_commands,
                main_control_dispatches: self.delivered_commands,
            });
            return Ok(true);
        }
        Ok(false)
    }

    #[must_use]
    pub const fn suspension_serial(&self) -> u64 {
        self.output_ledger.suspension_serial()
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
            suspensions: self.suspension_serial(),
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
                .saturating_add(std::mem::size_of::<MainControl>()),
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

/// Projects an artifact prefix into the optional DVI serialization view.
///
/// DVI-disabled sessions retain committed page artifacts for HTML/PDF while
/// deliberately keeping this view empty. When enabled, DVI plans are aligned
/// one-for-one with artifacts and ordinary slice bounds enforce that invariant.
fn dvi_page_prefix(pages: &[DviPagePlan], artifact_prefix: usize) -> &[DviPagePlan] {
    if pages.is_empty() {
        pages
    } else {
        &pages[..artifact_prefix]
    }
}

fn dvi_page_suffix(pages: &[DviPagePlan], artifact_prefix: usize) -> &[DviPagePlan] {
    if pages.is_empty() {
        pages
    } else {
        &pages[artifact_prefix..]
    }
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
    effect_sequences: Vec<tex_state::EffectSequence>,
    effect_publications: Vec<Option<tex_state::EffectPublicationId>>,
    effect_publication_record_ordinals: Vec<Option<tex_state::EffectPublicationRecordOrdinal>>,
    effect_episode_owners: Vec<Option<tex_state::PageOutputEpisodeId>>,
    effect_domains: Vec<tex_state::EffectDomain>,
    effect_semantic_record_ordinals: Vec<tex_state::EffectSemanticRecordOrdinal>,
    effect_placement_intra_orders: Vec<tex_state::EffectPlacementIntraOrder>,
    artifact_publications: Vec<tex_state::ArtifactPublicationRecord>,
    artifacts: Vec<CommittedArtifact>,
    dvi_pages: Vec<DviPagePlan>,
    history: Vec<BoundaryRecord>,
    accepted_paragraphs: Vec<ParagraphRegion>,
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
            effect_sequences: Vec::new(),
            effect_publications: Vec::new(),
            effect_publication_record_ordinals: Vec::new(),
            effect_episode_owners: Vec::new(),
            effect_domains: Vec::new(),
            effect_semantic_record_ordinals: Vec::new(),
            effect_placement_intra_orders: Vec::new(),
            artifact_publications: Vec::new(),
            artifacts: Vec::new(),
            dvi_pages: Vec::new(),
            history: Vec::new(),
            accepted_paragraphs: Vec::new(),
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

    /// Creates a private cold candidate without changing accepted session
    /// state. The returned owner may be retained across resource batches.
    pub fn start_cold_candidate(&self) -> Result<RevisionCandidate, SessionError> {
        let mut universe = self.template.clone();
        universe.begin_retained_session()?;
        universe.install_editor_fragments(&self.fragments, &self.layout)?;
        universe.set_root_editor_content_hash(ContentHash::from_bytes(self.source.as_bytes()));
        let control = candidate_control(
            &mut universe,
            CandidateControlOptions {
                job_name: &self.job_name,
                source_path: &self.source_path,
                bytes: self.source_file_bytes(&self.source),
                profile: self.effective_command_profile(),
                initex: self.initex,
                emit_dvi: self.dvi_output,
                root_framing: self.root_framing,
                root_framing_name: self.root_framing_name.as_deref(),
            },
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
            output_ledger: OutputLedger::new(CheckpointIdentity::Exact),
            delivered_commands: 0,
            effect_start,
            execution_budgets: tex_exec::ExecutionBudgets::default(),
            kind: RevisionCandidateKind::Initial {
                source_len: self.source.len(),
            },
            carried_paragraphs: Vec::new(),
            replay_suffix: Vec::new(),
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
            replacement_restart_boundary: None,
            allow_paragraph_replay: true,
            next_revision: self.revision,
            old_source: self.source.clone(),
            old_history: self.history.clone(),
            old_effects: self.effects.clone(),
            old_effect_sequences: self.effect_sequences.clone(),
            old_effect_publications: self.effect_publications.clone(),
            old_effect_publication_record_ordinals: self.effect_publication_record_ordinals.clone(),
            old_effect_episode_owners: self.effect_episode_owners.clone(),
            old_effect_domains: self.effect_domains.clone(),
            old_effect_semantic_record_ordinals: self.effect_semantic_record_ordinals.clone(),
            old_effect_placement_intra_orders: self.effect_placement_intra_orders.clone(),
            old_artifact_publications: self.artifact_publications.clone(),
            old_artifacts: self.artifacts.clone(),
            old_pages: self.dvi_pages.clone(),
            next: self.source.clone(),
            fragments,
            next_layout,
            map: EditMap::new(0..0, 0, true),
            revision_setup_latency: revision_setup_started.elapsed(),
        });
        let restart = setup
            .old_history
            .iter()
            .position(|record| record.key.boundary == EngineBoundary::JobStart);
        let can_restore = restart.is_some_and(|restart| {
            self.substrate.as_ref().is_some_and(|substrate| {
                setup.old_history[restart].checkpoint().can_fork_editor(
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
        let map = EditMap::new(
            edit.range.clone(),
            edit.replacement.len(),
            old_source
                .get(edit.range.clone())
                .is_some_and(|replaced| replaced == edit.replacement),
        );
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
            replacement_restart_boundary: None,
            allow_paragraph_replay: !force_job_start,
            next_revision,
            old_source,
            old_history,
            old_effects: self.effects.clone(),
            old_effect_sequences: self.effect_sequences.clone(),
            old_effect_publications: self.effect_publications.clone(),
            old_effect_publication_record_ordinals: self.effect_publication_record_ordinals.clone(),
            old_effect_episode_owners: self.effect_episode_owners.clone(),
            old_effect_domains: self.effect_domains.clone(),
            old_effect_semantic_record_ordinals: self.effect_semantic_record_ordinals.clone(),
            old_effect_placement_intra_orders: self.effect_placement_intra_orders.clone(),
            old_artifact_publications: self.artifact_publications.clone(),
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
                    setup.old_history[restart].checkpoint().can_fork_editor(
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
                    fallback.replacement_restart_boundary = Some(fallback.old_history[restart].key);
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
        let revised_root: Arc<[u8]> = Arc::from(setup.next.as_bytes());
        let mut paragraph_start_rehome_failures = 0;
        let rehomed_paragraphs = self
            .accepted_paragraphs
            .iter()
            .filter_map(|region| {
                let rehomed = region.rehome_edited_root(
                    setup.old_source.as_bytes(),
                    Arc::clone(&revised_root),
                    setup.map.old.clone(),
                );
                if rehomed.is_none()
                    && (setup.map.old.is_empty()
                        || region
                            .input()
                            .coverage()
                            .root_start()
                            .is_some_and(|start| start >= setup.map.old.end))
                {
                    paragraph_start_rehome_failures += 1;
                }
                rehomed
            })
            .collect::<Vec<_>>();
        let (carried_paragraphs, replay_suffix): (Vec<_>, Vec<_>) =
            rehomed_paragraphs.into_iter().partition(|region| {
                region
                    .input()
                    .coverage()
                    .root_end()
                    .is_some_and(|end| end <= anchor.key.position)
            });
        let allow_convergence = allow_convergence
            && !replay_suffix
                .iter()
                .any(tex_exec::ParagraphRegion::permits_contiguous_cold_replay);
        if carried_paragraphs.is_empty()
            && replay_suffix.is_empty()
            && paragraph_start_rehome_failures > 0
        {
            let mut candidate = self.start_replacement_candidate(setup)?;
            let mut memo = candidate.universe.take_pure_memo_runtime();
            for _ in 0..paragraph_start_rehome_failures {
                memo.record_paragraph_validation_failure(
                    tex_state::ParagraphValidationFailure::ParagraphStart,
                );
            }
            candidate.universe.install_pure_memo_runtime(memo);
            return Ok(candidate);
        }
        let mut control = if self.initex {
            MainControl::prepared_initex(self.effective_command_profile())
        } else {
            MainControl::with_profile(self.effective_command_profile())
        };
        control.set_dvi_output(self.dvi_output);
        control
            .capabilities_mut()
            .set_startup_job_name(&self.job_name);
        // Convergence keeps the accepted substrate, so the paragraph suffix
        // published with that generation must keep its substrate-owned input
        // handles. The fork returns separately materialized transactions for
        // speculative execution; those handles die with the scratch universe.
        let retained_replay_suffix = replay_suffix.clone();
        let (
            mut universe,
            restart_fork_latency,
            materialized_replay_suffix,
            mut validation_failures,
        ) = anchor.checkpoint().fork_editor_with_paragraphs(
            &mut control,
            substrate,
            tex_exec::EditorFork {
                old_source: setup.old_source.as_bytes(),
                new_source: Arc::from(self.source_file_bytes(&setup.next)),
                fragments: &setup.fragments,
                layout: &setup.next_layout,
                paragraphs: &replay_suffix,
            },
        )?;
        validation_failures.extend(std::iter::repeat_n(
            tex_state::ParagraphValidationFailure::ParagraphStart,
            paragraph_start_rehome_failures,
        ));
        if let Some(owners) = materialized_replay_suffix
            .iter()
            .map(tex_exec::ParagraphRegion::output_effect_episode_owners)
            .max_by_key(|owners| owners.len())
        {
            universe
                .world_mut()
                .install_output_effect_episode_owners(owners);
        }
        if let Some(publications) = materialized_replay_suffix
            .iter()
            .map(tex_exec::ParagraphRegion::output_effect_publications)
            .max_by_key(|publications| publications.len())
        {
            universe
                .world_mut()
                .install_effect_publications(publications);
        }
        if let Some(domains) = materialized_replay_suffix
            .iter()
            .map(tex_exec::ParagraphRegion::effect_domains)
            .max_by_key(|domains| domains.len())
        {
            universe.world_mut().install_effect_domains(domains);
        }
        if let Some(ordinals) = materialized_replay_suffix
            .iter()
            .map(tex_exec::ParagraphRegion::effect_semantic_record_ordinals)
            .max_by_key(|ordinals| ordinals.len())
        {
            universe
                .world_mut()
                .install_effect_semantic_record_ordinals(ordinals);
        }
        if let Some(orders) = materialized_replay_suffix
            .iter()
            .map(tex_exec::ParagraphRegion::effect_placement_intra_orders)
            .max_by_key(|orders| orders.len())
        {
            universe
                .world_mut()
                .install_effect_placement_intra_orders(orders);
        }
        control.install_paragraph_replay_regions(materialized_replay_suffix.iter().cloned());
        for (path, bytes) in &self.registered_inputs {
            universe.world_mut().set_memory_file(path, bytes.clone())?;
        }
        let mut memo = self.pure_memo.clone();
        memo.begin_paragraph_history(true);
        for failure in validation_failures {
            memo.record_paragraph_validation_failure(failure);
        }
        universe.install_pure_memo_runtime(std::mem::take(&mut memo));
        for region in &carried_paragraphs {
            region.publish_carried_history(&mut universe);
        }
        let sink = ResumeSink::new(&setup.old_history, restart, &setup.map, allow_convergence);
        let effect_start = universe.world().effect_records().len();
        Ok(RevisionCandidate {
            universe,
            control,
            sink: CandidateSink::Advance(sink),
            memo,
            completed: None,
            output_ledger: OutputLedger::resume(CheckpointIdentity::Exact),
            delivered_commands: 0,
            effect_start,
            execution_budgets: tex_exec::ExecutionBudgets::default(),
            kind: RevisionCandidateKind::Incremental {
                setup,
                restart,
                restart_fork_latency,
            },
            carried_paragraphs,
            replay_suffix: retained_replay_suffix,
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
        let mut control = candidate_control(
            &mut universe,
            CandidateControlOptions {
                job_name: &self.job_name,
                source_path: setup.next_layout.path(),
                bytes: self.source_file_bytes(&setup.next),
                profile: self.effective_command_profile(),
                initex: self.initex,
                emit_dvi: self.dvi_output,
                root_framing: self.root_framing,
                root_framing_name: self.root_framing_name.as_deref(),
            },
        )?;
        let revised_root: Arc<[u8]> = Arc::from(setup.next.as_bytes());
        let mut replay_suffix = Vec::new();
        for region in self
            .accepted_paragraphs
            .iter()
            .take_while(|_| setup.allow_paragraph_replay)
        {
            let coverage = region.input().coverage();
            let intersects_edit = coverage.root_start().is_some_and(|start| {
                coverage
                    .root_end()
                    .is_some_and(|end| start < setup.map.old.end && end > setup.map.old.start)
            });
            if intersects_edit {
                continue;
            }
            if !region.permits_contiguous_cold_replay() {
                break;
            }
            let Some(region) = region.rehome_edited_root(
                setup.old_source.as_bytes(),
                Arc::clone(&revised_root),
                setup.map.old.clone(),
            ) else {
                break;
            };
            replay_suffix.push(region);
        }
        let replay_suffix = match (setup.old_history.first(), self.substrate.as_ref()) {
            (Some(anchor), Some(substrate)) => anchor
                .checkpoint()
                .materialize_accepted_paragraphs_into(&mut universe, substrate, &replay_suffix)?,
            _ => Vec::new(),
        };
        control.install_contiguous_cold_paragraph_replay_regions(replay_suffix.iter().cloned());
        memo.begin_paragraph_history(false);
        universe.install_pure_memo_runtime(std::mem::take(&mut memo));
        let effect_start = universe.world().effect_records().len();
        Ok(RevisionCandidate {
            universe,
            control,
            sink: CandidateSink::Cold(HistorySink::default()),
            memo,
            completed: None,
            output_ledger: OutputLedger::new(CheckpointIdentity::Exact),
            delivered_commands: 0,
            effect_start,
            execution_budgets: tex_exec::ExecutionBudgets::default(),
            kind: RevisionCandidateKind::Replacement { setup },
            carried_paragraphs: Vec::new(),
            replay_suffix,
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
        let accepted_paragraphs = candidate.take_accepted_paragraphs(None);
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
        let effect_sequences = candidate
            .universe
            .world()
            .effect_sequences()
            .as_ref()
            .clone();
        let effect_publications = candidate
            .universe
            .world()
            .effect_publications()
            .as_ref()
            .clone();
        let effect_domains = candidate.universe.world().effect_domains().as_ref().clone();
        let effect_publication_record_ordinals = candidate
            .universe
            .world()
            .effect_publication_record_ordinals()
            .as_ref()
            .clone();
        let effect_episode_owners = candidate
            .universe
            .world()
            .output_effect_episode_owners()
            .as_ref()
            .clone();
        let effect_semantic_record_ordinals = candidate
            .universe
            .world()
            .effect_semantic_record_ordinals()
            .as_ref()
            .clone();
        let effect_placement_intra_orders = candidate
            .universe
            .world()
            .effect_placement_intra_orders()
            .as_ref()
            .clone();
        let artifacts = candidate.universe.world().committed_artifacts().to_vec();
        let artifact_publications = candidate.universe.world().artifact_publications().to_vec();
        let expansion_stats = ExpansionStats::default();
        let CandidateCompletion {
            prepared_dvi_pages,
            dumped_format,
            format_dump_receipt,
            delivered_tokens,
            main_control_dispatches,
        } = stats;
        let dvi_pages: Vec<DviPagePlan> = prepared_dvi_pages
            .into_iter()
            .map(tex_exec::PreparedDviPage::into_plan)
            .collect();
        let substrate = candidate.universe.freeze_generation();
        let history = retain_restorable_history(sink.records, &substrate)?;
        let reuse = ReuseMetrics {
            execution_path: setup.execution_path,
            restart_boundary: setup.replacement_restart_boundary,
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
            effect_sequences,
            effect_publications,
            effect_publication_record_ordinals,
            effect_episode_owners,
            effect_domains,
            effect_semantic_record_ordinals,
            effect_placement_intra_orders,
            artifact_publications,
            artifacts,
            dvi_pages,
            history,
            accepted_paragraphs,
            substrate: PendingSubstrate::Replaced {
                substrate,
                current_artifact_origins: Vec::new(),
            },
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
        let superseded_output_episodes = candidate.control.take_superseded_page_output_episodes();
        let convergence_position = match &candidate.sink {
            CandidateSink::Advance(sink) if sink.convergence_old_index.is_some() => {
                sink.records.last().map(|record| record.key.position)
            }
            _ => None,
        };
        let accepted_paragraphs = candidate.take_accepted_paragraphs(convergence_position);
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
        let break_dependency_index =
            tex_state::ParagraphValidationFailure::BreakDependency as usize;
        let broke_retained_paragraph = memo.stats().paragraph_validation_failure_reasons
            [break_dependency_index]
            > before_memo.paragraph_validation_failure_reasons[break_dependency_index];
        let CandidateCompletion {
            prepared_dvi_pages,
            dumped_format,
            format_dump_receipt: _,
            delivered_tokens,
            main_control_dispatches,
        } = stats;
        let live_dvi_publications = prepared_dvi_pages
            .iter()
            .map(tex_exec::PreparedDviPage::publication)
            .collect::<Vec<_>>();
        let dvi_pages: Vec<DviPagePlan> = prepared_dvi_pages
            .into_iter()
            .map(tex_exec::PreparedDviPage::into_plan)
            .collect();
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
        let live_effect_sequences = candidate.universe.world().effect_sequences();
        let live_effect_publications = candidate.universe.world().effect_publications();
        let live_effect_publication_record_ordinals = candidate
            .universe
            .world()
            .effect_publication_record_ordinals();
        let live_effect_episode_owners = candidate.universe.world().output_effect_episode_owners();
        let live_effect_domains = candidate.universe.world().effect_domains();
        let live_effect_semantic_record_ordinals =
            candidate.universe.world().effect_semantic_record_ordinals();
        let live_effect_placement_intra_orders =
            candidate.universe.world().effect_placement_intra_orders();
        let live_effect_publication_dispositions =
            candidate.universe.world().effect_publication_dispositions();
        let artifacts = candidate.universe.world().committed_artifacts().to_vec();
        let mut live_artifact_publications =
            candidate.universe.world().artifact_publications().to_vec();
        let effect_episode_owners = candidate.universe.world().output_effect_episode_owners();
        let artifact_base = candidate
            .universe
            .world()
            .artifact_pos()
            .saturating_sub(artifacts.len());
        let retained_artifact_prefix = setup.old_history[restart]
            .artifact_prefix
            .max(artifact_base);
        let mut pages_through_stop =
            dvi_page_prefix(&setup.old_pages, retained_artifact_prefix).to_vec();
        pages_through_stop.extend(dvi_pages.iter().cloned());

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
        let mut assembled_effect_sidecars = None;
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
                extend_live_effects_excluding_superseded_output(
                    &mut joined_effects,
                    &effects,
                    &effect_episode_owners,
                    0..scratch_effect_count,
                    &superseded_output_episodes,
                );
                // A terminal retained checkpoint can include final-cleanup
                // effects that the stopped scratch run has not executed. The
                // scratch prefix is an absolute effect position even when its
                // local record vector starts at a restored nonzero base; own
                // the retained tail from the earlier absolute prefix so the
                // cleanup records are neither dropped nor replayed twice.
                let adopted_effect_prefix = old_effect_prefix.min(new_effect_prefix);
                extend_adopted_effects_excluding_superseded_output(
                    &mut joined_effects,
                    &setup.old_effects,
                    adopted_effect_prefix,
                    &superseded_output_episodes,
                );

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
                joined_pages.extend_from_slice(dvi_page_suffix(&setup.old_pages, old_prefix));
                let mut history = Vec::with_capacity(
                    restart + 1 + setup.old_history.len().saturating_sub(old_index),
                );
                for mut record in setup.old_history[..=restart].iter().cloned() {
                    record.checkpoint = record
                        .checkpoint
                        .rehome_unchanged_prefix(substrate, &roots)?;
                    record.revision = setup.next_revision;
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
                let replayed_artifact_count = usize::try_from(paragraph_replay.hits).unwrap_or(0);
                let (
                    candidate_effects,
                    _candidate_sequences,
                    candidate_publications,
                    _candidate_publication_record_ordinals,
                    _candidate_episode_owners,
                    _candidate_domains,
                    _candidate_semantic_record_ordinals,
                    _candidate_placement_intra_orders,
                ) = assemble_effect_ledger(
                    &setup.old_effects,
                    &setup.old_effect_sequences,
                    &setup.old_effect_publications,
                    &setup.old_effect_publication_record_ordinals,
                    &setup.old_effect_episode_owners,
                    &setup.old_effect_domains,
                    &setup.old_effect_semantic_record_ordinals,
                    &setup.old_effect_placement_intra_orders,
                    &effects,
                    &live_effect_sequences,
                    &live_effect_publications,
                    &live_effect_publication_record_ordinals,
                    &live_effect_episode_owners,
                    &live_effect_domains,
                    &live_effect_semantic_record_ordinals,
                    &live_effect_placement_intra_orders,
                    anchor.effect_prefix,
                    &superseded_output_episodes,
                    &live_effect_publication_dispositions,
                    None,
                );
                let _ = replayed_artifact_count;
                let current_artifact_origins = if broke_retained_paragraph {
                    artifacts
                        .iter()
                        .flat_map(|artifact| artifact.live_render_origins().iter())
                        .copied()
                        .filter(|origin| {
                            candidate
                                .universe
                                .root_span_for_origin(*origin)
                                .is_some_and(|span| {
                                    matches!(
                                        candidate.universe.resolve_root_span(
                                            span,
                                            &setup.fragments,
                                            &setup.next_layout,
                                        ),
                                        LayoutResolvedOrigin::Current { .. }
                                    )
                                })
                        })
                        .collect()
                } else {
                    Vec::new()
                };
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
                let (joined_artifacts, joined_artifact_publications, joined_pages) =
                    assemble_artifact_ledger(
                        &setup.old_artifacts,
                        &setup.old_artifact_publications,
                        &setup.old_pages,
                        &artifacts,
                        &live_artifact_publications,
                        &dvi_pages,
                        &live_dvi_publications,
                        retained_artifact_prefix,
                        &superseded_output_episodes,
                        &candidate_publications,
                        &live_effect_publications,
                        &live_effect_sequences,
                    );
                let selected_artifact_effect_publications = setup
                    .old_artifact_publications
                    .iter()
                    .zip(&joined_artifact_publications)
                    .filter_map(|(accepted, selected)| {
                        Some((
                            accepted.effect_publication()?,
                            selected.effect_publication()?,
                        ))
                    })
                    .collect::<std::collections::BTreeMap<_, _>>();
                let selected_artifact_effect_publications = (setup.old_artifact_publications.len()
                    == joined_artifact_publications.len()
                    && selected_artifact_effect_publications.len()
                        == joined_artifact_publications.len())
                .then_some(&selected_artifact_effect_publications);
                let (
                    joined_effects,
                    candidate_sequences,
                    candidate_publications,
                    candidate_publication_record_ordinals,
                    candidate_episode_owners,
                    candidate_domains,
                    candidate_semantic_record_ordinals,
                    candidate_placement_intra_orders,
                ) = assemble_effect_ledger(
                    &setup.old_effects,
                    &setup.old_effect_sequences,
                    &setup.old_effect_publications,
                    &setup.old_effect_publication_record_ordinals,
                    &setup.old_effect_episode_owners,
                    &setup.old_effect_domains,
                    &setup.old_effect_semantic_record_ordinals,
                    &setup.old_effect_placement_intra_orders,
                    &effects,
                    &live_effect_sequences,
                    &live_effect_publications,
                    &live_effect_publication_record_ordinals,
                    &live_effect_episode_owners,
                    &live_effect_domains,
                    &live_effect_semantic_record_ordinals,
                    &live_effect_placement_intra_orders,
                    anchor.effect_prefix,
                    &superseded_output_episodes,
                    &live_effect_publication_dispositions,
                    selected_artifact_effect_publications,
                );
                assembled_effect_sidecars = Some((
                    candidate_sequences,
                    candidate_publications.clone(),
                    candidate_publication_record_ordinals,
                    candidate_episode_owners,
                    candidate_domains,
                    candidate_semantic_record_ordinals,
                    candidate_placement_intra_orders,
                ));
                let _ = candidate_effects;
                live_artifact_publications = joined_artifact_publications;
                (
                    joined_effects,
                    joined_artifacts,
                    joined_pages,
                    history,
                    PendingSubstrate::Replaced {
                        substrate: target,
                        current_artifact_origins,
                    },
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
            PendingSubstrate::Replaced { substrate, .. } => substrate,
        };
        let history = retain_restorable_history(history, retained_substrate)?;
        reuse.trace_retained_bytes = std::mem::size_of_val(history.as_slice());
        reuse.splice_latency = splice_started.elapsed();
        reuse.trace_replay_latency = reuse.splice_latency;
        let (
            effect_sequences,
            effect_publications,
            effect_publication_record_ordinals,
            effect_episode_owners,
            effect_domains,
            effect_semantic_record_ordinals,
            effect_placement_intra_orders,
        ) = assembled_effect_sidecars.unwrap_or_else(|| {
            (
                (1..=effects.len())
                    .map(|sequence| tex_state::EffectSequence::new(sequence as u64))
                    .collect(),
                vec![None; effects.len()],
                vec![None; effects.len()],
                vec![None; effects.len()],
                (1..=effects.len())
                    .map(|domain| tex_state::EffectDomain::World(domain as u64))
                    .collect(),
                vec![tex_state::EffectSemanticRecordOrdinal::new(1); effects.len()],
                (1..=effects.len())
                    .map(|order| tex_state::EffectPlacementIntraOrder::new(order as u64))
                    .collect(),
            )
        });
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
            effect_sequences,
            effect_publications,
            effect_publication_record_ordinals,
            effect_episode_owners,
            effect_domains,
            effect_semantic_record_ordinals,
            effect_placement_intra_orders,
            artifact_publications: live_artifact_publications,
            artifacts,
            dvi_pages: pages,
            history,
            accepted_paragraphs,
            substrate: pending_substrate,
            reuse,
            dumped_format,
            expansion_stats,
            format_dump_receipt: None,
            candidate_memo: Some(memo),
        })
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
        Ok(substrate.materialize_detached_outputs(
            self.effects.clone(),
            self.artifacts.clone(),
            self.artifact_publications.clone(),
        )?)
    }

    /// Consumes the accepted session into the reached engine state with its
    /// detached effects still uncommitted. This is the client finalization
    /// boundary for one-shot drivers.
    pub fn into_accepted_universe(mut self) -> Result<AcceptedUniverseFinalization, SessionError> {
        let substrate = self
            .substrate
            .take()
            .ok_or(SessionError::MissingAcceptedSubstrate)?;
        let mut universe = substrate.into_detached_universe(
            self.effects,
            self.artifacts,
            self.artifact_publications,
        )?;
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
        self.layout.prepare_line_index(&self.fragments);
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
        Ok(substrate.export_detached_outputs(
            self.effects,
            self.artifacts,
            self.artifact_publications,
        )?)
    }

    #[allow(clippy::disallowed_methods)] // Session telemetry; no TeX state observes it.
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
        let pending = self.prepare_advance_with_resolvers(next_revision, edit, host)?;
        self.accept_pending(pending)
    }

    pub fn advance_with_resource_resolvers(
        &mut self,
        next_revision: RevisionId,
        edit: Edit,
        host: &mut dyn ResourceHost,
    ) -> Result<AcceptedOutput, SessionError> {
        let pending = self.prepare_advance_with_resource_resolvers(next_revision, edit, host)?;
        self.accept_pending(pending)
    }

    /// Executes an edit into private candidate state without changing the
    /// accepted revision. The caller may validate all downstream output and
    /// either atomically accept the candidate or drop it.
    pub fn prepare_advance_with_resolvers(
        &mut self,
        next_revision: RevisionId,
        edit: Edit,
        host: &mut dyn ResourceHost,
    ) -> Result<PendingRevision, SessionError> {
        // Query caches are revision-attempt ephemera. A failed candidate keeps
        // accepted semantic state but must not keep maps lowered before it.
        self.clear_render_maps();
        let mut candidate = self.start_advance_candidate(next_revision, edit)?;
        drive_synchronous_candidate(&mut candidate, host)?;
        self.finish_advance_candidate(candidate)
    }

    pub fn prepare_advance_with_resource_resolvers(
        &mut self,
        next_revision: RevisionId,
        edit: Edit,
        host: &mut dyn ResourceHost,
    ) -> Result<PendingRevision, SessionError> {
        self.prepare_advance_with_resolvers(next_revision, edit, host)
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
            PendingSubstrate::Replaced { substrate, .. } => substrate,
        };
        let mut world = substrate.materialize_detached_outputs(
            pending.effects.clone(),
            pending.artifacts.clone(),
            pending.artifact_publications.clone(),
        )?;
        world.install_effect_sequences(&pending.effect_sequences);
        world.install_effect_publications(&pending.effect_publications);
        world.install_effect_publication_record_ordinals(
            &pending.effect_publication_record_ordinals,
        );
        world.install_output_effect_episode_owners(&pending.effect_episode_owners);
        world.install_effect_domains(&pending.effect_domains);
        world.install_effect_semantic_record_ordinals(&pending.effect_semantic_record_ordinals);
        world.install_effect_placement_intra_orders(&pending.effect_placement_intra_orders);
        Ok(world)
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
            effect_sequences,
            effect_publications,
            effect_publication_record_ordinals,
            effect_episode_owners,
            effect_domains,
            effect_semantic_record_ordinals,
            effect_placement_intra_orders,
            artifact_publications,
            artifacts,
            dvi_pages,
            history,
            accepted_paragraphs,
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
            } => {
                let retained_origins = artifacts
                    .iter()
                    .flat_map(|artifact| artifact.live_render_origins().iter())
                    .copied()
                    .collect::<Vec<_>>();
                let substrate = self
                    .substrate
                    .as_mut()
                    .ok_or(SessionError::MissingAcceptedSubstrate)?;
                substrate.retain_artifact_origin_spans(
                    &retained_origins,
                    &self.fragments,
                    &self.layout,
                );
                substrate.retain_artifact_origins_from_fork_with_layout(
                    &scratch,
                    &adopted_origins,
                    &fragments,
                    &layout,
                )?;
            }
            PendingSubstrate::Replaced {
                mut substrate,
                current_artifact_origins,
            } => {
                substrate.retain_artifact_origin_spans(
                    &current_artifact_origins,
                    &fragments,
                    &layout,
                );
                self.substrate = Some(substrate);
            }
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
        self.effect_sequences = effect_sequences;
        self.effect_publications = effect_publications;
        self.effect_publication_record_ordinals = effect_publication_record_ordinals;
        self.effect_episode_owners = effect_episode_owners;
        self.effect_domains = effect_domains;
        self.effect_semantic_record_ordinals = effect_semantic_record_ordinals;
        self.effect_placement_intra_orders = effect_placement_intra_orders;
        self.artifact_publications = artifact_publications;
        self.artifacts = artifacts;
        self.dvi_pages = dvi_pages;
        self.history = history;
        self.accepted_paragraphs = accepted_paragraphs;
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
        self.accepted_paragraphs = run.accepted_paragraphs;
        self.effects = run.effects;
        self.effect_sequences = run.effect_sequences;
        self.effect_publications = run.effect_publications;
        self.effect_publication_record_ordinals = run.effect_publication_record_ordinals;
        self.effect_episode_owners = run.effect_episode_owners;
        self.effect_domains = run.effect_domains;
        self.effect_semantic_record_ordinals = run.effect_semantic_record_ordinals;
        self.effect_placement_intra_orders = run.effect_placement_intra_orders;
        self.artifact_publications = run.artifact_publications;
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
            effects: materialize_effect_view(&self.effects, &self.effect_domains),
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
    accepted_paragraphs: Vec<ParagraphRegion>,
    effects: Vec<EffectRecord>,
    effect_sequences: Vec<tex_state::EffectSequence>,
    effect_publications: Vec<Option<tex_state::EffectPublicationId>>,
    effect_publication_record_ordinals: Vec<Option<tex_state::EffectPublicationRecordOrdinal>>,
    effect_episode_owners: Vec<Option<tex_state::PageOutputEpisodeId>>,
    effect_domains: Vec<tex_state::EffectDomain>,
    effect_semantic_record_ordinals: Vec<tex_state::EffectSemanticRecordOrdinal>,
    effect_placement_intra_orders: Vec<tex_state::EffectPlacementIntraOrder>,
    artifact_publications: Vec<tex_state::ArtifactPublicationRecord>,
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
    let accepted_paragraphs = candidate.take_accepted_paragraphs(None);
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
    let effect_sequences = candidate
        .universe
        .world()
        .effect_sequences()
        .as_ref()
        .clone();
    let effect_publications = candidate
        .universe
        .world()
        .effect_publications()
        .as_ref()
        .clone();
    let effect_domains = candidate.universe.world().effect_domains().as_ref().clone();
    let effect_publication_record_ordinals = candidate
        .universe
        .world()
        .effect_publication_record_ordinals()
        .as_ref()
        .clone();
    let effect_episode_owners = candidate
        .universe
        .world()
        .output_effect_episode_owners()
        .as_ref()
        .clone();
    let effect_semantic_record_ordinals = candidate
        .universe
        .world()
        .effect_semantic_record_ordinals()
        .as_ref()
        .clone();
    let effect_placement_intra_orders = candidate
        .universe
        .world()
        .effect_placement_intra_orders()
        .as_ref()
        .clone();
    let artifacts = candidate.universe.world().committed_artifacts().to_vec();
    let artifact_publications = candidate.universe.world().artifact_publications().to_vec();
    let output_bytes = candidate.universe.retained_output_bytes();
    let expansion_stats = ExpansionStats::default();
    let executed_paragraphs = sink
        .records
        .iter()
        .filter(|record| record.key.boundary == EngineBoundary::OuterParagraphEnd)
        .count();
    let CandidateCompletion {
        prepared_dvi_pages,
        dumped_format,
        format_dump_receipt,
        delivered_tokens,
        main_control_dispatches,
    } = stats;
    let dvi_pages = prepared_dvi_pages
        .into_iter()
        .map(tex_exec::PreparedDviPage::into_plan)
        .collect();
    Ok(FinishedColdCandidate {
        run: RevisionRun {
            history: sink.records,
            accepted_paragraphs,
            effects,
            effect_sequences,
            effect_publications,
            effect_publication_record_ordinals,
            effect_episode_owners,
            effect_domains,
            effect_semantic_record_ordinals,
            effect_placement_intra_orders,
            artifact_publications,
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
    compared_actual_effect_prefix: usize,
    compared_actual_artifact_prefix: usize,
    compared_old_effect_prefix: usize,
    compared_old_artifact_prefix: usize,
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
            changed_new_range: map.changed_new_range(),
            same_history_attempts: 0,
            same_history_hash_mismatches: 0,
            trace_validation_latency: Duration::ZERO,
            allow_convergence,
            compared_actual_effect_prefix: old[restart].effect_prefix,
            compared_actual_artifact_prefix: old[restart].artifact_prefix,
            compared_old_effect_prefix: old[restart].effect_prefix,
            compared_old_artifact_prefix: old[restart].artifact_prefix,
        }
    }

    fn defer_convergence_for_paragraph_replay(&mut self) {
        self.convergence_old_index = None;
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
        let output_matches = actual.checkpoint().output_segment_matches(
            self.compared_actual_effect_prefix,
            self.compared_actual_artifact_prefix,
            expected_record.checkpoint(),
            self.compared_old_effect_prefix,
            self.compared_old_artifact_prefix,
        );
        self.compared_actual_effect_prefix = actual.effect_prefix;
        self.compared_actual_artifact_prefix = actual.artifact_prefix;
        self.compared_old_effect_prefix = expected_record.effect_prefix;
        self.compared_old_artifact_prefix = expected_record.artifact_prefix;
        let exact_match = output_matches
            && actual
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
            // A shipout checkpoint at a zero-width insertion boundary owns a
            // loaded TeX input line whose old suffix begins at that cursor.
            // Restoring it would retain the old line front and skip bytes
            // inserted before that suffix. Select the preceding checkpoint so
            // TeX's §328 input stack delivers every newly inserted byte.
            (record.key.position < edit.range.start
                || record.key.position == edit.range.start
                    && (!edit.range.is_empty()
                        || record.key.boundary != EngineBoundary::ShipoutComplete))
                && old.as_bytes().get(..record.key.position)
                    == new.as_bytes().get(..record.key.position)
        })
        .map(|(index, _)| index)
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

fn drive_synchronous_candidate(
    candidate: &mut RevisionCandidate,
    host: &mut dyn ResourceHost,
) -> Result<(), SessionError> {
    match candidate.drive_with_resource_resolvers(host, &Cancellation::new())? {
        RevisionCandidateResult::Complete => Ok(()),
        RevisionCandidateResult::AwaitingResources(need) => Err(SessionError::ResourceNoProgress {
            need: Box::new(need),
            site: candidate.control.pending_resource_site(),
        }),
    }
}

fn map_step_failure(error: CanonicalStepFailure) -> SessionError {
    match error {
        CanonicalStepFailure::Execution(error) => SessionError::Execute(error),
        CanonicalStepFailure::Checkpoint(error) => SessionError::CommandSummary(error),
    }
}

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

#[derive(Clone, Debug)]
struct EditMap {
    old: std::ops::Range<usize>,
    replacement_len: usize,
    preserves_replaced_bytes: bool,
}

impl EditMap {
    fn new(
        old: std::ops::Range<usize>,
        replacement_len: usize,
        preserves_replaced_bytes: bool,
    ) -> Self {
        Self {
            old,
            replacement_len,
            preserves_replaced_bytes,
        }
    }

    fn map(&self, position: usize) -> Option<usize> {
        if position < self.old.start {
            Some(position)
        } else if position >= self.old.end {
            position
                .checked_sub(self.old.end - self.old.start)
                .and_then(|position| position.checked_add(self.replacement_len))
        } else if self.preserves_replaced_bytes {
            Some(position)
        } else {
            None
        }
    }

    fn changed_new_range(&self) -> std::ops::Range<usize> {
        if self.preserves_replaced_bytes {
            self.old.start..self.old.start
        } else {
            self.old.start..self.old.start + self.replacement_len
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

fn extend_adopted_effects_excluding_superseded_output(
    destination: &mut Vec<EffectRecord>,
    accepted: &[EffectRecord],
    adopted_start: usize,
    superseded: &[tex_exec::SupersededPageOutputEpisode],
) {
    destination.extend(
        accepted
            .iter()
            .enumerate()
            .skip(adopted_start)
            .filter(|(index, _)| {
                !superseded.iter().any(|episode| {
                    episode.effects().contains(index)
                        && seal_revision_output_publication(episode).is_some_and(|sealed| {
                            let _publication_identity = sealed.identity;
                            sealed.outcome
                                == tex_state::OutputEpisodePublicationOutcome::Regenerated
                        })
                })
            })
            .map(|(_, effect)| effect.clone()),
    );
}

#[derive(Clone)]
struct EffectAssemblyRecord {
    arbitration_key: EffectSemanticRecordArbitrationKey,
    sequence: tex_state::EffectSequence,
    publication: Option<tex_state::EffectPublicationId>,
    publication_record_ordinal: Option<tex_state::EffectPublicationRecordOrdinal>,
    episode_owner: Option<tex_state::PageOutputEpisodeId>,
    domain: tex_state::EffectDomain,
    semantic_record_ordinal: tex_state::EffectSemanticRecordOrdinal,
    placement_intra_order: tex_state::EffectPlacementIntraOrder,
    output_attempt: Option<tex_state::EffectOutputAttemptId>,
    effects: Vec<EffectRecord>,
    origin: EffectRecordOrigin,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum EffectRecordOrigin {
    Accepted,
    Live,
}

fn materialize_effect_view(
    effects: &[EffectRecord],
    domains: &[tex_state::EffectDomain],
) -> Vec<EffectRecord> {
    let (ordinary, mut terminal): (Vec<_>, Vec<_>) =
        effects.iter().zip(domains).partition(|(_, domain)| {
            !matches!(domain, tex_state::EffectDomain::TerminalPublication { .. })
        });
    terminal.sort_by_key(|(_, domain)| match domain {
        tex_state::EffectDomain::TerminalPublication {
            phase, intra_order, ..
        } => (*phase, *intra_order),
        _ => unreachable!(),
    });
    ordinary
        .into_iter()
        .chain(terminal)
        .map(|(effect, _)| effect.clone())
        .collect()
}

type AssembledEffectLedger = (
    Vec<EffectRecord>,
    Vec<tex_state::EffectSequence>,
    Vec<Option<tex_state::EffectPublicationId>>,
    Vec<Option<tex_state::EffectPublicationRecordOrdinal>>,
    Vec<Option<tex_state::PageOutputEpisodeId>>,
    Vec<tex_state::EffectDomain>,
    Vec<tex_state::EffectSemanticRecordOrdinal>,
    Vec<tex_state::EffectPlacementIntraOrder>,
);

fn map_publication_boundary_domain(
    domain: tex_state::EffectDomain,
    endpoint_replacements: &std::collections::BTreeMap<
        tex_state::EffectPublicationId,
        tex_state::EffectPublicationId,
    >,
    output_attempt_replacements: &std::collections::BTreeMap<
        tex_state::EffectOutputAttemptId,
        tex_state::EffectOutputAttemptId,
    >,
) -> tex_state::EffectDomain {
    let tex_state::EffectDomain::PublicationBoundary {
        left,
        right,
        output_attempt,
    } = domain
    else {
        return domain;
    };
    tex_state::EffectDomain::PublicationBoundary {
        left: left.map(|publication| {
            endpoint_replacements
                .get(&publication)
                .copied()
                .unwrap_or(publication)
        }),
        right: right.map(|publication| {
            endpoint_replacements
                .get(&publication)
                .copied()
                .unwrap_or(publication)
        }),
        output_attempt: output_attempt_replacements
            .get(&output_attempt)
            .copied()
            .unwrap_or(output_attempt),
    }
}

fn effect_semantic_record_key(
    domain: tex_state::EffectDomain,
    ordinal: tex_state::EffectSemanticRecordOrdinal,
) -> (
    tex_state::EffectDomain,
    tex_state::EffectSemanticRecordOrdinal,
) {
    let domain = match domain {
        // The attempt is the typed owner used to arbitrate candidates, not
        // part of the boundary record they compete to publish. Once the
        // endpoint publications map into the accepted restart domain, the
        // same endpoint gap and local ordinal are the canonical semantic
        // identity even when a replacement transaction owns the live claim.
        tex_state::EffectDomain::PublicationBoundary { left, right, .. } => {
            tex_state::EffectDomain::PublicationBoundary {
                left,
                right,
                output_attempt: tex_state::EffectOutputAttemptId::new(0),
            }
        }
        domain => domain,
    };
    (domain, ordinal)
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct EffectAttemptLineageSlot(Option<tex_state::EffectOutputAttemptId>);

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct EffectPlacementOwnerSlot(Option<tex_state::PageOutputEpisodeId>);

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct EffectSemanticRecordCollisionIdentity {
    domain: tex_state::EffectDomain,
    ordinal: tex_state::EffectSemanticRecordOrdinal,
    publication_record: Option<(
        tex_state::EffectPublicationId,
        tex_state::EffectPublicationRecordOrdinal,
    )>,
    publication_owned: bool,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct EffectSemanticRecordArbitrationKey {
    lineage_slot: EffectAttemptLineageSlot,
    placement_owner: EffectPlacementOwnerSlot,
    collision: EffectSemanticRecordCollisionIdentity,
}

fn effect_semantic_record_arbitration_key(
    domain: tex_state::EffectDomain,
    ordinal: tex_state::EffectSemanticRecordOrdinal,
) -> EffectSemanticRecordArbitrationKey {
    let lineage_slot = match domain {
        tex_state::EffectDomain::PublicationBoundary { output_attempt, .. } => Some(output_attempt),
        _ => None,
    };
    let (domain, ordinal) = effect_semantic_record_key(domain, ordinal);
    EffectSemanticRecordArbitrationKey {
        lineage_slot: EffectAttemptLineageSlot(lineage_slot),
        placement_owner: EffectPlacementOwnerSlot(None),
        collision: EffectSemanticRecordCollisionIdentity {
            domain,
            ordinal,
            publication_record: None,
            publication_owned: false,
        },
    }
}

fn effect_publication_record_arbitration_key(
    domain: tex_state::EffectDomain,
    ordinal: tex_state::EffectSemanticRecordOrdinal,
    publication: Option<tex_state::EffectPublicationId>,
    publication_record_ordinal: Option<tex_state::EffectPublicationRecordOrdinal>,
    episode_owner: Option<tex_state::PageOutputEpisodeId>,
) -> EffectSemanticRecordArbitrationKey {
    let mut key = effect_semantic_record_arbitration_key(domain, ordinal);
    key.placement_owner = EffectPlacementOwnerSlot(episode_owner);
    if publication.is_some() {
        key.collision.publication_owned = publication.is_some();
        key.collision.publication_record = publication.zip(publication_record_ordinal);
    }
    if key.collision.publication_record.is_some() {
        key.collision.ordinal = tex_state::EffectSemanticRecordOrdinal::new(0);
    }
    key
}

fn output_attempt_ancestry_mapping(
    accepted_attempts: &std::collections::BTreeSet<tex_state::EffectOutputAttemptId>,
    dispositions: &[tex_state::EffectPublicationDisposition],
    domain_replacements: &std::collections::BTreeMap<
        tex_state::EffectDomain,
        tex_state::EffectDomain,
    >,
    map_owner_transactions: bool,
) -> std::collections::BTreeMap<tex_state::EffectOutputAttemptId, tex_state::EffectOutputAttemptId>
{
    let mut accepted_episodes = std::collections::BTreeMap::new();
    let mut accepted_recursive_roots = std::collections::BTreeMap::new();
    let mut accepted_owner_attempts = std::collections::BTreeMap::new();
    for disposition in dispositions {
        let attempt = disposition.output_attempt();
        if !accepted_attempts.contains(&attempt) {
            continue;
        }
        if let Some(episode) = disposition.output_episode() {
            accepted_episodes.insert(episode, attempt);
            if disposition
                .recursive_receipt()
                .is_some_and(|receipt| receipt.identity() == episode.identity())
            {
                accepted_recursive_roots.insert(disposition.recursive_receipt(), attempt);
            }
        }
        if map_owner_transactions {
            accepted_owner_attempts.insert(
                (
                    disposition
                        .output_owner()
                        .iter()
                        .map(|owner| owner.identity())
                        .collect::<Vec<_>>(),
                    disposition.output_owner_ordinal(),
                ),
                attempt,
            );
        }
    }

    dispositions
        .iter()
        .filter(|disposition| !accepted_attempts.contains(&disposition.output_attempt()))
        .filter_map(|disposition| {
            let mapped_owner = disposition
                .output_owner()
                .iter()
                .map(|owner| {
                    let live = tex_state::EffectDomain::Paragraph(owner.identity());
                    match domain_replacements.get(&live).copied().unwrap_or(live) {
                        tex_state::EffectDomain::Paragraph(identity) => identity,
                        _ => unreachable!("paragraph owner maps only to a paragraph domain"),
                    }
                })
                .collect::<Vec<_>>();
            let accepted = map_owner_transactions
                .then_some(())
                .and_then(|_| {
                    accepted_owner_attempts
                        .get(&(mapped_owner, disposition.output_owner_ordinal()))
                        .copied()
                })
                .or_else(|| {
                    disposition
                        .output_episode()
                        .and_then(|episode| accepted_episodes.get(&episode).copied())
                        .or_else(|| {
                            accepted_recursive_roots
                                .get(&disposition.recursive_receipt())
                                .copied()
                        })
                })?;
            Some((disposition.output_attempt(), accepted))
        })
        .collect()
}

#[allow(clippy::too_many_arguments)]
fn assemble_effect_ledger(
    accepted: &[EffectRecord],
    accepted_sequences: &[tex_state::EffectSequence],
    accepted_publications: &[Option<tex_state::EffectPublicationId>],
    accepted_publication_record_ordinals: &[Option<tex_state::EffectPublicationRecordOrdinal>],
    accepted_episode_owners: &[Option<tex_state::PageOutputEpisodeId>],
    accepted_domains: &[tex_state::EffectDomain],
    accepted_semantic_record_ordinals: &[tex_state::EffectSemanticRecordOrdinal],
    accepted_placement_intra_orders: &[tex_state::EffectPlacementIntraOrder],
    live: &[EffectRecord],
    live_sequences: &[tex_state::EffectSequence],
    live_publications: &[Option<tex_state::EffectPublicationId>],
    live_publication_record_ordinals: &[Option<tex_state::EffectPublicationRecordOrdinal>],
    live_episode_owners: &[Option<tex_state::PageOutputEpisodeId>],
    live_domains: &[tex_state::EffectDomain],
    live_semantic_record_ordinals: &[tex_state::EffectSemanticRecordOrdinal],
    live_placement_intra_orders: &[tex_state::EffectPlacementIntraOrder],
    accepted_prefix: usize,
    superseded: &[tex_exec::SupersededPageOutputEpisode],
    effect_dispositions: &[tex_state::EffectPublicationDisposition],
    selected_artifact_publications: Option<
        &std::collections::BTreeMap<tex_state::EffectPublicationId, tex_state::EffectPublicationId>,
    >,
) -> AssembledEffectLedger {
    let mut rejected_accepted = std::collections::BTreeSet::new();
    let mut rejected_live = std::collections::BTreeSet::new();
    let mut inherited_live_sequences = std::collections::BTreeMap::new();
    let mut retained_winners = std::collections::BTreeSet::new();
    let mut selected_publications = std::collections::BTreeSet::new();
    let mut selected_output_attempts = std::collections::BTreeSet::new();
    let mut rejected_output_attempts = std::collections::BTreeSet::new();
    let mut final_winners = std::collections::BTreeMap::new();
    let mut rejected_publications = std::collections::BTreeSet::new();
    for disposition in effect_dispositions {
        if disposition.rejected().is_none() {
            selected_output_attempts.insert(disposition.output_attempt());
        } else {
            rejected_output_attempts.insert(disposition.output_attempt());
        }
        if let Some(rejected) = disposition.rejected() {
            rejected_publications.insert(rejected);
            if let Some(scratch) = final_winners.insert(rejected, disposition.winner()) {
                rejected_publications.insert(scratch);
            }
        } else {
            selected_publications.insert(disposition.winner());
        }
    }
    selected_output_attempts.retain(|attempt| !rejected_output_attempts.contains(attempt));
    selected_publications.extend(final_winners.values().copied());
    let accepted_publication_ids = accepted_publications
        .iter()
        .copied()
        .flatten()
        .collect::<std::collections::BTreeSet<_>>();
    for receipt in superseded {
        let Some(sealed) = seal_revision_output_publication(receipt) else {
            continue;
        };
        let retained = receipt.retained_publication();
        let regenerated = receipt
            .committed_publication()
            .map(|receipt| receipt.effect());
        match sealed.outcome {
            tex_state::OutputEpisodePublicationOutcome::Retained => {
                if let Some(regenerated) = regenerated {
                    rejected_live.insert(regenerated);
                }
                if let Some(retained) = retained {
                    retained_winners.insert(retained);
                }
            }
            tex_state::OutputEpisodePublicationOutcome::Regenerated => {
                if let Some(retained) = retained
                    && regenerated
                        .is_none_or(|regenerated| !rejected_publications.contains(&regenerated))
                {
                    rejected_accepted.insert(retained);
                }
                if let (Some(regenerated), Some(sequence)) =
                    (regenerated, receipt.retained_sequence())
                {
                    inherited_live_sequences.insert(regenerated, sequence);
                }
            }
        }
        if !rejected_publications.contains(&sealed.identity) {
            selected_publications.insert(sealed.identity);
        }
    }
    let pending_sequences = superseded
        .iter()
        .filter(|receipt| seal_revision_output_publication(receipt).is_none())
        .filter_map(tex_exec::SupersededPageOutputEpisode::retained_sequence)
        .collect::<Vec<_>>();
    let mut live_new_publications = Vec::new();
    for publication in live_publications.iter().copied().flatten() {
        if !accepted_publication_ids.contains(&publication)
            && !inherited_live_sequences.contains_key(&publication)
            && !live_new_publications.contains(&publication)
        {
            live_new_publications.push(publication);
        }
    }
    for (publication, sequence) in live_new_publications.into_iter().zip(pending_sequences) {
        inherited_live_sequences.insert(publication, sequence);
    }
    let artifact_constrained = selected_artifact_publications.is_some();
    if let Some(selected_artifact_publications) = selected_artifact_publications {
        let selected_artifact_publications = selected_artifact_publications
            .values()
            .map(|publication| {
                final_winners
                    .get(publication)
                    .copied()
                    .unwrap_or(*publication)
            })
            .collect::<std::collections::BTreeSet<_>>();
        selected_publications
            .retain(|publication| selected_artifact_publications.contains(publication));
    }
    let accepted_paragraph_domains =
        ordered_paragraph_domains(&accepted_domains[accepted_prefix.min(accepted_domains.len())..]);
    let live_paragraph_domains = ordered_paragraph_domains(live_domains);
    let has_live_paragraph_domains = !live_paragraph_domains.is_empty();
    let mut domain_replacements = live_paragraph_domains
        .into_iter()
        .zip(accepted_paragraph_domains)
        .collect::<std::collections::BTreeMap<_, _>>();
    let owner_domain_replacements = domain_replacements.clone();
    let accepted_publications_by_semantic_record = accepted_domains
        .iter()
        .copied()
        .zip(accepted_semantic_record_ordinals.iter().copied())
        .zip(accepted_publications.iter().copied())
        .filter_map(|((domain, ordinal), publication)| {
            publication
                .map(|publication| (effect_semantic_record_key(domain, ordinal), publication))
        })
        .collect::<std::collections::BTreeMap<_, _>>();
    let mut mapped_publications = final_winners.clone();
    for ((domain, ordinal), publication) in live_domains
        .iter()
        .copied()
        .zip(live_semantic_record_ordinals.iter().copied())
        .zip(live_publications.iter().copied())
    {
        let Some(publication) = publication else {
            continue;
        };
        let domain = domain_replacements.get(&domain).copied().unwrap_or(domain);
        if let Some(accepted) = accepted_publications_by_semantic_record
            .get(&effect_semantic_record_key(domain, ordinal))
        {
            mapped_publications.entry(publication).or_insert(*accepted);
        }
    }
    let accepted_boundary_attempts = accepted_domains
        .iter()
        .copied()
        .filter_map(|domain| {
            let tex_state::EffectDomain::PublicationBoundary { output_attempt, .. } = domain else {
                return None;
            };
            Some(output_attempt)
        })
        .collect::<std::collections::BTreeSet<_>>();
    let mut mapped_output_attempts = output_attempt_ancestry_mapping(
        &accepted_boundary_attempts,
        effect_dispositions,
        &owner_domain_replacements,
        true,
    );
    for (live, accepted) in output_attempt_ancestry_mapping(
        &accepted_boundary_attempts,
        effect_dispositions,
        &owner_domain_replacements,
        false,
    ) {
        mapped_output_attempts.entry(live).or_insert(accepted);
    }
    let mapped_accepted_output_attempts = mapped_output_attempts
        .values()
        .copied()
        .collect::<std::collections::BTreeSet<_>>();
    let dispositions_by_attempt = effect_dispositions
        .iter()
        .map(|disposition| (disposition.output_attempt(), disposition))
        .collect::<std::collections::BTreeMap<_, _>>();
    for (&live_attempt, &accepted_attempt) in &mapped_output_attempts {
        let (Some(live), Some(accepted)) = (
            dispositions_by_attempt.get(&live_attempt),
            dispositions_by_attempt.get(&accepted_attempt),
        ) else {
            continue;
        };
        let live_winner = live.rejected().unwrap_or_else(|| live.winner());
        mapped_publications.insert(live_winner, accepted.winner());
    }
    domain_replacements.extend(live_domains.iter().copied().filter_map(|domain| {
        let mapped =
            map_publication_boundary_domain(domain, &mapped_publications, &mapped_output_attempts);
        (mapped != domain).then_some((domain, mapped))
    }));
    let accepted_publication_semantic_ordinals = accepted_publications
        .iter()
        .copied()
        .zip(accepted_publication_record_ordinals.iter().copied())
        .zip(accepted_semantic_record_ordinals.iter().copied())
        .zip(accepted_domains.iter().copied())
        .filter_map(|(((publication, record), semantic), domain)| {
            Some(((publication?, record?), (semantic, domain)))
        })
        .collect::<std::collections::BTreeMap<_, _>>();
    let publication_record_replacements = live_publications
        .iter()
        .copied()
        .zip(live_publication_record_ordinals.iter().copied())
        .zip(live_semantic_record_ordinals.iter().copied())
        .zip(live_domains.iter().copied())
        .filter_map(|(((publication, record), semantic), domain)| {
            let publication = publication?;
            let record = record?;
            let accepted = mapped_publications.get(&publication).copied()?;
            accepted_publication_semantic_ordinals
                .get(&(accepted, record))
                .is_some_and(|(accepted_semantic, accepted_domain)| {
                    *accepted_domain == domain && *accepted_semantic != semantic
                })
                .then_some((publication, accepted))
        })
        .collect::<std::collections::BTreeMap<_, _>>();
    let accepted_semantic_sequences = accepted_domains
        .iter()
        .copied()
        .zip(accepted_semantic_record_ordinals.iter().copied())
        .zip(accepted_publications.iter().copied())
        .zip(accepted_publication_record_ordinals.iter().copied())
        .zip(accepted_episode_owners.iter().copied())
        .zip(accepted_sequences.iter().copied())
        .zip(accepted_placement_intra_orders.iter().copied())
        .map(
            |(
                (
                    ((((domain, ordinal), publication), publication_ordinal), episode_owner),
                    sequence,
                ),
                placement,
            )| {
                (
                    effect_publication_record_arbitration_key(
                        domain,
                        ordinal,
                        publication,
                        publication_ordinal,
                        episode_owner,
                    ),
                    (sequence, placement),
                )
            },
        )
        .collect::<std::collections::BTreeMap<_, _>>();
    let mut live_sequence_floor = None;
    let mut live_publication_positions = std::collections::BTreeMap::new();
    let mut retained_publication_positions = std::collections::BTreeMap::new();
    for (((publication, ordinal), sequence), placement) in live_publications
        .iter()
        .copied()
        .zip(live_publication_record_ordinals.iter().copied())
        .zip(live_sequences.iter().copied())
        .zip(live_placement_intra_orders.iter().copied())
    {
        let sequence = live_sequence_floor.map_or(sequence, |floor: tex_state::EffectSequence| {
            floor.max(sequence)
        });
        live_sequence_floor = Some(sequence);
        let (Some(publication), Some(ordinal)) = (publication, ordinal) else {
            continue;
        };
        live_publication_positions.insert((publication, ordinal), (sequence, placement));
        if let Some(retained) = final_winners.get(&publication).copied() {
            retained_publication_positions.insert((retained, ordinal), (sequence, placement));
        }
    }
    let live_domains_set = live_domains
        .iter()
        .copied()
        .map(|domain| domain_replacements.get(&domain).copied().unwrap_or(domain))
        .collect::<std::collections::BTreeSet<_>>();
    let live_world_floor = live_domains
        .iter()
        .filter_map(|domain| {
            let tex_state::EffectDomain::World(identity) = domain else {
                return None;
            };
            Some(*identity)
        })
        .min();
    let retained_prefix_publication_sequences =
        publication_sequence_set(accepted_sequences, accepted_publications, accepted_prefix);
    let mut records = Vec::new();
    collect_effect_assembly_records(
        &mut records,
        accepted,
        accepted_sequences,
        accepted_publications,
        accepted_publication_record_ordinals,
        accepted_episode_owners,
        accepted_domains,
        accepted_semantic_record_ordinals,
        accepted_placement_intra_orders,
        |index, _sequence, publication, domain, _ordinal| {
            index < accepted_prefix
                || publication.is_some_and(|id| retained_winners.contains(&id))
                || artifact_constrained
                    && publication.is_some_and(|id| selected_publications.contains(&id))
                || matches!(domain, tex_state::EffectDomain::TerminalPublication { .. })
                || publication.is_none()
                    && (match domain {
                        tex_state::EffectDomain::World(identity) => {
                            live_world_floor.is_none_or(|floor| identity < floor)
                        }
                        tex_state::EffectDomain::Paragraph(_) => !has_live_paragraph_domains,
                        _ => !live_domains_set.contains(&domain),
                    } || matches!(
                        domain,
                        tex_state::EffectDomain::PublicationBoundary {
                            output_attempt,
                            ..
                        } if mapped_accepted_output_attempts.contains(&output_attempt)
                    ))
        },
        &std::collections::BTreeMap::new(),
        &std::collections::BTreeMap::new(),
        &std::collections::BTreeMap::new(),
        &publication_record_replacements,
        &retained_publication_positions,
        EffectRecordOrigin::Accepted,
    );
    collect_effect_assembly_records(
        &mut records,
        live,
        live_sequences,
        live_publications,
        live_publication_record_ordinals,
        live_episode_owners,
        live_domains,
        live_semantic_record_ordinals,
        live_placement_intra_orders,
        |_, sequence, publication, domain, ordinal| {
            publication.is_none_or(|id| {
                !rejected_live.contains(&id)
                    && !rejected_publications.contains(&id)
                    && ((!effect_dispositions.is_empty()
                        && selected_publications.contains(&id)
                        && accepted_semantic_sequences.contains_key(
                            &effect_semantic_record_arbitration_key(domain, ordinal),
                        ))
                        || !retained_prefix_publication_sequences.contains(&sequence))
            })
        },
        &inherited_live_sequences,
        &domain_replacements,
        &accepted_semantic_sequences,
        &publication_record_replacements,
        &live_publication_positions,
        EffectRecordOrigin::Live,
    );
    let mut candidates_by_semantic_record = std::collections::BTreeMap::new();
    let mut terminal_candidates = Vec::new();
    for record in records {
        if let tex_state::EffectDomain::TerminalPublication {
            intra_order,
            committed,
            ..
        } = record.domain
        {
            let _ = (intra_order, committed);
            terminal_candidates.push(record);
            continue;
        }
        let key = record.arbitration_key;
        candidates_by_semantic_record
            .entry(key)
            .and_modify(|current: &mut EffectAssemblyRecord| {
                let is_selected = |candidate: &EffectAssemblyRecord| {
                    candidate.publication.map_or_else(
                        || {
                            candidate
                                .output_attempt
                                .is_none_or(|attempt| selected_output_attempts.contains(&attempt))
                        },
                        |publication| selected_publications.contains(&publication),
                    )
                };
                let record_selected = is_selected(&record);
                let current_selected = is_selected(current);
                if record_selected && !current_selected
                    || record_selected == current_selected
                        && record.origin == EffectRecordOrigin::Live
                        && current.origin == EffectRecordOrigin::Accepted
                {
                    *current = record.clone();
                }
            })
            .or_insert(record);
    }
    let mut selected_records = candidates_by_semantic_record
        .into_values()
        .filter(|record| {
            record.publication.map_or_else(
                || {
                    let selected_attempt = effect_dispositions.is_empty()
                        || record
                            .output_attempt
                            .is_none_or(|attempt| selected_output_attempts.contains(&attempt));
                    let selected_boundary = !artifact_constrained
                        || match record.domain {
                            tex_state::EffectDomain::PublicationBoundary {
                                left, right, ..
                            } => {
                                let map_endpoint = |publication| {
                                    let publication = selected_artifact_publications
                                        .and_then(|mapping| mapping.get(&publication).copied())
                                        .unwrap_or(publication);
                                    final_winners
                                        .get(&publication)
                                        .copied()
                                        .unwrap_or(publication)
                                };
                                let left = left.map(map_endpoint);
                                let right = right.map(map_endpoint);
                                left != right
                                    && left.into_iter().chain(right).any(|publication| {
                                        selected_publications.contains(&publication)
                                    })
                            }
                            _ => true,
                        };
                    selected_attempt && selected_boundary
                },
                |publication| selected_publications.contains(&publication),
            )
        })
        .collect::<Vec<_>>();
    if artifact_constrained {
        let map_endpoint = |publication| {
            let publication = selected_artifact_publications
                .and_then(|mapping| mapping.get(&publication).copied())
                .unwrap_or(publication);
            final_winners
                .get(&publication)
                .copied()
                .unwrap_or(publication)
        };
        let mut boundaries = std::collections::BTreeMap::new();
        selected_records.retain(|record| {
            let tex_state::EffectDomain::PublicationBoundary { left, right, .. } = record.domain
            else {
                return true;
            };
            let key = (left.map(map_endpoint), right.map(map_endpoint));
            boundaries
                .entry(key)
                .and_modify(|current: &mut EffectAssemblyRecord| {
                    if record.origin == EffectRecordOrigin::Live
                        && current.origin == EffectRecordOrigin::Accepted
                    {
                        *current = record.clone();
                    }
                })
                .or_insert_with(|| record.clone());
            false
        });
        selected_records.extend(boundaries.into_values());
    }
    let mut terminal = std::collections::BTreeMap::new();
    for record in terminal_candidates {
        let tex_state::EffectDomain::TerminalPublication {
            phase,
            intra_order,
            committed,
            ..
        } = record.domain
        else {
            unreachable!();
        };
        if !committed {
            continue;
        }
        terminal
            .entry((phase, intra_order))
            .and_modify(|current: &mut EffectAssemblyRecord| {
                if record.origin == EffectRecordOrigin::Live
                    && current.origin == EffectRecordOrigin::Accepted
                {
                    *current = record.clone();
                }
            })
            .or_insert(record);
    }
    let mut records = Vec::new();
    records.extend(terminal.into_values());
    records.extend(selected_records);
    records.sort_by_key(|record| (record.sequence, record.placement_intra_order));
    let mut effects = Vec::new();
    let mut sequences = Vec::new();
    let mut publications = Vec::new();
    let mut publication_record_ordinals = Vec::new();
    let mut episode_owners = Vec::new();
    let mut domains = Vec::new();
    let mut semantic_record_ordinals = Vec::new();
    let mut placement_intra_orders = Vec::new();
    for record in records {
        for effect in record.effects {
            effects.push(effect);
            sequences.push(record.sequence);
            publications.push(record.publication);
            publication_record_ordinals.push(record.publication_record_ordinal);
            episode_owners.push(record.episode_owner);
            domains.push(record.domain);
            semantic_record_ordinals.push(record.semantic_record_ordinal);
            placement_intra_orders.push(record.placement_intra_order);
        }
    }
    (
        effects,
        sequences,
        publications,
        publication_record_ordinals,
        episode_owners,
        domains,
        semantic_record_ordinals,
        placement_intra_orders,
    )
}

#[allow(clippy::too_many_arguments)]
fn assemble_artifact_ledger(
    accepted_artifacts: &[CommittedArtifact],
    accepted_records: &[tex_state::ArtifactPublicationRecord],
    accepted_pages: &[DviPagePlan],
    live_artifacts: &[CommittedArtifact],
    live_records: &[tex_state::ArtifactPublicationRecord],
    live_pages: &[DviPagePlan],
    live_dvi_records: &[tex_state::ArtifactPublicationRecord],
    accepted_prefix: usize,
    superseded: &[tex_exec::SupersededPageOutputEpisode],
    selected_effect_publications: &[Option<tex_state::EffectPublicationId>],
    live_effect_publications: &[Option<tex_state::EffectPublicationId>],
    live_effect_sequences: &[tex_state::EffectSequence],
) -> (
    Vec<CommittedArtifact>,
    Vec<tex_state::ArtifactPublicationRecord>,
    Vec<DviPagePlan>,
) {
    #[derive(Clone, Copy)]
    enum Origin {
        Accepted(usize),
        Live(usize),
    }

    let mut rejected_accepted = std::collections::BTreeSet::new();
    let mut rejected_live = std::collections::BTreeSet::new();
    let mut retained_winners = std::collections::BTreeSet::new();
    let mut live_order_sequence = None;
    let live_order = live_records
        .iter()
        .copied()
        .enumerate()
        .map(|(index, record)| {
            let sequence = live_order_sequence
                .map_or(record.sequence(), |floor: tex_state::EffectSequence| {
                    floor.max(record.sequence())
                });
            live_order_sequence = Some(sequence);
            (record.publication(), (sequence, index))
        })
        .collect::<std::collections::BTreeMap<_, _>>();
    let mut retained_live_order = std::collections::BTreeMap::new();
    let effect_sequences = live_effect_publications
        .iter()
        .copied()
        .zip(live_effect_sequences.iter().copied())
        .filter_map(|(publication, sequence)| {
            publication.map(|publication| (publication, sequence))
        })
        .collect::<std::collections::BTreeMap<_, _>>();
    let mut inherited = std::collections::BTreeMap::new();
    for receipt in superseded {
        let Some(sealed) = seal_revision_output_publication(receipt) else {
            continue;
        };
        let retained = receipt
            .accepted_artifacts()
            .clone()
            .filter_map(|index| accepted_records.get(index))
            .map(|record| record.publication())
            .collect::<Vec<_>>();
        let regenerated = receipt
            .committed_publication()
            .into_iter()
            .flat_map(tex_state::PageOutputPublicationReceipt::artifacts)
            .map(|record| record.publication())
            .collect::<Vec<_>>();
        match sealed.outcome {
            tex_state::OutputEpisodePublicationOutcome::Retained => {
                for (retained, regenerated) in retained.iter().copied().zip(&regenerated) {
                    if let Some(order) = live_order.get(regenerated).copied() {
                        retained_live_order.insert(retained, order);
                    }
                }
                for regenerated in regenerated {
                    rejected_live.insert(regenerated);
                }
                for retained in retained {
                    retained_winners.insert(retained);
                }
            }
            tex_state::OutputEpisodePublicationOutcome::Regenerated => {
                let _ = retained;
                let retained_records = receipt
                    .accepted_artifacts()
                    .clone()
                    .filter_map(|index| accepted_records.get(index))
                    .copied()
                    .collect::<Vec<_>>();
                let replacement_effects = receipt
                    .committed_publication()
                    .map(tex_state::PageOutputPublicationReceipt::effect)
                    .into_iter()
                    .collect::<std::collections::BTreeSet<_>>();
                let regenerated_records = receipt
                    .committed_publication()
                    .into_iter()
                    .flat_map(tex_state::PageOutputPublicationReceipt::artifacts);
                for regenerated in regenerated_records {
                    let sequence = artifact_winner_sequence(
                        &retained_records,
                        &replacement_effects,
                        *regenerated,
                        &effect_sequences,
                    );
                    inherited.insert(regenerated.publication(), sequence);
                }
            }
        }
    }

    let mut selected = std::collections::BTreeMap::new();
    let live_world_floor = live_records
        .iter()
        .filter_map(|record| match record.domain() {
            tex_state::EffectDomain::World(identity) => Some(identity),
            _ => None,
        })
        .min();
    let selected_effect_publications = selected_effect_publications
        .iter()
        .copied()
        .flatten()
        .collect::<std::collections::BTreeSet<_>>();
    let accepted_replacement_records = superseded
        .iter()
        .filter(|receipt| {
            seal_revision_output_publication(receipt).is_none_or(|sealed| {
                sealed.outcome == tex_state::OutputEpisodePublicationOutcome::Regenerated
            })
        })
        .flat_map(|receipt| receipt.accepted_artifacts().clone())
        .filter_map(|index| accepted_records.get(index).copied())
        .collect::<Vec<_>>();
    let live_replacement_publications = superseded
        .iter()
        .filter_map(tex_exec::SupersededPageOutputEpisode::committed_publication)
        .flat_map(tex_state::PageOutputPublicationReceipt::artifacts)
        .map(|record| record.publication())
        .collect::<std::collections::BTreeSet<_>>();
    let live_replacement_records = live_records
        .iter()
        .copied()
        .filter(|record| live_replacement_publications.contains(&record.publication()))
        .collect::<Vec<_>>();
    let retained_replacement_prefix = accepted_replacement_records
        .len()
        .saturating_sub(live_replacement_records.len());
    retained_winners.extend(
        accepted_replacement_records[..retained_replacement_prefix]
            .iter()
            .map(|record| record.publication()),
    );
    let accepted_replacement_records = accepted_replacement_records
        .into_iter()
        .rev()
        .take(live_replacement_records.len())
        .collect::<Vec<_>>()
        .into_iter()
        .rev();
    for (retained, regenerated) in accepted_replacement_records
        .into_iter()
        .zip(live_replacement_records)
    {
        rejected_accepted.insert(retained.publication());
        inherited.insert(regenerated.publication(), retained.sequence());
    }
    for live in live_records.iter().copied().filter(|record| {
        !rejected_live.contains(&record.publication())
            && !live_replacement_publications.contains(&record.publication())
    }) {
        if let Some(accepted) = accepted_records
            .iter()
            .copied()
            .skip(accepted_prefix)
            .find(|accepted| accepted.domain() == live.domain())
        {
            rejected_accepted.insert(accepted.publication());
            retained_winners.remove(&accepted.publication());
        }
    }
    let unmatched_accepted = accepted_records
        .iter()
        .copied()
        .enumerate()
        .skip(accepted_prefix)
        .filter(|(_, record)| {
            !rejected_accepted.contains(&record.publication())
                && !retained_winners.contains(&record.publication())
        })
        .map(|(_, record)| record.publication())
        .collect::<Vec<_>>();
    let unmatched_live = live_records.iter().copied().filter(|record| {
        !rejected_live.contains(&record.publication())
            && !live_replacement_publications.contains(&record.publication())
    });
    for (accepted, _) in unmatched_accepted.into_iter().zip(unmatched_live) {
        rejected_accepted.insert(accepted);
    }
    let mut accepted_effect_mapping = std::collections::BTreeMap::new();
    for receipt in superseded {
        let Some(sealed) = seal_revision_output_publication(receipt) else {
            continue;
        };
        match sealed.outcome {
            tex_state::OutputEpisodePublicationOutcome::Retained => {
                for index in receipt.accepted_artifacts().clone() {
                    accepted_effect_mapping.insert(index, sealed.identity);
                }
            }
            tex_state::OutputEpisodePublicationOutcome::Regenerated => {
                let _ = sealed;
            }
        }
    }
    for (index, record) in accepted_records.iter().copied().enumerate() {
        let mapped_effect = accepted_effect_mapping
            .get(&index)
            .copied()
            .or_else(|| record.effect_publication());
        let precedes_live_world_suffix = live_world_floor.is_none_or(|floor| {
            !matches!(record.domain(), tex_state::EffectDomain::World(identity) if identity >= floor)
        });
        if !rejected_accepted.contains(&record.publication())
            && (index < accepted_prefix
                || retained_winners.contains(&record.publication())
                || (precedes_live_world_suffix
                    && mapped_effect
                        .is_some_and(|effect| selected_effect_publications.contains(&effect))))
        {
            let (sequence, live_rank, live_index) = retained_live_order
                .get(&record.publication())
                .copied()
                .map_or((record.sequence(), 0, 0), |(sequence, index)| {
                    (sequence, 1, index)
                });
            selected.insert(
                (
                    sequence,
                    record.intra_order(),
                    live_rank,
                    live_index,
                    record.publication(),
                ),
                (record, Origin::Accepted(index)),
            );
        }
    }
    for (index, record) in live_records.iter().copied().enumerate() {
        if rejected_live.contains(&record.publication()) {
            continue;
        }
        // Regenerated artifacts keep their committed live order.  The
        // retained sequence identifies the accepted winner to replace; using
        // it as the regenerated record's sort key moves a later live page in
        // front of pages that were committed earlier in this generation.
        selected.retain(|_, (candidate, _)| candidate.publication() != record.publication());
        let inherited_sequence = inherited
            .get(&record.publication())
            .copied()
            .unwrap_or_else(|| record.sequence());
        let sequence = live_order
            .get(&record.publication())
            .map_or(inherited_sequence, |(sequence, _)| {
                (*sequence).max(inherited_sequence)
            });
        selected.insert(
            (
                sequence,
                record.intra_order(),
                1,
                index,
                record.publication(),
            ),
            (record, Origin::Live(index)),
        );
    }
    let mut artifacts = Vec::with_capacity(selected.len());
    let mut records = Vec::with_capacity(selected.len());
    let mut pages = Vec::new();
    let accepted_pages_by_publication = accepted_records
        .iter()
        .copied()
        .zip(accepted_pages)
        .map(|(record, page)| (record.publication(), page))
        .collect::<std::collections::BTreeMap<_, _>>();
    let live_pages_by_publication = live_dvi_records
        .iter()
        .copied()
        .zip(live_pages)
        .map(|(record, page)| (record.publication(), page))
        .collect::<std::collections::BTreeMap<_, _>>();
    for (_, (record, origin)) in selected {
        let (artifact, page) = match origin {
            Origin::Accepted(index) => (
                accepted_artifacts.get(index),
                accepted_pages_by_publication
                    .get(&record.publication())
                    .copied(),
            ),
            Origin::Live(index) => (
                live_artifacts.get(index),
                live_pages_by_publication
                    .get(&record.publication())
                    .copied(),
            ),
        };
        if let Some(artifact) = artifact {
            artifacts.push(artifact.clone());
            records.push(record);
            if let Some(page) = page {
                pages.push(page.clone());
            }
        }
    }
    (artifacts, records, pages)
}

fn artifact_winner_sequence(
    retained: &[tex_state::ArtifactPublicationRecord],
    replacement_effects: &std::collections::BTreeSet<tex_state::EffectPublicationId>,
    regenerated: tex_state::ArtifactPublicationRecord,
    effect_sequences: &std::collections::BTreeMap<
        tex_state::EffectPublicationId,
        tex_state::EffectSequence,
    >,
) -> tex_state::EffectSequence {
    regenerated
        .effect_publication()
        .filter(|effect| replacement_effects.contains(effect))
        .and_then(|_| {
            retained
                .iter()
                .find(|record| record.intra_order() == regenerated.intra_order())
        })
        .map(|record| record.sequence())
        .or_else(|| {
            regenerated
                .effect_publication()
                .and_then(|publication| effect_sequences.get(&publication).copied())
        })
        .unwrap_or_else(|| regenerated.sequence())
}

fn publication_sequence_set(
    sequences: &[tex_state::EffectSequence],
    publications: &[Option<tex_state::EffectPublicationId>],
    start_limit: usize,
) -> std::collections::BTreeSet<tex_state::EffectSequence> {
    let mut result = std::collections::BTreeSet::new();
    let mut previous = None;
    for (index, publication) in publications.iter().copied().enumerate() {
        if publication != previous {
            if index < start_limit
                && let (Some(_), Some(sequence)) = (publication, sequences.get(index).copied())
            {
                result.insert(sequence);
            }
            previous = publication;
        }
    }
    result
}

#[allow(clippy::too_many_arguments)]
fn collect_effect_assembly_records(
    destination: &mut Vec<EffectAssemblyRecord>,
    effects: &[EffectRecord],
    sequences: &[tex_state::EffectSequence],
    publications: &[Option<tex_state::EffectPublicationId>],
    publication_record_ordinals: &[Option<tex_state::EffectPublicationRecordOrdinal>],
    episode_owners: &[Option<tex_state::PageOutputEpisodeId>],
    domains: &[tex_state::EffectDomain],
    semantic_record_ordinals: &[tex_state::EffectSemanticRecordOrdinal],
    placement_intra_orders: &[tex_state::EffectPlacementIntraOrder],
    keep: impl Fn(
        usize,
        tex_state::EffectSequence,
        Option<tex_state::EffectPublicationId>,
        tex_state::EffectDomain,
        tex_state::EffectSemanticRecordOrdinal,
    ) -> bool,
    inherited_sequences: &std::collections::BTreeMap<
        tex_state::EffectPublicationId,
        tex_state::EffectSequence,
    >,
    domain_replacements: &std::collections::BTreeMap<
        tex_state::EffectDomain,
        tex_state::EffectDomain,
    >,
    inherited_semantic_sequences: &std::collections::BTreeMap<
        EffectSemanticRecordArbitrationKey,
        (
            tex_state::EffectSequence,
            tex_state::EffectPlacementIntraOrder,
        ),
    >,
    publication_replacements: &std::collections::BTreeMap<
        tex_state::EffectPublicationId,
        tex_state::EffectPublicationId,
    >,
    publication_positions: &std::collections::BTreeMap<
        (
            tex_state::EffectPublicationId,
            tex_state::EffectPublicationRecordOrdinal,
        ),
        (
            tex_state::EffectSequence,
            tex_state::EffectPlacementIntraOrder,
        ),
    >,
    origin: EffectRecordOrigin,
) {
    let mut index = 0;
    while index < effects.len() {
        let publication = publications.get(index).copied().flatten();
        let publication_record_ordinal = publication_record_ordinals.get(index).copied().flatten();
        let episode_owner = episode_owners.get(index).copied().flatten();
        let mapped_publication = publication.map(|publication| {
            publication_replacements
                .get(&publication)
                .copied()
                .unwrap_or(publication)
        });
        let end = index + 1;
        let raw_sequence = publication
            .and_then(|id| inherited_sequences.get(&id).copied())
            .or_else(|| sequences.get(index).copied())
            .unwrap_or_else(|| tex_state::EffectSequence::new(u64::MAX));
        let domain = domains
            .get(index)
            .copied()
            .unwrap_or(tex_state::EffectDomain::World(u64::MAX));
        let output_attempt = match domain {
            tex_state::EffectDomain::PublicationBoundary { output_attempt, .. } => {
                Some(output_attempt)
            }
            _ => None,
        };
        let domain = domain_replacements.get(&domain).copied().unwrap_or(domain);
        let semantic_record_ordinal = semantic_record_ordinals
            .get(index)
            .copied()
            .unwrap_or_else(|| tex_state::EffectSemanticRecordOrdinal::new(u64::MAX));
        let raw_placement_intra_order = placement_intra_orders
            .get(index)
            .copied()
            .unwrap_or_else(|| tex_state::EffectPlacementIntraOrder::new(u64::MAX));
        let semantic_key = effect_publication_record_arbitration_key(
            domain,
            semantic_record_ordinal,
            mapped_publication,
            publication_record_ordinal,
            episode_owner,
        );
        let first_semantic_key = effect_semantic_record_arbitration_key(
            domain,
            tex_state::EffectSemanticRecordOrdinal::new(0),
        );
        let (sequence, placement_intra_order) =
            if origin == EffectRecordOrigin::Live && output_attempt.is_some() {
                (raw_sequence, raw_placement_intra_order)
            } else {
                mapped_publication
                    .zip(publication_record_ordinal)
                    .and_then(|key| publication_positions.get(&key).copied())
                    .or_else(|| {
                        inherited_semantic_sequences
                            .get(&semantic_key)
                            .copied()
                            .or_else(|| {
                                if semantic_key.collision.publication_record.is_some() {
                                    return None;
                                }
                                inherited_semantic_sequences
                                    .range(first_semantic_key..=semantic_key)
                                    .next_back()
                                    .map(|(_, placement)| *placement)
                            })
                    })
                    .unwrap_or((raw_sequence, raw_placement_intra_order))
            };
        if keep(
            index,
            sequence,
            publication,
            domain,
            semantic_record_ordinal,
        ) {
            destination.push(EffectAssemblyRecord {
                arbitration_key: semantic_key,
                sequence,
                publication,
                publication_record_ordinal,
                episode_owner,
                domain,
                semantic_record_ordinal,
                placement_intra_order,
                output_attempt,
                effects: effects[index..end].to_vec(),
                origin,
            });
        }
        index = end;
    }
}

fn ordered_paragraph_domains(domains: &[tex_state::EffectDomain]) -> Vec<tex_state::EffectDomain> {
    let mut result = Vec::new();
    for domain in domains.iter().copied() {
        if matches!(domain, tex_state::EffectDomain::Paragraph(_)) && !result.contains(&domain) {
            result.push(domain);
        }
    }
    result
}

fn extend_live_effects_excluding_superseded_output(
    destination: &mut Vec<EffectRecord>,
    live: &[EffectRecord],
    owners: &[Option<tex_state::PageOutputEpisodeId>],
    range: std::ops::Range<usize>,
    superseded: &[tex_exec::SupersededPageOutputEpisode],
) {
    destination.extend(range.filter_map(|index| {
        let owner = owners.get(index).copied().flatten();
        let rejected = owner.is_some_and(|owner| {
            superseded.iter().any(|receipt| {
                receipt.identity() == owner
                    && receipt
                        .regenerated_effects()
                        .is_some_and(|range| range.contains(&index))
                    && seal_revision_output_publication(receipt).is_some_and(|sealed| {
                        sealed.outcome == tex_state::OutputEpisodePublicationOutcome::Retained
                    })
            })
        });
        (!rejected).then(|| live[index].clone())
    }));
}

#[derive(Clone, Copy)]
struct SealedRevisionOutputPublication {
    identity: tex_state::EffectPublicationId,
    outcome: tex_state::OutputEpisodePublicationOutcome,
}

fn seal_revision_output_publication(
    receipt: &tex_exec::SupersededPageOutputEpisode,
) -> Option<SealedRevisionOutputPublication> {
    let committed = receipt.committed_publication()?;
    let outcome = receipt.publication_candidate()?.revision_outcome();
    let identity = match outcome {
        tex_state::OutputEpisodePublicationOutcome::Retained => receipt.retained_publication()?,
        tex_state::OutputEpisodePublicationOutcome::Regenerated => committed.effect(),
    };
    Some(SealedRevisionOutputPublication { identity, outcome })
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
    UnexpectedResource,
    ResourceNoProgress {
        need: Box<ResourceNeed>,
        site: Option<tex_state::provenance::DiagnosticSite>,
    },
    SourceRegistration(SourceRegistrationError),
    CommandSummary(tex_command::CommandSummaryError),
    MissingAcceptedSubstrate,
    Execute(tex_exec::ExecError),
    World(WorldError),
    Restore(Box<EditorRestoreError>),
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
            Self::UnexpectedResource => {
                f.write_str("resource fulfillment does not match the pending need")
            }
            Self::ResourceNoProgress { need, .. } => write!(
                f,
                "retained host answered {need:?}, but replay suspended on the identical need again before committing progress"
            ),
            Self::SourceRegistration(error) => {
                write!(f, "source registration failed: {error}")
            }
            Self::CommandSummary(error) => write!(f, "checkpoint failed: {error}"),
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
            Self::ResourceNoProgress { site, .. } => site.clone(),
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
        Self::Restore(Box::new(value))
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
