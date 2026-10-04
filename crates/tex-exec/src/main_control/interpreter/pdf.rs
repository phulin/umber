//! Concrete PDF font operands, with text consumed from scanner storage.
use super::*;

pub(super) fn font_code<G>(
    processor: &mut Processor<'_, '_, G>,
    table: tex_state::PdfFontCode,
    receipts: &mut Option<Vec<MutationRecord>>,
) -> Result<MainControlStep, ExecError> {
    let font = processor.scan_font_selector().map_err(command_error)?;
    let character = processor
        .scan_restricted_integer(RestrictedIntegerClass::CharacterCode)
        .map_err(command_error)?
        .value as u8;
    processor.scan_optional_equals().map_err(command_error)?;
    let value = processor.scan_integer().map_err(command_error)?.value;
    report_scan(processor)?;
    let step =
        processor.with_execution_owners(|context, _, effects, _, _, execution, _, _, _| {
            execution.main_loop_active = false;
            semantic::execute_pdf_font_code(
                context, effects, receipts, table, font, character, value,
            )
        })?;
    assignments::complete(processor, receipts)?;
    Ok(step)
}

pub(super) fn no_ligatures<G>(
    processor: &mut Processor<'_, '_, G>,
    receipts: &mut Option<Vec<MutationRecord>>,
) -> Result<MainControlStep, ExecError> {
    let font = processor.scan_font_selector().map_err(command_error)?;
    report_scan(processor)?;
    let step =
        processor.with_execution_owners(|context, _, effects, _, _, execution, _, _, _| {
            execution.main_loop_active = false;
            semantic::execute_pdf_no_ligatures(context, effects, receipts, font)
        })?;
    assignments::complete(processor, receipts)?;
    Ok(step)
}

pub(super) fn font_expand<G>(
    processor: &mut Processor<'_, '_, G>,
) -> Result<MainControlStep, ExecError> {
    let font = processor.scan_font_selector().map_err(command_error)?;
    processor.scan_optional_equals().map_err(command_error)?;
    let stretch = processor.scan_integer().map_err(command_error)?.value;
    let shrink = processor.scan_integer().map_err(command_error)?.value;
    let step = processor.scan_integer().map_err(command_error)?.value;
    let auto_expand = processor
        .scan_keyword("autoexpand")
        .map_err(command_error)?;
    let spec = tex_typeset::expansion::FontExpansionSpec::new(stretch, shrink, step, auto_expand)?;
    report_scan(processor)?;
    let step = processor.with_execution_owners(|context, _, _, _, _, execution, _, _, _| {
        execution.main_loop_active = false;
        semantic::execute_pdf_font_expand(context, font, spec)
    })?;
    processor.complete_assignment().map_err(command_error)?;
    Ok(step)
}

fn text<G>(processor: &mut Processor<'_, '_, G>) -> Result<Vec<u8>, ExecError> {
    let tokens = processor
        .scan_balanced_text(true)
        .map_err(command_error)?
        .tokens;
    report_scan(processor)?;
    processor.with_execution_owners(|context, command, _, _, _, _, _, _, _| {
        let text = streams::token_text(context, command, &tokens)?;
        command
            .discard_attempt_tokens(tokens)
            .expect("consumed PDF text owner");
        Ok(tex_byte_text(&text))
    })
}

