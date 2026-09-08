use std::fmt;
use std::sync::Arc;

use tex_command::{CommandObserver, CommandSummaryError, ResourceProvider};
use tex_state::Universe;
use tex_state::fork_arena::{CheckpointMark, ChunkPool, ForkArena};

use crate::{
    Cancellation, CheckpointSink, EngineBoundary, ExecError, ExecutionBudgetCounters, MainControl,
    MainControlStep, ResourceFulfillment, ResourceNeed, SemanticEpisodeBarrier, StepResult,
};

/// Failure returned through the canonical step protocol.
#[derive(Debug)]
pub enum CanonicalStepFailure {
    Execution(ExecError),
    Checkpoint(CommandSummaryError),
}

impl fmt::Display for CanonicalStepFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Execution(error) => error.fmt(formatter),
            Self::Checkpoint(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for CanonicalStepFailure {}

/// Result of one bounded canonical operation.
#[derive(Debug)]
pub enum CanonicalStepResult {
    Progress(MainControlStep),
    ResourceNeed(ResourceNeed),
    /// A resource provider was called during this operation but declined to
    /// answer it.  The caller must suspend once and may retry the operation
    /// with a provider that can answer the owned need; no replay answer has
    /// been installed in the semantic state.
    ResourceSuspended(ResourceNeed),
    Committed(MainControlStep),
    Completed(MainControlStep),
    Failed(CanonicalStepFailure),
}

/// Unforgeable proof that the canonical runner reached a quiescent terminal
/// step for the ledger/control pair that will detach its output.
///
/// Construction is private. The receipt is intentionally neither `Clone` nor
/// `Copy`; [`OutputLedger::close_revision`] also validates the still-live
/// ledger and control state before consuming any output.
#[derive(Debug)]
pub struct TerminalRevisionReceipt {
    step: MainControlStep,
    suspension_serial: u64,
}

impl TerminalRevisionReceipt {
    #[must_use]
    pub const fn step(&self) -> MainControlStep {
        self.step
    }
}

/// Publication and retry state shared by cold and incremental revisions.
///
/// `MainControl` owns atomic semantic rollback. This ledger owns everything
/// that may become visible after such an operation commits: named checkpoint
/// capture, exact resource registration, authoritative absence, and the
/// monotonic suspension serial.
pub struct OutputLedger {
    pool: ChunkPool<crate::PreparedDviPage>,
    pages: ForkArena<crate::PreparedDviPage, OutputLane>,
    prepared_page_count: usize,
    accepted_head_count: Option<usize>,
    job_start_committed: bool,
    suspension_serial: u64,
    terminal_step: Option<MainControlStep>,
    terminal_closed: bool,
}

pub(crate) enum OutputLane {}

/// Fixed rooted coordinate into the one accepted output lineage.
#[derive(Clone, Copy, Debug)]
pub(crate) struct OutputLedgerCheckpoint {
    mark: CheckpointMark<OutputLane>,
    prepared_page_count: usize,
}

pub(crate) struct PreparedOutputReplayRestore {
    ledger: usize,
    checkpoint: OutputLedgerCheckpoint,
}

impl Default for OutputLedger {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for OutputLedger {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("OutputLedger")
            .field("prepared_dvi_pages", &self.prepared_page_count)
            .field("candidate", &self.accepted_head_count.is_some())
            .finish_non_exhaustive()
    }
}

impl OutputLedger {
    #[must_use]
    pub fn new() -> Self {
        Self {
            pool: ChunkPool::default(),
            pages: ForkArena::new(),
            prepared_page_count: 0,
            accepted_head_count: None,
            job_start_committed: false,
            suspension_serial: 0,
            terminal_step: None,
            terminal_closed: false,
        }
    }

    pub(crate) fn can_resume(&self, checkpoint: OutputLedgerCheckpoint) -> bool {
        self.accepted_head_count.is_none()
            && checkpoint.prepared_page_count <= self.prepared_page_count
            && self.pages.can_begin_checkpoint_candidate(checkpoint.mark)
    }

    /// Rewinds this sole output owner to a retained whole-chunk boundary.
    pub(crate) fn resume(
        &mut self,
        checkpoint: OutputLedgerCheckpoint,
    ) -> Result<(), tex_state::fork_arena::ForkArenaError> {
        if !self.can_resume(checkpoint) {
            return Err(tex_state::fork_arena::ForkArenaError::InvalidCheckpoint);
        }
        let accepted_head_count = self.prepared_page_count;
        self.pages
            .begin_checkpoint_candidate(&mut self.pool, checkpoint.mark)?;
        self.accepted_head_count = Some(accepted_head_count);
        self.prepared_page_count = checkpoint.prepared_page_count;
        self.job_start_committed = true;
        self.suspension_serial = 0;
        self.terminal_step = None;
        self.terminal_closed = false;
        Ok(())
    }

    /// Rewinds the current candidate output lineage to a whole-checkpoint
    /// mark.  A resource miss may happen after pages have been prepared, but
    /// it must never leave those pages visible when the engine retries from a
    /// full semantic checkpoint.
    pub(crate) fn rewind(
        &mut self,
        checkpoint: OutputLedgerCheckpoint,
    ) -> Result<(), tex_state::fork_arena::ForkArenaError> {
        if self.accepted_head_count.is_some() {
            self.pages
                .restore_current_checkpoint(&mut self.pool, checkpoint.mark)?;
            self.prepared_page_count = checkpoint.prepared_page_count;
            self.job_start_committed = true;
            self.suspension_serial = 0;
            self.terminal_step = None;
            self.terminal_closed = false;
            Ok(())
        } else {
            self.pages
                .restore_accepted_checkpoint(&mut self.pool, checkpoint.mark)?;
            self.prepared_page_count = checkpoint.prepared_page_count;
            self.job_start_committed = true;
            self.suspension_serial = 0;
            self.terminal_step = None;
            self.terminal_closed = false;
            Ok(())
        }
    }

    pub(crate) fn prepare_replay_restore(
        &self,
        checkpoint: OutputLedgerCheckpoint,
    ) -> Result<PreparedOutputReplayRestore, tex_state::fork_arena::ForkArenaError> {
        let mark_valid = if self.accepted_head_count.is_some() {
            self.pages.validates_checkpoint(checkpoint.mark)
        } else {
            self.pages.can_begin_checkpoint_candidate(checkpoint.mark)
        };
        if checkpoint.prepared_page_count > self.prepared_page_count || !mark_valid {
            return Err(tex_state::fork_arena::ForkArenaError::InvalidCheckpoint);
        }
        Ok(PreparedOutputReplayRestore {
            ledger: std::ptr::from_ref(self) as usize,
            checkpoint,
        })
    }

    pub(crate) fn apply_prepared_replay_restore(
        &mut self,
        prepared: PreparedOutputReplayRestore,
    ) -> Result<(), tex_state::fork_arena::ForkArenaError> {
        if prepared.ledger != std::ptr::from_ref(self) as usize {
            return Err(tex_state::fork_arena::ForkArenaError::InvalidCheckpoint);
        }
        self.rewind(prepared.checkpoint)
    }

    pub(crate) fn checkpoint(&mut self) -> OutputLedgerCheckpoint {
        let boundary = self
            .pages
            .seal_boundary(&mut self.pool)
            .expect("output checkpoint retires every page builder");
        let mark = self
            .pages
            .checkpoint_mark(boundary)
            .expect("output checkpoint names the just-sealed boundary");
        OutputLedgerCheckpoint {
            mark,
            prepared_page_count: self.prepared_page_count,
        }
    }

    #[doc(hidden)]
    pub fn accept_checkpoint_candidate(&mut self) {
        if self.accepted_head_count.take().is_none() {
            return;
        }
        let boundary = self
            .pages
            .seal_boundary(&mut self.pool)
            .expect("accepted output has no live page builder");
        self.pages
            .accept_checkpoint_candidate(&mut self.pool, boundary)
            .expect("accepted output settles its sole current lineage");
    }

    #[doc(hidden)]
    pub fn reject_checkpoint_candidate(&mut self) {
        let Some(accepted_head_count) = self.accepted_head_count.take() else {
            return;
        };
        let boundary = self
            .pages
            .seal_boundary(&mut self.pool)
            .expect("rejected output has no live page builder");
        self.pages
            .reject_checkpoint_candidate(&mut self.pool, boundary)
            .expect("rejected output reattaches its sole prior lineage");
        self.prepared_page_count = accepted_head_count;
    }

    fn collect_prepared_pages<G>(&mut self, control: &mut MainControl<G>) {
        let prepared = control.take_prepared_dvi_pages();
        if prepared.is_empty() {
            return;
        }
        let added = prepared.len();
        let mut builder = self
            .pages
            .begin_builder(&mut self.pool)
            .expect("output collection owns the sole page builder");
        for page in prepared {
            builder
                .push(page)
                .expect("prepared output page fits the fixed-chunk arena");
        }
        let _ = builder.finish();
        self.prepared_page_count = self.prepared_page_count.saturating_add(added);
    }

    #[must_use]
    pub const fn suspension_serial(&self) -> u64 {
        self.suspension_serial
    }

    /// Records a need that crossed the host boundary rather than being
    /// answered synchronously inside the current drive call.
    pub fn record_suspension(&mut self) {
        self.suspension_serial = self.suspension_serial.saturating_add(1);
    }

    /// Returns the terminal capability armed by this ledger's canonical
    /// runner. A guessed step or a partial/non-quiescent execution is rejected
    /// without changing either the ledger or the executor.
    pub fn terminal_receipt<G>(
        &self,
        control: &MainControl<G>,
        universe: &mut Universe<G>,
        step: MainControlStep,
    ) -> Result<TerminalRevisionReceipt, crate::EngineCompletionError> {
        if self.terminal_closed
            || self.terminal_step != Some(step)
            || !control.terminal_is_quiescent(universe)
        {
            return Err(crate::EngineCompletionError::TerminalRevisionUnavailable);
        }
        Ok(TerminalRevisionReceipt {
            step,
            suspension_serial: self.suspension_serial,
        })
    }

    /// Visits the terminal revision's retained DVI plans without moving or
    /// duplicating the sole output-ledger page owner.
    pub fn visit_terminal_dvi_pages<G>(
        &mut self,
        control: &mut MainControl<G>,
        receipt: &TerminalRevisionReceipt,
        visit: &mut dyn FnMut(&tex_out::dvi::DviPagePlan),
    ) -> Result<usize, crate::EngineCompletionError> {
        if self.terminal_closed
            || self.terminal_step != Some(receipt.step)
            || self.suspension_serial != receipt.suspension_serial
        {
            return Err(crate::EngineCompletionError::TerminalRevisionUnavailable);
        }
        self.collect_prepared_pages(control);
        let output_checkpoint = self.checkpoint();
        self.pages
            .visit_checkpoint_values(
                &self.pool,
                output_checkpoint.mark,
                &mut |page: &crate::PreparedDviPage| {
                    visit(page.plan());
                },
            )
            .expect("terminal output visits its sealed accepted/current lineage");
        Ok(self.prepared_page_count)
    }

    /// Closes all executor-owned output ledgers after a terminal committed
    /// step. Suspension never calls this method and therefore cannot expose a
    /// partial revision patch.
    pub fn close_revision<G>(
        &mut self,
        control: &mut MainControl<G>,
        universe: &mut Universe<G>,
        receipt: &TerminalRevisionReceipt,
        demand: crate::EngineCompletionDemand,
        artifact_base: usize,
    ) -> Result<crate::DetachedEngineCompletion, crate::EngineCompletionError> {
        if self.terminal_closed
            || self.terminal_step != Some(receipt.step)
            || self.suspension_serial != receipt.suspension_serial
            || !control.terminal_is_quiescent(universe)
        {
            return Err(crate::EngineCompletionError::TerminalRevisionUnavailable);
        }
        self.collect_prepared_pages(control);
        let pdf = demand
            .pdf()
            .then(|| {
                universe
                    .command_context()
                    .map_err(crate::EngineCompletionError::Admission)?
                    .detach_pdf_completion()
                    .map_err(crate::EngineCompletionError::Pdf)
            })
            .transpose()?;
        let world = universe.world();
        let effect_base = world
            .effect_pos()
            .raw()
            .saturating_sub(u64::try_from(world.effect_records().len()).unwrap_or(u64::MAX));
        let (effects, stream_open_contexts) = world.detached_effect_records();
        let artifacts = world
            .committed_artifacts()
            .get(artifact_base..)
            .ok_or(crate::EngineCompletionError::ArtifactPublicationCount)?;
        let artifact_publications = world
            .artifact_publications()
            .get(artifact_base..)
            .ok_or(crate::EngineCompletionError::ArtifactPublicationCount)?;
        let output_checkpoint = self.checkpoint();
        let completion = crate::DetachedEngineCompletion::capture_borrowed_pages(
            effect_base,
            effects,
            stream_open_contexts,
            artifacts.to_vec(),
            artifact_publications,
            self.prepared_page_count,
            |visit| {
                self.pages
                    .visit_checkpoint_values(&self.pool, output_checkpoint.mark, visit)
                    .expect("terminal output visits its sealed accepted/current lineage");
            },
            pdf,
        )?;
        self.terminal_closed = true;
        Ok(completion)
    }

    /// Detaches the output prefix sealed by the most recently published
    /// checkpoint. The live executor is intentionally left nonterminal: the
    /// incremental owner will reject this generation after joining the
    /// detached prefix to an accepted suffix.
    #[doc(hidden)]
    pub fn detach_checkpoint_prefix<G>(
        &mut self,
        control: &mut MainControl<G>,
        universe: &mut Universe<G>,
    ) -> Result<crate::DetachedEngineCompletion, crate::EngineCompletionError> {
        self.collect_prepared_pages(control);
        let world = universe.world();
        let effect_base = world
            .effect_pos()
            .raw()
            .saturating_sub(u64::try_from(world.effect_records().len()).unwrap_or(u64::MAX));
        let (effects, stream_open_contexts) = world.detached_effect_records();
        let artifacts = world.committed_artifacts();
        let artifact_publications = world.artifact_publications();
        let output_checkpoint = self.checkpoint();
        crate::DetachedEngineCompletion::capture_borrowed_pages(
            effect_base,
            effects,
            stream_open_contexts,
            artifacts.to_vec(),
            artifact_publications,
            self.prepared_page_count,
            |visit| {
                self.pages
                    .visit_checkpoint_values(&self.pool, output_checkpoint.mark, visit)
                    .expect("checkpoint output visits its sealed accepted/current lineage");
            },
            None,
        )
    }

    pub fn commit_job_start<G>(
        &mut self,
        control: &mut MainControl<G>,
        universe: &mut Universe<G>,
        sink: &mut dyn CheckpointSink<G>,
    ) -> Result<bool, CommandSummaryError> {
        if std::mem::replace(&mut self.job_start_committed, true) {
            return Ok(false);
        }
        if !sink.wants_checkpoint(EngineBoundary::JobStart) {
            let _ = control.take_job_start_eligibility();
            return Ok(true);
        }
        let eligibility = control
            .take_job_start_eligibility()
            .ok_or(CommandSummaryError::AttemptSuspended)?;
        let wants_identity = sink.wants_reachable_state_identity(EngineBoundary::JobStart);
        self.publish(control, universe, sink, Some((eligibility, wants_identity)))?;
        Ok(true)
    }

    pub fn fulfill<G>(
        &mut self,
        control: &mut MainControl<G>,
        need: &ResourceNeed,
        fulfillment: ResourceFulfillment,
    ) -> Result<(), Box<ResourceFulfillment>> {
        self.fulfill_with_dependencies(control, need, fulfillment, None)
    }

    /// Installs a retained resource answer and carries the semantic input
    /// observations made while acquiring it into the capability binding.
    /// Those observations are re-recorded by the command/executor lookup when
    /// a later retry hits the retained answer directly.
    pub fn fulfill_with_effects<G>(
        &mut self,
        control: &mut MainControl<G>,
        need: &ResourceNeed,
        fulfillment: ResourceFulfillment,
        effects: &[crate::ResourceReplayEffect],
    ) -> Result<(), Box<ResourceFulfillment>> {
        let dependencies = (!effects.is_empty()).then(|| {
            effects
                .iter()
                .map(crate::ResourceReplayEffect::input_dependency)
                .collect()
        });
        self.fulfill_with_dependencies(control, need, fulfillment, dependencies)
    }

    fn fulfill_with_dependencies<G>(
        &mut self,
        control: &mut MainControl<G>,
        need: &ResourceNeed,
        fulfillment: ResourceFulfillment,
        dependencies: Option<Vec<tex_state::InputDependency>>,
    ) -> Result<(), Box<ResourceFulfillment>> {
        let dependencies = dependencies.map_or_else(|| Arc::from([]), Into::into);
        control
            .capabilities_mut()
            .install_resource_answer(need, fulfillment, dependencies)?;
        control.acknowledge_resource_need();
        Ok(())
    }

    pub fn mark_unavailable<G>(
        &mut self,
        control: &mut MainControl<G>,
        need: &ResourceNeed,
        register_texinputs_alias: bool,
    ) {
        self.mark_unavailable_with_dependencies(
            control,
            need,
            register_texinputs_alias,
            Vec::new(),
        );
    }

    /// Settles an authoritative absence while retaining the input facts that
    /// led to it for future cached probe/open hits.
    pub fn mark_unavailable_with_effects<G>(
        &mut self,
        control: &mut MainControl<G>,
        need: &ResourceNeed,
        register_texinputs_alias: bool,
        effects: &[crate::ResourceReplayEffect],
    ) {
        let dependencies = effects
            .iter()
            .map(crate::ResourceReplayEffect::input_dependency)
            .collect();
        self.mark_unavailable_with_dependencies(
            control,
            need,
            register_texinputs_alias,
            dependencies,
        );
    }

    fn mark_unavailable_with_dependencies<G>(
        &mut self,
        control: &mut MainControl<G>,
        need: &ResourceNeed,
        register_texinputs_alias: bool,
        dependencies: Vec<tex_state::InputDependency>,
    ) {
        control.capabilities_mut().install_resource_unavailable(
            need,
            register_texinputs_alias,
            dependencies,
        );
        control.acknowledge_resource_need();
    }

    fn publish<G>(
        &mut self,
        control: &mut MainControl<G>,
        universe: &mut Universe<G>,
        sink: &mut dyn CheckpointSink<G>,
        checkpoint: Option<(crate::checkpoint::CheckpointEligibility, bool)>,
    ) -> Result<(), CommandSummaryError> {
        self.collect_prepared_pages(control);
        if let Some((eligibility, wants_identity)) = checkpoint {
            let counters = ExecutionBudgetCounters::default();
            let mut checkpoint = control.capture_checkpoint_with_identity_demand(
                eligibility,
                universe,
                counters,
                wants_identity,
            )?;
            checkpoint.set_output_ledger(self.checkpoint());
            sink.checkpoint(checkpoint, universe);
            while let Some(release) = sink.take_checkpoint_release() {
                release.apply(control, universe);
            }
        }
        Ok(())
    }
}

impl Drop for OutputLedger {
    fn drop(&mut self) {
        self.reject_checkpoint_candidate();
    }
}

/// Borrow-scoped driver for one bounded canonical engine operation.
pub struct CanonicalStepRunner<'a, G> {
    control: &'a mut MainControl<G>,
    universe: &'a mut Universe<G>,
    ledger: &'a mut OutputLedger,
}

