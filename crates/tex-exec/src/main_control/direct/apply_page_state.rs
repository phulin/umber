//! Apply scanned page state values directly.
use super::*;
impl<G> DirectDispatch<'_, '_, G> {
    pub(super) fn execute_continue(&mut self) -> Result<ReplayStep, ExecError> {
        self.report_scan()?;

        self.control.main_loop_active = false;

        self.control.apply_direct_leaf(
            self.stores,
            self.effects,
            self.provider,
            self.output_start,
            None,
            |_, _, _, _, _, _, _, _, _, _, _, _| semantic::execute_continue(),
        )
    }

    pub(super) fn execute_end_of_input(&mut self) -> Result<ReplayStep, ExecError> {
        self.report_scan()?;

        self.control.main_loop_active = false;

        self.control.apply_direct_leaf(
            self.stores,
            self.effects,
            self.provider,
            self.output_start,
            None,
            |_, _, _, _, _, _, _, _, _, _, _, _| semantic::execute_end_of_input(),
        )
    }

    pub(super) fn execute_stop(
        &mut self,
        command: tex_command::CurrentCommand<G>,
        dump: bool,
    ) -> Result<ReplayStep, ExecError> {
        let mut context = self
            .stores
            .command_context()
            .expect("stop command admission");
        // §1054 reads page/contribution/dead-cycle state only for the actual
        // outer-vertical stop; unrelated commands need none of these owners.
        let all_over = crate::page_output::job_is_all_over(&context);
        let mut facts = CommandMachineHostFacts::Forbidden;
        let mut processor = command_processor(
            &mut self.control.command,
            self.control.fuel.fuel_mut(),
            &mut self.control.capabilities,
            &mut facts as &mut dyn tex_command::CommandHostFacts<G>,
            &mut self.control.operation_observations,
            self.effects,
            &mut context,
        );
        if let Some(cursor) = self.cursor {
            processor.resume_delivery_cursor(cursor);
        }
        let incomplete_conditions = if all_over {
            processor.final_cleanup()
        } else {
            processor
                .back_input(command)
                .map_err(command_error)
                .map_err(|error| scan_error(error, self.current_origin))?;
            Vec::new()
        };
        self.cursor = Some(processor.delivery_cursor());
        self.diagnostics.extend(
            processor
                .take_semantic_diagnostics()
                .into_iter()
                .map(PendingDiagnostic::Command),
        );
        drop(processor);
        drop(context);
        if all_over {
            self.execute_end(dump, incomplete_conditions)
        } else {
            self.execute_eject_residual_page()
        }
    }

    pub(super) fn execute_end(
        &mut self,
        mut dump: bool,
        incomplete_conditions: Vec<tex_command::IncompleteCondition>,
    ) -> Result<ReplayStep, ExecError> {
        self.report_scan()?;
        self.control.main_loop_active = false;
        let result = {
            let mut context = self
                .stores
                .command_context()
                .expect("end command admission");
            let mut command = self
                .control
                .command_machine_with_resource_provider(self.effects, self.provider.take());
            let result = semantic::execute_end(&mut context, &mut command, &mut dump);
            if result.is_ok() {
                command.publish_named_token_list_pushes(&mut context);
            }
            *self.provider = command.resource_provider.take();
            result
        };
        let step = result?;
        if self.control.initex && dump {
            let context = self
                .stores
                .command_context()
                .expect("format dump accounting");
            let receipt = crate::job::FormatDumpReceipt::new(
                self.control.capabilities.job_name().to_owned(),
                context.int_param(IntParam::YEAR),
                context.int_param(IntParam::MONTH),
                context.int_param(IntParam::DAY),
            );
            drop(context);
            self.stores
                .command_context()
                .expect("format string pool")
                .make_string_pool_string(&receipt.pool_string());
            self.control.dumped_format = Some(receipt);
        }
        if step == ReplayStep::End {
            self.stores
                .world_mut()
                .publish_diagnostic_effects_preserving(self.effects);
            self.control
                .end_of_job_final_cleanup(self.stores, dump, incomplete_conditions);
        }
        let pending = PendingPageOutputFacts::capture(
            &self.stores.command_context().expect("end page output"),
        );
        self.control
            .fire_pending_page_output(self.stores, self.effects, pending)?;
        let mut records = Vec::new();
        records.extend(
            committed_stream_effect_observations(
                self.output_start.effect_count,
                self.output_start.prepared_page_count,
                self.stores,
                &self.control.prepared_dvi_pages,
            )
            .into_iter()
            .map(CommandObservation::Effect),
        );
        records.push(CommandObservation::Effect(engine_termination_effect()));
        records.extend(
            committed_shipout_observations(self.output_start.artifact_count, self.stores)
                .into_iter()
                .map(CommandObservation::Effect),
        );
        if let Some(mut observations) = self.control.page_output_observations.take() {
            observations.append_to(&mut records);
        }
        self.control.observe_committed(records);
        self.control
            .finish_paragraph_boundary(self.output_start.outer_paragraph_was_active, self.stores);
        Ok(step)
    }

