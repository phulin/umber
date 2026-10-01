//! Native scan alignment events in the sole main-control engine.
use super::*;
impl<G> DirectDispatch<'_, '_, G> {
    pub(super) fn scan_alignment_delivery_event(
        &mut self,
        alignment: AlignmentIdentity,
        event: tex_command::AlignmentDeliveryEvent<G>,
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

        let scan_origin = self.current_origin;

        match event {
            tex_command::AlignmentDeliveryEvent::EndTemplate(delimiter) => {
                (processor
                    .begin_alignment_v_template(
                        alignment,
                        tex_command::AlignmentDeliveryEvent::EndTemplate(delimiter),
                    )
                    .map_err(command_error))
                .map_err(|error| scan_error(error, scan_origin))?;
                finish_scan!(self.execute_alignment_template_entered())
            }
            tex_command::AlignmentDeliveryEvent::ClosingBrace(_) => {
                // TeX82 §1132 selects this executor-owned align_group branch. Raw
                // brace backup/correction and frozen-\cr insertion remain entirely
                // command-owned.
                (processor
                    .recover_alignment_closing_brace(event)
                    .map_err(command_error))
                .map_err(|error| scan_error(error, scan_origin))?;
                finish_scan!(self.execute_missing_alignment_cr())
            }
        }
    }

    pub(super) fn scan_display_alignment_tail(&mut self) -> Result<ReplayStep, ExecError> {
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

        let scan_origin = self.current_origin;
        let command = (processor
            .next_do_assignments_command()
            .map_err(command_error))
        .map_err(|error| scan_error(error, scan_origin))?;
        let Some(command) = command else {
            return finish_scan!(self.execute_end_of_input());
        };
        if tex_command::exceeds_max_non_prefixed_command(static_meaning(command.meaning()))
            || matches!(
                command.meaning(),
                ResolvedMeaning::Static(Meaning::CharToken {
                    cat: Catcode::MathShift,
                    ..
                })
            )
        {
            finish_scan!(self.run_command(Some(command), None, true, None))
        } else {
            (processor.back_input(command).map_err(command_error))
                .map_err(|error| scan_error(error, scan_origin))?;
            finish_scan!(self.execute_display_alignment_recovery())
        }
    }
}
