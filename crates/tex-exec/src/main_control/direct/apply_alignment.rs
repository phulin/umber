//! Apply scanned alignment values directly.
use super::*;
impl<G> DirectDispatch<'_, '_, G> {
    pub(super) fn execute_align_peek_restart(
        &mut self,
        mut alignment: AlignmentIdentity,
    ) -> Result<ReplayStep, ExecError> {
        self.report_scan()?;

        self.control.main_loop_active = false;

        self.control.apply_direct_leaf(
            self.stores,
            self.effects,
            self.provider,
            self.output_start,
            None,
            |_, _, _, active_alignment, _, _, _, _, _, _, _, _| {
                semantic::execute_align_peek_restart(active_alignment, &mut alignment)
            },
        )
    }

    pub(super) fn execute_alignment_template_entered(&mut self) -> Result<ReplayStep, ExecError> {
        self.report_scan()?;

        self.control.apply_direct_leaf(
            self.stores,
            self.effects,
            self.provider,
            self.output_start,
            None,
            |_, _, _, active_alignment, _, _, _, _, _, _, _, _| {
                semantic::execute_alignment_template_entered(active_alignment)
            },
        )
    }

    pub(super) fn execute_begin_alignment(
        &mut self,
        mut vertical: bool,
        mut owner: Option<tex_state::interner::Symbol>,
    ) -> Result<ReplayStep, ExecError> {
        self.report_scan()?;

        self.control.main_loop_active = false;
        let suspended_alignment = self.control.active_alignment.is_some();

        self.control.apply_direct_leaf(
            self.stores,
            self.effects,
            self.provider,
            self.output_start,
            None,
            |stores,
             modes,
             next_alignment_identity,
             active_alignment,
             command,
             boxes,
             _,
             _,
             _,
             _,
             _,
             _| {
                let result = semantic::execute_begin_alignment(
                    stores,
                    modes,
                    next_alignment_identity,
                    active_alignment,
                    command,
                    boxes,
                    &mut vertical,
                    &mut owner,
                );
                if result.is_ok() {
                    command.publish_named_token_list_pushes(stores);
                    if suspended_alignment
                        && let Some(record) = command.state.alignment_suspend_observation()
                    {
                        command.retain_hot_observation(CommandObservation::Alignment(record));
                    }
                    if let Some(record) = command.state.alignment_begin_observation() {
                        command.retain_hot_observation(CommandObservation::Alignment(record));
                    }
                }
                result
            },
        )
    }

    pub(super) fn execute_alignment_preamble_opening(
        &mut self,
        mut alignment: AlignmentIdentity,
        mut packing: ScannedPackingSpec,
    ) -> Result<ReplayStep, ExecError> {
        self.report_scan()?;

        self.control.main_loop_active = false;

        self.control.apply_direct_leaf(
            self.stores,
            self.effects,
            self.provider,
            self.output_start,
            None,
            |stores, _, _, active_alignment, command, _, _, _, _, _, _, _| {
                semantic::execute_alignment_preamble_opening(
                    stores,
                    active_alignment,
                    command,
                    &mut alignment,
                    &mut packing,
                )
            },
        )
    }

    pub(super) fn execute_alignment_preamble_start(
        &mut self,
        mut alignment: AlignmentIdentity,
    ) -> Result<ReplayStep, ExecError> {
        self.report_scan()?;
        let preamble = publication::publish_alignment_preamble(
            self.control.command.state_mut(),
            self.stores,
            alignment,
        )?;
        {
            if preamble.columns.is_empty() {
                return Err(ExecError::MissingToken {
                    context: "first alignment preamble column",
                });
            }
            let active = self
                .control
                .active_alignment
                .as_mut()
                .filter(|active| active.identity == preamble.alignment)
                .ok_or(ExecError::MissingToken {
                    context: "active alignment preamble",
                })?;
            active.columns = preamble.columns;
            active.tabskips = preamble.tabskips;
            active.default_tabskip = preamble.default_tabskip;
            active.repeat_start = preamble.repeat_start;
            active.column = 0;
            active.preamble_start_pending = false;
            active.align_peek_pending = true;
        }

        self.control.main_loop_active = false;

        self.control.apply_direct_leaf(
            self.stores,
            self.effects,
            self.provider,
            self.output_start,
            None,
            |stores, _, _, active_alignment, command, _, _, _, _, _, _, _| {
                let result = semantic::execute_alignment_preamble_start(
                    stores,
                    active_alignment,
                    command,
                    &mut alignment,
                );
                if result.is_ok() {
                    command.publish_named_token_list_pushes(stores);
                    if let Some(record) = command.state.alignment_cell_begin_observation() {
                        command.retain_hot_observation(CommandObservation::Alignment(record));
                    }
                }
                result
            },
        )
    }

