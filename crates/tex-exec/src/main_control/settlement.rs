//! Direct-operation settlement evidence and publication ownership.

use super::*;

/// Where one command-processor episode publishes its committed records.
///
/// An episode with no observer carries `None`. The slot is still a parameter
/// of [`command_processor`] so that no episode can be constructed without
/// stating which commit buffer it belongs to.
pub(super) type ObservationSlot = Option<ObservationBuffer>;

#[derive(Clone, Copy)]
pub(super) struct OperationOutputStart {
    pub(super) outer_paragraph_was_active: bool,
    pub(super) source_role: Option<tex_command::SourceRole>,
    pub(super) artifact_count: usize,
    pub(super) effect_count: usize,
    pub(super) prepared_page_count: usize,
}

pub(super) struct DirectFailureContext {
    pub(super) operations: usize,
    pub(super) initial_artifacts: usize,
    pub(super) initial_boundaries: usize,
    pub(super) initial_effect_pos: tex_state::EffectPos,
}

/// Fixed-size rollback coordinates for one direct command operation.
///
/// Mode roots are restored first; only then may the attempt and page-arena
/// suffixes be truncated.
#[derive(Debug)]
pub(super) struct DirectOperationMark<G> {
    pub(super) state: tex_state::StateOperation<G>,
    pub(super) mode: crate::mode::ModeJournalCursor,
    pub(super) attempt: tex_command::CommandAttemptOperation,
    pub(super) page: tex_state::fork_arena::OperationMark<tex_state::fork_arena::PageMaterialLane>,
    pub(super) active_box_len: usize,
}

#[derive(Clone, Copy)]
pub(super) enum OperationTransaction {
    Advance,
    Alignment,
}

impl<G> MainControl<G> {
    pub(super) fn capture_first_causal_context(
        &mut self,
        stores: &mut Universe<G>,
        diagnostics: &[PendingDiagnostic<G>],
    ) {
        if self.first_causal_context.is_none()
            && let Some(cause_kind) = diagnostics.iter().find_map(PendingDiagnostic::causal_kind)
        {
            let context = stores.command_context().expect("diagnostic admission");
            self.first_causal_context = Some(crate::FrozenDiagnosticContext::capture(
                &context,
                self.command.diagnostic_input_context(8),
                cause_kind,
            ));
        }
    }

    pub(super) fn capture_first_reported_command_error_context(
        &mut self,
        stores: &mut Universe<G>,
    ) {
        if self.first_causal_context.is_none() && stores.world().error_channel().error_count() > 0 {
            let context = stores.command_context().expect("live generation");
            self.first_causal_context = Some(crate::FrozenDiagnosticContext::capture(
                &context,
                self.command.diagnostic_input_context(8),
                "command-error",
            ));
        }
    }

    pub(super) fn operation_evidence_limit_error(&self) -> Option<ExecError> {
        self.operation_observations
            .as_ref()
            .and_then(ObservationBuffer::limit_error)
            .or_else(|| self.page_output_observations.limit_error())
    }

    /// Closes every live receipt category and performs the append-bound check
    /// that must precede any operation commit.
    pub(super) fn admit_observed_receipt(
        &mut self,
        stores: &Universe<G>,
        termination: OperationTermination,
    ) -> Option<ExecError> {
        let (Some(start), Some(pending)) = (
            self.operation_receipt_start,
            self.operation_observations.as_mut(),
        ) else {
            return self.operation_evidence_limit_error();
        };
        let live_effects = stores.world().effect_records();
        let effect_base = stores
            .world()
            .effect_pos()
            .raw()
            .saturating_sub(live_effects.len().try_into().unwrap_or(u64::MAX));
        let effect_start = start
            .effect
            .saturating_sub(effect_base)
            .try_into()
            .unwrap_or(usize::MAX)
            .min(live_effects.len());
        for effect in &live_effects[effect_start..] {
            pending.record_world_effect(effect.clone());
        }
        for artifact in &stores.world().artifact_commits()[start.artifact..] {
            pending.record_artifact(*artifact);
        }
        pending.receipt.set_termination(termination);
        self.operation_evidence_limit_error()
    }
}

