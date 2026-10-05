//! Endv packages the actual cell and directly selects its next template.
use super::*;

pub(super) fn finish_cell<G>(
    processor: &mut Processor<'_, '_, G>,
    receipts: &mut Option<Vec<MutationRecord>>,
) -> Result<MainControlStep, ExecError> {
    let identity =
        processor.with_execution_owners(|context, _, effects, fuel, _, execution, _, _, _| {
            execution.main_loop_active = false;
            if matches!(
                execution.modes.current_mode(),
                Mode::Horizontal | Mode::RestrictedHorizontal
            ) {
                crate::box_runtime::flush_pending_hchars_with_fuel(
                    &mut execution.modes,
                    context,
                    effects,
                    fuel,
                )?;
            }
            execution
                .active_alignment
                .as_ref()
                .map(|active| active.identity)
                .ok_or(ExecError::MissingToken {
                    context: "active endv alignment",
                })
        })?;
    let finished = processor
        .finish_alignment_cell(identity)
        .map_err(command_error)?;
    let delimiter = finished.delimiter;
    let (next_column, extra) = processor.with_execution_owners(
        |context, command, effects, fuel, mut observer, execution, _, _, _| {
            let active = execution
                .active_alignment
                .as_mut()
                .expect("live alignment cell");
            if active.kind == AlignmentKind::VAlign {
                let mut diagnostic = crate::diagnostics::ExecutionDiagnosticContext::source_free(
                    command.output_open_context(context),
                );
                diagnostic.current_line =
                    i32::try_from(finished.delimiter_line).unwrap_or(i32::MAX);
                let mut geometry = pack_geometry_sink(command, reborrow_observer(&mut observer));
                crate::paragraph_end::end_paragraph_with_context(
                    &mut execution.modes,
                    context,
                    effects,
                    &mut geometry,
                    fuel,
                    diagnostic,
                )?;
            }
            let next_column = match delimiter {
                AlignmentCellDelimiter::Tab | AlignmentCellDelimiter::Span => active
                    .column
                    .checked_add(1)
                    .ok_or(ExecError::ArithmeticOverflow)?,
                AlignmentCellDelimiter::Row => 0,
            };
            let extra = next_column >= active.columns.len()
                && active.repeat_start.is_none()
                && matches!(
                    delimiter,
                    AlignmentCellDelimiter::Tab | AlignmentCellDelimiter::Span
                );
            if extra {
                report_extra_alignment_tab(command, effects, context)?;
            }
            Ok::<_, ExecError>((next_column, extra))
        },
    )?;
    if extra {
        recovery::finish_error_dialogue(processor)?;
    }
    let delimiter = if extra {
        AlignmentCellDelimiter::Row
    } else {
        delimiter
    };
    if delimiter != AlignmentCellDelimiter::Span {
        groups::leave(processor, GroupKind::Align, "alignment entry group")?;
        processor.with_execution_owners(|context, command, effects, _, _, _, _, _, _| {
            enter_group(context, command, effects, GroupKind::Align)
        });
    }
    let row = processor.with_execution_owners(
        |context, command, effects, fuel, mut observer, execution, _, _, _| {
            let active = execution
                .active_alignment
                .as_mut()
                .expect("live next alignment column");
            if extra {
                command
                    .apply_alignment_request(context, AlignmentRequest::RecoverExtraTab(identity))
                    .map_err(|_| ExecError::MissingToken {
                        context: "alignment extra-tab recovery",
                    })?;
            }
            if extra
                && let Some(record) = command.take_alignment_extra_tab_recovery_observation()
                && let Some(observer) = observer.as_mut()
            {
                observer.committed(CommandObservation::Alignment(record));
            }
            if delimiter == AlignmentCellDelimiter::Span {
                active.cell_span = active
                    .cell_span
                    .checked_add(1)
                    .ok_or(ExecError::ArithmeticOverflow)?;
            } else {
                let mut geometry = pack_geometry_sink(command, reborrow_observer(&mut observer));
                capture_replay_alignment_cell(
                    active,
                    &mut execution.modes,
                    context,
                    effects,
                    &mut geometry,
                    fuel,
                )?;
            }
            active.column = if extra {
                0
            } else if next_column < active.columns.len() {
                next_column
            } else if let Some(repeat_start) = active.repeat_start {
                let repeat_len = active
                    .columns
                    .len()
                    .checked_sub(repeat_start)
                    .filter(|len| *len != 0)
                    .ok_or(ExecError::MissingToken {
                        context: "alignment periodic-preamble columns",
                    })?;
                repeat_start + (next_column - repeat_start) % repeat_len
            } else {
                next_column
            };
            let row = extra || delimiter == AlignmentCellDelimiter::Row;
            if row {
                let mut geometry = pack_geometry_sink(command, reborrow_observer(&mut observer));
                finish_replay_alignment_row(
                    active,
                    &mut execution.modes,
                    context,
                    effects,
                    &mut geometry,
                    fuel,
                )?;
                active.column = 0;
                schedule_everycr(command, context);
                command.publish_named_token_list_pushes(context, effects, observer);
            } else {
                let templates =
                    active
                        .columns
                        .get(active.column)
                        .cloned()
                        .ok_or(ExecError::MissingToken {
                            context: "next alignment preamble column",
                        })?;
                command
                    .begin_prepared_alignment_cell(identity, templates)
                    .map_err(|_| ExecError::MissingToken {
                        context: "alignment next-cell lifecycle",
                    })?;
                if let Some(observer) = observer.as_mut()
                    && let Some(record) = command.alignment_cell_begin_observation()
                {
                    observer.committed(CommandObservation::Alignment(record));
                }
                if delimiter == AlignmentCellDelimiter::Tab {
                    begin_replay_alignment_cell(active, &mut execution.modes, context, effects)?;
                }
            }
            Ok::<_, ExecError>(row)
        },
    )?;
    if row {
        return alignment::peek(processor, receipts);
    }
    let opening = processor
        .scan_alignment_next_cell_opening()
        .map_err(command_error)?;
    report_scan(processor)?;
    processor.with_execution_owners(|context, command, effects, _, observer, _, _, _, _| {
        command
            .apply_alignment_request(
                context,
                match opening {
                    AlignmentCellOpening::Template => {
                        AlignmentRequest::InstallCellTemplate(identity)
                    }
                    AlignmentCellOpening::Omit => {
                        AlignmentRequest::InstallOmitCellTemplate(identity)
                    }
                },
            )
            .map_err(|_| ExecError::MissingToken {
                context: "alignment next-cell template",
            })?;
        alignment::publish_cell_template(
            command,
            context,
            effects,
            observer,
            identity,
            opening == AlignmentCellOpening::Omit,
        );
        Ok(MainControlStep::Continue)
    })
}