pub(super) fn font_action<G>(
    processor: &mut Processor<'_, '_, G>,
    primitive: UnexpandablePrimitive,
) -> Result<MainControlStep, ExecError> {
    if processor.int_param(IntParam::PDF_OUTPUT) <= 0 {
        let name = match primitive {
            UnexpandablePrimitive::PdfFontAttr => Some("pdffontattr"),
            UnexpandablePrimitive::PdfIncludeChars => Some("pdfincludechars"),
            UnexpandablePrimitive::PdfMapFile => Some("pdfmapfile"),
            UnexpandablePrimitive::PdfMapLine => Some("pdfmapline"),
            _ => None,
        };
        if let Some(name) = name {
            return Err(ExecError::PdfExtensionInDviMode(name));
        }
    }
    let font = if matches!(
        primitive,
        UnexpandablePrimitive::PdfFontAttr
            | UnexpandablePrimitive::PdfIncludeChars
            | UnexpandablePrimitive::PdfNoBuiltinToUnicode
    ) {
        Some(processor.scan_font_selector().map_err(command_error)?)
    } else {
        None
    };
    let first = if primitive == UnexpandablePrimitive::PdfNoBuiltinToUnicode {
        None
    } else {
        Some(text(processor)?)
    };
    let second = if primitive == UnexpandablePrimitive::PdfGlyphToUnicode {
        Some(text(processor)?)
    } else {
        None
    };
    report_scan(processor)?;
    let step = processor.with_execution_owners(|context, _, _, _, _, execution, _, _, _| {
        execution.main_loop_active = false;
        semantic::execute_pdf_font_action(context, primitive, font, first, second)
    })?;
    processor.complete_assignment().map_err(command_error)?;
    Ok(step)
}

pub(super) fn graphics<G>(
    processor: &mut Processor<'_, '_, G>,
    primitive: UnexpandablePrimitive,
    spelling: TracedTokenWord,
    exceptional: Option<&tex_command::CurrentCommand<G>>,
) -> Result<MainControlStep, ExecError> {
    if primitive != UnexpandablePrimitive::PdfSavePos
        && processor.int_param(IntParam::PDF_OUTPUT) <= 0
    {
        let name = match primitive {
            UnexpandablePrimitive::PdfLiteral => "pdfliteral",
            UnexpandablePrimitive::PdfSetMatrix => "pdfsetmatrix",
            UnexpandablePrimitive::PdfSave => "pdfsave",
            UnexpandablePrimitive::PdfRestore => "pdfrestore",
            UnexpandablePrimitive::PdfColorStack => "pdfcolorstack",
            UnexpandablePrimitive::PdfSnapRefPoint => "pdfsnaprefpoint",
            UnexpandablePrimitive::PdfSnapY => "pdfsnapy",
            UnexpandablePrimitive::PdfSnapYComp => "pdfsnapycomp",
            _ => unreachable!("graphics selector"),
        };
        processor
            .back_consumed_word(spelling, exceptional)
            .map_err(command_error)?;
        return Err(ExecError::PdfExtensionInDviMode(name));
    }
    let scanned = processor
        .scan_pdf_graphics_request(primitive)
        .map_err(command_error)?
        .expect("selected PDF graphics grammar");
    report_scan(processor)?;
    processor.with_execution_owners(|context, command, effects, fuel, _, execution, _, _, _| {
        execution.main_loop_active = false;
        let mut request = publication::publish_tokens(command, context, scanned)?;
        apply_pdf_graphics_request(
            &mut request,
            context,
            &mut execution.modes,
            command,
            effects,
            fuel,
        )
    })
}

pub(super) fn navigation<G>(
    processor: &mut Processor<'_, '_, G>,
    primitive: UnexpandablePrimitive,
) -> Result<MainControlStep, ExecError> {
    let scanned = processor
        .scan_pdf_navigation_request(primitive)
        .map_err(command_error)?;
    report_scan(processor)?;
    processor.with_execution_owners(|context, command, effects, fuel, _, execution, _, _, _| {
        execution.main_loop_active = false;
        let mut request = publication::publish_tokens(command, context, scanned)?;
        apply_pdf_navigation_request(&mut request, context, &mut execution.modes, effects, fuel)
    })
}

pub(super) fn form<G>(
    processor: &mut Processor<'_, '_, G>,
    primitive: UnexpandablePrimitive,
    immediate: bool,
) -> Result<MainControlStep, ExecError> {
    let scanned = processor
        .scan_pdf_form_request(primitive)
        .map_err(command_error)?;
    report_scan(processor)?;
    processor.with_execution_owners(|context, command, effects, fuel, _, execution, _, _, _| {
        execution.main_loop_active = false;
        let mut request = publication::publish_tokens(command, context, scanned)?;
        let pending = apply_pdf_form_request(
            &mut request,
            context,
            &mut execution.modes,
            effects,
            fuel,
            immediate,
        )?;
        execution.prepared_pdf_form = pending;
        Ok(MainControlStep::Continue)
    })
}

