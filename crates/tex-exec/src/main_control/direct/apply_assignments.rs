//! Apply scanned assignments values directly.
use super::*;
impl<G> DirectDispatch<'_, '_, G> {
    pub(super) fn execute_paragraph_shape(
        &mut self,
        mut lines: Vec<ParagraphShapeLine>,
        mut global: bool,
    ) -> Result<ReplayStep, ExecError> {
        self.report_scan()?;

        self.control.execution.main_loop_active = false;

        let step = self.control.apply_direct_leaf(
            self.stores,
            self.effects,
            self.provider,
            self.output_start,
            None,
            |stores, _, _, _, command, _, _, _, _, _, _, _| {
                semantic::execute_paragraph_shape(stores, command, &mut lines, &mut global)
            },
        )?;
        self.schedule_assignment_token()?;
        Ok(step)
    }

    pub(super) fn execute_penalty_array(
        &mut self,
        mut kind: PenaltyArrayKind,
        mut values: Vec<i32>,
        mut global: bool,
    ) -> Result<ReplayStep, ExecError> {
        self.report_scan()?;

        self.control.execution.main_loop_active = false;

        let step = self.control.apply_direct_leaf(
            self.stores,
            self.effects,
            self.provider,
            self.output_start,
            None,
            |stores, _, _, _, command, _, _, _, _, _, _, _| {
                semantic::execute_penalty_array(
                    stores,
                    command,
                    &mut kind,
                    &mut values,
                    &mut global,
                )
            },
        )?;
        self.schedule_assignment_token()?;
        Ok(step)
    }

    pub(super) fn execute_character_definition(
        &mut self,
        mut primitive: UnexpandablePrimitive,
        mut target: Symbol,
        mut value: i32,
        mut global: bool,
    ) -> Result<ReplayStep, ExecError> {
        self.report_scan()?;

        self.control.execution.main_loop_active = false;

        let step = self.control.apply_direct_leaf(
            self.stores,
            self.effects,
            self.provider,
            self.output_start,
            None,
            |stores, _, _, _, command, _, _, _, _, _, _, _| {
                semantic::execute_character_definition(
                    stores,
                    command,
                    &mut primitive,
                    &mut target,
                    &mut value,
                    &mut global,
                )
            },
        )?;
        self.schedule_assignment_token()?;
        Ok(step)
    }

    pub(super) fn execute_register_definition(
        &mut self,
        mut primitive: UnexpandablePrimitive,
        mut target: Symbol,
        mut index: u16,
        mut global: bool,
    ) -> Result<ReplayStep, ExecError> {
        self.report_scan()?;

        self.control.execution.main_loop_active = false;

        let step = self.control.apply_direct_leaf(
            self.stores,
            self.effects,
            self.provider,
            self.output_start,
            None,
            |stores, _, _, _, command, _, _, _, _, _, _, _| {
                semantic::execute_register_definition(
                    stores,
                    command,
                    &mut primitive,
                    &mut target,
                    &mut index,
                    &mut global,
                )
            },
        )?;
        self.schedule_assignment_token()?;
        Ok(step)
    }

    pub(super) fn execute_catcode(
        &mut self,
        character: char,
        value: i32,
        global: bool,
    ) -> Result<ReplayStep, ExecError> {
        self.report_scan()?;
        self.control.execution.main_loop_active = false;

        let step = self.control.apply_direct_leaf(
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
                definition_assignments::apply_catcode(character, value, global, stores, command)
            },
        )?;
        self.schedule_assignment_token()?;
        Ok(step)
    }
}
