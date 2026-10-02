//! Apply scanned groups values directly.
use super::*;
impl<G> DirectDispatch<'_, '_, G> {
    pub(super) fn execute_after_group(
        &mut self,
        mut operand0: tex_state::token::TracedTokenWord,
    ) -> Result<ReplayStep, ExecError> {
        self.report_scan()?;

        self.control.main_loop_active = false;

        self.control.apply_direct_leaf(
            self.stores,
            self.effects,
            self.provider,
            self.output_start,
            None,
            |stores, _, _, _, command, _, _, _, _, _, _, _| {
                semantic::execute_after_group(stores, command, &mut operand0)
            },
        )
    }

    pub(super) fn execute_after_assignment(
        &mut self,
        mut operand0: tex_state::token::TracedTokenWord,
    ) -> Result<ReplayStep, ExecError> {
        self.report_scan()?;

        self.control.main_loop_active = false;

        self.control.apply_direct_leaf(
            self.stores,
            self.effects,
            self.provider,
            self.output_start,
            None,
            |stores, _, _, _, command, _, _, _, _, _, _, _| {
                semantic::execute_after_assignment(stores, command, &mut operand0)
            },
        )
    }

    pub(super) fn execute_begin_simple_group(&mut self) -> Result<ReplayStep, ExecError> {
        self.report_scan()?;

        self.control.main_loop_active = false;

        self.control.apply_direct_leaf(
            self.stores,
            self.effects,
            self.provider,
            self.output_start,
            None,
            |stores, _, _, _, command, boxes, _, _, _, _, _, _| {
                semantic::execute_begin_simple_group(stores, command, boxes)
            },
        )
    }

    pub(super) fn execute_end_simple_group(&mut self) -> Result<ReplayStep, ExecError> {
        self.report_scan()?;

        self.control.main_loop_active = false;

        self.control.apply_direct_leaf(
            self.stores,
            self.effects,
            self.provider,
            self.output_start,
            None,
            |stores, _, _, _, command, boxes, _, _, _, _, _, _| {
                semantic::execute_end_simple_group(stores, command, boxes)
            },
        )
    }

    pub(super) fn execute_output_routine_opening_brace(&mut self) -> Result<ReplayStep, ExecError> {
        self.report_scan()?;

        self.control.main_loop_active = false;

        self.control.apply_direct_leaf(
            self.stores,
            self.effects,
            self.provider,
            self.output_start,
            None,
            |stores, modes, _, _, command, boxes, _, _, _, _, _, _| {
                semantic::execute_output_routine_opening_brace(stores, modes, command, boxes)
            },
        )
    }

    pub(super) fn execute_end_output_routine(&mut self) -> Result<ReplayStep, ExecError> {
        self.report_scan()?;

        self.control.main_loop_active = false;

        self.control.apply_direct_leaf(
            self.stores,
            self.effects,
            self.provider,
            self.output_start,
            None,
            |stores, modes, _, _, command, boxes, _, _, _, _, _, _| {
                semantic::execute_end_output_routine(stores, modes, command, boxes)
            },
        )
    }
}

impl<G> MainControl<G> {
    pub(super) fn apply_native_group_enter(
        &mut self,
        context: &mut CommandContext<'_, G>,
        effects: &mut DiagnosticEffects,
        kind: GroupKind,
    ) -> Result<ReplayStep, ExecError> {
        crate::box_runtime::flush_pending_hchars_with_fuel(
            &mut self.modes,
            context,
            effects,
            self.fuel.fuel_mut(),
        )?;
        enter_group(context, &mut self.command, effects, kind);
        Ok(ReplayStep::Continue)
    }

    pub(super) fn apply_native_group_leave(
        &mut self,
        context: &mut CommandContext<'_, G>,
        effects: &mut DiagnosticEffects,
        kind: GroupKind,
        error_context: &'static str,
    ) -> Result<ReplayStep, ExecError> {
        crate::box_runtime::flush_pending_hchars_with_fuel(
            &mut self.modes,
            context,
            effects,
            self.fuel.fuel_mut(),
        )?;
        let mut facts = ExecutorHostFacts {
            modes: &self.modes,
            pdf_ignore_depth: self.pdf_ignore_depth,
            telemetry: &mut self.episode_telemetry,
        };
        let mut processor = command_processor(
            &mut self.command,
            self.fuel.fuel_mut(),
            &mut self.capabilities,
            &mut facts,
            &mut self.operation_observations,
            effects,
            context,
        );
        processor.set_output_routine_active(self.boxes.output_routine_active);
        finish_native_group_leave(&mut processor, kind, error_context)
    }
}

/// Unsave through the actual scanner that owns aftergroup input delivery.
pub(super) fn finish_native_group_leave<G>(
    processor: &mut CommandProcessor<'_, '_, G>,
    kind: GroupKind,
    error_context: &'static str,
) -> Result<ReplayStep, ExecError> {
    let (level, frame) = processor.apply_completed_command(|context, _, _, _| {
        let frames = context.group_frames();
        (frames.len(), frames.last().copied())
    });
    if let Some(frame) = frame {
        processor.warn_cross_file_group_close(
            level,
            frame.kind().group_text(),
            frame.entered_line(),
        );
    }
    let result = processor.end_group(kind, crate::assignments::tracing::trace_group_restoration);
    let frame = result.map_err(|_| ExecError::MissingToken {
        context: error_context,
    })?;
    processor.apply_completed_command(|context, _, effects, _| {
        context.trace_group_leave(
            effects,
            frame.kind(),
            u32::try_from(level).unwrap_or(u32::MAX),
            frame.entered_line(),
        );
    });
    Ok(ReplayStep::Continue)
}
