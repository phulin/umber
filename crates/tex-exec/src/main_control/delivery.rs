//! Command delivery, preflight scanning, and typed retry transitions.

use super::*;

pub(super) struct PreparedAlignmentPreamble<G> {
    pub(super) alignment: AlignmentIdentity,
    pub(super) columns: Vec<PreparedAlignmentCellTemplates<G>>,
    pub(super) tabskips: Vec<GlueSpec>,
    pub(super) default_tabskip: GlueSpec,
    pub(super) repeat_start: Option<usize>,
}

pub(super) fn fill_preflight_delivery_from_frame<G>(
    frame: &CommandEpisode<G>,
    preparation: &mut OperationPreparation<'_, G>,
    retained_preflight: Option<crate::transaction_protocol::CommandPreflight>,
) {
    let preflight = retained_preflight.unwrap_or_else(|| {
        match frame.phase.expect("retry frame owns its scalar phase") {
            PreflightCommandPhase::ImmediatePdfRetry(primitive) => {
                crate::transaction_protocol::canonical_static_command_preflight(
                    Meaning::UnexpandablePrimitive(primitive),
                )
            }
            PreflightCommandPhase::Expanding { .. } => {
                crate::transaction_protocol::canonical_static_command_preflight(Meaning::Relax)
            }
            _ => {
                crate::transaction_protocol::canonical_command_preflight(frame.current().meaning())
            }
        }
    });
    preparation.fill_preflight(OperationDelivery::Command, preflight, None, None);
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum PreflightReadiness {
    Ready,
    Failed,
}

/// Whether operand scanning must wait for a resource/transaction boundary.
///
/// These are the complete live facts consulted by the decision.  Passing
/// them through the already-admitted command episode lets ordinary delivery
/// continue directly into its scanner without opening a second context merely
/// to ask the same mode, group, and `\pdfoutput` questions again.
pub(super) fn command_requires_transaction_from_facts<G>(
    mode: Mode,
    boxes: &ReplayBoxes<G>,
    preflight: &crate::transaction_protocol::CommandPreflight,
    frame: &CommandEpisode<G>,
    pdf_output: i32,
    innermost_group: Option<GroupKind>,
) -> bool {
    let crate::transaction_protocol::CommandPreflight::Ordinary(ordinary) = preflight else {
        return true;
    };
    // pdfTeX's `check_pdfoutput` fails before operand scanning. ErrorStop can
    // change `\pdfoutput` and retry that untouched command, so DVI mode keeps
    // the retry transaction.
    if ordinary
        .mutation()
        .contains(crate::transaction_protocol::StateOwners::PDF)
        && pdf_output <= 0
    {
        return true;
    }
    if matches!(mode, Mode::Vertical | Mode::InternalVertical)
        && frame.current_option().is_some_and(|command| {
            matches!(
                command.meaning(),
                ResolvedMeaning::Static(Meaning::UnexpandablePrimitive(
                    UnexpandablePrimitive::PdfStartLink
                ))
            )
        })
    {
        return true;
    }
    if matches!(frame.phase, Some(PreflightCommandPhase::Expanding { .. })) {
        return true;
    }
    // A brace packaging an active box can enter page or shipout work. Braces
    // inside the box remain ordinary because their save-stack group differs.
    if frame.current_option().is_some_and(|command| {
        matches!(
            command.meaning(),
            ResolvedMeaning::Static(Meaning::CharToken {
                cat: Catcode::EndGroup,
                ..
            })
        )
    }) && boxes
        .active_boxes
        .last()
        .is_some_and(|active| innermost_group == Some(active.group_kind))
    {
        return true;
    }
    frame.current_option().is_some_and(|command| {
        matches!(
            command.meaning(),
            ResolvedMeaning::Static(Meaning::UnexpandablePrimitive(
                UnexpandablePrimitive::Global
                    | UnexpandablePrimitive::Long
                    | UnexpandablePrimitive::Outer
                    | UnexpandablePrimitive::Protected
                    | UnexpandablePrimitive::IgnoreSpaces
                    | UnexpandablePrimitive::NoBoundary
            ))
        )
    })
}

impl<G> MainControl<G> {
    pub(super) fn preflight_replay_delivery(
        &mut self,
        stores: &mut Universe<G>,
        host_preparation: &mut OperationPreparation<'_, G>,
        diagnostic_effects: &mut DiagnosticEffects,
        frame: &mut CommandEpisode<G>,
        cold: &mut ColdOperationSlot<G>,
    ) -> PreflightReadiness {
        frame.assert_empty();
        self.ensure_primitive_handles(stores);
        let mut diagnostics = Vec::new();
        let raw_main_loop_delivery = self.main_loop_active;
        let context_readiness = stores
            .with_command_context(|context| {
                let mode = self.modes.current_mode();
                if self.active_alignment.is_some()
                    || (mode == Mode::DisplayMath
                        && self.modes.current_list().has_display_alignment())
                {
                    host_preparation.fill_preflight(
                        OperationDelivery::Replay,
                        crate::transaction_protocol::canonical_static_command_preflight(
                            Meaning::Relax,
                        ),
                        None,
                        None,
                    );
                    return PreflightReadiness::Ready;
                }

                if self.enter_main_control(context) {
                    publish_named_token_list_pushes(
                        &mut self.command,
                        context,
                        diagnostic_effects,
                        &mut self.operation_observations,
                    );
                }
                let innermost_group = context.innermost_group_kind();
                let tracked_region_is_active = context.tracked_region_is_active();
                let job_is_all_over = crate::page_output::job_is_all_over(context);
                let display_eq_no = self.modes.current_list().display_eq_no().is_some();
                {
                    let mut host_facts = ExecutorHostFacts {
                        modes: &self.modes,
                        pdf_ignore_depth: self.pdf_ignore_depth,
                        telemetry: &mut self.episode_telemetry,
                    };
                    let mut processor = command_processor(
                        &mut self.command,
                        self.fuel.fuel_mut(),
                        &mut self.capabilities,
                        &mut host_facts,
                        &mut self.operation_observations,
                        diagnostic_effects,
                        context,
                    );
                    processor.set_output_routine_active(self.boxes.output_routine_active);
                    prepare_command_trace(&mut processor, mode, self.shown_mode);
                    // TeX82 has one raw-fetch/classification loop. Enter it once with
                    // §1038's first-command policy when the character loop is active;
                    // otherwise ordinary preflight publishes the unexpandable
                    // command's expanded observation directly and continues in place
                    // only when expansion is actually required.
                    let delivery = if raw_main_loop_delivery {
                        processor.main_loop_lookahead_into(&mut frame.command)
                    } else {
                        processor.preflight_command_into(&mut frame.command)
                    };
                    let status = match delivery {
                        Ok(status) => status,
                        Err(error) => {
                            // The expansion driver moves its live command into
                            // command state only after an actual immutable-host
                            // suspension. Fuel and semantic failures have no retry
                            // command and must not clone one speculatively.
                            if let Some(expansion) = processor.take_pending_expansion_work() {
                                frame.admit_expanding(
                                    expansion,
                                    self.main_loop_active,
                                    processor.delivery_cursor(),
                                );
                            }
                            drop(processor);
                            frame.error = Some(command_error(error));
                            return PreflightReadiness::Failed;
                        }
                    };
                    diagnostics.extend(
                        processor
                            .take_semantic_diagnostics()
                            .into_iter()
                            .map(PendingDiagnostic::Command),
                    );
                    // TeX82 §§299/367 advance `shown_mode` as soon as expansion
                    // prints a command trace. A recoverable expansion diagnostic is a
                    // reporting barrier below, but it does not undo that trace-state
                    // transition: the following settled command must not print the
                    // same mode prefix again in a fresh processor facade.
                    if processor.command_trace_printed() {
                        self.shown_mode = Some(mode);
                    }
                    let mut reported = false;
                    // Diagnostics are a real reporting barrier: preserve their
                    // established ordering before command tracing or operand work.
                    // The common diagnostic-free path continues in this same borrow.
                    if diagnostics.is_empty() && status == tex_command::DeliveryStatus::Command {
                        let continues_main_loop = self.main_loop_active
                            && matches!(
                                frame.current().meaning(),
                                ResolvedMeaning::Static(
                                    Meaning::CharToken {
                                        cat: Catcode::Letter | Catcode::Other,
                                        ..
                                    } | Meaning::CharGiven(_)
                                        | Meaning::UnexpandablePrimitive(
                                            UnexpandablePrimitive::Char
                                        )
                                )
                            );
                        if !continues_main_loop {
                            prepare_command_trace(&mut processor, mode, self.shown_mode);
                            report_main_control_command_trace(
                                &mut processor,
                                mode,
                                frame.current(),
                                &self.boxes,
                                &mut self.shown_mode,
                            );
                            reported = true;
                        }
                        let preflight = crate::transaction_protocol::canonical_command_preflight(
                            frame.current().meaning(),
                        );
                        let needs_barrier = tracked_region_is_active
                            || command_requires_transaction_from_facts(
                                mode,
                                &self.boxes,
                                &preflight,
                                frame,
                                processor.int_param(IntParam::PDF_OUTPUT),
                                innermost_group,
                            );
                        if !needs_barrier {
                            #[cfg(feature = "profiling")]
                            tex_state::measurement::record_hot_core_phase(
                                tex_state::measurement::HotCorePhase::DeliveryAndScan,
                            );
                            #[cfg(feature = "profiling")]
                            let _allocation_scope =
                                tex_state::measurement::hot_core_allocation_scope(
                                    tex_state::measurement::HotCoreAllocationOwner::DeliveryAndScan,
                                );
                            let scanned = dispatch_main_control_command(
                                &mut processor,
                                frame,
                                cold,
                                mode,
                                &self.boxes,
                                innermost_group,
                                job_is_all_over,
                                display_eq_no,
                                &mut self.shown_mode,
                                &mut diagnostics,
                                None,
                                true,
                            );
                            diagnostics.extend(
                                processor
                                    .take_semantic_diagnostics()
                                    .into_iter()
                                    .map(PendingDiagnostic::Command),
                            );
                            match scanned {
                                Ok(ScannedOperation::Hot) => {
                                    // The scanned operation now owns every durable
                                    // result. Retire the delivery/scanner episode as a
                                    // unit before handing that operation to execution;
                                    // no preflight marker belongs to the next stage.
                                    frame.retain_source_role();
                                    frame.clear_preflight();
                                    host_preparation.fill_preflight(
                                        OperationDelivery::ResidentHot,
                                        preflight,
                                        None,
                                        None,
                                    );
                                }
                                Ok(ScannedOperation::Cold) => {
                                    frame.retain_source_role();
                                    frame.clear_preflight();
                                    host_preparation.fill_preflight(
                                        OperationDelivery::ResidentCold,
                                        preflight,
                                        None,
                                        None,
                                    );
                                }
                                Err(error) => {
                                    let cursor = processor.delivery_cursor();
                                    if execution_error_needs_command_retry(&error) {
                                        if !frame.has_preflight() {
                                            let retry_expansion =
                                                processor.take_pending_expansion_work();
                                            let scanner = processor.take_scanner_resume();
                                            if let Some(expansion) = retry_expansion {
                                                frame.discard_resident_command();
                                                frame.admit_expanding(
                                                    expansion,
                                                    self.main_loop_active,
                                                    cursor,
                                                );
                                            } else {
                                                frame.mark_resident_settled(Some(cursor));
                                                frame.scanner = scanner;
                                            }
                                        }
                                    } else {
                                        frame.discard_resident_command();
                                    }
                                    frame.error = Some(error);
                                }
                            }
                        } else {
                            host_preparation.record_command_preflight(preflight);
                        }
                    }
                    if status == tex_command::DeliveryStatus::Command
                        && frame.current_option().is_some()
                        && !frame.has_preflight()
                        && !host_preparation.has_preflight()
                    {
                        let cursor = processor.delivery_cursor();
                        let continues_main_loop = self.main_loop_active
                            && matches!(
                                frame.current().meaning(),
                                ResolvedMeaning::Static(
                                    Meaning::CharToken {
                                        cat: Catcode::Letter | Catcode::Other,
                                        ..
                                    } | Meaning::CharGiven(_)
                                        | Meaning::UnexpandablePrimitive(
                                            UnexpandablePrimitive::Char
                                        )
                                )
                            );
                        if raw_main_loop_delivery && continues_main_loop {
                            frame.mark_resident_raw(Some(cursor));
                        } else {
                            frame.mark_resident_settled(Some(cursor));
                        }
                    }
                    host_preparation.record_delivery_status(status, reported);
                };
                PreflightReadiness::Ready
            })
            .expect("live generation");
        if context_readiness == PreflightReadiness::Failed {
            return PreflightReadiness::Failed;
        }
        let mode = self.modes.current_mode();
        self.capture_first_reported_command_error_context(stores);
        self.capture_first_causal_context(stores, &diagnostics);
        if let Err(error) = report_pending_diagnostics(stores, diagnostic_effects, diagnostics) {
            frame.error = Some(error);
            return PreflightReadiness::Failed;
        }
        if frame.error.is_some() {
            return PreflightReadiness::Failed;
        }
        if host_preparation.has_preflight() {
            host_preparation.discard_delivery_status();
            return PreflightReadiness::Ready;
        }

        let delivery_status = host_preparation.take_delivery_status();
        let trace_reported = host_preparation.take_trace_reported();

        let passive =
            || crate::transaction_protocol::canonical_static_command_preflight(Meaning::Relax);
        match delivery_status {
            tex_command::DeliveryStatus::End => {
                debug_assert!(frame.command.is_none());
                frame.write_unavailable(cold, ColdOperation::<G>::EndOfInput);
                host_preparation.fill_preflight(
                    OperationDelivery::SuspendedCold,
                    passive(),
                    None,
                    None,
                );
                return PreflightReadiness::Ready;
            }
            tex_command::DeliveryStatus::ReplayCompleted(episode) => {
                debug_assert!(frame.command.is_none());
                frame.write_unavailable(cold, ColdOperation::<G>::ReplayCompleted(episode));
                host_preparation.fill_preflight(
                    OperationDelivery::SuspendedCold,
                    passive(),
                    None,
                    None,
                );
                return PreflightReadiness::Ready;
            }
            tex_command::DeliveryStatus::Command => {}
            _ => unreachable!("raw preflight delivery has no alignment event"),
        }

        let continues_main_loop = self.main_loop_active
            && matches!(
                frame.current().meaning(),
                ResolvedMeaning::Static(
                    Meaning::CharToken {
                        cat: Catcode::Letter | Catcode::Other,
                        ..
                    } | Meaning::CharGiven(_)
                        | Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Char)
                )
            );
        if !continues_main_loop && !trace_reported {
            let mut context = stores.command_context().expect("live generation");
            let mut host_facts = ExecutorHostFacts {
                modes: &self.modes,
                pdf_ignore_depth: self.pdf_ignore_depth,
                telemetry: &mut self.episode_telemetry,
            };
            let mut processor = command_processor(
                &mut self.command,
                self.fuel.fuel_mut(),
                &mut self.capabilities,
                &mut host_facts,
                &mut self.operation_observations,
                diagnostic_effects,
                &mut context,
            );
            prepare_command_trace(&mut processor, mode, self.shown_mode);
            report_main_control_command_trace(
                &mut processor,
                mode,
                frame.current(),
                &self.boxes,
                &mut self.shown_mode,
            );
        }

        if self.main_loop_active
            && matches!(mode, Mode::Horizontal | Mode::RestrictedHorizontal)
            && matches!(
                frame.current().meaning(),
                ResolvedMeaning::Static(Meaning::UnexpandablePrimitive(
                    UnexpandablePrimitive::NoBoundary
                ))
            )
            && self.operation_observations.is_none()
        {
            frame.write_unavailable(
                cold,
                ColdOperation::<G>::NoBoundary {
                    suppress_right: true,
                },
            );
            let preflight = host_preparation
                .take_recorded_preflight()
                .unwrap_or_else(|| {
                    crate::transaction_protocol::canonical_command_preflight(
                        frame.current().meaning(),
                    )
                });
            frame.retain_source_role();
            frame.discard_resident_command();
            host_preparation.fill_preflight(
                OperationDelivery::SuspendedCold,
                preflight,
                None,
                None,
            );
            return PreflightReadiness::Ready;
        }
        let preflight = host_preparation
            .take_recorded_preflight()
            .unwrap_or_else(|| {
                crate::transaction_protocol::canonical_command_preflight(frame.current().meaning())
            });
        assert!(
            frame.cursor.is_some(),
            "a live command crossing preflight retains its delivery cursor"
        );
        debug_assert_eq!(
            matches!(frame.phase, Some(PreflightCommandPhase::Raw)),
            raw_main_loop_delivery && continues_main_loop
        );
        host_preparation.fill_preflight(OperationDelivery::Command, preflight, None, None);
        PreflightReadiness::Ready
    }
}

/// The closer TeX82 §1065 selects for `cur_group`, in the form its report
/// prints it: `print_esc` for the two frozen control sequences, `print_char`
/// for the two literal characters.
#[derive(Clone, Copy)]
pub(super) enum OffSaveCloser {
    EndGroup,
    MathShift,
    NullRight,
    RightBrace,
}

impl OffSaveCloser {
    pub(super) fn print<G>(self, report: &mut tex_state::print::ErrorReport<'_, G>) {
        match self {
            Self::EndGroup => report.print_esc("endgroup"),
            Self::MathShift => report.print_char('$'),
            Self::NullRight => report.print_esc("right."),
            Self::RightBrace => report.print_char('}'),
        };
    }
}

