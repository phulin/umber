//! Native scan fonts in the sole main-control engine.
use super::*;
impl<G> DirectDispatch<'_, '_, G> {
    pub(super) fn scan_font_dimen_assignment(&mut self) -> Result<ReplayStep, ExecError> {
        let boxes = &self.control.execution.boxes;

        let mut admitted_context = self
            .stores
            .command_context()
            .expect("direct scanner admission");

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
                direct_finish_scan!(self, owned_processor, admitted_context, $call)
            };
        }

        let scan_origin = self.current_origin;

        let number = take_native_scalar!(processor.scan_integer_retained(), scan_origin).value;
        let font = take_native_scalar!(processor.scan_font_selector_retained(), scan_origin);
        let recovery_context =
            (!processor.font_dimen_writable(font, number)).then(|| processor.error_context());
        let _ = take_native_scalar!(processor.scan_optional_equals_retained(), scan_origin);
        let value = take_native_scalar!(processor.scan_dimension_retained(), scan_origin);
        {
            let (direct_arg0, direct_arg1, direct_arg2, direct_arg3) =
                (font, number, value.value, recovery_context);
            finish_scan!(self.execute_font_dimen(
                direct_arg0,
                direct_arg1,
                direct_arg2,
                direct_arg3
            ))
        }
    }

    pub(super) fn scan_font_integer_assignment(
        &mut self,
        primitive: UnexpandablePrimitive,
    ) -> Result<ReplayStep, ExecError> {
        let boxes = &self.control.execution.boxes;

        let mut admitted_context = self
            .stores
            .command_context()
            .expect("direct scanner admission");

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
                direct_finish_scan!(self, owned_processor, admitted_context, $call)
            };
        }

        let scan_origin = self.current_origin;

        let font = take_native_scalar!(processor.scan_font_selector_retained(), scan_origin);
        let _ = take_native_scalar!(processor.scan_optional_equals_retained(), scan_origin);
        let value = take_native_scalar!(processor.scan_integer_retained(), scan_origin).value;
        {
            let (direct_arg0, direct_arg1, direct_arg2) =
                (font, primitive == UnexpandablePrimitive::SkewChar, value);
            finish_scan!(self.execute_font_integer(direct_arg0, direct_arg1, direct_arg2))
        }
    }

    pub(super) fn scan_font_only_operation(
        &mut self,
        meaning: Meaning,
    ) -> Result<ReplayStep, ExecError> {
        let boxes = &self.control.execution.boxes;

        let mut admitted_context = self
            .stores
            .command_context()
            .expect("direct scanner admission");

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
                direct_finish_scan!(self, owned_processor, admitted_context, $call)
            };
        }

        let scan_origin = self.current_origin;

        let font = take_native_scalar!(processor.scan_font_selector_retained(), scan_origin);
        match meaning {
            Meaning::UnexpandablePrimitive(UnexpandablePrimitive::PdfNoLigatures) => {
                let (direct_arg0,) = (font,);
                finish_scan!(self.execute_pdf_no_ligatures(direct_arg0))
            }
            _ => unreachable!("font-only descriptor restricts command meanings"),
        }
    }
}
