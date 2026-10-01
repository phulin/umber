//! Apply scanned boxes values directly.
use super::*;
impl<G> DirectDispatch<'_, '_, G> {
    pub(super) fn execute_box_dimension_assignment(
        &mut self,
        mut index: u16,
        mut dimension: tex_state::BoxDimension,
        mut value: Scaled,
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
                semantic::execute_box_dimension_assignment(
                    stores,
                    command,
                    &mut index,
                    &mut dimension,
                    &mut value,
                    &mut global,
                )
            },
        )?;
        self.schedule_assignment_token()?;
        Ok(step)
    }

    pub(super) fn execute_show_box(&mut self, mut index: u16) -> Result<ReplayStep, ExecError> {
        self.report_scan()?;

        self.control.main_loop_active = false;

        self.control.apply_direct_leaf(
            self.stores,
            self.effects,
            self.provider,
            self.output_start,
            None,
            |stores, _, _, _, command, _, _, _, _, _, _, _| {
                semantic::execute_show_box(stores, command, &mut index)
            },
        )
    }

    pub(super) fn execute_v_split(
        &mut self,
        mut operand0: ScannedVSplit,
    ) -> Result<ReplayStep, ExecError> {
        self.report_scan()?;

        self.control.main_loop_active = false;

        self.control.apply_direct_leaf(
            self.stores,
            self.effects,
            self.provider,
            self.output_start,
            None,
            |stores, modes, _, _, command, boxes, _, _, _, _, _, prepared_dvi_pages| {
                semantic::execute_v_split(
                    stores,
                    modes,
                    command,
                    boxes,
                    prepared_dvi_pages,
                    &mut operand0,
                )
            },
        )
    }

    pub(super) fn execute_box_register(
        &mut self,
        mut index: u16,
        mut copy: bool,
        mut ships_out: bool,
    ) -> Result<ReplayStep, ExecError> {
        self.report_scan()?;

        self.control.main_loop_active = false;

        self.control.apply_direct_leaf(
            self.stores,
            self.effects,
            self.provider,
            self.output_start,
            None,
            |stores, modes, _, _, command, boxes, _, _, _, _, _, prepared_dvi_pages| {
                semantic::execute_box_register(
                    stores,
                    modes,
                    command,
                    boxes,
                    prepared_dvi_pages,
                    &mut index,
                    &mut copy,
                    &mut ships_out,
                )
            },
        )
    }

    pub(super) fn execute_unbox(
        &mut self,
        mut primitive: UnexpandablePrimitive,
        mut index: u16,
        mut error_context: tex_command::DiagnosticContextCoordinate,
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
                semantic::execute_unbox(
                    stores,
                    modes,
                    command,
                    &mut primitive,
                    &mut index,
                    &mut error_context,
                )
            },
        )
    }

    pub(super) fn execute_saved_vertical_discards(
        &mut self,
        mut operand0: UnexpandablePrimitive,
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
                semantic::execute_saved_vertical_discards(stores, modes, command, &mut operand0)
            },
        )
    }

    pub(super) fn execute_last_box(
        &mut self,
        mut error_context: tex_command::DiagnosticContextCoordinate,
    ) -> Result<ReplayStep, ExecError> {
        self.report_scan()?;

        self.control.main_loop_active = false;

        self.control.apply_direct_leaf(
            self.stores,
            self.effects,
            self.provider,
            self.output_start,
            None,
            |stores, modes, _, _, command, boxes, _, _, _, _, _, prepared_dvi_pages| {
                semantic::execute_last_box(
                    stores,
                    modes,
                    command,
                    boxes,
                    prepared_dvi_pages,
                    &mut error_context,
                )
            },
        )
    }
}
