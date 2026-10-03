//! Apply scanned text values directly.
use super::*;
impl<G> DirectDispatch<'_, '_, G> {
    pub(super) fn execute_text_direction(
        &mut self,
        mut direction: tex_state::node::Direction,
        mut enabled: bool,
    ) -> Result<ReplayStep, ExecError> {
        self.report_scan()?;

        self.control.execution.main_loop_active = false;

        self.control.apply_direct_leaf(
            self.stores,
            self.effects,
            self.provider,
            self.output_start,
            None,
            |stores, modes, _, _, command, _, _, _, _, _, _, _| {
                semantic::execute_text_direction(
                    stores,
                    modes,
                    command,
                    &mut direction,
                    &mut enabled,
                )
            },
        )
    }

    pub(super) fn execute_replay_completed(
        &mut self,
        operand0: tex_command::CommandReplayEpisode,
    ) -> Result<ReplayStep, ExecError> {
        self.report_scan()?;
        let applied = {
            self.control.execution.completed_replay_episode = Some(operand0);
            Ok(ReplayStep::Continue)
        };
        self.finish_host(applied)
    }

    pub(super) fn execute_horizontal_skip(
        &mut self,
        mut value: GlueSpec,
        mut shared_zero_glue: bool,
    ) -> Result<ReplayStep, ExecError> {
        self.report_scan()?;

        self.control.execution.main_loop_active = false;

        self.control.apply_direct_leaf(
            self.stores,
            self.effects,
            self.provider,
            self.output_start,
            None,
            |stores, modes, _, _, command, _, _, _, _, _, _, _| {
                semantic::execute_horizontal_skip(
                    stores,
                    modes,
                    command,
                    &mut value,
                    &mut shared_zero_glue,
                )
            },
        )
    }

    pub(super) fn execute_penalty(&mut self, mut amount: i32) -> Result<ReplayStep, ExecError> {
        self.report_scan()?;

        self.control.execution.main_loop_active = false;

        self.control.apply_direct_leaf(
            self.stores,
            self.effects,
            self.provider,
            self.output_start,
            None,
            |stores, modes, _, _, command, _, _, _, _, _, _, _| {
                semantic::execute_penalty(stores, modes, command, &mut amount)
            },
        )
    }

    pub(super) fn execute_character_code(
        &mut self,
        mut value: i32,
        mut origin: tex_state::token::OriginId,
        mut suppress_left_boundary: bool,
    ) -> Result<ReplayStep, ExecError> {
        self.report_scan()?;
        suppress_left_boundary |= self.suppress_left_boundary;
        self.control.execution.main_loop_active = false;

        let step = self.control.apply_direct_leaf(
            self.stores,
            self.effects,
            self.provider,
            self.output_start,
            None,
            |stores, modes, _, _, command, _, _, _, _, _, _, _| {
                semantic::execute_character_code(
                    stores,
                    modes,
                    command,
                    &mut value,
                    &mut origin,
                    &mut suppress_left_boundary,
                )
            },
        )?;
        self.control.resume_character_loop(
            self.stores,
            u32::try_from(value).ok().and_then(char::from_u32),
        );
        Ok(step)
    }

    pub(super) fn execute_delete_last(
        &mut self,
        mut primitive: UnexpandablePrimitive,
        mut context: tex_command::DiagnosticContextCoordinate,
    ) -> Result<ReplayStep, ExecError> {
        self.report_scan()?;

        self.control.execution.main_loop_active = false;

        self.control.apply_direct_leaf(
            self.stores,
            self.effects,
            self.provider,
            self.output_start,
            None,
            |stores, modes, _, _, command, _, _, _, _, _, _, _| {
                semantic::execute_delete_last(stores, modes, command, &mut primitive, &mut context)
            },
        )
    }

    pub(super) fn execute_italic_correction(&mut self) -> Result<ReplayStep, ExecError> {
        self.report_scan()?;

        self.control.execution.main_loop_active = false;

        self.control.apply_direct_leaf(
            self.stores,
            self.effects,
            self.provider,
            self.output_start,
            None,
            |stores, modes, _, _, command, _, _, _, _, _, _, _| {
                semantic::execute_italic_correction(stores, modes, command)
            },
        )
    }

    pub(super) fn execute_no_boundary(
        &mut self,
        mut suppress_right: bool,
    ) -> Result<ReplayStep, ExecError> {
        self.report_scan()?;

        self.control.execution.main_loop_active = false;

        self.control.apply_direct_leaf(
            self.stores,
            self.effects,
            self.provider,
            self.output_start,
            None,
            |stores, modes, _, _, command, _, _, _, _, _, _, _| {
                semantic::execute_no_boundary(stores, modes, command, &mut suppress_right)
            },
        )
    }

