//! External font resource resolution and installation.
use super::*;
impl<G> DirectDispatch<'_, '_, G> {
    pub(super) fn execute_font_definition(
        &mut self,
        mut request: FontLoadRequest,
        mut global: bool,
    ) -> Result<ReplayStep, ExecError> {
        self.report_scan()?;
        let font_resource =
            self.control
                .resolve_font_request(&mut request, self.stores, self.provider)?;
        self.control.execution.main_loop_active = false;

        let step = self.control.apply_direct_leaf(
            self.stores,
            self.effects,
            self.provider,
            self.output_start,
            None,
            |stores, _, _, _, command, _, _, _, _, _, _, _| {
                semantic::execute_font_definition(
                    stores,
                    command,
                    font_resource.as_ref(),
                    &mut request,
                    &mut global,
                )
            },
        )?;
        self.schedule_assignment_token()?;
        Ok(step)
    }
}