impl<G> MainControl<G> {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn episode_commit_boundary(
        &self,
        stores: &Universe<G>,
        applied: &Result<ReplayStep, ExecError>,
        operations: usize,
        max_operations: usize,
        initial_boundaries: usize,
        initial_effect_pos: tex_state::EffectPos,
        initial_artifacts: usize,
        initial_format_dump: bool,
        initial_diagnostic: bool,
        initial_error_count: i32,
        tracked: bool,
    ) -> Option<crate::EpisodeCommitBoundary> {
        #[cfg(feature = "profiling")]
        tex_state::measurement::record_hot_core_phase(
            tex_state::measurement::HotCorePhase::BarrierDecision,
        );
        if applied.is_err() {
            return None;
        }
        if self.dumped_format.is_some() != initial_format_dump {
            return Some(crate::EpisodeCommitBoundary::Semantic(
                crate::SemanticEpisodeBarrier::Format,
            ));
        }
        if self.fatal.is_some() {
            return Some(crate::EpisodeCommitBoundary::Semantic(
                crate::SemanticEpisodeBarrier::Diagnostic,
            ));
        }
        if self.first_causal_context.is_some() != initial_diagnostic
            || stores.world().error_channel().error_count() != initial_error_count
        {
            return Some(crate::EpisodeCommitBoundary::Semantic(
                crate::SemanticEpisodeBarrier::Diagnostic,
            ));
        }
        if matches!(applied, Ok(ReplayStep::End | ReplayStep::EndOfInput)) {
            return Some(crate::EpisodeCommitBoundary::Terminal);
        }
        if stores.world().artifact_commits().len() != initial_artifacts {
            return Some(crate::EpisodeCommitBoundary::Semantic(
                crate::SemanticEpisodeBarrier::Output,
            ));
        }
        if self.completed_boundaries.len() != initial_boundaries {
            let boundary = self.completed_boundaries[initial_boundaries];
            return Some(crate::EpisodeCommitBoundary::NamedCheckpoint(boundary));
        }
        if stores.world().effect_pos() != initial_effect_pos {
            return Some(crate::EpisodeCommitBoundary::Semantic(
                crate::SemanticEpisodeBarrier::Effect,
            ));
        }
        if self.operation_observations.is_some() {
            return Some(crate::EpisodeCommitBoundary::Semantic(
                crate::SemanticEpisodeBarrier::Observer,
            ));
        }
        if tracked {
            return Some(crate::EpisodeCommitBoundary::Semantic(
                crate::SemanticEpisodeBarrier::StateIdentity,
            ));
        }
        (operations >= max_operations).then_some(crate::EpisodeCommitBoundary::SliceLimit)
    }
}

impl<G> MainControl<G> {
    pub(super) fn record_direct_episode_commit(
        &mut self,
        stores: &mut Universe<G>,
        operations: usize,
        boundary: crate::EpisodeCommitBoundary,
        initial_artifacts: usize,
        initial_boundaries: usize,
        initial_effect_pos: tex_state::EffectPos,
    ) {
        self.episode_telemetry
            .record_commit(crate::EpisodeCommit::new(
                operations
                    .try_into()
                    .expect("bounded episode operation count fits u16"),
                boundary,
            ));
        if stores.world().artifact_commits().len() != initial_artifacts
            && boundary
                != crate::EpisodeCommitBoundary::Semantic(crate::SemanticEpisodeBarrier::Output)
        {
            self.episode_telemetry
                .record_semantic_barrier(crate::SemanticEpisodeBarrier::Output);
        }
        if self.completed_boundaries.len() != initial_boundaries
            && !matches!(boundary, crate::EpisodeCommitBoundary::NamedCheckpoint(_))
        {
            self.episode_telemetry
                .record_semantic_barrier(crate::SemanticEpisodeBarrier::Checkpoint);
        }
        if stores.world().effect_pos() != initial_effect_pos
            && boundary
                != crate::EpisodeCommitBoundary::Semantic(crate::SemanticEpisodeBarrier::Effect)
        {
            self.episode_telemetry
                .record_semantic_barrier(crate::SemanticEpisodeBarrier::Effect);
        }
        self.advance_telemetry.commits += 1;
    }

    pub(super) fn begin_direct_operation(
        &mut self,
        stores: &mut Universe<G>,
        attempt: Option<tex_command::CommandAttemptOperation>,
    ) -> DirectOperationMark<G> {
        DirectOperationMark {
            state: stores
                .begin_state_operation()
                .expect("live generation has a state operation journal"),
            mode: self.modes.begin_journal(),
            attempt: attempt.unwrap_or_else(|| self.command.begin_attempt_operation()),
            page: stores.page_node_cursor(),
            active_box_len: self.boxes.active_boxes.len(),
        }
    }

    pub(super) fn commit_direct_operation(
        &mut self,
        stores: &mut Universe<G>,
        mark: DirectOperationMark<G>,
    ) {
        let DirectOperationMark {
            state,
            mode,
            attempt,
            ..
        } = mark;
        stores
            .commit_state_operation(state)
            .expect("direct operation owns the active state operation");
        self.modes
            .commit_journal(mode)
            .expect("direct operation owns the top mode journal frame");
        self.command
            .commit_attempt_operation(attempt)
            .expect("committed operation owns a valid command-attempt scope");
        self.finish_pending_page_region_succession(stores);
    }

