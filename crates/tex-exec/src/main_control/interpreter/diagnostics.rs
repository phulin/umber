//! Demanded displays complete inside the same processor and error dialogue.
use super::*;

fn finish<G>(
    processor: &mut Processor<'_, '_, G>,
    long: bool,
) -> Result<MainControlStep, ExecError> {
    processor.with_execution_owners(|context, command, effects, _, _, _, _, _, _| {
        let display = command.output_open_context(context);
        context.publish_diagnostic_effects_before_synchronous_print(effects);
        crate::diagnostics::complete_show(context, effects, long, Some(display))
    })?;
    recovery::finish_error_dialogue(processor)?;
    Ok(MainControlStep::Continue)
}

pub(super) fn show<G>(
    processor: &mut Processor<'_, '_, G>,
    internal: bool,
) -> Result<MainControlStep, ExecError> {
    let diagnostic = if internal {
        processor.scan_showthe()
    } else {
        processor.scan_show()
    }
    .map_err(command_error)?;
    report_scan(processor)?;
    processor.with_execution_owners(|context, _, effects, _, _, execution, _, _, _| {
        execution.main_loop_active = false;
        print_display_content(context, effects, &diagnostic.content);
    });
    finish(processor, false)
}

pub(super) fn tokens<G>(
    processor: &mut Processor<'_, '_, G>,
) -> Result<MainControlStep, ExecError> {
    let tokens = processor.scan_showtokens().map_err(command_error)?.tokens;
    report_scan(processor)?;
    processor.with_execution_owners(|context, command, effects, _, _, execution, _, _, _| {
        execution.main_loop_active = false;
        let newlinechar = u32::try_from(context.int_param(IntParam::NEWLINE_CHAR))
            .ok()
            .filter(|&code| code <= u8::MAX.into())
            .and_then(char::from_u32);
        let mut text = String::new();
        for word in command
            .attempt_token_words(&tokens)
            .map_err(|_| ExecError::MissingToken {
                context: "showtokens scanner owner",
            })?
        {
            tex_state::token_show::append_token_selector_text(
                context,
                word.semantic_token(),
                newlinechar,
                &mut text,
            );
        }
        command
            .discard_attempt_tokens(tokens)
            .expect("consumed showtokens owner");
        let mut diagnostic = context.begin_online_diagnostic(effects);
        diagnostic.print_nl("> ").print_rendered(&text);
        diagnostic.end_open();
        Ok::<_, ExecError>(())
    })?;
    finish(processor, false)
}

pub(super) fn lists<G>(processor: &mut Processor<'_, '_, G>) -> Result<MainControlStep, ExecError> {
    report_scan(processor)?;
    processor.with_execution_owners(|context, command, effects, fuel, _, execution, _, _, _| {
        execution.main_loop_active = false;
        crate::box_runtime::flush_pending_hchars(&mut execution.modes, context, effects, fuel)?;
        crate::diagnostics::execute_showlists(context, effects, &execution.modes, command.profile())
    })?;
    finish(processor, true)
}

pub(super) fn conditions<G>(
    processor: &mut Processor<'_, '_, G>,
) -> Result<MainControlStep, ExecError> {
    let conditions = processor.active_conditions();
    report_scan(processor)?;
    processor.with_execution_owners(|context, _, effects, _, _, execution, _, _, _| {
        execution.main_loop_active = false;
        let mut diagnostic = context.begin_diagnostic(effects);
        diagnostic.print_nl("").print_ln();
        diagnostic.print_rendered(&render_showifs(&conditions));
        diagnostic.end(true);
    });
    finish(processor, true)
}

pub(super) fn groups<G>(
    processor: &mut Processor<'_, '_, G>,
) -> Result<MainControlStep, ExecError> {
    report_scan(processor)?;
    processor.with_execution_owners(|context, _, effects, _, _, execution, _, _, _| {
        execution.main_loop_active = false;
        let diagnostic = detached_showgroups(
            context,
            &execution.active_alignment,
            &execution.boxes,
            &execution.active_discretionaries,
            &execution.active_math_choices,
            &execution.active_math_left_boundaries,
            &execution.active_math_shifts,
        );
        crate::diagnostics::execute_showgroups(context, effects, &diagnostic);
    });
    finish(processor, true)
}

pub(super) fn box_register<G>(
    processor: &mut Processor<'_, '_, G>,
) -> Result<MainControlStep, ExecError> {
    let index = processor
        .scan_profile_register_index()
        .map_err(command_error)?;
    report_scan(processor)?;
    processor.with_execution_owners(|context, command, effects, _, _, execution, _, _, _| {
        execution.main_loop_active = false;
        crate::diagnostics::execute_showbox(context, effects, index, command.profile());
    });
    finish(processor, true)
}
