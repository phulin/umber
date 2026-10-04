//! Direct recovery semantics on native owners.
use super::super::*;

pub(in crate::main_control) fn execute_illegal_stop<G>(
    stores: &mut tex_state::CommandContext<'_, G>,
    modes: &mut ModeNest,
    command: &mut CommandMachine<'_, '_, G>,
    token: &mut Token,
) -> Result<MainControlStep, ExecError> {
    {
        // TeX82 §1051's `privileged`: `\end`/`\dump` below outer
        // vertical mode reports and is discarded, exactly like the other
        // Forbidden cases.
        let context = command.state.output_open_context(&*stores);
        crate::diagnostics::report_illegal_case_with_context(
            stores,
            command.diagnostic_effects,
            *token,
            modes.current_mode(),
            Some(context),
        )?;
        Ok(MainControlStep::Continue)
    }
}

pub(in crate::main_control) fn execute_illegal_macro_parameter<G>(
    stores: &mut tex_state::CommandContext<'_, G>,
    modes: &mut ModeNest,
    command: &mut CommandMachine<'_, '_, G>,
    token: &mut Token,
) -> Result<MainControlStep, ExecError> {
    {
        let context = command.state.output_open_context(&*stores);
        crate::diagnostics::report_illegal_case_with_context(
            stores,
            command.diagnostic_effects,
            *token,
            modes.current_mode(),
            Some(context),
        )?;
        Ok(MainControlStep::Continue)
    }
}

pub(in crate::main_control) fn execute_extra_end_cs_name<G>(
    stores: &mut tex_state::CommandContext<'_, G>,
    command: &mut CommandMachine<'_, '_, G>,
) -> Result<MainControlStep, ExecError> {
    {
        // TeX82 §1135's `cs_error`.
        let context = command.state.output_open_context(&*stores);
        report_escaped_error(
            stores,
            command.diagnostic_effects,
            "Extra ",
            "endcsname",
            "",
            &["I'm ignoring this, since I wasn't doing a \\csname."],
            context,
        )?;
        Ok(MainControlStep::Continue)
    }
}

pub(in crate::main_control) fn execute_illegal_italic_correction<G>(
    stores: &mut tex_state::CommandContext<'_, G>,
    modes: &mut ModeNest,
    command: &mut CommandMachine<'_, '_, G>,
    token: &mut Token,
) -> Result<MainControlStep, ExecError> {
    {
        let context = command.state.output_open_context(&*stores);
        crate::diagnostics::report_illegal_case_with_context(
            stores,
            command.diagnostic_effects,
            *token,
            modes.current_mode(),
            Some(context),
        )?;
        Ok(MainControlStep::Continue)
    }
}

pub(in crate::main_control) fn execute_illegal_prev_depth<G>(
    stores: &mut tex_state::CommandContext<'_, G>,
    modes: &mut ModeNest,
    command: &mut CommandMachine<'_, '_, G>,
    token: &mut Token,
) -> Result<MainControlStep, ExecError> {
    {
        let context = command.state.output_open_context(&*stores);
        crate::diagnostics::report_illegal_case_with_context(
            stores,
            command.diagnostic_effects,
            *token,
            modes.current_mode(),
            Some(context),
        )?;
        Ok(MainControlStep::Continue)
    }
}

pub(in crate::main_control) fn execute_illegal_space_factor<G>(
    stores: &mut tex_state::CommandContext<'_, G>,
    modes: &mut ModeNest,
    command: &mut CommandMachine<'_, '_, G>,
    token: &mut Token,
) -> Result<MainControlStep, ExecError> {
    {
        let context = command.state.output_open_context(&*stores);
        crate::diagnostics::report_illegal_case_with_context(
            stores,
            command.diagnostic_effects,
            *token,
            modes.current_mode(),
            Some(context),
        )?;
        Ok(MainControlStep::Continue)
    }
}

pub(in crate::main_control) fn execute_illegal_set_language<G>(
    stores: &mut tex_state::CommandContext<'_, G>,
    modes: &mut ModeNest,
    command: &mut CommandMachine<'_, '_, G>,
    token: &mut Token,
) -> Result<MainControlStep, ExecError> {
    {
        let context = command.state.output_open_context(&*stores);
        crate::diagnostics::report_illegal_case_with_context(
            stores,
            command.diagnostic_effects,
            *token,
            modes.current_mode(),
            Some(context),
        )?;
        Ok(MainControlStep::Continue)
    }
}

pub(in crate::main_control) fn execute_illegal_last_item<G>(
    stores: &mut tex_state::CommandContext<'_, G>,
    modes: &mut ModeNest,
    command: &mut CommandMachine<'_, '_, G>,
    token: &mut Token,
    context: &mut String,
    site: &mut tex_state::diagnostic::DiagnosticSite,
) -> Result<MainControlStep, ExecError> {
    {
        crate::diagnostics::report_illegal_case_with_site(
            stores,
            command.diagnostic_effects,
            *token,
            modes.current_mode(),
            Some(std::mem::take(context)),
            Some(site.clone()),
        )?;
        Ok(MainControlStep::Continue)
    }
}