    pub(super) fn retain_direct_operation_for_retry(
        &mut self,
        stores: &mut Universe<G>,
        mark: DirectOperationMark<G>,
    ) -> tex_command::CommandAttemptOperation {
        let DirectOperationMark {
            state,
            mode,
            attempt,
            ..
        } = mark;
        stores
            .commit_state_operation(state)
            .expect("retained operation owns the active state operation");
        self.modes
            .commit_journal(mode)
            .expect("direct operation owns the top mode journal frame");
        attempt
    }

    pub(super) fn retain_direct_delivery_for_retry(
        &mut self,
        stores: &mut Universe<G>,
        mark: DirectOperationMark<G>,
        destination: PendingDirectDestination<G>,
    ) {
        let operation = self.retain_direct_operation_for_retry(stores, mark);
        assert!(
            self.pending_direct_operation
                .replace(PendingDirectOperation {
                    state: PendingDirectState::Retained(operation),
                    destination,
                })
                .is_none(),
            "one direct retry owns the active operation"
        );
    }

    pub(super) fn suspend_prepared_resource_operation(
        &mut self,
        stores: &Universe<G>,
        operation: tex_command::CommandAttemptOperation,
        frame: CommandEpisode<G>,
        cold: ColdOperationSlot<G>,
        capabilities: crate::transaction_protocol::CommandCapabilities,
    ) {
        let pending = SuspendedResourceResume::<G> {
            frame: OperationFrame::new(frame, cold),
            capabilities,
        };
        let attempt = self
            .command
            .suspend_attempt(stores, operation, SUSPENDED_RESOURCE_RESUME, pending)
            .expect("live main control can retain its admitted generation");
        self.pending_resource_operation = Some(PendingResourceOperation::<G> { attempt });
    }

    /// Finishes a failed prepared-resource preflight while the operation
    /// capability still has exactly one structural location.
    ///
    /// Diagnostic classification runs before suspension moves the attempt out
    /// of command state. A genuine resource suspension then moves it into the
    /// pending continuation and retains the other journals; every terminal
    /// result commits while the owner is still installed. Callers therefore
    /// cannot commit an emptied command attempt after moving its owner.
    pub(super) fn finish_unavailable_prepared_resource_operation(
        &mut self,
        stores: &mut Universe<G>,
        mark: DirectOperationMark<G>,
        mut frame: CommandEpisode<G>,
        cold: ColdOperationSlot<G>,
        capabilities: crate::transaction_protocol::CommandCapabilities,
    ) -> Result<StepResult, ExecError> {
        assert!(
            frame.has_unavailable(&cold),
            "unavailable resource remains in its attempt-owned frame"
        );
        let error = frame.take_error();
        let result = self.finish_resource_preflight_failure(stores, error);
        if matches!(result, Ok(StepResult::Suspended(_))) {
            let operation = self.retain_direct_operation_for_retry(stores, mark);
            self.suspend_prepared_resource_operation(stores, operation, frame, cold, capabilities);
        } else {
            self.commit_direct_operation(stores, mark);
        }
        result
    }

    pub(super) fn discard_direct_operation(
        &mut self,
        stores: &mut Universe<G>,
        mark: DirectOperationMark<G>,
    ) {
        stores
            .restore_state(mark.state)
            .expect("direct operation state cursor belongs to the live generation");
        self.modes
            .rollback_journal(mark.mode)
            .expect("direct operation owns the top mode journal frame");
        // ReplayBoxes is deliberately outside Universe and ModeNest. Any box
        // construction opened inside the rejected operation belongs to the
        // same retryable suffix as those transactional roots, so discard its
        // move-only owner before replay. BoxEndGroup retains pre-existing
        // owners until its own fallible completion has succeeded, hence a
        // rollback never needs to recreate an entry below this cursor.
        self.boxes.active_boxes.truncate(mark.active_box_len);
        self.command
            .rollback_attempt_operation(mark.attempt)
            .expect("rollback owns valid command-attempt coordinates");
        stores
            .truncate_page_nodes(mark.page)
            .expect("direct operation page cursor belongs to the live page arena");
    }

