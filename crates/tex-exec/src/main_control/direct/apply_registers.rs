//! Apply scanned registers values directly.
use super::*;
impl<G> DirectDispatch<'_, '_, G> {
    pub(super) fn execute_skip(
        &mut self,
        mut index: u16,
        mut value: GlueSpec,
        source_identity: Option<tex_state::GlueId<G>>,
        source_register: Option<(bool, u16)>,
        mut global: bool,
    ) -> Result<ReplayStep, ExecError> {
        self.report_scan()?;
        let context = self.stores.command_context().expect("glue assignment");
        let mut reassigning = MainControl::direct_glue_pointer_reassigned(
            &context,
            false,
            index,
            value,
            source_identity,
            source_register,
            global,
            &self.control.skip_pointer_sources,
            &self.control.muskip_pointer_sources,
        );
        let mut redundant = context.int_param(IntParam::ETEX_EXTENDED_MODE) > 0 && reassigning;
        drop(context);
        self.control.main_loop_active = false;

        let step = self.control.apply_direct_leaf(
            self.stores,
            self.effects,
            self.provider,
            self.output_start,
            None,
            |stores, _, _, _, command, _, _, _, _, _, _, _| {
                semantic::execute_skip(
                    stores,
                    command,
                    &mut index,
                    &mut value,
                    &mut redundant,
                    &mut reassigning,
                    &mut global,
                )
            },
        )?;
        if !redundant {
            self.record_glue_source(false, index, source_identity);
        }
        self.schedule_assignment_token()?;
        Ok(step)
    }

    pub(super) fn execute_muskip(
        &mut self,
        mut index: u16,
        mut value: GlueSpec,
        source_identity: Option<tex_state::GlueId<G>>,
        source_register: Option<(bool, u16)>,
        mut global: bool,
    ) -> Result<ReplayStep, ExecError> {
        self.report_scan()?;
        let context = self.stores.command_context().expect("glue assignment");
        let mut reassigning = MainControl::direct_glue_pointer_reassigned(
            &context,
            true,
            index,
            value,
            source_identity,
            source_register,
            global,
            &self.control.skip_pointer_sources,
            &self.control.muskip_pointer_sources,
        );
        let mut redundant = context.int_param(IntParam::ETEX_EXTENDED_MODE) > 0 && reassigning;
        drop(context);
        self.control.main_loop_active = false;

        let step = self.control.apply_direct_leaf(
            self.stores,
            self.effects,
            self.provider,
            self.output_start,
            None,
            |stores, _, _, _, command, _, _, _, _, _, _, _| {
                semantic::execute_muskip(
                    stores,
                    command,
                    &mut index,
                    &mut value,
                    &mut redundant,
                    &mut reassigning,
                    &mut global,
                )
            },
        )?;
        if !redundant {
            self.record_glue_source(true, index, source_identity);
        }
        self.schedule_assignment_token()?;
        Ok(step)
    }

    pub(super) fn execute_int_param(
        &mut self,
        mut index: u16,
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
                semantic::execute_int_param(stores, command, &mut index, &mut value, &mut global)
            },
        )?;
        self.schedule_assignment_token()?;
        Ok(step)
    }

    pub(super) fn execute_dimen_param(
        &mut self,
        mut index: u16,
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
                semantic::execute_dimen_param(stores, command, &mut index, &mut value, &mut global)
            },
        )?;
        self.schedule_assignment_token()?;
        Ok(step)
    }

    pub(super) fn execute_glue_param(
        &mut self,
        mut index: u16,
        mut value: GlueSpec,
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
                semantic::execute_glue_param(stores, command, &mut index, &mut value, &mut global)
            },
        )?;
        self.schedule_assignment_token()?;
        Ok(step)
    }

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
