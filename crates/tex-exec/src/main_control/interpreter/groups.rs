//! Actual group entry, warning and native unsave semantics.
use super::*;

impl<G> MainControl<G> {
    pub(super) fn apply_native_group_leave(
        &mut self,
        context: &mut CommandContext<'_, G>,
        effects: &mut DiagnosticEffects,
        kind: GroupKind,
        error_context: &'static str,
    ) -> Result<ReplayStep, ExecError> {
        crate::box_runtime::flush_pending_hchars_with_fuel(
            &mut self.execution.modes,
            context,
            effects,
            self.fuel.fuel_mut(),
        )?;
        let mut facts = ExecutorHostFacts {
            modes: &self.execution.modes,
            pdf_ignore_depth: self.execution.pdf_ignore_depth,
            telemetry: &mut self.execution.episode_telemetry,
        };
        let mut processor = command_processor(
            &mut self.command,
            self.fuel.fuel_mut(),
            &mut self.capabilities,
            &mut facts as &mut dyn tex_command::CommandHostFacts<G>,
            &mut self.operation_observations,
            effects,
            context,
        );
        processor.set_output_routine_active(self.execution.boxes.output_routine_active);
        finish_native_group_leave(&mut processor, kind, error_context)
    }
}

/// Unsave through the actual scanner that owns aftergroup input delivery.
pub(super) fn finish_native_group_leave<G, Facts: tex_command::CommandHostFacts<G>>(
    processor: &mut CommandProcessor<'_, '_, G, Facts>,
    kind: GroupKind,
    error_context: &'static str,
) -> Result<ReplayStep, ExecError> {
    let (level, frame) =
        processor.with_execution_owners(|context, _, _, _, _observer, _execution, _, _, _| {
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
    processor.with_execution_owners(|context, _, effects, _, _observer, _execution, _, _, _| {
        context.trace_group_leave(
            effects,
            frame.kind(),
            u32::try_from(level).unwrap_or(u32::MAX),
            frame.entered_line(),
        );
    });
    Ok(ReplayStep::Continue)
}

/// Source-free group entry through the sole actual mode and save-stack owners.
pub(in crate::main_control) fn begin_native_group<G>(
    context: &mut CommandContext<'_, G>,
    command: &mut tex_command::CommandState<G>,
    effects: &mut DiagnosticEffects,
    fuel: &mut tex_command::CommandFuel,
    modes: &mut ModeNest,
    kind: GroupKind,
) -> Result<ReplayStep, ExecError> {
    if modes.current_list().pending_hchars().is_some() {
        crate::box_runtime::flush_pending_hchars_with_fuel(modes, context, effects, fuel)?;
    }
    enter_group(context, command, effects, kind);
    Ok(ReplayStep::Continue)
}

pub(super) fn close_tex_group<G>(
    processor: &mut Processor<'_, '_, G>,
    code: u64,
    spelling: TracedTokenWord,
    meaning: &tex_state::meaning::MeaningWord<G>,
    control_sequence: Option<Symbol>,
    exceptional: Option<&tex_command::CurrentCommand<G>>,
) -> Result<ReplayStep, ExecError> {
    let group = processor
        .with_execution_owners(|context, _, _, _, _, _, _, _, _| context.innermost_group_kind());
    match (code, group) {
        (codes::CHAR_END_GROUP, Some(GroupKind::Simple))
        | (codes::PRIMITIVE_END_GROUP, Some(GroupKind::SemiSimple)) => {
            report_scan(processor)?;
            processor.with_execution_owners(
                |context, _, effects, fuel, _, execution, _, _, _| {
                    execution.main_loop_active = false;
                    crate::box_runtime::flush_pending_hchars_with_fuel(
                        &mut execution.modes,
                        context,
                        effects,
                        fuel,
                    )
                },
            )?;
            finish_native_group_leave(
                processor,
                group.expect("selected native group"),
                "ordinary group",
            )
        }
        (codes::CHAR_END_GROUP, Some(GroupKind::Math)) => math::close_field(processor),
        (codes::CHAR_END_GROUP, Some(GroupKind::MathChoice)) => math::close_choice(processor),
        (codes::CHAR_END_GROUP, Some(GroupKind::Disc)) => discretionary::close_part(processor),
        _ => close_mode_group(
            processor,
            code,
            group,
            spelling,
            meaning,
            control_sequence,
            exceptional,
        ),
    }
}
