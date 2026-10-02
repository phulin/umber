//! Apply scanned box builders values directly.
use super::*;
impl<G> DirectDispatch<'_, '_, G> {
    pub(super) fn execute_begin_shipout(&mut self) -> Result<ReplayStep, ExecError> {
        self.report_scan()?;

        self.control.execution.main_loop_active = false;

        self.control.apply_direct_leaf(
            self.stores,
            self.effects,
            self.provider,
            self.output_start,
            None,
            |stores, _, _, _, _, boxes, _, _, _, _, _, _| {
                semantic::execute_begin_shipout(stores, boxes)
            },
        )
    }

    pub(super) fn execute_set_box(
        &mut self,
        mut target: SetBoxTarget,
        mut path: ScannedSetBoxPath,
    ) -> Result<ReplayStep, ExecError> {
        self.report_scan()?;

        self.control.execution.main_loop_active = false;

        let step = self.control.apply_direct_leaf(
            self.stores,
            self.effects,
            self.provider,
            self.output_start,
            None,
            |stores,
             modes,
             _,
             active_alignment,
             command,
             boxes,
             active_discretionaries,
             active_math_choices,
             active_math_fields,
             _,
             _,
             prepared_dvi_pages| {
                semantic::execute_set_box(
                    stores,
                    modes,
                    active_alignment,
                    command,
                    boxes,
                    active_discretionaries,
                    active_math_choices,
                    active_math_fields,
                    prepared_dvi_pages,
                    &mut target,
                    &mut path,
                )
            },
        )?;
        self.schedule_assignment_token()?;
        Ok(step)
    }

    pub(super) fn execute_begin_box(
        &mut self,
        mut operand0: ScannedBoxConstruction,
    ) -> Result<ReplayStep, ExecError> {
        self.report_scan()?;

        self.control.execution.main_loop_active = false;

        self.control.apply_direct_leaf(
            self.stores,
            self.effects,
            self.provider,
            self.output_start,
            None,
            |stores, modes, _, _, command, boxes, _, _, _, _, _, _| {
                semantic::execute_begin_box(stores, modes, command, boxes, &mut operand0)
            },
        )
    }

    pub(super) fn execute_box_shift(
        &mut self,
        mut operand0: ScannedBoxShift,
    ) -> Result<ReplayStep, ExecError> {
        self.report_scan()?;

        self.control.execution.main_loop_active = false;

        self.control.apply_direct_leaf(
            self.stores,
            self.effects,
            self.provider,
            self.output_start,
            None,
            |stores, modes, _, _, command, boxes, _, _, _, _, _, _| {
                semantic::execute_box_shift(stores, modes, command, boxes, &mut operand0)
            },
        )
    }

    pub(super) fn execute_illegal_box_shift(
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
                semantic::execute_illegal_box_shift(stores, modes, command, &mut token)
            },
        )
    }

    pub(super) fn execute_begin_insert(
        &mut self,
        mut operand0: ScannedInsertConstruction,
    ) -> Result<ReplayStep, ExecError> {
        self.report_scan()?;

        self.control.execution.main_loop_active = false;

        self.control.apply_direct_leaf(
            self.stores,
            self.effects,
            self.provider,
            self.output_start,
            None,
            |stores, modes, _, _, command, boxes, _, _, _, _, _, _| {
                semantic::execute_begin_insert(stores, modes, command, boxes, &mut operand0)
            },
        )
    }

    pub(super) fn execute_illegal_insert_or_adjust(
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
                semantic::execute_illegal_insert_or_adjust(stores, modes, command, &mut token)
            },
        )
    }

    pub(super) fn execute_box_end_group(
        &mut self,
        mut ships_out: bool,
        mut current_line: i32,
    ) -> Result<ReplayStep, ExecError> {
        self.report_scan()?;

        self.control.execution.main_loop_active = false;

        self.control.apply_direct_leaf(
            self.stores,
            self.effects,
            self.provider,
            self.output_start,
            None,
            |stores, modes, _, _, command, boxes, _, _, _, _, _, _| {
                semantic::execute_box_end_group(
                    stores,
                    modes,
                    command,
                    boxes,
                    &mut ships_out,
                    &mut current_line,
                )
            },
        )
    }
}
