//! Native scan group recovery in the sole main-control engine.
use super::*;
impl<G> DirectDispatch<'_, '_, G> {
    pub(super) fn scan_off_save(
        &mut self,
        command: tex_command::CurrentCommand<G>,
    ) -> Result<ReplayStep, ExecError> {
        let boxes = &self.control.execution.boxes;

        let mut admitted_context = self
            .stores
            .command_context()
            .expect("direct scanner admission");
        let innermost_group = admitted_context.innermost_group_kind();

        let mut host_facts = ExecutorHostFacts {
            modes: &self.control.execution.modes,
            pdf_ignore_depth: self.control.execution.pdf_ignore_depth,
            telemetry: &mut self.control.execution.episode_telemetry,
        };
        let mut owned_processor = if let Some(provider) = self.provider.as_deref_mut() {
            command_processor_with_resource_provider(
                &mut self.control.command,
                self.control.fuel.fuel_mut(),
                &mut self.control.capabilities,
                &mut host_facts as &mut dyn tex_command::CommandHostFacts<G>,
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
                &mut host_facts as &mut dyn tex_command::CommandHostFacts<G>,
                &mut self.control.operation_observations,
                self.effects,
                &mut admitted_context,
            )
        };
        owned_processor.set_output_routine_active(boxes.output_routine_active);
        if let Some(cursor) = self.cursor {
            owned_processor.resume_delivery_cursor(cursor);
        }
        let processor = &mut owned_processor;
        // Finish this actual processor/context loan before semantic application.
        macro_rules! finish_scan {
            ($call:expr) => {
                direct_finish_scan!(self, owned_processor, admitted_context, host_facts, $call)
            };
        }

        let scan_origin = self.current_origin;

        let Some(kind) = innermost_group else {
            let token = command.spelling().semantic_token();
            processor.report_off_save_bottom_drop(&command);
            return {
                let (direct_arg0,) = (token,);
                finish_scan!(self.execute_off_save_bottom_drop(direct_arg0))
            };
        };
        match kind {
            GroupKind::SemiSimple => {
                let endgroup = (processor
                    .frozen_primitive_token("endgroup")
                    .map_err(command_error))
                .map_err(|error| scan_error(error, scan_origin))?;
                (processor
                    .recover_off_save(command, &[endgroup])
                    .map_err(command_error))
                .map_err(|error| scan_error(error, scan_origin))?;
                {
                    let (direct_arg0,) = (OffSaveCloser::EndGroup,);
                    finish_scan!(self.execute_off_save(direct_arg0))
                }
            }
            GroupKind::MathShift => {
                let dollar = Token::Char {
                    ch: '$',
                    cat: Catcode::MathShift,
                };
                (processor
                    .recover_off_save(command, &[dollar])
                    .map_err(command_error))
                .map_err(|error| scan_error(error, scan_origin))?;
                {
                    let (direct_arg0,) = (OffSaveCloser::MathShift,);
                    finish_scan!(self.execute_off_save(direct_arg0))
                }
            }
            GroupKind::MathLeft => {
                let right = (processor
                    .frozen_primitive_token("right")
                    .map_err(command_error))
                .map_err(|error| scan_error(error, scan_origin))?;
                let dot = Token::Char {
                    ch: '.',
                    cat: Catcode::Other,
                };
                (processor
                    .recover_off_save(command, &[right, dot])
                    .map_err(command_error))
                .map_err(|error| scan_error(error, scan_origin))?;
                {
                    let (direct_arg0,) = (OffSaveCloser::NullRight,);
                    finish_scan!(self.execute_off_save(direct_arg0))
                }
            }
            _ => {
                let right_brace = Token::Char {
                    ch: '}',
                    cat: Catcode::EndGroup,
                };
                (processor
                    .recover_off_save(command, &[right_brace])
                    .map_err(command_error))
                .map_err(|error| scan_error(error, scan_origin))?;
                {
                    let (direct_arg0,) = (OffSaveCloser::RightBrace,);
                    finish_scan!(self.execute_off_save(direct_arg0))
                }
            }
        }
    }
}
