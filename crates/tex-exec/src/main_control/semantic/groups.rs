//! Direct groups semantics on native owners.
use super::super::*;

pub(in crate::main_control) fn execute_after_group<G>(
    stores: &mut tex_state::CommandContext<'_, G>,
    command: &mut CommandMachine<'_, '_, G>,
    token: &mut tex_state::token::TracedTokenWord,
) -> Result<ReplayStep, ExecError> {
    let mut linear_stores = LinearCommandContext::new(stores);
    let stores = &mut linear_stores;
    {
        match command.state.state_mut().save_aftergroup(stores, *token) {
            // TeX82 §280's `save_for_after` consumes the operand at every
            // level, but creates an `insert_token` save word only when
            // `cur_level > level_one`. `NoOpenGroup` is therefore the
            // canonical outer-level no-op; every synchronization failure
            // remains an invariant violation.
            Ok(()) | Err(tex_command::CommandGroupError::NoOpenGroup) => {}
            Err(error) => {
                panic!("aftergroup is admitted only for the synchronized group stack: {error:?}")
            }
        }
        Ok(ReplayStep::Continue)
    }
}

pub(in crate::main_control) fn execute_after_assignment<G>(
    stores: &mut tex_state::CommandContext<'_, G>,
    command: &mut CommandMachine<'_, '_, G>,
    token: &mut tex_state::token::TracedTokenWord,
) -> Result<ReplayStep, ExecError> {
    let mut linear_stores = LinearCommandContext::new(stores);
    let stores = &mut linear_stores;
    {
        command
            .state
            .state_mut()
            .set_afterassignment(&**stores, *token)
            .expect("afterassignment uses the synchronized command generation");
        Ok(ReplayStep::Continue)
    }
}

pub(in crate::main_control) fn execute_begin_simple_group<G>(
    stores: &mut tex_state::CommandContext<'_, G>,
    command: &mut CommandMachine<'_, '_, G>,
    boxes: &mut ReplayBoxes<G>,
) -> Result<ReplayStep, ExecError> {
    let mut linear_stores = LinearCommandContext::new(stores);
    let stores = &mut linear_stores;
    {
        enter_group(
            stores,
            command.state,
            command.diagnostic_effects,
            GroupKind::Simple,
        );
        boxes.recovery_simple_group_pending = false;
        boxes.recovery_simple_group_open = true;
        Ok(ReplayStep::Continue)
    }
}

pub(in crate::main_control) fn execute_end_simple_group<G>(
    stores: &mut tex_state::CommandContext<'_, G>,
    command: &mut CommandMachine<'_, '_, G>,
    boxes: &mut ReplayBoxes<G>,
) -> Result<ReplayStep, ExecError> {
    let mut linear_stores = LinearCommandContext::new(stores);
    let stores = &mut linear_stores;
    {
        unsave_group(stores, command, GroupKind::Simple).map_err(|_| ExecError::MissingToken {
            context: "simple recovery group",
        })?;
        boxes.recovery_simple_group_open = false;
        Ok(ReplayStep::Continue)
    }
}

pub(in crate::main_control) fn execute_output_routine_opening_brace<G>(
    stores: &mut tex_state::CommandContext<'_, G>,
    modes: &mut ModeNest,
    command: &mut CommandMachine<'_, '_, G>,
    boxes: &mut ReplayBoxes<G>,
) -> Result<ReplayStep, ExecError> {
    let mut linear_stores = LinearCommandContext::new(stores);
    let stores = &mut linear_stores;
    {
        // TeX82 §1026 runs `normal_paragraph` after the output token list
        // and output save level have both been opened, as part of
        // `scan_left_brace`. Keep the reset at this consumed-opening-brace
        // boundary: doing it in the deferred page-fire tail runs before
        // command control has established the routine's body boundary.
        crate::paragraph_end::normal_paragraph(modes, stores, command.diagnostic_effects);
        boxes.output_routine_opening_pending = false;
        Ok(ReplayStep::Continue)
    }
}

pub(in crate::main_control) fn execute_end_output_routine<G>(
    stores: &mut tex_state::CommandContext<'_, G>,
    modes: &mut ModeNest,
    command: &mut CommandMachine<'_, '_, G>,
    boxes: &mut ReplayBoxes<G>,
) -> Result<ReplayStep, ExecError> {
    let mut linear_stores = LinearCommandContext::new(stores);
    let stores = &mut linear_stores;
    {
        let output_context = command.state.output_open_context(&**stores);
        let unbalanced = {
            let mut processor = command_processor(
                command.state,
                command.fuel,
                command.capabilities,
                &mut command.host_facts,
                command.observations,
                command.diagnostic_effects,
                stores,
            );
            let result = processor
                .finish_selected_output_routine()
                .map_err(command_error);
            processor.retire();
            result?
        };
        if unbalanced {
            crate::error_report::report_error(
                stores,
                command.diagnostic_effects,
                "Unbalanced output routine",
                &[
                    "Your sneaky output routine has problematic {'s and/or }'s.",
                    "I can't handle that very well; good luck.",
                ],
                output_context,
            )?;
        }
        // TeX82 §1026 has now semantically ended the output token list.
        // Section 1028's subsequent error therefore sees the source
        // level below every retained depleted `<output>` replay.
        let context = command.state.output_close_context(&**stores);
        // TeX82 §1026 retires the output token list, then runs §1096's
        // `end_graf` before it unsaves the output group. A non-null
        // paragraph left open by \output must be line-broken into this
        // internal vertical list; merely popping it discards the paragraph.
        // `end_paragraph` is the shared spelling of §1096: it ignores
        // non-horizontal modes and pops a null paragraph without a line.
        let diagnostic_context =
            command_diagnostic_context(command.state, command.output_routine_active);
        let mut geometry = pack_geometry_sink(command.state, command.observations);
        crate::paragraph_end::end_paragraph_with_fuel(
            modes,
            stores,
            command.diagnostic_effects,
            &mut geometry,
            diagnostic_context,
            command.fuel,
        )?;
        let mut output_level = crate::box_runtime::commit_current_list(
            modes,
            stores,
            command.diagnostic_effects,
            command.fuel,
        )?;
        unsave_group(stores, command, GroupKind::Output).map_err(|_| ExecError::MissingToken {
            context: "output routine group",
        })?;
        // TeX82 §1026 closes `output_group` with §282's `unsave`
        // before resuming `build_page`. `unsave` backs every saved
        // `insert_token` into input, including `\aftergroup` material.
        boxes.output_routine_active = false;
        crate::page_output::resume_page_builder_after_output(
            stores,
            command.diagnostic_effects,
            output_level.list_mutation().take_nodes(),
            crate::diagnostics::ExecutionDiagnosticContext::source_free(context),
        )?;
        Ok(ReplayStep::Continue)
    }
}
