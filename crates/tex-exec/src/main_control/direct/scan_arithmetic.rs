//! Arithmetic operand scanning in the actual command processor loan.
use super::*;

pub(super) fn scan_arithmetic_operands<G>(
    processor: &mut CommandProcessor<'_, '_, G>,
    scalar: &mut tex_command::ScalarScanFrame,
    primitive: UnexpandablePrimitive,
    scan_origin: Option<OriginId>,
) -> Result<Result<(ArithmeticTarget, ArithmeticOperand), tex_command::PrintCommand<G>>, ExecError>
{
    let mut destination = None;
    (processor
        .get_x_token_into(&mut destination)
        .map_err(command_error))
    .map_err(|error| scan_error(error, scan_origin))?;
    let target_command = (destination.ok_or(ExecError::UnsupportedAssignmentTarget))
        .map_err(|error| scan_error(error, scan_origin))?;
    let target = match target_command.meaning() {
        tex_state::meaning::ResolvedMeaning::Macro { .. } => {
            return Ok(Err(tex_command::PrintCommand::from_current(
                &target_command,
            )));
        }
        tex_state::meaning::ResolvedMeaning::Static(meaning) => match meaning {
            Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Count) => {
                let status = processor.scan_profile_register_index_into(scalar);
                ArithmeticTarget::IntegerRegister(take_control_scalar!(
                    scalar,
                    status,
                    take_register,
                    scan_origin
                ))
            }
            Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Dimen) => {
                let status = processor.scan_profile_register_index_into(scalar);
                ArithmeticTarget::DimensionRegister(take_control_scalar!(
                    scalar,
                    status,
                    take_register,
                    scan_origin
                ))
            }
            Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Skip) => {
                let status = processor.scan_profile_register_index_into(scalar);
                ArithmeticTarget::GlueRegister {
                    index: take_control_scalar!(scalar, status, take_register, scan_origin),
                    mu: false,
                }
            }
            Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Muskip) => {
                let status = processor.scan_profile_register_index_into(scalar);
                ArithmeticTarget::GlueRegister {
                    index: take_control_scalar!(scalar, status, take_register, scan_origin),
                    mu: true,
                }
            }
            Meaning::CountRegister(index) => ArithmeticTarget::IntegerRegister(index),
            Meaning::DimenRegister(index) => ArithmeticTarget::DimensionRegister(index),
            Meaning::SkipRegister(index) => ArithmeticTarget::GlueRegister { index, mu: false },
            Meaning::MuskipRegister(index) => ArithmeticTarget::GlueRegister { index, mu: true },
            Meaning::IntParam(index) => ArithmeticTarget::IntegerParameter(index),
            Meaning::DimenParam(index) => ArithmeticTarget::DimensionParameter(index),
            Meaning::GlueParam(index) => ArithmeticTarget::GlueParameter { index, mu: false },
            Meaning::MuGlueParam(index) => ArithmeticTarget::GlueParameter { index, mu: true },
            _ => {
                return Ok(Err(tex_command::PrintCommand::from_current(
                    &target_command,
                )));
            }
        },
    };

    let status = processor.scan_keyword_into("by", scalar);
    let _ = take_control_scalar!(scalar, status, take_boolean, scan_origin);

    let operand = match target {
        ArithmeticTarget::IntegerRegister(_) | ArithmeticTarget::IntegerParameter(_) => {
            let status = processor.scan_integer_into(scalar);
            ArithmeticOperand::Integer(
                take_control_scalar!(scalar, status, take_integer, scan_origin).value,
            )
        }
        ArithmeticTarget::DimensionRegister(_) | ArithmeticTarget::DimensionParameter(_) => {
            match primitive {
                UnexpandablePrimitive::Advance => {
                    let status = processor.scan_dimension_into(scalar);
                    ArithmeticOperand::Dimension(
                        take_control_scalar!(scalar, status, take_dimension, scan_origin).value,
                    )
                }
                UnexpandablePrimitive::Multiply | UnexpandablePrimitive::Divide => {
                    let status = processor.scan_integer_into(scalar);
                    ArithmeticOperand::Integer(
                        take_control_scalar!(scalar, status, take_integer, scan_origin).value,
                    )
                }
                _ => unreachable!("arithmetic primitive is filtered above"),
            }
        }
        ArithmeticTarget::GlueRegister { mu, .. } | ArithmeticTarget::GlueParameter { mu, .. } => {
            match primitive {
                UnexpandablePrimitive::Advance => {
                    let status = processor.scan_glue_into(mu, scalar);
                    ArithmeticOperand::Glue(
                        take_control_scalar!(scalar, status, take_glue, scan_origin).value,
                    )
                }
                UnexpandablePrimitive::Multiply | UnexpandablePrimitive::Divide => {
                    let status = processor.scan_integer_into(scalar);
                    ArithmeticOperand::Integer(
                        take_control_scalar!(scalar, status, take_integer, scan_origin).value,
                    )
                }
                _ => unreachable!("arithmetic primitive is filtered above"),
            }
        }
    };
    Ok(Ok((target, operand)))
}
