//! Native scan replay in the sole main-control engine.
use super::*;
impl<G> DirectDispatch<'_, '_, G> {
    pub(super) fn scan_replay_step(
        &mut self,
        alignment_preamble: Option<(AlignmentIdentity, AlignmentPreamblePhase)>,
    ) -> Result<ReplayStep, ExecError> {
        let mode = self.control.execution.modes.current_mode();
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

        let shown_mode = &mut self.control.execution.shown_mode;

        let scan_origin = self.current_origin;
        if let Some((alignment, phase)) = alignment_preamble {
            return match phase {
                AlignmentPreamblePhase::Opening => {
                    // TeX82 §§299/367: `scan_spec` expands its optional
                    // dimension while `init_align` is already in the alignment's
                    // newly pushed mode.  An expandable command such as `\the`
                    // therefore crosses `show_cur_cmd_chr` here, before ordinary
                    // main control gets another command.  Carry the same pending
                    // mode prefix and `shown_mode` update that `scan_step` owns
                    // around its `get_x_token` boundary.
                    prepare_command_trace(processor, mode, *shown_mode);
                    let packing = (processor
                        .scan_alignment_preamble_opening()
                        .map_err(command_error))
                    .map_err(|error| scan_error(error, scan_origin))?;
                    if processor.command_trace_printed() {
                        *shown_mode = Some(mode);
                    }
                    return {
                        let (direct_arg0, direct_arg1) = (alignment, packing);
                        finish_scan!(
                            self.execute_alignment_preamble_opening(direct_arg0, direct_arg1)
                        )
                    };
                }
                AlignmentPreamblePhase::Start { owner } => {
                    // TeX82 §§299, 367, 759, and 774: `init_align` has already
                    // pushed the alignment mode when §759 expands the token after
                    // `\span`. This scanner episode is the first processor that
                    // can print a command after that push, so it owns the pending
                    // mode prefix just like the packing-spec episode above.
                    prepare_command_trace(processor, mode, *shown_mode);
                    (processor
                        .begin_alignment_preamble_scan(owner)
                        .map_err(command_error))
                    .map_err(|error| scan_error(error, scan_origin))?;
                    if processor.command_trace_printed() {
                        *shown_mode = Some(mode);
                    }
                    return {
                        let (direct_arg0,) = (alignment,);
                        finish_scan!(self.execute_alignment_preamble_start(direct_arg0))
                    };
                }
                AlignmentPreamblePhase::CellOpening => {
                    let opening = (processor
                        .scan_alignment_cell_opening()
                        .map_err(command_error))
                    .map_err(|error| scan_error(error, scan_origin))?;
                    return {
                        let (direct_arg0, direct_arg1) = (alignment, opening);
                        finish_scan!(self.execute_alignment_cell_opening(direct_arg0, direct_arg1))
                    };
                }
                AlignmentPreamblePhase::NextCellOpening => {
                    let opening = (processor
                        .scan_alignment_next_cell_opening()
                        .map_err(command_error))
                    .map_err(|error| scan_error(error, scan_origin))?;
                    return {
                        let (direct_arg0, direct_arg1) = (alignment, opening);
                        finish_scan!(self.execute_alignment_cell_opening(direct_arg0, direct_arg1))
                    };
                }
                AlignmentPreamblePhase::AlignPeek { after_noalign } => {
                    return {
                        let (direct_arg0, direct_arg1) = (alignment, after_noalign);
                        finish_scan!(self.scan_alignment_peek(direct_arg0, direct_arg1))
                    };
                }
                AlignmentPreamblePhase::NoAlignBody => {
                    let (direct_arg0,) = (alignment,);
                    finish_scan!(self.scan_noalign_body(direct_arg0))
                }
                AlignmentPreamblePhase::CellDelivery => {
                    let (direct_arg0,) = (alignment,);
                    finish_scan!(self.scan_alignment_delivery_step(direct_arg0))
                }
            };
        }
        finish_scan!(self.run_command(None, None, true, None))
    }
}
