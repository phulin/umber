//! Alignment errors preserve the canonical native backup and brace correction.
use super::*;

pub(super) fn delimiter<G>(
    processor: &mut Processor<'_, '_, G>,
    spelling: TracedTokenWord,
    exceptional: Option<&tex_command::CurrentCommand<G>>,
) -> Result<MainControlStep, ExecError> {
    let inserted = processor
        .recover_align_error(spelling, exceptional)
        .map_err(command_error)?;
    report_scan(processor)?;
    processor.with_execution_owners(|context, command, effects, _, _, execution, _, _, _| {
        execution.main_loop_active = false;
        let display = command.output_open_context(context);
        if let Some(Token::Char { cat, .. }) = inserted {
            let opens = cat == Catcode::BeginGroup;
            crate::error_report::report_error(
                context,
                effects,
                if opens {
                    "Missing { inserted"
                } else {
                    "Missing } inserted"
                },
                &[
                    "I've put in what seems to be necessary to fix",
                    "the current column of the current alignment.",
                    "Try to go on, since this might almost work.",
                ],
                display,
            )?;
            execution.boxes.recovery_simple_group_pending = opens;
        } else {
            crate::diagnostics::report_misplaced_alignment_delimiter(
                context,
                effects,
                spelling.semantic_token(),
                Some(display),
            )?;
        }
        Ok::<_, ExecError>(())
    })?;
    recovery::finish_error_dialogue(processor)?;
    Ok(MainControlStep::Continue)
}

pub(super) fn misplaced<G>(
    processor: &mut Processor<'_, '_, G>,
    omit: bool,
) -> Result<MainControlStep, ExecError> {
    report_scan(processor)?;
    processor.with_execution_owners(|context, command, effects, _, _, execution, _, _, _| {
        execution.main_loop_active = false;
        let (name, help) = if omit {
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
            context,
            effects,
            name,
            &help,
            Some(command.output_open_context(context)),
        )
    })?;
    recovery::finish_error_dialogue(processor)?;
    Ok(MainControlStep::Continue)
}

pub(super) fn missing_cr<G>(
    processor: &mut Processor<'_, '_, G>,
    exceptional: Option<&tex_command::CurrentCommand<G>>,
) -> Result<MainControlStep, ExecError> {
    processor
        .recover_alignment_closing_brace(tex_command::AlignmentDeliveryEvent::ClosingBrace(
            *exceptional.expect("canonical alignment closing-brace event"),
        ))
        .map_err(command_error)?;
    report_scan(processor)?;
    processor.with_execution_owners(|context, command, effects, _, _, execution, _, _, _| {
        execution.main_loop_active = false;
        crate::error_report::report_error(
            context,
            effects,
            "Missing \\cr inserted",
            &["I'm guessing that you meant to end an alignment here."],
            command.output_open_context(context),
        )
    })?;
    recovery::finish_error_dialogue(processor)?;
    Ok(MainControlStep::Continue)
}