    pub(super) fn execute_alignment_cell_opening(
        &mut self,
        mut alignment: AlignmentIdentity,
        mut opening: AlignmentCellOpening,
    ) -> Result<ReplayStep, ExecError> {
        self.report_scan()?;

        self.control.main_loop_active = false;

        self.control.apply_direct_leaf(
            self.stores,
            self.effects,
            self.provider,
            self.output_start,
            None,
            |stores, _, _, active_alignment, command, _, _, _, _, _, _, _| {
                let result = semantic::execute_alignment_cell_opening(
                    stores,
                    active_alignment,
                    command,
                    &mut alignment,
                    &mut opening,
                );
                if result.is_ok() {
                    command.publish_named_token_list_pushes(stores);
                    if opening == AlignmentCellOpening::Template {
                        if let Some(record) = command
                            .state
                            .alignment_u_template_push_observation(alignment)
                        {
                            command.retain_hot_observation(CommandObservation::Input(record));
                        }
                        if let Some(record) = command
                            .state
                            .alignment_u_template_push_alignment_observation(alignment)
                        {
                            command.retain_hot_observation(CommandObservation::Alignment(record));
                        }
                    }
                    if opening == AlignmentCellOpening::Omit
                        && let Some(record) =
                            command.state.alignment_omit_cell_observation(alignment)
                    {
                        command.retain_hot_observation(CommandObservation::Alignment(record));
                    }
                }
                result
            },
        )
    }

    pub(super) fn execute_alignment_cell_finish(
        &mut self,
        mut alignment: AlignmentIdentity,
    ) -> Result<ReplayStep, ExecError> {
        self.report_scan()?;

        self.control.main_loop_active = false;
        let finishing_alignment = self
            .control
            .command
            .alignment_cell_finish_observation(alignment);

        self.control.apply_direct_leaf(
            self.stores,
            self.effects,
            self.provider,
            self.output_start,
            None,
            |stores, modes, _, active_alignment, command, _, _, _, _, _, _, _| {
                let result = semantic::execute_alignment_cell_finish(
                    stores,
                    modes,
                    active_alignment,
                    command,
                    &mut alignment,
                );
                if result.is_ok() {
                    command.publish_named_token_list_pushes(stores);
                    if let Some(record) = command
                        .state
                        .take_alignment_extra_tab_recovery_observation()
                    {
                        command.retain_hot_observation(CommandObservation::Alignment(record));
                    }
                    if let Some(record) = finishing_alignment {
                        command.retain_hot_observation(CommandObservation::Alignment(record));
                    }
                }
                result
            },
        )
    }

    pub(super) fn execute_alignment_finish(
        &mut self,
        mut alignment: AlignmentIdentity,
        mut current_line: u32,
    ) -> Result<ReplayStep, ExecError> {
        self.report_scan()?;

        self.control.main_loop_active = false;
        let finishing_alignment = self.control.command.alignment_finish_observation(alignment);

        self.control.apply_direct_leaf(
            self.stores,
            self.effects,
            self.provider,
            self.output_start,
            None,
            |stores, modes, _, active_alignment, command, boxes, _, _, _, _, _, _| {
                let result = semantic::execute_alignment_finish(
                    stores,
                    modes,
                    active_alignment,
                    command,
                    boxes,
                    &mut alignment,
                    &mut current_line,
                );
                if result.is_ok() {
                    command.publish_named_token_list_pushes(stores);
                    if let Some(record) = finishing_alignment {
                        command.retain_hot_observation(CommandObservation::Alignment(record));
                    }
                    if let Some(record) = command.state.alignment_resume_observation() {
                        command.retain_hot_observation(CommandObservation::Alignment(record));
                    }
                }
                result
            },
        )
    }

    pub(super) fn execute_begin_no_align(
        &mut self,
        mut alignment: AlignmentIdentity,
    ) -> Result<ReplayStep, ExecError> {
        self.report_scan()?;

        self.control.main_loop_active = false;

        self.control.apply_direct_leaf(
            self.stores,
            self.effects,
            self.provider,
            self.output_start,
            None,
            |stores, modes, _, active_alignment, command, _, _, _, _, _, _, _| {
                semantic::execute_begin_no_align(
                    stores,
                    modes,
                    active_alignment,
                    command,
                    &mut alignment,
                )
            },
        )
    }

    pub(super) fn execute_alignment_peek_cell(
        &mut self,
        mut alignment: AlignmentIdentity,
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
            |stores, modes, _, active_alignment, command, _, _, _, _, _, _, _| {
                let result = semantic::execute_alignment_peek_cell(
                    stores,
                    modes,
                    active_alignment,
                    command,
                    &mut alignment,
                    &mut omit,
                );
                if result.is_ok() {
                    command.publish_named_token_list_pushes(stores);
                    if !omit {
                        if let Some(record) = command
                            .state
                            .alignment_u_template_push_observation(alignment)
                        {
                            command.retain_hot_observation(CommandObservation::Input(record));
                        }
                        if let Some(record) = command
                            .state
                            .alignment_u_template_push_alignment_observation(alignment)
                        {
                            command.retain_hot_observation(CommandObservation::Alignment(record));
                        }
                    }
                    if omit
                        && let Some(record) =
                            command.state.alignment_omit_cell_observation(alignment)
                    {
                        command.retain_hot_observation(CommandObservation::Alignment(record));
                    }
                }
                result
            },
        )
    }

    pub(super) fn execute_no_align_end_group(
        &mut self,
        mut alignment: AlignmentIdentity,
    ) -> Result<ReplayStep, ExecError> {
        self.report_scan()?;

        self.control.main_loop_active = false;

        self.control.apply_direct_leaf(
            self.stores,
            self.effects,
            self.provider,
            self.output_start,
            None,
            |stores, modes, _, active_alignment, command, _, _, _, _, _, _, _| {
                semantic::execute_no_align_end_group(
                    stores,
                    modes,
                    active_alignment,
                    command,
                    &mut alignment,
                )
            },
        )
    }
}