    pub(super) fn execute_eject_residual_page(&mut self) -> Result<ReplayStep, ExecError> {
        self.report_scan()?;

        self.control.main_loop_active = false;

        self.control.apply_direct_leaf(
            self.stores,
            self.effects,
            self.provider,
            self.output_start,
            None,
            |stores, _, _, _, command, _, _, _, _, _, _, _| {
                semantic::execute_eject_residual_page(stores, command)
            },
        )
    }

    pub(super) fn execute_prev_depth(
        &mut self,
        mut value: Scaled,
    ) -> Result<ReplayStep, ExecError> {
        self.report_scan()?;

        self.control.main_loop_active = false;

        let step = self.control.apply_direct_leaf(
            self.stores,
            self.effects,
            self.provider,
            self.output_start,
            None,
            |stores, modes, _, _, command, _, _, _, _, _, _, _| {
                semantic::execute_prev_depth(stores, modes, command, &mut value)
            },
        )?;
        self.schedule_assignment_token()?;
        Ok(step)
    }

    pub(super) fn execute_space_factor(&mut self, mut value: i32) -> Result<ReplayStep, ExecError> {
        self.report_scan()?;

        self.control.main_loop_active = false;
        if !(1..=32767).contains(&value) {
            self.stores
                .world_mut()
                .publish_diagnostic_effects_preserving(self.effects);
        }

        let step = self.control.apply_direct_leaf(
            self.stores,
            self.effects,
            self.provider,
            self.output_start,
            None,
            |stores, modes, _, _, command, _, _, _, _, _, _, _| {
                semantic::execute_space_factor(stores, modes, command, &mut value)
            },
        )?;
        self.schedule_assignment_token()?;
        Ok(step)
    }

    pub(super) fn execute_prev_graf(&mut self, mut value: i32) -> Result<ReplayStep, ExecError> {
        self.report_scan()?;

        self.control.main_loop_active = false;
        if value < 0 {
            self.stores
                .world_mut()
                .publish_diagnostic_effects_preserving(self.effects);
        }

        let step = self.control.apply_direct_leaf(
            self.stores,
            self.effects,
            self.provider,
            self.output_start,
            None,
            |stores, modes, _, _, command, _, _, _, _, _, _, _| {
                semantic::execute_prev_graf(stores, modes, command, &mut value)
            },
        )?;
        self.schedule_assignment_token()?;
        Ok(step)
    }

    pub(super) fn execute_page_dimension(
        &mut self,
        mut dimension: PageDimension,
        mut value: Scaled,
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
                semantic::execute_page_dimension(stores, command, &mut dimension, &mut value)
            },
        )?;
        self.schedule_assignment_token()?;
        Ok(step)
    }

    pub(super) fn execute_page_integer(
        &mut self,
        mut integer: PageInteger,
        mut value: i32,
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
                semantic::execute_page_integer(stores, command, &mut integer, &mut value)
            },
        )?;
        self.schedule_assignment_token()?;
        Ok(step)
    }
}
