//! Actual math values and field destinations in the persistent interpreter.
use super::*;

pub(super) fn choice<G>(
    processor: &mut Processor<'_, '_, G>,
) -> Result<MainControlStep, ExecError> {
    report_scan(processor)?;
    processor.with_execution_owners(|context, _, _, _, _, execution, _, _, _| {
        let empty = tex_state::page_node_arena::PageListId::empty();
        execution.modes.current_list_mutation().push(
            context,
            Node::MathChoice(MathChoice {
                display: empty,
                text: empty,
                script: empty,
                script_script: empty,
            }),
        );
        execution.active_math_choices.push(0);
    });
    open_choice_branch(processor)
}

fn open_choice_branch<G>(
    processor: &mut Processor<'_, '_, G>,
) -> Result<MainControlStep, ExecError> {
    processor.with_execution_owners(
        |context, command, effects, _, observer, execution, _, _, _| {
            execution.main_loop_active = false;
            enter_group(context, command, effects, GroupKind::MathChoice);
            execution.modes.push_at_line(
                Mode::Math,
                command
                    .current_file_line_number()
                    .try_into()
                    .unwrap_or(i32::MAX),
            )?;
            command.publish_named_token_list_pushes(context, effects, observer);
            Ok::<(), ExecError>(())
        },
    )?;
    processor.scan_math_choice_group().map_err(command_error)?;
    report_scan(processor)?;
    Ok(MainControlStep::Continue)
}

pub(super) fn close_choice<G>(
    processor: &mut Processor<'_, '_, G>,
) -> Result<MainControlStep, ExecError> {
    groups::leave(processor, GroupKind::MathChoice, "math choice group")?;
    let complete =
        processor.with_execution_owners(|context, _, effects, fuel, _, execution, _, _, _| {
            let mut level = crate::box_runtime::commit_current_list(
                &mut execution.modes,
                context,
                effects,
                fuel,
            )?;
            let (nodes, incomplete) = {
                let mut list = level.list_mutation();
                (list.take_nodes(), list.take_incomplete_fraction())
            };
            let list = finish_math_list(nodes, incomplete, context)?;
            let branch = execution
                .active_math_choices
                .last_mut()
                .expect("live math choice branch");
            execution
                .modes
                .current_list_mutation()
                .with_last_node_mut(context, |node| {
                    let Node::MathChoice(choice) = node else {
                        unreachable!("choice destination remains in enclosing math list")
                    };
                    match *branch {
                        0 => choice.display = list,
                        1 => choice.text = list,
                        2 => choice.script = list,
                        3 => choice.script_script = list,
                        _ => unreachable!("four math choice branches"),
                    }
                })
                .expect("live math choice destination");
            *branch += 1;
            let complete = *branch == 4;
            if complete {
                execution.active_math_choices.pop();
            }
            execution.main_loop_active = false;
            Ok::<bool, ExecError>(complete)
        })?;
    if complete {
        Ok(MainControlStep::Continue)
    } else {
        open_choice_branch(processor)
    }
}

pub(super) fn text_accent<G>(
    processor: &mut Processor<'_, '_, G>,
) -> Result<MainControlStep, ExecError> {
    processor.with_execution_owners(|context, command, effects, _, _, _, _, _, _| {
        let display = command.output_open_context(context);
        let mut report = context.print_err("Please use \\mathaccent for accents in math mode");
        report.help(&[
            "I'm changing \\accent to \\mathaccent here; wish me luck.",
            "(Accents are not the same in formulas as they are in text.)",
        ]);
        report.context(display);
        report.error().defer_recovery(effects)
    })?;
    recovery::finish_error_dialogue(processor)?;
    let scanned = processor.scan_math_character().map_err(command_error)?;
    let accent = processor
        .with_execution_owners(|context, _, _, _, _, _, _, _, _| {
            math_char(context, u32::from(scanned.code), scanned.provenance.primary)
        })?
        .1;
    nucleus(processor, NoadKind::Accent { accent })
}

