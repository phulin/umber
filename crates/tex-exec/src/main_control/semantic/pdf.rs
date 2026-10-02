//! Direct pdf semantics on native owners.
use super::super::*;

pub(in crate::main_control) fn execute_pdf_font_code<G>(
    stores: &mut tex_state::CommandContext<'_, G>,
    command: &mut CommandMachine<'_, '_, G>,
    table: &mut tex_state::PdfFontCode,
    font: &mut FontId,
    character: &mut u8,
    value: &mut i32,
) -> Result<ReplayStep, ExecError> {
    let mut linear_stores = LinearCommandContext::new(stores);
    let stores = &mut linear_stores;
    {
        AssignmentCommitter::new(stores, command.diagnostic_effects)
            .unscoped(|stores| stores.set_pdf_font_code(*table, *font, *character, *value));
        if command.observes_mutations() {
            command.retain_assignment_mutation(MutationRecord {
                target: MutationTarget::Register,
                key: ObservationValue::Name(format!("{table:?}:{}:{character}", font.raw())),
                value: ObservationValue::Integer(i64::from(*value)),
                global: true,
            });
        }
        Ok(ReplayStep::Continue)
    }
}

pub(in crate::main_control) fn execute_pdf_no_ligatures<G>(
    stores: &mut tex_state::CommandContext<'_, G>,
    command: &mut CommandMachine<'_, '_, G>,
    font: &mut FontId,
) -> Result<ReplayStep, ExecError> {
    let mut linear_stores = LinearCommandContext::new(stores);
    let stores = &mut linear_stores;
    {
        AssignmentCommitter::new(stores, command.diagnostic_effects)
            .unscoped(|stores| stores.disable_pdf_font_ligatures(*font));
        if command.observes_mutations() {
            command.retain_assignment_mutation(MutationRecord {
                target: MutationTarget::Register,
                key: ObservationValue::Name("pdf_no_ligatures".into()),
                value: ObservationValue::Name(font.raw().to_string()),
                global: true,
            });
        }
        Ok(ReplayStep::Continue)
    }
}

pub(in crate::main_control) fn execute_pdf_x_image<G>(
    stores: &mut tex_state::CommandContext<'_, G>,
    request: &mut tex_command::PdfImageRequest<tex_state::TokenListId<G>>,
    resource: &mut PdfImageResource,
) -> Result<ReplayStep, ExecError> {
    let mut linear_stores = LinearCommandContext::new(stores);
    let stores = &mut linear_stores;
    {
        if stores.int_param(IntParam::PDF_OUTPUT) <= 0 {
            return Err(ExecError::PdfExtensionInDviMode("pdfximage"));
        }
        let source = match std::mem::replace(resource, PdfImageResource::Unavailable) {
            PdfImageResource::Available(source) => source,
            PdfImageResource::Unavailable => {
                return Err(ExecError::PdfImageOpen {
                    name: std::mem::take(&mut request.name),
                    message: "image is unavailable".to_owned(),
                });
            }
            PdfImageResource::Invalid(message) => {
                return Err(ExecError::PdfImageOpen {
                    name: std::mem::take(&mut request.name),
                    message,
                });
            }
        };
        let dimensions =
            pdf_image_dimensions(&source, request.width, request.height, request.depth);
        // pdftex.web §776 prints the `scan_image` attribute token list
        // verbatim before the image backend contributes its dictionary.
        let attributes = request
            .attr
            .as_ref()
            .map(|tokens| pdf_graphics_text(tokens, stores))
            .unwrap_or_default();
        stores
            .allocate_pdf_external_image(source, dimensions, request.color_space_object, attributes)
            .map_err(|_| ExecError::PdfObjectCapacity)?;
        Ok(ReplayStep::Continue)
    }
}

