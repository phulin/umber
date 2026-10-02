//! Apply scanned registers values directly.
use super::*;
impl<G> DirectDispatch<'_, '_, G> {
    pub(super) fn execute_invalid_arithmetic_target(
        &mut self,
        mut primitive: UnexpandablePrimitive,
        mut target: tex_command::PrintCommand<G>,
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
                semantic::execute_invalid_arithmetic_target(
                    stores,
                    command,
                    &mut primitive,
                    &mut target,
                )
            },
        )?;
        self.schedule_assignment_token()?;
        Ok(step)
    }
}
