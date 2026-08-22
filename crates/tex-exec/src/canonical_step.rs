use std::fmt;

use tex_command::{CommandObserver, CommandSummaryError, FontResource, PdfImageResource};
use tex_state::Universe;

use crate::{
    Cancellation, CheckpointSink, EngineBoundary, ExecError, ExecutionBudgetCounters, MainControl,
    MainControlStep, ResourceFulfillment, ResourceNeed, SemanticEpisodeBarrier, StepResult,
    canonical_font_resource_path,
};

/// Checkpoint identity policy retained by one revision/output transaction.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum CheckpointIdentity {
    #[default]
    Snapshot,
    Exact,
}

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
#[derive(Debug, Default)]
pub struct OutputLedger {
    checkpoint_identity: CheckpointIdentity,
    job_start_committed: bool,
    suspension_serial: u64,
    terminal_step: Option<MainControlStep>,
    terminal_closed: bool,
}

impl OutputLedger {
    #[must_use]
    pub const fn new(checkpoint_identity: CheckpointIdentity) -> Self {
        Self {
            checkpoint_identity,
            job_start_committed: false,
            suspension_serial: 0,
            terminal_step: None,
            terminal_closed: false,
        }
    }

    /// Creates a ledger resumed from an already retained `JobStart` record.
    #[must_use]
    pub const fn resume(checkpoint_identity: CheckpointIdentity) -> Self {
        Self {
            checkpoint_identity,
            job_start_committed: true,
            suspension_serial: 0,
            terminal_step: None,
            terminal_closed: false,
        }
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

    /// Closes all executor-owned output ledgers after a terminal committed
    /// step. Suspension never calls this method and therefore cannot expose a
    /// partial revision patch.
    pub fn close_revision<G>(
        &mut self,
        control: &mut MainControl<G>,
        universe: &mut Universe<G>,
        receipt: &TerminalRevisionReceipt,
        demand: crate::EngineCompletionDemand,
    ) -> Result<crate::DetachedEngineCompletion, crate::EngineCompletionError> {
        if self.terminal_closed
            || self.terminal_step != Some(receipt.step)
            || self.suspension_serial != receipt.suspension_serial
            || !control.terminal_revision_is_quiescent(receipt.step)
        {
            return Err(crate::EngineCompletionError::TerminalRevisionUnavailable);
        }
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
        if world.effect_pos().raw()
            != u64::try_from(world.effect_records().len()).unwrap_or(u64::MAX)
        {
            return Err(crate::EngineCompletionError::MaterializedEffectBase);
        }
        let (effects, stream_open_contexts) = world.detached_effect_records();
        let completion = crate::DetachedEngineCompletion::capture(
            effects,
            stream_open_contexts,
            world.committed_artifacts().to_vec(),
            world.artifact_publications(),
            control.prepared_dvi_pages().to_vec(),
            pdf,
        )?;
        let _ = control.take_prepared_dvi_pages();
        self.terminal_closed = true;
        control.close_terminal_revision(receipt.step);
        Ok(completion)
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
        self.publish(control, universe, sink, &[EngineBoundary::JobStart])?;
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
        &self,
        control: &mut MainControl<G>,
        universe: &mut Universe<G>,
        sink: &mut dyn CheckpointSink<G>,
        boundaries: &[EngineBoundary],
    ) -> Result<(), CommandSummaryError> {
        for &boundary in boundaries {
            if !sink.wants_checkpoint(boundary) {
                continue;
            }
            let counters = ExecutionBudgetCounters::default();
            let checkpoint = match self.checkpoint_identity {
                CheckpointIdentity::Snapshot => {
                    control.capture_checkpoint(boundary, universe, counters)?
                }
                CheckpointIdentity::Exact => {
                    control.capture_checkpoint_with_exact_identity(boundary, universe, counters)?
                }
            };
            sink.checkpoint(checkpoint);
        }
        Ok(())
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
        let boundaries = self.control.take_completed_boundaries();
        self.ledger
            .publish(self.control, self.universe, sink, &boundaries)
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
        if let Err(error) = self
            .ledger
            .publish(self.control, self.universe, sink, &boundaries)
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