    pub(super) fn finish_direct_failure(
        &mut self,
        stores: &mut Universe<G>,
        operation_mark: DirectOperationMark<G>,
        error: ExecError,
        context: DirectFailureContext,
        diagnostic_effects: DiagnosticEffects,
    ) -> Result<StepResult, ExecError> {
        let DirectFailureContext {
            operations,
            initial_artifacts,
            initial_boundaries,
            initial_effect_pos,
        } = context;
        let error = {
            let mut stores = stores.command_context().expect("live generation");
            error.freeze_diagnostic_origin(&mut stores, self.command.diagnostic_input_context(8))
        };
        let Some(fatal) = error.as_fatal() else {
            self.discard_direct_operation(stores, operation_mark);
            return Err(error);
        };
        let context = self
            .command
            .output_open_context(&stores.command_context().expect("live generation"));
        crate::diagnostics::report_irrecoverable_error(stores, fatal, context);
        self.captured_fatal_origin = match &error {
            ExecError::Captured { site, frozen, .. } if fatal != FatalError::TooManyErrors => {
                Some((
                    *site,
                    frozen
                        .as_deref()
                        .and_then(|evidence| evidence.origin.clone()),
                    self.first_causal_context.clone().or_else(|| {
                        frozen
                            .as_deref()
                            .and_then(|evidence| evidence.context.clone())
                    }),
                ))
            }
            _ => None,
        };
        let mut records = Vec::with_capacity(3);
        if let Some(location) = self.command.last_diagnostic_location() {
            records.push(CommandObservation::DiagnosticLifecycle(
                tex_command::DiagnosticLifecycleRecord::Report {
                    class: tex_command::DiagnosticClass::Fatal,
                    severity: "fatal",
                    diagnostic: fatal.diagnostic(),
                    arguments: fatal.record().arguments,
                    location,
                },
            ));
        }
        records.push(CommandObservation::Diagnostic(fatal.record()));
        records.push(CommandObservation::Effect(engine_termination_effect()));
        self.observe_committed(records);
        stores
            .world_mut()
            .publish_diagnostic_effects(diagnostic_effects);
        let evidence_error =
            self.admit_observed_receipt(stores, OperationTermination::Fatal(fatal));
        self.commit_direct_operation(stores, operation_mark);
        self.record_direct_episode_commit(
            stores,
            operations,
            crate::EpisodeCommitBoundary::Semantic(crate::SemanticEpisodeBarrier::Diagnostic),
            initial_artifacts,
            initial_boundaries,
            initial_effect_pos,
        );
        let terminal = self.succumb(fatal);
        evidence_error.map_or(Ok(StepResult::Progress(terminal)), Err)
    }
}

impl<G> MainControl<G> {
    pub(super) fn finish_host_owned_step(
        &mut self,
        applied: Result<ReplayStep, ExecError>,
        output_start: OperationOutputStart,
        stores: &mut Universe<G>,
        diagnostic_effects: &mut DiagnosticEffects,
    ) -> Result<ReplayStep, ExecError> {
        let applied = match applied {
            Ok(applied) => applied,
            Err(error) => {
                self.page_output_observations.clear();
                return Err(error);
            }
        };
        // A host-owned application can itself run `fire_pending_page_output`
        // before reaching this common tail. Retain that already-opened
        // episode here instead of asking only whether another fire-up is
        // currently pending; otherwise its observations survive into the
        // next command step and that command's raw delivery overtakes them.
        let pending_page_output =
            PendingPageOutputFacts::capture(&stores.command_context().expect("live generation"));
        let opens_output_batch = !self.page_output_observations.is_empty()
            || (pending_page_output.fire_up.is_some() && !self.boxes.output_routine_active);
        self.fire_pending_page_output(stores, diagnostic_effects, pending_page_output)?;
        {
            #[cfg(feature = "profiling")]
            tex_state::measurement::record_hot_core_phase(
                tex_state::measurement::HotCorePhase::EvidencePublication,
            );
            #[cfg(feature = "profiling")]
            let _allocation_scope = tex_state::measurement::hot_core_allocation_scope(
                tex_state::measurement::HotCoreAllocationOwner::EvidencePublication,
            );
            // Host-owned transitions are still complete main-control steps.
            // In particular, §1145's display-math `init_math` installs
            // `every_display` here, and §323 traces that list before the next
            // command is fetched. Leaving the push queued until an ordinary
            // step reverses those events: the hook's final command is traced
            // and executed before its own `begin_token_list` trace.
            publish_named_token_list_pushes(
                &mut self.command,
                &mut stores.command_context().expect("live generation"),
                diagnostic_effects,
                &mut self.operation_observations,
            );
            if opens_output_batch {
                let mut records = Vec::new();
                // Same order as the ordinary tail: the named token-list push
                // command state held across the transition, then the shipouts
                // it committed, then the episode's own records.
                records.extend(
                    committed_shipout_observations(output_start.artifact_count, stores)
                        .into_iter()
                        .map(CommandObservation::Effect),
                );
                records.extend(
                    committed_stream_effect_observations(
                        output_start.effect_count,
                        output_start.prepared_page_count,
                        stores,
                        &self.prepared_dvi_pages,
                    )
                    .into_iter()
                    .map(CommandObservation::Effect),
                );
                self.page_output_observations.append_to(&mut records);
                self.observe_committed(records);
            }
            self.page_output_observations.clear();
        }
        self.finish_shipout_publication(
            output_start.artifact_count,
            output_start.effect_count,
            stores,
            output_start.source_role,
        );
        self.finish_paragraph_boundary(
            output_start.outer_paragraph_was_active,
            output_start.source_role,
            stores,
        );
        Ok(applied)
    }