/// TeX82 §1069's `case cur_group of`: the group opener a stray `}` was
/// probably standing in for.
///
/// This is deliberately not [`OffSaveCloser`]. §1064 inserts a closer and says
/// what it inserted, so its `math_left_group` arm is `\right.` -- a complete
/// command. §1069 deletes the brace and only names what was forgotten, so its
/// arm is the bare `\right`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ForgottenGroupOpener {
    /// `semi_simple_group`.
    EndGroup,
    /// `math_shift_group`.
    MathShift,
    /// `math_left_group`.
    Right,
}

impl ForgottenGroupOpener {
    pub(super) fn print<G>(self, report: &mut tex_state::print::ErrorReport<'_, G>) {
        match self {
            Self::EndGroup => report.print_esc("endgroup"),
            Self::MathShift => report.print_char('$'),
            Self::Right => report.print_esc("right"),
        };
    }
}

/// Selects the one command-owned scanner that may consume input before
/// ordinary main control. Alignment preamble setup validates and backs up its
/// opening brace twice through successive command-owned backup levels; only
/// the second replay reaches TeX82's live preamble scanner.
#[allow(clippy::too_many_arguments)] // owns the replay-only command/input seam
pub(super) fn scan_replay_step<G>(
    processor: &mut CommandProcessor<'_, '_, G>,
    mode: Mode,
    boxes: &ReplayBoxes<G>,
    alignment_preamble: Option<(AlignmentIdentity, AlignmentPreamblePhase)>,
    innermost_group: Option<GroupKind>,
    job_is_all_over: bool,
    display_eq_no: bool,
    main_loop_active: bool,
    shown_mode: &mut Option<Mode>,
    diagnostics: &mut Vec<PendingDiagnostic<G>>,
    frame: &mut CommandEpisode<G>,
    cold: &mut ColdOperationSlot<G>,
) -> Result<ScannedOperation, ExecError> {
    if let Some((alignment, phase)) = alignment_preamble {
        return match phase {
            AlignmentPreamblePhase::Opening => {
                // TeX82 §§299/367: `scan_spec` expands its optional
                // dimension while `init_align` is already in the alignment's
                // newly pushed mode.  An expandable command such as `\the`
                // therefore crosses `show_cur_cmd_chr` here, before ordinary
                // main control gets another command.  Carry the same pending
                // mode prefix and `shown_mode` update that `scan_step` owns
                // around its `get_x_token` boundary.
                prepare_command_trace(processor, mode, *shown_mode);
                let packing = processor
                    .scan_alignment_preamble_opening()
                    .map_err(command_error)?;
                if processor.command_trace_printed() {
                    *shown_mode = Some(mode);
                }
                Ok(retain_cold_operation(
                    frame,
                    cold,
                    ColdOperation::<G>::AlignmentPreambleOpening { alignment, packing },
                ))
            }
            AlignmentPreamblePhase::Start { owner } => {
                // TeX82 §§299, 367, 759, and 774: `init_align` has already
                // pushed the alignment mode when §759 expands the token after
                // `\span`. This scanner episode is the first processor that
                // can print a command after that push, so it owns the pending
                // mode prefix just like the packing-spec episode above.
                prepare_command_trace(processor, mode, *shown_mode);
                processor
                    .begin_alignment_preamble_scan(owner)
                    .map_err(command_error)?;
                if processor.command_trace_printed() {
                    *shown_mode = Some(mode);
                }
                Ok(retain_cold_operation(
                    frame,
                    cold,
                    ColdOperation::<G>::AlignmentPreambleStart { alignment },
                ))
            }
            AlignmentPreamblePhase::CellOpening => {
                let opening = processor
                    .scan_alignment_cell_opening()
                    .map_err(command_error)?;
                Ok(retain_cold_operation(
                    frame,
                    cold,
                    ColdOperation::<G>::AlignmentCellOpening { alignment, opening },
                ))
            }
            AlignmentPreamblePhase::NextCellOpening => {
                let opening = processor
                    .scan_alignment_next_cell_opening()
                    .map_err(command_error)?;
                Ok(retain_cold_operation(
                    frame,
                    cold,
                    ColdOperation::<G>::AlignmentCellOpening { alignment, opening },
                ))
            }
            AlignmentPreamblePhase::AlignPeek { after_noalign } => {
                scan_alignment_peek(cold, processor, alignment, after_noalign)?;
                frame.mark_resident_cold(cold);
                Ok(ScannedOperation::Cold)
            }
            AlignmentPreamblePhase::NoAlignBody => scan_noalign_body(
                processor,
                alignment,
                boxes,
                innermost_group,
                mode,
                job_is_all_over,
                shown_mode,
                diagnostics,
                frame,
                cold,
            ),
            AlignmentPreamblePhase::CellDelivery => scan_alignment_delivery_step(
                processor,
                alignment,
                boxes,
                innermost_group,
                mode,
                job_is_all_over,
                main_loop_active,
                shown_mode,
                diagnostics,
                frame,
                cold,
            ),
        };
    }
    scan_step(
        processor,
        mode,
        boxes,
        innermost_group,
        job_is_all_over,
        display_eq_no,
        main_loop_active,
        shown_mode,
        diagnostics,
        frame,
        cold,
    )
}

#[derive(Clone, Copy)]
pub(super) enum AlignmentPreamblePhase {
    Opening,
    Start {
        owner: Option<tex_state::interner::Symbol>,
    },
    CellOpening,
    NextCellOpening,
    AlignPeek {
        after_noalign: bool,
    },
    NoAlignBody,
    CellDelivery,
}

pub(super) fn alignment_preamble<G>(
    active: Option<&mut ActiveReplayAlignment<G>>,
) -> Option<(AlignmentIdentity, AlignmentPreamblePhase)> {
    let active = active?;
    if active.preamble_opening_pending {
        Some((active.identity, AlignmentPreamblePhase::Opening))
    } else if active.preamble_start_pending {
        Some((
            active.identity,
            AlignmentPreamblePhase::Start {
                owner: active.owner,
            },
        ))
    } else if active.cell_opening_pending {
        Some((active.identity, AlignmentPreamblePhase::CellOpening))
    } else if active.next_cell_opening_pending {
        Some((active.identity, AlignmentPreamblePhase::NextCellOpening))
    } else if active.align_peek_pending {
        let after_noalign = active.align_peek_after_noalign;
        active.align_peek_after_noalign = false;
        Some((
            active.identity,
            AlignmentPreamblePhase::AlignPeek { after_noalign },
        ))
    } else if active.noalign_open {
        Some((active.identity, AlignmentPreamblePhase::NoAlignBody))
    } else {
        Some((active.identity, AlignmentPreamblePhase::CellDelivery))
    }
}

/// TeX82 §37's post-row lookahead.  This is deliberately separate from
/// `init_col`: `\\noalign` consumes its opening brace directly, whereas an
/// ordinary next-cell command is backed up for template installation.
pub(super) fn scan_alignment_peek<G>(
    cold: &mut ColdOperationSlot<G>,
    processor: &mut CommandProcessor<'_, '_, G>,
    alignment: AlignmentIdentity,
    _after_noalign: bool,
) -> Result<(), ExecError> {
    processor
        .begin_alignment_peek(_after_noalign)
        .map_err(command_error)?;
    let lookahead = processor
        .next_alignment_lookahead()
        .map_err(command_error)?
        .ok_or(ExecError::MissingToken {
            context: "alignment lookahead",
        })?;
    match lookahead.command().meaning() {
        ResolvedMeaning::Static(Meaning::UnexpandablePrimitive(UnexpandablePrimitive::NoAlign)) => {
            let _ = processor.commit_alignment_lookahead_delivery(lookahead);
            processor
                .scan_alignment_noalign_opening()
                .map_err(command_error)?;
            complete_cold_scan!(cold, ColdOperation::<G>::BeginNoAlign { alignment })
        }
        ResolvedMeaning::Static(Meaning::UnexpandablePrimitive(UnexpandablePrimitive::CrCr)) => {
            let _ = processor.commit_alignment_lookahead_delivery(lookahead);
            complete_cold_scan!(cold, ColdOperation::<G>::AlignPeekRestart { alignment })
        }
        ResolvedMeaning::Static(Meaning::CharToken {
            cat: Catcode::EndGroup,
            ..
        }) => {
            let command = processor.commit_alignment_lookahead_delivery(lookahead);
            let current_line = command
                .direct_source_line_number()
                .unwrap_or_else(|| processor.current_file_line_number());
            complete_cold_scan!(
                cold,
                ColdOperation::<G>::AlignmentFinish {
                    alignment,
                    current_line,
                }
            )
        }
        ResolvedMeaning::Static(Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Omit)) => {
            let _ = processor.commit_alignment_lookahead_delivery(lookahead);
            complete_cold_scan!(
                cold,
                ColdOperation::<G>::AlignmentPeekCell {
                    alignment,
                    omit: true,
                }
            )
        }
        _ => {
            processor
                .back_alignment_lookahead(lookahead)
                .map_err(command_error)?;
            complete_cold_scan!(
                cold,
                ColdOperation::<G>::AlignmentPeekCell {
                    alignment,
                    omit: false,
                }
            )
        }
    }
}

#[allow(clippy::too_many_arguments)] // carries command-owned noalign replay facts
pub(super) fn scan_noalign_body<G>(
    processor: &mut CommandProcessor<'_, '_, G>,
    alignment: AlignmentIdentity,
    boxes: &ReplayBoxes<G>,
    innermost_group: Option<GroupKind>,
    mode: Mode,
    job_is_all_over: bool,
    shown_mode: &mut Option<Mode>,
    diagnostics: &mut Vec<PendingDiagnostic<G>>,
    frame: &mut CommandEpisode<G>,
    cold: &mut ColdOperationSlot<G>,
) -> Result<ScannedOperation, ExecError> {
    prepare_command_trace(processor, mode, *shown_mode);
    let mut destination = None;
    if processor
        .get_x_token_into(&mut destination)
        .map_err(command_error)?
        != tex_command::DeliveryStatus::Command
    {
        return Ok(retain_cold_operation(
            frame,
            cold,
            ColdOperation::<G>::EndOfInput,
        ));
    }
    let command = destination
        .take()
        .expect("command status initializes destination");
    report_main_control_command_trace(processor, mode, &command, boxes, shown_mode);
    match command.meaning() {
        ResolvedMeaning::Static(Meaning::CharToken {
            cat: Catcode::EndGroup,
            ..
        }) if innermost_group == Some(GroupKind::NoAlign) => {
            if partoken_context_replays(processor, mode, 2) {
                processor
                    .insert_partoken_before(command)
                    .map_err(command_error)?;
                return Ok(retain_cold_operation(
                    frame,
                    cold,
                    ColdOperation::<G>::Continue,
                ));
            }
            Ok(retain_cold_operation(
                frame,
                cold,
                ColdOperation::<G>::NoAlignEndGroup { alignment },
            ))
        }
        // A `\noalign` body is ordinary main control between its braces
        // (TeX82 §785's `no_align_group`), so it dispatches through the same
        // §1030 `reswitch:`/§1211 prefix path as any other step.
        _ => {
            frame.admit_settled(command, None);
            dispatch_main_control_command(
                processor,
                frame,
                cold,
                mode,
                boxes,
                innermost_group,
                job_is_all_over,
                false,
                shown_mode,
                diagnostics,
                None,
                true,
            )
        }
    }
}

/// Delivers one active cell command through the command-owned alignment
/// boundary.  This remains separate from preamble and opener scans because a
/// completed scanner (such as a rule specification) can leave a backed-up
/// delimiter ready for the next main-control step.
#[allow(clippy::too_many_arguments)] // carries command-owned replay facts
pub(super) fn scan_alignment_delivery_step<G>(
    processor: &mut CommandProcessor<'_, '_, G>,
    alignment: AlignmentIdentity,
    boxes: &ReplayBoxes<G>,
    innermost_group: Option<GroupKind>,
    mode: Mode,
    job_is_all_over: bool,
    main_loop_active: bool,
    shown_mode: &mut Option<Mode>,
    diagnostics: &mut Vec<PendingDiagnostic<G>>,
    frame: &mut CommandEpisode<G>,
    cold: &mut ColdOperationSlot<G>,
) -> Result<ScannedOperation, ExecError> {
    prepare_command_trace(processor, mode, *shown_mode);
    let mut destination = None;
    let delivery = processor
        .get_x_alignment_delivery_into(main_loop_active, &mut destination)
        .map_err(command_error)?;
    match delivery {
        tex_command::DeliveryStatus::End => Ok(retain_cold_operation(
            frame,
            cold,
            ColdOperation::<G>::EndOfInput,
        )),
        // An executor-owned replay episode (a math field/group/choice branch
        // or discretionary part) retired mid-cell. This must be reported
        // exactly like ordinary `scan_step`'s `ReplayCompleted` case, rather
        // than falling through to interpret whatever the cascade found next
        // as this cell's own content: that next token can belong to the
        // *enclosing* cell/field context, not the just-retired episode.
        tex_command::DeliveryStatus::ReplayCompleted(episode) => Ok(retain_cold_operation(
            frame,
            cold,
            ColdOperation::<G>::ReplayCompleted(episode),
        )),
        tex_command::DeliveryStatus::Command => {
            let command = destination.expect("command status initializes destination");
            // TeX82 §§1034/1038 keeps an adjacent character fetched by
            // `main_loop_lookahead` inside `main_loop`, even when §789's
            // u-template/body handoff lies between the two characters. The
            // lookahead is a raw delivery owned by alignment control, but it
            // does not create a second §1030 `reswitch` trace boundary.
            let continues_main_loop = main_loop_active
                && matches!(
                    command.meaning(),
                    ResolvedMeaning::Static(
                        Meaning::CharToken {
                            cat: Catcode::Letter | Catcode::Other,
                            ..
                        } | Meaning::CharGiven(_)
                            | Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Char)
                    )
                );
            if !continues_main_loop {
                report_main_control_command_trace(processor, mode, &command, boxes, shown_mode);
            }
            // TeX82 §1132 dispatches every right brace seen with an active
            // `align_group` through the missing-\cr recovery, independent of
            // `align_state`. The command-owned fast path emits a structural
            // ClosingBrace event at the ordinary cell depth, but §1096's
            // `off_save` can insert the brace after `align_state` is already
            // negative. That brace must still back up behind frozen `\cr`;
            // treating it as an ordinary extra brace makes `\par` repeat
            // `off_save` forever while recovery input levels accumulate.
            if innermost_group == Some(GroupKind::Align)
                && matches!(
                    command.meaning(),
                    ResolvedMeaning::Static(Meaning::CharToken {
                        cat: Catcode::EndGroup,
                        ..
                    })
                )
            {
                processor
                    .recover_alignment_closing_brace(
                        tex_command::AlignmentDeliveryEvent::ClosingBrace(command),
                    )
                    .map_err(command_error)?;
                return Ok(retain_cold_operation(
                    frame,
                    cold,
                    ColdOperation::<G>::MissingAlignmentCr,
                ));
            }
            if matches!(command.meaning(), ResolvedMeaning::Static(Meaning::EndV)) {
                // TeX82 §§1046-1047 route `mmode+endv` through
                // `insert_dollar_sign`, just like every other command that
                // reaches an alignment v-template before its math mode has
                // closed. The synthesized `$` closes math first; the backed
                // up `endv` is then redelivered in the cell's h/v mode and
                // reaches §1131 below.
                if matches!(mode, Mode::Math | Mode::DisplayMath) {
                    processor
                        .recover_missing_math_shift(command)
                        .map_err(command_error)?;
                    return Ok(retain_cold_operation(
                        frame,
                        cold,
                        ColdOperation::<G>::MissingMathShift,
                    ));
                }
                if partoken_context_replays(processor, mode, 2) {
                    processor
                        .insert_partoken_before(command)
                        .map_err(command_error)?;
                    return Ok(retain_cold_operation(
                        frame,
                        cold,
                        ColdOperation::<G>::Continue,
                    ));
                }
                // TeX82 §1131 accepts end-v only when `cur_group=align_group`.
                // The replay driver tracks align-error's inserted `{`
                // separately because its structural alignment boundary is
                // executor-owned. Ordinary `\begingroup` is nevertheless a
                // real `semi_simple_group` save-stack level, and must close
                // through §§1064--1065 `off_save` before the same end-v is
                // replayed. Other intervening groups are intercepted by their
                // owning mode/box delivery paths before reaching this cell
                // finish boundary.
                if boxes.recovery_simple_group_open
                    || innermost_group == Some(GroupKind::SemiSimple)
                {
                    scan_off_save(cold, processor, command, innermost_group)?;
                    frame.mark_resident_cold(cold);
                    return Ok(ScannedOperation::Cold);
                }
                return Ok(retain_cold_operation(
                    frame,
                    cold,
                    ColdOperation::<G>::AlignmentCellFinish { alignment },
                ));
            }
            // An alignment cell's body is ordinary main control bounded by
            // §1130's `vmode+endv,hmode+endv: do_endv`, not a dispatcher of
            // its own, so it takes
            // the same §1030 `reswitch:`/§1211 prefix path as any other step.
            frame.admit_settled(command, None);
            dispatch_main_control_command(
                processor,
                frame,
                cold,
                mode,
                boxes,
                innermost_group,
                job_is_all_over,
                false,
                shown_mode,
                diagnostics,
                Some(alignment),
                true,
            )
        }
        tex_command::DeliveryStatus::AlignmentEndTemplate => {
            let event = tex_command::AlignmentDeliveryEvent::EndTemplate(
                destination.expect("alignment status initializes destination"),
            );
            scan_alignment_delivery_event(cold, processor, alignment, event)?;
            frame.mark_resident_cold(cold);
            Ok(ScannedOperation::Cold)
        }
        tex_command::DeliveryStatus::AlignmentClosingBrace => {
            let event = tex_command::AlignmentDeliveryEvent::ClosingBrace(
                destination.expect("alignment status initializes destination"),
            );
            scan_alignment_delivery_event(cold, processor, alignment, event)?;
            frame.mark_resident_cold(cold);
            Ok(ScannedOperation::Cold)
        }
        tex_command::DeliveryStatus::PendingExpanded => {
            unreachable!("alignment delivery commits terminal observations")
        }
    }
}