    pub(super) fn execute_non_script(&mut self) -> Result<ReplayStep, ExecError> {
        self.report_scan()?;

        self.control.execution.main_loop_active = false;

        self.control.apply_direct_leaf(
            self.stores,
            self.effects,
            self.provider,
            self.output_start,
            None,
            |stores, modes, _, _, _, _, _, _, _, _, _, _| {
                semantic::execute_non_script(stores, modes)
            },
        )
    }

    pub(super) fn execute_control_space(&mut self) -> Result<ReplayStep, ExecError> {
        self.report_scan()?;

        self.control.execution.main_loop_active = false;

        self.control.apply_direct_leaf(
            self.stores,
            self.effects,
            self.provider,
            self.output_start,
            None,
            |stores, modes, _, _, command, _, _, _, _, _, _, _| {
                semantic::execute_control_space(stores, modes, command)
            },
        )
    }

    pub(super) fn execute_fixed_horizontal_glue(
        &mut self,
        mut primitive: UnexpandablePrimitive,
    ) -> Result<ReplayStep, ExecError> {
        self.report_scan()?;

        self.control.execution.main_loop_active = false;

        self.control.apply_direct_leaf(
            self.stores,
            self.effects,
            self.provider,
            self.output_start,
            None,
            |stores, modes, _, _, command, _, _, _, _, _, _, _| {
                semantic::execute_fixed_horizontal_glue(stores, modes, command, &mut primitive)
            },
        )
    }

    pub(super) fn execute_rule(
        &mut self,
        mut width: Option<Scaled>,
        mut height: Option<Scaled>,
        mut depth: Option<Scaled>,
        mut horizontal: bool,
    ) -> Result<ReplayStep, ExecError> {
        self.report_scan()?;

        self.control.execution.main_loop_active = false;

        self.control.apply_direct_leaf(
            self.stores,
            self.effects,
            self.provider,
            self.output_start,
            None,
            |stores, modes, _, _, command, _, _, _, _, _, _, _| {
                semantic::execute_rule(
                    stores,
                    modes,
                    command,
                    &mut width,
                    &mut height,
                    &mut depth,
                    &mut horizontal,
                )
            },
        )
    }

    pub(super) fn execute_character(
        &mut self,
        mut ch: char,
        mut cat: Catcode,
        mut origin: tex_state::token::OriginId,
        mut suppress_left_boundary: bool,
    ) -> Result<ReplayStep, ExecError> {
        self.report_scan()?;
        suppress_left_boundary |= self.suppress_left_boundary;
        self.control.execution.main_loop_active = false;

        let step = self.control.apply_direct_leaf(
            self.stores,
            self.effects,
            self.provider,
            self.output_start,
            None,
            |stores, modes, _, _, command, _, _, _, _, _, _, _| {
                semantic::execute_character(
                    stores,
                    modes,
                    command,
                    &mut ch,
                    &mut cat,
                    &mut origin,
                    &mut suppress_left_boundary,
                )
            },
        )?;
        self.control.resume_character_loop(
            self.stores,
            matches!(cat, Catcode::Letter | Catcode::Other).then_some(ch),
        );
        Ok(step)
    }

    pub(super) fn execute_accent(
        &mut self,
        operand0: ScannedAccent,
    ) -> Result<ReplayStep, ExecError> {
        self.report_scan()?;
        let applied = {
            self.control.apply_accent(
                operand0,
                self.stores,
                self.effects,
                self.output_start.tracked_region_is_active,
                self.provider,
            )
        };
        self.finish_host(applied)
    }

    pub(super) fn execute_discretionary_opening(
        &mut self,
        operand0: ScannedDiscretionaryOpening,
    ) -> Result<ReplayStep, ExecError> {
        self.report_scan()?;
        let applied = {
            self.control
                .begin_discretionary(operand0, self.stores, self.effects)
        };
        self.finish_host(applied)
    }

    pub(super) fn execute_discretionary_part_end(&mut self) -> Result<ReplayStep, ExecError> {
        self.report_scan()?;
        let applied = {
            self.control
                .finish_discretionary_part(self.stores, self.effects, self.provider)
        };
        self.finish_host(applied)
    }

    pub(super) fn execute_discretionary_hyphen(
        &mut self,
        origin: tex_state::token::OriginId,
    ) -> Result<ReplayStep, ExecError> {
        self.report_scan()?;
        let applied = {
            self.control
                .apply_discretionary_hyphen(origin, self.stores, self.effects)
        };
        self.finish_host(applied)
    }
}