    /// Publishes the ordinary cold paragraph boundary after `end_graf`.
    pub(super) fn finish_paragraph_boundary(
        &mut self,
        outer_paragraph_was_active: bool,
        source_role: Option<tex_command::SourceRole>,
        stores: &mut Universe<G>,
    ) {
        if outer_paragraph_was_active
            && self.modes.current_mode() == Mode::Vertical
            && self.modes.depth() == 1
            && stores
                .command_context()
                .expect("paragraph-boundary admission")
                .execution_group_depth()
                == 0
        {
            self.pending_named_boundaries
                .push_back(PendingNamedBoundary {
                    boundary: crate::EngineBoundary::OuterParagraphEnd,
                    source_role,
                });
        }
    }

    /// Publishes at most one queued named boundary after every command-owned
    /// continuation has retired. This runs before another delivery, so a
    /// captured row cannot include effects from the following command.
    pub(super) fn publish_pending_named_boundary(
        &mut self,
        stores: &mut Universe<G>,
    ) -> Result<Option<crate::EngineBoundary>, ExecError> {
        loop {
            let Some(pending) = self.pending_named_boundaries.front().copied() else {
                return Ok(None);
            };
            if self.has_external_attempt_owner() {
                return Ok(None);
            }
            if pending.boundary == crate::EngineBoundary::OuterParagraphEnd
                && !self.modes.restart_checkpoint_is_quiescent()
            {
                return Ok(None);
            }
            if pending.boundary == crate::EngineBoundary::ShipoutComplete
                && (self.boxes.output_routine_active
                    || self.modes.depth() != 1
                    || stores
                        .command_context()
                        .expect("shipout-boundary admission")
                        .execution_group_depth()
                        != 0)
            {
                return Ok(None);
            }
            let mut diagnostic_effects = DiagnosticEffects::new();
            let attempt = self.command.begin_attempt_operation();
            let retirement = {
                let mut context = stores.command_context().expect("named-boundary admission");
                let mut processor = command_processor(
                    &mut self.command,
                    self.fuel.fuel_mut(),
                    &mut self.capabilities,
                    &mut self.operation_observations,
                    &mut diagnostic_effects,
                    &mut context,
                );
                processor.retire_exhausted_token_levels_for_named_boundary()
            };
            if let Err(error) = retirement {
                self.command
                    .rollback_attempt_operation(attempt)
                    .expect("named-boundary retirement owns its attempt scope");
                return Err(command_error(error));
            }
            self.command
                .commit_attempt_operation(attempt)
                .map_err(|_| ExecError::MissingToken {
                    context: "named-boundary attempt scope",
                })?;
            stores
                .world_mut()
                .publish_diagnostic_effects(diagnostic_effects);
            if !self.command.named_boundary_is_quiescent() {
                return Ok(None);
            }
            let published = self
                .pending_named_boundaries
                .pop_front()
                .expect("inspected named-boundary intent remains queued");
            debug_assert_eq!(published, pending);
            if !checkpoint_role_is_retained(published.source_role)
                || self.restartable_root_source_identity().is_none()
            {
                continue;
            }
            if published.boundary == crate::EngineBoundary::ShipoutComplete {
                stores
                    .release_page_suffix_if_rootless(self.modes.retains_page_node_handles())
                    .map_err(|_| ExecError::MissingToken {
                        context: "rootless shipout page release",
                    })?;
            }
            self.completed_checkpoint_eligibilities.push(
                crate::checkpoint::CheckpointEligibility::named(published.boundary),
            );
            self.completed_boundaries.push(published.boundary);
            return Ok(Some(published.boundary));
        }
    }

    /// Publishes every named boundary that became quiescent during terminal
    /// cleanup. Ordinary execution publishes one intent before the following
    /// delivery; terminal cleanup has no following delivery, so the canonical
    /// runner must drain the safe suffix before closing its output ledger.
    pub(crate) fn publish_terminal_named_boundaries(
        &mut self,
        stores: &mut Universe<G>,
    ) -> Result<(), ExecError> {
        while !self.pending_named_boundaries.is_empty()
            && self.publish_pending_named_boundary(stores)?.is_some()
        {}
        Ok(())
    }
}