impl<'a, G> CanonicalStepRunner<'a, G> {
    pub fn new(
        control: &'a mut MainControl<G>,
        universe: &'a mut Universe<G>,
        ledger: &'a mut OutputLedger,
    ) -> Self {
        Self {
            control,
            universe,
            ledger,
        }
    }

    pub fn step(
        &mut self,
        sink: &mut dyn CheckpointSink<G>,
        cancellation: &Cancellation,
    ) -> CanonicalStepResult {
        let result = self.step_inner(sink, cancellation, None, None);
        if let Some(error) = self.control.captured_fatal_error() {
            CanonicalStepResult::Failed(CanonicalStepFailure::Execution(error))
        } else if let Some(fatal) = self.control.fatal_error() {
            CanonicalStepResult::Failed(CanonicalStepFailure::Execution(ExecError::Fatal(fatal)))
        } else {
            result
        }
    }

    /// Runs one canonical operation with the cold resource provider enabled.
    /// Ready and unavailable answers continue in this call; only a provider
    /// decline reaches the outer suspension seam.
    pub fn step_with_resource_provider(
        &mut self,
        sink: &mut dyn CheckpointSink<G>,
        cancellation: &Cancellation,
        resource_provider: &mut dyn ResourceProvider<G>,
    ) -> CanonicalStepResult {
        let result = self.step_inner(sink, cancellation, None, Some(resource_provider));
        if let Some(error) = self.control.captured_fatal_error() {
            CanonicalStepResult::Failed(CanonicalStepFailure::Execution(error))
        } else if let Some(fatal) = self.control.fatal_error() {
            CanonicalStepResult::Failed(CanonicalStepFailure::Execution(ExecError::Fatal(fatal)))
        } else {
            result
        }
    }

