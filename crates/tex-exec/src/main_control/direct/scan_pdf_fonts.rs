//! Native scan pdf fonts in the sole main-control engine.
use super::*;
impl<G> DirectDispatch<'_, '_, G> {
    pub(super) fn scan_pdf_font_code_assignment(
        &mut self,
        primitive: UnexpandablePrimitive,
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

        let font = take_native_scalar!(processor.scan_font_selector_retained(), scan_origin);
        let character = take_native_scalar!(
            processor.scan_restricted_integer_retained(RestrictedIntegerClass::CharacterCode),
            scan_origin
        )
        .value;
        let character = u8::try_from(character).expect("pdfTeX character scanner is byte bounded");
        let _ = take_native_scalar!(processor.scan_optional_equals_retained(), scan_origin);
        let value = take_native_scalar!(processor.scan_integer_retained(), scan_origin).value;
        {
            let (direct_arg0, direct_arg1, direct_arg2, direct_arg3) =
                (pdf_font_code_table(primitive), font, character, value);
            finish_scan!(self.execute_pdf_font_code(
                direct_arg0,
                direct_arg1,
                direct_arg2,
                direct_arg3
            ))
        }
    }

    pub(super) fn scan_pdf_font_expand_assignment(&mut self) -> Result<ReplayStep, ExecError> {
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

        let font = take_native_scalar!(processor.scan_font_selector_retained(), scan_origin);
        let _ = take_native_scalar!(processor.scan_optional_equals_retained(), scan_origin);
        let stretch = take_native_scalar!(processor.scan_integer_retained(), scan_origin).value;
        let shrink = take_native_scalar!(processor.scan_integer_retained(), scan_origin).value;
        let step = take_native_scalar!(processor.scan_integer_retained(), scan_origin).value;
        let auto_expand =
            take_native_scalar!(processor.scan_keyword_retained("autoexpand"), scan_origin).value;
        let spec =
            (tex_typeset::expansion::FontExpansionSpec::new(stretch, shrink, step, auto_expand))
                .map_err(|error| scan_error(error, scan_origin))?;
        {
            let (direct_arg0, direct_arg1) = (font, spec);
            finish_scan!(self.execute_pdf_font_expand(direct_arg0, direct_arg1))
        }
    }
}
