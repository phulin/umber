//! Actual group entry, warning and native unsave semantics.
use super::*;

impl<G> MainControl<G> {}

/// Unsave through the actual scanner that owns aftergroup input delivery.
pub(super) fn finish_native_group_leave<G, Facts: tex_command::CommandHostFacts<G>>(
    processor: &mut CommandProcessor<'_, '_, G, Facts>,
    kind: GroupKind,
    error_context: &'static str,
) -> Result<MainControlStep, ExecError> {
    let (level, frame) =
        processor.with_execution_owners(|context, _, _, _, _observer, _execution, _, _, _| {
            let frames = context.group_frames();
            (frames.len(), frames.last().copied())
        });
    if let Some(frame) = frame {
        processor.warn_cross_file_group_close(
            level,
            frame.kind().group_text(),
            frame.entered_line(),
        );
    }
    let result = processor.end_group(kind, crate::assignments::tracing::trace_group_restoration);
    let frame = result.map_err(|_| ExecError::MissingToken {
        context: error_context,
    })?;
    processor.with_execution_owners(|context, _, effects, _, _observer, _execution, _, _, _| {
        context.trace_group_leave(
            effects,
            frame.kind(),
            u32::try_from(level).unwrap_or(u32::MAX),
            frame.entered_line(),
        );
    });
    Ok(MainControlStep::Continue)
}

/// Source-free group entry through the sole actual mode and save-stack owners.
pub(in crate::main_control) fn begin_native_group<G>(
    context: &mut CommandContext<'_, G>,
    command: &mut tex_command::CommandState<G>,
    effects: &mut DiagnosticEffects,
    fuel: &mut tex_command::CommandFuel,
    modes: &mut ModeNest,
    kind: GroupKind,
) -> Result<MainControlStep, ExecError> {
    if modes.current_list().pending_hchars().is_some() {
        crate::box_runtime::flush_pending_hchars_with_fuel(modes, context, effects, fuel)?;
    }
    enter_group(context, command, effects, kind);
    Ok(MainControlStep::Continue)
}

pub(super) fn close_tex_group<G>(
    processor: &mut Processor<'_, '_, G>,
    code: u64,
    spelling: TracedTokenWord,
    meaning: &tex_state::meaning::MeaningWord<G>,
    control_sequence: Option<Symbol>,
    exceptional: Option<&tex_command::CurrentCommand<G>>,
    receipts: &mut Option<Vec<MutationRecord>>,
) -> Result<MainControlStep, ExecError> {
    let group = processor
        .with_execution_owners(|context, _, _, _, _, _, _, _, _| context.innermost_group_kind());
    match (code, group) {
        (codes::CHAR_END_GROUP, Some(GroupKind::Simple))
        | (codes::PRIMITIVE_END_GROUP, Some(GroupKind::SemiSimple)) => {
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
            finish_native_group_leave(
                processor,
                group.expect("selected native group"),
                "ordinary group",
            )?;
            processor.with_execution_owners(|_, _, _, _, _, execution, _, _, _| {
                execution.boxes.recovery_simple_group_open = false
            });
            Ok(MainControlStep::Continue)
        }
        (codes::CHAR_END_GROUP, Some(GroupKind::Align)) => {
            alignment_recovery::missing_cr(processor, exceptional)
        }
        (codes::CHAR_END_GROUP, Some(GroupKind::NoAlign)) => {
            let mode = processor.with_execution_owners(|_, _, _, _, _, execution, _, _, _| {
                execution.modes.current_mode()
            });
            if partoken_context_replays(processor, mode, 2) {
                processor
                    .recover_stop_for_vertical_mode(spelling, exceptional)
                    .map_err(command_error)?;
                return Ok(MainControlStep::Continue);
            }
            alignment::close_noalign(processor, receipts)
        }
        (codes::CHAR_END_GROUP, Some(GroupKind::Math)) => math::close_field(processor),
        (codes::CHAR_END_GROUP, Some(GroupKind::MathChoice)) => math::close_choice(processor),
        (codes::CHAR_END_GROUP, Some(GroupKind::Disc)) => discretionary::close_part(processor),
        (codes::CHAR_END_GROUP, Some(GroupKind::Output)) => {
            let mode = processor.with_execution_owners(|_, _, _, _, _, execution, _, _, _| {
                execution.modes.current_mode()
            });
            if partoken_context_replays(processor, mode, 2) {
                processor
                    .recover_stop_for_vertical_mode(spelling, exceptional)
                    .map_err(command_error)?;
                return Ok(MainControlStep::Continue);
            }
            close_output(processor)
        }
        (codes::CHAR_END_GROUP, Some(kind))
            if processor.with_execution_owners(|_, _, _, _, _, execution, _, _, _| {
                execution
                    .boxes
                    .active_boxes
                    .last()
                    .is_some_and(|active| active.group_kind == kind)
            }) =>
        {
            let (mode, kind) =
                processor.with_execution_owners(|_, _, _, _, _, execution, _, _, _| {
                    (
                        execution.modes.current_mode(),
                        execution
                            .boxes
                            .active_boxes
                            .last()
                            .expect("selected box group")
                            .kind,
                    )
                });
            let threshold = match kind {
                ReplayBoxKind::VBox | ReplayBoxKind::VTop | ReplayBoxKind::VCenter => 1,
                ReplayBoxKind::Insert(..) => 2,
                ReplayBoxKind::HBox => 0,
            };
            if threshold != 0 && partoken_context_replays(processor, mode, threshold) {
                processor
                    .recover_stop_for_vertical_mode(spelling, exceptional)
                    .map_err(command_error)?;
                return Ok(MainControlStep::Continue);
            }
            boxes::close(processor, receipts)
        }
        (codes::PRIMITIVE_END_GROUP, _) => {
            recovery::off_save(processor, spelling, meaning, control_sequence, exceptional)
        }
        _ => extra_right_brace(processor, group),
    }
}

