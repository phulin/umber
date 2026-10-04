//! Font grammars and acquisition through the retained processor owners.
use super::*;

pub(super) fn define<G>(
    processor: &mut Processor<'_, '_, G>,
    global: bool,
    receipts: &mut Option<Vec<MutationRecord>>,
) -> Result<MainControlStep, ExecError> {
    let request = processor
        .scan_font_definition(global)
        .map_err(command_error)?;
    report_scan(processor)?;
    let resource = resolve(processor, &request)?;
    let step = processor.with_execution_owners(
        |context, command, effects, fuel, _, execution, _, _, _| {
            execution.main_loop_active = false;
            crate::box_runtime::flush_pending_hchars(&mut execution.modes, context, effects, fuel)?;
            semantic::execute_font_definition(
                context, command, effects, receipts, &resource, &request, global,
            )
        },
    )?;
    recovery::finish_error_dialogue(processor)?;
    processor.complete_assignment().map_err(command_error)?;
    Ok(step)
}

fn resolve<G>(
    processor: &mut Processor<'_, '_, G>,
    request: &FontLoadRequest,
) -> Result<FontResource, ExecError> {
    let path = crate::canonical_font_resource_path(&request.name);
    processor.with_execution_owners(
        |context, _, _, _, _, _, capabilities, mut provider, attempted_resource| {
            context.unsupported_host_capability();
            loop {
                if let Some(retained) = capabilities.font(&path) {
                    let dependencies = capabilities.font_dependencies(&path).unwrap_or_default();
                    let actual = context
                        .with_input_read_state(|input| retained.actual_use(input, &dependencies))
                        .map_err(ExecError::from)?;
                    let Some(actual) = actual else {
                        capabilities.invalidate_font_resource(&path);
                        continue;
                    };
                    if let Some(metrics) = font_metrics(&actual) {
                        let dependencies = tex_command::dependencies_for_actual_content(
                            &dependencies,
                            metrics.path(),
                            metrics.hash(),
                        );
                        context
                            .record_input_dependencies(&dependencies)
                            .map_err(ExecError::from)?;
                    } else {
                        context
                            .record_input_dependencies(&dependencies)
                            .map_err(ExecError::from)?;
                    }
                    return Ok(actual);
                }
                let need = ResourceNeed::Font {
                    request: request.clone(),
                };
                let Some(outcome) = resources::resolve_provider(
                    context,
                    capabilities,
                    provider.take(),
                    attempted_resource,
                    &need,
                    false,
                )?
                else {
                    return Err(ExecError::MissingFont {
                        request: Box::new(request.clone()),
                    });
                };
                match outcome {
                    ResourceInstallOutcome::Fulfilled(ResourceFulfillment::Font {
                        resource,
                        ..
                    }) => {
                        return Ok(*resource);
                    }
                    ResourceInstallOutcome::Fulfilled(_) => unreachable!("typed font fulfillment"),
                    ResourceInstallOutcome::Unavailable => return Ok(FontResource::Unavailable),
                    ResourceInstallOutcome::Declined => {
                        return Err(ExecError::MissingFont {
                            request: Box::new(request.clone()),
                        });
                    }
                    ResourceInstallOutcome::Failed(_) => {
                        unreachable!("failed provider outcomes are returned as errors")
                    }
                }
            }
        },
    )
}

pub(super) fn generated<G>(
    processor: &mut Processor<'_, '_, G>,
    kind: tex_command::GeneratedFontKind,
    global: bool,
    receipts: &mut Option<Vec<MutationRecord>>,
) -> Result<MainControlStep, ExecError> {
    let definition = processor
        .scan_generated_font_definition(kind, global)
        .map_err(command_error)?;
    report_scan(processor)?;
    let step =
        processor.with_execution_owners(|context, _, effects, _, _, execution, _, _, _| {
            execution.main_loop_active = false;
            semantic::execute_generated_font_definition(
                context, effects, receipts, definition, global,
            )
        })?;
    recovery::finish_error_dialogue(processor)?;
    processor.complete_assignment().map_err(command_error)?;
    Ok(step)
}

pub(super) fn dimension<G>(
    processor: &mut Processor<'_, '_, G>,
) -> Result<MainControlStep, ExecError> {
    let number = processor.scan_integer().map_err(command_error)?.value;
    let font = processor.scan_font_selector().map_err(command_error)?;
    let recovery_context =
        (!processor.font_dimen_writable(font, number)).then(|| processor.error_context());
    processor.scan_optional_equals().map_err(command_error)?;
    let value = processor.scan_dimension().map_err(command_error)?;
    report_scan(processor)?;
    let step =
        processor.with_execution_owners(|context, _, effects, _, _, execution, _, _, _| {
            execution.main_loop_active = false;
            semantic::execute_font_dimen(context, effects, font, number, value, recovery_context)
        })?;
    recovery::finish_error_dialogue(processor)?;
    processor.complete_assignment().map_err(command_error)?;
    Ok(step)
}

pub(super) fn integer<G>(
    processor: &mut Processor<'_, '_, G>,
    skew: bool,
) -> Result<MainControlStep, ExecError> {
    let font = processor.scan_font_selector().map_err(command_error)?;
    processor.scan_optional_equals().map_err(command_error)?;
    let value = processor.scan_integer().map_err(command_error)?.value;
    report_scan(processor)?;
    let step =
        processor.with_execution_owners(|context, _, effects, _, _, execution, _, _, _| {
            execution.main_loop_active = false;
            semantic::execute_font_integer(context, effects, font, skew, value)
        })?;
    processor.complete_assignment().map_err(command_error)?;
    Ok(step)
}

pub(super) fn hyphenation<G>(
    processor: &mut Processor<'_, '_, G>,
    patterns: bool,
) -> Result<MainControlStep, ExecError> {
    let rejection_context = processor.error_context();
    let trie_built = patterns && !processor.hyphenation_patterns_open();
    let scanned = if trie_built {
        let discarded = processor.scan_balanced_text(false).map_err(command_error)?;
        processor
            .with_execution_owners(|_, command, _, _, _, _, _, _, _| {
                command.discard_attempt_tokens(discarded.tokens)
            })
            .map_err(|_| ExecError::MissingToken {
                context: "discarded late patterns",
            })?;
        tex_command::ScannedHyphenationData::default()
    } else {
        processor
            .scan_hyphenation_data(if patterns {
                tex_command::HyphenationDataKind::Patterns
            } else {
                tex_command::HyphenationDataKind::Exceptions
            })
            .map_err(command_error)?
    };
    report_scan(processor)?;
    let step =
        processor.with_execution_owners(|context, _, effects, _, _, execution, _, _, _| {
            execution.main_loop_active = false;
            semantic::execute_hyphenation_data(
                context,
                effects,
                execution.initex,
                scanned.words,
                scanned.patterns,
                patterns,
                rejection_context,
                trie_built,
            )
        })?;
    recovery::finish_error_dialogue(processor)?;
    processor.complete_assignment().map_err(command_error)?;
    Ok(step)
}
