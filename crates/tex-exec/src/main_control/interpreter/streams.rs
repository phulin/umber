//! Streams and text consume actual scanner storage at their final destination.
use super::*;

pub(super) fn immediate<G>(
    processor: &mut Processor<'_, '_, G>,
    outer: TracedTokenWord,
    outer_exceptional: Option<&tex_command::CurrentCommand<G>>,
) -> Result<MainControlStep, ExecError> {
    let mut result = None;
    let status = processor
        .consume_main_control(&mut |processor, spelling, meaning, _, exceptional| {
            let code = match meaning {
                tex_state::meaning::MeaningWord::Static(word) => {
                    Meaning::runtime_word_control_code(*word)
                }
                _ => codes::FONT,
            };
            if code == codes::CHAR_SPACE {
                return Ok(ControlFlow::Continue(()));
            }
            result = Some((|| {
                let pdf_name = match code {
                    codes::PRIMITIVE_PDF_OBJECT => Some("pdfobj"),
                    codes::PRIMITIVE_PDF_X_FORM => Some("pdfxform"),
                    codes::PRIMITIVE_PDF_X_IMAGE => Some("pdfximage"),
                    _ => None,
                };
                if let Some(name) = pdf_name
                    && processor.int_param(IntParam::PDF_OUTPUT) <= 0
                {
                    processor
                        .back_consumed_word(spelling, exceptional)
                        .map_err(command_error)?;
                    processor
                        .back_consumed_word(outer, outer_exceptional)
                        .map_err(command_error)?;
                    return Err(ExecError::PdfExtensionInDviMode(name));
                }
                match code {
                    codes::PRIMITIVE_OPEN_OUT => open_out(processor, true),
                    codes::PRIMITIVE_WRITE => write(processor, true),
                    codes::PRIMITIVE_CLOSE_OUT => close_out(processor, true),
                    codes::PRIMITIVE_PDF_OBJECT => pdf::object(processor, true),
                    codes::PRIMITIVE_PDF_X_FORM => {
                        pdf::form(processor, UnexpandablePrimitive::PdfXForm, true)
                    }
                    codes::PRIMITIVE_PDF_X_IMAGE => pdf::image(processor),
                    _ => {
                        processor
                            .back_consumed_word(spelling, exceptional)
                            .map_err(command_error)?;
                        Ok(MainControlStep::Continue)
                    }
                }
            })());
            Ok(ControlFlow::Break(()))
        })
        .map_err(command_error)?;
    match result {
        Some(result) => result,
        None => finish_input_boundary(processor, status),
    }
}

pub(super) fn token_text<G>(
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
    processor.with_execution_owners(
        |context, command, effects, _, observer, execution, _, _, _| {
            execution.main_loop_active = false;
            let text = token_text(context, command, &tokens)?;
            command
                .discard_attempt_tokens(tokens)
                .expect("consumed message owner");
            if error {
                let display = command.output_open_context(context);
                issue_error_message(context, effects, &text, display)?;
            } else {
                context.publish_diagnostic_effects_before_synchronous_print(effects);
                issue_terminal_message(context, &text);
                if let Some(observer) = observer {
                    observer.committed(CommandObservation::Effect(EffectRecord {
                        kind: ObservationEffectKind::Message,
                        channel: "terminal".into(),
                        value: ObservationValue::Bytes(
                            command
                                .profile()
                                .encode_output_text(&text)
                                .expect("message belongs to the active character domain"),
                        ),
                        source: None,
                    }));
                }
            }
            Ok::<_, ExecError>(())
        },
    )?;
    if error {
        recovery::finish_error_dialogue(processor)?;
    }
    Ok(MainControlStep::Continue)
}