pub(super) fn equation_number<G>(
    processor: &mut Processor<'_, '_, G>,
    side: crate::mode::EqNoSide,
    spelling: TracedTokenWord,
) -> Result<MainControlStep, ExecError> {
    report_scan(processor)?;
    processor.with_execution_owners(|context, command, effects, _, _, execution, _, _, _| {
        execution.main_loop_active = false;
        if execution.modes.current_mode() != Mode::DisplayMath {
            let display = command.output_open_context(context);
            crate::diagnostics::report_illegal_case_with_context(
                context,
                effects,
                spelling.semantic_token(),
                execution.modes.current_mode(),
                Some(display),
            )?;
            return Ok::<MainControlStep, ExecError>(MainControlStep::Continue);
        }
        let (nodes, incomplete) = {
            let mut list = execution.modes.current_list_mutation();
            (list.take_nodes(), list.take_incomplete_fraction())
        };
        let display = finish_math_list(nodes, incomplete, context)?;
        enter_group(context, command, effects, GroupKind::MathShift);
        context
            .assign_int_param(IntParam::FAM, -1, tex_state::AssignmentScope::Local)
            .expect("family parameter is admitted");
        execution.modes.push_at_line(
            Mode::Math,
            command
                .current_file_line_number()
                .try_into()
                .unwrap_or(i32::MAX),
        )?;
        execution
            .active_math_shifts
            .push(MathShiftContext::EqNo(side));
        let display = context
            .admit_page_node_span(display)
            .expect("equation-number display belongs to live page owner");
        execution
            .modes
            .current_list_mutation()
            .set_display_eq_no(context, crate::mode::DisplayEqNo { side, display });
        Ok::<MainControlStep, ExecError>(MainControlStep::Continue)
    })?;
    recovery::finish_error_dialogue(processor)?;
    Ok::<MainControlStep, ExecError>(MainControlStep::Continue)
}

pub(super) fn character<G>(
    processor: &mut Processor<'_, '_, G>,
    code: u32,
    origin: OriginId,
) -> Result<MainControlStep, ExecError> {
    report_scan(processor)?;
    processor.with_execution_owners(|context, _, _, _, _, execution, _, _, _| {
        execution.main_loop_active = false;
        append_math_char(
            execution.modes.current_list_mutation(),
            context,
            code,
            origin,
        )?;
        Ok::<MainControlStep, ExecError>(MainControlStep::Continue)
    })
}

pub(super) fn style<G>(
    processor: &mut Processor<'_, '_, G>,
    style: MathStyle,
) -> Result<MainControlStep, ExecError> {
    report_scan(processor)?;
    processor.with_execution_owners(|context, _, _, _, _, execution, _, _, _| {
        execution.main_loop_active = false;
        execution
            .modes
            .current_list_mutation()
            .push(context, Node::MathStyle(style));
        Ok::<MainControlStep, ExecError>(MainControlStep::Continue)
    })
}

pub(super) fn nucleus<G>(
    processor: &mut Processor<'_, '_, G>,
    kind: NoadKind,
) -> Result<MainControlStep, ExecError> {
    report_scan(processor)?;
    let simplify_accent = kind == NoadKind::Normal(NoadClass::Ord);
    let node_index = processor.with_execution_owners(|context, _, _, _, _, execution, _, _, _| {
        execution.main_loop_active = false;
        let index = execution.modes.current_list().nodes(context).len();
        execution.modes.current_list_mutation().push(
            context,
            Node::MathNoad(MathNoad::new(kind, MathField::Empty)),
        );
        index
    });
    let field = processor.scan_math_field_episode().map_err(command_error)?;
    report_scan(processor)?;
    accept_field(
        processor,
        field,
        ActiveMathFieldTarget::Nucleus {
            node_index,
            simplify_accent,
        },
    )?;
    Ok::<MainControlStep, ExecError>(MainControlStep::Continue)
}

fn accept_field<G>(
    processor: &mut Processor<'_, '_, G>,
    episode: tex_command::MathFieldEpisode,
    target: ActiveMathFieldTarget,
) -> Result<(), ExecError> {
    processor.with_execution_owners(
        |context, command, effects, _, observer, execution, _, _, _| {
            let field = match episode.body {
                MathFieldBody::Missing => MathField::Empty,
                MathFieldBody::Character(code) => MathField::MathChar(
                    math_char(context, u32::from(code), episode.provenance.primary)?.1,
                ),
                MathFieldBody::OpenGroup => {
                    enter_group(context, command, effects, GroupKind::Math);
                    execution.modes.push_at_line(
                        Mode::Math,
                        command
                            .current_file_line_number()
                            .try_into()
                            .unwrap_or(i32::MAX),
                    )?;
                    execution.active_math_fields.push(target);
                    execution.main_loop_active = false;
                    command.publish_named_token_list_pushes(context, effects, observer);
                    return Ok(());
                }
            };
            fill_math_field_target(&mut execution.modes, context, target, field);
            Ok(())
        },
    )
}