impl<G> MainControl<G> {
    pub(super) fn observe_committed(
        &mut self,
        records: impl IntoIterator<Item = CommandObservation>,
    ) {
        if let Some(buffer) = self.operation_observations.as_mut() {
            buffer.extend(records);
        }
    }
}

impl<G> MainControl<G> {
    pub(super) fn finish_shipout_publication(
        &mut self,
        artifact_count: usize,
        _effect_count: usize,
        stores: &mut Universe<G>,
        source_role: Option<tex_command::SourceRole>,
    ) {
        let committed = stores
            .world()
            .artifact_commits()
            .len()
            .saturating_sub(artifact_count);
        let intent = PendingNamedBoundary {
            boundary: crate::EngineBoundary::ShipoutComplete,
            source_role,
        };
        for _ in 0..committed {
            self.pending_named_boundaries.push_back(intent);
        }
    }
}

impl<G> MainControl<G> {
    pub(super) fn publish_pdf_fatal_error(
        stores: &mut Universe<G>,
        error: &ExecError,
    ) -> Result<(), ExecError> {
        if error.is_pdftex_navigation_fatal() {
            crate::job::report_pdf_fatal_error(stores, &error.to_string());
            stores.publish_effect_prefix(stores.world().effect_pos())?;
        }
        Ok(())
    }

    pub(super) fn finish_resource_preflight_failure(
        &mut self,
        stores: &mut Universe<G>,
        error: ExecError,
    ) -> Result<StepResult, ExecError> {
        let error = {
            let mut context = stores.command_context().expect("live generation");
            error.freeze_diagnostic_origin(&mut context, self.command.diagnostic_input_context(8))
        };
        if let Some(fatal) = error.as_fatal() {
            let context = self
                .command
                .output_open_context(&stores.command_context().expect("live generation"));
            crate::diagnostics::report_irrecoverable_error(stores, fatal, context);
            self.captured_fatal_origin = match &error {
                ExecError::Captured { site, frozen, .. } if fatal != FatalError::TooManyErrors => {
                    Some((
                        *site,
                        frozen
                            .as_deref()
                            .and_then(|evidence| evidence.origin.clone()),
                        self.first_causal_context.clone().or_else(|| {
                            frozen
                                .as_deref()
                                .and_then(|evidence| evidence.context.clone())
                        }),
                    ))
                }
                _ => None,
            };
            let mut records = Vec::with_capacity(3);
            if let Some(location) = self.command.last_diagnostic_location() {
                records.push(CommandObservation::DiagnosticLifecycle(
                    tex_command::DiagnosticLifecycleRecord::Report {
                        class: tex_command::DiagnosticClass::Fatal,
                        severity: "fatal",
                        diagnostic: fatal.diagnostic(),
                        arguments: fatal.record().arguments,
                        location,
                    },
                ));
            }
            records.push(CommandObservation::Diagnostic(fatal.record()));
            records.push(CommandObservation::Effect(engine_termination_effect()));
            self.observe_committed(records);
            let evidence_error =
                self.admit_observed_receipt(stores, OperationTermination::Fatal(fatal));
            let terminal = self.succumb(fatal);
            return evidence_error.map_or(Ok(StepResult::Progress(terminal)), Err);
        }
        match error {
            ExecError::Captured {
                error,
                site,
                frozen,
            } => match *error {
                ExecError::MissingInput {
                    name,
                    original_name,
                } => {
                    self.pending_resource_site = site.primary_origin();
                    Ok(self.observed_suspension(ResourceNeed::Input {
                        name,
                        original_name,
                    }))
                }
                ExecError::MissingInputProbe { request } => {
                    self.pending_resource_site = site.primary_origin();
                    Ok(self.observed_suspension(ResourceNeed::InputProbe { request }))
                }
                ExecError::MissingFont { request } => {
                    self.pending_resource_site = site.primary_origin();
                    Ok(self.observed_suspension(ResourceNeed::Font { request }))
                }
                ExecError::MissingPdfImage { request } => {
                    self.pending_resource_site = site.primary_origin();
                    Ok(self.observed_suspension(ResourceNeed::PdfImage { request }))
                }
                error => Err(ExecError::Captured {
                    error: Box::new(error),
                    site,
                    frozen,
                }),
            },
            ExecError::MissingInput {
                name,
                original_name,
            } => Ok(self.observed_suspension(ResourceNeed::Input {
                name,
                original_name,
            })),
            ExecError::MissingInputProbe { request } => {
                Ok(self.observed_suspension(ResourceNeed::InputProbe { request }))
            }
            ExecError::MissingFont { request } => {
                Ok(self.observed_suspension(ResourceNeed::Font { request }))
            }
            ExecError::MissingPdfImage { request } => {
                Ok(self.observed_suspension(ResourceNeed::PdfImage { request }))
            }
            error => Err(error),
        }
    }