pub(in crate::main_control) fn execute_pdf_ref_x_image<G>(
    stores: &mut tex_state::CommandContext<'_, G>,
    modes: &mut ModeNest,
    command: &mut CommandMachine<'_, '_, G>,
    object: &mut i32,
) -> Result<ReplayStep, ExecError> {
    let mut linear_stores = LinearCommandContext::new(stores);
    let stores = &mut linear_stores;
    {
        if stores.int_param(IntParam::PDF_OUTPUT) <= 0 {
            return Err(ExecError::PdfExtensionInDviMode("pdfrefximage"));
        }
        let image = u32::try_from(*object)
            .ok()
            .and_then(|raw| tex_state::PdfExternalImageId::new(raw).ok())
            .and_then(|id| stores.pdf_external_image_record(id))
            .ok_or(ExecError::PdfReferencedObjectNotFound)?;
        let dimensions = image.dimensions();
        crate::box_runtime::append_whatsit(
            modes,
            stores,
            command.diagnostic_effects,
            command.fuel,
            Whatsit::PdfRefXImage {
                object: image.id().raw(),
                width: dimensions.width,
                height: dimensions.height,
                depth: dimensions.depth,
            },
        )?;
        Ok(ReplayStep::Continue)
    }
}

pub(in crate::main_control) fn execute_pdf_interword_space<G>(
    stores: &mut tex_state::CommandContext<'_, G>,
    modes: &mut ModeNest,
    command: &mut CommandMachine<'_, '_, G>,
    control: &mut tex_state::node::PdfAccessibilityControl,
) -> Result<ReplayStep, ExecError> {
    let mut linear_stores = LinearCommandContext::new(stores);
    let stores = &mut linear_stores;
    {
        if stores.int_param(IntParam::PDF_OUTPUT) <= 0 {
            let name = match control {
                tex_state::node::PdfAccessibilityControl::InterwordSpaceOn => "pdfinterwordspaceon",
                tex_state::node::PdfAccessibilityControl::InterwordSpaceOff => {
                    "pdfinterwordspaceoff"
                }
                tex_state::node::PdfAccessibilityControl::FakeSpace => "pdffakespace",
            };
            return Err(ExecError::PdfExtensionInDviMode(name));
        }
        crate::box_runtime::append_whatsit(
            modes,
            stores,
            command.diagnostic_effects,
            command.fuel,
            Whatsit::PdfAccessibility(*control),
        )?;
        Ok(ReplayStep::Continue)
    }
}

pub(in crate::main_control) fn execute_pdf_running_link<G>(
    stores: &mut tex_state::CommandContext<'_, G>,
    modes: &mut ModeNest,
    command: &mut CommandMachine<'_, '_, G>,
    enabled: &mut bool,
) -> Result<ReplayStep, ExecError> {
    let mut linear_stores = LinearCommandContext::new(stores);
    let stores = &mut linear_stores;
    {
        if stores.int_param(IntParam::PDF_OUTPUT) <= 0 {
            return Err(ExecError::PdfExtensionInDviMode(if *enabled {
                "pdfrunninglinkon"
            } else {
                "pdfrunninglinkoff"
            }));
        }
        crate::box_runtime::append_whatsit(
            modes,
            stores,
            command.diagnostic_effects,
            command.fuel,
            Whatsit::PdfRunningLink(*enabled),
        )?;
        Ok(ReplayStep::Continue)
    }
}

pub(in crate::main_control) fn execute_pdf_space_font<G>(
    stores: &mut tex_state::CommandContext<'_, G>,
    tokens: &mut tex_state::TokenListId<G>,
) -> Result<ReplayStep, ExecError> {
    let mut linear_stores = LinearCommandContext::new(stores);
    let stores = &mut linear_stores;
    {
        if stores.int_param(IntParam::PDF_OUTPUT) <= 0 {
            return Err(ExecError::PdfExtensionInDviMode("pdfspacefont"));
        }
        let name = pdf_graphics_text(tokens, stores);
        stores.set_pdf_space_font_name(name);
        Ok(ReplayStep::Continue)
    }
}

pub(in crate::main_control) fn execute_pdf_graphics<G>(
    stores: &mut tex_state::CommandContext<'_, G>,
    modes: &mut ModeNest,
    command: &mut CommandMachine<'_, '_, G>,
    request: &mut tex_command::PdfGraphicsRequest<tex_state::TokenListId<G>>,
) -> Result<ReplayStep, ExecError> {
    let mut linear_stores = LinearCommandContext::new(stores);
    let stores = &mut linear_stores;
    apply_pdf_graphics_request(
        request,
        stores,
        modes,
        command.state,
        command.diagnostic_effects,
        command.fuel,
    )
}

