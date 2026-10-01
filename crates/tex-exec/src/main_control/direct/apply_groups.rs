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

    pub(super) fn execute_enter_group(&mut self, kind: GroupKind) -> Result<ReplayStep, ExecError> {
        self.report_scan()?;
        self.control.main_loop_active = false;

        self.control.apply_direct_leaf(
            self.stores,
            self.effects,
            self.provider,
            self.output_start,
            None,
            |stores, modes, _, _, command, _, _, _, _, _, _, _| {
                crate::box_runtime::flush_pending_hchars_with_fuel(
                    modes,
                    stores,
                    command.diagnostic_effects,
                    command.fuel,
                )?;
                {
                    enter_group(stores, command.state, command.diagnostic_effects, kind);
                    Ok(ReplayStep::Continue)
                }
            },
        )
    }

    pub(super) fn execute_leave_group(
        &mut self,
        kind: GroupKind,
        context: &'static str,
    ) -> Result<ReplayStep, ExecError> {
        self.report_scan()?;
        self.control.main_loop_active = false;

        self.control.apply_direct_leaf(
            self.stores,
            self.effects,
            self.provider,
            self.output_start,
            None,
            |stores, modes, _, _, command, _, _, _, _, _, _, _| {
                crate::box_runtime::flush_pending_hchars_with_fuel(
                    modes,
                    stores,
                    command.diagnostic_effects,
                    command.fuel,
                )?;
                definition_assignments::leave_group(
                    kind,
                    context,
                    &mut LinearCommandContext::new(stores),
                    command,
                )
            },
        )
    }
}