pub(super) fn object<G>(
    processor: &mut Processor<'_, '_, G>,
    immediate: bool,
) -> Result<MainControlStep, ExecError> {
    let scanned = processor.scan_pdf_object_request().map_err(command_error)?;
    report_scan(processor)?;
    processor.with_execution_owners(|context, command, _, _, _, execution, _, _, _| {
        execution.main_loop_active = false;
        let mut request = publication::publish_tokens(command, context, scanned)?;
        if immediate && matches!(request, tex_command::PdfObjectRequest::Reserve) {
            return Err(ExecError::PdfImmediateReservedObject);
        }
        apply_pdf_object_request(&mut request, context, immediate)
    })
}

pub(super) fn document_fragment<G>(
    processor: &mut Processor<'_, '_, G>,
    primitive: UnexpandablePrimitive,
) -> Result<MainControlStep, ExecError> {
    let scanned = processor
        .scan_pdf_document_fragment_request(primitive)
        .map_err(command_error)?;
    report_scan(processor)?;
    processor.with_execution_owners(|context, command, _, _, _, execution, _, _, _| {
        execution.main_loop_active = false;
        let mut request = publication::publish_tokens(command, context, scanned)?;
        semantic::execute_pdf_document_fragment(context, &mut request)
    })
}

pub(super) fn require_output<G>(
    processor: &mut Processor<'_, '_, G>,
    spelling: TracedTokenWord,
    exceptional: Option<&tex_command::CurrentCommand<G>>,
    name: &'static str,
) -> Result<(), ExecError> {
    if processor.int_param(IntParam::PDF_OUTPUT) <= 0 {
        processor
            .back_consumed_word(spelling, exceptional)
            .map_err(command_error)?;
        return Err(ExecError::PdfExtensionInDviMode(name));
    }
    Ok(())
}

pub(super) fn space_font<G>(
    processor: &mut Processor<'_, '_, G>,
) -> Result<MainControlStep, ExecError> {
    let name = text(processor)?;
    processor.with_execution_owners(|context, _, _, _, _, execution, _, _, _| {
        execution.main_loop_active = false;
        context.set_pdf_space_font_name(name);
    });
    Ok(MainControlStep::Continue)
}

pub(super) fn image<G>(processor: &mut Processor<'_, '_, G>) -> Result<MainControlStep, ExecError> {
    let mut request = processor.scan_pdf_image_request().map_err(command_error)?;
    report_scan(processor)?;
    processor.with_execution_owners(
        |context, command, _, _, _, execution, capabilities, provider, attempted| {
            execution.main_loop_active = false;
            let resource =
                resources::image(&mut request, context, capabilities, provider, attempted)?;
            let source = match resource {
                PdfImageResource::Available(source) => source,
                PdfImageResource::Unavailable => {
                    return Err(ExecError::PdfImageOpen {
                        name: request.name,
                        message: "image is unavailable".to_owned(),
                    });
                }
                PdfImageResource::Invalid(message) => {
                    return Err(ExecError::PdfImageOpen {
                        name: request.name,
                        message,
                    });
                }
            };
            let dimensions =
                pdf_image_dimensions(&source, request.width, request.height, request.depth);
            let attributes = if let Some(tokens) = request.attr {
                let text = streams::token_text(context, command, &tokens)?;
                command
                    .discard_attempt_tokens(tokens)
                    .expect("consumed image attributes");
                tex_byte_text(&text)
            } else {
                Vec::new()
            };
            context
                .allocate_pdf_external_image(
                    source,
                    dimensions,
                    request.color_space_object,
                    attributes,
                )
                .map_err(|_| ExecError::PdfObjectCapacity)?;
            Ok(MainControlStep::Continue)
        },
    )
}
