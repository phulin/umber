//! Demanded diagnostics, evidence admission and bounded host-driver boundaries.

use super::*;

/// Where one command-processor episode publishes its committed records.
///
/// An episode with no observer carries `None`. The slot is still a parameter
/// of [`command_processor`] so that no episode can be constructed without
/// stating which commit buffer it belongs to.
pub(super) type ObservationSlot = Option<ObservationBuffer>;

#[derive(Clone, Copy)]
pub(super) struct OperationOutputStart {
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

impl<G> MainControl<G> {
    /// Promotes one operation-local recoverable report at its commit seam.
    ///
    /// The candidate is moved out of the detached collector when its
    /// operation commits, so a later operation can never replace it. A halted
    /// operation drops the collector with its unpromoted candidate.
    pub(super) fn promote_first_recoverable(
        &mut self,
        stores: &mut Universe<G>,
        diagnostic_effects: &mut DiagnosticEffects,
    ) {
        let Some(candidate) = diagnostic_effects.take_first_recoverable() else {
            return;
        };
        if self.execution.first_recoverable_diagnostic.is_some() {
            return;
        }

        let mut context = stores.command_context().expect("diagnostic admission");
        self.promote_recoverable_in_context(&mut context, candidate);
    }

    /// Completes promotion while the caller owns the admitted command context.
    fn promote_recoverable_in_context(
        &mut self,
        context: &mut tex_state::CommandContext<'_, G>,
        candidate: tex_state::diagnostic::RecoverableDiagnostic,
    ) {
        let (
            origin,
            observed_token,
            command,
            command_operand,
            frozen_context,
            mode,
            scanner_status,
        ) = match candidate.site {
            Some(site) => (
                site.origin,
                site.observed_token.map(observed_token_from_site),
                site.command,
                site.command_operand,
                site.context.map(|context| crate::FrozenDiagnosticContext {
                    cause_kind: candidate.kind,
                    input_frame_count: context.input_frame_count,
                    input_frame_tail: context.input_frame_tail,
                    group_depth: context.group_depth,
                    group_tail: context
                        .group_tail
                        .into_iter()
                        .map(|group| crate::FrozenDiagnosticGroup {
                            kind: group.kind,
                            entered_line: group.entered_line,
                        })
                        .collect(),
                }),
                diagnostic_mode(site.mode),
                site.scanner_status,
            ),
            None => (None, None, None, None, None, Mode::Vertical, "normal"),
        };
        let frozen_origin =
            origin.and_then(|origin| crate::error::freeze_diagnostic_origin(context, origin));
        let arguments = candidate
            .arguments
            .into_iter()
            .map(|argument| match argument {
                tex_state::diagnostic::RecoverableDiagnosticArgument::Name(value) => {
                    tex_command::DiagnosticArgument::Name(value)
                }
                tex_state::diagnostic::RecoverableDiagnosticArgument::Token(value) => {
                    tex_command::DiagnosticArgument::Token(observed_token_from_site(value))
                }
            })
            .collect();
        self.execution.first_recoverable_diagnostic =
            Some(Box::new(crate::FirstRecoverableDiagnostic {
                kind: candidate.kind,
                message: candidate.message,
                arguments,
                command,
                command_operand,
                observed_token,
                origin: frozen_origin,
                context: frozen_context,
                mode,
                scanner_status,
                interaction: candidate.interaction,
            }));
    }