pub(super) fn close_field<G>(
    processor: &mut Processor<'_, '_, G>,
) -> Result<MainControlStep, ExecError> {
    groups::leave(processor, GroupKind::Math, "math group")?;
    processor.with_execution_owners(|context, _, effects, fuel, _, execution, _, _, _| {
        let target = *execution
            .active_math_fields
            .last()
            .ok_or(ExecError::MissingToken {
                context: "active math field",
            })?;
        let mut level =
            crate::box_runtime::commit_current_list(&mut execution.modes, context, effects, fuel)?;
        let (nodes, incomplete) = {
            let mut list = level.list_mutation();
            (list.take_nodes(), list.take_incomplete_fraction())
        };
        let list = finish_math_list(nodes, incomplete, context)?;
        let field = collapse_singleton_math_group(context, list);
        fill_math_field_target(&mut execution.modes, context, target, field);
        execution.active_math_fields.pop();
        execution.main_loop_active = false;
        Ok::<MainControlStep, ExecError>(MainControlStep::Continue)
    })
}

pub(super) fn script<G>(
    processor: &mut Processor<'_, '_, G>,
    kind: MathScriptKind,
) -> Result<MainControlStep, ExecError> {
    report_scan(processor)?;
    let target =
        processor.with_execution_owners(|context, _, effects, _, _, execution, _, _, _| {
            execution.main_loop_active = false;
            reserve_script_target(
                execution.modes.current_list_mutation(),
                context,
                effects,
                kind,
            )
        })?;
    recovery::finish_error_dialogue(processor)?;
    let field = processor.scan_math_field_episode().map_err(command_error)?;
    report_scan(processor)?;
    accept_field(processor, field, ActiveMathFieldTarget::Script(target))?;
    Ok::<MainControlStep, ExecError>(MainControlStep::Continue)
}

pub(super) fn limits<G>(
    processor: &mut Processor<'_, '_, G>,
    kind: LimitType,
) -> Result<MainControlStep, ExecError> {
    report_scan(processor)?;
    let applied = processor.with_execution_owners(|context, _, _, _, _, execution, _, _, _| {
        execution.main_loop_active = false;
        apply_limits(execution.modes.current_list_mutation(), context, kind)
    });
    if !applied {
        processor.with_execution_owners(|context, command, effects, _, _, _, _, _, _| {
            let display = command.output_open_context(context);
            let mut report = context.print_err("Limit controls must follow a math operator");
            report.help(&["I'm ignoring this misplaced \\limits or \\nolimits command."]);
            report.context(display);
            report.error().defer_recovery(effects)?;
            Ok::<(), ExecError>(())
        })?;
        recovery::finish_error_dialogue(processor)?;
    }
    Ok(MainControlStep::Continue)
}

pub(super) fn fraction<G>(
    processor: &mut Processor<'_, '_, G>,
    kind: MathFractionKind,
    delimiters: bool,
) -> Result<MainControlStep, ExecError> {
    let fraction = processor
        .scan_math_fraction(kind, delimiters)
        .map_err(command_error)?;
    report_scan(processor)?;
    let applied = processor.with_execution_owners(|context, _, _, _, _, execution, _, _, _| {
        execution.main_loop_active = false;
        start_fraction(execution.modes.current_list_mutation(), context, fraction)
    });
    if !applied {
        processor.with_execution_owners(|context, command, effects, _, _, _, _, _, _| {
            let display = command.output_open_context(context);
            let mut report = context.print_err("Ambiguous; you need another { and }");
            report.help(&[
                "I'm ignoring this fraction specification, since I don't",
                "know whether a construction like `x \\over y \\over z'",
                "means `{x \\over y} \\over z' or `x \\over {y \\over z}'.",
            ]);
            report.context(display);
            report.error().defer_recovery(effects)?;
            Ok::<(), ExecError>(())
        })?;
        recovery::finish_error_dialogue(processor)?;
    }
    Ok(MainControlStep::Continue)
}