/// Applies a raw-delivery alignment boundary surfaced while TeX82 main
/// control owns an active entry.
///
/// Most boundaries come from `scan_alignment_delivery_step`'s initial
/// `get_x_token`, but §1045's `\ignorespaces` performs another expanded fetch
/// before returning to `reswitch`. TeX82 §342 still inserts the v-template at
/// that nested fetch, so the split executor must receive the same typed event
/// instead of dispatching the resulting frozen `\endv` as ordinary content.
pub(super) fn scan_alignment_delivery_event<G>(
    cold: &mut ColdOperationSlot<G>,
    processor: &mut CommandProcessor<'_, '_, G>,
    alignment: AlignmentIdentity,
    event: tex_command::AlignmentDeliveryEvent<G>,
) -> Result<(), ExecError> {
    match event {
        tex_command::AlignmentDeliveryEvent::EndTemplate(delimiter) => {
            processor
                .begin_alignment_v_template(
                    alignment,
                    tex_command::AlignmentDeliveryEvent::EndTemplate(delimiter),
                )
                .map_err(command_error)?;
            complete_cold_scan!(cold, ColdOperation::<G>::AlignmentTemplateEntered)
        }
        tex_command::AlignmentDeliveryEvent::ClosingBrace(_) => {
            // TeX82 §1132 selects this executor-owned align_group branch. Raw
            // brace backup/correction and frozen-\cr insertion remain entirely
            // command-owned.
            processor
                .recover_alignment_closing_brace(event)
                .map_err(command_error)?;
            complete_cold_scan!(cold, ColdOperation::<G>::MissingAlignmentCr)
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn settle_preflight_step<G>(
    processor: &mut CommandProcessor<'_, '_, G>,
    command: &mut CommandEpisode<G>,
    cold: &mut ColdOperationSlot<G>,
    main_loop: bool,
    mode: Mode,
    boxes: &ReplayBoxes<G>,
    innermost_group: Option<GroupKind>,
    job_is_all_over: bool,
    display_eq_no: bool,
    shown_mode: &mut Option<Mode>,
    diagnostics: &mut Vec<PendingDiagnostic<G>>,
) -> Result<ScannedOperation, ExecError> {
    let expansion = command.take_expansion();
    match processor
        .resume_expansion_into(expansion, main_loop, &mut command.command)
        .map_err(command_error)?
    {
        tex_command::DeliveryStatus::End => {
            return Ok(retain_cold_operation(
                command,
                cold,
                ColdOperation::<G>::EndOfInput,
            ));
        }
        tex_command::DeliveryStatus::ReplayCompleted(episode) => {
            return Ok(retain_cold_operation(
                command,
                cold,
                ColdOperation::<G>::ReplayCompleted(episode),
            ));
        }
        tex_command::DeliveryStatus::Command => {}
        _ => unreachable!("preflight settlement has no alignment event"),
    };
    command.settle_resident();
    // TeX82 §§380 and 473--479 keep operand scanning under the newly settled
    // unexpandable command. Expansion owns the retry only until settlement;
    // after this point a resource failure must re-enter this command before
    // any nested scanner continuation can resume.
    let continues_main_loop = main_loop
        && matches!(
            command.current().meaning(),
            ResolvedMeaning::Static(
                Meaning::CharToken {
                    cat: Catcode::Letter | Catcode::Other,
                    ..
                } | Meaning::CharGiven(_)
                    | Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Char)
            )
        );
    if !continues_main_loop {
        report_main_control_command_trace(processor, mode, command.current(), boxes, shown_mode);
    }
    if main_loop
        && matches!(mode, Mode::Horizontal | Mode::RestrictedHorizontal)
        && matches!(
            command.current().meaning(),
            ResolvedMeaning::Static(Meaning::UnexpandablePrimitive(
                UnexpandablePrimitive::NoBoundary
            ))
        )
    {
        return Ok(retain_cold_operation(
            command,
            cold,
            ColdOperation::<G>::NoBoundary {
                suppress_right: true,
            },
        ));
    }
    dispatch_main_control_command(
        processor,
        command,
        cold,
        mode,
        boxes,
        innermost_group,
        job_is_all_over,
        display_eq_no,
        shown_mode,
        diagnostics,
        None,
        true,
    )
}

#[allow(clippy::too_many_arguments)]
pub(super) fn scan_preflight_command<G>(
    processor: &mut CommandProcessor<'_, '_, G>,
    command: &mut CommandEpisode<G>,
    cold: &mut ColdOperationSlot<G>,
    mode: Mode,
    boxes: &ReplayBoxes<G>,
    innermost_group: Option<GroupKind>,
    job_is_all_over: bool,
    display_eq_no: bool,
    shown_mode: &mut Option<Mode>,
    diagnostics: &mut Vec<PendingDiagnostic<G>>,
) -> Result<ScannedOperation, ExecError> {
    if let Some(cursor) = command.cursor {
        processor.resume_delivery_cursor(cursor);
    }
    match command
        .phase
        .expect("operation frame owns its scalar phase")
    {
        PreflightCommandPhase::Settled | PreflightCommandPhase::Raw => {
            processor.resume_current_command(command.current());
            dispatch_main_control_command(
                processor,
                command,
                cold,
                mode,
                boxes,
                innermost_group,
                job_is_all_over,
                display_eq_no,
                shown_mode,
                diagnostics,
                None,
                true,
            )
        }
        PreflightCommandPhase::Expanding { main_loop } => {
            prepare_command_trace(processor, mode, *shown_mode);
            settle_preflight_step(
                processor,
                command,
                cold,
                main_loop,
                mode,
                boxes,
                innermost_group,
                job_is_all_over,
                display_eq_no,
                shown_mode,
                diagnostics,
            )
        }
        PreflightCommandPhase::OperationScan => {
            processor.resume_current_command(command.current());
            let phase = command
                .operation_scan
                .take()
                .expect("operation-scan phase owns its exact scalar state");
            let mut suspended = None;
            let result =
                resume_pending_operation_scan(processor, command, cold, phase, &mut suspended);
            if let Err(error) = &result
                && execution_error_needs_command_retry(error)
                && let Some(phase) = suspended
            {
                let child = processor
                    .take_scanner_resume()
                    .expect("a resuspended scalar scan retains its exact child capability");
                command.retain_operation_scan(processor.delivery_cursor(), phase, child);
            }
            if result.is_ok() {
                command.phase = Some(PreflightCommandPhase::Settled);
                command.operation_scan = None;
            }
            result
        }
        PreflightCommandPhase::PrefixScan {
            global,
            flags,
            alignment,
            set_box_allowed,
        } => {
            processor.resume_current_command(command.current());
            let origin = command.current().origin();
            dispatch_main_control_command_inner(
                processor,
                command,
                cold,
                mode,
                boxes,
                innermost_group,
                job_is_all_over,
                display_eq_no,
                shown_mode,
                diagnostics,
                alignment,
                set_box_allowed,
                Some((global, flags)),
            )
            .map_err(|error| error.capture_command_origin(origin))
        }
        PreflightCommandPhase::PrefixedCommandScan {
            global,
            flags,
            set_box_allowed,
        } => {
            processor.resume_current_command(command.current());
            let mut suspended_operation_scan = None;
            let result = scan_command(
                processor,
                command,
                cold,
                global,
                flags,
                mode,
                boxes,
                innermost_group,
                job_is_all_over,
                display_eq_no,
                set_box_allowed,
                shown_mode,
                &mut suspended_operation_scan,
            );
            if let Err(error) = &result
                && execution_error_needs_command_retry(error)
            {
                let child = processor
                    .take_scanner_resume()
                    .expect("a resuspended prefixed command retains its exact scanner child");
                if let Some(phase) = suspended_operation_scan {
                    command.retain_operation_scan(processor.delivery_cursor(), phase, child);
                } else {
                    command.phase = Some(PreflightCommandPhase::PrefixedCommandScan {
                        global,
                        flags,
                        set_box_allowed,
                    });
                    command.retain_scanner(processor.delivery_cursor(), Some(child));
                }
            }
            if result.is_ok() {
                command.phase = Some(PreflightCommandPhase::Settled);
                command.operation_scan = None;
            }
            result
        }
        PreflightCommandPhase::ImmediatePdfRetry(primitive) => {
            let operation = match primitive {
                UnexpandablePrimitive::PdfObject => ColdOperation::<G>::ImmediateExtension(
                    ImmediateExtension::PdfObject(
                        processor.scan_pdf_object_request().map_err(command_error)?,
                    )
                    .into(),
                ),
                UnexpandablePrimitive::PdfXForm => ColdOperation::<G>::ImmediateExtension(
                    ImmediateExtension::PdfForm(
                        processor
                            .scan_pdf_form_request(UnexpandablePrimitive::PdfXForm)
                            .map_err(command_error)?,
                    )
                    .into(),
                ),
                UnexpandablePrimitive::PdfXImage => ColdOperation::<G>::PdfXImage {
                    request: processor
                        .scan_pdf_image_request()
                        .map_err(command_error)?
                        .into(),
                    resource: PdfImageResource::Unavailable,
                },
                _ => unreachable!("only immediate PDF retries reach this delivery"),
            };
            write_cold_scan!(cold, operation);
            command.mark_resident_cold(cold);
            Ok(ScannedOperation::Cold)
        }
    }
}

#[allow(clippy::too_many_arguments)] // carries command-owned replay facts
pub(super) fn scan_step<G>(
    processor: &mut CommandProcessor<'_, '_, G>,
    mode: Mode,
    boxes: &ReplayBoxes<G>,
    innermost_group: Option<GroupKind>,
    job_is_all_over: bool,
    display_eq_no: bool,
    main_loop_active: bool,
    shown_mode: &mut Option<Mode>,
    diagnostics: &mut Vec<PendingDiagnostic<G>>,
    command_owner: &mut CommandEpisode<G>,
    cold: &mut ColdOperationSlot<G>,
) -> Result<ScannedOperation, ExecError> {
    // TeX82 §1030 has two fetch labels, not one. `big_switch` uses
    // `get_x_token`; §1034's inner character loop instead re-enters at
    // §1038's `main_loop_lookahead`, whose bare `get_next` is what keeps a
    // run of adjacent characters from being delivered through expansion.
    prepare_command_trace(processor, mode, *shown_mode);
    let mut destination = None;
    let delivery = if main_loop_active {
        processor.main_loop_lookahead_into(&mut destination)
    } else {
        processor.get_x_token_with_replay_completion_into(&mut destination)
    };
    match delivery.map_err(command_error)? {
        tex_command::DeliveryStatus::End => {
            return Ok(retain_cold_operation(
                command_owner,
                cold,
                ColdOperation::<G>::EndOfInput,
            ));
        }
        tex_command::DeliveryStatus::ReplayCompleted(episode) => {
            return Ok(retain_cold_operation(
                command_owner,
                cold,
                ColdOperation::<G>::ReplayCompleted(episode),
            ));
        }
        tex_command::DeliveryStatus::Command => {}
        _ => unreachable!("main-control delivery has no alignment event"),
    };
    let command = destination.expect("command status initializes destination");
    // TeX82 §§1034/1038 keeps a fetched character inside `main_loop`;
    // it reaches neither `reswitch` nor §1030's command trace. A
    // non-character fetched by the same lookahead does go to `reswitch` and
    // must retain the ordinary trace boundary.
    let continues_main_loop = main_loop_active
        && matches!(
            command.meaning(),
            ResolvedMeaning::Static(
                Meaning::CharToken {
                    cat: Catcode::Letter | Catcode::Other,
                    ..
                } | Meaning::CharGiven(_)
                    | Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Char)
            )
        );
    if !continues_main_loop {
        report_main_control_command_trace(processor, mode, &command, boxes, shown_mode);
    }
    if main_loop_active
        && matches!(mode, Mode::Horizontal | Mode::RestrictedHorizontal)
        && matches!(
            command.meaning(),
            ResolvedMeaning::Static(Meaning::UnexpandablePrimitive(
                UnexpandablePrimitive::NoBoundary
            ))
        )
    {
        return Ok(retain_cold_operation(
            command_owner,
            cold,
            ColdOperation::<G>::NoBoundary {
                suppress_right: true,
            },
        ));
    }
    command_owner.admit_settled(command, None);
    dispatch_main_control_command(
        processor,
        command_owner,
        cold,
        mode,
        boxes,
        innermost_group,
        job_is_all_over,
        display_eq_no,
        shown_mode,
        diagnostics,
        None,
        true,
    )
}

pub(super) fn execution_error_needs_command_retry(error: &ExecError) -> bool {
    match error {
        ExecError::Captured { error, .. } => execution_error_needs_command_retry(error),
        ExecError::MissingInput { .. }
        | ExecError::MissingInputProbe { .. }
        | ExecError::MissingFont { .. }
        | ExecError::MissingPdfImage { .. } => true,
        _ => false,
    }
}

pub(super) fn scan_count_register_assignment<G>(
    cold: &mut ColdOperationSlot<G>,
    processor: &mut CommandProcessor<'_, '_, G>,
    scalar: &mut tex_command::ScalarScanFrame,
    mut index: Option<u16>,
    global: bool,
    phase: RegisterAssignmentScanPhase,
    suspended: &mut Option<PendingOperationScanPhase>,
) -> Result<(), ExecError> {
    if phase == RegisterAssignmentScanPhase::RegisterIndex {
        let scalar_phase = PendingOperationScanPhase::Count {
            index,
            global,
            phase: RegisterAssignmentScanPhase::RegisterIndex,
        };
        let status = processor.scan_profile_register_index_into(scalar);
        index = Some(take_operation_scalar!(
            scalar,
            status,
            scalar_phase,
            suspended,
            take_register
        ));
    }
    if phase != RegisterAssignmentScanPhase::Value {
        let scalar_phase = PendingOperationScanPhase::Count {
            index,
            global,
            phase: RegisterAssignmentScanPhase::OptionalEquals,
        };
        let status = processor.scan_optional_equals_into(scalar);
        let _ = take_operation_scalar!(scalar, status, scalar_phase, suspended, take_boolean);
    }
    let scalar_phase = PendingOperationScanPhase::Count {
        index,
        global,
        phase: RegisterAssignmentScanPhase::Value,
    };
    let status = processor.scan_integer_into(scalar);
    let value = take_operation_scalar!(scalar, status, scalar_phase, suspended, take_integer).value;
    complete_cold_scan!(
        cold,
        ColdOperation::Count {
            index: index.expect("count assignment retains its completed register index"),
            value,
            global,
        }
    )
}

pub(super) fn scan_dimension_register_assignment<G>(
    cold: &mut ColdOperationSlot<G>,
    processor: &mut CommandProcessor<'_, '_, G>,
    scalar: &mut tex_command::ScalarScanFrame,
    mut index: Option<u16>,
    global: bool,
    phase: RegisterAssignmentScanPhase,
    suspended: &mut Option<PendingOperationScanPhase>,
) -> Result<(), ExecError> {
    if phase == RegisterAssignmentScanPhase::RegisterIndex {
        let scalar_phase = PendingOperationScanPhase::Dimension {
            index,
            global,
            phase: RegisterAssignmentScanPhase::RegisterIndex,
        };
        let status = processor.scan_profile_register_index_into(scalar);
        index = Some(take_operation_scalar!(
            scalar,
            status,
            scalar_phase,
            suspended,
            take_register
        ));
    }
    if phase != RegisterAssignmentScanPhase::Value {
        let scalar_phase = PendingOperationScanPhase::Dimension {
            index,
            global,
            phase: RegisterAssignmentScanPhase::OptionalEquals,
        };
        let status = processor.scan_optional_equals_into(scalar);
        let _ = take_operation_scalar!(scalar, status, scalar_phase, suspended, take_boolean);
    }
    let scalar_phase = PendingOperationScanPhase::Dimension {
        index,
        global,
        phase: RegisterAssignmentScanPhase::Value,
    };
    let status = processor.scan_dimension_into(scalar);
    let value =
        take_operation_scalar!(scalar, status, scalar_phase, suspended, take_dimension).value;
    complete_cold_scan!(
        cold,
        ColdOperation::Dimen {
            index: index.expect("dimension assignment retains its completed register index"),
            value,
            global,
        }
    )
}

#[allow(clippy::too_many_arguments)] // carries resident cold/scalar suspension destinations
pub(super) fn scan_glue_register_assignment<G>(
    cold: &mut ColdOperationSlot<G>,
    processor: &mut CommandProcessor<'_, '_, G>,
    scalar: &mut tex_command::ScalarScanFrame,
    mut index: Option<u16>,
    global: bool,
    mu: bool,
    phase: RegisterAssignmentScanPhase,
    suspended: &mut Option<PendingOperationScanPhase>,
) -> Result<(), ExecError> {
    if phase == RegisterAssignmentScanPhase::RegisterIndex {
        let scalar_phase = PendingOperationScanPhase::Glue {
            index,
            global,
            mu,
            phase: RegisterAssignmentScanPhase::RegisterIndex,
        };
        let status = processor.scan_profile_register_index_into(scalar);
        index = Some(take_operation_scalar!(
            scalar,
            status,
            scalar_phase,
            suspended,
            take_register
        ));
    }
    if phase != RegisterAssignmentScanPhase::Value {
        let scalar_phase = PendingOperationScanPhase::Glue {
            index,
            global,
            mu,
            phase: RegisterAssignmentScanPhase::OptionalEquals,
        };
        let status = processor.scan_optional_equals_into(scalar);
        let _ = take_operation_scalar!(scalar, status, scalar_phase, suspended, take_boolean);
    }
    let scalar_phase = PendingOperationScanPhase::Glue {
        index,
        global,
        mu,
        phase: RegisterAssignmentScanPhase::Value,
    };
    let status = processor.scan_glue_into(mu, scalar);
    let value = take_operation_scalar!(scalar, status, scalar_phase, suspended, take_glue).value;
    let source_identity = processor.scanned_glue_identity();
    let source_register = processor.scanned_glue_register();
    let index = index.expect("glue assignment retains its completed register index");
    if mu {
        complete_cold_scan!(
            cold,
            ColdOperation::Muskip {
                index,
                value,
                source_identity,
                source_register,
                redundant: false,
                reassigning: false,
                global,
            }
        )
    } else {
        complete_cold_scan!(
            cold,
            ColdOperation::Skip {
                index,
                value,
                source_identity,
                source_register,
                redundant: false,
                reassigning: false,
                global,
            }
        )
    }
}

#[allow(clippy::too_many_arguments)] // carries resident cold/scalar suspension destinations
pub(super) fn scan_box_dimension_assignment<G>(
    cold: &mut ColdOperationSlot<G>,
    processor: &mut CommandProcessor<'_, '_, G>,
    scalar: &mut tex_command::ScalarScanFrame,
    mut index: Option<u16>,
    dimension: tex_state::BoxDimension,
    global: bool,
    phase: RegisterAssignmentScanPhase,
    suspended: &mut Option<PendingOperationScanPhase>,
) -> Result<(), ExecError> {
    if phase == RegisterAssignmentScanPhase::RegisterIndex {
        let scalar_phase = PendingOperationScanPhase::BoxDimension {
            index,
            dimension,
            global,
            phase: RegisterAssignmentScanPhase::RegisterIndex,
        };
        let status = processor.scan_profile_register_index_into(scalar);
        index = Some(take_operation_scalar!(
            scalar,
            status,
            scalar_phase,
            suspended,
            take_register
        ));
    }
    if phase != RegisterAssignmentScanPhase::Value {
        let scalar_phase = PendingOperationScanPhase::BoxDimension {
            index,
            dimension,
            global,
            phase: RegisterAssignmentScanPhase::OptionalEquals,
        };
        let status = processor.scan_optional_equals_into(scalar);
        let _ = take_operation_scalar!(scalar, status, scalar_phase, suspended, take_boolean);
    }
    let scalar_phase = PendingOperationScanPhase::BoxDimension {
        index,
        dimension,
        global,
        phase: RegisterAssignmentScanPhase::Value,
    };
    let status = processor.scan_dimension_into(scalar);
    let value =
        take_operation_scalar!(scalar, status, scalar_phase, suspended, take_dimension).value;
    complete_cold_scan!(
        cold,
        ColdOperation::BoxDimensionAssignment {
            index: index.expect("box-dimension assignment retains its completed register index"),
            dimension,
            value,
            global,
        }
    )
}

/// Retains a non-scalar structured scanner child at its typed operation
/// phase. Scalar families use the frame-owned destination directly.
pub(super) fn retain_operation_child<G, T>(
    processor: &mut CommandProcessor<'_, '_, G>,
    scan: tex_command::RetainedScalarScan<G, T>,
    phase: PendingOperationScanPhase,
    suspended: &mut Option<PendingOperationScanPhase>,
) -> Result<T, ExecError> {
    match scan {
        tex_command::RetainedScalarScan::Complete(value) => {
            *suspended = None;
            Ok(value)
        }
        tex_command::RetainedScalarScan::Suspended { error, child } => {
            processor.install_scanner_resume(Some(child));
            *suspended = Some(phase);
            Err(command_error(error))
        }
        tex_command::RetainedScalarScan::Failed(error) => {
            *suspended = None;
            Err(command_error(error))
        }
    }
}

#[allow(clippy::too_many_arguments)] // carries resident cold/scalar suspension destinations
pub(super) fn scan_unary_scalar_operation<G>(
    cold: &mut ColdOperationSlot<G>,
    processor: &mut CommandProcessor<'_, '_, G>,
    scalar: &mut tex_command::ScalarScanFrame,
    meaning: Meaning,
    global: bool,
    origin: tex_state::token::OriginId,
    phase: UnaryOperationScanPhase,
    suspended: &mut Option<PendingOperationScanPhase>,
) -> Result<(), ExecError> {
    let has_optional_equals = matches!(
        meaning,
        Meaning::UnexpandablePrimitive(
            UnexpandablePrimitive::PrevDepth
                | UnexpandablePrimitive::InteractionMode
                | UnexpandablePrimitive::SpaceFactor
                | UnexpandablePrimitive::PrevGraf
        ) | Meaning::IntParam(_)
            | Meaning::DimenParam(_)
            | Meaning::PageDimension(_)
            | Meaning::PageInteger(_)
    );
    if phase == UnaryOperationScanPhase::OptionalEquals && has_optional_equals {
        let scalar_phase = PendingOperationScanPhase::Unary {
            meaning,
            global,
            origin,
            phase: UnaryOperationScanPhase::OptionalEquals,
        };
        let status = processor.scan_optional_equals_into(scalar);
        let _ = take_operation_scalar!(scalar, status, scalar_phase, suspended, take_boolean);
    }
    let scalar_phase = PendingOperationScanPhase::Unary {
        meaning,
        global,
        origin,
        phase: UnaryOperationScanPhase::Value,
    };
    match meaning {
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::HSkip) => {
            let status = processor.scan_glue_into(false, scalar);
            let value =
                take_operation_scalar!(scalar, status, scalar_phase, suspended, take_glue).value;
            complete_cold_scan!(cold, ColdOperation::HorizontalSkip { value })
        }
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::VSkip) => {
            let status = processor.scan_glue_into(false, scalar);
            let value =
                take_operation_scalar!(scalar, status, scalar_phase, suspended, take_glue).value;
            complete_cold_scan!(cold, ColdOperation::VerticalSkip { value })
        }
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Kern) => {
            let status = processor.scan_dimension_into(scalar);
            let amount =
                take_operation_scalar!(scalar, status, scalar_phase, suspended, take_dimension)
                    .value;
            complete_cold_scan!(cold, ColdOperation::Kern { amount })
        }
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::PrevDepth) => {
            let status = processor.scan_dimension_into(scalar);
            let value =
                take_operation_scalar!(scalar, status, scalar_phase, suspended, take_dimension)
                    .value;
            complete_cold_scan!(cold, ColdOperation::PrevDepth { value })
        }
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Penalty) => {
            let status = processor.scan_integer_into(scalar);
            let amount =
                take_operation_scalar!(scalar, status, scalar_phase, suspended, take_integer).value;
            complete_cold_scan!(cold, ColdOperation::Penalty { amount })
        }
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::PdfRefXImage) => {
            let status = processor.scan_integer_into(scalar);
            let object =
                take_operation_scalar!(scalar, status, scalar_phase, suspended, take_integer).value;
            complete_cold_scan!(cold, ColdOperation::PdfRefXImage { object })
        }
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::PdfSetRandomSeed) => {
            let status = processor.scan_integer_into(scalar);
            let seed =
                take_operation_scalar!(scalar, status, scalar_phase, suspended, take_integer).value;
            complete_cold_scan!(
                cold,
                ColdOperation::PdfSetRandomSeed {
                    seed: seed.saturating_abs(),
                }
            )
        }
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::SetLanguage) => {
            let status = processor.scan_integer_into(scalar);
            let language =
                take_operation_scalar!(scalar, status, scalar_phase, suspended, take_integer).value;
            complete_cold_scan!(cold, ColdOperation::SetLanguage { language })
        }
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::InteractionMode) => {
            let status = processor.scan_integer_into(scalar);
            let value =
                take_operation_scalar!(scalar, status, scalar_phase, suspended, take_integer).value;
            complete_cold_scan!(
                cold,
                ColdOperation::SetInteractionModeValue {
                    value,
                    context: processor.diagnostic_context_coordinate(),
                }
            )
        }
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::SpaceFactor) => {
            let status = processor.scan_integer_into(scalar);
            let value =
                take_operation_scalar!(scalar, status, scalar_phase, suspended, take_integer).value;
            complete_cold_scan!(cold, ColdOperation::SpaceFactor { value })
        }
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::PrevGraf) => {
            let status = processor.scan_integer_into(scalar);
            let value =
                take_operation_scalar!(scalar, status, scalar_phase, suspended, take_integer).value;
            complete_cold_scan!(cold, ColdOperation::PrevGraf { value })
        }
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Char) => {
            let status = processor.scan_integer_into(scalar);
            let value =
                take_operation_scalar!(scalar, status, scalar_phase, suspended, take_integer).value;
            complete_cold_scan!(
                cold,
                ColdOperation::CharacterCode {
                    value,
                    origin,
                    suppress_left_boundary: false,
                }
            )
        }
        Meaning::IntParam(index) => {
            let status = processor.scan_integer_into(scalar);
            let value =
                take_operation_scalar!(scalar, status, scalar_phase, suspended, take_integer).value;
            complete_cold_scan!(
                cold,
                ColdOperation::IntParam {
                    index,
                    value,
                    global,
                }
            )
        }
        Meaning::DimenParam(index) => {
            let status = processor.scan_dimension_into(scalar);
            let value =
                take_operation_scalar!(scalar, status, scalar_phase, suspended, take_dimension)
                    .value;
            complete_cold_scan!(
                cold,
                ColdOperation::DimenParam {
                    index,
                    value,
                    global,
                }
            )
        }
        Meaning::PageDimension(dimension) => {
            let status = processor.scan_dimension_into(scalar);
            let value =
                take_operation_scalar!(scalar, status, scalar_phase, suspended, take_dimension)
                    .value;
            complete_cold_scan!(cold, ColdOperation::PageDimension { dimension, value })
        }
        Meaning::PageInteger(integer) => {
            let status = processor.scan_integer_into(scalar);
            let value =
                take_operation_scalar!(scalar, status, scalar_phase, suspended, take_integer).value;
            complete_cold_scan!(cold, ColdOperation::PageInteger { integer, value })
        }
        _ => unreachable!("unary scalar descriptor restricts command meanings"),
    }
}

