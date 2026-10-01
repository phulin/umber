//! Direct alignment recovery semantics on native owners.
use super::super::*;

pub(in crate::main_control) fn execute_misplaced_alignment_delimiter<G>(
    stores: &mut tex_state::CommandContext<'_, G>,
    command: &mut CommandMachine<'_, '_, G>,
    token: &mut Token,
    context: &mut tex_command::DiagnosticContextCoordinate,
) -> Result<ReplayStep, ExecError> {
    let mut linear_stores = LinearCommandContext::new(stores);
    let stores = &mut linear_stores;
    {
        let context = render_diagnostic_coordinate(command, stores, *context)?;
        crate::diagnostics::report_misplaced_alignment_delimiter(
            stores,
            command.diagnostic_effects,
            *token,
            Some(context),
        )?;
        Ok(ReplayStep::Continue)
    }
}

pub(in crate::main_control) fn execute_misplaced_alignment_command<G>(
    stores: &mut tex_state::CommandContext<'_, G>,
    command: &mut CommandMachine<'_, '_, G>,
    omit: &mut bool,
) -> Result<ReplayStep, ExecError> {
    let mut linear_stores = LinearCommandContext::new(stores);
    let stores = &mut linear_stores;
    {
        let context = command.state.output_open_context(&**stores);
        let (name, help) = if *omit {
            (
                "omit",
                [
                    "I expect to see \\omit only after tab marks or the \\cr of",
                    "an alignment. Proceed, and I'll ignore this case.",
                ],
            )
        } else {
            (
                "noalign",
                [
                    "I expect to see \\noalign only after the \\cr of",
                    "an alignment. Proceed, and I'll ignore this case.",
                ],
            )
        };
        crate::diagnostics::report_misplaced_alignment_command(
            stores,
            command.diagnostic_effects,
            name,
            &help,
            Some(context),
        )?;
        Ok(ReplayStep::Continue)
    }
}

pub(in crate::main_control) fn execute_missing_alignment_cr<G>(
    stores: &mut tex_state::CommandContext<'_, G>,
    command: &mut CommandMachine<'_, '_, G>,
) -> Result<ReplayStep, ExecError> {
    let mut linear_stores = LinearCommandContext::new(stores);
    let stores = &mut linear_stores;
    {
        // TeX82 §1132 reaches §82's `ins_error` only after §325 has
        // backed up the brace and frozen `\cr` has been inserted. The
        // open context therefore labels the repair `<inserted text>` and
        // leaves the original brace below it as `<to be read again>`.
        let context = command.state.output_open_context(&**stores);
        crate::error_report::report_error(
            stores,
            command.diagnostic_effects,
            "Missing \\cr inserted",
            &["I'm guessing that you meant to end an alignment here."],
            context,
        )?;
        Ok(ReplayStep::Continue)
    }
}

pub(in crate::main_control) fn execute_alignment_recovery<G>(
    stores: &mut tex_state::CommandContext<'_, G>,
    command: &mut CommandMachine<'_, '_, G>,
    boxes: &mut ReplayBoxes<G>,
    brace: &mut Catcode,
) -> Result<ReplayStep, ExecError> {
    let mut linear_stores = LinearCommandContext::new(stores);
    let stores = &mut linear_stores;
    {
        let (message, opens_simple_group) = match brace {
            Catcode::BeginGroup => ("Missing { inserted", true),
            Catcode::EndGroup => ("Missing } inserted", false),
            _ => {
                return Err(ExecError::MissingToken {
                    context: "align_error balancing brace",
                });
            }
        };
        let context = command.state.output_open_context(&**stores);
        crate::error_report::report_error(
            stores,
            command.diagnostic_effects,
            message,
            &[
                "I've put in what seems to be necessary to fix",
                "the current column of the current alignment.",
                "Try to go on, since this might almost work.",
            ],
            context,
        )?;
        boxes.recovery_simple_group_pending = opens_simple_group;
        Ok(ReplayStep::Continue)
    }
}

pub(in crate::main_control) fn execute_illegal_h_align<G>(
    stores: &mut tex_state::CommandContext<'_, G>,
    modes: &mut ModeNest,
    command: &mut CommandMachine<'_, '_, G>,
    token: &mut Token,
) -> Result<ReplayStep, ExecError> {
    let mut linear_stores = LinearCommandContext::new(stores);
    let stores = &mut linear_stores;
    {
        let context = command.state.output_open_context(&**stores);
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
