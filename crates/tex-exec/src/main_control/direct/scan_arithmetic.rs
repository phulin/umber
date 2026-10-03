//! Scan the actual typed arithmetic operand in the live processor loan.
use super::*;

pub(super) fn scan_arithmetic_operand<G, Facts: tex_command::CommandHostFacts<G>>(
    processor: &mut CommandProcessor<'_, '_, G, Facts>,
    target: ArithmeticTarget,
    primitive: UnexpandablePrimitive,
    scan_origin: Option<OriginId>,
) -> Result<ArithmeticOperand, ExecError> {
    let _ = take_native_scalar!(processor.scan_keyword_retained("by"), scan_origin);

    let operand = match target {
        ArithmeticTarget::IntegerRegister(_) | ArithmeticTarget::IntegerParameter(_) => {
            ArithmeticOperand::Integer(
                take_native_scalar!(processor.scan_integer_retained(), scan_origin).value,
            )
        }
        ArithmeticTarget::DimensionRegister(_) | ArithmeticTarget::DimensionParameter(_) => {
            match primitive {
                UnexpandablePrimitive::Advance => ArithmeticOperand::Dimension(
                    take_native_scalar!(processor.scan_dimension_retained(), scan_origin),
                ),
                UnexpandablePrimitive::Multiply | UnexpandablePrimitive::Divide => {
                    ArithmeticOperand::Integer(
                        take_native_scalar!(processor.scan_integer_retained(), scan_origin).value,
                    )
                }
                _ => unreachable!("arithmetic primitive is filtered above"),
            }
        }
        ArithmeticTarget::GlueRegister { mu, .. } | ArithmeticTarget::GlueParameter { mu, .. } => {
            match primitive {
                UnexpandablePrimitive::Advance => ArithmeticOperand::Glue(take_native_scalar!(
                    processor.scan_glue_retained(mu),
                    scan_origin
                )),
                UnexpandablePrimitive::Multiply | UnexpandablePrimitive::Divide => {
                    ArithmeticOperand::Integer(
                        take_native_scalar!(processor.scan_integer_retained(), scan_origin).value,
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
    scan_origin: Option<OriginId>,
    observation_receipts: &mut Option<Vec<MutationRecord>>,
    mut pending_diagnostics: Option<&mut Vec<PendingDiagnostic<G>>>,
) -> Result<ReplayStep, ExecError> {
    let mut completed = None;
    processor.consume_expanded_words(&mut |processor, _, meaning, control_sequence, _| {
        completed = Some((|| {
            let target = match meaning.resolve() {
                ResolvedMeaning::Static(Meaning::UnexpandablePrimitive(
                    register @ (UnexpandablePrimitive::Count | UnexpandablePrimitive::Dimen
                    | UnexpandablePrimitive::Skip | UnexpandablePrimitive::Muskip))) => {
                    let index = take_native_scalar!(
                        processor.scan_profile_register_index_retained(), scan_origin);
                    match register {
                        UnexpandablePrimitive::Count => ArithmeticTarget::IntegerRegister(index),
                        UnexpandablePrimitive::Dimen => ArithmeticTarget::DimensionRegister(index),
                        UnexpandablePrimitive::Skip => ArithmeticTarget::GlueRegister { index, mu: false },
                        _ => ArithmeticTarget::GlueRegister { index, mu: true },
                    }
                }
                ResolvedMeaning::Static(Meaning::CountRegister(index)) => ArithmeticTarget::IntegerRegister(index),
                ResolvedMeaning::Static(Meaning::DimenRegister(index)) => ArithmeticTarget::DimensionRegister(index),
                ResolvedMeaning::Static(Meaning::SkipRegister(index)) => ArithmeticTarget::GlueRegister { index, mu: false },
                ResolvedMeaning::Static(Meaning::MuskipRegister(index)) => ArithmeticTarget::GlueRegister { index, mu: true },
                ResolvedMeaning::Static(Meaning::IntParam(index)) => ArithmeticTarget::IntegerParameter(index),
                ResolvedMeaning::Static(Meaning::DimenParam(index)) => ArithmeticTarget::DimensionParameter(index),
                ResolvedMeaning::Static(Meaning::GlueParam(index)) => ArithmeticTarget::GlueParameter { index, mu: false },
                ResolvedMeaning::Static(Meaning::MuGlueParam(index)) => ArithmeticTarget::GlueParameter { index, mu: true },
                _ => {
                    let target = tex_command::PrintCommand::from_native(meaning, control_sequence);
                    return with_native_assignment!(processor, observation_receipts, pending_diagnostics.take();
                        _receipts, |context, state, effects, _fuel, _observer, _execution| {
                            semantic::execute_invalid_arithmetic_target(context, state, effects, primitive, target)
                        });
                }
            };
            let operand = scan_arithmetic_operand(processor, target, primitive, scan_origin)?;
            with_native_assignment!(processor, observation_receipts, pending_diagnostics.take();
                receipts, |context, state, effects, _fuel, _observer, _execution| {
                    semantic::execute_arithmetic(context, effects, state, receipts,
                        primitive, target, operand, global)
                })
        })());
        Ok(std::ops::ControlFlow::Break(()))
    }).map_err(command_error).map_err(|error| scan_error(error, scan_origin))?;
    completed.ok_or_else(|| scan_error(ExecError::UnsupportedAssignmentTarget, scan_origin))?
}