pub(in crate::main_control) fn execute_pdf_object<G>(
    stores: &mut tex_state::CommandContext<'_, G>,
    request: &mut tex_command::PdfObjectRequest<tex_state::TokenListId<G>>,
) -> Result<ReplayStep, ExecError> {
    let mut linear_stores = LinearCommandContext::new(stores);
    let stores = &mut linear_stores;
    apply_pdf_object_request(request, stores, false)
}

pub(in crate::main_control) fn execute_pdf_reference_object<G>(
    stores: &mut tex_state::CommandContext<'_, G>,
    modes: &mut ModeNest,
    command: &mut CommandMachine<'_, '_, G>,
    request: &mut PdfReferenceObjectRequest,
) -> Result<ReplayStep, ExecError> {
    let mut linear_stores = LinearCommandContext::new(stores);
    let stores = &mut linear_stores;
    {
        if stores.int_param(IntParam::PDF_OUTPUT) <= 0 {
            return Err(ExecError::PdfExtensionInDviMode("pdfrefobj"));
        }
        let object = u32::try_from(request.object)
            .ok()
            .filter(|object| stores.pdf_raw_object(*object).is_some())
            .ok_or(ExecError::PdfReferencedObjectNotFound)?;
        crate::box_runtime::append_whatsit(
            modes,
            stores,
            command.diagnostic_effects,
            command.fuel,
            Whatsit::PdfReferenceObject { object },
        )?;
        Ok(ReplayStep::Continue)
    }
}

pub(in crate::main_control) fn execute_pdf_form<G>(
    stores: &mut tex_state::CommandContext<'_, G>,
    modes: &mut ModeNest,
    command: &mut CommandMachine<'_, '_, G>,
    request: &mut tex_command::PdfFormRequest<tex_state::TokenListId<G>>,
) -> Result<ReplayStep, ExecError> {
    let mut linear_stores = LinearCommandContext::new(stores);
    let stores = &mut linear_stores;
    {
        apply_pdf_form_request(request, stores, modes, command, false).map(|pending| {
            debug_assert!(pending.is_none());
            ReplayStep::Continue
        })
    }
}

pub(in crate::main_control) fn execute_pdf_document_fragment<G>(
    stores: &mut tex_state::CommandContext<'_, G>,
    request: &mut tex_command::PdfDocumentFragmentRequest<tex_state::TokenListId<G>>,
) -> Result<ReplayStep, ExecError> {
    let mut linear_stores = LinearCommandContext::new(stores);
    let stores = &mut linear_stores;
    {
        let dvi_only_error = matches!(request.kind, tex_state::PdfDocumentFragmentKind::Names);
        if stores.int_param(IntParam::PDF_OUTPUT) <= 0 {
            if dvi_only_error {
                return Err(ExecError::PdfExtensionInDviMode("pdfnames"));
            }
            let name = match request.kind {
                tex_state::PdfDocumentFragmentKind::Info => "pdfinfo",
                tex_state::PdfDocumentFragmentKind::Catalog => "pdfcatalog",
                tex_state::PdfDocumentFragmentKind::Trailer => "pdftrailer",
                tex_state::PdfDocumentFragmentKind::TrailerId => "pdftrailerid",
                tex_state::PdfDocumentFragmentKind::Names => {
                    unreachable!("pdfnames is rejected before the ignored-fragment warning")
                }
            };
            stores.printer().print_rendered(&format!(
                        "\npdfTeX warning (\\{name}): not allowed in DVI mode (\\pdfoutput <= 0); ignoring it\n"
                    ));
            return Ok(ReplayStep::Continue);
        }
        stores.append_pdf_document_fragment(request.kind, request.text.tokens.clone());
        if let Some(action) = &mut request.open_action {
            let action = take_admitted_pdf_action(action);
            if stores.pdf_catalog_open_action().is_some() {
                return Err(ExecError::PdfDuplicateOpenAction);
            }
            let (destination, structure, thread) = pdf_action_target_identities(stores, &action);
            stores
                .set_pdf_catalog_open_action_with_targets(action, destination, structure, thread)
                .map_err(|_| ExecError::PdfObjectCapacity)?;
        }
        Ok(ReplayStep::Continue)
    }
}