pub(super) fn scan_paragraph_shape_assignment<G>(
    cold: &mut ColdOperationSlot<G>,
    processor: &mut CommandProcessor<'_, '_, G>,
    scalar: &mut tex_command::ScalarScanFrame,
    global: bool,
    phase: ParagraphShapeScanPhase,
    suspended: &mut Option<PendingOperationScanPhase>,
) -> Result<(), ExecError> {
    let phase = if matches!(phase, ParagraphShapeScanPhase::OptionalEquals) {
        let scalar_phase = PendingOperationScanPhase::ParagraphShape {
            global,
            phase: ParagraphShapeScanPhase::OptionalEquals,
        };
        let status = processor.scan_optional_equals_into(scalar);
        let _ = take_operation_scalar!(scalar, status, scalar_phase, suspended, take_boolean);
        ParagraphShapeScanPhase::Count
    } else {
        phase
    };
    let phase = if matches!(phase, ParagraphShapeScanPhase::Count) {
        let scalar_phase = PendingOperationScanPhase::ParagraphShape {
            global,
            phase: ParagraphShapeScanPhase::Count,
        };
        let status = processor.scan_integer_into(scalar);
        let count = take_operation_scalar!(scalar, status, scalar_phase, suspended, take_integer)
            .value
            .max(0) as usize;
        let mut lines = Vec::new();
        lines
            .try_reserve_exact(count)
            .map_err(|_| ExecError::ArithmeticOverflow)?;
        ParagraphShapeScanPhase::Indent {
            remaining: count,
            lines,
        }
    } else {
        phase
    };
    let (mut remaining, mut lines, mut retained_indent) = match phase {
        ParagraphShapeScanPhase::Indent { remaining, lines } => (remaining, lines, None),
        ParagraphShapeScanPhase::Width {
            remaining,
            lines,
            indent,
        } => (remaining, lines, Some(indent)),
        ParagraphShapeScanPhase::OptionalEquals | ParagraphShapeScanPhase::Count => unreachable!(),
    };
    while remaining != 0 {
        let indent = match retained_indent.take() {
            Some(indent) => indent,
            None => {
                let status = processor.scan_dimension_into(scalar);
                take_operation_scalar!(
                    scalar,
                    status,
                    PendingOperationScanPhase::ParagraphShape {
                        global,
                        phase: ParagraphShapeScanPhase::Indent { remaining, lines },
                    },
                    suspended,
                    take_dimension
                )
                .value
            }
        };
        let status = processor.scan_dimension_into(scalar);
        let width = take_operation_scalar!(
            scalar,
            status,
            PendingOperationScanPhase::ParagraphShape {
                global,
                phase: ParagraphShapeScanPhase::Width {
                    remaining,
                    lines,
                    indent,
                },
            },
            suspended,
            take_dimension
        )
        .value;
        lines.push(ParagraphShapeLine { indent, width });
        remaining -= 1;
    }
    complete_cold_scan!(cold, ColdOperation::ParagraphShape { lines, global })
}

pub(super) fn scan_penalty_array_assignment<G>(
    cold: &mut ColdOperationSlot<G>,
    processor: &mut CommandProcessor<'_, '_, G>,
    scalar: &mut tex_command::ScalarScanFrame,
    kind: tex_state::PenaltyArrayKind,
    global: bool,
    phase: PenaltyArrayScanPhase,
    suspended: &mut Option<PendingOperationScanPhase>,
) -> Result<(), ExecError> {
    let phase = if matches!(phase, PenaltyArrayScanPhase::OptionalEquals) {
        let scalar_phase = PendingOperationScanPhase::PenaltyArray {
            kind,
            global,
            phase: PenaltyArrayScanPhase::OptionalEquals,
        };
        let status = processor.scan_optional_equals_into(scalar);
        let _ = take_operation_scalar!(scalar, status, scalar_phase, suspended, take_boolean);
        PenaltyArrayScanPhase::Count
    } else {
        phase
    };
    let phase = if matches!(phase, PenaltyArrayScanPhase::Count) {
        let scalar_phase = PendingOperationScanPhase::PenaltyArray {
            kind,
            global,
            phase: PenaltyArrayScanPhase::Count,
        };
        let status = processor.scan_integer_into(scalar);
        let count = take_operation_scalar!(scalar, status, scalar_phase, suspended, take_integer)
            .value
            .max(0) as usize;
        let mut values = Vec::new();
        values
            .try_reserve_exact(count)
            .map_err(|_| ExecError::ArithmeticOverflow)?;
        PenaltyArrayScanPhase::Value {
            remaining: count,
            values,
        }
    } else {
        phase
    };
    let PenaltyArrayScanPhase::Value {
        mut remaining,
        mut values,
    } = phase
    else {
        unreachable!()
    };
    while remaining != 0 {
        let status = processor.scan_integer_into(scalar);
        let value = take_operation_scalar!(
            scalar,
            status,
            PendingOperationScanPhase::PenaltyArray {
                kind,
                global,
                phase: PenaltyArrayScanPhase::Value { remaining, values },
            },
            suspended,
            take_integer
        )
        .value;
        values.push(value);
        remaining -= 1;
    }
    complete_cold_scan!(
        cold,
        ColdOperation::PenaltyArray {
            kind,
            values,
            global,
        }
    )
}

pub(super) fn scan_font_dimen_assignment<G>(
    cold: &mut ColdOperationSlot<G>,
    processor: &mut CommandProcessor<'_, '_, G>,
    scalar: &mut tex_command::ScalarScanFrame,
    phase: FontDimenScanPhase,
    suspended: &mut Option<PendingOperationScanPhase>,
) -> Result<(), ExecError> {
    let phase = if matches!(phase, FontDimenScanPhase::Number) {
        let status = processor.scan_integer_into(scalar);
        let number = take_operation_scalar!(
            scalar,
            status,
            PendingOperationScanPhase::FontDimen(FontDimenScanPhase::Number),
            suspended,
            take_integer
        )
        .value;
        FontDimenScanPhase::Font { number }
    } else {
        phase
    };
    let phase = match phase {
        FontDimenScanPhase::Font { number } => {
            let status = processor.scan_font_selector_into(scalar);
            let font = take_operation_scalar!(
                scalar,
                status,
                PendingOperationScanPhase::FontDimen(FontDimenScanPhase::Font { number }),
                suspended,
                take_font
            );
            let recovery_context =
                (!processor.font_dimen_writable(font, number)).then(|| processor.error_context());
            FontDimenScanPhase::OptionalEquals {
                number,
                font,
                recovery_context,
            }
        }
        phase => phase,
    };
    let phase = match phase {
        FontDimenScanPhase::OptionalEquals {
            number,
            font,
            recovery_context,
        } => {
            let status = processor.scan_optional_equals_into(scalar);
            let _ = take_operation_scalar!(
                scalar,
                status,
                PendingOperationScanPhase::FontDimen(FontDimenScanPhase::OptionalEquals {
                    number,
                    font,
                    recovery_context,
                }),
                suspended,
                take_boolean
            );
            FontDimenScanPhase::Value {
                number,
                font,
                recovery_context,
            }
        }
        phase => phase,
    };
    let FontDimenScanPhase::Value {
        number,
        font,
        recovery_context,
    } = phase
    else {
        unreachable!()
    };
    let status = processor.scan_dimension_into(scalar);
    let value = take_operation_scalar!(
        scalar,
        status,
        PendingOperationScanPhase::FontDimen(FontDimenScanPhase::Value {
            number,
            font,
            recovery_context,
        }),
        suspended,
        take_dimension
    );
    complete_cold_scan!(
        cold,
        ColdOperation::FontDimen {
            font,
            number,
            value: value.value,
            recovery_context,
        }
    )
}

