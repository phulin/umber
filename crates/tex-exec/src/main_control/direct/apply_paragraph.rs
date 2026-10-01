//! Apply scanned paragraph values directly.
use super::*;
impl<G> DirectDispatch<'_, '_, G> {
    pub(super) fn execute_paragraph_indent(
        &mut self,
        mut indent: bool,
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
                semantic::execute_paragraph_indent(stores, modes, command, &mut indent)
            },
        )
    }

    pub(super) fn execute_set_language(
        &mut self,
        mut language: i32,
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
                semantic::execute_set_language(stores, modes, command, &mut language)
            },
        )
    }

    pub(super) fn execute_mark(
        &mut self,
        mut class: u16,
        tokens: tex_command::AttemptTokenListId,
    ) -> Result<ReplayStep, ExecError> {
        self.report_scan()?;
        let mut tokens =
            publication::publish_tokens(self.control.command.state(), self.stores, tokens)?;
        self.control.main_loop_active = false;

        self.control.apply_direct_leaf(
            self.stores,
            self.effects,
            self.provider,
            self.output_start,
            None,
            |stores, modes, _, _, command, boxes, _, _, _, _, _, _| {
                semantic::execute_mark(stores, modes, command, boxes, &mut class, &mut tokens)
            },
        )
    }

    pub(super) fn execute_paragraph(
        &mut self,
        mut current_line: i32,
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
                semantic::execute_paragraph(stores, modes, command, &mut current_line)
            },
        )
    }

    pub(super) fn execute_paragraph_start(&mut self) -> Result<ReplayStep, ExecError> {
        self.report_scan()?;

        self.control.main_loop_active = false;

        self.control.apply_direct_leaf(
            self.stores,
            self.effects,
            self.provider,
            self.output_start,
            None,
            |stores, modes, _, _, command, _, _, _, _, _, _, _| {
                semantic::execute_paragraph_start(stores, modes, command)
            },
        )
    }
}
