//! Direct recovery semantics on native owners.
use super::super::*;

pub(in crate::main_control) fn execute_illegal_stop<G>(
    stores: &mut tex_state::CommandContext<'_, G>,
    modes: &mut ModeNest,
    command: &mut CommandMachine<'_, '_, G>,
    token: &mut Token,
) -> Result<ReplayStep, ExecError> {
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
        Ok(ReplayStep::Continue)
    }
}

pub(in crate::main_control) fn execute_illegal_macro_parameter<G>(
    stores: &mut tex_state::CommandContext<'_, G>,
    modes: &mut ModeNest,
    command: &mut CommandMachine<'_, '_, G>,
    token: &mut Token,
) -> Result<ReplayStep, ExecError> {
    {
        let context = command.state.output_open_context(&*stores);
        crate::diagnostics::report_illegal_case_with_context(
            stores,
            command.diagnostic_effects,
            *token,
            modes.current_mode(),
            Some(context),
        )?;
        Ok(ReplayStep::Continue)
    }
}

pub(in crate::main_control) fn execute_extra_end_cs_name<G>(
    stores: &mut tex_state::CommandContext<'_, G>,
    command: &mut CommandMachine<'_, '_, G>,
) -> Result<ReplayStep, ExecError> {
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
        Ok(ReplayStep::Continue)
    }
}

pub(in crate::main_control) fn execute_illegal_italic_correction<G>(
    stores: &mut tex_state::CommandContext<'_, G>,
    modes: &mut ModeNest,
    command: &mut CommandMachine<'_, '_, G>,
    token: &mut Token,
) -> Result<ReplayStep, ExecError> {
    {
        let context = command.state.output_open_context(&*stores);
        crate::diagnostics::report_illegal_case_with_context(
            stores,
            command.diagnostic_effects,
            *token,
            modes.current_mode(),
            Some(context),
        )?;
        Ok(ReplayStep::Continue)
    }
}

pub(in crate::main_control) fn execute_illegal_prev_depth<G>(
    stores: &mut tex_state::CommandContext<'_, G>,
    modes: &mut ModeNest,
    command: &mut CommandMachine<'_, '_, G>,
    token: &mut Token,
) -> Result<ReplayStep, ExecError> {
    {
        let context = command.state.output_open_context(&*stores);
        crate::diagnostics::report_illegal_case_with_context(
            stores,
            command.diagnostic_effects,
            *token,
            modes.current_mode(),
            Some(context),
        )?;
        Ok(ReplayStep::Continue)
    }
}

pub(in crate::main_control) fn execute_illegal_space_factor<G>(
    stores: &mut tex_state::CommandContext<'_, G>,
    modes: &mut ModeNest,
    command: &mut CommandMachine<'_, '_, G>,
    token: &mut Token,
) -> Result<ReplayStep, ExecError> {
    {
        let context = command.state.output_open_context(&*stores);
        crate::diagnostics::report_illegal_case_with_context(
            stores,
            command.diagnostic_effects,
            *token,
            modes.current_mode(),
            Some(context),
        )?;
        Ok(ReplayStep::Continue)
    }
}

pub(in crate::main_control) fn execute_illegal_set_language<G>(
    stores: &mut tex_state::CommandContext<'_, G>,
    modes: &mut ModeNest,
    command: &mut CommandMachine<'_, '_, G>,
    token: &mut Token,
) -> Result<ReplayStep, ExecError> {
    {
        let context = command.state.output_open_context(&*stores);
        crate::diagnostics::report_illegal_case_with_context(
            stores,
            command.diagnostic_effects,
            *token,
            modes.current_mode(),
            Some(context),
        )?;
        Ok(ReplayStep::Continue)
    }
}

pub(in crate::main_control) fn execute_extra_right_brace<G>(
    stores: &mut tex_state::CommandContext<'_, G>,
    command: &mut CommandMachine<'_, '_, G>,
    forgotten: &mut Option<ForgottenGroupOpener>,
) -> Result<ReplayStep, ExecError> {
    match forgotten {
        None => {
            // TeX82 §1068's `bottom_level` arm of `handle_right_brace`.
            let context = command.state.output_open_context(&*stores);
            crate::error_report::report_error(
                stores,
                command.diagnostic_effects,
                "Too many }'s",
                &[
                    "You've closed more groups than you opened.",
                    "Such booboos are generally harmless, so keep going.",
                ],
                context,
            )?;
            Ok(ReplayStep::Continue)
        }
        Some(forgotten) => {
            // TeX82 §1069's `extra_right_brace` reports and discards the
            // mismatched brace. It does not `unsave` the group it names.
            let context = command.state.output_open_context(&*stores);
            let mut report = stores.print_err("Extra }, or forgotten ");
            forgotten.print(&mut report);
            report.help(&[
                "I've deleted a group-closing symbol because it seems to be",
                "spurious, as in `$x}$'. But perhaps the } is legitimate and",
                "you forgot something else, as in `\\hbox{$x}'. In such cases",
                "the way to recover is to insert both the forgotten and the",
                "deleted material, e.g., by typing `I$}'.",
            ]);
            report.context(context);
            report.error().defer_recovery(command.diagnostic_effects)?;
            Ok(ReplayStep::Continue)
        }
    }
}

pub(in crate::main_control) fn execute_off_save<G>(
    stores: &mut tex_state::CommandContext<'_, G>,
    command: &mut CommandMachine<'_, '_, G>,
    closer: &mut OffSaveCloser,
) -> Result<ReplayStep, ExecError> {
    {
        // `scan_off_save` already ran the input recovery (backing up the
        // command behind its chosen closer); this only prints TeX82
        // §1064's report naming what §1065 inserted.
        let context = command.state.output_open_context(&*stores);
        let mut report = stores.print_err("Missing ");
        closer.print(&mut report);
        report
            .print(" inserted")
            .help(&OFF_SAVE_HELP)
            .context(context);
        report.error().defer_recovery(command.diagnostic_effects)?;
        Ok(ReplayStep::Continue)
    }
}

pub(in crate::main_control) fn execute_off_save_bottom_drop<G>(
    stores: &mut tex_state::CommandContext<'_, G>,
    command: &mut CommandMachine<'_, '_, G>,
    token: &mut Token,
) -> Result<ReplayStep, ExecError> {
    {
        // TeX82 §1066: "print_err("Extra "); print_cmd_chr(cur_cmd,
        // cur_chr)". `scan_off_save` already dropped the command itself
        // (no backup, nothing to replay); this only names it.
        let name = tex_command::command_token_text(stores, *token);
        let context = command.state.output_open_context(&*stores);
        crate::error_report::report_error(
            stores,
            command.diagnostic_effects,
            &format!("Extra {name}"),
            &["Things are pretty mixed up, but I think the worst is over."],
            context,
        )?;
        Ok(ReplayStep::Continue)
    }
}

pub(in crate::main_control) fn execute_illegal_last_item<G>(
    stores: &mut tex_state::CommandContext<'_, G>,
    modes: &mut ModeNest,
    command: &mut CommandMachine<'_, '_, G>,
    token: &mut Token,
    context: &mut String,
    site: &mut tex_state::diagnostic::DiagnosticSite,
) -> Result<ReplayStep, ExecError> {
    {
        crate::diagnostics::report_illegal_case_with_site(
            stores,
            command.diagnostic_effects,
            *token,
            modes.current_mode(),
            Some(std::mem::take(context)),
            Some(site.clone()),
        )?;
        Ok(ReplayStep::Continue)
    }
}