    pub(super) fn observed_suspension(&mut self, need: ResourceNeed) -> StepResult {
        if let Some(pending) = self.operation_observations.as_mut() {
            pending.record_resource(need.clone());
            pending
                .receipt
                .set_termination(OperationTermination::Suspended);
        }
        StepResult::Suspended(need)
    }
}

pub(super) const MAX_OPERATION_EVIDENCE_RECORDS: usize = 1_000_000;

#[derive(Debug)]
pub(super) struct ObservationBuffer {
    pub(super) records: Vec<CommandObservation>,
    pub(super) attempted: usize,
    pub(super) overflowed: bool,
    pub(super) receipt_attempted: usize,
    pub(super) receipt_overflowed: bool,
    pub(super) receipt: ExecutionReceipt,
}

#[derive(Clone, Copy, Debug)]
pub(super) struct OperationReceiptStart {
    pub(super) effect: u64,
    pub(super) artifact: usize,
}

/// One checkpoint intent frozen at the operation that formed its boundary.
///
/// Input retirement may expose an enclosing source before command state is
/// quiescent enough to publish a checkpoint. Retaining the active external
/// file decision here prevents that later stack transition from changing the
/// boundary's origin eligibility.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct PendingNamedBoundary {
    pub(super) boundary: crate::EngineBoundary,
    pub(super) source_role: Option<tex_command::SourceRole>,
}

const fn checkpoint_role_is_retained(role: Option<tex_command::SourceRole>) -> bool {
    matches!(
        role,
        Some(tex_command::SourceRole::RootDocument | tex_command::SourceRole::UserDocumentInclude)
    )
}

/// Explicit live-observer boundary for detached shipout geometry.
pub(super) struct MainControlShipoutGeometrySink<'a, G> {
    pub(super) command: &'a PersistentInterpreter<G>,
    pub(super) observations: &'a mut ObservationSlot,
}

/// Explicit operation-local boundary for TeX82 §§649--676 packing geometry.
///
/// The command cursor supplies detached source coordinates while the
/// observation slot remains the sole rollback/publication owner. The sink
/// cannot outlive this command operation and owns no engine-state handle.
pub(super) struct MainControlPackGeometrySink<'a> {
    pub(super) line: u32,
    pub(super) source: Option<tex_command::SourceId>,
    pub(super) observations: &'a mut ObservationSlot,
}

pub(super) fn pack_geometry_sink<'a, G>(
    command: &PersistentInterpreter<G>,
    observations: &'a mut ObservationSlot,
) -> MainControlPackGeometrySink<'a> {
    MainControlPackGeometrySink {
        line: command.current_file_line_number(),
        source: command.current_file_source_id(),
        observations,
    }
}

impl crate::geometry::PackGeometrySink for MainControlPackGeometrySink<'_> {
    fn committed_hpack(&mut self, width: Scaled, height: Scaled, depth: Scaled) {
        let Some(observations) = self.observations.as_mut() else {
            return;
        };
        observations.committed(CommandObservation::Geometry(GeometryRecord::Hpack {
            width_sp: i64::from(width.raw()),
            height_sp: i64::from(height.raw()),
            depth_sp: i64::from(depth.raw()),
            line: self.line,
            source: self.source,
        }));
    }

    fn committed_vpack(&mut self, width: Scaled, height: Scaled, depth: Scaled) {
        let Some(observations) = self.observations.as_mut() else {
            return;
        };
        observations.committed(CommandObservation::Geometry(GeometryRecord::Vpack {
            width_sp: i64::from(width.raw()),
            height_sp: i64::from(height.raw()),
            depth_sp: i64::from(depth.raw()),
            line: self.line,
            source: self.source,
        }));
    }
}

impl<G> crate::shipout::ShipoutGeometrySink for MainControlShipoutGeometrySink<'_, G> {
    fn committed_shipout_geometry(&mut self, geometry: crate::shipout::ShipoutGeometry) {
        let Some(observations) = self.observations.as_mut() else {
            return;
        };
        observations.committed(CommandObservation::Geometry(GeometryRecord::Shipout {
            page_width_sp: geometry.page_width_sp,
            page_height_sp: geometry.page_height_sp,
            counts: geometry.counts,
            line: self.command.current_file_line_number(),
            source: self.command.current_file_source_id(),
        }));
    }
}

