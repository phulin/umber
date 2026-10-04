//! Alignment producers drive canonical scans and native groups immediately.
use super::*;

pub(super) fn begin<G>(
    processor: &mut Processor<'_, '_, G>,
    vertical: bool,
    owner: Option<Symbol>,
    receipts: &mut Option<Vec<MutationRecord>>,
) -> Result<MainControlStep, ExecError> {
    let improper = processor.with_execution_owners(
        |context, command, effects, _, _, execution, _, _, _| {
            execution.main_loop_active = false;
            if execution.modes.current_mode() != Mode::DisplayMath
                || (execution.modes.current_list().nodes(context).is_empty()
                    && execution
                        .modes
                        .current_list()
                        .incomplete_fraction()
                        .is_none())
            {
                return Ok::<_, ExecError>(false);
            }
            let primitive = if vertical { "\\valign" } else { "\\halign" };
            let display = command.output_open_context(context);
            let mut report = context.print_err(&format!("Improper {primitive} inside $$'s"));
            report.help(&[
                "Displays can use special alignments (like \\eqalignno)",
                "only if nothing but the alignment itself is between $$'s.",
                "So I've deleted the formulas that preceded this alignment.",
            ]);
            report.context(display);
            report.error().defer_recovery(effects)?;
            let mut list = execution.modes.current_list_mutation();
            list.take_nodes();
            list.take_incomplete_fraction();
            Ok(true)
        },
    )?;
    if improper {
        recovery::finish_error_dialogue(processor)?;
    }
    let identity = processor.with_execution_owners(
        |context, command, _, _, mut observer, execution, _, _, _| {
            if let Some(outer) = execution.active_alignment.take() {
                command
                    .apply_alignment_request(context, AlignmentRequest::Suspend(outer.identity))
                    .map_err(|_| ExecError::MissingToken {
                        context: "nested alignment suspension",
                    })?;
                if let Some(observer) = observer.as_mut()
                    && let Some(record) = command.alignment_suspend_observation()
                {
                    observer.committed(CommandObservation::Alignment(record));
                }
                execution.boxes.suspended_alignments.push(outer);
            }
            let identity = AlignmentIdentity::new(execution.next_alignment_identity);
            execution.next_alignment_identity = execution.next_alignment_identity.wrapping_add(1);
            command
                .apply_alignment_request(context, AlignmentRequest::Begin(identity))
                .map_err(|_| ExecError::MissingToken {
                    context: "alignment lifecycle",
                })?;
            if let Some(observer) = observer
                && let Some(record) = command.alignment_begin_observation()
            {
                observer.committed(CommandObservation::Alignment(record));
            }
            let tabskip = context.glue_param(GlueParam::TAB_SKIP);
            let default_tabskip = tex_state::node::GlueValue {
                spec: tabskip.map_or(GlueSpec::ZERO, |id| context.glue(id)),
                origin: if tabskip.is_none() {
                    tex_state::node::GlueSpecOrigin::SharedZero
                } else {
                    tex_state::node::GlueSpecOrigin::Owned
                },
            };
            let kind = if vertical {
                AlignmentKind::VAlign
            } else {
                AlignmentKind::HAlign
            };
            execution.active_alignment = Some(ActiveReplayAlignment {
                identity,
                kind,
                packing: AlignmentPackSpec::Natural,
                columns: Vec::new(),
                repeat_start: None,
                column: 0,
                noalign_open: false,
                captured_row_count: 0,
                captured_cell_count: 0,
                tabskips: Vec::new(),
                default_tabskip,
                row_migrations: tex_state::page_node_arena::PageListSpan::empty(),
                cell_span: 1,
                row_open: false,
                cell_open: false,
            });
            let prev_depth = if execution.modes.current_mode() == Mode::DisplayMath {
                execution.modes.enclosing_vertical_prev_depth()
            } else {
                execution.modes.current_list().prev_depth()
            };
            execution.modes.push_at_line(
                replay_alignment_mode(kind),
                i32::try_from(command.current_file_line_number()).unwrap_or(i32::MAX),
            )?;
            if let Some(depth) = prev_depth {
                execution
                    .modes
                    .current_list_mutation()
                    .set_prev_depth(depth);
            }
            Ok::<_, ExecError>(identity)
        },
    )?;
    let packing = processor
        .scan_alignment_preamble_opening()
        .map_err(command_error)?;
    report_scan(processor)?;
    processor.with_execution_owners(|context, command, effects, _, _, execution, _, _, _| {
        command
            .apply_alignment_request(context, AlignmentRequest::Preamble(identity))
            .map_err(|_| ExecError::MissingToken {
                context: "alignment preamble lifecycle",
            })?;
        execution
            .active_alignment
            .as_mut()
            .expect("live alignment")
            .packing = alignment_pack_spec(packing);
        enter_group(context, command, effects, GroupKind::Align);
        Ok::<_, ExecError>(())
    })?;
    processor
        .begin_alignment_preamble_scan(owner)
        .map_err(command_error)?;
    report_scan(processor)?;
    processor.with_execution_owners(
        |context, command, effects, _, observer, execution, _, _, _| {
            publication::publish_alignment_preamble(
                command,
                context,
                execution
                    .active_alignment
                    .as_mut()
                    .expect("live alignment preamble"),
            )?;
            enter_group(context, command, effects, GroupKind::Align);
            schedule_everycr(command, context);
            command.publish_named_token_list_pushes(context, effects, observer);
            Ok::<_, ExecError>(())
        },
    )?;
    peek(processor, false, receipts)
}

