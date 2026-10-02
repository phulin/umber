//! Apply scanned fonts values directly.
use super::*;
impl<G> DirectDispatch<'_, '_, G> {
    pub(super) fn execute_font_select(
        &mut self,
        mut font: FontId,
        mut _selector: Option<Symbol>,
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
                semantic::execute_font_select(stores, command, &mut font, &mut global)
            },
        )?;
        self.schedule_assignment_token()?;
        Ok(step)
    }

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

    pub(super) fn execute_generated_font_definition(
        &mut self,
        mut definition: ScannedGeneratedFontDefinition,
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
                semantic::execute_generated_font_definition(
                    stores,
                    command,
                    &mut definition,
                    &mut global,
                )
            },
        )?;
        self.schedule_assignment_token()?;
        Ok(step)
    }

    pub(super) fn execute_font_dimen(
        &mut self,
        mut font: FontId,
        mut number: i32,
        mut value: Scaled,
        mut recovery_context: Option<String>,
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
                semantic::execute_font_dimen(
                    stores,
                    command,
                    &mut font,
                    &mut number,
                    &mut value,
                    &mut recovery_context,
                )
            },
        )?;
        self.schedule_assignment_token()?;
        Ok(step)
    }

    pub(super) fn execute_font_integer(
        &mut self,
        mut font: FontId,
        mut skew: bool,
        mut value: i32,
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
                semantic::execute_font_integer(stores, command, &mut font, &mut skew, &mut value)
            },
        )?;
        self.schedule_assignment_token()?;
        Ok(step)
    }

    pub(super) fn execute_hyphenation_data(
        &mut self,
        mut words: Vec<Vec<char>>,
        mut pattern_specs: Vec<tex_state::hyphenation::PatternSpec>,
        mut patterns: bool,
        mut rejection_context: String,
        mut trie_built: bool,
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
                semantic::execute_hyphenation_data(
                    stores,
                    command,
                    &mut words,
                    &mut pattern_specs,
                    &mut patterns,
                    &mut rejection_context,
                    &mut trie_built,
                )
            },
        )?;
        self.schedule_assignment_token()?;
        Ok(step)
    }
}