pub(super) fn scan_font_integer_assignment<G>(
    cold: &mut ColdOperationSlot<G>,
    processor: &mut CommandProcessor<'_, '_, G>,
    scalar: &mut tex_command::ScalarScanFrame,
    primitive: UnexpandablePrimitive,
    phase: FontIntegerScanPhase,
    suspended: &mut Option<PendingOperationScanPhase>,
) -> Result<(), ExecError> {
    let phase = if matches!(phase, FontIntegerScanPhase::Font) {
        let status = processor.scan_font_selector_into(scalar);
        let font = take_operation_scalar!(
            scalar,
            status,
            PendingOperationScanPhase::FontInteger {
                primitive,
                phase: FontIntegerScanPhase::Font,
            },
            suspended,
            take_font
        );
        FontIntegerScanPhase::OptionalEquals { font }
    } else {
        phase
    };
    let phase = match phase {
        FontIntegerScanPhase::OptionalEquals { font } => {
            let status = processor.scan_optional_equals_into(scalar);
            let _ = take_operation_scalar!(
                scalar,
                status,
                PendingOperationScanPhase::FontInteger {
                    primitive,
                    phase: FontIntegerScanPhase::OptionalEquals { font },
                },
                suspended,
                take_boolean
            );
            FontIntegerScanPhase::Value { font }
        }
        phase => phase,
    };
    let FontIntegerScanPhase::Value { font } = phase else {
        unreachable!()
    };
    let status = processor.scan_integer_into(scalar);
    let value = take_operation_scalar!(
        scalar,
        status,
        PendingOperationScanPhase::FontInteger {
            primitive,
            phase: FontIntegerScanPhase::Value { font },
        },
        suspended,
        take_integer
    )
    .value;
    complete_cold_scan!(
        cold,
        ColdOperation::FontInteger {
            font,
            skew: primitive == UnexpandablePrimitive::SkewChar,
            value,
        }
    )
}

pub(super) fn scan_code_table_assignment<G>(
    cold: &mut ColdOperationSlot<G>,
    processor: &mut CommandProcessor<'_, '_, G>,
    scalar: &mut tex_command::ScalarScanFrame,
    primitive: UnexpandablePrimitive,
    global: bool,
    phase: CodeTableScanPhase,
    suspended: &mut Option<PendingOperationScanPhase>,
) -> Result<(), ExecError> {
    let phase = if matches!(phase, CodeTableScanPhase::Character) {
        let status =
            processor.scan_restricted_integer_into(RestrictedIntegerClass::CharacterCode, scalar);
        let character = take_operation_scalar!(
            scalar,
            status,
            PendingOperationScanPhase::CodeTable {
                primitive,
                global,
                phase: CodeTableScanPhase::Character,
            },
            suspended,
            take_restricted
        )
        .value;
        let character =
            char::from_u32(character as u32).expect("scan_char_num returns a valid character");
        CodeTableScanPhase::OptionalEquals { character }
    } else {
        phase
    };
    let phase = match phase {
        CodeTableScanPhase::OptionalEquals { character } => {
            let status = processor.scan_optional_equals_into(scalar);
            let _ = take_operation_scalar!(
                scalar,
                status,
                PendingOperationScanPhase::CodeTable {
                    primitive,
                    global,
                    phase: CodeTableScanPhase::OptionalEquals { character },
                },
                suspended,
                take_boolean
            );
            CodeTableScanPhase::Value { character }
        }
        phase => phase,
    };
    let CodeTableScanPhase::Value { character } = phase else {
        unreachable!()
    };
    let status = processor.scan_integer_into(scalar);
    let value = take_operation_scalar!(
        scalar,
        status,
        PendingOperationScanPhase::CodeTable {
            primitive,
            global,
            phase: CodeTableScanPhase::Value { character },
        },
        suspended,
        take_integer
    )
    .value;
    complete_cold_scan!(
        cold,
        ColdOperation::CodeTable {
            primitive,
            character,
            value,
            global,
        }
    )
}

pub(super) fn scan_pdf_font_code_assignment<G>(
    cold: &mut ColdOperationSlot<G>,
    processor: &mut CommandProcessor<'_, '_, G>,
    scalar: &mut tex_command::ScalarScanFrame,
    primitive: UnexpandablePrimitive,
    phase: PdfFontCodeScanPhase,
    suspended: &mut Option<PendingOperationScanPhase>,
) -> Result<(), ExecError> {
    let phase = if matches!(phase, PdfFontCodeScanPhase::Font) {
        let status = processor.scan_font_selector_into(scalar);
        let font = take_operation_scalar!(
            scalar,
            status,
            PendingOperationScanPhase::PdfFontCode {
                primitive,
                phase: PdfFontCodeScanPhase::Font,
            },
            suspended,
            take_font
        );
        PdfFontCodeScanPhase::Character { font }
    } else {
        phase
    };
    let phase = match phase {
        PdfFontCodeScanPhase::Character { font } => {
            let status = processor
                .scan_restricted_integer_into(RestrictedIntegerClass::CharacterCode, scalar);
            let character = take_operation_scalar!(
                scalar,
                status,
                PendingOperationScanPhase::PdfFontCode {
                    primitive,
                    phase: PdfFontCodeScanPhase::Character { font },
                },
                suspended,
                take_restricted
            )
            .value;
            PdfFontCodeScanPhase::OptionalEquals {
                font,
                character: u8::try_from(character)
                    .expect("pdfTeX character scanner is byte bounded"),
            }
        }
        phase => phase,
    };
    let phase = match phase {
        PdfFontCodeScanPhase::OptionalEquals { font, character } => {
            let status = processor.scan_optional_equals_into(scalar);
            let _ = take_operation_scalar!(
                scalar,
                status,
                PendingOperationScanPhase::PdfFontCode {
                    primitive,
                    phase: PdfFontCodeScanPhase::OptionalEquals { font, character },
                },
                suspended,
                take_boolean
            );
            PdfFontCodeScanPhase::Value { font, character }
        }
        phase => phase,
    };
    let PdfFontCodeScanPhase::Value { font, character } = phase else {
        unreachable!()
    };
    let status = processor.scan_integer_into(scalar);
    let value = take_operation_scalar!(
        scalar,
        status,
        PendingOperationScanPhase::PdfFontCode {
            primitive,
            phase: PdfFontCodeScanPhase::Value { font, character },
        },
        suspended,
        take_integer
    )
    .value;
    complete_cold_scan!(
        cold,
        ColdOperation::PdfFontCode {
            table: pdf_font_code_table(primitive),
            font,
            character,
            value,
        }
    )
}

pub(super) fn scan_pdf_font_expand_assignment<G>(
    cold: &mut ColdOperationSlot<G>,
    processor: &mut CommandProcessor<'_, '_, G>,
    scalar: &mut tex_command::ScalarScanFrame,
    phase: PdfFontExpandScanPhase,
    suspended: &mut Option<PendingOperationScanPhase>,
) -> Result<(), ExecError> {
    let phase = if matches!(phase, PdfFontExpandScanPhase::Font) {
        let status = processor.scan_font_selector_into(scalar);
        let font = take_operation_scalar!(
            scalar,
            status,
            PendingOperationScanPhase::PdfFontExpand(PdfFontExpandScanPhase::Font),
            suspended,
            take_font
        );
        PdfFontExpandScanPhase::OptionalEquals { font }
    } else {
        phase
    };
    let phase = match phase {
        PdfFontExpandScanPhase::OptionalEquals { font } => {
            let status = processor.scan_optional_equals_into(scalar);
            let _ = take_operation_scalar!(
                scalar,
                status,
                PendingOperationScanPhase::PdfFontExpand(PdfFontExpandScanPhase::OptionalEquals {
                    font,
                }),
                suspended,
                take_boolean
            );
            PdfFontExpandScanPhase::Stretch { font }
        }
        phase => phase,
    };
    let phase = match phase {
        PdfFontExpandScanPhase::Stretch { font } => {
            let status = processor.scan_integer_into(scalar);
            let stretch = take_operation_scalar!(
                scalar,
                status,
                PendingOperationScanPhase::PdfFontExpand(PdfFontExpandScanPhase::Stretch { font }),
                suspended,
                take_integer
            )
            .value;
            PdfFontExpandScanPhase::Shrink { font, stretch }
        }
        phase => phase,
    };
    let phase = match phase {
        PdfFontExpandScanPhase::Shrink { font, stretch } => {
            let status = processor.scan_integer_into(scalar);
            let shrink = take_operation_scalar!(
                scalar,
                status,
                PendingOperationScanPhase::PdfFontExpand(PdfFontExpandScanPhase::Shrink {
                    font,
                    stretch,
                }),
                suspended,
                take_integer
            )
            .value;
            PdfFontExpandScanPhase::Step {
                font,
                stretch,
                shrink,
            }
        }
        phase => phase,
    };
    let phase = match phase {
        PdfFontExpandScanPhase::Step {
            font,
            stretch,
            shrink,
        } => {
            let status = processor.scan_integer_into(scalar);
            let step = take_operation_scalar!(
                scalar,
                status,
                PendingOperationScanPhase::PdfFontExpand(PdfFontExpandScanPhase::Step {
                    font,
                    stretch,
                    shrink,
                }),
                suspended,
                take_integer
            )
            .value;
            PdfFontExpandScanPhase::AutoExpand {
                font,
                stretch,
                shrink,
                step,
            }
        }
        phase => phase,
    };
    let PdfFontExpandScanPhase::AutoExpand {
        font,
        stretch,
        shrink,
        step,
    } = phase
    else {
        unreachable!()
    };
    let status = processor.scan_keyword_into("autoexpand", scalar);
    let auto_expand = take_operation_scalar!(
        scalar,
        status,
        PendingOperationScanPhase::PdfFontExpand(PdfFontExpandScanPhase::AutoExpand {
            font,
            stretch,
            shrink,
            step,
        }),
        suspended,
        take_boolean
    )
    .value;
    let spec = tex_typeset::expansion::FontExpansionSpec::new(stretch, shrink, step, auto_expand)?;
    complete_cold_scan!(cold, ColdOperation::PdfFontExpand { font, spec })
}

pub(super) fn scan_font_only_operation<G>(
    cold: &mut ColdOperationSlot<G>,
    processor: &mut CommandProcessor<'_, '_, G>,
    scalar: &mut tex_command::ScalarScanFrame,
    meaning: Meaning,
    suspended: &mut Option<PendingOperationScanPhase>,
) -> Result<(), ExecError> {
    let status = processor.scan_font_selector_into(scalar);
    let font = take_operation_scalar!(
        scalar,
        status,
        PendingOperationScanPhase::FontOnly { meaning },
        suspended,
        take_font
    );
    match meaning {
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::PdfNoLigatures) => {
            complete_cold_scan!(cold, ColdOperation::PdfNoLigatures { font })
        }
        _ => unreachable!("font-only descriptor restricts command meanings"),
    }
}

pub(super) fn scan_open_out_operation<G>(
    cold: &mut ColdOperationSlot<G>,
    processor: &mut CommandProcessor<'_, '_, G>,
    scalar: &mut tex_command::ScalarScanFrame,
    phase: OpenOutScanPhase,
    suspended: &mut Option<PendingOperationScanPhase>,
) -> Result<(), ExecError> {
    let phase = if matches!(phase, OpenOutScanPhase::Stream) {
        let status =
            processor.scan_restricted_integer_into(RestrictedIntegerClass::FourBit, scalar);
        let stream = take_operation_scalar!(
            scalar,
            status,
            PendingOperationScanPhase::OpenOut(OpenOutScanPhase::Stream),
            suspended,
            take_restricted
        )
        .value as u8;
        OpenOutScanPhase::OptionalEquals { stream }
    } else {
        phase
    };
    let phase = match phase {
        OpenOutScanPhase::OptionalEquals { stream } => {
            let status = processor.scan_optional_equals_into(scalar);
            let _ = take_operation_scalar!(
                scalar,
                status,
                PendingOperationScanPhase::OpenOut(OpenOutScanPhase::OptionalEquals { stream }),
                suspended,
                take_boolean
            );
            OpenOutScanPhase::FileName { stream }
        }
        phase => phase,
    };
    let OpenOutScanPhase::FileName { stream } = phase else {
        unreachable!()
    };
    let status = processor.scan_file_name_into(scalar);
    let file_name = take_operation_scalar!(
        scalar,
        status,
        PendingOperationScanPhase::OpenOut(OpenOutScanPhase::FileName { stream }),
        suspended,
        take_file_name
    );
    complete_cold_scan!(
        cold,
        ColdOperation::DeferredOpenOut {
            stream,
            file_name: file_name.packed(),
        }
    )
}

pub(super) fn scan_marks_operation<G>(
    cold: &mut ColdOperationSlot<G>,
    processor: &mut CommandProcessor<'_, '_, G>,
    scalar: &mut tex_command::ScalarScanFrame,
    phase: MarksScanPhase,
    suspended: &mut Option<PendingOperationScanPhase>,
) -> Result<(), ExecError> {
    let phase = if matches!(phase, MarksScanPhase::Class) {
        let status = processor.scan_extended_register_index_into(scalar);
        let class = take_operation_scalar!(
            scalar,
            status,
            PendingOperationScanPhase::Marks(MarksScanPhase::Class),
            suspended,
            take_register
        );
        MarksScanPhase::Text { class }
    } else {
        phase
    };
    let MarksScanPhase::Text { class } = phase else {
        unreachable!()
    };
    let scan = processor.scan_balanced_text_retained(true);
    let text = retain_operation_child(
        processor,
        scan,
        PendingOperationScanPhase::Marks(MarksScanPhase::Text { class }),
        suspended,
    )?;
    complete_cold_scan!(
        cold,
        ColdOperation::Mark {
            class,
            tokens: text.tokens.into(),
        }
    )
}

pub(super) fn scan_math_family_assignment<G>(
    cold: &mut ColdOperationSlot<G>,
    processor: &mut CommandProcessor<'_, '_, G>,
    scalar: &mut tex_command::ScalarScanFrame,
    size: tex_command::MathFamilySize,
    global: bool,
    phase: MathFamilyScanPhase,
    suspended: &mut Option<PendingOperationScanPhase>,
) -> Result<(), ExecError> {
    let phase = if matches!(phase, MathFamilyScanPhase::Family) {
        let scan = processor.scan_math_family_retained(size);
        let family = retain_operation_child(
            processor,
            scan,
            PendingOperationScanPhase::MathFamily {
                size,
                global,
                phase: MathFamilyScanPhase::Family,
            },
            suspended,
        )?;
        MathFamilyScanPhase::OptionalEquals { family }
    } else {
        phase
    };
    let phase = match phase {
        MathFamilyScanPhase::OptionalEquals { family } => {
            let status = processor.scan_optional_equals_into(scalar);
            let _ = take_operation_scalar!(
                scalar,
                status,
                PendingOperationScanPhase::MathFamily {
                    size,
                    global,
                    phase: MathFamilyScanPhase::OptionalEquals { family },
                },
                suspended,
                take_boolean
            );
            MathFamilyScanPhase::Font { family }
        }
        phase => phase,
    };
    let MathFamilyScanPhase::Font { family } = phase else {
        unreachable!()
    };
    let status = processor.scan_font_selector_into(scalar);
    let font = take_operation_scalar!(
        scalar,
        status,
        PendingOperationScanPhase::MathFamily {
            size,
            global,
            phase: MathFamilyScanPhase::Font { family },
        },
        suspended,
        take_font
    );
    complete_cold_scan!(
        cold,
        ColdOperation::MathFamily {
            family,
            font,
            global,
        }
    )
}

