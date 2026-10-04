//! Concrete operand scans and mutations in the actual persistent processor.
use super::*;

pub(super) fn tokens<G>(
    processor: &mut Processor<'_, '_, G>,
    owner: Symbol,
    index: u16,
    parameter: bool,
    global: bool,
    receipts: &mut Option<Vec<MutationRecord>>,
) -> Result<MainControlStep, ExecError> {
    let (tokens, source) = processor
        .scan_token_list_value(owner, parameter && index == TokParam::OUTPUT.raw())
        .map_err(command_error)?;
    report_scan(processor)?;
    let step = processor.with_execution_owners(
        |context, command, effects, _, _, execution, _, _, _| {
            execution.main_loop_active = false;
            let tokens = match source {
                Some(source) => Some(source),
                None => publication::publish_tokens(command, context, tokens)?,
            };
            semantic::execute_toks(
                context, effects, command, receipts, index, tokens, global, parameter,
            )
        },
    )?;
    processor.complete_assignment().map_err(command_error)?;
    Ok(step)
}

pub(super) fn code_table<G>(
    processor: &mut Processor<'_, '_, G>,
    primitive: UnexpandablePrimitive,
    global: bool,
    receipts: &mut Option<Vec<MutationRecord>>,
) -> Result<MainControlStep, ExecError> {
    let (character, value) = scan_code_table_operands(processor)?;
    report_scan(processor)?;
    let step = processor.with_execution_owners(
        |context, command, effects, _, _, execution, _, _, _| {
            execution.main_loop_active = false;
            if primitive == UnexpandablePrimitive::CatCode {
                definition_assignments::apply_catcode(
                    character, value, global, context, command, effects, receipts,
                )
            } else {
                semantic::execute_code_table(
                    context, command, effects, receipts, primitive, character, value, global,
                )
            }
        },
    )?;
    recovery::finish_error_dialogue(processor)?;
    processor.complete_assignment().map_err(command_error)?;
    Ok(step)
}

pub(super) fn character_definition<G>(
    processor: &mut Processor<'_, '_, G>,
    primitive: UnexpandablePrimitive,
    global: bool,
    receipts: &mut Option<Vec<MutationRecord>>,
) -> Result<MainControlStep, ExecError> {
    let class = if primitive == UnexpandablePrimitive::CharDef {
        RestrictedIntegerClass::CharacterCode
    } else {
        RestrictedIntegerClass::FifteenBit
    };
    let definition = processor
        .scan_character_definition(
            class,
            global,
            assignment_tracing::trace_provisional_meaning_write,
        )
        .map_err(command_error)?;
    report_scan(processor)?;
    let step =
        processor.with_execution_owners(|context, _, effects, _, _, execution, _, _, _| {
            execution.main_loop_active = false;
            semantic::execute_character_definition(
                context,
                effects,
                receipts,
                primitive,
                definition.target,
                definition.value,
                global,
            )
        })?;
    processor.complete_assignment().map_err(command_error)?;
    Ok(step)
}

pub(super) fn register_definition<G>(
    processor: &mut Processor<'_, '_, G>,
    primitive: UnexpandablePrimitive,
    global: bool,
    receipts: &mut Option<Vec<MutationRecord>>,
) -> Result<MainControlStep, ExecError> {
    let definition = processor
        .scan_register_definition(global, assignment_tracing::trace_provisional_meaning_write)
        .map_err(command_error)?;
    report_scan(processor)?;
    let step = processor.with_execution_owners(
        |context, command, effects, _, _, execution, _, _, _| {
            execution.main_loop_active = false;
            semantic::execute_register_definition(
                context,
                command,
                effects,
                receipts,
                primitive,
                definition.target,
                definition.index,
                global,
            )
        },
    )?;
    processor.complete_assignment().map_err(command_error)?;
    Ok(step)
}

pub(super) fn math_font<G>(
    processor: &mut Processor<'_, '_, G>,
    size: MathFontSize,
    global: bool,
) -> Result<MainControlStep, ExecError> {
    let family = processor
        .scan_restricted_integer(RestrictedIntegerClass::FourBit)
        .map_err(command_error)?
        .value as u8;
    processor.scan_optional_equals().map_err(command_error)?;
    let font = processor.scan_font_selector().map_err(command_error)?;
    report_scan(processor)?;
    processor.with_execution_owners(|context, _, effects, _, _, execution, _, _, _| {
        execution.main_loop_active = false;
        AssignmentCommitter::new(context, effects)
            .try_unscoped(|context| assign_math_family_font(context, size, family, font, global))
    })?;
    processor.complete_assignment().map_err(command_error)?;
    Ok(MainControlStep::Continue)
}