pub(super) fn open_out<G>(
    processor: &mut Processor<'_, '_, G>,
    immediate: bool,
) -> Result<MainControlStep, ExecError> {
    let stream = processor
        .scan_restricted_integer(RestrictedIntegerClass::FourBit)
        .map_err(command_error)?
        .value as u8;
    processor.scan_optional_equals().map_err(command_error)?;
    let path = processor.scan_file_name().map_err(command_error)?.packed();
    report_scan(processor)?;
    processor.with_execution_owners(
        |context, command, effects, fuel, _, execution, capabilities, _, _| {
            execution.main_loop_active = false;
            if immediate {
                let target = replay_openout_target(path);
                context.open_output_stream(StreamSlot::new(stream), target.clone().into());
                capabilities.invalidate_input_unavailability_for_output(&target);
                capabilities
                    .invalidate_font_unavailability_for_output(std::path::Path::new(&target));
                capabilities.invalidate_pdf_images_for_output(std::path::Path::new(&target));
                if command.engine_semantics().supports_pdftex() {
                    context.publish_diagnostic_effects_before_synchronous_print(effects);
                    let printer = context.printer();
                    let terminal_open = printer.terminal_offset() > 0;
                    let log_open = printer.log_offset() > 0;
                    let (sink, text) = crate::diagnostics::report_openout(
                        context.int_param(IntParam::TRACING_ONLINE),
                        terminal_open,
                        log_open,
                        stream,
                        &target,
                    );
                    context.publish_openout_notice(effects, sink, &text);
                }
                return Ok(MainControlStep::Continue);
            }

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
        },
    )
}

