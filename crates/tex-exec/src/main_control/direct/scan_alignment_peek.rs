//! Native scan alignment peek in the sole main-control engine.
use super::*;
impl<G> DirectDispatch<'_, '_, G> {
    pub(super) fn scan_alignment_peek(
        &mut self,
        alignment: AlignmentIdentity,
        _after_noalign: bool,
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
                direct_finish_scan!(self, owned_processor, admitted_context, host_facts, $call)
            };
        }

        let scan_origin = self.current_origin;

        (processor
            .begin_alignment_peek(_after_noalign)
            .map_err(command_error))
        .map_err(|error| scan_error(error, scan_origin))?;
        let lookahead = ((processor.next_alignment_lookahead().map_err(command_error))
            .map_err(|error| scan_error(error, scan_origin))?
            .ok_or(ExecError::MissingToken {
                context: "alignment lookahead",
            }))
        .map_err(|error| scan_error(error, scan_origin))?;
        match lookahead.command().meaning() {
            ResolvedMeaning::Static(Meaning::UnexpandablePrimitive(
                UnexpandablePrimitive::NoAlign,
            )) => {
                let _ = processor.commit_alignment_lookahead_delivery(lookahead);
                (processor
                    .scan_alignment_noalign_opening()
                    .map_err(command_error))
                .map_err(|error| scan_error(error, scan_origin))?;
                {
                    let (direct_arg0,) = (alignment,);
                    finish_scan!(self.execute_begin_no_align(direct_arg0))
                }
            }
            ResolvedMeaning::Static(Meaning::UnexpandablePrimitive(
                UnexpandablePrimitive::CrCr,
            )) => {
                let _ = processor.commit_alignment_lookahead_delivery(lookahead);
                {
                    let (direct_arg0,) = (alignment,);
                    finish_scan!(self.execute_align_peek_restart(direct_arg0))
                }
            }
            ResolvedMeaning::Static(Meaning::CharToken {
                cat: Catcode::EndGroup,
                ..
            }) => {
                let command = processor.commit_alignment_lookahead_delivery(lookahead);
                let current_line = command
                    .direct_source_line_number()
                    .unwrap_or_else(|| processor.current_file_line_number());
                {
                    let (direct_arg0, direct_arg1) = (alignment, current_line);
                    finish_scan!(self.execute_alignment_finish(direct_arg0, direct_arg1))
                }
            }
            ResolvedMeaning::Static(Meaning::UnexpandablePrimitive(
                UnexpandablePrimitive::Omit,
            )) => {
                let _ = processor.commit_alignment_lookahead_delivery(lookahead);
                {
                    let (direct_arg0, direct_arg1) = (alignment, true);
                    finish_scan!(self.execute_alignment_peek_cell(direct_arg0, direct_arg1))
                }
            }
            _ => {
                (processor
                    .back_alignment_lookahead(lookahead)
                    .map_err(command_error))
                .map_err(|error| scan_error(error, scan_origin))?;
                {
                    let (direct_arg0, direct_arg1) = (alignment, false);
                    finish_scan!(self.execute_alignment_peek_cell(direct_arg0, direct_arg1))
                }
            }
        }
    }
}
