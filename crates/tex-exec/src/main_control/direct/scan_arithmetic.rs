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
                    take_native_scalar!(processor.scan_dimension_retained(), scan_origin).value,
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
                UnexpandablePrimitive::Advance => ArithmeticOperand::Glue(
                    take_native_scalar!(processor.scan_glue_retained(mu), scan_origin).value,
                ),
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