pub(super) fn resume_pending_operation_scan<G>(
    processor: &mut CommandProcessor<'_, '_, G>,
    frame: &mut CommandEpisode<G>,
    cold: &mut ColdOperationSlot<G>,
    pending: PendingOperationScanPhase,
    suspended: &mut Option<PendingOperationScanPhase>,
) -> Result<ScannedOperation, ExecError> {
    if let PendingOperationScanPhase::CatCode { global, phase } = pending {
        let operation = hot_apply::scan_catcode_assignment(
            processor,
            &mut frame.scalar,
            global,
            phase,
            suspended,
        )?;
        return Ok(retain_hot_operation(frame, operation));
    }
    let scalar = &mut frame.scalar;
    match pending {
        PendingOperationScanPhase::Count {
            index,
            global,
            phase,
        } => {
            scan_count_register_assignment(cold, processor, scalar, index, global, phase, suspended)
        }
        PendingOperationScanPhase::Dimension {
            index,
            global,
            phase,
        } => scan_dimension_register_assignment(
            cold, processor, scalar, index, global, phase, suspended,
        ),
        PendingOperationScanPhase::BoxDimension {
            index,
            dimension,
            global,
            phase,
        } => scan_box_dimension_assignment(
            cold, processor, scalar, index, dimension, global, phase, suspended,
        ),
        PendingOperationScanPhase::Glue {
            index,
            global,
            mu,
            phase,
        } => scan_glue_register_assignment(
            cold, processor, scalar, index, global, mu, phase, suspended,
        ),
        PendingOperationScanPhase::Unary {
            meaning,
            global,
            origin,
            phase,
        } => scan_unary_scalar_operation(
            cold, processor, scalar, meaning, global, origin, phase, suspended,
        ),
        PendingOperationScanPhase::ParagraphShape { global, phase } => {
            scan_paragraph_shape_assignment(cold, processor, scalar, global, phase, suspended)
        }
        PendingOperationScanPhase::PenaltyArray {
            kind,
            global,
            phase,
        } => scan_penalty_array_assignment(cold, processor, scalar, kind, global, phase, suspended),
        PendingOperationScanPhase::FontDimen(phase) => {
            scan_font_dimen_assignment(cold, processor, scalar, phase, suspended)
        }
        PendingOperationScanPhase::FontInteger { primitive, phase } => {
            scan_font_integer_assignment(cold, processor, scalar, primitive, phase, suspended)
        }
        PendingOperationScanPhase::CodeTable {
            primitive,
            global,
            phase,
        } => {
            scan_code_table_assignment(cold, processor, scalar, primitive, global, phase, suspended)
        }
        PendingOperationScanPhase::PdfFontCode { primitive, phase } => {
            scan_pdf_font_code_assignment(cold, processor, scalar, primitive, phase, suspended)
        }
        PendingOperationScanPhase::PdfFontExpand(phase) => {
            scan_pdf_font_expand_assignment(cold, processor, scalar, phase, suspended)
        }
        PendingOperationScanPhase::FontOnly { meaning } => {
            scan_font_only_operation(cold, processor, scalar, meaning, suspended)
        }
        PendingOperationScanPhase::OpenOut(phase) => {
            scan_open_out_operation(cold, processor, scalar, phase, suspended)
        }
        PendingOperationScanPhase::Marks(phase) => {
            scan_marks_operation(cold, processor, scalar, phase, suspended)
        }
        PendingOperationScanPhase::CatCode { .. } => unreachable!(),
        PendingOperationScanPhase::MathFamily {
            size,
            global,
            phase,
        } => scan_math_family_assignment(cold, processor, scalar, size, global, phase, suspended),
        PendingOperationScanPhase::Arithmetic {
            primitive,
            global,
            phase,
        } => {
            scan_arithmetic_assignment(cold, processor, scalar, primitive, global, phase, suspended)
        }
        PendingOperationScanPhase::LeaderGlue { mode, result } => {
            scan_retained_leader_glue(cold, processor, scalar, mode, result, suspended)
        }
        PendingOperationScanPhase::LeaderPayload { primitive, mode } => {
            scan_leaders_step(cold, processor, scalar, primitive, mode, suspended)
        }
        PendingOperationScanPhase::LeaderCommand { mode, result } => {
            scan_retained_leader_command(cold, processor, scalar, mode, result, suspended)
        }
    }?;
    frame.mark_resident_cold(cold);
    Ok(ScannedOperation::Cold)
}

/// Dispatches one already-fetched command through TeX82 §1030's `reswitch:`
/// label and the big case below it.
///
/// This is the shared tail of *every* main-control step, whatever fetched the
/// command: §1030's own `get_x_token`, §1038's `main_loop_lookahead`, or an
/// alignment cell's template-aware delivery. tex.web has no second dispatcher
/// for alignment bodies -- §785's `align_peek` and §1130's `endv` case only
/// bound a cell, and everything between those bounds runs through the same
/// `main_control` big case -- so a caller that reaches `scan_command` without
/// passing through here is dispatching a *narrowed* main control that silently
/// drops whatever this function handles (`umber2-johp.208`).
///
/// Two things are handled here rather than in `scan_command` because tex.web
/// handles them before its big case reaches an assignment:
///
/// - §1211 `prefixed_command`'s `while cur_cmd=prefix` loop. §1210 routes
///   `any_mode(prefix)` -- so `\global`/`\long`/`\outer` (and e-TeX's
///   `\protected`) are prefixes in every mode, never mode-dispatched
///   primitives, and the accumulated `a` is what the assignment cases below
///   consult. Hoisting the loop above `scan_command` keeps that single
///   accumulation point, but only if every dispatch path runs it.
/// - §1045's `any_mode(ignore_spaces): begin <Get the next non-blank non-call
///   token>; goto reswitch; end`.
#[allow(clippy::too_many_arguments)] // carries command-owned replay facts
pub(super) fn dispatch_main_control_command<G>(
    processor: &mut CommandProcessor<'_, '_, G>,
    command: &mut CommandEpisode<G>,
    cold: &mut ColdOperationSlot<G>,
    mode: Mode,
    boxes: &ReplayBoxes<G>,
    innermost_group: Option<GroupKind>,
    job_is_all_over: bool,
    display_eq_no: bool,
    shown_mode: &mut Option<Mode>,
    diagnostics: &mut Vec<PendingDiagnostic<G>>,
    alignment: Option<AlignmentIdentity>,
    set_box_allowed: bool,
) -> Result<ScannedOperation, ExecError> {
    // TeX82 §1078 uses §404's non-blank, non-relax fetch after every leader
    // payload. Constructed boxes close in a separate replay step, so the first
    // token after the box has already reached this dispatcher. Finish §404
    // here without exposing its filler to main control or command tracing.
    if boxes.pending_leader.is_some()
        && matches!(
            command.current().meaning(),
            ResolvedMeaning::Static(
                Meaning::CharToken {
                    cat: Catcode::Space,
                    ..
                } | Meaning::Relax
            )
        )
    {
        let mut destination = None;
        if next_non_blank_non_relax_x_token_into(processor, &mut destination)
            .map_err(command_error)?
            != tex_command::DeliveryStatus::Command
        {
            return Err(ExecError::MissingToken {
                context: "leader glue",
            });
        }
        command.replace_current(
            destination
                .take()
                .expect("command status initializes destination"),
        );
    }
    let origin = command.current().origin();
    dispatch_main_control_command_inner(
        processor,
        command,
        cold,
        mode,
        boxes,
        innermost_group,
        job_is_all_over,
        display_eq_no,
        shown_mode,
        diagnostics,
        alignment,
        set_box_allowed,
        None,
    )
    .map_err(|error| error.capture_command_origin(origin))
}

#[cfg(feature = "profiling")]
pub(super) fn hot_core_command_family<G>(
    meaning: &ResolvedMeaning<G>,
) -> tex_state::measurement::HotCoreCommandFamily {
    use tex_state::measurement::HotCoreCommandFamily as Family;

    match meaning {
        ResolvedMeaning::Static(
            Meaning::CharGiven(_) | Meaning::CharToken { .. } | Meaning::MathCharGiven(_),
        ) => Family::Character,
        ResolvedMeaning::Static(Meaning::Relax) => Family::Relax,
        ResolvedMeaning::Static(Meaning::Undefined) => Family::Undefined,
        ResolvedMeaning::Macro { .. } => Family::Macro,
        ResolvedMeaning::Static(Meaning::ExpandablePrimitive(_)) => Family::ExpandablePrimitive,
        ResolvedMeaning::Static(Meaning::UnexpandablePrimitive(_)) => Family::UnexpandablePrimitive,
        ResolvedMeaning::Static(
            Meaning::CountRegister(_)
            | Meaning::DimenRegister(_)
            | Meaning::SkipRegister(_)
            | Meaning::MuskipRegister(_)
            | Meaning::ToksRegister(_)
            | Meaning::IntParam(_)
            | Meaning::DimenParam(_)
            | Meaning::GlueParam(_)
            | Meaning::MuGlueParam(_)
            | Meaning::TokParam(_)
            | Meaning::PageDimension(_)
            | Meaning::PageInteger(_),
        ) => Family::RegisterOrParameter,
        ResolvedMeaning::Static(Meaning::Font(_)) => Family::Font,
        ResolvedMeaning::Static(Meaning::InternalInteger(_)) => Family::InternalQuantity,
        ResolvedMeaning::Static(Meaning::EndV) => Family::EndTemplate,
        ResolvedMeaning::Static(Meaning::Unknown(_)) => Family::Unknown,
    }
}

#[cfg(feature = "profiling")]
fn hot_core_meaning_family<G>(
    meaning: &ResolvedMeaning<G>,
) -> tex_state::measurement::HotCoreMeaningFamily {
    use tex_state::measurement::HotCoreMeaningFamily as Family;

    match meaning {
        ResolvedMeaning::Static(Meaning::Undefined) => Family::Undefined,
        ResolvedMeaning::Static(Meaning::Relax) => Family::Relax,
        ResolvedMeaning::Macro { .. } => Family::Macro,
        ResolvedMeaning::Static(Meaning::CharGiven(_)) => Family::CharGiven,
        ResolvedMeaning::Static(Meaning::CharToken { .. }) => Family::CharToken,
        ResolvedMeaning::Static(Meaning::MathCharGiven(_)) => Family::MathCharGiven,
        ResolvedMeaning::Static(Meaning::CountRegister(_)) => Family::CountRegister,
        ResolvedMeaning::Static(Meaning::DimenRegister(_)) => Family::DimenRegister,
        ResolvedMeaning::Static(Meaning::SkipRegister(_)) => Family::SkipRegister,
        ResolvedMeaning::Static(Meaning::MuskipRegister(_)) => Family::MuskipRegister,
        ResolvedMeaning::Static(Meaning::ToksRegister(_)) => Family::ToksRegister,
        ResolvedMeaning::Static(Meaning::IntParam(_)) => Family::IntParam,
        ResolvedMeaning::Static(Meaning::DimenParam(_)) => Family::DimenParam,
        ResolvedMeaning::Static(Meaning::GlueParam(_)) => Family::GlueParam,
        ResolvedMeaning::Static(Meaning::MuGlueParam(_)) => Family::MuGlueParam,
        ResolvedMeaning::Static(Meaning::TokParam(_)) => Family::TokParam,
        ResolvedMeaning::Static(Meaning::PageDimension(_)) => Family::PageDimension,
        ResolvedMeaning::Static(Meaning::PageInteger(_)) => Family::PageInteger,
        ResolvedMeaning::Static(Meaning::InternalInteger(_)) => Family::InternalInteger,
        ResolvedMeaning::Static(Meaning::Font(_)) => Family::Font,
        ResolvedMeaning::Static(Meaning::ExpandablePrimitive(_)) => Family::ExpandablePrimitive,
        ResolvedMeaning::Static(Meaning::EndV) => Family::EndV,
        ResolvedMeaning::Static(Meaning::UnexpandablePrimitive(_)) => Family::UnexpandablePrimitive,
        ResolvedMeaning::Static(Meaning::Unknown(_)) => Family::Unknown,
    }
}

#[allow(clippy::too_many_arguments)] // carries command-owned replay facts
pub(super) fn dispatch_main_control_command_inner<G>(
    processor: &mut CommandProcessor<'_, '_, G>,
    command: &mut CommandEpisode<G>,
    cold: &mut ColdOperationSlot<G>,
    mode: Mode,
    boxes: &ReplayBoxes<G>,
    innermost_group: Option<GroupKind>,
    job_is_all_over: bool,
    display_eq_no: bool,
    shown_mode: &mut Option<Mode>,
    diagnostics: &mut Vec<PendingDiagnostic<G>>,
    alignment: Option<AlignmentIdentity>,
    set_box_allowed: bool,
    mut initial_prefix: Option<(bool, MeaningFlags)>,
) -> Result<ScannedOperation, ExecError> {
    // TeX82 §1078 fetches the command following a completed leader payload
    // inside `box_end`, before control returns to §1030's `big_switch` or
    // §1211's prefix loop. Split replay finishes the box in one step and
    // delivers that command in the next, so classify it at this same outer
    // boundary. In particular, a non-glue `\global` is the command that
    // `back_error` must restore; allowing it into the prefix loop first would
    // consume and restore the following assignment instead.
    if let Some((kind, payload)) = boxes.pending_leader.as_ref() {
        let result = LeaderGlueResult::Payload {
            kind: *kind,
            payload: *payload,
        };
        let mut suspended = None;
        let scanned = scan_leader_glue_command(
            cold,
            processor,
            &mut command.scalar,
            &mut command.command,
            mode,
            result,
            &mut suspended,
        );
        if let Err(error) = &scanned
            && execution_error_needs_command_retry(error)
            && let Some(phase) = suspended
        {
            let child = processor
                .take_scanner_resume()
                .expect("a suspended leader glue scan retains its exact child capability");
            command.retain_operation_scan(processor.delivery_cursor(), phase, child);
        }
        if !scanned? {
            return Ok(retain_cold_operation(
                command,
                cold,
                ColdOperation::<G>::LeadersNotFollowedByGlue,
            ));
        }
        command.mark_resident_cold(cold);
        return Ok(ScannedOperation::Cold);
    }
    // §1030's `reswitch:` label sits *above* the big case, not at the fetch:
    // a case that has already fetched its own replacement command dispatches
    // that command in place. `goto reswitch` is therefore not `back_input`,
    // and a case using it pushes no input level and delivers nothing twice.
    // This loop is that label.
    let mut suppress_left_boundary = false;
    loop {
        let (mut global, mut flags) = initial_prefix
            .take()
            .unwrap_or((false, MeaningFlags::EMPTY));
        loop {
            let retained_global = global;
            let retained_flags = flags;
            #[cfg(feature = "profiling")]
            {
                let meaning = command.current().meaning_ref();
                tex_state::measurement::record_hot_core_command_family(hot_core_command_family(
                    meaning,
                ));
                tex_state::measurement::record_hot_core_main_control_meaning(
                    hot_core_meaning_family(meaning),
                );
                if let ResolvedMeaning::Static(Meaning::UnexpandablePrimitive(primitive)) = meaning
                {
                    tex_state::measurement::record_hot_core_unexpandable_opcode(
                        usize::try_from(primitive.operand())
                            .expect("unexpandable primitive operand fits usize"),
                    );
                }
            }
            match command.current().meaning() {
                ResolvedMeaning::Static(Meaning::UnexpandablePrimitive(
                    UnexpandablePrimitive::Global,
                )) => global = true,
                ResolvedMeaning::Static(Meaning::UnexpandablePrimitive(
                    UnexpandablePrimitive::Long,
                )) => flags = flags | MeaningFlags::LONG,
                ResolvedMeaning::Static(Meaning::UnexpandablePrimitive(
                    UnexpandablePrimitive::Outer,
                )) => flags = flags | MeaningFlags::OUTER,
                ResolvedMeaning::Static(Meaning::UnexpandablePrimitive(
                    UnexpandablePrimitive::Protected,
                )) => flags = flags | MeaningFlags::PROTECTED,
                _ => break,
            }
            let mut destination = None;
            let next = match next_non_blank_non_relax_x_token_into(processor, &mut destination) {
                Ok(tex_command::DeliveryStatus::Command) => destination
                    .take()
                    .expect("command status initializes destination"),
                Ok(tex_command::DeliveryStatus::End) => {
                    return Err(ExecError::MissingPrefixedCommand);
                }
                Ok(_) => unreachable!("ordinary expanded delivery returns only commands"),
                Err(error) => {
                    let error = command_error(error);
                    if execution_error_needs_command_retry(&error) {
                        let child = processor
                            .take_scanner_resume()
                            .expect("a suspended prefix fetch retains its exact expansion child");
                        command.phase = Some(PreflightCommandPhase::PrefixScan {
                            global: retained_global,
                            flags: retained_flags,
                            alignment,
                            set_box_allowed,
                        });
                        command.retain_scanner(processor.delivery_cursor(), Some(child));
                    }
                    return Err(error);
                }
            };
            command.replace_current(next);
            // §1211's `if cur_cmd<=max_non_prefixed_command then <Discard
            // erroneous prefixes and return>`: §209's partition, not a
            // hand-listed set of assignment families.
            if !tex_command::exceeds_max_non_prefixed_command(static_meaning(
                command.current().meaning(),
            )) {
                let printed = tex_command::PrintCommand::from_current(command.current());
                // §1212's `back_error`: the substantive command is retained
                // and re-delivered without the discarded prefixes.
                processor
                    .back_input(command.take_current())
                    .map_err(command_error)?;
                // `back_error` is `back_input` *then* `error`, so §82 renders
                // the context with the backed-up level already on the stack.
                let etex = processor.profile().capabilities().supports_etex();
                diagnostics.push(PendingDiagnostic::PrefixOnNonPrefixedCommand(
                    printed,
                    processor.error_context(),
                    etex,
                ));
                return Ok(retain_cold_operation(
                    command,
                    cold,
                    ColdOperation::<G>::Continue,
                ));
            }
        }
        // §1213's `<Discard the prefixes \long and \outer if they are
        // irrelevant>`. §1214 deliberately leaves `a` unadjusted, so the
        // command still runs; only the report is owed. eTeX's `\protected`
        // is prefix code 8, which §1213's `a mod 4<>0` excludes.
        if flags.bits() & (MeaningFlags::LONG | MeaningFlags::OUTER).bits() != 0
            && !matches!(
                command.current().meaning(),
                ResolvedMeaning::Static(Meaning::UnexpandablePrimitive(
                    UnexpandablePrimitive::Def
                        | UnexpandablePrimitive::Edef
                        | UnexpandablePrimitive::Gdef
                        | UnexpandablePrimitive::Xdef
                ))
            )
        {
            let etex = processor.profile().capabilities().supports_etex();
            diagnostics.push(PendingDiagnostic::IrrelevantLongOuterPrefix(
                tex_command::PrintCommand::from_current(command.current()),
                processor.error_context(),
                etex,
            ));
        }
        // §406's helper is `repeat get_x_token until cur_cmd<>spacer` --
        // exactly `next_non_space` -- and the command it leaves in `cur_cmd`
        // is then dispatched by the case itself. Backing it up instead would
        // push a backup level, emit a recovery record, and deliver that
        // command a second time, none of which TeX82 does
        // (`umber2-johp.196`).
        if matches!(
            command.current().meaning(),
            ResolvedMeaning::Static(Meaning::UnexpandablePrimitive(
                UnexpandablePrimitive::IgnoreSpaces
            ))
        ) {
            let next = if let Some(alignment) = alignment {
                let mut destination = None;
                loop {
                    match processor
                        .get_x_alignment_delivery_into(false, &mut destination)
                        .map_err(command_error)?
                    {
                        tex_command::DeliveryStatus::End => {
                            return Ok(retain_cold_operation(
                                command,
                                cold,
                                ColdOperation::<G>::EndOfInput,
                            ));
                        }
                        tex_command::DeliveryStatus::ReplayCompleted(episode) => {
                            return Ok(retain_cold_operation(
                                command,
                                cold,
                                ColdOperation::<G>::ReplayCompleted(episode),
                            ));
                        }
                        tex_command::DeliveryStatus::AlignmentEndTemplate => {
                            let event = tex_command::AlignmentDeliveryEvent::EndTemplate(
                                destination
                                    .take()
                                    .expect("alignment status initializes destination"),
                            );
                            scan_alignment_delivery_event(cold, processor, alignment, event)?;
                            command.mark_resident_cold(cold);
                            return Ok(ScannedOperation::Cold);
                        }
                        tex_command::DeliveryStatus::AlignmentClosingBrace => {
                            let event = tex_command::AlignmentDeliveryEvent::ClosingBrace(
                                destination
                                    .take()
                                    .expect("alignment status initializes destination"),
                            );
                            scan_alignment_delivery_event(cold, processor, alignment, event)?;
                            command.mark_resident_cold(cold);
                            return Ok(ScannedOperation::Cold);
                        }
                        tex_command::DeliveryStatus::Command
                            if matches!(
                                destination
                                    .as_ref()
                                    .expect("command status initializes destination")
                                    .meaning(),
                                ResolvedMeaning::Static(Meaning::CharToken {
                                    cat: Catcode::Space,
                                    ..
                                })
                            ) =>
                        {
                            destination = None
                        }
                        tex_command::DeliveryStatus::Command => {
                            break destination
                                .take()
                                .expect("command status initializes destination");
                        }
                        tex_command::DeliveryStatus::PendingExpanded => {
                            unreachable!("alignment delivery commits terminal observations");
                        }
                    }
                }
            } else {
                let mut destination = None;
                if next_non_blank_x_token_into(processor, &mut destination)
                    .map_err(command_error)?
                    != tex_command::DeliveryStatus::Command
                {
                    return Ok(retain_cold_operation(
                        command,
                        cold,
                        ColdOperation::<G>::EndOfInput,
                    ));
                }
                destination
                    .take()
                    .expect("command status initializes destination")
            };
            command.replace_current(next);
            report_command_trace(processor, mode, command.current(), shown_mode);
            continue;
        }
        if matches!(mode, Mode::Horizontal | Mode::RestrictedHorizontal)
            && matches!(
                command.current().meaning(),
                ResolvedMeaning::Static(Meaning::UnexpandablePrimitive(
                    UnexpandablePrimitive::NoBoundary
                ))
            )
        {
            let mut destination = None;
            if processor
                .get_x_token_into(&mut destination)
                .map_err(command_error)?
                != tex_command::DeliveryStatus::Command
            {
                return Ok(retain_cold_operation(
                    command,
                    cold,
                    ColdOperation::<G>::Continue,
                ));
            }
            let next = destination
                .take()
                .expect("command status initializes destination");
            suppress_left_boundary = matches!(
                next.meaning(),
                ResolvedMeaning::Static(
                    Meaning::CharToken {
                        cat: Catcode::Letter | Catcode::Other,
                        ..
                    } | Meaning::CharGiven(_)
                        | Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Char)
                )
            );
            command.replace_current(next);
            report_command_trace(processor, mode, command.current(), shown_mode);
            continue;
        }
        // TeX82 §1214 resolves `\globaldefs` exactly once, before entering
        // §1211's assignment case. Every scanner-time provisional
        // definition, committed application, and mutation observation below
        // therefore receives the same effective value rather than
        // independently consulting live state at a later seam.
        let global = effective_global(
            processor.int_param(IntParam::GLOBAL_DEFS),
            global
                || matches!(
                    command.current().meaning(),
                    ResolvedMeaning::Static(Meaning::UnexpandablePrimitive(
                        UnexpandablePrimitive::Gdef | UnexpandablePrimitive::Xdef
                    ))
                ),
        );
        command.phase = Some(PreflightCommandPhase::Settled);
        command.operation_scan = None;
        let mut suspended_operation_scan = None;
        let scanned_result = scan_command(
            processor,
            command,
            cold,
            global,
            flags,
            mode,
            boxes,
            innermost_group,
            job_is_all_over,
            display_eq_no,
            set_box_allowed,
            shown_mode,
            &mut suspended_operation_scan,
        );
        if let Err(error) = &scanned_result
            && execution_error_needs_command_retry(error)
        {
            let child = processor
                .take_scanner_resume()
                .expect("a suspended substantive command retains its exact scanner capability");
            if let Some(phase) = suspended_operation_scan {
                command.retain_operation_scan(processor.delivery_cursor(), phase, child);
            } else {
                command.phase = Some(PreflightCommandPhase::PrefixedCommandScan {
                    global,
                    flags,
                    set_box_allowed,
                });
                command.retain_scanner(processor.delivery_cursor(), Some(child));
            }
        }
        let scanned = scanned_result?;
        if suppress_left_boundary
            && scanned == ScannedOperation::Cold
            && let ColdOperation::<G>::Character {
                suppress_left_boundary,
                ..
            }
            | ColdOperation::<G>::CharacterCode {
                suppress_left_boundary,
                ..
            } = command.unavailable_mut(cold)
        {
            *suppress_left_boundary = true;
        }
        return Ok(scanned);
    }
}

