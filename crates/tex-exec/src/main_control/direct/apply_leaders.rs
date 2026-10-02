//! Apply scanned leaders values directly.
use super::*;
impl<G> DirectDispatch<'_, '_, G> {
    pub(super) fn execute_h_rule_here_except_leaders(&mut self) -> Result<ReplayStep, ExecError> {
        self.report_scan()?;

        self.control.execution.main_loop_active = false;

        self.control.apply_direct_leaf(
            self.stores,
            self.effects,
            self.provider,
            self.output_start,
            None,
            |stores, _, _, _, command, _, _, _, _, _, _, _| {
                semantic::execute_h_rule_here_except_leaders(stores, command)
            },
        )
    }

    pub(super) fn execute_leaders(
        &mut self,
        mut kind: GlueKind,
        mut payload: LeaderPayload,
        mut glue: GlueSpec,
        mut shared_zero_glue: bool,
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
                semantic::execute_leaders(
                    stores,
                    modes,
                    command,
                    boxes,
                    &mut kind,
                    &mut payload,
                    &mut glue,
                    &mut shared_zero_glue,
                )
            },
        )
    }

    pub(super) fn execute_leader_register(
        &mut self,
        mut kind: GlueKind,
        mut index: u16,
        mut copy: bool,
        mut glue: GlueSpec,
        mut shared_zero_glue: bool,
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
                semantic::execute_leader_register(
                    stores,
                    modes,
                    command,
                    &mut kind,
                    &mut index,
                    &mut copy,
                    &mut glue,
                    &mut shared_zero_glue,
                )
            },
        )
    }

    pub(super) fn execute_missing_leader_payload(&mut self) -> Result<ReplayStep, ExecError> {
        self.report_scan()?;

        self.control.execution.main_loop_active = false;

        self.control.apply_direct_leaf(
            self.stores,
            self.effects,
            self.provider,
            self.output_start,
            None,
            |stores, _, _, _, command, _, _, _, _, _, _, _| {
                semantic::execute_missing_leader_payload(stores, command)
            },
        )
    }

    pub(super) fn execute_leaders_not_followed_by_glue(&mut self) -> Result<ReplayStep, ExecError> {
        self.report_scan()?;

        self.control.execution.main_loop_active = false;

        self.control.apply_direct_leaf(
            self.stores,
            self.effects,
            self.provider,
            self.output_start,
            None,
            |stores, _, _, _, command, boxes, _, _, _, _, _, _| {
                semantic::execute_leaders_not_followed_by_glue(stores, command, boxes)
            },
        )
    }

    pub(super) fn execute_begin_leader_box(
        &mut self,
        mut construction: ScannedBoxConstruction,
        mut kind: GlueKind,
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
                semantic::execute_begin_leader_box(
                    stores,
                    modes,
                    command,
                    boxes,
                    &mut construction,
                    &mut kind,
                )
            },
        )
    }
}
