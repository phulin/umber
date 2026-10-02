//! Native scan registers in the sole main-control engine.
use super::*;
impl<G> DirectDispatch<'_, '_, G> {
    pub(super) fn scan_glue_register_assignment(
        &mut self,
        index: Option<u16>,
        global: bool,
        mu: bool,
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

        let index = match index {
            Some(index) => index,
            None => {
                let status = processor.scan_profile_register_index_into(scalar);
                take_control_scalar!(scalar, status, take_register, scan_origin)
            }
        };
        {
            let status = processor.scan_optional_equals_into(scalar);
            let _ = take_control_scalar!(scalar, status, take_boolean, scan_origin);
        }
        let status = processor.scan_glue_into(mu, scalar);
        let value = take_control_scalar!(scalar, status, take_glue, scan_origin).value;
        let source_identity = processor.scanned_glue_identity();
        let source_register = processor.scanned_glue_register();
        if mu {
            {
                let (direct_arg0, direct_arg1, direct_arg2, direct_arg3, direct_arg4) =
                    (index, value, source_identity, source_register, global);
                finish_scan!(self.execute_muskip(
                    direct_arg0,
                    direct_arg1,
                    direct_arg2,
                    direct_arg3,
                    direct_arg4
                ))
            }
        } else {
            {
                let (direct_arg0, direct_arg1, direct_arg2, direct_arg3, direct_arg4) =
                    (index, value, source_identity, source_register, global);
                finish_scan!(self.execute_skip(
                    direct_arg0,
                    direct_arg1,
                    direct_arg2,
                    direct_arg3,
                    direct_arg4
                ))
            }
        }
    }
}
