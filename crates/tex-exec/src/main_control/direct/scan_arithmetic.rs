//! Native scan arithmetic in the sole main-control engine.
use super::*;
impl<G> DirectDispatch<'_, '_, G> {
    pub(super) fn scan_arithmetic_assignment(
        &mut self,
        primitive: UnexpandablePrimitive,
        global: bool,
    ) -> Result<ReplayStep, ExecError> {
        let boxes = &self.control.boxes;

        let mut admitted_context = self
            .stores
            .command_context()
            .expect("direct scanner admission");

        let mut host_facts = ExecutorHostFacts {
            modes: &self.control.modes,
            pdf_ignore_depth: self.control.pdf_ignore_depth,
            telemetry: &mut self.control.episode_telemetry,
        };
        let mut owned_processor = if let Some(provider) = self.provider.as_deref_mut() {
            command_processor_with_resource_provider(
                &mut self.control.command,
                self.control.fuel.fuel_mut(),
                &mut self.control.capabilities,
                &mut host_facts,
                provider,
                &mut self.control.declined_resource_attempt,
                &mut self.control.operation_observations,
                self.effects,
                &mut admitted_context,
            )
        } else {
            command_processor(
                &mut self.control.command,
                self.control.fuel.fuel_mut(),
                &mut self.control.capabilities,
                &mut host_facts,
                &mut self.control.operation_observations,
                self.effects,
                &mut admitted_context,
            )
        };
        owned_processor.set_output_routine_active(boxes.output_routine_active);
        if let Some(cursor) = self.cursor {
            owned_processor.resume_delivery_cursor(cursor);
        }
        let processor: &mut CommandProcessor<'_, '_, G> = &mut owned_processor;
        // Finish this actual processor/context loan before semantic application.
        macro_rules! finish_scan {
            ($call:expr) => {
                direct_finish_scan!(self, owned_processor, admitted_context, host_facts, $call)
            };
        }

        let mut scalar_storage = tex_command::ScalarScanFrame::default();
        let scalar = &mut scalar_storage;

        let scan_origin = self.current_origin;

        let mut destination = None;
        (processor
            .get_x_token_into(&mut destination)
            .map_err(command_error))
        .map_err(|error| scan_error(error, scan_origin))?;
        let target_command = (destination.ok_or(ExecError::UnsupportedAssignmentTarget))
            .map_err(|error| scan_error(error, scan_origin))?;
        let target = match target_command.meaning() {
            tex_state::meaning::ResolvedMeaning::Macro { .. } => {
                return {
                    let (direct_arg0, direct_arg1) = (
                        primitive,
                        tex_command::PrintCommand::from_current(&target_command),
                    );
                    finish_scan!(self.execute_invalid_arithmetic_target(direct_arg0, direct_arg1))
                };
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
                Meaning::MuskipRegister(index) => {
                    ArithmeticTarget::GlueRegister { index, mu: true }
                }
                Meaning::IntParam(index) => ArithmeticTarget::IntegerParameter(index),
                Meaning::DimenParam(index) => ArithmeticTarget::DimensionParameter(index),
                Meaning::GlueParam(index) => ArithmeticTarget::GlueParameter { index, mu: false },
                Meaning::MuGlueParam(index) => ArithmeticTarget::GlueParameter { index, mu: true },
                _ => {
                    return {
                        let (direct_arg0, direct_arg1) = (
                            primitive,
                            tex_command::PrintCommand::from_current(&target_command),
                        );
                        finish_scan!(
                            self.execute_invalid_arithmetic_target(direct_arg0, direct_arg1)
                        )
                    };
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
            ArithmeticTarget::GlueRegister { mu, .. }
            | ArithmeticTarget::GlueParameter { mu, .. } => match primitive {
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
            },
        };

        {
            let (direct_arg0, direct_arg1, direct_arg2, direct_arg3) =
                (primitive, target, operand, global);
            finish_scan!(self.execute_arithmetic(
                direct_arg0,
                direct_arg1,
                direct_arg2,
                direct_arg3
            ))
        }
    }
}
