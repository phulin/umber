//! Native scan streams in the sole main-control engine.
use super::*;
impl<G> DirectDispatch<'_, '_, G> {
    pub(super) fn scan_open_out_operation(&mut self) -> Result<ReplayStep, ExecError> {
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

        let status =
            processor.scan_restricted_integer_into(RestrictedIntegerClass::FourBit, scalar);
        let stream = take_control_scalar!(scalar, status, take_restricted, scan_origin).value as u8;
        let status = processor.scan_optional_equals_into(scalar);
        let _ = take_control_scalar!(scalar, status, take_boolean, scan_origin);
        let status = processor.scan_file_name_into(scalar);
        let file_name = take_control_scalar!(scalar, status, take_file_name, scan_origin);
        {
            let (direct_arg0, direct_arg1) = (stream, file_name.packed());
            finish_scan!(self.execute_deferred_open_out(direct_arg0, direct_arg1))
        }
    }

    pub(super) fn scan_marks_operation(&mut self) -> Result<ReplayStep, ExecError> {
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

        let status = processor.scan_extended_register_index_into(scalar);
        let class = take_control_scalar!(scalar, status, take_register, scan_origin);
        let scan = processor.scan_balanced_text_retained(true);
        let text = (retain_operation_child(processor, scan))
            .map_err(|error| scan_error(error, scan_origin))?;
        {
            let (direct_arg0, direct_arg1) = (class, text.tokens);
            finish_scan!(self.execute_mark(direct_arg0, direct_arg1))
        }
    }
}
