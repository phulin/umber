//! Math shifts keep scanning, font checks and native unsave on one processor.
use super::*;

pub(super) fn apply<G>(
    processor: &mut Processor<'_, '_, G>,
    spelling: TracedTokenWord,
    meaning: &tex_state::meaning::MeaningWord<G>,
    symbol: Option<Symbol>,
    exceptional: Option<&tex_command::CurrentCommand<G>>,
) -> Result<MainControlStep, ExecError> {
    let (mode, group, equation) =
        processor.with_execution_owners(|context, _, _, _, _, execution, _, _, _| {
            (
                execution.modes.current_mode(),
                context.innermost_group_kind(),
                execution.modes.current_list().display_eq_no().is_some(),
            )
        });
    match mode {
        Mode::Vertical | Mode::InternalVertical => {
            return recovery::start_paragraph_and_reswitch(
                processor,
                spelling,
                meaning,
                symbol,
                exceptional,
            );
        }
        Mode::Horizontal | Mode::RestrictedHorizontal => {
            let display = processor
                .scan_init_math_display_pair(mode == Mode::Horizontal)
                .map_err(command_error)?;
            report_scan(processor)?;
            processor.with_execution_owners(
                |context, _, effects, fuel, _, execution, _, _, _| {
                    execution.main_loop_active = false;
                    crate::box_runtime::flush_pending_hchars_with_fuel(
                        &mut execution.modes,
                        context,
                        effects,
                        fuel,
                    )
                },
            )?;
            if display {
                display_math::enter(processor)?;
            } else {
                enter(processor, false)?;
                processor.with_execution_owners(
                    |context, command, effects, _, observer, _, _, _, _| {
                        schedule_everymath(command, context, false);
                        command.publish_named_token_list_pushes(context, effects, observer);
                    },
                );
            }
        }
        Mode::Math | Mode::DisplayMath => {
            if group != Some(GroupKind::MathShift) {
                return recovery::off_save(processor, spelling, meaning, symbol, exceptional);
            }
            if equation {
                finish_equation_number(processor)?;
            } else if mode == Mode::Math {
                finish_inline(processor)?;
            } else {
                let alignment =
                    processor.with_execution_owners(|_, _, _, _, _, execution, _, _, _| {
                        execution
                            .modes
                            .current_list_mutation()
                            .take_display_alignment()
                    });
                if let Some((nodes, aux_prev_depth)) = alignment {
                    probe_end(processor)?;
                    display_math::finish_alignment(
                        processor,
                        crate::align::FinishedAlignment {
                            nodes,
                            aux_prev_depth,
                            aux_space_factor: None,
                        },
                        true,
                    )?;
                } else {
                    let content = prepare_list(processor)?;
                    let level = processor.with_execution_owners(
                        |context, _, effects, fuel, _, execution, _, _, _| {
                            crate::box_runtime::commit_current_list(
                                &mut execution.modes,
                                context,
                                effects,
                                fuel,
                            )
                        },
                    )?;
                    probe_end(processor)?;
                    display_math::finish(processor, content, None, true, Some(level))?;
                }
            }
        }
    }
    Ok(MainControlStep::Continue)
}

pub(super) fn enter<G>(
    processor: &mut Processor<'_, '_, G>,
    display: bool,
) -> Result<(), ExecError> {
    processor.with_execution_owners(|context, command, effects, _, _, execution, _, _, _| {
        execution.main_loop_active = false;
        enter_group(context, command, effects, GroupKind::MathShift);
        context
            .assign_int_param(IntParam::FAM, -1, tex_state::AssignmentScope::Local)
            .expect("family parameter is admitted");
        execution.modes.push_at_line(
            if display {
                Mode::DisplayMath
            } else {
                Mode::Math
            },
            i32::try_from(command.current_file_line_number()).unwrap_or(i32::MAX),
        )?;
        execution.active_math_shifts.push(if display {
            MathShiftContext::Display
        } else {
            MathShiftContext::Inline
        });
        Ok(())
    })
}