pub(super) fn close_out<G>(
    processor: &mut Processor<'_, '_, G>,
    immediate: bool,
) -> Result<MainControlStep, ExecError> {
    let stream = processor.scan_write_stream().map_err(command_error)?;
    report_scan(processor)?;
    processor.with_execution_owners(|context, _, effects, fuel, _, execution, _, _, _| {
        execution.main_loop_active = false;
        if immediate {
            if let Some(slot) = stream.stream_slot() {
                context.close_output_stream(slot);
            }
            return Ok(MainControlStep::Continue);
        }

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

pub(super) fn write<G>(
    processor: &mut Processor<'_, '_, G>,
    immediate: bool,
) -> Result<MainControlStep, ExecError> {
    let stream = processor.scan_write_stream().map_err(command_error)?;
    let tokens = if immediate {
        let tokens = processor
            .scan_immediate_write_text()
            .map_err(command_error)?;
        processor
            .expand_write_text(tokens)
            .map_err(command_error)?
            .tokens
    } else {
        processor
            .scan_balanced_text(false)
            .map_err(command_error)?
            .tokens
    };
    report_scan(processor)?;
    processor.with_execution_owners(
        |context, command, effects, fuel, observer, execution, _, _, _| {
            execution.main_loop_active = false;
            if immediate {
                let text = token_text(context, command, &tokens)?;
                if let Some(observer) = observer {
                    observer.committed(CommandObservation::Effect(EffectRecord {
                        kind: ObservationEffectKind::Write,
                        channel: write_effect_channel(replay_write_sink(stream)),
                        value: ObservationValue::Tokens(
                            command
                                .attempt_token_words(&tokens)
                                .expect("live expanded write")
                                .iter()
                                .map(|word| observed_macro_token(word.semantic_token(), context))
                                .collect(),
                        ),
                        source: None,
                    }));
                }
                command
                    .discard_attempt_tokens(tokens)
                    .expect("consumed immediate write owner");
                let mut text = crate::diagnostics::print_text_with_newlinechar(context, &text);
                text.push('\n');
                let bytes = command
                    .profile()
                    .encode_output_text(&text)
                    .expect("write text belongs to the active command character domain");
                if let Some(sink) = immediate_write_sink(stream, context) {
                    context.publish_immediate_write(effects, sink, &text, &bytes);
                }
                return Ok(MainControlStep::Continue);
            }

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
        },
    )
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

pub(super) fn close_in<G>(
    processor: &mut Processor<'_, '_, G>,
) -> Result<MainControlStep, ExecError> {
    let stream = processor
        .scan_restricted_integer(RestrictedIntegerClass::FourBit)
        .map_err(command_error)?
        .value;
    report_scan(processor)?;
    processor.with_execution_owners(|context, _, _, _, _, execution, _, _, _| {
        execution.main_loop_active = false;
        context.unsupported_host_capability();
        context.close_in(replay_stream_slot(stream));
    });
    Ok(MainControlStep::Continue)
}

pub(super) fn open_in<G>(
    processor: &mut Processor<'_, '_, G>,
) -> Result<MainControlStep, ExecError> {
    let stream = processor
        .scan_restricted_integer(RestrictedIntegerClass::FourBit)
        .map_err(command_error)?
        .value;
    processor.scan_optional_equals().map_err(command_error)?;
    let mut file_name = processor.scan_file_name().map_err(command_error)?;
    file_name.components.apply_default_extension(".tex");
    let name = file_name.packed();
    report_scan(processor)?;
    processor.with_execution_owners(
        |context, _, _, _, _, execution, capabilities, mut provider, attempted| {
            execution.main_loop_active = false;
            context.unsupported_host_capability();
            let request = tex_command::FileEnquiryRequest::new(
                name.clone(),
                tex_command::FileEnquiryIntent::OpenInProbe,
            );
            let source = loop {
                if let Some(retained) = capabilities.input_probe_resource(&name) {
                    let actual = context
                        .with_input_read_state(|input| retained.actual_use(input))
                        .map_err(ExecError::from)?;
                    let Some(actual) = actual else {
                        capabilities.invalidate_input_probe_resource(&name);
                        continue;
                    };
                    let dependencies = retained.dependencies_for_actual_use(&actual);
                    context
                        .record_input_dependencies(&dependencies)
                        .map_err(ExecError::from)?;
                    break Some(actual.source().clone());
                }
                if capabilities.input_probe_is_unavailable(&name) {
                    let dependencies = capabilities.input_probe_dependencies(&name);
                    context
                        .record_input_dependencies(&dependencies)
                        .map_err(ExecError::from)?;
                    break None;
                }
                let need = ResourceNeed::InputProbe {
                    request: request.clone(),
                };
                let Some(outcome) = resources::resolve_provider(
                    context,
                    capabilities,
                    provider.take(),
                    attempted,
                    &need,
                    false,
                )?
                else {
                    return Err(ExecError::MissingInputProbe { request });
                };
                match outcome {
                    ResourceInstallOutcome::Fulfilled(ResourceFulfillment::InputProbe {
                        resource,
                        ..
                    }) => {
                        break Some(resource.source().clone());
                    }
                    ResourceInstallOutcome::Fulfilled(_) => {
                        unreachable!("typed input probe fulfillment")
                    }
                    ResourceInstallOutcome::Unavailable => break None,
                    ResourceInstallOutcome::Declined => {
                        return Err(ExecError::MissingInputProbe { request });
                    }
                    ResourceInstallOutcome::Failed(_) => {
                        unreachable!("failed provider outcomes are returned as errors")
                    }
                }
            };
            let slot = replay_stream_slot(stream);
            context.close_in(slot);
            if let Some(source) = source {
                if let Some(record) = source.active_world_record() {
                    context.open_in_record(slot, record)?;
                } else {
                    context.open_in_bytes(slot, &name, source.bytes())?;
                }
            }
            Ok(MainControlStep::Continue)
        },
    )
}

pub(super) fn read<G>(
    processor: &mut Processor<'_, '_, G>,
    line: bool,
    global: bool,
    receipts: &mut Option<Vec<MutationRecord>>,
) -> Result<MainControlStep, ExecError> {
    let (target, definition) = processor
        .scan_read_definition(line)
        .map_err(command_error)?;
    report_scan(processor)?;
    let step =
        processor.with_execution_owners(|context, _, effects, _, _, execution, _, _, _| {
            execution.main_loop_active = false;
            context.unsupported_host_capability();
            semantic::execute_read_definition(
                context, effects, receipts, target, definition, global,
            )
        })?;
    assignments::complete(processor, receipts)?;
    Ok(step)
}
