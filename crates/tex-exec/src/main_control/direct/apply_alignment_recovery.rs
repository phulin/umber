//! Apply scanned alignment recovery values directly.
use super::*;
impl<G> DirectDispatch<'_, '_, G> {
    pub(super) fn execute_misplaced_alignment_delimiter(
        &mut self,
        mut token: Token,
        mut context: tex_command::DiagnosticContextCoordinate,
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
                semantic::execute_misplaced_alignment_delimiter(
                    stores,
                    command,
                    &mut token,
                    &mut context,
                )
            },
        )
    }

    pub(super) fn execute_misplaced_alignment_command(
        &mut self,
        mut omit: bool,
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
                semantic::execute_misplaced_alignment_command(stores, command, &mut omit)
            },
        )
    }

    pub(super) fn execute_missing_alignment_cr(&mut self) -> Result<ReplayStep, ExecError> {
        self.report_scan()?;

        self.control.main_loop_active = false;

        self.control.apply_direct_leaf(
            self.stores,
            self.effects,
            self.provider,
            self.output_start,
            None,
            |stores, _, _, _, command, _, _, _, _, _, _, _| {
                semantic::execute_missing_alignment_cr(stores, command)
            },
        )
    }

    pub(super) fn execute_display_alignment_recovery(&mut self) -> Result<ReplayStep, ExecError> {
        self.report_scan()?;
        let applied = {
            self.control
                .recover_display_alignment_closer(self.stores, self.effects, self.provider)
        };
        self.finish_host(applied)
    }

    pub(super) fn execute_alignment_recovery(
        &mut self,
        mut brace: Catcode,
    ) -> Result<ReplayStep, ExecError> {
        self.report_scan()?;

        self.control.main_loop_active = false;
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
            |stores, _, _, _, command, boxes, _, _, _, _, _, _| {
                semantic::execute_alignment_recovery(stores, command, boxes, &mut brace)
            },
        )
    }

    pub(super) fn execute_illegal_h_align(
        &mut self,
        mut token: Token,
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
                semantic::execute_illegal_h_align(stores, modes, command, &mut token)
            },
        )
    }
}