fn extra_right_brace<G>(
    processor: &mut Processor<'_, '_, G>,
    group: Option<GroupKind>,
) -> Result<MainControlStep, ExecError> {
    processor.with_execution_owners(|context, command, effects, _, _, execution, _, _, _| {
        execution.main_loop_active = false;
        let forgotten = match group {
            Some(GroupKind::SemiSimple) => Some(ForgottenGroupOpener::EndGroup),
            Some(GroupKind::MathShift) => Some(ForgottenGroupOpener::MathShift),
            Some(GroupKind::MathLeft) => Some(ForgottenGroupOpener::Right),
            _ => None,
        };
        let display = command.output_open_context(context);
        if let Some(forgotten) = forgotten {
            let mut report = context.print_err("Extra }, or forgotten ");
            forgotten.print(&mut report);
            report.help(&[
                "I've deleted a group-closing symbol because it seems to be",
                "spurious, as in `$x}$'. But perhaps the } is legitimate and",
                "you forgot something else, as in `\\hbox{$x}'. In such cases",
                "the way to recover is to insert both the forgotten and the",
                "deleted material, e.g., by typing `I$}'.",
            ]);
            report.context(display);
            report.error().defer_recovery(effects)?;
        } else {
            crate::error_report::report_error(
                context,
                effects,
                "Too many }'s",
                &[
                    "You've closed more groups than you opened.",
                    "Such booboos are generally harmless, so keep going.",
                ],
                display,
            )?;
        }
        Ok::<_, ExecError>(())
    })?;
    recovery::finish_error_dialogue(processor)?;
    Ok(MainControlStep::Continue)
}

fn close_output<G>(processor: &mut Processor<'_, '_, G>) -> Result<MainControlStep, ExecError> {
    let output_context =
        processor.with_execution_owners(|context, command, _, _, _, _, _, _, _| {
            command.output_open_context(context)
        });
    let unbalanced = processor
        .finish_selected_output_routine()
        .map_err(command_error)?;
    if unbalanced {
        processor.with_execution_owners(|context, _, effects, _, _, _, _, _, _| {
            crate::error_report::report_error(
                context,
                effects,
                "Unbalanced output routine",
                &[
                    "Your sneaky output routine has problematic {'s and/or }'s.",
                    "I can't handle that very well; good luck.",
                ],
                output_context,
            )
        })?;
        recovery::finish_error_dialogue(processor)?;
    }
    let (mut level, output_context) = processor.with_execution_owners(
        |context, command, effects, fuel, observer, execution, _, _, _| {
            let output_context = command.output_close_context(context);
            let diagnostic_context =
                command_diagnostic_context(command, execution.boxes.output_routine_active);
            let mut geometry = pack_geometry_sink(command, observer);
            crate::paragraph_end::end_paragraph_with_fuel(
                &mut execution.modes,
                context,
                effects,
                &mut geometry,
                diagnostic_context,
                fuel,
            )?;
            let level = crate::box_runtime::commit_current_list(
                &mut execution.modes,
                context,
                effects,
                fuel,
            )?;
            Ok::<_, ExecError>((level, output_context))
        },
    )?;
    finish_native_group_leave(processor, GroupKind::Output, "output routine group")?;
    processor.with_execution_owners(|context, _, effects, _, _, execution, _, _, _| {
        execution.main_loop_active = false;
        execution.boxes.output_routine_active = false;
        crate::page_output::resume_page_builder_after_output(
            context,
            effects,
            level.list_mutation().take_nodes(),
            crate::diagnostics::ExecutionDiagnosticContext::source_free(output_context),
        )?;
        Ok(MainControlStep::Continue)
    })
}