pub(in crate::main_control) fn execute_pdf_navigation<G>(
    stores: &mut tex_state::CommandContext<'_, G>,
    modes: &mut ModeNest,
    command: &mut CommandMachine<'_, '_, G>,
    request: &mut tex_command::PdfNavigationRequest<tex_state::TokenListId<G>>,
) -> Result<ReplayStep, ExecError> {
    let mut linear_stores = LinearCommandContext::new(stores);
    let stores = &mut linear_stores;
    apply_pdf_navigation_request(
        request,
        stores,
        modes,
        command.diagnostic_effects,
        command.fuel,
    )
}

pub(in crate::main_control) fn execute_pdf_font_expand<G>(
    stores: &mut tex_state::CommandContext<'_, G>,
    font: &mut FontId,
    spec: &mut tex_typeset::expansion::FontExpansionSpec,
) -> Result<ReplayStep, ExecError> {
    let mut linear_stores = LinearCommandContext::new(stores);
    let stores = &mut linear_stores;
    {
        stores.configure_font_expansion(
            *font,
            tex_state::font::FontExpansion {
                stretch: spec.stretch() as u16,
                shrink: spec.shrink() as u16,
                step: spec.step() as u8,
                auto_expand: spec.auto_expand(),
            },
        )?;
        Ok(ReplayStep::Continue)
    }
}

pub(in crate::main_control) fn execute_pdf_font_action<G>(
    stores: &mut tex_state::CommandContext<'_, G>,
    primitive: &mut UnexpandablePrimitive,
    font: &mut Option<FontId>,
    first: &mut Option<tex_state::TokenListId<G>>,
    second: &mut Option<tex_state::TokenListId<G>>,
) -> Result<ReplayStep, ExecError> {
    let mut linear_stores = LinearCommandContext::new(stores);
    let stores = &mut linear_stores;
    {
        let first = first
            .as_ref()
            .map(|tokens| pdf_graphics_text(tokens, stores));
        match primitive {
            UnexpandablePrimitive::PdfFontAttr => stores.set_pdf_font_attribute(
                font.expect("font attribute scanned a font"),
                first.expect("font attribute scanned text"),
            ),
            UnexpandablePrimitive::PdfIncludeChars => stores.include_pdf_font_chars(
                font.expect("include chars scanned a font"),
                first.expect("include chars scanned text"),
            ),
            UnexpandablePrimitive::PdfNoBuiltinToUnicode => stores
                .disable_pdf_builtin_to_unicode(font.expect("no builtin ToUnicode scanned a font")),
            UnexpandablePrimitive::PdfGlyphToUnicode => {
                let glyph = first.expect("glyph mapping scanned a glyph");
                let unicode = pdf_graphics_text(
                    second
                        .as_ref()
                        .expect("glyph mapping scanned a Unicode value"),
                    stores,
                );
                match parse_glyph_to_unicode(&glyph, &unicode) {
                    GlyphToUnicodeParse::Mapping(mapping) => {
                        stores.set_pdf_glyph_to_unicode(mapping)
                    }
                    GlyphToUnicodeParse::Warning(message) => {
                        stores.printer().print_rendered(&format!(
                            "\npdfTeX warning: pdftex: ToUnicode: {message}\n"
                        ));
                    }
                }
            }
            UnexpandablePrimitive::PdfMapFile => {
                let bytes = first.expect("map file scanned text");
                if bytes.iter().all(u8::is_ascii_whitespace) {
                    stores.push_pdf_font_map(tex_state::PdfFontMapOperation::BlockDefault);
                } else {
                    stores.push_pdf_font_map(tex_state::PdfFontMapOperation::File(
                        tex_fonts::PdfFontMapFile::parse(&bytes)?,
                    ));
                }
            }
            UnexpandablePrimitive::PdfMapLine => {
                let bytes = first.expect("map line scanned text");
                if bytes.iter().all(u8::is_ascii_whitespace) {
                    stores.push_pdf_font_map(tex_state::PdfFontMapOperation::BlockDefault);
                } else {
                    let duplicate_count = stores.pdf_font_map_duplicate_names().len();
                    stores.push_pdf_font_map(tex_state::PdfFontMapOperation::Line(
                        tex_fonts::PdfFontMapEntry::parse(&bytes)?,
                    ));
                    let duplicates = stores.pdf_font_map_duplicate_names();
                    if duplicates.len() > duplicate_count
                        && stores.int_param(IntParam::PDF_SUPPRESS_WARNING_DUP_MAP) <= 0
                    {
                        let name = String::from_utf8_lossy(
                            duplicates.last().expect("new duplicate has a name"),
                        );
                        stores.printer().print_rendered(&format!(
                                "\npdfTeX warning: pdftex: fontmap entry for `{name}' already exists, duplicates ignored\n"
                            ));
                    }
                }
            }
            _ => unreachable!("scanner restricts PDF font actions"),
        }
        Ok(ReplayStep::Continue)
    }
}