impl Default for ObservationBuffer {
    fn default() -> Self {
        Self {
            records: Vec::new(),
            attempted: 0,
            overflowed: false,
            receipt_attempted: 1,
            receipt_overflowed: false,
            receipt: ExecutionReceipt::default(),
        }
    }
}

impl ObservationBuffer {
    pub(super) fn consume_into(
        self,
        observer: Option<&mut dyn CommandObserver>,
    ) -> ConsumedExecutionReceipt {
        if let Some(observer) = observer {
            for observation in self.records {
                observer.committed(observation);
            }
        }
        self.receipt.consume()
    }

    pub(super) fn extend(&mut self, records: impl IntoIterator<Item = CommandObservation>) {
        for record in records {
            self.committed(record);
        }
    }

    pub(super) fn append(&mut self, other: &mut Self) {
        let omitted = other.attempted.saturating_sub(other.records.len());
        self.overflowed |= other.overflowed;
        self.receipt_overflowed |= other.receipt_overflowed;
        for record in other.records.drain(..) {
            self.committed(record);
        }
        let consumed = other.receipt.reset_for_next_operation();
        debug_assert!(consumed.records <= MAX_EXECUTION_RECEIPT_RECORDS);
        self.attempted = self.attempted.saturating_add(omitted);
        self.receipt_attempted = self.receipt_attempted.saturating_add(
            other
                .receipt_attempted
                .saturating_sub(other.receipt.record_count()),
        );
        self.overflowed |= self.attempted > MAX_OPERATION_EVIDENCE_RECORDS;
        self.receipt_overflowed |= self.receipt_attempted > MAX_EXECUTION_RECEIPT_RECORDS;
    }

    pub(super) fn append_to(&mut self, records: &mut Vec<CommandObservation>) {
        records.append(&mut self.records);
        let consumed = self.receipt.reset_for_next_operation();
        debug_assert!(consumed.records <= MAX_EXECUTION_RECEIPT_RECORDS);
        self.attempted = 0;
        self.overflowed = false;
        self.receipt_attempted = 1;
        self.receipt_overflowed = false;
    }

    pub(super) fn clear(&mut self) {
        self.records.clear();
        let consumed = self.receipt.reset_for_next_operation();
        debug_assert!(consumed.records <= MAX_EXECUTION_RECEIPT_RECORDS);
        self.attempted = 0;
        self.overflowed = false;
        self.receipt_attempted = 1;
        self.receipt_overflowed = false;
    }

    pub(super) fn is_empty(&self) -> bool {
        self.records.is_empty()
    }

    pub(super) fn limit_error(&self) -> Option<ExecError> {
        if self.overflowed {
            Some(ExecError::ResourceBudgetExceeded {
                resource: "operation evidence records",
                limit: MAX_OPERATION_EVIDENCE_RECORDS as u64,
                attempted: self.attempted.try_into().unwrap_or(u64::MAX),
            })
        } else if self.receipt_overflowed {
            Some(ExecError::ResourceBudgetExceeded {
                resource: "operation receipt records",
                limit: self.receipt.limit().try_into().unwrap_or(u64::MAX),
                attempted: self.receipt_attempted.try_into().unwrap_or(u64::MAX),
            })
        } else {
            None
        }
    }

    pub(super) fn record_receipt(&mut self, append: impl FnOnce(&mut ExecutionReceipt) -> bool) {
        self.receipt_attempted = self.receipt_attempted.saturating_add(1);
        if !append(&mut self.receipt) {
            self.receipt_overflowed = true;
        }
    }

    pub(super) fn record_world_effect(&mut self, effect: tex_state::EffectRecord) {
        self.record_receipt(|receipt| receipt.record_world_effect(effect));
    }

    pub(super) fn record_artifact(&mut self, artifact: tex_state::ContentHash) {
        self.record_receipt(|receipt| receipt.record_artifact(artifact));
    }

    pub(super) fn record_resource(&mut self, resource: ResourceNeed) {
        self.record_receipt(|receipt| receipt.record_resource(resource));
    }
}

impl CommandObserver for ObservationBuffer {
    fn committed(&mut self, observation: CommandObservation) {
        self.attempted = self.attempted.saturating_add(1);
        if self.records.len() < MAX_OPERATION_EVIDENCE_RECORDS {
            if matches!(
                observation,
                CommandObservation::Mutation(_)
                    | CommandObservation::Diagnostic(_)
                    | CommandObservation::Effect(_)
            ) {
                self.receipt_attempted = self.receipt_attempted.saturating_add(1);
                if !self.receipt.capture_observation(&observation) {
                    self.receipt_overflowed = true;
                }
            }
            self.records.push(observation);
        } else {
            self.overflowed = true;
        }
    }
}