pub(super) fn close_display<G>(
    processor: &mut Processor<'_, '_, G>,
    receipts: &mut Option<Vec<MutationRecord>>,
) -> Result<MainControlStep, ExecError> {
    let mut result = None;

    processor
        .consume_main_control(&mut tex_command::terminal_consumer::<
            Processor<'_, '_, G>,
            G,
            _,
        >(
            |processor, spelling, meaning, code, symbol, exceptional| {
                if matches!(code, codes::RELAX | codes::CHAR_SPACE) {
                    return Ok(ControlFlow::Continue(()));
                }
                let assignment = meaning.is_font()
                    || meaning.scalar_word().is_some_and(|word| {
                        tex_command::exceeds_max_non_prefixed_command(Meaning::from_runtime_word(
                            word,
                        ))
                    });
                if assignment {
                    processor.with_execution_owners(|_, _, _, _, _, execution, _, _, _| {
                        execution.set_box_forbidden_depth += 1
                    });
                    let step = dispatch::execute(
                        processor,
                        spelling,
                        meaning,
                        symbol,
                        exceptional,
                        receipts,
                        code,
                    );
                    processor.with_execution_owners(|_, _, _, _, _, execution, _, _, _| {
                        execution.set_box_forbidden_depth -= 1
                    });
                    if let Err(error) = step {
                        result = Some(Err(error));
                        return Ok(ControlFlow::Break(()));
                    }

                    return Ok(ControlFlow::Continue(()));
                }
                let shift = code == codes::CHAR_MATH_SHIFT;
                result = Some(if shift {
                    math_shift::apply(processor, spelling, meaning, symbol, exceptional)
                } else {
                    recover_display(processor, spelling, exceptional)
                });
                Ok(ControlFlow::Break(()))
            },
        ))
        .map_err(command_error)?;
    result.unwrap_or(Ok(MainControlStep::EndOfInput))
}

fn recover_display<G>(
    processor: &mut Processor<'_, '_, G>,
    spelling: TracedTokenWord,
    exceptional: Option<&tex_command::CurrentCommand<G>>,
) -> Result<MainControlStep, ExecError> {
    processor
        .back_consumed_word(spelling, exceptional)
        .map_err(command_error)?;
    processor.with_execution_owners(|context, command, effects, _, _, _, _, _, _| {
        crate::error_report::report_error(
            context,
            effects,
            "Missing $$ inserted",
            &[
                "Displays can use special alignments (like \\eqalignno)",
                "only if nothing but the alignment itself is between $$'s.",
            ],
            command.output_open_context(context),
        )
    })?;
    recovery::finish_error_dialogue(processor)?;
    let (nodes, aux_prev_depth) =
        processor.with_execution_owners(|_, _, _, _, _, execution, _, _, _| {
            execution
                .modes
                .current_list_mutation()
                .take_display_alignment()
                .expect("completed display alignment")
        });
    display_math::finish_alignment(
        processor,
        crate::align::FinishedAlignment {
            nodes,
            aux_prev_depth,
            aux_space_factor: None,
        },
        true,
    )?;
    Ok(MainControlStep::Continue)
}