    pub(super) fn capture_first_reported_command_error_context(
        &mut self,
        stores: &mut Universe<G>,
    ) {
        if self.execution.first_causal_context.is_none()
            && stores.world().error_channel().error_count() > 0
        {
            let context = stores.command_context().expect("live generation");
            self.execution.first_causal_context = Some(crate::FrozenDiagnosticContext::capture(
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
    }

    /// Closes every live receipt category and performs the append-bound check
    /// that must precede any operation commit.
    pub(super) fn admit_observed_receipt(
        &mut self,
        stores: &Universe<G>,
        termination: OperationTermination,
    ) -> Option<ExecError> {
        let (Some(start), Some(pending)) = (
            self.execution.operation_receipt_start,
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
        for artifact in &stores.world().committed_artifacts()[start.artifact..] {
            pending.record_artifact(artifact.hash());
        }
        pending.receipt.set_termination(termination);
        self.operation_evidence_limit_error()
    }
}

fn observed_token_from_site(
    token: tex_state::diagnostic::DiagnosticToken,
) -> tex_command::ObservedToken {
    match token {
        tex_state::diagnostic::DiagnosticToken::Character { character, catcode } => {
            tex_command::ObservedToken::Character { character, catcode }
        }
        tex_state::diagnostic::DiagnosticToken::ControlSequence(value) => {
            tex_command::ObservedToken::ControlSequence(value)
        }
        tex_state::diagnostic::DiagnosticToken::MacroMatch => {
            tex_command::ObservedToken::MacroMatch
        }
        tex_state::diagnostic::DiagnosticToken::MacroEndMatch => {
            tex_command::ObservedToken::MacroEndMatch
        }
        tex_state::diagnostic::DiagnosticToken::Parameter(value) => {
            tex_command::ObservedToken::Parameter(value)
        }
        tex_state::diagnostic::DiagnosticToken::FrozenEndTemplate => {
            tex_command::ObservedToken::FrozenEndTemplate
        }
        tex_state::diagnostic::DiagnosticToken::FrozenEndV => {
            tex_command::ObservedToken::FrozenEndV
        }
        tex_state::diagnostic::DiagnosticToken::FrozenPrimitive(value) => {
            tex_command::ObservedToken::FrozenPrimitive(value)
        }
        tex_state::diagnostic::DiagnosticToken::FrozenOther => {
            tex_command::ObservedToken::FrozenOther
        }
    }
}

fn diagnostic_mode(mode: Option<&'static str>) -> Mode {
    match mode {
        Some("vertical mode") => Mode::Vertical,
        Some("internal vertical mode") => Mode::InternalVertical,
        Some("horizontal mode") => Mode::Horizontal,
        Some("restricted horizontal mode") => Mode::RestrictedHorizontal,
        Some("math mode") => Mode::Math,
        Some("display math mode") => Mode::DisplayMath,
        None => Mode::Vertical,
        Some(_) => Mode::Vertical,
    }
}

impl<G> MainControl<G> {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn episode_commit_boundary(
        &self,
        stores: &Universe<G>,
        applied: &Result<MainControlStep, ExecError>,
        operations: usize,
        max_operations: usize,
        _initial_boundaries: usize,
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
        if self.execution.dumped_format.is_some() != initial_format_dump {
            return Some(crate::EpisodeCommitBoundary::Semantic(
                crate::SemanticEpisodeBarrier::Format,
            ));
        }
        if self.execution.fatal.is_some() {
            return Some(crate::EpisodeCommitBoundary::Semantic(
                crate::SemanticEpisodeBarrier::Diagnostic,
            ));
        }
        if self.execution.first_causal_context.is_some() != initial_diagnostic
            || stores.world().error_channel().error_count() != initial_error_count
        {
            return Some(crate::EpisodeCommitBoundary::Semantic(
                crate::SemanticEpisodeBarrier::Diagnostic,
            ));
        }
        if matches!(
            applied,
            Ok(MainControlStep::End | MainControlStep::EndOfInput)
        ) {
            return Some(crate::EpisodeCommitBoundary::Terminal);
        }
        if stores.world().committed_artifacts().len() != initial_artifacts {
            return Some(crate::EpisodeCommitBoundary::Semantic(
                crate::SemanticEpisodeBarrier::Output,
            ));
        }
        if self.execution.paragraph_checkpoint_cut {
            return Some(crate::EpisodeCommitBoundary::NamedCheckpoint(
                crate::EngineBoundary::OuterParagraphEnd,
            ));
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
        _initial_boundaries: usize,
        initial_effect_pos: tex_state::EffectPos,
    ) {
        self.execution
            .episode_telemetry
            .record_commit(crate::EpisodeCommit::new(
                operations
                    .try_into()
                    .expect("bounded episode operation count fits u16"),
                boundary,
            ));
        if stores.world().committed_artifacts().len() != initial_artifacts
            && boundary
                != crate::EpisodeCommitBoundary::Semantic(crate::SemanticEpisodeBarrier::Output)
        {
            self.execution
                .episode_telemetry
                .record_semantic_barrier(crate::SemanticEpisodeBarrier::Output);
        }
        if stores.world().effect_pos() != initial_effect_pos
            && boundary
                != crate::EpisodeCommitBoundary::Semantic(crate::SemanticEpisodeBarrier::Effect)
        {
            self.execution
                .episode_telemetry
                .record_semantic_barrier(crate::SemanticEpisodeBarrier::Effect);
        }
        self.execution.advance_telemetry.commits += 1;
    }

    /// Halts the session at a failed operation without undoing it.
    ///
    /// pdfTeX has no per-command rollback, and nothing a halted session does
    /// afterwards is observable: the host either ends the run or restores a
    /// full checkpoint, which replaces every root this operation touched and
    /// abandons its attempt scope. The latch refuses any other entry.
    pub(super) fn halt_direct_operation(&mut self) {
        self.execution.halted = true;
    }

    /// Closes the observed receipt before committing a direct operation that
    /// will return an error. The caller has already settled any terminal
    /// diagnostic effects; this seam records the operation's committed status
    /// without classifying the typed error for an outer observer.
    pub(super) fn commit_direct_operation_after_error(
        &mut self,
        stores: &mut Universe<G>,
        diagnostic_effects: &mut DiagnosticEffects,
        termination: OperationTermination,
    ) -> Option<ExecError> {
        let evidence_error = self.admit_observed_receipt(stores, termination);
        self.execution.error_operation_committed = true;
        if diagnostic_effects.has_first_recoverable() {
            self.promote_first_recoverable(stores, diagnostic_effects);
        }
        evidence_error
    }

    pub(super) fn finish_direct_failure(
        &mut self,
        stores: &mut Universe<G>,
        error: ExecError,
        context: DirectFailureContext,
        mut diagnostic_effects: DiagnosticEffects,
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
        // pdftex.web §32b's `pdf_error` calls TeX82 §93 `succumb`. These
        // output failures and an invalid output artifact publish their
        // committed receipt after the nested shipout has rolled back its own
        // speculative suffix.
        if error.requires_terminal_settlement() {
            if error.is_pdftex_output_fatal() {
                Self::publish_pdf_fatal_error(stores, &error)?;
                self.observe_committed([CommandObservation::Effect(engine_termination_effect())]);
            }
            let evidence_error = self.commit_direct_operation_after_error(
                stores,
                &mut diagnostic_effects,
                OperationTermination::Failed,
            );
            self.record_direct_episode_commit(
                stores,
                operations,
                crate::EpisodeCommitBoundary::Semantic(crate::SemanticEpisodeBarrier::Diagnostic),
                initial_artifacts,
                initial_boundaries,
                initial_effect_pos,
            );
            return evidence_error.map_or(Err(error), Err);
        }
        let Some(fatal) = error.as_fatal() else {
            self.halt_direct_operation();
            return Err(error);
        };
        self.promote_first_recoverable(stores, &mut diagnostic_effects);
        let context = self
            .command
            .output_open_context(&stores.command_context().expect("live generation"));
        crate::diagnostics::report_irrecoverable_error(stores, fatal, context);
        self.execution.captured_fatal_origin = match &error {
            ExecError::Captured { site, frozen, .. } if fatal != FatalError::TooManyErrors => {
                Some((
                    *site,
                    frozen
                        .as_deref()
                        .and_then(|evidence| evidence.origin.clone()),
                    self.execution.first_causal_context.clone().or_else(|| {
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
            .publish_diagnostic_effects_preserving(&mut diagnostic_effects);
        let evidence_error = self.commit_direct_operation_after_error(
            stores,
            &mut diagnostic_effects,
            OperationTermination::Fatal(fatal),
        );
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

impl<G> MainControl<G> {}

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
    pub(super) fn publish_pdf_fatal_error(
        stores: &mut Universe<G>,
        error: &ExecError,
    ) -> Result<(), ExecError> {
        if error.is_pdftex_output_fatal() {
            crate::job::report_pdf_fatal_error(stores, &error.to_string());
            stores.publish_effect_prefix(stores.world().effect_pos())?;
        }
        Ok(())
    }

    pub(super) fn finish_resource_preflight_failure(
        &mut self,
        stores: &mut Universe<G>,
        error: ExecError,
        diagnostic_effects: &mut DiagnosticEffects,
    ) -> Result<StepResult, ExecError> {
        let error = {
            let mut context = stores.command_context().expect("live generation");
            error.freeze_diagnostic_origin(&mut context, self.command.diagnostic_input_context(8))
        };
        if let Some(fatal) = error.as_fatal() {
            if self.execution.first_causal_context.is_none() {
                let context = stores.command_context().expect("live generation");
                self.execution.first_causal_context =
                    Some(crate::FrozenDiagnosticContext::capture(
                        &context,
                        self.command.diagnostic_input_context(8),
                        "command-error",
                    ));
            }
            self.promote_first_recoverable(stores, diagnostic_effects);
            let context = self
                .command
                .output_open_context(&stores.command_context().expect("live generation"));
            crate::diagnostics::report_irrecoverable_error(stores, fatal, context);
            self.execution.captured_fatal_origin = match &error {
                ExecError::Captured { site, frozen, .. } if fatal != FatalError::TooManyErrors => {
                    Some((
                        *site,
                        frozen
                            .as_deref()
                            .and_then(|evidence| evidence.origin.clone()),
                        self.execution.first_causal_context.clone().or_else(|| {
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
        // A provider decline is surfaced by the scanner as the ordinary
        // missing-capability error so that all transient command state can
        // unwind. Preserve the provider's exact canonical request here;
        // aliases (notably `TeXinputs:` input lookup) must not be rewritten
        // to the scanner's packed fallback before the outer driver consumes
        // the one attempted-resource sideband.
        let declined_need = self.declined_resource_attempt.clone();
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
                    self.execution.pending_resource_site = site.primary_origin();
                    Ok(
                        self.observed_suspension(declined_need.unwrap_or(ResourceNeed::Input {
                            name,
                            original_name,
                        })),
                    )
                }
                ExecError::MissingInputProbe { request } => {
                    self.execution.pending_resource_site = site.primary_origin();
                    Ok(self.observed_suspension(
                        declined_need.unwrap_or(ResourceNeed::InputProbe { request }),
                    ))
                }
                ExecError::MissingFont { request } => {
                    self.execution.pending_resource_site = site.primary_origin();
                    Ok(self.observed_suspension(
                        declined_need.unwrap_or(ResourceNeed::Font { request: *request }),
                    ))
                }
                ExecError::MissingPdfImage { request } => {
                    self.execution.pending_resource_site = site.primary_origin();
                    Ok(self.observed_suspension(
                        declined_need.unwrap_or(ResourceNeed::PdfImage { request: *request }),
                    ))
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
            } => Ok(
                self.observed_suspension(declined_need.unwrap_or(ResourceNeed::Input {
                    name,
                    original_name,
                })),
            ),
            ExecError::MissingInputProbe { request } => Ok(self.observed_suspension(
                declined_need.unwrap_or(ResourceNeed::InputProbe { request }),
            )),
            ExecError::MissingFont { request } => Ok(self.observed_suspension(
                declined_need.unwrap_or(ResourceNeed::Font { request: *request }),
            )),
            ExecError::MissingPdfImage { request } => Ok(self.observed_suspension(
                declined_need.unwrap_or(ResourceNeed::PdfImage { request: *request }),
            )),
            error => Err(error),
        }
    }

    pub(super) fn observed_suspension(&mut self, need: ResourceNeed) -> StepResult {
        // A host answer is retained by the outer candidate, but it cannot
        // revive this MainControl object; only full checkpoint replay clears
        // the latch.
        self.execution.halted = true;
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

/// Explicit live-observer boundary for detached shipout geometry.
pub(super) struct MainControlShipoutGeometrySink<'a, G> {
    pub(super) command: &'a CommandState<G>,
    pub(super) observations: Option<&'a mut dyn CommandObserver>,
}

/// Explicit operation-local boundary for TeX82 §§649--676 packing geometry.
///
/// The command cursor supplies detached source coordinates while the
/// observation slot remains the sole rollback/publication owner. The sink
/// cannot outlive this command operation and owns no engine-state handle.
pub(super) struct MainControlPackGeometrySink<'a> {
    pub(super) line: u32,
    pub(super) source: Option<tex_command::SourceId>,
    pub(super) observations: Option<&'a mut dyn CommandObserver>,
}

pub(super) fn pack_geometry_sink<'a, G>(
    command: &CommandState<G>,
    observations: Option<&'a mut dyn CommandObserver>,
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

pub(super) fn report_scan_on_owners<G>(
    context: &mut CommandContext<'_, G>,
    command: &tex_command::CommandState<G>,
    execution: &mut ExecutionState<G>,
    effects: &mut DiagnosticEffects,
    diagnostics: Vec<PendingDiagnostic<G>>,
) -> Result<(), ExecError> {
    if execution.first_causal_context.is_none() {
        let cause = if context.reported_error_count() > 0 {
            Some("command-error")
        } else {
            diagnostics.iter().find_map(PendingDiagnostic::causal_kind)
        };
        if let Some(cause) = cause {
            execution.first_causal_context = Some(crate::FrozenDiagnosticContext::capture(
                context,
                command.diagnostic_input_context(8),
                cause,
            ));
        }
    }
    if diagnostics.is_empty() {
        return Ok(());
    }
    report_pending_diagnostics_in_context(context, effects, diagnostics)
}