pub(super) fn peek<G>(
    processor: &mut Processor<'_, '_, G>,
    mut restarting: bool,
    receipts: &mut Option<Vec<MutationRecord>>,
) -> Result<MainControlStep, ExecError> {
    loop {
        processor
            .begin_alignment_peek(restarting)
            .map_err(command_error)?;
        trace::prepare(processor);
        let Some(lookahead) = processor
            .next_alignment_lookahead()
            .map_err(command_error)?
        else {
            return Ok(MainControlStep::EndOfInput);
        };
        report_scan(processor)?;
        let meaning = lookahead.command().meaning();
        match meaning {
            ResolvedMeaning::Static(Meaning::UnexpandablePrimitive(
                UnexpandablePrimitive::CrCr,
            )) => {
                let _ = processor.commit_alignment_lookahead_delivery(lookahead);
                restarting = true;
            }
            ResolvedMeaning::Static(Meaning::UnexpandablePrimitive(
                UnexpandablePrimitive::NoAlign,
            )) => {
                let _ = processor.commit_alignment_lookahead_delivery(lookahead);
                processor
                    .scan_alignment_noalign_opening()
                    .map_err(command_error)?;
                report_scan(processor)?;
                processor.with_execution_owners(
                    |context, command, effects, _, _, execution, _, _, _| {
                        execution
                            .active_alignment
                            .as_mut()
                            .expect("live noalign")
                            .noalign_open = true;
                        enter_group(context, command, effects, GroupKind::NoAlign);
                        if execution.modes.current_mode() == Mode::InternalVertical {
                            crate::paragraph_end::normal_paragraph(
                                &mut execution.modes,
                                context,
                                effects,
                            );
                        }
                        Ok::<_, ExecError>(MainControlStep::Continue)
                    },
                )?;
                return Ok(MainControlStep::Continue);
            }
            ResolvedMeaning::Static(Meaning::CharToken {
                cat: Catcode::EndGroup,
                ..
            }) => {
                let _ = processor.commit_alignment_lookahead_delivery(lookahead);
                return finish(processor, receipts);
            }
            _ => {
                let omit = matches!(
                    meaning,
                    ResolvedMeaning::Static(Meaning::UnexpandablePrimitive(
                        UnexpandablePrimitive::Omit
                    ))
                );
                if omit {
                    let _ = processor.commit_alignment_lookahead_delivery(lookahead);
                } else {
                    processor
                        .back_alignment_lookahead(lookahead)
                        .map_err(command_error)?;
                }
                processor.with_execution_owners(
                    |context, command, effects, _, observer, execution, _, _, _| {
                        let active = execution
                            .active_alignment
                            .as_mut()
                            .expect("live row lookahead");
                        let templates = active.columns.get(active.column).cloned().ok_or(
                            ExecError::MissingToken {
                                context: "next alignment preamble column",
                            },
                        )?;
                        command
                            .begin_prepared_alignment_cell(active.identity, templates)
                            .map_err(|_| ExecError::MissingToken {
                                context: "alignment next-row lifecycle",
                            })?;
                        begin_replay_alignment_cell(
                            active,
                            &mut execution.modes,
                            context,
                            effects,
                        )?;
                        if omit {
                            command
                                .apply_alignment_request(
                                    context,
                                    AlignmentRequest::PrepareCellLookahead(active.identity),
                                )
                                .map_err(|_| ExecError::MissingToken {
                                    context: "alignment omit lookahead",
                                })?;
                        }
                        command
                            .apply_alignment_request(
                                context,
                                if omit {
                                    AlignmentRequest::InstallOmitCellTemplate(active.identity)
                                } else {
                                    AlignmentRequest::InstallCellTemplate(active.identity)
                                },
                            )
                            .map_err(|_| ExecError::MissingToken {
                                context: "alignment cell template",
                            })?;
                        publish_cell_template(
                            command,
                            context,
                            effects,
                            observer,
                            active.identity,
                            omit,
                        );
                        Ok::<_, ExecError>(())
                    },
                )?;
                return Ok(MainControlStep::Continue);
            }
        }
    }
}

