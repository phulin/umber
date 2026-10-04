//! Real input recovery, without constructing ordinary command products.
use super::*;
use tex_state::meaning::MeaningWord;

pub(super) fn illegal_case<G>(
    processor: &mut Processor<'_, '_, G>,
    spelling: TracedTokenWord,
    meaning: &MeaningWord<G>,
    control_sequence: Option<Symbol>,
    exceptional: Option<&tex_command::CurrentCommand<G>>,
) -> Result<MainControlStep, ExecError> {
    let mut site =
        processor.capture_consumed_site(spelling, meaning, control_sequence, exceptional);
    processor.with_execution_owners(|context, command, _, _, _, execution, _, _, _| {
        let frozen = crate::FrozenDiagnosticContext::capture(
            context,
            command.diagnostic_input_context(8),
            "command-recoverable",
        );
        site.context = Some(tex_state::diagnostic::DiagnosticContext {
            input_frame_count: frozen.input_frame_count,
            input_frame_tail: frozen.input_frame_tail,
            group_depth: frozen.group_depth,
            group_tail: frozen
                .group_tail
                .into_iter()
                .map(|group| tex_state::diagnostic::DiagnosticGroup {
                    kind: group.kind,
                    entered_line: group.entered_line,
                })
                .collect(),
        });
        if execution.first_causal_context.is_none() {
            execution.first_causal_context = Some(crate::FrozenDiagnosticContext::capture(
                context,
                command.diagnostic_input_context(8),
                "command-recoverable",
            ));
        }
    });
    processor.with_execution_owners(|context, command, effects, _, _, execution, _, _, _| {
        execution.main_loop_active = false;
        let display = command.output_open_context(context);
        crate::diagnostics::report_illegal_case_with_site(
            context,
            effects,
            spelling.semantic_token(),
            execution.modes.current_mode(),
            Some(display),
            Some(site),
        )
    })?;
    finish_error_dialogue(processor)?;
    Ok(MainControlStep::Continue)
}

/// ErrorStop input edits belong to the diagnostic that requested them, before
/// its handler scans another operand or the interpreter fetches another word.
pub(super) fn finish_error_dialogue<G>(
    processor: &mut Processor<'_, '_, G>,
) -> Result<(), ExecError> {
    let request = processor.with_execution_owners(|_, _, effects, _, _, _, _, _, _| {
        effects.take_error_stop_recovery()
    });
    if let Some(request) = request {
        processor
            .apply_error_stop_recovery(request)
            .map_err(command_error)?;
    }
    Ok(())
}

pub(super) fn off_save<G>(
    processor: &mut Processor<'_, '_, G>,
    spelling: TracedTokenWord,
    meaning: &MeaningWord<G>,
    control_sequence: Option<Symbol>,
    exceptional: Option<&tex_command::CurrentCommand<G>>,
) -> Result<MainControlStep, ExecError> {
    let site = processor.capture_consumed_site(spelling, meaning, control_sequence, exceptional);
    let kind = processor
        .with_execution_owners(|context, _, _, _, _, _, _, _, _| context.innermost_group_kind());
    let Some(kind) = kind else {
        processor.report_off_save_bottom_drop(spelling, exceptional);
        processor.with_execution_owners(
            |context, command, effects, _, _, execution, _, _, _| {
                execution.main_loop_active = false;
                let name = tex_command::command_token_text(context, spelling.semantic_token());
                let display = command.output_open_context(context);
                let mut report = context.print_err(&format!("Extra {name}"));
                report.help(&["Things are pretty mixed up, but I think the worst is over."]);
                report.context(display);
                report.error_and_defer_at(effects, Some(site))
            },
        )?;
        finish_error_dialogue(processor)?;
        return Ok(MainControlStep::Continue);
    };
    let (closer, tokens, count) = match kind {
        GroupKind::SemiSimple => (
            OffSaveCloser::EndGroup,
            [
                processor
                    .frozen_primitive_token("endgroup")
                    .map_err(command_error)?,
                Token::Char {
                    ch: '.',
                    cat: Catcode::Other,
                },
            ],
            1,
        ),
        GroupKind::MathShift => (
            OffSaveCloser::MathShift,
            [
                Token::Char {
                    ch: '$',
                    cat: Catcode::MathShift,
                },
                Token::Char {
                    ch: '.',
                    cat: Catcode::Other,
                },
            ],
            1,
        ),
        GroupKind::MathLeft => (
            OffSaveCloser::NullRight,
            [
                processor
                    .frozen_primitive_token("right")
                    .map_err(command_error)?,
                Token::Char {
                    ch: '.',
                    cat: Catcode::Other,
                },
            ],
            2,
        ),
        _ => (
            OffSaveCloser::RightBrace,
            [
                Token::Char {
                    ch: '}',
                    cat: Catcode::EndGroup,
                },
                Token::Char {
                    ch: '.',
                    cat: Catcode::Other,
                },
            ],
            1,
        ),
    };
    processor
        .recover_off_save(spelling, exceptional, &tokens[..count])
        .map_err(command_error)?;
    processor.with_execution_owners(|context, command, effects, _, _, execution, _, _, _| {
        execution.main_loop_active = false;
        let display = command.output_open_context(context);
        let mut report = context.print_err("Missing ");
        closer.print(&mut report);
        report
            .print(" inserted")
            .help(&OFF_SAVE_HELP)
            .context(display);
        report.error_and_defer_at(effects, Some(site))
    })?;
    finish_error_dialogue(processor)?;
    Ok(MainControlStep::Continue)
}

pub(super) fn start_paragraph_and_reswitch<G>(
    processor: &mut Processor<'_, '_, G>,
    spelling: TracedTokenWord,
    _meaning: &MeaningWord<G>,
    _control_sequence: Option<Symbol>,
    exceptional: Option<&tex_command::CurrentCommand<G>>,
) -> Result<MainControlStep, ExecError> {
    processor
        .back_consumed_word(spelling, exceptional)
        .map_err(command_error)?;
    report_scan(processor)?;
    processor.with_execution_owners(
        |context, command, effects, _, observer, execution, _, _, _| {
            execution.main_loop_active = false;
            start_paragraph(command, &mut execution.modes, context, effects, true)?;
            command.publish_named_token_list_pushes(context, effects, observer);
            Ok::<MainControlStep, ExecError>(MainControlStep::Continue)
        },
    )
}

pub(super) fn insert_math_shift<G>(
    processor: &mut Processor<'_, '_, G>,
    spelling: TracedTokenWord,
    meaning: &MeaningWord<G>,
    control_sequence: Option<Symbol>,
    exceptional: Option<&tex_command::CurrentCommand<G>>,
) -> Result<MainControlStep, ExecError> {
    let site = processor.capture_consumed_site(spelling, meaning, control_sequence, exceptional);
    processor
        .recover_missing_math_shift(spelling, exceptional)
        .map_err(command_error)?;
    report_scan(processor)?;
    processor.with_execution_owners(|context, command, effects, _, _, execution, _, _, _| {
        execution.main_loop_active = false;
        let display = command.output_open_context(context);
        let mut report = context.print_err("Missing $ inserted");
        report.help(&[
            "I've inserted a begin-math/end-math symbol since I think",
            "you left one out. Proceed, with fingers crossed.",
        ]);
        report.context(display);
        report.error_and_defer_at(effects, Some(site))?;
        Ok::<MainControlStep, ExecError>(MainControlStep::Continue)
    })?;
    finish_error_dialogue(processor)?;
    Ok::<MainControlStep, ExecError>(MainControlStep::Continue)
}