    /// Advances a complete-job session through TeX82 §81's `jump_out`.
    ///
    /// Diagnostic-oriented callers use [`Self::step`] so a captured fatal
    /// retains its source site. A retained complete-job owner instead has the
    /// frame corresponding to §1332's `end_of_TEX`; it converts that same
    /// fatal into terminal completion so §1333 cleanup can run.
    pub fn step_completing_fatal(
        &mut self,
        sink: &mut dyn CheckpointSink<G>,
        cancellation: &Cancellation,
    ) -> CanonicalStepResult {
        let result = self.step_inner(sink, cancellation, None, None);
        match result {
            CanonicalStepResult::Failed(CanonicalStepFailure::Execution(error)) => {
                if let Some(fatal) = error.as_fatal() {
                    let step = self.control.succumb(fatal);
                    self.control.mark_ended();
                    self.ledger.terminal_step = Some(step);
                    CanonicalStepResult::Completed(step)
                } else {
                    CanonicalStepResult::Failed(CanonicalStepFailure::Execution(error))
                }
            }
            result => result,
        }
    }

    /// Provider-aware counterpart to [`Self::step_completing_fatal`]. Ready
    /// and unavailable resources are resolved during the current cold call;
    /// a fatal raised after that resolution still follows TeX82 §81's
    /// complete-job cleanup path.
    pub fn step_completing_fatal_with_resource_provider(
        &mut self,
        sink: &mut dyn CheckpointSink<G>,
        cancellation: &Cancellation,
        resource_provider: &mut dyn ResourceProvider<G>,
    ) -> CanonicalStepResult {
        let result = self.step_inner(sink, cancellation, None, Some(resource_provider));
        match result {
            CanonicalStepResult::Failed(CanonicalStepFailure::Execution(error)) => {
                if let Some(fatal) = error.as_fatal() {
                    let step = self.control.succumb(fatal);
                    self.control.mark_ended();
                    self.ledger.terminal_step = Some(step);
                    CanonicalStepResult::Completed(step)
                } else {
                    CanonicalStepResult::Failed(CanonicalStepFailure::Execution(error))
                }
            }
            result => result,
        }
    }

