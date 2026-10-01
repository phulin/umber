//! Native scan shapes in the sole main-control engine.
use super::*;
impl<G> DirectDispatch<'_, '_, G> {
    pub(super) fn scan_paragraph_shape_assignment(
        &mut self,
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

        let status = processor.scan_optional_equals_into(scalar);
        let _ = take_control_scalar!(scalar, status, take_boolean, scan_origin);
        let status = processor.scan_integer_into(scalar);
        let count = take_control_scalar!(scalar, status, take_integer, scan_origin)
            .value
            .max(0) as usize;
        let mut lines = Vec::new();
        (lines
            .try_reserve_exact(count)
            .map_err(|_| ExecError::ArithmeticOverflow))
        .map_err(|error| scan_error(error, scan_origin))?;
        for _ in 0..count {
            let status = processor.scan_dimension_into(scalar);
            let indent = take_control_scalar!(scalar, status, take_dimension, scan_origin).value;
            let status = processor.scan_dimension_into(scalar);
            let width = take_control_scalar!(scalar, status, take_dimension, scan_origin).value;
            lines.push(ParagraphShapeLine { indent, width });
        }
        {
            let (direct_arg0, direct_arg1) = (lines, global);
            finish_scan!(self.execute_paragraph_shape(direct_arg0, direct_arg1))
        }
    }

    pub(super) fn scan_penalty_array_assignment(
        &mut self,
        kind: tex_state::PenaltyArrayKind,
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

        let status = processor.scan_optional_equals_into(scalar);
        let _ = take_control_scalar!(scalar, status, take_boolean, scan_origin);
        let status = processor.scan_integer_into(scalar);
        let count = take_control_scalar!(scalar, status, take_integer, scan_origin)
            .value
            .max(0) as usize;
        let mut values = Vec::new();
        (values
            .try_reserve_exact(count)
            .map_err(|_| ExecError::ArithmeticOverflow))
        .map_err(|error| scan_error(error, scan_origin))?;
        for _ in 0..count {
            let status = processor.scan_integer_into(scalar);
            let value = take_control_scalar!(scalar, status, take_integer, scan_origin).value;
            values.push(value);
        }
        {
            let (direct_arg0, direct_arg1, direct_arg2) = (kind, values, global);
            finish_scan!(self.execute_penalty_array(direct_arg0, direct_arg1, direct_arg2))
        }
    }
}