pub(in crate::main_control) fn execute_immediate_extension<G>(
    stores: &mut tex_state::CommandContext<'_, G>,
    modes: &mut ModeNest,
    command: &mut CommandMachine<'_, '_, G>,
    extension: &mut tex_command::ImmediateExtension<tex_state::TokenListId<G>>,
) -> Result<ReplayStep, ExecError> {
    let mut linear_stores = LinearCommandContext::new(stores);
    let stores = &mut linear_stores;
    {
        match extension {
            tex_command::ImmediateExtension::Continue => {}
            tex_command::ImmediateExtension::PdfExtensionInDviMode(primitive) => {
                let name = match primitive {
                    UnexpandablePrimitive::PdfObject => "pdfobj",
                    UnexpandablePrimitive::PdfXForm => "pdfxform",
                    UnexpandablePrimitive::PdfXImage => "pdfximage",
                    _ => unreachable!("only immediate PDF extensions reach this result"),
                };
                return Err(ExecError::PdfExtensionInDviMode(name));
            }
            tex_command::ImmediateExtension::OpenOut { stream, file_name } => {
                let target = replay_openout_target(file_name.packed());
                stores.open_output_stream(StreamSlot::new(*stream), target.clone().into());
                command
                    .capabilities
                    .invalidate_input_unavailability_for_output(&target);
                command
                    .capabilities
                    .invalidate_font_unavailability_for_output(std::path::Path::new(&target));
                command
                    .capabilities
                    .invalidate_pdf_images_for_output(std::path::Path::new(&target));
                if command.state.engine_semantics().supports_pdftex() {
                    let tracing_online = stores.int_param(IntParam::TRACING_ONLINE);
                    let (terminal_line_is_open, log_line_is_open) = {
                        let printer = stores.printer();
                        (printer.terminal_offset() > 0, printer.log_offset() > 0)
                    };
                    let (sink, text) = crate::diagnostics::report_openout(
                        tracing_online,
                        terminal_line_is_open,
                        log_line_is_open,
                        *stream,
                        &target,
                    );
                    write_preframed_immediate_text(stores, command, sink, text);
                }
            }
            tex_command::ImmediateExtension::Write { stream, tokens } => {
                let text = write_text(tokens, stores);
                let bytes = command
                    .state
                    .profile()
                    .encode_output_text(&text)
                    .expect("write text belongs to the active command character domain");
                if let Some(sink) = immediate_write_sink(*stream, stores) {
                    write_immediate_encoded_text(stores, command, sink, text, bytes);
                }
            }
            tex_command::ImmediateExtension::CloseOut { stream } => {
                if let Some(stream) = stream.stream_slot() {
                    stores.close_output_stream(stream);
                }
            }
            tex_command::ImmediateExtension::PdfObject(request) => {
                if matches!(request, tex_command::PdfObjectRequest::Reserve) {
                    return Err(ExecError::PdfImmediateReservedObject);
                }
                apply_pdf_object_request(request, stores, true)?;
            }
            tex_command::ImmediateExtension::PdfForm(request) => {
                let pending = apply_pdf_form_request(request, stores, modes, command, true)?;
                assert!(
                    pending.is_none(),
                    "immediate form publication is intercepted before generic cold apply"
                );
            }
            tex_command::ImmediateExtension::PdfImage(_) => {
                unreachable!("immediate image requests are normalized before resolution")
            }
        }
        Ok(ReplayStep::Continue)
    }
}