pub(super) fn glue_parameter<G>(
    processor: &mut Processor<'_, '_, G>,
    index: u16,
    mu: bool,
    global: bool,
    receipts: &mut Option<Vec<MutationRecord>>,
) -> Result<MainControlStep, ExecError> {
    processor.scan_optional_equals().map_err(command_error)?;
    let value = processor.scan_glue(mu).map_err(command_error)?;
    report_scan(processor)?;
    let step = processor.with_execution_owners(
        |context, command, effects, _, _, execution, _, _, _| {
            execution.main_loop_active = false;
            semantic::execute_glue_param(context, effects, command, receipts, index, value, global)
        },
    )?;
    processor.complete_assignment().map_err(command_error)?;
    Ok(step)
}

pub(super) fn count<G>(
    processor: &mut Processor<'_, '_, G>,
    index: u16,
    global: bool,
    receipts: &mut Option<Vec<MutationRecord>>,
) -> Result<MainControlStep, ExecError> {
    processor.scan_optional_equals().map_err(command_error)?;
    let value = processor.scan_integer().map_err(command_error)?.value;
    report_scan(processor)?;
    let step =
        processor.with_execution_owners(|context, _, effects, _, _, execution, _, _, _| {
            execution.main_loop_active = false;
            semantic::execute_count(context, effects, receipts, index, value, global)
        })?;
    processor.complete_assignment().map_err(command_error)?;
    Ok(step)
}

pub(super) fn dimension<G>(
    processor: &mut Processor<'_, '_, G>,
    index: u16,
    global: bool,
    receipts: &mut Option<Vec<MutationRecord>>,
) -> Result<MainControlStep, ExecError> {
    processor.scan_optional_equals().map_err(command_error)?;
    let value = processor.scan_dimension().map_err(command_error)?;
    report_scan(processor)?;
    let step =
        processor.with_execution_owners(|context, _, effects, _, _, execution, _, _, _| {
            execution.main_loop_active = false;
            semantic::execute_dimen(context, effects, receipts, index, value, global)
        })?;
    processor.complete_assignment().map_err(command_error)?;
    Ok(step)
}

pub(super) fn integer_parameter<G>(
    processor: &mut Processor<'_, '_, G>,
    index: u16,
    global: bool,
    receipts: &mut Option<Vec<MutationRecord>>,
) -> Result<MainControlStep, ExecError> {
    processor.scan_optional_equals().map_err(command_error)?;
    let value = processor.scan_integer().map_err(command_error)?.value;
    report_scan(processor)?;
    let step = processor.with_execution_owners(
        |context, command, effects, _, _, execution, _, _, _| {
            execution.main_loop_active = false;
            semantic::execute_int_param(context, effects, command, receipts, index, value, global)
        },
    )?;
    processor.complete_assignment().map_err(command_error)?;
    Ok(step)
}

pub(super) fn dimension_parameter<G>(
    processor: &mut Processor<'_, '_, G>,
    index: u16,
    global: bool,
    receipts: &mut Option<Vec<MutationRecord>>,
) -> Result<MainControlStep, ExecError> {
    processor.scan_optional_equals().map_err(command_error)?;
    let value = processor.scan_dimension().map_err(command_error)?;
    report_scan(processor)?;
    let step = processor.with_execution_owners(
        |context, command, effects, _, _, execution, _, _, _| {
            execution.main_loop_active = false;
            semantic::execute_dimen_param(context, effects, command, receipts, index, value, global)
        },
    )?;
    processor.complete_assignment().map_err(command_error)?;
    Ok(step)
}

pub(super) fn glue_register<G>(
    processor: &mut Processor<'_, '_, G>,
    index: u16,
    mu: bool,
    global: bool,
    receipts: &mut Option<Vec<MutationRecord>>,
) -> Result<MainControlStep, ExecError> {
    processor.scan_optional_equals().map_err(command_error)?;
    let value = processor.scan_glue(mu).map_err(command_error)?;
    let source_identity = processor.scanned_glue_identity();
    let source_register = processor.scanned_glue_register();
    report_scan(processor)?;
    let step =
        processor.with_execution_owners(|context, _, effects, _, _, execution, _, _, _| {
            execution.main_loop_active = false;
            let reassigning = glue_identity::glue_pointer_reassigned(
                context,
                mu,
                index,
                value,
                source_identity,
                source_register,
                global,
                &execution.skip_pointer_sources,
                &execution.muskip_pointer_sources,
            );
            let redundant = context.int_param(IntParam::ETEX_EXTENDED_MODE) > 0 && reassigning;
            let step = semantic::execute_skip(
                context,
                effects,
                receipts,
                index,
                value,
                global,
                mu,
                redundant,
                reassigning,
            )?;
            if !redundant {
                let physical = if mu {
                    context.muskip(index)
                } else {
                    context.glue_register(index).ok().flatten()
                };
                if let Some(physical) = physical {
                    let pointers = if mu {
                        &mut execution.muskip_pointer_sources
                    } else {
                        &mut execution.skip_pointer_sources
                    };
                    if pointers.len() <= usize::from(index) {
                        pointers.resize(usize::from(index) + 1, None);
                    }
                    pointers[usize::from(index)] = Some((physical, source_identity));
                }
            }
            Ok::<MainControlStep, ExecError>(step)
        })?;
    processor.complete_assignment().map_err(command_error)?;
    Ok(step)
}

