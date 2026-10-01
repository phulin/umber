//! Apply scanned math values directly.
use super::*;
impl<G> DirectDispatch<'_, '_, G> {
    pub(super) fn execute_missing_math_shift(&mut self) -> Result<ReplayStep, ExecError> {
        self.report_scan()?;

        self.control.main_loop_active = false;

        self.control.apply_direct_leaf(
            self.stores,
            self.effects,
            self.provider,
            self.output_start,
            None,
            |stores, _, _, _, command, _, _, _, _, _, _, _| {
                semantic::execute_missing_math_shift(stores, command)
            },
        )
    }

    pub(super) fn execute_math(&mut self, operand0: MathRequest) -> Result<ReplayStep, ExecError> {
        self.report_scan()?;
        let applied = {
            self.control.apply_math_request(
                operand0,
                self.stores,
                self.effects,
                self.output_start.tracked_region_is_active,
                self.provider,
            )
        };
        self.finish_host(applied)
    }

    pub(super) fn execute_math_delimiter(
        &mut self,
        operand0: MathDelimiterBoundary,
    ) -> Result<ReplayStep, ExecError> {
        self.report_scan()?;
        let applied = {
            self.control
                .apply_math_delimiter(operand0, self.stores, self.effects)
        };
        self.finish_host(applied)
    }

    pub(super) fn execute_math_family(
        &mut self,
        mut family: tex_command::ScannedMathFamily,
        mut font: FontId,
        mut global: bool,
    ) -> Result<ReplayStep, ExecError> {
        self.report_scan()?;

        self.control.main_loop_active = false;

        let step = self.control.apply_direct_leaf(
            self.stores,
            self.effects,
            self.provider,
            self.output_start,
            None,
            |stores, _, _, _, command, _, _, _, _, _, _, _| {
                semantic::execute_math_family(stores, command, &mut family, &mut font, &mut global)
            },
        )?;
        self.schedule_assignment_token()?;
        Ok(step)
    }

    pub(super) fn execute_end_math_group(
        &mut self,
        mut operand0: GroupKind,
    ) -> Result<ReplayStep, ExecError> {
        self.report_scan()?;

        self.control.main_loop_active = false;

        self.control.apply_direct_leaf(
            self.stores,
            self.effects,
            self.provider,
            self.output_start,
            None,
            |stores, modes, _, _, command, _, _, _, active_math_fields, _, _, _| {
                semantic::execute_end_math_group(
                    stores,
                    modes,
                    command,
                    active_math_fields,
                    &mut operand0,
                )
            },
        )
    }

    pub(super) fn execute_illegal_eq_no(
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
                semantic::execute_illegal_eq_no(stores, modes, command, &mut token)
            },
        )
    }

    pub(super) fn execute_math_shift(
        &mut self,
        pairing: MathShiftPairing,
    ) -> Result<ReplayStep, ExecError> {
        self.report_scan()?;
        let applied = {
            self.control
                .apply_math_shift(pairing, self.stores, self.effects, self.provider)
        };
        self.finish_host(applied)
    }
}