    pub fn step_with_observer(
        &mut self,
        sink: &mut dyn CheckpointSink<G>,
        cancellation: &Cancellation,
        observer: &mut dyn CommandObserver,
    ) -> CanonicalStepResult {
        self.step_inner(sink, cancellation, Some(observer), None)
    }

    /// Observed canonical operation with the cold resource provider enabled.
    pub fn step_with_observer_and_resource_provider(
        &mut self,
        sink: &mut dyn CheckpointSink<G>,
        cancellation: &Cancellation,
        observer: &mut dyn CommandObserver,
        resource_provider: &mut dyn ResourceProvider<G>,
    ) -> CanonicalStepResult {
        self.step_inner(sink, cancellation, Some(observer), Some(resource_provider))
    }

    fn step_inner(
        &mut self,
        sink: &mut dyn CheckpointSink<G>,
        cancellation: &Cancellation,
        observer: Option<&mut dyn CommandObserver>,
        resource_provider: Option<&mut dyn ResourceProvider<G>>,
    ) -> CanonicalStepResult {
        if cancellation.is_cancelled() {
            self.control
                .record_external_episode_barrier(SemanticEpisodeBarrier::Cancellation);
            return CanonicalStepResult::Failed(CanonicalStepFailure::Execution(
                ExecError::ExecutionCancelled,
            ));
        }
        let wants_paragraph = sink.wants_checkpoint(EngineBoundary::OuterParagraphEnd);
        self.control.set_paragraph_checkpoint_demand(
            wants_paragraph
                .then(|| sink.wants_reachable_state_identity(EngineBoundary::OuterParagraphEnd)),
        );
        let result = match (observer, resource_provider) {
            (Some(observer), Some(resource_provider)) => {
                self.control.advance_with_observer_and_resource_provider(
                    self.universe,
                    observer,
                    resource_provider,
                )
            }
            (Some(observer), None) => self.control.advance_with_observer(self.universe, observer),
            (None, Some(resource_provider)) => self
                .control
                .advance_episode_with_resource_provider(self.universe, resource_provider),
            (None, None) => self.control.advance_episode(self.universe),
        };
        let step = match result {
            Ok(StepResult::Progress(step)) => step,
            Ok(StepResult::Suspended(need)) => {
                return self.control.take_declined_resource_attempt().map_or(
                    CanonicalStepResult::ResourceNeed(need),
                    CanonicalStepResult::ResourceSuspended,
                );
            }
            Err(error) => {
                return CanonicalStepResult::Failed(CanonicalStepFailure::Execution(error));
            }
        };
        let checkpoint = self.control.take_paragraph_checkpoint(self.universe);
        let committed = checkpoint.is_some();
        if let Err(error) = self
            .ledger
            .publish(self.control, self.universe, sink, checkpoint)
        {
            self.control
                .record_external_episode_barrier(SemanticEpisodeBarrier::Checkpoint);
            return CanonicalStepResult::Failed(CanonicalStepFailure::Checkpoint(error));
        }
        if matches!(step, MainControlStep::End | MainControlStep::EndOfInput) {
            self.control.mark_ended();
            self.ledger.terminal_step = Some(step);
            CanonicalStepResult::Completed(step)
        } else if committed {
            CanonicalStepResult::Committed(step)
        } else {
            CanonicalStepResult::Progress(step)
        }
    }
}
