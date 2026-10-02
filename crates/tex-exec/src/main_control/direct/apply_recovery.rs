//! Apply scanned recovery values directly.
use super::*;
impl<G> DirectDispatch<'_, '_, G> {
    pub(super) fn execute_illegal_stop(
        &mut self,
        mut token: Token,
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
                semantic::execute_illegal_stop(stores, modes, command, &mut token)
            },
        )
    }

    pub(super) fn execute_illegal_macro_parameter(
        &mut self,
        mut token: Token,
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
                semantic::execute_illegal_macro_parameter(stores, modes, command, &mut token)
            },
        )
    }

    pub(super) fn execute_extra_end_cs_name(&mut self) -> Result<ReplayStep, ExecError> {
        self.report_scan()?;

        self.control.execution.main_loop_active = false;

        self.control.apply_direct_leaf(
            self.stores,
            self.effects,
            self.provider,
            self.output_start,
            None,
            |stores, _, _, _, command, _, _, _, _, _, _, _| {
                semantic::execute_extra_end_cs_name(stores, command)
            },
        )
    }

    pub(super) fn execute_illegal_italic_correction(
        &mut self,
        mut token: Token,
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
                semantic::execute_illegal_italic_correction(stores, modes, command, &mut token)
            },
        )
    }

    pub(super) fn execute_illegal_prev_depth(
        &mut self,
        mut token: Token,
    ) -> Result<ReplayStep, ExecError> {
        self.report_scan()?;

        self.control.execution.main_loop_active = false;
        {
            self.stores
                .world_mut()
                .publish_diagnostic_effects_preserving(self.effects);
        }

        self.control.apply_direct_leaf(
            self.stores,
            self.effects,
            self.provider,
            self.output_start,
            None,
            |stores, modes, _, _, command, _, _, _, _, _, _, _| {
                semantic::execute_illegal_prev_depth(stores, modes, command, &mut token)
            },
        )
    }

    pub(super) fn execute_illegal_space_factor(
        &mut self,
        mut token: Token,
    ) -> Result<ReplayStep, ExecError> {
        self.report_scan()?;

        self.control.execution.main_loop_active = false;
        {
            self.stores
                .world_mut()
                .publish_diagnostic_effects_preserving(self.effects);
        }

        self.control.apply_direct_leaf(
            self.stores,
            self.effects,
            self.provider,
            self.output_start,
            None,
            |stores, modes, _, _, command, _, _, _, _, _, _, _| {
                semantic::execute_illegal_space_factor(stores, modes, command, &mut token)
            },
        )
    }

    pub(super) fn execute_illegal_set_language(
        &mut self,
        mut token: Token,
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
                semantic::execute_illegal_set_language(stores, modes, command, &mut token)
            },
        )
    }

    pub(super) fn execute_extra_right_brace(
        &mut self,
        mut forgotten: Option<ForgottenGroupOpener>,
    ) -> Result<ReplayStep, ExecError> {
        self.report_scan()?;

        self.control.execution.main_loop_active = false;

        self.control.apply_direct_leaf(
            self.stores,
            self.effects,
            self.provider,
            self.output_start,
            None,
            |stores, _, _, _, command, _, _, _, _, _, _, _| {
                semantic::execute_extra_right_brace(stores, command, &mut forgotten)
            },
        )
    }

    pub(super) fn execute_off_save(
        &mut self,
        mut operand0: OffSaveCloser,
    ) -> Result<ReplayStep, ExecError> {
        self.report_scan()?;

        self.control.execution.main_loop_active = false;
        {
            self.stores
                .world_mut()
                .publish_diagnostic_effects_preserving(self.effects);
        }

        self.control.apply_direct_leaf(
            self.stores,
            self.effects,
            self.provider,
            self.output_start,
            None,
            |stores, _, _, _, command, _, _, _, _, _, _, _| {
                semantic::execute_off_save(stores, command, &mut operand0)
            },
        )
    }

    pub(super) fn execute_off_save_bottom_drop(
        &mut self,
        mut token: Token,
    ) -> Result<ReplayStep, ExecError> {
        self.report_scan()?;

        self.control.execution.main_loop_active = false;

        self.control.apply_direct_leaf(
            self.stores,
            self.effects,
            self.provider,
            self.output_start,
            None,
            |stores, _, _, _, command, _, _, _, _, _, _, _| {
                semantic::execute_off_save_bottom_drop(stores, command, &mut token)
            },
        )
    }

    pub(super) fn execute_illegal_last_item(
        &mut self,
        mut token: Token,
        mut context: String,
        mut site: tex_state::diagnostic::DiagnosticSite,
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
                semantic::execute_illegal_last_item(
                    stores,
                    modes,
                    command,
                    &mut token,
                    &mut context,
                    &mut site,
                )
            },
        )
    }
}
