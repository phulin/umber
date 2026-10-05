//! Scan the actual typed arithmetic operand in the live processor loan.
use super::*;

pub(super) fn scan_arithmetic_operand<G, Facts: tex_command::CommandHostFacts<G>>(
    processor: &mut CommandProcessor<'_, '_, G, Facts>,
    target: ArithmeticTarget,
    primitive: UnexpandablePrimitive,
) -> Result<ArithmeticOperand, ExecError> {
    let _ = processor.scan_keyword("by").map_err(command_error)?;

    let operand = match target {
        ArithmeticTarget::IntegerRegister(_) | ArithmeticTarget::IntegerParameter(_) => {
            ArithmeticOperand::Integer(processor.scan_integer().map_err(command_error)?.value)
        }
        ArithmeticTarget::DimensionRegister(_) | ArithmeticTarget::DimensionParameter(_) => {
            match primitive {
                UnexpandablePrimitive::Advance => {
                    ArithmeticOperand::Dimension(processor.scan_dimension().map_err(command_error)?)
                }
                UnexpandablePrimitive::Multiply | UnexpandablePrimitive::Divide => {
                    ArithmeticOperand::Integer(
                        processor.scan_integer().map_err(command_error)?.value,
                    )
                }
                _ => unreachable!("arithmetic primitive is filtered above"),
            }
        }
        ArithmeticTarget::GlueRegister { mu, .. } | ArithmeticTarget::GlueParameter { mu, .. } => {
            match primitive {
                UnexpandablePrimitive::Advance => {
                    ArithmeticOperand::Glue(processor.scan_glue(mu).map_err(command_error)?)
                }
                UnexpandablePrimitive::Multiply | UnexpandablePrimitive::Divide => {
                    ArithmeticOperand::Integer(
                        processor.scan_integer().map_err(command_error)?.value,
                    )
                }
                _ => unreachable!("arithmetic primitive is filtered above"),
            }
        }
    };
    Ok(operand)
}

/// Consume the actual target word, operand and mutation in one processor loan.
/// Only invalid targets materialize a diagnostic command value.
pub(super) fn execute_arithmetic<'state, 'host, G>(
    processor: &mut CommandProcessor<'state, 'host, G, &'state mut ExecutionState<G>>,
    primitive: UnexpandablePrimitive,
    global: bool,
    receipts: &mut Option<Vec<MutationRecord>>,
) -> Result<MainControlStep, ExecError> {
    let mut completed: Option<Result<MainControlStep, ExecError>> = None;
    processor
        .consume_expanded_words(&mut tex_command::terminal_consumer::<
            Processor<'_, '_, G>,
            G,
            _,
        >(
            |processor, _, meaning, _code, control_sequence, _| {
                completed = Some((|| {
                    let target = match meaning.resolve() {
                        ResolvedMeaning::Static(Meaning::UnexpandablePrimitive(
                            register @ (UnexpandablePrimitive::Count
                            | UnexpandablePrimitive::Dimen
                            | UnexpandablePrimitive::Skip
                            | UnexpandablePrimitive::Muskip),
                        )) => {
                            let index = processor
                                .scan_profile_register_index()
                                .map_err(command_error)?;
                            match register {
                                UnexpandablePrimitive::Count => {
                                    ArithmeticTarget::IntegerRegister(index)
                                }
                                UnexpandablePrimitive::Dimen => {
                                    ArithmeticTarget::DimensionRegister(index)
                                }
                                UnexpandablePrimitive::Skip => {
                                    ArithmeticTarget::GlueRegister { index, mu: false }
                                }
                                _ => ArithmeticTarget::GlueRegister { index, mu: true },
                            }
                        }
                        ResolvedMeaning::Static(Meaning::CountRegister(index)) => {
                            ArithmeticTarget::IntegerRegister(index)
                        }
                        ResolvedMeaning::Static(Meaning::DimenRegister(index)) => {
                            ArithmeticTarget::DimensionRegister(index)
                        }
                        ResolvedMeaning::Static(Meaning::SkipRegister(index)) => {
                            ArithmeticTarget::GlueRegister { index, mu: false }
                        }
                        ResolvedMeaning::Static(Meaning::MuskipRegister(index)) => {
                            ArithmeticTarget::GlueRegister { index, mu: true }
                        }
                        ResolvedMeaning::Static(Meaning::IntParam(index)) => {
                            ArithmeticTarget::IntegerParameter(index)
                        }
                        ResolvedMeaning::Static(Meaning::DimenParam(index)) => {
                            ArithmeticTarget::DimensionParameter(index)
                        }
                        ResolvedMeaning::Static(Meaning::GlueParam(index)) => {
                            ArithmeticTarget::GlueParameter { index, mu: false }
                        }
                        ResolvedMeaning::Static(Meaning::MuGlueParam(index)) => {
                            ArithmeticTarget::GlueParameter { index, mu: true }
                        }
                        _ => {
                            let target =
                                tex_command::PrintCommand::from_native(meaning, control_sequence);
                            report_scan(processor)?;
                            let result = processor.with_execution_owners(
                                |context, state, effects, _, _, execution, _, _, _| {
                                    execution.main_loop_active = false;
                                    semantic::execute_invalid_arithmetic_target(
                                        context, state, effects, primitive, target,
                                    )
                                },
                            )?;
                            recovery::finish_error_dialogue(processor)?;
                            return Ok(result);
                        }
                    };
                    let operand = scan_arithmetic_operand(processor, target, primitive)?;
                    report_scan(processor)?;
                    let result = processor.with_execution_owners(
                        |context, state, effects, _, _, execution, _, _, _| {
                            execution.main_loop_active = false;
                            semantic::execute_arithmetic(
                                context, effects, state, receipts, primitive, target, operand,
                                global,
                            )
                        },
                    )?;
                    recovery::finish_error_dialogue(processor)?;
                    Ok(result)
                })());
                Ok(std::ops::ControlFlow::Break(()))
            },
        ))
        .map_err(command_error)?;
    completed.ok_or(ExecError::UnsupportedAssignmentTarget)??;
    assignments::complete(processor, receipts)?;
    Ok(MainControlStep::Continue)
}