pub(super) fn reject_fonts<G>(processor: &mut Processor<'_, '_, G>) -> Result<bool, ExecError> {
    let rejected =
        processor.with_execution_owners(|context, command, effects, _, _, _, _, _, _| {
            let Some(failure) = crate::math::math_font_failure(context) else {
                return Ok::<_, ExecError>(false);
            };
            let display = command.output_open_context(context);
            context.publish_diagnostic_effects_before_synchronous_print(effects);
            let (message, help) = failure.report();
            crate::error_report::report_error(context, effects, message, &help, display)?;
            Ok(true)
        })?;
    if rejected {
        recovery::finish_error_dialogue(processor)?;
    }
    Ok(rejected)
}

pub(super) fn prepare_list<G>(
    processor: &mut Processor<'_, '_, G>,
) -> Result<tex_state::page_node_arena::PageListId, ExecError> {
    let rejected = reject_fonts(processor)?;
    processor.with_execution_owners(|context, _, _, _, _, execution, _, _, _| {
        let mut list = execution.modes.current_list_mutation();
        let content =
            finish_math_list(list.take_nodes(), list.take_incomplete_fraction(), context)?;
        Ok(if rejected {
            tex_state::page_node_arena::PageListId::empty()
        } else {
            content
        })
    })
}

pub(super) fn probe_end<G>(processor: &mut Processor<'_, '_, G>) -> Result<(), ExecError> {
    let paired = processor
        .scan_display_end_math_shift()
        .map_err(command_error)?;
    report_scan(processor)?;
    if !paired {
        processor.with_execution_owners(|context, command, effects, _, _, _, _, _, _| {
            crate::error_report::report_error(
                context,
                effects,
                "Display math should end with $$",
                &[
                    "The `$' that I just saw supposedly matches a previous `$$'.",
                    "So I shall assume that you typed `$$' both times.",
                ],
                command.output_open_context(context),
            )
        })?;
        recovery::finish_error_dialogue(processor)?;
    }
    Ok(())
}

fn finish_inline<G>(processor: &mut Processor<'_, '_, G>) -> Result<(), ExecError> {
    let content = prepare_list(processor)?;
    processor.with_execution_owners(
        |context, command, effects, fuel, observer, execution, _, _, _| {
            crate::box_runtime::commit_current_list(&mut execution.modes, context, effects, fuel)?;
            let insert_penalties = execution.modes.current_mode() == Mode::Horizontal;
            let mut geometry = pack_geometry_sink(command, observer);
            let (nodes, _) = crate::math::finish_inline_math_list_node(
                context,
                effects,
                &mut geometry,
                tex_state::math::MathListNode {
                    display: false,
                    content,
                },
                insert_penalties,
                command,
            );
            let nodes = context.reclaim_unique_page_list(nodes);
            execution
                .modes
                .current_list_mutation()
                .append_unique_list(context, nodes);
            execution
                .modes
                .current_list_mutation()
                .set_space_factor(1000);
            execution.main_loop_active = false;
            Ok::<_, ExecError>(())
        },
    )?;
    groups::leave(processor, GroupKind::MathShift, "math shift group")?;
    processor.with_execution_owners(|_, _, _, _, _, execution, _, _, _| {
        execution.active_math_shifts.pop();
    });
    Ok(())
}

fn finish_equation_number<G>(processor: &mut Processor<'_, '_, G>) -> Result<(), ExecError> {
    let content = prepare_list(processor)?;
    let eq =
        processor.with_execution_owners(|context, _, effects, fuel, _, execution, _, _, _| {
            let mut level = crate::box_runtime::commit_current_list(
                &mut execution.modes,
                context,
                effects,
                fuel,
            )?;
            Ok::<_, ExecError>(
                level
                    .list_mutation()
                    .take_display_eq_no()
                    .expect("equation number mode state"),
            )
        })?;
    probe_end(processor)?;
    let finished =
        processor.with_execution_owners(|context, command, effects, _, observer, _, _, _, _| {
            let diagnostic = crate::diagnostics::ExecutionDiagnosticContext::live(
                i32::try_from(command.current_file_line_number()).unwrap_or(i32::MAX),
                false,
                command,
            );
            let mut geometry = pack_geometry_sink(command, observer);
            crate::math::display::finish_eq_no(
                context,
                effects,
                &mut geometry,
                &diagnostic,
                eq.side,
                content,
                Some(command),
            )
        });
    groups::leave(processor, GroupKind::MathShift, "equation number group")?;
    processor.with_execution_owners(|_, _, _, _, _, execution, _, _, _| {
        execution.active_math_shifts.pop();
    });
    display_math::finish(processor, eq.display.list(), Some(finished), false, None)
}