/// TeX82 §1030's `if tracing_commands>0 then show_cur_cmd_chr` at `reswitch`,
/// reached after `big_switch` and after cases such as §1045 `ignore_spaces`
/// fetch a replacement command. §1211's prefix loop does not return to that
/// label, so its internal fetches remain untraced.
pub(super) fn report_command_trace<G>(
    processor: &mut CommandProcessor<'_, '_, G>,
    mode: Mode,
    command: &tex_command::CurrentCommand<G>,
    shown_mode: &mut Option<Mode>,
) {
    if processor.int_param(IntParam::TRACING_COMMANDS) > 0 {
        *shown_mode = Some(mode);
        processor.print_command_trace(tex_command::PrintCommand::from_current(command));
    }
}

/// Applies TeX82's §1030 main-control trace boundary to a fetched command.
///
/// A constructed leader payload is one exception: after its box closes,
/// §1078's `box_end` fetches the following glue inside the leader case and
/// never returns to `big_switch`. The split replay lifecycle leaves that
/// internal fetch to the next processor episode, so `pending_leader` retains
/// the canonical boundary distinction and suppresses only §1030's settled
/// unexpandable-command trace. Expansion tracing performed by `get_x_token`
/// remains unchanged.
///
/// The opening brace of an output routine is the other exception. TeX82
/// §1025 consumes it with `scan_left_brace` before entering §1030, whereas
/// split replay delivers it as an explicit step. Suppressing that delivery
/// also leaves the mode prefix pending for the first command in the routine.
///
/// A `\shipout` box constructor is likewise scanner-owned: §§1075/1084 call
/// `scan_box` from the already-traced `leader_ship` case, so its `\hbox`,
/// `\vbox`, or `\vtop` never returns to §1030's `reswitch`. Split replay
/// retains `pending_shipout` across that internal fetch.
pub(super) fn report_main_control_command_trace<G>(
    processor: &mut CommandProcessor<'_, '_, G>,
    mode: Mode,
    command: &tex_command::CurrentCommand<G>,
    boxes: &ReplayBoxes<G>,
    shown_mode: &mut Option<Mode>,
) {
    // Expansion can invoke §299 itself for e-TeX's `\tracingifs`. Its mode
    // prefix and `shown_mode` transition precede this settled command.
    if processor.command_trace_printed() {
        *shown_mode = Some(mode);
    }
    let output_routine_opening = boxes.output_routine_opening_pending
        && matches!(
            command.meaning(),
            ResolvedMeaning::Static(Meaning::CharToken {
                cat: Catcode::BeginGroup,
                ..
            })
        );
    let shipout_box_constructor = boxes.pending_shipout.is_some()
        && matches!(
            command.meaning(),
            ResolvedMeaning::Static(Meaning::UnexpandablePrimitive(
                UnexpandablePrimitive::HBox
                    | UnexpandablePrimitive::VBox
                    | UnexpandablePrimitive::VTop
            ))
        );
    if boxes.pending_leader.is_none() && !output_routine_opening && !shipout_box_constructor {
        report_command_trace(processor, mode, command, shown_mode);
    }
}

pub(super) fn prepare_command_trace<G>(
    processor: &mut CommandProcessor<'_, '_, G>,
    mode: Mode,
    shown_mode: Option<Mode>,
) {
    // The mode text is owned because expansion may retain it until a later
    // command in this processor episode.  Do not allocate that ownership when
    // neither command nor conditional tracing can consume the prefix: this is
    // the ordinary production path and this boundary is crossed once per
    // main-control operation.
    let tracing_can_consume_prefix = processor.int_param(IntParam::TRACING_COMMANDS) > 0
        || processor.int_param(IntParam::TRACING_IFS) > 0;
    let mode_prefix = (tracing_can_consume_prefix && shown_mode != Some(mode))
        .then(|| mode_text_for_command_trace(mode).into());
    processor.set_command_trace_mode_prefix(mode_prefix);
}

pub(super) fn scan_leaders_step<G>(
    cold: &mut ColdOperationSlot<G>,
    processor: &mut CommandProcessor<'_, '_, G>,
    scalar: &mut tex_command::ScalarScanFrame,
    primitive: UnexpandablePrimitive,
    mode: Mode,
    suspended: &mut Option<PendingOperationScanPhase>,
) -> Result<(), ExecError> {
    let kind = crate::box_runtime::leader_glue_kind(primitive);
    *suspended = Some(PendingOperationScanPhase::LeaderPayload { primitive, mode });
    let payload = processor.scan_leader_payload().map_err(command_error)?;
    *suspended = None;
    match payload {
        ScannedLeaderPayload::Missing => {
            complete_cold_scan!(cold, ColdOperation::<G>::MissingLeaderPayload)
        }
        ScannedLeaderPayload::Construction(construction) => {
            complete_cold_scan!(
                cold,
                ColdOperation::<G>::BeginLeaderBox { construction, kind }
            )
        }
        ScannedLeaderPayload::Rule(rule) => {
            let payload = LeaderPayload::Rule {
                width: rule.width,
                height: rule.height,
                depth: rule.depth,
            };
            let result = LeaderGlueResult::Payload { kind, payload };
            *suspended = Some(PendingOperationScanPhase::LeaderCommand { mode, result });
            let mut destination = None;
            if next_non_blank_non_relax_x_token_into(processor, &mut destination)
                .map_err(command_error)?
                != tex_command::DeliveryStatus::Command
            {
                return Err(ExecError::MissingToken {
                    context: "leader glue",
                });
            }
            let mut glue_command = destination;
            *suspended = None;
            if !scan_leader_glue_command(
                cold,
                processor,
                scalar,
                &mut glue_command,
                mode,
                result,
                suspended,
            )? {
                return complete_cold_scan!(cold, ColdOperation::<G>::LeadersNotFollowedByGlue);
            }
            Ok(())
        }
        // Register payloads must retain their destructive/copy ownership at
        // replay time.  Keep the command scanner's completed glue read, then
        // use the regular typed box read path to obtain the node.
        ScannedLeaderPayload::BoxRegister { index, copy } => {
            let result = LeaderGlueResult::Register { kind, index, copy };
            *suspended = Some(PendingOperationScanPhase::LeaderCommand { mode, result });
            let mut destination = None;
            if next_non_blank_non_relax_x_token_into(processor, &mut destination)
                .map_err(command_error)?
                != tex_command::DeliveryStatus::Command
            {
                return Err(ExecError::MissingToken {
                    context: "leader glue",
                });
            }
            let mut glue_command = destination;
            *suspended = None;
            if !scan_leader_glue_command(
                cold,
                processor,
                scalar,
                &mut glue_command,
                mode,
                result,
                suspended,
            )? {
                return complete_cold_scan!(cold, ColdOperation::<G>::LeadersNotFollowedByGlue);
            }
            Ok(())
        }
    }
}

pub(super) fn scan_retained_leader_command<G>(
    cold: &mut ColdOperationSlot<G>,
    processor: &mut CommandProcessor<'_, '_, G>,
    scalar: &mut tex_command::ScalarScanFrame,
    mode: Mode,
    result: LeaderGlueResult,
    suspended: &mut Option<PendingOperationScanPhase>,
) -> Result<(), ExecError> {
    *suspended = Some(PendingOperationScanPhase::LeaderCommand { mode, result });
    let mut destination = None;
    if next_non_blank_non_relax_x_token_into(processor, &mut destination).map_err(command_error)?
        != tex_command::DeliveryStatus::Command
    {
        return Err(ExecError::MissingToken {
            context: "leader glue",
        });
    }
    let mut glue_command = destination;
    *suspended = None;
    if !scan_leader_glue_command(
        cold,
        processor,
        scalar,
        &mut glue_command,
        mode,
        result,
        suspended,
    )? {
        return Err(ExecError::MissingToken {
            context: "leader glue command",
        });
    }
    Ok(())
}

pub(super) fn scan_leader_glue_command<G>(
    cold: &mut ColdOperationSlot<G>,
    processor: &mut CommandProcessor<'_, '_, G>,
    scalar: &mut tex_command::ScalarScanFrame,
    command: &mut Option<tex_command::CurrentCommand<G>>,
    mode: Mode,
    result: LeaderGlueResult,
    suspended: &mut Option<PendingOperationScanPhase>,
) -> Result<bool, ExecError> {
    let horizontal = matches!(
        mode,
        Mode::Horizontal | Mode::RestrictedHorizontal | Mode::Math | Mode::DisplayMath
    );
    let primitive = match command
        .as_ref()
        .expect("leader glue scanner owns its current command")
        .meaning()
    {
        ResolvedMeaning::Static(Meaning::UnexpandablePrimitive(primitive)) => primitive,
        _ => {
            processor
                .back_input(
                    command
                        .take()
                        .expect("leader glue scanner owns its current command"),
                )
                .map_err(command_error)?;
            return Ok(false);
        }
    };
    if (horizontal && primitive == UnexpandablePrimitive::HSkip)
        || (!horizontal && primitive == UnexpandablePrimitive::VSkip)
    {
        let status = processor.scan_glue_into(false, scalar);
        let glue = take_operation_scalar!(
            scalar,
            status,
            PendingOperationScanPhase::LeaderGlue { mode, result },
            suspended,
            take_glue
        )
        .value;
        write_completed_leader_glue(cold, result, glue);
        return Ok(true);
    }
    let infinite = match (horizontal, primitive) {
        (true, UnexpandablePrimitive::HFil) | (false, UnexpandablePrimitive::VFil) => {
            Some((Order::Fil, false, false))
        }
        (true, UnexpandablePrimitive::HFill) | (false, UnexpandablePrimitive::VFill) => {
            Some((Order::Fill, false, false))
        }
        (true, UnexpandablePrimitive::HSs) | (false, UnexpandablePrimitive::VSs) => {
            Some((Order::Fil, false, true))
        }
        (true, UnexpandablePrimitive::HFilNeg) | (false, UnexpandablePrimitive::VFilNeg) => {
            Some((Order::Fil, true, false))
        }
        _ => None,
    };
    let Some((order, negative, shrink)) = infinite else {
        processor
            .back_input(
                command
                    .take()
                    .expect("leader glue scanner owns its current command"),
            )
            .map_err(command_error)?;
        return Ok(false);
    };
    let unit = Scaled::from_raw(if negative {
        -Scaled::UNITY
    } else {
        Scaled::UNITY
    });
    let zero = Scaled::from_raw(0);
    let glue = if shrink {
        GlueSpec {
            width: zero,
            stretch: zero,
            stretch_order: Order::Normal,
            shrink: unit,
            shrink_order: order,
        }
    } else {
        GlueSpec {
            width: zero,
            stretch: unit,
            stretch_order: order,
            shrink: zero,
            shrink_order: Order::Normal,
        }
    };
    write_completed_leader_glue(cold, result, glue);
    Ok(true)
}

pub(super) fn scan_retained_leader_glue<G>(
    cold: &mut ColdOperationSlot<G>,
    processor: &mut CommandProcessor<'_, '_, G>,
    scalar: &mut tex_command::ScalarScanFrame,
    mode: Mode,
    result: LeaderGlueResult,
    suspended: &mut Option<PendingOperationScanPhase>,
) -> Result<(), ExecError> {
    let status = processor.scan_glue_into(false, scalar);
    let glue = take_operation_scalar!(
        scalar,
        status,
        PendingOperationScanPhase::LeaderGlue { mode, result },
        suspended,
        take_glue
    )
    .value;
    write_completed_leader_glue(cold, result, glue);
    Ok(())
}

fn write_completed_leader_glue<G>(
    cold: &mut ColdOperationSlot<G>,
    result: LeaderGlueResult,
    glue: GlueSpec,
) {
    write_cold_scan!(
        cold,
        match result {
            LeaderGlueResult::Payload { kind, payload } => ColdOperation::Leaders {
                kind,
                payload,
                glue,
            },
            LeaderGlueResult::Register { kind, index, copy } => ColdOperation::LeaderRegister {
                kind,
                index,
                copy,
                glue,
            },
        }
    );
}

