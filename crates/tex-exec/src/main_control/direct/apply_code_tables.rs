//! Apply scanned code tables values directly.
use super::*;
impl<G> DirectDispatch<'_, '_, G> {
    pub(super) fn execute_code_table(
        &mut self,
        mut primitive: UnexpandablePrimitive,
        mut character: char,
        mut value: i32,
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
                semantic::execute_code_table(
                    stores,
                    command,
                    &mut primitive,
                    &mut character,
                    &mut value,
                    &mut global,
                )
            },
        )?;
        self.schedule_assignment_token()?;
        Ok(step)
    }
}
