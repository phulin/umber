use std::fmt;

use tex_command::{CommandObserver, CommandSummaryError, FontResource, PdfImageResource};
use tex_state::Universe;
use tex_state::fork_arena::{CheckpointMark, ChunkPool, ForkArena};

use crate::{
    Cancellation, CheckpointSink, ExecError, ExecutionBudgetCounters, MainControl, MainControlStep,
    ResourceFulfillment, ResourceNeed, SemanticEpisodeBarrier, StepResult,
    canonical_font_resource_path,
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
        let _ = builder
            .seal()
            .expect("prepared output page batch seals canonically");
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
        step: MainControlStep,
    ) -> Result<TerminalRevisionReceipt, crate::EngineCompletionError> {
        if self.terminal_closed
            || self.terminal_step != Some(step)
            || !control.terminal_revision_is_quiescent(step)
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
            || !control.terminal_revision_is_quiescent(receipt.step)
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
            || !control.terminal_revision_is_quiescent(receipt.step)
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
        control.close_terminal_revision(receipt.step);
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
        let eligibility = control
            .take_job_start_eligibility()
            .ok_or(CommandSummaryError::AttemptSuspended)?;
        self.publish(control, universe, sink, vec![eligibility])?;
        Ok(true)
    }

    pub fn fulfill<G>(
        &mut self,
        control: &mut MainControl<G>,
        need: &ResourceNeed,
        fulfillment: ResourceFulfillment,
    ) -> Result<(), Box<ResourceFulfillment>> {
        match (need, fulfillment) {
            (
                ResourceNeed::Input { name: expected, .. },
                ResourceFulfillment::Input { name, source },
            ) if expected == &name => control.capabilities_mut().register_input(name, source),
            (
                ResourceNeed::InputProbe { request: expected },
                ResourceFulfillment::InputProbe { request, resource },
            ) if expected == &request => control
                .capabilities_mut()
                .register_input_probe(request.name, resource),
            (
                ResourceNeed::Font { request: expected },
                ResourceFulfillment::Font { request, resource },
            ) if expected == &request => control
                .capabilities_mut()
                .register_font(canonical_font_resource_path(&request.name), *resource),
            (
                ResourceNeed::PdfImage { request: expected },
                ResourceFulfillment::PdfImage { request, resource },
            ) if expected == &request => control
                .capabilities_mut()
                .register_pdf_image(request, *resource),
            (_, fulfillment) => return Err(Box::new(fulfillment)),
        }
        control.acknowledge_resource_need();
        Ok(())
    }

    pub fn mark_unavailable<G>(
        &mut self,
        control: &mut MainControl<G>,
        need: &ResourceNeed,
        register_texinputs_alias: bool,
    ) {
        match need {
            ResourceNeed::Input { name, .. } => {
                let capabilities = control.capabilities_mut();
                capabilities.mark_input_unavailable(name);
                if register_texinputs_alias && !name.contains(['/', '\\', ':']) {
                    capabilities.mark_input_unavailable(format!("TeXinputs:{name}"));
                }
            }
            ResourceNeed::InputProbe { request } => control
                .capabilities_mut()
                .mark_input_probe_unavailable(&request.name),
            ResourceNeed::Font { request } => control.capabilities_mut().register_font(
                canonical_font_resource_path(&request.name),
                FontResource::Unavailable,
            ),
            ResourceNeed::PdfImage { request } => control
                .capabilities_mut()
                .register_pdf_image(request.clone(), PdfImageResource::Unavailable),
        }
        control.acknowledge_resource_need();
    }

    fn publish<G>(
        &mut self,
        control: &mut MainControl<G>,
        universe: &mut Universe<G>,
        sink: &mut dyn CheckpointSink<G>,
        eligibilities: Vec<crate::checkpoint::CheckpointEligibility>,
    ) -> Result<(), CommandSummaryError> {
        self.collect_prepared_pages(control);
        for eligibility in eligibilities {
            let boundary = eligibility.boundary();
            if !sink.wants_checkpoint(boundary) {
                continue;
            }
            let counters = ExecutionBudgetCounters::default();
            let mut checkpoint = control.capture_checkpoint_with_identity_demand(
                eligibility,
                universe,
                counters,
                sink.wants_reachable_state_identity(boundary),
            )?;
            checkpoint.set_output_ledger(self.checkpoint());
            sink.checkpoint(checkpoint);
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
        let result = self.step_inner(sink, cancellation, None);
        if let Some(error) = self.control.captured_fatal_error() {
            CanonicalStepResult::Failed(CanonicalStepFailure::Execution(error))
        } else if let Some(fatal) = self.control.fatal_error() {
            CanonicalStepResult::Failed(CanonicalStepFailure::Execution(ExecError::Fatal(fatal)))
        } else {
            result
        }
    }

    /// Publishes the quiescent named-boundary suffix left by terminal cleanup.
    ///
    /// A terminal format capture may first discard unread command-only input
    /// or macro replay levels. Its outer owner then calls this method before
    /// asking the ledger for the terminal receipt, so every newly quiescent
    /// boundary is checkpointed in canonical order.
    pub fn publish_terminal_boundary_suffix(
        &mut self,
        sink: &mut dyn CheckpointSink<G>,
    ) -> Result<(), CanonicalStepFailure> {
        self.control
            .publish_terminal_named_boundaries(self.universe)
            .map_err(CanonicalStepFailure::Execution)?;
        let _boundaries = self.control.take_completed_boundaries();
        let eligibilities = self.control.take_checkpoint_eligibilities();
        self.ledger
            .publish(self.control, self.universe, sink, eligibilities)
            .map_err(CanonicalStepFailure::Checkpoint)
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
        let result = self.step_inner(sink, cancellation, None);
        match result {
            CanonicalStepResult::Failed(CanonicalStepFailure::Execution(error)) => {
                if let Some(fatal) = error.as_fatal() {
                    let step = self.control.succumb(fatal);
                    self.control.arm_terminal_revision(step);
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
        self.step_inner(sink, cancellation, Some(observer))
    }

    fn step_inner(
        &mut self,
        sink: &mut dyn CheckpointSink<G>,
        cancellation: &Cancellation,
        observer: Option<&mut dyn CommandObserver>,
    ) -> CanonicalStepResult {
        if cancellation.is_cancelled() {
            self.control
                .record_external_episode_barrier(SemanticEpisodeBarrier::Cancellation);
            return CanonicalStepResult::Failed(CanonicalStepFailure::Execution(
                ExecError::ExecutionCancelled,
            ));
        }
        let result = match observer {
            Some(observer) => self.control.advance_with_observer(self.universe, observer),
            None => self.control.advance_episode(self.universe),
        };
        let step = match result {
            Ok(StepResult::Progress(step)) => step,
            Ok(StepResult::Suspended(need)) => {
                return CanonicalStepResult::ResourceNeed(need);
            }
            Err(error) => {
                return CanonicalStepResult::Failed(CanonicalStepFailure::Execution(error));
            }
        };
        if matches!(step, MainControlStep::End | MainControlStep::EndOfInput)
            && let Err(error) = self
                .control
                .publish_terminal_named_boundaries(self.universe)
        {
            return CanonicalStepResult::Failed(CanonicalStepFailure::Execution(error));
        }
        let boundaries = self.control.take_completed_boundaries().into_boxed_slice();
        let eligibilities = self.control.take_checkpoint_eligibilities();
        if let Err(error) = self
            .ledger
            .publish(self.control, self.universe, sink, eligibilities)
        {
            self.control
                .record_external_episode_barrier(SemanticEpisodeBarrier::Checkpoint);
            return CanonicalStepResult::Failed(CanonicalStepFailure::Checkpoint(error));
        }
        if matches!(step, MainControlStep::End | MainControlStep::EndOfInput) {
            self.control.arm_terminal_revision(step);
            self.ledger.terminal_step = Some(step);
            CanonicalStepResult::Completed(step)
        } else if boundaries.is_empty() {
            CanonicalStepResult::Progress(step)
        } else {
            CanonicalStepResult::Committed(step)
        }
    }
}
