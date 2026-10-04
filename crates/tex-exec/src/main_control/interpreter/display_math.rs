//! Display entry and final packaging consume the actual interrupted paragraph.
use super::*;

pub(super) fn enter<G>(processor: &mut Processor<'_, '_, G>) -> Result<(), ExecError> {
    let (paragraph, dimensions, pre_display_size, prototype, extended) = processor
        .with_execution_owners(
            |context, command, effects, fuel, observer, execution, _, _, _| {
                let diagnostic = crate::diagnostics::ExecutionDiagnosticContext::new(
                    i32::try_from(command.current_file_line_number()).unwrap_or(i32::MAX),
                    0,
                    false,
                    command.output_open_context(context),
                );
                let mut geometry = pack_geometry_sink(command, observer);
                let paragraph = crate::paragraph_end::interrupt_paragraph_for_display(
                    &mut execution.modes,
                    context,
                    effects,
                    &mut geometry,
                    fuel,
                    diagnostic,
                )?;
                let dimensions =
                    crate::paragraph_end::display_line_dimensions(&execution.modes, context);
                let pre_display_size = paragraph
                    .last_line
                    .as_ref()
                    .map_or(Scaled::from_raw(-Scaled::MAX_DIMEN.raw()), |line| {
                        crate::math::display::pre_display_size(context, line)
                    });
                let extended = context.int_param(IntParam::ETEX_EXTENDED_MODE) > 0;
                let prototype = if extended {
                    paragraph
                        .last_line
                        .as_ref()
                        .map(|line| crate::math::display::display_line_prototype(context, *line))
                } else {
                    None
                };
                Ok::<_, ExecError>((paragraph, dimensions, pre_display_size, prototype, extended))
            },
        )?;
    math_shift::enter(processor, true)?;
    processor.with_execution_owners(
        |context, command, effects, _, observer, execution, _, _, _| {
            for (parameter, value) in [
                (DimenParam::PRE_DISPLAY_SIZE, pre_display_size),
                (DimenParam::DISPLAY_WIDTH, dimensions.width),
                (DimenParam::DISPLAY_INDENT, dimensions.indent),
            ] {
                context
                    .assign_dimen_param(parameter, value, tex_state::AssignmentScope::Local)
                    .expect("display dimension parameter is admitted");
            }
            if extended {
                context
                    .assign_int_param(
                        IntParam::PRE_DISPLAY_DIRECTION,
                        match paragraph.active_directions.last() {
                            Some(tex_state::node::Direction::BeginL) => 1,
                            Some(tex_state::node::Direction::BeginR) => -1,
                            _ => 0,
                        },
                        tex_state::AssignmentScope::Local,
                    )
                    .expect("display direction parameter is admitted");
            }
            schedule_everymath(command, context, true);
            execution
                .modes
                .current_list_mutation()
                .set_display_interrupt(
                    context,
                    crate::mode::DisplayInterrupt::new(
                        context,
                        paragraph.active_directions,
                        prototype,
                    ),
                );
            command.publish_named_token_list_pushes(context, effects, observer);
        },
    );
    Ok(())
}

pub(super) fn finish<G>(
    processor: &mut Processor<'_, '_, G>,
    mut content: tex_state::page_node_arena::PageListId,
    eq_no: Option<crate::math::display::FinishedEqNo>,
    fonts_checked: bool,
    display_level: Option<crate::mode::ModeLevelSummary>,
) -> Result<(), ExecError> {
    if !fonts_checked && math_shift::reject_fonts(processor)? {
        content = tex_state::page_node_arena::PageListId::empty();
    }
    let directions =
        processor.with_execution_owners(
            |context, command, effects, fuel, observer, execution, _, _, _| {
                let diagnostic = crate::diagnostics::ExecutionDiagnosticContext::live(
                    i32::try_from(command.current_file_line_number()).unwrap_or(i32::MAX),
                    false,
                    command,
                );
                let mut level = match display_level {
                    Some(level) => level,
                    None => crate::box_runtime::commit_current_list(
                        &mut execution.modes,
                        context,
                        effects,
                        fuel,
                    )?,
                };
                let interrupt = level.list_mutation().take_display_interrupt().ok_or(
                    ExecError::MissingToken {
                        context: "display interrupt",
                    },
                )?;
                let (directions, prototype) = interrupt.into_parts();
                let mut geometry = pack_geometry_sink(command, observer);
                crate::math::display::finish_display_math(
                    &mut execution.modes,
                    context,
                    effects,
                    &mut geometry,
                    &diagnostic,
                    content,
                    eq_no,
                    prototype,
                    Some(command),
                )?;
                Ok::<_, ExecError>(directions)
            },
        )?;
    groups::finish_native_group_leave(processor, GroupKind::MathShift, "display math group")?;
    processor.with_execution_owners(|_, _, _, _, _, execution, _, _, _| {
        execution.active_math_shifts.pop();
    });
    resume(processor, directions, true)
}