pub(super) fn close_noalign<G>(
    processor: &mut Processor<'_, '_, G>,
    receipts: &mut Option<Vec<MutationRecord>>,
) -> Result<MainControlStep, ExecError> {
    processor.with_execution_owners(
        |context, command, effects, fuel, observer, execution, _, _, _| {
            execution.main_loop_active = false;
            let diagnostic =
                command_diagnostic_context(command, execution.boxes.output_routine_active);
            let mut geometry = pack_geometry_sink(command, observer);
            crate::paragraph_end::end_paragraph_with_fuel(
                &mut execution.modes,
                context,
                effects,
                &mut geometry,
                diagnostic,
                fuel,
            )
        },
    )?;
    groups::finish_native_group_leave(processor, GroupKind::NoAlign, "noalign group")?;
    processor.with_execution_owners(|_, _, _, _, _, execution, _, _, _| {
        execution
            .active_alignment
            .as_mut()
            .expect("live noalign group")
            .noalign_open = false
    });
    peek(processor, true, receipts)
}

fn finish<G>(
    processor: &mut Processor<'_, '_, G>,
    receipts: &mut Option<Vec<MutationRecord>>,
) -> Result<MainControlStep, ExecError> {
    groups::finish_native_group_leave(processor, GroupKind::Align, "align1")
        .map_err(|_| ExecError::Fatal(FatalError::confusion("align1")))?;
    groups::finish_native_group_leave(processor, GroupKind::Align, "align0")
        .map_err(|_| ExecError::Fatal(FatalError::confusion("align0")))?;
    let display = processor.with_execution_owners(
        |context, command, effects, fuel, observer, execution, _, _, _| {
            execution.main_loop_active = false;
            let active = execution
                .active_alignment
                .as_mut()
                .ok_or(ExecError::MissingToken {
                    context: "active alignment finish",
                })?;
            let identity = active.identity;
            let diagnostic = command.output_open_context(context);
            let mut geometry = pack_geometry_sink(command, observer);
            finish_replay_alignment(
                active,
                &mut execution.modes,
                context,
                effects,
                &mut geometry,
                fuel,
                AlignmentFinishSite::new(&diagnostic, command.current_file_line_number()),
            )?;
            if let Some(observer) = geometry.observations.as_mut()
                && let Some(record) = command.alignment_finish_observation(identity)
            {
                observer.committed(CommandObservation::Alignment(record));
            }
            command
                .apply_alignment_request(context, AlignmentRequest::Finish(identity))
                .map_err(|_| ExecError::MissingToken {
                    context: "alignment finish lifecycle",
                })?;
            execution.active_alignment = None;
            if let Some(outer) = execution.boxes.suspended_alignments.pop() {
                command
                    .apply_alignment_request(context, AlignmentRequest::Resume(outer.identity))
                    .map_err(|_| ExecError::MissingToken {
                        context: "nested alignment resume",
                    })?;
                if let Some(observer) = geometry.observations.as_mut()
                    && let Some(record) = command.alignment_resume_observation()
                {
                    observer.committed(CommandObservation::Alignment(record));
                }
                execution.active_alignment = Some(outer);
            }
            Ok::<_, ExecError>(execution.modes.current_mode() == Mode::DisplayMath)
        },
    )?;
    if display {
        alignment_columns::close_display(processor, receipts)
    } else {
        Ok(MainControlStep::Continue)
    }
}

/// Evidence is emitted at the actual template insertion, before its tokens run.
pub(super) fn publish_cell_template<G>(
    command: &mut CommandState<G>,
    context: &mut CommandContext<'_, G>,
    effects: &mut DiagnosticEffects,
    observer: Option<&mut dyn CommandObserver>,
    identity: AlignmentIdentity,
    omit: bool,
) {
    if let Some(observer) = observer {
        if let Some(record) = command.alignment_cell_begin_observation() {
            observer.committed(CommandObservation::Alignment(record));
        }
        if omit {
            if let Some(record) = command.alignment_omit_cell_observation(identity) {
                observer.committed(CommandObservation::Alignment(record));
            }
        } else if let Some(record) = command.alignment_u_template_push_observation(identity) {
            observer.committed(CommandObservation::Input(record));
            if let Some(record) = command.alignment_u_template_push_alignment_observation(identity)
            {
                observer.committed(CommandObservation::Alignment(record));
            }
        }
        command.publish_named_token_list_pushes(context, effects, Some(observer));
    } else {
        command.publish_named_token_list_pushes(context, effects, None);
    }
}