/// Recognizes membership in TeX82 §1090's shared vertical-mode
/// `back_input; new_graf(true)` case, listed there as
/// `vmode+letter,vmode+other_char,vmode+char_num,vmode+char_given,`
/// `vmode+math_shift,vmode+un_hbox,vmode+vrule,vmode+accent,`
/// `vmode+discretionary,vmode+hskip,vmode+valign,vmode+ex_space,`
/// `vmode+no_boundary`.
///
/// The caller has already established that the mode is (internal) vertical:
/// tex.web's big case is `case abs(mode)+cur_cmd of`, so `vmode+x` covers both
/// `vmode` and `-vmode`. Membership is decided purely from the delivered
/// command, exactly as tex.web decides it from `cur_cmd`.
pub(super) fn starts_paragraph_in_vertical_mode<G>(meaning: ResolvedMeaning<G>) -> bool {
    match meaning {
        // `vmode+letter`, `vmode+other_char`, and `vmode+math_shift`. A
        // `spacer` is deliberately absent: §1045's `vmode+spacer: do_nothing`
        // leaves vertical mode untouched, and every other category code
        // (braces, `#`, `^`, `_`, `~`) has its own case elsewhere.
        ResolvedMeaning::Static(Meaning::CharToken { cat, .. }) => {
            matches!(cat, Catcode::Letter | Catcode::Other | Catcode::MathShift)
        }
        // `vmode+char_given`: a `\chardef`'d token (§1224 installs it as
        // `char_given`), which §1090 treats exactly like `char_num`.
        ResolvedMeaning::Static(Meaning::CharGiven(_)) => true,
        ResolvedMeaning::Static(Meaning::UnexpandablePrimitive(primitive)) => matches!(
            primitive,
            // `vmode+char_num`: §265's `primitive("char",char_num,0)`.
            UnexpandablePrimitive::Char
                // `vmode+un_hbox`: §1107 installs `\unhbox` and
                // `\unhcopy` under the one `un_hbox` command code. `un_vbox`
                // is not in this group -- `\unvbox` legitimately appends an
                // unboxed vertical list to the enclosing vertical list.
                | UnexpandablePrimitive::UnHBox
                | UnexpandablePrimitive::UnHCopy
                // `vmode+vrule`: §265's `primitive("vrule",vrule,0)`.
                // `\hrule` is instead §1056's `vmode+hrule:
                // tail_append(scan_rule_spec)`, which stays in vertical mode.
                | UnexpandablePrimitive::VRule
                // `vmode+accent`: §265's `primitive("accent",accent,0)`.
                | UnexpandablePrimitive::Accent
                // `vmode+discretionary`: §1114 installs both `\-`
                // (chr 1) and `\discretionary` (chr 0) as `discretionary`.
                | UnexpandablePrimitive::Discretionary
                | UnexpandablePrimitive::DiscretionaryHyphen
                // `vmode+hskip`: §1058 installs `\hskip`, `\hfil`,
                // `\hfill`, `\hss`, and `\hfilneg` under the one `hskip`
                // command code. `\kern` is `kern`, not `hskip`, and §1057's
                // `vmode+kern` appends to the vertical list instead.
                | UnexpandablePrimitive::HSkip
                | UnexpandablePrimitive::HFil
                | UnexpandablePrimitive::HFill
                | UnexpandablePrimitive::HSs
                | UnexpandablePrimitive::HFilNeg
                // `vmode+valign`: §265's `primitive("valign",valign,0)`.
                // e-TeX 2.6 [53a.3826--3883] deliberately gives all four
                // text-direction primitives this same command code with
                // nonzero modifiers. TeX §1090 dispatches by command code,
                // so they also start a paragraph before their hmode action.
                | UnexpandablePrimitive::VAlign
                | UnexpandablePrimitive::BeginL
                | UnexpandablePrimitive::EndL
                | UnexpandablePrimitive::BeginR
                | UnexpandablePrimitive::EndR
                // `vmode+ex_space`: §265's `primitive("␣",ex_space,0)`.
                | UnexpandablePrimitive::ControlSpace
                // `vmode+no_boundary`: §265's
                // `primitive("noboundary",no_boundary,0)`.
                | UnexpandablePrimitive::NoBoundary
        ),
        _ => false,
    }
}

#[allow(clippy::too_many_arguments)] // mirrors TeX main-control dispatch inputs
pub(super) fn scan_command<G>(
    processor: &mut CommandProcessor<'_, '_, G>,
    command: &mut CommandEpisode<G>,
    cold: &mut ColdOperationSlot<G>,
    global: bool,
    flags: MeaningFlags,
    mode: Mode,
    boxes: &ReplayBoxes<G>,
    innermost_group: Option<GroupKind>,
    job_is_all_over: bool,
    display_eq_no: bool,
    set_box_allowed: bool,
    shown_mode: &mut Option<Mode>,
    suspended_operation_scan: &mut Option<PendingOperationScanPhase>,
) -> Result<ScannedOperation, ExecError> {
    if let ResolvedMeaning::Static(Meaning::UnexpandablePrimitive(
        primitive @ (UnexpandablePrimitive::TextFont
        | UnexpandablePrimitive::ScriptFont
        | UnexpandablePrimitive::ScriptScriptFont),
    )) = command.meaning()
    {
        let size = tex_command::MathFamilySize::of_primitive(primitive)
            .expect("the outer match restricts this to `def_family`");
        scan_math_family_assignment(
            cold,
            processor,
            &mut command.scalar,
            size,
            global,
            MathFamilyScanPhase::Family,
            suspended_operation_scan,
        )?;
        command.mark_resident_cold(cold);
        return Ok(ScannedOperation::Cold);
    }
    // Math operands are scanned exclusively by `tex-command`.  The replay
    // driver receives a typed scalar request and schedules any opaque field
    // episode only after this processor borrow has ended.
    if matches!(mode, Mode::Math | Mode::DisplayMath)
        && let ResolvedMeaning::Static(Meaning::UnexpandablePrimitive(
            primitive @ (UnexpandablePrimitive::Left
            | UnexpandablePrimitive::Right
            | UnexpandablePrimitive::Middle),
        )) = command.meaning()
    {
        let kind = match primitive {
            UnexpandablePrimitive::Left => MathDelimiterBoundaryKind::Left,
            UnexpandablePrimitive::Right => MathDelimiterBoundaryKind::Right,
            UnexpandablePrimitive::Middle => MathDelimiterBoundaryKind::Middle,
            _ => unreachable!(),
        };
        return Ok(retain_cold_operation(
            command,
            cold,
            ColdOperation::<G>::MathDelimiter(
                processor
                    .scan_math_delimiter_boundary(kind)
                    .map_err(command_error)?,
            ),
        ));
    }
    if matches!(mode, Mode::Math | Mode::DisplayMath)
        && let Some(request) = processor
            .scan_math_request(command)
            .map_err(command_error)?
    {
        return Ok(retain_cold_operation(
            command,
            cold,
            ColdOperation::<G>::Math(request),
        ));
    }
    if matches!(mode, Mode::Math | Mode::DisplayMath)
        && let ResolvedMeaning::Static(Meaning::CharToken {
            cat: Catcode::Superscript,
            ..
        }) = command.meaning()
    {
        return Ok(retain_cold_operation(
            command,
            cold,
            ColdOperation::<G>::Math(MathRequest::Script(tex_command::ScannedMathScript {
                kind: MathScriptKind::Superscript,
                provenance: tex_command::StructuredProvenance {
                    primary: command.origin(),
                },
            })),
        ));
    }
    if matches!(mode, Mode::Math | Mode::DisplayMath)
        && let ResolvedMeaning::Static(Meaning::CharToken {
            cat: Catcode::Subscript,
            ..
        }) = command.meaning()
    {
        return Ok(retain_cold_operation(
            command,
            cold,
            ColdOperation::<G>::Math(MathRequest::Script(tex_command::ScannedMathScript {
                kind: MathScriptKind::Subscript,
                provenance: tex_command::StructuredProvenance {
                    primary: command.origin(),
                },
            })),
        ));
    }

    if boxes.output_routine_opening_pending
        && matches!(
            command.meaning(),
            ResolvedMeaning::Static(Meaning::CharToken {
                cat: Catcode::BeginGroup,
                ..
            })
        )
    {
        return Ok(retain_cold_operation(
            command,
            cold,
            ColdOperation::<G>::OutputRoutineOpeningBrace,
        ));
    }
    // `align_error`'s inserted brace is an actual execution group, even when
    // it appears inside a replayed box body.  It must therefore win over the
    // box body's brace-depth bookkeeping so §1131 can observe it at end-v.
    if boxes.recovery_simple_group_pending
        && matches!(
            command.meaning(),
            ResolvedMeaning::Static(Meaning::CharToken {
                cat: Catcode::BeginGroup,
                ..
            })
        )
    {
        return Ok(retain_cold_operation(
            command,
            cold,
            ColdOperation::<G>::BeginSimpleGroup,
        ));
    }
    if boxes.recovery_simple_group_open
        && matches!(
            command.meaning(),
            ResolvedMeaning::Static(Meaning::CharToken {
                cat: Catcode::EndGroup,
                ..
            })
        )
    {
        return Ok(retain_cold_operation(
            command,
            cold,
            ColdOperation::<G>::EndSimpleGroup,
        ));
    }
    // TeX82 §1068 dispatches a right brace from the current `cur_group`.
    // An ancestor simple group must not make a nested box's body closer look
    // like an ordinary group closer.
    if innermost_group == Some(GroupKind::Simple)
        && matches!(
            command.meaning(),
            ResolvedMeaning::Static(Meaning::CharToken {
                cat: Catcode::EndGroup,
                ..
            })
        )
    {
        return Ok(retain_hot_operation(
            command,
            hot_apply::HotOperation::<G>::end_ordinary_group(),
        ));
    }
    // TeX82 §1186's `math_group` arm of `handle_right_brace` (the brace that
    // closes a subformula scanned by §1151's `scan_math`) and §1174's
    // `math_choice_group` arm (the brace that closes one `\mathchoice`
    // branch). §1153 and §1172/§1174 opened these levels with `push_math`,
    // so each pair really does bracket a save-stack level and its closer must
    // not fall through to the ordinary or box brace arms below.
    if let Some(kind @ (GroupKind::Math | GroupKind::MathChoice)) = innermost_group
        && matches!(
            command.meaning(),
            ResolvedMeaning::Static(Meaning::CharToken {
                cat: Catcode::EndGroup,
                ..
            })
        )
    {
        return Ok(retain_cold_operation(
            command,
            cold,
            ColdOperation::<G>::EndMathGroup(kind),
        ));
    }
    if innermost_group == Some(GroupKind::Disc)
        && matches!(
            command.meaning(),
            ResolvedMeaning::Static(Meaning::CharToken {
                cat: Catcode::EndGroup,
                ..
            })
        )
    {
        return Ok(retain_cold_operation(
            command,
            cold,
            ColdOperation::<G>::DiscretionaryPartEnd,
        ));
    }
    // TeX82 §1150's `mmode+left_brace`: a bare explicit brace encountered
    // directly in math mode starts a subformula that becomes the nucleus of a
    // freshly appended noad -- `tail_append(new_noad); back_input;
    // scan_math(nucleus(tail))` -- rather than an ordinary `simple_group`
    // scope. This must be checked before the general brace arms below: a math
    // formula nested inside an active box body (for example plain.tex's
    // `\maketable` macro, which replays its whole `\halign` argument inside
    // `\setbox1=\vbox{#2}`) otherwise had its bare `{`/`}` swallowed with no
    // noad ever appended, so a following `^`/`_` incorrectly saw the
    // *enclosing* list's last node (an ordinary character, from *outside*
    // the formula) as its attachment target instead of a fresh empty
    // nucleus. Reusing the existing `TextField(Ord)` request (the same
    // completed-field plumbing `\mathord{...}` already drives) is exact:
    // `scan_math`'s brace case and `\mathord`'s explicit field scan both
    // bottom out in one §1153 `math_group`/`fin_mlist` cycle, and an
    // Ord-classified noad is what an unornamented brace group produces. A
    // box's own mandatory opening brace never reaches this dispatch at all:
    // `scan_left_brace` (TeX82 §403) consumed it while the construction was
    // still being scanned.
    if matches!(mode, Mode::Math | Mode::DisplayMath)
        && let ResolvedMeaning::Static(Meaning::CharToken {
            cat: Catcode::BeginGroup,
            ..
        }) = command.meaning()
    {
        processor
            .back_input(command.take_current())
            .map_err(command_error)?;
        return Ok(retain_cold_operation(
            command,
            cold,
            ColdOperation::<G>::Math(MathRequest::TextField(MathTextFieldKind::Ord)),
        ));
    }
    // TeX82 §1068's `handle_right_brace` dispatches purely on `cur_group`, so
    // a box body's own closing brace is exactly the one delivered while the
    // innermost group is still the group `scan_spec`/`begin_insert_or_adjust`
    // opened for that body. Braces nested inside the body opened ordinary
    // `simple_group` levels of their own (§1063), and §1069's `simple_group:
    // unsave` -- reached through the `EndOrdinaryGroup` arm above -- closes
    // those. No separate brace-depth count is kept: the save stack already
    // holds every open level, and counting braces instead silently skipped
    // `unsave`, losing both the nested group's local restores and the
    // `\aftergroup` tokens §282 backs up when it pops.
    if let Some(box_state) = boxes.active_boxes.last()
        && innermost_group == Some(box_state.group_kind)
        && matches!(
            command.meaning(),
            ResolvedMeaning::Static(Meaning::CharToken {
                cat: Catcode::EndGroup,
                ..
            })
        )
    {
        let threshold = match box_state.kind {
            ReplayBoxKind::VBox | ReplayBoxKind::VTop | ReplayBoxKind::VCenter => 1,
            ReplayBoxKind::Insert(..) => 2,
            ReplayBoxKind::HBox => 0,
        };
        if threshold != 0 && partoken_context_replays(processor, mode, threshold) {
            processor
                .insert_partoken_before(command.take_current())
                .map_err(command_error)?;
            return Ok(retain_cold_operation(
                command,
                cold,
                ColdOperation::<G>::Continue,
            ));
        }
        return Ok(retain_cold_operation(
            command,
            cold,
            ColdOperation::<G>::BoxEndGroup {
                ships_out: box_state.shipout_region.is_some(),
                current_line: i32::try_from(processor.current_file_line_number())
                    .unwrap_or(i32::MAX),
            },
        ));
    }
    // TeX82 §1016 opens `output_group` before replaying the braced output
    // token list. A box body nested in that list owns its closing brace first;
    // only the live output group can close the enclosing output routine.
    if boxes.output_routine_active
        && innermost_group == Some(GroupKind::Output)
        && matches!(
            command.meaning(),
            ResolvedMeaning::Static(Meaning::CharToken {
                cat: Catcode::EndGroup,
                ..
            })
        )
    {
        if partoken_context_replays(processor, mode, 2) {
            processor
                .insert_partoken_before(command.take_current())
                .map_err(command_error)?;
            return Ok(retain_cold_operation(
                command,
                cold,
                ColdOperation::<G>::Continue,
            ));
        }
        return Ok(retain_cold_operation(
            command,
            cold,
            ColdOperation::<G>::EndOutputRoutine,
        ));
    }
    // TeX82 §1090's `@<Cases of |main_control| that build boxes and lists@>`
    // opens with one shared vertical-mode case, not thirteen separate ones:
    //
    //     vmode+letter,vmode+other_char,vmode+char_num,vmode+char_given,
    //     vmode+math_shift,vmode+un_hbox,vmode+vrule,
    //     vmode+accent,vmode+discretionary,vmode+hskip,vmode+valign,
    //     vmode+ex_space,vmode+no_boundary:
    //       begin back_input; new_graf(true); end;
    //
    // Every member takes the same two actions in the same order, *before* any
    // operand of its own is looked at: the triggering command is pushed back
    // (§325 `back_input`, which opens a `backed_up` input level), and §1091
    // `new_graf` then opens the paragraph and pushes `\everypar`. The backed-up
    // command is redelivered afterwards and dispatched again, now in horizontal
    // mode, where it scans its operand.
    //
    // Scanning the operand here instead -- `\char`'s character number,
    // `\hskip`'s glue, `\vrule`'s rule spec, `\accent`'s accent number and base
    // character, `\discretionary`'s three lists -- reads it in vertical mode,
    // before `\everypar` has run and before the paragraph's horizontal list
    // exists, and skips the backup level and redelivery entirely.
    if matches!(mode, Mode::Vertical | Mode::InternalVertical)
        && starts_paragraph_in_vertical_mode(command.meaning())
    {
        processor
            .back_input(command.take_current())
            .map_err(command_error)?;
        return Ok(retain_cold_operation(
            command,
            cold,
            ColdOperation::<G>::ParagraphStart,
        ));
    }
    if hot_apply::scan(
        processor,
        command,
        global,
        flags,
        innermost_group,
        suspended_operation_scan,
    )? {
        return Ok(ScannedOperation::Hot);
    }
    scan_cold_operation(
        processor,
        command,
        cold,
        global,
        mode,
        boxes,
        innermost_group,
        job_is_all_over,
        display_eq_no,
        set_box_allowed,
        shown_mode,
        suspended_operation_scan,
    )?;
    command.mark_resident_cold(cold);
    Ok(ScannedOperation::Cold)
}