pub(super) fn finish_alignment<G>(
    processor: &mut Processor<'_, '_, G>,
    finished: crate::align::FinishedAlignment,
    scan_space: bool,
) -> Result<(), ExecError> {
    let directions =
        processor.with_execution_owners(|context, _, effects, fuel, _, execution, _, _, _| {
            let mut level = crate::box_runtime::commit_current_list(
                &mut execution.modes,
                context,
                effects,
                fuel,
            )?;
            let interrupt =
                level
                    .list_mutation()
                    .take_display_interrupt()
                    .ok_or(ExecError::MissingToken {
                        context: "display alignment interrupt",
                    })?;
            crate::math::display::finish_display_alignment(
                &mut execution.modes,
                context,
                finished,
            )?;
            Ok::<_, ExecError>(interrupt.active_directions)
        })?;
    groups::finish_native_group_leave(processor, GroupKind::MathShift, "display alignment group")?;
    processor.with_execution_owners(|_, _, _, _, _, execution, _, _, _| {
        execution.active_math_shifts.pop();
    });
    resume(processor, directions, scan_space)
}

fn resume<G>(
    processor: &mut Processor<'_, '_, G>,
    directions: Vec<tex_state::node::Direction>,
    scan_space: bool,
) -> Result<(), ExecError> {
    processor.with_execution_owners(|context, command, _, _, _, execution, _, _, _| {
        let prev = execution
            .modes
            .enclosing_vertical_prev_graf()
            .checked_add(3)
            .expect("display prev_graf overflow");
        execution.modes.set_enclosing_vertical_prev_graf(prev);
        execution.modes.push_at_line(
            Mode::Horizontal,
            i32::try_from(command.current_file_line_number()).unwrap_or(i32::MAX),
        )?;
        let (language, left, right) = crate::box_runtime::hmode::current_hyphen_context(context);
        execution
            .modes
            .current_list_mutation()
            .begin_paragraph_hyphen_context(language, left, right);
        execution
            .modes
            .current_list_mutation()
            .set_space_factor(1000);
        execution
            .modes
            .current_list_mutation()
            .append(context, directions.into_iter().map(Node::Direction));
        execution.main_loop_active = false;
        Ok::<_, ExecError>(())
    })?;
    if scan_space {
        trace::prepare(processor);
        processor.consume_expanded_words(&mut |processor, spelling, meaning, _, exceptional| {
            let space = matches!(meaning, tex_state::meaning::MeaningWord::Static(word) if Meaning::runtime_word_control_code(*word) == codes::CHAR_SPACE);
            if !space { processor.back_consumed_word(spelling, exceptional)?; }
            Ok(ControlFlow::Break(()))
        }).map_err(command_error)?;
        report_scan(processor)?;
        if processor.command_trace_printed() {
            processor.with_execution_owners(|_, _, _, _, _, execution, _, _, _| {
                execution.shown_mode = Some(execution.modes.current_mode())
            });
        }
    }
    processor.with_execution_owners(|context, command, effects, _, _, execution, _, _, _| {
        let diagnostic = command.output_open_context(context);
        crate::math::display::build_page_after_display_resume(
            &execution.modes,
            context,
            effects,
            &diagnostic,
        )
    })
}
