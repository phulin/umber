//! Streams and text consume actual scanner storage at their final destination.
use super::*;

fn token_text<G>(
    context: &CommandContext<'_, G>,
    command: &CommandState<G>,
    tokens: &tex_command::AttemptTokenListId,
) -> Result<String, ExecError> {
    let mut text = String::new();
    for word in command
        .attempt_token_words(tokens)
        .map_err(|_| ExecError::MissingToken {
            context: "scanned text owner",
        })?
    {
        tex_state::token_show::append_token_string_text(context, word.semantic_token(), &mut text);
    }
    Ok(text)
}

pub(super) fn message<G>(
    processor: &mut Processor<'_, '_, G>,
    error: bool,
) -> Result<MainControlStep, ExecError> {
    let tokens = processor
        .scan_balanced_text(true)
        .map_err(command_error)?
        .tokens;
    report_scan(processor)?;
    processor.with_execution_owners(|context, command, effects, _, _, execution, _, _, _| {
        execution.main_loop_active = false;
        let text = token_text(context, command, &tokens)?;
        command
            .discard_attempt_tokens(tokens)
            .expect("consumed message owner");
        if error {
            let display = command.output_open_context(context);
            issue_error_message(context, effects, &text, display)?;
        } else {
            issue_terminal_message(context, &text);
        }
        Ok::<_, ExecError>(())
    })?;
    if error {
        recovery::finish_error_dialogue(processor)?;
    }
    Ok(MainControlStep::Continue)
}

pub(super) fn open_out<G>(
    processor: &mut Processor<'_, '_, G>,
) -> Result<MainControlStep, ExecError> {
    let stream = processor
        .scan_restricted_integer(RestrictedIntegerClass::FourBit)
        .map_err(command_error)?
        .value as u8;
    processor.scan_optional_equals().map_err(command_error)?;
    let path = processor.scan_file_name().map_err(command_error)?.packed();
    report_scan(processor)?;
    processor.with_execution_owners(|context, _, effects, fuel, _, execution, _, _, _| {
        execution.main_loop_active = false;
        crate::box_runtime::append_whatsit(
            &mut execution.modes,
            context,
            effects,
            fuel,
            Whatsit::OpenOut {
                slot: StreamSlot::new(stream),
                path,
            },
        )?;
        Ok(MainControlStep::Continue)
    })
}

pub(super) fn close_out<G>(
    processor: &mut Processor<'_, '_, G>,
) -> Result<MainControlStep, ExecError> {
    let stream = processor.scan_write_stream().map_err(command_error)?;
    report_scan(processor)?;
    processor.with_execution_owners(|context, _, effects, fuel, _, execution, _, _, _| {
        execution.main_loop_active = false;
        crate::box_runtime::append_whatsit(
            &mut execution.modes,
            context,
            effects,
            fuel,
            Whatsit::CloseOut {
                slot: stream.stream_slot(),
            },
        )?;
        Ok(MainControlStep::Continue)
    })
}

pub(super) fn write<G>(processor: &mut Processor<'_, '_, G>) -> Result<MainControlStep, ExecError> {
    let stream = processor.scan_write_stream().map_err(command_error)?;
    let tokens = processor
        .scan_balanced_text(false)
        .map_err(command_error)?
        .tokens;
    report_scan(processor)?;
    processor.with_execution_owners(|context, command, effects, fuel, _, execution, _, _, _| {
        execution.main_loop_active = false;
        let tokens = publication::publish_tokens(command, context, tokens)?;
        let tokens = context.node_token_list(&tokens);
        crate::box_runtime::append_whatsit(
            &mut execution.modes,
            context,
            effects,
            fuel,
            Whatsit::DeferredWrite {
                sink: replay_write_sink(stream),
                tokens,
            },
        )?;
        Ok(MainControlStep::Continue)
    })
}

pub(super) fn special<G>(
    processor: &mut Processor<'_, '_, G>,
) -> Result<MainControlStep, ExecError> {
    let (deferred, text) = processor.scan_special().map_err(command_error)?;
    let tokens = text.tokens;
    report_scan(processor)?;
    processor.with_execution_owners(|context, command, effects, fuel, _, execution, _, _, _| {
        execution.main_loop_active = false;
        let whatsit = if deferred {
            let tokens = publication::publish_tokens(command, context, tokens)?;
            Whatsit::DeferredSpecial {
                class: "dvi".to_owned(),
                tokens: context.node_token_list(&tokens),
            }
        } else {
            let text = token_text(context, command, &tokens)?;
            command
                .discard_attempt_tokens(tokens)
                .expect("consumed special owner");
            Whatsit::Special {
                class: "dvi".to_owned(),
                payload: tex_byte_text(&text),
            }
        };
        crate::box_runtime::append_whatsit(&mut execution.modes, context, effects, fuel, whatsit)?;
        Ok(MainControlStep::Continue)
    })
}