pub(super) fn definition<G>(
    processor: &mut Processor<'_, '_, G>,
    expanded: bool,
    flags: MeaningFlags,
    global: bool,
    receipts: &mut Option<Vec<MutationRecord>>,
) -> Result<MainControlStep, ExecError> {
    let definition = processor
        .scan_macro_definition(expanded)
        .map_err(command_error)?;
    report_scan(processor)?;
    let step = processor.with_execution_owners(
        |context, _, effects, fuel, observer, execution, _, _, _| {
            execution.main_loop_active = false;
            crate::box_runtime::flush_pending_hchars_with_fuel(
                &mut execution.modes,
                context,
                effects,
                fuel,
            )?;
            definition_assignments::apply_macro_definition(
                definition.target,
                definition.definition,
                flags,
                global,
                context,
                effects,
                observer,
                receipts,
            )
        },
    )?;
    processor.complete_assignment().map_err(command_error)?;
    Ok(step)
}

pub(super) fn bind<G>(
    processor: &mut Processor<'_, '_, G>,
    target: Symbol,
    meaning: tex_state::MeaningWord<G>,
    global: bool,
    receipts: &mut Option<Vec<MutationRecord>>,
) -> Result<MainControlStep, ExecError> {
    report_scan(processor)?;
    let step =
        processor.with_execution_owners(|context, _, effects, fuel, _, execution, _, _, _| {
            execution.main_loop_active = false;
            crate::box_runtime::flush_pending_hchars_with_fuel(
                &mut execution.modes,
                context,
                effects,
                fuel,
            )?;
            definition_assignments::apply_let(target, meaning, global, context, effects, receipts)
        })?;
    processor.complete_assignment().map_err(command_error)?;
    Ok(step)
}

pub(super) fn shape<G>(
    processor: &mut Processor<'_, '_, G>,
    global: bool,
) -> Result<MainControlStep, ExecError> {
    processor.scan_optional_equals().map_err(command_error)?;
    let count = processor
        .scan_integer()
        .map_err(command_error)?
        .value
        .max(0) as usize;
    let mut lines = Vec::new();
    lines
        .try_reserve_exact(count)
        .map_err(|_| ExecError::ArithmeticOverflow)?;
    for _ in 0..count {
        let indent = processor.scan_dimension().map_err(command_error)?;
        let width = processor.scan_dimension().map_err(command_error)?;
        lines.push(ParagraphShapeLine { indent, width });
    }
    report_scan(processor)?;
    let step =
        processor.with_execution_owners(|context, _, effects, _, _, execution, _, _, _| {
            execution.main_loop_active = false;
            semantic::execute_paragraph_shape(context, effects, &lines, global)
        })?;
    processor.complete_assignment().map_err(command_error)?;
    Ok(step)
}

pub(super) fn penalties<G>(
    processor: &mut Processor<'_, '_, G>,
    kind: PenaltyArrayKind,
    global: bool,
    receipts: &mut Option<Vec<MutationRecord>>,
) -> Result<MainControlStep, ExecError> {
    processor.scan_optional_equals().map_err(command_error)?;
    let count = processor
        .scan_integer()
        .map_err(command_error)?
        .value
        .max(0) as usize;
    let mut values = Vec::new();
    values
        .try_reserve_exact(count)
        .map_err(|_| ExecError::ArithmeticOverflow)?;
    for _ in 0..count {
        values.push(processor.scan_integer().map_err(command_error)?.value);
    }
    report_scan(processor)?;
    let step =
        processor.with_execution_owners(|context, _, effects, _, _, execution, _, _, _| {
            execution.main_loop_active = false;
            semantic::execute_penalty_array(context, effects, receipts, kind, &values, global)
        })?;
    processor.complete_assignment().map_err(command_error)?;
    Ok(step)
}
